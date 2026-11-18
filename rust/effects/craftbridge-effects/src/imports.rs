//! The commands that write beside the file they read (AD22), made to read a copy inside the container
//! when the folder that holds the file may not be written.
//!
//! `file.importLottie` writes each image a Lottie file embeds as `<name>_<asset id>.<ext>` beside the
//! file, and `layer.create` with `shapesFromVector` writes each image of a vector file as
//! `<file> Image <n>.png` beside it. A file in the app bundle would have them land in the bundle: on
//! iOS the Lottie import drops an image whose write fails without a word and the vector conversion
//! skips it, and on Android, where the bundle's unpacked assets can be written, they escape the
//! confinement. So the file is copied first, through the one staging a template goes through
//! ([`staging::stage`]), and the call reads the copy, beside which its writes land inside the
//! container.
//!
//! [`staged`] is the one door on every way a call reaches either command: `exec` and each step of
//! `run` ([`crate::methods::execute`]), each tool call of `mcp` and each line of `replay`, and inside
//! any of them each step of an `engine.batch`, read by the one reader of the commands a call reaches
//! that the door withholding commands reads too ([`crate::methods::reached`]). It runs before the
//! call, since the engine runs a batch's steps, a tool's and a replay's itself, and hands the call the
//! copy of each file the call may write beside that lies where the session may not write:
//!
//! - for the Lottie import, the file its `path` names, when the import, run on a scratch session that
//!   refuses every write ([`Probe`]), tries to write beside it: a Lottie file that embeds no image is
//!   read where it is;
//! - for a conversion of vector footage, every vector file the call names or the project's footage
//!   shows, since which footage a conversion reads turns on the layers, the selection and the footage
//!   the call's earlier steps may make, which nothing can read before the call runs.
//!
//! The call then reads the copy wherever it names the source, and every footage item that shows the
//! source names the copy. Only a call the confinement lets run as it came is handed a copy: a write to
//! the source is refused there, so the source is named in such a call only where it is read, and a
//! refused call stays refused in the confinement's words.
//!
//! An image a Lottie file names by a relative path, rather than embedding it, is resolved by the
//! import against the folder of the file it reads, which for a copy is the copy's. The command line
//! names the file beside the source, and the source's folder may be read, so once the call has run
//! each footage item the copy's import made that way names that file again ([`Staged::finish`]). Such
//! an image is not copied beside the copy: the copy's folder is named by the Lottie file's bytes alone,
//! so an image that changed would be written over under a project that names it, and an image a `..`
//! leads to lies outside the copy's folder altogether.

use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use craftbridge_core::Result;
use effectcraft_automation::Backend;
use effectcraft_automation::confine::Confinement;
use effectcraft_engine::project::{ItemId, ItemKind};
use effectcraft_engine::{Services, Session, vector};
use serde_json::Value;

use crate::staging;

/// The Lottie import (`crates/engine/src/cmd/lottie.rs`:30-53 at the pin), and the parameter that
/// names the file it reads and writes beside.
const LOTTIE: &str = "file.importLottie";
const LOTTIE_PATH: &str = "path";

/// The layer conversion (`crates/engine/src/cmd/create.rs`:98-176, :250-267), the parameter that
/// names what it makes, and the one it makes from vector footage, the only one that writes.
const CONVERT: &str = "layer.create";
const CONVERT_OP: &str = "op";
const FROM_VECTOR: &str = "shapesFromVector";

/// What the door leaves for after the call it handed copies to.
#[must_use = "a Lottie copy's external images name the files the command line names only once `finish` runs"]
pub(crate) struct Staged {
    /// Whether a parameter of the call now names a copy, so a path that holds the call in a form of
    /// its own writes it out again.
    rewritten: bool,
    /// Each file a Lottie copy's import names beside the copy, with the file the command line names,
    /// in the order the call's imports make them.
    externals: Vec<(String, String)>,
}

