//! Where the samples are, answered once for every platform: `cargo xtask swift-samples` and
//! `cargo xtask android-samples` print them for `tools/ci.sh`, which builds each, and the licence check and the
//! build-file rules read the same lists, so no sample is built and left unchecked, or checked and left unbuilt,
//! wherever under its platform's `samples` folder it sits.

use crate::scan;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The file Xcode opens a project by, inside its `<name>.xcodeproj` folder.
const XCODE_PROJECT: &str = "project.pbxproj";

/// The files Gradle takes a build's root folder by: the Kotlin settings file, and the Groovy one Gradle would read
/// in its place, which the build-file rules refuse and so must find.
const GRADLE_SETTINGS: [&str; 2] = ["settings.gradle.kts", "settings.gradle"];

/// The folders under `samples` that hold a file named one of `markers`, each once and in order, the walk
/// skipping build output and somebody else's packages as every rule does.
fn folders_holding(samples: &Path, markers: &[&str]) -> Result<Vec<PathBuf>, String> {
    let mut folders: Vec<PathBuf> = scan::walk(samples, "")?
        .into_iter()
        .filter(|file| file.path.file_name().and_then(|name| name.to_str()).is_some_and(|name| markers.contains(&name)))
        .filter_map(|file| file.path.parent().map(Path::to_path_buf))
        .collect();
    folders.sort();
    folders.dedup();
    Ok(folders)
}

/// Every Xcode project of the iOS samples, at any depth under `ios/samples`: a `<name>.xcodeproj` folder that holds
/// the `project.pbxproj` Xcode reads. `swift_build` builds each and the licence check reads each one's pins.
pub(crate) fn ios_projects(ios: &Path) -> Result<Vec<PathBuf>, String> {
    Ok(folders_holding(&ios.join("samples"), &[XCODE_PROJECT])?
        .into_iter()
        .filter(|project| project.extension().is_some_and(|extension| extension == "xcodeproj"))
        .collect())
}

/// Every Gradle build of the Android samples, at any depth under `android/samples`: a folder holding the settings
/// file Gradle takes a build's root by, and inside no other such folder, since a settings file inside a sample is
/// that sample's to answer for (the build-file rules refuse it). `android_samples` builds each, the licence check
/// reads each one's runtime classpaths, and the build-file rules read each one's build files.
pub(crate) fn android_builds(android: &Path) -> Result<Vec<PathBuf>, String> {
    let builds = folders_holding(&android.join("samples"), &GRADLE_SETTINGS)?;
    let outermost = builds.iter().filter(|build| !builds.iter().any(|other| other != *build && build.starts_with(other))).cloned().collect();
    Ok(outermost)
}

/// Every Swift file the iOS samples' projects compile, each project's as its one target's Sources build phase
/// lists it, read through `plutil` (`crate::plist`): a Swift file that sits in a sample's folder and is no
/// part of its project is compiled by no build, so it is no answer here. `swift_build` builds every project
/// [`ios_projects`] lists, in every configuration it ships, so these are the files those builds compile.
pub(crate) fn ios_sources(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for project in ios_projects(&root.join("ios"))? {
        files.extend(project_sources(&project)?);
    }
    Ok(files)
}

