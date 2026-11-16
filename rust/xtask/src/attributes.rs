//! Rust lint attributes that switch a lint off or down, read from stripped source.
//!
//! Two rules ask the same question of a Rust file — where does an `allow` of this lint sit? — the lint
//! table rule for the never-crash lints and the unsafe rule for `unsafe_code`. Both get one answer from
//! here, so a spelling one of them understands (an attribute spread over lines, an `allow` nested in
//! `cfg_attr`, `# ![…]` with a space, a raw identifier `clippy::r#unwrap_used`) can never be one the other
//! misses. A lint or a level a macro fills in (`#[allow(clippy::$lint)]`, `#[$level(…)]`, a whole
//! `#[$attribute]`) cannot be read before expansion at all, so it is reported as the [`UNREADABLE`] lint,
//! which both rules treat as reaching the lint they guard.

use crate::scan;

/// The lint levels that let a denied lint through: `allow` and `expect` silence it, and `warn` turns
/// an error into something a build only prints.
const WEAKENING_LEVELS: &[&str] = &["allow", "expect", "warn"];

/// What a lint item a macro fills in is reported as: the lint it names is only known after expansion,
/// so every rule must assume it is the one it guards.
pub(crate) const UNREADABLE: &str = "$macro-supplied";

/// The level of `#[$level(…)]`, a weakening level a macro may fill in.
pub(crate) const UNREADABLE_LEVEL: &str = "a macro-supplied level";

/// The level of a whole attribute a macro fills in, `#[$attribute]`. Forwarding attributes onto an item
/// is how macros pass `#[doc]` and `#[cfg]` along, so on one item it is that idiom; on a module or a
/// crate it could be any lint level for all the code inside, and the rules treat it so.
pub(crate) const MACRO_ATTRIBUTE: &str = "a macro-supplied attribute";

/// One weakening level inside one attribute.
#[derive(Debug)]
pub(crate) struct Allowance {
    /// `allow`, `expect` or `warn`, or a level a macro supplies.
    pub(crate) level: &'static str,
    /// The lints it names as the compiler reads them: whitespace removed and each path segment's `r#`
    /// dropped (`clippy::unwrap_used`), or [`UNREADABLE`] for one a macro fills in; a `reason = …` item is
    /// not a lint.
    pub(crate) lints: Vec<String>,
    /// The 1-based line of the attribute's `#`.
    pub(crate) line: usize,
    /// `#![…]`, which applies to the module or crate that holds it, rather than `#[…]` on one item.
    pub(crate) inner: bool,
    /// Byte offset just past the attribute's closing `]`, where the item it applies to begins.
    pub(crate) after: usize,
    /// Inside `cfg_attr(test, …)`, so it applies only when the crate is compiled for its own tests.
    pub(crate) test_only: bool,
}

/// Every weakening level in every attribute of `text`, which must already be stripped: with comments
/// and strings blanked, every bracket and parenthesis left is code, so the nesting can be counted.
pub(crate) fn allowances(text: &str) -> Vec<Allowance> {
    let mut found = Vec::new();
    for attribute in attributes(text) {
        let body = text.get(attribute.open + 1..attribute.close).unwrap_or_default();
        let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        let test_only = compact.starts_with("cfg_attr(test,");
        let mut levels = levels_in(body);
        if compact.starts_with('$') && levels.is_empty() {
            levels.push((MACRO_ATTRIBUTE, vec![UNREADABLE.to_string()]));
        }
        for (level, lints) in levels {
            let line = scan::line_of(text, attribute.hash);
            found.push(Allowance { level, lints, line, inner: attribute.inner, after: attribute.close + 1, test_only });
        }
    }
    found
}

/// Where one attribute sits in stripped text: its `#`, its `[` and the `]` that closes it.
struct Attribute {
    hash: usize,
    inner: bool,
    open: usize,
    close: usize,
}

