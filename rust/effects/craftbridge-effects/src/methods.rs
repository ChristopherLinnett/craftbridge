//! The `commands`, `exec` and `run` methods of the plan's §14 table, and the one mapping of every
//! EffectCraft failure onto a [`CraftError`] row.
//!
//! Each function mirrors what the EffectCraft command line does with the same input at the pin: the
//! same result shape, and the failure in the engine's own words, so the plugin and the command line
//! answer identically (the acceptance of plan §28 T1 to T3).

use std::borrow::Cow;
use std::path::Path;

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

/// `exec`: run one command through the door, which may hand it a copy in `folder`, the app's staged
/// folder, and answer its result.
pub(crate) fn exec(backend: &mut Backend, folder: &Path, params: &Value) -> Result<Value> {
    let obj = object_params(params, "exec")?;
    reject_unknown(&obj, "exec", &["command", "params"])?;
    let command = string_param(&obj, "command", "exec")?.ok_or_else(|| CraftError::invalid_params("`exec`: needs a `command`"))?;
    execute(backend, Some(folder), command, step_params(&obj), None)
}

/// `run`: run the steps in order through the door, as `exec` runs one, and answer
/// `[{command, result}]`, failing at the first step that fails with its index, as the command line
/// numbers them from zero.
pub(crate) fn run(backend: &mut Backend, folder: &Path, params: &Value) -> Result<Value> {
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
        let result = execute(backend, Some(folder), &command, step_params, u32::try_from(index).ok())?;
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

/// The door every command of `exec` and every step of `run` passes on its way to the session, the
/// render runner's own included: refuse here, before the engine sees the call, what [`refuse_process`]
/// withholds, hand a command that writes beside its source a copy of it to read ([`crate::imports`]),
/// then run the command, map its failure and pass a font listing through the platform's face list.
/// That filter sits on the value returned here, so `exec`, every `run` step and the file methods
/// answer through it, and nothing built over them can answer untrimmed. A render or a fill is
/// started only by its runner, through [`execute_listed`], once it has listed. A render's failed
/// items are not read here: a render started without waiting answers while its other items still
/// render, so the runner reads every item once the render has ended.
///
/// `folder` is the staged folder of the app whose command this is, where the door puts the copy a
/// command that writes beside its source reads. A command the app names carries it; the commands the
/// plugin's own methods and runners run carry `None`, since none of them is a command the door copies
/// for (a template's import reads the copy the template runner made before the call,
/// [`crate::templates::staged`]).
pub(crate) fn execute(backend: &mut Backend, folder: Option<&Path>, command: &str, params: Value, step: Option<u32>) -> Result<Value> {
    let mut params = if params.is_null() { empty_params() } else { params };
    refuse_process(command, &params, step)?;
    // A copy that cannot be made is a refusal of the disk, and a call the door cannot hand a copy is a
    // refusal of this build: either is worded as every other row's is in a step of a `run`.
    let staged = crate::imports::staged(backend, folder, &mut [(command, &mut params)]).map_err(|error| match error {
        CraftError::Io { message } => CraftError::io(said(command, message, step)),
        CraftError::Unsupported { message } => CraftError::unsupported(said(command, message, step)),
        other => other,
    })?;
    let answered = exec_on_engine(backend, command, &params, step);
    staged.finish(backend);
    answered.map(|value| crate::fonts::filtered(command, &params, value))
}

/// The runners' one way to start a command the door withholds ([`RENDER`] for the render runner,
/// [`FILL`] for the fill runner): the engine's command past the door's refusal, because the runner
/// has handed the journal every file the command may write first.
pub(crate) fn execute_listed(backend: &mut Backend, command: &str, params: Value) -> Result<Value> {
    exec_on_engine(backend, command, &params, None)
}

/// Run one command on the engine and map its failure.
fn exec_on_engine(backend: &mut Backend, command: &str, params: &Value, step: Option<u32>) -> Result<Value> {
    backend.exec(command, params.clone()).map_err(|error| map_error(error, &Call { command, params, step, tool: false }))
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
/// that reaches a model download, or a render or a fill nothing has listed, by whichever way, is refused as
/// `Unsupported`, and a step of `run` puts the step's command and index around the words, as it does
/// every other row's.
pub(crate) fn refuse_process(command: &str, params: &Value, step: Option<u32>) -> Result<()> {
    match withheld(command, params) {
        Some(words) => Err(CraftError::unsupported(said(command, words, step))),
        None => Ok(()),
    }
}

/// The words of the refusal of the first command one call reaches that the door withholds, if any:
/// the one reader every path asks, so each answers a withheld command in the same words.
pub(crate) fn withheld(command: &str, params: &Value) -> Option<String> {
    wants_a_process(command, params).map(starts_a_process).or_else(|| unlisted_write(command, params).map(starts_an_unlisted_write))
}

/// Whether the door withholds `command` whatever it is given, so that every path that hands an app's
/// call of it to the engine — `exec`, each step of `run`, each line of `replay`, each tool call of
/// `mcp`, and each step of an `engine.batch` in any of them — answers `Unsupported` before the engine
/// sees it: a model download, or a render or a fill nothing has listed, which only a runner starts,
/// past the door, once it has listed ([`execute_listed`]). The API surface marks each such command,
/// and neither facade offers a method for it, since every call of one could only be refused (plan
/// §21). It is the door's own answer, [`withheld`], for the command given nothing, less the one command
/// whose own parameters can let it through ([`read_by_its_parameters`]): no list is kept here beside
/// the door's.
pub fn withheld_outright(command: &str) -> bool {
    withheld(command, &Value::Null).is_some() && !read_by_its_parameters(command)
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

/// The command that starts a render, and names a render's failures.
pub(crate) const RENDER: &str = "renderQueue.render";

/// Every command that has the session's exporter write a render: those that start a render of the
/// queue — [`RENDER`] (`crates/engine/src/cmd/render_queue.rs`:718) and the watch folder's two, which
/// render the projects they find (`crates/engine/src/cmd/watch_folder.rs`:74, :138-155), as
/// `start_render` has no other caller — and the two that hand the exporter a job of their own: Save
/// Current Preview, which renders the work area straight to a movie (`crates/engine/src/cmd/render_queue.rs`:842-874), and
/// Export Timeline, which by default renders each layer a timeline cannot hold to a ProRes movie in a
/// staging folder of a random name (`crates/engine/src/cmd/timeline.rs`:113-155, :157-215,
/// `crates/engine/src/cmd/timeline/staging.rs`:17-36). `Exporter::export` has no other caller. The
/// pending-export journal learns a render's files only from the render runner, which lists them
/// before the render starts, so a call that reaches one of these any other way would write files no
/// restart could delete, and the door refuses it. Export Timeline is the one of these the call
/// itself can answer for: refused whole unless its own `prerender` reads as the mode that renders
/// nothing ([`writes_no_render`]), because whether any other Export Timeline pre-renders turns on
/// the project's layers, and what an `engine.batch` fills in from earlier steps as it runs is not
/// written until after the door has read the call.
const UNLISTED_RENDERS: [&str; 5] = [RENDER, "file.watchFolder", "file.watchFolder.poll", "render.saveCurrentPreview", EXPORT_TIMELINE];

/// Export Timeline, the one withheld render whose own parameters can say it renders nothing
/// ([`writes_no_render`]).
const EXPORT_TIMELINE: &str = "file.exportTimeline";

/// The command that generates a Content-Aware Fill: it writes a sequence of frames into a folder on a
/// background task (`crates/engine/src/cmd/content_fill.rs`:495-598 at the pin), outside the exporter,
/// so the pending-export journal learns the frames only from the fill runner, which lists them before
/// the task starts.
pub(crate) const FILL: &str = "contentFill.generate";

/// Every command that has a fill's frames written: [`FILL`] itself and Content-Aware Fill Layer, which
/// generates when it is given any parameter and otherwise only opens the panel
/// (`crates/engine/src/cmd/content_fill.rs`:659-665). A headless session has no panel to open, so the
/// door refuses it whatever it is given, and only the fill runner generates, after it has listed.
const UNLISTED_FILLS: [&str; 2] = [FILL, "layer.newContentAwareFill"];

/// The refusal of a command that writes files nothing has listed, worded as [`starts_a_process`] is,
/// and naming the method that lists: a render is listed by `export`, a fill by `contentFill.generate`
/// called with progress.
fn starts_an_unlisted_write(withheld: &str) -> String {
    if UNLISTED_FILLS.contains(&withheld) {
        return format!(
            "`{withheld}` is not available in this build: it generates the frames of a Content-Aware Fill, whose files the pending-export journal cannot list, and `{FILL}` called with progress lists them"
        );
    }
    format!("`{withheld}` is not available in this build: it starts a render whose files the pending-export journal cannot list, and `export` lists them")
}

/// The refusal of a withheld command, worded as the confinement words its own (`` `id` is not
/// available …``) in the engine's voice for a capability this build was put together without, so it
/// reads like every other refusal of its row; a step of `run` puts the step's command and index
/// around it, as it does every other row's words.
fn starts_a_process(withheld: &str) -> String {
    format!("`{withheld}` is not available in this build: it downloads by starting the system `curl`, and this build starts no process")
}

/// Every command one call reaches, each with the parameters it is reached with: the command itself,
/// then, for an `engine.batch`, each of its steps, whose ids the plug-in must read here because the
/// engine runs them itself and never brings them back through this door. It reads them as the engine
/// does — `command`, else `id` (`crates/engine/src/cmd/batch.rs`:77-81), and never as a `$N`, which
/// the engine substitutes only in the parameters (:85) — with each step's own `params`, and only one
/// level deep, because a `batch` inside a batch runs no steps at all: the engine refuses the nested
/// one before it runs (`NOT_IN_BATCH`, `crates/engine/src/cmd/batch.rs`:14,:82). It is the one reader
/// of the commands a call reaches, for every rule the door holds them to, the copies it hands the
/// commands that write beside their source among them ([`crate::imports`]), and each rule is asked of
/// every command it yields: a step a rule lets through must never stand in for the batch's later steps.
pub(crate) fn reached<'a>(command: &'a str, params: &'a Value) -> impl Iterator<Item = (&'a str, &'a Value)> {
    let steps = params.get("steps").and_then(Value::as_array).filter(|_| command == "engine.batch");
    std::iter::once((command, params)).chain(steps.into_iter().flatten().filter_map(|step| {
        let inner = step.get("command").or_else(|| step.get("id")).and_then(Value::as_str)?;
        Some((inner, step.get("params").unwrap_or(&Value::Null)))
    }))
}

/// The model download, if any, that one call reaches.
fn wants_a_process<'a>(command: &'a str, params: &'a Value) -> Option<&'a str> {
    reached(command, params).map(|(reached, _)| reached).find(|reached| NO_PROCESS.contains(reached))
}

/// The render or fill nothing has listed, if any, that one call reaches: the first command it reaches
/// that is one of [`UNLISTED_RENDERS`] or [`UNLISTED_FILLS`] and is not read, by its own parameters, to
/// write none.
fn unlisted_write<'a>(command: &'a str, params: &'a Value) -> Option<&'a str> {
    reached(command, params)
        .find(|(reached, given)| (UNLISTED_RENDERS.contains(reached) || UNLISTED_FILLS.contains(reached)) && !writes_no_render(reached, given))
        .map(|(reached, _)| reached)
}

/// The one withheld render a call can be read to start nothing: Export Timeline's `prerender`
/// parameter, which at the pin reads as the mode that renders nothing when it is the bool `false`
/// (`crates/engine/src/cmd/timeline.rs`:169-174; absent, `true` or any other kind reads as
/// `Unsupported`, the rendering default) or a spelling `PrerenderMode::parse` maps there —
/// `none`, `off`, `false` or `no`, after lower-casing and dropping spaces, underscores and hyphens
/// (`crates/interchange/src/export.rs`:32-40). In that mode no layer plans as a pre-render
/// (`crates/interchange/src/export.rs`:192-197 answers `Plan::Render` under no mode but `All` and
/// `Unsupported`), and only a plan of `Render` has Export Timeline's work call the exporter
/// (`crates/engine/src/cmd/timeline.rs`:232-249), so such a call renders nothing: it writes the one
/// file its own `path` names, where a call that does render also fills staging folders no listing
/// could name. The door may read the parameter as the engine will only where nothing changes it
/// between the two readings: `exec`'s and `run`'s parameters pass to the engine untouched, and a
/// literal in a batch step is what that step runs with; a `$N` is filled in as the batch runs, after
/// the door has read, so no reading of it is the door's, and it is refused. The spellings are read
/// from the pin, not invented here: a pin that parses `prerender` differently moves this reading.
fn writes_no_render(withheld: &str, given: &Value) -> bool {
    read_by_its_parameters(withheld)
        && match given.get("prerender") {
            Some(Value::Bool(false)) => true,
            Some(Value::String(spelling)) => matches!(spelling.to_ascii_lowercase().replace([' ', '_', '-'], "").as_str(), "none" | "off" | "false" | "no"),
            _ => false,
        }
}

/// Whether the door reads a withheld command's own parameters to let a call of it through: only
/// Export Timeline's ([`writes_no_render`]). Every other withheld command is refused whatever it is
/// given, which is what [`withheld_outright`] answers.
fn read_by_its_parameters(withheld: &str) -> bool {
    withheld == EXPORT_TIMELINE
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
/// for a comp, layer, property or control (the §14 `NotFound` row, a template's values naming a
/// control it lacks among them) rather than a command failing for some other reason ("unknown
/// effect" is a failed call, not a missing comp, layer or property). The
/// engine says these both as a command's own refusal and as a plain one: every command that takes a
/// list of layers refuses a missing one as "no layer …" outside its own refusal. A sentence counts
/// only when the missing thing is all it says or the reference it names follows — an id, a quoted
/// name, `#n`, `@uid` — because the same words open sentences about something else: "no layers",
/// "no layer uses motion blur", "no compatible unlocked property".
fn is_missing_named(message: &str) -> bool {
    const MISSING: [&str; 11] = [
        "no such property",
        "no such layer",
        "no such item",
        "no property group",
        "no property",
        "no layer",
        "no item",
        "no composition",
        "no comp",
        "no control named",
        "no control",
    ];
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
    use std::collections::BTreeSet;

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
            "no control named `Nope` (controls: Name, Photo)",
            "no control 74 on this instance",
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

    /// Every command that has the exporter write a render is withheld however a call reaches it — the
    /// queue's own, the watch folder's two, Save Current Preview and Export Timeline — and no other
    /// command is.
    #[test]
    fn every_command_that_starts_a_render_is_withheld_on_every_way_a_call_reaches_it() {
        for render in UNLISTED_RENDERS {
            assert_eq!(unlisted_write(render, &json!({})), Some(render), "{render} as its own command");
            assert_eq!(
                unlisted_write("engine.batch", &json!({"steps": [{"command": "comp.new"}, {"command": render}]})),
                Some(render),
                "{render} as a step of a batch"
            );
            assert_eq!(unlisted_write("engine.batch", &json!({"steps": [{"id": render}]})), Some(render), "{render} by `id` in a batch");
            assert_eq!(
                unlisted_write("engine.batch", &json!({"steps": [{"command": "engine.batch", "params": {"steps": [{"command": render}]}}]})),
                None,
                "a batch inside a batch runs no steps"
            );
        }
        assert_eq!(UNLISTED_RENDERS, ["renderQueue.render", "file.watchFolder", "file.watchFolder.poll", "render.saveCurrentPreview", "file.exportTimeline"]);
        assert_eq!(UNLISTED_FILLS, ["contentFill.generate", "layer.newContentAwareFill"]);
        // Pre-render and Create Proxy only queue an item, which `export` lists when it renders it.
        for other in ["renderQueue.add", "renderQueue.setRender", "renderQueue.stop", "renderQueue.list", "render.preRender", "file.createProxy", "file.save"] {
            assert_eq!(unlisted_write(other, &json!({})), None, "{other} starts no render");
        }
        assert_eq!(unlisted_write("comp.new", &json!({"name": "renderQueue.render"})), None, "a command only names a render when it runs one");
    }

    /// Both commands that can have a fill's frames written are withheld on every way a call reaches
    /// them, whatever parameters they are given: Content-Aware Fill Layer, which only opens a panel
    /// when it is given none, is refused too, since a headless session has no panel, and each is
    /// worded as a fill and names the call that lists, never `export`.
    #[test]
    fn every_command_that_generates_a_fill_is_withheld_whatever_it_is_given() {
        for fill in UNLISTED_FILLS {
            for given in [json!({}), json!({"layer": 1}), json!({"prerender": "none"}), Value::Null] {
                assert_eq!(unlisted_write(fill, &given), Some(fill), "{fill} given {given}");
                assert_eq!(unlisted_write("engine.batch", &json!({"steps": [{"command": "comp.new"}, {"command": fill, "params": given}]})), Some(fill));
            }
            assert_eq!(unlisted_write("engine.batch", &json!({"steps": [{"id": fill}]})), Some(fill), "{fill} by `id` in a batch");
            assert_eq!(
                unlisted_write("engine.batch", &json!({"steps": [{"command": "engine.batch", "params": {"steps": [{"command": fill}]}}]})),
                None,
                "a batch inside a batch runs no steps"
            );
            let quiet = json!({"command": "file.exportTimeline", "params": {"prerender": "none"}});
            assert_eq!(unlisted_write("engine.batch", &json!({"steps": [quiet, {"command": fill}]})), Some(fill), "{fill} after a quiet step");
            assert_eq!(
                withheld(fill, &json!({})),
                Some(format!(
                    "`{fill}` is not available in this build: it generates the frames of a Content-Aware Fill, whose files the pending-export journal cannot list, and `contentFill.generate` called with progress lists them"
                ))
            );
        }
        // The fill's settings change a panel's state and write nothing.
        for other in ["contentFill.set", "layer.addMask", "jobs.list", "jobs.cancel"] {
            assert_eq!(unlisted_write(other, &json!({})), None, "{other} generates no fill");
        }
        assert_eq!(unlisted_write("comp.new", &json!({"name": "contentFill.generate"})), None, "a command only names a fill when it runs one");
        for render in UNLISTED_RENDERS {
            let words = withheld(render, &json!({})).expect("a render is withheld");
            assert!(words.ends_with("and `export` lists them"), "{words}");
        }
    }

    /// Only Export Timeline's own no-render spellings are read past the refusal, exactly as the pin
    /// parses them: the bool `false`, and `none`, `off`, `false` and `no` however the engine's
    /// normalization folds them. Absent, a rendering spelling, and anything an `engine.batch` fills
    /// in later all stay refused; no other withheld command reads a parameter at all.
    #[test]
    fn only_an_export_timeline_that_says_it_renders_nothing_is_read_past_the_refusal() {
        for say in ["none", "None", " OFF ", "no", "false", "FaLsE", "-off-", "_n_o_"] {
            assert!(writes_no_render("file.exportTimeline", &json!({"prerender": say})), "`{say}` reads as none");
        }
        assert!(writes_no_render("file.exportTimeline", &json!({"prerender": false})), "the bool `false` reads as none");
        for say in ["", "all", "unsupported", "auto", "yes", "true", "nonelayer", "nope", "$0.prerender"] {
            assert!(!writes_no_render("file.exportTimeline", &json!({"prerender": say})), "`{say}` may render");
        }
        for say in [Value::Null, json!(true), json!(0), json!([])] {
            assert!(!writes_no_render("file.exportTimeline", &json!({"prerender": say})), "{say} may render or is refused by the engine");
        }
        assert!(!writes_no_render("file.exportTimeline", &json!({})), "absent `prerender` is the rendering default");
        for withheld in UNLISTED_RENDERS {
            if withheld != "file.exportTimeline" {
                assert!(!writes_no_render(withheld, &json!({"prerender": "none"})), "`{withheld}` renders whatever a `prerender` says");
            }
        }
    }

    /// The door asks its rule of every command a call reaches, each with its own parameters, so an
    /// Export Timeline it reads past never lets a later step of the batch through, whichever render
    /// that step starts, and a step is read by its own `prerender`, never by the batch's.
    #[test]
    fn a_step_read_past_the_refusal_hides_no_later_step() {
        let quiet = json!({"command": "file.exportTimeline", "params": {"prerender": "none"}});
        for render in UNLISTED_RENDERS {
            assert_eq!(unlisted_write("engine.batch", &json!({"steps": [quiet, {"command": render}]})), Some(render), "{render} after a quiet step");
            assert_eq!(unlisted_write("engine.batch", &json!({"steps": [quiet, {"id": render}]})), Some(render), "{render} by `id` after a quiet step");
        }
        assert_eq!(unlisted_write("engine.batch", &json!({"steps": [quiet, quiet]})), None, "quiet steps alone pass");
        assert_eq!(unlisted_write("file.exportTimeline", &json!({"prerender": "off"})), None, "a quiet call passes");
        assert_eq!(
            unlisted_write("engine.batch", &json!({"prerender": "none", "steps": [{"command": "file.exportTimeline"}]})),
            Some("file.exportTimeline"),
            "a step is read by its own parameters"
        );
        assert_eq!(
            wants_a_process("engine.batch", &json!({"steps": [quiet, {"command": "comp.new"}, {"id": "roto.model.download"}]})),
            Some("roto.model.download"),
            "every step is read for a download too"
        );
    }

    /// What the door refuses whatever it is given, and so what the API surface marks, is exactly the
    /// two model downloads, the renders nothing lists but Export Timeline, which a call saying it
    /// renders nothing passes, and the two fills: asked of every command of the pinned registry, so a
    /// command the door comes to withhold, or one it lets through, moves this list. A batch is no such
    /// command, since the door refuses it only for the steps it is given.
    #[test]
    fn only_what_no_call_gets_past_the_door_is_withheld_outright() {
        let outright: BTreeSet<&str> = effectcraft_engine::command_specs().iter().map(|spec| spec.id).filter(|id| withheld_outright(id)).collect();
        let expected = [
            "contentFill.generate",
            "face.model.download",
            "file.watchFolder",
            "file.watchFolder.poll",
            "layer.newContentAwareFill",
            "render.saveCurrentPreview",
            "renderQueue.render",
            "roto.model.download",
        ];
        assert_eq!(outright, BTreeSet::from(expected));
        for id in expected {
            for given in [Value::Null, json!({}), json!({"prerender": "none"}), json!({"prerender": false})] {
                assert!(withheld(id, &given).is_some(), "`{id}` given {given} passes the door");
            }
        }
        assert!(withheld("file.exportTimeline", &json!({})).is_some(), "an Export Timeline that may render is withheld");
        assert_eq!(withheld("file.exportTimeline", &json!({"prerender": "none"})), None, "one that renders nothing passes");
        assert!(withheld("engine.batch", &json!({"steps": [{"command": "renderQueue.render"}]})).is_some() && !withheld_outright("engine.batch"));
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
