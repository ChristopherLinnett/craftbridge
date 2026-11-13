//! Rule 10 — the gate's own machinery holds one copy of each configuration.
//!
//! The gate's machinery (`tools/ci.sh`, `tools/env.sh`, `tools/hooks/`, `rust/xtask/`,
//! `rust/.cargo/config.toml`, `android/sdk/settings.gradle.kts`, `android/sdk/gradle.properties`,
//! `android/sdk/build-logic/`, and the one copy of each configuration listed below) decides what the gate
//! checks, so it cannot guard itself: a change to one of those files is a gate change, which its pull
//! request names and its review checks for weakened rules. What a check can enforce is the other half:
//! that each configuration stays the only copy. A tool reads its configuration from the folder of what it
//! checks and the folders above it, so a second copy somewhere in the tree is read for the code below it
//! and not beside others, and the gate would then run two rule sets while printing the name of one. Every
//! configuration the gate's tools read therefore sits at exactly one path here; a second copy of any of
//! them, a Cargo configuration anywhere but `rust/.cargo/config.toml`, and a tool configuration the gate
//! keeps none of ([`REFUSED`]: shellcheck's rc files, the `.ignore` and nested `.gitignore` files jscpd
//! honours, and jscpd's ignore markers) fail the gate. Names are matched the way the machine's file systems
//! match them, and on this machine that is more than ASCII case: besides comparing the names ignoring
//! case, the check asks the file system (see `scan::reads_as`) whether opening a listed name in a file's
//! folder reaches that file, because a case-insensitive disk folds more letters than case changes
//! touch — on this volume the Kelvin sign opens under `k` and a long s under `s` — so `clippy.toml`
//! opened through a copy named `CLIPPY.toml`, or `rustfmt.toml` through one named with a long s, is
//! that copy's contents, not the gate's.

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
/// the workspace and merges the licence exceptions of a `deny.exceptions.toml` (or `.deny.exceptions.toml`,
/// also in a `.cargo` folder) it finds from there up, which would widen the one allowlist without it; ktlint
/// merges every `.editorconfig` from a Kotlin file up to the root's, so the root's is its configuration and
/// a nested one rewrites it for the code below; and the licence rule of AGENTS §8 is only one rule while
/// `licences.toml` is one file.
pub(crate) const CONFIGURATIONS: &[(&str, &[&str])] = &[
    (CLIPPY_CONFIGURATION, CLIPPY_CONFIGURATION_NAMES),
    ("rust/rustfmt.toml", &["rustfmt.toml", ".rustfmt.toml"]),
    ("rust/deny.toml", &["deny.toml", "deny.exceptions.toml", ".deny.exceptions.toml"]),
    ("ios/sdk/.swift-format", &[".swift-format"]),
    ("tools/licences.toml", &["licences.toml"]),
    (".editorconfig", &[".editorconfig"]),
];

/// Every file and folder of the gate's machinery (AGENTS.md §10) besides the configurations above: the
/// list the docs test holds both rule paragraphs to, so neither can drop a path the other keeps. The
/// check itself needs no list of them: a change to any is reviewed by hand.
#[cfg(test)]
const MACHINERY: &[&str] = &[
    "tools/ci.sh",
    "tools/env.sh",
    "tools/hooks/",
    "rust/xtask/",
    CARGO_CONFIGURATION,
    "android/sdk/settings.gradle.kts",
    GRADLE_PROPERTIES,
    "android/sdk/build-logic/",
];

/// Tool configurations the gate keeps no copy of, refused wherever they sit, with what each would do:
/// shellcheck reads `.shellcheckrc` or `shellcheckrc` from a script's folder up as file-wide directives
/// (the shell stage also runs it with `--norc`); jscpd skips what an `.ignore` file lists, at any depth.
const REFUSED: &[(&str, &str)] = &[
    (".shellcheckrc", "shellcheck reads it as directives for every script below it, so it can switch the shell stage's checks off"),
    ("shellcheckrc", "shellcheck reads it as directives for every script below it, so it can switch the shell stage's checks off"),
    (".ignore", "jscpd skips what it lists, so it can hide a copied block from the duplicates stage"),
];

