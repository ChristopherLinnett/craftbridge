//! Rust lint attributes that switch a lint off or down, read from stripped source.
//!
//! Two rules ask the same question of a Rust file — where does an `allow` of this lint sit? — the lint
//! table rule for the never-crash lints and the unsafe rule for `unsafe_code`. Both get one answer from
//! here, so a spelling one of them understands (an attribute spread over lines, an `allow` nested in
//! `cfg_attr`) can never be one the other misses.

use crate::scan;

/// The lint levels that let a denied lint through: `allow` and `expect` silence it, and `warn` turns
/// an error into something a build only prints.
const WEAKENING_LEVELS: &[&str] = &["allow", "expect", "warn"];

/// One weakening level inside one attribute.
#[derive(Debug)]
pub(crate) struct Allowance {
    /// `allow`, `expect` or `warn`.
    pub(crate) level: &'static str,
    /// The lints it names, whitespace removed (`clippy::unwrap_used`); a `reason = …` item is not a lint.
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
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut from = 0usize;
    while let Some(hash) = scan::find(bytes, from, b"#") {
        from = hash + 1;
        let mut open = hash + 1;
        let inner = bytes.get(open) == Some(&b'!');
        if inner {
            open += 1;
        }
        open = skip_whitespace(bytes, open);
        if bytes.get(open) != Some(&b'[') {
            continue;
        }
        let Some(close) = matching(bytes, open, b'[', b']') else { continue };
        let body = text.get(open + 1..close).unwrap_or_default();
        let test_only = body.chars().filter(|c| !c.is_whitespace()).collect::<String>().starts_with("cfg_attr(test,");
        for (level, lints) in levels_in(body) {
            found.push(Allowance { level, lints, line: scan::line_of(text, hash), inner, after: close + 1, test_only });
        }
        from = close + 1;
    }
    found
}

/// Each weakening level in an attribute body with the lints it names, at any depth, so
/// `cfg_attr(test, allow(x))` is read like `allow(x)`.
fn levels_in(body: &str) -> Vec<(&'static str, Vec<String>)> {
    let bytes = body.as_bytes();
    let mut levels = Vec::new();
    for level in WEAKENING_LEVELS {
        let mut from = 0usize;
        while let Some(at) = scan::find(bytes, from, level.as_bytes()) {
            from = at + level.len();
            if (at > 0 && scan::ident_byte(body, at - 1)) || scan::ident_byte(body, at + level.len()) {
                continue;
            }
            let open = skip_whitespace(bytes, at + level.len());
            if bytes.get(open) != Some(&b'(') {
                continue;
            }
            let Some(close) = matching(bytes, open, b'(', b')') else { continue };
            let lints = body
                .get(open + 1..close)
                .unwrap_or_default()
                .split(',')
                .map(|item| item.chars().filter(|c| !c.is_whitespace()).collect::<String>())
                .filter(|item| !item.is_empty() && !item.contains('='))
                .collect();
            levels.push((*level, lints));
        }
    }
    levels
}

/// The name of the module the item starting at `from` declares, when that item is a `mod`. Further
/// attributes (a `#[cfg(…)]` choosing the platform, say) and a visibility may sit between an attribute
/// and its `mod`, on the same line or spread over several. An attribute on a module applies to the whole
/// module, so both rules that ask where an `allow` sits need this answer.
pub(crate) fn declared_module(text: &str, from: usize) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut at = skip_whitespace(bytes, from);
    while bytes.get(at) == Some(&b'#') && bytes.get(at + 1) == Some(&b'[') {
        at = skip_whitespace(bytes, matching(bytes, at + 1, b'[', b']')? + 1);
    }
    if word_at(text, at, "pub") {
        at = skip_whitespace(bytes, at + "pub".len());
        if bytes.get(at) == Some(&b'(') {
            at = skip_whitespace(bytes, matching(bytes, at, b'(', b')')? + 1);
        }
    }
    if !word_at(text, at, "mod") {
        return None;
    }
    let name = skip_whitespace(bytes, at + "mod".len());
    let end = (name..bytes.len()).find(|&offset| !scan::ident_byte(text, offset)).unwrap_or(bytes.len());
    text.get(name..end).filter(|module| !module.is_empty())
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
    fn an_unclosed_attribute_is_not_read() {
        assert!(read("#![allow(clippy::panic\n").is_empty());
        assert_eq!(matching(b")(", 0, b'(', b')'), None);
    }
}
