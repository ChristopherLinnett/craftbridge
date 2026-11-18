//! The XCFramework the Swift package's binary target links (plan §22): `cargo xtask xcframework`, which
//! `tools/build-xcframework.sh` runs, and the gate's Swift stages through it. Nobody commits it.
//!
//! It holds the distribution library the SDK ships, built with `--release` for each Apple slice
//! `rust/rust-toolchain.toml` lists, for the iOS version the package's manifest names, each static library
//! with the headers and module map of the crates' low-level modules (`crate::swift_bindings`), joined by
//! `xcodebuild -create-xcframework` into the folder the manifest gives its binary target. Each of those
//! facts is read from what owns it — the library from the layer table (`crate::layers::sdk_library`), the
//! slices from the toolchain file (`crate::pins`), the version and the folder from SwiftPM's reading of the
//! manifest (`crate::direction`), each library's file from Cargo's report of the build, and the system
//! frameworks and libraries each library needs, which its module map links, from rustc's own answer for that
//! library (`--print native-static-libs`) — so none is written a second time, and a framework new platform
//! code calls into is linked with no other edit.
//!
//! The framework is joined again only when what it would hold changed. A join replaces it whole, and every
//! Swift build that links it then rebuilds all that imports it, so a run that changed nothing must leave it
//! as it is. Cargo rewrites a library only when it rebuilds it, and `xcodebuild` copies each library with
//! its time, so the framework holds the library a build reports when its copy there has that file's length
//! and time; the headers, written anew by every run, are compared byte for byte.

use crate::compiled::Artifact;
use crate::swift_bindings::Linked;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// What rustc prints, when asked, for a static library it builds: the native frameworks and libraries an
/// app must link beside it (`--print native-static-libs`), in a note that starts with these words.
const NATIVE_STATIC_LIBS: &str = "native-static-libs";

/// One slice's release build: its target, the static library Cargo reports writing, and what rustc says
/// that library needs linked beside it.
struct Slice {
    target: String,
    library: PathBuf,
    linked: Vec<Linked>,
}

/// The release build of the package `library` for one slice, for `version`. `cargo rustc` hands what
/// follows `--` to the library's own compilation alone, the one that writes the static library and so the
/// one rustc can say what it needs for; `--crate-type staticlib` builds the one kind the framework holds.
/// Cargo keeps a unit's notes and repeats them when the unit is up to date, so a run that rebuilds nothing
/// has the answer too.
fn build_library(rust_dir: &Path, library: &str, target: &str, version: &str) -> Result<Slice, String> {
    let reported = crate::compiled::compile(
        rust_dir,
        "rustc",
        &["--message-format=json", "--release", "--target", target, "--package", library, "--crate-type", "staticlib", "--", "--print", NATIVE_STATIC_LIBS],
        &[("IPHONEOS_DEPLOYMENT_TARGET", version)],
    )?;
    Ok(Slice {
        target: target.to_string(),
        library: static_library_of(&reported.artifacts, library, target)?,
        linked: linked_of(&reported.diagnostics, library, target)?,
    })
}

/// What rustc's note among `diagnostics` says the static library of the package `library` built for
/// `target` needs linked beside it: each framework and library once, in the order it first names them,
/// which a module map's links do not depend on. Anything else in the note is refused, as no module map
/// declaration could link it.
fn linked_of(diagnostics: &[String], library: &str, target: &str) -> Result<Vec<Linked>, String> {
    let prefix = format!("{NATIVE_STATIC_LIBS}: ");
    let answer = diagnostics
        .iter()
        .find_map(|text| text.strip_prefix(&prefix))
        .ok_or_else(|| format!("cargo built {library} for {target} but passed on no `{NATIVE_STATIC_LIBS}` note from rustc"))?;
    let refused = |word: &str| format!("rustc's `{NATIVE_STATIC_LIBS}` for {target} names `{word}`, which is no framework or library a module map can link");
    let mut words = answer.split_whitespace();
    let mut linked = Vec::new();
    while let Some(word) = words.next() {
        let link = if word == "-framework" {
            let name = words.next().unwrap_or_default();
            if !plain(name) {
                return Err(refused(format!("{word} {name}").trim_end()));
            }
            Linked::Framework(name.to_string())
        } else {
            match word.strip_prefix("-l") {
                Some(name) if plain(name) => Linked::Library(name.to_string()),
                _ => return Err(refused(word)),
            }
        };
        if !linked.contains(&link) {
            linked.push(link);
        }
    }
    Ok(linked)
}

