use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread::JoinHandle;
use std::time::Duration;

/// Minimal, non-async TaskManager for background work and periodic jobs.
///
/// This is intentionally small and non-destructive: it spawns threads and
/// provides a cancel token for graceful shutdown. It avoids introducing an
/// async runtime so it can be adopted incrementally.
pub struct TaskManager {
    handles: Mutex<Vec<JoinHandle<()>>>,
    cancel_tokens: Mutex<Vec<Arc<AtomicBool>>>,
}

impl TaskManager {
    /// Create a new TaskManager.
    pub fn new() -> Self {
        Self {
            handles: Mutex::new(Vec::new()),
            cancel_tokens: Mutex::new(Vec::new()),
        }
    }

    /// Spawn a one-shot background task. The provided closure receives a
    /// cancel token (Arc<AtomicBool>) that can be observed to stop early.
    pub fn spawn<F>(&self, name: impl Into<String>, f: F)
    where
        F: FnOnce(Arc<AtomicBool>) + Send + 'static,
    {
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        let name = name.into();
        let handle = std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                f(token);
            })
            .expect("failed to spawn task");

        self.cancel_tokens.lock().unwrap().push(cancel);
        self.handles.lock().unwrap().push(handle);
    }

    /// Spawn a periodic task that runs `f` every `interval`. The closure
    /// receives the cancel token and should return quickly.
    pub fn spawn_periodic<F>(&self, name: impl Into<String>, interval: Duration, f: F)
    where
        F: Fn(Arc<AtomicBool>) + Send + Sync + 'static,
    {
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        let f_arc = Arc::new(f);
        let name = name.into();

        let handle = std::thread::Builder::new()
            .name(name)
            .spawn(move || {
                while !token.load(Ordering::SeqCst) {
                    (f_arc)(token.clone());
                    std::thread::sleep(interval);
                }
            })
            .expect("failed to spawn periodic task");

        self.cancel_tokens.lock().unwrap().push(cancel);
        self.handles.lock().unwrap().push(handle);
    }

    /// Request shutdown and join background tasks.
    pub fn shutdown(&self) {
        for t in self.cancel_tokens.lock().unwrap().iter() {
            t.store(true, Ordering::SeqCst);
        }

        let mut handles = self.handles.lock().unwrap();
        while let Some(h) = handles.pop() {
            let _ = h.join();
        }
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}
