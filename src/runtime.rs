use std::future::Future;
use std::sync::OnceLock;

pub fn handle() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("mailbench-io")
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    handle().spawn(future)
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(h) => tokio::task::block_in_place(|| h.block_on(future)),
        Err(_) => handle().block_on(future),
    }
}
