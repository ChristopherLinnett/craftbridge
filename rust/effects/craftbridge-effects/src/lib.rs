//! The EffectCraft plugin: everything the EffectCraft command line does, as the method table the
//! universal core serves.
//!
//! The plugin builds its own EffectCraft session (AD1): it fills the session's public fields from
//! the app's [`Folders`](craftbridge_core::folders::Folders) — the models folder, the media pool,
//! the importer, the expression engine, EffectCraft's own script-runner install, and the plug-in
//! loader only where the app names a plug-in folder (PD13) — and runs it headless, confined before its first command to
//! the container as the write root with the container and bundle as read roots (L5, AD9). No helper
//! hooks exist: a helper is a separate process, and a phone starts none, so the commands that need
//! one answer `Unsupported`, as do the model downloads, which would start the system `curl` (AD11), and the renders and fills no one has
//! listed the files of for the pending-export journal (AD3, AD20, §19).
//!
//! Every call reaches the session through the MCP server built at construction, so the `mcp` call's
//! line transport and the method table act on one session. The methods the plugin answers are the
//! plan's §14 rows its method lists name — one for `request`, `replay` and `timings` among them, and
//! one for `request_with_progress`, which answers `export` through the render runner (AD3),
//! `template.render` through the template runner (AD5) and `contentFill.generate` through the fill
//! runner (AD20) — each
//! with the result shape and failure words of the EffectCraft command line at the pin (AD10); `mcp`
//! is a call of its own, not a row of either list, and `script` is not a method of the phone build
//! (PD13): a script's commands reach the session past the confinement, so an app runs the engine's
//! calls in succession instead; any other name either call is given answers
//! `Unsupported`, naming what was asked and the methods that call answers. The binary `frame` is
//! answered only through [`CraftApp::frame`](craftbridge_core::CraftApp::frame), since pixels cannot
//! come back through `request`'s JSON; its request is resolved as the command line's `render-frame`
//! resolves its flags (AD4), or, when its parameters name a `template`, of that template's instance
//! (AD5).
//!
//! At engine start the plugin registers the phone's font folders with EffectCraft and takes the
//! font listing every app-facing call answers from the platform's own answer (AD2), and takes back
//! the temporary files a killed write left in the engine's folders, which no journal could list
//! (AD21).

mod app;
mod exporter;
mod files;
mod fill;
mod fonts;
mod hardware;
mod imports;
/// What the generator writes, one file to a module under `src/generated/`.
mod generated {
    pub(crate) mod version;
}
mod info;
mod mcp;
mod methods;
mod orphans;
mod outputs;
mod plugins;
mod properties;
mod reference;
mod replay;
mod requests;
mod runner;
mod session;
mod staging;
mod templates;
#[cfg(test)]
mod test_plugin;
mod timings;

pub use app::EffectCraftApp;
pub use requests::{CompRef, RenderRequest, Resolution};
