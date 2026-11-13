//! The one way a check reports a problem, so every message reads alike: `rules: <check>: <message>`.
//!
//! A gate whose output changes shape between checks is a gate nobody can script, so checks never
//! print; they hand strings to these two functions.

use std::path::Path;

/// Report a check that found nothing.
pub(crate) fn ok(check: &str) {
    println!("rules: {check}: ok");
}

/// Report one problem a check found, one call per problem so each is a line of its own.
pub(crate) fn error(check: &str, message: impl AsRef<str>) {
    println!("rules: {check}: {}", message.as_ref());
}

/// Path as the repository expects to see it in a message: relative to the root, `/` separated, so a
/// report from another machine's checkout points at the same file.
pub(crate) fn shown(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
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
