//! The PD13 plug-in policy (plan §19): plug-ins are off unless the app names a plug-in folder,
//! and where they are on, which loader answers is the platform's own rule.
//!
//! Without a plug-in folder the loader and the previewer stay off the session entirely. With one,
//! Android and the desktop answer with EffectCraft's own WebAssembly loader — the engine's
//! confinement (L5) already refuses reading a module from anywhere outside the roots the folders
//! derive — and iOS answers with [`bundle_loader`], which takes a module only from the plug-in
//! folder the app named inside its bundle (PD13), because on iOS a script or a command must not be
//! able to load a plug-in out of the container even though the confinement lets the file through
//! (T24).

use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

use craftbridge_core::Platform;
use craftbridge_core::folders::Folders;
use effectcraft_engine::{PluginLoader, PluginPreviewer};

/// The two session fields the switch fills.
pub(crate) struct Hooks {
    pub loader: Option<PluginLoader>,
    pub previewer: Option<PluginPreviewer>,
}

/// The plug-in folders [`bundle_loader`] takes modules from, each resolved to its real path: a
/// `PluginLoader` is a plain function pointer, so the engine holds it with no context to read the
/// folders from, and the folders it needs must live where the function can reach them (plan §19).
/// Each iOS engine holds an entry for as long as it lives (see [`Admission`]), so a second engine
/// (§16) loads from the folder it named and a dropped engine leaves nothing behind. Every entry lies
/// inside the process's one app bundle, so the engines that live together see one another's folders
/// and no entry widens PD13. A folder two engines name has two entries, one for each of them.
static BUNDLED: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// The refusal of a module from anywhere but a bundled plug-in folder. It ends with the engine's
/// own "not available in this build" sentence so §14 takes it onto the `Unsupported` row, beside
/// every other capability a build is put together without.
const OUTSIDE_THE_BUNDLE: &str = "WebAssembly effect plug-ins from outside the app bundle's plug-in folder are not available in this build";

/// The policy of §19 as a plain function of the platform and the folders, so the tests pin every
/// platform's choice on the host while the gate's phone builds prove the code compiles for the
/// phones. The app's folder switch (PD13) comes first: no plug-in folder, no hooks, on any
/// platform. Then the platform's rule: iOS takes the bundle gate, Android and the desktop take
/// EffectCraft's own loader, the confinement guarding which files reach it.
pub(crate) fn hooks_for(platform: Platform, folders: &Folders) -> Hooks {
    let on = folders.plugins.is_some() && effectcraft_plugin::wasm_available();
    let loader = match platform {
        // Both arms name a `PluginLoader` pointer: `then_some` keeps a function's own item type,
        // which only coerces to the pointer where a context asks, so the cast is written here.
        Platform::Ios => on.then_some(bundle_loader as PluginLoader),
        Platform::Android | Platform::Host => on.then_some(effectcraft_plugin::loader as PluginLoader),
    };
    Hooks { loader, previewer: on.then_some(effectcraft_plugin::preview) }
}

/// An engine's place in [`BUNDLED`], held by the app for as long as the engine lives: dropping it
/// takes the engine's one entry out again, so the iOS gate admits a folder exactly while an engine
/// that named it is alive. It holds nothing on any platform but iOS, and nothing for an iOS app
/// whose plug-in folder does not lie inside its bundle.
pub(crate) struct Admission(Option<PathBuf>);

impl Drop for Admission {
    fn drop(&mut self) {
        let Some(folder) = self.0.take() else {
            return;
        };
        let mut bundled = BUNDLED.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = bundled.iter().position(|held| *held == folder) {
            bundled.remove(at);
        }
    }
}

