//! The Swift bindings of the distribution library: UniFFI's Swift for both `-ffi` crates, which compile into
//! the one `CraftBridgeFFI` module, and the C headers and module maps that module's `import` of each
//! crate's low-level module needs, which ship in the XCFramework beside the static libraries and link what
//! each of those libraries needs.
//!
//! The library-mode run is [`crate::bindings`]'s; this keeps what an iOS build takes from it. The Swift is a
//! generator `cargo xtask generate` runs, committed under a `Platform/` folder, because UniFFI's output is
//! built on pointer APIs, which AGENTS.md §4 confines to such a folder. It is written as the gate reads it:
//! formatted with the repository's own swift-format configuration, and without the `swiftlint:` commands
//! UniFFI writes, which AGENTS.md §10 refuses everywhere and which would switch SwiftLint off for the whole
//! file. What no formatting meets — the rules UniFFI's fixed templates break — the Swift stages exempt
//! this folder alone from, each rule named once with its reason where the stage keeps its rules
//! (`crate::swift_format`, `crate::swift_lint`). The headers are no source of any target: SwiftPM reads
//! them from the XCFramework (`crate::xcframework`), so they are never committed.

use std::fs;
use std::path::{Path, PathBuf};

/// Where the generated Swift lives: the generated folder inside the `Platform/` folder of the target the
/// UniFFI Swift of every crate compiles into.
pub(crate) const OUTPUT_DIR: &str = "ios/sdk/Sources/CraftBridgeFFI/Platform/Generated";

/// Whether a file, as the repository names it, lies in the folder the Swift bindings are generated into:
/// the one folder the Swift stages exempt from the rules UniFFI's templates cannot meet.
pub(crate) fn holds(file: &str) -> bool {
    Path::new(file).starts_with(OUTPUT_DIR)
}

/// The name the bindgen takes for the language.
const LANGUAGE: &str = "swift";

/// The one module map Clang reads a library's headers by: the name it looks for in the headers folder of a
/// static library of an XCFramework, so it holds every crate's module.
const MODULE_MAP: &str = "module.modulemap";

/// The submodule of each crate's module that links what the static library needs beside it. A module's
/// submodules that are not `explicit` come with every import of it, and their `link` declarations with
/// them, so whichever of the library's modules an app's Swift imports, its link takes them all.
const LINKS_SUBMODULE: &str = "NativeStaticLibs";

/// A native framework or library the static library needs linked beside it, as rustc names it: a static
/// library carries no link flags, so an app's link fails on the first symbol of one that is not linked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Linked {
    Framework(String),
    Library(String),
}

impl Linked {
    /// The module map's declaration that links it: Clang's `link`, with `framework` for a framework.
    fn declaration(&self) -> String {
        match self {
            Self::Framework(name) => format!("link framework \"{name}\""),
            Self::Library(name) => format!("link \"{name}\""),
        }
    }
}

/// The files the run wrote with `extension`, as paths from `written`.
fn files_with_extension(written: &Path, extension: &str) -> Result<Vec<PathBuf>, String> {
    Ok(crate::bindings::files_under(written)?.into_iter().filter(|file| file.extension().is_some_and(|found| found == extension)).collect())
}

/// Copy `files`, paths from `from`, into `out_dir`.
fn copy_into(from: &Path, files: &[PathBuf], out_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(out_dir).map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;
    for file in files {
        let name = file.file_name().ok_or_else(|| format!("{} has no file name", file.display()))?;
        fs::copy(from.join(file), out_dir.join(name)).map_err(|error| format!("could not copy {}: {error}", file.display()))?;
    }
    Ok(())
}

/// `text` without the lines that are SwiftLint commands: a line comment holding `swiftlint:`, as SwiftLint
/// reads one. UniFFI opens and closes each file with one, switching every SwiftLint rule off between them.
fn without_swiftlint_commands(text: &str) -> String {
    text.split_inclusive('\n')
        .filter(|line| !(line.trim_start().starts_with("//") && line.to_ascii_lowercase().contains(crate::swift_lint::SWIFTLINT_COMMAND)))
        .collect()
}

