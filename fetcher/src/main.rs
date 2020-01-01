#[macro_use]
extern crate fomat_macros;
#[macro_use]
extern crate futures;
#[macro_use]
extern crate thiserror;

mod inputs;
mod interactive;
mod machine;

use async_fetcher::checksum::Checksum;

use async_fetcher::{Error as FetchError, *};
use async_std::{fs::File, task};
use futures::{channel::mpsc, prelude::*};
use std::{
    io,
    num::NonZeroU16,
    os::unix::io::{AsRawFd, FromRawFd},
    path::Path,
    sync::Arc,
    time::Duration,
};
use surf::Client;

fn main() {
    better_panic::install();

    let (tx, rx) = mpsc::unbounded::<(Arc<Path>, FetchEvent)>();

    if atty::is(atty::Stream::Stdout) {
        interactive::run(tx, rx)
    } else {
        task::block_on(machine::run(tx, rx))
    }
}

async fn execute(
    etx: mpsc::UnboundedSender<(Arc<Path>, FetchEvent)>,
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
    S: Unpin + Send + Stream<Item = (Source, Option<Checksum>)> + 'static,
>(
    event_sender: mpsc::UnboundedSender<(Arc<Path>, FetchEvent)>,
    mut result_sender: mpsc::Sender<(Arc<Path>, Result<bool, FetchError>)>,
    mut checksum_sender: mpsc::Sender<(Arc<Path>, Checksum)>,
    sources: S,
) {
    let fetcher = Fetcher::new(Client::new())
        // Fetch each file in parts, using up to 4 concurrent connections per file
        .connections_per_file(NonZeroU16::new(4))
        // Pass in the event sender which events will be sent to
        .events(event_sender)
        // Configure a timeout to bail when a connection stalls for too long
        .timeout(Duration::from_secs(15))
        // Wrap it in an Arc
        .into_arc();

    let mut fetcher = FetcherSystem::new(fetcher)
        // Create a stream from the input sources
        .build(sources)
        // Concurrently fetch up to 4 at a time
        .buffer_unordered(4);

    while let Some((dest, result)) = fetcher.next().await {
        match result {
            Ok(Some(checksum)) => {
                let _ = result_sender.send((dest.clone(), Ok(true)));
                let _ = checksum_sender.send((dest, checksum)).await;
            }
            Ok(None) => {
                let _ = result_sender.send((dest, Ok(false))).await;
            }
            Err(why) => {
                let _ = result_sender.send((dest, Err(why))).await;
            }
        }
    }
}
