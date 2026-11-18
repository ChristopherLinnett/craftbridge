//! Rule 2 across the language wall — the universal core knows nothing about any Craft app, and an app
//! plugin depends on the core alone, never on another plugin or a sibling app's code.
//!
//! Cargo's graph is the layers check's; this is SwiftPM's, read from `swift package dump-package`
//! rather than re-parsed by hand, so a change in SwiftPM's manifest spelling shifts the dump along with
//! it and cannot silently make this check read nothing. The same dump answers which packages the manifest
//! takes: a local one is refused, because another folder's code would reach the build through a door
//! neither the direction rule nor the licence check can see. The Gradle side of the same rule sits in
//! `android/sdk/settings.gradle.kts`, where Gradle's own project graph answers the question.
//!
//! `swift package describe` answers what the dump cannot, SwiftPM having resolved each target: the folder
//! it sits in, which rule 1's layout holds to `Sources/<name>` for every regular target, and the files it
//! compiles, which every check must be able to read ([`resolved_errors`]). SwiftPM describes no package whose
//! binary target is missing, and the native library that target links is the gate's longest build, so the
//! rules read the manifest alone and the Swift lint, which reads the package resolved anyway, asks those.

use std::path::Path;
use std::process::Command;

/// The core's targets (plan §20, §22): its Swift API, the one generated `CraftBridgeFFI` module that
/// UniFFI's Swift for every crate compiles into, and the binary target holding the native library.
/// Every other target that ships is a plugin's, whatever it is named, so a target cannot step out of the
/// rule by a name §9 does not give it.
const CORE: &[&str] = &["CraftBridge", "CraftBridgeFFI", "CraftBridgeFFIBinary"];

/// A dumped target: its name, whether it is a test target, and the names it asks other targets for,
/// across every spelling a dependency can take in the dump (bare name, `byName`, `target`).
struct Target {
    name: String,
    test: bool,
    asked: Vec<String>,
}

fn asked_names(dependencies: &serde_json::Value) -> Vec<String> {
    let mut asked = Vec::new();
    if let Some(dependencies) = dependencies.as_array() {
        for dependency in dependencies {
            match dependency {
                serde_json::Value::String(name) => asked.push(name.clone()),
                _ => {
                    for key in ["byName", "target"] {
                        if let Some(names) = dependency.get(key).and_then(serde_json::Value::as_array) {
                            asked.extend(names.iter().filter_map(serde_json::Value::as_str).map(str::to_string));
                        }
                    }
                }
            }
        }
    }
    asked
}

/// The Swift package's manifest, as the repository names it.
pub(crate) const FILE: &str = "ios/sdk/Package.swift";

/// Local packages, named the way SwiftPM itself names them in the dump: `.package(path:)` is a
/// `fileSystem` dependency, and a `file://` git URL is a `sourceControl` whose location is `local`.
/// Either would bring another folder's code into the build through a door neither check here can see:
/// a local package is not a target of this package, so the direction rule cannot place it, and no
/// `Package.resolved` pin records it, so the licence check never asks its licence (§8, §2). A Swift
/// package arrives published or not at all.
fn local_dependency_errors(package: &serde_json::Value) -> Vec<String> {
    let mut errors = Vec::new();
    let refuse = |at: String| {
        format!("{FILE}: local package `{at}`: code the direction and licence checks cannot see; a Swift package is taken from a published release")
    };
    for dependency in package.get("dependencies").and_then(serde_json::Value::as_array).into_iter().flatten() {
        for local in dependency.get("fileSystem").and_then(serde_json::Value::as_array).into_iter().flatten() {
            let at = local.get("path").and_then(serde_json::Value::as_str).unwrap_or("?");
            errors.push(refuse(at.to_string()));
        }
        for source in dependency.get("sourceControl").and_then(serde_json::Value::as_array).into_iter().flatten() {
            if let Some(location) = source.get("location").and_then(|location| location.get("local")).and_then(serde_json::Value::as_array) {
                let at = location.first().and_then(serde_json::Value::as_str).unwrap_or("?");
                errors.push(refuse(at.to_string()));
            }
        }
    }
    errors
}

