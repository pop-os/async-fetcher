// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::path::Path;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::AsyncWriteExt;

pub(crate) struct FetchLocation {
    pub(crate) file: tokio::fs::File,
    pub(crate) dest: Arc<Path>,
}

impl FetchLocation {
    pub async fn create(
        dest: Arc<Path>,
        length: Option<u64>,
        append: bool,
    ) -> Result<Self, crate::Error> {
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&dest)
            .await
            .map_err(Error::FileCreate)?;

        if let Some(length) = length {
            file.set_len(length).await.map_err(Error::Write)?;
        }

        Ok(Self { file, dest })
    }
}
pub(crate) async fn get<Data: Send + Sync + 'static>(
    fetcher: Arc<Fetcher<Data>>,
    request: http::request::Builder,
    file: FetchLocation,
    final_destination: Arc<Path>,
    extra: Arc<Data>,
    attempts: Arc<AtomicU16>,
) -> Result<Arc<Path>, crate::Error> {
    let shutdown = fetcher.shutdown.clone();
    let FetchLocation { mut file, dest } = file;

    let request = request.body(()).expect("failed to build request");

    let task = async move {
        let _token = match shutdown.delay_shutdown_token() {
            Ok(token) => token,
            Err(_) => return Err(Error::Canceled),
        };

        let req = async {
            fetcher
                .client
                .send_async(request)
                .await
                .map_err(Error::from)
        };

        let initial_response = crate::utils::network_interrupt(req).await?;

        if initial_response.status() == StatusCode::NOT_MODIFIED {
            return Ok::<_, crate::Error>(());
        }

        let response = &mut validate(initial_response)?;

        let mut buffer = vec![0u8; 16 * 1024];
        let mut read;
        let mut read_total = 0;

        let mut now = Instant::now();

        let update_progress = |progress: usize| {
            fetcher.send(|| {
                (
                    final_destination.clone(),
                    extra.clone(),
                    FetchEvent::Progress(progress as u64),
                )
            });
        };

        let body = response.body_mut();

        loop {
            let _shutdown = match shutdown.delay_shutdown_token() {
                Ok(token) => token,
                Err(_) => {
                    let _ = file.shutdown().await;
                    return Err(Error::Canceled);
                }
            };

            read = {
                let reader = async { body.read(&mut buffer).await.map_err(Error::Write) };

                futures::pin_mut!(reader);

                let timed = crate::utils::run_timed(fetcher.timeout, reader);
                crate::utils::network_interrupt(timed).await?
            };

            if read == 0 {
                break;
            } else {
                file.write_all(&buffer[..read])
                    .await
                    .map_err(Error::Write)?;

                read_total += read;
                if now.elapsed().as_millis() > 500 {
                    update_progress(read_total);

                    now = Instant::now();
                    read_total = 0;

                    tokio::task::yield_now().await;
                }
            }

            attempts.store(0, Ordering::SeqCst);
        }

        if read_total != 0 {
            update_progress(read_total);
        }

        let _ = file.shutdown().await;

        Ok(())
    };

    tokio::spawn(task).await.unwrap()?;

    Ok(dest)
}
