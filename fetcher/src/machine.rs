use crate::*;
use async_fetcher::{Error as FetchError, FetchEvent};
use futures::{channel::mpsc, prelude::*};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Write},
    path::Path,
    sync::Arc,
    time::Instant,
};

pub fn run(
    etx: mpsc::UnboundedSender<(Arc<Path>, FetchEvent)>,
    mut erx: mpsc::UnboundedReceiver<(Arc<Path>, FetchEvent)>,
) -> Result<(), FetchError> {
    let output = io::stdout();
    let mut output = output.lock();

    futures::executor::block_on(execute(etx, async move {
        let mut state = HashMap::<Arc<Path>, (u64, u64, Instant)>::new();
        while let Some((dest, event)) = erx.next().await {
            let event = match event {
                FetchEvent::Progress(written) => {
                    if let Some(progress) = state.get_mut(&dest) {
                        progress.0 += written as u64;
                        let now = Instant::now();

                        if now.duration_since(progress.2).as_millis() > 250 {
                            progress.2 = now;
                            Output {
                                dest:  fomat!((dest.display())),
                                event: OutputEvent::Progress(progress.0, progress.1),
                            }
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }

                FetchEvent::AlreadyFetched => Output {
                    dest:  fomat!((dest.display())),
                    event: OutputEvent::AlreadyFetched,
                },

                FetchEvent::ContentLength(length) => {
                    state
                        .entry(dest.clone())
                        .and_modify(|bar| bar.1 = length)
                        .or_insert((0, length, Instant::now()));

                    Output {
                        dest:  fomat!((dest.display())),
                        event: OutputEvent::Length(length),
                    }
                }

                FetchEvent::Fetching => {
                    state.insert(dest.clone(), (0, 0, Instant::now()));
                    dbg!(&state);
                    Output {
                        dest:  fomat!((dest.display())),
                        event: OutputEvent::Fetching,
                    }
                }

                FetchEvent::Fetched(result) => {
                    state.remove(&dest);
                    Output {
                        dest:  fomat!((dest.display())),
                        event: {
                            match result {
                                Ok(()) => OutputEvent::Fetched,
                                Err(why) => {
                                    let mut message =
                                        fomat!("Failed " (dest.display()) ": " (why));

                                    let mut source = why.source();
                                    while let Some(why) = source {
                                        message.push_str(&fomat!("    caused by: "(why)));
                                        source = why.source();
                                    }

                                    OutputEvent::FetchError(message.into())
                                }
                            }
                        },
                    }
                }

                _ => continue,
            };

            match ron::ser::to_string(&event) {
                Ok(vector) => {
                    let res1 = output.write_all(vector.as_bytes());
                    let res2 = output.write_all(b"\n");

                    if let Err(why) = res1.and(res2) {
                        eprintln!("failed to write serialized string to stdout: {}", why);
                        return;
                    }
                }
                Err(why) => {
                    eprintln!("failed to serialize: {}", why);
                }
            }
        }
    }))
}

#[derive(Deserialize, Serialize)]
pub struct Output {
    dest:  String,
    event: OutputEvent,
}

#[derive(Deserialize, Serialize)]
pub enum OutputEvent {
    AlreadyFetched,
    Length(u64),
    Fetched,
    FetchError(Box<str>),
    Fetching,
    Progress(u64, u64),
}
