//! Rule 1 — layout: what may sit at the repository root and inside a platform folder.
//!
//! A root that accumulates loose directories is a repository where people start putting platform code
//! where no build reads it, and a platform folder that holds more than `sdk` and `samples` stops being
//! the complete build of its own that rule 1 promises. So the names are listed once and everything else
//! is an error; `.DS_Store` is ignored because macOS writes it no matter what the repository asks.

use crate::scan;
use std::path::Path;
use std::process::{Command, Stdio};

/// The only names allowed directly at the root, each as the folder or file it must be.
const ALLOWED_ROOT: &[(&str, Entry)] = &[
    ("AGENTS.md", Entry::File),
    ("CLAUDE.md", Entry::File),
    ("README.md", Entry::File),
    ("NOTICE", Entry::File),
    ("ATTRIBUTION.md", Entry::File),
    ("LICENSE-MIT", Entry::File),
    ("LICENSE-APACHE", Entry::File),
    ("CHANGELOG.md", Entry::File),
    (".gitignore", Entry::File),
    (".gitattributes", Entry::File),
    (".editorconfig", Entry::File),
    (".git", Entry::Either),
    (".github", Entry::Folder),
    ("rust", Entry::Folder),
    ("ios", Entry::Folder),
    ("android", Entry::Folder),
    ("conformance", Entry::Folder),
    ("docs", Entry::Folder),
    ("tools", Entry::Folder),
    ("plan", Entry::Folder),
];

/// The only names allowed directly inside `ios/` and `android/`.
const ALLOWED_PLATFORM: &[(&str, Entry)] = &[("sdk", Entry::Folder), ("samples", Entry::Folder)];

/// What an allowed name must be on disk. `.git` is a folder in a checkout and a file in a worktree.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Entry {
    File,
    Folder,
    Either,
}

/// Names a directory listing may hold that no rule cares about: macOS writes `.DS_Store` into any folder
/// Finder opens, whatever the repository asks, and git ignores it.
pub(crate) const IGNORED_NAMES: &[&str] = &[".DS_Store"];

/// The names in `dir` that hold nothing the repository can: neither a tracked file nor an untracked one
/// git would add. Those are ignored names — an editor's settings folder a contributor ignores locally,
/// a tool's cache, a folder whose every file is ignored (`.claude/` holding only an ignored
/// `settings.local.json`) — so no rule about what the repository holds applies to them. A tracked file
/// always counts, so a platform file forced past the ignore rules is still refused. Outside a git
/// checkout (a scratch tree) nothing is ignored.
fn git_ignored(dir: &Path, names: &[String]) -> Vec<String> {
    let listed = |args: &[&str]| -> Option<Vec<String>> {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .arg("--")
            .args(names)
            .env("GIT_LITERAL_PATHSPECS", "1")
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())?;
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        Some(text.split('\0').filter_map(|path| path.split('/').next()).filter(|name| !name.is_empty()).map(str::to_string).collect())
    };
    let (Some(tracked), Some(addable)) = (listed(&["ls-files", "-z"]), listed(&["ls-files", "-z", "--others", "--exclude-standard"])) else {
        return Vec::new();
    };
    names.iter().filter(|name| !tracked.contains(name) && !addable.contains(name)).cloned().collect()
}

/// Check one directory against an allowlist, returning one error per unexpected entry and per allowed
/// name of the wrong kind. `hint` finishes the sentence so the root and the platform folders share this
/// one function. A directory that cannot be listed is an error, never an empty listing.
fn entries_violations(root: &Path, dir: &Path, allowed: &[(&str, Entry)], hint: &str) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let mut names = Vec::new();
    for entry in entries {
        names.push(entry.map_err(|error| format!("{}: {error}", dir.display()))?.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    let ignored = git_ignored(dir, &names);
    let prefix = if dir == root { String::new() } else { format!("{}/", crate::report::shown(root, dir)) };
    let mut errors = Vec::new();
    for name in names.iter().filter(|name| !IGNORED_NAMES.contains(&name.as_str()) && !ignored.contains(name)) {
        match allowed.iter().find(|(allowed, _)| allowed == name) {
            None => errors.push(format!("{prefix}{name} is not allowed here: {hint}")),
            Some((_, kind)) => {
                let folder = dir.join(name).is_dir();
                match kind {
                    Entry::Folder if !folder => errors.push(format!("{prefix}{name} must be a folder, and here it is a file")),
                    Entry::File if folder => errors.push(format!("{prefix}{name} must be a file, and here it is a folder")),
                    _ => {}
                }
            }
        }
    }
    Ok(errors)
}

/// A `node_modules` folder anywhere in the code trees. Every reader in the gate skips one as somebody
/// else's packages, and SwiftPM, Gradle and Cargo compile what such a folder holds like any other, so
/// code inside it would build while no rule, formatter or duplicate check read it.
fn hidden_package_folders(root: &Path) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    for tree in CODE_TREES {
        let mut stack = vec![root.join(tree)];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() || scan::is_build_output(&path) {
                    continue;
                }
                if entry.file_name() == "node_modules" {
                    errors.push(format!(
                        "{}: a node_modules folder in the code: the platform builds compile what it holds while every rule skips it",
                        crate::report::shown(root, &path)
                    ));
                } else if entry.file_name() != ".git" {
                    stack.push(path);
                }
            }
        }
    }
    errors.sort();
    Ok(errors)
}

