#[macro_use]
extern crate futures;
//#[macro_use]
//extern crate log;
extern crate linkerd2_never as never;
extern crate tower_service as svc;

pub mod either;
pub mod layer;
mod map_err;
pub mod map_target;
pub mod phantom_data;
pub mod stack_per_request;
pub mod watch;

pub use self::either::Either;
pub use self::layer::Layer;

pub mod shared {
    use futures::{future, Poll};
    use never::Never;
    use svc;

    pub fn new<V: Clone>(v: V) -> Shared<V> {
        Shared(v)
    }

    /// Implements `Stack<T>` for any `T` by cloning a `V`-typed value.
    #[derive(Clone, Debug)]
    pub struct Shared<V: Clone>(V);

    impl<T, V: Clone> svc::Service<T> for Shared<V> {
        type Response = V;
        type Error = Never;
        type Future = future::FutureResult<V, Never>;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            Ok(().into())
        }

        fn call(&mut self, _: T) -> Self::Future {
            future::ok(self.0.clone())
        }
    }
}
