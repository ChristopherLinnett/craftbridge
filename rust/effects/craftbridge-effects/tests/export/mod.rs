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
/// just asked for an export gives it before taking it back. It is the acceptance's own behaviour, not
/// a bound the code is held to: [`REPORT_PAUSE`] holds the listing's beats open past it however quick
/// the host is, so the cancel falls in the export's start on any machine.
const CANCEL_AFTER: std::time::Duration = std::time::Duration::from_secs(1);

/// The pause every report of this export stands still for, through the seam this test owns: the export
/// is mid-report until its listener has returned — the engine reads the cancel, and the listing moves
/// on to the next chunk, only after that — so the pause slows the export's own steps the way a loaded
/// runner slows them, this export alone and without loading the machine. It is the app's own wait,
/// [`CANCEL_AFTER`], so the listing's second beat comes at least that long after its first, however
/// quick the host: the cancel is asked on the first beat or the second, never a later one. The
/// acceptance's files make three of the journal's chunks, a beat before each, so a chunk is still to
/// be listed when the cancel lands, on any machine, and a listing that went on past the cancel would
/// report again. What the test reads is what the export did, which no such slowness moves.
const REPORT_PAUSE: std::time::Duration = CANCEL_AFTER;

/// The one bound that stays: how long the wait for the export's answer may take, from its start, before
/// the wait itself is failed by name. An export stopped in its listing answers within seconds on any
/// runner, and so does every fault this test exists for — one that reports on despite a cancel, or
/// finishes despite it; none of them takes the ten minutes this allows, even slowed many times over
/// through [`REPORT_PAUSE`]. What passes this is an export that would sit the whole suite out in
/// silence, and this turns that into a failing test that names the wait. A ceiling on hangs, never a
/// bound on speed, and set far above any loaded runner's time.
const ANSWER_WITHIN: std::time::Duration = std::time::Duration::from_secs(600);

/// What the acceptance's export heard: every report it made, and the one that asked the cancel.
#[derive(Default)]
struct StartOfMany {
    /// Every report in the order it came: `(done, total)`, as the export counted itself.
    reports: Vec<(u64, u64)>,
    /// The report that asked the cancel, or none until a report a second into the export asks it.
    /// The engine reads the token as each report returns, so a cancel asked inside a report is read by
    /// that very report: none may follow [`Asked::read_by`].
    asked: Option<Asked>,
}

/// The report that asked the cancel: how many frames it counted the export as having to draw — the
/// listing's reports count none, and the render's count its item's frames once its first is drawn —
/// and how many reports the export could have made by the one that reads the cancel.
struct Asked {
    total: u64,
    read_by: usize,
}

