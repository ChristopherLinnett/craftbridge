//! The Content-Aware Fill runner (AD20): what `contentFill.generate` does when it is asked to wait,
//! written on EffectCraft's public session calls, with the wait turned into the progress contract of
//! the core's export call.
//!
//! The command writes the fill's frames, `fill_00000.png` on, into one folder from a background task
//! the exporter never sees, so nothing but this runner can tell the pending-export journal what a
//! killed fill leaves. The runner therefore reads the call as the command will, lists every frame it
//! may write before it starts the task, and gives the command exactly what it read — the composition
//! and layer by their ids, the folder, the range — so the command can write only what was listed. A
//! call it cannot read as the command would is not started: it is handed to the engine in a form that
//! fails where the command's own reading fails, so the refusal comes in the engine's words and no
//! fill can have started. The frames are written in place, at the names listed, never through the
//! session's own writer, whose temporary files no listing can name.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use craftbridge_core::{CraftError, PROGRESS_INTERVAL, Result};
use effectcraft_automation::confine::Access;
use effectcraft_automation::{Backend, Error};
use effectcraft_engine::project::{Comp, ItemId, LayerId};
use effectcraft_engine::{EngineError, Services, Session};
use effectcraft_time::Tick;
use serde_json::Value;
use serde_json::map::Map;

use crate::methods::{FILL, execute_listed, failure, object_params};
use crate::outputs;
use crate::runner::{Ended, ListOutputs, Progress, session};

/// How long the runner waits between looks at a fill it has stopped, while the task's thread winds
/// down: a stopped fill reports at its next frame, which is soon, so this is short, and it is no
/// bound on anything.
const WIND_DOWN: std::time::Duration = std::time::Duration::from_millis(2);

/// The name of frame `k` of a fill's sequence, which the command numbers from zero whatever the
/// frame of the composition it fills (`crates/engine/src/cmd/content_fill.rs`:423).
fn frame_name(k: usize) -> String {
    format!("fill_{k:05}.png")
}

/// What one call will write, read as the command reads it.
struct Plan {
    /// The folder of the sequence.
    dir: PathBuf,
    /// How many frames the command fills: frame `k` of `0..frames` is [`frame_name`].
    frames: usize,
    /// The composition, the layer and the range the command is told, so it reads nothing the plan did
    /// not: the range only when the call's own spelling is one the command accepts, since the command
    /// refuses any other before it writes.
    given: Vec<(&'static str, Value)>,
}

/// What was on the disk before the fill started, which a fill that does not finish must leave as it
/// was.
struct Before {
    files: HashSet<PathBuf>,
    /// The folder and each folder above it that was not there, deepest first: the fill's spill creates
    /// them all, and the pending-export journal removes them all after a kill.
    missing: Vec<PathBuf>,
}

impl Plan {
    /// Read `fields` as `contentFill.generate` does, or `None` where it would refuse: no composition,
    /// no layer, or none of them selected.
    fn of(session: &Session, fields: &Map<String, Value>) -> Option<Self> {
        let given = Value::Object(fields.clone());
        let comp_id = comp_of(session, &given)?;
        let comp = session.project.comp(comp_id)?;
        let layer_id = layer_of(session, comp, &given)?;
        let layer = comp.layer(layer_id)?;
        let range = range_of(session, &given);
        let entire = range.unwrap_or(true);
        let (from, to) = if entire { (layer.in_point, layer.out_point) } else { (comp.work_area.0.max(layer.in_point), comp.work_area.1.min(layer.out_point)) };
        let (from, to) = (from.max(Tick::ZERO), to.min(comp.duration));
        let dir = given.get("outputDir").and_then(Value::as_str).map_or_else(|| default_dir(session, &layer.name), PathBuf::from);
        let mut told =
            vec![("comp", Value::from(comp_id.0)), ("layer", Value::from(layer_id.0)), ("outputDir", Value::from(dir.to_string_lossy().into_owned()))];
        if let Some(entire) = range {
            told.push(("range", Value::from(if entire { "entire" } else { "workArea" })));
        }
        Some(Self { dir, frames: frames_between(comp, from, to), given: told })
    }

    /// Every file the fill may write, in the order it writes them.
    fn files(&self) -> impl Iterator<Item = PathBuf> + '_ {
        (0..self.frames).map(|k| self.dir.join(frame_name(k)))
    }

    /// Refuse, in the confinement's own words, a folder or a frame the session may not write, before
    /// anything is listed or started. The confinement checks an absolute path a call to the command
    /// names only as one it reads, a relative one not at all, and none when the command picks the
    /// folder itself, and the session's file services write wherever they are told
    /// (`crates/automation/src/confine.rs`:319-324, :360-366, :572-583). So without this a fill would
    /// write into the bundle, into the process's working folder, or into the system's temporary
    /// folder for a project with no file, wherever the engine keeps no journal to refuse its listing.
    /// Each frame is checked as well as the folder, so a frame whose name is a link out of the
    /// container is refused in the same words whether or not the engine keeps a journal, which
    /// refuses to list any link.
    fn check_writes(&self, backend: &Backend) -> std::result::Result<(), Error> {
        let Some(confinement) = backend.confinement() else { return Ok(()) };
        for path in std::iter::once(self.dir.clone()).chain(self.files()) {
            confinement.check(&path.to_string_lossy(), Access::Write)?;
        }
        Ok(())
    }

