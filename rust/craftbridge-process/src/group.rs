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

/// A group's two watchers, by the names their errors give them: the guard, which leads the group, and the
/// sweeper, which watches it from outside.
const GUARD_NAME: &str = "guard";
const SWEEPER_NAME: &str = "sweeper";

/// What a group's guard runs. It reads its end of a socket whose other end only the program that started
/// the group holds, and which that program never writes, so `read` returns only once the kernel closes the
/// program's end as the program ends, however it ends; then it kills its own process group, which it leads.
/// That one kill can leave a process of the group running, which the sweeper ([`sweeper_script`]) ends.
const GUARD: &str = "read -r held; kill -s KILL 0";

/// What the sweeper of the group `group` runs. It waits as the guard does, on a socket of its own, then kills
/// the group at once and again every [`POLL`] for as long as a kill of it succeeds. One kill can leave a
/// process running, for the reason [`end`] repeats its own: on macOS a process that forks while the kill is
/// being sent has a child the kill misses, which Linux's fork does not allow, and the guard, ended by its own
/// kill, sends no second. The sweeper leads a process group of its own, which no kill of the guard's group
/// reaches, so it asks its shell for no job control, which dash and zsh, either of which `/bin/sh` may be,
/// refuse without a terminal. A kill fails once the group is gone, or, on macOS, holds only processes that
/// have exited: stopping there keeps every kill on this group, whose id stays its own while any process of it
/// is left. For the same reason this program stops a group's sweeper before it reaps the group's guard, whose
/// id, the group's, is free to be another's once the guard is reaped.
fn sweeper_script(group: u32) -> String {
    format!("read -r held; while kill -s KILL -- -{group}; do sleep {}; done", POLL.as_secs_f64())
}

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

/// Kill the group `group` until no process of it is left alive, then reap `members`, processes of it this
/// program started, its leader last when it is one of them ([`Group::stop`] reaps its guard itself). The
/// kill is repeated: a process that forked while one was being sent joined the group after the signal's
/// list was made, and the next one reaches it. Nothing is reaped while a process of the group lives, and the
/// leader is reaped last: an unreaped process keeps its id, and the group's id is its leader's, so no other
/// group can have taken the number each kill is sent to.
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
/// joins the group, and [`Group::stop`] ends the group whole. The guard and a sweeper outside the group end
/// it too should the program that started it end first, however that ends (a Ctrl-C, which a terminal sends
/// to its own group and so no longer to this one, or a kill no handler sees), since the kernel then closes
/// this program's ends of their sockets.
pub struct Group {
    process: Child,
    guard: Child,
    sweeper: Child,
    /// This program's ends of the guard's and the sweeper's sockets, in that order, held and never written:
    /// their closing is what wakes the watchers, and a watcher's end closing, as it exits, is what this
    /// program's end reads as the stream's end.
    held: [UnixStream; 2],
    stopped: bool,
}

impl Group {
    /// Start `command` in a new process group, led by its guard and watched by its sweeper.
    pub fn spawn(command: &mut Command) -> std::io::Result<Self> {
        let (held, reads) = sockets()?;
        for end in &held {
            // Read only to learn whether its watcher has gone, which must never wait.
            end.set_nonblocking(true)?;
        }
        let (process, guard, sweeper) = guarded([WATCHER_SHELL; 2], command, reads)?;
        Ok(Self { process, guard, sweeper, held, stopped: false })
    }

    /// The group's id, which is its guard's process id: the guard leads it.
    pub fn id(&self) -> u32 {
        self.guard.id()
    }

