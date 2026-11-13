//! Rule 4 — where `unsafe` may appear at all.
//!
//! Hand-written `unsafe` is the one place the language stops helping, so it is confined to a platform
//! module of a crate the layer table marks as a platform crate: one small, reviewable surface per
//! platform instead of a keyword that can appear anywhere. The two tolerances are deliberate and
//! narrow — UniFFI's generated scaffolding needs `unsafe_code` off for a whole `-ffi` crate, and a
//! `mod platform` declaration needs it off to let its module contain blocks. Anything else is reported.
//!
//! Scanning happens on stripped text (comments and literals blanked, line numbers kept), so a keyword
//! quoted in a comment or a doc block is not a violation, and a real one is not missed because a string
//! ended early.

use crate::{attributes, layers, report, scan};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The lint this rule governs.
const UNSAFE_CODE: &str = "unsafe_code";

/// The one module of a platform crate that may switch it off.
const PLATFORM_MODULE: &str = "platform";

/// One `.rs` file, reduced to what this rule reads.
pub(crate) struct RustFile<'a> {
    /// How a message names the file.
    pub(crate) path: &'a str,
    /// The crate it belongs to, by the name of its folder under `rust/`.
    pub(crate) crate_name: &'a str,
    /// Directory components between `rust/` and the file, so `src/platform` is a pair of components
    /// rather than a substring someone could fake with `src/platformkit`.
    pub(crate) components: Vec<&'a str>,
    /// Whether this is the crate's own `src/lib.rs`, the only place a crate-wide attribute belongs.
    pub(crate) is_lib_root: bool,
    /// Stripped source, one entry per line.
    pub(crate) lines: Vec<String>,
}

/// Check one file's stripped lines against what its crate's row allows.
pub(crate) fn check_file(file: &RustFile<'_>) -> Vec<String> {
    let entry = layers::entry(file.crate_name);
    let platform_crate = entry.is_some_and(|entry| entry.platform);
    let ffi_crate = entry.is_some_and(|entry| entry.ffi);
    let in_platform_module = file.components.windows(2).any(|pair| pair == ["src", PLATFORM_MODULE]);
    let mut errors = Vec::new();

    for (index, line) in file.lines.iter().enumerate() {
        if scan::count_whole_word(line, "unsafe", true, true) > 0 && !(in_platform_module && platform_crate) {
            errors.push(format!("{}:{}: unsafe outside a platform module", file.path, index + 1));
        }
    }

    let text = file.lines.join("\n");
    for allowance in attributes::allowances(&text) {
        // A lint a macro fills in may be this one, so it is read as this one.
        if !allowance.lints.iter().any(|lint| lint == UNSAFE_CODE || lint == attributes::UNREADABLE) {
            continue;
        }
        let module = attributes::declared_module(&text, allowance.after);
        // A whole attribute a macro forwards onto one item is the forwarding idiom, not a level.
        if allowance.level == attributes::MACRO_ATTRIBUTE && !allowance.inner && module.is_none() {
            continue;
        }
        let at = format!("{}:{}: {}({UNSAFE_CODE})", file.path, allowance.line, allowance.level);
        if allowance.inner {
            if !(ffi_crate && file.is_lib_root) {
                errors.push(format!("{at} crate-wide belongs only to an -ffi crate's own src/lib.rs"));
            }
        } else if !(platform_crate && module == Some(PLATFORM_MODULE)) {
            errors.push(format!("{at} belongs only on a platform crate's `mod {PLATFORM_MODULE}` declaration"));
        }
    }
    errors
}

/// A `src/platform/` folder in a crate the table does not mark as a platform crate, reported once per
/// crate: the folder announces unsafe code the crate may not hold.
fn platform_dir_errors(root: &Path, crates: &BTreeMap<String, PathBuf>) -> Vec<String> {
    crates
        .iter()
        .filter(|(name, dir)| dir.join("src").join(PLATFORM_MODULE).is_dir() && !layers::entry(name).is_some_and(|entry| entry.platform))
        .map(|(name, dir)| {
            format!(
                "{}: {name} is not a platform crate (rust/xtask/src/layers.rs), so it has no platform module",
                report::shown(root, &dir.join("src").join(PLATFORM_MODULE))
            )
        })
        .collect()
}

