//! [`start_effects_engine`] as an app's first launch meets it, in a scratch container.
//!
//! The acceptance runs the whole exported call: the journal a killed export left and its partial
//! files are written first, the call starts the engine over them, and what it deleted, left and
//! cleared is what the returned handle reports.

use super::*;
use serde_json::json;
use std::fs;
use std::path::Path;

/// The folders an app names on its first launch with its sandbox at `container`: working folders
/// inside it, none created yet — spelled here as Swift and Kotlin hold them, in text.
fn folders(container: &Path) -> Folders {
    let under = |name: &str| container.join(name).display().to_string();
    Folders {
        container: container.display().to_string(),
        config: under("config"),
        cache: under("cache"),
        models: under("models"),
        staged: under("staged"),
        fonts: Vec::new(),
        bundle: None,
        plugins: None,
    }
}

/// The system killed an export mid-write: the journal it left lists what it was writing, the partial
/// files are on disk, and a file the export never touched sits beside them. The engine this call
/// starts deletes the partial files and clears the journal before answering, leaves the other file,
/// and says all of it through `recovery`; and the handle serves EffectCraft's own `info`.
#[test]
fn a_start_after_a_killed_export_deletes_its_partial_files_and_reports_it() {
    let scratch = tempfile::tempdir().expect("a scratch container");
    let container = scratch.path();
    let core_folders = craftbridge_core::folders::Folders::from(folders(container));
    core_folders.prepare().expect("the scratch folders prepare");
    fs::write(container.join("card.mp4"), b"half a movie").expect("the half-written output");
    fs::write(container.join("earlier.mp4"), b"a finished movie").expect("a file no export was writing");
    let journal = core_folders.config.join(craftbridge_core::JOURNAL_FILE);
    fs::write(&journal, json!({ "outputs": [{ "path": "card.mp4" }] }).to_string()).expect("the journal a kill left");

    let engine = start_effects_engine(folders(container), GpuChoice::CpuOnly).expect("the engine starts");

    let report = engine.recovery().expect("every path is Unicode");
    // A start resolves each entry inside the container as it stands now, and reports the paths it
    // resolved: the same file, named through the scratch folder the system symlinks on macOS.
    let resolved = container.canonicalize().expect("the container resolves");
    assert_eq!(report.removed, [resolved.join("card.mp4").display().to_string()], "{report:?}");
    assert!(!container.join("card.mp4").exists(), "a partial file survived the start");
    assert!(container.join("earlier.mp4").exists(), "a file the journal never listed was deleted");
    assert!(!journal.exists(), "the journal was not cleared");
    assert!(report.untouched.is_empty() && report.failed.is_empty(), "{report:?}");
    assert!(report.refused.is_empty() && report.journal.is_empty(), "{report:?}");

    // The handle is the EffectCraft engine, not a bare core one: `info` answers from the plugin's
    // method table, with the engine version the plugin reports.
    let answer = pollster::block_on(engine.request("info".to_string(), "{}".to_string())).expect("info answers");
    let version =
        serde_json::from_str::<serde_json::Value>(&answer).expect("the answer is JSON").get("version").and_then(serde_json::Value::as_str).map(str::to_owned);
    assert!(version.is_some_and(|version| !version.is_empty()), "info answered no version: {answer}");

    pollster::block_on(engine.shut_down()).expect("stopping the engine answers");
}

/// A container that is not there yet is made, not refused: an app names its folders on its very
/// first launch, before anything of the engine's exists.
#[test]
fn a_first_launch_starts_in_folders_that_are_not_there_yet() {
    let scratch = tempfile::tempdir().expect("a scratch parent");
    let container = scratch.path().join("Data/Library/Application Support");
    let engine = start_effects_engine(folders(&container), GpuChoice::CpuOnly).expect("the engine starts in folders it had to make");
    let report = engine.recovery().expect("every path is Unicode");
    assert!(report.removed.is_empty() && report.journal.is_empty(), "nothing was to deal with: {report:?}");
    assert!(container.join("config").is_dir(), "the config folder the start promises was not made");
}

/// Building the app runs the Craft app's own code on the caller's thread, before the engine thread
/// that guards every call exists: a panic there is the start's `Internal`, the error the call
/// declares, so it reaches the app as a `CraftError` and not as the binding's own panic error.
#[test]
fn a_panic_while_the_app_is_built_is_the_start_s_internal_error() {
    let scratch = tempfile::tempdir().expect("a scratch container");
    let started = start(folders(scratch.path()), GpuChoice::CpuOnly, |_| -> craftbridge_core::Result<craftbridge_effects::EffectCraftApp> {
        panic!("the session would not build")
    });
    assert_eq!(started.err(), Some(CraftError::internal("the session would not build")));
}

/// What `info.gpu` answers through the exported call, for a start with `gpu`.
fn gpu_of_a_start_with(gpu: GpuChoice) -> String {
    let scratch = tempfile::tempdir().expect("a scratch container");
    let engine = start_effects_engine(folders(scratch.path()), gpu).expect("the engine starts");
    let answer = pollster::block_on(engine.request("info".to_string(), "{}".to_string())).expect("info answers");
    pollster::block_on(engine.shut_down()).expect("stopping the engine answers");
    let info = serde_json::from_str::<serde_json::Value>(&answer).expect("the answer is JSON");
    info.get("gpu").and_then(serde_json::Value::as_str).map(str::to_owned).expect("info carries a `gpu` string")
}

/// The app's `Preferred` choice reaches the engine: on a desktop the device attaches and `info.gpu`
/// names it as the core names the device it asked for. A start that ignored the choice would answer
/// the CPU note instead.
#[test]
fn a_preferred_start_attaches_the_machine_s_device() {
    let gpu = craftbridge_core::GpuDevice::request(GpuChoice::Preferred);
    assert!(gpu.parts().is_some(), "a desktop must answer a device for this test to mean anything: {}", gpu.summary());
    assert_eq!(gpu_of_a_start_with(GpuChoice::Preferred), gpu.summary());
}

/// The app's `CpuOnly` choice reaches the engine: nothing attaches and `info.gpu` is the reason the
/// core gives for the CPU.
#[test]
fn a_cpu_only_start_answers_the_cpu_note() {
    let cpu = craftbridge_core::GpuDevice::request(GpuChoice::CpuOnly);
    assert_eq!(gpu_of_a_start_with(GpuChoice::CpuOnly), cpu.summary());
}
