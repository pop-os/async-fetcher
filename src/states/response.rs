use super::FetchedState;
use chrono::{DateTime, Utc};
use failure::{Fail, ResultExt};
use filetime::FileTime;
use futures::{future::ok as OkFuture, IntoFuture, Future, Stream};
use reqwest::{self, async::Response, header::CONTENT_LENGTH};
use std::{
    io::Write,
    path::Path,
    sync::Arc,
};
use tokio::{fs::File, io::flush};
use FetchError;
use FetchErrorKind;
use FetchEvent;
use FetcherExt;

pub trait RequestFuture:
    Future<Item = (Arc<Path>, Option<(Response, Option<DateTime<Utc>>)>), Error = reqwest::Error> + Send
{
}

impl<
        T: Future<Item = (Arc<Path>, Option<(Response, Option<DateTime<Utc>>)>), Error = reqwest::Error> + Send,
    > RequestFuture for T
{
}

/// This state manages downloading a response into the temporary location.
pub struct ResponseState<T: RequestFuture + 'static> {
    pub future:          T,
    pub(crate) progress: Option<Arc<dyn Fn(FetchEvent) + Send + Sync>>,
}

impl<T: RequestFuture + 'static> ResponseState<T> {
    /// If the file is to be downloaded, this will construct a future that does just that.
    pub fn then_download(self, download_location: Arc<Path>) -> FetchedState {
        let future = self.future;
        let cb = self.progress.clone();

        // Fetch the file to the download location.
        let download_location_ = download_location.clone();
        let dl1 = download_location_.clone();
        let dl2 = download_location_.clone();
        let download_future = future
            .map_err(move |why| {
                FetchError::from(why.context(FetchErrorKind::Fetch(dl1.to_path_buf())))
            })
            .and_then(move |(final_destination, resp)| {
                let future: Box<
                    dyn Future<Item = (Arc<Path>, Option<(Option<FileTime>)>), Error = FetchError> + Send,
                > = match resp {
                    None => Box::new(OkFuture((final_destination, None))),
                    Some((resp, date)) => {
                        let length = resp
                            .headers()
                            .get(CONTENT_LENGTH)
                            .and_then(|h| h.to_str().ok())
                            .and_then(|h| h.parse::<u64>().ok())
                            .unwrap_or(0);

                        // Signal the caller that we are about to fetch a file that is this size.
                        if let Some(cb) = cb.as_ref() {
                            cb(FetchEvent::Total(length));
                        }

                        let cb2 = cb.clone();

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
                                resp.into_body()
                                        .map_err(|why| {
                                            FetchError::from(why.context(FetchErrorKind::ChunkRequest))
                                        })
                                        // Attempt to write each chunk to our file.
                                        .for_each(move |chunk| {
                                            let chunk: &[u8] = chunk.as_ref();
                                            file.write_all(chunk)
                                                .map(|_| ())
                                                .context(FetchErrorKind::ChunkWrite)
                                                .map_err(FetchError::from)?;

                                            // Signal the caller that we just fetched this many bytes.
                                            if let Some(cb) = cb.as_ref() {
                                                cb(FetchEvent::Progress(chunk.len() as u64));
                                            }

                                            Ok(())
                                        })
                                        // Return the file on success.
                                        .map(move |_| {
                                            // Signal the caller that we just finished fetching many bytes.
                                            if let Some(cb) = cb2.as_ref() {
                                                cb(FetchEvent::DownloadComplete);
                                            }

                                            copy
                                        })
                            })
                            // Ensure that the file is fully written to the disk.
                            .and_then(|file| {
                                flush(file).map_err(|why| {
                                    FetchError::from(why.context(FetchErrorKind::Flush))
                                })
                            })
                            // On success, we will return the filetime to assign to the destionation.
                            .map(move |_| Some(date.map(|date| FileTime::from_unix_time(date.timestamp(), 0))));

                        Box::new(future.map(|v| (final_destination, v)))
                    }
                };

                future
            });

        FetchedState {
            future:            Box::new(download_future),
            download_location,
            progress:          self.progress,
        }
    }
}

impl<T: RequestFuture + 'static> FetcherExt for ResponseState<T> {
    fn wrap_future(
        mut self,
        mut func: impl FnMut(<Self as IntoFuture>::Future) -> <Self as IntoFuture>::Future + Send
    ) -> Self {
        self.future = func(self.future);
        self
    }
}

impl<T: RequestFuture + 'static> IntoFuture for ResponseState<T> {
    type Future = T;
    type Item = (Arc<Path>, Option<(Response, Option<DateTime<Utc>>)>);
    type Error = reqwest::Error;

    fn into_future(self) -> Self::Future {
        self.future
    }
}
