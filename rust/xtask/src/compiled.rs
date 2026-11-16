//! Rule 3's "nor may code the rules never read", from the compiler: every file a workspace crate's
//! compilation reads, or the gate's lint library's, is one of that crate's own `.rs` files that the rules
//! read.
//!
//! The compiler is the one reader that knows which files a crate is made of. `#[path]` and `include!` —
//! under any name `use` gives it, split over lines, or assembled by a macro — and a target `path` into a
//! folder the walk skips or out of `rust/` all show in the dep-info rustc writes beside each compilation.
//! So every cargo stage of `tools/ci.sh` that compiles the workspace, or the lint library, pipes cargo's
//! JSON messages through `cargo xtask compiled`, which prints the diagnostics as cargo would and then reads
//! the dep-info of each such unit cargo built: a file there that is not a `.rs` file inside the crate's own folder, found
//! by the walk every rule uses (or the lint configuration clippy records beside them), fails the stage,
//! naming the file and the crate. That reaches code behind
//! a feature, a target or a profile exactly where the stage compiles it, and refuses `include_str!` and
//! `include_bytes!` of any other file too: nothing here needs data the rules cannot see.

use crate::scan;
use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// One unit cargo reports having built: its package, its target, and the files it wrote.
pub(crate) struct Artifact {
    pub(crate) package_id: String,
    pub(crate) target_name: String,
    /// The target's kinds, as cargo names them (`lib`, `bin`, `test`, …).
    pub(crate) target_kinds: Vec<String>,
    pub(crate) filenames: Vec<String>,
    /// The program the unit built, when it built one.
    pub(crate) executable: Option<String>,
}

impl Artifact {
    /// The unit a `compiler-artifact` message reports.
    fn of(message: &serde_json::Value) -> Self {
        let strings = |value: Option<&serde_json::Value>| -> Vec<String> {
            value
                .and_then(serde_json::Value::as_array)
                .map(|items| items.iter().filter_map(serde_json::Value::as_str).map(str::to_string).collect())
                .unwrap_or_default()
        };
        let target = message.get("target");
        Self {
            package_id: message.get("package_id").and_then(serde_json::Value::as_str).unwrap_or_default().to_string(),
            target_name: target.and_then(|target| target.get("name")).and_then(serde_json::Value::as_str).unwrap_or_default().to_string(),
            target_kinds: strings(target.and_then(|target| target.get("kind"))),
            filenames: strings(message.get("filenames")),
            executable: message.get("executable").and_then(serde_json::Value::as_str).map(str::to_string),
        }
    }
}

/// Read cargo's messages: print what a person reads (each diagnostic as cargo renders it, and every line
/// that is not a message, such as a test's output), and keep each built unit.
pub(crate) fn read_messages(input: impl BufRead) -> Result<Vec<Artifact>, String> {
    let mut artifacts = Vec::new();
    for line in input.lines() {
        let line = line.map_err(|error| format!("cannot read cargo's messages: {error}"))?;
        let message = serde_json::from_str::<serde_json::Value>(&line).ok().filter(|value| value.get("reason").is_some());
        let Some(message) = message else {
            println!("{line}");
            continue;
        };
        match message.get("reason").and_then(serde_json::Value::as_str) {
            Some("compiler-message") => {
                if let Some(rendered) = message.get("message").and_then(|inner| inner.get("rendered")).and_then(serde_json::Value::as_str) {
                    print!("{rendered}");
                }
            }
            Some("compiler-artifact") => artifacts.push(Artifact::of(&message)),
            _ => {}
        }
    }
    Ok(artifacts)
}

