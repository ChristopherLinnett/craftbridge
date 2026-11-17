//! The repository's code folders, and the files each stage of `tools/ci.sh` hands to its tool:
//! `cargo xtask files <swift|shell|duplicates>`.
//!
//! The shell, Swift-format and duplicate stages read files the rules read too, and each choice of files
//! made apart from the rules' walk would be a second definition of what is source and what is build
//! output, free to drift from the first. So every stage takes its list from the one walk
//! (`scan::walk`), which skips build output only where a build tool writes it, and the code folders are
//! named once, here.

use crate::scan;
use std::path::Path;
use std::process::ExitCode;

/// The folders that hold the repository's own code and scripts. `plan/` is not one: it holds the plan of
/// record and the kit that builds it, and `rules: hygiene` refuses Rust, Swift, Kotlin and shell there.
pub(crate) const CODE_TREES: &[&str] = &["rust", "ios", "android", "conformance", "tools", "docs", ".github"];

/// The Gradle wrapper's script: Gradle's wrapper task writes it, and a fix to it would be lost the next
/// time the wrapper is renewed, so neither shellcheck nor jscpd reads it.
const GRADLE_WRAPPER: &str = "android/sdk/gradlew";

/// The shells shellcheck reads, as a `#!` line names them.
const SHELLS: &[&str] = &["sh", "bash", "dash", "ksh"];

/// Does a script's first line run one of [`SHELLS`]? `#!/bin/sh`, `#! /bin/bash -eu`,
/// `#!/usr/bin/env -S bash -eu` and `#!/usr/bin/env sh` all do: the shell is a word of the line after
/// `#!`, alone or the last part of a path.
pub(crate) fn runs_a_shell(first_line: &[u8]) -> bool {
    let Some(rest) = first_line.strip_prefix(b"#!") else { return false };
    String::from_utf8_lossy(rest).split(|c: char| c == '/' || c.is_whitespace()).any(|word| SHELLS.contains(&word))
}

/// Is this file a shell script: a `.sh` file, or one whose `#!` line runs a shell?
pub(crate) fn is_shell_script(path: &Path) -> bool {
    if path.extension().is_some_and(|extension| extension == "sh") {
        return true;
    }
    let Ok(bytes) = std::fs::read(path) else { return false };
    runs_a_shell(bytes.split(|byte| *byte == b'\n').next().unwrap_or_default())
}

/// The languages jscpd compares, by the extension rules a file of each must have (`rules: hygiene` holds
/// every file to these spellings).
const COMPARED_EXTENSIONS: &[&str] = &["rs", "swift", "kt", "kts"];

/// What one stage reads, as paths from the root, sorted. Generated code ships as written code does, so the
/// formatters and linters read it too; only the duplicate check leaves it out, since a generator repeats
/// one shape by design (AGENTS.md §5).
pub(crate) fn listed(root: &Path, kind: &str) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    for tree in CODE_TREES {
        for file in scan::walk(&root.join(tree), "")? {
            let shown = format!("{tree}/{}", file.relative);
            let extension = file.path.extension().and_then(|extension| extension.to_str()).unwrap_or_default();
            let shell = || shown != GRADLE_WRAPPER && is_shell_script(&file.path);
            let wanted = match kind {
                "swift" => *tree == "ios" && extension == "swift",
                "shell" => shell(),
                "duplicates" => !crate::generated::in_generated(&file.components) && (COMPARED_EXTENSIONS.contains(&extension) || shell()),
                _ => return Err(format!("no file list `{kind}` (swift, shell, duplicates)")),
            };
            if wanted {
                files.push(shown);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// `cargo xtask files <kind>`: one path per line, or the reason there is no list.
pub(crate) fn run(root: &Path, kind: &str) -> ExitCode {
    let Ok(files) = listed(root, kind).inspect_err(|error| eprintln!("files: {error}")) else { return ExitCode::from(2) };
    files.iter().for_each(|file| println!("{file}"));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn a_shell_is_read_from_the_first_line_however_it_is_started() {
        for line in ["#!/bin/sh", "#! /bin/bash -eu", "#!/usr/bin/env -S bash -eu", "#!/usr/bin/env sh", "#!/bin/dash", "#!/bin/ksh"] {
            assert!(runs_a_shell(line.as_bytes()), "{line}");
        }
        for line in ["#!/usr/bin/env python3", "#!/bin/zsh", "# sh", "#!/usr/bin/env bashful"] {
            assert!(!runs_a_shell(line.as_bytes()), "{line}");
        }
    }

    #[test]
    fn every_stage_reads_the_rules_walk_and_no_build_output_and_only_the_duplicate_check_skips_generated_code() {
        let scratch = Scratch::new("files");
        scratch.write("ios/sdk/Package.swift", "// swift-tools-version: 6.0\n");
        scratch.write("ios/sdk/Sources/CraftBridge/CraftBridge.swift", "\n");
        scratch.write("ios/sdk/Sources/CraftBridge/build/Inside.swift", "\n");
        scratch.write("ios/sdk/.build/checkouts/x/Out.swift", "\n");
        scratch.write("ios/sdk/Sources/CraftBridgeEffects/Generated/LayerCommands.swift", "\n");
        scratch.write("android/sdk/settings.gradle.kts", "\n");
        scratch.write("android/sdk/build/tmp/Out.kt", "\n");
        scratch.write("android/sdk/core/src/main/kotlin/x/build/Inside.kt", "\n");
        scratch.write("android/sdk/effects/src/generated/kotlin/x/Layer.kt", "\n");
        scratch.write("android/sdk/gradlew", "#!/bin/sh\n");
        scratch.write("tools/ci.sh", "\n");
        scratch.write("tools/generated/run.sh", "\n");
        scratch.write("tools/launch", "#!/usr/bin/env bash\n");
        scratch.write("conformance/run", "#!/usr/bin/env sh\n");
        scratch.write("rust/Cargo.toml", "\n");
        scratch.write("rust/target/debug/build/x/out/gen.rs", "\n");
        scratch.write("rust/core/x/src/lib.rs", "\n");
        scratch.write("rust/core/x/src/generated/version.rs", "\n");
        assert_eq!(
            listed(&scratch.0, "swift").unwrap(),
            [
                "ios/sdk/Package.swift",
                "ios/sdk/Sources/CraftBridge/CraftBridge.swift",
                "ios/sdk/Sources/CraftBridge/build/Inside.swift",
                "ios/sdk/Sources/CraftBridgeEffects/Generated/LayerCommands.swift",
            ]
        );
        assert_eq!(listed(&scratch.0, "shell").unwrap(), ["conformance/run", "tools/ci.sh", "tools/generated/run.sh", "tools/launch"]);
        assert_eq!(
            listed(&scratch.0, "duplicates").unwrap(),
            [
                "android/sdk/core/src/main/kotlin/x/build/Inside.kt",
                "android/sdk/settings.gradle.kts",
                "conformance/run",
                "ios/sdk/Package.swift",
                "ios/sdk/Sources/CraftBridge/CraftBridge.swift",
                "ios/sdk/Sources/CraftBridge/build/Inside.swift",
                "rust/core/x/src/lib.rs",
                "tools/ci.sh",
                "tools/launch",
            ]
        );
        assert!(listed(&scratch.0, "nope").is_err());
    }
}
