//! Step 7 of the update loop (plan §24): every version in `android/sdk/gradle/libs.versions.toml` and in
//! the Gradle wrapper that has a newer release, named for a person to take in a change of its own.
//!
//! The gate's Lint leaves `GradleDependency` and `AndroidGradlePluginVersion` off (AGENTS.md §10), so a
//! release elsewhere is never a fault of the change it gates, and this is where such releases are looked
//! for. The catalog is read as TOML through [`crate::pins`], the one reader of that file; each library's
//! releases come from the library's own published metadata — the `maven-metadata.xml` of Google's Maven,
//! of Maven Central or of the Gradle plugin portal, read as XML by `roxmltree` — and Gradle's from its
//! release service, read as JSON. Nothing here reads a web page. Every network read goes through one
//! [`Fetch`], so the tests give it fixtures and only a real run reaches the network.

use std::collections::BTreeMap;
use std::path::Path;

/// Where a published text comes from: the text at a URL, or the reason there is none.
pub(crate) type Fetch<'a> = &'a dyn Fn(&str) -> Result<String, String>;

/// The Gradle wrapper's properties file, the one place the wrapper names its Gradle.
const WRAPPER_PROPERTIES: &str = "android/sdk/gradle/wrapper/gradle-wrapper.properties";

/// Gradle's own release service: every release, with the flags that tell a release from a snapshot.
const GRADLE_RELEASES: &str = "https://services.gradle.org/versions/all";

/// Versions of the catalog that are platform levels rather than releases of a library, so no repository
/// holds a newer one to name. The machine-installed levels — an SDK, its build tools, a JDK — are the
/// keys `cargo xtask pins` prints and the doctor checks, listed once in [`crate::pins::CATALOG_PINS`];
/// these two are levels the build aims at — the oldest Android API it supports, the Java bytecode it
/// emits — which are choices of the build, not versions that gain newer releases.
const BUILD_TARGET_LEVELS: [&str; 2] = ["android-minSdk", "java-bytecode"];

/// Versions of the catalog that no library or plugin of it names, with the artifact that publishes them:
/// ktlint is run by Spotless from the version alone, so the catalog holds no library for it to read.
const UNREFERENCED: [(&str, &str, &str); 1] = [("ktlint", "com.pinterest.ktlint", "ktlint-cli")];

/// A published artifact, which names the repository that holds it by its group.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Artifact {
    group: String,
    name: String,
    /// Published as the marker of a Gradle plugin, which only the plugin portal holds.
    plugin_marker: bool,
}

impl Artifact {
    /// The URL of this artifact's `maven-metadata.xml`. Google's Maven holds the Android and AndroidX
    /// groups, the markers of Android's own plugins among them, the plugin portal every other plugin's
    /// marker, and Maven Central the rest.
    fn metadata_url(&self) -> String {
        let google = ["com.android.", "androidx.", "com.google.android."].iter().any(|prefix| format!("{}.", self.group).starts_with(prefix));
        let base = if google {
            "https://dl.google.com/dl/android/maven2"
        } else if self.plugin_marker {
            "https://plugins.gradle.org/m2"
        } else {
            "https://repo.maven.apache.org/maven2"
        };
        format!("{base}/{}/{}/maven-metadata.xml", self.group.replace('.', "/"), self.name)
    }

    fn coordinates(&self) -> String {
        format!("{}:{}", self.group, self.name)
    }
}

/// One version the catalog pins, with the artifact that publishes it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Pinned {
    /// What a message calls it: the catalog's version key, or a library's alias when it carries its own.
    name: String,
    version: String,
    artifact: Artifact,
}

/// A pinned version that has a newer release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Newer {
    pub(crate) name: String,
    pub(crate) current: String,
    /// The newest release after `current`.
    pub(crate) newest: String,
    /// Where it was found: an artifact's coordinates or the Gradle distribution.
    pub(crate) from: String,
}

impl Newer {
    pub(crate) fn line(&self) -> String {
        format!("{} {} ({}) has a newer release, {}", self.name, self.current, self.from, self.newest)
    }
}

fn string_of<'a>(table: &'a toml::Value, key: &str) -> Option<&'a str> {
    table.get(key).and_then(toml::Value::as_str)
}

