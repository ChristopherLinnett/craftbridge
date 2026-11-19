//! The acceptance of plan §28 T1 to T5 as a property of the plugin: on the same inputs each method
//! answers exactly what the EffectCraft command line at the pin answers. Each test
//! drives both — the oracle command line from `tools/oracle.sh` and a fresh plugin app in scratch
//! folders of its own — and compares whole documents, after rewriting each scratch folder to one
//! marker so the folders themselves do not count as a gap; a failure is compared in its words. The
//! command line confines only its `mcp` subcommand, so the comparison that needs the confinement on
//! both sides drives the command line through that door: one line of its confined `mcp`, which
//! answers through the same kind of server the plugin answers through.

mod common;
mod export;
mod imports;
mod scratch;
mod template;

use common::{EVERY_FORMAT, Scratch};
use craftbridge_core::{CancelToken, CraftApp, CraftError, Engine, FrameBytes, GpuChoice, PixelFormat};
use craftbridge_effects::EffectCraftApp;
use serde_json::{Value, json};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// The command line at the pin, asked of `tools/oracle.sh` once per test run, the builder the
/// conformance stage takes its oracle from: it builds the commit the workspace pins and skips the
/// install only while the installed source is that commit, so no comparison runs against a build of
/// another revision or a binary found on the machine. Helpers of a test answer `Result` and leave the
/// unwrapping to the `#[test]` function, as everywhere else.
fn oracle() -> std::result::Result<PathBuf, String> {
    static ORACLE: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    ORACLE
        .get_or_init(|| {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
            // Started as the script it is, so its own `#!` line picks the shell it is written for.
            let built = Command::new(root.join("tools/oracle.sh")).current_dir(&root).output().map_err(|e| format!("`tools/oracle.sh` runs: {e}"))?;
            if !built.status.success() {
                return Err(format!("`tools/oracle.sh` failed: {}", String::from_utf8_lossy(&built.stderr)));
            }
            let path = String::from_utf8(built.stdout).map_err(|e| format!("the oracle prints its path: {e}"))?;
            Ok(PathBuf::from(path.trim()))
        })
        .clone()
}

struct Run {
    stdout: String,
    stderr: String,
}

impl Run {
    /// The document the command line printed on a successful run.
    fn doc(&self) -> std::result::Result<Value, String> {
        serde_json::from_str(self.stdout.trim()).map_err(|e| format!("the command line printed no JSON document: {e}: {}", self.summary()))
    }

    /// The first lines of the run's output, for a message that explains a parse that failed.
    fn summary(&self) -> String {
        let mut text = self.stdout.trim().to_string();
        if text.is_empty() {
            text = self.stderr.trim().to_string();
        }
        text.chars().take(200).collect()
    }

    /// The failure the command line reported, in its own words: with `--json` it prints
    /// `{"error": …}` on standard output.
    fn failure(&self) -> std::result::Result<String, String> {
        self.doc()?.get("error").and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("the command line reported no failure: {}", self.summary()))
    }
}

/// The command line, set to run in `dir`: its own working folder, its own configuration folder,
/// and no inherited environment, as the conformance runner gives the oracle (plan §28), but with
/// the plugin's thread-pool size. The exporter's WebM muxer writes one cluster per render batch,
/// and a batch is one frame per thread of the pool the export runs on, so a file's bytes follow
/// that pool's size: a byte comparison of the two sides holds only when both pools are one size.
/// A desktop's own memory figure holds a frame in flight for every core at the sizes these tests
/// export, so the plugin's exports run on this process's global pool, as the command line's run on its
/// own, and rayon itself is asked that pool's size, which holds whatever set it — either of rayon's
/// variables, or the machine's cores — and the command line is given that size in the one variable it
/// reads first.
fn oracle_in(dir: &Path) -> std::result::Result<Command, String> {
    let mut command = Command::new(oracle()?);
    command
        .current_dir(dir)
        .env_clear()
        .env("EFFECTCRAFT_CONFIG_DIR", dir.join("cli-config"))
        .env("RAYON_NUM_THREADS", rayon::current_num_threads().to_string());
    Ok(command)
}

/// One run of the command line in `dir`.
fn cli(dir: &Path, args: &[&str]) -> std::result::Result<Run, String> {
    let output = oracle_in(dir)?.args(args).stdin(Stdio::null()).output().map_err(|e| format!("the oracle runs: {e}"))?;
    Ok(Run { stdout: String::from_utf8_lossy(&output.stdout).into_owned(), stderr: String::from_utf8_lossy(&output.stderr).into_owned() })
}

/// One confined `mcp` session of the command line: the folder is both of its automation roots, the
/// lines are sent in order, and every reply it answers is returned in order, each the whole JSON-RPC
/// document. This is the command line's own confined door — the plugin reaches its session through
/// the same kind of server. The session is empty, or holds `project`, which the command line opens
/// before it confines itself.
fn cli_mcp_session(dir: &Path, project: Option<&Path>, lines: &[String]) -> std::result::Result<Vec<Value>, String> {
    let dir_text = dir.display().to_string();
    let project_text = project.map(|project| project.display().to_string());
    let session = match &project_text {
        Some(project) => ["--project", project.as_str()].to_vec(),
        None => ["--empty"].to_vec(),
    };
    let mut child = oracle_in(dir)?
        .arg("mcp")
        .args(session)
        .args(["--automation-read-root", &dir_text, "--automation-write-root", &dir_text])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("the mcp run starts: {e}"))?;
    {
        let stdin = child.stdin.as_mut().ok_or("the mcp run takes a line")?;
        for line in lines {
            stdin.write_all(line.as_bytes()).map_err(|e| format!("the line goes in: {e}"))?;
            stdin.write_all(b"\n").map_err(|e| format!("the line ends: {e}"))?;
        }
    }
    let output = child.wait_with_output().map_err(|e| format!("the mcp run answers: {e}"))?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line.trim()).map_err(|e| format!("the mcp run answers JSON-RPC lines: {e}: {line}")))
        .collect()
}

/// One line of the command line's confined `mcp`, answered with the text of its reply to the request
/// with id 1: the words a refusal through that door is compared in.
fn cli_mcp_confined(dir: &Path, project: Option<&Path>, request: Value) -> std::result::Result<Value, String> {
    let replies = cli_mcp_session(dir, project, &[request.to_string()])?;
    let reply = replies.iter().find(|reply| reply.get("id") == Some(&json!(1))).ok_or("the mcp run answers the request")?;
    // The text of the reply's first content block, read with `pointer` so a reply that carries no
    // such block is reported by what is missing, not by a panic the helper could take on an answer
    // no test wrote (the gate's never-crash lints refuse an unchecked index outside a `#[test]`).
    reply.pointer("/result/content/0/text").cloned().ok_or_else(|| {
        let shown: String = reply.to_string().chars().take(200).collect();
        format!("the mcp run answers a text reply; this one carries no `result.content[0].text`: {shown}")
    })
}

/// Rewrite every way these scratch folders can be spelt — as handed over, and as the system spells
/// them after resolving links, which is what the command line prints — to one marker, in place.
fn scrub(doc: &mut Value, dirs: &[&Path]) {
    let mut spellings: Vec<String> = Vec::new();
    for dir in dirs {
        spellings.push(dir.display().to_string());
        if let Ok(real) = dir.canonicalize() {
            spellings.push(real.display().to_string());
        }
    }
    // The longest first, so a nested folder never leaves its parent's spelling half rewritten.
    spellings.sort_by_key(|spelling| std::cmp::Reverse(spelling.len()));
    scrub_strings(doc, &spellings);
}

fn scrub_strings(doc: &mut Value, spellings: &[String]) {
    match doc {
        Value::String(text) => {
            for spelling in spellings {
                *text = text.replace(spelling.as_str(), "<DIR>");
            }
        }
        Value::Array(items) => {
            for item in items {
                scrub_strings(item, spellings);
            }
        }
        Value::Object(entries) => {
            for (_key, value) in entries {
                scrub_strings(value, spellings);
            }
        }
        _ => {}
    }
}

/// A failure's words with the scratch folders rewritten to the marker.
fn scrubbed(text: &str, dirs: &[&Path]) -> Value {
    let mut text = Value::String(text.to_string());
    scrub(&mut text, dirs);
    text
}

fn without(doc: &mut Value, keys: &[&str]) -> Value {
    if let Some(entries) = doc.as_object_mut() {
        for key in keys {
            entries.remove(*key);
        }
    }
    doc.clone()
}

/// Whether a catalogue entry is one of the commands that need effect helpers, which a session built
/// with no helper hooks says it cannot run.
fn is_helper_command(entry: &Value) -> bool {
    entry.get("id").and_then(Value::as_str).is_some_and(|id| id.starts_with("effect.helpers."))
}

