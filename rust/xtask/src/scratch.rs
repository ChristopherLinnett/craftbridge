//! Fresh scratch directories, made in one place.
//!
//! `generate --check` regenerates into one, and every test that needs files on disk builds its tree in
//! one, so the rule that keeps two of them apart is written once; so is the copy of the real `rust/` tree
//! the rules' tests plant violations in.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A new, empty directory under the system's temporary folder.
pub(crate) fn create(tag: &str) -> Result<PathBuf, String> {
    create_in(&std::env::temp_dir(), tag)
}

/// A new, empty directory under `inside`, whose folders are made as needed: for a scratch that has to sit
/// where a build finds its own dependencies by walking up from its own folder, wherever the temporary
/// folder would be out of reach of them. The name is unique to this process and this call: the process id
/// separates concurrent runs, the counter two calls in one clock tick, which test threads make all the
/// time.
pub(crate) fn create_in(inside: &Path, tag: &str) -> Result<PathBuf, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = inside.join(format!("{}{}-{nanos}-{serial}", prefix(tag), std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    Ok(dir)
}

/// How the name of every scratch `create_in` makes for `tag` begins, so one that outlives its maker can be
/// told by its name.
pub(crate) fn prefix(tag: &str) -> String {
    format!("craftbridge-xtask-{tag}-")
}

/// A scratch directory removed when it goes out of scope, so a failing test leaves nothing behind.
#[cfg(test)]
pub(crate) struct Scratch(pub(crate) PathBuf);

#[cfg(test)]
impl Scratch {
    pub(crate) fn new(tag: &str) -> Self {
        Self(create(tag).unwrap())
    }

    /// Write `text` to `relative`, creating its folders.
    pub(crate) fn write(&self, relative: &str, text: impl AsRef<[u8]>) {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

    pub(crate) fn mkdir(&self, relative: &str) {
        std::fs::create_dir_all(self.0.join(relative)).unwrap();
    }
}

#[cfg(test)]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A copy of this repository's `rust/` tree in which a violation can be planted without touching the
/// real one. A real tree is what proves a rule: a synthetic fixture cannot show that a file in a crate
/// the table really marks, found by the walk that really runs, is reported. The scratch stands in for the
/// repository root, with the copy at `<scratch>/rust`.
#[cfg(test)]
pub(crate) fn rust_tree_copy(tag: &str) -> Scratch {
    let scratch = Scratch::new(tag);
    copy_tree(&crate::repo_root().join("rust"), &scratch.0.join("rust"));
    scratch
}

/// The `xtask` binary a test runs a command as: built once into the very folder the test binary sits
/// beside, because a command run by a test is the program's own self, not the test's.
#[cfg(test)]
pub(crate) fn xtask_binary() -> &'static std::path::Path {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT
        .get_or_init(|| {
            let made = std::process::Command::new(env!("CARGO"))
                .args(["build", "--package", "xtask", "--locked", "--quiet"])
                .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
                .status()
                .unwrap();
            assert!(made.success(), "the build of xtask for its own tests failed");
            let test = std::env::current_exe().unwrap();
            let built = test.parent().and_then(Path::parent).map(|debug| debug.join("xtask")).expect("the test binary sits under the build folder");
            assert!(built.exists(), "{} is not the built binary", built.display());
            built
        })
        .as_path()
}

/// Copy one directory tree, skipping build output; a scratch needs the manifests and sources only.
#[cfg(test)]
pub(crate) fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        if entry.file_name() == "target" {
            continue;
        }
        let child_to = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &child_to);
        } else {
            std::fs::copy(entry.path(), &child_to).unwrap();
        }
    }
}
