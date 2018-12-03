#[macro_use]
extern crate futures;
extern crate indexmap;
extern crate linkerd2_buffer as buffer;
extern crate linkerd2_stack as stack;
extern crate tokio;
extern crate tower_service as svc;

use futures::sync::{mpsc, oneshot};
use futures::{future::Executor, Async, Future, Poll, Stream};
use indexmap::IndexMap;
use std::hash::Hash;
use std::time::Duration;
use std::{error, fmt};
use svc::Service as _Service;
use tokio::executor::DefaultExecutor;

mod cache;

use self::cache::Cache;

const TODO_ROUTER_BUFFER_CAPACITY: usize = 1000;
const TODO_SERVICE_BUFFER_CAPACITY: usize = 100;

pub fn new<Req, Rec, Stk>(
    recognize: Rec,
    stack: Stk,
    route_table_capacity: usize,
) -> (Router<Req, Rec, Stk>, Daemon<Req, Rec, Stk>)
where
    Rec: Recognize<Req>,
    Stk: svc::Service<Rec::Target>,
    Stk::Response: svc::Service<Req>,
    Stk::Error: fmt::Display,
    <Stk::Response as svc::Service<Req>>::Error: fmt::Display,
{
    let (tx, rx) = mpsc::channel(TODO_ROUTER_BUFFER_CAPACITY);
    let router = Router { recognize, tx };

    let daemon = Daemon {
        rx,
        stack,
        // It's assumed a router will have at least two routes, otherwise there
        // wouldn't be a router.
        route_table: IndexMap::with_capacity(2),
        route_table_capacity,
    };

    (router, daemon)
}

/// Routes requests based on a configurable `Key`.
pub struct Router<Req, Rec, Stk>
where
    Rec: Recognize<Req>,
    Stk: svc::Service<Rec::Target>,
    Stk::Response: svc::Service<Req>,
{
    recognize: Rec,
    tx: Tx<Rec::Target, Req, Stk::Response, Stk::Error>,
}

pub struct Daemon<Req, Rec, Stk>
where
    Rec: Recognize<Req>,
    Stk: svc::Service<Rec::Target>,
    Stk::Response: svc::Service<Req>,
{
    rx: Rx<Rec::Target, Req, Stk::Response, Stk::Error>,
    stack: Stk,

    route_table: IndexMap<Rec::Target, buffer::Service<Req, Stk::Response>>,
    route_table_capacity: usize,
}

type SvcResult<R, S, E> = Result<buffer::Service<R, S>, Error<E, <S as svc::Service<R>>::Error>>;
type Rx<T, R, S, E> = mpsc::Receiver<(T, oneshot::Sender<SvcResult<R, S, E>>)>;
type Tx<T, R, S, E> = mpsc::Sender<(T, oneshot::Sender<SvcResult<R, S, E>>)>;

/// Provides a strategy for routing a Request to a Service.
///
/// Implementors must provide a `Key` type that identifies each unique route. The
/// `recognize()` method is used to determine the target for a given request. This target is
/// used to look up a route in a cache (i.e. in `Router`), or can be passed to
/// `bind_service` to instantiate the identified route.
pub trait Recognize<Request> {
    /// Identifies a Route.
    type Target: Clone + Eq + Hash;

    /// Determines the target for a route to handle the given request.
    fn recognize(&self, req: &Request) -> Option<Self::Target>;
}

#[derive(Debug, PartialEq)]
pub enum Error<I, E> {
    Stack(I),
    Service(E),
    NoCapacity(usize),
    NotRecognized,
    LostDaemon,
}

pub struct RspFuture<R, S, E>(RspState<R, S, E>)
where
    S: svc::Service<R>;

enum RspState<R, S, E>
where
    S: svc::Service<R>,
{
    Init(Option<R>, oneshot::Receiver<SvcResult<R, S, E>>),
    NotReady(Option<R>, buffer::Service<R, S>),
    Pending(<buffer::Service<R, S> as svc::Service<R>>::Future),
    NotRecognized,
}

// === impl Recognize ===

impl<R, T, F> Recognize<R> for F
where
    T: Clone + Eq + Hash,
    F: Fn(&R) -> Option<T>,
{
    type Target = T;

    fn recognize(&self, req: &R) -> Option<T> {
        (self)(req)
    }
}

// === impl Router ===

