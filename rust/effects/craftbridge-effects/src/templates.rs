//! The template runner (AD5): what the command line's `--template` does to a session, written on
//! EffectCraft's public commands and `Session` calls, and the three calls that use it.
//!
//! A template is set up the way the command line sets one up: its values are checked against a
//! scratch copy of the template first, so a bad value is refused before the import extracts any
//! media; the template is imported without being added to a comp; and a wrapper composition, a
//! duplicate of the template's own emptied of layers, holds one instance of it with the values set
//! on that instance. The wrapper is the active comp afterwards, so the render that follows takes it
//! as it takes the active comp of any session.
//!
//! * `template.open` sets the instance up and answers `{comp, warnings}`;
//! * `template.render` (through `request_with_progress`) sets it up and exports it through the render
//!   runner, so the files it may write are listed in the pending-export journal before any is
//!   written;
//! * the binary `frame` call sets it up and renders the frame of the instance, when its parameters
//!   name a `template`.
//!
//! The instance stays in the session afterwards, as it stays after a command line run, until the
//! project is replaced.
//!
//! The import extracts a template's media beside the template, so a template whose folder the session
//! may not write, such as one in the app bundle, is imported from a copy in the app's staged folder
//! ([`staged`], through the one staging every such source goes through, [`crate::staging`]). A template
//! whose folder lies inside the write root is imported where it is. A call that runs the import itself,
//! `essential.importTemplate` through `exec`, `run`, a batch, a tool or a replay, is handed its copy by
//! the imports' door instead ([`crate::imports::staged`]).
//!
//! The media the import extracts beside the template (`<stem> Media/<item>/<file>`) are not listed
//! in the pending-export journal, though their names could be read from the template first. The
//! engine writes each through its atomic write, a temporary file renamed over the name, so no media
//! file is ever partial at its name; and every import of a template writes the same names, so a
//! listing would have a restart delete whole media an earlier import wrote and a saved project may
//! name.
//! The one partial file a kill can leave is the write's own temporary file, whose name the engine
//! draws from a counter of its own as it writes, so no listing could name it.
//!
//! `warnings` lists the template's text layers whose font the phone lacks, as EffectCraft's own
//! `file.findMissing` reports them: the engine substitutes a fallback face and says nothing else.

use std::path::Path;

use craftbridge_core::{CraftError, FrameBytes, PixelFormat, Result};
use effectcraft_automation::{Backend, Error};
use effectcraft_engine::project::essential::EgKind;
use effectcraft_engine::project::{ItemId, LayerId};
use effectcraft_engine::{EngineError, Session};
use serde_json::Value;
use serde_json::map::Map;

use crate::methods::{empty_params, execute, failure, object, object_params, reject_unknown, string_param};
use crate::requests::{self, FrameRequest, RenderRequest};
use crate::runner::{self, ListOutputs, Progress, session};
use crate::staging;

/// The method that sets an instance up and answers it.
const OPEN: &str = "template.open";

/// The method that exports an instance, and the one a frame of it is asked of.
const RENDER: &str = "template.render";

/// The command that brings the template into the project, which every failure of the setup that is
/// no other command's is reported under, and the parameter that names the template file it reads, and
/// beside which it writes the template's media. The imports' door reads both for the call that runs the
/// command itself ([`crate::imports`]).
pub(crate) const IMPORT: &str = "essential.importTemplate";
pub(crate) const IMPORT_PATH: &str = "path";

/// The names the call's own parameters go under.
const TEMPLATE: &str = "template";
const VALUES: &str = "values";

/// The key a render call's request sits under.
const OUTPUT: &str = "output";

/// The field of an export request that names the file it writes.
const OUTPUT_FILE: &str = "output";

/// The key a frame call's request sits under, and the name of the binary call it is made through.
pub(crate) const FRAME: &str = "frame";

/// The fields of a request that would pick another comp than the instance.
const COMP: &str = "comp";
const QUEUE: &str = "queue";

/// What a call asked for: the template file, the values for its controls and, for a render or a frame,
/// the request for it.
struct Call {
    template: String,
    values: Map<String, Value>,
    request: Option<Value>,
}

