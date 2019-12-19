use async_fetcher::{FetchEvent, Fetcher, Source};
use futures::{
    channel::{mpsc, oneshot},
    prelude::*,
    stream,
};
use std::{error::Error as _, path::Path, sync::Arc, time::Duration};
use surf::Client;

fn main() {
    futures::executor::block_on(async {
        main_().await;
    });
}

async fn main_() {
    better_panic::install();

    let urls = &[
        ("http://apt.pop-os.org/staging/master/dists/bionic/main/binary-amd64/Packages.gz", "cache/Packages.gz"),
        ("http://apt.pop-os.org/staging/master/dists/bionic/main/source/Sources.gz", "cache/Sources.gz"),
    ];

    std::fs::create_dir_all("cache").unwrap();

    let (tx, rx) = oneshot::channel();
    let (etx, mut erx) = mpsc::unbounded();

    // A future for handling events sent by the fetcher.
    //
    // This is useful for tracking the progress of a download.
    let event_handler = async move {
        while let Some(event) = erx.next().await {
            match event {
                FetchEvent::AlreadyFetched(dest) => {
                    println!("{:?} was already fetched", dest);
                }
                FetchEvent::ContentLength(dest, total) => {
                    println!("{:?}: total {}", dest, total);
                }
                FetchEvent::Fetched(dest, result) => match result {
                    Ok(()) => println!("Fetched {:?}", dest),
                    Err(why) => {
                        eprintln!("Fetching {:?} failed: {}", dest, why);
                        let mut source = why.source();
                        while let Some(why) = source {
                            eprintln!("    caused by: {}", why);
                            source = why.source();
                        }
                    }
                },
                FetchEvent::Fetching(dest) => {
                    println!("Fetching {:?}", dest);
                }
                FetchEvent::Progress(_dest, _written) => {}
                FetchEvent::PartFetching(dest, part) => {
                    println!("fetching part {} of {:?}", part, dest);
                }
                FetchEvent::PartFetched(dest, part) => {
                    println!("fetched part {} of {:?}", part, dest);
                }
            }
        }
    };

    // The fetcher, which will be used to create futures for fetching files.
    let fetcher = Arc::new(
        Fetcher::new(Client::new())
            .concurrent_files(4)
            .connections_per_file(4)
            .events(etx)
            .timeout(Duration::from_secs(15)),
    );

    // The future for fetching each file from the provided source stream.
    let fetcher = async move {
        let iter = urls.into_iter().map(|(url, dest)| Source {
            urls: Arc::from(vec![Box::from(*url)]),
            dest: Arc::from(Path::new(dest)),
            part: None,
        });

        let _ = tx.send(fetcher.from_stream(stream::iter(iter)).await);
    };

    // Wait until both of these futures are resolved.
    future::join(event_handler, fetcher).await;

    // Check the final result of the fetcher.
    if let Ok(Err(why)) = rx.await {
        eprintln!("error occurred: {}", why);
        let mut source = why.source();
        while let Some(why) = source {
            eprintln!("    caused by: {}", why);
            source = why.source();
        }
    }
}
