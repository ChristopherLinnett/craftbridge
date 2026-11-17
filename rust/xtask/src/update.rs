//! `cargo xtask upstream update <rev>`: the whole upstream update of plan §24, which
//! `tools/update-upstream.sh` runs. Steps 1 and 2 are [`crate::upstream::pin`]; this module adds the rest
//! and decides how the run ends.
//!
//! 1. and 2. move the pin and write what comes from it,
//! 3. name the adaptations whose upstream items the revision change touches ([`crate::adaptations`]),
//! 4. regenerate the API surface and diff it against the one the tree held ([`crate::effects_api`]),
//! 5. run the gate, the conformance suite with it,
//! 6. append the record to `docs/upstream-updates.md`,
//! 7. name every version with a newer release ([`crate::versions`]).
//!
//! A run on the revision the tree already holds stops after step 2, having changed nothing: it fetches
//! only what the cache lacks for steps 1 and 2, and looks up no release. Every step sits behind [`Steps`],
//! so the order, the states and the exit codes are tested with steps that only record what was asked of
//! them.

use crate::adaptations::{Adaptation, Comparison, Repository, Revisions};
use crate::effects_api::SurfaceDiff;
use crate::versions::Newer;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Where the record of updates is kept.
const RECORD: &str = "docs/upstream-updates.md";

/// The two revisions a pin names: EffectCraft's, in full, and the FilmCraft revision EffectCraft names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Revs {
    pub(crate) effectcraft: String,
    pub(crate) filmcraft: String,
}

impl Revs {
    fn same_effectcraft(&self, other: &Self) -> bool {
        same_revision(&self.effectcraft, &other.effectcraft)
    }

    fn same_filmcraft(&self, other: &Self) -> bool {
        same_revision(&self.filmcraft, &other.filmcraft)
    }

    /// The same revisions of both?
    fn same_as(&self, other: &Self) -> bool {
        self.same_effectcraft(other) && self.same_filmcraft(other)
    }
}

/// The same commit? A revision may be spelt in full or abbreviated, so one that starts the other is the
/// same; an empty spelling names no commit, so it is the same as none.
fn same_revision(one: &str, two: &str) -> bool {
    !one.is_empty() && !two.is_empty() && (one.starts_with(two) || two.starts_with(one))
}

/// How a run ended: the states of plan §24.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    /// The tree already held the revision: nothing changed.
    UpToDate,
    /// Updated, with nothing to recheck.
    Clean,
    /// Updated, and the run lists adaptations to recheck before the update merges.
    Recheck,
    /// The new surface breaks an app written against the old: the next release is a major version.
    Breaking,
    /// The gate failed on the new revision.
    GateFailed,
}

/// What a run did, for the person who ran it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) state: State,
    /// The report to print.
    pub(crate) report: String,
    /// Why step 7 could not say which versions have newer releases, when it could not.
    pub(crate) unchecked: Option<String>,
}

impl Outcome {
    /// 0 when the update stands, 2 for a breaking API change, 1 when the gate failed or a step could not
    /// finish.
    pub(crate) fn code(&self) -> u8 {
        match self.state {
            State::GateFailed => 1,
            _ if self.unchecked.is_some() => 1,
            State::Breaking => 2,
            State::UpToDate | State::Clean | State::Recheck => 0,
        }
    }
}

/// What each step of the update does, so the order and the endings can be tested without a gate.
pub(crate) trait Steps {
    /// Before anything moves: the revisions the tree holds, with the adaptations list checked against
    /// them, the revisions the Craft libraries its places name are built at kept for step 3, and the
    /// surface kept for step 4.
    fn before(&self) -> Result<Revs, String>;
    /// Steps 1 and 2: move the pin to `rev`; the revisions it now holds.
    fn pin(&self, rev: &str) -> Result<Revs, String>;
    /// Step 3.
    fn adaptations(&self, old: &Revs, new: &Revs) -> Result<Comparison, String>;
    /// Step 4: regenerate, and diff against the surface kept in [`Steps::before`].
    fn surface(&self) -> Result<SurfaceDiff, String>;
    /// Step 5: did the gate pass?
    fn gate(&self) -> Result<bool, String>;
    /// Step 6.
    fn record(&self, section: &str) -> Result<(), String>;
    /// Step 7.
    fn versions(&self) -> Result<Vec<Newer>, String>;
}

fn short(revision: &str) -> &str {
    revision.get(..12).unwrap_or(revision)
}

