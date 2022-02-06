// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use super::*;
use std::path::Path;
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
    modified: &mut Option<HttpDate>,
    extra: Arc<Data>,
) -> Result<Arc<Path>, crate::Error> {
    let FetchLocation { mut file, dest } = file;

    let request = request.body(()).expect("failed to build request");

    let initial_response = if let Some(duration) = fetcher.timeout {
        timed(
            duration,
            Box::pin(async {
                fetcher
                    .client
                    .send_async(request)
                    .await
                    .map_err(Error::from)
            }),
        )
        .await??
    } else {
        fetcher.client.send_async(request).await?
    };

    if initial_response.status() == StatusCode::NOT_MODIFIED {
        return Ok(dest);
    }

    let response = &mut validate(initial_response)?;

    if modified.is_none() {
        *modified = response.last_modified();
    }

    let mut buffer = vec![0u8; 64 * 1024];
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

    loop {
        if fetcher.cancelled() {
            return Err(Error::Cancelled);
        }

        read = {
            let reader = async {
                response
                    .body_mut()
                    .read(&mut buffer)
                    .await
                    .map_err(Error::Write)
            };

            futures::pin_mut!(reader);

            match fetcher.timeout {
                Some(duration) => timed(duration, reader).await??,
                None => reader.await?,
            }
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
            }
        }
    }

    if read_total != 0 {
        update_progress(read_total);
    }

    let _ = file.flush().await;

    Ok(dest)
}