/// The acceptance of the issue: an export of thousands of files, cancelled a second after its start,
/// has already reported — so the cancel had a beat to land on — and stops there, leaving no file and no
/// folder behind, here as on an iPhone (`ExportStartTests`). Nothing is bound to how fast the machine
/// was: a loaded runner spends wall-clock seconds however it likes, and a test bound on them fails on
/// the runner, not on the code. What is held is what the export did — that it spoke before its render
/// counted a frame, that the cancel was asked in a report that counted no frames to draw, as the
/// listing's do, that no report followed the one that read the cancel, and that the container is
/// exactly what it was — and what timing the test keeps belongs to [`REPORT_PAUSE`], the seam this test
/// owns, while [`ANSWER_WITHIN`] bounds only the wait that must end. A render's first reports, before
/// its first frame is drawn, count none either, so the reports alone cannot tell a listing that kept
/// its beat from one that was silent and slow: the beat before every chunk, the cancel taken there, and
/// a listing that costs the folders rather than the files are each held by a test of their own, in the
/// runner and in the journal, which fail on any machine when their rule is broken.
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
    let nudge = token.clone();
    let started = std::time::Instant::now();
    let heard = Arc::new(Mutex::new(StartOfMany::default()));
    let into_export = heard.clone();
    let (answers, answer_wait) = std::sync::mpsc::channel();
    // The export's answer is waited on with a bound, because a wait that must end must end by name and
    // not by the suite being stopped from outside. The test may hold nothing whose end waits for the
    // export's: a thread is joined only once it has returned, and an engine is closed — dropped too, as
    // a failing test drops it — only once its export has ended. So the engine goes, with the call, to a
    // thread the test never joins, and comes back with the answer: an export that never answers keeps
    // that thread and that engine, and the test still ends, naming the wait.
    std::thread::spawn(move || {
        let answer = engine.request_with_progress(
            "export",
            request,
            move |done, total, _| {
                let asked_now = {
                    let mut into_export = into_export.lock().expect("the hearing is open");
                    into_export.reports.push((done, total));
                    if into_export.asked.is_none() && started.elapsed() >= CANCEL_AFTER {
                        into_export.asked = Some(Asked { total, read_by: into_export.reports.len() });
                        true
                    } else {
                        false
                    }
                };
                // The cancel comes from a report, as an app's does from its progress bar: the engine
                // reads the token as this report returns, so this report is the one that reads the
                // cancel, and no report may follow it.
                if asked_now {
                    cancel.cancel();
                }
                // The seam: the export stands here, mid-report, as a loaded runner stands still
                // everywhere at once.
                std::thread::sleep(REPORT_PAUSE);
            },
            &token,
        );
        let _ = answers.send((answer, engine));
    });
    let (answer, engine) = match answer_wait.recv_timeout(ANSWER_WITHIN) {
        Ok(answered) => answered,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            // Ask the stop on the way out: an export that still reads its token takes it, and its
            // thread then closes the engine rather than leave it running into a removed container.
            nudge.cancel();
            panic!(
                "the export of {MANY_FILES} files did not answer within {ANSWER_WITHIN:?} of its start: a wait that must end did not, so the export never stopped"
            )
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            panic!("the export of {MANY_FILES} files was never answered: the thread that asked for it ended without its answer")
        }
    };

    let heard = heard.lock().expect("the hearing is open");
    let mut missed = Vec::new();
    // The export spoke before its render counted a frame. A render's own first reports, before its
    // first frame is drawn, count nothing too, so this alone is no proof of speaking from the start:
    // the report that asked the cancel, below, catches a start that let the render count frames
    // before any cancel could land.
    match heard.reports.first() {
        None => missed.push(
            "the export reported nothing at all before it answered: an export that reports nothing cannot be cancelled until its first frame".to_string(),
        ),
        Some(&(done, total)) if done != 0 || total != 0 => {
            missed.push(format!("the export's first report came from the render, {done} of {total} frames counted: it did not report at once"))
        }
        Some(_) => {}
    }
    if answer != Err(CraftError::Cancelled) {
        missed.push(format!("a cancel a second into an export of {MANY_FILES} files ends it: the export answered {answer:?}"));
    }
    match &heard.asked {
        None => missed.push(format!("no report came {CANCEL_AFTER:?} after the export's start, so nothing asked the cancel while the export ran")),
        Some(asked) => {
            if asked.total != 0 {
                missed.push(format!(
                    "the cancel was asked in a report of the render, counting {} frames to draw: the export's start could not be reached by a cancel then",
                    asked.total
                ));
            }
            if heard.reports.len() > asked.read_by {
                missed.push(format!(
                    "the export made {} reports after the one that read its cancel: the cancel did not stop its work",
                    heard.reports.len() - asked.read_by
                ));
            }
        }
    }
    drop(heard);
    match contents_in(&scratch) {
        Ok(after) if after != before => missed.push(format!(
            "an export cancelled in its start leaves no file and no folder behind: the container now holds {} files and {} folders where it held {} and {}",
            after.0.len(),
            after.1.len(),
            before.0.len(),
            before.1.len()
        )),
        Err(why) => missed.push(format!("the container could not be read back: {why}")),
        _ => {}
    }
    assert!(missed.is_empty(), "the export of {MANY_FILES} files cancelled {CANCEL_AFTER:?} in: {missed:#?}");
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

/// The render queue reports a finished item at `file`: what a caller of a failed or cancelled queue learns of the
/// file that stayed.
fn assert_queue_reports_done_at(engine: &Engine, file: &Path) {
    let listed = engine.request("exec", json!({"command": "renderQueue.list", "params": {}}));
    let items = listed.as_ref().ok().and_then(|listed| listed.get("items")).and_then(Value::as_array);
    let done = items.and_then(|items| items.iter().find(|item| item.get("status") == Some(&json!("Done"))));
    assert_eq!(
        done.and_then(|item| item.get("last_output")),
        Some(&json!(file.display().to_string())),
        "the queue reports no finished item at its file: {listed:?}"
    );
}

/// The frame of a sequence the failing exports below cannot write: a read-only file stands at it, as
/// the exporter's `File::create` then fails there, after the frames before it were written.
const BLOCKED_FRAME: &str = "seq_00060.png";

