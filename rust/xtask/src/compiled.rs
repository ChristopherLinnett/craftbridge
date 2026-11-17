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
//! naming the file and the crate. A unit's dep-info is the one rustc wrote beside the file it built, under
//! the crate's name and the hash cargo gives the unit. A file a message names under its target's bare name
//! is cargo's copy of that file, an uplift, and the `.d` beside a copy is cargo's own, which lists every
//! local crate the unit links and is written only for what a command names, so a warm folder keeps an
//! earlier build's: a copy is traced by file identity to the file rustc wrote. That reaches code behind
//! a feature, a target or a profile exactly where the stage compiles it, and refuses `include_str!` and
//! `include_bytes!` of any other file too: nothing here needs data the rules cannot see.
//!
//! The same messages say which profile cargo built each unit in, as cargo resolved it, so this also holds
//! CraftBridge's own crates to overflow checks and debug assertions in every build of tests the gate makes
//! (AGENTS.md §7): a profile that switches them off is found however it is spelt — in `[profile.test]` or a
//! profile it `inherits`, in a package override under any form of package spec, for any member — since only
//! cargo's answer, not a reading of the manifest, follows every rule by which cargo picks a unit's settings.
//! Such a build is held in every unit of ours it compiles, not only in its tests: a test runs the libraries
//! of ours it links, a crate with no test of its own included, and the code our build scripts and
//! proc-macros make for them, which run on the host as the build goes.

use crate::scan;
use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The settings of a unit's profile, as cargo's report names them, that make an integer overflow and a
/// failed `debug_assert!` panic, each with the words a finding calls it by.
const PANICKING_CHECKS: [(&str, &str); 2] = [("overflow_checks", "overflow checks"), ("debug_assertions", "debug assertions")];

/// What cargo reports of the profile it built a unit in: whether the unit is a test, and which of the
/// [`PANICKING_CHECKS`] it was built without, by their words.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct UnitProfile {
    pub(crate) test: bool,
    pub(crate) without: Vec<&'static str>,
}

impl UnitProfile {
    /// The `profile` of a `compiler-artifact` message. A field the report leaves out counts against the
    /// unit, as a test and as a check that is off, so a change in what cargo reports fails the check instead
    /// of passing it.
    fn of(profile: &serde_json::Value) -> Self {
        let flag = |key: &str| profile.get(key).and_then(serde_json::Value::as_bool);
        Self {
            test: flag("test") != Some(false),
            without: PANICKING_CHECKS.iter().filter(|(key, _)| flag(key) != Some(true)).map(|(_, words)| *words).collect(),
        }
    }
}

/// One unit cargo reports having built: its package, its target, the files it wrote and its profile.
pub(crate) struct Artifact {
    pub(crate) package_id: String,
    pub(crate) target_name: String,
    /// The target's kinds, as cargo names them (`lib`, `bin`, `test`, …).
    pub(crate) target_kinds: Vec<String>,
    pub(crate) filenames: Vec<String>,
    /// The program the unit built, when it built one.
    pub(crate) executable: Option<String>,
    /// The profile cargo built the unit in, when the message says.
    pub(crate) profile: Option<UnitProfile>,
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
            profile: message.get("profile").map(UnitProfile::of),
        }
    }
}

/// What cargo reported of one command: every unit it built, and the text of every diagnostic it passed on
/// from rustc, as rustc wrote it rather than as cargo renders it.
pub(crate) struct Reported {
    pub(crate) artifacts: Vec<Artifact>,
    pub(crate) diagnostics: Vec<String>,
}

/// Ask cargo, in `rust_dir`, to build `packages` for the host, and return every unit it reports: Cargo,
/// not a guess at its target folder, says where each file was written. Both the codegen's binary and the
/// binding generators' library and bindgen come from here.
pub(crate) fn build(rust_dir: &Path, packages: &[&str]) -> Result<Vec<Artifact>, String> {
    let mut arguments = vec!["--message-format=json-render-diagnostics"];
    for package in packages {
        arguments.extend(["--package", package]);
    }
    compile(rust_dir, "build", &arguments, &[]).map(|reported| reported.artifacts)
}

/// Run `cargo <command> --locked <arguments>` in `rust_dir`, with `variables` for the compilers it runs, and
/// read what it reports in the JSON messages `arguments` asks for. Besides [`build`], the native library the
/// XCFramework holds is built here, through `cargo rustc`, which hands what follows `--` to that library's
/// own compilation alone.
pub(crate) fn compile(rust_dir: &Path, command: &str, arguments: &[&str], variables: &[(&str, &str)]) -> Result<Reported, String> {
    let output = std::process::Command::new(env!("CARGO"))
        .current_dir(rust_dir)
        .args([command, "--locked"])
        .args(arguments)
        .envs(variables.iter().copied())
        .output()
        .map_err(|error| format!("cannot run `{} {command}`: {error}", env!("CARGO")))?;
    if !output.status.success() {
        return Err(format!("`cargo {command} {}` exited with {}: {}", arguments.join(" "), output.status, String::from_utf8_lossy(&output.stderr).trim()));
    }
    read_reported(output.stdout.as_slice())
}

/// The binary of the target named `name` among the units cargo reported building.
pub(crate) fn executable_of(artifacts: &[Artifact], name: &str) -> Option<PathBuf> {
    artifacts
        .iter()
        .find(|artifact| artifact.target_name == name && artifact.target_kinds.iter().any(|kind| kind == "bin"))
        .and_then(|artifact| artifact.executable.as_deref())
        .map(PathBuf::from)
}

