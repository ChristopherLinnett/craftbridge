//! Rule 8 — the licences of the dependencies Cargo cannot see.
//!
//! Gradle libraries and SwiftPM packages are invisible to Cargo, so their licences are declared in
//! `tools/licences.toml` and checked here against that same allowlist — never a copy of it, because a
//! second list drifts the first time someone widens the real one.
//!
//! Both directions are checked: a dependency missing from `tools/licences.toml`, a declared licence the
//! allowlist refuses, and an entry nobody uses any more. A stale entry is an error because it is a licence
//! that has been quietly widened without a decision issue behind it.
//!
//! What is read is what the builds use: each library of the version catalog, each package a
//! `Package.swift` declares (as `swift package dump-package` reads it), each pin a `Package.resolved`
//! records, and — asked of Gradle itself rather than read off the build files — every external module
//! resolved onto each library module's release, unit-test and instrumented-test runtime classpaths, and
//! onto the tool classpaths the gate itself runs its Kotlin checks with: the rule project's compile,
//! runtime and test classpaths and the analyser's own runtime, as the build resolves them. That last
//! part is the point: transitive modules arrive inside the AAR, in the tests, and in the gate's own
//! tooling, without any build file naming them. Build plugins never ship, so they stay checked by
//! name, from the catalog's `[plugins]`.

use crate::{pins, scan};
use std::path::{Path, PathBuf};

/// A declared licence for one Gradle library or SwiftPM package.
pub(crate) struct Declaration<'a> {
    /// `group:artifact` for a Maven library; the package URL, or registry identity, for a SwiftPM package.
    pub(crate) key: &'a str,
    pub(crate) expression: &'a str,
    /// How a message names the file the declaration came from.
    pub(crate) source: &'a str,
    /// Which kind of dependency it is, so a message can say where to fix it.
    pub(crate) kind: &'static str,
}

/// A dependency a build file actually names.
pub(crate) struct Used {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) kind: &'static str,
}

/// How deep parentheses may nest before an expression is refused, so a malformed one cannot exhaust the
/// stack; real licence expressions nest once or twice.
const MAX_NESTING: usize = 16;

/// Is a licence identifier on the allowlist?
pub(crate) fn identifier_allowed(expression: &str, allow: &[String]) -> bool {
    allow.iter().map(|allowed| allowed.trim()).any(|allowed| allowed == expression.trim())
}

/// Is a whole SPDX expression allowed? `OR` needs one allowed side and `AND` needs both; `AND` binds
/// tighter than `OR`, as SPDX reads it, and parentheses group. `X WITH Y` is one licence compared
/// verbatim, because `WITH` qualifies a licence rather than choosing one. An expression this cannot read
/// is refused: refusing is the safe answer for a shape the check does not understand.
pub(crate) fn expression_allowed(expression: &str, allow: &[String]) -> bool {
    let spaced = expression.replace('(', " ( ").replace(')', " ) ");
    let tokens: Vec<&str> = spaced.split_whitespace().collect();
    let mut parser = Spdx { tokens: &tokens, at: 0, depth: 0, allow };
    parser.or_expression().is_some_and(|allowed| allowed && parser.at == tokens.len())
}

/// A recursive-descent reader of one SPDX expression; each step answers whether what it read is allowed,
/// or `None` when the expression is malformed.
struct Spdx<'a> {
    tokens: &'a [&'a str],
    at: usize,
    depth: usize,
    allow: &'a [String],
}

impl<'a> Spdx<'a> {
    fn peek(&self) -> Option<&'a str> {
        self.tokens.get(self.at).copied()
    }

    fn or_expression(&mut self) -> Option<bool> {
        let mut allowed = self.and_expression()?;
        while self.peek() == Some("OR") {
            self.at += 1;
            allowed |= self.and_expression()?;
        }
        Some(allowed)
    }

    fn and_expression(&mut self) -> Option<bool> {
        let mut allowed = self.term()?;
        while self.peek() == Some("AND") {
            self.at += 1;
            allowed &= self.term()?;
        }
        Some(allowed)
    }

    fn term(&mut self) -> Option<bool> {
        let token = self.peek()?;
        self.at += 1;
        if token == "(" {
            self.depth += 1;
            if self.depth > MAX_NESTING {
                return None;
            }
            let inner = self.or_expression()?;
            self.depth -= 1;
            return (self.peek() == Some(")")).then(|| {
                self.at += 1;
                inner
            });
        }
        let licence = Some(token).filter(|token| !is_operator(token))?;
        if self.peek() != Some("WITH") {
            return Some(identifier_allowed(licence, self.allow));
        }
        self.at += 1;
        let exception = self.peek().filter(|token| !is_operator(token))?;
        self.at += 1;
        Some(identifier_allowed(&format!("{licence} WITH {exception}"), self.allow))
    }
}

