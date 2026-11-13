//! Rule 3 and rule 4 — what hand-written Swift and Kotlin may not contain.
//!
//! People trust the engine with hours of work, so a bad call produces an error the app can act on and
//! never a crash: Swift throws, Kotlin documents its exceptions. The calls banned here abort or crash
//! the process, and a force-unwrap shortcut is the same failure one step later. Pointer APIs and JNI
//! declarations are not banned outright — they are where the platform boundary has to live — so they are
//! confined to a `Platform` (Swift) or `platform` (Kotlin) folder, which keeps the dangerous surface in
//! one small place a reviewer can find.
//!
//! Only hand-written code is read: generated bindings are UniFFI's and may contain what our own code may
//! not, and a test target is allowed the shortcuts library code is not. Scanning runs on stripped text so
//! a banned call quoted in a comment or a documentation string is not reported.
//!
//! swift-format is what refuses Swift's force unwraps, `as!`, `try!` and implicitly unwrapped optionals,
//! so those rules are only real while nothing switches them off: not the configuration, and not a
//! `swift-format-ignore-file` comment, which turns them off for a whole file as `#![allow(…)]` would for a
//! Rust crate. An ignore on one declaration is the one-item allowance `AGENTS.md` leaves to review.

use crate::scan;
use std::path::Path;

/// A Swift or Kotlin file, reduced to what these rules read.
pub(crate) struct PlatformFile<'a> {
    /// How a message names the file.
    pub(crate) path: &'a str,
    /// Directory components between the platform root (`ios`, `android`) and the file.
    pub(crate) components: Vec<&'a str>,
    /// Stripped source, one entry per line.
    pub(crate) lines: Vec<String>,
    /// The comments alone, one entry per line, everything else blanked (bar Kotlin's char literals, which
    /// the stripper blanks as it blanks a comment): where a tool's directives live.
    pub(crate) comments: Vec<String>,
}

impl<'a> PlatformFile<'a> {
    /// Reduce one file's source. A comment is whatever the stripper blanks while it keeps the strings, so
    /// the two readings of the source never disagree about where a comment is.
    pub(crate) fn read(path: &'a str, components: Vec<&'a str>, source: &str) -> Self {
        let kept_strings = scan::strip_comments(source);
        let comments = source
            .split('\n')
            .zip(kept_strings.split('\n'))
            .map(|(written, code)| {
                let bytes: Vec<u8> = written.bytes().zip(code.bytes()).map(|(byte, kept)| if kept == b' ' { byte } else { b' ' }).collect();
                String::from_utf8_lossy(&bytes).into_owned()
            })
            .collect();
        Self { path, components, lines: scan::strip_clike(source), comments }
    }
}

/// The swift-format rules that refuse Swift's crashing shortcuts, which the configuration must keep on.
const SWIFT_FORMAT_NEVER_CRASH_RULES: &[&str] = &["NeverForceUnwrap", "NeverUseForceTry", "NeverUseImplicitlyUnwrappedOptionals"];

/// The comment that switches swift-format rules off for a whole file: alone it means every rule, and with
/// `: A, B` the rules named.
const SWIFT_FORMAT_IGNORE_FILE: &str = "swift-format-ignore-file";

/// A name a Swift file may not call, and why.
const SWIFT_BANNED_CALLS: &[(&str, &str)] = &[
    ("fatalError", "a crash instead of a thrown error"),
    ("precondition", "a crash instead of a thrown error"),
    ("preconditionFailure", "a crash instead of a thrown error"),
    ("assert", "a crash instead of a thrown error"),
    ("assertionFailure", "a crash instead of a thrown error"),
    ("exit", "terminates the host app, which owns the process"),
    ("abort", "ends the host app's process without unwinding, which the engine's panic guard never sees"),
];

/// A Swift property that trusts a value is not nil without checking: an optimised build skips the check
/// and reads undefined memory when the trust is wrong. Not a call — a postfix property access — so it
/// takes its own matcher rather than `call_lines`.
const SWIFT_BANNED_PROPERTY: (&str, &str) =
    ("unsafelyUnwrapped", "trusts a value is not nil without checking, and an optimised build then reads undefined memory instead of failing");

