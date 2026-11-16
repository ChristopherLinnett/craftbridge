//! What a Rust lint attribute switches off or down, read from the attribute's own tokens.
//!
//! Two rules ask the same question of a Rust file — where does an `allow` of this lint sit? — the lint
//! table rule for the never-crash lints and the unsafe rule for `unsafe_code`. Both get one answer from
//! here: [`crate::rust_syntax`] finds every attribute as `syn` parses it (on a crate root, a module, an
//! item, an expression, a field, a parameter) and hands each one's tokens to [`readings`], so a spelling
//! one rule understands (an attribute spread over lines, an `allow` nested in `cfg_attr`, `# ![…]` with
//! a space, a raw identifier anywhere in it, `#![r#allow(clippy::r#unwrap_used)]`, which the compiler
//! reads as the plain words) can never be one the other misses. A lint or a level a macro fills in
//! (`#[allow(clippy::$lint)]`, `#[$level(…)]`, a whole `#[$attribute]`, also as what a `cfg_attr`
//! applies) cannot be read before expansion at all, so it is reported as the [`UNREADABLE`] lint, which
//! both rules treat as reaching the lint they guard.
//!
//! The tokens are already a tree: a lexer has paired every bracket and parenthesis, so no text is
//! scanned here, only the tree walked, and [`crate::rust_syntax`] has bounded how deep it nests before
//! any of it is read.

use proc_macro2::{Delimiter, TokenTree};
use syn::ext::IdentExt;

/// The lint levels that let a denied lint through: `allow` and `expect` silence it, and `warn` turns
/// an error into something a build only prints.
const WEAKENING_LEVELS: &[&str] = &["allow", "expect", "warn"];

/// What a lint item a macro fills in is reported as: the lint it names is only known after expansion,
/// so every rule must assume it is the one it guards.
pub(crate) const UNREADABLE: &str = "$macro-supplied";

/// The level of `#[$level(…)]`, a weakening level a macro may fill in.
pub(crate) const UNREADABLE_LEVEL: &str = "a macro-supplied level";

/// The level of a whole attribute a macro fills in, `#[$attribute]`, from an attribute its caller wrote
/// as one. Forwarding attributes onto an item is how macros pass `#[doc]` and `#[cfg]` along, so on one
/// item it is that idiom; on a module or a crate it could be any lint level for all the code inside, and
/// the rules treat it so. One whose caller wrote no attribute at all is [`UNREADABLE_LEVEL`].
pub(crate) const MACRO_ATTRIBUTE: &str = "a macro-supplied attribute";

/// The level of `#[rustfmt::skip]`, which is no lint but switches formatting off for what it covers.
pub(crate) const FORMAT_SKIP: &str = "rustfmt::skip";

/// What an attribute applies to, as far as the rules tell places apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Place {
    /// `#![…]`, which applies to the module or crate that holds it.
    Inner,
    /// `#[…]` on a `mod`, which covers everything the module holds. A module a macro names with a
    /// metavariable is `$name`.
    Module(String),
    /// `#[…]`, in a macro's tokens, on what the tokens do not show: an item a metavariable fills in
    /// (`$item`, `$($body)*`), tokens that start no item, or any item in the tokens of a macro call, which
    /// the macro may take apart and put together elsewhere. It may be a module, so the rules read it as
    /// one.
    Unknown,
    /// `#[…]` on a free function.
    Function(String),
    /// `#[…]` on anything else: another item, a statement, an expression, a field, a parameter.
    Other,
}

/// One weakening level inside one attribute.
#[derive(Debug)]
pub(crate) struct Allowance {
    /// `allow`, `expect` or `warn`, a level a macro supplies, or [`FORMAT_SKIP`].
    pub(crate) level: &'static str,
    /// The lints it names as the compiler reads them: each path segment's `r#` dropped
    /// (`clippy::unwrap_used`), or [`UNREADABLE`] for one a macro fills in; a `reason = …` item is not a
    /// lint.
    pub(crate) lints: Vec<String>,
    /// The 1-based line of the attribute's `#`.
    pub(crate) line: usize,
    /// What the attribute applies to.
    pub(crate) place: Place,
    /// Inside `cfg_attr(test, …)`, so it applies only when the crate is compiled for its own tests.
    pub(crate) test_only: bool,
}

