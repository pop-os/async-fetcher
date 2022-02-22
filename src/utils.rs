// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::Error;
use futures::future::select;
use std::future::Future;
use std::time::Duration;

pub async fn network_interrupt<T>(
    future: impl Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    let ifaces_changed = async {
        crate::iface::watch_change().await;
        Err(Error::NetworkChanged)
    };

    futures::pin_mut!(ifaces_changed);
    futures::pin_mut!(future);

    select(ifaces_changed, future).await.factor_first().0
}

pub async fn run_timed<F, T>(duration: Option<Duration>, future: F) -> Result<T, Error>
where
    F: Future<Output = T>,
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

pub async fn shutdown_cancel<F: Future<Output = Result<(), crate::Error>>>(
    shutdown: &async_shutdown::Shutdown,
    future: F,
) -> Result<(), crate::Error> {
    let canceled = shutdown.wait_shutdown_triggered();

    futures::pin_mut!(future);
    futures::pin_mut!(canceled);

    use futures::future::Either;

    match futures::future::select(canceled, future).await {
        Either::Left((_, _)) => Err(crate::Error::Canceled),
        Either::Right((result, _)) => result,
    }
}
