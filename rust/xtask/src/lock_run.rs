//! The run under a held lock, whole, which two commands share: `cargo xtask device-lock` queues its
//! runs of one device, and `cargo xtask release-lock` refuses its second runs of one staging; both hold
//! the lock on the same terms, and those terms are written here once.
//!
//! The run belongs to the process that started the command, its caller: a run still waiting when its
//! caller has gone never starts, and one running is asked to end (`TERM` to its process group, which a
//! run's own cleanup answers) and is killed once [`GRACE`] is out. The lock is let go only once the run
//! has ended, so the next run never meets this one's cleanup. The program runs in a process group of
//! its own that `craftbridge-process`'s watchers keep, so a kill of this command itself leaves nothing
//! of the run behind either, though nothing then cleans up after it; and the system lets go of the lock
//! however this command ends.
//!
//! The program is told it holds the lock by a variable naming the lock and this command's process id,
//! which a program checks against its own parent — the one fact no inherited or exported variable can
//! carry for it, so a shell that exported the variable, or a run started inside a run, holds no lock.

use crate::report;
use craftbridge_process::Group;
use std::io::Write;
use std::os::unix::process::parent_id;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};
use std::time::{Duration, Instant};

/// How often the wait for the program looks again, at the program and at the caller.
const POLL: Duration = Duration::from_millis(20);

/// How long a run whose caller has gone is given to end its own way before it is killed: a run's own
/// cleanup is a few quick calls — the device's uninstalls, a tag's `git tag -d` — each well inside it,
/// and a run past it has stopped answering.
pub(crate) const GRACE: Duration = Duration::from_secs(120);

/// What one lock command's locks are, all that sets it apart from the other's but what it does on a lock
/// another run holds ([`Busy`]).
pub(crate) struct Lock {
    /// The command's own name, which every line it says leads with.
    pub command: &'static str,
    /// What a lock's name names, for the refusal of a name no lock file can be named after, since one
    /// that spells a path would point the lock at another folder than the machine's.
    pub names: &'static str,
    /// How such a name is written, for the same refusal.
    pub written_as: &'static str,
    /// What the lines about a run call the thing its lock stands for, before its name ("device CB-…",
    /// "release lock CB-…").
    pub called: &'static str,
    /// The lock file a name takes in the machine's lock folder.
    pub marker: fn(&Path, &str) -> PathBuf,
    /// The variable the program runs with: `<name>:<pid>`, naming the lock and this command's process,
    /// which is the program's parent.
    pub holder: &'static str,
}

/// What a run does when another run already holds the lock.
pub(crate) enum Busy {
    /// Wait for the lock: say `said` once, before waiting, so the queue shows in the log, and if the
    /// caller goes while waiting, never start and fail with `gave_up`.
    Waits { said: String, gave_up: String },
    /// Refuse this run with `refused`, in words that name the lock: waiting would start it on facts the
    /// run it waited for has since changed.
    Refuses { refused: String },
}

/// The command itself: hold the lock of `name`, run the program under it and end as the program did, or
/// fail in the command's own words, naming what stopped the run.
pub(crate) fn run(lock: &Lock, name: &str, busy: &Busy, program: &str, arguments: &[&str]) -> ExitCode {
    match under_the_lock(lock, name, busy, program, arguments) {
        Ok(exit) => exit,
        Err(failure) => report::failed(lock.command, failure),
    }
}

/// Hold the lock, run the program under it, and end as the program did. A name no lock file can be
/// named after is refused before the locks folder is touched. The lock is taken before the program
/// starts, so a run that waits or is refused reads and writes nothing of its work meanwhile; it is held
/// whole until the program ends, its cleanup too when its caller has gone, and letting go of it is the
/// last thing this command does.
fn under_the_lock(lock: &Lock, name: &str, busy: &Busy, program: &str, arguments: &[&str]) -> Result<ExitCode, String> {
    if !crate::filelock::is_name(name) {
        return Err(format!(
            "{name} is no {} a lock can be named after: name it by {}, of letters, digits, dashes and underscores",
            lock.names, lock.written_as
        ));
    }
    let subject = format!("{} {name}", lock.called);
    let caller = parent_id();
    let marker = (lock.marker)(&crate::filelock::folder()?, name);
    let _held = match busy {
        Busy::Waits { said, gave_up } => {
            if crate::filelock::held(&marker)? {
                println!("{said}");
            }
            let held = crate::filelock::hold(&marker)?;
            if parent_id() != caller {
                return Err(gave_up.clone());
            }
            held
        }
        Busy::Refuses { refused } => crate::filelock::try_hold(&marker)?.ok_or_else(|| refused.clone())?,
    };
    let mut command = Command::new(program);
    command.args(arguments).env(lock.holder, format!("{name}:{}", std::process::id()));
    let mut group = Group::spawn(&mut command).map_err(|error| format!("could not run {program}: {error}"))?;
    let status = loop {
        if let Some(status) = ended(&mut group, program)? {
            break status;
        }
        if parent_id() != caller {
            // Said to the standard error, and lost without failing should that have gone with the caller.
            let _ = writeln!(std::io::stderr(), "{}: the caller of the run for {subject} has gone: the run is asked to end", lock.command);
            break stopped(&mut group, &subject, program, GRACE)?;
        }
        std::thread::sleep(POLL);
    };
    Ok(report::exit_of(lock.command, program, status))
}

/// The program's exit status once it has ended.
fn ended(group: &mut Group, program: &str) -> Result<Option<ExitStatus>, String> {
    group.try_wait().map_err(|error| format!("could not wait for {program}: {error}"))
}

/// Stop the run of a caller that has gone: its process group is asked to end, as a terminal's Ctrl-C asks
/// the run in its foreground, so the run's own cleanup runs while this command still holds the lock,
/// and whatever has not ended once `grace` is out is killed. The command gives the run [`GRACE`] — the
/// bound a real cleanup needs room for; `grace` is a parameter so its test can outwait it without
/// waiting out the bound itself.
pub(crate) fn stopped(group: &mut Group, subject: &str, program: &str, grace: Duration) -> Result<ExitStatus, String> {
    craftbridge_process::signal_group(group.id(), "TERM")?;
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if let Some(status) = ended(group, program)? {
            return Ok(status);
        }
        std::thread::sleep(POLL);
    }
    group.stop()?;
    Err(format!("the run for {subject} did not end within {} seconds of being asked, and was killed", grace.as_secs()))
}