/// The section of `docs/upstream-updates.md` for one update. How each adaptation was resolved is the
/// recheck's to write, and the section says where.
fn section(old: &Revs, new: &Revs, comparison: &Comparison, surface: &SurfaceDiff, gate_passed: bool) -> String {
    let mut text = format!("## {} to {}\n\n", short(&old.effectcraft), short(&new.effectcraft));
    text.push_str(&format!(
        "- From: EffectCraft `{}`, FilmCraft `{}`\n- To: EffectCraft `{}`, FilmCraft `{}`\n",
        old.effectcraft, old.filmcraft, new.effectcraft, new.filmcraft
    ));
    text.push_str(&format!("- Gate: {}\n", if gate_passed { "passed" } else { "failed" }));
    text.push_str(&format!("- API: {}\n\n", if surface.breaking { "breaking change: the next release is a major version" } else { "compatible" }));
    text.push_str("### Adaptations\n\n");
    if comparison.recheck.is_empty() {
        text.push_str("None named.\n\n");
    }
    for recheck in &comparison.recheck {
        text.push_str(&format!("- {} ({}): resolved by: (the recheck writes this)\n", recheck.id, recheck.crate_name));
        for touched in &recheck.touched {
            text.push_str(&format!("  - {} ({}, {})\n", touched.item, touched.file, touched.place));
        }
    }
    if !comparison.recheck.is_empty() {
        text.push('\n');
    }
    if !comparison.moved.is_empty() {
        text.push_str("Lines to refresh in `tools/adaptations.toml`, which the revision change moved without touching their items:\n\n");
        for moved in &comparison.moved {
            text.push_str(&format!("- {}: {} ({}): listed at {}, now at {}\n", moved.id, moved.item, moved.file, moved.from, moved.to));
        }
        text.push('\n');
    }
    text.push_str("### Commands added, changed and removed\n\n");
    if surface.report.is_empty() {
        text.push_str("None.\n");
    } else {
        text.push_str(&format!("```\n{}\n```\n", surface.report));
    }
    text.push('\n');
    text
}

/// What to print: the adaptations to recheck with their diffs, the API change, the gate, and the versions.
fn report(old: &Revs, new: &Revs, comparison: &Comparison, surface: &SurfaceDiff, gate_passed: bool, newer: Option<&[Newer]>) -> String {
    let mut text = format!("upstream: EffectCraft {} to {}\n", short(&old.effectcraft), short(&new.effectcraft));
    if comparison.recheck.is_empty() {
        text.push_str("upstream: no adaptation relies on anything the revision change touched\n");
    }
    for recheck in &comparison.recheck {
        text.push_str(&format!("upstream: RECHECK {} ({}) against upstream's new code\n", recheck.id, recheck.crate_name));
        for touched in &recheck.touched {
            text.push_str(&format!("  {} ({}, {})\n{}\n", touched.item, touched.file, touched.place, touched.diff.trim_end()));
        }
    }
    for moved in &comparison.moved {
        text.push_str(&format!(
            "upstream: {}: {} ({}) moved from lines {} to {}: write the new lines in tools/adaptations.toml\n",
            moved.id, moved.item, moved.file, moved.from, moved.to
        ));
    }
    text.push_str(&format!("upstream: API {}\n", if surface.breaking { "BREAKING: the next release is a major version" } else { "compatible" }));
    if !surface.report.is_empty() {
        text.push_str(&format!("{}\n", surface.report));
    }
    if !gate_passed {
        text.push_str("upstream: GATE FAILED: the failing stages and scenarios are above; the tree holds the new pin\n");
    }
    for newer in newer.unwrap_or_default() {
        text.push_str(&format!("upstream: newer release: {}\n", newer.line()));
    }
    if newer.is_some_and(<[Newer]>::is_empty) {
        text.push_str("upstream: every version of the Android build is at its newest release\n");
    }
    text
}

/// Said beside the failure of any step from the pin on. Such a failure can leave the tree holding the new
/// pin, and a run on a tree that holds it stops as up to date, so the steps after it would never run.
const UNFINISHED: &str = "steps 1 and 2 may have moved the pin, and a run on the revision the tree holds stops as up to date: put back what this run changed (git status names it) before running it again";

/// Take `rev`, step by step, and say how it ended.
pub(crate) fn update(steps: &dyn Steps, rev: &str) -> Result<Outcome, String> {
    let old = steps.before()?;
    from_the_pin(steps, rev, &old).map_err(|error| format!("{error}\nupstream: {UNFINISHED}"))
}

/// Every step from the pin on, which is where a failure can leave the tree changed.
fn from_the_pin(steps: &dyn Steps, rev: &str, old: &Revs) -> Result<Outcome, String> {
    let new = steps.pin(rev)?;
    if old.same_as(&new) {
        return Ok(Outcome {
            state: State::UpToDate,
            report: format!("upstream: up to date at EffectCraft {}: nothing changed\n", short(&new.effectcraft)),
            unchecked: None,
        });
    }
    let comparison = steps.adaptations(old, &new)?;
    let surface = steps.surface()?;
    let gate_passed = steps.gate()?;
    steps.record(&section(old, &new, &comparison, &surface, gate_passed))?;
    if !gate_passed {
        return Ok(Outcome { state: State::GateFailed, report: report(old, &new, &comparison, &surface, false, None), unchecked: None });
    }
    let (newer, unchecked) = match steps.versions() {
        Ok(newer) => (Some(newer), None),
        Err(error) => (None, Some(format!("the newer releases of the Android build could not be looked up: {error}"))),
    };
    let state = if surface.breaking {
        State::Breaking
    } else if comparison.recheck.is_empty() {
        State::Clean
    } else {
        State::Recheck
    };
    Ok(Outcome { state, report: report(old, &new, &comparison, &surface, true, newer.as_deref()), unchecked })
}

