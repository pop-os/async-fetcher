use digest::Digest;
use failure::ResultExt;
use filetime;
use futures::{self, Future, Stream};
use hashing::hash_from_path;
use std::{path::Path, sync::Arc};
use FetchError;

/// The state which signals that fetched file is now at the destination, and provides an optional
/// checksum comparison method.
pub struct CompletedState<T: Future<Item = (), Error = FetchError> + Send> {
    pub(crate) future: T,
    pub(crate) destination: Arc<Path>,
}

impl<T: Future<Item = (), Error = FetchError> + Send> CompletedState<T> {
    pub fn with_destination_checksum<D: Digest>(
        self,
        checksum: Arc<str>,
    ) -> impl Future<Item = (), Error = FetchError> + Send {
        let destination = self.destination;
        let future = self.future;

        future.and_then(move |_| {
            hash_from_path::<D>(&destination, &checksum).with_context(|why| {
                format!(
                    "failed to validate hash for {}: {}",
                    destination.display(),
                    why
                )
            })?;

            Ok(())
        })
    }

    /// Convert this state into the future that it owns.
    pub fn into_future(self) -> T {
        self.future
    }
}
