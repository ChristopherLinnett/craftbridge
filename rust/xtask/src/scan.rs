//! The shared machinery every check that reads source files uses: one file walker, and one stripper
//! for Rust, Swift and Kotlin, which share their comment syntax and differ only in their literals.
//!
//! Checks must agree about what they are looking at. Two walkers would mean one check seeing a file
//! another skipped, and a check acting on text that still contains a comment would report a violation
//! nobody wrote — or miss a real one hidden behind a comment. So the exclusions and the comment and
//! string removal live here once, and each check only says which extension it wants.

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

/// The language whose literals a stripper reads. All three share their comments: `//` to the end of the
/// line, and `/* */` blocks that nest (`swiftc -parse` accepts `/* a /* b */ c */`, and an inner `*/`
/// alone leaves the outer comment unterminated). They differ in their literals:
/// - Rust: `"…"` (which may span lines), `b"…"`, `c"…"`, the raw forms `r#"…"#`, `br"…"` and `cr"…"`, and
///   char literals (`'\u{1F600}'`), told apart from lifetimes;
/// - Swift: `"…"` and `"""…"""`, both with `\(…)` interpolation, and `#"…"#` delimiters, whose escapes and
///   interpolation take the same number of `#` (`\#(…)`); Swift has no `'` literal at all;
/// - Kotlin: `"…"` with escapes and `"""…"""` without, both with `${…}` templates, and char literals,
///   whose Unicode escape is `\uXXXX` with no braces.
///
/// Code inside an interpolation or a template is code: `"\(value.unsafelyUnwrapped)"` reads the value
/// as surely as the same call outside a string, so it is kept and stripped like any other code, and
/// only the literal text around it is blanked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Language {
    Rust,
    Swift,
    Kotlin,
}

/// What a stripper does with a string or char literal: blank it, for a check that reads only code, or
/// keep it, for a check that reads a literal's content (a quoted path in a build file) but nothing a
/// comment hides.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Strings {
    Blank,
    Keep,
}

/// How deep interpolations may nest inside strings inside interpolations before the stripper stops
/// looking for more: real code nests once or twice, and a bound keeps hostile input from exhausting the
/// stack. Past it, a string's interpolations are read as its text.
const MAX_NESTING: usize = 16;

/// One string literal: where it ends, and the byte ranges of code inside it (interpolations and
/// templates), in order.
struct Literal {
    end: usize,
    code: Vec<(usize, usize)>,
}

/// Source with every comment replaced by spaces, and string literals blanked or kept, one output line
/// per input line, so a match still reports the line the author wrote. The one stripper every check
/// uses: only the literal syntax differs between languages.
fn strip(source: &str, language: Language, strings: Strings) -> Vec<String> {
    let mut output = Stripped { lines: Vec::new(), line: String::new(), strings };
    strip_range(source, 0, source.len(), language, 0, &mut output);
    output.lines.push(output.line);
    output.lines
}

/// The stripped text so far: the finished lines and the one still being written.
struct Stripped {
    lines: Vec<String>,
    line: String,
    strings: Strings,
}

impl Stripped {
    /// Append a literal's text `[from, to)` of `source`, blanked or kept.
    fn literal(&mut self, source: &str, from: usize, to: usize) {
        if from >= to {
            return;
        }
        match self.strings {
            Strings::Blank => append_blank(&mut self.lines, &mut self.line, &blanked(source, from, to)),
            Strings::Keep => append_blank(&mut self.lines, &mut self.line, source.get(from..to).unwrap_or_default()),
        }
    }
}

/// Strip `[from, to)` of `source` onto `output`: the whole file, or the code inside one interpolation,
/// which is stripped exactly as code outside a string is.
fn strip_range(source: &str, from: usize, to: usize, language: Language, depth: usize, output: &mut Stripped) {
    let bytes = source.as_bytes();
    let mut i = from;
    while i < to {
        let Some(&byte) = bytes.get(i) else { break };
        let next = bytes.get(i + 1).copied().filter(|_| i + 1 < to);
        if byte == b'\n' {
            output.lines.push(std::mem::take(&mut output.line));
            i += 1;
        } else if byte == b'/' && next == Some(b'/') {
            let end = bytes.get(i..to).and_then(|rest| rest.iter().position(|b| *b == b'\n')).map_or(to, |p| i + p);
            blank(&mut output.line, i, end);
            i = end;
        } else if byte == b'/' && next == Some(b'*') {
            let end = block_comment_end(bytes, i).min(to);
            append_blank(&mut output.lines, &mut output.line, &blanked(source, i, end));
            i = end;
        } else if let Some(literal) = string_literal(source, i, language, depth) {
            let end = literal.end.min(to);
            let mut at = i;
            for (code_from, code_to) in literal.code.into_iter().filter(|(_, code_to)| *code_to <= end) {
                output.literal(source, at, code_from);
                strip_range(source, code_from, code_to, language, depth + 1, output);
                at = code_to;
            }
            output.literal(source, at, end);
            i = end;
        } else if byte == b'\'' && language != Language::Swift {
            match char_literal_end(source, i, language) {
                Some(end) => {
                    output.literal(source, i, end);
                    i = end;
                }
                None => {
                    // A Rust lifetime or label: copy the quote, the name goes through as code.
                    output.line.push('\'');
                    i += 1;
                }
            }
        } else {
            let advance = character_advance(source, i);
            output.line.push_str(source.get(i..i + advance).unwrap_or(" "));
            i += advance;
        }
    }
}