impl Call {
    /// Read the parameters of `method`: `{template, values?}`, and the request under `request_key`
    /// when the method has one. Nothing is touched yet, so a refusal leaves the session as it was.
    ///
    /// `before_values` is the method's own refusals of what the command line refuses before it reads
    /// its values file (its parser's refusal of an option it does not know, then `template.rs`'s
    /// `validate`), made on the template and the request as the call names them; the values are read
    /// after it, as the command line reads them after `validate`.
    fn parse(params: &Value, method: &str, request_key: Option<&str>, before_values: impl FnOnce(&str, Option<&Value>) -> Result<()>) -> Result<Self> {
        let obj = object_params(params, method)?;
        let known: Vec<&str> = [TEMPLATE, VALUES].into_iter().chain(request_key).collect();
        reject_unknown(&obj, method, &known)?;
        let template =
            string_param(&obj, TEMPLATE, method)?.map(str::to_string).ok_or_else(|| CraftError::invalid_params(format!("`{method}`: needs a `{TEMPLATE}`")))?;
        let request = request_key.and_then(|key| obj.get(key)).cloned();
        before_values(&template, request.as_ref())?;
        let values = match obj.get(VALUES) {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(values)) => values.clone(),
            Some(_) => return Err(CraftError::invalid_params(format!("`{method}`: `{VALUES}` must be a JSON object mapping control names to values"))),
        };
        Ok(Self { template, values, request })
    }
}

/// `template.open`: set the instance up and answer its wrapper composition, with the warnings of
/// the template's fonts the phone lacks.
pub(crate) fn open(backend: &mut Backend, params: &Value) -> Result<Value> {
    let call = read_open(params)?;
    let set_up = prepare(backend, &call)?;
    Ok(object([("comp", Value::from(set_up.comp)), ("warnings", Value::Array(set_up.warnings))]))
}

/// A `template.open` call read whole: nothing but its parameters, which the command line has no
/// refusal of its own for.
fn read_open(params: &Value) -> Result<Call> {
    Call::parse(params, OPEN, None, |_, _| Ok(()))
}

/// Where a `template.render` call's render request sits: under `output`.
pub(crate) fn render_request(params: &Value) -> Option<&Value> {
    params.get(OUTPUT)
}

/// `template.render`: export the instance. The request under `output` is `export`'s, bar the two fields
/// the command line refuses beside `--template`, which renders a new instance and so names neither a
/// comp nor the queue; the answer is `export`'s with the template's `warnings` beside `rendered`.
pub(crate) fn render(backend: &mut Backend, params: &Value, on_progress: Progress<'_>, list_outputs: ListOutputs<'_>) -> Result<Value> {
    let (call, request) = read_render(params)?;
    let set_up = prepare(backend, &call)?;
    let mut answer = runner::export(backend, &request, on_progress, list_outputs)?;
    if let Some(rendered) = answer.as_object_mut() {
        rendered.insert("warnings".to_string(), Value::Array(set_up.warnings));
    }
    Ok(answer)
}

/// A `template.render` call read whole, as the command line reads its whole command line before it
/// imports or renders, so a call refused here leaves no instance and no copy behind, and in the order
/// the command line refuses: the parameters, and the request's own fields, as its parser refuses an
/// option it does not know before anything else; a request that names neither a comp nor the queue,
/// and an output (the `--out` it requires), which is not the template (`validate`); the values; and
/// last the values of the request's fields, which the command line reads after its import and this
/// reads before. What only the engine refuses (a format it does not know, an output outside the
/// container) is refused once the instance is set up, when the render's own commands first see it, as
/// the command line refuses a format it does not know after its import.
fn read_render(params: &Value) -> Result<(Call, Value)> {
    let mut call = Call::parse(params, RENDER, Some(OUTPUT), |template, request| {
        let request = request.ok_or_else(|| CraftError::invalid_params(format!("`{RENDER}`: needs an `{OUTPUT}` request")))?;
        RenderRequest::fields(request)?;
        refuse_instance_choice(request, OUTPUT)?;
        match request.get(OUTPUT_FILE) {
            None | Some(Value::Null) => RenderRequest::default().to_queue_params().map(drop),
            Some(Value::String(output)) => refuse_overwrite(template, output),
            Some(_) => Ok(()),
        }
    })?;
    // The reading above refused a call without the request.
    let request = call.request.take().unwrap_or(Value::Null);
    RenderRequest::from_params(&request)?.to_queue_params()?;
    Ok((call, request))
}

