//! Rule 7 (upstream) — EffectCraft's crates at the official pin, and what must read exactly as the
//! update script wrote it (AGENTS.md §7, plan §15 "Pins", §24, D-21, PD4, PD17).
//!
//! CraftBridge builds EffectCraft as it is: the workspace's EffectCraft and FilmCraft dependencies
//! point at one official revision each, and the `[patch]` tables, the release profile and the starting
//! `Cargo.lock` come from that revision's own manifest and lock — through this module, which
//! `tools/update-upstream.sh` runs, so they are never written by hand. This check then asks Cargo's own
//! questions, and nothing else:
//!
//! * the EffectCraft pin is an official revision: a commit, named by its id, that official main
//!   reaches, and the FilmCraft pin is the revision that commit itself names,
//! * `[patch]` and `[profile.release]` in `rust/Cargo.toml` hold exactly the entries the generator would
//!   write from the pinned upstream,
//! * every EffectCraft, FilmCraft or Craft library package in `rust/Cargo.lock` resolves to a source the
//!   pin sanctions, compared whole, down to the commit Cargo builds — a registry copy, a fork, another
//!   branch or commit, a local path, or a second copy of a library crate upstream's own lock holds
//!   once, is named, because any of them could build a changed EffectCraft the conformance suite never
//!   checked,
//! * no Cargo configuration Cargo reads for a build inside `rust/` carries a `[patch]`, the old `paths`
//!   key or a `[source]` replacement: the repository's own, those in every folder above the checkout,
//!   the cargo home's, whatever those `include`, and the environment the gate runs in — each is
//!   Cargo's word on where the build takes its crates, exactly like the lock is its word on commits, and
//! * the workspace still depends on the pinned crates: a pin nothing imports would let the check
//!   verify nothing.
//!
//! Everything here reads Cargo's own data — manifests and the lock as TOML, never a scanned line of
//! text — and git's, through [`crate::git`]. The generator keeps the rest of `rust/Cargo.toml` byte for
//! byte, which is what lets `tools/update-upstream.sh` run twice on the same revision and change nothing.

use crate::layers;
use crate::layers::{CraftFamily, family};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The EffectCraft crates CraftBridge depends on (plan §15 "Pins"): each gets a git pin in
/// `[workspace.dependencies]`, and the lock must resolve every one of them at the pinned revision, so
/// the workspace builds against the pin rather than around it.
pub(crate) const EFFECTCRAFT_CRATES: &[&str] = &[
    "effectcraft-engine",
    "effectcraft-host",
    "effectcraft-automation",
    "effectcraft-gpu",
    "effectcraft-render",
    "effectcraft-time",
    "effectcraft-text",
    "effectcraft-media",
    "effectcraft-expr",
    "effectcraft-script",
    "effectcraft-plugin",
];

fn effectcraft_app() -> Result<&'static layers::CraftApp, String> {
    layers::app_by_noun("effects").ok_or_else(|| "the layer table names no effects app to pin against".to_string())
}

fn film_app() -> Result<&'static layers::CraftApp, String> {
    layers::app_by_noun("film").ok_or_else(|| "the layer table names no film app to pin against".to_string())
}

/// The https URL of an app's official repository.
fn app_url(app: &layers::CraftApp) -> String {
    format!("https://{}", app.repository)
}

/// The shared Craft libraries' official repository URL.
fn library_url() -> String {
    format!("https://{}", layers::CRAFT_LIBRARY_REPOSITORY)
}

/// The Cargo source prefix every package of the official Craft library repository carries: one place
/// spells it, the lock check and the duplicate check read the same form.
pub(crate) fn library_source_prefix() -> String {
    format!("git+{}?rev=", library_url())
}

/// The Craft-app crates the layer table lets a core crate use: what CraftBridge's own code names
/// directly, so these are the FilmCraft pins CraftBridge declares for itself rather than inherits.
pub(crate) fn core_named_upstream() -> Vec<&'static str> {
    let mut names: Vec<&'static str> =
        layers::ENTRIES.iter().filter(|entry| matches!(entry.kind, layers::Kind::Core)).flat_map(|entry| entry.upstream).copied().collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// The two revisions the workspace pins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pin {
    pub(crate) effectcraft_rev: String,
    pub(crate) filmcraft_rev: String,
}

