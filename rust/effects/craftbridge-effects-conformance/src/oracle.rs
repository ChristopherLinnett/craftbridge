//! The oracle: the EffectCraft command line, built from the pinned official revision by
//! `tools/oracle.sh`. Behind a trait so the comparison is tested without it.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What one invocation did.
pub(crate) struct Outcome {
    pub(crate) success: bool,
    /// How the process ended, for the failure that names it.
    pub(crate) ended: String,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) trait Oracle {
    /// Runs the oracle with `args` in `folder`, which holds every file the invocation reads or writes.
    fn run(&self, folder: &Path, args: &[String]) -> Result<Outcome, String>;
}

/// The command-line binary. A step that outlives `timeout` is killed, so a hung export fails the gate
/// instead of holding it.
pub(crate) struct Cli {
    pub(crate) binary: PathBuf,
    pub(crate) timeout: Duration,
}

/// How often a running step is looked at again.
const POLL: Duration = Duration::from_millis(20);

impl Oracle for Cli {
    fn run(&self, folder: &Path, args: &[String]) -> Result<Outcome, String> {
        // Output goes to files, not pipes: nothing then has to be read while the process runs, and a step
        // that prints a lot cannot block on a full pipe.
        let stdout_path = folder.join(".oracle-stdout");
        let stderr_path = folder.join(".oracle-stderr");
        let create = |path: &Path| File::create(path).map_err(|error| format!("cannot create {}: {error}", path.display()));
        let mut child = Command::new(&self.binary)
            .args(args)
            .current_dir(folder)
            // The oracle inherits nothing from the runner's environment and is given one variable, its
            // configuration folder, the scenario's own. EffectCraft, the Craft libraries and the crates
            // under them read settings from variables spelt with no common prefix (font folders, the
            // compositor, the GPU adapter, thread counts, the locale, the time zone, the user's own font
            // folder through HOME, the loader's DYLD_ ones), and more with every pin, so no list of what to
            // drop stays complete: a variable the oracle is not given is one it reads at its default, which
            // is EffectCraft as it is, whatever shell starts the gate. It needs none to run; what stays the
            // machine's is the system itself, its own fonts among it.
            .env_clear()
            .env("EFFECTCRAFT_CONFIG_DIR", folder.join(".config"))
            .stdin(Stdio::null())
            .stdout(create(&stdout_path)?)
            .stderr(create(&stderr_path)?)
            .spawn()
            .map_err(|error| format!("cannot start {}: {error}", self.binary.display()))?;
        let started = Instant::now();
        let status = loop {
            match child.try_wait().map_err(|error| format!("cannot wait for {}: {error}", self.binary.display()))? {
                Some(status) => break status,
                None if started.elapsed() >= self.timeout => {
                    // The process may have ended in the meantime; either way it is reaped.
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("did not finish within {} s and was stopped", self.timeout.as_secs()));
                }
                None => std::thread::sleep(POLL),
            }
        };
        let read = |path: &Path| {
            std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).map_err(|error| format!("cannot read {}: {error}", path.display()))
        };
        Ok(Outcome { success: status.success(), ended: status.to_string(), stdout: read(&stdout_path)?, stderr: read(&stderr_path)? })
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// How long the step the timeout test stops would run if nothing stopped it.
    const OUTLASTING_STEP: Duration = Duration::from_secs(30);

    /// A runner of the script `body`, under the bound the runner gives every real step. Every step but the
    /// timeout test's ends on its own, so that bound only stops one that is truly stuck: a loaded machine,
    /// or a freshly written script waiting for the system's first-run scan, holds a step far past any pace
    /// a test might guess, and no test's pace depends on it.
    fn script(folder: &Path, body: &str) -> Cli {
        let binary = folder.join("oracle.sh");
        std::fs::write(&binary, format!("#!/bin/sh\n{body}\n")).expect("write the script");
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).expect("make it executable");
        Cli { binary, timeout: crate::STEP_TIMEOUT }
    }

    /// The oracle's whole environment is its configuration folder: whatever the runner's process holds (HOME,
    /// and here cargo's own variables) stays behind. Only the names the shell running the script exports
    /// for itself are left out of the comparison.
    #[test]
    fn the_oracle_is_given_its_configuration_folder_and_nothing_of_the_runners_environment() {
        let scratch = tempfile::tempdir().expect("scratch");
        let dir = scratch.path().to_path_buf();
        let outcome = script(&dir, "exec /usr/bin/env").run(&dir, &[]).expect("runs");
        let shells_own = ["PWD", "OLDPWD", "SHLVL", "_"];
        let given: Vec<&str> = outcome.stdout.lines().filter(|line| !shells_own.iter().any(|name| line.starts_with(&format!("{name}=")))).collect();
        assert_eq!(given, [format!("EFFECTCRAFT_CONFIG_DIR={}", dir.join(".config").display())], "{}", outcome.stdout);
        assert!(std::env::var_os("HOME").is_some(), "the runner's own environment holds what must not reach the oracle");
    }

    #[test]
    fn output_arguments_and_the_working_folder_reach_the_process_and_come_back() {
        let scratch = tempfile::tempdir().expect("scratch");
        let dir = scratch.path().to_path_buf();
        let cli = script(&dir, "echo \"$1 $2\"; pwd >&2; echo made > out.txt");
        let outcome = cli.run(&dir, &["a b".to_string(), "c".to_string()]).expect("runs");
        assert!(outcome.success);
        assert_eq!(outcome.stdout.trim(), "a b c");
        assert!(outcome.stderr.trim().ends_with(dir.file_name().and_then(|name| name.to_str()).unwrap_or("?")), "{}", outcome.stderr);
        assert!(dir.join("out.txt").is_file());
    }

    #[test]
    fn a_failing_step_reports_how_it_ended() {
        let scratch = tempfile::tempdir().expect("scratch");
        let dir = scratch.path().to_path_buf();
        let outcome = script(&dir, "echo no >&2; exit 3").run(&dir, &[]).expect("runs");
        assert!(!outcome.success && outcome.ended.contains('3') && outcome.stderr.trim() == "no");
    }

    #[test]
    fn a_step_that_outlives_the_timeout_is_stopped_and_named() {
        let scratch = tempfile::tempdir().expect("scratch");
        let dir = scratch.path().to_path_buf();
        // `exec` makes the sleep the process the runner kills: a shell killed while it waits on a sleep
        // would leave the sleep running on after the test.
        let mut cli = script(&dir, &format!("exec sleep {}", OUTLASTING_STEP.as_secs()));
        cli.timeout = Duration::from_millis(200);
        let started = Instant::now();
        let error = cli.run(&dir, &[]).err().unwrap_or_default();
        let elapsed = started.elapsed();
        assert!(error.contains("was stopped"), "{error}");
        // The step starts after `started` and cannot end by itself sooner than `OUTLASTING_STEP` after it,
        // so a return before then is the runner's stop: a slow start, the first-run scan included, delays
        // only the step, and the kill reaches a step held by the scan at once.
        assert!(elapsed >= cli.timeout && elapsed < OUTLASTING_STEP, "stopped after {elapsed:?}");
    }

    #[test]
    fn a_missing_binary_is_an_error_naming_it() {
        let scratch = tempfile::tempdir().expect("scratch");
        let dir = scratch.path().to_path_buf();
        let cli = Cli { binary: dir.join("nothing-here"), timeout: crate::STEP_TIMEOUT };
        assert!(cli.run(&dir, &[]).err().unwrap_or_default().contains("nothing-here"));
    }
}
