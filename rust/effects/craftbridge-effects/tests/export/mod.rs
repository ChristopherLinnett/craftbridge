//! The acceptance of plan §28 T9 to T16 and T22 as a property of the plugin: an `export` writes what
//! the command line's `render` writes on the same inputs, and a cancelled export leaves no output.
//!
//! D-12 asks that an encoded export match the command line's in frame count, size and, frame by
//! frame, within 40 dB. These tests hold every output to the stronger relation that implies: the
//! same files with the same bytes. The command line and the plugin run the same encoders on the
//! same machine, and no encoder writes a time, a host name or a random id; what a file's bytes do
//! depend on is the size of the thread pool the export runs on (the WebM muxer's clusters), which
//! `oracle_in` hands the command line as the plugin's own. A build whose encoder differed between
//! runs would fail here, and the corpus's D-12 comparison (the conformance runner) is where such a
//! difference is measured.

mod kill;

use super::common::Scratch;
use super::{cli, cli_run, scrub, scrubbed, steps_document};
use craftbridge_core::{CancelToken, CraftError, Engine, GpuChoice};
use craftbridge_effects::RenderRequest;
use effectcraft_automation::confine::{Access, Confinement};
use effectcraft_engine::project::render_queue::OutputFormat;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The refusal of a render request that sets both `queue` and `hardware`, for an `export` and for a
/// template's `output` alike.
pub(super) const QUEUED_HARDWARE: &str = "`export`: `queue` cannot be set with `hardware`: a queued render encodes through the render queue and its own encoders, never the device's hardware encoder";

/// One flag of the command line's `render` and the request field that stands for it.
type Flag = (&'static str, &'static str, Value);

/// A short, small slice of the demo project: half a second at a quarter of its size.
fn small() -> Vec<Flag> {
    vec![("--end", "end", json!(0.5)), ("--resolution", "resolution", json!("quarter"))]
}

/// The command line's arguments for these flags: a flag with a text or number value, or a bare
/// flag for `true`.
fn cli_flags(flags: &[Flag]) -> Vec<String> {
    let mut args = Vec::new();
    for (flag, _, value) in flags {
        args.push((*flag).to_string());
        match value {
            Value::Bool(_) => {}
            Value::String(text) => args.push(text.clone()),
            other => args.push(other.to_string()),
        }
    }
    args
}

fn request_of(flags: &[Flag], output: &Path) -> Value {
    let mut request = serde_json::Map::new();
    for (_, key, value) in flags {
        request.insert((*key).to_string(), value.clone());
    }
    request.insert("output".to_string(), json!(output.display().to_string()));
    Value::Object(request)
}

/// Every file in `dir`, by name; the folders beside them (the engine's own working folders) are not output.
fn files_in(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        files.insert(name, std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?);
    }
    Ok(files)
}

/// Whether two output folders hold the same files with the same bytes, naming what differs.
pub(super) fn same_files(plugin: &Path, command_line: &Path) -> Result<usize, String> {
    let (ours, theirs) = (files_in(plugin)?, files_in(command_line)?);
    if ours.keys().ne(theirs.keys()) {
        return Err(format!("the plugin wrote {:?}, the command line {:?}", ours.keys().collect::<Vec<_>>(), theirs.keys().collect::<Vec<_>>()));
    }
    if ours.is_empty() {
        return Err("neither wrote anything".to_string());
    }
    for (name, bytes) in &ours {
        if theirs.get(name) != Some(bytes) {
            return Err(format!("{name} differs from the command line's: {} bytes against {}", bytes.len(), theirs.get(name).map_or(0, Vec::len)));
        }
    }
    Ok(ours.len())
}

/// `request` with `hardware` set.
fn with_hardware(mut request: Value) -> Value {
    if let Value::Object(fields) = &mut request {
        fields.insert("hardware".to_string(), Value::Bool(true));
    }
    request
}

/// The engine an app crate's constructor makes: started in the app's folders, so it keeps the journal
/// that a cancelled export's files are deleted by.
fn start_engine(scratch: &Scratch) -> Result<Engine, String> {
    let engine = Engine::start_in(&scratch.folders, scratch.app().map_err(|e| e.to_string())?, GpuChoice::CpuOnly).map_err(|e| e.to_string())?;
    engine.request("exec", json!({"command": "file.openDemoProject"})).map_err(|e| format!("the demo project opens: {e}"))?;
    Ok(engine)
}

