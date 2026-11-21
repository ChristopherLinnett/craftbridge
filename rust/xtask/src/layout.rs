//! Rule 1 — layout: what may sit at the repository root and inside a platform folder.
//!
//! A root that accumulates loose directories is a repository where people start putting platform code
//! where no build reads it, and a platform folder that holds more than `sdk` and `samples` stops being
//! the complete build of its own that rule 1 promises. So the names are listed once and everything else
//! is an error; `.DS_Store` is ignored because macOS writes it no matter what the repository asks.

use crate::scan;
use std::path::Path;

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

/// Check one directory against an allowlist, returning one error per unexpected entry and per allowed
/// name of the wrong kind. `hint` finishes the sentence so the root and the platform folders share this
/// one function. A directory that cannot be listed is an error, never an empty listing, and so is a git
/// that cannot say which of its entries it ignores.
fn entries_violations(root: &Path, dir: &Path, allowed: &[(&str, Entry)], hint: &str) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let mut found = Vec::new();
    for entry in entries {
        found.push(entry.map_err(|error| format!("{}: {error}", dir.display()))?.path());
    }
    let mut names: Vec<String> = crate::git::in_repository(dir, found, |path| path.as_path())?
        .iter()
        .filter_map(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
        .collect();
    names.sort();
    let prefix = if dir == root { String::new() } else { format!("{}/", crate::report::shown(root, dir)) };
    let mut errors = Vec::new();
    for name in names.iter().filter(|name| !IGNORED_NAMES.contains(&name.as_str())) {
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
/// code inside it would build while no rule, formatter or duplicate check read it. One that holds nothing
/// git would add is no part of the repository (§1), so the folders found are put to git in one question,
/// as the walk's files are; what a build compiles from one on this machine alone is the build stages'
/// own refusal, of a file they compile that no check reads.
fn hidden_package_folders(root: &Path) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    for tree in crate::files::CODE_TREES {
        let mut stack = vec![root.join(tree)];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() || scan::is_build_output(&path) {
                    continue;
                }
                if entry.file_name() == "node_modules" {
                    found.push(path);
                } else if entry.file_name() != ".git" {
                    stack.push(path);
                }
            }
        }
    }
    let mut errors: Vec<String> = crate::git::in_repository(root, found, |path| path.as_path())?
        .iter()
        .map(|path| {
            format!(
                "{}: a node_modules folder in the code: the platform builds compile what it holds while every rule skips it",
                crate::report::shown(root, path)
            )
        })
        .collect();
    errors.sort();
    Ok(errors)
}

/// The files each toolchain reads to build, with the one platform folder they belong to. A build file
/// anywhere else is read by no build — cargo, SwiftPM and Gradle each read their own folder — yet tells
/// everyone the folder builds something: in `conformance/`, `tools/` or `docs/`, in another platform's
/// folder (a `Cargo.toml` under `ios/` is a crate the workspace, its lint table and its rules never see),
/// or in `plan/`, where `rules: hygiene` refuses the Swift and Gradle scripts but not a `.toml`.
const BUILD_FILES: &[(&str, &str)] = &[
    ("Cargo.toml", "rust"),
    ("Package.swift", "ios"),
    ("build.gradle", "android"),
    ("build.gradle.kts", "android"),
    ("settings.gradle", "android"),
    ("settings.gradle.kts", "android"),
];

/// Every build file the repository holds outside its own platform folder, each message naming the file.
/// The files are the repository's, as `rules: hygiene` lists them, so a file git ignores is no part of it
/// (§1). The root is the root list's to judge, and the probes kept under `plan/evidence/` stay as they ran.
fn build_files_outside_their_folders(root: &Path) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    for file in crate::hygiene::repository_files(root)? {
        let Some((top, _)) = file.split_once('/') else { continue };
        if file.starts_with(crate::hygiene::EVIDENCE) {
            continue;
        }
        let name = file.rsplit('/').next().unwrap_or_default();
        if let Some((_, own)) = BUILD_FILES.iter().find(|(build, own)| *build == name && *own != top) {
            errors.push(format!(
                "{file}: a {name} outside {own}/: each toolchain's build files are read only in their own folder, so this belongs to no build (AGENTS.md §1)"
            ));
        }
    }
    errors.sort();
    Ok(errors)
}

