//! Rule 6 — the plan's views are rebuilt from their sources and compared byte for byte, so no view holds a
//! hand edit (AGENTS.md §6, plan §15).
//!
//! A plan view is what the plan kit's own builders write from the master, `plan/EFFECTCRAFT-PLUGIN-PORT-
//! SCOPING.md`, and from the diagram sources in `plan/diagrams/src/`, and is never edited by hand: the
//! master is the one file that says what a task is. The kit's `plan check --final` weighs only a view's
//! *time* against its source's, so a hand edit made after the last build passes it and every later build
//! keeps it. This check copies the plan, runs the kit's builders in the copy in the order
//! `plan/package.json` names them, and compares every byte with the plan folder as it stands: whichever
//! file the rebuild writes and the folder holds differently fails here, named, whatever its time says. No
//! list of the views is kept here, and none is needed: what the builders wrote is what is compared, so a
//! view added to the master is checked from the change that adds it, and one taken out of it stops being
//! compared when the kit stops writing it.
//!
//! The diagrams' folder is the kit's alone (`diagramsOut` in its configuration, asked of the kit): all it
//! holds are the exports of the sources in `plan/diagrams/src/`. An export whose source is gone is one no
//! build writes again, which a hand could then edit unseen while the HTML goes on inlining its drawing, so
//! it fails too. A rewrite with the same bytes shows in nothing but the file's time, so each file of the
//! copy's diagrams' folder is dated to the epoch before the rebuild, and one still bearing that date after
//! it is one no source built: the kit writes both exports of every diagram it builds.
//!
//! The plan's files are the ones the rules take the repository to hold — git's tracked files and those it
//! would add (`hygiene::repository_files`) — so git's ignore files alone say what is no part of the plan
//! (the kit's renders in `review/` and its dependencies in `node_modules/`, by `plan/.gitignore`; the
//! deployed snapshots in `analysis/`, by the root's), and no second list here can drift from them.
//!
//! The tracked plan is never written. The scratch stands in `plan/node_modules/.cache/`, where npm's
//! packages keep their caches, and goes when the check ends. From there the rebuild finds the kit's
//! dependencies by walking up from its own folder, exactly as a build run in the plan folder does, with
//! nothing linked or copied in to reach them; and no listing of the plan — git's, the gate's walk, the
//! kit's own — looks inside a `node_modules`, so a second run's scratch, or one a stopped run left behind,
//! is never read as part of the plan, and the next check removes what a stopped run left
//! (`claim_scratch`).
//!
//! The rebuild needs Node and `npm ci` in `plan/`, which `tools/doctor.sh --gate` asks for, and nothing
//! else. `build/build-plan.mjs --no-shots` is the kit's own switch that keeps a browser down: mockup
//! screenshots are all it would start one for, and this plan has no mockup gallery, so the full build and
//! this one write the very same bytes. `build/build-diagrams.mjs` writes each diagram's draw.io source,
//! `<id>.drawio`, from `plan/diagrams/src/` itself, then has draw.io Desktop export the SVG and trims what
//! it exported; it starts whatever program `DRAWIO_BIN` names as that exporter, and here that is `true`,
//! which exports nothing. So everything the kit writes is weighed — each `.drawio`, and the trimming of
//! each committed SVG — and draw.io is never started.
//!
//! The SVG exports are the one view not weighed whole. Each is draw.io Desktop's own rendering, and holds
//! no copy of the diagram it was drawn from (the kit exports without `--embed-diagram`), so nothing short
//! of that app can say what one should hold, and no version of it is pinned here: a byte check would fail
//! on two machines that differ only in that app. A hand edit to an SVG still fails through the HTML,
//! which inlines it, until the HTML is rebuilt after it; `plan check --final` reports an export older than
//! its source.

use crate::scan::FoundFile;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

/// The kit's builders, in the order `plan/package.json` names them: the diagrams first, because the HTML
/// the plan's build writes inlines their SVG exports.
const BUILDERS: &[(&str, &[&str])] = &[("build/build-diagrams.mjs", &[]), ("build/build-plan.mjs", &["--no-shots"])];

