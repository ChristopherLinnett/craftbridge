//! Staging (AD5): a file the session may read from a folder it may not write is read from a copy
//! inside the container instead.
//!
//! Some of EffectCraft's imports write beside the file they read, in the folder that holds it as it
//! is named: a template's import writes its media to `<stem> Media/` there. A file in the app bundle,
//! a root the session may read and never write, would have those writes land in the bundle: on iOS
//! they fail without a word and what they carry comes out missing, and on Android, where the bundle's
//! unpacked assets can be written, they escape the confinement. So such a file is copied first into
//! the app's staged folder, inside the container, and the command reads the copy, whose writes then
//! land beside it inside the container. The copies belong to what a saved project may name, so they
//! sit where the system will not clear them as it may clear the cache. [`stage`] knows nothing of
//! what the file is: each caller names the source a command reads, and hands the command the path it
//! answers. The callers are the template runner ([`crate::templates`]) and the imports that write their
//! images beside a Lottie or a vector file ([`crate::imports`]). Each is handed the staged folder with
//! the call it serves, as a parameter passed down from the app's own call, never as state of the
//! thread or the process, so only a call of the app that owns the folder can copy into it.
//!
//! A copy is named by what it holds, never by where its source sits. An app update installs the
//! bundle at a new path on iOS, and may change a file at the same path on Android: a copy named by its
//! source's path would be made anew at every update, leaving the last one, and the media its import
//! wrote, for nothing to name again, or be written over where it was, under the media a project saved
//! before the update still names.

use std::ffi::OsString;
use std::hash::Hasher;
use std::io::Read;
use std::path::{Path, PathBuf};

use craftbridge_core::{CraftError, Result};
use effectcraft_automation::Backend;
use effectcraft_automation::confine::{Access, Confinement};
use siphasher::sip128::{Hasher128, SipHasher13};

/// The path to read `source` from: a copy inside the container when the folder that holds `source`,
/// as it is named, is one the confinement lets the session read and not write; `None` when it is read
/// where it is, because that folder may be written, or because the confinement refuses to read the
/// source or it is no regular file, which the command then says in its own words.
///
/// The writes land beside the name the command is given, not where a link at that name leads, so the
/// folder holding the name is the one asked about: a link in the bundle to a file inside the container
/// is copied too. The copy goes to `<staged>/<id>/<name>`, `<staged>` the app's staged folder, `<id>`
/// the hash of the bytes the copy holds and `<name>` the source's file name as given, so what is
/// written beside the copy is named as it would be beside the source. The same file finds the same
/// copy wherever its bundle is installed; a source that changed finds a copy of what it holds now,
/// never the copy of what it held before, which stays as it was for a project that names what its
/// import wrote; and two sources of one name share a copy only when they hold the same bytes. The copy
/// is written again on every call, whole before it takes the name ([`write_partial`]).
pub(crate) fn stage(backend: &Backend, staged: &Path, source: &str) -> Result<Option<String>> {
    let Some(confinement) = backend.confinement() else { return Ok(None) };
    let Some(resolved) = needs_copy(confinement, source) else { return Ok(None) };
    let Some(name) = Path::new(source).file_name() else { return Ok(None) };
    let refused = |error: &dyn std::fmt::Display| CraftError::io(format!("cannot copy {source} into the container: {error}"));
    let mut partial_name = OsString::from(".");
    partial_name.push(name);
    partial_name.push(".partial");
    let partial = staged.join(partial_name);
    // The copy goes only where the session may write, which the confinement says of the staged folder
    // before anything is made, so one that lay outside the write root would be refused here, not
    // written. The partial file's own name is not asked about: whatever sits there, a link included,
    // is removed and the file made new, never followed.
    confinement.check(&staged.to_string_lossy(), Access::Write).map_err(|error| refused(&error))?;
    std::fs::create_dir_all(staged).map_err(|error| refused(&error))?;
    let id = write_partial(&resolved, &partial).map_err(|error| refused(&error))?;
    let copy = staged.join(id).join(name);
    // The copy's own path is asked about too, once its name is known: a folder already there by that
    // name could be a link out of the container.
    let placed = confinement.check(&copy.to_string_lossy(), Access::Write).map_err(|error| refused(&error)).and_then(|_| {
        let made = copy.parent().map_or(Ok(()), std::fs::create_dir_all);
        made.and_then(|()| std::fs::rename(&partial, &copy)).map_err(|error| refused(&error))
    });
    if placed.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    placed.map(|()| Some(copy.to_string_lossy().into_owned()))
}

