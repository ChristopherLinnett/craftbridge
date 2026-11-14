//! The one error every call over the native boundary fails with, and the mapping onto it.
//!
//! Each row of the plan's mapping table (plan §14) is one variant, and each kind of source has one
//! way onto it: a constructor for a failure that comes with its own words, the core's or a Craft
//! app's (an app plugin sorts its app's errors into the rows through these, since the core knows no
//! app), or `Cancelled` itself, which carries none; a `From` for the error types the core itself
//! meets (`std::io`, `serde_json`); and [`CraftError::from_panic`] for a panic caught at the engine
//! actor's boundary. Swift and Kotlin see these variants and nothing else, so no other error ever
//! reaches a phone: a panic or a foreign error type crossing the boundary would stop the app there,
//! which AGENTS.md §3 forbids.

use std::any::Any;
use std::fmt;

/// The error every call over the native boundary fails with: one variant per row of the plan's
/// mapping table (§14).
///
/// The `message` fields hold the sentence the app's user sees, in the words of whoever met the
/// failure: the engine's for a command, the operating system's for a file, with the core's note of
/// which folder or file before it where the core adds one. `Display` shows it unchanged, with a
/// `CommandFailed`'s command before it and its step after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CraftError {
    /// Unknown parameter, bad JSON, or parameters a check refuses: the engine's check of a command's,
    /// or the core's of the folder paths an engine starts with.
    InvalidParams { message: String },
    /// A command answered `Err` or a render item failed; `step` is the step of a `run` that failed.
    CommandFailed { command: String, message: String, step: Option<u32> },
    /// No comp, layer or property the call named.
    NotFound { message: String },
    /// A capability this platform lacks, such as an export format the build does not carry.
    Unsupported { message: String },
    /// The caller's `CancelToken` was set, or the exporter reported the render cancelled.
    Cancelled,
    /// A file access outside the confinement or one the disk refuses: a folder missing, not a folder,
    /// unreadable, unwritable or outside the container, a file that will not read, a disk full.
    Io { message: String },
    /// Device creation or a surface failure.
    Gpu { message: String },
    /// A panic caught at the actor's boundary, after which the engine thread keeps the document, or a
    /// call the engine itself can no longer answer, such as one made after `close` (plan §16).
    Internal { message: String },
}

impl CraftError {
    /// The row for a call's parameters: an unknown parameter, bad JSON, or a value the engine or the
    /// core refuses, such as a folder path that names no folder.
    #[must_use]
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::InvalidParams { message: message.into() }
    }

    /// The row for a command's own failure; `step` is `Some` only for one step of a `run`.
    #[must_use]
    pub fn command_failed(command: impl Into<String>, message: impl Into<String>, step: Option<u32>) -> Self {
        Self::CommandFailed { command: command.into(), message: message.into(), step }
    }

    /// The row for a comp, layer or property that is not there.
    #[must_use]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound { message: message.into() }
    }

    /// The row for something this platform does not do.
    #[must_use]
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported { message: message.into() }
    }

    /// The row for a file or folder the engine may not touch or cannot use, or a disk that cannot
    /// take the write; the message names the folder or file.
    #[must_use]
    pub fn io(message: impl Into<String>) -> Self {
        Self::Io { message: message.into() }
    }

    /// The row for a GPU device or surface that will not be made.
    #[must_use]
    pub fn gpu(message: impl Into<String>) -> Self {
        Self::Gpu { message: message.into() }
    }

    /// The row for a failure of the engine itself, in the core's own words, such as a call after
    /// `close` (plan §16); a caught panic goes through [`CraftError::from_panic`] instead.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal { message: message.into() }
    }

    /// The panic row, given what `std::panic::catch_unwind` returned: the payload's text when it is a
    /// string, as `panic!` with a message makes it, and a fixed sentence otherwise, since a payload of
    /// any other type holds nothing a user could read.
    #[must_use]
    pub fn from_panic(caught: Box<dyn Any + Send>) -> Self {
        let message = caught
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| caught.downcast_ref::<&str>().map(|text| (*text).to_string()))
            .unwrap_or_else(|| "a caught panic carried no message".to_string());
        Self::Internal { message }
    }
}

/// Bad JSON at the boundary is a parameter error: every call that takes parameters carries them as
/// JSON, so JSON that does not parse is a call with bad parameters, whatever method it carried.
/// JSON read from a file or stream that failed to read is a file failure instead, the `Io` row: the
/// text was never seen, so nothing says it was bad.
impl From<serde_json::Error> for CraftError {
    fn from(error: serde_json::Error) -> Self {
        let message = error.to_string();
        if error.is_io() { Self::Io { message } } else { Self::InvalidParams { message } }
    }
}

