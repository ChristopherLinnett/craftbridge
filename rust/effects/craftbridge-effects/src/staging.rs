//! Staging (AD5): a file the session may read from a folder it may not write is read from a copy
//! inside the container instead.
//!
//! Some of EffectCraft's imports write beside the file they read, in the folder that holds it as it
//! is named: a template's import writes its media to `<stem> Media/` there. A file in the app bundle,
//! a root the session may read and never write, would have those writes land in the bundle: on iOS
//! they fail without a word and what they carry comes out missing, and on Android, where the bundle's
//! unpacked assets can be written, they escape the confinement. So such a file is copied first into a
//! folder of the container's cache, and the command reads the copy, whose writes then land beside it
//! inside the container. [`stage`] knows nothing of what the file is: each caller names the source a
//! command reads, and hands the command the path it answers.

use std::collections::hash_map::DefaultHasher;
use std::ffi::{OsStr, OsString};
use std::hash::{Hash, Hasher};
use std::path::Path;

use craftbridge_core::{CraftError, Result};
use effectcraft_automation::Backend;
use effectcraft_automation::confine::Access;

/// The folder of the cache the copies go under, one folder to each source.
const STAGING: &str = "staged";

/// The path to read `source` from: a copy inside the container when the folder that holds `source`,
/// as it is named, is one the confinement lets the session read and not write; `None` when it is read
/// where it is, because that folder may be written, or because the confinement refuses to read the
/// source or it is no regular file, which the command then says in its own words.
///
/// The writes land beside the name the command is given, not where a link at that name leads, so the
/// folder holding the name is the one asked about: a link in the bundle to a file inside the container
/// is copied too. The copy goes to `<cache>/staged/<id>/<name>`, `<id>` the hash of the source's
/// resolved path and `<name>` its file name as given, so what is written beside the copy is named as it
/// would be beside the source, two sources never share a folder, and one source always finds its own.
/// It is written again on every call, whole before it takes the name ([`write_copy`]).
pub(crate) fn stage(backend: &Backend, cache: &Path, source: &str) -> Result<Option<String>> {
    let Some(confinement) = backend.confinement() else { return Ok(None) };
    let Ok(resolved) = confinement.check(source, Access::Read) else { return Ok(None) };
    if !resolved.is_file() {
        return Ok(None);
    }
    let named = Path::new(source);
    let (Some(folder), Some(name)) = (named.parent(), named.file_name()) else { return Ok(None) };
    if confinement.check(&folder.to_string_lossy(), Access::Write).is_ok() {
        return Ok(None);
    }
    let mut id = DefaultHasher::new();
    resolved.hash(&mut id);
    let into = cache.join(STAGING).join(format!("{:016x}", id.finish()));
    let copy = into.join(name).to_string_lossy().into_owned();
    let refused = |error: &dyn std::fmt::Display| CraftError::io(format!("cannot copy {source} into the container: {error}"));
    // The copy goes only where the session may write, which the confinement says before anything is
    // made, so a cache that lay outside the write root would be refused here, not written.
    confinement.check(&copy, Access::Write).map_err(|error| refused(&error))?;
    std::fs::create_dir_all(&into).map_err(|error| refused(&error))?;
    write_copy(&resolved, &into, name).map_err(|error| refused(&error))?;
    Ok(Some(copy))
}

/// Copy `source` to `<into>/<name>` through a partial file renamed over the name. The copy is a file
/// of the copier's own making whatever the source's mode, so a read-only source is copied again on the
/// next call, where a copy that kept the source's mode could not be written over; and a kill mid-copy
/// leaves the name as it was and the partial file, which the next copy of the source makes anew.
fn write_copy(source: &Path, into: &Path, name: &OsStr) -> std::io::Result<()> {
    let mut partial_name = OsString::from(".");
    partial_name.push(name);
    partial_name.push(".partial");
    let partial = into.join(partial_name);
    match std::fs::remove_file(&partial) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    // Made new, so nothing already at the name, a link least of all, is opened or followed.
    let mut written = std::fs::OpenOptions::new().write(true).create_new(true).open(&partial)?;
    let copied = std::fs::File::open(source).and_then(|mut read| std::io::copy(&mut read, &mut written));
    drop(written);
    let renamed = copied.and_then(|_| std::fs::rename(&partial, into.join(name)));
    if renamed.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    renamed
}

