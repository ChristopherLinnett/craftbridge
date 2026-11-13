//! The shared machinery every check that reads source files uses: one file walker, and one stripper
//! for Rust, Swift and Kotlin, which share their comment syntax and differ only in their literals.
//!
//! Checks must agree about what they are looking at. Two walkers would mean one check seeing a file
//! another skipped, and a check acting on text that still contains a comment would report a violation
//! nobody wrote — or miss a real one hidden behind a comment. So the exclusions and the comment and
//! string removal live here once, and each check only says which extension it wants.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Build output and tool caches, each skipped only where its tool writes it: beside the file that tool
/// reads. Matching the name alone would also skip source that happens to share it — a Rust module
/// `target/`, a Swift folder `Build/` — and every rule would silently stop reading that code.
/// `tools/env.sh` names the same folders for the shell stages as paths from the repository root; keep
/// the two in step.
const BUILD_OUTPUTS: &[(&str, &[&str])] = &[
    ("target", &["Cargo.toml"]),
    (".build", &["Package.swift"]),
    (".swiftpm", &["Package.swift"]),
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
    NEVER_SOURCE.contains(&name)
        || BUILD_OUTPUTS
            .iter()
            .any(|(output, beside)| *output == name && dir.parent().is_some_and(|parent| beside.iter().any(|file| parent.join(file).is_file())))
}

/// Walk a tree depth first, yielding files whose name ends with `extension` — every file when it is
/// empty — and skipping build output. A missing root is an empty tree; a directory that exists but
/// cannot be read is an error, because a rule that cannot see a file must not report it clean.
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

/// Read a file as UTF-8 text.
pub(crate) fn read_text(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}

/// Has a file's first 8 KB a NUL byte? That is what text tools call binary, and finding it does not
/// need the whole file decoded.
pub(crate) fn looks_binary(path: &Path) -> bool {
    match fs::File::open(path) {
        Ok(mut file) => {
            let mut buffer = vec![0u8; 8192];
            match file.read(&mut buffer) {
                Ok(read) => buffer.get(..read).is_some_and(|head| head.contains(&0)),
                Err(_) => true,
            }
        }
        Err(_) => true,
    }
}

/// May `c` sit inside an identifier? ASCII alphanumerics and underscore only, which is all that
/// keywords, banned calls and lint names are built from; anything else ends one.
fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Blank the newline-free range `[from, to)` by appending that many spaces to `line`, so byte offsets in
/// the stripped copy line up with the original and a report names the right column. The range never
/// spans a newline: a line comment stops at one, and anything that may span lines goes through
/// `blanked` and `append_blank`.
fn blank(line: &mut String, from: usize, to: usize) {
    for _ in from..to {
        line.push(' ');
    }
}

/// How many bytes the character at `index` occupies, so copying never splits a multi-byte character.
fn character_advance(source: &str, index: usize) -> usize {
    source.get(index..).and_then(|rest| rest.chars().next()).map(char::len_utf8).unwrap_or(1)
}

/// Append a blanked range that may span lines: every part before a `\n` joins the pending line, each
/// `\n` ends a line, and the remainder stays pending so later lines keep their numbers.
fn append_blank(out: &mut Vec<String>, pending: &mut String, blanked: &str) {
    let mut parts = blanked.split('\n').peekable();
    while let Some(part) = parts.next() {
        pending.push_str(part);
        if parts.peek().is_some() {
            out.push(std::mem::take(pending));
        }
    }
}

/// The literals one language adds to the comment syntax Rust, Swift and Kotlin share: `//` to the end
/// of the line, and `/* */` blocks that nest in all three (`swiftc -parse` accepts
/// `/* a /* b */ c */`, and an inner `*/` alone leaves the outer comment unterminated).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Literals {
    /// `"…"` (which may span lines), `b"…"`, `c"…"`, the raw forms `r#"…"#`, `br"…"` and `cr"…"`, and
    /// char literals, told apart from lifetimes.
    Rust,
    /// `"…"` and `"""…"""`, which Swift and Kotlin share, Swift's `#"…"#` delimiters, and Kotlin's
    /// char literals (Swift has no `'` literal at all).
    SwiftKotlin,
}

/// What a stripper does with a string literal: blank it, for a check that reads only code, or keep it,
/// for a check that reads a literal's content (a SwiftPM URL) but nothing a comment hides.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Strings {
    Blank,
    Keep,
}