/// Any failure of the filesystem or the disk — refused, unwritable, full — is the `Io` row; the caller
/// adds which folder or file it was touching to its own construction, this one only carries what the
/// operating system said.
impl From<std::io::Error> for CraftError {
    fn from(error: std::io::Error) -> Self {
        Self::Io { message: error.to_string() }
    }
}

impl fmt::Display for CraftError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParams { message } => out.write_str(message),
            Self::CommandFailed { command, message, step } => match step {
                Some(step) => write!(out, "{command}: {message} (step {step})"),
                None => write!(out, "{command}: {message}"),
            },
            Self::NotFound { message } | Self::Unsupported { message } | Self::Io { message } | Self::Gpu { message } | Self::Internal { message } => {
                out.write_str(message)
            }
            Self::Cancelled => out.write_str("cancelled"),
        }
    }
}

impl std::error::Error for CraftError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_json_maps_to_invalid_params() {
        let broken = serde_json::from_str::<serde_json::Value>("{ \"comp\": }").err();
        let error = CraftError::from(broken.expect("the JSON above does not parse"));
        assert!(matches!(error, CraftError::InvalidParams { .. }), "{error:?}");
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn json_that_could_not_be_read_maps_to_io() {
        /// A reader whose disk has gone: every read fails before any JSON is seen.
        struct Unreadable;
        impl std::io::Read for Unreadable {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("the disk went away"))
            }
        }
        let unread = serde_json::from_reader::<_, serde_json::Value>(Unreadable).err();
        let error = CraftError::from(unread.expect("nothing could be read"));
        assert!(matches!(error, CraftError::Io { .. }), "{error:?}");
        assert!(error.to_string().contains("the disk went away"), "{error}");
    }

    #[test]
    fn invalid_params_shows_the_engine_check_its_own_words() {
        let error = CraftError::invalid_params("comp.new: unknown parameter(s) widht");
        assert_eq!(error.to_string(), "comp.new: unknown parameter(s) widht");
    }

    #[test]
    fn command_failed_shows_the_command_the_message_and_the_step() {
        let failed = CraftError::command_failed("effect.apply", "no effect \"Gausian Blur\"", Some(3));
        assert_eq!(failed.to_string(), "effect.apply: no effect \"Gausian Blur\" (step 3)");
        let unwrapped = CraftError::command_failed("effect.apply", "no effect \"Gausian Blur\"", None);
        assert_eq!(unwrapped.to_string(), "effect.apply: no effect \"Gausian Blur\"");
    }

    #[test]
    fn missing_document_objects_map_to_not_found() {
        let error = CraftError::not_found("no layer \"#9\" in \"Birthday Card\"");
        assert!(matches!(error, CraftError::NotFound { .. }), "{error:?}");
        assert_eq!(error.to_string(), "no layer \"#9\" in \"Birthday Card\"");
    }

    #[test]
    fn missing_capabilities_map_to_unsupported() {
        let error = CraftError::unsupported("external effect helpers are not available on iOS");
        assert!(matches!(error, CraftError::Unsupported { .. }), "{error:?}");
        assert_eq!(error.to_string(), "external effect helpers are not available on iOS");
    }

    #[test]
    fn cancellation_maps_to_cancelled() {
        assert_eq!(CraftError::Cancelled.to_string(), "cancelled");
    }

    #[test]
    fn file_and_disk_failures_map_to_io() {
        let missing = std::fs::read("/no/such/place/for-craftbridge-tests").err();
        let error = CraftError::from(missing.expect("that path does not exist"));
        assert!(matches!(error, CraftError::Io { .. }), "{error:?}");
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn device_and_surface_failures_map_to_gpu() {
        let error = CraftError::gpu("no Metal adapter");
        assert!(matches!(error, CraftError::Gpu { .. }), "{error:?}");
        assert_eq!(error.to_string(), "no Metal adapter");
    }

    #[test]
    fn a_caught_panic_maps_to_internal_with_its_own_words() {
        let caught = std::panic::catch_unwind(|| panic!("the actor call blew up"));
        let payload = caught.expect_err("the closure above panics on purpose");
        let error = CraftError::from_panic(payload);
        assert_eq!(error, CraftError::internal("the actor call blew up"));
        assert_eq!(error.to_string(), "the actor call blew up");
    }

    #[test]
    fn a_caught_panic_without_a_string_keeps_a_plain_sentence() {
        let text = CraftError::from_panic(Box::new(String::from("a boxed message")));
        let counted = CraftError::from_panic(Box::new(7_u8));
        assert_eq!(text, CraftError::internal("a boxed message"));
        assert!(matches!(counted, CraftError::Internal { .. }), "{counted:?}");
        assert!(!counted.to_string().is_empty());
    }
}