/// The program the diagram build starts in draw.io Desktop's place: it ignores its arguments and writes
/// nothing, so the drawing in each committed SVG stays as committed and no app starts.
const NO_EXPORTER: &str = "true";

/// Where the diagram build writes, asked of the kit's own configuration, its default filled in, so no
/// second copy of that folder's name is kept here.
const DIAGRAMS_FOLDER: &str = "const kit = await import('./build/lib/common.mjs'); process.stdout.write((await kit.loadConfig()).diagramsOut);";

/// The date each file of the copy's diagrams' folder is given before the rebuild (see the module doc).
const UNWRITTEN: SystemTime = UNIX_EPOCH;

/// What this check's scratches are named by, and the file in each that its check holds locked.
const SCRATCH_TAG: &str = "plan-views";
const HELD: &str = "held";

/// Rebuild the plan's views and compare them with the plan folder, printing what was found: `cargo xtask
/// plan-views --check`, the gate's plan stage.
pub(crate) fn run(root: &Path) -> ExitCode {
    match check(root, &root.join("plan").join("node_modules")) {
        Ok(compared) => {
            println!("plan-views: {compared} files of plan/ compared against a rebuild from their sources: every byte the kit writes matches");
            ExitCode::SUCCESS
        }
        Err(problems) => {
            for problem in problems {
                println!("plan-views: {problem}");
            }
            ExitCode::from(1)
        }
    }
}

/// The check of the plan folder in `root`, rebuilt with the kit's dependencies in `dependencies`: the gate
/// passes this repository and the plan's own `node_modules`, a test a copy of the plan planted with a hand
/// edit and the real plan's dependencies. `Ok(count)` of the files compared when every byte matched; every
/// disagreement when they did not, a check that could not run saying so rather than reporting nothing.
fn check(root: &Path, dependencies: &Path) -> Result<usize, Vec<String>> {
    let plan = root.join("plan");
    if !plan.is_dir() {
        return Err(vec![format!("no plan folder at {}: the plan of record is part of the repository", plan.display())]);
    }
    if let Some((builder, _)) = BUILDERS.iter().find(|(builder, _)| !plan.join(builder).is_file()) {
        return Err(vec![format!("{}: no plan kit: install the kit into plan/build, as plan/build/KIT-INSTALLED.md tells", plan.join(builder).display())]);
    }
    // The scratch is made inside the dependencies folder, so the folder must be there, and is looked for
    // before the scratch would make it, holding no dependency at all. Whether it holds the versions the
    // lock pins is the prerequisite the doctor and the gate's plan stage ask, through one definition
    // (`craftbridge_plan_dependencies` in tools/env.sh), before this check runs.
    if !dependencies.is_dir() {
        return Err(vec![format!("{}: the plan kit's dependencies are not installed: run (cd plan && npm ci)", dependencies.display())]);
    }
    // Removed whatever the rebuild found, so a run that fails leaves nothing behind. `_held` keeps the lock
    // until then: bound to `_`, it would be let go at once, and a check starting beside this one would
    // sweep this scratch away mid-rebuild.
    let (scratch, _held) = claim_scratch(&dependencies.join(".cache")).map_err(|error| vec![error])?;
    let outcome = rebuild_and_compare(root, &scratch);
    match remove_scratch(&scratch) {
        Ok(()) => outcome,
        Err(error) => {
            let mut problems = outcome.err().unwrap_or_default();
            problems.push(format!("could not remove {}: {error}", scratch.display()));
            Err(problems)
        }
    }
}

