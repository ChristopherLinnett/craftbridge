//! The universal runtime that hosts one Craft app session on a phone.
//!
//! It knows nothing about any particular Craft app: an app plugin implements its interface and the
//! core runs it on its own thread, with the GPU device, folders, progress and cancellation.

pub mod app;
pub mod blit;
pub mod cancel;
pub mod engine;
pub mod error;
pub mod export;
pub mod folders;
pub mod frames;
pub mod gpu;
pub mod journal;
pub mod surface;
#[cfg(test)]
mod testing;

/// The platform's own calls, the one place hand-written `unsafe` may sit in this crate (AGENTS.md §4):
/// how much memory the app may still allocate on every target, and on iOS the faces CoreText offers an
/// app (plan §19, AD2, AD7).
#[allow(unsafe_code)]
pub mod platform;

pub use app::CraftApp;
pub use cancel::CancelToken;
pub use engine::{Answer, Engine};
pub use error::CraftError;
pub use export::PROGRESS_INTERVAL;
pub use frames::{FrameBytes, PixelFormat};
pub use gpu::{GpuChoice, GpuDevice, GpuFailure, Platform};
pub use journal::{JOURNAL_FILE, Recovery};
pub use platform::memory::available_memory;
pub use surface::{Presented, PreviewSurface};

/// The result of a runtime call: what the call answers, or the [`CraftError`] it failed with. The
/// error has a default so a signature can name just its answer, as the plan's runtime sketches write it.
pub type Result<T, E = CraftError> = std::result::Result<T, E>;
