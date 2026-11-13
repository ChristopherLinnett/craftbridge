//! Rule 10 — the gate's own machinery holds one copy of each configuration.
//!
//! The gate's machinery (`tools/ci.sh`, `tools/env.sh`, `tools/hooks/`, `rust/xtask/`,
//! `rust/.cargo/config.toml`, `android/sdk/settings.gradle.kts`, `android/sdk/build-logic/`, and the one
//! copy of each configuration listed below) decides what the gate checks, so it cannot guard itself: a
//! change to one of those files is a gate change, which its pull request names and its review checks for
//! weakened rules. What a check can enforce is the other half: that each configuration stays the only
//! copy. A tool reads its configuration from the folder of what it checks and the folders above it, so a
//! second copy somewhere in the tree is read for the code below it and not beside others, and the gate
//! would then run two rule sets while printing the name of one. Every configuration the gate's tools read
//! therefore sits at exactly one path here; a second copy of any of them, and a Cargo configuration
//! anywhere but `rust/.cargo/`, fails the gate. The names are compared as file systems on this machine
//! compare them — case-insensitively — because `clippy.toml` opened through a copy named `CLIPPY.toml`
//! on a case-insensitive disk is that copy's contents, not the gate's.

use crate::scan;
use std::path::Path;

/// The one clippy configuration, and the file names clippy reads it from (`.clippy.toml` is the name it
/// read in older versions; a file under either name elsewhere is still a second rule set on a disk that
/// matches names without case).
pub(crate) const CLIPPY_CONFIGURATION: &str = "rust/clippy.toml";
pub(crate) const CLIPPY_CONFIGURATION_NAMES: &[&str] = &["clippy.toml", ".clippy.toml"];

/// Each configuration the gate's tools read: its one path, and the file names a tool reads it under.
/// rustfmt reads `rustfmt.toml` (or `.rustfmt.toml`) from the crate folder and above; `swift format`
/// reads `.swift-format` from the formatted file's folder and above; cargo-deny reads `deny.toml` beside
/// the workspace; and the licence rule of AGENTS §8 is only one rule while `licences.toml` is one file.
pub(crate) const CONFIGURATIONS: &[(&str, &[&str])] = &[
    (CLIPPY_CONFIGURATION, CLIPPY_CONFIGURATION_NAMES),
    ("rust/rustfmt.toml", &["rustfmt.toml", ".rustfmt.toml"]),
    ("rust/deny.toml", &["deny.toml"]),
    ("ios/sdk/.swift-format", &[".swift-format"]),
    ("tools/licences.toml", &["licences.toml"]),
];

/// Cargo merges a configuration from every `.cargo` folder from the folder it runs in up to the root,
/// so exactly one may reach a gate command: the one in `rust/.cargo`, which is also the only one the
/// `lints` check reads. A `.cargo/config.toml` at the root or above `rust/` reaches every gate command
/// too, and one inside `rust/` replaces the read one for crates below it. Cargo reads the file under
/// either name, the extensionless one first when both exist.
const CARGO_FOLDER: &str = ".cargo";
pub(crate) const CARGO_CONFIGURATION_FOLDER: &str = "rust/.cargo";
pub(crate) const CARGO_CONFIGURATION_NAMES: &[&str] = &["config.toml", "config"];

/// Is `name` a configuration file's name under some listed configuration? Returns the configuration's
/// own path, so a message can name the copy against the one.
fn listed_configuration(name: &str) -> Option<&'static str> {
    CONFIGURATIONS.iter().find(|(_, names)| names.iter().any(|file| name.eq_ignore_ascii_case(file))).map(|(path, _)| *path)
}

/// One file of the tree against the list: is it a second copy of a listed configuration, or a Cargo
/// configuration outside `rust/.cargo/`? `shown` is the path as the gate prints it.
pub(crate) fn second_copy(shown: &str, name: &str, folders: &[String]) -> Option<String> {
    if let Some(configuration) = listed_configuration(name) {
        return (shown != configuration).then(|| {
            format!("{shown}: a second copy of `{configuration}`: a tool reads its configuration nearest what it checks, so this copy would rule the code below it and split the gate across rule sets")
        });
    }
    if folders.last().is_some_and(|folder| folder.eq_ignore_ascii_case(CARGO_FOLDER))
        && CARGO_CONFIGURATION_NAMES.iter().any(|file| name.eq_ignore_ascii_case(file))
    {
        return (!shown.starts_with(&format!("{CARGO_CONFIGURATION_FOLDER}/"))).then(|| {
            format!("{shown}: a Cargo configuration outside `{CARGO_CONFIGURATION_FOLDER}`: Cargo merges every one above the folder it runs in, so this could restack lints and commands under a gate command")
        });
    }
    None
}

