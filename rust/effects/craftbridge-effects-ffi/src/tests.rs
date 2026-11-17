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

/// A previous engine of the app was killed mid-write: its temporary files sit in the engine's
/// folders, where no journal could list them, the config folder's beside the journal itself. The
/// engine this call starts takes them back before answering, while the journal's step on the same
/// start path finds nothing of its own to take, and leaves a file of another name and a temporary
/// outside the engine's folders where they were.
#[test]
fn a_start_takes_back_the_temporaries_a_killed_write_left_beside_the_journal_s_files() {
    let scratch = tempfile::tempdir().expect("a scratch container");
    let container = scratch.path();
    let core_folders = craftbridge_core::folders::Folders::from(folders(container));
    core_folders.prepare().expect("the scratch folders prepare");
    // The writer of a killed write has ended: a process started and reaped here, so none has its id.
    let mut ended = std::process::Command::new("true").spawn().expect("a process starts");
    let previous = ended.id();
    ended.wait().expect("the process ends and is reaped");
    let temporary = |folder: &Path, number: u64| folder.join(format!(".effectcraft-{previous}-{number}.tmp"));
    let working = core_folders.working().map(|(_, folder)| folder);
    for (number, folder) in (0_u64..).zip(working) {
        fs::write(temporary(folder, number), b"half a write").expect("a temporary a killed write left in an engine folder");
    }
    let outside = temporary(container, u64::MAX);
    fs::write(&outside, b"a leftover beside the folders").expect("a temporary outside the engine's folders");
    fs::write(core_folders.cache.join("settings.json"), b"a finished file").expect("a file of another name");
    fs::write(core_folders.config.join("last-project.ecproj"), b"a finished project").expect("a finished file config keeps");
    fs::write(core_folders.config.join(craftbridge_core::JOURNAL_FILE), json!({ "outputs": [] }).to_string())
        .expect("a journal with nothing of its own to take");

    let engine = start_effects_engine(folders(container), GpuChoice::CpuOnly).expect("the engine starts");

    let report = engine.recovery().expect("every path is Unicode");
    assert!(report.removed.is_empty() && report.journal.is_empty(), "the journal had nothing of its own to take: {report:?}");
    for folder in working {
        assert!(
            !folder.read_dir().expect("the folder reads").filter_map(Result::ok).any(|entry| entry.file_name().to_string_lossy().starts_with(".effectcraft-")),
            "a temporary of a process that has ended survived the start in {:?}",
            folder.display()
        );
    }
    assert!(outside.exists(), "the start swept the container root, which no engine folder is");
    assert!(
        core_folders.cache.join("settings.json").exists() && core_folders.config.join("last-project.ecproj").exists(),
        "the start deleted a file of another name"
    );

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

/// The constructor hands the build to the core's start rather than building the app itself, so
/// EffectCraft's code runs only once the core has checked the processor. That check comes before
/// the core makes the folders (the core's own tests pin that order), so an app built from folders
/// the start has made was built after it; a constructor that built the app first, and handed the core
/// an app already built, would run EffectCraft's code on a processor below the floor, and on a first
/// launch would build it from folders not made yet.
#[test]
fn the_app_is_built_inside_the_core_s_start_after_its_processor_check() {
    let scratch = tempfile::tempdir().expect("a scratch parent");
    let container = scratch.path().join("first launch");
    let made = std::cell::Cell::new(None);
    let engine = start(folders(&container), GpuChoice::CpuOnly, |folders| {
        made.set(Some(folders.config.is_dir()));
        craftbridge_effects::EffectCraftApp::new(folders)
    })
    .expect("the engine starts");
    pollster::block_on(engine.shut_down()).expect("stopping the engine answers");
    assert_eq!(made.get(), Some(true), "the app was built before the core's start made its folders, so before it checked the processor");
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
