//! A render or a Content-Aware Fill the system kills mid-way leaves no partial file, and no folder it
//! made, once the app starts again, and one that finishes keeps its files (plan §16, §19, D-17).
//!
//! The export runs in a second process, which is the test binary itself started on one test of its
//! own, and the parent kills it by its exact process id once the first output file exists. That is
//! the kill the journal exists for: the process never returns from the export, so nothing clears the
//! list or removes a file, and only the next start's recovery can. Stopping the export from inside
//! would take the clean cancel's path, which removes its own files, and prove nothing about the list.

use super::super::common::Scratch;
use super::super::scratch::folders_in;
use super::super::template;
use super::{contents_in, outputs_in};
use craftbridge_core::{CancelToken, Engine, GpuChoice};
use craftbridge_effects::EffectCraftApp;
use serde_json::{Value, json};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The method of the exports that need no template.
const EXPORT: &str = "export";

/// The variable that makes this binary run one export or fill, as the child, instead of the tests: the
/// case it names is the JSON of [`Case`].
const CHILD: &str = "CRAFTBRIDGE_EXPORT_TO_KILL";

/// How long the parent waits for the child's first output file before it gives up.
const PATIENCE: Duration = Duration::from_secs(120);

/// The names of [`cases`], in the order they run.
const CASES: [&str; 6] = ["single", "sequence", "segmented", "queue", "template", "fill"];

const FILL: &str = "contentFill.generate";

/// The frames of the fill case: a comp of this many tenths of a second at ten frames a second.
const FILL_FRAMES: u64 = 60;

/// One export, template render or fill: the method that answers it, whether the demo project is opened
/// first, the commands that set the project up, the request, the starts of the file names that show it
/// has begun writing, any one of them, and the number of files it writes when it is left to finish.
struct Case {
    name: &'static str,
    /// The call with progress the case makes: `export`, `template.render` or `contentFill.generate`.
    method: &'static str,
    /// Whether the project is the demo project; a fill builds its own small one with `setup`.
    demo: bool,
    setup: Vec<(&'static str, Value)>,
    request: Value,
    begun: &'static [&'static str],
    finished: usize,
}

/// The six kinds of render the journal covers, over the demo project cut to a short, small render
/// that still takes the child far longer than the parent needs to see its first file: a single file,
/// an image sequence, a segmented sequence, the whole queue and a template's instance, which sets
/// its own project up; and a Content-Aware Fill of a small composition, whose frames the engine
/// writes one after another as it synthesises them. `dir` is the container.
fn cases(dir: &Path) -> Vec<Case> {
    let at = |name: &str| dir.join(name).display().to_string();
    let quarter = json!({"end": 4, "resolution": "quarter"});
    let with = |extra: Value| -> Value {
        let mut request = quarter.clone();
        request.as_object_mut().into_iter().for_each(|fields| fields.extend(extra.as_object().cloned().unwrap_or_default()));
        request
    };
    vec![
        Case {
            name: "single",
            method: EXPORT,
            demo: true,
            setup: vec![],
            request: with(json!({"output": at("single.mp4"), "format": "h264"})),
            begun: &["single"],
            finished: 1,
        },
        Case {
            name: "sequence",
            method: EXPORT,
            demo: true,
            setup: vec![],
            request: with(json!({"output": at("seq.png"), "format": "png"})),
            begun: &["seq_"],
            finished: 120,
        },
        Case {
            name: "segmented",
            method: EXPORT,
            demo: true,
            setup: vec![
                ("prefs.set", json!({"key": "export.segmentSequences", "value": true})),
                ("prefs.set", json!({"key": "export.segmentSequenceFiles", "value": 10})),
            ],
            request: with(json!({"output": at("part.png"), "format": "png"})),
            begun: &["part_"],
            finished: 120,
        },
        Case {
            name: "queue",
            method: EXPORT,
            demo: true,
            setup: vec![
                ("renderQueue.add", json!({"format": "h264", "output": at("first.mp4"), "end": 1, "resolution": "quarter"})),
                ("renderQueue.add", json!({"format": "png", "output": at("second.png"), "end": 4, "resolution": "quarter"})),
            ],
            request: json!({"queue": true}),
            begun: &["second_"],
            finished: 121,
        },
        Case {
            name: "template",
            method: template::RENDER,
            demo: false,
            setup: vec![],
            request: json!({"template": at(template::FILE), "values": {"Name": "Sam", "Photo": at(template::REPLACEMENT)}, "output": {"output": at("card.png"), "format": "png"}}),
            begun: &["card_"],
            finished: template::FRAMES,
        },
        Case {
            name: "fill",
            method: FILL,
            demo: false,
            setup: vec![
                ("comp.new", json!({"name": "A", "width": 96, "height": 64, "duration": FILL_FRAMES / 10, "frameRate": 10})),
                ("layer.newSolid", json!({"comp": 1, "name": "Bg"})),
                ("layer.addMask", json!({"rect": [30.0, 20.0, 20.0, 18.0], "mode": "subtract"})),
            ],
            request: json!({"layer": 1, "outputDir": at("made/fill")}),
            // The first frame, or the engine's temporary for one should a frame ever be written through
            // the session's own atomic write. Never the first file of any name: the fill's scratch file
            // is made with a name and unlinked at once on macOS, so a kill there lands before any frame.
            begun: &["fill_", ".effectcraft-"],
            finished: FILL_FRAMES as usize,
        },
    ]
}

