//! Rule 3 and rule 4 — what hand-written Swift and Kotlin may not contain.
//!
//! People trust the engine with hours of work, so a bad call produces an error the app can act on and
//! never a crash: Swift throws, Kotlin documents its exceptions. The calls banned here abort or crash
//! the process, and a force-unwrap shortcut is the same failure one step later. Pointer APIs and JNI
//! declarations are not banned outright — they are where the platform boundary has to live — so they are
//! confined to a `Platform` (Swift) or `platform` (Kotlin) folder, which keeps the dangerous surface in
//! one small place a reviewer can find, in tests as everywhere else.
//!
//! Only hand-written code is read: generated bindings are UniFFI's and may contain what our own code may
//! not, and a test target is allowed the crashing shortcuts library code is not. Which Swift is a test is
//! SwiftPM's answer, not a folder name's: a file is test code only inside a target `swift package
//! dump-package` types `test`. Scanning runs on stripped text so a banned call quoted in a comment or a
//! documentation string is not reported, while code inside a string interpolation is read as code.
//!
//! swift-format is what refuses Swift's force unwraps, `as!`, `try!` and implicitly unwrapped optionals,
//! so those rules are only real while nothing switches them off: not the configuration, not a
//! `swift-format-ignore-file` comment, which turns them off for a whole file as `#![allow(…)]` would for a
//! Rust crate, and not swift-format's own exemption for test code, which it grants any file importing
//! XCTest and any function marked `@Test`: so XCTest is refused everywhere (new tests use Swift Testing),
//! and Swift Testing outside a test target. swift-format also reads an implicitly unwrapped optional only
//! in a variable's declaration and never a key path's `!`, so every postfix `!` is refused here as well. An
//! ignore on one declaration is the one-item allowance `AGENTS.md` leaves to review.

use crate::scan;
use std::path::Path;

/// A Swift or Kotlin file, reduced to what these rules read.
pub(crate) struct PlatformFile<'a> {
    /// How a message names the file.
    pub(crate) path: &'a str,
    /// Directory components between the platform root (`ios`, `android`) and the file.
    pub(crate) components: Vec<&'a str>,
    /// Stripped source, one entry per line, with the backticks that escape an identifier removed:
    /// `` `abort`() `` calls `abort` exactly as `abort()` does.
    pub(crate) lines: Vec<String>,
    /// The comments alone, one entry per line, everything else blanked: where a tool's directives live.
    pub(crate) comments: Vec<String>,
}

impl<'a> PlatformFile<'a> {
    /// Reduce one file's source. A comment is whatever the stripper blanks while it keeps every literal,
    /// so the two readings of the source never disagree about where a comment is.
    pub(crate) fn read(path: &'a str, components: Vec<&'a str>, source: &str, language: scan::Language) -> Self {
        let kept_literals = scan::strip_comments(source, language);
        let comments = source
            .split('\n')
            .zip(kept_literals.split('\n'))
            .map(|(written, code)| {
                let bytes: Vec<u8> = written.bytes().zip(code.bytes()).map(|(byte, kept)| if kept == b' ' { byte } else { b' ' }).collect();
                String::from_utf8_lossy(&bytes).into_owned()
            })
            .collect();
        let lines = scan::strip_clike(source, language).into_iter().map(|line| line.replace('`', "")).collect();
        Self { path, components, lines, comments }
    }
}

/// The swift-format rules that refuse Swift's crashing shortcuts, which the configuration must keep on.
const SWIFT_FORMAT_NEVER_CRASH_RULES: &[&str] = &["NeverForceUnwrap", "NeverUseForceTry", "NeverUseImplicitlyUnwrappedOptionals"];

/// The comment that switches swift-format rules off for a whole file: alone it means every rule, and with
/// `: A, B` the rules named.
const SWIFT_FORMAT_IGNORE_FILE: &str = "swift-format-ignore-file";

/// A name a Swift file may not use, and why.
const SWIFT_BANNED_NAMES: &[(&str, &str)] = &[
    ("fatalError", "a crash instead of a thrown error"),
    ("precondition", "a crash instead of a thrown error"),
    ("preconditionFailure", "a crash instead of a thrown error"),
    ("assert", "a crash instead of a thrown error"),
    ("assertionFailure", "a crash instead of a thrown error"),
    ("exit", "terminates the host app, which owns the process"),
    ("abort", "ends the host app's process without unwinding, which the engine's panic guard never sees"),
];

/// The modules that export the banned functions: `Darwin.exit` is the banned call under a qualifier,
/// while `.exit` on anything else is a member of our own (`session.exit()`, `Command.exit(code:)`).
const SWIFT_MODULES: &[&str] = &["Swift", "Darwin", "Glibc", "Musl", "Foundation"];

/// A Swift property that trusts a value is not nil without checking: an optimised build skips the check
/// and reads undefined memory when the trust is wrong. Always a member access, so it takes its own
/// matcher rather than `banned_uses`.
const SWIFT_BANNED_PROPERTY: (&str, &str) =
    ("unsafelyUnwrapped", "trusts a value is not nil without checking, and an optimised build then reads undefined memory instead of failing");

/// Swift identifiers that touch memory the compiler is not tracking. Every `Unsafe…Pointer` type
/// (`AutoreleasingUnsafeMutablePointer` too) and every `withUnsafe…` function is one, whatever its
/// middle; these are the rest.
const SWIFT_POINTER_APIS: &[&str] = &["Unmanaged", "OpaquePointer", "unsafeBitCast", "unsafeDowncast"];

