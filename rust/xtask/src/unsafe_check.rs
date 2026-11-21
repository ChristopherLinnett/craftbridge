//! Rule 4 — where `unsafe` may appear at all.
//!
//! Hand-written `unsafe` is the one place the language stops helping, so it is confined to a platform
//! module of a crate the layer table marks as a platform crate: one small, reviewable surface per
//! platform instead of a keyword that can appear anywhere. The three tolerances are deliberate and
//! narrow — UniFFI's generated scaffolding needs `unsafe_code` off for a whole `-ffi` crate, a
//! `mod platform` declaration needs it off to let its module contain blocks, and the gate's own lint
//! library (`AGENTS.md §10`) exports its two dylint entry points by symbol name, which an edition-2024
//! attribute written with this keyword (`#[unsafe(no_mangle)]`) is the only way to write and an item-level
//! allowance beside it the only level that lets it compile. Anything else is reported.
//!
//! The keyword is found as `syn` parses it (`crate::rust_syntax`): blocks, functions, impls, traits,
//! extern blocks and attributes, and inside a macro's tokens, so one quoted in a comment, a doc block or
//! a string is no violation, and a real one is not missed because a string ended early.

use crate::attributes::{self, Place};
use crate::rust_syntax::{self, Facts};
use crate::{layers, machinery, report, scan};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The lint this rule governs.
const UNSAFE_CODE: &str = "unsafe_code";

/// The one module of a platform crate that may switch it off.
const PLATFORM_MODULE: &str = "platform";

/// The two items a dylint library exports by symbol name, the names the driver looks them up by, and the
/// one attribute that exports them: the only place outside a platform module this keyword may be written.
const DYLINT_EXPORTS: &[&str] = &["dylint_version", "register_lints"];

/// Is this crate the gate's lint library? Its folder is the last component of the machinery path.
fn is_lint_library(crate_name: &str) -> bool {
    machinery::LINT_LIBRARY.trim_end_matches('/').rsplit('/').next().is_some_and(|folder| folder == crate_name)
}

/// Is `name` one of the two functions a dylint library exports?
fn is_dylint_export(name: &str) -> bool {
    DYLINT_EXPORTS.contains(&name)
}

/// One `.rs` file, reduced to what this rule reads.
pub(crate) struct RustFile<'a> {
    /// How a message names the file.
    pub(crate) path: &'a str,
    /// The crate it belongs to, by the name of its folder under `rust/`.
    pub(crate) crate_name: &'a str,
    /// Directory components between the crate's own folder and the file, so the platform module is the
    /// crate's own `src/platform/`, matched as components rather than a substring someone could fake with
    /// `src/platformkit`, and not a `src/platform` pair found deeper (`tests/src/platform/`).
    pub(crate) components: Vec<&'a str>,
    /// Whether this is the crate's own `src/lib.rs`, the only place a crate-wide attribute belongs.
    pub(crate) is_lib_root: bool,
    /// What the file's syntax tree holds.
    pub(crate) facts: Facts,
}

