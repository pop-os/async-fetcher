use numtoa::NumToA;

pub(crate) fn calc(length: u64, nthreads: u64, part: u64) -> Option<(u64, u64)> {
    if length == 0 || nthreads == 1 {
        None
    } else {
        let section = length / nthreads;
        let from = part * section;
        let to = if part + 1 == nthreads { length } else { (part + 1) * section } - 1;
        Some((from, to))
    }
}

pub(crate) fn to_string(from: u64, to: u64) -> String {
    let mut from_a = [0u8; 20];
    let mut to_a = [0u8; 20];
    ["bytes=", from.numtoa_str(10, &mut from_a), "-", to.numtoa_str(10, &mut to_a)]
        .concat()
}
