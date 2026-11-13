//! The one reader of `cargo metadata`, shared by every check that needs the package graph.
//!
//! Three checks need the graph and they need it with different breadth: the layer rules and the lint
//! table only want the workspace's own packages, the duplicate check wants the whole resolved graph. The
//! flags differ, but the reading, the error text and the shape of a package are defined once here.

use std::path::Path;
use std::process::Command;

/// A package as `cargo metadata` describes it, kept small: only the fields a rule needs.
pub(crate) struct Crate {
    pub(crate) name: String,
    /// The full package id, which is what distinguishes two copies of the same name.
    pub(crate) id: String,
    /// `None` for a path package; a source string for one from a registry or git.
    pub(crate) source: Option<String>,
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
}

pub(crate) struct CrateDependency {
    pub(crate) name: String,
    /// Absolute path for a workspace/path dependency, `None` for an external one.
    pub(crate) path: Option<String>,
    /// A dev-dependency, built only for the crate's own tests, examples and benchmarks.
    pub(crate) dev: bool,
}

/// Every target kind a library can be built as. A crate whose only library type is `staticlib` reports
/// its target as kind `staticlib`, not `lib`, so filtering on `lib` alone would hide exactly the native
/// library the layer rules look for.
const LIBRARY_KINDS: &[&str] = &["lib", "rlib", "dylib", "staticlib", "cdylib"];

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
    let mut crates = Vec::with_capacity(list.len());
    for package in list {
        let text = |key: &str| package.get(key).and_then(Metadata::as_str).unwrap_or_default().to_string();
        let source = package.get("source").and_then(Metadata::as_str).map(str::to_string);
        let dependencies = package
            .get("dependencies")
            .and_then(Metadata::as_array)
            .map(|deps| {
                deps.iter()
                    .map(|dep| CrateDependency {
                        name: dep.get("name").and_then(Metadata::as_str).unwrap_or_default().to_string(),
                        path: dep.get("path").and_then(Metadata::as_str).map(str::to_string),
                        dev: dep.get("kind").and_then(Metadata::as_str) == Some("dev"),
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
        let id = text("id");
        let depends_on = resolved_dependencies(metadata, &id);
        crates.push(Crate { name: text("name"), id, source, manifest_path: text("manifest_path"), dependencies, library_targets, build_script, depends_on });
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

/// Run cargo metadata and read it. `no_deps` asks only for the workspace's own packages, which is all
/// the layer rules need and far less for cargo to resolve. `--locked` because the gate reads the graph
/// `Cargo.lock` pins and never writes it: a lock file that needs updating fails here instead of
/// changing under the change being checked.
pub(crate) fn read(rust_dir: &Path, no_deps: bool) -> Result<Vec<Crate>, String> {
    let mut command = Command::new(env!("CARGO"));
    command.current_dir(rust_dir).arg("metadata").arg("--format-version").arg("1").arg("--locked");
    if no_deps {
        command.arg("--no-deps");
    }
    let output = command.output().map_err(|error| format!("could not run `{}`: {error}", env!("CARGO")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("cargo metadata exited with {}: {}", output.status, stderr.trim()));
    }
    packages(&parse(&output.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The minimum JSON `cargo metadata` prints for one package, plus a dev-dependency and a non-library
    /// target the reader must drop.
    const ONE_PACKAGE: &str = r#"{
        "packages": [{
            "name": "craftbridge-core", "version": "0.1.0", "id": "path+file:///x/craftbridge-core#0.1.0",
            "manifest_path": "/x/craftbridge-core/Cargo.toml",
            "dependencies": [
                {"name": "craftbridge-media", "kind": null, "path": "/x/craftbridge-media"},
                {"name": "insta", "kind": "dev"},
                {"name": "uniffi", "kind": "build"}
            ],
            "targets": [
                {"kind": ["lib"], "name": "craftbridge_core", "crate_types": ["lib", "cdylib"]},
                {"kind": ["bin"], "crate_types": ["bin"]}
            ]
        }],
        "workspace_members": [], "resolve": null, "target_directory": "", "version": 1, "workspace_root": "/x"
    }"#;

    #[test]
    fn dev_dependencies_are_marked_and_non_library_targets_dropped() {
        let parsed = packages(&serde_json::from_str::<Metadata>(ONE_PACKAGE).unwrap()).unwrap();
        assert_eq!(parsed.len(), 1);
        let package = &parsed[0];
        assert_eq!(package.source, None);
        assert_eq!(package.id, "path+file:///x/craftbridge-core#0.1.0");
        let names: Vec<(&str, bool)> = package.dependencies.iter().map(|dep| (dep.name.as_str(), dep.dev)).collect();
        assert_eq!(names, [("craftbridge-media", false), ("insta", true), ("uniffi", false)]);
        assert_eq!(package.dependencies[0].path.as_deref(), Some("/x/craftbridge-media"));
        assert_eq!(package.library_targets.len(), 1);
        assert_eq!(package.library_targets[0].crate_types, ["lib".to_string(), "cdylib".to_string()]);
        assert_eq!(package.library_targets[0].name, "craftbridge_core");
        assert_eq!(package.build_script, None);
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
    fn unreadable_metadata_is_an_error_not_a_panic() {
        assert!(parse(b"not json").is_err());
        assert!(packages(&serde_json::json!({})).is_err());
    }
}
