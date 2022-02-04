// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::checksum::{Checksum, ChecksumError};
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

/// Generates a stream of futures that validate checksums.
///
/// The caller can choose to distribute these futures across a thread pool.
///
/// ```
/// let mut stream = checksum_stream(checksums).map(tokio::spawn).buffered(8);
/// while let Some((path, result)) = stream.next().await {
///     eprintln!("{:?} checksum result: {:?}", path, result);
/// }
/// ```
pub fn checksum_stream<I: Stream<Item = (Arc<Path>, Checksum)> + Send + Unpin + 'static>(
    inputs: I,
) -> impl Stream<Item = impl Future<Output = (Arc<Path>, Result<(), ChecksummerError>)>> {
    let buffer_pool = Pool::new(|| Box::new([0u8; 8 * 1024]));

    inputs.map(move |(dest, checksum)| {
        let pool = buffer_pool.clone();

        async {
            tokio::task::spawn_blocking(move || {
                let buf = &mut **pool.get();
                let result = validate_checksum(buf, &dest, &checksum);
                (dest, result)
            })
            .await
            .unwrap()
        }
    })
}

/// Validates the checksum of a single file
pub fn validate_checksum(
    buf: &mut [u8],
    dest: &Path,
    checksum: &Checksum,
) -> Result<(), ChecksummerError> {
    let error = match std::fs::File::open(&*dest) {
        Ok(file) => match checksum.validate(file, buf) {
            Ok(()) => return Ok(()),
            Err(why) => ChecksummerError::Checksum(why),
        },
        Err(why) => ChecksummerError::Open(why),
    };

    let _ = std::fs::remove_file(&*dest);
    Err(error)
}
