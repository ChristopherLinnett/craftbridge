//! Every git command the gate runs, in its checks and in their tests, started one way.
//!
//! Git exports its repository variables (`GIT_DIR`, `GIT_INDEX_FILE` and the rest of
//! `git rev-parse --local-env-vars`) to a hook and to everything a hook, `git rebase --exec` or
//! `git bisect run` starts. A command that then runs `git -C <folder>` acts on the repository those
//! variables name, with `<folder>` taken as the top of its work tree: in a hook of a linked worktree a
//! check would read that worktree's index as if `android/` were the root, and a test's
//! `git init` and `git add` in a scratch folder would set `core.bare` in the shared configuration and
//! stage the scratch files into that index. So every command here starts with those variables removed and
//! finds its repository from the folder it is given, whoever started the gate.
//!
//! The same list is also how the environment hands git configuration of its own: it names `GIT_CONFIG`,
//! `GIT_CONFIG_PARAMETERS` and `GIT_CONFIG_COUNT`, and with `GIT_CONFIG_COUNT` set git reads every
//! `GIT_CONFIG_KEY_n`/`GIT_CONFIG_VALUE_n` pair as if it had been written with `-c`, above every file,
//! where `include.path` can even pull in a file the machine chose. Those pairs are not listed, being
//! counted, but git reads none of them without the count, so removing the listed names leaves a command
//! with nothing but the configuration of the repository it was pointed at — which is what lets a check
//! that must not read the machine's git configuration, such as the upstream cache
//! (`rust/xtask/src/upstream.rs`), rely on this one removal instead of another.
//!
//! `core.precomposeunicode=false` makes git report every path as it is stored. With the macOS default, git
//! reports a name stored decomposed in its composed form, so a check that compares git's names with a
//! directory listing would not find that name and would treat it as something git does not hold.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

/// The repository variables git itself lists, asked once. Git answers from its own build, whatever the
/// environment holds, so the answer is right for whichever git the gate runs.
pub(crate) fn local_env_vars() -> &'static [String] {
    static VARIABLES: OnceLock<Vec<String>> = OnceLock::new();
    VARIABLES.get_or_init(|| {
        Command::new("git")
            .args(["rev-parse", "--local-env-vars"])
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).lines().map(str::to_string).filter(|name| !name.is_empty()).collect())
            .unwrap_or_default()
    })
}

/// `git -C <dir>`, with git's repository variables removed, paths reported as stored, and every path the
/// gate passes taken literally: a file name holding `*` or `[` names that file, not a pattern.
pub(crate) fn command(dir: &Path) -> Command {
    let mut git = Command::new("git");
    for variable in local_env_vars() {
        git.env_remove(variable);
    }
    git.arg("-c").arg("core.precomposeunicode=false").arg("--literal-pathspecs").arg("-C").arg(dir);
    git
}

/// Run a git command in `dir` and return what it printed, or git's own words when it fails.
pub(crate) fn output(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    run(command(dir), args)
}

/// Run `git`, a [`command`] its caller has set up further, with `args`, and return what it printed, or
/// git's own words when it fails.
pub(crate) fn run(git: Command, args: &[&str]) -> Result<Vec<u8>, String> {
    run_judged(git, args, |output| output.status.success())
}