/// Is a token an operator or a parenthesis rather than a licence?
fn is_operator(token: &str) -> bool {
    matches!(token, "(" | ")" | "AND" | "OR" | "WITH")
}

/// Read the allowlist from `rust/deny.toml`: the one definition, so this check and cargo-deny agree.
pub(crate) fn allowlist(deny_source: &str, name: &str) -> Result<Vec<String>, String> {
    let document = toml::from_str::<toml::Value>(deny_source).map_err(|error| format!("{name}: is not valid TOML: {error}"))?;
    let allow = document
        .get("licenses")
        .and_then(toml::Value::as_table)
        .and_then(|licenses| licenses.get("allow"))
        .and_then(toml::Value::as_array)
        .ok_or_else(|| format!("{name}: [licenses].allow is missing, so no licence can be checked"))?;
    let identifiers: Vec<String> = allow.iter().filter_map(toml::Value::as_str).map(str::to_string).collect();
    if identifiers.is_empty() {
        return Err(format!("{name}: [licenses].allow is empty, so no licence can be checked"));
    }
    Ok(identifiers)
}

/// Read the declared licences: `[maven]` and `[swiftpm]`. A missing file is empty, which is right for a
/// repository that has no Gradle libraries and no SwiftPM packages yet.
pub(crate) fn declarations(licences_source: Option<&str>, name: &str) -> Result<(Vec<(String, String)>, Vec<(String, String)>), String> {
    let Some(source) = licences_source else { return Ok((Vec::new(), Vec::new())) };
    let document = toml::from_str::<toml::Value>(source).map_err(|error| format!("{name}: is not valid TOML: {error}"))?;
    let table = |key: &str| -> Vec<(String, String)> {
        document
            .get(key)
            .and_then(toml::Value::as_table)
            .map(|entries| entries.iter().filter_map(|(name, value)| value.as_str().map(|expression| (name.clone(), expression.to_string()))).collect())
            .unwrap_or_default()
    };
    Ok((table("maven"), table("swiftpm")))
}

/// The `[libraries]` entries of a Gradle version catalog, in the three forms a catalog allows:
/// `"group:artifact:version"`, `{ module = "group:artifact", version.ref = … }` and
/// `{ group = "…", name = "…", … }`.
pub(crate) fn gradle_libraries(catalog: &toml::Value, name: &str) -> Result<Vec<String>, String> {
    let Some(libraries) = catalog.get("libraries").and_then(toml::Value::as_table) else { return Ok(Vec::new()) };
    let mut keys = Vec::new();
    for (alias, entry) in libraries {
        let module = match entry {
            toml::Value::String(module) => module.clone(),
            toml::Value::Table(table) => {
                if let Some(module) = table.get("module").and_then(toml::Value::as_str) {
                    module.to_string()
                } else {
                    match (table.get("group").and_then(toml::Value::as_str), table.get("name").and_then(toml::Value::as_str)) {
                        (Some(group), Some(name)) => format!("{group}:{name}"),
                        _ => return Err(format!("{name}: library `{alias}` gives neither `module` nor a `group` and `name` pair")),
                    }
                }
            }
            _ => return Err(format!("{name}: library `{alias}` is not a string or a table")),
        };
        keys.push(module.split(':').take(2).collect::<Vec<_>>().join(":"));
    }
    keys.sort();
    Ok(keys)
}

/// The `[plugins]` entries of a Gradle version catalog, as the Maven module Gradle resolves for each
/// plugin id: its marker, `<id>:<id>.gradle.plugin`. A plugin never ships, but it runs in the build and
/// arrives from a repository like any library, so its licence is declared like one; a plugin added to
/// the catalog alone reaches the build through a root `alias(…) apply false` line.
pub(crate) fn gradle_plugins(catalog: &toml::Value, name: &str) -> Result<Vec<String>, String> {
    let Some(plugins) = catalog.get("plugins").and_then(toml::Value::as_table) else { return Ok(Vec::new()) };
    let mut keys = Vec::new();
    for (alias, entry) in plugins {
        let id = match entry {
            toml::Value::String(notation) => notation.split(':').next().unwrap_or_default().to_string(),
            toml::Value::Table(table) => table.get("id").and_then(toml::Value::as_str).unwrap_or_default().to_string(),
            _ => String::new(),
        };
        if id.is_empty() {
            return Err(format!("{name}: plugin `{alias}` gives no plugin id"));
        }
        keys.push(format!("{id}:{id}.gradle.plugin"));
    }
    keys.sort();
    Ok(keys)
}

