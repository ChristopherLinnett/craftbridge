//! The Kotlin bindings of the distribution library: UniFFI's Kotlin for each `-ffi` crate, one package per
//! crate, each written into its own Android module's generated Kotlin source folder (plan §20, §23).
//!
//! The library-mode run is [`crate::bindings`]'s; this keeps what an Android build takes from it and adapts
//! it to the build that compiles it. Each module's bindings are a generator of their own, since each
//! generator owns one folder.

use std::path::{Path, PathBuf};

/// The name the bindgen takes for the language.
const LANGUAGE: &str = "kotlin";

/// The Kotlin the bindgen writes suppresses one diagnostic for the whole file, on this line.
const FILE_SUPPRESSION: &str = "@file:Suppress(\"NAME_SHADOWING\")";

/// What the file suppresses once the bindings' own exemption is added to it. The generated Kotlin declares
/// its helpers and callbacks with no visibility and no return type, which explicit API mode refuses on a
/// public declaration, and Kotlin has no per-folder setting for it. So the bindings, and only they,
/// suppress those two diagnostics for themselves; detekt refuses a suppression of any compiler error in
/// the written code and the command facade beside them, so they keep the mode whole. A file may carry the
/// annotation once, which is why the line UniFFI writes is replaced rather than joined. The compiler's
/// warning about suppressing an error is silenced by the convention plugin, which would otherwise fail
/// every build under warnings as errors.
const FILE_SUPPRESSIONS: &str = "@file:Suppress(\"NAME_SHADOWING\", \"NO_EXPLICIT_VISIBILITY_IN_API_MODE\", \"NO_EXPLICIT_RETURN_TYPE_IN_API_MODE\")";

/// One file of the Kotlin the bindgen wrote, adapted to the build that compiles it: its one file-wide
/// suppression widened to the bindings' exemption. A line the bindgen no longer writes is an error, not a
/// silent skip, since the build would then fail on the diagnostics it was there to quiet.
fn adapted(text: &str) -> Result<String, String> {
    if text.matches(FILE_SUPPRESSION).count() != 1 {
        return Err(format!("the Kotlin the bindgen wrote does not carry {FILE_SUPPRESSION} exactly once"));
    }
    Ok(text.replace(FILE_SUPPRESSION, FILE_SUPPRESSIONS))
}

/// Where the Kotlin bindings' packages sit inside a module's generated Kotlin source folder: one folder per
/// module, named for it, beside the folder the command facade owns (plan §20, §23).
const PACKAGES: &str = "io/github/christopherlinnett/craftbridge/ffi";

/// The modules that have Kotlin bindings: the universal core's and the EffectCraft plugin's.
const MODULES: [&str; 2] = ["core", "effects"];

/// The folder of the core's bindings, from the repository root: the one folder its generator owns.
pub(crate) const CORE_OUTPUT_DIR: &str = "android/sdk/core/src/generated/kotlin/io/github/christopherlinnett/craftbridge/ffi/core";

/// The folder of the plugin's bindings, the same way.
pub(crate) const EFFECTS_OUTPUT_DIR: &str = "android/sdk/effects/src/generated/kotlin/io/github/christopherlinnett/craftbridge/ffi/effects";

/// Which of `files`, as the bindgen wrote them for Kotlin, lie in no module's package folder. A crate whose
/// bindings no generator owns would otherwise be generated into nothing and never compared.
fn unowned(files: &[PathBuf]) -> Vec<String> {
    files
        .iter()
        .filter(|file| !MODULES.iter().any(|module| file.starts_with(Path::new(PACKAGES).join(module))))
        .map(|file| file.display().to_string())
        .collect()
}

/// Write the Kotlin bindings of `module` into `out_dir`: the bindgen's run for Kotlin, whose output holds a
/// package folder for each crate of the library, and of it the one folder named for the module, each file
/// adapted ([`adapted`]).
fn generate(module: &str, out_dir: &Path) -> Result<(), String> {
    crate::bindings::with_run(LANGUAGE, |written| {
        let files = crate::bindings::files_under(written)?;
        let unowned = unowned(&files);
        if !unowned.is_empty() {
            return Err(format!("the bindgen wrote {}, which no module of {} owns: add its generator", unowned.join(", "), MODULES.join(", ")));
        }
        let own = Path::new(PACKAGES).join(module);
        let mut copied = 0;
        for file in files.iter().filter(|file| file.starts_with(&own)) {
            let target = out_dir.join(file.strip_prefix(&own).unwrap_or(file));
            let source = written.join(file);
            let text = std::fs::read_to_string(&source).map_err(|error| format!("{}: {error}", source.display()))?;
            std::fs::create_dir_all(out_dir).map_err(|error| format!("{}: {error}", out_dir.display()))?;
            std::fs::write(&target, adapted(&text)?).map_err(|error| format!("{}: {error}", target.display()))?;
            copied += 1;
        }
        if copied == 0 { Err(format!("the bindgen wrote no Kotlin for the `{module}` module")) } else { Ok(()) }
    })
}