/// Compare two catalogues as whole documents, entry for entry and in order. The one difference a
/// phone's session makes is in the helper commands: the command line, which starts helpers, lists
/// them as it can run them, while a session with no helper hooks says it cannot, so they are left
/// out of this comparison on both sides, and `the_catalogue_reports_the_phones_capabilities` pins
/// what the plugin says of them instead. Every other byte of every other entry must match.
fn compare_catalogues(plugin: &Value, command_line: &Value, params: &Value) {
    let others = |doc: &Value| -> Vec<Value> { doc.as_array().into_iter().flatten().filter(|entry| !is_helper_command(entry)).cloned().collect() };
    let (mine, theirs) = (others(plugin), others(command_line));
    assert!(!theirs.is_empty(), "the command line listed no commands for {params}");
    assert_eq!(mine.len(), theirs.len(), "the catalogues list different commands for {params}");
    for (mine, theirs) in mine.iter().zip(&theirs) {
        assert_eq!(mine, theirs, "the catalogue differs for {params}");
    }
}

fn plugin_info(app: &mut EffectCraftApp) -> std::result::Result<Value, String> {
    app.request("info", json!({})).map_err(|e| format!("`info` answers: {e}"))
}

/// Mark the one field of the project that states how this front end was started: the command line's
/// sentence names its `--gpu` start flag, which a session built by an SDK has no way to be given,
/// and this session has no GPU attached. Every other byte of the state must match.
fn mark_accel_note(doc: &mut Value) {
    let target = if doc.pointer("/project/renderer").is_some() { "/project/renderer" } else { "/renderer" };
    if let Some(renderer) = doc.pointer_mut(target)
        && let Some(entries) = renderer.as_object_mut()
    {
        entries.insert("why".to_string(), Value::String("<NOTE>".to_string()));
    }
}

/// The plugin's `info` against the command line's: the plugin names its engine and its pin beside
/// the same engine state, which is what plan §14 asks of it.
fn compare_info(cli_doc: &mut Value, plugin_doc: &mut Value, dirs: &[&Path]) {
    let mut plugin = without(plugin_doc, &["engine", "pin", "gpu", "hardwareDecoding", "decoding", "encoding"]);
    let mut command_line = without(cli_doc, &["mode"]);
    mark_accel_note(&mut plugin);
    mark_accel_note(&mut command_line);
    scrub(&mut plugin, dirs);
    scrub(&mut command_line, dirs);
    assert_eq!(plugin, command_line, "the plugin and the command line answered differently");
}

#[test]
fn info_of_an_empty_session_is_the_commands_info() {
    let scratch = Scratch::new().expect("a scratch folder");
    let cli_run = cli(scratch.root(), &["info", "--json", "--empty"]).expect("the command line ran");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let mut cli_doc = cli_run.doc().expect("the command line printed its document");
    let mut plugin_doc = plugin_info(&mut app).expect("the plugin answers info");
    compare_info(&mut cli_doc, &mut plugin_doc, &[scratch.root(), scratch.path()]);
}

#[test]
fn info_with_the_demo_project_is_the_commands_info() {
    let scratch = Scratch::new().expect("a scratch folder");
    let cli_run = cli(scratch.root(), &["info", "--json"]).expect("the command line ran");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    // The command line opens its demo project unless told not to; the plugin is opened the same
    // command, since the app asks for what it wants.
    app.request("run", json!({"steps": [{"command": "file.openDemoProject"}]})).expect("the demo project opens");
    let mut cli_doc = cli_run.doc().expect("the command line printed its document");
    let mut plugin_doc = plugin_info(&mut app).expect("the plugin answers info");
    compare_info(&mut cli_doc, &mut plugin_doc, &[scratch.root(), scratch.path()]);
}

#[test]
fn the_command_catalogue_is_the_commands_catalogue() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    // Each of the method's parameters beside the flag of the command line's `commands` it stands for.
    for (params, flags) in [
        (json!({}), vec![]),
        (json!({"schemas": true}), vec!["--schemas"]),
        (json!({"filter": "effect.apply"}), vec!["--filter", "effect.apply"]),
        (json!({"enabledOnly": true}), vec!["--enabled"]),
        (json!({"filter": "layer.new", "enabledOnly": true, "schemas": true}), vec!["--filter", "layer.new", "--enabled", "--schemas"]),
    ] {
        let mut args = vec!["commands", "--json"];
        args.extend(flags);
        let command_line = cli(scratch.root(), &args).expect("the command line ran").doc().expect("the command line printed its catalogue");
        let plugin = app.request("commands", params.clone()).expect("`commands` answers");
        compare_catalogues(&plugin, &command_line, &params);
    }
}

#[test]
fn a_command_run_once_is_the_commands_exec() {
    let scratch = Scratch::new().expect("a scratch folder");
    let comp = json!({"name": "Once", "width": 320, "height": 240, "duration": 2, "frameRate": 24});
    let command_line = cli(scratch.root(), &["exec", "--json", "--empty", "comp.new", &comp.to_string()])
        .expect("the command line ran")
        .doc()
        .expect("the command line printed its result");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let plugin = app.request("exec", json!({"command": "comp.new", "params": comp})).expect("`exec` answers");
    assert_eq!(plugin, command_line, "the plugin's `exec` answered differently from the command line's");

    // A failure of `exec` is the command line's failure, word for word.
    let missing = json!({"comp": "nope"});
    let command_line = cli(scratch.root(), &["exec", "--json", "--empty", "comp.info", &missing.to_string()])
        .expect("the command line ran")
        .failure()
        .expect("the command line reported the failure");
    match app.request("exec", json!({"command": "comp.info", "params": missing})).expect_err("the comp is not there") {
        CraftError::NotFound { message } => assert_eq!(message, command_line, "the failure is not in the command line's words"),
        other => panic!("a missing comp is `NotFound`, got {other:?}"),
    }
}

/// The T3 build, as one list of steps both sides run; the save path is filled in per side.
fn t3_steps(save_path: &str) -> Vec<(String, Value)> {
    vec![
        ("comp.new".to_string(), json!({"name": "T3", "width": 640, "height": 360, "duration": 3, "frameRate": 30})),
        ("layer.newSolid".to_string(), json!({"comp": 1, "name": "Bg"})),
        ("layer.newText".to_string(), json!({"comp": 1, "text": "Hello", "name": "Title"})),
        ("effect.apply".to_string(), json!({"comp": 1, "layer": "Title", "effect": "Gaussian Blur"})),
        ("file.save".to_string(), json!({"path": save_path})),
        ("file.open".to_string(), json!({"path": save_path})),
    ]
}

fn run_arguments(steps: &[(String, Value)]) -> Vec<String> {
    let mut args: Vec<String> = vec!["run".to_string(), "--json".to_string(), "--empty".to_string()];
    for (command, params) in steps {
        args.push(command.clone());
        args.push(params.to_string());
    }
    args
}

fn steps_document(steps: &[(String, Value)]) -> Value {
    Value::Array(steps.iter().map(|(command, params)| json!({"command": command, "params": params})).collect())
}

/// The command line's `run` of these steps, in its scratch folder.
fn cli_run(dir: &Path, steps: &[(String, Value)]) -> std::result::Result<Run, String> {
    cli(dir, &run_arguments(steps).iter().map(String::as_str).collect::<Vec<_>>())
}

#[test]
fn a_build_saved_and_reopened_is_the_commands_build() {
    let scratch = Scratch::new().expect("a scratch folder");
    let plugin_steps = t3_steps(&scratch.path().join("t3.ecproj").display().to_string());
    let command_line_steps = t3_steps(&scratch.root().join("t3.ecproj").display().to_string());
    let mut command_line = cli_run(scratch.root(), &command_line_steps).expect("the command line ran").doc().expect("the command line printed its document");

    let mut app = scratch.app().expect("the app starts from prepared folders");
    let mut plugin = app.request("run", json!({"steps": steps_document(&plugin_steps)})).expect("every step of the build succeeds");
    scrub(&mut plugin, &[scratch.root(), scratch.path()]);
    scrub(&mut command_line, &[scratch.root(), scratch.path()]);
    assert_eq!(plugin, command_line, "the results of the build steps differ from the command line's");

    // The project that came back from the save is the same document on both sides too.
    let reopened = cli(scratch.root(), &["info", "--json", "--project", &scratch.root().join("t3.ecproj").display().to_string()])
        .expect("the command line ran")
        .doc()
        .expect("the command line printed its document");
    let plugin_doc = plugin_info(&mut app).expect("the plugin answers info");
    let mut plugin_project = plugin_doc["project"].clone();
    let mut reopened_project = reopened["project"].clone();
    mark_accel_note(&mut plugin_project);
    mark_accel_note(&mut reopened_project);
    scrub(&mut plugin_project, &[scratch.root(), scratch.path()]);
    scrub(&mut reopened_project, &[scratch.root(), scratch.path()]);
    assert_eq!(plugin_project, reopened_project, "the reopened project differs from the command line's");
}

