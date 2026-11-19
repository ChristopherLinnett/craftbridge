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
//! * **Each listing belongs to one export.** Every entry keeps the identity of the export that made
//!   it, unique among the exports of the process that wrote it, so an export's end takes apart exactly
//!   its own listings, never another export's, and an entry a build without identities wrote keeps
//!   none and belongs to no export running now. A path a running export of the process has listed
//!   belongs to that export alone until it ends, and a second listing of it is refused, as a path
//!   outside the container is: two exports writing one path at once leave no defined file there,
//!   and once two exports had listed one path and the one that finished cleared its entry, no rule
//!   could tell whose file the one at the path was, and the other's cancel, kill or failure could
//!   delete a file it never wrote, at the time or after a restart. An export may list its own path
//!   again, and two exports whose files need one new folder each keep that folder's entry apart, as
//!   each export makes the folder its own files need.
//! * **Two paths are one file when the volume holding them says so, never by their spelling.** Every
//!   comparison in the journal asks one question through [`PathId`]: a path that is there is the
//!   volume's own identity for it — its device and file number, with where its name lies beside them,
//!   which is what one that is not there yet is told by: each folder above it that is there, with the
//!   names below that folder folded under the volume's own rules for case and Unicode form, read from
//!   the volume by asking it and never assumed from the platform. Two paths name one file when they
//!   share the numbers or one of those dwellings, so a file a running export puts up between one
//!   operation's looks at it, or under a folder it makes between them, is still the file it was
//!   before its writer wrote it. That is no equivalence, so the journal gathers paths by each of
//!   those keys apart and never sorts them by one order. A volume
//!   whose rules cannot be read is treated as the one that loses no work, which compares as if it
//!   ignored both. One file spelt two ways is then refused a second export exactly as one path listed
//!   twice is, so a finished export's file survives another export's cancel, kill or failure however
//!   the second export spelt the path; and two files a volume keeps apart stay two exports.
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
//! releases the lock when a process ends, however it ends. A process lets go of it by unlocking it
//! before it closes its handle, never by the close alone: the lock belongs to the open file, and a
//! process the app starts from any thread keeps a copy of every handle the app has open until it
//! runs its program, so a close would leave the lock with that copy, and the app's next start or
//! listing would be refused as if another process ran exports with the journal. A start that finds
//! another process holding it deals with nothing and says so; a listing that does is refused, so
//! its export does not run; each later start and listing tries again. The lock is never held
//! longer, because iOS terminates a suspended app that holds a file lock outside its own data
//! container, as in an App Group container a host may share with an extension (Apple's developer
//! forums, thread 655225: "You must release file locks before coming eligible for suspension"), and
//! an app keeps a running export from suspension with a background task.
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
use crate::folders::{Confined, Confinement, Folders, resolved};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::btree_map::Entry as Slot;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File, TryLockError};
use std::io::{self, Read, Write};
use std::iter;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;
use unicase::UniCase;
use unicode_normalization::UnicodeNormalization;

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
/// Which export listed the entry, as [`NEXT_EXPORT`] numbers it; absent from an entry written before
/// identities.
const EXPORT: &str = "export";

/// What a temporary file of the journal's atomic write starts with, and the name a probe of the
/// volume's rules puts up beside it ([`probe_name`]), so a start can sweep what a killed write or
/// probe left behind and nothing else.
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

/// Numbers the exports of this process, so each one's listings can be told from every other's: the
/// next [`Journal::pending`] takes one, and every entry that export lists carries it. Exports of
/// other processes may hold the same numbers, which is harmless: entries read from another process's
/// journal are no export running here's, and are dealt with as a start deals with them.
static NEXT_EXPORT: AtomicU64 = AtomicU64::new(1);

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
    lock: Option<Hold>,
}

/// This process's hold on the lock on [`LOCK_FILE`], taken through the handle it keeps, and let go
/// of when it drops.
#[derive(Debug)]
struct Hold(File);

impl Drop for Hold {
    /// Unlock, then close. The lock belongs to the open file, which every copy of the handle shares,
    /// and a process the app starts while the hold lives keeps a copy until it runs its program, so
    /// the close alone would leave the lock held by that copy, and the app's next start or listing
    /// would be refused for it. An unlock lets go for every copy at once.
    fn drop(&mut self) {
        // A drop has nobody to answer, and an unlock that fails leaves only what the close leaves: a
        // lock that goes once the last copy of the handle closes.
        let _unlocked = self.0.unlock();
    }
}

impl Known {
    /// Every entry the journal holds.
    fn entries(&self) -> Vec<Entry> {
        self.held.iter().chain(&self.running).cloned().collect()
    }
}

// ——— Whether two paths name one file: the one definition every comparison in the journal uses ———

/// What the volume holding `found` says that file is, where the system numbers files at all: the
/// volume's and the file's own numbers, which one file carries under every name and spelling of it.
#[cfg(unix)]
fn identity_of(found: &fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((found.dev(), found.ino()))
}

/// This system gives no file numbers to tell a path by, so no path can be told by one: every
/// spelling of a path is then folded before it is compared, which is the reading that loses no work
/// ([`VolumeRules::default`]).
#[cfg(not(unix))]
fn identity_of(_found: &fs::Metadata) -> Option<(u64, u64)> {
    None
}

/// Which volume `found` sits on, where the system names volumes at all: [`VOLUMES`] keys each read
/// volume's rules by that number.
#[cfg(unix)]
fn volume_of(found: &fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(found.dev())
}

/// No volume to name, so none to read rules from or key them under: every volume is then one whose
/// rules could not be read, and keeps [`VolumeRules::default`].
#[cfg(not(unix))]
fn volume_of(_found: &fs::Metadata) -> Option<u64> {
    None
}

/// How the volume holding the journal compares the names under it, read from the volume itself by
/// [`VolumeRules::read`] and never assumed from the platform: a Mac's internal disk folds case and
/// Unicode form, an external volume formatted another way on the same Mac may fold neither, and a
/// Linux volume may keep even case apart, so only the volume can say what the volume does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VolumeRules {
    /// Whether two names differing only in case are kept and compared apart.
    case_sensitive: bool,
    /// Whether two names differing only in Unicode form are kept and compared apart.
    form_sensitive: bool,
}

impl Default for VolumeRules {
    /// The reading of a volume whose rules cannot be read: the one that loses no work. Reading a
    /// volume as folding more than it does can only refuse a listing two files could have run, since
    /// two files that are there are told apart by their own numbers whatever the fold; reading it as
    /// stricter than it folds lets two spellings of one file pass as two exports, which is how a
    /// cancel deletes a file another export finished. Lost work outranks a needless refusal, so an
    /// unread volume folds both.
    fn default() -> Self {
        Self { case_sensitive: false, form_sensitive: false }
    }
}

/// The rules read from each volume this process has asked, keyed by the volume's number: the asking
/// writes a probe file, so no volume is asked twice in one process.
static VOLUMES: Mutex<BTreeMap<u64, VolumeRules>> = Mutex::new(BTreeMap::new());

/// Names each probe of [`VolumeRules::ask`] apart from every other's, in this process and after it.
static PROBED: AtomicU64 = AtomicU64::new(1);

impl VolumeRules {
    /// The rules of the volume holding `folder` — the journal's own folder — with that volume's
    /// number, asked once per volume in a process and remembered; none when `folder` names no
    /// volume, whose names then fold by [`VolumeRules::default`]. The rules hold for that volume
    /// alone: the number goes with them, so they fold no name on another.
    ///
    /// The volume is asked by what it does with names, because no call could be read instead: POSIX
    /// `pathconf` names no rule for case, Apple's volumes carry a flag for that but none for form,
    /// and nothing anywhere answers Unicode form at all — which is exactly what a Mac's disks fold.
    /// So the volume is shown a name and asked what answers a spelling of it, and it answers both
    /// rules at once in its own behaviour rather than a platform's reputation.
    fn read(folder: &Path) -> Option<(u64, Self)> {
        let volume = fs::metadata(folder).ok().as_ref().and_then(volume_of)?;
        let rules = *lock(&VOLUMES).entry(volume).or_insert_with(|| Self::ask(folder));
        Some((volume, rules))
    }

    /// Ask the volume holding `folder` its rules, by a probe put up in `folder` and taken back
    /// ([`names_apart`]): a rule the volume could not be asked about folds, the reading that loses no
    /// work.
    fn ask(folder: &Path) -> Self {
        let probe = probe_name();
        Self {
            case_sensitive: names_apart(folder, &probe, &probe.to_uppercase()).unwrap_or(false),
            form_sensitive: names_apart(folder, &format!("{probe}\u{fc}"), &format!("{probe}u\u{308}")).unwrap_or(false),
        }
    }
}

/// The name a probe of [`VolumeRules::ask`] puts up, which no other probe of this process or of any
/// other running process puts up. It starts with [`TEMP_PREFIX`], so a start's sweep removes what a
/// process killed mid-probe left in the journal's folder.
fn probe_name() -> String {
    format!("{TEMP_PREFIX}volume-rules-{:x}-{:x}", std::process::id(), PROBED.fetch_add(1, Ordering::Relaxed))
}

/// Ask the volume holding `folder` whether it keeps the name `name` and the name `spelling` — the
/// same name spelt another way, differing in the one rule being asked about — apart: `Some(true)`
/// for two files, `Some(false)` for one file answering both names, `None` when the volume could not
/// be asked, whose reader folds instead ([`VolumeRules::default`]), the answer that loses no work.
/// The spelling is looked at before the name, and that order is what makes two names a proof: a
/// sweep of a process that runs without the journal's lock may take the probe away at any moment,
/// and nothing but this ask puts its name up, so a name still there after the spelling found nothing
/// was there when the spelling was looked at, while the other order would read a probe swept between
/// the two looks as two names on a volume that folds them.
fn names_apart(folder: &Path, name: &str, spelling: &str) -> Option<bool> {
    let put_up = folder.join(name);
    let spelled = folder.join(spelling);
    match fs::OpenOptions::new().write(true).create_new(true).open(&put_up) {
        // The probe is opened and dropped at once: its presence under a name is the whole answer,
        // and nothing of the ask may hold a file in the journal's folder afterwards.
        Ok(probe) => drop(probe),
        // A file already answers the name: the same question asked twice at once has one answer
        // either way, so asking against what is already there is enough.
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        // The folder takes no probe: the volume cannot be asked.
        Err(_) => return None,
    }
    let answer = match (fs::symlink_metadata(&spelled), fs::symlink_metadata(&put_up)) {
        // The other name answers too: one file, if the numbers say it is the same file.
        (Ok(other), Ok(one)) => (identity_of(&one) == identity_of(&other)).then_some(false),
        // Nothing answers the other name while the name itself is there to say the volume kept the
        // two apart: two names.
        (Err(error), Ok(_)) if error.kind() == io::ErrorKind::NotFound => Some(true),
        _ => None,
    };
    // Take the probe back however it is spelt: on a folding volume both names are the one file,
    // and the name that finds nothing has taken it back already.
    let _gone = fs::remove_file(&spelled);
    let _gone = fs::remove_file(&put_up);
    answer
}

/// No folder to stand at the head of a folded path: the container itself could not be looked at.
const NO_FOLDER: (u64, u64) = (0, 0);

/// Where a name lies, as one folder above it that is there tells it: that folder's numbers, and the
/// names from it down to the name, each folded as the volume holding it folds names. A folder is
/// looked at through its links, as the writer's open and a start's removal reach what lies in it, so
/// two paths that reach one folder by different ways lie in one dwelling of it. A path is told from
/// every folder above it that is there, the container first, and a dwelling stays one across a file
/// or a folder below it being put up, which numbers cannot: they exist only once a file is there.
#[derive(Clone, Debug)]
struct Dwelling {
    /// The folder's numbers ([`NO_FOLDER`] for a container that cannot be looked at).
    folder: (u64, u64),
    /// The names from the container down to the folder the path's own name lies in, of which those
    /// below `folder` begin at `below`: shared by every dwelling of every path in that folder.
    folders: Rc<[String]>,
    below: usize,
    /// The path's own name.
    name: Option<Rc<str>>,
}

impl Dwelling {
    /// What a dwelling is: the folder, and the names below it with the path's own after them.
    fn parts(&self) -> ((u64, u64), &[String], Option<&str>) {
        (self.folder, self.folders.get(self.below..).unwrap_or_default(), self.name.as_deref())
    }
}

impl PartialEq for Dwelling {
    fn eq(&self, other: &Self) -> bool {
        self.parts() == other.parts()
    }
}

impl Eq for Dwelling {}

impl Ord for Dwelling {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.parts().cmp(&other.parts())
    }
}

impl PartialOrd for Dwelling {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Where a path below the container lies, told the way the volume holding it tells one file from
/// another: the journal's one answer to whether two paths name one file, which no comparison in it
/// asks any other way. A path that is there carries the volume's own numbers for it, and every path
/// carries its dwellings, one from each folder above it that is there: an export lists the file it is
/// about to write before it writes it, and the listing holds or refuses it by what the volume would
/// make of the spelling, never by the spelling. Two paths name one file when they share a key — the
/// numbers, when both looks found the file, or a dwelling, whenever one of them came before a writer
/// put the file, or a folder of it, up — so one file is one file however its paths were looked at,
/// before or after it came to be. That relation is no equivalence (a file put at a path in place of
/// another shares the path's dwellings with the file before it, a second name of the first file in
/// another folder shares its numbers, and those two share nothing), so no order can stand for it and
/// none is given: the journal gathers tellings in [`Tellings`], which looks one up by every key.
#[derive(Clone, Debug)]
struct PathId {
    /// The volume's and the file's numbers, when a file is there.
    numbers: Option<(u64, u64)>,
    /// The names from the container down to the folder the path lies in, folded.
    folders: Rc<[String]>,
    /// Each folder of the path that is there, from the container down: how many of `folders` lie
    /// above it, and its numbers, none for a container that cannot be looked at.
    there: Rc<[(usize, Option<(u64, u64)>)]>,
    /// The path's own name, folded as its folder's volume folds names.
    name: Option<Rc<str>>,
}

impl PathId {
    /// Where this path lies, as each folder above it that is there tells it.
    fn dwellings(&self) -> impl Iterator<Item = Dwelling> + '_ {
        self.there.iter().map(|&(below, folder)| Dwelling {
            folder: folder.unwrap_or(NO_FOLDER),
            folders: Rc::clone(&self.folders),
            below,
            name: self.name.clone(),
        })
    }
}

/// Tellings gathered so that another finds every one it names one file with ([`PathId`]): each is kept
/// under its numbers, when its file is there, and under each of its dwellings, each key in an order
/// of its own, and a telling is looked up under every key it has. One map ordered by whole tellings
/// would lose them: a telling sorts by its first parts, and a lookup that sorts past the one it shares
/// a key with never reaches it.
struct Tellings<V> {
    by_numbers: BTreeMap<(u64, u64), V>,
    by_dwelling: BTreeMap<Dwelling, V>,
}

impl<V> Default for Tellings<V> {
    fn default() -> Self {
        Self { by_numbers: BTreeMap::new(), by_dwelling: BTreeMap::new() }
    }
}

impl<V: Clone> Tellings<V> {
    /// Gather `id` with `value`, which `join` adds to what each key of it holds already.
    fn add(&mut self, id: &PathId, value: V, join: impl Fn(&mut V, &V)) {
        if let Some(numbers) = id.numbers {
            gather(self.by_numbers.entry(numbers), &value, &join);
        }
        for dwelling in id.dwellings() {
            gather(self.by_dwelling.entry(dwelling), &value, &join);
        }
    }

    /// What the tellings that name one file with `id` were gathered with, under each key it shares
    /// with them.
    fn of<'a>(&'a self, id: &'a PathId) -> impl Iterator<Item = &'a V> + 'a {
        let numbers = id.numbers.and_then(|numbers| self.by_numbers.get(&numbers));
        numbers.into_iter().chain(id.dwellings().filter_map(|dwelling| self.by_dwelling.get(&dwelling)))
    }

    /// Whether a telling gathered names one file with `id`.
    fn holds(&self, id: &PathId) -> bool {
        self.of(id).next().is_some()
    }

    /// Whether nothing is gathered.
    fn is_empty(&self) -> bool {
        self.by_dwelling.is_empty() && self.by_numbers.is_empty()
    }
}

/// Put `value` under the key `slot` stands for, or `join` it to what is there.
fn gather<K: Ord, V: Clone>(slot: Slot<'_, K, V>, value: &V, join: &impl Fn(&mut V, &V)) {
    match slot {
        Slot::Vacant(free) => {
            free.insert(value.clone());
        }
        Slot::Occupied(mut held) => join(held.get_mut(), value),
    }
}

