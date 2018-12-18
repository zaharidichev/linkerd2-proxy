use futures::{future, Future};
use hyper::{Body, server::conn::Http, service::Service};
use tokio::executor::current_thread::TaskExecutor;
use tokio_trace_futures::Instrument;

use task;
use transport::BoundPort;

pub fn serve_http<S>(
    name: &'static str,
    bound_port: BoundPort,
    service: S,
) -> impl Future<Item = (), Error = ()>
where
    S: Service<ReqBody = Body> + Clone + Send + 'static,
    <S as Service>::Future: Send,
{
    let mut span = span!("serve_http", name = name, local_addr = ::tokio_trace::field::debug(bound_port.local_addr()));
    let fut = {
        bound_port
            .listen_and_fold(Http::new(), move |hyper, (conn, remote)| {
                let serve = hyper
                    .serve_connection(conn, service.clone())
                    .map(|_| {})
                    .map_err(move |e| {
                        error!("error serving {}: {:?}", name, e);
                    })
                    .instrument(span!("serve_connection", remote_addr = ::tokio_trace::field::debug(remote)));

                let r = TaskExecutor::current()
                    .spawn_local(Box::new(serve))
                    .map(move |()| hyper)
                    .map_err(task::Error::into_io);

                future::result(r)
            }).map_err(move |err| { error!("listener error: {}", err); })

    };

    fut.instrument(span)
}
