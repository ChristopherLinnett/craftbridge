//! Where the samples are, answered once for every platform: `cargo xtask swift-samples` and
//! `cargo xtask android-samples` print them for `tools/ci.sh`, which builds each, and the licence check and the
//! build-file rules read the same lists, so no sample is built and left unchecked, or checked and left unbuilt,
//! wherever under its platform's `samples` folder it sits.

use crate::scan;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

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

/// The Swift files the project at `project`, a `<name>.xcodeproj` folder, compiles: its one target's Sources
/// build phase, each file found where Xcode finds it — through the groups that hold it, from the folder the
/// project sits in, or by an absolute path. A project of several targets, a target that compiles a folder
/// Xcode keeps in step with the disk (whose files no list names) and a file found any other way are refused,
/// since what the gate's build compiles of them cannot be read here.
///
/// A Swift file that sits in a sample's folder and is no part of its project is compiled by no build, so it
/// is no answer here; `swift_build` builds every project [`ios_projects`] lists, in every configuration it
/// ships, so these are the files those builds compile.
pub(crate) fn project_build(project: &Path) -> Result<ProjectBuild, String> {
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
    Ok(ProjectBuild { target: name, sources: files })
}

/// What a sample's project builds: its one target, by name, and the Swift files that target compiles.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ProjectBuild {
    pub(crate) target: String,
    pub(crate) sources: Vec<PathBuf>,
}

/// The guide check's read of one compiled file: its lines and the lines the build compiles to nothing.
fn guide_source(path: &Path, dead: std::collections::BTreeSet<usize>) -> Result<crate::guide_samples::Source, String> {
    let text = scan::read_text(path)?;
    Ok(crate::guide_samples::Source { path: path.to_path_buf(), lines: text.lines().map(str::to_string).collect(), dead })
}

/// The files a guide answer names as compiled, in answer order, and each one's dead lines. An answer is
/// lines of three kinds, each with its fields between tabs — `S` and a file a build compiles, then `I`,
/// that file and one of its lines that compile to nothing, or `E`, a file and why it could not be read —
/// and nothing else: the writers, the Kotlin builds' `guideKotlin` and `tools/guide-swift-lines.swift`,
/// write into a file or a stream of their own, so a line of any other shape is no noise to pass over but
/// an answer this cannot read, and is refused, since a dead line the check missed and one it invented are
/// the same failure.
fn guide_answers(answer: &str) -> Result<(Vec<PathBuf>, std::collections::BTreeMap<PathBuf, std::collections::BTreeSet<usize>>), String> {
    let mut compiled: Vec<PathBuf> = Vec::new();
    let mut dead: std::collections::BTreeMap<PathBuf, std::collections::BTreeSet<usize>> = std::collections::BTreeMap::new();
    for line in answer.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["S", file] if !file.is_empty() => compiled.push(PathBuf::from(file)),
            ["I", file, number] => {
                let file = PathBuf::from(file);
                let number = number
                    .parse::<usize>()
                    .ok()
                    .filter(|number| *number > 0)
                    .ok_or_else(|| format!("{line:?}: an inert-line answer names no line number it can read"))?;
                if !compiled.contains(&file) {
                    return Err(format!("{line:?}: an inert-line answer names a file no answer before it said was compiled"));
                }
                dead.entry(file).or_default().insert(number);
            }
            ["E", file, reason] => return Err(format!("{file}: {reason}")),
            _ => return Err(format!("{line:?}: no answer the guide protocol knows: `S`, `I` or `E`, each with its own fields between tabs")),
        }
    }
    Ok((compiled, dead))
}

/// The variable a sample build's `guideKotlin` takes its answer folder from, as the convention plugin reads
/// it (`android/sdk/build-logic/convention/.../CraftbridgeGuideKotlin.kt`): read when the task runs, so a
/// new folder each ask leaves the build's stored configuration as it was.
const GUIDE_ANSWERS: &str = "CRAFTBRIDGE_GUIDE_ANSWERS";

