// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

#[macro_use]
extern crate fomat_macros;
#[macro_use]
extern crate futures;
#[macro_use]
extern crate thiserror;

mod inputs;
mod interactive;
mod machine;

use async_fetcher::{Error as FetchError, *};
use async_shutdown::Shutdown;
use futures::prelude::*;
use once_cell::sync::OnceCell;
use std::{
    io,
    os::unix::io::{AsRawFd, FromRawFd},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::fs::File;
use tokio::sync::mpsc;

fn shutdown_handle() -> &'static Shutdown {
    static SHUTDOWN: OnceCell<Shutdown> = OnceCell::new();
    SHUTDOWN.get_or_init(Shutdown::new)
}

#[tokio::main]
async fn main() {
    better_panic::install();

    let (tx, rx) = mpsc::unbounded_channel::<(Arc<Path>, Arc<Option<Checksum>>, FetchEvent)>();

    if atty::is(atty::Stream::Stdout) {
        interactive::run(tx, rx).await
    } else {
        machine::run(tx, rx).await
    }

    shutdown_handle().wait_shutdown_complete().await;
}

async fn execute(
    etx: mpsc::UnboundedSender<(Arc<Path>, Arc<Option<Checksum>>, FetchEvent)>,
    result_sender: mpsc::Sender<(Arc<Path>, Result<bool, FetchError>)>,
    checksum_sender: mpsc::Sender<(Arc<Path>, Checksum)>,
) {
    let stdin = io::stdin();
    let stdin = stdin.lock();

    let input_stream = inputs::stream(unsafe { File::from_raw_fd(stdin.as_raw_fd()) });

    fetcher_stream(etx, result_sender, checksum_sender, input_stream).await
}

/// The fetcher, which will be used to create futures for fetching files.
async fn fetcher_stream<
    S: Unpin + Send + Stream<Item = (Source, Arc<Option<Checksum>>)> + 'static,
>(
    event_sender: mpsc::UnboundedSender<(Arc<Path>, Arc<Option<Checksum>>, FetchEvent)>,
    result_sender: mpsc::Sender<(Arc<Path>, Result<bool, FetchError>)>,
    checksum_sender: mpsc::Sender<(Arc<Path>, Checksum)>,
    sources: S,
) {
    let fetcher = Fetcher::default()
        // Add a handle to the shutdown mechanism used by this application.
        .shutdown(crate::shutdown_handle().clone())
        // Fetch each file in parts, using up to 2 concurrent connections per file
        .connections_per_file(2)
        // Pass in the event sender which events will be sent to
        .events(event_sender)
        // Configure a timeout to bail when a connection stalls for too long
        .timeout(Duration::from_secs(15))
        // Finalize the fetcher so that it can perform fetch tasks.
        .build()
        // Build a stream that will perform fetches when polled.
        .stream_from(sources, 4);

    futures::pin_mut!(fetcher);

    while let Some((dest, checksum, result)) = fetcher.next().await {
        match result {
            Ok(()) => {
                let _ = result_sender.send((dest.clone(), Ok(true)));
                if let Some(checksum) = checksum.as_ref() {
                    let _ = checksum_sender.send((dest, checksum.clone())).await;
                }
            }
            Err(why) => {
                let _ = result_sender.send((dest, Err(why))).await;
            }
        }
    }
}