/// The frame call of an app: a template's frame when `params` name a `template`, which is
/// `{template, values?, frame?: FrameRequest}`, and any other call's frame as [`requests::frame`]
/// answers it. The instance is the active comp when its frame is rendered, as the command line's
/// `render-frame --template` renders it, so the request names none.
pub(crate) fn frame(backend: &mut Backend, params: &Value, format: PixelFormat) -> Result<FrameBytes> {
    if params.get(TEMPLATE).is_none() {
        return requests::frame(backend, params, format);
    }
    let (call, request) = read_frame(params)?;
    prepare(backend, &call)?;
    request.frame(backend, format)
}

/// A template's `frame` call read whole, as [`read_render`] reads a render, so a call refused here
/// leaves no instance and no copy behind: its parameters, and the request's own fields; a request that
/// names neither a comp nor the queue (`validate`); the values; and the values of the request's fields.
/// `queue` is no field of a frame's, but the command line knows the flag and refuses it beside
/// `--template`, so it is refused here in those words, not as a field the frame does not take.
fn read_frame(params: &Value) -> Result<(Call, FrameRequest)> {
    let mut call = Call::parse(params, RENDER, Some(FRAME), |_, request| match request {
        None => Ok(()),
        Some(request) => {
            FrameRequest::fields(request, &[QUEUE])?;
            refuse_instance_choice(request, FRAME)
        }
    })?;
    let request = call.request.take().unwrap_or_else(empty_params);
    Ok((call, FrameRequest::parse(&request)?))
}

/// The call's parameters with its template read from a copy inside the container where the folder
/// holding it may not be written ([`staging::stage`]), when `method` names a template:
/// `template.open`, `template.render`, or a `frame` whose parameters name one. Every other call goes on
/// as it came. The call is first read whole, as its method reads it, against the template as the call
/// names it, so a call refused for its shape makes no copy, and a render's output is weighed against
/// the template as the call named it, before the call names the copy instead.
pub(crate) fn staged(backend: &Backend, staged: &Path, method: &str, mut params: Value) -> Result<Value> {
    match method {
        OPEN => drop(read_open(&params)?),
        RENDER => drop(read_render(&params)?),
        FRAME if params.get(TEMPLATE).is_some() => drop(read_frame(&params)?),
        _ => return Ok(params),
    }
    let Some(template) = params.get(TEMPLATE).and_then(Value::as_str) else { return Ok(params) };
    if let Some(copy) = staging::stage(backend, staged, template)?
        && let Some(slot) = params.get_mut(TEMPLATE)
    {
        *slot = Value::from(copy);
    }
    Ok(params)
}

/// The parameters of the import of the template at `path`, which adds it to no comp.
fn import_params(path: &str) -> Value {
    object([(IMPORT_PATH, Value::from(path)), ("addToComp", Value::Bool(false))])
}

/// The parameters of `essential.set` for one control of the instance, given as its value or as the
/// media item it is to show.
fn set_control(comp: u64, layer: u64, control: &str, set: (&str, Value)) -> Value {
    object([("comp", Value::from(comp)), ("layer", Value::from(layer)), ("control", Value::from(control)), set])
}