/// Read the pins from the workspace manifest text: every EffectCraft crate shares one `rev`, and so
/// does every FilmCraft crate; anything else is not a pin and is reported as such.
pub(crate) fn pins_from_manifest(text: &str) -> Result<Pin, Vec<String>> {
    let document: toml::Value = toml::from_str(text).map_err(|error| vec![format!("rust/Cargo.toml does not parse: {error}")])?;
    let dependencies = document.get("workspace").and_then(|workspace| workspace.get("dependencies")).and_then(toml::Value::as_table);
    let effects_url = app_url(effectcraft_app().map_err(|error| vec![error])?);
    let film_url = app_url(film_app().map_err(|error| vec![error])?);
    let mut errors = Vec::new();
    let read = |names: &[&str], url: &str, what: &str, errors: &mut Vec<String>| -> Option<String> {
        let mut rev: Option<String> = None;
        for name in names {
            let entry = dependencies.and_then(|table| table.get(*name)).and_then(toml::Value::as_table);
            match entry {
                None => errors.push(format!("rust/Cargo.toml: [workspace.dependencies] has no {name}: run tools/update-upstream.sh to write the {what} pin")),
                Some(entry) => {
                    if entry.get("git").and_then(toml::Value::as_str) != Some(url) {
                        errors.push(format!("rust/Cargo.toml: [workspace.dependencies].{name} must come from {url} — the official {what} repository, never a fork, a registry copy or a path"));
                    }
                    match entry.get("rev").and_then(toml::Value::as_str) {
                        None => {
                            errors.push(format!("rust/Cargo.toml: [workspace.dependencies].{name} has no rev: a branch or a tag moves, and the pin must not"))
                        }
                        Some(found) => {
                            if rev.as_deref().is_some_and(|taken| taken != found) {
                                errors
                                    .push(format!("rust/Cargo.toml: [workspace.dependencies] pins {what} at two different revisions: one revision is the pin"));
                            } else {
                                rev = Some(found.to_string());
                            }
                        }
                    }
                }
            }
        }
        rev
    };
    let effectcraft_rev = read(EFFECTCRAFT_CRATES, &effects_url, "EffectCraft", &mut errors);
    let filmcraft_rev = read(&core_named_upstream(), &film_url, "FilmCraft", &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    match (effectcraft_rev, filmcraft_rev) {
        (Some(effectcraft_rev), Some(filmcraft_rev)) => Ok(Pin { effectcraft_rev, filmcraft_rev }),
        _ => Err(vec!["rust/Cargo.toml: [workspace.dependencies] has no pin: run tools/update-upstream.sh".to_string()]),
    }
}

/// The revisions of `url` the manifest text pins some dependency or patch at: how the pinned EffectCraft's
/// own manifest names the FilmCraft revision CraftBridge must pin. Which sources a lock package may show
/// is upstream's lock's to say ([`upstream_sources`]), not its manifest's.
pub(crate) fn upstream_git_revs(manifest_text: &str, url: &str) -> BTreeSet<String> {
    let Ok(document) = toml::from_str::<toml::Value>(manifest_text) else { return BTreeSet::new() };
    let mut revs = BTreeSet::new();
    let mut stack = vec![&document];
    while let Some(value) = stack.pop() {
        match value {
            toml::Value::Table(table) => {
                if table.get("git").and_then(toml::Value::as_str) == Some(url)
                    && let Some(rev) = table.get("rev").and_then(toml::Value::as_str)
                {
                    revs.insert(rev.to_string());
                }
                stack.extend(table.values());
            }
            toml::Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    revs
}

/// The one FilmCraft revision the pinned EffectCraft manifest names: CraftBridge pins FilmCraft at
/// EffectCraft's own revision (plan §24), never at one of its own choosing.
pub(crate) fn upstream_filmcraft_rev(manifest_text: &str) -> Result<String, String> {
    let revs = upstream_git_revs(manifest_text, &app_url(film_app()?));
    match revs.len() {
        1 => revs.into_iter().next().ok_or_else(|| "the pinned FilmCraft revision vanished".to_string()),
        0 => Err("the pinned EffectCraft names no FilmCraft revision".to_string()),
        many => Err(format!("the pinned EffectCraft names {many} FilmCraft revisions: one revision is the pin")),
    }
}

/// The line naming who wrote a generated table. Identical between runs, so a second run of the
/// generator on the same revision leaves the file byte for byte as it is.
const GENERATED_BY: &str = "# Written by tools/update-upstream.sh from the pinned EffectCraft; never edit by hand (AGENTS.md \u{a7}6, \u{a7}7).\n";

/// Point one `[workspace.dependencies]` entry at `url` at `rev`, leaving any other key (its features,
/// say) in place: those are CraftBridge's choice. Cargo itself refuses a `path`, a `branch` or a `tag`
/// beside the pin's `git` and `rev`, and whatever source an entry does resolve to, the lock check holds
/// it to the pin. A newly written group of pins starts on a line of its own, so the hand-written entries
/// above it and the generated ones below read apart.
fn set_pin(deps: &mut toml_edit::Table, name: &str, url: &str, rev: &str, blank_before: bool) {
    let (existing, lead) = match deps.get(name).and_then(toml_edit::Item::as_value).and_then(toml_edit::Value::as_inline_table) {
        Some(existing) => (existing.clone(), String::new()),
        None => (toml_edit::InlineTable::new(), if blank_before { "\n" } else { "" }.to_string()),
    };
    let mut inline = existing;
    inline.insert("git", toml_edit::Value::from(url.to_string()));
    inline.insert("rev", toml_edit::Value::from(rev.to_string()));
    inline.set_dotted(false);
    deps.insert(name, toml_edit::Item::Value(toml_edit::Value::InlineTable(inline)));
    if !lead.is_empty()
        && let Some(mut key) = deps.key_mut(name)
    {
        key.leaf_decor_mut().set_prefix(lead);
    }
}

/// Move one top-level table out of the upstream document into ours, replacing whatever was there;
/// when upstream holds none, ours goes too. The first explicit table inside carries the written-by line.
fn transplant(ours: &mut toml_edit::DocumentMut, upstream: &toml_edit::DocumentMut, key: &str) {
    let moved = upstream.get(key).cloned();
    match moved {
        Some(mut item) => {
            if let Some(table) = item.as_table_like_mut() {
                let first = table.iter().next().map(|(child, _)| child.to_string());
                if let Some(name) = first
                    && let Some(child) = table.get_mut(name.as_str()).and_then(toml_edit::Item::as_table_mut)
                {
                    child.decor_mut().set_prefix(format!("\n{GENERATED_BY}"));
                }
            }
            ours[key] = item;
        }
        None => {
            ours.as_table_mut().remove(key);
        }
    }
}

/// `[profile.release]` moves whole: it lives under `[profile]`, whose other tables (a future
/// `[profile.dev]`, say) stay ours. The `[profile]` parent is created implicit when ours has none, so
/// only `[profile.release]` prints its header, exactly as upstream writes it.
fn transplant_release(ours: &mut toml_edit::DocumentMut, upstream: &toml_edit::DocumentMut) {
    let release = upstream.get("profile").and_then(toml_edit::Item::as_table_like).and_then(|profile| profile.get("release")).cloned();
    match release {
        Some(mut release) => {
            if let Some(table) = release.as_table_mut() {
                table.decor_mut().set_prefix(format!("\n{GENERATED_BY}"));
            }
            if !ours.get("profile").is_some_and(toml_edit::Item::is_table) {
                let mut profile = toml_edit::Table::new();
                profile.set_implicit(true);
                ours["profile"] = toml_edit::Item::Table(profile);
            }
            if let Some(profile) = ours.get_mut("profile").and_then(toml_edit::Item::as_table_mut) {
                profile["release"] = release;
            }
        }
        None => {
            if let Some(profile) = ours.get_mut("profile").and_then(toml_edit::Item::as_table_like_mut) {
                profile.remove("release");
                if profile.is_empty() {
                    ours.as_table_mut().remove("profile");
                }
            }
        }
    }
}

/// Write what `tools/update-upstream.sh` writes for one revision into the manifest text: the pins, and
/// the `[patch]` and `[profile.release]` tables copied from the upstream manifest. Everything else in
/// the file is kept byte for byte.
pub(crate) fn regenerate(ours: &str, upstream_manifest: &str, effectcraft_rev: &str, filmcraft_rev: &str) -> Result<String, String> {
    let mut document = ours.parse::<toml_edit::DocumentMut>().map_err(|error| format!("rust/Cargo.toml does not parse: {error}"))?;
    let upstream = upstream_manifest.parse::<toml_edit::DocumentMut>().map_err(|error| format!("the upstream Cargo.toml does not parse: {error}"))?;
    let effects_url = app_url(effectcraft_app()?);
    let film_url = app_url(film_app()?);
    let dependencies = document
        .as_table_mut()
        .get_mut("workspace")
        .and_then(toml_edit::Item::as_table_mut)
        .and_then(|workspace| workspace.get_mut("dependencies"))
        .and_then(toml_edit::Item::as_table_mut)
        .ok_or("rust/Cargo.toml: [workspace.dependencies] is missing or is not a table")?;
    for (at, name) in EFFECTCRAFT_CRATES.iter().enumerate() {
        set_pin(dependencies, name, &effects_url, effectcraft_rev, at == 0);
    }
    for (at, name) in core_named_upstream().iter().enumerate() {
        set_pin(dependencies, name, &film_url, filmcraft_rev, at == 0);
    }
    transplant(&mut document, &upstream, "patch");
    transplant_release(&mut document, &upstream);
    Ok(document.to_string())
}

/// Flatten one generated table into named entries: `[patch]` becomes one entry per source-and-crate
/// (`crates-io.egui`), `[profile.release]` one per key (`lto`). A whole missing table shows up as its
/// entries all missing, so every finding names the entry a reader can act on.
fn flatten_entries(table: Option<&toml::Value>) -> BTreeMap<String, toml::Value> {
    let mut entries = BTreeMap::new();
    let Some(table) = table.and_then(toml::Value::as_table) else { return entries };
    for (key, child) in table {
        match child.as_table() {
            Some(inner) => {
                for (name, entry) in inner {
                    entries.insert(format!("{key}.{name}"), entry.clone());
                }
            }
            _ => {
                entries.insert(key.clone(), child.clone());
            }
        }
    }
    entries
}

/// Do the generated tables of ours hold exactly what the generator writes? Compared as data, entry by
/// entry, so the entries themselves are checked and only the entries: an added, dropped or changed
/// patch line, or a profile key out of step with upstream, is a finding named by entry; whitespace is not.
pub(crate) fn generated_table_errors(ours: &str, regenerated: &str) -> Vec<String> {
    let (Ok(ours), Ok(written)) = (toml::from_str::<toml::Value>(ours), toml::from_str::<toml::Value>(regenerated)) else {
        return vec!["rust/Cargo.toml does not parse".to_string()];
    };
    let read = |value: &toml::Value, table: &str, sub: &str| -> Option<toml::Value> {
        let top = value.get(table)?;
        Some(if sub.is_empty() { top.clone() } else { top.get(sub)?.clone() })
    };
    let list = |names: &[String]| -> String { names.join(", ") };
    let mut errors = Vec::new();
    for (table, sub, name) in [("patch", "", "[patch]"), ("profile", "release", "[profile.release]")] {
        let mine = flatten_entries(read(&ours, table, sub).as_ref());
        let theirs = flatten_entries(read(&written, table, sub).as_ref());
        let extra: Vec<String> = mine.keys().filter(|entry| !theirs.contains_key(entry.as_str())).cloned().collect();
        let missing: Vec<String> = theirs.keys().filter(|entry| !mine.contains_key(entry.as_str())).cloned().collect();
        let changed: Vec<String> = mine.keys().filter(|entry| theirs.get(entry.as_str()).is_some_and(|value| value != &mine[*entry])).cloned().collect();
        if extra.is_empty() && missing.is_empty() && changed.is_empty() {
            continue;
        }
        let mut clauses = Vec::new();
        if !extra.is_empty() {
            clauses.push(format!("it adds {} that the script does not write", list(&extra)));
        }
        if !missing.is_empty() {
            clauses.push(format!("it lacks {}", list(&missing)));
        }
        if !changed.is_empty() {
            clauses.push(format!("it spells {} differently from upstream", list(&changed)));
        }
        errors.push(format!(
            "rust/Cargo.toml: {name} is not what tools/update-upstream.sh writes for the pinned upstream: {}; run tools/update-upstream.sh",
            clauses.join("; ")
        ));
    }
    errors
}

/// One `[[package]]` of `rust/Cargo.lock`: name and, when it has one, source.
struct Locked {
    name: String,
    source: Option<String>,
}

/// The packages of one lock file, `what` naming it in a message.
fn lock_packages(lock: &str, what: &str) -> Result<Vec<Locked>, String> {
    let document: toml::Value = toml::from_str(lock).map_err(|error| format!("{what} does not parse: {error}"))?;
    let packages = document.get("package").and_then(toml::Value::as_array).ok_or_else(|| format!("{what} has no [[package]] entries"))?;
    packages
        .iter()
        .map(|package| {
            let name = package.get("name").and_then(toml::Value::as_str).ok_or_else(|| format!("a {what} package has no name"))?;
            Ok(Locked { name: name.to_string(), source: package.get("source").and_then(toml::Value::as_str).map(str::to_string) })
        })
        .collect()
}

/// The source string a git dependency on `url` at `rev` names before the `#`: exactly this way, Cargo
/// spells it, with the resolved commit as the fragment after.
fn pinned_source(url: &str, rev: &str) -> String {
    format!("git+{url}?rev={rev}")
}

/// The sources the pinned upstream's own lock resolves each crate name to, whole: what the lock check
/// holds every FilmCraft and Craft library package to. Upstream's lock is the authority on which
/// revisions of those repositories the pin builds — FilmCraft names a `craft-media` revision of its own,
/// and only the lock says so — and on which commit each revision is.
fn upstream_sources(upstream_lock: &str) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let mut sources: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for package in lock_packages(upstream_lock, "the pinned EffectCraft's Cargo.lock")? {
        if let Some(source) = package.source {
            sources.entry(package.name).or_default().insert(source);
        }
    }
    Ok(sources)
}

/// What a lock message says a package resolves to.
fn shown_source(source: Option<&str>) -> String {
    source.map_or_else(|| "a local path".to_string(), |source| format!("its source {source}"))
}

/// Check every Craft-family package of `rust/Cargo.lock` against the pin. Sources are compared whole,
/// so the commit after the `#` is checked as well as the revision before it: Cargo builds the commit the
/// lock records, whatever the `rev` says. An EffectCraft crate comes from the pinned official revision
/// at `effectcraft_commit`. A FilmCraft or Craft library crate comes from a source `upstream` (the pinned
/// upstream's own lock, [`upstream_sources`]) resolves that crate to, or — for a crate upstream does not
/// use — once, from a source upstream resolves for the crate's repository: so the only second copy of a
/// library crate the gate passes is one upstream's own lock carries, which is what the duplicate rule's
/// exemption relies on. Every pinned crate must be in the lock at all, or nothing builds against the pin.
pub(crate) fn lock_pin_errors(lock: &str, pin: &Pin, effectcraft_commit: &str, upstream: &BTreeMap<String, BTreeSet<String>>) -> Result<Vec<String>, String> {
    let packages = lock_packages(lock, "rust/Cargo.lock")?;
    let effects = effectcraft_app()?;
    let film = film_app()?;
    let effectcraft_source = format!("{}#{effectcraft_commit}", pinned_source(&app_url(effects), &pin.effectcraft_rev));
    let mut copies: BTreeMap<&str, usize> = BTreeMap::new();
    for package in &packages {
        *copies.entry(package.name.as_str()).or_default() += 1;
    }
    let mut errors = Vec::new();
    for package in &packages {
        let source = package.source.as_deref();
        let repository = match family(&package.name, source) {
            None => continue,
            Some(CraftFamily::App(app)) if app.noun == effects.noun => {
                if source != Some(effectcraft_source.as_str()) {
                    errors.push(format!(
                        "rust/Cargo.lock: {} must resolve to the pinned official revision {effectcraft_source}; it resolves to {}",
                        package.name,
                        shown_source(source)
                    ));
                }
                continue;
            }
            Some(CraftFamily::App(app)) if app.noun == film.noun => film.repository,
            Some(CraftFamily::Library) => layers::CRAFT_LIBRARY_REPOSITORY,
            Some(CraftFamily::App(app)) => {
                errors.push(format!(
                    "rust/Cargo.lock: {} comes from {}, which nothing has pinned yet: only the official EffectCraft and FilmCraft revisions may be built (AGENTS.md \u{a7}7)",
                    package.name,
                    app_url(app)
                ));
                continue;
            }
        };
        let (allowed, wanted): (BTreeSet<&str>, String) = match upstream.get(&package.name) {
            Some(own) => (own.iter().map(String::as_str).collect(), "a source the pinned EffectCraft's own lock resolves it to".to_string()),
            None if copies.get(package.name.as_str()) == Some(&1) => (
                upstream.values().flatten().map(String::as_str).filter(|candidate| layers::from_repository(candidate, repository)).collect(),
                format!("a https://{repository} source the pinned EffectCraft's own lock resolves"),
            ),
            None => (BTreeSet::new(), "one copy: the pinned EffectCraft's own lock carries no split of it".to_string()),
        };
        if !source.is_some_and(|source| allowed.contains(source)) {
            let listed: Vec<&str> = allowed.into_iter().collect();
            let among = if listed.is_empty() { String::new() } else { format!(" ({})", listed.join(", ")) };
            errors.push(format!("rust/Cargo.lock: {} must resolve to {wanted}{among}; it resolves to {}", package.name, shown_source(source)));
        }
    }
    let names: BTreeSet<String> = packages.iter().map(|package| package.name.clone()).collect();
    for name in EFFECTCRAFT_CRATES.iter().chain(core_named_upstream().iter()) {
        if !names.contains(*name) {
            errors.push(format!("rust/Cargo.lock has no {name}: nothing in the workspace depends on the pinned upstream (plan \u{a7}15 \"Pins\")"));
        }
    }
    Ok(errors)
}

/// The three carriers a Cargo configuration could use to build a copy of a pinned crate around the
/// pin: a `[patch]` table, the old `paths` key and a `[source]` replacement. None belongs in a
/// configuration: patches enter only through the generated tables of `rust/Cargo.toml`, which
/// `tools/update-upstream.sh` writes, and every configuration Cargo reads is refused the same way,
/// wherever it sits.
fn configuration_carriers(document: &toml::Value, name: &str) -> Vec<String> {
    let mut errors = Vec::new();
    // Cargo reads a `[patch]` table and the old `paths` key from its configuration as well as from a
    // manifest, and either would stand between the build and the pinned EffectCraft.
    if document.get("patch").is_some() {
        errors.push(format!(
            "{name}: [patch] redirects a crate around the pinned upstream: the generated tables of rust/Cargo.toml are the one place patches live"
        ));
    }
    if document.get("paths").is_some() {
        errors.push(format!("{name}: paths overrides a crate with a local folder, which would stand between the build and the pinned upstream"));
    }
    // A `[source]` replacement (what `cargo vendor` asks for) builds a directory's or another
    // registry's copy of every crate of a source while Cargo.lock still names the official one, so
    // neither the lock check nor cargo-deny's sources check could see a changed EffectCraft behind it.
    if document.get("source").is_some() {
        errors.push(format!(
            "{name}: [source] replaces where Cargo takes crates from while the lock still names the official source, so a changed copy of the pinned upstream would build unseen"
        ));
    }
    errors
}

/// The variables of `environment` that Cargo reads as one of the three carriers: `CARGO_SOURCE_…`,
/// `CARGO_PATCH_…` and `CARGO_PATHS` are Cargo's own spelling of `source.…`, `patch.…` and `paths`.
/// The pinned stable Cargo builds a whole table out of the environment once nightly features are
/// unlocked (`RUSTC_BOOTSTRAP=1` with `CARGO_UNSTABLE_ADVANCED_ENV`), and then a `[source]` replacement
/// that sits in no file builds a changed copy behind a lock that still names the official source. The
/// gate hands one environment to every stage, so each such variable is refused, unlocked or not.
fn environment_carriers(environment: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>) -> Vec<String> {
    let mut names: Vec<String> = environment.into_iter().filter_map(|(name, _)| name.into_string().ok()).collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            let carrier = if name.starts_with("CARGO_SOURCE_") {
                "[source]"
            } else if name.starts_with("CARGO_PATCH_") {
                "[patch]"
            } else if name == "CARGO_PATHS" {
                "paths"
            } else {
                return None;
            };
            Some(format!(
                "the environment's {name}: Cargo reads it as {carrier} configuration, which would build another copy around the pinned upstream: unset it"
            ))
        })
        .collect()
}

