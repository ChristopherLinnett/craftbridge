//! The UniFFI bindings' one runner.
//!
//! The bindings are generated code (plan §20), written from the library an app links, by the workspace's
//! own `uniffi-bindgen` in library mode, which reads the metadata that library embeds. Every generator
//! registered with `cargo xtask generate` that writes bindings goes through [`run`]: it builds the
//! distribution library and the bindgen for the host (once for the whole process, however many generators
//! run), runs the bindgen for one language into a folder, and prepends the `@generated` marker UniFFI does
//! not write. Nothing else in xtask knows how the bindgen is run; what a language adds is the generators of
//! its own folders, which keep what they own of what the run wrote.
//!
//! xtask may not depend on a host tool, so it asks cargo for the two files and runs them, as it runs the
//! codegen (`crate::effects_api`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The distribution library, the one library library mode reads every crate's exports from, and the one an
/// app links (`crate::xcframework`).
pub(crate) const LIBRARY_PACKAGE: &str = "craftbridge-effects-ffi";

/// The workspace's binding generator, built at the version the libraries are (`rust/uniffi-bindgen`).
const BINDGEN_PACKAGE: &str = "uniffi-bindgen";

/// The two files a run needs, built once.
struct HostBuild {
    bindgen: PathBuf,
    library: PathBuf,
}

/// Build the bindgen and the distribution library for the host, and find the two files.
fn build_for_host(rust_dir: &Path) -> Result<HostBuild, String> {
    host_build_of(&crate::compiled::build(rust_dir, &[BINDGEN_PACKAGE, LIBRARY_PACKAGE])?)
}

/// The two files among the units cargo reported building. The library is the unit of the package that
/// cargo reports as a `cdylib`, under the name the host gives a dynamic library.
fn host_build_of(artifacts: &[crate::compiled::Artifact]) -> Result<HostBuild, String> {
    let bindgen =
        crate::compiled::executable_of(artifacts, BINDGEN_PACKAGE).ok_or_else(|| format!("cargo built {BINDGEN_PACKAGE} but reported no binary for it"))?;
    let library_name = format!("{}{}{}", std::env::consts::DLL_PREFIX, LIBRARY_PACKAGE.replace('-', "_"), std::env::consts::DLL_SUFFIX);
    let library = artifacts
        .iter()
        .filter(|artifact| artifact.target_kinds.iter().any(|kind| kind == "cdylib"))
        .flat_map(|artifact| artifact.filenames.iter())
        .map(PathBuf::from)
        .find(|file| file.file_name().is_some_and(|name| name.to_string_lossy() == library_name))
        .ok_or_else(|| format!("cargo built {LIBRARY_PACKAGE} but reported no {library_name}"))?;
    Ok(HostBuild { bindgen, library })
}

/// The host build, made by the first run of the process and answered to every later one, with the failure
/// of that build when it failed.
fn host_build() -> Result<&'static HostBuild, String> {
    static BUILT: OnceLock<Result<HostBuild, String>> = OnceLock::new();
    BUILT.get_or_init(|| build_for_host(&crate::repository_root()?.join("rust"))).as_ref().map_err(Clone::clone)
}

/// What a generated file starts with for `language`: the `@generated` marker the gate looks for in its
/// first lines (UniFFI writes none).
fn header(language: &str) -> String {
    format!(
        "// {} by `cargo xtask generate` (uniffi-bindgen, {language}): do not edit by hand.\n// Change the library's exports or the generator.\n",
        crate::generated::MARKER
    )
}

