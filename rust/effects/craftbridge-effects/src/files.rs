//! The `open`, `save`, `saveAs` and `new` methods of the plan's §14 table: the project file commands
//! the command line's `--project`, `--save`, `--save-as` and `--empty` run, answered as the commands
//! answer them.
//!
//! Each goes through the one door every command goes through, so a path outside the confinement's
//! roots is refused by the confinement's own check and by nothing of this crate's, and the engine
//! alone says what a missing `path` is.

use craftbridge_core::Result;
use effectcraft_automation::Backend;
use serde_json::Value;

use crate::methods::{empty_params, execute, object, object_params, reject_unknown, string_param};

/// `open`: the project at `path`, which replaces the session's.
pub(crate) fn open(backend: &mut Backend, params: &Value) -> Result<Value> {
    file_command(backend, params, "open", "file.open", &["path"])
}

/// `save`: the project to the file it was opened from or last saved to, or to `path`, which is the
/// file it saves to from then on, as after `saveAs`.
pub(crate) fn save(backend: &mut Backend, params: &Value) -> Result<Value> {
    file_command(backend, params, "save", "file.save", &["path"])
}

/// `saveAs`: the project to `path`, which becomes the file it saves to from then on.
pub(crate) fn save_as(backend: &mut Backend, params: &Value) -> Result<Value> {
    file_command(backend, params, "saveAs", "file.saveAs", &["path"])
}

/// `new`: an empty project in place of the session's, or the project template the settings name.
pub(crate) fn new(backend: &mut Backend, params: &Value) -> Result<Value> {
    file_command(backend, params, "new", "file.newProject", &[])
}

/// One method answered by one file command: the parameters the method takes, each handed over as it
/// came, and the command's own result.
fn file_command(backend: &mut Backend, params: &Value, method: &str, command: &str, known: &[&str]) -> Result<Value> {
    let obj = object_params(params, method)?;
    reject_unknown(&obj, method, known)?;
    let arguments = string_param(&obj, "path", method)?.map_or_else(empty_params, |path| object([("path", Value::from(path))]));
    execute(backend, None, command, arguments, None)
}