/// The version a catalog library or plugin carries: a reference to `[versions]`, a string of its own, or
/// none, as a library whose version a platform or BOM decides has none.
enum Version<'a> {
    Reference(&'a str),
    Literal(&'a str),
    Unpinned,
}

/// A catalog library or plugin as Gradle reads it: the artifact that publishes it and its version.
/// Gradle takes `"group:name:version"` (`"id:version"` for a plugin) as well as a table, so both are
/// read. A rich version (`strictly`, `require`, `prefer`) or an entry this cannot read is an error
/// naming it, never a version passed over.
fn entry_of<'a>(kind: &str, alias: &str, entry: &'a toml::Value) -> Result<(Artifact, Version<'a>), String> {
    let unreadable = || format!("{kind} {alias} is in no form this reads, so its newer releases would never be looked for");
    let marker = |id: &str| Artifact { group: id.to_string(), name: format!("{id}.gradle.plugin"), plugin_marker: true };
    let library = |group: &str, name: &str| Artifact { group: group.to_string(), name: name.to_string(), plugin_marker: false };
    if let Some(notation) = entry.as_str() {
        let parts: Vec<&str> = notation.split(':').collect();
        return match (kind, parts.as_slice()) {
            ("plugin", [id, version]) => Ok((marker(id), Version::Literal(version))),
            ("library", [group, name, version]) => Ok((library(group, name), Version::Literal(version))),
            _ => Err(unreadable()),
        };
    }
    let artifact = match (kind, string_of(entry, "id"), string_of(entry, "module")) {
        ("plugin", Some(id), _) => marker(id),
        ("library", _, Some(module)) => module.split_once(':').map(|(group, name)| library(group, name)).ok_or_else(unreadable)?,
        ("library", _, None) => library(string_of(entry, "group").ok_or_else(unreadable)?, string_of(entry, "name").ok_or_else(unreadable)?),
        _ => return Err(unreadable()),
    };
    let version = match entry.get("version") {
        None => Version::Unpinned,
        Some(toml::Value::String(literal)) => Version::Literal(literal),
        Some(table) => string_of(table, "ref")
            .map(Version::Reference)
            .ok_or_else(|| format!("{kind} {alias} carries a rich version, which names no one release to look a newer one up for"))?,
    };
    Ok((artifact, version))
}

/// Every version the catalog pins that a release can follow, with the artifact that publishes it, read
/// from the catalog's TOML. A key of `[versions]` that no library or plugin names, that is no platform
/// level and that [`UNREFERENCED`] does not know is an error: a version this step cannot look up would
/// go unchecked for ever without anyone knowing.
fn pinned(catalog: &toml::Value) -> Result<Vec<Pinned>, String> {
    let versions = catalog.get("versions").and_then(toml::Value::as_table).ok_or("the catalog names no [versions] table")?;
    let table = |name: &str| catalog.get(name).and_then(toml::Value::as_table).into_iter().flat_map(|table| table.iter());
    let mut found: BTreeMap<String, Pinned> = BTreeMap::new();
    // Libraries first: a plugin whose version a library shares is looked up through that library.
    for (kind, alias, entry) in
        table("libraries").map(|(alias, entry)| ("library", alias, entry)).chain(table("plugins").map(|(alias, entry)| ("plugin", alias, entry)))
    {
        let (artifact, version) = entry_of(kind, alias, entry)?;
        let called = if kind == "plugin" { format!("plugin {alias}") } else { alias.to_string() };
        match version {
            Version::Reference(key) => {
                let version = versions
                    .get(key)
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| format!("{kind} {alias} refers to version {key}, which [versions] does not hold as a string"))?;
                let name = if kind == "plugin" { format!("{key} ({called})") } else { key.to_string() };
                found.entry(key.to_string()).or_insert(Pinned { name, version: version.to_string(), artifact });
            }
            Version::Literal(version) => {
                found.insert(format!("{kind} {alias}"), Pinned { name: called, version: version.to_string(), artifact });
            }
            Version::Unpinned => {}
        }
    }
    for (key, value) in versions {
        if found.contains_key(key) || crate::pins::CATALOG_PINS.contains(&key.as_str()) || BUILD_TARGET_LEVELS.contains(&key.as_str()) {
            continue;
        }
        let version = value.as_str().ok_or_else(|| format!("version {key} is no string: a release cannot be looked up for it"))?;
        match UNREFERENCED.iter().find(|(known, ..)| known == key) {
            Some((_, group, name)) => {
                found.insert(
                    key.clone(),
                    Pinned {
                        name: key.clone(),
                        version: version.to_string(),
                        artifact: Artifact { group: group.to_string(), name: name.to_string(), plugin_marker: false },
                    },
                );
            }
            None => {
                return Err(format!(
                    "version {key} is named by no library or plugin of the catalog, is no platform level and is not one of the artifacts rust/xtask/src/versions.rs knows: its newer releases would never be looked for"
                ));
            }
        }
    }
    Ok(found.into_values().collect())
}