/// Read cargo's messages as [`read_reported`] does, keeping only the units it built.
pub(crate) fn read_messages(input: impl BufRead) -> Result<Vec<Artifact>, String> {
    read_reported(input).map(|reported| reported.artifacts)
}

/// Read cargo's messages: print what a person reads (each diagnostic as cargo renders it, and every line
/// that is not a message, such as a test's output), and keep each built unit and each diagnostic's text.
pub(crate) fn read_reported(input: impl BufRead) -> Result<Reported, String> {
    let mut reported = Reported { artifacts: Vec::new(), diagnostics: Vec::new() };
    for line in input.lines() {
        let line = line.map_err(|error| format!("cannot read cargo's messages: {error}"))?;
        let message = serde_json::from_str::<serde_json::Value>(&line).ok().filter(|value| value.get("reason").is_some());
        let Some(message) = message else {
            println!("{line}");
            continue;
        };
        match message.get("reason").and_then(serde_json::Value::as_str) {
            Some("compiler-message") => {
                let diagnostic = message.get("message");
                if let Some(rendered) = diagnostic.and_then(|inner| inner.get("rendered")).and_then(serde_json::Value::as_str) {
                    print!("{rendered}");
                }
                if let Some(text) = diagnostic.and_then(|inner| inner.get("message")).and_then(serde_json::Value::as_str) {
                    reported.diagnostics.push(text.to_string());
                }
            }
            Some("compiler-artifact") => reported.artifacts.push(Artifact::of(&message)),
            _ => {}
        }
    }
    Ok(reported)
}

/// The dep-info files rustc wrote for the unit a `compiler-artifact` message reports. A file the message
/// names under rustc's own name for the unit has rustc's dep-info beside it. Any other is cargo's copy of
/// one — a binary, an example, a build script or what a command named, copied out under the target's bare
/// name — and nothing beside a copy is read: the unit's dep-info is beside the file rustc wrote that the
/// copy is. Cargo's messages never name that file, so it is looked for where rustc writes: beside the
/// copy, as a build script's sits, in the `deps` folder beside the copy, and in every `deps` folder the
/// stream's units sit in, with the `examples` folder beside it, which is where rustc writes when
/// `build.build-dir` keeps the build apart from the folder the copies go to.
fn dep_infos_of(artifact: &Artifact, rustc_folders: &BTreeSet<PathBuf>) -> Vec<PathBuf> {
    let crate_name = artifact.target_name.replace('-', "_");
    let files = || artifact.filenames.iter().map(Path::new);
    if let Some(dep_info) = files().find_map(|file| rustc_dep_info(file, &crate_name)) {
        return vec![dep_info];
    }
    files().map(|copy| copied_from(copy, &crate_name, rustc_folders)).find(|found| !found.is_empty()).unwrap_or_default()
}

/// The folders rustc wrote this stream's units into, as cargo's messages show them: each `deps` folder a
/// reported file sits in, and the `examples` folder cargo keeps beside it for the examples rustc builds.
fn rustc_folders(artifacts: &[Artifact]) -> BTreeSet<PathBuf> {
    let deps = artifacts.iter().flat_map(|artifact| &artifact.filenames).filter_map(|file| Path::new(file).parent()).filter(|dir| dir.ends_with("deps"));
    deps.flat_map(|dir| [dir.to_path_buf(), dir.with_file_name("examples")]).collect()
}

/// The dep-info rustc wrote beside `file`, when `file` carries the name rustc gives a unit of `crate_name`:
/// the crate's name (after `lib` for a library), then the `-C extra-filename` cargo passes, a `-` and the
/// unit's sixteen-digit hash, then at most one extension. rustc names the dep-info after the crate and that
/// same suffix, `<crate>-<hash>.d`, so a crate whose name begins with this one's does not reach it. A
/// local dylib or cdylib carries no hash, and only in `deps`, where cargo copies nothing to, is such a
/// bare name rustc's: anywhere else it is cargo's copy, and so it is in a profile folder named `deps`,
/// which holds the `examples` folder cargo makes in every profile folder.
fn rustc_dep_info(file: &Path, crate_name: &str) -> Option<PathBuf> {
    let dir = file.parent()?;
    let name = file.file_name()?.to_str()?;
    ["", "lib"].into_iter().find_map(|prefix| {
        let (extra, extension) = name.strip_prefix(prefix)?.strip_prefix(crate_name).map(|rest| rest.split_once('.').unwrap_or((rest, "")))?;
        let hashed = extra.strip_prefix('-').is_some_and(|hash| hash.len() == 16 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let bare_in_deps = || extra.is_empty() && dir.ends_with("deps") && !dir.join("examples").exists();
        let dep_info = dir.join(format!("{crate_name}{extra}.d"));
        ((hashed || bare_in_deps()) && !extension.contains('.') && dep_info.is_file()).then_some(dep_info)
    })
}

/// The dep-info of the file rustc wrote for `crate_name` that `copy` is a copy of. Cargo hard-links where it
/// can, so the file that is `copy` by inode is the one. On macOS it copies instead, which APFS makes a
/// clone, keeping the bytes but not the inode, so failing an inode each file with `copy`'s bytes answers;
/// and two builds of a unit whose hashes differ only in what never reaches the binary, such as a lint level
/// in `RUSTFLAGS`, can leave byte-identical files, so every one of them is judged rather than one picked
/// by its name.
fn copied_from(copy: &Path, crate_name: &str, rustc_folders: &BTreeSet<PathBuf>) -> Vec<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let Ok(copied) = copy.metadata() else { return Vec::new() };
    let mut folders = rustc_folders.clone();
    if let Some(dir) = copy.parent() {
        folders.extend([dir.to_path_buf(), dir.join("deps")]);
    }
    let candidates: Vec<(PathBuf, PathBuf, std::fs::Metadata)> = folders
        .iter()
        .filter_map(|folder| std::fs::read_dir(folder).ok())
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let dep_info = rustc_dep_info(&path, crate_name)?;
            let metadata = path.metadata().ok().filter(|metadata| metadata.len() == copied.len())?;
            Some((path, dep_info, metadata))
        })
        .collect();
    if let Some((_, dep_info, _)) = candidates.iter().find(|(_, _, metadata)| metadata.ino() == copied.ino() && metadata.dev() == copied.dev()) {
        return vec![dep_info.clone()];
    }
    let twins: BTreeSet<PathBuf> =
        candidates.into_iter().filter(|(path, _, _)| same_bytes(copy, path).unwrap_or(false)).map(|(_, dep_info, _)| dep_info).collect();
    twins.into_iter().collect()
}