#[test]
fn a_failed_step_answers_in_the_commands_words() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");

    // A step the command refuses: `CommandFailed`, with the step.
    let steps: Vec<(String, Value)> = vec![
        ("comp.new".to_string(), json!({"name": "A"})),
        ("layer.newSolid".to_string(), json!({"comp": 1, "name": "s"})),
        ("effect.apply".to_string(), json!({"comp": 1, "layer": "s", "effect": "Gausian Blur"})),
    ];
    let command_line = cli_run(scratch.root(), &steps).expect("the command line ran").failure().expect("the command line reported the failure");
    let error = app.request("run", json!({"steps": steps_document(&steps)})).expect_err("the mistyped effect name fails the run");
    assert!(matches!(error, CraftError::CommandFailed { .. }), "a failed step is `CommandFailed`, got {error:?}");
    // The command line prefixes the engine's own words with the doubled command name of its
    // "invalid parameters" wrapper; the plan's §14 shape is the plain `{command}: {message}`.
    let spaced = command_line.replace("invalid parameters for `effect.apply`: ", "");
    assert_eq!(error.to_string(), spaced, "the failed step is not in the command line's words");

    // A step whose failure maps onto another row: the row keeps the step in the same words.
    let steps: Vec<(String, Value)> = vec![("comp.new".to_string(), json!({"name": "B"})), ("comp.info".to_string(), json!({"comp": "nope"}))];
    let command_line = cli_run(scratch.root(), &steps).expect("the command line ran").failure().expect("the command line reported the failure");
    let error = app.request("run", json!({"steps": steps_document(&steps)})).expect_err("the comp is not there");
    assert!(matches!(error, CraftError::NotFound { .. }), "a missing comp is `NotFound`, got {error:?}");
    assert_eq!(error.to_string(), command_line, "the failed step is not in the command line's words");
}

/// A render whose items each fail: the queue renders a comp to outputs that are folders, which the
/// exporter cannot open as files, so the failure joins one `comp: reason` per item. The outputs are
/// filled in per side.
fn failing_render_steps(outputs: &[PathBuf]) -> Vec<(String, Value)> {
    let mut steps = vec![
        ("comp.new".to_string(), json!({"name": "R", "width": 64, "height": 64, "duration": 0.1, "frameRate": 10})),
        ("layer.newSolid".to_string(), json!({"comp": 1, "name": "Bg"})),
    ];
    steps.extend(outputs.iter().map(|output| ("renderQueue.add".to_string(), json!({"comp": 1, "output": output.display().to_string()}))));
    steps
}

/// Make the folder a failing render writes to, for one side.
fn folder_in_the_way(output: &Path) -> std::result::Result<(), String> {
    std::fs::create_dir(output).map_err(|e| format!("the folder in the render's way is made: {e}"))
}

#[test]
fn a_render_with_a_failed_item_fails_as_the_command_line_fails_it() {
    let scratch = Scratch::new().expect("a scratch folder");
    let names = ["out.mov", "two.mov"];
    let (cli_outputs, plugin_outputs): (Vec<PathBuf>, Vec<PathBuf>) = names.iter().map(|name| (scratch.root().join(name), scratch.path().join(name))).unzip();
    for output in cli_outputs.iter().chain(&plugin_outputs) {
        folder_in_the_way(output).expect("the render is blocked");
    }

    // The command line renders a saved queue with `exec`; the plugin renders the same queue through
    // `export`, the one way it starts a render, and fails it in the same words.
    let queued = scratch.root().join("queued.ecproj");
    let mut save = failing_render_steps(&cli_outputs);
    save.push(("file.saveAs".to_string(), json!({"path": queued.display().to_string()})));
    cli_run(scratch.root(), &save).expect("the command line ran").doc().expect("the command line saved its queue");
    let command_line = cli(scratch.root(), &["exec", "--json", "--project", &queued.display().to_string(), "renderQueue.render"])
        .expect("the command line ran")
        .failure()
        .expect("the command line reported the failure");
    assert_eq!(command_line.matches("; ").count(), 1, "each item's failure is named, joined once: {command_line}");
    let engine = Engine::start(scratch.app().expect("the app starts from prepared folders"), GpuChoice::CpuOnly).expect("the engine starts");
    engine.request("run", json!({"steps": steps_document(&failing_render_steps(&plugin_outputs))})).expect("the plugin's queue is set up");
    match engine.request_with_progress("export", json!({"queue": true}), |_, _, _| {}, &CancelToken::new()).expect_err("the failed item fails the render") {
        CraftError::CommandFailed { command, message, step } => {
            assert_eq!((command.as_str(), step), ("renderQueue.render", None));
            assert_eq!(
                scrubbed(&message, &[scratch.path()]),
                scrubbed(&command_line, &[scratch.root()]),
                "the failed render is not in the command line's words"
            );
        }
        other => panic!("a failed render item is `CommandFailed`, got {other:?}"),
    }
    engine.close().expect("the engine closes");
}

/// One confined refusal in parity: the same command, with the same parameters, goes to the command
/// line's confined `mcp` and to the plugin's `exec`, and the pair comes back as the command line's
/// words (folders already rewritten to the marker) and the plugin's whole error, which the test
/// matches on for the row it expects.
fn confined_refusal_pair(scratch: &Scratch, command: &str, params: &Value) -> std::result::Result<(Value, CraftError), String> {
    let command_line = cli_mcp_confined(scratch.root(), None, tools_call("execute_command", &json!({"command": command, "params": params})))?;
    let command_line = match command_line {
        Value::String(message) => scrubbed(&message, &[scratch.root()]),
        other => return Err(format!("the confined command line answered {other} where it should have refused")),
    };
    let mut app = scratch.app().map_err(|e| e.to_string())?;
    match app.request("exec", json!({"command": command, "params": params})) {
        Ok(answer) => Err(format!("the confinement should refuse the call; it answered {answer}")),
        Err(error) => Ok((command_line, error)),
    }
}

#[test]
fn the_confinement_answers_in_the_commands_words() {
    let scratch = Scratch::new().expect("a scratch folder");
    let outside = scratch.outside_file().expect("a file outside both roots");
    let (command_line, error) = confined_refusal_pair(&scratch, "file.open", &json!({"path": outside})).expect("both doors refused");
    match error {
        CraftError::Io { message } => {
            assert_eq!(scrubbed(&message, &[scratch.path()]), command_line, "the confinement refusal differs from the command line's")
        }
        other => panic!("a refused read must be `Io`, got {other:?}"),
    }
}

/// Three comps with something to see at every time and a corner left empty, saved with the first
/// one active at half a second in, so the comp, the time, the size and the alpha of a frame each
/// change its pixels:
/// * `Frames`, 64 x 48 at 10 frames a second, a red box moving across a transparent ground;
/// * `Tall`, 30 x 50 at 25 frames a second, a green box moving down it: its longest side is its
///   height, and its frame rate is another;
/// * a 20 x 20 comp named `1`, so the digits `1` read as a name would find another comp than the id
///   they spell.
fn frame_steps() -> Vec<(String, Value)> {
    let step = |command: &str, params: Value| (command.to_string(), params);
    vec![
        step("comp.new", json!({"name": "Frames", "width": 64, "height": 48, "duration": 2, "frameRate": 10})),
        step("layer.newSolid", json!({"comp": "Frames", "name": "Box", "color": "#ff0000", "width": 20, "height": 20})),
        step("prop.addKey", json!({"comp": "Frames", "layer": "Box", "path": "transform/position", "time": 0, "value": [10, 10]})),
        step("prop.addKey", json!({"comp": "Frames", "layer": "Box", "path": "transform/position", "time": 1, "value": [50, 30]})),
        step("comp.new", json!({"name": "Tall", "width": 30, "height": 50, "duration": 2, "frameRate": 25})),
        step("layer.newSolid", json!({"comp": "Tall", "name": "Drop", "color": "#00ff00", "width": 10, "height": 10})),
        step("prop.addKey", json!({"comp": "Tall", "layer": "Drop", "path": "transform/position", "time": 0, "value": [5, 5]})),
        step("prop.addKey", json!({"comp": "Tall", "layer": "Drop", "path": "transform/position", "time": 1, "value": [25, 45]})),
        step("comp.new", json!({"name": "1", "width": 20, "height": 20, "duration": 1, "frameRate": 24})),
        step("comp.open", json!({"comp": "Frames"})),
        step("time.set", json!({"time": 0.5})),
    ]
}

/// The ids the engine gave the fixture's comps, which it numbers among every item it makes.
struct Comps {
    tall: u64,
    named_one: u64,
}

/// The ids of the comps a `run` document made, in order.
fn made_comps(doc: &Value) -> Vec<u64> {
    doc.as_array()
        .into_iter()
        .flatten()
        .filter(|step| step.get("command").and_then(Value::as_str) == Some("comp.new"))
        .filter_map(|step| step.pointer("/result/comp").and_then(Value::as_u64))
        .collect()
}

