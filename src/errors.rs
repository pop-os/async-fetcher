use failure::{Backtrace, Context, Fail};
use std::fmt::{self, Display};
use std::path::PathBuf;

/// An error that occurred when fetching a file with this library.
#[derive(Debug)]
pub struct FetchError {
    inner: Context<FetchErrorKind>,
}

impl FetchError {
    pub fn kind(&self) -> &FetchErrorKind {
        self.inner.get_context()
    }
}

#[derive(Clone, Debug, Fail, PartialEq)]
pub enum FetchErrorKind {
    #[fail(display = "async fetch for {:?} failed", _0)]
    Fetch(PathBuf),
    #[fail(display = "failed to validate destination hash for {:?}", _0)]
    DestinationHash(PathBuf),
    #[fail(display = "failed to remove {:?}", _0)]
    Remove(PathBuf),
    #[fail(display = "failed to rename {:?} to {:?}", src, dst)]
    Rename { src: PathBuf, dst: PathBuf },
    #[fail(display = "failed to copy {:?} to {:?}", src, dst)]
    Copy { src: PathBuf, dst: PathBuf },
    #[fail(display = "failed to open file ({:?})", _0)]
    Open(PathBuf),
    #[fail(display = "failed to create file ({:?})", _0)]
    Create(PathBuf),
    #[fail(display = "failed to set file times for {:?}", _0)]
    FileTime(PathBuf),
    #[fail(display = "failed to create destination file ({:?})", _0)]
    CreateDestination(PathBuf),
    #[fail(display = "failed to construct writer for destination ({:?})", _0)]
    WriterConstruction(PathBuf),
    #[fail(display = "chunk request")]
    ChunkRequest,
    #[fail(display = "chunk write")]
    ChunkWrite,
    #[fail(display = "failed to set length of file")]
    LengthSet,
    #[fail(display = "failed to flush a file")]
    Flush,
    #[fail(display = "failed to copy file descriptor")]
    FdCopy,
}

impl Fail for FetchError {
    fn cause(&self) -> Option<&Fail> {
        self.inner.cause()
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        self.inner.backtrace()
    }
}

impl Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.inner)?;
        Fail::iter_causes(self)
            .map(|cause| write!(f, ": {}", cause))
            .collect()
    }
}

impl<K: Into<FetchErrorKind>> From<K> for FetchError {
    fn from(kind: K) -> FetchError {
        FetchError {
            inner: Context::new(kind.into()),
        }
    }
}

impl From<Context<FetchErrorKind>> for FetchError {
    fn from(inner: Context<FetchErrorKind>) -> FetchError {
        FetchError { inner: inner }
    }
}
