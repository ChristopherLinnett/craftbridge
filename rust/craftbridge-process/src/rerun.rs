//! Run one test of the current test binary again, in a child process started with a changed environment.
//!
//! A core crate may not reach a host tool even in its tests (AGENTS.md §2), so the helper lives here,
//! the one crate the tools' tests and the core's tests may both take — once, so a fix reaches both.

use crate::command::output_of;
use std::ffi::OsStr;
use std::fmt;
use std::process::ExitStatus;

/// Why a rerun did not pass: the caller's `failure`, or the refusal of a name that ran no test, then both
/// of the child's streams, and the status the child ended with where one ran. A test unwraps it, and `unwrap`
/// shows an error by its `Debug`, which for a plain string escapes every line break and quote and folds the
/// child's streams, a panic message or an `assert_eq!`'s two sides, into one line. So its `Debug`, like its
/// `Display`, is the text as written.
pub struct RerunError {
    text: String,
    status: Option<ExitStatus>,
}

impl RerunError {
    fn before_the_child(text: String) -> Self {
        Self { text, status: None }
    }

    /// The status the child ended with, or `None` where none was started: a test that weighs how a process
    /// of tests ends, not only that its test failed, reads it here.
    pub fn status(&self) -> Option<ExitStatus> {
        self.status
    }
}

impl fmt::Display for RerunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
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
    rerun_unless_said(test, env, failure, None)
}

/// [`rerun`], and an answer of `failure` too when either of the child's streams holds `refused`: a process
/// can say the platform aborted it, as bionic does for a mutex locked after its destruction, after its
/// own exit has set a status of 0, so the status alone passes a child that was taken down.
pub fn rerun_refusing(test: &str, env: &[(&str, &OsStr)], failure: &str, refused: &str) -> Result<(), RerunError> {
    rerun_unless_said(test, env, failure, Some(refused))
}

fn rerun_unless_said(test: &str, env: &[(&str, &OsStr)], failure: &str, refused: Option<&str>) -> Result<(), RerunError> {
    let exe = std::env::current_exe().map_err(|error| RerunError::before_the_child(format!("could not find the current test binary: {error}")))?;
    let mut child = std::process::Command::new(exe);
    child.args(["--exact", test, "--ignored", "--nocapture", "--test-threads=1"]);
    for (name, value) in env {
        child.env(name, value);
    }
    let output = output_of(&mut child).map_err(RerunError::before_the_child)?;
    let ran = String::from_utf8_lossy(&output.stdout);
    let said = String::from_utf8_lossy(&output.stderr);
    // The harness prints its progress to stdout, while a failed test writes its panic message to stderr,
    // and a child that cannot start its test says why on stderr alone: either stream alone can leave the
    // cause out.
    let answer = |text: String| RerunError { text: format!("{text}\nthe child's stdout:\n{ran}\nthe child's stderr:\n{said}"), status: Some(output.status) };
    // A rename can leave this name matching no test, and a run of no tests exits zero: the child's status
    // would then pass having weighed nothing. The name must still name the one ignored child this helper
    // exists to rerun.
    if !ran.contains("running 1 test") {
        return Err(answer(format!("rerun named no test: {test}")));
    }
    let refused_said = refused.is_some_and(|text| ran.contains(text) || said.contains(text));
    if output.status.success() && !refused_said { Ok(()) } else { Err(answer(failure.to_string())) }
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

    /// The answer carries the status the child ended with: a failed test's is the harness's own failure status,
    /// one that passed but said refused words is 0, and a name that ran no test still ran a child.
    #[test]
    fn a_child_that_did_not_pass_answers_the_status_it_ended_with() {
        let failed = rerun("rerun::tests::child_that_fails_on_purpose", &[], "never").unwrap_err();
        assert_eq!(failed.status().and_then(|status| status.code()), Some(101), "{failed}");
        let said = rerun_refusing("rerun::tests::child_that_says_it_on_stdout", &[], "never", "refused words").unwrap_err();
        assert_eq!(said.status().and_then(|status| status.code()), Some(0), "{said}");
        let none = rerun("rerun::tests::no_test_bears_this_name", &[], "never").unwrap_err();
        assert_eq!(none.status().and_then(|status| status.code()), Some(0), "{none}");
    }

    /// A child that exits with status 0 but said what the caller refuses fails, as one that said it on either
    /// stream, and one that said nothing of it passes.
    #[test]
    fn a_child_that_says_what_is_refused_fails_whatever_its_status() {
        for stream in ["stdout", "stderr"] {
            let problem = rerun_refusing(&format!("rerun::tests::child_that_says_it_on_{stream}"), &[], "the child said it", "refused words").unwrap_err();
            assert!(problem.to_string().starts_with("the child said it\nthe child's stdout:\n"), "{stream}: {problem}");
        }
        rerun_refusing("rerun::tests::child_reads_the_planted_variable", &[("CRAFTBRIDGE_RERUN_PROBE", OsStr::new("planted"))], "never", "refused words")
            .unwrap();
    }

    #[test]
    #[ignore = "run by a_child_that_says_what_is_refused_fails_whatever_its_status, which refuses the words"]
    fn child_that_says_it_on_stdout() {
        println!("some refused words");
    }

    #[test]
    #[ignore = "run by a_child_that_says_what_is_refused_fails_whatever_its_status, which refuses the words"]
    fn child_that_says_it_on_stderr() {
        eprintln!("some refused words");
    }

    #[test]
    #[ignore = "run by a_failing_child_answers_the_failure_and_both_streams, which weighs its two streams"]
    fn child_that_fails_on_purpose() {
        println!("said to stdout \"quoted\"");
        eprintln!("said to stderr");
        panic!("the child fails on purpose");
    }
}
