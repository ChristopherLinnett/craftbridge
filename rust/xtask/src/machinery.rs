//! Rule 10 — the gate's own machinery holds one copy of each configuration.
//!
//! The gate's machinery (the files and folders `MACHINERY` lists, which the docs test holds AGENTS.md §10
//! and the plan to, and the one copy of each configuration listed below) decides what the gate checks, so
//! it cannot guard itself: a change to one of those files is a gate change, which its pull request names
//! and its review checks for weakened rules. What a check can enforce is the other half:
//! that each configuration stays the only copy. A tool reads its configuration from the folder of what it
//! checks and the folders above it, so a second copy somewhere in the tree is read for the code below it
//! and not beside others, and the gate would then run two rule sets while printing the name of one. Every
//! configuration the gate's tools read therefore sits at exactly one path here; a second copy of any of
//! them, a Cargo configuration anywhere but `rust/.cargo/config.toml`, and a tool configuration the gate
//! keeps none of ([`REFUSED`]: shellcheck's rc files, the `.ignore` and nested `.gitignore` files jscpd
//! honours, jscpd's ignore markers, and SourceKit-LSP's `.sourcekit-lsp` folder), and a second
//! `gradle.properties` under `android/sdk`, fail the gate. Names are compared exactly: `rules: hygiene`
//! refuses every other spelling of each name here (a `CLIPPY.toml`, or a `rustfmt.toml` spelt with a long
//! s, which a case-insensitive disk opens under the plain name), so a copy can only sit here under its own
//! name.

use crate::scan;
use std::path::Path;

/// The one clippy configuration, and the file names clippy reads it from (`.clippy.toml` is the name it
/// read in older versions; a file under either name elsewhere is still a second rule set on a disk that
/// matches names without case).
pub(crate) const CLIPPY_CONFIGURATION: &str = "rust/clippy.toml";
pub(crate) const CLIPPY_CONFIGURATION_NAMES: &[&str] = &["clippy.toml", ".clippy.toml"];

/// cargo-deny's configuration, whose exceptions files widen it rather than replace it.
const DENY_CONFIGURATION: &str = "rust/deny.toml";

/// Each configuration the gate's tools read: its one path, and the file names a tool reads it under.
/// rustfmt reads `rustfmt.toml` (or `.rustfmt.toml`) from the crate folder and above; `swift format`
/// reads `.swift-format` from the formatted file's folder and above; cargo-deny reads `deny.toml` beside
/// the workspace and merges the licence exceptions of a `deny.exceptions.toml` (or `.deny.exceptions.toml`,
/// also in a `.cargo` folder) it finds from there up, which would widen the one allowlist without it; ktlint
/// merges every `.editorconfig` from a Kotlin file up to the root's, so the root's is its configuration and
/// a nested one rewrites it for the code below; and the licence rule of AGENTS §8 is only one rule while
/// `licences.toml` is one file. rustup reads the toolchain pin from the nearest folder that holds one, from
/// the folder a command runs in up, under `rust-toolchain.toml` or the older `rust-toolchain` (the one it
/// reads when both sit in one folder), so a second copy switches which clippy and rustfmt the gate
/// commands get; and every Gradle version — ktlint and Spotless included — is once in the
/// build's own catalog (AGENTS §5), so a second `libs.versions.toml` anywhere is a second source of the
/// versions the Kotlin checks run under, `build-logic` included.
pub(crate) const CONFIGURATIONS: &[(&str, &[&str])] = &[
    (CLIPPY_CONFIGURATION, CLIPPY_CONFIGURATION_NAMES),
    ("rust/rustfmt.toml", &["rustfmt.toml", ".rustfmt.toml"]),
    (DENY_CONFIGURATION, &["deny.toml", "deny.exceptions.toml", ".deny.exceptions.toml"]),
    (crate::swift_format::CONFIGURATION, &[".swift-format"]),
    ("ios/sdk/.swiftlint.yml", &[".swiftlint.yml"]),
    ("tools/licences.toml", &["licences.toml"]),
    (".editorconfig", &[".editorconfig"]),
    ("rust/rust-toolchain.toml", &["rust-toolchain.toml", "rust-toolchain"]),
    ("android/sdk/gradle/libs.versions.toml", &["libs.versions.toml"]),
];

