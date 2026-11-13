//! Rule 2 across the language wall — the universal core knows nothing about any Craft app, and an app
//! plugin depends on the core alone, never on another plugin or a sibling app's code.
//!
//! Cargo's graph is the layers check's; this is SwiftPM's, read from `swift package dump-package`
//! rather than re-parsed by hand, so a change in SwiftPM's manifest spelling shifts the dump along with
//! it and cannot silently make this check read nothing. The same dump answers which packages the manifest
//! takes: a local one is refused, because another folder's code would reach the build through a door
//! neither the direction rule nor the licence check can see. The Gradle side of the same rule sits in
//! `android/sdk/settings.gradle.kts`, where Gradle's own project graph answers the question.

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

const FILE: &str = "ios/sdk/Package.swift";

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

/// Run the check on the repository's Swift package. A tree without one (a scratch tree) has nothing to ask.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let sdk = root.join("ios").join("sdk");
    if !sdk.join("Package.swift").is_file() {
        return Ok(Vec::new());
    }
    let output = Command::new("xcrun")
        .args(["swift", "package", "dump-package"])
        .current_dir(&sdk)
        .output()
        .map_err(|error| format!("{FILE}: cannot ask SwiftPM for its manifest: {error}"))?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{FILE}: `swift package dump-package` failed: {}", text.trim()));
    }
    let package: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|error| format!("{FILE}: dump-package is not JSON this check can read: {error}"))?;
    Ok(package_errors(&package))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_published_package_dependency_stays_legal() {
        let legal = dependencies(
            r#"{"sourceControl": [{"identity": "uniffi", "location": {"remote": ["https://github.com/mozilla/uniffi.git"]}, "requirement": {"branch": ["main"]}}]}"#,
        );
        assert!(package_errors(&legal).is_empty(), "{:?}", package_errors(&legal));
    }
}
