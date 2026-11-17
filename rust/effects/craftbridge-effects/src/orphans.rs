//! The engine start's sweep of the temporary files a killed write left (AD21).
//!
//! EffectCraft's atomic write, which every file the session's file services write goes through, puts
//! the content in a temporary file beside the name it is renamed over, `.effectcraft-<pid>-<n>.tmp`,
//! so no such file is ever half-written under its own name (`crates/engine/src/settings/config.rs` at
//! the pin): a kill before the rename leaves that file alone, and its number is drawn as the write
//! starts, so no journal could list its name beforehand — the pending-export journal the core deals
//! with at start (`craftbridge_core::journal`) cannot name it. This is the other half of that same
//! start: the sweep takes the leftovers the journal cannot, in the folders the engine writes.
//!
//! The sweep touches what a killed EffectCraft write alone could have left, and nothing else:
//!
//! * the name is EffectCraft's own naming rule whole — `.effectcraft-`, a decimal number spelled as
//!   a number prints (the writing process's id), a `-`, another such number (the write's number),
//!   `.tmp` — which is exactly what printing a `u32` id and a `u64` counter gives; a name outside
//!   that shape is not EffectCraft's file and stays;
//! * no process has the id in the name now. A process that has it may be the writer, still writing:
//!   this very process, or another engine's on the same folders — an app extension sharing the app's
//!   container, or the last launch still being torn down as this one starts, both of which the core's
//!   journal allows for — and a live write's file is no leftover.
//!   The system answers whether a process has an id through `kill` with no signal, which sends
//!   nothing: POSIX has it fail with `ESRCH` only when no process has the id, and a process the
//!   caller may not signal, such as another app's, still answers that it is there. An id the system
//!   has given a new process since the writer died, as iOS and Android do in time, reads as taken, so
//!   a reused id keeps a leftover for a later start. And the check and the removal never race a live
//!   write the other way either: a writer that dies just after the check found it only leaves its
//!   file for a later start, and a new process given a reused id cannot take a leftover's name in
//!   the time between the check and the removal, because the exclusive create gives no name already
//!   held; only a second start sweeping the same folders at the very moment the id is reused could
//!   slip a fresh file into a just-freed name, and there it is that fresh write's own rename that
//!   fails — an error it reports, never a half-published file;
//! * the sweep walks the one list of the engine's folders the core's [`Folders::working`] keeps,
//!   each into its subfolders, because a killed write's file sits beside the file it was writing,
//!   which may be deep inside one (a template import's media, say); a folder added to that list is
//!   swept without a second list anywhere. Nothing outside those folders is touched;
//! * only regular files: a folder or a symbolic link of that name is not a write's leftover (the
//!   exclusive create that makes the temporary file opens neither, and follows neither), and the
//!   walk neither steps into a symbolic link nor deletes one, so nothing outside the folders can be
//!   reached through them.
//!
//! Like the core's dealing with the journal, the sweep never fails the start: a folder it cannot
//! read, or a file it cannot remove, is left as it was and tried again at the next start.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use craftbridge_core::folders::Folders;
use rustix::io::Errno;
use rustix::process::{Pid, test_kill_process};

/// The two pieces EffectCraft spells around the writer's id and the write's number:
/// `.effectcraft-<pid>-<n>.tmp` (AD21).
const PREFIX: &str = ".effectcraft-";
const SUFFIX: &str = ".tmp";

/// Remove every temporary file a killed write of a process that has ended left in the engine's
/// folders.
pub(crate) fn sweep(folders: &Folders) {
    for (_, folder) in folders.working() {
        sweep_folder(folder);
    }
}

/// Sweep `root` and every folder under it, following no link out of them. The walk keeps the
/// folders left to visit in its own list rather than in the call stack, so no tree is too deep for
/// it; the list holds only folders the disk itself called folders, and a tree of those cannot cycle,
/// so the walk ends.
fn sweep_folder(root: &Path) {
    let mut walk: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(folder) = walk.pop() {
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        for entry in entries.filter_map(Result::ok) {
            // `file_type` asks the folder's own record and follows no link: only what the killed
            // write itself could be — a real folder to walk, or a real file to take — passes on.
            let Ok(file_type) = entry.file_type() else { continue };
            if file_type.is_dir() {
                walk.push(entry.path());
            } else if file_type.is_file() && left_by_a_process_that_has_ended(&entry.file_name()) {
                // Best effort: a file the disk will not give up now is not what the start waits
                // for, and the next start will try it again.
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Whether a file name is the name a killed EffectCraft write would leave: the naming rule of AD21,
/// with a process id no process has now.
fn left_by_a_process_that_has_ended(name: &OsStr) -> bool {
    // A name that is not UTF-8 spells no number, so it is not EffectCraft's own shape and stays.
    let Some(name) = name.to_str() else { return false };
    let Some(both) = name.strip_prefix(PREFIX).and_then(|rest| rest.strip_suffix(SUFFIX)) else { return false };
    // The id and the number are digits with no `-` in either, so a name of the shape holds exactly
    // one `-` between them; anything with more is of no such name.
    let Some((id, number)) = both.split_once('-') else { return false };
    is_a_number(id) && is_a_number(number) && number.parse::<u64>().is_ok() && id.parse::<u32>().is_ok_and(no_process_has)
}

/// Whether `part` is a whole decimal number as it prints: digits throughout, and a leading zero
/// only for a lone `0`. EffectCraft prints its id and counter exactly so (AD21), and each parse of
/// the caller's line is what makes a number that is too big for the `u32` or the `u64` it printed
/// no such write either.
fn is_a_number(part: &str) -> bool {
    let digits = part.as_bytes();
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit) && (part.len() == 1 || !part.starts_with('0'))
}

/// Whether the system has no process of the id `id` now, which this process, being one, never is.
/// A process id is a positive `pid_t`, so `0` and an id past its range name no process the system can
/// be asked about, and their files stay: a negative `pid_t` would ask about a whole process group.
fn no_process_has(id: u32) -> bool {
    let Some(pid) = i32::try_from(id).ok().and_then(Pid::from_raw) else { return false };
    matches!(test_kill_process(pid), Err(Errno::SRCH))
}