/// The crate a file belongs to: the nearest folder above it holding a `Cargo.toml`. Its folder name is its
/// crate name (the layer rules check that), whether it sits directly under `rust/` or under a grouping
/// folder such as `core/` or `effects/`.
fn owning_crate(rust_dir: &Path, components: &[String]) -> Option<(String, PathBuf)> {
    (1..=components.len()).rev().find_map(|depth| {
        let parts = components.get(..depth)?;
        let dir = parts.iter().fold(rust_dir.to_path_buf(), |dir, part| dir.join(part));
        let name = parts.last()?;
        dir.join("Cargo.toml").is_file().then(|| (name.clone(), dir))
    })
}

/// Run the check: every `.rs` file under `rust/`, generated code excepted (a generator's output is
/// checked by regenerating it, not by reading it). A file in no crate's folder is reported rather than
/// skipped: only `#[path]` or `include!` can bring it into a build, and the code it holds would then be
/// judged as no crate's.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let rust_dir = root.join("rust");
    let mut errors = Vec::new();
    let mut crates = BTreeMap::new();
    for file in scan::walk(&rust_dir, ".rs")? {
        if crate::generated::in_generated(&file.components) {
            continue;
        }
        let Some((crate_name, crate_dir)) = owning_crate(&rust_dir, &file.components) else {
            errors.push(format!(
                "{}: a Rust file in no crate's folder: a crate's code lives under its own Cargo.toml, where every rule reads it as that crate's",
                report::shown(root, &file.path)
            ));
            continue;
        };
        let shown = report::shown(root, &file.path);
        let input = RustFile {
            path: &shown,
            crate_name: &crate_name,
            components: file.components.iter().map(String::as_str).collect(),
            is_lib_root: file.path == crate_dir.join("src").join("lib.rs"),
            lines: scan::strip_rust(&scan::read_text(&file.path)?),
        };
        errors.extend(check_file(&input));
        crates.insert(crate_name, crate_dir);
    }
    errors.extend(platform_dir_errors(root, &crates));
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::{Scratch, rust_tree_copy};

    fn file<'a>(path: &'a str, crate_name: &'a str, components: Vec<&'a str>, source: &'a str) -> RustFile<'a> {
        RustFile { path, crate_name, components, is_lib_root: path.ends_with("/src/lib.rs"), lines: scan::strip_rust(source) }
    }

    /// The rule's verdict on `source` as the `src/lib.rs` of the crate `crate_name`.
    fn lib_of(crate_name: &str, source: &str) -> Vec<String> {
        let path = format!("{crate_name}/src/lib.rs");
        check_file(&file(&path, crate_name, vec![crate_name, "src"], source))
    }

    fn media_lib(source: &str) -> Vec<String> {
        lib_of("craftbridge-media", source)
    }

    /// The plugin crate, which the table does not mark as a platform crate: the example of a crate that
    /// may hold no `unsafe` at all.
    fn plugin_lib(source: &str) -> Vec<String> {
        lib_of("craftbridge-effects", source)
    }

    #[test]
    fn unsafe_in_a_platform_module_of_a_platform_crate_passes() {
        let source = "pub fn read() -> u8 {\n    // SAFETY: the pointer comes from the caller.\n    unsafe { 0 }\n}\n";
        let errors = check_file(&file("craftbridge-media/src/platform/decode.rs", "craftbridge-media", vec!["craftbridge-media", "src", "platform"], source));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn unsafe_anywhere_else_is_reported_with_its_line() {
        let outside = "fn f() {\n    unsafe { }\n}\n";
        let errors = plugin_lib(outside);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].ends_with(":2: unsafe outside a platform module"), "{errors:?}");

        // Even in a `src/platform/` folder, a crate the table does not mark as a platform crate may not
        // use unsafe.
        let wrong_crate = "unsafe { }\n";
        let errors =
            check_file(&file("craftbridge-effects/src/platform/x.rs", "craftbridge-effects", vec!["craftbridge-effects", "src", "platform"], wrong_crate));
        assert!(errors.iter().any(|e| e.contains("unsafe outside a platform module")), "{errors:?}");
    }

    #[test]
    fn a_crate_wide_allow_only_suits_an_ffi_crate_root() {
        let source = format!("#![allow({UNSAFE_CODE})]\n\nfn f() {{}}\n");
        let ffi = check_file(&file("craftbridge-core-ffi/src/lib.rs", "craftbridge-core-ffi", vec!["craftbridge-core-ffi", "src"], &source));
        assert!(ffi.is_empty(), "{ffi:?}");

        assert!(media_lib(&source).iter().any(|e| e.contains("-ffi crate's own src/lib.rs")));

        let wrong_file = check_file(&file("craftbridge-core-ffi/src/main.rs", "craftbridge-core-ffi", vec!["craftbridge-core-ffi", "src"], &source));
        assert!(!wrong_file.is_empty(), "{wrong_file:?}");
    }

    #[test]
    fn a_crate_wide_allow_however_it_is_spelled_is_one() {
        // Spread over lines, nested in cfg_attr, or at the warn level: each still lets unsafe code through.
        for spelling in
            [format!("#![allow(\n    {UNSAFE_CODE},\n)]\n"), format!("#![cfg_attr(all(), allow({UNSAFE_CODE}))]\n"), format!("#![warn({UNSAFE_CODE})]\n")]
        {
            let errors = plugin_lib(&spelling);
            assert!(errors.iter().any(|e| e.contains(":1:") && e.contains("crate-wide")), "{spelling:?}: {errors:?}");
        }
    }

    #[test]
    fn an_allow_on_the_platform_module_is_the_other_allowed_form() {
        for declaration in [
            "mod a;\n\n#[allow(unsafe_code)]\npub(crate) mod platform;\n",
            "#[allow(unsafe_code)]\npub mod platform;\n",
            "#[allow(unsafe_code)] mod platform;\n",
            "#[allow(unsafe_code)]\n#[cfg(any(target_os = \"ios\", target_os = \"android\"))]\nmod platform;\n",
            "#[allow(unsafe_code)]\npub(in crate)\nmod\nplatform {}\n",
        ] {
            assert!(media_lib(declaration).is_empty(), "{declaration:?}: {:?}", media_lib(declaration));
        }
        for elsewhere in
            ["fn f() {}\n#[allow(unsafe_code)]\nfn g() {}\n", "#[allow(unsafe_code)]\nmod platformkit;\n", "#[allow(unsafe_code)]\nmod other;\nmod platform;\n"]
        {
            assert!(media_lib(elsewhere).iter().any(|e| e.contains("mod platform")), "{elsewhere:?}");
        }
        let wrong_crate = plugin_lib("#[allow(unsafe_code)]\npub mod platform;\n");
        assert!(!wrong_crate.is_empty(), "{wrong_crate:?}");
    }

    #[test]
    fn a_quoted_or_commented_unsafe_does_not_count() {
        let source = "//! the keyword this rule confines is quoted in docs all the time: unsafe\nlet example = \"unsafe\";\n// unsafe\n/* unsafe */\nlet raw = r\"unsafe\";\nlet hint = \"#![allow(unsafe_code)]\";\n";
        let errors = plugin_lib(source);
        assert!(errors.is_empty(), "{errors:?}");
        // And the keyword still counts on the line after a multi-line comment.
        let after = "/* doc\nunsafe */\nunsafe { }\n";
        let errors = plugin_lib(after);
        assert!(errors.iter().any(|e| e.contains(":3:")), "{errors:?}");
    }

    #[test]
    fn the_check_reads_the_workspace_on_disk_and_finds_nothing() {
        let errors = check(&crate::repo_root()).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_planted_violation_in_a_real_tree_is_caught() {
        let copy = rust_tree_copy("planted-unsafe");
        let planted = "rust/core/craftbridge-media/src/planted.rs";
        copy.write(planted, "unsafe fn read(raw: *const u8) -> u8 {\n    // SAFETY: a planted block, but not in the platform module.\n    *raw\n}\n");
        let errors = check(&copy.0).unwrap();
        assert!(errors.iter().any(|error| error.contains("planted.rs:1: unsafe outside a platform module")), "{errors:?}");
        // The same tree without the planted file is clean, so the report came from the planted file.
        std::fs::remove_file(copy.0.join(planted)).unwrap();
        assert!(check(&copy.0).unwrap().is_empty());
    }

    #[test]
    fn a_planted_block_in_the_platform_module_is_the_permitting_case() {
        let copy = rust_tree_copy("planted-legal");
        copy.write(
            "rust/core/craftbridge-media/src/platform/planted.rs",
            "unsafe fn peek(raw: *const u8) -> u8 {\n    // SAFETY: the caller promises the pointer is valid for one byte.\n    *raw\n}\n",
        );
        assert!(check(&copy.0).unwrap().is_empty(), "a block in the platform module of a platform crate is the permitted place");
    }

    #[test]
    fn a_planted_crate_wide_allow_outside_an_ffi_crate_is_caught() {
        let copy = rust_tree_copy("planted-allow");
        let lib = copy.0.join("rust/core/craftbridge-core/src/lib.rs");
        let original = std::fs::read_to_string(&lib).unwrap();
        std::fs::write(&lib, format!("#![allow({UNSAFE_CODE})]\n{original}")).unwrap();
        let errors = check(&copy.0).unwrap();
        assert!(errors.iter().any(|error| error.contains("craftbridge-core/src/lib.rs:1: allow(unsafe_code) crate-wide")), "{errors:?}");
        std::fs::write(&lib, &original).unwrap();
        assert!(check(&copy.0).unwrap().is_empty());
    }

    #[test]
    fn a_platform_folder_outside_a_platform_crate_is_reported_once() {
        let copy = rust_tree_copy("planted-platform-dir");
        copy.write("rust/effects/craftbridge-effects/src/platform/mod.rs", "//! Platform code.\n");
        copy.write("rust/effects/craftbridge-effects/src/extra.rs", "//! More.\n");
        let errors = check(&copy.0).unwrap();
        assert_eq!(
            errors,
            [
                "rust/effects/craftbridge-effects/src/platform: craftbridge-effects is not a platform crate (rust/xtask/src/layers.rs), so it has no platform module"
            ]
        );
    }

    #[test]
    fn the_core_is_a_platform_crate_for_its_memory_module() {
        // Plan §16 puts the core's memory-pressure code in `craftbridge-core/src/platform/`.
        assert!(
            check_file(&file(
                "craftbridge-core/src/platform/memory.rs",
                "craftbridge-core",
                vec!["craftbridge-core", "src", "platform"],
                "// SAFETY: x\nunsafe {}\n"
            ))
            .is_empty()
        );
        assert!(lib_of("craftbridge-core", "#[allow(unsafe_code)]\nmod platform;\n").is_empty());
    }

    #[test]
    fn an_allow_spelt_as_the_compiler_also_reads_it_is_one() {
        for spelling in ["#![allow(r#unsafe_code)]\n", "# ![allow(unsafe_code)]\n", "#\n![allow(unsafe_code)]\n"] {
            let errors = lib_of("craftbridge-media", spelling);
            assert!(errors.iter().any(|e| e.contains(":1:") && e.contains("crate-wide")), "{spelling:?}: {errors:?}");
        }
        let outer = lib_of("craftbridge-media", "# [allow(r#unsafe_code)]\nfn f() {}\n");
        assert!(outer.iter().any(|e| e.contains("belongs only on a platform crate's `mod platform`")), "{outer:?}");
        // A lint a macro fills in may be `unsafe_code`; an attribute forwarded onto one item is not a level.
        let macro_lint = lib_of("craftbridge-media", "macro_rules! m { ($l:ident) => { #![allow($l)] }; }\n");
        assert!(!macro_lint.is_empty(), "{macro_lint:?}");
        assert!(lib_of("craftbridge-media", "macro_rules! m { ($(#[$a:meta])* $i:item) => { $(#[$a])* $i }; }\n").is_empty());
    }

    #[test]
    fn a_rust_file_in_no_crate_is_reported() {
        let copy = rust_tree_copy("planted-loose-file");
        copy.write(
            "rust/core/shared.rs",
            "#![allow(unsafe_code)]\npub fn read(value: &u8) -> u8 {\n    // SAFETY: a reference is valid.\n    unsafe { *std::ptr::from_ref(value) }\n}\n",
        );
        let errors = check(&copy.0).unwrap();
        assert!(errors.iter().any(|e| e.starts_with("rust/core/shared.rs: a Rust file in no crate's folder")), "{errors:?}");
    }

    #[test]
    fn a_file_belongs_to_the_nearest_crate_above_it() {
        let scratch = Scratch::new("owning-crate");
        scratch.write("core/craftbridge-media/Cargo.toml", "");
        scratch.write("xtask/Cargo.toml", "");
        let parts = |path: &str| path.split('/').map(str::to_string).collect::<Vec<_>>();
        let (name, dir) = owning_crate(&scratch.0, &parts("core/craftbridge-media/src/platform")).unwrap();
        assert_eq!(name, "craftbridge-media", "a crate under a grouping folder is named by its own folder");
        assert_eq!(dir, scratch.0.join("core/craftbridge-media"));
        assert_eq!(owning_crate(&scratch.0, &parts("xtask/src")).unwrap().0, "xtask");
        assert!(owning_crate(&scratch.0, &parts("core")).is_none(), "a grouping folder is not a crate");
        assert!(owning_crate(&scratch.0, &[]).is_none());
    }
}
