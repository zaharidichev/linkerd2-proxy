#[macro_use]
extern crate futures;
extern crate linkerd2_metrics;
extern crate tokio_timer;
extern crate tower_service;

use futures::{Async, Future, Poll};
use linkerd2_metrics::histogram::{Bounds, Histogram};
use linkerd2_metrics::latency;
use tokio_timer::{clock, Delay};
use tower_service::Service;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A "retry policy" to classify if a request should be pre-emptively retried.
pub trait Policy<Request>: Sized {
    fn can_retry(&self, req: &Request) -> bool;
    fn clone_request(&self, req: &Request) -> Option<Request>;
}

/// A middleware pre-emptively retries requests which have been outstanding for
/// longer than a given latency percentile.  If either of the original future
/// or the retry future completes, that value is used.
#[derive(Clone, Debug)]
pub struct Hedge<P, S> {
    policy: P,
    service: S,
    latency_percentile: f32,
    // TODO: rotate this histogram on a regular interval
    latency_histogram: Arc<Mutex<Histogram<latency::Ms>>>,
}

pub struct ResponseFuture<P, S, Request>
where
    P: Policy<Request>,
    S: Service<Request>,
{
    request: Option<Request>,
    start: Instant,
    hedge: Hedge<P, S>,
    orig_fut: S::Future,
    hedge_fut: Option<S::Future>,
    delay: Delay,
}

impl<P, S> Hedge<P, S> {
    pub fn new<Request>(
        policy: P,
        service: S,
        latency_percentile: f32,
        bounds: &'static Bounds,
    ) -> Self
    where
        P: Policy<Request> + Clone,
        S: Service<Request>,
    {
        Hedge {
            policy,
            service,
            latency_percentile,
            latency_histogram: Arc::new(Mutex::new(Histogram::new(bounds))),
        }
    }
}

impl<P, S, Request> Service<Request> for Hedge<P, S>
where
    P: Policy<Request> + Clone,
    S: Service<Request> + Clone,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<P, S, Request>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        self.service.poll_ready()
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let cloned = self.policy.clone_request(&request);
        let orig_fut = self.service.call(request);

        // TODO: Only create a hedge timeout if there are sufficiently many
        // data points in the histogram.
        let hedge_timeout = self
            .latency_histogram
            .lock()
            .unwrap()
            .percentile(self.latency_percentile);
        let start = clock::now();
        let delay = Delay::new(start + Duration::from_millis(hedge_timeout));

        ResponseFuture {
            request: cloned,
            start,
            hedge: self.clone(),
            orig_fut,
            hedge_fut: None,
            delay,
        }
    }
}

impl<P, S, Request> ResponseFuture<P, S, Request> where
    P: Policy<Request>,
    S: Service<Request>,
{
    /// Record the latency of a completed request in the latency histogram.
    fn record(&self) {
        let duration = clock::now() - self.start;
        self.hedge.latency_histogram.lock().unwrap().add(duration);
    }
}

impl<P, S, Request> Future for ResponseFuture<P, S, Request>
where
    P: Policy<Request> + Clone,
    S: Service<Request> + Clone,
{
    type Item = S::Response;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            // If the original future is complete, return its result.
            match self.orig_fut.poll() {
                Ok(Async::Ready(rsp)) => {
                    self.record();
                    return Ok(Async::Ready(rsp));
                }
                Ok(Async::NotReady) => {}
                Err(e) => {
                    self.record();
                    return Err(e);
                }
            }

            if let Some(ref mut hedge_fut) = self.hedge_fut {
                // If the hedge future exists, return its result.
                return hedge_fut.poll();
            } else {
                // Original future is pending, but hedge hasn't started.  Check
                // the delay.
                match self.delay.poll() {
                    Ok(Async::Ready(_)) => {
                        try_ready!(self.hedge.poll_ready());
                        if let Some(req) = self.request.take() {
                            if self.hedge.policy.can_retry(&req) {
                                // Start the hedge request.
                                self.request = self.hedge.policy.clone_request(&req);
                                self.hedge_fut = Some(self.hedge.service.call(req));
                            } else {
                                // Policy says we can't retry.
                                // Put the taken request back.
                                self.request = Some(req);
                                return Ok(Async::NotReady);
                            }
                        } else {
                            // No cloned request, can't retry.
                            return Ok(Async::NotReady);
                        }
                    },
                    Ok(Async::NotReady) => return Ok(Async::NotReady), // Not time to retry yet.
                    Err(_) => return Ok(Async::NotReady), // Timer error, don't retry.
                }
            }
        }
    }
}
