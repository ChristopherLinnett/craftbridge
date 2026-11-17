//! The pending-export journal: what lets the next start delete the half-written files of an export
//! the system killed (plan §16, D-17).
//!
//! EffectCraft's exporter removes the files of an export only when it is cancelled cleanly; an app
//! the system kills mid-export leaves the partial output behind. So while an export runs, the app
//! lists in `pending-exports.json`, in the config folder, every file it is about to write, before it
//! writes the first (`CraftApp::request_with_progress`), and the engine clears them when the export
//! ends, whatever its outcome. At the next start each file the journal still lists is dealt with as
//! the clean cancel the kill prevented would have dealt with it, and the journal is cleared.
//!
//! These rules keep this from doing harm:
//!
//! * **Every write is atomic.** The journal is written to a temporary file in its own folder and
//!   renamed over the old one, so a kill mid-write leaves the old journal or the new one, never half
//!   of either.
//! * **Only what the export wrote is deleted.** Each entry keeps the size and modification time of
//!   the file that was at its path when it was listed, if one was. A start deletes a listed file only
//!   when it is not that file, so a finished file the export was about to replace but never reached
//!   (a later item of a queue, the rest of an older sequence, a kill between listing and writing)
//!   stays as it was, as a clean cancel leaves it.
//! * **Only regular files inside the container are touched.** Entries are stored relative to the
//!   container, because the absolute path of an iOS app's container can change between one launch
//!   and the next (an update moves it). A start joins each entry to the container it has now,
//!   resolves the entry's folder and refuses one outside the container, and looks at the file itself
//!   without following a link, so a folder, a link and everything outside the container are left
//!   alone and reported. Apple's File System Programming Guide calls a path-based URL "potentially
//!   fragile between launches" (*Accessing Files and Directories*), and an Apple engineer answers on
//!   the developer forums (thread 92693) that the path to an app's container, "and hence everything
//!   inside" it, "can change", with the system making sure the app is not running when it does, and
//!   tells apps to store paths relative to the container's root.
//! * **A journal that cannot be read is an answer, not a crash.** One whose content is no journal
//!   names no file, so nothing is deleted, and the start replaces it and says so. One the disk will
//!   not read when a process first starts on it is a killed run's, so it is renamed, unread, to a
//!   file of its own beside it (the start says so and why): its entries are not lost and no export is
//!   refused for it. Every later start tries each such file again, deals with what it lists once it
//!   reads, removes it and says so. At most [`MAX_SET_ASIDE`] wait at once; past that the unread
//!   journal stays where it is, and each start deals with nothing and each listing is refused, naming
//!   it, until one can read it.
//! * **The journal is bounded** by [`MAX_JOURNAL_BYTES`], so a damaged or hostile file cannot run the
//!   phone out of memory at start; an export whose list would grow past it is refused.
//!
//! Engines of one process share the journal, under one process-wide lock, and a start never deletes a
//! file another engine of the process is writing now.
//!
//! Nothing but this would keep a second process out of the journal (an app extension sharing the
//! app's container, a launch while the last one is still being torn down), and a start cannot tell
//! another process's running export from a killed one's. So a process holds an exclusive lock on
//! [`LOCK_FILE`], beside the journal, while a start of its own deals with the journal and while
//! exports of its own run with it: from the first listing until the last of them clears. The system
//! releases the lock when a process ends, however it ends. A start that finds another process holding
//! it deals with nothing and says so; a listing that does is refused, so its export does not run; each
//! later start and listing tries again. The lock is never held longer, because iOS terminates a
//! suspended app that holds a file lock outside its own data container, as in an App Group container a
//! host may share with an extension (Apple's developer forums, thread 655225: "You must release file
//! locks before coming eligible for suspension"), and an app keeps a running export from suspension
//! with a background task.
//!
//! While it holds the lock, a process knows what the journal holds, the entries it keeps for a start
//! and those of its running exports, and every listing and clear writes that without reading the file.
//! A journal the disk stops reading mid-run is therefore replaced whole, and never set aside with a
//! running export's entries that its clear could not reach. When the process takes the lock it reads
//! the journal, which then holds no running export's entries, since a process that runs one holds the
//! lock: so a journal the disk will not read can be set aside then, and what it lists dealt with by a
//! later start, without touching a file a running export writes.

use crate::Result;
use crate::engine::lock;
use crate::error::CraftError;
use crate::folders::{Confinement, Folders, entry_in, resolved, written_file};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::collections::btree_map::Entry as Slot;
use std::fs::{self, File, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;

/// The journal's file name, in the app's config folder.
pub const JOURNAL_FILE: &str = "pending-exports.json";

/// The most bytes the journal holds, read whole at start: hundreds of thousands of entries, since an
/// entry is a short path below the container.
pub const MAX_JOURNAL_BYTES: u64 = 16 << 20;

/// The key of the list of entries in the journal's JSON object.
const OUTPUTS: &str = "outputs";
/// An entry's path below the container.
const PATH: &str = "path";
/// The size of the file at the entry's path when it was listed; absent when there was none.
const BYTES: &str = "bytes";
/// That file's modification time, in nanoseconds since the Unix epoch, when the disk gives one.
const MODIFIED: &str = "modified";

/// What a temporary file of the journal's atomic write starts with, so a start can sweep the ones a
/// killed write left behind and nothing else.
const TEMP_PREFIX: &str = ".pending-exports-";

/// The file, beside the journal, whose exclusive lock a process holds while it deals with the journal
/// or runs exports with it. It is never deleted, and holds nothing: the lock is the content.
pub const LOCK_FILE: &str = "pending-exports.lock";

/// What the name of a journal the disk would not read starts with once it is set aside, so a later
/// start finds it. It must not start with [`TEMP_PREFIX`], whose files a start deletes.
const SHELF_PREFIX: &str = "pending-exports-unread-";

/// The most journals set aside at once: each later start reads every one, and a disk that keeps
/// refusing to read new journals would otherwise fill the config folder with them.
pub const MAX_SET_ASIDE: usize = 8;

/// Tells the names of two journals set aside in one instant apart.
static SHELVED: AtomicU64 = AtomicU64::new(0);

/// What this process knows each journal holds, by journal file, while exports of its own run with it.
/// The lock serialises every change of every journal in the process.
static KNOWN: Mutex<BTreeMap<PathBuf, Known>> = Mutex::new(BTreeMap::new());

/// What one journal holds, as this process wrote it while holding it.
#[derive(Debug, Default)]
struct Known {
    /// Entries no export runs for now (files a start could not delete, and what a killed run left),
    /// kept for a start to deal with.
    held: Vec<Entry>,
    /// The entries of the exports running in this process now.
    running: Vec<Entry>,
    /// The lock on [`LOCK_FILE`], which keeps every other process from the journal while this one
    /// holds it; none on a disk that has no locks, where this process is the one it is used by.
    lock: Option<File>,
}

impl Known {
    /// Every entry the journal holds.
    fn entries(&self) -> Vec<Entry> {
        self.held.iter().chain(&self.running).cloned().collect()
    }
}

/// What a start did about the journal it found: the answer to each way it can disagree with the
/// disk. The files are named where they are now, inside the container.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recovery {
    /// Listed files the killed exports wrote, now deleted.
    pub removed: Vec<PathBuf>,
    /// Listed files still as they were when listed: the export never reached them, so they are left.
    pub untouched: Vec<PathBuf>,
    /// Listed paths the start refused to touch, with why: one outside the container, a folder or a
    /// link. They are dropped from the journal, since no later start would act on them.
    pub refused: Vec<(PathBuf, String)>,
    /// Listed files that could not be deleted, with why. They stay listed, so the next start tries
    /// again.
    pub failed: Vec<(PathBuf, String)>,
    /// What the start found about the journal files themselves: one it could not read, set aside or
    /// clear, each journal an earlier start set aside that is still unread, and each it read now,
    /// dealt with and removed.
    pub journal: Vec<String>,
}

/// The journal of one app: where it is, and the container its entries lie in.
#[derive(Debug, Clone)]
pub(crate) struct Journal {
    file: PathBuf,
    /// The container as the operating system resolves it: every entry is a path below it.
    root: PathBuf,
}

/// The entries one export has listed, cleared with [`Pending::clear`] when the export ends.
#[derive(Debug)]
pub(crate) struct Pending {
    journal: Journal,
    listed: Vec<Entry>,
}

/// One listed file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// Where the file is below the container, as the export's writer opens it.
    path: String,
    /// The file that was there when the entry was listed, or none.
    found: Option<Stamp>,
}

/// What tells the file that was at a path when it was listed from one written there since: writing
/// a file, or replacing it, changes its modification time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    bytes: u64,
    modified: Option<u64>,
}

/// Why the journal was not read.
enum Unread {
    /// What is there is no journal: it names nothing a start can act on.
    Damaged(String),
    /// The disk would not give it up now; a later start may read it.
    Disk(String),
}

/// What taking the journal's lock came to.
enum Locked {
    /// This process holds it now.
    Taken(File),
    /// Another live process holds it: why.
    Elsewhere(String),
    /// The disk gave no answer: why.
    Unsupported(String),
}

/// What a start did with one listed file.
enum Settled {
    /// Nothing is at its path.
    Absent,
    /// It is the file that was there when it was listed, so it is left.
    Untouched,
    /// The export wrote it, and it is deleted.
    Removed,
}

/// Why a listed path was not dealt with.
enum Refusal {
    /// It is not a regular file, which is all an export writes: what it is instead.
    NotAFile(&'static str),
    /// The disk refused.
    Disk(String),
}

impl Journal {
    /// The journal of `folders`, whose config folder `confinement` (from [`Folders::prepare`]) has
    /// been made. The file's folder and the container are resolved once, so every engine of the app
    /// names the one file and the same entries however its folders were spelt.
    pub(crate) fn new(folders: &Folders, confinement: &Confinement) -> Result<Self> {
        let file = resolved("config folder", &folders.config)?.join(JOURNAL_FILE);
        Ok(Self { file, root: confinement.resolved_write_root()? })
    }

