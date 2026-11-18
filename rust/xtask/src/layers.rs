//! The one definition of the layer table **and** the rules read from it (rule 2).
//!
//! The table lists every crate's kind, layer rank and capabilities once, in dependency order, and a new
//! crate is added here in the same change that adds it. This file is a single table plus the rules that
//! read it, because a rule stated somewhere else is a second definition that drifts: the folder a crate
//! lives in, what it may depend on, whether it may use `uniffi` or an upstream Craft library, whether it
//! may hold `unsafe`, whether it may be built as a native library and whether it may run a build script
//! all come from these rows.
//!
//! Why the rules exist: two Rust static libraries in one app collide on the standard library's symbols,
//! so each bundle an app ships has exactly one native library; the universal core must stay free of any
//! Craft app to be reusable; and a host tool pulled into a shipped library drags the build host into the
//! phone.
//!
//! `cargo metadata` is read by [`crate::meta`]; every rule here is a pure function over [`CrateInput`]
//! values, so a test can feed a synthetic graph instead of the workspace.

use crate::meta::Crate;
use std::path::Path;

/// What a crate is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The universal core: knows nothing about any Craft app.
    Core,
    /// A plugin for one Craft app, named by the app's noun (`effects`, later `film`).
    App(&'static str),
    /// A host tool: may depend on anything, and nothing may depend on it.
    Tool,
    /// A host-only support crate, which the tools share (starting a process and stopping its group, and
    /// running a test of the current test binary again in a child started with a changed environment). A
    /// tool may depend on it; the core's crates, the apps' and a combined distribution crate only from their
    /// tests, since nothing they ship may carry host code; and it depends on no CraftBridge crate and no
    /// Craft app's crate.
    Support,
    /// A combined distribution crate: the one native library of a bundle that ships several apps together
    /// (`craftbridge-all-ffi`), which links their crates into one Rust runtime. It takes the crates of two
    /// apps or more and may take the core's; nothing but a host tool depends on it, so no other library
    /// carries it. Its bundle links it in place of each app's own native library, never beside one, which
    /// the SDK's builds keep by building the one library [`sdk_library`] gives: the rules see crates, not
    /// what a bundle links.
    Distribution,
}

impl Kind {
    /// The app noun of an app plugin's crate, and `None` for every other kind.
    fn app(self) -> Option<&'static str> {
        if let Kind::App(noun) = self { Some(noun) } else { None }
    }

    /// Does a crate of this kind ship, linked into an app through its native library? The core's crates,
    /// the app plugins' and a combined distribution crate do; a host tool and the support crate run only
    /// on the build host. The workspace's default members are exactly the crates that ship
    /// ([`default_member_errors`]), and a crate that ships takes the support crate from its tests alone
    /// ([`check_edge`]).
    fn ships(self) -> bool {
        match self {
            Kind::Core | Kind::App(_) | Kind::Distribution => true,
            Kind::Tool | Kind::Support => false,
        }
    }
}

/// One row of the table.
#[derive(Clone, Copy)]
pub(crate) struct Entry {
    /// The crate name, which is also the folder it lives in.
    pub(crate) name: &'static str,
    pub(crate) kind: Kind,
    /// Layer within its kind: an edge must point at a smaller rank. Tools, support crates and combined
    /// distribution crates are unordered: theirs is 0 and never read.
    pub(crate) rank: u8,
    /// A UniFFI boundary, so it may depend on `uniffi` and allow `unsafe_code` crate-wide.
    pub(crate) ffi: bool,
    /// May hold hand-written `unsafe`, and only under `src/platform/`.
    pub(crate) platform: bool,
    /// May have a build script: a build script runs before the compiler reads any lint table and can hand
    /// the compiler flags and cfgs around it, so only a crate marked here may have one. Nothing is marked:
    /// a crate that ever needs one is a decision of its own.
    pub(crate) build: bool,
    /// The Craft apps' crates a core crate may use. The core knows nothing about any Craft app, with one
    /// exception the plan makes: CraftBridge's decoders register into FilmCraft's decoder registry, the
    /// one EffectCraft decodes through, so the core's media crate reads FilmCraft's codecs and container, and the
    /// frame and colour types its decoders return.
    pub(crate) upstream: &'static [&'static str],
}

impl Entry {
    /// Is this crate a distribution library, the one native library a bundle links, built as a `staticlib`
    /// (iOS) and a `cdylib` (Android)? A bundle of one app links that app's `craftbridge-<app>-ffi`, and a
    /// bundle of several apps their combined distribution crate. The kind and the name answer it, so no
    /// flag on the row says it a second time, where the two could disagree.
    fn distribution_library(&self) -> bool {
        match self.kind {
            Kind::App(noun) => self.name == app_library(noun),
            Kind::Distribution => true,
            Kind::Core | Kind::Tool | Kind::Support => false,
        }
    }
}

/// The start of every crate's name but that of a tool keeping its convention's name (`xtask`).
const PREFIX: &str = "craftbridge-";

/// How the name of a UniFFI boundary ends, a distribution library's among them.
const BOUNDARY: &str = "-ffi";

/// The native library of a bundle that ships the app `noun` alone.
fn app_library(noun: &str) -> String {
    format!("{PREFIX}{noun}{BOUNDARY}")
}

/// The name of the package `package`'s library: what code imports and what the library's file carries.
/// Cargo names a library target after its package, `-` read as `_`, and [`check_crate`] holds every
/// crate's library to that name, so whoever looks for a crate's library by its package's name finds it.
pub(crate) fn library_name(package: &str) -> String {
    package.replace('-', "_")
}

/// The table's own file, which a message about a row names.
const TABLE: &str = "rust/xtask/src/layers.rs";

/// Every crate in the workspace, lowest layer first. Add a crate here in the same change that adds it,
/// or the check fails; a row whose crate is gone fails it too, whether its folder went or the crate in it
/// was renamed.
pub(crate) const ENTRIES: &[Entry] = &[
    Entry {
        name: "craftbridge-media",
        kind: Kind::Core,
        rank: 0,
        ffi: false,
        platform: true,
        build: false,
        upstream: &["filmcraft-codecs", "filmcraft-color", "filmcraft-frame", "filmcraft-isobmff"],
    },
    Entry { name: "craftbridge-core", kind: Kind::Core, rank: 1, ffi: false, platform: true, build: false, upstream: &[] },
    Entry { name: "craftbridge-core-ffi", kind: Kind::Core, rank: 2, ffi: true, platform: true, build: false, upstream: &[] },
    Entry { name: "craftbridge-effects", kind: Kind::App("effects"), rank: 0, ffi: false, platform: false, build: false, upstream: &[] },
    Entry { name: "craftbridge-effects-ffi", kind: Kind::App("effects"), rank: 1, ffi: true, platform: false, build: false, upstream: &[] },
    Entry { name: "craftbridge-effects-codegen", kind: Kind::Tool, rank: 0, ffi: false, platform: false, build: false, upstream: &[] },
    Entry { name: "craftbridge-effects-conformance", kind: Kind::Tool, rank: 0, ffi: false, platform: false, build: false, upstream: &[] },
    Entry { name: "craftbridge-process", kind: Kind::Support, rank: 0, ffi: false, platform: false, build: false, upstream: &[] },
    Entry { name: "xtask", kind: Kind::Tool, rank: 0, ffi: false, platform: false, build: false, upstream: &[] },
    Entry { name: "uniffi-bindgen", kind: Kind::Tool, rank: 0, ffi: false, platform: false, build: false, upstream: &[] },
];

/// A crate's row, or `None` for a crate the table has never heard of.
pub(crate) fn entry(name: &str) -> Option<&'static Entry> {
    row_in(ENTRIES, name)
}

/// A crate's row in `table`: the shipped table, or a test's, which files a planted crate as each kind it
/// might be given.
fn row_in<'table>(table: &'table [Entry], name: &str) -> Option<&'table Entry> {
    table.iter().find(|candidate| candidate.name == name)
}

/// Every app noun a table's rows use, which the table's own checks hold to [`CRAFT_APPS`].
fn app_nouns(table: &[Entry]) -> Vec<&'static str> {
    let mut nouns: Vec<&'static str> = table.iter().filter_map(|row| row.kind.app()).collect();
    nouns.sort_unstable();
    nouns.dedup();
    nouns
}

/// A Craft app: the noun its plugin's crates carry, the prefix of its own crates, and the repository they
/// come from.
pub(crate) struct CraftApp {
    pub(crate) noun: &'static str,
    pub(crate) crates: &'static str,
    pub(crate) repository: &'static str,
}

impl CraftApp {
    /// Is the package `name`, from `source`, one of this app's crates: its prefix, the bare app name
    /// (`effectcraft`) the app's own binary crate carries, or any crate of the app's own repository, whatever
    /// it is called (an example plugin of EffectCraft's, say)?
    pub(crate) fn owns(&self, name: &str, source: Option<&str>) -> bool {
        name.starts_with(self.crates) || name == self.crates.trim_end_matches('-') || source.is_some_and(|source| from_repository(source, self.repository))
    }
}

/// Is `source` (a `cargo metadata` source, `git+https://github.com/storytold/effectcraft?rev=…#…`) the
/// repository named, and not one whose name only starts the same way?
pub(crate) fn from_repository(source: &str, repository: &str) -> bool {
    source.match_indices(repository).any(|(at, _)| {
        let after = source.get(at + repository.len()..).unwrap_or_default();
        after.is_empty() || after.starts_with(['?', '#', '/']) || after.starts_with(".git")
    })
}

/// The Craft apps whose crates can reach this workspace: EffectCraft, which the effects plugin wraps, and
/// FilmCraft, whose codecs EffectCraft decodes through. A plugin uses its own app's crates and no other's.
pub(crate) const CRAFT_APPS: &[CraftApp] = &[
    CraftApp { noun: "effects", crates: "effectcraft-", repository: "github.com/storytold/effectcraft" },
    CraftApp { noun: "film", crates: "filmcraft-", repository: "github.com/storytold/filmcraft" },
];

/// The shared Craft libraries, which belong to no app and serve every plugin, and their repository.
pub(crate) const CRAFT_LIBRARY_PREFIX: &str = "craft-";
pub(crate) const CRAFT_LIBRARY_REPOSITORY: &str = "github.com/storytold/craft-libs";

/// The Craft app whose crate the package `name` from `source` is, if it is one.
fn craft_app_of(name: &str, source: Option<&str>) -> Option<&'static CraftApp> {
    CRAFT_APPS.iter().find(|app| app.owns(name, source))
}

/// The Craft app with this noun, when the table has one: the pin rule names the apps it pins by noun.
pub(crate) fn app_by_noun(noun: &str) -> Option<&'static CraftApp> {
    CRAFT_APPS.iter().find(|app| app.noun == noun)
}

