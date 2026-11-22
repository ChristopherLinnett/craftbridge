//! `cargo xtask aar-native-libraries <jar> <module> <aar>`: the AAR the release publishes of the module
//! that packages the SDK's native library carries that library for every Android ABI the toolchain names.
//!
//! The JDK's own `jar` lists an AAR's entries and this judges what it printed, as `cargo xtask
//! elf-alignment` judges what `llvm-readelf` printed: for the module the layer table packages the
//! distribution library in, `jni/<abi>/lib<library>.so` must stand for every ABI
//! `rust/rust-toolchain.toml` names, because that path is exactly where Android's loader looks when an app
//! that installed the AAR starts the engine, and an AAR that ships none ships an app that opens to nothing.
//! The module, the library's file name and the ABIs are read from the definitions `tools/build-android.sh`
//! builds the library from — the layer table and the toolchain file — so no list of them stands here: a
//! target added to the toolchain file joins the required set through this check alone. A module of the
//! SDK's that packages no native library of its own, the core among them, has nothing required of it.

use std::path::Path;
use std::process::{Command, ExitCode};

/// What the AAR of one module must hold: nothing, when the module packages no native library, or one
/// entry a piece for every ABI the toolchain names.
#[derive(Debug)]
enum Required {
    NoNativeLibrary,
    Abis(Vec<&'static str>),
}

/// [`Required`] for the module `module`, read from where the build reads: the layer table for which module
/// packages the SDK's library and what its file is called, the toolchain file for the ABIs it is built for.
fn required(root: &Path, module: &str) -> Result<Required, String> {
    if crate::layers::sdk_module()? != module {
        return Ok(Required::NoNativeLibrary);
    }
    Ok(Required::Abis(crate::pins::android_abis(root)?))
}

/// The library's entry of one ABI's folder, as `jar --list --file` prints it: Android packages a native
/// library under `jni/<abi>/`, and Cargo names the file after the library ([`crate::layers`]).
fn entry(abi: &str, library: &str) -> String {
    format!("jni/{abi}/{library}")
}

/// The entries a `jar --list --file` listing holds not of `want`. A directory entry of the listing is no
/// match for a file entry of the wish: only the library itself loads, its folder alone holds nothing.
fn missing(listing: &str, want: &[String]) -> Vec<String> {
    want.iter().filter(|entry| !listing.lines().any(|line| line == **entry)).cloned().collect()
}

/// The listing `jar` prints of `aar`, or why it could not be had.
fn listing(jar: &Path, aar: &Path) -> Result<String, String> {
    let output = Command::new(jar).args(["--list", "--file"]).arg(aar).output().map_err(|error| format!("cannot run {}: {error}", jar.display()))?;
    if !output.status.success() {
        return Err(format!("{} exited with {}: {}", jar.display(), output.status, String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The verdict of one AAR: nothing was asked of it, the library stands for each ABI it ships, or the AAR is
/// refused with the reason the run says.
#[derive(Debug)]
enum Report {
    NoNativeLibrary,
    Held(Vec<&'static str>),
}

/// Judge `aar` as the AAR published by the module `module`: everything that is known of it is read from
/// the tree at `root`, and `jar` — the tool that reads a zip the way the Android tooling's own JDK does —
/// is asked what the AAR holds.
fn check(jar: &Path, root: &Path, module: &str, aar: &Path) -> Result<Report, String> {
    let Required::Abis(abis) = required(root, module)? else {
        return Ok(Report::NoNativeLibrary);
    };
    let library = crate::layers::android_library_file()?;
    let want: Vec<String> = abis.iter().map(|abi| entry(abi, &library)).collect();
    let absent = missing(&listing(jar, aar)?, &want);
    if !absent.is_empty() {
        return Err(format!("its jni/ holds no {}: an app that installs it has no engine to load", absent.join(", no ")));
    }
    Ok(Report::Held(abis))
}

/// The line a run says of one AAR, and the status it ends with. The publish task and the release print the
/// line and stop on the status, so the two together are the verdict both enforcers give.
fn verdict(module: &str, aar: &str, report: Result<Report, String>) -> (String, ExitCode) {
    match report {
        Ok(Report::NoNativeLibrary) => {
            (format!("aar-native-libraries: {module}: it packages no native library, so {aar} is published as it stands"), ExitCode::SUCCESS)
        }
        Ok(Report::Held(abis)) => (format!("aar-native-libraries: {aar}: the library for each ABI it ships: {}", abis.join(", ")), ExitCode::SUCCESS),
        Err(reason) => (format!("aar-native-libraries: {aar}: {reason}"), ExitCode::from(1)),
    }
}

/// `cargo xtask aar-native-libraries <jar> <module> <aar>`: say what the one AAR it is given holds, or
/// refuse it.
pub(crate) fn run(root: &Path, jar: &str, module: &str, aar: &str) -> ExitCode {
    let (line, status) = verdict(module, aar, check(Path::new(jar), root, module, Path::new(aar)));
    println!("{line}");
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    /// A `jar --list --file` listing of an AAR that holds the library under each folder of `abis`, beside
    /// the entries every AAR holds: the classes and the manifest.
    fn listing_of(abis: &[&str]) -> String {
        let mut lines: Vec<String> =
            vec!["META-INF/com/android/build/gradle/aar-metadata.properties".to_string(), "classes.jar".to_string(), "AndroidManifest.xml".to_string()];
        lines.extend(abis.iter().map(|abi| entry(abi, LIBRARY)));
        lines.push("R.txt".to_string());
        lines.join("\n")
    }

    /// The library name is read from the table by the check itself; the tests fix only what the listing
    /// spells, so a renamed library moves the listing and the wish together, as it moves the build.
    const LIBRARY: &str = "libcraftbridge_effects_ffi.so";

    #[test]
    fn an_aar_holding_the_library_for_every_abi_of_the_toolchain_passes() {
        let root = crate::repo_root();
        let Required::Abis(abis) = required(&root, "effects").unwrap() else {
            panic!("the module the table packages the library in is required to hold it, wherever its row points it");
        };
        let entries: Vec<String> = abis.iter().map(|abi| entry(abi, LIBRARY)).collect();
        let listing = listing_of(&abis);
        assert!(missing(&listing, &entries).is_empty(), "every entry the check wishes is in the listing of an AAR that ships them: {listing}");
        assert!(
            entries.contains(&"jni/arm64-v8a/libcraftbridge_effects_ffi.so".to_string())
                && entries.contains(&"jni/x86_64/libcraftbridge_effects_ffi.so".to_string()),
            "the two ABIs an Android app of this release runs on are required: {entries:?}"
        );
    }

    #[test]
    fn an_aar_missing_one_abi_names_that_abi_and_all_of_them_when_it_holds_none() {
        let root = crate::repo_root();
        let Required::Abis(abis) = required(&root, "effects").unwrap() else {
            panic!("the module the table packages the library in is required to hold it");
        };
        let entries: Vec<String> = abis.iter().map(|abi| entry(abi, LIBRARY)).collect();
        // One ABI emptied: the check names the entry of that ABI, and no other.
        let arm_only = listing_of(&["arm64-v8a"]);
        let absent = missing(&arm_only, &entries);
        assert_eq!(absent, vec!["jni/x86_64/libcraftbridge_effects_ffi.so"], "the missing ABI alone is named: {absent:?}");
        // The whole folder gone, as a build that never ran leaves it: every ABI is named, none is skipped.
        let absent = missing(&listing_of(&[]), &entries);
        assert_eq!(absent, entries, "an AAR that holds no native library is refused for every ABI at once");
    }

    #[test]
    fn a_folder_entry_holds_nothing_and_a_module_that_ships_none_is_published_as_it_stands() {
        let root = crate::repo_root();
        // A listing that holds each ABI's folder and no library: Android finds nothing to load, and so does
        // the check — a folder is not its file.
        let folders_only = "jni/arm64-v8a/\njni/x86_64/\n";
        let Required::Abis(abis) = required(&root, "effects").unwrap() else {
            panic!("the module the table packages the library in is required to hold it");
        };
        let entries: Vec<String> = abis.iter().map(|abi| entry(abi, LIBRARY)).collect();
        assert_eq!(missing(folders_only, &entries), entries, "a folder stands for no library");
        // The core ships Kotlin only: nothing is required of its AAR, and it is published as it always was.
        assert!(matches!(required(&root, "core").unwrap(), Required::NoNativeLibrary));
    }

    /// A `jar` standing in for the JDK's: given exactly `--list --file <aar>` it prints the scratch's
    /// `listing.txt`, as the JDK's own prints an AAR's entries, and it refuses any other call, so the check
    /// is read end to end — the call, the listing, the verdict — with no JDK at hand. A second tool refuses
    /// every file, as `jar` refuses one it cannot open. Each is written once, before any is run.
    #[cfg(unix)]
    fn tools(scratch: &Scratch, aar: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let listing = scratch.0.join("listing.txt");
        scratch.write(
            "bin/jar",
            format!(
                "#!/bin/sh\n[ \"$#\" = 3 ] && [ \"$1\" = --list ] && [ \"$2\" = --file ] && [ \"$3\" = '{}' ] || {{ echo \"asked $*\" >&2; exit 2; }}\ncat '{}'\n",
                aar.display(),
                listing.display()
            ),
        );
        scratch.write("bin/refusing-jar", "#!/bin/sh\necho 'java.nio.file.NoSuchFileException' >&2\nexit 1\n");
        for tool in ["bin/jar", "bin/refusing-jar"] {
            std::fs::set_permissions(scratch.0.join(tool), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (scratch.0.join("bin/jar"), scratch.0.join("bin/refusing-jar"))
    }

    /// What the check makes of the listing `jar` prints, through the verdict a run says: the effects AAR
    /// with the library under every ABI passes, one missing an ABI is refused naming that entry alone, one
    /// with none is refused naming every entry, and the core's AAR, which ships none, passes as it stands.
    /// Each line is pinned by its words and each status by its number, since the publish task and the
    /// release stop on the status and print the line.
    #[test]
    #[cfg(unix)]
    fn the_verdict_of_an_aar_is_its_line_and_its_status_read_from_what_jar_lists() {
        let root = crate::repo_root();
        let scratch = Scratch::new("aar-native-libraries-verdict");
        let aar = scratch.0.join("effects-0.1.0.aar");
        let shown = aar.display().to_string();
        let (jar, _) = tools(&scratch, &aar);
        let judged = |module: &str, abis: &[&str]| {
            scratch.write("listing.txt", listing_of(abis));
            verdict(module, &shown, check(&jar, &root, module, &aar))
        };
        assert_eq!(
            judged("effects", &["arm64-v8a", "x86_64"]),
            (format!("aar-native-libraries: {shown}: the library for each ABI it ships: arm64-v8a, x86_64"), ExitCode::SUCCESS)
        );
        assert_eq!(
            judged("effects", &["arm64-v8a"]),
            (
                format!(
                    "aar-native-libraries: {shown}: its jni/ holds no jni/x86_64/libcraftbridge_effects_ffi.so: an app that installs it has no engine to load"
                ),
                ExitCode::from(1)
            ),
            "one ABI missing is refused naming that entry alone"
        );
        assert_eq!(
            judged("effects", &[]),
            (
                format!(
                    "aar-native-libraries: {shown}: its jni/ holds no jni/arm64-v8a/libcraftbridge_effects_ffi.so, no jni/x86_64/libcraftbridge_effects_ffi.so: an app that installs it has no engine to load"
                ),
                ExitCode::from(1)
            ),
            "an AAR with no native library is refused naming every entry"
        );
        assert_eq!(
            judged("core", &[]),
            (format!("aar-native-libraries: core: it packages no native library, so {shown} is published as it stands"), ExitCode::SUCCESS),
            "the core ships Kotlin only, and is published as it always was"
        );
        scratch.write("listing.txt", listing_of(&[]));
        assert_eq!(run(&root, &jar.display().to_string(), "effects", &shown), ExitCode::from(1), "the command ends on the verdict's status");
    }

    #[test]
    #[cfg(unix)]
    fn a_tool_that_cannot_list_the_aar_is_the_checks_own_failure() {
        let root = crate::repo_root();
        let scratch = Scratch::new("aar-native-libraries-tools");
        let aar = scratch.0.join("effects-0.1.0.aar");
        let (_, refusing) = tools(&scratch, &aar);
        // A tool that is not there, and a tool that refuses the file: a check that read nothing must not
        // pass, so both are a refusal in the tool's own words, not a silent hold.
        let missing_tool = check(Path::new("/nonexistent/jar"), &root, "effects", &aar);
        assert!(
            missing_tool.as_ref().is_err_and(|reason| reason.contains("cannot run /nonexistent/jar")),
            "a tool that will not start is named: {missing_tool:?}"
        );
        let unreadable = check(&refusing, &root, "effects", &aar);
        assert!(
            unreadable.as_ref().is_err_and(|reason| reason.contains("exited with") && reason.ends_with("java.nio.file.NoSuchFileException")),
            "the tool's own refusal of an AAR it cannot open is the verdict, in its words: {unreadable:?}"
        );
    }

    /// The script the publish task runs refuses any call but a module and an AAR, with its usage, before it
    /// builds or asks anything.
    #[test]
    fn the_check_script_misused_exits_64_with_its_usage() {
        crate::assert_script_refuses(
            "aar-native-libraries.sh",
            &[&[], &["effects"], &["effects", "effects-0.1.0.aar", "again"]],
            "usage: tools/aar-native-libraries.sh <module> <aar>",
        );
    }
}
