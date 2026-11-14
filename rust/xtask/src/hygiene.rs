//! One reading of every file, whichever check or tool reads it.
//!
//! Each reader decides for itself what a file is: jscpd skips a file that is not UTF-8 without a word, a
//! marker search that skipped a file holding a NUL byte would miss the marker jscpd still honours, a check
//! that compared names exactly would miss the `inner.RS` or `package.swift` a case-insensitive disk hands
//! the compiler or SwiftPM, and a reader that follows a symbolic link reads what git does not store. So
//! the repository's files are held to one hygiene that leaves no reader such a choice:
//! - every file the gate reads as text — one it knows by extension or name, or a script known by its `#!`
//!   line — is UTF-8 with no NUL byte;
//! - every path is in NFC, the composed form; a decomposed name is a different string to git and to a
//!   case-sensitive disk, and the same file to this machine's;
//! - a name or extension that is a spelling of one the gate knows, in ASCII case or in any fold this
//!   disk makes (the long s for `s`, the Kelvin sign for `k`, the `fi` ligature: see `scan::reads_as`),
//!   is spelt the canonical way, so every other check can compare names exactly;
//! - no path is a symbolic link, so what git stores is what every tool reads;
//! - Rust, Swift, Kotlin and shell sit only in the code folders, which every stage reads; outside them are
//!   only `plan/plan`, the plan kit's launcher, and the measurement records under `plan/evidence/`, which
//!   copy the probes a verification ran exactly as they ran, so no stage may rewrite them.
//!
//! The files are git's — tracked, and untracked ones git would add — the same list that decides what the
//! repository holds; outside a git checkout (a scratch tree), the walk's.

use crate::scan;
use std::path::Path;
use unicode_normalization::is_nfc;

/// The extensions the gate reads code by, each the one spelling a check compares: the four languages' and
/// Kotlin's script, plus Gradle's Groovy ending, which the build-file rule refuses by name.
const EXTENSIONS: &[&str] = &["rs", "swift", "kt", "kts", "sh", "gradle"];

/// Other extensions shellcheck and jscpd read as shell: a script is named `.sh`, so the shell and duplicate
/// stages select it by one ending and never disagree about another.
const OTHER_SHELL_EXTENSIONS: &[&str] = &["bash", "ksh", "dash", "zsh"];

/// The file names a tool or a check opens by name, each in the one spelling a check compares.
const NAMES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "build.rs",
    "rust-toolchain.toml",
    "rust-toolchain",
    "clippy.toml",
    ".clippy.toml",
    "rustfmt.toml",
    ".rustfmt.toml",
    "deny.toml",
    "deny.exceptions.toml",
    ".deny.exceptions.toml",
    "Package.swift",
    "Package.resolved",
    ".swift-format",
    "build.gradle.kts",
    "settings.gradle.kts",
    "build.gradle",
    "settings.gradle",
    "gradle.properties",
    "libs.versions.toml",
    "gradle-wrapper.properties",
    "lint.xml",
    "gradlew",
    "gradlew.bat",
    "buildSrc",
    ".editorconfig",
    ".gitignore",
    ".gitattributes",
    ".shellcheckrc",
    "shellcheckrc",
    ".ignore",
    "licences.toml",
    ".cargo",
];

/// The names Cargo opens inside a `.cargo` folder, which are ordinary names anywhere else.
const CARGO_FOLDER_NAMES: &[&str] = &["config.toml", "config"];

/// The text formats among the names: every name above is read as text but the wrapper's own script for
/// Windows, which Gradle writes, and the folders.
fn read_as_text_by_name(name: &str) -> bool {
    NAMES.contains(&name) && !["gradlew.bat", "buildSrc", ".cargo"].contains(&name)
}

/// The plan's measurement records, which keep the probe code each verification ran exactly as it ran, so
/// no rule about what code or build files sit where reads them.
pub(crate) const EVIDENCE: &str = "plan/evidence/";