/// jscpd honours a `.gitignore` at any depth too, and the duplicates stage runs it with `--no-gitignore`
/// as well; the repository needs one at the root, and the plan kit keeps its own under `plan/`, which
/// jscpd never reads.
const GITIGNORE: &str = ".gitignore";

/// The folders the duplicates stage reads, where a jscpd ignore marker would hide a block from it.
const CODE_TREES: &[&str] = &["rust", "ios", "android", "conformance", "tools", "docs", ".github"];

/// jscpd's marker pair (its name, a colon, then `ignore-start` and `ignore-end`), which switches the check
/// off between them. Spelt in two halves so this file does not carry it.
const JSCPD_MARKER: &str = concat!("jscpd", ":ignore");

/// Cargo merges a configuration from every `.cargo` folder from the folder it runs in up to the root,
/// so exactly one may reach a gate command: the one in `rust/.cargo`, which is also the only one the
/// `lints` check reads. A `.cargo/config.toml` at the root or above `rust/` reaches every gate command
/// too, and one inside `rust/` replaces the read one for crates below it. Cargo reads the file under
/// either name, the extensionless one first when both exist.
const CARGO_FOLDER: &str = ".cargo";
pub(crate) const CARGO_CONFIGURATION_FOLDER: &str = "rust/.cargo";
pub(crate) const CARGO_CONFIGURATION_NAMES: &[&str] = &["config.toml", "config"];
/// The one Cargo configuration: Cargo reads the extensionless `config` instead when both exist, so a
/// `config` beside it would replace the listed file, unseen.
pub(crate) const CARGO_CONFIGURATION: &str = "rust/.cargo/config.toml";

/// The build's Gradle properties, and the one copy of them. Gradle reads a
/// `gradle.properties` in a project's own folder as that project's properties, so any other copy under
/// `android/sdk` would set one module apart from the build every gate check runs as; the build reads its
/// own at this path alone. `build-logic` is exempt: it is an included build with properties of its own,
/// and this is a gate change — like everything it holds — when those move.
pub(crate) const GRADLE_PROPERTIES: &str = "android/sdk/gradle.properties";
const GRADLE_PROPERTIES_NAME: &str = "gradle.properties";

/// Is `name` a configuration file's name under some listed configuration? Returns the configuration's
/// own path, so a message can name the copy against the one.
fn listed_configuration(name: &str) -> Option<&'static str> {
    CONFIGURATIONS.iter().find(|(_, names)| names.iter().any(|file| name.eq_ignore_ascii_case(file))).map(|(path, _)| *path)
}

/// A copy of `gradle.properties` apart from the build's own, if this one is: under `android/sdk`, not in
/// the exempt `build-logic`, and not the listed path itself. A misspelling of the listed path says so —
/// a case-insensitive disk hands Gradle that file where it opens the plain name.
fn second_gradle_properties(shown: &str, folders: &[String]) -> Option<String> {
    if folders.first().is_some_and(|first| first.eq_ignore_ascii_case("android"))
        && folders.get(1).is_some_and(|second| second.eq_ignore_ascii_case("sdk"))
        && !folders.get(2).is_some_and(|third| third.eq_ignore_ascii_case("build-logic"))
        && shown != GRADLE_PROPERTIES
    {
        return Some(if folders.len() == 2 {
            format!(
                "{shown}: spell it {GRADLE_PROPERTIES_NAME}: a case-insensitive disk hands Gradle this file where it opens the plain name, and the build's one properties file is {GRADLE_PROPERTIES}"
            )
        } else {
            format!(
                "{shown}: a second `gradle.properties`: Gradle reads one in a project's own folder as that project's properties, so it sets one module apart from the build every gate check runs as; the build's properties are {GRADLE_PROPERTIES} alone"
            )
        });
    }
    None
}

