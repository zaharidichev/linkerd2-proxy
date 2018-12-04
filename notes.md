
- buffer needs to take a future of a service to buffer requests while a
  service is being created.

- we need need need an mpsc that does not allocate when cloning the sender.

- want a utility that spawns a future so that `Service::poll_ready`,
  especially, can consume futures without depending on requests to drive the
  future to completion.
```rust
fn bg<F: Future>(fut: F) -> Bg<F::Item, F::Error> {
    let (tx, rx) = oneshot::channel();

    let res = DefaultExecutor::current().spawn(fut.then(|res| {
        let _ = tx.send(res);
    }));

    Bg(res.ok().map(|_| rx))
}

pub struct Bg<I, E>(Option<oneshot::Receiver<Result<I, E>>>);

impl<I, E> Future for Bg<I, E> {
    type Item = I;
    type Error = Error<E>;

    fn poll(&mut self) -> Poll<I, E> {
        match self.0.as_mut() {
            Some(fut) => {
                let res = try_ready(fut.poll().map_err(|_| Error::Lost);
                res.map(Async::Ready).map_err(Error::Inner)
            }
            None => Err(Error::Lost),
        }
    }
}
```

- also probably something like this backed by ~mpsc to stream updates to `Service`s.

- router::Cache should be removed in favor of a simple
  `route_table: IndexMap<T, S>, route_table_capacity: usize` managed by a background task.
