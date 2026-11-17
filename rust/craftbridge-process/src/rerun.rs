//! Run one test of the current test binary again, in a child process started with a changed environment.
//!
//! A core crate may not reach a host tool even in its tests (AGENTS.md §2), so the helper lives here,
//! the one crate the tools' tests and the core's tests may both take — once, so a fix reaches both.

use crate::command::output_of;
use std::ffi::OsStr;
use std::fmt;

/// Why a rerun did not pass: the caller's `failure`, or the refusal of a name that ran no test, then both
/// of the child's streams. A test unwraps it, and `unwrap` shows an error by its `Debug`, which for a
/// plain string escapes every line break and quote and folds the child's streams, a panic message or an
/// `assert_eq!`'s two sides, into one line. So its `Debug`, like its `Display`, is the text as written.
pub struct RerunError(String);

impl fmt::Display for RerunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl fmt::Debug for RerunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for RerunError {}

/// Run one test of the current test binary again, in a child process with `env` added, and answer a
/// [`RerunError`] naming `failure` unless the child ran that test and it passed. A test cannot set
/// variables on its own process while other tests run beside it (git's, which a hook starts the gate
/// with, or one a library reads as it starts), and a child is the only way the code under test meets them
/// as it would where they are set. The child test is `#[ignore]`d, so it runs only when started this way.
/// The check is made here, so no caller can weigh the child by less than both of its streams; a caller
/// that only wants the test failed unwraps the answer, as test code may, and the failure names the
/// caller's line.
pub fn rerun(test: &str, env: &[(&str, &OsStr)], failure: &str) -> Result<(), RerunError> {
    let exe = std::env::current_exe().map_err(|error| RerunError(format!("could not find the current test binary: {error}")))?;
    let mut child = std::process::Command::new(exe);
    child.args(["--exact", test, "--ignored", "--nocapture", "--test-threads=1"]);
    for (name, value) in env {
        child.env(name, value);
    }
    let output = output_of(&mut child).map_err(RerunError)?;
    let ran = String::from_utf8_lossy(&output.stdout);
    // The harness prints its progress to stdout, while a failed test writes its panic message to stderr,
    // and a child that cannot start its test says why on stderr alone: either stream alone can leave the
    // cause out.
    let streams = format!("the child's stdout:\n{ran}\nthe child's stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    // A rename can leave this name matching no test, and a run of no tests exits zero: the child's status
    // would then pass having weighed nothing. The name must still name the one ignored child this helper
    // exists to rerun.
    if !ran.contains("running 1 test") {
        return Err(RerunError(format!("rerun named no test: {test}\n{streams}")));
    }
    if output.status.success() { Ok(()) } else { Err(RerunError(format!("{failure}\n{streams}"))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The child runs the named ignored test of this very binary and sees the variables exactly as the
    /// caller set them.
    #[test]
    fn the_child_runs_and_carries_the_environment() {
        rerun(
            "rerun::tests::child_reads_the_planted_variable",
            &[("CRAFTBRIDGE_RERUN_PROBE", OsStr::new("planted"))],
            "the child did not read the variable it was started with",
        )
        .unwrap();
    }

    #[test]
    #[ignore = "run by the_child_runs_and_carries_the_environment, which starts it with the variable set"]
    fn child_reads_the_planted_variable() {
        assert_eq!(std::env::var_os("CRAFTBRIDGE_RERUN_PROBE").as_deref(), Some(OsStr::new("planted")));
    }

    /// A name that names no test is refused by that name, with both streams: a run that selects no test
    /// exits zero, so the status alone would pass having weighed nothing.
    #[test]
    fn a_name_naming_no_test_is_refused_naming_it() {
        let problem = rerun("rerun::tests::no_test_bears_this_name", &[], "a run of no test must not pass").unwrap_err().to_string();
        assert!(problem.starts_with("rerun named no test: rerun::tests::no_test_bears_this_name\nthe child's stdout:"), "{problem}");
    }

    /// A child whose test fails answers `failure` first, then both streams, so the caller's failure
    /// names what it asked the child for and everything the child said. The answer is read as `unwrap`
    /// shows it, by its `Debug`: each line as the child wrote it, never escaped into one.
    #[test]
    fn a_failing_child_answers_the_failure_and_both_streams() {
        let problem = rerun("rerun::tests::child_that_fails_on_purpose", &[], "the failing child did not fail").unwrap_err();
        let shown = format!("{problem:?}");
        assert_eq!(shown, problem.to_string());
        assert!(shown.starts_with("the failing child did not fail\nthe child's stdout:\n"), "{shown}");
        assert!(shown.contains("said to stdout \"quoted\"\n"), "{shown}");
        assert!(shown.contains("\nthe child's stderr:\nsaid to stderr\n"), "{shown}");
    }

    #[test]
    #[ignore = "run by a_failing_child_answers_the_failure_and_both_streams, which weighs its two streams"]
    fn child_that_fails_on_purpose() {
        println!("said to stdout \"quoted\"");
        eprintln!("said to stderr");
        panic!("the child fails on purpose");
    }
}