/// Run `git` as [`run`] does, for a command whose exit alone does not tell whether it answered: return what
/// it printed when `answered` says it did, or git's own words when it did not.
pub(crate) fn run_judged(mut git: Command, args: &[&str], answered: impl Fn(&Output) -> bool) -> Result<Vec<u8>, String> {
    let output = git.args(args).output().map_err(|error| format!("could not run git: {error}"))?;
    if answered(&output) { Ok(output.stdout) } else { Err(format!("`git {}` failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim())) }
}

/// The failure a check reports when `git` (a [`command`] its caller has set up) is run with `args`: the
/// command, then what git itself said on its error output. Git's words are taken from the git that runs,
/// since its wording changes between releases; the rest is written out here rather than taken from
/// [`run_judged`], so a check that lost or rewrote git's words is held to git's, never to its own report.
/// A command that says nothing there is no failure to compare with; its exit is the caller's to judge,
/// since `git grep` names a file it could not read there whether it exits 0 or 1.
#[cfg(test)]
pub(crate) fn said(mut git: Command, args: &[&str]) -> String {
    let output = git.args(args).output().unwrap();
    let said = String::from_utf8_lossy(&output.stderr).trim().to_string();
    assert!(!said.is_empty(), "`git {}` said nothing on its error output", args.join(" "));
    format!("`git {}` failed: {said}", args.join(" "))
}

/// The paths a `-z` listing printed, one per entry. The NUL separator leaves every name as stored, with no
/// quoting of the bytes git would otherwise escape.
pub(crate) fn listed(dir: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    Ok(String::from_utf8_lossy(&output(dir, args)?).split('\0').filter(|path| !path.is_empty()).map(str::to_string).collect())
}

/// The path git lists `path` by inside `dir`: its components below `dir` joined by `/`, each exactly as
/// the disk names it. A message's path is no question to put to git: it reads the `\` a Unix name may hold
/// as a separator and replaces what is not UTF-8, so git would be asked about a file that is not there and
/// list nothing for it. None when `path` is not inside `dir` or a component is not UTF-8, which no
/// question in git's listing words can name.
fn listed_as(dir: &Path, path: &Path) -> Option<String> {
    let parts: Option<Vec<&str>> = path.strip_prefix(dir).ok()?.components().map(|part| part.as_os_str().to_str()).collect();
    parts.map(|parts| parts.join("/"))
}

/// Is `root` the top of a git checkout? A `.git` folder in the main checkout, a `.git` file in a linked
/// worktree; a scratch tree has neither.
pub(crate) fn is_checkout(root: &Path) -> bool {
    root.join(".git").exists()
}

/// The listing of what the repository holds below a folder, as git answers it: every path its index
/// tracks, and every untracked one its ignore rules (each `.gitignore`, the checkout's `info/exclude` and
/// the user's `core.excludesFile`) leave to `git add`, one listing for both. A tracked file a pattern
/// matches is in it, since git keeps what it tracks; a file git ignores is not (AGENTS.md §1).
pub(crate) const TRACKED_OR_ADDABLE: &[&str] = &["ls-files", "-z", "--cached", "--others", "--exclude-standard"];

/// What the repository holds below `dir` ([`TRACKED_OR_ADDABLE`]), as paths inside it, or git's own words
/// when it cannot answer.
pub(crate) fn tracked_or_addable(dir: &Path) -> Result<Vec<String>, String> {
    listed(dir, TRACKED_OR_ADDABLE)
}

/// What an `ls-files` listing holds: every path it lists, and every folder above one of them, so a
/// folder's name is held exactly when the listing holds some path inside it — a folder holds what its
/// files are, and not what a sibling whose name begins with its own holds.
struct Held(std::collections::HashSet<String>);

impl Held {
    fn of(paths: Vec<String>) -> Held {
        let mut held = std::collections::HashSet::new();
        for path in paths {
            for boundary in path.bytes().enumerate().filter_map(|(at, byte)| (byte == b'/').then_some(at)) {
                // `get` rather than an index (§3): a `/` byte is a whole-character boundary by itself,
                // so `get` cuts where an index would too, and refuses instead of panicking if it ever did not.
                if let Some(ancestor) = path.get(..boundary) {
                    held.insert(ancestor.to_string());
                }
            }
            held.insert(path);
        }
        Held(held)
    }

    fn holds(&self, name: &str) -> bool {
        self.0.contains(name)
    }
}

/// The names in `names` of a directory `dir` that hold nothing the repository can: neither a tracked file
/// nor an untracked one git would add. A name is a path inside `dir` as git lists it (`listed_as`) — a
/// folder's own name, or a file's whole path at any depth — and it counts as one git can list when the
/// listing prints it itself, or prints anything under it at a folder boundary. Those are no part of the
/// repository — an editor's settings folder a contributor ignores locally, a tool's cache, the folder a
/// staged release stands in, a folder whose every file is ignored, and an empty folder, which git cannot
/// hold — so no rule about what the repository holds applies to them (AGENTS.md §1). A tracked file
/// always counts, so a file forced past the ignore rules is still the repository's. The question costs one
/// full listing ([`TRACKED_OR_ADDABLE`]), read whole and answered here: every name git itself printed
/// matches, however its folder is spelt, without a pathspec having to match a name git may spell its own
/// way, and with no list of names to pass that a large tree would make longer than a command line holds.
///
/// Git is asked only where a checkout stands, at `dir` or above it (`is_checkout`): there a git that
/// cannot answer is an error in its own words, since a check must not read everything as tracked behind
/// it. Whether a checkout stands there is the tree's answer, not git's: a git that cannot start, or a
/// `.git` naming a git folder that is gone, says nothing about where it is. Outside every checkout — a
/// scratch tree a test builds — there is no git to ask, the folder's own files stand, and nothing is held
/// back.
fn git_ignored(dir: &Path, names: &[String]) -> Result<Vec<String>, String> {
    if names.is_empty() || !dir.ancestors().any(is_checkout) {
        // Nothing is asked, or there is no git to ask it of, so nothing is taken for ignored — and no
        // listing is started to answer an empty question, which a check's clean run must not pay for.
        return Ok(Vec::new());
    }
    let held = Held::of(tracked_or_addable(dir)?);
    Ok(names.iter().filter(|name| !held.holds(name)).cloned().collect())
}

/// Those of `found` that are the repository's: each is a path inside `dir` (`path_of`), and every one that
/// holds nothing git would add is dropped, all of them asked in one question ([`git_ignored`]), each by
/// its listing path. One no listing path can name — a component that is not text — is no question git can
/// answer, so it stays, and the rule that reads it refuses it rather than git's silence passing it.
pub(crate) fn in_repository<T>(dir: &Path, found: Vec<T>, path_of: impl Fn(&T) -> &Path) -> Result<Vec<T>, String> {
    let asked: Vec<Option<String>> = found.iter().map(|item| listed_as(dir, path_of(item))).collect();
    let ignored: std::collections::HashSet<String> = git_ignored(dir, &asked.iter().flatten().cloned().collect::<Vec<_>>())?.into_iter().collect();
    Ok(found.into_iter().zip(asked).filter(|(_, name)| name.as_deref().is_none_or(|name| !ignored.contains(name))).map(|(item, _)| item).collect())
}

/// A bare scratch repository whose `origin` is `url`, holding nothing yet: the upstream cache a test
/// builds its revisions in.
#[cfg(test)]
pub(crate) fn bare_scratch(tag: &str, url: &str) -> crate::scratch::Scratch {
    let repository = crate::scratch::Scratch::new(tag);
    output(&repository.0, &["init", "--quiet", "--bare"]).unwrap();
    output(&repository.0, &["--git-dir=.", "remote", "add", "origin", url]).unwrap();
    repository
}

/// A commit in the bare repository at `repository` holding exactly `files`, each a path and its text, made
/// with git's plumbing from an index of its own, since a bare repository has no work tree to add from;
/// with no files it holds the empty tree.
#[cfg(test)]
pub(crate) fn commit_files(repository: &Path, parent: Option<&str>, message: &str, files: &[(&str, &str)]) -> String {
    let blobs = crate::scratch::Scratch::new("git-commit");
    let with_index = || {
        let mut git = command(repository);
        git.env("GIT_INDEX_FILE", blobs.0.join("index"));
        git
    };
    for (number, (name, text)) in files.iter().enumerate() {
        let blob = blobs.0.join(format!("blob-{number}"));
        std::fs::write(&blob, text).unwrap();
        let hash = String::from_utf8(output(repository, &["--git-dir=.", "hash-object", "-w", &blob.to_string_lossy()]).unwrap()).unwrap();
        run(with_index(), &["--git-dir=.", "update-index", "--add", "--cacheinfo", &format!("100644,{},{name}", hash.trim())]).unwrap();
    }
    let tree = String::from_utf8(run(with_index(), &["--git-dir=.", "write-tree"]).unwrap()).unwrap();
    let mut args = vec!["--git-dir=.", "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit-tree", tree.trim(), "-m", message];
    if let Some(parent) = parent {
        args.extend(["-p", parent]);
    }
    String::from_utf8(output(repository, &args).unwrap()).unwrap().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn every_repository_variable_git_lists_is_removed() {
        let names = local_env_vars();
        // The last three are git's configuration carriers, not repository variables: a check that reads no
        // machine configuration (rust/xtask/src/upstream.rs) depends on their removal as much as the rest.
        for expected in
            ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY", "GIT_CONFIG", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG_COUNT"]
        {
            assert!(names.iter().any(|name| name == expected), "{expected} is one git lists: {names:?}");
        }
        let git = command(Path::new("."));
        let removed: Vec<&std::ffi::OsStr> = git.get_envs().filter(|(_, value)| value.is_none()).map(|(name, _)| name).collect();
        for name in names {
            assert!(removed.iter().any(|removed| *removed == name.as_str()), "{name} is not removed");
        }
        let args: Vec<&std::ffi::OsStr> = git.get_args().collect();
        assert_eq!(&args[..3], ["-c", "core.precomposeunicode=false", "--literal-pathspecs"]);
    }

    /// The child half of the test below: run only when the parent starts it with a hook's environment.
    #[test]
    #[ignore = "run by a_command_started_from_a_hook_acts_on_its_own_folder, in a hook's environment"]
    fn child_stages_a_file_in_its_own_folder() {
        let Some(own) = std::env::var_os("XTASK_GIT_OWN") else { return };
        let own = Path::new(&own);
        assert!(std::env::var_os("GIT_DIR").is_some(), "the parent sets the hook's GIT_DIR");
        output(own, &["add", "--", "planted.txt"]).unwrap();
        let toplevel = String::from_utf8(output(own, &["rev-parse", "--show-toplevel"]).unwrap()).unwrap();
        assert_eq!(Path::new(toplevel.trim()).canonicalize().unwrap(), own.canonicalize().unwrap());
    }

    #[test]
    fn a_command_started_from_a_hook_acts_on_its_own_folder() {
        let own = Scratch::new("git-own");
        let other = Scratch::new("git-other");
        output(&own.0, &["init", "-q"]).unwrap();
        output(&other.0, &["init", "-q"]).unwrap();
        own.write("planted.txt", "x\n");
        craftbridge_process::rerun(
            "git::tests::child_stages_a_file_in_its_own_folder",
            &[("GIT_DIR", other.0.join(".git").as_os_str()), ("XTASK_GIT_OWN", own.0.as_os_str())],
            "the command started from a hook did not act on its own folder",
        )
        .unwrap();
        assert_eq!(listed(&own.0, &["ls-files", "-z"]).unwrap(), ["planted.txt"], "the file is staged where the command was told to act");
        assert!(listed(&other.0, &["ls-files", "-z"]).unwrap().is_empty(), "the hook's repository is untouched");
        assert_eq!(String::from_utf8(output(&other.0, &["config", "core.bare"]).unwrap()).unwrap().trim(), "false");
    }

    #[test]
    fn a_decomposed_name_comes_back_as_stored() {
        let repository = Scratch::new("git-nfd");
        output(&repository.0, &["init", "-q"]).unwrap();
        let decomposed = "e\u{301}tude";
        repository.write(&format!("{decomposed}/notes.md"), "x\n");
        let listed = listed(&repository.0, &["ls-files", "-z", "--others", "--exclude-standard"]).unwrap();
        assert_eq!(listed, [format!("{decomposed}/notes.md")]);
    }

    /// The path a check puts to git is the file's own components below the folder joined by `/`, each
    /// exactly as the disk names it: a `\` stays inside a name, a name git would quote or escape comes
    /// whole, and a component that is no text has no listing path at all, so a check asks nothing about
    /// such a file and refuses it rather than read git's silence for an answer. The last two paths are
    /// bytes only, never a folder made: an APFS volume answers `Illegal byte sequence` for a name that is
    /// no text, so nothing but the bytes themselves shows what happens to one.
    #[cfg(unix)]
    #[test]
    fn a_listing_path_names_each_component_as_the_disk_does() {
        use std::os::unix::ffi::OsStrExt;
        let tree = Path::new("/tree");
        let file = |name: &str| tree.join(name).join(".swift-format");
        let names = ["kept\\here", "trailing\\", "with space", "trail ", "ünïcode/日本", "e\u{301}tude", "quo\"te\nnewline", "[ab]", "*star"];
        for name in names {
            assert_eq!(listed_as(tree, &file(name)).as_deref(), Some(format!("{name}/.swift-format").as_str()), "{name:?}");
        }
        assert_eq!(listed_as(tree, &tree.join("rust").join("core").join("clippy.toml")).as_deref(), Some("rust/core/clippy.toml"), "a path at any depth");
        assert_eq!(listed_as(&tree.join("rust"), &file("kept\\here")), None, "a path outside the folder has no listing path in it");
        let no_text = std::ffi::OsStr::from_bytes(b"not\xffutf8");
        assert_eq!(listed_as(tree, &tree.join(no_text).join(".swift-format")), None, "a component that is no text has no listing path");
    }

    #[test]
    fn a_failing_command_names_git_s_error_and_a_scratch_tree_is_no_checkout() {
        // A `.git` naming a folder that is not there: git's words name the tree, so the ones the failure is
        // held to can only be this tree's.
        let broken = Scratch::new("git-broken");
        broken.write(".git", "gitdir: does-not-exist\n");
        let error = output(&broken.0, &["ls-files"]).unwrap_err();
        assert_eq!(error, said(command(&broken.0), &["ls-files"]));
        let scratch = Scratch::new("git-none");
        assert!(!is_checkout(&scratch.0));
        assert!(is_checkout(&crate::repo_root()));
    }

    /// Git is asked only where a checkout stands, at the folder asked about or above it: a scratch tree with
    /// none is its own files, and nothing is held back. A checkout whose git cannot answer refuses in git's
    /// own words, from its top and from a folder inside it: a `.git` naming a git folder that is gone, as a
    /// linked worktree's does once its repository has moved, and an index that is no index. Reading every
    /// file as tracked there is what the refusal exists to stop.
    #[test]
    fn git_is_asked_only_in_a_checkout_and_one_that_cannot_answer_refuses() {
        let plain = Scratch::new("git-ignored-scratch");
        plain.write("planted/.swift-format", "{}\n");
        assert_eq!(git_ignored(&plain.0, &["planted/.swift-format".to_string()]).unwrap(), Vec::<String>::new(), "a scratch tree asks git nothing");
        let dangling = Scratch::new("git-ignored-dangling");
        dangling.write(".git", "gitdir: does-not-exist\n");
        let unindexed = Scratch::new("git-ignored-unindexed");
        output(&unindexed.0, &["init", "-q"]).unwrap();
        unindexed.write(".git/index", "no index here");
        for broken in [&dangling, &unindexed] {
            broken.write("planted/.swift-format", "{}\n");
            for dir in [broken.0.clone(), broken.0.join("planted")] {
                let error = git_ignored(&dir, &[".swift-format".to_string()]).unwrap_err();
                assert_eq!(error, said(command(&dir), TRACKED_OR_ADDABLE), "the refusal names git's own failure, asked in {}", dir.display());
            }
        }
    }

    /// The child half of the test below: run only when the parent starts it with no git to start.
    #[test]
    #[ignore = "run by a_git_that_cannot_start_refuses_in_a_checkout_and_is_never_started_outside_one, with no git on PATH"]
    fn child_asks_with_no_git_to_start() {
        let (Some(checkout), Some(plain)) = (std::env::var_os("XTASK_GIT_CHECKOUT"), std::env::var_os("XTASK_GIT_PLAIN")) else { return };
        let asked = ["kept.rs".to_string()];
        let error = git_ignored(Path::new(&checkout), &asked).unwrap_err();
        assert!(error.starts_with("could not run git: "), "{error}");
        assert_eq!(git_ignored(Path::new(&plain), &asked).unwrap(), Vec::<String>::new(), "outside a checkout no git is started");
    }

    /// A git that cannot even start says nothing about where it is, so inside a checkout its silence is a
    /// refusal like any other failure, and outside one no git is started at all.
    #[test]
    fn a_git_that_cannot_start_refuses_in_a_checkout_and_is_never_started_outside_one() {
        let checkout = Scratch::new("git-ignored-no-git");
        output(&checkout.0, &["init", "-q"]).unwrap();
        checkout.write("kept.rs", "// x\n");
        let plain = Scratch::new("git-ignored-no-git-plain");
        plain.write("kept.rs", "// x\n");
        let no_git = Scratch::new("git-ignored-empty-path");
        craftbridge_process::rerun(
            "git::tests::child_asks_with_no_git_to_start",
            &[("PATH", no_git.0.as_os_str()), ("XTASK_GIT_CHECKOUT", checkout.0.as_os_str()), ("XTASK_GIT_PLAIN", plain.0.as_os_str())],
            "a git that could not start was read as an answer",
        )
        .unwrap();
    }
}