/// The frame `name` in `folder`, made read-only over an earlier frame there, so a render fails as it
/// reaches it; none when the permissions do not hold (root can write the frame, and no render fails).
#[cfg(unix)]
fn blocked_frame(folder: &Path, name: &str) -> std::io::Result<Option<PathBuf>> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(folder)?;
    let blocked = folder.join(name);
    std::fs::write(&blocked, b"an earlier frame")?;
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o444))?;
    Ok(std::fs::OpenOptions::new().write(true).open(&blocked).is_err().then_some(blocked))
}

/// An export of the demo project that fails part way, after it wrote frames of its own, over EffectCraft's
/// real exporter: it leaves none of them and no folder it made, and the file that was already at the
/// blocked frame stays as it was. An item of its queue that finished before the failing one keeps its
/// movie, which the queue reports as done at that path and a post-render import has made footage of.
/// The caller hears the render's own failure.
#[cfg(unix)]
#[test]
fn an_export_that_fails_part_way_leaves_no_output_but_what_was_already_there() {
    for queued in [false, true] {
        let scratch = Scratch::new().expect("a scratch folder");
        let engine = start_engine(&scratch).expect("the engine starts");
        let folder = scratch.path().join("out");
        let Some(blocked) = blocked_frame(&folder, BLOCKED_FRAME).expect("the blocked frame") else { return };
        let first = folder.join("first.mp4");
        let exec = |command: &str, params: Value| engine.request("exec", json!({"command": command, "params": params})).expect("the command runs");
        let request = if queued {
            exec("renderQueue.add", json!({"format": "h264", "output": first.display().to_string(), "end": 1, "resolution": "quarter"}));
            let index = exec("renderQueue.list", json!({}))["items"].as_array().map_or(0, Vec::len);
            exec("renderQueue.setOutputModule", json!({"index": index, "postRenderAction": "import"}));
            exec("renderQueue.add", json!({"format": "png", "output": folder.join("seq.png").display().to_string(), "end": 10, "resolution": "half"}));
            json!({"queue": true})
        } else {
            json!({"output": folder.join("seq.png").display().to_string(), "format": "png", "end": 10, "resolution": "half"})
        };
        let before = contents_in(&scratch).expect("the container reads");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let heard = seen.clone();
        let watched = folder.clone();
        let answer = engine.request_with_progress(
            "export",
            request,
            move |_, _, _| {
                if let Ok(mut seen) = heard.lock() {
                    seen.extend(names_in(&watched));
                }
            },
            &CancelToken::new(),
        );
        let written: BTreeSet<String> = seen.lock().map(|seen| seen.iter().cloned().collect()).unwrap_or_default();
        let error = answer.expect_err("the render fails at the blocked frame");
        assert!(matches!(error, CraftError::CommandFailed { .. }), "queued {queued}: the render's own failure, unchanged in kind: {error:?}");
        assert!(written.iter().any(|name| name != BLOCKED_FRAME), "queued {queued}: the render had written nothing before it failed: {written:?}");
        let finished: &[&str] = if queued { &["first.mp4"] } else { &[] };
        let left: BTreeSet<String> = names_in(&folder).into_iter().collect();
        let due: BTreeSet<String> = finished.iter().copied().chain([BLOCKED_FRAME]).map(str::to_string).collect();
        assert_eq!(left, due, "queued {queued}: a failed export left a file of its own, or lost a finished item's");
        assert_eq!(std::fs::read(&blocked).expect("the blocked frame reads"), b"an earlier frame", "queued {queued}: the file that was there changed");
        let (mut files, folders) = contents_in(&scratch).expect("the container reads");
        files.retain(|file| !finished.iter().any(|name| file.ends_with(name)));
        assert_eq!((files, folders), before, "queued {queued}: a failed export left a file or a folder");
        if queued {
            assert_queue_reports_done_at(&engine, &first);
            let summary = exec("project.summary", json!({}));
            let footage = summary["items"].as_array().and_then(|items| items.iter().find(|item| item["name"] == "first.mp4")).expect("the post-render import");
            assert_eq!(footage["missing"], false, "the footage the import made of the finished item lost its file");
        }
        // The session serves the next export, and a finished one keeps its file.
        let again = export(&engine, request_of(&small(), &scratch.path().join("after.mp4")), &CancelToken::new(), None);
        assert!(again.answer.is_ok() && scratch.path().join("after.mp4").is_file(), "queued {queued}: {:?}", again.answer);
        engine.close().expect("the engine closes");
    }
}