/// Refuse the fields that would pick another comp than the instance: the command line refuses
/// `--comp` and `--queue` beside `--template`, since the template renders a new instance. A field asks
/// for one as the requests read it: a `comp` that is null names none, and a `queue` that is null or
/// false renders no queue.
fn refuse_instance_choice(request: &Value, key: &str) -> Result<()> {
    let asks = |field: &&str| request.get(*field).is_some_and(|given| !given.is_null() && (*field != QUEUE || given != &Value::Bool(false)));
    let named: Vec<&str> = [COMP, QUEUE].into_iter().filter(asks).collect();
    if named.is_empty() {
        return Ok(());
    }
    Err(CraftError::invalid_params(format!("`{RENDER}`: `{key}` cannot name `{}`: a template renders a new instance", named.join("`, `"))))
}

/// Refuse an output that is the template file itself, by its path or through a hard link, which the
/// command line checks before it imports or renders, since the render would overwrite its input. A
/// file that is not there yet cannot be one.
fn refuse_overwrite(template: &str, output: &str) -> Result<()> {
    let (Ok(template_path), Ok(output_path)) = (std::fs::canonicalize(template), std::fs::canonicalize(output)) else { return Ok(()) };
    if template_path == output_path || same_file(&template_path, &output_path) {
        return Err(CraftError::invalid_params(format!("`{RENDER}`: `{OUTPUT}` {output} would overwrite `{TEMPLATE}` {template}")));
    }
    Ok(())
}

