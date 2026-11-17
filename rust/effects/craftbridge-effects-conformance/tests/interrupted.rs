//! A runner that ends while a step runs leaves nothing of the step's process group running: interrupted
//! from its terminal, whose Ctrl-C reaches the runner's group and not the step's, or killed outright, which
//! no handler of the runner's would see. Run as a process, because what is held is what happens after the
//! runner itself has gone.

use craftbridge_process::{group_has_live_process, poll_until, signal_group};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Longest the step may take to start: a freshly written script can wait on the system's first-run scan
/// for many seconds, so this is the bound the runner gives a step itself.
const STARTS_WITHIN: Duration = Duration::from_secs(300);

/// Longest the step's group may outlive the runner: its guard kills the group as the runner ends.
const GONE_WITHIN: Duration = Duration::from_secs(60);

#[test]
fn a_runner_that_ends_while_a_step_runs_leaves_nothing_of_the_step_running() {
    // Ctrl-C at a terminal signals the runner's whole group; a kill outright reaches the runner alone.
    for (signal, to_the_group) in [("INT", true), ("KILL", false)] {
        let scratch = tempfile::tempdir().expect("scratch");
        let group_file = scratch.path().join("group");
        let oracle = scratch.path().join("oracle.sh");
        // The step's shell waits on its `sleep`, and both outlast every bound here.
        std::fs::write(&oracle, format!("#!/bin/sh\nps -o pgid= -p $$ > '{}'\nsleep 600; true\n", group_file.display())).expect("write the oracle");
        std::fs::set_permissions(&oracle, std::fs::Permissions::from_mode(0o755)).expect("make it executable");
        let mut runner = Command::new(env!("CARGO_BIN_EXE_craftbridge-effects-conformance"))
            .args(["--scenario", "T1", "--oracle"])
            .arg(&oracle)
            // The runner's own scratch folders, which it cannot remove once it has been ended, go here.
            .env("TMPDIR", scratch.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // A job of its own, as a terminal starts one.
            .process_group(0)
            .spawn()
            .expect("start the runner");
        let mut group = None;
        let read = || std::fs::read_to_string(&group_file).ok().and_then(|text| text.trim().parse::<u32>().ok());
        let started = poll_until(STARTS_WITHIN, || {
            group = read();
            group.is_some()
        });
        assert!(started, "{signal}: the step never started");
        let group = group.expect("the step's group");
        if to_the_group {
            signal_group(runner.id(), signal).expect("signal the runner's group");
        } else {
            runner.kill().expect("kill the runner");
        }
        let ended_by = runner.wait().expect("reap the runner");
        let gone = poll_until(GONE_WITHIN, || !group_has_live_process(group).unwrap_or(true));
        if !gone {
            let _ = signal_group(group, "KILL");
        }
        assert!(gone, "{signal}: the runner ended ({ended_by}) and a process of its step's group {group} ran on");
    }
}
