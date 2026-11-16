//! The PD13 plug-in switch: plug-ins are off unless the app names a plug-in folder.
//!
//! The app switches plug-ins by what it passes as `Folders.plugins`: none keeps the loader and the
//! previewer off the session entirely; a folder lets EffectCraft's own WebAssembly loader answer,
//! and the engine's confinement (L5) already refuses reading a module from anywhere outside the
//! roots the folders derive. The per-platform policy of where that folder may live (the iOS bundle
//! only) arrives with the platform loaders; the switch itself is the whole of this file.

use craftbridge_core::folders::Folders;
use effectcraft_engine::{PluginLoader, PluginPreviewer};

/// The two session fields the switch fills.
pub(crate) struct Hooks {
    pub loader: Option<PluginLoader>,
    pub previewer: Option<PluginPreviewer>,
}

/// The session's plug-in hooks for these folders: none when the app names no plug-in folder (PD13),
/// EffectCraft's own loader gated on this build carrying the runtime otherwise.
pub(crate) fn hooks(folders: &Folders) -> Hooks {
    let on = folders.plugins.is_some() && effectcraft_plugin::wasm_available();
    Hooks { loader: on.then_some(effectcraft_plugin::loader), previewer: on.then_some(effectcraft_plugin::preview) }
}
