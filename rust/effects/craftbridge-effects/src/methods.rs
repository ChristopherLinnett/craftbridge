//! The `commands`, `exec` and `run` methods of the plan's §14 table, and the one mapping of every
//! EffectCraft failure onto a [`CraftError`] row.
//!
//! Each function mirrors what the EffectCraft command line does with the same input at the pin: the
//! same result shape, and the failure in the engine's own words, so the plugin and the command line
//! answer identically (the acceptance of plan §28 T1 to T3).

use std::borrow::Cow;

use craftbridge_core::{CraftError, Result};
use effectcraft_automation::tools::{self, Reply};
use effectcraft_automation::{Backend, Error};
use effectcraft_engine::commands::check_params;
use effectcraft_engine::project::ProjectError;
use effectcraft_engine::{EngineError, find_command};
use serde_json::Value;
use serde_json::map::Map;

/// `commands`: the command catalogue, filtered the way the tool filters it.
pub(crate) fn commands(backend: &mut Backend, params: &Value) -> Result<Value> {
    let obj = object_params(params, "commands")?;
    reject_unknown(&obj, "commands", &["filter", "enabledOnly", "schemas"])?;
    let filter = string_param(&obj, "filter", "commands")?;
    let enabled_only = bool_param(&obj, "enabledOnly", "commands")?.unwrap_or(false);
    let schemas = bool_param(&obj, "schemas", "commands")?.unwrap_or(false);
    let arguments =
        object([("filter", filter.map_or(Value::Null, Value::from)), ("enabled_only", Value::Bool(enabled_only)), ("schemas", Value::Bool(schemas))]);
    tool_json(backend, "list_commands", &arguments)
}

/// `exec`: run one command and answer its result, reporting a render whose items failed as the
/// command line reports them.
pub(crate) fn exec(backend: &mut Backend, params: &Value) -> Result<Value> {
    let obj = object_params(params, "exec")?;
    reject_unknown(&obj, "exec", &["command", "params"])?;
    let command = string_param(&obj, "command", "exec")?.ok_or_else(|| CraftError::invalid_params("`exec`: needs a `command`"))?;
    run_command(backend, command, step_params(&obj), None)
}

/// `run`: run the steps in order and answer `[{command, result}]`, failing at the first step that
/// fails with its index, as the command line numbers them from zero.
pub(crate) fn run(backend: &mut Backend, params: &Value) -> Result<Value> {
    let obj = object_params(params, "run")?;
    reject_unknown(&obj, "run", &["steps"])?;
    let steps = obj.get("steps").and_then(Value::as_array).ok_or_else(|| CraftError::invalid_params("`run`: needs a `steps` array"))?;
    if steps.is_empty() {
        return Err(CraftError::invalid_params("`run`: `steps` names no steps"));
    }
    // The whole batch is checked before the first step runs, as the command line parses its whole
    // command line first: a malformed later step must not leave an earlier step's work behind.
    let mut parsed = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        let what = format!("step {index} of `run`");
        let obj = object_params(step, &what)?;
        reject_unknown(&obj, &what, &["command", "params"])?;
        let command = string_param(&obj, "command", &what)?.ok_or_else(|| CraftError::invalid_params(format!("`{what}`: needs a `command`")))?;
        parsed.push((command.to_string(), step_params(&obj)));
    }
    let mut results = Vec::with_capacity(parsed.len());
    for (index, (command, step_params)) in parsed.into_iter().enumerate() {
        let result = run_command(backend, &command, step_params, u32::try_from(index).ok())?;
        results.push(object([("command", Value::String(command)), ("result", result)]));
    }
    Ok(Value::Array(results))
}

/// One call to EffectCraft, as the mapping needs to know it: what was run, with what parameters,
/// and the step of a `run` that was asking, if one was.
struct Call<'a> {
    command: &'a str,
    params: &'a Value,
    step: Option<u32>,
    /// Whether `command` names an automation tool rather than a command: a tool runs commands of its
    /// own with parameters it builds, so a refusal can name any of them, and the parameters the
    /// caller wrote are not the ones that command was given.
    tool: bool,
}

