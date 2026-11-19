//! The system's file lock, taken in one place.
//!
//! A mutex holds within one process only, and two runs of the gate's checks or tests are two processes
//! that share folders on disk. The lock the system keeps on an open file does hold across processes, is
//! let go however its holder ends, and excludes a second open of the same file even inside one process, so
//! it is what every run that must take turns takes.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

/// The variable naming the folder of the locks, which `tools/env.sh` sets for every run on the machine.
const FOLDER: &str = "CRAFTBRIDGE_SIMULATOR_LOCKS";

/// The folder `tools/env.sh` names for the locks.
pub(crate) fn folder() -> Result<PathBuf, String> {
    std::env::var_os(FOLDER)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| format!("{FOLDER} is not set to an absolute path: run through tools/env.sh, which names the folder where the runs on this machine take turns on its simulators and devices"))
}

/// The lock file of the machine's simulator `id`, its UDID: one per simulator, named after it, in the folder
/// `folder` — outside every checkout, so a run in one worktree sees the runs of another's.
pub(crate) fn for_id(locks: &Path, id: &str) -> PathBuf {
    locks.join(format!("{id}.lock"))
}

/// The lock file of the physical device `id`, in the same folder: named apart from every simulator's, so
/// that no name a device may be given, a simulator's UDID among them, makes a device's runs and a
/// simulator's wait on one lock.
pub(crate) fn for_device(locks: &Path, id: &str) -> PathBuf {
    for_id(locks, &format!("device-{id}"))
}

/// Whether somebody holds the lock at `path` now: the lock file exists and its lock is taken. Only a lock
/// tried answers one held apart from one free, since the file being open would exclude even this holder;
/// a caller found busy says so before waiting behind it, so the queue shows in the log.
pub(crate) fn held(path: &Path) -> Result<bool, String> {
    let Ok(held) = File::open(path) else { return Ok(false) };
    try_lock(&held).map_err(|error| format!("could not lock {}: {error}", path.display())).map(|free| !free)
}

/// Wait for the exclusive lock on `path`: on the folder itself when it is one, and otherwise on the file
/// there, made empty when it is missing. The lock is held for as long as the returned file is kept: bound
/// to `_` it would be let go at once.
pub(crate) fn hold(path: &Path) -> Result<File, String> {
    let file = if path.is_dir() { File::open(path) } else { OpenOptions::new().create(true).write(true).truncate(false).open(path) };
    file.and_then(|file| file.lock().map(|()| file)).map_err(|error| format!("could not lock {}: {error}", path.display()))
}

/// Take the exclusive lock on the open `file` if nobody holds it now: `false` when somebody does. Whoever
/// opened a lock file before its holder removed it and let it go can take the lock on a file that is no
/// longer where it was, so the caller that removes lock files checks its file is still there once it holds
/// the lock.
pub(crate) fn try_lock(file: &File) -> io::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;
    use std::time::Duration;

    /// How long a lock may outlast its holder: a child another test starts while the lock is held keeps a
    /// copy of it until the child's own program replaces this one, an instant a loaded machine stretches,
    /// and the lock is let go only once every copy is gone.
    const OUTLASTS: Duration = Duration::from_secs(600);

    /// While one holder has `path`, neither `try_lock` nor a second `hold` gets it, in this one process
    /// too, and once the first lets go the waiting `hold` gets it, and `try_lock` once that one has. A
    /// shared lock would let two holders in together and still refuse `try_lock`, so the second holder is
    /// what tells the two apart.
    fn takes_turns(path: &Path) {
        let first = hold(path).unwrap();
        let opened = File::open(path).unwrap();
        assert!(!try_lock(&opened).unwrap(), "{path:?}: somebody holds it, and `try_lock` took it");
        let (took, taken) = std::sync::mpsc::channel();
        let waiting = path.to_path_buf();
        let second = std::thread::spawn(move || {
            let held = hold(&waiting).unwrap();
            took.send(()).unwrap();
            held
        });
        assert!(taken.recv_timeout(Duration::from_millis(500)).is_err(), "{path:?}: a second holder took it while the first held it");
        drop(first);
        taken.recv_timeout(OUTLASTS).expect("the second holder takes it once the first lets go");
        drop(second.join().unwrap());
        let free = std::time::Instant::now();
        while !try_lock(&opened).unwrap() {
            assert!(free.elapsed() < OUTLASTS, "{path:?}: nobody holds it, and `try_lock` was refused");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_held_file_cannot_be_taken_until_it_is_let_go_even_in_one_process() {
        let scratch = Scratch::new("filelock");
        takes_turns(&scratch.0.join("held"));
    }

    #[test]
    fn a_folder_is_locked_as_a_file_is() {
        let scratch = Scratch::new("filelock-folder");
        takes_turns(&scratch.0);
    }

    /// `held` answers a lock held apart from one free and apart from no lock file at all, which is what
    /// a run says busy before it waits: `for_id` names that file, one per simulator, and `for_device` one per
    /// device, never a simulator's.
    #[test]
    fn held_knows_a_lock_held_from_one_free() {
        let scratch = Scratch::new("filelock-held");
        let marker = for_id(&scratch.0, "DEVICE");
        assert_eq!(marker.file_name().and_then(|name| name.to_str()), Some("DEVICE.lock"), "one lock file per device, named after it");
        let udid = "E33474EA-1897-4EF8-9DFB-DD3FFA178C8D";
        assert_eq!(for_device(&scratch.0, "DEVICE").file_name().and_then(|name| name.to_str()), Some("device-DEVICE.lock"));
        assert_ne!(for_device(&scratch.0, udid), for_id(&scratch.0, udid), "a device given a simulator's UDID never takes the simulator's lock");
        assert!(!held(&scratch.0.join("no-such")).unwrap(), "no lock file is held by nobody");
        let holding = hold(&marker).unwrap();
        assert!(held(&marker).unwrap(), "someone holds it");
        drop(holding);
        assert!(!held(&marker).unwrap(), "nobody holds it once the holder lets go");
    }
}