    /// The start's step: deal with every file the journal and the journals set aside still list, then
    /// clear them. Never fails: a problem is a line of the answer, and the start goes on.
    pub(crate) fn recover(&self) -> Recovery {
        let mut known = lock(&KNOWN);
        let mut recovery = Recovery::default();
        // While exports of this process run with the journal it holds it already, and knows what it
        // holds; otherwise the start takes it now and reads it.
        let (Known { held, running, lock }, taken_now) = match known.remove(&self.file) {
            Some(ours) => (ours, false),
            None => match self.take_lock() {
                Locked::Taken(file) => (Known { lock: Some(file), ..Known::default() }, true),
                // A disk with no locks cannot keep a second process out: the start goes on as the one
                // process there is.
                Locked::Unsupported(why) => {
                    recovery.journal.push(why);
                    (Known::default(), true)
                }
                // The process that holds it may be running an export, which a start cannot tell from a
                // killed one's, so nothing is touched.
                Locked::Elsewhere(why) => {
                    recovery.journal.push(format!("{why}; nothing is dealt with now, and the next start tries again"));
                    return recovery;
                }
            },
        };
        self.sweep_temporaries(&mut recovery);
        let mut stale = held;
        // The journals set aside earlier that are dealt with now, removed once what they list has been
        // dealt with and the journal is written, each with why it is no journal when it is not; and
        // how many still wait unread.
        let (mut read_now, mut waiting) = (Vec::new(), 0_usize);
        for shelf in self.shelves() {
            match self.read_at(&shelf) {
                Ok(listed) => {
                    stale.extend(listed);
                    read_now.push((shelf, None));
                }
                Err(Unread::Damaged(reason)) => read_now.push((shelf, Some(reason))),
                Err(Unread::Disk(reason)) => {
                    recovery.journal.push(format!("{reason}; it stays set aside for a later start"));
                    waiting = waiting.saturating_add(1);
                }
            }
        }
        if taken_now {
            match self.take_entries(waiting) {
                Ok((listed, note)) => {
                    stale.extend(listed);
                    recovery.journal.extend(note);
                }
                // Writing the journal now would replace what nobody has read, so nothing is dealt with
                // or written; the lock goes as the start returns.
                Err(why) => {
                    recovery.journal.push(format!("{why}, so nothing is dealt with now, and each later start and listing tries again"));
                    for (_, damaged) in read_now {
                        if let Some(reason) = damaged {
                            recovery.journal.push(format!("{reason}; it names no file to delete, and goes when a start can write the journal"));
                        }
                    }
                    return recovery;
                }
            }
        }
        let mut held = Vec::new();
        for entry in stale {
            // A file another engine of this process is writing now belongs to that export, not to a
            // killed one, whatever an older entry says.
            if running.iter().any(|ours| ours.path == entry.path) {
                continue;
            }
            let target = match entry_in(&self.root, Path::new(&entry.path)) {
                Ok(target) => target,
                Err(refusal) => {
                    recovery.refused.push((PathBuf::from(entry.path), refusal.to_string()));
                    continue;
                }
            };
            match settle(&target, entry.found) {
                Ok(Settled::Absent) => {}
                Ok(Settled::Untouched) => recovery.untouched.push(target),
                Ok(Settled::Removed) => recovery.removed.push(target),
                Err(Refusal::NotAFile(what)) => recovery.refused.push((target, format!("it is {what}, not a file an export writes"))),
                Err(Refusal::Disk(reason)) => {
                    recovery.failed.push((target, reason));
                    held.push(entry);
                }
            }
        }
        let state = Known { held, running, lock };
        match self.write(&state.entries()) {
            // The set-aside journals stay for a start that can write the journal, since what they
            // list is held nowhere else.
            Err(error) => {
                recovery.journal.push(format!("cannot clear the pending-export journal {}: {error}", self.file.display()));
                for (_, damaged) in read_now {
                    if let Some(reason) = damaged {
                        recovery.journal.push(format!("{reason}; it names no file to delete, and goes when a start can write the journal"));
                    }
                }
            }
            // What the set-aside journals listed is dealt with now, or held in the journal.
            Ok(()) => {
                for (shelf, damaged) in read_now {
                    match (fs::remove_file(&shelf), damaged) {
                        (Ok(()), None) => {
                            recovery.journal.push(format!("read the set-aside journal {}, dealt with what it listed and removed it", shelf.display()))
                        }
                        (Ok(()), Some(reason)) => recovery.journal.push(format!("{reason}; it names no file to delete, so it is removed")),
                        (Err(error), _) => recovery.journal.push(format!("cannot remove the set-aside journal {}: {error}", shelf.display())),
                    }
                }
            }
        }
        // With no export of this process running, the lock goes, and with it what the process knows:
        // another process may take the journal now, and this one reads it again when it next does.
        if !state.running.is_empty() {
            known.insert(self.file.clone(), state);
        }
        recovery
    }

    /// What the journal holds, read by a process that has just taken it, when no export of any process
    /// runs with it: its entries, with a line when it held no journal and is replaced or the disk would
    /// not read it and it is set aside, which is safe now, since no running export's entries are in it.
    /// Past [`MAX_SET_ASIDE`], with `waiting` set aside already, it stays, and the answer is why nothing
    /// may be written over it.
    fn take_entries(&self, waiting: usize) -> std::result::Result<(Vec<Entry>, Option<String>), String> {
        match self.read_at(&self.file) {
            Ok(listed) => Ok((listed, None)),
            Err(Unread::Damaged(reason)) => Ok((Vec::new(), Some(format!("{reason}; it names no file to delete, so it is replaced")))),
            Err(Unread::Disk(reason)) => {
                let set_aside = if waiting >= MAX_SET_ASIDE {
                    Err(format!("{waiting} journals already wait set aside, the most kept"))
                } else {
                    self.shelve().map_err(|error| error.to_string())
                };
                match set_aside {
                    Ok(shelf) => Ok((Vec::new(), Some(format!("{reason}; it is set aside as {} for a later start", shelf.display())))),
                    Err(why) => Err(format!("{reason}; and it cannot be set aside ({why})")),
                }
            }
        }
    }

