//! The universal runtime that hosts one Craft app session on a phone.
//!
//! It knows nothing about any particular Craft app: an app plugin implements its interface and the
//! core runs it on its own thread, with the GPU device, folders, progress and cancellation.

pub mod error;
pub mod folders;