/// What is wrong with one path's spelling, without the disk: the NFC rule, an ASCII-case spelling of a
/// known extension or name, and a shell script named otherwise than `.sh`.
pub(crate) fn spelling_problems(path: &str) -> Vec<String> {
    let mut problems = Vec::new();
    if !is_nfc(path) {
        problems.push(format!("{path}: the name is not in NFC: git, a case-sensitive disk and this one would each read a different name; write it composed"));
    }
    let components: Vec<&str> = path.split('/').collect();
    for (index, component) in components.iter().enumerate() {
        let in_cargo_folder = index > 0 && components.get(index - 1) == Some(&".cargo");
        let known = NAMES.iter().chain(in_cargo_folder.then_some(CARGO_FOLDER_NAMES).into_iter().flatten());
        if let Some(canonical) = known.into_iter().find(|name| component.eq_ignore_ascii_case(name) && component != *name) {
            problems.push(format!(
                "{path}: spell it `{canonical}`: a case-insensitive disk opens this where a tool asks for `{canonical}`, and a case-sensitive one ignores it"
            ));
        }
    }
    let name = components.last().copied().unwrap_or_default();
    if let Some((_, extension)) = name.rsplit_once('.') {
        if let Some(canonical) = EXTENSIONS.iter().find(|known| extension.eq_ignore_ascii_case(known) && extension != **known) {
            problems.push(format!("{path}: spell the ending `.{canonical}`: a case-insensitive disk hands this file to the tools that read `.{canonical}`, and the checks that compare names exactly skip it"));
        }
        if OTHER_SHELL_EXTENSIONS.iter().any(|shell| extension.eq_ignore_ascii_case(shell)) {
            problems.push(format!("{path}: name a shell script `.sh`: the shell and duplicate stages select scripts by that one ending or a `#!` line"));
        }
    }
    problems
}

/// The spellings only the disk folds, which [`spelling_problems`] cannot see: does opening a known name,
/// or this file's stem with a known extension, in its folder reach this very file, though it is spelt
/// otherwise? A case-sensitive disk folds nothing, and the answer there is always no.
fn folded_problems(file: &Path, shown: &str) -> Vec<String> {
    let (Some(dir), Some(name)) = (file.parent(), file.file_name().and_then(|name| name.to_str())) else { return Vec::new() };
    let mut problems = Vec::new();
    let in_cargo_folder = dir.file_name().is_some_and(|folder| folder == ".cargo");
    let known = NAMES.iter().chain(in_cargo_folder.then_some(CARGO_FOLDER_NAMES).into_iter().flatten());
    if let Some(canonical) = known.into_iter().find(|known| !known.eq_ignore_ascii_case(name) && scan::reads_as(dir, file, known)) {
        problems.push(format!(
            "{shown}: spell it `{canonical}`: this disk opens it where a tool asks for `{canonical}`, and a check comparing names would skip it"
        ));
    }
    if let Some((stem, extension)) = name.rsplit_once('.')
        && let Some(canonical) = EXTENSIONS.iter().find(|known| !extension.eq_ignore_ascii_case(known) && scan::reads_as(dir, file, &format!("{stem}.{known}")))
    {
        problems.push(format!("{shown}: spell the ending `.{canonical}`: this disk hands the file to the tools that read `.{canonical}`, and the checks that compare names exactly skip it"));
    }
    problems
}

/// Does the gate read this file as text? Code and configuration it knows by extension or name, and every
/// script its `#!` line marks.
fn read_as_text(name: &str, bytes: &[u8]) -> bool {
    let extension = name.rsplit_once('.').map(|(_, extension)| extension).unwrap_or_default();
    EXTENSIONS.contains(&extension) || extension == "toml" || read_as_text_by_name(name) || bytes.starts_with(b"#!")
}