/// Judge one dumped package. Product and package dependencies point at other packages — SwiftPM refuses
/// a product of the package itself — so only names that are targets of this package are direction. A
/// test target ships nothing and wires whatever its tests need; SwiftPM's own `type` says which those are.
pub(crate) fn package_errors(package: &serde_json::Value) -> Vec<String> {
    let mut errors = local_dependency_errors(package);
    let targets: Vec<Target> = package
        .get("targets")
        .and_then(serde_json::Value::as_array)
        .map(|targets| {
            targets
                .iter()
                .filter_map(|target| {
                    Some(Target {
                        name: target.get("name").and_then(serde_json::Value::as_str)?.to_string(),
                        test: target.get("type").and_then(serde_json::Value::as_str) == Some("test"),
                        asked: target.get("dependencies").map(asked_names).unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let names: Vec<&str> = targets.iter().map(|target| target.name.as_str()).collect();
    for target in targets.iter().filter(|target| !target.test) {
        let core = CORE.contains(&target.name.as_str());
        for dependency in target.asked.iter().filter(|asked| names.contains(&asked.as_str()) && !CORE.contains(&asked.as_str())) {
            errors.push(if core {
                format!("{FILE}: core target `{}` depends on `{dependency}`: the universal core knows nothing about any Craft app", target.name)
            } else {
                format!(
                    "{FILE}: plugin target `{}` depends on `{dependency}`: a plugin depends on the core's targets alone, never on another plugin",
                    target.name
                )
            });
        }
    }
    errors
}

/// Run the check on the repository's Swift package: the direction of its targets' dependencies, as the
/// manifest declares them. A tree without one (a scratch tree) has nothing to ask.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    Ok(dump_package(&root.join("ios").join("sdk"), FILE)?.map(|package| package_errors(&package)).unwrap_or_default())
}

/// What SwiftPM resolved for the package in `sdk`, as [`describe_package`] reads `targets`: where its
/// shipping targets sit, and what SwiftPM builds that no check sees.
pub(crate) fn resolved_errors(sdk: &Path, targets: &[SwiftTarget]) -> Result<Vec<String>, String> {
    let read = crate::scan::walk(sdk, "")?.into_iter().map(|file| file.relative).collect();
    let mut errors = placement_errors(targets);
    errors.extend(target_errors(targets, &read));
    Ok(errors)
}

/// The iOS version a dumped package's `platforms` name, which every build of the package and of the native
/// library it links targets.
pub(crate) fn ios_version(package: &serde_json::Value) -> Result<String, String> {
    package
        .get("platforms")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|platform| platform.get("platformName").and_then(serde_json::Value::as_str) == Some("ios"))
        .and_then(|platform| platform.get("version").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .ok_or_else(|| format!("{FILE}: names no iOS version to build for"))
}

/// The folder, relative to the package, of a dumped package's one binary target built from a path: what the
/// target links, and so where the build of the native library puts it. That build replaces the folder
/// whole, so a path that is not an XCFramework inside the package is refused rather than cleared.
pub(crate) fn binary_target_path(package: &serde_json::Value) -> Result<String, String> {
    let paths: Vec<&str> = package
        .get("targets")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|target| target.get("type").and_then(serde_json::Value::as_str) == Some("binary"))
        .filter_map(|target| target.get("path").and_then(serde_json::Value::as_str))
        .collect();
    let [path] = paths.as_slice() else {
        return Err(format!("{FILE}: holds {} binary targets built from a path, where the native library is one", paths.len()));
    };
    let inside = Path::new(path).components().all(|part| matches!(part, std::path::Component::Normal(_)));
    if !inside || !path.ends_with(".xcframework") {
        return Err(format!("{FILE}: the binary target's path `{path}` is no XCFramework inside the package, the one folder its build may replace"));
    }
    Ok((*path).to_string())
}

/// A regular target — the core's or a plugin's, the code that ships — sits at `Sources/<name>`, where
/// AGENTS.md §1's layout puts the core beside each plugin. SwiftPM's resolved folder is the answer, not the
/// manifest's `path:`: SwiftPM also finds a target with no `path:` in `Source/`, `src/` or `srcs/` when the
/// package has no `Sources/`, so a manifest that sets no path still moves every target once the folder is
/// renamed. A test target sits wherever its tests are (what SwiftPM compiles into it is test code, §3), and
/// a binary target's folder is the vendored library itself.
pub(crate) fn placement_errors(targets: &[SwiftTarget]) -> Vec<String> {
    targets
        .iter()
        .filter(|target| target.kind == "library" && target.path != format!("Sources/{}", target.name))
        .map(|target| {
            format!(
                "{FILE}: target `{}` sits in `{}`: a regular target sits at `Sources/<name>`, where the layout puts the core beside each plugin, moved by no `path:` and found in no other source folder",
                target.name, target.path
            )
        })
        .collect()
}

/// SwiftPM's own reading of the `Package.swift` in `package_dir`, or `None` where there is none (a
/// scratch tree). The one way every check asks a Swift package what it holds — its targets, their types
/// and paths, and the packages it takes — so a manifest spelling SwiftPM still accepts (a deprecated
/// `.package(name:url:…)`, a target moved by `path:`) cannot be read one way by one check and another way,
/// or not at all, by the next. `shown` is how a message names the manifest.
pub(crate) fn dump_package(package_dir: &Path, shown: &str) -> Result<Option<serde_json::Value>, String> {
    if !package_dir.join("Package.swift").is_file() {
        return Ok(None);
    }
    let output = Command::new("xcrun")
        .args(["swift", "package", "dump-package"])
        .current_dir(package_dir)
        .output()
        .map_err(|error| format!("{shown}: cannot ask SwiftPM for its manifest: {error}"))?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{shown}: `swift package dump-package` failed: {}", text.trim()));
    }
    serde_json::from_slice(&output.stdout).map(Some).map_err(|error| format!("{shown}: dump-package is not JSON this check can read: {error}"))
}

/// A target as `swift package describe --type json` resolves it: its type, its folder and the files it
/// compiles (`exclude` and `sources` applied, `./` spellings normalised), the products it ships in, and
/// the targets it depends on. SwiftPM's own answer to what each target compiles, so no check guesses it
/// from folder names.
#[derive(Debug, Default)]
pub(crate) struct SwiftTarget {
    pub(crate) name: String,
    /// SwiftPM's type: `library` for a regular target, `test`, `executable`, `binary`, `macro`, …
    pub(crate) kind: String,
    /// The folder SwiftPM resolved for it, relative to the package, as SwiftPM normalises it (no `./`, no
    /// trailing `/`): the `path:` the manifest sets, or the default folder SwiftPM found.
    pub(crate) path: String,
    /// The files it compiles, relative to the package.
    pub(crate) sources: Vec<String>,
    pub(crate) products: Vec<String>,
    pub(crate) dependencies: Vec<String>,
}

/// SwiftPM's resolved description of the package in `package_dir`, or `None` where there is none (a scratch
/// tree). `shown` is how a message names the manifest.
pub(crate) fn describe_package(package_dir: &Path, shown: &str) -> Result<Option<Vec<SwiftTarget>>, String> {
    if !package_dir.join("Package.swift").is_file() {
        return Ok(None);
    }
    let output = Command::new("xcrun")
        .args(["swift", "package", "describe", "--type", "json"])
        .current_dir(package_dir)
        .output()
        .map_err(|error| format!("{shown}: cannot ask SwiftPM for its targets: {error}"))?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{shown}: `swift package describe` failed: {}", text.trim()));
    }
    let described: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|error| format!("{shown}: describe is not JSON this check can read: {error}"))?;
    Ok(Some(swift_targets(&described)))
}

/// The targets of a described package.
pub(crate) fn swift_targets(described: &serde_json::Value) -> Vec<SwiftTarget> {
    let strings = |target: &serde_json::Value, key: &str| -> Vec<String> {
        target
            .get(key)
            .and_then(serde_json::Value::as_array)
            .map(|items| items.iter().filter_map(serde_json::Value::as_str).map(str::to_string).collect())
            .unwrap_or_default()
    };
    described
        .get("targets")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|target| {
            let path = target.get("path").and_then(serde_json::Value::as_str).unwrap_or_default().trim_end_matches('/').to_string();
            SwiftTarget {
                name: target.get("name").and_then(serde_json::Value::as_str).unwrap_or_default().to_string(),
                kind: target.get("type").and_then(serde_json::Value::as_str).unwrap_or_default().to_string(),
                sources: strings(target, "sources").into_iter().map(|source| if path.is_empty() { source } else { format!("{path}/{source}") }).collect(),
                path,
                products: strings(target, "product_memberships"),
                dependencies: strings(target, "target_dependencies"),
            }
        })
        .collect()
}

