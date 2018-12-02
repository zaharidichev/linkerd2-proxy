extern crate futures_watch;

use self::futures_watch::Watch;
use futures::future::{self, MapErr};
use futures::{Async, Future, Poll, Stream};
use std::marker::PhantomData;
use std::{error, fmt};

use svc;

/// Implemented by targets that can be updated by a `Watch<U>`
pub trait WithUpdate<U> {
    type Updated;

    fn with_update(self, update: &U) -> Self::Updated;
}

#[derive(Debug)]
pub struct Layer<T: WithUpdate<U>, U, M> {
    watch: Watch<U>,
    _p: PhantomData<fn() -> (T, M)>,
}

#[derive(Debug)]
pub struct Stack<T: WithUpdate<U>, U, M> {
    watch: Watch<U>,
    inner: M,
    _p: PhantomData<fn() -> T>,
}

pub struct StackFuture<T: WithUpdate<U>, U, M: svc::Service<T::Updated>> {
    future: M::Future,
    inner: Option<(T, Watch<U>, M)>,
}

/// A Service that updates itself as a Watch updates.
pub struct Service<T: WithUpdate<U>, U, M: svc::Service<T::Updated>> {
    watch: Watch<U>,
    target: T,
    stack: M,
    state: State<T, U, M>,
}

enum State<T: WithUpdate<U>, U, M: svc::Service<T::Updated>> {
    Pending(Pending<T::Updated, M>),
    Ready(M::Response),
}

enum Pending<T, M: svc::Service<T>> {
    Init { target: Option<T>, stack: M },
    Pending(M::Future),
}

#[derive(Debug)]
pub enum Error<I, M> {
    Stack(M),
    Inner(I),
}

/// A special implemtation of WithUpdate that clones the observed update value.
#[derive(Clone, Debug)]
pub struct CloneUpdate;

// === impl Layer ===

pub fn layer<T, U, M>(watch: Watch<U>) -> Layer<T, U, M>
where
    T: WithUpdate<U> + Clone,
    M: svc::Service<T::Updated> + Clone,
{
    Layer {
        watch,
        _p: PhantomData,
    }
}

impl<T, U, M> Clone for Layer<T, U, M>
where
    T: WithUpdate<U> + Clone,
    M: svc::Service<T::Updated> + Clone,
{
    fn clone(&self) -> Self {
        layer(self.watch.clone())
    }
}

impl<T, U, M> super::Layer<T, T::Updated, M> for Layer<T, U, M>
where
    T: WithUpdate<U> + Clone,
    M: svc::Service<T::Updated> + Clone,
{
    type Value = <Stack<T, U, M> as svc::Service<T>>::Response;
    type Error = <Stack<T, U, M> as svc::Service<T>>::Error;
    type Stack = Stack<T, U, M>;

    fn bind(&self, inner: M) -> Self::Stack {
        Stack {
            inner,
            watch: self.watch.clone(),
            _p: PhantomData,
        }
    }
}

// === impl Stack ===

impl<T: WithUpdate<U>, U, M: Clone> Clone for Stack<T, U, M> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            watch: self.watch.clone(),
            _p: PhantomData,
        }
    }
}

impl<T, U, M> svc::Service<T> for Stack<T, U, M>
where
    T: WithUpdate<U> + Clone,
    M: svc::Service<T::Updated> + Clone,
{
    type Response = Service<T, U, M>;
    type Error = M::Error;
    type Future = future::FutureResult<Self::Response, Self::Error>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.inner.poll_ready()
    }

    fn call(&mut self, target: T) -> Self::Future {
        let p = Pending::Init {
            target: Some(target.clone().with_update(&*self.watch.borrow())),
            stack: self.inner.clone(),
        };
        future::ok(Service {
            target,
            stack: self.inner.clone(),
            watch: self.watch.clone(),
            state: State::Pending(p),
        })
    }
}

// === impl Service ===

impl<T, U, M, R> svc::Service<R> for Service<T, U, M>
where
    T: WithUpdate<U> + Clone,
    M: svc::Service<T::Updated> + Clone,
    M::Response: svc::Service<R>,
{
    type Response = <M::Response as svc::Service<R>>::Response;
    type Error = Error<<M::Response as svc::Service<R>>::Error, M::Error>;
    type Future = MapErr<
        <M::Response as svc::Service<R>>::Future,
        fn(<M::Response as svc::Service<R>>::Error) -> Self::Error,
    >;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        while let Ok(Async::Ready(Some(()))) = self.watch.poll() {
            let target = Some(self.target.clone().with_update(&*self.watch.borrow()));
            let stack = self.stack.clone();
            self.state = State::Pending(Pending::Init { target, stack });
        }

        loop {
            self.state = match self.state {
                State::Pending(ref mut p) => {
                    let svc = try_ready!(p.poll().map_err(Error::Stack));
                    State::Ready(svc)
                }
                State::Ready(ref mut svc) => {
                    return svc.poll_ready().map_err(Error::Inner);
                }
            };
        }
    }

    fn call(&mut self, req: R) -> Self::Future {
        if let State::Ready(ref mut svc) = self.state {
            return svc.call(req).map_err(Error::Inner);
        }

        unreachable!("service must be ready");
    }
}

