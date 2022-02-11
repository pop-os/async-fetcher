use std::{
    collections::hash_map::DefaultHasher,
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
