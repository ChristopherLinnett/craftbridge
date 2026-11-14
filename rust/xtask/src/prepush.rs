//! The pre-push hook, proved where only it can be proved: in scratch repositories, never in the one whose
//! hooks it installs. A scratch repository of its own, with a scratch bare remote of its own, is made by
//! the gate's git helpers and driven by a real `git push` — the only way a real git runs a real hook with
//! a real stdin of refs. The hook under test is this repository's own `tools/hooks/pre-push`, copied in so
//! the proof judges the file that ships. Each scratch repository commits a stub `tools/ci.sh` in place of
//! the real gate: the real gate belongs to this repository and its hours of building, not to a proof that
//! only needs to know what the gate was handed and whether it passed. The stub fails when the tree it
//! gates holds `ci-fails`, which is how a commit is made to fail its own gate, and it writes one line per
//! run to a log per push, so a test can count the commits a push gated. Each acceptance line of the
//! hook's contract is one test here, and so is each property its design rests on.

#[cfg(test)]
mod tests {
    use crate::git::{command, output};
    use crate::repo_root;
    use crate::scratch::Scratch;
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant, SystemTime};

    /// The stub gate a scratch repository commits in place of this repository's `tools/ci.sh`. It judges
    /// its own tree — the pushed commit's checkout the hook ran it from, never the tree the push came
    /// from — and logs what the hook handed it: the commit its checkout stands at, the build folder, any
    /// of git's repository variables, and the bytes it could read from its stdin. In the build folder it marks that it is running, so two
    /// gates in it at once are caught, and leaves an artifact as it ends; it keeps the times of its own
    /// checkout and of the artifact the gate before it left. `GATE_HOLD` holds the push's first gate until
    /// the test lets it go, and makes a signal end that gate with an ordinary failing exit, as a JVM's
    /// shutdown does, letting go of the running marker on the way out so a gate that died mid-signal
    /// leaves no later gate to believe it overlapped; `GATE_SLEEP` makes every gate slow; `GATE_DIRTY`
    /// leaves a file standing in the checkout the gate runs in, as a real gate's build output does.
    const STUB_GATE: &str = r#"#!/bin/sh
tree="$(cd "$(dirname "$0")/.." && pwd -P)"
target="${CRAFTBRIDGE_PUSH_TARGET:-}"
leaked=""
for name in $(git rev-parse --local-env-vars); do
  if eval "[ -n \"\${$name+set}\" ]"; then leaked="$leaked $name"; fi
done
stdin="$(wc -c | tr -d ' ')"
head="$(git -C "$tree" rev-parse HEAD)"
if [ -n "${GATE_DIRTY:-}" ]; then : >"$tree/gate-dirt"; fi
overlap=0
inside=0
if [ -n "$target" ]; then
  mkdir -p "$target"
  if mkdir "$target/gate-running" 2>/dev/null; then inside=1; else overlap=1; fi
  if [ -e "$target/artifact" ]; then cp -p "$target/artifact" "$GATE_LOG.$$.artifact"; fi
  cp -p "$0" "$GATE_LOG.$$.checkout"
fi
if [ -n "${GATE_HOLD:-}" ] && [ ! -e "$GATE_HOLD" ]; then
  trap 'if [ "$inside" = 1 ]; then rmdir "$target/gate-running" 2>/dev/null; fi; exit 1' HUP INT TERM
  printf '%s %s\n' "$PPID" "$tree" >"$GATE_HOLD.tmp" && mv "$GATE_HOLD.tmp" "$GATE_HOLD"
  n=0
  while [ ! -e "$GATE_HOLD.go" ] && [ "$n" -lt 600 ]; do sleep 0.1; n=$((n + 1)); done
fi
sleep "${GATE_SLEEP:-0}"
printf 'target=%s\ttree=%s\tleaked=%s\tstdin=%s\toverlap=%s\thead=%s\tid=%s\n' "${target:-none}" "$tree" "$leaked" "$stdin" "$overlap" "$head" "$$" >>"$GATE_LOG"
if [ -n "$target" ]; then
  : >"$target/artifact"
  if [ "$inside" = 1 ]; then rmdir "$target/gate-running"; fi