impl Staged {
    fn none() -> Self {
        Self { rewritten: false, externals: Vec::new() }
    }

    /// Whether a parameter of the call was given a copy in place of its source.
    pub(crate) fn rewritten(&self) -> bool {
        self.rewritten
    }

    /// Once the call has run, have each footage item a Lottie copy's import made, that names a file
    /// beside the copy for an image the Lottie file names, name the file the command line names. The
    /// items are taken in the order the project made them, which is the order the call's imports ran,
    /// so two sources of one name and bytes in two folders, which share one copy, each get their own
    /// folder's file. It is no undo step: the item is the import's, as the command line makes it.
    pub(crate) fn finish(self, backend: &mut Backend) {
        let Some(session) = backend.session().filter(|_| !self.externals.is_empty()) else { return };
        let mut pending: Vec<Option<(String, String)>> = self.externals.into_iter().map(Some).collect();
        let mut named: Vec<(ItemId, String)> = Vec::new();
        for item in session.project.items.values() {
            let ItemKind::Footage(footage) = &item.kind else { continue };
            let first = pending.iter_mut().find(|slot| slot.as_ref().is_some_and(|(on_copy, _)| *on_copy == footage.path));
            if let Some((_, beside_source)) = first.and_then(Option::take) {
                named.push((item.id, beside_source));
            }
        }
        if named.is_empty() {
            return;
        }
        let project = Arc::make_mut(&mut session.project);
        for (id, path) in named {
            if let Some(ItemKind::Footage(footage)) = project.items.get_mut(&id).map(|item| &mut item.kind) {
                footage.path = path;
            }
        }
        session.bump();
    }
}

/// One Lottie import the call reaches whose file must be read from a copy: the parameters it runs
/// with, the file, and what the import of the file itself names for each image it does not embed.
struct Lottie {
    params: Value,
    source: String,
    externals: Vec<String>,
}

/// The door: `folder` is the staged folder of the app whose call this is, where the copies go, and
/// `calls` are the commands the call names, each with the parameters it runs with, which the door
/// rewrites in place where they name a file it copies. With no folder (a command of the plugin's own,
/// or a server no app holds), and behind a bridge, which has no files of its own to stage, every call
/// goes on as it came.
pub(crate) fn staged(backend: &mut Backend, folder: Option<&Path>, calls: &mut [(&str, &mut Value)]) -> Result<Staged> {
    let mut imports: Vec<Value> = Vec::new();
    let mut converts = false;
    for (command, params) in calls.iter() {
        for (reached, given) in crate::methods::reached(command, params) {
            if reached == LOTTIE {
                imports.push(given.clone());
            }
            converts |= reached == CONVERT && converts_vector(given);
        }
    }
    if imports.is_empty() && !converts {
        return Ok(Staged::none());
    }
    let (Some(folder), Some(confinement)) = (folder, backend.confinement().cloned()) else { return Ok(Staged::none()) };
    if calls.iter().any(|(command, params)| confinement.check_command(command, params).is_err()) {
        return Ok(Staged::none());
    }
    let mut lotties: Vec<Lottie> = Vec::new();
    for params in imports {
        let Some(source) = params.get(LOTTIE_PATH).and_then(Value::as_str).filter(|source| staging::needs_copy(&confinement, source).is_some()) else {
            continue;
        };
        let source = source.to_string();
        let (written, externals) = probe(backend, &confinement, &params);
        if written.iter().any(|written| Path::new(written).parent() == Path::new(&source).parent()) {
            lotties.push(Lottie { params, source, externals });
        }
    }
    let mut sources: Vec<String> = lotties.iter().map(|lottie| lottie.source.clone()).collect();
    if converts {
        vector_files(backend, &confinement, calls, &mut sources);
    }
    let mut staged = Staged::none();
    let mut copies: Vec<(String, String)> = Vec::new();
    for source in sources {
        if copies.iter().any(|(staged_source, _)| *staged_source == source) {
            continue;
        }
        let Some(copy) = staging::stage(backend, folder, &source)? else { continue };
        for (_, params) in calls.iter_mut() {
            staged.rewritten |= substitute(params, &source, &copy);
        }
        repoint(backend, &source, &copy);
        copies.push((source, copy));
    }
    for lottie in lotties.into_iter().filter(|lottie| !lottie.externals.is_empty()) {
        let Some((_, copy)) = copies.iter().find(|(source, _)| *source == lottie.source) else { continue };
        let mut params = lottie.params;
        substitute(&mut params, &lottie.source, copy);
        let (written, on_copy) = probe(backend, &confinement, &params);
        // The probe on the copy makes the same items in the same order as the probe on the source,
        // since both refuse every write: an image named where the import also writes one is the
        // written image, beside the copy as beside the source on the command line.
        let pairs = on_copy.into_iter().zip(lottie.externals).filter(|(on_copy, beside_source)| on_copy != beside_source && !written.contains(on_copy));
        staged.externals.extend(pairs);
    }
    Ok(staged)
}

