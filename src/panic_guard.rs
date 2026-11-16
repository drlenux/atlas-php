//! Keeps one request's panic from taking the whole service down.
//!
//! The service lives behind a `Mutex`; a panic while it is held poisons the lock and every
//! later request fails until restart. Transports run each use case through [`run`] (the panic
//! becomes an error for that request only) and take the lock through [`lock`] (a lock poisoned
//! by another thread, e.g. the watcher, is still usable).
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
};

static CAUGHT: AtomicU64 = AtomicU64::new(0);

/// Runs `action`; a panic is counted and returned as its message. The default panic hook
/// has already written location and message to stderr (`docker logs`).
pub fn run<T>(action: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(action)).map_err(|payload| {
        CAUGHT.fetch_add(1, Ordering::Relaxed);
        payload
            .downcast_ref::<&str>()
            .map(|message| (*message).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic without a message".to_owned())
    })
}

/// Panics caught by [`run`] since the process started (shown by `/health`).
pub fn caught() -> u64 {
    CAUGHT.load(Ordering::Relaxed)
}

/// Locks `mutex` even when another thread panicked while holding it.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn run_returns_the_value_or_the_panic_message_and_counts_panics() {
        let before = caught();
        assert_eq!(run(|| 7), Ok(7));
        assert_eq!(caught(), before);
        assert_eq!(
            run(|| panic!("static message")),
            Err::<(), _>("static message".into())
        );
        let formatted = run(|| panic!("code {}", 42));
        assert_eq!(formatted, Err::<(), _>("code 42".into()));
        assert!(caught() >= before + 2);
    }

    #[test]
    fn lock_recovers_a_poisoned_mutex() {
        let shared = Arc::new(Mutex::new(1));
        let poisoner = shared.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison");
        })
        .join();
        assert!(shared.is_poisoned());
        assert_eq!(*lock(&shared), 1);
    }
}
