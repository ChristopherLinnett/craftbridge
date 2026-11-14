//! The folders an engine starts with, and the confinement they make.
//!
//! The app hands these plain paths over the boundary (plan §14): a container it owns, its bundle,
//! the working folders the engine writes inside the container, and the folders it only reads.
//! [`Folders::prepare`] creates the container and the working folders that are missing and checks
//! the rest, and [`Folders::confinement`] derives the read roots and the write root of the plan's
//! L5: the engine reads only inside the container, the bundle and paths the app passes, and writes
//! only inside the container. The roots stay plain paths — the app plugin hands them to the engine
//! behind it (plan §19, EffectCraft's own `Backend::confine`), which is what refuses each file access
//! outside them; refusing accesses here as well would be a second confinement to keep in step.

use std::fs;
use std::io;
use std::iter;
use std::path::{Path, PathBuf};

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
    /// Where the app keeps its plug-ins, or none while they are switched off (PD13). The engine only
    /// reads it, and on iOS it lies inside the read-only bundle, so it must already be there.
    pub plugins: Option<PathBuf>,
    /// Extra font folders the app names, after the system's own; the engine only reads them.
    pub fonts: Vec<PathBuf>,
}

impl Folders {
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
    /// container and the working folders that are missing, confirm the container and each working
    /// folder can be read and written and each working folder resolves inside the container, and
    /// confirm every folder the engine only reads is a directory it can read. A refusal is a
    /// `CraftError` from the mapping: `InvalidParams` for a path that cannot name a folder at all,
    /// `Io` for a folder that is not there, is not one, cannot be used, or lies outside the container
    /// it must be in; nothing here panics on hostile paths, and nothing is created before the path it
    /// is made at is known to be inside the container, as the disk stands when it is checked.
    ///
    /// That check is made by path, so a folder replaced by a link out of the container between the
    /// check and the creation is followed. Only something writing in the container at that moment can
    /// do that, and the same writer can repoint the folder once `prepare` has returned, so no check
    /// made at start could hold against it.
    pub fn prepare(&self) -> Result<Confinement, CraftError> {
        let working = self.working();
        let read_only = self.read_only();
        for (what, path) in iter::once(("container", self.container.as_path())).chain(working).chain(read_only.iter().copied()) {
            names_a_folder(what, path)?;
        }
        let container = resolved("container", &self.container)?;
        ensure_dir("container", &container)?;
        for (what, path) in working {
            let target = resolved(what, path)?;
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
        }
        for (what, path) in read_only {
            readable_dir(what, path)?;
        }
        Ok(self.confinement())
    }

    /// The working folders, with what to call each in an error about it: config, cache and models,
    /// which the engine writes and which lie inside the container, the one write root.
    fn working(&self) -> [(&'static str, &Path); 3] {
        [("config", self.config.as_path()), ("cache", self.cache.as_path()), ("models", self.models.as_path())]
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
}

/// Refuse a path that names no folder wherever the engine runs: an empty one, one holding a NUL
/// byte (no operating system's file calls take one, so it is the caller's mistake, not the disk's),
/// and a relative one.
fn names_a_folder(what: &'static str, path: &Path) -> Result<(), CraftError> {
    if path.as_os_str().is_empty() {
        return Err(CraftError::invalid_params(format!("the {what} folder path is empty")));
    }
    if path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(CraftError::invalid_params(format!("the {what} folder path holds a NUL byte: {}", path.display())));
    }
    if !path.is_absolute() {
        return Err(CraftError::invalid_params(format!("the {what} folder path must be absolute: {}", path.display())));
    }
    Ok(())
}

/// Where `path` is, or will be once its missing folders are made: its nearest folder that exists,
/// resolved as the operating system resolves it (symbolic links followed, `..` taken), with the
/// missing names after it. A missing name cannot be `..`, since there is no folder yet to step out
/// of, so such a path is refused rather than guessed at.
fn resolved(what: &'static str, path: &Path) -> Result<PathBuf, CraftError> {
    let unresolved = |error: io::Error| CraftError::io(format!("cannot resolve the {what} folder {}: {error}", path.display()));
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

/// Make a folder the engine writes, the container or a working folder, be there and prove the engine
/// can use it: create it when missing, read the directory, and create and remove a probe file,
/// because a folder a start-up cannot write is a start that would fail the first save and is better
/// failed here, named. The probe is tempfile's: a fresh random name created only if nothing holds it,
/// so two engines starting at once never share one and no file the app keeps is ever opened or
/// removed.
fn ensure_dir(what: &'static str, path: &Path) -> Result<(), CraftError> {
    fs::create_dir_all(path).map_err(|error| CraftError::io(format!("cannot create the {what} folder {}: {error}", path.display())))?;
    readable_dir(what, path)?;
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
        let folders = Folders {
            config: container.join("config"),
            cache: container.join("cache"),
            models: container.join("models"),
            container,
            bundle: None,
            plugins: None,
            fonts: Vec::new(),
        };
        Ok(Sandbox { root, folders })
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
        for folder in [&sand.folders.container, &sand.folders.config, &sand.folders.cache, &sand.folders.models] {
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
        let expected: BTreeSet<OsString> = ["cache", "config", "models", "project.ecproj"].into_iter().map(OsString::from).collect();
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

    #[test]
    fn confinement_is_a_plain_derivation_and_needs_no_disk() {
        let folders = Folders {
            container: PathBuf::from("/sandbox/container"),
            bundle: Some(PathBuf::from("/app/App.app")),
            config: PathBuf::from("/sandbox/container/config"),
            cache: PathBuf::from("/sandbox/container/cache"),
            models: PathBuf::from("/sandbox/container/models"),
            plugins: None,
            fonts: Vec::new(),
        };
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

    #[test]
    fn prepare_refuses_a_working_folder_outside_the_container() -> Test {
        let mut sand = sandbox()?;
        let elsewhere = sand.root.path().join("elsewhere");
        sand.folders.cache = elsewhere.clone();
        let error = sand.folders.prepare().expect_err("the engine writes only inside its container");
        assert!(matches!(error, CraftError::Io { .. }), "{error:?}");
        assert!(error.to_string().contains("cache"));
        assert!(!elsewhere.exists(), "prepare wrote outside the container before refusing");
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
        sand.folders.prepare()?;
        assert!(real.join("config").is_dir());
        Ok(())
    }

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
        assert!(error.to_string().contains("cannot write to the container"), "{error}");
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
        let fields: [(&str, fn(&mut Folders, PathBuf)); 7] = [
            ("container", |folders, path| folders.container = path),
            ("config", |folders, path| folders.config = path),
            ("cache", |folders, path| folders.cache = path),
            ("models", |folders, path| folders.models = path),
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
