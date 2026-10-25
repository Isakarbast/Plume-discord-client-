//! GTK widgets may only be touched on the main thread, while networking
//! runs on a small tokio runtime in the background. These helpers move work
//! and results between the two.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// The background runtime: two worker threads are plenty for a chat client.
pub fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("plume-net")
            .enable_all()
            .build()
            .expect("couldn't start the background runtime")
    })
}

/// Runs `work` in the background, then `done` with its result on the GTK
/// main thread (so `done` may update widgets).
pub fn spawn<T, F, D>(work: F, done: D)
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
    D: FnOnce(T) + 'static,
{
    let (sender, receiver) = tokio::sync::oneshot::channel();
    runtime().spawn(async move {
        let _ = sender.send(work.await);
    });
    gtk::glib::spawn_future_local(async move {
        if let Ok(result) = receiver.await {
            done(result);
        }
    });
}

/// Runs `work` in the background and ignores the result.
pub fn spawn_detached<F>(work: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    runtime().spawn(work);
}

/// Forwards a tokio channel into one the GTK main loop can await.
pub fn forward<T: Send + 'static>(
    mut from: tokio::sync::mpsc::Receiver<T>,
) -> async_channel::Receiver<T> {
    let (sender, receiver) = async_channel::bounded(256);
    runtime().spawn(async move {
        while let Some(item) = from.recv().await {
            if sender.send(item).await.is_err() {
                break; // the window is gone
            }
        }
    });
    receiver
}
