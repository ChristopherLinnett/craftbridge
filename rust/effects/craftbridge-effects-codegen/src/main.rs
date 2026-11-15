//! Generates the EffectCraft API surface from the engine's command registry, the Swift and Kotlin
//! facades over it, and the diff between two surfaces. `cargo xtask generate` runs it.
//!
//! ```text
//! craftbridge-effects-codegen surface --pin <commit> <out-dir>
//! craftbridge-effects-codegen swift <out-dir>
//! craftbridge-effects-codegen diff <old> <new>
//! ```
//!
//! `surface` writes `api-surface.json` into `<out-dir>`, `swift` one Swift file for each command namespace.
//! `diff` reads two surface files and lists, on standard output, the commands, keys and tools that were
//! added, removed or changed (see [`diff`] for the lines and for what breaks an app). Its exit code is 0
//! when no change breaks an app, 2 when one does and 1 when a file cannot be read or parsed, or the report
//! cannot be written, so that 2 means a breaking change and nothing else; a command line that is no
//! command also exits 1.

mod diff;
mod facade;
mod model;
mod surface;
mod swift;
mod types;

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

/// The exit code of a diff that found a breaking change.
const BREAKING: u8 = 2;

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
        [command, out_dir] if command == "swift" => report(write_swift(Path::new(out_dir))),
        [command, old, new] if command == "diff" => diff_files(Path::new(old), Path::new(new)),
        _ => {
            say(
                std::io::stderr(),
                "usage: craftbridge-effects-codegen surface --pin <commit> <out-dir>\n       craftbridge-effects-codegen swift <out-dir>\n       craftbridge-effects-codegen diff <old> <new>",
            );
            ExitCode::FAILURE
        }
    }
}

/// Read the surfaces in `old` and `new` and print how they differ.
fn diff_files(old: &Path, new: &Path) -> ExitCode {
    let changes = match (read_surface(old), read_surface(new)) {
        (Ok(old), Ok(new)) => diff::compare(&old, &new),
        (Err(error), _) | (_, Err(error)) => return report(Err(error)),
    };
    if let Err(error) = write_to(std::io::stdout(), &diff::report(&changes)) {
        return report(Err(format!("cannot write the report: {error}")));
    }
    if diff::is_breaking(&changes) { ExitCode::from(BREAKING) } else { ExitCode::SUCCESS }
}

/// Write `text` to `stream` whole. Every word the program says goes through here, not through `print!` or
/// `eprint!`, which panic when the reader of the stream has gone: the exit code must still say how the run
/// went.
fn write_to(mut stream: impl Write, text: &str) -> std::io::Result<()> {
    stream.write_all(text.as_bytes())?;
    stream.flush()
}

/// Write `line` to `stream` where no one may be left to read it: what the run did stands either way, and
/// the exit code says it.
fn say(stream: impl Write, line: &str) {
    let _ = write_to(stream, &format!("{line}\n"));
}