/// A frame the command line wrote: the PNG file, and the figures it decodes to.
struct CliFrame {
    png: Vec<u8>,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Run the command line's `render-frame` on the saved project with these flags and read the file.
fn cli_frame(dir: &Path, flags: &[&str]) -> std::result::Result<CliFrame, String> {
    let out = dir.join("cli-frame.png");
    let (project, out_text) = (dir.join("frames.ecproj").display().to_string(), out.display().to_string());
    let mut args = vec!["render-frame", "--json", "--project", project.as_str(), "--out", out_text.as_str()];
    args.extend_from_slice(flags);
    cli(dir, &args)?.doc()?;
    let png = std::fs::read(&out).map_err(|e| format!("the frame the command line wrote reads: {e}"))?;
    let image = image::load_from_memory(&png).map_err(|e| format!("the frame decodes: {e}"))?.to_rgba8();
    Ok(CliFrame { width: image.width(), height: image.height(), rgba: image.into_raw(), png })
}

/// The frames of one case in every format the core answers, each held to what the command line wrote.
fn assert_frame_matches(app: &mut EffectCraftApp, request: &Value, theirs: &CliFrame) -> std::result::Result<(), String> {
    let mut frame = |format| app.frame(request.clone(), format).map_err(|e| format!("{request} as {format:?} renders: {e}"));
    let png = frame(PixelFormat::Png)?;
    assert_eq!((png.width, png.height), (theirs.width, theirs.height), "{request}: the PNG's size");
    assert_eq!(png.data, theirs.png, "{request}: the PNG is not the command line's file, byte for byte");
    let straight = frame(PixelFormat::Rgba8Straight)?;
    assert_eq!((straight.width, straight.height), (theirs.width, theirs.height), "{request}: the straight frame's size");
    assert_eq!(straight.data, theirs.rgba, "{request}: the straight pixels are not the command line's");
    let premultiplied = frame(PixelFormat::Bgra8Premultiplied)?;
    let expected = FrameBytes::new(theirs.width, theirs.height, PixelFormat::Bgra8Premultiplied, theirs.rgba.clone()).map_err(|e| format!("{e}"))?;
    assert_eq!(premultiplied, expected, "{request}: the premultiplied pixels are not the command line's, converted");
    Ok(())
}

/// Build the fixture's comps on both sides, the plugin's session and the command line's saved
/// project, and answer the ids they were given, which both sides must agree on.
fn frames_on_both_sides(scratch: &Scratch) -> std::result::Result<(EffectCraftApp, Comps), String> {
    let steps = frame_steps();
    let mut saved = steps.clone();
    saved.push(("file.save".to_string(), json!({"path": scratch.root().join("frames.ecproj").display().to_string()})));
    let theirs = made_comps(&cli_run(scratch.root(), &saved)?.doc()?);
    let mut app = scratch.app().map_err(|e| format!("the app starts: {e}"))?;
    let built = app.request("run", json!({"steps": steps_document(&steps)})).map_err(|e| format!("the plugin builds the comps: {e}"))?;
    let mine = made_comps(&built);
    if mine != theirs {
        return Err(format!("the comps were given other ids: {mine:?} by the plugin, {theirs:?} by the command line"));
    }
    match mine.as_slice() {
        [_, tall, named_one] => Ok((app, Comps { tall: *tall, named_one: *named_one })),
        other => Err(format!("the fixture makes three comps, the run made {other:?}")),
    }
}

/// What the build steps of a document leave behind: a comp with a solid, a text layer named `T` with
/// a blur on it, and a solid named `4`, saved in the folder of each side.
fn document_steps() -> Vec<(String, Value)> {
    vec![
        ("comp.new".to_string(), json!({"name": "A", "width": 640, "height": 360, "duration": 3, "frameRate": 30})),
        ("layer.newSolid".to_string(), json!({"comp": 1, "name": "Bg"})),
        ("layer.newText".to_string(), json!({"comp": 1, "text": "Hello", "name": "T"})),
        ("effect.apply".to_string(), json!({"comp": 1, "layer": "T", "effect": "Gaussian Blur"})),
        // A layer named as another layer's id is written: digits mean the id, as the command line reads them.
        ("layer.newSolid".to_string(), json!({"comp": 1, "name": "4"})),
    ]
}

/// The same document on both sides: the command line's saved as `doc.ecproj` in its folder, and the
/// plugin's built by the same steps and saved the same way in the container.
struct Documents {
    scratch: Scratch,
    app: EffectCraftApp,
}

impl Documents {
    fn new() -> std::result::Result<Self, String> {
        Self::in_scratch(Scratch::new().map_err(|e| e.to_string())?)
    }

    fn in_scratch(scratch: Scratch) -> std::result::Result<Self, String> {
        let mut steps = document_steps();
        let saved = |dir: &Path| ("file.saveAs".to_string(), json!({"path": dir.join("doc.ecproj").display().to_string()}));
        steps.push(saved(scratch.root()));
        cli_run(scratch.root(), &steps)?.doc()?;
        steps.pop();
        steps.push(saved(scratch.path()));
        let mut app = scratch.app().map_err(|e| e.to_string())?;
        app.request("run", json!({"steps": steps_document(&steps)})).map_err(|e| format!("the plugin builds the document: {e}"))?;
        Ok(Self { scratch, app })
    }

    /// Every spelling of both folders, to rewrite to the marker.
    fn dirs(&self) -> [&Path; 2] {
        [self.scratch.root(), self.scratch.path()]
    }

    /// The command line run on its own copy of the document, with `arguments` after the command.
    fn cli(&self, command: &str, arguments: &[&str]) -> std::result::Result<Run, String> {
        let project = self.scratch.root().join("doc.ecproj").display().to_string();
        let mut args = vec![command, "--json", "--project", &project];
        args.extend(arguments);
        cli(self.scratch.root(), &args)
    }

    /// The command line's answer of `tool` called with `arguments` through its confined `mcp` door,
    /// on its own copy of the document, as the document the tool answered.
    fn tool(&self, tool: &str, arguments: &Value) -> std::result::Result<Value, String> {
        match cli_mcp_confined(self.scratch.root(), Some(&self.scratch.root().join("doc.ecproj")), tools_call(tool, arguments))? {
            Value::String(text) => serde_json::from_str(&text).map_err(|e| format!("`{tool}` answered no document: {e}: {text}")),
            other => Err(format!("`{tool}` answered {other}")),
        }
    }

    /// The file each side has saved, as whole documents: the plugin's session is saved in place, as
    /// the command line saved its own copy after each change.
    fn files(&mut self) -> std::result::Result<(Value, Value), String> {
        self.app.request("save", json!({})).map_err(|e| format!("the plugin saves in place: {e}"))?;
        Ok((saved_file(self.scratch.path(), "doc.ecproj", &self.dirs())?, saved_file(self.scratch.root(), "doc.ecproj", &self.dirs())?))
    }
}

/// A failure's words as the plugin words every refusal of a command: the command line's "invalid
/// parameters for `command`:" wrapper, which it puts before an engine refusal, reads as the plain
/// `command:` the plan's §14 shape has. The second is whether the command line wrapped them.
fn plain_words(words: &str) -> (String, bool) {
    match words.strip_prefix("invalid parameters for `").and_then(|rest| rest.split_once("`: ")) {
        Some((command, refusal)) => (format!("{command}: {refusal}"), true),
        None => (words.to_string(), false),
    }
}

/// The plugin's words for a failure: a `CommandFailed` keeps its command apart from its message, so
/// the command goes back before the message only where the command line's words name one.
fn failure_words(error: &CraftError, named: bool) -> String {
    match error {
        CraftError::CommandFailed { command, message, step: None } if named => format!("{command}: {message}"),
        CraftError::CommandFailed { message, step: None, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// One of the command line's own ways to ask the tools and commands the methods go through, and the
/// words it fails in: the same way a person at a terminal, or a script, would get them.
enum Oracle<'a> {
    /// The subcommand of the same name, with its arguments after `--project doc.ecproj`.
    Subcommand(&'a str, Vec<&'a str>),
    /// The confined `mcp` door calling a tool, which is what the command line has for a refusal
    /// its subcommands cannot reach, and for the confinement's own words.
    Tool(&'a str, Value),
    /// The same door with the project at the path open, as a session that opened it has it.
    ToolOn(&'a Path, &'a str, Value),
}

impl Oracle<'_> {
    /// The confined door running one command, as its `execute_command` tool runs it.
    fn command(command: &str, params: Value) -> Self {
        Oracle::Tool("execute_command", json!({"command": command, "params": params}))
    }

    fn failure(&self, documents: &Documents) -> std::result::Result<String, String> {
        let (project, name, arguments) = match self {
            Oracle::Subcommand(command, arguments) => return documents.cli(command, arguments)?.failure(),
            Oracle::Tool(name, arguments) => (None, name, arguments),
            Oracle::ToolOn(project, name, arguments) => (Some(*project), name, arguments),
        };
        match cli_mcp_confined(documents.scratch.root(), project, tools_call(name, arguments))? {
            Value::String(words) => Ok(words),
            other => Err(format!("the confined command line answered {other} where it should have refused")),
        }
    }
}

/// The one line of JSON-RPC that calls `tool` with `arguments`.
fn tools_call(tool: &str, arguments: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool, "arguments": arguments}})
}