/// Whether two canonical paths name one file through hard links, which the same device and inode tell.
#[cfg(unix)]
fn same_file(first: &std::path::Path, second: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(first), std::fs::metadata(second)) {
        (Ok(first), Ok(second)) => first.dev() == second.dev() && first.ino() == second.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_file(_: &std::path::Path, _: &std::path::Path) -> bool {
    false
}

/// An instance set up: its wrapper composition and the template's missing fonts.
struct SetUp {
    comp: u64,
    warnings: Vec<Value>,
}

/// The command line's `prepare`: check the values on a scratch copy when there are any, import the
/// template without adding it to a comp, make the instance and set the values on it.
fn prepare(backend: &mut Backend, call: &Call) -> Result<SetUp> {
    if !call.values.is_empty() {
        preflight(backend, &call.template, &call.values)?;
    }
    let imported = execute(backend, None, IMPORT, import_params(&call.template), None)?;
    let source =
        imported.get("comp").and_then(Value::as_u64).ok_or_else(|| CraftError::command_failed(IMPORT, "template import did not return a composition", None))?;
    let comp = instantiate(backend, source, &call.values, true)?;
    let items: Vec<u64> = imported.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect();
    Ok(SetUp { comp, warnings: missing_fonts(backend, &items)? })
}

/// The template's text layers whose font is missing, as `file.findMissing` reports them
/// (`{comp, layer, font}`), among the compositions the import brought in: the project's own layers are
/// not the template's to warn about.
fn missing_fonts(backend: &mut Backend, items: &[u64]) -> Result<Vec<Value>> {
    let found = execute(backend, None, "file.findMissing", object([("what", Value::from("fonts"))]), None)?;
    let fonts = found.get("fonts").and_then(Value::as_array).into_iter().flatten();
    Ok(fonts.filter(|font| font.get("comp").and_then(Value::as_u64).is_some_and(|comp| items.contains(&comp))).cloned().collect())
}

/// Apply the values to a scratch copy of the template, which fails on a bad one before the real
/// import extracts media. The scratch session is a bare one, as the command line's is, and takes the
/// same commands through the same door; the template is read through the session's own file service, which the confinement already holds to its roots.
fn preflight(backend: &mut Backend, path: &str, values: &Map<String, Value>) -> Result<()> {
    let live = session(backend)?;
    let bytes = live.services.read_file(path).map_err(|error| CraftError::io(format!("cannot read {path}: {error}")))?;
    let base = live.project.next_id;
    // The scratch work can panic on a hostile template, which the engine's own guard turns into an
    // error, as it does for a command.
    let checked = effectcraft_engine::guard::guarded("template value validation", || {
        let bad = |msg: String| EngineError::BadParams { cmd: IMPORT.into(), msg };
        let (manifest, mut project, _) = effectcraft_engine::commands::essential::read_template(&bytes).map_err(bad)?;
        let source = manifest.get("comp").and_then(Value::as_u64).ok_or_else(|| bad("manifest has no `comp`".into()))?;
        let source = source.checked_add(base).ok_or_else(|| bad("template composition ID overflows the import offset".into()))?;
        effectcraft_engine::project::essential::offset_ids(&mut project, base);
        project.fix_next_id();
        let mut scratch = Session::new();
        scratch.project = project.into();
        Ok(instantiate(&mut Backend::headless(scratch), source, values, false))
    });
    match checked {
        Ok(instantiated) => instantiated.map(|_| ()),
        Err(error) => Err(failure(Error::from(error), IMPORT, &Value::Null, false)),
    }
}

/// The command line's `instantiate`: duplicate the source and empty the duplicate with commands, which
/// keeps its exact rational frame rate, tick duration, background, work area and renderer settings
/// without round-tripping floats; add the source into it as one layer; give the wrapper the source's
/// own name, for `[compName]` outputs; and set each value on the instance. The scratch run
/// (`import_media` false) cannot import files, so a media control's file is left to the real run.
fn instantiate(backend: &mut Backend, source: u64, values: &Map<String, Value>, import_media: bool) -> Result<u64> {
    let run = |backend: &mut Backend, command: &str, params: Value| execute(backend, None, command, params, None);
    let duplicate = run(backend, "project.duplicate", object([("items", Value::from(vec![source]))]))?;
    let parent = duplicate
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_u64)
        .ok_or_else(|| CraftError::command_failed("project.duplicate", "cannot create template render composition", None))?;
    run(backend, "comp.open", object([("comp", Value::from(parent))]))?;
    let layers: Vec<u64> = session(backend)?
        .project
        .comp(ItemId(parent))
        .ok_or_else(|| CraftError::command_failed(IMPORT, "template render composition is missing", None))?
        .layers
        .iter()
        .map(|layer| layer.id.0)
        .collect();
    if !layers.is_empty() {
        run(
            backend,
            "layer.setSwitch",
            object([("comp", Value::from(parent)), ("layers", Value::from(layers.clone())), ("switch", Value::from("lock")), ("value", Value::Bool(false))]),
        )?;
        run(backend, "edit.clear", object([("comp", Value::from(parent)), ("layers", Value::from(layers))]))?;
    }
    let added = run(backend, "layer.addItem", object([("comp", Value::from(parent)), ("item", Value::from(source)), ("time", Value::from(0))]))?;
    let instance =
        added.get("layer").and_then(Value::as_u64).ok_or_else(|| CraftError::command_failed("layer.addItem", "cannot create template instance", None))?;
    name_wrapper(backend, source, parent, instance)?;
    // The wrapper carries the source composition's authored motion blur.
    run(
        backend,
        "layer.setSwitch",
        object([("comp", Value::from(parent)), ("layers", Value::from(vec![instance])), ("switch", Value::from("motionBlur")), ("value", Value::Bool(true))]),
    )?;
    for (control, value) in values {
        if is_media_control(session(backend)?, source, control) {
            let path = media_path(value).ok_or_else(|| CraftError::invalid_params(format!("control `{control}` is a media control: give it a file path")))?;
            if !import_media {
                continue;
            }
            let item = import_footage(backend, control, path)?;
            run(backend, "essential.set", set_control(parent, instance, control, ("item", Value::from(item))))?;
            continue;
        }
        run(backend, "essential.set", set_control(parent, instance, control, ("value", value.clone())))?;
    }
    Ok(parent)
}

/// Keep a partial final frame and the source's exact name on the wrapper: `layer.addItem` snaps its
/// default end to the frame grid, which would cut the last partial frame, and the duplicate's own
/// name is a numbered one, which a `[compName]` output would carry.
fn name_wrapper(backend: &mut Backend, source: u64, parent: u64, instance: u64) -> Result<()> {
    let edited = session(backend)?.edit("Template Duration", None, |project, _| {
        let name = project.item(ItemId(source)).ok_or_else(|| EngineError::Other("template composition is missing".into()))?.name.clone();
        project.item_mut(ItemId(parent)).ok_or_else(|| EngineError::Other("template render composition is missing".into()))?.name = name;
        let comp = project.comp_mut(ItemId(parent)).ok_or(EngineError::NoComp)?;
        let duration = comp.duration;
        let layer = comp.layer_mut(LayerId(instance)).ok_or_else(|| EngineError::Other("template instance is missing".into()))?;
        layer.out_point = duration;
        Ok(())
    });
    edited.map_err(|error| failure(Error::from(error), IMPORT, &Value::Null, false))
}