    /// The call as the command is given it: the caller's own parameters, the plan's over them, and no
    /// waiting, which would run the whole fill with no progress and nothing to stop it.
    fn call(&self, mut fields: Map<String, Value>) -> Value {
        for (key, value) in &self.given {
            fields.insert((*key).to_string(), value.clone());
        }
        fields.insert("wait".to_string(), Value::Bool(false));
        Value::Object(fields)
    }

    fn before(&self) -> Before {
        let missing = self.dir.ancestors().take_while(|folder| !folder.as_os_str().is_empty() && !folder.exists()).map(Path::to_path_buf).collect();
        Before { files: self.files().filter(|path| path.exists()).collect(), missing }
    }

    /// Take away what a fill that failed left: the engine leaves every folder its scratch file made
    /// above the fill's own. What the call found already there stays. A fill that was stopped is not
    /// withdrawn here: the core deletes what a call listed once it answers `Cancelled`.
    fn withdraw(&self, before: &Before) {
        for path in self.files().filter(|path| !before.files.contains(path)) {
            let _ = std::fs::remove_file(path);
        }
        for folder in &before.missing {
            // A folder that still holds anything else stays: `remove_dir` leaves it.
            let _ = std::fs::remove_dir(folder);
        }
    }
}

/// The session's file services while a fill's task starts, which the task keeps for its whole run:
/// every call is the session's own, but for the frames the runner listed, each written straight to its
/// own name. The session's own write puts a file down through a temporary file beside it, named for the
/// process and a count the engine keeps to itself (`crates/engine/src/services.rs`:32-37, :99-101,
/// `crates/engine/src/settings/config.rs`:196-236), which no listing can name, so a kill that lands
/// while a frame is written would leave that file, and the folders that hold it, for good. Written in
/// place, a frame a kill cuts short is one the journal lists. A link at a frame's name is taken away
/// first, as the session's own rename over it would replace it, so no frame is written through one.
struct FrameWrites {
    inner: Arc<dyn Services>,
    frames: HashSet<PathBuf>,
}

impl Services for FrameWrites {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        self.inner.read_file(path)
    }
    fn read_file_bounded(&self, path: &str, max: u64) -> std::io::Result<Vec<u8>> {
        self.inner.read_file_bounded(path, max)
    }
    fn write_file(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
        self.inner.write_file(path, data)
    }
    fn exists(&self, path: &str) -> bool {
        self.inner.exists(path)
    }
    fn stamp(&self, path: &str) -> Option<(u64, u128)> {
        self.inner.stamp(path)
    }
    fn store_file(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
        let frame = Path::new(path);
        if !self.frames.contains(frame) {
            return self.inner.store_file(path, data);
        }
        // As the session's own store makes the folder: a folder it cannot make fails the write itself.
        if let Some(folder) = frame.parent().filter(|folder| !folder.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(folder);
        }
        if std::fs::symlink_metadata(frame).is_ok_and(|found| found.file_type().is_symlink()) {
            std::fs::remove_file(frame)?;
        }
        std::fs::write(frame, data)
    }
    fn list_dir(&self, dir: &str) -> Vec<String> {
        self.inner.list_dir(dir)
    }
    fn is_dir(&self, path: &str) -> bool {
        self.inner.is_dir(path)
    }
    fn full_path(&self, path: &str) -> Option<String> {
        self.inner.full_path(path)
    }
}

/// Start the fill's task as the plan calls it, with the session's services the [`FrameWrites`] over
/// its own while the task starts, and its own again once it has: the task keeps the services it
/// started with, and no other call writes the frames.
fn start(backend: &mut Backend, plan: &Plan, fields: Map<String, Value>) -> Result<Value> {
    let own = session(backend)?.services.clone();
    session(backend)?.services = Arc::new(FrameWrites { inner: own.clone(), frames: plan.files().collect() });
    let started = execute_listed(backend, FILL, plan.call(fields));
    if let Some(session) = backend.session() {
        session.services = own;
    }
    started
}

/// The frames the command fills between `from` and `to`: the frame the nearest frame boundary to
/// `from` falls in, through the frame before `to`, none for a range that holds none
/// (`crates/engine/src/cmd/content_fill.rs`:520-527).
fn frames_between(comp: &Comp, from: Tick, to: Tick) -> usize {
    if to <= from {
        return 0;
    }
    let rate = comp.frame_rate;
    let (first, last) = (rate.frame_at(rate.snap_nearest(from)), rate.frame_at(to - Tick(1)));
    usize::try_from(last.saturating_sub(first).saturating_add(1)).unwrap_or(0)
}

/// Whether the fill covers the layer's whole duration, as the command reads `range`: the call's own
/// spelling, else the panel's setting, `entire` only when it says so. `None` for a spelling the
/// command refuses (`crates/engine/src/cmd/content_fill.rs`:69-82).
fn range_of(session: &Session, given: &Value) -> Option<bool> {
    match given.get("range").and_then(Value::as_str) {
        Some("workArea" | "work area") => Some(false),
        Some("entire" | "entireDuration" | "layer") => Some(true),
        Some(_) => None,
        None => Some(session.state.content_fill.range == "entire"),
    }
}

