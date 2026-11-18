//! The pins: `cargo xtask pins` prints what `rust/rust-toolchain.toml` and the Gradle version catalog
//! name — the toolchain's targets and the catalog's Android pins — as `key=value` lines, and
//! `cargo xtask swift-builds` prints the Swift package's shipping builds — a configuration and an
//! `xcodebuild` destination, and the architectures to build it for, for every Apple slice the toolchain
//! file names — as `configuration<TAB>destination<TAB>architectures` lines, and `cargo xtask swift-suite`
//! the one of those the Swift Testing suite runs on, in the same shape. `cargo xtask swift-products`
//! prints the package's library products, one name per line, as its manifest lists them, which the
//! `swift_build` stage builds the way an app extension links them. `tools/ci.sh` and
//! `tools/doctor.sh` read every pin through these commands and read no pin by text themselves, and the
//! XCFramework is built for the same slices (`crate::xcframework`).
//!
//! Both files are parsed with the `toml` crate, the parser rustup itself reads the toolchain file with, so
//! every form rustup and Gradle accept gives these commands the same values: basic and literal strings,
//! inline tables, arrays over any number of lines at any spacing. A text reader in a script would know one
//! spelling of each value and misread or stop on every other.
//!
//! This is the one reader of both files: of the toolchain file's channel and targets, and its Apple slices — the rows below
//! say what the Swift checks read the sources as (`rust/xtask/src/swift_lint.rs`), what the `swift_build`
//! stage compiles, and on which of those destinations the Swift Testing suite runs, so the list of slices
//! the package ships for exists exactly once — and of
//! the catalog, whose pins the doctor checks and whose libraries and plugins the licence check reads
//! (`rust/xtask/src/licences.rs`).

use std::path::Path;
use std::process::ExitCode;

/// The file rustup reads to choose the toolchain inside `rust/`; its `targets` are what the native library
/// is built for.
pub(crate) const TOOLCHAIN: &str = "rust/rust-toolchain.toml";

/// The version catalog the Android build reads; the pins named below are what the scripts read of it.
pub(crate) const CATALOG: &str = "android/sdk/gradle/libs.versions.toml";

/// The catalog keys the scripts read: the levels of the platform the machine installs, which the doctor
/// checks, and the oldest Android API, the level `tools/env.sh` builds the native library for. None is a
/// release a repository holds newer ones of, so the update loop's newer-release step leaves them alone
/// (`rust/xtask/src/versions.rs`).
pub(crate) const CATALOG_PINS: [&str; 4] = ["android-compileSdk", "android-minSdk", "android-buildTools", "jdk"];

/// The configurations the Swift package ships as: the Debug a developer builds, and the Release a library
/// is shipped in. An `#if !DEBUG` compiles code no Debug build sees, so both ship.
const SWIFT_CONFIGURATIONS: [&str; 2] = ["Debug", "Release"];

/// The configuration the Swift Testing suite runs in: the one a developer and `swift test` run it in.
/// `swift_build` compiles the test targets in both configurations, so the Release build of every `#if`
/// branch is still compiled; running the suite twice a gate would exercise the same native library twice,
/// at the suite's whole cost again, to test the same code with the optimiser on.
const SWIFT_TEST_CONFIGURATION: &str = "Debug";

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

