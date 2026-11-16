//! Cancellation of a running call: the one handle an app and its engine share to stop an export.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

/// A standing request to stop the export a call is running (plan §16).
///
/// A token is an `Arc<AtomicBool>`, so answering a cancel needs no engine call: the engine is
/// inside the export while a cancel arrives, and a cancel has to land there, not queue behind it
/// (plan §13, L3). The engine hands the same token into the export's progress closure, so the flag
/// is checked at every progress poll, and an app that stops at the first `false` the closure
/// answers stops within one progress interval of the cancel (plan §16).
///
/// Cloning a token shares the one flag, so the app's layer keeps a clone for its cancel button and
/// hands the original (or any clone) to the call it wants to be able to stop.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    /// A token nobody has cancelled yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the call this token was given to stop: an export running stops at its next progress
    /// report, and one still waiting for its turn answers `Cancelled` without running. Idempotent
    /// and safe from any thread at any time. A token stays cancelled, so every export later given
    /// it answers `Cancelled` at once.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Whether [`CancelToken::cancel`] has been called on this token or any clone of it.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn a_fresh_token_is_not_cancelled() {
        assert!(!CancelToken::new().is_cancelled());
        assert!(!CancelToken::default().is_cancelled());
    }

    #[test]
    fn cancelling_a_token_cancels_every_clone() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert!(!clone.is_cancelled());
        clone.cancel();
        assert!(token.is_cancelled(), "the clone shares the one flag");
        // Cancelling again changes nothing.
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn a_cancel_from_another_thread_is_visible() {
        let token = CancelToken::new();
        let canceller = {
            let token = token.clone();
            thread::spawn(move || token.cancel())
        };
        canceller.join().unwrap();
        assert!(token.is_cancelled(), "the flag is visible across threads");
    }
}