/// Every `Package.swift` under `ios/`: generated code is not a dependency declaration, and `.build` holds
/// someone else's checkout.
pub(crate) fn package_manifests(ios: &Path) -> Result<Vec<PathBuf>, String> {
    Ok(scan::walk(ios, "Package.swift")?
        .into_iter()
        .filter(|file| file.path.file_name().is_some_and(|name| name == "Package.swift") && !crate::generated::in_generated(&file.components))
        .map(|file| file.path)
        .collect())
}

/// Every Xcode project of the iOS samples, at any depth under `ios/samples`: a `<name>.xcodeproj` folder that
/// holds the `project.pbxproj` Xcode reads, the walk skipping build output as every rule does. This is the one
/// answer to which apps the samples are: `swift_build` builds each (`cargo xtask swift-samples`) and the licence
/// check reads each one's pins, so no sample is built and left unread, or read and left unbuilt.
pub(crate) fn sample_projects(ios: &Path) -> Result<Vec<PathBuf>, String> {
    Ok(scan::walk(&ios.join("samples"), "project.pbxproj")?
        .into_iter()
        .filter(|file| file.path.file_name().is_some_and(|name| name == "project.pbxproj"))
        .filter_map(|file| file.path.parent().filter(|project| project.extension().is_some_and(|extension| extension == "xcodeproj")).map(Path::to_path_buf))
        .collect())
}

/// Every `Package.resolved` the Swift under `ios/` ships with: the SDK package's, beside its manifest, and each
/// sample project's, which Xcode writes at `<name>.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/`.
/// A file elsewhere under `ios/samples` is not one Xcode reads for a project, so it is no pin of an app.
pub(crate) fn resolved_files(ios: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = vec![ios.join("sdk").join("Package.resolved")];
    for project in sample_projects(ios)? {
        files.push(project.join("project.xcworkspace").join("xcshareddata").join("swiftpm").join("Package.resolved"));
    }
    Ok(files)
}

/// `cargo xtask swift-samples`: the Xcode projects of the iOS samples, one path from the repository root per
/// line.
pub(crate) fn samples(root: &Path) -> std::process::ExitCode {
    let projects = sample_projects(&root.join("ios")).map(|projects| projects.iter().map(|project| crate::report::shown(root, project)).collect());
    crate::report::print_lines(projects, "swift-samples")
}

