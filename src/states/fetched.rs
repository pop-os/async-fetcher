use super::CompletedState;
use digest::Digest;
use failure::{Fail, ResultExt};
use filetime::{self, FileTime};
use futures::{self, future::ok as OkFuture, Future, Stream};
use hashing::hash_from_path;
use std::{
    fs::{remove_file as remove_file_sync, File as SyncFile},
    io::{self, Write},
    path::Path,
    sync::Arc,
};
use tokio::fs::{remove_file, rename};
use FetchError;

/// This state manages renaming to the destination, and setting the timestamp of the fetched file.
pub struct FetchedState {
    pub future: Box<dyn Future<Item = Option<Option<FileTime>>, Error = FetchError> + Send>,
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

                    hash_from_path::<H>(&download_location, &checksum).with_context(|why| {
                        format!(
                            "failed to validate hash at {}: {}",
                            download_location.display(),
                            why
                        )
                    })?;

                    Ok(resp)
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
    pub fn then_rename(self) -> CompletedState<impl Future<Item = (), Error = FetchError> + Send> {
        let partial = self.download_location;
        let dest = self.final_destination;
        let dest_copy = dest.clone();

        let future = {
            let dest = dest.clone();
            self.future
                .and_then(move |ftime| {
                    let requires_rename = ftime.is_some();

                    // Remove the original file and rename, if required.
                    let rename_future: Box<
                        dyn Future<Item = (), Error = FetchError> + Send,
                    > = {
                        if requires_rename && partial != dest {
                            if dest.exists() {
                                let d1 = dest.clone();
                                let future = remove_file(dest.clone())
                                    .map_err(move |why| {
                                        let desc = format!("failed to remove {}", d1.display());
                                        FetchError::from(why.context(desc))
                                    })
                                    .and_then(move |_| {
                                        rename(partial.clone(), dest.clone()).map_err(move |why| {
                                            let desc = format!(
                                                "failed to rename {} to {}",
                                                partial.display(),
                                                dest.display()
                                            );
                                            FetchError::from(why.context(desc))
                                        })
                                    });

                                Box::new(future)
                            } else {
                                let future =
                                    rename(partial.clone(), dest.clone()).map_err(move |why| {
                                        let desc = format!(
                                            "failed to rename {} to {}",
                                            partial.display(),
                                            dest.display()
                                        );
                                        FetchError::from(why.context(desc))
                                    });

                                Box::new(future)
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

                            filetime::set_file_times(dest_copy.as_ref(), ftime, ftime)
                                .with_context(|why| {
                                    format!(
                                        "failed to set file times for {}: {}",
                                        dest_copy.display(),
                                        why
                                    )
                                })?;
                        }

                        Ok(())
                    })
                })
        };

        CompletedState {
            future: Box::new(future),
            destination: dest,
        }
    }

    /// Processes the fetched file, storing the output to the destination, then setting the file times.
    ///
    /// Use this to decompress an archive if the fetched file was an archive.
    pub fn then_process<F>(
        self,
        construct_writer: F,
    ) -> CompletedState<impl Future<Item = (), Error = FetchError> + Send>
    where
        F: Fn(SyncFile) -> io::Result<Box<dyn Write + Send>> + Send,
    {
        let partial = self.download_location;
        let dest = self.final_destination;
        let dest_copy = dest.clone();

        let future = {
            let dest = dest.clone();
            self.future
                .and_then(move |ftime| {
                    let requires_processing = ftime.is_some();

                    let decompress_future = {
                        futures::future::lazy(move || {
                            if requires_processing {
                                debug!("constructing decompressor for {}", dest.display());
                                let file = SyncFile::create(dest.as_ref()).with_context(|why| {
                                    format!(
                                        "failed to create destination file ({}): {}",
                                        dest.display(),
                                        why
                                    )
                                })?;

                                let mut destination = construct_writer(file).with_context(|why| {
                                    format!(
                                        "failed to construct writer for destination ({}): {}",
                                        dest.display(),
                                        why
                                    )
                                })?;

                                let mut partial_file = SyncFile::open(partial.as_ref())
                                    .with_context(|why| {
                                        format!(
                                            "failed to open partial file ({}): {}",
                                            partial.display(),
                                            why
                                        )
                                    })?;

                                debug!("processing to {}", dest.display());
                                io::copy(&mut partial_file, &mut destination).with_context(
                                    |why| {
                                        format!(
                                            "failed to copy {} to {}: {}",
                                            partial.display(),
                                            dest.display(),
                                            why
                                        )
                                    },
                                )?;

                                debug!("removing partial file at {}", partial.display());
                                remove_file_sync(partial.as_ref()).with_context(|why| {
                                    format!(
                                        "failed to remove file at {}: {}",
                                        partial.display(),
                                        why
                                    )
                                })?;
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

                            filetime::set_file_times(dest_copy.as_ref(), ftime, ftime)
                                .with_context(|why| {
                                    format!(
                                        "failed to set file times for {}: {}",
                                        dest_copy.display(),
                                        why
                                    )
                                })?;
                        }

                        Ok(())
                    })
                })
        };

        CompletedState {
            future: Box::new(future),
            destination: dest,
        }
    }

    pub fn into_future(
        self,
    ) -> impl Future<Item = Option<Option<FileTime>>, Error = FetchError> + Send {
        self.future
    }
}
