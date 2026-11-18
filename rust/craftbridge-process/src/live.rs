//! Which processes of a group are alive, read from the platform's own record of its processes and never
//! from the wording of a tool: a process that has exited and not been reaped is a zombie, holds its place
//! in the group and takes no work, so it is not alive.

#[cfg(any(target_os = "linux", test))]
use std::path::Path;

/// `ESRCH`, which the read of a process's `stat` answers when the process was reaped after the file was opened.
#[cfg(any(target_os = "linux", test))]
const NO_SUCH_PROCESS: i32 = 3;

/// The ids of the processes of the group `group` that have not yet exited, in no particular order, read
/// from the kernel's table of processes, `/proc`.
#[cfg(target_os = "linux")]
pub(crate) fn live_processes(group: u32) -> Result<Vec<u32>, String> {
    live_processes_in(Path::new("/proc"), group)
}

/// [`live_processes`] reading the table of processes kept in `table`, a folder of one folder for each process,
/// named by its id and holding that process's `stat`.
#[cfg(any(target_os = "linux", test))]
fn live_processes_in(table: &Path, group: u32) -> Result<Vec<u32>, String> {
    // A process reaped while the table is read is no longer in it, which is no failure: its folder has gone, or
    // the `stat` opened before it was reaped answers `ESRCH`.
    let gone = |error: &std::io::Error| error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(NO_SUCH_PROCESS);
    let unread = |error: std::io::Error| format!("cannot read {}: {error}", table.display());
    let mut live = Vec::new();
    for entry in std::fs::read_dir(table).map_err(unread)? {
        let entry = entry.map_err(unread)?;
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        let path = entry.path().join("stat");
        // Read as bytes: a process's name is whatever bytes it was given, of which the kernel keeps 15, a cut that
        // can fall inside a character, and no other process's name may stop the table being read.
        let text = match std::fs::read(&path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(error) if gone(&error) => continue,
            Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
        };
        if is_alive_in(group, &text).ok_or_else(|| format!("cannot read the state of process {pid} from {}", path.display()))? {
            live.push(pid);
        }
    }
    Ok(live)
}

/// The ids of the processes of the group `group` that have not yet exited, in no particular order. There is
/// no `/proc` here, and `pgrep -g` matches no zombie on this platform.
#[cfg(not(target_os = "linux"))]
pub(crate) fn live_processes(group: u32) -> Result<Vec<u32>, String> {
    let found = crate::command::output_of(std::process::Command::new("pgrep").args(["-g", &group.to_string()]))?;
    from_pgrep(group, found.status.code(), &String::from_utf8_lossy(&found.stdout), &String::from_utf8_lossy(&found.stderr))
}

/// What `pgrep -g group` answered: by its exit status alone whether it found any, 0 for a match, 1 for none,
/// and anything else (or no status, as for a signal) for a `pgrep` that could not tell, which judges nothing,
/// and by its output the ids of the matches, one a line.
#[cfg(any(not(target_os = "linux"), test))]
fn from_pgrep(group: u32, status: Option<i32>, stdout: &str, stderr: &str) -> Result<Vec<u32>, String> {
    match status {
        Some(0) => {
            stdout.lines().map(|line| line.trim().parse().map_err(|error| format!("pgrep named a process {line:?} of the group {group}: {error}"))).collect()
        }
        Some(1) => Ok(Vec::new()),
        _ => Err(format!("pgrep could not search the group {group}: {}", stderr.trim())),
    }
}