/// What the plugin's `export` answered and heard as progress, and the files of the watched folder
/// when it was asked to stop.
struct Exported {
    answer: Result<Value, CraftError>,
    reports: Vec<(u64, u64)>,
    at_cancel: Vec<String>,
}

/// The names of the files in `dir`, or none when it cannot be read.
fn names_in(dir: &Path) -> Vec<String> {
    let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
    entries.filter(|entry| entry.path().is_file()).map(|entry| entry.file_name().to_string_lossy().into_owned()).collect()
}

/// Run `export`; with `stop_at`, cancel it once that percent of its frames are done, noting the files
/// in the folder at that moment, which the export had written and the stop must remove.
fn export(engine: &Engine, request: Value, cancel: &CancelToken, stop_at: Option<(&Path, u64)>) -> Exported {
    let reports = Arc::new(Mutex::new(Vec::new()));
    let at_cancel = Arc::new(Mutex::new(Vec::new()));
    let (heard, seen) = (reports.clone(), at_cancel.clone());
    let token = cancel.clone();
    let watched = stop_at.map(|(dir, percent)| (dir.to_path_buf(), percent));
    let answer = engine.request_with_progress(
        "export",
        request,
        move |done, total, _| {
            if let Ok(mut heard) = heard.lock() {
                heard.push((done, total));
            }
            if let Some((dir, percent)) = &watched
                && total > 0
                && done * 100 >= total * percent
                && !token.is_cancelled()
            {
                if let Ok(mut seen) = seen.lock() {
                    *seen = names_in(dir);
                }
                token.cancel();
            }
        },
        cancel,
    );
    let reports = reports.lock().map(|r| r.clone()).unwrap_or_default();
    let at_cancel = at_cancel.lock().map(|r| r.clone()).unwrap_or_default();
    Exported { answer, reports, at_cancel }
}

/// One export on both sides, from the demo project, with the same flags: the files and the answer's
/// comp and output must match.
fn parity(flags: &[Flag], file: &str) -> Result<usize, String> {
    let scratch = Scratch::new().map_err(|e| e.to_string())?;
    let (cli_out, plugin_out) = (scratch.root().join("cli-out"), scratch.path().join("plugin-out"));
    for dir in [&cli_out, &plugin_out] {
        std::fs::create_dir(dir).map_err(|e| e.to_string())?;
    }
    let out = cli_out.join(file).display().to_string();
    let mut args: Vec<String> = ["render", "--json", "--out", out.as_str()].map(String::from).to_vec();
    args.extend(cli_flags(flags));
    let run = cli(scratch.root(), &args.iter().map(String::as_str).collect::<Vec<_>>())?;
    let mut command_line = run.doc()?;

    let engine = start_engine(&scratch)?;
    let exported = export(&engine, request_of(flags, &plugin_out.join(file)), &CancelToken::new(), None);
    engine.close().map_err(|e| e.to_string())?;
    let mut plugin = exported.answer.map_err(|e| format!("the plugin's export failed: {e}"))?;

    let seconds = |doc: &mut Value| doc.pointer_mut("/rendered/0").and_then(Value::as_object_mut).map(|item| item.remove("seconds"));
    seconds(&mut command_line);
    seconds(&mut plugin);
    scrub(&mut command_line, &[&cli_out]);
    scrub(&mut plugin, &[&plugin_out]);
    if plugin != command_line {
        return Err(format!("the answers differ: {plugin} against {command_line}"));
    }
    same_files(&plugin_out, &cli_out)
}