/// Whether a framework's or library's name can stand in a module map's quoted name as it is.
fn plain(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|character| character.is_ascii_alphanumeric() || "_-.+".contains(character))
}

/// The static library of the package `library` among the units cargo reported building for `target`: the
/// `.a` of its unit that cargo reports as a `staticlib`, the kind of the distribution library the
/// XCFramework is joined from. Cargo builds every kind a dependency's library declares, so a combined
/// distribution crate's build reports each plugin's own static library too.
fn static_library_of(artifacts: &[Artifact], library: &str, target: &str) -> Result<PathBuf, String> {
    let name = crate::layers::library_name(library);
    artifacts
        .iter()
        .filter(|artifact| artifact.target_name == name && artifact.target_kinds.iter().any(|kind| kind == "staticlib"))
        .flat_map(|artifact| artifact.filenames.iter())
        .map(PathBuf::from)
        .find(|file| file.extension().is_some_and(|extension| extension == "a"))
        .ok_or_else(|| format!("cargo built {library} for {target} but reported no static library"))
}

/// What a framework holds, as its `Info.plist` lists it, read through `plutil`, Apple's own reader of the
/// format: each library's file and the headers folder beside it. `None` for no framework, or one this
/// cannot read, which is then joined again.
fn held(framework: &Path) -> Option<Vec<(PathBuf, PathBuf)>> {
    let output = Command::new("plutil").args(["-convert", "json", "-o", "-"]).arg(framework.join("Info.plist")).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    info.get("AvailableLibraries")?
        .as_array()?
        .iter()
        .map(|library| {
            let folder = framework.join(library.get("LibraryIdentifier")?.as_str()?);
            Some((folder.join(library.get("LibraryPath")?.as_str()?), folder.join(library.get("HeadersPath")?.as_str()?)))
        })
        .collect()
}

/// Whether one file is a copy of another made with its time: the same length and the same modification time.
fn same_stamp(file: &Path, copy: &Path) -> bool {
    match (fs::metadata(file), fs::metadata(copy)) {
        (Ok(file), Ok(copy)) => file.len() == copy.len() && file.modified().ok().is_some_and(|time| copy.modified().ok() == Some(time)),
        _ => false,
    }
}

/// Whether two folders hold the same files, by name and by bytes.
fn same_files(folder: &Path, other: &Path) -> bool {
    let (Ok(names), Ok(other_names)) = (crate::bindings::files_under(folder), crate::bindings::files_under(other)) else { return false };
    names == other_names
        && names.iter().all(|name| matches!((fs::read(folder.join(name)), fs::read(other.join(name))), (Ok(bytes), Ok(other_bytes)) if bytes == other_bytes))
}

/// Whether `framework` already holds exactly `libraries`, each a library and its headers folder: as many
/// libraries as it lists, each a copy of a different one of them beside a headers folder holding that
/// library's headers.
fn holds_already(framework: &Path, libraries: &[(PathBuf, PathBuf)]) -> bool {
    let Some(mut held) = held(framework) else { return false };
    if held.len() != libraries.len() {
        return false;
    }
    for (library, headers) in libraries {
        let Some(at) = held.iter().position(|(copy, _)| same_stamp(library, copy)) else { return false };
        let (_, folder) = held.swap_remove(at);
        if !same_files(headers, &folder) {
            return false;
        }
    }
    true
}