/// Compare what the builds use against what is declared: every use needs a declaration with an allowed
/// expression, and every declaration needs a use.
pub(crate) fn compare(used: &[Used], declared: &[Declaration<'_>], allow: &[String]) -> Vec<String> {
    let mut errors = Vec::new();
    for use_ in used {
        match declared.iter().find(|declaration| declaration.key == use_.key) {
            None => errors.push(format!("{}: {} `{}` has no licence in tools/licences.toml", use_.source, use_.kind, use_.key)),
            Some(declaration) => {
                if !expression_allowed(declaration.expression, allow) {
                    errors.push(format!(
                        "{}: {} `{}` is licensed `{}`, outside the allowlist in rust/deny.toml",
                        declaration.source, declaration.kind, declaration.key, declaration.expression
                    ));
                }
            }
        }
    }
    for declaration in declared {
        if !used.iter().any(|use_| use_.key == declaration.key) {
            errors.push(format!(
                "{}: {} `{}` is declared but no build file uses it: a licence entry is only widened by a decision issue",
                declaration.source, declaration.kind, declaration.key
            ));
        }
    }
    errors
}

/// A file's text, or `None` when it does not exist; any other failure to read it is an error.
fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match path.try_exists() {
        Ok(true) => scan::read_text(path).map(Some),
        Ok(false) => Ok(None),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// Run the check: read the allowlist, the declarations and the build files, and compare.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let allow = allowlist(&scan::read_text(&root.join("rust").join("deny.toml"))?, "rust/deny.toml")?;
    let (maven, swiftpm) = declarations(read_optional(&root.join("tools").join("licences.toml"))?.as_deref(), "tools/licences.toml")?;
    let declared: Vec<Declaration<'_>> = maven
        .iter()
        .map(|(key, expression)| Declaration { key, expression, source: "tools/licences.toml", kind: "maven library" })
        .chain(swiftpm.iter().map(|(key, expression)| Declaration { key, expression, source: "tools/licences.toml", kind: "SwiftPM package" }))
        .collect();

    // The Gradle version catalog, the one place the Android build names a library, read by the one reader
    // of the pinned files (rust/xtask/src/pins.rs).
    let mut used = Vec::new();
    if let Some(catalog) = pins::catalog(root)? {
        for key in gradle_libraries(&catalog, pins::CATALOG)? {
            used.push(Used { key, source: pins::CATALOG.to_string(), kind: "library" });
        }
        for key in gradle_plugins(&catalog, pins::CATALOG)? {
            used.push(Used { key, source: pins::CATALOG.to_string(), kind: "build plugin" });
        }
    }
    for module in gradle_resolved_modules(root)? {
        used.push(Used { key: module, source: "Gradle runtime classpaths".to_string(), kind: "module" });
    }
    // SwiftPM's own reading of each manifest, so every spelling it accepts — the deprecated
    // `.package(name:url:…)` included — names its package here as it does to the build.
    let mut errors = Vec::new();
    for manifest in package_manifests(&root.join("ios"))? {
        let shown = crate::report::shown(root, &manifest);
        let Some(folder) = manifest.parent() else { continue };
        let published: Vec<String> = crate::direction::dump_package(folder, &shown)?.iter().flat_map(crate::direction::published_packages).collect();
        errors.extend(unpinned(&shown, &published, folder.join("Package.resolved").is_file()));
        for key in published {
            used.push(Used { key, source: shown.clone(), kind: "package" });
        }
    }
    // SwiftPM writes the resolved graph next to the manifest once it has resolved the package, and Xcode
    // writes a sample project's into the project's own workspace; every pin ships with the app, transitive
    // ones included, whether or not any manifest names it.
    for resolved in resolved_files(&root.join("ios"))? {
        let Some(source) = read_optional(&resolved)? else { continue };
        let shown = crate::report::shown(root, &resolved);
        for (identity, location) in resolved_pins(&source, &shown)? {
            // A declaration may key the package by its URL or by its registry identity; a pin matches
            // either. Matching neither, it is reported under the URL, which is what a person fetches.
            let key = if swiftpm.iter().any(|(key, _)| key.trim_end_matches('/') == location) {
                location
            } else if swiftpm.iter().any(|(key, _)| key == &identity) {
                identity
            } else {
                location
            };
            used.push(Used { key, source: shown.clone(), kind: "resolved package" });
        }
    }
    // The catalog and the classpath can name one module, and a pin repeats its manifest's package;
    // one use is one use.
    used.sort_by(|a, b| a.key.cmp(&b.key).then_with(|| a.source.cmp(&b.source)));
    used.dedup_by(|a, b| a.key == b.key);

    errors.extend(compare(&used, &declared, &allow));
    Ok(errors)
}

/// A package that takes a published package needs its `Package.resolved` committed: SwiftPM writes the
/// packages those take only there, so without it their licences go unread on a fresh checkout, where this
/// stage runs before any build has resolved anything.
fn unpinned(shown: &str, published: &[String], resolved: bool) -> Option<String> {
    (!published.is_empty() && !resolved).then(|| {
        format!(
            "{shown}: takes {} but no Package.resolved sits beside it: the packages those take, and their licences, are written only there",
            published.join(", ")
        )
    })
}

/// The external modules Gradle resolves for the licences check: what lands on the Android library
/// modules' runtime classpaths, and what lands on the gate's own detekt tooling (the rule project's
/// compile, runtime and test classpaths, and the analyser the gate forks). Gradle
/// itself answers — `android/sdk`'s `runtimeLicences` and `:build-logic:detekt-rules:gateToolLicences`
/// tasks print their resolved graphs — because the transitive modules are the whole point and no build
/// file lists them. A tree without the Gradle wrapper (a scratch tree) has no answer to ask for; a
/// Gradle that fails is an error, because the licences of what ships cannot be guessed at in its place.
fn gradle_resolved_modules(root: &Path) -> Result<Vec<String>, String> {
    let sdk = root.join("android").join("sdk");
    if !sdk.join("gradlew").is_file() {
        return Ok(Vec::new());
    }
    let output = std::process::Command::new(sdk.join("gradlew"))
        .current_dir(&sdk)
        .args(["--quiet", "--no-daemon", "--console=plain", "runtimeLicences", ":build-logic:detekt-rules:gateToolLicences"])
        .output()
        .map_err(|error| format!("android/sdk: cannot ask Gradle for the runtime classpaths: {error}"))?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stderr);
        // Gradle buries the reason under `What went wrong`; the tail of the log is help-links noise.
        let reason: Vec<&str> = text
            .lines()
            .skip_while(|line| !line.contains("What went wrong"))
            .skip(1)
            .take_while(|line| !line.trim_start().starts_with('*'))
            .filter(|line| !line.trim().is_empty())
            .map(str::trim)
            .collect();
        let reason = if reason.is_empty() {
            let noise: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
            // The last four non-blank lines, or every one when there are fewer.
            noise.iter().skip(noise.len().saturating_sub(4)).copied().collect()
        } else {
            reason
        };
        // Gradle needs the JDK and Android SDK tools/env.sh finds; run by hand, a shell without them gets
        // Gradle's own complaint, so the message says where the environment comes from.
        return Err(format!(
            "android/sdk: `runtimeLicences`/`gateToolLicences` failed, so the licences of what ships and of the gate's own tooling cannot be checked: {} (tools/ci.sh rules runs it with the JDK and Android SDK tools/env.sh finds)",
            reason.join(" ")
        ));
    }
    Ok(resolved_modules(&String::from_utf8_lossy(&output.stdout)))
}

