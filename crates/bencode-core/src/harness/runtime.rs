use std::sync::OnceLock;

use tokio::runtime::{Builder, Runtime};

const WORKER_THREADS: usize = 2;

/// Process-wide Tokio runtime for harness child processes.
///
/// GPUI drives its own executor, which has no Tokio reactor, so anything that
/// touches `tokio::process` or `tokio::spawn` must be scheduled here. Futures
/// such as `JoinHandle` and `mpsc::Receiver::recv` are runtime-agnostic and can
/// be awaited from `cx.spawn`.
pub fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(WORKER_THREADS)
            .thread_name("bencode-harness")
            .enable_all()
            .build()
            .expect("failed to start harness Tokio runtime")
    })
}
