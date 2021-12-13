// Copyright 2021 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::execute;

use async_fetcher::{ChecksumSystem, FetchEvent};
use futures::{channel::mpsc, prelude::*};
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

pub async fn run(
    etx: mpsc::UnboundedSender<(Arc<Path>, FetchEvent)>,
    mut erx: mpsc::UnboundedReceiver<(Arc<Path>, FetchEvent)>,
) {
    let complete = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(MultiBar::new());

    let fetcher = {
        let complete = Arc::clone(&complete);
        let progress = Arc::clone(&progress);

        async move {
            let mut state = HashMap::<Arc<Path>, ProgressBar<Pipe>>::new();

            while let Some((dest, event)) = erx.next().await {
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

                    _ => (),
                }
            }

            complete.store(true, Ordering::SeqCst);
        }
    };

    let (fetch_tx, mut fetch_rx) = mpsc::channel::<(Arc<Path>, _)>(0);
    let fetch_results = async move {
        while let Some((dest, result)) = fetch_rx.next().await {
            match result {
                Ok(false) => epintln!((dest.display()) " was successfully fetched"),
                Ok(true) => epintln!((dest.display()) " is now validating"),
                Err(why) => epintln!((dest.display()) " failed: " [why]),
            }
        }
    };

    let (sum_tx, sum_rx) = mpsc::channel::<(Arc<Path>, _)>(0);
    let sum_results = async move {
        let mut stream = ChecksumSystem::new()
            .build(sum_rx)
            // Distribute each checksum future across a thread pool.
            .map(|future| blocking::unblock(|| async_io::block_on(future)))
            // Limiting up to 32 concurrent tasks at a time.
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
            let _ = progress.listen();
            async_io::Timer::after(Duration::from_millis(1000)).await;
        }
    };

    join!(events, progress_ticker);

    let _ = progress.listen();
}
