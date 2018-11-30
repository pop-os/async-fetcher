use failure::{Backtrace, Context, Fail};
use std::fmt::{self, Display};

#[derive(Debug)]
pub struct FetchError {
    inner: Context<String>,
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
        Display::fmt(&self.inner, f)?;
        for cause in Fail::iter_causes(self) {
            let _ = write!(f, ": {}", cause);
        }
        Ok(())
    }
}

impl From<&'static str> for FetchError {
    fn from(msg: &'static str) -> FetchError {
        FetchError {
            inner: Context::new(msg.into()),
        }
    }
}

impl From<Context<String>> for FetchError {
    fn from(inner: Context<String>) -> FetchError {
        FetchError { inner }
    }
}

impl From<Context<&'static str>> for FetchError {
    fn from(inner: Context<&'static str>) -> FetchError {
        FetchError {
            inner: inner.map(|s| s.to_string()),
        }
    }
}