/// Which Craft family a package belongs to: one Craft app's crates, or the shared Craft libraries.
/// The one reading of that question: the duplicate rule (one copy of each) and the pin rule (every copy
/// from its pinned official git source) both ask it, and a second answer to it would drift from the first.
#[derive(Clone, Copy)]
pub(crate) enum CraftFamily {
    App(&'static CraftApp),
    Library,
}

/// The family of the package `name` from `source`, or `None` when it is no Craft app's and no Craft library.
pub(crate) fn family(name: &str, source: Option<&str>) -> Option<CraftFamily> {
    if let Some(app) = craft_app_of(name, source) {
        return Some(CraftFamily::App(app));
    }
    if name.starts_with(CRAFT_LIBRARY_PREFIX) || source.is_some_and(|source| from_repository(source, CRAFT_LIBRARY_REPOSITORY)) {
        return Some(CraftFamily::Library);
    }
    None
}

/// A dependency one package declares.
#[derive(Clone, Debug)]
pub(crate) struct Dependency {
    pub(crate) name: String,
    /// The workspace crate it points at, when it points at one.
    pub(crate) workspace: Option<String>,
    /// A declared path that is no workspace package's folder, as a message shows it: a crate the table
    /// and the lint table never see, linked in anyway.
    pub(crate) outside: Option<String>,
    /// A dev-dependency: built for the crate's own tests only.
    pub(crate) dev: bool,
    /// A build-dependency: built for the crate's build script only, which runs on the build host.
    pub(crate) build: bool,
    /// Where an external dependency comes from, as the manifest declares it.
    pub(crate) source: Option<String>,
}

/// A package reduced to what the layer rules read. A struct rather than a tuple because the types would
/// say nothing at a call site. Owned, so a test can build one from literals and the real check can build
/// one from `cargo metadata` without either side fighting a lifetime.
#[derive(Clone, Debug)]
pub(crate) struct CrateInput {
    pub(crate) name: String,
    /// The crate's folder relative to `rust/`, `/` separated (`core/craftbridge-core`).
    pub(crate) folder: String,
    /// The crate's manifest as a message names it, from the repository root.
    pub(crate) manifest: String,
    pub(crate) dependencies: Vec<Dependency>,
    /// `crate_types` of the package's library targets.
    pub(crate) crate_types: Vec<String>,
    /// The names of the package's library targets, which are what code imports and what a native
    /// library's file is called.
    pub(crate) library_names: Vec<String>,
    /// The names of the package's binary targets, which are the commands it builds.
    pub(crate) binary_names: Vec<String>,
    /// One of the workspace's default members, as Cargo resolves `default-members`.
    pub(crate) default_member: bool,
}

/// The one folder the table says a crate lives in: `<app>/<name>` for an app plugin, `core/<name>` for
/// the core, for a tool `<app>/<name>` when it is an app's tool (`craftbridge-<app>-…`) or `<name>`
/// directly under `rust/` when it is no app's, and `<name>` directly under `rust/` for a support crate,
/// which serves every app's tools alike, and for a combined distribution crate, which ships several apps
/// and so sits under none of them. The folder name is always the crate's name. Which tool is an app's
/// comes from [`CRAFT_APPS`], the list the naming rule reads, so a tool's folder and its name never
/// disagree about its app, whether or not the app has a plugin row yet.
fn expected_folder(row: &Entry) -> String {
    match row.kind {
        Kind::Core => format!("core/{}", row.name),
        Kind::App(app) => format!("{app}/{}", row.name),
        Kind::Support | Kind::Distribution => row.name.to_string(),
        Kind::Tool => match CRAFT_APPS.iter().map(|app| app.noun).find(|app| row.name.starts_with(&format!("{PREFIX}{app}-"))) {
            Some(app) => format!("{app}/{}", row.name),
            None => row.name.to_string(),
        },
    }
}

/// Check one package against `table`: its folder, its name, every edge and its native-library targets.
/// Every message opens with the crate's manifest, the file to open.
fn check_crate(table: &[Entry], input: &CrateInput) -> Vec<String> {
    let at = &input.manifest;
    let Some(row) = row_in(table, &input.name) else {
        return vec![format!("{at}: {} is not in the layer table (rust/xtask/src/layers.rs)", input.name)];
    };
    let mut errors = Vec::new();
    let folder = expected_folder(row);
    if folder != input.folder {
        errors.push(format!(
            "{at}: {} lives in `{}`, the table says `{folder}` (a crate's folder is its name, under its kind's folder)",
            input.name,
            if input.folder.is_empty() { "outside rust/" } else { &input.folder }
        ));
    }
    check_name(row, at, &mut errors);
    // A `[lib] name` other than the one Cargo gives renames what code imports and the native library that
    // ships, past every naming rule above.
    let library = library_name(row.name);
    for name in input.library_names.iter().filter(|name| **name != library) {
        errors.push(format!("{at}: {}'s library is named `{name}`: a crate's library carries the crate's name, `{library}`", row.name));
    }
    // Cargo names a binary target after its package; one of another name is a command the plan and the
    // upgrade steps cannot find by the crate's name.
    for name in input.binary_names.iter().filter(|name| **name != row.name) {
        errors.push(format!("{at}: {}'s binary is named `{name}`: a crate's binary carries the crate's name, `{}`", row.name, row.name));
    }
    for dependency in &input.dependencies {
        if let Some(path) = &dependency.outside {
            errors.push(format!(
                "{at}: {} depends on {} by a path outside the workspace (`{path}`): a crate lives at rust/<kind>/<name> as a workspace member with a row in the layer table",
                row.name, dependency.name
            ));
            continue;
        }
        match &dependency.workspace {
            Some(target_name) => match row_in(table, target_name) {
                Some(target) => check_edge(row, target, dependency.dev, at, &mut errors),
                None => errors.push(format!("{at}: {} depends on {target_name}: {target_name} is not in the layer table (rust/xtask/src/layers.rs)", row.name)),
            },
            None if row_in(table, &dependency.name).is_some() => errors.push(format!(
                "{at}: {} depends on {} from outside the workspace: that is the name of this workspace's own crate, which is taken by its path",
                row.name, dependency.name
            )),
            None => check_external(row, &dependency.name, dependency.source.as_deref(), dependency.dev, at, &mut errors),
        }
    }
    if input.crate_types.iter().any(|kind| kind == "dylib") {
        errors.push(format!("{at}: {} is built as a Rust dylib, which carries its own copy of the Rust runtime: an app links one native library", row.name));
    }
    if builds_native(&input.crate_types) && !row.distribution_library() {
        errors.push(format!(
            "{at}: {} is built as a staticlib or cdylib: only a distribution library may be, an app's craftbridge-<app>-ffi or a combined distribution crate",
            row.name
        ));
    }
    if row.distribution_library() && !(input.crate_types.iter().any(|kind| kind == "staticlib") && input.crate_types.iter().any(|kind| kind == "cdylib")) {
        errors.push(format!(
            "{at}: {} is a distribution library, the one native library its bundle links, so it builds both a staticlib (iOS) and a cdylib (Android): crate-type = [\"staticlib\", \"cdylib\", \"lib\"]",
            row.name
        ));
    }
    if row.kind == Kind::Distribution {
        check_shipped_apps(table, row, input, &mut errors);
    }
    errors
}

/// A combined distribution crate ships two apps or more together, and the apps it ships are those whose
/// crates it takes: one that took a single app's crates would be a second native library of that app,
/// whose bundle already links its own `craftbridge-<app>-ffi`. A dev-dependency and a build-dependency ship
/// nothing — the one is built for the crate's tests, the other for a build script, which runs on the build
/// host — so neither is counted.
fn check_shipped_apps(table: &[Entry], row: &Entry, input: &CrateInput, errors: &mut Vec<String>) {
    let mut apps: Vec<&str> = input
        .dependencies
        .iter()
        .filter(|dependency| !dependency.dev && !dependency.build)
        .filter_map(|dependency| dependency.workspace.as_deref().and_then(|target| row_in(table, target)))
        .filter_map(|target| target.kind.app())
        .collect();
    apps.sort_unstable();
    apps.dedup();
    let taken = match apps.as_slice() {
        [] => "no app's crates".to_string(),
        [noun] => format!("only the {noun} app's crates, whose bundle links {} alone", app_library(noun)),
        _ => return,
    };
    errors.push(format!("{}: {} takes {taken}: a combined distribution crate ships two apps or more together", input.manifest, row.name));
}

/// The segments of a crate's name, or of a part of one. Cargo reads `-` and `_` in a package name alike —
/// `craftbridge-film_utils` builds the library `craftbridge_film_utils` — so a segment ends at either.
fn segments(name: &str) -> std::str::Split<'_, [char; 2]> {
    name.split(['-', '_'])
}

/// The Craft app a crate's name names, as one of its [`segments`]: [`CRAFT_APPS`]' noun. A noun in another
/// case names the app as plainly.
fn named_app(name: &str) -> Option<&'static str> {
    CRAFT_APPS.iter().map(|app| app.noun).find(|noun| segments(name).any(|segment| segment.eq_ignore_ascii_case(noun)))
}

/// A core crate is `craftbridge-<area>`, and its name carries no app's noun: the core serves every app,
/// so a core name that names one app mislabels the shelf it sits on. An app plugin carries its app's
/// noun, which stops one app's crate being mistaken for another's, and so does an app's tool. Any other
/// tool keeps the name its convention gives it (`xtask`, `uniffi-bindgen`). A combined distribution crate
/// is `craftbridge-<bundle>-ffi`, the UniFFI boundary its bundle's bindings are generated from, and names
/// no app: it ships several, and a name carrying one app's noun would read as that app's crate. `at` is
/// how a message names the crate's manifest.
fn check_name(row: &Entry, at: &str, errors: &mut Vec<String>) {
    match row.kind {
        Kind::Core => {
            if !row.name.starts_with(PREFIX) {
                errors.push(format!("{at}: {}: a core crate is named craftbridge-<area>", row.name));
            }
            if let Some(noun) = named_app(row.name) {
                errors.push(format!("{at}: {}: a core crate names no app: {noun} is one, and the core serves them all", row.name));
            }
        }
        Kind::Support => {
            if !row.name.starts_with(PREFIX) {
                errors.push(format!("{at}: {}: a support crate is named craftbridge-<job>", row.name));
            }
            if let Some(noun) = named_app(row.name) {
                errors.push(format!("{at}: {}: a support crate names no app: {noun} is one, and every app's tools share it", row.name));
            }
        }
        Kind::App(app) => {
            let prefixed = format!("{PREFIX}{app}-");
            if row.name != format!("{PREFIX}{app}") && !row.name.starts_with(&prefixed) {
                errors.push(format!("{at}: {}: {} plugin crate is named craftbridge-{app} or craftbridge-{app}-…", row.name, with_article(app)));
            }
        }
        Kind::Tool => {
            if let Some(noun) = named_app(row.name).filter(|noun| !row.name.starts_with(&format!("{PREFIX}{noun}-"))) {
                errors.push(format!("{at}: {}: a tool of the {noun} app is named craftbridge-{noun}-…, like the app's other crates", row.name));
            }
        }
        Kind::Distribution => {
            // The bundle is one segment or more, none of them empty: Cargo accepts a bundle of separators
            // alone (`craftbridge---ffi`), which names none, and one with a doubled separator.
            let bundle = row.name.strip_prefix(PREFIX).and_then(|rest| rest.strip_suffix(BOUNDARY));
            if !bundle.is_some_and(|bundle| segments(bundle).all(|segment| !segment.is_empty())) {
                errors.push(format!("{at}: {}: a combined distribution crate is named craftbridge-<bundle>-ffi", row.name));
            }
            if let Some(noun) = named_app(row.name) {
                errors.push(format!("{at}: {}: a combined distribution crate names no app: {noun} is one, and it ships several", row.name));
            }
        }
    }
}

/// `a` or `an` before an app noun, as a sentence says it.
fn with_article(noun: &str) -> String {
    let article = if noun.starts_with(['a', 'e', 'i', 'o', 'u']) { "an" } else { "a" };
    format!("{article} {noun}")
}

/// One edge between two workspace crates. A dev-dependency is held to the isolation rules — the core
/// knows no app plugin, and a plugin knows no other app's — and to the tool and distribution rules,
/// because a crate's tests are its code, and a tool may reach every app, as a combined distribution crate
/// links every app it ships: a test that took either would reach them all through it. Only the rank rule
/// and the rule that a shipped crate takes the support crate from its tests alone are about what ships,
/// and a test build ships nothing. A combined distribution crate may take the core's crates and every
/// app's, in any rank.
fn check_edge(row: &Entry, target: &Entry, dev: bool, at: &str, errors: &mut Vec<String>) {
    match (&row.kind, &target.kind) {
        // A tool may depend on anything; nothing may depend on a tool.
        (Kind::Tool, _) => {}
        (Kind::Support, _) => {
            errors.push(edge(at, row, target, "a support crate depends on no CraftBridge crate: every tool shares it, so it stays below them all"))
        }
        (Kind::Core, Kind::App(app)) => {
            errors.push(edge(at, row, target, format!("the universal core never depends on the {app} plugin, not even in its tests")))
        }
        (Kind::App(other), Kind::App(app)) if other != app => {
            errors.push(edge(at, row, target, format!("an app plugin never depends on another app's plugin ({other} on {app}), not even in its tests")))
        }
        (_, Kind::Tool) => errors.push(edge(at, row, target, "nothing depends on a host tool, not even a crate's tests: a tool may reach every app")),
        (_, Kind::Distribution) => errors.push(edge(
            at,
            row,
            target,
            "nothing but a host tool depends on a combined distribution crate, not even a crate's tests: it links every app it ships, so whatever took it would carry them all",
        )),
        (_, Kind::Support) if row.kind.ships() && !dev => {
            errors.push(edge(at, row, target, "a support crate is host code: a shipped crate takes it only from its tests (a dev-dependency)"));
        }
        _ if dev => {}
        // Within one kind, and only within it, ranks order the edges; the same rank is a cycle waiting
        // to happen, so equality is not allowed either.
        (Kind::Core, Kind::Core) | (Kind::App(_), Kind::App(_)) if target.rank >= row.rank => {
            errors.push(edge(at, row, target, format!("layers point down: {} is rank {} and {} is rank {}", row.name, row.rank, target.name, target.rank)));
        }
        _ => {}
    }
}

fn edge(at: &str, row: &Entry, target: &Entry, reason: impl AsRef<str>) -> String {
    format!("{at}: {} depends on {}: {}", row.name, target.name, reason.as_ref())
}

