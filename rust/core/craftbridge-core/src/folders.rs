//! The folders an engine starts with, and the confinement they make.
//!
//! The app hands these plain paths over the boundary (plan §14): a container it owns, its bundle,
//! the working folders the engine writes inside the container, and the folders it only reads.
//! [`Folders::prepare`] creates the container and the working folders that are missing and checks
//! the rest, and [`Folders::confinement`] derives the read roots and the write root of the plan's
//! L5: the engine reads only inside the container, the bundle and paths the app passes, and writes
//! only inside the container. The roots stay plain paths — the app plugin hands them to the engine
//! behind it (plan §19, EffectCraft's own `Backend::confine`), which is what refuses each file access
//! outside them; refusing accesses here as well would be a second confinement to keep in step. The
//! one file access the core makes on its own account is the pending-export journal's (`crate::journal`),
//! which deletes files by name at start, so the core checks those paths itself ([`Confined::written`],
//! [`Confined::entry`]).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::iter;
use std::path::{Component, Path, PathBuf};

use crate::error::CraftError;

/// The folders the app names when it starts an engine: the boundary's record (plan §14) field for
/// field, its strings taken as paths.
///
/// Every path must be absolute: the engine's working directory is not the app's, so a relative root
/// would mean a different folder depending on where the process happens to sit, and a confinement
/// that moves with the working directory confines nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folders {
    /// The app's sandbox: the engine's read and write root.
    pub container: PathBuf,
    /// The app bundle (`iOS`) or its unpacked assets (`Android`): a read root, or none.
    pub bundle: Option<PathBuf>,
    /// Where the engine keeps its settings; it writes there, so it lies inside the container.
    pub config: PathBuf,
    /// Where the engine keeps what it can throw away; it writes there, so it lies inside the container.
    pub cache: PathBuf,
    /// Where the engine keeps models it has installed; it writes there, so it lies inside the container.
    pub models: PathBuf,
    /// Where the engine keeps the copies it makes of sources it imports from folders it may not
    /// write; it writes there, so it lies inside the container. The app names a folder the system
    /// keeps, never one under the cache, which the system may empty while the app is not running: a
    /// project saved after such an import names what was copied here, and must find it when it is
    /// opened again.
    pub staged: PathBuf,
    /// Where the app keeps its plug-ins, or none while they are switched off (PD13). The engine only
    /// reads it, and on iOS it lies inside the read-only bundle, so it must already be there.
    pub plugins: Option<PathBuf>,
    /// Extra font folders the app names, after the system's own; the engine only reads them.
    pub fonts: Vec<PathBuf>,
}

impl Folders {
    /// What each working folder is called, in an error about it and in the name [`Folders::inside`]
    /// gives its folder, in the order [`Folders::working`] lists them.
    const WORKING: [&str; 4] = ["config", "cache", "models", "staged"];

    /// The folders of an app that keeps every working folder directly inside its container, each at
    /// `<prefix><name>` for the name [`Folders::working`] calls it by (a prefix of `.` keeps them out
    /// of a plain listing of the container), with no bundle, no plug-in folder and no extra font
    /// folder, which a caller that has them sets on the result. A host that runs the engine over one
    /// folder lays its working folders out through this, so a folder added to the list is laid out
    /// for every such host.
    #[must_use]
    pub fn inside(container: PathBuf, prefix: &str) -> Self {
        let [config, cache, models, staged] = Self::WORKING.map(|name| container.join(format!("{prefix}{name}")));
        Self { container, bundle: None, config, cache, models, staged, plugins: None, fonts: Vec::new() }
    }

    /// The confinement these folders make (L5): the container, and the bundle when there is one, to
    /// read; the container alone to write. A pure derivation: it reads nothing from the disk, so it
    /// can be taken before the folders are prepared.
    #[must_use]
    pub fn confinement(&self) -> Confinement {
        let mut read_roots = vec![self.container.clone()];
        if let Some(bundle) = &self.bundle {
            read_roots.push(bundle.clone());
        }
        Confinement::new(read_roots, self.container.clone())
    }

