//! The universal runtime that hosts one Craft app session on a phone.
//!
//! It knows nothing about any particular Craft app: an app plugin implements its interface and the
//! core runs it on its own thread, with the GPU device, folders, progress and cancellation.

pub mod app;
pub mod engine;
pub mod error;
pub mod folders;
pub mod frames;

pub use app::CraftApp;
pub use engine::Engine;
pub use error::CraftError;
pub use frames::{FrameBytes, PixelFormat};

/// The result of a runtime call: what the call answers, or the [`CraftError`] it failed with. The
/// error has a default so a signature can name just its answer, as the plan's runtime sketches write it.
pub type Result<T, E = CraftError> = std::result::Result<T, E>;