/// An external crate: `uniffi` only at a UniFFI boundary; no Craft app's crates in the universal core
/// beyond the row's `upstream`; in an app plugin, its own Craft app's crates and no sibling's; and none
/// in a combined distribution crate, which reaches each app through its plugin and holds no app's code.
/// A Craft app's crate is known by its name or by the app's repository it comes from. A tool may depend
/// on anything. A dev-dependency is held to the Craft-app rules only, as in [`check_edge`].
fn check_external(row: &Entry, name: &str, source: Option<&str>, dev: bool, at: &str, errors: &mut Vec<String>) {
    if row.kind == Kind::Tool {
        return;
    }
    if name.starts_with("uniffi") && !row.ffi && !dev {
        errors.push(format!("{at}: {} depends on {name}: uniffi appears only in -ffi crates (rust/xtask/src/layers.rs)", row.name));
    }
    let Some(app) = craft_app_of(name, source) else { return };
    match row.kind {
        Kind::Core if !row.upstream.contains(&name) => {
            errors.push(format!("{at}: {} depends on {name}: the universal core knows nothing about any Craft app", row.name));
        }
        Kind::App(noun) if noun != app.noun => {
            errors.push(format!(
                "{at}: {} depends on {name}: an app plugin uses only its own Craft app's crates; code two plugins need moves into the core",
                row.name
            ));
        }
        Kind::Support => errors.push(format!("{at}: {} depends on {name}: a support crate knows nothing about any Craft app", row.name)),
        Kind::Distribution => errors.push(format!(
            "{at}: {} depends on {name}: a combined distribution crate takes a Craft app only through that app's plugin, and holds no app's code",
            row.name
        )),
        _ => {}
    }
}

/// Does a library target build a native library an app links, rather than a Rust-only `lib`?
fn builds_native(crate_types: &[String]) -> bool {
    crate_types.iter().any(|kind| kind == "staticlib" || kind == "cdylib")
}

/// The rules the table's own rows must keep, so a row added by hand cannot quietly break one: a crate has
/// one row, since a lookup reads the first and a second would hold rules nobody reads; every app has its
/// native library, its own row named `craftbridge-<app>-ffi`, which a bundle of that app alone links (two
/// Rust static libraries in one app collide); a row is a UniFFI boundary exactly when its name ends in
/// `-ffi`; an app noun names a known Craft app; and only a core crate lists Craft app crates it may use.
fn table_errors(table: &[Entry]) -> Vec<String> {
    let mut errors = Vec::new();
    let at = TABLE;
    for (index, row) in table.iter().enumerate() {
        if table.iter().take(index).any(|earlier| earlier.name == row.name) {
            errors.push(format!("{at}: {}: a crate has one row in the table, and the rules read only the first", row.name));
        }
        if row.ffi != row.name.ends_with(BOUNDARY) {
            errors.push(format!("{at}: {}: a UniFFI boundary, and only one, is named -ffi", row.name));
        }
        if !row.upstream.is_empty() && row.kind != Kind::Core {
            errors.push(format!("{at}: {}: only a core crate lists the Craft app crates it may use; a plugin uses its own app's", row.name));
        }
    }
    for noun in app_nouns(table) {
        if !CRAFT_APPS.iter().any(|app| app.noun == noun) {
            errors.push(format!("{at}: {noun}: not a Craft app this table knows (CRAFT_APPS)"));
        }
        if own_library(table, noun).is_none() {
            errors.push(no_library(noun));
        }
    }
    errors
}

/// The app `noun`'s own native library in `table`: its row named `craftbridge-<noun>-ffi`.
fn own_library<'table>(table: &'table [Entry], noun: &str) -> Option<&'table Entry> {
    table.iter().find(|row| row.kind.app() == Some(noun) && row.distribution_library())
}

/// What the table says of an app with no native library of its own.
fn no_library(noun: &str) -> String {
    format!("{TABLE}: the {noun} plugin has no row {}: an app has exactly one native library, which a bundle of that app alone links", app_library(noun))
}

/// The distribution library the SDK ships, the one native library an app that uses its packages links:
/// `cargo xtask generate` writes the bindings from it, `cargo xtask xcframework` builds it, and
/// `cargo xtask sdk-library` names it to `tools/build-android.sh`, which builds it. The SDK's
/// packages carry every plugin the table has behind one native library, so it is the one app's own
/// `craftbridge-<app>-ffi` while the table has one app, and the combined distribution crate that ships
/// them together once it has several. The table answers it, so no constant names the library again.
pub(crate) fn sdk_library() -> Result<&'static str, String> {
    sdk_library_of(ENTRIES)
}

/// [`sdk_library`], read from the rows of `table`.
fn sdk_library_of(table: &[Entry]) -> Result<&'static str, String> {
    let combined: Vec<&'static str> = table.iter().filter(|row| row.kind == Kind::Distribution).map(|row| row.name).collect();
    match (app_nouns(table).as_slice(), combined.as_slice()) {
        ([], _) => Err(format!("{TABLE}: the table has no app plugin, so the SDK has no native library to ship")),
        // One app ships in its own library: a combined distribution crate beside it could take only that
        // app's crates, which the crate's own rule refuses.
        ([noun], _) => own_library(table, noun).map(|row| row.name).ok_or_else(|| no_library(noun)),
        (_, [name]) => Ok(*name),
        (nouns, []) => Err(format!(
            "{TABLE}: the table has the {} plugins and no combined distribution crate, the one native library the SDK ships them all in",
            nouns.join(" and ")
        )),
        (_, names) => {
            Err(format!("{TABLE}: the SDK ships one native library, and the table has several combined distribution crates: {}", names.join(" and ")))
        }
    }
}

/// The library [`sdk_library_of`] gives, as the `key=value` lines `cargo xtask sdk-library` prints for a
/// build script to read as data: `package`, the package Cargo builds, and `library`, the name of its library,
/// which the file every build of it writes carries. A script that named either itself would say a second
/// time which crate is the distribution library.
fn sdk_library_lines(table: &[Entry]) -> Result<Vec<String>, String> {
    let package = sdk_library_of(table)?;
    Ok(vec![format!("package={package}"), format!("library={}", library_name(package))])
}

/// `cargo xtask sdk-library`: the distribution library the SDK ships, for `tools/build-android.sh`.
pub(crate) fn run_sdk_library() -> std::process::ExitCode {
    crate::report::print_lines(sdk_library_lines(ENTRIES), "sdk-library")
}

/// A table row whose folder is not on disk, or whose crate no workspace package is any more, so a crate
/// renamed in place or deleted cannot leave a dead row.
fn missing_crates(table: &[Entry], rust_dir: &Path, packages: &[&str]) -> Vec<String> {
    let mut errors = Vec::new();
    for row in table {
        let folder = expected_folder(row);
        if !rust_dir.join(&folder).is_dir() {
            errors.push(format!("{TABLE}: {}: the table says `rust/{folder}`, but there is no such folder", row.name));
        } else if !packages.contains(&row.name) {
            errors.push(format!("{TABLE}: {}: the table has a row for it, but no workspace package is called that any more", row.name));
        }
    }
    errors
}

/// The workspace's default members, what Cargo builds when no package is named, against the rows that
/// ship ([`Kind::ships`]). The gate's phone-target runs, which alone compile what only a phone builds,
/// name no package, so Cargo builds the default members with their tests and every feature, and of any
/// other member only what a default member takes, with the features its takers turn on, which clippy's and
/// the lint library's drivers read as the workspace's own code. A crate that ships but is left out of the
/// list then has its tests and its other features read for no phone, and all of it when no default member
/// takes it, as none takes a combined distribution crate; and a crate in the list that ships nothing is
/// host code built for a phone it never runs on. Cargo reads the list only from the workspace manifest,
/// which is written by hand but for what `tools/update-upstream.sh` copies from the pinned EffectCraft
/// when the pin moves, so the rules hold the list to the table rather than a second writer keeping it.
/// `at` names that manifest.
fn default_member_errors(table: &[Entry], inputs: &[CrateInput], at: &str) -> Vec<String> {
    let mut errors = Vec::new();
    for input in inputs {
        let ships = row_in(table, &input.name).is_some_and(|row| row.kind.ships());
        if ships && !input.default_member {
            errors.push(format!(
                "{at}: default-members leaves out {}, which the layer table ({TABLE}) says ships: the phone targets' lint runs read a crate in full, its tests and every feature, only when it is a default member, and not at all when no default member takes it",
                input.name
            ));
        } else if !ships && input.default_member {
            errors.push(format!(
                "{at}: default-members holds {}, which the layer table ({TABLE}) does not say ships: the default members are exactly the crates an app links, which the phone targets' runs build",
                input.name
            ));
        }
    }
    errors
}

/// A crate folder whose name differs from the table's only in case. A case-insensitive disk lets Cargo
/// find `rust/Core/…` through the members glob `core/*`, so the build passes here and the folder looks
/// right to every name the rules compare; a case-sensitive checkout then finds no crate at all. The
/// listing of each parent folder is what the disk really holds.
fn folder_case_errors(rust_dir: &Path, inputs: &[CrateInput]) -> Vec<String> {
    let mut errors = Vec::new();
    for input in inputs.iter().filter(|input| !input.folder.is_empty()) {
        let mut dir = rust_dir.to_path_buf();
        for part in input.folder.split('/') {
            let listed: Vec<String> =
                std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
            if !listed.iter().any(|name| name == part) {
                if let Some(stored) = listed.iter().find(|name| name.eq_ignore_ascii_case(part)) {
                    errors.push(format!("{}: the folder `{part}` is stored as `{stored}`: a case-sensitive checkout would find no crate here", input.manifest));
                }
                break;
            }
            dir = dir.join(part);
        }
    }
    errors
}

/// A manifest's folder relative to `rust/`, `/` separated, or `None` outside it. `rust_dir` is tried as
/// given and as the disk resolves it, as `report::shown` does, so a run through a symbolic link to the
/// repository still finds every crate inside it.
fn manifest_folder(rust_dir: &Path, manifest: &str) -> Option<String> {
    let parent = Path::new(manifest).parent()?;
    let canonical = std::fs::canonicalize(rust_dir).ok();
    let folder = parent.strip_prefix(rust_dir).ok().or_else(|| canonical.as_deref().and_then(|resolved| parent.strip_prefix(resolved).ok()))?;
    Some(folder.to_string_lossy().replace('\\', "/"))
}

/// Reduce `cargo metadata` to the rules' input. A dependency belongs to the workspace when its declared
/// path is the folder of a workspace package's manifest; a dependency with a path anywhere else is a
/// crate no row and no lint table governs, and is refused; one without a path is external and is judged
/// by its name. `root` is the repository root, which messages name files from.
fn inputs_from(crates: &[Crate], rust_dir: &Path, root: &Path) -> Vec<CrateInput> {
    let owner_at = |path: &str| {
        crates.iter().find(|candidate| Path::new(&candidate.manifest_path).parent() == Some(Path::new(path))).map(|candidate| candidate.name.clone())
    };
    crates
        .iter()
        .map(|package| CrateInput {
            name: package.name.clone(),
            folder: manifest_folder(rust_dir, &package.manifest_path).unwrap_or_default(),
            manifest: crate::report::shown(root, Path::new(&package.manifest_path)),
            dependencies: package
                .dependencies
                .iter()
                .map(|dependency| {
                    let workspace = dependency.path.as_deref().and_then(owner_at);
                    let outside = dependency.path.as_deref().filter(|_| workspace.is_none()).map(|path| crate::report::shown(root, Path::new(path)));
                    Dependency {
                        name: dependency.name.clone(),
                        workspace,
                        outside,
                        dev: dependency.dev,
                        build: dependency.build,
                        source: dependency.source.clone(),
                    }
                })
                .collect(),
            crate_types: package.library_targets.iter().flat_map(|target| target.crate_types.iter().cloned()).collect(),
            library_names: package.library_targets.iter().map(|target| target.name.clone()).collect(),
            binary_names: package.binary_targets.clone(),
            default_member: package.default_member,
        })
        .collect()
}

/// Every local package the resolved graph holds that is no workspace member, named with the packages that
/// ask for it: a path dependency, a `[patch]` or a `[replace]` that points at a folder brings code in that
/// no table row, no lint table and no rule reads, however the manifest spells it.
fn local_packages_outside(resolved: &[Crate], root: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    for package in resolved.iter().filter(|package| package.source.is_none() && !package.member) {
        let askers: Vec<String> = resolved
            .iter()
            .filter(|asker| asker.depends_on.contains(&package.id))
            .map(|asker| if asker.member { crate::report::shown(root, Path::new(&asker.manifest_path)) } else { asker.name.clone() })
            .collect();
        errors.push(format!(
            "{}: {} is a local package outside the workspace, asked for by {}: a crate lives at rust/<kind>/<name> as a workspace member with a row in the layer table",
            crate::report::shown(root, Path::new(&package.manifest_path)),
            package.name,
            if askers.is_empty() { "nothing".to_string() } else { askers.join(", ") }
        ));
    }
    errors
}

