//! What a start did about the files an export killed mid-way left behind.

use crate::error::CraftError;
use std::path::{Path, PathBuf};

/// What the engine's start did about the pending-export journal it found, as the core reports it
/// ([`craftbridge_core::Recovery`]) with every path as text: Swift and Kotlin hold paths as strings.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Recovery {
    /// Listed files the killed exports wrote, now deleted, then the folders they made for them, each
    /// removed once it held nothing.
    pub removed: Vec<String>,
    /// Listed files still as they were when listed: the export never reached them, so they are left;
    /// and folders an export made that hold something it did not write, which are left too.
    pub untouched: Vec<String>,
    /// Listed paths the start refused to touch, with why: one outside the container, a folder where a
    /// file was listed, anything but a folder where a folder was, or a link.
    pub refused: Vec<RecoveryNote>,
    /// Listed files and folders that could not be deleted, with why. They stay listed, so the next
    /// start tries again.
    pub failed: Vec<RecoveryNote>,
    /// What the start found about the journal files themselves.
    pub journal: Vec<String>,
}

/// One path the start did not delete, and why.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RecoveryNote {
    /// The path, as text.
    pub path: String,
    /// Why the start did not delete it.
    pub reason: String,
}

/// A path as text. A path that is not valid Unicode has no text that names the same file, and the app
/// acts on these paths, so it is an error rather than a lossy name for a file that is not there.
fn text(path: &Path) -> Result<String, CraftError> {
    path.to_str().map(str::to_owned).ok_or_else(|| CraftError::io(format!("a recovered path is not valid Unicode: {}", path.display())))
}

fn texts(paths: &[PathBuf]) -> Result<Vec<String>, CraftError> {
    paths.iter().map(|path| text(path)).collect()
}

fn notes(listed: &[(PathBuf, String)]) -> Result<Vec<RecoveryNote>, CraftError> {
    listed.iter().map(|(path, reason)| Ok(RecoveryNote { path: text(path)?, reason: reason.clone() })).collect()
}

impl TryFrom<&craftbridge_core::Recovery> for Recovery {
    type Error = CraftError;

    fn try_from(recovery: &craftbridge_core::Recovery) -> Result<Self, CraftError> {
        Ok(Self {
            removed: texts(&recovery.removed)?,
            untouched: texts(&recovery.untouched)?,
            refused: notes(&recovery.refused)?,
            failed: notes(&recovery.failed)?,
            journal: recovery.journal.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_recovery() -> craftbridge_core::Recovery {
        craftbridge_core::Recovery {
            removed: vec![PathBuf::from("/app/out/a.mp4")],
            untouched: vec![PathBuf::from("/app/out/b.mp4"), PathBuf::from("/app/out/c.mp4")],
            refused: vec![(PathBuf::from("/elsewhere/x"), "outside the container".to_string())],
            failed: vec![(PathBuf::from("/app/out/d.mp4"), "permission denied".to_string())],
            journal: vec!["set aside a journal it could not read".to_string()],
        }
    }

    #[test]
    fn every_part_of_the_cores_report_crosses_as_text() {
        let report = Recovery::try_from(&core_recovery()).expect("every path is Unicode");
        assert_eq!(
            report,
            Recovery {
                removed: vec!["/app/out/a.mp4".to_string()],
                untouched: vec!["/app/out/b.mp4".to_string(), "/app/out/c.mp4".to_string()],
                refused: vec![RecoveryNote { path: "/elsewhere/x".to_string(), reason: "outside the container".to_string() }],
                failed: vec![RecoveryNote { path: "/app/out/d.mp4".to_string(), reason: "permission denied".to_string() }],
                journal: vec!["set aside a journal it could not read".to_string()],
            }
        );
    }

    #[test]
    fn an_empty_report_is_empty_text() {
        let report = Recovery::try_from(&craftbridge_core::Recovery::default()).expect("nothing to convert");
        assert_eq!(report, Recovery { removed: vec![], untouched: vec![], refused: vec![], failed: vec![], journal: vec![] });
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_unicode_throws_io_in_whichever_list_holds_it() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let broken = PathBuf::from(OsString::from_vec(vec![b'/', 0xff, b'x']));
        for place in 0..4 {
            let mut core = core_recovery();
            match place {
                0 => core.removed.push(broken.clone()),
                1 => core.untouched.push(broken.clone()),
                2 => core.refused.push((broken.clone(), "why".to_string())),
                _ => core.failed.push((broken.clone(), "why".to_string())),
            }
            assert!(matches!(Recovery::try_from(&core), Err(CraftError::Io { .. })), "list {place}");
        }
    }
}
