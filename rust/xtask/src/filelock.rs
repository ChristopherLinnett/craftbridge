//! The system's file lock, taken in one place.
//!
//! A mutex holds within one process only, and two runs of the gate's checks or tests are two processes
//! that share folders on disk. The lock the system keeps on an open file does hold across processes, is
//! let go however its holder ends, and excludes a second open of the same file even inside one process, so
//! it is what every run that must take turns takes.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;

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
}
