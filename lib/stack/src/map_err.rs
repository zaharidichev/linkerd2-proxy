use futures::{Future, Poll};
use svc;

pub fn layer<E, M>(map_err: M) -> Layer<M>
where
    M: MapErr<E>,
{
    Layer(map_err)
}

pub(super) fn stack<T, S, M>(inner: S, map_err: M) -> Service<S, M>
where
    S: svc::Service<T>,
    M: MapErr<S::Error>,
{
    Service {
        inner,
        map_err,
    }
}

pub trait MapErr<Input> {
    type Output;

    fn map_err(&self, e: Input) -> Self::Output;
}

#[derive(Clone, Debug)]
pub struct Layer<M>(M);

#[derive(Clone, Debug)]
pub struct Service<S, M> {
    inner: S,
    map_err: M,
}

pub struct ResponseFuture<F, M>(F, M);

impl<T, S, M> super::Layer<T, T, S> for Layer<M>
where
    S: svc::Service<T>,
    M: MapErr<S::Error> + Clone,
{
    type Value = <Service<S, M> as svc::Service<T>>::Response;
    type Error = <Service<S, M> as svc::Service<T>>::Error;
    type Stack = Service<S, M>;

    fn bind(&self, inner: S) -> Self::Stack {
        Service {
            inner,
            map_err: self.0.clone(),
        }
    }
}

impl<T, S, M> svc::Service<T> for Service<S, M>
where
    S: svc::Service<T>,
    M: MapErr<S::Error> + Clone,
{
    type Response = S::Response;
    type Error = M::Output;
    type Future = ResponseFuture<S::Future, M>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.inner.poll_ready().map_err(|e| self.map_err.map_err(e))
    }

    fn call(&mut self, target: T) -> Self::Future {
        ResponseFuture(self.inner.call(target), self.map_err.clone())
    }
}

impl<F, I, O> MapErr<I> for F
where
    F: Fn(I) -> O,
{
    type Output = O;
    fn map_err(&self, i: I) -> O {
        (self)(i)
    }
}

impl<F, M> Future for ResponseFuture<F, M>
where
    F: Future,
    M: MapErr<F::Error>
{
    type Item = F::Item;
    type Error = M::Output;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0.poll().map_err(|e| self.1.map_err(e))
    }
}
