//! Generates the EffectCraft API surface from the engine's command registry, the Swift and Kotlin
//! facades over it, the constants the plugin's `info` reports, and the diff between two surfaces. `cargo xtask generate` runs it.
//!
//! ```text
//! craftbridge-effects-codegen surface --pin <commit> <out-dir>
//! craftbridge-effects-codegen swift <out-dir>
//! craftbridge-effects-codegen kotlin <out-dir>
//! craftbridge-effects-codegen version --pin <commit> <out-dir>
//! craftbridge-effects-codegen diff <old> <new>
//! ```
//!
//! `surface` writes `api-surface.json` into `<out-dir>`, `swift` one Swift file for each command namespace,
//! `kotlin` one Kotlin file for each, in the package the `:effects` module declares, and `version` the
//! plugin's Rust file of the engine's version and pin.
//! `diff` reads two surface files and lists, on standard output, the commands, keys and tools that were
//! added, removed or changed (see [`diff`] for the lines and for what breaks an app). Its exit code is 0
//! when no change breaks an app, 2 when one does and 1 when a file cannot be read or parsed, or the report
//! cannot be written, so that 2 means a breaking change and nothing else; a command line that is no
//! command also exits 1.

mod diff;
mod facade;
mod kotlin;
mod model;
mod refused;
mod surface;
mod swift;
mod types;
mod version;

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
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
        [command, flag, pin, out_dir] if command == "surface" && flag == "--pin" => {
            report(pin_text(pin).and_then(|pin| write_surface(pin, Path::new(out_dir))))
        }
        [command, out_dir] if command == swift::COMMAND => report(write_facade(&SWIFT, Path::new(out_dir))),
        [command, out_dir] if command == kotlin::COMMAND => report(write_facade(&KOTLIN, Path::new(out_dir))),
        [command, flag, pin, out_dir] if command == version::COMMAND && flag == "--pin" => {
            report(pin_text(pin).and_then(|pin| write_version(pin, Path::new(out_dir))))
        }
        [command, old, new] if command == "diff" => diff_files(Path::new(old), Path::new(new)),
        _ => {
            say(
                std::io::stderr(),
                "usage: craftbridge-effects-codegen surface --pin <commit> <out-dir>\n       craftbridge-effects-codegen swift <out-dir>\n       craftbridge-effects-codegen kotlin <out-dir>\n       craftbridge-effects-codegen version --pin <commit> <out-dir>\n       craftbridge-effects-codegen diff <old> <new>",
            );
            ExitCode::FAILURE
        }
    }
}

