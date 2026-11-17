//! The construction of CraftBridge's own EffectCraft session (AD1).
//!
//! EffectCraft's own host builder takes its folders from `EFFECTCRAFT_CONFIG_DIR` or the desktop
//! conventions and starts external helper processes; a phone has neither. This builds the same
//! session through the engine's public fields alone: the app's [`Folders`] supply what the desktop
//! host derives, the media importer and the Render-Queue exporter are EffectCraft's own, and the
//! one thing deliberately missing is the helper hooks, so a script or a command that wants a helper
//! finds none and answers that helpers are not available, and nothing here can start a process.

use std::sync::Arc;

use craftbridge_core::folders::Folders;
use effectcraft_engine::project::Footage;
use effectcraft_engine::{Exporter, Importer, Session};

use crate::{fonts, plugins};

/// The importer EffectCraft's host uses: a probe is EffectCraft's own media probe, so a phone reads
/// footage exactly as the command line does (AD1).
struct MediaImporter;

impl Importer for MediaImporter {
    fn probe(&self, path: &str) -> std::result::Result<Footage, String> {
        effectcraft_media::probe(path).map_err(|error| error.to_string())
    }
}

/// The exporter seam: the one place the session's exporter is chosen, so the phone's own exporter
/// (AD7) takes over every export here and nowhere else. It is EffectCraft's file exporter, the one
/// the command line exports through, so every export the session makes is EffectCraft's own.
fn exporter() -> Arc<dyn Exporter> {
    Arc::new(effectcraft_host::FileExporter::default())
}

/// Build the session (plan §19, steps 1 and 2): every field from public inputs, the plug-in hooks from the
/// PD13 switch, no helper hooks, and then EffectCraft's own scripting install.
pub(crate) fn build(folders: &Folders) -> Session {
    let hooks = plugins::hooks(folders);
    let mut session = Session {
        models_dir: Some(folders.models.clone()),
        exporter: Some(exporter()),
        footage: Arc::new(effectcraft_media::MediaPool::new()),
        importer: Some(Arc::new(MediaImporter)),
        expr: Some(Arc::new(effectcraft_expr::Expressions)),
        expr_check: Some(effectcraft_expr::check_syntax),
        plugin_loader: hooks.loader,
        plugin_previewer: hooks.previewer,
        plugins_dir: folders.plugins.clone(),
        // The session says how it renders, as EffectCraft's headless front ends have it say. The
        // command line's note ends "unless started with --gpu"; this session has no start flags and
        // no GPU attached, so the note says the plain truth.
        accel_note: Some("headless: renders on the CPU".into()),
        // The desktop host fills `helpers` with hooks that start helper processes; a phone starts
        // none, so the field keeps its default and every command that needs a helper finds none.
        ..Default::default()
    };
    // The one call EffectCraft's own host makes after filling the fields: it installs the script
    // runner and ScriptUI dispatches over the commands, leaving every switch at its default.
    effectcraft_script::install(&mut session);
    // AD2 (plan §19, step 1): then the phone's fonts. The platform's font folders are registered
    // through EffectCraft's one registration entry, and the font-list filter is set from the same
    // answer, before the session serves its first command.
    fonts::register(folders);
    session
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The folders as a value: building the session reads only the names it is given, never the
    /// disk, so these need not exist (the tests that drive a session prepare real ones). Shared with the
    /// tests of other modules that build a session the same way.
    pub(crate) fn folders(plugins: Option<std::path::PathBuf>) -> Folders {
        Folders {
            container: "/app".into(),
            bundle: None,
            config: "/app/config".into(),
            cache: "/app/cache".into(),
            models: "/app/models".into(),
            plugins,
            fonts: Vec::new(),
        }
    }

    #[test]
    fn the_session_carries_no_helper_hooks() {
        // The acceptance: no helper process can start, because the hooks that start one are not there.
        let session = build(&folders(None));
        assert!(session.helpers.is_none(), "a phone's session must hold no helper hooks at all");
    }

    #[test]
    fn the_session_runs_scripting() {
        let session = build(&folders(None));
        assert!(session.script.is_some(), "scripts run on both platforms (PD13)");
    }

    #[test]
    fn the_session_keeps_its_models_in_the_apps_folder() {
        let session = build(&folders(None));
        assert_eq!(session.models_dir.as_deref(), Some(std::path::Path::new("/app/models")));
    }

    #[test]
    fn plug_ins_are_off_without_a_folder() {
        let session = build(&folders(None));
        assert!(session.plugin_loader.is_none());
        assert!(session.plugin_previewer.is_none());
        assert!(session.plugins_dir.is_none());
    }

    #[test]
    fn plug_ins_are_on_with_a_folder() {
        let plugins = std::path::PathBuf::from("/app/plug-ins");
        let session = build(&folders(Some(plugins.clone())));
        // The engine answers with EffectCraft's own runtime gate on top of the app's switch.
        assert_eq!(session.plugin_loader.is_some(), effectcraft_plugin::wasm_available());
        assert_eq!(session.plugin_previewer.is_some(), effectcraft_plugin::wasm_available());
        assert_eq!(session.plugins_dir.as_deref(), Some(plugins.as_path()));
    }
}