/// Run the check over the whole tree: build output and caches are the walker's to skip, as for every
/// other check; a configuration inside them is a tool's own copy, not the repository's.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    for file in scan::walk(root, "")? {
        let Some(name) = file.path.file_name().and_then(|name| name.to_str()) else { continue };
        let shown = crate::report::shown(root, &file.path);
        errors.extend(second_copy(&shown, name, &file.components));
    }
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real tree carries each configuration exactly once — the check must say so about the repository
    /// itself, walking every folder a second copy could hide in.
    #[test]
    fn the_repository_holds_one_copy_of_every_configuration() {
        let errors = check(&crate::repo_root()).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_second_copy_of_any_listed_configuration_is_refused() {
        let copy = crate::scratch::rust_tree_copy("machinery");
        copy.mkdir("ios/sdk");
        copy.mkdir("tools");
        // One plant per listed configuration, and the case spellings a case-insensitive file system
        // would open under the tool's own name — placed where no same-named file already sits, since
        // there a case variant would be that file, not a copy.
        let plants = [
            ("rust/core/craftbridge-core/clippy.toml", "allow-unwrap-in-tests = true\n"),
            ("rust/core/craftbridge-core/.CLIPPY.TOML", "allow-unwrap-in-tests = true\n"),
            ("rust/core/craftbridge-core/rustfmt.toml", "max_width = 1\n"),
            ("ios/sdk/RUSTFMT.TOML", "hard_tabs = true\n"),
            ("rust/tools/deny.toml", "[licenses]\n"),
            ("ios/sdk/Sources/.swift-format", "{{}}\n"),
            ("tools/other/licences.toml", "[allow]\n"),
            ("tools/LICENCES.toml", "[allow]\n"),
        ];
        for (path, text) in plants {
            copy.write(path, text);
        }
        let errors = check(&copy.0).unwrap();
        assert_eq!(errors.len(), plants.len(), "{errors:?}");
        for (path, _) in plants {
            assert!(errors.iter().any(|e| e.starts_with(&format!("{path}: a second copy of"))), "{path} missing from {errors:?}");
        }
        for (path, _) in plants {
            std::fs::remove_file(copy.0.join(path)).unwrap();
        }
        // The rust/ tree copy already carries rust/clippy.toml, rustfmt.toml and deny.toml at their
        // listed paths; with only those, the check is clean.
        assert!(check(&copy.0).unwrap().is_empty(), "the copies are what the errors were");
    }

    #[test]
    fn a_cargo_configuration_outside_rust_is_refused_wherever_it_sits() {
        let copy = crate::scratch::rust_tree_copy("machinery-cargo");
        copy.mkdir("ios/sdk");
        copy.write("ios/sdk/.cargo/config.toml", "[build]\nrustflags = []\n");
        copy.write(".cargo/config", "[alias]\nb = \"build\"\n");
        let errors = check(&copy.0).unwrap();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().all(|e| e.contains("a Cargo configuration outside `rust/.cargo`")), "{errors:?}");
        std::fs::remove_file(copy.0.join("ios/sdk/.cargo/config.toml")).unwrap();
        std::fs::remove_file(copy.0.join(".cargo/config")).unwrap();
        // The real copy at rust/.cargo/config.toml is the listed one, not a second.
        assert!(check(&copy.0).unwrap().is_empty(), "{:?}", check(&copy.0).unwrap());
    }

    #[test]
    fn a_listed_configuration_at_its_own_path_is_not_a_second_copy() {
        assert!(second_copy("rust/clippy.toml", "clippy.toml", &["rust".into()]).is_none());
        assert!(second_copy("ios/sdk/.swift-format", ".swift-format", &["ios".into(), "sdk".into()]).is_none());
        assert!(second_copy("tools/licences.toml", "licences.toml", &["tools".into()]).is_none());
        assert!(second_copy("rust/.cargo/config.toml", "config.toml", &["rust".into(), ".cargo".into()]).is_none());
        assert!(second_copy("rust/.cargo/config", "config", &["rust".into(), ".cargo".into()]).is_none());
    }

    /// AGENTS.md §10 and plan §15 name the machinery's configurations for people in one paragraph each;
    /// this list is what the gate holds them to. A path added here and not there is one list split in
    /// two, so each paragraph must name every path the list holds.
    #[test]
    fn agents_and_the_plan_name_every_listed_configuration() {
        let cargo = format!("{CARGO_CONFIGURATION_FOLDER}/{}", CARGO_CONFIGURATION_NAMES[0]);
        for (document, opening) in [("AGENTS.md", "- The gate's own machinery"), ("plan/EFFECTCRAFT-PLUGIN-PORT-SCOPING.md", "**Gate machinery.**")] {
            let text = std::fs::read_to_string(crate::repo_root().join(document)).unwrap();
            let paragraph = text.lines().find(|line| line.starts_with(opening)).unwrap_or_else(|| panic!("{document} has no paragraph opening {opening:?}"));
            for path in CONFIGURATIONS.iter().map(|(path, _)| *path).chain([cargo.as_str()]) {
                assert!(paragraph.contains(&format!("`{path}`")), "{document}'s machinery paragraph does not name `{path}`");
            }
        }
    }

    #[test]
    fn a_file_named_after_a_configuration_but_outside_a_cargo_folder_is_just_a_file() {
        assert!(second_copy("rust/xtask/src/config.rs", "config.rs", &["rust".into(), "xtask".into(), "src".into()]).is_none());
        assert!(second_copy("docs/cargo.toml", "cargo.toml", &["docs".into()]).is_none());
    }
}
