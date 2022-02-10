// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::Error;
use futures::future::select;
use std::future::Future;
use std::time::Duration;

pub async fn run_timed<F, T>(duration: Option<Duration>, future: F) -> Result<T, Error>
where
    F: Future<Output = T> + Unpin,
{
    let timeout = async move {
        match duration {
            Some(duration) => {
                tokio::time::sleep(duration).await;
                Err(Error::TimedOut)
            }
            None => futures::future::pending().await,
        }
    };

    let result = async move { Ok(future.await) };

    futures::pin_mut!(result);
    futures::pin_mut!(timeout);

    select(timeout, result).await.factor_first().0
}