/// Whether a `layer.create` makes shapes from vector footage: its `op` is that one, or a `$`
/// reference to an earlier step's result, which a batch fills in only as it runs. Any other op writes
/// nothing beside its source, and one that is not a string is refused by the command.
fn converts_vector(params: &Value) -> bool {
    params.get(CONVERT_OP).and_then(Value::as_str).is_some_and(|op| op == FROM_VECTOR || op.starts_with('$'))
}

/// Add to `sources` every vector file, as the engine tells one ([`vector::is_vector`]), that must be
/// read from a copy ([`staging::needs_copy`]) and that the project's footage shows or a string of the
/// call names: the import a conversion's footage may come from is a step of the same call.
fn vector_files(backend: &mut Backend, confinement: &Confinement, calls: &[(&str, &mut Value)], sources: &mut Vec<String>) {
    let Some(session) = backend.session() else { return };
    let mut add = |path: &str| {
        if !sources.iter().any(|known| known == path) {
            sources.push(path.to_string());
        }
    };
    for item in session.project.items.values() {
        if let ItemKind::Footage(footage) = &item.kind
            && vector::is_vector(footage)
            && staging::needs_copy(confinement, &footage.path).is_some()
        {
            add(&footage.path);
        }
    }
    let Some(importer) = session.importer.clone() else { return };
    for (_, params) in calls {
        each_string(params, &mut |text| {
            if staging::needs_copy(confinement, text).is_some() && importer.probe(text).is_ok_and(|footage| vector::is_vector(&footage)) {
                add(text);
            }
        });
    }
}

/// Call `each` with every string `value` holds. The depth is the parse's: every call's parameters
/// come from JSON text, which `serde_json` reads to a bounded depth.
fn each_string(value: &Value, each: &mut dyn FnMut(&str)) {
    match value {
        Value::String(text) => each(text),
        Value::Array(values) => values.iter().for_each(|value| each_string(value, each)),
        Value::Object(fields) => fields.values().for_each(|value| each_string(value, each)),
        _ => {}
    }
}

/// Have every string of `value` that is `source` be `copy`; whether any was.
fn substitute(value: &mut Value, source: &str, copy: &str) -> bool {
    match value {
        Value::String(text) if text == source => {
            *text = copy.to_string();
            true
        }
        Value::Array(values) => values.iter_mut().fold(false, |any, value| substitute(value, source, copy) | any),
        Value::Object(fields) => fields.values_mut().fold(false, |any, value| substitute(value, source, copy) | any),
        _ => false,
    }
}

