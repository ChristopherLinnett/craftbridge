//! Rule 7 — one copy of each upstream library crate.
//!
//! CraftBridge's decoders register themselves into the decoder registry EffectCraft decodes through,
//! and a registry is only one copy deep: if a second copy of an EffectCraft or FilmCraft crate lands in
//! the graph, the decoders register into the copy the decoder does not consult, and the broken
//! registration is silent — video that will not decode, with no error anywhere. So a duplicated name in
//! these families is an error, not a warning, and it is reported with the package ids so the two copies
//! (usually one from crates.io and one from the pinned git revision) can be told apart.
//!
//! A Craft *library* crate is exempt when every copy comes from the official Craft library repository:
//! the pinned upstream's own lock resolves some library crates (craft-media today) at two revisions,
//! because FilmCraft names a revision of its own, and this rule refusing that would refuse the official
//! pin itself. Which split passes is the `upstream` rule's to say, since only it reads upstream's lock:
//! it holds a library crate upstream's lock resolves to the sources it resolves that very crate to, and
//! any other library crate to one copy, so the gate passes exactly upstream's split and no other — not a
//! second revision of a crate upstream holds once, nor a revision whose commit upstream never locked. A second copy from anywhere else —
//! crates.io, a fork, a path — is this rule's business still, and every app crate (each Craft app's own
//! crates, where the decoder registry lives) stays strictly one.
//!
//! Other crates duplicate all the time in a graph this size and are cargo-deny's business (it warns);
//! only these prefixes are the ownership boundary this rule protects.

use crate::layers::{CraftFamily, family};
use crate::meta::Crate;
use std::path::Path;

/// Is the package `name` from `source` a crate of one of the families that must appear once: a Craft
/// app's (the layer table's list, by name or repository) or a shared Craft library? The one answer to
/// "which packages are Craft family" is [`crate::layers::family`], which the pin rule reads too.
fn owned(name: &str, source: Option<&str>) -> bool {
    family(name, source).is_some()
}

/// Are these copies of one name all git packages of the official Craft library repository, so their
/// split is upstream's own doing (see the module docs) rather than a second copy this repository added?
fn sanctioned_library_split(copies: &[&Crate]) -> bool {
    let prefix = crate::upstream::library_source_prefix();
    copies.iter().all(|package| {
        matches!(family(&package.name, package.source.as_deref()), Some(CraftFamily::Library))
            && package.source.as_deref().is_some_and(|source| source.starts_with(&prefix))
    })
}