/// A Kotlin expression shipping code may not contain, and why. Every entry but `!!` names a call, and a
/// dotted name is matched whole, qualifier included, so `System.exit(` and `java.lang.System.exit(` are
/// the call while a method of our own named `exit` is not; `!!` is symbolic and counts wherever it
/// appears.
const KOTLIN_BANNED: &[(&str, &str)] = &[
    ("!!", "force unwraps a null to a NullPointerException instead of a documented exception"),
    ("TODO(", "a stub that crashes when reached"),
    ("exitProcess(", "terminates the host app, which owns the process"),
    ("System.exit(", "terminates the host app, which owns the process"),
    ("Runtime.getRuntime().exit(", "terminates the host app, which owns the process"),
    ("Runtime.getRuntime().halt(", "terminates the host process without unwinding"),
];

/// The 1-based lines where `name` is called: the whole name (no identifier byte touching it on either
/// side; a dot inside a qualified name such as `System.exit` is part of the pattern), then any
/// whitespace, newlines included because a formatter may hang the parenthesis on the next line, then
/// `(`, unless the name is being declared. A Kotlin function reference is not a call, so `System::exit`
/// passed around is not one either.
fn call_lines(lines: &[String], name: &str) -> Vec<usize> {
    let text = lines.join("\n");
    let bytes = text.as_bytes();
    let mut hits = Vec::new();
    for offset in whole_words(&text, name) {
        let probe = skip_spaces(bytes, offset + name.len());
        if bytes.get(probe) == Some(&b'(') && !declares(text.get(..offset).unwrap_or_default()) {
            hits.push(scan::line_of(&text, offset));
        }
    }
    hits
}

/// The 1-based lines where a banned Swift name is used at all: called, or taken as a value
/// (`let stop: () -> Never = abort`), which calls it later. A declaration of our own (`func exit`, `case
/// exit(code:)`), an argument or parameter label (`configure(exit: true)`) and a member of anything but a
/// module that exports the function (`session.exit()`, `.exit(code: 0)`) are not uses of it.
fn banned_uses(lines: &[String], name: &str) -> Vec<usize> {
    let text = lines.join("\n");
    let bytes = text.as_bytes();
    let mut hits = Vec::new();
    for offset in whole_words(&text, name) {
        let before = text.get(..offset).unwrap_or_default().trim_end();
        let label = bytes.get(offset + name.len()) == Some(&b':');
        let member = before
            .strip_suffix('.')
            .map(|qualified| qualified.trim_end().rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or_default());
        let foreign_member = member.is_some_and(|qualifier| !SWIFT_MODULES.contains(&qualifier));
        if !label && !foreign_member && !declares(before) {
            hits.push(scan::line_of(&text, offset));
        }
    }
    hits
}

/// The offsets of every whole-word occurrence of `name` in `text`.
fn whole_words(text: &str, name: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut from = 0usize;
    while let Some(offset) = scan::find(bytes, from, name.as_bytes()) {
        if (offset == 0 || !scan::ident_byte(text, offset - 1)) && !scan::ident_byte(text, offset + name.len()) {
            found.push(offset);
        }
        from = offset + 1;
    }
    found
}

/// The first offset at or after `from` that is not whitespace.
fn skip_spaces(bytes: &[u8], from: usize) -> usize {
    let mut probe = from;
    while bytes.get(probe).is_some_and(u8::is_ascii_whitespace) {
        probe += 1;
    }
    probe
}

/// Does the text before a name make it a declaration rather than a use? `func exit(code: Int)`,
/// `fun exitProcess(code: Int)`, `case assert(term: String)` and `let abort = …` name something without
/// invoking it.
fn declares(before: &str) -> bool {
    let previous = before.trim_end().rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or_default();
    matches!(previous, "func" | "fun" | "case" | "var" | "let" | "val" | "typealias" | "struct" | "class" | "enum" | "protocol" | "actor")
}

/// The 1-based lines of each postfix `!`: a force unwrap (`value!`, `\Holder.value!`) or an implicitly
/// unwrapped optional (`Int!`, `init!`), both a crash on nil. A postfix operator touches the operand before
/// it, so a `!` right after a name, a closing bracket or `?` is one; `!=`, `!==` and the prefix `!` are not.
/// `try!` is reported by its own rule and `as!` is named as the force cast it is.
fn postfix_bangs(lines: &[String]) -> Vec<(usize, &'static str)> {
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let bytes = line.as_bytes();
        for (at, byte) in bytes.iter().enumerate() {
            if *byte != b'!' || at == 0 || bytes.get(at + 1) == Some(&b'=') {
                continue;
            }
            let Some(&before) = bytes.get(at - 1) else { continue };
            if !(scan::ident_byte(line, at - 1) || matches!(before, b')' | b']' | b'>' | b'?')) {
                continue;
            }
            let word = line.get(..at).unwrap_or_default().rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or_default();
            match word {
                "try" => {}
                "as" => found.push((index + 1, "as!: a force cast, a crash when the cast fails")),
                _ => found.push((index + 1, "postfix !: a force unwrap or an implicitly unwrapped optional, a crash on nil")),
            }
        }
    }
    found
}

/// The 1-based lines that import `module`, in any form: `import XCTest`, `@testable import XCTest`,
/// `public import XCTest`, `import class XCTest.XCTestCase`, and spread over lines.
fn import_lines(lines: &[String], module: &str) -> Vec<usize> {
    let text = lines.join("\n");
    let bytes = text.as_bytes();
    let mut hits = Vec::new();
    for offset in whole_words(&text, "import") {
        let mut at = skip_spaces(bytes, offset + "import".len());
        let word_end = |from: usize| (from..bytes.len()).find(|&end| !scan::ident_byte(&text, end)).unwrap_or(bytes.len());
        let first = text.get(at..word_end(at)).unwrap_or_default();
        if matches!(first, "class" | "struct" | "enum" | "protocol" | "typealias" | "func" | "let" | "var") {
            at = skip_spaces(bytes, word_end(at));
        }
        if text.get(at..word_end(at)) == Some(module) {
            hits.push(scan::line_of(&text, offset));
        }
    }
    hits
}

