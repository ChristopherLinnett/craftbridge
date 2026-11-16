//! The acceptance of plan §28 T1 to T3 as a property of the plugin: on the same inputs each of the
//! four methods answers exactly what the EffectCraft command line at the pin answers. Each test
//! drives both — the oracle command line from `tools/oracle.sh` and a fresh plugin app in scratch
//! folders of its own — and compares whole documents, after rewriting each scratch folder to one
//! marker so the folders themselves do not count as a gap; a failure is compared in its words. The
//! command line confines only its `mcp` subcommand, so the comparison that needs the confinement on
//! both sides drives the command line through that door: one line of its confined `mcp`, which
//! answers through the same kind of server the plugin answers through.

mod common;

use common::Scratch;
use craftbridge_core::{CraftApp, CraftError};
use craftbridge_effects::EffectCraftApp;
use serde_json::{Value, json};
use std::io::Write;
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
/// and no inherited environment, exactly as the conformance runner gives the oracle (plan §28).
fn oracle_in(dir: &Path) -> std::result::Result<Command, String> {
    let mut command = Command::new(oracle()?);
    command.current_dir(dir).env_clear().env("EFFECTCRAFT_CONFIG_DIR", dir.join("cli-config"));
    Ok(command)
}

/// One run of the command line in `dir`.
fn cli(dir: &Path, args: &[&str]) -> std::result::Result<Run, String> {
    let output = oracle_in(dir)?.args(args).stdin(Stdio::null()).output().map_err(|e| format!("the oracle runs: {e}"))?;
    Ok(Run { stdout: String::from_utf8_lossy(&output.stdout).into_owned(), stderr: String::from_utf8_lossy(&output.stderr).into_owned() })
}

/// One confined `mcp` run of the command line: the folder is both of its automation roots, one line
/// of JSON-RPC is sent, and the text of the one reply it answers is returned. This is the command
/// line's own confined door — the plugin reaches its session through the same kind of server.
fn cli_mcp_confined(dir: &Path, request: Value) -> std::result::Result<Value, String> {
    let dir_text = dir.display().to_string();
    let mut child = oracle_in(dir)?
        .args(["mcp", "--empty", "--automation-read-root", &dir_text, "--automation-write-root", &dir_text])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("the mcp run starts: {e}"))?;
    {
        let stdin = child.stdin.as_mut().ok_or("the mcp run takes a line")?;
        stdin.write_all(request.to_string().as_bytes()).map_err(|e| format!("the line goes in: {e}"))?;
        stdin.write_all(b"\n").map_err(|e| format!("the line ends: {e}"))?;
    }
    let output = child.wait_with_output().map_err(|e| format!("the mcp run answers: {e}"))?;
    let answer = String::from_utf8_lossy(&output.stdout);
    let reply = answer
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .find(|reply| reply.get("id") == Some(&json!(1)))
        .ok_or("the mcp run answers the request")?;
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
    let mut plugin = without(plugin_doc, &["engine", "pin", "gpu", "hardwareDecoding"]);
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

/// A render whose one item fails: the queue renders a comp to an output that is a folder, which
/// the exporter cannot open as a file. `output` is filled in per side.
fn failing_render_steps(output: &Path) -> Vec<(String, Value)> {
    vec![
        ("comp.new".to_string(), json!({"name": "R", "width": 64, "height": 64, "duration": 0.1, "frameRate": 10})),
        ("layer.newSolid".to_string(), json!({"comp": 1, "name": "Bg"})),
        ("renderQueue.add".to_string(), json!({"comp": 1, "output": output.display().to_string()})),
    ]
}

/// Make the folder a failing render writes to, for one side.
fn folder_in_the_way(output: &Path) -> std::result::Result<(), String> {
    std::fs::create_dir(output).map_err(|e| format!("the folder in the render's way is made: {e}"))
}

#[test]
fn a_render_with_a_failed_item_fails_as_the_command_line_fails_it() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (cli_output, plugin_output) = (scratch.root().join("out.mov"), scratch.path().join("out.mov"));
    folder_in_the_way(&cli_output).expect("the command line's render is blocked");
    folder_in_the_way(&plugin_output).expect("the plugin's render is blocked");
    let render = ("renderQueue.render".to_string(), json!({}));

    // Under `run`: the render's step fails, in the command line's words, with its step.
    let mut cli_steps = failing_render_steps(&cli_output);
    cli_steps.push(render.clone());
    let command_line = cli_run(scratch.root(), &cli_steps).expect("the command line ran").failure().expect("the command line reported the failure");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let mut plugin_steps = failing_render_steps(&plugin_output);
    plugin_steps.push(render);
    let error = app.request("run", json!({"steps": steps_document(&plugin_steps)})).expect_err("the failed item fails the run");
    assert!(matches!(error, CraftError::CommandFailed { step: Some(3), .. }), "a failed render item is `CommandFailed` at its step, got {error:?}");
    assert_eq!(
        scrubbed(&error.to_string(), &[scratch.path()]),
        scrubbed(&command_line, &[scratch.root()]),
        "the failed render is not in the command line's words"
    );

    // Under `exec`: the same render of a saved queue fails as the command line's `exec` fails it.
    let queued = scratch.root().join("queued.ecproj");
    let mut save = failing_render_steps(&cli_output);
    save.push(("file.saveAs".to_string(), json!({"path": queued.display().to_string()})));
    cli_run(scratch.root(), &save).expect("the command line ran").doc().expect("the command line saved its queue");
    let command_line = cli(scratch.root(), &["exec", "--json", "--project", &queued.display().to_string(), "renderQueue.render"])
        .expect("the command line ran")
        .failure()
        .expect("the command line reported the failure");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    app.request("run", json!({"steps": steps_document(&failing_render_steps(&plugin_output))})).expect("the plugin's queue is set up");
    match app.request("exec", json!({"command": "renderQueue.render"})).expect_err("the failed item fails the render") {
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
}

#[test]
fn the_confinement_answers_in_the_commands_words() {
    let scratch = Scratch::new().expect("a scratch folder");
    let command_line = cli_mcp_confined(
        scratch.root(),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "execute_command", "arguments": {"command": "file.open", "params": {"path": "/etc/hosts"}}}}),
    )
    .expect("the confined command line answered");
    let command_line = match command_line {
        Value::String(message) => scrubbed(&message, &[scratch.root()]),
        other => panic!("the confined command line answered {other} where it should have refused"),
    };

    let mut app = scratch.app().expect("the app starts from prepared folders");
    let answer = app.request("exec", json!({"command": "file.open", "params": {"path": "/etc/hosts"}}));
    match answer.expect_err("the confinement refuses the read") {
        CraftError::Io { message } => {
            assert_eq!(scrubbed(&message, &[scratch.path()]), command_line, "the confinement refusal differs from the command line's")
        }
        other => panic!("a refused read must be `Io`, got {other:?}"),
    }
}
