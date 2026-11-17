//! The error every exported call throws.

pub use craftbridge_core::CraftError;

// The core's own error, exported as it is defined there, so the boundary has no second copy of its
// variants to fall out of step with the plan's mapping table (§14). The compiler refuses this
// declaration if it stops matching the core's. Each variant's `message` crosses as `detail`
// (`uniffi.toml` says why).
#[uniffi::remote(Error)]
pub enum CraftError {
    InvalidParams { message: String },
    CommandFailed { command: String, message: String, step: Option<u32> },
    NotFound { message: String },
    Unsupported { message: String },
    Cancelled,
    Io { message: String },
    Gpu { message: String },
    Internal { message: String },
}
