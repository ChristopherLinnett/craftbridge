//! Rule 6 — generated code lives where it is recognisable, says what it is, and has a generator.
//!
//! A generated file is never edited by hand: to change one you change its generator. That only holds if
//! a human can tell generated from written at a glance, if a stray hand edit is caught, and if the gate
//! reruns the generator of every generated file. So a file in a `generated`/`Generated` directory must
//! carry the marker its generator writes in a header comment, a file outside such a directory that
//! carries it is a generated file in the wrong place, and every file in a generated directory must lie
//! in the output of a generator `cargo xtask generate` registers. Without that last rule a generated
//! directory nothing owns would hold code every other rule skips and nothing compares.
//!
//! JSON is the exception the rule itself makes: it has no comment syntax, so it cannot carry a header.
//! Binary files are skipped; they are not source. Two kinds of written-for-you file sit outside this
//! rule, as §6 says: what a build tool writes for itself (`Cargo.lock`, `Package.resolved`, the Gradle
//! wrapper), which the tool owns: their formats are not read here, bar the wrapper's properties file,
//! which carries no marker; and the plan's views in `plan/`, which are not in the trees below at all —
//! the plan kit builds them from their sources and its own `plan check --final` owns them.

use crate::scan::{self, FoundFile};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// The marker every generated file carries in its header.
pub(crate) const MARKER: &str = "@generated";

/// How many lines a header is allowed to hide in: UniFFI's and Cargo's own markers are first or second,
/// and five is room enough for a shebang and a licence line without letting a marker sink out of sight.
const HEADER_LINES: usize = 5;

/// The comment openers a header can start with, across the formats in [`HEADER_EXTENSIONS`]: `#` (shell,
/// TOML, YAML, ProGuard, properties), `//` (Rust, Swift, Kotlin, C, Objective-C, JS, module maps), `/*`
/// (the same languages' block comments, CSS) and `<!--` (XML, SVG, HTML). The marker counts only inside
/// a line that opens with one, which is what a *header* means.
const COMMENT_OPENERS: &[&str] = &["#", "//", "/*", "<!--"];

/// The extensions of the formats these trees hold that can carry a header comment. A header exists only
/// where the format allows a comment, so a file outside this list is neither required to carry one nor
/// believed when it claims to: `.md`, `.json`, `.lock`, `.png` and the rest are not generated source.
const HEADER_EXTENSIONS: &[&str] = &[
    ".rs",
    ".swift",
    ".kt",
    ".kts",
    ".java",
    ".h",
    ".c",
    ".m",
    ".modulemap",
    ".toml",
    ".ts",
    ".js",
    ".py",
    ".sh",
    ".bash",
    ".zsh",
    ".yaml",
    ".yml",
    ".xml",
    ".svg",
    ".html",
    ".css",
    ".scss",
    ".pro",
    ".properties",
    ".gradle",
];

/// The trees that hold code or data a generator could write.
const TREES: &[&str] = &["rust", "ios", "android", "docs", "conformance", "tools", ".github"];

/// Can this file name hold a header comment at all? `.json` is named in the rule precisely because it
/// cannot; everything outside [`HEADER_EXTENSIONS`] cannot for the same reason.
fn can_carry_header(path: &Path) -> bool {
    let name = path.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
    HEADER_EXTENSIONS.iter().any(|extension| name.ends_with(extension))
}

/// A file's head, split into lines, with the marker question already answered.
#[derive(Debug)]
pub(crate) struct Head {
    /// How a message names the file.
    pub(crate) path: String,
    /// The first lines of the file, as far as they could be read.
    pub(crate) lines: Vec<String>,
    /// Whether the file sits inside a `generated`/`Generated` directory.
    pub(crate) in_generated_dir: bool,
    /// Whether the format can hold a comment at all.
    pub(crate) can_carry_header: bool,
}

/// Check one head against where the file sits.
pub(crate) fn check_head(head: &Head) -> Vec<String> {
    // A file whose format cannot hold a header comment neither has to carry one nor is believed when it
    // seems to: JSON is the rule's named exception, and a Cargo.lock line quoting Cargo's own banner is
    // not this repository claiming a file as generated.
    if !head.can_carry_header {
        return Vec::new();
    }
    match (head.in_generated_dir, has_header(head)) {
        (true, false) => vec![format!("{}: a file in a generated directory must carry {MARKER} in a comment in its first {HEADER_LINES} lines", head.path)],
        (false, true) => vec![format!("{}: generated file outside a generated directory", head.path)],
        _ => Vec::new(),
    }
}

