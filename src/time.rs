// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::Error;
use filetime::FileTime;
use httpdate::HttpDate;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn date_as_timestamp(date: HttpDate) -> u64 {
    SystemTime::from(date)
        .duration_since(UNIX_EPOCH)
        .expect("time backwards")
        .as_secs()
}

pub fn update_modified(to: &Arc<Path>, modified: HttpDate) -> Result<(), Error> {
    let filetime = FileTime::from_unix_time(date_as_timestamp(modified) as i64, 0);
    filetime::set_file_times(&to, filetime, filetime)
        .map_err(|why| Error::FileTime(to.clone(), why))
}