/// The composition the call names, as the command resolves it (`crates/engine/src/cmd/params.rs`:
/// 91-106): an id, a name, a string of digits that names no composition read as an id, else the
/// active one.
fn comp_of(session: &Session, given: &Value) -> Option<ItemId> {
    let exists = |id: &ItemId| session.project.comp(*id).is_some();
    match given.get("comp") {
        Some(Value::Number(id)) => id.as_u64().map(ItemId).filter(exists),
        Some(Value::String(name)) => session
            .project
            .items
            .values()
            .find(|item| &item.name == name && item.as_comp().is_some())
            .map(|item| item.id)
            .or_else(|| digits(name).map(ItemId).filter(exists)),
        _ => session.active_comp_id(),
    }
}

/// The layer the call names, else the first selected one (`crates/engine/src/cmd/params.rs`:134-147).
fn layer_of(session: &Session, comp: &Comp, given: &Value) -> Option<LayerId> {
    match given.get("layer") {
        Some(named) => resolve_layer(comp, named),
        None => session.state.selected_layers.first().copied().filter(|id| comp.layer(*id).is_some()),
    }
}

/// A layer reference: an id, else the one-based index when no layer has that id, `#n` for an index, a
/// name, or a string of digits read as the number (`crates/engine/src/cmd/params.rs`:117-131).
fn resolve_layer(comp: &Comp, named: &Value) -> Option<LayerId> {
    match named {
        Value::Number(number) => {
            let id = number.as_u64()?;
            if comp.layer(LayerId(id)).is_some() {
                return Some(LayerId(id));
            }
            comp.layers.get(usize::try_from(id).ok()?.checked_sub(1)?).map(|layer| layer.id)
        }
        Value::String(text) => match text.strip_prefix('#').and_then(|index| index.parse::<usize>().ok()) {
            Some(index) => comp.layers.get(index.checked_sub(1)?).map(|layer| layer.id),
            None => comp
                .layers
                .iter()
                .find(|layer| &layer.name == text)
                .map(|layer| layer.id)
                .or_else(|| digits(text).and_then(|id| resolve_layer(comp, &Value::from(id)))),
        },
        _ => None,
    }
}

/// A string of ASCII digits and nothing else as the number it spells.
fn digits(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) { None } else { text.parse().ok() }
}

/// The folder the command chooses when the call names none: `<layer>_Fill_<k>` for the first `k` that
/// holds nothing, in the `Fill` folder beside the project, or in the system's temporary folder for a
/// project with no file (`crates/engine/src/cmd/content_fill.rs`:134-149).
fn default_dir(session: &Session, layer_name: &str) -> PathBuf {
    let base = match session.path.as_deref().and_then(|saved| Path::new(saved).parent()) {
        Some(folder) => folder.join("Fill"),
        None => std::env::temp_dir().join("effectcraft-fill"),
    };
    let safe: String = layer_name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let free = |dir: &PathBuf| {
        let named = dir.to_string_lossy();
        !session.services.exists(&named) && !session.services.exists(&format!("{named}/{}", frame_name(0)))
    };
    (1_u64..).map(|k| base.join(format!("{safe}_Fill_{k}"))).find(free).unwrap_or(base)
}

/// A fill renders no movie, so its call holds no render request and never chooses hardware encoding.
pub(crate) fn render_request(_: &Value) -> Option<&Value> {
    None
}

/// `contentFill.generate`: read the call, refuse a folder or frame the confinement does not let the
/// session write, list every frame the fill may write with `list_outputs`, start the fill as a
/// background task, report its progress every [`PROGRESS_INTERVAL`] and answer what the command
/// answers when it waits, `{layer, item, name, frames, files}`, or stop the task as soon as
/// `on_progress` answers `false`.
///
/// The progress is the task's own `(done, total)`: the layer's frames read, then the fill, then the
/// files written. It has no estimate of the time left.
pub(crate) fn generate(backend: &mut Backend, params: &Value, on_progress: Progress<'_>, list_outputs: ListOutputs<'_>) -> Result<Value> {
    let fields = object_params(params, FILL)?.into_owned();
    let Some(plan) = Plan::of(session(backend)?, &fields) else { return Err(refused(backend, fields)) };
    plan.check_writes(backend).map_err(|error| failure(error, FILL, params, false))?;
    outputs::list_in_chunks(plan.files(), list_outputs)?;
    let before = plan.before();
    let started = start(backend, &plan, fields)?;
    let task = match started.get("job").and_then(Value::as_str) {
        Some(job) if job.starts_with("task:") => job.to_string(),
        _ => return Err(CraftError::internal(format!("`{FILL}` did not start a background task: {started}"))),
    };
    let answer = match watch(session(backend)?, &task, on_progress) {
        Ended::Stopped => Err(CraftError::Cancelled),
        Ended::Ran => outcome(session(backend)?, &task, params),
    };
    // A fill that fails leaves the container as a restart after a kill does: the engine takes away the
    // frames and the one folder it made only when its task sees the failure, and never the folders
    // above that its scratch file made. A stop is the core's: it deletes what was listed.
    if matches!(&answer, Err(error) if *error != CraftError::Cancelled) {
        plan.withdraw(&before);
    }
    answer
}

/// The refusal of a call the runner cannot read as the command does, in the engine's own words. The
/// command is handed the call with a `range` it refuses, which it checks right after it has resolved
/// the composition and the layer, so it fails where its own reading of the call fails, and fails
/// there for any call the runner read too narrowly: a fill is never started.
fn refused(backend: &mut Backend, mut fields: Map<String, Value>) -> CraftError {
    fields.insert("range".to_string(), Value::from(""));
    match execute_listed(backend, FILL, Value::Object(fields)) {
        Err(error) => error,
        Ok(answer) => CraftError::internal(format!("`{FILL}` answered a call that names no range: {answer}")),
    }
}