    /// Make the confinement usable before an engine starts (plan §16, behaviour 1): create the
    /// container and the working folders that are missing, confirm the container can be read, each
    /// working folder resolves inside the container and can be read and written, and every folder the
    /// engine only reads is a directory it can read. A refusal is a
    /// `CraftError` from the mapping: `InvalidParams` for a path that cannot name a folder at all,
    /// `Io` for a folder that is not there, is not one, cannot be used, or lies outside the container
    /// it must be in; nothing here panics on hostile paths, and nothing is created before the path it
    /// is made at is known to be inside the container, as the disk stands when it is checked.
    ///
    /// The engine's writes inside the container are proven by the working folders' probes, which all
    /// lie inside it, and by no file of the container's own: the top of a sandbox may refuse a new file
    /// while the folders in it take one, as an iOS app's does, and it is still the root every folder an
    /// app writes in lies under — its documents and its temporary files beside the working folders — so
    /// a probe of the top would refuse a container the engine can use.
    ///
    /// That check is made by path, so a folder replaced by a link out of the container between the
    /// check and the creation is followed. Only something writing in the container at that moment can
    /// do that, and the same writer can repoint the folder once `prepare` has returned, so no check
    /// made at start could hold against it.
    pub fn prepare(&self) -> Result<Confinement, CraftError> {
        let working = self.working();
        let read_only = self.read_only();
        for (what, path) in iter::once(("container", self.container.as_path())).chain(working).chain(read_only.iter().copied()) {
            names_a_path(&format!("{what} folder"), path)?;
        }
        let container = resolved("container folder", &self.container)?;
        ensure_dir("container", &container)?;
        for (what, path) in working {
            let target = resolved(&format!("{what} folder"), path)?;
            if !target.starts_with(&container) {
                return Err(CraftError::io(format!(
                    "the {what} folder {} is outside the container {}: the engine writes only inside its container",
                    path.display(),
                    self.container.display()
                )));
            }
            // The folder made is the one that was checked, spelt as the check resolved it: made from
            // the app's own spelling it could differ (a trailing `.` makes `mkdir` fail outright).
            ensure_dir(what, &target)?;
            prove_writable(what, &target)?;
        }
        for (what, path) in read_only {
            readable_dir(what, path)?;
        }
        Ok(self.confinement())
    }

    /// The working folders, with what to call each in an error about it: config, cache, models and
    /// staged — the ones the engine writes, which lie inside the container, the one write root. This
    /// is the one list of them: [`Folders::prepare`] creates and checks every folder it names, and
    /// whatever else deals with the folders the engine writes, a plugin or a test, takes them from
    /// here, so a folder added to the list is reached by every one of them.
    #[must_use]
    pub fn working(&self) -> [(&'static str, &Path); 4] {
        let [config, cache, models, staged] = Self::WORKING;
        [(config, self.config.as_path()), (cache, self.cache.as_path()), (models, self.models.as_path()), (staged, self.staged.as_path())]
    }

    /// The folders the engine only reads, with what to call each in an error about it.
    fn read_only(&self) -> Vec<(&'static str, &Path)> {
        let bundle = self.bundle.iter().map(|bundle| ("bundle", bundle.as_path()));
        let plugins = self.plugins.iter().map(|plugins| ("plugins", plugins.as_path()));
        let fonts = self.fonts.iter().map(|font| ("font", font.as_path()));
        bundle.chain(plugins).chain(fonts).collect()
    }
}

/// The read roots and the write root the confinement allows (L5, and plan §19's call that makes
/// the engine's own confinement from these same roots).
///
/// This type carries the paths; refusing an access outside them is what the engine behind the
/// plugin does with them, not something done twice in the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confinement {
    read_roots: Vec<PathBuf>,
    write_root: PathBuf,
}

impl Confinement {
    /// The roots as given: `read_roots` the folders that may be read, which [`Folders::confinement`]
    /// lists container first, then the bundle; `write_root` the one folder everything of the session
    /// writes under.
    #[must_use]
    pub fn new(read_roots: Vec<PathBuf>, write_root: PathBuf) -> Self {
        Self { read_roots, write_root }
    }

    /// The folders this confinement lets the engine read inside: for [`Folders::confinement`], the
    /// container and the bundle when there is one.
    #[must_use]
    pub fn read_roots(&self) -> &[PathBuf] {
        &self.read_roots
    }

    /// The one folder the engine may write inside: for [`Folders::confinement`], the container.
    #[must_use]
    pub fn write_root(&self) -> &Path {
        &self.write_root
    }

