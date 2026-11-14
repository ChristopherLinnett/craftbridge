//! Rule 3 and rule 4 for Kotlin — what hand-written Kotlin may not contain — and the swift-format
//! configuration's never-crash rules.
//!
//! People trust the engine with hours of work, so a bad call produces an error the app can act on and
//! never a crash: Kotlin documents its exceptions. The calls banned here crash or end the process, and `!!`
//! is the same failure one step later. JNI declarations are not banned outright — they are where the
//! platform boundary has to live — so they are confined to a `platform` folder, which keeps the dangerous
//! surface in one small place a reviewer can find, in tests as everywhere else. Swift's half of rules 3 and
//! 4 is `cargo xtask swift-lint`'s, read through the compiler's language server; what stays of Swift here is
//! that the swift-format configuration keeps its three never-crash rules on.
//!
//! Only hand-written code is read: generated bindings are UniFFI's and may contain what our own code may
//! not. Scanning runs on stripped text so a banned call quoted in a comment or a documentation string is not
//! reported, while code inside a string template is read as code.

use crate::scan;
use std::path::Path;

/// A Kotlin file, reduced to what these rules read.
pub(crate) struct PlatformFile<'a> {
    /// How a message names the file.
    pub(crate) path: &'a str,
    /// Directory components between `android/` and the file.
    pub(crate) components: Vec<&'a str>,
    /// Stripped source, one entry per line, with the backticks that escape an identifier removed:
    /// `` `exitProcess`() `` calls `exitProcess` exactly as `exitProcess()` does.
    pub(crate) lines: Vec<String>,
}

impl<'a> PlatformFile<'a> {
    /// Reduce one file's source: stripped so comments read as blanks, and the backticks that escape an
    /// identifier removed — `` `exitProcess`() `` calls `exitProcess` exactly as `exitProcess`() does.
    pub(crate) fn read(path: &'a str, components: Vec<&'a str>, source: &str) -> Self {
        let lines = scan::strip_clike(source, scan::Language::Kotlin).into_iter().map(|line| line.replace('`', "")).collect();
        Self { path, components, lines }
    }
}

/// The swift-format rules that refuse Swift's crashing shortcuts, which the configuration must keep on and
/// no ignore may switch off file-wide (`cargo xtask swift-lint`).
pub(crate) const SWIFT_FORMAT_NEVER_CRASH_RULES: &[&str] = &["NeverForceUnwrap", "NeverUseForceTry", "NeverUseImplicitlyUnwrappedOptionals"];

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

/// Does the text before a name make it a Kotlin declaration rather than a use? `fun exitProcess(code: Int)`
/// and `val exitProcess = …` name something without invoking it.
fn declares(before: &str) -> bool {
    let previous = before.trim_end().rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or_default();
    matches!(previous, "fun" | "var" | "val" | "typealias" | "class")
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

/// Every hand-written Kotlin file under `android/`, stripped and checked with whether it is test code.
fn check_kotlin_tree(root: &Path) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    for file in scan::walk(&root.join("android"), ".kt")? {
        if crate::generated::in_generated(&file.components) || file.components.iter().any(|component| component == "build-logic") {
            continue;
        }
        let shown = crate::report::shown(root, &file.path);
        let reduced = PlatformFile::read(&shown, file.components.iter().map(String::as_str).collect(), &scan::read_text(&file.path)?);
        errors.extend(check_kotlin(&reduced, !is_kotlin_shipped(&file.components)));
    }
    Ok(errors)
}

/// The file Android Lint reads its issue settings from, in a module folder and in any folder of its
/// sources, each applying to everything below it: one can switch a check off for a whole module or package
/// without the build file saying so, so the checks are set in `build-logic` alone. The name is matched
/// without case because this machine's file system is case-insensitive, and a `Lint.xml` would then be
/// opened in place of `lint.xml`, doing its quiet work while a case-sensitive check looked away.
const LINT_CONFIG: &str = "lint.xml";

/// Run the check: Kotlin under `android/`, generated code excepted and the crashing shortcuts allowed in
/// tests, every swift-format configuration under `ios/`, and no Lint configuration file under `android/`.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let mut errors = check_kotlin_tree(root)?;
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
    fn a_qualified_kotlin_call_and_a_two_line_call_are_both_the_call() {
        // A `.` ends an identifier, so the dotted names are beyond the whole-word matcher: each
        // spelling below is the same call and must be reported. A formatter may also move the opening
        // parenthesis onto the next line, so a call split in two is still a call.
        let source = "fun a() = System.exit(9)\nfun b() = java.lang.System.exit(\n    2,\n)\nfun c() = Runtime.getRuntime().halt(3)\nfun d(scope: Scope) = scope.exit()\nfun e() {\n    System.exit\n    (1)\n}\n";
        let file = kotlin("core/src/main/kotlin/A.kt", vec!["core", "src", "main", "kotlin"], source);
        let errors = check_kotlin(&file, false);
        assert_eq!(errors.len(), 4, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains(":1: System.exit(")) && errors.iter().any(|e| e.contains(":2: System.exit(")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains(":5: Runtime.getRuntime().halt(")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains(":8: System.exit(")), "the call split over two lines: {errors:?}");
    }

    #[test]
    fn a_backticked_kotlin_name_is_the_name_and_an_interpolation_is_read_as_code() {
        let source = "fun stop(): Nothing = `exitProcess`(1)\nfun shown(s: String?): String = \"length ${s!!.length}\"\n";
        let errors = check_kotlin_shipped(&kotlin("android/sdk/core/src/main/kotlin/A.kt", vec!["sdk", "core", "src", "main", "kotlin"], source));
        assert!(errors.iter().any(|e| e.contains(":1: exitProcess(")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains(":2: !!")), "{errors:?}");
    }

    #[test]
    fn jni_stays_in_a_platform_folder_in_tests_too() {
        // A test source set keeps the never-crash exception — the shortcuts are what tests use — but not
        // the JNI confinement: AGENTS.md §4 counts tests, everywhere.
        let errors = check_kotlin(
            &kotlin("android/sdk/core/src/test/kotlin/N.kt", vec!["sdk", "core", "src", "test", "kotlin"], "external fun version(): Int\n"),
            false,
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        let errors = check_kotlin(&kotlin("android/sdk/core/src/test/kotlin/N.kt", vec!["sdk", "core", "src", "test", "kotlin"], "val t = s!!\n"), true);
        assert!(errors.is_empty(), "a test set keeps the never-crash exception: {errors:?}");
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
        PlatformFile::read(path, components, source)
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