/// The channel the toolchain file names: the toolchain every build of the gate runs in (AGENTS.md §7).
/// Every cargo and rustc a tool starts names it in the variable rustup itself gives every command it
/// starts under the pin (`crate::pin`), so no crate's own toolchain file decides which toolchain builds
/// CraftBridge. A string is the only form a channel takes, in a `[toolchain]` table or an inline one.
pub(crate) fn toolchain_channel(root: &Path) -> Result<String, String> {
    toml_file(root, TOOLCHAIN)?
        .get("toolchain")
        .and_then(|toolchain| toolchain.get("channel"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("{TOOLCHAIN}: names no channel for the build to run in"))
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
        for configuration in SWIFT_CONFIGURATIONS {
            let line = build_line(configuration, slice, &slices);
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
    }
    Ok(lines)
}

/// One destination's build line, read from the table: the configuration, the destination the table gives
/// the slice, and the architectures of every slice that shares that destination, which a build takes as
/// `ARCHS` because a generic destination builds them all.
fn build_line(configuration: &str, slice: &Slice, slices: &[&Slice]) -> String {
    let mut architectures: Vec<&str> = Vec::new();
    for same in slices.iter().filter(|same| same.destination == slice.destination) {
        if !architectures.contains(&same.architecture) {
            architectures.push(same.architecture);
        }
    }
    format!("{configuration}\t{}\t{}", slice.destination, architectures.join(" "))
}

/// Is this slice a simulator's? Its row's environment, which its Swift target triple is built from, is
/// `-simulator`, and a device's row has none — the Intel simulator's Rust target, `x86_64-apple-ios`,
/// spells none either — so the table says which slices a suite may run on without a second list naming
/// them.
fn is_simulator(slice: &Slice) -> bool {
    slice.environment == "-simulator"
}

/// The build the Swift Testing suite runs on: the simulator build of [`swift_builds`], in the
/// configuration the suite runs in. The destination is the table's own, so the stage that runs the suite
/// reads its destination from the one list and holds none of its own; it makes that destination concrete
/// with a simulator the machine already has, which is the machine's to have and not the gate's to name. A
/// toolchain that lists no simulator slice leaves the suite with no destination to run on, which stops the
/// stage rather than letting it pass having run nothing: a phone is no destination for the gate, whose
/// corpus on a phone is a device acceptance run (plan §28).
fn swift_suite(root: &Path) -> Result<Vec<String>, String> {
    let slices = apple_slices(root)?;
    let mut lines: Vec<String> = Vec::new();
    for slice in &slices {
        if is_simulator(slice) {
            let line = build_line(SWIFT_TEST_CONFIGURATION, slice, &slices);
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
    }
    if lines.is_empty() {
        return Err(format!("{TOOLCHAIN}: names no simulator target, so the Swift Testing suite has no destination to run on (rust/xtask/src/pins.rs)"));
    }
    Ok(lines)
}

/// `cargo xtask pins`: the pins the doctor and the gate read.
pub(crate) fn run(root: &Path) -> ExitCode {
    crate::report::print_lines(pins(root), "pins")
}

/// `cargo xtask swift-builds`: the Swift package's shipping builds.
pub(crate) fn builds(root: &Path) -> ExitCode {
    crate::report::print_lines(swift_builds(root), "swift-builds")
}

/// `cargo xtask swift-suite`: the build the Swift Testing suite runs on.
pub(crate) fn suite(root: &Path) -> ExitCode {
    crate::report::print_lines(swift_suite(root), "swift-suite")
}

/// The Swift package's library products, in the manifest's order: what an app links, and what its own
/// extension may link too. `swift_build` builds each of them the way an extension links it, and names
/// them through this command so no stage holds a second list of what the package ships.
fn swift_products(root: &Path) -> Result<Vec<String>, String> {
    let package = crate::direction::dump_package(&root.join("ios").join("sdk"), crate::direction::FILE)?
        .ok_or_else(|| format!("{}: no Swift package to list products of", crate::direction::FILE))?;
    library_names(&package)
}

/// The kinds of product SwiftPM writes that are no library, so no extension links them.
const OTHER_PRODUCT_KINDS: &[&str] = &["executable", "plugin", "test", "macro", "snippet"];

/// The names of the library products of a dumped package: a library of any kind — automatic, static or
/// dynamic — ships as a library whoever links it, an extension included. SwiftPM writes a product's type
/// as a table tagged by its one kind, `{"library": ["automatic"]}` beside `{"executable": null}`. A product
/// whose type is written any other way — no kind, several, or one SwiftPM does not write — is refused by
/// name rather than skipped: a library this reader failed to recognise would be left out of the extension
/// builds with nothing to say so.
fn library_names(package: &serde_json::Value) -> Result<Vec<String>, String> {
    let file = crate::direction::FILE;
    let mut names = Vec::new();
    for product in package.get("products").and_then(serde_json::Value::as_array).into_iter().flatten() {
        let name = product.get("name").and_then(serde_json::Value::as_str).unwrap_or_default();
        let kind = product.get("type").and_then(serde_json::Value::as_object).filter(|kinds| kinds.len() == 1).and_then(|kinds| kinds.keys().next());
        match kind.map(String::as_str) {
            Some("library") if name.is_empty() => return Err(format!("{file}: a library product has no name to build")),
            Some("library") => names.push(name.to_string()),
            Some(other) if OTHER_PRODUCT_KINDS.contains(&other) => {}
            _ => {
                return Err(format!(
                    "{file}: product `{name}` has a type this reader does not know, so it cannot tell whether an extension links it: {}",
                    product.get("type").unwrap_or(&serde_json::Value::Null)
                ));
            }
        }
    }
    Ok(names)
}

/// `cargo xtask swift-products`: the Swift package's library products, as its manifest lists them.
pub(crate) fn products(root: &Path) -> ExitCode {
    crate::report::print_lines(swift_products(root), "swift-products")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The toolchain file as rustup writes it after an update: basic strings, one line.
    const TOOLCHAIN_BASIC: &str = "[toolchain]\nchannel = \"1.99.1\"\ncomponents = [\"rustfmt\", \"clippy\"]\ntargets = [\"aarch64-apple-ios\", \"aarch64-apple-ios-sim\", \"aarch64-linux-android\", \"x86_64-linux-android\"]\n";

    /// The same toolchain file written the other legal ways: one inline table, literal and basic strings
    /// mixed, an array over several lines, comments, and spacing none of the old text readers knew.
    const TOOLCHAIN_OTHER_FORM: &str = "toolchain = {\n  channel = '1.99.1',\n  components = [ 'rustfmt', \"clippy\" ],\n  targets = [\n    'aarch64-apple-ios', # device\n    \"aarch64-apple-ios-sim\",\n    'aarch64-linux-android',\n    'x86_64-linux-android',\n  ]\n}\n";

    const CATALOG_BASIC: &str = "[versions]\nandroid-compileSdk = \"37\"\nandroid-minSdk = \"29\"\nandroid-buildTools = \"37.0.0\"\njdk = \"21\"\n";

    /// The same catalog written as one inline table of literal strings with odd spacing.
    const CATALOG_OTHER_FORM: &str =
        "versions = {\n  android-compileSdk = '37',\n  android-minSdk = '29',\n  android-buildTools = \"37.0.0\", # the AGP's own pin\n  jdk = '21',\n}\n";

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
                "android-minSdk=29",
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
            "[versions]\nandroid-compileSdk = \"37\"\nandroid-minSdk = \"29\"\nandroid-buildTools = { require = \"37.0.0\" }\njdk = \"21\"\n",
            "pins-table",
        );
        assert!(pins(&not_a_version.0).unwrap_err().contains("android-buildTools"), "a version table is not a version the doctor can check");
        let a_number = scratch_with(
            TOOLCHAIN_BASIC,
            "[versions]\nandroid-compileSdk = \"37\"\nandroid-minSdk = \"29\"\nandroid-buildTools = \"37.0.0\"\njdk = 21\n",
            "pins-number",
        );
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
    fn the_suite_runs_on_the_simulator_build_the_table_lists() {
        let canonical = scratch_with(TOOLCHAIN_BASIC, CATALOG_BASIC, "suite-canonical");
        let suite = swift_suite(&canonical.0).unwrap();
        assert_eq!(suite, ["Debug\tgeneric/platform=iOS Simulator\tarm64"], "the device build runs no suite: the gate has no phone");
        for line in &suite {
            assert!(swift_builds(&canonical.0).unwrap().contains(line), "the suite runs on a build the table lists, never on one of its own: {line}");
        }
        // The Intel simulator shares the arm64 one's destination, so the one line names both its
        // architectures, exactly as the build line of that destination does.
        let intel =
            scratch_with(&TOOLCHAIN_BASIC.replace("aarch64-apple-ios-sim", "aarch64-apple-ios-sim\", \"x86_64-apple-ios"), CATALOG_BASIC, "suite-intel");
        assert_eq!(swift_suite(&intel.0).unwrap(), ["Debug\tgeneric/platform=iOS Simulator\tarm64 x86_64"]);
        // A toolchain of device slices alone leaves the suite with nowhere to run, which is refused rather
        // than a stage that passes having run nothing.
        let device_only = scratch_with("[toolchain]\ntargets = [\"aarch64-apple-ios\"]\n", CATALOG_BASIC, "suite-device-only");
        assert_eq!(
            swift_suite(&device_only.0).unwrap_err(),
            format!("{TOOLCHAIN}: names no simulator target, so the Swift Testing suite has no destination to run on (rust/xtask/src/pins.rs)")
        );
        assert_eq!(
            swift_suite(&crate::repo_root()).unwrap(),
            ["Debug\tgeneric/platform=iOS Simulator\tarm64"],
            "the tree's own toolchain runs the suite on the arm64 simulator, and on one destination only"
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

    /// The channel is read the same from every spelling rustup reads, and a file naming none fails with its
    /// name: it is the value `crate::pin` names to every cargo a tool starts, so a second reader of the
    /// file — or a silent default when the key is gone — is the one thing this cannot be.
    #[test]
    fn the_channel_is_named_however_the_file_writes_it() {
        assert_eq!(toolchain_channel(&scratch_with(TOOLCHAIN_BASIC, CATALOG_BASIC, "pins-channel").0).unwrap(), "1.99.1");
        assert_eq!(toolchain_channel(&scratch_with(TOOLCHAIN_OTHER_FORM, CATALOG_BASIC, "pins-channel-form").0).unwrap(), "1.99.1");
        let no_channel = scratch_with("[toolchain]\ntargets = [\"aarch64-apple-ios\"]\n", CATALOG_BASIC, "pins-channel-missing");
        assert_eq!(toolchain_channel(&no_channel.0).unwrap_err(), format!("{TOOLCHAIN}: names no channel for the build to run in"));
    }

    /// The shape `swift package dump-package` prints its products in: automatic, static and dynamic
    /// libraries, an executable, a plugin.
    const PRODUCTS: &str = r#"{"products": [
        {"name": "CraftBridge", "type": {"library": ["automatic"]}},
        {"name": "CraftBridgeStatic", "type": {"library": ["static"]}},
        {"name": "Tool", "type": {"executable": null}},
        {"name": "Plug", "type": {"plugin": null}},
        {"name": "CraftBridgeDynamic", "type": {"library": ["dynamic"]}}
    ]}"#;

    /// A library of any kind is a library whoever links it, so every kind reads as one; an executable and
    /// a plugin are no library.
    #[test]
    fn library_products_are_read_however_the_manifest_writes_them() {
        let package: serde_json::Value = serde_json::from_str(PRODUCTS).unwrap();
        assert_eq!(library_names(&package).unwrap(), ["CraftBridge", "CraftBridgeStatic", "CraftBridgeDynamic"]);
    }

    /// A product typed in a shape SwiftPM does not write — the plain word, no type, an empty table, two
    /// kinds in one, a kind of no name this reader knows, a known one in a case SwiftPM does not write — is refused by name, in these words, as is a
    /// library with no name: a skipped one would be a library the extension builds leave out.
    #[test]
    fn a_product_the_reader_cannot_place_is_refused_by_name() {
        let unknown = |name: &str, kind: &str| {
            format!(
                "{}: product `{name}` has a type this reader does not know, so it cannot tell whether an extension links it: {kind}",
                crate::direction::FILE
            )
        };
        let unnamed = format!("{}: a library product has no name to build", crate::direction::FILE);
        for (product, message) in [
            (r#"{"name": "Plain", "type": "library"}"#, unknown("Plain", r#""library""#)),
            (r#"{"name": "NoType"}"#, unknown("NoType", "null")),
            (r#"{"name": "Empty", "type": {}}"#, unknown("Empty", "{}")),
            (
                r#"{"name": "Mixed", "type": {"library": ["automatic"], "executable": null}}"#,
                unknown("Mixed", r#"{"executable":null,"library":["automatic"]}"#),
            ),
            (r#"{"name": "Tools", "type": {"executable": null, "plugin": null}}"#, unknown("Tools", r#"{"executable":null,"plugin":null}"#)),
            (r#"{"name": "Odd", "type": {"framework": null}}"#, unknown("Odd", r#"{"framework":null}"#)),
            (r#"{"name": "Cased", "type": {"Library": ["automatic"]}}"#, unknown("Cased", r#"{"Library":["automatic"]}"#)),
            (r#"{"name": "Shouted", "type": {"EXECUTABLE": null}}"#, unknown("Shouted", r#"{"EXECUTABLE":null}"#)),
            (r#"{"name": "", "type": {"library": ["automatic"]}}"#, unnamed.clone()),
            (r#"{"type": {"library": ["static"]}}"#, unnamed),
        ] {
            let package: serde_json::Value = serde_json::from_str(&format!(r#"{{"products": [{product}]}}"#)).unwrap();
            assert_eq!(library_names(&package).unwrap_err(), message, "{product}");
        }
    }

    /// SwiftPM's own dump of a manifest, not a fixture of it: every library product the manifest lists is
    /// read, of each kind and in its order, and a product of another kind is not, so a library added to the
    /// manifest joins the list with no other edit; a manifest that lists none gives an empty list, which the
    /// stage refuses rather than building nothing. SwiftPM dumps the manifest alone, so the targets it names
    /// need no sources and the list stands before any build of the package.
    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "asks SwiftPM through xcrun, which only macOS has")]
    fn the_products_are_what_swiftpm_dumps_of_the_manifest() {
        let manifest = |products: &str| {
            format!(
                "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(\n    name: \"CraftBridge\",\n    products: [{products}],\n    targets: [.target(name: \"CraftBridge\"), .target(name: \"CraftBridgeEffects\", dependencies: [\"CraftBridge\"]), .executableTarget(name: \"Tool\")]\n)\n"
            )
        };
        let scratch = crate::scratch::Scratch::new("swift-products");
        scratch.write(
            crate::direction::FILE,
            manifest(
                ".library(name: \"CraftBridge\", targets: [\"CraftBridge\"]), .executable(name: \"Tool\", targets: [\"Tool\"]), .library(name: \"CraftBridgeEffects\", type: .static, targets: [\"CraftBridgeEffects\"]), .library(name: \"CraftBridgeAdded\", type: .dynamic, targets: [\"CraftBridgeEffects\"])",
            ),
        );
        assert_eq!(swift_products(&scratch.0).unwrap(), ["CraftBridge", "CraftBridgeEffects", "CraftBridgeAdded"]);
        scratch.write(crate::direction::FILE, manifest(""));
        assert_eq!(swift_products(&scratch.0).unwrap(), Vec::<String>::new());
    }
}
