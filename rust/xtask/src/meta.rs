//! The one reader of `cargo metadata`, shared by every check that needs the package graph.
//!
//! The checks need the graph with different breadth: the layer rules and the lint table read the
//! workspace's own packages and what each declares, the layer rules and the duplicate check read the whole
//! resolved graph as well. The flags differ, but the reading, the error text and the shape of a package are
//! defined once here.

use std::path::{Path, PathBuf};

/// A package as `cargo metadata` describes it, kept small: only the fields a rule needs.
pub(crate) struct Crate {
    pub(crate) name: String,
    /// The full package id, which is what distinguishes two copies of the same name.
    pub(crate) id: String,
    /// `None` for a path package; a source string for one from a registry or git.
    pub(crate) source: Option<String>,
    /// The edition Cargo resolves this package to compile under — its own `edition`, or the workspace's
    /// when the manifest says `edition.workspace = true`. Cargo's own answer, so no manifest spelling —
    /// the old `[project]` table included — hides an edition from the rules.
    pub(crate) edition: String,
    /// Each target's name and the edition Cargo compiles it under: the package's, unless the target's own
    /// table (`[lib]`, `[[bin]]`, `[[test]]`, `[[example]]`, `[[bench]]`) sets one, which Cargo only warns
    /// about.
    pub(crate) target_editions: Vec<(String, String)>,
    /// Absolute path of the crate's `Cargo.toml`.
    pub(crate) manifest_path: String,
    /// Every declared dependency, each marked when it is a dev-dependency: a test build ships nothing, so
    /// some rules leave those out, but a crate's tests are still the crate's code.
    pub(crate) dependencies: Vec<CrateDependency>,
    /// Library-like targets, which is where a `staticlib` or `cdylib` would be declared.
    pub(crate) library_targets: Vec<CrateTarget>,
    /// The build script Cargo found for the package, as the path Cargo gives it: a `build.rs` beside the
    /// manifest, or what `package.build` (or the old `[project]` table's `build`) names. Cargo's own answer,
    /// so no spelling of a manifest can hide one from the rules.
    pub(crate) build_script: Option<String>,
    /// The package ids this package's resolved dependencies are, when the metadata was read with them.
    pub(crate) depends_on: Vec<String>,
    /// A member of the workspace, as `cargo metadata`'s `workspace_members` lists them.
    pub(crate) member: bool,
    /// A default member of the workspace, what Cargo builds when no package is named, as `cargo metadata`'s
    /// `workspace_default_members` lists them. Cargo's own answer, so `default-members`' globs and paths
    /// are read as Cargo resolves them, and a workspace that leaves the list out, which makes every member
    /// a default one, is read as Cargo builds it.
    pub(crate) default_member: bool,
    /// The names of the package's binary targets, which are the commands it builds.
    pub(crate) binary_targets: Vec<String>,
}

pub(crate) struct CrateDependency {
    pub(crate) name: String,
    /// Absolute path for a workspace/path dependency, `None` for an external one.
    pub(crate) path: Option<String>,
    /// Where an external dependency comes from (`git+https://github.com/storytold/effectcraft?rev=…`, a
    /// registry), as the manifest declares it.
    pub(crate) source: Option<String>,
    /// A dev-dependency, built only for the crate's own tests, examples and benchmarks.
    pub(crate) dev: bool,
    /// A build-dependency, built only for the crate's build script, which runs on the build host.
    pub(crate) build: bool,
}

/// Every target kind a library can be built as. A crate whose only library type is `staticlib` reports
/// its target as kind `staticlib`, not `lib`, so filtering on `lib` alone would hide exactly the native
/// library the layer rules look for; a proc-macro library reports `proc-macro`, and its `[lib] name` is
/// the name code imports as surely as any other library's.
const LIBRARY_KINDS: &[&str] = &["lib", "rlib", "dylib", "staticlib", "cdylib", "proc-macro"];

pub(crate) struct CrateTarget {
    /// The target's name, which is the crate name code imports it by and the file name of what it builds.
    pub(crate) name: String,
    /// `crate_types` in cargo metadata is a string array.
    pub(crate) crate_types: Vec<String>,
}

/// The JSON `cargo metadata` prints.
type Metadata = serde_json::Value;

/// Parse the metadata a cargo run printed.
fn parse(stdout: &[u8]) -> Result<Metadata, String> {
    serde_json::from_slice(stdout).map_err(|error| format!("cargo metadata printed JSON this check cannot read: {error}"))
}

