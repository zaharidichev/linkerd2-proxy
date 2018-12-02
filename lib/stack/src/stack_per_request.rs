use futures::{future, Future, Poll};
use std::fmt;

use svc;

pub trait ShouldStackPerRequest {
    fn should_stack_per_request(&self) -> bool;
}

/// A `Layer` produces a `Service` `Stack` that creates a new service for each
/// request.
#[derive(Clone, Debug)]
pub struct Layer();

/// A `Stack` that builds a new `Service` for each request it serves.
#[derive(Clone, Debug)]
pub struct Stack<M> {
    inner: M,
}

/// A `Service` that uses a new inner service for each request.
///
/// `Service` does not handle any underlying errors and it is expected that an
/// instance will not be used after an error is returned.
pub struct Service<T, S>
where
    T: ShouldStackPerRequest + Clone,
    S: svc::Service<T>,
{
    stack: S,
    target: T,
    state: State<S::Future>,
}

pub enum Error<M, S> {
    Stack(M),
    Service(S),
}

enum State<F: Future> {
    Pending(F),
    Ready(Option<F::Item>),
}

// === Layer ===

pub fn layer() -> Layer {
    Layer()
}

impl<T, N> super::Layer<T, T, N> for Layer
where
    T: ShouldStackPerRequest + Clone,
    N: svc::Service<T> + Clone,
    N::Error: fmt::Debug,
{
    type Value = <Stack<N> as svc::Service<T>>::Response;
    type Error = <Stack<N> as svc::Service<T>>::Error;
    type Stack = Stack<N>;

    fn bind(&self, inner: N) -> Self::Stack {
        Stack { inner }
    }
}

// === Stack ===

impl<T, N> svc::Service<T> for Stack<N>
where
    T: ShouldStackPerRequest + Clone,
    N: svc::Service<T> + Clone,
    N::Error: fmt::Debug,
{
    type Response = super::Either<Service<T, N>, N::Response>;
    type Error = N::Error;
    type Future = future::Either<
        future::FutureResult<Self::Response, Self::Error>,
        future::Map<N::Future, fn(N::Response) -> Self::Response>,
    >;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.inner.poll_ready()
    }

    fn call(&mut self, target: T) -> Self::Future {
        if target.should_stack_per_request() {
            let svc = Service {
                target,
                stack: self.inner.clone(),
                state: State::Ready(None),
            };
            future::Either::A(future::ok(super::Either::A(svc)))
        } else {
            let fut = self.inner.call(target);
            future::Either::B(fut.map(|s| super::Either::B(s)))
        }
    }
}

// === Service ===

impl<T, N, R> svc::Service<R> for Service<T, N>
where
    T: ShouldStackPerRequest + Clone,
    N: svc::Service<T> + Clone,
    N::Response: svc::Service<R>,
    N::Error: fmt::Debug,
{
    type Response = <N::Response as svc::Service<R>>::Response;
    type Error = Error<N::Error, <N::Response as svc::Service<R>>::Error>;
    type Future = future::MapErr<
        <N::Response as svc::Service<R>>::Future,
        fn(<N::Response as svc::Service<R>>::Error) -> Self::Error,
    >;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        loop {
            self.state = match self.state {
                State::Ready(None) => {
                    try_ready!(self.stack.poll_ready().map_err(Error::Stack));
                    let fut = self.stack.call(self.target.clone());
                    State::Pending(fut)
                }
                State::Pending(ref mut f) => {
                    let svc = try_ready!(f.poll().map_err(Error::Stack));
                    State::Ready(Some(svc))
                }
                State::Ready(Some(ref mut svc)) => {
                    return svc.poll_ready().map_err(Error::Service);
                }
            };
        }
    }

    fn call(&mut self, request: R) -> Self::Future {
        if let State::Ready(ref mut svc) = self.state {
            if let Some(ref mut svc) = svc.take() {
                return svc.call(request).map_err(Error::Service);
            }
        }

        unreachable!("poll_ready must return ready before calling this service");
    }
}