#[test]
fn every_format_exports_what_the_command_line_exports() {
    let with = |extra: Vec<Flag>| {
        let mut flags = small();
        flags.extend(extra);
        flags
    };
    let format = |name: &'static str| -> Flag { ("--format", "format", json!(name)) };
    let cases: Vec<(&str, Vec<Flag>)> = vec![
        ("h264.mp4", with(vec![format("h264")])),
        (
            "hevc.mp4",
            with(vec![
                format("hevc"),
                ("--profile", "profile", json!("main")),
                ("--keyint", "keyint", json!(12)),
                ("--rate-control", "rateControl", json!("quality")),
                ("--video-quality", "videoQuality", json!(60)),
            ]),
        ),
        ("av1.mp4", with(vec![format("av1")])),
        ("prores.mov", with(vec![format("prores"), ("--prores", "prores", json!("standard"))])),
        ("uncompressed.mov", with(vec![format("uncompressed")])),
        ("vp9.webm", with(vec![format("webm"), ("--bitrate", "bitrate", json!(900))])),
        (
            "webm-av1.webm",
            with(vec![
                format("webm"),
                ("--webm-codec", "webmCodec", json!("av1")),
                ("--audio-bitrate", "audioBitrate", json!(96)),
                ("--opus-app", "opusApp", json!("voice")),
            ]),
        ),
        ("png.png", with(vec![format("png"), ("--channels", "channels", json!("rgba"))])),
        ("jpeg.jpg", with(vec![format("jpeg"), ("--jpeg-quality", "jpegQuality", json!(70))])),
        ("tiff.tif", with(vec![format("tiff")])),
        ("exr.exr", with(vec![format("exr"), ("--exr-compression", "exrCompression", json!("zip")), ("--exr-depth", "exrDepth", json!("half"))])),
        ("tga.tga", with(vec![format("tga")])),
        ("gif.gif", with(vec![format("gif")])),
        ("audio.wav", with(vec![format("wav")])),
        ("audio.aif", with(vec![format("aiff")])),
        // No `--format`: the extension names it, and with no extension the default template's does.
        ("by-extension.gif", small()),
        ("no-extension", small()),
        // `--scale`, the command line's other name for `--resolution`.
        ("scale.mp4", vec![("--end", "end", json!(0.5)), ("--scale", "scale", json!("quarter"))]),
        // The range and rate flags.
        (
            "range.mp4",
            vec![
                ("--start", "start", json!(0.1)),
                ("--end", "end", json!(0.4)),
                ("--fps", "fps", json!(10)),
                ("--resolution", "resolution", json!(0.25)),
                ("--quality", "quality", json!("draft")),
            ],
        ),
        ("comp.mp4", with(vec![("--comp", "comp", json!("EffectCraft Intro")), ("--audio", "audio", json!("off"))])),
        ("work-area.mp4", vec![("--work-area", "workArea", json!(true)), ("--resolution", "resolution", json!("quarter"))]),
    ];
    // A format the pin's render offers and no case exercises is a format never compared: a pin
    // that adds one fails here until it has a case.
    let exercised: BTreeSet<&str> = cases
        .iter()
        .flat_map(|(_, flags)| flags)
        .filter(|(flag, ..)| *flag == "--format")
        .filter_map(|(.., value)| value.as_str())
        .filter_map(OutputFormat::from_name)
        .map(OutputFormat::label)
        .collect();
    let offered: BTreeSet<&str> = OutputFormat::ALL.into_iter().map(OutputFormat::label).collect();
    assert_eq!(exercised, offered, "every export format of the pin has a case");
    let mut failures = Vec::new();
    for (file, flags) in &cases {
        match parity(flags, file) {
            Ok(written) => assert!(written >= 1),
            Err(why) => failures.push(format!("{file}: {why}")),
        }
    }
    assert!(failures.is_empty(), "exports that differ from the command line's: {failures:#?}");
}

/// The queue T16 renders: a small comp and three items of different formats, outputs in `dir`; the
/// second is unqueued, so only the first and the third render.
fn queue_steps(dir: &Path) -> Vec<(String, Value)> {
    let item = |file: &str, format: &str| ("renderQueue.add".to_string(), json!({"comp": 1, "format": format, "output": dir.join(file).display().to_string()}));
    vec![
        ("comp.new".to_string(), json!({"name": "Q", "width": 96, "height": 64, "duration": 0.5, "frameRate": 10})),
        ("layer.newSolid".to_string(), json!({"comp": 1, "name": "Bg"})),
        item("first.mp4", "h264"),
        item("unqueued.png", "png"),
        ("renderQueue.setRender".to_string(), json!({"index": 2, "render": false})),
        item("third.gif", "gif"),
    ]
}

