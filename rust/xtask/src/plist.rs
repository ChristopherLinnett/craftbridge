//! Apple's property lists, read through `plutil`, macOS's own reader of every form they are written in —
//! the XML and binary ones a bundle's `Info.plist` takes and the old text one an Xcode project is written in
//! — as the JSON it converts them to, so no check reads the format itself.

use std::path::Path;
use std::process::Command;

/// The property list `file`, as `plutil -convert json` gives it. A file plutil cannot read, or a machine with
/// no plutil, is an error naming the file.
pub(crate) fn read_as_json(file: &Path) -> Result<serde_json::Value, String> {
    let output = Command::new("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(file)
        .output()
        .map_err(|error| format!("cannot run plutil on {}: {error}", file.display()))?;
    if !output.status.success() {
        return Err(format!("plutil cannot read {}: {}", file.display(), String::from_utf8_lossy(&output.stderr).trim()));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("{}: plutil answered JSON that does not parse: {error}", file.display()))
}