    /// The process's exit status once it has ended, as [`Child::try_wait`] gives it. A watcher that has gone
    /// while the process still runs is an error naming it: something ended it, and the group is no longer
    /// watched as it must be should the program that started it end first. The group is then still whole,
    /// and [`Group::stop`] ends it.
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        if let Some(status) = self.process.try_wait()? {
            return Ok(Some(status));
        }
        let [guard, sweeper] = &self.held;
        for (name, held) in [(GUARD_NAME, guard), (SWEEPER_NAME, sweeper)] {
            if has_gone(held)? {
                return Err(std::io::Error::other(format!("the process group's {name} has gone while its process still runs")));
            }
        }
        Ok(None)
    }

    /// Stop the whole group, and answer once no process of it is left alive and the process and both
    /// watchers have been reaped. The sweeper is stopped only once no process of the group is left alive,
    /// so a program that ends while it stops a group still leaves none of it running, and before the guard
    /// is reaped ([`sweeper_script`]): until then the guard, killed with its group, keeps the group's id.
    /// A group already stopped is not signalled again: its id may since be another's.
    pub fn stop(&mut self) -> Result<(), String> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        let group = self.id();
        end(group, &mut [&mut self.process])?;
        stop_watcher(&mut self.sweeper);
        stop_watcher(&mut self.guard);
        Ok(())
    }
}

/// Has the watcher reading the other end of `held` exited? Its end of the socket closes as it exits, which
/// `held` reads as the stream's end. Nothing is reaped here: the guard leads the group, and until it is
/// reaped its id, which is the group's, can be no other process's or group's, so every kill [`Group::stop`]
/// sends reaches this group. The sweeper kills only once this program has ended, and keeps to the group as
/// [`sweeper_script`] says.
fn has_gone(held: &UnixStream) -> std::io::Result<bool> {
    let mut held = held;
    match held.read(&mut [0; 1]) {
        Ok(0) => Ok(true),
        // A watcher writes nothing, and a read the system broke off, or one with nothing to read yet, is a
        // watcher still waiting.
        Ok(_) => Ok(false),
        Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => Ok(false),
        Err(error) => Err(error),
    }
}

/// The shell a group's watchers run in.
const WATCHER_SHELL: &str = "/bin/sh";

/// A socket for each of a group's watchers, the guard's and then the sweeper's: this program's ends, which
/// it holds and never writes, and the ends the watchers read.
fn sockets() -> std::io::Result<([UnixStream; 2], [UnixStream; 2])> {
    let (guard_held, guard_reads) = UnixStream::pair()?;
    let (sweeper_held, sweeper_reads) = UnixStream::pair()?;
    Ok(([guard_held, sweeper_held], [guard_reads, sweeper_reads]))
}

/// Start the watcher `name`, `script` in `shell`, reading `reads` and leading a process group of its own.
fn watcher(shell: &str, name: &str, script: &str, reads: UnixStream) -> std::io::Result<Child> {
    Command::new(shell)
        .args(["-c", script])
        .process_group(0)
        .stdin(OwnedFd::from(reads))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| std::io::Error::other(format!("could not start its process group's {name}, {shell}: {error}")))
}

/// Kill `watcher` by its own id, which no other process can carry while it is unreaped, and reap it. Nothing
/// here fails or panics: it runs while a failure unwinds, and on a watcher that has already gone.
fn stop_watcher(watcher: &mut Child) {
    let _ = watcher.kill();
    let _ = watcher.wait();
}

/// Start a group's guard, in the first of `shells` and reading the first of `reads`, leading a group of its
/// own; then its sweeper, in the second shell and reading the second; then `command` in the guard's group, in
/// that order. A shell for each lets a test make either watcher's start fail. Both watchers start
/// first, so a watcher that cannot start fails the process before it runs, and the process never runs
/// unwatched. std's sockets are closed on exec, so neither the process nor anything it starts holds this
/// program's ends, which would keep a watcher waiting after the program had gone. On macOS std can only mark
/// a socket close-on-exec just after making it, so a process another thread of this program starts in that
/// instant holds a copy, and the watchers then wait for that process to end as well: a program whose
/// watchers must answer for it alone makes its groups while none of its other threads starts a process.
fn guarded(shells: [&str; 2], command: &mut Command, reads: [UnixStream; 2]) -> std::io::Result<(Child, Child, Child)> {
    let ([guard_shell, sweeper_shell], [guard_reads, sweeper_reads]) = (shells, reads);
    let mut guard = watcher(guard_shell, GUARD_NAME, GUARD, guard_reads)?;
    let group = guard.id();
    let mut sweeper = match watcher(sweeper_shell, SWEEPER_NAME, &sweeper_script(group), sweeper_reads) {
        Ok(sweeper) => sweeper,
        Err(error) => {
            stop_watcher(&mut guard);
            return Err(error);
        }
    };
    match i32::try_from(group).map_err(std::io::Error::other).and_then(|group| command.process_group(group).spawn()) {
        Ok(process) => Ok((process, guard, sweeper)),
        Err(error) => {
            stop_watcher(&mut sweeper);
            stop_watcher(&mut guard);
            Err(error)
        }
    }
}