/// Give the iOS gate this app's plug-in folder, once its folders are prepared (the folder must be
/// there to be resolved). Only the iOS loader reads the list, so no other platform adds to it, and
/// on iOS a folder the app names outside its bundle is never added: PD13 loads plug-ins there only
/// from files the host app bundles, so such an app's own folder admits nothing, and it loads only
/// what another engine alive in the process has admitted from the bundle. The entry stays while the
/// returned [`Admission`] does.
pub(crate) fn admit(platform: Platform, folders: &Folders) -> Admission {
    let folder = (platform == Platform::Ios).then(|| bundled_plugin_folder(folders)).flatten();
    if let Some(folder) = &folder {
        BUNDLED.write().unwrap_or_else(PoisonError::into_inner).push(folder.clone());
    }
    Admission(folder)
}

/// The app's plug-in folder, resolved, when it lies inside the app's bundle, also resolved: each
/// is compared as the disk has it, so neither a link nor another spelling of the same folder
/// decides what counts as bundled.
fn bundled_plugin_folder(folders: &Folders) -> Option<PathBuf> {
    let bundle = folders.bundle.as_deref()?.canonicalize().ok()?;
    let plugins = folders.plugins.as_deref()?.canonicalize().ok()?;
    plugins.starts_with(&bundle).then_some(plugins)
}

