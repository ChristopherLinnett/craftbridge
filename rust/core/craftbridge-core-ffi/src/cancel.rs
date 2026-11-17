//! The handle a foreign caller keeps to stop an export.

use std::sync::Arc;

/// A standing request to stop the export a call is running. The app keeps the token it hands to
/// `request_with_progress` and cancels it when its user does: UniFFI futures cannot be cancelled, so
/// this flag is the only way to stop one.
#[derive(uniffi::Object)]
pub struct CancelToken {
    core: craftbridge_core::CancelToken,
}

#[uniffi::export]
impl CancelToken {
    /// A token nobody has cancelled yet.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self { core: craftbridge_core::CancelToken::new() })
    }

    /// Stop the export this token was given to, at its next progress report. Safe from any thread,
    /// and a token stays cancelled.
    pub fn cancel(&self) {
        self.core.cancel();
    }

    /// Whether [`CancelToken::cancel`] has been called.
    pub fn is_cancelled(&self) -> bool {
        self.core.is_cancelled()
    }
}

impl CancelToken {
    /// The core's token, sharing this one's flag.
    pub(crate) fn core(&self) -> craftbridge_core::CancelToken {
        self.core.clone()
    }
}