/// The surface file at `path`; an error names the file.
fn read_surface(path: &Path) -> Result<model::Surface, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    model::Surface::parse(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn report(outcome: Result<String, String>) -> ExitCode {
    match outcome {
        Ok(done) => {
            say(std::io::stdout(), &format!("craftbridge-effects-codegen: {done}"));
            ExitCode::SUCCESS
        }
        Err(error) => {
            say(std::io::stderr(), &format!("craftbridge-effects-codegen: {error}"));
            ExitCode::FAILURE
        }
    }
}

/// Build the surface at `pin` and write it into `out_dir`, saying what it holds.
fn write_surface(pin: &str, out_dir: &Path) -> Result<String, String> {
    let surface = surface::build(pin)?;
    let text = surface.render()?;
    std::fs::create_dir_all(out_dir).map_err(|error| format!("cannot create {}: {error}", out_dir.display()))?;
    let file = out_dir.join(surface::FILE);
    std::fs::write(&file, text).map_err(|error| format!("cannot write {}: {error}", file.display()))?;
    let counts = surface::type_counts(&surface).into_iter().map(|(name, count)| format!("{count} {name}")).collect::<Vec<_>>().join(", ");
    Ok(format!("{} holds {} commands and {} tools; keys typed {counts}", file.display(), surface.commands.len(), surface.tools.len()))
}

/// Write the Swift facade of every command into `out_dir`, saying what it holds.
fn write_swift(out_dir: &Path) -> Result<String, String> {
    let commands = surface::commands();
    let namespaces = facade::plan(&commands)?;
    swift::check(&namespaces)?;
    let files = swift::files(&namespaces);
    std::fs::create_dir_all(out_dir).map_err(|error| format!("cannot create {}: {error}", out_dir.display()))?;
    for (name, text) in &files {
        let file = out_dir.join(name);
        std::fs::write(&file, text).map_err(|error| format!("cannot write {}: {error}", file.display()))?;
    }
    Ok(format!("{} holds {} files with {} methods", out_dir.display(), files.len(), commands.len()))
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
        assert_eq!(text, surface::build(surface::TEST_PIN).unwrap().render().unwrap());
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn the_swift_is_written_one_file_for_each_namespace() {
        let out = temp("swift");
        let _ = std::fs::remove_dir_all(&out);
        let done = write_swift(&out).unwrap();
        let names: Vec<String> = std::fs::read_dir(&out).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"LayerCommands.swift".to_string()) && names.contains(&"LayerMaskCommands.swift".to_string()), "{names:?}");
        assert!(done.contains(&format!("{} files", names.len())), "{done}");
        let text = std::fs::read_to_string(out.join("RenderQueueCommands.swift")).unwrap();
        assert!(text.contains("public func setRender("), "{text}");
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
        assert!(run(&line(&[b"swift", folder])) == ExitCode::SUCCESS);
        assert!(out.join("LayerCommands.swift").is_file());
        let _ = std::fs::remove_dir_all(&out);
        for usage in [
            &[b"surface" as &[u8], b"--pin", pin][..],
            &[b"surfaces", b"--pin", pin, folder],
            &[b"surface", b"--pins", pin, folder],
            &[b"swift"],
            &[b"swift", folder, b"more"],
            &[b"diff", b"one"],
            &[],
        ] {
            assert!(run(&line(usage)) == ExitCode::FAILURE, "{usage:?} is no command, and 2 is for a breaking diff");
        }
        assert!(run(&line(&[b"surface", b"--pin", b"0123abc\xff", folder])) == ExitCode::FAILURE, "a pin that is not Unicode");
        assert!(!out.exists());
        // macOS refuses to create a name that is not UTF-8 and Linux takes it: either way an outcome, no panic.
        let odd = [folder, b"-\xff"].concat();
        let _ = run(&line(&[b"surface", b"--pin", pin, &odd]));
        let _ = std::fs::remove_dir_all(OsString::from_vec(odd));
    }

    fn key(name: &str, required: bool, key_type: &str, values: Option<&[&str]>) -> model::Key {
        model::Key {
            name: name.to_string(),
            required,
            key_type: key_type.to_string(),
            values: values.map(|words| words.iter().map(|word| (*word).to_string()).collect()),
            doc: String::new(),
        }
    }

    /// A small surface with one thing of each kind a change can break, and a tool no app can call.
    fn fixture() -> model::Surface {
        let command = |id: &str, keys: Vec<model::Key>| model::Command::of(id, id, keys);
        let tool = |name: &str, bridge_only: bool| model::Tool {
            name: name.to_string(),
            description: String::new(),
            bridge_only,
            input_schema: serde_json::json!({"type": "object", "properties": {"x": {"type": "integer"}}, "required": []}),
        };
        model::Surface {
            app: "effectcraft".to_string(),
            engine_version: "1.0.0".to_string(),
            pin: surface::TEST_PIN.to_string(),
            commands: vec![
                command("a.keep", vec![key("on", false, "bool", None), key("must", true, "string", None), key("mode", false, "enum", Some(&["x", "y"]))]),
                command("b.gone", vec![]),
            ],
            tools: vec![tool("undo", false), tool("ui_click", true), tool("redo", false)],
        }
    }

    /// Run `diff` on the fixture and the fixture with `edit` applied, in files of their own.
    fn exit_of_diff(name: &str, edit: impl FnOnce(&mut model::Surface)) -> ExitCode {
        let folder = temp(name);
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let (old, new) = (folder.join("old.json"), folder.join("new.json"));
        let mut edited = fixture();
        edit(&mut edited);
        std::fs::write(&old, fixture().render().unwrap()).unwrap();
        std::fs::write(&new, edited.render().unwrap()).unwrap();
        let code = run(&[OsString::from("diff"), old.into_os_string(), new.into_os_string()]);
        let _ = std::fs::remove_dir_all(&folder);
        code
    }

    fn first_command(surface: &mut model::Surface) -> &mut model::Command {
        surface.commands.first_mut().unwrap()
    }

    #[test]
    fn a_diff_that_loses_a_command_key_or_tool_or_makes_one_stricter_exits_2() {
        type Edit = fn(&mut model::Surface);
        let breaking: [(&str, Edit); 11] = [
            ("removed-command", |s| drop(s.commands.pop())),
            ("removed-key", |s| drop(first_command(s).keys.remove(0))),
            ("removed-tool", |s| drop(s.tools.pop())),
            ("key-became-required", |s| first_command(s).keys.first_mut().unwrap().required = true),
            ("key-type-changed", |s| first_command(s).keys.first_mut().unwrap().key_type = "json".to_string()),
            ("new-required-key", |s| first_command(s).keys.push(key("fresh", true, "bool", None))),
            ("removed-enum-word", |s| first_command(s).keys.get_mut(2).unwrap().values = Some(vec!["x".to_string()])),
            ("removed-whole-word-list", |s| first_command(s).keys.get_mut(2).unwrap().values = None),
            ("keys-reordered", |s| first_command(s).keys.swap(0, 1)),
            ("new-optional-key-first", |s| first_command(s).keys.insert(0, key("fresh", false, "bool", None))),
            ("tool-became-bridge-only", |s| s.tools.first_mut().unwrap().bridge_only = true),
        ];
        for (name, edit) in breaking {
            assert!(exit_of_diff(name, edit) == ExitCode::from(BREAKING), "{name}");
        }
    }

    #[test]
    fn a_diff_that_only_adds_or_loosens_exits_0() {
        type Edit = fn(&mut model::Surface);
        let compatible: [(&str, Edit); 8] = [
            ("nothing", |_| {}),
            ("new-command", |s| s.commands.push(model::Command { journal: false, ..model::Command::of("c.new", "", vec![]) })),
            ("new-optional-key-last", |s| first_command(s).keys.push(key("fresh", false, "bool", None))),
            ("bridge-only-tool-removed", |s| s.tools.retain(|tool| !tool.bridge_only)),
            ("new-enum-word", |s| first_command(s).keys.get_mut(2).unwrap().values = Some(vec!["x".to_string(), "y".to_string(), "z".to_string()])),
            ("key-became-optional", |s| first_command(s).keys.get_mut(1).unwrap().required = false),
            ("new-label", |s| first_command(s).label = "Other".to_string()),
            ("new-tool", |s| {
                s.tools.push(model::Tool { name: "zoom".to_string(), description: String::new(), bridge_only: false, input_schema: serde_json::json!({}) })
            }),
        ];
        for (name, edit) in compatible {
            assert!(exit_of_diff(name, edit) == ExitCode::SUCCESS, "{name}");
        }
    }

    #[test]
    fn a_file_that_cannot_be_read_exits_1_naming_it() {
        let folder = temp("unreadable");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let good = folder.join("good.json");
        std::fs::write(&good, fixture().render().unwrap()).unwrap();
        let cases = [
            ("missing.json", None),
            ("empty.json", Some("")),
            ("junk.json", Some("not json")),
            ("array.json", Some("[]")),
            ("twice.json", Some("{\"app\":\"effectcraft\"}")),
        ];
        for (name, text) in cases {
            let bad = folder.join(name);
            if let Some(text) = text {
                std::fs::write(&bad, text).unwrap();
            }
            for (old, new) in [(&bad, &good), (&good, &bad)] {
                assert!(diff_files(old, new) == ExitCode::FAILURE, "{name}");
                let message = read_surface(&bad).unwrap_err();
                assert!(message.contains(&bad.display().to_string()), "{message}");
            }
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_file_that_repeats_a_command_exits_1() {
        let folder = temp("repeat");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let mut repeated = fixture();
        let again = repeated.commands.first().unwrap().clone();
        repeated.commands.push(again);
        let file = folder.join("repeated.json");
        std::fs::write(&file, serde_json::to_string(&repeated).unwrap()).unwrap();
        let message = read_surface(&file).unwrap_err();
        assert!(message.contains("repeated.json") && message.contains("`a.keep`"), "{message}");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
