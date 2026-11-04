//! Thread polling utilities.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use std::sync::Arc;
use std::thread;

use crossbeam_channel::{Receiver, Sender};
use tracing::trace;


/// This structure is made to block on multiple thread at the same time and repeatedly
/// in order to aggregate the value they are returning.
#[derive(Debug)]
pub struct ThreadPoll<T> {
    tx: Sender<T>,
    rx: Receiver<T>,
}

impl<T: Send + 'static> ThreadPoll<T> {

    /// Create and initialize a new thread poll.
    pub fn new() -> Self {
        let (tx, rx) = crossbeam_channel::bounded(2);
        Self {
            tx, rx,
        }
    }

    /// Inner function that we can provide with a trace name for the closure.
    fn inner_spawn<F>(&self, mut producer: F, name: &'static str, with_handle: bool)
    where 
        F: FnMut() -> Option<T>,
        F: Send + 'static,
    {

        static UNIQUE_ID: AtomicUsize = AtomicUsize::new(0);

        let tx = self.tx.clone();
        let id = UNIQUE_ID.fetch_add(1, Ordering::Relaxed);
        
        thread::Builder::new()
            .name(format!("poll-worker-{id:04x}"))
            .spawn(move || {
                trace!("New poll worker #{id:04X} ({name}, handle: {with_handle})");
                while let Some(value) = producer() {
                    if tx.send(value).is_err() {
                        break;
                    }
                }
                trace!("Kill poll worker #{id:04X}")
            })
            .unwrap();

    }

    /// Spawn a new value producer that will be continuously polled and its result will
    /// be added to the internal queue that can be retrieved with [`Self::poll`], this
    /// producer's thread terminates when this aggregator is dropped. In order for this
    /// to properly work you should be using some kind of timeout on the producer.
    pub fn spawn<F>(&self, producer: F)
    where 
        F: FnMut() -> Option<T>,
        F: Send + 'static,
    {
        self.inner_spawn(producer, std::any::type_name::<F>(), false)
    }

    /// Same as [`Self::spawn`] but also returning a handle that, when dropped, will end
    /// the associated worker thread.
    pub fn spawn_with_handle<F>(&self, mut producer: F) -> ThreadWorker
    where 
        F: FnMut() -> Option<T>,
        F: Send + 'static,
    {
        let alive = Arc::new(());
        let thread_alive = Arc::clone(&alive);
        self.inner_spawn(move || {
            let ret = producer();
            if Arc::strong_count(&thread_alive) > 1 {
                return ret;
            } else {
                None
            }
        }, std::any::type_name::<F>(), true);
        ThreadWorker(alive)
    }

    /// Block until a new value is available.
    pub fn poll(&self) -> T {
        // Unwrap because we own both ends so it should not disconnect.
        self.rx.recv().unwrap()
    }

    /// Block until a new value is available or the deadline passes.
    pub fn poll_until(&self, deadline: Instant) -> Option<T> {
        self.rx.recv_deadline(deadline).ok()
    }

    /// Non-blocking poll.
    pub fn try_poll(&self) -> Option<T> {
        // Don't care of the "disconnected" error because it should not happen.
        self.rx.try_recv().ok()
    }

}

/// Represent a handle to a thread poll worker, when all (cloned) handles are dropped
/// then the producer will be stopped on the next iteration.
#[derive(Debug, Clone)]
#[allow(unused)]
pub struct ThreadWorker(Arc<()>);