/// Hand-written Java anywhere under `android/sdk`. The Android SDK is Kotlin (AGENTS.md §9), and its
/// never-crash and JNI rules (§3, §4) are detekt rules over Kotlin's own syntax tree: a `.java` file
/// would compile into the AAR beside the Kotlin, checked by none of them. Build output is skipped as
/// everywhere (a Gradle-generated `build/` may hold Java), and the extension is matched without case
/// because this machine's file system is, so a `Foo.Java` would compile for a case-insensitive compiler
/// while a case-sensitive check looked away.
fn hand_written_java(root: &Path) -> Result<Vec<String>, String> {
    let sdk = root.join("android").join("sdk");
    let mut errors = Vec::new();
    for file in scan::walk(&sdk, "")?.into_iter().filter(|file| file.path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("java"))) {
        errors.push(format!(
            "{}: hand-written Java: the Android SDK is Kotlin, so its never-crash and JNI rules read Kotlin's own syntax tree and see none of this file",
            crate::report::shown(root, &file.path)
        ));
    }
    Ok(errors)
}

/// Run the layout check: the root, then `ios/` and `android/` when they exist, the code trees for a
/// folder every rule would skip, every file for a platform's build file outside that platform's folder, and no
/// hand-written Java under the Android SDK.
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
    errors.extend(build_files_outside_their_folders(root)?);
    errors.extend(hand_written_java(root)?);
    errors.extend(kept_templates(root)?);
    Ok(errors)
}

/// The extension of an EffectCraft template.
const TEMPLATE_EXTENSION: &str = "ectemplate";

