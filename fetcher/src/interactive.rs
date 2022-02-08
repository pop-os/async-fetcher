// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::execute;

use async_fetcher::{checksum_stream, Checksum, FetchEvent};
use futures::prelude::*;
use pbr::{MultiBar, Pipe, ProgressBar, Units};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::mpsc;

pub async fn run(
    etx: mpsc::UnboundedSender<(Arc<Path>, Arc<Option<Checksum>>, FetchEvent)>,
    mut erx: mpsc::UnboundedReceiver<(Arc<Path>, Arc<Option<Checksum>>, FetchEvent)>,
) {
    let complete = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(MultiBar::new());

    let fetcher = {
        let complete = Arc::clone(&complete);
        let progress = Arc::clone(&progress);

        async move {
            let mut state = HashMap::<Arc<Path>, ProgressBar<Pipe>>::new();

            while let Some((dest, _checksum, event)) = erx.recv().await {
                match event {
                    FetchEvent::Progress(written) => {
                        if let Some(bar) = state.get_mut(&dest) {
                            bar.add(written as u64);
                        }
                    }

                    FetchEvent::AlreadyFetched => {
                        if let Some(mut bar) = state.remove(&dest) {
                            bar.finish_print(&fomat!("Already fetched "(dest.display())));
                        }
                    }

                    FetchEvent::ContentLength(total) => {
                        state
                            .entry(dest.clone())
                            .and_modify(|bar| {
                                bar.set(total);
                            })
                            .or_insert_with(|| {
                                let mut bar = progress.create_bar(total);
                                bar.set_units(Units::Bytes);
                                bar.message(&fomat!((dest.display()) ": "));
                                bar
                            });
                    }

                    FetchEvent::Fetched => {
                        if let Some(mut bar) = state.remove(&dest) {
                            bar.finish_print(&fomat!("Fetched "(dest.display())));
                        }
                    }

                    FetchEvent::Retrying => {
                        if let Some(mut bar) = state.remove(&dest) {
                            bar.finish_print(&fomat!("Retrying "(dest.display())));
                        }
                    }

                    _ => (),
                }
            }

            complete.store(true, Ordering::SeqCst);
        }
    };

    let (fetch_tx, mut fetch_rx) = mpsc::channel::<(Arc<Path>, _)>(1);
    let fetch_results = async move {
        while let Some((dest, result)) = fetch_rx.recv().await {
            match result {
                Ok(false) => epintln!((dest.display()) " was successfully fetched"),
                Ok(true) => epintln!((dest.display()) " is now validating"),
                Err(why) => epintln!((dest.display()) " failed: " [why]),
            }
        }
    };

    let (sum_tx, sum_rx) = mpsc::channel::<(Arc<Path>, _)>(1);
    let sum_results = async move {
        let mut stream = checksum_stream(tokio_stream::wrappers::ReceiverStream::new(sum_rx))
            // Limiting up to 32 parallel tasks at a time.
            .buffer_unordered(32);

        while let Some((dest, result)) = stream.next().await {
            match result {
                Ok(()) => epintln!((dest.display()) " was successfully validated"),
                Err(why) => epintln!((dest.display()) " failed to validate: " [why]),
            }
        }
    };

    let events = async move {
        join!(
            fetcher,
            fetch_results,
            sum_results,
            execute(etx, fetch_tx, sum_tx).boxed_local(),
        )
    };

    let progress_ticker = async {
        while !complete.load(Ordering::SeqCst) {
            eprintln!("update");
            let _ = progress.listen();
            tokio::time::sleep(Duration::from_millis(1000)).await;
        }
    };

    join!(events, progress_ticker);

    let _ = progress.listen();
}
