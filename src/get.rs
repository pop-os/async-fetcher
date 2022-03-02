// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub(crate) struct FetchLocation {
    pub(crate) file: std::fs::File,
    pub(crate) dest: Arc<Path>,
}

impl FetchLocation {
    pub async fn create(
        dest: Arc<Path>,
        length: Option<u64>,
        append: bool,
    ) -> Result<Self, crate::Error> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&dest)
            .map_err(Error::FileCreate)?;

        if let Some(length) = length {
            file.set_len(length).map_err(Error::Write)?;
        }

        Ok(Self { file, dest })
    }
}
pub(crate) async fn get<Data: Send + Sync + 'static>(
    fetcher: Arc<Fetcher<Data>>,
    request: reqwest::RequestBuilder,
    file: FetchLocation,
    final_destination: Arc<Path>,
    extra: Arc<Data>,
    attempts: Arc<AtomicU16>,
) -> Result<Arc<Path>, crate::Error> {
    crate::utils::shutdown_check(&fetcher.shutdown)?;

    let shutdown = fetcher.shutdown.clone();
    let FetchLocation { mut file, dest } = file;

    let request = request.build().expect("failed to build request");

    let task = async move {
        let _token = match shutdown.delay_shutdown_token() {
            Ok(token) => token,
            Err(_) => return Err(Error::Canceled),
        };

        let req = async { fetcher.client.execute(request).await.map_err(Error::from) };

        let initial_response = crate::utils::network_interrupt(req).await?;

        if initial_response.status() == StatusCode::NOT_MODIFIED {
            return Ok::<Arc<Path>, crate::Error>(dest);
        }

        let mut response = validate(initial_response)?.bytes_stream();

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

        let fetch_loop = async {
            loop {
                if shutdown.shutdown_started() || shutdown.shutdown_completed() {
                    let _ = file.flush();
                    return Err(Error::Canceled);
                }

                let chunk = {
                    let reader = async { Ok(response.next().await) };

                    futures::pin_mut!(reader);

                    let timed = crate::utils::run_timed(fetcher.timeout, reader);
                    match crate::utils::network_interrupt(timed).await {
                        Ok(chunk) => chunk,
                        Err(why) => {
                            let _ = file.flush();
                            debug!("GET {} interrupted", dest.display());
                            return Err(why);
                        }
                    }
                };

                match chunk {
                    Some(chunk) => {
                        let bytes = chunk.map_err(Error::Read)?;

                        file.write_all(&*bytes).map_err(Error::Write)?;

                        read_total += bytes.len();
                        if now.elapsed().as_millis() > 500 {
                            update_progress(read_total);

                            now = Instant::now();
                            read_total = 0;
                        }
                    }
                    None => break,
                }

                attempts.store(0, Ordering::SeqCst);
            }

            Ok(())
        };

        let result = fetch_loop.await;

        let _ = file.flush();

        if result.is_ok() && read_total != 0 {
            update_progress(read_total);
        }

        result.map(|_| dest)
    };

    tokio::spawn(task).await.unwrap()
}