/// Swift identifiers that touch memory the compiler is not tracking, and why they are confined.
const SWIFT_POINTER_APIS: &[&str] = &[
    "Unmanaged",
    "UnsafePointer",
    "UnsafeMutablePointer",
    "UnsafeRawPointer",
    "UnsafeMutableRawPointer",
    "UnsafeBufferPointer",
    "UnsafeMutableBufferPointer",
    "UnsafeRawBufferPointer",
    "UnsafeMutableRawBufferPointer",
    "OpaquePointer",
    "unsafeBitCast",
    "unsafeDowncast",
];

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
/// `(`, unless the name is being declared. Every banned call goes through this one matcher, so no
/// spelling of a call is seen by one rule and missed by another.
fn call_lines(lines: &[String], name: &str) -> Vec<usize> {
    let text = lines.join("\n");
    let bytes = text.as_bytes();
    let mut hits = Vec::new();
    let mut from = 0usize;
    while let Some(offset) = scan::find(bytes, from, name.as_bytes()) {
        let after = offset + name.len();
        let whole = (offset == 0 || !scan::ident_byte(&text, offset - 1)) && !scan::ident_byte(&text, after);
        let mut probe = after;
        while bytes.get(probe).is_some_and(u8::is_ascii_whitespace) {
            probe += 1;
        }
        if whole && bytes.get(probe) == Some(&b'(') && !declares(text.get(..offset).unwrap_or_default()) {
            hits.push(bytes.iter().take(offset).filter(|byte| **byte == b'\n').count() + 1);
        }
        from = offset + 1;
    }
    hits
}

/// Does the text before a name make it a declaration rather than a call? `func exit(code: Int)`,
/// `fun exitProcess(code: Int)` and `case assert(term: String)` name something without invoking it.
fn declares(before: &str) -> bool {
    let previous = before.trim_end().rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or_default();
    matches!(previous, "func" | "fun" | "case")
}

