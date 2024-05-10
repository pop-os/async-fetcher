// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::Error;
use futures::future::select;
use std::future::Future;
use std::time::Duration;

pub async fn timed_interrupt<F, T>(duration: Duration, future: F) -> Result<T, Error>
where
    F: Future<Output = Result<T, Error>>,
{
    run_timed(duration, network_interrupt(future)).await
}

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

pub async fn run_timed<F, T>(duration: Duration, future: F) -> Result<T, Error>
where
    F: Future<Output = Result<T, Error>>,
{
    let timeout = async move {
        tokio::time::sleep(duration).await;
        Err(Error::TimedOut)
    };

    futures::pin_mut!(future);
    futures::pin_mut!(timeout);

    select(timeout, future).await.factor_first().0
}

pub fn shutdown_check(shutdown: &async_shutdown::ShutdownManager<()>) -> Result<(), crate::Error> {
    if shutdown.is_shutdown_triggered() || shutdown.is_shutdown_completed() {
        return Err(Error::Canceled);
    }

    Ok(())
}