/// Is the process whose `/proc/<pid>/stat` line is `text` alive and in the group `group`? `None` for a line
/// that holds no state, group and number of threads. The line is `pid (name) state parent group …`, its
/// twentieth field the number of threads: the name sits in parentheses and may hold spaces and parentheses
/// itself, so the fields are read after the last `)`, which the kernel puts after the name. A process caught
/// as it leaves the table has its group as `-1`, which is why a group is signed here. A zombie, `Z`, has
/// exited and not been reaped, and `X` is dead, which a process is only for a moment before it leaves the
/// table. The state is the process's first thread's, which can end while others run on: the process has
/// exited only once it counts no thread but that one, and until then it is alive, as macOS's `pgrep` counts
/// it too.
#[cfg(any(target_os = "linux", test))]
fn is_alive_in(group: u32, text: &str) -> Option<bool> {
    let (_, fields) = text.rsplit_once(')')?;
    let mut fields = fields.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let _parent = fields.next()?;
    let process_group: i64 = fields.next()?.parse().ok()?;
    // The field after the group is the sixth, so the twentieth is the fifteenth after it.
    let threads: i64 = fields.nth(14)?.parse().ok()?;
    let exited = matches!(state, 'Z' | 'X' | 'x') && threads <= 1;
    Some(process_group == i64::from(group) && !exited)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `stat` line, its first twenty-two fields as the kernel writes them, of a process named `name`, in state
    /// `state`, in the group `group` and counting `threads` threads.
    fn line(pid: u32, name: &str, state: char, group: i64, threads: i64) -> String {
        format!("{pid} ({name}) {state} 1 {group} {group} 0 -1 4194560 635 0 0 0 0 0 0 0 20 0 {threads} 0 2138\n")
    }

    /// Lines the kernel wrote: a process whose first thread had ended while its second slept on, which `ps`
    /// shows as `Zl` in the group 7, and a zombie in the group 1.
    const FIRST_THREAD_ENDED: &str = "7 (python3) Z 1 7 1 0 -1 4227404 635 0 0 0 0 0 0 0 20 0 2 0 2138 0 0 18446744073709551615 0 0 0 0 0 0 0 16781312 2 0 0 0 17 3 0 0 0 0 0 0 0 0 0 0 0 0 0\n";
    const ZOMBIE: &str = "12 (python3) Z 1 1 1 0 -1 4227148 391 0 0 0 0 0 0 0 20 0 1 0 2239 0 0 18446744073709551615 0 0 0 0 0 0 0 16781312 2 1 0 0 17 4 0 0 0 0 0 0 0 0 0 0 0 0 0\n";

    /// A table of processes in a scratch folder, one `stat` for each `(pid, line)`.
    fn table(entries: &[(&str, Option<String>)]) -> tempfile_free::Scratch {
        let scratch = tempfile_free::Scratch::new();
        for (name, stat) in entries {
            let folder = scratch.path().join(name);
            std::fs::create_dir(&folder).unwrap();
            if let Some(stat) = stat {
                std::fs::write(folder.join("stat"), stat).unwrap();
            }
        }
        scratch
    }

    /// The crate takes no dependency, so a scratch folder is made and removed here.
    mod tempfile_free {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicUsize, Ordering};

        pub struct Scratch(PathBuf);

        impl Scratch {
            pub fn new() -> Self {
                static NEXT: AtomicUsize = AtomicUsize::new(0);
                let path = std::env::temp_dir().join(format!("craftbridge-process-table-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
                std::fs::create_dir(&path).unwrap();
                Self(path)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    /// Only a process that has not exited, in the group asked for, is alive; a name with spaces and
    /// parentheses in it, which a reader that split on spaces or stopped at the first `)` would take the wrong
    /// fields from, is read right. A process whose first thread has ended while another runs on has not
    /// exited, though its state is that thread's.
    #[test]
    fn a_stat_line_says_whether_its_process_is_alive_in_the_group() {
        assert_eq!(is_alive_in(777, &line(4242, "a (b) c", 'S', 777, 1)), Some(true));
        assert_eq!(is_alive_in(777, &line(4242, "sleep", 'S', 888, 1)), Some(false), "another group's process is no process of this one");
        for state in ['R', 'S', 'D', 'T', 't', 'I', 'W', 'K', 'P'] {
            assert_eq!(is_alive_in(777, &line(4242, "sleep", state, 777, 1)), Some(true), "{state}");
        }
        for state in ['Z', 'X', 'x'] {
            assert_eq!(is_alive_in(777, &line(4242, "sleep", state, 777, 1)), Some(false), "{state}");
            assert_eq!(is_alive_in(777, &line(4242, "threads", state, 777, 3)), Some(true), "{state} with threads running on");
        }
        assert_eq!(is_alive_in(7, FIRST_THREAD_ENDED), Some(true), "a process whose first thread ended while another runs is alive");
        assert_eq!(is_alive_in(8, FIRST_THREAD_ENDED), Some(false), "and in its own group alone");
        assert_eq!(is_alive_in(1, ZOMBIE), Some(false), "a zombie, whose only thread has ended, is not alive");
        assert_eq!(is_alive_in(777, &line(4242, "kill", 'X', -1, 0)), Some(false), "a process caught as it leaves the table has no group");
        let unreadable = ["no name here", "4242 (sleep) S 1", "4242 (sleep) S 1 not-a-number", "4242 (sleep)", "4242 (sleep) S 1 777 777 0 -1"];
        let no_count = line(4242, "sleep", 'S', 777, 1).replace(" 0 1 0 2138", " 0 many 0 2138");
        for unread in unreadable.iter().copied().chain([no_count.as_str()]) {
            assert_eq!(is_alive_in(777, unread), None, "{unread}");
        }
    }

    /// A table is read whole: the live processes of the group are named, a zombie and another group's
    /// process are not, and what is no process (a name that is no id, an entry whose `stat` has gone) is
    /// passed over.
    #[test]
    fn a_table_names_the_live_processes_of_the_group_and_passes_over_the_rest() {
        let scratch = table(&[
            ("101", Some(line(101, "sh", 'S', 100, 1))),
            ("102", Some(line(102, "sleep", 'R', 100, 1))),
            ("103", Some(line(103, "exited", 'Z', 100, 1))),
            ("104", Some(line(104, "other", 'S', 200, 1))),
            ("105", None),
            ("106", Some(line(106, "threads", 'Z', 100, 2))),
            ("self", Some(line(1, "not a process", 'S', 100, 1))),
        ]);
        let mut live = live_processes_in(scratch.path(), 100).unwrap();
        live.sort_unstable();
        assert_eq!(live, [101, 102, 106]);
        assert_eq!(live_processes_in(scratch.path(), 300), Ok(Vec::new()));
    }

    /// A process's name is whatever bytes it was given, of which the kernel keeps 15, here cutting a two-byte `ñ`
    /// in half: its line is read all the same, and so is every other process's.
    #[test]
    fn a_name_that_is_no_text_is_read_past() {
        let scratch = table(&[("101", Some(line(101, "sh", 'S', 100, 1))), ("102", None)]);
        let mut cut = line(102, "\u{f1}\u{f1}\u{f1}\u{f1}\u{f1}\u{f1}\u{f1}\u{f1}", 'S', 100, 1).into_bytes();
        // The eighth `ñ`'s second byte is the sixteenth of the name, which the kernel does not keep.
        cut.remove("102 (".len() + 15);
        assert!(String::from_utf8(cut.clone()).is_err(), "the cut name is still text");
        std::fs::write(scratch.path().join("102").join("stat"), cut).unwrap();
        let mut live = live_processes_in(scratch.path(), 100).unwrap();
        live.sort_unstable();
        assert_eq!(live, [101, 102]);
    }

    /// A process reaped between the opening of its `stat` and the reading of it, which `/proc` answers with
    /// `ESRCH`, has left the table, and is passed over: here the table's entry is a link to such a `stat`, which
    /// this program opened before it reaped the process and reads after.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_process_reaped_while_its_entry_is_read_is_passed_over() {
        use std::os::fd::AsRawFd;
        let mut reaped = std::process::Command::new("true").spawn().unwrap();
        let opened = std::fs::File::open(format!("/proc/{}/stat", reaped.id())).unwrap();
        reaped.wait().unwrap();
        let scratch = table(&[("101", Some(line(101, "sh", 'S', 100, 1))), ("102", None)]);
        let stat = scratch.path().join("102").join("stat");
        std::os::unix::fs::symlink(format!("/proc/self/fd/{}", opened.as_raw_fd()), &stat).unwrap();
        let read = std::fs::read(&stat).unwrap_err();
        assert_eq!(read.raw_os_error(), Some(NO_SUCH_PROCESS), "the reaped process's stat did not answer ESRCH: {read}");
        assert_eq!(live_processes_in(scratch.path(), 100), Ok(vec![101]));
    }

    /// A table nothing can be read from, or whose entry says nothing of its state, is an error in these
    /// words, not an answer about the group.
    #[test]
    fn a_table_that_cannot_be_read_is_an_error() {
        let scratch = table(&[("101", Some("not a stat line".to_string()))]);
        let stat = scratch.path().join("101").join("stat");
        assert_eq!(live_processes_in(scratch.path(), 100), Err(format!("cannot read the state of process 101 from {}", stat.display())));
        // A `stat` that is a folder cannot be read, and is no process that has gone.
        let unreadable = table(&[("102", None)]);
        let folder = unreadable.path().join("102").join("stat");
        std::fs::create_dir(&folder).unwrap();
        let error = live_processes_in(unreadable.path(), 100).unwrap_err();
        assert!(error.starts_with(&format!("cannot read {}: ", folder.display())), "{error}");
        let missing = scratch.path().join("no-such-table");
        let error = live_processes_in(&missing, 100).unwrap_err();
        assert!(error.starts_with(&format!("cannot read {}: ", missing.display())), "{error}");
    }

    /// `pgrep`'s answers are read by its exit status and its output's ids, and nothing it says in words.
    #[test]
    fn pgreps_answer_is_read_by_its_status_and_its_ids() {
        assert_eq!(from_pgrep(9, Some(0), "101\n102\n", ""), Ok(vec![101, 102]));
        assert_eq!(from_pgrep(9, Some(1), "", "whatever it says"), Ok(Vec::new()));
        assert_eq!(from_pgrep(9, Some(2), "", " pgrep: bad option\n"), Err("pgrep could not search the group 9: pgrep: bad option".to_string()));
        assert_eq!(from_pgrep(9, None, "", ""), Err("pgrep could not search the group 9: ".to_string()), "a pgrep ended by a signal could not tell");
        let unread = from_pgrep(9, Some(0), "101\nnot-an-id\n", "").unwrap_err();
        assert!(unread.starts_with("pgrep named a process \"not-an-id\" of the group 9: "), "{unread}");
    }
}
