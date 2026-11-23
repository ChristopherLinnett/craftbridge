//! The generators behind `generated/api-surface.json` and the facade model planned from it, the Swift and
//! Kotlin facades and the plugin's version constants, and the comparison of two surfaces the upstream
//! update makes: the codegen binary, built and run by cargo.
//!
//! The codegen links the pinned EffectCraft and reads its command registry at run time, and xtask may not
//! depend on a host tool, so xtask asks cargo for the binary and runs it. The pin the surface records is
//! the commit Cargo builds for the pin `rust/Cargo.toml` writes, in full as `rust/Cargo.lock` records it,
//! read by the upstream rule's own readers: the rule takes the pin in full or abbreviated, and the
//! surface's schema wants at least eight digits, so the spelling never reaches the surface. The version
//! constants take the same full commit, and the codegen gives them the version of the engine it links, as
//! it gives the surface, so neither is read from text.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the surface lives: the folder of the crate that writes it.
pub(crate) const OUTPUT_DIR: &str = "rust/effects/craftbridge-effects-codegen/generated";

/// Where the Swift facade lives: the generated folder of the plugin's Swift target.
pub(crate) const SWIFT_OUTPUT_DIR: &str = "ios/sdk/Sources/CraftBridgeEffects/Generated";

/// Where the Kotlin facade lives: the folder of the package it declares, inside the generated Kotlin source
/// folder of the plugin's Android module. It owns that folder alone, so the UniFFI bindings generator can
/// own the module's `ffi` package folder beside it.
pub(crate) const KOTLIN_OUTPUT_DIR: &str = "android/sdk/effects/src/generated/kotlin/io/github/christopherlinnett/craftbridge/effects";

/// Where the plugin's version constants live: the generated folder of the plugin's own source.
pub(crate) const VERSION_OUTPUT_DIR: &str = "rust/effects/craftbridge-effects/src/generated";

/// Where the command reference lives: the generated folder of the docs tree.
pub(crate) const DOCS_OUTPUT_DIR: &str = "docs/generated";

/// The codegen's package, and the binary it builds.
const CODEGEN: &str = "craftbridge-effects-codegen";

/// Write the surface into `out_dir`, and then the facade model planned from it. Both files are this one
/// step's, written into this one folder: the model is rendered from the surface file the step has just
/// written, so the pair cannot come apart — a pin move rewrites the surface and the model with it.
pub(crate) fn generate(out_dir: &Path) -> Result<(), String> {
    run_codegen(&["surface", "--pin", &pinned_commit(&crate::repository_root()?)?], out_dir)?;
    run_codegen(&["facade-model", &out_dir.join(SURFACE_FILE_NAME).to_string_lossy()], out_dir)
}

/// Write the version constants into `out_dir`.
pub(crate) fn generate_version(out_dir: &Path) -> Result<(), String> {
    run_codegen(&["version", "--pin", &pinned_commit(&crate::repository_root()?)?], out_dir)
}

/// The commit Cargo builds for the pin `rust/Cargo.toml` writes, in full.
fn pinned_commit(root: &Path) -> Result<String, String> {
    let pin = crate::upstream::manifest_pin(root)?;
    crate::upstream::locked_effectcraft_commit(&crate::scan::read_text(&root.join("rust/Cargo.lock"))?, &pin)
}

/// Write the Swift facade into `out_dir`.
pub(crate) fn generate_swift(out_dir: &Path) -> Result<(), String> {
    run_codegen(&["swift"], out_dir)
}

/// Write the command reference into `out_dir`, from the surface file in this tree. The registry runs
/// this generator after the one that writes that file, so a generate that pulls a new surface renders
/// the reference from what it just wrote; a `--check` compares the reference against the surface the
/// tree holds, and a surface that is itself stale fails the surface generator's own check.
pub(crate) fn generate_docs(out_dir: &Path) -> Result<(), String> {
    let surface = crate::repository_root()?.join(SURFACE_FILE);
    run_codegen(&["docs", &surface.to_string_lossy()], out_dir)
}

