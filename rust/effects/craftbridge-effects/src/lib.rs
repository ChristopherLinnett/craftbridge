//! The EffectCraft plugin: everything the EffectCraft command line does, as the method table the
//! universal core serves.
//!
//! The plugin builds its own EffectCraft session (AD1): it fills the session's public fields from
//! the app's [`Folders`](craftbridge_core::folders::Folders) — the models folder, the media pool,
//! the importer, the expression engine, the scripting runtime, and the plug-in loader only where the
//! app names a plug-in folder (PD13) — and runs it headless, confined before its first command to
//! the container as the write root with the container and bundle as read roots (L5, AD9). No helper
//! hooks exist: a helper is a separate process, and a phone starts none, so the commands that need
//! one answer `Unsupported`, as do the model downloads, which would start the system `curl` (AD11).
//!
//! Every call reaches the session through the MCP server built at construction, so the `mcp`
//! method's line transport and the method table act on one session. The methods the plugin answers
//! are the plan's §14 rows its one method list names, each with the result shape and failure words
//! of the EffectCraft command line at the pin (AD10); any other name `request` is given answers
//! `Unsupported`, naming what was asked and the methods `request` answers. The binary `frame` is
//! answered only through [`CraftApp::frame`](craftbridge_core::CraftApp::frame), since pixels cannot
//! come back through `request`'s JSON; its request is resolved as the command line's `render-frame`
//! resolves its flags (AD4).

mod app;
/// What the generator writes, one file to a module under `src/generated/`.
mod generated {
    pub(crate) mod version;
}
mod info;
mod methods;
mod plugins;
mod reference;
mod requests;
mod session;

pub use app::EffectCraftApp;
