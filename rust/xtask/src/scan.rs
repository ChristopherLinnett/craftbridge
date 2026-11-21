//! The shared machinery every check that reads source files uses: one file walker and one text reader.
//!
//! Checks must agree about what they are looking at. Two walkers would mean one check seeing a file
//! another skipped, so the exclusions live here once, and each check only says which extension it wants.
//! What the rules read is decided here too: the walk yields only what git would add, so no rule that
//! walks a tree reads a file git ignores, and none asks git for itself (AGENTS.md §1). What a language's
//! source means is never decided here: Rust is read through `syn` (`crate::rust_syntax`), Swift through
//! the compiler's language server and Kotlin through detekt.

use std::fs;
use std::path::{Path, PathBuf};

/// Build output and tool caches, each skipped only where its tool writes it: beside the file that tool
/// reads. Matching the name alone would also skip source that happens to share it — a Rust module
/// `target/`, a Kotlin package `build/` — and every rule would silently stop reading that code. This is
/// the gate's one definition of build output: the shell, Swift-format and duplicate stages read the
/// file lists this walk makes (`cargo xtask files`).
const BUILD_OUTPUTS: &[(&str, &[&str])] = &[
    ("target", &["Cargo.toml"]),
    (".build", &["Package.swift"]),
    (".swiftpm", &["Package.swift"]),
    // The built xcframework `tools/build-xcframework.sh` writes beside the package, never committed.
    ("Frameworks", &["Package.swift"]),
    ("build", &["build.gradle.kts", "settings.gradle.kts"]),
    (".gradle", &["build.gradle.kts", "settings.gradle.kts"]),
    (".kotlin", &["build.gradle.kts", "settings.gradle.kts"]),
];

/// Directories that never hold this repository's source, wherever they sit.
const NEVER_SOURCE: &[&str] = &[".git", "node_modules"];

/// Does reading `dir/expected` read the file `found`?
///
/// A case-insensitive file system answers a lookup for a name with any spelling of it it holds, and
/// it folds more than ASCII case: probed on this APFS volume, a name carrying the Kelvin sign (U+212A)
/// is found under a plain `k`, one carrying a long s (U+017F) under a plain `s`, and an `fi` ligature
/// under `fi`. A tool that looks its file up by name — Gradle a project's `build.gradle.kts`, a
/// formatter its `rustfmt.toml` — therefore reads such a file where it asked for the plain name, just
/// as it reads a case variant, so a check that matched stored names string by string would leave that
/// file unread while every tool still ran it. Answering through the file system — resolve the name the
/// way an open of it resolves, and compare the file resolved — covers every fold this machine makes,
/// known to the check or not, and says no everywhere on a case-sensitive volume, so the check needs no
/// list of folds and no platform branches.
pub(crate) fn reads_as(dir: &Path, found: &Path, expected: &str) -> bool {
    match (fs::canonicalize(dir.join(expected)), fs::canonicalize(found)) {
        (Ok(opened), Ok(walked)) => opened == walked,
        _ => false,
    }
}

/// One source file found by [`walk`].
#[derive(Debug)]
pub(crate) struct FoundFile {
    /// Absolute path on disk.
    pub(crate) path: PathBuf,
    /// Path relative to the directory the walk started in, `/` separated: what a message names.
    pub(crate) relative: String,
    /// Directory names between the walk root and this file, innermost last. A rule that lives "under
    /// `Platform/`" means a component, not a substring, so it keys off these instead of a path prefix.
    pub(crate) components: Vec<String>,
}

/// Is `dir`, named `name`, build output or a cache rather than source?
fn skip_dir(dir: &Path, name: &str) -> bool {
    NEVER_SOURCE.contains(&name) || is_build_output(dir)
}

/// Is `dir` a folder a build tool writes its output or cache into, beside the file that tool reads?
pub(crate) fn is_build_output(dir: &Path) -> bool {
    let name = dir.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    BUILD_OUTPUTS.iter().any(|(output, beside)| *output == name && dir.parent().is_some_and(|parent| beside.iter().any(|file| parent.join(file).is_file())))
}

