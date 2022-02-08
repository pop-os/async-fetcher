// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use std::path::Path;
use std::sync::Arc;

/// Information about a source being fetched.
#[derive(Debug)]
pub struct Source {
    /// URLs whereby the file can be found.
    pub urls: Arc<[Box<str>]>,

    /// Where the file shall ultimately be fetched to.
    pub dest: Arc<Path>,

    /// Where partial files should be stored.
    pub part: Option<Arc<Path>>,
}

impl Source {
    pub fn builder(dest: Arc<Path>, url: Box<str>) -> SourceBuilder {
        SourceBuilder::new(dest, url)
    }

    pub fn new(urls: Arc<[Box<str>]>, dest: Arc<Path>) -> Self {
        Self {
            urls,
            dest,
            part: None,
        }
    }

    /// Sets the partial destination of a source.
    pub fn set_part(&mut self, part: Option<Arc<Path>>) {
        self.part = part;
    }
}

/// Constructs a `Source`.
pub struct SourceBuilder {
    urls: Vec<Box<str>>,
    dest: Arc<Path>,
    part: Option<Arc<Path>>,
}

impl SourceBuilder {
    pub fn new(dest: Arc<Path>, url: Box<str>) -> Self {
        SourceBuilder {
            dest,
            urls: vec![url],
            part: None,
        }
    }

    /// A mirror where the source can be located.
    pub fn append_url(mut self, url: Box<str>) -> Self {
        self.urls.push(url);
        self
    }

    /// A partial destination for a source.
    pub fn partial(mut self, part: Arc<Path>) -> Self {
        self.part = Some(part);
        self
    }

    pub fn build(self) -> Source {
        Source {
            urls: Arc::from(self.urls),
            dest: self.dest,
            part: self.part,
        }
    }
}
