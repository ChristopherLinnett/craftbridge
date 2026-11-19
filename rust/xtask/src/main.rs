//! CraftBridge's rule checks and regeneration: `cargo xtask rules` and `cargo xtask generate`; the plan's
//! views, `cargo xtask plan-views --check`, rebuilt from their sources and compared; the judgements
//! `tools/ci.sh` hands over from the stages it runs, `cargo xtask skipped-tasks <gradle log>`,
//! `cargo xtask compiled`, and the lint library's run under its lock, `cargo xtask dylint …`; the file
//! lists its stages read, `cargo xtask files <kind>`; the pins the doctor and the gate read,
//! `cargo xtask pins`, the Swift package's builds `cargo xtask swift-builds`, and the one of those the
//! Swift Testing suite runs on, `cargo xtask swift-suite`, and the iPhone simulator it runs on, shared
//! with every other run on the machine, `cargo xtask simulator -- <program>…`; the package's library products,
//! which an app's own extension links too, `cargo xtask swift-products`, and the samples' Xcode projects,
//! `cargo xtask swift-samples`, and the Android samples' builds, `cargo xtask android-samples`; the Swift stage's
//! formatter check,
//! `cargo xtask swift-format`, and its check of the names hand-written Swift gives beside generated Swift,
//! `cargo xtask swift-api <graphs>…`; the XCFramework `tools/build-xcframework.sh` builds,
//! `cargo xtask xcframework`; and for `tools/build-android.sh`, the native library the SDK ships,
//! `cargo xtask sdk-library`, and the 16 KB alignment it hands over, `cargo xtask elf-alignment
//! <llvm-readelf> <library>…`.
//!
//! The rules in `AGENTS.md` that no compiler, formatter or linter already enforces are checked here,
//! behind one command, so a change cannot merge with one broken. Each check lives in its own module: a
//! `check` that reads the repository and hands what it read to pure functions, which the module's tests
//! feed with input built in memory.
//!
//! Checks run even when an earlier one fails: one run reports everything that is broken, because a
//! half-fixed change still gates.

mod adaptations;
mod attributes;
mod bindings;
mod boundary_surface;
mod buildfiles;
mod compiled;
mod device_lock;
mod direction;
mod duplicates;
mod effects_api;
mod elf_alignment;
mod filelock;
mod files;
mod generate;
mod generated;
mod git;
mod gradle_log;
mod home;
mod hygiene;
mod kotlin_bindings;
mod layers;
mod layout;
mod licences;
mod lints;
mod machinery;
mod meta;
mod pin;
mod pins;
mod plan_views;
mod report;
mod rust_syntax;
mod samples;
mod scan;
mod scratch;
mod simulator;
mod sources;
mod swift_api;
mod swift_bindings;
mod swift_format;
mod swift_lint;
mod unsafe_check;
mod update;
mod upstream;
mod versions;
mod xcframework;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The files a CraftBridge tree holds. Every folder this run takes as its repository must hold all of
/// them, so a folder that is not the repository is refused by name instead of being read as if it were.
const REPOSITORY_MARKERS: [&str; 4] = ["AGENTS.md", "rust/Cargo.toml", "rust/xtask/Cargo.toml", "tools/ci.sh"];

/// The variable a `tools/` script names its own tree in (`tools/env.sh`).
const NAMED_TREE: &str = "CRAFTBRIDGE_ROOT";

/// The repository this run works on: xtask's one reading of the root, which every caller goes through.
///
/// It is found from the run, not from where the binary was compiled. A binary that takes its root from
/// its own compile-time manifest folder writes the tree it was built in, wherever it is run: a copy of
/// a tree whose build folder and file times came from another tree (an `rsync -a` keeps both) reuses
/// that other tree's compiled xtask, which would write the original's `rust/Cargo.toml` and
/// `rust/Cargo.lock` on the copy's say-so. So the tree comes from the working folder and what the run
/// is handed there (`found_root`), and a run whose tree is not the one the binary was built for is
/// refused naming both folders, so a stale binary never reads or writes another tree.
fn repository_root() -> Result<PathBuf, String> {
    let here = std::env::current_dir().map_err(|error| format!("the working folder cannot be read: {error}"))?;
    let named = std::env::var_os(NAMED_TREE).filter(|named| !named.is_empty());
    root_of_run(&built_root()?, &here, named.as_deref().map(Path::new))
}

/// The tree a run standing in `here`, and handed `named`, works on with a binary built in `built`: the
/// tree it finds, when that is the very folder the binary was built in. Folders are compared as the file
/// system identifies them (`same_folder`), so the refusal neither misses a copy nor refuses the tree
/// itself reached by another path. The tree is named as the build names it, which is how cargo spells
/// every path it reports, and the checks relate those paths to the root: a script's own spelling of its
/// tree, through a firmlink, would leave every one of them outside it.
fn root_of_run(built: &Path, here: &Path, named: Option<&Path>) -> Result<PathBuf, String> {
    let found = found_root(here, named)?;
    if same_folder(&found, built) {
        return Ok(built.to_path_buf());
    }
    Err(format!(
        "this binary was built in `{}` but was run for `{}`; a copy of a tree that kept another tree's build folder runs that other tree's xtask: build in the tree to work on, or run that tree's own tools/ scripts",
        built.display(),
        found.display()
    ))
}

/// This crate's own tests read their repository through the same definition: a stale test binary,
/// compiled in one tree and run from a copy of it, fails here naming both trees rather than judge
/// the copy against the tree it was built in.
#[cfg(test)]
fn repo_root() -> PathBuf {
    repository_root().unwrap_or_else(|reason| panic!("xtask: refused: {reason}"))
}

/// A script of `tools/` run with a command line it refuses ends with 64, `EX_USAGE`, before it builds
/// anything, and says `usage` on its standard error, so a failed build is never taken for a wrong command
/// line. One definition for the tests of every script that takes arguments.
#[cfg(test)]
fn assert_script_refuses(script: &str, misused: &[&[&str]], usage: &str) {
    let path = repo_root().join("tools").join(script);
    for args in misused {
        let output = std::process::Command::new("bash").arg(&path).args(*args).output().unwrap();
        assert_eq!(output.status.code(), Some(MISUSE.into()), "{script} {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stderr).contains(usage), "{script} {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }
}

/// The tree this binary was built in: its manifest folder, two parents up, as the system resolves it.
fn built_root() -> Result<PathBuf, String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .filter(|root| !root.as_os_str().is_empty())
        .ok_or_else(|| format!("this binary names its build folder `{}`, which has no repository above it", manifest.display()))?;
    resolved(root).ok_or_else(|| format!("this binary was built in `{}`, which no longer resolves", root.display()))
}

/// The tree to work on, as the run standing in `here` names it: the tree a `tools/` script names in
/// `named`, its own, when the run stands inside it, as every `tools/` script starts xtask from inside its
/// tree — git need not know that tree; or else the git top level of the working folder, so a run by
/// hand finds the tree it stands in. A copy answers git with its own `.git` file, so even a copied tree
/// is named as itself, not as the tree it was copied from.
///
/// The variable counts only inside its tree because it outlives the script that set it: everything a
/// script starts inherits it, and so does a shell that sourced `tools/env.sh`. Taken wherever it was met,
/// a variable naming the original tree would hand the original's stale binary, run in a copy, the very
/// tree it was built for, and the copy's run would write the original.
fn found_root(here: &Path, named: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(named) = named.filter(|named| stands_inside(here, named)) {
        return craftbridge_tree(named, &format!("the tools script that started this run ({NAMED_TREE})"));
    }
    if let Some(top) = git_top_level(here) {
        return craftbridge_tree(&top, "the git top level of the working folder");
    }
    let unnamed = match named {
        Some(named) => format!("`{}` stands outside `{}`, the tree {NAMED_TREE} names, and inside no git checkout", here.display(), named.display()),
        None => format!("no tools script set {NAMED_TREE} and `{}` stands inside no git checkout", here.display()),
    };
    Err(format!("no tree to work on is named: {unnamed}; run xtask from inside a CraftBridge tree or through its tools/ scripts"))
}

/// Does `here` stand inside `tree`, or is it `tree`? Asked of each folder above it, as the file system
/// identifies folders.
fn stands_inside(here: &Path, tree: &Path) -> bool {
    here.ancestors().any(|folder| same_folder(folder, tree))
}

/// Are `one` and `other` one folder? The file system's own identity of a file, its device and inode,
/// answers for every path to one folder: through a link, `/tmp` for `/private/tmp`, in a case the disk
/// folds, and through a macOS firmlink (`/System/Volumes/Data/Users/…` for `/Users/…`), which resolving a
/// path leaves as spelt. A copy is a folder of its own, and never the same.
fn same_folder(one: &Path, other: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(one), std::fs::metadata(other)) {
        (Ok(one), Ok(other)) => one.dev() == other.dev() && one.ino() == other.ino(),
        _ => false,
    }
}

