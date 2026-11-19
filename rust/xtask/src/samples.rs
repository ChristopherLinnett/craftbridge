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