/// Group packages by name and return every owned name that has more than one copy, with their ids and
/// what asks for each: a manifest in the repository by its path from `root`, any other package by its id,
/// so the message points at the line to change.
pub(crate) fn duplicate_ids(crates: &[Crate], root: &Path) -> Vec<String> {
    let mut names: Vec<&str> = crates.iter().filter(|package| owned(&package.name, package.source.as_deref())).map(|package| package.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    let mut errors = Vec::new();
    for name in names {
        let copies: Vec<&Crate> = crates.iter().filter(|package| package.name == name).collect();
        if copies.len() > 1 && !sanctioned_library_split(&copies) {
            let ids: Vec<String> = copies
                .iter()
                .map(|package| {
                    let requesters: Vec<String> = crates
                        .iter()
                        .filter(|requester| requester.depends_on.contains(&package.id))
                        .map(|requester| {
                            let manifest = Path::new(&requester.manifest_path);
                            if manifest.starts_with(root) { crate::report::shown(root, manifest) } else { requester.id.clone() }
                        })
                        .collect();
                    let asked = if requesters.is_empty() { String::new() } else { format!(", asked for by {}", requesters.join(", ")) };
                    format!("{} ({}{asked})", package.id, package.source.as_deref().unwrap_or("a local path"))
                })
                .collect();
            errors.push(format!("{} is in the graph {} times: {}", name, copies.len(), ids.join("; ")));
        }
    }
    errors
}

#[cfg(test)]
fn package(name: &str, id: &str, source: Option<&str>) -> Crate {
    Crate {
        name: name.to_string(),
        id: id.to_string(),
        source: source.map(str::to_string),
        edition: "2024".to_string(),
        target_editions: Vec::new(),
        manifest_path: format!("/repo/rust/{name}/Cargo.toml"),
        dependencies: Vec::new(),
        library_targets: Vec::new(),
        build_script: None,
        depends_on: Vec::new(),
        member: false,
        binary_targets: Vec::new(),
    }
}

/// Run the check against the resolved graph: `cargo metadata` with dependencies.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let crates = crate::meta::read(&root.join("rust"), crate::meta::Graph::Resolved)?;
    Ok(duplicate_ids(&crates, root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_copy_of_an_owned_crate_is_reported_with_its_ids() {
        let crates = vec![
            package(
                "effectcraft-engine",
                "registry+https://github.com/rust-lang/crates.io-index#effectcraft-engine@1.2.0",
                Some("registry+https://github.com/rust-lang/crates.io-index"),
            ),
            package(
                "effectcraft-engine",
                "git+https://github.com/storytold/effectcraft?rev=abcd#effectcraft-engine@1.3.0",
                Some("git+https://github.com/storytold/effectcraft?rev=abcd"),
            ),
            package("filmcraft-media", "path+file:///repo/rust/core/filmcraft-media#0.1.0", None),
        ];
        let errors = duplicate_ids(&crates, Path::new("/repo"));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("effectcraft-engine is in the graph 2 times"), "{errors:?}");
        assert!(errors[0].contains("crates.io-index") && errors[0].contains("storytold/effectcraft"), "both copies must be named: {errors:?}");
    }

    #[test]
    fn each_copy_names_the_manifest_that_asks_for_it() {
        let registry = "registry+https://github.com/rust-lang/crates.io-index#effectcraft-time@1.0.0";
        let git = "git+https://github.com/storytold/effectcraft?rev=abcd#effectcraft-time@1.1.0";
        let mut ours = package("craftbridge-effects", "path+file:///repo/rust/effects/craftbridge-effects#0.1.0", None);
        ours.manifest_path = "/repo/rust/effects/craftbridge-effects/Cargo.toml".to_string();
        ours.depends_on = vec![git.to_string()];
        let mut theirs = package("effectcraft-engine", "git+https://github.com/storytold/effectcraft?rev=abcd#effectcraft-engine@1.1.0", None);
        theirs.manifest_path = "/home/.cargo/git/checkouts/engine/Cargo.toml".to_string();
        theirs.depends_on = vec![registry.to_string()];
        let crates = vec![package("effectcraft-time", registry, Some("registry")), package("effectcraft-time", git, Some("git")), ours, theirs];
        let errors = duplicate_ids(&crates, Path::new("/repo"));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("asked for by rust/effects/craftbridge-effects/Cargo.toml"), "{errors:?}");
        assert!(errors[0].contains("asked for by git+https://github.com/storytold/effectcraft?rev=abcd#effectcraft-engine@1.1.0"), "{errors:?}");
    }

    #[test]
    fn one_copy_of_each_owned_crate_passes() {
        let crates = vec![
            package(
                "effectcraft-engine",
                "git+https://github.com/storytold/effectcraft?rev=abcd#effectcraft-engine@1.3.0",
                Some("git+https://github.com/storytold/effectcraft?rev=abcd"),
            ),
            package("filmcraft-media", "path+file:///repo/core/filmcraft-media#0.1.0", None),
            package("craft-core", "path+file:///repo/core/craft-core#0.1.0", None),
        ];
        assert!(duplicate_ids(&crates, Path::new("/repo")).is_empty());
    }

    #[test]
    fn crates_outside_the_owned_families_are_not_this_rule() {
        let crates = vec![
            package("libc", "registry+https://github.com/rust-lang/crates.io-index#libc@0.2.1", Some("registry+https://github.com/rust-lang/crates.io-index")),
            package("libc", "registry+https://github.com/rust-lang/crates.io-index#libc@0.1.0", Some("registry+https://github.com/rust-lang/crates.io-index")),
            package("craftbridge-effects", "path+file:///repo/effects/craftbridge-effects#0.1.0", None),
            package("effectcraftness", "path+file:///repo/x#1.0.0", None),
        ];
        assert!(
            duplicate_ids(&crates, Path::new("/repo")).is_empty(),
            "an unowned duplicate and a crate that merely starts like an owned prefix are both left alone"
        );
    }

    /// A `craft-media` package from one revision of the official Craft library repository.
    fn official_craft_media(rev: &str) -> Crate {
        let source = format!("git+https://github.com/storytold/craft-libs?rev={rev}");
        package("craft-media", &format!("{source}#craft-media@0.1.0"), Some(&source))
    }

    #[test]
    fn two_official_revisions_of_one_craft_library_crate_are_upstreams_split_not_ours() {
        let crates = vec![official_craft_media("aaaa"), official_craft_media("bbbb")];
        assert!(duplicate_ids(&crates, Path::new("/repo")).is_empty(), "the pinned upstream resolves craft-media at two revisions itself");
    }

    #[test]
    fn a_craft_library_crate_with_a_copy_from_elsewhere_is_still_a_second_copy() {
        let crates = vec![official_craft_media("aaaa"), package("craft-media", "path+file:///repo/vendor/craft-media#0.1.0", None)];
        let errors = duplicate_ids(&crates, Path::new("/repo"));
        assert_eq!(errors.len(), 1, "only the official repository's own split is exempt: {errors:?}");
    }

    /// The whole path on a copy of the real tree: a second copy of a Craft crate that only a feature brings
    /// in is in the graph the check reads, because it reads every feature's dependencies.
    #[test]
    fn a_second_copy_behind_a_feature_is_in_the_graph_read() {
        let copy = crate::scratch::rust_tree_copy("duplicates-feature");
        for (folder, version) in [("a", "0.1.0"), ("b", "0.2.0")] {
            copy.write(&format!("fixtures/{folder}/Cargo.toml"), format!("[package]\nname = \"craft-plant\"\nversion = \"{version}\"\nedition = \"2024\"\n"));
            copy.write(&format!("fixtures/{folder}/src/lib.rs"), "\n");
        }
        let manifest = copy.0.join("rust/core/craftbridge-core/Cargo.toml");
        let text = std::fs::read_to_string(&manifest).unwrap().replace(
            "[dependencies]\n",
            "[dependencies]\nplant-a = { package = \"craft-plant\", path = \"../../../fixtures/a\", optional = true }\nplant-b = { package = \"craft-plant\", path = \"../../../fixtures/b\", optional = true }\n",
        );
        std::fs::write(&manifest, format!("{text}\n[features]\npinned = [\"dep:plant-a\", \"dep:plant-b\"]\n")).unwrap();
        let locked = std::process::Command::new(env!("CARGO"))
            .current_dir(copy.0.join("rust"))
            .args(["metadata", "--offline", "--format-version", "1"])
            .output()
            .unwrap();
        assert!(locked.status.success(), "{}", String::from_utf8_lossy(&locked.stderr));
        let errors = check(&copy.0).unwrap();
        assert!(errors.len() == 1 && errors[0].starts_with("craft-plant is in the graph 2 times"), "{errors:?}");
    }
}