/// One level an attribute's tokens hold, before it is placed.
#[derive(Debug)]
pub(crate) struct Reading {
    pub(crate) level: &'static str,
    pub(crate) lints: Vec<String>,
    pub(crate) test_only: bool,
    /// For [`MACRO_ATTRIBUTE`], the metavariable that fills the attribute in (`a` for `#[$a]`): whether
    /// its caller wrote it as an attribute is the macro's matcher's to say.
    pub(crate) supplied_by: Option<String>,
}

/// Every weakening level in the tokens of one attribute (what sits between its brackets), at any depth,
/// so `cfg_attr(test, allow(x))` is read like `allow(x)` and marked as the tests' own.
pub(crate) fn readings(attribute: &[TokenTree]) -> Vec<Reading> {
    let mut found = Vec::new();
    meta(attribute, false, &mut found);
    found
}

/// One attribute's worth of tokens: an attribute's whole body, or one of the attributes a `cfg_attr`
/// applies. Where it is only a metavariable (`$a`), the attribute is one a macro fills in. Where it
/// starts with a `$` and holds more (`$name $args`, `$($tokens)*`), the macro fills in the attribute's
/// own name, which may be any level, and nobody wrote it as an attribute.
fn meta(tokens: &[TokenTree], test_only: bool, found: &mut Vec<Reading>) {
    let before = found.len();
    collect(tokens, test_only, found);
    if found.len() != before {
        return;
    }
    match tokens {
        [TokenTree::Punct(dollar), TokenTree::Ident(name)] if dollar.as_char() == '$' => {
            found.push(Reading { level: MACRO_ATTRIBUTE, lints: vec![UNREADABLE.to_string()], test_only, supplied_by: Some(name.to_string()) });
        }
        [TokenTree::Punct(dollar), ..] if dollar.as_char() == '$' => found.push(unreadable_level(test_only)),
        _ => {}
    }
}

/// A level a macro fills in, whose lints are known only after expansion.
fn unreadable_level(test_only: bool) -> Reading {
    Reading { level: UNREADABLE_LEVEL, lints: vec![UNREADABLE.to_string()], test_only, supplied_by: None }
}

/// Is `tree` a `$`, which starts a metavariable or a repetition of a macro's body?
fn is_dollar(tree: Option<&TokenTree>) -> bool {
    tree.is_some_and(|tree| is_punct(tree, '$'))
}

/// The word an identifier token is to the compiler: a raw identifier's `r#` is no part of it.
fn word(tree: &TokenTree) -> Option<String> {
    match tree {
        TokenTree::Ident(ident) => Some(ident.unraw().to_string()),
        _ => None,
    }
}