/// Do two files hold the same bytes? Read a chunk of each at a time, so a binary of hundreds of megabytes
/// is never held whole; `take` fills each chunk through short reads, so they cannot make two identical files
/// look unlike.
fn same_bytes(path: &Path, other: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    const CHUNK: u64 = 1 << 20;
    let (mut first, mut second) = (std::fs::File::open(path)?, std::fs::File::open(other)?);
    let (mut first_chunk, mut second_chunk) = (Vec::new(), Vec::new());
    loop {
        first_chunk.clear();
        second_chunk.clear();
        let read = (&mut first).take(CHUNK).read_to_end(&mut first_chunk)?;
        (&mut second).take(CHUNK).read_to_end(&mut second_chunk)?;
        if first_chunk != second_chunk {
            return Ok(false);
        }
        if read == 0 {
            return Ok(true);
        }
    }
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
/// own sources, which the rules read where they sit. That build is passed only where `cargo dylint` puts
/// it, under `target_directory`, the folder cargo names for the workspace, which the pre-push hook moves
/// for each gate.
pub(crate) fn judge(
    root: &Path,
    workspace_dir: &Path,
    target_directory: &Path,
    crate_name: &str,
    crate_dir: &Path,
    read: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    let lint_configuration = [workspace_dir.join("Cargo.toml"), crate_dir.join("Cargo.toml"), root.join(crate::machinery::CLIPPY_CONFIGURATION)];
    let own: BTreeSet<PathBuf> =
        scan::walk(crate_dir, ".rs")?.into_iter().map(|file| file.path).chain(lint_configuration).filter_map(|path| std::fs::canonicalize(path).ok()).collect();
    let lint_library_builds = std::fs::canonicalize(target_directory).ok().map(|target| target.join("dylint").join("libraries"));
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

/// Is `path` the gate's lint library as `cargo dylint` builds it: under the workspace's build folder's
/// `dylint/libraries`, named `lib<name>@<toolchain>.<extension>`, the form dylint loads a library under?
fn is_lint_library_build(path: &Path, builds: &Path) -> bool {
    let prefix = format!("lib{}@", crate::machinery::lint_library_name());
    path.starts_with(builds) && path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(&prefix))
}

/// The finding for a unit of a crate of ours that cargo built without overflow checks or debug assertions
/// in a build of tests (`builds_tests`), so an overflow or a broken invariant in its code would pass
/// unnoticed, or whose profile cargo did not report. Every unit of ours such a build compiles is held: the
/// tests run a test target and each library it links, its own crate's or another's, whether or not that
/// crate has a test of its own, and a build script, a proc-macro or a library one of them links runs on the
/// host to make what they run. A build that compiles no test, such as a release build, may build without
/// them.
fn profile_error(name: &str, artifact: &Artifact, builds_tests: bool) -> Option<String> {
    let target = &artifact.target_name;
    match &artifact.profile {
        None => Some(format!("{name}: cargo reported no profile for {target}, so whether its tests keep overflow checks and debug assertions is unknown")),
        Some(profile) if builds_tests && !profile.without.is_empty() => {
            let unit = if profile.test { format!("the test {target}") } else { target.to_string() };
            Some(format!(
                "{name}: cargo built {unit} without {} in a build of tests: CraftBridge's own crates keep overflow checks and debug assertions in every unit a build of tests compiles, so an overflow or a broken debug assertion in their code fails the build or the test instead of passing unnoticed (AGENTS.md §7)",
                profile.without.join(" or ")
            ))
        }
        Some(_) => None,
    }
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
    let target_directory = crate::meta::target_directory(rust_dir)?;
    let folders = rustc_folders(artifacts);
    let mut dep_infos: BTreeMap<PathBuf, &(crate::meta::Crate, PathBuf)> = BTreeMap::new();
    let builds_tests =
        artifacts.iter().any(|artifact| members.contains_key(&artifact.package_id) && artifact.profile.as_ref().is_some_and(|profile| profile.test));
    let mut errors = Vec::new();
    for artifact in artifacts {
        let Some(package) = members.get(&artifact.package_id) else { continue };
        errors.extend(profile_error(&package.0.name, artifact, builds_tests));
        let found = dep_infos_of(artifact, &folders);
        if found.is_empty() {
            errors.push(format!("{}: no dep-info rustc wrote for {:?} is found, so what its compilation read is unknown", package.0.name, artifact.filenames));
        }
        dep_infos.extend(found.into_iter().map(|dep_info| (dep_info, package)));
    }
    let mut seen = BTreeSet::new();
    for (dep_info, (package, workspace_dir)) in dep_infos {
        let text = std::fs::read_to_string(&dep_info).map_err(|error| format!("{}: {error}", dep_info.display()))?;
        let crate_dir = Path::new(&package.manifest_path).parent().unwrap_or(workspace_dir);
        for error in judge(root, workspace_dir, &target_directory, &package.name, crate_dir, &dep_info_files(&text))? {
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

    /// An artifact shaped like a message, for target `target`, naming files under this scratch.
    fn an_artifact(scratch: &Scratch, target: &str, files: &[&str]) -> Artifact {
        Artifact {
            package_id: String::new(),
            target_name: target.to_string(),
            target_kinds: vec!["bin".to_string()],
            filenames: files.iter().map(|file| scratch.0.join(file).to_string_lossy().into_owned()).collect(),
            executable: None,
            profile: Some(UnitProfile::default()),
        }
    }

    /// The lint library's unit as a message names it, package `id` having written `file`.
    fn library_unit(id: &str, file: &Path) -> Artifact {
        Artifact {
            package_id: id.to_string(),
            target_name: "never-crash-lints".to_string(),
            target_kinds: vec!["lib".to_string()],
            filenames: vec![file.to_string_lossy().into_owned()],
            executable: None,
            profile: Some(UnitProfile::default()),
        }
    }

    /// Write each file under its own name, so no two hold the same bytes.
    fn write_each(scratch: &Scratch, names: &[&str]) {
        for name in names {
            scratch.write(name, name);
        }
    }

    /// A file rustc wrote and cargo names has rustc's dep-info beside it, under the crate's name and the
    /// unit's hash: in `deps`, bare for a local cdylib, and in the per-unit `out` folder `cargo dylint`'s
    /// nightly builds into. Another crate whose name starts the same does not reach it.
    #[test]
    fn the_dep_info_rustc_wrote_is_found_beside_the_file_it_named() {
        let scratch = Scratch::new("compiled-dep-info");
        let out = "dylint/debug/build/w/0123456789abcdef/out";
        write_each(
            &scratch,
            &[
                "debug/deps/libx-0123456789abcdef.rmeta",
                "debug/deps/x-0123456789abcdef.d",
                "debug/deps/x-0123456789abcdef.x1.rcgu.o",
                "debug/deps/libffi.dylib",
                "debug/deps/ffi.d",
                &format!("{out}/libw-0123456789abcdef.rmeta"),
                &format!("{out}/w-0123456789abcdef.d"),
                "debug/deps/z-0123456789abcdef",
                "debug/deps/xy-0123456789abcdef",
                "debug/deps/xy-0123456789abcdef.d",
                "debug/deps/v-1",
                "debug/deps/v-1.d",
            ],
        );
        let found = |target: &str, file: &str| dep_infos_of(&an_artifact(&scratch, target, &[file]), &BTreeSet::new());
        assert_eq!(found("x", "debug/deps/libx-0123456789abcdef.rmeta"), [scratch.0.join("debug/deps/x-0123456789abcdef.d")]);
        assert_eq!(found("ffi", "debug/deps/libffi.dylib"), [scratch.0.join("debug/deps/ffi.d")]);
        assert_eq!(found("w", &format!("{out}/libw-0123456789abcdef.rmeta")), [scratch.0.join(format!("{out}/w-0123456789abcdef.d"))]);
        assert_eq!(found("z", "debug/deps/z-0123456789abcdef"), Vec::<PathBuf>::new());
        assert_eq!(found("x", "debug/deps/xy-0123456789abcdef"), Vec::<PathBuf>::new());
        // A suffix that is no hash cargo writes is not rustc's name for the unit.
        assert_eq!(found("v", "debug/deps/v-1"), Vec::<PathBuf>::new());
        // A codegen unit's object file is no output rustc names for the unit.
        assert_eq!(found("x", "debug/deps/x-0123456789abcdef.x1.rcgu.o"), Vec::<PathBuf>::new());
    }

    /// What sits beside a copy is cargo's, never read. A copy is the file rustc wrote that it is by inode,
    /// where cargo hard-linked it, even when another of the crate's files holds the same bytes; failing an
    /// inode, every file with its bytes is judged, and a file of the same crate with other bytes is passed
    /// over, so identity picks and not the name.
    #[test]
    fn a_copy_is_traced_by_identity_to_the_file_rustc_wrote() {
        let cloned = Scratch::new("compiled-copy");
        cloned.write("debug/my-tool.d", "planted");
        cloned.write("debug/my-tool", "bytes");
        cloned.write("debug/deps/my_tool-0000000000000001", "other");
        cloned.write("debug/deps/my_tool-0000000000000002", "bytes");
        cloned.write("debug/deps/my_tool-0000000000000003", "bytes");
        write_each(&cloned, &["debug/deps/my_tool-0000000000000001.d", "debug/deps/my_tool-0000000000000002.d", "debug/deps/my_tool-0000000000000003.d"]);
        let twins = ["debug/deps/my_tool-0000000000000002.d", "debug/deps/my_tool-0000000000000003.d"].map(|file| cloned.0.join(file));
        assert_eq!(dep_infos_of(&an_artifact(&cloned, "my-tool", &["debug/my-tool"]), &BTreeSet::new()), twins);
        let linked = Scratch::new("compiled-link");
        linked.write("debug/tool.d", "planted");
        linked.write("debug/deps/tool-0000000000000001", "bytes");
        linked.write("debug/deps/tool-0000000000000002", "bytes");
        write_each(&linked, &["debug/deps/tool-0000000000000001.d", "debug/deps/tool-0000000000000002.d"]);
        std::fs::hard_link(linked.0.join("debug/deps/tool-0000000000000002"), linked.0.join("debug/tool")).unwrap();
        assert_eq!(dep_infos_of(&an_artifact(&linked, "tool", &["debug/tool"]), &BTreeSet::new()), [linked.0.join("debug/deps/tool-0000000000000002.d")]);
    }

    /// Where the copies go is not always beside rustc's folders. With `build.build-dir` apart, a copy's
    /// source sits in a `deps` folder the stream's other units name, and a stream that names none leaves the
    /// unit unfound. A profile folder named `deps` holds copies under bare names, as every profile folder
    /// does beside its `examples`, and those are not rustc's.
    #[test]
    fn a_copy_is_traced_wherever_cargo_writes_its_source() {
        let scratch = Scratch::new("compiled-apart");
        scratch.write("build/debug/deps/tool-0000000000000001", "bytes");
        scratch.write("out/debug/tool", "bytes");
        write_each(&scratch, &["build/debug/deps/tool-0000000000000001.d", "build/debug/deps/libdep-0000000000000005.rlib", "out/debug/tool.d"]);
        let tool = an_artifact(&scratch, "tool", &["out/debug/tool"]);
        let stream = [an_artifact(&scratch, "dep", &["build/debug/deps/libdep-0000000000000005.rlib"]), an_artifact(&scratch, "tool", &["out/debug/tool"])];
        assert_eq!(dep_infos_of(&tool, &rustc_folders(&stream)), [scratch.0.join("build/debug/deps/tool-0000000000000001.d")]);
        assert_eq!(dep_infos_of(&tool, &BTreeSet::new()), Vec::<PathBuf>::new());
        scratch.mkdir("profile/deps/examples");
        scratch.write("profile/deps/tool", "bytes");
        scratch.write("profile/deps/deps/tool-0000000000000001", "bytes");
        write_each(&scratch, &["profile/deps/tool.d", "profile/deps/deps/tool-0000000000000001.d"]);
        let named_deps = an_artifact(&scratch, "tool", &["profile/deps/tool"]);
        assert_eq!(dep_infos_of(&named_deps, &BTreeSet::new()), [scratch.0.join("profile/deps/deps/tool-0000000000000001.d")]);
    }

    /// The layouts cargo itself writes: binaries, with and without a hyphen in the name, an example and a
    /// build script, each copied out under its target's name, built into one folder, again with the build
    /// folder apart, and checked. Every unit leads only to dep-info rustc wrote for it, and never to the `.d`
    /// beside a copy, planted here to name a file outside the crate.
    #[test]
    fn every_unit_cargo_builds_leads_to_the_dep_info_rustc_wrote_for_it() {
        let scratch = Scratch::new("compiled-layouts");
        scratch.write("probe/Cargo.toml", "[package]\nname = \"probe-tool\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n");
        scratch.write("probe/build.rs", "fn main() {}\n");
        scratch.write("probe/src/lib.rs", "pub fn probe() {}\n");
        scratch.write("probe/src/main.rs", "fn main() {\n    probe_tool::probe();\n}\n");
        scratch.write("probe/src/bin/probe.rs", "fn main() {\n    probe_tool::probe();\n}\n");
        scratch.write("probe/examples/probe-example.rs", "fn main() {\n    probe_tool::probe();\n}\n");
        let outside = crate::repo_root().join("README.md");
        let runs = [("build", "one", "one"), ("build", "apart", "intermediate"), ("check", "one", "one")];
        for (command, target_dir, build_dir) in runs {
            let output = std::process::Command::new(env!("CARGO"))
                .current_dir(scratch.0.join("probe"))
                .env("CARGO_BUILD_BUILD_DIR", scratch.0.join(build_dir))
                .args([command, "--offline", "--bins", "--examples", "--message-format=json", "--target-dir"])
                .arg(scratch.0.join(target_dir))
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let artifacts = read_messages(output.stdout.as_slice()).unwrap();
            // Beside every file cargo names: a copy's `.d` is cargo's own, and every other file these runs name
            // starts with `lib` (a library, or the metadata `check` writes for any target), so the plant takes
            // `lib<crate>-<hash>.d` and never the `<crate>-<hash>.d` rustc wrote.
            for file in artifacts.iter().flat_map(|artifact| &artifact.filenames) {
                std::fs::write(Path::new(file).with_extension("d"), format!("{file}: {}\n", outside.display())).unwrap();
            }
            // Cargo copies an executable out under its target's name, which tells the copies apart without
            // asking the code under test.
            let copied = artifacts
                .iter()
                .filter(|artifact| artifact.filenames.iter().all(|file| Path::new(file).file_name().is_some_and(|name| *name == *artifact.target_name)));
            let kinds: BTreeSet<&str> = copied.flat_map(|artifact| artifact.target_kinds.iter().map(String::as_str)).collect();
            let expected: BTreeSet<&str> = if command == "build" { ["bin", "custom-build", "example"].into() } else { ["custom-build"].into() };
            assert_eq!(kinds, expected, "{command} into {target_dir}");
            let folders = rustc_folders(&artifacts);
            for artifact in &artifacts {
                let found = dep_infos_of(artifact, &folders);
                assert!(!found.is_empty(), "{command} into {target_dir}: {:?}", artifact.filenames);
                for dep_info in found {
                    let text = std::fs::read_to_string(&dep_info).unwrap();
                    assert!(!text.contains(&outside.display().to_string()), "{} was read for {:?}", dep_info.display(), artifact.filenames);
                }
            }
        }
    }

    /// A unit of a message that leads to no dep-info fails, and the error names it.
    #[test]
    fn a_unit_whose_dep_info_cannot_be_found_fails_naming_it() {
        let root = crate::repo_root();
        let library = crate::meta::read(&root.join(crate::machinery::LINT_LIBRARY), crate::meta::Graph::Workspace).unwrap();
        let scratch = Scratch::new("compiled-no-dep-info");
        scratch.write("debug/never-crash-lints", "bytes");
        let mut artifact = an_artifact(&scratch, "never-crash-lints", &["debug/never-crash-lints"]);
        artifact.package_id = library[0].id.clone();
        let found = errors(&root, &root.join("rust"), &[artifact]).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("never-crash-lints: no dep-info rustc wrote for"), "{found:?}");
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
        let errors = judge(&scratch.0, &scratch.0.join("rust"), &scratch.0.join("rust/target"), "x", &scratch.0.join("rust/core/x"), &read).unwrap();
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

    const TOOLCHAIN: &str = "nightly-2026-08-20-aarch64-apple-darwin";

    /// What judging a crate `x` that read the files `read` (and its own source) says, with the workspace
    /// building in `target_directory`, a path relative to the scratch root.
    fn judged_in(scratch: &Scratch, target_directory: &str, read: &[&String]) -> Vec<String> {
        scratch.write("rust/Cargo.toml", "");
        scratch.write("rust/core/x/Cargo.toml", "");
        scratch.write("rust/core/x/src/lib.rs", "");
        for file in read {
            scratch.write(&format!("rust/{file}"), "");
        }
        let read: BTreeSet<String> = ["core/x/src/lib.rs".to_string()].into_iter().chain(read.iter().map(|file| file.to_string())).collect();
        judge(&scratch.0, &scratch.0.join("rust"), &scratch.0.join(target_directory), "x", &scratch.0.join("rust/core/x"), &read).unwrap()
    }

    /// The path `cargo dylint` gives a lint library's build in the build folder `builds`.
    fn built_under(builds: &str, name: &str) -> String {
        format!("{builds}/dylint/libraries/{TOOLCHAIN}/release/lib{name}@{TOOLCHAIN}.dylib")
    }

    /// The lint library's run records the library dylint loads; that build of the gate's own sources is
    /// the one file outside a crate it may read, and only where `cargo dylint` builds it, under its name.
    #[test]
    fn the_lint_library_dylint_loads_is_no_crate_code_but_a_look_alike_is() {
        let scratch = Scratch::new("compiled-lint-library");
        let library = built_under("../target", "never_crash_lints");
        let other = built_under("../target", "something_else");
        let elsewhere = format!("../target/debug/lib{}@{TOOLCHAIN}.dylib", "never_crash_lints");
        let errors = judged_in(&scratch, "target", &[&library, &other, &elsewhere]);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("libsomething_else@")), "another library: {errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("target/debug/libnever_crash_lints@")), "outside dylint's own folder: {errors:?}");
    }

    /// The pre-push hook gates each commit with its own build folder, so `cargo dylint` builds the library
    /// there, and the folder is the one cargo names, not `rust/target`.
    #[test]
    fn the_lint_library_built_in_the_folder_the_hook_names_is_accepted_and_one_built_outside_any_is_refused() {
        let scratch = Scratch::new("compiled-moved-target");
        let moved = "pushed/target-7";
        let in_the_hook_s_folder = built_under(&format!("../{moved}"), "never_crash_lints");
        let in_the_default_folder = built_under("target", "never_crash_lints");
        let outside_any_build_folder = built_under("../unrelated", "never_crash_lints");
        let errors = judged_in(&scratch, moved, &[&in_the_hook_s_folder, &in_the_default_folder, &outside_any_build_folder]);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("rust/target/dylint/libraries/") && e.contains("compilation reads it, but it is not")), "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("unrelated/dylint/libraries/")), "{errors:?}");
        assert!(!errors.iter().any(|e| e.contains("pushed/")), "the folder the hook names is accepted: {errors:?}");
    }

    /// The child half of the test below, run with the build folder in `CARGO_TARGET_DIR` as the hook's gate
    /// has it: the whole path from cargo's messages, through cargo's own answer for the build folder, to the
    /// verdict on the library's dep-info, with the lint library's real crate.
    #[test]
    #[ignore = "run by the_build_folder_cargo_names_is_where_the_lint_library_is_accepted, in a gate's environment"]
    fn child_judges_the_lint_library_s_run_in_the_build_folder_cargo_names() {
        let Some(scratch) = std::env::var_os("XTASK_COMPILED_SCRATCH") else { return };
        let scratch = Path::new(&scratch);
        let root = crate::repo_root();
        let library = crate::meta::read(&root.join(crate::machinery::LINT_LIBRARY), crate::meta::Graph::Workspace).unwrap();
        let target = crate::meta::target_directory(&root.join("rust")).unwrap();
        assert_eq!(target, scratch.join("pushed/target-7"), "cargo names the folder the environment sets");
        let artifact = library_unit(&library[0].id, &scratch.join("deps/libnever_crash_lints-0123456789abcdef.rmeta"));
        let found = errors(&root, &root.join("rust"), &[artifact]).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("unrelated/dylint/libraries/"), "{found:?}");
    }

    #[test]
    fn the_build_folder_cargo_names_is_where_the_lint_library_is_accepted() {
        let scratch = Scratch::new("compiled-cargo-target");
        let under = |folder: &str| scratch.0.join(built_under(folder, "never_crash_lints"));
        let in_the_hook_s_folder = under("pushed/target-7");
        let outside_any_build_folder = under("unrelated");
        for library in [&in_the_hook_s_folder, &outside_any_build_folder] {
            std::fs::create_dir_all(library.parent().unwrap()).unwrap();
            std::fs::write(library, "").unwrap();
        }
        scratch.write(
            "deps/never_crash_lints-0123456789abcdef.d",
            format!("deps/libnever_crash_lints-0123456789abcdef.rmeta: src/lib.rs {} {}\n", in_the_hook_s_folder.display(), outside_any_build_folder.display()),
        );
        scratch.write("deps/libnever_crash_lints-0123456789abcdef.rmeta", "");
        craftbridge_process::rerun(
            "compiled::tests::child_judges_the_lint_library_s_run_in_the_build_folder_cargo_names",
            &[("XTASK_COMPILED_SCRATCH", scratch.0.as_os_str()), ("CARGO_TARGET_DIR", scratch.0.join("pushed/target-7").as_os_str())],
            "the run in the build folder cargo names did not accept the library there and refuse the one outside",
        )
        .unwrap();
    }

    /// The lint library is a workspace of its own the gate compiles too, so a unit of it is judged like a
    /// member's, from its own folder, where rustc runs for it: its own source passes, and any other file
    /// its compilation reads is refused.
    #[test]
    fn the_lint_library_s_own_compilation_is_judged_from_its_own_folder() {
        let root = crate::repo_root();
        let library = crate::meta::read(&root.join(crate::machinery::LINT_LIBRARY), crate::meta::Graph::Workspace).unwrap();
        let scratch = Scratch::new("compiled-library");
        scratch.write("deps/never_crash_lints-0123456789abcdef.d", "deps/libnever_crash_lints-0123456789abcdef.rmeta: src/lib.rs Cargo.toml ../../README.md\n");
        scratch.write("deps/libnever_crash_lints-0123456789abcdef.rmeta", "");
        let artifact = library_unit(&library[0].id, &scratch.0.join("deps/libnever_crash_lints-0123456789abcdef.rmeta"));
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
            "{\"reason\":\"compiler-message\",\"message\":{\"message\":\"x\",\"rendered\":\"warning: x\\n\"}}\n",
            "{\"reason\":\"compiler-message\",\"message\":{\"rendered\":\"warning: rendered alone\\n\"}}\n",
            "test a ... ok\n",
            "{\"reason\":\"build-finished\",\"success\":true}\n"
        );
        assert_eq!(read_reported(input.as_bytes()).unwrap().diagnostics, ["x"], "each diagnostic's own text, not cargo's rendering");
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

    /// The finding for a unit of crate `name` built in a build of tests without `without`, in its words.
    fn refusal(name: &str, unit: &str, without: &str) -> String {
        format!(
            "{name}: cargo built {unit} without {without} in a build of tests: CraftBridge's own crates keep overflow checks and debug assertions in every unit a build of tests compiles, so an overflow or a broken debug assertion in their code fails the build or the test instead of passing unnoticed (AGENTS.md §7)"
        )
    }

    /// Cargo's report of a unit's profile, read as cargo gives it: in a build of tests, a unit built without
    /// either check is a finding naming the crate, the unit and the checks, a test and a library the tests
    /// link alike; a unit with both is not, nor is any unit of a build that compiles no test, as a release
    /// build's; a unit whose profile cargo leaves out, or a check it leaves out of one, is a finding, and a
    /// report that leaves out whether the unit is a test counts it as one.
    #[test]
    fn a_unit_cargo_built_for_tests_without_overflow_checks_or_debug_assertions_is_refused() {
        let unit = |profile: &str| {
            let line = format!(
                "{{\"reason\":\"compiler-artifact\",\"package_id\":\"p\",\"target\":{{\"name\":\"t\",\"kind\":[\"lib\"]}},\"filenames\":[]{profile}}}\n"
            );
            read_messages(line.as_bytes()).unwrap().remove(0)
        };
        let profile = |test: bool, overflow: bool, assertions: bool| {
            format!(",\"profile\":{{\"opt_level\":\"1\",\"debuginfo\":1,\"debug_assertions\":{assertions},\"overflow_checks\":{overflow},\"test\":{test}}}")
        };
        let (test, linked) = ("the test t", "t");
        assert_eq!(profile_error("x", &unit(&profile(true, false, true)), true), Some(refusal("x", test, "overflow checks")));
        assert_eq!(profile_error("x", &unit(&profile(true, true, false)), true), Some(refusal("x", test, "debug assertions")));
        assert_eq!(profile_error("x", &unit(&profile(true, false, false)), true), Some(refusal("x", test, "overflow checks or debug assertions")));
        assert_eq!(profile_error("x", &unit(&profile(false, false, true)), true), Some(refusal("x", linked, "overflow checks")), "a library the tests link");
        assert_eq!(profile_error("x", &unit(&profile(true, true, true)), true), None);
        assert_eq!(profile_error("x", &unit(&profile(false, false, false)), false), None, "a build that compiles no test may build without them");
        let unknown = "x: cargo reported no profile for t, so whether its tests keep overflow checks and debug assertions is unknown";
        assert_eq!(profile_error("x", &unit(""), false), Some(unknown.to_string()));
        assert_eq!(profile_error("x", &unit(",\"profile\":{\"test\":true}"), true), Some(refusal("x", test, "overflow checks or debug assertions")));
        let untold = unit(",\"profile\":{\"debug_assertions\":true,\"overflow_checks\":true}");
        assert_eq!(untold.profile, Some(UnitProfile { test: true, without: Vec::new() }), "a report silent on `test` counts the unit as a test");
    }

    /// The whole path with the real cargo, over a probe workspace whose members, named as no crate of ours
    /// is, are a library with a test, the library it links, which has no test of its own and a build script,
    /// and a proc-macro it uses: each way a profile can switch a check off for what a build of tests compiles
    /// — a test profile that inherits another, a package override by a package spec's URL, one by the bare
    /// name in the dev profile tests inherit, one for the library with no test, and the build override,
    /// which reaches only what runs on the host — is found from cargo's report, naming each unit it reaches;
    /// switching them off only for dependencies, or only in the release profile, is not, and neither is a
    /// release build, which compiles no test.
    #[test]
    fn a_member_s_units_built_for_tests_without_the_checks_are_found_however_the_profile_spells_it() {
        let scratch = Scratch::new("compiled-profiles");
        let package = |name: &str, rest: &str| format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n{rest}");
        scratch.write("rust/probe/Cargo.toml", package("probe", "[dependencies]\nlinked = { path = \"../linked\" }\nshaped = { path = \"../shaped\" }\n"));
        scratch.write("rust/probe/src/lib.rs", "pub fn probe() {\n    shaped::same!(linked::linked());\n}\n");
        scratch.write("rust/probe/tests/behaviour.rs", "#[test]\nfn probes() {\n    probe::probe();\n}\n");
        scratch.write("rust/linked/Cargo.toml", package("linked", "[lib]\ntest = false\n"));
        scratch.write("rust/linked/build.rs", "fn main() {}\n");
        scratch.write("rust/linked/src/lib.rs", "pub fn linked() {}\n");
        scratch.write("rust/shaped/Cargo.toml", package("shaped", "[lib]\nproc-macro = true\n"));
        scratch.write(
            "rust/shaped/src/lib.rs",
            "/// Its input, unchanged.\n#[proc_macro]\npub fn same(input: proc_macro::TokenStream) -> proc_macro::TokenStream {\n    input\n}\n",
        );
        let spec = format!("path+file://{}#0.1.0", std::fs::canonicalize(scratch.0.join("rust/probe")).unwrap().display());
        let probe = [("probe", "the test behaviour"), ("probe", "the test probe"), ("probe", "probe")];
        let linked = [("linked", "linked"), ("linked", "build-script-build")];
        let shaped = [("shaped", "the test shaped"), ("shaped", "shaped")];
        let tests: &[&str] = &["test", "--no-run"];
        let cases: [(String, &[&str], Vec<(&str, &str)>, &str); 7] = [
            (
                "[profile.fast]\ninherits = \"release\"\n\n[profile.test]\ninherits = \"fast\"\n".to_string(),
                tests,
                [probe.as_slice(), &linked, &shaped].concat(),
                "overflow checks or debug assertions",
            ),
            (format!("[profile.test.package.\"{spec}\"]\noverflow-checks = false\n"), tests, probe.to_vec(), "overflow checks"),
            ("[profile.dev.package.probe]\ndebug-assertions = false\n".to_string(), tests, probe.to_vec(), "debug assertions"),
            ("[profile.test.package.linked]\noverflow-checks = false\n".to_string(), tests, linked.to_vec(), "overflow checks"),
            (
                "[profile.dev.build-override]\noverflow-checks = false\ndebug-assertions = false\n".to_string(),
                tests,
                vec![("linked", "build-script-build"), ("shaped", "shaped")],
                "overflow checks or debug assertions",
            ),
            (
                "[profile.test.package.\"*\"]\noverflow-checks = false\ndebug-assertions = false\n\n[profile.release]\noverflow-checks = false\n".to_string(),
                tests,
                Vec::new(),
                "",
            ),
            (String::new(), &["build", "--release"], Vec::new(), ""),
        ];
        for (profiles, command, units, without) in cases {
            scratch.write("rust/Cargo.toml", format!("[workspace]\nmembers = [\"probe\", \"linked\", \"shaped\"]\nresolver = \"3\"\n\n{profiles}"));
            let output = std::process::Command::new(env!("CARGO"))
                .current_dir(scratch.0.join("rust"))
                .env("CARGO_BUILD_BUILD_DIR", scratch.0.join("target"))
                .args(command)
                .args(["--offline", "--message-format=json", "--target-dir"])
                .arg(scratch.0.join("target"))
                .output()
                .unwrap();
            assert!(output.status.success(), "{profiles}: {}", String::from_utf8_lossy(&output.stderr));
            let artifacts = read_messages(output.stdout.as_slice()).unwrap();
            let mut found = errors(&scratch.0, &scratch.0.join("rust"), &artifacts).unwrap();
            found.sort();
            let mut expected: Vec<String> = units.iter().map(|(name, unit)| refusal(name, unit, without)).collect();
            expected.sort();
            assert_eq!(found, expected, "{profiles} {command:?}");
        }
    }

    /// What the check above holds every build of our tests to, seen from inside one: this crate's tests,
    /// built in the workspace's test profile, panic on an overflow and on a failed debug assertion.
    #[test]
    fn an_overflow_in_a_crate_of_ours_panics_in_a_test() {
        let overflow = std::panic::catch_unwind(|| std::hint::black_box(u8::MAX) + std::hint::black_box(1));
        assert!(overflow.is_err(), "overflow-checks must stay on in the profile tests are built with");
        let broken = std::panic::catch_unwind(|| debug_assert!(std::hint::black_box(false)));
        assert!(broken.is_err(), "debug-assertions must stay on in the profile tests are built with");
    }
}
