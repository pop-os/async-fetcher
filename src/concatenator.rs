// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::Error;

use async_shutdown::Shutdown;
use futures::{Stream, StreamExt};
use std::fs::{self, File};
use std::io::copy;
use std::{path::Path, sync::Arc};

/// Accepts a stream of future file `parts` and concatenates them into the `dest` file.
pub async fn concatenator<P: 'static>(
    mut dest: File,
    mut parts: P,
    _path: Arc<Path>,
    shutdown: Shutdown,
) -> Result<(), Error>
where
    P: Stream<Item = Result<(Arc<Path>, File), Error>> + Send + Unpin,
{
    let main = async move {
        let _token = match shutdown.delay_shutdown_token() {
            Ok(token) => token,
            Err(_) => return Err(Error::Canceled),
        };

        let task = async {
            while let Some(task_result) = parts.next().await {
                crate::utils::shutdown_check(&shutdown)?;

                let (source, mut source_file) = task_result?;
                concatenate(&mut dest, source, &mut source_file)?;
            }

            Ok(())
        };

        let result = task.await;

        result
    };

    tokio::task::spawn_blocking(|| futures::executor::block_on(main))
        .await
        .unwrap()
}

/// Concatenates a part into a file.
fn concatenate(
    concatenated_file: &mut File,
    part_path: Arc<Path>,
    part_file: &mut File,
) -> Result<(), Error> {
    copy(part_file, concatenated_file).map_err(Error::Concatenate)?;

    if let Err(why) = fs::remove_file(&*part_path) {
        error!("failed to remove part file ({:?}): {}", part_path, why);
    }

    Ok(())
}
