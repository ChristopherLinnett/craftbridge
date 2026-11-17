//! The `replay` method: EffectCraft's own replay of a JSON-lines repro script, run on the confined
//! session (plan §14).
//!
//! `replay::run` executes each line on the session itself, so a line never passes the checks
//! `Backend::exec` makes — the confinement's, and the door that withholds the model downloads
//! (AD11). The method therefore reads the script's lines before running any of them and refuses the
//! whole call if either check refuses one of them, the way `run` checks its whole batch before its
//! first step: a refused line must not leave the earlier lines' work behind.

use craftbridge_core::{CraftError, Result};
use effectcraft_automation::Backend;
use effectcraft_engine::replay;
use serde_json::Value;

use crate::methods::{bool_param, check_unrun, empty_params, object_params, refuse_process, reject_unknown, string_param};

/// `replay`: `{script, keepGoing?}`, answering the report EffectCraft's replay makes — what the command
/// line prints with `--json`. A script whose steps fail is an answer, `ok: false` with the first
/// failure named, as it is the command line's report; a script a check refuses is an error, and runs
/// nothing.
pub(crate) fn replay(backend: &mut Backend, params: &Value) -> Result<Value> {
    let obj = object_params(params, "replay")?;
    reject_unknown(&obj, "replay", &["script", "keepGoing"])?;
    let script = string_param(&obj, "script", "replay")?.ok_or_else(|| CraftError::invalid_params("`replay`: needs a `script`"))?;
    let options = replay::Options { keep_going: bool_param(&obj, "keepGoing", "replay")?.unwrap_or(false), base_dir: None };
    for (line, command, step_params) in commands_of(script) {
        refuse_process(&command, &step_params, None).and_then(|()| check_unrun(backend, &command, &step_params)).map_err(|error| on_line(error, line))?;
    }
    let session = backend.session().ok_or_else(|| CraftError::internal("`replay`: the backend holds no session"))?;
    let report = replay::run(session, script, &options);
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
fn commands_of(script: &str) -> Vec<(usize, String, Value)> {
    if script.len() > replay::MAX_SCRIPT_BYTES {
        return Vec::new();
    }
    let script = script.strip_prefix('\u{feff}').unwrap_or(script);
    let mut found = Vec::new();
    for (index, text) in script.lines().enumerate().take(replay::MAX_LINES) {
        let text = text.trim();
        if text.is_empty() || text.starts_with("//") || text.starts_with('#') {
            continue;
        }
        let Ok(Value::Object(line)) = serde_json::from_str::<Value>(text) else { continue };
        let number = index.saturating_add(1);
        if let Some(command) = line.get("command").and_then(Value::as_str) {
            found.push((number, command.to_string(), params_of(line.get("params"))));
        }
        if let Some(expect) = line.get("expect")
            && let Some(query) = expect.get("query").and_then(Value::as_str)
        {
            found.push((number, query.to_string(), params_of(expect.get("params"))));
        }
        if let Some(path) = line.get("start").and_then(|start| start.get("project")).and_then(Value::as_str) {
            found.push((number, "file.open".to_string(), crate::methods::object([("path", Value::from(path))])));
        }
    }
    found
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
        let commands: Vec<(usize, String, Value)> = commands_of(&script);
        assert_eq!(
            commands,
            vec![
                (1, "file.open".to_string(), json!({"path": "/a/b.ecproj"})),
                (5, "comp.new".to_string(), json!({"name": "A"})),
                (6, "comp.info".to_string(), json!({"comp": 1})),
                (10, "layer.newSolid".to_string(), json!({})),
            ]
        );
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
