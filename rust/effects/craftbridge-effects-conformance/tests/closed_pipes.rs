//! The runner is a command-line tool whose output is piped: a reader that closes either pipe early must
//! not crash it (AGENTS.md §3). Run as a process, because what is held here is how its two output macros
//! behave, and clippy does not look inside a macro the crate defines.

use std::process::{Command, ExitStatus, Stdio};

/// The runner, run with `args` and one of its two outputs a pipe whose read end is already closed, so the
/// first line it writes there fails with a broken pipe on any machine, however slow.
fn run_into_a_closed_pipe(args: &[&str], stderr: bool) -> std::io::Result<ExitStatus> {
    let (reader, writer) = std::io::pipe()?;
    drop(reader);
    let mut runner = Command::new(env!("CARGO_BIN_EXE_craftbridge-effects-conformance"));
    runner.args(args);
    if stderr {
        runner.stdout(Stdio::null()).stderr(writer);
    } else {
        runner.stdout(writer);
    }
    runner.status()
}

#[test]
fn a_reader_that_closed_stdout_before_the_runner_writes_does_not_crash_it() {
    let status = run_into_a_closed_pipe(&["--list"], false).expect("run the runner");
    assert!(status.success(), "the runner crashed writing to a closed stdout: {status}");
}

#[test]
fn a_reader_that_closed_stderr_before_the_runner_complains_does_not_crash_it() {
    // A bad command line is answered on stderr, with exit status 2; a crash would end with 101.
    let status = run_into_a_closed_pipe(&["--no-such-flag"], true).expect("run the runner");
    assert_eq!(status.code(), Some(2), "the runner crashed writing to a closed stderr: {status}");
}
