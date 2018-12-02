use futures::Poll;
use svc;

pub fn layer<T, M>(map_target: M) -> Layer<M>
where
    M: MapTarget<T>,
{
    Layer(map_target)
}

pub trait MapTarget<T> {
    type Target;

    fn map_target(&self, t: T) -> Self::Target;
}

#[derive(Clone, Debug)]
pub struct Layer<M>(M);

#[derive(Clone, Debug)]
pub struct Service<S, M> {
    inner: S,
    map_target: M,
}

impl<T, S, M> super::Layer<T, M::Target, S> for Layer<M>
where
    S: svc::Service<M::Target>,
    M: MapTarget<T> + Clone,
{
    type Value = <Service<S, M> as svc::Service<T>>::Response;
    type Error = <Service<S, M> as svc::Service<T>>::Error;
    type Stack = Service<S, M>;

    fn bind(&self, inner: S) -> Self::Stack {
        Service {
            inner,
            map_target: self.0.clone(),
        }
    }
}

impl<T, S, M> svc::Service<T> for Service<S, M>
where
    S: svc::Service<M::Target>,
    M: MapTarget<T>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.inner.poll_ready()
    }

    fn call(&mut self, target: T) -> Self::Future {
        self.inner.call(self.map_target.map_target(target))
    }
}

impl<F, T, U> MapTarget<T> for F
where
    F: Fn(T) -> U,
{
    type Target = U;
    fn map_target(&self, t: T) -> U {
        (self)(t)
    }
}