/// A scratch of this check's own in `cache`, and the lock it is held by, made after removing every scratch
/// an earlier check left there. A check stopped by a signal never removes its own — a Ctrl-C at the
/// terminal reaches it as surely as the gate — and nothing but `npm ci` would. A check holds a lock on
/// `HELD` in its scratch for as long as it runs, which the system lets go however the check ends, so a
/// scratch whose lock can be taken is one no check is using. The cache folder is locked while scratches are
/// swept and made, so no sweep sees one made and not yet held.
fn claim_scratch(cache: &Path) -> Result<(PathBuf, fs::File), String> {
    fs::create_dir_all(cache).map_err(|error| format!("could not create {}: {error}", cache.display()))?;
    let _folder = crate::filelock::hold(cache)?;
    let left = fs::read_dir(cache).map_err(|error| format!("{}: {error}", cache.display()))?;
    for entry in left {
        let entry = entry.map_err(|error| format!("{}: {error}", cache.display()))?;
        if !entry.file_name().to_string_lossy().starts_with(&crate::scratch::prefix(SCRATCH_TAG)) {
            continue;
        }
        let abandoned = match fs::File::open(entry.path().join(HELD)) {
            Ok(opened) => abandoned(&entry.path(), &opened).map_err(|error| format!("{}: {error}", entry.path().join(HELD).display()))?,
            // A scratch with no lock file is one its check is still making or already removing, which it
            // holds all the while; a check stopped in between leaves no more than an empty folder.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(format!("{}: {error}", entry.path().join(HELD).display())),
        };
        if abandoned {
            remove_scratch(&entry.path()).map_err(|error| format!("could not remove {}, which a stopped check left: {error}", entry.path().display()))?;
        }
    }
    let scratch = crate::scratch::create_in(cache, SCRATCH_TAG)?;
    let held = crate::filelock::hold(&scratch.join(HELD))?;
    Ok((scratch, held))
}

/// Whether `scratch`, whose lock file was opened as `opened`, is one no check is using. A check removes the
/// lock file before it lets the lock go, so a lock taken on a file no longer in the scratch was taken from
/// a check that has just finished and removed its scratch, not from one that stopped: removing that scratch
/// again would fail.
fn abandoned(scratch: &Path, opened: &fs::File) -> std::io::Result<bool> {
    Ok(crate::filelock::try_lock(opened)? && scratch.join(HELD).exists())
}

/// Remove a scratch with its lock file last, so that one stopped while it is removed still has its lock
/// file, let go, beside whatever was not removed yet, and the next check finds it.
fn remove_scratch(scratch: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(scratch)? {
        let entry = entry?;
        if entry.file_name() == HELD {
            continue;
        }
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    fs::remove_file(scratch.join(HELD))?;
    fs::remove_dir(scratch)
}

/// Copy the plan twice into `scratch`, rebuild the views in one copy, and compare the two: the difference
/// between them is exactly what the rebuild wrote and how it disagrees, with nothing else in the way —
/// which is why the copies, not the tracked folder, are what is compared, so an untracked note somebody
/// left in plan/ cannot read as a hand-edited view.
fn rebuild_and_compare(root: &Path, scratch: &Path) -> Result<usize, Vec<String>> {
    let (before, after) = (scratch.join("before"), scratch.join("after"));
    let files = plan_files(root).map_err(|error| vec![error])?;
    for copy in [&before, &after] {
        copy_files(&root.join("plan"), &files, copy).map_err(|error| vec![error])?;
    }
    let exporter = no_exporter().map_err(|error| vec![error])?;
    let diagrams = diagrams_folder(&after, &exporter).map_err(|error| vec![error])?;
    let dated = date_files(&after.join(&diagrams)).map_err(|error| vec![error])?;
    for (builder, arguments) in BUILDERS {
        node_in(&after, &[&[*builder][..], arguments].concat(), &exporter).map_err(|error| vec![error])?;
    }
    let mut found = sourceless(&dated, &diagrams).map_err(|error| vec![error])?;
    let differences = crate::generate::tree_differences(&after, &before, "the rebuild", "the plan folder").map_err(|error| vec![error])?;
    found.extend(differences.into_iter().map(|difference| format!("plan/{difference}")));
    if found.is_empty() {
        return Ok(files.len());
    }
    let mut problems = vec!["the plan folder is not what rebuilding its views from their sources writes:".to_string()];
    problems.extend(found);
    problems.push("a plan view is written from its source, never edited: edit plan/EFFECTCRAFT-PLUGIN-PORT-SCOPING.md, or the diagram's source in plan/diagrams/src/, then rebuild with (cd plan && node build/build-diagrams.mjs && node build/build-plan.mjs), the diagrams with draw.io Desktop installed; an export built from no source goes, with its diagram's tag in the master, or its source comes back".to_string());
    Err(problems)
}

/// The diagrams' folder of the plan copy `dir`, as the kit's configuration names it. It must lie inside
/// the plan, since every file in it is dated: a folder named outside would date files the check does not
/// own.
fn diagrams_folder(dir: &Path, exporter: &Path) -> Result<String, String> {
    let folder = String::from_utf8_lossy(&node_in(dir, &["--input-type=module", "--eval", DIAGRAMS_FOLDER], exporter)?).into_owned();
    if folder.is_empty() || !Path::new(&folder).components().all(|component| matches!(component, Component::Normal(_))) {
        return Err(format!("the plan kit names `{folder}` as the folder its diagrams are written to: it must be a folder inside the plan"));
    }
    Ok(folder)
}

/// Every file under `folder`, dated to `UNWRITTEN`.
fn date_files(folder: &Path) -> Result<Vec<FoundFile>, String> {
    let files = crate::scan::walk(folder, "")?;
    for file in &files {
        fs::File::open(&file.path)
            .and_then(|opened| opened.set_modified(UNWRITTEN))
            .map_err(|error| format!("could not date {}: {error}", file.path.display()))?;
    }
    Ok(files)
}

/// The files of the diagrams' folder `diagrams` the rebuild left as `date_files` dated them: built from no
/// source. Finder's `.DS_Store` is no export, as the tree comparison skips it too.
fn sourceless(dated: &[FoundFile], diagrams: &str) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    for file in dated.iter().filter(|file| !file.path.file_name().is_some_and(|name| crate::layout::IGNORED_NAMES.iter().any(|ignored| name == *ignored))) {
        let modified = fs::metadata(&file.path).and_then(|metadata| metadata.modified()).map_err(|error| format!("{}: {error}", file.path.display()))?;
        if modified == UNWRITTEN {
            found.push(format!("plan/{diagrams}/{}: built from no source", file.relative));
        }
    }
    Ok(found)
}

