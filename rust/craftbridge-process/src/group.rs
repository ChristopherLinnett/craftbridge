use crate::command::output_of;
use crate::poll::POLL;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

/// Longest a group is given to end once it has been killed: a kill ends a process at once, so a group
/// still alive after this holds a process no kill reaches, which the error then names.
const STOP_BOUND: Duration = Duration::from_secs(30);

/// What a group's guard runs. It reads its end of a socket whose other end only the program that started
/// the group holds, and which that program never writes, so `read` returns only once the kernel closes the
/// program's end as the program ends, however it ends; then it kills its own process group, which it leads.
const GUARD: &str = "read -r held; kill -s KILL 0";

/// Send `signal` to every process in the group `group` leads, as a terminal sends Ctrl-C. A group with
/// nothing left alive in it is as gone as one that no longer exists: the signal has no one to reach,
/// and that is no failure. Only a group that still holds a live process the signal did not reach is
/// one, and the system's own words are the error. Nothing here panics: a [`Group`]'s `Drop` stops it
/// through here while a failure unwinds, where a panic would abort the whole suite.
pub fn signal_group(group: u32, signal: &str) -> Result<(), String> {
    let sent = output_of(Command::new("kill").args(["-s", signal, "--", &format!("-{group}")]))?;
    if sent.status.success() || !group_has_live_process(group)? {
        return Ok(());
    }
    Err(String::from_utf8_lossy(&sent.stderr).trim().to_string())
}

/// Does the process group `group` hold a process that has not yet exited? `kill` answers "No such
/// process" for a group that is gone, but macOS refuses a group whose every process has exited and
/// not yet been reaped as "Operation not permitted", since its kernel signals no zombie. `pgrep -g`,
/// which matches no zombie either, says which of the two a refusal was by its exit status alone: 0
/// for a match, 1 for none, anything else for a `pgrep` that could not tell, which judges nothing.
pub fn group_has_live_process(group: u32) -> Result<bool, String> {
    let found = pgrep(group)?;
    match found.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!("pgrep could not search the group {group}: {}", String::from_utf8_lossy(&found.stderr).trim())),
    }
}

/// `pgrep -g group`: the group's live processes, one id a line, and whether it found any.
fn pgrep(group: u32) -> Result<Output, String> {
    output_of(Command::new("pgrep").args(["-g", &group.to_string()]))
}

/// Stop the whole group `leader` leads, and answer once no process of it is left alive and `leader` has
/// been reaped. `leader` was started leading a group of its own (`process_group(0)`) and has not been
/// reaped since: a reaped process's id is free, and another group may already carry it.
pub fn stop_group(leader: &mut Child) -> Result<(), String> {
    let group = leader.id();
    end(group, &mut [leader])
}

/// Kill the group `group` until no process of it is left alive, then reap `members`, the processes of it
/// this program started, its leader last. The kill is repeated: a process that forked while one was being
/// sent joined the group after the signal's list was made, and the next one reaches it. Nothing is reaped
/// while a process of the group lives, and the leader is reaped last: an unreaped process keeps its id,
/// and the group's id is its leader's, so no other group can have taken the number each kill is sent to.
fn end(group: u32, members: &mut [&mut Child]) -> Result<(), String> {
    let deadline = Instant::now() + STOP_BOUND;
    loop {
        signal_group(group, "KILL")?;
        if !group_has_live_process(group)? && reaped(group, members)? {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("process group {group} was not gone {} s after it was killed", STOP_BOUND.as_secs()));
        }
        std::thread::sleep(POLL);
    }
}

