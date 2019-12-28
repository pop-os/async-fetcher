#[macro_use]
extern crate enclose;
#[macro_use]
extern crate fomat_macros;
#[macro_use]
extern crate futures;
#[macro_use]
extern crate thiserror;

mod checksum;
mod inputs;
mod interactive;
mod machine;
mod validator;

use crate::checksum::Checksum;

use async_fetcher::{Error as FetchError, FetchEvent, Fetcher, Source};
use async_std::{fs::File, task};
use futures::{channel::mpsc, prelude::*};
use std::{
    io,
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

async fn execute<E: Future<Output = ()>, R: Future<Output = ()>>(
    etx: mpsc::UnboundedSender<(Arc<Path>, FetchEvent)>,
    result_tx: mpsc::Sender<(Arc<Path>, Result<Option<Checksum>, FetchError>)>,
    event_handler: E,
    result_handler: R,
) {
    let stdin = io::stdin();
    let stdin = stdin.lock();

    let input_stream = inputs::stream(unsafe { File::from_raw_fd(stdin.as_raw_fd()) });
    let fetcher_stream = fetcher_stream(etx, result_tx, input_stream).boxed_local();

    let _ = futures::join!(event_handler, result_handler, fetcher_stream);
}

/// The fetcher, which will be used to create futures for fetching files.
async fn fetcher_stream<
    S: Unpin + Send + Stream<Item = (Source, Option<Checksum>)> + 'static,
>(
    event_sender: mpsc::UnboundedSender<(Arc<Path>, FetchEvent)>,
    result_sender: mpsc::Sender<(Arc<Path>, Result<Option<Checksum>, FetchError>)>,
    sources: S,
) {
    Fetcher::new(Client::new())
        // Download up to 4 files concurrently
        .concurrent_files(4)
        // Fetch each file in parts, using up to 4 concurrent connections per file
        .connections_per_file(4)
        // Define that a part must be at least this size
        .min_part_size(1 * 1024)
        // Define that a part must be no larger than this size
        .max_part_size(4 * 1024)
        // Pass in the event sender which events will be sent to
        .events(event_sender)
        // Configure a timeout to bail when a connection stalls for too long
        .timeout(Duration::from_secs(15))
        // Wrap it in an Arc
        .into_arc()
        // Then begin fetching from the input stream
        .from_stream(sources, move |path, result| {
            let mut tx = result_sender.clone();
            async move {
                tx.send((path, result)).await;
                true
            }
        })
        // Commit the awaiter
        .await;
}
