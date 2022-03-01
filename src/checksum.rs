// Copyright 2021 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use digest::{generic_array::GenericArray, Digest, OutputSizeUser};
use hex::FromHex;
use md5::Md5;
use serde::Deserialize;
use sha2::Sha256;
use std::{convert::TryFrom, io};

/// A checksum of a `Source` as a fixed-sized byte array.
#[derive(Debug, Clone)]
pub enum Checksum {
    Md5(GenericArray<u8, <Md5 as OutputSizeUser>::OutputSize>),
    Sha256(GenericArray<u8, <Sha256 as OutputSizeUser>::OutputSize>),
}

/// An error that can occur from a failed checksum validation.
#[derive(Debug, Error)]
pub enum ChecksumError {
    #[error("expected {}, found {}", _0, _1)]
    Invalid(String, String),
    #[error("I/O error encountered while reading from reader")]
    IO(#[from] io::Error),
}

/// The `&str` representation of a `Checksum`.
pub enum SumStr<'a> {
    Md5(&'a str),
    Sha256(&'a str),
}

/// The `String` representation of a `Checksum`.
#[derive(Deserialize)]
pub enum SumStrBuf {
    Md5(String),
    Sha256(String),
}

impl SumStrBuf {
    pub fn as_ref(&self) -> SumStr {
        match self {
            SumStrBuf::Md5(string) => SumStr::Md5(string.as_str()),
            SumStrBuf::Sha256(string) => SumStr::Sha256(string.as_str()),
        }
    }
}

impl<'a> TryFrom<SumStr<'a>> for Checksum {
    type Error = hex::FromHexError;

    fn try_from(input: SumStr) -> Result<Self, Self::Error> {
        match input {
            SumStr::Md5(sum) => <[u8; 16]>::from_hex(sum)
                .map(GenericArray::from)
                .map(Checksum::Md5),
            SumStr::Sha256(sum) => <[u8; 32]>::from_hex(sum)
                .map(GenericArray::from)
                .map(Checksum::Sha256),
        }
    }
}

impl Checksum {
    pub fn validate<F: std::io::Read>(
        &self,
        reader: F,
        buffer: &mut [u8],
    ) -> Result<(), ChecksumError> {
        match self {
            Checksum::Md5(sum) => checksum::<Md5, F>(reader, buffer, sum),
            Checksum::Sha256(sum) => checksum::<Sha256, F>(reader, buffer, sum),
        }
    }
}

pub(crate) fn checksum<D: Digest, F: io::Read>(
    reader: F,
    buffer: &mut [u8],
    expected: &GenericArray<u8, D::OutputSize>,
) -> Result<(), ChecksumError> {
    let result = generate_checksum::<D, F>(reader, buffer).map_err(ChecksumError::IO)?;

    if result == *expected {
        Ok(())
    } else {
        Err(ChecksumError::Invalid(
            hex::encode(expected),
            hex::encode(result),
        ))
    }
}

pub(crate) fn generate_checksum<D: Digest, F: io::Read>(
    mut reader: F,
    buffer: &mut [u8],
) -> io::Result<GenericArray<u8, D::OutputSize>> {
    let mut hasher = D::new();
    let mut read;

    loop {
        read = reader.read(buffer)?;

        if read == 0 {
            return Ok(hasher.finalize());
        }

        hasher.update(&buffer[..read]);
    }
}