/// Every `Cargo.toml` under `rust/` that is neither the workspace's nor a member's: a package no member
/// glob covers and nothing depends on is compiled by no stage and read by no rule, so it cannot sit there.
fn stray_manifests(rust_dir: &Path, root: &Path, members: &[Crate]) -> Result<Vec<String>, String> {
    let known: Vec<std::path::PathBuf> = std::iter::once(rust_dir.join("Cargo.toml"))
        .chain(members.iter().map(|member| Path::new(&member.manifest_path).to_path_buf()))
        .filter_map(|path| std::fs::canonicalize(path).ok())
        .collect();
    Ok(crate::scan::walk(rust_dir, "Cargo.toml")?
        .into_iter()
        .filter(|file| file.path.file_name().is_some_and(|name| name == "Cargo.toml"))
        .filter(|file| std::fs::canonicalize(&file.path).is_ok_and(|path| !known.contains(&path)))
        // The gate's lint library is the one package under `rust/` outside the workspace: `cargo dylint`
        // builds it in the stages that run it, and the rules read its sources (AGENTS.md §3, §10). Its
        // manifest alone is named, so any other package off the workspace graph is still refused.
        .filter(|file| file.path != root.join(crate::machinery::LINT_LIBRARY).join("Cargo.toml"))
        .map(|file| {
            format!("{}: a Cargo package that is no workspace member: no stage compiles it and no rule reads it", crate::report::shown(root, &file.path))
        })
        .collect())
}

/// Run the check against the workspace on disk: read the graph, apply every rule, and make sure the
/// table still matches the crates that exist.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    check_against(root, ENTRIES)
}

