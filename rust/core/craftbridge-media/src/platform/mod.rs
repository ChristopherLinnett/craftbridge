//! Code that calls into a phone's own media frameworks, the only code of this crate that may hold
//! `unsafe` (AGENTS.md §4). Each backend is one module; a backend's pointer handling stays inside it, and
//! what it hands the rest of the crate is safe.

#[cfg(target_os = "android")]
pub(crate) mod mediacodec;
#[cfg(target_os = "ios")]
pub(crate) mod videotoolbox;