/// The files the Lottie import with `params` tries to write, and the files the footage it makes
/// shows, found by running it once on a scratch session ([`scratch`]). Nothing is written: an embedded
/// image whose write is refused is dropped, so each footage item the probe makes is an image the Lottie
/// file names, in the order the import makes them. The import's own failure is the real run's to answer.
fn probe(backend: &mut Backend, confinement: &Confinement, params: &Value) -> (Vec<String>, Vec<String>) {
    let Some(session) = backend.session() else { return (Vec::new(), Vec::new()) };
    let probe = Arc::new(Probe { inner: session.services.clone(), writes: Mutex::new(Vec::new()) });
    let mut scratch = scratch(session, probe.clone(), confinement);
    let made = scratch.exec(LOTTIE, params.clone()).ok();
    let ids = made.as_ref().and_then(|made| made.get("items")).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64);
    let project = scratch.session().map(|scratch| scratch.project.clone());
    let shown = ids
        .filter_map(|id| match project.as_ref().and_then(|project| project.items.get(&ItemId(id))).map(|item| &item.kind) {
            Some(ItemKind::Footage(footage)) => Some(footage.path.clone()),
            _ => None,
        })
        .collect();
    (noted(&probe.writes), shown)
}

/// A scratch session that shares the project, the editor state and the file service of `session`,
/// through `probe`, and is confined as `session` is, so the confinement checks every command and every
/// read the scratch makes as it checks the real session's, and the probe sees what the session may
/// see and no more.
fn scratch(session: &Session, probe: Arc<Probe>, confinement: &Confinement) -> Backend {
    let mut scratch = Session::new();
    scratch.services = probe;
    scratch.project = session.project.clone();
    scratch.state = session.state.clone();
    let mut scratch = Backend::headless(scratch);
    scratch.confine(confinement.clone());
    scratch
}

/// Make every footage item that shows `source` show `copy`. It is no undo step, since the item shows
/// the same bytes, but the project is changed, as it should be saved: a saved project then names a
/// file inside the container, which an update of the bundle does not move.
fn repoint(backend: &mut Backend, source: &str, copy: &str) {
    let Some(session) = backend.session() else { return };
    let named = |kind: &ItemKind| matches!(kind, ItemKind::Footage(footage) if footage.path == source);
    if !session.project.items.values().any(|item| named(&item.kind)) {
        return;
    }
    for item in Arc::make_mut(&mut session.project).items.values_mut() {
        if let ItemKind::Footage(footage) = &mut item.kind
            && footage.path == source
        {
            footage.path = copy.to_string();
        }
    }
    session.bump();
}

/// The file service beneath the scratch session's confinement: a file is read by the real session's
/// service, and both ways a command writes are noted and refused, as a folder the session may not
/// write refuses them, keeping a file among them, whose own way makes the file's folder first. What
/// else a command asks of the files is the trait's own answer, as it is the session's own service's,
/// past the confinement's check.
struct Probe {
    inner: Arc<dyn Services>,
    writes: Mutex<Vec<String>>,
}

impl Probe {
    fn refuse(&self, path: &str) -> io::Error {
        match self.writes.lock() {
            Ok(mut held) => held.push(path.to_string()),
            Err(poisoned) => poisoned.into_inner().push(path.to_string()),
        }
        io::Error::new(io::ErrorKind::PermissionDenied, "a probe writes nothing")
    }
}

/// What a list holds now; a list whose lock a panic poisoned holds what was noted before it.
fn noted(list: &Mutex<Vec<String>>) -> Vec<String> {
    list.lock().map_or_else(|poisoned| poisoned.into_inner().clone(), |held| held.clone())
}

impl Services for Probe {
    fn read_file(&self, path: &str) -> io::Result<Vec<u8>> {
        self.inner.read_file(path)
    }

    fn read_file_bounded(&self, path: &str, max: u64) -> io::Result<Vec<u8>> {
        self.inner.read_file_bounded(path, max)
    }

    fn write_file(&self, path: &str, _: &[u8]) -> io::Result<()> {
        Err(self.refuse(path))
    }

