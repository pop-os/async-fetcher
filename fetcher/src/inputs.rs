// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::{Checksum, SumStrBuf};

use async_fetcher::Source;
use bytes::BytesMut;
use futures::prelude::*;
use serde::Deserialize;
use std::sync::Arc;
use std::{convert::TryFrom, io, path::PathBuf};
use tokio::fs::File;
use tokio_util::codec::{Decoder, FramedRead};

#[derive(Debug, Error)]
pub enum InputError {
    #[error("decoder error")]
    Decoder {
        input: Box<str>,
        source: ron::de::Error,
    },
    #[error("read error")]
    Read(#[from] io::Error),
}

#[derive(Default)]
struct Inputs;

impl Decoder for Inputs {
    type Error = InputError;
    type Item = Input;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let mut read = 0;

        for line in src.as_mut().split(|&byte| byte == b'\n') {
            read += line.len() + 1;

            if line.is_empty() {
                continue;
            }

            if line[0] == b')' {
                let value = ron::de::from_bytes::<Input>(&src[..read - 1]);

                eprintln!("({})", String::from_utf8_lossy(&src[..read - 1]));

                let remaining = src.len() + 1 - read;
                src.as_mut().copy_within(read - 1.., 0);
                src.truncate(remaining);

                return value.map(Some).map_err(|source| InputError::Decoder {
                    input: String::from_utf8_lossy(src).into_owned().into(),
                    source: source.into(),
                });
            }
        }

        Ok(None)
    }
}

#[derive(Deserialize)]
struct Input {
    urls: Vec<Box<str>>,
    dest: String,
    part: Option<String>,
    sum: Option<SumStrBuf>,
}

pub fn stream(input: File) -> impl Stream<Item = (Source, Arc<Option<Checksum>>)> + Send + Unpin {
    FramedRead::new(input, Inputs::default())
        .filter_map(|result| async move {
            match result {
                Ok(input) => {
                    let mut source =
                        Source::new(Arc::from(input.urls), Arc::from(PathBuf::from(input.dest)));

                    source.set_part(input.part.map(PathBuf::from).map(Arc::from));

                    let sum = match input.sum {
                        Some(sum) => match Checksum::try_from(sum.as_ref()) {
                            Ok(sum) => Some(sum),
                            Err(why) => {
                                eprintln!("invalid checksum: {}", why);
                                None
                            }
                        },
                        None => None,
                    };

                    Some((source, Arc::new(sum)))
                }
                Err(InputError::Read(why)) => {
                    epintln!("read error: "(why));
                    None
                }
                Err(InputError::Decoder { input, source }) => {
                    epintln!(
                        "parsing error: " (source) "\n"
                        "    caused by input: " (input)
                    );

                    None
                }
            }
        })
        .boxed()
}