/// Every file under `folder`, as a path from it.
pub(crate) fn files_under(folder: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|error| format!("{}: {error}", dir.display()))? {
            let path = entry.map_err(|error| format!("{}: {error}", dir.display()))?.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path.strip_prefix(folder).unwrap_or(&path).to_path_buf());
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Run the bindgen for `language` over the host build of the distribution library, into `out_dir`, and give
/// every file it wrote the header of [`header`].
pub(crate) fn run(language: &str, out_dir: &Path) -> Result<(), String> {
    let built = host_build()?;
    // The bindgen asks cargo about the library's crates, which it finds from the folder it runs in.
    let output = Command::new(&built.bindgen)
        .current_dir(crate::repository_root()?.join("rust"))
        .args(["generate", "--no-format", "--library"])
        .arg(&built.library)
        .args(["--language", language, "--out-dir"])
        .arg(out_dir)
        .output()
        .map_err(|error| format!("cannot run {}: {error}", built.bindgen.display()))?;
    if !output.status.success() {
        return Err(format!("{BINDGEN_PACKAGE} exited with {} for {language}: {}", output.status, String::from_utf8_lossy(&output.stderr).trim()));
    }
    for relative in files_under(out_dir)? {
        let file = out_dir.join(relative);
        let text = std::fs::read_to_string(&file).map_err(|error| format!("{}: {error}", file.display()))?;
        std::fs::write(&file, format!("{}{text}", header(language))).map_err(|error| format!("{}: {error}", file.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_gets_the_marker_in_its_first_lines() {
        for language in ["kotlin", "swift"] {
            let head = header(language);
            assert!(
                head.lines()
                    .next()
                    .is_some_and(|line| line.starts_with("// @generated by `cargo xtask generate` (uniffi-bindgen, ") && line.contains(language)),
                "{language}: {head}"
            );
        }
    }

    /// What cargo reports for the two packages: the bindgen's binary, the library's three outputs.
    fn reported(bindgen: bool, library: bool) -> Vec<crate::compiled::Artifact> {
        let name = format!("{}craftbridge_effects_ffi{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
        let bindgen_unit = r#"{"reason":"compiler-artifact","target":{"name":"uniffi-bindgen","kind":["bin"]},"executable":"/t/debug/uniffi-bindgen"}"#;
        let library_unit = format!(
            r#"{{"reason":"compiler-artifact","target":{{"name":"craftbridge_effects_ffi","kind":["staticlib","cdylib","lib"]}},"filenames":["/t/debug/libcraftbridge_effects_ffi.a","/t/debug/{name}"]}}"#
        );
        let lines = [bindgen.then(|| bindgen_unit.to_string()), library.then_some(library_unit)].into_iter().flatten().collect::<Vec<_>>().join("\n");
        crate::compiled::read_messages(lines.as_bytes()).unwrap()
    }

    #[test]
    fn the_host_build_is_the_bindgens_binary_and_the_librarys_dynamic_file() {
        let built = host_build_of(&reported(true, true)).unwrap();
        assert_eq!(built.bindgen, PathBuf::from("/t/debug/uniffi-bindgen"));
        assert!(built.library.starts_with("/t/debug") && built.library.extension().is_some_and(|extension| extension != "a"), "{:?}", built.library);
    }

    #[test]
    fn a_build_that_reports_either_file_missing_says_which() {
        assert_eq!(host_build_of(&reported(false, true)).err(), Some("cargo built uniffi-bindgen but reported no binary for it".to_string()));
        let name = format!("{}craftbridge_effects_ffi{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX);
        assert_eq!(host_build_of(&reported(true, false)).err(), Some(format!("cargo built craftbridge-effects-ffi but reported no {name}")));
    }

    #[test]
    fn a_run_writes_the_libraries_swift_under_the_marker_beside_the_headers_its_module_needs() {
        let scratch = crate::scratch::Scratch::new("bindings-swift");
        run("swift", &scratch.0).unwrap();
        let files = files_under(&scratch.0).unwrap();
        for file in ["craftbridge_core_ffi.swift", "craftbridge_core_ffiFFI.h", "craftbridge_effects_ffi.swift", "craftbridge_effects_ffiFFI.h"] {
            assert!(files.contains(&PathBuf::from(file)), "{file} was not written: {files:?}");
            let text = std::fs::read_to_string(scratch.0.join(file)).unwrap();
            assert!(text.starts_with(&header("swift")), "{file} does not open with the marker");
        }
        let swift = std::fs::read_to_string(scratch.0.join("craftbridge_effects_ffi.swift")).unwrap();
        assert!(swift.contains("public func startEffectsEngine("), "the library's one export is in its Swift");
    }

    #[test]
    fn a_run_for_a_language_the_bindgen_does_not_know_fails_naming_it() {
        let scratch = crate::scratch::Scratch::new("bindings-unknown");
        let error = run("klingon", &scratch.0).unwrap_err();
        assert!(error.starts_with("uniffi-bindgen exited with "), "{error}");
        assert!(error.contains(" for klingon: "), "{error}");
    }
}