/// The caches of the official repositories read for two revisions: what [`Revisions`] asks of git.
struct CacheRevisions {
    /// Each repository whose revision the update compares, with its cache and its old and new commits. A
    /// repository not here kept the revision it had, so nothing in it is compared or looked up.
    changed: Vec<(Repository, crate::upstream::Cache, String, String)>,
}

impl CacheRevisions {
    fn of(&self, repository: Repository) -> Option<(&crate::upstream::Cache, &str, &str)> {
        self.changed.iter().find(|(known, ..)| *known == repository).map(|(_, cache, old, new)| (cache, old.as_str(), new.as_str()))
    }
}

impl Revisions for CacheRevisions {
    fn holds(&self, repository: Repository, file: &str) -> Result<bool, String> {
        Ok(self.of(repository).is_none_or(|(cache, old, _)| cache.git(&["cat-file", "-e", &format!("{old}:{file}")]).is_ok()))
    }

    fn diff(&self, repository: Repository, file: &str) -> Result<String, String> {
        let Some((cache, old, new)) = self.of(repository) else { return Ok(String::new()) };
        // `--text`: a file git would call binary is still compared line by line, never passed over as
        // unchanged.
        cache.git(&["diff", "--no-color", "--no-ext-diff", "--text", "-U0", old, new, "--", file])
    }
}

/// The Craft-library sides a list reads places in: the libraries as each app that relies on them builds
/// them, once each.
fn library_sides(list: &[Adaptation]) -> Vec<Repository> {
    let mut sides: Vec<Repository> = Vec::new();
    for location in list.iter().flat_map(|adaptation| &adaptation.relies).flat_map(|relies| &relies.at) {
        if location.repository.is_libraries() && !sides.contains(&location.repository) {
            sides.push(location.repository);
        }
    }
    sides
}

/// The commits the resolved graph builds the Craft libraries at for `side`'s app: each library package
/// one of the app's own crates depends on, at the commit Cargo resolved it to.
fn library_commits(crates: &[crate::meta::Crate], side: Repository) -> Result<BTreeSet<String>, String> {
    let app = side.app().spelt().ok_or("the layer table names no repository for the app")?;
    let prefix = crate::upstream::library_source_prefix();
    Ok(crates
        .iter()
        .filter(|krate| krate.source.as_deref().is_some_and(|source| crate::layers::from_repository(source, app)))
        .flat_map(|krate| &krate.depends_on)
        .filter_map(|id| crates.iter().find(|library| &library.id == id))
        .filter_map(|library| library.source.as_deref().filter(|source| source.starts_with(&prefix)))
        .filter_map(|source| source.split_once('#').map(|(_, commit)| commit.to_string()))
        .collect())
}

/// The one commit `commits` holds for `side`, which a place in the libraries is read at. An app that built
/// the libraries at two revisions would leave a place unable to say which it means.
fn the_one_commit(commits: BTreeSet<String>, side: Repository) -> Result<String, String> {
    let named: Vec<String> = commits.iter().cloned().collect();
    let mut each = commits.into_iter();
    match (each.next(), each.next()) {
        (Some(commit), None) => Ok(commit),
        _ => Err(format!(
            "{}'s crates build the Craft libraries at {} revisions ({}), so a place in the libraries cannot say which it is read at",
            side.app().spelt().unwrap_or("an app"),
            named.len(),
            named.join(", ")
        )),
    }
}

/// The commit each library side of `list` is built at in the workspace as it stands.
fn library_revisions(root: &Path, list: &[Adaptation]) -> Result<Vec<(Repository, String)>, String> {
    let sides = library_sides(list);
    if sides.is_empty() {
        return Ok(Vec::new());
    }
    let crates = crate::meta::read(&root.join("rust"), crate::meta::Graph::Resolved)?;
    sides.into_iter().map(|side| Ok((side, the_one_commit(library_commits(&crates, side)?, side)?))).collect()
}

/// The steps as a real run does them, in the repository at `root`.
struct Live {
    root: PathBuf,
    /// The surface the tree held before the pin moved, kept for step 4.
    kept: RefCell<Option<PathBuf>>,
    /// The commit each Craft-library side of the list was built at before the pin moved, kept for step 3.
    libraries: RefCell<Vec<(Repository, String)>>,
}

