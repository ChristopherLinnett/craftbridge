//! The callback a foreign caller gives an export to hear its progress.

/// Told how far an export has got, on the engine's thread, once every progress interval. The
/// engine ignores a listener that fails, so a listener that throws never stops an export.
#[uniffi::export(with_foreign)]
pub trait ProgressListener: Send + Sync {
    /// `done` of `total`, in the export's own units, and the seconds left when it can say.
    fn progress(&self, done: u64, total: u64, seconds_left: Option<f64>);
}
