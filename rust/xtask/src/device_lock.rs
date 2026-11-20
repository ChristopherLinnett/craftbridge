//! The machine's lock on one device, and the run held under it: `cargo xtask device-lock <device> --
//! <program> <argument>…` holds the lock of `device` — in the folder of the locks
//! (`CRAFTBRIDGE_SIMULATOR_LOCKS`) where `cargo xtask simulator` takes one per simulator, through the same
//! functions and the same system file lock (`crate::filelock`), under a name no simulator's can be — then
//! runs the program under it and ends as the program did.
//!
//! `tools/ios-device-test.sh` runs itself again under this command because every run of it installs its
//! host app on the phone under one pair of bundle ids: two runs started at once replace each other's apps
//! mid-run, and the one that ends first uninstalls what the other is still testing on, so what fails is the
//! runs, not the code. One lock per device makes them queue instead, and a run that waits for another says
//! so with the device's name, so the queue shows in the log. The run's guard — its caller watched, the run
//! asked and waited out and killed, the lock let go only once the run has ended, the program told it holds
//! the lock — is `crate::lock_run`'s, the same one `cargo xtask release-lock` holds a staging run under.

use std::process::ExitCode;

/// The variable the program runs with: `<device>:<pid>`, the device whose lock it runs under and the
/// process id of this command, which holds that lock and is the program's parent.
const HOLDER: &str = "CRAFTBRIDGE_DEVICE_LOCK";

/// A device's lock, named by the identifier CoreDevice answers for an iPhone: letters, digits and
/// dashes, which a lock file can be named after.
const LOCK: crate::lock_run::Lock = crate::lock_run::Lock {
    command: "device-lock",
    names: "device",
    written_as: "its identifier",
    called: "device",
    marker: crate::filelock::for_device,
    holder: HOLDER,
};

/// `cargo xtask device-lock <device> -- <program> <argument>…`: hold the lock of `device` and run the
/// program under it. The runs of one device queue, and a run that finds the device taken says so, naming
/// the device, before it waits.
pub(crate) fn run(device: &str, program: &str, arguments: &[&str]) -> ExitCode {
    let busy = crate::lock_run::Busy::Waits {
        said: format!("device-lock: the run waits for device {device}: another run is testing on it"),
        gave_up: format!("the caller of the run for device {device} went while it waited for the device, so the run never started"),
    };
    crate::lock_run::run(&LOCK, device, &busy, program, arguments)
}

#[cfg(test)]
mod tests {
    use super::HOLDER;
    use crate::scratch::{Scratch, xtask_binary};
    use std::io::BufRead;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// How long a wait inside a test may run before the test calls it lost.
    const PATIENCE: Duration = Duration::from_secs(120);

    /// A phone no machine has, named as CoreDevice names one.
    const FAKE_PHONE: &str = "00000000-0000-4000-8000-0000000000A1";

    /// The command, run on the locks in `locks`: a test's own device id keeps every test its own lane,
    /// and the lock file is the command's own to make.
    fn on_locks(device: &str, program: &str, arguments: &[&str], locks: &Path) -> std::process::Child {
        let mut command = Command::new(xtask_binary());
        command.arg("device-lock").arg(device).arg("--").arg(program).args(arguments).env("CRAFTBRIDGE_SIMULATOR_LOCKS", locks).stdout(Stdio::piped());
        command.spawn().unwrap()
    }