impl Live {
    fn revs(&self) -> Result<Revs, String> {
        let text = crate::scan::read_text(&self.root.join("rust/Cargo.toml"))?;
        let pin = crate::upstream::pins_from_manifest(&text).map_err(|errors| errors.join("\n"))?;
        let effectcraft = crate::upstream::Cache::create()?.official_commit_fetching(&pin.effectcraft_rev)?;
        Ok(Revs { effectcraft, filmcraft: pin.filmcraft_rev })
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        if let Some(kept) = self.kept.get_mut().take() {
            // A leftover scratch folder costs nothing but space.
            let _ = std::fs::remove_dir_all(kept);
        }
    }
}

impl Steps for Live {
    fn before(&self) -> Result<Revs, String> {
        let old = self.revs()?;
        let cache = crate::upstream::Cache::create()?;
        let list = crate::adaptations::read(&self.root)?;
        *self.libraries.borrow_mut() = library_revisions(&self.root, &list)?;
        let revisions = CacheRevisions { changed: vec![(Repository::Effectcraft, cache, old.effectcraft.clone(), old.effectcraft.clone())] };
        let absent = crate::adaptations::missing(&list, &revisions)?;
        if let Some((id, file)) = absent.first() {
            return Err(format!(
                "{}: {id} lists {file}, which EffectCraft {} does not hold: correct the list before the pin moves",
                crate::adaptations::LIST,
                short(&old.effectcraft)
            ));
        }
        let kept = crate::scratch::create("update-surface")?;
        std::fs::copy(self.root.join(crate::effects_api::SURFACE_FILE), kept.join("api-surface.json"))
            .map_err(|error| format!("cannot keep the API surface the tree holds: {error}"))?;
        *self.kept.borrow_mut() = Some(kept);
        Ok(old)
    }

    fn pin(&self, rev: &str) -> Result<Revs, String> {
        println!("upstream: {}", crate::upstream::pin(&self.root, rev)?);
        self.revs()
    }

    fn adaptations(&self, old: &Revs, new: &Revs) -> Result<Comparison, String> {
        let mut changed = vec![(Repository::Effectcraft, crate::upstream::Cache::create()?, old.effectcraft.clone(), new.effectcraft.clone())];
        if !old.same_filmcraft(new) {
            let film = crate::upstream::Cache::create_film()?;
            let (old_film, new_film) = (film.official_commit_fetching(&old.filmcraft)?, film.official_commit_fetching(&new.filmcraft)?);
            changed.push((Repository::Filmcraft, film, old_film, new_film));
        }
        let list = crate::adaptations::read(&self.root)?;
        let now = library_revisions(&self.root, &list)?;
        for (side, before) in self.libraries.borrow().iter() {
            let Some((_, after)) = now.iter().find(|(known, _)| known == side) else { continue };
            if !same_revision(before, after) {
                let libraries = crate::upstream::Cache::create_libraries()?;
                let (old_libraries, new_libraries) = (libraries.official_commit_fetching(before)?, libraries.official_commit_fetching(after)?);
                changed.push((*side, libraries, old_libraries, new_libraries));
            }
        }
        let revisions = CacheRevisions { changed };
        if let Some((id, file)) = crate::adaptations::missing(&list, &revisions)?.first() {
            return Err(format!("{}: {id} lists {file}, which the revision the tree held does not have", crate::adaptations::LIST));
        }
        crate::adaptations::compare(&list, &revisions)
    }

    fn surface(&self) -> Result<SurfaceDiff, String> {
        if crate::generate::run(&self.root, false) != ExitCode::SUCCESS {
            return Err("cargo xtask generate failed on the new revision: the API surface and the facades are not regenerated".to_string());
        }
        let kept = self.kept.borrow().clone().ok_or("the API surface of the old revision was not kept")?;
        crate::effects_api::diff(&self.root, &kept.join("api-surface.json"), &self.root.join(crate::effects_api::SURFACE_FILE))
    }

    fn gate(&self) -> Result<bool, String> {
        std::process::Command::new(self.root.join("tools/ci.sh"))
            .current_dir(&self.root)
            .status()
            .map(|status| status.success())
            .map_err(|error| format!("cannot run tools/ci.sh: {error}"))
    }

    fn record(&self, section: &str) -> Result<(), String> {
        append_record(&self.root, section)
    }

    fn versions(&self) -> Result<Vec<Newer>, String> {
        crate::versions::newer_releases_in(&self.root, &crate::versions::curl)
    }
}

/// The heading a record starts with, written once when the file is made.
const RECORD_HEADING: &str = "# Upstream updates\n\nEach EffectCraft revision `tools/update-upstream.sh` has taken, newest last: the revisions it moved between, the gate's result, the commands added, changed and removed, and each adaptation it named with how the recheck resolved it (plan \u{a7}24).\n\n";

/// Append `section` to the record, which is made with its heading when it does not exist yet.
fn append_record(root: &Path, section: &str) -> Result<(), String> {
    let path = root.join(RECORD);
    let mut text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => RECORD_HEADING.to_string(),
        Err(error) => return Err(format!("cannot read {RECORD}: {error}")),
    };
    text.push_str(section);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
    }
    std::fs::write(&path, text).map_err(|error| format!("cannot write {RECORD}: {error}"))
}