/// Where `source` is read from when [`stage`] copies it: resolved, when the confinement lets the
/// session read it, it is a regular file, and the folder holding its name may not be written; `None`
/// when it is read where it is. A caller that must know before anything is copied asks this, so the
/// rule is the one [`stage`] keeps.
pub(crate) fn needs_copy(confinement: &Confinement, source: &str) -> Option<PathBuf> {
    let resolved = confinement.check(source, Access::Read).ok().filter(|resolved| resolved.is_file())?;
    let named = Path::new(source);
    let (folder, _) = named.parent().zip(named.file_name())?;
    confinement.check(&folder.to_string_lossy(), Access::Write).is_err().then_some(resolved)
}

/// Copy `source` to `partial`, made new, and answer the name of the folder the copy belongs in: the
/// hash of exactly the bytes written, so the name says what the copy holds even should the source
/// change while it is read. The copy is a file of the copier's own making whatever the source's mode,
/// so a read-only source is copied again on the next call, where a copy that kept the source's mode
/// could not be written over; and a kill mid-copy leaves only the partial file, which the next copy of
/// a source of that name makes anew. A copy that fails takes its partial file with it.
fn write_partial(source: &Path, partial: &Path) -> std::io::Result<String> {
    match std::fs::remove_file(partial) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    // Made new, so nothing already at the name, a link least of all, is opened or followed.
    let mut written = std::fs::OpenOptions::new().write(true).create_new(true).open(partial)?;
    let copied = std::fs::File::open(source).and_then(|file| {
        let mut read = Hashing { inner: file, hash: SipHasher13::new() };
        std::io::copy(&mut read, &mut written).map(|_| read.hash.finish128())
    });
    drop(written);
    if copied.is_err() {
        let _ = std::fs::remove_file(partial);
    }
    // SipHash, keyed alike everywhere, and its bytes in a fixed order: the name a copy is given must
    // be the one the next build, on any platform, gives the same bytes.
    copied.map(|hash| format!("{:032x}", u128::from_le_bytes(hash.as_bytes())))
}

/// A reader that hashes every byte it hands on.
struct Hashing<R> {
    inner: R,
    hash: SipHasher13,
}