/// Walk a tree depth first, yielding files whose name ends with `extension` — every file when it is
/// empty — and skipping build output. A missing root is an empty tree; a directory that exists but
/// cannot be read is an error, because a rule that cannot see a file must not report it clean. The
/// extension is compared exactly: `rules: hygiene` refuses every other spelling of a known one (an
/// `inner.RS` a case-insensitive disk would compile), so no rule needs to guess at folds.
///
/// What the walk yields is what git would add: a file that holds nothing git would add is no part of the
/// repository (AGENTS.md §1), so it is dropped here, where every rule's file list comes from, at any depth
/// and whichever file made git ignore it, and no rule reads it by walking again. The whole walk is one
/// question to git, asked at the walk's root (`git::in_repository`), never one per file. In a checkout
/// whose git cannot answer, or cannot start, the walk fails in git's own words rather than letting every
/// file read as tracked; a root no checkout holds, such as a scratch tree a test builds, is its own files.
///
/// The rules whose subject is what git does not track read ignored files outside this walk, asking git
/// themselves: `cargo xtask generate` refuses generated output git leaves untracked, ignored ones included
/// (`generate::uncommitted` lists `ls-files --others` with no ignore rule), and `tools/env.sh` refuses a
/// leftover Xcode scheme or per-user setting beside a checkout
/// (`craftbridge_refuse_leftover_xcode_and_swiftpm_settings`, which refuses every file there git does not
/// track, ignored or not). A file a build compiles that this walk drops is no gap: the stages that ask a
/// build what it compiles refuse each such file no rule reads (`compiled::judge`, `direction::target_errors`).
/// A new rule whose subject is untracked state names itself here.
pub(crate) fn walk(root: &Path, extension: &str) -> Result<Vec<FoundFile>, String> {
    let mut files = Vec::new();
    if !root.is_dir() {
        return Ok(files);
    }
    let mut stack: Vec<(PathBuf, Vec<String>)> = vec![(root.to_path_buf(), Vec::new())];
    while let Some((dir, components)) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("{}: {error}", dir.display()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if path.is_dir() {
                if !skip_dir(&path, &name) {
                    let mut deeper = components.clone();
                    deeper.push(name);
                    stack.push((path, deeper));
                }
            } else if extension.is_empty() || name.ends_with(extension) {
                let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                files.push(FoundFile { path, relative, components: components.clone() });
            }
        }
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    // One question for the whole walk, so git answers a tree per listing rather than per file.
    crate::git::in_repository(root, files, |file| &file.path)
}

/// Read a file as UTF-8 text, without the byte-order mark an editor may write first. Every tool the
/// gate runs skips it — swift-format then honours a directive on the first line, a compiler reads the
/// first line as code — so a rule that counted it as text would read a different first line than they do.
pub(crate) fn read_text(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map(|text| without_bom(&text).to_string()).map_err(|error| format!("{}: {error}", path.display()))
}