fn case_json(dir: &Path, case: &Case) -> Value {
    json!({
        "container": dir,
        "method": case.method,
        "demo": case.demo,
        "setup": case.setup.iter().map(|(command, params)| json!([command, params])).collect::<Vec<_>>(),
        "request": case.request,
    })
}

/// Start the app in `container` as an app crate's constructor does, with the journal on.
fn start(container: &Path) -> Result<Engine, String> {
    let folders = folders_in(container, None, Vec::new());
    let app = EffectCraftApp::new(&folders).map_err(|error| error.to_string())?;
    Engine::start_in(&folders, app, GpuChoice::CpuOnly).map_err(|error| error.to_string())
}

/// Set the demo project up as the case says and run its export to the end, answering what it answered.
fn export_of(engine: &Engine, case: &Value) -> Result<Value, String> {
    if case["demo"].as_bool().unwrap_or(true) {
        engine.request("exec", json!({"command": "file.openDemoProject"})).map_err(|error| error.to_string())?;
    }
    for step in case["setup"].as_array().into_iter().flatten() {
        let (command, params) = (step[0].as_str().unwrap_or_default(), step[1].clone());
        engine.request("exec", json!({"command": command, "params": params})).map_err(|error| format!("{command}: {error}"))?;
    }
    engine
        .request_with_progress(case["method"].as_str().unwrap_or(EXPORT), case["request"].clone(), |_, _, _| {}, &CancelToken::new())
        .map_err(|error| error.to_string())
}

/// The child: runs only when the parent names a case, and otherwise passes at once, so a plain run of
/// the tests runs it as a test that does nothing.
#[test]
fn the_export_the_parent_kills() {
    let Ok(case) = std::env::var(CHILD) else { return };
    let case: Value = serde_json::from_str(&case).expect("the parent's case is JSON");
    let container = PathBuf::from(case["container"].as_str().expect("the case names its container"));
    let engine = start(&container).expect("the child's engine starts");
    export_of(&engine, &case).expect("the child's export ran to its end");
    engine.close().expect("the child's engine closes");
}

/// The child, killed by its exact process id and reaped when the test lets go of it still running:
/// a test that fails before its own kill would otherwise leave the export running, and writing a
/// sequence's frames into a container it makes again once the scratch folder has gone.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn spawn_child(case: &Value) -> Result<Running, String> {
    let binary = std::env::current_exe().map_err(|error| format!("the test binary's own path: {error}"))?;
    Command::new(binary)
        .args(["--exact", "export::kill::the_export_the_parent_kills", "--test-threads=1"])
        .env(CHILD, case.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(Running)
        .map_err(|error| format!("the child starts: {error}"))
}

/// The names of the files below the container that are not in `before`: what an export has written.
fn written(scratch: &Scratch, before: &[PathBuf]) -> Result<Vec<String>, String> {
    let found = outputs_in(scratch)?;
    Ok(found.iter().filter(|path| !before.contains(path)).filter_map(|path| path.file_name().map(|name| name.to_string_lossy().into_owned())).collect())
}