impl<R: Read> Read for Hashing<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buf)?;
        // A reader never answers more than the buffer holds; `get` keeps one that did from panicking.
        self.hash.write(buf.get(..count).unwrap_or_default());
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use effectcraft_automation::confine::Confinement;
    use effectcraft_engine::Session;
    use std::path::PathBuf;

    /// A confined backend over a container it may write and a bundle it may only read, with the
    /// container's staged folder, all inside a scratch folder of their own.
    struct Roots {
        _dir: tempfile::TempDir,
        container: PathBuf,
        bundle: PathBuf,
        outside: PathBuf,
        backend: Backend,
    }

    fn roots() -> Roots {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let real = dir.path().canonicalize().expect("the scratch folder resolves");
        let (container, bundle, outside) = (real.join("container"), real.join("bundle"), real.join("outside"));
        for folder in [&container, &bundle, &outside, &container.join("staged")] {
            std::fs::create_dir_all(folder).expect("a folder");
        }
        let mut backend = Backend::headless(Session::new());
        backend.confine(Confinement::new(&[container.clone(), bundle.clone()], std::slice::from_ref(&container)).expect("the confinement"));
        Roots { _dir: dir, container, bundle, outside, backend }
    }

    fn file(path: &Path, bytes: &[u8]) -> String {
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("its folder");
        std::fs::write(path, bytes).expect("the file is written");
        path.display().to_string()
    }

    /// A source in a folder the session may only read is read from a copy of the same name and bytes in
    /// the staged folder, which is the same copy the next time and a different one for another source of the
    /// same name and other bytes; a source whose folder may be written, and one the session may not read,
    /// are read where they are.
    #[test]
    fn a_source_is_copied_only_where_its_folder_may_not_be_written() {
        let roots = roots();
        let staged = roots.container.join("staged");
        let first = file(&roots.bundle.join("a").join("card.json"), b"first");
        let copy = stage(&roots.backend, &staged, &first).expect("staged").expect("a copy");
        assert!(copy.starts_with(&staged.display().to_string()) && copy.ends_with("card.json"), "{copy}");
        assert_eq!(std::fs::read(&copy).expect("the copy reads"), b"first");
        assert_eq!(stage(&roots.backend, &staged, &first).expect("staged again"), Some(copy.clone()), "one source finds its own copy");
        let second = file(&roots.bundle.join("b").join("card.json"), b"second");
        let other = stage(&roots.backend, &staged, &second).expect("staged").expect("a copy");
        assert_ne!(other, copy, "two sources of one name and other bytes never share a copy");
        assert_eq!(std::fs::read(&copy).expect("the first copy reads"), b"first");
        let inside = file(&roots.container.join("card.json"), b"inside");
        assert_eq!(stage(&roots.backend, &staged, &inside).expect("not staged"), None, "a folder that may be written is read where it is");
        let unread = file(&roots.outside.join("card.json"), b"outside");
        assert_eq!(stage(&roots.backend, &staged, &unread).expect("not staged"), None, "the command refuses what the session may not read");
        let folder = roots.bundle.join("folder.json");
        std::fs::create_dir(&folder).expect("a folder in the bundle");
        assert_eq!(stage(&roots.backend, &staged, &folder.display().to_string()).expect("not staged"), None, "the command refuses what is no file");
    }

    /// A copy is named by what it holds. The same file, installed by an update at a new path, finds the
    /// copy it had, so no update leaves a copy nothing names again; once an update changes it, it finds
    /// a copy of what it holds now, and the copy of what it held, with what was written beside it, stays
    /// as it was. The name is SipHash's of the bytes, the same from every build on every platform.
    #[test]
    fn a_copy_is_named_by_what_it_holds_not_by_where_its_source_sits() {
        let roots = roots();
        let staged = roots.container.join("staged");
        let before = file(&roots.bundle.join("install-1").join("assets").join("card.json"), b"first");
        let copy = stage(&roots.backend, &staged, &before).expect("staged").expect("a copy");
        let id = Path::new(&copy).parent().and_then(Path::file_name).expect("the copy's folder").to_os_string();
        assert_eq!(id, "78bd54d4840d83d320dd8dfedaf174ff", "the name of a copy of `first`");
        let beside = Path::new(&copy).with_file_name("card Media").join("1").join("photo.png");
        file(&beside, b"what the import wrote");
        let moved = file(&roots.bundle.join("install-2").join("assets").join("card.json"), b"first");
        assert_eq!(stage(&roots.backend, &staged, &moved).expect("staged").as_ref(), Some(&copy), "the same file at a new path finds its copy");
        let folders = || std::fs::read_dir(&staged).expect("the staged folder reads").flatten().map(|entry| entry.file_name()).collect::<Vec<_>>();
        assert_eq!(folders(), std::slice::from_ref(&id), "no second copy of the same file");
        std::fs::write(&moved, b"second").expect("an update changes the file");
        let changed = stage(&roots.backend, &staged, &moved).expect("staged").expect("a copy");
        assert_ne!(changed, copy, "a changed source is never read from the copy of what it held");
        assert_eq!(std::fs::read(&changed).expect("the copy reads"), b"second");
        assert_eq!(std::fs::read(&copy).expect("the first copy reads"), b"first", "the copy of what it held stays");
        assert_eq!(std::fs::read(&beside).expect("the media read"), b"what the import wrote", "and what was written beside it");
    }

    /// Every byte of a source is in the name its copy takes, however many reads it takes to copy it: two
    /// sources that agree up to their last byte are two copies, and the names are SipHash's of the whole
    /// source as an independent implementation of it gives them.
    #[test]
    fn a_copy_is_named_by_every_byte_of_a_source_that_takes_many_reads_to_copy() {
        let roots = roots();
        let staged = roots.container.join("staged");
        let long: Vec<u8> = (0..100_000_u32).map(|index| u8::try_from(index % 251).expect("a byte")).collect();
        let mut changed = long.clone();
        if let Some(last) = changed.last_mut() {
            *last ^= 1;
        }
        let id = |bytes: &[u8], folder: &str| {
            let named = file(&roots.bundle.join(folder).join("card.json"), bytes);
            let copy = stage(&roots.backend, &staged, &named).expect("staged").expect("a copy");
            assert_eq!(std::fs::read(&copy).expect("the copy reads"), bytes, "the copy holds every byte");
            Path::new(&copy).parent().and_then(Path::file_name).expect("the copy's folder").to_string_lossy().into_owned()
        };
        assert_eq!(id(&long, "a"), "ed52f08072fea5c87e6154bbed957c06");
        assert_eq!(id(&changed, "b"), "58e459c27602879a9339a4b885fcd6e0");
    }

    /// The writes land beside the name the command is given, so a link in the bundle to a file inside
    /// the container is copied, though the file it leads to sits where the session may write.
    #[test]
    fn a_link_in_a_folder_that_may_not_be_written_is_copied() {
        let roots = roots();
        let target = file(&roots.container.join("real.json"), b"linked");
        let link = roots.bundle.join("link.json");
        std::os::unix::fs::symlink(&target, &link).expect("a link in the bundle");
        let copy = stage(&roots.backend, &roots.container.join("staged"), &link.display().to_string()).expect("staged").expect("a copy");
        assert!(copy.ends_with("link.json"), "the copy keeps the name the command is given: {copy}");
        assert_eq!(std::fs::read(&copy).expect("the copy reads"), b"linked");
    }

    /// The folder that holds the name decides, wherever the name or the folder leads: a link in the
    /// container to a file in the bundle is read where it is, a folder in the container that is a link
    /// into the bundle holds a file that is copied, and a folder in the bundle that is a link into the
    /// container holds one that is not.
    #[test]
    fn the_folder_that_holds_the_name_decides_wherever_a_link_leads() {
        let roots = roots();
        let staged = roots.container.join("staged");
        let in_bundle = file(&roots.bundle.join("real.json"), b"bundled");
        let link = roots.container.join("link.json");
        std::os::unix::fs::symlink(&in_bundle, &link).expect("a link in the container");
        assert_eq!(stage(&roots.backend, &staged, &link.display().to_string()).expect("not staged"), None, "a link in the container is read where it is");
        file(&roots.bundle.join("dir").join("card.json"), b"bundled");
        let into_bundle = roots.container.join("into-bundle");
        std::os::unix::fs::symlink(roots.bundle.join("dir"), &into_bundle).expect("a folder link in the container");
        let named = into_bundle.join("card.json").display().to_string();
        assert!(stage(&roots.backend, &staged, &named).expect("staged").is_some(), "a folder of the container that leads into the bundle is copied from");
        file(&roots.container.join("dir").join("card.json"), b"inside");
        let into_container = roots.bundle.join("into-container");
        std::os::unix::fs::symlink(roots.container.join("dir"), &into_container).expect("a folder link in the bundle");
        let named = into_container.join("card.json").display().to_string();
        assert_eq!(stage(&roots.backend, &staged, &named).expect("not staged"), None, "a folder of the bundle that leads into the container may be written");
    }

    /// A link left at the partial file's name is replaced, never written through.
    #[test]
    fn a_link_at_the_partial_file_is_not_written_through() {
        let roots = roots();
        let staged = roots.container.join("staged");
        let named = file(&roots.bundle.join("card.json"), b"source");
        let victim = roots.outside.join("victim");
        std::fs::write(&victim, b"untouched").expect("the file a link could lead to");
        let copy = stage(&roots.backend, &staged, &named).expect("staged").expect("a copy");
        std::os::unix::fs::symlink(&victim, staged.join(".card.json.partial")).expect("a link at the partial file");
        stage(&roots.backend, &staged, &named).expect("staged again").expect("a copy");
        assert_eq!(std::fs::read(&victim).expect("the file reads"), b"untouched", "nothing was written through the link");
        assert_eq!(std::fs::read(&copy).expect("the copy reads"), b"source");
    }

    /// A read-only source is copied again on every call, as it stands then, over its own read-only
    /// copy and over what a kill left of an earlier copy, and the copy leaves no partial file behind.
    #[test]
    fn a_read_only_source_is_copied_again() {
        let roots = roots();
        let staged = roots.container.join("staged");
        let source = roots.bundle.join("card.json");
        let read_only = |only: bool| {
            let mode = if only { 0o444 } else { 0o644 };
            std::fs::set_permissions(&source, std::os::unix::fs::PermissionsExt::from_mode(mode)).expect("the mode is set");
        };
        let named = file(&source, b"first");
        read_only(true);
        let copy = stage(&roots.backend, &staged, &named).expect("staged").expect("a copy");
        std::fs::set_permissions(&copy, std::os::unix::fs::PermissionsExt::from_mode(0o444)).expect("the copy is read-only");
        assert_eq!(stage(&roots.backend, &staged, &named).expect("staged again").as_ref(), Some(&copy), "copied again over a read-only copy");
        std::fs::write(staged.join(".card.json.partial"), b"what a kill left").expect("a partial copy");
        read_only(false);
        std::fs::write(&source, b"second").expect("the source changes");
        read_only(true);
        let again = stage(&roots.backend, &staged, &named).expect("staged again").expect("a copy");
        read_only(false);
        assert_eq!(std::fs::read(&again).expect("the copy reads"), b"second", "the copy is the source as it stands");
        let names: Vec<_> = std::fs::read_dir(&staged).expect("the staged folder reads").flatten().map(|entry| entry.file_name()).collect();
        assert!(names.iter().all(|name| !name.to_string_lossy().ends_with(".partial")), "no partial file is left: {names:?}");
    }

    /// A copy that fails, here of a source the system will not open, is refused and leaves no partial
    /// file behind.
    #[test]
    fn a_copy_that_fails_leaves_no_partial_file() {
        let roots = roots();
        let staged = roots.container.join("staged");
        let source = roots.bundle.join("card.json");
        let named = file(&source, b"unreadable");
        std::fs::set_permissions(&source, std::os::unix::fs::PermissionsExt::from_mode(0o000)).expect("the mode is set");
        let refused = stage(&roots.backend, &staged, &named).expect_err("refused");
        std::fs::set_permissions(&source, std::os::unix::fs::PermissionsExt::from_mode(0o644)).expect("the mode is set back");
        assert!(matches!(refused, CraftError::Io { .. }), "{refused:?}");
        let made: Vec<_> = std::fs::read_dir(&staged).expect("the staged folder reads").flatten().map(|entry| entry.file_name()).collect();
        assert!(made.is_empty(), "no partial file and no copy's folder is left: {made:?}");
    }

    /// A copy's folder that is a link out of the container is refused before the copy takes its name
    /// there, and the partial file goes with the refusal.
    #[test]
    fn a_copy_whose_folder_leads_out_of_the_container_is_refused() {
        let roots = roots();
        let staged = roots.container.join("staged");
        let named = file(&roots.bundle.join("card.json"), b"source");
        let copy = stage(&roots.backend, &staged, &named).expect("staged").expect("a copy");
        let folder = Path::new(&copy).parent().expect("the copy's folder").to_path_buf();
        std::fs::remove_dir_all(&folder).expect("the copy's folder is removed");
        std::os::unix::fs::symlink(&roots.outside, &folder).expect("a link out of the container in its place");
        let refused = stage(&roots.backend, &staged, &named).expect_err("refused");
        assert!(matches!(refused, CraftError::Io { .. }), "{refused:?}");
        assert_eq!(std::fs::read_dir(&roots.outside).expect("the folder outside reads").count(), 0, "nothing was written outside the container");
        assert!(!staged.join(".card.json.partial").exists(), "the partial file went with the refusal");
    }

    /// A staged folder the session may not write is refused before anything is made in it.
    #[test]
    fn a_copy_outside_the_write_root_is_refused_before_anything_is_made() {
        let roots = roots();
        let source = file(&roots.bundle.join("card.json"), b"source");
        let staged = roots.outside.join("staged");
        let refused = stage(&roots.backend, &staged, &source).expect_err("refused");
        assert!(matches!(refused, CraftError::Io { .. }), "{refused:?}");
        assert!(!staged.exists(), "nothing was made outside the write root");
    }
}