/// Write the Swift of both crates into `out_dir`, without SwiftLint's commands and formatted as the gate reads
/// it. The bindgen writes each crate's headers and module map beside its Swift; no target compiles those, so
/// they are left to [`write_headers`].
pub(crate) fn generate(out_dir: &Path) -> Result<(), String> {
    crate::bindings::with_run(LANGUAGE, |written| {
        let swift = files_with_extension(written, "swift")?;
        for file in &swift {
            let path = written.join(file);
            let text = fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            fs::write(&path, without_swiftlint_commands(&text)).map_err(|error| format!("{}: {error}", path.display()))?;
        }
        crate::swift_format::format_in_place(&swift.iter().map(|file| written.join(file)).collect::<Vec<_>>())?;
        copy_into(written, &swift, out_dir)
    })
}

/// Write what each of the XCFramework's headers folders holds into its folder: each crate's header, and one
/// module map holding each crate's module and, in a submodule of each, the declarations that link what that
/// folder's static library needs. Every folder comes from one run, since only the links differ between them.
pub(crate) fn write_headers(folders: &[(&Path, &[Linked])]) -> Result<(), String> {
    crate::bindings::with_run(LANGUAGE, |written| {
        let headers = files_with_extension(written, "h")?;
        let mut maps = Vec::new();
        for map in files_with_extension(written, "modulemap")? {
            let path = written.join(&map);
            let text = fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            // UniFFI names each crate's module map after the module it declares, unless the crate's settings
            // name the file apart, which none does; a name that is not the module's fails the build rather
            // than linking nothing, since Clang refuses a submodule of a module not declared before it.
            let module = map.file_stem().map(|stem| stem.to_string_lossy().into_owned()).ok_or_else(|| format!("{} has no name", map.display()))?;
            maps.push((module, text));
        }
        for (out_dir, linked) in folders {
            copy_into(written, &headers, out_dir)?;
            fs::write(out_dir.join(MODULE_MAP), module_map(&maps, linked)).map_err(|error| format!("could not write {MODULE_MAP}: {error}"))?;
        }
        Ok(())
    })
}

