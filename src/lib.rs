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
extern crate digest;
extern crate filetime;
extern crate futures;
extern crate hex_view;
extern crate reqwest;
extern crate tokio;

use chrono::{DateTime, Utc};
use digest::Digest;
use filetime::FileTime;
use futures::{future::ok as OkFuture, Future, Stream};
use hex_view::HexView;
use reqwest::{
    async::{Client, RequestBuilder, Response},
    header::{CONTENT_LENGTH, IF_MODIFIED_SINCE, LAST_MODIFIED},
    StatusCode,
};
use std::{
    fs::{remove_file as remove_file_sync, File as SyncFile},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::fs::{remove_file, rename, File};

/// Slightly opinionated asynchronous fetcher of files via an async reqwest Client.
///
/// - Files will only be downloaded if the timestamp of the file is older than the timestamp on the server.
/// - Partial downloads may be stored at an optional temporary location, then renamed after completion.
pub struct AsyncFetcher<'a> {
    client: &'a Client,
    from_url: String,
}

impl<'a> AsyncFetcher<'a> {
    /// Initialze a new featuer to fetch from the given URL.
    ///
    /// Stores the complete file to `to_path` when done.
    pub fn new(client: &'a Client, from_url: String) -> Self {
        Self { client, from_url }
    }

    /// Submit the GET request for the file, if the modified time is too old.
    ///
    /// If the `to_path` path already exists, the time of this file will be used with the
    /// `IF_MODIFIED_SINCE` GET header when requesting the file. The server will respond with
    /// `NOT_MODIFIED` if the file we want to fetch has already been fetched.
    ///
    /// Returns a `ResponseState`, which can either be manually handled by the caller, or used
    /// to commit the download with this API.
    pub fn request_to_path(
        self,
        to_path: PathBuf,
    ) -> ResponseState<
        impl Future<Item = Option<(Response, Option<DateTime<Utc>>)>, Error = reqwest::Error> + Send,
    > {
        let (req, current) = self.set_if_modified_since(&to_path, self.client.get(&self.from_url));
        let url = self.from_url;

        ResponseState {
            future: req
                .send()
                .and_then(|resp| resp.error_for_status())
                .map(move |resp| check_response(resp, &url, current)),
            path: to_path,
        }
    }

    /// Submit the GET request for the file, if checksums are a mismatch.
    ///
    /// Returns a `ResponseState`, which can either be manually handled by the caller, or used
    /// to commit the download with this API.
    pub fn request_with_checksum_to_path<D: Digest>(
        self,
        to_path: PathBuf,
        checksum: Arc<str>,
    ) -> ResponseState<
        impl Future<Item = Option<(Response, Option<DateTime<Utc>>)>, Error = reqwest::Error> + Send,
    > {
        let future: Box<
            dyn Future<Item = Option<(Response, Option<DateTime<Utc>>)>, Error = reqwest::Error>
                + Send,
        > = if hash_from_path::<D>(&to_path, &checksum).is_ok() {
            debug!("checksum of destination matches the requested checksum");
            Box::new(OkFuture(None))
        } else {
            let req = self.client.get(&self.from_url);
            let url = self.from_url;
            let future = req
                .send()
                .and_then(|resp| resp.error_for_status())
                .map(move |resp| check_response(resp, &url, None));
            Box::new(future)
        };

        ResponseState {
            future,
            path: to_path,
        }
    }

    fn set_if_modified_since(
        &self,
        to_path: &Path,
        mut req: RequestBuilder,
    ) -> (RequestBuilder, Option<DateTime<Utc>>) {
        let date = if to_path.exists() {
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

        (req, date)
    }

    fn date_time(&self, to_path: &Path) -> io::Result<DateTime<Utc>> {
        Ok(DateTime::from(to_path.metadata()?.modified()?))
    }
}

/// This state manages downloading a response into the download location, whether it is a
/// temporary or final destination.
pub struct ResponseState<
    T: Future<Item = Option<(Response, Option<DateTime<Utc>>)>, Error = reqwest::Error>
        + Send
        + 'static,
> {
    pub future: T,
    pub path: PathBuf,
}

impl<
        T: Future<Item = Option<(Response, Option<DateTime<Utc>>)>, Error = reqwest::Error>
            + Send
            + 'static,
    > ResponseState<T>
{
    pub fn then_download(self, download_location: PathBuf) -> FetchedState {
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
                    None => Box::new(OkFuture(None)),
                    Some((resp, date)) => {
                        // TODO: Use this to set length of async file.
                        // let length = resp
                        //     .headers()
                        //     .get(CONTENT_LENGTH)
                        //     .and_then(|h| h.to_str().ok())
                        //     .and_then(|h| h.parse::<usize>().ok())
                        //     .unwrap_or(0);

                        let future = File::create(download_location_.clone()).and_then(
                            move |mut file| {
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
                            },
                        );

                        Box::new(future)
                    }
                };

                future
            });

        FetchedState {
            future: Box::new(download_future),
            download_location: Arc::from(download_location),
            final_destination: Arc::from(final_destination),
        }
    }
}

