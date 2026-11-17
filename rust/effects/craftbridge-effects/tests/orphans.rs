//! The engine start's sweep of the temporary files a killed write left: a new start takes them back
//! out of the engine's folders and touches nothing else (AD21).
//!
//! Every test here plants files the way a kill between an EffectCraft write's two moves leaves
//! them — its temporary file beside the file it was renaming over, in some folder of the engine's —
//! starts an engine over them the way an app does, and checks what is gone and what is kept.

mod scratch;

use scratch::Scratch;
use std::fs;
use std::path::Path;
use std::process::Command;

/// The name EffectCraft gives the temporary file of a write: the writing process's id and the
/// write's own number, spelled between the two halves of the name (AD21).
fn temporary_name(process: u32, number: u64) -> String {
    format!(".effectcraft-{process}-{number}.tmp")
}

/// The id of a process that has ended, as a killed write's writer has: one started and reaped here, so
/// no process has its id now.
fn a_process_that_has_ended() -> std::io::Result<u32> {
    let mut ended = Command::new("true").spawn()?;
    let id = ended.id();
    ended.wait()?;
    Ok(id)
}

/// The working folders of the scratch container, from the core's one list of them, which the sweep
/// reads too: a folder added to that list is planted in here with no second list.
fn working(scratch: &Scratch) -> impl Iterator<Item = &Path> {
    scratch.folders.working().into_iter().map(|(_, folder)| folder)
}

/// The acceptance of the issue: a temporary file a previous process's killed write left in each of
/// the app's folders is gone after a new engine starts, and a file of another name beside each is
/// kept.
#[test]
fn a_temporary_a_previous_process_left_in_the_app_s_folders_is_gone_after_a_start() {
    let scratch = Scratch::new().expect("a scratch folder");
    let previous = a_process_that_has_ended().expect("a process that has ended");
    for folder in working(&scratch) {
        fs::write(folder.join(temporary_name(previous, 0)), b"half a write").expect("a temporary a killed write left");
        fs::write(folder.join("finished.txt"), b"a finished file").expect("a file of another name");
    }

    let _app = scratch.app().expect("an engine starts over them");

    for folder in working(&scratch) {
        assert!(!folder.join(temporary_name(previous, 0)).exists(), "the start left a temporary file of a previous process behind");
        assert!(folder.join("finished.txt").exists(), "the start deleted a file of another name");
    }
}

/// A killed write's temporary sits beside the file it was writing, which may be deep inside an
/// engine folder — an import's media, say — so the sweep goes into the subfolders.
#[test]
fn a_temporary_deep_inside_an_engine_folder_is_gone_after_a_start() {
    let scratch = Scratch::new().expect("a scratch folder");
    let deep = scratch.folders.cache.join("staged").join("one card").join("media");
    let previous = a_process_that_has_ended().expect("a process that has ended");
    fs::create_dir_all(&deep).expect("a folder deep inside the cache");
    fs::write(deep.join("card.png"), b"a finished card").expect("a finished file");
    fs::write(deep.join(temporary_name(previous, 7)), b"half a card").expect("a temporary a killed write left deep");

    let _app = scratch.app().expect("an engine starts over them");

    assert!(!deep.join(temporary_name(previous, 7)).exists(), "the start left a temporary file deep in an engine folder behind");
    assert!(deep.join("card.png").exists(), "the start deleted a finished file deep in an engine folder");
}

/// Nothing outside the app's folders is touched: a temporary name outside the container, and one in
/// the container beside the engine's own folders, which no folder of the core's list holds, both stay
/// where they were.
#[test]
fn a_temporary_outside_the_engine_s_folders_is_kept() {
    let scratch = Scratch::new().expect("a scratch folder");
    let previous = a_process_that_has_ended().expect("a process that has ended");
    fs::write(scratch.root().join(temporary_name(previous, 0)), b"outside").expect("a temporary outside the container");
    fs::write(scratch.path().join(temporary_name(previous, 1)), b"beside the folders").expect("a temporary beside the engine's folders");

    let _app = scratch.app().expect("an engine starts over them");

    assert!(scratch.root().join(temporary_name(previous, 0)).exists(), "the start swept a file outside the container");
    assert!(scratch.path().join(temporary_name(previous, 1)).exists(), "the start swept the container root, which no engine folder is");
}

/// A temporary file named with this process's own id could be a write of this very engine's that is
/// still going, so the start keeps it.
#[test]
fn a_temporary_named_with_this_process_s_own_id_is_kept() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mine = temporary_name(std::process::id(), 0);
    fs::write(scratch.folders.config.join(&mine), b"a write still going").expect("a temporary of this process");

    let _app = scratch.app().expect("an engine starts beside it");

    assert!(scratch.folders.config.join(&mine).exists(), "the start deleted a temporary file of this very process");
}

