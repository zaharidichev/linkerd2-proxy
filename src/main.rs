#![deny(warnings)]

extern crate linkerd2_proxy;

#[macro_use] extern crate log;
extern crate tokio;

use std::process;

use tokio::{
    executor::thread_pool,
    runtime,
};

use linkerd2_proxy::task::MainRuntime;

mod signal;

// Look in lib.rs.
fn main() {
    // Load configuration.
    let config = match linkerd2_proxy::app::init() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("configuration error: {:#?}", e);
            process::exit(64)
        }
    };

    let runtime: MainRuntime = match config.worker_threads {
        Some(n) if n > 1 => {
            info!("using thread pool with {} workers", n);
            let mut pool_builder = thread_pool::Builder::new();
            pool_builder
                .name_prefix("worker-")
                // Note: we may want to tune other pool parameters later,
                // or make them configurable with env variables.
                .pool_size(n);
            runtime::Builder::new()
                .threadpool_builder(pool_builder)
                .build()
                .expect("initialize main thread pool")
                .into()
        },
        _ => {
            info!("using single-threaded exexutor");
            runtime::current_thread::Runtime::new()
                .expect("initialize main runtime")
                .into()
        }
    };
    let main = linkerd2_proxy::Main::new(
        config,
        linkerd2_proxy::SoOriginalDst,
        runtime,
    );
    let shutdown_signal = signal::shutdown();
    main.run_until(shutdown_signal);
}