    /// The lines a run prints, as they come: a reader thread hands each to the test, which asks for the
    /// next with a patience no test may wait out silently.
    fn lines_as_they_come(child: &mut std::process::Child) -> mpsc::Receiver<String> {
        let (sender, lines) = mpsc::channel();
        let pipe = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(pipe).lines().map_while(Result::ok) {
                let _ = sender.send(line);
            }
        });
        lines
    }

    fn next_line(lines: &mpsc::Receiver<String>) -> String {
        lines.recv_timeout(PATIENCE).unwrap_or_else(|_| panic!("the line did not come"))
    }

    /// A run that finds the device taken waits, names the device it waits for, and starts its program
    /// only when the lock is let go — and ends with the program's own result.
    #[test]
    fn a_busy_device_makes_the_run_wait_and_say_which_device() {
        let scratch = Scratch::new("device-lock-wait");
        let locks = scratch.0.join("locks");
        let device = "CB-WAIT-1";
        let marker = crate::filelock::for_device(&locks, device);
        scratch.mkdir("locks");
        let testing = crate::filelock::hold(&marker).unwrap();
        let ran = scratch.0.join("ran");
        let mut child = on_locks(device, "/bin/sh", &["-c", &format!("printf ran > {}", ran.display())], &locks);
        let lines = lines_as_they_come(&mut child);
        assert_eq!(next_line(&lines), format!("device-lock: the run waits for device {device}: another run is testing on it"));
        std::thread::sleep(Duration::from_secs(2)); // time for a program started too early to write what it writes
        assert!(!ran.exists(), "the program ran while another run held the device");
        drop(testing);
        assert!(child.wait().unwrap().success(), "the run did not end as its program did");
        assert_eq!(std::fs::read_to_string(&ran).unwrap(), "ran", "the program did not run under the lock");
    }

    /// The program's result is the command's: a run that failed on the phone must not be reported as a
    /// run that passed.
    #[test]
    fn the_programs_result_is_the_runs_result() {
        let scratch = Scratch::new("device-lock-status");
        let locks = scratch.0.join("locks");
        let mut child = on_locks("CB-STATUS-1", "/bin/sh", &["-c", "exit 7"], &locks);
        assert_eq!(child.wait().unwrap().code(), Some(7));
    }

    /// A run killed by its exact id — which, because the runner hands itself to this command, is all a
    /// kill of the runner needs — leaves neither the lock nor its program behind, and the next run of
    /// that device walks straight in and passes.
    #[test]
    fn a_killed_run_leaves_nothing_behind_and_the_next_run_goes_in() {
        let scratch = Scratch::new("device-lock-kill");
        let locks = scratch.0.join("locks");
        let device = "CB-KILL-1";
        let marker = crate::filelock::for_device(&locks, device);
        let late = scratch.0.join("late");
        let mut child = on_locks(device, "/bin/sh", &["-c", &format!("sleep 2; printf late > {}", late.display())], &locks);
        let took = craftbridge_process::poll_until(PATIENCE, || crate::filelock::held(&marker).unwrap_or(false));
        assert!(took, "the run did not take the device lock");
        child.kill().unwrap();
        child.wait().unwrap();
        let freed = craftbridge_process::poll_until(PATIENCE, || !crate::filelock::held(&marker).unwrap_or(true));
        assert!(freed, "the killed run kept the device lock");
        std::thread::sleep(Duration::from_secs(3)); // past when the killed run's own program would have gone on to write
        assert!(!late.exists(), "the killed run left its program running after it");
        let mut next = on_locks(device, "/bin/sh", &["-c", "exit 0"], &locks);
        let lines = lines_as_they_come(&mut next);
        assert!(next.wait().unwrap().success(), "the next run did not go in after the kill");
        assert_eq!(lines.try_iter().collect::<String>(), "", "a run that waited said so with nothing to wait for");
    }

    /// The program is told it runs under the lock in the one way no other process can tell it: the device,
    /// and the id of the command holding the lock, which is the program's own parent.
    #[test]
    fn the_program_is_told_the_device_and_that_its_parent_holds_it() {
        let scratch = Scratch::new("device-lock-holder");
        let locks = scratch.0.join("locks");
        let told = scratch.0.join("told");
        let script = format!("printf '%s %s' \"$CRAFTBRIDGE_DEVICE_LOCK\" \"$PPID\" > {}", told.display());
        let mut child = on_locks("CB-HOLDER-1", "/bin/sh", &["-c", &script], &locks);
        assert!(child.wait().unwrap().success());
        assert_eq!(std::fs::read_to_string(&told).unwrap(), format!("CB-HOLDER-1:{} {}", child.id(), child.id()));
    }

    /// The command, started by a caller of its own, a shell that waits for it in the background as the
    /// runner does: the caller, its log, which the command's output goes to, and the lock file.
    fn under_a_caller(scratch: &Scratch, device: &str, script: &str) -> (std::process::Child, PathBuf, PathBuf) {
        let locks = scratch.0.join("locks");
        let log = scratch.0.join("caller.log");
        let mut caller = Command::new("/bin/sh");
        caller
            .args(["-c", "\"$0\" device-lock \"$1\" -- /bin/sh -c \"$2\" & wait"])
            .arg(xtask_binary())
            .arg(device)
            .arg(script)
            .env("CRAFTBRIDGE_SIMULATOR_LOCKS", &locks);
        let file = std::fs::File::create(&log).unwrap();
        let caller = caller.stdout(Stdio::from(file.try_clone().unwrap())).stderr(Stdio::from(file)).spawn().unwrap();
        (caller, log, crate::filelock::for_device(&locks, device))
    }

    /// A run whose caller goes — interrupted, or killed by its id — is asked to end, and ends its own way:
    /// its cleanup runs while the lock is still held, so the next run never meets it on the phone, and the
    /// lock is let go once it has ended, nothing of the run left running.
    #[test]
    fn a_run_whose_caller_goes_cleans_up_under_the_lock_and_then_lets_it_go() {
        let scratch = Scratch::new("device-lock-caller");
        let [started, cleaning, cleaned, late] = ["started", "cleaning", "cleaned", "late"].map(|name| scratch.0.join(name));
        let script = format!(
            "trap 'printf x > {}; sleep 2; printf x > {}; exit 1' TERM; printf x > {}; (sleep 4; printf x > {}) & wait",
            cleaning.display(),
            cleaned.display(),
            started.display(),
            late.display()
        );
        let (mut caller, log, marker) = under_a_caller(&scratch, "CB-CALLER-1", &script);
        assert!(craftbridge_process::poll_until(PATIENCE, || started.exists()), "the run never started");
        caller.kill().unwrap();
        caller.wait().unwrap();
        assert!(craftbridge_process::poll_until(PATIENCE, || cleaning.exists()), "the run was not asked to end once its caller had gone: {}", read(&log));
        assert!(crate::filelock::held(&marker).unwrap(), "the lock was let go while the run was still cleaning up");
        assert!(craftbridge_process::poll_until(PATIENCE, || !crate::filelock::held(&marker).unwrap_or(true)), "the lock was never let go");
        assert!(cleaned.exists(), "the lock was let go before the run's cleanup ended");
        std::thread::sleep(Duration::from_secs(5)); // past when what the run started would have gone on to write
        assert!(!late.exists(), "something the run started ran on after it");
        assert!(read(&log).contains("device-lock: the caller of the run for device CB-CALLER-1 has gone: the run is asked to end"), "{}", read(&log));
    }

    /// A run that answers the ask with nothing — a phone hung, a `devicectl` that never answers — is
    /// killed once the ask's grace is out, and nothing of it is left to keep the next run waiting. The
    /// ask, the waiting and the kill are `crate::lock_run::stopped` whole, at a grace short enough for a
    /// test to outwait; the bound the command actually runs it at is `crate::lock_run::GRACE`, named
    /// where it is used alone.
    /// The run ignores every softer signal, so only the kill of the whole group ends it: were the guard
    /// not to see the ask out, the waiting would error on the gone watcher instead of ending the run.
    #[test]
    fn a_run_that_ignores_the_ask_is_killed_when_its_grace_is_out() {
        let scratch = Scratch::new("device-lock-grace");
        let started = scratch.0.join("started");
        // The loop outlives every soft signal: the script ignores them, and each `sleep 1` it starts
        // inherits them ignored, so only a kill of the whole group ends it.
        let script = format!("trap '' TERM INT HUP; printf x > {}; while :; do sleep 1; done", started.display());
        let mut command = Command::new("/bin/sh");
        command.args(["-c", &script]);
        let mut group = craftbridge_process::Group::spawn(&mut command).unwrap();
        assert!(craftbridge_process::poll_until(PATIENCE, || started.exists()), "the run never started");
        let grace = Duration::from_secs(2);
        let asked = Instant::now();
        match crate::lock_run::stopped(&mut group, "device CB-GRACE-1", "/bin/sh", grace) {
            Ok(status) => panic!("a run that ignores every ask ended on its own: {status}"),
            Err(failure) => assert_eq!(failure, "the run for device CB-GRACE-1 did not end within 2 seconds of being asked, and was killed"),
        }
        // The grace given is the grace waited: neither a kill at once, which leaves the phone uncleaned, nor
        // a wait for some other bound. The upper margin is a hang guard for a slow machine, far below the
        // bound the command itself uses.
        let waited = asked.elapsed();
        assert!(waited >= grace, "the run was killed {waited:?} after the ask, before its grace was out");
        assert!(waited < grace + Duration::from_secs(60), "the run was killed {waited:?} after the ask, long past its grace");
        assert_eq!(craftbridge_process::group_has_live_process(group.id()), Ok(false), "a process of the killed run is left running");
    }

    /// A run whose caller goes while it waits for the device never starts once the device is free, and
    /// lets the lock go at once for the run behind it.
    #[test]
    fn a_run_whose_caller_goes_while_it_waits_never_starts() {
        let scratch = Scratch::new("device-lock-gone");
        let device = "CB-GONE-1";
        let ran = scratch.0.join("ran");
        scratch.mkdir("locks");
        let marker = crate::filelock::for_device(&scratch.0.join("locks"), device);
        let testing = crate::filelock::hold(&marker).unwrap();
        let (mut caller, log, _) = under_a_caller(&scratch, device, &format!("printf x > {}", ran.display()));
        assert!(said_within(PATIENCE, &log, &format!("device-lock: the run waits for device {device}")), "the run did not wait: {}", read(&log));
        caller.kill().unwrap();
        caller.wait().unwrap();
        drop(testing);
        let gave_up = format!("the caller of the run for device {device} went while it waited for the device, so the run never started");
        assert!(said_within(PATIENCE, &log, &gave_up), "the run did not give up for its gone caller: {}", read(&log));
        assert!(!ran.exists(), "the run of a caller that had gone was started");
        assert!(craftbridge_process::poll_until(PATIENCE, || !crate::filelock::held(&marker).unwrap_or(true)), "the lock was kept");
    }

    /// Nothing that spells a path or an empty name is let name a lock file, and a run is refused before
    /// any program starts. The locks folder is one this run may use, so the refusal below is the device's
    /// own name and nothing else.
    #[test]
    fn a_device_a_lock_cannot_be_named_after_is_refused() {
        let scratch = Scratch::new("device-lock-refusal");
        for device in ["", "the device", "../elsewhere", "a/b", "a b", "dotted.id"] {
            let made = Command::new(xtask_binary())
                .args(["device-lock", device, "--", "/bin/true"])
                .env("CRAFTBRIDGE_SIMULATOR_LOCKS", &scratch.0)
                .stderr(Stdio::piped())
                .output()
                .unwrap();
            assert!(!made.status.success(), "{device:?} was not refused");
            let said = String::from_utf8_lossy(&made.stderr);
            assert!(
                said.contains("is no device a lock can be named after: name it by its identifier, of letters, digits, dashes and underscores"),
                "{device:?} was refused for something else: {said}"
            );
        }
    }

    /// The runner's own half, whole without a phone: `tools/ios-device-test.sh` hands its run to this
    /// command from the one place it names its device, so a run that finds the device taken waits
    /// before it builds anything, says which device it waits for, and starts its run when the lock is
    /// let go. The profile is a file that is there but is no profile, which is the first thing the run
    /// needs past the lock and cannot read, so the run ends at its own first refusal: no build runs,
    /// and no phone is named.
    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "the runner checks for xcrun, codesign and security, which only macOS has, before it reaches the lock")]
    fn the_runner_waits_behind_the_device_lock_and_names_the_device() {
        let scratch = Scratch::new("device-lock-runner");
        let locks = scratch.0.join("locks");
        let device = FAKE_PHONE;
        let marker = crate::filelock::for_device(&locks, device);
        scratch.mkdir("locks");
        scratch.mkdir("tmp");
        scratch.write("profile", "not a profile");
        let waiting = format!("device-lock: the run waits for device {device}: another run is testing on it");
        let refused = format!("ios-device-test: cannot read the profile {}", scratch.0.join("profile").display());
        let testing = crate::filelock::hold(&marker).unwrap();
        let (mut script, log) = runner(&scratch, device, &locks);
        let mut runner = script.spawn().unwrap();
        assert!(said_within(PATIENCE, &log, &waiting), "the runner did not say it waits for the device: {}", read(&log));
        assert!(!said_within(Duration::from_secs(2), &log, "cannot read the profile"), "the run read the profile while another run held the device");
        drop(testing);
        assert_eq!(runner.wait().unwrap().code(), Some(1), "the run did not end as its own refusal ends it");
        let said = read(&log);
        assert!(said.contains(&refused), "the run did not go on to its own first refusal once the device was free: {said}");
        let freed = craftbridge_process::poll_until(PATIENCE, || !crate::filelock::held(&marker).unwrap_or(true));
        assert!(freed, "the run that failed on the profile kept the device lock");
        let left: Vec<_> = std::fs::read_dir(scratch.0.join("tmp")).unwrap().filter_map(Result::ok).map(|entry| entry.file_name()).collect();
        assert!(
            !left.iter().any(|name| name.to_string_lossy().starts_with("craftbridge-device-lock.")),
            "the run left the lock command's folder behind: {left:?}"
        );
    }

    /// The variable that tells a run it holds the lock is believed only from the lock's own holder, the run's
    /// parent: a shell that exported it, with the device alone or with the holder's id as a run inside the
    /// holder's run inherits it, holds no lock by it, so its run waits behind the run that holds the device
    /// like any other.
    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "the runner checks for xcrun, codesign and security, which only macOS has, before it reaches the lock")]
    fn a_marker_from_anywhere_but_the_holder_holds_no_lock() {
        let scratch = Scratch::new("device-lock-marker");
        let locks = scratch.0.join("locks");
        let device = FAKE_PHONE;
        let marker = crate::filelock::for_device(&locks, device);
        scratch.mkdir("locks");
        scratch.mkdir("tmp");
        scratch.write("profile", "not a profile");
        let waiting = format!("device-lock: the run waits for device {device}: another run is testing on it");
        let mut holder = on_locks(device, "/bin/sleep", &["600"], &locks);
        assert!(craftbridge_process::poll_until(PATIENCE, || crate::filelock::held(&marker).unwrap_or(false)), "the holder never took the device");
        for (said, exported) in [("the device alone", device.to_string()), ("the holder's own id, inherited", format!("{device}:{}", holder.id()))] {
            let (mut wrapped, log) = runner(&scratch, device, &locks);
            wrapped.env(HOLDER, &exported);
            let mut child = wrapped.spawn().unwrap();
            assert!(said_within(PATIENCE, &log, &waiting), "a run with {said} exported did not wait for the device: {}", read(&log));
            assert!(!read(&log).contains("cannot read the profile"), "a run with {said} exported went past the lock: {}", read(&log));
            child.kill().unwrap();
            child.wait().unwrap();
        }
        holder.kill().unwrap();
        holder.wait().unwrap();
    }

    /// The phone is named one way only, as CoreDevice names it, since the lock is taken per name: its UDID,
    /// a lower-case spelling or a name would each take a lock of its own on the same phone, and a run naming
    /// it so is refused before it builds or locks anything.
    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "the runner checks for xcrun, codesign and security, which only macOS has, before it reaches the lock")]
    fn a_phone_named_any_way_but_as_coredevice_names_it_is_refused() {
        let scratch = Scratch::new("device-lock-named");
        let locks = scratch.0.join("locks");
        scratch.mkdir("locks");
        scratch.mkdir("tmp");
        scratch.write("profile", "not a profile");
        for device in ["00008101-0002613A1110001E", &FAKE_PHONE.to_lowercase(), "SeaEl-iPhone", "00000000-0000-4000-8000-0000000000a1x"] {
            let (mut script, log) = runner(&scratch, device, &locks);
            assert_eq!(script.spawn().unwrap().wait().unwrap().code(), Some(1), "{device} was not refused");
            let said = read(&log);
            assert!(
                said.contains(&format!("ios-device-test: CRAFTBRIDGE_DEVICE_ID is {device}, which is no CoreDevice identifier: name the phone as `xcrun devicectl list devices` does, since another name for it would take another lock")),
                "{device} was refused for something else: {said}"
            );
        }
        assert_eq!(std::fs::read_dir(&locks).unwrap().count(), 0, "a refused run took a lock");
    }

    /// The runner, ready to start on a scratch machine: one `--swift` run whose every step before the
    /// lock reads only these variables and files, and whose first step past it needs a real phone. Its
    /// output goes to a file, so the test can read what it has said while it runs; the command comes
    /// back before starting, so a test can hand one variable on last.
    fn runner(scratch: &Scratch, device: &str, locks: &Path) -> (Command, PathBuf) {
        let log = scratch.0.join("run.log");
        let file = std::fs::File::create(&log).unwrap();
        let mut command = Command::new("bash");
        command
            .arg(crate::repo_root().join("tools/ios-device-test.sh"))
            .arg("--swift")
            .arg("--")
            .arg("--filter")
            .arg("no_such_test_on_this_machine")
            .env("CRAFTBRIDGE_SIMULATOR_LOCKS", locks)
            .env("CRAFTBRIDGE_DEVICE_ID", device)
            .env("CRAFTBRIDGE_SIGNING_IDENTITY", "no such identity")
            .env("CRAFTBRIDGE_PROVISIONING_PROFILE", scratch.0.join("profile"))
            .env("CRAFTBRIDGE_DEVICE_LOGS", scratch.0.join("logs"))
            .env("TMPDIR", scratch.0.join("tmp"))
            .env_remove(HOLDER)
            .stdout(Stdio::from(file.try_clone().unwrap()))
            .stderr(Stdio::from(file));
        (command, log)
    }

    /// Whether the log says `said` within `patience`.
    fn said_within(patience: Duration, log: &Path, said: &str) -> bool {
        craftbridge_process::poll_until(patience, || std::fs::read_to_string(log).is_ok_and(|text| text.contains(said)))
    }

    fn read(log: &Path) -> String {
        std::fs::read_to_string(log).unwrap_or_default()
    }
}