/// The core's Kotlin bindings, for `:core`.
pub(crate) fn generate_core(out_dir: &Path) -> Result<(), String> {
    generate("core", out_dir)
}

/// The EffectCraft plugin's Kotlin bindings, for `:effects`.
pub(crate) fn generate_effects(out_dir: &Path) -> Result<(), String> {
    generate("effects", out_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The marker's first line the runner writes for Kotlin, as the gate reads it.
    const MARKER_LINE: &str = "// @generated by `cargo xtask generate` (uniffi-bindgen, kotlin): do not edit by hand.\n";

    #[test]
    fn kotlin_suppresses_the_two_explicit_api_diagnostics_once() {
        let written = format!("{MARKER_LINE}// UniFFI's own header\n\n{FILE_SUPPRESSION}\n\npackage p\n");
        let kotlin = adapted(&written).unwrap();
        assert!(kotlin.starts_with(MARKER_LINE), "the marker stays first");
        assert_eq!(kotlin.matches("@file:Suppress(").count(), 1, "a file carries the annotation once: {kotlin}");
        assert!(kotlin.contains("\"NAME_SHADOWING\", \"NO_EXPLICIT_VISIBILITY_IN_API_MODE\", \"NO_EXPLICIT_RETURN_TYPE_IN_API_MODE\")"));
        assert!(adapted("package p\n").is_err_and(|error| error.contains("exactly once")), "a line the bindgen stopped writing is not skipped");
        assert!(adapted(&format!("{written}{written}")).is_err(), "a second annotation would not compile");
    }

    #[test]
    fn a_package_folder_no_module_owns_is_refused() {
        let own = |module: &str| Path::new(PACKAGES).join(module).join("f.kt");
        assert!(unowned(&[own("core"), own("effects")]).is_empty());
        let stray = Path::new(PACKAGES).join("film/f.kt");
        assert_eq!(unowned(&[own("core"), stray.clone()]), vec![stray.display().to_string()]);
        assert_eq!(unowned(&[PathBuf::from("loose.kt")]), ["loose.kt"], "a file outside every package folder is no module's");
    }

    #[test]
    fn each_modules_folder_lies_in_its_own_modules_generated_source_folder() {
        assert_eq!(CORE_OUTPUT_DIR, "android/sdk/core/src/generated/kotlin/io/github/christopherlinnett/craftbridge/ffi/core");
        assert_eq!(EFFECTS_OUTPUT_DIR, "android/sdk/effects/src/generated/kotlin/io/github/christopherlinnett/craftbridge/ffi/effects");
    }

    #[test]
    fn the_generators_write_their_own_package_and_nothing_of_the_other() {
        let core = crate::scratch::Scratch::new("bindings-core");
        let effects = crate::scratch::Scratch::new("bindings-effects");
        generate_core(&core.0).unwrap();
        generate_effects(&effects.0).unwrap();
        let core_files = crate::bindings::files_under(&core.0).unwrap();
        let effects_files = crate::bindings::files_under(&effects.0).unwrap();
        assert_eq!(core_files, [PathBuf::from("craftbridge_core_ffi.kt")]);
        assert_eq!(effects_files, [PathBuf::from("craftbridge_effects_ffi.kt")]);
        let core_text = std::fs::read_to_string(core.0.join("craftbridge_core_ffi.kt")).unwrap();
        assert!(core_text.starts_with(MARKER_LINE), "the marker comes first");
        assert!(core_text.contains("@file:Suppress(\"NAME_SHADOWING\", \"NO_EXPLICIT_VISIBILITY_IN_API_MODE\""));
        assert!(core_text.contains("\npackage io.github.christopherlinnett.craftbridge.ffi.core\n"));
        let effects_text = std::fs::read_to_string(effects.0.join("craftbridge_effects_ffi.kt")).unwrap();
        assert!(effects_text.contains("\npackage io.github.christopherlinnett.craftbridge.ffi.effects\n"));
    }
}