/// What is wrong with a text file's bytes: a NUL byte, which text tools take for a binary file and skip, or
/// a sequence that is not UTF-8, which jscpd skips without a word.
pub(crate) fn text_problem(bytes: &[u8]) -> Option<String> {
    if let Some(at) = bytes.iter().position(|byte| *byte == 0) {
        return Some(format!("a NUL byte at offset {at}: text tools take the file for a binary one and skip it"));
    }
    std::str::from_utf8(bytes).err().map(|error| format!("not UTF-8 from offset {}: jscpd skips such a file without saying so", error.valid_up_to()))
}

/// Is a path outside the code folders a file of the four languages? `plan/plan` is the one script there,
/// and the evidence records the one place a verification's own probe code is kept as it ran.
fn code_outside_the_trees(shown: &str, file: &Path) -> bool {
    let top = shown.split('/').next().unwrap_or_default();
    if crate::files::CODE_TREES.contains(&top) || shown == "plan/plan" || shown.starts_with(EVIDENCE) || !shown.contains('/') {
        return false;
    }
    let extension = file.extension().and_then(|extension| extension.to_str()).unwrap_or_default();
    EXTENSIONS.contains(&extension) || crate::files::is_shell_script(file)
}

/// The path as the disk stores it, component by component: git on a case-insensitive disk lists a tracked
/// file under the name it tracks even after a case-only rename on disk (`Package.swift` stored as
/// `package.swift`), and every tool opens the stored file, so the spelling rules judge that one. Each
/// component is matched by its inode, which answers for every fold the disk makes.
fn stored_spelling(root: &Path, listed: &str, folders: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<(String, u64)>>) -> String {
    use std::os::unix::fs::MetadataExt;
    let mut dir = root.to_path_buf();
    let mut stored = Vec::new();
    for component in listed.split('/') {
        let entries = folders.entry(dir.clone()).or_insert_with(|| {
            std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|entry| Some((entry.file_name().into_string().ok()?, std::fs::symlink_metadata(entry.path()).ok()?.ino())))
                .collect()
        });
        let wanted = std::fs::symlink_metadata(dir.join(component)).map(|metadata| metadata.ino()).ok();
        let name = entries
            .iter()
            .find(|(name, _)| name == component)
            .or_else(|| entries.iter().find(|(_, inode)| Some(*inode) == wanted))
            .map_or(component, |(name, _)| name.as_str())
            .to_string();
        dir = dir.join(&name);
        stored.push(name);
    }
    stored.join("/")
}

/// Run the check over every file the repository holds.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    let mut folders = std::collections::BTreeMap::new();
    for listed in repository_files(root)? {
        let shown = stored_spelling(root, &listed, &mut folders);
        if shown != listed {
            errors.push(format!(
                "{shown}: git tracks it as `{listed}`: a rename that only changes how the disk folds the name is one git here cannot see, so name it back"
            ));
        }
        let file = root.join(&shown);
        errors.extend(spelling_problems(&shown));
        let metadata = std::fs::symlink_metadata(&file).map_err(|error| format!("{shown}: {error}"))?;
        if metadata.file_type().is_symlink() {
            errors.push(format!("{shown}: a symbolic link: git stores the link while the tools read what it points at, so no path in the repository is one"));
            continue;
        }
        errors.extend(folded_problems(&file, &shown));
        if code_outside_the_trees(&shown, &file) {
            errors.push(format!(
                "{shown}: Rust, Swift, Kotlin and shell live in the code folders, which every stage reads; plan/ holds the plan kit, whose one script is plan/plan, and the evidence records"
            ));
        }
        let name = shown.rsplit('/').next().unwrap_or_default();
        let bytes = std::fs::read(&file).map_err(|error| format!("{shown}: {error}"))?;
        if read_as_text(name, &bytes)
            && let Some(problem) = text_problem(&bytes)
        {
            errors.push(format!("{shown}: {problem}"));
        }
    }
    Ok(errors)
}

