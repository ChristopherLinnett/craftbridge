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
//! A header exists only where the format allows a comment, so every format is read for one except those
//! that cannot hold a comment at all — JSON, which the rule itself names, lock files and plain text —
//! and binary files, which are not source. A format nobody listed is read rather than skipped, because
//! a list of what can hold a comment would quietly skip the next format to arrive. Markdown counts only
//! its HTML comment, `<!--`, so a `# heading` that names the marker is not taken for a header. Two kinds of
//! written-for-you file sit outside this rule, as §6 says: what a build tool writes for itself
//! (`Cargo.lock`, `Package.resolved`, the Gradle wrapper), which the tool owns, so their files are not
//! read here, bar the wrapper's properties file, which carries no marker; and the plan's views in
//! `plan/`, which are not in the trees below at all — the plan kit builds them from their sources and
//! its own `plan check --final` owns them. What `tools/update-upstream.sh` writes into `rust/Cargo.toml`
//! is a table in a hand-written file, which this rule does not read either.

use crate::scan::{self, FoundFile};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// The marker every generated file carries in its header.
pub(crate) const MARKER: &str = "@generated";

/// How many lines a header is allowed to hide in. Cargo's own banner is on line 1; UniFFI writes no
/// marker at all, so whatever registers UniFFI's output prepends one. Five is a policy, room for a shebang
/// and a licence line without letting a marker sink out of sight, not a fit to any tool's output.
const HEADER_LINES: usize = 5;

/// The comment openers a header can start with: `#` (shell, TOML, YAML, ProGuard, properties), `//`
/// (Rust, Swift, Kotlin, C, Objective-C, JS, module maps), `/*` (the same languages' block comments,
/// CSS) and `<!--` (XML, SVG, HTML, Markdown). The marker counts only inside a comment that opens a
/// line, or inside a block comment opened that way, which is what a *header* means.
const COMMENT_OPENERS: &[&str] = &["#", "//", "/*", "<!--"];

/// The block comments a header may spread over lines, with what closes each.
const BLOCK_COMMENTS: &[(&str, &str)] = &[("/*", "*/"), ("<!--", "-->")];

/// Formats that hold no comment, so neither need nor can carry a header: a marker quoted in one (a
/// Cargo.lock line, a JSON string) is data, not this repository claiming a file.
const NO_COMMENT_EXTENSIONS: &[&str] = &[".json", ".lock", ".resolved", ".txt", ".jar"];

/// Formats whose only comment is the HTML one.
const HTML_COMMENT_ONLY: &[&str] = &[".md", ".markdown"];

/// The files a build tool writes for itself, which §6 leaves to that tool.
const BUILD_TOOL_FILES: &[&str] = &["Cargo.lock", "Package.resolved", "gradlew", "gradlew.bat"];

/// The trees that hold code or data a generator could write.
pub(crate) const TREES: &[&str] = &["rust", "ios", "android", "docs", "conformance", "tools", ".github"];

/// Can this file hold a header comment at all? Every format can but those in [`NO_COMMENT_EXTENSIONS`]
/// and the build tools' own files.
fn can_carry_header(path: &Path) -> bool {
    let stored = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let name = stored.to_lowercase();
    !BUILD_TOOL_FILES.contains(&stored.as_str()) && !NO_COMMENT_EXTENSIONS.iter().any(|extension| name.ends_with(extension))
}

/// The comment openers a file's format has.
fn openers_for(path: &str) -> &'static [&'static str] {
    let name = path.to_lowercase();
    if HTML_COMMENT_ONLY.iter().any(|extension| name.ends_with(extension)) { &["<!--"] } else { COMMENT_OPENERS }
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

/// Does the head carry the marker, in a comment, within the first lines? A line counts when it opens
/// with one of the format's comment openers, or when it lies inside a block comment such a line opened
/// (` * @generated` in a docblock).
fn has_header(head: &Head) -> bool {
    let openers = openers_for(&head.path);
    let mut open_block: Option<&str> = None;
    for line in head.lines.iter().take(HEADER_LINES) {
        let text = scan::without_bom(line).trim_start();
        let commented = match open_block {
            Some(closer) => {
                if text.contains(closer) {
                    open_block = None;
                }
                true
            }
            None => match openers.iter().find(|opener| text.starts_with(**opener)) {
                Some(opener) => {
                    let closer = BLOCK_COMMENTS.iter().find(|(block, _)| block == opener).map(|(_, closer)| *closer);
                    open_block = closer.filter(|closer| !text.get(opener.len()..).is_some_and(|rest| rest.contains(closer)));
                    true
                }
                None => false,
            },
        };
        if commented && text.contains(MARKER) {
            return true;
        }
    }
    false
}

/// Read the first `HEADER_LINES` lines without reading the rest: a marker is always near the top, and a
/// large committed file is nobody's business here. A line that is not UTF-8 ends the head, and a leading
/// byte-order mark is not part of the first line.
fn head_lines(path: &Path) -> Vec<String> {
    match std::fs::File::open(path) {
        Ok(file) => BufReader::new(file).lines().take(HEADER_LINES).map_while(Result::ok).map(|line| scan::without_bom(&line).to_string()).collect(),
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
    fn every_format_that_holds_a_comment_is_read_and_only_those_that_cannot_are_skipped() {
        for name in ["commands.md", "Bridge.mm", "Plant.xcconfig", "Info.plist", "tool.mjs", "plant.rb", "Package.swift", "pre-push"] {
            assert!(can_carry_header(Path::new(name)), "{name} holds a comment");
        }
        for name in ["corpus.json", "Cargo.lock", "Package.resolved", "notes.txt", "gradlew", "gradlew.bat", "gradle-wrapper.jar"] {
            assert!(!can_carry_header(Path::new(name)), "{name} holds none, or is its tool's");
        }
        // A generated Markdown file needs the marker in its HTML comment, and a stray one is caught.
        let unmarked = head("docs/generated/commands.md", &["# Commands", ""], true);
        assert!(check_head(&unmarked).iter().any(|e| e.contains("must carry")), "{unmarked:?}");
        let stray = head("conformance/effects/README.md", &[&format!("<!-- {MARKER} by a tool -->")], false);
        assert_eq!(check_head(&stray), ["conformance/effects/README.md: generated file outside a generated directory"]);
        // A heading that names the marker is prose, not a header.
        assert!(check_head(&head("docs/rules.md", &[&format!("# Why {MARKER} matters")], false)).is_empty());
    }

    #[test]
    fn a_marker_inside_a_block_comment_or_after_a_byte_order_mark_counts() {
        let docblock = head("rust/core/x/src/plant.rs", &["/*", &format!(" * {MARKER} by a tool"), " */"], false);
        assert!(!check_head(&docblock).is_empty(), "a docblock marker outside a generated directory");
        let generated = head("ios/sdk/Sources/Generated/Api.swift", &["/*", &format!(" * {MARKER} by a tool"), " */"], true);
        assert!(check_head(&generated).is_empty(), "the same docblock is a header inside one");
        let html = head("docs/generated/a.md", &["<!--", &format!("  {MARKER}"), "-->"], true);
        assert!(check_head(&html).is_empty());
        let closed = head("x.rs", &["/* a comment */", &format!("let marker = \"{MARKER}\";")], false);
        assert!(check_head(&closed).is_empty(), "past a closed block comment the marker is code again");
        let bom = head("rust/core/x/src/plant_bom.rs", &[&format!("{}// {MARKER}", '\u{FEFF}')], false);
        assert!(!check_head(&bom).is_empty(), "a byte-order mark is not the line's first character");
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
