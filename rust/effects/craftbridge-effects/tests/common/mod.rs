//! The scratch folders one test of this crate runs on: a container with the working folders inside
//! it, and nothing else — no bundle, no plug-in folder, no extra fonts. The helpers answer `Result`
//! like every helper of a test in this repository, so the unwraps sit in the `#[test]` functions,
//! where the lint rules let them.

use craftbridge_core::{CraftError, Result, folders::Folders};
use craftbridge_effects::EffectCraftApp;

pub struct Scratch {
    pub dir: tempfile::TempDir,
    pub folders: Folders,
}

impl Scratch {
    pub fn new() -> Result<Self> {
        let dir = tempfile::tempdir().map_err(|e| CraftError::io(format!("a scratch folder: {e}")))?;
        let container = dir.path().join("container");
        let folders = Folders {
            container: container.clone(),
            bundle: None,
            config: container.join(".config"),
            cache: container.join(".cache"),
            models: container.join(".models"),
            plugins: None,
            fonts: Vec::new(),
        };
        folders.prepare()?;
        Ok(Self { dir, folders })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.folders.container
    }

    /// The scratch folder the container sits in; the command line runs in it.
    pub fn root(&self) -> &std::path::Path {
        self.dir.path()
    }

    pub fn app(&self) -> Result<EffectCraftApp> {
        EffectCraftApp::new(&self.folders)
    }
}