    /// The write root as the operating system resolves it (links followed), the one spelling every
    /// path checked against it is compared in: an app's container is often named through a link
    /// (`/var` is `/private/var` on Apple's systems).
    pub(crate) fn resolved_write_root(&self) -> Result<PathBuf, CraftError> {
        resolved("container folder", &self.write_root)
    }
}

/// The files an engine writes or deletes, with each folder of theirs looked at once.
///
/// [`resolved`] costs the disk a resolution of every name above the one it is asked of, and an export
/// lists every file it may write before it writes any, so a sequence of ten thousand frames in one
/// folder would ask of the ten thousandth exactly what it asked of the first. Where a name lands is
/// its folder's resolution with the name below it unless a link stands at the name, so each folder is
/// resolved once and its answer kept here, and only a link at a name, which only the name can say
/// where it points, is resolved on its own ([`Confined::written`]).
#[derive(Default)]
pub(crate) struct Confined {
    /// Each folder already resolved, by the path it was asked under.
    folders: RefCell<BTreeMap<PathBuf, Real>>,
}

/// One folder already resolved: where it is with its links followed, and whether something stands
/// there that is no folder, which is where nothing below it can be put.
#[derive(Clone)]
struct Real {
    at: PathBuf,
    blocked: bool,
}

impl Confined {
    /// Where `path`, a file about to be written, lands below `root` (a resolved write root): resolved
    /// as the writer's open will resolve it (links followed, `..` taken; see [`resolved`]), so a
    /// spelling that climbs out of the root or runs through a link out of it is refused as the file it
    /// reaches, and the root itself refused as no file. This is the whole resolution, name and all, for
    /// a name a link may stand at; [`Confined::landing`] is the folder's half, kept for every name
    /// beside it.
    pub(crate) fn written(&self, root: &Path, path: &Path) -> Result<PathBuf, CraftError> {
        names_a_path("output file", path)?;
        let target = resolved("output file", path)?;
        match target.strip_prefix(root) {
            Ok(below) if !below.as_os_str().is_empty() => Ok(below.to_path_buf()),
            _ => Err(outside_the_container(path, root)),
        }
    }

    /// Where the file `path` lands below `root` when no link stands at its own name: its folder
    /// resolved, once however many files lie in it, with the name below it as it is spelt, the file
    /// [`Confined::written`] reaches for such a name. `None` when the name must be resolved on its own
    /// instead — something that is no folder stands where its folder should be, the folder lands
    /// outside `root`, or there is no name to ask — which is for [`Confined::written`] to answer, as it
    /// refuses what lands outside the root.
    pub(crate) fn landing(&self, root: &Path, path: &Path) -> Result<Option<PathBuf>, CraftError> {
        names_a_path("output file", path)?;
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return Ok(None) };
        let folder = self.folder("output file's folder", parent)?;
        if folder.blocked {
            return Ok(None);
        }
        Ok(folder.at.strip_prefix(root).ok().map(|below| below.join(name)))
    }

    /// Where the entry `relative` names under `root` (a resolved write root), to be deleted: its
    /// folder resolved, so a folder that is a link out of the root is refused as the folder it
    /// reaches, and its own name kept as it is, never followed, so a file someone replaced with a
    /// link is looked at as the link and nothing it points at is reached. `relative` is a plain name
    /// below the root, every part a name: an absolute path, `..`, or nothing at all names no file the
    /// root holds and is refused unread. Each folder is resolved once for the entries beside it.
    pub(crate) fn entry(&self, root: &Path, relative: &Path) -> Result<PathBuf, CraftError> {
        let plain = !relative.as_os_str().is_empty() && relative.components().all(|part| matches!(part, Component::Normal(_)));
        let (true, Some(name), Some(parent)) = (plain, relative.file_name(), root.join(relative).parent().map(Path::to_path_buf)) else {
            return Err(CraftError::io(format!(
                "the entry {} is not a path inside the container {}: it must be a plain name below it",
                relative.display(),
                root.display()
            )));
        };
        let folder = self.folder("output file's folder", &parent)?;
        if !folder.at.starts_with(root) {
            return Err(outside_the_container(relative, root));
        }
        Ok(folder.at.join(name))
    }

    /// Where `folder` really is, its links followed, looked at once however many paths ask for it.
    fn folder(&self, subject: &str, folder: &Path) -> Result<Real, CraftError> {
        if let Some(known) = self.folders.borrow().get(folder) {
            return Ok(known.clone());
        }
        let real = resolved(subject, folder)?;
        let known = Real { blocked: fs::metadata(&real).is_ok_and(|found| !found.is_dir()), at: real };
        self.folders.borrow_mut().insert(folder.to_path_buf(), known.clone());
        Ok(known)
    }
}

