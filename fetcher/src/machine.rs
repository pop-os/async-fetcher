use crate::{execute, validator::validator};

use async_fetcher::FetchEvent;
use futures::{channel::mpsc, prelude::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{self, Write},
    path::Path,
    sync::Arc,
    time::Instant,
};

pub async fn run(
    etx: mpsc::UnboundedSender<(Arc<Path>, FetchEvent)>,
    mut erx: mpsc::UnboundedReceiver<(Arc<Path>, FetchEvent)>,
) {
    let (events_tx, mut events_rx) = mpsc::channel(0);
    let (result_tx, result_rx) = mpsc::channel(0);
    let mut events_tx_ = events_tx.clone();

    let fetcher = async move {
        let mut state = HashMap::<Arc<Path>, (u64, u64, Instant)>::new();

        while let Some((dest, event)) = erx.next().await {
            let event = match event {
                FetchEvent::Progress(written) => {
                    if let Some(progress) = state.get_mut(&dest) {
                        progress.0 += written as u64;
                        let now = Instant::now();

                        if now.duration_since(progress.2).as_millis() > 250 {
                            progress.2 = now;
                            Output(
                                fomat!((dest.display())),
                                OutputEvent::Progress(progress.0, progress.1),
                            )
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }

                FetchEvent::AlreadyFetched => {
                    Output(fomat!((dest.display())), OutputEvent::AlreadyFetched)
                }

                FetchEvent::ContentLength(length) => {
                    state
                        .entry(dest.clone())
                        .and_modify(|bar| bar.1 = length)
                        .or_insert((0, length, Instant::now()));

                    Output(fomat!((dest.display())), OutputEvent::Length(length))
                }

                FetchEvent::Fetching => {
                    state.insert(dest.clone(), (0, 0, Instant::now()));
                    Output(fomat!((dest.display())), OutputEvent::Fetching)
                }

                FetchEvent::Fetched => {
                    state.remove(&dest);
                    Output(fomat!((dest.display())), OutputEvent::Fetched)
                }

                _ => continue,
            };

            if events_tx_.send(event).await.is_err() {
                break;
            }
        }
    };

    let checksum_validator = validator(result_rx, move |dest, result| {
        let mut events_tx = events_tx.clone();
        async move {
            let event = match result {
                Ok(()) => Output(fomat!((dest.display())), OutputEvent::Validated),

                Err(why) => {
                    epintln!((dest.display()) " failed to validate: " [why]);

                    Output(fomat!((dest.display())), OutputEvent::Invalid)
                }
            };

            events_tx.send(event).await;
        }
    });

    let stdout_writer = async move {
        let output = io::stdout();
        let mut output = output.lock();

        while let Some(event) = events_rx.next().await {
            match ron::ser::to_string(&event) {
                Ok(vector) => {
                    let res1 = output.write_all(vector.as_bytes());
                    let res2 = output.write_all(b"\n");

                    if let Err(why) = res1.and(res2) {
                        epintln!("failed to write serialized string to stdout: "(why));
                        return;
                    }
                }
                Err(why) => {
                    epintln!("failed to serialize: "(why));
                }
            }
        }
    };

    let _ = join!(
        stdout_writer,
        execute(etx, result_tx, fetcher, checksum_validator).boxed_local()
    );
}

#[derive(Deserialize, Serialize)]
pub struct Output(String, OutputEvent);

#[derive(Deserialize, Serialize)]
pub enum OutputEvent {
    AlreadyFetched,
    Fetched,
    Fetching,
    Invalid,
    Length(u64),
    Progress(u64, u64),
    Validated,
}