/// Write the Kotlin facade into `out_dir`.
pub(crate) fn generate_kotlin(out_dir: &Path) -> Result<(), String> {
    run_codegen(&["kotlin"], out_dir)
}

/// Build the codegen and run it with `args` and the output folder.
fn run_codegen(args: &[&str], out_dir: &Path) -> Result<(), String> {
    let binary = build_codegen(&crate::repository_root()?.join("rust"))?;
    let output = Command::new(&binary).args(args).arg(out_dir).output().map_err(|error| format!("cannot run {}: {error}", binary.display()))?;
    if output.status.success() { Ok(()) } else { Err(format!("{CODEGEN} exited with {}: {}", output.status, String::from_utf8_lossy(&output.stderr).trim())) }
}

/// The surface file's name, inside [`OUTPUT_DIR`] and inside any folder the codegen's `surface` command is
/// asked to write into.
pub(crate) const SURFACE_FILE_NAME: &str = "api-surface.json";

/// The surface file inside [`OUTPUT_DIR`].
pub(crate) const SURFACE_FILE: &str = "rust/effects/craftbridge-effects-codegen/generated/api-surface.json";

/// What the codegen's `diff` found between two surfaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SurfaceDiff {
    /// An app written against the old surface can fail to build or run against the new one.
    pub(crate) breaking: bool,
    /// The codegen's listing of the commands, keys and tools added, removed and changed.
    pub(crate) report: String,
}

/// The codegen's exit code for a breaking change (`craftbridge-effects-codegen diff`): 0 is a compatible
/// difference, and any other code is the tool failing.
const BREAKING: i32 = 2;

