use crate::checksum::{Checksum, ChecksumError};
use async_std::fs::{self, File};
use async_stream::stream;
use futures::{channel::mpsc, executor, prelude::*};
use std::{io, path::Path, rc::Rc, sync::Arc};

#[derive(Debug, Error)]
pub enum ChecksummerError {
    #[error("checksum is invalid")]
    Checksum(#[source] ChecksumError),
    #[error("failed to open source for checksum validation")]
    Open(#[source] io::Error),
}

#[derive(new)]
pub struct ChecksumSystem;

impl ChecksumSystem {
    pub fn build<I: Stream<Item = (Arc<Path>, Checksum)> + Unpin>(
        self,
        inputs: I,
    ) -> impl Stream<Item = impl Future<Output = (Arc<Path>, Result<(), ChecksummerError>)>>
    {
        inputs.map(|(dest, checksum)| async move {
            let buf = &mut [0u8; 8 * 1024];

            let result = match File::open(&*dest).await {
                Ok(file) => match checksum.validate(file, buf).await {
                    Ok(()) => Ok(()),
                    Err(why) => Err(ChecksummerError::Checksum(why)),
                },
                Err(why) => Err(ChecksummerError::Open(why)),
            };

            match result {
                Ok(()) => (dest, Ok(())),
                Err(why) => {
                    let _ = fs::remove_file(&*dest).await;
                    (dest, Err(why))
                }
            }
        })
    }
}