/// The Cargo configuration of one folder whose `.cargo` Cargo probes: `.cargo/config` when it is
/// there, because Cargo reads it instead of `.cargo/config.toml`, and `.cargo/config.toml` otherwise.
fn configuration_of(folder: &Path) -> Option<PathBuf> {
    let legacy = folder.join(".cargo/config");
    if legacy.is_file() {
        return Some(legacy);
    }
    let modern = folder.join(".cargo/config.toml");
    modern.is_file().then_some(modern)
}

/// The configuration inside a cargo home: `config` when it is there, as beside any other folder
/// the legacy name wins, and `config.toml` otherwise — the two files Cargo reads in the home
/// itself, where its caches live, not under a `.cargo` folder again.
fn configuration_in_home(home: &Path) -> Option<PathBuf> {
    let legacy = home.join("config");
    if legacy.is_file() {
        return Some(legacy);
    }
    let modern = home.join("config.toml");
    modern.is_file().then_some(modern)
}

/// The cargo home Cargo reads for a build it runs in `cwd`: `cargo_home` (the value of `$CARGO_HOME`)
/// when it names one, resolved against `cwd` as Cargo resolves a relative one, and `.cargo` in the
/// `home` folder (`$HOME`) otherwise; nothing names one when neither is set. The gate runs every cargo
/// command in `rust/`, so that is the folder a relative `$CARGO_HOME` lies in, wherever the rules run.
fn cargo_home(cargo_home: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>, cwd: &Path) -> Option<PathBuf> {
    match cargo_home {
        Some(named) if !named.is_empty() => Some(cwd.join(named)),
        _ => home.filter(|home| !home.is_empty()).map(|home| PathBuf::from(home).join(".cargo")),
    }
}

/// One `include` entry of a configuration: the path it names and whether Cargo may miss it. An entry
/// is a path or a `{ path = ..., optional = ... }` table; a table that names no path stops Cargo
/// itself, so it stops here too.
fn include_entry(value: &toml::Value) -> Option<(String, bool)> {
    match value {
        toml::Value::String(path) => Some((path.clone(), false)),
        toml::Value::Table(entry) => entry
            .get("path")
            .and_then(toml::Value::as_str)
            .map(|path| (path.to_string(), entry.get("optional").and_then(toml::Value::as_bool).unwrap_or(false))),
        _ => None,
    }
}

