//! The universal core's foreign-function boundary, exported to Swift and Kotlin through UniFFI.
//!
//! It is a plain library: an app's distribution crate links it into the one native library.
//!
//! Every call of an [`Engine`] — `request`, `request_with_progress`, `frame`, `mcp`, `suspend_gpu`,
//! `resume_gpu`, `present`, `detach_preview`, `attach_window` and `shut_down` — and
//! `attach_metal_layer`, which takes the engine, is an `async` function that fails by throwing
//! [`CraftError`], never by answering with one: a closed engine and a panic the engine caught throw
//! as `request` does, so a reply the app never wrote is never read as one, and an `mcp` line that
//! needs no reply answers `None`.
//!
//! `recovery` answers at once with what the start cleaned up after an export the system killed.
//!
//! No value crosses without a copy: only an object crosses as a handle. UniFFI serialises every
//! record, a [`FrameBytes`] with its `data` included, into a buffer it hands to the foreign side,
//! and Kotlin's `ByteArray` lives on the JVM heap and Swift's `Data` in memory Swift owns, so a
//! frame's bytes are copied at least once on the way out of Rust, and the boundary promises no
//! zero-copy. A value UniFFI cannot carry, a frame or an answer of about `i32::MAX` bytes, is refused
//! with `InvalidParams` before it is lowered (see [`MAX_FRAME_BYTES`], [`MAX_ANSWER_BYTES`] and
//! [`MAX_MCP_LINE_BYTES`]).

// The scaffolding UniFFI generates needs `unsafe`, which the workspace's lints otherwise deny. What
// unsafe this crate writes of its own — taking in the app's layer and window, the JNI call that hands
// a window over, and the wgpu surfaces made from them — sits under `platform` alone, each block
// carried by a `// SAFETY:` reason (plan §20).
#![allow(unsafe_code)]

mod blocking;
mod cancel;
mod crossing;
mod engine;
mod error;
mod folders;
mod frames;
mod gpu;
mod platform;
mod progress;
mod recovery;
#[cfg(test)]
mod surface_tests;

pub use cancel::CancelToken;
pub use crossing::{MAX_ANSWER_BYTES, MAX_FRAME_BYTES, MAX_MCP_LINE_BYTES};
pub use engine::Engine;
pub use error::CraftError;
pub use folders::Folders;
pub use frames::{FrameBytes, PixelFormat};
pub use gpu::GpuChoice;
pub use progress::ProgressListener;
pub use recovery::{Recovery, RecoveryNote};

uniffi::setup_scaffolding!();
