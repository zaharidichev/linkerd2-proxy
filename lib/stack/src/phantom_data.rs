use futures::Poll;
use std::marker::PhantomData;
use svc;

pub fn layer<T, M>() -> Layer<T, M>
where
    M: svc::Service<T>,
{
    Layer(PhantomData)
}

#[derive(Clone, Debug)]
pub struct Layer<T, M>(PhantomData<fn() -> (T, M)>);

#[derive(Clone, Debug)]
pub struct Service<T, M> {
    inner: M,
    _p: PhantomData<fn() -> T>,
}

impl<T, M: svc::Service<T>> super::Layer<T, T, M> for Layer<T, M> {
    type Value = <Service<T, M> as svc::Service<T>>::Response;
    type Error = <Service<T, M> as svc::Service<T>>::Error;
    type Stack = Service<T, M>;

    fn bind(&self, inner: M) -> Self::Stack {
        Service {
            inner,
            _p: PhantomData
        }
    }
}

impl<T, M: svc::Service<T>> svc::Service<T> for Service<T, M> {
    type Response = M::Response;
    type Error = M::Error;
    type Future = M::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.inner.poll_ready()
    }

    fn call(&mut self, target: T) -> Self::Future {
        self.inner.call(target)
    }
}