fn collect(trees: &[TokenTree], test_only: bool, found: &mut Vec<Reading>) {
    let mut at = 0usize;
    while let Some(tree) = trees.get(at) {
        let group = match trees.get(at + 1) {
            Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis => Some(group),
            _ => None,
        };
        let name = word(tree);
        let level = name.as_deref().and_then(|name| WEAKENING_LEVELS.iter().find(|level| **level == name).copied());
        match (name.as_deref(), level, group) {
            (Some("cfg_attr"), _, Some(group)) => {
                let mut parts = split_commas(group.stream()).into_iter();
                let tests = test_only || parts.next().is_some_and(|predicate| is_test(&predicate));
                for part in parts {
                    meta(&part, tests, found);
                }
                at += 2;
            }
            (_, Some(level), Some(group)) => {
                found.push(Reading { level, lints: lint_names(group.stream()), test_only, supplied_by: None });
                at += 2;
            }
            // A level whose lints a macro fills in as one token tree or a repetition, `allow $lints`,
            // and a `cfg_attr` whose attributes it fills in the same way.
            (_, Some(level), None) if is_dollar(trees.get(at + 1)) => {
                found.push(Reading { level, lints: vec![UNREADABLE.to_string()], test_only, supplied_by: None });
                at += 1;
            }
            (Some("cfg_attr"), _, None) if is_dollar(trees.get(at + 1)) => {
                found.push(unreadable_level(test_only));
                at += 1;
            }
            // The attribute rustfmt still honours under its old name, `#[cfg_attr(rustfmt, rustfmt_skip)]`,
            // which the compiler never sees, since no compiler sets the `rustfmt` cfg.
            (Some("rustfmt_skip"), _, _) => {
                found.push(Reading { level: FORMAT_SKIP, lints: Vec::new(), test_only, supplied_by: None });
                at += 1;
            }
            (Some("rustfmt"), _, _) if is_skip_path(trees, at) => {
                found.push(Reading { level: FORMAT_SKIP, lints: Vec::new(), test_only, supplied_by: None });
                at += 4;
            }
            _ => match tree {
                TokenTree::Punct(dollar) if dollar.as_char() == '$' && matches!(trees.get(at + 1), Some(TokenTree::Ident(_))) => {
                    // A level a macro fills in (`$level(clippy::unwrap_used)`) may be any of the weakening ones.
                    match trees.get(at + 2) {
                        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis => {
                            let mut lints = lint_names(group.stream());
                            lints.push(UNREADABLE.to_string());
                            found.push(Reading { level: UNREADABLE_LEVEL, lints, test_only, supplied_by: None });
                            at += 3;
                        }
                        _ => at += 1,
                    }
                }
                TokenTree::Group(group) => {
                    let inner: Vec<TokenTree> = group.stream().into_iter().collect();
                    collect(&inner, test_only, found);
                    at += 1;
                }
                _ => at += 1,
            },
        }
    }
}

/// Is `rustfmt :: skip` the tokens at `at`, each word as the compiler reads it, or `rustfmt ::` and a
/// metavariable that may be `skip`?
fn is_skip_path(trees: &[TokenTree], at: usize) -> bool {
    matches!(
        (trees.get(at + 1), trees.get(at + 2)),
        (Some(TokenTree::Punct(first)), Some(TokenTree::Punct(second))) if first.as_char() == ':' && second.as_char() == ':'
    ) && (trees.get(at + 3).and_then(word).is_some_and(|skip| skip == "skip") || is_dollar(trees.get(at + 3)))
}

/// Is the predicate of a `cfg_attr` exactly `test`?
fn is_test(predicate: &[TokenTree]) -> bool {
    matches!(predicate, [tree] if word(tree).as_deref() == Some("test"))
}

/// Is `tree` the punctuation character `character`?
pub(crate) fn is_punct(tree: &TokenTree, character: char) -> bool {
    matches!(tree, TokenTree::Punct(punct) if punct.as_char() == character)
}

/// The tokens between a group's top-level commas.
fn split_commas(stream: proc_macro2::TokenStream) -> Vec<Vec<TokenTree>> {
    let mut parts = vec![Vec::new()];
    for tree in stream {
        if is_punct(&tree, ',') {
            parts.push(Vec::new());
        } else if let Some(last) = parts.last_mut() {
            last.push(tree);
        }
    }
    parts
}