/// A queue cancelled while its second item renders keeps the movie of the first, which the queue reports as
/// done at that path, and leaves nothing of the second, whose frames and folder go. The caller hears
/// `Cancelled`. A single export cancelled leaves nothing (`a_cancelled_export_answers_cancelled_...`).
#[test]
fn a_cancelled_queue_keeps_the_items_that_finished_and_nothing_else() {
    let scratch = Scratch::new().expect("a scratch folder");
    let engine = start_engine(&scratch).expect("the engine starts");
    let folder = scratch.path().join("out");
    let first = folder.join("first.mp4");
    let exec = |command: &str, params: Value| engine.request("exec", json!({"command": command, "params": params})).expect("the command runs");
    exec("renderQueue.add", json!({"format": "h264", "output": first.display().to_string(), "end": 1, "resolution": "quarter"}));
    exec("renderQueue.add", json!({"format": "png", "output": folder.join("seq.png").display().to_string(), "end": 10, "resolution": "half"}));
    let before = contents_in(&scratch).expect("the container reads");
    let (token, watched) = (CancelToken::new(), folder.clone());
    let asking = token.clone();
    let answer = engine.request_with_progress(
        "export",
        json!({"queue": true}),
        move |_, _, _| {
            if names_in(&watched).iter().any(|name| name.starts_with("seq_")) {
                asking.cancel();
            }
        },
        &token,
    );
    assert_eq!(answer, Err(CraftError::Cancelled));
    assert_eq!(names_in(&folder), ["first.mp4"], "a cancelled queue left a file of the item it stopped, or lost a finished item's");
    let (mut files, folders) = contents_in(&scratch).expect("the container reads");
    files.retain(|file| !file.ends_with("first.mp4"));
    assert_eq!(
        (files, folders.into_iter().filter(|folder| !folder.ends_with("out")).collect::<Vec<_>>()),
        before,
        "a cancelled queue left more than the finished movie"
    );
    assert_queue_reports_done_at(&engine, &first);
    engine.close().expect("the engine closes");
}

/// Two items of one queue that name one file are refused before anything is written, which is what lets a
/// failed or cancelled queue keep a finished item's file: no later item of the job rewrites it.
#[test]
fn two_items_of_a_queue_on_one_output_are_refused_before_a_file_is_written() {
    let scratch = Scratch::new().expect("a scratch folder");
    let engine = start_engine(&scratch).expect("the engine starts");
    let same = scratch.path().join("out").join("same.mp4").display().to_string();
    for _ in 0..2 {
        engine
            .request("exec", json!({"command": "renderQueue.add", "params": {"format": "h264", "output": same, "end": 1, "resolution": "quarter"}}))
            .expect("the item is queued");
    }
    let before = contents_in(&scratch).expect("the container reads");
    let error = engine.request_with_progress("export", json!({"queue": true}), |_, _, _| {}, &CancelToken::new()).expect_err("the queue is refused");
    assert!(error.to_string().contains("Nothing was rendered"), "{error}");
    assert_eq!(contents_in(&scratch).expect("the container reads"), before, "a refused queue wrote a file");
    engine.close().expect("the engine closes");
}

/// The files left in a folder by a queue of a movie that finishes, with the render log `movie_log` asks
/// for, and then a sequence named `sequence` that fails at its blocked frame (`<stem>_00060.png`), after
/// the caller heard the render's own failure and the queue reported the movie done at its path; none when
/// the permissions do not hold.
#[cfg(unix)]
fn left_by_a_movie_then_a_failing_sequence(movie: &str, movie_log: &str, sequence: &str) -> Result<Option<BTreeSet<String>>, String> {
    let scratch = Scratch::new().map_err(|e| e.to_string())?;
    let engine = start_engine(&scratch)?;
    let folder = scratch.path().join("out");
    let stem = sequence.trim_end_matches(".png");
    if blocked_frame(&folder, &format!("{stem}_00060.png")).map_err(|e| e.to_string())?.is_none() {
        return Ok(None);
    }
    let exec = |command: &str, params: Value| engine.request("exec", json!({"command": command, "params": params})).map_err(|e| e.to_string());
    let movie = folder.join(movie);
    exec("renderQueue.add", json!({"format": "h264", "output": movie.display().to_string(), "log": movie_log, "end": 1, "resolution": "quarter"}))?;
    exec("renderQueue.add", json!({"format": "png", "output": folder.join(sequence).display().to_string(), "end": 10, "resolution": "half"}))?;
    let heard = engine.request_with_progress("export", json!({"queue": true}), |_, _, _| {}, &CancelToken::new());
    if !matches!(heard, Err(CraftError::CommandFailed { .. })) {
        return Err(format!("the sequence did not fail as a command: {heard:?}"));
    }
    assert_queue_reports_done_at(&engine, &movie);
    engine.close().map_err(|e| e.to_string())?;
    Ok(Some(names_in(&folder).into_iter().collect()))
}