/// The one entry every command of `exec` and every step of `run` passes on its way to the session:
/// the door, then the report of a render whose items failed. `step` names the step of a `run` for
/// the rows that carry one.
pub(crate) fn run_command(backend: &mut Backend, command: &str, params: Value, step: Option<u32>) -> Result<Value> {
    let value = execute(backend, command, params, step)?;
    match failed_render(command, &value) {
        Some(failure) => Err(CraftError::command_failed(command, failure, step)),
        None => Ok(value),
    }
}

/// The door every command goes through, the render runner's own included: refuse here, before the
/// engine sees the call, the commands that would start a process — a phone starts none — and then
/// run the command and map its failure. A render's failed items are not read here: a render started
/// without waiting answers while its other items still render, so the render runner reads every
/// item once the render has ended.
pub(crate) fn execute(backend: &mut Backend, command: &str, params: Value, step: Option<u32>) -> Result<Value> {
    let params = if params.is_null() { empty_params() } else { params };
    refuse_process(command, &params, step)?;
    backend.exec(command, params.clone()).map_err(|error| map_error(error, &Call { command, params: &params, step, tool: false }))
}

/// The model downloads fetch their weights by starting the system's `curl`
/// (`crates/engine/src/analysis/models.rs`:639-660 at the pin, which is why the engine's own command
/// specs mark both commands "desktop; uses the system curl":718,:747). A phone has no `curl` to
/// start, and a phone's session starts no process (plan §29); so the plug-in withholds both ids at
/// its own door, as the confinement withholds its own commands (`crates/automation/src/confine.rs`:285).
/// Install from File — `roto.model.install` and `face.model.install` reading a file inside the
/// container — is the phone's way to get weights, and stays open.
const NO_PROCESS: [&str; 2] = ["roto.model.download", "face.model.download"];

/// The door itself, which every path that hands a command to the engine passes before the engine
/// sees it — `exec`, each step of `run`, each line of a `replay` and each tool call of `mcp`: a call
/// that reaches a model download, by whichever way, is refused as `Unsupported`, and a step of `run`
/// puts the step's command and index around the words, as it does every other row's.
pub(crate) fn refuse_process(command: &str, params: &Value, step: Option<u32>) -> Result<()> {
    match wants_a_process(command, params) {
        Some(withheld) => Err(CraftError::unsupported(said(command, starts_a_process(withheld), step))),
        None => Ok(()),
    }
}

/// What the confinement refuses of a command that is to run later, asked without running it, for
/// the paths that run commands past [`Backend::exec`] and so past the checks it makes (`replay`
/// runs its lines on the session itself). The confinement already checks commands that run later
/// than its check: the steps of an `engine.batch` (`crates/automation/src/confine.rs`:297-318). So the
/// command is checked as the one step of a batch, in the confinement's own rule and words: as
/// `Backend::exec` checks it, and a command whose destination only a live lookup can tell — Save,
/// Increment and Save, rendering the queue — refused, since the lookup would be of a document the
/// earlier lines have not yet made.
pub(crate) fn check_unrun(backend: &Backend, command: &str, params: &Value) -> Result<()> {
    let Some(confinement) = backend.confinement() else { return Ok(()) };
    let as_a_step = object([("steps", Value::Array(vec![object([("command", Value::from(command)), ("params", params.clone())])]))]);
    match confinement.check_command("engine.batch", &as_a_step) {
        Ok(_) => Ok(()),
        Err(error) => Err(failure(error, command, params, false)),
    }
}

/// The refusal of a withheld command, worded as the confinement words its own (`` `id` is not
/// available …``) in the engine's voice for a capability this build was put together without, so it
/// reads like every other refusal of its row; a step of `run` puts the step's command and index
/// around it, as it does every other row's words.
pub(crate) fn starts_a_process(withheld: &str) -> String {
    format!("`{withheld}` is not available in this build: it downloads by starting the system `curl`, and this build starts no process")
}

/// Which model download, if any, one call reaches. The command itself may be one; an `engine.batch`
/// reaches one through its steps, whose ids the plug-in must read here because the engine runs them
/// itself and never brings them back through this door. It reads them as the engine does — `command`,
/// else `id` (`crates/engine/src/cmd/batch.rs`:77-81), and never as a `$N`, which the engine
/// substitutes only in the parameters (:85) — and only one level deep, because a `batch` inside a
/// batch runs no steps at all: the engine refuses the nested one before it runs (`NOT_IN_BATCH`,
/// `crates/engine/src/cmd/batch.rs`:14,:82).
pub(crate) fn wants_a_process<'a>(command: &'a str, params: &'a Value) -> Option<&'a str> {
    if NO_PROCESS.contains(&command) {
        return Some(command);
    }
    if command != "engine.batch" {
        return None;
    }
    params
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|step| step.get("command").or_else(|| step.get("id")).and_then(Value::as_str))
        .find(|inner| NO_PROCESS.contains(inner))
}