/// The Kotlin files the Android samples' builds compile, each with the lines they compile to nothing.
/// Each sample build answers its own through `guideKotlin`, the detekt run the convention plugin sets
/// beside the checks every module already makes: its folders are the variants' own as AGP's variant API
/// lists them, never a second list, and its dead lines are `CompilesToNothing`, a rule of the gate's
/// own detekt rules, reading the Kotlin with detekt's parser (§10).
pub(crate) fn android_guide_sources(root: &Path) -> Result<Vec<crate::guide_samples::Source>, String> {
    let wrapper = root.join("android").join("sdk").join("gradlew");
    let mut compiled: Vec<PathBuf> = Vec::new();
    let mut dead: std::collections::BTreeMap<PathBuf, std::collections::BTreeSet<usize>> = std::collections::BTreeMap::new();
    for build in android_builds(&root.join("android"))? {
        let folder = crate::scratch::create("guide-kotlin")?;
        let answer = kotlin_guide_answer(&wrapper, &build, &folder);
        let _ = std::fs::remove_dir_all(&folder);
        let (files, lines) = guide_answers(&answer?)?;
        compiled.extend(files);
        dead.extend(lines);
    }
    compiled.sort();
    compiled.dedup();
    compiled.into_iter().map(|file| guide_source(&file, dead.remove(&file).unwrap_or_default())).collect()
}

/// What the build at `build` answers through `guideKotlin`, every module's answer file in `folder` read in
/// turn: a build that fails, or answers into no file at all, is refused in its own words, never read as a
/// build that compiles no Kotlin.
fn kotlin_guide_answer(wrapper: &Path, build: &Path, folder: &Path) -> Result<String, String> {
    let run = Command::new(wrapper)
        .arg("-p")
        .arg(build)
        .args(["--no-daemon", "--continue", "--warning-mode=fail", "--console=plain", "guideKotlin"])
        .env(GUIDE_ANSWERS, folder)
        .output()
        .map_err(|failure| format!("{} could not run to ask what {} compiles: {failure}", wrapper.display(), build.display()))?;
    if !run.status.success() {
        return Err(format!(
            "{}: the build at {} answered no guide Kotlin: {}",
            wrapper.display(),
            build.display(),
            String::from_utf8_lossy(&run.stderr).trim()
        ));
    }
    let mut answers: Vec<PathBuf> =
        std::fs::read_dir(folder).map_err(|failure| format!("{}: {failure}", folder.display()))?.filter_map(Result::ok).map(|entry| entry.path()).collect();
    answers.sort();
    if answers.is_empty() {
        return Err(format!("{}: the build at {} ran guideKotlin and wrote no answer into {}", wrapper.display(), build.display(), folder.display()));
    }
    answers.iter().map(|answer| scan::read_text(answer)).collect::<Result<Vec<_>, _>>().map(|texts| texts.concat())
}

/// The conditions `configuration` of `target`, in the project at `project`, compiles its Swift under for
/// `sdk`, as Xcode resolves them: `xcodebuild -showBuildSettings` reads the target's settings over the
/// project's, `$(inherited)`, a configuration file and a setting written for one SDK alone the way the
/// build `swift_build` makes of the sample reads them, so nothing here reads the settings a second way.
fn swift_conditions(project: &Path, target: &str, configuration: &str, sdk: &str) -> Result<Vec<String>, String> {
    use serde_json::Value;
    let said = Command::new("xcodebuild")
        .arg("-showBuildSettings")
        .arg("-json")
        .arg("-project")
        .arg(project)
        .args(["-target", target, "-configuration", configuration, "-sdk", sdk, "-disableAutomaticPackageResolution"])
        .output()
        .map_err(|failure| format!("xcodebuild could not run to read {}: {failure}", project.display()))?;
    let asked = format!("{} builds {target} in {configuration} for {sdk}", project.display());
    if !said.status.success() {
        return Err(format!("xcodebuild cannot say what {asked} with: {}", String::from_utf8_lossy(&said.stderr).trim()));
    }
    let answer: Value = serde_json::from_slice(&said.stdout).map_err(|failure| format!("xcodebuild's settings of what {asked} with are no JSON: {failure}"))?;
    let settings = answer
        .as_array()
        .and_then(|targets| targets.iter().find(|entry| entry.get("target").and_then(Value::as_str) == Some(target)))
        .and_then(|entry| entry.get("buildSettings"))
        .ok_or_else(|| format!("xcodebuild names no settings of target {target} for what {asked} with"))?;
    // Asked for a configuration the project lacks, xcodebuild warns and answers its default one instead, so
    // the answer is held to what was asked.
    for (setting, wanted) in [("CONFIGURATION", configuration), ("PLATFORM_NAME", sdk)] {
        let answered = settings.get(setting).and_then(Value::as_str).unwrap_or_default();
        if answered != wanted {
            return Err(format!("xcodebuild answered {setting} {answered:?} for what {asked} with, so the project builds no such configuration"));
        }
    }
    Ok(settings.get("SWIFT_ACTIVE_COMPILATION_CONDITIONS").and_then(Value::as_str).unwrap_or_default().split_whitespace().map(str::to_string).collect())
}

