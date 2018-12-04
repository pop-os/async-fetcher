use super::FetchedState;
use chrono::{DateTime, Utc};
use failure::{Fail, ResultExt};
use filetime::FileTime;
use futures::{future::ok as OkFuture, Future, Stream};
use reqwest::{self, async::Response, header::CONTENT_LENGTH};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{fs::File, io::flush};
use {FetchError, FetchErrorKind};

pub trait RequestFuture:
    Future<Item = Option<(Response, Option<DateTime<Utc>>)>, Error = reqwest::Error> + Send
{
}

impl<
        T: Future<Item = Option<(Response, Option<DateTime<Utc>>)>, Error = reqwest::Error> + Send,
    > RequestFuture for T
{}

/// This state manages downloading a response into the temporary location.
pub struct ResponseState<T: RequestFuture + 'static> {
    pub future: T,
    pub path: PathBuf,
}

impl<T: RequestFuture + 'static> ResponseState<T> {
    /// If the file is to be downloaded, this will construct a future that does just that.
    pub fn then_download(self, download_location: PathBuf) -> FetchedState {
        let final_destination = self.path;
        let future = self.future;

        // Fetch the file to the download location.
        let download_location_: Arc<Path> = Arc::from(download_location.clone());
        let dl1 = download_location_.clone();
        let dl2 = download_location_.clone();
        let download_future = future
            .map_err(move |why| {
                FetchError::from(why.context(FetchErrorKind::Fetch(dl1.to_path_buf())))
            })
            .and_then(move |resp| {
                let future: Box<
                    dyn Future<Item = Option<Option<FileTime>>, Error = FetchError> + Send,
                > = match resp {
                    None => Box::new(OkFuture(None)),
                    Some((resp, date)) => {
                        let length = resp
                            .headers()
                            .get(CONTENT_LENGTH)
                            .and_then(|h| h.to_str().ok())
                            .and_then(|h| h.parse::<u64>().ok())
                            .unwrap_or(0);

                        let future = File::create(download_location_.clone())
                            .map_err(move |why| {
                                FetchError::from(why.context(FetchErrorKind::Create(dl2.to_path_buf())))
                            })
                            // Set the length of the file to the length we fetched from the header.
                            .and_then(move |file| {
                                let file = file.into_std();
                                file.set_len(length).context(FetchErrorKind::LengthSet)?;
                                let copy = file.try_clone().context(FetchErrorKind::FdCopy)?;
                                Ok((File::from_std(file), File::from_std(copy)))
                            })
                            // Download the file to the given download location.
                            .and_then(move |(mut file, copy)| {
                                debug!("downloading to {}", download_location_.display());
                                resp.into_body()
                                        .map_err(|why| {
                                            FetchError::from(why.context(FetchErrorKind::ChunkRequest))
                                        })
                                        // Attempt to write each chunk to our file.
                                        .for_each(move |chunk| {
                                            file.write_all(chunk.as_ref())
                                                .map(|_| ())
                                                .context(FetchErrorKind::ChunkWrite)
                                                .map_err(FetchError::from)
                                        })
                                        // Return the file on success.
                                        .map(move |_| copy)
                            })
                            // Ensure that the file is fully written to the disk.
                            .and_then(|file| {
                                flush(file).map_err(|why| {
                                    FetchError::from(why.context(FetchErrorKind::Flush))
                                })
                            })
                            // On success, we will return the filetime to assign to the destionation.
                            .map(move |_| Some(date.map(|date| FileTime::from_unix_time(date.timestamp(), 0))));

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

    /// Convert this state into the future that it owns.
    pub fn into_future(self) -> T {
        self.future
    }
}