/// The pin a command line names, which is text: a commit id is hexadecimal digits.
fn pin_text(pin: &OsStr) -> Result<&str, String> {
    pin.to_str().ok_or_else(|| format!("`{}` is no pin: a commit id is hexadecimal digits", pin.to_string_lossy()))
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

/// The first line of every file the generator writes in a format that holds comments (the surface's JSON
/// holds none), the marker the gate's `generated` stage recognises them by: `command` names the
/// subcommand that wrote it, as the module that writes the file names it for `run`.
pub(crate) fn generated_notice(command: &str) -> String {
    format!("// @generated by `cargo xtask generate` (craftbridge-effects-codegen {command}): do not edit by hand.\n")
}

/// Write `text` as the file `name` in `out_dir`, creating the folder first, and return the file's path:
/// every command's one way of writing what it generated, so each failure names the folder or file.
fn write_into(out_dir: &Path, name: &str, text: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(out_dir).map_err(|error| format!("cannot create {}: {error}", out_dir.display()))?;
    let file = out_dir.join(name);
    std::fs::write(&file, text).map_err(|error| format!("cannot write {}: {error}", file.display()))?;
    Ok(file)
}

/// Build the surface at `pin` and write it into `out_dir`, saying what it holds.
fn write_surface(pin: &str, out_dir: &Path) -> Result<String, String> {
    let surface = surface::build(pin)?;
    let file = write_into(out_dir, surface::FILE, &surface.render()?)?;
    let counts = surface::type_counts(&surface).into_iter().map(|(name, count)| format!("{count} {name}")).collect::<Vec<_>>().join(", ");
    Ok(format!("{} holds {} commands and {} tools; keys typed {counts}", file.display(), surface.commands.len(), surface.tools.len()))
}

/// One facade's emitter: the checks of the names it declares for itself, then its files, each a path
/// below the output folder and the file's text.
struct Emitter {
    check: fn(&[facade::Namespace]) -> Result<(), String>,
    files: fn(&[facade::Namespace]) -> Vec<(String, String)>,
}

const SWIFT: Emitter = Emitter { check: swift::check, files: swift::files };
const KOTLIN: Emitter = Emitter { check: kotlin::check, files: kotlin::files };

/// Write one facade of every command an app can call into `out_dir`, saying what it holds.
fn write_facade(emitter: &Emitter, out_dir: &Path) -> Result<String, String> {
    let namespaces = facade::plan(&surface::commands()?)?;
    (emitter.check)(&namespaces)?;
    let files = (emitter.files)(&namespaces);
    for (name, text) in &files {
        write_into(out_dir, name, text)?;
    }
    let methods: usize = namespaces.iter().map(|namespace| namespace.methods.len()).sum();
    Ok(format!("{} holds {} files with {methods} methods", out_dir.display(), files.len()))
}

/// Write the engine's version and pin constants into `out_dir`.
fn write_version(pin: &str, out_dir: &Path) -> Result<String, String> {
    let file = write_into(out_dir, version::FILE, &version::build(pin)?)?;
    Ok(format!("{} holds version {} at pin {pin}", file.display(), surface::ENGINE_VERSION))
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
    fn each_facade_is_written_one_file_for_each_namespace() {
        for (emitter, extension, call) in [(&SWIFT, "swift", "public func setRender("), (&KOTLIN, "kt", "public suspend fun setRender(")] {
            let out = temp(extension);
            let _ = std::fs::remove_dir_all(&out);
            let done = write_facade(emitter, &out).unwrap();
            let names: Vec<String> = std::fs::read_dir(&out).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
            for file in ["LayerCommands", "LayerMaskCommands"] {
                assert!(names.contains(&format!("{file}.{extension}")), "{names:?}");
            }
            assert!(done.contains(&format!("{} files with {} methods", names.len(), facade::callable_at_the_pin())), "{done}");
            let text = std::fs::read_to_string(out.join(format!("RenderQueueCommands.{extension}"))).unwrap();
            assert!(text.contains(call), "{text}");
            let _ = std::fs::remove_dir_all(&out);
        }
    }

    #[test]
    fn the_version_is_written_where_asked_and_a_bad_pin_writes_nothing() {
        let out = temp("version");
        let _ = std::fs::remove_dir_all(&out);
        let done = write_version(surface::TEST_PIN, &out).unwrap();
        assert!(done.contains(surface::ENGINE_VERSION) && done.contains(surface::TEST_PIN), "{done}");
        let text = std::fs::read_to_string(out.join(version::FILE)).unwrap();
        assert_eq!(text, version::build(surface::TEST_PIN).unwrap());
        let _ = std::fs::remove_dir_all(&out);
        assert!(write_version("main", &out).is_err());
        assert!(!out.exists());
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
        assert!(run(&line(&[b"kotlin", folder])) == ExitCode::SUCCESS);
        assert!(out.join("LayerCommands.kt").is_file());
        let _ = std::fs::remove_dir_all(&out);
        assert!(run(&line(&[b"version", b"--pin", pin, folder])) == ExitCode::SUCCESS);
        assert!(out.join(version::FILE).is_file());
        let _ = std::fs::remove_dir_all(&out);
        for usage in [
            &[b"surface" as &[u8], b"--pin", pin][..],
            &[b"surfaces", b"--pin", pin, folder],
            &[b"surface", b"--pins", pin, folder],
            &[b"swift"],
            &[b"swift", folder, b"more"],
            &[b"kotlin"],
            &[b"kotlin", folder, b"more"],
            &[b"version", b"--pin", pin],
            &[b"version", b"--pins", pin, folder],
            &[b"version", b"--pin", pin, b"--engine-version", b"1.2.3", folder],
            &[b"diff", b"one"],
            &[],
        ] {
            assert!(run(&line(usage)) == ExitCode::FAILURE, "{usage:?} is no command, and 2 is for a breaking diff");
        }
        assert!(run(&line(&[b"surface", b"--pin", b"0123abc\xff", folder])) == ExitCode::FAILURE, "a pin that is not Unicode");
        assert!(run(&line(&[b"version", b"--pin", b"0123abc\xff", folder])) == ExitCode::FAILURE, "a pin that is not Unicode");
        assert!(!out.exists());
        // macOS refuses to create a name that is not UTF-8 and Linux takes it: either way an outcome, no panic.
        let odd = [folder, b"-\xff"].concat();
        let _ = run(&line(&[b"surface", b"--pin", pin, &odd]));
        let _ = std::fs::remove_dir_all(OsString::from_vec(odd));
    }

    /// A small surface with one thing of each kind a change can break, and a tool no app can call.
    fn fixture() -> model::Surface {
        let command = |id: &str, keys: Vec<model::Key>| model::Command::of(id, id, keys);
        let input = serde_json::json!({"type": "object", "properties": {"x": {"type": "integer"}}, "required": []});
        let mut hidden = model::Tool::of("ui_click", input.clone());
        hidden.bridge_only = true;
        model::Surface::of(
            vec![
                command(
                    "a.keep",
                    vec![
                        model::Key::raw("on", false, "bool", None),
                        model::Key::raw("must", true, "string", None),
                        model::Key::raw("mode", false, "enum", Some(&["x", "y"])),
                    ],
                ),
                command("b.gone", vec![]),
            ],
            vec![model::Tool::of("undo", input.clone()), hidden, model::Tool::of("redo", input)],
        )
    }

    /// Run `diff` on the fixture and the fixture with `edit` applied, in files of their own.
    fn exit_of_diff(name: &str, edit: impl FnOnce(&mut model::Surface)) -> ExitCode {
        exit_of_change(name, |_| {}, edit)
    }

    /// Run `diff` on the fixture with `before` applied as the old file and with `after` applied as the
    /// new one, so a test can move the old surface too.
    fn exit_of_change(name: &str, before: impl FnOnce(&mut model::Surface), after: impl FnOnce(&mut model::Surface)) -> ExitCode {
        let folder = temp(name);
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let (old, new) = (folder.join("old.json"), folder.join("new.json"));
        let (mut old_surface, mut new_surface) = (fixture(), fixture());
        before(&mut old_surface);
        after(&mut new_surface);
        std::fs::write(&old, old_surface.render().unwrap()).unwrap();
        std::fs::write(&new, new_surface.render().unwrap()).unwrap();
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
        let breaking: [(&str, Edit); 10] = [
            ("removed-command", |s| drop(s.commands.pop())),
            ("removed-key", |s| drop(first_command(s).keys.remove(0))),
            ("removed-tool", |s| drop(s.tools.pop())),
            ("key-became-required", |s| first_command(s).keys.first_mut().unwrap().required = true),
            ("key-type-changed", |s| first_command(s).keys.first_mut().unwrap().key_type = "json".to_string()),
            ("new-required-key", |s| first_command(s).keys.push(model::Key::raw("fresh", true, "bool", None))),
            ("removed-enum-word", |s| first_command(s).keys.get_mut(2).unwrap().values = Some(vec!["x".to_string()])),
            ("removed-whole-word-list", |s| first_command(s).keys.get_mut(2).unwrap().values = None),
            ("keys-reordered", |s| first_command(s).keys.swap(0, 1)),
            ("tool-became-bridge-only", |s| s.tools.first_mut().unwrap().bridge_only = true),
        ];
        for (name, edit) in breaking {
            assert!(exit_of_diff(name, edit) == ExitCode::from(BREAKING), "{name}");
        }
    }

    #[test]
    fn a_diff_that_only_adds_or_loosens_exits_0() {
        type Edit = fn(&mut model::Surface);
        let compatible: [(&str, Edit); 9] = [
            ("nothing", |_| {}),
            ("new-command", |s| s.commands.push(model::Command { journal: false, ..model::Command::of("c.new", "", vec![]) })),
            ("new-optional-key-last", |s| first_command(s).keys.push(model::Key::raw("fresh", false, "bool", None))),
            ("new-optional-key-first", |s| first_command(s).keys.insert(0, model::Key::raw("fresh", false, "bool", None))),
            ("bridge-only-tool-removed", |s| s.tools.retain(|tool| !tool.bridge_only)),
            ("new-enum-word", |s| first_command(s).keys.get_mut(2).unwrap().values = Some(vec!["x".to_string(), "y".to_string(), "z".to_string()])),
            ("key-became-optional", |s| first_command(s).keys.get_mut(1).unwrap().required = false),
            ("new-label", |s| first_command(s).label = "Other".to_string()),
            ("new-tool", |s| s.tools.push(model::Tool::of("zoom", serde_json::json!({})))),
        ];
        for (name, edit) in compatible {
            assert!(exit_of_diff(name, edit) == ExitCode::SUCCESS, "{name}");
        }
    }

    #[test]
    fn a_change_to_a_refused_entry_exits_0_and_the_same_change_to_a_callable_one_exits_2() {
        type Edit = fn(&mut model::Surface);
        type Marks<'a> = &'a [(&'a str, Edit)];
        // Each mark that keeps the fixture's first command, or its first tool, from apps.
        let command_marks: [(&str, Edit); 2] =
            [("refused", |s| first_command(s).refused_under_confinement = true), ("withheld", |s| first_command(s).withheld_by_plugin = true)];
        let tool_marks: [(&str, Edit); 1] = [("refused", |s| s.tools.first_mut().unwrap().refused_under_confinement = true)];
        // Each edit falls on that first entry, open to apps in the fixture: the same edit on it once a
        // mark keeps it from apps, on both sides, is no call an app loses, since no facade offers one.
        let command_edits: [(&str, Edit); 5] = [
            ("marks-command-removed", |s| drop(s.commands.remove(0))),
            ("marks-key-removed", |s| drop(first_command(s).keys.remove(1))),
            ("marks-key-required", |s| first_command(s).keys.first_mut().unwrap().required = true),
            ("marks-key-retyped", |s| first_command(s).keys.first_mut().unwrap().key_type = "json".to_string()),
            ("marks-keys-reordered", |s| first_command(s).keys.swap(0, 1)),
        ];
        let tool_edits: [(&str, Edit); 3] = [
            ("marks-tool-removed", |s| drop(s.tools.remove(0))),
            ("marks-tool-input-changed", |s| s.tools.first_mut().unwrap().input_schema = serde_json::json!({})),
            ("marks-tool-input-required", |s| s.tools.first_mut().unwrap().input_schema["required"] = serde_json::json!(["x"])),
        ];
        let kinds: [(Marks, Marks); 2] = [(&command_marks, &command_edits), (&tool_marks, &tool_edits)];
        for (marks, edits) in kinds {
            for &(name, edit) in edits {
                assert!(exit_of_diff(name, edit) == ExitCode::from(BREAKING), "{name} on a callable entry");
                for &(mark_name, mark) in marks {
                    let marked_and_edited = move |s: &mut model::Surface| {
                        mark(s);
                        edit(s);
                    };
                    assert!(exit_of_change(name, mark, marked_and_edited) == ExitCode::SUCCESS, "{name} on a {mark_name} entry");
                }
            }
            for &(mark_name, mark) in marks {
                // Becoming refused or withheld takes a call away, whatever else stays: exit 2, like any
                // other breaking move; being released only frees a call: exit 0.
                assert!(exit_of_change("marks-set", |_| {}, mark) == ExitCode::from(BREAKING), "an entry became {mark_name}");
                assert!(exit_of_change("marks-lifted", mark, |_| {}) == ExitCode::SUCCESS, "a {mark_name} entry is callable again");
            }
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