/// The refusal of a file the engine would write or delete outside its write root.
fn outside_the_container(path: &Path, root: &Path) -> CraftError {
    CraftError::io(format!(
        "the output file {} is outside the container {}: the engine writes and deletes only inside its container",
        path.display(),
        root.display()
    ))
}

/// Refuse a path that names nothing wherever the engine runs: an empty one, one holding a NUL byte
/// (no operating system's file calls take one, so it is the caller's mistake, not the disk's), and a
/// relative one. `subject` is what the path names, as the message says it ("the container folder").
fn names_a_path(subject: &str, path: &Path) -> Result<(), CraftError> {
    if path.as_os_str().is_empty() {
        return Err(CraftError::invalid_params(format!("the {subject} path is empty")));
    }
    if path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(CraftError::invalid_params(format!("the {subject} path holds a NUL byte: {}", path.display())));
    }
    if !path.is_absolute() {
        return Err(CraftError::invalid_params(format!("the {subject} path must be absolute: {}", path.display())));
    }
    Ok(())
}

/// Where `path` is, or will be once its missing folders are made: its nearest folder that exists,
/// resolved as the operating system resolves it (symbolic links followed, `..` taken), with the
/// missing names after it. A missing name cannot be `..`, since there is no folder yet to step out
/// of, so such a path is refused rather than guessed at. `subject` is what the path names, as the
/// message says it ("the container folder").
pub(crate) fn resolved(subject: &str, path: &Path) -> Result<PathBuf, CraftError> {
    let unresolved = |error: io::Error| CraftError::io(format!("cannot resolve the {subject} {}: {error}", path.display()));
    let mut missing = Vec::new();
    let mut nearest = path;
    loop {
        match fs::canonicalize(nearest) {
            Ok(real) => return Ok(missing.iter().rev().fold(real, |real, name| real.join(name))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let (Some(name), Some(parent)) = (nearest.file_name(), nearest.parent()) else {
                    return Err(unresolved(error));
                };
                missing.push(name);
                nearest = parent;
            }
            Err(error) => return Err(unresolved(error)),
        }
    }
}

/// Make a folder the engine writes inside, the container or a working folder, be there: create it
/// when missing, and read the directory.
fn ensure_dir(what: &'static str, path: &Path) -> Result<(), CraftError> {
    fs::create_dir_all(path).map_err(|error| CraftError::io(format!("cannot create the {what} folder {}: {error}", path.display())))?;
    readable_dir(what, path)
}

/// Prove the engine can write in a working folder by creating and removing a probe file, because a
/// folder a start-up cannot write is a start that would fail the first save and is better failed
/// here, named. The probe is tempfile's: a fresh random name created only if nothing holds it, so two
/// engines starting at once never share one and no file the app keeps is ever opened or removed.
fn prove_writable(what: &'static str, path: &Path) -> Result<(), CraftError> {
    let unwritable = |error: io::Error| CraftError::io(format!("cannot write to the {what} folder {}: {error}", path.display()));
    tempfile::Builder::new().prefix(".craftbridge-write-probe-").tempfile_in(path).map_err(unwritable)?.close().map_err(unwritable)
}