/// Poll the running fill at each beat and hand its progress to the listener, until the task ends or
/// the listener answers `false`. A stop waits for the task's thread to wind down: the command removes
/// the frames it wrote only when its task sees the stop, so returning earlier would hand back a
/// session whose frames still exist and whose task a later call would find running.
fn watch(session: &mut Session, task: &str, on_progress: Progress<'_>) -> Ended {
    loop {
        session.poll_jobs();
        let Some(job) = session.jobs().into_iter().find(|job| job.id == task) else { return Ended::Ran };
        if !on_progress(job.done, job.total, None) {
            session.cancel_job(task);
            loop {
                session.poll_jobs();
                if !session.jobs().iter().any(|job| job.id == task) {
                    return Ended::Stopped;
                }
                std::thread::sleep(WIND_DOWN);
            }
        }
        std::thread::sleep(PROGRESS_INTERVAL);
    }
}

/// The fill's answer once its task has ended: what the command answers when it waits, or its failure
/// as the same call would fail it.
fn outcome(session: &Session, task: &str, params: &Value) -> Result<Value> {
    let Some(record) = session.job_log.iter().rev().find(|record| record.id == task) else {
        return Err(CraftError::internal(format!("`{FILL}`: the task {task} ended and left no record")));
    };
    if record.status == "done" {
        return Ok(record.result.clone());
    }
    let why = if record.message.is_empty() { record.status.clone() } else { record.message.clone() };
    Err(failure(Error::Engine(EngineError::Other(why)), FILL, params, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session;
    use serde_json::json;
    use std::collections::BTreeSet;
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    /// How long a test waits for a gate before it fails instead of hanging: a bound on a failure, never a
    /// timing the passing test depends on.
    const DEADLINE: Duration = Duration::from_secs(30);

    /// What the fill's file writes wait at: each write arrives and waits until the gate is open, so a test
    /// holds the task at a point it can name, whatever the machine's speed.
    #[derive(Default)]
    struct Gate {
        state: Mutex<(usize, bool)>,
        changed: Condvar,
    }

    impl Gate {
        fn arrive_and_wait(&self) {
            let mut state = self.state.lock().expect("the gate's lock");
            state.0 += 1;
            self.changed.notify_all();
            let _ = self.changed.wait_timeout_while(state, DEADLINE, |state| !state.1).expect("the gate's lock");
        }

        fn wait_until_arrived(&self, writes: usize) {
            let state = self.state.lock().expect("the gate's lock");
            let (state, _) = self.changed.wait_timeout_while(state, DEADLINE, |state| state.0 < writes).expect("the gate's lock");
            assert!(state.0 >= writes, "the fill never reached its write");
        }

        fn open(&self) {
            self.state.lock().expect("the gate's lock").1 = true;
            self.changed.notify_all();
        }
    }

    /// The session's own file services, with every frame the fill writes held at a [`Gate`] and every
    /// file the session's services write noted: the command asks whether each frame is there right
    /// before it writes it, and the runner's own writer of the frames passes every other call on.
    struct Gated {
        inner: Arc<dyn Services>,
        gate: Arc<Gate>,
        written: Arc<Mutex<Vec<String>>>,
    }

    impl Services for Gated {
        fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
            self.inner.read_file(path)
        }
        fn write_file(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
            self.written.lock().expect("the record").push(path.to_string());
            self.inner.write_file(path, data)
        }
        fn store_file(&self, path: &str, data: &[u8]) -> std::io::Result<()> {
            self.written.lock().expect("the record").push(path.to_string());
            self.inner.store_file(path, data)
        }
        fn exists(&self, path: &str) -> bool {
            if Path::new(path).file_name().is_some_and(|name| name.to_string_lossy().starts_with("fill_")) {
                self.gate.arrive_and_wait();
            }
            self.inner.exists(path)
        }
    }

    /// The file services that only record the path each `store_file` is asked to store, so a test can
    /// name who was asked to write.
    #[derive(Default)]
    struct Recorder {
        stored: Mutex<Vec<String>>,
    }

    impl Services for Recorder {
        fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("the recorder has nothing at {path}")))
        }
        fn write_file(&self, _path: &str, _data: &[u8]) -> std::io::Result<()> {
            Ok(())
        }
        fn exists(&self, _path: &str) -> bool {
            false
        }
        fn store_file(&self, path: &str, _data: &[u8]) -> std::io::Result<()> {
            self.stored.lock().expect("the recorder").push(path.to_string());
            Ok(())
        }
    }

    struct Rig {
        backend: Backend,
        dir: tempfile::TempDir,
    }

    /// A project of one 32 by 24 solid comp of `frames` frames at ten a second, with a hole cut in its
    /// solid, so a fill has something to fill.
    fn rig(frames: u32) -> Rig {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let backend = session::tests::backend_running([
            ("comp.new", json!({"name": "A", "width": 32, "height": 24, "duration": f64::from(frames) / 10.0, "frameRate": 10})),
            ("layer.newSolid", json!({"comp": 1, "name": "Bg"})),
            ("layer.addMask", json!({"rect": [10.0, 8.0, 6.0, 6.0], "mode": "subtract"})),
        ]);
        Rig { backend, dir }
    }

    impl Rig {
        fn out(&self, name: &str) -> String {
            self.dir.path().join(name).display().to_string()
        }

        /// Hold the fill's frames at a gate, and note what the session's own services write.
        fn watch(&mut self) -> (Arc<Gate>, Arc<Mutex<Vec<String>>>) {
            let (gate, written) = (Arc::new(Gate::default()), Arc::new(Mutex::new(Vec::new())));
            let session = self.backend.session().expect("a session");
            session.services = Arc::new(Gated { inner: session.services.clone(), gate: gate.clone(), written: written.clone() });
            (gate, written)
        }

        fn gate(&mut self) -> Arc<Gate> {
            self.watch().0
        }
    }

    /// Run `generate` with the progress and lister given, answering what the lister was handed.
    fn run(rig: &mut Rig, params: &Value, on_progress: Progress<'_>) -> (Result<Value>, Vec<PathBuf>) {
        let mut listed = Vec::new();
        let answer = generate(&mut rig.backend, params, on_progress, &mut |paths| {
            listed.extend(paths.iter().cloned());
            Ok(())
        });
        (answer, listed)
    }

    fn files_below(dir: &str) -> BTreeSet<PathBuf> {
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|entry| entry.path()).collect()
    }

    /// What the command itself answers `params` with, mapped as `exec` maps it, which the runner's own
    /// refusal must match word for word.
    fn engine_says(rig: &mut Rig, params: &Value) -> CraftError {
        match rig.backend.exec(FILL, params.clone()) {
            Err(error) => failure(error, FILL, params, false),
            Ok(answer) => panic!("the engine answered {answer}"),
        }
    }

    /// Every call the command reads one way is listed as the files it writes, however the call spells its
    /// composition, layer and range: what is listed is what a real fill wrote, a frame for frame, and
    /// every frame is listed before the first is written.
    #[test]
    fn what_is_listed_is_what_a_real_fill_writes_for_every_way_a_call_names_its_range_and_layer() {
        let narrowed = [("comp.workArea", json!({"start": 0.13, "end": 0.37}))];
        let cases: Vec<(&str, Vec<(&str, Value)>, Value, usize)> = vec![
            ("the work area is the whole comp", vec![], json!({"layer": 1}), 5),
            ("a narrowed work area", narrowed.to_vec(), json!({"layer": 1}), 3),
            ("the same, spelt `work area`", narrowed.to_vec(), json!({"layer": 1, "range": "work area"}), 3),
            ("the whole layer", narrowed.to_vec(), json!({"layer": 1, "range": "entire"}), 5),
            ("`entireDuration`", narrowed.to_vec(), json!({"layer": 1, "range": "entireDuration"}), 5),
            ("`layer`", narrowed.to_vec(), json!({"layer": 1, "range": "layer"}), 5),
            ("the range the panel last set", vec![("contentFill.set", json!({"range": "entire"})), narrowed[0].clone()], json!({"layer": 1}), 5),
            (
                "a call's range beats the panel's",
                vec![("contentFill.set", json!({"range": "entire"})), narrowed[0].clone()],
                json!({"layer": 1, "range": "workArea"}),
                3,
            ),
            ("a comp by name and a layer by name", vec![], json!({"comp": "A", "layer": "Bg"}), 5),
            ("a comp by id, a layer by index", vec![], json!({"comp": 1, "layer": "#1"}), 5),
            ("a comp and a layer by digits", vec![], json!({"comp": "1", "layer": "1"}), 5),
            ("the active comp and the selected layer", vec![], json!({}), 5),
        ];
        for (what, setup, params, frames) in cases {
            let mut rig = rig(5);
            for (command, given) in setup {
                rig.backend.exec(command, given).unwrap_or_else(|error| panic!("{what}: {command}: {error}"));
            }
            let out = rig.out("fill");
            let mut params = params;
            params["outputDir"] = json!(out);
            let before = files_below(&out);
            let mut listed = Vec::new();
            let answer = generate(&mut rig.backend, &params, &mut |_, _, _| true, &mut |paths| {
                assert_eq!(files_below(&out), before, "{what}: every frame is listed before the first is written");
                listed.extend(paths.iter().cloned());
                Ok(())
            })
            .unwrap_or_else(|error| panic!("{what}: {error}"));
            assert_eq!(answer["frames"], json!(frames), "{what}");
            assert_eq!(listed.iter().cloned().collect::<BTreeSet<_>>(), files_below(&out), "{what}: listed and written differ");
            assert_eq!(listed.len(), frames, "{what}");
        }
    }

    /// A frame rate that is no whole number puts the range's edges between frame boundaries, where the
    /// frame count is the nearest-boundary rule's.
    #[test]
    fn a_range_between_frame_boundaries_is_listed_as_the_command_counts_it() {
        for (rate, start, end) in [(23.976, 0.1, 0.9), (29.97, 0.02, 0.5), (24.0, 0.0205, 0.4795), (30.0, 0.0166, 0.0334)] {
            let mut rig = rig(5);
            let params = [
                ("comp.new", json!({"name": "B", "width": 32, "height": 24, "duration": 1.0, "frameRate": rate})),
                ("layer.newSolid", json!({"comp": "B", "name": "Bg"})),
                ("layer.addMask", json!({"rect": [10.0, 8.0, 6.0, 6.0], "mode": "subtract"})),
                ("comp.workArea", json!({"start": start, "end": end})),
            ];
            for (command, given) in params {
                rig.backend.exec(command, given).unwrap_or_else(|error| panic!("{command}: {error}"));
            }
            let out = rig.out("fill");
            let (answer, listed) = run(&mut rig, &json!({"comp": "B", "layer": "Bg", "outputDir": out}), &mut |_, _, _| true);
            let answer = answer.unwrap_or_else(|error| panic!("{rate}: {error}"));
            assert_eq!(listed.iter().cloned().collect::<BTreeSet<_>>(), files_below(&out), "{rate} from {start} to {end}");
            assert_eq!(answer["frames"], json!(listed.len()), "{rate}");
        }
    }

    /// With no folder named the command picks `<layer>_Fill_<k>` beside the project's file, the first
    /// `k` that holds nothing, and the runner lists that folder's frames.
    #[test]
    fn the_folder_the_command_would_choose_is_the_one_listed_and_written() {
        let mut rig = rig(3);
        let project = rig.out("p.ecproj");
        rig.backend.exec("file.saveAs", json!({"path": project})).expect("the project saves");
        std::fs::create_dir_all(rig.dir.path().join("Fill/Bg_Fill_1")).expect("a folder that is taken");
        let (answer, listed) = run(&mut rig, &json!({"layer": 1}), &mut |_, _, _| true);
        let answer = answer.expect("the fill runs");
        let taken = rig.dir.path().join("Fill/Bg_Fill_2");
        assert_eq!(listed, (0..3).map(|k| taken.join(frame_name(k))).collect::<Vec<_>>());
        assert_eq!(files_below(&taken.display().to_string()), listed.iter().cloned().collect::<BTreeSet<_>>());
        assert_eq!(answer["frames"], json!(3));
    }

    /// A call the runner cannot read as the command does is refused in the engine's own words, and
    /// starts and lists nothing.
    #[test]
    fn a_call_the_engine_refuses_while_it_reads_it_is_refused_in_the_engines_words_and_lists_nothing() {
        let calls = [
            json!({"layer": 9}),
            json!({"layer": "nope"}),
            json!({"layer": null}),
            json!({"layer": "#0"}),
            json!({"comp": "zzz", "layer": 1}),
            json!({"comp": 7, "layer": 1}),
            json!({"layer": 1, "lyer": 2}),
            json!({"layer": 1, "range": "forever"}),
            json!({"layer": 1, "method": "magic"}),
        ];
        for params in calls {
            let mut rig = rig(3);
            let out = rig.out("fill");
            let mut params = params;
            params["outputDir"] = json!(out);
            let (answer, listed) = run(&mut rig, &params, &mut |_, _, _| true);
            let expected = engine_says(&mut rig, &params);
            assert_eq!(answer, Err(expected), "{params}");
            assert!(files_below(&out).is_empty(), "{params}: nothing was written");
            assert!(rig.backend.session().expect("a session").jobs().is_empty(), "{params}: no task started");
            let _ = listed;
        }
        // No layer given and none selected: the engine's own refusal, with nothing to list.
        let mut rig = rig(3);
        rig.backend.exec("layer.select", json!({"layers": []})).ok();
        rig.backend.session().expect("a session").state.selected_layers.clear();
        let (answer, listed) = run(&mut rig, &json!({}), &mut |_, _, _| true);
        let expected = engine_says(&mut rig, &json!({}));
        assert_eq!((answer, listed), (Err(expected), Vec::new()));
    }

    /// The runner's refusal hands the command a range it refuses right after it resolves the
    /// composition and the layer, so even a call the command would have run, which a runner reading
    /// too narrowly could hand it, is refused in the engine's words and starts no fill.
    #[test]
    fn the_refusal_starts_no_fill_even_for_a_call_the_command_would_run() {
        let mut rig = rig(3);
        let out = rig.out("fill");
        let call = json!({"layer": 1, "outputDir": out});
        let answer = refused(&mut rig.backend, call.as_object().cloned().expect("the call is an object"));
        assert!(rig.backend.session().expect("a session").jobs().is_empty(), "a task started");
        let mut with_no_range = call;
        with_no_range["range"] = json!("");
        assert_eq!(answer, engine_says(&mut rig, &with_no_range));
        assert!(!Path::new(&out).exists(), "nothing was written");
    }

    /// A parameter that is no object is refused as the door refuses every method's.
    #[test]
    fn parameters_that_are_no_object_are_refused() {
        let mut rig = rig(3);
        let (answer, listed) = run(&mut rig, &json!([1]), &mut |_, _, _| true);
        assert_eq!(answer, Err(CraftError::invalid_params("`contentFill.generate`: its parameters must be a JSON object")));
        assert!(listed.is_empty());
    }

    /// The journal refusing the list refuses the fill before any frame is rendered or written: the
    /// folder is not even made.
    #[test]
    fn a_refused_listing_stops_the_fill_before_it_starts() {
        let mut rig = rig(3);
        let out = rig.out("fill");
        let refused =
            generate(&mut rig.backend, &json!({"layer": 1, "outputDir": out}), &mut |_, _, _| true, &mut |_| Err(CraftError::io("outside the container")));
        assert_eq!(refused, Err(CraftError::io("outside the container")));
        assert!(!Path::new(&out).exists(), "nothing was written, the folder included");
        assert!(rig.backend.session().expect("a session").jobs().is_empty(), "no task started");
        assert_eq!(rig.backend.session().expect("a session").project.comp(ItemId(1)).map(|comp| comp.layers.len()), Some(1));
    }

    /// The task is held at its first write, so the listener hears the fill while it runs, and a call
    /// that asks to wait still runs as a task the runner watches: waiting would hold this thread at the
    /// gate for good.
    #[test]
    fn progress_is_relayed_while_the_task_runs_and_a_call_that_waits_still_reports() {
        let mut rig = rig(4);
        let gate = rig.gate();
        let out = rig.out("fill");
        let mut heard = Vec::new();
        let (answer, _) = run(&mut rig, &json!({"layer": 1, "outputDir": out, "wait": true}), &mut |done, total, left| {
            gate.wait_until_arrived(1);
            heard.push((done, total, left));
            gate.open();
            true
        });
        let answer = answer.expect("the fill ends");
        assert_eq!(answer["frames"], json!(4));
        assert!(heard.iter().all(|(done, total, left)| (*total == 12 || *total == 0) && done <= total && left.is_none()), "{heard:?}");
        assert!(!heard.is_empty());
        assert_eq!(files_below(&out).len(), 4, "a finished fill keeps its frames");
        assert_eq!(rig.backend.session().expect("a session").project.comp(ItemId(1)).map(|comp| comp.layers.len()), Some(2), "and adds its layer");
    }

    /// The first `false` stops the fill: the listener is never asked again, the call answers `Cancelled`,
    /// the task is gone, and no layer is left. The folders the call made are the core's to take away
    /// once it has answered, so they are not looked at here (`tests/export`).
    #[test]
    fn the_first_false_stops_the_fill_and_leaves_no_layer() {
        let mut rig = rig(4);
        let gate = rig.gate();
        let out = rig.out("deep/er/fill");
        let mut calls = 0;
        let (answer, _) = run(&mut rig, &json!({"layer": 1, "outputDir": out}), &mut |_, _, _| {
            calls += 1;
            // The task is held before its first frame is written.
            gate.wait_until_arrived(1);
            assert!(files_below(&out).is_empty(), "no frame is written before the stop");
            gate.open();
            false
        });
        assert_eq!(answer, Err(CraftError::Cancelled));
        assert_eq!(calls, 1, "the listener is not asked again once it has said stop");
        let session = rig.backend.session().expect("a session");
        assert!(session.jobs().is_empty(), "the task has wound down on return");
        assert_eq!(session.project.comp(ItemId(1)).map(|comp| comp.layers.len()), Some(1), "no layer was added");
        // Only the core deletes what a stopped call listed: the runner keeps no second way to.
        assert!(rig.dir.path().join("deep/er").is_dir(), "the runner took away a folder the core is to");
    }

    /// The frames are written in place by the runner's own writer, never through the session's
    /// services, whose write puts each file down through a temporary file no listing can name, and the
    /// session has its own services back once the fill's task has started.
    #[test]
    fn the_frames_never_go_through_the_sessions_own_writer() {
        let mut rig = rig(3);
        let (gate, written) = rig.watch();
        gate.open();
        let out = rig.out("fill");
        let (answer, listed) = run(&mut rig, &json!({"layer": 1, "outputDir": out}), &mut |_, _, _| true);
        assert_eq!(answer.expect("the fill runs")["frames"], json!(3));
        assert_eq!(files_below(&out), listed.iter().cloned().collect::<BTreeSet<_>>(), "listed and written differ");
        assert_eq!(*written.lock().expect("the record"), Vec::<String>::new(), "a frame went through the session's own writer");
        let again = listed.first().expect("a frame").display().to_string();
        rig.backend.session().expect("a session").services.store_file(&again, b"again").expect("the session stores a file");
        assert_eq!(*written.lock().expect("the record"), [again], "the session's own services are back");
    }

    /// What the runner's writer writes straight to a name is only the frames it was given: any other
    /// path it is asked to store goes to the services it wraps, and nothing of it is written in place.
    #[test]
    fn frame_writes_writes_only_the_frames_it_listed_and_passes_any_other_path_on() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let listed = dir.path().join(frame_name(0));
        let other = dir.path().join("not-a-frame.txt");
        let recorder = Arc::<Recorder>::default();
        let writes = FrameWrites { inner: recorder.clone(), frames: [listed.clone()].into_iter().collect() };
        writes.store_file(&listed.to_string_lossy(), b"frame").expect("the listed frame is written");
        writes.store_file(&other.to_string_lossy(), b"other").expect("the other path is passed on");
        assert_eq!(std::fs::read(&listed).expect("the frame reads"), b"frame", "the listed frame was not written in place");
        assert!(!other.exists(), "a path that was not listed was written in place");
        assert_eq!(*recorder.stored.lock().expect("the recorder"), [other.to_string_lossy()], "the other path did not reach the services the writer wraps");
    }

    /// The session has its own services back on every way the start can end, not only when the fill
    /// runs: here the command refused the call after the runner had wrapped the services, a text layer
    /// being one the command turns down while it reads the call.
    #[test]
    fn the_sessions_own_services_are_back_when_the_command_refuses_a_call_it_read() {
        let mut rig = rig(3);
        rig.backend.exec("layer.newText", json!({"comp": 1, "name": "T", "text": "words"})).expect("a layer of words");
        let (gate, written) = rig.watch();
        gate.open();
        let out = rig.out("fill");
        let (answer, listed) = run(&mut rig, &json!({"layer": "T", "outputDir": out}), &mut |_, _, _| true);
        assert_eq!(listed.len(), 3, "the call was listed before it was refused, as every fill is");
        assert!(!Path::new(&out).exists(), "the refused fill made no folder");
        let waited = json!({"layer": "T", "outputDir": out, "wait": true});
        assert_eq!(answer, Err(engine_says(&mut rig, &waited)), "the command refuses a layer it has no fill for");
        let marker = Path::new(&out).join(frame_name(0)).display().to_string();
        rig.backend.session().expect("a session").services.store_file(&marker, b"note").expect("the session stores a file");
        assert_eq!(*written.lock().expect("the record"), [marker], "the session's own services are not back after the refusal");
    }

    /// A link at a frame's name is replaced by the frame, as the session's own write replaces it, and
    /// what it leads to is never written.
    #[test]
    fn a_link_at_a_frames_name_is_replaced_and_never_written_through() {
        let mut rig = rig(3);
        let out = rig.out("fill");
        let target = rig.dir.path().join("target.png");
        std::fs::write(&target, b"not a frame").expect("a file a link leads to");
        std::fs::create_dir(&out).expect("the fill's folder");
        std::os::unix::fs::symlink(&target, Path::new(&out).join(frame_name(0))).expect("a link at the first frame's name");
        let (answer, _) = run(&mut rig, &json!({"layer": 1, "outputDir": out}), &mut |_, _, _| true);
        answer.expect("the fill runs");
        assert_eq!(std::fs::read(&target).expect("the file reads"), b"not a frame", "the frame was written through the link");
        let first = std::fs::symlink_metadata(Path::new(&out).join(frame_name(0))).expect("the first frame");
        assert!(first.file_type().is_file(), "the link is still there");
    }

    /// A fill that fails once its task has started, here for a layer with no hole to fill, answers the
    /// failure as the command answers it when it waits, and leaves no frame and no folder it made, as a
    /// stopped fill and a restart after a killed one do.
    #[test]
    fn a_fill_that_fails_answers_as_the_command_does_and_leaves_no_folder_it_made() {
        let mut rig = rig(3);
        rig.backend.exec("layer.newSolid", json!({"comp": 1, "name": "Plain"})).expect("a layer with no hole");
        let out = rig.out("deep/er/fill");
        let (answer, listed) = run(&mut rig, &json!({"layer": "Plain", "outputDir": out}), &mut |_, _, _| true);
        assert_eq!(listed.len(), 3, "the fill was listed, so its task started");
        let waited = json!({"layer": "Plain", "outputDir": rig.out("waited"), "wait": true});
        assert_eq!(answer, Err(engine_says(&mut rig, &waited)));
        assert!(!rig.dir.path().join("deep").exists(), "a folder the failed fill made was left");
    }

    /// The same failure for a saved project that names no folder: the default folder beside the
    /// project, and the `Fill` folder its frames' writer made above it, are taken away too.
    #[test]
    fn a_failed_fill_on_a_saved_project_leaves_no_default_folder_it_made() {
        let mut rig = rig(3);
        let project = rig.out("p.ecproj");
        rig.backend.exec("file.saveAs", json!({"path": project})).expect("the project saves");
        rig.backend.exec("layer.newSolid", json!({"comp": 1, "name": "Plain"})).expect("a layer with no hole");
        let (answer, listed) = run(&mut rig, &json!({"layer": "Plain"}), &mut |_, _, _| true);
        assert_eq!(listed.len(), 3, "the fill was listed, so its task started");
        assert!(answer.is_err(), "a fill with nothing to fill fails: {answer:?}");
        assert!(!rig.dir.path().join("Fill").exists(), "the failed fill left its default folder");
    }

    /// A fill that failed is taken away as the call made it, and only that: a frame that was there
    /// stays, and so does a folder that was.
    #[test]
    fn a_failure_takes_away_the_frames_the_call_made_and_no_other() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let made = |name: &str| dir.path().join(name);
        let plan = |folder: PathBuf| Plan { dir: folder, frames: 3, given: Vec::new() };
        // A folder the fill made: its frames and the folder go.
        let fresh = plan(made("fresh"));
        let before = fresh.before();
        std::fs::create_dir(made("fresh")).expect("the fill's folder");
        for file in fresh.files() {
            std::fs::write(file, b"frame").expect("a frame");
        }
        fresh.withdraw(&before);
        assert!(!made("fresh").exists());
        // A folder that was there with a frame of an earlier fill: that frame and the folder stay.
        let old = plan(made("old"));
        std::fs::create_dir(made("old")).expect("an old folder");
        std::fs::write(made("old").join(frame_name(1)), b"earlier").expect("an earlier frame");
        std::fs::write(made("old").join("keep.txt"), b"other").expect("another file");
        let before = old.before();
        for file in old.files() {
            std::fs::write(file, b"frame").expect("a frame");
        }
        old.withdraw(&before);
        let left: BTreeSet<String> = files_below(&made("old").display().to_string())
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, BTreeSet::from([frame_name(1), "keep.txt".to_string()]));
        assert_eq!(
            std::fs::read(made("old").join(frame_name(1))).expect("the earlier frame"),
            b"frame",
            "it was written over by the fill and stays as it is now"
        );
    }
}