/// This state manages renaming to the destination, and setting the timestamp of the fetched file.
pub struct FetchedState {
    pub future: Box<dyn Future<Item = Option<Option<FileTime>>, Error = io::Error> + Send>,
    pub download_location: Arc<Path>,
    pub final_destination: Arc<Path>,
}

impl FetchedState {
    /// Apply a `Digest`-able hash method to the downloaded file, and compare the checksum to the
    /// given input.
    pub fn with_checksum<H: Digest>(self, checksum: Arc<str>) -> Self {
        let download_location = self.download_location;
        let final_destination = self.final_destination;

        // Simply "enhance" our future to append an extra action.
        let new_future = {
            let download_location = download_location.clone();
            self.future.and_then(|resp| {
                futures::future::lazy(move || {
                    if resp.is_none() {
                        return Ok(resp);
                    }

                    hash_from_path::<H>(&download_location, &checksum).map(move |_| resp)
                })
            })
        };

        Self {
            future: Box::new(new_future),
            download_location: download_location,
            final_destination: final_destination,
        }
    }

    /// Replaces and renames the fetched file, then sets the file times.
    pub fn then_rename(self) -> impl Future<Item = (), Error = io::Error> + Send {
        let partial = self.download_location;
        let dest = self.final_destination;
        let dest_copy = dest.clone();

        self.future
            .and_then(move |ftime| {
                let requires_rename = ftime.is_some();

                // Remove the original file and rename, if required.
                let rename_future: Box<
                    dyn Future<Item = (), Error = io::Error> + Send,
                > = {
                    if requires_rename && partial != dest {
                        if dest.exists() {
                            debug!("replacing {} with {}", dest.display(), partial.display());
                            let future =
                                remove_file(dest.clone()).and_then(move |_| rename(partial, dest));
                            Box::new(future)
                        } else {
                            debug!("renaming {} to {}", partial.display(), dest.display());
                            Box::new(rename(partial, dest))
                        }
                    } else {
                        Box::new(OkFuture(()))
                    }
                };

                rename_future.map(move |_| ftime)
            })
            .and_then(|ftime| {
                futures::future::lazy(move || {
                    if let Some(Some(ftime)) = ftime {
                        debug!(
                            "setting timestamp on {} to {:?}",
                            dest_copy.as_ref().display(),
                            ftime
                        );
                        filetime::set_file_times(dest_copy.as_ref(), ftime, ftime)?;
                    }

                    Ok(())
                })
            })
    }

    /// Processes the fetched file, storing the output to the destination, then setting the file times.
    ///
    /// Use this to decompress an archive if the fetched file was an archive.
    pub fn then_process<F>(
        self,
        construct_writer: F,
    ) -> impl Future<Item = (), Error = io::Error> + Send
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
                            let file = SyncFile::create(dest.as_ref())?;
                            let mut writer = construct_writer(file);

                            debug!("processing to {}", dest.display());
                            io::copy(&mut SyncFile::open(partial.as_ref())?, &mut writer)?;

                            debug!("removing partial file at {}", partial.display());
                            remove_file_sync(partial.as_ref())?;
                        }

                        Ok(())
                    })
                };

                decompress_future.map(move |_| ftime)
            })
            .and_then(|ftime| {
                futures::future::lazy(move || {
                    if let Some(Some(ftime)) = ftime {
                        debug!(
                            "setting timestamp on {} to {:?}",
                            dest_copy.display(),
                            ftime
                        );
                        filetime::set_file_times(dest_copy.as_ref(), ftime, ftime)?;
                    }

                    Ok(())
                })
            })
    }
}

fn check_response(
    resp: Response,
    url: &str,
    current: Option<DateTime<Utc>>,
) -> Option<(Response, Option<DateTime<Utc>>)> {
    if resp.status() == StatusCode::NOT_MODIFIED {
        debug!("{} was already fetched", url);
        None
    } else {
        let date = resp
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|h| h.to_str().ok())
            .and_then(|header| DateTime::parse_from_rfc2822(header).ok())
            .map(|tz| tz.with_timezone(&Utc));

        let fetch = date
            .as_ref()
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
}

fn hash_from_path<D: Digest>(path: &Path, checksum: &str) -> io::Result<()> {
    debug!("constructing hasher for {}", path.display());
    let reader = SyncFile::open(path)?;
    hasher::<D, SyncFile>(reader, checksum)
}

fn hasher<D: Digest, R: Read>(mut reader: R, checksum: &str) -> io::Result<()> {
    let mut buffer = [0u8; 8 * 1024];
    let mut hasher = D::new();

    loop {
        let read = reader.read(&mut buffer)?;

        if read == 0 {
            break;
        }
        hasher.input(&buffer[..read]);
    }

    let result = hasher.result();
    let hash = format!("{:x}", HexView::from(result.as_slice()));
    if hash == checksum.as_ref() {
        debug!("checksum is valid");
        Ok(())
    } else {
        debug!("invalid checksum found: {} != {}", hash, checksum);
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid checksum",
        ))
    }
}
