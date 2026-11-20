//! The machine's lock on one repository's staging, and the run held under it: `cargo xtask
//! release-lock <lock> -- <program> <argument>…` holds the lock named for the repository a release
//! stages — in the folder of the locks (`CRAFTBRIDGE_SIMULATOR_LOCKS`) where `cargo xtask device-lock`
//! takes one per device and `cargo xtask simulator` one per simulator, through the same functions and
//! the same system file lock (`crate::filelock`), under a name no device's or simulator's can be — and
//! runs the program under it, ending as the program did.
//!
//! `tools/release.sh` runs itself again under this command because a release stages one version of one
//! tree once: two staging runs of one repository read the same newest release and both mean to tag the
//! version after it, and a run that fails takes back what it reads as its own tag — so the second of
//! them would refuse the first's version mid-flight, or undo the first's tag on failing, in the one
//! place either writes: the repository. A device's queue is right for a device, which is none the worse
//! for starting late; a staging run that waited would start on the version, the changelog and the tip
//! the first has just moved past, so this command refuses a run that finds the lock held, naming the
//! lock, and runs nothing.
//!
//! The guard around the run is `crate::lock_run`'s, whole: the caller watched, the run asked and waited
//! out and killed, the lock let go only once the run has ended. The program is told it holds the lock by
//! [`HOLDER`], which names the lock and this command's process id, and is checked against the program's
//! own parent exactly as `device_lock`'s marker is.

use std::process::ExitCode;

/// The variable the program runs with: `<lock>:<pid>`, the release lock it runs under and the process
/// id of this command, which holds that lock and is the program's parent.
const HOLDER: &str = "CRAFTBRIDGE_RELEASE_LOCK";

/// A repository's staging lock, named by the hash of its git folder.
const LOCK: crate::lock_run::Lock = crate::lock_run::Lock {
    command: "release-lock",
    names: "release",
    written_as: "its hash",
    called: "release lock",
    marker: crate::filelock::for_release,
    holder: HOLDER,
};

/// `cargo xtask release-lock <lock> -- <program> <argument>…`: hold the release lock now or refuse, so
/// the run reads and writes nothing of its release while another staging run holds the repository, and
/// is never left waiting behind it. The release script names a repository by a hash of its git folder
/// because a lock's name lets no path through, and two spellings of one repository must hash alike
/// only where git itself resolves the folder to the same place.
pub(crate) fn run(lock: &str, program: &str, arguments: &[&str]) -> ExitCode {
    let busy = crate::lock_run::Busy::Refuses {
        refused: format!(
            "the release lock {lock} is held: another run stages this repository, and a release stages one version of one tree once, so a second run is refused and not queued"
        ),
    };
    crate::lock_run::run(&LOCK, lock, &busy, program, arguments)
}

#[cfg(test)]
mod tests {
    use crate::scratch::{Scratch, xtask_binary};
    use std::path::Path;
    use std::process::{Command, Stdio};

    /// The command, run on the locks in `locks`: a test's own lock name keeps every test its own lane,
    /// and the lock file is the command's own to make. Both its streams are pipes, so nothing a refusal
    /// says goes to the test's own log, and what it fails with the test can read.
    fn on_locks(lock: &str, program: &str, arguments: &[&str], locks: &Path) -> std::process::Child {
        let mut command = Command::new(xtask_binary());
        command
            .arg("release-lock")
            .arg(lock)
            .arg("--")
            .arg(program)
            .args(arguments)
            .env("CRAFTBRIDGE_SIMULATOR_LOCKS", locks)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.spawn().unwrap()
    }

    /// A second staging run of a repository whose lock another run holds is refused before its program
    /// starts, in words that name the lock: what it would stage is being staged, and waiting would only
    /// start it on facts the first run is moving under its feet.
    #[test]
    fn a_second_staging_run_is_refused_naming_the_lock() {
        let scratch = Scratch::new("release-lock-held");
        let locks = scratch.0.join("locks");
        let lock = "9f86d081884c7d65";
        let marker = crate::filelock::for_release(&locks, lock);
        scratch.mkdir("locks");
        let staging = crate::filelock::hold(&marker).unwrap();
        let ran = scratch.0.join("ran");
        let made = on_locks(lock, "/bin/sh", &["-c", &format!("printf ran > {}", ran.display())], &locks).wait_with_output().unwrap();
        assert!(!made.status.success(), "a second staging run was not refused");
        let said = String::from_utf8_lossy(&made.stderr);
        assert!(
            said.contains(&format!("cargo xtask release-lock: the release lock {lock} is held: another run stages this repository, and a release stages one version of one tree once, so a second run is refused and not queued")),
            "the refusal did not name the lock: {said}"
        );
        assert!(!ran.exists(), "the refused run ran its program while another held the lock");
        assert!(crate::filelock::held(&marker).unwrap(), "the refused run took the lock");
        drop(staging);
    }

    /// A free lock is taken at once: the program runs under it and is told which lock and which parent
    /// holds it, the command ends as the program did, and once the program has ended the lock stands
    /// free for the next staging.
    #[test]
    fn a_staging_run_takes_a_free_lock_and_ends_as_its_program_did() {
        let scratch = Scratch::new("release-lock-free");
        let locks = scratch.0.join("locks");
        let lock = "0123456789abcdef";
        let told = scratch.0.join("told");
        let script = format!("printf '%s %s' \"$CRAFTBRIDGE_RELEASE_LOCK\" \"$PPID\" > {}", told.display());
        let mut child = on_locks(lock, "/bin/sh", &["-c", &script], &locks);
        assert!(child.wait().unwrap().success(), "a staging run on a free lock did not end as its program did");
        assert_eq!(
            std::fs::read_to_string(&told).unwrap(),
            format!("{lock}:{} {}", child.id(), child.id()),
            "the program was not told the lock and its holder"
        );
        let freed = craftbridge_process::poll_until(std::time::Duration::from_secs(120), || {
            !crate::filelock::held(&crate::filelock::for_release(&locks, lock)).unwrap_or(true)
        });
        assert!(freed, "the run kept the release lock after its program ended");
    }

    /// Nothing that spells a path or nothing at all is let name a release lock, and a run is refused
    /// before any lock file or program is in play.
    #[test]
    fn a_release_a_lock_cannot_be_named_after_is_refused() {
        let scratch = Scratch::new("release-lock-name");
        for lock in ["", "the release", "../elsewhere", "a/b", "a b", "dotted.id"] {
            let made = Command::new(xtask_binary())
                .args(["release-lock", lock, "--", "/bin/true"])
                .env("CRAFTBRIDGE_SIMULATOR_LOCKS", &scratch.0)
                .stderr(Stdio::piped())
                .output()
                .unwrap();
            assert!(!made.status.success(), "{lock:?} was not refused");
            let said = String::from_utf8_lossy(&made.stderr);
            assert!(
                said.contains("is no release a lock can be named after: name it by its hash, of letters, digits, dashes and underscores"),
                "{lock:?} was refused for something else: {said}"
            );
        }
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 0, "a refused run took a lock");
    }
}