/// A release with no qualifier: digits and dots. Everything else — `alpha`, `beta`, `rc`, a milestone, a
/// snapshot — is a preview, which nobody is told to take in place of a release.
fn is_release(version: &str) -> bool {
    !version.is_empty() && version.chars().all(|character| character.is_ascii_digit() || character == '.')
}

/// The versions a `maven-metadata.xml` lists, in the order the repository lists them: Maven Central's and
/// Google's lists hold their releases in version order.
fn published_versions(metadata: &str) -> Result<Vec<String>, String> {
    let document = roxmltree::Document::parse(metadata).map_err(|error| format!("the repository's metadata is no XML: {error}"))?;
    let listed: Vec<String> = document
        .descendants()
        .filter(|node| node.has_tag_name("versions"))
        .flat_map(|versions| versions.children().filter(|node| node.has_tag_name("version")))
        .filter_map(|node| node.text().map(|text| text.trim().to_string()))
        .collect();
    if listed.is_empty() {
        return Err("the repository's metadata lists no versions".to_string());
    }
    Ok(listed)
}

/// The newest release after `current` in a list in version order, oldest first. A release is only ever
/// followed by a release; a preview is followed by anything newer, since the catalog took it on purpose.
fn newest_after<S: AsRef<str>>(published: &[S], current: &str) -> Result<Option<String>, String> {
    let at = published.iter().position(|version| version.as_ref() == current).ok_or_else(|| format!("{current} is not among the versions listed"))?;
    Ok(published.iter().skip(at.saturating_add(1)).map(AsRef::as_ref).rev().find(|version| !is_release(current) || is_release(version)).map(str::to_string))
}

/// The Gradle a wrapper properties file names: the version in its `distributionUrl`, a file name of the
/// form `gradle-<version>-bin.zip` or `-all.zip`. The file is a Java properties file, whose `\:` and `\=`
/// escapes stand for the characters they precede.
fn wrapper_version(properties: &str) -> Result<String, String> {
    let url = properties
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#') && !line.starts_with('!'))
        .find_map(|line| {
            // The key ends where its separator, `=`, `:` or a blank, begins: `distributionUrlX` is another key.
            let rest = line.strip_prefix("distributionUrl").filter(|rest| rest.starts_with(['=', ':', ' ', '\t']))?;
            Some(rest.trim_start().trim_start_matches(['=', ':']).trim().replace("\\:", ":").replace("\\=", "="))
        })
        .ok_or_else(|| format!("{WRAPPER_PROPERTIES} names no distributionUrl"))?;
    let file = url.rsplit('/').next().unwrap_or_default();
    file.strip_prefix("gradle-")
        .and_then(|rest| rest.strip_suffix("-bin.zip").or_else(|| rest.strip_suffix("-all.zip")))
        .map(str::to_string)
        .ok_or_else(|| format!("{WRAPPER_PROPERTIES}: the distribution {url} is not a gradle-<version>-bin.zip or -all.zip"))
}

/// A release's numbers, so two releases compare as numbers: `9.10.0` follows `9.9.1`.
fn numbers(release: &str) -> Vec<u64> {
    release.split('.').map(|part| part.parse().unwrap_or(0)).collect()
}