#[test]
fn the_projects_own_queue_exports_as_the_command_lines_queue() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (cli_out, plugin_out) = (scratch.root().join("cli-out"), scratch.path().join("plugin-out"));
    std::fs::create_dir(&cli_out).expect("the command line's folder");
    std::fs::create_dir(&plugin_out).expect("the plugin's folder");
    let mut steps = queue_steps(&cli_out);
    steps.push(("file.saveAs".to_string(), json!({"path": scratch.root().join("q.ecproj").display().to_string()})));
    cli_run(scratch.root(), &steps).expect("the command line ran").doc().expect("the command line saved its queue");
    let command_line = cli(scratch.root(), &["render", "--json", "--project", &scratch.root().join("q.ecproj").display().to_string(), "--queue"])
        .expect("the command line ran")
        .doc()
        .expect("the command line rendered its queue");

    let engine = Engine::start(scratch.app().expect("the app starts"), GpuChoice::CpuOnly).expect("the engine starts");
    engine.request("run", json!({"steps": steps_document(&queue_steps(&plugin_out))})).expect("the plugin's queue is set up");
    let exported = export(&engine, json!({"queue": true}), &CancelToken::new(), None);
    engine.close().expect("the engine closes");
    let plugin = exported.answer.expect("the queue renders");

    let comps =
        |doc: &Value| -> Vec<String> { doc["rendered"].as_array().into_iter().flatten().filter_map(|item| item["comp"].as_str().map(String::from)).collect() };
    assert_eq!(comps(&plugin), comps(&command_line), "the same items rendered");
    assert_eq!(comps(&plugin).len(), 2);
    let outputs = |doc: &Value, dir: &Path| -> Vec<Value> {
        doc["rendered"].as_array().into_iter().flatten().map(|item| scrubbed(item["output"].as_str().unwrap_or(""), &[dir])).collect()
    };
    assert_eq!(outputs(&plugin, &plugin_out), outputs(&command_line, &cli_out));
    assert_eq!(outputs(&plugin, &plugin_out), [json!("<DIR>/first.mp4"), json!("<DIR>/third.gif")], "the queued items, in the queue's order");
    assert_eq!(same_files(&plugin_out, &cli_out).expect("the queue's files are the command line's"), 2);
}

/// A small composition with a hole cut in its one solid, and the Content-Aware Fill of it that writes
/// into `dir`: the steps both sides run, the fill last.
fn fill_steps(dir: &Path) -> Vec<(String, Value)> {
    vec![
        ("comp.new".to_string(), json!({"name": "F", "width": 48, "height": 32, "duration": 0.4, "frameRate": 10})),
        ("layer.newSolid".to_string(), json!({"comp": 1, "name": "Bg"})),
        ("layer.addMask".to_string(), json!({"rect": [20.0, 10.0, 8.0, 8.0], "mode": "subtract"})),
        ("contentFill.generate".to_string(), json!({"layer": 1, "outputDir": dir.display().to_string(), "method": "edgeBlend"})),
    ]
}

#[test]
fn a_fill_answers_and_writes_what_the_command_lines_fill_does() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (cli_out, plugin_out) = (scratch.root().join("cli-fill"), scratch.path().join("plugin-fill"));
    // The command line waits for the fill, which its `run` otherwise leaves to the background.
    let mut steps = fill_steps(&cli_out);
    if let Some((_, params)) = steps.last_mut() {
        params["wait"] = json!(true);
    }
    let mut command_line = cli_run(scratch.root(), &steps).expect("the command line ran").doc().expect("the command line printed its document");
    let mut command_line = command_line.as_array_mut().and_then(|steps| steps.pop()).expect("the fill's result is the last step's");

    let engine = Engine::start(scratch.app().expect("the app starts"), GpuChoice::CpuOnly).expect("the engine starts");
    let mut plugin_steps = fill_steps(&plugin_out);
    let (_, fill) = plugin_steps.pop().expect("the fill is the last step");
    engine.request("run", json!({"steps": steps_document(&plugin_steps)})).expect("the plugin's project is built");
    let mut plugin = engine.request_with_progress("contentFill.generate", fill, |_, _, _| {}, &CancelToken::new()).expect("the plugin's fill runs");
    engine.close().expect("the engine closes");
    scrub(&mut plugin, &[&plugin_out]);
    scrub(&mut command_line["result"], &[&cli_out]);
    assert_eq!(plugin, command_line["result"], "the fill's answers differ");
    assert_eq!(plugin["frames"], json!(4));
    assert_eq!(same_files(&plugin_out, &cli_out).expect("the fills' files are the command line's"), 4);
}