/// Every call here fails: the plugin's words equal the command line's, scrubbed of the folders and
/// with the wrapper read as above, and the failure is the row the plan's §14 gives it.
fn assert_fails_as(documents: &mut Documents, cases: Vec<(&str, Value, Oracle, &str)>) -> std::result::Result<(), String> {
    for (method, params, oracle, row) in cases {
        let expected = oracle.failure(documents)?;
        let Err(error) = documents.app.request(method, params.clone()) else {
            return Err(format!("`{method}` {params} was answered where the command line refused it"));
        };
        let kind = format!("{error:?}");
        assert!(kind.starts_with(row), "`{method}` {params} is `{row}`, got {kind}");
        let (expected, named) = plain_words(&expected);
        let (words, expected) = (scrubbed(&failure_words(&error, named), &documents.dirs()), scrubbed(&expected, &documents.dirs()));
        assert_eq!(words, expected, "`{method}` {params} failed in other words than the command line's");
    }
    Ok(())
}

#[test]
fn props_is_the_commands_props() {
    let mut documents = Documents::new().expect("the same document on both sides");
    // Each parameter beside the argument of the command line's `props <comp> <layer>` it stands for.
    for (params, arguments) in [
        (json!({"comp": 1, "layer": "T"}), vec!["1", "T"]),
        (json!({"comp": "A", "layer": 27}), vec!["A", "27"]),
        (json!({"comp": "1", "layer": "#1"}), vec!["1", "#1"]),
        (json!({"comp": 1, "layer": "4"}), vec!["1", "4"]),
        (json!({"comp": "-", "layer": "Bg"}), vec!["-", "Bg"]),
        (json!({"layer": "Bg"}), vec!["-", "Bg"]),
        (json!({"comp": 1, "layer": "T", "flat": true}), vec!["1", "T", "--flat"]),
        (json!({"comp": 1, "layer": "T", "flat": false}), vec!["1", "T"]),
        (json!({"comp": 1, "layer": "T", "time": 1.5}), vec!["1", "T", "--time", "1.5"]),
        (json!({"comp": 1, "layer": "T", "flat": true, "time": 1}), vec!["1", "T", "--flat", "--time", "1"]),
    ] {
        let mut expected = documents.cli("props", &arguments).expect("the command line ran").doc().expect("the command line printed the tree");
        let mut tree = documents.app.request("props", params.clone()).expect("`props` answers");
        scrub(&mut tree, &documents.dirs());
        scrub(&mut expected, &documents.dirs());
        assert!(expected["properties"].is_array() || expected["properties"].is_object(), "the command line printed no tree for {params}");
        assert_eq!(tree, expected, "`props` {params} answered differently from the command line");
    }
    // `depth` against the command line's own door to the same tool: its `props --depth` parses the
    // number as a decimal, which the tool ignores, while its `mcp` hands the tool the whole number.
    for params in [
        json!({"comp": 1, "layer": "T", "depth": 0}),
        json!({"comp": 1, "layer": "T", "depth": 1}),
        json!({"comp": "A", "layer": 27, "flat": true, "time": 1, "depth": 2}),
    ] {
        let mut expected = documents.tool("get_layer", &params).expect("the command line answered the tool");
        let mut tree = documents.app.request("props", params.clone()).expect("`props` answers");
        scrub(&mut tree, &documents.dirs());
        scrub(&mut expected, &documents.dirs());
        assert!(expected.to_string().contains("\"truncated\":true"), "the command line limited no group for {params}");
        assert_eq!(tree, expected, "`props` {params} answered differently from the command line's tool");
    }
}

#[test]
fn a_frame_is_the_command_lines_frame_for_the_same_flags() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (mut app, comps) = frames_on_both_sides(&scratch).expect("the comps are built on both sides");
    let (tall, named_one) = (comps.tall.to_string(), comps.named_one.to_string());
    // Each field of the request beside the flag of `render-frame` it stands for: the active comp at
    // its current time, and another comp at that same time; a time in seconds and a frame number at
    // each comp's own rate; a longest side, whole, fractional, zero, negative and past the comp; a
    // scale that rounds up, one that rounds down, one past the comp, zero, and one of a tall comp; the
    // comp by its name, by its id as a number and as digits (a `+` before them too), by `-` and by
    // nothing; and the alpha.
    let cases: Vec<(Value, Vec<&str>)> = vec![
        (json!({}), vec![]),
        (json!({"time": 0}), vec!["--time", "0"]),
        (json!({"time": 1}), vec!["--time", "1"]),
        (json!({"frame": 7}), vec!["--frame", "7"]),
        (json!({"frame": 3.5}), vec!["--frame", "3.5"]),
        (json!({"comp": "Tall", "frame": 5}), vec!["--comp", "Tall", "--frame", "5"]),
        (json!({"maxSide": 40}), vec!["--max-side", "40"]),
        (json!({"maxSide": 40.7}), vec!["--max-side", "40.7"]),
        (json!({"maxSide": 0}), vec!["--max-side", "0"]),
        (json!({"maxSide": -5}), vec!["--max-side", "-5"]),
        (json!({"maxSide": 100}), vec!["--max-side", "100"]),
        (json!({"scale": 0.34}), vec!["--scale", "0.34"]),
        (json!({"scale": 0.3}), vec!["--scale", "0.3"]),
        (json!({"scale": 2}), vec!["--scale", "2"]),
        (json!({"scale": 0}), vec!["--scale", "0"]),
        (json!({"comp": "Tall", "scale": 0.5}), vec!["--comp", "Tall", "--scale", "0.5"]),
        (json!({"transparent": true}), vec!["--transparent"]),
        (json!({"transparent": false}), vec![]),
        (json!({"comp": "Frames"}), vec!["--comp", "Frames"]),
        (json!({"comp": "Tall"}), vec!["--comp", "Tall"]),
        (json!({"comp": comps.tall}), vec!["--comp", tall.as_str()]),
        (json!({"comp": named_one.as_str()}), vec!["--comp", named_one.as_str()]),
        (json!({"comp": 1}), vec!["--comp", "1"]),
        (json!({"comp": "1"}), vec!["--comp", "1"]),
        (json!({"comp": "+1"}), vec!["--comp", "+1"]),
        (json!({"comp": "-"}), vec!["--comp", "-"]),
        (json!({"comp": ""}), vec!["--comp", ""]),
        (json!({"comp": "Tall", "frame": 7, "scale": 0.5, "transparent": true}), vec!["--comp", "Tall", "--frame", "7", "--scale", "0.5", "--transparent"]),
    ];
    for (request, flags) in &cases {
        assert_frame_matches(&mut app, request, &cli_frame(scratch.root(), flags).expect("the command line rendered the frame"))
            .expect("the plugin rendered the frames");
    }
}

#[test]
fn each_rule_of_the_request_changes_the_frame_it_is_held_to() {
    // The comparison above would pass for a request that ignored a field if the command line's frames
    // did not differ by it too: so the fixture is shown to bite on each rule, against the command line.
    let scratch = Scratch::new().expect("a scratch folder");
    let (_app, comps) = frames_on_both_sides(&scratch).expect("the comps are built on both sides");
    let frame = |flags: &[&str]| cli_frame(scratch.root(), flags).expect("the command line rendered the frame");
    let size = |frame: &CliFrame| (frame.width, frame.height);
    let (now, start, later) = (frame(&[]), frame(&["--time", "0"]), frame(&["--time", "1"]));
    assert_ne!(now.rgba, start.rgba, "the saved current time is not the start");
    assert_ne!(start.rgba, later.rgba, "time moves the box");
    assert_eq!(frame(&["--frame", "10"]).rgba, later.rgba, "frame 10 at 10 fps is the same moment as second 1");
    assert_ne!(frame(&["--comp", "Tall"]).rgba, frame(&["--comp", "Tall", "--time", "0"]).rgba, "another comp is rendered at the active comp's time");
    assert_eq!(size(&now), (64, 48));
    assert_eq!(size(&frame(&["--max-side", "40"])), (40, 30), "the longest side is the cap");
    assert_eq!(size(&frame(&["--max-side", "40.7"])), (40, 30), "a fractional side is cut, not rounded");
    assert_eq!(size(&frame(&["--max-side", "-5"])), (64, 48), "a negative side is the full size");
    assert_eq!(size(&frame(&["--scale", "0.34"])), (22, 17), "a scale rounds to the nearest pixel");
    assert_eq!(size(&frame(&["--scale", "0"])), (1, 1), "a scale is never below one pixel");
    assert_eq!(size(&frame(&["--comp", "Tall", "--scale", "0.5"])), (15, 25), "a scale is of the longest side, here the height");
    assert_eq!(size(&frame(&["--comp", &comps.named_one.to_string()])), (20, 20));
    assert_eq!(size(&frame(&["--comp", "1"])), (64, 48), "the digits 1 are an id, not the comp named 1");
    let transparent = frame(&["--transparent"]);
    let alphas = |frame: &CliFrame| frame.rgba.as_chunks::<4>().0.iter().map(|pixel| pixel[3]).collect::<Vec<u8>>();
    assert!(alphas(&now).iter().all(|alpha| *alpha == 255), "a frame over the background is opaque");
    assert!(alphas(&transparent).contains(&0), "a transparent frame keeps the empty corner empty");
}

