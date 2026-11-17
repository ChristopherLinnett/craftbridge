//! The pins: `cargo xtask pins` prints what `rust/rust-toolchain.toml` and the Gradle version catalog
//! name — the toolchain's targets and the catalog's Android pins — as `key=value` lines, and
//! `cargo xtask swift-builds` prints the Swift package's shipping builds — a configuration and an
//! `xcodebuild` destination, and the architectures to build it for, for every Apple slice the toolchain
//! file names — as `configuration<TAB>destination<TAB>architectures` lines. `tools/ci.sh` and
//! `tools/doctor.sh` read every pin through these commands and read no pin by text themselves, and the
//! XCFramework is built for the same slices (`crate::xcframework`).
//!
//! Both files are parsed with the `toml` crate, the parser rustup itself reads the toolchain file with, so
//! every form rustup and Gradle accept gives these commands the same values: basic and literal strings,
//! inline tables, arrays over any number of lines at any spacing. A text reader in a script would know one
//! spelling of each value and misread or stop on every other.
//!
//! This is the one reader of both files: of the toolchain file's targets and Apple slices — the rows below
//! say what the Swift checks read the sources as (`rust/xtask/src/swift_lint.rs`) and what the
//! `swift_build` stage compiles, so the list of slices the package ships for exists exactly once — and of
//! the catalog, whose pins the doctor checks and whose libraries and plugins the licence check reads
//! (`rust/xtask/src/licences.rs`).

use std::path::Path;
use std::process::ExitCode;

/// The file rustup reads to choose the toolchain inside `rust/`; its `targets` are what the native library
/// is built for.
pub(crate) const TOOLCHAIN: &str = "rust/rust-toolchain.toml";

/// The version catalog the Android build reads; the pins named below are the levels the doctor checks the
/// machine has installed.
pub(crate) const CATALOG: &str = "android/sdk/gradle/libs.versions.toml";

/// The catalog keys the doctor's checks read, and the gate beside them: the levels of the platform the
/// machine installs, which the update loop's newer-release step also leaves alone, there being no
/// repository of newer ones (`rust/xtask/src/versions.rs`).
pub(crate) const CATALOG_PINS: [&str; 3] = ["android-compileSdk", "android-buildTools", "jdk"];

/// The configurations the Swift package ships as: the Debug a developer builds, and the Release a library
/// is shipped in. An `#if !DEBUG` compiles code no Debug build sees, so both ship.
const SWIFT_CONFIGURATIONS: [&str; 2] = ["Debug", "Release"];

/// One Apple slice: the Rust target that names it, the architecture and environment its Swift target
/// triple is built from, the SDK `xcrun` names, how a message names it, and the destination `xcodebuild`
/// compiles it as.
pub(crate) struct Slice {
    pub(crate) target: &'static str,
    pub(crate) architecture: &'static str,
    pub(crate) environment: &'static str,
    pub(crate) sdk: &'static str,
    pub(crate) message: &'static str,
    pub(crate) destination: &'static str,
}

/// The Apple slices the native library is built for, as the Rust toolchain names them, and how the Swift
/// package is built for each. The Swift package is compiled only for the slices its binary target links,
/// so the slices the toolchain lists (`rust/rust-toolchain.toml`) are the destinations the gate reads and
/// builds, whatever processor the machine running it has. A slice of the toolchain's this table does not
/// know stops the stage rather than going unread or unbuilt.
const APPLE_SLICES: &[&Slice; 3] = &[
    &Slice { target: "aarch64-apple-ios", architecture: "arm64", environment: "", sdk: "iphoneos", message: "iOS device", destination: "generic/platform=iOS" },
    &Slice {
        target: "aarch64-apple-ios-sim",
        architecture: "arm64",
        environment: "-simulator",
        sdk: "iphonesimulator",
        message: "iOS Simulator",
        destination: "generic/platform=iOS Simulator",
    },
    &Slice {
        target: "x86_64-apple-ios",
        architecture: "x86_64",
        environment: "-simulator",
        sdk: "iphonesimulator",
        message: "Intel iOS Simulator",
        destination: "generic/platform=iOS Simulator",
    },
];

/// One pinned file, read and parsed: the one way this repository opens the two files above. A leading
/// byte-order mark is no part of the TOML, as rustup reads it.
fn toml_file(root: &Path, file: &str) -> Result<toml::Value, String> {
    let source = crate::scan::read_text(&root.join(file))?;
    toml::from_str::<toml::Value>(&source).map_err(|error| format!("{file}: is not valid TOML: {error}"))
}

