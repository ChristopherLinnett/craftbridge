//! CraftBridge's rule checks and regeneration: `cargo xtask rules` and `cargo xtask generate`; the plan's
//! views, `cargo xtask plan-views --check`, rebuilt from their sources and compared; the judgements
//! `tools/ci.sh` hands over from the stages it runs, `cargo xtask skipped-tasks <gradle log>`,
//! `cargo xtask compiled`, and the lint library's run under its lock, `cargo xtask dylint …`; the file
//! lists its stages read, `cargo xtask files <kind>`; and the pins the doctor and the gate read,
//! `cargo xtask pins` and `cargo xtask swift-builds`.
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
mod buildfiles;
mod compiled;
mod direction;
mod duplicates;
mod effects_api;
mod filelock;
mod files;
mod generate;
mod generated;
mod git;
mod gradle_log;
mod hygiene;
mod layers;
mod layout;
mod licences;
mod lints;
mod machinery;
mod meta;
mod pins;
mod plan_views;
mod prepush;
mod report;
mod rust_syntax;
mod scan;
mod scratch;
mod sources;
mod swift_lint;
mod unsafe_check;
mod update;
mod upstream;
mod versions;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Repository root: this crate is at `<R>/rust/xtask`, so the root is two parents up. Derived from
/// the manifest directory instead of the working directory, because the checks must find the same
/// files whether invoked by hand, by `tools/ci.sh` or by a test.
fn repo_root() -> PathBuf {
    match Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent) {
        Some(root) => root.to_path_buf(),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    }
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
    let root = repo_root();
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
        ["swift-lint"] => swift_lint::run(&root),
        ["pins"] => pins::run(&root),
        ["swift-builds"] => pins::builds(&root),
        _ => {
            usage();
            ExitCode::from(MISUSE)
        }
    }
}

/// The exit of a misused command, sysexits' `EX_USAGE`: 1 is a check or step that failed, and 2 is the
/// upstream update's breaking API change, so a misuse must be neither.
const MISUSE: u8 = 64;

fn usage() {
    eprintln!("usage: cargo xtask rules");
    eprintln!("       cargo xtask generate [--check]");
    eprintln!("       cargo xtask plan-views --check");
    eprintln!("       cargo xtask skipped-tasks <gradle log>");
    eprintln!("       cargo xtask files <swift|shell|duplicates>");
    eprintln!("       cargo xtask upstream update <rev>   (through tools/update-upstream.sh: plan \u{a7}24 to <rev>, a commit of official EffectCraft main)");
    eprintln!("       cargo xtask upstream source   (the official URL and the full commit the pin names on official main, for tools/oracle.sh)");
    eprintln!("       cargo <check|clippy|test> --message-format=json … | cargo xtask compiled");
    eprintln!("       cargo xtask dylint <cargo-dylint arguments…>   (cargo dylint, taking turns on the lock every lint library run shares)");
    eprintln!("       cargo xtask swift-lint");
    eprintln!("       cargo xtask pins");
    eprintln!("       cargo xtask swift-builds")
}

/// Run every check, print one line per failure and `ok` per clean check, and fail if any failed.
mod rules {
    use crate::report;
    use std::path::Path;
    use std::process::ExitCode;

    /// One check: its name, and what it reads from the repository to return its errors.
    type Check = (&'static str, fn(&Path) -> Result<Vec<String>, String>);

    pub(super) fn run(root: &Path) -> ExitCode {
        let checks: [Check; 13] = [
            ("hygiene", crate::hygiene::check),
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
}
