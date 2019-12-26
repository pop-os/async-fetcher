#[macro_use]
extern crate fomat_macros;
#[macro_use]
extern crate thiserror;

mod inputs;
mod interactive;
mod machine;

use async_fetcher::{Error as FetchError, FetchEvent, Fetcher, Source};
use async_std::fs::File;
use futures::{channel::mpsc, prelude::*};
use std::{
    collections::HashMap,
    error::Error as _,
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

    let result = if atty::is(atty::Stream::Stdout) {
        interactive::run(tx, rx)
    } else {
        machine::run(tx, rx)
    };

    // Check the final result of the fetcher.
    if let Err(why) = result {
        eprintln!("error occurred: {}", why);
        let mut source = why.source();
        while let Some(why) = source {
            eprintln!("    caused by: {}", why);
            source = why.source();
        }

        std::process::exit(1);
    }
}

async fn execute<E: Future<Output = ()>>(
    etx: mpsc::UnboundedSender<(Arc<Path>, FetchEvent)>,
    event_handler: E,
) -> Result<(), FetchError> {
    let stdin = io::stdin();
    let stdin = stdin.lock();

    let input_stream = inputs::stream(unsafe { File::from_raw_fd(stdin.as_raw_fd()) });

    let (_, fetch_res) =
        futures::join!(event_handler, fetcher_stream(etx, input_stream).boxed_local(),);

    fetch_res
}

/// The fetcher, which will be used to create futures for fetching files.
async fn fetcher_stream<S: Unpin + Send + Stream<Item = Source> + 'static>(
    etx: mpsc::UnboundedSender<(Arc<Path>, FetchEvent)>,
    sources: S,
) -> Result<(), FetchError> {
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
        .events(etx)
        // Configure a timeout to bail when a connection stalls for too long
        .timeout(Duration::from_secs(15))
        // Wrap it in an Arc
        .into_arc()
        // Then begin fetching from the input stream
        .from_stream(sources)
        .await
}
