//! The platform's own calls, confined to the one module that may hold them (AGENTS.md §4): the code
//! that asks a phone's own system a question, and the only code of this crate that may hold `unsafe`.
//! Each question is one module, and what it hands the rest of the crate is safe.
//!
//! An app may not know its fonts by folder: iOS keeps some of them inside private frameworks, and
//! what an app is shown is CoreText's answer, not a directory listing (V1). So the platform asks
//! its system here, in plain terms the layers above can filter with, and the pointer work to the
//! system frameworks never leaves this folder.
//!
//! `memory` builds on every target, since each one answers (or answers that it has no figure);
//! `cpu` builds on every target too, and answers that there is no floor to meet everywhere but Android
//! arm64; `coretext` is iOS's alone, because no other platform lists its fonts through the system.

#[cfg(target_os = "ios")]
pub use coretext::{CoreTextFace, available_family_names, coretext_faces};

#[cfg(target_os = "ios")]
mod coretext;
pub(crate) mod cpu;
pub(crate) mod memory;