/// A fill writes only inside the container, whether or not the engine keeps a journal: a folder
/// outside it, one in the bundle the app may only read, one named relative to the process, the one
/// the command picks for a project with no file (in the system's temporary folder), and a frame that
/// is a link out of the container are each refused in the confinement's own words, before anything
/// is listed, started or written.
#[test]
fn a_fill_is_refused_any_folder_or_frame_outside_the_container_before_anything_is_written() {
    let scratch = Scratch::with_bundle().expect("a scratch folder with a bundle");
    let (bundle, outside) = (scratch.bundle().expect("the bundle").to_path_buf(), scratch.outside().to_path_buf());
    let victim = outside.join("victim.png");
    std::fs::write(&victim, b"not the app's").expect("a file outside the container");
    let linked = scratch.path().join("linked");
    std::fs::create_dir(&linked).expect("a folder of an earlier fill");
    std::os::unix::fs::symlink(&victim, linked.join("fill_00000.png")).expect("its first frame made a link out of the container");
    let shown = |path: &Path| path.display().to_string();
    let unsaved = std::env::temp_dir().join("effectcraft-fill").join("Bg_Fill_1");
    let cases = [
        (Some(shown(&outside.join("fill"))), shown(&outside.join("fill"))),
        (Some(shown(&bundle.join("fill"))), shown(&bundle.join("fill"))),
        (Some("fill".to_string()), "fill".to_string()),
        (None, shown(&unsaved)),
        (Some(shown(&linked)), shown(&linked.join("fill_00000.png"))),
    ];
    let write_roots = [scratch.path().to_path_buf()];
    let confinement = Confinement::new(&[], &write_roots).expect("the confinement builds");
    for journal in [true, false] {
        for (folder, refused_at) in &cases {
            let app = scratch.app().expect("the app starts");
            let engine = if journal { Engine::start_in(&scratch.folders, app, GpuChoice::CpuOnly) } else { Engine::start(app, GpuChoice::CpuOnly) }
                .expect("the engine starts");
            let mut steps = fill_steps(scratch.path());
            steps.pop();
            engine.request("run", json!({"steps": steps_document(&steps)})).expect("the project is built");
            let mut fill = json!({"layer": 1});
            if let Some(folder) = folder {
                fill["outputDir"] = json!(folder);
            }
            let refused = engine.request_with_progress("contentFill.generate", fill, |_, _, _| {}, &CancelToken::new());
            let jobs = engine.request("exec", json!({"command": "jobs.list"})).expect("the jobs list");
            engine.close().expect("the engine closes");
            let words = confinement.check(refused_at, Access::Write).expect_err("the confinement refuses it").to_string();
            assert_eq!(refused, Err(CraftError::io(words)), "journal {journal}, folder {folder:?}");
            assert_eq!((&jobs["running"], &jobs["finished"]), (&json!([]), &json!([])), "a fill started: {jobs}");
            assert!(!outside.join("fill").exists() && !bundle.join("fill").exists(), "a folder was made outside the container");
            assert_eq!(std::fs::read(&victim).expect("the file outside reads"), b"not the app's", "a frame was written through the link");
            assert_eq!(std::fs::read_dir(&linked).expect("the folder reads").count(), 1, "a frame was written beside the link");
        }
    }
    assert!(!Path::new("fill").exists(), "a frame was written into the process's own folder");
}

/// Every file in the container, by its path inside it, bar the app's own working folders, which the
/// engine keeps its settings and caches in: what an export could have left behind.
pub(super) fn outputs_in(scratch: &Scratch) -> Result<Vec<PathBuf>, String> {
    contents_in(scratch).map(|(files, _)| files)
}

/// Every file and every folder in the container, each by its path inside it, bar the app's own
/// working folders and what they hold: the files, then the folders.
pub(super) fn contents_in(scratch: &Scratch) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let working = scratch.folders.working().map(|(_, folder)| folder.to_path_buf());
    let (mut files, mut folders) = (Vec::new(), Vec::new());
    for path in entries_under(scratch.path(), &working.iter().collect::<Vec<_>>())? {
        if scratch.path().join(&path).is_dir() {
            folders.push(path);
        } else {
            files.push(path);
        }
    }
    Ok((files, folders))
}

