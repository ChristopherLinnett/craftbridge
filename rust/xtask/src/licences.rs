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
//! What is read is what ships: each library of the version catalog, each package a `Package.swift`
//! declares, each pin a `Package.resolved` records, and — asked of Gradle itself rather than read off the
//! build files — every external module resolved onto each library module's release runtime classpath.
//! That last part is the point: transitive modules arrive inside the AAR without any build file naming
//! them. Build plugins never ship, so they stay checked by name, from the catalog.

use crate::scan;
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

/// The Gradle version catalog, the one place the Android build names a library.
const CATALOG: &str = "android/sdk/gradle/libs.versions.toml";

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
pub(crate) fn gradle_libraries(catalog_source: &str, name: &str) -> Result<Vec<String>, String> {
    let document = toml::from_str::<toml::Value>(catalog_source).map_err(|error| format!("{name}: is not valid TOML: {error}"))?;
    let Some(libraries) = document.get("libraries").and_then(toml::Value::as_table) else { return Ok(Vec::new()) };
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

/// The packages a `Package.swift` depends on: the URL of each `.package(url: "…")` and the registry
/// identity of each `.package(id: "…")`, tolerating whitespace and line breaks anywhere in the
/// declaration. A `.package(path:)` is a package of this repository and carries no licence of its own.
pub(crate) fn swiftpm_packages(package_source: &str) -> Vec<String> {
    let mut packages = Vec::new();
    // The declaration is found in a copy with the comments stripped (so a commented-out one is not a
    // dependency) and the whitespace removed (so one spread over lines still matches). The stripper keeps
    // strings and every byte's offset, so the quoted value is then read from the stripped text itself,
    // starting where the declaration ended.
    let stripped = scan::strip_comments(package_source);
    let (offsets, compact): (Vec<usize>, Vec<u8>) = stripped.bytes().enumerate().filter(|(_, byte)| !byte.is_ascii_whitespace()).unzip();
    for needle in [".package(url:", ".package(id:"] {
        let mut from = 0usize;
        while let Some(offset) = scan::find(&compact, from, needle.as_bytes()) {
            let end = offsets.get(offset + needle.len() - 1).copied().unwrap_or(stripped.len());
            if let Some(value) = quoted_after(&stripped, end + 1) {
                packages.push(value);
            }
            from = offset + needle.len();
        }
    }
    packages.sort();
    packages.dedup();
    packages
}

/// The text between the first pair of double quotes at or after `from`, reading through line breaks.
/// A package's URL or identity can only sit inside a string literal, so the two quotes around it are
/// unambiguous delimiters.
fn quoted_after(text: &str, from: usize) -> Option<String> {
    let rest = text.get(from..)?;
    let start = rest.find('"')?;
    let after = rest.get(start + 1..)?;
    let end = after.find('"')?;
    after.get(..end).map(str::to_string)
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

    let mut used = Vec::new();
    if let Some(catalog) = read_optional(&root.join(CATALOG))? {
        for key in gradle_libraries(&catalog, CATALOG)? {
            used.push(Used { key, source: CATALOG.to_string(), kind: "library" });
        }
    }
    for module in gradle_release_modules(root)? {
        used.push(Used { key: module, source: "Gradle release runtime classpath".to_string(), kind: "module" });
    }
    for manifest in package_manifests(&root.join("ios"))? {
        let shown = crate::report::shown(root, &manifest);
        for key in swiftpm_packages(&scan::read_text(&manifest)?) {
            used.push(Used { key, source: shown.clone(), kind: "package" });
        }
    }
    // SwiftPM writes the resolved graph next to the manifest once it has resolved the package; every pin
    // ships with the app, transitive ones included, whether or not any manifest names it.
    let resolved = root.join("ios").join("sdk").join("Package.resolved");
    if let Some(source) = read_optional(&resolved)? {
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

    Ok(compare(&used, &declared, &allow))
}

/// The external modules that land on the Android library modules' release runtime classpaths. Gradle
/// itself answers — `android/sdk`'s `runtimeLicences` task prints its resolved graph — because the
/// transitive modules are the whole point and no build file lists them. A tree without the Gradle
/// wrapper (a scratch tree) has no answer to ask for; a Gradle that fails is an error, because the
/// licences of what ships cannot be guessed at in its place.
fn gradle_release_modules(root: &Path) -> Result<Vec<String>, String> {
    let sdk = root.join("android").join("sdk");
    if !sdk.join("gradlew").is_file() {
        return Ok(Vec::new());
    }
    let output = std::process::Command::new(sdk.join("gradlew"))
        .current_dir(&sdk)
        .args(["--quiet", "--no-daemon", "--console=plain", "runtimeLicences"])
        .output()
        .map_err(|error| format!("android/sdk: cannot ask Gradle for the release runtime classpath: {error}"))?;
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
            noise[noise.len().saturating_sub(4)..].to_vec()
        } else {
            reason
        };
        // Gradle needs the JDK and Android SDK tools/env.sh finds; run by hand, a shell without them gets
        // Gradle's own complaint, so the message says where the environment comes from.
        return Err(format!(
            "android/sdk: `runtimeLicences` failed, so the licences of what ships cannot be checked: {} (tools/ci.sh rules runs it with the JDK and Android SDK tools/env.sh finds)",
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
    use crate::scratch::Scratch;

    const ALLOW: &[&str] = &["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-3-Clause", "MPL-2.0"];

    fn allow() -> Vec<String> {
        ALLOW.iter().map(|identifier| identifier.to_string()).collect()
    }

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
                scratch.write(CATALOG, catalog);
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
    fn a_compliant_repository_finds_nothing() {
        let repo = ScratchRepo::new(
            "clean",
            "[maven]\n\"junit:junit\" = \"MIT\"\n[swiftpm]\n\"https://github.com/mozilla/uniffi.git\" = \"MPL-2.0\"\n",
            "[libraries]\njunit = \"junit:junit:4.13.2\"\n",
            "// swift-tools-version: 6.0\nlet package = Package(dependencies: [.package(url: \"https://github.com/mozilla/uniffi.git\", from: \"0.32.0\")])\n",
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
    fn a_resolved_pin_needs_a_declaration_like_any_package() {
        let manifest =
            "// swift-tools-version: 6.0\nlet package = Package(dependencies: [.package(url: \"https://github.com/mozilla/uniffi.git\", from: \"0.32.0\")])\n";
        let repo = ScratchRepo::new("pins", "[swiftpm]\n\"https://github.com/mozilla/uniffi.git\" = \"MPL-2.0\"\n", "", manifest);
        assert!(repo.errors().is_empty(), "{:?}", repo.errors()); // no Package.resolved written yet
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
        let keys = gradle_libraries(catalog, "libs.versions.toml").unwrap();
        assert_eq!(keys, vec!["androidx.annotation:annotation", "net.java.dev.jna:jna", "org.jetbrains.kotlinx:kotlinx-coroutines-core"]);
        assert!(gradle_libraries("[libraries]\nbroken = { version.ref = \"x\" }\n", "c").is_err());
    }

    #[test]
    fn swiftpm_packages_survive_whitespace_and_extra_arguments() {
        let manifest = r#"
// swift-tools-version: 6.0
let package = Package(
    name: "CraftBridge",
    dependencies: [
        .package(url: "https://github.com/mozilla/uniffi.git",  from: "0.32.0"),
        .package(
            url: "https://github.com/tylermh/swift-format.git",
            branch: "main"
        ),
    ]
)
"#;
        assert_eq!(swiftpm_packages(manifest), vec!["https://github.com/mozilla/uniffi.git", "https://github.com/tylermh/swift-format.git"]);
        assert!(swiftpm_packages("// .package(url: \"https://commented/out.git\", from: \"1\")\n").is_empty(), "a commented declaration is not a dependency");
        let registry =
            "// é\nlet package = Package(name: \"é\", dependencies: [.package(id: \"mona.LinkedList\", from: \"1.0.0\"), .package(path: \"../Local\")])\n";
        assert_eq!(swiftpm_packages(registry), vec!["mona.LinkedList"], "a registry identity is a dependency, a local path is not");
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
        let used = [Used { key: "net.java.dev.jna:jna".to_string(), source: CATALOG.to_string(), kind: "library" }];
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
