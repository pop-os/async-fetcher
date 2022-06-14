// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::get::FetchLocation;
use crate::*;
use std::sync::atomic::AtomicU16;

#[allow(clippy::too_many_arguments)]
pub async fn get_many<Data: Send + Sync + 'static>(
    fetcher: Arc<Fetcher<Data>>,
    to: Arc<Path>,
    uris: Arc<[Box<str>]>,
    offset: u64,
    length: u64,
    modified: Option<HttpDate>,
    extra: Arc<Data>,
    attempts: Arc<AtomicU16>,
) -> Result<(), Error> {
    let shutdown = fetcher.shutdown.clone();
    let parent = to.parent().ok_or(Error::Parentless)?.to_owned();
    let filename = to.file_name().ok_or(Error::Nameless)?.to_owned();

    let mut buf = [0u8; 20];

    let FetchLocation { file, .. } = FetchLocation::create(to.clone(), offset != 0).await?;

    let concurrent_fetches = fetcher.connections_per_file as usize;

    let to_ = to.clone();
    let parts =
        stream::iter(range::generate(length, fetcher.max_part_size.into(), offset).enumerate())
            // Generate a future for fetching each part that a range describes.
            .map(move |(partn, (range_start, range_end))| {
                let uri = uris[partn % uris.len()].clone();

                let part_path = {
                    let mut new_filename = filename.to_os_string();
                    new_filename.push(&[".part", partn.numtoa_str(10, &mut buf)].concat());
                    parent.join(new_filename)
                };

                if part_path.exists() {
                    let _ = std::fs::remove_file(&part_path);
                }

                let fetcher = fetcher.clone();
                let to = to_.clone();
                let extra = extra.clone();
                let attempts = attempts.clone();

                let builder = match &fetcher.client {
                    Client::Isahc(_) => RequestBuilder::Http(HttpRequest::get(&*uri)),
                    #[cfg(feature = "reqwest")]
                    Client::Reqwest(client) => RequestBuilder::Reqwest(client.get(&*uri)),
                };

                async move {
                    let range = range::to_string(range_start, Some(range_end));
                    let part_path: Arc<Path> = Arc::from(part_path);

                    let request = match builder {
                        RequestBuilder::Http(inner) => {
                            RequestBuilder::Http(inner.header("range", range.as_str()))
                        }
                        #[cfg(feature = "reqwest")]
                        RequestBuilder::Reqwest(inner) => {
                            RequestBuilder::Reqwest(inner.header("range", range.as_str()))
                        }
                    };

                    crate::get(
                        fetcher.clone(),
                        request,
                        FetchLocation::create(part_path.clone(), false).await?,
                        to.clone(),
                        extra.clone(),
                        attempts.clone(),
                    )
                    .await
                }
            })
            // Ensure that only this many connections are happenning concurrently at a
            // time
            .buffered(concurrent_fetches);

    let _shutdown_token = shutdown.delay_shutdown_token();

    concatenator(file, parts, to.clone(), shutdown.clone()).await?;

    if let Some(modified) = modified {
        crate::time::update_modified(&to, modified)?;
    }

    Ok(())
}
