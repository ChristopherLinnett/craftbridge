//! `cargo xtask upstream update <rev>`: the whole upstream update of plan §24, which
//! `tools/update-upstream.sh` runs. Steps 1 and 2 are [`crate::upstream::pin`]; this module adds the rest
//! and decides how the run ends.
//!
//! 1. and 2. move the pin and write what comes from it,
//! 3. name the live adaptations whose upstream items the revision change touches, and list the list's
//!    retired entries apart ([`crate::adaptations`]),
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

/// What to print: the list's retired entries apart, then the live adaptations to recheck with their
/// diffs, the API change, the gate, and the versions.
fn report(old: &Revs, new: &Revs, comparison: &Comparison, surface: &SurfaceDiff, gate_passed: bool, newer: Option<&[Newer]>) -> String {
    let mut text = format!("upstream: EffectCraft {} to {}\n", short(&old.effectcraft), short(&new.effectcraft));
    for retired in &comparison.retired {
        text.push_str(&format!("upstream: retired: {} replaced upstream by {}\n", retired.id, retired.replaced_by));
    }
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
    use crate::adaptations::{Moved, Recheck, Retired, Touched};

    fn revs(effectcraft: &str) -> Revs {
        Revs { effectcraft: format!("{effectcraft:0<40}"), filmcraft: "d5ead4ba".to_string() }
    }

    /// Steps that record what they were asked and answer from fixtures.
    struct Fake {
        held: Revs,
        taken: Revs,
        recheck: Vec<Recheck>,
        moved: Vec<Moved>,
        retired: Vec<Retired>,
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
                retired: Vec::new(),
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
            Ok(Comparison { recheck: self.recheck.clone(), moved: self.moved.clone(), retired: self.retired.clone() })
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
    fn the_report_lists_retired_entries_apart_from_the_live_ones() {
        let mut fake = Fake::new();
        fake.recheck = vec![touched_adaptation()];
        fake.retired = vec![Retired { id: "AD5".to_string(), replaced_by: "0d1e2f3a4b5c".to_string() }];
        let outcome = update(&fake, "2").unwrap();
        let lines: Vec<&str> = outcome.report.lines().collect();
        assert!(lines.contains(&"upstream: retired: AD5 replaced upstream by 0d1e2f3a4b5c"), "the retired entry on its own line: {}", outcome.report);
        assert!(lines.iter().any(|line| line.starts_with("upstream: RECHECK AD1")), "the live adaptation is still named apart: {}", outcome.report);
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

    /// Whether a place is EffectCraft's command line, whose code the entries that mirror it list.
    fn command_line(place: &crate::adaptations::Location) -> bool {
        place.repository == Repository::Effectcraft && place.file.starts_with("apps/effectcraft-cli/")
    }

    /// The ids a printed report names for a recheck, each read off its own `RECHECK` line, so one id
    /// that begins another (`AD1`, `AD13`) is never taken for it.
    fn rechecked(printed: &str) -> BTreeSet<&str> {
        printed.lines().filter_map(|line| line.strip_prefix("upstream: RECHECK ")).filter_map(|rest| rest.split(' ').next()).collect()
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

    /// The given list compared, with real git, between a revision holding the numbered fixture files
    /// and one holding `second` instead.
    fn compared_against(list_text: &str, second: &[(&str, &str)]) -> Comparison {
        let session = numbered(40, None);
        let backend = numbered(30, None);
        let first = [
            ("crates/engine/src/lib.rs", session.as_str()),
            ("crates/automation/src/backend.rs", backend.as_str()),
            ("crates/engine/src/render_queue.rs", backend.as_str()),
        ];
        let (cache, old, new) = cache_with_two_revisions(&first, second);
        let cache = crate::upstream::Cache::at(cache.0.clone()).unwrap();
        let revisions = CacheRevisions { changed: vec![(Repository::Effectcraft, cache, old, new)] };
        let list = crate::adaptations::parse(list_text).unwrap();
        assert!(crate::adaptations::missing(&list, &revisions).unwrap().is_empty());
        crate::adaptations::compare(&list, &revisions).unwrap()
    }

    /// The fixture list compared, with real git, between a revision holding its three files and one
    /// holding `second` instead.
    fn compared_with(second: &[(&str, &str)]) -> Comparison {
        compared_against(FIXTURE_LIST, second)
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

    /// A fixture list with a retired entry, compared against a revision that changes a live entry's
    /// lines: the live one is named for the recheck and the retired one is shown apart, in the
    /// comparison and in the report printed from it, and the retired entry is looked for in no file,
    /// since it lists none.
    #[test]
    fn a_fixture_revision_shows_a_retired_entry_apart_from_the_live_ones() {
        let backend = numbered(30, None);
        let session = numbered(40, Some(15));
        let found = compared_against(
            &format!("{FIXTURE_LIST}\n[[adaptation]]\nid = \"AD4\"\nretired = \"0d1e2f3a4b5c\"\n"),
            &[("crates/engine/src/lib.rs", &session), ("crates/automation/src/backend.rs", &backend), ("crates/engine/src/render_queue.rs", &backend)],
        );
        assert_eq!(found.recheck.iter().map(|recheck| recheck.id.as_str()).collect::<Vec<_>>(), ["AD1"], "the live adaptation whose lines changed");
        assert_eq!(found.retired, [Retired { id: "AD4".to_string(), replaced_by: "0d1e2f3a4b5c".to_string() }], "the retired entry, named apart");
        let printed = report(&revs("1"), &revs("2"), &found, &SurfaceDiff { breaking: false, report: String::new() }, true, None);
        assert_eq!(
            printed.lines().filter(|line| line.contains("AD4")).collect::<Vec<_>>(),
            ["upstream: retired: AD4 replaced upstream by 0d1e2f3a4b5c"],
            "the retired entry on a line of its own, and on no recheck or moved line: {printed}"
        );
        assert!(printed.lines().any(|line| line.starts_with("upstream: RECHECK AD1 ")), "the live entry rechecked: {printed}");
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

    /// The real list, compared between fixture revisions the way step 3 compares it between the pin's,
    /// with one line of EffectCraft's command line changed at a time: the first and the last line of
    /// every place the list names there, and the line on either side of each. Every plant names, in
    /// the update loop's report, exactly the adaptations with a place that holds the changed line, so a
    /// change inside the code an adaptation mirrors names it and a change beside that code names it
    /// only where another of its places holds the line. The places, the files and the fixture's length
    /// are read off the list, so the test states no second copy of any of them. Only the command
    /// line's places are compared: every other place lies in a file no fixture revision holds, so its
    /// diff is empty and it can name nothing.
    #[test]
    fn a_fixture_revision_in_the_command_line_code_the_real_list_names_names_those_adaptations() {
        let list: Vec<Adaptation> = crate::adaptations::read(&crate::repo_root())
            .unwrap()
            .into_iter()
            .map(|entry| Adaptation {
                relies: entry
                    .relies
                    .iter()
                    .map(|item| crate::adaptations::Relies {
                        item: item.item.clone(),
                        at: item.at.iter().filter(|place| command_line(place)).cloned().collect(),
                    })
                    .filter(|item| !item.at.is_empty())
                    .collect(),
                ..entry
            })
            .collect();
        let places: Vec<&crate::adaptations::Location> = list.iter().flat_map(|entry| &entry.relies).flat_map(|item| &item.at).collect();
        assert!(!places.is_empty(), "the list names no command-line code to plant in");
        // Long enough to hold every listed line and the line after the last of them.
        let length = places.iter().flat_map(|place| &place.lines).map(|span| span.ends().1).max().unwrap_or(1) + 2;
        let files: BTreeSet<&str> = places.iter().map(|place| place.file.as_str()).collect();
        let mut plants: BTreeSet<(&str, u64)> = BTreeSet::new();
        for place in &places {
            let edges: Vec<(u64, u64)> =
                if place.lines.is_empty() { vec![(1, length)] } else { place.lines.iter().map(crate::adaptations::Span::ends).collect() };
            for (first, last) in edges {
                let lines = [first - 1, first, last, last + 1].into_iter().filter(|line| (1..=length).contains(line));
                plants.extend(lines.map(|line| (place.file.as_str(), line)));
            }
        }
        let unchanged = numbered(length as usize, None);
        let before: Vec<(&str, &str)> = files.iter().map(|file| (*file, unchanged.as_str())).collect();
        let cache = crate::upstream::official_scratch("update-command-line-plants");
        let old = crate::git::commit_files(&cache.0, None, "old", &before);
        for (file, line) in plants {
            let changed = numbered(length as usize, Some(line as usize));
            let after: Vec<(&str, &str)> = files.iter().map(|name| (*name, if *name == file { changed.as_str() } else { unchanged.as_str() })).collect();
            let new = crate::git::commit_files(&cache.0, Some(&old), "new", &after);
            let revisions = CacheRevisions { changed: vec![(Repository::Effectcraft, crate::upstream::Cache::at(cache.0.clone()).unwrap(), old.clone(), new)] };
            let found = crate::adaptations::compare(&list, &revisions).unwrap();
            let holds = |place: &crate::adaptations::Location| {
                place.file == file
                    && (place.lines.is_empty()
                        || place.lines.iter().any(|span| {
                            let (first, last) = span.ends();
                            first <= line && line <= last
                        }))
            };
            let expected: BTreeSet<&str> =
                list.iter().filter(|entry| entry.relies.iter().flat_map(|item| &item.at).any(holds)).map(|entry| entry.id.as_str()).collect();
            assert_eq!(found.recheck.iter().map(|recheck| recheck.id.as_str()).collect::<BTreeSet<_>>(), expected, "{file} line {line}: {found:?}");
            assert!(found.moved.is_empty(), "{file} line {line}: a line changed in place moves nothing: {found:?}");
            let printed = report(&revs("old"), &revs("new"), &found, &SurfaceDiff { breaking: false, report: String::new() }, true, None);
            assert_eq!(rechecked(&printed), expected, "the report for a change to {file} line {line}: {printed}");
        }
    }

    /// The entries that follow the command line are the ones the loop must be able to name when it
    /// changes: an entry with no place in the command line's code would be named by no change to it,
    /// and the plants above, read off the list, would not miss it.
    #[test]
    fn every_entry_that_mirrors_the_command_line_lists_command_line_code() {
        let list = crate::adaptations::read(&crate::repo_root()).unwrap();
        for id in ["AD1", "AD3", "AD4", "AD5", "AD6", "AD10", "AD13"] {
            let entry = list.iter().find(|entry| entry.id == id).unwrap();
            assert!(entry.relies.iter().flat_map(|item| &item.at).any(command_line), "{id} lists no command-line code, though it mirrors the command line");
        }
    }

    /// The pinned EffectCraft as Cargo checked it out to build the workspace: the checkout holding a crate
    /// the resolved graph takes from EffectCraft's repository.
    fn pinned_effectcraft() -> PathBuf {
        let app = crate::layers::app_by_noun("effects").unwrap();
        let crates = crate::meta::read(&crate::repo_root().join("rust"), crate::meta::Graph::Resolved).unwrap();
        let built = crates.iter().find(|krate| krate.source.as_deref().is_some_and(|source| crate::layers::from_repository(source, app.repository))).unwrap();
        Path::new(&built.manifest_path).ancestors().find(|folder| crate::git::is_checkout(folder)).unwrap().to_path_buf()
    }

    /// What the language's own lexer finds in a source file: the first and last line of every bracketed
    /// group, at every depth (the blocks, the argument lists and the attributes), and the lines that
    /// hold a token, a doc comment included.
    #[derive(Default)]
    struct Code {
        groups: Vec<(usize, usize)>,
        token_lines: BTreeSet<usize>,
    }

    impl Code {
        fn read(&mut self, tokens: proc_macro2::TokenStream) {
            for tree in tokens {
                let span = tree.span();
                self.token_lines.extend(span.start().line..=span.end().line);
                if let proc_macro2::TokenTree::Group(group) = tree {
                    self.groups.push((group.span_open().start().line, group.span_close().end().line));
                    self.read(group.stream());
                }
            }
        }
    }

    /// Every place the list names in the command line is whole code at the pin: no group of its tokens
    /// opens inside the place and closes outside it, or the other way round, and the place starts and
    /// ends on a line of code, so it neither runs past the end of the file nor ends on a blank line.
    /// A place that stops short of the line closing its block lets a line added inside the block past,
    /// since an added line touches a span only between two of its lines, so the change names no
    /// adaptation; one that runs past the end of its code takes in code its entry does not mirror. The
    /// plants above read their places off the list, so only the pin's own code can say a place is cut.
    #[test]
    fn every_command_line_place_the_list_names_is_whole_code_at_the_pin() {
        let checkout = pinned_effectcraft();
        let list = crate::adaptations::read(&crate::repo_root()).unwrap();
        let mut cut = Vec::new();
        for entry in &list {
            for place in entry.relies.iter().flat_map(|item| &item.at).filter(|place| command_line(place)) {
                let text = std::fs::read_to_string(checkout.join(&place.file)).unwrap();
                let mut code = Code::default();
                code.read(text.parse().unwrap());
                for span in &place.lines {
                    let (first, last) = span.ends();
                    let inside = |line: usize| (first..=last).contains(&(line as u64));
                    for (open, close) in code.groups.iter().filter(|(open, close)| inside(*open) != inside(*close)) {
                        cut.push(format!("{} lists {} lines {}, which cut the group of lines {open}-{close}", entry.id, place.file, span.text));
                    }
                    for edge in [first, last].into_iter().filter(|edge| !code.token_lines.contains(&(*edge as usize))) {
                        cut.push(format!("{} lists {} lines {}, whose line {edge} holds no code", entry.id, place.file, span.text));
                    }
                }
            }
        }
        assert!(cut.is_empty(), "command-line places that are not whole code at the pin: {cut:#?}");
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