/// The gate's lint library (AGENTS.md §3), which `cargo dylint` builds and runs over the workspace: a
/// package of its own beside the workspace, since it compiles against the compiler's internals.
pub(crate) const LINT_LIBRARY: &str = "rust/never-crash-lints/";

/// The library's own toolchain pin, the nightly its compiler internals are written against: rustup reads
/// it only for a command run in the library's folder, which holds no workspace code, so it switches no
/// tool the gate runs over the workspace, and it is the one other `rust-toolchain.toml` the tree holds.
fn lint_library_toolchain() -> String {
    format!("{LINT_LIBRARY}rust-toolchain.toml")
}

/// The library's name as dylint names its build, `lib<name>@<toolchain>`: the package name with `_`.
pub(crate) fn lint_library_name() -> String {
    crate::layers::library_name(LINT_LIBRARY.trim_end_matches('/').rsplit('/').next().unwrap_or_default())
}

/// Every file and folder of the gate's machinery (AGENTS.md §10) besides the configurations above: the
/// list the docs test holds both rule paragraphs to, so neither can drop a path the other keeps. The
/// check itself needs no list of them: a change to any is reviewed by hand.
#[cfg(test)]
const MACHINERY: &[&str] = &[
    "tools/ci.sh",
    "tools/env.sh",
    "tools/refused-switches.tsv",
    "tools/oracle.sh",
    "tools/ios-device-test.sh",
    "tools/device-tests.tsv",
    // The app the runner wraps a Swift package's test bundle in to run its Swift Testing suite on the phone.
    "tools/ios-test-host.swift",
    // The guide check's Swift reader: which lines of the Swift the samples' projects compile compile to
    // nothing is its answer, read with the compiler's own parser, and nothing else builds it.
    "tools/guide-swift-lines.swift",
    // The builds of the native library: the XCFramework the Swift stages build and link against, and the
    // Android libraries the Android stage packages from `jniLibs`.
    "tools/build-xcframework.sh",
    "tools/build-android.sh",
    ".github/workflows/ci.yml",
    "rust/xtask/",
    "rust/effects/craftbridge-effects-conformance/",
    // The runner stops the processes it starts through it, and xtask's tests rerun one of their own in a
    // changed environment through it.
    "rust/craftbridge-process/",
    LINT_LIBRARY,
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

/// SourceKit-LSP's folder of workspace configuration: the server merges its `config.json` over the options
/// `cargo xtask swift-lint` starts it with, so it could hand the Swift stage another platform's build, other
/// compiler flags or another build system's answers. Refused with whatever it holds, wherever it sits.
const SOURCEKIT_LSP_FOLDER: &str = ".sourcekit-lsp";

/// jscpd honours a `.gitignore` at any depth too, and the duplicates stage runs it with `--no-gitignore`
/// as well; the repository needs one at the root, and the plan kit keeps its own at `plan/.gitignore`,
/// in a folder jscpd never reads.
const GITIGNORE: &str = ".gitignore";
const PLAN_GITIGNORE: &str = "plan/.gitignore";

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
    CONFIGURATIONS.iter().find(|(_, names)| names.contains(&name)).map(|(path, _)| *path)
}

/// A copy of `gradle.properties` apart from the build's own, if this one is: under `android/sdk`, not in
/// the exempt `build-logic`, and not the listed path itself.
fn second_gradle_properties(shown: &str, folders: &[String]) -> Option<String> {
    (folders.first().is_some_and(|first| first == "android")
        && folders.get(1).is_some_and(|second| second == "sdk")
        && !folders.get(2).is_some_and(|third| third == "build-logic")
        && shown != GRADLE_PROPERTIES)
        .then(|| {
            format!(
                "{shown}: a second `gradle.properties`: Gradle reads one in a project's own folder as that project's properties, so it sets one module apart from the build every gate check runs as; the build's properties are {GRADLE_PROPERTIES} alone"
            )
        })
}