/// Gradle's answer, one `group:name` per line: a module coordinate is one colon and no spaces, so blank
/// lines and anything the daemon logs anyway are not module lines.
pub(crate) fn resolved_modules(stdout: &str) -> Vec<String> {
    stdout.lines().map(str::trim).filter(|line| !line.is_empty() && line.matches(':').count() == 1 && !line.contains(' ')).map(str::to_string).collect()
}

/// The pins of a SwiftPM `Package.resolved`, as `(identity, location)`. Every pin ships, so every pin is
/// checked; v2 and v3 spell the same fields, so the file format is not pinned to a version here.
pub(crate) fn resolved_pins(source: &str, name: &str) -> Result<Vec<(String, String)>, String> {
    let document: serde_json::Value = serde_json::from_str(source).map_err(|error| format!("{name}: is not valid JSON: {error}"))?;
    let pins = document.get("pins").and_then(serde_json::Value::as_array).ok_or_else(|| format!("{name}: has no pins list"))?;
    pins.iter()
        .map(|pin| {
            let location = pin.get("location").and_then(serde_json::Value::as_str).ok_or_else(|| format!("{name}: a pin has no location"))?;
            let identity = pin.get("identity").and_then(serde_json::Value::as_str).unwrap_or(location);
            Ok((identity.trim_end_matches('/').to_string(), location.trim_end_matches('/').to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_taking_a_published_one_needs_its_pins_committed() {
        let missing = unpinned("ios/sdk/Package.swift", &["https://github.com/apple/swift-collections".to_string()], false);
        assert!(missing.is_some_and(|error| error.contains("no Package.resolved")));
        assert!(unpinned("ios/sdk/Package.swift", &["https://github.com/apple/swift-collections".to_string()], true).is_none());
        assert!(unpinned("ios/sdk/Package.swift", &[], false).is_none(), "a package that takes none has nothing to pin");
    }
    use crate::scratch::Scratch;

    const ALLOW: &[&str] = &["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-3-Clause", "MPL-2.0"];

    fn allow() -> Vec<String> {
        ALLOW.iter().map(|identifier| identifier.to_string()).collect()
    }

    /// A catalog's text as the pins reader hands it to the licence check.
    fn parsed(catalog: &str) -> toml::Value {
        toml::from_str(catalog).unwrap()
    }

    /// A manifest SwiftPM reads, taking one published package in the deprecated `name:url:` spelling the
    /// licence check once missed.
    const UNIFFI_MANIFEST: &str = "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(\n    name: \"Probe\",\n    dependencies: [.package(name: \"uniffi\", url: \"https://github.com/mozilla/uniffi.git\", from: \"0.32.0\")],\n    targets: []\n)\n";

    /// A scratch repository with the files `check` reads — `rust/deny.toml`, `tools/licences.toml`, the
    /// version catalog and one `ios/sdk/Package.swift` — so the check runs against a tree on disk without
    /// depending on what the real repository happens to hold.
    struct ScratchRepo(Scratch);

    impl ScratchRepo {
        fn new(tag: &str, licences: &str, catalog: &str, manifest: &str) -> Self {
            let scratch = Scratch::new(tag);
            scratch.write("rust/deny.toml", format!("[licenses]\nversion = 2\nallow = {ALLOW:?}\n"));
            scratch.write("tools/licences.toml", licences);
            if !catalog.is_empty() {
                scratch.write(pins::CATALOG, catalog);
            }
            if !manifest.is_empty() {
                scratch.write("ios/sdk/Package.swift", manifest);
            }
            Self(scratch)
        }

        fn errors(&self) -> Vec<String> {
            check(&self.0.0).unwrap()
        }
    }

    #[test]
    fn a_declared_but_forbidden_licence_is_reported_end_to_end() {
        // LGPL is on no branch the allowlist permits, so the declaration is refused even though a
        // build file uses it. (`EPL-1.0 OR MIT` would pass: an OR only needs one allowed branch.)
        let repo = ScratchRepo::new("forbidden", "[maven]\n\"junit:junit\" = \"LGPL-2.1-or-later\"\n", "[libraries]\njunit = \"junit:junit:4.13.2\"\n", "");
        let errors = repo.errors();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("LGPL-2.1-or-later") && errors[0].contains("rust/deny.toml"), "{errors:?}");
    }

    #[test]
    fn a_used_library_without_a_declaration_is_reported_end_to_end() {
        let repo = ScratchRepo::new("missing", "[maven]\n", "[libraries]\njunit = \"junit:junit:4.13.2\"\n", "");
        let errors = repo.errors();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("junit:junit") && errors[0].contains("has no licence"), "{errors:?}");
    }

    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "asks SwiftPM through xcrun, which only macOS has")]
    fn a_compliant_repository_finds_nothing() {
        let repo = ScratchRepo::new(
            "clean",
            "[maven]\n\"junit:junit\" = \"MIT\"\n[swiftpm]\n\"https://github.com/mozilla/uniffi.git\" = \"MPL-2.0\"\n",
            "[libraries]\njunit = \"junit:junit:4.13.2\"\n",
            UNIFFI_MANIFEST,
        );
        repo.0.write(
            "ios/sdk/Package.resolved",
            r#"{ "version": 2, "pins": [{ "identity": "uniffi", "location": "https://github.com/mozilla/uniffi.git", "state": { "revision": "x" } }] }"#,
        );
        assert!(repo.errors().is_empty(), "{:?}", repo.errors());
    }

    #[test]
    fn a_stale_declaration_in_a_real_tree_is_reported() {
        // Declared in tools/licences.toml but no build file in this scratch tree uses it.
        let repo = ScratchRepo::new("stale", "[maven]\n\"g:gone\" = \"MIT\"\n", "[libraries]\n", "");
        let errors = repo.errors();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("no build file uses it"), "{errors:?}");
    }

    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "asks SwiftPM through xcrun, which only macOS has")]
    fn a_resolved_pin_needs_a_declaration_like_any_package() {
        let repo = ScratchRepo::new("pins", "[swiftpm]\n\"https://github.com/mozilla/uniffi.git\" = \"MPL-2.0\"\n", "", UNIFFI_MANIFEST);
        let unpinned = repo.errors();
        assert!(unpinned.len() == 1 && unpinned[0].contains("no Package.resolved sits beside it"), "{unpinned:?}");
        repo.0.write(
            "ios/sdk/Package.resolved",
            r#"{ "version": 2, "pins": [
                { "identity": "uniffi", "location": "https://github.com/mozilla/uniffi.git", "state": { "revision": "x" } },
                { "identity": "swift-log", "location": "https://github.com/apple/swift-log.git", "state": { "revision": "y" } } ] }"#,
        );
        let errors = repo.errors();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("swift-log") && errors[0].contains("has no licence"), "{errors:?}");
        assert!(errors[0].contains("Package.resolved"), "{errors:?}");
    }

    const SWIFT_LOG_PIN: &str =
        r#"{ "version": 3, "pins": [{ "identity": "swift-log", "location": "https://github.com/apple/swift-log.git", "state": { "revision": "y" } }] }"#;

    /// Where Xcode keeps the pins of the project at `project`, under `ios/samples`.
    fn project_pins(project: &str) -> String {
        format!("ios/samples/{project}/project.xcworkspace/xcshareddata/swiftpm/Package.resolved")
    }

    #[test]
    fn a_sample_projects_pin_needs_a_declaration_like_the_sdks() {
        for project in ["App/App.xcodeproj", "Group/App/App.xcodeproj"] {
            let repo = ScratchRepo::new("sample-pin", "[swiftpm]\n", "", "");
            repo.0.write(&format!("ios/samples/{project}/project.pbxproj"), "");
            repo.0.write(&project_pins(project), SWIFT_LOG_PIN);
            let errors = repo.errors();
            assert_eq!(errors.len(), 1, "{project}: {errors:?}");
            assert!(errors[0].contains("swift-log") && errors[0].contains("has no licence"), "{errors:?}");
            assert!(errors[0].contains(&project_pins(project)), "{errors:?}");
        }
    }

    #[test]
    fn a_resolved_file_no_project_reads_is_no_pin_of_an_app() {
        let repo = ScratchRepo::new("sample-stray", "[swiftpm]\n", "", "");
        repo.0.write("ios/samples/App/App.xcodeproj/project.pbxproj", "");
        repo.0.write("ios/samples/App/Package.resolved", SWIFT_LOG_PIN);
        repo.0.write("ios/samples/App/App.xcodeproj/Package.resolved", SWIFT_LOG_PIN);
        repo.0.write("ios/samples/App/App.xcodeproj/project.xcworkspace/Package.resolved", SWIFT_LOG_PIN);
        // A project folder Xcode cannot open, holding no `project.pbxproj`, is no app either.
        repo.0.write(&project_pins("Other/Other.xcodeproj"), SWIFT_LOG_PIN);
        assert!(repo.errors().is_empty(), "{:?}", repo.errors());
    }

    /// The samples' projects are the `.xcodeproj` folders Xcode can open, however deep, and nothing in build
    /// output or somebody else's packages: what `swift_build` builds and what the licence check reads are this one
    /// list.
    #[test]
    fn the_samples_projects_are_every_project_xcode_opens_under_the_samples() {
        let scratch = Scratch::new("sample-projects");
        for file in [
            "ios/samples/App/App.xcodeproj/project.pbxproj",
            "ios/samples/Group/Other/Other.xcodeproj/project.pbxproj",
            "ios/samples/Empty/Empty.xcodeproj/project.xcworkspace/contents.xcworkspacedata",
            "ios/samples/Loose/project.pbxproj",
            "ios/samples/Backup/Backup.xcodeproj/old-project.pbxproj",
            "ios/samples/App/node_modules/Copy.xcodeproj/project.pbxproj",
        ] {
            scratch.write(file, "");
        }
        let ios = scratch.0.join("ios");
        let projects: Vec<String> = sample_projects(&ios).unwrap().iter().map(|project| crate::report::shown(&scratch.0, project)).collect();
        assert_eq!(projects, ["ios/samples/App/App.xcodeproj", "ios/samples/Group/Other/Other.xcodeproj"]);
    }

    #[test]
    fn pin_lists_and_gradle_answers_are_read_exactly() {
        assert_eq!(
            resolved_pins(r#"{"version": 2, "pins": [{"identity": "mona.LinkedList", "location": "mona.LinkedList"}]}"#, "r").unwrap(),
            vec![("mona.LinkedList".to_string(), "mona.LinkedList".to_string())]
        );
        assert!(resolved_pins("{}", "r").is_err(), "a file without pins is not a resolved file");
        assert!(resolved_pins("not json", "r").is_err());
        assert!(resolved_pins(r#"{"pins": [{"identity": "x"}]}"#, "r").is_err(), "a pin without a location is not a pin");
        // Gradle's answer, with any logging it makes anyway filtered out: a module line is one colon, no spaces.
        assert_eq!(
            resolved_modules("org.jetbrains.kotlin:kotlin-stdlib\n  org.jetbrains:annotations \n\nCalculating task graph\na:b:c\n"),
            ["org.jetbrains.kotlin:kotlin-stdlib", "org.jetbrains:annotations"]
        );
    }

    #[test]
    fn single_and_compound_expressions_are_allowed() {
        assert!(expression_allowed("MIT", &allow()));
        assert!(expression_allowed("MIT OR Apache-2.0", &allow()));
        assert!(expression_allowed("Apache-2.0 WITH LLVM-exception", &allow()), "a WITH term is one identifier");
        assert!(expression_allowed("MIT AND Apache-2.0", &allow()));
        assert!(expression_allowed("MIT OR (BSD-3-Clause AND MPL-2.0)", &allow()), "a parenthesised group is its own alternative");
        assert!(expression_allowed("(MIT OR Apache-2.0)", &allow()), "a whole expression in parentheses");
        assert!(expression_allowed("(BSD-3-Clause AND MPL-2.0) OR GPL-3.0-only", &allow()), "the allowed alternative may come first");
        assert!(expression_allowed("(GPL-3.0-only OR MIT) AND Apache-2.0", &allow()));
        assert!(expression_allowed("(Apache-2.0 WITH LLVM-exception) OR MIT", &allow()));
    }

    #[test]
    fn a_forbidden_licence_is_reported_however_it_hides() {
        assert!(!expression_allowed("GPL-3.0-only", &allow()));
        // An `AND` needs both sides, so a group whose every choice is forbidden sinks the whole expression,
        // while a group with one allowed choice is satisfied by it, as cargo-deny reads the same expression.
        assert!(!expression_allowed("MIT AND (GPL-3.0-only OR LGPL-2.1-only)", &allow()), "no allowed choice in the required group");
        assert!(!expression_allowed("(MIT OR Apache-2.0) AND GPL-3.0-only", &allow()));
        assert!(expression_allowed("MIT AND (Apache-2.0 OR GPL-3.0-only)", &allow()), "choosing Apache-2.0 satisfies the group");
        assert!(!expression_allowed("MIT AND GPL-3.0-only", &allow()));
        assert!(!expression_allowed("Apache-2.0 WITH LLVM-exception AND GPL-3.0-only", &allow()));
        assert!(!expression_allowed("MITNO", &allow()), "the allowlist is compared whole, not as a prefix");
        assert!(!expression_allowed("", &allow()));
        for malformed in ["MIT OR", "(MIT", "MIT)", "AND MIT", "MIT Apache-2.0", "()", "MIT WITH", &"(".repeat(64)] {
            assert!(!expression_allowed(malformed, &allow()), "{malformed:?} cannot be read, so it is refused");
        }
    }

    #[test]
    fn the_allowlist_comes_from_deny_toml() {
        let deny = "[licenses]\nversion = 2\nallow = [\"MIT\", \"Apache-2.0\"]\n";
        assert_eq!(allowlist(deny, "rust/deny.toml").unwrap(), vec!["MIT".to_string(), "Apache-2.0".to_string()]);
        assert!(allowlist("[licenses]\nversion = 2\n", "rust/deny.toml").is_err());
        assert!(allowlist("[licenses]\nallow = []\n", "rust/deny.toml").is_err());
        assert!(allowlist("not toml", "rust/deny.toml").is_err());
    }

    #[test]
    fn a_missing_licence_file_declares_nothing() {
        let (maven, swiftpm) = declarations(None, "tools/licences.toml").unwrap();
        assert!(maven.is_empty() && swiftpm.is_empty());
        let (maven, swiftpm) =
            declarations(Some("[maven]\n\"g:a\" = \"MIT\"\n[swiftpm]\n\"https://x/y\" = \"Apache-2.0 OR MIT\"\n"), "tools/licences.toml").unwrap();
        assert_eq!(maven, vec![("g:a".to_string(), "MIT".to_string())]);
        assert_eq!(swiftpm, vec![("https://x/y".to_string(), "Apache-2.0 OR MIT".to_string())]);
    }

    #[test]
    fn every_gradle_library_form_is_read() {
        let catalog = r#"
[libraries]
plain = "net.java.dev.jna:jna:5.13.1"
table = { module = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version.ref = "coroutines" }
split = { group = "androidx.annotation", name = "annotation", version = "1.8.0" }
"#;
        let keys = gradle_libraries(&parsed(catalog), "libs.versions.toml").unwrap();
        assert_eq!(keys, vec!["androidx.annotation:annotation", "net.java.dev.jna:jna", "org.jetbrains.kotlinx:kotlinx-coroutines-core"]);
        assert!(gradle_libraries(&parsed("[libraries]\nbroken = { version.ref = \"x\" }\n"), "c").is_err());
    }

    #[test]
    fn every_catalog_plugin_is_read_as_its_marker_module() {
        let catalog =
            "[plugins]\nandroid-library = { id = \"com.android.library\", version.ref = \"agp\" }\ncompose = \"org.jetbrains.kotlin.plugin.compose:2.4.21\"\n";
        assert_eq!(
            gradle_plugins(&parsed(catalog), "libs.versions.toml").unwrap(),
            ["com.android.library:com.android.library.gradle.plugin", "org.jetbrains.kotlin.plugin.compose:org.jetbrains.kotlin.plugin.compose.gradle.plugin"]
        );
        assert!(gradle_plugins(&parsed("[plugins]\nbroken = { version = \"1\" }\n"), "c").is_err());
        let repo = ScratchRepo::new("plugins", "[maven]\n", catalog, "");
        let errors = repo.errors();
        assert!(errors.len() == 2 && errors.iter().all(|e| e.contains("build plugin") && e.contains("has no licence")), "{errors:?}");
    }

    #[test]
    fn only_a_file_named_package_swift_is_a_manifest() {
        let scratch = Scratch::new("manifests");
        scratch.write("sdk/Package.swift", "");
        scratch.write("sdk/Sources/X/MyPackage.swift", "");
        scratch.write("sdk/Sources/Generated/Package.swift", "");
        assert_eq!(package_manifests(&scratch.0).unwrap(), vec![scratch.0.join("sdk/Package.swift")]);
    }

    #[test]
    fn a_missing_declaration_a_forbidden_licence_and_a_stale_entry_are_all_reported() {
        let used = [Used { key: "net.java.dev.jna:jna".to_string(), source: pins::CATALOG.to_string(), kind: "library" }];
        let missing = compare(&used, &[], &allow());
        assert!(missing[0].contains("has no licence in tools/licences.toml"), "{missing:?}");

        let forbidden = [Declaration { key: "net.java.dev.jna:jna", expression: "LGPL-2.1-or-later", source: "tools/licences.toml", kind: "maven library" }];
        let errors = compare(&used, &forbidden, &allow());
        assert!(errors[0].contains("LGPL-2.1-or-later") && errors[0].contains("rust/deny.toml"), "{errors:?}");

        let good = [Declaration { key: "net.java.dev.jna:jna", expression: "Apache-2.0", source: "tools/licences.toml", kind: "maven library" }];
        assert!(compare(&used, &good, &allow()).is_empty());

        let stale = [Declaration { key: "g:gone", expression: "MIT", source: "tools/licences.toml", kind: "maven library" }];
        let errors = compare(&[], &stale, &allow());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("no build file uses it"), "{errors:?}");
    }
}