/// The repository's files: git's tracked and would-be-added files in a checkout, the walk's elsewhere.
/// A tracked file deleted from the working tree is no file to read. Every rule about which files the
/// repository holds reads this one list, so a file git ignores is outside them all alike (AGENTS.md §1).
pub(crate) fn repository_files(root: &Path) -> Result<Vec<String>, String> {
    let mut files = if crate::git::is_checkout(root) {
        crate::git::listed(root, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"])?
    } else {
        scan::walk(root, "")?.into_iter().map(|file| file.relative).collect()
    };
    files.retain(|file| std::fs::symlink_metadata(root.join(file)).is_ok());
    files.sort();
    files.dedup();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn every_known_spelling_but_the_canonical_one_is_refused() {
        for path in [
            "rust/core/x/src/inner.RS",
            "ios/sdk/Sources/CraftBridge/Api.Swift",
            "android/sdk/core/src/main/kotlin/x/A.KT",
            "android/sdk/core/build.gradle.KTS",
            "tools/run.SH",
            "ios/sdk/package.swift",
            "android/sdk/core/GRADLE.PROPERTIES",
            "rust/core/x/Build.rs",
            "rust/core/x/CLIPPY.toml",
            "rust/.CARGO/config.toml",
            "rust/.cargo/CONFIG.toml",
            "android/sdk/Build.gradle.kts",
            "rust/core/x/.EditorConfig",
            "rust/core/.GitIgnore",
        ] {
            assert!(!spelling_problems(path).is_empty(), "{path}");
        }
        for path in [
            "rust/core/x/src/inner.rs",
            "ios/sdk/Package.swift",
            "android/sdk/gradle.properties",
            "rust/.cargo/config.toml",
            "docs/Config.toml",
            "rust/xtask/src/config.rs",
        ] {
            assert!(spelling_problems(path).is_empty(), "{path}: {:?}", spelling_problems(path));
        }
    }

    #[test]
    fn a_shell_script_is_named_sh() {
        for path in ["tools/lib.bash", "tools/x.ksh", "tools/y.zsh", "tools/z.dash"] {
            assert!(spelling_problems(path).iter().any(|e| e.contains("name a shell script `.sh`")), "{path}");
        }
    }

    #[test]
    fn a_decomposed_name_is_refused() {
        let decomposed = "ios/e\u{301}tude/notes.md";
        assert!(spelling_problems(decomposed).iter().any(|e| e.contains("not in NFC")), "{decomposed}");
        assert!(spelling_problems("ios/\u{e9}tude/notes.md").is_empty());
    }

    #[test]
    fn a_nul_byte_or_a_byte_that_is_not_utf8_is_refused_in_text() {
        assert!(text_problem(b"// \0\nfn x() {}\n").is_some_and(|problem| problem.contains("NUL")));
        assert!(text_problem(b"# caf\xe9\necho\n").is_some_and(|problem| problem.contains("not UTF-8")));
        assert!(text_problem("// café\n".as_bytes()).is_none());
        assert!(read_as_text("plant.sh", b"echo") && read_as_text("run", b"#!/bin/sh\n") && read_as_text("Cargo.toml", b""));
        assert!(!read_as_text("gradle-wrapper.jar", b"PK\x03\x04") && !read_as_text("icon.png", b"\x89PNG"));
    }

    /// The folds a case-insensitive volume makes beyond ASCII case: probed, since on a case-sensitive
    /// volume the same plant is just another file and must pass.
    #[test]
    fn a_spelling_the_disk_folds_is_refused_where_the_disk_folds_it() {
        let scratch = Scratch::new("hygiene-fold");
        for (plant, plain) in [
            ("rust/core/x/src/inner.r\u{17F}", "rust/core/x/src/inner.rs"),
            ("rust/core/x/ru\u{17F}tfmt.toml", "rust/core/x/rustfmt.toml"),
            ("android/sdk/gradle.propertie\u{17F}", "android/sdk/gradle.properties"),
            ("rust/.cargo/con\u{FB01}g", "rust/.cargo/config"),
        ] {
            scratch.write(plant, "x\n");
            let folded = std::fs::canonicalize(scratch.0.join(plain)).ok() == std::fs::canonicalize(scratch.0.join(plant)).ok();
            let errors = check(&scratch.0).unwrap();
            if folded {
                assert!(errors.iter().any(|e| e.starts_with(&format!("{plant}: spell"))), "{plant}: {errors:?}");
            } else {
                assert!(!errors.iter().any(|e| e.contains("spell")), "{plant}: {errors:?}");
            }
            std::fs::remove_file(scratch.0.join(plant)).unwrap();
        }
    }

    #[test]
    fn a_symbolic_link_and_code_outside_the_code_folders_are_refused() {
        let scratch = Scratch::new("hygiene-links");
        crate::git::output(&scratch.0, &["init", "-q"]).unwrap();
        scratch.write("docs-outside/Linked.swift", "\n");
        scratch.write("plan/plan", "#!/bin/sh\n");
        scratch.write("plan/build/kit.mjs", "\n");
        assert!(check(&scratch.0).unwrap().iter().all(|e| e.starts_with("docs-outside/")), "plan/plan and the kit's JavaScript are the plan's");
        std::fs::create_dir_all(scratch.0.join("ios/sdk/Sources/CraftBridge")).unwrap();
        std::os::unix::fs::symlink("../../../../docs-outside/Linked.swift", scratch.0.join("ios/sdk/Sources/CraftBridge/Linked.swift")).unwrap();
        std::os::unix::fs::symlink("../docs-outside", scratch.0.join("docs")).unwrap();
        scratch.write("plan/tools/copy.sh", "echo x\n");
        scratch.write("plan/evidence/v1/review-scripts/run-sim.sh", "echo x\n");
        scratch.write("plan/tools/run", "#!/usr/bin/env bash\necho x\n");
        let errors = check(&scratch.0).unwrap();
        for expected in [
            "ios/sdk/Sources/CraftBridge/Linked.swift: a symbolic link",
            "docs: a symbolic link",
            "plan/tools/copy.sh: Rust, Swift, Kotlin and shell",
            "plan/tools/run: Rust, Swift",
        ] {
            assert!(errors.iter().any(|e| e.starts_with(expected)), "{expected}: {errors:?}");
        }
    }

    #[test]
    fn a_tracked_file_renamed_in_case_on_disk_is_judged_as_the_disk_stores_it() {
        let scratch = Scratch::new("hygiene-stored");
        crate::git::output(&scratch.0, &["init", "-q"]).unwrap();
        scratch.write("ios/sdk/Package.swift", "// swift-tools-version: 6.0\n");
        crate::git::output(&scratch.0, &["add", "ios/sdk/Package.swift"]).unwrap();
        std::fs::rename(scratch.0.join("ios/sdk/Package.swift"), scratch.0.join("ios/sdk/package.swift")).unwrap();
        let tracked = crate::git::listed(&scratch.0, &["ls-files", "-z"]).unwrap();
        let errors = check(&scratch.0).unwrap();
        if tracked == ["ios/sdk/Package.swift"] && std::fs::symlink_metadata(scratch.0.join("ios/sdk/Package.swift")).is_ok() {
            // A case-insensitive disk: git still lists the tracked name.
            assert!(errors.iter().any(|e| e.starts_with("ios/sdk/package.swift: git tracks it as `ios/sdk/Package.swift`")), "{errors:?}");
        }
        assert!(errors.iter().any(|e| e.starts_with("ios/sdk/package.swift: spell it `Package.swift`")), "{errors:?}");
    }

    #[test]
    fn the_check_reads_the_repository_and_finds_nothing() {
        let errors = check(&crate::repo_root()).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