/// Check one file's syntax against what its crate's row allows.
pub(crate) fn check_file(file: &RustFile<'_>) -> Vec<String> {
    let entry = layers::entry(file.crate_name);
    let platform_crate = entry.is_some_and(|entry| entry.platform);
    let ffi_crate = entry.is_some_and(|entry| entry.ffi);
    let lint_library = is_lint_library(file.crate_name);
    let in_platform_module = file.components.starts_with(&["src", PLATFORM_MODULE]);
    let mut errors = Vec::new();

    if !(in_platform_module && platform_crate) {
        let lines: BTreeSet<usize> = file
            .facts
            .unsafe_uses
            .iter()
            .filter(|used| !(lint_library && used.exports.as_deref().is_some_and(is_dylint_export)))
            .map(|used| used.line)
            .collect();
        for line in lines {
            errors.push(format!("{}:{line}: unsafe outside a platform module", file.path));
        }
    }

    for allowance in &file.facts.allowances {
        // A lint a macro fills in may be this one, so it is read as this one.
        if !allowance.lints.iter().any(|lint| lint == UNSAFE_CODE || lint == attributes::UNREADABLE) {
            continue;
        }
        let at = format!("{}:{}: {}({UNSAFE_CODE})", file.path, allowance.line, allowance.level);
        match &allowance.place {
            // A whole attribute a macro forwards onto one item is the forwarding idiom, not a level.
            Place::Function(_) | Place::Other if allowance.level == attributes::MACRO_ATTRIBUTE => {}
            Place::Inner => {
                if !(ffi_crate && file.is_lib_root) {
                    errors.push(format!("{at} crate-wide belongs only to an -ffi crate's own src/lib.rs"));
                }
            }
            Place::Module(module) => {
                if !(platform_crate && module == PLATFORM_MODULE && file.is_lib_root) {
                    errors.push(format!("{at} belongs only on a platform crate's `mod {PLATFORM_MODULE}` declaration in its src/lib.rs"));
                }
            }
            Place::Unknown => errors.push(format!(
                "{at} on what a macro's tokens do not show, which may be a module: it belongs only on a platform crate's `mod {PLATFORM_MODULE}` declaration in its src/lib.rs"
            )),
            Place::Function(name) if lint_library && is_dylint_export(name) => {}
            Place::Function(_) | Place::Other => {
                errors.push(format!("{at} on one item belongs only to the gate's lint library, on its two dylint exports"));
            }
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
        let inside: Vec<String> = file
            .path
            .parent()
            .and_then(|parent| parent.strip_prefix(&crate_dir).ok())
            .map(|relative| relative.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        let text = scan::read_text(&file.path)?;
        crates.insert(crate_name.clone(), crate_dir.clone());
        let facts = match rust_syntax::read(&text, &shown) {
            Ok(facts) => facts,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        let input = RustFile {
            path: &shown,
            crate_name: &crate_name,
            components: inside.iter().map(String::as_str).collect(),
            is_lib_root: file.path == crate_dir.join("src").join("lib.rs"),
            facts,
        };
        errors.extend(check_file(&input));
    }
    errors.extend(platform_dir_errors(root, &crates));
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::{Scratch, rust_tree_copy};

    fn file<'a>(path: &'a str, crate_name: &'a str, components: Vec<&'a str>, source: &'a str) -> RustFile<'a> {
        RustFile { path, crate_name, components, is_lib_root: path.ends_with("/src/lib.rs"), facts: rust_syntax::read(source, path).unwrap() }
    }

    /// The rule's verdict on `source` as the `src/lib.rs` of the crate `crate_name`.
    fn lib_of(crate_name: &str, source: &str) -> Vec<String> {
        let path = format!("{crate_name}/src/lib.rs");
        check_file(&file(&path, crate_name, vec!["src"], source))
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
        let errors = check_file(&file("craftbridge-media/src/platform/decode.rs", "craftbridge-media", vec!["src", "platform"], source));
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
        let wrong_crate = "fn f() {\n    unsafe { }\n}\n";
        let errors = check_file(&file("craftbridge-effects/src/platform/x.rs", "craftbridge-effects", vec!["src", "platform"], wrong_crate));
        assert!(errors.iter().any(|e| e.contains("unsafe outside a platform module")), "{errors:?}");
    }

    #[test]
    fn a_crate_wide_allow_only_suits_an_ffi_crate_root() {
        let source = format!("#![allow({UNSAFE_CODE})]\n\nfn f() {{}}\n");
        let ffi = check_file(&file("craftbridge-core-ffi/src/lib.rs", "craftbridge-core-ffi", vec!["src"], &source));
        assert!(ffi.is_empty(), "{ffi:?}");

        assert!(media_lib(&source).iter().any(|e| e.contains("-ffi crate's own src/lib.rs")));

        let wrong_file = check_file(&file("craftbridge-core-ffi/src/main.rs", "craftbridge-core-ffi", vec!["src"], &source));
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
        // An allowance on a module that is not the platform one, and an allowance on one item, which is
        // the lint library's shape and nobody else's.
        assert!(media_lib("fn f() {}\n#[allow(unsafe_code)]\nfn g() {}\n").iter().any(|e| e.contains("one item")));
        for elsewhere in ["#[allow(unsafe_code)]\nmod platformkit;\n", "#[allow(unsafe_code)]\nmod other;\nmod platform;\n"] {
            assert!(media_lib(elsewhere).iter().any(|e| e.contains("mod platform")), "{elsewhere:?}");
        }
        let wrong_crate = plugin_lib("#[allow(unsafe_code)]\npub mod platform;\n");
        assert!(!wrong_crate.is_empty(), "{wrong_crate:?}");
    }

    /// The gate's own lint library exports its two dylint entry points by symbol name, as dylint loads
    /// a library by name: those two items write the keyword's attribute form and allow the lint beside
    /// it, and the library has that tolerance and no other.
    #[test]
    fn the_lint_library_writes_the_keyword_only_for_its_dylint_exports() {
        let exports = [
            "#[allow(unsafe_code)]\n#[unsafe(no_mangle)]\npub extern \"C\" fn dylint_version() -> *mut std::os::raw::c_char { }\n",
            "#[unsafe(no_mangle)]\n#[allow(unsafe_code)]\npub fn register_lints(_sess: &Session, store: &mut LintStore) { }\n",
            "#[allow(unsafe_code)] #[unsafe ( no_mangle )]\npub extern \"C\"\nfn dylint_version() { }\n",
        ];
        for export in exports {
            assert!(lib_of("never-crash-lints", export).is_empty(), "{export:?}: {:?}", lib_of("never-crash-lints", export));
        }

        // The two exports are the tolerance's whole extent: the same attribute and allowance on any other
        // item, an unsafe block, any other unsafe attribute, and a crate-wide allowance are each still
        // reported in this crate as in any other.
        for other_item in [
            "#[allow(unsafe_code)]\n#[unsafe(no_mangle)]\npub extern \"C\" fn planted_symbol() { }\n",
            "#[allow(unsafe_code)]\n#[unsafe(no_mangle)]\npub static DYLINT_VERSION: u8 = 0;\n",
            "#[allow(unsafe_code)]\n#[unsafe(no_mangle)]\npub extern \"C\" fn dylint_version_too() { }\n",
        ] {
            let errors = lib_of("never-crash-lints", other_item);
            assert!(errors.iter().any(|e| e.ends_with(":2: unsafe outside a platform module")), "{other_item:?}: {errors:?}");
            assert!(errors.iter().any(|e| e.contains("on its two dylint exports")), "{other_item:?}: {errors:?}");
        }
        let allowed_elsewhere = lib_of("never-crash-lints", "#[allow(unsafe_code)]\nfn helper() { }\n");
        assert!(allowed_elsewhere.iter().any(|e| e.contains("on its two dylint exports")), "{allowed_elsewhere:?}");
        let block = "fn f() {\n    unsafe { }\n}\n";
        assert!(lib_of("never-crash-lints", block).iter().any(|e| e.ends_with(":2: unsafe outside a platform module")), "{block:?}");
        let other = "#![allow(unsafe_code)]\n#[unsafe(export_name = \"x\")]\nfn f() {}\n";
        assert_eq!(lib_of("never-crash-lints", other).len(), 2, "{other:?}");
        // And the exports' shape belongs to no other crate: the keyword count does not save it either.
        let elsewhere = plugin_lib("#[allow(unsafe_code)]\n#[unsafe(no_mangle)]\npub extern \"C\" fn dylint_version() { }\n");
        assert!(elsewhere.iter().any(|e| e.contains("one item")) && elsewhere.iter().any(|e| e.contains("unsafe outside")), "{elsewhere:?}");
    }

    #[test]
    fn only_the_crate_s_own_platform_module_may_hold_unsafe() {
        let source = "pub fn read() -> u8 {\n    // SAFETY: the pointer comes from the caller.\n    unsafe { 0 }\n}\n";
        for deeper in [vec!["tests", "src", "platform"], vec!["src", "x", "src", "platform"]] {
            let errors = check_file(&file("craftbridge-media/tests/src/platform/mod.rs", "craftbridge-media", deeper.clone(), source));
            assert!(errors.iter().any(|e| e.ends_with("unsafe outside a platform module")), "{deeper:?}: {errors:?}");
        }
        let declared_deeper = check_file(&file(
            "craftbridge-media/tests/src/mod.rs",
            "craftbridge-media",
            vec!["tests", "src"],
            "#[allow(unsafe_code)]\npub(crate) mod platform;\n",
        ));
        assert!(declared_deeper.iter().any(|e| e.contains("in its src/lib.rs")), "{declared_deeper:?}");
    }

    #[test]
    fn a_quoted_or_commented_unsafe_does_not_count() {
        let source = "//! the keyword this rule confines is quoted in docs all the time: unsafe\nfn f() {\n    let example = \"unsafe\";\n    // unsafe\n    /* unsafe */\n    let raw = r\"unsafe\";\n    let hint = \"#![allow(unsafe_code)]\";\n}\n";
        let errors = plugin_lib(source);
        assert!(errors.is_empty(), "{errors:?}");
        // And the keyword still counts on the line after a multi-line comment.
        let after = "/* doc\nunsafe */\nfn f() { unsafe { } }\n";
        let errors = plugin_lib(after);
        assert!(errors.iter().any(|e| e.contains(":3:")), "{errors:?}");
        // Every other literal holds it as text too, and the line a report names is the keyword's own,
        // past a literal that spans lines.
        let literals = "fn f() {\n    let r = r#\"\nunsafe \"x\" unsafe\n\"#;\n    let s = \"first\nunsafe\";\n    let b = b\"un\\\"safe\";\n    let c = c\"unsafe\";\n    let br = br#\"unsafe\"#;\n    let q = '\"'; unsafe { }\n}\n";
        assert_eq!(plugin_lib(literals), ["craftbridge-effects/src/lib.rs:10: unsafe outside a platform module"]);
    }

    #[test]
    fn the_keyword_is_found_wherever_syn_finds_it() {
        for source in [
            "fn f() { unsafe { } }\n",
            "unsafe fn f() {}\n",
            "struct S;\nunsafe impl Send for S {}\n",
            "unsafe trait T {}\n",
            "trait T { unsafe fn f(); }\n",
            "struct S;\nimpl S { unsafe fn f() {} }\n",
            "unsafe extern \"C\" { fn f(); }\n",
            "type F = unsafe fn();\n",
            "type F = unsafe extern \"C\" fn();\n",
            "#[unsafe(no_mangle)]\nfn f() {}\n",
            "#[cfg_attr(all(), unsafe(export_name = \"x\"))]\nfn f() {}\n",
            "static S: u8 = { unsafe { 0 } };\n",
            "macro_rules! m { () => { unsafe { } }; }\n",
            "fn f() { m!(unsafe { }); }\n",
        ] {
            let errors = plugin_lib(source);
            assert!(errors.iter().any(|e| e.contains("unsafe outside a platform module")), "{source:?}: {errors:?}");
        }
        for source in ["fn unsafe_len() {}\nstruct Unsafe;\nfn f() { let r#unsafe = 1; }\n", "extern \"C\" { fn f(); }\n"] {
            assert!(plugin_lib(source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn a_file_syn_cannot_parse_is_reported_naming_it() {
        let copy = rust_tree_copy("planted-unparsable");
        copy.write("rust/core/craftbridge-media/src/broken.rs", "fn f( {\n");
        let errors = check(&copy.0).unwrap();
        assert!(errors.iter().any(|e| e.starts_with("rust/core/craftbridge-media/src/broken.rs:1: not Rust that syn can parse")), "{errors:?}");
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

    /// An `unsafe` the walk reaches but git keeps out of `git add .` is read by no rule, at any depth and
    /// from whichever ignore source: a root pattern, a folder's own `.gitignore`, and the checkout's own
    /// `info/exclude`. What git would add is read as ever: a copy tracked past the pattern is refused again.
    #[test]
    fn an_ignored_rust_file_holding_unsafe_is_read_by_no_rule() {
        let copy = rust_tree_copy("planted-unsafe-ignored");
        crate::git::output(&copy.0, &["init", "-q"]).unwrap();
        let planted = "unsafe fn read(raw: *const u8) -> u8 {\n    // SAFETY: a planted block, but not in the platform module.\n    *raw\n}\n";
        copy.write(".gitignore", "/rust/toolcache/\n");
        copy.write("rust/toolcache/loose.rs", planted);
        copy.write("rust/core/craftbridge-core/src/toolcache/.gitignore", "*\n");
        copy.write("rust/core/craftbridge-core/src/toolcache/loose.rs", planted);
        copy.write(".git/info/exclude", "/rust/planted-loose/\n");
        copy.write("rust/planted-loose/loose.rs", planted);
        assert!(check(&copy.0).unwrap().is_empty(), "an unsafe git ignores belongs to no crate anyone must guard");
        crate::git::output(&copy.0, &["add", "-f", "rust/toolcache/loose.rs"]).unwrap();
        let errors = check(&copy.0).unwrap();
        assert_eq!(errors.len(), 1, "tracked, the planted file is read again: {errors:?}");
        assert!(errors[0].starts_with("rust/toolcache/loose.rs:"), "{errors:?}");
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
            check_file(&file("craftbridge-core/src/platform/memory.rs", "craftbridge-core", vec!["src", "platform"], "fn f() {\n// SAFETY: x\nunsafe {}\n}\n"))
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
        assert!(outer.iter().any(|e| e.contains("one item")), "{outer:?}");
        // A lint a macro fills in may be `unsafe_code`; an attribute its caller wrote, forwarded onto one
        // item, is not a level.
        let macro_lint = lib_of("craftbridge-media", "macro_rules! m { ($l:ident) => { #![allow($l)] }; }\n");
        assert!(!macro_lint.is_empty(), "{macro_lint:?}");
        assert!(lib_of("craftbridge-media", "macro_rules! m { ($(#[$a:meta])* $v:vis fn $n:ident) => { $(#[$a])* $v fn $n() {} }; }\n").is_empty());
    }

    #[test]
    fn an_allow_a_macro_puts_where_its_tokens_do_not_show_may_be_on_a_module() {
        for source in [
            "macro_rules! m { ($i:item) => { #[allow(unsafe_code)] $i }; }\n",
            "macro_rules! m { ($k:tt $n:ident) => { #[allow(unsafe_code)] $k $n {} }; }\n",
            "macro_rules! m { ($(#[$a:meta])* $i:item) => { $(#[$a])* $i }; }\n",
            "macro_rules! m { ($a:meta) => { #[cfg_attr(all(), $a)] fn f() {} }; }\n",
            // A macro that fills in the attribute's own name, or all of it, from tokens.
            "macro_rules! m { ($($a:tt)*) => { #[$($a)*] mod inner {} }; }\n",
            "macro_rules! m { ($n:ident $args:tt) => { #[$n $args] fn f() {} }; }\n",
            "macro_rules! m { ($args:tt) => { #[allow $args] mod inner {} }; }\n",
            // A `#` and brackets the macro is handed apart, which the compiler makes one attribute.
            "macro_rules! m { ($p:tt, $a:meta) => { $p [$a] mod inner {} }; }\nm!(#, allow(unsafe_code));\n",
            // An attribute written in a call, on the item the caller wrote after it, which the macro may move.
            "macro_rules! m { ($p:tt $b:tt $($rest:tt)*) => { $($rest)* $p $b mod inner {} }; }\nm!(#[allow(unsafe_code)] fn z() {});\n",
        ] {
            let errors = lib_of("craftbridge-media", source);
            assert!(errors.iter().any(|e| e.contains("(unsafe_code)")), "{source:?}: {errors:?}");
        }
        // The allowance a dylint export may carry, written in a call on that export, is one the macro
        // may move onto a module.
        let moved = "macro_rules! m { ($p:tt $b:tt $($rest:tt)*) => { $($rest)* $p $b mod inner {} }; }\nm!(#[allow(unsafe_code)] fn dylint_version() {});\n";
        let errors = lib_of("never-crash-lints", moved);
        assert!(errors.iter().any(|e| e.contains("on what a macro's tokens do not show")), "{errors:?}");
        assert!(lib_of("never-crash-lints", "#[allow(unsafe_code)]\nfn dylint_version() {}\n").is_empty(), "written as itself it is the export's");
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
