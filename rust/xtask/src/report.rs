//! The one way a check reports a problem, so every message reads alike: `rules: <check>: <message>`.
//!
//! A gate whose output changes shape between checks is a gate nobody can script, so checks never
//! print; their strings become lines through these two functions.

use std::path::Path;

/// The line for a check that found nothing.
pub(crate) fn ok_line(check: &str) -> String {
    format!("rules: {check}: ok")
}

/// The line for one problem a check found, one per problem so each is a line of its own.
pub(crate) fn error_line(check: &str, message: impl AsRef<str>) -> String {
    format!("rules: {check}: {}", message.as_ref())
}

/// The exit of a command that could not do its work, with why on its standard error, so every command
/// says it alike: `cargo xtask <command>: <failure>`.
pub(crate) fn failed(command: &str, failure: impl std::fmt::Display) -> std::process::ExitCode {
    eprintln!("cargo xtask {command}: {failure}");
    std::process::ExitCode::from(1)
}

/// Path as the repository expects to see it in a message: relative to the root, `/` separated, so a
/// report from another machine's checkout points at the same file. A tool may hand back the root's
/// resolved path (Cargo does, for a checkout reached through a symbolic link such as macOS's `/var`), so
/// that spelling of the root is stripped too.
pub(crate) fn shown(root: &Path, path: &Path) -> String {
    let resolved = std::fs::canonicalize(root).ok();
    let relative = path.strip_prefix(root).ok().or_else(|| resolved.as_deref().and_then(|real| path.strip_prefix(real).ok())).unwrap_or(path);
    let text = relative.to_string_lossy().replace('\\', "/");
    if text.is_empty() { ".".to_string() } else { text }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn relative_shown_drops_the_root() {
        let root = PathBuf::from("/repo");
        assert_eq!(shown(&root, &PathBuf::from("/repo/rust/core/x/src/lib.rs")), "rust/core/x/src/lib.rs");
    }

    #[test]
    fn outside_the_root_is_left_alone() {
        let root = PathBuf::from("/repo");
        assert_eq!(shown(&root, &PathBuf::from("/elsewhere/y")), "/elsewhere/y");
    }
}
