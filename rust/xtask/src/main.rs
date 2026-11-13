//! CraftBridge's rule checks and regeneration: `cargo xtask rules` and `cargo xtask generate`.
//!
//! The rules in `AGENTS.md` that no compiler, formatter or linter already enforces are checked here,
//! behind one command, so a change cannot merge with one broken. Each check lives in its own module: a
//! `check` that reads the repository and hands what it read to pure functions, which the module's tests
//! feed with input built in memory.
//!
//! Checks run even when an earlier one fails: one run reports everything that is broken, because a
//! half-fixed change still gates.

mod attributes;
mod buildfiles;
mod direction;
mod duplicates;
mod generate;
mod generated;
mod layers;
mod layout;
mod licences;
mod lints;
mod machinery;
mod meta;
mod report;
mod scan;
mod scratch;
mod sources;
mod unsafe_check;

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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = repo_root();
    // Every argument is matched whole: a misspelt `--check` must not quietly become a run that writes.
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["rules"] => rules::run(&root),
        ["generate"] => generate::run(&root, false),
        ["generate", "--check"] => generate::run(&root, true),
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}

fn usage() {
    eprintln!("usage: cargo xtask rules");
    eprintln!("       cargo xtask generate [--check]");
}

/// Run every check, print one line per failure and `ok` per clean check, and fail if any failed.
mod rules {
    use crate::report;
    use std::path::Path;
    use std::process::ExitCode;

    pub(super) fn run(root: &Path) -> ExitCode {
        // Each check reads what it needs from the repository and returns its errors.
        let checks: [(&str, fn(&Path) -> Result<Vec<String>, String>); 11] = [
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
        ];
        let mut failed = false;
        for (name, run_check) in checks {
            match run_check(root) {
                Ok(errors) if errors.is_empty() => report::ok(name),
                Ok(errors) => {
                    failed = true;
                    for error in errors {
                        report::error(name, error);
                    }
                }
                Err(failure) => {
                    failed = true;
                    report::error(name, format!("could not run this check: {failure}"));
                }
            }
        }
        if failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
    }
}