/// Join `libraries`, each a library and its headers folder, into a new framework at `framework`, which
/// `xcodebuild` refuses to write over.
fn join(framework: &Path, libraries: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    if framework.exists() {
        fs::remove_dir_all(framework).map_err(|error| format!("could not clear {}: {error}", framework.display()))?;
    }
    if let Some(parent) = framework.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let mut command = Command::new("xcodebuild");
    command.arg("-create-xcframework");
    for (library, headers) in libraries {
        command.arg("-library").arg(library).arg("-headers").arg(headers);
    }
    let output = command.arg("-output").arg(framework).output().map_err(|error| format!("cannot run xcodebuild: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "xcodebuild -create-xcframework exited with {}: {}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// Build the libraries and their headers, and join them unless the framework holds them already. Returns
/// the framework and whether it was joined. Each slice has a headers folder of its own, since its module map
/// links what rustc says that slice's library needs.
fn build(root: &Path) -> Result<(PathBuf, bool), String> {
    let manifest = Path::new(crate::direction::FILE);
    let package_dir = root.join(manifest.parent().unwrap_or(manifest));
    let package = crate::direction::dump_package(&package_dir, crate::direction::FILE)?
        .ok_or_else(|| format!("{}: no package to build the native library for", crate::direction::FILE))?;
    let version = crate::direction::ios_version(&package)?;
    let framework = package_dir.join(crate::direction::binary_target_path(&package)?);
    let rust_dir = root.join("rust");
    let library = crate::layers::sdk_library()?;
    let slices =
        crate::pins::apple_slices(root)?.iter().map(|slice| build_library(&rust_dir, library, slice.target, &version)).collect::<Result<Vec<_>, _>>()?;
    let headers = crate::scratch::create("xcframework-headers")?;
    let libraries: Vec<(PathBuf, PathBuf)> = slices.iter().map(|slice| (slice.library.clone(), headers.join(&slice.target))).collect();
    let folders: Vec<(&Path, &[Linked])> = libraries.iter().zip(&slices).map(|((_, folder), slice)| (folder.as_path(), slice.linked.as_slice())).collect();
    let joined = crate::swift_bindings::write_headers(&folders)
        .and_then(|()| if holds_already(&framework, &libraries) { Ok(false) } else { join(&framework, &libraries).map(|()| true) });
    let _ = fs::remove_dir_all(&headers);
    joined.map(|joined| (framework, joined))
}

/// What a run says it did with the framework, `shown` as the repository names it.
fn outcome(shown: &str, joined: bool) -> String {
    format!("xcframework: {shown}: {}", if joined { "joined" } else { "unchanged: it holds these libraries and headers already" })
}

/// `cargo xtask xcframework`: build the native library's XCFramework, or leave it as it is when it already
/// holds what the build made, and say which.
pub(crate) fn run(root: &Path) -> ExitCode {
    match build(root) {
        Ok((framework, joined)) => {
            println!("{}", outcome(&crate::report::shown(root, &framework), joined));
            ExitCode::SUCCESS
        }
        Err(failure) => crate::report::failed("xcframework", failure),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;
    use std::time::{Duration, SystemTime};

    #[test]
    fn the_build_script_misused_exits_64_with_its_usage() {
        crate::assert_script_refuses(
            "build-xcframework.sh",
            &[&["--bogus"], &["--keep-bindings", "again"], &["--keep-bindings "]],
            "usage: tools/build-xcframework.sh [--keep-bindings]",
        );
    }

    #[test]
    fn the_library_is_the_static_one_cargo_reports() {
        let messages = [
            r#"{"reason":"compiler-artifact","target":{"name":"serde","kind":["lib"]},"filenames":["/t/release/deps/libserde-1.rlib"]}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"craftbridge_effects_ffi","kind":["staticlib"]},"filenames":["/t/aarch64-apple-ios/release/libcraftbridge_effects_ffi.a"]}"#,
        ]
        .join("\n");
        let artifacts = crate::compiled::read_messages(messages.as_bytes()).unwrap();
        assert_eq!(
            static_library_of(&artifacts, "craftbridge-effects-ffi", "aarch64-apple-ios"),
            Ok(PathBuf::from("/t/aarch64-apple-ios/release/libcraftbridge_effects_ffi.a"))
        );
        let none = crate::compiled::read_messages(messages.lines().next().unwrap().as_bytes()).unwrap();
        assert_eq!(
            static_library_of(&none, "craftbridge-effects-ffi", "aarch64-apple-ios"),
            Err("cargo built craftbridge-effects-ffi for aarch64-apple-ios but reported no static library".to_string())
        );
    }

    /// A combined distribution crate's build reports each plugin's static library before its own, since
    /// Cargo builds every kind a dependency declares: the framework holds the library asked for.
    #[test]
    fn the_library_is_the_package_s_own_among_its_plugins_static_libraries() {
        let messages = [
            r#"{"reason":"compiler-artifact","target":{"name":"craftbridge_effects_ffi","kind":["staticlib","cdylib","lib"]},"filenames":["/t/aarch64-apple-ios/release/deps/libcraftbridge_effects_ffi.a","/t/aarch64-apple-ios/release/deps/libcraftbridge_effects_ffi.rlib"]}"#,
            r#"{"reason":"compiler-artifact","target":{"name":"craftbridge_all_ffi","kind":["staticlib"]},"filenames":["/t/aarch64-apple-ios/release/libcraftbridge_all_ffi.a"]}"#,
        ]
        .join("\n");
        let artifacts = crate::compiled::read_messages(messages.as_bytes()).unwrap();
        assert_eq!(
            static_library_of(&artifacts, "craftbridge-all-ffi", "aarch64-apple-ios"),
            Ok(PathBuf::from("/t/aarch64-apple-ios/release/libcraftbridge_all_ffi.a"))
        );
    }

    #[test]
    fn what_the_library_needs_linked_is_each_framework_and_library_rustc_names_once() {
        // rustc's two notes for the distribution library, as cargo passes them on.
        let notes = [
            "link against the following native artifacts when linking against this static library. The order and any duplication can be significant on some platforms".to_string(),
            "native-static-libs: -framework VideoToolbox -framework CoreVideo -framework CoreMedia -framework AudioToolbox -lSystem -framework QuartzCore -framework CoreGraphics -framework Metal -framework Foundation -framework CoreText -framework CoreFoundation -lobjc -framework Foundation -liconv -lSystem -lc -lm".to_string(),
        ];
        let framework = |name: &str| Linked::Framework(name.to_string());
        let library = |name: &str| Linked::Library(name.to_string());
        assert_eq!(
            linked_of(&notes, "craftbridge-effects-ffi", "aarch64-apple-ios"),
            Ok(vec![
                framework("VideoToolbox"),
                framework("CoreVideo"),
                framework("CoreMedia"),
                framework("AudioToolbox"),
                library("System"),
                framework("QuartzCore"),
                framework("CoreGraphics"),
                framework("Metal"),
                framework("Foundation"),
                framework("CoreText"),
                framework("CoreFoundation"),
                library("objc"),
                library("iconv"),
                library("c"),
                library("m"),
            ])
        );
        assert_eq!(
            linked_of(&notes[..1], "craftbridge-effects-ffi", "aarch64-apple-ios"),
            Err("cargo built craftbridge-effects-ffi for aarch64-apple-ios but passed on no `native-static-libs` note from rustc".to_string())
        );
        for (word, refused) in [
            ("-framework", "-framework"),
            ("-framework Core\"Text", "-framework Core\"Text"),
            ("-l", "-l"),
            ("-l:libz.a", "-l:libz.a"),
            ("-Wl,-needed_framework", "-Wl,-needed_framework"),
            ("/usr/lib/libz.a", "/usr/lib/libz.a"),
        ] {
            assert_eq!(
                linked_of(&[format!("native-static-libs: -lc {word}")], "craftbridge-effects-ffi", "aarch64-apple-ios-sim"),
                Err(format!(
                    "rustc's `native-static-libs` for aarch64-apple-ios-sim names `{refused}`, which is no framework or library a module map can link"
                )),
                "{word}"
            );
        }
    }

    #[test]
    fn a_tree_with_no_package_has_no_native_library_to_build() {
        let scratch = Scratch::new("xcframework-no-package");
        assert_eq!(build(&scratch.0).unwrap_err(), "ios/sdk/Package.swift: no package to build the native library for");
    }

    #[test]
    fn a_run_says_whether_it_joined_the_framework() {
        let shown = "ios/sdk/Frameworks/CraftBridgeEffectsFFI.xcframework";
        assert_eq!(outcome(shown, true), "xcframework: ios/sdk/Frameworks/CraftBridgeEffectsFFI.xcframework: joined");
        assert_eq!(
            outcome(shown, false),
            "xcframework: ios/sdk/Frameworks/CraftBridgeEffectsFFI.xcframework: unchanged: it holds these libraries and headers already"
        );
    }

    /// The layout `xcodebuild -create-xcframework` writes, as its `Info.plist` names it: a framework holding
    /// a copy, made with its time, of each library in `libraries`, beside a copy of that library's headers.
    fn framework_of(scratch: &Scratch, libraries: &[(&str, &Path)]) -> PathBuf {
        let framework = scratch.0.join("F.xcframework");
        let mut entries = String::new();
        for (index, (library, headers)) in libraries.iter().enumerate() {
            let slice = framework.join(format!("slice-{index}"));
            fs::create_dir_all(slice.join("Headers")).unwrap();
            let copy = slice.join("lib.a");
            fs::copy(scratch.0.join(library), &copy).unwrap();
            fs::File::options().write(true).open(&copy).unwrap().set_modified(fs::metadata(scratch.0.join(library)).unwrap().modified().unwrap()).unwrap();
            for name in crate::bindings::files_under(headers).unwrap() {
                fs::copy(headers.join(&name), slice.join("Headers").join(&name)).unwrap();
            }
            entries.push_str(&format!(
                "<dict><key>LibraryIdentifier</key><string>slice-{index}</string><key>LibraryPath</key><string>lib.a</string><key>HeadersPath</key><string>Headers</string></dict>"
            ));
        }
        fs::write(
            framework.join("Info.plist"),
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict><key>AvailableLibraries</key><array>{entries}</array></dict></plist>\n"),
        )
        .unwrap();
        framework
    }

    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "reads the framework with plutil, which only macOS has")]
    fn a_framework_is_joined_again_only_when_a_library_or_its_headers_changed() {
        let scratch = Scratch::new("xcframework-held");
        let map = |links: &str| format!("module a_ffiFFI {{}}\nmodule a_ffiFFI.NativeStaticLibs {{\n    link \"{links}\"\n}}\n");
        for slice in ["device", "simulator"] {
            scratch.write(&format!("{slice}/lib.a"), format!("{slice} library"));
            scratch.write(&format!("{slice}-headers/a_ffiFFI.h"), "// a\n");
            scratch.write(&format!("{slice}-headers/module.modulemap"), map(slice));
        }
        let device = (scratch.0.join("device/lib.a"), scratch.0.join("device-headers"));
        let simulator = (scratch.0.join("simulator/lib.a"), scratch.0.join("simulator-headers"));
        let both = [device.clone(), simulator.clone()];
        let joined = || framework_of(&scratch, &[("device/lib.a", &device.1), ("simulator/lib.a", &simulator.1)]);
        let framework = joined();
        assert!(holds_already(&framework, &both), "the framework holds both libraries, each beside its headers");
        assert!(holds_already(&framework, &[simulator.clone(), device.clone()]), "in whichever order");
        let swapped = [(device.0.clone(), simulator.1.clone()), (simulator.0.clone(), device.1.clone())];
        assert!(!holds_already(&framework, &swapped), "each library beside its own headers, not another slice's");

        // A library Cargo rebuilt has a time of its own, though it may hold the same bytes.
        let rebuilt = SystemTime::now() + Duration::from_secs(5);
        fs::File::options().write(true).open(&simulator.0).unwrap().set_modified(rebuilt).unwrap();
        assert!(!holds_already(&framework, &both), "a rebuilt library");
        let framework = joined();
        assert!(holds_already(&framework, &both));

        scratch.write("device-headers/a_ffiFFI.h", "// b\n");
        assert!(!holds_already(&framework, &both), "a header with other bytes");
        scratch.write("device-headers/a_ffiFFI.h", "// a\n");
        scratch.write("simulator-headers/module.modulemap", map("other"));
        assert!(!holds_already(&framework, &both), "a library that needs another link");
        scratch.write("simulator-headers/module.modulemap", map("simulator"));
        scratch.write("simulator-headers/b_ffiFFI.h", "// b\n");
        assert!(!holds_already(&framework, &both), "a header more");
        fs::remove_file(simulator.1.join("b_ffiFFI.h")).unwrap();
        assert!(holds_already(&framework, &both));

        assert!(!holds_already(&framework, &both[..1]), "a slice the toolchain no longer lists");
        scratch.write("intel/lib.a", "intel library");
        assert!(!holds_already(&framework, &[device.clone(), simulator.clone(), (scratch.0.join("intel/lib.a"), device.1.clone())]), "a slice more");
        assert!(!holds_already(&framework, &[device.clone(), device.clone()]), "one library twice is not two");
        assert!(!holds_already(&scratch.0.join("Missing.xcframework"), &both), "no framework");
        fs::write(framework.join("Info.plist"), "not a property list").unwrap();
        assert!(!holds_already(&framework, &both), "a framework this cannot read");
    }
}