#[test]
fn a_frame_that_cannot_be_rendered_fails_in_the_commands_words_in_every_format() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (mut app, _comps) = frames_on_both_sides(&scratch).expect("the comps are built on both sides");
    let project = scratch.root().join("frames.ecproj").display().to_string();
    let theirs = |flags: &[&str]| {
        let mut args = vec!["render-frame", "--json", "--out", "x.png"];
        args.extend_from_slice(flags);
        cli(scratch.root(), &args).expect("the command line ran").failure().expect("the command line reported the failure")
    };
    // A comp named nowhere, by a name and by an id, refused by the render; and one a frame number or
    // a scale must first read the figures of, refused by the tool that reads them, in its own words.
    let cases: [(Value, Vec<&str>); 4] = [
        (json!({"comp": "nope"}), vec!["--project", &project, "--comp", "nope"]),
        (json!({"comp": 99999}), vec!["--project", &project, "--comp", "99999"]),
        (json!({"comp": "nope", "frame": 3}), vec!["--project", &project, "--comp", "nope", "--frame", "3"]),
        (json!({"comp": "nope", "scale": 0.5}), vec!["--project", &project, "--comp", "nope", "--scale", "0.5"]),
    ];
    let mut empty = scratch.app().expect("the app starts from prepared folders");
    let no_comp_open = theirs(&["--empty"]);
    for format in EVERY_FORMAT {
        for (request, flags) in &cases {
            match app.frame(request.clone(), format).expect_err("the comp is not there") {
                CraftError::NotFound { message } => assert_eq!(message, theirs(flags), "{request} as {format:?}: not the command line's words"),
                other => panic!("{request} as {format:?}: a missing comp is `NotFound`, got {other:?}"),
            }
        }
        match empty.frame(json!({}), format).expect_err("no comp is open") {
            CraftError::NotFound { message } => assert_eq!(message, no_comp_open, "{format:?}: not the command line's words for no comp open"),
            other => panic!("{format:?}: no comp open is `NotFound`, got {other:?}"),
        }
    }
}

#[test]
fn get_is_the_commands_get() {
    let mut documents = Documents::new().expect("the same document on both sides");
    for (params, arguments) in [
        (json!({"comp": 1, "layer": "T", "path": "transform/opacity"}), vec!["1", "T", "transform/opacity"]),
        (json!({"comp": 1, "layer": 27, "path": "transform/position", "time": 0.5}), vec!["1", "27", "transform/position", "--time", "0.5"]),
        (json!({"comp": "A", "layer": "T", "path": "effects/#1/blurriness"}), vec!["A", "T", "effects/#1/blurriness"]),
        (json!({"layer": "T", "path": "text/sourceText"}), vec!["-", "T", "text/sourceText"]),
    ] {
        let expected = documents.cli("get", &arguments).expect("the command line ran").doc().expect("the command line printed the property");
        let property = documents.app.request("get", params.clone()).expect("`get` answers");
        assert_eq!(property, expected, "`get` {params} answered differently from the command line");
    }
}

#[test]
fn set_is_the_commands_set() {
    let mut documents = Documents::new().expect("the same document on both sides");
    // Each case sets a property of its own, so the document the command line saves in place, one
    // call after another, is the document the plugin's session builds up the same way.
    for (params, arguments) in [
        (json!({"comp": 1, "layer": "T", "path": "transform/opacity", "value": 50}), vec!["1", "T", "transform/opacity", "50"]),
        (json!({"comp": 1, "layer": "T", "path": "transform/position", "value": [10, 20]}), vec!["1", "T", "transform/position", "[10,20]"]),
        (json!({"comp": 1, "layer": "T", "path": "text/sourceText", "value": "Bye"}), vec!["1", "T", "text/sourceText", "Bye"]),
        (json!({"comp": 1, "layer": "T", "path": "text/perChar3d", "value": true}), vec!["1", "T", "text/perChar3d", "true"]),
        (json!({"comp": 1, "layer": "Bg", "path": "transform/opacity", "value": 25, "time": 1}), vec!["1", "Bg", "transform/opacity", "25", "--time", "1"]),
        (json!({"layer": "T", "path": "transform/rotation", "expression": "time*90"}), vec!["-", "T", "transform/rotation", "--expression", "time*90"]),
        (
            json!({"comp": "A", "layer": 27, "path": "effects/#1/blurriness", "value": 12, "expression": "value*2"}),
            vec!["A", "27", "effects/#1/blurriness", "12", "--expression", "value*2"],
        ),
    ] {
        let mut arguments = arguments;
        arguments.push("--save");
        let mut expected = documents.cli("set", &arguments).expect("the command line ran").doc().expect("the command line printed the property");
        let property = documents.app.request("set", params.clone()).expect("`set` answers");
        assert_eq!(property, expected["result"], "`set` {params} answered differently from the command line");
        assert!(expected.as_object_mut().is_some_and(|entries| entries.remove("saved").is_some()), "the command line saved the document");
    }
    let (plugin, command_line) = documents.files().expect("both documents are saved");
    assert_eq!(plugin, command_line, "the document after the changes differs from the command line's");
}

#[test]
fn a_property_call_that_fails_fails_in_the_commands_words() {
    let mut documents = Documents::new().expect("the same document on both sides");
    assert_fails_as(
        &mut documents,
        vec![
            ("props", json!({"comp": "nope", "layer": "T"}), Oracle::Subcommand("props", vec!["nope", "T"]), "NotFound"),
            ("props", json!({"comp": 1, "layer": "Nobody"}), Oracle::Subcommand("props", vec!["1", "Nobody"]), "NotFound"),
            ("props", json!({"comp": 1}), Oracle::Tool("get_layer", json!({"comp": 1})), "InvalidParams"),
            ("get", json!({"comp": 1, "layer": "T", "path": "transform/nope"}), Oracle::Subcommand("get", vec!["1", "T", "transform/nope"]), "NotFound"),
            ("get", json!({"comp": 1, "layer": 99, "path": "transform/opacity"}), Oracle::Subcommand("get", vec!["1", "99", "transform/opacity"]), "NotFound"),
            ("get", json!({"comp": 1, "layer": "T"}), Oracle::Tool("get_property", json!({"comp": 1, "layer": "T"})), "InvalidParams"),
            ("set", json!({"comp": 1, "layer": "T", "path": "nope/x", "value": 5}), Oracle::Subcommand("set", vec!["1", "T", "nope/x", "5"]), "NotFound"),
            (
                "set",
                json!({"comp": 1, "layer": "T", "path": "transform/opacity", "value": "abc"}),
                Oracle::Subcommand("set", vec!["1", "T", "transform/opacity", "abc"]),
                "CommandFailed",
            ),
            (
                "set",
                json!({"comp": 1, "layer": "T", "path": "transform/opacity"}),
                Oracle::Tool("set_property", json!({"comp": 1, "layer": "T", "path": "transform/opacity"})),
                "InvalidParams",
            ),
        ],
    )
    .expect("every call fails as the command line fails it");
}

/// The project file one side holds under `name`, as a whole document.
fn saved_file(dir: &Path, name: &str, dirs: &[&Path]) -> std::result::Result<Value, String> {
    let text = std::fs::read_to_string(dir.join(name)).map_err(|e| format!("`{name}` reads: {e}"))?;
    let mut file: Value = serde_json::from_str(&text).map_err(|e| format!("`{name}` is JSON: {e}"))?;
    scrub(&mut file, dirs);
    Ok(file)
}

/// The command line's own run of one file command on its copy of the document, in its folder.
fn cli_command(documents: &Documents, command: &str, params: &Value) -> std::result::Result<Value, String> {
    let params = params.to_string();
    documents.cli("exec", &[command, &params])?.doc()
}