/// Every attribute in stripped text, `#[…]` and `#![…]`. The compiler reads `#`, `!` and `[` as three
/// tokens, so whitespace may sit between each; with comments and strings blanked, every bracket left is
/// code, so the nesting can be counted.
fn attributes(text: &str) -> Vec<Attribute> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut from = 0usize;
    while let Some(hash) = scan::find(bytes, from, b"#") {
        from = hash + 1;
        let mut open = skip_whitespace(bytes, hash + 1);
        let inner = bytes.get(open) == Some(&b'!');
        if inner {
            open = skip_whitespace(bytes, open + 1);
        }
        if bytes.get(open) != Some(&b'[') {
            continue;
        }
        let Some(close) = matching(bytes, open, b'[', b']') else { continue };
        found.push(Attribute { hash, inner, open, close });
        from = close + 1;
    }
    found
}

/// A lint item as the compiler reads it: whitespace gone, and a raw identifier's `r#` dropped from every
/// path segment, so `r#clippy :: r#unwrap_used` is `clippy::unwrap_used`. An item holding a `$` is a
/// metavariable whose lint is known only after expansion.
fn lint_name(item: &str) -> String {
    let compact: String = item.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.contains('$') {
        return UNREADABLE.to_string();
    }
    compact.split("::").map(|segment| segment.strip_prefix("r#").unwrap_or(segment)).collect::<Vec<_>>().join("::")
}

/// Each weakening level in an attribute body with the lints it names, at any depth, so
/// `cfg_attr(test, allow(x))` is read like `allow(x)`.
fn levels_in(body: &str) -> Vec<(&'static str, Vec<String>)> {
    let bytes = body.as_bytes();
    let mut levels = Vec::new();
    let lints_in = |open: usize, close: usize| -> Vec<String> {
        body.get(open + 1..close).unwrap_or_default().split(',').filter(|item| !item.trim().is_empty() && !item.contains('=')).map(lint_name).collect()
    };
    for level in WEAKENING_LEVELS {
        let mut from = 0usize;
        while let Some(at) = scan::find(bytes, from, level.as_bytes()) {
            from = at + level.len();
            // `r#allow` is not read as `allow` by the compiler's attribute parser, but the `r#` in front
            // is two identifier-like bytes too; only a real word boundary counts.
            let start = if at >= 2 && body.get(at - 2..at) == Some("r#") { at - 2 } else { at };
            if (start > 0 && scan::ident_byte(body, start - 1)) || scan::ident_byte(body, at + level.len()) {
                continue;
            }
            let open = skip_whitespace(bytes, at + level.len());
            if bytes.get(open) != Some(&b'(') {
                continue;
            }
            let Some(close) = matching(bytes, open, b'(', b')') else { continue };
            levels.push((*level, lints_in(open, close)));
        }
    }
    // A level a macro fills in (`$level(clippy::unwrap_used)`) may be any of the weakening ones.
    let mut from = 0usize;
    while let Some(at) = scan::find(bytes, from, b"$") {
        from = at + 1;
        let name_end = (at + 1..bytes.len()).find(|&end| !scan::ident_byte(body, end)).unwrap_or(bytes.len());
        let open = skip_whitespace(bytes, name_end);
        if name_end == at + 1 || bytes.get(open) != Some(&b'(') {
            continue;
        }
        if let Some(close) = matching(bytes, open, b'(', b')') {
            let mut lints = lints_in(open, close);
            lints.push(UNREADABLE.to_string());
            levels.push((UNREADABLE_LEVEL, lints));
        }
    }
    levels
}

/// The name of the module the item starting at `from` declares, when that item is a `mod`. Further
/// attributes (a `#[cfg(…)]` choosing the platform, say) and a visibility may sit between an attribute
/// and its `mod`, on the same line or spread over several. An attribute on a module applies to the whole
/// module, so both rules that ask where an `allow` sits need this answer.
pub(crate) fn declared_module(text: &str, from: usize) -> Option<&str> {
    declared_item(text, from, "mod")
}