/// The string literal starting at `index`, or `None` when no string starts there.
fn string_literal(source: &str, index: usize, language: Language, depth: usize) -> Option<Literal> {
    match language {
        Language::Rust => rust_string_end(source.as_bytes(), index).map(|end| Literal { end, code: Vec::new() }),
        Language::Swift | Language::Kotlin => interpolated_string(source, index, language, depth),
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

/// A Swift or Kotlin string literal at `index`, with the code its interpolations hold. Swift's `#`
/// delimiters come first, then `"""` or `"`. A delimited string and a `"""` block end only at their own
/// closing quotes, so their contents never end them early; a one-line `"…"` cannot span lines in either
/// language, so a newline ends it and a quote the stripper misreads cannot carry its error onto the next
/// line. Kotlin's `"""` has no escapes at all, which is why the languages are read apart.
fn interpolated_string(source: &str, index: usize, language: Language, depth: usize) -> Option<Literal> {
    let bytes = source.as_bytes();
    let mut i = index;
    while language == Language::Swift && bytes.get(i) == Some(&b'#') {
        i += 1;
    }
    let hashes = i - index;
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    let quotes = if bytes.get(i..).is_some_and(|rest| rest.starts_with(b"\"\"\"")) { 3 } else { 1 };
    let escapes = !(language == Language::Kotlin && quotes == 3);
    let mut at = i + quotes;
    let mut code = Vec::new();
    while let Some(&byte) = bytes.get(at) {
        if closes(bytes, at, quotes, hashes) {
            let mut end = at + quotes + hashes;
            // Kotlin's `""""` closes on the last three quotes, the extra one belonging to the string.
            while hashes == 0 && quotes == 3 && bytes.get(end) == Some(&b'"') {
                end += 1;
            }
            return Some(Literal { end, code });
        }
        if byte == b'\n' && quotes == 1 {
            return Some(Literal { end: at, code });
        }
        let delimited_escape = byte == b'\\' && bytes.get(at + 1..at + 1 + hashes).is_some_and(|run| run.iter().all(|c| *c == b'#'));
        if escapes && delimited_escape {
            let after = at + 1 + hashes;
            if language == Language::Swift
                && bytes.get(after) == Some(&b'(')
                && depth < MAX_NESTING
                && let Some(close) = matching_close(source, after, (b'(', b')'), language, quotes == 1, depth)
            {
                code.push((after + 1, close));
                at = close + 1;
                continue;
            }
            // Any other escape is one more character, which can never close the string.
            at = after + 1;
            continue;
        }
        if language == Language::Kotlin
            && byte == b'$'
            && bytes.get(at + 1) == Some(&b'{')
            && depth < MAX_NESTING
            && let Some(close) = matching_close(source, at + 1, (b'{', b'}'), language, quotes == 1, depth)
        {
            code.push((at + 2, close));
            at = close + 1;
            continue;
        }
        at += 1;
    }
    Some(Literal { end: bytes.len(), code })
}

/// Does a run of `quotes` double quotes followed by `hashes` hashes start at `at`?
fn closes(bytes: &[u8], at: usize, quotes: usize, hashes: usize) -> bool {
    bytes.get(at..at + quotes).is_some_and(|run| run.iter().all(|c| *c == b'"'))
        && bytes.get(at + quotes..at + quotes + hashes).is_some_and(|run| run.iter().all(|c| *c == b'#'))
}

/// The bracket that closes an interpolation opened at `open`, counting nested brackets and stepping over
/// the strings, char literals and comments inside it, so `"\(f(")"))"` closes where Swift closes it.
/// `None` when nothing closes it (before the line ends, in a one-line string): the text is then read as
/// the string's own.
fn matching_close(source: &str, open: usize, (opener, closer): (u8, u8), language: Language, one_line: bool, depth: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut level = 0usize;
    let mut at = open;
    while let Some(&byte) = bytes.get(at) {
        if byte == b'\n' && one_line {
            return None;
        }
        if at > open {
            if let Some(nested) = string_literal(source, at, language, depth + 1) {
                at = nested.end.max(at + 1);
                continue;
            }
            if byte == b'/' && bytes.get(at + 1) == Some(&b'*') {
                at = block_comment_end(bytes, at);
                continue;
            }
            if byte == b'\''
                && language == Language::Kotlin
                && let Some(end) = char_literal_end(source, at, language)
            {
                at = end;
                continue;
            }
        }
        if byte == opener {
            level += 1;
        } else if byte == closer {
            level = level.checked_sub(1)?;
            if level == 0 {
                return Some(at);
            }
        }
        at += 1;
    }
    None
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
    let mut i = from;
    while i < bytes.len() {
        if closes(bytes, i, quotes, hashes) {
            return i + quotes + hashes;
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

/// Where a char literal starting at `index` ends, if one does: one character or one escape and the
/// closing quote, never across a line. Rust escapes are `'\n'`, `'\''`, `'\x41'` and `'\u{1F600}'`;
/// Kotlin's Unicode escape is `'A'`, exactly four hex digits and no braces, so reading it the Rust
/// way would run on to the next `}` and swallow the code between. A Rust lifetime or label (`'a`,
/// `'outer:`) has no closing quote right after its first character, which is what tells them apart.
fn char_literal_end(source: &str, index: usize, language: Language) -> Option<usize> {
    let bytes = source.as_bytes();
    let first = source.get(index + 1..)?.chars().next()?;
    if first == '\n' {
        return None;
    }
    let mut end = index + 1 + first.len_utf8();
    if first == '\\' {
        match (bytes.get(end), language) {
            (Some(b'u'), Language::Rust) => {
                let rest = bytes.get(end..)?;
                let close = rest.iter().take_while(|c| **c != b'\n').position(|c| *c == b'}')?;
                end += close + 1;
            }
            (Some(b'u'), _) => {
                let digits = bytes.get(end + 1..end + 5)?;
                if !digits.iter().all(u8::is_ascii_hexdigit) {
                    return None;
                }
                end += 5;
            }
            (Some(b'x'), Language::Rust) => end += 3,
            (Some(b'\n') | None, _) => return None,
            (Some(_), _) => end += 1,
        }
    }
    (bytes.get(end) == Some(&b'\'')).then_some(end + 1)
}

/// Rust source with every comment, string and char literal blanked. Doc comments are comments, so a
/// lint or generated marker quoted in one cannot fool a check; raw and byte strings are blanked whole,
/// because an `unsafe` inside one is data; and `'a` is left alone, because a lifetime is not a char
/// literal.
pub(crate) fn strip_rust(source: &str) -> Vec<String> {
    strip(source, Language::Rust, Strings::Blank)
}

/// Swift or Kotlin source with comments and literal text blanked, the code inside interpolations kept.
/// A `"""…"""` literal holds example code, which is exactly the text that would otherwise be reported,
/// so its text is blanked whole.
pub(crate) fn strip_clike(source: &str, language: Language) -> Vec<String> {
    strip(source, language, Strings::Blank)
}

/// Swift or Kotlin source with comments blanked and every literal kept, char literals included: for a
/// scanner that must read a literal's content, a quoted path in a build file say, while still ignoring
/// everything a comment hides. A literal it kept is never mistaken for a comment, and a line holding a
/// literal shows that literal to a grammar that may refuse it.
pub(crate) fn strip_comments(source: &str, language: Language) -> String {
    strip(source, language, Strings::Keep).join("\n")
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
        let stripped = strip_clike(source, Language::Swift);
        assert_eq!(stripped.len(), 7);
        for line in stripped.iter().take(5) {
            assert!(!line.contains("fatalError"), "{line}");
        }
        assert!(stripped[5].contains("exit(1)"));
    }

    #[test]
    fn a_kotlin_char_literal_holding_a_quote_opens_no_string() {
        let stripped = strip_clike("val quote = '\"'\nfun f(s: String?) = s!!.length\nval label = \"x\"\n", Language::Kotlin);
        assert!(stripped[1].contains("!!"), "{}", stripped[1]);
    }

    #[test]
    fn swift_delimited_strings_hold_quotes() {
        let stripped = strip_clike("let json = #\"{\"fatalError\": \"x\"}\"#\nlet multi = #\"\"\"\nfatalError()\n\"\"\"#\nexit(1)\n", Language::Swift);
        assert!(!stripped[0].contains("fatalError"), "{}", stripped[0]);
        assert!(!stripped[2].contains("fatalError"), "{}", stripped[2]);
        assert!(stripped[4].contains("exit(1)"), "{}", stripped[4]);
    }

    #[test]
    fn clike_stripper_nests_block_comments() {
        // Swift and Kotlin block comments nest, so text after an inner `*/` is still commented out
        // until the outer one closes.
        let stripped = strip_clike("/* a /* inner */ fatalError() */ live\nexit(1)\n", Language::Swift);
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
    fn code_inside_a_swift_interpolation_is_code() {
        // `\(…)` holds code in every Swift string form; `\#(…)` in a delimited one. The literal text around
        // it stays blanked, and a string inside the interpolation is still a string.
        let source = "let a = \"value: \\(value.unsafelyUnwrapped)\"\n\
                      let b = #\"raw \\(no) \\#(abort())\"#\n\
                      let c = \"\"\"\n    first \\(x.unsafelyUnwrapped)\n    \"\"\"\n\
                      let d = \"\\(v.map { _ in fatalError(\")\") })\" + after\n\
                      let e = \"escaped \\\\(text) stays text\"\n";
        let stripped = strip_clike(source, Language::Swift);
        assert!(stripped[0].contains("value.unsafelyUnwrapped") && !stripped[0].contains("value:"), "{}", stripped[0]);
        assert!(stripped[1].contains("abort()") && !stripped[1].contains("no") && !stripped[1].contains("raw"), "{}", stripped[1]);
        assert!(stripped[3].contains("x.unsafelyUnwrapped") && !stripped[3].contains("first"), "{}", stripped[3]);
        assert!(stripped[5].contains("fatalError(") && stripped[5].contains("after") && !stripped[5].contains("\")\""), "{}", stripped[5]);
        assert!(!stripped[6].contains("text"), "an escaped backslash opens no interpolation: {}", stripped[6]);
    }

    #[test]
    fn code_inside_a_kotlin_template_is_code() {
        let source = "fun f(s: String?) = \"length ${s!!.length}\"\nval raw = \"\"\"${'$'}x \\${keep!!}\"\"\"\nval escaped = \"\\${not!!}\"\n";
        let stripped = strip_clike(source, Language::Kotlin);
        assert!(stripped[0].contains("s!!.length") && stripped[0].matches("length").count() == 1, "the text is blanked: {}", stripped[0]);
        // A raw string has no escapes, so a backslash before `${` is text and the template is code.
        assert!(stripped[1].contains("keep!!"), "{}", stripped[1]);
        assert!(!stripped[2].contains("not!!"), "an escaped dollar opens no template: {}", stripped[2]);
    }

    #[test]
    fn a_kotlin_unicode_char_escape_is_four_hex_digits() {
        // Read the Rust way, `'\u0041'` would run on to the next `}` and blank the code between.
        let source = "'\\u0041'.let(::plant)\nfun plant(c: Char) = afterEvaluate(::late)\nval close = '}'\n";
        let stripped = strip_clike(source, Language::Kotlin);
        assert!(stripped[0].contains(".let(::plant)") && !stripped[0].contains("0041"), "{}", stripped[0]);
        assert!(stripped[1].contains("afterEvaluate"), "{}", stripped[1]);
        // A char literal never spans a line, in either language that has them.
        assert!(strip_rust("let c = '\nunsafe {}\n").iter().any(|line| line.contains("unsafe")));
        assert!(strip_rust("let u = '\\u{41}';\nlet v = unsafe { 0 };\n")[1].contains("unsafe"));
    }

    #[test]
    fn kept_literals_include_char_literals_and_interpolations() {
        let kept = strip_comments("val a = 'x' // note\nval b = \"${c} d\" /* gone */\n", Language::Kotlin);
        assert_eq!(kept, "val a = 'x'        \nval b = \"${c} d\"           \n");
        // Swift has no char literal: a quote is not a literal there, so nothing after it hides.
        assert!(strip_clike("let a = 'x\nfatalError()\n", Language::Swift)[1].contains("fatalError"));
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
        let alphabet: &[char] = &['"', '\'', '/', '*', '\\', 'r', 'b', 'c', '#', '\n', 'a', 'u', 'x', '{', '}', ' ', 'é', '(', ')', '$', '0'];
        let mut state: u64 = 20261112;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };
        for _ in 0..40000 {
            let length = 1 + next() % 24;
            let text: String = (0..length).map(|_| alphabet[next() % alphabet.len()]).collect();
            assert!(std::panic::catch_unwind(|| strip_rust(&text)).is_ok(), "strip_rust panicked on {text:?}");
            for language in [Language::Swift, Language::Kotlin] {
                let stripped = std::panic::catch_unwind(|| strip_clike(&text, language));
                assert!(stripped.is_ok(), "strip_clike({language:?}) panicked on {text:?}");
                let kept = std::panic::catch_unwind(|| strip_comments(&text, language));
                assert!(kept.is_ok(), "strip_comments({language:?}) panicked on {text:?}");
            }
        }
    }
}
