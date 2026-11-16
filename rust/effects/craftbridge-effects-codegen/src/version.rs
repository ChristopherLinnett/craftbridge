//! The engine's version and pin as the constants the plugin's `info` reports.
//!
//! The pin comes from the caller, which reads it with Cargo's own readers from the manifest and lock it is
//! written in; the version is the linked engine's own, the one the surface records too. This module only
//! checks that each is safe to spell as a string literal and renders the file, so a regeneration is byte
//! for byte the file that was committed.

use crate::surface::{ENGINE_VERSION, check_pin};

/// The subcommand that writes the constants: the one name `run` answers to and the file's notice gives.
pub(crate) const COMMAND: &str = "version";

/// The file the constants are written to.
pub(crate) const FILE: &str = "version.rs";

/// The file's text for `pin`, the commit the workspace pin names, and the version of the engine this
/// binary links, which is EffectCraft at that commit.
pub(crate) fn build(pin: &str) -> Result<String, String> {
    render(pin, ENGINE_VERSION)
}

/// The file's text for `pin` and `engine_version`: kept apart from [`build`] so a test can hand it a
/// version of its own.
fn render(pin: &str, engine_version: &str) -> Result<String, String> {
    check_pin(pin)?;
    check_version(engine_version)?;
    let notice = crate::generated_notice(COMMAND);
    Ok(format!(
        "{notice}\
         // Change the generator, which takes the commit rust/Cargo.toml pins and the version of the EffectCraft it links at that commit.\n\
         \n\
         /// The version of EffectCraft's crates at the pin, which is what the engine's own command line reports as its version.\n\
         pub(crate) const ENGINE_VERSION: &str = \"{engine_version}\";\n\
         \n\
         /// The commit of EffectCraft this plugin is built from, in full, so the same pin reads the same however `rust/Cargo.toml` spells it.\n\
         pub(crate) const ENGINE_PIN: &str = \"{pin}\";\n"
    ))
}

/// A version as Cargo writes one: a digit first, then only the characters a semantic version holds, which
/// are also the characters a string literal can carry unescaped.
fn check_version(version: &str) -> Result<(), String> {
    let starts_with_digit = version.chars().next().is_some_and(|first| first.is_ascii_digit());
    if starts_with_digit && version.chars().all(|each| each.is_ascii_alphanumeric() || matches!(each, '.' | '-' | '+')) {
        Ok(())
    } else {
        Err(format!("`{version}` is no version: a semantic version starts with a digit and holds only letters, digits, `.`, `-` and `+`"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::TEST_PIN;

    #[test]
    fn the_file_holds_the_two_constants_under_the_generated_marker() {
        let text = render(TEST_PIN, "1.2.3-rc.1+b5").unwrap();
        assert!(text.starts_with("// @generated "), "{text}");
        assert!(text.contains("pub(crate) const ENGINE_VERSION: &str = \"1.2.3-rc.1+b5\";\n"), "{text}");
        assert!(text.contains(&format!("pub(crate) const ENGINE_PIN: &str = \"{TEST_PIN}\";\n")), "{text}");
        assert!(text.ends_with(";\n") && !text.ends_with("\n\n"), "{text}");
    }

    #[test]
    fn the_version_is_the_one_the_surface_records() {
        let surface = crate::surface::build(TEST_PIN).unwrap();
        let text = build(TEST_PIN).unwrap();
        assert!(text.contains(&format!("pub(crate) const ENGINE_VERSION: &str = \"{}\";\n", surface.engine_version)), "{text}");
        assert_eq!(surface.engine_version, effectcraft_engine::project::APP_VERSION);
    }

    #[test]
    fn a_pin_or_version_that_would_not_be_a_plain_literal_writes_nothing() {
        for pin in ["", "main", "0123ABCD", "0123abcd\"", &"a".repeat(41)] {
            assert!(build(pin).is_err(), "{pin:?}");
        }
        for version in ["", "v1.2.3", "1.2.3\"; pub fn x() {}", "1.2 3", "1.2.3\n"] {
            assert!(render(TEST_PIN, version).is_err(), "{version:?}");
        }
    }
}