impl<Req, Rec, Stk> svc::Service<Req> for Router<Req, Rec, Stk>
where
    Rec: Recognize<Req>,
    Stk: svc::Service<Rec::Target>,
    Stk::Response: svc::Service<Req>,
{
    type Response = <Stk::Response as svc::Service<Req>>::Response;
    type Error = Error<Stk::Error, <Stk::Response as svc::Service<Req>>::Error>;
    type Future = RspFuture<Req, Stk::Response, Stk::Error>;

    /// Always ready to serve.
    ///
    /// Graceful backpressure is **not** supported at this level, since each request may
    /// be routed to different resources. Instead, requests should be issued and each
    /// route should support a queue of requests.
    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.tx.poll_ready().map_err(|_| Error::LostDaemon)
    }

    /// Routes the request through an underlying service.
    ///
    /// The response fails when the request cannot be routed.
    fn call(&mut self, request: Req) -> Self::Future {
        let target = match self.recognize.recognize(&request) {
            Some(target) => target,
            None => return RspFuture(RspState::NotRecognized),
        };

        let (tx, rx) = oneshot::channel();
        let _ = self.tx.try_send((target, tx));
        RspFuture(RspState::Init(Some(request), rx))
    }
}

impl<Req, Rec, Stk> Clone for Router<Req, Rec, Stk>
where
    Rec: Recognize<Req> + Clone,
    Stk: svc::Service<Rec::Target>,
    Stk::Response: svc::Service<Req>,
{
    fn clone(&self) -> Self {
        Router {
            tx: self.tx.clone(),
            recognize: self.recognize.clone(),
        }
    }
}

// === impl RspFuture ===

impl<R, S, E> Future for RspFuture<R, S, E>
where
    S: svc::Service<R>,
{
    type Item = S::Response;
    type Error = Error<E, S::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            self.0 = match self.0 {
                RspState::Init(ref mut req, ref mut svc_rx) => {
                    let svc = try_ready!(svc_rx.poll().map_err(|_| Error::LostDaemon))?;
                    RspState::NotReady(req.take(), svc)
                }
                RspState::NotReady(ref mut req, ref mut svc) => {
                    try_ready!(svc.poll_ready().map_err(|_| Error::LostDaemon));
                    let req = req.take().expect("request must be set");
                    RspState::Pending(svc.call(req))
                }
                RspState::Pending(ref mut fut) => {
                    return fut.poll().map_err(|e| match e {
                        buffer::Error::LostDaemon => Error::LostDaemon,
                        buffer::Error::Service(e) => Error::Service(e),
                    })
                }
                RspState::NotRecognized => return Err(Error::NotRecognized),
            };
        }
    }
}

// === impl Daemon ===

impl<Req, Rec, Stk> Future for Daemon<Req, Rec, Stk>
where
    Req: Send + 'static,
    Rec: Recognize<Req>,
    Stk: svc::Service<Rec::Target>,
    Stk::Response: svc::Service<Req> + Send + 'static,
    Stk::Future: Send + 'static,
    Stk::Error: fmt::Display,
    <Stk::Response as svc::Service<Req>>::Error: fmt::Display,
    <Stk::Response as svc::Service<Req>>::Response: Send + 'static,
    <Stk::Response as svc::Service<Req>>::Future: Send + 'static,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        // Drop servicesthat are no longer able to receive requests.
        // Because the route table holds buffered services, these failures
        // indicate that the buffer'sdaemon task has completed. This may happen
        // because:
        // - The stack did not initialize a service; or
        // - The service failed to poll_ready().
        self.route_table
            .retain(|_, ref mut svc| svc.poll_ready().is_ok());

        loop {
            match self.rx.poll() {
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Ok(Async::Ready(Some((target, svc_tx)))) => {
                    // First, try to load a cached route for `target`.
                    if let Some(ref mut svc) = self.route_table.get_mut(&target) {
                        if svc.poll_ready().is_ok() {
                            let _ = svc_tx.send(Ok(svc.clone()));
                            continue;
                        }
                    }

                    // Since there wasn't a cached route, ensure that there is capacity for a
                    // new one.
                    let reserve = match self.cache.reserve() {
                        Ok(r) => r,
                        Err(cache::CapacityExhausted { capacity }) => {
                            let _ = svc_tx.send(Err(Error::NoCapacity(capacity)));
                            continue;
                        }
                    };

                    let fut = self.stack.call(target.clone());
                    let (svc, daemon) = buffer::new(fut, TODO_SERVICE_BUFFER_CAPACITY);
                    let _ = DefaultExecutor::current().execute(daemon);

                    reserve.store(target, svc.clone());
                    let _ = svc_tx.send(Ok(svc));
                }
                Ok(Async::Ready(None)) | Err(_) => return Ok(Async::Ready(())),
            }
        }
    }
}