/// Source with every comment replaced by spaces, and string literals blanked or kept, one output line
/// per input line, so a match still reports the line the author wrote. The one stripper every check
/// uses: only the literal syntax differs between languages.
fn strip(source: &str, literals: Literals, strings: Strings) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut i = 0usize;
    while let Some(&byte) = bytes.get(i) {
        let next = bytes.get(i + 1).copied();
        if byte == b'\n' {
            out.push(std::mem::take(&mut line));
            i += 1;
        } else if byte == b'/' && next == Some(b'/') {
            let end = bytes[i..].iter().position(|b| *b == b'\n').map(|p| i + p).unwrap_or(bytes.len());
            blank(&mut line, i, end);
            i = end;
        } else if byte == b'/' && next == Some(b'*') {
            let end = block_comment_end(bytes, i);
            append_blank(&mut out, &mut line, &blanked(source, i, end));
            i = end;
        } else if let Some(end) = string_literal_end(bytes, i, literals) {
            let text = match strings {
                Strings::Blank => blanked(source, i, end),
                Strings::Keep => source.get(i..end).unwrap_or_default().to_string(),
            };
            append_blank(&mut out, &mut line, &text);
            i = end;
        } else if byte == b'\'' {
            match char_literal_end(source, i) {
                Some(end) => {
                    blank(&mut line, i, end);
                    i = end;
                }
                None => {
                    // A Rust lifetime or label: copy the quote, the name goes through as code.
                    line.push('\'');
                    i += 1;
                }
            }
        } else {
            let advance = character_advance(source, i);
            line.push_str(source.get(i..i + advance).unwrap_or(" "));
            i += advance;
        }
    }
    out.push(line);
    out
}

/// Where a string literal starting at `index` ends, or `None` when no string starts there.
fn string_literal_end(bytes: &[u8], index: usize, literals: Literals) -> Option<usize> {
    match literals {
        Literals::Rust => rust_string_end(bytes, index),
        Literals::SwiftKotlin => swift_kotlin_string_end(bytes, index),
    }
}

/// A Rust string literal at `index`: an optional `b` or `c`, an optional `r` and its hashes, then `"`.
/// The prefix letters only count at the start of a token, so `bar"` is never mistaken for a byte string.
fn rust_string_end(bytes: &[u8], index: usize) -> Option<usize> {
    let mut i = index;
    if matches!(bytes.get(i), Some(b'b' | b'c')) {
        i += 1;
    }
    let raw = bytes.get(i) == Some(&b'r');
    if raw {
        i += 1;
    }
    let mut hashes = 0usize;
    while raw && bytes.get(i) == Some(&b'#') {
        hashes += 1;
        i += 1;
    }
    if bytes.get(i) != Some(&b'"') || (i > index && index > 0 && bytes.get(index - 1).copied().is_some_and(is_ident_byte)) {
        return None;
    }
    Some(if raw { raw_string_end(bytes, i + 1, 1, hashes) } else { escaped_string_end(bytes, i, false) })
}

/// A Swift or Kotlin string literal at `index`: Swift's `#` delimiters, then `"""` or `"`. A delimited
/// string and a `"""` block end only at their own closing quotes, so their contents never end them
/// early; a plain `"…"` cannot span lines in either language, so a newline ends it and a quote the
/// stripper misreads cannot carry its error onto the next line.
fn swift_kotlin_string_end(bytes: &[u8], index: usize) -> Option<usize> {
    let mut i = index;
    while bytes.get(i) == Some(&b'#') {
        i += 1;
    }
    let hashes = i - index;
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    if bytes[i..].starts_with(b"\"\"\"") {
        return Some(raw_string_end(bytes, i + 3, 3, hashes));
    }
    Some(if hashes > 0 { raw_string_end(bytes, i + 1, 1, hashes) } else { escaped_string_end(bytes, i, true) })
}

/// Spaces for the characters of the literal spanning `[from, to)`, newlines kept in place so the
/// caller can break the stripped text into lines.
fn blanked(source: &str, from: usize, to: usize) -> String {
    source.get(from..to).unwrap_or_default().bytes().map(|byte| if byte == b'\n' { '\n' } else { ' ' }).collect()
}

