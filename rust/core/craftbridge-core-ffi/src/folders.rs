//! The folders an app names when it starts an engine.

/// The folders the app names when it starts an engine, as text: Swift and Kotlin hold paths as
/// strings. The core takes them as paths through [`From`], and refuses a relative one when the
/// engine starts.
#[derive(uniffi::Record)]
pub struct Folders {
    /// The app's sandbox: the engine's read and write root.
    pub container: String,
    /// The app bundle (iOS) or its unpacked assets (Android): a read root, or none.
    pub bundle: Option<String>,
    /// Where the engine keeps its settings, inside the container.
    pub config: String,
    /// Where the engine keeps what it can throw away, inside the container.
    pub cache: String,
    /// Where the engine keeps models it has installed, inside the container.
    pub models: String,
    /// Where the app keeps its plug-ins, or none while they are switched off.
    pub plugins: Option<String>,
    /// Extra font folders, after the system's own.
    pub fonts: Vec<String>,
}

impl From<Folders> for craftbridge_core::folders::Folders {
    fn from(folders: Folders) -> Self {
        Self {
            container: folders.container.into(),
            bundle: folders.bundle.map(Into::into),
            config: folders.config.into(),
            cache: folders.cache.into(),
            models: folders.models.into(),
            plugins: folders.plugins.map(Into::into),
            fonts: folders.fonts.into_iter().map(Into::into).collect(),
        }
    }
}
