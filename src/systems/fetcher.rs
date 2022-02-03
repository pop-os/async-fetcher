// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::{Error, FetchEvent, Fetcher, Source};

use futures::prelude::*;
use std::{path::Path, sync::Arc};
use tokio::fs;

#[derive(new, Setters)]
pub struct FetcherSystem {
    #[setters(skip)]
    client: Arc<Fetcher>,
}

impl FetcherSystem {
    pub fn build<I, T>(
        self,
        inputs: I,
    ) -> impl Stream<Item = impl Future<Output = (Arc<Path>, Result<T, Error>)>>
    where
        I: Stream<Item = (Source, T)> + Unpin + Send + 'static,
    {
        inputs.map(move |(source, extra)| {
            let fetcher = self.client.clone();

            async move {
                let Source {
                    dest, urls, part, ..
                } = source;

                fetcher.send((dest.clone(), FetchEvent::Fetching));

                let result = match part {
                    Some(part) => match fetcher.clone().request(urls, part.clone()).await {
                        Ok(()) => fs::rename(&*part, &*dest).await.map_err(Error::Rename),
                        Err(why) => Err(why),
                    },
                    None => fetcher.clone().request(urls, dest.clone()).await,
                };

                fetcher.send((dest.clone(), FetchEvent::Fetched));

                (dest, result.map(|_| extra))
            }
        })
    }
}
