//! The scratch folders one test of this crate runs on: a container with the working folders inside
//! it, and nothing else — no bundle unless the test asks for one, no plug-in folder, no extra fonts.
//! The helpers answer `Result` like every helper of a test in this repository, so the unwraps sit in
//! the `#[test]` functions, where the lint rules let them.

use std::path::{Path, PathBuf};

use craftbridge_core::{CraftError, PixelFormat, Result, folders::Folders};
use craftbridge_effects::EffectCraftApp;
use tempfile::TempDir;

pub struct Scratch {
    pub dir: TempDir,
    /// The bundle's folder, when the test gives the app one. It is a folder of its own, outside the
    /// scratch folder, so it lies outside the command line's roots as it lies outside the container.
    pub bundle: Option<TempDir>,
    /// A folder of its own that lies outside the container, the bundle and the command line's
    /// folder: where a test aims what the confinement must refuse, so a refused write is seen as a
    /// file that appeared, and nothing another test or run left behind can stand in for it.
    pub outside: TempDir,
    pub folders: Folders,
}

impl Scratch {
    pub fn new() -> Result<Self> {
        Self::build(None)
    }

    /// The same folders, with a bundle the app may read and never write.
    pub fn with_bundle() -> Result<Self> {
        Self::build(Some(temporary("a bundle folder")?))
    }

    fn build(bundle: Option<TempDir>) -> Result<Self> {
        let dir = temporary("a scratch folder")?;
        let container = dir.path().join("container");
        let folders = Folders {
            container: container.clone(),
            bundle: bundle.as_ref().map(|bundle| bundle.path().to_path_buf()),
            config: container.join(".config"),
            cache: container.join(".cache"),
            models: container.join(".models"),
            plugins: None,
            fonts: Vec::new(),
        };
        folders.prepare()?;
        Ok(Self { dir, bundle, outside: temporary("a folder outside the container")?, folders })
    }

    pub fn path(&self) -> &Path {
        &self.folders.container
    }

    /// The scratch folder the container sits in; the command line runs in it.
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// The folder outside every root: empty, but for what the test puts there.
    pub fn outside(&self) -> &Path {
        self.outside.path()
    }

    /// A file outside every root that reads fine, for the read the confinement must refuse.
    pub fn outside_file(&self) -> Result<PathBuf> {
        let file = self.outside().join("readable.txt");
        std::fs::write(&file, "readable outside the container").map_err(|e| CraftError::io(format!("a file outside the container: {e}")))?;
        Ok(file)
    }

    /// The bundle's folder, when the test gave the app one.
    pub fn bundle(&self) -> Option<&Path> {
        self.bundle.as_ref().map(TempDir::path)
    }

    pub fn app(&self) -> Result<EffectCraftApp> {
        EffectCraftApp::new(&self.folders)
    }
}

/// Every format a frame is answered in, for the tests that hold each rule to all of them.
pub const EVERY_FORMAT: [PixelFormat; 3] = [PixelFormat::Png, PixelFormat::Rgba8Straight, PixelFormat::Bgra8Premultiplied];

fn temporary(what: &str) -> Result<TempDir> {
    tempfile::tempdir().map_err(|e| CraftError::io(format!("{what}: {e}")))
}
