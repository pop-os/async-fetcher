// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub(crate) struct FetchLocation {
    pub(crate) file: std::fs::File,
    pub(crate) dest: Arc<Path>,
}

impl FetchLocation {
    pub async fn create(dest: Arc<Path>, append: bool) -> Result<Self, crate::Error> {
        let mut builder = std::fs::OpenOptions::new();

        builder.create(true).write(true).read(true);

        if append {
            builder.append(true);
        } else {
            builder.truncate(true);
        }

        let file = builder.open(&dest).map_err(Error::FileCreate)?;

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
) -> Result<(Arc<Path>, File), crate::Error> {
    crate::utils::shutdown_check(&fetcher.shutdown)?;

    let shutdown = fetcher.shutdown.clone();
    let FetchLocation { mut file, dest } = file;

    let request = request.body(()).expect("failed to build request");

    let main = async move {
        let _token = match shutdown.delay_shutdown_token() {
            Ok(token) => token,
            Err(_) => return Err(Error::Canceled),
        };

        let req = async { fetcher.client.send_async(request).await.map_err(Error::from) };

        let initial_response = crate::utils::timed_interrupt(Duration::from_secs(3), req).await?;

        if initial_response.status() == StatusCode::NOT_MODIFIED {
            return Ok::<_, crate::Error>((dest, file));
        }

        let mut response = validate(initial_response)?.into_body();

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

        let mut buffer = vec![0u8; 8192];

        let fetch_loop = async {
            loop {
                if shutdown.shutdown_started() || shutdown.shutdown_completed() {
                    return Err(Error::Canceled);
                }

                let bytes_read = async { response.read(&mut buffer).await.map_err(Error::Read) };

                let read = match fetcher.timeout {
                    Some(timeout) => crate::utils::timed_interrupt(timeout, bytes_read).await,
                    None => crate::utils::network_interrupt(bytes_read).await,
                }?;

                if read == 0 {
                    break;
                }

                read_total += read;

                file.write_all(&buffer[..read]).map_err(Error::Write)?;

                if now.elapsed().as_millis() as u64 > fetcher.progress_interval {
                    update_progress(read_total);

                    now = Instant::now();
                    read_total = 0;
                }

                attempts.store(0, Ordering::SeqCst);
            }

            Ok(())
        };

        let fetch_result = fetch_loop.await;
        let seek_result = file.seek(SeekFrom::Start(0)).map_err(Error::Write);

        let result = fetch_result.and(seek_result);

        if result.is_ok() && read_total != 0 {
            update_progress(read_total);
        }

        result.map(|_| (dest, file))
    };

    tokio::task::spawn_blocking(|| futures::executor::block_on(main))
        .await
        .unwrap()
}