/// Every template the repository holds, its extension in any case a disk folds: the running example's template is
/// made from its recipe, `conformance/effects/birthday-card.jsonl`, by `tools/birthday-card.sh` whenever a sample is
/// built, so a kept one is a second template, made some other way, that a sample could bundle in its place.
fn kept_templates(root: &Path) -> Result<Vec<String>, String> {
    Ok(crate::hygiene::repository_files(root)?
        .into_iter()
        .filter(|file| Path::new(file).extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case(TEMPLATE_EXTENSION)))
        .map(|file| {
            format!(
                "{file}: a kept template: the running example's template is made from its recipe, conformance/effects/birthday-card.jsonl, by tools/birthday-card.sh whenever a sample is built, and the repository keeps no copy of one (AGENTS.md §10)"
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn a_compliant_root_passes() {
        let scratch = Scratch::new("ok");
        // Every name but `.git`, which would make the scratch a checkout whose files only git can list; the
        // checkout tests below hold one.
        for (name, kind) in ALLOWED_ROOT.iter().filter(|(name, _)| *name != ".git") {
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
        scratch.write("somewhere-else/notes.md", "x\n");
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
        scratch.write("README.md/notes.md", "x\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e == "ios must be a folder, and here it is a file"), "{errors:?}");
        assert!(errors.iter().any(|e| e == "README.md must be a file, and here it is a folder"), "{errors:?}");
    }

    #[test]
    fn a_name_git_ignores_is_not_the_repository_s() {
        let scratch = Scratch::new("ignored");
        let git = |args: &[&str]| crate::git::output(&scratch.0, args).unwrap();
        git(&["init", "-q"]);
        scratch.write(".gitignore", "**/.claude/settings.local.json\n.vscode/\nPackage.swift\n");
        // A folder whose only file is ignored holds nothing the repository can, as git sees it.
        scratch.write(".claude/settings.local.json", "{}\n");
        scratch.write(".vscode/settings.json", "{}\n");
        scratch.write("android/.idea/workspace.xml", "<x/>\n");
        scratch.write("android/.gitignore", ".idea/\n");
        git(&["add", "-f", "android/.gitignore"]);
        let errors = check(&scratch.0).unwrap();
        assert_eq!(
            errors,
            ["android/.gitignore is not allowed here: only `sdk` and `samples` belong in android/"],
            "only the tracked file is the repository's"
        );
        // A file forced past the ignore rules is tracked, so it is the repository's and refused.
        scratch.write("Package.swift", "");
        git(&["add", "-f", "Package.swift"]);
        assert!(check(&scratch.0).unwrap().iter().any(|e| e.starts_with("Package.swift is not allowed here")));
    }

    #[test]
    fn an_empty_folder_holds_nothing_git_would_add_so_a_checkout_skips_it() {
        let scratch = Scratch::new("empty-folder");
        crate::git::output(&scratch.0, &["init", "-q"]).unwrap();
        scratch.mkdir("ios/extra");
        scratch.mkdir("swift");
        assert!(check(&scratch.0).unwrap().is_empty(), "git cannot hold an empty folder, so it is no part of the repository");
        scratch.write("ios/extra/notes.md", "x\n");
        scratch.write("swift/notes.md", "x\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 2, "a file git would add makes the folder the repository's: {errors:?}");
    }

    /// A folder holds what git lists under it at a folder boundary, never what a sibling whose name begins
    /// with the folder's holds: the ignored `release/` a staging run leaves stays no part of the repository
    /// beside a `release-notes.md` git would add, which is refused alone.
    #[test]
    fn an_ignored_folder_is_not_held_by_a_sibling_whose_name_begins_with_its_own() {
        let scratch = Scratch::new("layout-prefix");
        crate::git::output(&scratch.0, &["init", "-q"]).unwrap();
        scratch.write(".gitignore", "/release/\n");
        scratch.write("release/0.1.0/notes.md", "x\n");
        scratch.write("release-notes.md", "x\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("release-notes.md is not allowed here"), "{errors:?}");
    }

    /// The child half of the test below: run only when the parent starts it with a hook's environment.
    #[test]
    #[ignore = "run by the_check_in_a_hook_s_environment_judges_its_own_tree, in a hook's environment"]
    fn child_checks_its_own_tree() {
        let Some(own) = std::env::var_os("XTASK_LAYOUT_OWN") else { return };
        let errors = check(Path::new(&own)).unwrap();
        assert_eq!(errors, ["android/stray is not allowed here: only `sdk` and `samples` belong in android/"]);
    }

    #[test]
    fn the_check_in_a_hook_s_environment_judges_its_own_tree() {
        let own = Scratch::new("layout-own");
        let other = Scratch::new("layout-other");
        crate::git::output(&own.0, &["init", "-q"]).unwrap();
        crate::git::output(&other.0, &["init", "-q"]).unwrap();
        own.write(".gitignore", "/android/.idea/\n");
        own.write("android/.idea/workspace.xml", "<x/>\n");
        own.write("android/stray/notes.md", "x\n");
        craftbridge_process::rerun(
            "layout::tests::child_checks_its_own_tree",
            &[("GIT_DIR", other.0.join(".git").as_os_str()), ("XTASK_LAYOUT_OWN", own.0.as_os_str())],
            "the check in a hook's environment did not judge its own tree",
        )
        .unwrap();
    }

    #[test]
    fn a_name_stored_decomposed_is_the_repository_s_and_refused() {
        let scratch = Scratch::new("layout-nfd");
        crate::git::output(&scratch.0, &["init", "-q"]).unwrap();
        scratch.write("ios/e\u{301}tude/notes.md", "x\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors, ["ios/e\u{301}tude is not allowed here: only `sdk` and `samples` belong in ios/"]);
    }

    #[test]
    fn a_kept_template_is_refused_in_any_case_of_its_extension() {
        let scratch = Scratch::new("templates");
        scratch.write("android/samples/card/app/src/main/assets/Birthday Card.ectemplate", "x");
        scratch.write("ios/samples/Card/Card.ECTemplate", "x");
        scratch.write("conformance/effects/fixtures/card.ectemplate.json", "{}");
        let mut errors: Vec<String> = check(&scratch.0).unwrap().into_iter().filter(|error| error.contains("template")).collect();
        errors.sort();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].starts_with("android/samples/card/app/src/main/assets/Birthday Card.ectemplate: a kept template"), "{errors:?}");
        assert!(errors[1].starts_with("ios/samples/Card/Card.ECTemplate: a kept template"), "{errors:?}");
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

    /// A `node_modules` folder git ignores holds nothing the repository can, so it is no part of the code
    /// (§1), at any depth and from each ignore source: a root pattern, a folder's own `.gitignore`, and the
    /// checkout's `info/exclude`. One whose files git would add is refused, and so is an ignored one once a
    /// file in it is tracked past the ignore rules.
    #[test]
    fn a_node_modules_folder_git_ignores_is_no_part_of_the_code() {
        let scratch = Scratch::new("node-modules-ignored");
        let git = |args: &[&str]| crate::git::output(&scratch.0, args).unwrap();
        git(&["init", "-q"]);
        scratch.write(".gitignore", "/tools/node_modules/\n");
        scratch.write("tools/node_modules/kit/index.js", "\n");
        scratch.write("docs/guide/.gitignore", "node_modules/\n");
        scratch.write("docs/guide/node_modules/kit/index.js", "\n");
        scratch.write(".git/info/exclude", "/ios/sdk/Sources/CraftBridge/node_modules/\n");
        scratch.write("ios/sdk/Sources/CraftBridge/node_modules/Plant.swift", "let x = Int(\"4\")!\n");
        scratch.write("conformance/effects/node_modules/kit/index.js", "\n");
        let errors = hidden_package_folders(&scratch.0).unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("conformance/effects/node_modules: a node_modules folder"), "{errors:?}");
        git(&["add", "-f", "ios/sdk/Sources/CraftBridge/node_modules/Plant.swift"]);
        let errors = hidden_package_folders(&scratch.0).unwrap();
        assert_eq!(errors.len(), 2, "tracked, the folder is the repository's again: {errors:?}");
        assert!(errors.iter().any(|error| error.starts_with("ios/sdk/Sources/CraftBridge/node_modules: a node_modules folder")), "{errors:?}");
    }

    #[test]
    fn hand_written_java_under_the_android_sdk_is_refused() {
        let scratch = Scratch::new("java");
        scratch.write("android/sdk/core/src/main/java/io/x/Plant.java", "class Plant {}\n");
        scratch.write("android/sdk/effects/src/test/java/PlantTest.java", "class PlantTest {}\n");
        // A case variant: a case-insensitive file system would hand it to the compiler in place of a
        // `.java`, so the check must see it too.
        scratch.write("android/sdk/core/src/main/java/Variant.Java", "class Variant {}\n");
        // Build output is skipped as everywhere: Gradle writes Java there itself. A `build` counts as
        // such only beside the Gradle file it belongs to, so the module's build file sits beside it.
        scratch.write("android/sdk/core/build.gradle.kts", "");
        scratch.write("android/sdk/core/build/generated/FromGradle.java", "class FromGradle {}\n");
        // Kotlin is read by detekt, not here, and samples are not the SDK.
        scratch.write("android/sdk/core/src/main/kotlin/Ok.kt", "package io.github.christopherlinnett.craftbridge.core\n");
        scratch.write("android/samples/app/src/main/java/App.java", "class App {}\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 3, "{errors:?}");
        for path in [
            "android/sdk/core/src/main/java/io/x/Plant.java",
            "android/sdk/effects/src/test/java/PlantTest.java",
            "android/sdk/core/src/main/java/Variant.Java",
        ] {
            assert!(errors.iter().any(|e| e.starts_with(&format!("{path}: hand-written Java"))), "{path}: {errors:?}");
        }
    }

    #[test]
    fn a_platform_folder_holds_only_sdk_and_samples() {
        let scratch = Scratch::new("platform");
        scratch.mkdir("ios/sdk");
        scratch.mkdir("ios/samples");
        scratch.write("ios/.DS_Store", "x");
        scratch.write("ios/extra/notes.md", "x\n");
        scratch.write("android/third_party/notes.md", "x\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("extra") && e.contains("only `sdk` and `samples` belong in ios/")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("third_party") && e.contains("android/")), "{errors:?}");
    }

    #[test]
    fn a_platform_build_file_outside_its_own_platform_folder_is_refused() {
        let scratch = Scratch::new("build-files");
        let planted = [
            ("conformance/Cargo.toml", "rust"),
            ("conformance/probe/Package.swift", "ios"),
            ("tools/Cargo.toml", "rust"),
            ("tools/probe/build.gradle", "android"),
            ("tools/probe/build.gradle.kts", "android"),
            ("docs/app/settings.gradle", "android"),
            ("docs/app/settings.gradle.kts", "android"),
            (".github/Cargo.toml", "rust"),
            // Another platform's folder is no more its own: no build there reads it.
            ("ios/sdk/rustlib/Cargo.toml", "rust"),
            ("android/sdk/swift/Package.swift", "ios"),
            ("rust/swift/Package.swift", "ios"),
            ("rust/core/x/build.gradle.kts", "android"),
            ("ios/sdk/settings.gradle", "android"),
            // Nor is the plan, outside the probes its evidence keeps as they ran.
            ("plan/tasks/Cargo.toml", "rust"),
        ];
        for (path, _) in planted {
            scratch.write(path, "");
        }
        scratch.write("conformance/notes.md", "x\n");
        scratch.write("plan/evidence/probe/Cargo.toml", "");
        scratch.write("plan/evidence/v7/app/settings.gradle.kts", "");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), planted.len(), "{errors:?}");
        for (path, own) in planted {
            let name = path.rsplit('/').next().unwrap_or_default();
            assert!(errors.iter().any(|e| e.starts_with(&format!("{path}: a {name} outside {own}/: "))), "{path}: {errors:?}");
        }
        assert!(errors.iter().all(|e| e.contains("belongs to no build")), "{errors:?}");
    }

    #[test]
    fn a_build_file_git_ignores_is_not_the_repository_s() {
        let scratch = Scratch::new("build-files-ignored");
        crate::git::output(&scratch.0, &["init", "-q"]).unwrap();
        scratch.write(".gitignore", "/tools/scratch/\n");
        scratch.write("tools/scratch/Cargo.toml", "");
        scratch.write("tools/kept/Cargo.toml", "");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("tools/kept/Cargo.toml: a Cargo.toml outside rust/"), "{errors:?}");
    }

    #[test]
    fn build_files_stay_legal_inside_their_own_platform_trees() {
        let scratch = Scratch::new("build-files-legal");
        scratch.write("rust/Cargo.toml", "");
        scratch.write("rust/core/craftbridge-core/Cargo.toml", "");
        scratch.write("ios/sdk/Package.swift", "");
        scratch.write("android/sdk/build.gradle.kts", "");
        scratch.write("android/sdk/settings.gradle.kts", "");
        scratch.write("android/sdk/core/build.gradle.kts", "");
        // A sample is its platform's too, consuming the library the way an app does (§1).
        scratch.write("ios/samples/Viewer/Package.swift", "");
        scratch.write("android/samples/viewer/settings.gradle.kts", "");
        assert!(check(&scratch.0).unwrap().is_empty());
    }
}
