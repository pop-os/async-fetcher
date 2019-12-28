use crate::{execute, validator::validator};

use async_fetcher::FetchEvent;
use async_std::task;
use futures::{channel::mpsc, prelude::*};
use pbr::{MultiBar, Pipe, ProgressBar, Units};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

pub fn run(
    etx: mpsc::UnboundedSender<(Arc<Path>, FetchEvent)>,
    mut erx: mpsc::UnboundedReceiver<(Arc<Path>, FetchEvent)>,
) {
    let complete = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(MultiBar::new());

    let (result_tx, result_rx) = mpsc::channel(0);

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

    let checksum_validator = validator(
        result_rx,
        enclose!((progress) move |dest, result| {
            let progress = progress.clone();
            async move {
                &match result {
                    Ok(()) => epintln!((dest.display()) " was successfully validated"),
                    Err(why) => epintln!((dest.display()) " failed to validate: " [why]),
                };
            }
        }),
    );

    let handle = thread::spawn(|| {
        task::block_on(execute(etx, result_tx, fetcher, checksum_validator))
    });

    while !complete.load(Ordering::SeqCst) {
        let _ = progress.listen();
        thread::sleep(Duration::from_millis(1000));
    }

    handle.join();

    let _ = progress.listen();
}
