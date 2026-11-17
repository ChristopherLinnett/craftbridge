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
use craftbridge_core::{CancelToken, CraftError, Engine};
use effectcraft_engine::project::render_queue::OutputFormat;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
fn same_files(plugin: &Path, command_line: &Path) -> Result<usize, String> {
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

fn start_engine(scratch: &Scratch) -> Result<Engine, String> {
    let engine = Engine::start(scratch.app().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
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

/// Run `export`; with `stop_at_half`, cancel it once half its frames are done, noting the files in
/// that folder at that moment, which the export had written and the stop must remove.
fn export(engine: &Engine, request: Value, cancel: &CancelToken, stop_at_half: Option<&Path>) -> Exported {
    let reports = Arc::new(Mutex::new(Vec::new()));
    let at_cancel = Arc::new(Mutex::new(Vec::new()));
    let (heard, seen) = (reports.clone(), at_cancel.clone());
    let token = cancel.clone();
    let watched = stop_at_half.map(Path::to_path_buf);
    let answer = engine.request_with_progress(
        "export",
        request,
        move |done, total, _| {
            if let Ok(mut heard) = heard.lock() {
                heard.push((done, total));
            }
            if let Some(dir) = &watched
                && total > 0
                && done * 2 >= total
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
    if !exported.reports.iter().any(|(_, total)| *total > 0) && file.ends_with(".mp4") {
        return Err(format!("the plugin reported no frames: {:?}", exported.reports));
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

    let engine = Engine::start(scratch.app().expect("the app starts")).expect("the engine starts");
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

/// Every file in the container, by its path inside it, bar the app's own working folders, which the
/// engine keeps its settings and caches in: what an export could have left behind.
pub(super) fn outputs_in(scratch: &Scratch) -> Result<Vec<PathBuf>, String> {
    let working = [&scratch.folders.config, &scratch.folders.cache, &scratch.folders.models];
    let mut found = Vec::new();
    let mut folders = vec![scratch.path().to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).map_err(|e| format!("{}: {e}", folder.display()))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if working.contains(&&path) {
                continue;
            }
            if path.is_dir() {
                folders.push(path);
            } else {
                found.push(path.strip_prefix(scratch.path()).map_err(|e| e.to_string())?.to_path_buf());
            }
        }
    }
    found.sort();
    Ok(found)
}

#[test]
fn a_cancelled_export_answers_cancelled_and_leaves_no_output() {
    let scratch = Scratch::new().expect("a scratch folder");
    let engine = start_engine(&scratch).expect("the engine starts");
    // Long enough that the cancel lands while the exporter is still writing: a quarter-size second
    // takes the command line a few hundredths of a second, and the first beat comes after a tenth.
    let slow =
        |file: &str, format: &str| json!({"output": scratch.path().join(file).display().to_string(), "format": format, "end": 4, "resolution": "quarter"});

    // The same export, left alone, writes: what the cancel leaves out would have been there.
    let finished = export(&engine, slow("control.mp4", "h264"), &CancelToken::new(), None);
    assert!(finished.answer.is_ok(), "{:?}", finished.answer);
    assert!(scratch.path().join("control.mp4").is_file());

    let before = outputs_in(&scratch).expect("the container reads");
    for (file, format) in [("cancelled.mp4", "h264"), ("cancelled_seq.png", "png")] {
        let cancelled = export(&engine, slow(file, format), &CancelToken::new(), Some(scratch.path()));
        assert_eq!(cancelled.answer, Err(CraftError::Cancelled), "{file}");
        let stem = file.split('.').next().unwrap_or(file);
        assert!(cancelled.at_cancel.iter().any(|name| name.starts_with(stem)), "{file} had been written when the cancel came: {:?}", cancelled.at_cancel);
        assert!(cancelled.reports.iter().any(|(done, total)| *done > 0 && *total > 0), "{file} reported progress before the cancel: {:?}", cancelled.reports);
        assert_eq!(outputs_in(&scratch).expect("the container reads"), before, "a cancelled export leaves no file anywhere in the container ({file})");
    }
    // The session is free again: nothing is left rendering behind the cancel.
    let again = export(&engine, slow("after.mp4", "h264"), &CancelToken::new(), None);
    assert!(again.answer.is_ok(), "{:?}", again.answer);
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