/// The top level of the git checkout enclosing `dir`, if git finds one. Every git command starts
/// through `crate::git`, which removes git's repository variables: the top level is the checkout the
/// folder stands in, never a repository a hook's or a parent's exported variables name.
fn git_top_level(dir: &Path) -> Option<PathBuf> {
    let top = String::from_utf8(crate::git::output(dir, &["rev-parse", "--show-toplevel"]).ok()?).ok()?;
    Some(PathBuf::from(top.trim_end()))
}

/// Is `candidate` a CraftBridge tree? It must hold every marker file, and resolve; the resolved path
/// is the answer, so a refusal names the tree as the system does.
fn craftbridge_tree(candidate: &Path, named: &str) -> Result<PathBuf, String> {
    let missing: Vec<&str> = REPOSITORY_MARKERS.iter().copied().filter(|marker| !candidate.join(marker).exists()).collect();
    if !missing.is_empty() {
        return Err(format!("{named} `{}` holds no CraftBridge tree (no {})", candidate.display(), missing.join(", no ")));
    }
    resolved(candidate).ok_or_else(|| format!("{named} `{}` does not resolve", candidate.display()))
}

/// The path as the system resolves it: links followed, `/tmp` become `/private/tmp`, case as stored.
fn resolved(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

fn main() -> ExitCode {
    // `args_os`, never `args`: one argument that is not valid Unicode panics the UTF-8 iterator
    // mid-iteration (AGENTS.md §3).
    dispatch(std::env::args_os().skip(1))
}

/// Run the command `arguments` name. They are collected into `String`s, and a non-Unicode one ends the
/// run with a line naming it, not with a panic.
fn dispatch(arguments: impl Iterator<Item = std::ffi::OsString>) -> ExitCode {
    let args: Vec<String> = match arguments.map(std::ffi::OsString::into_string).collect::<Result<Vec<_>, _>>() {
        Ok(args) => args,
        Err(argument) => {
            eprintln!("xtask: argument `{}` is not valid Unicode; nothing ran", argument.to_string_lossy());
            return ExitCode::from(MISUSE);
        }
    };
    // The tree to work on is settled before anything else runs: a stale binary is refused here, so no
    // command — the ones that write above all — can reach a tree other than the one it was built for.
    let root = match repository_root() {
        Ok(root) => root,
        Err(reason) => {
            eprintln!("xtask: refused: {reason}");
            return ExitCode::from(MISUSE);
        }
    };
    // Every argument is matched whole: a misspelt `--check` must not quietly become a run that writes.
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["rules"] => rules::run(&root),
        ["generate"] => generate::run(&root, false),
        ["generate", "--check"] => generate::run(&root, true),
        ["plan-views", "--check"] => plan_views::run(&root),
        ["skipped-tasks", log] => gradle_log::run(log),
        ["files", kind] => files::run(&root, kind),
        ["compiled"] => compiled::run(&root),
        ["dylint", arguments @ ..] => lints::run_dylint(&root, arguments),
        ["upstream", "update", rev] => update::run(&root, rev),
        ["upstream", "source"] => upstream::run_source(&root),
        ["upstream", "pin"] => upstream::run_pin(&root),
        ["swift-format"] => swift_format::run(&root),
        ["swift-lint"] => swift_lint::run(&root),
        ["swift-api", folders @ ..] => swift_api::run(&root, folders),
        ["pins"] => pins::run(&root),
        ["elf-alignment", readelf, libraries @ ..] => elf_alignment::run(readelf, libraries),
        ["swift-builds"] => pins::builds(&root),
        ["swift-suite"] => pins::suite(&root),
        ["swift-products"] => pins::products(&root),
        ["swift-samples"] => samples::run_swift(&root),
        ["android-samples"] => samples::run_android(&root),
        ["device-lock", device, "--", program, arguments @ ..] => device_lock::run(device, program, arguments),
        ["simulator", "--", program @ ..] => simulator::run(program),
        ["xcframework"] => xcframework::run(&root),
        ["sdk-library"] => layers::run_sdk_library(),
        _ => {
            usage();
            ExitCode::from(MISUSE)
        }
    }
}

/// The exit of a run that cannot proceed and ran nothing, sysexits' `EX_USAGE`: a misused command, an
/// argument that is not Unicode, a run refused its tree. 1 is a check or step that failed, and 2 is the
/// upstream update's breaking API change, so a run that cannot proceed must be neither.
const MISUSE: u8 = 64;

fn usage() {
    eprintln!("usage: cargo xtask rules");
    eprintln!("       cargo xtask generate [--check]");
    eprintln!("       cargo xtask plan-views --check");
    eprintln!("       cargo xtask skipped-tasks <gradle log>");
    eprintln!("       cargo xtask files <swift|shell|duplicates>");
    eprintln!("       cargo xtask upstream update <rev>   (through tools/update-upstream.sh: plan \u{a7}24 to <rev>, a commit of official EffectCraft main)");
    eprintln!("       cargo xtask upstream source   (the official URL and the full commit the pin names on official main, for tools/oracle.sh)");
    eprintln!("       cargo xtask upstream pin   (the EffectCraft revision rust/Cargo.toml pins, for tools/update-upstream.sh to fetch into a new cache)");
    eprintln!("       cargo <check|clippy|test> --message-format=json … | cargo xtask compiled");
    eprintln!("       cargo xtask dylint <cargo-dylint arguments…>   (cargo dylint, taking turns on the lock every lint library run shares)");
    eprintln!("       cargo xtask swift-format");
    eprintln!("       cargo xtask swift-lint");
    eprintln!(
        "       cargo xtask swift-api <symbol graph folder>...   (one folder for each Swift build: no hand-written public member named like a generated one of its type)"
    );
    eprintln!("       cargo xtask pins");
    eprintln!("       cargo xtask elf-alignment <llvm-readelf> <library>...   (every library aligned to 16 KB pages)");
    eprintln!("       cargo xtask swift-builds   (every build the Swift package ships: a configuration, a destination and its architectures per line)");
    eprintln!("       cargo xtask swift-suite   (the build the Swift Testing suite runs on, read from that same table)");
    eprintln!("       cargo xtask swift-products   (the Swift package's library products, as its manifest lists them: what an app's extension links)");
    eprintln!(
        "       cargo xtask swift-samples   (the Xcode projects of the iOS samples, which the Swift build builds and the licence check reads the pins of)"
    );
    eprintln!(
        "       cargo xtask android-samples   (the Gradle builds of the Android samples, which the gate builds and the licence check and the build-file rules read)"
    );
    eprintln!(
        "       cargo xtask simulator -- <program> <argument>...   (the program on one of the machine's iPhone simulators, its id where an argument says {{simulator}})"
    );
    eprintln!(
        "       cargo xtask device-lock <device> -- <program> <argument>...   (the program under the machine's lock for one device, which the runs of one device take turns on)"
    );
    eprintln!("       cargo xtask xcframework   (through tools/build-xcframework.sh: the native library's XCFramework, joined again only when it changed)");
    eprintln!("       cargo xtask sdk-library   (the native library the SDK ships, as the layer table gives it, for tools/build-android.sh)")
}

/// Run every check, print one line per failure and `ok` per clean check, and fail if any failed.
mod rules {
    use crate::report;
    use std::path::Path;
    use std::process::ExitCode;