/// One file of the tree against the list: is it a second copy of a listed configuration, or a Cargo
/// configuration outside `rust/.cargo/`, or a second `gradle.properties` under `android/sdk`?
/// `shown` is the path as the gate prints it.
pub(crate) fn second_copy(shown: &str, name: &str, folders: &[String]) -> Option<String> {
    if let Some(configuration) = listed_configuration(name) {
        return (shown != configuration && shown != lint_library_toolchain()).then(|| second_copy_message(shown, configuration));
    }
    if folders.last().is_some_and(|folder| folder == CARGO_FOLDER) && CARGO_CONFIGURATION_NAMES.contains(&name) {
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
    if name == GRADLE_PROPERTIES_NAME {
        return second_gradle_properties(shown, folders);
    }
    if folders.iter().any(|folder| folder == SOURCEKIT_LSP_FOLDER) {
        return Some(format!(
            "{shown}: SourceKit-LSP merges a `{SOURCEKIT_LSP_FOLDER}` folder's configuration over the options the Swift stage starts it with, so it could change the build the stage reads; the gate keeps no such folder"
        ));
    }
    if let Some((_, does)) = REFUSED.iter().find(|(refused, _)| name == *refused) {
        return Some(format!("{shown}: {does}; the gate keeps no such file"));
    }
    if name == GITIGNORE && shown != GITIGNORE && shown != PLAN_GITIGNORE {
        return Some(format!(
            "{shown}: a nested .gitignore: jscpd skips what it lists, so it can hide a copied block from the duplicates stage; ignore patterns live in the root .gitignore"
        ));
    }
    None
}

/// The lines of a file that carry jscpd's ignore marker, which switches the duplicates check off between a
/// start and an end. Read as bytes: jscpd finds the marker in any file it reads, whatever else the file's
/// bytes hold, so a search that first decoded the file could be blinded by one byte.
fn jscpd_markers(bytes: &[u8]) -> Vec<usize> {
    bytes
        .split(|byte| *byte == b'\n')
        .enumerate()
        .filter(|(_, line)| line.windows(JSCPD_MARKER.len()).any(|window| window == JSCPD_MARKER.as_bytes()))
        .map(|(index, _)| index + 1)
        .collect()
}

fn second_copy_message(shown: &str, configuration: &str) -> String {
    if configuration == DENY_CONFIGURATION && shown.rsplit('/').next().is_some_and(|name| name != "deny.toml") {
        return format!(
            "{shown}: a licence-exceptions file beside `{configuration}`: cargo-deny merges it into the one allowlist, so it would widen the licences AGENTS.md §8 allows"
        );
    }
    format!(
        "{shown}: a second copy of `{configuration}`: a tool reads its configuration nearest what it checks, so this copy would rule the code below it and split the gate across rule sets"
    )
}

fn cargo_outside_message(shown: &str) -> String {
    format!(
        "{shown}: a Cargo configuration outside `{CARGO_CONFIGURATION_FOLDER}`: Cargo merges every one above the folder it runs in, so this could restack lints and commands under a gate command"
    )
}

/// Run the check over the whole tree: build output, caches and anything that holds nothing git would
/// add are the walk's to skip, as for every other check — the ignored `release/` folder a staging run
/// leaves its release in, a tool's cache below a platform folder, a folder a checkout's own
/// `.git/info/exclude` names, at whatever depth and by whichever file git ignores it (AGENTS.md §1). A
/// configuration there governs no code the repository holds, and no code that governs is read from it.
/// A file forced past the ignore rules is tracked, and its copy is refused here as ever; a git that
/// cannot answer from inside a worktree fails the check, which then reads nothing as tracked.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let mut errors = Vec::new();
    for file in scan::walk(root, "")? {
        let Some(name) = file.path.file_name().and_then(|name| name.to_str()) else { continue };
        let shown = crate::report::shown(root, &file.path);
        if let Some(error) = second_copy(&shown, name, &file.components) {
            errors.push(error);
        }
        if file.components.first().is_some_and(|tree| crate::files::CODE_TREES.contains(&tree.as_str())) {
            let bytes = std::fs::read(&file.path).map_err(|error| format!("{shown}: {error}"))?;
            for line in jscpd_markers(&bytes) {
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

    /// A staging run of `tools/release.sh` leaves its release in the repository's ignored `release/<version>`,
    /// and that release is the `ios/sdk` tree — the listed configurations `.swift-format` and `.swiftlint.yml`
    /// included. Those copies govern no code the repository holds: a name git ignores is no part of the
    /// repository (AGENTS.md §1), so the gate reads none of it and a release staged does not break the gate.
    #[test]
    fn a_configuration_inside_an_ignored_release_folder_is_no_second_copy() {
        let scratch = crate::scratch::Scratch::new("machinery-release");
        let git = |args: &[&str]| crate::git::output(&scratch.0, args).unwrap();
        git(&["init", "-q"]);
        scratch.write(".gitignore", "/release/\n");
        scratch.write("ios/sdk/.swift-format", "{{}}\n");
        scratch.write("ios/sdk/Package.swift", "// swift-tools-version: 6.0\n");
        scratch.write("release/0.1.0/ChristopherLinnett/SwiftEffects/.swift-format", "{{}}\n");
        scratch.write("release/0.1.0/ChristopherLinnett/SwiftEffects/.swiftlint.yml", "disable_rules\n");
        scratch.write("release/0.1.0/maven/core-0.1.0.aar", "\u{0}binary\n");
        assert!(check(&scratch.0).unwrap().is_empty(), "a staged release is no part of the repository");
        // A file forced past the ignore rules is the repository's again, and its copy is refused as ever,
        // while the ignored file beside it stays no part of the repository: the exemption is git's answer
        // for each file, not the check's forgetting to look.
        git(&["add", "-f", "release/0.1.0/ChristopherLinnett/SwiftEffects/.swift-format"]);
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("release/0.1.0/ChristopherLinnett/SwiftEffects/.swift-format: a second copy of `ios/sdk/.swift-format`"), "{errors:?}");
    }

    /// What git ignores is no part of the repository at any depth, by whichever rule ignores it: a
    /// folder the root's `.gitignore` names below the root, a folder whose own `.gitignore` ignores all it
    /// holds (itself included), and a folder only the checkout's `.git/info/exclude` names. A copy git
    /// would add is refused wherever it sits: one already tracked by an ordinary add, and one forced past
    /// the ignore rules, while the ignored file beside it stays no part of the repository.
    #[test]
    fn a_copy_git_ignores_at_any_depth_is_no_second_copy_and_one_it_would_add_is() {
        let scratch = crate::scratch::Scratch::new("machinery-depth");
        let git = |args: &[&str]| crate::git::output(&scratch.0, args).unwrap();
        git(&["init", "-q"]);
        scratch.write(".gitignore", "/ios/sdk/scratch/\n");
        scratch.write("ios/sdk/.swift-format", "{{}}\n");
        scratch.write("ios/sdk/scratch/.swift-format", "{{}}\n");
        scratch.write("ios/sdk/scratch/.swiftlint.yml", "disable_rules\n");
        scratch.write("rust/cache/.gitignore", "*\n");
        scratch.write("rust/cache/rustfmt.toml", "edition = \"2024\"\n");
        scratch.write(".git/info/exclude", "/android/local/\n");
        scratch.write("android/local/.editorconfig", "root = true\n");
        scratch.write("ios/sdk/Sources/Probe/.swift-format", "{{}}\n");
        // Each ignored folder sits beside files git would add, as in a real checkout, so no top-level
        // folder holds only what git ignores.
        scratch.write("rust/Cargo.toml", "[workspace]\n");
        scratch.write("android/sdk/settings.gradle.kts", "// settings\n");
        // A copy git already tracks through an ordinary add is refused at its depth too.
        scratch.write("docs/readme.md", "x\n");
        scratch.write("docs/notes/.clippy.toml", "allow-unwrap-in-tests = true\n");
        git(&["add", "docs/notes/.clippy.toml"]);
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|error| error.starts_with("ios/sdk/Sources/Probe/.swift-format: a second copy of `ios/sdk/.swift-format`")), "{errors:?}");
        assert!(errors.iter().any(|error| error.starts_with("docs/notes/.clippy.toml: a second copy of `rust/clippy.toml`")), "{errors:?}");
        git(&["add", "-f", "ios/sdk/scratch/.swift-format"]);
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors.iter().any(|error| error.starts_with("ios/sdk/scratch/.swift-format: a second copy of")), "{errors:?}");
        assert!(!errors.iter().any(|error| error.starts_with("ios/sdk/scratch/.swiftlint.yml")), "{errors:?}");
    }

    /// A checkout whose git cannot say what it ignores — one whose `.git` names a git folder that is gone,
    /// and one whose index is no index — fails the check in git's own words: reading every copy as tracked
    /// behind it would refuse the ignored release's copies today, and pass an ignored copy the next rule
    /// should never have read.
    #[test]
    fn a_checkout_whose_git_cannot_answer_fails_the_check() {
        let dangling = crate::scratch::Scratch::new("machinery-no-git");
        dangling.write(".git", "gitdir: /nonexistent/craftbridge-no-such-git-folder\n");
        let unindexed = crate::scratch::Scratch::new("machinery-broken-checkout");
        crate::git::output(&unindexed.0, &["init", "-q"]).unwrap();
        unindexed.write(".git/index", "no index here");
        for broken in [&dangling, &unindexed] {
            broken.write(".gitignore", "/release/\n");
            broken.write("ios/sdk/.swift-format", "{{}}\n");
            broken.write("release/0.1.0/ChristopherLinnett/SwiftEffects/.swift-format", "{{}}\n");
            let failure = check(&broken.0).unwrap_err();
            assert_eq!(failure, crate::git::said(crate::git::command(&broken.0), crate::git::TRACKED_OR_ADDABLE), "the failure is git's own words");
        }
    }

    /// Git is asked about a file by the path it lists it by, which is no message's path: on a Unix disk a
    /// `\` is part of a folder's name, so a copy git would add below such a folder is refused, and a copy
    /// below one whose own `.gitignore` ignores it all stays no part of the repository.
    #[cfg(unix)]
    #[test]
    fn a_copy_below_a_folder_whose_name_holds_a_backslash_is_judged_by_its_own_path() {
        let scratch = crate::scratch::Scratch::new("machinery-backslash");
        crate::git::output(&scratch.0, &["init", "-q"]).unwrap();
        scratch.write("kept\\here/.swift-format", "{{}}\n");
        scratch.write("dropped\\here/.gitignore", "*\n");
        scratch.write("dropped\\here/.swift-format", "{{}}\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("a second copy of `ios/sdk/.swift-format`") && errors[0].starts_with("kept"), "{errors:?}");
    }

    #[test]
    fn a_second_copy_of_any_listed_configuration_is_refused() {
        let copy = crate::scratch::rust_tree_copy("machinery");
        copy.mkdir("ios/sdk");
        copy.mkdir("tools");
        // One plant per listed configuration and per name a tool reads it under. Every other spelling of
        // these names is `rules: hygiene`'s to refuse.
        let plants = [
            ("rust/core/craftbridge-core/clippy.toml", "allow-unwrap-in-tests = true\n"),
            ("rust/core/craftbridge-core/.clippy.toml", "allow-unwrap-in-tests = true\n"),
            ("rust/core/craftbridge-core/rustfmt.toml", "max_width = 1\n"),
            ("ios/sdk/.rustfmt.toml", "hard_tabs = true\n"),
            ("rust/tools/deny.toml", "[licenses]\n"),
            ("ios/sdk/Sources/.swift-format", "{{}}\n"),
            ("tools/other/licences.toml", "[allow]\n"),
            // ktlint merges every .editorconfig up to the root's.
            ("rust/core/craftbridge-core/.editorconfig", "[*.kt]\nktlint_standard = disabled\n"),
            // rustup reads the toolchain pin under either name, from the folder it runs in and above, and
            // the older name in place of the listed file beside it.
            ("rust/core/craftbridge-core/rust-toolchain.toml", "[toolchain]\nchannel = \"nightly\"\n"),
            ("ios/sdk/rust-toolchain", "nightly\n"),
            ("rust/rust-toolchain", "nightly\n"),
            // The lint library's own pin is the one other `rust-toolchain.toml`; the older name beside it
            // would be read in its place.
            ("rust/never-crash-lints/rust-toolchain", "nightly\n"),
            // Every Gradle version is once in the build's own catalog, build-logic's build too.
            ("android/sdk/build-logic/gradle/libs.versions.toml", "[versions]\nktlint = \"0.0.1\"\n"),
            ("android/samples/app/gradle/libs.versions.toml", "[versions]\nktlint = \"0.0.1\"\n"),
        ];
        // cargo-deny merges these into the licence configuration, widening the one allowlist.
        let exceptions = [
            ("rust/deny.exceptions.toml", "exceptions = []\n"),
            ("rust/core/.deny.exceptions.toml", "exceptions = []\n"),
            ("rust/.cargo/deny.exceptions.toml", "exceptions = []\n"),
        ];
        for (path, text) in plants.iter().chain(&exceptions) {
            copy.write(path, text);
        }
        let errors = check(&copy.0).unwrap();
        assert_eq!(errors.len(), plants.len() + exceptions.len(), "{errors:?}");
        for (path, _) in plants {
            assert!(errors.iter().any(|e| e.starts_with(&format!("{path}: a second copy of"))), "{path} missing from {errors:?}");
        }
        for (path, _) in exceptions {
            assert!(
                errors.iter().any(|e| e.starts_with(&format!("{path}: a licence-exceptions file beside `rust/deny.toml`"))),
                "{path} missing from {errors:?}"
            );
        }
        for (path, _) in plants.iter().chain(&exceptions) {
            std::fs::remove_file(copy.0.join(path)).unwrap();
        }
        // The rust/ tree copy already carries rust/clippy.toml, rustfmt.toml and deny.toml at their
        // listed paths, and the lint library's own toolchain pin; with only those, the check is clean.
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
        let plants = [
            "tools/.shellcheckrc",
            "tools/sub/shellcheckrc",
            "conformance/dup/.ignore",
            "conformance/dup/.gitignore",
            "plan/sub/.gitignore",
            "ios/sdk/.sourcekit-lsp/config.json",
            "ios/.sourcekit-lsp/anything",
        ];
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
        // A byte that is not UTF-8, or a NUL, hides nothing: jscpd still reads the marker in such a file.
        scratch.write("conformance/latin1.sh", [b"# caf\xe9\n".as_slice(), format!("# {JSCPD_MARKER}-start\n").as_bytes()].concat());
        scratch.write("rust/core/x/src/nul.rs", [b"// \0\n".as_slice(), format!("// {JSCPD_MARKER}-start\n").as_bytes()].concat());
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 5, "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("rust/core/x/src/plant.rs:1: a jscpd ignore marker")), "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("rust/core/x/src/plant.rs:3: ")), "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("conformance/plant.sh:1: ")), "plan/ is outside the duplicates stage: {errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("conformance/latin1.sh:2: ")), "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("rust/core/x/src/nul.rs:2: ")), "{errors:?}");
    }

    /// The build reads one properties file, `android/sdk/gradle.properties`; a copy in a module's folder
    /// would become that module's own properties. `build-logic` is an included build with properties of its
    /// own.
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
        // Every other spelling of the name (`Gradle.Properties`, a long s) is refused by `rules: hygiene`.
        // Decided scope: under `android/sdk` only — a sample app beside it is another build's business.
        scratch.write("android/samples/app/gradle.properties", "org.gradle.jvmargs=-Xmx2g\n");
        assert!(check(&scratch.0).unwrap().is_empty(), "{:?}", check(&scratch.0).unwrap());
    }

    /// AGENTS.md §10 and plan §15 name the machinery for people in one paragraph each; these lists are
    /// what the gate holds them to. A path added on one side and not the other is one list split in two,
    /// so each paragraph's list names exactly the machinery paths and the listed configurations — a path
    /// only the paragraph names is one dropped here — and the paragraph every other name a tool reads a
    /// configuration under and every file the gate refuses outright. The list is read alone: the
    /// paragraph names some paths a second time in its rules, and a path dropped from the list would still
    /// be found there.
    #[test]
    fn agents_and_the_plan_name_every_machinery_path() {
        let documents = [
            ("AGENTS.md", "- The gate's own machinery", "The gate's own machinery —", "— decides what the gate checks"),
            ("plan/EFFECTCRAFT-PLUGIN-PORT-SCOPING.md", "**Gate machinery.**", "cannot guard itself:", ". A change to any"),
        ];
        for (document, opening, list_from, list_to) in documents {
            let text = std::fs::read_to_string(crate::repo_root().join(document)).unwrap();
            let paragraph = text.lines().find(|line| line.starts_with(opening)).unwrap_or_else(|| panic!("{document} has no paragraph opening {opening:?}"));
            let start = paragraph.find(list_from).unwrap_or_else(|| panic!("{document}: no {list_from:?}")) + list_from.len();
            let end = paragraph.get(start..).and_then(|rest| rest.find(list_to)).unwrap_or_else(|| panic!("{document}: no {list_to:?}")) + start;
            let list = paragraph.get(start..end).unwrap_or_default();
            let kept: Vec<&str> = MACHINERY.iter().copied().chain(CONFIGURATIONS.iter().map(|(path, _)| *path)).collect();
            for path in &kept {
                assert!(list.contains(&format!("`{path}`")), "{document}'s machinery list does not name `{path}`");
            }
            // Every name the list sets in code is one of those paths: the odd pieces between backticks.
            for named in list.split('`').skip(1).step_by(2) {
                assert!(kept.contains(&named), "{document}'s machinery list names `{named}`, which machinery.rs does not");
            }
            let other_names = CONFIGURATIONS.iter().flat_map(|(path, names)| names.iter().filter(move |name| !path.ends_with(*name)));
            for name in other_names.chain(REFUSED.iter().map(|(name, _)| name)).chain([&SOURCEKIT_LSP_FOLDER]) {
                assert!(paragraph.contains(&format!("`{name}`")), "{document}'s machinery paragraph does not name `{name}`");
            }
        }
    }

    /// The workspace's manifest and the lint library's own, read for the tests that compare what the
    /// workspace states with what the library, standing outside it, carries of the same.
    fn manifests() -> (toml::Value, toml::Value) {
        let read = |path: &str| -> toml::Value { toml::from_str(&std::fs::read_to_string(crate::repo_root().join(path)).unwrap()).unwrap() };
        (read("rust/Cargo.toml"), read(&format!("{LINT_LIBRARY}Cargo.toml")))
    }

    /// The library is no workspace member, so it cannot inherit the workspace's tables the members inherit;
    /// the gate holds its crate to the members' rules all the same (AGENTS.md §3), and the way it can is
    /// that the library's own manifest carries the tables as its own workspace's and inherits them the
    /// members' way. Each copy is one definition written out again, and this test is what keeps it so: the
    /// lint table whole, and each `[workspace.package]` key the library carries (its edition and licence,
    /// never the release version, which is the workspace's alone), must match; and the library's `[package]`
    /// writes no version of its own, so the release version sits in no second file whichever table asked.
    #[test]
    fn the_lint_library_carries_the_workspace_tables() {
        let (workspace, library) = manifests();
        let table = |document: &toml::Value, name: &str| document.get("workspace").and_then(|workspace| workspace.get(name)).cloned();
        let expected = table(&workspace, "lints");
        assert!(expected.is_some(), "the workspace states its lint table");
        assert_eq!(table(&library, "lints"), expected, "the lint library carries the workspace's lint table as its own workspace's");
        let carried = table(&library, "package").and_then(|package| package.as_table().cloned()).unwrap_or_default();
        for key in ["edition", "license"] {
            assert!(carried.contains_key(key), "the lint library carries the workspace's {key}");
        }
        assert!(!carried.contains_key("version"), "the release version is written only in rust/Cargo.toml");
        // A version can be carried under the library's own `[package]` too, and one held there is the
        // release version written in a second file just as much (AGENTS.md §11): neither table of the
        // library's manifest may write a version at all, so neither can hold the release one.
        let own = library.get("package").and_then(|package| package.as_table().cloned()).unwrap_or_default();
        assert!(
            !own.contains_key("version"),
            "the lint library's [package] must carry no version either: {LINT_LIBRARY}Cargo.toml writes one, and the release version is written only in rust/Cargo.toml"
        );
        let stated = table(&workspace, "package");
        for (key, value) in &carried {
            assert_eq!(stated.as_ref().and_then(|package| package.get(key)), Some(value), "the lint library's [workspace.package].{key}");
        }
    }

    #[test]
    fn a_file_named_after_a_configuration_but_outside_a_cargo_folder_is_just_a_file() {
        assert!(second_copy("rust/xtask/src/config.rs", "config.rs", &["rust".into(), "xtask".into(), "src".into()]).is_none());
        assert!(second_copy("docs/cargo.toml", "cargo.toml", &["docs".into()]).is_none());
    }
}
