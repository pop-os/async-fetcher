use crate::checksum::{Checksum, ChecksumError};

use async_fetcher::Error as FetchError;
use async_std::{
    fs::{self, File},
    task,
};
use futures::{channel::mpsc, executor, prelude::*};
use std::{io, path::Path, sync::Arc};

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("failed to fetch")]
    Fetch(#[source] FetchError),
    #[error("checksum is invalid")]
    Checksum(#[source] ChecksumError),
    #[error("failed to open source for checksum validation")]
    Open(#[source] io::Error),
    #[error("failed to remove file")]
    Remove(#[source] io::Error),
}

pub async fn validator<E, F>(
    mut rx: mpsc::Receiver<(Arc<Path>, Result<Option<Checksum>, FetchError>)>,
    event_handler: E,
) where
    E: Fn(Arc<Path>, Result<(), ValidationError>) -> F + 'static,
    F: Future<Output = ()>,
{
    let mut waiting = Vec::new();
    while let Some((dest, result)) = rx.next().await {
        match result {
            Ok(None) => (),
            Ok(Some(checksum)) => {
                let handle = task::spawn_blocking(|| {
                    executor::block_on(async move {
                        let buf = &mut [0u8; 8 * 1024];

                        match File::open(&*dest).await {
                            Ok(file) => match checksum.validate(file, buf).await {
                                Ok(()) => (dest, Ok(())),
                                Err(why) => (dest, Err(ValidationError::Checksum(why))),
                            },
                            Err(why) => (dest, Err(ValidationError::Open(why))),
                        }
                    })
                });

                waiting.push(handle);
            }
            Err(why) => {
                event_handler(dest, Err(ValidationError::Fetch(why))).await;
            }
        }
    }

    for handle in waiting {
        let (path, result) = handle.await;

        match result {
            Ok(()) => event_handler(path, Ok(())).await,
            Err(why) => {
                event_handler(path.clone(), Err(why)).await;

                if let Err(why) = fs::remove_file(&*path).await {
                    event_handler(path, Err(ValidationError::Remove(why))).await;
                }
            }
        }
    }
}