/// The dep-info file rustc wrote for a unit, found from one of the files it built: `libname-HASH.rmeta`
/// and `name-HASH` have `name-HASH.d` beside them, an uplifted `name` has `name.d`.
fn dep_info_for(filename: &Path) -> Option<PathBuf> {
    let dir = filename.parent()?;
    let name = filename.file_name()?.to_str()?;
    let stem =
        ["rmeta", "rlib", "a", "so", "dylib", "dll", "lib", "exe"].iter().find_map(|extension| name.strip_suffix(&format!(".{extension}"))).unwrap_or(name);
    let mut candidates = vec![dir.join(format!("{stem}.d"))];
    if let Some(bare) = stem.strip_prefix("lib") {
        candidates.push(dir.join(format!("{bare}.d")));
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

/// Every file a dep-info file names as read: the dependencies of each rule, and the phony rule rustc writes
/// for each of them. Makefile syntax, with a space in a path escaped by a backslash; comment lines carry
/// environment and checksum notes, not files.
pub(crate) fn dep_info_files(text: &str) -> BTreeSet<String> {
    let mut files = BTreeSet::new();
    for line in text.lines().filter(|line| !line.is_empty() && !line.starts_with('#')) {
        let (target, dependencies) = match line.find(": ") {
            Some(at) => (line.get(..at).unwrap_or_default(), line.get(at + 2..).unwrap_or_default()),
            None => (line.strip_suffix(':').unwrap_or(line), ""),
        };
        if dependencies.trim().is_empty() {
            files.insert(target.replace("\\ ", " "));
            continue;
        }
        let mut current = String::new();
        let mut escaped = false;
        for character in dependencies.chars() {
            match character {
                '\\' if !escaped => escaped = true,
                ' ' if !escaped => {
                    if !current.is_empty() {
                        files.insert(std::mem::take(&mut current));
                    }
                }
                other => {
                    if escaped && other != ' ' {
                        current.push('\\');
                    }
                    escaped = false;
                    current.push(other);
                }
            }
        }
        if !current.is_empty() {
            files.insert(current);
        }
    }
    files
}

/// Judge the files one crate's compilation read against the crate's own `.rs` files the rules read.
/// Relative paths are rustc's, from the root of the workspace the crate builds in (`workspace_dir`):
/// `rust/` for the workspace's crates, the lint library's own folder for it. Clippy also records the
/// configuration it lints by, so a change to it lints again: the workspace manifest, which holds the lint
/// table, the crate's own manifest and `rust/clippy.toml`. Those hold no code, and `rules: lints` reads
/// each. The lint library's run records the library it loads, as built by `cargo dylint` from the gate's
/// own sources, which the rules read where they sit.
pub(crate) fn judge(root: &Path, workspace_dir: &Path, crate_name: &str, crate_dir: &Path, read: &BTreeSet<String>) -> Result<Vec<String>, String> {
    let lint_configuration = [workspace_dir.join("Cargo.toml"), crate_dir.join("Cargo.toml"), root.join(crate::machinery::CLIPPY_CONFIGURATION)];
    let own: BTreeSet<PathBuf> =
        scan::walk(crate_dir, ".rs")?.into_iter().map(|file| file.path).chain(lint_configuration).filter_map(|path| std::fs::canonicalize(path).ok()).collect();
    let lint_library_builds = std::fs::canonicalize(root.join("rust").join("target")).ok().map(|target| target.join("dylint").join("libraries"));
    let mut errors = Vec::new();
    for file in read {
        let path = workspace_dir.join(file);
        let resolved = std::fs::canonicalize(&path);
        let shown = crate::report::shown(root, resolved.as_deref().unwrap_or(&path));
        match resolved {
            Ok(canonical) if own.contains(&canonical) => {}
            Ok(canonical) if lint_library_builds.as_deref().is_some_and(|builds| is_lint_library_build(&canonical, builds)) => {}
            Ok(_) => errors.push(format!(
                "{shown}: {crate_name}'s compilation reads it, but it is not one of the crate's own .rs files the rules read (AGENTS.md §3): the code it holds is checked by no rule"
            )),
            Err(error) => errors.push(format!("{shown}: {crate_name}'s compilation read it, and it cannot be found now: {error}")),
        }
    }
    Ok(errors)
}

/// Is `path` the gate's lint library as `cargo dylint` builds it: under the workspace target folder's
/// `dylint/libraries`, named `lib<name>@<toolchain>.<extension>`, the form dylint loads a library under?
fn is_lint_library_build(path: &Path, builds: &Path) -> bool {
    let prefix = format!("lib{}@", crate::machinery::lint_library_name());
    path.starts_with(builds) && path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(&prefix))
}

/// The errors of every unit among `artifacts` of a crate the rules read: the workspace's members, and
/// the gate's lint library, which is a workspace of its own the gate compiles too. Each is judged from the
/// root of the workspace it builds in.
fn errors(root: &Path, rust_dir: &Path, artifacts: &[Artifact]) -> Result<Vec<String>, String> {
    let library_dir = root.join(crate::machinery::LINT_LIBRARY);
    let mut members: BTreeMap<String, (crate::meta::Crate, PathBuf)> = BTreeMap::new();
    for (workspace_dir, read) in [(rust_dir.to_path_buf(), true), (library_dir.clone(), library_dir.join("Cargo.toml").is_file())] {
        if !read {
            continue;
        }
        for package in crate::meta::read(&workspace_dir, crate::meta::Graph::Workspace)? {
            members.insert(package.id.clone(), (package, workspace_dir.clone()));
        }
    }
    let mut dep_infos: BTreeMap<PathBuf, &(crate::meta::Crate, PathBuf)> = BTreeMap::new();
    let mut errors = Vec::new();
    for artifact in artifacts {
        let Some(package) = members.get(&artifact.package_id) else { continue };
        match artifact.filenames.iter().find_map(|filename| dep_info_for(Path::new(filename))) {
            Some(dep_info) => {
                dep_infos.insert(dep_info, package);
            }
            None => errors.push(format!("{}: no dep-info beside {:?}, so what its compilation read is unknown", package.0.name, artifact.filenames)),
        }
    }
    let mut seen = BTreeSet::new();
    for (dep_info, (package, workspace_dir)) in dep_infos {
        let text = std::fs::read_to_string(&dep_info).map_err(|error| format!("{}: {error}", dep_info.display()))?;
        let crate_dir = Path::new(&package.manifest_path).parent().unwrap_or(workspace_dir);
        for error in judge(root, workspace_dir, &package.name, crate_dir, &dep_info_files(&text))? {
            if seen.insert(error.clone()) {
                errors.push(error);
            }
        }
    }
    Ok(errors)
}

/// Read cargo's messages from standard input, print them, and judge every workspace unit built.
pub(crate) fn run(root: &Path) -> ExitCode {
    let rust_dir = root.join("rust");
    let outcome = read_messages(std::io::stdin().lock()).and_then(|artifacts| errors(root, &rust_dir, &artifacts));
    match outcome {
        Ok(errors) if errors.is_empty() => ExitCode::SUCCESS,
        Ok(errors) => {
            for error in errors {
                println!("compiled: {error}");
            }
            ExitCode::from(1)
        }
        Err(error) => {
            println!("compiled: could not run this check: {error}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn a_dep_info_file_names_every_file_read_and_no_output() {
        let text = "/t/debug/deps/x-1.d: core/x/src/lib.rs core/x/src/my\\ file.txt\n\n/t/debug/deps/libx-1.rmeta: core/x/src/lib.rs core/x/src/my\\ file.txt\n\ncore/x/src/lib.rs:\ncore/x/src/my\\ file.txt:\n\n# env-dep:CARGO_PKG_NAME=x\n";
        assert_eq!(dep_info_files(text).into_iter().collect::<Vec<_>>(), ["core/x/src/lib.rs", "core/x/src/my file.txt"]);
    }

    #[test]
    fn the_dep_info_is_found_beside_whatever_the_unit_built() {
        let scratch = Scratch::new("compiled-dep-info");
        for name in ["deps/x-1.d", "deps/libx-1.rmeta", "deps/y-2", "deps/y-2.d", "tool", "tool.d"] {
            scratch.write(name, "");
        }
        assert_eq!(dep_info_for(&scratch.0.join("deps/libx-1.rmeta")), Some(scratch.0.join("deps/x-1.d")));
        assert_eq!(dep_info_for(&scratch.0.join("deps/y-2")), Some(scratch.0.join("deps/y-2.d")));
        assert_eq!(dep_info_for(&scratch.0.join("tool")), Some(scratch.0.join("tool.d")));
        assert_eq!(dep_info_for(&scratch.0.join("deps/libz-3.rlib")), None);
    }

    #[test]
    fn a_file_outside_the_crate_or_not_rust_or_where_the_walk_does_not_look_is_refused() {
        let scratch = Scratch::new("compiled-judge");
        scratch.write("rust/Cargo.toml", "");
        scratch.write("rust/core/x/Cargo.toml", "");
        scratch.write("rust/core/x/src/lib.rs", "");
        scratch.write("rust/core/x/src/inner.rs", "");
        scratch.write("rust/core/x/src/hidden.txt", "");
        scratch.write("rust/core/x/target/lib.rs", "");
        scratch.write("rust/core/shared.rs", "");
        scratch.write("rust/clippy.toml", "");
        scratch.write("rust/core/x/Build.toml", "");
        let read: BTreeSet<String> = [
            "core/x/src/lib.rs",
            "core/x/src/inner.rs",
            "core/x/src/hidden.txt",
            "core/x/target/lib.rs",
            "core/x/src/../../shared.rs",
            "Cargo.toml",
            "core/x/Cargo.toml",
            "clippy.toml",
            "core/x/Build.toml",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let errors = judge(&scratch.0, &scratch.0.join("rust"), "x", &scratch.0.join("rust/core/x"), &read).unwrap();
        assert_eq!(errors.len(), 4, "the lint configuration clippy records is no code: {errors:?}");
        for refused in [
            "rust/core/x/src/hidden.txt: x's compilation reads it",
            "rust/core/x/target/lib.rs: x's",
            "rust/core/shared.rs: x's",
            "rust/core/x/Build.toml: x's",
        ] {
            assert!(errors.iter().any(|e| e.starts_with(refused)), "{refused}: {errors:?}");
        }
    }

    /// The lint library's run records the library dylint loads; that build of the gate's own sources is
    /// the one file outside a crate it may read, and only where `cargo dylint` builds it, under its name.
    #[test]
    fn the_lint_library_dylint_loads_is_no_crate_code_but_a_look_alike_is() {
        let scratch = Scratch::new("compiled-lint-library");
        scratch.write("rust/Cargo.toml", "");
        scratch.write("rust/core/x/Cargo.toml", "");
        scratch.write("rust/core/x/src/lib.rs", "");
        let builds = "target/dylint/libraries/nightly-2026-08-20-aarch64-apple-darwin/release";
        let library = format!("{builds}/libnever_crash_lints@nightly-2026-08-20-aarch64-apple-darwin.dylib");
        let other = format!("{builds}/libsomething_else@nightly-2026-08-20-aarch64-apple-darwin.dylib");
        let elsewhere = "target/debug/libnever_crash_lints@nightly-2026-08-20-aarch64-apple-darwin.dylib".to_string();
        for file in [&library, &other, &elsewhere] {
            scratch.write(&format!("rust/{file}"), "");
        }
        let read: BTreeSet<String> = ["core/x/src/lib.rs".to_string(), library, other, elsewhere].into_iter().collect();
        let errors = judge(&scratch.0, &scratch.0.join("rust"), "x", &scratch.0.join("rust/core/x"), &read).unwrap();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("libsomething_else@")), "another library: {errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("rust/target/debug/libnever_crash_lints@")), "outside dylint's own folder: {errors:?}");
    }

    /// The lint library is a workspace of its own the gate compiles too, so a unit of it is judged like a
    /// member's, from its own folder, where rustc runs for it: its own source passes, and any other file
    /// its compilation reads is refused.
    #[test]
    fn the_lint_library_s_own_compilation_is_judged_from_its_own_folder() {
        let root = crate::repo_root();
        let library = crate::meta::read(&root.join(crate::machinery::LINT_LIBRARY), crate::meta::Graph::Workspace).unwrap();
        let scratch = Scratch::new("compiled-library");
        scratch.write("deps/never_crash_lints-1.d", "deps/libnever_crash_lints-1.rmeta: src/lib.rs Cargo.toml ../../README.md\n");
        scratch.write("deps/libnever_crash_lints-1.rmeta", "");
        let artifact = Artifact {
            package_id: library[0].id.clone(),
            target_name: "never-crash-lints".to_string(),
            target_kinds: vec!["lib".to_string()],
            filenames: vec![scratch.0.join("deps/libnever_crash_lints-1.rmeta").to_string_lossy().into_owned()],
            executable: None,
        };
        let found = errors(&root, &root.join("rust"), &[artifact]).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("README.md: never-crash-lints's compilation reads it"), "{found:?}");
    }

    #[test]
    fn cargo_s_messages_are_printed_and_its_units_kept() {
        let input = concat!(
            "{\"reason\":\"compiler-artifact\",\"package_id\":\"path+file:///r/core/x#0.1.0\",\"target\":{\"name\":\"x\",\"kind\":[\"lib\"]},",
            "\"filenames\":[\"/t/deps/libx-1.rmeta\"],\"executable\":null}\n",
            "{\"reason\":\"compiler-artifact\",\"package_id\":\"path+file:///r/core/y#0.1.0\",\"target\":{\"name\":\"y\",\"kind\":[\"bin\"]},",
            "\"filenames\":[\"/t/debug/y\"],\"executable\":\"/t/debug/y\"}\n",
            "{\"reason\":\"compiler-message\",\"message\":{\"rendered\":\"warning: x\\n\"}}\n",
            "test a ... ok\n",
            "{\"reason\":\"build-finished\",\"success\":true}\n"
        );
        let artifacts = read_messages(input.as_bytes()).unwrap();
        let [lib, bin] = artifacts.as_slice() else { panic!("two units: {}", artifacts.len()) };
        assert_eq!(lib.package_id, "path+file:///r/core/x#0.1.0");
        assert_eq!((lib.target_name.as_str(), lib.target_kinds.as_slice()), ("x", ["lib".to_string()].as_slice()));
        assert_eq!(lib.filenames, ["/t/deps/libx-1.rmeta"]);
        assert_eq!(lib.executable, None);
        assert_eq!((bin.target_kinds.as_slice(), bin.executable.as_deref()), (["bin".to_string()].as_slice(), Some("/t/debug/y")));
    }

    /// The whole path on a copy of the real tree, with the real compiler: `include!` under the name `use`
    /// gives it pastes a file in, which only the compiler's dep-info names.
    #[test]
    fn the_compiler_s_record_of_a_renamed_include_is_refused() {
        let copy = crate::scratch::rust_tree_copy("compiled-real");
        let core = copy.0.join("rust/core/craftbridge-core");
        std::fs::write(core.join("src/hidden.txt"), "/// Pasted in.\npub fn pasted() -> u8 { 1 }\n").unwrap();
        let lib = std::fs::read_to_string(core.join("src/lib.rs")).unwrap();
        std::fs::write(
            core.join("src/lib.rs"),
            format!("{lib}\n/// Hidden.\npub mod hidden {{\n    use std::include as paste;\n    paste!(\"hidden.txt\");\n}}\n"),
        )
        .unwrap();
        let output = std::process::Command::new(env!("CARGO"))
            .current_dir(copy.0.join("rust"))
            .env("CARGO_TARGET_DIR", copy.0.join("target"))
            .args(["check", "--offline", "--locked", "--package", "craftbridge-core", "--message-format=json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let artifacts = read_messages(output.stdout.as_slice()).unwrap();
        let errors = errors(&copy.0, &copy.0.join("rust"), &artifacts).unwrap();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("rust/core/craftbridge-core/src/hidden.txt: craftbridge-core's compilation reads it"), "{errors:?}");
    }
}