/// A folder the engine reads: it is there, it is a directory, and its list can be opened.
fn readable_dir(what: &'static str, path: &Path) -> Result<(), CraftError> {
    let metadata = fs::metadata(path).map_err(|error| CraftError::io(format!("cannot find the {what} folder {}: {error}", path.display())))?;
    if !metadata.is_dir() {
        return Err(CraftError::io(format!("the {what} folder {} is not a directory", path.display())));
    }
    fs::read_dir(path).map_err(|error| CraftError::io(format!("cannot read the {what} folder {}: {error}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::ffi::OsString;
    use tempfile::TempDir;

    /// A scratch sandbox: a root folder that deletes itself when the test ends, with the folders of
    /// the app's first launch under it — the working folders inside the container, as an app names
    /// them — none of them created yet unless a test says so.
    struct Sandbox {
        root: TempDir,
        folders: Folders,
    }

    /// A test's result: any failure of what the test does — to the disk or through `prepare` — ends
    /// it with that error.
    type Test = Result<(), Box<dyn std::error::Error>>;

    fn sandbox() -> Result<Sandbox, std::io::Error> {
        let root = TempDir::new()?;
        let container = root.path().join("container");
        Ok(Sandbox { root, folders: Folders::inside(container, "") })
    }

    /// The names a folder holds, so a test can say nothing else was left in it.
    fn names(folder: &Path) -> Result<BTreeSet<OsString>, std::io::Error> {
        fs::read_dir(folder)?.map(|entry| entry.map(|entry| entry.file_name())).collect()
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) -> Result<(), std::io::Error> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }

    #[test]
    fn prepare_creates_the_missing_container_folders() -> Test {
        let sand = sandbox()?;
        let confinement = sand.folders.prepare()?;
        assert_eq!(confinement.write_root(), sand.folders.container.as_path());
        assert_eq!(confinement.read_roots(), std::slice::from_ref(&sand.folders.container));
        for folder in iter::once(sand.folders.container.as_path()).chain(sand.folders.working().map(|(_, path)| path)) {
            assert!(folder.is_dir(), "the engine start would find {} missing", folder.display());
        }
        Ok(())
    }

    #[test]
    fn prepare_leaves_a_folder_it_found_as_it_was() -> Test {
        let sand = sandbox()?;
        fs::create_dir_all(&sand.folders.container)?;
        fs::write(sand.folders.container.join("project.ecproj"), b"kept")?;
        sand.folders.prepare()?;
        sand.folders.prepare()?;
        assert_eq!(fs::read(sand.folders.container.join("project.ecproj"))?, b"kept");
        // The write probes are gone: the container holds what the app put there and the working
        // folders, and each working folder is as empty as `prepare` made it.
        let expected: BTreeSet<OsString> = ["cache", "config", "models", "project.ecproj", "staged"].into_iter().map(OsString::from).collect();
        assert_eq!(names(&sand.folders.container)?, expected);
        for folder in sand.folders.working().map(|(_, path)| path) {
            assert_eq!(names(folder)?, BTreeSet::new(), "{} kept a probe", folder.display());
        }
        Ok(())
    }

    #[test]
    fn engines_starting_at_once_all_start() -> Test {
        let sand = sandbox()?;
        sand.folders.prepare()?;
        let refused: usize = std::thread::scope(|scope| {
            let starts: Vec<_> = (0..8).map(|_| scope.spawn(|| (0..200).filter(|_| sand.folders.prepare().is_err()).count())).collect();
            starts.into_iter().map(|start| start.join().expect("a start thread panicked")).sum()
        });
        assert_eq!(refused, 0, "engines starting together refused one another's starts");
        Ok(())
    }

    /// Every working folder `inside` lays out sits directly in the container, at the name `working`
    /// calls it by after the prefix: the two read the one list of names, in the one order.
    #[test]
    fn inside_lays_each_working_folder_out_at_the_name_working_calls_it() {
        let container = PathBuf::from("/sandbox/container");
        for prefix in ["", "."] {
            let folders = Folders::inside(container.clone(), prefix);
            for (what, path) in folders.working() {
                assert_eq!(path, container.join(format!("{prefix}{what}")), "{what}");
            }
            assert_eq!((&folders.container, &folders.bundle, &folders.plugins, folders.fonts.len()), (&container, &None, &None, 0));
        }
    }

    #[test]
    fn confinement_is_a_plain_derivation_and_needs_no_disk() {
        let folders = Folders { bundle: Some(PathBuf::from("/app/App.app")), ..Folders::inside(PathBuf::from("/sandbox/container"), "") };
        let confinement = folders.confinement();
        assert_eq!(confinement.read_roots(), [PathBuf::from("/sandbox/container"), PathBuf::from("/app/App.app")]);
        assert_eq!(confinement.write_root(), PathBuf::from("/sandbox/container").as_path());
    }

    #[test]
    fn prepare_adds_a_bundle_folder_to_the_read_roots() -> Test {
        let mut sand = sandbox()?;
        let bundle = sand.root.path().join("App.app");
        fs::create_dir_all(&bundle)?;
        fs::write(bundle.join("fonts.txt"), b"fonts")?;
        sand.folders.bundle = Some(bundle.clone());
        let confinement = sand.folders.prepare()?;
        assert_eq!(confinement.read_roots(), vec![sand.folders.container.clone(), bundle]);
        assert_eq!(confinement.write_root(), sand.folders.container.as_path());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn prepare_reads_plug_ins_inside_a_bundle_it_cannot_write() -> Test {
        let mut sand = sandbox()?;
        let bundle = sand.root.path().join("App.app");
        let plugins = bundle.join("PlugIns");
        fs::create_dir_all(&plugins)?;
        fs::write(plugins.join("blur.wasm"), b"plug-in")?;
        set_mode(&plugins, 0o555)?;
        set_mode(&bundle, 0o555)?;
        sand.folders.bundle = Some(bundle.clone());
        sand.folders.plugins = Some(plugins.clone());
        let answer = sand.folders.prepare();
        set_mode(&bundle, 0o755)?;
        set_mode(&plugins, 0o755)?;
        answer?;
        let expected: BTreeSet<OsString> = iter::once(OsString::from("blur.wasm")).collect();
        assert_eq!(names(&plugins)?, expected, "the plug-ins folder was written to");
        Ok(())
    }

    #[test]
    fn prepare_refuses_a_plug_ins_folder_that_is_not_there() -> Test {
        let mut sand = sandbox()?;
        let plugins = sand.folders.container.join("PlugIns");
        sand.folders.plugins = Some(plugins.clone());
        let error = sand.folders.prepare().expect_err("plug-ins are the app's to put there");
        assert!(matches!(error, CraftError::Io { .. }), "{error:?}");
        assert!(error.to_string().contains("plugins"));
        assert!(!plugins.exists(), "prepare made a plug-ins folder the app never filled");
        Ok(())
    }

    #[test]
    fn prepare_accepts_font_folders_it_can_read() -> Test {
        let mut sand = sandbox()?;
        let font = sand.root.path().join("MyFont");
        fs::create_dir_all(&font)?;
        sand.folders.fonts = vec![font];
        sand.folders.prepare()?;
        Ok(())
    }

    #[test]
    fn prepare_refuses_a_bundle_folder_that_is_not_there() {
        let mut sand = sandbox().expect("a scratch folder");
        sand.folders.bundle = Some(sand.root.path().join("gone.app"));
        let error = sand.folders.prepare().expect_err("the bundle is not there");
        assert!(matches!(error, CraftError::Io { .. }), "{error:?}");
        assert!(error.to_string().contains("bundle"));
    }

    #[test]
    fn prepare_refuses_a_bundle_folder_that_is_a_file() -> Test {
        let mut sand = sandbox()?;
        let bundle = sand.root.path().join("App.app");
        fs::write(&bundle, b"not a folder")?;
        sand.folders.bundle = Some(bundle);
        let error = sand.folders.prepare().expect_err("a file is not a folder");
        assert!(matches!(error, CraftError::Io { .. }), "{error:?}");
        assert!(error.to_string().contains("not a directory"));
        Ok(())
    }

    #[test]
    fn prepare_refuses_a_font_folder_that_is_not_there() {
        let mut sand = sandbox().expect("a scratch folder");
        sand.folders.fonts = vec![sand.root.path().join("gone-font")];
        let error = sand.folders.prepare().expect_err("the font folder is not there");
        assert!(matches!(error, CraftError::Io { .. }), "{error:?}");
        assert!(error.to_string().contains("font"));
    }

    #[test]
    fn prepare_refuses_a_folder_that_is_a_file() -> Result<(), std::io::Error> {
        let sand = sandbox()?;
        fs::create_dir_all(&sand.folders.container)?;
        fs::write(&sand.folders.config, b"not a folder")?;
        let error = sand.folders.prepare().expect_err("a file is not a folder");
        assert!(matches!(error, CraftError::Io { .. }), "{error:?}");
        Ok(())
    }

    /// Each working folder, set in turn outside the container, is refused as the folder it is, and
    /// nothing is made at it before the refusal: the engine writes only inside its container.
    #[test]
    fn prepare_refuses_a_working_folder_outside_the_container() -> Test {
        let fields: [(&str, fn(&mut Folders, PathBuf)); 4] = [
            ("config", |folders, path| folders.config = path),
            ("cache", |folders, path| folders.cache = path),
            ("models", |folders, path| folders.models = path),
            ("staged", |folders, path| folders.staged = path),
        ];
        for (what, set) in fields {
            let mut sand = sandbox()?;
            let elsewhere = sand.root.path().join(format!("elsewhere-{what}"));
            set(&mut sand.folders, elsewhere.clone());
            let error = sand.folders.prepare().expect_err("the engine writes only inside its container");
            assert!(matches!(error, CraftError::Io { .. }), "{what}: {error:?}");
            assert!(error.to_string().contains(what), "{what}: {error}");
            assert!(!elsewhere.exists(), "{what}: prepare wrote outside the container before refusing");
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn prepare_refuses_a_working_folder_a_link_takes_outside_the_container() -> Test {
        let sand = sandbox()?;
        let elsewhere = sand.root.path().join("elsewhere");
        fs::create_dir_all(&elsewhere)?;
        fs::create_dir_all(&sand.folders.container)?;
        std::os::unix::fs::symlink(&elsewhere, &sand.folders.models)?;
        let error = sand.folders.prepare().expect_err("the link leads out of the container");
        assert!(matches!(error, CraftError::Io { .. }), "{error:?}");
        assert!(error.to_string().contains("models"));
        assert_eq!(names(&elsewhere)?, BTreeSet::new(), "prepare wrote through the link");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn prepare_accepts_a_container_named_through_a_link() -> Test {
        let mut sand = sandbox()?;
        let real = sand.root.path().join("real-container");
        fs::create_dir_all(&real)?;
        let link = sand.root.path().join("container-link");
        std::os::unix::fs::symlink(&real, &link)?;
        // The working folders spelt through the real path and the container through the link, as an
        // app does when one of its paths came resolved and another did not.
        sand.folders.container = link;
        sand.folders.config = real.join("config");
        sand.folders.cache = real.join("cache");
        sand.folders.models = real.join("models");
        sand.folders.staged = real.join("staged");
        sand.folders.prepare()?;
        assert!(real.join("config").is_dir());
        Ok(())
    }

    /// A container nothing can be written in is refused at the first working folder it cannot hold.
    #[cfg(unix)]
    #[test]
    fn prepare_refuses_a_container_it_cannot_write() -> Result<(), std::io::Error> {
        let sand = sandbox()?;
        fs::create_dir_all(&sand.folders.container)?;
        set_mode(&sand.folders.container, 0o555)?;
        if fs::File::create(sand.folders.container.join("probe")).is_ok() {
            // The mode did not keep a file out, so this process escapes the permission bits (it is
            // the super-user, or the volume ignores them): there is nothing here to refuse.
            return Ok(());
        }
        let answer = sand.folders.prepare();
        set_mode(&sand.folders.container, 0o755)?;
        let error = answer.expect_err("the container cannot be written");
        assert!(matches!(error, CraftError::Io { .. }), "{error:?}");
        // The message names the folder as the check resolved it, the spelling it was made at.
        let config = fs::canonicalize(&sand.folders.container)?.join("config");
        assert!(error.to_string().starts_with(&format!("cannot create the config folder {}: ", config.display())), "{error}");
        Ok(())
    }

    /// An iOS app's sandbox: its top refuses a new file while the folders in it take one. The engine
    /// can use it, since its working folders lie inside and their probes pass, so the start makes no
    /// file at the top and keeps it as the write root.
    #[cfg(unix)]
    #[test]
    fn prepare_accepts_a_container_whose_top_takes_no_file_while_its_folders_do() -> Test {
        let sand = sandbox()?;
        for (_, folder) in sand.folders.working() {
            fs::create_dir_all(folder)?;
        }
        set_mode(&sand.folders.container, 0o555)?;
        if fs::File::create(sand.folders.container.join("probe")).is_ok() {
            // As above: the permission bits do not bind this process, so the top takes a file anyway.
            set_mode(&sand.folders.container, 0o755)?;
            return Ok(());
        }
        let answer = sand.folders.prepare();
        set_mode(&sand.folders.container, 0o755)?;
        assert_eq!(answer?.write_root(), sand.folders.container.as_path());
        let expected: BTreeSet<OsString> = Folders::WORKING.into_iter().map(OsString::from).collect();
        assert_eq!(names(&sand.folders.container)?, expected, "the start wrote at the container's top");
        Ok(())
    }

    /// Each working folder, made in turn so it takes no file, is refused as the folder it is: the
    /// working folders' probes are what prove the engine can write inside its container.
    #[cfg(unix)]
    #[test]
    fn prepare_refuses_a_working_folder_it_cannot_write() -> Test {
        for (index, what) in Folders::WORKING.into_iter().enumerate() {
            let sand = sandbox()?;
            let folder = sand.folders.working().get(index).map(|(_, path)| path.to_path_buf()).ok_or("no such working folder")?;
            fs::create_dir_all(&folder)?;
            set_mode(&folder, 0o555)?;
            if fs::File::create(folder.join("probe")).is_ok() {
                // As above: the permission bits do not bind this process, so there is nothing to refuse.
                set_mode(&folder, 0o755)?;
                return Ok(());
            }
            let answer = sand.folders.prepare();
            set_mode(&folder, 0o755)?;
            let error = answer.expect_err("the working folder cannot be written");
            assert!(matches!(error, CraftError::Io { .. }), "{what}: {error:?}");
            let resolved = fs::canonicalize(&folder)?;
            assert!(error.to_string().starts_with(&format!("cannot write to the {what} folder {}: ", resolved.display())), "{what}: {error}");
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn prepare_refuses_a_container_it_can_write_but_not_read() -> Result<(), std::io::Error> {
        let sand = sandbox()?;
        fs::create_dir_all(&sand.folders.container)?;
        set_mode(&sand.folders.container, 0o333)?;
        if fs::read_dir(&sand.folders.container).is_ok() {
            // As above: the permission bits do not bind this process, so there is nothing to refuse.
            set_mode(&sand.folders.container, 0o755)?;
            return Ok(());
        }
        let answer = sand.folders.prepare();
        set_mode(&sand.folders.container, 0o755)?;
        let error = answer.expect_err("the container cannot be read");
        assert!(matches!(error, CraftError::Io { .. }), "{error:?}");
        assert!(error.to_string().contains("cannot read the container"), "{error}");
        Ok(())
    }

    #[test]
    fn prepare_makes_the_folder_it_checked_whatever_the_spelling() -> Test {
        let mut sand = sandbox()?;
        sand.folders.config = sand.folders.container.join("./spelt//twice/.");
        sand.folders.cache = sand.folders.container.join("../container/cache");
        // The container too: missing, and spelt with a trailing `.` that `mkdir` refuses.
        sand.folders.container = sand.root.path().join("./container/.");
        sand.folders.prepare()?;
        assert!(sand.root.path().join("container").is_dir());
        assert!(sand.root.path().join("container/spelt/twice").is_dir());
        assert!(sand.folders.container.join("cache").is_dir());
        Ok(())
    }

    #[test]
    fn prepare_refuses_a_folder_that_cannot_be_created() -> Test {
        let mut sand = sandbox()?;
        let inside_a_file = sand.root.path().join("not-a-folder");
        fs::write(&inside_a_file, b"x")?;
        sand.folders.container = inside_a_file.join("container");
        let error = sand.folders.prepare().expect_err("nothing can be created inside a file");
        assert!(matches!(error, CraftError::Io { .. }), "{error:?}");
        Ok(())
    }

    /// Every path the app names, set in turn to one that names no folder, is refused as a bad
    /// parameter naming that folder, before anything is made on the disk.
    #[test]
    fn prepare_refuses_paths_that_cannot_name_a_place() -> Test {
        let fields: [(&str, fn(&mut Folders, PathBuf)); 8] = [
            ("container", |folders, path| folders.container = path),
            ("config", |folders, path| folders.config = path),
            ("cache", |folders, path| folders.cache = path),
            ("models", |folders, path| folders.models = path),
            ("staged", |folders, path| folders.staged = path),
            ("bundle", |folders, path| folders.bundle = Some(path)),
            ("plugins", |folders, path| folders.plugins = Some(path)),
            ("font", |folders, path| folders.fonts = vec![path]),
        ];
        let paths = [("relative/folder", "must be absolute"), ("", "is empty"), ("/sandbox/a\0b", "holds a NUL byte")];
        for (what, set) in fields {
            for (path, why) in paths {
                let mut sand = sandbox()?;
                set(&mut sand.folders, PathBuf::from(path));
                let error = sand.folders.prepare().expect_err("a path that names no folder");
                assert!(matches!(error, CraftError::InvalidParams { .. }), "{what} {path:?}: {error:?}");
                assert!(error.to_string().contains(&format!("the {what} folder path {why}")), "{what} {path:?}: {error}");
                assert!(!sand.root.path().join("container").exists(), "{what} {path:?}: prepare touched the disk first");
            }
        }
        Ok(())
    }
}
