//! Generates the EffectCraft API surface from the engine's command registry, the Swift and Kotlin
//! facades over it, and the diff between two surfaces. `cargo xtask generate` runs it.
//!
//! ```text
//! craftbridge-effects-codegen surface --pin <commit> <out-dir>
//! ```
//!
//! writes `api-surface.json` into `<out-dir>`.

mod surface;
mod types;

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    // `args_os`, not `args`: `args` panics on an argument that is not Unicode, and the output folder is a
    // path, which need not be.
    run(&std::env::args_os().skip(1).collect::<Vec<_>>())
}

/// Run the command line `args`, the program's name left out.
fn run(args: &[OsString]) -> ExitCode {
    match args {
        [command, flag, pin, out_dir] if command == "surface" && flag == "--pin" => report(
            pin.to_str()
                .ok_or_else(|| format!("`{}` is no pin: a commit id is hexadecimal digits", pin.to_string_lossy()))
                .and_then(|pin| write_surface(pin, Path::new(out_dir))),
        ),
        _ => {
            eprintln!("usage: craftbridge-effects-codegen surface --pin <commit> <out-dir>");
            ExitCode::from(2)
        }
    }
}

fn report(outcome: Result<String, String>) -> ExitCode {
    match outcome {
        Ok(done) => {
            println!("craftbridge-effects-codegen: {done}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("craftbridge-effects-codegen: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Build the surface at `pin` and write it into `out_dir`, saying what it holds.
fn write_surface(pin: &str, out_dir: &Path) -> Result<String, String> {
    let surface = surface::build(pin)?;
    let text = surface::render(&surface)?;
    std::fs::create_dir_all(out_dir).map_err(|error| format!("cannot create {}: {error}", out_dir.display()))?;
    let file = out_dir.join(surface::FILE);
    std::fs::write(&file, text).map_err(|error| format!("cannot write {}: {error}", file.display()))?;
    let counts = surface::type_counts(&surface).into_iter().map(|(name, count)| format!("{count} {name}")).collect::<Vec<_>>().join(", ");
    let commands = surface.get("commands").and_then(serde_json::Value::as_array).map_or(0, Vec::len);
    let tools = surface.get("tools").and_then(serde_json::Value::as_array).map_or(0, Vec::len);
    Ok(format!("{} holds {commands} commands and {tools} tools; keys typed {counts}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("craftbridge-codegen-test-{name}-{}", std::process::id()))
    }

    #[test]
    fn the_surface_is_written_where_asked() {
        let out = temp("written");
        let _ = std::fs::remove_dir_all(&out);
        let done = write_surface(surface::TEST_PIN, &out).unwrap();
        assert!(done.contains("commands") && done.contains("tools"), "{done}");
        let text = std::fs::read_to_string(out.join(surface::FILE)).unwrap();
        assert!(text.starts_with("{\n") && text.ends_with("}\n"));
        assert_eq!(text, surface::render(&surface::build(surface::TEST_PIN).unwrap()).unwrap());
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn a_pin_that_is_none_writes_nothing() {
        let out = temp("bad");
        assert!(write_surface("main", &out).is_err());
        assert!(!out.exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_command_line_is_read_whole_and_bytes_that_are_not_unicode_fail_without_a_crash() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let out = temp("line");
        let _ = std::fs::remove_dir_all(&out);
        let line = |args: &[&[u8]]| args.iter().map(|arg| OsString::from_vec(arg.to_vec())).collect::<Vec<_>>();
        let pin = surface::TEST_PIN.as_bytes();
        let folder = out.as_os_str().as_bytes();
        assert!(run(&line(&[b"surface", b"--pin", pin, folder])) == ExitCode::SUCCESS);
        assert!(out.join(surface::FILE).is_file());
        let _ = std::fs::remove_dir_all(&out);
        assert!(run(&line(&[b"surface", b"--pin", pin])) == ExitCode::from(2));
        assert!(run(&line(&[b"surfaces", b"--pin", pin, folder])) == ExitCode::from(2));
        assert!(run(&line(&[b"surface", b"--pins", pin, folder])) == ExitCode::from(2));
        assert!(run(&line(&[])) == ExitCode::from(2));
        assert!(run(&line(&[b"surface", b"--pin", b"0123abc\xff", folder])) == ExitCode::FAILURE, "a pin that is not Unicode");
        assert!(!out.exists());
        // macOS refuses to create a name that is not UTF-8 and Linux takes it: either way an outcome, no panic.
        let odd = [folder, b"-\xff"].concat();
        assert!(run(&line(&[b"surface", b"--pin", pin, &odd])) != ExitCode::from(2));
        let _ = std::fs::remove_dir_all(OsString::from_vec(odd));
    }
}