/// The files SwiftPM compiles into test targets, relative to the package: the only Swift that is test
/// code, whatever folder it sits in and whatever folder a shipping target shares with it.
pub(crate) fn test_sources(targets: &[SwiftTarget]) -> std::collections::BTreeSet<String> {
    targets.iter().filter(|target| target.kind == "test").flat_map(|target| target.sources.iter().cloned()).collect()
}

/// What SwiftPM builds that no check sees: a file a target compiles that the rules' walk does not read
/// (one inside build output, which every stage prunes), and a regular target that no product and no test
/// or executable target uses, which the package scheme the `swift_build` stage builds never compiles.
/// `read` are the walk's files, relative to the package.
pub(crate) fn target_errors(targets: &[SwiftTarget], read: &std::collections::BTreeSet<String>) -> Vec<String> {
    let mut errors = Vec::new();
    for target in targets.iter().filter(|target| target.kind != "binary") {
        for source in target.sources.iter().filter(|source| !read.contains(*source)) {
            errors.push(format!(
                "{FILE}: target `{}` compiles ios/sdk/{source}, which lies in build output no check reads: a target's sources sit where every check reads them",
                target.name
            ));
        }
    }
    let mut built: Vec<&str> = targets
        .iter()
        .filter(|target| !target.products.is_empty() || target.kind == "test" || target.kind == "executable")
        .map(|target| target.name.as_str())
        .collect();
    let mut next = 0;
    while let Some(name) = built.get(next).copied() {
        next += 1;
        for dependency in targets.iter().filter(|target| target.name == name).flat_map(|target| &target.dependencies) {
            if !built.contains(&dependency.as_str()) {
                built.push(dependency);
            }
        }
    }
    for target in targets.iter().filter(|target| target.kind == "library" && !built.contains(&target.name.as_str())) {
        errors.push(format!(
            "{FILE}: target `{}` is in no product and no test or executable target depends on it, so the swift_build stage never compiles it",
            target.name
        ));
    }
    errors
}