#[cfg(test)]
mod tests {
    use super::*;
    use effectcraft_automation::confine::Confinement;
    use effectcraft_engine::Session;
    use std::path::PathBuf;

    /// A confined backend over a container it may write and a bundle it may only read, with the
    /// container's cache, all inside a scratch folder of their own.
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
        for folder in [&container, &bundle, &outside, &container.join("cache")] {
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
    /// the cache, which is the same copy the next time and a different one for another source of the
    /// same name; a source whose folder may be written, and one the session may not read, are read
    /// where they are.
    #[test]
    fn a_source_is_copied_only_where_its_folder_may_not_be_written() {
        let roots = roots();
        let cache = roots.container.join("cache");
        let first = file(&roots.bundle.join("a").join("card.json"), b"first");
        let copy = stage(&roots.backend, &cache, &first).expect("staged").expect("a copy");
        assert!(copy.starts_with(&cache.join(STAGING).display().to_string()) && copy.ends_with("card.json"), "{copy}");
        assert_eq!(std::fs::read(&copy).expect("the copy reads"), b"first");
        assert_eq!(stage(&roots.backend, &cache, &first).expect("staged again"), Some(copy.clone()), "one source finds its own copy");
        let second = file(&roots.bundle.join("b").join("card.json"), b"second");
        let other = stage(&roots.backend, &cache, &second).expect("staged").expect("a copy");
        assert_ne!(other, copy, "two sources never share a folder");
        assert_eq!(std::fs::read(&copy).expect("the first copy reads"), b"first");
        let inside = file(&roots.container.join("card.json"), b"inside");
        assert_eq!(stage(&roots.backend, &cache, &inside).expect("not staged"), None, "a folder that may be written is read where it is");
        let unread = file(&roots.outside.join("card.json"), b"outside");
        assert_eq!(stage(&roots.backend, &cache, &unread).expect("not staged"), None, "the command refuses what the session may not read");
        let folder = roots.bundle.join("folder.json");
        std::fs::create_dir(&folder).expect("a folder in the bundle");
        assert_eq!(stage(&roots.backend, &cache, &folder.display().to_string()).expect("not staged"), None, "the command refuses what is no file");
    }

    /// The writes land beside the name the command is given, so a link in the bundle to a file inside
    /// the container is copied, though the file it leads to sits where the session may write.
    #[test]
    fn a_link_in_a_folder_that_may_not_be_written_is_copied() {
        let roots = roots();
        let target = file(&roots.container.join("real.json"), b"linked");
        let link = roots.bundle.join("link.json");
        std::os::unix::fs::symlink(&target, &link).expect("a link in the bundle");
        let copy = stage(&roots.backend, &roots.container.join("cache"), &link.display().to_string()).expect("staged").expect("a copy");
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
        let cache = roots.container.join("cache");
        let in_bundle = file(&roots.bundle.join("real.json"), b"bundled");
        let link = roots.container.join("link.json");
        std::os::unix::fs::symlink(&in_bundle, &link).expect("a link in the container");
        assert_eq!(stage(&roots.backend, &cache, &link.display().to_string()).expect("not staged"), None, "a link in the container is read where it is");
        file(&roots.bundle.join("dir").join("card.json"), b"bundled");
        let into_bundle = roots.container.join("into-bundle");
        std::os::unix::fs::symlink(roots.bundle.join("dir"), &into_bundle).expect("a folder link in the container");
        let named = into_bundle.join("card.json").display().to_string();
        assert!(stage(&roots.backend, &cache, &named).expect("staged").is_some(), "a folder of the container that leads into the bundle is copied from");
        file(&roots.container.join("dir").join("card.json"), b"inside");
        let into_container = roots.bundle.join("into-container");
        std::os::unix::fs::symlink(roots.container.join("dir"), &into_container).expect("a folder link in the bundle");
        let named = into_container.join("card.json").display().to_string();
        assert_eq!(stage(&roots.backend, &cache, &named).expect("not staged"), None, "a folder of the bundle that leads into the container may be written");
    }

    /// A link left at the partial file's name is replaced, never written through.
    #[test]
    fn a_link_at_the_partial_file_is_not_written_through() {
        let roots = roots();
        let cache = roots.container.join("cache");
        let named = file(&roots.bundle.join("card.json"), b"source");
        let victim = roots.outside.join("victim");
        std::fs::write(&victim, b"untouched").expect("the file a link could lead to");
        let copy = stage(&roots.backend, &cache, &named).expect("staged").expect("a copy");
        let folder = Path::new(&copy).parent().expect("the copy's folder").to_path_buf();
        std::os::unix::fs::symlink(&victim, folder.join(".card.json.partial")).expect("a link at the partial file");
        stage(&roots.backend, &cache, &named).expect("staged again").expect("a copy");
        assert_eq!(std::fs::read(&victim).expect("the file reads"), b"untouched", "nothing was written through the link");
        assert_eq!(std::fs::read(&copy).expect("the copy reads"), b"source");
    }

    /// A read-only source is copied again on every call, as it stands then, over what a kill left of an
    /// earlier copy, and the copy leaves no partial file behind.
    #[test]
    fn a_read_only_source_is_copied_again() {
        let roots = roots();
        let cache = roots.container.join("cache");
        let source = roots.bundle.join("card.json");
        let read_only = |only: bool| {
            let mode = if only { 0o444 } else { 0o644 };
            std::fs::set_permissions(&source, std::os::unix::fs::PermissionsExt::from_mode(mode)).expect("the mode is set");
        };
        let named = file(&source, b"first");
        read_only(true);
        let copy = stage(&roots.backend, &cache, &named).expect("staged").expect("a copy");
        let folder = Path::new(&copy).parent().expect("the copy's folder").to_path_buf();
        std::fs::write(folder.join(".card.json.partial"), b"what a kill left").expect("a partial copy");
        read_only(false);
        std::fs::write(&source, b"second").expect("the source changes");
        read_only(true);
        assert_eq!(stage(&roots.backend, &cache, &named).expect("staged again"), Some(copy.clone()));
        read_only(false);
        assert_eq!(std::fs::read(&copy).expect("the copy reads"), b"second", "the copy is the source as it stands");
        let names: Vec<_> = std::fs::read_dir(&folder).expect("the folder reads").flatten().map(|entry| entry.file_name()).collect();
        assert_eq!(names, vec![OsString::from("card.json")], "no partial file is left");
    }

    /// A copy that fails, here of a source the system will not open, is refused and leaves no partial
    /// file behind.
    #[test]
    fn a_copy_that_fails_leaves_no_partial_file() {
        let roots = roots();
        let cache = roots.container.join("cache");
        let source = roots.bundle.join("card.json");
        let named = file(&source, b"unreadable");
        std::fs::set_permissions(&source, std::os::unix::fs::PermissionsExt::from_mode(0o000)).expect("the mode is set");
        let refused = stage(&roots.backend, &cache, &named).expect_err("refused");
        std::fs::set_permissions(&source, std::os::unix::fs::PermissionsExt::from_mode(0o644)).expect("the mode is set back");
        assert!(matches!(refused, CraftError::Io { .. }), "{refused:?}");
        let made: Vec<_> = std::fs::read_dir(cache.join(STAGING))
            .expect("the copy's folder was made")
            .flatten()
            .flat_map(|entry| std::fs::read_dir(entry.path()).into_iter().flatten().flatten())
            .collect();
        assert!(made.is_empty(), "no partial file is left: {made:?}");
    }

    /// A cache the session may not write is refused before anything is made in it.
    #[test]
    fn a_copy_outside_the_write_root_is_refused_before_anything_is_made() {
        let roots = roots();
        let source = file(&roots.bundle.join("card.json"), b"source");
        let cache = roots.outside.join("cache");
        let refused = stage(&roots.backend, &cache, &source).expect_err("refused");
        assert!(matches!(refused, CraftError::Io { .. }), "{refused:?}");
        assert!(!cache.exists(), "nothing was made outside the write root");
    }
}