/// The name the item starting at `from` declares, when that item is the `keyword` kind of item (`mod`,
/// `fn`): past further attributes, a visibility and, for a function, an `extern` ABI, as the compiler
/// reads them between an attribute and its item.
pub(crate) fn declared_item<'a>(text: &'a str, from: usize, keyword: &str) -> Option<&'a str> {
    let bytes = text.as_bytes();
    let mut at = skip_whitespace(bytes, from);
    while bytes.get(at) == Some(&b'#') && bytes.get(skip_whitespace(bytes, at + 1)) == Some(&b'[') {
        at = skip_whitespace(bytes, matching(bytes, skip_whitespace(bytes, at + 1), b'[', b']')? + 1);
    }
    if word_at(text, at, "pub") {
        at = skip_whitespace(bytes, at + "pub".len());
        if bytes.get(at) == Some(&b'(') {
            at = skip_whitespace(bytes, matching(bytes, at, b'(', b')')? + 1);
        }
    }
    if word_at(text, at, "extern") {
        at = skip_whitespace(bytes, at + "extern".len());
        // The ABI's string, blanked by the stripping but still between its quotes.
        if bytes.get(at) == Some(&b'"') {
            at = skip_whitespace(bytes, scan::find(bytes, at + 1, b"\"")? + 1);
        }
    }
    if !word_at(text, at, keyword) {
        return None;
    }
    let name = skip_whitespace(bytes, at + keyword.len());
    let end = (name..bytes.len()).find(|&offset| !scan::ident_byte(text, offset)).unwrap_or(bytes.len());
    text.get(name..end).filter(|item| !item.is_empty())
}

/// The outer attributes of stripped `text` that hold exactly `content`, whitespace aside (`unsafe(no_mangle)`,
/// say): for each, the span of what it holds, and where the item it applies to begins.
pub(crate) fn outer_attributes_holding(text: &str, content: &str) -> Vec<(std::ops::Range<usize>, usize)> {
    attributes(text)
        .into_iter()
        .filter(|attribute| !attribute.inner)
        .filter(|attribute| {
            let body = text.get(attribute.open + 1..attribute.close).unwrap_or_default();
            body.chars().filter(|c| !c.is_whitespace()).eq(content.chars())
        })
        .map(|attribute| (attribute.open + 1..attribute.close, attribute.close + 1))
        .collect()
}

/// Is the whole word `word` at `at`?
fn word_at(text: &str, at: usize, word: &str) -> bool {
    text.get(at..).is_some_and(|rest| rest.starts_with(word)) && !scan::ident_byte(text, at + word.len())
}

/// The first offset at or after `from` that is not whitespace.
pub(crate) fn skip_whitespace(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
        at += 1;
    }
    at
}

