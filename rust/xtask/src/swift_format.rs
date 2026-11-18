//! The Swift stage's formatter: `swift-format`, the toolchain's own, with the repository's one configuration,
//! for the gate's check of every Swift file (`cargo xtask swift-format`) and for the Swift a generator writes.
//!
//! The check runs `swift-format lint --strict` over every Swift file the rules' walk lists (`cargo xtask
//! files swift`), generated Swift included: it ships as written Swift does, so it meets the same rules. The
//! one exception is the folder UniFFI's Swift bindings are generated into ([`crate::swift_bindings`]):
//! UniFFI's templates are fixed and no setting of theirs meets the rules [`BINDINGS_EXEMPTIONS`] names, so
//! that folder alone is linted with the same configuration and those rules switched off. Every other rule
//! holds there, and every rule holds everywhere else, the command facade's generated Swift included.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// The repository's one swift-format configuration (AGENTS.md §10), which every run here reads.
pub(crate) const CONFIGURATION: &str = "ios/sdk/.swift-format";

/// The rules of [`CONFIGURATION`] UniFFI's Swift cannot meet, each with what in UniFFI's fixed templates
/// breaks it. The bindings' folder alone is linted with these switched off; a rule named here that the
/// configuration does not switch on stops the check, so the list names exactly what it exempts.
const BINDINGS_EXEMPTIONS: &[(&str, &str)] = &[
    (
        "AllPublicDeclarationsHaveDocumentation",
        "UniFFI makes public the converters it writes for every type (`FfiConverter…`, `lift`, `lower`) and documents none of them",
    ),
    (
        "AlwaysUseLowerCamelCase",
        "UniFFI's Swift names its converters and the C library's constants as the C header spells them (`FfiConverterTypeRecovery_lift`, `CALL_ERROR`)",
    ),
    ("UseTripleSlashForDocumentationComments", "UniFFI writes each Rust item's documentation as a `/** … */` block comment"),
    ("NeverUseForceTry", "UniFFI calls every library function that declares no error as `try! rustCall(…)`"),
    ("NeverForceUnwrap", "UniFFI force-unwraps the data of every buffer it reads and force-casts the value a call hands back to its type"),
];

/// The configuration the bindings' folder is linted with: [`CONFIGURATION`]'s own text, `configuration`,
/// with each rule of [`BINDINGS_EXEMPTIONS`] switched off and nothing else changed.
fn bindings_configuration(configuration: &str) -> Result<String, String> {
    let mut parsed: serde_json::Value = serde_json::from_str(configuration).map_err(|error| format!("{CONFIGURATION}: is not valid JSON: {error}"))?;
    let rules = parsed.get_mut("rules").and_then(serde_json::Value::as_object_mut).ok_or_else(|| format!("{CONFIGURATION}: holds no `rules` table"))?;
    for (rule, _) in BINDINGS_EXEMPTIONS {
        match rules.get_mut(*rule) {
            Some(value) if value.as_bool() == Some(true) => *value = serde_json::Value::Bool(false),
            _ => {
                return Err(format!(
                    "{CONFIGURATION}: rules.{rule} is not switched on, so the generated bindings are exempt from no such rule: drop it from the exemptions"
                ));
            }
        }
    }
    serde_json::to_string(&parsed).map_err(|error| format!("cannot write the bindings' configuration: {error}"))
}

/// Run swift-format with `arguments` from `root`, its diagnostics passed through to the stage's output.
/// `Ok(false)` is swift-format's own verdict against the files; `Err` is a swift-format that could not run.
fn swift_format(root: &Path, arguments: &[&str], files: &[impl AsRef<std::ffi::OsStr>]) -> Result<bool, String> {
    let status = Command::new("xcrun")
        .arg("swift-format")
        .args(arguments)
        .args(files)
        .current_dir(root)
        .status()
        .map_err(|error| format!("cannot run `xcrun swift-format`: {error}"))?;
    Ok(status.success())
}

/// Lint every Swift file the walk lists: the bindings' folder with [`bindings_configuration`], every other
/// file with [`CONFIGURATION`]. Both runs happen even when the first fails, so one run names every file that
/// is wrong. A walk that lists no Swift outside the bindings read nothing a check must read.
fn lint(root: &Path) -> Result<bool, String> {
    let (bindings, written): (Vec<String>, Vec<String>) = crate::files::listed(root, "swift")?.into_iter().partition(|file| crate::swift_bindings::holds(file));
    if written.is_empty() {
        return Err("no Swift files for swift-format to read".to_string());
    }
    let mut passed = swift_format(root, &["lint", "--strict", "--configuration", CONFIGURATION], &written)?;
    if !bindings.is_empty() {
        let configuration = bindings_configuration(&crate::scan::read_text(&root.join(CONFIGURATION))?)?;
        passed &= swift_format(root, &["lint", "--strict", "--configuration", &configuration], &bindings)?;
    }
    Ok(passed)
}