/// The plan's files, as paths inside `plan/`: the repository's files that sit under it.
fn plan_files(root: &Path) -> Result<Vec<String>, String> {
    Ok(crate::hygiene::repository_files(root)?.into_iter().filter_map(|file| file.strip_prefix("plan/").map(str::to_string)).collect())
}

/// Copy each of `files` from the plan folder `from` to the same place under `to`.
fn copy_files(from: &Path, files: &[String], to: &Path) -> Result<(), String> {
    for file in files {
        let target = to.join(file);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        fs::copy(from.join(file), &target).map_err(|error| format!("could not copy plan/{file}: {error}"))?;
    }
    Ok(())
}

/// `true`, found on `PATH` as a shell finds it: the kit takes `DRAWIO_BIN` only as the path of a file that
/// exists, never as a name to look up.
fn no_exporter() -> Result<PathBuf, String> {
    no_exporter_on(&std::env::var_os("PATH").unwrap_or_default())
}

/// `true` as found on the search path `path`, made absolute. The kit looks for the file from the copy it
/// builds in, not from where this check runs, and starts draw.io Desktop when it finds none there, so a
/// relative path — from a relative or an empty entry on `PATH`, which names the folder a program runs
/// in — would start the very app this stands in for.
fn no_exporter_on(path: &std::ffi::OsStr) -> Result<PathBuf, String> {
    std::env::split_paths(path)
        .map(|folder| folder.join(NO_EXPORTER))
        .find(|program| program.is_file())
        .and_then(|program| fs::canonicalize(program).ok())
        .ok_or_else(|| format!("no `{NO_EXPORTER}` on PATH: the diagram build starts it in draw.io Desktop's place, so that no app starts"))
}

