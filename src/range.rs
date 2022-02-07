// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use numtoa::NumToA;

pub fn generate(
    mut length: u64,
    max_part_size: u64,
    mut offset: u64,
) -> impl Iterator<Item = (u64, u64)> + Send + 'static {
    length -= offset;

    std::iter::from_fn(move || {
        if length == 0 {
            return None;
        }

        let next;
        if length > max_part_size {
            next = (offset, offset + max_part_size - 1);
            offset += max_part_size;
            length -= max_part_size
        } else {
            next = (offset, offset + length - 1);
            length = 0;
        }

        Some(next)
    })
}

pub(crate) fn to_string(from: u64, to: Option<u64>) -> String {
    let mut from_a = [0u8; 20];
    let mut to_a = [0u8; 20];
    [
        "bytes=",
        from.numtoa_str(10, &mut from_a),
        "-",
        if let Some(to) = to {
            to.numtoa_str(10, &mut to_a)
        } else {
            ""
        },
    ]
    .concat()
}