    /// One check: its name, and what it reads from the repository to return its errors.
    type Check = (&'static str, fn(&Path) -> Result<Vec<String>, String>);

    pub(super) fn run(root: &Path) -> ExitCode {
        let checks: [Check; 14] = [
            ("hygiene", crate::hygiene::check),
            ("home", crate::home::check),
            ("layout", crate::layout::check),
            ("machinery", crate::machinery::check),
            ("buildfiles", crate::buildfiles::check),
            ("layers", crate::layers::check),
            ("direction", crate::direction::check),
            ("lints", crate::lints::check),
            ("unsafe", crate::unsafe_check::check),
            ("sources", crate::sources::check),
            ("generated", crate::generated::check),
            ("licences", crate::licences::check),
            ("duplicates", crate::duplicates::check),
            ("upstream", crate::upstream::check),
        ];
        let (failed, lines) = outcome(root, &checks);
        for line in lines {
            println!("{line}");
        }
        if failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
    }

    /// Run every check, even after one fails, and return whether any did with the lines to print: one
    /// per failure, `ok` per clean check, and a check that could not run is a failure in its own words.
    fn outcome(root: &Path, checks: &[Check]) -> (bool, Vec<String>) {
        let mut failed = false;
        let mut lines = Vec::new();
        for (name, run_check) in checks {
            match run_check(root) {
                Ok(errors) if errors.is_empty() => lines.push(report::ok_line(name)),
                Ok(errors) => {
                    failed = true;
                    lines.extend(errors.into_iter().map(|error| report::error_line(name, error)));
                }
                Err(failure) => {
                    failed = true;
                    lines.push(report::error_line(name, format!("could not run this check: {failure}")));
                }
            }
        }
        (failed, lines)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn every_check_runs_after_one_fails_and_one_that_cannot_run_fails() {
            let checks: [Check; 3] = [
                ("first", |_| Ok(vec!["broken here".to_string()])),
                ("second", |_| Err("cargo metadata exited with 101".to_string())),
                ("third", |_| Ok(Vec::new())),
            ];
            let (failed, lines) = outcome(Path::new("."), &checks);
            assert!(failed);
            assert_eq!(lines, ["rules: first: broken here", "rules: second: could not run this check: cargo metadata exited with 101", "rules: third: ok"]);
            let (failed, lines) = outcome(Path::new("."), &checks[2..]);
            assert!(!failed && lines == ["rules: third: ok"]);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::process::ExitCode;

    /// A misused command exits 64, never 2, which the upstream update keeps for a breaking API change:
    /// an argument that is not Unicode, which `tools/update-upstream.sh` passes through, and a command
    /// missing a word.
    #[test]
    fn a_misused_command_exits_64() {
        use std::os::unix::ffi::OsStringExt;
        let not_unicode = OsString::from_vec(vec![0xff]);
        assert_eq!(super::dispatch([OsString::from("upstream"), OsString::from("update"), not_unicode].into_iter()), ExitCode::from(64));
        assert_eq!(super::dispatch([OsString::from("upstream"), OsString::from("update")].into_iter()), ExitCode::from(64));
    }

    /// `upstream pin` is a command a script runs on the tree as it stands: it answers with the tree's pin,
    /// never the usage a word the dispatch does not know gets.
    #[test]
    fn the_pin_is_answered_for_the_tree() {
        assert_eq!(super::dispatch([OsString::from("upstream"), OsString::from("pin")].into_iter()), ExitCode::SUCCESS);
    }

    /// What the gate at `gate` does with a selection it only reads, a refusal or `--list`, for a runner whose
    /// system is `runner` (`RUNNER_OS`, which a hosted runner sets in every test's environment too).
    fn gate_output_of(gate: &std::path::Path, runner: Option<&str>, args: &[&str]) -> std::process::Output {
        let mut command = std::process::Command::new("bash");
        command.arg(gate).args(args);
        match runner {
            Some(system) => command.env("RUNNER_OS", system),
            None => command.env_remove("RUNNER_OS"),
        };
        command.output().unwrap()
    }

    /// [`gate_output_of`] for the tree's own `tools/ci.sh`.
    fn gate_output(runner: Option<&str>, args: &[&str]) -> std::process::Output {
        gate_output_of(&crate::repo_root().join("tools/ci.sh"), runner, args)
    }

    /// The gate refuses a selection it cannot run before any stage starts, in its own words: a stage name
    /// it does not know, `--except` naming nothing, `--except` leaving nothing, and `--except` anywhere but
    /// first, which is no stage; and `--hosted` on a runner whose system it has no job for, none at all, or
    /// with a stage named beside it. So a selection that misspells a stage or whose list empties, and a
    /// hosted job on a runner the gate has no job for, fail rather than pass on less. Every stage is read
    /// from the gate's own refusal, so no second list of them stands here. Each selection is asked behind
    /// `--list`, which reaches every refusal first, so a gate that stopped refusing one lists its stages
    /// here rather than running them, this test among them.
    #[test]
    fn the_gate_refuses_a_selection_it_cannot_run() {
        let gate = |runner: Option<&str>, args: &[&str]| {
            let output = gate_output(runner, &[&["--list"], args].concat());
            (output.status.code(), String::from_utf8_lossy(&output.stderr).trim_end().to_string())
        };
        let (code, misspelt) = gate(None, &["rust_fmtt"]);
        assert_eq!(code, Some(2), "{misspelt}");
        let stages = misspelt
            .strip_prefix("ci: no stage 'rust_fmtt' (stages: ")
            .and_then(|rest| rest.strip_suffix("; groups: rust, swift)"))
            .unwrap_or_else(|| panic!("the refusal names every stage: {misspelt}"));
        assert_eq!(
            gate(None, &["--except"]),
            (Some(2), "ci: --except names no stage to leave out: name them, or run tools/ci.sh alone for every stage".to_string())
        );
        let every: Vec<&str> = std::iter::once("--except").chain(stages.split(' ')).collect();
        assert_eq!(gate(None, &every), (Some(2), "ci: --except leaves no stage to run".to_string()));
        let (code, late) = gate(None, &["shell", "--except", "rules"]);
        assert!(code == Some(2) && late.starts_with("ci: no stage '--except' (stages: "), "{late}");
        for (runner, named) in [(Some("Windows"), "Windows"), (Some("linux"), "linux"), (None, "")] {
            assert_eq!(
                gate(runner, &["--hosted"]),
                (Some(2), format!("ci: --hosted runs the hosted job of the runner's system, which RUNNER_OS names Linux or macOS; it names '{named}'"))
            );
        }
        assert_eq!(
            gate(Some("Linux"), &["--hosted", "rules"]),
            (Some(2), "ci: --hosted names no stage: RUNNER_OS, the runner's system, chooses the hosted job's stages".to_string())
        );
    }

    /// The hosted jobs' stages, read as each job asks for them (`tools/ci.sh --hosted`, the workflow's only
    /// words for them): the Linux job runs the host tools' clippy and lint library runs and their tests, so
    /// the code only Linux compiles (`cfg(target_os = "linux")`) meets a compiler, the lints and a test
    /// there; the macOS job runs the whole workspace's, which the host tools' are part of, so Linux alone
    /// runs those as stages of their own; and the stages that need Xcode, the phones' SDKs, the iOS
    /// Simulator or the host's GPU stay on macOS.
    #[test]
    fn the_linux_job_lints_and_tests_the_host_tools_and_macos_keeps_the_rest() {
        let job = |system: &str| {
            let output = gate_output(Some(system), &["--list", "--hosted"]);
            assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr));
            String::from_utf8_lossy(&output.stdout).split_whitespace().map(str::to_string).collect::<Vec<_>>()
        };
        let linux = job("Linux");
        let macos = job("macOS");
        for (host, whole) in [("rust_host_clippy", "rust_clippy"), ("rust_host_test", "rust_test")] {
            assert!(linux.contains(&host.to_string()), "the Linux job does not run {host}: {linux:?}");
            assert!(!macos.contains(&host.to_string()), "the macOS job runs {host} through {whole}: {macos:?}");
        }
        for stage in [
            "doctor",
            "rules",
            "rust_clippy",
            "rust_test",
            "conformance",
            "rust_mobile",
            "generated",
            "swift_format",
            "swift_build",
            "swift_test",
            "swift_lint",
            "android_samples",
            "shell",
        ] {
            assert!(macos.contains(&stage.to_string()) && !linux.contains(&stage.to_string()), "{stage} runs on macOS alone: linux {linux:?}, macos {macos:?}");
        }
    }

    /// The workflow names no stage: its one gate step is `tools/ci.sh --hosted`, which takes each job's stages
    /// from the runner's system, and nothing in it sets that system. A workflow that ran the gate another way
    /// would choose stages in YAML, where the split tests above cannot see it.
    #[test]
    fn the_workflow_runs_the_hosted_gate_and_names_no_stage() {
        let workflow = std::fs::read_to_string(crate::repo_root().join(".github/workflows/ci.yml")).unwrap();
        let code: Vec<&str> = workflow.lines().map(str::trim).filter(|line| !line.starts_with('#')).collect();
        let gate: Vec<&&str> = code.iter().filter(|line| line.contains("ci.sh") && !line.starts_with("- name:")).collect();
        assert_eq!(gate, [&"run: tools/ci.sh --hosted"], "the workflow runs the gate other than as `tools/ci.sh --hosted`");
        assert!(code.iter().all(|line| !line.contains("RUNNER_OS")), "the workflow sets or reads the runner's system, which only the runner may");
    }

    /// The host tools' clippy, lint library and test runs read one list of packages, `host_tools`, so a tool
    /// added to it is linted and tested by all three, and none of them can quietly keep a list of its own:
    /// the lines of the two stages name no package, and each of the three runs takes the list.
    #[test]
    fn the_host_tools_stages_share_one_list_of_packages() {
        let gate = std::fs::read_to_string(crate::repo_root().join("tools/ci.sh")).unwrap();
        let (_, from_clippy) = gate.split_once("stage_rust_host_clippy() {").expect("the gate has the host tools' clippy stage");
        let (clippy, from_test) = from_clippy.split_once("stage_rust_host_test()").expect("the gate has the host tools' test stage after it");
        let stages = format!("{clippy}{}", from_test.lines().next().expect("the test stage ends its line"));
        assert!(!stages.contains("--package") && !stages.contains("-p "), "a host tools stage names a package of its own:\n{stages}");
        assert_eq!(stages.matches("\"${host_tools[@]}\"").count(), 3, "clippy, the lint library and the tests each take the list:\n{stages}");
    }

    /// The gate holds its hosted split before it reads any name, `--list` too: a stage function `stages` does
    /// not list, which neither job would run; a stage the Linux job does not run that gives no reason it is
    /// left to macOS, whether it is new or its reason went; and a reason for a stage the Linux job runs, for no
    /// stage, a second one, or one that is no `<stage>: <reason>` — each refused in its own words. Each plant
    /// is made in a copy of the tree's own gate, which passes untouched.
    #[test]
    fn the_gate_refuses_a_hosted_split_that_loses_a_stage_or_its_reason() {
        let root = crate::repo_root();
        let gate = std::fs::read_to_string(root.join("tools/ci.sh")).unwrap();
        let env = std::fs::read_to_string(root.join("tools/env.sh")).unwrap();
        // The gate with each `(from, to)` made in turn, each `from` asserted to be there, so no plant is lost
        // to a moved line.
        let planted = |plants: &[(&str, &str)]| {
            let mut text = gate.clone();
            for (from, to) in plants {
                assert!(text.contains(from), "the plant's anchor {from:?} is in tools/ci.sh");
                text = text.replacen(from, to, 1);
            }
            let scratch = crate::scratch::Scratch::new("ci-split");
            scratch.write("tools/env.sh", &env);
            scratch.write("tools/ci.sh", text);
            let output = gate_output_of(&scratch.0.join("tools/ci.sh"), None, &["--list", "rules"]);
            (output.status.code(), String::from_utf8_lossy(&output.stderr).trim_end().to_string())
        };
        let refused = |messages: &[&str]| (Some(2), messages.iter().map(|message| format!("ci: {message}")).collect::<Vec<_>>().join("\n"));
        assert_eq!(planted(&[]), (Some(0), String::new()), "the tree's own split");
        let function = ("\nstage_rules() {", "\nstage_planted() { :; }\nstage_rules() {");
        let listed = ("stages=(doctor rules ", "stages=(doctor rules planted ");
        let doctor = "  \"doctor: it checks Xcode, the NDK and the phone targets\"\n";
        let reason = |entry: &str| format!("left_to_macos=(\n  \"{entry}\"\n");
        let no_reason = |stage: &str| format!("{stage} runs in no Linux job, and left_to_macos gives no reason it is left to macOS");
        assert_eq!(planted(&[function]), refused(&["stage_planted is a stage `stages` does not list, so no hosted job runs it"]));
        assert_eq!(planted(&[function, listed]), refused(&[&no_reason("planted")]));
        assert_eq!(planted(&[(doctor, "")]), refused(&[&no_reason("doctor")]));
        for (stage, why) in [("rust_fmt", "which the Linux job runs"), ("rust_host_clippy", "which the Linux job runs"), ("rust_fmtt", "which is no stage")] {
            let entry = reason(&format!("{stage}: planted"));
            assert_eq!(planted(&[("left_to_macos=(\n", &entry)]), refused(&[&format!("left_to_macos gives a reason for {stage}, {why}")]));
        }
        let twice = reason("shell doctor: planted");
        assert_eq!(
            planted(&[("left_to_macos=(\n", &twice)]),
            refused(&["left_to_macos gives doctor a second reason", "left_to_macos gives shell a second reason"])
        );
        for malformed in ["doctor:", "doctor: ", "doctor - planted", ": planted", " : planted"] {
            let entry = format!("  \"{malformed}\"\n");
            assert_eq!(
                planted(&[(doctor, &entry)]),
                refused(&[&format!("left_to_macos holds '{malformed}', which is no '<stage>…: <reason>'"), &no_reason("doctor")])
            );
        }
    }

    /// A script that removes its run's folder through `craftbridge_remove_on_exit` passes only by saying
    /// so: one stopped by an unset variable under `set -u` fails, which bash 3.2 would otherwise end with
    /// the 0 of its EXIT trap; one that exits with a status of its own keeps it; and every way out removes
    /// the folder. A cleanup the script names runs on every way out, while the folder is still there, and
    /// stops at its first failing command, however the script ended; one that fails, or that an unset
    /// variable stops, fails a run that would have passed and still lets the folder go, as a folder that
    /// will not go fails it. A second folder, and a cleanup that names no function, are refused at the call
    /// in their own words, and both folders go. Run with the bash on the search path, macOS's 3.2 here, as
    /// the scripts' `#!` line runs.
    #[test]
    fn a_script_s_run_folder_goes_and_only_a_finished_script_passes() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = crate::scratch::Scratch::new("env-remove-on-exit");
        let env = crate::repo_root().join("tools/env.sh");
        let folder = scratch.0.join("run");
        let marker = scratch.0.join("cleaned");
        let run_said = |call: &str, body: &str| {
            std::fs::create_dir_all(&folder).unwrap();
            let _ = std::fs::remove_file(&marker);
            let script = format!("set -euo pipefail\nsource \"$1\"\nfolder=\"$2\" marker=\"$3\"\n{call}\n{body}\n");
            let output = std::process::Command::new("bash").arg("-c").arg(script).arg("bash").arg(&env).arg(&folder).arg(&marker).output().unwrap();
            (output.status.code(), folder.exists(), marker.exists(), String::from_utf8_lossy(&output.stderr).trim_end().to_string())
        };
        let run = |cleanup: Option<&str>, body: &str| {
            let call = match cleanup {
                Some(cleanup) => format!("tidy() {{ {cleanup}; }}\ncraftbridge_remove_on_exit \"$folder\" tidy"),
                None => "craftbridge_remove_on_exit \"$folder\"".to_string(),
            };
            let (code, left, cleaned, _) = run_said(&call, body);
            (code, left, cleaned)
        };
        assert_eq!(run(None, "echo \"$craftbridge_no_such_variable\"\ncraftbridge_passed=1"), (Some(1), false, false), "stopped by an unset variable");
        assert_eq!(
            run(None, "f() { echo \"$craftbridge_no_such_variable\"; }\nif f; then :; fi\ncraftbridge_passed=1"),
            (Some(1), false, false),
            "inside a test"
        );
        assert_eq!(run(None, "true"), (Some(1), false, false), "ended without saying it passed");
        assert_eq!(run(None, "exit 64"), (Some(64), false, false), "an exit of its own");
        assert_eq!(run(None, "sh -c 'exit 2'\ncraftbridge_passed=1"), (Some(2), false, false), "a failure set -e stops on");
        assert_eq!(run(None, "craftbridge_passed=1"), (Some(0), false, false), "finished");
        let tidy = Some("[ -d \"$folder\" ] && touch \"$marker\"");
        assert_eq!(run(tidy, "craftbridge_passed=1"), (Some(0), false, true), "the cleanup runs before the folder goes, and a pass stays one");
        assert_eq!(
            run(tidy, "echo \"$craftbridge_no_such_variable\"\ncraftbridge_passed=1"),
            (Some(1), false, true),
            "the cleanup runs after an unset variable"
        );
        assert_eq!(run(tidy, "exit 64"), (Some(64), false, true), "the cleanup runs after an exit of its own, which keeps its status");
        assert_eq!(run(Some("false"), "craftbridge_passed=1"), (Some(1), false, false), "a cleanup that fails fails the run");
        assert_eq!(run(Some("echo \"$craftbridge_no_such_variable\""), "craftbridge_passed=1"), (Some(1), false, false), "a cleanup an unset variable stops");
        assert_eq!(run(Some("false"), "exit 64"), (Some(64), false, false), "a failed cleanup keeps a failure's own status");
        let stops = Some("false\ntouch \"$marker\"");
        assert_eq!(run(stops, "craftbridge_passed=1"), (Some(1), false, false), "a cleanup stops at its first failing command, which fails the run");
        assert_eq!(run(stops, "f() { exit 0; }\nif f; then :; fi"), (Some(1), false, false), "and so it does when the script ended inside a test");
        let kept = folder.join("kept");
        assert_eq!(
            run(None, "mkdir \"$folder/kept\"\ntouch \"$folder/kept/file\"\nchmod 500 \"$folder/kept\"\ncraftbridge_passed=1"),
            (Some(1), true, false),
            "a folder that will not go fails the run"
        );
        std::fs::set_permissions(&kept, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(&folder).unwrap();
        let second = scratch.0.join("run.second");
        assert_eq!(
            run_said("craftbridge_remove_on_exit \"$folder\"", "mkdir \"$folder.second\"\ncraftbridge_remove_on_exit \"$folder.second\"\ncraftbridge_passed=1"),
            (
                Some(1),
                false,
                false,
                format!(
                    "bash: craftbridge_remove_on_exit is called once a run, and {} is this run's folder: make {} inside it",
                    folder.display(),
                    second.display()
                )
            ),
            "a second folder is refused"
        );
        assert!(!second.exists(), "the refused folder goes too");
        assert_eq!(
            run_said("craftbridge_remove_on_exit \"$folder\" untidy", "craftbridge_passed=1"),
            (Some(1), false, false, "bash: craftbridge_remove_on_exit names the cleanup untidy, which is no function".to_string()),
            "a cleanup that names no function is refused"
        );
    }

    /// Every bash this machine can run the gate's scripts with: the one `bash` names on the search
    /// path, plus every standard place a machine's own or an installed bash sits, each file named
    /// once. The two bashes the gate meets say different things about the names a shell cannot hold
    /// that an environment hands its children: bash 5 — the hosted Linux job's — leaves them out of
    /// `compgen -e` altogether, while bash 3.2, which macOS ships at `/bin/bash`, lists them beside
    /// every other exported name, and that is the only difference between the two listings, since
    /// `compgen -e` holds exported names and nothing else. So a case that catches a drop reading its
    /// names from `compgen -e` rather than from the environment can only fail under bash 5; every
    /// case therefore runs under every bash found, and the hosted Linux job is one that always finds
    /// one. Under bash 3.2 alone no such case is writable, which is why the loop asks for more.
    fn the_bashes_the_gate_runs_under() -> Vec<std::path::PathBuf> {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        let path = std::env::var_os("PATH").unwrap_or_default();
        candidates.extend(std::env::split_paths(&path).map(|folder| folder.join("bash")));
        candidates.extend(["/bin/bash", "/opt/homebrew/bin/bash", "/usr/local/bin/bash"].map(std::path::PathBuf::from));
        let mut found: Vec<std::path::PathBuf> = Vec::new();
        for candidate in candidates {
            if !candidate.is_file() {
                continue;
            }
            let Ok(bash) = candidate.canonicalize() else {
                continue;
            };
            if found.contains(&bash) {
                continue;
            }
            let answer = std::process::Command::new(&bash).arg("--version").output();
            if !matches!(answer, Ok(run) if run.status.success()) {
                continue;
            }
            found.push(bash);
        }
        assert!(!found.is_empty(), "these scripts run under a bash, and none answers here");
        found
    }

    /// The `swift_test` stage hands the suite's test run its environment through `craftbridge_set_test_runner_env`,
    /// because `xcodebuild test` gives every `TEST_RUNNER_<name>` variable of its environment to the test
    /// process under `<name>`, the prefix dropped, and `simctl` gives every `SIMCTL_CHILD_<name>` to what it
    /// starts in the simulator, the boot of the run's own simulator among them: left alone, a
    /// `TEST_RUNNER_CRAFTBRIDGE_BASELINE=1` or `SIMCTL_CHILD_CRAFTBRIDGE_BASELINE=1` still exported after a
    /// measurement run asked for by hand would ask that measurement for its figures beside every other
    /// suite. What the environment holds afterwards of either prefix is exactly the `TEST_RUNNER_`
    /// assignments the run names — an inherited one of the same name gives way to the run's — and every
    /// other variable stands as it was. A name is read off the environment whole, whatever its value
    /// holds: a value with a newline or another `=` in it drops as any other does, and the run's own
    /// assignment of such a value stands as any other does. A word that is no such assignment is
    /// refused in these words, and so is an inherited variable of either prefix whose name the shell
    /// will not unset, however it is spelt — a dash, a dot, a bang or a space in it: bash hands every
    /// such one on to the tests all the same. A refusal is status 2 whether the call stands alone or
    /// opens an `&&` as the stage runs it, and what follows the call never runs. Every case runs under
    /// every bash the machine has, which `the_bashes_the_gate_runs_under` lists; no simulator is
    /// asked for, since what is judged is the environment itself.
    #[test]
    fn the_suite_run_holds_only_the_test_environment_its_stage_names() {
        let env = crate::repo_root().join("tools/env.sh");
        let started = |bash: &std::path::Path, shape: &str, caller: &[(&str, &str)], named: &[&str]| {
            let script = match shape {
                // The stage calls the helper on the left of an `&&`, where bash holds its `set -e` off:
                // a refusal must skip what runs after it, and the call's own status must stand.
                "and" => "set -euo pipefail\nsource \"$1\"\nshift\ncraftbridge_set_test_runner_env \"$@\" && printenv\nexit $?",
                _ => "set -euo pipefail\nsource \"$1\"\nshift\ncraftbridge_set_test_runner_env \"$@\"\nprintenv",
            };
            let mut command = std::process::Command::new(bash);
            command.arg("-c").arg(script).arg("bash").arg(&env).args(named);
            for (name, value) in caller {
                command.env(name, value);
            }
            let output = command.output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let mut held: Vec<&str> = stdout.lines().filter(|line| line.starts_with("TEST_RUNNER_") || line.starts_with("SIMCTL_CHILD_")).collect();
            held.sort_unstable();
            (output.status.code(), held.join("\n"), stdout, String::from_utf8_lossy(&output.stderr).trim_end().to_string())
        };
        for bash in the_bashes_the_gate_runs_under() {
            let (code, held, all, err) = started(
                &bash,
                "direct",
                &[
                    ("TEST_RUNNER_CRAFTBRIDGE_BASELINE", "1"),
                    ("SIMCTL_CHILD_CRAFTBRIDGE_BASELINE", "1"),
                    ("TEST_RUNNER_HOME", "/the/developer/home"),
                    ("TEST_RUNNER_TMPDIR", "/the/simulator/own/temp"),
                    ("CRAFTBRIDGE_NOT_A_RUNNER_VARIABLE", "stays"),
                ],
                &["TEST_RUNNER_HOME=/the/run/sandbox", "TEST_RUNNER_CFFIXED_USER_HOME=/the/run/sandbox", "TEST_RUNNER_TMPDIR=/the/run/sandbox/tmp/"],
            );
            assert_eq!(
                (code, held.as_str(), err.as_str()),
                (Some(0), "TEST_RUNNER_CFFIXED_USER_HOME=/the/run/sandbox\nTEST_RUNNER_HOME=/the/run/sandbox\nTEST_RUNNER_TMPDIR=/the/run/sandbox/tmp/", ""),
                "the run's own assignments are all the TEST_RUNNER_ and SIMCTL_CHILD_ environment holds"
            );
            assert!(
                all.lines().any(|line| line == "CRAFTBRIDGE_NOT_A_RUNNER_VARIABLE=stays"),
                "a variable without the prefix passes through as it was:\n{all}"
            );
            let (code, held, all, err) = started(
                &bash,
                "direct",
                &[("TEST_RUNNER_B3EQ", "a=b"), ("TEST_RUNNER_B3NL", "first\nsecond"), ("B3_PLAIN", "stays")],
                &["TEST_RUNNER_HOME=/run/a=b", "TEST_RUNNER_TMP=/run/x\ny"],
            );
            assert_eq!(
                (code, held.as_str(), err.as_str()),
                (Some(0), "TEST_RUNNER_HOME=/run/a=b\nTEST_RUNNER_TMP=/run/x", ""),
                "a name reads whole however its value spells itself: an inherited value with a newline or another = in it drops, and the run's own such value stands"
            );
            assert!(all.contains("TEST_RUNNER_TMP=/run/x\ny"), "the newline in the value passes into the environment whole:\n{all}");
            assert!(all.lines().any(|line| line == "B3_PLAIN=stays"), "a variable without the prefix passes through as it was:\n{all}");
            let (code, _, _, err) = started(&bash, "direct", &[], &["TEST_RUNNER_A B=x", "TEST_RUNNER_HOME=/ok"]);
            assert_eq!(
                (code, err.lines().last().unwrap_or("")),
                (Some(2), "bash: craftbridge_set_test_runner_env takes the run's TEST_RUNNER_ assignments, and TEST_RUNNER_A B=x is none"),
                "a word whose name the shell itself will not export is refused in the same words, even beside a good word after it"
            );
            for name in ["TEST_RUNNER_A-B", "SIMCTL_CHILD_C.D", "TEST_RUNNER_A!B", "SIMCTL_CHILD_A B"] {
                let (code, held, _, err) = started(&bash, "direct", &[(name, "1"), ("TEST_RUNNER_DROPPED", "1")], &["TEST_RUNNER_ONLY=one"]);
                assert_eq!(
                    (code, held.as_str(), err.lines().last().unwrap_or("")),
                    (
                        Some(2),
                        "",
                        format!(
                            "bash: craftbridge_set_test_runner_env cannot drop the inherited {name}, a name the shell will not unset, so it would reach the tests: run again without it"
                        )
                        .as_str()
                    ),
                    "an inherited variable whose name the shell will not unset stops the run"
                );
            }
            let (code, all, _, err) = started(&bash, "and", &[("TEST_RUNNER_A-B", "1"), ("TEST_RUNNER_DROPPED", "1")], &["TEST_RUNNER_ONLY=one"]);
            assert_eq!(
                (code, all.as_str(), err.lines().last().unwrap_or("")),
                (
                    Some(2),
                    "",
                    "bash: craftbridge_set_test_runner_env cannot drop the inherited TEST_RUNNER_A-B, a name the shell will not unset, so it would reach the tests: run again without it"
                ),
                "the stage's own shape — the call on the left of an `&&` — stops on the refusal with the same status and nothing after it running"
            );
            let (code, held, _, err) = started(&bash, "direct", &[("SIMCTL_CHILD_CRAFTBRIDGE_BASELINE", "1")], &["TEST_RUNNER_ONLY=one"]);
            assert_eq!(
                (code, held.as_str(), err.as_str()),
                (Some(0), "TEST_RUNNER_ONLY=one", ""),
                "a SIMCTL_CHILD_ variable inherited with no TEST_RUNNER_ one beside it"
            );
            let (code, held, _, err) = started(&bash, "direct", &[], &["TEST_RUNNER_ONLY=one"]);
            assert_eq!((code, held.as_str(), err.as_str()), (Some(0), "TEST_RUNNER_ONLY=one", ""), "a run of its own with nothing inherited");
            let (code, held, _, err) = started(&bash, "direct", &[], &[]);
            assert_eq!((code, held.as_str(), err.as_str()), (Some(0), "", ""), "a scrub with nothing to keep leaves none");
            assert_eq!(
                started(&bash, "direct", &[], &["PATH=/nowhere"]),
                (
                    Some(2),
                    String::new(),
                    String::new(),
                    "bash: craftbridge_set_test_runner_env takes the run's TEST_RUNNER_ assignments, and PATH=/nowhere is none".to_string()
                ),
                "a word that is no TEST_RUNNER_ assignment is refused"
            );
        }
    }

    /// The build and test stages refuse, before anything is built or any simulator is taken, a scheme or a
    /// test plan a developer's Xcode left where `xcodebuild` reads schemes: given `-scheme <name>` it runs
    /// the file `<name>.xcscheme` on disk — its test environment and any test plan it names included —
    /// instead of the scheme it generates from the tree, so under such a file the gate would answer for the
    /// machine rather than the tree. Only a file git does not track is refused, an ignored one as much as any:
    /// a scheme the tree commits is the tree's own, and passes. The refusal is status 2, and what follows
    /// the call never runs, whether the call stands alone under `set -e` or stands on the left of a list,
    /// where bash holds `set -e` off: the stages run it as `... || return 1`, and a case below runs it on
    /// the left of an `&&`. It names every file from the root on a line of its own, with what it is and a
    /// removal command that, pasted as printed, removes that file whatever its name holds; and it leaves
    /// every file as it found it, since removing a developer's own file is the developer's choice.
    /// Whatever else the folders of Xcode's state hold — the management files and folders Xcode writes
    /// beside its schemes — passes, and so does a folder that is not there; a git that cannot answer,
    /// outside a checkout, refuses, since the check could not tell a left file from the tree's. No
    /// simulator is asked for: what is judged is the disk.
    #[test]
    fn the_stages_refuse_the_schemes_and_test_plans_xcode_left_on_disk() {
        let env = crate::repo_root().join("tools/env.sh");
        let scheme = "pkg/.swiftpm/xcode/xcshareddata/xcschemes/CraftBridge-Package.xcscheme";
        let plan = "pkg/.swiftpm/xcode/xcshareddata/xcschemes/OneTest.xctestplan";
        let user_scheme = "app/PostcardStudio.xcodeproj/project.xcworkspace/xcuserdata/dev.xcuserdatad/xcschemes/PostcardStudio.xcscheme";
        let shared_scheme = "app/PostcardStudio.xcodeproj/xcshareddata/xcschemes/PostcardStudio.xcscheme";
        let odd_plan = "app/PostcardStudio.xcodeproj/xcshareddata/xcschemes/it's a \"plan\".xctestplan";
        let reason = "xcodebuild reads the schemes kept there in place of the ones it generates from the tree, and runs the test plans they name, so a run would answer for what this machine's Xcode left there, not for the tree";
        // The tree a run judges: a checkout that ignores the package's state folder, as the repository does,
        // and commits `tracked`; `left` is on disk beside it, never added. `checkout` false leaves git out.
        let started = |bash: &std::path::Path, shape: &str, checkout: bool, tracked: &[&str], left: &[&str], folders: &[&str]| {
            let script = match shape {
                // The stages call the helper as `... || return 1`; this shape calls it on the left of an
                // `&&`. Either stands on the left of a list, where bash holds its `set -e` off: a refusal
                // must skip what runs after it, and the call's own status must stand.
                "and" => "set -euo pipefail\nsource \"$1\"\nshift\ncraftbridge_refuse_leftover_schemes_and_test_plans \"$@\" && echo ran after\nexit $?",
                _ => "set -euo pipefail\nsource \"$1\"\nshift\ncraftbridge_refuse_leftover_schemes_and_test_plans \"$@\"\necho ran after",
            };
            let scratch = crate::scratch::Scratch::new("leftover-schemes");
            if checkout {
                crate::git::output(&scratch.0, &["init", "-q"]).unwrap();
                scratch.write(".gitignore", "/pkg/.swiftpm/\n");
            }
            for path in tracked.iter().chain(left) {
                scratch.write(path, "planted by the test, untouched by the refusal\n");
            }
            if !tracked.is_empty() {
                let mut add = vec!["add", "--"];
                add.extend(tracked);
                crate::git::output(&scratch.0, &add).unwrap();
            }
            let mut command = std::process::Command::new(bash);
            command.arg("-c").arg(script).arg("bash").arg(&env).args(folders).current_dir(&scratch.0);
            // As the gate runs it: tools/ci.sh starts with git's repository variables removed, so a run from
            // a hook asks the checkout it is in, not the repository the hook was started for.
            for variable in crate::git::local_env_vars() {
                command.env_remove(variable);
            }
            let output = command.output().unwrap();
            let kept: Vec<bool> = tracked.iter().chain(left).map(|path| scratch.0.join(path).exists()).collect();
            (
                output.status.code(),
                String::from_utf8_lossy(&output.stdout).trim_end().to_string(),
                String::from_utf8_lossy(&output.stderr).trim_end().to_string(),
                kept,
                scratch,
            )
        };
        for bash in the_bashes_the_gate_runs_under() {
            let run = |shape: &str, tracked: &[&str], left: &[&str], folders: &[&str]| {
                let (code, out, err, kept, _) = started(&bash, shape, true, tracked, left, folders);
                (code, out, err, kept)
            };
            let (code, out, err, kept) = run("direct", &[], &[scheme], &["pkg/.swiftpm"]);
            assert_eq!((code, out.as_str()), (Some(2), ""), "a scheme left in the package's ignored state folder is refused, and nothing runs after it");
            assert_eq!(
                err,
                format!(
                    "bash: craftbridge_refuse_leftover_schemes_and_test_plans refuses the scheme {scheme}, which git does not track: {reason}\nbash:   remove it and run again: rm -f -- {scheme}"
                ),
                "the refusal names the file from the root, what it is, and the command that removes it"
            );
            assert_eq!(kept, vec![true], "the developer's own file is left exactly as it was found");
            let (code, _out, err, kept) = run("direct", &[], &[plan], &["pkg/.swiftpm"]);
            assert_eq!(code, Some(2), "a test plan left beside the schemes is refused too: {err}");
            assert!(
                err.starts_with(&format!("bash: craftbridge_refuse_leftover_schemes_and_test_plans refuses the test plan {plan}, which git does not track:")),
                "the refusal says which it is: {err}"
            );
            assert_eq!(kept, vec![true], "the developer's own file is left exactly as it was found");
            let (code, out, err, kept) = run("direct", &[], &[scheme, plan, user_scheme], &["pkg/.swiftpm", "app/PostcardStudio.xcodeproj"]);
            assert_eq!((code, out.as_str()), (Some(2), ""), "the stages name the sample projects beside the package, and one refusal lists them all");
            for named in [scheme, plan, user_scheme] {
                assert!(
                    err.contains(&format!(
                        "refuses the {} {named}, which git does not track:",
                        if named.ends_with(".xcscheme") { "scheme" } else { "test plan" }
                    )),
                    "{named} is not named: {err}"
                );
            }
            assert_eq!(err.lines().count(), 6, "each file is named on its own line beside its removal line: {err}");
            assert_eq!(kept, vec![true, true, true], "no file of any folder is touched");
            let (code, out, _err, _) = run("and", &[], &[scheme], &["pkg/.swiftpm"]);
            assert_eq!(
                (code, out.as_str()),
                (Some(2), ""),
                "a call on the left of an `&&`, where bash holds `set -e` off as it does in the stages' `... || return 1`, stops on the refusal with its own status and nothing after it running"
            );
            let (code, out, err, kept) = run("direct", &[shared_scheme], &[], &["pkg/.swiftpm", "app/PostcardStudio.xcodeproj"]);
            assert_eq!(
                (code, out.as_str(), err.as_str(), kept),
                (Some(0), "ran after", "", vec![true]),
                "a shared scheme the tree commits is the tree's own, and passes"
            );
            let (code, _out, err, kept) = run("direct", &[shared_scheme], &[user_scheme], &["app/PostcardStudio.xcodeproj"]);
            assert_eq!(code, Some(2), "a user's scheme left beside a committed one is still refused: {err}");
            assert!(err.contains(user_scheme) && !err.contains(shared_scheme), "only the file git does not track is named: {err}");
            assert_eq!(kept, vec![true, true], "neither file is touched");
            let (code, _out, err, kept, scratch) = started(&bash, "direct", true, &[], &[odd_plan], &["app/PostcardStudio.xcodeproj"]);
            assert_eq!((code, kept), (Some(2), vec![true]), "a name holding a space and quotes is refused like any other: {err}");
            let removal = err.lines().last().and_then(|line| line.strip_prefix("bash:   remove it and run again: ")).unwrap_or_default().to_string();
            let pasted = std::process::Command::new(&bash).arg("-c").arg(&removal).current_dir(&scratch.0).output().unwrap();
            assert!(
                pasted.status.success() && !scratch.0.join(odd_plan).exists() && scratch.0.join("app/PostcardStudio.xcodeproj/xcshareddata/xcschemes").is_dir(),
                "the removal line, pasted as printed, removes exactly that file: {removal}"
            );
            let (code, out, err, _) =
                run("direct", &[], &["pkg/.swiftpm/xcode/xcschememanagement.plist", "pkg/.swiftpm/configuration/stub"], &["pkg/.swiftpm"]);
            assert_eq!(
                (code, out.as_str(), err.as_str()),
                (Some(0), "ran after", ""),
                "the rest of what Xcode leaves beside its schemes is nobody's business: only a scheme or a test plan refuses"
            );
            let (code, out, err, _) = run("direct", &[], &[], &["pkg/.swiftpm"]);
            assert_eq!((code, out.as_str(), err.as_str()), (Some(0), "ran after", ""), "the tree with none of them left passes, state folder or not");
            let (code, out, err, _, _) = started(&bash, "direct", false, &[], &[], &["pkg/.swiftpm"]);
            assert_eq!(
                (code, out.as_str(), err.lines().last().unwrap_or("")),
                (
                    Some(2),
                    "",
                    "bash: craftbridge_refuse_leftover_schemes_and_test_plans cannot ask git which files it tracks under pkg/.swiftpm, so it cannot tell a scheme Xcode left there from one the tree commits"
                ),
                "outside a checkout git cannot answer, and the check refuses rather than pass what it could not read"
            );
        }
    }
}

/// The one reading of the root, proved: the folder found from the run, its checks, and the refusal of
/// a binary that belongs to one tree but was handed another.
#[cfg(test)]
mod repository {
    use super::*;
    use crate::scratch::Scratch;
    use std::ffi::OsString;

