//! The scratch folders every test of this crate that starts an app runs on: a container with the
//! working folders inside it, and nothing else. A test binary compiles only the helpers it declares,
//! and dead-code analysis is per binary, so this module holds what every one of them uses; what only
//! some use sits in their own modules (`common` for the confinement tests, `fontlisting` for the font
//! tests), built over [`Scratch::build`]. The helpers answer `Result` like every helper of a test in
//! this repository, so the unwraps sit in the `#[test]` functions, where the lint rules let them.

use std::path::{Path, PathBuf};

use craftbridge_core::{CraftError, Result, folders::Folders};
use craftbridge_effects::EffectCraftApp;
use tempfile::TempDir;

pub struct Scratch {
    pub dir: TempDir,
    pub folders: Folders,
}

impl Scratch {
    pub fn new() -> Result<Self> {
        Self::build(None, Vec::new())
    }

    /// The folders of an app that has the given bundle and font folders, set before the folders are
    /// prepared, as an app's are.
    pub fn build(bundle: Option<PathBuf>, fonts: Vec<PathBuf>) -> Result<Self> {
        let dir = temporary("a scratch folder")?;
        let folders = folders_in(&dir.path().join("container"), bundle, fonts);
        folders.prepare()?;
        Ok(Self { dir, folders })
    }

    pub fn path(&self) -> &Path {
        &self.folders.container
    }

    /// The scratch folder the container sits in; the command line runs in it.
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn app(&self) -> Result<EffectCraftApp> {
        EffectCraftApp::new(&self.folders)
    }
}

/// The folders an app keeps inside `container`, as every test of the crate lays them out, and so as a
/// second process started on the same container finds them.
pub fn folders_in(container: &Path, bundle: Option<PathBuf>, fonts: Vec<PathBuf>) -> Folders {
    Folders { bundle, fonts, ..Folders::inside(container.to_path_buf(), ".") }
}

pub fn temporary(what: &str) -> Result<TempDir> {
    tempfile::tempdir().map_err(|e| CraftError::io(format!("{what}: {e}")))
}