/// Every Swift pointer API a line names: the fixed list, any `Unsafe…Pointer` and any `withUnsafe…`.
fn pointer_apis(line: &str) -> Vec<&str> {
    let mut found: Vec<&str> = line
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| SWIFT_POINTER_APIS.contains(word) || (word.contains("Unsafe") && word.ends_with("Pointer")) || word.starts_with("withUnsafe"))
        .collect();
    found.dedup();
    found
}

/// Check one Swift file: the crashing shortcuts outside tests, XCTest everywhere, Swift Testing outside
/// tests, and pointer APIs only under `Platform`.
pub(crate) fn check_swift(file: &PlatformFile<'_>, test: bool) -> Vec<String> {
    let mut errors = Vec::new();
    let may_use_pointers = file.components.contains(&"Platform");
    for line in import_lines(&file.lines, "XCTest") {
        errors.push(format!(
            "{}:{line}: import XCTest: swift-format exempts a file that imports XCTest from its never-crash rules, and new tests use Swift Testing",
            file.path
        ));
    }
    for (index, line) in file.lines.iter().enumerate() {
        for pointer in pointer_apis(line) {
            if !may_use_pointers {
                errors.push(format!("{}:{}: {pointer} only under a Platform/ folder", file.path, index + 1));
            }
        }
    }
    if test {
        return errors;
    }
    for line in import_lines(&file.lines, "Testing") {
        errors.push(format!("{}:{line}: import Testing outside a test target: swift-format exempts test code from its never-crash rules", file.path));
    }
    for (index, line) in file.lines.iter().enumerate() {
        if scan::count_whole_word(line, "@Test", false, true) > 0 {
            errors.push(format!("{}:{}: @Test outside a test target: swift-format exempts a @Test function from its never-crash rules", file.path, index + 1));
        }
    }
    for (name, reason) in SWIFT_BANNED_NAMES {
        for line in banned_uses(&file.lines, name) {
            errors.push(format!("{}:{line}: {name}(): {reason}", file.path));
        }
    }
    for (index, line) in file.lines.iter().enumerate() {
        if scan::count_whole_word(line, SWIFT_BANNED_PROPERTY.0, true, true) > 0 {
            errors.push(format!("{}:{}: .{}: {}", file.path, index + 1, SWIFT_BANNED_PROPERTY.0, SWIFT_BANNED_PROPERTY.1));
        }
        if scan::count_whole_word(line, "try!", false, false) > 0 {
            errors.push(format!("{}:{}: try!: a crash on a thrown error instead of handling it", file.path, index + 1));
        }
    }
    for (line, reason) in postfix_bangs(&file.lines) {
        errors.push(format!("{}:{line}: {reason}", file.path));
    }
    // swift-format reads the directive only from the comments above the file's first line of code.
    let leading = file.lines.iter().take_while(|code| code.trim().is_empty()).count();
    for (index, comment) in file.comments.iter().take(leading).enumerate() {
        if let Some(ignored) = never_crash_rules_ignored(comment) {
            errors.push(format!("{}:{}: {SWIFT_FORMAT_IGNORE_FILE} switches off {ignored} for the whole file", file.path, index + 1));
        }
    }
    errors
}

/// The never-crash rules a `swift-format-ignore-file` line comment switches off, if any. swift-format
/// reads the directive from a `//` comment only, not from a `///` documentation comment or a block
/// comment, and only as the whole comment or followed by `:` and the rules it names (probed against
/// swift-format 6.3: `// swift-format-ignore-file because …` and an empty list switch nothing off).
fn never_crash_rules_ignored(comment: &str) -> Option<String> {
    let after = comment.trim_start().strip_prefix("//").filter(|rest| !rest.starts_with('/'))?;
    let rest = after.trim_start().strip_prefix(SWIFT_FORMAT_IGNORE_FILE)?.trim_end_matches('\r');
    if rest.is_empty() {
        return Some("every rule, the never-crash ones included".to_string());
    }
    let named: Vec<&str> = rest.strip_prefix(':')?.split(',').map(str::trim).filter(|rule| SWIFT_FORMAT_NEVER_CRASH_RULES.contains(rule)).collect();
    (!named.is_empty()).then(|| named.join(", "))
}

/// Check a swift-format configuration: each never-crash rule is switched on.
pub(crate) fn swift_format_violations(source: &str, name: &str) -> Vec<String> {
    let configuration = match serde_json::from_str::<serde_json::Value>(source) {
        Ok(configuration) => configuration,
        Err(error) => return vec![format!("{name}: is not valid JSON: {error}")],
    };
    SWIFT_FORMAT_NEVER_CRASH_RULES
        .iter()
        .filter(|rule| configuration.get("rules").and_then(|rules| rules.get(**rule)).and_then(serde_json::Value::as_bool) != Some(true))
        .map(|rule| format!("{name}: rules.{rule} must be true: swift-format is what refuses Swift's crashing shortcuts"))
        .collect()
}

/// Check one Kotlin file: banned expressions outside tests, `external` only under `platform` in every
/// source set, tests included.
pub(crate) fn check_kotlin(file: &PlatformFile<'_>, test: bool) -> Vec<String> {
    let mut errors = Vec::new();
    let may_declare_jni = file.components.contains(&"platform");
    for (banned, reason) in KOTLIN_BANNED.iter().filter(|_| !test) {
        // A call goes through the one call matcher; a symbolic operator like `!!` is not a word at all,
        // so every occurrence counts.
        let (shown, lines) = match banned.strip_suffix('(') {
            Some(call) => (format!("{call}()"), call_lines(&file.lines, call)),
            None => (
                banned.to_string(),
                (1..=file.lines.len())
                    .filter(|line| file.lines.get(line - 1).is_some_and(|text| scan::count_whole_word(text, banned, false, false) > 0))
                    .collect(),
            ),
        };
        for line in lines {
            errors.push(format!("{}:{line}: {shown}: {reason}", file.path));
        }
    }
    for (index, line) in file.lines.iter().enumerate() {
        if !may_declare_jni && scan::count_whole_word(line, "external", true, true) > 0 {
            errors.push(format!("{}:{}: external (JNI) only under a platform/ folder", file.path, index + 1));
        }
    }
    errors
}