/// The files one configuration pulls in through Cargo's `include` key, resolved the way Cargo
/// resolves them: a relative path lies beside the file that includes it, and only `.toml` files are
/// configurations at all. A missing include that is not optional breaks Cargo's own read of the
/// build, so the finding says the configuration cannot be checked rather than pass it unseen.
fn included_by(file: &Path, document: &toml::Value, errors: &mut Vec<String>) -> Vec<PathBuf> {
    let Some(includes) = document.get("include") else { return Vec::new() };
    let name = file.display();
    let entries: Vec<&toml::Value> = match includes {
        toml::Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    let mut files = Vec::new();
    for entry in entries {
        let Some((text, optional)) = include_entry(entry) else {
            errors.push(format!("{name}: an include names no path, and Cargo stops on it before it reads anything else"));
            continue;
        };
        if !text.ends_with(".toml") {
            errors.push(format!("{name}: include of `{text}` is not a .toml file: Cargo reads no configuration that is not one"));
            continue;
        }
        let named = Path::new(&text);
        let path = if named.is_absolute() {
            named.to_path_buf()
        } else {
            match file.parent() {
                Some(parent) => parent.join(named),
                None => named.to_path_buf(),
            }
        };
        if path.is_file() {
            files.push(path);
        } else if !optional {
            errors.push(format!("{name}: include of `{text}` names no file: what Cargo would read for a build in rust/ is missing"));
        }
    }
    files
}

/// The configuration files Cargo reads for a build inside `root/rust`, before any `include`: the one
/// in `rust/` and in every folder above it, and the cargo home's — exactly the files Cargo probes,
/// in Cargo's own terms, so a carrier hidden in any of them is the finding it is in `rust/.cargo`.
fn configuration_files(root: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = root.join("rust").ancestors().filter_map(configuration_of).collect();
    files.extend(home.and_then(configuration_in_home));
    files
}

/// Every Cargo configuration a build inside `rust/` reads, checked for the carriers: the files
/// [`configuration_files`] names and, followed wherever it leads, the files they `include`. A file
/// reads once, so a cycle of includes ends where it begins. The other ways Cargo takes
/// configuration are closed elsewhere: the gate's own cargo commands carry no `--config`, and no
/// alias stands in for a built-in command; the environment is [`environment_carriers`]'; and a git
/// `url.<base>.insteadOf` can redirect Cargo's own fetches only to deliver the commits the lock names
/// — which [`lock_pin_errors`] holds to official ones — because the cache that decides which commits
/// are official never reads the machine's git configuration ([`Cache::command`]).
pub(crate) fn cargo_configuration_errors(root: &Path, home: Option<&Path>) -> Vec<String> {
    let mut errors = Vec::new();
    let mut read: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue = configuration_files(root, home);
    while let Some(file) = queue.pop() {
        if !read.insert(file.canonicalize().unwrap_or_else(|_| file.clone())) {
            continue;
        }
        let name = crate::report::shown(root, &file);
        let source = match std::fs::read_to_string(&file) {
            Ok(source) => source,
            Err(error) => {
                errors.push(format!("{name}: cannot be read: {error}"));
                continue;
            }
        };
        let document: toml::Value = match toml::from_str(&source) {
            Ok(document) => document,
            Err(error) => {
                errors.push(format!("{name}: does not parse as TOML: {error}; Cargo fails the build on it the same way"));
                continue;
            }
        };
        errors.extend(configuration_carriers(&document, &name));
        queue.extend(included_by(&file, &document, &mut errors));
    }
    errors
}

/// The directory holding the read-only git cache of the official EffectCraft. `tools/env.sh` exports
/// it; a bare `cargo xtask rules` that never set it gets the message naming that fix.
fn cache_dir() -> Result<PathBuf, String> {
    std::env::var_os("CRAFTBRIDGE_UPSTREAM_CACHE").map(PathBuf::from).filter(|path| path.is_absolute()).ok_or_else(|| {
        "CRAFTBRIDGE_UPSTREAM_CACHE is not set to an absolute path: run through tools/env.sh, which points it at the machine's read-only EffectCraft cache"
            .to_string()
    })
}

/// Is `rev` a commit id, in full or abbreviated, as git writes one? A branch, a tag or an expression such
/// as `main~3` resolves too, but to whatever it names on the day: the pin must not move.
fn commit_id(rev: &str) -> Result<(), String> {
    if !rev.is_empty() && rev.len() <= 40 && rev.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return Ok(());
    }
    Err(format!(
        "{rev} is no commit id: the EffectCraft pin names a commit of official main by its id, in full or abbreviated, never a branch, a tag or an expression that moves with one"
    ))
}

/// The machine's read-only git cache of official EffectCraft: a bare clone that fetches from the
/// official repository alone. The rule reads upstream's own manifest and lock from it, so a cache that
/// fetched from a fork would hold the workspace to a fork's tables; and it is where an official revision
/// is told from any other commit.
struct Cache(PathBuf);

impl Cache {
    /// The cache `tools/env.sh` names, as it stands.
    fn open() -> Result<Self, String> {
        Self::at(cache_dir()?)
    }

    /// The cache at `dir`, refused unless it is a bare clone of the official repository: the URL its
    /// fetches really use, after any `url.<base>.insteadOf` rewrite in its own configuration, must be the
    /// official one.
    fn at(dir: PathBuf) -> Result<Self, String> {
        if !dir.join("objects").is_dir() {
            return Err(format!("there is no upstream cache at {}", dir.display()));
        }
        let official = app_url(effectcraft_app()?);
        let cache = Self(dir);
        let origin = cache.git(&["ls-remote", "--get-url", "origin"]).unwrap_or_default();
        if origin != official {
            return Err(format!(
                "the upstream cache at {} fetches from {}, not {official}: point CRAFTBRIDGE_UPSTREAM_CACHE at a bare clone of the official repository, or move it aside",
                cache.0.display(),
                if origin.is_empty() { "no origin" } else { origin.as_str() }
            ));
        }
        Ok(cache)
    }

    /// The cache `tools/env.sh` names, made first when this machine has none: an empty bare clone
    /// fetching from the official repository. A folder that is there already and is no cache is left
    /// alone and refused.
    fn create() -> Result<Self, String> {
        let dir = cache_dir()?;
        let empty = std::fs::read_dir(&dir).map(|mut entries| entries.next().is_none()).unwrap_or(true);
        if !dir.join("objects").is_dir() && empty {
            std::fs::create_dir_all(&dir).map_err(|error| format!("cannot create the upstream cache at {}: {error}", dir.display()))?;
            crate::git::run(Self::command(&dir), &["init", "--quiet", "--bare"])?;
            crate::git::run(Self::command(&dir), &["--git-dir=.", "remote", "add", "origin", &app_url(effectcraft_app()?)])?;
        }
        Self::at(dir)
    }

    /// Git started on the cache with the cache's own configuration the only one it reads. A
    /// `url.<base>.insteadOf` there would rewrite the official URL and fill the cache's main from anywhere,
    /// and that main is what decides which commits are official. The user's file and the system's file are
    /// refused by name; configuration carried by the environment, which outranks every file, is refused by
    /// [`crate::git`] starting git without the variables git lists, among them git's own environment
    /// configuration. The cache's own configuration, which [`Cache::at`] holds to the official URL, is all
    /// its git reads; a proxy still reaches git through the environment.
    fn command(dir: &Path) -> Command {
        let mut git = crate::git::command(dir);
        git.env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1");
        git
    }

    /// A git command on the cache itself: `--git-dir` names it, so a folder set inside some other
    /// repository is never taken for that repository.
    fn git(&self, args: &[&str]) -> Result<String, String> {
        let mut all = vec!["--git-dir=."];
        all.extend_from_slice(args);
        crate::git::run(Self::command(&self.0), &all).map(|out| String::from_utf8_lossy(&out).trim().to_string())
    }

    /// Bring official main into the cache. Forced, so the cache's main is upstream's as it stands: a
    /// commit upstream's main no longer reaches stops being an official revision here too.
    fn fetch_main(&self) -> Result<(), String> {
        self.git(&["fetch", "--quiet", "origin", "+refs/heads/main:refs/heads/main"]).map(drop)
    }

    /// The full commit of the official revision `rev`. Official means merged: official main reaches
    /// it. Being fetchable from the official URL proves nothing — GitHub serves any commit of the
    /// repository by its id, a pull request's head included, and so does Cargo's fetch — so a commit the
    /// cache holds for any other reason is refused.
    fn official_commit(&self, rev: &str) -> Result<String, String> {
        commit_id(rev)?;
        let full = self.git(&["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")]).map_err(|_| {
            format!(
                "the upstream cache at {} holds no commit {rev}: tools/update-upstream.sh {rev} fetches official main into it, and a revision it still cannot find there is none of official main's",
                self.0.display()
            )
        })?;
        if !full.starts_with(rev) {
            return Err(format!("{rev} resolves to {full}, which it does not abbreviate: name the commit by its id"));
        }
        if self.git(&["for-each-ref", "--contains", &full, "--format=%(refname)", "refs/heads/main"])?.is_empty() {
            return Err(format!(
                "{rev} ({full}) is not on official main: only a commit storytold/effectcraft has merged is an official revision (AGENTS.md \u{a7}7), never a pull request's or another branch's"
            ));
        }
        Ok(full)
    }

    /// One file of `commit`.
    fn file(&self, commit: &str, name: &str) -> Result<String, String> {
        self.git(&["show", &format!("{commit}:{name}")])
            .map_err(|error| format!("cannot read {name} of the pinned EffectCraft from {}: {error}", self.0.display()))
    }
}

