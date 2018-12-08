use digest::Digest;
use failure::Fail;
use futures::{Future, IntoFuture};
use hashing::hash_from_path;
use std::{path::Path, sync::Arc};
use FetchError;
use FetchErrorKind;

/// The state which signals that fetched file is now at the destination, and provides an optional
/// checksum comparison method.
pub struct CompletedState<T: Future<Item = (), Error = FetchError> + Send> {
    pub(crate) future:      T,
    pub(crate) destination: Arc<Path>,
}

impl<T: Future<Item = (), Error = FetchError> + Send> CompletedState<T> {
    pub fn with_destination_checksum<D: Digest>(
        self,
        checksum: Arc<str>,
    ) -> impl Future<Item = (), Error = FetchError> + Send
    {
        let destination = self.destination;
        let future = self.future;

        future.and_then(move |_| {
            hash_from_path::<D>(&destination, &checksum).map_err(|why| {
                why.context(FetchErrorKind::DestinationHash(destination.to_path_buf()))
            })?;

            Ok(())
        })
    }
}

impl<T: Future<Item = (), Error = FetchError> + Send> IntoFuture for CompletedState<T> {
    type Future = T;
    type Item = ();
    type Error = FetchError;

    fn into_future(self) -> Self::Future {
        self.future
    }
}