/// One module map of `maps`, each a module's name and the module map UniFFI wrote for it, each followed by
/// the submodule linking `linked`.
fn module_map(maps: &[(String, String)], linked: &[Linked]) -> String {
    let declarations: String = linked.iter().map(|link| format!("    {}\n", link.declaration())).collect();
    maps.iter().map(|(module, map)| format!("{map}\nmodule {module}.{LINKS_SUBMODULE} {{\n{declarations}}}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn the_swift_lives_under_a_platform_folder_inside_a_generated_one_of_the_ffi_target() {
        assert_eq!(OUTPUT_DIR, "ios/sdk/Sources/CraftBridgeFFI/Platform/Generated");
        assert!(crate::generated::is_generated_dir("Generated"));
    }

    #[test]
    fn only_the_bindings_folder_is_the_bindings_folder() {
        assert!(holds("ios/sdk/Sources/CraftBridgeFFI/Platform/Generated/craftbridge_core_ffi.swift"));
        for other in [
            "ios/sdk/Sources/CraftBridgeFFI/Platform/Written.swift",
            "ios/sdk/Sources/CraftBridgeFFI/Platform/GeneratedSwift/x.swift",
            "ios/sdk/Sources/CraftBridgeFFI/platform/Generated/x.swift",
            "ios/sdk/Sources/CraftBridgeEffects/Generated/LayerCommands.swift",
            "ios/sdk/Sources/CraftBridge/Platform/Generated/x.swift",
            "x/ios/sdk/Sources/CraftBridgeFFI/Platform/Generated/x.swift",
        ] {
            assert!(!holds(other), "{other}");
        }
    }

    #[test]
    fn every_swiftlint_command_line_goes_and_nothing_else() {
        let written =
            "// header\n// swiftlint:disable all\nimport Foundation\n  //SwiftLint:enable all\nlet swiftlint = \"swiftlint:\"\r\n// swiftlint:enable all";
        assert_eq!(without_swiftlint_commands(written), "// header\nimport Foundation\nlet swiftlint = \"swiftlint:\"\r\n");
    }

    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "runs swift-format through xcrun, which only macOS has")]
    fn the_swift_of_both_crates_is_written_and_nothing_a_target_does_not_compile() {
        let scratch = crate::scratch::Scratch::new("swift-generate");
        generate(&scratch.0).unwrap();
        let names: Vec<String> = fs::read_dir(&scratch.0).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(sorted, ["craftbridge_core_ffi.swift", "craftbridge_effects_ffi.swift"], "{names:?}");
        for name in &sorted {
            let text = fs::read_to_string(scratch.0.join(name)).unwrap();
            assert!(text.starts_with("// @generated by `cargo xtask generate` (uniffi-bindgen, swift)"), "{name}: the marker stays first");
            assert!(!text.to_ascii_lowercase().contains(crate::swift_lint::SWIFTLINT_COMMAND), "{name} keeps a SwiftLint command");
            let formatted = Command::new("xcrun")
                .args(["swift-format", "format", "--configuration"])
                .arg(crate::repo_root().join(crate::swift_format::CONFIGURATION))
                .arg(scratch.0.join(name))
                .output()
                .unwrap();
            assert!(formatted.status.success() && formatted.stdout == text.as_bytes(), "{name} is not as swift-format writes it");
        }
    }

    #[test]
    fn each_headers_folder_holds_each_crates_header_and_one_module_map_linking_its_own_librarys_needs() {
        let scratch = crate::scratch::Scratch::new("swift-headers");
        let (device, simulator) = (scratch.0.join("device"), scratch.0.join("simulator"));
        let device_links = [Linked::Framework("VideoToolbox".to_string()), Linked::Library("iconv".to_string())];
        let simulator_links = [Linked::Framework("Metal".to_string())];
        write_headers(&[(&device, &device_links), (&simulator, &simulator_links)]).unwrap();
        for (folder, links, other) in
            [(&device, "    link framework \"VideoToolbox\"\n    link \"iconv\"\n", "Metal"), (&simulator, "    link framework \"Metal\"\n", "iconv")]
        {
            for header in ["craftbridge_core_ffiFFI.h", "craftbridge_effects_ffiFFI.h"] {
                assert!(folder.join(header).is_file(), "{header} was not written");
            }
            let map = fs::read_to_string(folder.join(MODULE_MAP)).unwrap();
            for module in ["craftbridge_core_ffiFFI", "craftbridge_effects_ffiFFI"] {
                assert!(map.lines().any(|line| line == format!("module {module} {{")), "{module} is not in the module map:\n{map}");
                // Declared after its module, as Clang requires, with every one of the library's needs.
                let links_at = map.find(&format!("module {module}.NativeStaticLibs {{\n{links}}}\n"));
                assert!(links_at.is_some_and(|at| map.find(&format!("module {module} {{")).is_some_and(|module_at| module_at < at)), "{module}:\n{map}");
            }
            assert_eq!(map.matches("export *").count(), 2, "every module re-exports what its header declares");
            assert!(!map.contains(other), "a folder links only its own library's needs:\n{map}");
        }
    }

    #[test]
    fn a_framework_is_linked_by_framework_and_a_library_by_name() {
        assert_eq!(Linked::Framework("CoreMedia".to_string()).declaration(), "link framework \"CoreMedia\"");
        assert_eq!(Linked::Library("c++".to_string()).declaration(), "link \"c++\"");
    }
}
