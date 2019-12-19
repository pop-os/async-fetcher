use crate::*;
use async_fetcher::{Error as FetchError, FetchEvent};
use futures::{
    channel::{mpsc, oneshot},
    prelude::*,
};
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
) -> Result<(), FetchError> {
    let complete = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(MultiBar::new());

    let (tx, rx) = oneshot::channel();

    thread::spawn({
        let complete = Arc::clone(&complete);
        let progress = Arc::clone(&progress);
        || {
            let _ = tx.send(futures::executor::block_on(execute(etx, async move {
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
                                bar.finish_print(&fomat!("Already fetched "(
                                    dest.display()
                                )));
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

                        FetchEvent::Fetched(result) => {
                            if let Some(mut bar) = state.remove(&dest) {
                                let message = match result {
                                    Ok(()) => fomat!("Fetched "(dest.display())),
                                    Err(why) => {
                                        let mut message =
                                            fomat!("Failed " (dest.display()) ": " (why));

                                        let mut source = why.source();
                                        while let Some(why) = source {
                                            message.push_str(&fomat!("    caused by: "(
                                                why
                                            )));
                                            source = why.source();
                                        }

                                        message
                                    }
                                };

                                bar.finish_print(&message);
                            }
                        }

                        _ => (),
                    }
                }

                complete.store(true, Ordering::SeqCst);
            })));
        }
    });

    while !complete.load(Ordering::SeqCst) {
        let _ = progress.listen();
        thread::sleep(Duration::from_millis(1000));
    }

    let _ = progress.listen();

    futures::executor::block_on(async move { rx.await.unwrap() })
}