impl Drop for Group {
    /// A process still running when its group is dropped is one its program gave up on, through an error
    /// or a panic, so the group is stopped. One that ended on its own leaves its group as a process that
    /// ends leaves it, and only its watchers go, the sweeper first ([`sweeper_script`]). Nothing here panics:
    /// a drop runs while a failure unwinds too.
    fn drop(&mut self) {
        if !matches!(self.process.try_wait(), Ok(Some(_))) {
            let _ = self.stop();
        }
        stop_watcher(&mut self.sweeper);
        stop_watcher(&mut self.guard);
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

    /// Start `script` in a guarded group, and once the group runs `count` live processes let go of this
    /// program's ends of its watchers' sockets, as a program that ends does: whether its watchers then ended
    /// the group. Whatever they left is killed, and the process and the guard are reaped, as the system reaps
    /// what an ended program started; the sweeper must then end by itself, the group being gone.
    fn ended_by_its_watchers(script: &str, count: usize) -> bool {
        let (held, reads) = sockets().unwrap();
        let (mut process, mut guard, mut sweeper) = guarded([WATCHER_SHELL; 2], Command::new("sh").args(["-c", script]), reads).unwrap();
        let group = guard.id();
        wait_for_processes(group, count, script);
        drop(held);
        let ended = poll_until(STUCK_AFTER, || !group_has_live_process(group).unwrap());
        if !ended {
            signal_group(group, "KILL").unwrap();
        }
        process.wait().unwrap();
        guard.wait().unwrap();
        let swept = poll_until(STUCK_AFTER, || sweeper.try_wait().unwrap().is_some());
        stop_watcher(&mut sweeper);
        assert!(swept, "{script}: the sweeper ran on after its group had gone");
        ended
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

    /// A guarded group is stopped whole, its guard with it, and the three processes this program started, the
    /// process and its two watchers, are reaped.
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
            assert!(group.sweeper.try_wait().unwrap().is_some(), "and its sweeper: {script}");
        }
    }

    /// A group whose program ends without stopping it, as one does when it is interrupted or killed, is
    /// ended by its guard and sweeper: the kernel closes the program's ends of the sockets they read, as
    /// letting go of them here does.
    #[test]
    fn a_group_whose_program_ends_is_ended_by_its_guard() {
        for (script, count) in outlasting_scripts() {
            assert!(ended_by_its_watchers(&script, count + 1), "its watchers did not end its group when its program let go: {script}");
        }
    }

    /// How many times the forking test's program ends: a kill lands inside a fork only some of the time.
    const FORKING_ENDS: usize = 50;

    /// A group whose program ends while its process forks is ended whole all the same: on macOS a kill sent
    /// while a process forks misses the child that fork makes, which a guard that killed once would leave
    /// running. The step forks child after child, each of which would outlast the test, and kills and reaps
    /// each before the next, and its program ends over and over, so the guard's kill lands inside a fork.
    #[test]
    fn a_group_whose_program_ends_while_its_process_forks_is_ended_whole() {
        // The child the shell starts first lives on, so the group's count shows when the forking has begun:
        // each later child lives too briefly to be counted.
        let script = format!("sleep {OUTLASTING} & while :; do sleep {OUTLASTING} & p=$!; kill -s KILL $p; wait $p; done");
        for end in 1..=FORKING_ENDS {
            // The guard, the shell and its first child; the sweeper is in a group of its own.
            assert!(ended_by_its_watchers(&script, 3), "end {end}: the watchers left a process of their group running when its program let go while it forked");
        }
    }

