#[macro_use]
extern crate futures;
#[macro_use]
extern crate log;
extern crate tower_service as svc;

use futures::sync::{mpsc, oneshot};
use futures::{Async, Future, Poll, Stream};
use std::{error, fmt};
use svc::Service as _Service;

pub fn new<R, F>(fut: F, capacity: usize) -> (Service<R, F::Item>, Daemon<F, R>)
where
    F: Future,
    F::Item: svc::Service<R>,
{
    let state = StackState::Init(fut);
    let (tx, rx) = mpsc::channel(capacity);
    (Service { tx }, Daemon { rx, state })
}

#[derive(Debug)]
pub struct Service<Req, Svc: svc::Service<Req>> {
    tx: mpsc::Sender<(Req, oneshot::Sender<Svc::Future>)>,
}

#[must_use = "daemon must be polled"]
pub struct Daemon<F, Req>
where
    F: Future,
    F::Item: svc::Service<Req>,
{
    rx: mpsc::Receiver<(Req, oneshot::Sender<<F::Item as svc::Service<Req>>::Future>)>,
    state: StackState<F>,
}

#[derive(Debug)]
enum StackState<F: Future> {
    Init(F),
    Active(F::Item),
}

#[derive(Debug)]
pub enum Error<E> {
    LostDaemon,
    Service(E),
}

pub struct RspFuture<F>(RspState<F>);

enum RspState<F> {
    Dispatch(oneshot::Receiver<F>),
    Pending(F),
}

// === impl Service ===

impl<Req, Svc: svc::Service<Req>> Clone for Service<Req, Svc> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl<Req, Svc: svc::Service<Req>> svc::Service<Req> for Service<Req, Svc> {
    type Response = Svc::Response;
    type Error = Error<Svc::Error>;
    type Future = RspFuture<Svc::Future>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.tx.poll_ready().map_err(|_| Error::LostDaemon)
    }

    fn call(&mut self, req: Req) -> Self::Future {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.try_send((req, tx));
        RspFuture(RspState::Dispatch(rx))
    }
}

// === impl RspFuture ===

impl<F: Future> Future for RspFuture<F> {
    type Item = F::Item;
    type Error = Error<F::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            self.0 = match self.0 {
                RspState::Dispatch(ref mut rx) => {
                    let fut = try_ready!(rx.poll().map_err(|_| Error::LostDaemon));
                    RspState::Pending(fut)
                }
                RspState::Pending(ref mut fut) => {
                    return fut.poll().map_err(Error::Service);
                }
            };
        }
    }
}

// === impl Daemon ===

impl<F, Req> Future for Daemon<F, Req>
where
    F: Future,
    F::Error: fmt::Display,
    F::Item: svc::Service<Req>,
    <F::Item as svc::Service<Req>>::Error: fmt::Display,
{
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<(), ()> {
        loop {
            self.state = match self.state {
                StackState::Init(ref mut f) => match f.poll() {
                    Ok(Async::NotReady) => return Ok(Async::NotReady),
                    Ok(Async::Ready(svc)) => StackState::Active(svc),
                    Err(e) => {
                        error!("failed to build stack: {}", e);
                        return Ok(Async::Ready(()));
                    }
                },
                StackState::Active(ref mut svc) => loop {
                    match svc.poll_ready() {
                        Ok(Async::NotReady) => return Ok(Async::NotReady),
                        Ok(Async::Ready(())) => match self.rx.poll() {
                            Ok(Async::NotReady) => return Ok(Async::NotReady),
                            Ok(Async::Ready(Some((req, tx)))) => {
                                let _ = tx.send(svc.call(req));
                            }
                            Ok(Async::Ready(None)) | Err(_) => {
                                debug!("lost router");
                                return Ok(Async::Ready(()));
                            }
                        },
                        Err(e) => {
                            error!("stack failed: {}", e);
                            return Ok(Async::Ready(()));
                        }
                    }
                },
            };
        }
    }
}

// === impl Error ===

impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Service(e) => e.fmt(f),
            Error::LostDaemon => write!(f, "lost damemon task"),
        }
    }
}

impl<E: error::Error> error::Error for Error<E> {
    fn cause(&self) -> Option<&error::Error> {
        match self {
            Error::Service(ref e) => Some(e),
            Error::LostDaemon => None,
        }
    }
}