/// Node run in the scratch copy `dir` with `arguments`, `exporter` standing in for draw.io: one of the
/// kit's builders, or a question to the kit. What it prints is its answer, or its own report, on a run that
/// worked; on one that did not, its words come back as this check's error, both streams, since the
/// diagram build prints its lint on one and its verdict on the other.
fn node_in(dir: &Path, arguments: &[&str], exporter: &Path) -> Result<Vec<u8>, String> {
    let output = Command::new("node")
        .args(arguments)
        .current_dir(dir)
        .env("DRAWIO_BIN", exporter)
        .output()
        .map_err(|error| format!("could not run node to rebuild the plan's views: {error} (install Node 20 or newer; tools/doctor.sh checks it)"))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let command = [&["node"][..], arguments].concat().join(" ");
    let stopped = output.status.code().map(|code| code.to_string()).unwrap_or_else(|| "a signal".to_string());
    let printed = [String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)].join("\n");
    Err(format!("`{command}` in the scratch copy of plan/ stopped with {stopped}:\n{}", printed.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;
    use std::ffi::OsStr;

    /// The plan's own dependencies, which every rebuild here runs beside.
    fn dependencies() -> PathBuf {
        crate::repo_root().join("plan").join("node_modules")
    }

    /// Every view of this repository's plan is rebuilt and matches it: the check run on the real plan, which
    /// is the state a change must leave. A machine that has not run `(cd plan && npm ci)` cannot rebuild, and
    /// `check` says so; that message is the fix, not a failure of the check.
    #[test]
    fn a_rebuild_from_the_master_matches_the_plan() {
        match check(&crate::repo_root(), &dependencies()) {
            Ok(compared) => assert!(compared > 500, "the rebuild compared {compared} files: too few to be the plan folder, so it checked nothing"),
            Err(problems) => panic!("plan/ is not what a rebuild of its views writes:\n{}", problems.join("\n")),
        }
    }

    /// What `NODE_OPTIONS` hands every Node the child starts: the clock stopped at an instant years from
    /// any build, and every folder listing reversed (`fs.readdirSync`, how the kit lists a folder). A data
    /// URL holds no `#`, `?` or `%`, which would end or escape it, and the value no `"`, which quotes it.
    const ANOTHER_MACHINE: &str = "--import=\"data:text/javascript,import fs from 'node:fs'; import { syncBuiltinESMExports } from 'node:module'; \
        const Clock = Date; const instant = Clock.UTC(2031, 6, 15, 12, 34, 56); \
        globalThis.Date = class extends Clock { constructor(...time) { if (!time.length) time = [instant]; super(...time); } static now() { return instant; } }; \
        const list = fs.readdirSync; fs.readdirSync = (...folder) => list(...folder).reverse(); syncBuiltinESMExports();\"";

    /// Nothing the builders write may come from the machine rather than the plan: the master carries its
    /// date as text somebody wrote, and two machines keep different clocks, zones and languages and list a
    /// folder in different orders. A child runs the whole check with the clock in 2031, the time zone 14
    /// hours ahead, Turkish as the language and every folder listing reversed, where a builder reading any
    /// of them would show itself: the same plan, bytes apart.
    #[test]
    fn the_rebuild_reads_nothing_of_the_machine() {
        crate::scratch::rerun(
            "plan_views::tests::the_plan_rebuilt_on_another_machine",
            &[
                ("TZ", OsStr::new("Pacific/Kiritimati")),
                ("LC_ALL", OsStr::new("tr_TR.UTF-8")),
                ("LANG", OsStr::new("tr_TR.UTF-8")),
                ("NODE_OPTIONS", OsStr::new(ANOTHER_MACHINE)),
            ],
            "rebuilt with another clock, zone, language and listing order, the plan did not match",
        );
    }

    /// The same check as `a_rebuild_from_the_master_matches_the_plan`, run only by the child above: a test
    /// cannot change the environment of a process other tests are running in beside it.
    #[test]
    #[ignore = "run by `the_rebuild_reads_nothing_of_the_machine`, with another clock, zone, language and listing order"]
    fn the_plan_rebuilt_on_another_machine() {
        a_rebuild_from_the_master_matches_the_plan();
    }

    /// The acceptance, on a copy of the real plan: untouched, it passes; then views are edited by hand — a
    /// task list, the HTML the plan is read in, a module view, and a diagram's draw.io source — and each
    /// fails, named. A view whose text is not prose, a contract's JSON, is edited too, so a check that only
    /// looked at Markdown would not pass for it.
    #[test]
    fn a_hand_edit_to_a_view_fails_naming_the_file() {
        let planted = planted_plan("plan-views-hand-edit");
        let clean = check(&planted.0, &dependencies());
        assert!(clean.is_ok(), "an untouched copy of the plan failed its own check: {:?}", clean.unwrap_err());
        let views = [
            "tasks/ops.md",
            "EFFECTCRAFT-PLUGIN-PORT-PLAN.html",
            "modules/repository-and-pins.md",
            "contracts/api-surface.schema.json",
            "diagrams/drawio/d01-journey.drawio",
        ];
        for view in views {
            let path = planted.0.join("plan").join(view);
            let bytes = fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            fs::write(&path, [bytes.as_slice(), b"\nEdited by hand.\n"].concat()).unwrap();
        }
        let problems = check(&planted.0, &dependencies()).expect_err("a hand-edited view must not pass the check");
        for view in views {
            assert!(
                problems.iter().any(|problem| problem.starts_with(&format!("plan/{view}: changed"))),
                "the report does not name plan/{view}:\n{}",
                problems.join("\n")
            );
        }
    }

    /// An SVG export is weighed as far as the kit writes it, and draw.io never starts. Draw.io adds a PNG
    /// copy of every label, which the diagram build cuts out, so an export taken straight from draw.io,
    /// past the kit, fails, named; the HTML inlines the export as the kit trims it, so it is not named for
    /// that one. A hand edit to the drawing of another export fails through the HTML alone: had draw.io
    /// run, it would have drawn that export afresh, and the export would be named too.
    #[test]
    fn an_export_is_weighed_as_far_as_the_kit_writes_it() {
        let planted = planted_plan("plan-views-export");
        let edit = |export: &str, from: &str, to: &str| {
            let path = planted.0.join("plan/diagrams/drawio").join(export);
            let svg = fs::read_to_string(&path).unwrap();
            let edited = svg.replacen(from, to, 1);
            assert_ne!(edited, svg, "{export} holds no {from:?} to plant beside");
            fs::write(&path, edited).unwrap();
        };
        let fallback = r#"<image x="0" y="0" width="1" height="1" xlink:href="data:image/png;base64,iVBORw0KGgo="/>"#;
        edit("d01-journey.svg", "</switch>", &format!("{fallback}</switch>"));
        edit("d02-architecture.svg", "Text is not SVG - cannot display", "Edited by hand");
        let problems = check(&planted.0, &dependencies()).expect_err("an untrimmed or hand-edited export must not pass the check");
        let named: Vec<&String> = problems.iter().filter(|problem| problem.starts_with("plan/")).collect();
        assert_eq!(named, ["plan/EFFECTCRAFT-PLUGIN-PORT-PLAN.html: changed", "plan/diagrams/drawio/d01-journey.svg: changed"], "{}", problems.join("\n"));
    }

    /// A diagram whose source is gone leaves exports no build writes again, which a hand could then edit
    /// unseen while the HTML, still tagged for the diagram, goes on inlining its drawing: both exports
    /// fail, named, and nothing else does, since every byte the rebuild writes still matches — not even
    /// the `.DS_Store` Finder leaves beside them, which the copy holds here, outside a checkout.
    #[test]
    fn a_diagram_whose_source_is_gone_fails_naming_its_exports() {
        let planted = planted_plan("plan-views-sourceless");
        fs::remove_file(planted.0.join("plan/diagrams/src/d01-journey.mjs")).unwrap();
        planted.write("plan/diagrams/drawio/.DS_Store", [0u8, 1]);
        let problems = check(&planted.0, &dependencies()).expect_err("the exports of a removed source must not pass the check");
        let named: Vec<&String> = problems.iter().filter(|problem| problem.starts_with("plan/")).collect();
        assert_eq!(
            named,
            ["plan/diagrams/drawio/d01-journey.drawio: built from no source", "plan/diagrams/drawio/d01-journey.svg: built from no source"],
            "{}",
            problems.join("\n")
        );
    }

    /// The check dates every file in the diagrams' folder, so a configuration naming one outside the plan
    /// is refused before anything is dated there.
    #[test]
    fn a_diagrams_folder_outside_the_plan_is_refused() {
        let planted = planted_plan("plan-views-folder-outside");
        let config = planted.0.join("plan/plan.config.mjs");
        let text = fs::read_to_string(&config).unwrap();
        let moved = text.replacen("generators: {},", "generators: {},\n  diagramsOut: '../elsewhere',", 1);
        assert_ne!(moved, text, "the configuration holds no `generators` entry to plant beside");
        fs::write(&config, moved).unwrap();
        let problems = check(&planted.0, &dependencies()).expect_err("a diagrams' folder outside the plan must not pass the check");
        assert_eq!(problems.len(), 1, "{}", problems.join("\n"));
        assert!(problems[0].contains("`../elsewhere`") && problems[0].contains("must be a folder inside the plan"), "{problems:?}");
    }

    /// Where the child below writes the path of the scratch it leaves behind.
    const LEFT_AT: &str = "CRAFTBRIDGE_PLAN_VIEWS_LEFT_AT";

    /// A check that stops before removing its scratch leaves it behind, its lock let go when its process
    /// ends, and the next check removes it. It leaves alone the scratch of a check still running beside it,
    /// which holds its own, and one with no lock file, which a check is still making or already removing.
    /// The stopped check is a process of its own, as it is at the terminal: a lock this process let go of
    /// could still be held for an instant through a child another test is starting, and a check that has
    /// ended holds nothing. Each scratch made here is removed as a check removes its own, before its lock
    /// is let go, so no sweep beside this test removes it at the same time.
    #[test]
    fn a_scratch_a_stopped_check_left_goes_and_a_held_one_stays() {
        let cache = dependencies().join(".cache");
        let (running, held) = claim_scratch(&cache).unwrap();
        let note = Scratch::new("plan-views-left-at");
        crate::scratch::rerun(
            "plan_views::tests::a_check_stops_before_removing_its_scratch",
            &[(LEFT_AT, note.0.join("path").as_os_str())],
            "the stopping check did not run",
        );
        let stopped = PathBuf::from(fs::read_to_string(note.0.join("path")).unwrap());
        let unheld = crate::scratch::create_in(&cache, SCRATCH_TAG).unwrap();
        let (next, own) = claim_scratch(&cache).unwrap();
        assert!(!stopped.exists(), "the scratch of a stopped check is still there: {stopped:?}");
        assert!(running.exists(), "the scratch of a running check was removed: {running:?}");
        assert!(unheld.exists(), "a scratch with no lock file, which its check may be making or removing, was removed: {unheld:?}");
        fs::remove_dir(&unheld).unwrap();
        for (scratch, lock) in [(running, held), (next, own)] {
            remove_scratch(&scratch).unwrap();
            drop(lock);
        }
    }

    /// A check that finishes between a sweep opening its lock file and taking the lock has removed its
    /// scratch and let the lock go: the sweep must not take it for one a stopped check left.
    #[test]
    fn a_check_that_finishes_while_a_sweep_looks_is_not_a_stopped_one() {
        let (scratch, held) = claim_scratch(&dependencies().join(".cache")).unwrap();
        let opened = fs::File::open(scratch.join(HELD)).unwrap();
        assert!(!abandoned(&scratch, &opened).unwrap(), "the scratch of a running check was taken for a stopped one");
        remove_scratch(&scratch).unwrap();
        drop(held);
        assert!(!abandoned(&scratch, &opened).unwrap(), "the scratch of a check that finished was taken for a stopped one");
    }

    /// A check that stops between claiming its scratch and removing it, run only by the test above: it
    /// leaves a rebuild's worth of files and writes where it left them.
    #[test]
    #[ignore = "run by `a_scratch_a_stopped_check_left_goes_and_a_held_one_stays`, as a check that stops before removing its scratch"]
    fn a_check_stops_before_removing_its_scratch() {
        let (scratch, _held) = claim_scratch(&dependencies().join(".cache")).unwrap();
        fs::create_dir_all(scratch.join("after/tasks")).unwrap();
        fs::write(scratch.join("after/tasks/ops.md"), "left behind").unwrap();
        fs::write(std::env::var_os(LEFT_AT).unwrap(), scratch.to_string_lossy().as_bytes()).unwrap();
    }

    /// The stand-in for draw.io's exporter reaches the kit as an absolute path, whatever form the `PATH`
    /// entry it was found under takes. Here the entry is relative: from this test's folder it names the
    /// folder `true` is in, and from the copy the kit builds in, nothing, so the kit would start draw.io.
    #[test]
    fn the_stand_in_exporter_is_named_by_an_absolute_path() {
        let path = std::env::var_os("PATH").unwrap();
        let folder = std::env::split_paths(&path).find(|folder| folder.is_absolute() && folder.join(NO_EXPORTER).is_file()).unwrap();
        let depth = std::env::current_dir().unwrap().components().count();
        let relative = PathBuf::from("../".repeat(depth)).join(folder.strip_prefix("/").unwrap());
        assert!(relative.is_relative() && relative.join(NO_EXPORTER).is_file(), "{relative:?} does not reach `{NO_EXPORTER}`");
        let exporter = no_exporter_on(relative.as_os_str()).unwrap();
        assert!(exporter.is_absolute(), "the kit would be handed {exporter:?}, which it looks for from another folder");
        assert_eq!(exporter, fs::canonicalize(folder.join(NO_EXPORTER)).unwrap());
    }

    /// The plan of record is part of the repository, so a root without one is a broken tree, not a plan
    /// whose views happen to match nothing: it fails with the folder it looked in.
    #[test]
    fn a_root_with_no_plan_folder_says_so() {
        let scratch = Scratch::new("plan-views-no-plan");
        let problems = check(&scratch.0, &dependencies()).expect_err("a root with no plan folder must not pass the check");
        assert_eq!(problems, vec![format!("no plan folder at {}: the plan of record is part of the repository", scratch.0.join("plan").display())]);
    }

    /// A plan folder with no kit in it is reported as missing the kit, with where the kit comes from, rather
    /// than as a plan full of hand-edited views: a check that could not run says what is missing, and the
    /// machine's own doctor says it too before this stage is ever reached.
    #[test]
    fn a_plan_with_no_kit_says_the_kit_is_missing() {
        let scratch = Scratch::new("plan-views-no-kit");
        scratch.mkdir("plan");
        let missing = scratch.0.join("plan").join(BUILDERS[0].0);
        let problems = check(&scratch.0, &dependencies()).expect_err("a plan with no kit in it must not pass the check");
        assert_eq!(problems.len(), 1, "the report is not the one thing missing:\n{}", problems.join("\n"));
        assert!(problems[0].starts_with(&missing.display().to_string()), "the report does not name {missing:?}:\n{problems:?}");
        assert!(problems[0].contains("install the kit into plan/build"), "{problems:?}");
    }

    /// The kit is there and its dependencies are not — a fresh clone before anyone ran `npm ci`. The check
    /// names that command rather than letting a builder fail on a missing import, and makes no folder where
    /// the dependencies belong, which would read as installed to the next look.
    #[test]
    fn a_plan_with_no_dependencies_names_the_command_that_fixes_it() {
        let scratch = Scratch::new("plan-views-no-dependencies");
        for (builder, _) in BUILDERS {
            scratch.write(&format!("plan/{builder}"), "");
        }
        let missing = scratch.0.join("plan/node_modules");
        let problems = check(&scratch.0, &missing).expect_err("a plan whose dependencies are missing must not pass the check");
        assert_eq!(problems.len(), 1, "the report is not the one thing missing:\n{}", problems.join("\n"));
        assert!(problems[0].contains("the plan kit's dependencies are not installed"), "{problems:?}");
        assert!(problems[0].contains("(cd plan && npm ci)"), "the report does not name the command that fixes it:\n{problems:?}");
        assert!(!missing.exists(), "the check made {missing:?}, where the dependencies belong");
    }

    /// A copy of the real plan, made of the files the check copies, in a scratch standing in for the
    /// repository root, so a view can be edited by hand without touching the plan of record. The real
    /// dependencies rebuild it, since the check's own scratch stands beside them whichever plan it checks.
    fn planted_plan(tag: &str) -> Scratch {
        let root = crate::repo_root();
        let scratch = Scratch::new(tag);
        copy_files(&root.join("plan"), &plan_files(&root).unwrap(), &scratch.0.join("plan")).unwrap();
        scratch
    }
}