/// The toolchain file's targets, in the file's order: a list of strings, the one form rustup takes (it
/// refuses a string, comma-separated or not, as "expected a sequence").
pub(crate) fn toolchain_targets(root: &Path) -> Result<Vec<String>, String> {
    let targets = toml_file(root, TOOLCHAIN)?
        .get("toolchain")
        .and_then(|toolchain| toolchain.get("targets"))
        .cloned()
        .ok_or_else(|| format!("{TOOLCHAIN}: names no targets for the build to compile for"))?;
    let toml::Value::Array(items) = targets else {
        return Err(format!("{TOOLCHAIN}: targets are not a list of strings, the one form rustup reads: {targets}"));
    };
    items.iter().map(|item| item.as_str().map(str::to_string).ok_or_else(|| format!("{TOOLCHAIN}: a target is not a string: {item}"))).collect()
}

/// The Apple targets the toolchain lists, each with its row of [`APPLE_SLICES`].
pub(crate) fn apple_slices(root: &Path) -> Result<Vec<&'static Slice>, String> {
    let targets: Vec<String> = toolchain_targets(root)?.into_iter().filter(|target| target.contains("-apple-")).collect();
    if targets.is_empty() {
        return Err(format!("{TOOLCHAIN}: lists no Apple target, so the Swift package ships for no destination the gate could read or build"));
    }
    targets
        .iter()
        .map(|target| {
            APPLE_SLICES.iter().find(|slice| slice.target == target).copied().ok_or_else(|| {
                format!("{TOOLCHAIN}: the native library is built for `{target}`, a slice the Swift checks and builds have no row for (rust/xtask/src/pins.rs)")
            })
        })
        .collect()
}

/// The version catalog, parsed, or `None` when the repository has no Android build to read yet.
pub(crate) fn catalog(root: &Path) -> Result<Option<toml::Value>, String> {
    match root.join(CATALOG).try_exists() {
        Ok(true) => toml_file(root, CATALOG).map(Some),
        Ok(false) => Ok(None),
        Err(error) => Err(format!("{CATALOG}: {error}")),
    }
}

/// One version from the catalog's `[versions]` table: a string, the one form that names a single level the
/// doctor can look for. Gradle refuses every value but a string or a table of rich-version constraints,
/// and a constraint names a range or a preference rather than one installed level.
fn catalog_pin(versions: &toml::value::Table, key: &str) -> Result<String, String> {
    match versions.get(key) {
        Some(toml::Value::String(pin)) => Ok(pin.clone()),
        Some(other) => Err(format!("{CATALOG}: {key} is not a version string the doctor can check: {other}")),
        None => Err(format!("{CATALOG}: names no {key}")),
    }
}

/// The pins as `key=value` lines: one `rust-target` for every target the toolchain file names, then one
/// line for every catalog pin. Scripts read these lines as data; the parsing of the files stays here.
fn pins(root: &Path) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for target in toolchain_targets(root)? {
        lines.push(format!("rust-target={target}"));
    }
    let versions = toml_file(root, CATALOG)?
        .get("versions")
        .and_then(toml::Value::as_table)
        .cloned()
        .ok_or_else(|| format!("{CATALOG}: names no [versions] table to read the pins from"))?;
    for pin in CATALOG_PINS {
        lines.push(format!("{pin}={}", catalog_pin(&versions, pin)?));
    }
    Ok(lines)
}