/// Text without a leading byte-order mark.
pub(crate) fn without_bom(text: &str) -> &str {
    text.strip_prefix('\u{FEFF}').unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    fn relative_names(files: &[FoundFile]) -> Vec<&str> {
        files.iter().map(|found| found.relative.as_str()).collect()
    }

    #[test]
    fn walker_skips_build_output_beside_its_manifest_only() {
        let root = Scratch::new("walker");
        root.write("Cargo.toml", "");
        root.write("target/debug/lib.rs", "// x");
        root.write("src/lib.rs", "// x");
        root.write("src/node_modules/lib.rs", "// x");
        // A module that shares a build directory's name is source, so every rule must read it.
        root.write("src/target/mod.rs", "// x");
        root.write("src/build/mod.rs", "// x");
        let files = walk(&root.0, ".rs").unwrap();
        assert_eq!(relative_names(&files), ["src/build/mod.rs", "src/lib.rs", "src/target/mod.rs"]);
    }

    #[test]
    fn walker_skips_gradle_and_swiftpm_output() {
        let root = Scratch::new("walker-platforms");
        root.write("sdk/settings.gradle.kts", "");
        root.write("sdk/build/generated/A.kt", "");
        root.write("sdk/.gradle/A.kt", "");
        root.write("sdk/core/build.gradle.kts", "");
        root.write("sdk/core/build/tmp/A.kt", "");
        root.write("sdk/core/src/main/kotlin/A.kt", "");
        root.write("pkg/Package.swift", "");
        root.write("pkg/.build/checkouts/A.kt", "");
        assert_eq!(relative_names(&walk(&root.0, ".kt").unwrap()), ["sdk/core/src/main/kotlin/A.kt"]);
    }

    #[test]
    fn walker_records_the_containing_directories_not_the_file_name() {
        let root = Scratch::new("walker-components");
        root.write("Sources/Platform/Buffer.swift", "// x");
        let files = walk(&root.0, ".swift").unwrap();
        assert_eq!(files[0].components, ["Sources".to_string(), "Platform".to_string()]);
    }

    #[test]
    fn a_missing_tree_is_empty() {
        let root = Scratch::new("walker-missing");
        assert!(walk(&root.0.join("absent"), ".rs").unwrap().is_empty());
    }

    #[test]
    fn a_byte_order_mark_is_not_text() {
        let root = Scratch::new("bom");
        root.write("a.swift", "\u{FEFF}// swift-format-ignore-file\n");
        assert_eq!(read_text(&root.0.join("a.swift")).unwrap(), "// swift-format-ignore-file\n");
        assert_eq!(without_bom("plain"), "plain");
    }

    #[test]
    fn the_extension_is_matched_exactly_because_hygiene_refuses_every_other_spelling() {
        let root = Scratch::new("walker-case");
        root.write("src/lib.rs", "// x");
        root.write("src/notes.rst", "x");
        root.write("src/inner.RS", "// x");
        assert_eq!(relative_names(&walk(&root.0, ".rs").unwrap()), ["src/lib.rs"]);
        assert!(!crate::hygiene::spelling_problems("src/inner.RS").is_empty(), "the spelling the walk skips is refused");
    }

    /// A file git keeps out of `git add .` is in no rule's file list, at any depth, whichever file made
    /// it ignored: a root pattern, a nested `.gitignore` honouring everything below it, and the
    /// checkout's own `info/exclude` each drop their file. A tracked file a pattern still matches is
    /// read — git would not un-add it — and one added past the ignore rules is read again, because git
    /// would add it now.
    #[test]
    fn a_walk_reads_only_what_git_would_add_from_every_ignore_source() {
        let root = Scratch::new("walker-git");
        crate::git::output(&root.0, &["init", "-q"]).unwrap();
        root.write(".gitignore", "/by-root/deep/\ntracked.rs\n");
        root.write("tracked.rs", "// a pattern matches it, git tracks it regardless\n");
        crate::git::output(&root.0, &["add", "-f", "tracked.rs"]).unwrap();
        root.write("by-root/deep/deeper/hidden.rs", "// x");
        root.write("by-root/kept.rs", "// x");
        root.write("by-nested/deep/.gitignore", "*\n");
        root.write("by-nested/deep/hidden.rs", "// x");
        root.write(".git/info/exclude", "/by-exclude/deep/hidden.rs\n");
        root.write("by-exclude/deep/hidden.rs", "// x");
        assert_eq!(relative_names(&walk(&root.0, ".rs").unwrap()), ["by-root/kept.rs", "tracked.rs"]);
        // A file forced past the ignore rules is one git would add, so every rule reads it again.
        crate::git::output(&root.0, &["add", "-f", "by-root/deep/deeper/hidden.rs"]).unwrap();
        assert_eq!(relative_names(&walk(&root.0, ".rs").unwrap()), ["by-root/deep/deeper/hidden.rs", "by-root/kept.rs", "tracked.rs"]);
    }

    /// The answer holds for a rule walking one tree of a checkout rather than its root: git is asked at
    /// the walk's own root, and git's ignore files answer alike from any folder it is asked in.
    #[test]
    fn a_walk_of_one_tree_from_a_checkout_reads_what_git_would_add_to_it() {
        let checkout = Scratch::new("walker-git-tree");
        crate::git::output(&checkout.0, &["init", "-q"]).unwrap();
        checkout.write(".gitignore", "/ios/cache/\n");
        checkout.write("ios/sdk/App.swift", "// x");
        checkout.write("ios/cache/Scratch.swift", "// x");
        checkout.write("ios/sdk/.gitignore", "Ignored.swift\n");
        checkout.write("ios/sdk/Ignored.swift", "// x");
        assert_eq!(relative_names(&walk(&checkout.0.join("ios"), ".swift").unwrap()), ["sdk/App.swift"]);
    }

    /// A git that cannot list from inside a worktree — here one whose index is no index — fails every
    /// walk as an error rather than letting the tree read as all tracked: a rule that would read what
    /// git ignores has no clean answer to report, so the check says so in git's words.
    #[test]
    fn a_walk_in_a_checkout_git_cannot_answer_refuses() {
        let root = Scratch::new("walker-broken-git");
        crate::git::output(&root.0, &["init", "-q"]).unwrap();
        root.write("kept.rs", "// x");
        root.write(".git/index", "no index here");
        let failure = walk(&root.0, ".rs").unwrap_err();
        assert_eq!(failure, crate::git::said(crate::git::command(&root.0), crate::git::TRACKED_OR_ADDABLE), "the walk fails in git's own words");
    }

    /// The child half of the test below: run only when the parent starts it with a git first on `PATH`
    /// that writes down every start before it runs the real one.
    #[test]
    #[ignore = "run by a_walk_asks_git_one_question_however_many_files_and_folders_it_finds, with a counting git on PATH"]
    fn child_walks_a_checkout_through_a_counting_git() {
        let (Some(checkout), Some(log)) = (std::env::var_os("XTASK_SCAN_CHECKOUT"), std::env::var_os("XTASK_SCAN_GIT_LOG")) else { return };
        // Git's repository variables are asked once for the whole process, before its first command; what
        // is counted is what the walk itself asks.
        crate::git::local_env_vars();
        let before = fs::read_to_string(&log).unwrap_or_default().lines().count();
        let files = walk(Path::new(&checkout), ".rs").unwrap();
        assert_eq!(relative_names(&files), ["a.rs", "deep/b.rs", "deep/deeper/c.rs", "other/e.rs"]);
        let started = fs::read_to_string(&log).unwrap();
        let asked: Vec<&str> = started.lines().skip(before).collect();
        assert_eq!(asked.len(), 1, "one walk is one question to git: {asked:?}");
        assert!(asked.iter().all(|line| line.ends_with(&crate::git::TRACKED_OR_ADDABLE.join(" "))), "{asked:?}");
    }

    /// Each walk asks git once, whatever it finds: never once per file, folder or listing, so a tree of
    /// any size costs every rule one listing (AGENTS.md §10).
    #[cfg(unix)]
    #[test]
    fn a_walk_asks_git_one_question_however_many_files_and_folders_it_finds() {
        use std::os::unix::fs::PermissionsExt;
        let checkout = Scratch::new("walker-counted");
        crate::git::output(&checkout.0, &["init", "-q"]).unwrap();
        checkout.write(".gitignore", "/ignored/\n");
        for file in ["a.rs", "deep/b.rs", "deep/deeper/c.rs", "ignored/d.rs", "other/e.rs"] {
            checkout.write(file, "// x\n");
        }
        let real = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|folder| folder.join("git")).find(|git| git.is_file()).unwrap();
        let counting = Scratch::new("walker-counting-git");
        let log = counting.0.join("started.log");
        counting.write("bin/git", format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec '{}' \"$@\"\n", log.display(), real.display()));
        fs::set_permissions(counting.0.join("bin/git"), fs::Permissions::from_mode(0o755)).unwrap();
        craftbridge_process::rerun(
            "scan::tests::child_walks_a_checkout_through_a_counting_git",
            &[("PATH", counting.0.join("bin").as_os_str()), ("XTASK_SCAN_CHECKOUT", checkout.0.as_os_str()), ("XTASK_SCAN_GIT_LOG", log.as_os_str())],
            "a walk asked git more than once",
        )
        .unwrap();
    }

    /// What the walk keeps is what git itself would add, shape by shape, held to git's own answer: what
    /// `git add --all` puts into an index of the test's own, beside what git already tracks. A tracked file
    /// under an ignored folder or matching a pattern, a `!` that takes a file back and one that cannot
    /// (its folder is ignored), a folder's own `.gitignore` taking back a root pattern, the user's
    /// `core.excludesFile`, the checkout's `info/exclude`, a folder that ignores itself, names holding a
    /// space, glob characters and an accent composed or decomposed, and symbolic links, of which git adds
    /// the link and never what a linked folder holds.
    #[cfg(unix)]
    #[test]
    fn a_walk_keeps_exactly_what_git_itself_would_add_whatever_the_shape() {
        let checkout = Scratch::new("walker-shapes");
        let git = |args: &[&str]| crate::git::output(&checkout.0, args).unwrap();
        git(&["init", "-q"]);
        let excludes = Scratch::new("walker-shapes-excludes");
        excludes.write("ignore", "*.bak\n");
        git(&["config", "core.excludesFile", &excludes.0.join("ignore").to_string_lossy()]);
        checkout.write(".gitignore", "/ign/\ntracked.rs\n*.log\n!keep.log\n/dir/\n!/dir/keep.rs\n/dir2/*\n!/dir2/keep.rs\n*.tmp\n");
        checkout.write(".git/info/exclude", "/infoex/\n");
        checkout.write("sub/.gitignore", "!*.tmp\n");
        checkout.write("cache/.gitignore", "*\n");
        for file in [
            "ign/tracked.rs",
            "ign/other.rs",
            "tracked.rs",
            "a.log",
            "keep.log",
            "dir/keep.rs",
            "dir2/keep.rs",
            "dir2/drop.rs",
            "root.tmp",
            "sub/kept.tmp",
            "real/a.rs",
            "cache/x.rs",
            "excluded/x.bak",
            "infoex/x.rs",
            "with space.rs",
            "caf\u{e9}.rs",
            "cafe\u{301}-decomposed.rs",
            "star*.rs",
            "br[a].rs",
        ] {
            checkout.write(file, "// x\n");
        }
        std::os::unix::fs::symlink("real", checkout.0.join("link")).unwrap();
        std::os::unix::fs::symlink("real/a.rs", checkout.0.join("filelink.rs")).unwrap();
        git(&["add", "-f", "ign/tracked.rs", "tracked.rs"]);
        // Git's own answer, written into an index of the test's own so the checkout's stays as it was.
        let index = Scratch::new("walker-shapes-index");
        std::fs::copy(checkout.0.join(".git/index"), index.0.join("index")).unwrap();
        let with_index = |args: &[&str]| {
            let mut git = crate::git::command(&checkout.0);
            git.env("GIT_INDEX_FILE", index.0.join("index"));
            crate::git::run(git, args).unwrap()
        };
        with_index(&["add", "--all"]);
        let added = String::from_utf8(with_index(&["ls-files", "-z"])).unwrap();
        let mut expected: Vec<&str> = added.split('\0').filter(|path| !path.is_empty() && !checkout.0.join(path).is_dir()).collect();
        expected.sort_unstable();
        assert!(expected.contains(&"ign/tracked.rs") && expected.contains(&"keep.log") && !expected.contains(&"ign/other.rs"), "{expected:?}");
        assert_eq!(relative_names(&walk(&checkout.0, "").unwrap()), expected);
    }
}