/// A render log is named from its output's stem, so a movie and a sequence of one stem share one log, which
/// the refusal of two outputs on one file does not read. The sequence's failure writes it, and it is no file
/// of the movie that finished: it goes with the failed item.
#[cfg(unix)]
#[test]
fn a_failed_items_log_goes_though_a_finished_item_shares_its_name() {
    let Some(left) = left_by_a_movie_then_a_failing_sequence("same.mp4", "errorsOnly", "same.png").expect("the queue runs") else { return };
    let due: BTreeSet<String> = ["same.mp4", "same_00060.png"].map(str::to_string).into();
    assert_eq!(left, due, "the failed item's log stayed, or the finished movie went");
}

/// A render log the finished item asked for is its own file, kept with its movie when a later item, which
/// has a log of its own, fails: only the failed item's log goes.
#[cfg(unix)]
#[test]
fn a_finished_items_own_log_stays_with_its_movie() {
    let Some(left) = left_by_a_movie_then_a_failing_sequence("first.mp4", "plusSettings", "second.png").expect("the queue runs") else { return };
    let due: BTreeSet<String> = ["first.mp4", "first_RenderLog.txt", "second_00060.png"].map(str::to_string).into();
    assert_eq!(left, due, "the finished movie's log went, or the failed item's stayed");
}

/// The variable that makes this binary, started by the test below under a limit on the size of a file,
/// run the movie that limit stops.
const MOVIE_CHILD: &str = "CRAFTBRIDGE_MOVIE_THE_DISK_STOPS";

/// The largest file the child may write, in the shell's blocks: far smaller than the movie, and far
/// larger than the journal it has to keep.
const MOVIE_LIMIT_BLOCKS: u32 = 256;

/// What the movie the limit stops writes over when a file is already at its output.
const EARLIER_MOVIE: &[u8] = b"a movie an earlier export finished";

/// The child of the test below: a real H.264 export the file-size limit stops part way, with and
/// without a file at its output. Passes at once unless the parent started it.
#[test]
fn the_movie_the_disk_stops() {
    if std::env::var_os(MOVIE_CHILD).is_none() {
        return;
    }
    for earlier in [false, true] {
        let scratch = Scratch::new().expect("a scratch folder");
        let engine = start_engine(&scratch).expect("the engine starts");
        let output = scratch.path().join("movie.mp4");
        if earlier {
            std::fs::write(&output, EARLIER_MOVIE).expect("the earlier movie");
        }
        let before = contents_in(&scratch).expect("the container reads");
        let request = json!({"output": output.display().to_string(), "format": "h264", "end": 4, "resolution": "half"});
        let answer = engine.request_with_progress("export", request, |_, _, _| {}, &CancelToken::new());
        assert!(matches!(answer, Err(CraftError::CommandFailed { .. })), "earlier {earlier}: the limit did not fail the movie: {answer:?}");
        assert!(!output.exists(), "earlier {earlier}: a movie cut short was left at the output");
        let (files, folders) = contents_in(&scratch).expect("the container reads");
        let without_output = |listed: &[PathBuf]| listed.iter().filter(|file| !file.ends_with("movie.mp4")).cloned().collect::<Vec<_>>();
        assert_eq!((without_output(&files), folders), (without_output(&before.0), before.1), "earlier {earlier}: the failed movie left something else");
        engine.close().expect("the engine closes");
    }
}

/// A single movie whose muxer the disk stops part way leaves no truncated file, whether or not a file
/// stood at its output: the export had begun to write over that one, so it is no finished render.
/// The limit is the shell's, on the child alone, with the signal it raises ignored so the write fails
/// as a full disk fails it.
#[cfg(unix)]
#[test]
fn a_movie_the_disk_stops_part_way_leaves_no_truncated_file() {
    let binary = std::env::current_exe().expect("the test binary's own path");
    let script = format!("trap '' XFSZ; ulimit -f {MOVIE_LIMIT_BLOCKS}; exec \"$0\" --exact export::the_movie_the_disk_stops --test-threads=1");
    let child = std::process::Command::new("/bin/sh").arg("-c").arg(script).arg(binary).env(MOVIE_CHILD, "1").output().expect("the child runs");
    assert!(child.status.success(), "the child's export did not end as it should: {}\n{}", child.status, String::from_utf8_lossy(&child.stdout));
}