impl<U, M> Service<CloneUpdate, U, M>
where
    U: Clone,
    M: svc::Service<U> + Clone,
{
    pub fn cloning(watch: Watch<U>, stack: M) -> Self {
        let p = Pending::Init {
            target: Some((*watch.borrow()).clone()),
            stack: stack.clone(),
        };
        Service {
            stack,
            watch,
            target: CloneUpdate,
            state: State::Pending(p),
        }
    }
}

impl<T, U, M> Clone for Service<T, U, M>
where
    T: WithUpdate<U> + Clone,
    T::Updated: Clone,
    M: svc::Service<T::Updated> + Clone,
    M::Response: Clone,
{
    fn clone(&self) -> Self {
        Self {
            stack: self.stack.clone(),
            watch: self.watch.clone(),
            target: self.target.clone(),
            state: match self.state {
                State::Pending(Pending::Init {
                    ref target,
                    ref stack,
                }) => State::Pending(Pending::Init {
                    target: target.clone(),
                    stack: stack.clone(),
                }),
                State::Pending(Pending::Pending(_)) => State::Pending(Pending::Init {
                    target: Some(self.target.clone().with_update(&*self.watch.borrow())),
                    stack: self.stack.clone(),
                }),
                State::Ready(ref svc) => State::Ready(svc.clone()),
            },
        }
    }
}

// === impl Pending ===

impl<T, M: svc::Service<T>> Future for Pending<T, M> {
    type Item = M::Response;
    type Error = M::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            *self = match self {
                Pending::Init {
                    ref mut target,
                    ref mut stack,
                } => {
                    try_ready!(stack.poll_ready());
                    let t = target.take().expect("target must be set");
                    Pending::Pending(stack.call(t))
                }
                Pending::Pending(ref mut f) => {
                    return f.poll();
                }
            };
        }
    }
}

// === impl CloneUpdate ===

impl<U: Clone> WithUpdate<U> for CloneUpdate {
    type Updated = U;

    fn with_update(self, update: &U) -> U {
        update.clone()
    }
}

// === impl Error ===

impl<I: fmt::Display, M: fmt::Display> fmt::Display for Error<I, M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Inner(i) => i.fmt(f),
            Error::Stack(m) => m.fmt(f),
        }
    }
}

impl<I: error::Error, M: error::Error> error::Error for Error<I, M> {}

#[cfg(test)]
mod tests {
    extern crate linkerd2_task as task;
    extern crate tokio;

    use self::task::test_util::BlockOnFor;
    use self::tokio::runtime::current_thread::Runtime;
    use super::*;
    use futures::{future, Poll};
    use std::time::Duration;
    use svc::Service as _Service;

    const TIMEOUT: Duration = Duration::from_secs(60);

    #[test]
    fn rebind() {
        struct Svc(usize);
        impl svc::Service<()> for Svc {
            type Response = usize;
            type Error = ();
            type Future = future::FutureResult<usize, ()>;
            fn poll_ready(&mut self) -> Poll<(), Self::Error> {
                Ok(().into())
            }
            fn call(&mut self, _: ()) -> Self::Future {
                future::ok(self.0)
            }
        }

        let mut rt = Runtime::new().unwrap();
        macro_rules! assert_ready {
            ($svc:expr) => {
                rt.block_on_for(TIMEOUT, future::poll_fn(|| $svc.poll_ready()))
                    .expect("ready")
            };
        }
        macro_rules! call {
            ($svc:expr) => {
                rt.block_on_for(TIMEOUT, $svc.call(())).expect("call")
            };
        }

        struct Stack;
        impl ::svc::Service<usize> for Stack {
            type Response = Svc;
            type Error = ();
            type Future = future::FutureResult<Self::Response, Self::Error>;

            fn poll_ready(&self) -> Poll<(), Never> {
                Ok(().into())
            }

            fn call(&self, n: &usize) -> Self::Future {
                future::ok(Svc(*n))
            }
        }

        let (watch, mut store) = Watch::new(1);
        let mut svc = Service::cloning(watch, Stack);

        assert_ready!(svc);
        assert_eq!(call!(svc), 1);

        assert_ready!(svc);
        assert_eq!(call!(svc), 1);

        store.store(2).expect("store");
        assert_ready!(svc);
        assert_eq!(call!(svc), 2);

        store.store(3).expect("store");
        store.store(4).expect("store");
        assert_ready!(svc);
        assert_eq!(call!(svc), 4);

        drop(store);
        assert_ready!(svc);
        assert_eq!(call!(svc), 4);
    }
}