/// The folders that hold the repository's code, which `hidden_package_folders` reads.
const CODE_TREES: &[&str] = &["rust", "ios", "android", "conformance", "tools", "docs", ".github"];

/// Run the layout check: the root, then `ios/` and `android/` when they exist, and the code trees for a
/// folder every rule would skip.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let mut errors = entries_violations(
        root,
        root,
        ALLOWED_ROOT,
        "it is not on the root's list (AGENTS.md §1); platform files belong in rust/, ios/ or android/, and anything else in one of the listed folders",
    )?;
    for platform in ["ios", "android"] {
        let dir = root.join(platform);
        if dir.is_dir() {
            errors.extend(entries_violations(root, &dir, ALLOWED_PLATFORM, &format!("only `sdk` and `samples` belong in {platform}/"))?);
        }
    }
    errors.extend(hidden_package_folders(root)?);
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn a_compliant_root_passes() {
        let scratch = Scratch::new("ok");
        for (name, kind) in ALLOWED_ROOT {
            if *kind == Entry::File {
                scratch.write(name, "x");
            } else {
                scratch.mkdir(name);
            }
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
    fn a_platform_manifest_at_the_root_or_in_a_platform_folder_is_refused() {
        let scratch = Scratch::new("manifests");
        for name in ["Package.swift", "Cargo.toml", "build.gradle.kts", "settings.gradle.kts", "ios/Package.swift"] {
            scratch.write(name, "");
        }
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 5, "{errors:?}");
        for name in ["Package.swift", "Cargo.toml", "build.gradle.kts", "settings.gradle.kts", "ios/Package.swift"] {
            assert!(errors.iter().any(|e| e.starts_with(&format!("{name} is not allowed here"))), "{name}: {errors:?}");
        }
        assert!(errors.iter().any(|e| e.contains("not on the root's list")), "{errors:?}");
    }

    #[test]
    fn a_listed_name_of_the_wrong_kind_is_refused() {
        let scratch = Scratch::new("kinds");
        scratch.write("ios", "not a folder");
        scratch.mkdir("README.md");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e == "ios must be a folder, and here it is a file"), "{errors:?}");
        assert!(errors.iter().any(|e| e == "README.md must be a file, and here it is a folder"), "{errors:?}");
    }

    #[test]
    fn a_name_git_ignores_is_not_the_repository_s() {
        let scratch = Scratch::new("ignored");
        let git = |args: &[&str]| Command::new("git").arg("-C").arg(&scratch.0).args(args).output().unwrap();
        assert!(git(&["init", "-q"]).status.success());
        scratch.write(".gitignore", "**/.claude/settings.local.json\n.vscode/\nPackage.swift\n");
        // A folder whose only file is ignored holds nothing the repository can, as git sees it.
        scratch.write(".claude/settings.local.json", "{}\n");
        scratch.write(".vscode/settings.json", "{}\n");
        scratch.write("android/.idea/workspace.xml", "<x/>\n");
        scratch.write("android/.gitignore", ".idea/\n");
        assert!(git(&["add", "-f", "android/.gitignore"]).status.success());
        let errors = check(&scratch.0).unwrap();
        assert_eq!(
            errors,
            ["android/.gitignore is not allowed here: only `sdk` and `samples` belong in android/"],
            "only the tracked file is the repository's"
        );
        // A file forced past the ignore rules is tracked, so it is the repository's and refused.
        scratch.write("Package.swift", "");
        assert!(git(&["add", "-f", "Package.swift"]).status.success());
        assert!(check(&scratch.0).unwrap().iter().any(|e| e.starts_with("Package.swift is not allowed here")));
    }

    #[test]
    fn a_node_modules_folder_in_the_code_is_refused() {
        let scratch = Scratch::new("node-modules");
        scratch.write("ios/sdk/Sources/CraftBridge/node_modules/Plant.swift", "let x = Int(\"4\")!\n");
        scratch.write("plan/node_modules/kit/index.js", "\n");
        scratch.write("ios/sdk/Package.swift", "");
        scratch.write("ios/sdk/.build/checkouts/x/node_modules/y.js", "\n");
        let errors = check(&scratch.0).unwrap();
        // plan/ is the plan kit's, and a tool's checkout under .build is build output: neither is code.
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("ios/sdk/Sources/CraftBridge/node_modules: a node_modules folder"), "{errors:?}");
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