/// The identities of the paths below one container, gathered for one operation — one listing, one
/// start, one export's end — and kept for it: telling a missing path asks what the disk says of each
/// name of it from the container down, and a listing of a long sequence would ask it of every path
/// once per rule that asks, so each path gets one answer per operation instead.
///
/// The answers are kept *per folder*, not per path, because what the disk says of a folder is one
/// answer however many paths lie in it: one export of thousands of frames asks the same of the
/// thousandth as of the first, and an export whose every file is asked for at length spends so long
/// in its start that no cancel can reach it. So a folder is looked at once — its own numbers, and,
/// when the disk will list it, the names it holds — and a name a listed folder does not hold needs no
/// look of its own: it is nowhere for the operation. Every operation that tells paths holds the
/// journal's lock throughout, which no export of the process lists or ends without, so what an export
/// put up and ended with is in the listing; what a running export's writer puts up after it, taking
/// no lock, is claimed by that export's own entries.
struct Paths {
    root: PathBuf,
    /// The container's own numbers, which head every folded path below it; none for a container
    /// that cannot be looked at.
    anchor: Option<(u64, u64)>,
    /// The volume whose rules were read, with those rules ([`VolumeRules::read`]); the names on any
    /// other volume fold by [`VolumeRules::default`].
    read: Option<(u64, VolumeRules)>,
    asked: RefCell<BTreeMap<String, Told>>,
    /// Each folder of or below the container this operation has looked at, by the path it was asked
    /// under ([`Paths::folder`]). Shared, never copied, with every path told from it: a folder's
    /// listing holds a name for every file in it, so a copy for each path told would cost each file
    /// of a sequence the whole sequence again.
    folders: RefCell<BTreeMap<PathBuf, Rc<Folder>>>,
    /// Where each folder of them lands, so the files beside it are confined by the same look and not
    /// by one of their own ([`Confined`]).
    confined: Confined,
}

/// One folder of or below the container, told once for the operation.
struct Folder {
    /// The names from the container down to it, each folded as the volume of the folder above it
    /// folds names, or, below a folder that is not there, as the deepest that is folds them.
    names: Rc<[String]>,
    /// Each folder from the container down to it that is there: how many of `names` lie above it,
    /// and its numbers, none for a container that cannot be looked at, whose names then fold by
    /// [`VolumeRules::default`], the reading that loses no work. The last is the deepest that is
    /// there, this folder itself when it is.
    there: Rc<[(usize, Option<(u64, u64)>)]>,
    /// The names it holds, folded as its volume folds them, when it is there and its listing tells
    /// every name from every other; none when it is not, or when the listing says nothing of them,
    /// which leaves each name below to be looked at on its own.
    held: Option<BTreeSet<String>>,
}

impl Folder {
    /// The deepest folder of it that is there: its depth below the container, and its numbers.
    fn deepest(&self) -> (usize, Option<(u64, u64)>) {
        self.there.last().copied().unwrap_or((0, None))
    }

    /// The volume whose rules fold the names in it: the deepest folder's of it that is there.
    fn device(&self) -> Option<u64> {
        self.deepest().1.map(|(device, _)| device)
    }

    /// Whether the folder itself is there.
    fn is_there(&self) -> bool {
        self.deepest().0 == self.names.len()
    }
}

/// What one path of the container was told: where it lies, and whether nothing can be at it.
#[derive(Clone)]
struct Told {
    id: PathId,
    absent: bool,
}

impl Paths {
    /// The identities below `root`, with the rules read from `folder`, the journal's own folder.
    fn new(root: &Path, folder: &Path) -> Self {
        Self::with(root, VolumeRules::read(folder))
    }

    /// The identities below `root`, folding the names on the volume `read` names by the rules it
    /// carries. Those rules are never taken for another volume's, however near: the container may sit
    /// on another volume than the journal's folder, and a volume may be mounted under the container,
    /// so a folder of a path on any other volume, or below a container that cannot be looked at at
    /// all, folds by [`VolumeRules::default`], the reading that loses no work.
    fn with(root: &Path, read: Option<(u64, VolumeRules)>) -> Self {
        let anchor = fs::symlink_metadata(root).ok().as_ref().and_then(identity_of);
        Self {
            root: root.to_path_buf(),
            anchor,
            read,
            asked: RefCell::new(BTreeMap::new()),
            folders: RefCell::new(BTreeMap::new()),
            confined: Confined::default(),
        }
    }

    /// Where `output`, a file an export is about to write, lands below the container, resolved name
    /// and all, as a link at its own name must be ([`Confined::written`]).
    fn written(&self, output: &Path) -> Result<PathBuf, CraftError> {
        self.confined.written(&self.root, output)
    }

    /// Where the file `output` lands below the container when no link stands at its own name, its
    /// folder looked at once for every file beside it, without the disk being asked of the name at
    /// all ([`Confined::landing`]).
    fn landing(&self, output: &Path) -> Result<Option<PathBuf>, CraftError> {
        self.confined.landing(&self.root, output)
    }

    /// Where the entry `entry` lies, to be dealt with, each folder of it looked at once for every
    /// entry beside it ([`Confined::entry`]).
    fn target(&self, entry: &Path) -> Result<PathBuf, CraftError> {
        self.confined.entry(&self.root, entry)
    }

    /// Whether nothing can be at the entry path `path`, told by the folder it lies in ([`Paths::tell`]).
    fn absent(&self, path: &str) -> bool {
        self.tell(path).absent
    }

    /// The folder `at`, at or below the container, looked at once however many paths lie in it: every
    /// folder above it that nothing has looked at yet is looked at on the way, from the container down,
    /// because a folder that is not there is folded under the deepest folder of it that is, exactly
    /// as a name below it is. `None` when `at` is below nothing the container can be walked down to,
    /// which leaves each path of it to the disk itself.
    fn folder(&self, at: &Path) -> Option<Rc<Folder>> {
        let names: Vec<&OsStr> = at.strip_prefix(&self.root).ok()?.iter().collect();
        let mut known = self.folders.borrow_mut();
        // The container stands where its own numbers do, read as itself, so a container reached
        // through a link is folded under the link's numbers, as every path below it is told. It is
        // listed once for the operation, since every folder below it is reached through it and it
        // holds the names of everything beside them.
        let mut current = match known.get(&self.root) {
            Some(container) => Rc::clone(container),
            None => {
                let container = Rc::new(self.container(self.held_of(self.root.as_path(), self.anchor)));
                known.insert(self.root.clone(), Rc::clone(&container));
                container
            }
        };
        let mut here = self.root.clone();
        for name in names {
            here.push(name);
            if let Some(hit) = known.get(&here) {
                current = Rc::clone(hit);
                continue;
            }
            // A folder is looked at through a link, as the writer's open and a start's removal reach
            // what lies in it; one the disk shows but will not list holds its names to themselves.
            // Its name is kept whether it is there or not, so a path below it is told from every
            // folder above it that is there, and from the same ones once it is made.
            let names: Rc<[String]> = current.names.iter().cloned().chain(iter::once(self.fold(name, current.device()))).collect();
            current = Rc::new(match fs::metadata(&here).ok().as_ref().and_then(identity_of) {
                Some(numbers) => Folder {
                    there: current.there.iter().copied().chain(iter::once((names.len(), Some(numbers)))).collect(),
                    names,
                    held: self.held_of(&here, Some(numbers)),
                },
                None => Folder { names, there: Rc::clone(&current.there), held: None },
            });
            known.insert(here.clone(), Rc::clone(&current));
        }
        Some(current)
    }

    /// The container as a folder: no names above it, and its own numbers, holding `held`.
    fn container(&self, held: Option<BTreeSet<String>>) -> Folder {
        Folder { names: Rc::from([]), there: Rc::from([(0, self.anchor)]), held }
    }

    /// The names of the folder whose numbers are `numbers`, folded as that volume folds them, or none
    /// when there is no folder to list, none to be read, or two names that fold alike: a listing that
    /// cannot tell one name from another says nothing of any of them.
    fn held_of(&self, at: &Path, numbers: Option<(u64, u64)>) -> Option<BTreeSet<String>> {
        let (device, _) = numbers?;
        let mut held = BTreeSet::new();
        for entry in fs::read_dir(at).ok()? {
            if !held.insert(self.fold(&entry.ok()?.file_name(), Some(device))) {
                return None;
            }
        }
        Some(held)
    }

    /// Where the path `path` — an entry's, below the container — lies, told by the volume: [`PathId`].
    fn id(&self, path: &str) -> PathId {
        self.tell(path).id
    }

    /// What the volume says of the entry path `path`: where it lies, and whether nothing can be at it.
    /// One answer per path per operation, however many rules ask it, because each answer is told from
    /// the folder the path lies in, and the folder has been looked at once for the operation
    /// ([`Paths::folder`]).
    fn tell(&self, path: &str) -> Told {
        if let Some(told) = self.asked.borrow().get(path) {
            return told.clone();
        }
        let (parent, name) = (Path::new(path).parent().unwrap_or(Path::new("")), Path::new(path).file_name());
        let folder = self.folder(&self.root.join(parent)).unwrap_or_else(|| Rc::new(self.container(None)));
        // The name as its volume tells it, which is what both questions below read. A name a folder
        // the disk listed holds nothing of is not there, and nothing lies below a folder that is not
        // there at all, so the disk is not asked of the name. Every other name is asked of itself —
        // the folder was reached through its links, as the writer's open and a start's removal reach
        // what lies in it, while the path's own name is looked at as itself, as a start deals with a
        // link there.
        let folded = name.map(|name| self.fold(name, folder.device()));
        let absent = !folder.is_there()
            || match (&folder.held, &folded) {
                (Some(held), Some(folded)) => !held.contains(folded),
                _ => false,
            };
        let found = match absent {
            true => None,
            false => fs::symlink_metadata(self.root.join(path)).ok().as_ref().and_then(identity_of),
        };
        // The name's dwellings are told whichever way the disk answered, and a file there carries
        // them beside its numbers: a path of a file looked at before its writer put it, or a folder
        // of it, up, and one looked at after, are then told one dwelling, and one file, whoever
        // compares them.
        let id = PathId { numbers: found, folders: Rc::clone(&folder.names), there: Rc::clone(&folder.there), name: folded.map(Rc::from) };
        let told = Told { id, absent };
        self.asked.borrow_mut().insert(path.to_string(), told.clone());
        told
    }

    /// A name as the volume of the number `device` sees it when it asks whether two names are one:
    /// folded as that volume's own answer said it folds names ([`VolumeRules::read`]) — for case by
    /// Unicode's full case folding, which holds `straße` and `STRASSE`, `ſ` and `s`, as one name as a
    /// Mac's disk does, and for form to the decomposed form — and folded for both where the folder
    /// is on no volume whose rules were read. Full folding is the fold a Mac's disk makes, and a
    /// volume that folds case by a simpler table folds no more than it does, so the most that table
    /// can cost is a refusal, never a file. Case and form feed each other — a folded case can expose
    /// a form to fold, a folding can expose a case to fold — so folding runs on until it stops
    /// changing, bounded so no name costs more than a fixed few rounds however it is spelt.
    fn fold(&self, name: &OsStr, device: Option<u64>) -> String {
        let VolumeRules { case_sensitive, form_sensitive } = match (self.read, device) {
            (Some((volume, rules)), Some(device)) if volume == device => rules,
            _ => VolumeRules::default(),
        };
        let mut folded = name.to_string_lossy().into_owned();
        for _ in 0..4 {
            let mut next = if case_sensitive { folded.clone() } else { UniCase::new(folded.as_str()).to_folded_case() };
            if !form_sensitive {
                next = next.nfd().collect();
            }
            if next == folded {
                break;
            }
            folded = next;
        }
        folded
    }
}

/// What some entries claim, told as the volumes holding them tell files ([`PathId`]): the file or
/// folder each names, and every folder each lies in. A start and a cancel ask of every stale entry
/// whether a running export claims it, and of every folder whether an entry left listed lies in it,
/// so what those entries claim is gathered once, and each stale entry asks a lookup rather than
/// walking them all.
#[derive(Default)]
struct Claims {
    /// What each entry names.
    named: Tellings<()>,
    /// Each folder an entry lies in, and what the entry names.
    within: Tellings<()>,
}

impl Claims {
    /// What `entries` claim.
    fn of(paths: &Paths, entries: &[Entry]) -> Self {
        let mut claims = Self::default();
        for listed in entries {
            claims.add(paths, listed);
        }
        claims
    }

    /// Claim what `listed` names, and every folder it lies in, as well.
    fn add(&mut self, paths: &Paths, listed: &Entry) {
        self.named.add(&paths.id(&listed.path), (), |_, _| {});
        for folder in 1..=depth(&listed.path) {
            self.within.add(&paths.id(&under(&listed.path, folder)), (), |_, _| {});
        }
    }

    /// Whether `entry` claims one of the entries gathered: a file entry claims the one that names
    /// its file, and a folder entry each one that names the folder or lies in it.
    fn by(&self, paths: &Paths, entry: &Entry) -> bool {
        let id = paths.id(&entry.path);
        match entry.kind {
            Kind::File(_) => self.named.holds(&id),
            Kind::Folder => self.within.holds(&id),
        }
    }
}

/// `path` (below the container) cut back to its first `depth` names: the folder as deep as `depth`
/// that the path lies in — the path to tell when a folder entry is asked whether `listed` lies in
/// it. Cut the way a folder entry's own path reads, so the two tellings of one folder come out one.
fn under(path: &str, depth: usize) -> String {
    Path::new(path)
        .components()
        .take(depth)
        .map(|part| {
            let name: &OsStr = part.as_ref();
            name.to_string_lossy().into_owned()
        })
        .collect::<Vec<_>>()
        .join("/")
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

    /// Whether a set-aside entry of the file at a path yields: whether that path names one file with
    /// a recorded one, as the volumes holding them decide ([`PathId`]) — never by the spelling the
    /// record happens to carry. The record is told once, so each entry a start hands over asks one
    /// lookup however many files the record names.
    fn yields<'p>(&self, paths: &'p Paths) -> impl Fn(&str) -> bool + use<'p> {
        let mut files = Tellings::default();
        for file in &self.files {
            files.add(&paths.id(file), (), |_, _| {});
        }
        let every = self.every;
        move |path| every || files.holds(&paths.id(path))
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
    /// The identity every entry of this listing carries ([`Entry::export`]), so its end clears exactly
    /// its own entries and no other export can list what it holds.
    export: u64,
}

