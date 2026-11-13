//! Rule 7 — one copy of each upstream library crate.
//!
//! CraftBridge's decoders register themselves into the decoder registry EffectCraft decodes through,
//! and a registry is only one copy deep: if a second copy of an EffectCraft, FilmCraft or Craft crate
//! lands in the graph, the decoders register into the copy the decoder does not consult, and the broken
//! registration is silent — video that will not decode, with no error anywhere. So a duplicated name in
//! these three families is an error, not a warning, and it is reported with the package ids so the two
//! copies (usually one from crates.io and one from the pinned git revision) can be told apart.
//!
//! Other crates duplicate all the time in a graph this size and are cargo-deny's business (it warns);
//! only these prefixes are the ownership boundary this rule protects.

use crate::layers::{CRAFT_APPS, CRAFT_LIBRARY_PREFIX, CRAFT_LIBRARY_REPOSITORY};
use crate::meta::Crate;
use std::path::Path;

/// Is the package `name` from `source` a crate of one of the families that must appear once: a Craft
/// app's (the layer table's list, by name or repository) or a shared Craft library?
fn owned(name: &str, source: Option<&str>) -> bool {
    name.starts_with(CRAFT_LIBRARY_PREFIX)
        || source.is_some_and(|source| crate::layers::from_repository(source, CRAFT_LIBRARY_REPOSITORY))
        || CRAFT_APPS.iter().any(|app| app.owns(name, source))
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
        if copies.len() > 1 {
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