/// [`check`] with the rows of `table`, so a test can read a crate planted in a copy of the workspace as
/// each kind it might be filed as.
fn check_against(root: &Path, table: &[Entry]) -> Result<Vec<String>, String> {
    let rust_dir = root.join("rust");
    let crates = crate::meta::read(&rust_dir, crate::meta::Graph::Workspace)?;
    let mut errors = table_errors(table);
    errors.extend(local_packages_outside(&crate::meta::read(&rust_dir, crate::meta::Graph::Resolved)?, root));
    // The gate's lint library is a workspace of its own the gate compiles: a local package its graph holds,
    // a workspace crate by path included, would bring code in no rule reads as its, the same way.
    let library_dir = root.join(crate::machinery::LINT_LIBRARY);
    if library_dir.join("Cargo.toml").is_file() {
        errors.extend(local_packages_outside(&crate::meta::read(&library_dir, crate::meta::Graph::Resolved)?, root));
    }
    errors.extend(stray_manifests(&rust_dir, root, &crates)?);
    let inputs = inputs_from(&crates, &rust_dir, root);
    for input in &inputs {
        errors.extend(check_crate(table, input));
    }
    errors.extend(folder_case_errors(&rust_dir, &inputs));
    errors.extend(default_member_errors(table, &inputs, &crate::report::shown(root, &rust_dir.join("Cargo.toml"))));
    // A crate in the workspace but not in the table is reported by check_crate; a table row with no
    // crate is reported here.
    let names: Vec<&str> = crates.iter().map(|package| package.name.as_str()).collect();
    errors.extend(missing_crates(table, &rust_dir, &names));
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{CrateDependency, CrateTarget};
    use crate::scratch::Scratch;

    fn workspace(name: &str) -> Dependency {
        Dependency { name: name.to_string(), workspace: Some(name.to_string()), outside: None, dev: false, build: false, source: None }
    }

    fn external(name: &str) -> Dependency {
        Dependency { name: name.to_string(), workspace: None, outside: None, dev: false, build: false, source: None }
    }

    fn dev(dependency: Dependency) -> Dependency {
        Dependency { dev: true, ..dependency }
    }

    fn build(dependency: Dependency) -> Dependency {
        Dependency { build: true, ..dependency }
    }

    fn input(name: &str, folder: &str, dependencies: Vec<Dependency>, crate_types: Vec<&str>) -> CrateInput {
        CrateInput {
            name: name.to_string(),
            folder: folder.to_string(),
            manifest: format!("rust/{folder}/Cargo.toml"),
            dependencies,
            crate_types: crate_types.iter().map(|kind| kind.to_string()).collect(),
            library_names: if crate_types.is_empty() { Vec::new() } else { vec![name.replace('-', "_")] },
            binary_names: Vec::new(),
            default_member: false,
        }
    }

    /// The crate types each row is built as when it is compliant: the distribution library builds both
    /// native kinds, every other crate a plain library.
    fn compliant_types(row: &Entry) -> Vec<&'static str> {
        if row.distribution_library() { vec!["staticlib", "cdylib", "lib"] } else { vec!["lib"] }
    }

    /// The table as its rows must satisfy it with no edges at all: this is what a fresh skeleton must pass.
    #[test]
    fn every_table_row_satisfies_its_own_rules() {
        assert!(table_errors(ENTRIES).is_empty(), "{:?}", table_errors(ENTRIES));
        for row in ENTRIES {
            let errors = check_crate(ENTRIES, &input(row.name, &expected_folder(row), Vec::new(), compliant_types(row)));
            assert!(errors.is_empty(), "{}: {errors:?}", row.name);
        }
    }

    /// Every table row has its folder and its crate on disk.
    #[test]
    fn the_table_matches_the_workspace_on_disk() {
        let names: Vec<&str> = ENTRIES.iter().map(|row| row.name).collect();
        let errors = missing_crates(ENTRIES, &crate::repo_root().join("rust"), &names);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn an_unlisted_crate_is_reported_with_its_manifest() {
        let errors = check_crate(ENTRIES, &input("craftbridge-mystery", "core/craftbridge-mystery", Vec::new(), Vec::new()));
        assert_eq!(errors, ["rust/core/craftbridge-mystery/Cargo.toml: craftbridge-mystery is not in the layer table (rust/xtask/src/layers.rs)"]);
    }

    #[test]
    fn a_table_row_without_a_folder_or_a_crate_is_reported() {
        // A folder tree missing one row's folder reports that row, and only it.
        let scratch = Scratch::new("complete");
        for row in ENTRIES.iter().skip(1) {
            scratch.mkdir(&expected_folder(row));
        }
        let names: Vec<&str> = ENTRIES.iter().map(|row| row.name).collect();
        let errors = missing_crates(ENTRIES, &scratch.0, &names);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains(ENTRIES[0].name) && errors[0].contains("no such folder"), "{errors:?}");
        // And a folder tree holding every row finds nothing, which is the compliant case.
        scratch.mkdir(&expected_folder(&ENTRIES[0]));
        assert!(missing_crates(ENTRIES, &scratch.0, &names).is_empty());
        // A crate renamed in place keeps its folder, and its row is dead all the same.
        let renamed: Vec<&str> = names.iter().copied().filter(|name| *name != "craftbridge-core").collect();
        let errors = missing_crates(ENTRIES, &scratch.0, &renamed);
        assert!(errors.len() == 1 && errors[0].contains("craftbridge-core: the table has a row for it"), "{errors:?}");
    }

    #[test]
    fn a_crate_in_the_wrong_folder_is_reported() {
        let errors = check_crate(ENTRIES, &input("craftbridge-core", "effects/craftbridge-core", Vec::new(), Vec::new()));
        assert!(errors.iter().any(|e| e.contains("`core/craftbridge-core`")), "{errors:?}");
        // An app's tool lives under its app, and only there; a tool that is no app's sits at the top.
        assert!(check_crate(ENTRIES, &input("craftbridge-effects-codegen", "effects/craftbridge-effects-codegen", Vec::new(), Vec::new())).is_empty());
        let top = check_crate(ENTRIES, &input("craftbridge-effects-codegen", "craftbridge-effects-codegen", Vec::new(), Vec::new()));
        assert!(top.iter().any(|e| e.contains("the table says `effects/craftbridge-effects-codegen`")), "{top:?}");
        assert!(!check_crate(ENTRIES, &input("xtask", "core/xtask", Vec::new(), Vec::new())).is_empty());
        // A plugin crate in the core's folder is in the wrong kind's folder.
        let plugin_in_core = check_crate(ENTRIES, &input("craftbridge-effects", "core/craftbridge-effects", Vec::new(), Vec::new()));
        assert!(plugin_in_core.iter().any(|e| e.contains("the table says `effects/craftbridge-effects`")), "{plugin_in_core:?}");
        // A tool of an app with no plugin row yet still lives under that app, where its name puts it.
        let film_tool = Entry { name: "craftbridge-film-codegen", kind: Kind::Tool, rank: 0, ffi: false, platform: false, build: false, upstream: &[] };
        assert_eq!(expected_folder(&film_tool), "film/craftbridge-film-codegen");
        let mut naming = Vec::new();
        check_name(&film_tool, "x", &mut naming);
        assert!(naming.is_empty(), "the naming rule reads the same app: {naming:?}");
    }

    #[test]
    fn a_binary_target_of_another_name_is_reported() {
        let mut renamed = input("craftbridge-effects-codegen", "effects/craftbridge-effects-codegen", Vec::new(), Vec::new());
        renamed.binary_names = vec!["film-codegen".into()];
        let errors = check_crate(ENTRIES, &renamed);
        assert!(errors.len() == 1 && errors[0].contains("binary is named `film-codegen`"), "{errors:?}");
        renamed.binary_names = vec!["craftbridge-effects-codegen".into()];
        assert!(check_crate(ENTRIES, &renamed).is_empty());
    }

    #[test]
    fn a_craft_app_s_crate_is_known_by_its_repository_whatever_its_name() {
        let from_effectcraft =
            Dependency { source: Some("git+https://github.com/storytold/effectcraft?rev=1037605#1037605".into()), ..external("posterize-bands") };
        let core = check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", vec![from_effectcraft.clone()], Vec::new()));
        assert!(core.iter().any(|e| e.contains("knows nothing about any Craft app")), "{core:?}");
        assert!(check_crate(ENTRIES, &input("craftbridge-effects", "effects/craftbridge-effects", vec![from_effectcraft], Vec::new())).is_empty());
        assert!(from_repository("git+https://github.com/storytold/effectcraft.git?rev=1#1", "github.com/storytold/effectcraft"));
        assert!(!from_repository("git+https://github.com/storytold/effectcraft-fork?rev=1#1", "github.com/storytold/effectcraft"));
    }

    #[test]
    fn a_local_package_outside_the_workspace_is_refused_naming_who_asks_for_it() {
        let package = |name: &str, id: &str, source: Option<&str>, member: bool, depends_on: &[&str]| Crate {
            name: name.into(),
            id: id.into(),
            source: source.map(str::to_string),
            edition: "2024".into(),
            target_editions: Vec::new(),
            manifest_path: format!("/repo/{}/Cargo.toml", if member { format!("rust/core/{name}") } else { format!("tools/{name}") }),
            dependencies: Vec::new(),
            library_targets: Vec::new(),
            build_script: None,
            depends_on: depends_on.iter().map(|id| id.to_string()).collect(),
            member,
            default_member: false,
            binary_targets: Vec::new(),
        };
        let resolved = [
            package("craftbridge-core", "core#1", None, true, &["helper#1", "serde#1"]),
            package("craftbridge-effects-helper", "helper#1", None, false, &[]),
            package("serde", "serde#1", Some("registry+https://github.com/rust-lang/crates.io-index"), false, &[]),
        ];
        let errors = local_packages_outside(&resolved, Path::new("/repo"));
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("tools/craftbridge-effects-helper/Cargo.toml: craftbridge-effects-helper is a local package outside the workspace, asked for by rust/core/craftbridge-core/Cargo.toml"), "{errors:?}");
    }

    #[test]
    fn a_cargo_package_under_rust_that_no_member_covers_is_refused() {
        let copy = crate::scratch::rust_tree_copy("layers-stray");
        copy.write("rust/misc/stray/Cargo.toml", "[package]\nname = \"film-stray\"\nversion = \"0.1.0\"\nedition = \"2024\"\n");
        copy.write("rust/misc/stray/src/lib.rs", "\n");
        // The copy carries the lint library, the one package outside the workspace the rule exempts.
        assert!(copy.0.join(crate::machinery::LINT_LIBRARY).join("Cargo.toml").is_file(), "the copy holds the lint library");
        let errors = check(&copy.0).unwrap();
        assert_eq!(errors, ["rust/misc/stray/Cargo.toml: a Cargo package that is no workspace member: no stage compiles it and no rule reads it"]);
    }

    /// The gate's lint library is a workspace of its own, and a local package its graph holds — a
    /// workspace crate by path, here — is refused as one in the workspace's graph is.
    #[test]
    fn a_local_package_in_the_lint_library_s_graph_is_refused() {
        let copy = crate::scratch::rust_tree_copy("layers-library-path");
        let manifest = copy.0.join(crate::machinery::LINT_LIBRARY).join("Cargo.toml");
        let original = std::fs::read_to_string(&manifest).unwrap();
        let with_path = original.replacen("[lib]", "[dependencies]\ncraftbridge-core = { path = \"../core/craftbridge-core\" }\n\n[lib]", 1);
        std::fs::write(&manifest, with_path).unwrap();
        relock(manifest.parent().unwrap());
        let errors = check(&copy.0).unwrap();
        assert!(
            errors.iter().any(|e| e.starts_with("rust/core/craftbridge-core/Cargo.toml: craftbridge-core is a local package outside the workspace, asked for by rust/never-crash-lints/Cargo.toml")),
            "{errors:?}"
        );
    }

    #[test]
    fn a_folder_of_the_right_kind_with_another_name_is_reported() {
        let renamed = check_crate(ENTRIES, &input("craftbridge-media", "core/media", Vec::new(), Vec::new()));
        assert!(renamed.iter().any(|e| e.contains("lives in `core/media`")), "{renamed:?}");
        // Two crates that swapped folders are each in a folder that is not their name.
        let codegen = check_crate(ENTRIES, &input("craftbridge-effects-codegen", "effects/craftbridge-effects-conformance", Vec::new(), Vec::new()));
        let conformance = check_crate(ENTRIES, &input("craftbridge-effects-conformance", "effects/craftbridge-effects-codegen", Vec::new(), Vec::new()));
        assert!(codegen.iter().any(|e| e.contains("lives in")) && conformance.iter().any(|e| e.contains("lives in")), "{codegen:?} {conformance:?}");
        let outside = check_crate(ENTRIES, &input("craftbridge-core", "", Vec::new(), Vec::new()));
        assert!(outside.iter().any(|e| e.contains("lives in `outside rust/`")), "{outside:?}");
    }

    #[test]
    fn a_folder_stored_in_another_case_is_reported() {
        let scratch = Scratch::new("case-folder");
        scratch.mkdir("Core/craftbridge-core");
        let errors = folder_case_errors(&scratch.0, &[input("craftbridge-core", "core/craftbridge-core", Vec::new(), Vec::new())]);
        assert!(errors.len() == 1 && errors[0].contains("`core` is stored as `Core`"), "{errors:?}");
        scratch.mkdir("core-other");
        let scratch = Scratch::new("case-folder-ok");
        scratch.mkdir("core/craftbridge-core");
        assert!(folder_case_errors(&scratch.0, &[input("craftbridge-core", "core/craftbridge-core", Vec::new(), Vec::new())]).is_empty());
    }

    #[test]
    fn an_app_crate_must_carry_its_app_noun() {
        let effects = entry("craftbridge-effects").unwrap();
        assert!(check_name_of(effects, "craftbridge-effects").is_empty() && check_name_of(effects, "craftbridge-effects-ffi").is_empty());
        let other = check_name_of(effects, "craftbridge-other");
        assert!(other.iter().any(|e| e.contains("an effects plugin crate is named")), "{other:?}");
    }

    #[test]
    fn an_app_s_tool_carries_the_app_prefix_and_other_tools_their_own_names() {
        let tool = entry("xtask").unwrap();
        assert!(check_name_of(tool, "xtask").is_empty() && check_name_of(tool, "uniffi-bindgen").is_empty());
        assert!(check_name_of(tool, "craftbridge-film-codegen").is_empty());
        for misnamed in ["film-codegen", "codegen-effects"] {
            assert!(check_name_of(tool, misnamed).iter().any(|e| e.contains("a tool of the")), "{misnamed}");
        }
    }

    #[test]
    fn a_core_crate_must_carry_the_craftbridge_prefix() {
        let core = entry("craftbridge-core").unwrap();
        assert!(check_name_of(core, "craftbridge-new-thing").is_empty());
        assert!(check_name_of(core, "core-mystery").iter().any(|e| e.contains("craftbridge-<area>")));
    }

    #[test]
    fn a_library_target_named_otherwise_is_reported() {
        let mut ffi = input("craftbridge-effects-ffi", "effects/craftbridge-effects-ffi", Vec::new(), vec!["staticlib", "cdylib", "lib"]);
        assert!(check_crate(ENTRIES, &ffi).is_empty());
        ffi.library_names = vec!["film_ffi".to_string()];
        let errors = check_crate(ENTRIES, &ffi);
        assert!(errors.len() == 1 && errors[0].contains("library is named `film_ffi`") && errors[0].contains("`craftbridge_effects_ffi`"), "{errors:?}");
    }

    #[test]
    fn core_to_app_is_forbidden_and_app_to_core_is_allowed() {
        let upward = check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", vec![workspace("craftbridge-effects")], Vec::new()));
        assert!(upward.iter().any(|e| e.contains("the universal core never depends on the effects plugin")), "{upward:?}");
        let downward = check_crate(ENTRIES, &input("craftbridge-effects", "effects/craftbridge-effects", vec![workspace("craftbridge-core")], Vec::new()));
        assert!(downward.is_empty(), "{downward:?}");
    }

    #[test]
    fn dev_dependencies_keep_the_isolation_rules_and_nothing_else() {
        // The core's tests are the core's code: they know no app plugin and no Craft app either.
        let core = check_crate(
            ENTRIES,
            &input("craftbridge-core", "core/craftbridge-core", vec![dev(workspace("craftbridge-effects")), dev(external("effectcraft-time"))], Vec::new()),
        );
        assert_eq!(core.len(), 2, "{core:?}");
        // A test build ships nothing, so the rank and uniffi rules leave dev-dependencies alone.
        let shipping_rules = check_crate(
            ENTRIES,
            &input("craftbridge-media", "core/craftbridge-media", vec![dev(workspace("craftbridge-core")), dev(external("uniffi"))], Vec::new()),
        );
        assert!(shipping_rules.is_empty(), "{shipping_rules:?}");
        // A tool may reach every app, so a test that took one would reach the plugins and EffectCraft
        // through it: the tool rule holds for tests too.
        let tool =
            check_crate(ENTRIES, &input("craftbridge-media", "core/craftbridge-media", vec![dev(workspace("craftbridge-effects-conformance"))], Vec::new()));
        assert!(tool.len() == 1 && tool[0].contains("nothing depends on a host tool, not even a crate's tests"), "{tool:?}");
    }

    #[test]
    fn a_path_dependency_outside_the_workspace_is_refused() {
        let outside = Dependency {
            name: "craftbridge-effects-helper".into(),
            workspace: None,
            outside: Some("tools/craftbridge-effects-helper".into()),
            dev: false,
            build: false,
            source: None,
        };
        let errors = check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", vec![outside], Vec::new()));
        assert!(
            errors.len() == 1
                && errors[0].contains("depends on craftbridge-effects-helper by a path outside the workspace (`tools/craftbridge-effects-helper`)"),
            "{errors:?}"
        );
        // The workspace's own crate under its name, taken from anywhere but its path, is a copy of it.
        let copy = check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", vec![external("craftbridge-media")], Vec::new()));
        assert!(copy.iter().any(|e| e.contains("from outside the workspace")), "{copy:?}");
    }

    #[test]
    fn nothing_depends_on_a_tool_and_a_tool_depends_on_anything() {
        let errors = check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", vec![workspace("xtask")], Vec::new()));
        assert!(errors.iter().any(|e| e.contains("nothing depends on a host tool")), "{errors:?}");
        let app_errors = check_crate(ENTRIES, &input("craftbridge-effects", "effects/craftbridge-effects", vec![workspace("xtask")], Vec::new()));
        assert!(app_errors.iter().any(|e| e.contains("nothing depends on a host tool")), "{app_errors:?}");
        let tool = check_crate(
            ENTRIES,
            &input(
                "craftbridge-effects-codegen",
                "effects/craftbridge-effects-codegen",
                vec![workspace("craftbridge-effects-ffi"), workspace("xtask"), external("uniffi"), external("effectcraft-host"), external("filmcraft-media")],
                Vec::new(),
            ),
        );
        assert!(tool.is_empty(), "{tool:?}");
    }

    /// The support crate is host code the tools share: a tool takes it freely, a shipped crate only from
    /// its tests, and it takes no CraftBridge crate nor any Craft app's crate.
    #[test]
    fn a_support_crate_is_taken_by_tools_and_by_tests_only_and_takes_no_crate_of_ours() {
        let support = "craftbridge-process";
        let tool = check_crate(ENTRIES, &input("xtask", "xtask", vec![workspace(support)], Vec::new()));
        assert!(tool.is_empty(), "{tool:?}");
        let app_tool = check_crate(
            ENTRIES,
            &input("craftbridge-effects-conformance", "effects/craftbridge-effects-conformance", vec![dev(workspace(support))], Vec::new()),
        );
        assert!(app_tool.is_empty(), "{app_tool:?}");
        for (name, folder) in [("craftbridge-core", "core/craftbridge-core"), ("craftbridge-effects", "effects/craftbridge-effects")] {
            let tests = check_crate(ENTRIES, &input(name, folder, vec![dev(workspace(support))], Vec::new()));
            assert!(tests.is_empty(), "{name}: a test may take it: {tests:?}");
            let shipped = check_crate(ENTRIES, &input(name, folder, vec![workspace(support)], Vec::new()));
            assert!(shipped.len() == 1 && shipped[0].contains(&format!("depends on {support}: a support crate is host code")), "{name}: {shipped:?}");
        }
        for (target, dev_only) in [("craftbridge-core", false), ("craftbridge-effects", true), ("xtask", false)] {
            let dependency = if dev_only { dev(workspace(target)) } else { workspace(target) };
            let errors = check_crate(ENTRIES, &input(support, support, vec![dependency], Vec::new()));
            assert!(errors.len() == 1 && errors[0].contains("a support crate depends on no CraftBridge crate"), "{target}: {errors:?}");
        }
        for craft_crate in [external("effectcraft-time"), dev(external("effectcraft-time"))] {
            let craft = check_crate(ENTRIES, &input(support, support, vec![craft_crate], Vec::new()));
            assert!(craft.len() == 1 && craft[0].contains("a support crate knows nothing about any Craft app"), "even from its tests: {craft:?}");
        }
        let plain = check_crate(ENTRIES, &input(support, support, vec![external("serde")], Vec::new()));
        assert!(plain.is_empty(), "external crates that are no Craft app's are allowed: {plain:?}");
        let misplaced = check_crate(ENTRIES, &input(support, "core/craftbridge-process", Vec::new(), Vec::new()));
        assert!(misplaced.iter().any(|e| e.contains("the table says `craftbridge-process`")), "{misplaced:?}");
    }

    /// A support crate is named for its job: `craftbridge-<job>`, and no Craft app's noun.
    #[test]
    fn a_support_crate_carries_the_prefix_and_no_app_noun() {
        let row = ENTRIES.iter().find(|row| row.name == "craftbridge-process").unwrap();
        assert!(check_name_of(row, "craftbridge-process").is_empty());
        assert!(check_name_of(row, "process-support").iter().any(|e| e.contains("a support crate is named craftbridge-<job>")));
        let named = check_name_of(row, "craftbridge-film-process");
        assert!(named.len() == 1 && named[0].contains("a support crate names no app"), "{named:?}");
    }

    #[test]
    fn layers_point_down_within_a_kind() {
        let upward = check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", vec![workspace("craftbridge-core-ffi")], Vec::new()));
        assert!(upward.iter().any(|e| e.contains("layers point down")), "{upward:?}");
        // An edge to a crate of the same rank, itself included, is refused: it is a cycle in waiting.
        let same_rank = check_crate(ENTRIES, &input("craftbridge-core-ffi", "core/craftbridge-core-ffi", vec![workspace("craftbridge-core-ffi")], Vec::new()));
        assert!(same_rank.iter().any(|e| e.contains("layers point down")), "{same_rank:?}");
        let down = check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", vec![workspace("craftbridge-media")], Vec::new()));
        assert!(down.is_empty(), "rank 1 on rank 0 points down: {down:?}");
        let compliant = check_crate(
            ENTRIES,
            &input("craftbridge-core-ffi", "core/craftbridge-core-ffi", vec![workspace("craftbridge-core"), workspace("craftbridge-media")], Vec::new()),
        );
        assert!(compliant.is_empty(), "{compliant:?}");
    }

    /// A second app's plugin row, which the shipped table does not have yet.
    const FILM: Entry = Entry { name: "craftbridge-film", kind: Kind::App("film"), rank: 0, ffi: false, platform: false, build: false, upstream: &[] };

    /// The second app's native library, which the shipped table does not have yet either.
    const FILM_FFI: Entry = Entry { name: "craftbridge-film-ffi", rank: 1, ffi: true, ..FILM };

    /// The combined distribution crate plan §20 names, which ships the effects app and a second one in one
    /// bundle.
    const ALL: Entry = Entry { name: "craftbridge-all-ffi", kind: Kind::Distribution, rank: 0, ffi: true, platform: false, build: false, upstream: &[] };

    /// The shipped table and `rows`, as a test files a crate the shipped table does not have.
    fn with_rows(rows: &[Entry]) -> Vec<Entry> {
        ENTRIES.iter().chain(rows).copied().collect()
    }

    /// What the rule refuses a dependency on a combined distribution crate with, from any crate but a tool.
    const ON_A_DISTRIBUTION: &str = "nothing but a host tool depends on a combined distribution crate, not even a crate's tests: it links every app it ships, so whatever took it would carry them all";

    /// What the rule says of a crate built as a native library that is no distribution library.
    const NOT_A_LIBRARY: &str =
        "is built as a staticlib or cdylib: only a distribution library may be, an app's craftbridge-<app>-ffi or a combined distribution crate";

    /// What the rule says of a distribution library that does not build both native kinds.
    const HALF_A_LIBRARY: &str = "is a distribution library, the one native library its bundle links, so it builds both a staticlib (iOS) and a cdylib (Android): crate-type = [\"staticlib\", \"cdylib\", \"lib\"]";

    /// The crates of both apps, as their bundle's combined distribution crate takes them.
    fn both_apps() -> Vec<Dependency> {
        vec![workspace("craftbridge-effects-ffi"), workspace("craftbridge-film-ffi")]
    }

    /// The combined distribution crate in its own folder, built as a distribution library is.
    fn distribution(dependencies: Vec<Dependency>) -> CrateInput {
        input(ALL.name, ALL.name, dependencies, vec!["staticlib", "cdylib", "lib"])
    }

    #[test]
    fn apps_never_depend_on_each_other() {
        let effects = entry("craftbridge-effects").unwrap();
        let mut errors = Vec::new();
        check_edge(effects, &FILM, false, "x", &mut errors);
        check_edge(&FILM, effects, true, "x", &mut errors);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().all(|e| e.contains("another app's plugin")), "{errors:?}");
        // Two crates of the same app are ordered by rank, and the compliant edge passes.
        assert!(
            check_crate(
                ENTRIES,
                &input("craftbridge-effects-ffi", "effects/craftbridge-effects-ffi", vec![workspace("craftbridge-effects")], vec!["staticlib", "cdylib"])
            )
            .is_empty()
        );
    }

    #[test]
    fn a_plugin_uses_its_own_craft_app_and_no_sibling() {
        let own = check_crate(
            ENTRIES,
            &input(
                "craftbridge-effects",
                "effects/craftbridge-effects",
                vec![external("effectcraft-host"), external("craft-docpath"), external("effectcraft")],
                Vec::new(),
            ),
        );
        assert!(own.is_empty(), "{own:?}");
        let sibling = check_crate(ENTRIES, &input("craftbridge-effects", "effects/craftbridge-effects", vec![external("filmcraft-media")], Vec::new()));
        assert!(sibling.iter().any(|e| e.contains("its own Craft app's crates")), "{sibling:?}");
        let bare = check_crate(ENTRIES, &input("craftbridge-effects", "effects/craftbridge-effects", vec![external("filmcraft")], Vec::new()));
        assert!(bare.iter().any(|e| e.contains("its own Craft app's crates")), "the app's bare crate name is the app's: {bare:?}");
        let mut errors = Vec::new();
        check_external(&FILM, "effectcraft-engine", None, false, "x", &mut errors);
        assert!(errors.iter().any(|e| e.contains("its own Craft app's crates")), "{errors:?}");
    }

    #[test]
    fn the_core_uses_no_craft_app_beyond_its_rows_upstream() {
        let core_upstream = check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", vec![external("effectcraft-host")], Vec::new()));
        assert!(core_upstream.iter().any(|e| e.contains("knows nothing about any Craft app")), "{core_upstream:?}");
        // FilmCraft's decoder registry is where the core's decoders register, so the media crate reads it.
        let media = check_crate(
            ENTRIES,
            &input(
                "craftbridge-media",
                "core/craftbridge-media",
                vec![external("filmcraft-codecs"), external("filmcraft-color"), external("filmcraft-frame"), external("filmcraft-isobmff")],
                Vec::new(),
            ),
        );
        assert!(media.is_empty(), "{media:?}");
        assert!(!check_crate(ENTRIES, &input("craftbridge-media", "core/craftbridge-media", vec![external("filmcraft-media")], Vec::new())).is_empty());
        assert!(!check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", vec![external("filmcraft-frame")], Vec::new())).is_empty());
        assert!(!check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", vec![external("filmcraft-codecs")], Vec::new())).is_empty());
        assert!(!check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", vec![external("effectcraft")], Vec::new())).is_empty());
        assert!(
            check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", vec![external("craft-docpath"), external("serde")], Vec::new()))
                .is_empty()
        );
    }

    #[test]
    fn uniffi_appears_only_in_ffi_crates() {
        let outside = check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", vec![external("uniffi")], Vec::new()));
        assert!(outside.iter().any(|e| e.contains("-ffi crates")), "{outside:?}");
        assert!(
            check_crate(ENTRIES, &input("craftbridge-core-ffi", "core/craftbridge-core-ffi", vec![external("uniffi"), external("serde")], Vec::new()))
                .is_empty()
        );
    }

    #[test]
    fn only_a_distribution_library_becomes_a_native_library() {
        let stray = check_crate(ENTRIES, &input("craftbridge-effects", "effects/craftbridge-effects", Vec::new(), vec!["cdylib"]));
        assert_eq!(stray, [format!("rust/effects/craftbridge-effects/Cargo.toml: craftbridge-effects {NOT_A_LIBRARY}")]);
        // An `-ffi` name makes a UniFFI boundary, not a native library: the core's boundary and an app's
        // second boundary are Rust libraries its native library links.
        let core_ffi = check_crate(ENTRIES, &input("craftbridge-core-ffi", "core/craftbridge-core-ffi", Vec::new(), vec!["staticlib", "lib"]));
        assert_eq!(core_ffi, [format!("rust/core/craftbridge-core-ffi/Cargo.toml: craftbridge-core-ffi {NOT_A_LIBRARY}")]);
        let extra = Entry { name: "craftbridge-effects-extra-ffi", ..*entry("craftbridge-effects-ffi").unwrap() };
        let second = check_crate(&with_rows(&[extra]), &input(extra.name, "effects/craftbridge-effects-extra-ffi", Vec::new(), vec!["cdylib", "lib"]));
        assert_eq!(second, [format!("rust/effects/craftbridge-effects-extra-ffi/Cargo.toml: craftbridge-effects-extra-ffi {NOT_A_LIBRARY}")]);
        assert!(
            check_crate(ENTRIES, &input("craftbridge-effects-ffi", "effects/craftbridge-effects-ffi", Vec::new(), vec!["staticlib", "cdylib", "lib"]))
                .is_empty()
        );
        let table = with_rows(&[FILM_FFI, ALL]);
        assert!(check_crate(&table, &distribution(both_apps())).is_empty());
        // A distribution library, an app's or a combined distribution crate, must build both native kinds,
        // or its bundle has no native library on one of the platforms.
        for half in [vec!["lib"], vec!["staticlib", "lib"], vec!["cdylib", "lib"]] {
            let built = check_crate(ENTRIES, &input("craftbridge-effects-ffi", "effects/craftbridge-effects-ffi", Vec::new(), half.clone()));
            assert_eq!(built, [format!("rust/effects/craftbridge-effects-ffi/Cargo.toml: craftbridge-effects-ffi {HALF_A_LIBRARY}")], "{half:?}");
            let combined = check_crate(&table, &CrateInput { crate_types: half.iter().map(|kind| kind.to_string()).collect(), ..distribution(both_apps()) });
            assert_eq!(combined, [format!("rust/craftbridge-all-ffi/Cargo.toml: craftbridge-all-ffi {HALF_A_LIBRARY}")], "{half:?}");
        }
        // A Rust dylib carries its own runtime, which two native libraries in one app would duplicate.
        let dylib = check_crate(ENTRIES, &input("craftbridge-core", "core/craftbridge-core", Vec::new(), vec!["dylib"]));
        assert_eq!(
            dylib,
            [
                "rust/core/craftbridge-core/Cargo.toml: craftbridge-core is built as a Rust dylib, which carries its own copy of the Rust runtime: an app links one native library"
            ]
        );
    }

    #[test]
    fn the_table_keeps_one_native_library_per_app() {
        let effects = *entry("craftbridge-effects").unwrap();
        let library = *entry("craftbridge-effects-ffi").unwrap();
        let no_library = "rust/xtask/src/layers.rs: the effects plugin has no row craftbridge-effects-ffi: an app has exactly one native library, which a bundle of that app alone links";
        let extra = Entry { name: "craftbridge-effects-extra-ffi", ..library };
        assert_eq!(table_errors(&[effects, extra]), [no_library], "an app's second boundary is not its native library");
        // A row of that name of another kind is not the app's.
        assert_eq!(table_errors(&[effects, Entry { kind: Kind::Core, ..library }]), [no_library]);
        // Nor is a combined distribution crate: it ships the app with another, and a bundle of the film app
        // alone still needs the film app's own library.
        assert_eq!(
            table_errors(&[effects, library, FILM, ALL]),
            [
                "rust/xtask/src/layers.rs: the film plugin has no row craftbridge-film-ffi: an app has exactly one native library, which a bundle of that app alone links"
            ]
        );
        assert!(table_errors(&with_rows(&[FILM, FILM_FFI, ALL])).is_empty(), "{:?}", table_errors(&with_rows(&[FILM, FILM_FFI, ALL])));
        // Two rows of one crate are one crate with rules nobody reads, the app's library among them.
        for twice in [library, *entry("craftbridge-core").unwrap(), ALL] {
            assert_eq!(
                table_errors(&with_rows(&[FILM_FFI, ALL, twice])),
                [format!("rust/xtask/src/layers.rs: {}: a crate has one row in the table, and the rules read only the first", twice.name)]
            );
        }
        let unnamed_boundary = [Entry { ffi: true, ..FILM }, FILM_FFI];
        assert!(table_errors(&unnamed_boundary).iter().any(|e| e.contains("is named -ffi")), "{:?}", table_errors(&unnamed_boundary));
        // A combined distribution crate is its bundle's UniFFI boundary, which its `-ffi` name says.
        assert_eq!(
            table_errors(&with_rows(&[FILM_FFI, Entry { ffi: false, ..ALL }])),
            ["rust/xtask/src/layers.rs: craftbridge-all-ffi: a UniFFI boundary, and only one, is named -ffi"]
        );
        let unknown_app = [Entry { name: "craftbridge-sound-ffi", kind: Kind::App("sound"), ffi: true, ..FILM }];
        assert!(table_errors(&unknown_app).iter().any(|e| e.contains("not a Craft app")), "{:?}", table_errors(&unknown_app));
    }

    /// A build script reads the SDK's library as two lines: its package and its library's name, which
    /// follow the table, and the table's own refusal when it gives none.
    #[test]
    fn a_build_script_reads_the_sdk_s_library_as_its_package_and_its_library_s_name() {
        assert_eq!(sdk_library_lines(ENTRIES), Ok(vec!["package=craftbridge-effects-ffi".to_string(), "library=craftbridge_effects_ffi".to_string()]));
        assert_eq!(
            sdk_library_lines(&with_rows(&[FILM, FILM_FFI, ALL])),
            Ok(vec!["package=craftbridge-all-ffi".to_string(), "library=craftbridge_all_ffi".to_string()])
        );
        assert_eq!(
            sdk_library_lines(&with_rows(&[FILM, FILM_FFI])),
            Err("rust/xtask/src/layers.rs: the table has the effects and film plugins and no combined distribution crate, the one native library the SDK ships them all in".to_string())
        );
    }

    /// The SDK ships every plugin the table has behind one native library: the one app's own while there
    /// is one, the combined distribution crate once there are several, and no guess where the table gives
    /// none or two.
    #[test]
    fn the_sdk_ships_its_one_app_s_library_or_the_combined_distribution_crate() {
        assert_eq!(sdk_library(), Ok("craftbridge-effects-ffi"));
        assert_eq!(sdk_library_of(&with_rows(&[FILM, FILM_FFI, ALL])), Ok("craftbridge-all-ffi"));
        assert_eq!(sdk_library_of(&with_rows(&[ALL])), Ok("craftbridge-effects-ffi"), "one app ships in its own library");
        assert_eq!(
            sdk_library_of(&with_rows(&[FILM, FILM_FFI])),
            Err("rust/xtask/src/layers.rs: the table has the effects and film plugins and no combined distribution crate, the one native library the SDK ships them all in".to_string())
        );
        let more = Entry { name: "craftbridge-more-ffi", ..ALL };
        assert_eq!(
            sdk_library_of(&with_rows(&[FILM, FILM_FFI, ALL, more])),
            Err("rust/xtask/src/layers.rs: the SDK ships one native library, and the table has several combined distribution crates: craftbridge-all-ffi and craftbridge-more-ffi".to_string())
        );
        let effects = *entry("craftbridge-effects").unwrap();
        assert_eq!(
            sdk_library_of(&[effects, Entry { name: "craftbridge-effects-extra-ffi", ..*entry("craftbridge-effects-ffi").unwrap() }]),
            Err("rust/xtask/src/layers.rs: the effects plugin has no row craftbridge-effects-ffi: an app has exactly one native library, which a bundle of that app alone links".to_string()),
            "an app's second boundary is not its native library"
        );
        assert_eq!(
            sdk_library_of(&[*entry("craftbridge-core").unwrap(), *entry("xtask").unwrap(), ALL]),
            Err("rust/xtask/src/layers.rs: the table has no app plugin, so the SDK has no native library to ship".to_string())
        );
    }

    /// A combined distribution crate is `craftbridge-<bundle>-ffi`, at the top of `rust/`, and names no
    /// app.
    #[test]
    fn a_distribution_crate_sits_under_no_app_and_names_none() {
        assert_eq!(expected_folder(&ALL), "craftbridge-all-ffi");
        let table = with_rows(&[FILM_FFI, ALL]);
        let under_an_app = check_crate(&table, &CrateInput { folder: "effects/craftbridge-all-ffi".into(), ..distribution(both_apps()) });
        assert_eq!(
            under_an_app,
            [
                "rust/craftbridge-all-ffi/Cargo.toml: craftbridge-all-ffi lives in `effects/craftbridge-all-ffi`, the table says `craftbridge-all-ffi` (a crate's folder is its name, under its kind's folder)"
            ]
        );
        assert!(check_name_of(&ALL, "craftbridge-all-ffi").is_empty());
        assert!(check_name_of(&ALL, "craftbridge-studio_suite-ffi").is_empty(), "a bundle may be named in several segments");
        for misnamed in [
            "craftbridge-all",
            "all-ffi",
            "craftbridge-ffi",
            "craftbridge--ffi",
            "bundle-all-ffi",
            // A bundle of separators alone, or with an empty segment, which Cargo accepts as a name.
            "craftbridge---ffi",
            "craftbridge-_-ffi",
            "craftbridge-all--ffi",
            "craftbridge-_all-ffi",
        ] {
            assert_eq!(check_name_of(&ALL, misnamed), [format!("x: {misnamed}: a combined distribution crate is named craftbridge-<bundle>-ffi")]);
        }
        // Cargo reads `_` as `-`, and a noun in capitals is the noun, as for a core crate.
        for (named, noun) in [("craftbridge-effects-film-ffi", "effects"), ("craftbridge-all_Film-ffi", "film")] {
            assert_eq!(check_name_of(&ALL, named), [format!("x: {named}: a combined distribution crate names no app: {noun} is one, and it ships several")]);
        }
    }

    /// A combined distribution crate takes the core's crates and two apps' or more, in any rank, and
    /// through their plugins only; a tool and a shipped support crate stay out of it as out of any shipped
    /// crate.
    #[test]
    fn a_distribution_crate_ships_two_apps_or_more_through_their_plugins() {
        let table = with_rows(&[FILM, FILM_FFI, ALL]);
        let at = "rust/craftbridge-all-ffi/Cargo.toml: craftbridge-all-ffi";
        let full = [
            workspace("craftbridge-effects-ffi"),
            workspace("craftbridge-effects"),
            workspace("craftbridge-film-ffi"),
            workspace("craftbridge-film"),
            workspace("craftbridge-core-ffi"),
            workspace("craftbridge-core"),
            workspace("craftbridge-media"),
            dev(workspace("craftbridge-process")),
            external("uniffi"),
            external("serde"),
            external("craft-docpath"),
        ];
        assert!(check_crate(&table, &distribution(full.to_vec())).is_empty(), "{:?}", check_crate(&table, &distribution(full.to_vec())));
        let one_app = format!(
            "{at} takes only the effects app's crates, whose bundle links craftbridge-effects-ffi alone: a combined distribution crate ships two apps or more together"
        );
        for single in [
            vec![workspace("craftbridge-effects-ffi")],
            vec![workspace("craftbridge-effects-ffi"), workspace("craftbridge-effects"), workspace("craftbridge-core")],
            // A dev-dependency ships nothing, so a second app taken only by the tests is not shipped, and
            // nor is one taken only by a build script, which runs on the build host.
            vec![workspace("craftbridge-effects-ffi"), dev(workspace("craftbridge-film-ffi"))],
            vec![workspace("craftbridge-effects-ffi"), build(workspace("craftbridge-film-ffi")), build(workspace("craftbridge-film"))],
        ] {
            assert_eq!(check_crate(&table, &distribution(single.clone())), [one_app.as_str()], "{single:?}");
        }
        let no_app = format!("{at} takes no app's crates: a combined distribution crate ships two apps or more together");
        assert_eq!(check_crate(&table, &distribution(vec![workspace("craftbridge-core")])), [no_app.as_str()]);
        assert_eq!(check_crate(&table, &distribution(Vec::new())), [no_app]);
        // A tool, a shipped support crate and another combined distribution crate are refused by their own
        // rules.
        let more = Entry { name: "craftbridge-more-ffi", ..ALL };
        let with_more = with_rows(&[FILM_FFI, ALL, more]);
        for (dependency, reason) in [
            (workspace("xtask"), "nothing depends on a host tool, not even a crate's tests: a tool may reach every app"),
            (dev(workspace("uniffi-bindgen")), "nothing depends on a host tool, not even a crate's tests: a tool may reach every app"),
            (workspace("craftbridge-process"), "a support crate is host code: a shipped crate takes it only from its tests (a dev-dependency)"),
            (workspace(more.name), ON_A_DISTRIBUTION),
        ] {
            let target = dependency.name.clone();
            let mut dependencies = both_apps();
            dependencies.push(dependency);
            assert_eq!(check_crate(&with_more, &distribution(dependencies)), [format!("{at} depends on {target}: {reason}")]);
        }
        // A Craft app's own crate, known by its name or its repository, comes in through its plugin, even
        // for the combined distribution crate's tests.
        let from_effectcraft =
            Dependency { source: Some("git+https://github.com/storytold/effectcraft?rev=1037605#1037605".into()), ..external("posterize-bands") };
        for craft in
            [external("effectcraft-engine"), dev(external("effectcraft-host")), external("filmcraft-codecs"), external("effectcraft"), from_effectcraft]
        {
            let name = craft.name.clone();
            let mut dependencies = both_apps();
            dependencies.push(craft);
            assert_eq!(
                check_crate(&table, &distribution(dependencies)),
                [format!("{at} depends on {name}: a combined distribution crate takes a Craft app only through that app's plugin, and holds no app's code")]
            );
        }
    }

    /// Nothing but a tool depends on a combined distribution crate, as on a tool, not even from its tests:
    /// the crate links every app it ships into whatever takes it.
    #[test]
    fn nothing_but_a_tool_depends_on_a_distribution_crate() {
        let more = Entry { name: "craftbridge-more-ffi", ..ALL };
        let table = with_rows(&[FILM_FFI, ALL, more]);
        for (name, folder, crate_types) in [
            ("craftbridge-media", "core/craftbridge-media", vec![]),
            ("craftbridge-core-ffi", "core/craftbridge-core-ffi", vec![]),
            ("craftbridge-effects", "effects/craftbridge-effects", vec![]),
            ("craftbridge-effects-ffi", "effects/craftbridge-effects-ffi", vec!["staticlib", "cdylib", "lib"]),
            ("craftbridge-film-ffi", "film/craftbridge-film-ffi", vec!["staticlib", "cdylib", "lib"]),
        ] {
            for dependency in [workspace(ALL.name), dev(workspace(ALL.name))] {
                let errors = check_crate(&table, &input(name, folder, vec![dependency], crate_types.clone()));
                assert_eq!(errors, [format!("rust/{folder}/Cargo.toml: {name} depends on craftbridge-all-ffi: {ON_A_DISTRIBUTION}")]);
            }
        }
        for dependency in [workspace(ALL.name), dev(workspace(ALL.name))] {
            let mut dependencies = both_apps();
            dependencies.push(dependency);
            let errors = check_crate(&table, &input(more.name, more.name, dependencies, vec!["staticlib", "cdylib", "lib"]));
            assert_eq!(errors, [format!("rust/craftbridge-more-ffi/Cargo.toml: craftbridge-more-ffi depends on craftbridge-all-ffi: {ON_A_DISTRIBUTION}")]);
        }
        // The support crate takes no CraftBridge crate at all, and its own rule says so first.
        let support = check_crate(&table, &input("craftbridge-process", "craftbridge-process", vec![dev(workspace(ALL.name))], Vec::new()));
        assert_eq!(
            support,
            [
                "rust/craftbridge-process/Cargo.toml: craftbridge-process depends on craftbridge-all-ffi: a support crate depends on no CraftBridge crate: every tool shares it, so it stays below them all"
            ]
        );
        // A tool may depend on anything: the bindgen builds a distribution library to read its exports.
        for (name, folder) in [("uniffi-bindgen", "uniffi-bindgen"), ("xtask", "xtask"), ("craftbridge-effects-codegen", "effects/craftbridge-effects-codegen")]
        {
            for dependency in [workspace(ALL.name), dev(workspace(ALL.name))] {
                let errors = check_crate(&table, &input(name, folder, vec![dependency], Vec::new()));
                assert!(errors.is_empty(), "{name}: {errors:?}");
            }
        }
    }

    /// Bring the lock of a copied workspace up to date with what was planted in it, offline, from what the
    /// workspace's own build already fetched: the rules read the graph `--locked`.
    fn relock(dir: &Path) {
        let locked = std::process::Command::new(env!("CARGO")).args(["metadata", "--format-version", "1", "--offline"]).current_dir(dir).output().unwrap();
        assert!(locked.status.success(), "{}", String::from_utf8_lossy(&locked.stderr));
    }

    /// Push `entries` onto the list `key` (`members`, `default-members`) of the `[workspace]` table of the
    /// workspace copied to `rust_dir`; with no entries, leave the list out of the table.
    fn edit_list(rust_dir: &Path, key: &str, entries: &[&str]) {
        let path = rust_dir.join("Cargo.toml");
        let mut manifest: toml_edit::DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let workspace = manifest["workspace"].as_table_mut().unwrap();
        if entries.is_empty() {
            workspace.remove(key).unwrap();
        } else {
            let list = workspace[key].as_array_mut().unwrap();
            entries.iter().for_each(|entry| list.push(*entry));
        }
        std::fs::write(&path, manifest.to_string()).unwrap();
    }

    /// What the rule says of a crate the table says ships that `default-members` leaves out.
    fn left_out(name: &str) -> String {
        format!(
            "rust/Cargo.toml: default-members leaves out {name}, which the layer table (rust/xtask/src/layers.rs) says ships: the phone targets' lint runs read a crate in full, its tests and every feature, only when it is a default member, and not at all when no default member takes it"
        )
    }

    /// What the rule says of a crate in `default-members` the table does not say ships.
    fn held(name: &str) -> String {
        format!(
            "rust/Cargo.toml: default-members holds {name}, which the layer table (rust/xtask/src/layers.rs) does not say ships: the default members are exactly the crates an app links, which the phone targets' runs build"
        )
    }

    /// A planted `craftbridge-all-ffi`, a member of a copy of the workspace taking the effects app's native
    /// library and a stand-in second app's, read through `cargo metadata` as the gate reads the workspace:
    /// left out of the default members with the stand-in app's library, both are named; listed there and
    /// filed as a combined distribution crate it passes, and filed as an app's crate or a core crate it
    /// fails the isolation rule that kind keeps, in that rule's words. Once the stand-in app's tests take
    /// it, the distribution rule refuses them.
    #[test]
    fn a_planted_distribution_crate_passes_as_one_and_fails_as_an_app_or_a_core_crate() {
        let copy = crate::scratch::rust_tree_copy("layers-distribution");
        let rust_dir = copy.0.join("rust");
        edit_list(&rust_dir, "members", &["film/*", ALL.name]);
        let native = |name: &str, dependencies: &str| {
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\ncrate-type = [\"staticlib\", \"cdylib\", \"lib\"]\n{dependencies}"
            )
        };
        copy.write("rust/film/craftbridge-film-ffi/Cargo.toml", native(FILM_FFI.name, ""));
        copy.write("rust/film/craftbridge-film-ffi/src/lib.rs", "\n");
        let apps = "\n[dependencies]\ncraftbridge-effects-ffi = { path = \"../effects/craftbridge-effects-ffi\" }\ncraftbridge-film-ffi = { path = \"../film/craftbridge-film-ffi\" }\n";
        copy.write("rust/craftbridge-all-ffi/Cargo.toml", native(ALL.name, apps));
        copy.write("rust/craftbridge-all-ffi/src/lib.rs", "\n");
        relock(&rust_dir);
        let filed = |row: Entry| {
            let mut errors = check_against(&copy.0, &with_rows(&[FILM_FFI, row])).unwrap();
            errors.sort();
            errors
        };
        // Both ship, and the members' globs alone list neither as a default member, so the phone runs
        // would lint neither.
        assert_eq!(filed(ALL), [left_out(ALL.name), left_out(FILM_FFI.name)]);
        edit_list(&rust_dir, "default-members", &["film/craftbridge-film-ffi", ALL.name]);
        assert_eq!(filed(ALL), Vec::<String>::new(), "filed as a combined distribution crate");
        let at = "rust/craftbridge-all-ffi/Cargo.toml: craftbridge-all-ffi";
        // Filed as an effects crate above the effects library, it reaches the film app's plugin.
        assert_eq!(
            filed(Entry { kind: Kind::App("effects"), rank: 2, ..ALL }),
            [
                format!("{at} depends on craftbridge-film-ffi: an app plugin never depends on another app's plugin (effects on film), not even in its tests"),
                format!("{at} {NOT_A_LIBRARY}"),
                format!(
                    "{at} lives in `craftbridge-all-ffi`, the table says `effects/craftbridge-all-ffi` (a crate's folder is its name, under its kind's folder)"
                ),
                format!("{at}: an effects plugin crate is named craftbridge-effects or craftbridge-effects-…"),
                "rust/xtask/src/layers.rs: craftbridge-all-ffi: the table says `rust/effects/craftbridge-all-ffi`, but there is no such folder".to_string(),
            ]
        );
        // Filed as a film crate, it reaches the effects app's plugin.
        assert_eq!(
            filed(Entry { kind: Kind::App("film"), rank: 2, ..ALL }),
            [
                format!(
                    "{at} depends on craftbridge-effects-ffi: an app plugin never depends on another app's plugin (film on effects), not even in its tests"
                ),
                format!("{at} {NOT_A_LIBRARY}"),
                format!(
                    "{at} lives in `craftbridge-all-ffi`, the table says `film/craftbridge-all-ffi` (a crate's folder is its name, under its kind's folder)"
                ),
                format!("{at}: a film plugin crate is named craftbridge-film or craftbridge-film-…"),
                "rust/xtask/src/layers.rs: craftbridge-all-ffi: the table says `rust/film/craftbridge-all-ffi`, but there is no such folder".to_string(),
            ]
        );
        // Filed as a core crate above every core rank, it reaches both apps' plugins.
        assert_eq!(
            filed(Entry { kind: Kind::Core, rank: 3, ..ALL }),
            [
                format!("{at} depends on craftbridge-effects-ffi: the universal core never depends on the effects plugin, not even in its tests"),
                format!("{at} depends on craftbridge-film-ffi: the universal core never depends on the film plugin, not even in its tests"),
                format!("{at} {NOT_A_LIBRARY}"),
                format!(
                    "{at} lives in `craftbridge-all-ffi`, the table says `core/craftbridge-all-ffi` (a crate's folder is its name, under its kind's folder)"
                ),
                "rust/xtask/src/layers.rs: craftbridge-all-ffi: the table says `rust/core/craftbridge-all-ffi`, but there is no such folder".to_string(),
            ]
        );
        // The stand-in app's tests take the combined distribution crate, which would carry the film app
        // into them.
        copy.write(
            "rust/film/craftbridge-film-ffi/Cargo.toml",
            native(FILM_FFI.name, "\n[dev-dependencies]\ncraftbridge-all-ffi = { path = \"../../craftbridge-all-ffi\" }\n"),
        );
        relock(&rust_dir);
        assert_eq!(
            filed(ALL),
            [format!("rust/film/craftbridge-film-ffi/Cargo.toml: craftbridge-film-ffi depends on craftbridge-all-ffi: {ON_A_DISTRIBUTION}")]
        );
    }

    /// Which kinds ship: what an app links, never what runs on the build host alone.
    #[test]
    fn the_core_the_plugins_and_a_combined_distribution_crate_ship_and_host_code_does_not() {
        let kinds = [Kind::Core, Kind::App("effects"), Kind::Distribution, Kind::Tool, Kind::Support];
        let shipped: Vec<bool> = kinds.iter().map(|kind| kind.ships()).collect();
        assert_eq!(shipped, [true, true, true, false, false]);
    }

    /// The default members are exactly the rows that ship: a crate that ships and is no default member is
    /// named as left out, and a default member that ships nothing, or has no row, as held, each once.
    #[test]
    fn the_default_members_are_exactly_the_crates_that_ship() {
        /// A crate for each of `table`'s rows, a default member where `default_member` says so.
        fn workspace(table: &[Entry], default_member: impl Fn(&Entry) -> bool) -> Vec<CrateInput> {
            table
                .iter()
                .map(|row| CrateInput { default_member: default_member(row), ..input(row.name, &expected_folder(row), Vec::new(), Vec::new()) })
                .collect()
        }
        let table = with_rows(&[FILM, FILM_FFI, ALL]);
        let at = "rust/Cargo.toml";
        let errors = |inputs: &[CrateInput]| default_member_errors(&table, inputs, at);
        assert_eq!(errors(&workspace(&table, |row| row.kind.ships())), Vec::<String>::new(), "every crate that ships, and no other");
        // The combined distribution crate, at rust/<name>, is under no glob that lists the plugins.
        assert_eq!(errors(&workspace(&table, |row| row.kind.ships() && row.kind != Kind::Distribution)), [left_out(ALL.name)]);
        assert_eq!(errors(&workspace(&table, |row| row.kind.ships() && row.name != "craftbridge-effects-ffi")), [left_out("craftbridge-effects-ffi")]);
        assert_eq!(
            errors(&workspace(&table, |row| row.kind.ships() && row.kind != Kind::Core)),
            ["craftbridge-media", "craftbridge-core", "craftbridge-core-ffi"].map(left_out)
        );
        let with_hosts = workspace(&table, |row| row.kind.ships() || row.name == "xtask" || row.kind == Kind::Support);
        assert_eq!(errors(&with_hosts), [held("craftbridge-process"), held("xtask")]);
        // A default member with no row: the table says nothing of it, so it does not say it ships.
        let mut stray = workspace(&table, |row| row.kind.ships());
        stray.push(CrateInput { default_member: true, ..input("craftbridge-mystery", "core/craftbridge-mystery", Vec::new(), Vec::new()) });
        assert_eq!(errors(&stray), [held("craftbridge-mystery")]);
    }

    /// The acceptance plants, in a copy of the workspace read through `cargo metadata` as the gate reads it,
    /// whose answer resolves `default-members`' globs and paths: a crate the table says ships that the list
    /// leaves out, and one in the list that the table does not say ships, or has no row for, each fail
    /// naming it, and a workspace that leaves the list out, which makes every member a default one, fails
    /// naming each host crate. The tree as it stands, and the planted crate once listed, pass.
    #[test]
    fn the_default_members_are_held_to_what_the_table_says_ships() {
        let copy = crate::scratch::rust_tree_copy("layers-default-members");
        let rust_dir = copy.0.join("rust");
        let checked = |table: &[Entry]| {
            let mut errors = check_against(&copy.0, table).unwrap();
            errors.sort();
            errors
        };
        assert_eq!(checked(ENTRIES), Vec::<String>::new(), "the tree as it stands");
        // A new plugin crate: `effects/*` makes it a member, and the list names the effects crates one by one.
        const EXTRA: Entry = Entry { name: "craftbridge-effects-extra", kind: Kind::App("effects"), rank: 2, ..FILM };
        copy.write(
            "rust/effects/craftbridge-effects-extra/Cargo.toml",
            "[package]\nname = \"craftbridge-effects-extra\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        copy.write("rust/effects/craftbridge-effects-extra/src/lib.rs", "\n");
        relock(&rust_dir);
        let table = with_rows(&[EXTRA]);
        assert_eq!(checked(&table), [left_out(EXTRA.name)], "in the table and left out of the list");
        edit_list(&rust_dir, "default-members", &["effects/craftbridge-effects-extra"]);
        assert_eq!(checked(&table), Vec::<String>::new(), "listed, it passes");
        assert_eq!(
            checked(ENTRIES),
            [
                held(EXTRA.name),
                "rust/effects/craftbridge-effects-extra/Cargo.toml: craftbridge-effects-extra is not in the layer table (rust/xtask/src/layers.rs)".to_string()
            ],
            "in the list and not in the table"
        );
        edit_list(&rust_dir, "default-members", &["xtask"]);
        assert_eq!(checked(&table), [held("xtask")], "in the list, and a host tool in the table");
        edit_list(&rust_dir, "default-members", &[]);
        assert_eq!(
            checked(&table),
            ["craftbridge-effects-codegen", "craftbridge-effects-conformance", "craftbridge-process", "uniffi-bindgen", "xtask"].map(held),
            "no list: Cargo builds every member"
        );
    }

    /// `check_name` against a row and a name that may differ from the row's own, so a test can ask
    /// what the naming rule says about a crate the table does not contain verbatim.
    fn check_name_of(row: &'static Entry, name: &'static str) -> Vec<String> {
        let mut errors = Vec::new();
        check_name(&Entry { name, ..*row }, "x", &mut errors);
        errors
    }

    #[test]
    fn folders_and_workspace_edges_come_from_manifest_paths() {
        let dependency =
            |name: &str, path: Option<&str>| CrateDependency { name: name.into(), path: path.map(str::to_string), source: None, dev: false, build: false };
        let crates = vec![
            Crate {
                name: "craftbridge-core".into(),
                id: "id1".into(),
                source: None,
                edition: "2024".into(),
                target_editions: Vec::new(),
                manifest_path: "/repo/rust/core/craftbridge-core/Cargo.toml".into(),
                dependencies: vec![
                    dependency("serde", None),
                    dependency("craftbridge-media", Some("/repo/rust/core/craftbridge-media")),
                    dependency("effectcraft-time", Some("/elsewhere/effectcraft/crates/time")),
                ],
                library_targets: vec![CrateTarget { name: "craftbridge_core".into(), crate_types: vec!["lib".into()] }],
                build_script: None,
                depends_on: Vec::new(),
                member: true,
                default_member: true,
                binary_targets: Vec::new(),
            },
            Crate {
                name: "craftbridge-media".into(),
                id: "id2".into(),
                source: None,
                edition: "2024".into(),
                target_editions: Vec::new(),
                manifest_path: "/repo/rust/core/craftbridge-media/Cargo.toml".into(),
                dependencies: Vec::new(),
                library_targets: Vec::new(),
                build_script: None,
                depends_on: Vec::new(),
                member: true,
                default_member: false,
                binary_targets: Vec::new(),
            },
        ];
        let inputs = inputs_from(&crates, Path::new("/repo/rust"), Path::new("/repo"));
        assert_eq!(inputs[0].folder, "core/craftbridge-core");
        assert_eq!(inputs[0].manifest, "rust/core/craftbridge-core/Cargo.toml");
        assert_eq!(inputs[0].dependencies[0].workspace, None, "an external dependency points at no workspace crate");
        assert_eq!(inputs[0].dependencies[0].outside, None, "and has no path to be outside of");
        assert_eq!(inputs[0].dependencies[1].workspace.as_deref(), Some("craftbridge-media"));
        assert_eq!(inputs[0].dependencies[2].outside.as_deref(), Some("/elsewhere/effectcraft/crates/time"), "a path outside the workspace is refused");
        assert_eq!(inputs[0].crate_types, ["lib"]);
        assert_eq!(inputs[0].library_names, ["craftbridge_core"]);
        assert!(inputs[0].default_member && !inputs[1].default_member, "each crate's default membership is Cargo's");
        assert_eq!(manifest_folder(Path::new("/repo/rust"), "/repo/rust-other/x/Cargo.toml"), None);
    }

    #[test]
    fn app_nouns_come_from_the_rows() {
        assert_eq!(app_nouns(ENTRIES), vec!["effects"]);
    }

    #[test]
    fn a_core_name_carries_no_app_noun() {
        let core = ENTRIES.iter().find(|row| row.name == "craftbridge-core").unwrap();
        assert!(check_name_of(core, "craftbridge-media").is_empty(), "the tree's own core names pass");
        let planted = check_name_of(core, "craftbridge-film-utils");
        assert_eq!(planted.len(), 1, "{planted:?}");
        assert!(planted[0].starts_with("x: craftbridge-film-utils:") && planted[0].contains("film") && planted[0].contains("serves them all"), "{planted:?}");
        let whole_app = check_name_of(core, "craftbridge-effects");
        assert_eq!(whole_app.len(), 1, "{whole_app:?}");
        assert!(whole_app[0].contains("effects"), "{whole_app:?}");
        // Cargo reads `_` as `-`, so the noun is a segment between either; and a noun in capitals is the noun.
        for spelt in ["craftbridge-film_utils", "craftbridge-media_Effects", "craftbridge-FILM-codecs"] {
            let named = check_name_of(core, spelt);
            assert!(named.len() == 1 && named[0].contains("names no app"), "{spelt}: {named:?}");
        }
        assert!(check_name_of(core, "craftbridge-filmstrip").is_empty(), "a noun inside a longer word is no segment");
        // The tool rule reads the same segments: an app's tool spelt with `_` is still that app's.
        let tool = ENTRIES.iter().find(|row| row.name == "xtask").unwrap();
        let film_tool = check_name_of(tool, "craftbridge-film_codegen");
        assert!(film_tool.len() == 1 && film_tool[0].contains("a tool of the film app"), "{film_tool:?}");
    }
}