/// Read the codegen's judgement of `diff <old> <new>` from how it ended.
fn judged(code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> Result<SurfaceDiff, String> {
    let report = String::from_utf8_lossy(stdout).trim_end().to_string();
    match code {
        Some(0) => Ok(SurfaceDiff { breaking: false, report }),
        Some(BREAKING) => Ok(SurfaceDiff { breaking: true, report }),
        other => Err(format!("{CODEGEN} diff ended with {other:?}: {}", String::from_utf8_lossy(stderr).trim())),
    }
}

/// Run the codegen's `diff`, built from the workspace at `root`, on two surface files: what changed, and
/// whether it breaks an app.
pub(crate) fn diff(root: &Path, old: &Path, new: &Path) -> Result<SurfaceDiff, String> {
    let binary = build_codegen(&root.join("rust"))?;
    let output = Command::new(&binary).arg("diff").arg(old).arg(new).output().map_err(|error| format!("cannot run {}: {error}", binary.display()))?;
    judged(output.status.code(), &output.stdout, &output.stderr)
}

/// Build the codegen and return the path of its binary, as Cargo reports it.
fn build_codegen(rust_dir: &Path) -> Result<PathBuf, String> {
    let artifacts = crate::compiled::build(rust_dir, &[CODEGEN])?;
    crate::compiled::executable_of(&artifacts, CODEGEN).ok_or_else(|| format!("cargo built {CODEGEN} but reported no binary for it"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diff_is_breaking_only_when_the_codegen_says_so() {
        assert_eq!(judged(Some(0), b"+ command a\n", b"").map(|found| found.breaking), Ok(false));
        let breaking = judged(Some(2), b"- command a\n", b"").unwrap();
        assert!(breaking.breaking && breaking.report == "- command a");
        assert!(judged(Some(1), b"", b"no such file").is_err_and(|error| error.contains("no such file")), "a tool that failed judged nothing");
        assert!(judged(None, b"", b"").is_err(), "nor one a signal ended");
    }

    #[test]
    fn the_surface_file_is_where_the_generator_writes_it() {
        assert_eq!(SURFACE_FILE, format!("{OUTPUT_DIR}/{SURFACE_FILE_NAME}"));
    }

    #[test]
    fn a_surface_that_lost_a_command_is_a_breaking_change_to_the_real_codegen() {
        let root = crate::repo_root();
        let smaller: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join(SURFACE_FILE)).unwrap()).unwrap();
        // The command lost is one no mark keeps from apps, which is what the schema's defaults leave it: the
        // loss of a command the confined engine or the plugin refuses breaks no app, so whichever command a
        // pin happens to list must not decide what this test expects.
        let mut surface = smaller.clone();
        let commands = surface.get_mut("commands").and_then(serde_json::Value::as_array_mut).expect("the surface lists its commands");
        commands.push(serde_json::json!({"id": "probe.lost", "label": "Lost", "params": "{}", "journal": false, "keys": []}));
        let scratch = crate::scratch::Scratch::new("surface-diff");
        scratch.write("old.json", serde_json::to_string(&surface).unwrap());
        scratch.write("new.json", serde_json::to_string(&smaller).unwrap());
        let found = diff(&root, &scratch.0.join("old.json"), &scratch.0.join("new.json")).unwrap();
        assert!(found.breaking && found.report.contains("probe.lost"), "the loss of a command an app can call breaks it: {}", found.report);
        let same = diff(&root, &scratch.0.join("old.json"), &scratch.0.join("old.json")).unwrap();
        assert!(!same.breaking && same.report.contains("0 added, 0 removed, 0 changed"), "{}", same.report);
    }

    #[test]
    fn the_binary_is_the_codegens_own_executable() {
        let messages = [
            r#"{"reason":"compiler-artifact","target":{"name":"serde_json","kind":["lib"]},"executable":null}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"craftbridge-effects-codegen","kind":["bin"]},"executable":"/t/debug/craftbridge-effects-codegen"}"#,
            r#"{"reason":"build-finished","success":true}"#,
        ]
        .join("\n");
        let artifacts = crate::compiled::read_messages(messages.as_bytes()).unwrap();
        assert_eq!(crate::compiled::executable_of(&artifacts, CODEGEN), Some(PathBuf::from("/t/debug/craftbridge-effects-codegen")));
    }

    #[test]
    fn a_build_that_reports_no_codegen_binary_has_none() {
        let other_binary = r#"{"reason":"compiler-artifact","target":{"name":"xtask","kind":["bin"]},"executable":"/t/debug/xtask"}"#;
        let codegen_test =
            r#"{"reason":"compiler-artifact","target":{"name":"craftbridge-effects-codegen","kind":["test"]},"executable":"/t/debug/deps/codegen-1"}"#;
        let artifacts = crate::compiled::read_messages(format!("{other_binary}\n{codegen_test}\n").as_bytes()).unwrap();
        assert_eq!(crate::compiled::executable_of(&artifacts, CODEGEN), None);
    }

    #[test]
    fn the_version_constants_live_in_the_plugins_own_generated_folder() {
        assert_eq!(VERSION_OUTPUT_DIR, "rust/effects/craftbridge-effects/src/generated");
    }

    #[test]
    fn the_surface_lives_in_the_codegens_own_generated_folder() {
        assert_eq!(OUTPUT_DIR, format!("rust/effects/{CODEGEN}/generated"));
    }

    #[test]
    fn the_facades_live_in_their_modules_generated_folders() {
        assert!(SWIFT_OUTPUT_DIR.ends_with("/Generated") && SWIFT_OUTPUT_DIR.starts_with("ios/sdk/"), "{SWIFT_OUTPUT_DIR}");
        assert!(
            KOTLIN_OUTPUT_DIR.starts_with("android/sdk/effects/src/generated/kotlin/") && KOTLIN_OUTPUT_DIR.ends_with("/craftbridge/effects"),
            "{KOTLIN_OUTPUT_DIR}"
        );
    }
}