/// The newest Gradle release after `current` in the release service's list, which is in no order of its
/// own, so its releases are put in version order first: a release is one the service flags as no
/// snapshot, nightly, release candidate, milestone or broken build.
fn newest_gradle(releases: &str, current: &str) -> Result<Option<String>, String> {
    let list: serde_json::Value = serde_json::from_str(releases).map_err(|error| format!("Gradle's release service answered no JSON: {error}"))?;
    let entries = list.as_array().ok_or("Gradle's release service answered no list")?;
    let flagged = |entry: &serde_json::Value, flag: &str| entry.get(flag).and_then(serde_json::Value::as_bool).unwrap_or(false);
    let named = |entry: &serde_json::Value, key: &str| entry.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().is_empty();
    let mut releases: Vec<&str> = entries
        .iter()
        .filter(|entry| {
            !flagged(entry, "snapshot") && !flagged(entry, "nightly") && !flagged(entry, "broken") && named(entry, "rcFor") && named(entry, "milestoneFor")
        })
        .filter_map(|entry| entry.get("version").and_then(serde_json::Value::as_str))
        .filter(|version| is_release(version))
        .collect();
    releases.sort_by_key(|release| numbers(release));
    newest_after(&releases, current).map_err(|error| format!("{error} as releases by Gradle's release service"))
}

/// Every pinned version with a newer release, from a catalog and a wrapper properties file read as text,
/// asking `fetch` for the repositories' metadata and Gradle's releases. A lookup that fails is an error
/// naming what it was for: a step that says nothing for want of an answer has told no one.
pub(crate) fn newer_releases(catalog: &toml::Value, wrapper: &str, fetch: Fetch<'_>) -> Result<Vec<Newer>, String> {
    let mut found = Vec::new();
    for pin in pinned(catalog)? {
        let url = pin.artifact.metadata_url();
        let metadata = fetch(&url).map_err(|error| format!("{} ({}): {error}", pin.name, pin.artifact.coordinates()))?;
        let published = published_versions(&metadata).map_err(|error| format!("{} ({}): {error}", pin.name, url))?;
        if let Some(newest) = newest_after(&published, &pin.version).map_err(|error| format!("{} ({}): {error}", pin.name, pin.artifact.coordinates()))? {
            found.push(Newer { name: pin.name, current: pin.version, newest, from: pin.artifact.coordinates() });
        }
    }
    let gradle = wrapper_version(wrapper)?;
    let releases = fetch(GRADLE_RELEASES).map_err(|error| format!("Gradle {gradle}: {error}"))?;
    if let Some(newest) = newest_gradle(&releases, &gradle).map_err(|error| format!("Gradle {gradle}: {error}"))? {
        found.push(Newer { name: "gradle wrapper".to_string(), current: gradle, newest, from: GRADLE_RELEASES.to_string() });
    }
    Ok(found)
}

/// The same, reading the catalog and the wrapper from the repository and the releases from the network:
/// what a real run does.
pub(crate) fn newer_releases_in(root: &Path, fetch: Fetch<'_>) -> Result<Vec<Newer>, String> {
    let catalog = crate::pins::catalog(root)?
        .ok_or_else(|| format!("{} does not exist: there is no Android build to look for newer releases of", crate::pins::CATALOG))?;
    let wrapper = crate::scan::read_text(&root.join(WRAPPER_PROPERTIES))?;
    newer_releases(&catalog, &wrapper, fetch)
}