/// Turn the `packages` array into the small shape the rules use.
fn packages(metadata: &Metadata) -> Result<Vec<Crate>, String> {
    let list = metadata.get("packages").and_then(Metadata::as_array).ok_or("cargo metadata has no `packages` array")?;
    let ids = |key: &str| metadata.get(key).and_then(Metadata::as_array).map(Vec::as_slice).unwrap_or_default();
    let (members, default_members) = (ids("workspace_members"), ids("workspace_default_members"));
    let mut crates = Vec::with_capacity(list.len());
    for package in list {
        let text = |key: &str| package.get(key).and_then(Metadata::as_str).unwrap_or_default().to_string();
        let source = package.get("source").and_then(Metadata::as_str).map(str::to_string);
        let dependencies = package
            .get("dependencies")
            .and_then(Metadata::as_array)
            .map(|deps| {
                deps.iter()
                    .map(|dep| {
                        let kind = dep.get("kind").and_then(Metadata::as_str);
                        CrateDependency {
                            name: dep.get("name").and_then(Metadata::as_str).unwrap_or_default().to_string(),
                            path: dep.get("path").and_then(Metadata::as_str).map(str::to_string),
                            source: dep.get("source").and_then(Metadata::as_str).map(str::to_string),
                            dev: kind == Some("dev"),
                            build: kind == Some("build"),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let targets = package.get("targets").and_then(Metadata::as_array);
        let has_kind = |target: &Metadata, wanted: &dyn Fn(&str) -> bool| {
            target.get("kind").and_then(Metadata::as_array).is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str().is_some_and(wanted)))
        };
        let build_script = targets
            .into_iter()
            .flatten()
            .find(|target| has_kind(target, &|kind| kind == "custom-build"))
            .map(|target| target.get("src_path").and_then(Metadata::as_str).unwrap_or("build.rs").to_string());
        let library_targets = targets
            .into_iter()
            .flatten()
            .filter(|target| has_kind(target, &|kind| LIBRARY_KINDS.contains(&kind)))
            .map(|target| CrateTarget {
                name: target.get("name").and_then(Metadata::as_str).unwrap_or_default().to_string(),
                crate_types: target
                    .get("crate_types")
                    .and_then(Metadata::as_array)
                    .map(|types| types.iter().filter_map(Metadata::as_str).map(str::to_string).collect())
                    .unwrap_or_default(),
            })
            .collect();
        let target_editions = targets
            .into_iter()
            .flatten()
            .map(|target| {
                let field = |key: &str| target.get(key).and_then(Metadata::as_str).unwrap_or_default().to_string();
                (field("name"), field("edition"))
            })
            .collect();
        let binary_targets = targets
            .into_iter()
            .flatten()
            .filter(|target| has_kind(target, &|kind| kind == "bin"))
            .map(|target| target.get("name").and_then(Metadata::as_str).unwrap_or_default().to_string())
            .collect();
        let id = text("id");
        let depends_on = resolved_dependencies(metadata, &id);
        let listed = |list: &[Metadata]| list.iter().any(|listed| listed.as_str() == Some(id.as_str()));
        let (member, default_member) = (listed(members), listed(default_members));
        crates.push(Crate {
            name: text("name"),
            id,
            source,
            edition: text("edition"),
            target_editions,
            manifest_path: text("manifest_path"),
            dependencies,
            library_targets,
            build_script,
            depends_on,
            member,
            default_member,
            binary_targets,
        });
    }
    Ok(crates)
}

/// The package ids the resolved graph says `id` depends on; empty when the metadata has no graph.
fn resolved_dependencies(metadata: &Metadata, id: &str) -> Vec<String> {
    let nodes = metadata.get("resolve").and_then(|resolve| resolve.get("nodes")).and_then(Metadata::as_array);
    nodes
        .into_iter()
        .flatten()
        .find(|node| node.get("id").and_then(Metadata::as_str) == Some(id))
        .and_then(|node| node.get("dependencies").and_then(Metadata::as_array))
        .map(|ids| ids.iter().filter_map(Metadata::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

/// Which graph a check reads.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Graph {
    /// The workspace's own packages and what each declares (`--no-deps`): far less for cargo to resolve.
    Workspace,
    /// The whole graph Cargo resolves with every feature of every member turned on (`--all-features`), so
    /// a package a feature, a `[patch]` or a `[replace]` brings in is in it: code behind a feature is code
    /// that ships to whoever turns the feature on, as clippy and cargo-deny read it too.
    Resolved,
}

/// Run cargo metadata and parse what it printed. `--locked` because the gate reads the graph `Cargo.lock`
/// pins and never writes it: a lock file that needs updating fails here instead of changing under the
/// change being checked. The run starts through `crate::pin`, like every cargo a tool starts: with the
/// pinned toolchain named, and in the lint library's folder with that folder's own (AGENTS.md §7).
fn run(rust_dir: &Path, graph: Graph) -> Result<Metadata, String> {
    let mut command = crate::pin::cargo_in(rust_dir)?;
    command.arg("metadata").arg("--format-version").arg("1").arg("--locked");
    command.arg(match graph {
        Graph::Workspace => "--no-deps",
        Graph::Resolved => "--all-features",
    });
    let output = command.output().map_err(|error| format!("could not run `{}`: {error}", Path::new(command.get_program()).display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("cargo metadata exited with {}: {}", output.status, stderr.trim()));
    }
    parse(&output.stdout)
}

/// Run cargo metadata and read its packages.
pub(crate) fn read(rust_dir: &Path, graph: Graph) -> Result<Vec<Crate>, String> {
    packages(&run(rust_dir, graph)?)
}

/// The folder cargo builds the workspace in, as `cargo metadata` names it. `cargo dylint` takes the folder
/// it builds the lint library under from the same answer for the same workspace, so Cargo's own answer is
/// the one that agrees with it however the folder is set: `CARGO_TARGET_DIR`, which `tools/env.sh` sets
/// for every gate, the configuration's `build.target-dir`, or the workspace's `target/`.
pub(crate) fn target_directory(rust_dir: &Path) -> Result<PathBuf, String> {
    target_directory_of(&run(rust_dir, Graph::Workspace)?)
}

fn target_directory_of(metadata: &Metadata) -> Result<PathBuf, String> {
    metadata.get("target_directory").and_then(Metadata::as_str).map(PathBuf::from).ok_or_else(|| "cargo metadata has no `target_directory`".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The minimum JSON `cargo metadata` prints for one package, plus a dev-dependency, a build-dependency
    /// and a non-library target the reader must drop.
    const ONE_PACKAGE: &str = r#"{
        "packages": [{
            "name": "craftbridge-core", "version": "0.1.0", "id": "path+file:///x/craftbridge-core#0.1.0",
            "manifest_path": "/x/craftbridge-core/Cargo.toml", "edition": "2024",
            "dependencies": [
                {"name": "craftbridge-media", "kind": null, "path": "/x/craftbridge-media"},
                {"name": "insta", "kind": "dev"},
                {"name": "uniffi", "kind": "build"}
            ],
            "targets": [
                {"kind": ["lib"], "name": "craftbridge_core", "crate_types": ["lib", "cdylib"], "edition": "2024"},
                {"kind": ["bin"], "name": "core-tool", "crate_types": ["bin"], "edition": "2021"}
            ]
        }],
        "workspace_members": ["path+file:///x/craftbridge-core#0.1.0"], "resolve": null, "target_directory": "", "version": 1, "workspace_root": "/x"
    }"#;

    #[test]
    fn dev_and_build_dependencies_are_marked_and_non_library_targets_dropped() {
        let parsed = packages(&serde_json::from_str::<Metadata>(ONE_PACKAGE).unwrap()).unwrap();
        assert_eq!(parsed.len(), 1);
        let package = &parsed[0];
        assert_eq!(package.source, None);
        assert_eq!(package.id, "path+file:///x/craftbridge-core#0.1.0");
        assert_eq!(package.edition, "2024");
        let editions: Vec<(&str, &str)> = package.target_editions.iter().map(|(name, edition)| (name.as_str(), edition.as_str())).collect();
        assert_eq!(editions, [("craftbridge_core", "2024"), ("core-tool", "2021")], "each target's own edition, as Cargo resolved it");
        let names: Vec<(&str, bool, bool)> = package.dependencies.iter().map(|dep| (dep.name.as_str(), dep.dev, dep.build)).collect();
        assert_eq!(names, [("craftbridge-media", false, false), ("insta", true, false), ("uniffi", false, true)]);
        assert_eq!(package.dependencies[0].path.as_deref(), Some("/x/craftbridge-media"));
        assert_eq!(package.library_targets.len(), 1);
        assert_eq!(package.library_targets[0].crate_types, ["lib".to_string(), "cdylib".to_string()]);
        assert_eq!(package.library_targets[0].name, "craftbridge_core");
        assert_eq!(package.build_script, None);
        assert!(package.member);
        assert_eq!(package.binary_targets, ["core-tool"]);
    }

    #[test]
    fn a_proc_macro_library_is_a_library_target_and_a_declared_source_is_kept() {
        let json = r#"{"packages": [{"name": "a", "id": "a#1", "version": "1", "manifest_path": "/a/Cargo.toml",
            "dependencies": [{"name": "posterize-bands", "kind": null, "source": "git+https://github.com/storytold/effectcraft?rev=1#1"}],
            "targets": [{"kind": ["proc-macro"], "name": "film_x", "crate_types": ["proc-macro"]}]}], "workspace_members": []}"#;
        let parsed = packages(&serde_json::from_str::<Metadata>(json).unwrap()).unwrap();
        assert_eq!(parsed[0].library_targets.len(), 1);
        assert_eq!(parsed[0].library_targets[0].name, "film_x");
        assert_eq!(parsed[0].dependencies[0].source.as_deref(), Some("git+https://github.com/storytold/effectcraft?rev=1#1"));
        assert!(!parsed[0].member);
    }

    /// A member Cargo leaves out of `workspace_default_members` is a member and no default one, and a
    /// package in no list is neither.
    #[test]
    fn a_default_member_is_the_one_cargo_lists_as_one() {
        let json = r#"{"packages": [
            {"name": "shipped", "id": "shipped#1", "version": "1", "manifest_path": "/x/shipped/Cargo.toml"},
            {"name": "tool", "id": "tool#1", "version": "1", "manifest_path": "/x/tool/Cargo.toml"},
            {"name": "outside", "id": "outside#1", "version": "1", "manifest_path": "/y/outside/Cargo.toml"}],
            "workspace_members": ["shipped#1", "tool#1"], "workspace_default_members": ["shipped#1"]}"#;
        let parsed = packages(&serde_json::from_str::<Metadata>(json).unwrap()).unwrap();
        let flags: Vec<(&str, bool, bool)> = parsed.iter().map(|package| (package.name.as_str(), package.member, package.default_member)).collect();
        assert_eq!(flags, [("shipped", true, true), ("tool", true, false), ("outside", false, false)]);
    }

    #[test]
    fn a_build_script_and_the_resolved_graph_are_cargo_s_answers() {
        let json = r#"{"packages": [{"name": "a", "id": "a#1", "version": "1", "manifest_path": "/a/Cargo.toml",
            "targets": [{"kind": ["lib"], "name": "a", "crate_types": ["lib"]}, {"kind": ["custom-build"], "name": "build-script-gen", "src_path": "/a/gen.rs"}]}],
            "resolve": {"nodes": [{"id": "a#1", "dependencies": ["b#2"]}]}}"#;
        let parsed = packages(&serde_json::from_str::<Metadata>(json).unwrap()).unwrap();
        assert_eq!(parsed[0].build_script.as_deref(), Some("/a/gen.rs"));
        assert_eq!(parsed[0].depends_on, ["b#2"]);
    }

    #[test]
    fn a_library_built_only_as_a_staticlib_is_still_a_library_target() {
        let json = r#"{"packages": [{"name": "a", "id": "a#1", "version": "1", "manifest_path": "/a/Cargo.toml",
            "targets": [{"kind": ["staticlib"], "crate_types": ["staticlib"]}, {"kind": ["custom-build"], "crate_types": ["bin"]}]}]}"#;
        let parsed = packages(&serde_json::from_str::<Metadata>(json).unwrap()).unwrap();
        assert_eq!(parsed[0].library_targets.len(), 1);
        assert_eq!(parsed[0].library_targets[0].crate_types, ["staticlib".to_string()]);
    }

    #[test]
    fn a_package_without_optional_fields_still_reads() {
        let json = r#"{"packages": [{"name": "a", "id": "a#1", "version": "1", "manifest_path": "/a/Cargo.toml"}]}"#;
        let parsed = packages(&serde_json::from_str::<Metadata>(json).unwrap()).unwrap();
        assert!(parsed[0].dependencies.is_empty() && parsed[0].library_targets.is_empty() && parsed[0].source.is_none());
    }

    #[test]
    fn the_build_folder_is_the_one_cargo_names() {
        let named = serde_json::json!({"target_directory": "/push/target-1"});
        assert_eq!(target_directory_of(&named).unwrap(), PathBuf::from("/push/target-1"));
        assert_eq!(target_directory_of(&serde_json::json!({})).unwrap_err(), "cargo metadata has no `target_directory`");
    }

    #[test]
    fn unreadable_metadata_is_an_error_not_a_panic() {
        assert!(parse(b"not json").is_err());
        assert!(packages(&serde_json::json!({})).is_err());
    }
}
