//! No tracked file names a machine's user through an absolute home-folder path (AGENTS.md §1).
//!
//! The repository is public, and a path such as `/Users/<name>/…` or `/home/<name>/…`, which build logs,
//! simulator and emulator captures and a kit's state files print as a matter of course, names the user of
//! the machine that wrote it. Such a path is written `$HOME/…` instead.
//!
//! Git answers which files the rule reads: `git grep` searches the work tree's copy of every file git
//! tracks, so a plant in a tracked file fails before it is even staged, while a file git does not track,
//! or ignores, is no part of what the repository publishes and is not read. It searches every tracked file
//! as text (`-a`) and byte by byte (the `C` locale, whatever the machine's is): git takes a file holding a
//! NUL byte for a binary one, and in a UTF-8 locale the regular-expression engines git uses miss a path
//! behind a byte that is not UTF-8, which a binary's lines hold. macOS's reads a line only up to its first
//! such byte, and glibc's, on Linux, misses a path right behind one. A device log with one stray NUL, or a
//! binary built with its build folder inside it, would otherwise publish the name unread.
//!
//! A home-folder path is `/Users/` or `/home/` followed by an account name, which starts with a letter, a
//! digit or `_` as every account name does; in any case, because the Mac's disk folds case, so
//! `/users/<name>` opens the same folder. The folder must begin a path: what comes before it cannot
//! continue one, so `file:///Users/<name>` and `"/home/<name>"` are home folders while `src/home/<name>/`,
//! `~/home/<name>` and `${HOME}/home/<name>` are folders inside another path. A path the tools that write
//! captures print in another spelling is one too: through macOS's firmlink
//! (`/System/Volumes/Data/Users/<name>`), under `/var/home` or `/usr/home`, where Fedora's atomic desktops
//! and FreeBSD keep home folders, glued to a compiler's option in front of any of those (`-L/home/<name>`,
//! `-I/var/home/<name>`, `-isystem`), and with JSON's escaped slashes (`\/Users\/<name>`), which Apple's
//! JSON writers use, escaped again or not (`\\/Users\\/<name>`, as JSON inside JSON or a debug print of
//! it writes them). `/Users/Shared` is no one's home folder but the folder macOS gives every user, so it
//! passes. A placeholder names nobody and passes too: `/Users/<name>/`, `/Users/…`, `/home/.cargo`.
//! Neither this file nor its tests may hold a real one, so the tests build theirs.

use std::path::Path;

/// The pattern `git grep` matches, case-insensitively, as one POSIX extended expression: its groups in
/// order, each optional one apart, since an option glues to whichever path it takes.
const HOME_FOLDER: &str = concat!(
    // Where a path begins: the start of a line, or a character that cannot continue a path. A letter, a
    // digit, `.`, `_` and `-` continue a relative one, `~` the home folder, and `)` or `}` an expansion
    // such as `$(HOME)` or `${HOME}`, so what follows any of them lies inside another folder.
    "(^|[^a-z0-9._~)}-])",
    // A compiler's option glued to the path, optional: `-L`, `-I`, `-isystem`.
    "(-[a-z]+)?",
    // A folder above the home folders, optional: macOS's firmlink, and `/var` and `/usr`.
    "(/system/volumes/data|/var|/usr)?",
    // The home folders, the slash after each plain or escaped as JSON writes it (`\/`), however often the
    // escape is escaped again, as JSON inside JSON or a debug print of it writes it (`\\/`), then the first
    // character of an account name. Under `home` any name. Under `Users` any name but `Shared`, spelt out
    // letter by letter, since an extended expression has no look-ahead.
    "/(home\\\\*/[a-z0-9_]",
    "|users\\\\*/([a-rt-z0-9_]|s($|[^h])|sh($|[^a])|sha($|[^r])|shar($|[^e])|share($|[^d])|shared[a-z0-9._-]))",
);

/// What every line the pattern matches holds, in any case. `git grep` finds a fixed string without the
/// regular-expression engine and runs the pattern only on the lines holding one, which reads the whole
/// tree many times faster than the pattern alone; a spelling of the pattern that held neither would fail
/// the tracked-file test, which expects each of its lines named.
const FOLDERS: [&str; 2] = ["/home", "/users"];

/// What a line holding a home-folder path is told, after its file and line number. The path itself is not
/// repeated: the gate's output would publish it again.
const MESSAGE: &str =
    "an absolute home-folder path names the user of the machine that wrote it, and the repository is public: write the home folder as `$HOME`";