    /// Start a group, kill the watcher `pick` names by its id while the process runs, and answer what the
    /// group's `try_wait` then said and the state `ps` then gave the guard. The group must still stop whole.
    fn told_of_a_killed_watcher(pick: fn(&Group) -> u32) -> (String, String) {
        let script = format!("sleep {OUTLASTING}; true");
        let mut group = Group::spawn(Command::new("sh").args(["-c", &script])).unwrap();
        let id = group.id();
        wait_for_processes(id, 3, &script);
        assert!(group.try_wait().unwrap().is_none(), "watchers that wait are no error");
        let killed = output_of(Command::new("kill").args(["-s", "KILL", &pick(&group).to_string()])).unwrap();
        assert!(killed.status.success(), "the watcher alone was killed");
        let mut told = String::new();
        poll_until(STUCK_AFTER, || {
            told = group.try_wait().err().map(|error| error.to_string()).unwrap_or_default();
            !told.is_empty()
        });
        let guard_state = state_of(id);
        group.stop().unwrap();
        assert!(!group_has_live_process(id).unwrap(), "a process of the group is still alive");
        assert!(group.try_wait().unwrap().is_some(), "the process was reaped");
        (told, guard_state)
    }

    /// A guard killed while its process runs is told of, not left: a group whose watchers no longer both run
    /// is stopped rather than trusted to the one left. The group is whole still, and stopping it ends it.
    #[test]
    fn a_guard_that_ends_before_its_process_is_an_error_and_the_group_still_stops() {
        let (told, guard_state) = told_of_a_killed_watcher(Group::id);
        assert_eq!(told, "the process group's guard has gone while its process still runs");
        assert!(
            guard_state.starts_with('Z'),
            "the guard was reaped before its group had ended, freeing the id every kill of the group goes to: {guard_state:?}"
        );
    }

    /// A sweeper killed while its process runs is told of too: the guard's one kill alone could leave a
    /// process of the group running. The group is whole still, and stopping it ends it.
    #[test]
    fn a_sweeper_that_ends_before_its_process_is_an_error_and_the_group_still_stops() {
        let (told, _) = told_of_a_killed_watcher(|group| group.sweeper.id());
        assert_eq!(told, "the process group's sweeper has gone while its process still runs");
    }

    /// A watcher that cannot start fails its group before the process is started: the process never runs
    /// unwatched, the error names the watcher and its shell, and no watcher is left waiting, the guard
    /// started before a sweeper that could not start included.
    #[test]
    fn a_watcher_that_cannot_start_fails_the_group_before_its_process_runs() {
        let missing = "/craftbridge-no-such-shell";
        for (shells, name) in [([missing, WATCHER_SHELL], GUARD_NAME), ([WATCHER_SHELL, missing], SWEEPER_NAME)] {
            // What the process writes is read to the end of the pipe once the command, which holds its own end, is
            // dropped: a process that was started has written by then, and one never started has not, with no
            // wait for either.
            let (mut written, writes) = std::io::pipe().unwrap();
            let (held, reads) = sockets().unwrap();
            for end in &held {
                end.set_nonblocking(true).unwrap();
            }
            let mut command = Command::new("sh");
            command.args(["-c", "echo ran"]).stdout(writes);
            let error = guarded(shells, &mut command, reads).err().map(|error| error.to_string()).unwrap_or_default();
            drop(command);
            let mut ran = String::new();
            written.read_to_string(&mut ran).unwrap();
            let waiting = !poll_until(STUCK_AFTER, || held.iter().all(|end| has_gone(end).unwrap()));
            assert!(error.starts_with(&format!("could not start its process group's {name}, {missing}: ")), "{error}");
            assert_eq!(ran, "", "the process ran although its {name} never started");
            assert!(!waiting, "a watcher was left waiting after the {name} could not start");
        }
    }

