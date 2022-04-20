// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::execute;

use async_fetcher::{checksum_stream, Checksum, FetchEvent};
use futures::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{self, Write},
    path::Path,
    sync::Arc,
};
use tokio::sync::mpsc;

pub async fn run(
    etx: mpsc::UnboundedSender<(Arc<Path>, Arc<Option<Checksum>>, FetchEvent)>,
    mut erx: mpsc::UnboundedReceiver<(Arc<Path>, Arc<Option<Checksum>>, FetchEvent)>,
) {
    let (events_tx, mut events_rx) = mpsc::channel(1);

    // Handles all callback events from the fetcher
    let events_tx_ = events_tx.clone();
    let fetch_events = async move {
        let mut state = HashMap::<Arc<Path>, (u64, u64)>::new();
        while let Some((dest, _checksum, event)) = erx.recv().await {
            let event = match event {
                FetchEvent::Progress(written) => {
                    if let Some(progress) = state.get_mut(&dest) {
                        progress.0 += written as u64;
                        Output(
                            fomat!((dest.display())),
                            OutputEvent::Progress(progress.0, progress.1),
                        )
                    } else {
                        continue;
                    }
                }

                FetchEvent::ContentLength(length) => {
                    state
                        .entry(dest.clone())
                        .and_modify(|bar| bar.1 = length)
                        .or_insert((0, length));

                    Output(fomat!((dest.display())), OutputEvent::Length(length))
                }

                FetchEvent::Fetching => {
                    state.insert(dest.clone(), (0, 0));
                    Output(fomat!((dest.display())), OutputEvent::Fetching)
                }

                FetchEvent::Fetched => {
                    state.remove(&dest);
                    Output(fomat!((dest.display())), OutputEvent::Fetched)
                }

                FetchEvent::Retrying => {
                    state.remove(&dest);
                    Output(fomat!((dest.display())), OutputEvent::Retrying)
                }
            };

            if events_tx_.send(event).await.is_err() {
                break;
            }
        }
    };

    // Handles all results from the fetcher.
    let events_tx_ = events_tx.clone();
    let (fetch_tx, mut fetch_rx) = mpsc::channel::<(Arc<Path>, _)>(1);
    let fetch_results = async move {
        while let Some((dest, result)) = fetch_rx.recv().await {
            let event = match result {
                Ok(false) => None,
                Ok(true) => Some(Output(fomat!((dest.display())), OutputEvent::Validating)),
                Err(why) => {
                    epintln!((dest.display()) " failed to validate: " [why]);

                    Some(Output(fomat!((dest.display())), OutputEvent::Failed))
                }
            };

            if let Some(event) = event {
                if events_tx_.send(event).await.is_err() {
                    break;
                }
            }
        }
    };

    // Handles all results from checksum operations.
    let (sum_tx, sum_rx) = mpsc::channel::<(Arc<Path>, _)>(1);
    let sum_results = async move {
        let mut stream = checksum_stream(tokio_stream::wrappers::ReceiverStream::new(sum_rx))
            // Limiting up to 32 parallel tasks at a time.
            .buffer_unordered(32);

        while let Some((dest, result)) = stream.next().await {
            let event = match result {
                Ok(()) => Output(fomat!((dest.display())), OutputEvent::Validated),

                Err(why) => {
                    epintln!((dest.display()) " failed to validate: " [why]);

                    Output(fomat!((dest.display())), OutputEvent::Invalid)
                }
            };

            if events_tx.send(event).await.is_err() {
                break;
            }
        }
    };

    // Centrally writes all events to standard out.
    let stdout_writer = async move {
        let output = io::stdout();
        let mut output = output.lock();

        while let Some(event) = events_rx.recv().await {
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
        fetch_results,
        sum_results,
        fetch_events,
        execute(etx, fetch_tx, sum_tx).boxed_local()
    );
}

#[derive(Deserialize, Serialize)]
pub struct Output(String, OutputEvent);

#[derive(Deserialize, Serialize)]
pub enum OutputEvent {
    AlreadyFetched,
    Failed,
    Fetched,
    Fetching,
    Invalid,
    Length(u64),
    Progress(u64, u64),
    Retrying,
    Validated,
    Validating,
}
