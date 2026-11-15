//! The binary as a script runs it, its output a pipe whose reader has gone (`| head -1`, a caller that
//! stopped reading): every outcome is still an exit code the caller can read, never a panic.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

/// A surface of one command and one tool an app can call, and the same surface with the command gone: the
/// second is breaking, so only a report someone can read earns it the exit code that says so.
const ONE_COMMAND: &str = concat!(
    r#"{"app":"effectcraft","engineVersion":"1","pin":"0123abcd","commands":[{"id":"a.b","label":"A","params":"{k}","journal":true,"#,
    r#""keys":[{"name":"k","required":false,"type":"bool"}]}],"tools":[{"name":"undo","inputSchema":{}}]}"#,
);
const ONE_COMMAND_LESS: &str = r#"{"app":"effectcraft","engineVersion":"1","pin":"0123abcd","commands":[],"tools":[{"name":"undo","inputSchema":{}}]}"#;

/// The exit code of the binary run with `args`, its standard output and standard error going where those
/// two say.
fn exit_with(args: &[&OsStr], stdout: Stdio, stderr: Stdio) -> std::io::Result<Option<i32>> {
    let status = Command::new(env!("CARGO_BIN_EXE_craftbridge-effects-codegen")).args(args).stdin(Stdio::null()).stdout(stdout).stderr(stderr).status()?;
    Ok(status.code())
}

/// The same, with both streams a pipe that no one reads any more.
fn exit_unread(args: &[&OsStr]) -> std::io::Result<Option<i32>> {
    let (reader, writer) = std::io::pipe()?;
    drop(reader);
    exit_with(args, Stdio::from(writer.try_clone()?), Stdio::from(writer))
}

#[test]
fn output_no_one_reads_ends_in_an_exit_code_not_a_panic() -> std::io::Result<()> {
    let arg = OsStr::new;
    let surface = Path::new(env!("CARGO_MANIFEST_DIR")).join("generated").join("api-surface.json");
    let out = std::env::temp_dir().join(format!("craftbridge-codegen-unread-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let missing = out.join("missing.json");
    let (was, gone) = (out.join("was.json"), out.join("gone.json"));
    std::fs::create_dir_all(&out)?;
    std::fs::write(&was, ONE_COMMAND)?;
    std::fs::write(&gone, ONE_COMMAND_LESS)?;
    let (surface, missing, folder) = (surface.as_os_str(), missing.as_os_str(), out.as_os_str());
    let (was, gone) = (was.as_os_str(), gone.as_os_str());
    assert_eq!(exit_unread(&[arg("diff"), surface, surface])?, Some(1), "a report no one can read is a diff that failed");
    // A breaking change is exit 2 because the report named it; with no one to read the report the run
    // simply failed, so 1, which is what keeps 2 meaning a breaking change and nothing else.
    assert_eq!(exit_unread(&[arg("diff"), was, gone])?, Some(1), "a breaking change no one can read about is a failed run");
    assert_eq!(exit_with(&[arg("diff"), was, gone], Stdio::null(), Stdio::null())?, Some(2), "the same two files, read by someone, are a breaking change");
    assert_eq!(exit_unread(&[arg("diff"), surface, missing])?, Some(1), "a file that cannot be read");
    assert_eq!(exit_unread(&[arg("no-such-command")])?, Some(1), "a command line that is no command");
    assert_eq!(exit_unread(&[arg("surface"), arg("--pin"), arg("0123abcd"), folder])?, Some(0), "the surface is written; only the word saying so is lost");
    assert!(out.join("api-surface.json").is_file());
    let _ = std::fs::remove_dir_all(&out);
    Ok(())
}