/// The revision text to write: the tree's own spelling of `full` when it already names that commit —
/// in full or abbreviated — and otherwise the spelling asked for. A rerun of the same revision by its
/// full id, when the manifest names it by an abbreviated one, is the same revision and must write the
/// same bytes; taking the requested spelling instead would rewrite every pin and churn the lock's
/// source strings for no reason beyond a difference in id length.
fn rev_text(ours: &str, full: &str, requested: &str, cache: &Cache) -> String {
    match pins_from_manifest(ours) {
        Ok(pin) if cache.official_commit(&pin.effectcraft_rev).as_deref() == Ok(full) => pin.effectcraft_rev,
        _ => requested.to_string(),
    }
}

/// `cargo xtask upstream pin <rev>`: fetch official main into the cache when it does not reach `rev`
/// yet, then write the pins, `[patch]`, `[profile.release]` and the starting lock for that revision and
/// let Cargo resolve the workspace on top of that lock (plan \u{a7}24 steps 1 and 2). When the tree already
/// holds exactly what the revision writes, nothing is written: the lock then stays as Cargo left it,
/// rather than having CraftBridge's own crates resolved again against whatever the registry offers on
/// the day, so a second run on the same revision changes nothing.
pub(crate) fn pin(root: &Path, rev: &str) -> Result<String, String> {
    // Before the cache is touched: a name that is no commit id neither makes a cache nor fetches.
    commit_id(rev)?;
    let cache = Cache::create()?;
    let full = match cache.official_commit(rev) {
        Ok(full) => full,
        Err(_) => {
            cache.fetch_main()?;
            cache.official_commit(rev)?
        }
    };
    let upstream_manifest = cache.file(&full, "Cargo.toml")?;
    let upstream_lock = cache.file(&full, "Cargo.lock")?;
    let filmcraft_rev = upstream_filmcraft_rev(&upstream_manifest)?;
    let manifest_path = root.join("rust/Cargo.toml");
    let ours = std::fs::read_to_string(&manifest_path).map_err(|error| format!("cannot read rust/Cargo.toml: {error}"))?;
    let spelling = rev_text(&ours, &full, rev, &cache);
    let written = regenerate(&ours, &upstream_manifest, &spelling, &filmcraft_rev)?;
    if written == ours {
        return Ok(format!("rust/Cargo.toml already holds effectcraft {spelling} ({full}), filmcraft {filmcraft_rev} and their tables: nothing written"));
    }
    std::fs::write(&manifest_path, written).map_err(|error| format!("cannot write rust/Cargo.toml: {error}"))?;
    std::fs::write(root.join("rust/Cargo.lock"), &upstream_lock).map_err(|error| format!("cannot write rust/Cargo.lock: {error}"))?;
    // Resolve CraftBridge's own crates on top of the copied lock (D-21): versions upstream locked
    // stay locked; only what CraftBridge adds beyond them resolves fresh.
    let output = Command::new(env!("CARGO"))
        .current_dir(root.join("rust"))
        .args(["metadata", "--format-version", "1"])
        .output()
        .map_err(|error| format!("cannot run cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!("cargo metadata failed while resolving the pin:\n{}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(format!("pinned effectcraft {spelling} ({full}), filmcraft {filmcraft_rev}; rust/Cargo.lock started from upstream's and resolved"))
}

/// `cargo xtask upstream pin <rev>` as a command.
pub(crate) fn run(root: &Path, rev: &str) -> std::process::ExitCode {
    match pin(root, rev) {
        Ok(done) => {
            println!("upstream: {done}");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("upstream: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// The `upstream` rule check. The pinned revision's manifest and lock are read from the read-only cache
/// `tools/update-upstream.sh` maintains; a cache without the pin is a failure with a named fix, not a
/// check that silently checked nothing.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    // What Cargo's configuration carries is read whatever state the pin is in, so one run names a
    // carrier beside every other finding rather than after the pin is put right.
    let home = cargo_home(std::env::var_os("CARGO_HOME"), std::env::var_os("HOME"), &root.join("rust"));
    let mut errors = cargo_configuration_errors(root, home.as_deref());
    errors.extend(environment_carriers(std::env::vars_os()));
    let text = crate::scan::read_text(&root.join("rust/Cargo.toml"))?;
    let pin = match pins_from_manifest(&text) {
        Ok(pin) => pin,
        Err(found) => {
            errors.extend(found);
            return Ok(errors);
        }
    };
    let rev = &pin.effectcraft_rev;
    let cache = match Cache::open() {
        Ok(cache) => cache,
        Err(error) => {
            errors.push(format!(
                "{error}; run tools/update-upstream.sh {rev}, which fetches the pinned revision and, on a tree that already holds it, writes nothing"
            ));
            return Ok(errors);
        }
    };
    let full = match cache.official_commit(rev) {
        Ok(full) => full,
        Err(error) => {
            errors.push(format!("rust/Cargo.toml pins EffectCraft at {rev}: {error}"));
            return Ok(errors);
        }
    };
    let upstream_manifest = cache.file(&full, "Cargo.toml")?;
    match upstream_filmcraft_rev(&upstream_manifest) {
        Ok(film) => {
            if film != pin.filmcraft_rev {
                errors.push(format!(
                    "rust/Cargo.toml pins FilmCraft at {}, but the pinned EffectCraft names {film}: FilmCraft's revision is EffectCraft's own (plan \u{a7}24)",
                    pin.filmcraft_rev
                ));
            }
        }
        Err(error) => errors.push(error),
    }
    match regenerate(&text, &upstream_manifest, rev, &pin.filmcraft_rev) {
        Ok(regenerated) => errors.extend(generated_table_errors(&text, &regenerated)),
        Err(error) => errors.push(error),
    }
    let lock = crate::scan::read_text(&root.join("rust/Cargo.lock"))?;
    let upstream = upstream_sources(&cache.file(&full, "Cargo.lock")?)?;
    errors.extend(lock_pin_errors(&lock, &pin, &full, &upstream)?);
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use crate::scratch::Scratch;

    use super::*;

    fn workspace_manifest(effect_rev: &str, film_rev: &str) -> String {
        let mut text = String::from("[workspace]\nresolver = \"3\"\n\n[workspace.dependencies]\ncraftbridge-core = { path = \"core/craftbridge-core\" }\n");
        for name in EFFECTCRAFT_CRATES {
            text.push_str(&format!("{name} = {{ git = \"https://github.com/storytold/effectcraft\", rev = \"{effect_rev}\" }}\n"));
        }
        for name in core_named_upstream() {
            text.push_str(&format!("{name} = {{ git = \"https://github.com/storytold/filmcraft\", rev = \"{film_rev}\" }}\n"));
        }
        text
    }

    fn upstream_manifest(film_rev: &str, library_rev: &str) -> String {
        format!(
            "[workspace.dependencies]\neffectcraft-engine = {{ path = \"crates/engine\" }}\nfilmcraft-codecs = {{ git = \"https://github.com/storytold/filmcraft\", rev = \"{film_rev}\" }}\ncraft-docpath = {{ git = \"https://github.com/storytold/craft-libs\", rev = \"{library_rev}\" }}\n\n[profile.release]\nlto = \"thin\"\ncodegen-units = 1\n\n[patch.crates-io]\nserde = {{ git = \"https://github.com/storytold/example\", rev = \"beef\" }}\n"
        )
    }

    #[test]
    fn the_pins_are_read_from_the_workspace_manifest() {
        let pin = pins_from_manifest(&workspace_manifest("10376058", "d5ead4ba")).expect("the fixture is a pin");
        assert_eq!(pin.effectcraft_rev, "10376058");
        assert_eq!(pin.filmcraft_rev, "d5ead4ba");
    }

    #[test]
    fn a_pin_outside_the_official_repository_or_moving_is_refused() {
        let fork = workspace_manifest("10376058", "d5ead4ba").replace("https://github.com/storytold/effectcraft", "https://github.com/someone/effectcraft");
        assert!(
            pins_from_manifest(&fork)
                .is_err_and(|errors| errors.iter().any(|error| error.contains("effectcraft-engine") && error.contains("official EffectCraft")))
        );
        let moving = workspace_manifest("10376058", "d5ead4ba").replace("rev = \"10376058\"", "branch = \"main\"");
        assert!(pins_from_manifest(&moving).is_err_and(|errors| errors.iter().any(|error| error.contains("rev"))));
        assert!(
            pins_from_manifest("[workspace]\n[workspace.dependencies]\n").is_err_and(|errors| errors.iter().any(|error| error.contains("update-upstream")))
        );
    }

    #[test]
    fn two_revisions_of_one_upstream_are_refused() {
        let mixed = workspace_manifest("10376058", "d5ead4ba").replace(
            "effectcraft-time = { git = \"https://github.com/storytold/effectcraft\", rev = \"10376058\" }",
            "effectcraft-time = { git = \"https://github.com/storytold/effectcraft\", rev = \"another\" }",
        );
        assert!(pins_from_manifest(&mixed).is_err_and(|errors| errors.iter().any(|error| error.contains("two different revisions"))));
    }

    #[test]
    fn upstream_names_its_filmcraft_and_craft_library_revisions() {
        assert_eq!(upstream_filmcraft_rev(&upstream_manifest("d5ead4ba", "e7c3dda")).expect("one revision"), "d5ead4ba");
        let revs = upstream_git_revs(&upstream_manifest("d5ead4ba", "e7c3dda"), "https://github.com/storytold/craft-libs");
        assert_eq!(revs, BTreeSet::from(["e7c3dda".to_string()]));
    }

    #[test]
    fn the_generated_tables_land_and_a_second_run_writes_nothing_new() {
        let ours = workspace_manifest("10376058", "d5ead4ba");
        let once = regenerate(&ours, &upstream_manifest("d5ead4ba", "e7c3dda"), "10376058", "d5ead4ba").expect("the fixture parses");
        assert!(once.contains("lto = \"thin\"") && once.contains("[patch.crates-io]"), "{once}");
        assert_eq!(
            once,
            regenerate(&once, &upstream_manifest("d5ead4ba", "e7c3dda"), "10376058", "d5ead4ba").expect("the rerun parses"),
            "running the generator twice writes the same bytes"
        );
        assert!(generated_table_errors(&once, &once).is_empty(), "the generated tables of a generated file match themselves");
        let missing = generated_table_errors(&ours, &once);
        assert!(
            missing.iter().any(|error| error.contains("[patch]")) && missing.iter().any(|error| error.contains("[profile.release]")),
            "tables the script has not written yet are findings: {missing:?}"
        );
        assert!(once.contains("craftbridge-core"), "what the script does not generate is kept: {once}");
    }

    #[test]
    fn a_hand_written_patch_entry_or_profile_key_is_a_finding() {
        let ours = workspace_manifest("10376058", "d5ead4ba");
        let written = regenerate(&ours, &upstream_manifest("d5ead4ba", "e7c3dda"), "10376058", "d5ead4ba").expect("the fixture parses");
        let planted = written.replace("[patch.crates-io]", "[patch.crates-io]\nmy-copy = { path = \"../a-changed-effectcraft\" }");
        let errors = generated_table_errors(&planted, &written);
        assert!(errors.iter().any(|error| error.contains("[patch]")), "{errors:?}");
        let slower = written.replace("lto = \"thin\"", "lto = false");
        assert!(generated_table_errors(&slower, &written).iter().any(|error| error.contains("[profile.release]")));
    }

    fn lock_entry(name: &str, source: &str) -> String {
        format!("[[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\nsource = \"{source}\"\n")
    }

    const EFFECTCRAFT_COMMIT: &str = "1037605800000000000000000000000000000000";
    const FILMCRAFT_SOURCE: &str = "git+https://github.com/storytold/filmcraft?rev=d5ead4ba#d5ead4ba11111111111111111111111111111111";
    const LIBRARY_SOURCE: &str = "git+https://github.com/storytold/craft-libs?rev=e7c3dda#e7c3dda222222222222222222222222222222222";
    /// The second Craft library revision upstream's own lock resolves, for one crate only: FilmCraft's.
    const FILMCRAFT_LIBRARY_SOURCE: &str = "git+https://github.com/storytold/craft-libs?rev=f0ad6c1e#f0ad6c1e33333333333333333333333333333333";

    fn good_lock() -> String {
        let mut text = lock_entry("craftbridge-core", "path+file:///repo/rust/core/craftbridge-core#0.1.0");
        for name in EFFECTCRAFT_CRATES {
            text.push_str(&lock_entry(name, &format!("git+https://github.com/storytold/effectcraft?rev=10376058#{EFFECTCRAFT_COMMIT}")));
        }
        for name in core_named_upstream() {
            text.push_str(&lock_entry(name, FILMCRAFT_SOURCE));
        }
        text.push_str(&lock_entry("craft-docpath", LIBRARY_SOURCE));
        text.push_str(&lock_entry("craft-media", LIBRARY_SOURCE));
        text.push_str(&lock_entry("craft-media", FILMCRAFT_LIBRARY_SOURCE));
        text
    }

    /// Upstream's own lock for the fixture: EffectCraft's crates are its workspace members, so they carry
    /// no source; FilmCraft and the Craft libraries at the revisions it resolves, `craft-media` at two.
    fn upstream() -> BTreeMap<String, BTreeSet<String>> {
        let mut text = lock_entry("effectcraft-engine", "").replace("source = \"\"\n", "");
        for name in ["filmcraft-codecs", "filmcraft-isobmff", "filmcraft-time"] {
            text.push_str(&lock_entry(name, FILMCRAFT_SOURCE));
        }
        text.push_str(&lock_entry("craft-docpath", LIBRARY_SOURCE));
        text.push_str(&lock_entry("craft-media", LIBRARY_SOURCE));
        text.push_str(&lock_entry("craft-media", FILMCRAFT_LIBRARY_SOURCE));
        text.push_str(&lock_entry("serde", "registry+https://github.com/rust-lang/crates.io-index"));
        upstream_sources(&text).expect("the fixture parses")
    }

    fn fixture_pin() -> Pin {
        Pin { effectcraft_rev: "10376058".to_string(), filmcraft_rev: "d5ead4ba".to_string() }
    }

    fn lock_errors(lock: &str) -> Vec<String> {
        lock_pin_errors(lock, &fixture_pin(), EFFECTCRAFT_COMMIT, &upstream()).expect("the fixture parses")
    }

    #[test]
    fn the_lock_at_the_pin_passes_with_upstream_s_own_split() {
        let errors = lock_errors(&good_lock());
        assert!(errors.is_empty(), "craft-media at the two revisions upstream's own lock resolves is upstream's split: {errors:?}");
    }

    #[test]
    fn a_registry_a_fork_another_revision_or_a_path_copy_is_a_finding() {
        let registry =
            good_lock().replace("git+https://github.com/storytold/effectcraft?rev=10376058#", "registry+https://github.com/rust-lang/crates.io-index#");
        assert!(!lock_errors(&registry).is_empty(), "a registry copy of an EffectCraft crate");
        let fork = good_lock().replace("github.com/storytold/effectcraft", "github.com/someone/effectcraft");
        assert!(!lock_errors(&fork).is_empty(), "a fork is not the official revision");
        let moved = good_lock().replace("?rev=10376058", "?rev=anotherone");
        assert!(!lock_errors(&moved).is_empty(), "another revision is not the pin");
        let path_copy = good_lock().replacen(&format!("source = \"git+https://github.com/storytold/effectcraft?rev=10376058#{EFFECTCRAFT_COMMIT}\"\n"), "", 1);
        let errors = lock_errors(&path_copy);
        assert!(errors.iter().any(|error| error.contains("effectcraft-engine") && error.contains("local path")), "{errors:?}");
        let other_library = good_lock().replace("craft-libs?rev=e7c3dda", "craft-libs?rev=deadbeef");
        assert!(!lock_errors(&other_library).is_empty(), "a Craft library at a revision upstream does not resolve");
    }

    /// The commit after the `#` is what Cargo builds, whatever the revision before it says, so a lock that
    /// keeps the pinned revision and swaps the commit builds code nobody pinned.
    #[test]
    fn a_lock_entry_is_checked_down_to_the_commit_cargo_builds() {
        let effectcraft = format!("git+https://github.com/storytold/effectcraft?rev=10376058#{EFFECTCRAFT_COMMIT}");
        for (source, name) in [(effectcraft.as_str(), "effectcraft-engine"), (FILMCRAFT_SOURCE, "filmcraft-codecs"), (LIBRARY_SOURCE, "craft-docpath")] {
            let (base, _) = source.split_once('#').expect("a fixture source names its commit");
            let other = format!("{base}#{}", "9".repeat(40));
            let errors = lock_errors(&good_lock().replace(source, &other));
            assert!(errors.iter().any(|error| error.contains(name) && error.contains(&other)), "{name} at a commit nobody pinned: {errors:?}");
        }
    }

    /// The duplicate rule lets every Craft library split from the official repository through; this rule
    /// is what holds that split to upstream's own.
    #[test]
    fn a_second_copy_upstream_does_not_carry_is_a_finding() {
        let docpath_twice = format!("{}{}", good_lock(), lock_entry("craft-docpath", FILMCRAFT_LIBRARY_SOURCE));
        let errors = lock_errors(&docpath_twice);
        assert!(
            errors.len() == 1 && errors[0].contains("craft-docpath") && errors[0].contains("f0ad6c1e"),
            "craft-docpath at the revision upstream resolves only craft-media at: {errors:?}"
        );
        let ours_alone = format!("{}{}", good_lock(), lock_entry("craft-quit", LIBRARY_SOURCE));
        assert!(lock_errors(&ours_alone).is_empty(), "a library crate upstream does not use, once, from a revision upstream resolves");
        let ours_twice = format!("{ours_alone}{}", lock_entry("craft-quit", FILMCRAFT_LIBRARY_SOURCE));
        let errors = lock_errors(&ours_twice);
        assert!(errors.len() == 2 && errors.iter().all(|error| error.contains("craft-quit") && error.contains("one copy")), "{errors:?}");
        let film_from_libraries = format!("{}{}", good_lock(), lock_entry("filmcraft-new", LIBRARY_SOURCE));
        assert!(!lock_errors(&film_from_libraries).is_empty(), "a FilmCraft crate upstream does not use still comes from a FilmCraft source");
    }

    #[test]
    fn a_workspace_without_the_pinned_crates_in_lock_is_a_finding() {
        let lock = lock_entry("craftbridge-core", "path+file:///repo/rust/core/craftbridge-core#0.1.0");
        let errors = lock_errors(&lock);
        assert!(errors.iter().any(|error| error.contains("effectcraft-engine") && error.contains("depends on the pinned upstream")), "{errors:?}");
    }

    #[test]
    fn a_pin_is_a_commit_id_never_a_name_that_moves() {
        for rev in ["10376058", "103760584a60074c9aaeef686d2d05d89f08155e", "1037"] {
            assert!(commit_id(rev).is_ok(), "{rev}");
        }
        for rev in ["main", "main~32", "HEAD", "v0.24.1", "10376058^", "1037605G", "1037605A", "", &"1".repeat(41)] {
            assert!(commit_id(rev).is_err_and(|error| error.contains("no commit id")), "{rev}");
        }
    }

    /// A bare scratch cache that fetches from the official URL, with one commit, which main points at.
    fn official_scratch_cache(tag: &str) -> (Scratch, String) {
        let scratch = Scratch::new(tag);
        crate::git::output(&scratch.0, &["init", "--quiet", "--bare"]).unwrap();
        crate::git::output(&scratch.0, &["--git-dir=.", "remote", "add", "origin", "https://github.com/storytold/effectcraft"]).unwrap();
        let first = commit(&scratch, None, "one");
        point_main_at(&scratch, &first);
        (scratch, first)
    }

    fn point_main_at(cache: &Scratch, commit: &str) {
        crate::git::output(&cache.0, &["--git-dir=.", "update-ref", "refs/heads/main", commit]).unwrap();
    }

    /// One commit on the scratch repository, made the way git's plumbing makes one in a bare repository.
    fn commit(cache: &Scratch, parent: Option<&str>, message: &str) -> String {
        let tree = String::from_utf8(crate::git::output(&cache.0, &["--git-dir=.", "mktree"]).unwrap()).unwrap();
        let mut args = vec!["--git-dir=.", "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit-tree", tree.trim(), "-m", message];
        if let Some(parent) = parent {
            args.extend(["-p", parent]);
        }
        String::from_utf8(crate::git::output(&cache.0, &args).unwrap()).unwrap().trim().to_string()
    }

    /// An official revision is a commit official main reaches, named by its id: an unmerged commit the
    /// cache happens to hold — a pull request's head, say — and a name or expression are refused, and so
    /// is a cache that fetches from anywhere but the official repository.
    #[test]
    fn an_official_revision_is_a_commit_official_main_reaches() {
        let (scratch, first) = official_scratch_cache("upstream-cache");
        let second = commit(&scratch, Some(&first), "two");
        let unmerged = commit(&scratch, Some(&first), "a pull request's head");
        point_main_at(&scratch, &second);
        crate::git::output(&scratch.0, &["--git-dir=.", "update-ref", "refs/pull/1/head", &unmerged]).unwrap();
        let cache = Cache::at(scratch.0.clone()).expect("a bare clone of the official repository");
        assert_eq!(cache.official_commit(&second[..8]).as_deref(), Ok(second.as_str()), "an abbreviated id of main's tip");
        assert_eq!(cache.official_commit(&first).as_deref(), Ok(first.as_str()), "a commit main reaches, in full");
        let refused = cache.official_commit(&unmerged).expect_err("a commit main does not reach");
        assert!(refused.contains("not on official main"), "{refused}");
        for name in ["main", "main~1", "HEAD"] {
            assert!(cache.official_commit(name).is_err_and(|error| error.contains("no commit id")), "{name} resolves, and moves");
        }
        crate::git::output(&scratch.0, &["--git-dir=.", "remote", "set-url", "origin", "https://github.com/someone/effectcraft"]).unwrap();
        assert!(Cache::at(scratch.0.clone()).is_err_and(|error| error.contains("someone/effectcraft")), "a fork's clone is no upstream cache");
        let empty = Scratch::new("upstream-cache-none");
        assert!(Cache::at(empty.0.clone()).is_err_and(|error| error.contains("no upstream cache")));
    }

    #[test]
    fn a_cargo_configuration_may_not_patch_path_or_replace_around_the_pin() {
        // Cargo reads `[patch]`, the old `paths` key and `[source]` replacements from its configuration;
        // each stands between the build and the pinned upstream, so the gate names the configuration
        // carrying it and points the fix at the one place patches live.
        let carriers = |source: &str| -> Vec<String> {
            let document: toml::Value = toml::from_str(source).expect("the fixture parses");
            configuration_carriers(&document, "c")
        };
        let patch = carriers("[patch.crates-io]\neffectcraft-engine = { path = \"../local/effectcraft\" }\n");
        assert!(patch.iter().any(|error| error.contains("[patch]") && error.contains("rust/Cargo.toml")), "{patch:?}");
        let paths = carriers("paths = [\"../local/effectcraft\"]\n");
        assert_eq!(paths.len(), 1, "{paths:?}");
        assert!(paths[0].contains("paths") && paths[0].contains("local folder"), "{paths:?}");
        let vendored = "[source.effectcraft]\ngit = \"https://github.com/storytold/effectcraft\"\nrev = \"10376058\"\nreplace-with = \"vendored\"\n\n[source.vendored]\ndirectory = \"/elsewhere/vendor\"\n";
        let replaced = carriers(vendored);
        assert!(replaced.len() == 1 && replaced[0].contains("[source]"), "{replaced:?}");
    }

    /// A checkout under a scratch root, clean below and ready to be planted above.
    fn scratch_checkout(tag: &str) -> (Scratch, PathBuf) {
        let tree = Scratch::new(tag);
        tree.mkdir("checkout/rust");
        let checkout = tree.0.join("checkout");
        (tree, checkout)
    }

    #[test]
    fn a_carrier_in_the_repository_or_above_the_checkout_is_named_as_cargo_reads_it() {
        let (tree, checkout) = scratch_checkout("upstream-config-tree");
        tree.write("checkout/rust/.cargo/config.toml", "[alias]\nxtask = \"run --package xtask --\"\n");
        let errors = cargo_configuration_errors(&checkout, None);
        assert!(errors.is_empty(), "the workspace alias is what a configuration is for: {errors:?}");
        tree.write("checkout/rust/.cargo/config.toml", "[patch.crates-io]\nany = { path = \"../elsewhere\" }\n");
        tree.write(".cargo/config.toml", "[source.planted]\ndirectory = \"/elsewhere\"\n");
        let errors = cargo_configuration_errors(&checkout, None);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("rust/.cargo/config.toml: [patch]")), "{errors:?}");
        assert!(errors.iter().any(|e| e.ends_with("[source] replaces where Cargo takes crates from while the lock still names the official source, so a changed copy of the pinned upstream would build unseen") && e.contains(".cargo/config.toml")), "{errors:?}");
    }

    #[test]
    fn a_legacy_configuration_is_the_one_read_and_the_toml_one_beside_it_is_not() {
        // Cargo reads `.cargo/config` instead of `.cargo/config.toml` when both sit in one folder,
        // so the gate reads exactly what Cargo reads: a carrier only the unread file holds is no
        // finding, and once the file Cargo reads carries one itself, it names that file alone.
        let (tree, checkout) = scratch_checkout("upstream-config-legacy");
        tree.write("checkout/.cargo/config", "[net]\ngit-fetch-with-cli = true\n");
        tree.write("checkout/.cargo/config.toml", "paths = [\"/elsewhere\"]\n");
        assert!(cargo_configuration_errors(&checkout, None).is_empty(), "Cargo never reads the file the legacy one hides");
        tree.write("checkout/.cargo/config", "[patch.crates-io]\nany = { path = \"../elsewhere\" }\n");
        let errors = cargo_configuration_errors(&checkout, None);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with(".cargo/config: [patch]"), "{errors:?}");
    }

    #[test]
    fn the_cargo_home_configuration_is_read_once_even_when_a_folder_above_is_cargo_home() {
        let (tree, checkout) = scratch_checkout("upstream-config-home");
        tree.write("cargo-home/config.toml", "paths = [\"/elsewhere\"]\n");
        let errors = cargo_configuration_errors(&checkout, Some(&tree.0.join("cargo-home")));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("cargo-home/config.toml") && errors[0].contains("paths"), "{errors:?}");
        // A cargo home that is also the `.cargo` of a folder above the checkout is one file, checked once.
        let (tree, checkout) = scratch_checkout("upstream-config-home-ancestor");
        tree.write("checkout/../.cargo/config.toml", "paths = [\"/elsewhere\"]\n");
        let home = tree.0.join(".cargo");
        let errors = cargo_configuration_errors(&checkout, Some(&home));
        assert_eq!(errors.len(), 1, "the same file is no two findings: {errors:?}");
    }

    #[test]
    fn configurations_pulled_in_by_include_are_read_as_if_written_into_the_file_that_includes_them() {
        let (tree, checkout) = scratch_checkout("upstream-config-include");
        let cargo_dir = "checkout/rust/.cargo";
        tree.write(&format!("{cargo_dir}/config.toml"), "include = [\"carriers.toml\", { path = \"none.toml\", optional = true }]\n");
        tree.write(&format!("{cargo_dir}/carriers.toml"), "[patch.crates-io]\nany = { path = \"../elsewhere\" }\n");
        let errors = cargo_configuration_errors(&checkout, None);
        assert_eq!(errors.len(), 1, "the optional include that is absent is no finding: {errors:?}");
        assert!(errors[0].contains("carriers.toml") && errors[0].contains("[patch]"), "{errors:?}");
        // An include Cargo itself would stop on is a finding naming the file that includes it,
        // whether the name is gone or is no configuration at all; a cycle of includes ends where it begins.
        tree.write(&format!("{cargo_dir}/config.toml"), "include = \"gone.toml\"\n");
        let errors = cargo_configuration_errors(&checkout, None);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("gone.toml` names no file"), "{errors:?}");
        tree.write(&format!("{cargo_dir}/config.toml"), "include = [\"loop-a.toml\", \"notes.txt\"]\n");
        tree.write(&format!("{cargo_dir}/loop-a.toml"), "include = [\"loop-b.toml\"]\n");
        tree.write(&format!("{cargo_dir}/loop-b.toml"), "include = [\"loop-a.toml\"]\n");
        tree.write(&format!("{cargo_dir}/notes.txt"), "[patch.crates-io]\nany = { path = \"../elsewhere\" }\n");
        let errors = cargo_configuration_errors(&checkout, None);
        assert_eq!(errors.len(), 1, "the cycle ends where it begins, and a non-`.toml` include is refused unread: {errors:?}");
        assert!(errors[0].contains("notes.txt"), "{errors:?}");
    }

    #[test]
    fn a_rerun_by_the_full_id_keeps_the_spelling_the_tree_already_holds() {
        // The pin is the commit, not the way its id is written: a tree naming it in full, or in
        // eight characters, already holds what the same revision's script writes and says so.
        let (scratch, only) = official_scratch_cache("upstream-rev-text");
        let cache = Cache::at(scratch.0.clone()).expect("a bare clone of the official repository");
        let abbreviated = workspace_manifest(&only[..8], "d5ead4ba");
        assert_eq!(rev_text(&abbreviated, &only, &only, &cache), only[..8], "a rerun by the full id takes the tree's own spelling");
        assert_eq!(rev_text(&abbreviated, &only, &only, &cache).len(), 8, "the lock's source strings stay as they are");
        let in_full = workspace_manifest(&only, "d5ead4ba");
        assert_eq!(rev_text(&in_full, &only, &only, &cache), only, "a tree written in full stays in full");
        let next = commit(&scratch, Some(&only), "two");
        point_main_at(&scratch, &next);
        assert_eq!(rev_text(&abbreviated, &next, &next, &cache), next, "another revision is written as asked for");
    }

    #[test]
    fn a_carrier_in_the_environment_is_named_by_its_variable() {
        let environment = [
            ("CARGO_SOURCE_FILMLIBS_REPLACE_WITH", "PLANTED"),
            ("CARGO_SOURCE_PLANTED_DIRECTORY", "/elsewhere"),
            ("CARGO_PATCH_CRATES_IO_ANY_PATH", "/elsewhere"),
            ("CARGO_PATHS", "/elsewhere"),
            ("CARGO_HOME", "/home/.cargo"),
            ("CARGO_SOURCES", "not a key Cargo reads"),
            ("CARGO_PKG_NAME", "xtask"),
        ]
        .map(|(name, value)| (std::ffi::OsString::from(name), std::ffi::OsString::from(value)));
        let errors = environment_carriers(environment);
        assert_eq!(errors.len(), 4, "the four carriers and none of the rest: {errors:?}");
        for (name, carrier) in [
            ("CARGO_SOURCE_FILMLIBS_REPLACE_WITH", "[source]"),
            ("CARGO_SOURCE_PLANTED_DIRECTORY", "[source]"),
            ("CARGO_PATCH_CRATES_IO_ANY_PATH", "[patch]"),
            ("CARGO_PATHS", "paths"),
        ] {
            assert!(errors.iter().any(|error| error.contains(&format!("environment's {name}:")) && error.contains(carrier)), "{name}: {errors:?}");
        }
    }

    /// The URL the cache fetches from is the one its fetches really use: a rewrite in the cache's own
    /// configuration points them elsewhere, and is refused.
    #[test]
    fn a_cache_whose_own_configuration_rewrites_the_official_url_is_refused() {
        let (scratch, _) = official_scratch_cache("upstream-cache-rewritten");
        assert!(Cache::at(scratch.0.clone()).is_ok());
        crate::git::output(&scratch.0, &["--git-dir=.", "config", "url./elsewhere/effectcraft.git.insteadOf", "https://github.com/storytold/effectcraft"])
            .unwrap();
        assert!(Cache::at(scratch.0.clone()).is_err_and(|error| error.contains("/elsewhere/effectcraft.git")), "the rewritten URL is what it fetches from");
    }

    /// The child half of the test below: run only when the parent starts it with git configuration that
    /// rewrites the official URL — the same rewrite in the user's file, the system's file and the
    /// environment, the route plain git still honors shown first, so the cache's answer can only come from
    /// its own configuration.
    #[test]
    #[ignore = "run by the_machine_s_git_configuration_cannot_redirect_the_cache, with GIT_CONFIG_GLOBAL, GIT_CONFIG_SYSTEM and GIT_CONFIG_COUNT set"]
    fn child_opens_the_cache_past_a_machine_rewrite() {
        let Some(own) = std::env::var_os("XTASK_UPSTREAM_CACHE_OWN") else { return };
        let own = PathBuf::from(own);
        let plain = crate::git::output(&own, &["--git-dir=.", "ls-remote", "--get-url", "origin"]).unwrap();
        assert_eq!(String::from_utf8_lossy(&plain).trim(), "/elsewhere/effectcraft.git", "the machine's configuration does rewrite the URL for plain git");
        let cache = Cache::at(own).unwrap();
        assert_eq!(cache.git(&["ls-remote", "--get-url", "origin"]).unwrap(), "https://github.com/storytold/effectcraft", "the cache's git never reads it");
    }

    /// A `url.<base>.insteadOf` would fill the cache's main, which decides what is official, from wherever
    /// it points. Git takes that setting from the user's file, the system's file and the environment, the
    /// environment above every file, so the cache's git reads none of them: only the cache's own
    /// configuration, which [`Cache::at`] holds to the official URL.
    #[test]
    fn the_machine_s_git_configuration_cannot_redirect_the_cache() {
        let (scratch, _) = official_scratch_cache("upstream-cache-machine");
        let machine = Scratch::new("upstream-cache-machine-config");
        machine.write("gitconfig", "[url \"/elsewhere/effectcraft.git\"]\n\tinsteadOf = https://github.com/storytold/effectcraft\n");
        let child = crate::scratch::rerun(
            "upstream::tests::child_opens_the_cache_past_a_machine_rewrite",
            &[
                ("GIT_CONFIG_GLOBAL", machine.0.join("gitconfig").as_os_str()),
                ("GIT_CONFIG_SYSTEM", machine.0.join("gitconfig").as_os_str()),
                ("GIT_CONFIG_COUNT", std::ffi::OsStr::new("1")),
                ("GIT_CONFIG_KEY_0", std::ffi::OsStr::new("url./elsewhere/effectcraft.git.insteadOf")),
                ("GIT_CONFIG_VALUE_0", std::ffi::OsStr::new("https://github.com/storytold/effectcraft")),
                ("XTASK_UPSTREAM_CACHE_OWN", scratch.0.as_os_str()),
            ],
        );
        assert!(child.status.success(), "{}", String::from_utf8_lossy(&child.stdout));
    }

    #[test]
    fn a_relative_cargo_home_lies_in_rust_where_the_gate_runs_cargo() {
        let rust = Path::new("/repo/rust");
        assert_eq!(cargo_home(Some(".home".into()), Some("/u".into()), rust), Some(PathBuf::from("/repo/rust/.home")));
        assert_eq!(cargo_home(Some("/abs/home".into()), Some("/u".into()), rust), Some(PathBuf::from("/abs/home")));
        assert_eq!(cargo_home(None, Some("/u".into()), rust), Some(PathBuf::from("/u/.cargo")));
        assert_eq!(cargo_home(Some("".into()), None, rust), None);
    }
}