/// Is this Swift file test code? Only a file SwiftPM compiles into a test target; `tests` are those files
/// relative to the platform root (`sdk/Tests/CraftBridgeTests/RenderTests.swift`).
fn is_swift_test(relative: &str, tests: &std::collections::BTreeSet<String>) -> bool {
    tests.contains(relative)
}

/// Does this Kotlin file ship? Every source set of a module ships (`main`, and the variant sets such as
/// `release` or `debug`) except the test sets (`test…`, `androidTest…`). `testFixtures` shares the
/// exception with the tests although fixtures do travel in the library's artifacts: they run only
/// inside a consumer's tests, which is exactly the code the shortcuts exist for. A file outside any
/// `src/` folder ships too, because a build file can add any folder to a source set and only a test set
/// earns the exception. The convention plugins of the included build `build-logic` are build tooling, not
/// library code, and the walk never sees a `.kts`.
fn is_kotlin_shipped(components: &[String]) -> bool {
    let source_set = components.windows(2).find(|pair| pair[0] == "src").map(|pair| pair[1].as_str());
    source_set.is_none_or(|set| !set.starts_with("test") && !set.starts_with("androidTest"))
}

/// Every hand-written file of one language under `dir`, stripped and checked by `rule` with whether it
/// is test code: the one loop both languages go through.
fn check_tree(
    root: &Path,
    dir: &str,
    language: scan::Language,
    test: &dyn Fn(&scan::FoundFile) -> bool,
    rule: fn(&PlatformFile<'_>, bool) -> Vec<String>,
) -> Result<Vec<String>, String> {
    let extension = if language == scan::Language::Swift { ".swift" } else { ".kt" };
    let mut errors = Vec::new();
    for file in scan::walk(&root.join(dir), extension)? {
        if crate::generated::in_generated(&file.components) || file.components.iter().any(|component| component == "build-logic") {
            continue;
        }
        let shown = crate::report::shown(root, &file.path);
        let reduced = PlatformFile::read(&shown, file.components.iter().map(String::as_str).collect(), &scan::read_text(&file.path)?, language);
        errors.extend(rule(&reduced, test(&file)));
    }
    Ok(errors)
}

/// The file Android Lint reads its issue settings from, in a module folder and in any folder of its
/// sources, each applying to everything below it: one can switch a check off for a whole module or package
/// without the build file saying so, so the checks are set in `build-logic` alone. The name is matched
/// without case because this machine's file system is case-insensitive, and a `Lint.xml` would then be
/// opened in place of `lint.xml`, doing its quiet work while a case-sensitive check looked away.
const LINT_CONFIG: &str = "lint.xml";

/// Run the check: Swift under `ios/`, Kotlin under `android/`, generated code excepted and the crashing
/// shortcuts allowed in tests, every swift-format configuration under `ios/`, and no Lint configuration
/// file under `android/`.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let targets = crate::direction::describe_package(&root.join("ios").join("sdk"), "ios/sdk/Package.swift")?.unwrap_or_default();
    let tests = crate::direction::test_sources(&targets).into_iter().map(|source| format!("sdk/{source}")).collect();
    let mut errors = check_tree(root, "ios", scan::Language::Swift, &|file| is_swift_test(&file.relative, &tests), check_swift)?;
    errors.extend(check_tree(root, "android", scan::Language::Kotlin, &|file| !is_kotlin_shipped(&file.components), check_kotlin)?);
    for configuration in scan::walk(&root.join("ios"), ".swift-format")?.into_iter().filter(|file| file.path.ends_with(".swift-format")) {
        errors.extend(swift_format_violations(&scan::read_text(&configuration.path)?, &crate::report::shown(root, &configuration.path)));
    }
    for lint in scan::walk(&root.join("android"), "")?
        .into_iter()
        .filter(|file| file.path.file_name().is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(LINT_CONFIG)))
    {
        errors.push(format!(
            "{}: Lint reads it for everything below its folder, so it can switch checks off that build-logic turns on",
            crate::report::shown(root, &lint.path)
        ));
    }
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swift<'a>(path: &'a str, components: Vec<&'a str>, source: &'a str) -> PlatformFile<'a> {
        PlatformFile::read(path, components, source, scan::Language::Swift)
    }

    /// The rule's verdict on shipping Swift, the case nearly every test asks about.
    fn shipped(file: &PlatformFile<'_>) -> Vec<String> {
        check_swift(file, false)
    }

    #[test]
    fn swift_banned_calls_are_reported_with_their_line() {
        let source = "func f() -> Int {\n    fatalError(\"unreachable\")\n    return 1\n}\n";
        let errors = shipped(&swift("ios/sdk/Sources/CraftBridge/Api.swift", vec!["sdk", "Sources", "CraftBridge"], source));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains(":2: fatalError(") && errors[0].contains("thrown error"), "{errors:?}");
    }

    #[test]
    fn qualified_calls_match_and_declarations_labels_and_other_methods_do_not() {
        let source = "fun a() = System.exit(9)\nfun b() = java.lang.System.exit(\n    2,\n)\nfun c() = Runtime.getRuntime().halt(3)\n\
                      fun exitProcess(code: Int) = code\nfun d(scope: Scope) = scope.exit()\n";
        let file = kotlin("core/src/main/kotlin/A.kt", vec!["core", "src", "main", "kotlin"], source);
        let errors = check_kotlin(&file, false);
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains(":1: System.exit(")) && errors.iter().any(|e| e.contains(":2: System.exit(")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains(":5: Runtime.getRuntime().halt(")), "{errors:?}");

        let swift_source = "public enum Term {\n    case assert(term: String)\n}\npublic func exit(code: Int) {}\nfunc labels() { configure(exit: true) }\n";
        assert!(shipped(&swift("Sources/A.swift", vec!["Sources"], swift_source)).is_empty(), "a declaration or a label is not a call");
    }

    #[test]
    fn a_banned_call_split_over_two_lines_is_still_caught() {
        // A formatter may move the opening parenthesis onto the next line; the per-line scanner
        // sees a bare identifier and misses the call, so the two-line rule must not.
        let source = "let a = 1\nfatalError\n(\"boom\")\n";
        let errors = shipped(&swift("Sources/A.swift", vec!["Sources"], source));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains(":2: fatalError("), "{errors:?}");

        let source = "fun f() {\n    System.exit\n    (1)\n}\n";
        let file = kotlin("a/sdk/core/src/main/kotlin/A.kt", vec!["sdk", "core", "src", "main", "kotlin"], source);
        let errors = check_kotlin(&file, false);
        assert!(errors.iter().any(|e| e.contains(":2: System.exit(")), "{errors:?}");

        // A name merely ending in the call's spelling inside a longer identifier does not count:
        let source = "let x = myfatalError\n(3)\n";
        assert!(shipped(&swift("Sources/A.swift", vec!["Sources"], source)).is_empty(), "identifier suffix matched");
    }

    #[test]
    fn every_swift_banned_call_is_caught_including_a_spaced_call() {
        for (call, _) in SWIFT_BANNED_NAMES {
            let source = format!("let x = 1\n{call} (\"stop\")\n");
            let errors = shipped(&swift("Sources/A.swift", vec!["Sources"], &source));
            assert!(errors.iter().any(|e| e.contains(":2:")), "{call} not reported: {errors:?}");
        }
        let force_try = "let a = [1]\nlet b = try! read()\n";
        let errors = shipped(&swift("Sources/A.swift", vec!["Sources"], force_try));
        assert!(errors.iter().any(|e| e.contains(":2: try!")), "{errors:?}");
    }

    #[test]
    fn abort_and_unsafely_unwrapped_are_reported_and_quoted_uses_are_not() {
        let errors = shipped(&swift("Sources/A.swift", vec!["Sources"], "func stop() {\n    abort()\n}\n"));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains(":2: abort("), "{errors:?}");
        let errors = shipped(&swift("Sources/A.swift", vec!["Sources"], "let value: Int? = 4\nlet shown = value.unsafelyUnwrapped\n"));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains(":2: .unsafelyUnwrapped"), "{errors:?}");
        // The same words quoted in a comment or a string are not uses of them, and a longer identifier
        // that ends in the property's name is not it either.
        let quoted = "// never call abort() or use .unsafelyUnwrapped\nlet example = \"abort() .unsafelyUnwrapped\"\nlet x = valueNotUnsafelyUnwrapped\n";
        assert!(shipped(&swift("Sources/A.swift", vec!["Sources"], quoted)).is_empty(), "quoted words are not uses");
    }

    #[test]
    fn every_kotlin_banned_call_is_caught_however_its_qualifier_is_written() {
        // The dotted names are the ones the whole-word test cannot see, because a `.` ends an identifier:
        // each spelling below is the same call, so each must be reported, and a file naming no call must
        // report nothing.
        let source = "fun one(): Int {\n    System.exit(1)\n    return 2\n}\n\nfun two(): Int {\n    Runtime.getRuntime().halt(3)\n    return 4\n}\n";
        let errors = check_kotlin_shipped(&kotlin("a/sdk/core/src/main/kotlin/A.kt", vec!["sdk", "core", "src", "main", "kotlin"], source));
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].contains(":2: System.exit(") && errors[1].contains(":7: Runtime.getRuntime().halt("), "{errors:?}");
        let qualified = "import java.lang.System\n\nfun f() {\n    System.exit(1)\n    java.lang.System.exit(2)\n}\n";
        let errors = check_kotlin_shipped(&kotlin("a/sdk/core/src/main/kotlin/A.kt", vec!["sdk", "core", "src", "main", "kotlin"], qualified));
        assert_eq!(errors.len(), 2, "a fully qualified name is the same call: {errors:?}");
        // A banned call whose parenthesis is passed through a variable is not a call, so the rule reports
        // nothing, exactly as the compiler does not see a call either.
        let not_a_call = "fun g(exitCode: Int) {\n    val terminator = System::exit\n    terminator(exitCode)\n}\n";
        assert!(check_kotlin_shipped(&kotlin("a/sdk/core/src/main/kotlin/A.kt", vec!["sdk", "core", "src", "main", "kotlin"], not_a_call)).is_empty());
    }

    #[test]
    fn swift_pointer_apis_need_a_platform_folder() {
        let outside = "let p: UnsafeRawPointer = raw\nlet q = unsafeBitCast(p, to: Int.self)\n";
        let errors = shipped(&swift("ios/sdk/Sources/CraftBridge/Reader.swift", vec!["sdk", "Sources", "CraftBridge"], outside));
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].contains(":1: UnsafeRawPointer") && errors[1].contains(":2: unsafeBitCast"), "{errors:?}");

        let inside = "let p: UnsafeRawPointer = raw\nwithUnsafeBytes { }\n";
        let errors = shipped(&swift("ios/sdk/Sources/Platform/Reader.swift", vec!["sdk", "Sources", "Platform"], inside));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_swift_comment_or_string_is_not_a_violation() {
        let source = "// fatalError is banned\nlet example = \"assert(1)\"\nlet raw = \"\"\"\nexit(2)\n\"\"\"\n";
        let errors = shipped(&swift("Sources/A.swift", vec!["Sources"], source));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_name_that_merely_contains_a_banned_call_is_not_one() {
        // `app_exit(` and `my_assert` are not the banned calls; whole-identifier matching is the rule.
        let source = "app_exit(1)\nmy_assert(x)\nlet preconditioned = 1\n";
        assert!(shipped(&swift("Sources/A.swift", vec!["Sources"], source)).is_empty());
    }

    #[test]
    fn a_swift_declaration_labelled_like_a_banned_call_is_not_one() {
        // A `func` defining a name that matches a banned call, or an enum case with a label, declares
        // rather than calls: our own verb is a legal name, and a case's label is not a call either.
        let swift_source = "func exit(code: Int) { }\nlet a = 1\ncase assert(term: String)\nlet b = 2\n";
        assert!(shipped(&swift("Sources/A.swift", vec!["Sources"], swift_source)).is_empty());
    }

    #[test]
    fn a_kotlin_declaration_named_like_a_banned_call_is_not_one() {
        // A `fun` defining a verb that shares a banned call's name, with its parameter list, declares it
        // rather than calling it: a build tool is free to name its own helper.
        let source = "fun exitProcess(code: Int) { }\n";
        assert!(check_kotlin_shipped(&kotlin("a/sdk/core/src/main/kotlin/A.kt", vec!["sdk", "core", "src", "main", "kotlin"], source)).is_empty());
    }

    fn check_kotlin_shipped(file: &PlatformFile<'_>) -> Vec<String> {
        check_kotlin(file, false)
    }

    fn kotlin<'a>(path: &'a str, components: Vec<&'a str>, source: &'a str) -> PlatformFile<'a> {
        PlatformFile::read(path, components, source, scan::Language::Kotlin)
    }

    #[test]
    fn kotlin_banned_expressions_are_reported_with_their_line() {
        let source = "fun f(s: String?): Int {\n    val t = s!!\n    TODO(\"later\")\n    exitProcess(1)\n    System.exit(2)\n    Runtime.getRuntime().halt(3)\n    return t.length\n}\n";
        let errors = check_kotlin_shipped(&kotlin("android/sdk/core/src/main/java/A.kt", vec!["sdk", "core", "src", "main", "java"], source));
        assert_eq!(errors.len(), 5, "{errors:?}");
        assert!(errors[0].contains(":2: !!"), "{errors:?}");
        assert!(errors[1].contains(":3: TODO(") && errors[4].contains(":6: Runtime.getRuntime().halt("), "{errors:?}");
    }

    #[test]
    fn kotlin_external_needs_a_platform_folder() {
        let outside = "external fun nativeCall(handle: Long)\n\nexternal val version: Int\n";
        let errors = check_kotlin_shipped(&kotlin("android/sdk/core/src/main/java/A.kt", vec!["sdk", "core", "src", "main"], outside));
        assert_eq!(errors.len(), 2, "{errors:?}");
        let inside = "external fun nativeCall(handle: Long)\n";
        let errors =
            check_kotlin_shipped(&kotlin("android/sdk/core/src/main/java/platform/Jni.kt", vec!["sdk", "core", "src", "main", "java", "platform"], inside));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_kotlin_comment_or_string_is_not_a_violation() {
        let source = "// never use !!\nval example = \"exitProcess(1)\"\n/* TODO(unify) */\nval ok = 1\n";
        assert!(check_kotlin_shipped(&kotlin("a/src/main/A.kt", vec!["src", "main"], source)).is_empty());
    }

    fn parts(path: &str) -> Vec<String> {
        path.split('/').map(str::to_string).collect()
    }

    #[test]
    fn source_set_and_target_rules_decide_which_files_are_read() {
        assert!(is_kotlin_shipped(&parts("sdk/core/src/main/kotlin")));
        assert!(is_kotlin_shipped(&parts("sdk/core/src/release/kotlin")), "a variant source set ships");
        assert!(!is_kotlin_shipped(&parts("sdk/core/src/test/kotlin")));
        assert!(!is_kotlin_shipped(&parts("sdk/core/src/testDebug/kotlin")));
        assert!(!is_kotlin_shipped(&parts("sdk/core/src/androidTest/kotlin")));
        // Fixtures travel to consumers but run only inside those consumers' tests, which is what the
        // shortcuts exist for: the exception is deliberate, not an accident of the `test` prefix.
        assert!(!is_kotlin_shipped(&parts("sdk/core/src/testFixtures/kotlin")));
        // A folder a build file adds to a source set is not under `src/`, and it ships like any other.
        assert!(is_kotlin_shipped(&parts("sdk/core/lib")));
        // Swift test code is what SwiftPM compiles into its test targets, and nothing else, whatever
        // folder holds it: a shipping target nested in a test target's folder ships, and a test target
        // at `path: "Sources"` excluding the library folders makes none of the library a test.
        let tests: std::collections::BTreeSet<String> =
            ["sdk/Tests/CraftBridgeTests/A.swift", "sdk/Sources/StopTests.swift"].into_iter().map(str::to_string).collect();
        assert!(is_swift_test("sdk/Tests/CraftBridgeTests/A.swift", &tests));
        assert!(is_swift_test("sdk/Sources/StopTests.swift", &tests));
        assert!(!is_swift_test("sdk/Sources/CraftBridge/CraftBridge.swift", &tests), "a library beside a test target's file ships");
        assert!(!is_swift_test("sdk/Tests/CraftBridgeTests/Effects/CraftBridgeEffects.swift", &tests), "a shipping target nested in a test folder ships");
    }

    #[test]
    fn a_file_wide_swift_format_ignore_of_a_never_crash_rule_is_reported() {
        let path = "ios/sdk/Sources/CraftBridge/A.swift";
        let components = vec!["sdk", "Sources", "CraftBridge"];
        // The spellings swift-format honours (each checked against `swift-format lint`): a line comment
        // above the first code, after other comments, with or without a space or a list of rules.
        for (source, line) in [
            ("// swift-format-ignore-file\nlet a = 1\n", 1),
            ("// The core's API.\n\n// swift-format-ignore-file: OrderedImports, NeverForceUnwrap\nimport Foundation\n", 3),
            ("  //swift-format-ignore-file:NeverUseForceTry\n", 1),
        ] {
            let errors = shipped(&swift(path, components.clone(), source));
            assert!(errors.len() == 1 && errors[0].contains(&format!(":{line}: swift-format-ignore-file switches off")), "{source:?}: {errors:?}");
        }
        // And the ones it does not, or that switch off no never-crash rule.
        for harmless in [
            "// swift-format-ignore-file: OrderedImports\n",
            "import Foundation\n// swift-format-ignore-file\n",
            "/// swift-format-ignore-file: NeverForceUnwrap\n",
            "/* swift-format-ignore-file: NeverUseForceTry */\n",
            "/// Doc.\npublic func f() -> Int {\n    // swift-format-ignore: NeverForceUnwrap\n    return 1\n}\n",
            "let note = \"// swift-format-ignore-file\"\n",
        ] {
            assert!(shipped(&swift(path, components.clone(), harmless)).is_empty(), "{harmless:?}");
        }
    }

    #[test]
    fn the_swift_format_configuration_keeps_the_never_crash_rules_on() {
        let on = r#"{"rules": {"NeverForceUnwrap": true, "NeverUseForceTry": true, "NeverUseImplicitlyUnwrappedOptionals": true, "OrderedImports": false}}"#;
        assert!(swift_format_violations(on, ".swift-format").is_empty());
        let off = on.replace("\"NeverUseForceTry\": true", "\"NeverUseForceTry\": false");
        assert_eq!(swift_format_violations(&off, ".swift-format").len(), 1);
        let missing = on.replace("\"NeverForceUnwrap\": true, ", "");
        assert!(swift_format_violations(&missing, ".swift-format")[0].contains("rules.NeverForceUnwrap"), "a rule left out is off");
        assert!(!swift_format_violations("{", ".swift-format").is_empty());
    }

    #[test]
    fn comments_are_read_apart_from_code_and_strings() {
        let file = swift("A.swift", vec![], "let s = \"// not a comment\" // a comment\n/* block\nstill */ let t = 1\n");
        assert_eq!(file.comments[0].trim(), "// a comment");
        assert_eq!(file.comments[1].trim(), "/* block");
        assert_eq!(file.comments[2].trim(), "still */");
    }

    #[test]
    fn a_lint_configuration_anywhere_under_android_is_reported() {
        let scratch = crate::scratch::Scratch::new("lint-config");
        scratch.write("android/sdk/effects/lint.xml", "<lint/>\n");
        scratch.write("android/sdk/core/src/main/kotlin/io/x/lint.xml", "<lint/>\n");
        // A case-insensitive file system opens these in place of `lint.xml`, so they count here.
        scratch.write("android/sdk/core/Lint.xml", "<lint/>\n");
        scratch.write("android/sdk/core/src/main/kotlin/io/x/notlint.xml", "<lint/>\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(
            errors.iter().all(|e| e.ends_with("Lint reads it for everything below its folder, so it can switch checks off that build-logic turns on")),
            "{errors:?}"
        );
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/core/Lint.xml:")), "a case variant: {errors:?}");
    }

    const LIBRARY: &str = "ios/sdk/Sources/CraftBridge/A.swift";

    fn library_components() -> Vec<&'static str> {
        vec!["sdk", "Sources", "CraftBridge"]
    }

    #[test]
    fn xctest_is_refused_in_every_import_form_tests_included() {
        for source in [
            "import XCTest\n",
            "@testable import XCTest\n",
            "@preconcurrency import XCTest\n",
            "public import XCTest\n",
            "import class XCTest.XCTestCase\n",
            "import\n    XCTest\n",
        ] {
            let file = swift(LIBRARY, library_components(), source);
            for test in [false, true] {
                let errors = check_swift(&file, test);
                assert!(errors.iter().any(|e| e.contains(":1: import XCTest")), "{source:?} (test {test}): {errors:?}");
            }
        }
        assert!(shipped(&swift(LIBRARY, library_components(), "import XCTestHelpers\nimport Foundation\n")).is_empty(), "a module merely starting with XCTest");
    }

    #[test]
    fn swift_testing_belongs_to_test_targets_alone() {
        let source = "import Testing\n\n@Test func reads() {\n    let value = 4\n}\n";
        let errors = shipped(&swift(LIBRARY, library_components(), source));
        assert!(errors.iter().any(|e| e.contains(":1: import Testing outside a test target")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains(":3: @Test outside a test target")), "{errors:?}");
        assert!(check_swift(&swift("ios/sdk/Tests/T/A.swift", vec!["sdk", "Tests", "T"], source), true).is_empty());
    }

    #[test]
    fn every_postfix_bang_is_a_crash_on_nil() {
        let source = "public func passed(_ value: Int!) -> Int! { value }\n\
                      public init!(text: String) {}\n\
                      let read = holder[keyPath: \\Holder.value!]\n\
                      let a = values[0]!.count + f()! + (x as! Int)\n\
                      let b = try! load()\n\
                      if !ready && a != b && a !== c { flag = !flag }\n\
                      #if !DEBUG\n";
        let errors = shipped(&swift(LIBRARY, library_components(), source));
        let bangs: Vec<&String> = errors.iter().filter(|e| e.contains("postfix !") || e.contains("as!:")).collect();
        assert!(bangs.iter().filter(|e| e.contains(":1: postfix !")).count() == 2, "both the parameter and the result: {errors:?}");
        assert!(bangs.iter().any(|e| e.contains(":2: postfix !")) && bangs.iter().any(|e| e.contains(":3: postfix !")), "{errors:?}");
        assert_eq!(bangs.iter().filter(|e| e.contains(":4:")).count(), 3, "{errors:?}");
        assert!(bangs.iter().any(|e| e.contains(":4: as!: a force cast")), "{errors:?}");
        assert!(!bangs.iter().any(|e| e.contains(":5:") || e.contains(":6:") || e.contains(":7:")), "try!, !=, !== and a prefix ! are not postfix: {errors:?}");
        assert!(errors.iter().any(|e| e.contains(":5: try!")), "try! keeps its own message: {errors:?}");
        assert!(check_swift(&swift("ios/sdk/Tests/T/A.swift", vec!["sdk", "Tests", "T"], "let v = Int(\"4\")!\n"), true).is_empty(), "tests may");
    }

    #[test]
    fn a_banned_function_taken_as_a_value_is_a_use_and_our_own_members_are_not() {
        let source = "let quit: (Int32) -> Never = exit\n\
                      let halt: () -> Never = abort\n\
                      Darwin.exit(1)\n\
                      Swift.fatalError(\"x\")\n\
                      session.exit()\n\
                      let command = Command.exit(code: 0)\n\
                      return .abort(reason: \"cancelled\")\n\
                      configure(exit: true)\n\
                      case .exit: break\n";
        let errors = shipped(&swift(LIBRARY, library_components(), source));
        for line in 1..=4 {
            assert!(errors.iter().any(|e| e.contains(&format!(":{line}: "))), "line {line}: {errors:?}");
        }
        assert_eq!(errors.len(), 4, "members of our own and labels are not uses: {errors:?}");
    }

    #[test]
    fn a_backticked_name_is_the_name() {
        let errors = shipped(&swift(LIBRARY, library_components(), "public func stop() { `abort`() }\nlet x = value.`unsafelyUnwrapped`\n"));
        assert!(errors.iter().any(|e| e.contains(":1: abort()")) && errors.iter().any(|e| e.contains(":2: .unsafelyUnwrapped")), "{errors:?}");
        let kotlin_source = "fun stop(): Nothing = `exitProcess`(1)\nfun end() = System.`exit`(2)\n";
        let errors = check_kotlin_shipped(&kotlin("android/sdk/core/src/main/kotlin/A.kt", vec!["sdk", "core", "src", "main", "kotlin"], kotlin_source));
        assert_eq!(errors.len(), 2, "{errors:?}");
    }

    #[test]
    fn a_banned_use_inside_an_interpolation_is_reported() {
        let source = "public func describe(_ value: Int?) -> String { \"value: \\(value.unsafelyUnwrapped)\" }\n\
                      public func stopLater() -> String { \"\\(abort())\" }\n\
                      let raw = #\"value: \\#(value.unsafelyUnwrapped)\"#\n\
                      let mapped = \"\\(v.map { _ in fatalError() })\"\n\
                      let text = \"value: \\\\(abort()) is text\"\n";
        let errors = shipped(&swift(LIBRARY, library_components(), source));
        assert!(errors.iter().any(|e| e.contains(":1: .unsafelyUnwrapped")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains(":2: abort()")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains(":3: .unsafelyUnwrapped")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains(":4: fatalError()")), "{errors:?}");
        assert!(!errors.iter().any(|e| e.contains(":5:")), "an escaped backslash is text: {errors:?}");
        let kotlin_source = "fun shown(s: String?): String = \"length ${s!!.length}\"\n";
        let errors = check_kotlin_shipped(&kotlin("android/sdk/core/src/main/kotlin/A.kt", vec!["sdk", "core", "src", "main", "kotlin"], kotlin_source));
        assert!(errors.iter().any(|e| e.contains(":1: !!")), "{errors:?}");
    }

    #[test]
    fn pointer_apis_and_jni_stay_in_platform_folders_in_tests_too() {
        let source = "let p = withUnsafeBytes(of: 1) { $0.count }\nlet q: OpaquePointer? = nil\nlet r: AutoreleasingUnsafeMutablePointer<Int>? = nil\n";
        let errors = check_swift(&swift("ios/sdk/Tests/T/PointerTests.swift", vec!["sdk", "Tests", "T"], source), true);
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("AutoreleasingUnsafeMutablePointer only under a Platform/")), "{errors:?}");
        let errors = check_kotlin(
            &kotlin("android/sdk/core/src/test/kotlin/N.kt", vec!["sdk", "core", "src", "test", "kotlin"], "external fun version(): Int\n"),
            false,
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        let errors = check_kotlin(&kotlin("android/sdk/core/src/test/kotlin/N.kt", vec!["sdk", "core", "src", "test", "kotlin"], "val t = s!!\n"), true);
        assert!(errors.is_empty(), "a test set keeps the never-crash exception: {errors:?}");
    }

    #[test]
    fn an_ignore_file_spelling_swift_format_does_not_read_is_not_refused() {
        for (source, refused) in [
            ("// swift-format-ignore-file because the generator writes this\nlet a = 1\n", false),
            ("// swift-format-ignore-file:\nlet a = 1\n", false),
            ("// swift-format-ignore-file\r\nlet a = 1\n", true),
        ] {
            let errors = shipped(&swift(LIBRARY, library_components(), source));
            assert_eq!(!errors.is_empty(), refused, "{source:?}: {errors:?}");
        }
    }

    #[test]
    fn a_shipped_kotlin_file_on_disk_is_read_as_shipped_and_a_test_one_as_a_test() {
        let scratch = crate::scratch::Scratch::new("kotlin-sets");
        scratch.write("android/sdk/core/src/main/kotlin/A.kt", "internal fun a(s: String?) = s!!\n");
        scratch.write("android/sdk/core/src/test/kotlin/B.kt", "internal fun b(s: String?) = s!!\ninternal external fun c(): Int\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/core/src/main/kotlin/A.kt:1: !!")), "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/core/src/test/kotlin/B.kt:2: external")), "{errors:?}");
    }

    #[test]
    fn the_check_reads_the_platforms_on_disk_and_finds_nothing() {
        let errors = check(&crate::repo_root()).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