/// `cargo xtask swift-format`: the gate's `swift_format` stage.
pub(crate) fn run(root: &Path) -> ExitCode {
    match lint(root) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(failure) => crate::report::failed("swift-format", failure),
    }
}

/// Format `files` in place with the repository's configuration, as the gate's lint reads them: a generator
/// whose output is formatted here writes Swift the check passes, or fails only on rules no formatting meets.
pub(crate) fn format_in_place(files: &[PathBuf]) -> Result<(), String> {
    let root = crate::repository_root()?;
    let configuration = root.join(CONFIGURATION);
    let output = Command::new("xcrun")
        .args(["swift-format", "format", "--in-place", "--configuration"])
        .arg(&configuration)
        .args(files)
        .output()
        .map_err(|error| format!("cannot run `xcrun swift-format`: {error}"))?;
    if !output.status.success() {
        return Err(format!("swift-format format exited with {}: {}", output.status, String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn committed_configuration() -> String {
        std::fs::read_to_string(crate::repo_root().join(CONFIGURATION)).unwrap()
    }

    #[test]
    fn the_bindings_configuration_is_the_repositorys_with_exactly_the_exempted_rules_off() {
        let committed: serde_json::Value = serde_json::from_str(&committed_configuration()).unwrap();
        let derived: serde_json::Value = serde_json::from_str(&bindings_configuration(&committed_configuration()).unwrap()).unwrap();
        let mut expected = committed.clone();
        for (rule, _) in BINDINGS_EXEMPTIONS {
            expected["rules"][rule] = serde_json::Value::Bool(false);
        }
        assert_eq!(derived, expected, "every other key and rule is the repository's");
        assert!(BINDINGS_EXEMPTIONS.iter().all(|(rule, _)| committed["rules"][rule] == serde_json::Value::Bool(true)));
    }

    #[test]
    fn an_exemption_the_configuration_does_not_switch_on_stops_the_check() {
        let off = committed_configuration().replace("\"AlwaysUseLowerCamelCase\": true", "\"AlwaysUseLowerCamelCase\": false");
        assert_eq!(
            bindings_configuration(&off).unwrap_err(),
            "ios/sdk/.swift-format: rules.AlwaysUseLowerCamelCase is not switched on, so the generated bindings are exempt from no such rule: drop it from the exemptions"
        );
        let missing = committed_configuration().replace("\"NeverForceUnwrap\": true,", "");
        assert!(bindings_configuration(&missing).unwrap_err().contains("rules.NeverForceUnwrap is not switched on"));
    }

    #[test]
    fn a_configuration_that_cannot_be_exempted_from_is_refused_in_the_exact_words() {
        assert!(bindings_configuration("{").unwrap_err().starts_with("ios/sdk/.swift-format: is not valid JSON: "));
        assert_eq!(bindings_configuration("{}").unwrap_err(), "ios/sdk/.swift-format: holds no `rules` table");
    }

    #[test]
    fn a_tree_with_no_written_swift_has_nothing_for_swift_format_to_read() {
        let scratch = crate::scratch::Scratch::new("swift-format-none");
        scratch.write("ios/sdk/Sources/CraftBridgeFFI/Platform/Generated/only_bindings.swift", "// bindings only\n");
        assert_eq!(lint(&scratch.0), Err("no Swift files for swift-format to read".to_string()));
    }

    /// Each exemption is one the committed bindings need: with any one of the rules switched back on, the
    /// bindings fail it. The stage itself shows that the exemptions are enough.
    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "runs swift-format through xcrun, which only macOS has")]
    fn every_exempted_rule_is_one_the_committed_bindings_break() {
        let root = crate::repo_root();
        let bindings: Vec<String> = crate::files::listed(&root, "swift").unwrap().into_iter().filter(|file| crate::swift_bindings::holds(file)).collect();
        assert_eq!(bindings.len(), 2, "{bindings:?}");
        let derived: serde_json::Value = serde_json::from_str(&bindings_configuration(&committed_configuration()).unwrap()).unwrap();
        for (rule, _) in BINDINGS_EXEMPTIONS {
            let mut one_on = derived.clone();
            one_on["rules"][rule] = serde_json::Value::Bool(true);
            let output = Command::new("xcrun")
                .args(["swift-format", "lint", "--strict", "--configuration", &one_on.to_string()])
                .args(&bindings)
                .current_dir(&root)
                .output()
                .unwrap();
            assert!(!output.status.success() && String::from_utf8_lossy(&output.stderr).contains(&format!("[{rule}]")), "{rule} is exempted but never broken");
        }
    }
}