    fn store_file(&self, path: &str, _: &[u8]) -> io::Result<()> {
        Err(self.refuse(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file service that answers every read as no file at all and keeps what it is asked to write,
    /// so a probe over it shows whether a write reached it.
    #[derive(Default)]
    struct Nothing {
        written: Mutex<Vec<String>>,
    }

    impl Services for Nothing {
        fn read_file(&self, _: &str) -> io::Result<Vec<u8>> {
            Err(io::Error::new(io::ErrorKind::NotFound, "nothing"))
        }

        fn write_file(&self, path: &str, _: &[u8]) -> io::Result<()> {
            self.written.lock().expect("the list").push(path.to_string());
            Ok(())
        }
    }

    /// The scratch session reads a file through the service beneath and sees only what the
    /// confinement lets the session see, and its probe refuses both ways a command writes without the
    /// service or the disk seeing them: no folder is made for a file it is asked to keep.
    #[test]
    fn the_scratch_session_reads_as_the_session_may_and_writes_nothing() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let real = dir.path().canonicalize().expect("the folder resolves");
        let (inside, outside) = (real.join("inside"), real.join("outside"));
        for folder in [&inside, &outside] {
            std::fs::create_dir_all(folder).expect("a folder");
            std::fs::write(folder.join("there.txt"), b"there").expect("a file");
        }
        let confinement = Confinement::new(std::slice::from_ref(&inside), &[]).expect("the confinement");
        let beneath = Arc::new(Nothing::default());
        let probe = Arc::new(Probe { inner: beneath.clone(), writes: Mutex::new(Vec::new()) });
        let mut scratch = scratch(&Session::new(), probe.clone(), &confinement);
        let services = scratch.session().expect("a session").services.clone();
        let file = |folder: &Path| folder.join("there.txt").display().to_string();
        assert!(services.read_file(&file(&inside)).is_err(), "a file is read by the service beneath");
        assert!(services.exists(&file(&inside)) && services.is_dir(&inside.display().to_string()), "what the session may read is seen");
        assert!(!services.exists(&file(&outside)) && services.list_dir(&outside.display().to_string()).is_empty(), "nothing else is");
        let kept = inside.join("made").join("kept.png").display().to_string();
        assert!(services.write_file(&file(&inside), b"x").is_err() && services.store_file(&kept, b"x").is_err());
        assert_eq!(noted(&probe.writes), vec![file(&inside), kept], "each write is noted");
        assert!(noted(&beneath.written).is_empty(), "no write reached the service");
        assert!(!inside.join("made").exists(), "no folder was made");
    }

    /// Every string the parameters hold that is the source, at any depth, becomes the copy, and no
    /// other string changes, not one that only starts with the source's name.
    #[test]
    fn the_copy_takes_the_place_of_every_string_that_is_the_source() {
        let mut params = serde_json::json!({"path": "/b/a.pdf", "steps": [{"params": {"paths": ["/b/a.pdf", "/b/a.pdf.png"]}}], "name": "/b/a"});
        assert!(substitute(&mut params, "/b/a.pdf", "/s/1/a.pdf"));
        assert_eq!(params, serde_json::json!({"path": "/s/1/a.pdf", "steps": [{"params": {"paths": ["/s/1/a.pdf", "/b/a.pdf.png"]}}], "name": "/b/a"}));
        assert!(!substitute(&mut params, "/b/other.pdf", "/s/2/other.pdf"), "nothing names another file");
    }

    /// A conversion writes beside its source only under `shapesFromVector`, or when a batch fills its
    /// op in as it runs; every other op, and one that is missing or not a string, writes nothing.
    #[test]
    fn only_a_conversion_from_vector_footage_counts_as_one() {
        for op in [serde_json::json!("shapesFromVector"), serde_json::json!("$1.op")] {
            assert!(converts_vector(&serde_json::json!({ "op": op })), "{op}");
        }
        for params in
            [serde_json::json!({"op": "editableText"}), serde_json::json!({"op": "ShapesFromVector"}), serde_json::json!({"op": 1}), serde_json::json!({})]
        {
            assert!(!converts_vector(&params), "{params}");
        }
    }
}
