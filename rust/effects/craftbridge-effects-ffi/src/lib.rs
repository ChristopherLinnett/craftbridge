//! The EffectCraft distribution library: the one native library an app links.
//!
//! Two Rust static libraries in one app collide on the standard library's symbols, so the core's and
//! the plugin's exports are linked here, once.

// The scaffolding UniFFI generates for the exported call needs `unsafe`, which the workspace's lints
// otherwise deny. This crate writes none of its own.
#![allow(unsafe_code)]

use craftbridge_core::CraftApp;
use craftbridge_core::engine::guarded;
use craftbridge_core_ffi::{CraftError, Engine, Folders, GpuChoice};
use std::sync::Arc;

// A record or an object carries its conversions into every UniFFI boundary that links it, but an
// error declared `remote` carries its own only into the boundary that declared it. This forwarding
// is what lets the call fail with the core's `CraftError` through the core's conversions: the library
// answers one `CraftError`, named in one namespace, which is what library mode and the bindings read.
uniffi::use_remote_type!(craftbridge_core_ffi::CraftError);
// The same holds for the app's GPU choice, a `remote` enum the core declared.
uniffi::use_remote_type!(craftbridge_core_ffi::GpuChoice);

/// Start an EffectCraft engine in the app's `folders`, answering its handle or the [`CraftError`]
/// that refused it: folders that would not prepare, an app that would not build from them, a panic
/// while it was built (`Internal`), or the thread the engine runs on refusing to start.
///
/// The app it starts is EffectCraft's, built for the folders the app names (plan §19), and it starts
/// with the core's `Engine::start_in`, so every export the engine runs from then on is journaled:
/// the start deals first with the files the pending-export journal still lists, deleting the ones an
/// export the system killed wrote and leaving the ones it never reached, and clears the journal
/// (plan §16, D-17); [`Engine::recovery`] reports what it did.
///
/// `gpu` is the app's choice of renderer (plan §14, §17). `Preferred` has the engine ask for the
/// machine's GPU device on its own thread, attach it, and name it in `info.gpu`; where the machine
/// gives none, or when it is lost, the engine renders on the CPU and `info.gpu` says why. `CpuOnly`
/// asks for no device: every render runs on the CPU and `info.gpu` gives that as the reason.
#[uniffi::export]
pub fn start_effects_engine(folders: Folders, gpu: GpuChoice) -> Result<Arc<Engine>, CraftError> {
    start(folders, gpu, craftbridge_effects::EffectCraftApp::new)
}

/// Start the engine of the app `build` makes from `folders`, with the core's `Engine::start_in` and the app's `gpu` choice.
///
/// The app is built on the caller's thread, before an engine thread exists to guard it, and building
/// it runs the Craft app's own code, so the build runs behind the guard the engine runs every call
/// behind: a panic in it is the start's `Internal`, as it is any call's. Unguarded, UniFFI would catch
/// it and throw an error of its own instead (Swift's `rustPanic`, Kotlin's `InternalException`),
/// which an app catching the `CraftError` the call declares would not catch.
fn start<A: CraftApp + 'static>(
    folders: Folders,
    gpu: GpuChoice,
    build: impl FnOnce(&craftbridge_core::folders::Folders) -> craftbridge_core::Result<A>,
) -> Result<Arc<Engine>, CraftError> {
    let folders = craftbridge_core::folders::Folders::from(folders);
    let core = guarded(|| craftbridge_core::Engine::start_in(&folders, build(&folders)?, gpu)).flatten()?;
    Ok(Engine::from_core(core))
}

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests;