    /// This process's hold on the journal, for a listing or a clear: what it knows the journal holds
    /// while exports of its own run with it, or else the journal taken now, its lock and what it holds,
    /// kept for a start to deal with.
    fn hold<'k>(&self, known: &'k mut BTreeMap<PathBuf, Known>) -> Result<&'k mut Known> {
        match known.entry(self.file.clone()) {
            Slot::Occupied(ours) => Ok(ours.into_mut()),
            Slot::Vacant(slot) => {
                let lock = match self.take_lock() {
                    Locked::Taken(file) => Some(file),
                    Locked::Unsupported(_) => None,
                    Locked::Elsewhere(why) => return Err(CraftError::io(format!("{why}; nothing can be listed while it does"))),
                };
                let (held, _) =
                    self.take_entries(self.shelves().len()).map_err(|why| CraftError::io(format!("{why}, so nothing can be listed until a start reads it")))?;
                Ok(slot.insert(Known { held, running: Vec::new(), lock }))
            }
        }
    }

    /// Take the exclusive lock that keeps every other process from the journal while this one deals with
    /// it or runs exports with it.
    fn take_lock(&self) -> Locked {
        let path = self.file.with_file_name(LOCK_FILE);
        let unsupported = |error: &dyn std::fmt::Display| {
            Locked::Unsupported(format!("cannot lock {}, so no other process is kept from the journal: {error}", path.display()))
        };
        let file = match open_lock(&path) {
            Ok(file) => file,
            Err(error) => return unsupported(&error),
        };
        match file.try_lock() {
            Ok(()) => Locked::Taken(file),
            Err(TryLockError::WouldBlock) => {
                Locked::Elsewhere(format!("another process holds the lock on {}, so it may be running exports of its own", path.display()))
            }
            Err(TryLockError::Error(error)) => unsupported(&error),
        }
    }

    /// An export's entry, empty until the export lists the files it is about to write.
    pub(crate) fn pending(&self) -> Pending {
        Pending { journal: self.clone(), listed: Vec::new() }
    }

    /// The entry of `output`, a file an export is about to write: where its writer's open lands,
    /// below the container, and the file that is there now, if any. Refused when that is outside the
    /// container, or is anything but a file or nothing: a start would refuse to touch it.
    fn entry_of(&self, output: &Path) -> Result<Entry> {
        let below = written_file(&self.root, output)?;
        let path = below.to_str().ok_or_else(|| {
            CraftError::io(format!("the output file {} cannot be listed in the pending-export journal: its path is not UTF-8", output.display()))
        })?;
        let target = self.root.join(&below);
        let found = match fs::symlink_metadata(&target) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(CraftError::io(format!("cannot look at the output file {}: {error}", target.display()))),
            Ok(found) if found.is_file() => Some(Stamp::of(&found)),
            // A link the resolution could not follow points at nothing yet: the writer would create
            // its target wherever it points.
            Ok(_) => return Err(CraftError::io(format!("the output file {} is a folder or a link, not a file an export writes", target.display()))),
        };
        Ok(Entry { path: path.to_string(), found })
    }

    /// Rename the journal, unread, to a file of its own beside it, so a start that can read it deals
    /// with what it lists and no export waits on it. Answers where it went.
    fn shelve(&self) -> io::Result<PathBuf> {
        let folder = self.file.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the journal has no folder"))?;
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_nanos());
        // A rename replaces what it lands on, so the name is one nothing holds; the counter gives each
        // try a new one, and the tries are bounded so a disk that answers oddly cannot hold the start.
        for _ in 0..MAX_SET_ASIDE.saturating_mul(16) {
            let shelf = folder.join(format!("{SHELF_PREFIX}{nanos}-{}.json", SHELVED.fetch_add(1, Ordering::Relaxed)));
            if fs::symlink_metadata(&shelf).is_err() {
                fs::rename(&self.file, &shelf)?;
                return Ok(shelf);
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "every name tried for it is taken"))
    }

    /// The journals earlier starts set aside, in name order.
    fn shelves(&self) -> Vec<PathBuf> {
        let Some(folder) = self.file.parent() else { return Vec::new() };
        let Ok(names) = fs::read_dir(folder) else { return Vec::new() };
        let mut shelves: Vec<PathBuf> =
            names.flatten().filter(|entry| entry.file_name().to_string_lossy().starts_with(SHELF_PREFIX)).map(|entry| entry.path()).collect();
        shelves.sort();
        shelves
    }

    /// The entries of the journal at `file`. A missing file lists none.
    fn read_at(&self, file: &Path) -> std::result::Result<Vec<Entry>, Unread> {
        let disk = |error: io::Error| Unread::Disk(format!("cannot read the pending-export journal {}: {error}", file.display()));
        let bytes = match fs::File::open(file) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(disk(error)),
            Ok(file) => bounded(file).map_err(disk)?,
        };
        let damaged = |what: &str| Unread::Damaged(format!("the pending-export journal {} {what}", file.display()));
        if u64::try_from(bytes.len()).map_or(true, |length| length > MAX_JOURNAL_BYTES) {
            return Err(damaged(&format!("is longer than the {MAX_JOURNAL_BYTES} bytes a journal holds")));
        }
        let document: Value = serde_json::from_slice(&bytes).map_err(|error| damaged(&format!("is not JSON: {error}")))?;
        let outputs = document.get(OUTPUTS).and_then(Value::as_array).ok_or_else(|| damaged(&format!("has no `{OUTPUTS}` list")))?;
        outputs
            .iter()
            .map(|output| Entry::from_value(output).ok_or_else(|| damaged(&format!("lists an entry that is not a `{PATH}` with what was there: {output}"))))
            .collect()
    }

    /// Replace the journal with one listing `entries`, atomically; with none to list the file is
    /// removed, which reads as an empty journal.
    fn write(&self, entries: &[Entry]) -> io::Result<()> {
        if entries.is_empty() {
            return match fs::remove_file(&self.file) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        let document = Value::Object(Map::from_iter([(OUTPUTS.to_string(), Value::Array(entries.iter().map(Entry::to_value).collect()))]));
        let bytes = serde_json::to_vec(&document).map_err(io::Error::from)?;
        if u64::try_from(bytes.len()).map_or(true, |length| length > MAX_JOURNAL_BYTES) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("the journal would grow past the {MAX_JOURNAL_BYTES} bytes it holds")));
        }
        self.publish(self.stage(&bytes)?)
    }

    /// The first half of an atomic write: the new content, complete on disk in a temporary file of
    /// the journal's own folder (so the rename is within one filesystem), with the journal itself
    /// untouched.
    fn stage(&self, bytes: &[u8]) -> io::Result<NamedTempFile> {
        let folder = self.file.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the journal has no folder"))?;
        let mut staged = tempfile::Builder::new().prefix(TEMP_PREFIX).suffix(".tmp").tempfile_in(folder)?;
        staged.write_all(bytes)?;
        staged.as_file().sync_all()?;
        Ok(staged)
    }

    /// The second half: the rename that makes the staged content the journal in one step. A failed
    /// rename leaves the old journal whole, and the staged file is removed as it drops.
    fn publish(&self, staged: NamedTempFile) -> io::Result<()> {
        staged.persist(&self.file).map(drop).map_err(|failure| failure.error)
    }

    /// Delete the temporary files a write killed before its rename left in the journal's folder.
    fn sweep_temporaries(&self, recovery: &mut Recovery) {
        let Some(folder) = self.file.parent() else { return };
        let Ok(names) = fs::read_dir(folder) else { return };
        for entry in names.flatten() {
            if entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX)
                && let Err(error) = fs::remove_file(entry.path())
            {
                recovery.journal.push(format!("cannot remove the leftover {}: {error}", entry.path().display()));
            }
        }
    }
}

impl Pending {
    /// List `outputs`, files the export is about to write, before it writes them. Refused, so the
    /// export must not write them, when one lies outside the container or the journal cannot be
    /// written: an export that could be killed unlisted would leave partial files nothing deletes.
    pub(crate) fn list(&mut self, outputs: &[PathBuf]) -> Result<()> {
        let entries = outputs.iter().map(|output| self.journal.entry_of(output)).collect::<Result<Vec<_>>>()?;
        if entries.is_empty() {
            return Ok(());
        }
        let mut known = lock(&KNOWN);
        let state = self.journal.hold(&mut known)?;
        let mut listed = state.entries();
        listed.extend(entries.iter().cloned());
        let written = self.journal.write(&listed);
        if written.is_ok() {
            state.running.extend(entries.iter().cloned());
        }
        // A process with no export running holds no lock.
        if state.running.is_empty() {
            known.remove(&self.journal.file);
        }
        written.map_err(|error| {
            CraftError::io(format!("cannot list the export's files in the pending-export journal {}: {error}", self.journal.file.display()))
        })?;
        self.listed.extend(entries);
        Ok(())
    }

    /// Remove the export's entries, because it has ended. When they cannot be cleared they stay
    /// listed until the journal is next written, and a start before that deletes what the export
    /// wrote, finished or not, so the error says that and the caller must pass it on rather than
    /// answer success.
    pub(crate) fn clear(self) -> Result<()> {
        let Self { journal, listed } = self;
        if listed.is_empty() {
            return Ok(());
        }
        let mut known = lock(&KNOWN);
        let uncleared = |error: String| {
            let first = listed.first().map_or_else(PathBuf::new, |entry| journal.root.join(&entry.path));
            let more = listed.len().saturating_sub(1);
            CraftError::io(format!(
                "the export ended but its entries in the pending-export journal {} could not be cleared, so the next start may delete {}{}: copy what it wrote elsewhere first ({error})",
                journal.file.display(),
                first.display(),
                if more == 0 { String::new() } else { format!(" and {more} more") }
            ))
        };
        let state = journal.hold(&mut known).map_err(|error| uncleared(error.to_string()))?;
        for entry in &listed {
            take_one(&mut state.running, entry);
        }
        let written = journal.write(&state.entries());
        // The process's last export with the journal has ended: the lock goes, and another process may
        // take the journal.
        if state.running.is_empty() {
            known.remove(&journal.file);
        }
        written.map_err(|error| uncleared(error.to_string()))
    }
}

impl Entry {
    /// The entry a journal's JSON holds, or none for one that is not an entry.
    fn from_value(value: &Value) -> Option<Self> {
        let path = value.get(PATH)?.as_str()?.to_string();
        let found = match (value.get(BYTES), value.get(MODIFIED)) {
            (None, None) => None,
            (Some(bytes), modified) => {
                let modified = match modified {
                    None => None,
                    Some(modified) => Some(modified.as_u64()?),
                };
                Some(Stamp { bytes: bytes.as_u64()?, modified })
            }
            (None, Some(_)) => return None,
        };
        Some(Self { path, found })
    }

    fn to_value(&self) -> Value {
        let mut fields = Map::new();
        fields.insert(PATH.to_string(), Value::from(self.path.as_str()));
        if let Some(found) = self.found {
            fields.insert(BYTES.to_string(), Value::from(found.bytes));
            if let Some(modified) = found.modified {
                fields.insert(MODIFIED.to_string(), Value::from(modified));
            }
        }
        Value::Object(fields)
    }
}

impl Stamp {
    fn of(file: &fs::Metadata) -> Self {
        let modified = file.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).and_then(|since| u64::try_from(since.as_nanos()).ok());
        Self { bytes: file.len(), modified }
    }
}

/// Deal with the listed file at `target`, a path inside the container whose own name is not
/// followed: leave it when it is missing or is still the file `found` describes, delete it when
/// the export wrote it, and refuse anything that is not a regular file.
fn settle(target: &Path, found: Option<Stamp>) -> std::result::Result<Settled, Refusal> {
    let now = match fs::symlink_metadata(target) {
        // A folder of the path that is now a file holds nothing either.
        Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => return Ok(Settled::Absent),
        Err(error) => return Err(Refusal::Disk(error.to_string())),
        Ok(now) => now,
    };
    let kind = now.file_type();
    if kind.is_symlink() {
        return Err(Refusal::NotAFile("a symbolic link"));
    }
    if kind.is_dir() {
        return Err(Refusal::NotAFile("a folder"));
    }
    if !kind.is_file() {
        return Err(Refusal::NotAFile("neither a file nor a folder"));
    }
    if found == Some(Stamp::of(&now)) {
        return Ok(Settled::Untouched);
    }
    fs::remove_file(target).map(|()| Settled::Removed).map_err(|error| Refusal::Disk(error.to_string()))
}

/// The lock file at `path`, made if it is not there, whose content is never read or written.
fn open_lock(path: &Path) -> io::Result<File> {
    fs::OpenOptions::new().write(true).create(true).truncate(false).open(path)
}

