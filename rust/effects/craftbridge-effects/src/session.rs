//! The construction of CraftBridge's own EffectCraft session (AD1).
//!
//! EffectCraft's own host builder takes its folders from `EFFECTCRAFT_CONFIG_DIR` or the desktop
//! conventions and starts external helper processes; a phone has neither. This builds the same
//! session through the engine's public fields alone: the app's [`Folders`] supply what the desktop
//! host derives, the media importer is EffectCraft's own and the exporter is CraftBridge's (AD7), and the
//! one thing deliberately missing is the helper hooks, so a command that wants a helper finds none
//! and answers that helpers are not available, and nothing here can start a process.

use std::sync::Arc;

use craftbridge_core::Platform;
use craftbridge_core::folders::Folders;
use effectcraft_engine::project::Footage;
use effectcraft_engine::{Importer, Session};

use crate::hardware::{HardwareChoice, HardwareExporter};
use crate::{fonts, plugins};

/// The importer EffectCraft's host uses: a probe is EffectCraft's own media probe, so a phone reads
/// footage exactly as the command line does (AD1).
struct MediaImporter;

impl Importer for MediaImporter {
    fn probe(&self, path: &str) -> std::result::Result<Footage, String> {
        effectcraft_media::probe(path).map_err(|error| error.to_string())
    }
}

/// The exporter seam: the one place the session's exporter is chosen. It is CraftBridge's own (AD7):
/// EffectCraft's file exporter, the one the command line exports through, run on a pool the phone's
/// memory can feed, with the device's hardware encoder for the H.264 and HEVC movies `choice` says a request
/// chose it for.
fn exporter(choice: HardwareChoice) -> Arc<HardwareExporter> {
    Arc::new(HardwareExporter::new(choice))
}

/// Build the session (plan §19, steps 1 and 2): every field from public inputs, the plug-in hooks from the
/// PD13 switch for `platform`, no helper hooks, and then EffectCraft's own script-runner install. The
/// platform is a parameter so a test can build the iOS session's shape on the host and pin its
/// policy there, while the gate's phone builds prove the code compiles for the phones. The exporter
/// encodes in hardware while `choice` says the request being answered chose it.
pub(crate) fn build(platform: Platform, folders: &Folders, choice: &HardwareChoice) -> Session {
    let hooks = plugins::hooks_for(platform, folders);
    let mut session = Session {
        models_dir: Some(folders.models.clone()),
        exporter: Some(exporter(choice.clone())),
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
    // runner and ScriptUI dispatches over the commands, leaving every switch at its default. No
    // call of the phone's reaches the runner: the confinement refuses every command that runs a
    // script, and a script would reach the engine past it (PD13).
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

    /// A headless session built as the app builds it, with `commands` run on it, which a test of a
    /// runner or a listing fills its project with.
    pub(crate) fn backend_running(commands: impl IntoIterator<Item = (&'static str, serde_json::Value)>) -> effectcraft_automation::Backend {
        let mut backend = effectcraft_automation::Backend::headless(build(Platform::current(), &folders(None), &HardwareChoice::default()));
        for (command, params) in commands {
            backend.exec(command, params).unwrap_or_else(|error| panic!("{command}: {error}"));
        }
        backend
    }

    /// An app's folders on disk, in a scratch folder of their own: the container with its working
    /// folders, a bundle beside it when asked for, and the plug-in folder the app names, given
    /// relative to the scratch folder, made. Each test makes its own, so nothing one test puts in
    /// a folder sits in another test's.
    pub(crate) struct Scratch {
        dir: tempfile::TempDir,
        pub(crate) folders: Folders,
    }

    impl Scratch {
        pub(crate) fn new(bundle: bool, plugins: Option<&std::path::Path>) -> Self {
            let dir = tempfile::tempdir().expect("a scratch folder");
            let container = dir.path().join("container");
            let mut folders = folders(plugins.map(|named| dir.path().join(named)));
            folders.bundle = bundle.then(|| dir.path().join("App.app"));
            for folder in folders.bundle.iter().chain(&folders.plugins) {
                std::fs::create_dir_all(folder).expect("the read-only folder appears");
            }
            folders.config = container.join(".config");
            folders.cache = container.join(".cache");
            folders.models = container.join(".models");
            folders.container = container;
            Self { dir, folders }
        }

        /// The folder the scratch sits in, where each test puts what it names.
        pub(crate) fn root(&self) -> &std::path::Path {
            self.dir.path()
        }

        /// The app as it is built for `platform`, through the one construction
        /// `EffectCraftApp::new` makes for its own: so every call goes through the app's own door,
        /// its own confinement and its own admission of the plug-in folder.
        pub(crate) fn app(&self, platform: Platform) -> crate::EffectCraftApp {
            crate::EffectCraftApp::for_platform(platform, &self.folders).expect("the app starts from prepared folders")
        }
    }

    #[test]
    fn the_session_exports_through_craftbridges_exporter() {
        // The seam hands out one exporter type everywhere; what a platform changes is only what that
        // exporter can reach at runtime — a pool the memory sizes, a hardware encoder the device has —
        // so this test pins the type, and the export tests pin what an export of each kind comes to.
        let _: Arc<HardwareExporter> = exporter(HardwareChoice::default());
        assert!(build(Platform::current(), &folders(None), &HardwareChoice::default()).exporter.is_some());
    }

    #[test]
    fn the_session_carries_no_helper_hooks() {
        // The acceptance: no helper process can start, because the hooks that start one are not there.
        let session = build(Platform::current(), &folders(None), &HardwareChoice::default());
        assert!(session.helpers.is_none(), "a phone's session must hold no helper hooks at all");
    }

    #[test]
    fn the_session_is_built_as_effectcrafts_own_host_builds_it() {
        let session = build(Platform::current(), &folders(None), &HardwareChoice::default());
        assert!(session.script.is_some(), "EffectCraft's own install is made, though the confinement refuses every command that reaches it");
    }

    #[test]
    fn the_session_keeps_its_models_in_the_apps_folder() {
        let session = build(Platform::current(), &folders(None), &HardwareChoice::default());
        assert_eq!(session.models_dir.as_deref(), Some(std::path::Path::new("/app/models")));
    }

    #[test]
    fn plug_ins_are_off_without_a_folder() {
        let session = build(Platform::current(), &folders(None), &HardwareChoice::default());
        assert!(session.plugin_loader.is_none());
        assert!(session.plugin_previewer.is_none());
        assert!(session.plugins_dir.is_none());
    }

    #[test]
    fn plug_ins_are_on_with_a_folder() {
        let plugins = std::path::PathBuf::from("/app/plug-ins");
        let session = build(Platform::current(), &folders(Some(plugins.clone())), &HardwareChoice::default());
        // The engine answers with EffectCraft's own runtime gate on top of the app's switch.
        assert_eq!(session.plugin_loader.is_some(), effectcraft_plugin::wasm_available());
        assert_eq!(session.plugin_previewer.is_some(), effectcraft_plugin::wasm_available());
        assert_eq!(session.plugins_dir.as_deref(), Some(plugins.as_path()));
    }
}
