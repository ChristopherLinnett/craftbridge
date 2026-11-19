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

/// Is `root` the top of a git checkout? A `.git` folder in the main checkout, a `.git` file in a linked
/// worktree; a scratch tree has neither.
pub(crate) fn is_checkout(root: &Path) -> bool {
    root.join(".git").exists()
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
}
