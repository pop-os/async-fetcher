use crate::checksum::{Checksum, ChecksumError};
use async_std::fs::{self, File};
use futures::prelude::*;
use remem::Pool;
use std::{io, path::Path, sync::Arc};

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
        let buffer_pool = Pool::new(|| Box::new([0u8; 8 * 1024]));

        inputs.map(move |(dest, checksum)| {
            let pool = buffer_pool.clone();

            async move {
                let buf = &mut **pool.get();

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
            }
        })
    }
}