/// Every file and folder under `root` but those in `skip` and below them, by its path inside `root`,
/// sorted.
pub(super) fn entries_under(root: &Path, skip: &[&PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).map_err(|e| format!("{}: {e}", folder.display()))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if skip.contains(&&path) {
                continue;
            }
            if path.is_dir() {
                folders.push(path.clone());
            }
            found.push(path.strip_prefix(root).map_err(|e| e.to_string())?.to_path_buf());
        }
    }
    found.sort();
    Ok(found)
}

#[test]
fn a_cancelled_export_answers_cancelled_and_leaves_no_output() {
    let scratch = Scratch::new().expect("a scratch folder");
    let engine = start_engine(&scratch).expect("the engine starts");
    // The whole demo at half its size, so the cancel lands while the exporter is still writing: a cancel
    // lands only on a progress beat, a tenth of a second apart, and EffectCraft, optimised in the tests
    // as in its own workspace, writes a short export's second half between two beats.
    let slow = |file: &str, format: &str| json!({"output": scratch.path().join(file).display().to_string(), "format": format, "end": 10, "resolution": "half"});

    // The same export, left alone, writes: what the cancel leaves out would have been there.
    let finished = export(&engine, slow("control.mp4", "h264"), &CancelToken::new(), None);
    assert!(finished.answer.is_ok(), "{:?}", finished.answer);
    assert!(scratch.path().join("control.mp4").is_file());
    // Progress is reported on the beat, so only an export that outlasts a beat reports any frames: the
    // short exports of the parity cases may end before their first.
    assert!(finished.reports.iter().any(|(_, total)| *total > 0), "an export longer than a beat reports its frames: {:?}", finished.reports);

    // The cancelled exports write into a folder the export makes: the exporter removes the file it
    // opened when it sees the stop, but only the engine removes the folder, so the folder is what
    // shows that the engine deleted what the call listed.
    let made = scratch.path().join("made");
    let before = contents_in(&scratch).expect("the container reads");
    for (file, format) in [("cancelled.mp4", "h264"), ("cancelled_seq.png", "png")] {
        let cancelled = export(&engine, slow(&format!("made/{file}"), format), &CancelToken::new(), Some((&made, 50)));
        assert_eq!(cancelled.answer, Err(CraftError::Cancelled), "{file}");
        let stem = file.split('.').next().unwrap_or(file);
        assert!(cancelled.at_cancel.iter().any(|name| name.starts_with(stem)), "{file} had been written when the cancel came: {:?}", cancelled.at_cancel);
        assert!(cancelled.reports.iter().any(|(done, total)| *done > 0 && *total > 0), "{file} reported progress before the cancel: {:?}", cancelled.reports);
        assert_eq!(contents_in(&scratch).expect("the container reads"), before, "a cancelled export leaves no file and no folder it made ({file})");
    }
    // The session is free again: nothing is left rendering behind the cancel.
    let again = export(&engine, request_of(&small(), &scratch.path().join("after.mp4")), &CancelToken::new(), None);
    assert!(again.answer.is_ok(), "{:?}", again.answer);
    engine.close().expect("the engine closes");
}

/// The files of the acceptance's export: a sequence long enough that an app could not wait out its
/// start, which is what makes the cancel during it worth testing.
const MANY_FILES: u32 = 18_000;

/// How long after its start an export of [`MANY_FILES`] files is cancelled: the wait an app that has
/// just asked for an export gives it before taking it back. The bound on the answer, from that stop,
/// is [`STOPPED_BY`].
const CANCEL_AFTER: std::time::Duration = std::time::Duration::from_secs(1);

/// The bound on the first progress report, from the export's start: the export reports before the
/// journal looks at a single file, so a stop has a beat to land on from the moment the app asked, not
/// once the listing of its thousands of files happens to be finished.
const FIRST_REPORT_BY: std::time::Duration = std::time::Duration::from_millis(300);

/// The bound on the answer, from the cancel: what the stop has to wind down a render and take back
/// what the call had listed, none of which an export stopped in its listing has to do.
const STOPPED_BY: std::time::Duration = std::time::Duration::from_secs(3);