/// One listed path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    /// Where it is below the container, as the export's writer opens it.
    path: String,
    /// What the export's writer puts there.
    kind: Kind,
    /// Which export listed it, as [`NEXT_EXPORT`] numbers it: an export's end takes apart exactly
    /// the entries of its own listing, and a running entry of one export is no other's to take.
    /// None for an entry a journal written without identities left, and for the entries a start
    /// invents for a set-aside journal's files: neither is any export running now's.
    export: Option<u64>,
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
    Taken(Hold),
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
                Locked::Taken(hold) => (Known { lock: Some(hold), ..Known::default() }, true),
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
        let paths = self.paths();
        let read_now = if waits {
            set_aside.wait(&mut recovery)
        } else {
            stale.extend(still_set_aside(set_aside.contents, &mut taken, &self.root, &paths, &mut recovery));
            set_aside.read
        };
        stale.extend(found.entries);
        let held = self.deal_with_all(&paths, stale, &running, &mut recovery);
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
    /// Paths are told by `paths`, the operation's own ([`Paths`]).
    fn deal_with_all(&self, paths: &Paths, stale: Vec<Entry>, running: &[Entry], recovery: &mut Recovery) -> Vec<Entry> {
        let (files, mut folders): (Vec<Entry>, Vec<Entry>) = stale.into_iter().partition(|entry| matches!(entry.kind, Kind::File(_)));
        folders.sort_by(|a, b| depth(&b.path).cmp(&depth(&a.path)).then_with(|| b.path.cmp(&a.path)));
        // A folder one file is, however many exports made an entry for a spelling of its path, each
        // of their files needing it: the entries differ only in the export that made each, and one
        // of them stands for the folder. Spellings one volume folds can sort apart from each other,
        // so what stands is the first entry of each folder seen, not the one that sorted beside it.
        let mut one_folder = Tellings::default();
        folders.retain(|entry| {
            let id = paths.id(&entry.path);
            let first = !one_folder.holds(&id);
            if first {
                one_folder.add(&id, (), |_, _| {});
            }
            first
        });
        let running = Claims::of(paths, running);
        // What stays listed, and what it claims, which grows with it.
        let (mut held, mut kept) = (Vec::new(), Claims::default());
        for entry in files.into_iter().chain(folders) {
            if running.by(paths, &entry) {
                continue;
            }
            // A folder that holds a file this start could not delete stays listed with it, for the
            // start that deletes the file.
            if entry.kind == Kind::Folder && kept.by(paths, &entry) {
                kept.add(paths, &entry);
                held.push(entry);
                continue;
            }
            let target = match paths.target(Path::new(&entry.path)) {
                Ok(target) => target,
                Err(refusal) => {
                    recovery.refused.push((PathBuf::from(entry.path), refusal.to_string()));
                    continue;
                }
            };
            // The folder the entry lies in has been told for this operation, and a name its listing
            // holds nothing of is nowhere and left alone: a file put up after the listing, by a writer
            // that takes no journal lock, is never deleted for an entry, the side that loses no work.
            let absent = paths.absent(&entry.path);
            let settled = match entry.kind {
                Kind::File(found) => settle(&target, found, absent),
                Kind::Folder => settle_folder(&target, absent),
            };
            match settled {
                Ok(Settled::Absent) => {}
                Ok(Settled::Untouched) => recovery.untouched.push(target),
                Ok(Settled::Removed) => recovery.removed.push(target),
                Err(Refusal::NotAFile(what)) => recovery.refused.push((target, format!("it is {what}, not a file an export writes"))),
                Err(Refusal::NotAFolder(what)) => recovery.refused.push((target, format!("it is {what}, not a folder a writer makes"))),
                Err(Refusal::Disk(reason)) => {
                    recovery.failed.push((target, reason));
                    kept.add(paths, &entry);
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
                    Locked::Taken(hold) => Some(hold),
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
            Ok(()) => Locked::Taken(Hold(file)),
            Err(TryLockError::WouldBlock) => {
                Locked::Elsewhere(format!("another process holds the lock on {}, so it may be running exports of its own", path.display()))
            }
            Err(TryLockError::Error(error)) => unsupported(&error),
        }
    }

    /// An export's entry, empty until the export lists the files it is about to write. It is one
    /// export's, numbered from [`NEXT_EXPORT`]: every entry it lists carries its number.
    pub(crate) fn pending(&self) -> Pending {
        Pending { journal: self.clone(), listed: Vec::new(), looked: BTreeSet::new(), export: NEXT_EXPORT.fetch_add(1, Ordering::Relaxed) }
    }

    /// The entry of `output`, a file the export numbered `export` is about to write: where its
    /// writer's open lands, below the container, and the file that is there now, if any. Refused when
    /// that is outside the container, or is anything but a file or nothing: a start would refuse to
    /// touch it. `paths` is the operation's own, so the folders of `output` are looked at once for
    /// every file beside them, and a file none of them holds a name of is not looked at at all.
    fn entry_of(&self, paths: &Paths, output: &Path, export: u64) -> Result<Entry> {
        // Where the name lands is told by its folder, which is looked at once for the operation however
        // many files lie in it: a name lands where its folder lands with the name below it, which is
        // where its writer puts it once it has made the folder, unless a link stands at the name, and
        // only the name can say where a link points. So a name a listed folder holds nothing of is
        // nowhere and is not looked at; one that may be there is looked at once, as itself, which both
        // tells whether a link stands there and gives the file's stamp; and only a link, or a name whose
        // folder cannot answer for it, is resolved on its own.
        let landing = paths.landing(output)?;
        let (below, found) = match landing {
            Some(below) if below.to_str().is_some_and(|spelled| paths.absent(spelled)) => (below, None),
            Some(below) => {
                let looked = fs::symlink_metadata(self.root.join(&below));
                match looked {
                    Ok(found) if found.file_type().is_symlink() => self.resolved_entry(paths, output)?,
                    looked => {
                        let found = self.stamp_at(&below, looked)?;
                        (below, found)
                    }
                }
            }
            None => self.resolved_entry(paths, output)?,
        };
        let path = below.to_str().ok_or_else(|| {
            CraftError::io(format!("the output file {} cannot be listed in the pending-export journal: its path is not UTF-8", output.display()))
        })?;
        Ok(Entry { path: path.to_string(), kind: Kind::File(found), export: Some(export) })
    }

    /// Where `output` lands below the container with its own name resolved as well, a link at it
    /// followed as its writer's open follows it, and the file that is there now, if any.
    fn resolved_entry(&self, paths: &Paths, output: &Path) -> Result<(PathBuf, Option<Stamp>)> {
        let below = paths.written(output)?;
        let found = self.stamp_at(&below, fs::symlink_metadata(self.root.join(&below)))?;
        Ok((below, found))
    }

    /// The stamp of the file `looked` found at `below`, a path inside the container looked at without
    /// following its name: none when nothing is there, and refused when what is there is no file.
    fn stamp_at(&self, below: &Path, looked: io::Result<fs::Metadata>) -> Result<Option<Stamp>> {
        let target = self.root.join(below);
        match looked {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(CraftError::io(format!("cannot look at the output file {}: {error}", target.display()))),
            Ok(found) if found.is_file() => Ok(Some(Stamp::of(&found))),
            // A link the resolution could not follow points at nothing yet: the writer would create
            // its target wherever it points.
            Ok(_) => Err(CraftError::io(format!("the output file {} is a folder or a link, not a file an export writes", target.display()))),
        }
    }

    /// The folders of the path `file` (an entry's, below the container) that are not there now, the
    /// deepest first: the ones its writer makes, each entered for the export numbered `export`. A
    /// folder this export has looked at already (in `looked`, or in `seen` during this listing) is not
    /// looked at again, and nor is any folder above it, which was looked at with it; one that is there
    /// holds only folders that are.
    fn made_folders(&self, file: &str, looked: &BTreeSet<PathBuf>, seen: &mut BTreeSet<PathBuf>, export: u64) -> Vec<Entry> {
        let mut made = Vec::new();
        for folder in Path::new(file).ancestors().skip(1).take_while(|folder| !folder.as_os_str().is_empty()) {
            if looked.contains(folder) || !seen.insert(folder.to_path_buf()) {
                break;
            }
            match fs::symlink_metadata(self.root.join(folder)) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    made.push(Entry { path: folder.to_string_lossy().into_owned(), kind: Kind::Folder, export: Some(export) });
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

    /// The journal's answer to whether two paths below the container name one file, asked of the
    /// volume its own folder lies on — the volume an app's entries live on too ([`PathId`]).
    fn paths(&self) -> Paths {
        Paths::new(&self.root, self.folder().unwrap_or(self.root.as_path()))
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
    /// export must not write them, when one lies outside the container, or is a path a running export
    /// of this process has listed and not ended, or the journal cannot be written: an export that
    /// could be killed unlisted would leave partial files nothing deletes. The refused path holds
    /// whatever entry of the listing ran into it, a path the second export derives as much as one it
    /// was given: an exporter that writes its log beside the output, named from the output's stem,
    /// gives two formats of one name one shared log, and the second export is refused there, for two
    /// writers on one log at once leave it no more defined a file than two writers on one output. The
    /// app waits for the export that holds the log, or writes its outputs under names whose logs
    /// differ; a refused listing wrote nothing, so waiting for the holder costs nothing.
    pub(crate) fn list(&mut self, outputs: &[PathBuf]) -> Result<()> {
        if outputs.is_empty() {
            return Ok(());
        }
        // Every look this listing takes at the disk is taken under the journal's lock, which no export
        // of this process can end or list without, so a file another export put up and finished before
        // the record is seen as there, and kept by this export's cancel, kill or failure. Looked at
        // before the lock, it could be nothing yet, and that export could finish it and end, its entry
        // gone, before this one is recorded: nothing would refuse this listing, and its cancel would
        // delete the finished file as its own. A folder's listing answers for every name in it for the
        // whole operation, so a look before the lock would hold that window open for every file of a
        // sequence at once.
        let mut known = lock(&KNOWN);
        let state = self.journal.hold(&mut known)?;
        let listed = self.list_held(state, &self.journal.paths(), outputs);
        // A process with no export running holds no lock.
        if state.running.is_empty() {
            known.remove(&self.journal.file);
        }
        listed
    }

    /// [`Pending::list`] with the journal held, as `state`, its paths told by `paths`, the listing's own.
    fn list_held(&mut self, state: &mut Known, paths: &Paths, outputs: &[PathBuf]) -> Result<()> {
        // A path a running export of this process has listed belongs to that export alone until it
        // ends. Two exports writing one path at once leave no defined file there, and once two exports
        // had listed one path and the one that finished cleared its entry, nothing in the journal could
        // tell whose file the one at the path was: the other's cancel, kill or failure could then
        // delete a file it never wrote, at the time or after a restart. So the second listing is
        // refused here, before any of it is written. An export may list its own path again, and a
        // folder two exports' files each need is listed by both; refused is only a path this listing
        // would put a file, or a folder of its own making, where a running export of another listing
        // has one of its own — one file told as the volume holding them decides ([`PathId`]), so a
        // second export of one file spelt another way is refused exactly as one spelt the same way
        // is. What the others hold is gathered once, each file with whether one of them puts a file
        // there, so a listing of a long sequence looks each of its paths up instead of walking every
        // running entry for each.
        //
        // A running export's writer puts files up as it renders, taking no journal lock, and this
        // call cannot wait the creation out: it holds this journal's lock from its first look to its
        // record while the write lands between the looks. Each listing records its files at their
        // landings, their folders resolved, so two exports' paths of one file in a folder that is
        // there are one spelling, told once, wherever the volume's resolution answers one spelling
        // for each folder. Where it does not — a volume that folds names its resolution keeps as
        // given — the identity waits the creation out: a file carries where its name lies, as each
        // folder above it that is there tells it, beside its numbers, and a path looked at before the
        // file, or a folder of it, was put up shares a dwelling with one looked at after, so a second
        // export of a file that appears between the looks is refused like any other ([`PathId`]).
        // What stays open is a path through a link that is itself made between the looks: before, the
        // link is nowhere and its name lies under the container as spelt; after, it is the folder it
        // reaches, and no folder above both tells the two alike. What is put up after a folder was
        // listed is none the less never deleted for a stale entry, whoever wrote it: the start and
        // the cancel weigh deletions only against the answer their listing gave them, and a name
        // absent from it is nowhere ([`settle`]).
        let mut others = Tellings::default();
        for listed in state.running.iter().filter(|listed| listed.export != Some(self.export)) {
            others.add(&paths.id(&listed.path), matches!(listed.kind, Kind::File(_)), |file, also| *file |= *also);
        }
        let (mut entries, mut seen) = (Vec::with_capacity(outputs.len()), BTreeSet::new());
        for output in outputs {
            let file = self.journal.entry_of(paths, output, self.export)?;
            let made = self.journal.made_folders(&file.path, &self.looked, &mut seen, self.export);
            entries.push(file);
            entries.extend(made);
        }
        let clash = entries.iter().find(|entry| others.of(&paths.id(&entry.path)).any(|&file| file || matches!(entry.kind, Kind::File(_))));
        if let Some(entry) = clash {
            return Err(CraftError::io(format!(
                "the output file {} is listed by an export that has not ended, so two exports cannot write it at once",
                self.journal.root.join(&entry.path).display()
            )));
        }
        let held = taken_over(&state.held, &mut entries, paths);
        let mut listed: Vec<Entry> = held.iter().chain(&state.running).cloned().collect();
        listed.extend(entries.iter().cloned());
        self.journal.write(&listed, &state.taken).map_err(|error| {
            CraftError::io(format!("cannot list the export's files in the pending-export journal {}: {error}", self.journal.file.display()))
        })?;
        state.held = held;
        state.running.extend(entries.iter().cloned());
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
            let held = journal.deal_with_all(&journal.paths(), listed.clone(), &state.running, &mut removal);
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
    /// The file entry a journal's JSON holds, or none for one that is not an entry.
    fn file_from(value: &Value) -> Option<Self> {
        let path = path_from(value)?;
        let export = listed_by(value)?;
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
        Some(Self { path, kind: Kind::File(found), export })
    }

    /// The folder entry a journal's JSON holds, or none for one that is not an entry.
    fn folder_from(value: &Value) -> Option<Self> {
        Some(Self { path: path_from(value)?, kind: Kind::Folder, export: listed_by(value)? })
    }

    fn to_value(&self) -> Value {
        let mut fields = named(&self.path);
        if let Some(export) = self.export {
            fields.insert(EXPORT.to_string(), Value::from(export));
        }
        if let Kind::File(Some(found)) = self.kind {
            fields.insert(BYTES.to_string(), Value::from(found.bytes));
            if let Some(modified) = found.modified {
                fields.insert(MODIFIED.to_string(), Value::from(modified));
            }
        }
        Value::Object(fields)
    }
}

/// Which export an entry's JSON names as having listed it: none when it names no [`EXPORT`], as an
/// entry a journal written before identities left, and no entry at all when it names something that
/// is no export's number.
fn listed_by(value: &Value) -> Option<Option<u64>> {
    match value.get(EXPORT) {
        None => Some(None),
        Some(export) => export.as_u64().map(Some),
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
/// the export wrote it, and refuse anything that is not a regular file. `absent` says the folder it
/// lies in has been listed for this operation and holds nothing of the name, which is the same answer
/// as looking at the name and finding nothing.
fn settle(target: &Path, found: Option<Stamp>, absent: bool) -> std::result::Result<Settled, Refusal> {
    let Some(now) = found_at(target, absent)? else { return Ok(Settled::Absent) };
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
/// even if the folder changed since it was looked at. `absent` is as in [`settle`].
fn settle_folder(target: &Path, absent: bool) -> std::result::Result<Settled, Refusal> {
    let Some(now) = found_at(target, absent)? else { return Ok(Settled::Absent) };
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
/// path that is now a file holds nothing either. When the folder of the name has been listed for this
/// operation and holds nothing of it (`absent`), the disk is not asked again.
fn found_at(target: &Path, absent: bool) -> std::result::Result<Option<fs::Metadata>, Refusal> {
    if absent {
        return Ok(None);
    }
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
/// deleted only if this export is killed or cancelled, and the file it finishes is kept. The file a
/// listing lists again is told as the volume holding it says, not by the spelling either entry
/// carries ([`PathId`]), so one file left by an earlier export is taken over however the new export
/// spells its path.
fn taken_over(held: &[Entry], listing: &mut [Entry], paths: &Paths) -> Vec<Entry> {
    let is_file = |entry: &Entry| matches!(entry.kind, Kind::File(_));
    // Each file's held entries are told once, and each entry of the listing looks its file up, so a
    // long listing over a long held list costs no more than the two of them. The first held entry of
    // a file is the oldest, the nearest to what was there before any export wrote it.
    let mut oldest = Tellings::default();
    for (at, entry) in held.iter().enumerate().filter(|(_, entry)| is_file(entry)) {
        oldest.add(&paths.id(&entry.path), (at, entry.kind), |_, _| {});
    }
    if oldest.is_empty() {
        return held.to_vec();
    }
    let mut listed_again = Tellings::default();
    for listed in listing.iter_mut().filter(|listed| is_file(listed)) {
        let id = paths.id(&listed.path);
        // A file may be one with held entries that are not one with each other — told by its numbers
        // with one, by where its name lies with another — and the oldest of them is what was there
        // before any export wrote it.
        if let Some(&(_, older)) = oldest.of(&id).min_by_key(|(at, _)| *at) {
            listed.kind = older;
            listed_again.add(&id, (), |_, _| {});
        }
    }
    held.iter().filter(|entry| !(is_file(entry) && listed_again.holds(&paths.id(&entry.path)))).cloned().collect()
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
fn still_set_aside(set_aside: Contents, taken: &mut Taken, root: &Path, paths: &Paths, recovery: &mut Recovery) -> Vec<Entry> {
    let Contents { entries, taken: recorded } = set_aside;
    taken.merge(recorded);
    let yields = taken.yields(paths);
    let yielding: BTreeSet<String> =
        entries.iter().filter(|entry| matches!(entry.kind, Kind::File(_)) && yields(&entry.path)).map(|entry| entry.path.clone()).collect();
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
    let mut ended: Vec<Entry> = yielding.into_iter().map(|path| Entry { path, kind: Kind::File(None), export: None }).collect();
    taken_over(&entries, &mut ended, paths)
}

/// Remove one entry equal to `entry` from `entries`: an export's end takes apart its own listing
/// alone, never another's. Two exports may each have entered one folder their files need, the
/// entries equal but for the export that entered each, and only the entry of the export that ends
/// goes, so the folder stays listed while the other still writes into it.
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
    use crate::gpu::{GpuChoice, Platform};
    use crate::testing::Gate;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::thread;
    use tempfile::TempDir;

    type Test = std::result::Result<(), Box<dyn std::error::Error>>;

    /// Whether two tellings name one file, asked as every comparison in the journal asks it: of the
    /// tellings gathered, here the one ([`Tellings`]).
    fn one_file(one: &PathId, other: &PathId) -> bool {
        let mut gathered = Tellings::default();
        gathered.add(one, (), |_, _| {});
        gathered.holds(other)
    }

    // The journal's lock is one for the whole process, and these listings are measured by the
    // clock: one export's listing of thousands of files must not lengthen another timed test's
    // every look, so the timed listings take their turns, never side by side.
    static TIMED_LISTING: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// What a listing costs the folders, not the files: the same count of files, once gathered in one
    /// folder and once spread over as many folders, tells them apart by itself, on any machine, because
    /// each side pays for what the disk is asked of and the counts of files are equal. A sequence of
    /// frames in one folder is one folder: one that is not there answers for every name below it, one
    /// that is there is listed once and answers for every name it does not hold, and a name it holds is
    /// looked at once, as itself, never resolved through every folder above it again. So the listing of
    /// a sequence costs the files and not the folders above them, whether the export writes it new or
    /// writes it over the one it wrote before, which is what lets an export of thousands of files
    /// start, and be cancelled, in the time its files take.
    #[test]
    fn the_listing_of_a_sequence_asks_its_folder_once_not_of_every_file() -> Test {
        let _turn = TIMED_LISTING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        /// The files of the phone's measured start: enough that a cost for each file outweighs a cost
        /// for each folder many times over.
        const FILES: usize = 6_000;
        for there in [false, true] {
            let (one, many) = (listing_of(FILES, 1, there)?, listing_of(FILES, FILES, there)?);
            assert!(
                one * 4 <= many,
                "{FILES} files {}in one folder cost {one:?}, the same {FILES} files in {FILES} folders cost {many:?}: a folder is looked at once, however many files lie in it",
                if there { "already there " } else { "" }
            );
        }
        Ok(())
    }

    /// The time a listing of `files` files takes, gathered into `folders` folders below the container —
    /// one file per folder at most — and already there when `there`: the faster of two listings of
    /// them, each by an export that then ends, so a pause of the machine's own costs no side its bound.
    /// The container lies deeper than a phone's does, so a resolution of a name, which looks at every
    /// folder above it, costs many looks of the name itself, as it does on the phone's disk.
    fn listing_of(files: usize, folders: usize, there: bool) -> std::result::Result<std::time::Duration, Box<dyn std::error::Error>> {
        let deep = std::iter::repeat_n("deep", 24).chain(std::iter::once("container")).collect::<Vec<_>>().join("/");
        let sand = Sandbox::at(TempDir::new()?, &deep)?;
        let paths: Vec<_> = (0..files)
            .map(|file| {
                let (folder, name) = match folders {
                    1 => ("seq".to_string(), format!("frame_{file:05}.png")),
                    _ => (format!("folder_{file:05}"), "frame.png".to_string()),
                };
                sand.folders.container.join(folder).join(name)
            })
            .collect();
        if there {
            for path in &paths {
                fs::create_dir_all(path.parent().ok_or("a frame lies in a folder")?)?;
                fs::write(path, b"an earlier frame")?;
            }
        }
        let mut fastest = std::time::Duration::MAX;
        for _ in 0..2 {
            let mut export = sand.journal().pending();
            let then = std::time::Instant::now();
            export.list(&paths)?;
            fastest = fastest.min(then.elapsed());
            export.clear()?;
        }
        println!("{files} files in {folders} folders, already there: {there}: {fastest:?} ({:.3} us each)", fastest.as_micros() as f64 / files as f64);
        Ok(fastest)
    }

    /// A folder's listing, taken once for an operation, answers for every name it lacks for the whole
    /// of it: such a name is told absent without a look of its own, so a sequence of thousands of names
    /// in one folder costs the disk one listing. A name it holds, folded as the folder's volume folds
    /// names, is looked at as itself, so another spelling of a file the volume holds as one is that
    /// file; and a listing in which two names fold alike, as they do under the reading a volume whose
    /// rules were never read is given, says nothing of any name, each of which is then looked at.
    #[test]
    fn a_folders_listing_answers_for_the_names_it_lacks_and_folds_the_names_it_holds() -> Test {
        let sand = Sandbox::new()?;
        sand.put("out/there.png", b"a file");
        let paths = sand.journal().paths();
        assert!(!paths.absent("out/there.png"), "a name the folder holds was told absent");
        assert!(paths.absent("out/missing.png"), "a name the folder's listing lacks was not told absent by it");
        sand.put("out/later.png", b"put up after the folder was listed");
        assert!(paths.absent("out/later.png"), "one operation listed the folder again, or asked the disk of a name its listing lacks");
        assert!(!sand.journal().paths().absent("out/later.png"), "the next operation did not list the folder for itself");
        for (spelt, other) in spellings_folding_to_one(&sand) {
            let (spelt, other) = (format!("out/{spelt}"), format!("out/{other}"));
            sand.put(&spelt, b"one file, two spellings");
            let paths = sand.journal().paths();
            assert!(!paths.absent(&other) && one_file(&paths.id(&other), &paths.id(&spelt)), "{other} was not told as the file {spelt} its volume holds it as");
        }
        // Under the reading that folds case, these two are one name; only a volume that keeps them
        // apart holds both, and there no name is told by a listing that cannot tell them apart.
        sand.put("apart/Apart.png", b"one");
        sand.put("apart/APART.png", b"two");
        if fs::read_dir(sand.folders.container.join("apart"))?.count() == 2 {
            let unread = Paths::with(&sand.journal().root, None);
            assert!(!unread.absent("apart/missing.png"), "a listing whose names fold alike answered for a name it lacks");
        }
        Ok(())
    }

    /// An output whose folder the volume will not resolve refuses the listing in words that name the
    /// folder as the folder it is, since it is the folder that was asked of the disk.
    #[cfg(unix)]
    #[test]
    fn an_output_whose_folder_cannot_be_resolved_is_refused_naming_its_folder() -> Test {
        let sand = Sandbox::new()?;
        if !permissions_hold(&sand)? {
            return Ok(());
        }
        let locked = sand.folders.container.join("locked");
        fs::create_dir_all(locked.join("sub"))?;
        set_mode(&locked, 0o000)?;
        let refused = sand.journal().pending().list(&[locked.join("sub/frame.png")]);
        set_mode(&locked, 0o700)?;
        let folder = locked.join("sub");
        assert_eq!(refused, Err(CraftError::io(format!("cannot resolve the output file's folder {}: Permission denied (os error 13)", folder.display()))));
        Ok(())
    }

    /// A file another export finishes while a listing waits for the journal is seen by that listing as
    /// the file it is: every look the listing takes is taken under the journal's lock, so the export
    /// that finished it could not end between the listing's look and its record, and the listing's
    /// cancel, or the start after its kill, keeps the file as a finished file it never reached. Looked
    /// at before the lock, the name was nothing yet, and the cancel or the start deleted the finished
    /// file as one the listing's export had written.
    #[test]
    fn a_file_finished_while_a_listing_waits_for_the_journal_outlives_the_listings_end() -> Test {
        for cancelled in [true, false] {
            let sand = Sandbox::new()?;
            // The folder holds something already, so its listing is what tells the name.
            sand.put("out/earlier.png", b"an earlier file");
            let journal = sand.journal();
            let late = sand.folders.container.join("out/late.png");
            let held = lock(&KNOWN);
            let listing = {
                let (late, mut export) = (late.clone(), journal.pending());
                thread::spawn(move || export.list(&[late]).map(|()| export))
            };
            // Time for a listing that looks before it takes the lock to have looked.
            thread::sleep(std::time::Duration::from_millis(300));
            fs::write(&late, b"finished by another export")?;
            drop(held);
            let export = listing.join().map_err(|_| "the listing panicked")??;
            match cancelled {
                true => export.remove()?,
                false => {
                    export.killed();
                    sand.start()?;
                }
            }
            assert_eq!(
                fs::read(&late).ok().as_deref(),
                Some(&b"finished by another export"[..]),
                "the file finished while the listing waited was deleted by the listing's {}",
                if cancelled { "cancel" } else { "kill" }
            );
        }
        Ok(())
    }

    /// The listing of one export holds the journal for one chunk of its files at a time, so the
    /// rest of the app is answered while it lists: another export's own listing is answered within
    /// the bound a start is given, its progress reports are answered while the first export is
    /// still listing, its cancel is answered, and its ending takes back what it listed, all while
    /// the first export lists eighteen thousand files.
    #[test]
    fn another_export_while_another_lists_its_files() -> Test {
        const FILES: usize = 18_000;
        const CHUNK: usize = 8_192;
        let _turn = TIMED_LISTING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let sand = Sandbox::new()?;
        let paths: Vec<PathBuf> = (0..FILES).map(|file| sand.folders.container.join(format!("seq/frame_{file:05}.png"))).collect();
        for path in &paths {
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(path, b"an earlier frame")?;
        }
        // Both engines start before either export runs: a start, and a start while another export
        // holds the journal's file lock, are questions of their own.
        let long_paths = paths.clone();
        let chunks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let chunked = chunks.clone();
        let long_engine = sand.start_running(Box::new(move |on_progress, list_outputs| {
            for chunk in long_paths.chunks(CHUNK) {
                if !on_progress(0, 0, None) {
                    return Err(CraftError::Cancelled);
                }
                list_outputs(chunk)?;
                chunked.fetch_add(1, Ordering::SeqCst);
            }
            Ok(Value::Null)
        }))?;
        let mine = sand.folders.container.join("other/frame.png");
        let listed_for = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let cost = listed_for.clone();
        let reports = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let told = reports.clone();
        let short_engine = sand.start_running(Box::new(move |on_progress, list_outputs| {
            let then = std::time::Instant::now();
            list_outputs(std::slice::from_ref(&mine))?;
            cost.store(then.elapsed().as_micros() as u64, Ordering::SeqCst);
            while on_progress(1, 10, None) {
                told.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(CraftError::Cancelled)
        }))?;
        let long_thread = thread::spawn(move || long_engine.request_with_progress("export", Value::Null, |_, _, _| {}, &CancelToken::new()));
        let short_cancel = CancelToken::new();
        let ask_short = short_cancel.clone();
        let worker = thread::spawn(move || {
            let began = std::time::Instant::now();
            let answer = short_engine.request_with_progress("export", Value::Null, |_, _, _| {}, &short_cancel);
            (answer, began.elapsed())
        });
        // The cancel is asked once the other export has a report of its own and the listing still
        // has a chunk to hand over, so it lands while the journal is held between looks at files.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while (reports.load(Ordering::SeqCst) == 0 || chunks.load(Ordering::SeqCst) < FILES / CHUNK) && std::time::Instant::now() < deadline {
            thread::sleep(std::time::Duration::from_millis(1));
        }
        let asked = std::time::Instant::now();
        ask_short.cancel();
        let (answer, _) = worker.join().map_err(|_| "the other export panicked")?;
        let long_answer = long_thread.join().unwrap();
        assert_eq!(answer, Err(CraftError::Cancelled), "the cancel of an export running beside another's listing of {FILES} files was not answered");
        assert!(asked.elapsed() <= std::time::Duration::from_secs(3), "the other export's cancel took {:?} to be answered", asked.elapsed());
        assert!(reports.load(Ordering::SeqCst) >= 1, "the other export heard no progress report of its own while the first export was still listing");
        assert!(
            listed_for.load(Ordering::SeqCst) <= 3_000_000,
            "the other export's own listing waited {:?} us, over the bound a start gives any export",
            listed_for.load(Ordering::SeqCst)
        );
        assert!(long_answer.is_ok(), "the listing export failed: {long_answer:?}");
        assert!(!sand.file().exists(), "an export that ended while another listed left its entries in the journal");
        Ok(())
    }

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

    /// An app's folders under a scratch root, with `outside` a folder beside the container.
    struct Sandbox {
        root: TempDir,
        folders: Folders,
        outside: PathBuf,
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
            Ok(Self { root, folders, outside })
        }

        fn journal(&self) -> Journal {
            Journal::new(&self.folders, &self.folders.prepare().unwrap()).unwrap()
        }

        fn file(&self) -> PathBuf {
            self.folders.config.join(JOURNAL_FILE)
        }

        /// A folder `folder` in the container, and a link `link` beside it that reaches it.
        #[cfg(unix)]
        fn linked(&self, folder: &str, link: &str) -> std::result::Result<(), io::Error> {
            fs::create_dir_all(self.folders.container.join(folder))?;
            std::os::unix::fs::symlink(self.folders.container.join(folder), self.folders.container.join(link))
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

        /// The refusal of a listing that would write `name`, which a running export of another listing
        /// holds, in its exact words.
        fn held_by_another(&self, name: &str) -> CraftError {
            CraftError::io(format!(
                "the output file {} is listed by an export that has not ended, so two exports cannot write it at once",
                self.real(name).display()
            ))
        }
    }

    /// Run `program` with `args` to its end. Other tests hold journals' locks meanwhile, and the process
    /// keeps a copy of each lock's handle until it runs its program; this process lets go of a lock by
    /// unlocking it ([`Hold`]), so that copy keeps none.
    #[cfg(all(unix, not(target_os = "android")))]
    fn run_program(program: &str, args: &[&std::ffi::OsStr]) -> io::Result<()> {
        let status = std::process::Command::new(program).args(args).status()?;
        if status.success() { Ok(()) } else { Err(io::Error::other(format!("{program} answered {status}"))) }
    }

    /// Whether a refusal of a special file is what this platform's security policy is known to give:
    /// Android's, which denies a shell run (`adb shell`, as the device runs are) a fifo, which an app
    /// may make in its own data folder, and denies the shell and every app a hard link (AOSP
    /// `system/sepolicy`: `untrusted_app_all.te` allows an app's fifo, `app_neverallows.te` forbids
    /// its hard links). Everywhere else a refusal is a real failure, so a test never goes unrun on a
    /// machine that allows what it needs.
    fn refusal_is_expected(platform: Platform, error: &io::Error) -> bool {
        platform == Platform::Android && error.kind() == io::ErrorKind::PermissionDenied
    }

    /// Say on `said` that the running test is skipped, and why. The harness names each test's thread
    /// after the test, so the line names it without the test spelling its own name. The line goes in
    /// one write: `adb shell` carries it and the harness's own output on two streams, and a line
    /// written in pieces reaches the reader split by the harness's words.
    fn say_skipped(said: &mut dyn Write, why: &str) -> io::Result<()> {
        said.write_all(format!("SKIPPED {}: {why}\n", thread::current().name().unwrap_or("a test")).as_bytes())
    }

    /// [`say_skipped`] on stderr, straight through the handle the harness does not capture, so a
    /// run's output shows every skipped test and its reason.
    fn skipped(why: &str) -> io::Result<()> {
        say_skipped(&mut io::stderr(), why)
    }

    /// What making a special file gave the running test: the file's maker, or `None` once the test
    /// has said on `said` that it is skipped and why. Any other error is the test's own.
    fn unless_refused<T>(platform: Platform, made: io::Result<T>, said: &mut dyn Write) -> io::Result<Option<T>> {
        match made {
            Ok(made) => Ok(Some(made)),
            Err(error) if refusal_is_expected(platform, &error) => {
                say_skipped(said, &format!("the platform's security policy refused the special file it needs ({error})"))?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// [`unless_refused`] for this platform, saying a skip on stderr.
    fn skipped_if_refused<T>(made: io::Result<T>) -> io::Result<Option<T>> {
        unless_refused(Platform::current(), made, &mut io::stderr())
    }

    /// Make a fifo at `path`. On Android, where a refusal skips, by the system call, whose error number
    /// tells the policy's refusal from a fault of the test's own (a folder that is not there, a name
    /// already taken): the `mkfifo` program ends in the same failure for each.
    #[cfg(target_os = "android")]
    fn make_fifo(path: &Path) -> io::Result<()> {
        use rustix::fs::{CWD, Mode, mkfifoat};
        mkfifoat(CWD, path, Mode::RUSR | Mode::WUSR).map_err(io::Error::from)
    }

    /// Make a fifo at `path` with the `mkfifo` program, where a refusal is always the test's failure.
    #[cfg(all(unix, not(target_os = "android")))]
    fn make_fifo(path: &Path) -> io::Result<()> {
        run_program("mkfifo", &[path.as_os_str()])
    }

    /// What was written, each write apart, so a test sees a line handed over in pieces.
    #[derive(Default)]
    struct Writes(Vec<Vec<u8>>);

    impl Write for Writes {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.push(bytes.to_vec());
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Only Android's refusal of a special file skips a test, and it says why, in one write; the same
    /// refusal anywhere else, and any other error on Android, is the test's failure, and a file that
    /// was made is never skipped.
    #[test]
    fn a_special_file_is_skipped_only_where_the_platform_refuses_it() -> Test {
        for platform in [Platform::Ios, Platform::Android, Platform::Host] {
            let mut said = Writes::default();
            assert_eq!(unless_refused(platform, Ok(7), &mut said)?, Some(7), "{platform:?} skipped a file that was made");
            assert!(said.0.is_empty(), "{platform:?} said something of a file that was made");
            for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::NotFound, io::ErrorKind::Other] {
                let outcome = unless_refused::<()>(platform, Err(io::Error::from(kind)), &mut said);
                let skipped = platform == Platform::Android && kind == io::ErrorKind::PermissionDenied;
                assert_eq!(matches!(outcome, Ok(None)), skipped, "{platform:?} {kind:?}: {outcome:?}");
                assert_eq!(outcome.is_err(), !skipped, "{platform:?} {kind:?} neither skipped nor failed");
            }
            let said = said.0.into_iter().map(String::from_utf8).collect::<std::result::Result<Vec<_>, _>>()?;
            let reason = format!(
                "SKIPPED {}: the platform's security policy refused the special file it needs (permission denied)\n",
                thread::current().name().unwrap_or("a test")
            );
            assert_eq!(said, if platform == Platform::Android { vec![reason] } else { Vec::new() }, "{platform:?}");
        }
        Ok(())
    }

    /// A skip reaches the run's own stderr, past the harness's capture of a test's output, which no
    /// test sees from inside. So a child of this binary says one, as the harness runs it by default
    /// (captured), and the parent reads the child's two streams. (An iPhone starts no process.)
    #[cfg(not(target_os = "ios"))]
    #[test]
    fn a_skip_is_said_on_stderr_past_the_harnesss_capture() -> Test {
        let mut child = std::process::Command::new(std::env::current_exe()?);
        child.args(["--exact", CHILD_THAT_SKIPS, "--ignored", "--test-threads=1"]);
        let output = craftbridge_process::output_of(&mut child)?;
        let (said, out) = (String::from_utf8_lossy(&output.stderr), String::from_utf8_lossy(&output.stdout));
        assert!(output.status.success(), "the child failed:\n{out}\n{said}");
        assert!(out.contains("running 1 test"), "the name {CHILD_THAT_SKIPS} ran no test:\n{out}");
        assert_eq!(said.lines().collect::<Vec<_>>(), [format!("SKIPPED {CHILD_THAT_SKIPS}: a reason said to be read")], "stderr");
        assert!(!out.contains("SKIPPED"), "the skip went to stdout:\n{out}");
        Ok(())
    }

    #[cfg(not(target_os = "ios"))]
    const CHILD_THAT_SKIPS: &str = "journal::tests::child_that_skips";

    #[cfg(not(target_os = "ios"))]
    #[test]
    #[ignore = "run by a_skip_is_said_on_stderr_past_the_harnesss_capture, which reads what it says"]
    fn child_that_skips() -> Test {
        Ok(skipped("a reason said to be read")?)
    }

    /// On a desktop the fifo and the hard link the skipping tests need are made, so none of those
    /// tests is skipped there. (An iPhone starts no `mkfifo`, so its run leaves out the tests that do.)
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    #[test]
    fn a_desktop_makes_the_special_files_the_skipping_tests_need() -> Test {
        let sand = Sandbox::new()?;
        sand.write_journal(&[]);
        make_fifo(&sand.folders.config.join("fifo"))?;
        fs::hard_link(sand.file(), sand.folders.config.join("link"))?;
        Ok(())
    }

    /// A fifo that cannot be made for a fault of the test's own, a folder that is not there or a name
    /// already taken, is never an error Android's refusal is told by, so it fails the test there too.
    /// (An iPhone starts no `mkfifo`.)
    #[cfg(all(unix, not(target_os = "ios")))]
    #[test]
    fn a_fifo_the_test_itself_cannot_make_is_never_taken_for_a_refusal() -> Test {
        let sand = Sandbox::new()?;
        let taken = sand.put("taken", b"a file");
        for path in [sand.folders.container.join("missing/fifo"), taken] {
            let error = make_fifo(&path).err().ok_or("the fifo was made")?;
            assert!(!refusal_is_expected(Platform::Android, &error), "{}: {error}", path.display());
        }
        Ok(())
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
    /// writes whatever its mode, so a test of a disk that refuses cannot be made that way for it, and
    /// the test that asked is said to be skipped.
    #[cfg(unix)]
    fn permissions_hold(sand: &Sandbox) -> std::result::Result<bool, io::Error> {
        use std::os::unix::fs::PermissionsExt;
        let probe = sand.folders.cache.join("permission-probe");
        fs::write(&probe, b"x")?;
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o000))?;
        let held = fs::read(&probe).is_err();
        fs::remove_file(&probe)?;
        if !held {
            skipped("the permissions do not hold this user back (root)")?;
        }
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

    /// The file that was at a listed path is told from one written there since by its size and by its
    /// modification time, each alone: a rewrite of as many bytes by its later time, and one within the
    /// grain of the disk's clock, which on a phone gives two writes in a row one time, by its size; the
    /// same bytes at the same time are the listed file, which stays. The times are set rather than
    /// waited for, so no clock's grain decides the test.
    #[test]
    fn a_rewrite_of_a_listed_file_is_told_by_its_size_or_its_modification_time_alone() -> Test {
        let listed = b"last week's movie";
        let at = UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000);
        let later = at + std::time::Duration::from_secs(1);
        let same_size = listed.to_ascii_uppercase();
        for (written, when, deleted) in [(&listed[..], at, false), (&same_size[..], later, true), (&listed[..4], at, true)] {
            let sand = Sandbox::new()?;
            let card = sand.put("card.mp4", listed);
            File::options().write(true).open(&card)?.set_modified(at)?;
            let mut export = sand.journal().pending();
            export.list(std::slice::from_ref(&card))?;
            fs::write(&card, written)?;
            File::options().write(true).open(&card)?.set_modified(when)?;
            export.killed();
            let engine = sand.start()?;
            assert_eq!(card.exists(), !deleted, "{} bytes at {when:?}: {:?}", written.len(), engine.recovery());
        }
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
        let Sandbox { root, folders, .. } = before;
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
            let still = if refused == Refused::Folder { r#""path":"exports"}"# } else { "card.mp4" };
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

    /// What a later run that is killed writes over [`FINISHED`]. A file is told from the one that was
    /// listed by its size and modification time, and a phone's clock ticks too coarsely to tell two
    /// writes a test makes in a row apart, so the size must differ.
    #[cfg(unix)]
    const PARTIAL_OVER_FINISHED: &[u8] = b"half a newer movie, cut short";
    #[cfg(unix)]
    const _: () = assert!(PARTIAL_OVER_FINISHED.len() != FINISHED.len());

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
                        let _other = held_elsewhere(&sand)?;
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
            r#"{"outputs": [{"path": "card.mp4", "export": "one"}]}"#,
            r#"{"files": []}"#,
            r#"{"outputs": [{"path": "card.mp4"}], "folders": "exports"}"#,
            r#"{"outputs": [{"path": "card.mp4"}], "folders": [{"path": 1}]}"#,
            r#"{"outputs": [{"path": "card.mp4"}], "folders": ["exports"]}"#,
            r#"{"outputs": [{"path": "card.mp4"}], "folders": [{"path": "exports", "export": [1]}]}"#,
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

    /// The journal's lock, taken as another process takes it, through a handle of its own, until the
    /// hold drops.
    fn held_elsewhere(sand: &Sandbox) -> io::Result<Hold> {
        let handle = lock_handle(sand)?;
        handle.lock()?;
        Ok(Hold(handle))
    }

    /// Whether another process could take the journal's lock now: a handle of its own on the lock file,
    /// as another process has, takes it, and lets it go as a hold does, whether it took it or not, since
    /// unlocking a handle that holds nothing changes nothing.
    fn lock_is_free(sand: &Sandbox) -> io::Result<bool> {
        let probe = lock_handle(sand)?;
        let free = probe.try_lock().is_ok();
        drop(Hold(probe));
        Ok(free)
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
        // Every entry of one listing names the same export, by [`EXPORT`].
        let by = export.export;
        assert_eq!(journal[FOLDERS], json!([{ PATH: "seq/a", EXPORT: by }, { PATH: "seq", EXPORT: by }, { PATH: "seq/b", EXPORT: by }]), "{journal}");
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
        assert_eq!(now[FOLDERS], json!([{ PATH: "made", EXPORT: first.export }]), "{now}");
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
        run_program("chflags", &["uchg".as_ref(), older.as_os_str()])?;
        let first = sand.start();
        run_program("chflags", &["nouchg".as_ref(), older.as_os_str()])?;
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
        run_program("chflags", &["uchg".as_ref(), journal.as_os_str()])?;
        let first = sand.start();
        run_program("chflags", &["nouchg".as_ref(), journal.as_os_str()])?;
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
                fs::write(&card, PARTIAL_OVER_FINISHED)?;
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
        fs::write(&card, PARTIAL_OVER_FINISHED)?;
        killed.killed();
        set_mode(&sand.file(), 0o000)?;
        sand.start()?;
        for shelf in shelved(&sand)? {
            set_mode(&shelf, 0o600)?;
        }
        let later = sand.start()?;
        let recovery = later.recovery();
        assert_eq!(fs::read(&card)?, PARTIAL_OVER_FINISHED, "{recovery:?}");
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
        let other = held_elsewhere(&sand)?;

        let engine = sand.start()?;
        assert_eq!(fs::read(&running)?, b"another process is writing this", "a start deleted another process's running export");
        assert_eq!(fs::read(sand.file())?, before, "a start wrote a journal another process holds");
        let recovery = engine.recovery();
        assert!(recovery.removed.is_empty() && recovery.untouched.is_empty(), "{recovery:?}");
        assert!(recovery.journal.iter().any(|line| line.contains("another process holds the lock")), "{recovery:?}");
        assert_listing_refused(&sand, "another process holds the lock");
        let again = sand.start()?;
        assert!(running.exists() && again.recovery().journal.iter().any(|line| line.contains("another process holds the lock")), "{:?}", again.recovery());

        drop(other);
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

    /// A sandbox, the writer that frees its start's read of the journal, and the start.
    #[cfg(unix)]
    type BlockedStart = (Sandbox, fs::File, thread::JoinHandle<std::result::Result<Recovery, CraftError>>);

    /// A fresh sandbox whose start is held mid-read, for a test that wants to run while a start of its
    /// own holds the journal's lock: the journal is a pipe the start waits to read, which slows the
    /// start itself and nothing else. Opening the returned writer returns once the start has opened
    /// the pipe to read, which it does after taking the lock; whatever the test does next runs while
    /// the start holds the lock and has dealt with nothing. Writing to the writer ends the read, and
    /// joining the start answers what it dealt with.
    /// `None` where the platform refuses the fifo, with the skip said ([`unless_refused`]).
    #[cfg(unix)]
    fn start_blocked_reading_the_journal() -> io::Result<Option<BlockedStart>> {
        let pipes = TempDir::new()?;
        let pipe = pipes.path().join("journal");
        if skipped_if_refused(make_fifo(&pipe))?.is_none() {
            return Ok(None);
        }
        let sand = Sandbox::new()?;
        std::os::unix::fs::symlink(&pipe, sand.file())?;
        let start = thread::spawn({
            let folders = sand.folders.clone();
            move || Engine::start_in(&folders, Scripted::idle(), GpuChoice::CpuOnly).map(|engine| engine.recovery().clone())
        });
        // Both ends of the pipe are open by the time this returns, so the pipe's own scratch folder
        // can go: the read carries on through the open handles.
        fs::OpenOptions::new().write(true).open(&pipe).map(|writer| Some((sand, writer, start)))
    }

    /// A start holds the journal's lock from before it reads the journal until it has dealt with it, so
    /// no other process sets the journal aside or deletes a listed file while it does. The journal is a
    /// pipe the start waits to read, which slows the start itself and nothing else.
    #[cfg(unix)]
    #[test]
    fn a_start_holds_the_journals_lock_while_it_deals_with_the_journal() -> Test {
        let Some((sand, mut writer, start)) = start_blocked_reading_the_journal()? else {
            return Ok(());
        };
        let free_while_reading = lock_is_free(&sand);
        writer.write_all(br#"{"outputs": []}"#)?;
        drop(writer);
        let recovery = start.join().map_err(|_| "the start panicked")??;
        assert!(!free_while_reading?, "another process could take the journal while a start dealt with it");
        assert!(recovery.journal.is_empty(), "{recovery:?}");
        assert!(lock_is_free(&sand)?, "the start kept the lock once it was done");
        Ok(())
    }

    /// A start deals with the journal in process: its listing and clearing wait for a start of this
    /// process that is reading it, rather than running through it or being refused by it. So an export
    /// never writes the journal under a start's nose, and no start deletes the finished file of an
    /// export that ended while it was still reading, which is what a listing kept for its own export
    /// and cleared when that export ended.
    #[cfg(unix)]
    #[test]
    fn an_export_lists_only_after_a_start_has_read_the_journal() -> Test {
        let Some((sand, mut writer, start)) = start_blocked_reading_the_journal()? else {
            return Ok(());
        };
        sand.put("stale.mp4", b"a killed run left this");
        let (sent, listed) = mpsc::channel();
        let export = thread::spawn({
            let folders = sand.folders.clone();
            move || -> Result<()> {
                let journal = Journal::new(&folders, &folders.prepare().unwrap()).unwrap();
                let movie = folders.container.join("movie.mp4");
                let mut export = journal.pending();
                let got = export.list(std::slice::from_ref(&movie));
                let _ = sent.send(got.is_ok());
                got?;
                fs::write(&movie, b"a finished movie").map_err(|error| CraftError::io(error.to_string()))?;
                export.clear()
            }
        });
        // The start is still reading the journal, so the export's listing waits: nothing came through.
        assert!(
            matches!(listed.recv_timeout(std::time::Duration::from_secs(2)), Err(mpsc::RecvTimeoutError::Timeout)),
            "the export's listing came through, or failed, while a start was still reading the journal"
        );
        // The start's read ends and it deals with the journal it read; the listing, which waited,
        // then writes its entry.
        writer.write_all(br#"{"outputs": [{"path": "stale.mp4"}]}"#)?;
        drop(writer);
        let recovery = start.join().map_err(|_| "the start panicked")??;
        assert_eq!(recovery.removed, [sand.real("stale.mp4")], "{recovery:?}");
        export.join().map_err(|_| "the export panicked")??;
        // The export wrote its file and finished; the start that was mid-read never saw its entry, so
        // the file stays through that start and the next.
        let movie = sand.folders.container.join("movie.mp4");
        assert_eq!(fs::read(&movie)?, b"a finished movie");
        let later = sand.start()?;
        assert!(later.recovery().removed.is_empty(), "{:?}", later.recovery());
        assert_eq!(fs::read(&movie)?, b"a finished movie", "a start deleted a file a finished export wrote");
        Ok(())
    }

    /// The race, made to happen every time: one engine, the handle of its lock work held open, then an
    /// export. A process the app starts from any thread keeps a copy of every handle the app has open
    /// until it runs its program, the one the journal's lock is held through among them, and the lock
    /// belongs to the open file those copies share. Here a copy of the handle the engine's export holds
    /// the lock through outlives the export. The process has let go of the lock, so its next export
    /// lists, its next start deals with the journal, and another process could take it: none of them
    /// hears of another process holding it.
    #[test]
    fn a_process_the_app_starts_keeps_no_lock_once_the_app_lets_go() -> Test {
        let sand = Sandbox::new()?;
        let journal = sand.journal().file;
        let card = sand.folders.container.join("card.mp4");
        let (sent, copies) = mpsc::channel();
        let engine = sand.start_running(Box::new(move |_, list_outputs| {
            list_outputs(std::slice::from_ref(&card))?;
            // What a process the app starts now keeps: a copy of the handle the export holds the lock through.
            let copy = lock(&KNOWN).get(&journal).and_then(|ours| ours.lock.as_ref()).map(|held| held.0.try_clone());
            sent.send(copy).unwrap();
            Ok(Value::Null)
        }))?;
        let export = || engine.request_with_progress("export", Value::Null, |_, _, _| {}, &CancelToken::new());
        export()?;
        let copy = copies.recv()?.ok_or("the export held no lock")??;
        let again = export();
        assert!(again.is_ok(), "the engine's next export was refused for a lock its process had let go of: {again:?}");
        let started = sand.start()?;
        assert!(started.recovery().journal.is_empty(), "{:?}", started.recovery());
        assert!(lock_is_free(&sand)?, "a copy of the handle kept the lock after the process let go of it");
        drop(copy);
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
        let other = held_elsewhere(&sand)?;
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
        if skipped_if_refused(fs::hard_link(sand.file(), &old))?.is_none() {
            return Ok(());
        }

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
        let entries: Vec<Entry> = (0..17).map(|n| Entry { path: format!("{n}{long}"), kind: Kind::File(None), export: None }).collect();
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

    /// A second export of the process may not list a file a running export has listed and not ended:
    /// its listing is refused, before any of it is written, in exactly these words, and the export
    /// answers the refusal without touching the file the first export is writing. Once the first has
    /// ended, a later export may list the path after all.
    #[test]
    fn a_second_export_of_a_running_exports_file_is_refused_in_its_exact_words() -> Test {
        let sand = Sandbox::new()?;
        let output = sand.folders.container.join("card.mp4");
        let (script, listed, release) = held_export(&output, true);
        let first = Arc::new(sand.start_running(script)?);
        let export = exporting(&first);
        assert!(listed.wait(), "the export never listed its output");

        let held = output.clone();
        let second = sand.start_running(Box::new(move |_, list_outputs| list_outputs(std::slice::from_ref(&held)).map(|()| Value::Null)))?;
        let refused = second.request_with_progress("export", Value::Null, |_, _, _| {}, &CancelToken::new());
        assert_eq!(refused, Err(sand.held_by_another("card.mp4")), "the refusal, in its exact words: {refused:?}");
        assert_eq!(fs::read(&output)?, b"being written", "a refused second export touched the file the first export is writing");
        assert!(fs::read_to_string(sand.file())?.contains("card.mp4"), "the refused listing disturbed the running export's entry");

        release.open();
        assert_eq!(export.join().unwrap(), Ok(Value::Null));
        assert!(!sand.file().exists(), "an entry outlived the export");
        // The path belongs to no running export now: a later export may list it, and its own end
        // leaves the file the first export finished, having never written over it.
        let mut later = sand.journal().pending();
        later.list(std::slice::from_ref(&output))?;
        later.clear()?;
        assert_eq!(fs::read(&output)?, b"being written");
        Ok(())
    }

    /// Refusal is one export's path from another's, not a path from itself: an export may list its
    /// own path again, and its end clears both of its listings of it.
    #[test]
    fn an_export_may_list_its_own_path_again() -> Test {
        let sand = Sandbox::new()?;
        let output = sand.folders.container.join("card.mp4");
        let mut export = sand.journal().pending();
        export.list(std::slice::from_ref(&output))?;
        export.list(std::slice::from_ref(&output))?;
        let journal: Value = serde_json::from_slice(&fs::read(sand.file())?)?;
        assert_eq!(journal[OUTPUTS].as_array().map(Vec::len), Some(2), "{journal}");
        export.clear()?;
        assert!(!sand.file().exists());
        Ok(())
    }

    /// A second listing of the path a running export has listed is refused before any of it is
    /// written, so the refused export has nothing of its own to delete however it ends — and, on a
    /// volume that folds them, so is a second listing of the same file spelt another way, which is
    /// the same one file. The order is the one in which entries told apart by their value lost the
    /// file: the second export lists while nothing is at the path yet, the holder writes there and
    /// finishes, and the second ends after it, cancelled, killed or failed. The file the holder
    /// finished stays through each ending and the restart after it, the refused listing wrote
    /// nothing, and the refusal names the path as the second export spelt it.
    #[test]
    fn a_refused_export_deletes_nothing_of_the_export_that_holds_the_path() -> Test {
        // One path as one export lists it, and — where this volume folds them — that file spelt
        // another way: other case, another Unicode form, and both at once.
        let mut spellings = vec![("card.mp4".to_string(), "card.mp4".to_string())];
        spellings.extend(spellings_folding_to_one(&Sandbox::new()?));
        for (spelling, another) in &spellings {
            for (ending, end) in endings() {
                let sand = Sandbox::new()?;
                let output = sand.folders.container.join(spelling);
                let alt = sand.folders.container.join(another);
                let (mut holder, mut refused) = (sand.journal().pending(), sand.journal().pending());
                let held_by = holder.export;
                holder.list(std::slice::from_ref(&output))?;
                let listing = refused.list(std::slice::from_ref(&alt));
                let after_refusal: Value = serde_json::from_slice(&fs::read(sand.file())?)?;
                sand.put(spelling, b"the finished movie");
                holder.clear()?;
                end(refused)?;
                let engine = sand.start()?;
                let kept = fs::read(&output).ok();
                assert_eq!(kept.as_deref(), Some(&b"the finished movie"[..]), "{spelling} and {another}, {ending}: the holder's finished file was lost");
                assert_eq!(engine.recovery(), &Recovery::default(), "{spelling} and {another}, {ending}");
                assert!(!sand.file().exists(), "{spelling} and {another}, {ending}: the journal outlived both exports");
                assert_eq!(listing, Err(sand.held_by_another(another)), "{spelling} and {another}, {ending}");
                assert_eq!(
                    after_refusal[OUTPUTS],
                    json!([{ PATH: spelling, EXPORT: held_by }]),
                    "{spelling} and {another}, {ending}: the refused listing was written: {after_refusal}"
                );
            }
        }
        Ok(())
    }

    /// A path is one export's whatever the other would put there: a file where a running export's
    /// writer makes a folder for its own files is refused, and so is a file whose path needs a folder
    /// where a running export writes a file, each in the refusal's words naming that path.
    #[test]
    fn a_file_and_a_folder_at_one_path_are_two_exports_writing_it() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        let (mut sequence, mut over_its_folder) = (sand.journal().pending(), sand.journal().pending());
        sequence.list(&[container.join("frames/f_00000.png")])?;
        assert_eq!(over_its_folder.list(&[container.join("frames")]), Err(sand.held_by_another("frames")));
        sequence.clear()?;
        let (mut movie, mut below_it) = (sand.journal().pending(), sand.journal().pending());
        movie.list(&[container.join("card")])?;
        assert_eq!(below_it.list(&[container.join("card/f_00000.png")]), Err(sand.held_by_another("card")));
        movie.clear()?;
        assert!(!sand.file().exists(), "a refused listing was written");
        Ok(())
    }

    /// A running export's path holds however its own entry names it, not only by the kind this
    /// listing would put there: an export that listed `card` as a file and `card/x.png` — whose
    /// writer can make only one of the two, but whose entries hold the name either way — holds
    /// `card` against a second export whose own new file would need the folder `card`, and the
    /// refusal names it. Every entry of a listing counts, so no entry's kind hides another's.
    #[test]
    fn a_path_held_as_a_file_and_as_a_folder_holds_by_both() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        let mut both = sand.journal().pending();
        both.list(&[container.join("card"), container.join("card/x.png")])?;
        let mut other = sand.journal().pending();
        assert_eq!(other.list(std::slice::from_ref(&container.join("card/y.png"))), Err(sand.held_by_another("card")));
        both.clear()?;
        assert!(!sand.file().exists(), "a refused listing was written");
        Ok(())
    }

    /// A cancelled export removes no folder another running export of the process still needs: the
    /// new folder both exports' files need stays, though the cancel leaves it empty, listed by the
    /// other export alone, and goes once no running export needs it, with the other's own cancel.
    #[test]
    fn a_cancelled_export_leaves_a_folder_another_running_export_needs() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        let (mut running, mut cancelled) = (sand.journal().pending(), sand.journal().pending());
        running.list(&[container.join("shared/one.png")])?;
        cancelled.list(&[container.join("shared/two.png")])?;
        let written = sand.put("shared/two.png", b"half");
        cancelled.remove()?;
        assert!(!written.exists(), "the cancel left the file it wrote");
        assert!(container.join("shared").is_dir(), "a cancel removed the folder another running export needs");
        let journal: Value = serde_json::from_slice(&fs::read(sand.file())?)?;
        assert_eq!(journal[FOLDERS], json!([{ PATH: "shared", EXPORT: running.export }]), "{journal}");
        sand.put("shared/one.png", b"half");
        running.remove()?;
        assert!(!container.join("shared").exists(), "the folder outlived every export that needed it");
        assert!(!sand.file().exists());
        Ok(())
    }

    /// A dead process's exports may carry the numbers this process's running exports carry, which is
    /// harmless: an entry read from a journal is no running export's, whatever number it names. An
    /// export's end takes nothing of it, and a start deals with it by its own record while an export
    /// of the same number runs.
    #[test]
    fn a_dead_processs_export_numbers_equal_to_live_ones_belong_to_no_live_export() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        // What a killed process left: a partial file and the folder its writer made, listed by an
        // export of the number `live` carries.
        let left = |live: &Pending| {
            let partial = sand.put("dead/card.mp4", b"half");
            sand.write_journal_with(&[json!({ PATH: "dead/card.mp4", EXPORT: live.export })], &[json!({ PATH: "dead", EXPORT: live.export })]);
            partial
        };
        let mut ending = sand.journal().pending();
        let number = ending.export;
        let partial = left(&ending);
        ending.list(&[container.join("live/card.mp4")])?;
        sand.put("live/card.mp4", b"half of the live one");
        ending.remove()?;
        assert!(partial.exists(), "an export's end deleted what a dead export of its number left");
        let journal: Value = serde_json::from_slice(&fs::read(sand.file())?)?;
        assert_eq!(journal[OUTPUTS], json!([{ PATH: "dead/card.mp4", EXPORT: number }]), "an export's end took a dead export's entry: {journal}");
        assert_eq!(journal[FOLDERS], json!([{ PATH: "dead", EXPORT: number }]), "{journal}");

        let mut running = sand.journal().pending();
        let partial = left(&running);
        running.list(&[container.join("live/card.mp4")])?;
        let engine = sand.start()?;
        assert_eq!(engine.recovery().removed, [sand.real("dead/card.mp4"), sand.real("dead")], "{:?}", engine.recovery());
        assert!(!partial.exists());
        running.clear()?;
        assert!(!sand.file().exists());
        Ok(())
    }

    /// What a later export of a finished export's path does at its end.
    #[derive(Clone, Copy, Debug)]
    enum Ending {
        /// Cancelled before it reached the path: the finished file stays as it was.
        CancelledBeforeReaching,
        /// Cancelled after it wrote its own file there: what it wrote goes, since it wrote it.
        CancelledAfterReaching,
        /// Killed before it reached the path: the next start keeps the file it found when it listed.
        KilledBeforeReaching,
        /// Failed there, whatever it left: the entries clear and the file stays.
        Failed,
        /// Killed after it wrote: the next start deletes exactly what that export wrote.
        KilledAfterReaching,
    }

    /// A finished export keeps its file whatever a later export of the same path does, before a
    /// restart and after one. The later export may only list the path once the first has ended, and
    /// its entry records the finished file as what was there before: a cancel before it reaches the
    /// path, a kill that leaves an entry no write followed, and a failure all leave that file, and
    /// only a cancel or a kill after the export wrote there removes anything, and then exactly the
    /// bytes that export wrote, never the finished file it found.
    #[test]
    fn a_finished_export_keeps_its_file_through_every_ending_of_a_later_export_of_its_path() -> Test {
        let endings =
            [Ending::CancelledBeforeReaching, Ending::CancelledAfterReaching, Ending::KilledBeforeReaching, Ending::Failed, Ending::KilledAfterReaching];
        for ending in endings {
            let sand = Sandbox::new()?;
            let output = sand.folders.container.join("card.mp4");
            let (mut finished, mut later) = (sand.journal().pending(), sand.journal().pending());
            finished.list(std::slice::from_ref(&output))?;
            sand.put("card.mp4", b"the finished movie");
            finished.clear()?;
            later.list(std::slice::from_ref(&output))?;
            match ending {
                Ending::CancelledBeforeReaching => {
                    later.remove()?;
                    assert_eq!(fs::read(&output)?, b"the finished movie", "a cancel before reaching the path deleted a finished file");
                }
                Ending::CancelledAfterReaching => {
                    sand.put("card.mp4", b"half a newer movie, cut short");
                    later.remove()?;
                    assert!(!output.exists(), "a cancel after writing left the file this export wrote");
                }
                Ending::Failed => {
                    later.clear()?;
                    assert_eq!(fs::read(&output)?, b"the finished movie", "a failed export deleted what it listed");
                }
                Ending::KilledBeforeReaching => {
                    later.killed();
                    let engine = sand.start()?;
                    assert_eq!(engine.recovery().untouched, [sand.real("card.mp4")], "{:?}", engine.recovery());
                    assert_eq!(fs::read(&output)?, b"the finished movie", "a restart deleted the file a killed run listed but never wrote");
                }
                Ending::KilledAfterReaching => {
                    sand.put("card.mp4", b"half a newer movie, cut short");
                    later.killed();
                    let engine = sand.start()?;
                    assert_eq!(engine.recovery().removed, [sand.real("card.mp4")], "{:?}", engine.recovery());
                    assert!(!output.exists(), "a restart kept the partial file a killed run wrote");
                }
            }
            assert!(!sand.file().exists(), "{ending:?}: the journal outlived the endings");
        }
        Ok(())
    }

    /// Every entry names the export that listed it, and an export's end takes apart exactly its own
    /// entries: two exports whose new files share one folder keep that folder's entry apart, and it
    /// stays listed as one export's while the other still writes into it, going only once the last
    /// of them has ended.
    #[test]
    fn the_journal_names_the_export_that_made_each_entry() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        let (mut first, mut second) = (sand.journal().pending(), sand.journal().pending());
        assert_ne!(first.export, second.export, "two exports of one process share an identity");
        first.list(&[container.join("shared/one.png")])?;
        second.list(&[container.join("shared/two.png")])?;
        let journal: Value = serde_json::from_slice(&fs::read(sand.file())?)?;
        assert_eq!(journal[FOLDERS], json!([{ PATH: "shared", EXPORT: first.export }, { PATH: "shared", EXPORT: second.export }]), "{journal}");
        assert_eq!(journal[OUTPUTS], json!([{ PATH: "shared/one.png", EXPORT: first.export }, { PATH: "shared/two.png", EXPORT: second.export }]), "{journal}");
        // The later export's end takes its own entries only: the folder stays listed, as first's.
        second.clear()?;
        let journal: Value = serde_json::from_slice(&fs::read(sand.file())?)?;
        assert_eq!(journal[FOLDERS], json!([{ PATH: "shared", EXPORT: first.export }]), "one export's end cleared the other's folder entry: {journal}");
        assert_eq!(journal[OUTPUTS], json!([{ PATH: "shared/one.png", EXPORT: first.export }]), "{journal}");
        first.clear()?;
        assert!(!sand.file().exists());
        Ok(())
    }

    /// The export an entry names is part of the record: an entry a start could not delete keeps the
    /// number of the export that listed it through the rewrite that lists it again, and when a later
    /// export lists the path and takes the entry over, the record of what was there before comes
    /// along with it while the entry becomes the listing export's own.
    #[cfg(unix)]
    #[test]
    fn a_journals_export_identity_survives_a_restart_and_a_take_over() -> Test {
        use std::os::unix::fs::PermissionsExt;
        let sand = Sandbox::new()?;
        if !permissions_hold(&sand)? {
            return Ok(());
        }
        let file = sand.put("locked/card.mp4", b"the finished movie");
        let held = json!({ PATH: "locked/card.mp4", BYTES: 18, MODIFIED: 1234, EXPORT: 9 });
        sand.write_journal(std::slice::from_ref(&held));
        let locked = sand.folders.container.join("locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500))?;
        let engine = sand.start()?;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700))?;
        if engine.recovery().failed.is_empty() {
            // A user the permissions do not hold back (root) deleted it anyway.
            return Ok(());
        }
        let now: Value = serde_json::from_slice(&fs::read(sand.file())?)?;
        assert_eq!(now[OUTPUTS], json!([held]), "the rewrite lost the export that listed the entry it could not delete: {now}");
        // A later export listing the path takes the entry over: the record of what was there comes
        // along, and the identity becomes the listing export's.
        listed_as_one_entry(&sand, &file, "locked/card.mp4", 18, 1234)?;
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
        if !partial.exists() {
            // A user the permissions do not hold back (root) deletes it anyway, and the emptied
            // folder goes with it, so there is nothing left to open again.
            return Ok(());
        }
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700))?;
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

    // ——— One file spelt two ways: every comparison asks the volume that holds the paths ———

    /// The endings an export can have in these tests, and how to end one so: cancelled deletes what
    /// it wrote, killed leaves the journal and forgets the export, as a kill does, and failed keeps
    /// what the export wrote. Where a start meets the killed export after the other export of the
    /// path has ended, the killed one is all that process was ever running.
    fn endings() -> [(&'static str, fn(Pending) -> Result<()>); 3] {
        [
            ("cancelled", Pending::remove),
            ("killed", |ended| {
                ended.killed();
                Ok(())
            }),
            ("failed", Pending::clear),
        ]
    }

    /// The rules this process read from the volume the sandbox's journal lies on.
    fn volume_rules(sand: &Sandbox) -> VolumeRules {
        VolumeRules::read(&sand.folders.config).map_or_else(VolumeRules::default, |(_, rules)| rules)
    }

    /// The spellings of one name this volume folds to one file: other case, if it folds case, both
    /// as lower-casing folds it and as only Unicode's full case folding does (`ß` against `SS`, which
    /// a Mac's disk holds as one name); another Unicode form, if it folds that; and one differing in
    /// both at once, if it folds both. A volume that keeps every spelling apart has none here, since
    /// a test that needs one file named two ways then has two files — the volume of two files has
    /// its own test next to these.
    fn spellings_folding_to_one(sand: &Sandbox) -> Vec<(String, String)> {
        let VolumeRules { case_sensitive, form_sensitive } = volume_rules(sand);
        let mut pairs = Vec::new();
        if !case_sensitive {
            pairs.push(("Card.MP4".to_string(), "CARD.MP4".to_string()));
            pairs.push(("Stra\u{df}e.mp4".to_string(), "STRASSE.MP4".to_string()));
        }
        if !form_sensitive {
            pairs.push(("\u{e1}lbum.mov".to_string(), "a\u{301}lbum.mov".to_string()));
        }
        if !case_sensitive && !form_sensitive {
            pairs.push(("\u{c1}lb\u{fa}m.MOV".to_string(), "a\u{301}lbu\u{301}m.mov".to_string()));
        }
        pairs
    }

    /// One name and another spelling of it that this volume folds to one file, if it folds any.
    /// Two spellings this volume folds to one name, or none after saying why, when the volume keeps
    /// every spelling apart and there is nothing for the journal to fold.
    fn folded_or_skip(sand: &Sandbox) -> std::result::Result<Option<(String, String)>, io::Error> {
        if let Some(pair) = one_spelling_folded(sand) {
            return Ok(Some(pair));
        }
        skipped("this volume keeps every spelling of a name apart")?;
        Ok(None)
    }

    /// A newer export lists `file` and the journal is read back showing one entry for it: the newer
    /// export's own, holding the stamp `bytes` and `modified` the other entry was holding — the take-
    /// over one path is, whatever spelling its listing came in under. That export is then cleared.
    fn listed_as_one_entry(sand: &Sandbox, file: &PathBuf, path: &str, bytes: u64, modified: u64) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut later = sand.journal().pending();
        later.list(std::slice::from_ref(file))?;
        let now: Value = serde_json::from_slice(&fs::read(sand.file())?)?;
        assert_eq!(
            now[OUTPUTS],
            json!([{ PATH: path, EXPORT: later.export, BYTES: bytes, MODIFIED: modified }]),
            "a newer listing did not become one entry carrying the older record: {now}"
        );
        later.clear()?;
        Ok(())
    }

    fn one_spelling_folded(sand: &Sandbox) -> Option<(String, String)> {
        spellings_folding_to_one(sand).into_iter().next()
    }

    /// A pair of spellings of one name this volume keeps apart — the case pair where it reads case,
    /// the form pair where it reads form — and none where it folds both, since then no two spellings
    /// of one name anywhere under it are two files.
    fn one_spelling_kept_apart(sand: &Sandbox) -> Option<(String, String)> {
        let VolumeRules { case_sensitive, form_sensitive } = volume_rules(sand);
        if case_sensitive {
            Some(("Card.MP4".to_string(), "CARD.MP4".to_string()))
        } else if form_sensitive {
            Some(("\u{e1}lbum.mov".to_string(), "a\u{301}lbum.mov".to_string()))
        } else {
            None
        }
    }

    /// Whether a name and another spelling of it name one file on this volume: a file is put up
    /// under the first in the test's cache folder and the second is looked at for the same file.
    /// That is the volume's answer told another way than the probe asks it, and this test is where
    /// the two tellings must agree.
    fn answer_about(sand: &Sandbox, name: &str, spelling: &str) -> std::result::Result<bool, io::Error> {
        let put_up = sand.folders.cache.join(name);
        fs::write(&put_up, b"x")?;
        let folded = fs::symlink_metadata(sand.folders.cache.join(spelling)).is_ok();
        fs::remove_file(&put_up)?;
        Ok(folded)
    }

    /// The rules are read from the volume and not from the platform: what the volume does with a
    /// name and a spelling of it matches the rule read from it, on whatever machine runs the test —
    /// a Mac, whose internal disks fold case and form, or a Linux container that keeps both apart.
    #[test]
    fn the_volume_is_read_by_what_it_answers_and_not_by_its_platform() -> Test {
        let sand = Sandbox::new()?;
        let VolumeRules { case_sensitive, form_sensitive } = volume_rules(&sand);
        let folds_case = answer_about(&sand, "Probe.Case", "PROBE.CASE")?;
        assert_eq!(folds_case, !case_sensitive, "the case rule read is not what this volume does with names");
        let folds_form = answer_about(&sand, "\u{e1}lbum-probe", "a\u{301}lbum-probe")?;
        assert_eq!(folds_form, !form_sensitive, "the form rule read is not what this volume does with names");
        Ok(())
    }

    /// The take-over reaches across spellings: the journal holds an entry an earlier export left of
    /// a file, and a later export lists that same file spelt another way, with nothing there when it
    /// lists. The volume says one file, so the listing takes the left entry over — the record of
    /// what was there comes along, and the older entry is gone — and when the newer export writes
    /// and finishes, nothing of the older entry deletes its file, at the end or at a restart. A
    /// journal written before spellings were told apart may hold a later entry of the file too,
    /// spelt as the listing spells it: the listing takes every entry of the file over, and the
    /// record it keeps is the oldest's, the nearest to what was there before any export wrote it.
    #[test]
    fn a_later_export_of_one_file_spelt_another_way_takes_the_left_entry_over() -> Test {
        let sand = Sandbox::new()?;
        let Some((left, listing)) = folded_or_skip(&sand)? else {
            return Ok(());
        };
        sand.write_journal(&[json!({ PATH: left, BYTES: 18, MODIFIED: 1234, EXPORT: 9 }), json!({ PATH: listing, BYTES: 4, MODIFIED: 99, EXPORT: 10 })]);
        let file = sand.folders.container.join(&listing);
        listed_as_one_entry(&sand, &file, &listing, 18, 1234)?;
        sand.put(&listing, b"the finished movie");
        let engine = sand.start()?;
        assert_eq!(
            engine.recovery(),
            &Recovery::default(),
            "a restart deleted a finished export's file for an older entry of its other spelling: {:?}",
            engine.recovery()
        );
        assert_eq!(fs::read(&file)?, b"the finished movie", "the finished file did not survive the older entry");
        assert!(!sand.file().exists(), "the older entry outlived the export that took it over");
        Ok(())
    }

    /// Two killed exports whose files needed one folder, each spelt another way, leave that folder
    /// listed twice: a start deals with it once and reports it once, because the volume says one
    /// folder however the entries spelt it, whichever way the two spellings sorted.
    #[test]
    fn two_spellings_of_one_folder_a_killed_export_made_are_dealt_with_and_reported_once() -> Test {
        let sand = Sandbox::new()?;
        let Some((name, other)) = folded_or_skip(&sand)? else {
            return Ok(());
        };
        let container = &sand.folders.container;
        let (mut one, mut two) = (sand.journal().pending(), sand.journal().pending());
        one.list(&[container.join(format!("{name}/one.png"))])?;
        two.list(&[container.join(format!("{other}/two.png"))])?;
        sand.put(&format!("{name}/one.png"), b"half");
        sand.put(&format!("{name}/notes.txt"), b"not the export's");
        one.killed();
        two.killed();

        let engine = sand.start()?;
        let recovery = engine.recovery();
        assert_eq!(recovery.removed, [sand.real(&format!("{name}/one.png"))], "{recovery:?}");
        assert_eq!(recovery.untouched.len(), 1, "one folder spelt two ways was dealt with twice: {recovery:?}");
        if let Some(kept) = recovery.untouched.first() {
            assert_eq!(fs::canonicalize(kept)?, fs::canonicalize(sand.real(&name))?, "the entry left standing does not name the folder: {recovery:?}");
        }
        assert!(!sand.file().exists());
        Ok(())
    }

    /// A folder a running export writes into is left by a start however a killed export's entry
    /// spelt its path: the claim that a running export's file lies in a folder entry's folder is
    /// asked through the volume, not the spelling the older entry carries. The killed export listed
    /// its file and the folder it was to make, and the kill came before it wrote, so the running
    /// export lists with nothing there and keeps its own spelling, which a folder already there would
    /// have had the listing resolve to the folder's own.
    #[test]
    fn a_start_leaves_a_folder_a_running_export_needs_however_a_killed_export_spelt_it() -> Test {
        let sand = Sandbox::new()?;
        let Some((name, other)) = folded_or_skip(&sand)? else {
            return Ok(());
        };
        let container = &sand.folders.container;
        sand.write_journal_with(&[entry(&format!("{name}/one.png"))], &[entry(&name)]);
        let running_file = format!("{other}/two.png");
        let mut running = sand.journal().pending();
        running.list(&[container.join(&running_file)])?;
        sand.put(&running_file, b"half of a newer movie");

        let engine = sand.start()?;
        assert_eq!(
            engine.recovery(),
            &Recovery::default(),
            "a start dealt with the folder a running export writes into, spelt another way by the killed entry: {:?}",
            engine.recovery()
        );
        assert_eq!(fs::read(container.join(&running_file))?, b"half of a newer movie");
        running.remove()?;
        assert!(!container.join(&other).exists(), "the running export's own cancel did not remove its file and the folder it made");
        assert!(!sand.file().exists(), "the journal outlived the running export");
        Ok(())
    }

    /// A file a running export writes is left by a start however a journal set aside spelt it: the
    /// set-aside entry is the running export's file, which it claims as the volume tells files, so
    /// the start neither deletes the file nor keeps the entry.
    #[test]
    fn a_start_leaves_a_running_exports_file_however_a_set_aside_journal_spelt_it() -> Test {
        let sand = Sandbox::new()?;
        let Some((name, other)) = folded_or_skip(&sand)? else {
            return Ok(());
        };
        fs::write(sand.folders.config.join(format!("{SHELF_PREFIX}1.json")), json!({ OUTPUTS: [entry(&name)] }).to_string())?;
        let file = sand.folders.container.join(&other);
        let mut running = sand.journal().pending();
        running.list(std::slice::from_ref(&file))?;
        sand.put(&other, b"half of a newer movie");

        let engine = sand.start()?;
        let recovery = engine.recovery();
        assert!(recovery.removed.is_empty() && recovery.untouched.is_empty(), "a start dealt with a running export's file: {recovery:?}");
        assert_eq!(fs::read(&file)?, b"half of a newer movie", "a start deleted a running export's file for a set-aside entry of its other spelling");
        assert!(shelved(&sand)?.is_empty(), "the set-aside journal outlived the start that dealt with it");
        running.clear()?;
        assert_eq!(fs::read(&file)?, b"half of a newer movie");
        assert!(!sand.file().exists(), "the journal outlived the running export");
        Ok(())
    }

    /// A journal set aside lists a file that an export ended at while it waited — and the record
    /// that the export ended there spells the file another way. The volume says one file, so the
    /// set-aside entry yields to the record exactly as it would to a record spelt its own way, and
    /// the finished file the export left stands.
    #[test]
    fn a_set_aside_entry_of_a_file_taken_under_another_spelling_yields_too() -> Test {
        let sand = Sandbox::new()?;
        let Some((name, taken_as)) = folded_or_skip(&sand)? else {
            return Ok(());
        };
        sand.put(&name, b"the finished movie");
        fs::write(sand.folders.config.join(format!("{SHELF_PREFIX}1.json")), json!({ OUTPUTS: [entry(&name)], TAKEN: [{ PATH: taken_as }] }).to_string())?;

        let engine = sand.start()?;
        let recovery = engine.recovery();
        assert!(
            recovery.removed.is_empty() && recovery.untouched.is_empty(),
            "a set-aside entry of a file taken under another spelling was dealt with anyway: {recovery:?}"
        );
        assert!(recovery.journal.iter().any(|line| line.contains("yield")), "the start did not say the entry yielded: {recovery:?}");
        assert_eq!(
            fs::read(sand.folders.container.join(&name))?,
            b"the finished movie",
            "the file an export ended at was deleted by a set-aside entry of its other spelling"
        );
        assert!(!sand.file().exists(), "the journal outlived the entries that yielded: {:?}", engine.recovery());
        Ok(())
    }

    /// Where the volume keeps two spellings apart they are two files, and two exports have them:
    /// both listings run — no refusal — and every end removes its own file alone, a restart included.
    /// A volume that folds every spelling has nothing to tell apart here and says itself skipped;
    /// its proof is the tests above.
    #[test]
    fn two_spellings_a_volume_keeps_apart_are_two_files_in_the_journal_too() -> Test {
        let sand = Sandbox::new()?;
        let (one_name, other_name) = match one_spelling_kept_apart(&sand) {
            Some(pair) => pair,
            None => {
                skipped("this volume folds every spelling of a name together")?;
                return Ok(());
            }
        };
        let (mine, theirs) = (sand.folders.container.join(&one_name), sand.folders.container.join(&other_name));
        sand.put(&one_name, b"the one movie");
        sand.put(&other_name, b"the other movie");
        assert_ne!(fs::canonicalize(&mine)?, fs::canonicalize(&theirs)?, "the volume folded the spellings the read says it keeps apart");
        let (mut one, mut two) = (sand.journal().pending(), sand.journal().pending());
        one.list(std::slice::from_ref(&mine))?;
        two.list(std::slice::from_ref(&theirs))?;
        sand.put(&one_name, b"the newer movie");
        sand.put(&other_name, b"the newer other");
        one.remove()?;
        assert!(!mine.exists(), "a cancel did not remove its own file");
        assert_eq!(fs::read(&theirs)?, b"the newer other", "a cancel removed the other export's file");
        let engine = sand.start()?;
        assert_eq!(engine.recovery(), &Recovery::default(), "a start dealt with a running export's file: {:?}", engine.recovery());
        assert_eq!(fs::read(&theirs)?, b"the newer other", "a start removed the running export's file");
        two.remove()?;
        assert!(!theirs.exists(), "a cancel did not remove its own file after a start");
        assert!(!sand.file().exists(), "the journal outlived both exports");
        Ok(())
    }

    /// The volume is asked in the journal's folder and the asking leaves nothing there, and what an
    /// ask a kill stopped midway put up there goes at the next start, as a killed write's file does.
    #[test]
    fn asking_the_volume_leaves_nothing_in_the_journals_folder() -> Test {
        let sand = Sandbox::new()?;
        assert_eq!(VolumeRules::ask(&sand.folders.config), volume_rules(&sand), "asking again answered other than this volume's rules");
        let names: Vec<std::ffi::OsString> = fs::read_dir(&sand.folders.config)?.map(|entry| entry.map(|e| e.file_name())).collect::<io::Result<_>>()?;
        assert!(names.is_empty(), "the asking left {names:?} in the journal's folder");
        let left = sand.folders.config.join(probe_name());
        fs::write(&left, b"")?;
        sand.start()?;
        assert!(!left.exists(), "a start left what an ask stopped midway put up");
        Ok(())
    }

    /// A folder the volume cannot be asked in, one this user may only read, is read as a volume that
    /// folds both, the reading that loses no work.
    #[cfg(unix)]
    #[test]
    fn a_volume_that_cannot_be_asked_is_read_as_one_that_folds_both() -> Test {
        let sand = Sandbox::new()?;
        if !permissions_hold(&sand)? {
            return Ok(());
        }
        set_mode(&sand.folders.cache, 0o500)?;
        let unasked = VolumeRules::ask(&sand.folders.cache);
        set_mode(&sand.folders.cache, 0o700)?;
        assert_eq!(unasked, VolumeRules::default(), "a folder that takes no probe was read as a volume that keeps names apart");
        Ok(())
    }

    /// The rules read from a volume fold the names on that volume alone, which its number tells:
    /// the names below a container on another volume, or one that cannot be looked at, fold by the
    /// reading that loses no work however the volume read keeps names apart, and the names on the
    /// volume read fold by what it answered.
    #[cfg(unix)]
    #[test]
    fn the_rules_read_from_a_volume_fold_the_names_on_that_volume_alone() -> Test {
        use std::os::unix::fs::MetadataExt;
        let sand = Sandbox::new()?;
        let root = sand.journal().root;
        let device = fs::metadata(&root)?.dev();
        let keeps_apart = Some(VolumeRules { case_sensitive: true, form_sensitive: true });
        let one = |paths: Paths| one_file(&paths.id("Card.MP4"), &paths.id("CARD.MP4"));
        assert!(one(Paths::with(&root, keeps_apart.map(|rules| (device ^ 1, rules)))), "another volume's rules folded the container's names");
        assert!(
            one(Paths::with(&root.join("gone"), keeps_apart.map(|rules| (NO_FOLDER.0, rules)))),
            "the rules read folded the names below a container that could not be looked at"
        );
        assert!(!one(Paths::with(&root, keeps_apart.map(|rules| (device, rules)))), "the rules read did not fold the names of the volume they were read from");
        Ok(())
    }

    /// A path is told as the volume reaches it: a link that stands for a folder of the path is the
    /// folder it reaches, so a file in that folder is one file however the path reaches it, there or
    /// not yet; while a link at the path's own name is the link, never what it points at, as a start
    /// deals with a link there.
    #[cfg(unix)]
    #[test]
    fn a_path_through_a_linked_folder_is_told_as_the_folder_the_link_reaches() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        fs::create_dir(container.join("real"))?;
        std::os::unix::fs::symlink(container.join("real"), container.join("linked"))?;
        sand.put("real/finished.mp4", b"finished");
        std::os::unix::fs::symlink(container.join("real/finished.mp4"), container.join("pointer.mp4"))?;
        let paths = sand.journal().paths();
        assert!(one_file(&paths.id("linked/new.mp4"), &paths.id("real/new.mp4")), "a file not there yet was told by the link its folder was reached through");
        assert!(one_file(&paths.id("linked/finished.mp4"), &paths.id("real/finished.mp4")), "a file there was told by the link its folder was reached through");
        assert!(!one_file(&paths.id("pointer.mp4"), &paths.id("real/finished.mp4")), "a link at a path's own name was told as what it points at");
        Ok(())
    }

    /// A writer puts a file up between one operation's look at a path in one folder and its look at
    /// the same path through another spelling of that folder: one file still, because the identity
    /// told to the second look — the file's numbers with the folder and folded name beside them —
    /// names one file with the dwelling the first look was told, whichever look came before the
    /// putting up. The Paths of one operation is that operation: its two tells are the two looks,
    /// and the write is what the writer does between them.
    #[test]
    fn a_file_put_up_between_one_operations_two_looks_is_one_file_through_a_folder_spelt_another_way() -> Test {
        let sand = Sandbox::new()?;
        if volume_rules(&sand).case_sensitive {
            skipped("this volume keeps folder names apart by case")?;
            return Ok(());
        }
        let container = &sand.folders.container;
        fs::create_dir(container.join("Movie"))?;
        let paths = sand.journal().paths();
        let before = paths.id("Movie/late.png");
        sand.put("MOVIE/late.png", b"half of the movie");
        assert!(one_file(&paths.id("MOVIE/late.png"), &before), "a file put up between one operation's two looks, its folder spelt apart, was told two files");
        Ok(())
    }

    /// The same between the looks of one operation as they reach the file one way before it is there
    /// and another after: the folder reached through a link and the folder itself are one folder, so
    /// the dwelling is one dwelling and the two tells one file, on a volume that keeps every
    /// spelling of a name apart as much as on one that folds them.
    #[cfg(unix)]
    #[test]
    fn a_file_put_up_between_one_operations_two_looks_is_one_file_reached_through_a_link() -> Test {
        let sand = Sandbox::new()?;
        sand.linked("real", "link")?;
        let paths = sand.journal().paths();
        let before = paths.id("link/movie.mp4");
        sand.put("real/movie.mp4", b"half of the movie");
        assert!(one_file(&paths.id("real/movie.mp4"), &before), "a file reached through a link, put up between one operation's two looks, was told two files");
        Ok(())
    }

    /// One file reached under two names, each in another folder, is one file while both names are
    /// there: the volume's numbers for it are one, and they name one file across dwellings that
    /// differ. A rule that told one file by where its name lies alone would tell the two names two
    /// files, run a second export beside the first, and leave one export's cancel to delete the
    /// other's file.
    #[test]
    fn a_file_under_two_names_in_two_folders_is_one_file_by_its_numbers() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        sand.put("first/movie.mp4", b"the movie");
        fs::create_dir(container.join("second"))?;
        fs::hard_link(container.join("first/movie.mp4"), container.join("second/other.mp4"))?;
        let paths = sand.journal().paths();
        assert!(
            one_file(&paths.id("first/movie.mp4"), &paths.id("second/other.mp4")),
            "one file under two names was told two files, though its numbers were one"
        );
        Ok(())
    }

    /// Places one movie under two names in two folders — `first/movie.mp4`, and a hard link
    /// naming it at `second/other.mp4` — and returns a running export listing it under the link's
    /// name. Both tests beside it start from this one file named two ways.
    #[cfg(unix)]
    fn movie_under_two_names(sand: &Sandbox) -> std::result::Result<Pending, Box<dyn std::error::Error>> {
        let container = &sand.folders.container;
        sand.put("first/movie.mp4", b"the movie");
        fs::create_dir(container.join("second"))?;
        fs::hard_link(container.join("first/movie.mp4"), container.join("second/other.mp4"))?;
        let mut holder = sand.journal().pending();
        holder.list(std::slice::from_ref(&container.join("second/other.mp4")))?;
        Ok(holder)
    }

    /// The refusal, with a running export's file reached by another name in another folder: its
    /// numbers are told under both names, so the listing gathers the running entry whatever name
    /// an export reached the file by, and no second export of it gets in under another name. The
    /// refused listing enters nothing of its own.
    #[cfg(unix)]
    #[test]
    fn a_second_export_of_a_running_exports_file_is_refused_under_another_name_of_it() -> Test {
        let sand = Sandbox::new()?;
        let holder = movie_under_two_names(&sand)?;
        let mut second = sand.journal().pending();
        assert_eq!(
            second.list(std::slice::from_ref(&sand.folders.container.join("first/movie.mp4"))),
            Err(sand.held_by_another("first/movie.mp4")),
            "a second export listed, under its other name, the file a running export holds"
        );
        let written = fs::read_to_string(sand.file())?;
        assert!(written.contains("second/other.mp4") && !written.contains("first/movie.mp4"), "the refused listing entered the journal: {written}");
        holder.clear()?;
        Ok(())
    }

    /// One file lies under two names in two folders; a running export lists it by one, and then
    /// another file is put at the other name in its place. The two names now tell two files, and
    /// a second export may list the replaced one: a comparison asks what the two tellings share
    /// now, and binds nothing through an older telling that matches neither — so two files are two
    /// exports, and neither listing reaches across to the other's file.
    #[cfg(unix)]
    #[test]
    fn the_file_put_in_the_place_of_a_running_exports_other_name_is_another_file() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        let holder = movie_under_two_names(&sand)?;
        fs::remove_file(container.join("first/movie.mp4"))?;
        let replaced = sand.put("first/movie.mp4", b"a movie that took its place");
        let mut second = sand.journal().pending();
        second.list(std::slice::from_ref(&container.join("first/movie.mp4")))?;
        assert_eq!(fs::read(&replaced)?, b"a movie that took its place", "the export of the replaced name lost the file there");
        assert_eq!(fs::read(container.join("second/other.mp4"))?, b"the movie", "a listing of one file reached across to the file of another");
        second.clear()?;
        holder.clear()?;
        assert!(!sand.file().exists(), "an export's end cleared more than it listed");
        Ok(())
    }

    /// Two files of one name under two folders are two files: the name folds to one, but each lies
    /// in another folder and carries its own numbers, so no comparison may take one for the other —
    /// neither a refusal of a second export of the one a running export holds, nor a claim,
    /// take-over or yielding across the two.
    #[test]
    fn two_files_of_one_name_under_two_folders_are_two_files() -> Test {
        let sand = Sandbox::new()?;
        sand.put("first/movie.mp4", b"the movie");
        sand.put("second/movie.mp4", b"the other movie");
        let paths = sand.journal().paths();
        assert!(!one_file(&paths.id("first/movie.mp4"), &paths.id("second/movie.mp4")), "two files of one name under two folders were told one file");
        Ok(())
    }

    /// A path is told once for an operation, however many of its rules ask: the file put at it in
    /// place of the one first told is the one that path stands for until the operation ends, so a
    /// path's two askings never tell two files.
    #[test]
    fn a_path_is_told_once_for_an_operation() -> Test {
        let sand = Sandbox::new()?;
        let path = sand.put("out/card.mp4", b"the first movie");
        let paths = sand.journal().paths();
        let first = paths.id("out/card.mp4");
        fs::remove_file(&path)?;
        let _kept = sand.put("out/kept.mp4", b"put first, so the movie put next is not given the first one's freed number");
        sand.put("out/card.mp4", b"another movie in its place");
        assert_eq!(paths.id("out/card.mp4").numbers, first.numbers, "one path was asked of the disk twice in one operation");
        assert_ne!(sand.journal().paths().id("out/card.mp4").numbers, first.numbers, "the next operation did not ask for itself");
        Ok(())
    }

    /// The files of a running export of a sequence, below `folder`: `frames` frames and `late.png`,
    /// each listed by the export numbered 1 and none written yet.
    fn running_sequence(folder: &str, frames: usize) -> Vec<Entry> {
        (0..frames)
            .map(|frame| format!("{folder}/f_{frame:05}.png"))
            .chain(iter::once(format!("{folder}/late.png")))
            .map(|path| Entry { path, kind: Kind::File(None), export: Some(1) })
            .collect()
    }

    /// Three tellings of the kind the journal's comparisons meet, built on the disk: A, a file by its
    /// path; B, another file put at that path in its place, told through another way to its folder;
    /// C, the first file by a second name in another folder. A is one file with B, by where they lie,
    /// and with C, by its numbers, while B and C are two, so no order can keep the three. Every way the
    /// journal gathers tellings — what entries claim, the take-over of held entries, the files a record
    /// yields to, and the gathering the refusal and the folders' one-of-each use — finds each telling it
    /// is one file with, among many others and in whatever order they came, and none of the rest.
    #[cfg(unix)]
    #[test]
    fn every_gathering_finds_each_telling_it_is_one_file_with_among_many_in_any_order() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        sand.put("a/x.png", b"the first file");
        fs::create_dir(container.join("b"))?;
        fs::hard_link(container.join("a/x.png"), container.join("b/q.png"))?;
        sand.linked("a", "alias")?;
        let paths = sand.journal().paths();
        let (a, c) = (paths.id("a/x.png"), paths.id("b/q.png"));
        fs::remove_file(container.join("a/x.png"))?;
        sand.put("a/x.png", b"another file in its place");
        let b = paths.id("alias/x.png");
        assert!(a.numbers.is_some() && b.numbers.is_some() && a.numbers != b.numbers && a.numbers == c.numbers, "{a:?} {b:?} {c:?}");
        assert!(one_file(&a, &b) && one_file(&b, &a) && one_file(&a, &c) && one_file(&c, &a), "{a:?} {b:?} {c:?}");
        assert!(!one_file(&b, &c) && !one_file(&c, &b), "{b:?} {c:?}");
        // Many others, in both folders, around them in every order a gathering may meet them.
        let others: Vec<PathId> = running_sequence("a", 32).iter().chain(&running_sequence("b", 32)).map(|entry| paths.id(&entry.path)).collect();
        let named = [("A", &a), ("B", &b), ("C", &c)];
        for gathered in [vec![0], vec![1], vec![2], vec![1, 2], vec![2, 1], vec![0, 1, 2], vec![2, 1, 0]] {
            for others_first in [true, false] {
                let mut tellings = Tellings::default();
                let mut add = |id: &PathId| tellings.add(id, (), |_, _| {});
                if others_first {
                    others.iter().for_each(&mut add);
                }
                gathered.iter().filter_map(|&at| named.get(at)).for_each(|(_, id)| add(id));
                if !others_first {
                    others.iter().for_each(&mut add);
                }
                for (probe, id) in named {
                    let due = gathered.iter().filter_map(|&at| named.get(at)).any(|(_, held)| one_file(held, id));
                    assert_eq!(tellings.holds(id), due, "{probe} among {gathered:?}, the others {}", if others_first { "first" } else { "last" });
                }
            }
        }
        // The journal's own gatherings, over the paths the tellings were told by.
        let entry = |path: &str| Entry { path: path.to_string(), kind: Kind::File(None), export: None };
        let mut running = running_sequence("b", 32);
        running.push(entry("b/q.png"));
        let claims = Claims::of(&paths, &running);
        assert!(claims.by(&paths, &entry("a/x.png")) && !claims.by(&paths, &entry("alias/x.png")), "what entries claim");
        let taken = Taken { files: ["b/q.png".to_string()].into(), every: false };
        let yields = taken.yields(&paths);
        assert!(yields("a/x.png") && !yields("alias/x.png"), "the files a record yields to");
        let held =
            vec![entry("b/f_00000.png"), Entry { kind: Kind::File(Some(Stamp { bytes: 3, modified: Some(1) })), ..entry("b/q.png") }, entry("alias/x.png")];
        let mut listing = vec![entry("a/x.png")];
        let still = taken_over(&held, &mut listing, &paths);
        assert_eq!(listing.first().map(|listed| listed.kind), held.get(1).map(|older| older.kind), "the oldest held entry of the file listed again");
        assert_eq!(still, vec![entry("b/f_00000.png")], "the held entries the listing took over");
        Ok(())
    }

    /// A start, or a cancel, tells the paths a running export claims before it weighs the stale
    /// entries, and a running export's writer puts a file up between the two: the stale entry of
    /// that file, spelt through another way to its folder, is still the running export's, however
    /// many files that export lists, and the file it is writing is left. Told by one order of whole
    /// tellings, a gathering of more than one claim sorted the late telling past the one it is one
    /// file with, and the start deleted the file being written.
    #[cfg(unix)]
    #[test]
    fn a_file_put_up_while_a_start_tells_its_paths_is_left_to_the_export_writing_it() -> Test {
        for frames in [0, 1, 64] {
            let sand = Sandbox::new()?;
            sand.linked("real", "link")?;
            let running = running_sequence("real", frames);
            let paths = sand.journal().paths();
            // The start's first look: what the running export claims, before its writer has put a frame up.
            assert!(Claims::of(&paths, &running).by(&paths, &running[running.len() - 1]));
            let late = sand.put("real/late.png", b"half of the frame");
            let stale = vec![Entry { path: "link/late.png".to_string(), kind: Kind::File(None), export: None }];
            let mut recovery = Recovery::default();
            let left = sand.journal().deal_with_all(&paths, stale, &running, &mut recovery);
            assert!(left.is_empty() && recovery.removed.is_empty(), "{frames} frames: {recovery:?}");
            assert_eq!(fs::read(&late)?, b"half of the frame", "{frames} frames: the start deleted the file a running export is writing");
        }
        Ok(())
    }

    /// The listing's refusal, with a running export's file put up between its look at the running
    /// export's entries and its look at its own: the second export of that file is refused, spelt
    /// through another way to its folder. Each listing records its files at their landings, their
    /// folders resolved, so the two paths are one spelling and one telling, which the putting up
    /// between the looks cannot split.
    #[cfg(unix)]
    #[test]
    fn a_file_put_up_between_the_listings_two_looks_is_refused_to_a_second_export() -> Test {
        let sand = Sandbox::new()?;
        let container = &sand.folders.container;
        sand.linked("real", "link")?;
        let journal = sand.journal();
        let mut holder = journal.pending();
        let frames: Vec<PathBuf> = running_sequence("real", 64).iter().map(|entry| container.join(&entry.path)).collect();
        holder.list(&frames)?;
        let paths = journal.paths();
        // The listing's look at the running export's entries, which its writer has not reached yet.
        for listed in &holder.listed {
            paths.id(&listed.path);
        }
        sand.put("real/late.png", b"half of the frame");
        let mut second = journal.pending();
        let refused = {
            let mut known = lock(&KNOWN);
            let state = journal.hold(&mut known)?;
            second.list_held(state, &paths, &[container.join("link/late.png")])
        };
        assert_eq!(refused, Err(sand.held_by_another("real/late.png")));
        holder.clear()?;
        Ok(())
    }

    /// A file under a folder that one look found nowhere, and the writer made before the next look,
    /// is one file for both looks, its folder spelt another way: the first look tells where the name
    /// lies from the container, the folder above it that is there, and the second from the container
    /// and from the folder made since, so the container's telling is the one they share.
    #[test]
    fn a_file_under_a_folder_made_between_one_operations_two_looks_is_one_file() -> Test {
        let sand = Sandbox::new()?;
        if volume_rules(&sand).case_sensitive {
            skipped("this volume keeps folder names apart by case")?;
            return Ok(());
        }
        let paths = sand.journal().paths();
        let before = paths.id("New/late.png");
        sand.put("NEW/late.png", b"half of the movie");
        let after = paths.id("NEW/late.png");
        assert!(before.numbers.is_none() && after.numbers.is_some(), "{before:?} {after:?}");
        assert!(one_file(&before, &after), "a file under a folder made between one operation's two looks was told two files: {before:?} {after:?}");
        Ok(())
    }
}