/// Does the head carry the marker, in a comment, within the first lines?
fn has_header(head: &Head) -> bool {
    head.lines.iter().take(HEADER_LINES).any(|line| {
        let text = line.trim_start();
        COMMENT_OPENERS.iter().any(|opener| text.starts_with(opener)) && text.contains(MARKER)
    })
}

/// Read the first `HEADER_LINES` lines without reading the rest: a marker is always near the top, and a
/// large committed file is nobody's business here. A line that is not UTF-8 ends the head.
fn head_lines(path: &Path) -> Vec<String> {
    match std::fs::File::open(path) {
        Ok(file) => BufReader::new(file).lines().take(HEADER_LINES).map_while(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

/// Is any directory component a generated directory?
pub(crate) fn in_generated(components: &[String]) -> bool {
    components.iter().any(|component| component == "generated" || component == "Generated")
}

/// The outermost generated directory holding a file, relative to the root: what a message names when no
/// generator owns it.
fn outermost_generated_dir(tree: &str, file: &FoundFile) -> String {
    let depth = file.components.iter().position(|component| component == "generated" || component == "Generated").unwrap_or(0);
    std::iter::once(tree).chain(file.components.iter().take(depth + 1).map(String::as_str)).collect::<Vec<_>>().join("/")
}

/// Reduce a walked file to what this rule reads.
fn head_of(root: &Path, file: &FoundFile) -> Head {
    Head {
        path: crate::report::shown(root, &file.path),
        lines: head_lines(&file.path),
        in_generated_dir: in_generated(&file.components),
        can_carry_header: can_carry_header(&file.path),
    }
}

/// Run the check over every tree that holds code, against the generators `cargo xtask generate` runs.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let registry = crate::generate::generators();
    let outputs: Vec<&str> = registry.iter().map(|generator| generator.output_dir).collect();
    check_against(root, &outputs)
}

/// The check, with the generators' output directories (relative to the root) given.
fn check_against(root: &Path, outputs: &[&str]) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    let mut unowned = BTreeSet::new();
    for tree in TREES {
        for file in scan::walk(&root.join(tree), "")? {
            if !scan::looks_binary(&file.path) {
                errors.extend(check_head(&head_of(root, &file)));
            }
            let path = format!("{tree}/{}", file.relative);
            if in_generated(&file.components) && !outputs.iter().any(|output| path.starts_with(&format!("{}/", output.trim_end_matches('/')))) {
                unowned.insert(outermost_generated_dir(tree, &file));
            }
        }
    }
    errors.extend(unowned.into_iter().map(|dir| {
        format!("{dir}: no generator in rust/xtask/src/generate.rs writes here, so nothing regenerates these files and every other rule skips them")
    }));
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    fn head(path: &str, lines: &[&str], in_generated_dir: bool) -> Head {
        Head { path: path.to_string(), lines: lines.iter().map(|line| line.to_string()).collect(), in_generated_dir, can_carry_header: true }
    }

    #[test]
    fn a_generated_file_with_its_marker_passes() {
        let ok = head(
            "ios/sdk/Sources/Generated/Effects.swift",
            &["// Generated by craftbridge-effects-codegen. Do not edit by hand.", &format!("// {MARKER}"), ""],
            true,
        );
        assert!(check_head(&ok).is_empty());
        let deep = head("rust/core/x/src/generated/api.rs", &[&format!("//! {MARKER} by the codegen; see rust/xtask."), ""], true);
        assert!(check_head(&deep).is_empty());
    }

    #[test]
    fn a_generated_file_without_its_marker_is_reported() {
        let errors = check_head(&head("ios/sdk/Sources/Generated/Effects.swift", &["// someone deleted the header", "import Foundation"], true));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("must carry"), "{errors:?}");
        // The marker only counts inside a comment: prose or code that quotes it is not a header.
        let prose = format!("The rule says to write `{MARKER}` at the top of the file.");
        assert!(check_head(&head("docs/notes.toml", &[prose.as_str()], false)).is_empty());
        assert!(
            check_head(&head("rust/src/lib.rs", &[format!("let s = \"{MARKER}\";").as_str()], false)).is_empty(),
            "a marker inside a string literal is not a header"
        );
        // Past the fifth line the header no longer counts as a header.
        assert!(!check_head(&head("x.swift", &["a", "b", "c", "d", "e", &format!("// {MARKER}")], true)).is_empty());
    }

    #[test]
    fn json_is_exempt_because_it_cannot_hold_a_comment() {
        let json = Head {
            path: "conformance/effects/corpus.json".to_string(),
            lines: vec!["{".to_string(), "  \"name\": \"x\"".to_string()],
            in_generated_dir: true,
            can_carry_header: can_carry_header(Path::new("corpus.json")),
        };
        assert!(check_head(&json).is_empty(), "{json:?}");
        let swift = Head { can_carry_header: can_carry_header(Path::new("Api.swift")), ..json };
        assert!(!check_head(&swift).is_empty());
        assert!(
            can_carry_header(Path::new("CraftBridgeFFI.h")) && can_carry_header(Path::new("module.modulemap")),
            "UniFFI's C header and module map hold comments"
        );
    }

    #[test]
    fn a_marker_outside_a_generated_directory_is_reported() {
        // A source file (a header is expected of it) carrying the marker outside `generated/` is wrong:
        // it was generated, and generated files live in `generated/` only.
        let errors = check_head(&head(
            "rust/effects/craftbridge-effects/src/api.rs",
            &[&format!("// {MARKER} by the old script. Do not edit."), "use craftbridge_core as core;"],
            false,
        ));
        assert_eq!(errors, ["rust/effects/craftbridge-effects/src/api.rs: generated file outside a generated directory"]);
        // Cargo.lock opens with Cargo's own banner; its format is not in the list, so the banner is not
        // read as this repository claiming the file. The other build-tool files §6 names are outside the
        // rule the same way: their formats are the tool's, and `plan/` is not in the trees at all.
        let lock = Head {
            can_carry_header: can_carry_header(Path::new("Cargo.lock")),
            ..head("rust/Cargo.lock", &[&format!("# This file is automatically {MARKER} by Cargo."), "# It is not intended for manual editing."], false)
        };
        assert!(check_head(&lock).is_empty(), "{lock:?}");
        for build_tool_file in ["Package.resolved", "gradlew", "gradlew.bat", "gradle-wrapper.jar"] {
            assert!(!can_carry_header(Path::new(build_tool_file)), "{build_tool_file} belongs to its tool, not to this check");
        }
    }

    #[test]
    fn plan_views_are_outside_the_rule_because_the_plan_kit_owns_them() {
        let scratch = Scratch::new("plan-views");
        scratch.mkdir("plan");
        scratch.write("plan/GENERATED.md", format!("<!-- {MARKER} by the plan kit. -->\n"));
        assert!(check_against(&scratch.0, &[]).unwrap().is_empty(), "plan/ is not one of the trees this rule reads");
    }

    #[test]
    fn an_ordinary_file_passes() {
        assert!(
            check_head(&head("rust/core/craftbridge-core/src/lib.rs", &["//! The universal runtime.", "", "use craftbridge_media as media;"], false))
                .is_empty()
        );
    }

    #[test]
    fn the_check_reads_a_real_tree() {
        let scratch = Scratch::new("generated");
        scratch.write("rust/src/generated/api.rs", format!("// {MARKER}\n"));
        scratch.write("rust/src/lib.rs", "//! hand-written\n");
        scratch.write("rust/src/blob.rs", [0u8, 1, 2]);
        assert!(check_against(&scratch.0, &["rust/src/generated"]).unwrap().is_empty());
        // And a marker written into an ordinary file is caught by the same pass.
        scratch.write("rust/src/lib.rs", format!("// {MARKER}\n"));
        let caught = check_against(&scratch.0, &["rust/src/generated"]).unwrap();
        assert_eq!(caught, ["rust/src/lib.rs: generated file outside a generated directory"]);
    }

    #[test]
    fn a_generated_directory_no_generator_writes_is_reported_once() {
        let scratch = Scratch::new("generated-unowned");
        scratch.write("rust/core/x/src/generated/a.rs", format!("// {MARKER}\n"));
        scratch.write("rust/core/x/src/generated/deeper/b.rs", format!("// {MARKER}\n"));
        let errors = check_against(&scratch.0, &[]).unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("rust/core/x/src/generated: no generator"), "{errors:?}");
        assert!(check_against(&scratch.0, &["rust/core/x/src/generated"]).unwrap().is_empty());
        assert_eq!(check_against(&scratch.0, &["rust/core/x/src/generated/deeper"]).unwrap().len(), 1, "a.rs lies outside that output");
    }

    #[test]
    fn the_check_reads_the_repository_and_finds_nothing() {
        let errors = check(&crate::repo_root()).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
