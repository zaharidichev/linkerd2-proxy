use futures::future::{self, FutureResult};
use tower_service::{NewService, Service};

pub struct NewServiceFn<T> {
    f: T,
}

impl<T, N, R> NewServiceFn<T>
where
    T: Fn() -> N,
    N: Service<R>,
{
    pub fn new(f: T) -> Self {
        NewServiceFn {
            f,
        }
    }
}

impl<T, N, R> NewService<R> for NewServiceFn<T>
where
    T: Fn() -> N,
    N: Service<R>,
{
    type Response = N::Response;
    type Error = N::Error;
    type Service = N;
    type InitError = ();
    type Future = FutureResult<Self::Service, Self::InitError>;

    fn new_service(&self) -> Self::Future {
        future::ok((self.f)())
    }
}