#[test]
fn the_file_methods_are_the_file_commands() {
    let mut documents = Documents::new().expect("the same document on both sides");
    // Each method beside the command line's command it goes through, the file it writes, and the
    // parameters it takes: the command line's `--project`, `--save` and `--save-as` run these.
    for (method, name, command, file) in [
        ("saveAs", Some("copy.ecproj"), "file.saveAs", Some("copy.ecproj")),
        ("save", Some("other.ecproj"), "file.save", Some("other.ecproj")),
        ("open", Some("doc.ecproj"), "file.open", None),
        ("save", None, "file.save", Some("doc.ecproj")),
        ("new", None, "file.newProject", None),
    ] {
        let at = |dir: &Path| name.map_or_else(|| json!({}), |name| json!({"path": dir.join(name).display().to_string()}));
        let mut expected = cli_command(&documents, command, &at(documents.scratch.root())).expect("the command line ran the command");
        let mut result = documents.app.request(method, at(documents.scratch.path())).expect("the method answers");
        scrub(&mut result, &documents.dirs());
        scrub(&mut expected, &documents.dirs());
        assert_eq!(result, expected, "`{method}` answered differently from the command line's `{command}`");
        if let Some(file) = file {
            let (mine, theirs) = (saved_file(documents.scratch.path(), file, &documents.dirs()), saved_file(documents.scratch.root(), file, &documents.dirs()));
            assert_eq!(
                mine.expect("the plugin wrote the file"),
                theirs.expect("the command line wrote the file"),
                "`{method}` wrote another file than the command line"
            );
        }
    }
    // `new` leaves the session as the command line's `--empty` starts it.
    let command_line =
        cli(documents.scratch.root(), &["info", "--json", "--empty"]).expect("the command line ran").doc().expect("the command line printed its document");
    let (mut command_line, mut plugin) = (command_line, plugin_info(&mut documents.app).expect("the plugin answers info"));
    compare_info(&mut command_line, &mut plugin, &documents.dirs());

    // With the settings naming a project template, `new` opens the template as an untitled project
    // and answers which: the result, and the project it leaves, are the command line's.
    let mut command_line_steps = template_settings(&documents.scratch.root().join("doc.ecproj"));
    command_line_steps.extend([("file.newProject".to_string(), json!({})), ("project.summary".to_string(), json!({}))]);
    let mut command_line =
        cli_run(documents.scratch.root(), &command_line_steps).expect("the command line ran").doc().expect("the command line printed its results");
    documents
        .app
        .request("run", json!({"steps": steps_document(&template_settings(&documents.scratch.path().join("doc.ecproj")))}))
        .expect("the settings name a template");
    let mut opened = documents.app.request("new", json!({})).expect("`new` opens the template");
    let mut project = documents.app.request("exec", json!({"command": "project.summary"})).expect("the plugin summarises its project");
    for doc in [&mut command_line, &mut opened, &mut project] {
        scrub(doc, &documents.dirs());
    }
    assert!(opened.get("template").is_some(), "`new` answered no template: {opened}");
    assert_eq!(opened, command_line[2]["result"], "`new` from a template answered differently from the command line");
    let mut expected = command_line[3]["result"].clone();
    mark_accel_note(&mut project);
    mark_accel_note(&mut expected);
    assert_eq!(project, expected, "the project `new` opened from a template differs from the command line's");
}

#[test]
fn a_file_call_that_fails_fails_in_the_commands_words() {
    let mut documents = Documents::new().expect("the same document on both sides");
    for dir in [documents.scratch.root(), documents.scratch.path()] {
        std::fs::write(dir.join("junk.ecproj"), "not a project").expect("a file that is no project");
        std::fs::write(dir.join("binary.ecproj"), [0xff, 0xfe, 0x00, 0x80]).expect("a file that is no text");
        std::fs::write(dir.join("broken.ecprojx"), "<?xml version=\"1.0\"?><project>").expect("a file that is broken XML");
        std::fs::create_dir(dir.join("folder.ecproj")).expect("a folder where a file is named");
        std::fs::create_dir(dir.join("locked")).expect("a folder to lock");
        std::fs::set_permissions(dir.join("locked"), std::fs::Permissions::from_mode(0o555)).expect("a folder nothing may be written in");
    }
    let (root, container) = (documents.scratch.root().to_path_buf(), documents.scratch.path().to_path_buf());
    let at = |dir: &Path, name: &str| json!({"path": dir.join(name).display().to_string()});
    let command = Oracle::command;
    // A file the disk will not read or write is `Io`, whatever the system's reason; a file that reads
    // but holds no project, and a call that names no file, are the command's own failure.
    assert_fails_as(
        &mut documents,
        vec![
            ("open", at(&container, "missing.ecproj"), command("file.open", at(&root, "missing.ecproj")), "Io"),
            ("open", at(&container, "folder.ecproj"), command("file.open", at(&root, "folder.ecproj")), "Io"),
            ("open", at(&container, "junk.ecproj"), command("file.open", at(&root, "junk.ecproj")), "CommandFailed"),
            ("open", at(&container, "binary.ecproj"), command("file.open", at(&root, "binary.ecproj")), "CommandFailed"),
            ("open", at(&container, "broken.ecprojx"), command("file.open", at(&root, "broken.ecprojx")), "CommandFailed"),
            ("open", json!({}), command("file.open", json!({})), "CommandFailed"),
            ("saveAs", json!({}), command("file.saveAs", json!({})), "CommandFailed"),
            ("saveAs", at(&container, "no-folder/x.ecproj"), command("file.saveAs", at(&root, "no-folder/x.ecproj")), "Io"),
            ("saveAs", at(&container, "locked/x.ecproj"), command("file.saveAs", at(&root, "locked/x.ecproj")), "Io"),
            ("save", at(&container, "locked/x.ecproj"), command("file.save", at(&root, "locked/x.ecproj")), "Io"),
        ],
    )
    .expect("every call fails as the command line fails it");
    // A project that was never saved has nowhere to save to: the confinement's own words, which say
    // to give a `path` inside the write root.
    documents.app.request("new", json!({})).expect("a project with no file");
    let unsaved = Oracle::command("file.save", json!({}));
    assert_fails_as(&mut documents, vec![("save", json!({}), unsaved, "Io")]).expect("every call fails as the command line fails it");
}

/// The names in a folder, sorted. A folder or an entry that cannot be read is an error, never
/// left out: the list is how a test sees that nothing was written there.
fn entries(dir: &Path) -> std::result::Result<Vec<String>, String> {
    let listed = std::fs::read_dir(dir).map_err(|e| format!("`{}` lists: {e}", dir.display()))?;
    let mut names = listed
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<Vec<String>>>()
        .map_err(|e| format!("an entry of `{}` reads: {e}", dir.display()))?;
    names.sort();
    Ok(names)
}

/// The settings that make `new` open `template` as an untitled project. They do not outlast a call
/// of the command line, so its side runs them in the same `run` as the call that reads them.
fn template_settings(template: &Path) -> Vec<(String, Value)> {
    vec![
        ("prefs.set".to_string(), json!({"key": "project.useTemplate", "value": true})),
        ("prefs.set".to_string(), json!({"key": "project.templatePath", "value": template.display().to_string()})),
    ]
}

/// `new` with the settings naming a project template it cannot use. The command line can only ask
/// for it as the step of a `run` after its settings, which words the failure as every failed step,
/// `{command}: {message} (step n)` (plan §14); the plugin's `new` is asked on its own, after its
/// settings.
#[test]
fn a_new_project_whose_template_will_not_open_fails_as_the_command_line_fails_it() {
    let mut documents = Documents::new().expect("the same document on both sides");
    for dir in [documents.scratch.root(), documents.scratch.path()] {
        std::fs::write(dir.join("binary.ecproj"), [0xff, 0xfe, 0x00, 0x80]).expect("a template that is no text");
        std::fs::write(dir.join("junk.ecproj"), "not a project").expect("a template that is no project");
        std::fs::create_dir(dir.join("folder")).expect("a folder where a template is named");
    }
    // The disk will not read the template: `Io`. A template that read but is no project is the
    // command's own failure.
    for (template, row) in [("missing.ecproj", "Io"), ("folder", "Io"), ("binary.ecproj", "CommandFailed"), ("junk.ecproj", "CommandFailed")] {
        let mut steps = template_settings(&documents.scratch.root().join(template));
        steps.push(("file.newProject".to_string(), json!({})));
        let step = steps.len() - 1;
        let expected = cli_run(documents.scratch.root(), &steps).expect("the command line ran").failure().expect("the command line reported the failure");
        let settings = steps_document(&template_settings(&documents.scratch.path().join(template)));
        documents.app.request("run", json!({"steps": settings})).expect("the settings name the template");
        let error = documents.app.request("new", json!({})).expect_err("the template would not open");
        let message = match &error {
            CraftError::Io { message } if row == "Io" => message,
            CraftError::CommandFailed { command, message, step: None } if row == "CommandFailed" && command == "file.newProject" => message,
            other => panic!("a `{template}` template is `{row}` from `file.newProject`, got {other:?}"),
        };
        let words = scrubbed(&format!("file.newProject: {message} (step {step})"), &documents.dirs());
        assert_eq!(words, scrubbed(&expected, &documents.dirs()), "a `{template}` template failed in other words than the command line's");
    }
}

