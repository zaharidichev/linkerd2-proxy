extern crate futures;
extern crate linkerd2_hedge as hedge;
extern crate linkerd2_metrics as metrics;
extern crate tokio_executor;
extern crate tokio_timer;
extern crate tower_mock;
extern crate tower_service;

use futures::{future, Future};
use hedge::Policy;
use metrics::latency;
use tokio_executor::enter;
use tokio_timer::clock;
use tower_service::Service;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[test]
fn hedge_orig_completes_first() {
    let (mut service, mut handle) = new_service(TestPolicy);

    let time = Arc::new(Mutex::new(Instant::now()));
    let clock = clock::Clock::new_with_now(Now(time.clone()));
    let mut enter = enter().expect("enter");

    clock::with_default(&clock, &mut enter, |_| {
        let mut fut = service.call("orig");
        let req = handle.next_request().expect("orig");
        assert_not_ready(&mut fut);

        *time.lock().unwrap() += Duration::from_millis(10);
        assert_not_ready(&mut fut);

        req.respond("orig-done");
        assert_eq!(fut.wait().unwrap(), "orig-done");
    });
}

#[test]
fn hedge_hedge_completes_first() {

}

#[test]
fn completes_before_hedge() {

}

#[test]
fn request_not_retyable() {

}

#[test]
fn request_not_clonable() {

}

type Req = &'static str;
type Res = &'static str;
type Error = &'static str;
type Mock = tower_mock::Mock<Req, Res, Error>;
type Handle = tower_mock::Handle<Req, Res, Error>;
type Histogram = Arc<Mutex<metrics::Histogram<latency::Ms>>>;

static NOT_RETRYABLE: &'static str = "NOT_RETRYABLE";
static NOT_CLONABLE: &'static str = "NOT_CLONABLE";

#[derive(Clone)]
struct TestPolicy;

impl Policy<Req> for TestPolicy {
    fn can_retry(&self, req: &Req) -> bool {
        *req != NOT_RETRYABLE
    }

    fn clone_request(&self, req: &Req) -> Option<Req> {
        if *req == NOT_CLONABLE {
            None
        } else {
            Some(req)
        }
    }
}

fn new_service<P: Policy<Req> + Clone>(
    policy: P,
) -> (hedge::Hedge<P, Mock>, Handle) {
    let (service, mut handle) = Mock::new();
    let mut service = hedge::Hedge::new(policy, service, 0.9, latency::BOUNDS);
    populate_histogram(&mut service, &mut handle);
    (service, handle)
}

fn populate_histogram<P: Policy<Req> + Clone>(service: &mut hedge::Hedge<P, Mock>, handle: &mut Handle) {
    let time = Arc::new(Mutex::new(Instant::now()));
    let clock = clock::Clock::new_with_now(Now(time.clone()));
    let mut enter = enter().expect("enter");

    clock::with_default(&clock, &mut enter, |_| {
        for _ in 0..8 {
            let mut fut = service.call("populate");
            let req = handle.next_request().expect("populate");
            *time.lock().unwrap() += Duration::from_millis(1);
            req.respond("done");
            fut.wait().unwrap();
        }
        for _ in 8..10 {
            let mut fut = service.call("populate");
            let req = handle.next_request().expect("populate");
            *time.lock().unwrap() += Duration::from_millis(10);
            req.respond("done");
            fut.wait().unwrap();
        }
    });
}

struct Now(Arc<Mutex<Instant>>);
impl clock::Now for Now {
    fn now(&self) -> Instant {
        *self.0.lock().expect("lock")
    }
}

fn assert_not_ready<F: Future>(f: &mut F)
where
    F::Error: ::std::fmt::Debug,
{
    use futures::future;
    future::poll_fn(|| {
        assert!(f.poll().unwrap().is_not_ready());
        Ok::<_, ()>(().into())
    })
    .wait()
    .unwrap();
}