    /// A step that cannot start leaves neither watcher waiting behind it. This program's ends of their sockets
    /// stay open here, so nothing but a kill of each, by its own id, can end them: the error may only answer
    /// once both have gone, the way the drop ends the watchers of a group whose process ended, or a guard left
    /// waiting holds a group's id over a group with nothing in it, and a sweeper left waiting holds a kill
    /// for that id once this program ends.
    #[test]
    fn a_step_that_cannot_start_leaves_no_watcher_waiting() {
        let (held, reads) = sockets().unwrap();
        for end in &held {
            end.set_nonblocking(true).unwrap();
        }
        let mut command = Command::new("/craftbridge-no-such-binary");
        assert!(guarded([WATCHER_SHELL; 2], &mut command, reads).is_err(), "a step that cannot start is an error");
        let gone = poll_until(STUCK_AFTER, || held.iter().all(|end| has_gone(end).unwrap()));
        assert!(gone, "a watcher was still waiting after the step that could not start had failed: its end of the socket never closed");
    }

    /// A group whose process ends on its own is left as the process that ends leaves it: what the step
    /// started in the group runs on, for its own program to answer for, and the drop takes only the
    /// watchers. The runner answers for a step it cut short; for one that finished on its own it kills
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

    /// A group that ends leaves no watcher once it is dropped: one whose process ended on its own, whose
    /// watchers are killed by their own ids and reaped, and one whose every process was killed from outside,
    /// which answers that kill as the process's end.
    #[test]
    fn a_group_that_ends_leaves_no_watcher() {
        let mut ended = Group::spawn(&mut Command::new("true")).unwrap();
        let (id, sweeper) = (ended.id(), ended.sweeper.id());
        assert!(poll_until(STUCK_AFTER, || ended.try_wait().unwrap().is_some_and(|status| status.success())), "the process never ended");
        drop(ended);
        assert_eq!(state_of(id), "", "the guard of a group whose process ended was left behind");
        assert_eq!(state_of(sweeper), "", "the sweeper of a group whose process ended was left behind");
        let script = format!("sleep {OUTLASTING}; true");
        let mut killed = Group::spawn(Command::new("sh").args(["-c", &script])).unwrap();
        let (id, sweeper) = (killed.id(), killed.sweeper.id());
        wait_for_processes(id, 3, &script);
        signal_group(id, "KILL").unwrap();
        let answered = poll_until(STUCK_AFTER, || killed.try_wait().is_ok_and(|status| status.is_some_and(|status| !status.success())));
        drop(killed);
        assert!(answered, "a group killed from outside never answered the kill as its process's end");
        assert_eq!(state_of(id), "", "the guard of a group killed from outside was left unreaped");
        assert_eq!(state_of(sweeper), "", "the sweeper of a group killed from outside was left behind");
        assert!(!group_has_live_process(id).unwrap(), "a process of the group is still alive");
    }

    /// A group already gone is stopped without complaint: a leader that exited and was not yet reaped, and
    /// a guarded group whose process has ended and been reaped, its guard keeping the group's id its own.
    #[test]
    fn stopping_a_group_that_has_already_ended_is_no_failure() {
        let mut ended = leader("true", &[]);
        // A stop sent before the leader has exited kills it, which this would read as no end of its own.
        assert!(poll_until(STUCK_AFTER, || state_of(ended.id()).starts_with('Z')), "gave up waiting for the leader to exit");
        stop_group(&mut ended).unwrap();
        assert!(ended.try_wait().unwrap().is_some_and(|status| status.success()), "the leader ended on its own and was reaped");
        let mut group = Group::spawn(&mut Command::new("true")).unwrap();
        assert!(poll_until(STUCK_AFTER, || group.try_wait().unwrap().is_some()), "the guarded process never ended");
        group.stop().unwrap();
        group.stop().unwrap();
        assert!(!group_has_live_process(group.id()).unwrap(), "the guard went with the stop");
    }
}