/// A `renderQueue.render` that succeeded but carries failed items is a failure, in the command
/// line's own words: every `compName: reason` joined, so the app sees which item broke the render,
/// with the command line's `?` for an item that names no comp and `failed` for a reason that is no
/// sentence, so no failed item is ever dropped from the report.
fn failed_render(command: &str, result: &Value) -> Option<String> {
    if command != "renderQueue.render" {
        return None;
    }
    let failed: Vec<String> = result
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let reason = item.get("status")?.get("Failed")?;
            let comp = item.get("compName").and_then(Value::as_str).unwrap_or("?");
            Some(format!("{comp}: {}", reason.as_str().unwrap_or("failed")))
        })
        .collect();
    (!failed.is_empty()).then(|| failed.join("; "))
}

/// Run one EffectCraft automation tool and answer its document. Every §14 row whose result is a
/// tool's output goes through here, so the tool's refusals reach the same mapping as a command's.
pub(crate) fn tool_json(backend: &mut Backend, name: &str, args: &Value) -> Result<Value> {
    match tools::run(backend, name, args) {
        Ok(Reply::Json(value)) => Ok(value),
        // None of the tools reached here answers a picture; an answer that is one says the plugin
        // called the wrong tool, which is the plugin's own bug, not the app's error.
        Ok(Reply::Image { .. }) => Err(CraftError::internal(format!("`{name}`: answered a picture where a document was expected"))),
        Err(error) => Err(failure(error, name, args, true)),
    }
}

/// Map the failure of one EffectCraft automation call that is no command of a `run`: the tools, and
/// the calls the plugin makes on the backend itself, such as a frame's render. `tool` says whether
/// `command` names an automation tool, whose refusals are told by their words (see [`Call::tool`]).
pub(crate) fn failure(error: Error, command: &str, params: &Value, tool: bool) -> CraftError {
    map_error(error, &Call { command, params, step: None, tool })
}

/// Map one EffectCraft automation failure onto its row (plan §14).
fn map_error(error: Error, call: &Call) -> CraftError {
    match error {
        Error::BadArgs(message) => map_denial(message, call),
        Error::Engine(inner) => map_engine(inner, call),
        // A headless session talks to no desktop app; these are the automation layer failing the
        // call itself, which is its own row: an internal failure with the layer's words.
        Error::Bridge(_) | Error::App(_) | Error::Other(_) => CraftError::internal(said(call.command, error.to_string(), call.step)),
    }
}

/// Sort a refusal of the automation layer, which in a confined session is mostly the confinement's.
/// It refuses in four voices: a command or setting it withholds while confined (a helper, a script, a
/// plug-in installer — capabilities this platform does not have) is `Unsupported`; a comp its render
/// finds nowhere, or finds none open, is `NotFound`; a file access outside the roots, or a write with
/// nowhere inside them to go, is `Io`; and a parameter it cannot check (a path that is not a string,
/// a reference resolved too late, a path not given) is the caller's to fix, `InvalidParams`, which is
/// also what the layer's own argument refusals are.
fn map_denial(message: String, call: &Call) -> CraftError {
    let row: fn(String) -> CraftError =
        if message.contains("is not available while file access is confined") || message.contains("can't be changed while file access is confined") {
            CraftError::unsupported
        } else if is_missing_named(&message) || message.starts_with("no active composition") {
            // The automation layer's own words for a comp that is not there: a render of a comp
            // named nowhere, or with none open, is the §14 row of a missing comp.
            CraftError::not_found
        } else if message.starts_with("cannot ") || message.starts_with("the project has no file yet") {
            CraftError::io
        } else {
            CraftError::invalid_params
        };
    row(said(call.command, message, call.step))
}

