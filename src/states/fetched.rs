use super::CompletedState;
use digest::Digest;
use failure::Fail;
use filetime::{self, FileTime};
use futures::{
    future::{lazy, ok as OkFuture, IntoFuture, Future},
    sync::oneshot,
};
use hashing::hash_from_path;
use std::{
    fs::{remove_file as remove_file_sync, File as SyncFile},
    io::{self, Write},
    path::Path,
    sync::Arc,
};
use tokio::{
    executor::DefaultExecutor,
    fs::{remove_file, rename},
};
use FetchError;
use FetchErrorKind;
use FetchEvent;
use FetcherExt;

/// This state manages renaming to the destination, and setting the timestamp of the fetched file.
pub struct FetchedState {
    pub future: Box<dyn Future<Item = (Arc<Path>, Option<(Option<FileTime>)>), Error = FetchError> + Send>,
    pub download_location: Arc<Path>,
    pub(crate) progress:   Option<Arc<dyn Fn(FetchEvent) + Send + Sync>>,
}

impl FetchedState {
    /// Apply a `Digest`-able hash method to the downloaded file, and compare the checksum to the
    /// given input.
    pub fn with_checksum<H: Digest>(self, checksum: &str) -> Self {
        let checksum: Arc<str> = checksum.into();
        let download_location = self.download_location;
        let cb = self.progress.clone();

        // Simply "enhance" our future to append an extra action.
        let new_future = {
            let download_location = download_location.clone();
            self.future.and_then(|resp| {
                oneshot::spawn_fn(
                    move || {
                        if resp.1.is_none() {
                            return Ok(resp);
                        }

                        if let Some(cb) = cb {
                            cb(FetchEvent::Processing);
                        }

                        hash_from_path::<H>(&download_location, &checksum).map_err(|why| {
                            why.context(FetchErrorKind::DestinationHash(
                                download_location.to_path_buf(),
                            ))
                        })?;

                        Ok(resp)
                    },
                    &DefaultExecutor::current()
                )
            })
        };

        Self {
            future: Box::new(new_future),
            download_location,
            progress: self.progress,
        }
    }

    /// Replaces and renames the fetched file, then sets the file times.
    pub fn then_rename(self) -> CompletedState<impl Future<Item = Arc<Path>, Error = FetchError> + Send> {
        let partial = self.download_location;

        let future =
            {
                self.future
                    .and_then(move |(dest, resp)| {
                        let requires_processing = resp.is_some();
                        let dest_ = dest.clone();

                        // Remove the original file and rename, if required.
                        let rename_future: Box<dyn Future<Item = (), Error = FetchError> + Send> = {
                            if requires_processing {
                                if dest.exists() {
                                    let d1 = dest.clone();
                                    let future =
                                        remove_file(dest.clone())
                                            .map_err(move |why| {
                                                FetchError::from(why.context(
                                                    FetchErrorKind::Remove(d1.to_path_buf()),
                                                ))
                                            })
                                            .and_then(move |_| {
                                                rename(partial.clone(), dest.clone())
                                                    .map_err(move |why| {
                                                        why.context(FetchErrorKind::Rename {
                                                            src: partial.to_path_buf(),
                                                            dst: dest.to_path_buf(),
                                                        })
                                                    })
                                                    .map_err(FetchError::from)
                                            });

                                    Box::new(future)
                                } else {
                                    let future = rename(partial.clone(), dest.clone())
                                        .map_err(move |why| {
                                            why.context(FetchErrorKind::Rename {
                                                src: partial.to_path_buf(),
                                                dst: dest.to_path_buf(),
                                            })
                                        })
                                        .map_err(FetchError::from);

                                    Box::new(future)
                                }
                            } else {
                                Box::new(OkFuture(()))
                            }
                        };

                        rename_future.map(move |_| (dest_, resp))
                    })
                    .and_then(|(dest, resp)| {
                        lazy(move || {
                            if let Some(Some(ftime)) = resp {
                                debug!(
                                    "setting timestamp on {} to {:?}",
                                    dest.as_ref().display(),
                                    ftime
                                );

                                filetime::set_file_times(dest.as_ref(), ftime, ftime)
                                    .map_err(|why| {
                                        FetchError::from(why.context(FetchErrorKind::FileTime(
                                            dest.to_path_buf(),
                                        )))
                                    })?;
                            }

                            Ok(dest)
                        })
                    })
            };

        CompletedState {
            future:      Box::new(future),
        }
    }

