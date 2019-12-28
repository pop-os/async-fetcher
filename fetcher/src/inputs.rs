use crate::checksum::{Checksum, SumString};

use async_fetcher::Source;
use async_std::fs::File;
use bytes::BytesMut;
use futures::prelude::*;
use futures_codec::{Decoder, FramedRead};
use serde::Deserialize;
use std::{convert::TryFrom, io, path::PathBuf};

#[derive(Debug, Error)]
pub enum InputError {
    #[error("decoder error")]
    Decoder { input: Box<str>, source: ron::de::Error },
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

                let remaining = src.len() - read;
                src.as_mut().copy_within(read.., 0);
                src.truncate(remaining);

                return value.map(Some).map_err(|source| InputError::Decoder {
                    input: String::from_utf8_lossy(&src).into_owned().into(),
                    source,
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
    sum:  Option<SumString>,
}

pub fn stream(
    input: File,
) -> impl Stream<Item = (Source, Option<Checksum>)> + Send + Unpin {
    FramedRead::new(input, Inputs::default())
        .filter_map(|result| async move {
            match result {
                Ok(input) => {
                    let mut source = Source::new(input.urls, PathBuf::from(input.dest));

                    if let Some(part) = input.part {
                        source = source.part(PathBuf::from(part));
                    }

                    let sum = match input.sum {
                        Some(sum) => match Checksum::try_from(sum) {
                            Ok(sum) => Some(sum),
                            Err(why) => {
                                eprintln!("invalid checksum: {}", why);
                                None
                            }
                        },
                        None => None,
                    };

                    Some((source, sum))
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