fi
[ ! -e "$tree/ci-fails" ]
"#;

    /// How long a test waits for a push, or a hook, before it calls the hook stuck.
    const PATIENCE: Duration = Duration::from_secs(60);

    /// The words a hook prints when it waits for another push's gate.
    const WAITING: &str = "waiting for the gate of another push";

    /// One gate run, as the stub logged it.
    struct Run {
        fields: BTreeMap<String, String>,
        log: PathBuf,
    }

    impl Run {
        fn field(&self, name: &str) -> &str {
            self.fields.get(name).map(String::as_str).unwrap_or_default()
        }

        /// The build folder the hook handed the gate, canonicalised; `None` when it handed none.
        fn target(&self) -> Option<PathBuf> {
            Some(self.field("target")).filter(|target| *target != "none").map(|target| std::fs::canonicalize(target).unwrap())
        }

        /// When this run's checkout was written, and when the artifact the gate before it left was.
        fn checkout_and_previous_artifact(&self) -> (SystemTime, Option<SystemTime>) {
            let at =
                |suffix: &str| std::fs::metadata(format!("{}.{}.{suffix}", self.log.display(), self.field("id"))).ok().map(|meta| meta.modified().unwrap());
            (at("checkout").unwrap(), at("artifact"))
        }
    }

    /// One real `git push`: whether it succeeded, the gate runs it caused, and git's and the hook's words.
    struct Push {
        ok: bool,
        runs: Vec<Run>,
        why: String,
    }

    /// A `git push` under way, in a process group of its own, so a test can signal the whole push the way
    /// a terminal does.
    struct Pushing {
        child: Child,
        log: PathBuf,
        stderr: PathBuf,
    }

    impl Pushing {
        fn finish(mut self) -> Push {
            let deadline = Instant::now() + PATIENCE;
            let status = loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    break status;
                }
                if Instant::now() > deadline {
                    signal_group(self.child.id(), "KILL");
                    panic!("the push did not finish: {}", std::fs::read_to_string(&self.stderr).unwrap_or_default());
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            Push { ok: status.success(), runs: runs_in(&self.log), why: std::fs::read_to_string(&self.stderr).unwrap().trim().to_string() }
        }
    }

    /// A push a test leaves unfinished — an assertion failed while it ran — is killed with its whole
    /// group, so no push, hook or gate outlives the test that started it, waiting on a scratch repository
    /// that is gone.
    impl Drop for Pushing {
        fn drop(&mut self) {
            if let Ok(None) = self.child.try_wait() {
                signal_group(self.child.id(), "KILL");
                let _ = self.child.wait();
            }
        }
    }

    /// The gate runs a push's log holds, one per line.
    fn runs_in(log: &Path) -> Vec<Run> {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .map(|line| Run {
                fields: line.split('\t').filter_map(|field| field.split_once('=')).map(|(name, value)| (name.to_string(), value.to_string())).collect(),
                log: log.to_path_buf(),
            })
            .collect()
    }

    /// Send `signal` to every process in the group `group` leads, as a terminal sends Ctrl-C.
    fn signal_group(group: u32, signal: &str) {
        let _ = Command::new("kill").args(["-s", signal, "--", &format!("-{group}")]).status();
    }

    fn running(pid: &str) -> bool {
        Command::new("kill").args(["-0", pid]).stderr(Stdio::null()).status().unwrap().success()
    }

    /// Wait for `done` to hold, or call the hook stuck.
    fn wait_until(what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done() {
            assert!(Instant::now() < deadline, "gave up waiting: {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A scratch repository holding the hook under test, a stub gate, and a bare remote to push to.
    struct ScratchRepo {
        scratch: Scratch,
        work: PathBuf,
        pushes: std::cell::Cell<u32>,
    }

    impl ScratchRepo {
        /// A scratch repository whose first commit holds the hook under test and the stub gate.
        fn new(tag: &str) -> Self {
            let prover = Self::bare(tag);
            let work = &prover.work;
            // The hook under test is this repository's own file, copied in executable; the stub gate
            // stands where the real gate stands in the commit.
            std::fs::create_dir_all(work.join("tools/hooks")).unwrap();
            let hook = work.join("tools/hooks/pre-push");
            std::fs::copy(repo_root().join("tools/hooks/pre-push"), &hook).unwrap();
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
            let gate = work.join("tools/ci.sh");
            std::fs::write(&gate, STUB_GATE).unwrap();
            std::fs::set_permissions(&gate, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::write(work.join("note.txt"), "a clean commit\n").unwrap();
            prover.commit("first");
            prover
        }

        /// A scratch repository whose first commit holds a copy of this repository's own files: every one
        /// git tracks or would add, as they stand, the hook under test and the real gate among them.
        fn holding_this_repository(tag: &str) -> Self {
            let prover = Self::bare(tag);
            for file in crate::git::listed(&repo_root(), &["ls-files", "-z", "--cached", "--others", "--exclude-standard"]).unwrap() {
                let to = prover.work.join(&file);
                std::fs::create_dir_all(to.parent().unwrap()).unwrap();
                std::fs::copy(repo_root().join(&file), &to).unwrap();
            }
            prover
        }

        /// The scratch repository, its identity and its bare remote, holding nothing yet.
        fn bare(tag: &str) -> Self {
            let scratch = Scratch::new(tag);
            let work = scratch.0.join("work");
            let remote = scratch.0.join("remote.git");
            output(&scratch.0, &["init", "--bare", "-q", "remote.git"]).unwrap();
            // The remote's HEAD names a branch no proof ever pushes: a bare remote refuses the deletion
            // of its current branch, which would hide what a delete-only push does.
            output(&scratch.0, &["-C", "remote.git", "symbolic-ref", "HEAD", "refs/heads/nothing-here"]).unwrap();
            output(&scratch.0, &["init", "-q", "-b", "main", "work"]).unwrap();
            // Every setting is repository-local and made here: the proof reads nothing of the machine it
            // runs on — no global identity, no global signing rule, no hooks but these.
            for setting in [
                ["user.name", "Scratch Prover"],
                ["user.email", "prover@example.invalid"],
                ["commit.gpgsign", "false"],
                ["tag.gpgsign", "false"],
                // The setting `tools/doctor.sh --install-hooks` makes here: relative, so every worktree
                // of the repository runs its own `tools/hooks/pre-push`.
                ["core.hooksPath", "tools/hooks"],
            ] {
                output(&work, &["config", setting[0], setting[1]]).unwrap();
            }
            output(&work, &["remote", "add", "origin", remote.to_str().unwrap()]).unwrap();
            ScratchRepo { scratch, work, pushes: std::cell::Cell::new(0) }
        }

        /// Put `ci-fails` in `dir`: whichever commit commits that file fails the stub gate.
        fn plant_failure(&self, dir: &Path) {
            std::fs::write(dir.join("ci-fails"), "a file that fails the gate\n").unwrap();
        }

        fn write(&self, dir: &Path, name: &str, text: &str) {
            std::fs::write(dir.join(name), text).unwrap();
        }

        fn commit(&self, message: &str) {
            output(&self.work, &["add", "-A"]).unwrap();
            output(&self.work, &["commit", "-q", "-m", message]).unwrap();
        }

        /// Run git in the checkout with the committer's date set, so a commit can be dated against the
        /// shape of the history it stands in.
        fn git_dated(&self, args: &[&str], when: &str) {
            let run = command(&self.work).env("GIT_COMMITTER_DATE", when).args(args).output().unwrap();
            assert!(run.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&run.stderr));
        }

        fn git(&self, args: &[&str]) -> String {
            String::from_utf8(output(&self.work, args).unwrap()).unwrap().trim().to_string()
        }

        /// The remote's refs, as `git ls-remote` gives them: what a push did or failed to do.
        fn remote_refs(&self) -> String {
            self.git(&["ls-remote", "origin"])
        }

        /// The paths git has registered as this repository's worktrees, canonicalised where they still
        /// exist. The hook's temporary worktrees must be registered while it runs and gone by the time the
        /// push ends.
        fn registered_worktrees(&self) -> Vec<PathBuf> {
            let listing = self.git(&["worktree", "list", "--porcelain"]);
            listing
                .lines()
                .filter_map(|line| line.strip_prefix("worktree "))
                .map(|path| std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path)))
                .collect()
        }

        fn expected_worktrees(&self) -> Vec<PathBuf> {
            vec![std::fs::canonicalize(&self.work).unwrap()]
        }

        /// The folder the hook keeps for this repository in its git folder, shared by every worktree.
        fn hook_folder(&self) -> PathBuf {
            std::fs::canonicalize(self.work.join(".git")).unwrap().join("craftbridge-pre-push")
        }

        /// Is a push's lock left behind? A dangling symbolic link exists for `symlink_metadata` alone.
        fn lock_is_held(&self) -> bool {
            std::fs::symlink_metadata(self.hook_folder().join("lock")).is_ok()
        }

        /// Start a real `git push` from `dir`, the way a person pushes: git runs the hook and the hook
        /// gates the pushed commits. Each push gets its own gate log, so gate runs are counted per push.
        fn start(&self, dir: &Path, args: &[&str], env: &[(&str, &Path)]) -> Pushing {
            let stderr = self.scratch.0.join(format!("push-{}.stderr", self.pushes.get()));
            let into = std::fs::File::create(&stderr).unwrap();
            self.spawn(dir, args, env, into.into(), stderr)
        }

        /// The push `start` makes, its stderr going to `into`: a file `stderr` names, or a pipe a test
        /// may close, as a client that stops reading does.
        fn spawn(&self, dir: &Path, args: &[&str], env: &[(&str, &Path)], into: Stdio, stderr: PathBuf) -> Pushing {
            let n = self.pushes.get();
            self.pushes.set(n + 1);
            let log = self.scratch.0.join(format!("gate-run-{n}.log"));
            let mut git = command(dir);
            git.env("GATE_LOG", &log).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(into).process_group(0);
            for (name, value) in env {
                git.env(name, value);
            }
            Pushing { child: git.spawn().unwrap(), log, stderr }
        }

        fn push(&self, dir: &Path, args: &[&str]) -> Push {
            self.start(dir, args, &[]).finish()
        }

        /// Push `main` clean, so later pushes carry only what they add.
        fn push_main(&self) {
            let push = self.push(&self.work, &["push", "-q", "origin", "main"]);
            assert!(push.ok, "the clean first push: {}", push.why);
        }

        /// Start a push of `main`, in `env` beside the hold, whose gate holds until `let_go(hold)`, and wait
        /// until it holds: the push, the pid of its hook, and the temporary worktree its gate runs in.
        fn start_held(&self, hold: &Path, env: &[(&str, &Path)]) -> (Pushing, String, PathBuf) {
            let _ = std::fs::remove_file(hold.with_extension("go"));
            let _ = std::fs::remove_file(hold);
            let mut env = env.to_vec();
            env.push(("GATE_HOLD", hold));
            let pushing = self.start(&self.work, &["push", "-q", "origin", "main"], &env);
            let (hook, tree) = held(hold);
            (pushing, hook, tree)
        }
    }

    /// Wait until a gate holds at `hold`, and read what it wrote there: the pid of its hook, and the
    /// temporary worktree it runs in.
    fn held(hold: &Path) -> (String, PathBuf) {
        wait_until("the gate to start", || hold.exists());
        let held = std::fs::read_to_string(hold).unwrap();
        let (hook, tree) = held.trim().split_once(' ').unwrap();
        (hook.to_string(), PathBuf::from(tree))
    }

    fn let_go(hold: &Path) {
        std::fs::write(hold.with_extension("go"), "").unwrap();
    }

    /// Acceptance: an untracked file that would not be pushed no longer fails the hook.
    #[test]
    fn an_untracked_file_that_is_no_part_of_the_push_does_not_gate_the_push() {
        let prover = ScratchRepo::new("prepush-untracked");
        // A working tree that would fail a gate run on it — but nothing of it is in the commit.
        prover.plant_failure(&prover.work);
        let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
        assert!(push.ok, "the untracked file is no part of the push, so the push stands: {}", push.why);
        assert_eq!(push.runs.len(), 1, "the push's own commit was gated, once, not the tree");
        assert!(prover.remote_refs().contains("refs/heads/main"), "the clean commit reached the remote");
        assert!(prover.work.join("ci-fails").exists(), "and nothing of the untracked working tree was touched");
        assert_eq!(prover.registered_worktrees(), prover.expected_worktrees(), "the hook's temporary worktree is gone with the push");
        let tree = Path::new(push.runs[0].field("tree"));
        assert!(!tree.exists() && !tree.parent().unwrap().exists(), "and so is the temporary folder it sat in");
        assert!(!prover.lock_is_held(), "the push let its lock go");
    }

    /// Acceptance: a pushed commit that fails the gate is refused while the checkout holds an
    /// uncommitted fix.
    #[test]
    fn a_pushed_commit_that_fails_the_gate_is_refused_while_the_checkout_holds_an_uncommitted_fix() {
        let prover = ScratchRepo::new("prepush-refused");
        prover.plant_failure(&prover.work);
        prover.commit("a commit that fails the gate");
        // The fix, uncommitted: the failure is gone from the working tree, but the pushed commit still
        // holds it. A hook that gated the tree would let this push through; this one judges the commit.
        std::fs::remove_file(prover.work.join("ci-fails")).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
        assert!(!push.ok, "the commit fails its gate, and an uncommitted fix beside it is no part of the judgement");
        assert_eq!(push.runs.len(), 2, "the gate ran on each commit the push carries, not on the tree");
        assert!(!prover.remote_refs().contains("refs/heads/main"), "the refused push left the remote untouched");
        assert_eq!(prover.registered_worktrees(), prover.expected_worktrees(), "the temporary worktree goes after a failed gate as after a passed one");
        assert!(!prover.lock_is_held(), "a refused push lets its lock go too");
    }

    /// Acceptance: pushing a branch other than the checked-out one gates that branch's commit.
    #[test]
    fn pushing_a_branch_other_than_the_checked_out_one_gates_that_branch_s_commit() {
        let prover = ScratchRepo::new("prepush-other-branch");
        prover.push_main();
        // `broken` carries a failing commit and moves ahead of `main`, which stays checked out and clean:
        // a hook that gated the tree would find nothing broken here at all.
        output(&prover.work, &["checkout", "-q", "-b", "broken"]).unwrap();
        prover.plant_failure(&prover.work);
        prover.commit("a commit that fails the gate");
        output(&prover.work, &["checkout", "-q", "main"]).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "broken"]);
        assert!(!push.ok, "the pushed branch's own failing commit gates the push while the checked-out main is clean: {}", push.why);
        assert_eq!(push.runs.len(), 1, "that branch's one new commit was gated, and not the commit the remote already holds");
        assert!(!prover.remote_refs().contains("refs/heads/broken"), "the refused branch stayed local");
    }

    /// Acceptance: a push that only deletes branches is still not gated.
    #[test]
    fn a_push_that_only_deletes_branches_is_not_gated() {
        let prover = ScratchRepo::new("prepush-delete-only");
        prover.push_main();
        // Now a local tip that would fail a gate, behind no push: only the deletion goes. There is no
        // commit for the hook to judge, so there is nothing to gate — even against this failing tip.
        prover.plant_failure(&prover.work);
        prover.commit("a local commit that fails the gate, unpushed");
        let push = prover.push(&prover.work, &["push", "-q", "--delete", "origin", "main"]);
        assert!(push.ok, "a push that only deletes branches carries nothing to gate: {}", push.why);
        assert!(push.runs.is_empty(), "the gate did not run at all for a deletion-only push");
        assert!(prover.remote_refs().is_empty(), "the deletion went through");
        assert_eq!(prover.registered_worktrees(), prover.expected_worktrees(), "and the ungated path leaves nothing registered");
    }

    /// Acceptance: a push from a linked worktree still leaves that worktree's index and the shared
    /// configuration untouched. Git hands a hook its repository variables, and a push may carry more of
    /// them in its own environment: the hook clears every one before its own first git command and
    /// before the gate, so neither writes an index the variables name and the gate inherits none.
    #[test]
    fn a_push_from_a_linked_worktree_leaves_its_index_and_the_shared_configuration_untouched() {
        let prover = ScratchRepo::new("prepush-linked");
        prover.push_main();
        let linked = prover.scratch.0.join("linked");
        output(&prover.work, &["worktree", "add", "-q", "-b", "side", linked.to_str().unwrap(), "main"]).unwrap();
        // Work in the linked worktree that reaches no commit: staged, unstaged, untracked. None of it may
        // move when the hook gates a push that comes from that worktree. The empty commit is the branch
        // tip the push carries; the index work lands beside it after, so it stays in the index.
        output(&linked, &["commit", "-q", "--allow-empty", "-m", "a clean commit from the linked worktree"]).unwrap();
        prover.write(&linked, "staged.txt", "staged, not committed\n");
        output(&linked, &["add", "staged.txt"]).unwrap();
        prover.write(&linked, "note.txt", "tracked, edited after the commit\n");
        prover.write(&linked, "untracked.txt", "untracked\n");
        let config_before = prover.git(&["config", "list", "--local"]);
        let stray_index = prover.scratch.0.join("stray-index");
        let push = prover.start(&linked, &["push", "-q", "origin", "side"], &[("GIT_INDEX_FILE", &stray_index)]).finish();
        assert!(push.ok, "the linked worktree's clean commit passes its own gate: {}", push.why);
        assert_eq!(push.runs.len(), 1, "the linked worktree's push gated its own commit");
        assert_eq!(push.runs[0].field("leaked").trim(), "", "the gate inherited none of git's repository variables");
        assert!(!stray_index.exists(), "no git command of the hook wrote the index the push's environment named");
        assert!(prover.remote_refs().contains("refs/heads/side"), "the side branch reached the remote");
        let status = String::from_utf8(output(&linked, &["status", "--porcelain"]).unwrap()).unwrap();
        assert!(status.contains("A  staged.txt"), "the file staged before the push is still staged: {status}");
        assert!(status.contains(" M note.txt"), "the edit unstaged before the push is still unstaged: {status}");
        assert!(status.contains("?? untracked.txt"), "the untracked file stayed untracked: {status}");
        assert_eq!(config_before, prover.git(&["config", "list", "--local"]), "the shared configuration is byte for byte what it was");
        let mut expected = prover.expected_worktrees();
        expected.push(std::fs::canonicalize(&linked).unwrap());
        assert_eq!(prover.registered_worktrees(), expected, "the hook's own worktrees are gone; the linked one it ran from stays registered");
    }

    /// The gate builds in the folder the hook names. The scratch proofs' stub gate reads the variable the
    /// hook exports directly; only this repository's own `tools/env.sh` shows what the real environment
    /// does with it, so that half of the wiring is proved here, against the real file.
    #[test]
    fn the_gate_builds_in_the_folder_the_hook_names() {
        let print_target = r#"source tools/env.sh >/dev/null 2>&1 || exit 1; printf '%s' "$CARGO_TARGET_DIR""#;
        let named = std::env::temp_dir().join("craftbridge-push-folder-named-by-a-proved-run");
        let with = Command::new("bash").current_dir(repo_root()).arg("-c").arg(print_target).env("CRAFTBRIDGE_PUSH_TARGET", &named).output().unwrap();
        assert!(with.status.success(), "sourcing tools/env.sh failed: {}", String::from_utf8_lossy(&with.stderr));
        assert_eq!(
            String::from_utf8_lossy(&with.stdout),
            named.to_str().unwrap(),
            "a gate started in the environment the hook names builds where the hook named"
        );
        let plain = Command::new("bash").current_dir(repo_root()).arg("-c").arg(print_target).env_remove("CRAFTBRIDGE_PUSH_TARGET").output().unwrap();
        assert!(plain.status.success(), "sourcing tools/env.sh failed: {}", String::from_utf8_lossy(&plain.stderr));
        assert_eq!(
            String::from_utf8_lossy(&plain.stdout),
            format!("{}/rust/target", repo_root().display()),
            "with no hook naming a folder, every tree keeps its own rust/target"
        );
    }

    /// A gated tree is a fresh checkout and holds nothing git ignores, yet the gate demands one thing that
    /// lives in the tree and that git ignores there: the plan kit's dependencies in `plan/node_modules`,
    /// which the gate's doctor and its plan stage require as `plan/package-lock.json` pins them. A copy of
    /// this repository's own files is pushed through the hook with its real gate, the real `tools/ci.sh`,
    /// run for those two stages; it is committed beside a `tools/ci.sh` that starts it so, as the hook
    /// starts every gate with no arguments. The checkout the push comes from holds an install of its own,
    /// answering to no lock: like any file git ignores there, it is no part of the judgement, and the push
    /// leaves it as it was. Every other stage needs only what the machine holds — the
    /// toolchains, npm's and Cargo's caches, the upstream cache — and a full gate here would build
    /// EffectCraft from cold on every run, so the full gate of a fresh tree was run once, by hand, instead.
    #[test]
    fn a_fresh_gated_tree_meets_the_prerequisites_of_this_repository_s_own_gate() {
        let prover = ScratchRepo::holding_this_repository("prepush-own-gate");
        let tools = prover.work.join("tools");
        std::fs::rename(tools.join("ci.sh"), tools.join("ci-of-this-repository.sh")).unwrap();
        std::fs::write(tools.join("ci.sh"), "#!/usr/bin/env bash\nexec \"$(dirname \"$0\")/ci-of-this-repository.sh\" doctor plan_views >&2\n").unwrap();
        std::fs::set_permissions(tools.join("ci.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        prover.commit("this repository's files, gated for the prerequisites a tree holds");
        let stale = prover.work.join("plan/node_modules/a-stale-install");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("package.json"), "{\"name\": \"a-stale-install\", \"version\": \"0.0.0\"}\n").unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
        assert!(push.ok, "the real doctor and plan stage pass in the gated tree: {}", push.why);
        assert!(push.why.contains("ci: ok (doctor plan_views)"), "and both stages ran: {}", push.why);
        let left = std::fs::read_dir(prover.work.join("plan/node_modules")).unwrap().flatten().map(|entry| entry.file_name()).collect::<Vec<_>>();
        assert_eq!(left, ["a-stale-install"], "the checkout's own install is as the push found it");
    }

    /// Every commit a push carries is a pushed commit, not only the tip a ref names: a failing commit
    /// under a clean tip is refused. A commit the remote already holds is carried by no push, so a new
    /// ref at one gates nothing.
    #[test]
    fn every_commit_the_push_carries_is_gated_and_none_the_remote_already_holds() {
        let prover = ScratchRepo::new("prepush-each-commit");
        prover.push_main();
        prover.plant_failure(&prover.work);
        prover.commit("a commit that fails the gate");
        std::fs::remove_file(prover.work.join("ci-fails")).unwrap();
        prover.commit("a clean tip above it");
        let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
        assert!(!push.ok, "the failing commit under the clean tip is a pushed commit too: {}", push.why);
        assert_eq!(push.runs.len(), 2, "both commits the push carries were gated, the one the remote holds was not");
        // Each gate says which commit its checkout stood at, so the order the gates ran in is on the record:
        // the oldest commit goes first, so a refusal names the earliest trouble the push carries before
        // anything built on it.
        let gated = gated_heads(&push);
        let carries = [prover.git(&["rev-parse", "HEAD~1"]), prover.git(&["rev-parse", "HEAD"])];
        assert_eq!(gated, carries, "the gates ran the oldest commit first, as the hook's bound orders them");
        assert!(push.why.contains("a commit that fails the gate"), "the refusal names the failing commit: {}", push.why);
        let held = prover.git(&["rev-parse", "origin/main"]);
        output(&prover.work, &["branch", "held", &held]).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "held"]);
        assert!(push.ok, "a new ref at a commit the remote holds carries nothing new: {}", push.why);
        assert!(push.runs.is_empty(), "so nothing was gated");
    }

    /// The commits a push's gates ran, in the order they ran.
    fn gated_heads(push: &Push) -> Vec<String> {
        push.runs.iter().map(|run| run.field("head").to_string()).collect()
    }

    /// Was `earlier` gated before `later` among the commits a push gated? Both must have been gated.
    fn gated_before(gated: &[String], earlier: &str, later: &str) -> bool {
        let at = |commit: &str| gated.iter().position(|head| head == commit).unwrap();
        at(earlier) < at(later)
    }

    /// A commit is gated after the commits it stands on, whichever way their dates fall: a commit is never
    /// judged before the trouble underneath it, and a refusal names the earliest commit the push carries
    /// first. Two shapes make the dates and the history ask for different orders. A merge dated before the
    /// commit it merges. And two branches pushed at once, one carrying a commit dated years before the
    /// commit it stands on: git's walk takes commits by date across branches, so it meets that parent
    /// before the child on the other branch, and only `--topo-order` keeps every parent after all its
    /// children, so that, reversed, every commit comes after the commits it stands on.
    #[test]
    fn each_commit_is_gated_after_the_commits_it_stands_on_whatever_their_dates() {
        let prover = ScratchRepo::new("prepush-order");
        prover.push_main();
        output(&prover.work, &["checkout", "-q", "-b", "carried"]).unwrap();
        prover.git_dated(&["commit", "-q", "--allow-empty", "-m", "a commit dated after the merge above it"], "Thu Jan  1 00:00:00 2032 +0000");
        let carried = prover.git(&["rev-parse", "HEAD"]);
        output(&prover.work, &["checkout", "-q", "main"]).unwrap();
        prover.git_dated(
            &["merge", "-q", "--no-ff", "--no-edit", "-m", "a merge dated before the commit it merges", "carried"],
            "Wed Jan  1 00:00:00 2025 +0000",
        );
        let merge = prover.git(&["rev-parse", "HEAD"]);
        let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
        assert!(push.ok, "both commits stand on their parents, so both pass: {}", push.why);
        assert_eq!(gated_heads(&push), [carried, merge], "the carried commit was gated before the merge that carries it, though it was dated later");

        let prover = ScratchRepo::new("prepush-order-branches");
        prover.push_main();
        output(&prover.work, &["checkout", "-q", "-b", "left"]).unwrap();
        prover.git_dated(&["commit", "-q", "--allow-empty", "-m", "the parent both branches stand on"], "Tue Jan  1 00:00:00 2030 +0000");
        let parent = prover.git(&["rev-parse", "HEAD"]);
        prover.git_dated(&["commit", "-q", "--allow-empty", "-m", "the left tip"], "Wed Jan  1 00:00:00 2031 +0000");
        let left = prover.git(&["rev-parse", "HEAD"]);
        output(&prover.work, &["checkout", "-q", "-b", "right", &parent]).unwrap();
        prover.git_dated(&["commit", "-q", "--allow-empty", "-m", "a child dated years before its parent"], "Wed Jan  1 00:00:00 2020 +0000");
        let child = prover.git(&["rev-parse", "HEAD"]);
        prover.git_dated(&["commit", "-q", "--allow-empty", "-m", "the right tip"], "Fri Jan  1 00:00:00 2021 +0000");
        let right = prover.git(&["rev-parse", "HEAD"]);
        output(&prover.work, &["checkout", "-q", "main"]).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "left", "right"]);
        assert!(push.ok, "{}", push.why);
        let gated = gated_heads(&push);
        assert_eq!(gated.len(), 4, "each commit the two branches carry was gated once");
        assert!(
            gated_before(&gated, &parent, &child) && gated_before(&gated, &parent, &left) && gated_before(&gated, &child, &right),
            "every commit was gated after the commits it stands on: {gated:?}"
        );
    }

    /// Three new branches pushed at once gate every commit after the commits it stands on, wherever the
    /// dates of the three chains fall: one chain's middle is dated years before the parent it stands on,
    /// and its own tip is dated between them. Git's default walk takes commits newest-date-first across
    /// all three tips at once, so it meets the parent before the old middle that stands on it, and,
    /// reversed, would gate the middle first; only a topological walk gates a commit after every commit
    /// it stands on, whatever the dates ask for.
    #[test]
    fn three_branches_pushed_at_once_gate_every_commit_after_the_ones_it_stands_on() {
        let prover = ScratchRepo::new("prepush-order-three");
        prover.push_main();
        output(&prover.work, &["checkout", "-q", "-b", "left"]).unwrap();
        prover.git_dated(&["commit", "-q", "--allow-empty", "-m", "the shared parent both chains stand on"], "Tue Jan  1 00:00:00 2030 +0000");
        let parent = prover.git(&["rev-parse", "HEAD"]);
        prover.git_dated(&["commit", "-q", "--allow-empty", "-m", "the left tip"], "Sun Jan  1 00:00:00 2035 +0000");
        let left = prover.git(&["rev-parse", "HEAD"]);
        output(&prover.work, &["checkout", "-q", "-b", "mid", &parent]).unwrap();
        prover.git_dated(&["commit", "-q", "--allow-empty", "-m", "a middle dated before its parent"], "Mon Jan  1 00:00:00 2019 +0000");
        let mid = prover.git(&["rev-parse", "HEAD"]);
        output(&prover.work, &["checkout", "-q", "-b", "right", &mid]).unwrap();
        prover.git_dated(&["commit", "-q", "--allow-empty", "-m", "a right tip above the old middle"], "Thu Jan  1 00:00:00 2024 +0000");
        let right = prover.git(&["rev-parse", "HEAD"]);
        output(&prover.work, &["checkout", "-q", "main"]).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "left", "mid", "right"]);
        assert!(push.ok, "{}", push.why);
        let gated = gated_heads(&push);
        assert_eq!(gated.len(), 4, "the three new branches carry four new commits");
        assert!(
            gated_before(&gated, &parent, &left) && gated_before(&gated, &parent, &mid) && gated_before(&gated, &mid, &right),
            "every commit was gated after the commits it stands on: {gated:?}"
        );
    }

    /// What the remote holds is asked of the remote, not read from this repository's remote-tracking refs:
    /// those keep naming a branch the remote has since deleted, and the commits on it, which the remote no
    /// longer holds, would pass ungated. Here a failing commit reached the remote past the gate, on a
    /// branch the remote then dropped; a new branch carrying it is refused.
    #[test]
    fn a_commit_the_remote_dropped_is_gated_though_a_remote_tracking_ref_still_names_it() {
        let prover = ScratchRepo::new("prepush-stale-tracking");
        prover.push_main();
        output(&prover.work, &["checkout", "-q", "-b", "dropped"]).unwrap();
        prover.plant_failure(&prover.work);
        prover.commit("a failing commit on a branch the remote later drops");
        output(&prover.work, &["push", "-q", "--no-verify", "origin", "dropped"]).unwrap();
        output(&prover.scratch.0, &["-C", "remote.git", "update-ref", "-d", "refs/heads/dropped"]).unwrap();
        assert!(!prover.git(&["rev-parse", "--verify", "--quiet", "refs/remotes/origin/dropped"]).is_empty(), "this repository still names the dropped branch");
        output(&prover.work, &["checkout", "-q", "-b", "carried"]).unwrap();
        std::fs::remove_file(prover.work.join("ci-fails")).unwrap();
        prover.commit("a clean commit above it");
        output(&prover.work, &["checkout", "-q", "main"]).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "carried"]);
        assert!(!push.ok, "the commit the remote no longer holds is carried by this push, so it is gated: {}", push.why);
        assert_eq!(push.runs.len(), 2, "both commits the remote lacks were gated");
        assert!(push.why.contains("a failing commit on a branch the remote later drops"), "the refusal names it: {}", push.why);
    }

    /// A remote can fetch from one place and push to another (`pushurl`), as a fork's does. What counts is
    /// what the push's own destination holds: a failing commit the fetch side holds, and the destination
    /// lacks, is carried by the push and gated.
    #[test]
    fn a_push_asks_its_own_destination_what_it_holds() {
        let prover = ScratchRepo::new("prepush-push-url");
        prover.push_main();
        output(&prover.work, &["checkout", "-q", "-b", "upstream-only"]).unwrap();
        prover.plant_failure(&prover.work);
        prover.commit("a failing commit the fetch side holds");
        output(&prover.work, &["push", "-q", "--no-verify", "origin", "upstream-only"]).unwrap();
        output(&prover.work, &["checkout", "-q", "main"]).unwrap();
        output(&prover.scratch.0, &["init", "--bare", "-q", "fork.git"]).unwrap();
        output(&prover.work, &["config", "remote.origin.pushurl", prover.scratch.0.join("fork.git").to_str().unwrap()]).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "upstream-only"]);
        assert!(!push.ok, "the push's destination lacks the failing commit, so the push carries it and is refused: {}", push.why);
        assert_eq!(push.runs.len(), 2, "every commit the destination lacks was gated");
    }

    /// A push whose destination cannot say what it holds is refused, never judged against what this
    /// repository last heard; a push that only deletes carries no commit, so it never asks, and goes
    /// through all the same. Git's own URL rewriting makes the asking fail: the push reaches the remote
    /// through `pushInsteadOf`, while an `insteadOf` rule sends anything that asks that same URL for its
    /// refs to a remote that does not exist.
    #[test]
    fn a_push_whose_destination_cannot_be_asked_is_refused() {
        let prover = ScratchRepo::new("prepush-unaskable");
        prover.push_main();
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a clean commit to an unaskable remote"]).unwrap();
        let real = prover.scratch.0.join("remote.git");
        let real = real.to_str().unwrap();
        let nowhere = prover.scratch.0.join("no-such-remote.git");
        output(&prover.work, &["remote", "set-url", "origin", "unaskable:remote"]).unwrap();
        output(&prover.work, &["config", &format!("url.{real}.pushInsteadOf"), "unaskable:remote"]).unwrap();
        output(&prover.work, &["config", &format!("url.{}.insteadOf", nowhere.display()), real]).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
        assert!(!push.ok, "{}", push.why);
        assert!(push.runs.is_empty(), "no gate ran");
        assert!(push.why.contains(&format!("could not ask {real} which commits it holds")), "the refusal names the destination and why: {}", push.why);
        let push = prover.push(&prover.work, &["push", "-q", "--delete", "origin", "main"]);
        assert!(push.ok, "a push that only deletes is not gated, and never needs to ask: {}", push.why);
    }

    /// A push that names the remote by its URL has no remote-tracking refs at all; asking the remote what
    /// it holds still bounds the push to what it adds.
    #[test]
    fn a_push_to_a_url_gates_only_what_that_remote_lacks() {
        let prover = ScratchRepo::new("prepush-url");
        prover.push_main();
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a commit for a new branch"]).unwrap();
        let url = prover.scratch.0.join("remote.git");
        let push = prover.push(&prover.work, &["push", "-q", url.to_str().unwrap(), "HEAD:refs/heads/fresh"]);
        assert!(push.ok, "{}", push.why);
        assert_eq!(push.runs.len(), 1, "only the commit the remote lacks was gated, not the one it holds");
    }

    /// A forced push can land on a remote whose tips this repository never fetched: an object it lacks is
    /// no reason to refuse the push, and it cannot vouch for the commits beneath it either, so the hook
    /// gates every commit the push could be carrying — its own and the older one the remote's unknown tip
    /// may or may not still hold — rather than take a local record's word for what the remote holds.
    #[test]
    fn a_forced_push_over_commits_this_repository_lacks_gates_all_it_cannot_prove_held() {
        let prover = ScratchRepo::new("prepush-forced");
        prover.push_main();
        let other = prover.scratch.0.join("other");
        output(&prover.scratch.0, &["clone", "-q", "-b", "main", "remote.git", "other"]).unwrap();
        output(&other, &["-c", "user.name=Other", "-c", "user.email=other@example.invalid", "commit", "-q", "--allow-empty", "-m", "a commit from elsewhere"])
            .unwrap();
        output(&other, &["push", "-q", "origin", "HEAD:main"]).unwrap();
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a commit that replaces it"]).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "--force", "origin", "main"]);
        assert!(push.ok, "the forced push gates its commits and passes: {}", push.why);
        assert_eq!(push.runs.len(), 2, "both commits nothing the remote names is known to reach were gated");
    }

    /// A commit whose own gate is gone or cannot run is refused, never passed: a gate that a commit can
    /// switch off by deleting it, or by clearing its executable bit, or by making a folder of that name —
    /// which git can hold and nothing can run — would not be a gate.
    #[test]
    fn a_commit_with_no_gate_that_can_run_is_refused() {
        let spellings: [(&str, fn(&Path)); 3] = [
            ("deleted", |work| {
                output(work, &["rm", "-q", "--", "tools/ci.sh"]).unwrap();
            }),
            ("not-executable", |work| {
                output(work, &["update-index", "--chmod=-x", "--", "tools/ci.sh"]).unwrap();
            }),
            ("a-folder", |work| {
                output(work, &["rm", "-q", "--", "tools/ci.sh"]).unwrap();
                std::fs::create_dir_all(work.join("tools/ci.sh")).unwrap();
                std::fs::write(work.join("tools/ci.sh/not-a-gate"), "a folder is no gate\n").unwrap();
                // git holds a folder only through what stands in it, so the folder of that name has to be
                // staged as well as the removal, or the commit would hold no gate at all and prove the
                // wrong thing.
                output(work, &["add", "-A", "--", "tools/ci.sh"]).unwrap();
            }),
        ];
        for (tag, unmake) in spellings {
            let prover = ScratchRepo::new(&format!("prepush-no-gate-{tag}"));
            prover.push_main();
            unmake(&prover.work);
            output(&prover.work, &["commit", "-q", "-m", "a commit without a gate that can run"]).unwrap();
            let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
            assert!(!push.ok, "a commit whose tools/ci.sh is {tag} is refused: {}", push.why);
            assert!(push.runs.is_empty(), "there was no gate to run");
            assert!(push.why.contains("tools/ci.sh"), "the refusal names the missing gate: {}", push.why);
            assert_eq!(prover.registered_worktrees(), prover.expected_worktrees(), "and its worktree is gone");
        }
    }

    /// A real gate writes into the checkout of the commit it judges: build output, regenerated files.
    /// Git refuses to remove a worktree holding changes or untracked files unless it is told to, so
    /// every removal of a gate's worktree is a forced one: dirt an ordinary gate left must neither
    /// refuse a push its gate passed nor leave a worktree registered behind a push it failed.
    #[test]
    fn a_gate_that_leaves_its_checkout_dirty_goes_with_the_push_whether_it_passed_or_failed() {
        let prover = ScratchRepo::new("prepush-dirty-gate");
        prover.push_main();
        let dirty: &[(&str, &Path)] = &[("GATE_DIRTY", Path::new("1"))];
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a clean commit whose gate leaves dirt behind"]).unwrap();
        let push = prover.start(&prover.work, &["push", "-q", "origin", "main"], dirty).finish();
        assert!(push.ok, "the dirt its gate left judges nothing: {}", push.why);
        assert_eq!(push.runs.len(), 1, "the commit was gated");
        assert_eq!(prover.registered_worktrees(), prover.expected_worktrees(), "and its dirty checkout went with the push");
        prover.plant_failure(&prover.work);
        prover.commit("a commit whose gate fails, as dirty as the one that passed");
        let push = prover.start(&prover.work, &["push", "-q", "origin", "main"], dirty).finish();
        assert!(!push.ok, "the failing gate is refused, dirt and all: {}", push.why);
        assert_eq!(push.runs.len(), 1, "the failing commit was gated");
        assert_eq!(prover.registered_worktrees(), prover.expected_worktrees(), "and its dirty checkout went with the refused push too");
    }

    /// A tag is pushed like a branch: a new tag, lightweight or annotated, gates the new commit it names,
    /// and a tag of a commit the remote holds carries nothing to gate. Every ref line git hands the hook
    /// is read before a gate runs, so no gate can read the lines meant for the hook.
    #[test]
    fn a_pushed_tag_gates_the_new_commit_it_names() {
        let prover = ScratchRepo::new("prepush-tags");
        prover.push_main();
        output(&prover.work, &["tag", "-a", "-m", "a held commit", "on-held", "main"]).unwrap();
        output(&prover.work, &["checkout", "-q", "-b", "broken"]).unwrap();
        prover.plant_failure(&prover.work);
        prover.commit("a commit that fails the gate");
        output(&prover.work, &["tag", "light-broken"]).unwrap();
        output(&prover.work, &["tag", "-a", "-m", "an annotated tag", "annotated-broken"]).unwrap();
        output(&prover.work, &["checkout", "-q", "main"]).unwrap();
        for tag in ["light-broken", "annotated-broken"] {
            let push = prover.push(&prover.work, &["push", "-q", "origin", tag]);
            assert!(!push.ok, "{tag} names a new commit that fails its gate: {}", push.why);
            assert_eq!(push.runs.len(), 1, "{tag}'s commit was gated");
        }
        let push = prover.push(&prover.work, &["push", "-q", "origin", "on-held"]);
        assert!(push.ok && push.runs.is_empty(), "an annotated tag of a held commit carries no commit to gate: {}", push.why);
        let tree = prover.git(&["rev-parse", "main^{tree}"]);
        output(&prover.work, &["tag", "-a", "-m", "a tag of a tree", "of-a-tree", &tree]).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "of-a-tree"]);
        assert!(push.ok && push.runs.is_empty(), "a tag of a tree names no commit, so it carries none to gate: {}", push.why);
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a new clean commit"]).unwrap();
        output(&prover.work, &["tag", "-a", "-m", "beside the branch", "beside", "main"]).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "main", "beside", "light-broken"]);
        assert!(!push.ok, "one failing ref refuses the push: {}", push.why);
        assert_eq!(push.runs.len(), 2, "the branch and its tag share one commit, gated once, and the broken tag's commit once");
        assert!(push.runs.iter().all(|run| run.field("stdin") == "0"), "no gate could read the hook's ref lines");
    }

    /// Two pushes at once, from two worktrees of one repository, never run their gates at the same time
    /// in the one build folder they share, and each checks its commit out only after the other's gate has
    /// written its last artifact, so every file it builds is newer than every artifact in the folder.
    #[test]
    fn the_gates_of_simultaneous_pushes_run_one_at_a_time_in_one_warm_build_folder() {
        let prover = ScratchRepo::new("prepush-simultaneous");
        prover.push_main();
        let linked = prover.scratch.0.join("linked");
        output(&prover.work, &["worktree", "add", "-q", "-b", "side", linked.to_str().unwrap(), "main"]).unwrap();
        output(&linked, &["commit", "-q", "--allow-empty", "-m", "a clean commit on side"]).unwrap();
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a clean commit on main"]).unwrap();
        let slow: &[(&str, &Path)] = &[("GATE_SLEEP", Path::new("1"))];
        let pushes = [prover.start(&prover.work, &["push", "-q", "origin", "main"], slow), prover.start(&linked, &["push", "-q", "origin", "side"], slow)];
        let pushes: Vec<Push> = pushes.into_iter().map(Pushing::finish).collect();
        assert!(pushes.iter().all(|push| push.ok), "both pushes pass: {:?}", pushes.iter().map(|push| &push.why).collect::<Vec<_>>());
        assert!(pushes.iter().any(|push| push.why.contains(WAITING)), "one push waited for the other's gate");
        let runs: Vec<&Run> = pushes.iter().flat_map(|push| &push.runs).collect();
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|run| run.field("overlap") == "0"), "no gate ran while another was running in the folder");
        assert_ne!(pushes[0].runs[0].field("tree"), pushes[1].runs[0].field("tree"), "each push's gate ran in its own checkout, never the other's");
        let target = runs[0].target().unwrap();
        assert_eq!(target.parent(), Some(prover.hook_folder().as_path()), "the build folder is the repository's own");
        assert_eq!(runs[0].target(), runs[1].target(), "one build folder, shared by the worktrees' pushes");
        for run in runs {
            let (checkout, artifact) = run.checkout_and_previous_artifact();
            let artifact = artifact.expect("a gate found an artifact in its build folder, so the folder survived from the push before it to this one");
            assert!(checkout > artifact, "a commit was checked out before the gate before it had finished");
        }
        assert!(target.join("artifact").exists(), "the folder stays warm for the next push");
    }

    /// A push interrupted while its gate runs — Ctrl-C, a TERM, a closed terminal — stops there, even when
    /// the gate turns the signal into an ordinary exit, as Gradle's JVM does: it gates no further commit,
    /// removes its temporary worktree and its temporary folder, and lets its lock go, so the next push is
    /// not held up. The gate leaves dirt in the checkout it dies above, as a real build does, so the
    /// removal that cleans up after the push is proven against a tree git would otherwise refuse to
    /// remove; and its running marker goes with it: the clean push after the signals gates without ever
    /// believing another gate is running beside it.
    #[test]
    fn an_interrupted_push_stops_and_cleans_up_on_every_signal() {
        let prover = ScratchRepo::new("prepush-interrupted");
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a second commit to gate"]).unwrap();
        let hold = prover.scratch.0.join("hold");
        let dirty: &[(&str, &Path)] = &[("GATE_DIRTY", Path::new("1"))];
        for signal in ["INT", "TERM", "HUP"] {
            let (pushing, hook, tree) = prover.start_held(&hold, dirty);
            let gate_log = pushing.log.clone();
            signal_group(pushing.child.id(), signal);
            assert!(!pushing.finish().ok, "an interrupted push is refused");
            // Git dies of the signal at once; its hook goes on until it has cleaned up.
            wait_until("the interrupted hook to exit", || !running(&hook));
            assert!(std::fs::read_to_string(&gate_log).unwrap_or_default().is_empty(), "the hook gated no commit after {signal}");
            assert_eq!(prover.registered_worktrees(), prover.expected_worktrees(), "the temporary worktree is gone after {signal}");
            assert!(!tree.parent().unwrap().exists(), "and so is its temporary folder after {signal}");
            assert!(!prover.lock_is_held(), "the lock is let go after {signal}");
        }
        let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
        assert!(push.ok, "the clean push after the signals passes: {}", push.why);
        assert!(
            push.runs.iter().all(|run| run.field("overlap") == "0"),
            "the gate a signal took let its running marker go, so no later gate sees a false overlap: {:?}",
            push.runs.iter().map(|run| run.field("overlap")).collect::<Vec<_>>()
        );
    }

    /// Do two pushes under `name=value` name a live process's start time differently? Where they do not,
    /// that round of the waiting test below would prove nothing, so it is skipped on that machine.
    fn start_named_differently_under(name: &str, value: &str) -> bool {
        let mut sleeper = Command::new("sleep").arg("30").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let pid = sleeper.id().to_string();
        let named = |extra: Option<(&str, &str)>| {
            let mut probe = Command::new("ps");
            probe.args(["-o", "lstart=", "-p", &pid]);
            if let Some((name, value)) = extra {
                probe.env(name, value);
            }
            String::from_utf8(probe.output().unwrap().stdout).unwrap().trim().to_string()
        };
        let differs = named(None) != named(Some((name, value)));
        let _ = sleeper.kill();
        let _ = sleeper.wait();
        differs
    }

    /// A client can stop reading a push's stderr — a GUI client, an IDE, a script that gave up — and leave
    /// it a closed pipe, where every word the hook writes meets SIGPIPE. None may change what the hook does
    /// next: the push its gate passed still goes, and the hook still lets its lock and its record go and
    /// leaves nothing registered. This push's stderr closes while its gate runs, and the gate then
    /// finishes, so the hook's last word meets the closed pipe just before the hook lets go.
    #[test]
    fn a_push_whose_stderr_was_closed_still_lets_its_lock_go() {
        let prover = ScratchRepo::new("prepush-closed-stderr");
        let hold = prover.scratch.0.join("hold");
        let unread = prover.scratch.0.join("unread.stderr");
        let mut pushing = prover.spawn(&prover.work, &["push", "-q", "origin", "main"], &[("GATE_HOLD", &hold)], Stdio::piped(), unread);
        let (hook, tree) = held(&hold);
        drop(pushing.child.stderr.take());
        let_go(&hold);
        wait_until("the hook to exit", || !running(&hook));
        assert!(pushing.child.wait().unwrap().success(), "the gate passed, and a word no one read does not refuse the push");
        assert!(prover.remote_refs().contains("refs/heads/main"), "the commit reached the remote");
        assert!(!prover.lock_is_held(), "the hook let its lock go though no one read its last word");
        assert!(!prover.hook_folder().join("tree").exists(), "and its record of the worktree");
        assert_eq!(prover.registered_worktrees(), prover.expected_worktrees(), "and left no worktree registered");
        assert!(!tree.parent().unwrap().exists(), "nor its temporary folder");
    }

    /// A push interrupted while it waits for another push's gate leaves that push's lock alone: only the
    /// holder lets a lock go, so the gate under way is not joined by the next push's. A hook judges its
    /// holder by the start time of its process, so the wait is proved for every way two pushes can come to
    /// name one start differently — a locale or a time zone of their own — and for a `ps` that cannot
    /// answer, on the waiter's side or the holder's: a holder it cannot judge is a live one, waited for.
    #[test]
    fn a_push_interrupted_while_it_waits_leaves_the_other_push_s_lock_alone() {
        let prover = ScratchRepo::new("prepush-interrupted-waiting");
        prover.push_main();
        let hold = prover.scratch.0.join("hold");
        // A `ps` that answers nothing, first on the PATH of whichever side the round names.
        let no_ps = prover.scratch.0.join("no-ps");
        std::fs::create_dir_all(&no_ps).unwrap();
        std::fs::write(no_ps.join("ps"), "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(no_ps.join("ps"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut path = std::ffi::OsString::from(&no_ps);
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap());
        let path = PathBuf::from(path);
        let mut rounds: Vec<(Vec<(&str, PathBuf)>, Vec<(&str, PathBuf)>)> = vec![(Vec::new(), Vec::new())];
        // Each round runs under the first of its candidates that names a start differently on this
        // machine; where none does, the machine has no difference to prove, and the test says so rather
        // than pass in silence on a round it never ran. Two time zones nine hours apart cannot both name a
        // start as the machine's own does, so the time-zone round is skipped only where ps ignores TZ.
        let candidates: [(&str, &[&str]); 2] = [("LC_ALL", &["fr_FR.UTF-8", "de_DE.UTF-8", "ja_JP.UTF-8"]), ("TZ", &["UTC0", "JST-9"])];
        for (name, values) in candidates {
            match values.iter().find(|value| start_named_differently_under(name, value)) {
                Some(value) => rounds.push((Vec::new(), vec![(name, PathBuf::from(value))])),
                None => eprintln!("note: no {name} round runs here: under each of {values:?}, ps names a start as it does without it"),
            }
        }
        rounds.push((Vec::new(), vec![("PATH", path.clone())]));
        rounds.push((vec![("PATH", path)], Vec::new()));
        for (n, (holder_env, waiter_env)) in rounds.iter().enumerate() {
            let message = format!("a clean commit the round-{n} waiter waits behind");
            output(&prover.work, &["commit", "-q", "--allow-empty", "-m", &message]).unwrap();
            let holder_env: Vec<(&str, &Path)> = holder_env.iter().map(|(name, value)| (*name, value.as_path())).collect();
            let (holder, hook, tree) = prover.start_held(&hold, &holder_env);
            output(&prover.work, &["branch", "--force", "waiter", "main"]).unwrap();
            let env: Vec<(&str, &Path)> = waiter_env.iter().map(|(name, value)| (*name, value.as_path())).collect();
            let waiter = prover.start(&prover.work, &["push", "-q", "origin", "waiter"], &env);
            wait_until(&format!("the round-{n} push to wait"), || std::fs::read_to_string(&waiter.stderr).is_ok_and(|text| text.contains(WAITING)));
            let words = std::fs::read_to_string(&waiter.stderr).unwrap();
            assert!(words.contains(&format!("held by {hook}")), "the wait names the push it waits for: {words}");
            signal_group(waiter.child.id(), "INT");
            assert!(!waiter.finish().ok, "the interrupted waiter is refused");
            assert!(prover.lock_is_held(), "the first push still holds its lock");
            let record = std::fs::read_to_string(prover.hook_folder().join("tree")).unwrap();
            assert_eq!(std::fs::canonicalize(record.trim()).unwrap(), tree, "and its record of the worktree its gate runs in");
            let_go(&hold);
            let push = holder.finish();
            assert!(push.ok, "the first push finishes its gate: {}", push.why);
            assert!(!prover.lock_is_held(), "and lets its lock go");
            assert_eq!(prover.registered_worktrees(), prover.expected_worktrees());
        }
    }

    /// The first push of two new branches whose new commits overlap gates the shared commits once: git
    /// names both tips, one rev-list lets each commit show once, and what the remote already holds gates
    /// nothing, wherever a ref's old tip is all zeroes.
    #[test]
    fn pushing_two_new_branches_sharing_new_commits_gates_each_shared_commit_once() {
        let prover = ScratchRepo::new("prepush-shared");
        prover.push_main();
        // alpha's one commit fails the gate; beta adds a clean one above it. Both branches are new, so
        // neither old tip names a commit, and the only held commit is the one the remote already has.
        output(&prover.work, &["checkout", "-q", "-b", "alpha"]).unwrap();
        prover.plant_failure(&prover.work);
        prover.commit("a commit both branches share, failing the gate");
        output(&prover.work, &["checkout", "-q", "-b", "beta"]).unwrap();
        std::fs::remove_file(prover.work.join("ci-fails")).unwrap();
        prover.commit("a clean commit only beta carries");
        let push = prover.push(&prover.work, &["push", "-q", "origin", "alpha", "beta"]);
        assert!(!push.ok, "the commit two new branches share fails, so neither branch goes: {}", push.why);
        assert_eq!(push.runs.len(), 2, "the shared commit was gated once and beta's own commit once, the held one not at all");
        assert!(push.why.contains("a commit both branches share"), "the refusal names the shared commit: {}", push.why);
        let refs = prover.remote_refs();
        assert!(!refs.contains("refs/heads/alpha") && !refs.contains("refs/heads/beta"), "both new branches stayed local");
    }

    /// A forced push that rewinds the remote moves it back to a commit the remote already holds: the
    /// push carries no new commit, so it gates nothing.
    #[test]
    fn a_forced_push_that_rewinds_the_remote_gates_nothing() {
        let prover = ScratchRepo::new("prepush-rewind");
        prover.push_main();
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a commit the rewind drops"]).unwrap();
        prover.push_main();
        let back = prover.git(&["rev-parse", "HEAD~1"]);
        output(&prover.work, &["reset", "-q", "--hard", "HEAD~1"]).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "--force", "origin", "main"]);
        assert!(push.ok, "rewinding to a commit the remote holds carries nothing new: {}", push.why);
        assert!(push.runs.is_empty(), "so the gate ran on nothing");
        let remote = prover.git(&["ls-remote", "origin", "refs/heads/main"]);
        assert_eq!(remote.split_whitespace().next(), Some(back.as_str()), "the remote stands at the commit the push moved it to");
    }

    /// A killed hook's lock can come to name a pid the system has since handed to another process: the
    /// start time tells the dead holder from that stranger, so the lock is stale although the pid lives.
    /// The stranger is left running, exactly as a live holder would be.
    #[test]
    fn a_stale_lock_whose_pid_a_stranger_now_holds_is_broken() {
        let prover = ScratchRepo::new("prepush-reused-pid");
        prover.push_main();
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a clean commit behind a stranger's lock"]).unwrap();
        let mut stranger = Command::new("sleep").arg("120").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let area = prover.hook_folder();
        std::fs::create_dir_all(&area).unwrap();
        std::os::unix::fs::symlink(format!("{} Wed Jan 1 00:00:00 2025", stranger.id()), area.join("lock")).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
        let _ = stranger.kill();
        let _ = stranger.wait();
        assert!(push.ok, "a lock whose pid lives but whose start time does not match is broken: {}", push.why);
        assert_eq!(push.runs.len(), 1, "the push gated its own commit after breaking the stale lock");
        assert_eq!(std::fs::read_to_string(area.join("generation")).unwrap().trim(), "1", "breaking the lock moved the build folder on");
        assert!(!prover.lock_is_held(), "and the lock is let go");
    }

    /// A stale lock can change hands while a waiter breaks it: another waiter breaks it first and a new
    /// push takes it. The waiter removes the lock only while it still names the dead holder it judged, so
    /// the new holder's lock is put back and waited for, never removed. The change of hands is made
    /// exactly there: a `ps` that, asked about the dead holder, first hands the lock to a live process
    /// named as a hook names itself.
    #[test]
    fn a_lock_that_changes_hands_while_it_is_broken_is_put_back_and_waited_for() {
        let prover = ScratchRepo::new("prepush-changes-hands");
        prover.push_main();
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a clean commit behind a lock that changes hands"]).unwrap();
        let area = prover.hook_folder();
        std::fs::create_dir_all(&area).unwrap();
        let lock = area.join("lock");
        let mut gone = Command::new("true").spawn().unwrap();
        gone.wait().unwrap();
        std::os::unix::fs::symlink(format!("{} Wed Jan  1 00:00:00 2025", gone.id()), &lock).unwrap();
        let mut taker = Command::new("sleep").arg("120").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let start = Command::new("ps").env("LC_ALL", "C").env("TZ", "UTC0").args(["-o", "lstart=", "-p", &taker.id().to_string()]).output().unwrap();
        let taken = format!("{} {}", taker.id(), String::from_utf8(start.stdout).unwrap().trim_end_matches('\n'));
        let fake = prover.scratch.0.join("hands-over");
        std::fs::create_dir_all(&fake).unwrap();
        let path = std::env::var("PATH").unwrap();
        let script = format!(
            "#!/bin/sh\nif [ \"$4\" = {dead} ] && [ ! -e {flag} ]; then : >{flag}; rm -f {lock}; ln -s '{taken}' {lock}; fi\nPATH='{path}' exec ps \"$@\"\n",
            dead = gone.id(),
            flag = fake.join("handed").display(),
            lock = lock.display(),
        );
        std::fs::write(fake.join("ps"), script).unwrap();
        std::fs::set_permissions(fake.join("ps"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let waiter = prover.start(&prover.work, &["push", "-q", "origin", "main"], &[("PATH", &PathBuf::from(format!("{}:{path}", fake.display())))]);
        wait_until("the push to wait for the lock's new holder", || std::fs::read_to_string(&waiter.stderr).is_ok_and(|text| text.contains(WAITING)));
        let words = std::fs::read_to_string(&waiter.stderr).unwrap();
        assert!(words.contains(&format!("held by {}", taker.id())), "the push waits for the lock's new holder: {words}");
        assert_eq!(std::fs::read_link(&lock).unwrap(), PathBuf::from(&taken), "the new holder's lock stands, put back as it was");
        let _ = taker.kill();
        let _ = taker.wait();
        let push = waiter.finish();
        assert!(push.ok, "once the new holder is gone too, the push breaks its lock and gates: {}", push.why);
        assert_eq!(push.runs.len(), 1);
        assert!(!prover.lock_is_held(), "and lets its own lock go");
    }

    /// A push lets go only the lock and the record that name it. Should either have passed to another push
    /// while it ran, it leaves the other push's standing as it finishes, so the gate under way there is
    /// not joined by a third and its worktree is not thrown away as a leftover.
    #[test]
    fn a_push_lets_go_only_the_lock_that_names_it() {
        let prover = ScratchRepo::new("prepush-own-lock");
        let hold = prover.scratch.0.join("hold");
        let (holder, _, _) = prover.start_held(&hold, &[]);
        let lock = prover.hook_folder().join("lock");
        let record = prover.hook_folder().join("tree");
        let mut other = Command::new("sleep").arg("120").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let other_lock = PathBuf::from(format!("{} named as another push", other.id()));
        std::fs::remove_file(&lock).unwrap();
        std::os::unix::fs::symlink(&other_lock, &lock).unwrap();
        std::fs::write(&record, "another push's worktree\n").unwrap();
        let_go(&hold);
        let push = holder.finish();
        let _ = other.kill();
        let _ = other.wait();
        assert!(push.ok, "the push finishes its gate: {}", push.why);
        assert_eq!(std::fs::read_link(&lock).unwrap(), other_lock, "the lock that names another push still stands");
        assert_eq!(std::fs::read_to_string(&record).unwrap_or_default(), "another push's worktree\n", "and so does the record naming another push's worktree");
    }

    /// A file that comes to stand at the lock name while a push holds it was made by no push either, so it
    /// stays: the push puts back what it never made, and the next push refuses and names it. Removing it
    /// here would throw away a person's file in the name of a lock, and leave nothing to tell them the
    /// lock is gone.
    #[test]
    fn a_file_that_takes_the_lock_name_while_a_push_ran_is_left_where_it_was() {
        let prover = ScratchRepo::new("prepush-lock-replaced");
        prover.push_main();
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a clean commit a push holds"]).unwrap();
        let hold = prover.scratch.0.join("hold");
        let (pushing, _, _) = prover.start_held(&hold, &[]);
        let lock = prover.hook_folder().join("lock");
        std::fs::remove_file(&lock).unwrap();
        std::fs::write(&lock, "not a lock this hook made\n").unwrap();
        let_go(&hold);
        let push = pushing.finish();
        assert!(push.ok, "the gate this push was gating went through; what stands at the lock name now judges nothing: {}", push.why);
        assert_eq!(
            std::fs::read_to_string(&lock).unwrap_or_default(),
            "not a lock this hook made\n",
            "the file stands where it was put: a push removes only the lock it made"
        );
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a clean commit behind that file"]).unwrap();
        let next = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
        assert!(!next.ok, "the next push is refused by the file, rather than gate unserialised: {}", next.why);
        assert!(next.why.contains("is not a lock this hook made"), "and is refused in the claim's own words: {}", next.why);
    }

    /// A remote can push to several places at once (its fork and a mirror, say), and git runs the hook once
    /// for each of them. Each is asked on its own what it holds, so a commit one place lacks is gated even
    /// while another place already has it: asking one place for all of them, or this repository's own
    /// record of the remote, would let the failing commit reach the destination that never held it.
    #[test]
    fn a_push_to_several_destinations_judges_each_by_what_it_holds() {
        let prover = ScratchRepo::new("prepush-several-urls");
        prover.push_main();
        let held = prover.git(&["rev-parse", "main"]);
        let first = prover.scratch.0.join("remote.git");
        // A second destination, kept behind the first: a bare clone as it stood, holding the first commit
        // and nothing above it.
        let second = Scratch::new("remote-two");
        output(&second.0, &["clone", "-q", "--bare", first.to_str().unwrap(), "two.git"]).unwrap();
        let two = second.0.join("two.git");
        output(&prover.work, &["remote", "set-url", "--add", "--push", "origin", first.to_str().unwrap()]).unwrap();
        output(&prover.work, &["remote", "set-url", "--add", "--push", "origin", two.to_str().unwrap()]).unwrap();
        // A failing commit goes to the first destination only, and past the gate: the second never held it.
        prover.plant_failure(&prover.work);
        prover.commit("a failing commit only the first destination holds");
        let failing = prover.git(&["rev-parse", "HEAD"]);
        output(&prover.work, &["push", "-q", "--no-verify", first.to_str().unwrap(), "main"]).unwrap();
        std::fs::remove_file(prover.work.join("ci-fails")).unwrap();
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a clean commit above the failing one"]).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
        assert!(!push.ok, "the destination that lacks the failing commit refuses the push: {}", push.why);
        assert!(push.why.contains("a failing commit only the first destination holds"), "the refusal names it: {}", push.why);
        assert!(gated_heads(&push).contains(&failing), "the destination that lacks it gated it, though the other holds it");
        let where_two_stands = String::from_utf8(output(&two, &["rev-parse", "refs/heads/main"]).unwrap()).unwrap().trim().to_string();
        assert_eq!(where_two_stands, held, "the refusing destination took none of the push");
    }

    /// The lock is a link this hook makes; a folder or a file at its name was never made by a push and
    /// names no holder to wait for or to judge. A folder there would take the hook's link inside itself
    /// and let every push through unserialised, so the push is refused, naming what stands there and what
    /// to do, and nothing there is touched. A link — even one that points nowhere — names a holder by its
    /// target instead: a live one is waited for, a dead one broken as any dead holder's lock is, so a
    /// push cleans a stray link up rather than wait on it or refuse for it.
    #[test]
    fn a_lock_name_holding_what_the_hook_never_makes_refuses_the_push() {
        for folder in [true, false] {
            let prover = ScratchRepo::new("prepush-foreign-lock");
            prover.push_main();
            output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a clean commit behind a foreign lock"]).unwrap();
            let area = prover.hook_folder();
            std::fs::create_dir_all(&area).unwrap();
            let foreign = area.join("lock");
            if folder {
                std::fs::create_dir(&foreign).unwrap();
            } else {
                std::fs::write(&foreign, "not a lock\n").unwrap();
            }
            let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
            assert!(!push.ok, "a push behind a lock that is no link is refused, not let through or left waiting: {}", push.why);
            assert!(push.runs.is_empty(), "no gate ran");
            assert!(
                push.why.contains(&format!("{} is not a lock", foreign.display())) && push.why.contains("remove it") && !push.why.contains("could not make"),
                "the refusal is the claim's own words, naming it and the fix: {}",
                push.why
            );
            assert!(!folder || std::fs::read_dir(&foreign).unwrap().next().is_none(), "the folder was left as it was found, empty");
            assert_eq!(prover.registered_worktrees(), prover.expected_worktrees());
        }
        // A link at the lock name that points nowhere is the same shape the hook makes its own lock as:
        // a link whose named holder is gone. It is broken like a stale lock, and the push gates.
        let prover = ScratchRepo::new("prepush-dangling-lock");
        prover.push_main();
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a clean commit behind a dangling link"]).unwrap();
        let area = prover.hook_folder();
        std::fs::create_dir_all(&area).unwrap();
        let foreign = area.join("lock");
        std::os::unix::fs::symlink("nowhere-this-path-is-not-a-push", &foreign).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
        assert!(push.ok, "a dangling link names no live holder, so it is broken and the push gates: {}", push.why);
        assert_eq!(push.runs.len(), 1, "the push's own commit was gated");
        assert!(!prover.lock_is_held(), "and the push let its lock go");
    }

    /// Beside the lock the hook keeps two files: the record of the worktree a gate runs in, and the number
    /// that names the build folder. Anything else at either name was never made by a push. A folder at
    /// the number's name would take each new number inside itself, so no broken lock would ever retire the
    /// folder a dead holder's gate still writes to; a link at the record's name would have the hook write
    /// through it, to wherever it points. The push is refused, naming it, and nothing there is touched.
    #[test]
    fn a_hook_file_name_holding_what_the_hook_never_makes_refuses_the_push() {
        for (name, kind) in [("generation", "folder"), ("tree", "folder"), ("tree", "link"), ("generation", "link")] {
            let prover = ScratchRepo::new("prepush-foreign-file");
            prover.push_main();
            output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a clean commit behind a foreign file"]).unwrap();
            let area = prover.hook_folder();
            std::fs::create_dir_all(&area).unwrap();
            let foreign = area.join(name);
            let elsewhere = prover.scratch.0.join("somebody-else-s-file");
            std::fs::write(&elsewhere, "not the hook's\n").unwrap();
            if kind == "folder" {
                std::fs::create_dir(&foreign).unwrap();
            } else {
                std::os::unix::fs::symlink(&elsewhere, &foreign).unwrap();
            }
            let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
            assert!(!push.ok, "a {kind} at {name} refuses the push: {}", push.why);
            assert!(push.runs.is_empty(), "no gate ran");
            assert!(
                push.why.contains(&format!("{} is not a file this hook made", foreign.display())) && push.why.contains("remove it"),
                "the refusal names it and the fix: {}",
                push.why
            );
            assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "not the hook's\n", "and nothing it points to was written");
            assert!(kind != "folder" || std::fs::read_dir(&foreign).unwrap().next().is_none(), "the folder was left as it was found, empty");
        }
    }

    /// A push that cannot make the lock at all — the hook's folder belongs to another user, or is read-only
    /// — is refused at once, in words that name the lock and carry the system's own reason, never left to
    /// wait on a lock nobody holds.
    #[test]
    fn a_push_that_cannot_make_the_lock_is_refused_saying_why() {
        let prover = ScratchRepo::new("prepush-unwritable");
        prover.push_main();
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a clean commit behind a folder it cannot write"]).unwrap();
        let area = prover.hook_folder();
        std::fs::create_dir_all(&area).unwrap();
        std::fs::set_permissions(&area, std::fs::Permissions::from_mode(0o555)).unwrap();
        let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
        std::fs::set_permissions(&area, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!push.ok, "{}", push.why);
        assert!(push.runs.is_empty(), "no gate ran");
        let lock = area.join("lock");
        assert!(push.why.contains(&format!("could not make the lock {}: ", lock.display())), "the refusal names the lock and the reason: {}", push.why);
        assert!(!push.why.contains(WAITING), "and it never waited: {}", push.why);
    }

    /// A push killed outright while it waits leaves nothing in the temporary folder: it makes its
    /// temporary worktree's folder only once it holds the lock, so a hook that dies waiting, with no trap
    /// to run, has made none.
    #[test]
    fn a_push_killed_outright_while_it_waits_leaves_nothing_behind() {
        let prover = ScratchRepo::new("prepush-killed-waiting");
        prover.push_main();
        output(&prover.work, &["commit", "-q", "--allow-empty", "-m", "a clean commit a killed waiter waits behind"]).unwrap();
        let hold = prover.scratch.0.join("hold");
        let (holder, _, _) = prover.start_held(&hold, &[]);
        let own_tmp = prover.scratch.0.join("waiter-tmp");
        std::fs::create_dir_all(&own_tmp).unwrap();
        output(&prover.work, &["branch", "--force", "waiter", "main"]).unwrap();
        let waiter = prover.start(&prover.work, &["push", "-q", "origin", "waiter"], &[("TMPDIR", &own_tmp)]);
        wait_until("the push to wait", || std::fs::read_to_string(&waiter.stderr).is_ok_and(|text| text.contains(WAITING)));
        signal_group(waiter.child.id(), "KILL");
        assert!(!waiter.finish().ok);
        assert!(std::fs::read_dir(&own_tmp).unwrap().next().is_none(), "the killed waiter left nothing in its temporary folder");
        let_go(&hold);
        assert!(holder.finish().ok, "the holder's push is untouched by the waiter's death");
    }

    /// A hook killed outright runs no trap: it leaves its lock and its worktree behind, and its gate's
    /// processes running. The next push finds the lock's holder gone, breaks the lock, removes the
    /// worktree, and gates in a new build folder, so nothing the dead hook's gate still writes reaches
    /// it. The worktree it removes is dirty, as one a real gate left always is.
    #[test]
    fn a_push_after_a_hook_was_killed_outright_breaks_its_lock_and_removes_its_worktree() {
        let prover = ScratchRepo::new("prepush-killed");
        let hold = prover.scratch.0.join("hold");
        let (pushing, hook, tree) = prover.start_held(&hold, &[("GATE_DIRTY", Path::new("1"))]);
        assert!(Command::new("kill").args(["-s", "KILL", &hook]).status().unwrap().success());
        let gate_log = pushing.log.clone();
        let push = pushing.finish();
        assert!(!push.ok, "the killed hook's push fails");
        // The killed hook's gate, orphaned, is let finish, as it would on its own.
        let_go(&hold);
        wait_until("the orphaned gate to finish", || !runs_in(&gate_log).is_empty());
        let orphaned = runs_in(&gate_log)[0].target().unwrap();
        assert!(prover.lock_is_held() && tree.exists(), "the killed hook left its lock and its worktree");
        let push = prover.push(&prover.work, &["push", "-q", "origin", "main"]);
        assert!(push.ok, "the next push breaks the dead hook's lock and gates: {}", push.why);
        assert_eq!(push.runs.len(), 1);
        assert_ne!(push.runs[0].target().unwrap(), orphaned, "the next gate builds in a folder the dead hook's gate never named");
        assert!(!orphaned.exists(), "and the folder that gate had is gone");
        assert!(!tree.exists() && !tree.parent().unwrap().exists(), "the dead hook's worktree is removed, and the temporary folder it sat in");
        assert_eq!(prover.registered_worktrees(), prover.expected_worktrees(), "and is no longer registered");
        assert!(!prover.lock_is_held(), "the lock is let go");
    }
}
