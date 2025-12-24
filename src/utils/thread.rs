use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::Duration;

static QUEUE_SENDER: Mutex<Option<mpsc::Sender<Box<dyn FnOnce() + Send + 'static>>>> =
    Mutex::new(None);

const QUEUE_THREAD_TIMEOUT_SECS: u64 = 5;

/// Offload a blocking function to a queue that is processed sequentially by a
/// single shared thread.
///
/// Functions will be run in a predictable order, but they will block subsequent
/// functions until they complete.
///
/// The thread will destroy itself after a period of inactivity.
pub(crate) fn queue_blocking(f: impl FnOnce() + Send + 'static) {
    // Hold the lock until done.
    let mut guard = QUEUE_SENDER.lock().unwrap();

    let job: Box<dyn FnOnce() + Send + 'static> = Box::new(f);

    // Try to send the job to an existing thread if any.
    if let Some(sender) = guard.as_ref() {
        sender.send(job).expect("Background thread panicked");
        return;
    }

    let (tx, rx) = mpsc::channel();
    tx.send(job).unwrap();
    *guard = Some(tx);

    thread::spawn(move || {
        loop {
            // Wait for a job or timeout.
            match rx.recv_timeout(Duration::from_secs(QUEUE_THREAD_TIMEOUT_SECS)) {
                Ok(func) => func(),
                Err(_) => {
                    // Try once more with the queue locked before destroying the thread.
                    let mut guard = QUEUE_SENDER.lock().unwrap();
                    match rx.try_recv() {
                        Ok(func) => {
                            drop(guard);
                            func();
                        }
                        Err(_) => {
                            // Leave None before releasing the lock to signal the thread is gone.
                            *guard = None;
                            break;
                        }
                    }
                }
            }
        }
    });
}
