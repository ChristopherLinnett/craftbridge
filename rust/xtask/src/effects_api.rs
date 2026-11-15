//! The generators behind `generated/api-surface.json` and the Swift facade: the codegen binary, built and
//! run by cargo.
//!
//! The codegen links the pinned EffectCraft and reads its command registry at run time, and xtask may not
//! depend on a host tool, so xtask asks cargo for the binary and runs it. The pin the surface records is
//! the commit Cargo builds for the pin `rust/Cargo.toml` writes, in full as `rust/Cargo.lock` records it,
//! read by the upstream rule's own readers: the rule takes the pin in full or abbreviated, and the
//! surface's schema wants at least eight digits, so the spelling never reaches the surface.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the surface lives: the folder of the crate that writes it.
pub(crate) const OUTPUT_DIR: &str = "rust/effects/craftbridge-effects-codegen/generated";

/// Where the Swift facade lives: the generated folder of the plugin's Swift target.
pub(crate) const SWIFT_OUTPUT_DIR: &str = "ios/sdk/Sources/CraftBridgeEffects/Generated";

/// The codegen's package, and the binary it builds.
const CODEGEN: &str = "craftbridge-effects-codegen";

/// Write the surface into `out_dir`.
pub(crate) fn generate(out_dir: &Path) -> Result<(), String> {
    let root = crate::repo_root();
    let manifest = crate::scan::read_text(&root.join("rust/Cargo.toml"))?;
    let pin = crate::upstream::pins_from_manifest(&manifest).map_err(|errors| errors.join("; "))?;
    let commit = crate::upstream::locked_effectcraft_commit(&crate::scan::read_text(&root.join("rust/Cargo.lock"))?, &pin)?;
    run_codegen(&["surface", "--pin", &commit], out_dir)
}

/// Write the Swift facade into `out_dir`.
pub(crate) fn generate_swift(out_dir: &Path) -> Result<(), String> {
    run_codegen(&["swift"], out_dir)
}

/// Build the codegen and run it with `args` and the output folder.
fn run_codegen(args: &[&str], out_dir: &Path) -> Result<(), String> {
    let binary = build_codegen(&crate::repo_root().join("rust"))?;
    let output = Command::new(&binary).args(args).arg(out_dir).output().map_err(|error| format!("cannot run {}: {error}", binary.display()))?;
    if output.status.success() { Ok(()) } else { Err(format!("{CODEGEN} exited with {}: {}", output.status, String::from_utf8_lossy(&output.stderr).trim())) }
}

/// Build the codegen and return the path of its binary, as Cargo reports it: Cargo, not a guess at its
/// target folder, says where the build wrote it.
fn build_codegen(rust_dir: &Path) -> Result<PathBuf, String> {
    let output = Command::new(env!("CARGO"))
        .current_dir(rust_dir)
        .args(["build", "--locked", "--package", CODEGEN, "--message-format=json-render-diagnostics"])
        .output()
        .map_err(|error| format!("cannot run `{} build`: {error}", env!("CARGO")))?;
    if !output.status.success() {
        return Err(format!("building {CODEGEN} exited with {}: {}", output.status, String::from_utf8_lossy(&output.stderr).trim()));
    }
    let artifacts = crate::compiled::read_messages(output.stdout.as_slice())?;
    executable_of(&artifacts).ok_or_else(|| format!("cargo built {CODEGEN} but reported no binary for it"))
}

/// The codegen's binary among the units cargo reported building.
fn executable_of(artifacts: &[crate::compiled::Artifact]) -> Option<PathBuf> {
    artifacts
        .iter()
        .find(|artifact| artifact.target_name == CODEGEN && artifact.target_kinds.iter().any(|kind| kind == "bin"))
        .and_then(|artifact| artifact.executable.as_deref())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_binary_is_the_codegens_own_executable() {
        let messages = [
            r#"{"reason":"compiler-artifact","target":{"name":"serde_json","kind":["lib"]},"executable":null}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"craftbridge-effects-codegen","kind":["bin"]},"executable":"/t/debug/craftbridge-effects-codegen"}"#,
            r#"{"reason":"build-finished","success":true}"#,
        ]
        .join("\n");
        let artifacts = crate::compiled::read_messages(messages.as_bytes()).unwrap();
        assert_eq!(executable_of(&artifacts), Some(PathBuf::from("/t/debug/craftbridge-effects-codegen")));
    }

    #[test]
    fn a_build_that_reports_no_codegen_binary_has_none() {
        let other_binary = r#"{"reason":"compiler-artifact","target":{"name":"xtask","kind":["bin"]},"executable":"/t/debug/xtask"}"#;
        let codegen_test =
            r#"{"reason":"compiler-artifact","target":{"name":"craftbridge-effects-codegen","kind":["test"]},"executable":"/t/debug/deps/codegen-1"}"#;
        let artifacts = crate::compiled::read_messages(format!("{other_binary}\n{codegen_test}\n").as_bytes()).unwrap();
        assert_eq!(executable_of(&artifacts), None);
    }

    #[test]
    fn the_surface_lives_in_the_codegens_own_generated_folder() {
        assert_eq!(OUTPUT_DIR, format!("rust/effects/{CODEGEN}/generated"));
    }
}