/// One file of the tree against the list: is it a second copy of a listed configuration, or a Cargo
/// configuration outside `rust/.cargo/`, or a second `gradle.properties` under `android/sdk`?
/// `shown` is the path as the gate prints it.
pub(crate) fn second_copy(shown: &str, name: &str, folders: &[String]) -> Option<String> {
    if let Some(configuration) = listed_configuration(name) {
        return (shown != configuration).then(|| second_copy_message(shown, configuration));
    }
    if folders.last().is_some_and(|folder| folder.eq_ignore_ascii_case(CARGO_FOLDER))
        && CARGO_CONFIGURATION_NAMES.iter().any(|file| name.eq_ignore_ascii_case(file))
    {
        if shown == CARGO_CONFIGURATION {
            return None;
        }
        return Some(if shown.starts_with(&format!("{CARGO_CONFIGURATION_FOLDER}/")) {
            format!(
                "{shown}: a second Cargo configuration beside `{CARGO_CONFIGURATION}`: Cargo reads this one instead when both exist, so the listed file would stop being read"
            )
        } else {
            cargo_outside_message(shown)
        });
    }
    if name.eq_ignore_ascii_case(GRADLE_PROPERTIES_NAME) {
        return second_gradle_properties(shown, folders);
    }
    if let Some((_, does)) = REFUSED.iter().find(|(refused, _)| name.eq_ignore_ascii_case(refused)) {
        return Some(format!("{shown}: {does}; the gate keeps no such file"));
    }
    if name.eq_ignore_ascii_case(GITIGNORE) && shown != GITIGNORE && !folders.first().is_some_and(|first| first == "plan") {
        return Some(format!(
            "{shown}: a nested .gitignore: jscpd skips what it lists, so it can hide a copied block from the duplicates stage; ignore patterns live in the root .gitignore"
        ));
    }
    None
}

/// The lines of a code file that carry jscpd's ignore marker, which switches the duplicates check off
/// between a start and an end.
fn jscpd_markers(text: &str) -> Vec<usize> {
    text.lines().enumerate().filter(|(_, line)| line.contains(JSCPD_MARKER)).map(|(index, _)| index + 1).collect()
}

fn second_copy_message(shown: &str, configuration: &str) -> String {
    format!(
        "{shown}: a second copy of `{configuration}`: a tool reads its configuration nearest what it checks, so this copy would rule the code below it and split the gate across rule sets"
    )
}

fn cargo_outside_message(shown: &str) -> String {
    format!(
        "{shown}: a Cargo configuration outside `{CARGO_CONFIGURATION_FOLDER}`: Cargo merges every one above the folder it runs in, so this could restack lints and commands under a gate command"
    )
}