/// The offset of the bracket that closes the one at `open`, counting nesting.
pub(crate) fn matching(bytes: &[u8], open: usize, opener: u8, closer: u8) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, byte) in bytes.iter().enumerate().skip(open) {
        if *byte == opener {
            depth += 1;
        } else if *byte == closer {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(offset);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(source: &str) -> Vec<Allowance> {
        allowances(&scan::strip_rust(source).join("\n"))
    }

    #[test]
    fn inner_and_outer_attributes_are_told_apart() {
        let found = read("#![allow(dead_code)]\n#[expect(clippy::unwrap_used, reason = \"a literal\")]\nfn f() {}\n");
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].inner && found[0].level == "allow" && found[0].lints == ["dead_code"] && found[0].line == 1);
        assert!(!found[1].inner && found[1].level == "expect" && found[1].lints == ["clippy::unwrap_used"] && found[1].line == 2);
    }

    #[test]
    fn an_attribute_spread_over_lines_is_one_attribute() {
        let found = read("fn a() {}\n#![allow(\n    clippy::panic,\n    clippy::todo,\n)]\n");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].lints, ["clippy::panic", "clippy::todo"]);
        assert_eq!(found[0].line, 2);
    }

    #[test]
    fn a_level_nested_in_cfg_attr_counts() {
        let found = read("#![cfg_attr(not(test), warn(clippy::expect_used))]\n#![cfg_attr( test , allow(clippy::panic))]\n");
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].inner && found[0].level == "warn" && found[0].lints == ["clippy::expect_used"] && !found[0].test_only);
        assert!(found[1].test_only, "only `cfg_attr(test, …)` is limited to the crate's tests: {found:?}");
    }

    #[test]
    fn deny_forbid_and_quoted_attributes_are_not_allowances() {
        assert!(read("#![deny(clippy::unwrap_used)]\n#![forbid(unsafe_code)]\nfn allow() {}\n").is_empty());
        assert!(read("// #![allow(clippy::panic)]\nconst S: &str = \"#![allow(clippy::panic)]\";\n").is_empty());
    }

    #[test]
    fn after_points_past_the_attribute() {
        let text = "#[allow(x)] mod platform;";
        let found = allowances(text);
        assert_eq!(text.get(found[0].after..), Some(" mod platform;"));
    }

    #[test]
    fn the_module_an_attribute_sits_on_is_named() {
        let module = |source: &str| {
            let found = allowances(source);
            declared_module(source, found[0].after).map(str::to_string)
        };
        assert_eq!(module("#[allow(x)] mod platform;").as_deref(), Some("platform"));
        assert_eq!(module("#[allow(x)]\n#[cfg(test)]\npub(in crate)\nmod\ninner {}\n").as_deref(), Some("inner"));
        assert_eq!(module("#[allow(x)] pub(crate) mod platform_kit;").as_deref(), Some("platform_kit"));
        assert_eq!(module("#[allow(x)] fn module() {}"), None, "a function is one item, not a module");
        assert_eq!(module("#[allow(x)] model::run();"), None, "a word that merely starts with mod is not one");
    }

    #[test]
    fn spellings_the_compiler_reads_as_the_same_attribute_are_read_alike() {
        // A raw identifier in any path segment, and whitespace between `#`, `!` and `[`, change nothing
        // for the compiler, so they change nothing here.
        for source in [
            "#![allow(clippy::r#unwrap_used)]\n",
            "#![allow(r#clippy::unwrap_used)]\n",
            "# ![allow(clippy::unwrap_used)]\n",
            "#\n!\n[allow(clippy :: unwrap_used)]\n",
        ] {
            let found = read(source);
            assert!(found.len() == 1 && found[0].inner && found[0].lints == ["clippy::unwrap_used"], "{source:?}: {found:?}");
        }
        let outer = read("# [allow(r#unsafe_code)]\nmod platform;\n");
        assert!(outer.len() == 1 && !outer[0].inner && outer[0].lints == ["unsafe_code"], "{outer:?}");
    }

    #[test]
    fn a_lint_or_level_a_macro_fills_in_is_unreadable() {
        let lint = read("macro_rules! quiet { ($lint:ident, $($body:item)*) => { #[allow(clippy::$lint)] pub mod inner { $($body)* } }; }\n");
        assert!(lint.iter().any(|found| found.lints.iter().any(|lint| lint == UNREADABLE)), "{lint:?}");
        let level = read("macro_rules! level { ($level:ident) => { #[$level(clippy::expect_used)] mod inner {} }; }\n");
        assert!(level.iter().any(|found| found.level == UNREADABLE_LEVEL && found.lints.contains(&"clippy::expect_used".to_string())), "{level:?}");
        let whole = read("macro_rules! attr { ($a:meta) => { #[$a] mod inner {} }; }\n");
        assert!(whole.iter().any(|found| found.level == MACRO_ATTRIBUTE && found.lints == [UNREADABLE]), "{whole:?}");
    }

    #[test]
    fn an_unclosed_attribute_is_not_read() {
        assert!(read("#![allow(clippy::panic\n").is_empty());
        assert_eq!(matching(b")(", 0, b'(', b')'), None);
    }
}
