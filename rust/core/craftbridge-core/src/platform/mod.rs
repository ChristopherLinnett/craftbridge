//! The platform's own calls, confined to the one module that may hold them (AGENTS.md §4).
//!
//! An app may not know its fonts by folder: iOS keeps some of them inside private frameworks, and
//! what an app is shown is CoreText's answer, not a directory listing (V1). So the platform asks
//! its system here, in plain terms the layers above can filter with, and the pointer work to the
//! system frameworks never leaves this folder.

pub use coretext::{CoreTextFace, available_family_names, coretext_faces};

mod coretext;