/// The text at `url`, read by `curl`, which follows redirects and fails on an HTTP error.
pub(crate) fn curl(url: &str) -> Result<String, String> {
    let output = std::process::Command::new("curl")
        // `--disable` first: the user's .curlrc would otherwise add options of its own to the request.
        .args(["--disable", "--fail", "--silent", "--show-error", "--location", "--max-time", "60", url])
        .output()
        .map_err(|error| format!("cannot run curl: {error}"))?;
    if !output.status.success() {
        return Err(format!("curl {url} failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    String::from_utf8(output.stdout).map_err(|_| format!("{url} answered something that is no UTF-8 text"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A catalog written as the repository's is: references, a plugin that names the same key as a
    /// library, one that names its own, a library with a version of its own, and the platform levels.
    const CATALOG: &str = r#"
[versions]
android-compileSdk = "37"
jdk = "21"
agp = "9.5.1"
spotless = "8.11.0"
ktlint = "1.8.0"
detekt = "2.0.0-alpha.6"
[plugins]
android-library = { id = "com.android.library", version.ref = "agp" }
spotless = { id = "com.diffplug.spotless", version.ref = "spotless" }
[libraries]
android-gradle-plugin = { module = "com.android.tools.build:gradle", version.ref = "agp" }
detekt-cli = { module = "dev.detekt:detekt-cli", version.ref = "detekt" }
detekt-api = { module = "dev.detekt:detekt-api", version.ref = "detekt" }
guava = { group = "com.google.guava", name = "guava", version = "33.0.0" }
"#;

    const WRAPPER: &str =
        "distributionBase=GRADLE_USER_HOME\ndistributionUrl=https\\://services.gradle.org/distributions/gradle-9.9.1-bin.zip\nzipStorePath=wrapper/dists\n";

    fn metadata(versions: &[&str]) -> String {
        let listed: String = versions.iter().map(|version| format!("      <version>{version}</version>\n")).collect();
        format!(
            "<?xml version=\"1.0\"?>\n<metadata>\n  <versioning>\n    <latest>x</latest>\n    <versions>\n{listed}    </versions>\n  </versioning>\n</metadata>\n"
        )
    }

    /// Newest first, then an older line's patch, as the service lists them: no order a position or a
    /// string comparison can read.
    const GRADLE: &str = r#"[
      {"version":"9.10.0-20261116003216+0000","snapshot":true,"nightly":true,"rcFor":"","milestoneFor":"","broken":false},
      {"version":"9.12.0-rc-1","snapshot":false,"nightly":false,"rcFor":"9.12.0","milestoneFor":"","broken":false},
      {"version":"9.11.0","snapshot":false,"nightly":false,"rcFor":"","milestoneFor":"","broken":true},
      {"version":"9.10.0","snapshot":false,"nightly":false,"rcFor":"","milestoneFor":"","broken":false},
      {"version":"8.14.7","snapshot":false,"nightly":false,"rcFor":"","milestoneFor":"","broken":false},
      {"version":"9.9.1","snapshot":false,"nightly":false,"rcFor":"","milestoneFor":"","broken":false}
    ]"#;

    /// A fetch that answers from a table of URLs, as the repositories would, and fails for any other.
    fn answers(table: Vec<(String, String)>) -> impl Fn(&str) -> Result<String, String> {
        move |url| table.iter().find(|(known, _)| known == url).map(|(_, text)| text.clone()).ok_or_else(|| format!("no fixture for {url}"))
    }

    fn fixture_table(agp: &[&str]) -> Vec<(String, String)> {
        let catalog: toml::Value = toml::from_str(CATALOG).unwrap();
        let mut table: Vec<(String, String)> = pinned(&catalog)
            .unwrap()
            .into_iter()
            .map(|pin| {
                let listed: Vec<&str> = if pin.artifact.name == "gradle" { agp.to_vec() } else { vec![pin.version.as_str()] };
                (pin.artifact.metadata_url(), metadata(&listed))
            })
            .collect();
        table.push((GRADLE_RELEASES.to_string(), GRADLE.to_string()));
        table
    }

    #[test]
    fn a_catalog_version_with_a_newer_release_is_named_with_that_release() {
        let catalog: toml::Value = toml::from_str(CATALOG).unwrap();
        let fetch = answers(fixture_table(&["9.4.0", "9.5.0", "9.5.1", "9.5.2", "9.6.0-alpha04"]));
        let found = newer_releases(&catalog, WRAPPER, &fetch).unwrap();
        let agp = found.iter().find(|newer| newer.current == "9.5.1").expect("the outdated AGP is named");
        assert_eq!((agp.newest.as_str(), agp.from.as_str()), ("9.5.2", "com.android.tools.build:gradle"), "{found:?}");
        assert!(agp.line().contains("9.5.1") && agp.line().contains("9.5.2"), "{}", agp.line());
        let gradle = found.iter().find(|newer| newer.name == "gradle wrapper").expect("the wrapper's Gradle is named");
        assert_eq!((gradle.current.as_str(), gradle.newest.as_str()), ("9.9.1", "9.10.0"), "the broken 9.11.0 and the release candidate are no release");
        assert_eq!(found.len(), 2, "every other version is at its newest: {found:?}");
    }

    #[test]
    fn a_catalog_at_its_newest_releases_names_nothing() {
        let catalog: toml::Value = toml::from_str(CATALOG).unwrap();
        let mut table = fixture_table(&["9.5.0", "9.5.1"]);
        table.retain(|(url, _)| url != GRADLE_RELEASES);
        table.push((GRADLE_RELEASES.to_string(), GRADLE.replace("\"9.10.0\"", "\"9.9.0\"")));
        assert_eq!(newer_releases(&catalog, WRAPPER, &answers(table)).unwrap(), []);
    }

    #[test]
    fn a_preview_is_followed_by_anything_newer_and_a_release_only_by_a_release() {
        let published: Vec<String> = ["2.0.0-alpha.5", "2.0.0-alpha.6", "2.0.0-alpha.7", "2.0.0"].iter().map(ToString::to_string).collect();
        assert_eq!(newest_after(&published, "2.0.0-alpha.6"), Ok(Some("2.0.0".to_string())));
        let beside_previews: Vec<String> = ["9.5.1", "9.6.0-alpha01"].iter().map(ToString::to_string).collect();
        assert_eq!(newest_after(&beside_previews, "9.5.1"), Ok(None), "a release is not told to move to a preview");
        let unhyphenated: Vec<String> = ["1.0.0", "1.1.0.RC1", "1.1.0M2", "1.1.0.Beta"].iter().map(ToString::to_string).collect();
        assert_eq!(newest_after(&unhyphenated, "1.0.0"), Ok(None), "a qualifier is a preview with or without a hyphen: only digits and dots make a release");
        assert!(newest_after(&beside_previews, "9.4.0").is_err_and(|error| error.contains("9.4.0")), "a version the repository does not list is no answer");
    }

    #[test]
    fn the_repository_is_the_one_that_holds_the_artifact() {
        let artifact = |group: &str, name: &str, plugin_marker| Artifact { group: group.to_string(), name: name.to_string(), plugin_marker };
        assert_eq!(
            artifact("com.android.tools.build", "gradle", false).metadata_url(),
            "https://dl.google.com/dl/android/maven2/com/android/tools/build/gradle/maven-metadata.xml"
        );
        assert_eq!(artifact("dev.detekt", "detekt-cli", false).metadata_url(), "https://repo.maven.apache.org/maven2/dev/detekt/detekt-cli/maven-metadata.xml");
        assert_eq!(
            artifact("org.example", "org.example.gradle.plugin", true).metadata_url(),
            "https://plugins.gradle.org/m2/org/example/org.example.gradle.plugin/maven-metadata.xml"
        );
        assert_eq!(
            artifact("com.android.application", "com.android.application.gradle.plugin", true).metadata_url(),
            "https://dl.google.com/dl/android/maven2/com/android/application/com.android.application.gradle.plugin/maven-metadata.xml",
            "Android's own plugins publish their markers to Google's Maven, which the portal does not hold"
        );
        assert!(
            artifact("com.androidx", "x", false).metadata_url().starts_with("https://repo.maven.apache.org/"),
            "a group is matched whole, segment by segment"
        );
    }

    #[test]
    fn every_form_gradle_reads_is_looked_up_or_refused_by_name() {
        let catalog: toml::Value = toml::from_str(
            "[versions]\n[plugins]\nshadow = \"com.gradleup.shadow:9.0.0\"\nversioned = { id = \"org.example.tool\", version = \"2.1\" }\n[libraries]\ngson = \"com.google.code.gson:gson:2.13.1\"\nno-version = { module = \"androidx.core:core\" }\n",
        )
        .unwrap();
        let found: Vec<(String, String, String)> =
            pinned(&catalog).unwrap().into_iter().map(|pin| (pin.name, pin.version, pin.artifact.coordinates())).collect();
        assert_eq!(
            found,
            [
                ("gson".to_string(), "2.13.1".to_string(), "com.google.code.gson:gson".to_string()),
                ("plugin shadow".to_string(), "9.0.0".to_string(), "com.gradleup.shadow:com.gradleup.shadow.gradle.plugin".to_string()),
                ("plugin versioned".to_string(), "2.1".to_string(), "org.example.tool:org.example.tool.gradle.plugin".to_string()),
            ],
            "a string notation and a plugin's own version are looked up; a library with no version pins none"
        );
        for (entry, named) in [
            ("[libraries]\nrich = { module = \"a:b\", version = { strictly = \"1.0\" } }\n", "library rich"),
            ("[libraries]\nodd = \"a:b:c:d\"\n", "library odd"),
            ("[libraries]\nshort = \"a:b\"\n", "library short"),
            ("[libraries]\nbare = { module = \"ab\", version = \"1\" }\n", "library bare"),
            ("[plugins]\nloose = { version = \"1\" }\n", "plugin loose"),
            ("[plugins]\ndangling = { id = \"a.b\", version.ref = \"missing\" }\n", "plugin dangling"),
        ] {
            let catalog: toml::Value = toml::from_str(&format!("[versions]\n{entry}")).unwrap();
            assert!(pinned(&catalog).is_err_and(|error| error.contains(named)), "{named} is named, not passed over");
        }
    }

    #[test]
    fn a_version_nothing_can_look_up_is_an_error_not_a_silence() {
        let catalog: toml::Value = toml::from_str("[versions]\nmystery = \"1.0\"\n").unwrap();
        assert!(pinned(&catalog).is_err_and(|error| error.contains("mystery")));
        let platform: toml::Value = toml::from_str("[versions]\njdk = \"21\"\nktlint = \"1.8.0\"\n").unwrap();
        let pins = pinned(&platform).unwrap();
        assert_eq!(pins.len(), 1, "the JDK is a platform level; ktlint is looked up by the table's artifact");
        assert_eq!(pins.first().map(|pin| pin.artifact.coordinates()), Some("com.pinterest.ktlint:ktlint-cli".to_string()));
    }

    #[test]
    fn a_failed_lookup_is_named_and_stops_the_step() {
        let catalog: toml::Value = toml::from_str(CATALOG).unwrap();
        let error = newer_releases(&catalog, WRAPPER, &|_| Err("offline".to_string())).unwrap_err();
        assert!(error.contains("offline") && error.contains(':'), "{error}");
    }

    #[test]
    fn the_wrapper_names_its_gradle_in_any_spelling_the_properties_format_allows() {
        assert_eq!(wrapper_version(WRAPPER).as_deref(), Ok("9.9.1"));
        assert_eq!(
            wrapper_version("# a comment\ndistributionUrl = https\\://services.gradle.org/distributions/gradle-8.14.7-all.zip\n").as_deref(),
            Ok("8.14.7")
        );
        assert_eq!(wrapper_version("distributionUrl: https://example.invalid/gradle-9.0-bin.zip\n").as_deref(), Ok("9.0"));
        assert!(wrapper_version("zipStorePath=wrapper/dists\n").is_err_and(|error| error.contains("distributionUrl")));
        assert!(wrapper_version("distributionUrl=https\\://example.invalid/other.zip\n").is_err());
        assert!(
            wrapper_version(
                "distributionUrlLegacy=https\\://example.invalid/gradle-1.0-bin.zip\ndistributionUrl=https\\://example.invalid/gradle-9.9.1-bin.zip\n"
            )
            .as_deref()
                == Ok("9.9.1"),
            "a key that only starts the same is another key"
        );
    }

    #[test]
    fn the_real_catalog_and_wrapper_are_all_looked_up() {
        let root = crate::repo_root();
        let catalog = crate::pins::catalog(&root).unwrap().expect("the repository has an Android build");
        let pins = pinned(&catalog).unwrap();
        for key in ["agp", "kotlin", "spotless", "ktlint", "detekt"] {
            assert!(pins.iter().any(|pin| pin.name.starts_with(key)), "{key} is not looked up: {pins:?}");
        }
        let wrapper = std::fs::read_to_string(root.join(WRAPPER_PROPERTIES)).unwrap();
        assert!(wrapper_version(&wrapper).is_ok());
    }
}
