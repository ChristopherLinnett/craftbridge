//! The pending-export journal: what lets the next start delete the half-written files of an export
//! the system killed (plan §16, D-17).
//!
//! EffectCraft's exporter removes the files of an export only when it is cancelled cleanly; an app
//! the system kills mid-export leaves the partial output behind. So while an export runs, the app
//! lists in `pending-exports.json`, in the config folder, every file it is about to write, before it
//! writes the first (`CraftApp::request_with_progress`), and the engine clears them when the export
//! ends, whatever its outcome; an export it answers `Cancelled` has its files deleted first, by the
//! rules below, since the caller was told none was made. At the next start each file the journal
//! still lists is dealt with as the clean cancel the kill prevented would have dealt with it, and the
//! journal is cleared.
//!
//! A writer makes the folders a file's path needs, so the journal also keeps, apart from the files,
//! each folder of a listed file's path that was not there when the file was listed. The next start
//! deals with those after the files, the deepest first, and removes each one that is then an empty
//! folder, so a killed export leaves the container as it found it: a folder that holds anything the
//! export did not write stays, as does every folder that was there before the export.
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
//!   stays as it was, as a clean cancel leaves it. A file that could not be deleted stays listed for
//!   the next start, until an export lists its path again and takes its entry over, keeping what
//!   was there before: a finished file that export writes there is never deleted for the older entry,
//!   and one that fails leaves whatever is there, as a failed export leaves its own files.
//! * **Only regular files, and empty folders an export made, inside the container are touched.**
//!   Entries are stored relative to the container, because the absolute path of an iOS app's
//!   container can change between one launch and the next (an update moves it). A start joins each
//!   entry to the container it has now, resolves the entry's folder and refuses one outside the
//!   container, and looks at what is there without following a link: a listed file is deleted only
//!   when it is a regular file, and a listed folder only when it is a folder that holds nothing,
//!   which the system's own removal of a folder checks as it removes it, so a link, a folder that
//!   holds anything and everything outside the container are left alone and reported. Apple's File
//!   System Programming Guide calls a path-based URL "potentially fragile between launches"
//!   (*Accessing Files and Directories*), and an Apple engineer answers on the developer forums
//!   (thread 92693) that the path to an app's container, "and hence everything inside" it, "can
//!   change", with the system making sure the app is not running when it does, and tells apps to
//!   store paths relative to the container's root.
//! * **A journal that cannot be read is an answer, not a crash.** One whose content is no journal
//!   names no file, so nothing is deleted, and the start replaces it and says so. One the disk will
//!   not read when a process first starts on it is a killed run's, so it is renamed, unread, to a
//!   file of its own beside it (the start says so and why): its entries are not lost and no export is
//!   refused for it. Every later start tries each such file again, deals with what they list once it
//!   reads every one, removes them and says so. At most [`MAX_SET_ASIDE`] wait at once; past that
//!   the unread journal stays where it is, and each start deals with nothing and each listing is
//!   refused, naming it, until one can read it.
//! * **A set-aside entry yields to every later export of its file.** A listing never reads the
//!   journals set aside, which the disk may still refuse to read, so an export that lists one of their
//!   files cannot take its entry over as it lists. Instead, while one waits, the journal also keeps
//!   the files of each export that ended other than by a cancel (`taken`), and a start hands each
//!   set-aside entry to the same take-over rule, with those exports as the later listing: the entry of
//!   a file one of them ended at is the set-aside journal's no more, and the file that export left is
//!   never deleted for it. A cancelled export has deleted what it wrote, and one a kill stopped is
//!   still listed, so for those the set-aside entry deals with what is left, as the entry that took it
//!   over would have. The start names the files whose entries yielded. Any journal may record such an
//!   export, so a start deals with what the set-aside journals list only once it reads every one and
//!   the journal: while one cannot be read, or their folder cannot be listed, they all wait. Nothing
//!   expires: one never read may have recorded an export of any file, so while one never reads again
//!   the others wait with it for as long, with the partial files they list and the record, and each
//!   start says so. One that reads as no journal may have recorded an export of any file they list, so
//!   the journal records instead that every set-aside entry of a file yields, and none of those is
//!   deleted; it names no file itself, so it is replaced, or removed once that record is written, and
//!   never waits or takes a place of the [`MAX_SET_ASIDE`], past which listings would be refused while
//!   it lasts. The journal's own entries are later than every set-aside one, and a later listing takes
//!   them over as it lists, so a start deals with them by their own record whatever the set-aside
//!   journals hold. The record says nothing of when an export ended, so an entry the journal lists
//!   after one, and keeps once the journal is set aside in its turn, yields to it as well, and a
//!   partial file its killed run wrote there is left: a partial file left costs no work, while a
//!   finished one deleted would. The record goes once no journal waits set aside, and only then: a
//!   folder the disk will not list may hold some. It names at most [`MAX_TAKEN`] files, past which
//!   every set-aside entry of a file yields, so a journal set aside that never reads again cannot grow
//!   the journal without end.
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
use std::collections::btree_map::Entry as Slot;
use std::collections::{BTreeMap, BTreeSet};
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
/// The key of the list of folders the listed files' writers make, kept apart from the files, since a
/// start deals with them only once the files are dealt with; absent when there are none.
const FOLDERS: &str = "folders";
/// The key of the list of files whose set-aside entries yield: each one an export ended at, other than
/// by a cancel, while a journal waited set aside; or [`EVERY`], past [`MAX_TAKEN`] of them, or once a
/// journal that is no journal may have recorded such an export of any file. Kept while a journal waits
/// set aside; absent when there are none.
const TAKEN: &str = "taken";
/// What [`TAKEN`] holds in place of its list when every set-aside entry of a file yields.
const EVERY: &str = "every";
/// The most files [`TAKEN`] names one by one. Every listing and clear writes the whole journal, and a
/// journal set aside that never reads again would have the list grow with each file exported until
/// the journal passed [`MAX_JOURNAL_BYTES`] and every listing were refused. Past this many it records
/// that every set-aside entry of a file yields instead, which may leave a partial file a set-aside
/// journal lists, but never deletes a file an export left.
const MAX_TAKEN: usize = 4096;
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
    /// Entries no export runs for now (files a start or a cancelled export could not delete, and what
    /// a killed run left), kept for a start to deal with, or for an export that lists the same file
    /// to take over.
    held: Vec<Entry>,
    /// The entries of the exports running in this process now.
    running: Vec<Entry>,
    /// The files whose set-aside entries yield ([`TAKEN`]).
    taken: Taken,
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

/// The files whose set-aside entries yield to an export that ended at them ([`TAKEN`]).
#[derive(Debug, Default)]
struct Taken {
    /// Those files, by path below the container, while there are no more than [`MAX_TAKEN`].
    files: BTreeSet<String>,
    /// Every set-aside entry of a file yields, since more than [`MAX_TAKEN`] files were taken, or a
    /// journal that is no journal may have recorded an export of any file.
    every: bool,
}

impl Taken {
    /// What is taken once every set-aside entry of a file yields.
    fn every() -> Self {
        Self { files: BTreeSet::new(), every: true }
    }

    /// Take `files` too, or every file once there would be more than [`MAX_TAKEN`].
    fn extend(&mut self, files: impl IntoIterator<Item = String>) {
        for file in files {
            if self.every {
                break;
            }
            self.files.insert(file);
            self.every = self.files.len() > MAX_TAKEN;
        }
        if self.every {
            self.files.clear();
        }
    }

    /// Take what `other` takes too.
    fn merge(&mut self, other: Self) {
        self.every |= other.every;
        self.extend(other.files);
    }

    /// Whether a set-aside entry of the file at `path` yields.
    fn yields(&self, path: &str) -> bool {
        self.every || self.files.contains(path)
    }

    fn is_empty(&self) -> bool {
        !self.every && self.files.is_empty()
    }

    fn clear(&mut self) {
        *self = Self::default();
    }

    /// The JSON [`TAKEN`] holds, or none when nothing is taken.
    fn to_value(&self) -> Option<Value> {
        if self.every {
            Some(Value::from(EVERY))
        } else if self.files.is_empty() {
            None
        } else {
            Some(Value::Array(self.files.iter().map(|path| Value::Object(named(path))).collect()))
        }
    }
}

/// What a journal lists: its entries, and the files whose set-aside entries yield ([`TAKEN`]).
#[derive(Debug, Default)]
struct Contents {
    entries: Vec<Entry>,
    taken: Taken,
}

impl Contents {
    fn extend(&mut self, other: Self) {
        self.entries.extend(other.entries);
        self.taken.merge(other.taken);
    }
}

/// The journals earlier takes set aside, as a start reads them.
#[derive(Debug, Default)]
struct SetAside {
    /// What those that read as journals list, together.
    contents: Contents,
    /// Each one read, with why it is no journal when it is not.
    read: Vec<(PathBuf, Option<String>)>,
    /// How many the disk would not read now, their folder counted as one when the disk will not list
    /// it, since any journal there may be set aside.
    unread: usize,
}

impl SetAside {
    /// Say of each one read as a journal that it waits with the rest, unremoved, for a start that reads
    /// them all, and answer those that are no journal, with why: they name no file to delete, and the
    /// journal's record that every set-aside entry of a file yields stands for what they may have held,
    /// so they need not wait.
    fn wait(self, recovery: &mut Recovery) -> Vec<(PathBuf, Option<String>)> {
        let (journals, damaged): (Vec<_>, Vec<_>) = self.read.into_iter().partition(|(_, damaged)| damaged.is_none());
        for (shelf, _) in journals {
            recovery.journal.push(format!(
                "the set-aside journal {} waits for those that cannot be read now, which may hold a later export of a file it lists",
                shelf.display()
            ));
        }
        damaged
    }
}

/// Say of each of `shelves`, set-aside journals a start read, that is no journal that it stays until
/// a start can write the journal, which must first record what its going costs.
fn kept_until_written(shelves: Vec<(PathBuf, Option<String>)>, recovery: &mut Recovery) {
    for (_, damaged) in shelves {
        if let Some(reason) = damaged {
            recovery.journal.push(format!("{reason}; it names no file to delete, and goes when a start can write the journal"));
        }
    }
}

/// What a process that has just taken the journal found it to be.
enum Found {
    /// A journal, or nothing: what it lists.
    Read(Contents),
    /// No journal, which names nothing and is replaced: what the start says of it, and what the journal
    /// that replaces it records set-aside entries yield to.
    Replaced(String, Taken),
    /// A journal the disk would not read, set aside unread for a later start: what the start says of it.
    SetAside(String),
}

/// What a start did about the journal it found: the answer to each way it can disagree with the
/// disk. The files are named where they are now, inside the container.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recovery {
    /// Listed files the killed exports wrote, now deleted, then the folders they made for them,
    /// each removed once it held nothing.
    pub removed: Vec<PathBuf>,
    /// Listed files still as they were when listed: the export never reached them, so they are left;
    /// and folders an export made that hold something it did not write, which are left too.
    pub untouched: Vec<PathBuf>,
    /// Listed paths the start refused to touch, with why: one outside the container, a folder where
    /// a file was listed, anything but a folder where a folder was, or a link. They are dropped from
    /// the journal, since no later start would act on them.
    pub refused: Vec<(PathBuf, String)>,
    /// Listed files and folders that could not be deleted, with why. They stay listed, so the next
    /// start tries again, unless an export lists one of those files first and takes its entry over.
    pub failed: Vec<(PathBuf, String)>,
    /// What the start found about the journal files themselves: one it could not read, set aside or
    /// clear, each journal set aside earlier that is still unread, each it read that waits for those,
    /// and each it read now, dealt with and removed; and which files the set-aside journals list it
    /// left because an export ended at them while the journals waited, or might have.
    pub journal: Vec<String>,
}

/// The journal of one app: where it is, and the container its entries lie in.
#[derive(Debug, Clone)]
pub(crate) struct Journal {
    file: PathBuf,
    /// The container as the operating system resolves it: every entry is a path below it.
    root: PathBuf,
}

/// The entries one export has listed, cleared with [`Pending::clear`] when the export ends, or with
/// [`Pending::remove`], which first deletes what they name, when it is cancelled.
#[derive(Debug)]
pub(crate) struct Pending {
    journal: Journal,
    listed: Vec<Entry>,
    /// Every folder of a listed file's path this export has looked at, there or not, so each is looked
    /// at once however many files of a sequence it holds.
    looked: BTreeSet<PathBuf>,
}

/// One listed path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// Where it is below the container, as the export's writer opens it.
    path: String,
    /// What the export's writer puts there.
    kind: Kind,
}

/// What a listed path is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A file the export is about to write, with the file that was there when it was listed, or none.
    File(Option<Stamp>),
    /// A folder of a listed file's path that was not there when the file was listed, which the
    /// export's writer makes as it writes the file.
    Folder,
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
    /// It is the file that was there when it was listed, or a folder that holds something the export
    /// did not write, so it is left.
    Untouched,
    /// The export wrote it, or made it and it holds nothing, and it is deleted.
    Removed,
}