    /// A scratch folder under `inside` holding every marker file: a CraftBridge tree as far as these
    /// checks go.
    fn tree_in(inside: &Path, tag: &str) -> Scratch {
        let scratch = Scratch(crate::scratch::create_in(inside, tag).unwrap());
        for marker in REPOSITORY_MARKERS {
            scratch.write(marker, "x\n");
        }
        scratch
    }

    fn tree(tag: &str) -> Scratch {
        tree_in(&std::env::temp_dir(), tag)
    }

    /// A tree that is a git checkout of its own, as a copy of a checkout is to git.
    fn checkout(tag: &str) -> Scratch {
        let checkout = tree(tag);
        crate::git::output(&checkout.0, &["init", "-q"]).unwrap();
        checkout
    }

    /// The refusal of a binary built in `built` and run for the tree `copy`, in its exact words.
    fn refusal(built: &Path, copy: &Path) -> String {
        format!(
            "this binary was built in `{}` but was run for `{}`; a copy of a tree that kept another tree's build folder runs that other tree's xtask: build in the tree to work on, or run that tree's own tools/ scripts",
            built.display(),
            resolved(copy).unwrap().display()
        )
    }

    /// A run in its own tree passes the whole check: the tree found from the run is the tree the
    /// binary was built in, whichever hand named it — a `tools/` script's or the working folder's.
    #[test]
    fn this_binary_finds_the_tree_it_was_built_in() {
        assert_eq!(repo_root(), built_root().unwrap());
    }