// === impl Error ===

impl<I, E> fmt::Display for Error<I, E>
where
    I: fmt::Display,
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Stack(ref why) => fmt::Display::fmt(why, f),
            Error::Service(ref why) => fmt::Display::fmt(why, f),
            Error::NotRecognized => f.pad("route not recognized"),
            Error::NoCapacity(capacity) => write!(f, "router capacity reached ({})", capacity),
            Error::LostDaemon => write!(f, "daemon task ended prematurely"),
        }
    }
}

impl<I, E> error::Error for Error<I, E>
where
    I: error::Error,
    E: error::Error,
{
    fn cause(&self) -> Option<&error::Error> {
        match self {
            Error::Stack(ref why) => Some(why),
            Error::Service(ref why) => Some(why),
            _ => None,
        }
    }

    fn description(&self) -> &str {
        match self {
            Error::Stack(_) => "inner stacking error",
            Error::Service(_) => "inner service error",
            Error::NoCapacity(_) => "router capacity reached",
            Error::NotRecognized => "route not recognized",
            Error::LostDaemon => "daemon task ended prematurely",
        }
    }
}

#[cfg(test)]
mod test_util {
    use futures::{future, Poll};
    use svc::Service;

    pub struct Recognize;

    #[derive(Debug)]
    pub struct MultiplyAndAssign(usize);

    #[derive(Debug)]
    pub enum Request {
        NotRecognized,
        Recognized(usize),
    }

    // === impl Recognize ===

    impl super::Recognize<Request> for Recognize {
        type Target = usize;

        fn recognize(&self, req: &Request) -> Option<Self::Target> {
            match *req {
                Request::NotRecognized => None,
                Request::Recognized(n) => Some(n),
            }
        }
    }

    impl svc::Service<usize> for Recognize {
        type Response = MultiplyAndAssign;
        type Error = ();
        type Future = future::FutureResult<Self::Response, Self::Error>;

        fn call(&self, _: &usize) -> Self::Future {
            futture::ok(MultiplyAndAssign(1))
        }
    }

    // === impl MultiplyAndAssign ===

    impl Default for MultiplyAndAssign {
        fn default() -> Self {
            MultiplyAndAssign(1)
        }
    }

    impl Service<Request> for MultiplyAndAssign {
        type Response = usize;
        type Error = ();
        type Future = future::FutureResult<usize, ()>;

        fn poll_ready(&mut self) -> Poll<(), ()> {
            unreachable!("not called in test")
        }

        fn call(&mut self, req: Request) -> Self::Future {
            let n = match req {
                Request::NotRecognized => unreachable!(),
                Request::Recognized(n) => n,
            };
            self.0 *= n;
            future::ok(self.0)
        }
    }

    impl From<usize> for Request {
        fn from(n: usize) -> Request {
            Request::Recognized(n)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, Router};
    use futures::Future;
    use std::time::Duration;
    use svc::Service;
    use test_util::*;

    impl Router<Request, Recognize, Recognize> {
        fn call_ok(&mut self, req: Request) -> usize {
            self.call(req).wait().expect("should route")
        }

        fn call_err(&mut self, req: Request) -> super::Error<(), ()> {
            self.call(req).wait().expect_err("should not route")
        }
    }

    #[test]
    fn invalid() {
        let mut router = Router::new(Recognize, Recognize, 1, Duration::from_secs(0));

        let rsp = router.call_err(Request::NotRecognized);
        assert_eq!(rsp, Error::NotRecognized);
    }

    #[test]
    fn cache_limited_by_capacity() {
        let mut router = Router::new(Recognize, Recognize, 1, Duration::from_secs(1));

        let rsp = router.call_ok(2.into());
        assert_eq!(rsp, 2);

        let rsp = router.call_err(3.into());
        assert_eq!(rsp, Error::NoCapacity(1));
    }

    #[test]
    fn services_cached() {
        let mut router = Router::new(Recognize, Recognize, 1, Duration::from_secs(0));

        let rsp = router.call_ok(2.into());
        assert_eq!(rsp, 2);

        let rsp = router.call_ok(2.into());
        assert_eq!(rsp, 4);
    }
}