/// Why a listed path was not dealt with.
enum Refusal {
    /// It is not a regular file, which is all an export writes: what it is instead.
    NotAFile(&'static str),
    /// It is not a folder, which is all a writer makes for a file: what it is instead.
    NotAFolder(&'static str),
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
        let (Known { held, running, taken, lock }, taken_now) = match known.remove(&self.file) {
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
        let set_aside = self.read_set_aside(&mut recovery);
        // Whether a journal this start cannot read now may hold a later export of a file a set-aside
        // journal lists.
        let mut waits = set_aside.unread > 0;
        let mut found = Contents::default();
        if taken_now {
            match self.take_entries() {
                Ok(Found::Read(contents)) => found = contents,
                Ok(Found::Replaced(note, taken)) => {
                    found.taken = taken;
                    recovery.journal.push(note);
                }
                Ok(Found::SetAside(note)) => {
                    waits = true;
                    recovery.journal.push(note);
                }
                // Writing the journal now would replace what nobody has read, so nothing is dealt with
                // or written; the lock goes as the start returns.
                Err(why) => {
                    recovery.journal.push(format!("{why}, so nothing is dealt with now, and each later start and listing tries again"));
                    kept_until_written(set_aside.wait(&mut recovery), &mut recovery);
                    return recovery;
                }
            }
        }
        let (mut stale, mut taken) = (held, taken);
        taken.merge(found.taken);
        // A set-aside journal that is no journal may have recorded a later export of any file the others
        // list, so from now on every set-aside entry of a file yields, which the journal records before
        // it goes.
        if set_aside.read.iter().any(|(_, damaged)| damaged.is_some()) {
            taken.merge(Taken::every());
        }
        // The set-aside journals removed once what they list has been dealt with and the journal is
        // written, each with why it is no journal when it is not: every one read, unless one waits,
        // and then those that are no journal.
        let read_now = if waits {
            set_aside.wait(&mut recovery)
        } else {
            stale.extend(still_set_aside(set_aside.contents, &mut taken, &self.root, &mut recovery));
            set_aside.read
        };
        stale.extend(found.entries);
        let held = self.deal_with_all(stale, &running, &mut recovery);
        let mut state = Known { held, running, taken, lock };
        let uncleared = |error: io::Error| format!("cannot clear the pending-export journal {}: {error}", self.file.display());
        match self.write(&state.entries(), &state.taken) {
            // The set-aside journals stay for a start that can write the journal, since what they
            // list, or what their going costs, is held nowhere else.
            Err(error) => {
                recovery.journal.push(uncleared(error));
                kept_until_written(read_now, &mut recovery);
            }
            // What the set-aside journals listed is dealt with now, or held in the journal, and the
            // record of what their entries yield to is written with it, for one the disk keeps.
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
                // With no journal set aside, no entry is left to yield, and the record goes.
                if !state.taken.is_empty() && self.none_waits() {
                    state.taken.clear();
                    if let Err(error) = self.write(&state.entries(), &state.taken) {
                        recovery.journal.push(uncleared(error));
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

    /// Deal with every entry of `stale` as a start does, into `recovery`, and answer those the disk
    /// would not let it delete, which stay listed for the next start. The files go first, in the order
    /// they were listed, then the folders their writers made, the deepest first, so each folder is
    /// looked at once what the export put in it has gone. An entry that a running export of this
    /// process (`running`) lists belongs to that export, whatever an older entry says, so it is left.
    fn deal_with_all(&self, stale: Vec<Entry>, running: &[Entry], recovery: &mut Recovery) -> Vec<Entry> {
        let (files, mut folders): (Vec<Entry>, Vec<Entry>) = stale.into_iter().partition(|entry| matches!(entry.kind, Kind::File(_)));
        folders.sort_by(|a, b| depth(&b.path).cmp(&depth(&a.path)).then_with(|| b.path.cmp(&a.path)));
        folders.dedup();
        let mut held = Vec::new();
        for entry in files.into_iter().chain(folders) {
            if running.iter().any(|listed| entry.claims(listed)) {
                continue;
            }
            // A folder that holds a file this start could not delete stays listed with it, for the
            // start that deletes the file.
            if entry.kind == Kind::Folder && held.iter().any(|listed| entry.claims(listed)) {
                held.push(entry);
                continue;
            }
            let target = match entry_in(&self.root, Path::new(&entry.path)) {
                Ok(target) => target,
                Err(refusal) => {
                    recovery.refused.push((PathBuf::from(entry.path), refusal.to_string()));
                    continue;
                }
            };
            let settled = match entry.kind {
                Kind::File(found) => settle(&target, found),
                Kind::Folder => settle_folder(&target),
            };
            match settled {
                Ok(Settled::Absent) => {}
                Ok(Settled::Untouched) => recovery.untouched.push(target),
                Ok(Settled::Removed) => recovery.removed.push(target),
                Err(Refusal::NotAFile(what)) => recovery.refused.push((target, format!("it is {what}, not a file an export writes"))),
                Err(Refusal::NotAFolder(what)) => recovery.refused.push((target, format!("it is {what}, not a folder a writer makes"))),
                Err(Refusal::Disk(reason)) => {
                    recovery.failed.push((target, reason));
                    held.push(entry);
                }
            }
        }
        held
    }

    /// What the journal holds, read by a process that has just taken it, when no export of any process
    /// runs with it: what it lists, or else a line saying what became of it. One that is no journal
    /// names no file and is replaced; while a journal may wait set aside, it may have recorded a later
    /// export of a file one of those lists, so the journal that replaces it records that every
    /// set-aside entry of a file yields, and a damaged journal never waits or takes a place of the
    /// [`MAX_SET_ASIDE`]. One the disk will not read is set aside, which is safe now, since no running
    /// export's entries are in it; past [`MAX_SET_ASIDE`], or when the disk will not list the journals
    /// set aside to count them, it stays, and the answer is why nothing may be written over it.
    fn take_entries(&self) -> std::result::Result<Found, String> {
        let none_waits = self.none_waits();
        match self.read_at(&self.file) {
            Ok(mut contents) => {
                // What exports ended at while a journal waited set aside matters to a set-aside entry
                // alone, so with none waiting the record goes.
                if none_waits {
                    contents.taken.clear();
                }
                Ok(Found::Read(contents))
            }
            Err(Unread::Damaged(reason)) if none_waits => {
                Ok(Found::Replaced(format!("{reason}; it names no file to delete, so it is replaced"), Taken::default()))
            }
            Err(Unread::Damaged(reason)) => Ok(Found::Replaced(
                format!(
                    "{reason}; it names no file to delete, so it is replaced, and since it may have held a later export of a file a set-aside journal lists, none of those is deleted"
                ),
                Taken::every(),
            )),
            Err(Unread::Disk(reason)) => {
                let shelved = match self.shelves() {
                    Err(why) => Err(why),
                    Ok(shelves) if shelves.len() >= MAX_SET_ASIDE => Err(format!("{} journals already wait set aside, the most kept", shelves.len())),
                    Ok(_) => self.shelve().map_err(|error| error.to_string()),
                };
                match shelved {
                    Ok(shelf) => Ok(Found::SetAside(format!("{reason}; it is set aside as {} for a later start", shelf.display()))),
                    Err(why) => Err(format!("{reason}; and it cannot be set aside ({why})")),
                }
            }
        }
    }

    /// This process's hold on the journal, for a listing or a clear: what it knows the journal holds
    /// while exports of its own run with it, or else the journal taken now, its lock and what it holds,
    /// kept for a start to deal with. The set-aside journals are left to a start.
    fn hold<'k>(&self, known: &'k mut BTreeMap<PathBuf, Known>) -> Result<&'k mut Known> {
        match known.entry(self.file.clone()) {
            Slot::Occupied(ours) => Ok(ours.into_mut()),
            Slot::Vacant(slot) => {
                let lock = match self.take_lock() {
                    Locked::Taken(file) => Some(file),
                    Locked::Unsupported(_) => None,
                    Locked::Elsewhere(why) => return Err(CraftError::io(format!("{why}; nothing can be listed while it does"))),
                };
                let found = self.take_entries().map_err(|why| CraftError::io(format!("{why}, so nothing can be listed until a start reads it")))?;
                let Contents { entries, taken } = match found {
                    Found::Read(contents) => contents,
                    Found::Replaced(_, taken) => Contents { entries: Vec::new(), taken },
                    Found::SetAside(_) => Contents::default(),
                };
                Ok(slot.insert(Known { held: entries, running: Vec::new(), taken, lock }))
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
        Pending { journal: self.clone(), listed: Vec::new(), looked: BTreeSet::new() }
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
        Ok(Entry { path: path.to_string(), kind: Kind::File(found) })
    }

    /// The folders of the path `file` (an entry's, below the container) that are not there now, the
    /// deepest first: the ones its writer makes. A folder this export has looked at already (in
    /// `looked`, or in `seen` during this listing) is not looked at again, and nor is any folder
    /// above it, which was looked at with it; one that is there holds only folders that are.
    fn made_folders(&self, file: &str, looked: &BTreeSet<PathBuf>, seen: &mut BTreeSet<PathBuf>) -> Vec<Entry> {
        let mut made = Vec::new();
        for folder in Path::new(file).ancestors().skip(1).take_while(|folder| !folder.as_os_str().is_empty()) {
            if looked.contains(folder) || !seen.insert(folder.to_path_buf()) {
                break;
            }
            match fs::symlink_metadata(self.root.join(folder)) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    made.push(Entry { path: folder.to_string_lossy().into_owned(), kind: Kind::Folder });
                }
                // Whatever is there was there before the export, so it is not the export's to remove;
                // one the disk will not show is not counted as the export's either.
                _ => break,
            }
        }
        made
    }

    /// Rename the journal, unread, to a file of its own beside it, so a start that can read it deals
    /// with what it lists and no export waits on it. Answers where it went.
    fn shelve(&self) -> io::Result<PathBuf> {
        let folder = self.folder()?;
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

    /// The journal's folder, which holds its temporary files and the journals set aside.
    fn folder(&self) -> io::Result<&Path> {
        self.file.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the journal has no folder"))
    }

    /// The journals earlier takes set aside, in name order, or why the disk will not list them.
    fn shelves(&self) -> std::result::Result<Vec<PathBuf>, String> {
        let folder = self.folder().map_err(|error| error.to_string())?;
        let names = fs::read_dir(folder)
            .and_then(Iterator::collect::<io::Result<Vec<_>>>)
            .map_err(|error| format!("cannot list the journals set aside in {}: {error}", folder.display()))?;
        let mut shelves: Vec<PathBuf> =
            names.into_iter().filter(|entry| entry.file_name().to_string_lossy().starts_with(SHELF_PREFIX)).map(|entry| entry.path()).collect();
        shelves.sort();
        Ok(shelves)
    }

    /// Whether no journal waits set aside. A folder the disk will not list may hold some, whose
    /// entries yield to what exports end at meanwhile, so it never counts as holding none.
    fn none_waits(&self) -> bool {
        self.shelves().is_ok_and(|shelves| shelves.is_empty())
    }

    /// Read every journal earlier takes set aside, for a start, each one the disk will not read now
    /// answered in `recovery`.
    fn read_set_aside(&self, recovery: &mut Recovery) -> SetAside {
        let mut set_aside = SetAside::default();
        let shelves = match self.shelves() {
            Ok(shelves) => shelves,
            Err(why) => {
                recovery.journal.push(format!("{why}; they stay set aside for a later start"));
                set_aside.unread = 1;
                return set_aside;
            }
        };
        for shelf in shelves {
            match self.read_at(&shelf) {
                Ok(contents) => {
                    set_aside.contents.extend(contents);
                    set_aside.read.push((shelf, None));
                }
                Err(Unread::Damaged(reason)) => set_aside.read.push((shelf, Some(reason))),
                Err(Unread::Disk(reason)) => {
                    recovery.journal.push(format!("{reason}; it stays set aside for a later start"));
                    set_aside.unread = set_aside.unread.saturating_add(1);
                }
            }
        }
        set_aside
    }

    /// What the journal at `file` lists. A missing file lists nothing.
    fn read_at(&self, file: &Path) -> std::result::Result<Contents, Unread> {
        let disk = |error: io::Error| Unread::Disk(format!("cannot read the pending-export journal {}: {error}", file.display()));
        let bytes = match fs::File::open(file) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Contents::default()),
            Err(error) => return Err(disk(error)),
            Ok(file) => bounded(file).map_err(disk)?,
        };
        let damaged = |what: &str| Unread::Damaged(format!("the pending-export journal {} {what}", file.display()));
        if u64::try_from(bytes.len()).map_or(true, |length| length > MAX_JOURNAL_BYTES) {
            return Err(damaged(&format!("is longer than the {MAX_JOURNAL_BYTES} bytes a journal holds")));
        }
        let document: Value = serde_json::from_slice(&bytes).map_err(|error| damaged(&format!("is not JSON: {error}")))?;
        let outputs = document.get(OUTPUTS).and_then(Value::as_array).ok_or_else(|| damaged(&format!("has no `{OUTPUTS}` list")))?;
        // The lists a journal holds only when it has something in them.
        let optional = |key: &str| match document.get(key) {
            None => Ok(&[][..]),
            Some(list) => list.as_array().map(Vec::as_slice).ok_or_else(|| damaged(&format!("has a `{key}` that is not a list"))),
        };
        let folders = optional(FOLDERS)?;
        let files = outputs
            .iter()
            .map(|output| Entry::file_from(output).ok_or_else(|| damaged(&format!("lists an entry that is not a `{PATH}` with what was there: {output}"))));
        let made = folders.iter().map(|folder| Entry::folder_from(folder).ok_or_else(|| damaged(&format!("lists a folder that is not a `{PATH}`: {folder}"))));
        let entries = files.chain(made).collect::<std::result::Result<_, _>>()?;
        let mut taken = Taken { every: document.get(TAKEN).and_then(Value::as_str) == Some(EVERY), ..Taken::default() };
        if !taken.every {
            let files =
                optional(TAKEN)?.iter().map(|file| path_from(file).ok_or_else(|| damaged(&format!("lists a taken file that is not a `{PATH}`: {file}"))));
            taken.extend(files.collect::<std::result::Result<Vec<_>, _>>()?);
        }
        Ok(Contents { entries, taken })
    }

    /// Replace the journal with one listing `entries` and `taken`, atomically; with nothing to list
    /// the file is removed, which reads as an empty journal.
    fn write(&self, entries: &[Entry], taken: &Taken) -> io::Result<()> {
        if entries.is_empty() && taken.is_empty() {
            return match fs::remove_file(&self.file) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        let (files, folders): (Vec<&Entry>, Vec<&Entry>) = entries.iter().partition(|entry| matches!(entry.kind, Kind::File(_)));
        let listed = |entries: Vec<&Entry>| Value::Array(entries.into_iter().map(Entry::to_value).collect());
        let mut document = Map::from_iter([(OUTPUTS.to_string(), listed(files))]);
        if !folders.is_empty() {
            document.insert(FOLDERS.to_string(), listed(folders));
        }
        if let Some(taken) = taken.to_value() {
            document.insert(TAKEN.to_string(), taken);
        }
        let bytes = serde_json::to_vec(&Value::Object(document)).map_err(io::Error::from)?;
        if u64::try_from(bytes.len()).map_or(true, |length| length > MAX_JOURNAL_BYTES) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("the journal would grow past the {MAX_JOURNAL_BYTES} bytes it holds")));
        }
        self.publish(self.stage(&bytes)?)
    }

    /// The first half of an atomic write: the new content, complete on disk in a temporary file of
    /// the journal's own folder (so the rename is within one filesystem), with the journal itself
    /// untouched.
    fn stage(&self, bytes: &[u8]) -> io::Result<NamedTempFile> {
        let folder = self.folder()?;
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
        let Ok(folder) = self.folder() else { return };
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
        let (mut entries, mut seen) = (Vec::with_capacity(outputs.len()), BTreeSet::new());
        for output in outputs {
            let file = self.journal.entry_of(output)?;
            let made = self.journal.made_folders(&file.path, &self.looked, &mut seen);
            entries.push(file);
            entries.extend(made);
        }
        if entries.is_empty() {
            return Ok(());
        }
        let mut known = lock(&KNOWN);
        let state = self.journal.hold(&mut known)?;
        let held = taken_over(&state.held, &mut entries);
        let mut listed: Vec<Entry> = held.iter().chain(&state.running).cloned().collect();
        listed.extend(entries.iter().cloned());
        let written = self.journal.write(&listed, &state.taken);
        if written.is_ok() {
            state.held = held;
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
        self.looked.extend(seen);
        Ok(())
    }

    /// Remove the export's entries, because it has ended. When they cannot be cleared they stay
    /// listed until the journal is next written, and a start before that deletes what the export
    /// wrote, finished or not, so the error says that and the caller must pass it on rather than
    /// answer success.
    pub(crate) fn clear(self) -> Result<()> {
        self.end(false)
    }

    /// End an export that was cancelled: delete what it wrote by the rules a start deletes by (a file
    /// it wrote, a folder it made that holds nothing now; a file still as it was when it was listed,
    /// and a folder that holds anything else, stay), then clear its entries. A file the disk will not
    /// let go of stays listed for the next start, and the error names it, since the caller must not
    /// hear a plain cancel while a file of the export is still there. A folder the disk will not
    /// remove, which then holds nothing of the export, stays listed for the next start too, and like
    /// entries that cannot be cleared once every file has gone, it is no error.
    pub(crate) fn remove(self) -> Result<()> {
        self.end(true)
    }

    fn end(self, remove: bool) -> Result<()> {
        let Self { journal, listed, .. } = self;
        if listed.is_empty() {
            return Ok(());
        }
        let mut known = lock(&KNOWN);
        let uncleared = |error: String| {
            // A start removes a folder only once it holds nothing, so only the files are named.
            let files: Vec<&Entry> = listed.iter().filter(|entry| matches!(entry.kind, Kind::File(_))).collect();
            let first = files.first().map_or_else(PathBuf::new, |entry| journal.root.join(&entry.path));
            let more = files.len().saturating_sub(1);
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
        let mut removal = Recovery::default();
        // Whether the disk would not let go of a file the cancelled export wrote.
        let mut file_kept = false;
        if remove {
            let held = journal.deal_with_all(listed.clone(), &state.running, &mut removal);
            file_kept = held.iter().any(|entry| matches!(entry.kind, Kind::File(_)));
            state.held.extend(held);
        } else if !journal.none_waits() {
            // What an export that ends other than by a cancel left at its files is its own, and the
            // entry of one in a journal set aside unread could not be taken over as it listed: the
            // record has the start that reads it hand it over now. A cancel records nothing, since
            // the export deleted what it wrote by its own entries, and the set-aside entry deals with
            // what was there before, as the entry this export would have taken over from it would.
            state.taken.extend(listed.iter().filter(|entry| matches!(entry.kind, Kind::File(_))).map(|entry| entry.path.clone()));
        }
        let written = journal.write(&state.entries(), &state.taken);
        // The process's last export with the journal has ended: the lock goes, and another process may
        // take the journal.
        if state.running.is_empty() {
            known.remove(&journal.file);
        }
        // The files are dealt with before the folders, so a file the disk kept is the first failure. A
        // folder the disk would not remove holds nothing of the export by then, since one that holds
        // a file the disk kept is held with it unremoved, so it stays listed for the next start and
        // is no reason on its own to answer anything but the cancel.
        match removal.failed.first().filter(|_| file_kept) {
            Some((path, why)) => Err(CraftError::io(format!(
                "the export was cancelled but {} could not be removed ({why}){}, so they stay listed in the pending-export journal and the next start tries again",
                path.display(),
                match removal.failed.len().saturating_sub(1) {
                    0 => String::new(),
                    more => format!(", nor {more} more"),
                }
            ))),
            // Every file the cancelled export wrote is gone, so nothing of it is left for the caller to
            // copy, and the plain cancel is the answer: an entry the journal could not clear names a
            // path that is empty now, which a later export that lists it takes over.
            None if remove => Ok(()),
            None => written.map_err(|error| uncleared(error.to_string())),
        }
    }
}

impl Entry {
    /// Whether `listed` is this entry's, or lies under it: a file is claimed by the entry of its own
    /// path, and a folder by every entry below it.
    fn claims(&self, listed: &Entry) -> bool {
        match self.kind {
            Kind::File(_) => listed.path == self.path,
            Kind::Folder => Path::new(&listed.path).starts_with(&self.path),
        }
    }

    /// The file entry a journal's JSON holds, or none for one that is not an entry.
    fn file_from(value: &Value) -> Option<Self> {
        let path = path_from(value)?;
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
        Some(Self { path, kind: Kind::File(found) })
    }

    /// The folder entry a journal's JSON holds, or none for one that is not an entry.
    fn folder_from(value: &Value) -> Option<Self> {
        Some(Self { path: path_from(value)?, kind: Kind::Folder })
    }

    fn to_value(&self) -> Value {
        let mut fields = named(&self.path);
        if let Kind::File(Some(found)) = self.kind {
            fields.insert(BYTES.to_string(), Value::from(found.bytes));
            if let Some(modified) = found.modified {
                fields.insert(MODIFIED.to_string(), Value::from(modified));
            }
        }
        Value::Object(fields)
    }
}

/// The path an entry's JSON names, or none for one that names none.
fn path_from(value: &Value) -> Option<String> {
    Some(value.get(PATH)?.as_str()?.to_string())
}

/// The JSON of an entry that names `path` and nothing more.
fn named(path: &str) -> Map<String, Value> {
    Map::from_iter([(PATH.to_string(), Value::from(path))])
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
    let Some(now) = found_at(target)? else { return Ok(Settled::Absent) };
    let kind = now.file_type();
    if !kind.is_file() {
        return Err(Refusal::NotAFile(what_is(kind)));
    }
    if found == Some(Stamp::of(&now)) {
        return Ok(Settled::Untouched);
    }
    fs::remove_file(target).map(|()| Settled::Removed).map_err(|error| Refusal::Disk(error.to_string()))
}

/// Deal with the listed folder at `target`, a path inside the container whose own name is not
/// followed: remove it when it is a folder that holds nothing, leave it when it holds anything, and
/// refuse anything that is not a folder. The system's removal of a folder removes only an empty one
/// and never follows a link at its name, so nothing a folder holds, or a link reaches, is touched
/// even if the folder changed since it was looked at.
fn settle_folder(target: &Path) -> std::result::Result<Settled, Refusal> {
    let Some(now) = found_at(target)? else { return Ok(Settled::Absent) };
    let kind = now.file_type();
    if !kind.is_dir() {
        return Err(Refusal::NotAFolder(what_is(kind)));
    }
    match fs::remove_dir(target) {
        Ok(()) => Ok(Settled::Removed),
        // POSIX lets `rmdir` answer a folder that holds anything with either error.
        Err(error) if matches!(error.kind(), io::ErrorKind::DirectoryNotEmpty | io::ErrorKind::AlreadyExists) => Ok(Settled::Untouched),
        Err(error) => Err(Refusal::Disk(error.to_string())),
    }
}

/// What is at `target` now, its own name not followed, or `None` when nothing is: a folder of its
/// path that is now a file holds nothing either.
fn found_at(target: &Path) -> std::result::Result<Option<fs::Metadata>, Refusal> {
    match fs::symlink_metadata(target) {
        Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => Ok(None),
        Err(error) => Err(Refusal::Disk(error.to_string())),
        Ok(now) => Ok(Some(now)),
    }
}

/// What a start found at a listed path, as its refusal names it.
fn what_is(kind: fs::FileType) -> &'static str {
    if kind.is_symlink() {
        "a symbolic link"
    } else if kind.is_dir() {
        "a folder"
    } else if kind.is_file() {
        "a file"
    } else {
        "neither a file nor a folder"
    }
}

/// How many names deep `path` (an entry's, below the container) lies.
fn depth(path: &str) -> usize {
    Path::new(path).components().count()
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

/// Hand `listing` the held entries of the files it lists again, and answer the entries still held:
/// the one rule by which a later listing of a file takes an earlier entry of it over, whether that
/// entry is held or set aside ([`still_set_aside`]). A file a start or a cancelled export could not
/// delete stays listed for the next start, which deletes whatever is at its path then unless it is
/// still the file that was there before that export; an export that lists the path again would
/// otherwise have the finished file it writes there deleted. So the new entry takes the held one's
/// place, with the file that was there before the export that left it: what is at the path is
/// deleted only if this export is killed or cancelled, and the file it finishes is kept.
fn taken_over(held: &[Entry], listing: &mut [Entry]) -> Vec<Entry> {
    let same_file = |a: &Entry, b: &Entry| matches!((a.kind, b.kind), (Kind::File(_), Kind::File(_))) && a.path == b.path;
    for listed in listing.iter_mut() {
        // The first held entry of the path is the oldest, the nearest to what was there before any
        // export wrote it.
        if let Some(older) = held.iter().find(|entry| same_file(entry, listed)) {
            listed.kind = older.kind;
        }
    }
    held.iter().filter(|entry| !listing.iter().any(|listed| same_file(listed, entry))).cloned().collect()
}

/// What the journals set aside list (`set_aside`) that is still theirs to deal with. Each entry of a
/// file `taken` names, which an export ended at other than by a cancel while a journal waited set
/// aside, is handed to that export by the rule a later listing takes an entry over by, and is theirs
/// no more: the export has ended, so what it left at the file is its own, and no entry of it is left
/// to keep the older record. The record says nothing of when, so an entry listed after such an
/// export, in a journal set aside in its turn, yields too, and a partial file its killed run wrote
/// there is left: a file left costs no work, while one deleted may be a finished one. Once `taken`
/// holds every file, past [`MAX_TAKEN`] or after a journal that is no journal, none of their files is
/// deleted; their folders are still removed once they hold nothing, which costs no file. The start
/// says which files yielded and why, naming them below `root`. `taken` gains what the set-aside
/// journals recorded too, kept for a start that reads one of them again because the disk would not
/// remove it.
fn still_set_aside(set_aside: Contents, taken: &mut Taken, root: &Path, recovery: &mut Recovery) -> Vec<Entry> {
    let Contents { entries, taken: recorded } = set_aside;
    taken.merge(recorded);
    let yielding: BTreeSet<String> =
        entries.iter().filter(|entry| matches!(entry.kind, Kind::File(_)) && taken.yields(&entry.path)).map(|entry| entry.path.clone()).collect();
    if let Some(first) = yielding.first() {
        let files = format!(
            "{}{}",
            root.join(first).display(),
            match yielding.len().saturating_sub(1) {
                0 => String::new(),
                more => format!(" and {more} more files"),
            }
        );
        recovery.journal.push(if taken.every {
            format!(
                "an export may have ended at any file the set-aside journals list while they waited, since more than {MAX_TAKEN} files were exported meanwhile or a journal was no journal, so none of them is deleted: {files}"
            )
        } else {
            format!("the set-aside journals' entries of {files} yield to the exports that ended there while they waited, so what is there is kept")
        });
    }
    // The exports that ended at those files, as the later listing.
    let mut ended: Vec<Entry> = yielding.into_iter().map(|path| Entry { path, kind: Kind::File(None) }).collect();
    taken_over(&entries, &mut ended)
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
    use crate::gpu::GpuChoice;
    use crate::testing::Gate;
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
            let folders = Folders::inside(root.path().join(container), "");
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

        /// The journal a killed run left, with these entries and folders as written.
        fn write_journal_with(&self, entries: &[Value], folders: &[Value]) {
            fs::write(self.file(), json!({ OUTPUTS: entries, FOLDERS: folders }).to_string()).unwrap();
        }

        fn start(&self) -> std::result::Result<Engine, CraftError> {
            Engine::start_in(&self.folders, Scripted::idle(), GpuChoice::CpuOnly)
        }

        /// An engine on these folders whose export runs `script`.
        fn start_running(&self, script: Script) -> std::result::Result<Engine, CraftError> {
            Engine::start_in(&self.folders, Scripted(script), GpuChoice::CpuOnly)
        }

        /// The resolved place of `name` in the container, as a recovery reports it.
        fn real(&self, name: &str) -> PathBuf {
            fs::canonicalize(&self.folders.container).unwrap().join(name)
        }

        /// Run `program` with `args` on these folders: the sandbox lets go of its hold for the run
        /// ([`run_alone`]), and the same folders come back, held again.
        #[cfg(target_os = "macos")]
        fn running(self, program: &str, args: &[&std::ffi::OsStr]) -> std::result::Result<Self, io::Error> {
            let Self { root, _alive, .. } = self;
            drop(_alive);
            run_alone(program, args)?;
            Self::at(root, "container")
        }
    }

    /// Run `program` with `args` while no sandbox lives, as a test that starts a process must
    /// ([`NO_PROCESS_STARTS`]).
    #[cfg(unix)]
    fn run_alone(program: &str, args: &[&std::ffi::OsStr]) -> io::Result<()> {
        let _alone = NO_PROCESS_STARTS.write().unwrap_or_else(PoisonError::into_inner);
        let status = std::process::Command::new(program).args(args).status()?;
        if status.success() { Ok(()) } else { Err(io::Error::other(format!("{program} answered {status}"))) }
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
        assert_eq!(engine.recovery().removed, [after.real("exports/card.mp4"), after.real("exports")], "{:?}", engine.recovery());
        assert!(!after.folders.container.join("exports").exists());
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
            let engine = sand.start_running(script)?;
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
        let engine = sand.start_running(script)?;
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
        let engine = Engine::start(Scripted(script), GpuChoice::CpuOnly)?;
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
        let engine = Arc::new(sand.start_running(script)?);
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

    /// Where an export that listed `exports/card.mp4` (its folder not there yet) is when it learns it is
    /// stopped. The script's own steps fix the order, so no test depends on how long anything took.
    #[derive(Clone, Copy, Debug)]
    enum Reached {
        /// Before it wrote anything.
        BeforeTheFirstFrame,
        /// With a partial file written.
        HalfWay,
        /// With the whole file written and every frame done, so the stop lands once the job has
        /// finished but before the call ends, and the app answers success.
        AfterTheLastFrame,
        /// With the whole file written, the app stops of its own accord and answers `Cancelled`,
        /// though nothing asked it to stop: the engine answers that, so it deletes as it does for a
        /// stop it asked for.
        ByItself,
        /// Never stopped.
        NotStopped,
    }

    /// The export of `exports/card.mp4`, which the writer makes the folder for, stopped at `reached`.
    /// An app that left its file where it stopped is the case the engine must clean up after: the
    /// real exporter removes its file at a stop only if it sees the stop before it finishes.
    fn stopped_at(reached: Reached, container: PathBuf, cancel: CancelToken) -> Script {
        Box::new(move |on_progress, list_outputs| {
            let (folder, file) = (container.join("exports"), container.join("exports/card.mp4"));
            list_outputs(std::slice::from_ref(&file))?;
            fs::create_dir_all(&folder).unwrap();
            if matches!(reached, Reached::HalfWay | Reached::AfterTheLastFrame | Reached::ByItself | Reached::NotStopped) {
                fs::write(&file, b"half a movie").unwrap();
            }
            if matches!(reached, Reached::AfterTheLastFrame | Reached::ByItself | Reached::NotStopped) {
                assert!(on_progress(1, 2, None), "nothing has stopped it yet");
                fs::write(&file, b"the whole movie").unwrap();
            }
            if !matches!(reached, Reached::ByItself | Reached::NotStopped) {
                cancel.cancel();
            }
            let go_on = on_progress(2, 2, None);
            match (reached, go_on) {
                (Reached::ByItself | Reached::NotStopped, true) | (_, false) => (),
                (reached, go_on) => panic!("a stop at {reached:?} was answered {go_on}"),
            }
            match reached {
                Reached::AfterTheLastFrame | Reached::NotStopped => Ok(json!({"rendered": 1})),
                Reached::ByItself => Err(CraftError::Cancelled),
                _ => Err(CraftError::command_failed("export", "stopped by its caller", None)),
            }
        })
    }

    /// An export answered `Cancelled` has removed every file it listed and the folder it made, whether
    /// it was stopped before its first frame, half way or after its last, or stopped itself, and what
    /// is not its own is left; one that finishes answers success with its file whole.
    #[test]
    fn an_export_answered_cancelled_leaves_none_of_its_listed_files_and_one_that_finishes_keeps_them() -> Test {
        // Every case is run and the faults listed together, so a fault in one never hides another.
        let mut faults = Vec::new();
        for reached in [Reached::BeforeTheFirstFrame, Reached::HalfWay, Reached::AfterTheLastFrame, Reached::ByItself, Reached::NotStopped] {
            let sand = Sandbox::new()?;
            let earlier = sand.put("earlier.mp4", b"a finished movie");
            let cancel = CancelToken::new();
            let engine = sand.start_running(stopped_at(reached, sand.folders.container.clone(), cancel.clone()))?;
            let answered = engine.request_with_progress("export", Value::Null, |_, _, _| {}, &cancel);
            let (folder, file) = (sand.folders.container.join("exports"), sand.folders.container.join("exports/card.mp4"));
            let mut fault = |what: &str| faults.push(format!("{reached:?}: {what}"));
            match reached {
                Reached::NotStopped => {
                    if answered != Ok(json!({"rendered": 1})) {
                        fault(&format!("answered {answered:?}"));
                    }
                    if fs::read(&file).ok().as_deref() != Some(b"the whole movie".as_slice()) {
                        fault("a finished export lost its file");
                    }
                }
                _ => {
                    if answered != Err(CraftError::Cancelled) {
                        fault(&format!("answered {answered:?}"));
                    }
                    if file.exists() || folder.exists() {
                        fault("left its file or its folder");
                    }
                }
            }
            if fs::read(&earlier).ok().as_deref() != Some(b"a finished movie".as_slice()) {
                fault("a file that is not the export's was touched");
            }
            if sand.file().exists() {
                fault("its entries outlived the export");
            }
        }
        assert!(faults.is_empty(), "{faults:#?}");
        Ok(())
    }

    /// The deletion is the journal's own: a listed file that was there as it is now, and a folder that
    /// holds a file the export did not write, are left, as a restart leaves them.
    #[test]
    fn a_cancelled_export_deletes_by_the_journals_rules() -> Test {
        let sand = Sandbox::new()?;
        let kept = sand.put("old/card_00000.png", b"an earlier frame");
        let (written, foreign) = (sand.folders.container.join("old/card_00001.png"), sand.folders.container.join("made/notes.txt"));
        let made = sand.folders.container.join("made/deep/card_00002.png");
        let outputs = [kept.clone(), written.clone(), made.clone()];
        let cancel = CancelToken::new();
        let stop = cancel.clone();
        let script: Script = Box::new(move |on_progress, list_outputs| {
            list_outputs(&outputs)?;
            fs::write(&outputs[1], b"a frame").unwrap();
            fs::create_dir_all(outputs[2].parent().unwrap()).unwrap();
            fs::write(&outputs[2], b"a frame").unwrap();
            fs::write(outputs[2].parent().unwrap().parent().unwrap().join("notes.txt"), b"not the export's").unwrap();
            stop.cancel();
            assert!(!on_progress(1, 3, None));
            Ok(Value::Null)
        });
        let engine = sand.start_running(script)?;
        assert_eq!(engine.request_with_progress("export", Value::Null, |_, _, _| {}, &cancel), Err(CraftError::Cancelled));
        assert_eq!(fs::read(&kept)?, b"an earlier frame", "a file that was there as it is now stays");
        assert!(!written.exists() && !made.exists(), "what the export wrote goes");
        assert!(foreign.is_file(), "a folder that holds what the export did not write stays with it");
        assert!(!sand.folders.container.join("made/deep").exists(), "the empty folder the export made goes");
        assert!(!sand.file().exists());
        Ok(())
    }

    /// Run, on an engine started in `sand`, an export of `files` that writes each whole in the folders
    /// it makes, after which `locked` takes no more changes, and that is cancelled then: an export
    /// cancelled while the disk refuses the engine what it does at the end. Answers the engine and
    /// what the export answered.
    #[cfg(unix)]
    fn cancelled_while_refused(sand: &Sandbox, files: &[PathBuf], locked: &Path) -> std::result::Result<(Engine, Result<Value>), CraftError> {
        use std::os::unix::fs::PermissionsExt;
        let cancel = CancelToken::new();
        let (stop, listed, locked) = (cancel.clone(), files.to_vec(), locked.to_path_buf());
        let engine = sand.start_running(Box::new(move |on_progress, list_outputs| {
            list_outputs(&listed)?;
            for file in &listed {
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(file, b"a whole movie").unwrap();
            }
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
            stop.cancel();
            assert!(!on_progress(1, 1, None));
            Ok(Value::Null)
        }))?;
        let answered = engine.request_with_progress("export", Value::Null, |_, _, _| {}, &cancel);
        Ok((engine, answered))
    }

    /// What the disk refuses a cancelled export at its end.
    #[cfg(unix)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Refused {
        /// Its files: their folder takes no changes, so they stay.
        Files,
        /// The folder it made: the folder above takes no changes, so the files go and the folder stays.
        Folder,
        /// The clear: the journal's folder takes no changes.
        Journal,
    }

    /// A cancelled export the disk refuses something at its end. A file it will not let go of is not
    /// answered as a plain cancel: the error names it, and it stays listed, so the next start removes
    /// it. A folder it made that will not go once its files have gone holds nothing of the export, so
    /// the plain cancel is answered, and it stays listed for the next start to remove. Entries the
    /// journal will not clear once every file has gone leave the plain cancel too, since nothing of
    /// the export is left to copy: they name a path that is empty now, and a later export that lists
    /// it takes the entry over, so the file that export finishes there survives the next start.
    #[cfg(unix)]
    #[test]
    fn a_cancelled_export_the_disk_refuses_at_its_end_says_so_only_while_a_file_of_it_is_left() -> Test {
        use std::os::unix::fs::PermissionsExt;
        for refused in [Refused::Files, Refused::Folder, Refused::Journal] {
            let sand = Sandbox::new()?;
            if !permissions_hold(&sand)? {
                return Ok(());
            }
            let (folder, file, sound) =
                (sand.folders.container.join("exports"), sand.folders.container.join("exports/card.mp4"), sand.folders.container.join("exports/card.wav"));
            let locked = match refused {
                Refused::Files => folder.clone(),
                Refused::Folder => sand.folders.container.clone(),
                Refused::Journal => sand.folders.config.clone(),
            };
            let (engine, answered) = cancelled_while_refused(&sand, &[file.clone(), sound.clone()], &locked)?;
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o700))?;
            let still = if refused == Refused::Folder { r#"{"path":"exports"}"# } else { "card.mp4" };
            assert!(fs::read_to_string(sand.file())?.contains(still), "{refused:?}: what the disk kept is no longer listed");
            match refused {
                Refused::Files => {
                    let why = io::Error::from_raw_os_error(13);
                    let named = format!(
                        "the export was cancelled but {} could not be removed ({why}), nor 1 more, so they stay listed in the pending-export journal and the next start tries again",
                        sand.real("exports/card.mp4").display()
                    );
                    assert_eq!(answered, Err(CraftError::io(named)));
                    assert!(file.exists() && sound.exists(), "a file the disk would not let go of is gone");
                }
                Refused::Folder => {
                    assert_eq!(answered, Err(CraftError::Cancelled), "a folder that holds nothing of the export was answered");
                    assert!(!file.exists() && !sound.exists() && folder.is_dir(), "the files went, or the folder the disk kept");
                }
                Refused::Journal => {
                    assert_eq!(answered, Err(CraftError::Cancelled));
                    assert!(!file.exists() && !sound.exists() && !folder.exists(), "the cancelled export left a file or its folder");
                    let mut later = sand.journal().pending();
                    later.list(std::slice::from_ref(&file))?;
                    sand.put("exports/card.mp4", b"the finished movie");
                    later.clear()?;
                }
            }
            engine.close()?;
            sand.journal().process_ended();
            let restarted = sand.start()?;
            if refused == Refused::Journal {
                assert_eq!(fs::read(&file)?, b"the finished movie", "the next start deleted a finished file: {:?}", restarted.recovery());
            } else {
                assert!(!file.exists() && !sound.exists() && !folder.exists(), "{refused:?}: {:?}", restarted.recovery());
            }
        }
        Ok(())
    }

    /// What left the entry of a path that a later export lists again.
    #[cfg(unix)]
    #[derive(Clone, Copy, Debug)]
    enum LeftBy {
        /// A start that could not delete the partial file a killed run wrote.
        Start,
        /// A cancelled export that could not delete its partial file.
        Cancel,
        /// A run killed before it wrote over the finished file at the path, whose entry no start has
        /// dealt with yet, since the start found another process holding the journal.
        KilledRunThatNeverWrote,
    }

    /// What an export that finishes writes at the path it lists.
    #[cfg(unix)]
    const FINISHED: &[u8] = b"the finished movie";

    /// How the export that lists the path again ends.
    #[cfg(unix)]
    #[derive(Clone, Copy, Debug)]
    enum Then {
        /// It writes the file whole and finishes.
        Finishes,
        /// It is cancelled before it writes.
        IsCancelled,
        /// It is killed before it writes.
        IsKilled,
    }

    #[cfg(unix)]
    impl Then {
        /// An export in `sand` that lists `path`, about to write it, and then ends so.
        fn end_an_export_of(self, sand: &Sandbox, path: &Path) -> Test {
            let mut export = sand.journal().pending();
            export.list(&[path.to_path_buf()])?;
            match self {
                Then::Finishes => {
                    fs::write(path, FINISHED)?;
                    export.clear()?;
                }
                Then::IsCancelled => export.remove()?,
                Then::IsKilled => export.killed(),
            }
            Ok(())
        }
    }

    /// An entry a start or a cancel could not deal with stays listed, and an export that lists its path
    /// again takes the entry over with what was there before the run that left it. The file that
    /// export finishes there survives the next start. One stopped before it writes, by a cancel or a
    /// kill, has the partial file the earlier run left deleted, and a finished file that was there
    /// before that run kept, as the earlier entry would have had them.
    #[cfg(unix)]
    #[test]
    fn an_export_that_lists_a_path_still_listed_takes_its_entry_over() -> Test {
        use std::os::unix::fs::PermissionsExt;
        // Every case is run and the faults listed together, so a fault in one never hides another.
        let mut faults = Vec::new();
        for left in [LeftBy::Start, LeftBy::Cancel, LeftBy::KilledRunThatNeverWrote] {
            for then in [Then::Finishes, Then::IsCancelled, Then::IsKilled] {
                let sand = Sandbox::new()?;
                if !permissions_hold(&sand)? {
                    return Ok(());
                }
                let (locked, path) = (sand.folders.container.join("locked"), sand.folders.container.join("locked/card.mp4"));
                let lock_up = |mode| fs::set_permissions(&locked, fs::Permissions::from_mode(mode));
                let engine = match left {
                    LeftBy::Start => {
                        sand.put("locked/card.mp4", b"half");
                        sand.write_journal_with(&[entry("locked/card.mp4")], &[entry("locked")]);
                        lock_up(0o500)?;
                        sand.start()?
                    }
                    LeftBy::Cancel => {
                        let (engine, answered) = cancelled_while_refused(&sand, std::slice::from_ref(&path), &locked)?;
                        assert!(answered.is_err(), "the cancelled export removed a file it could not");
                        engine
                    }
                    LeftBy::KilledRunThatNeverWrote => {
                        sand.put("locked/card.mp4", b"a finished movie");
                        let mut killed = sand.journal().pending();
                        killed.list(std::slice::from_ref(&path))?;
                        killed.killed();
                        let other = lock_handle(&sand)?;
                        other.lock()?;
                        sand.start()?
                    }
                };
                lock_up(0o700)?;
                assert!(fs::read_to_string(sand.file())?.contains("card.mp4"), "{left:?}: the path is not listed");
                then.end_an_export_of(&sand, &path)?;
                engine.close()?;
                sand.journal().process_ended();
                let restarted = sand.start()?;
                let due: Option<&[u8]> = match (then, left) {
                    (Then::Finishes, _) => Some(FINISHED),
                    (_, LeftBy::KilledRunThatNeverWrote) => Some(b"a finished movie"),
                    _ => None,
                };
                let now = fs::read(&path).ok();
                if now.as_deref() != due {
                    let (now, due) = (now.as_deref().map(String::from_utf8_lossy), due.map(String::from_utf8_lossy));
                    faults.push(format!("{left:?}, then {then:?}: the next start left {now:?} where {due:?} was due ({:?})", restarted.recovery()));
                }
            }
        }
        assert!(faults.is_empty(), "{faults:#?}");
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
        let engine = sand.start_running(script)?;
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
    /// replaces it, and says what it found. A listing that takes it, with no journal set aside,
    /// replaces it too.
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
            r#"{"outputs": [{"path": "card.mp4"}], "folders": "exports"}"#,
            r#"{"outputs": [{"path": "card.mp4"}], "folders": [{"path": 1}]}"#,
            r#"{"outputs": [{"path": "card.mp4"}], "folders": ["exports"]}"#,
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
        let sand = Sandbox::new()?;
        fs::write(sand.file(), "\u{0}\u{1}")?;
        let mut export = sand.journal().pending();
        export.list(&[sand.folders.container.join("card.mp4")])?;
        assert!(shelved(&sand)?.is_empty(), "a listing set aside a damaged journal with none waiting");
        let now = fs::read_to_string(sand.file())?;
        assert!(now.contains("card.mp4") && !now.contains(TAKEN), "the listing did not replace the damaged journal with its own: {now}");
        export.clear()?;
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
    /// the file is still found at the next start, and the folder its writer made goes after it.
    #[test]
    fn a_file_in_a_folder_made_after_it_was_listed_is_found_at_the_next_start() -> Test {
        let sand = Sandbox::new()?;
        let mut export = sand.journal().pending();
        export.list(&[sand.folders.container.join("card_001/card_00000.png")])?;
        sand.put("card_001/card_00000.png", b"half");
        export.killed();
        let engine = sand.start()?;
        assert_eq!(engine.recovery().removed, [sand.real("card_001/card_00000.png"), sand.real("card_001")], "{:?}", engine.recovery());
        Ok(())
    }

    /// The folders a listed file's path needs that are not there when it is listed are kept apart from
    /// the files, each once however many files of a sequence and however many listings name it, and a
    /// folder that is there is never kept: it is not the export's.
    #[test]
    fn the_folders_a_listing_needs_are_kept_apart_once_each() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        sand.put("kept/old.png", b"there before");
        let mut export = sand.journal().pending();
        export.list(&[container.join("seq/a/f_00000.png"), container.join("seq/a/f_00001.png"), container.join("kept/f_00000.png")])?;
        export.list(&[container.join("seq/a/f_00002.png"), container.join("seq/b/f_00000.png"), container.join("top.png")])?;
        let journal: Value = serde_json::from_slice(&fs::read(sand.file())?)?;
        assert_eq!(journal[FOLDERS], json!([{ PATH: "seq/a" }, { PATH: "seq" }, { PATH: "seq/b" }]), "{journal}");
        assert_eq!(journal[OUTPUTS].as_array().map(Vec::len), Some(6), "{journal}");
        export.clear()?;
        assert!(!sand.file().exists());
        // A listing of files whose folders are all there keeps no folder list at all.
        let mut plain = sand.journal().pending();
        plain.list(&[container.join("kept/f_00001.png"), container.join("top.png")])?;
        let journal: Value = serde_json::from_slice(&fs::read(sand.file())?)?;
        assert_eq!(journal.get(FOLDERS), None, "{journal}");
        plain.clear()?;
        Ok(())
    }

    /// The acceptance for folders: a killed export leaves the container as it found it. Its files go
    /// first, then each folder it made, the deepest first, once it holds nothing; a folder it made
    /// that holds a file it did not write stays, and so does a folder that was there before it.
    #[test]
    fn a_restart_removes_the_folders_a_killed_export_made_once_they_hold_nothing() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        sand.put("kept/old.png", b"there before");
        let listed = ["a/b/one.png", "a/b/two.png", "a/three.png", "kept/four.png", "shared/five.png"];
        let mut export = sand.journal().pending();
        export.list(&listed.map(|name| container.join(name)))?;
        // The writer reached all but `two.png`, and someone put a file of their own beside `five.png`.
        for name in ["a/b/one.png", "a/three.png", "kept/four.png", "shared/five.png"] {
            sand.put(name, b"half");
        }
        let theirs = sand.put("shared/notes.txt", b"not the export's");
        export.killed();

        let engine = sand.start()?;
        let recovery = engine.recovery();
        let real = |names: &[&str]| names.iter().map(|name| sand.real(name)).collect::<Vec<_>>();
        assert_eq!(recovery.removed, real(&["a/b/one.png", "a/three.png", "kept/four.png", "shared/five.png", "a/b", "a"]), "{recovery:?}");
        assert_eq!(recovery.untouched, real(&["shared"]), "{recovery:?}");
        assert!(recovery.refused.is_empty() && recovery.failed.is_empty() && recovery.journal.is_empty(), "{recovery:?}");
        assert!(!container.join("a").exists(), "a folder the export made was left");
        assert_eq!(fs::read(container.join("kept/old.png"))?, b"there before", "a folder that was there lost what it held");
        assert_eq!(fs::read(&theirs)?, b"not the export's", "a file the export did not write was deleted");
        assert!(!sand.file().exists(), "the journal was not cleared");
        Ok(())
    }