    /// The variable xtask reads is the one every `tools/` script exports, spelt once on each side.
    #[test]
    fn the_variable_read_is_the_one_tools_env_sh_exports() {
        let env = std::fs::read_to_string(repo_root().join("tools/env.sh")).unwrap();
        assert!(env.lines().any(|line| line == format!("export {NAMED_TREE}")), "tools/env.sh exports no {NAMED_TREE}");
    }

    /// A binary built here and run in a copy refuses, whether a `tools/` script of the copy named the copy
    /// or the copy's own checkout answers git, and its words name both trees: the tree it was built in,
    /// which it would otherwise rewrite, and the copy it was run for.
    #[test]
    fn a_binary_run_in_a_copy_is_refused_naming_both_trees() {
        let built = built_root().unwrap();
        let named = tree("root-named-copy");
        assert_eq!(root_of_run(&built, &named.0.join("rust"), Some(&named.0)).unwrap_err(), refusal(&built, &named.0));
        let checkout = checkout("root-git-copy");
        assert_eq!(root_of_run(&built, &checkout.0.join("tools"), None).unwrap_err(), refusal(&built, &checkout.0));
    }

    /// The variable a `tools/` script sets outlives the script, so it counts only for a run standing
    /// inside the tree it names. A run in a copy that inherited the original's variable is refused as the
    /// copy: taking the variable there would hand the original's stale binary its own tree to write. A run
    /// in its own tree works on it, whatever tree an inherited variable names.
    #[test]
    fn a_tree_a_tools_script_named_counts_only_for_a_run_inside_it() {
        let built = built_root().unwrap();
        let checkout = checkout("root-inherited");
        assert_eq!(root_of_run(&built, &checkout.0, Some(&built)).unwrap_err(), refusal(&built, &checkout.0));
        let plain = tree("root-inherited-plain");
        assert_eq!(
            root_of_run(&built, &plain.0, Some(&built)).unwrap_err(),
            format!(
                "no tree to work on is named: `{}` stands outside `{}`, the tree CRAFTBRIDGE_ROOT names, and inside no git checkout; run xtask from inside a CraftBridge tree or through its tools/ scripts",
                plain.0.display(),
                built.display()
            )
        );
        assert_eq!(root_of_run(&built, &built.join("rust/xtask"), Some(&checkout.0)).unwrap(), built);
    }