/// Another process that is still running may be writing in the same folders — an app extension sharing
/// the container, or the last launch still being torn down — so a temporary named for it is a live
/// write's, and the start keeps it: for a process this one may signal (its parent, which the system
/// names as a running process however its first parent ended) and for one it may not (the system's
/// first process, which runs as long as the system does), as another app's process is on a phone.
#[test]
fn a_temporary_of_another_process_still_running_is_kept() {
    let scratch = Scratch::new().expect("a scratch folder");
    let running = [std::os::unix::process::parent_id(), 1];
    for folder in working(&scratch) {
        for (number, id) in (0_u64..).zip(running) {
            fs::write(folder.join(temporary_name(id, number)), b"a write still going").expect("a temporary of a process still running");
        }
    }

    let _app = scratch.app().expect("an engine starts beside them");

    for folder in working(&scratch) {
        for (number, id) in (0_u64..).zip(running) {
            assert!(folder.join(temporary_name(id, number)).exists(), "the start deleted the temporary of process {id}, which is still running");
        }
    }
}

/// Only EffectCraft's own naming rule is a leftover: a name that misses the rule at any one place —
/// no leading dot, another ending, either number not a number, either spelled with a leading zero,
/// either too big for the type EffectCraft prints — names no killed write's file and stays, and so
/// does one whose id the system cannot be asked about at all: `0`, which is no process, or one past
/// the range of a process id.
#[test]
fn a_name_short_of_effectcraft_s_own_spelling_is_kept() {
    let scratch = Scratch::new().expect("a scratch folder");
    let previous = a_process_that_has_ended().expect("a process that has ended");
    let misses = [
        format!("effectcraft-{previous}-0.tmp"),
        format!(".effectcraft-{previous}-0.bak"),
        format!(".effectcraft-{previous}.tmp"),
        ".effectcraft-ff-0.tmp".to_string(),
        format!(".effectcraft-{previous}-ff.tmp"),
        ".effectcraft-07-0.tmp".to_string(),
        format!(".effectcraft-{previous}-07.tmp"),
        ".effectcraft-4294967296-0.tmp".to_string(),
        format!(".effectcraft-{previous}-18446744073709551616.tmp"),
        ".effectcraft-0-0.tmp".to_string(),
        ".effectcraft-2147483648-0.tmp".to_string(),
    ];
    for name in &misses {
        fs::write(scratch.folders.cache.join(name), b"not a killed write's file").expect("a file of a near name");
    }

    let _app = scratch.app().expect("an engine starts beside them");

    for name in &misses {
        assert!(scratch.folders.cache.join(name).exists(), "`{name}` misses EffectCraft's spelling, and the start deleted it anyway");
    }
}

/// A folder of a leftover's name is not a leftover — the write's own create could not have made a
/// folder — so it stays with everything inside it.
#[test]
fn a_folder_of_a_leftover_s_name_is_kept() {
    let scratch = Scratch::new().expect("a scratch folder");
    let named = scratch.folders.cache.join(temporary_name(a_process_that_has_ended().expect("a process that has ended"), 2));
    fs::create_dir(&named).expect("a folder of a leftover's name");
    fs::write(named.join("inside.txt"), b"the folder's own file").expect("a file inside that folder");

    let _app = scratch.app().expect("an engine starts beside it");

    assert!(named.join("inside.txt").exists(), "the start deleted a folder of a leftover's name or what it held");
}

/// A symbolic link of a leftover's name is no leftover either: the create that makes the temporary
/// file could not open one, and the sweep follows nothing out of the folders — neither deletes the
/// link nor takes what it points at, wherever that is.
#[cfg(unix)]
#[test]
fn a_link_of_a_leftover_s_name_is_kept_and_nothing_through_a_link_is_touched() {
    use std::os::unix::fs::symlink;

    let scratch = Scratch::new().expect("a scratch folder");
    let previous = a_process_that_has_ended().expect("a process that has ended");
    let target = scratch.root().join("real-file");
    fs::write(&target, b"a file outside the folders").expect("a file outside the folders");
    symlink(&target, scratch.folders.cache.join(temporary_name(previous, 3))).expect("a link of a leftover's name");
    let hidden = scratch.root().join("hidden");
    fs::create_dir(&hidden).expect("a folder outside the folders");
    fs::write(hidden.join(temporary_name(previous, 4)), b"half a write out of reach").expect("a temporary outside the folders");
    symlink(&hidden, scratch.folders.cache.join("a-linked-folder")).expect("a folder linked out of the engine's folder");

    let _app = scratch.app().expect("an engine starts beside them");

    assert!(scratch.folders.cache.join(temporary_name(previous, 3)).exists(), "the start deleted a link of a leftover's name");
    assert!(target.exists(), "the start took a file a link named like a leftover pointed at");
    assert!(hidden.join(temporary_name(previous, 4)).exists(), "the start swept through a linked folder out of the engine's folders");
}
