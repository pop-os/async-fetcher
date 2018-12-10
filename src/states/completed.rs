use digest::Digest;
use failure::Fail;
use futures::{Future, IntoFuture};
use hashing::hash_from_path;
use std::{path::Path, sync::Arc};
use FetchError;
use FetchErrorKind;
use FetcherExt;

/// The state which signals that fetched file is now at the destination, and provides an optional
/// checksum comparison method.
pub struct CompletedState<T: Future<Item = Arc<Path>, Error = FetchError> + Send> {
    pub(crate) future:      T,
}

impl<T: Future<Item = Arc<Path>, Error = FetchError> + Send> CompletedState<T> {
    pub fn with_destination_checksum<D: Digest>(
        self,
        checksum: Arc<str>,
    ) -> impl Future<Item = Arc<Path>, Error = FetchError> + Send
    {
        self.future.and_then(move |destination| {
            hash_from_path::<D>(&destination, &checksum).map_err(|why| {
                why.context(FetchErrorKind::DestinationHash(destination.to_path_buf()))
            })?;

            Ok(destination)
        })
    }
}

impl<T: Future<Item = Arc<Path>, Error = FetchError> + Send> FetcherExt for CompletedState<T> {
    fn wrap_future(
        mut self,
        mut func: impl FnMut(<Self as IntoFuture>::Future) -> <Self as IntoFuture>::Future + Send
    ) -> Self {
        self.future = func(self.future);
        self
    }
}

impl<T: Future<Item = Arc<Path>, Error = FetchError> + Send> IntoFuture for CompletedState<T> {
    type Future = T;
    type Item = Arc<Path>;
    type Error = FetchError;

    fn into_future(self) -> Self::Future {
        self.future
    }
}