/// Run the check over the tracked files of the checkout at `root`. Outside a git checkout nothing is
/// tracked, so there is nothing to report; inside one, a git that cannot answer is an error, never an
/// empty answer, or the check would pass files nobody read.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    if !crate::git::is_checkout(root) {
        return Ok(Vec::new());
    }
    let [first, second] = FOLDERS;
    let args = ["grep", "-z", "-n", "-a", "-i", "-E", "--no-color", "--no-column", "(", "-e", first, "-e", second, ")", "--and", "-e", HOME_FOLDER];
    // `LC_ALL` outranks every other locale variable the machine sets, and with `C` gettext prints git's
    // own words in an error the same on every machine too.
    let output = crate::git::command(root).env("LC_ALL", "C").args(args).output().map_err(|error| format!("could not run git: {error}"))?;
    // `git grep` exits 0 when a line matches and 1 when none does. A file it could not read is named on its
    // error output, whichever of the two it exits with, and then the answer leaves that file unread.
    match output.status.code() {
        Some(0 | 1) if output.stderr.is_empty() => Ok(found(&output.stdout)),
        _ => Err(format!("`git {}` failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim())),
    }
}

/// One error per matching line of what `git grep -z -n` printed: each record is the path, a NUL, the line
/// number, a NUL and the line up to its newline. The path holds no NUL and the number no newline, so each
/// is read up to its own separator, and the line, which `-a` may hand over holding NUL bytes, up to the
/// newline that ends the record.
fn found(printed: &[u8]) -> Vec<String> {
    let mut errors = Vec::new();
    let mut rest = printed;
    while let Some((path, after_path)) = split_at_byte(rest, 0)
        && let Some((line, after_line)) = split_at_byte(after_path, 0)
    {
        errors.push(format!("{}:{}: {MESSAGE}", String::from_utf8_lossy(path), String::from_utf8_lossy(line)));
        rest = split_at_byte(after_line, b'\n').map_or(&[], |(_, next)| next);
    }
    errors
}

/// The bytes before the first `separator` and those after it, or nothing when it does not occur.
fn split_at_byte(bytes: &[u8], separator: u8) -> Option<(&[u8], &[u8])> {
    let at = bytes.iter().position(|byte| *byte == separator)?;
    Some((bytes.get(..at)?, bytes.get(at.checked_add(1)?..)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    /// A home-folder path, built here so that this file, which git tracks, holds none itself.
    fn home(folder: &str, rest: &str) -> String {
        format!("/{folder}/{rest}")
    }

    /// A scratch checkout holding `tracked`, each file added to git's index, and `.gitignore`.
    fn checkout(tag: &str, tracked: &[(&str, &[u8])], ignore: &str) -> Scratch {
        let scratch = Scratch::new(tag);
        crate::git::output(&scratch.0, &["init", "-q"]).unwrap();
        scratch.write(".gitignore", ignore);
        crate::git::output(&scratch.0, &["add", ".gitignore"]).unwrap();
        for (path, bytes) in tracked {
            scratch.write(path, bytes);
            crate::git::output(&scratch.0, &["add", "--", path]).unwrap();
        }
        scratch
    }

    fn error(path: &str, line: usize) -> String {
        format!("{path}:{line}: {MESSAGE}")
    }

    /// Every `Users` account name a prefix of `Shared` leaves: each prefix ending the line or followed by
    /// another letter, and the whole word followed by one, so each step of the spelt-out exemption is held.
    fn names_beside_shared() -> Vec<String> {
        let word = "Shared";
        let prefixes = (1..word.len()).map(|end| word.chars().take(end).collect::<String>());
        let mut names: Vec<String> = prefixes.flat_map(|prefix| [format!("{prefix}x/"), prefix]).collect();
        names.push(format!("{word}x/"));
        names.push("SHARED_old".to_string());
        names
    }

    /// Every spelling a capture or a person writes is named by its file and line: at a line's start, after
    /// a space, an `=`, a quote, a backtick, an opening bracket or brace or a URL's `file://`, the folder
    /// alone and with a path below it, in every case, an account name starting with a digit or `_`, glued
    /// to a compiler's option, through macOS's firmlink, under `/var` and `/usr`, each of those three behind
    /// a glued option, with JSON's escaped slashes, escaped again or not, every account name beside
    /// `Shared`, and in a file git takes for binary, whatever its attributes say, behind a byte that is not
    /// UTF-8 too: right before the path, where a binary plist writes a long string's length, and earlier on
    /// its line; and whatever the checkout's configuration sets for how `git grep` matches and prints.
    #[test]
    fn a_home_folder_path_in_a_tracked_file_is_named_by_its_file_and_line() {
        let mut lines = vec![
            home("Users", "x/notes.md"),
            format!("HOME = {}", home("home", "x")),
            format!("git = \"file://{}\"", home("Users", "x/dev/effectcraft")),
            format!("`{}` and '{}'", home("users", "x/"), home("HOME", "X/a")),
            format!("SIMULATOR_HOST_HOME = {}", home("USERS", "Alice")),
            format!("({})", home("home", "_build/x")),
            format!("{{{}}}", home("home", "x/")),
            format!("cd {}", home("Home", "1x/")),
            format!("cc -L{} -I{}", home("home", "x/lib"), home("home", "x/include")),
            format!("-isystem{}", home("Users", "x/include")),
            format!("/System/Volumes/Data{}", home("Users", "x/dev")),
            format!("/var{}", home("home", "x")),
            format!("/usr{}", home("HOME", "x")),
            format!("-F/System/Volumes/Data{}", home("Users", "x/Frameworks")),
            format!("cc -L/var{}", home("home", "x/lib")),
            format!("-I/usr{}", home("home", "x/include")),
            format!("\"dataPath\" : \"\\/{}\\/x\\/Library\"", "Users"),
            format!("\"\\/{}\\/x\"", "home"),
            format!(r#"left: "{{\"path\":\"\\/{}\\/x\"}}""#, "Users"),
            format!(r"\\\/{}\\\/x", "home"),
            format!("{} and {}", home("Users", "Shared/"), home("Users", "Sharedx/")),
            home("home", "Shared/"),
            home("Users", "_x/"),
            home("users", "9x/"),
        ];
        lines.extend(names_beside_shared().iter().map(|name| home("Users", name)));
        let text = lines.join("\n") + "\n";
        let mut binary = b"\x89PNG\0".to_vec();
        binary.extend_from_slice(format!("\n{}\0\n", home("Users", "x/build")).as_bytes());
        binary.extend_from_slice(b"\x96");
        binary.extend_from_slice(format!("{}\0\n", home("Users", "x/build/x.o")).as_bytes());
        binary.extend_from_slice(b"\xff\xfe\0abc ");
        binary.extend_from_slice(format!("{}\n", home("home", "x/lib")).as_bytes());
        let scratch = checkout(
            "home-tracked",
            &[("plan/evidence/v1/capture/report.txt", text.as_bytes()), ("android/build.bin", &binary), ("docs/plain.md", b"nothing here\n")],
            "",
        );
        scratch.write(".gitattributes", "*.bin binary\n");
        // Configuration that would change what `git grep` prints, as a machine's own may set it.
        for (key, value) in [("grep.column", "true"), ("color.grep", "always"), ("grep.patternType", "fixed")] {
            crate::git::output(&scratch.0, &["config", key, value]).unwrap();
        }
        let errors = check(&scratch.0).unwrap();
        let binary_lines = (2..=4).map(|line| error("android/build.bin", line));
        let expected: Vec<String> = binary_lines.chain((1..=lines.len()).map(|line| error("plan/evidence/v1/capture/report.txt", line))).collect();
        assert_eq!(errors, expected);
    }

    /// The machine's locale changes nothing the check finds: the test above runs again in a child under a
    /// UTF-8 locale, set the way that outranks every other locale variable, where a regular-expression
    /// engine reading the machine's locale stops at the binary file's bytes that are not UTF-8.
    #[test]
    fn the_locale_changes_nothing_the_check_finds() {
        craftbridge_process::rerun(
            "home::tests::child_names_every_home_folder_path_under_a_utf8_locale",
            &[("LC_ALL", std::ffi::OsStr::new("C.UTF-8"))],
            "under a UTF-8 locale, the check did not name every home-folder path",
        )
        .unwrap();
    }

    #[test]
    #[ignore = "run by `the_locale_changes_nothing_the_check_finds`, under a UTF-8 locale"]
    fn child_names_every_home_folder_path_under_a_utf8_locale() {
        a_home_folder_path_in_a_tracked_file_is_named_by_its_file_and_line();
    }

    /// What names nobody passes: the home folder written `$HOME` or `~`, a folder inside it or inside an
    /// expansion (`~/home/x`, `${HOME}/home/x`, `$(HOME)/Users/x`, `{root}/home/x`), a placeholder for the
    /// name, a dot-named folder, macOS's `Shared` folder in any case and spelling, a `home` folder inside a
    /// relative path or a URL, an option-like folder inside one, and the folder's name alone. So do a file
    /// git does not track and one it ignores, and a tracked file whose plant left the work tree.
    #[test]
    fn a_path_that_names_nobody_and_a_file_git_does_not_track_pass() {
        let legal = [
            "$HOME/Library/Developer/CoreSimulator".to_string(),
            "~/dev/artcraft-dev".to_string(),
            format!("~{} and ~{}", home("home", "x/"), home("Users", "x/")),
            format!("${{HOME}}{}", home("home", "x/")),
            format!("$(HOME){}", home("Users", "x/")),
            format!("{{root}}{}", home("home", "x/")),
            home("Users", "<name>/"),
            format!("-L{}", home("home", "<name>/lib")),
            home("Users", "…"),
            home("home", ".cargo/git/checkouts"),
            home("Users", "Shared/Caches"),
            home("Users", "Shared"),
            home("USERS", "SHARED/x"),
            format!("{}\r", home("users", "shared")),
            format!("\\/{}\\/Shared\\/", "Users"),
            format!(r"\\/{}\\/Shared\\/", "Users"),
            format!("/System/Volumes/Data{}", home("Users", "Shared/")),
            format!("-F/System/Volumes/Data{}", home("Users", "Shared/Frameworks")),
            "crates/engine/src/home/templates.rs".to_string(),
            format!("src{}", home("home", "x/y.rs")),
            format!("craftbridge-core{}", home("home", "x/")),
            format!("x-I{}", home("Users", "y/")),
            format!("https://github.com{}", home("users", "x/projects/9")),
            format!("SRC{}", home("HOME", "X/")),
            home("Users", ""),
            format!("{} x", home("home", "")),
        ];
        let scratch = checkout(
            "home-legal",
            &[("docs/legal.md", (legal.join("\n") + "\n").as_bytes()), ("docs/removed.md", home("Users", "x/\n").as_bytes())],
            "ignored/\n",
        );
        scratch.write("docs/removed.md", "gone\n");
        scratch.write("docs/untracked.md", home("Users", "x/notes\n"));
        scratch.write("ignored/kept.md", home("home", "x/notes\n"));
        assert_eq!(check(&scratch.0).unwrap(), Vec::<String>::new());
    }

    /// Outside a checkout no file is tracked, so nothing is read; a checkout git cannot read is an error in
    /// git's own words, never a pass.
    #[test]
    fn a_tree_git_does_not_hold_passes_and_a_checkout_git_cannot_read_fails() {
        let plain = Scratch::new("home-plain");
        plain.write("notes.md", home("Users", "x/\n"));
        assert_eq!(check(&plain.0).unwrap(), Vec::<String>::new());
        let broken = Scratch::new("home-broken");
        broken.write(".git", "gitdir: does-not-exist\n");
        let failure = check(&broken.0).unwrap_err();
        assert!(failure.starts_with("`git grep -z -n -a -i -E --no-color --no-column ( -e /home -e /users ) --and -e "), "{failure}");
        assert!(failure.contains("not a git repository"), "{failure}");
    }

    /// A tracked file git cannot open is an error in git's words, never a pass: `git grep` names it on its
    /// error output and exits 1 when nothing else matches, as it does when nothing matches at all, and 0
    /// when another file does. A user the file system lets read every file, as root is, reads it and has
    /// the line named instead.
    #[test]
    fn a_tracked_file_git_cannot_open_fails() {
        use std::os::unix::fs::PermissionsExt;
        let (locked_text, open_text) = (home("Users", "x/\n"), home("home", "x/\n"));
        for (tag, others) in [("home-unreadable", &[][..]), ("home-unreadable-beside", &[("docs/open.md", open_text.as_str())][..])] {
            let mut tracked: Vec<(&str, &[u8])> = vec![("docs/locked.md", locked_text.as_bytes())];
            tracked.extend(others.iter().map(|(path, text)| (*path, text.as_bytes())));
            let scratch = checkout(tag, &tracked, "");
            let locked = scratch.0.join("docs/locked.md");
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
            let outcome = check(&scratch.0);
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
            match outcome {
                Err(failure) => assert!(failure.contains("'docs/locked.md'"), "{tag}: {failure}"),
                Ok(errors) => {
                    let mut expected = vec![error("docs/locked.md", 1)];
                    expected.extend(others.iter().map(|(path, _)| error(path, 1)));
                    assert_eq!(errors, expected, "{tag}");
                }
            }
        }
    }

    /// The parser reads each record to its own separators, so a path with a newline in it and a line with
    /// NUL bytes in it are named as git printed them.
    #[test]
    fn each_record_is_read_to_its_own_separators() {
        assert_eq!(found(b"a\nb.txt\x003\x00x\x00y\nc.md\x0012\x00z\n"), [error("a\nb.txt", 3), error("c.md", 12)]);
        assert_eq!(found(b""), Vec::<String>::new());
        assert_eq!(found(b"d.md\x001\x00no newline at the end"), [error("d.md", 1)]);
        assert_eq!(
            error("e.md", 7),
            "e.md:7: an absolute home-folder path names the user of the machine that wrote it, and the repository is public: write the home folder as `$HOME`",
            "the words a contributor reads"
        );
    }

    #[test]
    fn the_check_reads_the_repository_and_finds_nothing() {
        let errors = check(&crate::repo_root()).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