/// Ask the Swift helper about `files` on `ships`, in the `xcrun` toolchain's script mode, against the
/// swift-syntax that toolchain carries for code built against it, in its `usr/lib/swift/host`: one run
/// answers a whole project, since compiling the helper is the slow part. A toolchain without that copy,
/// or a helper it cannot compile, fails in the compiler's own words, every line of them, naming the Xcode
/// the gate pins, since only its toolchain carries that swift-syntax.
fn swift_guide_answer(script: &Path, ships: &[String], files: &[PathBuf]) -> Result<String, String> {
    let located = Command::new("xcrun").args(["-f", "swiftc"]).output().map_err(|failure| format!("`xcrun -f swiftc` could not run: {failure}"))?;
    if !located.status.success() {
        return Err("`xcrun -f swiftc` named no Swift compiler".to_string());
    }
    let swiftc = PathBuf::from(String::from_utf8_lossy(&located.stdout).trim());
    let host = swiftc
        .parent()
        .and_then(Path::parent)
        .map(|toolchain| toolchain.join("lib").join("swift").join("host"))
        .ok_or_else(|| format!("{} sits where no toolchain library folder can be read", swiftc.display()))?;
    let mut command = Command::new("xcrun");
    command.args(["swift", "-I"]).arg(&host).arg("-L").arg(&host).args(["-Xlinker", "-rpath", "-Xlinker"]).arg(&host).arg(script);
    for ship in ships {
        command.arg("--ship").arg(ship);
    }
    let answer = command.args(files).output().map_err(|failure| format!("{} could not run: {failure}", script.display()))?;
    if !answer.status.success() {
        let words = [String::from_utf8_lossy(&answer.stdout), String::from_utf8_lossy(&answer.stderr)].concat();
        // A toolchain without these modules is almost always an Xcode other than the pinned one, so the
        // failure names the Xcode the gate expects: `tools/env.sh` exports its pin, and a run outside it
        // is sent to where it is pinned.
        let expected = match std::env::var_os("CRAFTBRIDGE_XCODE").filter(|pin| !pin.is_empty()) {
            Some(pin) => format!("Xcode {}, pinned by `tools/env.sh` and checked by `tools/doctor.sh`", pin.to_string_lossy()),
            None => "the Xcode `tools/env.sh` pins and `tools/doctor.sh` checks".to_string(),
        };
        return Err(format!(
            "{} failed under the toolchain's swift-syntax at {}, which must be the swift-syntax of {expected}: {}",
            script.display(),
            host.display(),
            words.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&answer.stdout).into_owned())
}

/// The Swift files the iOS samples' projects compile, each with the lines those builds compile to
/// nothing. A project's files are the Swift its one target's Sources phase lists; its builds are the ones
/// `swift_build` makes of it, every configuration the gate builds ([`crate::pins::SWIFT_CONFIGURATIONS`])
/// on every slice the toolchain pins, each with the conditions Xcode resolves for it; and
/// `tools/guide-swift-lines.swift` weighs each file with the compiler's own parser on all of them:
/// comments and string bodies are dead, and so is an `#if` branch no build compiles, while a branch only
/// the Debug configuration or only a simulator slice sees stays live, because a shipping build does see it
/// (AGENTS §10).
pub(crate) fn swift_guide_sources(root: &Path) -> Result<Vec<crate::guide_samples::Source>, String> {
    let script = root.join("tools").join("guide-swift-lines.swift");
    let mut sources = Vec::new();
    for project in ios_projects(&root.join("ios"))? {
        let build = project_build(&project)?;
        if build.sources.is_empty() {
            continue;
        }
        // The slices are the toolchain's, asked for only once there is a project to answer about: a tree
        // with no samples is answered empty, not by a toolchain no such tree pins.
        let slices = crate::pins::apple_slices(root)?;
        let mut ships = Vec::new();
        for configuration in crate::pins::SWIFT_CONFIGURATIONS {
            let mut read: std::collections::BTreeMap<&str, Vec<String>> = std::collections::BTreeMap::new();
            for slice in &slices {
                if !read.contains_key(slice.sdk) {
                    read.insert(slice.sdk, swift_conditions(&project, &build.target, configuration, slice.sdk)?);
                }
                let conditions = read.get(slice.sdk).map(|conditions| conditions.join(",")).unwrap_or_default();
                ships.push(format!("{configuration}:{}:{}:{}:{conditions}", slice.os, slice.architecture, slice.environment.trim_start_matches('-')));
            }
        }
        let answer = swift_guide_answer(&script, &ships, &build.sources)?;
        let (answered, mut dead) = guide_answers(&answer)?;
        for file in build.sources {
            if !answered.contains(&file) {
                return Err(format!("{}: the Swift helper answered nothing for it", file.display()));
            }
            let lines = dead.remove(&file).unwrap_or_default();
            sources.push(guide_source(&file, lines)?);
        }
    }
    Ok(sources)
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
        let sources: Vec<String> = project_build(&scratch.0.join("ios/samples/App/App.xcodeproj"))
            .unwrap()
            .sources
            .iter()
            .map(|file| crate::report::shown(&scratch.0, file))
            .collect();
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
            project_build(file.parent().unwrap()).unwrap_err().replace(&file.display().to_string(), "<project>")
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

    /// The guide answers read the protocol and nothing else: `S` files in order, `I` their dead lines, once
    /// each however often they are answered, and an `E` refused as the problem it names. Every other line
    /// is refused too — a writer's noise, an `I` whose number cannot be read, one for a file no `S` named —
    /// since each writer answers into a file or a stream of its own, and a line the check cannot read would
    /// otherwise pass as a live one.
    #[test]
    fn the_guide_answers_are_read_line_by_line_and_nothing_else_is() {
        let (compiled, dead) = guide_answers("S\t/a/A.swift\nI\t/a/A.swift\t3\nI\t/a/A.swift\t4\nI\t/a/A.swift\t4\nS\t/a/B.swift\n").unwrap();
        assert_eq!(compiled, [PathBuf::from("/a/A.swift"), PathBuf::from("/a/B.swift")]);
        assert_eq!(dead.get(&PathBuf::from("/a/A.swift")).map(|lines| lines.len()), Some(2));
        assert_eq!(dead.get(&PathBuf::from("/a/B.swift")), None);
        assert_eq!(guide_answers("E\t/a/A.swift\tis not valid UTF-8"), Err("/a/A.swift: is not valid UTF-8".to_string()));
        let refused = |line: &str, why: &str| assert_eq!(guide_answers(&format!("S\t/a/A.swift\n{line}\n")), Err(format!("{line:?}: {why}")));
        let unknown = "no answer the guide protocol knows: `S`, `I` or `E`, each with its own fields between tabs";
        refused("I\t/a/A.swift\tno-number", "an inert-line answer names no line number it can read");
        refused("I\t/a/A.swift\t0", "an inert-line answer names no line number it can read");
        refused("I\t/a/B.swift\t2", "an inert-line answer names a file no answer before it said was compiled");
        refused("> Task :app:guideKotlin", unknown);
        refused("S\t/a/A.swift\textra", unknown);
        refused("S\t", unknown);
        refused("", unknown);
    }

    /// What a sample build's guide run answers into the folder it is named becomes the check's source — the
    /// file it compiled with the lines it called inert, whatever the build prints beside it; a build that
    /// fails is refused naming the build and what it said; a build that answers into no file is refused;
    /// and a wrapper that cannot start is refused too, rather than read as a build with no Kotlin.
    #[test]
    #[cfg(unix)]
    fn the_gradle_guide_run_is_read_back_or_refused_by_name() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new("guide-kotlin-answer");
        scratch.write("android/samples/one/settings.gradle.kts", "");
        scratch.write("android/sdk/app/src/main/kotlin/A.kt", "one\ntwo\nthree\nfour\nfive\n");
        let wrapper = scratch.0.join("android/sdk/gradlew");
        let build = scratch.0.join("android/samples/one");
        let answer = |script: &str| {
            scratch.write("android/sdk/gradlew", script);
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        answer(
            "#!/bin/sh\nhere=$(dirname \"$0\")\necho '> Task :app:guideKotlin'\nprintf 'S\\t%s/app/src/main/kotlin/A.kt\\nI\\t%s/app/src/main/kotlin/A.kt\\t4\\nI\\t%s/app/src/main/kotlin/A.kt\\t5\\n' \"$here\" \"$here\" \"$here\" > \"$CRAFTBRIDGE_GUIDE_ANSWERS/app.guideKotlin.tsv\"\n",
        );
        let sources = android_guide_sources(&scratch.0).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].path, wrapper.parent().unwrap().join("app/src/main/kotlin/A.kt"));
        assert_eq!(sources[0].lines.len(), 5);
        assert_eq!(sources[0].dead, [4, 5].into_iter().collect());
        answer("#!/bin/sh\necho 'guideKotlin found nothing answerable' >&2\nexit 1\n");
        assert_eq!(
            android_guide_sources(&scratch.0).err().unwrap(),
            format!("{}: the build at {} answered no guide Kotlin: guideKotlin found nothing answerable", wrapper.display(), build.display())
        );
        answer("#!/bin/sh\necho 'S\tprinted, not answered'\n");
        assert!(
            android_guide_sources(&scratch.0).err().unwrap().starts_with(&format!(
                "{}: the build at {} ran guideKotlin and wrote no answer into ",
                wrapper.display(),
                build.display()
            )),
            "a build's console is no answer"
        );
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(android_guide_sources(&scratch.0).err().unwrap().starts_with(&format!(
            "{} could not run to ask what {} compiles: ",
            wrapper.display(),
            build.display()
        )));
    }

    /// The conditions a sample's Swift compiles under are Xcode's own answer for each configuration the gate
    /// builds, on each SDK: Debug's `DEBUG` from the project's settings, none for Release, and a configuration
    /// the project lacks refused, rather than read as the default one xcodebuild falls back on.
    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "asks xcodebuild, which only macOS has")]
    fn the_guide_reads_each_builds_conditions_as_xcode_resolves_them() {
        let project = crate::repo_root().join("ios/samples/PostcardStudio/PostcardStudio.xcodeproj");
        assert_eq!(swift_conditions(&project, "PostcardStudio", "Debug", "iphoneos").unwrap(), ["DEBUG"]);
        assert_eq!(swift_conditions(&project, "PostcardStudio", "Release", "iphonesimulator").unwrap(), Vec::<String>::new());
        assert_eq!(
            swift_conditions(&project, "PostcardStudio", "Profile", "iphoneos").unwrap_err(),
            format!(
                "xcodebuild answered CONFIGURATION \"Release\" for what {} builds PostcardStudio in Profile for iphoneos with, so the project builds no such configuration",
                project.display()
            )
        );
    }

    /// The Swift helper marks dead exactly what no shipping build compiles, read as the compiler reads it:
    /// an `#elseif` and an `#else` after a clause that always holds, a nested `#if` no build under its outer
    /// clause takes, another operating system's branch, a dead branch among a type's members, the bodies of
    /// multi-line strings, raw or not, bar the code interpolated into them, and comments. Live stay a branch
    /// only Debug or only the simulator compiles, and both sides of a condition no source can settle — a
    /// `canImport`, a language version — however they read.
    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "runs the helper on the Xcode toolchain's swift-syntax")]
    fn the_swift_helper_marks_what_no_shipping_build_compiles() {
        let scratch = Scratch::new("guide-swift-lines");
        scratch.write("Probe.swift", FIXTURE);
        let file = scratch.0.join("Probe.swift");
        let ships: Vec<String> =
            ["Debug:iOS:arm64::DEBUG", "Debug:iOS:arm64:simulator:DEBUG", "Release:iOS:arm64::", "Release:iOS:arm64:simulator:"].map(str::to_string).to_vec();
        let answer = swift_guide_answer(&crate::repo_root().join("tools/guide-swift-lines.swift"), &ships, std::slice::from_ref(&file)).unwrap();
        let (answered, dead) = guide_answers(&answer).unwrap();
        assert_eq!(answered, std::slice::from_ref(&file));
        let dead: Vec<usize> = dead.get(&file).map(|lines| lines.iter().copied().collect()).unwrap_or_default();
        assert_eq!(dead, [4, 6, 9, 10, 11, 13, 14, 15, 18, 40, 42, 44, 45, 47]);
    }

    /// One Swift file holding every kind of region the helper weighs, line numbers as the test above names them.
    const FIXTURE: &str = r##"#if true
let always = 1
#elseif DEBUG
let neverAfterTrue = 2
#else
let neverElse = 3
#endif
#if DEBUG
#if !DEBUG
let nestedNever = 4
#endif
#endif
#if os(macOS)
let macOnly = 5
#endif
struct Holder {
#if NEVER
    func hiddenMember() {}
#else
    func shownMember() {}
#endif
}
#if canImport(UIKit)
let maybeUIKit = 6
#else
let maybeNoUIKit = 7
#endif
#if swift(>=5.9)
let maybeNew = 8
#else
let maybeOld = 9
#endif
#if targetEnvironment(simulator)
let simulatorOnly = 10
#endif
#if DEBUG
let debugOnly = 11
#endif
let poem = """
    func inString() {}
    \(always)
    """
let raw = #"""
    func inRaw() {}
    """#
let one = #"one line"#
// func commented() {}
let tail = 12 // after code
"##;

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