/// The exit code of a run that ended with `result`: its outcome's, or 1 for a step that could not finish.
fn code_of(result: &Result<Outcome, String>) -> u8 {
    result.as_ref().map_or(1, Outcome::code)
}

/// `cargo xtask upstream update <rev>` as a command.
pub(crate) fn run(root: &Path, rev: &str) -> ExitCode {
    let live = Live { root: root.to_path_buf(), kept: RefCell::new(None), libraries: RefCell::new(Vec::new()) };
    let result = update(&live, rev);
    match &result {
        Ok(outcome) => {
            print!("{}", outcome.report);
            if let Some(why) = &outcome.unchecked {
                eprintln!("upstream: {why}");
            }
        }
        Err(error) => eprintln!("upstream: {error}"),
    }
    ExitCode::from(code_of(&result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptations::{Moved, Recheck, Touched};

    fn revs(effectcraft: &str) -> Revs {
        Revs { effectcraft: format!("{effectcraft:0<40}"), filmcraft: "d5ead4ba".to_string() }
    }

    /// Steps that record what they were asked and answer from fixtures.
    struct Fake {
        held: Revs,
        taken: Revs,
        recheck: Vec<Recheck>,
        moved: Vec<Moved>,
        breaking: bool,
        gate: bool,
        versions: Result<Vec<Newer>, String>,
        /// The step that fails, when one does.
        fails: Option<&'static str>,
        calls: RefCell<Vec<&'static str>>,
        recorded: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                held: revs("1"),
                taken: revs("2"),
                recheck: Vec::new(),
                moved: Vec::new(),
                breaking: false,
                gate: true,
                versions: Ok(Vec::new()),
                fails: None,
                calls: RefCell::default(),
                recorded: RefCell::default(),
            }
        }
        fn called(&self, step: &'static str) -> Result<(), String> {
            self.calls.borrow_mut().push(step);
            if self.fails == Some(step) { Err(format!("the {step} step failed")) } else { Ok(()) }
        }
    }

    impl Steps for Fake {
        fn before(&self) -> Result<Revs, String> {
            self.called("before")?;
            Ok(self.held.clone())
        }
        fn pin(&self, _: &str) -> Result<Revs, String> {
            self.called("pin")?;
            Ok(self.taken.clone())
        }
        fn adaptations(&self, _: &Revs, _: &Revs) -> Result<Comparison, String> {
            self.called("adaptations")?;
            Ok(Comparison { recheck: self.recheck.clone(), moved: self.moved.clone() })
        }
        fn surface(&self) -> Result<SurfaceDiff, String> {
            self.called("surface")?;
            Ok(SurfaceDiff { breaking: self.breaking, report: if self.breaking { "removed command anim.addKeyframe".to_string() } else { String::new() } })
        }
        fn gate(&self) -> Result<bool, String> {
            self.called("gate")?;
            Ok(self.gate)
        }
        fn record(&self, section: &str) -> Result<(), String> {
            self.called("record")?;
            self.recorded.borrow_mut().push(section.to_string());
            Ok(())
        }
        fn versions(&self) -> Result<Vec<Newer>, String> {
            self.called("versions")?;
            self.versions.clone()
        }
    }

    fn touched_adaptation() -> Recheck {
        Recheck {
            id: "AD1".to_string(),
            crate_name: "craftbridge-effects".to_string(),
            touched: vec![Touched {
                item: "Session".to_string(),
                file: "crates/engine/src/lib.rs".to_string(),
                place: "lines 104-269".to_string(),
                diff: "@@ -110 +110 @@\n-a\n+b\n".to_string(),
            }],
        }
    }

    #[test]
    fn a_run_on_the_revision_the_tree_holds_changes_and_reads_nothing() {
        let mut fake = Fake::new();
        fake.taken = fake.held.clone();
        let outcome = update(&fake, "1").unwrap();
        assert_eq!((outcome.state, outcome.code()), (State::UpToDate, 0));
        assert_eq!(*fake.calls.borrow(), ["before", "pin"], "no adaptation compared, nothing regenerated, no gate, no record, no network");
        assert!(fake.recorded.borrow().is_empty());
        let mut abbreviated = Fake::new();
        abbreviated.taken = Revs { effectcraft: abbreviated.held.effectcraft.get(..8).unwrap_or_default().to_string(), ..abbreviated.held.clone() };
        assert_eq!(update(&abbreviated, "1").unwrap().state, State::UpToDate, "an abbreviated spelling of the revision held is the same revision");
    }

    #[test]
    fn a_pin_that_moved_only_filmcraft_is_still_an_update() {
        let mut fake = Fake::new();
        fake.taken = Revs { filmcraft: "0123456789ab".to_string(), ..fake.held.clone() };
        assert_eq!(update(&fake, "1").unwrap().state, State::Clean, "the pin wrote a new FilmCraft revision, so the tree changed");
        assert_eq!(fake.calls.borrow().first(), Some(&"before"));
        assert!(fake.calls.borrow().contains(&"adaptations"), "{:?}", fake.calls.borrow());
    }

    #[test]
    fn a_clean_update_runs_every_step_in_order_and_records_it() {
        let fake = Fake::new();
        let outcome = update(&fake, "2").unwrap();
        assert_eq!((outcome.state, outcome.code()), (State::Clean, 0));
        assert_eq!(*fake.calls.borrow(), ["before", "pin", "adaptations", "surface", "gate", "record", "versions"]);
        let recorded = fake.recorded.borrow().join("");
        assert!(recorded.contains("- Gate: passed") && recorded.contains("- API: compatible") && recorded.contains("None named."), "{recorded}");
        assert!(outcome.report.contains("every version of the Android build is at its newest release"), "{}", outcome.report);
    }

    #[test]
    fn an_adaptation_the_revision_touched_is_named_and_the_run_still_completes() {
        let mut fake = Fake::new();
        fake.recheck = vec![touched_adaptation()];
        fake.moved = vec![Moved {
            id: "AD6".to_string(),
            item: "LayerCache".to_string(),
            file: "crates/render/src/cache.rs".to_string(),
            from: "39".to_string(),
            to: "41".to_string(),
        }];
        fake.versions = Ok(vec![Newer {
            name: "agp".to_string(),
            current: "9.5.1".to_string(),
            newest: "9.5.2".to_string(),
            from: "com.android.tools.build:gradle".to_string(),
        }]);
        let outcome = update(&fake, "2").unwrap();
        assert_eq!((outcome.state, outcome.code()), (State::Recheck, 0), "the update merges once each is rechecked: the run itself succeeds");
        assert!(outcome.report.contains("RECHECK AD1") && outcome.report.contains("+b"), "the adaptation and its diff: {}", outcome.report);
        assert!(outcome.report.contains("agp 9.5.1") && outcome.report.contains("9.5.2"), "the version and its newer release: {}", outcome.report);
        assert!(outcome.report.contains("AD6: LayerCache (crates/render/src/cache.rs) moved from lines 39 to 41"), "{}", outcome.report);
        let recorded = fake.recorded.borrow().join("");
        assert!(recorded.contains("- AD1 (craftbridge-effects): resolved by:"), "the record names it for the recheck to resolve: {recorded}");
        assert!(recorded.contains("  - Session (crates/engine/src/lib.rs, lines 104-269)\n"), "with each touched place: {recorded}");
        assert!(recorded.contains("- AD6: LayerCache (crates/render/src/cache.rs): listed at 39, now at 41"), "and the lines to refresh: {recorded}");
    }

    #[test]
    fn a_breaking_surface_ends_the_run_with_exit_2() {
        let mut fake = Fake::new();
        fake.breaking = true;
        let outcome = update(&fake, "2").unwrap();
        assert_eq!((outcome.state, outcome.code()), (State::Breaking, 2));
        assert!(outcome.report.contains("BREAKING") && outcome.report.contains("removed command anim.addKeyframe"), "{}", outcome.report);
        let recorded = fake.recorded.borrow().join("");
        assert!(recorded.contains("breaking change") && recorded.contains("removed command anim.addKeyframe"), "{recorded}");
    }

    #[test]
    fn a_failed_gate_stops_the_run_with_its_failure_recorded_and_no_version_lookup() {
        let mut fake = Fake::new();
        fake.gate = false;
        fake.breaking = true;
        let outcome = update(&fake, "2").unwrap();
        assert_eq!((outcome.state, outcome.code()), (State::GateFailed, 1), "a failing gate outranks a breaking change");
        assert!(!fake.calls.borrow().contains(&"versions"));
        assert!(fake.recorded.borrow().join("").contains("- Gate: failed"));
    }

    #[test]
    fn versions_that_could_not_be_looked_up_fail_the_run_after_the_update_is_recorded() {
        let mut fake = Fake::new();
        fake.versions = Err("offline".to_string());
        let outcome = update(&fake, "2").unwrap();
        assert_eq!(outcome.code(), 1);
        assert!(outcome.unchecked.is_some_and(|why| why.contains("offline")));
        assert_eq!(fake.recorded.borrow().len(), 1);
    }

    #[test]
    fn the_record_is_made_with_its_heading_and_each_update_is_appended() {
        let scratch = crate::scratch::Scratch::new("update-record");
        append_record(&scratch.0, "## one\n\n").unwrap();
        append_record(&scratch.0, "## two\n\n").unwrap();
        let text = std::fs::read_to_string(scratch.0.join(RECORD)).unwrap();
        assert!(text.starts_with("# Upstream updates") && text.ends_with("## one\n\n## two\n\n"), "{text}");
    }

    /// A bare scratch cache of the official repository whose main holds `first` and then, in a second
    /// commit, exactly `second`: the two revisions an update moves between.
    fn cache_with_two_revisions(first: &[(&str, &str)], second: &[(&str, &str)]) -> (crate::scratch::Scratch, String, String) {
        let cache = crate::upstream::official_scratch("update-cache");
        let old = crate::git::commit_files(&cache.0, None, "old", first);
        let new = crate::git::commit_files(&cache.0, Some(&old), "new", second);
        crate::git::output(&cache.0, &["--git-dir=.", "update-ref", "refs/heads/main", &new]).unwrap();
        (cache, old, new)
    }

    fn numbered(lines: usize, changed: Option<usize>) -> String {
        (1..=lines).map(|line| if Some(line) == changed { format!("changed {line}\n") } else { format!("line {line}\n") }).collect()
    }

    const FIXTURE_LIST: &str = r#"
[[adaptation]]
id = "AD1"
crate = "craftbridge-effects"
depends = [{ item = "Session", at = [{ file = "crates/engine/src/lib.rs", lines = ["10-20"] }] }]

[[adaptation]]
id = "AD2"
crate = "craftbridge-effects"
depends = [{ item = "Backend", at = [{ file = "crates/automation/src/backend.rs", lines = ["5-8"] }] }]

[[adaptation]]
id = "AD3"
crate = "craftbridge-effects"
depends = [{ item = "queue", at = [{ file = "crates/engine/src/render_queue.rs" }] }]
"#;

    /// The fixture list compared, with real git, between a revision holding its three files and one
    /// holding `second` instead.
    fn compared_with(second: &[(&str, &str)]) -> Comparison {
        let backend = numbered(30, None);
        let first = [
            ("crates/engine/src/lib.rs", numbered(40, None)),
            ("crates/automation/src/backend.rs", backend.clone()),
            ("crates/engine/src/render_queue.rs", backend),
        ];
        let first: Vec<(&str, &str)> = first.iter().map(|(name, text)| (*name, text.as_str())).collect();
        let (cache, old, new) = cache_with_two_revisions(&first, second);
        let cache = crate::upstream::Cache::at(cache.0.clone()).unwrap();
        let revisions = CacheRevisions { changed: vec![(Repository::Effectcraft, cache, old, new)] };
        let list = crate::adaptations::parse(FIXTURE_LIST).unwrap();
        assert!(crate::adaptations::missing(&list, &revisions).unwrap().is_empty());
        crate::adaptations::compare(&list, &revisions).unwrap()
    }

    /// The same, with only the first adaptation's file changed to `session`.
    fn compared(session: &str) -> Comparison {
        let backend = numbered(30, None);
        compared_with(&[("crates/engine/src/lib.rs", session), ("crates/automation/src/backend.rs", &backend), ("crates/engine/src/render_queue.rs", &backend)])
    }

    /// A place in the Craft libraries is read at the revision its entry's app builds them at, which the
    /// resolved graph says: in the workspace, FilmCraft's crates build them at a revision of their own.
    #[test]
    fn the_libraries_are_read_at_the_revision_each_app_builds_them_at() {
        let crates = crate::meta::read(&crate::repo_root().join("rust"), crate::meta::Graph::Resolved).unwrap();
        let lock = std::fs::read_to_string(crate::repo_root().join("rust/Cargo.lock")).unwrap();
        for side in [Repository::EffectcraftLibraries, Repository::FilmcraftLibraries] {
            let commits = library_commits(&crates, side).unwrap();
            assert_eq!(commits.len(), 1, "{side:?}: {commits:?}");
            let commit = commits.iter().next().unwrap();
            assert!(lock.contains(&format!("craft-libs?rev={commit}#{commit}")), "{side:?} reads a revision the lock holds: {commit}");
        }
        let film = library_commits(&crates, Repository::FilmcraftLibraries).unwrap();
        assert_ne!(film, library_commits(&crates, Repository::EffectcraftLibraries).unwrap(), "each app builds them at its own revision");
        assert_eq!(the_one_commit(film.clone(), Repository::FilmcraftLibraries), Ok(film.into_iter().next().unwrap()));
        let two: BTreeSet<String> = ["aaaa".to_string(), "bbbb".to_string()].into();
        let refused = the_one_commit(two, Repository::FilmcraftLibraries).unwrap_err();
        assert!(refused.contains("aaaa") && refused.contains("bbbb") && refused.contains("filmcraft"), "{refused}");
        assert!(the_one_commit(BTreeSet::new(), Repository::EffectcraftLibraries).is_err(), "an app that builds no library has no revision to read one at");
    }

    #[test]
    fn a_list_names_each_library_side_it_reads_once_and_a_repository_left_alone_is_not_compared() {
        let list = crate::adaptations::parse(
            "[[adaptation]]\nid = \"AD1\"\ncrate = \"c\"\nrepository = \"storytold/filmcraft\"\ndepends = [\n  { item = \"a\", at = [{ repository = \"storytold/craft-libs\", file = \"crates/media/src/a.rs\" }] },\n  { item = \"b\", at = [{ repository = \"storytold/craft-libs\", file = \"crates/media/src/b.rs\" }, { file = \"crates/codecs/src/lib.rs\" }] },\n]\n",
        )
        .unwrap();
        assert_eq!(library_sides(&list), [Repository::FilmcraftLibraries]);
        let nothing = CacheRevisions { changed: Vec::new() };
        assert_eq!(nothing.holds(Repository::FilmcraftLibraries, "crates/media/src/a.rs"), Ok(true));
        assert_eq!(nothing.diff(Repository::FilmcraftLibraries, "crates/media/src/a.rs"), Ok(String::new()));
    }

    #[test]
    fn a_fixture_revision_that_touches_an_item_of_an_adaptation_names_it() {
        let found = compared(&numbered(40, Some(15)));
        assert_eq!(found.recheck.iter().map(|recheck| recheck.id.as_str()).collect::<Vec<_>>(), ["AD1"], "only the adaptation whose lines changed");
        assert!(found.recheck.first().and_then(|recheck| recheck.touched.first()).is_some_and(|touched| touched.diff.contains("+changed 15")));
    }

    #[test]
    fn a_fixture_revision_that_changes_a_listed_file_below_its_lines_names_nothing() {
        assert_eq!(compared(&numbered(40, Some(30))), Comparison::default());
    }

    #[test]
    fn a_fixture_revision_that_adds_lines_above_an_item_names_where_they_moved() {
        let found = compared(&format!("new 1\nnew 2\nnew 3\n{}", numbered(40, None)));
        assert!(found.recheck.is_empty(), "{found:?}");
        assert_eq!(
            found.moved.iter().map(|moved| (moved.id.as_str(), moved.from.as_str(), moved.to.as_str())).collect::<Vec<_>>(),
            [("AD1", "10-20", "13-23")]
        );
    }

    #[test]
    fn a_fixture_revision_that_changes_no_listed_file_names_nothing() {
        assert_eq!(compared(&numbered(40, None)), Comparison::default());
    }

    #[test]
    fn a_fixture_revision_that_deletes_or_renames_a_listed_file_names_its_adaptation() {
        let session = numbered(40, None);
        let found = compared_with(&[("crates/engine/src/lib.rs", &session), ("crates/engine/src/queue/mod.rs", &numbered(30, None))]);
        assert_eq!(
            found.recheck.iter().map(|recheck| recheck.id.as_str()).collect::<Vec<_>>(),
            ["AD2", "AD3"],
            "a file gone from its path, deleted or under another name, is a change to all of it: {found:?}"
        );
    }

    #[test]
    fn a_fixture_revision_that_changes_a_file_git_calls_binary_still_names_it() {
        let with_nul: String =
            (1..=40).map(|line| if line == 15 { "\0 a NUL byte on a listed line\n".to_string() } else { format!("line {line}\n") }).collect();
        let found = compared(&with_nul);
        assert_eq!(found.recheck.iter().map(|recheck| recheck.id.as_str()).collect::<Vec<_>>(), ["AD1"], "{found:?}");
    }

    #[test]
    fn a_step_that_fails_from_the_pin_on_says_how_to_run_the_update_again() {
        let mut fake = Fake::new();
        fake.fails = Some("surface");
        let failed = update(&fake, "2");
        assert!(failed.as_ref().is_err_and(|error| error.contains("the surface step failed") && error.contains(UNFINISHED)), "{failed:?}");
        assert_eq!(code_of(&failed), 1);
        assert!(fake.recorded.borrow().is_empty(), "nothing is recorded for a run that did not finish");
        let mut early = Fake::new();
        early.fails = Some("before");
        assert!(update(&early, "2").is_err_and(|error| !error.contains(UNFINISHED)), "before the pin nothing has moved");
        assert_eq!(code_of(&update(&Fake::new(), "2")), 0);
    }

    #[test]
    fn spellings_of_one_commit_are_the_same_and_no_spelling_is_none() {
        assert!(same_revision("d5ead4ba", "d5ead4ba1ee2f1cd13cb86a4239aee2730a3d9f5") && same_revision("d5ead4ba1ee2", "d5ead4ba"));
        assert!(!same_revision("d5ead4ba", "d5ead4bb") && !same_revision("", "d5ead4ba") && !same_revision("", ""));
    }

    /// The script's own misuse ends with 64, so 2 says only that the API broke; it stops before it builds
    /// anything.
    #[test]
    fn the_script_misused_exits_64() {
        let script = crate::repo_root().join("tools/update-upstream.sh");
        for args in [&[][..], &["10376058", "again"][..]] {
            let output = std::process::Command::new("bash").arg(&script).args(args).output().unwrap();
            assert_eq!(output.status.code(), Some(64), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
            assert!(String::from_utf8_lossy(&output.stderr).contains("usage: tools/update-upstream.sh"));
        }
    }
}