/// Where a string with backslash escapes, opening with the quote at `quote`, ends: just past its closing
/// quote. An escaped quote never closes it. An unterminated literal runs to the end of the source (or of
/// the line, when `stops_at_newline`), which is the honest answer for a broken file.
fn escaped_string_end(bytes: &[u8], quote: usize, stops_at_newline: bool) -> usize {
    let mut i = quote + 1;
    while let Some(&byte) = bytes.get(i) {
        match byte {
            b'\\' => i += 2,
            b'"' => return i + 1,
            b'\n' if stops_at_newline => return i,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Where a string without escapes ends, its body starting at `from`: at the first run of `quotes` double
/// quotes followed by `hashes` hashes. A run that hits the end of the file is not a close, and the
/// literal runs to the file's end.
fn raw_string_end(bytes: &[u8], from: usize, quotes: usize, hashes: usize) -> usize {
    let close = |at: usize| {
        bytes.get(at..at + quotes).is_some_and(|run| run.iter().all(|c| *c == b'"'))
            && bytes.get(at + quotes..at + quotes + hashes).is_some_and(|run| run.iter().all(|c| *c == b'#'))
    };
    let mut i = from;
    while i < bytes.len() {
        if close(i) {
            let mut end = i + quotes + hashes;
            // Kotlin's `""""` closes on the last three quotes, the extra one belonging to the string.
            while hashes == 0 && quotes == 3 && bytes.get(end) == Some(&b'"') {
                end += 1;
            }
            return end;
        }
        i += 1;
    }
    bytes.len()
}

/// Where a block comment starting at `index` (`/`) ends: just past the `*/` that closes it. A `/*`
/// inside opens a deeper comment, as all three languages read it.
fn block_comment_end(bytes: &[u8], index: usize) -> usize {
    let mut i = index + 2;
    let mut depth = 1usize;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

/// Where a char literal starting at `index` ends, if one does: one character or one escape (`'x'`,
/// `'"'`, `'\n'`, `'\''`, `'\x41'`, `'\u{1F600}'`) and the closing quote. A Rust lifetime or label (`'a`,
/// `'outer:`) has no closing quote right after its first character, which is what tells them apart.
fn char_literal_end(source: &str, index: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let first = source.get(index + 1..)?.chars().next()?;
    let mut end = index + 1 + first.len_utf8();
    if first == '\\' {
        match bytes.get(end) {
            Some(b'u') => {
                let close = bytes.get(end..)?.iter().position(|c| *c == b'}')?;
                end += close + 1;
            }
            Some(b'x') => end += 3,
            Some(_) => end += 1,
            None => return None,
        }
    }
    (bytes.get(end) == Some(&b'\'')).then_some(end + 1)
}

/// Rust source with every comment, string and char literal blanked. Doc comments are comments, so a
/// lint or generated marker quoted in one cannot fool a check; raw and byte strings are blanked whole,
/// because an `unsafe` inside one is data; and `'a` is left alone, because a lifetime is not a char
/// literal.
pub(crate) fn strip_rust(source: &str) -> Vec<String> {
    strip(source, Literals::Rust, Strings::Blank)
}

/// Swift or Kotlin source with comments and string literals blanked. A `"""…"""` literal holds example
/// code, which is exactly the text that would otherwise be reported, so it is blanked whole.
pub(crate) fn strip_clike(source: &str) -> Vec<String> {
    strip(source, Literals::SwiftKotlin, Strings::Blank)
}

/// Swift or Kotlin source with comments blanked and string literals kept: for a scanner that must read
/// a literal's content, a SwiftPM URL say, while still ignoring everything a comment hides.
pub(crate) fn strip_comments(source: &str) -> String {
    strip(source, Literals::SwiftKotlin, Strings::Keep).join("\n")
}

/// Count occurrences of `needle` that are whole words: `left` forbids an identifier byte before the
/// match, `right` forbids one after it. That is what stops `exit(` from reporting `prohibit_exit(1)`
/// and lets `!!` match without caring about the characters around it.
pub(crate) fn count_whole_word(line: &str, needle: &str, left: bool, right: bool) -> usize {
    let bytes = line.as_bytes();
    let pattern = needle.as_bytes();
    let mut hits = 0usize;
    let mut from = 0usize;
    while let Some(offset) = find(bytes, from, pattern) {
        let before_ok = !left || offset == 0 || !ident_byte(line, offset - 1);
        let after_ok = !right || !ident_byte(line, offset + pattern.len());
        if before_ok && after_ok {
            hits += 1;
        }
        // Skip the whole match, not one byte: overlapping candidates of a repeated substring would
        // otherwise be counted twice, and a match inside the previous match was never a word boundary.
        from = offset + pattern.len();
    }
    hits
}

/// Index of the next occurrence of `pattern` in `haystack` at or after `from`.
pub(crate) fn find(haystack: &[u8], from: usize, pattern: &[u8]) -> Option<usize> {
    if pattern.is_empty() || haystack.len() < pattern.len() || from > haystack.len() {
        return None;
    }
    (from..=haystack.len() - pattern.len()).find(|&start| haystack.get(start..start + pattern.len()) == Some(pattern))
}

/// Is the byte at `index` an identifier byte? Out of range counts as "no", which is what a match at
/// the very end of a line needs.
pub(crate) fn ident_byte(line: &str, index: usize) -> bool {
    line.as_bytes().get(index).copied().is_some_and(is_ident_byte)
}

/// The 1-based line holding byte `offset` of `text`.
pub(crate) fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes().iter().take(offset).filter(|byte| **byte == b'\n').count() + 1
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
    fn rust_stripper_blanks_line_comments_and_keeps_line_numbers() {
        let stripped = strip_rust("fn f() {}\n// unsafe here\nunsafe {}\n");
        assert_eq!(stripped.len(), 4);
        assert!(!stripped[1].contains("unsafe"), "a comment must not count");
        assert!(stripped[2].contains("unsafe"));
    }

    #[test]
    fn rust_stripper_blanks_doc_comments_and_nested_block_comments() {
        let source = "//! unsafe in a doc comment\n/* a /* nested */ unsafe */ still\nlet a = 1; // unsafe\n";
        let stripped = strip_rust(source);
        assert_eq!(stripped.len(), 4);
        assert!(!stripped[0].contains("unsafe"));
        assert!(!stripped[1].contains("nested"));
        assert!(stripped[1].contains("still"), "code after a nested block comment survives: {}", stripped[1]);
        assert!(!stripped[2].contains("unsafe"));
    }

    #[test]
    fn rust_stripper_blanks_strings_but_keeps_lifetimes() {
        let stripped = strip_rust("let s = \"unsafe \\\" still\";\nfn g<'a>(x: &'a str) -> &'a str { x }\n");
        assert!(!stripped[0].contains("unsafe"), "an escaped quote must not end the string early");
        assert!(stripped[0].contains("let s ="), "code before the literal survives");
        assert!(stripped[1].contains("'a"), "a lifetime is not a char literal: {}", stripped[1]);
    }

    #[test]
    fn a_rust_string_spans_lines_until_its_closing_quote() {
        // Rust strings may hold a newline: the second line is still inside the literal, and the code
        // after the closing quote is code.
        let stripped = strip_rust("let s = \"first\nthe unsafe word is prose\".len() + unsafe { 0 };\n");
        assert!(!stripped[1].contains("prose"), "{}", stripped[1]);
        assert_eq!(count_whole_word(&stripped[1], "unsafe", true, true), 1, "{}", stripped[1]);
    }

    #[test]
    fn rust_stripper_blanks_raw_byte_and_c_strings() {
        let source =
            "let r = r\"unsafe\";\nlet h = r#\"unsafe \"x\" unsafe\"#;\nlet b = b\"un\\\"safe\";\nlet c = c\"unsafe\";\nlet br = br#\"unsafe\"#;\nlet z = 1;\n";
        let stripped = strip_rust(source);
        assert_eq!(stripped.len(), 7);
        for line in stripped.iter().take(5) {
            assert!(!line.contains("unsafe") && !line.contains("safe"), "{line}");
        }
        assert!(stripped[5].contains("let z = 1;"));
    }

    #[test]
    fn rust_stripper_blanks_multiline_raw_strings_with_their_line_numbers() {
        let source = "let r = r#\"\nunsafe\n\"#;\nlet u = unsafe;\n";
        let stripped = strip_rust(source);
        assert_eq!(stripped.len(), 5);
        assert!(!stripped[1].contains("unsafe"));
        assert!(stripped[3].contains("unsafe"), "the line after the literal is line 4: {}", stripped[3]);
    }

    #[test]
    fn rust_stripper_handles_char_literals_and_escapes() {
        let stripped = strip_rust("let c = '\\n';\nlet q = '\\'';\nlet u = '\\u{1F600}';\nlet d = '\"'; let after = unsafe;\nlet a = 'a';\nlet e = 'é';\n");
        assert!(stripped[0].contains(';') && !stripped[0].contains('\''), "{}", stripped[0]);
        assert!(stripped[1].contains(';') && !stripped[1].contains('\''), "{}", stripped[1]);
        assert!(stripped[3].contains("unsafe"), "a quote inside a char literal opens no string: {}", stripped[3]);
        assert!(!stripped[4].contains('\''), "a one-letter char literal is not a lifetime: {}", stripped[4]);
        assert!(!stripped[5].contains('\''), "{}", stripped[5]);
    }

    #[test]
    fn clike_stripper_blanks_comments_strings_and_raw_strings() {
        let source = "// fatalError(x)\nlet s = \"fatalError(y)\"\nlet r = \"\"\"\nfatalError(z)\n\"\"\"\nexit(1)\n";
        let stripped = strip_clike(source);
        assert_eq!(stripped.len(), 7);
        for line in stripped.iter().take(5) {
            assert!(!line.contains("fatalError"), "{line}");
        }
        assert!(stripped[5].contains("exit(1)"));
    }

    #[test]
    fn a_kotlin_char_literal_holding_a_quote_opens_no_string() {
        let stripped = strip_clike("val quote = '\"'\nfun f(s: String?) = s!!.length\nval label = \"x\"\n");
        assert!(stripped[1].contains("!!"), "{}", stripped[1]);
    }

    #[test]
    fn swift_delimited_strings_hold_quotes() {
        let stripped = strip_clike("let json = #\"{\"fatalError\": \"x\"}\"#\nlet multi = #\"\"\"\nfatalError()\n\"\"\"#\nexit(1)\n");
        assert!(!stripped[0].contains("fatalError"), "{}", stripped[0]);
        assert!(!stripped[2].contains("fatalError"), "{}", stripped[2]);
        assert!(stripped[4].contains("exit(1)"), "{}", stripped[4]);
    }

    #[test]
    fn clike_stripper_nests_block_comments() {
        // Swift and Kotlin block comments nest, so text after an inner `*/` is still commented out
        // until the outer one closes.
        let stripped = strip_clike("/* a /* inner */ fatalError() */ live\nexit(1)\n");
        assert!(!stripped[0].contains("fatalError"), "still inside the outer comment: {}", stripped[0]);
        assert!(stripped[0].contains("live"), "code after the outer `*/` is live: {}", stripped[0]);
        assert!(stripped[1].contains("exit"));
    }

    #[test]
    fn stripped_lines_keep_byte_offsets() {
        let source = "let s = \"é\"; // ü\nlet t = 1;\n";
        let stripped = strip_rust(source);
        for (original, kept) in source.split('\n').zip(&stripped) {
            assert_eq!(original.len(), kept.len(), "{kept:?}");
        }
    }

    #[test]
    fn whole_word_counts_ignore_longer_identifiers() {
        assert_eq!(count_whole_word("exit(1)", "exit", true, false), 1);
        assert_eq!(count_whole_word("prohibit_exit(1)", "exit", true, false), 0);
        assert_eq!(count_whole_word("x!!.y!!z", "!!", false, false), 2);
        assert_eq!(count_whole_word("external fun", "external", true, true), 1);
        assert_eq!(count_whole_word("notexternal fun", "external", true, true), 0);
    }

    #[test]
    fn binary_detection_reads_the_head_only() {
        let root = Scratch::new("binary");
        root.write("blob.rs", [b'a', 0, b'b']);
        root.write("text.rs", "// fine\n");
        assert!(looks_binary(&root.0.join("blob.rs")));
        assert!(!looks_binary(&root.0.join("text.rs")));
    }

    #[test]
    fn a_raw_string_whose_close_runs_off_the_end_does_not_crash() {
        // The blank runs to the end of the file: a truncated hash run is never a close.
        assert_eq!(strip_rust("r#\"\""), vec!["    "]);
        assert_eq!(strip_rust("r##\"\"#"), vec!["      "]);
        assert_eq!(strip_rust("r#\"x\"#"), vec!["      "]);
        assert_eq!(strip_rust("r##\"a \"# b\"##"), vec!["             "]);
    }

    #[test]
    fn the_strippers_never_crash_on_any_input() {
        // A byte that starts a string whose end never comes must blank, not panic: a rule checker
        // that crashes on half-written source is worse than one that misses a match.
        let alphabet: &[char] = &['"', '\'', '/', '*', '\\', 'r', 'b', 'c', '#', '\n', 'a', 'u', 'x', '{', '}', ' ', 'é'];
        let mut state: u64 = 20261112;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };
        for _ in 0..40000 {
            let length = 1 + next() % 24;
            let text: String = (0..length).map(|_| alphabet[next() % alphabet.len()]).collect();
            for strip_one in [strip_rust, strip_clike] {
                let stripped = std::panic::catch_unwind(|| strip_one(&text));
                assert!(stripped.is_ok(), "a stripper panicked on {text:?}");
            }
            assert!(std::panic::catch_unwind(|| strip_comments(&text)).is_ok(), "strip_comments panicked on {text:?}");
        }
    }
}