    /// The tree a `tools/` script names, and its run stands in, passes, whether or not git knows it and
    /// whatever path the script reached it by: this is every gate run, every doctor run, every tools
    /// script. The tree is named as the build names it.
    #[test]
    fn the_tree_a_tools_script_names_and_stands_in_passes() {
        let built = built_root().unwrap();
        assert_eq!(root_of_run(&built, &built.join("rust/xtask"), Some(&built)).unwrap(), built);
        let unknown_to_git = tree("root-unknown-to-git");
        let own = resolved(&unknown_to_git.0).unwrap();
        let parent = Scratch::new("root-named-through-a-link");
        let link = parent.0.join("tree");
        std::os::unix::fs::symlink(&unknown_to_git.0, &link).unwrap();
        for named in [unknown_to_git.0.as_path(), link.as_path()] {
            assert_eq!(root_of_run(&own, &own.join("tools"), Some(named)).unwrap(), own, "{}", named.display());
        }
        assert_eq!(
            root_of_run(&own, &own, None).unwrap_err(),
            format!(
                "no tree to work on is named: no tools script set CRAFTBRIDGE_ROOT and `{}` stands inside no git checkout; run xtask from inside a CraftBridge tree or through its tools/ scripts",
                own.display()
            ),
            "by hand, a tree git does not know is named by nothing"
        );
    }

