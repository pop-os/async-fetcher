// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::Error;

use async_shutdown::Shutdown;
use futures::{Stream, StreamExt};
use std::{path::Path, sync::Arc};
use tokio::fs::{self, File};
use tokio::io::{copy, AsyncWriteExt};

/// Accepts a stream of future file `parts` and concatenates them into the `dest` file.
pub async fn concatenator<P: 'static>(
    mut dest: File,
    mut parts: P,
    path: Arc<Path>,
    shutdown: Shutdown,
) -> Result<(), Error>
where
    P: Stream<Item = Result<Arc<Path>, Error>> + Send + Unpin,
{
    let task = tokio::spawn(async move {
        let _token = match shutdown.delay_shutdown_token() {
            Ok(token) => token,
            Err(_) => return Err(Error::Canceled),
        };

        let task = async {
            while let Some(task_result) = parts.next().await {
                crate::utils::shutdown_check(&shutdown)?;

                let part_path: Arc<Path> = task_result?;
                debug!("CONCAT {:?}", part_path.display());
                concatenate(&mut dest, part_path).await?;

                crate::utils::shutdown_check(&shutdown)?;
            }

            crate::utils::shutdown_check(&shutdown)?;

            Ok(())
        };

        let result = task.await;

        let _ = dest.shutdown().await;
        debug!("CONCAT flush {}", path.display());

        result
    });

    task.await.unwrap()
}

/// Concatenates a part into a file.
async fn concatenate(concatenated_file: &mut File, part_path: Arc<Path>) -> Result<(), Error> {
    let mut file = File::open(&*part_path)
        .await
        .map_err(|why| Error::OpenPart(part_path.clone(), why))?;

    copy(&mut file, concatenated_file)
        .await
        .map_err(Error::Concatenate)?;

    if let Err(why) = fs::remove_file(&*part_path).await {
        error!("failed to remove part file ({:?}): {}", part_path, why);
    }

    Ok(())
}