    /// Processes the fetched file, storing the output to the destination, then setting the file times.
    ///
    /// Use this to decompress an archive if the fetched file was an archive.
    pub fn then_process<F>(
        self,
        construct_writer: F,
    ) -> CompletedState<impl Future<Item = Arc<Path>, Error = FetchError> + Send>
    where
        F: Fn(SyncFile) -> io::Result<Box<dyn Write + Send>> + Send + 'static,
    {
        let partial = self.download_location;

        let future = {
            self.future.and_then(move |(dest, ftime)| {
                let requires_processing = ftime.is_some();
                let dest_out = dest.clone();

                let decompress: Box<dyn Future<Item = (), Error = FetchError> + Send> = if requires_processing {
                    Box::new(oneshot::spawn_fn(
                        move || {
                            debug!("constructing decompressor for {}", dest.display());
                            let file = SyncFile::create(dest.as_ref()).map_err(|why| {
                                FetchError::from(
                                    why.context(FetchErrorKind::CreateDestination(dest.to_path_buf())),
                                )
                            })?;

                            let mut destination = construct_writer(file).map_err(|why| {
                                FetchError::from(
                                    why.context(FetchErrorKind::WriterConstruction(dest.to_path_buf())),
                                )
                            })?;

                            let mut partial_file = SyncFile::open(partial.as_ref()).map_err(|why| {
                                FetchError::from(
                                    why.context(FetchErrorKind::Open(partial.to_path_buf())),
                                )
                            })?;

                            debug!("processing {} to {}", partial.display(), dest.display());
                            io::copy(&mut partial_file, &mut destination).map_err(|why| {
                                FetchError::from(why.context(FetchErrorKind::Copy {
                                    src: partial.to_path_buf(),
                                    dst: dest.to_path_buf(),
                                }))
                            })?;

                            debug!("removing partial file at {}", partial.display());
                            remove_file_sync(partial.as_ref()).map_err(|why| {
                                FetchError::from(
                                    why.context(FetchErrorKind::Remove(partial.to_path_buf())),
                                )
                            })?;

                            if let Some(Some(ftime)) = ftime {
                                debug!(
                                    "setting timestamp on {} to {:?}",
                                    dest.display(),
                                    ftime
                                );

                                filetime::set_file_times(dest.as_ref(), ftime, ftime)
                                    .map_err(|why| {
                                        FetchError::from(why.context(FetchErrorKind::FileTime(
                                            dest.to_path_buf(),
                                        )))
                                    })?;
                            }

                            Ok(())
                        },
                        &DefaultExecutor::current()
                    ))
                } else {
                    Box::new(OkFuture(()))
                };

                decompress.map(|_| dest_out)
            })
        };

        CompletedState {
            future:      Box::new(future),
        }
    }
}

impl FetcherExt for FetchedState {
    fn wrap_future(
        mut self,
        mut func: impl FnMut(<Self as IntoFuture>::Future) -> <Self as IntoFuture>::Future + Send
    ) -> Self {
        self.future = func(self.future);
        self
    }
}

impl IntoFuture for FetchedState {
    type Future = Box<dyn Future<Item = Self::Item, Error = Self::Error> + Send>;
    type Item = (Arc<Path>, Option<Option<FileTime>>);
    type Error = FetchError;

    fn into_future(self) -> Self::Future {
        self.future
    }
}