    /// Two exports that each listed a file below a folder neither found there leave it listed twice;
    /// a start deals with it once, and reports a folder it leaves once.
    #[test]
    fn a_folder_two_killed_exports_listed_is_dealt_with_and_reported_once() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        let (mut one, mut two) = (sand.journal().pending(), sand.journal().pending());
        one.list(&[container.join("shared/one.png")])?;
        two.list(&[container.join("shared/two.png")])?;
        sand.put("shared/one.png", b"half");
        sand.put("shared/notes.txt", b"not the export's");
        one.killed();
        two.killed();

        let engine = sand.start()?;
        assert_eq!(engine.recovery().removed, [sand.real("shared/one.png")], "{:?}", engine.recovery());
        assert_eq!(engine.recovery().untouched, [sand.real("shared")], "{:?}", engine.recovery());
        assert!(!sand.file().exists());
        Ok(())
    }

    /// A folder entry is removed only as the empty folder it names: one that is now a link, out of the
    /// container or not, or a file, is refused and nothing it reaches is touched, and one that is not
    /// there is no answer at all.
    #[cfg(unix)]
    #[test]
    fn a_listed_folder_is_never_removed_through_a_link_or_as_a_file() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        fs::write(sand.outside.join("photo.jpg"), b"not ours")?;
        std::os::unix::fs::symlink(&sand.outside, container.join("out"))?;
        fs::create_dir(container.join("inner"))?;
        std::os::unix::fs::symlink(container.join("inner"), container.join("linked"))?;
        sand.put("plain", b"a file where a folder was listed");
        sand.write_journal_with(&[], &[entry("out"), entry("linked"), entry("plain"), entry("gone"), entry("../outside")]);

        let engine = sand.start()?;
        let recovery = engine.recovery();
        assert!(recovery.removed.is_empty() && recovery.untouched.is_empty() && recovery.failed.is_empty(), "{recovery:?}");
        let why = |name: &str| recovery.refused.iter().find(|(path, _)| path.ends_with(name)).map(|(_, why)| why.as_str());
        assert_eq!(recovery.refused.len(), 4, "{recovery:?}");
        assert_eq!(why("out"), Some("it is a symbolic link, not a folder a writer makes"));
        assert_eq!(why("linked"), Some("it is a symbolic link, not a folder a writer makes"));
        assert_eq!(why("plain"), Some("it is a file, not a folder a writer makes"));
        assert!(why("../outside").is_some_and(|why| why.contains("it must be a plain name below it")), "{recovery:?}");
        assert_eq!(fs::read(sand.outside.join("photo.jpg"))?, b"not ours");
        assert!(fs::symlink_metadata(container.join("out"))?.file_type().is_symlink() && container.join("inner").is_dir());
        assert!(container.join("plain").is_file() && sand.outside.is_dir());
        assert!(!sand.file().exists(), "refused folders stayed listed");
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

    /// The folders an export made are kept through every way the journal stops being read: one the
    /// disk will not read at a start is set aside whole and a later start removes the folders it lists
    /// after its files; one it stops reading mid-run is replaced with the folders the process knows;
    /// and a journal of an older build, which has no folder list, or of a newer one, which has keys
    /// this one does not know, is read for what it lists.
    #[cfg(unix)]
    #[test]
    fn the_folders_an_export_made_survive_an_unreadable_journal_and_an_older_or_newer_one() -> Test {
        use std::os::unix::fs::PermissionsExt;
        let sand = Sandbox::new()?;
        if !permissions_hold(&sand)? {
            return Ok(());
        }
        let container = &sand.folders.container;
        let mut export = sand.journal().pending();
        export.list(&[container.join("seq/take/f_00000.png")])?;
        let (frame, folder) = (sand.put("seq/take/f_00000.png", b"half"), container.join("seq"));
        export.killed();
        fs::set_permissions(sand.file(), fs::Permissions::from_mode(0o000))?;
        let engine = sand.start()?;
        assert!(frame.exists(), "a file was deleted on the word of a journal nobody read: {:?}", engine.recovery());
        let shelf = shelved(&sand)?;
        assert_eq!(shelf.len(), 1, "the unread journal was not set aside");
        for one in &shelf {
            fs::set_permissions(one, fs::Permissions::from_mode(0o600))?;
        }
        let later = sand.start()?;
        assert_eq!(later.recovery().removed, [sand.real("seq/take/f_00000.png"), sand.real("seq/take"), sand.real("seq")], "{:?}", later.recovery());
        assert!(!folder.exists() && shelved(&sand)?.is_empty() && !sand.file().exists());

        // The disk stops reading the journal while two exports run: the second's listing writes the
        // first's folders with its own.
        let (mut first, mut second) = (sand.journal().pending(), sand.journal().pending());
        first.list(&[container.join("made/f_00000.png")])?;
        fs::set_permissions(sand.file(), fs::Permissions::from_mode(0o000))?;
        second.list(&[container.join("other.png")])?;
        let now: Value = serde_json::from_slice(&fs::read(sand.file())?)?;
        assert_eq!(now[FOLDERS], json!([entry("made")]), "{now}");
        first.clear()?;
        second.clear()?;
        assert!(!sand.file().exists());

        // A newer build writes keys this build does not read, and an older one wrote no folder list.
        let old = sand.put("old.mp4", b"half");
        let made = sand.put("new/f.png", b"half");
        fs::write(sand.file(), json!({ OUTPUTS: [entry("new/f.png")], FOLDERS: [entry("new")], "version": 9 }).to_string())?;
        let engine = sand.start()?;
        assert!(!made.exists() && !container.join("new").exists() && old.exists(), "{:?}", engine.recovery());
        assert_eq!(engine.recovery().removed, [sand.real("new/f.png"), sand.real("new")]);
        sand.write_journal(&[entry("old.mp4")]);
        let engine = sand.start()?;
        assert!(!old.exists() && engine.recovery().removed == [sand.real("old.mp4")], "{:?}", engine.recovery());
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
                    Engine::start_in(&folders, Scripted::idle(), GpuChoice::CpuOnly).unwrap();
                }
                Ok(json!({"done": true}))
            });
            let engine = sand.start_running(script)?;
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

    /// Several journals set aside: while one cannot be read, a start deals with none of them and says
    /// so of each, removing none; once every one reads, it deals with what each lists, keeps in the
    /// journal what it could not delete so nothing listed is lost, removes them all, and says one line
    /// for each.
    #[cfg(unix)]
    #[test]
    fn several_set_aside_journals_are_each_dealt_with_and_answered() -> Test {
        let sand = Sandbox::new()?;
        if !permissions_hold(&sand)? {
            return Ok(());
        }
        let shelf = |n: u32| sand.folders.config.join(format!("{SHELF_PREFIX}0-{n}.json"));
        let partial = sand.put("card.mp4", b"half");
        let stuck = sand.put("locked/frame.png", b"half");
        fs::write(shelf(1), json!({ OUTPUTS: [entry("card.mp4")] }).to_string())?;
        fs::write(shelf(2), json!({ OUTPUTS: [entry("locked/frame.png")] }).to_string())?;
        fs::write(shelf(3), json!({ OUTPUTS: [entry("later.mp4")] }).to_string())?;
        set_mode(&shelf(3), 0o000)?;
        let real = |n: u32| real_shelf(&sand, &shelf(n));

        let first = sand.start()?;
        assert_eq!(first.recovery().journal, [stays_unread(&real(3)?), waits(&real(1)?), waits(&real(2)?)]);
        assert!(partial.exists() && stuck.exists() && first.recovery().removed.is_empty(), "{:?}", first.recovery());
        assert_eq!(shelved(&sand)?.len(), 3, "a set-aside journal that waits was removed");

        set_mode(&shelf(3), 0o600)?;
        let locked = sand.folders.container.join("locked");
        set_mode(&locked, 0o500)?;
        let engine = sand.start()?;
        set_mode(&locked, 0o700)?;
        let recovery = engine.recovery();
        assert!(!partial.exists() && stuck.exists(), "{recovery:?}");
        assert_eq!(recovery.removed, [sand.real("card.mp4")]);
        assert_eq!(recovery.failed.len(), 1, "{recovery:?}");
        assert!(fs::read_to_string(sand.file())?.contains("locked/frame.png"), "what a set-aside journal listed was lost");
        assert!(shelved(&sand)?.is_empty(), "a set-aside journal that was read was kept");
        assert_eq!(recovery.journal, [read_and_removed(&real(1)?), read_and_removed(&real(2)?), read_and_removed(&real(3)?)]);
        Ok(())
    }

    /// At most [`MAX_SET_ASIDE`] journals wait set aside: past that a start leaves the unread journal
    /// where it is and refuses listings, naming it, and a later start that reads it deals with it. One
    /// set aside that reads waits with the rest, and says so; one that is no journal stays too, since
    /// nothing records yet what its going costs.
    #[cfg(unix)]
    #[test]
    fn set_aside_journals_are_bounded() -> Test {
        use std::os::unix::fs::PermissionsExt;
        let sand = Sandbox::new()?;
        if !permissions_hold(&sand)? {
            return Ok(());
        }
        let shelf = |n: usize| sand.folders.config.join(format!("{SHELF_PREFIX}0-{n}.json"));
        for n in 0..MAX_SET_ASIDE {
            fs::write(shelf(n), if n == 1 { b"{".as_slice() } else { b"{\"outputs\": []}".as_slice() })?;
            fs::set_permissions(shelf(n), fs::Permissions::from_mode(if n < 2 { 0o600 } else { 0o000 }))?;
        }
        let partial = sand.put("card.mp4", b"half");
        sand.write_journal(&[entry("card.mp4")]);
        fs::set_permissions(sand.file(), fs::Permissions::from_mode(0o000))?;

        let engine = sand.start()?;
        assert_eq!(shelved(&sand)?.len(), MAX_SET_ASIDE, "a journal was set aside past the bound, or one went while nothing was written");
        assert!(sand.file().exists() && partial.exists());
        assert!(engine.recovery().journal.iter().any(|line| line.contains("cannot be set aside")), "{:?}", engine.recovery());
        let not_json = serde_json::from_slice::<Value>(b"{").err().ok_or("the damage reads as JSON")?;
        let kept = format!(
            "the pending-export journal {} is not JSON: {not_json}; it names no file to delete, and goes when a start can write the journal",
            real_shelf(&sand, &shelf(1))?.display()
        );
        assert!(engine.recovery().journal.ends_with(&[waits(&real_shelf(&sand, &shelf(0))?), kept]), "{:?}", engine.recovery());
        assert_listing_refused(&sand, "until a start reads it");

        fs::set_permissions(sand.file(), fs::Permissions::from_mode(0o600))?;
        assert_start_frees_the_journal(&sand)
    }

    /// A journal that is no journal never waits set aside, so it refuses nothing however many wait: with
    /// [`MAX_SET_ASIDE`] set aside and one of them unreadable, a listing and a start each replace it by
    /// one that records that every set-aside entry of a file yields, and once every one reads, none of
    /// the files they list is deleted.
    #[cfg(unix)]
    #[test]
    fn a_journal_that_is_no_journal_is_replaced_however_many_wait() -> Test {
        let sand = Sandbox::new()?;
        if !permissions_hold(&sand)? {
            return Ok(());
        }
        let shelf = |n: usize| sand.folders.config.join(format!("{SHELF_PREFIX}0-{n}.json"));
        for n in 0..MAX_SET_ASIDE {
            sand.put(&format!("p{n}.mp4"), b"half");
            fs::write(shelf(n), json!({ OUTPUTS: [entry(&format!("p{n}.mp4"))] }).to_string())?;
        }
        set_mode(&shelf(0), 0o000)?;
        let every = || -> std::result::Result<bool, Box<dyn std::error::Error>> {
            Ok(serde_json::from_slice::<Value>(&fs::read(sand.file())?)?.get(TAKEN) == Some(&json!(EVERY)))
        };
        fs::write(sand.file(), b"{")?;
        let mut export = sand.journal().pending();
        export.list(&[sand.folders.container.join("new.mp4")])?;
        export.clear()?;
        assert!(every()?, "the listing did not record that every set-aside entry yields");
        fs::write(sand.file(), b"{")?;
        let start = sand.start()?;
        assert!(!start.recovery().journal.iter().any(|line| line.contains("cannot be set aside")), "{:?}", start.recovery());
        assert!(every()? && shelved(&sand)?.len() == MAX_SET_ASIDE, "{:?}", start.recovery());

        set_mode(&shelf(0), 0o600)?;
        let later = sand.start()?;
        let recovery = later.recovery();
        assert!((0..MAX_SET_ASIDE).all(|n| sand.folders.container.join(format!("p{n}.mp4")).exists()), "{recovery:?}");
        assert_eq!(recovery.journal.first(), Some(&none_deleted(&sand.real("p0.mp4"), MAX_SET_ASIDE - 1)), "{recovery:?}");
        assert!(shelved(&sand)?.is_empty() && !sand.file().exists(), "a journal outlived the start that dealt with it");
        Ok(())
    }

    /// Give `path` the permissions `mode` spells.
    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }

    /// What a start says of `shelf`, a set-aside journal it read, while one cannot be read now.
    #[cfg(unix)]
    fn waits(shelf: &Path) -> String {
        format!("the set-aside journal {} waits for those that cannot be read now, which may hold a later export of a file it lists", shelf.display())
    }

    /// What a start says of `shelf`, a set-aside journal it read, dealt with and removed.
    #[cfg(unix)]
    fn read_and_removed(shelf: &Path) -> String {
        format!("read the set-aside journal {}, dealt with what it listed and removed it", shelf.display())
    }

    /// How a start names `first` and `more` other files set-aside journals list, as the recovery names
    /// files.
    #[cfg(unix)]
    fn named_files(first: &Path, more: usize) -> String {
        let more = if more == 0 { String::new() } else { format!(" and {more} more files") };
        format!("{}{more}", first.display())
    }

    /// What a start says of the entries of `first` and `more` other files that set-aside journals
    /// list and that yielded to the exports that ended there.
    #[cfg(unix)]
    fn yields_to_exports(first: &Path, more: usize) -> String {
        format!(
            "the set-aside journals' entries of {} yield to the exports that ended there while they waited, so what is there is kept",
            named_files(first, more)
        )
    }

    /// What a start says of `first` and `more` other files set-aside journals list once every set-aside
    /// entry of a file yields, past the record's bound or after a journal that is no journal.
    #[cfg(unix)]
    fn none_deleted(first: &Path, more: usize) -> String {
        format!(
            "an export may have ended at any file the set-aside journals list while they waited, since more than {MAX_TAKEN} files were exported meanwhile or a journal was no journal, so none of them is deleted: {}",
            named_files(first, more)
        )
    }

    /// What a start says of the journal it replaces, `reason` being why it is no journal, while
    /// journals may wait set aside.
    #[cfg(unix)]
    fn replaced_while_one_waits(reason: &str) -> String {
        format!(
            "{reason}; it names no file to delete, so it is replaced, and since it may have held a later export of a file a set-aside journal lists, none of those is deleted"
        )
    }

    /// What a start says of `shelf`, a set-aside journal its user may not read.
    #[cfg(unix)]
    fn stays_unread(shelf: &Path) -> String {
        format!("cannot read the pending-export journal {}: {}; it stays set aside for a later start", shelf.display(), io::Error::from_raw_os_error(13))
    }

    /// Where `shelf`, a journal set aside in `sand`, is as a recovery names it.
    #[cfg(unix)]
    fn real_shelf(sand: &Sandbox, shelf: &Path) -> std::result::Result<PathBuf, Box<dyn std::error::Error>> {
        Ok(fs::canonicalize(&sand.folders.config)?.join(shelf.file_name().ok_or("a set-aside journal has no name")?))
    }

    /// What a killed run left, its journal set aside unread by the start after it: partial files at
    /// `card`, `card.mp4`, and `frame`, `frames/f_00000.png`, which it listed with nothing there, in a
    /// folder it made.
    #[cfg(unix)]
    struct AfterAKill {
        sand: Sandbox,
        /// The set-aside journal, which the disk still will not read.
        shelf: PathBuf,
        card: PathBuf,
        frame: PathBuf,
    }

    #[cfg(unix)]
    impl AfterAKill {
        /// None for a user the permissions do not hold back (root), for whom the disk never refuses
        /// to read the journal.
        fn new() -> std::result::Result<Option<Self>, Box<dyn std::error::Error>> {
            let sand = Sandbox::new()?;
            if !permissions_hold(&sand)? {
                return Ok(None);
            }
            let (card, frame) = (sand.folders.container.join("card.mp4"), sand.folders.container.join("frames/f_00000.png"));
            let mut killed = sand.journal().pending();
            killed.list(&[card.clone(), frame.clone()])?;
            sand.put("card.mp4", b"half");
            sand.put("frames/f_00000.png", b"half");
            killed.killed();
            set_mode(&sand.file(), 0o000)?;
            sand.start()?;
            let shelf = match shelved(&sand)?.as_slice() {
                [shelf] => shelf.clone(),
                other => return Err(format!("the unread journal was not set aside: {other:?}").into()),
            };
            Ok(Some(Self { sand, shelf, card, frame }))
        }
    }

    /// The journal set aside in `sand` beside `older`, the one a later start set aside.
    #[cfg(unix)]
    fn newer_than(sand: &Sandbox, older: &Path) -> std::result::Result<PathBuf, Box<dyn std::error::Error>> {
        Ok(shelved(sand)?.into_iter().find(|shelf| shelf != older).ok_or("the journal was not set aside")?)
    }

    /// The acceptance: a journal the disk would not read is set aside, so no export can take its
    /// entries over as it lists, and an export lists one of its files again. Once a start can read it,
    /// the file that export finished stays, and the start says so; one cancelled or killed before it
    /// wrote has the partial file the killed run left deleted, as the take-over would have; and every
    /// other entry the set-aside journal lists is dealt with as before, the journal cleared.
    #[cfg(unix)]
    #[test]
    fn a_set_aside_entry_yields_to_a_later_export_of_its_file() -> Test {
        // Every case is run and the faults listed together, so a fault in one never hides another.
        let mut faults = Vec::new();
        for then in [Then::Finishes, Then::IsCancelled, Then::IsKilled] {
            let Some(AfterAKill { sand, shelf, card, .. }) = AfterAKill::new()? else { return Ok(()) };
            then.end_an_export_of(&sand, &card)?;
            set_mode(&shelf, 0o600)?;
            let read = read_and_removed(&real_shelf(&sand, &shelf)?);
            sand.journal().process_ended();
            let restarted = sand.start()?;
            let recovery = restarted.recovery();
            let (due, removed, lines): (Option<&[u8]>, &[&str], Vec<String>) = match then {
                Then::Finishes => (Some(FINISHED), &["frames/f_00000.png", "frames"], vec![yields_to_exports(&sand.real("card.mp4"), 0), read]),
                Then::IsCancelled | Then::IsKilled => (None, &["card.mp4", "frames/f_00000.png", "frames"], vec![read]),
            };
            let now = fs::read(&card).ok();
            if now.as_deref() != due {
                let (now, due) = (now.as_deref().map(String::from_utf8_lossy), due.map(String::from_utf8_lossy));
                faults.push(format!("{then:?}: the start left {now:?} where {due:?} was due ({recovery:?})"));
            }
            if recovery.removed != removed.iter().map(|name| sand.real(name)).collect::<Vec<_>>() || recovery.journal != lines {
                faults.push(format!("{then:?}: the other set-aside entries were not dealt with as before: {recovery:?}"));
            }
            if !shelved(&sand)?.is_empty() || sand.file().exists() {
                faults.push(format!("{then:?}: a journal outlived the start that dealt with it"));
            }
        }
        assert!(faults.is_empty(), "{faults:#?}");
        Ok(())
    }

    /// A set-aside entry may yield to an export another journal records, so a start deals with what
    /// the set-aside journals list only once it reads every one and the journal: while one cannot be
    /// read they all wait, unremoved, and the file the export finished stays. Here the record is in
    /// the journal, which the disk then stops reading, so a start sets it aside in its turn.
    #[cfg(unix)]
    #[test]
    fn set_aside_journals_wait_while_one_cannot_be_read() -> Test {
        let Some(AfterAKill { sand, shelf: older, card, frame }) = AfterAKill::new()? else { return Ok(()) };
        Then::Finishes.end_an_export_of(&sand, &card)?;
        set_mode(&older, 0o600)?;
        set_mode(&sand.file(), 0o000)?;
        let waits = waits(&real_shelf(&sand, &older)?);
        let untouched = |when: &str, recovery: &Recovery| -> Test {
            assert_eq!(fs::read(&card)?, FINISHED, "{when}: the file a later export finished was deleted: {recovery:?}");
            assert_eq!(fs::read(&frame)?, b"half", "{when}: a set-aside entry was dealt with while a journal could not be read: {recovery:?}");
            assert_eq!(shelved(&sand)?.len(), 2, "{when}: a set-aside journal that waits was removed");
            Ok(())
        };

        let first = sand.start()?;
        untouched("the start that set the journal aside", first.recovery())?;
        let newer = real_shelf(&sand, &newer_than(&sand, &older)?)?;
        let set_aside = format!(
            "cannot read the pending-export journal {}: {}; it is set aside as {} for a later start",
            fs::canonicalize(&sand.folders.config)?.join(JOURNAL_FILE).display(),
            io::Error::from_raw_os_error(13),
            newer.display()
        );
        assert_eq!(first.recovery().journal, [set_aside, waits.clone()]);
        let second = sand.start()?;
        untouched("the start that could not read it", second.recovery())?;
        assert_eq!(second.recovery().journal, [stays_unread(&newer), waits]);

        set_mode(&newer, 0o600)?;
        let third = sand.start()?;
        assert_eq!(fs::read(&card)?, FINISHED, "the file a later export finished was deleted: {:?}", third.recovery());
        assert_eq!(third.recovery().removed, [sand.real("frames/f_00000.png"), sand.real("frames")], "{:?}", third.recovery());
        assert!(shelved(&sand)?.is_empty() && !sand.file().exists(), "a journal outlived the start that dealt with it");
        Ok(())
    }

    /// Where a journal that is no journal lies, which may have recorded the export a set-aside entry
    /// yields to.
    #[cfg(unix)]
    #[derive(Clone, Copy, Debug)]
    enum NoJournal {
        /// The journal, read by a start that reads every set-aside journal.
        TheJournal,
        /// A journal set aside unread, which the disk has damaged since, read by a start while
        /// another cannot be read.
        ASetAsideOne,
        /// The journal, read by a start that cannot read a set-aside journal.
        TheJournalWhileOneCannotBeRead,
        /// The journal, read by a listing, which never reads the set-aside journals.
        TheJournalAtAListing,
    }

    /// A journal that is no journal may have recorded an export a set-aside entry yields to, so it
    /// costs no file: the journal records that every set-aside entry of a file yields, the journal that
    /// is no journal is replaced or removed and never waits, and once every journal reads, none of the
    /// files the set-aside journals list is deleted.
    #[cfg(unix)]
    #[test]
    fn a_journal_that_is_no_journal_costs_no_file_a_set_aside_journal_lists() -> Test {
        let garbage = br#"{"outputs": [{"path": "card"#;
        let not_json = serde_json::from_slice::<Value>(garbage).err().ok_or("the damage reads as JSON")?;
        let mut faults = Vec::new();
        for no_journal in [NoJournal::TheJournal, NoJournal::ASetAsideOne, NoJournal::TheJournalWhileOneCannotBeRead, NoJournal::TheJournalAtAListing] {
            let Some(AfterAKill { sand, shelf: older, card, frame }) = AfterAKill::new()? else { return Ok(()) };
            Then::Finishes.end_an_export_of(&sand, &card)?;
            let damaged = |file: &Path| format!("the pending-export journal {} is not JSON: {not_json}", file.display());
            let journal = fs::canonicalize(&sand.folders.config)?.join(JOURNAL_FILE);
            let older_real = real_shelf(&sand, &older)?;
            let mut lines = Vec::new();
            // What a start said while the older set-aside journal could not be read, with what was due.
            let mut meanwhile = Vec::new();
            match no_journal {
                NoJournal::TheJournal => {
                    fs::write(sand.file(), garbage)?;
                    lines.push(replaced_while_one_waits(&damaged(&journal)));
                }
                NoJournal::ASetAsideOne => {
                    set_mode(&sand.file(), 0o000)?;
                    sand.start()?;
                    let shelf = newer_than(&sand, &older)?;
                    set_mode(&shelf, 0o600)?;
                    fs::write(&shelf, garbage)?;
                    let removed = format!("{}; it names no file to delete, so it is removed", damaged(&real_shelf(&sand, &shelf)?));
                    meanwhile.push((sand.start()?.recovery().journal.clone(), vec![stays_unread(&older_real), removed]));
                }
                NoJournal::TheJournalWhileOneCannotBeRead => {
                    fs::write(sand.file(), garbage)?;
                    meanwhile.push((sand.start()?.recovery().journal.clone(), vec![stays_unread(&older_real), replaced_while_one_waits(&damaged(&journal))]));
                    meanwhile.push((sand.start()?.recovery().journal.clone(), vec![stays_unread(&older_real)]));
                }
                NoJournal::TheJournalAtAListing => {
                    fs::write(sand.file(), garbage)?;
                    let mut other = sand.journal().pending();
                    other.list(&[sand.folders.container.join("other.mp4")])?;
                    other.clear()?;
                }
            }
            for (said, due) in meanwhile {
                if said != due {
                    faults.push(format!("{no_journal:?}: while one could not be read, the start said {said:?}"));
                }
            }
            if !matches!(no_journal, NoJournal::TheJournal) {
                let record = serde_json::from_slice::<Value>(&fs::read(sand.file())?).ok().and_then(|now| now.get(TAKEN).cloned());
                if shelved(&sand)? != [older.clone()] || record != Some(json!(EVERY)) {
                    faults.push(format!("{no_journal:?}: what is no journal waits, or the journal does not record that every entry yields: {record:?}"));
                }
            }
            set_mode(&older, 0o600)?;
            let last = sand.start()?;
            lines.extend([none_deleted(&sand.real("card.mp4"), 1), read_and_removed(&older_real)]);
            let recovery = last.recovery();
            if fs::read(&card).ok().as_deref() != Some(FINISHED) || fs::read(&frame).ok().as_deref() != Some(b"half".as_slice()) {
                faults.push(format!("{no_journal:?}: a file a set-aside journal lists was deleted on the word of what is no journal: {recovery:?}"));
            }
            if recovery.journal != lines || !recovery.removed.is_empty() || !shelved(&sand)?.is_empty() || sand.file().exists() {
                faults.push(format!("{no_journal:?}: the start did not answer and clear as due: {recovery:?}"));
            }
        }
        assert!(faults.is_empty(), "{faults:#?}");
        Ok(())
    }

    /// A start writes what the set-aside entries yield to with the journal before it removes the
    /// journals set aside, so one the disk will not remove, which the next start reads again, still
    /// yields. Here the record was in another set-aside journal, which went.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_set_aside_journal_the_disk_keeps_yields_again_at_the_next_start() -> Test {
        let Some(AfterAKill { sand, shelf: older, card, .. }) = AfterAKill::new()? else { return Ok(()) };
        Then::Finishes.end_an_export_of(&sand, &card)?;
        set_mode(&older, 0o600)?;
        set_mode(&sand.file(), 0o000)?;
        sand.start()?;
        set_mode(&newer_than(&sand, &older)?, 0o600)?;
        // The system's flag that keeps a file from being removed, even by its owner, while it reads.
        let sand = sand.running("chflags", &["uchg".as_ref(), older.as_os_str()])?;
        let first = sand.start();
        let sand = sand.running("chflags", &["nouchg".as_ref(), older.as_os_str()])?;
        let first = first?;
        let kept = format!("cannot remove the set-aside journal {}: {}", real_shelf(&sand, &older)?.display(), io::Error::from_raw_os_error(1));
        assert!(first.recovery().journal.contains(&kept), "{:?}", first.recovery());
        assert_eq!(fs::read(&card)?, FINISHED, "the file a later export finished was deleted: {:?}", first.recovery());

        let second = sand.start()?;
        assert_eq!(fs::read(&card)?, FINISHED, "a set-aside journal read again deleted the file a later export finished: {:?}", second.recovery());
        assert!(second.recovery().removed.is_empty(), "{:?}", second.recovery());
        assert!(shelved(&sand)?.is_empty() && !sand.file().exists(), "a journal outlived the start that dealt with it");
        Ok(())
    }

    /// A set-aside journal that is no journal goes only once the journal records what its going costs,
    /// that every set-aside entry of a file yields: while the disk will not take the journal's write it
    /// stays, and the start says so, so the export it may have recorded keeps its file. Here it did
    /// record one, the export that finished at a file the older set-aside journal lists.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_set_aside_journal_that_is_no_journal_goes_only_once_the_journal_is_written() -> Test {
        let Some(AfterAKill { sand, shelf: older, card, frame }) = AfterAKill::new()? else { return Ok(()) };
        Then::Finishes.end_an_export_of(&sand, &card)?;
        set_mode(&sand.file(), 0o000)?;
        sand.start()?;
        let newer = newer_than(&sand, &older)?;
        set_mode(&newer, 0o600)?;
        fs::write(&newer, b"{")?;
        let not_json = serde_json::from_slice::<Value>(b"{").err().ok_or("the damage reads as JSON")?;
        let damaged = format!("the pending-export journal {} is not JSON: {not_json}", real_shelf(&sand, &newer)?.display());
        let unread = stays_unread(&real_shelf(&sand, &older)?);
        // A journal the disk reads, but will not let a start replace or remove.
        let journal = sand.file();
        fs::write(&journal, json!({ OUTPUTS: [] }).to_string())?;
        let sand = sand.running("chflags", &["uchg".as_ref(), journal.as_os_str()])?;
        let first = sand.start();
        let sand = sand.running("chflags", &["nouchg".as_ref(), journal.as_os_str()])?;
        let first = first?;
        let uncleared = format!(
            "cannot clear the pending-export journal {}: {}",
            fs::canonicalize(&sand.folders.config)?.join(JOURNAL_FILE).display(),
            io::Error::from_raw_os_error(1)
        );
        let kept = format!("{damaged}; it names no file to delete, and goes when a start can write the journal");
        assert_eq!(first.recovery().journal, [unread.clone(), uncleared, kept]);
        assert_eq!(shelved(&sand)?.len(), 2, "a journal that is no journal went before the journal recorded what that costs");

        let second = sand.start()?;
        assert_eq!(second.recovery().journal, [unread, format!("{damaged}; it names no file to delete, so it is removed")]);
        set_mode(&older, 0o600)?;
        let third = sand.start()?;
        assert_eq!(fs::read(&card)?, FINISHED, "the file a later export finished was deleted: {:?}", third.recovery());
        assert_eq!(fs::read(&frame)?, b"half", "{:?}", third.recovery());
        assert!(shelved(&sand)?.is_empty() && !sand.file().exists(), "a journal outlived the start that dealt with it");
        Ok(())
    }

    /// What set-aside entries yield to is kept only while a journal waits set aside: a journal that
    /// holds it with none waiting, as a start killed before it dropped it leaves one, is read for its
    /// entries alone, and the next change of it, a listing's or a start's, drops it.
    #[test]
    fn what_set_aside_entries_yield_to_goes_with_the_last_of_them() -> Test {
        let stale = json!([entry("card.mp4"), entry("frames/f_00000.png")]);
        let sand = Sandbox::new()?;
        fs::write(sand.file(), json!({ OUTPUTS: [], TAKEN: stale }).to_string())?;
        let mut export = sand.journal().pending();
        export.list(&[sand.folders.container.join("other.mp4")])?;
        assert!(!fs::read_to_string(sand.file())?.contains(TAKEN), "a listing kept what set-aside entries yield to with none waiting");
        export.clear()?;
        assert!(!sand.file().exists(), "what set-aside entries yield to outlived the last of them");

        let partial = sand.put("card.mp4", b"half");
        fs::write(sand.file(), json!({ OUTPUTS: [entry("card.mp4")], TAKEN: stale }).to_string())?;
        let engine = sand.start()?;
        assert!(!partial.exists(), "an entry of the journal yielded: {:?}", engine.recovery());
        assert!(!sand.file().exists(), "what set-aside entries yield to outlived the last of them");
        Ok(())
    }

    /// The journal's own entries are later than every set-aside one, so each is dealt with by its own
    /// record, never a set-aside one's: an export killed after another finished at a file a set-aside
    /// journal lists has its partial file deleted when it wrote one, and when it never did, the
    /// finished file is kept, which the older record would delete.
    #[cfg(unix)]
    #[test]
    fn an_export_killed_after_one_a_set_aside_entry_yields_to_is_dealt_with_by_its_own_record() -> Test {
        let mut faults = Vec::new();
        for wrote in [true, false] {
            let Some(AfterAKill { sand, shelf, card, .. }) = AfterAKill::new()? else { return Ok(()) };
            Then::Finishes.end_an_export_of(&sand, &card)?;
            let mut killed = sand.journal().pending();
            killed.list(std::slice::from_ref(&card))?;
            if wrote {
                fs::write(&card, b"half a newer movie")?;
            }
            killed.killed();
            set_mode(&shelf, 0o600)?;
            let restarted = sand.start()?;
            let due = if wrote { None } else { Some(FINISHED) };
            let now = fs::read(&card).ok();
            if now.as_deref() != due {
                let (now, due) = (now.as_deref().map(String::from_utf8_lossy), due.map(String::from_utf8_lossy));
                faults.push(format!("wrote: {wrote}: the start left {now:?} where {due:?} was due ({:?})", restarted.recovery()));
            }
        }
        assert!(faults.is_empty(), "{faults:#?}");
        Ok(())
    }

    /// The record says nothing of when an export ended, so an entry listed after one, in a journal set
    /// aside in its turn, yields to it too: the partial file its killed run wrote there is left, never
    /// a finished file deleted, and the start names the files that yielded.
    #[cfg(unix)]
    #[test]
    fn an_entry_set_aside_after_an_export_it_yields_to_leaves_its_partial_file_and_says_so() -> Test {
        let Some(AfterAKill { sand, card, frame, .. }) = AfterAKill::new()? else { return Ok(()) };
        for file in [&card, &frame] {
            Then::Finishes.end_an_export_of(&sand, file)?;
        }
        let mut killed = sand.journal().pending();
        killed.list(std::slice::from_ref(&card))?;
        fs::write(&card, b"half a newer movie")?;
        killed.killed();
        set_mode(&sand.file(), 0o000)?;
        sand.start()?;
        for shelf in shelved(&sand)? {
            set_mode(&shelf, 0o600)?;
        }
        let later = sand.start()?;
        let recovery = later.recovery();
        assert_eq!(fs::read(&card)?, b"half a newer movie", "{recovery:?}");
        assert_eq!(fs::read(&frame)?, FINISHED, "a finished file was deleted: {recovery:?}");
        assert!(recovery.removed.is_empty(), "{recovery:?}");
        assert_eq!(recovery.journal.first(), Some(&yields_to_exports(&sand.real("card.mp4"), 1)), "{recovery:?}");
        assert!(shelved(&sand)?.is_empty() && !sand.file().exists(), "a journal outlived the start that dealt with it");
        Ok(())
    }

    /// The record names at most [`MAX_TAKEN`] files, and past that records that every set-aside entry
    /// of a file yields, so a journal set aside that never reads again cannot grow the journal until
    /// every listing is refused. Once it reads, none of the files it lists is deleted, the start says
    /// which and why, and the record goes.
    #[cfg(unix)]
    #[test]
    fn what_set_aside_entries_yield_to_is_bounded() -> Test {
        let Some(AfterAKill { sand, shelf, card, frame }) = AfterAKill::new()? else { return Ok(()) };
        let taken: Vec<Value> = (1..MAX_TAKEN).map(|n| entry(&format!("other-{n}.mp4"))).collect();
        fs::write(sand.file(), json!({ OUTPUTS: [], TAKEN: taken }).to_string())?;
        let recorded =
            || -> std::result::Result<Value, Box<dyn std::error::Error>> { Ok(serde_json::from_slice::<Value>(&fs::read(sand.file())?)?[TAKEN].clone()) };
        Then::Finishes.end_an_export_of(&sand, &sand.folders.container.join("other-0.mp4"))?;
        assert_eq!(recorded()?.as_array().map(Vec::len), Some(MAX_TAKEN), "the record stopped naming files before its bound");
        Then::Finishes.end_an_export_of(&sand, &sand.folders.container.join("one-more.mp4"))?;
        assert_eq!(recorded()?, json!(EVERY), "the record grew past its bound");

        set_mode(&shelf, 0o600)?;
        let later = sand.start()?;
        let recovery = later.recovery();
        let half = Some(b"half".to_vec());
        assert_eq!((fs::read(&card).ok(), fs::read(&frame).ok()), (half.clone(), half), "a file a set-aside journal lists was deleted: {recovery:?}");
        assert_eq!(recovery.journal, [none_deleted(&sand.real("card.mp4"), 1), read_and_removed(&real_shelf(&sand, &shelf)?)]);
        assert!(recovery.removed.is_empty(), "{recovery:?}");
        assert!(shelved(&sand)?.is_empty() && !sand.file().exists(), "the record outlived the last journal set aside");
        Ok(())
    }

    /// What runs while the disk will not list the journal's folder.
    #[cfg(unix)]
    #[derive(Clone, Copy, Debug)]
    enum UnlistedDuring {
        /// The end of an export of a file a set-aside journal lists.
        ExportEnd,
        /// A later listing's take, with the record of that export in the journal.
        Listing,
        /// A later listing's take, with the record in a journal that is no journal now.
        ListingWithNoJournal,
        /// A start, with the record in the journal.
        Start,
        /// A start, with the record in a journal that is no journal now.
        StartWithNoJournal,
    }

    #[cfg(unix)]
    impl UnlistedDuring {
        fn meets_no_journal(self) -> bool {
            matches!(self, UnlistedDuring::ListingWithNoJournal | UnlistedDuring::StartWithNoJournal)
        }
    }

    /// A folder the disk will not list may hold journals set aside, so it never counts as holding
    /// none: an export that ends records its files, a take keeps the record, a start that cannot list
    /// them waits and says so, and a journal that is no journal is replaced by one that records that
    /// every set-aside entry of a file yields, refusing nothing. Once the disk lists the folder again
    /// the file the export finished stays.
    #[cfg(unix)]
    #[test]
    fn a_journal_folder_the_disk_will_not_list_may_hold_journals_set_aside() -> Test {
        let mut faults = Vec::new();
        for when in [
            UnlistedDuring::ExportEnd,
            UnlistedDuring::Listing,
            UnlistedDuring::ListingWithNoJournal,
            UnlistedDuring::Start,
            UnlistedDuring::StartWithNoJournal,
        ] {
            let Some(AfterAKill { sand, shelf, card, frame }) = AfterAKill::new()? else { return Ok(()) };
            // An engine's journal is made at its start, while the folder lists.
            let (journal, config) = (sand.journal(), sand.folders.config.clone());
            let real = fs::canonicalize(&config)?;
            let unlisted = format!(
                "cannot list the journals set aside in {}: {}; they stay set aside for a later start",
                real.display(),
                io::Error::from_raw_os_error(13)
            );
            let garbage = b"{";
            let not_json = serde_json::from_slice::<Value>(garbage).err().ok_or("the damage reads as JSON")?;
            let damaged = format!("the pending-export journal {} is not JSON: {not_json}", real.join(JOURNAL_FILE).display());
            let read = read_and_removed(&real_shelf(&sand, &shelf)?);
            if matches!(when, UnlistedDuring::ExportEnd) {
                let mut export = journal.pending();
                export.list(std::slice::from_ref(&card))?;
                fs::write(&card, FINISHED)?;
                set_mode(&config, 0o300)?;
                let cleared = export.clear();
                set_mode(&config, 0o700)?;
                cleared?;
            } else {
                Then::Finishes.end_an_export_of(&sand, &card)?;
            }
            let lines = if when.meets_no_journal() {
                fs::write(sand.file(), garbage)?;
                vec![none_deleted(&sand.real("card.mp4"), 1), read]
            } else {
                vec![yields_to_exports(&sand.real("card.mp4"), 0), read]
            };
            set_mode(&config, 0o300)?;
            let meanwhile = match when {
                UnlistedDuring::ExportEnd => None,
                UnlistedDuring::Listing | UnlistedDuring::ListingWithNoJournal => {
                    let mut other = journal.pending();
                    let listed = other.list(&[sand.folders.container.join("other.mp4")]);
                    Some(listed.and_then(|()| other.clear()).map(|()| Recovery::default()))
                }
                UnlistedDuring::Start | UnlistedDuring::StartWithNoJournal => Some(Ok(journal.recover())),
            };
            set_mode(&config, 0o700)?;
            let due = match when {
                UnlistedDuring::Start => Some(vec![unlisted]),
                UnlistedDuring::StartWithNoJournal => Some(vec![unlisted, replaced_while_one_waits(&damaged)]),
                UnlistedDuring::ExportEnd | UnlistedDuring::Listing | UnlistedDuring::ListingWithNoJournal => None,
            };
            match (meanwhile, due) {
                (Some(Ok(recovery)), Some(due)) if recovery.journal != due => faults.push(format!("{when:?}: the start said {:?}", recovery.journal)),
                (Some(Err(error)), _) => faults.push(format!("{when:?}: {error:?}")),
                _ => {}
            }
            let record = serde_json::from_slice::<Value>(&fs::read(sand.file())?).ok().and_then(|now| now.get(TAKEN).cloned());
            if when.meets_no_journal() && record != Some(json!(EVERY)) {
                faults.push(format!("{when:?}: the journal that is no journal was not replaced by one recording that every entry yields: {record:?}"));
            }
            set_mode(&shelf, 0o600)?;
            let later = sand.start()?;
            let recovery = later.recovery();
            let frame_due: Option<&[u8]> = if when.meets_no_journal() { Some(b"half") } else { None };
            if fs::read(&card).ok().as_deref() != Some(FINISHED) || fs::read(&frame).ok().as_deref() != frame_due || recovery.journal != lines {
                faults.push(format!("{when:?}: once the folder lists again, the start answered {recovery:?}"));
            }
        }
        assert!(faults.is_empty(), "{faults:#?}");
        Ok(())
    }

    /// What set-aside entries yield to is a list of paths, as the folders are: anything else makes the
    /// journal no journal, which names no file to delete, and the start says why.
    #[test]
    fn a_journal_whose_record_of_what_entries_yield_to_is_no_list_of_paths_is_no_journal() -> Test {
        for (taken, why) in [
            (json!("card.mp4"), "has a `taken` that is not a list".to_string()),
            (json!([{ PATH: 1 }]), format!("lists a taken file that is not a `path`: {}", json!({ PATH: 1 }))),
        ] {
            let sand = Sandbox::new()?;
            let kept = sand.put("card.mp4", b"half");
            fs::write(sand.file(), json!({ OUTPUTS: [entry("card.mp4")], TAKEN: taken }).to_string())?;
            let engine = sand.start()?;
            let journal = fs::canonicalize(&sand.folders.config)?.join(JOURNAL_FILE);
            assert_eq!(
                engine.recovery().journal,
                [format!("the pending-export journal {} {why}; it names no file to delete, so it is replaced", journal.display())]
            );
            assert!(kept.exists(), "a file was deleted on the word of a damaged journal");
        }
        Ok(())
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
        run_alone("mkfifo", &[pipe.as_os_str()])?;
        let sand = Sandbox::new()?;
        std::os::unix::fs::symlink(&pipe, sand.file())?;
        let start = thread::spawn({
            let folders = sand.folders.clone();
            move || Engine::start_in(&folders, Scripted::idle(), GpuChoice::CpuOnly).map(|engine| engine.recovery().clone())
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
                    move || Engine::start_in(&folders, Scripted::idle(), GpuChoice::CpuOnly).map(|engine| engine.recovery().clone())
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
        let entries: Vec<Entry> = (0..17).map(|n| Entry { path: format!("{n}{long}"), kind: Kind::File(None) }).collect();
        let refused = journal.write(&entries, &Taken::default());
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

    /// An export that lists `output` and is then held, with the gate it opens once it has listed and
    /// the gate that lets it end: it writes `output` before it is held when `early`, and once it is let
    /// go otherwise.
    fn held_export(output: &Path, early: bool) -> (Script, Arc<Gate>, Arc<Gate>) {
        let (listed, release) = (Arc::new(Gate::default()), Arc::new(Gate::default()));
        let (opened, wait, written) = (listed.clone(), release.clone(), output.to_path_buf());
        let script: Script = Box::new(move |_, list_outputs| {
            list_outputs(std::slice::from_ref(&written))?;
            let write = || fs::write(&written, b"being written").map_err(|error| CraftError::io(error.to_string()));
            if early {
                write()?;
            }
            opened.open();
            assert!(wait.wait(), "the test never let the export end");
            if !early {
                write()?;
            }
            Ok(Value::Null)
        });
        (script, listed, release)
    }

    /// The export of `engine`, run on a thread of its own as an app makes its call with progress.
    fn exporting(engine: &Arc<Engine>) -> thread::JoinHandle<Result<Value>> {
        let engine = engine.clone();
        thread::spawn(move || engine.request_with_progress("export", Value::Null, |_, _, _| {}, &CancelToken::new()))
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
        let (script, listed, release) = held_export(&output, true);
        let first = Arc::new(sand.start_running(script)?);
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
        let export = exporting(&first);
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

    /// A second engine's start never removes a folder a running export of the process is about to
    /// write into, even one that is empty now and that a killed run listed as one it made.
    #[test]
    fn a_second_engines_start_leaves_the_folder_a_running_export_writes_into() -> Test {
        let sand = Sandbox::new()?;
        let output = sand.folders.container.join("busy/card.mp4");
        let (script, listed, release) = held_export(&output, false);
        let first = Arc::new(sand.start_running(script)?);
        // A killed run of another process made the folder, and the export's listing reads its entry
        // when it takes the journal.
        fs::create_dir(sand.folders.container.join("busy"))?;
        sand.write_journal_with(&[], &[entry("busy")]);
        let export = exporting(&first);
        assert!(listed.wait(), "the export never listed its output");

        let second = sand.start()?;
        assert!(sand.folders.container.join("busy").is_dir(), "the start removed the folder a running export writes into: {:?}", second.recovery());
        assert!(second.recovery().removed.is_empty(), "{:?}", second.recovery());
        release.open();
        assert_eq!(export.join().unwrap(), Ok(Value::Null));
        assert_eq!(fs::read(&output)?, b"being written");
        Ok(())
    }

    /// A cancelled export never deletes a file another export of the process is still writing, even
    /// one it wrote to itself: the file is the running export's, whose own end deals with it.
    #[test]
    fn a_cancelled_export_leaves_a_file_another_running_export_writes() -> Test {
        let sand = Sandbox::new()?;
        let output = sand.folders.container.join("card.mp4");
        let (script, listed, release) = held_export(&output, true);
        let first = Arc::new(sand.start_running(script)?);
        let export = exporting(&first);
        assert!(listed.wait(), "the export never listed its output");

        let cancel = CancelToken::new();
        let (stop, written) = (cancel.clone(), output.clone());
        let second = sand.start_running(Box::new(move |on_progress, list_outputs| {
            list_outputs(std::slice::from_ref(&written))?;
            fs::write(&written, b"written over by the cancelled export").unwrap();
            stop.cancel();
            assert!(!on_progress(1, 1, None));
            Ok(Value::Null)
        }))?;
        assert_eq!(second.request_with_progress("export", Value::Null, |_, _, _| {}, &cancel), Err(CraftError::Cancelled));
        assert!(output.is_file(), "a cancelled export deleted a file another export of the process is writing");
        release.open();
        assert_eq!(export.join().unwrap(), Ok(Value::Null));
        assert!(!sand.file().exists(), "an entry outlived both exports");
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

    /// A listed file the disk will not let go stays listed, so the next start tries again, and so does
    /// a folder the export made that holds it, which that start removes after it.
    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_deleted_stays_listed() -> Test {
        use std::os::unix::fs::PermissionsExt;
        let sand = Sandbox::new()?;
        let partial = sand.put("made/locked/card.mp4", b"half");
        sand.write_journal_with(&[entry("made/locked/card.mp4")], &[entry("made/locked"), entry("made")]);
        let locked = partial.parent().unwrap().to_path_buf();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500))?;

        let engine = sand.start()?;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700))?;
        if !partial.exists() {
            // A user the permissions do not hold back (root) deletes it anyway.
            return Ok(());
        }
        assert_eq!(engine.recovery().failed.len(), 1, "{:?}", engine.recovery());
        assert!(engine.recovery().untouched.is_empty() && engine.recovery().removed.is_empty(), "{:?}", engine.recovery());
        let journal: Value = serde_json::from_slice(&fs::read(sand.file())?)?;
        assert_eq!(journal[OUTPUTS], json!([entry("made/locked/card.mp4")]), "an entry that could not be deleted was dropped");
        assert_eq!(journal[FOLDERS], json!([entry("made/locked"), entry("made")]), "the folders that hold it were dropped");
        // The next start of the same process tries it again, from the journal it reads again.
        let again = sand.start()?;
        assert_eq!(again.recovery().removed, [sand.real("made/locked/card.mp4"), sand.real("made/locked"), sand.real("made")], "{:?}", again.recovery());
        assert!(!sand.file().exists() && !sand.folders.container.join("made").exists());
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
        let engine = sand.start_running(script)?;
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
