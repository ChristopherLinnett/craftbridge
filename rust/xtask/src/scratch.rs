//! Fresh scratch directories under the system's temporary folder, made in one place.
//!
//! `generate --check` regenerates into one, and every test that needs files on disk builds its tree in
//! one, so the rule that keeps two of them apart is written once; so is the copy of the real `rust/` tree
//! the rules' tests plant violations in.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A new, empty directory, unique to this process and this call. The process id separates concurrent
/// runs; the counter separates two calls in one clock tick, which test threads make all the time.
pub(crate) fn create(tag: &str) -> Result<PathBuf, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_nanos()).unwrap_or(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("craftbridge-xtask-{tag}-{}-{nanos}-{serial}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    Ok(dir)
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

/// Copy one directory tree, skipping build output; a scratch needs the manifests and sources only.
#[cfg(test)]
fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
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