/// Check one Swift file: banned calls anywhere, pointer APIs only under `Platform`.
pub(crate) fn check_swift(file: &PlatformFile<'_>) -> Vec<String> {
    let mut errors = Vec::new();
    let may_use_pointers = file.components.contains(&"Platform");
    for (call, reason) in SWIFT_BANNED_CALLS {
        for line in call_lines(&file.lines, call) {
            errors.push(format!("{}:{line}: {call}(): {reason}", file.path));
        }
    }
    for (index, line) in file.lines.iter().enumerate() {
        if scan::count_whole_word(line, SWIFT_BANNED_PROPERTY.0, true, true) > 0 {
            errors.push(format!("{}:{}: .{}: {}", file.path, index + 1, SWIFT_BANNED_PROPERTY.0, SWIFT_BANNED_PROPERTY.1));
        }
        if scan::count_whole_word(line, "try!", false, false) > 0 {
            errors.push(format!("{}:{}: try!: a crash on a thrown error instead of handling it", file.path, index + 1));
        }
        for pointer in SWIFT_POINTER_APIS {
            if scan::count_whole_word(line, pointer, true, true) > 0 && !may_use_pointers {
                errors.push(format!("{}:{}: {pointer} only under a Platform/ folder", file.path, index + 1));
            }
        }
        // `withUnsafe…` is a family, not a fixed list: the prefix is the rule.
        if !may_use_pointers && let Some(found) = with_unsafe_on_line(line) {
            errors.push(format!("{}:{}: {found} only under a Platform/ folder", file.path, index + 1));
        }
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

/// The never-crash rules a `swift-format-ignore-file` line comment switches off, if any. swift-format reads
/// the directive from a `//` comment only: not from a `///` documentation comment or a block comment.
fn never_crash_rules_ignored(comment: &str) -> Option<String> {
    let after = comment.trim_start().strip_prefix("//").filter(|rest| !rest.starts_with('/'))?;
    let rest = after.trim_start().strip_prefix(SWIFT_FORMAT_IGNORE_FILE)?.trim_start();
    let Some(listed) = rest.strip_prefix(':') else { return Some("every rule, the never-crash ones included".to_string()) };
    let named: Vec<&str> = listed.split(',').map(str::trim).filter(|rule| SWIFT_FORMAT_NEVER_CRASH_RULES.contains(rule)).collect();
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

/// A `withUnsafe…` identifier on the line, if it has one.
fn with_unsafe_on_line(line: &str) -> Option<String> {
    let mut word = String::new();
    let mut found = None;
    for c in line.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            if word.starts_with("withUnsafe") {
                found = Some(std::mem::take(&mut word));
                break;
            }
            word.clear();
        }
    }
    found.or(if word.starts_with("withUnsafe") { Some(word) } else { None })
}

/// Check one shipping Kotlin file: banned expressions anywhere, `external` only under `platform`.
pub(crate) fn check_kotlin(file: &PlatformFile<'_>) -> Vec<String> {
    let mut errors = Vec::new();
    let may_declare_jni = file.components.contains(&"platform");
    for (banned, reason) in KOTLIN_BANNED {
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

/// Is this Swift test code? A test target lives in the package's `Tests/` folder, Swift's own convention,
/// and tests are allowed the shortcuts library code is not. A `Tests` folder inside `Sources/` is part of
/// a library target and ships with it.
fn is_swift_test(components: &[String]) -> bool {
    components.iter().take_while(|component| *component != "Sources").any(|component| component == "Tests")
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
    !components.iter().any(|component| component == "build-logic") && source_set.is_none_or(|set| !set.starts_with("test") && !set.starts_with("androidTest"))
}

/// Every hand-written file of one language under `dir` that `keep` accepts, stripped and checked by
/// `rule`: the one loop both languages go through.
fn check_tree(root: &Path, dir: &str, extension: &str, keep: fn(&[String]) -> bool, rule: fn(&PlatformFile<'_>) -> Vec<String>) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    for file in scan::walk(&root.join(dir), extension)? {
        if crate::generated::in_generated(&file.components) || !keep(&file.components) {
            continue;
        }
        let shown = crate::report::shown(root, &file.path);
        let reduced = PlatformFile::read(&shown, file.components.iter().map(String::as_str).collect(), &scan::read_text(&file.path)?);
        errors.extend(rule(&reduced));
    }
    Ok(errors)
}

/// The file Android Lint reads its issue settings from, in a module folder and in any folder of its
/// sources, each applying to everything below it: one can switch a check off for a whole module or package
/// without the build file saying so, so the checks are set in `build-logic` alone. The name is matched
/// without case because this machine's file system is case-insensitive, and a `Lint.xml` would then be
/// opened in place of `lint.xml`, doing its quiet work while a case-sensitive check looked away.
const LINT_CONFIG: &str = "lint.xml";

/// Run the check: Swift under `ios/`, Kotlin under `android/`, generated code and tests excepted, every
/// swift-format configuration under `ios/`, and no Lint configuration file under `android/`.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let mut errors = check_tree(root, "ios", ".swift", |components| !is_swift_test(components), check_swift)?;
    errors.extend(check_tree(root, "android", ".kt", is_kotlin_shipped, check_kotlin)?);
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
        PlatformFile::read(path, components, source)
    }

    #[test]
    fn swift_banned_calls_are_reported_with_their_line() {
        let source = "func f() -> Int {\n    fatalError(\"unreachable\")\n    return 1\n}\n";
        let errors = check_swift(&swift("ios/sdk/Sources/CraftBridge/Api.swift", vec!["sdk", "Sources", "CraftBridge"], source));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains(":2: fatalError(") && errors[0].contains("thrown error"), "{errors:?}");
    }

    #[test]
    fn qualified_calls_match_and_declarations_labels_and_other_methods_do_not() {
        let kotlin = "fun a() = System.exit(9)\nfun b() = java.lang.System.exit(\n    2,\n)\nfun c() = Runtime.getRuntime().halt(3)\n\
                      fun exitProcess(code: Int) = code\nfun d(scope: Scope) = scope.exit()\n";
        let file = PlatformFile::read("core/src/main/kotlin/A.kt", vec!["core", "src", "main", "kotlin"], kotlin);
        let errors = check_kotlin(&file);
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains(":1: System.exit(")) && errors.iter().any(|e| e.contains(":2: System.exit(")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains(":5: Runtime.getRuntime().halt(")), "{errors:?}");

        let swift_source = "public enum Term {\n    case assert(term: String)\n}\npublic func exit(code: Int) {}\nfunc labels() { configure(exit: true) }\n";
        assert!(check_swift(&swift("Sources/A.swift", vec!["Sources"], swift_source)).is_empty(), "a declaration or a label is not a call");
    }

    #[test]
    fn a_banned_call_split_over_two_lines_is_still_caught() {
        // A formatter may move the opening parenthesis onto the next line; the per-line scanner
        // sees a bare identifier and misses the call, so the two-line rule must not.
        let source = "let a = 1\nfatalError\n(\"boom\")\n";
        let errors = check_swift(&swift("Sources/A.swift", vec!["Sources"], source));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains(":2: fatalError("), "{errors:?}");

        let kotlin = "fun f() {\n    System.exit\n    (1)\n}\n";
        let file = PlatformFile::read("a/sdk/core/src/main/kotlin/A.kt", vec!["sdk", "core", "src", "main", "kotlin"], kotlin);
        let errors = check_kotlin(&file);
        assert!(errors.iter().any(|e| e.contains(":2: System.exit(")), "{errors:?}");

        // A name merely ending in the call's spelling inside a longer identifier does not count:
        let source = "let x = myfatalError\n(3)\n";
        assert!(check_swift(&swift("Sources/A.swift", vec!["Sources"], source)).is_empty(), "identifier suffix matched");
    }

    #[test]
    fn every_swift_banned_call_is_caught_including_a_spaced_call() {
        for (call, _) in SWIFT_BANNED_CALLS {
            let source = format!("let x = 1\n{call} (\"stop\")\n");
            let errors = check_swift(&swift("Sources/A.swift", vec!["Sources"], &source));
            assert!(errors.iter().any(|e| e.contains(":2:")), "{call} not reported: {errors:?}");
        }
        let force_try = "let a = [1]\nlet b = try! read()\n";
        let errors = check_swift(&swift("Sources/A.swift", vec!["Sources"], force_try));
        assert!(errors.iter().any(|e| e.contains(":2: try!")), "{errors:?}");
    }

    #[test]
    fn abort_and_unsafely_unwrapped_are_reported_and_quoted_uses_are_not() {
        let errors = check_swift(&swift("Sources/A.swift", vec!["Sources"], "func stop() {\n    abort()\n}\n"));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains(":2: abort("), "{errors:?}");
        let errors = check_swift(&swift("Sources/A.swift", vec!["Sources"], "let value: Int? = 4\nlet shown = value.unsafelyUnwrapped\n"));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains(":2: .unsafelyUnwrapped"), "{errors:?}");
        // The same words quoted in a comment or a string are not uses of them, and a longer identifier
        // that ends in the property's name is not it either.
        let quoted = "// never call abort() or use .unsafelyUnwrapped\nlet example = \"abort() .unsafelyUnwrapped\"\nlet x = valueNotUnsafelyUnwrapped\n";
        assert!(check_swift(&swift("Sources/A.swift", vec!["Sources"], quoted)).is_empty(), "quoted words are not uses");
    }

    #[test]
    fn every_kotlin_banned_call_is_caught_however_its_qualifier_is_written() {
        // The dotted names are the ones the whole-word test cannot see, because a `.` ends an identifier:
        // each spelling below is the same call, so each must be reported, and a file naming no call must
        // report nothing.
        let source = "fun one(): Int {\n    System.exit(1)\n    return 2\n}\n\nfun two(): Int {\n    Runtime.getRuntime().halt(3)\n    return 4\n}\n";
        let errors = check_kotlin(&kotlin("a/sdk/core/src/main/kotlin/A.kt", vec!["sdk", "core", "src", "main", "kotlin"], source));
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].contains(":2: System.exit(") && errors[1].contains(":7: Runtime.getRuntime().halt("), "{errors:?}");
        let qualified = "import java.lang.System\n\nfun f() {\n    System.exit(1)\n    java.lang.System.exit(2)\n}\n";
        let errors = check_kotlin(&kotlin("a/sdk/core/src/main/kotlin/A.kt", vec!["sdk", "core", "src", "main", "kotlin"], qualified));
        assert_eq!(errors.len(), 2, "a fully qualified name is the same call: {errors:?}");
        // A banned call whose parenthesis is passed through a variable is not a call, so the rule reports
        // nothing, exactly as the compiler does not see a call either.
        let not_a_call = "fun g(exitCode: Int) {\n    val terminator = System::exit\n    terminator(exitCode)\n}\n";
        assert!(check_kotlin(&kotlin("a/sdk/core/src/main/kotlin/A.kt", vec!["sdk", "core", "src", "main", "kotlin"], not_a_call)).is_empty());
    }

    #[test]
    fn swift_pointer_apis_need_a_platform_folder() {
        let outside = "let p: UnsafeRawPointer = raw\nlet q = unsafeBitCast(p, to: Int.self)\n";
        let errors = check_swift(&swift("ios/sdk/Sources/CraftBridge/Reader.swift", vec!["sdk", "Sources", "CraftBridge"], outside));
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].contains(":1: UnsafeRawPointer") && errors[1].contains(":2: unsafeBitCast"), "{errors:?}");

        let inside = "let p: UnsafeRawPointer = raw\nwithUnsafeBytes { }\n";
        let errors = check_swift(&swift("ios/sdk/Sources/Platform/Reader.swift", vec!["sdk", "Sources", "Platform"], inside));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_swift_comment_or_string_is_not_a_violation() {
        let source = "// fatalError is banned\nlet example = \"assert(1)\"\nlet raw = \"\"\"\nexit(2)\n\"\"\"\n";
        let errors = check_swift(&swift("Sources/A.swift", vec!["Sources"], source));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_name_that_merely_contains_a_banned_call_is_not_one() {
        // `app_exit(` and `my_assert` are not the banned calls; whole-identifier matching is the rule.
        let source = "app_exit(1)\nmy_assert(x)\nlet preconditioned = 1\n";
        assert!(check_swift(&swift("Sources/A.swift", vec!["Sources"], source)).is_empty());
    }

    #[test]
    fn a_swift_declaration_labelled_like_a_banned_call_is_not_one() {
        // A `func` defining a name that matches a banned call, or an enum case with a label, declares
        // rather than calls: our own verb is a legal name, and a case's label is not a call either.
        let swift_source = "func exit(code: Int) { }\nlet a = 1\ncase assert(term: String)\nlet b = 2\n";
        assert!(check_swift(&swift("Sources/A.swift", vec!["Sources"], swift_source)).is_empty());
    }

    #[test]
    fn a_kotlin_declaration_named_like_a_banned_call_is_not_one() {
        // A `fun` defining a verb that shares a banned call's name, with its parameter list, declares it
        // rather than calling it: a build tool is free to name its own helper.
        let source = "fun exitProcess(code: Int) { }\n";
        assert!(check_kotlin(&kotlin("a/sdk/core/src/main/kotlin/A.kt", vec!["sdk", "core", "src", "main", "kotlin"], source)).is_empty());
    }

    fn kotlin<'a>(path: &'a str, components: Vec<&'a str>, source: &'a str) -> PlatformFile<'a> {
        PlatformFile::read(path, components, source)
    }

    #[test]
    fn kotlin_banned_expressions_are_reported_with_their_line() {
        let source = "fun f(s: String?): Int {\n    val t = s!!\n    TODO(\"later\")\n    exitProcess(1)\n    System.exit(2)\n    Runtime.getRuntime().halt(3)\n    return t.length\n}\n";
        let errors = check_kotlin(&kotlin("android/sdk/core/src/main/java/A.kt", vec!["sdk", "core", "src", "main", "java"], source));
        assert_eq!(errors.len(), 5, "{errors:?}");
        assert!(errors[0].contains(":2: !!"), "{errors:?}");
        assert!(errors[1].contains(":3: TODO(") && errors[4].contains(":6: Runtime.getRuntime().halt("), "{errors:?}");
    }

    #[test]
    fn kotlin_external_needs_a_platform_folder() {
        let outside = "external fun nativeCall(handle: Long)\n\nexternal val version: Int\n";
        let errors = check_kotlin(&kotlin("android/sdk/core/src/main/java/A.kt", vec!["sdk", "core", "src", "main"], outside));
        assert_eq!(errors.len(), 2, "{errors:?}");
        let inside = "external fun nativeCall(handle: Long)\n";
        let errors = check_kotlin(&kotlin("android/sdk/core/src/main/java/platform/Jni.kt", vec!["sdk", "core", "src", "main", "java", "platform"], inside));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_kotlin_comment_or_string_is_not_a_violation() {
        let source = "// never use !!\nval example = \"exitProcess(1)\"\n/* TODO(unify) */\nval ok = 1\n";
        assert!(check_kotlin(&kotlin("a/src/main/A.kt", vec!["src", "main"], source)).is_empty());
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
        // The convention plugins are an included build of tooling, not a module that ships.
        assert!(!is_kotlin_shipped(&parts("sdk/build-logic/convention/src/main/kotlin")));
        // A folder a build file adds to a source set is not under `src/`, and it ships like any other.
        assert!(is_kotlin_shipped(&parts("sdk/core/lib")));
        assert!(is_swift_test(&parts("sdk/Tests/CraftBridgeTests")));
        assert!(!is_swift_test(&parts("sdk/Sources/CraftBridge")));
        assert!(!is_swift_test(&parts("sdk/Sources/CraftBridge/Tests")), "a Tests folder inside a library target ships");
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
            let errors = check_swift(&swift(path, components.clone(), source));
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
            assert!(check_swift(&swift(path, components.clone(), harmless)).is_empty(), "{harmless:?}");
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
        let file = PlatformFile::read("A.swift", vec![], "let s = \"// not a comment\" // a comment\n/* block\nstill */ let t = 1\n");
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

    #[test]
    fn the_check_reads_the_platforms_on_disk_and_finds_nothing() {
        let errors = check(&crate::repo_root()).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
