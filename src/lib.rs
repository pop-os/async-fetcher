//! Provides a high level abstraction over a reqwest async client for optionally fetching
//! files from a given URL based on a timestamp comparison of the `LAST_MODIFIED` header
//! and an existing local file; and optionally processing the fetched file before
//! moving it into the final destination.
//!
//! # Notes
//! - The generated futures are compatible with multi-threaded runtimes.
//! - See the examples directory in the source repository for an example of it in practice.
#[macro_use]
extern crate log;

extern crate chrono;
extern crate filetime;
extern crate futures;
extern crate reqwest;
extern crate tokio;

use chrono::{DateTime, Utc};
use filetime::FileTime;
use futures::{Future, Stream};
use reqwest::{
    async::{Client, Response},
    header::{CONTENT_LENGTH, IF_MODIFIED_SINCE, LAST_MODIFIED},
    StatusCode,
};
use std::{
    fs::{remove_file as remove_file_sync, File as SyncFile},
    io::{self, Write},
    path::{Path, PathBuf},
};
use tokio::fs::{remove_file, rename, File};

/// Slightly opinionated asynchronous fetcher of files via an async reqwest Client.
///
/// - Files will only be downloaded if the timestamp of the file is older than the timestamp on the server.
/// - Partial downloads may be stored at an optional temporary location, then renamed after completion.
pub struct AsyncFetcher {
    from_url: String,
}

impl AsyncFetcher {
    /// Initialze a new featuer to fetch from the given URL.
    ///
    /// Stores the complete file to `to_path` when done.
    pub fn new(from_url: String) -> Self {
        Self { from_url }
    }

    /// Submit the GET request for the file, and get an optional response if a download is required.
    ///
    /// If the `to_path` path already exists, the time of this file will be used with the
    /// `IF_MODIFIED_SINCE` GET header when requesting the file. The server will respond with
    /// `NOT_MODIFIED` if the file we want to fetch has already been fetched.
    ///
    /// Returns a `ResponseState`, which can either be manually handled by the caller, or used
    /// to commit the download with this API.
    pub fn request_to_path(
        self,
        client: &Client,
        to_path: PathBuf,
    ) -> ResponseState<impl Future<Item = Option<(Response, Option<DateTime<Utc>>)>, Error = reqwest::Error> + Send> {
        let mut req = client.get(&self.from_url);

        let current = if to_path.exists() {
            let when = self.date_time(&to_path).unwrap();
            let rfc = when.to_rfc2822();
            debug!(
                "Setting IF_MODIFIED_SINCE header for {} to {}",
                to_path.display(),
                rfc
            );

            req = req.header(IF_MODIFIED_SINCE, rfc);
            Some(when)
        } else {
            None
        };

        let url = self.from_url;
        let future = req
            .send()
            .and_then(|resp| resp.error_for_status())
            .map(move |resp| {
                if resp.status() == StatusCode::NOT_MODIFIED {
                    debug!("{} was already fetched", url);
                    None
                } else {
                    let date = resp.headers()
                        .get(LAST_MODIFIED)
                        .and_then(|h| h.to_str().ok())
                        .and_then(|header| DateTime::parse_from_rfc2822(header).ok())
                        .map(|tz| tz.with_timezone(&Utc));

                        let fetch = date.as_ref()
                            .and_then(|server| current.map(|current| (server, current)))
                            .map_or(true, |(&server, current)| current < server);

                    if fetch {
                        debug!("GET {}", url);
                        Some((resp, date))
                    } else {
                        debug!("{} was already fetched", url);
                        None
                    }
                }
            });

        ResponseState {
            future,
            path: to_path,
        }
    }

    fn date_time(&self, to_path: &Path) -> io::Result<DateTime<Utc>> {
        Ok(DateTime::from(to_path.metadata()?.modified()?))
    }
}

/// This state manages downloading a response into the download location, whether it is a
/// temporary or final destination.
pub struct ResponseState<T: Future<Item = Option<(Response, Option<DateTime<Utc>>)>, Error = reqwest::Error> + Send> {
    pub future: T,
    pub path: PathBuf,
}