/// The acceptance of the issue: an export of thousands of files, cancelled a second after its start,
/// has already reported — so the cancel had a beat to land on — and leaves no file and no folder
/// behind, here as on an iPhone (`ExportStartTests`), and here it is answered within a bound of the
/// cancel. A host can be quick enough to be past the listing in that second, and to report soon after
/// it even if it reported nothing before, so these are the acceptance's numbers rather than the rules'
/// pins: the beat before every chunk, the cancel taken there, and a listing that costs the folders
/// rather than the files are each held by a test of their own, in the runner and in the journal, which
/// fails on any machine when its rule is broken.
#[test]
fn an_export_of_many_files_cancelled_a_second_in_reports_at_once_and_leaves_nothing() {
    let scratch = Scratch::new().expect("a scratch folder");
    let engine = start_engine(&scratch).expect("the engine starts");
    // 16 by 16 at ten frames a second: the frames are the least an export can be asked to draw, so
    // what the export spends its time on is the files themselves — the listing and the journal.
    let long = json!({"command": "comp.new", "params": {"name": "Many", "width": 16, "height": 16, "frameRate": 10, "duration": MANY_FILES as f64 / 10.0}});
    engine.request("exec", long).expect("the long comp builds");
    let before = contents_in(&scratch).expect("the container reads");
    let out = scratch.path().join("many").join("frame.png");
    let request = json!({"comp": "Many", "output": out.display().to_string(), "format": "png", "resolution": 1.0, "audio": "off"});

    let token = CancelToken::new();
    let cancel = token.clone();
    let started = std::time::Instant::now();
    let (first, stop) = (Arc::new(Mutex::new(None)), Arc::new(Mutex::new(None)));
    let (heard, answered) = (first.clone(), stop.clone());
    let answer = engine.request_with_progress(
        "export",
        request,
        move |_, _, _| {
            let since = started.elapsed();
            if let Ok(mut moment) = heard.lock()
                && moment.is_none()
            {
                *moment = Some(since);
            }
            // The cancel comes once, a second in, on the beat of whatever report is then being heard.
            if since >= CANCEL_AFTER && answered.lock().map(|stop| stop.is_none()).unwrap_or(false) {
                if let Ok(mut moment) = answered.lock() {
                    *moment = Some(since);
                }
                cancel.cancel();
            }
        },
        &token,
    );
    let first = first.lock().expect("the first report's moment").expect("an export reports, or nothing could cancel it");
    assert!(first <= FIRST_REPORT_BY, "the first progress report came {first:?} after the export's start, over the bound of {FIRST_REPORT_BY:?}");
    assert_eq!(answer, Err(CraftError::Cancelled), "a cancel a second into an export of {MANY_FILES} files ends it");
    let stopped_at = stop.lock().expect("the stop's moment").expect("the cancel was asked, so its moment was noted");
    let ended = started.elapsed();
    assert!(ended - stopped_at <= STOPPED_BY, "the export was answered {:?} after the cancel, over the bound of {STOPPED_BY:?}", ended - stopped_at);
    assert_eq!(contents_in(&scratch).expect("the container reads"), before, "an export cancelled in its start leaves no file and no folder behind");
    engine.close().expect("the engine closes");
}

/// A fill stopped at its first report answers `Cancelled`, and the engine takes away every frame and
/// the folders the call made, from what the runner listed: the runner itself takes nothing away for a
/// stop. A fill left alone keeps its frames.
#[test]
fn a_cancelled_fill_leaves_no_frame_and_no_folder_it_made() {
    let scratch = Scratch::new().expect("a scratch folder");
    // Started in the app's folders, as an app crate's constructor does, so the engine keeps the journal.
    let engine = Engine::start_in(&scratch.folders, scratch.app().expect("the app starts"), GpuChoice::CpuOnly).expect("the engine starts");
    // A fill long enough that the first report comes while its task still runs.
    let setup = [
        ("comp.new", json!({"name": "A", "width": 96, "height": 64, "duration": 6, "frameRate": 10})),
        ("layer.newSolid", json!({"comp": 1, "name": "Bg"})),
        ("layer.addMask", json!({"rect": [30.0, 20.0, 20.0, 18.0], "mode": "subtract"})),
    ];
    for (command, params) in setup {
        engine.request("exec", json!({"command": command, "params": params})).expect("the project builds");
    }
    let before = contents_in(&scratch).expect("the container reads");
    let fill = |dir: &str, cancel: &CancelToken, stop: bool| {
        let token = cancel.clone();
        let request = json!({"layer": 1, "outputDir": scratch.path().join(dir).display().to_string()});
        engine.request_with_progress(
            "contentFill.generate",
            request,
            move |_, _, _| {
                if stop {
                    token.cancel()
                }
            },
            cancel,
        )
    };
    let cancelled = fill("deep/er/fill", &CancelToken::new(), true);
    assert_eq!(cancelled, Err(CraftError::Cancelled));
    assert_eq!(contents_in(&scratch).expect("the container reads"), before, "a cancelled fill leaves no frame and no folder");
    let finished = fill("deep/er/fill", &CancelToken::new(), false);
    assert!(finished.is_ok(), "{finished:?}");
    assert_eq!(outputs_in(&scratch).expect("the container reads").len(), 60, "a finished fill keeps its frames");
    engine.close().expect("the engine closes");
}