/// The iOS loader (PD13, plan §19): it refuses every `source` that does not resolve inside a
/// plug-in folder an app named in its bundle, and otherwise calls EffectCraft's own loader, so a
/// WebAssembly effect loads from the files the host app bundled and from nowhere else (T24). The
/// source is judged by where it resolves, the file the engine has just read, not by how it is
/// spelt: a link inside the folder that leads out of it is refused, and another spelling of the
/// bundled folder is the folder.
fn bundle_loader(bytes: &[u8], source: &str) -> std::result::Result<serde_json::Value, String> {
    let inside =
        Path::new(source).canonicalize().is_ok_and(|real| BUNDLED.read().unwrap_or_else(PoisonError::into_inner).iter().any(|folder| real.starts_with(folder)));
    if !inside {
        return Err(OUTSIDE_THE_BUNDLE.to_string());
    }
    effectcraft_plugin::loader(bytes, source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EffectCraftApp;
    use crate::session::tests::{Scratch, folders};
    use crate::test_plugin;
    use craftbridge_core::{CraftApp, CraftError, GpuChoice};
    use serde_json::{Value, json};

    /// The switch is off on every platform when the app names no plug-in folder (PD13).
    #[test]
    fn no_folder_no_hooks_on_any_platform() {
        for platform in [Platform::Ios, Platform::Android, Platform::Host] {
            let hooks = hooks_for(platform, &folders(None));
            assert!(hooks.loader.is_none(), "{platform:?} answered a loader with no folder");
            assert!(hooks.previewer.is_none());
        }
    }

    /// Whether a policy's loader is exactly this function. Function pointers have no `Debug`, so
    /// the answer is a fact, not a `left == right` printout; pointer equality itself goes through
    /// `fn_addr_eq`, the one comparison Rust settles without a warning.
    fn names(loader: Option<PluginLoader>, answer: PluginLoader) -> bool {
        loader.is_some_and(|f| std::ptr::fn_addr_eq(f, answer))
    }

    /// Each platform's rule, pinned by pointer so a swapped arm fails here on the host: iOS is
    /// gated to the bundle, Android and the desktop answer with EffectCraft's own loader (T24).
    /// The runtime gate stands on all three: without the WebAssembly feature no loader is named.
    #[test]
    fn each_platform_picks_its_loader() {
        let with_folder = folders(Some("/app/plug-ins".into()));
        let on = effectcraft_plugin::wasm_available();
        assert!(names(hooks_for(Platform::Ios, &with_folder).loader, bundle_loader as PluginLoader), "iOS must pick the bundle gate");
        assert!(names(hooks_for(Platform::Android, &with_folder).loader, effectcraft_plugin::loader), "Android must pick EffectCraft's own loader");
        assert!(names(hooks_for(Platform::Host, &with_folder).loader, effectcraft_plugin::loader), "the desktop must pick EffectCraft's own loader");
        // The previewer is the same everywhere: its only callers are the Plug-in Library's
        // `library.*` commands, which download their module and which the confinement withholds,
        // so no file this policy guards ever reaches it.
        assert_eq!(hooks_for(Platform::Ios, &with_folder).previewer.is_some(), on);
        assert_eq!(hooks_for(Platform::Android, &with_folder).previewer.is_some(), on);
    }

    /// Write a probe module with this id at `path`, making its folder; the path, as text, back.
    fn module(path: &Path, id: &str) -> String {
        std::fs::create_dir_all(path.parent().expect("a folder to make")).expect("the module's folder appears");
        std::fs::write(path, test_plugin::module(id)).expect("the module is written");
        path.to_string_lossy().into_owned()
    }

    fn load(app: &mut EffectCraftApp, path: &str) -> Result<Value, CraftError> {
        app.request("exec", json!({"command": "effect.plugins.load", "params": {"path": path}}))
    }

    /// The iOS refusal through the door, as `effect.plugins.load` reports a refused file: the path,
    /// then the gate's words (`crates/engine/src/cmd/effect.rs`:63-66), on the `Unsupported` row.
    fn refused_by_the_gate(app: &mut EffectCraftApp, path: &str) {
        match load(app, path) {
            Err(CraftError::Unsupported { message }) => assert_eq!(message, format!("{path}: {OUTSIDE_THE_BUNDLE}")),
            other => panic!("a module from `{path}` must be refused by the iOS gate, got {other:?}"),
        }
    }

    /// T24 on iOS, through a real app: the allowed folder is the one the app named inside its
    /// bundle — `PlugIns`, the name `Folders.appDefault.allowingPlugins(in:)` gives it (§22), not
    /// EffectCraft's own `Plug-ins` — and a module there loads through EffectCraft's own loader,
    /// as the folder's real spelling too; everything else is refused in the gate's words: the
    /// container, which the confinement lets through, the bundle outside the named folder, the
    /// bundle's folder under EffectCraft's name, and a link inside the named folder that leads to
    /// the container.
    #[test]
    fn ios_loads_only_from_the_plug_in_folder_the_app_bundles() {
        let scratch = Scratch::new(true, Some(Path::new("App.app/PlugIns")));
        let bundle = scratch.root().join("App.app");
        let allowed = module(&bundle.join("PlugIns").join("probe.wasm"), "io.github.christopherlinnett.craftbridge.iosprobe");
        let container = module(&scratch.root().join("container").join("probe.wasm"), "io.github.christopherlinnett.craftbridge.iosprobe");
        let elsewhere = module(&bundle.join("Frameworks").join("probe.wasm"), "io.github.christopherlinnett.craftbridge.iosprobe");
        let engine_named = module(&bundle.join("Plug-ins").join("probe.wasm"), "io.github.christopherlinnett.craftbridge.iosprobe");
        let link = bundle.join("PlugIns").join("link.wasm");
        std::os::unix::fs::symlink(&container, &link).expect("a link to the container's module");
        let link = link.to_string_lossy().into_owned();
        let mut app = scratch.app(Platform::Ios);
        for outside in [&container, &elsewhere, &engine_named, &link] {
            refused_by_the_gate(&mut app, outside);
        }
        let loaded = load(&mut app, &allowed).expect("the bundled module loads");
        assert_eq!(loaded.pointer("/loaded/0/name"), Some(&json!("Gate Probe")), "the allowed folder loads: {loaded}");
        // The same file under the folder's real spelling (on macOS a scratch folder's `/var` is
        // `/private/var`) is the same bundled file; a second id, since a process registers one
        // plug-in per id.
        let real = module(
            &bundle.canonicalize().expect("the bundle resolves").join("PlugIns").join("again.wasm"),
            "io.github.christopherlinnett.craftbridge.iosagain",
        );
        assert!(load(&mut app, &real).is_ok(), "another spelling of the bundled folder is that folder");
    }

    /// On iOS a plug-in folder the app names outside its bundle admits nothing (PD13: only files
    /// the host app bundles), even a module in that very folder.
    #[test]
    fn ios_loads_nothing_from_a_folder_outside_the_bundle() {
        let scratch = Scratch::new(true, Some(Path::new("container/plug-ins")));
        let named = module(&scratch.root().join("container").join("plug-ins").join("probe.wasm"), "io.github.christopherlinnett.craftbridge.iosunbundled");
        refused_by_the_gate(&mut scratch.app(Platform::Ios), &named);
    }

    /// How many entries [`BUNDLED`] holds for this folder (as the disk spells it).
    fn held(folder: &Path) -> usize {
        BUNDLED.read().unwrap_or_else(PoisonError::into_inner).iter().filter(|held| *held == folder).count()
    }

    /// The gate's list follows the engines: an iOS engine holds its folder while it lives, two
    /// engines naming one folder hold it twice so the first to go leaves the other's, and a dropped
    /// engine's folder admits nothing afterwards, even to an engine still alive. A phone's engines
    /// are not the only ones that start many: the list must end each test empty-handed.
    #[test]
    fn an_ios_engine_holds_its_plug_in_folder_while_it_lives_and_no_longer() {
        let scratch = Scratch::new(true, Some(Path::new("App.app/PlugIns")));
        let folder = scratch.folders.plugins.as_deref().expect("the app names a folder").canonicalize().expect("the folder resolves");
        let module = module(&folder.join("probe.wasm"), "io.github.christopherlinnett.craftbridge.iosheld");
        assert_eq!(held(&folder), 0, "no engine yet");
        let first = scratch.app(Platform::Ios);
        let second = scratch.app(Platform::Ios);
        assert_eq!(held(&folder), 2, "each engine holds the folder");
        drop(first);
        assert_eq!(held(&folder), 1, "the engine that is gone took only its own entry");
        // A neighbour in the same bundle, naming a folder of its own.
        let mut neighbours = scratch.folders.clone();
        neighbours.plugins = Some(scratch.root().join("App.app").join("More"));
        std::fs::create_dir_all(neighbours.plugins.as_deref().expect("a folder named")).expect("the neighbour's folder appears");
        let mut neighbour = EffectCraftApp::for_platform(Platform::Ios, &neighbours).expect("the neighbour starts");
        assert!(load(&mut neighbour, &module).is_ok(), "an engine alive sees the folder of another engine alive: one bundle, one gate");
        drop(second);
        assert_eq!(held(&folder), 0, "the last engine to go took the folder out");
        refused_by_the_gate(&mut neighbour, &module);
        for platform in [Platform::Android, Platform::Host] {
            let _app = scratch.app(platform);
            assert_eq!(held(&folder), 0, "{platform:?} adds nothing: only the iOS gate reads the list");
        }
    }

    /// On a phone each app lives on its core engine's own thread (§16) and is dropped there: when
    /// the engine closes, or its handle is dropped, which closes it. Two engines naming one folder
    /// each hold it on their own thread, and each takes out its own entry, and only its own, as it
    /// goes; meanwhile the engine still alive loads from the folder through its own door.
    #[test]
    fn an_engine_on_its_own_thread_holds_its_folder_until_it_closes() {
        let scratch = Scratch::new(true, Some(Path::new("App.app/PlugIns")));
        let folder = scratch.folders.plugins.as_deref().expect("the app names a folder").canonicalize().expect("the folder resolves");
        let module = module(&folder.join("probe.wasm"), "io.github.christopherlinnett.craftbridge.iosthreads");
        let closed = craftbridge_core::Engine::start(scratch.app(Platform::Ios), GpuChoice::CpuOnly).expect("the first engine starts");
        let dropped = craftbridge_core::Engine::start(scratch.app(Platform::Ios), GpuChoice::CpuOnly).expect("the second engine starts");
        assert_eq!(held(&folder), 2, "each engine's app holds the folder on its engine thread");
        closed.close().expect("the first engine closes");
        assert_eq!(held(&folder), 1, "closing an engine dropped its app, and its entry alone, on its thread");
        let loaded = dropped.request("exec", json!({"command": "effect.plugins.load", "params": {"path": module}})).expect("the engine still alive loads");
        assert_eq!(loaded.pointer("/loaded/0/name"), Some(&json!("Gate Probe")), "{loaded}");
        drop(dropped);
        assert_eq!(held(&folder), 0, "a dropped engine handle closes, and its app takes the folder out");
    }

    /// A plug-in folder that sits in the bundle by name but is a link to a folder outside it is
    /// not a bundled folder: it admits nothing, under either spelling of a module in it (PD13).
    #[test]
    fn ios_loads_nothing_through_a_plug_in_folder_that_links_out_of_the_bundle() {
        let scratch = Scratch::new(true, Some(Path::new("container/plug-ins")));
        let outside = module(&scratch.root().join("container").join("plug-ins").join("probe.wasm"), "io.github.christopherlinnett.craftbridge.ioslinked");
        let mut linked = scratch.folders.clone();
        let door = scratch.root().join("App.app").join("PlugIns");
        std::os::unix::fs::symlink(scratch.root().join("container").join("plug-ins"), &door).expect("a link out of the bundle");
        linked.plugins = Some(door.clone());
        let mut app = EffectCraftApp::for_platform(Platform::Ios, &linked).expect("the app starts");
        refused_by_the_gate(&mut app, &outside);
        refused_by_the_gate(&mut app, &door.join("probe.wasm").to_string_lossy());
    }

    /// T24 on Android and the desktop: the folder the app named loads through EffectCraft's own
    /// loader, and a module outside every root the confinement derives is refused by the
    /// confinement, as `Io` (§19's row for an access outside it) and in its own words.
    #[test]
    fn android_and_the_desktop_load_what_the_confinement_lets_through() {
        for (platform, id) in [
            (Platform::Android, "io.github.christopherlinnett.craftbridge.androidprobe"),
            (Platform::Host, "io.github.christopherlinnett.craftbridge.hostprobe"),
        ] {
            let scratch = Scratch::new(false, Some(Path::new("container/plug-ins")));
            let named = module(&scratch.root().join("container").join("plug-ins").join("probe.wasm"), id);
            let outside = module(&scratch.root().join("outside").join("probe.wasm"), id);
            let mut app = scratch.app(platform);
            match load(&mut app, &outside) {
                Err(CraftError::Io { message }) => {
                    assert!(message.starts_with(&format!("cannot read `{outside}`: it is outside the automation roots")), "{platform:?}: {message}")
                }
                other => panic!("{platform:?} must refuse a module outside the confinement as `Io`, got {other:?}"),
            }
            let loaded = load(&mut app, &named).unwrap_or_else(|error| panic!("{platform:?} must load the folder its app named: {error}"));
            assert_eq!(loaded.pointer("/loaded/0/name"), Some(&json!("Gate Probe")), "{platform:?} loads its folder: {loaded}");
        }
    }

    /// The switch through the door, on every platform: with no plug-in folder named, a load
    /// answers the engine's own no-loader refusal (`crates/engine/src/cmd/effect.rs`:39) on the
    /// `Unsupported` row, even for a module inside the container.
    #[test]
    fn no_folder_loads_nothing_on_any_platform() {
        for platform in [Platform::Ios, Platform::Android, Platform::Host] {
            let scratch = Scratch::new(true, None);
            let module = module(&scratch.root().join("container").join("probe.wasm"), "io.github.christopherlinnett.craftbridge.offprobe");
            match load(&mut scratch.app(platform), &module) {
                Err(CraftError::Unsupported { message }) => assert_eq!(message, "WebAssembly effect plug-ins are not available in this build", "{platform:?}"),
                other => panic!("{platform:?} with plug-ins off must answer `Unsupported`, got {other:?}"),
            }
        }
    }
}
