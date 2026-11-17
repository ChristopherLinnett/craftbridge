//! Code that calls into the platform's own media frameworks (a phone's, and a Mac's, where the encoders' host
//! tests run them), the only code of this crate that may hold `unsafe` (AGENTS.md §4). Each backend is one
//! module; a backend's pointer handling stays inside it, and what it hands the rest of the crate is safe.

#[cfg(target_vendor = "apple")]
pub(crate) mod audioconverter;
#[cfg(target_os = "android")]
pub(crate) mod mediacodec;
#[cfg(target_os = "ios")]
pub(crate) mod videotoolbox;
#[cfg(target_vendor = "apple")]
pub(crate) mod videotoolbox_encoder;

/// `Ok` when the `status` an Apple framework call returned says success; otherwise what failed, naming the
/// `call`. The one wording of a framework failure, a decoder's or an encoder's, whichever framework it was.
#[cfg(target_vendor = "apple")]
pub(crate) fn check(status: i32, call: &str) -> Result<(), String> {
    if status == 0 { Ok(()) } else { Err(format!("{call} failed ({status})")) }
}
