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
    F: Future<Output = Result<T, Error>>,
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

    futures::pin_mut!(future);
    futures::pin_mut!(timeout);

    select(timeout, future).await.factor_first().0
}

pub fn shutdown_check(shutdown: &async_shutdown::Shutdown) -> Result<(), crate::Error> {
    if shutdown.shutdown_started() || shutdown.shutdown_completed() {
        return Err(Error::Canceled);
    }

    Ok(())
}
