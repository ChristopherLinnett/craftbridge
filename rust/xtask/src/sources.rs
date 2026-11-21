//! The never-crash and Lint configuration rules that stay with `cargo xtask rules`.
//!
//! Neither language's source is read here: Swift's never-crash and pointer rules are `cargo xtask
//! swift-lint`'s, read through the compiler's language server, and Kotlin's are detekt's, run in the
//! android stage over Kotlin's own syntax tree and configured once in `android/sdk/build-logic` (plan
//! D-28). What stays is what no source scan can answer: that the swift-format configuration keeps its
//! three never-crash rules on, and that no Lint configuration file switches Lint's checks off.

use crate::scan;
use std::path::Path;

/// The swift-format rules that refuse Swift's crashing shortcuts, which the configuration must keep on and
/// no ignore may switch off file-wide (`cargo xtask swift-lint`).
pub(crate) const SWIFT_FORMAT_NEVER_CRASH_RULES: &[&str] = &["NeverForceUnwrap", "NeverUseForceTry", "NeverUseImplicitlyUnwrappedOptionals"];

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

/// The file Android Lint reads its issue settings from, in a module folder and in any folder of its
/// sources, each applying to everything below it: one can switch a check off for a whole module or package
/// without the build file saying so, so the checks are set in `build-logic` alone. The name is matched
/// without case because this machine's file system is case-insensitive, and a `Lint.xml` would then be
/// opened in place of `lint.xml`, doing its quiet work while a case-sensitive check looked away.
const LINT_CONFIG: &str = "lint.xml";

/// Run the check: every swift-format configuration under `ios/`, and no Lint configuration file under
/// `android/`.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
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
    fn the_check_reads_the_platforms_on_disk_and_finds_nothing() {
        let errors = check(&crate::repo_root()).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// Neither a swift-format configuration nor a Lint configuration git keeps out of `git add .` fails
    /// a rule — neither one a root pattern names, nor one a nested `.gitignore` honours for everything
    /// below it, nor one the checkout's own `info/exclude` names. What git would add is refused wherever
    /// it sits: an untracked copy the ignore rules do not touch, and one tracked past them.
    #[test]
    fn an_ignored_swift_format_or_lint_configuration_fails_no_rule() {
        let scratch = crate::scratch::Scratch::new("sources-git-ignored");
        let git = |args: &[&str]| crate::git::output(&scratch.0, args).unwrap();
        git(&["init", "-q"]);
        let off = r#"{"rules": {"NeverForceUnwrap": false, "NeverUseForceTry": true, "NeverUseImplicitlyUnwrappedOptionals": true}}"#;
        scratch.write(".gitignore", "/ios/sdk/staging/\n");
        scratch.write("ios/sdk/staging/.swift-format", off);
        scratch.write("ios/cache/.gitignore", "*\n");
        scratch.write("ios/cache/.swift-format", off);
        scratch.write(".git/info/exclude", "/android/local/\n");
        scratch.write("android/local/lint.xml", "<lint/>\n");
        assert!(check(&scratch.0).unwrap().is_empty(), "a configuration git ignores governs no code the repository holds");
        // An untracked copy git would add is read as ever.
        scratch.write("android/sdk/lint.xml", "<lint/>\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 1, "an addable Lint configuration: {errors:?}");
        assert!(errors[0].starts_with("android/sdk/lint.xml:"), "{errors:?}");
        // A tracked copy the ignore rules still match is read, forced past them as it was.
        git(&["add", "-f", "ios/sdk/staging/.swift-format"]);
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 2, "tracked, the swift-format copy is read again: {errors:?}");
        assert!(errors.iter().any(|error| error.starts_with("ios/sdk/staging/.swift-format: rules.NeverForceUnwrap must be true")), "{errors:?}");
    }
}
