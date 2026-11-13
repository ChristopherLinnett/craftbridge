//! Rule 1 — layout: what may sit at the repository root and inside a platform folder.
//!
//! A root that accumulates loose directories is a repository where people start putting platform code
//! where no build reads it, and a platform folder that holds more than `sdk` and `samples` stops being
//! the complete build of its own that rule 1 promises. So the names are listed once and everything else
//! is an error; `.DS_Store` is ignored because macOS writes it no matter what the repository asks.

use std::path::Path;

/// The only names allowed directly at the root.
const ALLOWED_ROOT: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "README.md",
    "NOTICE",
    "ATTRIBUTION.md",
    "LICENSE-MIT",
    "LICENSE-APACHE",
    "CHANGELOG.md",
    ".gitignore",
    ".gitattributes",
    ".editorconfig",
    ".git",
    ".github",
    "rust",
    "ios",
    "android",
    "conformance",
    "docs",
    "tools",
    "plan",
];

/// The only names allowed directly inside `ios/` and `android/`.
const ALLOWED_PLATFORM: &[&str] = &["sdk", "samples"];

/// Names a directory listing may hold that no rule cares about: macOS writes `.DS_Store` into any folder
/// Finder opens, whatever the repository asks, and git ignores it.
pub(crate) const IGNORED_NAMES: &[&str] = &[".DS_Store"];

/// Check one directory against an allowlist, returning one error per unexpected entry. `hint` finishes
/// the sentence so the root and the platform folders share this one function. A directory that cannot
/// be listed is an error, never an empty listing.
fn entries_violations(root: &Path, dir: &Path, allowed: &[&str], hint: &str) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let mut names = Vec::new();
    for entry in entries {
        names.push(entry.map_err(|error| format!("{}: {error}", dir.display()))?.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    let prefix = if dir == root { String::new() } else { format!("{}/", crate::report::shown(root, dir)) };
    Ok(names
        .into_iter()
        .filter(|name| !IGNORED_NAMES.contains(&name.as_str()) && !allowed.contains(&name.as_str()))
        .map(|name| format!("{prefix}{name} is not allowed here: {hint}"))
        .collect())
}

/// Run the layout check: the root, then `ios/` and `android/` when they exist.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let mut errors = entries_violations(root, root, ALLOWED_ROOT, "platform files belong in rust/, ios/ or android/")?;
    for platform in ["ios", "android"] {
        let dir = root.join(platform);
        if dir.is_dir() {
            errors.extend(entries_violations(root, &dir, ALLOWED_PLATFORM, &format!("only `sdk` and `samples` belong in {platform}/"))?);
        }
    }
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn a_compliant_root_passes() {
        let scratch = Scratch::new("ok");
        for name in ALLOWED_ROOT {
            scratch.mkdir(name);
        }
        scratch.write(".DS_Store", "x");
        assert!(check(&scratch.0).unwrap().is_empty());
    }

    #[test]
    fn a_stray_directory_at_the_root_is_reported() {
        let scratch = Scratch::new("stray");
        scratch.mkdir("rust");
        scratch.mkdir("somewhere-else");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("somewhere-else"), "{errors:?}");
        assert!(errors[0].contains("not allowed here"), "{errors:?}");
    }

    #[test]
    fn a_platform_folder_holds_only_sdk_and_samples() {
        let scratch = Scratch::new("platform");
        scratch.mkdir("ios/sdk");
        scratch.mkdir("ios/samples");
        scratch.write("ios/.DS_Store", "x");
        scratch.mkdir("ios/extra");
        scratch.mkdir("android/third_party");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("extra") && e.contains("only `sdk` and `samples` belong in ios/")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("third_party") && e.contains("android/")), "{errors:?}");
    }
}