/// Sort an engine error onto its row (plan §14). The engine answers `Err` in a handful of shapes
/// and a command's own refusal shares its variant with the parameter check's, so the check itself
/// is re-run to tell them apart: EffectCraft's own `check_params` is one mechanism for both. Only a
/// refusal naming the command that was run is re-checked: one a command passes on from a command it
/// ran itself was given parameters the caller never wrote, so it is that command's failure. A tool's
/// call is the exception: the commands it ran are told by their words alone, since the arguments
/// the caller wrote are the tool's, not theirs.
fn map_engine(error: EngineError, call: &Call) -> CraftError {
    let Call { command, params, step, tool } = *call;
    match &error {
        EngineError::UnknownCommand(_) => CraftError::invalid_params(said(command, error.to_string(), step)),
        EngineError::NoComp
        | EngineError::NoSuchComp(_)
        | EngineError::Project(ProjectError::NoItem(_) | ProjectError::NotComp(_) | ProjectError::NoLayer(_) | ProjectError::NoProperty(_)) => {
            CraftError::not_found(said(command, error.to_string(), step))
        }
        EngineError::BadParams { cmd, msg } if cmd == command || tool => {
            if lacks_capability(msg) {
                CraftError::unsupported(named(cmd, msg, step))
            } else if !tool && find_command(cmd).is_some_and(|spec| check_params(spec, params).is_err()) {
                CraftError::invalid_params(named(cmd, msg, step))
            } else if is_missing_named(msg) {
                CraftError::not_found(named(cmd, msg, step))
            } else {
                CraftError::command_failed(cmd.clone(), msg.clone(), step)
            }
        }
        EngineError::Other(why) | EngineError::Disabled(_, why) if lacks_capability(why) => CraftError::unsupported(said(command, error.to_string(), step)),
        EngineError::Other(why) if is_refused_by_the_disk(why) => CraftError::io(said(command, error.to_string(), step)),
        EngineError::Other(why) if is_missing_named(why) => CraftError::not_found(said(command, error.to_string(), step)),
        _ => CraftError::command_failed(command, error.to_string(), step),
    }
}

/// Whether the engine refused for a capability this build was put together without: the helper
/// hooks, the WebAssembly loader, scripting, import, export, the Media Browser — every such sentence
/// of the engine's ends the same way ("… is not available in this build", "… are not …") — or an
/// export format the session's exporter does not write, which the engine words "{format} export is
/// not available", both as a command's refusal and as the reason a render item failed.
pub(crate) fn lacks_capability(why: &str) -> bool {
    why.ends_with("not available in this build") || why.ends_with(" export is not available")
}

/// Whether the engine failed because the disk would not read or write a file: a file or folder
/// missing, unreadable or unwritable, a disk full (the §14 `Io` row, as every such failure of
/// CraftBridge's own is). The engine carries no kind for a disk refusal, only its words, and
/// reports a file it could not read or write as `cannot read {file}: {why}` or `cannot write
/// {file}: {why}`, with the system's reason after, which is every disk refusal `open`, `save`,
/// `saveAs` and `new` can meet.
///
/// A file that read but holds no project ("… is not a project EffectCraft can open") is the
/// command's own failure. The engine's few disk refusals worded otherwise stay that row too: a
/// parameter refusal (`BadParams`) is sorted before this, and a wider match on `cannot` would take
/// in its refusals of what a file holds ("cannot import …").
fn is_refused_by_the_disk(why: &str) -> bool {
    why.starts_with("cannot read ") || why.starts_with("cannot write ")
}

/// Whether a command's refusal is one of the engine's "the thing you named is not there" sentences
/// for a comp, layer or property (the §14 `NotFound` row) rather than a command failing for some
/// other reason ("unknown effect" is a failed call, not a missing comp, layer or property). The
/// engine says these both as a command's own refusal and as a plain one: every command that takes a
/// list of layers refuses a missing one as "no layer …" outside its own refusal. A sentence counts
/// only when the missing thing is all it says or the reference it names follows — an id, a quoted
/// name, `#n`, `@uid` — because the same words open sentences about something else: "no layers",
/// "no layer uses motion blur", "no compatible unlocked property".
fn is_missing_named(message: &str) -> bool {
    const MISSING: [&str; 9] =
        ["no such property", "no such layer", "no such item", "no property group", "no property", "no layer", "no item", "no composition", "no comp"];
    let names_a_reference = |rest: &str| rest.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '"' | '`' | '#' | '@' | '-'));
    MISSING.iter().any(|missing| {
        message.strip_prefix(missing).is_some_and(|rest| rest.is_empty() || rest == " is open" || rest.strip_prefix(' ').is_some_and(names_a_reference))
    })
}

