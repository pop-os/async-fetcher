mod completed;
mod fetched;
mod response;

pub use futures::IntoFuture;
pub use self::{completed::*, fetched::*, response::*};

pub trait FetcherExt: IntoFuture {
    /// Enables the caller to manipulate the inner future of this state, making it possible to
    /// append and prepand futures of their own to the generated future.
    fn wrap_future(
        self,
        func: impl FnMut(<Self as IntoFuture>::Future) -> <Self as IntoFuture>::Future + Send
    ) -> Self;
}