/// A queued render encodes through the render queue's own encoders, so `hardware` beside `queue` is
/// refused before anything renders: the project and the container are as they were. Each field alone is
/// no such refusal.
#[test]
fn a_queued_export_that_asks_for_hardware_is_refused_before_anything_renders() {
    let scratch = Scratch::new().expect("a scratch folder");
    let engine = start_engine(&scratch).expect("the engine starts");
    // The demo project's own queue is empty, so a render of it would leave nothing to see: a ticked item
    // in it makes a render that ran before the refusal show in the queue and in the container.
    let queued = scratch.path().join("queued.mp4");
    let item = RenderRequest::from_params(&request_of(&small(), &queued))
        .and_then(|request| request.to_queue_params())
        .expect("the item reads")
        .expect("it adds an item");
    engine.request("run", json!({"steps": steps_document(&[("renderQueue.add".to_string(), item)])})).expect("the item is queued");
    let state = || {
        (
            engine.request("exec", json!({"command": "renderQueue.list"})).expect("the queue lists"),
            entries_under(scratch.path(), &[]).expect("the container lists"),
        )
    };
    let before = state();
    let output = scratch.path().join("x.mp4").display().to_string();
    for request in [json!({"queue": true, "hardware": true}), json!({"output": output, "format": "h264", "queue": true, "hardware": true})] {
        let refused = export(&engine, request.clone(), &CancelToken::new(), None);
        assert_eq!(refused.answer.expect_err("refused").to_string(), QUEUED_HARDWARE, "{request}");
        assert!(refused.reports.is_empty(), "nothing rendered for {request}");
        assert_eq!(state(), before, "{request}: the project and the container are as they were");
    }
    for request in [json!({"queue": true}), json!({"queue": true, "hardware": false})] {
        let answer = export(&engine, request.clone(), &CancelToken::new(), None).answer;
        assert_ne!(answer.err().map(|error| error.to_string()).as_deref(), Some(QUEUED_HARDWARE), "{request}: a queue alone is no such refusal");
    }
    assert!(queued.is_file(), "a queue alone renders the project's own queue");
    let small = request_of(&small(), &scratch.path().join("y.mp4"));
    let answer = export(&engine, with_hardware(small), &CancelToken::new(), None).answer;
    assert_ne!(answer.err().map(|error| error.to_string()).as_deref(), Some(QUEUED_HARDWARE), "hardware alone is no such refusal");
    engine.close().expect("the engine closes");
}

#[test]
fn a_request_the_command_line_refuses_fails_in_the_command_lines_words() {
    let scratch = Scratch::new().expect("a scratch folder");
    let out = scratch.root().join("x.mp4").display().to_string();
    let command_line =
        cli(scratch.root(), &["render", "--json", "--out", &out, "--format", "mpeg"]).expect("the command line ran").failure().expect("it refuses");
    let engine = start_engine(&scratch).expect("the engine starts");
    let request = json!({"output": scratch.path().join("x.mp4").display().to_string(), "format": "mpeg"});
    let error = export(&engine, request, &CancelToken::new(), None).answer.expect_err("the format is refused");
    engine.close().expect("the engine closes");
    // The command line's wrapper of an invalid parameter names the command twice; the plan's §14 shape
    // is the plain `{command}: {message}` (AD10), as `run` words it.
    let plain = command_line.replace("invalid parameters for `renderQueue.add`: ", "");
    assert_eq!(error.to_string(), format!("renderQueue.add: {plain}"), "the refusal is not in the command line's words");
    // The row is a command's refusal of a value its own schema passes, as for the same call under `exec`.
    assert!(matches!(&error, CraftError::CommandFailed { command, step: None, .. } if command == "renderQueue.add"), "{error:?}");
    assert!(!scratch.path().join("x.mp4").exists());
}
