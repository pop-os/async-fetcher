// Copyright 2021 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use numtoa::NumToA;
use std::num::NonZeroU64;

pub fn generate(mut length: u64, max_part_size: NonZeroU64) -> impl Iterator<Item = (u64, u64)> {
    let mut offset = 0u64;

    std::iter::from_fn(move || {
        if length == 0 {
            return None;
        }

        let next;
        if length > max_part_size.get() {
            next = (offset, offset + max_part_size.get() - 1);
            offset += max_part_size.get();
            length -= max_part_size.get()
        } else {
            next = (offset, offset + length - 1);
            length = 0;
        }

        Some(next)
    })
}

pub(crate) fn to_string(from: u64, to: u64) -> String {
    let mut from_a = [0u8; 20];
    let mut to_a = [0u8; 20];
    [
        "bytes=",
        from.numtoa_str(10, &mut from_a),
        "-",
        to.numtoa_str(10, &mut to_a),
    ]
    .concat()
}