/// The Swift files the project at `project`, a `<name>.xcodeproj` folder, compiles: its one target's Sources
/// build phase, each file found where Xcode finds it — through the groups that hold it, from the folder the
/// project sits in, or by an absolute path. A project of several targets, a target that compiles a folder
/// Xcode keeps in step with the disk (whose files no list names) and a file found any other way are refused,
/// since what the gate's build compiles of them cannot be read here.
fn project_sources(project: &Path) -> Result<Vec<PathBuf>, String> {
    use serde_json::Value;
    let file = project.join(XCODE_PROJECT);
    let shown = file.display();
    let plist = crate::plist::read_as_json(&file)?;
    let objects = plist.get("objects").and_then(Value::as_object).ok_or_else(|| format!("{shown}: holds no objects"))?;
    let object = |id: &Value| id.as_str().and_then(|id| objects.get_key_value(id)).map(|(id, found)| (id.as_str(), found));
    let field = |found: &Value, key: &str| found.get(key).and_then(Value::as_str).map(str::to_string);
    let (_, root) = plist.get("rootObject").and_then(object).ok_or_else(|| format!("{shown}: names no project object"))?;
    let folder = project.parent().unwrap_or(project).join(field(root, "projectDirPath").unwrap_or_default());
    let main_group = field(root, "mainGroup").unwrap_or_default();
    let targets = root.get("targets").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let [target] = targets else {
        return Err(format!("{shown}: holds {} targets, and the check reads a project whose one target is the app its scheme builds", targets.len()));
    };
    let (_, target) = object(target).ok_or_else(|| format!("{shown}: names a target it does not hold"))?;
    let name = field(target, "name").unwrap_or_default();
    if target.get("fileSystemSynchronizedGroups").and_then(Value::as_array).is_some_and(|groups| !groups.is_empty()) {
        return Err(format!("{shown}: target {name} compiles folders Xcode keeps in step with the disk, whose files no list names"));
    }
    // Each group or file's group, from every group's list of children: Xcode finds a group-relative path
    // under the path of the group holding it.
    let mut parents = std::collections::BTreeMap::new();
    for (id, found) in objects {
        if field(found, "isa").as_deref() == Some("PBXGroup") {
            for child in found.get("children").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                parents.insert(child.to_string(), id.clone());
            }
        }
    }
    let located = |id: &str| -> Result<PathBuf, String> {
        // Up through the groups to the one whose path is not its group's, at most once per object, so a
        // project whose groups held each other ends rather than walking on.
        let mut parts = Vec::new();
        let mut at = id.to_string();
        for _ in 0..=objects.len() {
            let found = objects.get(&at).ok_or_else(|| format!("{shown}: names an object {at} it does not hold"))?;
            if let Some(path) = field(found, "path") {
                parts.push(path);
            }
            let start = match field(found, "sourceTree").as_deref() {
                Some("<group>") if at == main_group => Some(folder.clone()),
                Some("<group>") => None,
                Some("SOURCE_ROOT") => Some(folder.clone()),
                Some("<absolute>") => Some(PathBuf::new()),
                tree => {
                    let tree = tree.unwrap_or("nowhere");
                    return Err(format!("{shown}: target {name} compiles a file found from {tree}, which is no folder of the repository"));
                }
            };
            if let Some(start) = start {
                return Ok(parts.iter().rev().fold(start, |path, part| path.join(part)));
            }
            at = parents.get(&at).cloned().ok_or_else(|| format!("{shown}: target {name} compiles a file no group holds"))?;
        }
        Err(format!("{shown}: its groups hold each other"))
    };
    let mut files = Vec::new();
    let phases = target.get("buildPhases").and_then(Value::as_array).into_iter().flatten().filter_map(object);
    for (_, phase) in phases.filter(|(_, phase)| field(phase, "isa").as_deref() == Some("PBXSourcesBuildPhase")) {
        for (_, built) in phase.get("files").and_then(Value::as_array).into_iter().flatten().filter_map(object) {
            let (reference, found) =
                built.get("fileRef").and_then(object).ok_or_else(|| format!("{shown}: target {name} compiles something that is no file"))?;
            if field(found, "isa").as_deref() != Some("PBXFileReference") {
                return Err(format!("{shown}: target {name} compiles a {}, which is no single file", field(found, "isa").unwrap_or_default()));
            }
            let path = located(reference)?;
            if path.extension().is_some_and(|extension| extension == "swift") {
                files.push(path);
            }
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// Every Kotlin file of the Android samples' builds, which `android_samples` compiles: each build the
/// convention plugin sets up refuses a Kotlin file no compilation of its module reads (AGENTS §3), so every
/// Kotlin file under a build, build output aside, is one a compilation of that build reads.
pub(crate) fn android_sources(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for build in android_builds(&root.join("android"))? {
        files.extend(scan::walk(&build, ".kt")?.into_iter().map(|file| file.path));
    }
    Ok(files)
}

/// Prints `folders`, one path from the repository root per line, as `command`'s answer.
fn print(root: &Path, folders: Result<Vec<PathBuf>, String>, command: &str) -> ExitCode {
    let shown = folders.map(|folders| folders.iter().map(|folder| crate::report::shown(root, folder)).collect());
    crate::report::print_lines(shown, command)
}

/// `cargo xtask swift-samples`: the Xcode projects of the iOS samples.
pub(crate) fn run_swift(root: &Path) -> ExitCode {
    print(root, ios_projects(&root.join("ios")), "swift-samples")
}

/// `cargo xtask android-samples`: the Gradle builds of the Android samples.
pub(crate) fn run_android(root: &Path) -> ExitCode {
    print(root, android_builds(&root.join("android")), "android-samples")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    /// The folders `list` answers for a tree holding `files`, as the repository names them.
    fn listed(files: &[&str], list: fn(&Path) -> Result<Vec<PathBuf>, String>, platform: &str) -> Vec<String> {
        let scratch = Scratch::new("samples");
        for file in files {
            scratch.write(file, "");
        }
        list(&scratch.0.join(platform)).unwrap().iter().map(|folder| crate::report::shown(&scratch.0, folder)).collect()
    }

    /// The samples' projects are the `.xcodeproj` folders Xcode can open, however deep, and nothing in build
    /// output or somebody else's packages: what `swift_build` builds and what the licence check reads are this one
    /// list.
    #[test]
    fn the_ios_samples_are_every_project_xcode_opens_under_the_samples() {
        let files = [
            "ios/samples/App/App.xcodeproj/project.pbxproj",
            "ios/samples/Group/Other/Other.xcodeproj/project.pbxproj",
            "ios/samples/Empty/Empty.xcodeproj/project.xcworkspace/contents.xcworkspacedata",
            "ios/samples/Loose/project.pbxproj",
            "ios/samples/Backup/Backup.xcodeproj/old-project.pbxproj",
            "ios/samples/App/node_modules/Copy.xcodeproj/project.pbxproj",
        ];
        assert_eq!(listed(&files, ios_projects, "ios"), ["ios/samples/App/App.xcodeproj", "ios/samples/Group/Other/Other.xcodeproj"]);
    }

    /// A project file in the old text form Xcode writes, holding `objects` and naming the targets `targets`, its
    /// main group `MAIN`.
    fn project(objects: &str, targets: &str) -> String {
        format!(
            "// !$*UTF8*$!\n{{ archiveVersion = 1; objectVersion = 77; objects = {{ ROOT = {{ isa = PBXProject; mainGroup = MAIN; projectDirPath = \"\"; targets = ({targets}); }}; {objects} }}; rootObject = ROOT; }}\n"
        )
    }

    /// One app target, `APP`, whose Sources phase compiles `files` and whose Resources phase copies `BR`, beside
    /// `objects`.
    fn app(files: &str, objects: &str) -> String {
        project(
            &format!(
                "APP = {{ isa = PBXNativeTarget; name = App; buildPhases = (SOURCES, RESOURCES); }}; SOURCES = {{ isa = PBXSourcesBuildPhase; files = ({files}); }}; RESOURCES = {{ isa = PBXResourcesBuildPhase; files = (BR); }}; {objects}"
            ),
            "APP",
        )
    }

    /// The Swift files a sample's project compiles are the ones its target's Sources phase lists, each found
    /// where Xcode finds it — under the groups that hold it, from the project's folder, or by an absolute path —
    /// and never one that only sits in the folder, or one another phase only copies: no build compiles those.
    /// What the phase lists that is not Swift is no Swift source.
    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "reads the project through plutil, which only macOS has")]
    fn an_ios_samples_sources_are_what_its_project_compiles() {
        let scratch = Scratch::new("ios-sources");
        let objects = "MAIN = { isa = PBXGroup; children = (GROUP); sourceTree = \"<group>\"; }; \
            GROUP = { isa = PBXGroup; path = App; children = (NESTED, A, PLIST, R); sourceTree = \"<group>\"; }; \
            NESTED = { isa = PBXGroup; path = Screens; children = (B); sourceTree = \"<group>\"; }; \
            A = { isa = PBXFileReference; path = A.swift; sourceTree = \"<group>\"; }; \
            B = { isa = PBXFileReference; path = \"B View.swift\"; sourceTree = \"<group>\"; }; \
            C = { isa = PBXFileReference; path = Shared/C.swift; sourceTree = SOURCE_ROOT; }; \
            PLIST = { isa = PBXFileReference; path = Info.plist; sourceTree = \"<group>\"; }; \
            R = { isa = PBXFileReference; path = Copied.swift; sourceTree = \"<group>\"; }; BR = { isa = PBXBuildFile; fileRef = R; }; \
            BA = { isa = PBXBuildFile; fileRef = A; }; BB = { isa = PBXBuildFile; fileRef = B; }; \
            BC = { isa = PBXBuildFile; fileRef = C; }; BP = { isa = PBXBuildFile; fileRef = PLIST; };";
        scratch.write("ios/samples/App/App.xcodeproj/project.pbxproj", app("BA, BB, BC, BP", objects));
        scratch.write("ios/samples/App/App/Unlisted.swift", "");
        let sources: Vec<String> = ios_sources(&scratch.0).unwrap().iter().map(|file| crate::report::shown(&scratch.0, file)).collect();
        assert_eq!(sources, ["ios/samples/App/App/A.swift", "ios/samples/App/App/Screens/B View.swift", "ios/samples/App/Shared/C.swift"]);
    }

    /// What the check cannot read of a project is refused by name rather than read as nothing: a second target,
    /// a folder Xcode compiles whole, and a file found from a folder of the build, not of the repository.
    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "reads the project through plutil, which only macOS has")]
    fn a_project_whose_compiled_files_cannot_be_listed_is_refused() {
        let refusal = |text: String| {
            let scratch = Scratch::new("ios-sources-refused");
            scratch.write("ios/samples/App/App.xcodeproj/project.pbxproj", text);
            let file = scratch.0.join("ios/samples/App/App.xcodeproj/project.pbxproj");
            ios_sources(&scratch.0).unwrap_err().replace(&file.display().to_string(), "<project>")
        };
        let group = "MAIN = { isa = PBXGroup; children = (A); sourceTree = \"<group>\"; }; BA = { isa = PBXBuildFile; fileRef = A; };";
        assert_eq!(
            refusal(project(&format!("{group} A = {{ isa = PBXFileReference; path = A.swift; sourceTree = \"<group>\"; }};"), "APP, TESTS")),
            "<project>: holds 2 targets, and the check reads a project whose one target is the app its scheme builds"
        );
        assert_eq!(
            refusal(project(&format!("{group} APP = {{ isa = PBXNativeTarget; name = App; buildPhases = (); fileSystemSynchronizedGroups = (A); }};"), "APP")),
            "<project>: target App compiles folders Xcode keeps in step with the disk, whose files no list names"
        );
        assert_eq!(
            refusal(app("BA", &format!("{group} A = {{ isa = PBXFileReference; path = A.swift; sourceTree = BUILT_PRODUCTS_DIR; }};"))),
            "<project>: target App compiles a file found from BUILT_PRODUCTS_DIR, which is no folder of the repository"
        );
    }

    /// The Kotlin an Android sample's build compiles is every Kotlin file under it, whichever module or source
    /// set holds it, bar build output: the build refuses a Kotlin file none of its compilations reads.
    #[test]
    fn an_android_samples_sources_are_every_kotlin_file_of_its_build() {
        let scratch = Scratch::new("android-sources");
        for file in [
            "android/samples/one/settings.gradle.kts",
            "android/samples/one/app/build.gradle.kts",
            "android/samples/one/app/src/main/kotlin/A.kt",
            "android/samples/one/app/src/debug/kotlin/B.kt",
            "android/samples/one/app/build/generated/C.kt",
            "android/samples/one/app/src/main/kotlin/D.kts",
        ] {
            scratch.write(file, "");
        }
        let sources: Vec<String> = android_sources(&scratch.0).unwrap().iter().map(|file| crate::report::shown(&scratch.0, file)).collect();
        assert_eq!(sources, ["android/samples/one/app/src/debug/kotlin/B.kt", "android/samples/one/app/src/main/kotlin/A.kt"]);
    }

    /// The Android samples are the folders Gradle takes as a build's root, however deep, the Groovy settings file
    /// too, so the build-file rules see it to refuse it, and nothing in a build's output, in somebody else's
    /// packages, or inside another sample, whose own rules refuse a second settings file.
    #[test]
    fn the_android_samples_are_every_build_gradle_opens_under_the_samples() {
        let files = [
            "android/samples/one/settings.gradle.kts",
            "android/samples/one/app/build.gradle.kts",
            "android/samples/one/app/build/settings.gradle.kts",
            "android/samples/one/app/settings.gradle.kts",
            "android/samples/one/node_modules/copy/settings.gradle.kts",
            "android/samples/group/two/settings.gradle.kts",
            "android/samples/groovy/settings.gradle",
            "android/samples/both/settings.gradle.kts",
            "android/samples/both/settings.gradle",
            "android/samples/loose/build.gradle.kts",
            "android/samples/old/old-settings.gradle.kts",
        ];
        assert_eq!(
            listed(&files, android_builds, "android"),
            ["android/samples/both", "android/samples/groovy", "android/samples/group/two", "android/samples/one"]
        );
    }
}