/// The words of a failure the engine phrased without the command: as the engine said them for one
/// command, and for a step of a `run` with the step's command before them and its index after, as
/// the command line's `run` reports every failed step, so the app can tell which step failed
/// whichever row the failure maps onto.
fn said(command: &str, text: String, step: Option<u32>) -> String {
    if step.is_some() { named(command, &text, step) } else { text }
}

/// The words of a failure that names its command, `command: text`, with the step of a `run` after.
fn named(command: &str, text: &str, step: Option<u32>) -> String {
    match step {
        Some(step) => format!("{command}: {text} (step {step})"),
        None => format!("{command}: {text}"),
    }
}

/// The parameters a method was called with, as an object: absent parameters are an empty object,
/// anything that is not one is refused with the method named.
pub(crate) fn object_params<'a>(params: &'a Value, method: &str) -> Result<Cow<'a, Map<String, Value>>> {
    match params {
        Value::Object(obj) => Ok(Cow::Borrowed(obj)),
        Value::Null => Ok(Cow::Owned(Map::new())),
        _ => Err(CraftError::invalid_params(format!("`{method}`: its parameters must be a JSON object"))),
    }
}

pub(crate) fn reject_unknown(obj: &Map<String, Value>, method: &str, known: &[&str]) -> Result<()> {
    let unknown: Vec<&str> = obj.keys().filter(|key| !known.contains(&key.as_str())).map(String::as_str).collect();
    if unknown.is_empty() { Ok(()) } else { Err(CraftError::invalid_params(format!("`{method}`: unknown parameter(s) `{}`", unknown.join("`, `")))) }
}

pub(crate) fn string_param<'a>(obj: &'a Map<String, Value>, key: &str, method: &str) -> Result<Option<&'a str>> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(CraftError::invalid_params(format!("`{method}`: `{key}` must be a string"))),
    }
}

pub(crate) fn bool_param(obj: &Map<String, Value>, key: &str, method: &str) -> Result<Option<bool>> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(flag)) => Ok(Some(*flag)),
        Some(_) => Err(CraftError::invalid_params(format!("`{method}`: `{key}` must be a bool"))),
    }
}

/// A numeric parameter, as the JSON number it is: absent and null mean not given.
pub(crate) fn number_param(obj: &Map<String, Value>, key: &str, method: &str) -> Result<Option<f64>> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number.as_f64().map(Some).ok_or_else(|| CraftError::invalid_params(format!("`{method}`: `{key}` must be a number"))),
        Some(_) => Err(CraftError::invalid_params(format!("`{method}`: `{key}` must be a number"))),
    }
}

/// The `params` of an `exec` or one `run` step: absent means none, `{}`, as the command line defaults it.
fn step_params(obj: &Map<String, Value>) -> Value {
    obj.get("params").cloned().unwrap_or_else(empty_params)
}

/// A JSON object from key-value pairs, which it takes over rather than copies, since a value may be
/// a whole project document. The library builds its documents this way rather than with the `json!`
/// macro, because the macro expands to `unwrap` calls and library code never carries one (AGENTS
/// §3); the tests may still use the macro.
pub(crate) fn object<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::Object(pairs.into_iter().map(|(key, value)| (key.to_string(), value)).collect())
}

