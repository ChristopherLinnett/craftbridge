//! What the confinement and parity tests add to the plain scratch folders: a bundle the app may read
//! and never write, a folder outside every root, and the frame formats. The font tests use none of
//! it, so they do not compile this module.

use std::ops::Deref;
use std::path::{Path, PathBuf};

use craftbridge_core::{CraftError, PixelFormat, Result};
use tempfile::TempDir;

use crate::scratch::{self, Scratch as Plain};

/// A plain scratch with the extra folders, read through to it: its container, root and app are the
/// plain scratch's own.
pub struct Scratch {
    plain: Plain,
    /// The bundle's folder, when the test gives the app one. It is a folder of its own, outside the
    /// scratch folder, so it lies outside the command line's roots as it lies outside the container.
    bundle: Option<TempDir>,
    /// A folder of its own that lies outside the container, the bundle and the command line's
    /// folder: where a test aims what the confinement must refuse, so a refused write is seen as a
    /// file that appeared, and nothing another test or run left behind can stand in for it.
    outside: TempDir,
}

impl Deref for Scratch {
    type Target = Plain;

    fn deref(&self) -> &Plain {
        &self.plain
    }
}

impl Scratch {
    pub fn new() -> Result<Self> {
        Self::around(Plain::new()?, None)
    }

    /// The same folders, with a bundle the app may read and never write.
    pub fn with_bundle() -> Result<Self> {
        let bundle = scratch::temporary("a bundle folder")?;
        Self::around(Plain::build(Some(bundle.path().to_path_buf()), Vec::new())?, Some(bundle))
    }

    fn around(plain: Plain, bundle: Option<TempDir>) -> Result<Self> {
        Ok(Self { plain, bundle, outside: scratch::temporary("a folder outside the container")? })
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
}

/// Every format a frame is answered in, for the tests that hold each rule to all of them.
pub const EVERY_FORMAT: [PixelFormat; 3] = [PixelFormat::Png, PixelFormat::Rgba8Straight, PixelFormat::Bgra8Premultiplied];