/// The lints a level's parentheses name, as the compiler reads them: the identifiers of each path joined
/// by `::` with a raw identifier's `r#` dropped, so `r#clippy :: r#unwrap_used` is `clippy::unwrap_used`.
/// A `reason = …` item is no lint, and an item holding a `$` is a metavariable whose lint is known only
/// after expansion.
fn lint_names(stream: proc_macro2::TokenStream) -> Vec<String> {
    let mut names = Vec::new();
    for part in split_commas(stream) {
        if part.is_empty() || part.iter().any(|tree| is_punct(tree, '=')) {
            continue;
        }
        if part.iter().any(|tree| is_punct(tree, '$')) {
            names.push(UNREADABLE.to_string());
            continue;
        }
        let segments: Vec<String> = part
            .iter()
            .filter_map(|tree| match tree {
                TokenTree::Ident(ident) => Some(ident.unraw().to_string()),
                _ => None,
            })
            .collect();
        names.push(segments.join("::"));
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rust_syntax;

    fn read(source: &str) -> Vec<Allowance> {
        rust_syntax::read(source, "x.rs").unwrap().allowances
    }

    #[test]
    fn inner_and_outer_attributes_are_told_apart() {
        let found = read("#![allow(dead_code)]\n#[expect(clippy::unwrap_used, reason = \"a literal\")]\nfn f() {}\n");
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].place == Place::Inner && found[0].level == "allow" && found[0].lints == ["dead_code"] && found[0].line == 1);
        assert!(
            found[1].place == Place::Function("f".to_string()) && found[1].level == "expect" && found[1].lints == ["clippy::unwrap_used"] && found[1].line == 2
        );
    }

    #[test]
    fn an_attribute_spread_over_lines_is_one_attribute() {
        let found = read("#![allow(\n    clippy::panic,\n    clippy::todo,\n)]\nfn a() {}\n");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].lints, ["clippy::panic", "clippy::todo"]);
        assert_eq!(found[0].line, 1);
    }

    #[test]
    fn a_level_nested_in_cfg_attr_counts() {
        let found = read("#![cfg_attr(not(test), warn(clippy::expect_used))]\n#![cfg_attr( test , allow(clippy::panic))]\n");
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].place == Place::Inner && found[0].level == "warn" && found[0].lints == ["clippy::expect_used"] && !found[0].test_only);
        assert!(found[1].test_only, "only `cfg_attr(test, …)` is limited to the crate's tests: {found:?}");
        let nested = read("#![cfg_attr(feature = \"x\", cfg_attr(test, allow(clippy::panic)))]\n");
        assert!(nested.len() == 1 && nested[0].test_only, "a test-only attribute inside another cfg_attr stays the tests' own: {nested:?}");
    }

    #[test]
    fn deny_forbid_and_quoted_attributes_are_not_allowances() {
        assert!(read("#![deny(clippy::unwrap_used)]\n#![forbid(unsafe_code)]\nfn allow() {}\n").is_empty());
        assert!(read("// #![allow(clippy::panic)]\nconst S: &str = \"#![allow(clippy::panic)]\";\n").is_empty());
        assert!(read("#[doc = \"allow(clippy::panic)\"]\nfn f() {}\n/// allow(clippy::panic)\nfn g() {}\n").is_empty());
    }

    #[test]
    fn the_item_an_attribute_sits_on_is_named() {
        let place = |source: &str| read(source).into_iter().next().map(|found| found.place);
        assert_eq!(place("#[allow(x)] mod platform;"), Some(Place::Module("platform".to_string())));
        assert_eq!(place("#[allow(x)]\n#[cfg(test)]\npub(in crate)\nmod\ninner {}\n"), Some(Place::Module("inner".to_string())));
        assert_eq!(place("#[allow(x)] pub(crate) mod platform_kit;"), Some(Place::Module("platform_kit".to_string())));
        assert_eq!(place("#[allow(x)] fn module() {}"), Some(Place::Function("module".to_string())), "a function is one item, not a module");
        assert_eq!(place("fn f() { #[allow(x)] model::run(); }"), Some(Place::Other));
        assert_eq!(place("fn f() { #![allow(x)] }"), Some(Place::Inner));
    }

    #[test]
    fn an_attribute_on_an_expression_a_field_or_a_parameter_is_read() {
        for source in [
            "fn f() { #[allow(clippy::expect_used)] let _x = 1; }\n",
            "fn f() { let _x = #[allow(clippy::expect_used)] 1; }\n",
            "struct S { #[allow(clippy::expect_used)] a: u8 }\n",
            "fn f(#[allow(clippy::expect_used)] _a: u8) {}\n",
            "enum E { #[allow(clippy::expect_used)] A }\n",
            "impl S { #[allow(clippy::expect_used)] fn f() {} }\n",
            "fn f() { match 1u8 { #[allow(clippy::expect_used)] _ => {} } }\n",
        ] {
            let found = read(source);
            assert!(found.len() == 1 && found[0].lints == ["clippy::expect_used"], "{source:?}: {found:?}");
        }
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
            assert!(found.len() == 1 && found[0].place == Place::Inner && found[0].lints == ["clippy::unwrap_used"], "{source:?}: {found:?}");
        }
        let outer = read("# [allow(r#unsafe_code)]\nmod platform;\n");
        assert!(outer.len() == 1 && outer[0].place == Place::Module("platform".to_string()) && outer[0].lints == ["unsafe_code"], "{outer:?}");
        // The compiler reads a raw identifier as the attribute's own name too: `r#allow` is `allow`, and
        // rustfmt reads `r#rustfmt::r#skip` as `rustfmt::skip`.
        for (source, level) in [
            ("#![r#allow(clippy::expect_used)]\n", "allow"),
            ("#[r#expect(warnings)]\nmod m {}\n", "expect"),
            ("#![r#cfg_attr(r#test, r#warn(clippy::all))]\n", "warn"),
            ("#[r#rustfmt::r#skip]\nmod m {}\n", FORMAT_SKIP),
        ] {
            let found = read(source);
            assert!(found.len() == 1 && found[0].level == level, "{source:?}: {found:?}");
        }
        assert!(read("#![r#cfg_attr(r#test, r#warn(clippy::all))]\n")[0].test_only, "`r#test` is the tests' cfg");
    }

    #[test]
    fn a_lint_or_level_a_macro_fills_in_is_unreadable() {
        let lint = read("macro_rules! quiet { ($lint:ident, $($body:item)*) => { #[allow(clippy::$lint)] pub mod inner { $($body)* } }; }\n");
        assert!(lint.iter().any(|found| found.lints.iter().any(|lint| lint == UNREADABLE) && found.place == Place::Module("inner".to_string())), "{lint:?}");
        let level = read("macro_rules! level { ($level:ident) => { #[$level(clippy::expect_used)] mod inner {} }; }\n");
        assert!(level.iter().any(|found| found.level == UNREADABLE_LEVEL && found.lints.contains(&"clippy::expect_used".to_string())), "{level:?}");
        // A whole attribute its caller wrote as one is forwarded; one its caller wrote bare, as `$a:meta`,
        // is written nowhere as an attribute, so it may be any level.
        let whole = read("macro_rules! attr { (#[$a:meta]) => { #[$a] mod inner {} }; }\n");
        assert!(whole.len() == 1 && whole[0].level == MACRO_ATTRIBUTE && whole[0].lints == [UNREADABLE], "{whole:?}");
        for source in [
            "macro_rules! attr { ($a:meta) => { #[$a] fn f() {} }; }\n",
            "macro_rules! attr { ($a:meta) => { #[cfg_attr(all(), $a)] fn f() {} }; }\n",
            "macro_rules! attr { (#[$b:meta] $a:meta) => { #[$a] fn f() {} }; }\n",
            "fn f() { m! { #[$a] fn g() {} } }\n",
            // The attribute's own name, or all of it, is what a macro fills in with a repetition or a
            // name and its arguments.
            "macro_rules! attr { ($($a:tt)*) => { #[$($a)*] mod inner {} }; }\n",
            "macro_rules! attr { ($($a:tt)*) => { #[cfg_attr(all(), $($a)*)] mod inner {} }; }\n",
            "macro_rules! attr { ($n:ident $args:tt) => { #[$n $args] mod inner {} }; }\n",
            "macro_rules! attr { ($args:tt) => { #[cfg_attr $args] mod inner {} }; }\n",
            "macro_rules! attr { (#[$n:meta] $args:tt) => { #[$n $args] mod inner {} }; }\n",
        ] {
            let found = read(source);
            assert!(found.len() == 1 && found[0].level == UNREADABLE_LEVEL && found[0].lints == [UNREADABLE], "{source:?}: {found:?}");
        }
        // The lints a level takes may be the one thing a macro fills in, as a token tree or a repetition.
        for source in [
            "macro_rules! attr { ($args:tt) => { #[allow $args] mod inner {} }; }\n",
            "macro_rules! attr { ($($args:tt)*) => { #[cfg_attr(all(), expect $($args)*)] mod inner {} }; }\n",
        ] {
            let found = read(source);
            assert!(found.len() == 1 && found[0].lints == [UNREADABLE] && WEAKENING_LEVELS.contains(&found[0].level), "{source:?}: {found:?}");
        }
        let cfg = read("macro_rules! attr { ($(#[$a:meta])*) => { #[cfg_attr(feature = \"x\", $a)] mod inner {} }; }\n");
        assert!(cfg.len() == 1 && cfg[0].level == MACRO_ATTRIBUTE && cfg[0].place == Place::Module("inner".to_string()), "{cfg:?}");
    }

    #[test]
    fn a_module_a_macro_names_with_a_metavariable_is_a_module() {
        let named = read("macro_rules! m { ($name:ident) => { #[allow(clippy::expect_used)] pub mod $name {} }; }\n");
        assert!(named.len() == 1 && named[0].place == Place::Module("$name".to_string()), "{named:?}");
        // An attribute a repetition forwards sits before the item the repetition precedes.
        let forwarded = read("macro_rules! m { ($(#[$a:meta])* $v:vis mod $name:ident) => { $(#[$a])* $v mod $name {} }; }\n");
        assert!(forwarded.len() == 1 && forwarded[0].place == Place::Module("$name".to_string()), "{forwarded:?}");
        let function = read("macro_rules! m { ($(#[$a:meta])* $v:vis fn $name:ident) => { $(#[$a])* $v fn $name() {} }; }\n");
        assert!(function.len() == 1 && function[0].place == Place::Function("$name".to_string()), "{function:?}");
        let structure = read("macro_rules! m { ($(#[$a:meta])* $v:vis struct $name:ident) => { $(#[$a])* $v struct $name; }; }\n");
        assert!(structure.len() == 1 && structure[0].place == Place::Other, "{structure:?}");
    }

    #[test]
    fn what_a_macros_tokens_do_not_show_may_be_a_module() {
        for source in [
            "macro_rules! m { ($(#[$a:meta])* $i:item) => { $(#[$a])* $i }; }\n",
            "macro_rules! m { ($i:item) => { #[allow(clippy::expect_used)] $i }; }\n",
            "macro_rules! m { ($k:tt $n:ident) => { #[allow(clippy::expect_used)] $k $n {} }; }\n",
            "macro_rules! m { (#[$a:meta] fn $f:ident; $($rest:tt)*) => { #[$a] $($rest)* }; }\n",
            "macro_rules! outer { () => { macro_rules! inner { ($i:item) => { #[allow(clippy::expect_used)] $i }; } }; }\n",
            "fn f() { m!(#[allow(clippy::expect_used)]); }\n",
        ] {
            let found = read(source);
            assert!(!found.is_empty() && found.iter().all(|found| found.place == Place::Unknown), "{source:?}: {found:?}");
        }
    }

    #[test]
    fn a_hash_a_macro_may_join_to_brackets_it_is_given_apart_is_an_attribute_of_any_level() {
        // The compiler makes `#` and `[…]` one attribute whichever tokens of a macro's expansion they
        // came from, so a `#` that opens no attribute written out may be one: in the code a macro writes,
        // or in the tokens it is handed.
        for source in [
            "macro_rules! m { ($p:tt) => { $p [allow(clippy::expect_used)] mod inner {} }; }\nm!(#);\n",
            "macro_rules! m { ($b:tt) => { # $b mod inner {} }; }\n",
            "macro_rules! m { ($b:tt) => { #! $b }; }\n",
            "macro_rules! m { ($p:tt, $b:tt) => { $p $b mod inner {} }; }\nm!(#, [allow(clippy::expect_used)]);\n",
            "macro_rules! m { ($p:tt, $a:meta) => { $p [$a] mod inner {} }; }\nm!(#, allow(clippy::expect_used));\n",
            "macro_rules! m { ($p:tt) => { mod inner { $p ! [allow(clippy::expect_used)] } }; }\nm!(#);\n",
        ] {
            let found = read(source);
            assert!(found.iter().any(|found| found.level == UNREADABLE_LEVEL && found.place == Place::Unknown), "{source:?}: {found:?}");
        }
        // An attribute written out whole, and a `#` inside a literal, are no such thing.
        let legal = "macro_rules! m { ($($t:tt)*) => { $($t)* }; }\nm! { #[derive(Debug)] struct S; }\nfn f() { let _ = format!(\"{:#?} #\", 1); }\n";
        assert!(read(legal).is_empty(), "{:?}", read(legal));
    }

    #[test]
    fn an_attribute_written_in_the_tokens_a_macro_is_handed_is_on_what_the_macro_makes_of_it() {
        // A macro that takes its call apart (`$hash $brackets`) and puts it together elsewhere writes
        // no `#` and no brackets of its own, so the attribute it moves onto a module sits, in the
        // call, on the function or item the caller wrote after it.
        let take_apart = "macro_rules! m { ($p:tt $b:tt $($rest:tt)*) => { $($rest)* $p $b mod inner {} }; }\n";
        for call in [
            "m!(#[allow(clippy::expect_used)] fn z() {});\n",
            "m!(#[allow(clippy::expect_used)] struct S;);\n",
            "fn f() { m!(#[allow(unsafe_code)] static X: u8 = 1;); }\n",
            "macro_rules! o { ($($t:tt)*) => { m!($($t)*); }; }\no!(#[allow(clippy::expect_used)] fn z() {});\n",
            "mod outer { m! { #[allow(clippy::expect_used)] fn z() {} } }\n",
            // A call a macro's own code makes hands its tokens over just the same.
            "macro_rules! o { () => { m! { #[allow(clippy::expect_used)] fn z() {} } }; }\n",
            "macro_rules! o { ($n:ident) => { $n! { #[allow(clippy::expect_used)] fn z() {} } }; }\n",
            "macro_rules! o { () => { crate::m!(#[allow(clippy::expect_used)] struct S;); }; }\n",
            // A path a repetition writes (`$($p)::*!`) names the macro as well as any other.
            "macro_rules! o { ($($p:ident)::*) => { $($p)::* ! { #[allow(clippy::expect_used)] fn z() {} } }; }\n",
            // A weak keyword names a macro as well as any word, and a raw identifier is how one is named
            // after a keyword.
            "macro_rules! o { () => { union! { #[allow(clippy::expect_used)] fn z() {} } }; }\n",
            "macro_rules! o { () => { r#if! { #[allow(clippy::expect_used)] fn z() {} } }; }\n",
            // And the `!` itself may be a token tree the macro is handed, alone or at a repetition's end.
            "macro_rules! o { ($n:ident $b:tt) => { $n $b { #[allow(clippy::expect_used)] fn z() {} } }; }\n",
            "macro_rules! o { ($($c:tt)*) => { $($c)* { #[allow(clippy::expect_used)] fn z() {} } }; }\n",
            "macro_rules! o { ($($c:tt),+) => { $($c),+ [ #[allow(clippy::expect_used)] fn z() {} ] }; }\n",
        ] {
            let found = read(&format!("{take_apart}{call}"));
            assert!(found.iter().any(|found| found.level == "allow" && found.place == Place::Unknown), "{call:?}: {found:?}");
        }
        // Written where the compiler reads it, the same attribute is on that function or item.
        assert_eq!(read("#[allow(clippy::expect_used)]\nfn z() {}\n")[0].place, Place::Function("z".to_string()));
        assert_eq!(read("macro_rules! m { ($n:ident) => { #[allow(clippy::expect_used)] fn $n() {} }; }\n")[0].place, Place::Function("$n".to_string()));
        assert_eq!(read("macro_rules! m { () => { #[allow(clippy::expect_used)] struct S; }; }\n")[0].place, Place::Other);
        // A `!` that calls no macro hands nothing over: the never type before a body, a negation, an inner
        // attribute, a `!` a matcher matches.
        for source in [
            "macro_rules! m { () => { fn stop() -> ! { #[allow(clippy::expect_used)] fn inner() {} loop {} } }; }\n",
            "macro_rules! m { () => { fn g() { let _f = || -> ! { #[allow(clippy::expect_used)] fn inner() {} loop {} }; } }; }\n",
            "macro_rules! m { () => { fn g() -> bool { !{ #[allow(clippy::expect_used)] fn inner() {} true } } }; }\n",
            "macro_rules! m { ($a:expr) => { fn g() -> bool { $a != (1) && !($a) } }; }\nmacro_rules! n { () => { mod x { #![doc = \"x\"] #[allow(clippy::expect_used)] fn inner() {} } }; }\n",
            "macro_rules! m { (! ($x:expr)) => { #[allow(clippy::expect_used)] fn inner() {} }; }\n",
            // A metavariable that is no single token tree can be no `!`, so the group after it is the
            // macro's own: an `impl` of a type it is given, a struct it names.
            "macro_rules! m { ($t:ty, $n:ident) => { impl $t { #[allow(clippy::expect_used)] fn inner() {} } struct $n { a: u8 } }; }\n",
            "macro_rules! m { ($c:expr) => { fn g() -> u8 { if $c? { #[allow(clippy::expect_used)] fn inner() {} } 1 } }; }\n",
            // A keyword names no macro, so a `!` after one negates.
            "macro_rules! m { () => { fn g() -> bool { if !{ #[allow(clippy::expect_used)] fn inner() {} true } { true } else { false } } }; }\n",
            "macro_rules! m { ($x:expr) => { fn g() -> i32 { match $x { y if !{ #[allow(clippy::expect_used)] fn inner() {} true } => y, y => y } } }; }\n",
            "macro_rules! m { () => { fn g() -> bool { return !( { #[allow(clippy::expect_used)] fn inner() {} true } ); } }; }\n",
        ] {
            let found = read(source);
            assert!(
                found.iter().all(|found| found.place == Place::Function("inner".to_string()) || found.lints != ["clippy::expect_used"]),
                "{source:?}: {found:?}"
            );
            assert!(found.iter().any(|found| found.place == Place::Function("inner".to_string())), "{source:?}: {found:?}");
        }
        // What a call hands that weakens no level stays what it is.
        assert!(read(&format!("{take_apart}m!(#[derive(Debug)] #[doc = \"x\"] fn z() {{}});\n")).is_empty());
    }

    #[test]
    fn a_matcher_is_a_pattern_and_holds_no_attribute() {
        assert!(read("macro_rules! m { (#[allow(clippy::all)] $(#[$a:meta])* $i:item) => {}; }\n").is_empty());
        let unsafe_uses = rust_syntax::read("macro_rules! m { (unsafe $b:block) => {}; }\n", "x.rs").unwrap().unsafe_uses;
        assert_eq!(unsafe_uses.len(), 1, "the keyword still counts wherever it is written");
    }

    #[test]
    fn rustfmt_skip_is_read_beside_the_lints() {
        let found = read("#![rustfmt::skip]\n#![cfg_attr(test, rustfmt::skip)]\n#[rustfmt::skip]\nmod m;\n#[rustfmt :: skip]\nfn f() {}\n");
        assert_eq!(found.len(), 4, "{found:?}");
        assert!(found.iter().all(|found| found.level == FORMAT_SKIP && found.lints.is_empty()));
        assert!(found[1].test_only && !found[0].test_only);
        // rustfmt still honours its old attribute, which only `cfg_attr(rustfmt, …)` lets compile.
        // A name a macro fills in after `rustfmt::` may be `skip`.
        let filled = read("macro_rules! m { ($w:ident) => { #[rustfmt::$w] mod inner {} }; }\n");
        assert!(filled.len() == 1 && filled[0].level == FORMAT_SKIP, "{filled:?}");
        let old = read("#![cfg_attr(rustfmt, rustfmt_skip)]\n#[cfg_attr(rustfmt, r#rustfmt_skip)]\nmod m {}\n");
        assert!(old.len() == 2 && old.iter().all(|found| found.level == FORMAT_SKIP && !found.test_only), "{old:?}");
    }
}