/// The same question the file system answers better than a name comparison can: does opening a listed
/// configuration's name in this file's folder reach this file, though its stored spelling differs?
/// That is what a tool does — it opens the name it wants — and a case-insensitive disk answers with any
/// spelling it folds to the name, not only one of a different case. Only spellings a name comparison
/// already missed come through here; the listed file itself never reports against its own path.
fn second_spelling(file: &scan::FoundFile, shown: &str) -> Option<String> {
    let dir = file.path.parent()?;
    let name = file.path.file_name().and_then(|name| name.to_str())?;
    for (configuration, names) in CONFIGURATIONS {
        for expected in names.iter().copied().filter(|expected| *expected != name) {
            if scan::reads_as(dir, &file.path, expected) {
                return Some(second_copy_message(shown, configuration));
            }
        }
    }
    if file.components.last().is_some_and(|folder| folder.eq_ignore_ascii_case(CARGO_FOLDER)) {
        for expected in CARGO_CONFIGURATION_NAMES.iter().copied().filter(|expected| !expected.eq_ignore_ascii_case(name)) {
            if scan::reads_as(dir, &file.path, expected) {
                return Some(if shown.starts_with(&format!("{CARGO_CONFIGURATION_FOLDER}/")) {
                    format!(
                        "{shown}: spell it `{expected}`: a case-insensitive disk hands Cargo this file where it opens that name, so it rules `rust` below it as the folder's own Cargo configuration"
                    )
                } else {
                    cargo_outside_message(shown)
                });
            }
        }
    }
    if GRADLE_PROPERTIES_NAME != name && scan::reads_as(dir, &file.path, GRADLE_PROPERTIES_NAME) {
        return second_gradle_properties(shown, &file.components);
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
        match second_copy(&shown, name, &file.components) {
            Some(error) => errors.push(error),
            None => errors.extend(second_spelling(&file, &shown)),
        }
        if file.components.first().is_some_and(|tree| CODE_TREES.contains(&tree.as_str()))
            && !scan::looks_binary(&file.path)
            && let Ok(text) = std::fs::read_to_string(&file.path)
        {
            for line in jscpd_markers(&text) {
                errors.push(format!("{shown}:{line}: a jscpd ignore marker hides the code after it from the duplicates stage"));
            }
        }
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
            // cargo-deny merges these into the licence configuration, widening the one allowlist.
            ("rust/deny.exceptions.toml", "exceptions = []\n"),
            ("rust/core/.deny.exceptions.toml", "exceptions = []\n"),
            ("rust/.cargo/deny.exceptions.toml", "exceptions = []\n"),
            // ktlint merges every .editorconfig up to the root's.
            ("rust/core/craftbridge-core/.editorconfig", "[*.kt]\nktlint_standard = disabled\n"),
            ("ios/sdk/.EditorConfig", "[*]\n"),
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
        assert!(second_copy(".editorconfig", ".editorconfig", &[]).is_none());
        // Cargo reads the extensionless name in place of the listed one, so it is a second copy there.
        let beside = second_copy("rust/.cargo/config", "config", &["rust".into(), ".cargo".into()]);
        assert!(beside.as_deref().is_some_and(|e| e.contains("a second Cargo configuration beside `rust/.cargo/config.toml`")), "{beside:?}");
    }

    #[test]
    fn a_tool_configuration_the_gate_keeps_none_of_is_refused_wherever_it_sits() {
        let scratch = crate::scratch::Scratch::new("machinery-refused");
        scratch.write(".gitignore", "/target/\n");
        scratch.write("plan/.gitignore", "node_modules/\n");
        assert!(check(&scratch.0).unwrap().is_empty(), "the root's and the plan kit's .gitignore are the repository's own");
        let plants = ["tools/.shellcheckrc", "tools/sub/shellcheckrc", "conformance/dup/.ignore", "conformance/dup/.gitignore", "rust/core/.GitIgnore"];
        for plant in plants {
            scratch.write(plant, "disable=all\n");
        }
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), plants.len(), "{errors:?}");
        for plant in plants {
            assert!(errors.iter().any(|e| e.starts_with(&format!("{plant}: "))), "{plant} missing from {errors:?}");
        }
    }

    #[test]
    fn a_jscpd_ignore_marker_in_the_code_trees_is_refused() {
        let scratch = crate::scratch::Scratch::new("machinery-jscpd");
        let marker = format!("// {JSCPD_MARKER}-start\nfn copied() {{}}\n// {JSCPD_MARKER}-end\n");
        scratch.write("rust/core/x/src/plant.rs", &marker);
        scratch.write("conformance/plant.sh", format!("# {JSCPD_MARKER}-start\n"));
        scratch.write("plan/notes.md", &marker);
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("rust/core/x/src/plant.rs:1: a jscpd ignore marker")), "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("rust/core/x/src/plant.rs:3: ")), "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("conformance/plant.sh:1: ")), "plan/ is outside the duplicates stage: {errors:?}");
    }

    /// The build reads one properties file, `android/sdk/gradle.properties`; a copy in a
    /// module's folder would become that module's own properties, and a misspelling of the listed path is
    /// read where the plain name is opened. `build-logic` is an included build with properties of its own.
    #[test]
    fn a_second_gradle_properties_under_android_sdk_is_refused_naming_the_file() {
        let scratch = crate::scratch::Scratch::new("machinery-gradle-properties");
        scratch.write("android/sdk/gradle.properties", "org.gradle.jvmargs=-Xmx2g\n");
        scratch.write("android/sdk/build-logic/gradle.properties", "kotlin.code.style=official\n");
        assert!(check(&scratch.0).unwrap().is_empty(), "the listed path and the included build's own file are not second copies");
        scratch.write("android/sdk/core/gradle.properties", "org.gradle.jvmargs=-Xmx16g\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("android/sdk/core/gradle.properties: a second `gradle.properties`"), "{errors:?}");
        assert!(errors[0].contains(GRADLE_PROPERTIES), "the refusal names the build's own: {errors:?}");
        std::fs::remove_file(scratch.0.join("android/sdk/core/gradle.properties")).unwrap();
        // A case spelling of the listed path: on a case-insensitive disk Gradle opens this file where
        // it opens `gradle.properties` — there it cannot even coexist with the plain name, so the test
        // sets the listed file aside first — and on a case-sensitive one nothing reads it. Either way
        // it must be refused and spelled exactly.
        std::fs::rename(scratch.0.join("android/sdk/gradle.properties"), scratch.0.join("listed.bak")).unwrap();
        scratch.write("android/sdk/Gradle.Properties", "org.gradle.jvmargs=-Xmx16g\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("android/sdk/Gradle.Properties: spell it gradle.properties"), "{errors:?}");
        std::fs::remove_file(scratch.0.join("android/sdk/Gradle.Properties")).unwrap();
        std::fs::rename(scratch.0.join("listed.bak"), scratch.0.join("android/sdk/gradle.properties")).unwrap();
        // Decided scope: under `android/sdk` only — a sample app beside it is another build's business.
        scratch.write("android/samples/app/gradle.properties", "org.gradle.jvmargs=-Xmx2g\n");
        assert!(check(&scratch.0).unwrap().is_empty(), "{:?}", check(&scratch.0).unwrap());
    }

    /// A configuration whose name is spelled with a letter a case-insensitive volume folds — the long s
    /// for `s` — is the configuration to every tool that opens the plain name, so it is a second copy.
    /// Where the volume does not fold, the same plant is an unrelated file, and must stay unseen; the
    /// probe asks the file system which machine this is rather than assume either.
    #[test]
    fn a_spelling_the_disk_opens_under_a_listed_name_is_a_second_copy() {
        let copy = crate::scratch::rust_tree_copy("machinery-fold");
        let plant = copy.0.join("rust/core/craftbridge-core").join("ru\u{017F}tfmt.toml");
        std::fs::write(&plant, "max_width = 1\n").unwrap();
        let folded = std::fs::canonicalize(copy.0.join("rust/core/craftbridge-core/rustfmt.toml")).ok() == std::fs::canonicalize(&plant).ok();
        let errors = check(&copy.0).unwrap();
        if folded {
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(errors[0].contains("a second copy of `rust/rustfmt.toml`"), "{errors:?}");
        } else {
            assert!(errors.is_empty(), "{errors:?}");
        }
    }

    /// AGENTS.md §10 and plan §15 name the machinery for people in one paragraph each; these lists are
    /// what the gate holds them to. A path added here and not there is one list split in two, so each
    /// paragraph must name every machinery path and every listed configuration, and every file the gate
    /// refuses outright.
    #[test]
    fn agents_and_the_plan_name_every_machinery_path() {
        for (document, opening) in [("AGENTS.md", "- The gate's own machinery"), ("plan/EFFECTCRAFT-PLUGIN-PORT-SCOPING.md", "**Gate machinery.**")] {
            let text = std::fs::read_to_string(crate::repo_root().join(document)).unwrap();
            let paragraph = text.lines().find(|line| line.starts_with(opening)).unwrap_or_else(|| panic!("{document} has no paragraph opening {opening:?}"));
            let named = MACHINERY.iter().copied().chain(CONFIGURATIONS.iter().map(|(path, _)| *path)).chain(REFUSED.iter().map(|(name, _)| *name));
            for path in named {
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