/// Import the file a media control is given as footage and answer its item, naming the control in
/// every failure, as the command line does.
fn import_footage(backend: &mut Backend, control: &str, path: &str) -> Result<u64> {
    let imported = execute(backend, None, "file.import", object([("paths", Value::from(vec![path]))]), None)?;
    if let Some(error) = imported.get("errors").and_then(Value::as_array).and_then(|errors| errors.first()) {
        return Err(CraftError::command_failed("file.import", format!("control `{control}`: cannot import {path}: {error}"), None));
    }
    imported
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_u64)
        .ok_or_else(|| CraftError::command_failed("file.import", format!("control `{control}`: {path} did not import as footage"), None))
}

/// The file a Media Replacement control is given: a path, or `{path|file|media: path}`.
fn media_path(value: &Value) -> Option<&str> {
    match value {
        Value::String(path) => Some(path.as_str()),
        Value::Object(fields) => ["path", "file", "media"].iter().find_map(|key| fields.get(*key)).and_then(Value::as_str),
        _ => None,
    }
}

/// Whether the template's control of that name, looked up as `essential.set` looks it up (by name,
/// else by name in any case), is a Media Replacement.
fn is_media_control(session: &Session, source: u64, name: &str) -> bool {
    let Some(essential) = session.project.comp(ItemId(source)).and_then(|comp| comp.essential.as_ref()) else { return false };
    let all = essential.flat();
    let control = all.iter().find(|control| control.name == name).or_else(|| all.iter().find(|control| control.name.eq_ignore_ascii_case(name)));
    control.is_some_and(|control| matches!(control.kind, EgKind::Media { .. }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use effectcraft_time::Tick;

    /// The wrapper's layer ends at the composition's own end and the wrapper takes the source's own
    /// name, which the duplicate's numbered one would otherwise hide from a `[compName]` output. The
    /// end is the one the frame grid would cut: a composition made by a command always ends on the
    /// grid, so the test sets an end between two frames, as an imported project can have one.
    #[test]
    fn the_instance_ends_where_the_wrapper_does_and_the_wrapper_has_the_sources_name() {
        let mut backend = Backend::headless(Session::new());
        let mut exec = |command: &str, params: Value| backend.exec(command, params).expect("the command runs");
        exec("comp.new", object([("name", Value::from("Source")), ("width", Value::from(8)), ("height", Value::from(8)), ("duration", Value::from(1))]));
        let duplicate = exec("project.duplicate", object([("items", Value::from(vec![1_u64]))]));
        let parent = duplicate["items"][0].as_u64().expect("the duplicate's id");
        let added = exec("layer.addItem", object([("comp", Value::from(parent)), ("item", Value::from(1)), ("time", Value::from(0))]));
        let instance = added["layer"].as_u64().expect("the instance's id");
        let between = Tick::from_seconds_f64(1.0111);
        session(&mut backend)
            .expect("a session")
            .edit("Duration", None, |project, _| {
                project.comp_mut(ItemId(parent)).expect("the wrapper").duration = between;
                Ok(())
            })
            .expect("the duration is set");
        name_wrapper(&mut backend, 1, parent, instance).expect("the wrapper is named");
        let project = &session(&mut backend).expect("a session").project;
        assert_eq!(project.item(ItemId(parent)).map(|item| item.name.as_str()), Some("Source"));
        let end = project.comp(ItemId(parent)).and_then(|comp| comp.layer(LayerId(instance))).map(|layer| layer.out_point);
        assert_eq!(end, Some(between));
    }
}
