// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::checksum::{Checksum, ChecksumError};
use futures::prelude::*;
use poolshark::{global::Pool, Poolable};
use std::{path::Path, sync::Arc};

/// Generates a stream of futures that validate checksums.
///
/// The caller can choose to distribute these futures across a thread pool.
///
/// ```no_test
/// let mut stream = checksum_stream(checksums).map(tokio::spawn).buffered(8);
/// while let Some((path, result)) = stream.next().await {
///     eprintln!("{:?} checksum result: {:?}", path, result);
/// }
/// ```
pub fn checksum_stream<I: Stream<Item = (Arc<Path>, Checksum)> + Send + Unpin + 'static>(
    inputs: I,
) -> impl Stream<Item = impl Future<Output = (Arc<Path>, Result<(), ChecksumError>)>> {
    let buffer_pool = Arc::new(Pool::<Buf>::new(64, 1));

    inputs.map(move |(dest, checksum)| {
        let pool = buffer_pool.clone();

        async {
            tokio::task::spawn_blocking(move || {
                let mut pooled_buf = pool.take();
                let result = validate_checksum(&mut (&mut *pooled_buf).0[..], &dest, &checksum);
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
) -> Result<(), ChecksumError> {
    let error = match std::fs::File::open(&*dest) {
        Ok(file) => match checksum.validate(file, buf) {
            Ok(()) => return Ok(()),
            Err(why) => why,
        },
        Err(why) => ChecksumError::from(why),
    };

    let _ = std::fs::remove_file(&*dest);
    Err(error)
}

/// Poolable 8K buffer
struct Buf(Box<[u8; 8 * 1024]>);

impl Poolable for Buf {
    fn empty() -> Self {
        Self(Box::new([0u8; 8 * 1024]))
    }

    fn reset(&mut self) {
        self.0.fill(0);
    }

    fn capacity(&self) -> usize {
        1
    }
}