/// Reap each of `members` in turn, and stop at the first that has not ended, so every one after it,
/// the leader among them, stays unreaped.
fn reaped(group: u32, members: &mut [&mut Child]) -> Result<bool, String> {
    for member in members.iter_mut() {
        let id = member.id();
        if member.try_wait().map_err(|error| format!("cannot wait for process {id} of group {group}: {error}"))?.is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A process started in a process group of its own, which a guard leads: everything the process starts
/// joins the group, and [`Group::stop`] ends the group whole. The guard ends it too should the program that
/// started it end first, however that ends (a Ctrl-C, which a terminal sends to its own group and so no
/// longer to this one, or a kill no handler sees), since the kernel then closes this program's end of the
/// guard's socket.
pub struct Group {
    process: Child,
    guard: Child,
    /// This program's end of the guard's socket, held and never written: its closing is what wakes the
    /// guard, and the guard's end closing, as the guard exits, is what this end reads as the stream's end.
    held: UnixStream,
    stopped: bool,
}

impl Group {
    /// Start `command` in a new process group, led by its guard.
    pub fn spawn(command: &mut Command) -> std::io::Result<Self> {
        let (held, waits_on) = UnixStream::pair()?;
        // Read only to learn whether the guard has gone, which must never wait.
        held.set_nonblocking(true)?;
        let (process, guard) = guarded(GUARD_SHELL, command, waits_on)?;
        Ok(Self { process, guard, held, stopped: false })
    }

    /// The group's id, which is its guard's process id: the guard leads it.
    pub fn id(&self) -> u32 {
        self.guard.id()
    }

    /// The process's exit status once it has ended, as [`Child::try_wait`] gives it. A guard that has gone
    /// while the process still runs is an error, since nothing would end the group were the program that
    /// started it to end first; the group is then still whole, and [`Group::stop`] ends it.
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        if let Some(status) = self.process.try_wait()? {
            return Ok(Some(status));
        }
        if self.guard_has_gone()? {
            return Err(std::io::Error::other("the process group's guard has gone while its process still runs"));
        }
        Ok(None)
    }

    /// Has the guard exited? Its end of the socket closes as it exits, which this end reads as the stream's
    /// end. The guard is not reaped here: it leads the group, and until it is reaped its id, which is the
    /// group's, can be no other process's or group's, so every kill [`Group::stop`] sends reaches this group.
    fn guard_has_gone(&self) -> std::io::Result<bool> {
        match (&self.held).read(&mut [0; 1]) {
            Ok(0) => Ok(true),
            // The guard writes nothing, and a read the system broke off, or one with nothing to read yet,
            // is a guard still waiting.
            Ok(_) => Ok(false),
            Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Stop the whole group, and answer once no process of it is left alive and the process and its guard
    /// have been reaped. A group already stopped is not signalled again: its id may since be another's.
    pub fn stop(&mut self) -> Result<(), String> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        let group = self.id();
        end(group, &mut [&mut self.process, &mut self.guard])
    }
}

/// The shell a guard runs in.
const GUARD_SHELL: &str = "/bin/sh";

/// Start a guard in `shell`, reading `waits_on` and leading a group of its own, then `command` in that
/// group: the guard and the process, in that order. The guard starts first, so a guard that cannot start
/// fails the process before it runs, and the process never runs unguarded. std's sockets are closed on
/// exec, so neither the process nor anything it starts holds this program's end, which would keep the
/// guard waiting after the program had gone.
fn guarded(shell: &str, command: &mut Command, waits_on: UnixStream) -> std::io::Result<(Child, Child)> {
    let mut guard = Command::new(shell)
        .args(["-c", GUARD])
        .process_group(0)
        .stdin(OwnedFd::from(waits_on))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| std::io::Error::other(format!("could not start its process group's guard, {shell}: {error}")))?;
    match i32::try_from(guard.id()).map_err(std::io::Error::other).and_then(|group| command.process_group(group).spawn()) {
        Ok(process) => Ok((process, guard)),
        Err(error) => {
            let _ = guard.kill();
            let _ = guard.wait();
            Err(error)
        }
    }
}

impl Drop for Group {
    /// A process still running when its group is dropped is one its program gave up on, through an error
    /// or a panic, so the group is stopped. One that ended on its own leaves its group as a process that
    /// ends leaves it, and only the guard goes, killed by its own id, which no other process can carry
    /// while it is unreaped. Nothing here panics: a drop runs while a failure unwinds too.
    fn drop(&mut self) {
        if !matches!(self.process.try_wait(), Ok(Some(_))) {
            let _ = self.stop();
        }
        let _ = self.guard.kill();
        let _ = self.guard.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poll_until;

    /// Long enough that nothing in a test outlives it by chance, short enough that a leak ends by itself.
    const OUTLASTING: &str = "120";
    const STUCK_AFTER: Duration = Duration::from_secs(60);

    /// `program` started leading a group of its own, unguarded, as [`stop_group`] takes its leader.
    fn leader(program: &str, args: &[&str]) -> Child {
        Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::null()).process_group(0).spawn().unwrap()
    }

    /// The live processes of the group `group`.
    fn pids_of(group: u32) -> Vec<String> {
        String::from_utf8_lossy(&pgrep(group).unwrap().stdout).lines().map(str::to_string).collect()
    }

    /// Scripts that outlast every test, each with the number of processes it runs once it has started them
    /// all: a shell running `sleep` without `exec`, which waits on a child of its own; one whose child starts
    /// a child; and one whose child and grandchild ignore every signal a terminal or a softer stop sends.
    fn outlasting_scripts() -> [(String, usize); 3] {
        [
            (format!("sleep {OUTLASTING}; true"), 2),
            (format!("sh -c 'sleep {OUTLASTING}; true' & wait"), 3),
            (format!("sh -c 'trap \"\" TERM INT HUP; sleep {OUTLASTING}; true' & wait"), 3),
        ]
    }

    /// The state `ps` gives the process `pid`, `Z` for one that has exited and not been reaped, and nothing
    /// for one that is gone.
    fn state_of(pid: u32) -> String {
        let state = output_of(Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()])).unwrap();
        String::from_utf8_lossy(&state.stdout).trim().to_string()
    }

    /// Wait until the group `group` runs `count` live processes.
    fn wait_for_processes(group: u32, count: usize, script: &str) {
        assert!(poll_until(STUCK_AFTER, || pids_of(group).len() == count), "{script}: the group never ran {count} processes");
    }

    /// A process group is signalled quietly and correctly whatever state it is in: a live one is killed, one
    /// whose every process has exited but not been reaped (which macOS refuses as "Operation not
    /// permitted", a zombie taking no signal) and one that is gone altogether are no failure, and say
    /// nothing; a live one the signal did not reach is a failure, in `kill`'s own words.
    #[test]
    fn a_group_is_killed_when_alive_and_left_quietly_when_gone() {
        let mut live = leader("sleep", &[OUTLASTING]);
        let unsent = signal_group(live.id(), "NOSUCHSIGNAL");
        assert!(unsent.as_ref().is_err_and(|words| words.contains("NOSUCHSIGNAL")), "a live group no signal reached is a failure, in kill's words: {unsent:?}");
        assert_eq!(live.try_wait().unwrap(), None, "and the group it did not reach lives on");
        signal_group(live.id(), "KILL").unwrap();
        assert!(!live.wait().unwrap().success(), "the live group's leader was killed");
        let exited = leader("true", &[]);
        assert!(poll_until(STUCK_AFTER, || state_of(exited.id()).starts_with('Z')), "gave up waiting for the group's only process to exit");
        assert_eq!(signal_group(exited.id(), "KILL"), Ok(()), "a group of exited processes is gone, not unreachable");
        let mut exited = exited;
        exited.wait().unwrap();
        assert_eq!(signal_group(exited.id(), "KILL"), Ok(()), "and so is one whose processes were all reaped");
    }

    /// Stopping a group leaves no process of it alive and its leader reaped, whatever the leader runs: a
    /// kill of the leader alone leaves its children running.
    #[test]
    fn stopping_a_group_leaves_no_process_of_it_alive() {
        for (script, count) in outlasting_scripts() {
            let mut shell = leader("sh", &["-c", &script]);
            let group = shell.id();
            wait_for_processes(group, count, &script);
            stop_group(&mut shell).unwrap();
            assert_eq!(shell.try_wait().unwrap().map(|status| status.success()), Some(false), "the leader was reaped: {script}");
            assert!(!group_has_live_process(group).unwrap(), "a process of the group is still alive: {script}");
        }
    }

    /// A guarded group is stopped whole, its guard with it, and both of the processes this program started
    /// are reaped.
    #[test]
    fn a_guarded_group_is_stopped_whole() {
        for (script, count) in outlasting_scripts() {
            let mut group = Group::spawn(Command::new("sh").args(["-c", &script])).unwrap();
            let id = group.id();
            wait_for_processes(id, count + 1, &script);
            group.stop().unwrap();
            assert!(!group_has_live_process(id).unwrap(), "a process of the group is still alive: {script}");
            assert_eq!(group.try_wait().unwrap().map(|status| status.success()), Some(false), "the process was reaped: {script}");
            assert!(group.guard.try_wait().unwrap().is_some(), "and so was its guard: {script}");
        }
    }

    /// A group whose program ends without stopping it, as one does when it is interrupted or killed, is
    /// ended by its guard: the kernel closes the program's end of the socket the guard reads, as letting go of it here
    /// does.
    #[test]
    fn a_group_whose_program_ends_is_ended_by_its_guard() {
        for (script, count) in outlasting_scripts() {
            let (held, waits_on) = UnixStream::pair().unwrap();
            let (mut process, mut guard) = guarded(GUARD_SHELL, Command::new("sh").args(["-c", &script]), waits_on).unwrap();
            let group = guard.id();
            wait_for_processes(group, count + 1, &script);
            drop(held);
            let ended = poll_until(STUCK_AFTER, || !group_has_live_process(group).unwrap());
            if !ended {
                signal_group(group, "KILL").unwrap();
            }
            process.wait().unwrap();
            guard.wait().unwrap();
            assert!(ended, "the guard did not end its group when its program let go: {script}");
        }
    }

    /// A guard killed while its process runs is told of, not left: the group would outlive the program that
    /// started it. The group is whole still, and stopping it ends it.
    #[test]
    fn a_guard_that_ends_before_its_process_is_an_error_and_the_group_still_stops() {
        let script = format!("sleep {OUTLASTING}; true");
        let mut group = Group::spawn(Command::new("sh").args(["-c", &script])).unwrap();
        let id = group.id();
        wait_for_processes(id, 3, &script);
        assert!(group.try_wait().unwrap().is_none(), "a guard that waits is no error");
        let killed = output_of(Command::new("kill").args(["-s", "KILL", &id.to_string()])).unwrap();
        assert!(killed.status.success(), "the guard alone was killed");
        let told = poll_until(STUCK_AFTER, || group.try_wait().is_err());
        let guard_state = state_of(id);
        group.stop().unwrap();
        assert!(told, "the guard's end was never told of");
        assert!(
            guard_state.starts_with('Z'),
            "the guard was reaped before its group had ended, freeing the id every kill of the group goes to: {guard_state:?}"
        );
        assert!(!group_has_live_process(id).unwrap(), "a process of the group is still alive");
        assert!(group.try_wait().unwrap().is_some(), "the process was reaped");
    }

    /// A guard that cannot start fails its group before the process is started: the process never runs
    /// unguarded, and the error names the guard's shell.
    #[test]
    fn a_guard_that_cannot_start_fails_the_group_before_its_process_runs() {
        let marker = std::env::temp_dir().join(format!("craftbridge-process-unguarded-{}", std::process::id()));
        let (_held, waits_on) = UnixStream::pair().unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "touch \"$1\"", "sh"]).arg(&marker);
        let error = guarded("/craftbridge-no-such-shell", &mut command, waits_on).err().map(|error| error.to_string()).unwrap_or_default();
        assert!(error.starts_with("could not start its process group's guard, /craftbridge-no-such-shell: "), "{error}");
        let ran = marker.exists();
        let _ = std::fs::remove_file(&marker);
        assert!(!ran, "the process ran although its guard never started");
    }

    /// A step that cannot start leaves no guard waiting behind it either. This end of the guard's socket
    /// stays open here, so nothing but the guard's own kill, by the id its group carries, can end it:
    /// the error may only answer once the guard has gone, the way the drop ends the guard of a group
    /// whose process ended, or a guard left waiting holds a group's id over a group with nothing in it.
    #[test]
    fn a_step_that_cannot_start_leaves_no_guard_waiting() {
        let (held, waits_on) = UnixStream::pair().unwrap();
        held.set_nonblocking(true).unwrap();
        let mut command = Command::new("/craftbridge-no-such-binary");
        assert!(guarded(GUARD_SHELL, &mut command, waits_on).is_err(), "a step that cannot start is an error");
        let gone = poll_until(STUCK_AFTER, || matches!((&held).read(&mut [0; 1]), Ok(0)));
        assert!(gone, "the guard was still waiting after the step that could not start had failed: its end of the socket never closed");
    }

    /// A group whose process ends on its own is left as the process that ends leaves it: what the step
    /// started in the group runs on, for its own program to answer for, and the drop takes only the
    /// guard. The runner answers for a step it cut short; for one that finished on its own it kills
    /// nothing it did not ask to stop, so a step that ends while leaving work running finds that work
    /// still there.
    #[test]
    fn a_group_whose_process_ends_on_its_own_leaves_the_group_behind_it() {
        let marker = std::env::temp_dir().join(format!("craftbridge-process-leftover-{}", std::process::id()));
        let script = format!("sleep {OUTLASTING} & echo $! > '{}'", marker.display());
        let mut ended = Group::spawn(Command::new("sh").args(["-c", &script])).unwrap();
        let group = ended.id();
        assert!(poll_until(STUCK_AFTER, || marker.is_file()), "the step never wrote the id of what it left running");
        let leftover: u32 = std::fs::read_to_string(&marker).unwrap().trim().parse().unwrap();
        let _ = std::fs::remove_file(&marker);
        assert!(poll_until(STUCK_AFTER, || ended.try_wait().unwrap().is_some()), "the step never ended");
        drop(ended);
        let state = state_of(leftover);
        assert!(state.starts_with('S') || state.starts_with('R'), "a step that ended on its own stopped its group anyway: the process it left is {state:?}");
        signal_group(group, "KILL").unwrap();
    }

    /// A group that ends leaves no guard once it is dropped: one whose process ended on its own, whose guard
    /// is killed by its own id and reaped, and one whose every process was killed from outside, which
    /// answers that kill as the process's end.
    #[test]
    fn a_group_that_ends_leaves_no_guard() {
        let mut ended = Group::spawn(&mut Command::new("true")).unwrap();
        let id = ended.id();
        assert!(poll_until(STUCK_AFTER, || ended.try_wait().unwrap().is_some_and(|status| status.success())), "the process never ended");
        drop(ended);
        assert_eq!(state_of(id), "", "the guard of a group whose process ended was left behind");
        let script = format!("sleep {OUTLASTING}; true");
        let mut killed = Group::spawn(Command::new("sh").args(["-c", &script])).unwrap();
        let id = killed.id();
        wait_for_processes(id, 3, &script);
        signal_group(id, "KILL").unwrap();
        let answered = poll_until(STUCK_AFTER, || killed.try_wait().is_ok_and(|status| status.is_some_and(|status| !status.success())));
        drop(killed);
        assert!(answered, "a group killed from outside never answered the kill as its process's end");
        assert_eq!(state_of(id), "", "the guard of a group killed from outside was left unreaped");
        assert!(!group_has_live_process(id).unwrap(), "a process of the group is still alive");
    }

    /// A group already gone is stopped without complaint: a leader that exited and was not yet reaped, and
    /// a guarded group whose process has ended and been reaped, its guard keeping the group's id its own.
    #[test]
    fn stopping_a_group_that_has_already_ended_is_no_failure() {
        let mut ended = leader("true", &[]);
        stop_group(&mut ended).unwrap();
        assert!(ended.try_wait().unwrap().is_some_and(|status| status.success()), "the leader ended on its own and was reaped");
        let mut group = Group::spawn(&mut Command::new("true")).unwrap();
        assert!(poll_until(STUCK_AFTER, || group.try_wait().unwrap().is_some()), "the guarded process never ended");
        group.stop().unwrap();
        group.stop().unwrap();
        assert!(!group_has_live_process(group.id()).unwrap(), "the guard went with the stop");
    }
}