    /// Every path to the tree is the tree, and a copy is never it: through a link, which resolving the
    /// path also settles, and in another case where the disk folds case.
    #[test]
    fn a_tree_reached_through_a_link_or_in_another_case_is_the_tree_itself_and_a_copy_is_not() {
        let real = tree("root-real");
        let parent = Scratch::new("root-link");
        let link = parent.0.join("worktree");
        std::os::unix::fs::symlink(&real.0, &link).unwrap();
        assert!(same_folder(&real.0, &link));
        assert_eq!(craftbridge_tree(&link, "the hand").unwrap(), resolved(&real.0).unwrap());
        assert!(!same_folder(&real.0, &tree("root-copy").0));
        let built = built_root().unwrap();
        let shouted = PathBuf::from(built.to_string_lossy().to_uppercase());
        if shouted.exists() {
            assert!(same_folder(&built, &shouted), "{} is {} on a disk that folds case", shouted.display(), built.display());
        }
    }

    /// On macOS the data volume's own path to a folder, behind a firmlink, is the folder itself, though
    /// resolving the path leaves it as spelt: a `tools/` script started through it names its own tree.
    #[test]
    #[cfg(target_os = "macos")]
    fn a_tree_reached_through_a_firmlink_is_the_tree_itself() {
        let real = tree_in(Path::new("/private/tmp"), "root-firmlink");
        let own = resolved(&real.0).unwrap();
        let firmlinked = Path::new("/System/Volumes/Data").join(own.strip_prefix("/").unwrap());
        assert!(same_folder(&own, &firmlinked), "{} is {} through the data volume's firmlink", firmlinked.display(), own.display());
        assert_eq!(root_of_run(&own, &own.join("rust"), Some(&firmlinked)).unwrap(), own);
    }