/// The Swift package's shipping builds: every configuration of every slice the toolchain names, one
/// `configuration<TAB>destination<TAB>architectures` line each, for the `swift_build` stage to build in
/// order. A generic destination builds every architecture of its platform — `generic/platform=iOS
/// Simulator` compiles the arm64 and the Intel simulator alike, whichever the native library holds — so
/// the line names the architectures of the slices the toolchain lists for that destination, which the
/// stage hands the build as `ARCHS`: the package is built for the slices its binary target links and no
/// other. Two slices that name one destination share its builds, and no build is listed twice.
fn swift_builds(root: &Path) -> Result<Vec<String>, String> {
    let slices = apple_slices(root)?;
    let mut lines: Vec<String> = Vec::new();
    for slice in &slices {
        let mut architectures: Vec<&str> = Vec::new();
        for same in slices.iter().filter(|same| same.destination == slice.destination) {
            if !architectures.contains(&same.architecture) {
                architectures.push(same.architecture);
            }
        }
        for configuration in SWIFT_CONFIGURATIONS {
            let line = format!("{configuration}\t{}\t{}", slice.destination, architectures.join(" "));
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
    }
    Ok(lines)
}

/// Print one list, or name on stderr why it could not be read.
fn print(lines: Result<Vec<String>, String>, command: &str) -> ExitCode {
    match lines {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(failure) => crate::report::failed(command, failure),
    }
}

/// `cargo xtask pins`: the pins the doctor and the gate read.
pub(crate) fn run(root: &Path) -> ExitCode {
    print(pins(root), "pins")
}

/// `cargo xtask swift-builds`: the Swift package's shipping builds.
pub(crate) fn builds(root: &Path) -> ExitCode {
    print(swift_builds(root), "swift-builds")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The toolchain file as rustup writes it after an update: basic strings, one line.
    const TOOLCHAIN_BASIC: &str = "[toolchain]\nchannel = \"1.99.1\"\ncomponents = [\"rustfmt\", \"clippy\"]\ntargets = [\"aarch64-apple-ios\", \"aarch64-apple-ios-sim\", \"aarch64-linux-android\", \"x86_64-linux-android\"]\n";

    /// The same toolchain file written the other legal ways: one inline table, literal and basic strings
    /// mixed, an array over several lines, comments, and spacing none of the old text readers knew.
    const TOOLCHAIN_OTHER_FORM: &str = "toolchain = {\n  channel = '1.99.1',\n  components = [ 'rustfmt', \"clippy\" ],\n  targets = [\n    'aarch64-apple-ios', # device\n    \"aarch64-apple-ios-sim\",\n    'aarch64-linux-android',\n    'x86_64-linux-android',\n  ]\n}\n";

    const CATALOG_BASIC: &str = "[versions]\nandroid-compileSdk = \"37\"\nandroid-buildTools = \"37.0.0\"\njdk = \"21\"\n";

    /// The same catalog written as one inline table of literal strings with odd spacing.
    const CATALOG_OTHER_FORM: &str = "versions = {\n  android-compileSdk = '37',\n  android-buildTools = \"37.0.0\", # the AGP's own pin\n  jdk = '21',\n}\n";

    /// The pins of two scratch trees, one written every way the tools read and one written canonically.
    fn scratch_with(toolchain: &str, catalog: &str, tag: &str) -> crate::scratch::Scratch {
        let scratch = crate::scratch::Scratch::new(tag);
        scratch.write(TOOLCHAIN, toolchain);
        scratch.write(CATALOG, catalog);
        scratch
    }

    #[test]
    fn the_pins_are_the_same_values_whatever_form_the_files_write_them_in() {
        let canonical = scratch_with(TOOLCHAIN_BASIC, CATALOG_BASIC, "pins-canonical");
        let other_form = scratch_with(TOOLCHAIN_OTHER_FORM, CATALOG_OTHER_FORM, "pins-other-form");
        assert_eq!(pins(&canonical.0).unwrap(), pins(&other_form.0).unwrap(), "literal strings, an inline table and a multi-line array give the same pins");
        assert_eq!(
            pins(&canonical.0).unwrap(),
            [
                "rust-target=aarch64-apple-ios",
                "rust-target=aarch64-apple-ios-sim",
                "rust-target=aarch64-linux-android",
                "rust-target=x86_64-linux-android",
                "android-compileSdk=37",
                "android-buildTools=37.0.0",
                "jdk=21",
            ]
        );
    }

    #[test]
    fn a_pin_broken_in_either_file_is_named_and_refused() {
        let no_targets = scratch_with("[toolchain]\nchannel = \"1.99.1\"\n", CATALOG_BASIC, "pins-no-targets");
        assert!(pins(&no_targets.0).unwrap_err().contains("names no targets"), "a toolchain file with no targets stops the reader");
        let not_toml = scratch_with("[toolchain\n", CATALOG_BASIC, "pins-not-toml");
        assert!(pins(&not_toml.0).unwrap_err().contains("is not valid TOML"), "a broken file is refused as the file it is");
        let missing_pin = scratch_with(TOOLCHAIN_BASIC, "[versions]\njdk = \"21\"\n", "pins-missing");
        let error = pins(&missing_pin.0).unwrap_err();
        assert!(error.contains("android-compileSdk") && error.contains(CATALOG), "the gap names the key and the file: {error}");
        let not_a_version = scratch_with(
            TOOLCHAIN_BASIC,
            "[versions]\nandroid-compileSdk = \"37\"\nandroid-buildTools = { require = \"37.0.0\" }\njdk = \"21\"\n",
            "pins-table",
        );
        assert!(pins(&not_a_version.0).unwrap_err().contains("android-buildTools"), "a version table is not a version the doctor can check");
        let a_number = scratch_with(TOOLCHAIN_BASIC, "[versions]\nandroid-compileSdk = \"37\"\nandroid-buildTools = \"37.0.0\"\njdk = 21\n", "pins-number");
        assert!(pins(&a_number.0).unwrap_err().contains("jdk"), "a number is no version Gradle reads, so it is no pin either");
    }

    #[test]
    fn targets_written_as_a_string_are_refused_as_rustup_refuses_them() {
        let scratch = scratch_with("[toolchain]\ntargets = 'aarch64-apple-ios, aarch64-apple-ios-sim'\n", CATALOG_BASIC, "pins-comma");
        assert!(toolchain_targets(&scratch.0).is_err_and(|error| error.contains("not a list of strings")), "rustup reads no string form");
    }

    #[test]
    fn the_swift_builds_are_every_configuration_of_every_slice_the_toolchain_names() {
        let canonical = scratch_with(TOOLCHAIN_BASIC, CATALOG_BASIC, "builds-canonical");
        assert_eq!(
            swift_builds(&canonical.0).unwrap(),
            [
                "Debug\tgeneric/platform=iOS\tarm64",
                "Release\tgeneric/platform=iOS\tarm64",
                "Debug\tgeneric/platform=iOS Simulator\tarm64",
                "Release\tgeneric/platform=iOS Simulator\tarm64"
            ]
        );
        // A slice added to the toolchain file joins the builds with no other edit anywhere: alone, it is built
        // on its own; beside the arm64 simulator, in the simulator builds the two share.
        let intel_alone = scratch_with("[toolchain]\ntargets = [\"x86_64-apple-ios\"]\n", CATALOG_BASIC, "builds-intel-alone");
        assert_eq!(
            swift_builds(&intel_alone.0).unwrap(),
            ["Debug\tgeneric/platform=iOS Simulator\tx86_64", "Release\tgeneric/platform=iOS Simulator\tx86_64"],
            "a slice the library is not built for is not built for: the Intel simulator alone names its own architecture"
        );
        let intel =
            scratch_with(&TOOLCHAIN_BASIC.replace("aarch64-apple-ios-sim", "aarch64-apple-ios-sim\", \"x86_64-apple-ios"), CATALOG_BASIC, "builds-intel");
        assert_eq!(
            swift_builds(&intel.0).unwrap(),
            [
                "Debug\tgeneric/platform=iOS\tarm64",
                "Release\tgeneric/platform=iOS\tarm64",
                "Debug\tgeneric/platform=iOS Simulator\tarm64 x86_64",
                "Release\tgeneric/platform=iOS Simulator\tarm64 x86_64"
            ],
            "one destination is built once, for every architecture the toolchain names for it"
        );
    }

    #[test]
    fn the_analysis_reads_the_slices_the_toolchain_builds_and_refuses_one_it_has_no_build_for() {
        let shipped: Vec<(&str, &str, &str)> =
            apple_slices(&crate::repo_root()).unwrap().into_iter().map(|slice| (slice.architecture, slice.environment, slice.sdk)).collect();
        assert_eq!(shipped, [("arm64", "", "iphoneos"), ("arm64", "-simulator", "iphonesimulator")], "the device and the simulator, both arm64");
        let scratch = crate::scratch::Scratch::new("apple-slices");
        scratch.write(TOOLCHAIN, "[toolchain]\ntargets = [\"x86_64-apple-ios\", \"aarch64-linux-android\"]\n");
        assert_eq!(
            apple_slices(&scratch.0).unwrap().into_iter().map(|slice| slice.architecture).collect::<Vec<_>>(),
            ["x86_64"],
            "an Intel simulator slice is read as one"
        );
        scratch.write(TOOLCHAIN, "[toolchain]\ntargets = [\"aarch64-apple-ios\", \"aarch64-apple-tvos\"]\n");
        assert!(apple_slices(&scratch.0).is_err_and(|error| error.contains("`aarch64-apple-tvos`")), "a slice with no build stops the stage");
        scratch.write(TOOLCHAIN, "[toolchain]\ntargets = [\"aarch64-linux-android\"]\n");
        assert!(apple_slices(&scratch.0).is_err(), "no Apple slice at all");
    }

    #[test]
    fn the_doctor_checks_the_sdk_of_every_slice_row() {
        // A missing SDK stops the stage with a message that sends the reader to tools/doctor.sh, so the
        // doctor must check every SDK any row of the table could build against: a row added without its
        // SDK in the doctor's loop would fail with a promise the doctor cannot keep.
        let doctor = std::fs::read_to_string(crate::repo_root().join("tools/doctor.sh")).unwrap();
        let mut sdks: Vec<&str> = APPLE_SLICES.iter().map(|slice| slice.sdk).collect();
        sdks.sort_unstable();
        sdks.dedup();
        for sdk in sdks {
            assert!(doctor.contains(&format!("\"{sdk}:")), "tools/doctor.sh checks no {sdk} SDK, which a row of APPLE_SLICES builds against");
        }
    }
}
