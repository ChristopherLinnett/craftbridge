//! The `replay` method: EffectCraft's own replay of a JSON-lines repro script, run on the confined
//! session (plan §14).
//!
//! `replay::run` executes each line on the session itself, so a line never passes the checks
//! `Backend::exec` makes — the confinement's, and the door that withholds the model downloads
//! and the renders and fills no one has listed (AD11, AD3, AD20). The method therefore reads the script's lines
//! before running any of them and refuses the whole call if either check refuses one of them, the way `run` checks its whole batch before its
//! first step: a refused line must not leave the earlier lines' work behind. The lines then pass the
//! door that hands a command writing beside its source the copy to read (AD22), and a line whose
//! parameters it rewrote is written again in the script the replay runs.

use std::borrow::Cow;
use std::path::Path;

use craftbridge_core::{CraftError, Result};
use effectcraft_automation::Backend;
use effectcraft_engine::replay;
use serde_json::Value;

use crate::imports;
use crate::methods::{bool_param, check_unrun, empty_params, object_params, refuse_process, reject_unknown, string_param};

/// A command a replay of a script could run: its line, counted from one as the report counts them,
/// the command, its parameters, and where they sit in the line's object, `None` for the `start`
/// line's project, whose parameters the replay builds itself.
type Found = (usize, String, Value, Option<&'static str>);

/// `replay`: `{script, keepGoing?}`, answering the report EffectCraft's replay makes — what the command
/// line prints with `--json`. A script whose steps fail is an answer, `ok: false` with the first
/// failure named, as it is the command line's report; a script a check refuses is an error, and runs
/// nothing. A line that would write beside a source in a folder the session may not write reads a
/// copy in `folder`, the app's staged folder.
pub(crate) fn replay(backend: &mut Backend, folder: &Path, params: &Value) -> Result<Value> {
    let obj = object_params(params, "replay")?;
    reject_unknown(&obj, "replay", &["script", "keepGoing"])?;
    let script = string_param(&obj, "script", "replay")?.ok_or_else(|| CraftError::invalid_params("`replay`: needs a `script`"))?;
    let options = replay::Options { keep_going: bool_param(&obj, "keepGoing", "replay")?.unwrap_or(false), base_dir: None };
    let mut found = commands_of(script);
    for (line, command, step_params, _) in &found {
        refuse_process(command, step_params, None).and_then(|()| check_unrun(backend, command, step_params)).map_err(|error| on_line(error, *line))?;
    }
    // Every line the replay could run goes to the door, the `start` project's `file.open` among them:
    // it is a command the call reaches, and a conversion later in the script cannot be handed a copy of
    // what that project shows. What the door changes of a line whose parameters it builds itself is not
    // written back, and nothing needs it to be: a project is read from where it lies, so that its own
    // media resolve beside it, and a script that converts a file its project shows where it must be
    // read from a copy is refused before a copy of anything is made.
    let mut calls: Vec<(&str, &mut Value)> = found.iter_mut().map(|(_, command, step_params, _)| (command.as_str(), step_params)).collect();
    let staged = imports::staged(backend, Some(folder), &mut calls)?;
    let script = if staged.rewritten() { Cow::Owned(rewritten(script, &found)) } else { Cow::Borrowed(script) };
    let session = backend.session().ok_or_else(|| CraftError::internal("`replay`: the backend holds no session"))?;
    let report = replay::run(session, &script, &options);
    staged.finish(backend);
    serde_json::to_value(&report).map_err(|error| CraftError::internal(format!("`replay`: the report cannot be written: {error}")))
}

/// Every command a replay of `script` could run, with its line (counted from one, as the report
/// counts them) and parameters. EffectCraft's replay reads these lines with a parser it keeps to
/// itself, from the format its own documentation gives (`crates/engine/src/replay.rs`:1-27): a
/// command is a line's `command`, a query assertion's `expect.query` and a `start` object's
/// `project`, which opens it. This reads every line for all three, whichever position the line
/// holds, so it names every command the replay can run and may name one it would not, such as a
/// `start` that is not the first line: a check that refuses a command the replay would not run is
/// harmless, one that missed a command it runs is not. Blank lines, `//` and `#` comments, a leading
/// byte order mark and the replay's limits on size and lines are skipped as the replay skips them;
/// a script past the size limit holds no step, because the replay refuses it unread.
fn commands_of(script: &str) -> Vec<Found> {
    if script.len() > replay::MAX_SCRIPT_BYTES {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (index, text) in lines_of(script).enumerate().take(replay::MAX_LINES) {
        let text = text.trim();
        if text.is_empty() || text.starts_with("//") || text.starts_with('#') {
            continue;
        }
        let Ok(Value::Object(line)) = serde_json::from_str::<Value>(text) else { continue };
        let number = index.saturating_add(1);
        if let Some(command) = line.get("command").and_then(Value::as_str) {
            found.push((number, command.to_string(), params_of(line.get("params")), Some("/params")));
        }
        if let Some(expect) = line.get("expect")
            && let Some(query) = expect.get("query").and_then(Value::as_str)
        {
            found.push((number, query.to_string(), params_of(expect.get("params")), Some("/expect/params")));
        }
        if let Some(path) = line.get("start").and_then(|start| start.get("project")).and_then(Value::as_str) {
            found.push((number, "file.open".to_string(), crate::methods::object([("path", Value::from(path))]), None));
        }
    }
    found
}

/// The lines of a script as the replay reads them, past a leading byte order mark.
fn lines_of(script: &str) -> std::str::Lines<'_> {
    script.strip_prefix('\u{feff}').unwrap_or(script).lines()
}

/// `script` with the parameters of each command `found` names written again where its line holds
/// them, every other line as it was. A line's parameters that were not an object, which the replay
/// reads as none, stay as they were, as do the `start` line's.
fn rewritten(script: &str, found: &[Found]) -> String {
    let mut lines: Vec<String> = lines_of(script).map(str::to_string).collect();
    for (number, _, params, at) in found {
        let (Some(at), Some(text)) = (at, number.checked_sub(1).and_then(|index| lines.get_mut(index))) else { continue };
        let Ok(mut line) = serde_json::from_str::<Value>(text.trim()) else { continue };
        if let Some(slot) = line.pointer_mut(at).filter(|slot| slot.is_object()) {
            *slot = params.clone();
            *text = line.to_string();
        }
    }
    lines.join("\n")
}

/// A line's parameters as the replay reads them: an object, or none. Anything else is a line the
/// replay reports invalid and never runs.
fn params_of(params: Option<&Value>) -> Value {
    match params {
        Some(object @ Value::Object(_)) => object.clone(),
        _ => empty_params(),
    }
}

/// The refusal of a line, with the line named in its words as the report names a failed one.
fn on_line(error: CraftError, line: usize) -> CraftError {
    let at = |message: String| format!("line {line}: {message}");
    match error {
        CraftError::InvalidParams { message } => CraftError::invalid_params(at(message)),
        CraftError::NotFound { message } => CraftError::not_found(at(message)),
        CraftError::Unsupported { message } => CraftError::unsupported(at(message)),
        CraftError::Io { message } => CraftError::io(at(message)),
        CraftError::Gpu { message } => CraftError::gpu(at(message)),
        CraftError::Internal { message } => CraftError::internal(at(message)),
        CraftError::CommandFailed { command, message, step } => CraftError::command_failed(command, at(message), step),
        CraftError::Cancelled => CraftError::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Each spelling a replay script gives a command, and each line it skips.
    #[test]
    fn the_commands_of_a_script_are_those_the_replay_reads() {
        let script = [
            "\u{feff}{\"start\": {\"project\": \"/a/b.ecproj\"}, \"allow_errors\": true}",
            "",
            "// a comment",
            "# another",
            "  {\"command\": \"comp.new\", \"params\": {\"name\": \"A\"}}  ",
            "{\"expect\": {\"query\": \"comp.info\", \"params\": {\"comp\": 1}, \"pointer\": \"/name\"}}",
            "{\"expect_no_error\": true}",
            "not json",
            "[1, 2]",
            "{\"command\": \"layer.newSolid\", \"params\": [1]}",
        ]
        .join("\n");
        let commands: Vec<Found> = commands_of(&script);
        assert_eq!(
            commands,
            vec![
                (1, "file.open".to_string(), json!({"path": "/a/b.ecproj"}), None),
                (5, "comp.new".to_string(), json!({"name": "A"}), Some("/params")),
                (6, "comp.info".to_string(), json!({"comp": 1}), Some("/expect/params")),
                (10, "layer.newSolid".to_string(), json!({}), Some("/params")),
            ]
        );
    }

    /// A rewritten script holds each command's parameters where its line had them, a command's and
    /// a query's in one line alike, and every other line as it was, the `start` line and a line whose
    /// parameters the replay reads as none among them.
    #[test]
    fn a_rewritten_script_holds_each_commands_parameters_where_its_line_had_them() {
        let script = [
            "\u{feff}{\"start\": {\"project\": \"/b/p.ecproj\"}}",
            "# a comment",
            "{\"command\": \"file.importLottie\", \"params\": {\"path\": \"/b/a.json\"}, \"expect\": {\"query\": \"comp.info\", \"params\": {\"path\": \"/b/a.json\"}}}",
            "{\"command\": \"layer.newSolid\", \"params\": [1]}",
        ]
        .join("\r\n");
        let mut found = commands_of(&script);
        for (.., params, _) in &mut found {
            if let Some(path) = params.get_mut("path").filter(|path| *path == "/b/a.json") {
                *path = json!("/s/1/a.json");
            }
        }
        let lines: Vec<String> = rewritten(&script, &found).lines().map(str::to_string).collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "{\"start\": {\"project\": \"/b/p.ecproj\"}}");
        assert_eq!(lines[1], "# a comment");
        let line: Value = serde_json::from_str(&lines[2]).expect("the line is JSON");
        assert_eq!(
            line,
            json!({"command": "file.importLottie", "params": {"path": "/s/1/a.json"}, "expect": {"query": "comp.info", "params": {"path": "/s/1/a.json"}}})
        );
        assert_eq!(lines[3], "{\"command\": \"layer.newSolid\", \"params\": [1]}");
        assert_eq!(commands_of(&rewritten(&script, &found)), found, "the replay reads the rewritten script as the parameters say");
    }

    /// A script past the replay's size limit holds no step (the replay refuses it unread), and one
    /// past its line limit is read only as far as the replay reads it.
    #[test]
    fn the_replays_own_limits_bound_the_read() {
        let oversize = " ".repeat(replay::MAX_SCRIPT_BYTES + 1);
        assert!(commands_of(&oversize).is_empty());
        let mut lines = vec!["{\"command\": \"comp.new\"}"; replay::MAX_LINES];
        lines.push("{\"command\": \"roto.model.download\"}");
        let found = commands_of(&lines.join("\n"));
        assert_eq!(found.len(), replay::MAX_LINES, "the line past the limit is never run, so never named");
    }
}
