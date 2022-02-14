use std::{
    collections::hash_map::DefaultHasher,
    future::Future,
    hash::{Hash, Hasher},
};

/// Get the current state of network connections as a hash.
pub fn state() -> u64 {
    let mut hash = DefaultHasher::new();

    if let Ok(ifaces) = ifaces::Interface::get_all() {
        for iface in ifaces {
            iface.addr.hash(&mut hash);
        }
    }

    hash.finish()
}

/// Future which exits when the network state has changed.
pub async fn watch_change() {
    let current = state();

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let new = state();
        if new != current {
            break;
        }
    }
}

/// Re-attempts a request on network changes.
pub async fn reconnect_on_change<Func, Fut, Res, Retry, Cont>(func: Func, cont: Retry) -> Res
where
    Func: Fn() -> Fut,
    Fut: Future<Output = Res>,
    Retry: Fn() -> Cont,
    Cont: Future<Output = Option<Res>>,
{
    loop {
        let changed = watch_change();
        let future = func();

        futures::pin_mut!(future);
        futures::pin_mut!(changed);

        use futures::future::Either;

        match futures::future::select(future, changed).await {
            Either::Left((result, _)) => break result,
            Either::Right(_) => {
                if let Some(result) = cont().await {
                    break result;
                }
            }
        }
    }
}