/// What `reader` holds, read no further than one byte past [`MAX_JOURNAL_BYTES`]: enough to tell a
/// journal that is too long, without reading into memory a damaged one the size of the disk.
fn bounded(reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(MAX_JOURNAL_BYTES.saturating_add(1)).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Remove one entry equal to `entry` from `entries`: two exports may list the same file, and each
/// clears only its own listing of it.
fn take_one(entries: &mut Vec<Entry>, entry: &Entry) {
    let mut taken = false;
    entries.retain(|listed| {
        let take = !taken && listed == entry;
        taken |= take;
        !take
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::CraftApp;
    use crate::cancel::CancelToken;
    use crate::engine::Engine;
    use crate::testing::{Gate, scratch_folders};
    use serde_json::json;
    use std::sync::mpsc;
    use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard};
    use std::thread;
    use tempfile::TempDir;

    type Test = std::result::Result<(), Box<dyn std::error::Error>>;

    /// An export the test writes: it gets the progress callback and the lister an app's export gets.
    type Script = Box<dyn FnMut(&mut dyn FnMut(u64, u64, Option<f64>) -> bool, &mut dyn FnMut(&[PathBuf]) -> Result<()>) -> Result<Value> + Send>;

    /// An app whose export is the test's script.
    struct Scripted(Script);

    impl Scripted {
        /// An app whose start is all a test looks at.
        fn idle() -> Self {
            Self(Box::new(|_, _| Ok(Value::Null)))
        }
    }

    impl CraftApp for Scripted {
        fn request(&mut self, _method: &str, _params: Value) -> Result<Value> {
            Ok(Value::Null)
        }

        fn request_with_progress(
            &mut self,
            _method: &str,
            _params: Value,
            on_progress: &mut dyn FnMut(u64, u64, Option<f64>) -> bool,
            list_outputs: &mut dyn FnMut(&[PathBuf]) -> Result<()>,
        ) -> Result<Value> {
            (self.0)(on_progress, list_outputs)
        }
    }

    /// Held shared by every [`Sandbox`] while it lives, and exclusively while a test starts a process:
    /// a process started while another test holds the journal's lock keeps a copy of that lock until it
    /// runs its program, and would make that test's start meet a lock no process holds.
    static NO_PROCESS_STARTS: RwLock<()> = RwLock::new(());

    /// An app's folders under a scratch root, with `outside` a folder beside the container.
    struct Sandbox {
        root: TempDir,
        folders: Folders,
        outside: PathBuf,
        _alive: RwLockReadGuard<'static, ()>,
    }

    impl Sandbox {
        fn new() -> std::result::Result<Self, io::Error> {
            let root = TempDir::new()?;
            let sand = Self::at(root, "container")?;
            Ok(sand)
        }

        fn at(root: TempDir, container: &str) -> std::result::Result<Self, io::Error> {
            let outside = root.path().join("outside");
            fs::create_dir_all(&outside)?;
            let folders = scratch_folders(root.path().join(container));
            folders.prepare().map_err(|error| io::Error::other(error.to_string()))?;
            Ok(Self { root, folders, outside, _alive: NO_PROCESS_STARTS.read().unwrap_or_else(PoisonError::into_inner) })
        }

        fn journal(&self) -> Journal {
            Journal::new(&self.folders, &self.folders.prepare().unwrap()).unwrap()
        }

        fn file(&self) -> PathBuf {
            self.folders.config.join(JOURNAL_FILE)
        }

        /// A file in the container, with `bytes` in it.
        fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.folders.container.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            path
        }

        /// The journal a killed run left, with these entries as written.
        fn write_journal(&self, entries: &[Value]) {
            fs::write(self.file(), json!({ OUTPUTS: entries }).to_string()).unwrap();
        }

        fn start(&self) -> std::result::Result<Engine, CraftError> {
            Engine::start_in(&self.folders, Scripted::idle())
        }

        /// The resolved place of `name` in the container, as a recovery reports it.
        fn real(&self, name: &str) -> PathBuf {
            fs::canonicalize(&self.folders.container).unwrap().join(name)
        }
    }

    /// An entry as a run that listed `path` (below the container) with nothing there leaves it.
    fn entry(path: &str) -> Value {
        json!({ PATH: path })
    }

    impl Journal {
        /// What the end of the process leaves: the journal on disk, and nothing that remembers what
        /// it holds, so the next start reads it as a new process's first start does.
        fn process_ended(&self) {
            lock(&KNOWN).remove(&self.file);
        }
    }

    impl Pending {
        /// What a kill leaves of a listing: the journal as it was written, and no process that
        /// remembers the export, so the next start meets the entries as a killed run's.
        fn killed(self) {
            self.journal.process_ended();
        }
    }

    /// Whether the permissions hold this user back: they do not hold back root, which reads and
    /// writes whatever its mode, so a test of a disk that refuses cannot be made that way for it.
    #[cfg(unix)]
    fn permissions_hold(sand: &Sandbox) -> std::result::Result<bool, io::Error> {
        use std::os::unix::fs::PermissionsExt;
        let probe = sand.folders.cache.join("permission-probe");
        fs::write(&probe, b"x")?;
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o000))?;
        let held = fs::read(&probe).is_err();
        fs::remove_file(&probe)?;
        Ok(held)
    }

    /// The acceptance: the app was killed mid-export, so the journal still lists what it was writing
    /// and the partial files are on disk; a new engine's start removes them and clears the journal.
    /// No process is killed: the killed run is the journal and the files it left. An image sequence
    /// is many files, and the ones the export never reached are not there to delete.
    #[test]
    fn a_restart_after_a_killed_export_removes_the_partial_files() -> Test {
        let sand = Sandbox::new()?;
        let unlisted = sand.put("earlier.mp4", b"a finished movie");
        let mut export = sand.journal().pending();
        let movie = sand.folders.container.join("card.mp4");
        let frames: Vec<PathBuf> = (0..3).map(|frame| sand.folders.container.join(format!("card_{frame:05}.png"))).collect();
        export.list(std::slice::from_ref(&movie))?;
        export.list(&frames)?;
        sand.put("card.mp4", b"half a movie");
        sand.put("card_00000.png", b"a frame");
        sand.put("card_00001.png", b"half a fr");
        export.killed();

        let engine = sand.start()?;
        assert!(!movie.exists() && !frames.iter().any(|frame| frame.exists()), "a partial file survived the restart");
        assert!(unlisted.exists(), "a file the journal never listed was deleted");
        assert!(!sand.file().exists(), "the journal was not cleared");
        let recovery = engine.recovery();
        assert_eq!(recovery.removed, [sand.real("card.mp4"), sand.real("card_00000.png"), sand.real("card_00001.png")]);
        assert!(recovery.journal.is_empty() && recovery.refused.is_empty() && recovery.failed.is_empty() && recovery.untouched.is_empty(), "{recovery:?}");
        Ok(())
    }

    /// A finished file the export was about to replace but never reached is still the file that was
    /// there when it was listed, and a start leaves it, as a clean cancel would; one the export did
    /// write over is deleted.
    #[test]
    fn a_start_leaves_a_finished_file_the_export_never_reached() -> Test {
        let sand = Sandbox::new()?;
        let reached = sand.put("first.mp4", b"last week's movie");
        let never_reached = sand.put("second.mp4", b"last week's other movie");
        let mut export = sand.journal().pending();
        export.list(&[reached.clone(), never_reached.clone()])?;
        // The writer replaces the first item's file; the kill comes before it opens the second's.
        fs::remove_file(&reached)?;
        fs::write(&reached, b"half")?;
        export.killed();

        let engine = sand.start()?;
        assert!(!reached.exists(), "the file the export wrote survived");
        assert_eq!(fs::read(&never_reached)?, b"last week's other movie", "a finished file the export never touched was deleted");
        assert_eq!(engine.recovery().untouched, [sand.real("second.mp4")]);
        assert!(!sand.file().exists());
        Ok(())
    }

    /// The journal names files below the container, not by its absolute path, which on iOS can
    /// change from one launch to the next: a start in the container's new place still finds them.
    #[test]
    fn a_start_finds_the_files_of_a_container_that_moved() -> Test {
        let root = TempDir::new()?;
        let before = Sandbox::at(root, "Application-1")?;
        let mut export = before.journal().pending();
        export.list(&[before.folders.container.join("exports/card.mp4")])?;
        before.put("exports/card.mp4", b"half");
        export.killed();
        let Sandbox { root, folders, outside, _alive } = before;
        // The guard goes before the next sandbox takes its own: a thread that waits for a second shared
        // hold behind a waiting writer would wait for ever.
        drop((outside, _alive));
        fs::rename(&folders.container, root.path().join("Application-2"))?;

        let after = Sandbox::at(root, "Application-2")?;
        let engine = after.start()?;
        assert_eq!(engine.recovery().removed, [after.real("exports/card.mp4")], "{:?}", engine.recovery());
        assert!(!after.folders.container.join("exports/card.mp4").exists());
        Ok(())
    }

    /// An export's list is in the journal while it runs, and gone once it ends: whether it finished,
    /// failed or panicked.
    #[test]
    fn an_export_is_listed_while_it_runs_and_cleared_when_it_ends_whatever_the_outcome() -> Test {
        for how in ["finishes", "fails", "panics"] {
            let sand = Sandbox::new()?;
            let (output, journal) = (sand.folders.container.join("card.mp4"), sand.file());
            let (seen, heard) = mpsc::channel();
            let script: Script = Box::new(move |_, list_outputs| {
                list_outputs(std::slice::from_ref(&output))?;
                let _ = seen.send(fs::read_to_string(&journal).unwrap_or_default());
                fs::write(&output, b"half a movie").unwrap();
                match how {
                    "finishes" => Ok(json!({"done": true})),
                    "fails" => Err(CraftError::command_failed("export", "disk full", None)),
                    _ => panic!("boom mid-export"),
                }
            });
            let engine = Engine::start_in(&sand.folders, Scripted(script))?;
            let answered = engine.request_with_progress("export", Value::Null, |_, _, _| {}, &CancelToken::new());
            match how {
                "finishes" => assert_eq!(answered, Ok(json!({"done": true}))),
                "fails" => assert_eq!(answered, Err(CraftError::command_failed("export", "disk full", None))),
                _ => assert!(matches!(answered, Err(CraftError::Internal { .. })), "{answered:?}"),
            }
            assert!(heard.recv()?.contains("card.mp4"), "the output was not listed while the export that {how} ran");
            assert!(!sand.file().exists(), "the entry outlived an export that {how}");
        }
        Ok(())
    }

    /// An export asked to stop before its turn came never runs, so it lists nothing.
    #[test]
    fn an_export_cancelled_before_its_turn_lists_nothing() -> Test {
        let sand = Sandbox::new()?;
        let output = sand.folders.container.join("card.mp4");
        let script: Script = Box::new(move |_, list_outputs| {
            list_outputs(std::slice::from_ref(&output))?;
            Ok(Value::Null)
        });
        let engine = Engine::start_in(&sand.folders, Scripted(script))?;
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(engine.request_with_progress("export", Value::Null, |_, _, _| {}, &cancel), Err(CraftError::Cancelled));
        assert!(!sand.file().exists());
        Ok(())
    }

    /// An engine started without folders keeps no journal: its lister takes the list and writes
    /// nothing.
    #[test]
    fn an_engine_without_folders_lists_nothing() -> Test {
        let sand = Sandbox::new()?;
        let output = sand.folders.container.join("card.mp4");
        let script: Script = Box::new(move |_, list_outputs| {
            list_outputs(std::slice::from_ref(&output))?;
            Ok(json!("ran"))
        });
        let engine = Engine::start(Scripted(script))?;
        assert_eq!(engine.request_with_progress("export", Value::Null, |_, _, _| {}, &CancelToken::new()), Ok(json!("ran")));
        assert!(!sand.file().exists());
        Ok(())
    }

    /// An export stopped mid-run clears its list too.
    #[test]
    fn an_export_stopped_mid_run_clears_its_list() -> Test {
        let sand = Sandbox::new()?;
        let output = sand.folders.container.join("card.mp4");
        let (listed, at_listed) = (Arc::new(Gate::default()), Arc::new(Gate::default()));
        let (opened, wait) = (listed.clone(), at_listed.clone());
        let script: Script = Box::new(move |on_progress, list_outputs| {
            list_outputs(std::slice::from_ref(&output))?;
            opened.open();
            assert!(wait.wait(), "the test never let the export go on");
            on_progress(1, 2, None);
            Err(CraftError::command_failed("export", "stopped by its caller", None))
        });
        let engine = Arc::new(Engine::start_in(&sand.folders, Scripted(script))?);
        let cancel = CancelToken::new();
        let exporting = (engine.clone(), cancel.clone());
        let export = thread::spawn(move || exporting.0.request_with_progress("export", Value::Null, |_, _, _| {}, &exporting.1));
        assert!(listed.wait(), "the export never listed its output");
        assert!(fs::read_to_string(sand.file())?.contains("card.mp4"));
        cancel.cancel();
        at_listed.open();
        assert_eq!(export.join().unwrap(), Err(CraftError::Cancelled));
        assert!(!sand.file().exists(), "a stopped export left its entry");
        Ok(())
    }

    /// Every path a listing is refused for: one outside the container however it is spelt, a link
    /// out of it, the container itself, a folder, a dangling link and a relative path. The listing
    /// fails, nothing is listed, and an export that passes the refusal on never writes.
    #[cfg(unix)]
    #[test]
    fn a_listing_of_a_path_outside_the_container_is_refused() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        std::os::unix::fs::symlink(sand.outside.join("photo.jpg"), container.join("dangling.mp4"))?;
        std::os::unix::fs::symlink(&sand.outside, container.join("out"))?;
        fs::write(sand.outside.join("there.mp4"), b"not ours")?;
        std::os::unix::fs::symlink(sand.outside.join("there.mp4"), container.join("link.mp4"))?;
        let refused = [
            (sand.outside.join("card.mp4"), "outside the container"),
            (container.join("..").join("outside").join("card.mp4"), "outside the container"),
            (container.join("out").join("card.mp4"), "outside the container"),
            (container.join("link.mp4"), "outside the container"),
            (container.clone(), "outside the container"),
            (container.join("dangling.mp4"), "a folder or a link"),
            (container.join("cache"), "a folder or a link"),
            (PathBuf::from("card.mp4"), "must be absolute"),
        ];
        for (path, why) in refused {
            let mut export = sand.journal().pending();
            let listed = export.list(&[container.join("fine.mp4"), path.clone()]);
            assert!(matches!(&listed, Err(error) if error.to_string().contains(why)), "{}: {listed:?}", path.display());
            assert!(!sand.file().exists(), "a refused listing of {} wrote the journal", path.display());
        }
        let output = sand.outside.join("card.mp4");
        let written = output.clone();
        let script: Script = Box::new(move |_, list_outputs| {
            list_outputs(std::slice::from_ref(&written))?;
            fs::write(&written, b"never").unwrap();
            Ok(Value::Null)
        });
        let engine = Engine::start_in(&sand.folders, Scripted(script))?;
        let answered = engine.request_with_progress("export", Value::Null, |_, _, _| {}, &CancelToken::new());
        assert!(matches!(answered, Err(CraftError::Io { .. })), "{answered:?}");
        assert!(!output.exists() && !sand.file().exists());
        Ok(())
    }

    /// A journal edited or damaged to name what no export writes deletes none of it, and says why:
    /// an absolute path, a climb out with `..`, the container itself, a folder, a link out, a folder
    /// that is a link out, and a listed file someone replaced with a link to a file of the app's.
    /// Only the one listed file inside the container is deleted.
    #[cfg(unix)]
    #[test]
    fn a_start_never_deletes_what_it_must_not() -> Test {
        let sand = Sandbox::new()?;
        let stranger = sand.outside.join("photo.jpg");
        fs::write(&stranger, b"not ours")?;
        let precious = sand.put("precious.mov", b"the user's own");
        let container = &sand.folders.container;
        std::os::unix::fs::symlink(&stranger, container.join("link-out.mp4"))?;
        std::os::unix::fs::symlink(&sand.outside, container.join("out"))?;
        std::os::unix::fs::symlink(&precious, container.join("swapped.mp4"))?;
        let partial = sand.put("card.mp4", b"half");
        sand.write_journal(&[
            entry(&stranger.display().to_string()),
            entry(&precious.display().to_string()),
            entry("../outside/photo.jpg"),
            entry("exports/../../outside/photo.jpg"),
            entry(""),
            entry("."),
            entry("cache"),
            entry("link-out.mp4"),
            entry("out/photo.jpg"),
            entry("swapped.mp4"),
            entry("card.mp4"),
        ]);

        let engine = sand.start()?;
        assert_eq!(fs::read(&stranger)?, b"not ours", "a file outside the container was deleted");
        assert_eq!(fs::read(&precious)?, b"the user's own", "a file behind a link was deleted");
        for kept in ["link-out.mp4", "out", "swapped.mp4"] {
            assert!(fs::symlink_metadata(container.join(kept)).is_ok(), "the link {kept} was deleted");
        }
        assert!(container.join("cache").is_dir() && container.is_dir());
        assert!(!partial.exists(), "the listed file inside the container was kept");
        let recovery = engine.recovery();
        assert_eq!(recovery.removed, [sand.real("card.mp4")]);
        assert_eq!(recovery.refused.len(), 10, "{recovery:?}");
        assert!(!sand.file().exists(), "refused entries stayed listed");
        Ok(())
    }

    /// A journal whose content is no journal names no file: the start deletes nothing, goes on,
    /// replaces it, and says what it found.
    #[test]
    fn a_damaged_journal_is_answered_replaced_and_the_engine_starts() -> Test {
        let too_long = format!("{{\"outputs\": [{{\"path\": \"card.mp4\"}}]}}{}", " ".repeat(usize::try_from(MAX_JOURNAL_BYTES)?));
        let damaged = [
            "{\"outputs\": [{\"path\": \"card",
            "",
            r#"{"outputs": ["card.mp4"]}"#,
            r#"{"outputs": [{"path": 1}]}"#,
            r#"{"outputs": [{"path": "card.mp4", "bytes": "four"}]}"#,
            r#"{"outputs": [{"path": "card.mp4", "modified": 1}]}"#,
            r#"{"files": []}"#,
            "\u{0}\u{1}",
            too_long.as_str(),
        ];
        for content in damaged {
            let sand = Sandbox::new()?;
            let kept = sand.put("card.mp4", b"half");
            fs::write(sand.file(), content)?;
            let engine = sand.start()?;
            let shown = content.get(..40).unwrap_or(content);
            assert!(kept.exists(), "{shown:?}: a file was deleted on the word of a damaged journal");
            assert_eq!(engine.recovery().journal.len(), 1, "{shown:?}: {:?}", engine.recovery());
            assert!(!sand.file().exists(), "{shown:?}: the damaged journal was kept");
        }
        Ok(())
    }

    /// A listing in `sand`'s journal is refused with an `Io` error whose message holds `why`.
    fn assert_listing_refused(sand: &Sandbox, why: &str) {
        let refused = sand.journal().pending().list(&[sand.folders.container.join("other.mp4")]);
        assert!(matches!(&refused, Err(CraftError::Io { message }) if message.contains(why)), "{refused:?}");
    }

    /// A start now removes `card.mp4`, the one file the journal listed, and listings work again.
    fn assert_start_frees_the_journal(sand: &Sandbox) -> Test {
        let later = sand.start()?;
        assert_eq!(later.recovery().removed, [sand.real("card.mp4")], "{:?}", later.recovery());
        sand.journal().pending().list(&[sand.folders.container.join("other.mp4")])?;
        Ok(())
    }

    /// A handle on the journal's lock file, as another process would open it.
    fn lock_handle(sand: &Sandbox) -> io::Result<File> {
        open_lock(&sand.folders.config.join(LOCK_FILE))
    }

    /// Whether another process could take the journal's lock now: a handle of its own on the lock file,
    /// as another process has, takes it, and lets it go as it drops.
    fn lock_is_free(sand: &Sandbox) -> io::Result<bool> {
        Ok(lock_handle(sand)?.try_lock().is_ok())
    }

    /// The journals set aside in the config folder, with what each holds.
    fn shelved(sand: &Sandbox) -> std::result::Result<Vec<PathBuf>, io::Error> {
        let mut found = Vec::new();
        for entry in fs::read_dir(&sand.folders.config)? {
            let path = entry?.path();
            if path.file_name().is_some_and(|name| name.to_string_lossy().starts_with(SHELF_PREFIX)) {
                found.push(path);
            }
        }
        Ok(found)
    }

    /// The folders of an output may not exist yet when it is listed (a segment's numbered folder):
    /// the file is still found at the next start.
    #[test]
    fn a_file_in_a_folder_made_after_it_was_listed_is_found_at_the_next_start() -> Test {
        let sand = Sandbox::new()?;
        let mut export = sand.journal().pending();
        export.list(&[sand.folders.container.join("card_001/card_00000.png")])?;
        sand.put("card_001/card_00000.png", b"half");
        export.killed();
        let engine = sand.start()?;
        assert_eq!(engine.recovery().removed, [sand.real("card_001/card_00000.png")], "{:?}", engine.recovery());
        Ok(())
    }

    /// A journal the disk will not read is set aside whole, so no export is refused for it and a later
    /// start that can read it deals with what it lists, then removes it.
    #[cfg(unix)]
    #[test]
    fn a_journal_the_disk_will_not_read_is_set_aside_for_a_later_start() -> Test {
        use std::os::unix::fs::PermissionsExt;
        let sand = Sandbox::new()?;
        let partial = sand.put("card.mp4", b"half");
        sand.write_journal(&[entry("card.mp4")]);
        let before = fs::read(sand.file())?;
        fs::set_permissions(sand.file(), fs::Permissions::from_mode(0o000))?;
        if fs::read(sand.file()).is_ok() {
            // A user the permissions do not hold back (root) cannot make the disk refuse this way.
            return Ok(());
        }

        let engine = sand.start()?;
        assert!(partial.exists(), "a file was deleted on the word of a journal nobody read");
        assert_eq!(engine.recovery().journal.len(), 1, "{:?}", engine.recovery());
        let shelf = shelved(&sand)?;
        assert_eq!(shelf.len(), 1, "the unread journal was not set aside");
        assert!(engine.recovery().journal.iter().any(|line| line.contains("cannot read") && line.contains("set aside")), "{:?}", engine.recovery());
        assert!(!sand.file().exists());
        sand.journal().pending().list(&[sand.folders.container.join("other.mp4")])?;

        // The disk will not read it at the next start either: the answer names it again.
        let again = sand.start()?;
        assert!(partial.exists() && again.recovery().journal.iter().any(|line| line.contains("stays set aside")), "{:?}", again.recovery());
        let one = shelf.first().ok_or("no shelf")?;
        fs::set_permissions(one, fs::Permissions::from_mode(0o600))?;
        assert_eq!(fs::read(one)?, before, "the unread journal was changed");

        let engine = sand.start()?;
        assert!(!partial.exists(), "a later start that can read the journal kept the partial file");
        assert_eq!(engine.recovery().removed, [sand.real("card.mp4")]);
        assert!(shelved(&sand)?.is_empty(), "a journal that was read was kept");
        assert!(engine.recovery().journal.iter().any(|line| line.contains("read the set-aside journal")), "{:?}", engine.recovery());
        Ok(())
    }

    /// A journal the disk stops reading mid-run is replaced whole from what the process knows it
    /// holds, the held entries and every running export's: nothing is refused and nothing set aside.
    #[cfg(unix)]
    #[test]
    fn a_listing_after_the_disk_stops_reading_the_journal_replaces_it() -> Test {
        use std::os::unix::fs::PermissionsExt;
        let sand = Sandbox::new()?;
        if !permissions_hold(&sand)? {
            return Ok(());
        }
        sand.start()?;
        let mut first = sand.journal().pending();
        first.list(&[sand.folders.container.join("first.mp4")])?;
        fs::set_permissions(sand.file(), fs::Permissions::from_mode(0o000))?;
        let mut second = sand.journal().pending();
        second.list(&[sand.folders.container.join("second.mp4")])?;
        let now = fs::read_to_string(sand.file())?;
        assert!(now.contains("first.mp4") && now.contains("second.mp4"), "{now}");
        assert!(shelved(&sand)?.is_empty(), "a journal was set aside mid-run");
        first.clear()?;
        second.clear()?;
        assert!(!sand.file().exists());
        Ok(())
    }

    /// An export that finished while the disk would not read the journal keeps its files, even when
    /// another engine started meanwhile: no copy of the journal holding its entries was set aside
    /// where its clear could not reach, so no later start, once the disk reads again, deletes what it
    /// wrote.
    #[cfg(unix)]
    #[test]
    fn a_finished_export_keeps_its_files_when_the_disk_stopped_reading_the_journal() -> Test {
        use std::os::unix::fs::PermissionsExt;
        for another_engine_starts in [false, true] {
            let sand = Sandbox::new()?;
            if !permissions_hold(&sand)? {
                return Ok(());
            }
            let (output, journal, folders) = (sand.folders.container.join("card.mp4"), sand.file(), sand.folders.clone());
            let script: Script = Box::new(move |_, list_outputs| {
                list_outputs(std::slice::from_ref(&output))?;
                fs::write(&output, b"a finished movie").unwrap();
                fs::set_permissions(&journal, fs::Permissions::from_mode(0o000)).unwrap();
                if another_engine_starts {
                    // Its start meets the unreadable journal before this export's clear does.
                    Engine::start_in(&folders, Scripted::idle()).unwrap();
                }
                Ok(json!({"done": true}))
            });
            let engine = Engine::start_in(&sand.folders, Scripted(script))?;
            let answered = engine.request_with_progress("export", Value::Null, |_, _, _| {}, &CancelToken::new());
            assert_eq!(answered, Ok(json!({"done": true})));
            // The disk reads every file again, and the process ends.
            let shelves = shelved(&sand)?;
            for shelf in &shelves {
                fs::set_permissions(shelf, fs::Permissions::from_mode(0o600))?;
            }
            sand.journal().process_ended();
            let later = sand.start()?;
            let kept = fs::read(sand.folders.container.join("card.mp4"))?;
            assert_eq!(kept, b"a finished movie", "another engine started: {another_engine_starts}; a later start deleted what a finished export wrote");
            assert!(shelves.is_empty() && later.recovery().removed.is_empty(), "{shelves:?} {:?}", later.recovery());
        }
        Ok(())
    }

    /// Several journals set aside: a start reads each it can, deals with what each lists, keeps in the
    /// journal what it could not delete so nothing listed is lost, removes those it read and the one
    /// that is no journal, leaves the one still unread, and says one line for each.
    #[cfg(unix)]
    #[test]
    fn several_set_aside_journals_are_each_dealt_with_and_answered() -> Test {
        use std::os::unix::fs::PermissionsExt;
        let sand = Sandbox::new()?;
        if !permissions_hold(&sand)? {
            return Ok(());
        }
        let shelf = |n: u32| sand.folders.config.join(format!("{SHELF_PREFIX}0-{n}.json"));
        let partial = sand.put("card.mp4", b"half");
        let stuck = sand.put("locked/frame.png", b"half");
        fs::write(shelf(1), json!({ OUTPUTS: [entry("card.mp4")] }).to_string())?;
        fs::write(shelf(2), json!({ OUTPUTS: [entry("locked/frame.png")] }).to_string())?;
        fs::write(shelf(3), b"{\"outputs\": [{\"path\": \"card")?;
        fs::write(shelf(4), json!({ OUTPUTS: [entry("later.mp4")] }).to_string())?;
        fs::set_permissions(shelf(4), fs::Permissions::from_mode(0o000))?;
        let locked = sand.folders.container.join("locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500))?;

        let engine = sand.start()?;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700))?;
        let recovery = engine.recovery();
        assert!(!partial.exists() && stuck.exists(), "{recovery:?}");
        assert_eq!(recovery.removed, [sand.real("card.mp4")]);
        assert_eq!(recovery.failed.len(), 1, "{recovery:?}");
        assert!(fs::read_to_string(sand.file())?.contains("locked/frame.png"), "what a set-aside journal listed was lost");
        assert_eq!(shelved(&sand)?, [shelf(4)], "the journals read and the damaged one were not all removed, or the unread one was");
        let lines = &recovery.journal;
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert_eq!(lines.iter().filter(|line| line.contains("read the set-aside journal")).count(), 2, "{lines:?}");
        assert!(lines.iter().any(|line| line.contains("is not JSON") && line.contains("removed")), "{lines:?}");
        assert!(lines.iter().any(|line| line.contains("stays set aside")), "{lines:?}");
        Ok(())
    }

    /// At most [`MAX_SET_ASIDE`] journals wait set aside: past that a start leaves the unread journal
    /// where it is and refuses listings, naming it, and a later start that reads it deals with it.
    #[cfg(unix)]
    #[test]
    fn set_aside_journals_are_bounded() -> Test {
        use std::os::unix::fs::PermissionsExt;
        let sand = Sandbox::new()?;
        if !permissions_hold(&sand)? {
            return Ok(());
        }
        for n in 0..MAX_SET_ASIDE {
            let shelf = sand.folders.config.join(format!("{SHELF_PREFIX}0-{n}.json"));
            fs::write(&shelf, b"{\"outputs\": []}")?;
            fs::set_permissions(&shelf, fs::Permissions::from_mode(0o000))?;
        }
        let partial = sand.put("card.mp4", b"half");
        sand.write_journal(&[entry("card.mp4")]);
        fs::set_permissions(sand.file(), fs::Permissions::from_mode(0o000))?;

        let engine = sand.start()?;
        assert_eq!(shelved(&sand)?.len(), MAX_SET_ASIDE, "a journal was set aside past the bound");
        assert!(sand.file().exists() && partial.exists());
        assert!(engine.recovery().journal.iter().any(|line| line.contains("cannot be set aside")), "{:?}", engine.recovery());
        assert_listing_refused(&sand, "until a start reads it");

        fs::set_permissions(sand.file(), fs::Permissions::from_mode(0o600))?;
        assert_start_frees_the_journal(&sand)
    }

    /// Another live process holds the journal's lock, so what it lists may be that process's running
    /// export: a start deletes nothing, writes nothing and answers why, every listing is refused
    /// with the same reason, and the next start that finds the lock free deals with the journal.
    #[test]
    fn a_start_while_another_process_holds_the_journal_deletes_nothing() -> Test {
        let sand = Sandbox::new()?;
        let running = sand.put("card.mp4", b"another process is writing this");
        sand.write_journal(&[entry("card.mp4")]);
        let before = fs::read(sand.file())?;
        let other = lock_handle(&sand)?;
        other.lock()?;

        let engine = sand.start()?;
        assert_eq!(fs::read(&running)?, b"another process is writing this", "a start deleted another process's running export");
        assert_eq!(fs::read(sand.file())?, before, "a start wrote a journal another process holds");
        let recovery = engine.recovery();
        assert!(recovery.removed.is_empty() && recovery.untouched.is_empty(), "{recovery:?}");
        assert!(recovery.journal.iter().any(|line| line.contains("another process holds the lock")), "{recovery:?}");
        assert_listing_refused(&sand, "another process holds the lock");
        let again = sand.start()?;
        assert!(running.exists() && again.recovery().journal.iter().any(|line| line.contains("another process holds the lock")), "{:?}", again.recovery());

        other.unlock()?;
        assert_start_frees_the_journal(&sand)
    }

    /// A process holds the journal's lock while its exports run with it, from the first listing until
    /// the last of them clears, and no longer: a start that runs while they do keeps it, and an idle
    /// process holds none, since iOS terminates a suspended app that holds a file lock outside its own
    /// data container.
    #[test]
    fn a_process_holds_the_journals_lock_only_while_its_exports_run() -> Test {
        let sand = Sandbox::new()?;
        sand.start()?;
        assert!(lock_is_free(&sand)?, "an idle process held the journal's lock after its start");
        let (mut first, mut second) = (sand.journal().pending(), sand.journal().pending());
        first.list(&[sand.folders.container.join("first.mp4")])?;
        assert!(!lock_is_free(&sand)?, "another process could take a journal this one's export runs with");
        second.list(&[sand.folders.container.join("second.mp4")])?;
        sand.start()?;
        first.clear()?;
        assert!(!lock_is_free(&sand)?, "the lock went while an export still ran");
        second.clear()?;
        assert!(lock_is_free(&sand)?, "the lock outlived the process's last export");
        Ok(())
    }

    /// A start holds the journal's lock from before it reads the journal until it has dealt with it, so
    /// no other process sets the journal aside or deletes a listed file while it does. The journal is a
    /// pipe the start waits to read, which slows the start itself and nothing else.
    #[cfg(unix)]
    #[test]
    fn a_start_holds_the_journals_lock_while_it_deals_with_the_journal() -> Test {
        let pipes = TempDir::new()?;
        let pipe = pipes.path().join("journal");
        {
            let _alone = NO_PROCESS_STARTS.write().unwrap_or_else(PoisonError::into_inner);
            assert!(std::process::Command::new("mkfifo").arg(&pipe).status()?.success(), "no pipe was made");
        }
        let sand = Sandbox::new()?;
        std::os::unix::fs::symlink(&pipe, sand.file())?;
        let start = thread::spawn({
            let folders = sand.folders.clone();
            move || Engine::start_in(&folders, Scripted::idle()).map(|engine| engine.recovery().clone())
        });
        // Opening the pipe to write returns once the start opens it to read, which it does after taking
        // the lock, and then the start waits for what is written.
        let mut writer = fs::OpenOptions::new().write(true).open(&pipe)?;
        let free_while_reading = lock_is_free(&sand);
        writer.write_all(br#"{"outputs": []}"#)?;
        drop(writer);
        let recovery = start.join().map_err(|_| "the start panicked")??;
        assert!(!free_while_reading?, "another process could take the journal while a start dealt with it");
        assert!(recovery.journal.is_empty(), "{recovery:?}");
        assert!(lock_is_free(&sand)?, "the start kept the lock once it was done");
        Ok(())
    }

    /// A listing the disk refuses leaves the process idle, holding no lock: a process that took the
    /// lock to list and could not write would otherwise hold it with no export to keep it for.
    #[cfg(unix)]
    #[test]
    fn a_listing_the_disk_refuses_leaves_no_lock_held() -> Test {
        use std::os::unix::fs::PermissionsExt;
        let sand = Sandbox::new()?;
        // The start makes the lock file, which a folder that takes no changes could not.
        sand.start()?;
        let journal = sand.journal();
        fs::set_permissions(&sand.folders.config, fs::Permissions::from_mode(0o500))?;
        let refused = journal.pending().list(&[sand.folders.container.join("card.mp4")]);
        let held_back = fs::write(sand.folders.config.join("probe"), b"").is_err();
        let free = lock_is_free(&sand);
        fs::set_permissions(&sand.folders.config, fs::Permissions::from_mode(0o700))?;
        // A user the permissions do not hold back (root) can always write it.
        if !held_back {
            return Ok(());
        }
        assert!(matches!(&refused, Err(CraftError::Io { message }) if message.contains("cannot list")), "{refused:?}");
        assert!(free?, "a refused listing left the lock held by an idle process");
        sand.journal().pending().list(&[sand.folders.container.join("card.mp4")])?;
        Ok(())
    }

    /// A start that met another process holding the journal refuses nothing for good: once that
    /// process lets go, the next listing runs without a new start, and keeps what the journal held
    /// for a start to deal with.
    #[test]
    fn a_listing_after_another_process_lets_go_needs_no_new_start() -> Test {
        let sand = Sandbox::new()?;
        let partial = sand.put("card.mp4", b"half");
        sand.write_journal(&[entry("card.mp4")]);
        let other = lock_handle(&sand)?;
        other.lock()?;
        let _engine = sand.start()?;
        assert_listing_refused(&sand, "another process holds the lock");
        drop(other);

        let mut export = sand.journal().pending();
        export.list(&[sand.folders.container.join("other.mp4")])?;
        let now = fs::read_to_string(sand.file())?;
        assert!(now.contains("other.mp4") && now.contains("card.mp4"), "{now}");
        assert!(partial.exists(), "a listing deleted a file: only a start deals with the journal");
        export.clear()?;
        assert_start_frees_the_journal(&sand)
    }

    /// Engines of one process start on the same journal at once: the first deals with what it lists,
    /// and no other starts as a first start of the process, so each file is dealt with once and none
    /// is reported failed or refused.
    #[test]
    fn engines_starting_at_once_deal_with_the_journal_once() -> Test {
        let sand = Sandbox::new()?;
        let names: Vec<String> = (0..8).map(|n| format!("frames/card_{n}.png")).collect();
        let entries: Vec<Value> = names.iter().map(|name| entry(name)).collect();
        for name in &names {
            sand.put(name, b"half");
        }
        sand.write_journal(&entries);
        let folders = sand.folders.clone();
        let starts: Vec<_> = (0..8)
            .map(|_| {
                thread::spawn({
                    let folders = folders.clone();
                    move || Engine::start_in(&folders, Scripted::idle()).map(|engine| engine.recovery().clone())
                })
            })
            .collect();
        let mut removed = 0;
        for start in starts {
            let recovery = start.join().map_err(|_| "a start panicked")??;
            assert!(recovery.failed.is_empty() && recovery.refused.is_empty() && recovery.journal.is_empty(), "{recovery:?}");
            removed += recovery.removed.len();
        }
        assert_eq!(removed, names.len(), "each listed file is deleted by exactly one start");
        assert!(!sand.file().exists());
        Ok(())
    }

    /// A journal that is a folder can never be read: it is set aside, the start answers, and exports
    /// still list.
    #[test]
    fn a_journal_that_is_a_folder_is_set_aside_and_blocks_nothing() -> Test {
        let sand = Sandbox::new()?;
        fs::create_dir_all(sand.file())?;
        let engine = sand.start()?;
        assert_eq!(engine.recovery().journal.len(), 1, "{:?}", engine.recovery());
        assert!(!sand.file().exists());
        sand.journal().pending().list(&[sand.folders.container.join("card.mp4")])?;
        assert!(sand.file().exists());
        let again = sand.start()?;
        assert!(again.recovery().journal.iter().any(|line| line.contains("stays set aside")), "{:?}", again.recovery());
        Ok(())
    }

    /// The atomic write: staging the new journal leaves the old one whole, the staged copy sits in
    /// the journal's own folder (a rename within one filesystem) and is complete, and publishing
    /// swaps it in with nothing left over.
    #[test]
    fn a_write_is_staged_whole_beside_the_journal_then_renamed_over_it() -> Test {
        let sand = Sandbox::new()?;
        let journal = sand.journal();
        sand.write_journal(&[entry("old.mp4")]);
        let before = fs::read(sand.file())?;

        let staged = journal.stage(b"{\"outputs\": [{\"path\": \"new.mp4\"}]}")?;
        assert_eq!(fs::read(sand.file())?, before, "staging changed the journal");
        assert_eq!(staged.path().parent(), journal.file.parent(), "the staged copy is not beside the journal");
        assert_eq!(fs::read(staged.path())?, b"{\"outputs\": [{\"path\": \"new.mp4\"}]}", "the staged copy is not complete");

        journal.publish(staged)?;
        assert_eq!(fs::read(sand.file())?, b"{\"outputs\": [{\"path\": \"new.mp4\"}]}");
        let names: Vec<_> = fs::read_dir(&sand.folders.config)?.map(|entry| entry.map(|e| e.file_name())).collect::<std::result::Result<_, _>>()?;
        assert_eq!(names, [std::ffi::OsString::from(JOURNAL_FILE)], "a staged copy was left behind");
        Ok(())
    }

    /// Every change of the journal is that rename: the file it replaces is never opened for
    /// writing, so a second name for the old file still reads the old journal whole, as a reader
    /// mid-way through it would.
    #[test]
    fn every_change_replaces_the_journal_and_never_writes_into_it() -> Test {
        let sand = Sandbox::new()?;
        sand.write_journal(&[entry("old.mp4")]);
        let before = fs::read(sand.file())?;
        let old = sand.folders.config.join("old-journal");
        fs::hard_link(sand.file(), &old)?;

        let (mut first, mut second) = (sand.journal().pending(), sand.journal().pending());
        first.list(&[sand.folders.container.join("new.mp4")])?;
        assert!(fs::read_to_string(sand.file())?.contains("new.mp4"), "the listing is not in the journal");
        assert_eq!(fs::read(&old)?, before, "the listing was written into the old journal");
        second.list(&[sand.folders.container.join("other.mp4")])?;
        let listed = fs::read(sand.file())?;
        fs::remove_file(&old)?;
        fs::hard_link(sand.file(), &old)?;
        first.clear()?;
        assert!(!fs::read_to_string(sand.file())?.contains("new.mp4"), "the clear is not in the journal");
        assert_eq!(fs::read(&old)?, listed, "the clear was written into the journal it replaced");
        second.clear()?;
        Ok(())
    }

    /// A journal is read no further than its bound, however long the file: one byte past it tells a
    /// journal that is too long.
    #[test]
    fn a_journal_is_never_read_past_its_bound() -> Test {
        struct Counted<R> {
            inner: R,
            read: u64,
        }
        impl<R: Read> Read for Counted<R> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let read = self.inner.read(buf)?;
                self.read += u64::try_from(read).unwrap();
                Ok(read)
            }
        }
        let mut file = Counted { inner: io::repeat(b' ').take(4 * MAX_JOURNAL_BYTES), read: 0 };
        let bytes = bounded(&mut file)?;
        assert_eq!(u64::try_from(bytes.len())?, MAX_JOURNAL_BYTES + 1);
        assert_eq!(file.read, MAX_JOURNAL_BYTES + 1, "the journal was read past its bound");
        Ok(())
    }

    /// A list longer than the journal holds is refused, and the journal as it was stays.
    #[test]
    fn a_list_past_the_journals_bound_is_refused() -> Test {
        let sand = Sandbox::new()?;
        let journal = sand.journal();
        sand.write_journal(&[entry("old.mp4")]);
        let before = fs::read(sand.file())?;
        let long = "x".repeat(1 << 20);
        let entries: Vec<Entry> = (0..17).map(|n| Entry { path: format!("{n}{long}"), found: None }).collect();
        let refused = journal.write(&entries);
        assert!(refused.is_err(), "a journal past its bound was written");
        assert_eq!(fs::read(sand.file())?, before);
        Ok(())
    }

    /// A write killed before its rename leaves a temporary file beside the journal; the next start
    /// removes it, and reads the journal the kill left whole.
    #[test]
    fn a_start_sweeps_what_a_killed_write_left_and_reads_the_old_journal() -> Test {
        let sand = Sandbox::new()?;
        let partial = sand.put("card.mp4", b"half");
        sand.write_journal(&[entry("card.mp4")]);
        let leftover = sand.folders.config.join(format!("{TEMP_PREFIX}abc.tmp"));
        fs::write(&leftover, b"{\"outputs\": [{\"path\": \"half writ")?;
        let mine = sand.folders.config.join("settings.json");
        fs::write(&mine, b"{}")?;

        sand.start()?;
        assert!(!leftover.exists() && !partial.exists());
        assert!(mine.exists(), "the sweep removed a file that is not the journal's");
        Ok(())
    }

    /// Engines of one app share the journal: a second engine started while the first one's export
    /// runs does not delete what it is writing nor drop its entry, even where an entry the first
    /// start could not deal with names the same file, deals with what a killed run of another process
    /// left, and the first still clears its entry at its end.
    #[cfg(unix)]
    #[test]
    fn a_second_engines_start_leaves_a_running_exports_files_alone() -> Test {
        use std::os::unix::fs::PermissionsExt;
        let sand = Sandbox::new()?;
        // A killed run left a partial file the first start cannot delete, so it stays held.
        let output = sand.put("locked/running.mp4", b"half");
        sand.write_journal(&[entry("locked/running.mp4")]);
        let locked = sand.folders.container.join("locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500))?;
        let (listed, release) = (Arc::new(Gate::default()), Arc::new(Gate::default()));
        let (opened, wait, written) = (listed.clone(), release.clone(), output.clone());
        let script: Script = Box::new(move |_, list_outputs| {
            list_outputs(std::slice::from_ref(&written))?;
            fs::write(&written, b"being written").unwrap();
            opened.open();
            assert!(wait.wait(), "the test never let the export end");
            Ok(Value::Null)
        });
        let first = Arc::new(Engine::start_in(&sand.folders, Scripted(script))?);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700))?;
        if first.recovery().failed.is_empty() {
            // A user the permissions do not hold back (root) deleted it anyway.
            return Ok(());
        }
        // Another process ran an export with the journal meanwhile and was killed: the export's
        // listing reads that when it takes the journal, and keeps it for a start.
        let stale = sand.put("stale.mp4", b"half");
        let mut journal: Value = serde_json::from_slice(&fs::read(sand.file())?)?;
        journal[OUTPUTS].as_array_mut().ok_or("no list")?.push(entry("stale.mp4"));
        fs::write(sand.file(), journal.to_string())?;
        let exporting = first.clone();
        let export = thread::spawn(move || exporting.request_with_progress("export", Value::Null, |_, _, _| {}, &CancelToken::new()));
        assert!(listed.wait(), "the export never listed its output");

        let second = sand.start()?;
        assert_eq!(fs::read(&output)?, b"being written", "the start deleted a running export's file");
        assert!(!stale.exists(), "a start kept what a killed run of another process left");
        assert_eq!(second.recovery().removed, [sand.real("stale.mp4")], "{:?}", second.recovery());
        assert!(second.recovery().failed.is_empty(), "{:?}", second.recovery());
        assert!(fs::read_to_string(sand.file())?.contains("locked/running.mp4"), "the running export's entry was dropped");

        release.open();
        assert_eq!(export.join().unwrap(), Ok(Value::Null));
        assert!(!sand.file().exists(), "the running export's entry outlived it");
        Ok(())
    }

    /// Two exports that list one file each clear only their own listing of it, so the file stays
    /// listed while the other still writes it.
    #[test]
    fn two_exports_of_one_file_each_clear_their_own_listing() -> Test {
        let sand = Sandbox::new()?;
        let output = sand.folders.container.join("card.mp4");
        let (mut first, mut second) = (sand.journal().pending(), sand.journal().pending());
        first.list(std::slice::from_ref(&output))?;
        second.list(std::slice::from_ref(&output))?;
        first.clear()?;
        assert!(fs::read_to_string(sand.file())?.contains("card.mp4"), "one export's end unlisted the other's file");
        second.clear()?;
        assert!(!sand.file().exists());
        Ok(())
    }

    /// A listed file whose folder is now a file is not there to delete: the entry is dropped, not
    /// tried again at every start.
    #[test]
    fn an_entry_whose_folder_is_now_a_file_is_absent() -> Test {
        let sand = Sandbox::new()?;
        let in_the_way = sand.put("exports", b"a file where the folder was");
        sand.write_journal(&[entry("exports/card.mp4")]);
        let engine = sand.start()?;
        let recovery = engine.recovery();
        assert!(recovery.failed.is_empty() && recovery.removed.is_empty(), "{recovery:?}");
        assert!(!sand.file().exists(), "an entry naming nothing stayed listed");
        assert_eq!(fs::read(&in_the_way)?, b"a file where the folder was");
        Ok(())
    }

    /// A listed file the disk will not let go stays listed, so the next start tries again.
    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_deleted_stays_listed() -> Test {
        use std::os::unix::fs::PermissionsExt;
        let sand = Sandbox::new()?;
        let partial = sand.put("locked/card.mp4", b"half");
        sand.write_journal(&[entry("locked/card.mp4")]);
        let locked = partial.parent().unwrap().to_path_buf();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500))?;

        let engine = sand.start()?;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700))?;
        if !partial.exists() {
            // A user the permissions do not hold back (root) deletes it anyway.
            return Ok(());
        }
        assert_eq!(engine.recovery().failed.len(), 1, "{:?}", engine.recovery());
        assert!(fs::read_to_string(sand.file())?.contains("locked/card.mp4"), "an entry that could not be deleted was dropped");
        // The next start of the same process tries it again, from the journal it reads again.
        let again = sand.start()?;
        assert_eq!(again.recovery().removed, [sand.real("locked/card.mp4")], "{:?}", again.recovery());
        assert!(!sand.file().exists());
        Ok(())
    }

    /// An export whose list cannot be cleared when it ends is answered as an error, never a success:
    /// the entry would make the next start delete the finished file.
    #[cfg(unix)]
    #[test]
    fn an_entry_that_cannot_be_cleared_is_reported_not_hidden() -> Test {
        use std::os::unix::fs::PermissionsExt;
        let sand = Sandbox::new()?;
        let (output, config) = (sand.folders.container.join("card.mp4"), sand.folders.config.clone());
        let script: Script = Box::new(move |_, list_outputs| {
            list_outputs(std::slice::from_ref(&output))?;
            fs::write(&output, b"a finished movie").unwrap();
            // The disk refuses the clear: the journal's folder no longer takes changes.
            fs::set_permissions(&config, fs::Permissions::from_mode(0o500)).unwrap();
            Ok(json!({"done": true}))
        });
        let engine = Engine::start_in(&sand.folders, Scripted(script))?;
        let answered = engine.request_with_progress("export", Value::Null, |_, _, _| {}, &CancelToken::new());
        let held_back = fs::write(sand.folders.config.join("probe"), b"").is_err();
        fs::set_permissions(&sand.folders.config, fs::Permissions::from_mode(0o700))?;
        // A user the permissions do not hold back (root) can always clear it.
        if !held_back {
            return Ok(());
        }
        assert!(matches!(&answered, Err(CraftError::Io { message }) if message.contains("card.mp4")), "{answered:?}");
        Ok(())
    }
}