impl<T: Future<Item = Option<(Response, Option<DateTime<Utc>>)>, Error = reqwest::Error> + Send> ResponseState<T> {
    /// Commits the download from the response, writing it to a file.
    pub fn then_download(
        self,
        download_location: PathBuf,
    ) -> FetchedState<impl Future<Item = Option<Option<FileTime>>, Error = io::Error> + Send> {
        let final_destination = self.path;
        let future = self.future;

        // Fetch the file to the download location.
        let download_location_ = download_location.clone();
        let download_future = future
            .map_err(|why| {
                io::Error::new(io::ErrorKind::Other, format!("async fetch failed: {}", why))
            })
            .and_then(move |resp| {
                let future: Box<
                    dyn Future<Item = Option<Option<FileTime>>, Error = io::Error> + Send,
                > = match resp {
                    None => Box::new(futures::future::ok(None)),
                    Some((resp, date)) => {
                        // TODO: Use this to set length of async file.
                        let length = resp
                            .headers()
                            .get(CONTENT_LENGTH)
                            .and_then(|h| h.to_str().ok())
                            .and_then(|h| h.parse::<usize>().ok())
                            .unwrap_or(0);

                        let future = File::create(download_location_.clone())
                            .and_then(move |mut file| {
                                debug!("downloading to {}", download_location_.display());
                                resp.into_body()
                                    .map_err(|why| io::Error::new(
                                        io::ErrorKind::Other,
                                        format!("async I/O write error: {}", why)
                                    ))
                                    // Attempt to write each chunk to our file.
                                    .for_each(move |chunk| {
                                        file.write_all(chunk.as_ref())
                                            .map(|_| ())
                                    })
                                    // On success, we will return the filetime to assign to the destionation.
                                    .map(move |_| Some(date.map(|date| FileTime::from_unix_time(date.timestamp(), 0))))
                            });

                        Box::new(future)
                    }
                };

                future
            });

        FetchedState {
            future: download_future,
            download_location,
            final_destination,
        }
    }
}

/// This state manages renaming to the destination, and setting the timestamp of the fetched file.
pub struct FetchedState<T: Future<Item = Option<Option<FileTime>>, Error = io::Error> + Send> {
    pub future: T,
    pub download_location: PathBuf,
    pub final_destination: PathBuf,
}

impl<T: Future<Item = Option<Option<FileTime>>, Error = io::Error> + Send> FetchedState<T> {
    /// Replaces and renames the fetched file, then sets the file times.
    pub fn then_rename(self) -> impl Future<Item = (), Error = io::Error> + Send {
        let partial = self.download_location;
        let dest = self.final_destination;
        let dest_copy = dest.clone();

        self.future
            .and_then(|ftime| {
                let requires_rename = ftime.is_some();

                // Remove the original file and rename, if required.
                let rename_future: Box<dyn Future<Item = (), Error = io::Error> + Send> = {
                    if requires_rename && partial != dest {
                        if dest.exists() {
                            debug!("replacing {} with {}", dest.display(), partial.display());
                            Box::new(remove_file(dest.clone()).and_then(|_| rename(partial, dest)))
                        } else {
                            debug!("renaming {} to {}", partial.display(), dest.display());
                            Box::new(rename(partial, dest))
                        }
                    } else {
                        Box::new(futures::future::ok(()))
                    }
                };

                rename_future.map(move |_| ftime)
            })
            .and_then(|ftime| {
                futures::future::lazy(move || {
                    if let Some(Some(ftime)) = ftime {
                        debug!("setting timestamp on {} to {:?}", dest_copy.display(), ftime);
                        filetime::set_file_times(dest_copy, ftime, ftime)?;
                    }

                    Ok(())
                })
            })
    }

    /// Processes the fetched file, storing the output to the destination, then setting the file times.
    ///
    /// Use this to decompress an archive if the fetched file was an archive.
    pub fn then_process<F>(self, construct_writer: F) -> impl Future<Item = (), Error = io::Error> + Send
    where
        F: Fn(SyncFile) -> Box<dyn Write + Send> + Send,
    {
        let partial = self.download_location;
        let dest = self.final_destination;
        let dest_copy = dest.clone();

        self.future
            .and_then(move |ftime| {
                let requires_processing = ftime.is_some();

                let decompress_future = {
                    futures::future::lazy(move || {
                        if requires_processing {
                            debug!("constructing decompressor for {}", dest.display());
                            let file = SyncFile::create(&dest)?;
                            let mut writer = construct_writer(file);

                            debug!("processing to {}", dest.display());
                            io::copy(&mut SyncFile::open(&partial)?, &mut writer)?;

                            debug!("removing partial file at {}", partial.display());
                            remove_file_sync(partial)?;
                        }

                        Ok(())
                    })
                };

                decompress_future.map(move |_| ftime)
            })
            .and_then(|ftime| {
                futures::future::lazy(move || {
                    if let Some(Some(ftime)) = ftime {
                        debug!("setting timestamp on {} to {:?}", dest_copy.display(), ftime);
                        filetime::set_file_times(dest_copy, ftime, ftime)?;
                    }

                    Ok(())
                })
            })
    }
}