    /// A command is refused before it runs, any command, even one that reads no tree: `dispatch` takes
    /// its tree from the one reading, never from where the binary was built. The child runs a command in
    /// its own tree, where it passes, and the same command in a copy whose tools script named it.
    #[test]
    fn a_command_run_in_a_copy_is_refused_before_it_runs() {
        let copy = tree("root-command");
        craftbridge_process::rerun(
            "repository::child_runs_a_command_here_and_in_the_copy",
            &[(NAMED_TREE, copy.0.as_os_str())],
            "the command ran in the copy, or not in its own tree",
        )
        .unwrap();
    }

    /// The child half of the test above: alone in a process of its own, it may move its working folder.
    #[test]
    #[ignore = "run by a_command_run_in_a_copy_is_refused_before_it_runs, with a copy's tools script's variable"]
    fn child_runs_a_command_here_and_in_the_copy() {
        let copy = PathBuf::from(std::env::var_os(NAMED_TREE).unwrap());
        let log = Scratch::new("root-command-log");
        log.write("gradle.log", "> Task :core:compileKotlin\n");
        let skipped_tasks = || dispatch([OsString::from("skipped-tasks"), log.0.join("gradle.log").into_os_string()].into_iter());
        assert_eq!(skipped_tasks(), ExitCode::SUCCESS, "standing in its own tree, under a variable naming another, the binary runs the command");
        std::env::set_current_dir(&copy).unwrap();
        assert_eq!(
            skipped_tasks(),
            ExitCode::from(MISUSE),
            "standing in the copy its tools script named, the binary must refuse the command with the exit of a run that cannot proceed"
        );
    }

    /// The top level is the checkout the folder stands in, found through `crate::git` — whose removal
    /// of git's repository variables is what keeps a hook's git folder from naming it — and nothing is
    /// found where no checkout stands. A worktree's top level is the worktree, and a plain copy of it
    /// answers git with itself: carrying its own `.git` file, it is a tree of its own to git wherever
    /// it stands, which is what makes a copy differ from the tree the copied binary was built for —
    /// and so be refused.
    #[test]
    fn the_git_top_level_names_the_checkout_a_folder_stands_in() {
        let checkout = checkout("root-git");
        let nested = checkout.0.join("rust/xtask");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(git_top_level(&nested).unwrap(), resolved(&checkout.0).unwrap());
        assert_eq!(git_top_level(&Scratch::new("root-nogit").0), None);
        crate::git::output(&checkout.0, &["add", "-A"]).unwrap();
        crate::git::output(&checkout.0, &["-c", "user.name=xtask", "-c", "user.email=xtask@example.invalid", "commit", "-q", "-m", "base"]).unwrap();
        let worktree = checkout.0.join("wt");
        crate::git::output(&checkout.0, &["worktree", "add", "-q", worktree.to_str().unwrap(), "-b", "wt"]).unwrap();
        assert_eq!(git_top_level(&worktree).unwrap(), resolved(&worktree).unwrap());
        let copy = Scratch::new("root-wtcopy");
        crate::scratch::copy_tree(&worktree, &copy.0);
        assert_eq!(git_top_level(&copy.0).unwrap(), resolved(&copy.0).unwrap());
    }

    /// A folder is refused as what it lacks, so the fix says what the tree is missing.
    #[test]
    fn a_folder_that_holds_nothing_of_a_tree_is_named_by_what_it_lacks() {
        let scratch = Scratch::new("root-plain");
        let (held, lacked) = REPOSITORY_MARKERS.split_first().unwrap();
        scratch.write(held, "x\n");
        let error = craftbridge_tree(&scratch.0, "the hand").unwrap_err();
        assert_eq!(error, format!("the hand `{}` holds no CraftBridge tree (no {})", scratch.0.display(), lacked.join(", no ")));
        let ok = tree("root-ok");
        assert_eq!(craftbridge_tree(&ok.0, "the hand").unwrap(), resolved(&ok.0).unwrap());
    }
}