/// The published packages a dumped package takes: each source-control dependency's URL and each
/// registry dependency's identity. SwiftPM prints a remote location as a plain string or, in newer
/// versions, as `{"urlString": …}`; both are read.
pub(crate) fn published_packages(package: &serde_json::Value) -> Vec<String> {
    let mut packages = Vec::new();
    for dependency in package.get("dependencies").and_then(serde_json::Value::as_array).into_iter().flatten() {
        for source in dependency.get("sourceControl").and_then(serde_json::Value::as_array).into_iter().flatten() {
            let remotes = source.get("location").and_then(|location| location.get("remote")).and_then(serde_json::Value::as_array);
            for remote in remotes.into_iter().flatten() {
                if let Some(url) = remote.as_str().or_else(|| remote.get("urlString").and_then(serde_json::Value::as_str)) {
                    packages.push(url.to_string());
                }
            }
        }
        for registry in dependency.get("registry").and_then(serde_json::Value::as_array).into_iter().flatten() {
            if let Some(identity) = registry.get("identity").and_then(serde_json::Value::as_str) {
                packages.push(identity.to_string());
            }
        }
    }
    packages.sort();
    packages.dedup();
    packages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    fn package(targets: &str) -> serde_json::Value {
        serde_json::from_str(&format!(r#"{{"targets": {targets}}}"#)).unwrap()
    }

    #[test]
    fn the_planned_package_with_two_plugins_is_legal() {
        // The shape `swift package dump-package` prints for plan §22's targets, plus a second plugin.
        let legal = package(
            r#"[{"name":"CraftBridgeFFIBinary","type":"binary","dependencies":[]},
                {"name":"CraftBridgeFFI","type":"regular","dependencies":[{"byName":["CraftBridgeFFIBinary",null]}]},
                {"name":"CraftBridge","type":"regular","dependencies":[{"byName":["CraftBridgeFFI",null]}]},
                {"name":"CraftBridgeEffects","type":"regular","dependencies":[{"byName":["CraftBridge",null]},{"byName":["CraftBridgeFFI",null]}]},
                {"name":"CraftBridgeFilm","type":"regular","dependencies":[{"target":["CraftBridge",null]},"CraftBridgeFFIBinary"]},
                {"name":"CraftBridgeEffectsTests","type":"test","dependencies":[{"byName":["CraftBridgeEffects",null]},{"byName":["CraftBridgeFilm",null]}]}]"#,
        );
        assert!(package_errors(&legal).is_empty(), "{:?}", package_errors(&legal));
    }

    #[test]
    fn a_plugin_reaching_for_another_plugin_is_reported() {
        let errors = package_errors(&package(
            r#"[{"name":"CraftBridge","type":"regular","dependencies":[]},
                {"name":"CraftBridgeEffects","type":"regular","dependencies":["CraftBridge",{"byName":["CraftBridgeFilm",null]}]},
                {"name":"CraftBridgeFilm","type":"regular","dependencies":[]}]"#,
        ));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].contains("CraftBridgeEffects") && errors[0].contains("CraftBridgeFilm") && errors[0].contains("never on another plugin"),
            "{errors:?}"
        );
    }

    #[test]
    fn the_core_reaching_beyond_its_own_targets_is_reported_whatever_the_target_is_called() {
        let errors = package_errors(&package(
            r#"[{"name":"CraftBridge","type":"regular","dependencies":["CraftBridgeEffects",{"target":["EffectsKit",null]}]},
                {"name":"CraftBridgeEffects","type":"regular","dependencies":["CraftBridge"]},
                {"name":"EffectsKit","type":"regular","dependencies":[]}]"#,
        ));
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().all(|e| e.contains("core target `CraftBridge`") && e.contains("knows nothing about any Craft app")), "{errors:?}");
    }

    #[test]
    fn only_a_test_target_by_type_may_wire_plugins_together() {
        // A shipping target is judged however its name ends; only SwiftPM's `test` type earns the exception.
        let errors = package_errors(&package(
            r#"[{"name":"CraftBridge","type":"regular","dependencies":[]},
                {"name":"CraftBridgeEffects","type":"regular","dependencies":["CraftBridge"]},
                {"name":"CraftBridgeShimTests","type":"regular","dependencies":["CraftBridgeEffects"]}]"#,
        ));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("plugin target `CraftBridgeShimTests`"), "{errors:?}");
    }

    #[test]
    fn dependencies_on_other_packages_are_not_direction() {
        let legal = package(
            r#"[{"name":"CraftBridgeEffects","type":"regular","dependencies":[{"product":["Logging","swift-log",null,null]},{"target":["CraftBridge",null]}]},
                {"name":"CraftBridge","type":"regular","dependencies":[]}]"#,
        );
        assert!(package_errors(&legal).is_empty(), "{:?}", package_errors(&legal));
    }

    /// The two shapes `swift package dump-package` prints for a package taken from a folder: probed on
    /// scratch manifests, one with `.package(path:)`, one with a `file://` git URL.
    fn dependencies(shaped: &str) -> serde_json::Value {
        serde_json::from_str(&format!(r#"{{"targets": [], "dependencies": [{shaped}]}}"#)).unwrap()
    }

    #[test]
    fn a_path_package_is_refused() {
        let errors = package_errors(&dependencies(r#"{"fileSystem": [{"identity": "local", "path": "/private/tmp/probe/local", "productFilter": null}]}"#));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("local package `/private/tmp/probe/local`") && errors[0].contains("direction and licence"), "{errors:?}");
    }

    #[test]
    fn a_git_url_that_points_at_this_machine_is_a_local_package_too() {
        let errors = package_errors(&dependencies(
            r#"{"sourceControl": [{"identity": "localgit", "location": {"local": ["/tmp/probe/localgit"]}, "requirement": {"branch": ["main"]}}]}"#,
        ));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("local package `/tmp/probe/localgit`"), "{errors:?}");
    }

    /// The shapes `swift package dump-package` printed for a scratch manifest holding `.package(url:)`,
    /// the deprecated `.package(name:url:)`, `.package(id:)`, a library target moved under `Tests/` and a
    /// test target with a `path:` of its own.
    const PROBED: &str = r#"{
        "dependencies": [
            {"sourceControl": [{"identity": "swift-log", "location": {"remote": [{"urlString": "https://github.com/apple/swift-log"}]}}]},
            {"sourceControl": [{"identity": "swift-collections", "location": {"remote": [{"urlString": "https://github.com/apple/swift-collections"}]},
                                "nameForTargetDependencyResolutionOnly": "swift-collections"}]},
            {"sourceControl": [{"identity": "uniffi", "location": {"remote": ["https://github.com/mozilla/uniffi.git"]}}]},
            {"registry": [{"identity": "mona.LinkedList"}]}
        ],
        "targets": [
            {"name": "A", "type": "regular", "path": null},
            {"name": "B", "type": "regular", "path": "Tests/Support"},
            {"name": "ATests", "type": "test", "path": null},
            {"name": "CTests", "type": "test", "path": "Checks/"}
        ]
    }"#;

    #[test]
    fn every_published_dependency_is_read_however_the_manifest_spells_it() {
        let package: serde_json::Value = serde_json::from_str(PROBED).unwrap();
        assert_eq!(
            published_packages(&package),
            ["https://github.com/apple/swift-collections", "https://github.com/apple/swift-log", "https://github.com/mozilla/uniffi.git", "mona.LinkedList"]
        );
    }

    /// The shape `swift package describe --type json` printed for a scratch package: a library, one under
    /// the build-output `Frameworks/`, a shipping target nested in a test target's folder (which excludes
    /// it), and a test target spelt `./Tests/ATests`.
    const DESCRIBED: &str = r#"{"targets": [
        {"name": "N", "type": "library", "path": "Tests/ATests/Nested", "sources": ["N.swift"]},
        {"name": "B", "type": "library", "path": "Frameworks/B", "sources": ["B.swift"]},
        {"name": "ATests", "type": "test", "path": "Tests/ATests", "sources": ["T.swift"], "target_dependencies": ["A", "N"]},
        {"name": "A", "type": "library", "path": "Sources/A", "sources": ["A.swift"], "product_memberships": ["A"]},
        {"name": "Extras", "type": "library", "path": "Sources/Extras", "sources": ["Extras.swift"]},
        {"name": "FFIBinary", "type": "binary", "path": "Frameworks/X.xcframework"}
    ]}"#;

    #[test]
    fn only_the_files_swiftpm_compiles_into_a_test_target_are_test_code() {
        let targets = swift_targets(&serde_json::from_str(DESCRIBED).unwrap());
        assert_eq!(test_sources(&targets).into_iter().collect::<Vec<_>>(), ["Tests/ATests/T.swift"], "the nested shipping target is not a test");
    }

    #[test]
    fn a_target_built_from_build_output_or_built_by_nothing_is_refused() {
        let targets = swift_targets(&serde_json::from_str(DESCRIBED).unwrap());
        let read = ["Sources/A/A.swift", "Tests/ATests/T.swift", "Tests/ATests/Nested/N.swift", "Sources/Extras/Extras.swift"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let errors = target_errors(&targets, &read);
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("target `B` compiles ios/sdk/Frameworks/B/B.swift, which lies in build output")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("target `B` is in no product")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("target `Extras` is in no product")), "{errors:?}");
    }

    #[test]
    fn the_ios_version_and_the_binary_targets_folder_are_the_manifests() {
        let dumped = |targets: &str| package(targets);
        let mut planned = dumped(
            r#"[{"name":"CraftBridgeFFIBinary","type":"binary","path":"Frameworks/CraftBridgeEffectsFFI.xcframework"},
                {"name":"CraftBridgeFFI","type":"regular","path":null}]"#,
        );
        assert_eq!(binary_target_path(&planned).unwrap(), "Frameworks/CraftBridgeEffectsFFI.xcframework");
        assert_eq!(ios_version(&planned).unwrap_err(), "ios/sdk/Package.swift: names no iOS version to build for");
        planned["platforms"] = serde_json::json!([{"platformName": "macos", "version": "14.0"}, {"platformName": "ios", "version": "17.0"}]);
        assert_eq!(ios_version(&planned).unwrap(), "17.0");
        let none = dumped(r#"[{"name":"A","type":"regular","path":null},{"name":"B","type":"binary","url":"https://x/b.zip","checksum":"0"}]"#);
        assert_eq!(binary_target_path(&none).unwrap_err(), "ios/sdk/Package.swift: holds 0 binary targets built from a path, where the native library is one");
        let two = dumped(r#"[{"name":"B","type":"binary","path":"Frameworks/B.xcframework"},{"name":"C","type":"binary","path":"Frameworks/C.xcframework"}]"#);
        assert!(binary_target_path(&two).unwrap_err().contains("holds 2 binary targets"));
        for outside in ["../Frameworks/B.xcframework", "/tmp/B.xcframework", "Frameworks/../../B.xcframework", "Frameworks", "Frameworks/./B.xcframework/.."] {
            let path = dumped(&format!(r#"[{{"name":"B","type":"binary","path":"{outside}"}}]"#));
            assert_eq!(
                binary_target_path(&path).unwrap_err(),
                format!(
                    "ios/sdk/Package.swift: the binary target's path `{outside}` is no XCFramework inside the package, the one folder its build may replace"
                )
            );
        }
    }

    /// The rules read the package without its binary target built: SwiftPM describes no such package, so a
    /// check of the rules' that asked it would fail on every checkout the native library is not built in.
    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "asks SwiftPM through xcrun, which only macOS has")]
    fn the_rules_read_a_package_whose_binary_target_is_not_built() {
        let scratch = scratch_package(
            ".binaryTarget(name: \"CraftBridgeFFIBinary\", path: \"Frameworks/CraftBridgeEffectsFFI.xcframework\"), .target(name: \"CraftBridge\", dependencies: [\"CraftBridgeFFIBinary\"]), .target(name: \"CraftBridgeEffects\", dependencies: [\"CraftBridge\", \"CraftBridgeEffectsFilm\"]), .target(name: \"CraftBridgeEffectsFilm\")",
        );
        scratch.write("ios/sdk/Sources/CraftBridgeEffectsFilm/F.swift", "public let film = 1\n");
        let sdk = scratch.0.join("ios/sdk");
        assert!(describe_package(&sdk, FILE).unwrap_err().contains("does not contain binary artifact"), "SwiftPM describes no package missing its binary");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("plugin target `CraftBridgeEffects` depends on `CraftBridgeEffectsFilm`"), "{errors:?}");
    }

    #[test]
    fn a_published_package_dependency_stays_legal() {
        let legal = dependencies(
            r#"{"sourceControl": [{"identity": "uniffi", "location": {"remote": ["https://github.com/mozilla/uniffi.git"]}, "requirement": {"branch": ["main"]}}]}"#,
        );
        assert!(package_errors(&legal).is_empty(), "{:?}", package_errors(&legal));
    }

    /// The folders `swift package describe` printed (Swift 6.3) for scratch packages: two targets whose
    /// `path:` (`./Sources/CraftBridge/`, `Sources/CraftBridge/../CraftBridgeFilm`) SwiftPM normalised to
    /// their default folder, one moved by `path:`, one whose `path:` spells `Sources` in another case, and
    /// the test and binary targets that keep folders of their own.
    const PLACED: &str = r#"{"targets": [
        {"name": "CraftBridge", "type": "library", "path": "Sources/CraftBridge", "sources": ["A.swift"]},
        {"name": "CraftBridgeFilm", "type": "library", "path": "Sources/CraftBridgeFilm", "sources": ["C.swift"]},
        {"name": "CraftBridgeEffects", "type": "library", "path": "Plugins/CraftBridgeEffects", "sources": ["B.swift"]},
        {"name": "CraftBridgeAudio", "type": "library", "path": "sources/CraftBridgeAudio", "sources": ["D.swift"]},
        {"name": "CraftBridgeEffectsTests", "type": "test", "path": "Checks/Effects", "sources": ["T.swift"]},
        {"name": "CraftBridgeFFIBinary", "type": "binary", "path": "Frameworks/CraftBridgeFFI.xcframework"}
    ]}"#;

    #[test]
    fn a_regular_target_sits_at_sources_and_nowhere_else() {
        let errors = placement_errors(&swift_targets(&serde_json::from_str(PLACED).unwrap()));
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("ios/sdk/Package.swift: target `CraftBridgeEffects` sits in `Plugins/CraftBridgeEffects`")), "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("ios/sdk/Package.swift: target `CraftBridgeAudio` sits in `sources/CraftBridgeAudio`")), "{errors:?}");
        assert!(errors.iter().all(|e| e.contains("Sources/<name>")), "{errors:?}");
    }

    /// A scratch package of the layout's shape, its core and one plugin, with `Package.swift` as given.
    fn scratch_package(manifest_targets: &str) -> Scratch {
        let scratch = Scratch::new("direction-placement");
        for (folder, file) in [("CraftBridge", "CraftBridge.swift"), ("CraftBridgeEffects", "CraftBridgeEffects.swift")] {
            scratch.write(&format!("ios/sdk/Sources/{folder}/{file}"), "public let placed = 1\n");
        }
        scratch.write(
            "ios/sdk/Package.swift",
            format!(
                "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(\n    name: \"CraftBridge\",\n    products: [.library(name: \"CraftBridge\", targets: [\"CraftBridge\", \"CraftBridgeEffects\"])],\n    targets: [{manifest_targets}],\n)\n"
            ),
        );
        scratch
    }

    /// What the Swift lint reports of a package SwiftPM resolved, in `root`'s `ios/sdk`.
    fn resolved(root: &Path) -> Vec<String> {
        let sdk = root.join("ios/sdk");
        resolved_errors(&sdk, &describe_package(&sdk, FILE).unwrap().unwrap()).unwrap()
    }

    /// The acceptance shape, asked of the real tool through the check that describes the package: a plugin
    /// moved to `Plugins/` by `path:`, and every target moved with no `path:` at all by renaming `Sources/` to
    /// a folder SwiftPM also searches. The default layout passes.
    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "asks SwiftPM through xcrun, which only macOS has")]
    fn a_target_moved_out_of_sources_fails_the_check_that_asks_swiftpm() {
        let targets = ".target(name: \"CraftBridge\"), .target(name: \"CraftBridgeEffects\", dependencies: [\"CraftBridge\"])";
        let scratch = scratch_package(targets);
        let sdk = scratch.0.join("ios/sdk");
        assert!(resolved(&scratch.0).is_empty(), "the default layout");
        assert!(check(&scratch.0).unwrap().is_empty(), "the rules read the manifest");

        let moved = scratch_package(&targets.replace("[\"CraftBridge\"])", "[\"CraftBridge\"], path: \"Plugins/CraftBridgeEffects\")"));
        std::fs::create_dir_all(moved.0.join("ios/sdk/Plugins")).unwrap();
        std::fs::rename(moved.0.join("ios/sdk/Sources/CraftBridgeEffects"), moved.0.join("ios/sdk/Plugins/CraftBridgeEffects")).unwrap();
        let errors = resolved(&moved.0);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("ios/sdk/Package.swift: target `CraftBridgeEffects` sits in `Plugins/CraftBridgeEffects`"), "{errors:?}");

        std::fs::rename(sdk.join("Sources"), sdk.join("src")).unwrap();
        let errors = resolved(&scratch.0);
        assert_eq!(errors.len(), 2, "{errors:?}");
        for name in ["CraftBridge", "CraftBridgeEffects"] {
            assert!(errors.iter().any(|e| e.starts_with(&format!("ios/sdk/Package.swift: target `{name}` sits in `src/{name}`"))), "{errors:?}");
        }
    }
}
