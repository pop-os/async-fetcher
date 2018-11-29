use super::FetchedState;
use chrono::{DateTime, Utc};
use failure::{Fail, ResultExt};
use filetime::FileTime;
use futures::{future::ok as OkFuture, Future, Stream};
use reqwest::{self, async::Response};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::fs::File;
use FetchError;

/// This state manages downloading a response into the temporary location.
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
                let desc = format!("async fetch for {} failed", dl1.display());
                FetchError::from(why.context(desc))
            })
            .and_then(move |resp| {
                let future: Box<
                    dyn Future<Item = Option<Option<FileTime>>, Error = FetchError> + Send,
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

                        let future = File::create(download_location_.clone())
                            .map_err(move |why| {
                                let desc = format!("failed to create file at {}", dl2.display());
                                FetchError::from(why.context(desc))
                            })
                            .and_then(move |mut file| {
                                debug!("downloading to {}", download_location_.display());
                                resp.into_body()
                                        .map_err(|why| {
                                            let desc = "async I/O write error";
                                            FetchError::from(why.context(desc))
                                        })
                                        // Attempt to write each chunk to our file.
                                        .for_each(move |chunk| {
                                            file.write_all(chunk.as_ref())
                                                .map(|_| ())
                                                .map_err(|why| {
                                                    let desc = "async I/O write error";
                                                    FetchError::from(why.context(desc))
                                                })
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