#[test]
fn the_file_methods_stay_inside_the_confinement() {
    let mut documents = Documents::new().expect("the same document on both sides");
    // Outside the roots of both sides: the scratch folder's own, which no other test or run shares.
    let elsewhere = documents.scratch.outside().to_path_buf();
    let readable = documents.scratch.outside_file().expect("a file outside both roots");
    let escape = elsewhere.join("escape.ecproj").display().to_string();
    // The links that would lead out, laid alike in the command line's folder and the container: to a
    // folder outside, and to a file outside.
    let (root, container) = (documents.scratch.root().to_path_buf(), documents.scratch.path().to_path_buf());
    for dir in [&root, &container] {
        std::os::unix::fs::symlink(&elsewhere, dir.join("out")).expect("a link to a folder outside");
        std::os::unix::fs::symlink(&readable, dir.join("hosts")).expect("a link to a file outside");
    }
    let path = |dir: &Path, rest: &str| format!("{}/{rest}", dir.display());
    let outside = |command: &str, path: &str| Oracle::command(command, json!({"path": path}));
    let mut cases = vec![
        ("open", json!({"path": readable}), outside("file.open", &readable.display().to_string()), "Io"),
        ("save", json!({"path": escape}), outside("file.save", &escape), "Io"),
        ("saveAs", json!({"path": escape}), outside("file.saveAs", &escape), "Io"),
        // A relative path, which the command line's own folder holds a project at.
        ("open", json!({"path": "doc.ecproj"}), outside("file.open", "doc.ecproj"), "Io"),
        ("saveAs", json!({"path": "escape.ecproj"}), outside("file.saveAs", "escape.ecproj"), "Io"),
    ];
    // `..` out of the container reaches the command line's project; out of its folder, the folder above.
    for (method, command, rest) in
        [("open", "file.open", "../doc.ecproj"), ("save", "file.save", "../escape.ecproj"), ("saveAs", "file.saveAs", "../escape.ecproj")]
    {
        cases.push((method, json!({"path": path(&container, rest)}), outside(command, &path(&root, rest)), "Io"));
    }
    cases.push(("open", json!({"path": path(&container, "hosts")}), outside("file.open", &path(&root, "hosts")), "Io"));
    for (method, command) in [("save", "file.save"), ("saveAs", "file.saveAs")] {
        cases.push((method, json!({"path": path(&container, "out/escape.ecproj")}), outside(command, &path(&root, "out/escape.ecproj")), "Io"));
    }
    assert_fails_as(&mut documents, cases).expect("every call fails as the command line fails it");
    assert_eq!(entries(&elsewhere).expect("the folder outside lists"), ["readable.txt"], "the refused writes left nothing outside the container");
    for written in [root.join("escape.ecproj"), PathBuf::from("escape.ecproj")] {
        assert!(!written.exists(), "the refused writes left nothing at {}", written.display());
    }

    // A link to a file that is not there yet leads nowhere: the save replaces the link with the
    // project, inside the container, as the command line's does in its folder.
    for (dir, target) in [(&root, "cli.ecproj"), (&container, "plugin.ecproj")] {
        std::os::unix::fs::symlink(elsewhere.join(target), dir.join("dangling.ecproj")).expect("a link to a file not there yet");
    }
    let mut expected = cli_command(&documents, "file.saveAs", &json!({"path": path(&root, "dangling.ecproj")})).expect("the command line saved");
    let mut saved = documents.app.request("saveAs", json!({"path": path(&container, "dangling.ecproj")})).expect("`saveAs` saves");
    scrub(&mut saved, &documents.dirs());
    scrub(&mut expected, &documents.dirs());
    assert_eq!(saved, expected, "`saveAs` through a link answered differently from the command line");
    let link = std::fs::symlink_metadata(container.join("dangling.ecproj")).expect("the saved file is there");
    assert!(link.file_type().is_file(), "the project replaced the link inside the container");
    assert_eq!(entries(&elsewhere).expect("the folder outside lists"), ["readable.txt"], "nothing was written where the link led");
}

#[test]
fn the_file_methods_read_the_bundle_and_never_write_it() {
    let mut documents = Documents::in_scratch(Scratch::with_bundle().expect("a scratch folder with a bundle")).expect("the same document on both sides");
    let bundle = documents.scratch.bundle().expect("the scratch folder has a bundle").to_path_buf();
    // A project the app ships: outside the command line's folder, as it is outside the container.
    let shipped = bundle.join("shipped.ecproj");
    std::fs::copy(documents.scratch.root().join("doc.ecproj"), &shipped).expect("a project in the bundle");
    let shipped_bytes = std::fs::read(&shipped).expect("the shipped project reads");
    let at = |path: &Path| json!({"path": path.display().to_string()});
    let expected = cli_command(&documents, "file.open", &at(&shipped)).expect("the command line opened the shipped project");
    let opened = documents.app.request("open", at(&shipped)).expect("a project in the bundle opens");
    assert_eq!(opened, expected, "`open` of the bundle answered differently from the command line");
    // Saved in place, the project opened from the bundle would go back into it; by name, a new file.
    let copy = bundle.join("copy.ecproj");
    let outside = |command: &str| Oracle::command(command, at(&copy));
    let in_place = Oracle::ToolOn(&shipped, "execute_command", json!({"command": "file.save", "params": {}}));
    assert_fails_as(
        &mut documents,
        vec![("save", json!({}), in_place, "Io"), ("save", at(&copy), outside("file.save"), "Io"), ("saveAs", at(&copy), outside("file.saveAs"), "Io")],
    )
    .expect("every call fails as the command line fails it");
    assert_eq!(std::fs::read(&shipped).expect("the shipped project reads"), shipped_bytes, "the shipped project is as it was");
    assert!(!copy.exists(), "nothing new is written into the bundle");
}

#[test]
fn a_replay_is_the_commands_replay() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let passing = [
        r#"{"start": "new"}"#,
        r#"{"command": "comp.new", "params": {"name": "R", "width": 64, "height": 64}}"#,
        r#"{"expect": {"query": "comp.info", "params": {"comp": 1}, "pointer": "/name", "equals": "R"}}"#,
        r#"{"expect_no_error": true}"#,
    ]
    .join("\n");
    let failing =
        [passing.as_str(), r#"{"command": "comp.info", "params": {"comp": "nope"}}"#, r#"{"command": "comp.new", "params": {"name": "after"}}"#, r#"not json"#]
            .join("\n");
    for (script, keep_going) in [(passing.as_str(), false), (failing.as_str(), false), (failing.as_str(), true)] {
        let mut args = vec!["replay", "--json"];
        if keep_going {
            args.push("--keep-going");
        }
        let file = scratch.root().join("repro.jsonl");
        std::fs::write(&file, script).expect("the script is written");
        let file_text = file.display().to_string();
        args.push(&file_text);
        let command_line = cli(scratch.root(), &args).expect("the command line ran").doc().expect("the command line printed its report");
        let plugin = app.request("replay", json!({"script": script, "keepGoing": keep_going})).expect("`replay` answers its report");
        assert_eq!(plugin, command_line, "the replay report differs for keepGoing={keep_going}");
    }
}

/// One MCP session, line for line, against the command line's confined `mcp`: every reply is the whole
/// JSON-RPC document the command line answers — the handshake, a notification it does not answer,
/// the tool list, a command that runs, one that fails, a script and a read the confinement refuses,
/// a JSON-RPC batch, an unknown tool, a line that is no JSON, and a download whose arguments the
/// server refuses before it runs anything.
#[test]
fn an_mcp_session_is_answered_as_the_commands_mcp_answers_it() {
    let scratch = Scratch::new().expect("a scratch folder");
    let call =
        |id: u64, name: &str, arguments: Value| json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": arguments}});
    let lines: Vec<String> = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "parity", "version": "1"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "ping"}),
        call(4, "execute_command", json!({"command": "comp.new", "params": {"name": "M", "width": 64, "height": 64}})),
        call(5, "execute_command", json!({"command": "comp.info", "params": {"comp": "nope"}})),
        call(6, "run_script", json!({"code": "1"})),
        call(7, "execute_command", json!({"command": "file.open", "params": {"path": "/etc/hosts"}})),
        json!([call(8, "command_run", json!({"id": "layer.newSolid", "params": {"comp": 1}})), call(9, "execute_command", json!({"command": "comp.info", "params": {"comp": 1}}))]),
        call(10, "no_such_tool", json!({})),
        call(11, "execute_command", json!({"command": "roto.model.download", "params": {"id": "no-such-model"}, "extra": 1})),
    ]
    .iter()
    .map(Value::to_string)
    .chain(["not json".to_string()])
    .collect();
    let mut command_line = cli_mcp_session(scratch.root(), None, &lines).expect("the confined command line answered");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let mut plugin: Vec<Value> =
        lines.iter().filter_map(|line| app.mcp(line)).map(|reply| serde_json::from_str(&reply).expect("the plugin answers JSON-RPC lines")).collect();
    for reply in &mut command_line {
        scrub(reply, &[scratch.root()]);
    }
    for reply in &mut plugin {
        scrub(reply, &[scratch.path()]);
    }
    assert_eq!(plugin.len(), command_line.len(), "the plugin answered other lines than the command line: {plugin:?}");
    for (mine, theirs) in plugin.iter().zip(&command_line) {
        assert_eq!(mine, theirs, "the reply differs from the command line's");
    }
}

/// T18: the scenario's two calls — `script.run` of `app.project.numItems`, and of a script that
/// fails — through the command line's confined `mcp` and the plugin's `exec`. `script.run` is a
/// command that runs scripts, so both confined doors refuse each before its script runs, and they
/// refuse in the same words. The phone build runs no script (PD13), so there is no output or error
/// position to compare.
#[test]
fn the_confined_script_refusal_is_the_commands_refusal() {
    let scratch = Scratch::new().expect("a scratch folder");
    for code in ["app.project.numItems", "throw new Error('the scenario fails here')"] {
        let (command_line, error) = confined_refusal_pair(&scratch, "script.run", &json!({"code": code})).expect("both doors refused");
        match error {
            CraftError::Unsupported { message } => {
                assert_eq!(scrubbed(&message, &[scratch.path()]), command_line, "the confined refusal of `{code}` differs from the command line's")
            }
            other => panic!("a confined script run must be `Unsupported`, got {other:?}"),
        }
    }
}