#[test]
fn a_killed_render_leaves_no_partial_file_after_a_restart() {
    for name in CASES {
        let scratch = Scratch::new().expect("a scratch folder");
        let case = case_in(&scratch, name).expect("a case of that name");
        let kept = scratch.path().join("kept.txt");
        std::fs::write(&kept, b"not an output").expect("a file the export has no part in");
        let before = outputs_in(&scratch).expect("the container reads");
        let (_, folders_before) = contents_in(&scratch).expect("the container reads");

        // Declared after `scratch`, so a failing test kills the child before the scratch folder goes.
        let mut child = spawn_child(&case_json(scratch.path(), &case)).expect("the child starts");
        let begun = Instant::now();
        while !written(&scratch, &before).expect("the container reads").iter().any(|name| case.begun.iter().any(|start| name.starts_with(start))) {
            assert!(child.0.try_wait().expect("the child can be asked").is_none(), "{}: the child ended before it wrote a file", case.name);
            assert!(begun.elapsed() < PATIENCE, "{}: no file appeared", case.name);
            std::thread::sleep(Duration::from_millis(2));
        }
        // The kill lands on the process this test started, by its id, while the render is still going.
        assert!(child.0.try_wait().expect("the child can be asked").is_none(), "{}: the render ended before the kill", case.name);
        child.0.kill().expect("the child is killed by its id");
        let status = child.0.wait().expect("the child is reaped");
        assert_eq!(status.signal(), Some(9), "{}: the child ended by the kill, not by itself", case.name);
        let partial = written(&scratch, &before).expect("the container reads");
        // A movie is one file from the start, so a kill part-way shows only in the child having been alive.
        let room = if case.finished == 1 { 1 } else { case.finished - 1 };
        assert!(!partial.is_empty() && partial.len() <= room, "{}: killed part-way, {} of {} files: {partial:?}", case.name, partial.len(), case.finished);

        let engine = start(scratch.path()).expect("the app starts again");
        let recovery = engine.recovery().clone();
        engine.close().expect("the engine closes");
        assert_eq!(outputs_in(&scratch).expect("the container reads"), before, "{}: a restart leaves no partial file: {recovery:?}", case.name);
        let (_, folders_after) = contents_in(&scratch).expect("the container reads");
        assert_eq!(folders_after, folders_before, "{}: a restart leaves no folder the export made: {recovery:?}", case.name);
        assert!(kept.is_file(), "{}: a file no export wrote is left alone", case.name);
        assert!(recovery.removed.len() >= partial.len(), "{}: the restart reports each file it deleted: {recovery:?}", case.name);
        assert!(recovery.refused.is_empty() && recovery.failed.is_empty(), "{}: nothing was refused or stuck: {recovery:?}", case.name);
    }
}

#[test]
fn a_finished_render_keeps_its_files_across_a_restart() {
    for name in CASES {
        let scratch = Scratch::new().expect("a scratch folder");
        let case = case_in(&scratch, name).expect("a case of that name");
        let before = outputs_in(&scratch).expect("the container reads");
        let engine = start(scratch.path()).expect("the app starts");
        export_of(&engine, &case_json(scratch.path(), &case)).unwrap_or_else(|error| panic!("{}: {error}", case.name));
        engine.close().expect("the engine closes");
        let finished = outputs_in(&scratch).expect("the container reads");
        assert_eq!(finished.len() - before.len(), case.finished, "{}: the render wrote every file: {finished:?}", case.name);

        let engine = start(scratch.path()).expect("the app starts again");
        let recovery = engine.recovery().clone();
        engine.close().expect("the engine closes");
        assert_eq!(outputs_in(&scratch).expect("the container reads"), finished, "{}: a restart keeps a finished render's files", case.name);
        assert!(recovery.removed.is_empty(), "{}: nothing was deleted: {recovery:?}", case.name);
    }
}

/// The case `name` with its files in `scratch`'s container. The template's is made first, and opened
/// once so the media the engine extracts beside it is there before the files an export writes are
/// counted: the extraction is not the export's.
fn case_in(scratch: &Scratch, name: &str) -> Result<Case, String> {
    if name == "template" {
        template::make(scratch, template::SIZE, template::SECONDS, "Arial")?;
        let engine = start(scratch.path())?;
        engine.request("template.open", json!({"template": scratch.path().join(template::FILE).display().to_string()})).map_err(|error| error.to_string())?;
        engine.close().map_err(|error| error.to_string())?;
    }
    cases(scratch.path()).into_iter().find(|case| case.name == name).ok_or_else(|| format!("no case named {name}"))
}
