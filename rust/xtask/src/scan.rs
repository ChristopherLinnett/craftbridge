//! The shared machinery every check that reads source files uses: one file walker and one text reader.
//!
//! Checks must agree about what they are looking at. Two walkers would mean one check seeing a file
//! another skipped, so the exclusions live here once, and each check only says which extension it wants.
//! What a language's source means is never decided here: Rust is read through `syn`
//! (`crate::rust_syntax`), Swift through the compiler's language server and Kotlin through detekt.

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
    Ok(files)
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
}