/// The empty parameters document, for tools that take none.
pub(crate) fn empty_params() -> Value {
    Value::Object(Map::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The command line's own words for each shape of failed item (`apps/effectcraft-cli/src/main.rs`
    /// `failed_render`): a missing comp name reads `?`, a reason that is no sentence reads `failed`,
    /// and an item that did not fail is left out.
    #[test]
    fn a_failed_render_names_every_failed_item_as_the_command_line_does() {
        let result = json!({"items": [
            {"compName": "Card", "status": {"Failed": "i/o: Is a directory"}},
            {"compName": "Done", "status": "Done"},
            {"status": {"Failed": "no output"}},
            {"compName": "Odd", "status": {"Failed": 7}},
        ]});
        assert_eq!(failed_render("renderQueue.render", &result).as_deref(), Some("Card: i/o: Is a directory; ?: no output; Odd: failed"));
        assert_eq!(failed_render("renderQueue.render", &json!({"items": [{"compName": "Done", "status": "Done"}]})), None);
        assert_eq!(failed_render("comp.new", &result), None, "only a render reports its items");
    }

    /// The engine's own sentences, as its commands word them: each that names a missing comp, layer,
    /// item or property, and each that opens with the same words about something else.
    #[test]
    fn a_missing_thing_is_told_from_a_sentence_that_opens_alike() {
        for missing in [
            "no layer 99 (use a layer id, \"#n\" or a name from comp.info)",
            "no layer \"Title\"",
            "no layer",
            "no such layer",
            "no such item",
            "no such property `x`",
            "no item `Card`",
            "no item 7",
            "no composition \"nope\"",
            "no composition is open",
            "no property `transform/opacity`",
            "no property group @12",
        ] {
            assert!(is_missing_named(missing), "`{missing}` names a missing thing");
        }
        for other in
            ["no layers", "no layers with bounds to align", "no layer uses motion blur", "no layer to track", "no compatible unlocked property to paste into"]
        {
            assert!(!is_missing_named(other), "`{other}` names nothing missing");
        }
    }

    fn call<'a>(command: &'a str, params: &'a Value) -> Call<'a> {
        Call { command, params, step: None, tool: false }
    }

    /// Each of the engine's ways of saying the build lacks a capability is the `Unsupported` row:
    /// as a plain refusal, as a command's own refusal, and as the reason a command is disabled.
    #[test]
    fn every_capability_the_build_lacks_answers_unsupported() {
        let params = json!({});
        for error in [
            EngineError::Other("media import is not available in this build".into()),
            EngineError::Other("WebAssembly effect plug-ins are not available in this build".into()),
            EngineError::BadParams { cmd: "footage.relink".into(), msg: "media import is not available in this build".into() },
            EngineError::Disabled("footage.relink".into(), "effect helpers are not available in this build".into()),
            // `render.saveCurrentPreview`'s refusal of a format the session's exporter does not write.
            EngineError::Other("Animated GIF export is not available".into()),
        ] {
            let mapped = map_engine(error, &call("footage.relink", &params));
            assert!(matches!(mapped, CraftError::Unsupported { .. }), "a missing capability is `Unsupported`, got {mapped:?}");
        }
        // A sentence that only mentions an export is not one.
        assert!(!lacks_capability("the export is not available until the comp has a duration, try again"));
    }

    /// The engine's sentence for a file the disk would not read or write is the `Io` row, in the
    /// engine's words with a `run` step's around them: a full disk, an unwritable folder, a file that
    /// is not there. A file that read but holds no project, and a refusal worded alike as a
    /// parameter's, are the command's own failure.
    #[test]
    fn a_file_the_disk_refuses_answers_io() {
        let params = json!({});
        for (command, why) in [
            ("file.save", "cannot write /c/doc.ecproj: No space left on device (os error 28)"),
            ("file.saveAs", "cannot write /c/locked/doc.ecproj: Permission denied (os error 13)"),
            ("file.open", "cannot read /c/missing.ecproj: No such file or directory (os error 2)"),
            ("file.newProject", "cannot read the new project template /c/t.ecproj: No such file or directory (os error 2)"),
        ] {
            match map_engine(EngineError::Other(why.into()), &call(command, &params)) {
                CraftError::Io { message } => assert_eq!(message, why, "the refusal keeps the engine's words"),
                other => panic!("`{why}` is `Io`, got {other:?}"),
            }
        }
        let stepped = Call { command: "file.save", params: &params, step: Some(2), tool: false };
        match map_engine(EngineError::Other("cannot write /c/doc.ecproj: Disk quota exceeded (os error 69)".into()), &stepped) {
            CraftError::Io { message } => assert_eq!(message, "file.save: cannot write /c/doc.ecproj: Disk quota exceeded (os error 69) (step 2)"),
            other => panic!("a refused write in a step is `Io` naming the step, got {other:?}"),
        }
        let not_a_project = EngineError::Other("/c/junk.ecproj is not a project EffectCraft can open: expected value".into());
        assert!(matches!(map_engine(not_a_project, &call("file.open", &params)), CraftError::CommandFailed { .. }), "a file that read is no disk refusal");
        let svg = EngineError::BadParams { cmd: "edit.pasteSvg".into(), msg: "cannot read the SVG: unexpected end".into() };
        assert!(matches!(map_engine(svg, &call("edit.pasteSvg", &params)), CraftError::CommandFailed { .. }), "a parameter refused is no disk refusal");
    }

    /// Both model downloads are withheld however a call reaches them and nothing else is: a step's
    /// command is read as the engine reads it — `command`, else `id` — and a batch inside a batch
    /// runs no steps at all, so its inner steps reach nothing.
    #[test]
    fn the_model_downloads_are_withheld_on_every_way_a_call_reaches_them() {
        let weights = json!({"id": "classical"});
        assert_eq!(wants_a_process("roto.model.download", &weights), Some("roto.model.download"));
        assert_eq!(wants_a_process("face.model.download", &weights), Some("face.model.download"));
        assert_eq!(wants_a_process("roto.model.install", &json!({"path": "weights.safetensors"})), None, "installing from a file starts no process");
        assert_eq!(wants_a_process("comp.new", &json!({"name": "roto.model.download"})), None, "a command only names a process when it runs one");
        assert_eq!(
            wants_a_process("engine.batch", &json!({"steps": [{"command": "comp.new", "params": {}}, {"command": "roto.model.download", "params": weights}]})),
            Some("roto.model.download"),
            "a step of a batch reaches the door through the batch"
        );
        assert_eq!(
            wants_a_process("engine.batch", &json!({"steps": [{"id": "face.model.download"}]})),
            Some("face.model.download"),
            "the engine reads a step by `id` where it names no `command`"
        );
        assert_eq!(
            wants_a_process("engine.batch", &json!({"steps": [{"command": "engine.batch", "params": {"steps": [{"command": "roto.model.download"}]}}]})),
            None,
            "the engine refuses a nested batch before it runs, so its steps reach nothing"
        );
        assert_eq!(wants_a_process("engine.batch", &json!({"steps": [{"command": "comp.new"}, {"params": {}}]})), None);
    }

    /// A tool runs commands of its own with parameters it builds, so a refusal naming one of them is
    /// told by its words, and the arguments the caller wrote are never checked against that command's:
    /// `flat` is the tool's own and the command it ran was never given it.
    #[test]
    fn a_refusal_of_a_command_a_tool_ran_is_told_by_its_words() {
        let arguments = json!({"layer": 5, "flat": true});
        let tool = Call { command: "get_layer", params: &arguments, step: None, tool: true };
        let refusal = |msg: &str| EngineError::BadParams { cmd: "layer.tree".into(), msg: msg.into() };
        assert!(matches!(map_engine(refusal("no layer 5"), &tool), CraftError::NotFound { .. }), "a missing layer is `NotFound`");
        assert!(matches!(map_engine(refusal("media import is not available in this build"), &tool), CraftError::Unsupported { .. }));
        match map_engine(refusal("can't use \"abc\" for a scalar property"), &tool) {
            CraftError::CommandFailed { command, message, step } => {
                assert_eq!((command.as_str(), message.as_str(), step), ("layer.tree", "can't use \"abc\" for a scalar property", None));
            }
            other => panic!("another refusal is the command's failure, got {other:?}"),
        }
    }

    /// A refusal a command passes on from a command it ran itself is that command's failure: the
    /// parameters the inner command refused are not the ones the caller wrote, so they are never
    /// checked against the caller's.
    #[test]
    fn a_refusal_passed_on_from_an_inner_command_is_the_outer_commands_failure() {
        let params = json!({"layer": 1, "track": true});
        let inner = EngineError::BadParams { cmd: "effect.apply".into(), msg: "unknown effect `Warp Stabilizer`".into() };
        match map_engine(inner, &call("warp.stabilize", &params)) {
            CraftError::CommandFailed { command, message, step } => {
                assert_eq!(command, "warp.stabilize", "the failure names the command the caller ran");
                assert_eq!(message, "invalid parameters for `effect.apply`: unknown effect `Warp Stabilizer`", "the engine's words name the inner command");
                assert_eq!(step, None);
            }
            other => panic!("an inner command's refusal is the outer command's failure, got {other:?}"),
        }
    }
}
