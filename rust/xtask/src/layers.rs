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
//! so each app has exactly one native library; the universal core must stay free of any Craft app to be
//! reusable; and a host tool pulled into a shipped library drags the build host into the phone.
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
}

/// One row of the table.
#[derive(Clone, Copy)]
pub(crate) struct Entry {
    /// The crate name, which is also the folder it lives in.
    pub(crate) name: &'static str,
    pub(crate) kind: Kind,
    /// Layer within its kind: an edge must point at a smaller rank. Tools are unordered: theirs is 0 and
    /// never read.
    pub(crate) rank: u8,
    /// A UniFFI boundary, so it may depend on `uniffi` and allow `unsafe_code` crate-wide.
    pub(crate) ffi: bool,
    /// The distribution library an app links: the only crate built as `staticlib` or `cdylib`.
    pub(crate) distribution: bool,
    /// May hold hand-written `unsafe`, and only under `src/platform/`.
    pub(crate) platform: bool,
    /// May have a build script: a build script runs before the compiler reads any lint table and can hand
    /// the compiler flags and cfgs around it, so only a crate marked here may have one. Nothing is marked:
    /// a crate that ever needs one is a decision of its own.
    pub(crate) build: bool,
    /// The Craft apps' crates a core crate may use. The core knows nothing about any Craft app, with one
    /// exception the plan makes: CraftBridge's decoders register into FilmCraft's decoder registry, the
    /// one EffectCraft decodes through, so the core's media crate reads FilmCraft's codecs and container.
    pub(crate) upstream: &'static [&'static str],
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
        distribution: false,
        platform: true,
        build: false,
        upstream: &["filmcraft-codecs", "filmcraft-isobmff"],
    },
    Entry { name: "craftbridge-core", kind: Kind::Core, rank: 1, ffi: false, distribution: false, platform: true, build: false, upstream: &[] },
    Entry { name: "craftbridge-core-ffi", kind: Kind::Core, rank: 2, ffi: true, distribution: false, platform: true, build: false, upstream: &[] },
    Entry { name: "craftbridge-effects", kind: Kind::App("effects"), rank: 0, ffi: false, distribution: false, platform: false, build: false, upstream: &[] },
    Entry { name: "craftbridge-effects-ffi", kind: Kind::App("effects"), rank: 1, ffi: true, distribution: true, platform: false, build: false, upstream: &[] },
    Entry { name: "craftbridge-effects-codegen", kind: Kind::Tool, rank: 0, ffi: false, distribution: false, platform: false, build: false, upstream: &[] },
    Entry { name: "craftbridge-effects-conformance", kind: Kind::Tool, rank: 0, ffi: false, distribution: false, platform: false, build: false, upstream: &[] },
    Entry { name: "xtask", kind: Kind::Tool, rank: 0, ffi: false, distribution: false, platform: false, build: false, upstream: &[] },
];

/// A crate's row, or `None` for a crate the table has never heard of.
pub(crate) fn entry(name: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|candidate| candidate.name == name)
}

/// Every app noun a table's rows use, which the table's own checks hold to [`CRAFT_APPS`].
fn app_nouns(table: &[Entry]) -> Vec<&'static str> {
    let mut nouns: Vec<&'static str> = table.iter().filter_map(|row| if let Kind::App(noun) = row.kind { Some(noun) } else { None }).collect();
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
}

/// The one folder the table says a crate lives in: `<app>/<name>` for an app plugin, `core/<name>` for
/// the core, and for a tool `<app>/<name>` when it is an app's tool (`craftbridge-<app>-…`) or `<name>`
/// directly under `rust/` when it is no app's. The folder name is always the crate's name. Which tool is
/// an app's comes from [`CRAFT_APPS`], the list the naming rule reads, so a tool's folder and its name
/// never disagree about its app, whether or not the app has a plugin row yet.
fn expected_folder(row: &Entry) -> String {
    match row.kind {
        Kind::Core => format!("core/{}", row.name),
        Kind::App(app) => format!("{app}/{}", row.name),
        Kind::Tool => match CRAFT_APPS.iter().map(|app| app.noun).find(|app| row.name.starts_with(&format!("craftbridge-{app}-"))) {
            Some(app) => format!("{app}/{}", row.name),
            None => row.name.to_string(),
        },
    }
}

/// Check one package: its folder, its name, every edge and its native-library targets. Every message
/// opens with the crate's manifest, the file to open.
pub(crate) fn check_crate(input: &CrateInput) -> Vec<String> {
    let at = &input.manifest;
    let Some(row) = entry(&input.name) else {
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
    // Cargo names a library target after its package, `-` read as `_`; a `[lib] name` of another name
    // renames what code imports and the native library that ships, past every naming rule above.
    let library = row.name.replace('-', "_");
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
            Some(target_name) => match entry(target_name) {
                Some(target) => check_edge(row, target, dependency.dev, at, &mut errors),
                None => errors.push(format!("{at}: {} depends on {target_name}: {target_name} is not in the layer table (rust/xtask/src/layers.rs)", row.name)),
            },
            None if entry(&dependency.name).is_some() => errors.push(format!(
                "{at}: {} depends on {} from outside the workspace: that is the name of this workspace's own crate, which is taken by its path",
                row.name, dependency.name
            )),
            None => check_external(row, &dependency.name, dependency.source.as_deref(), dependency.dev, at, &mut errors),
        }
    }
    if input.crate_types.iter().any(|kind| kind == "dylib") {
        errors.push(format!("{at}: {} is built as a Rust dylib, which carries its own copy of the Rust runtime: an app links one native library", row.name));
    }
    if builds_native(&input.crate_types) && !row.distribution {
        errors.push(format!("{at}: {} is built as a staticlib or cdylib: only the distribution library the table marks may be", row.name));
    }
    if row.distribution && !(input.crate_types.iter().any(|kind| kind == "staticlib") && input.crate_types.iter().any(|kind| kind == "cdylib")) {
        errors.push(format!(
            "{at}: {} is the app's distribution library, so it builds both a staticlib (iOS) and a cdylib (Android): crate-type = [\"staticlib\", \"cdylib\", \"lib\"]",
            row.name
        ));
    }
    errors
}

/// The Craft app a crate's name names, as one of its segments: [`CRAFT_APPS`]' noun. Cargo reads `-` and
/// `_` in a package name alike — `craftbridge-film_utils` builds the library `craftbridge_film_utils` — so
/// a segment ends at either, and a noun in another case names the app as plainly.
fn named_app(name: &str) -> Option<&'static str> {
    CRAFT_APPS.iter().map(|app| app.noun).find(|noun| name.split(['-', '_']).any(|segment| segment.eq_ignore_ascii_case(noun)))
}

/// A core crate is `craftbridge-<area>`, and its name carries no app's noun: the core serves every app,
/// so a core name that names one app mislabels the shelf it sits on. An app plugin carries its app's
/// noun, which stops one app's crate being mistaken for another's, and so does an app's tool. Any other
/// tool keeps the name its convention gives it (`xtask`, `uniffi-bindgen`). `at` is how a message names
/// the crate's manifest.
fn check_name(row: &Entry, at: &str, errors: &mut Vec<String>) {
    match row.kind {
        Kind::Core => {
            if !row.name.starts_with("craftbridge-") {
                errors.push(format!("{at}: {}: a core crate is named craftbridge-<area>", row.name));
            }
            if let Some(noun) = named_app(row.name) {
                errors.push(format!("{at}: {}: a core crate names no app: {noun} is one, and the core serves them all", row.name));
            }
        }
        Kind::App(app) => {
            let prefixed = format!("craftbridge-{app}-");
            if row.name != format!("craftbridge-{app}") && !row.name.starts_with(&prefixed) {
                errors.push(format!("{at}: {}: {} plugin crate is named craftbridge-{app} or craftbridge-{app}-…", row.name, with_article(app)));
            }
        }
        Kind::Tool => {
            if let Some(noun) = named_app(row.name).filter(|noun| !row.name.starts_with(&format!("craftbridge-{noun}-"))) {
                errors.push(format!("{at}: {}: a tool of the {noun} app is named craftbridge-{noun}-…, like the app's other crates", row.name));
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
/// knows no app plugin, and a plugin knows no other app's — and to the tool rule, because a crate's tests
/// are its code, and a tool may reach every app: a test that took one would reach them all through it. Only
/// the rank rule is about what ships alone, and a test build ships nothing.
fn check_edge(row: &Entry, target: &Entry, dev: bool, at: &str, errors: &mut Vec<String>) {
    match (&row.kind, &target.kind) {
        // A tool may depend on anything; nothing may depend on a tool.
        (Kind::Tool, _) => {}
        (Kind::Core, Kind::App(app)) => {
            errors.push(edge(at, row, target, format!("the universal core never depends on the {app} plugin, not even in its tests")))
        }
        (Kind::App(other), Kind::App(app)) if other != app => {
            errors.push(edge(at, row, target, format!("an app plugin never depends on another app's plugin ({other} on {app}), not even in its tests")))
        }
        (_, Kind::Tool) => errors.push(edge(at, row, target, "nothing depends on a host tool, not even a crate's tests: a tool may reach every app")),
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
/// beyond the row's `upstream`; and in an app plugin, its own Craft app's crates and no sibling's. A Craft
/// app's crate is known by its name or by the app's repository it comes from. A tool may depend on
/// anything. A dev-dependency is held to the Craft-app rules only, as in [`check_edge`].
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
        _ => {}
    }
}

/// Does a library target build a native library an app links, rather than a Rust-only `lib`?
fn builds_native(crate_types: &[String]) -> bool {
    crate_types.iter().any(|kind| kind == "staticlib" || kind == "cdylib")
}

/// The rules the table's own rows must keep, so a row added by hand cannot quietly break one: every app
/// has exactly one distribution library, its `craftbridge-<app>-ffi` (two Rust static libraries in one app
/// collide), and nothing else is one; a row is a UniFFI boundary exactly when its name ends in `-ffi`; an
/// app noun names a known Craft app; and only a core crate lists Craft app crates it may use.
fn table_errors(table: &[Entry]) -> Vec<String> {
    let mut errors = Vec::new();
    let at = TABLE;
    for row in table {
        if row.ffi != row.name.ends_with("-ffi") {
            errors.push(format!("{at}: {}: a UniFFI boundary, and only one, is named -ffi", row.name));
        }
        if !row.upstream.is_empty() && row.kind != Kind::Core {
            errors.push(format!("{at}: {}: only a core crate lists the Craft app crates it may use; a plugin uses its own app's", row.name));
        }
        match row.kind {
            Kind::App(noun) if row.distribution && row.name != format!("craftbridge-{noun}-ffi") => {
                errors.push(format!("{at}: {}: the {noun} plugin's distribution library is craftbridge-{noun}-ffi", row.name));
            }
            Kind::App(_) => {}
            Kind::Core | Kind::Tool if row.distribution => errors.push(format!("{at}: {}: only an app plugin has a distribution library", row.name)),
            Kind::Core | Kind::Tool => {}
        }
    }
    for noun in app_nouns(table) {
        if !CRAFT_APPS.iter().any(|app| app.noun == noun) {
            errors.push(format!("{at}: {noun}: not a Craft app this table knows (CRAFT_APPS)"));
        }
        let libraries = table.iter().filter(|row| row.kind == Kind::App(noun) && row.distribution).count();
        if libraries != 1 {
            errors.push(format!("{at}: the {noun} plugin has {libraries} distribution libraries: an app has exactly one native library"));
        }
    }
    errors
}

/// A table row whose folder is not on disk, or whose crate no workspace package is any more, so a crate
/// renamed in place or deleted cannot leave a dead row.
fn missing_crates(rust_dir: &Path, packages: &[&str]) -> Vec<String> {
    let mut errors = Vec::new();
    for row in ENTRIES {
        let folder = expected_folder(row);
        if !rust_dir.join(&folder).is_dir() {
            errors.push(format!("{TABLE}: {}: the table says `rust/{folder}`, but there is no such folder", row.name));
        } else if !packages.contains(&row.name) {
            errors.push(format!("{TABLE}: {}: the table has a row for it, but no workspace package is called that any more", row.name));
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
                    Dependency { name: dependency.name.clone(), workspace, outside, dev: dependency.dev, source: dependency.source.clone() }
                })
                .collect(),
            crate_types: package.library_targets.iter().flat_map(|target| target.crate_types.iter().cloned()).collect(),
            library_names: package.library_targets.iter().map(|target| target.name.clone()).collect(),
            binary_names: package.binary_targets.clone(),
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
        .map(|file| {
            format!("{}: a Cargo package that is no workspace member: no stage compiles it and no rule reads it", crate::report::shown(root, &file.path))
        })
        .collect())
}

/// Run the check against the workspace on disk: read the graph, apply every rule, and make sure the
/// table still matches the crates that exist.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let rust_dir = root.join("rust");
    let crates = crate::meta::read(&rust_dir, crate::meta::Graph::Workspace)?;
    let mut errors = table_errors(ENTRIES);
    errors.extend(local_packages_outside(&crate::meta::read(&rust_dir, crate::meta::Graph::Resolved)?, root));
    errors.extend(stray_manifests(&rust_dir, root, &crates)?);
    let inputs = inputs_from(&crates, &rust_dir, root);
    for input in &inputs {
        errors.extend(check_crate(input));
    }
    errors.extend(folder_case_errors(&rust_dir, &inputs));
    // A crate in the workspace but not in the table is reported by check_crate; a table row with no
    // crate is reported here.
    let names: Vec<&str> = crates.iter().map(|package| package.name.as_str()).collect();
    errors.extend(missing_crates(&rust_dir, &names));
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{CrateDependency, CrateTarget};
    use crate::scratch::Scratch;

    fn workspace(name: &str) -> Dependency {
        Dependency { name: name.to_string(), workspace: Some(name.to_string()), outside: None, dev: false, source: None }
    }

    fn external(name: &str) -> Dependency {
        Dependency { name: name.to_string(), workspace: None, outside: None, dev: false, source: None }
    }

    fn dev(dependency: Dependency) -> Dependency {
        Dependency { dev: true, ..dependency }
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
        }
    }

    /// The crate types each row is built as when it is compliant: the distribution library builds both
    /// native kinds, every other crate a plain library.
    fn compliant_types(row: &Entry) -> Vec<&'static str> {
        if row.distribution { vec!["staticlib", "cdylib", "lib"] } else { vec!["lib"] }
    }

    /// The table as its rows must satisfy it with no edges at all: this is what a fresh skeleton must pass.
    #[test]
    fn every_table_row_satisfies_its_own_rules() {
        assert!(table_errors(ENTRIES).is_empty(), "{:?}", table_errors(ENTRIES));
        for row in ENTRIES {
            let errors = check_crate(&input(row.name, &expected_folder(row), Vec::new(), compliant_types(row)));
            assert!(errors.is_empty(), "{}: {errors:?}", row.name);
        }
    }

    /// Every table row has its folder and its crate on disk.
    #[test]
    fn the_table_matches_the_workspace_on_disk() {
        let names: Vec<&str> = ENTRIES.iter().map(|row| row.name).collect();
        let errors = missing_crates(&crate::repo_root().join("rust"), &names);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn an_unlisted_crate_is_reported_with_its_manifest() {
        let errors = check_crate(&input("craftbridge-mystery", "core/craftbridge-mystery", Vec::new(), Vec::new()));
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
        let errors = missing_crates(&scratch.0, &names);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains(ENTRIES[0].name) && errors[0].contains("no such folder"), "{errors:?}");
        // And a folder tree holding every row finds nothing, which is the compliant case.
        scratch.mkdir(&expected_folder(&ENTRIES[0]));
        assert!(missing_crates(&scratch.0, &names).is_empty());
        // A crate renamed in place keeps its folder, and its row is dead all the same.
        let renamed: Vec<&str> = names.iter().copied().filter(|name| *name != "craftbridge-core").collect();
        let errors = missing_crates(&scratch.0, &renamed);
        assert!(errors.len() == 1 && errors[0].contains("craftbridge-core: the table has a row for it"), "{errors:?}");
    }

    #[test]
    fn a_crate_in_the_wrong_folder_is_reported() {
        let errors = check_crate(&input("craftbridge-core", "effects/craftbridge-core", Vec::new(), Vec::new()));
        assert!(errors.iter().any(|e| e.contains("`core/craftbridge-core`")), "{errors:?}");
        // An app's tool lives under its app, and only there; a tool that is no app's sits at the top.
        assert!(check_crate(&input("craftbridge-effects-codegen", "effects/craftbridge-effects-codegen", Vec::new(), Vec::new())).is_empty());
        let top = check_crate(&input("craftbridge-effects-codegen", "craftbridge-effects-codegen", Vec::new(), Vec::new()));
        assert!(top.iter().any(|e| e.contains("the table says `effects/craftbridge-effects-codegen`")), "{top:?}");
        assert!(!check_crate(&input("xtask", "core/xtask", Vec::new(), Vec::new())).is_empty());
        // A plugin crate in the core's folder is in the wrong kind's folder.
        let plugin_in_core = check_crate(&input("craftbridge-effects", "core/craftbridge-effects", Vec::new(), Vec::new()));
        assert!(plugin_in_core.iter().any(|e| e.contains("the table says `effects/craftbridge-effects`")), "{plugin_in_core:?}");
        // A tool of an app with no plugin row yet still lives under that app, where its name puts it.
        let film_tool = Entry {
            name: "craftbridge-film-codegen",
            kind: Kind::Tool,
            rank: 0,
            ffi: false,
            distribution: false,
            platform: false,
            build: false,
            upstream: &[],
        };
        assert_eq!(expected_folder(&film_tool), "film/craftbridge-film-codegen");
        let mut naming = Vec::new();
        check_name(&film_tool, "x", &mut naming);
        assert!(naming.is_empty(), "the naming rule reads the same app: {naming:?}");
    }

    #[test]
    fn a_binary_target_of_another_name_is_reported() {
        let mut renamed = input("craftbridge-effects-codegen", "effects/craftbridge-effects-codegen", Vec::new(), Vec::new());
        renamed.binary_names = vec!["film-codegen".into()];
        let errors = check_crate(&renamed);
        assert!(errors.len() == 1 && errors[0].contains("binary is named `film-codegen`"), "{errors:?}");
        renamed.binary_names = vec!["craftbridge-effects-codegen".into()];
        assert!(check_crate(&renamed).is_empty());
    }

    #[test]
    fn a_craft_app_s_crate_is_known_by_its_repository_whatever_its_name() {
        let from_effectcraft =
            Dependency { source: Some("git+https://github.com/storytold/effectcraft?rev=1037605#1037605".into()), ..external("posterize-bands") };
        let core = check_crate(&input("craftbridge-core", "core/craftbridge-core", vec![from_effectcraft.clone()], Vec::new()));
        assert!(core.iter().any(|e| e.contains("knows nothing about any Craft app")), "{core:?}");
        assert!(check_crate(&input("craftbridge-effects", "effects/craftbridge-effects", vec![from_effectcraft], Vec::new())).is_empty());
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
        let errors = check(&copy.0).unwrap();
        assert_eq!(errors, ["rust/misc/stray/Cargo.toml: a Cargo package that is no workspace member: no stage compiles it and no rule reads it"]);
    }

    #[test]
    fn a_folder_of_the_right_kind_with_another_name_is_reported() {
        let renamed = check_crate(&input("craftbridge-media", "core/media", Vec::new(), Vec::new()));
        assert!(renamed.iter().any(|e| e.contains("lives in `core/media`")), "{renamed:?}");
        // Two crates that swapped folders are each in a folder that is not their name.
        let codegen = check_crate(&input("craftbridge-effects-codegen", "effects/craftbridge-effects-conformance", Vec::new(), Vec::new()));
        let conformance = check_crate(&input("craftbridge-effects-conformance", "effects/craftbridge-effects-codegen", Vec::new(), Vec::new()));
        assert!(codegen.iter().any(|e| e.contains("lives in")) && conformance.iter().any(|e| e.contains("lives in")), "{codegen:?} {conformance:?}");
        let outside = check_crate(&input("craftbridge-core", "", Vec::new(), Vec::new()));
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
        assert!(check_crate(&ffi).is_empty());
        ffi.library_names = vec!["film_ffi".to_string()];
        let errors = check_crate(&ffi);
        assert!(errors.len() == 1 && errors[0].contains("library is named `film_ffi`") && errors[0].contains("`craftbridge_effects_ffi`"), "{errors:?}");
    }

    #[test]
    fn core_to_app_is_forbidden_and_app_to_core_is_allowed() {
        let upward = check_crate(&input("craftbridge-core", "core/craftbridge-core", vec![workspace("craftbridge-effects")], Vec::new()));
        assert!(upward.iter().any(|e| e.contains("the universal core never depends on the effects plugin")), "{upward:?}");
        let downward = check_crate(&input("craftbridge-effects", "effects/craftbridge-effects", vec![workspace("craftbridge-core")], Vec::new()));
        assert!(downward.is_empty(), "{downward:?}");
    }

    #[test]
    fn dev_dependencies_keep_the_isolation_rules_and_nothing_else() {
        // The core's tests are the core's code: they know no app plugin and no Craft app either.
        let core = check_crate(&input(
            "craftbridge-core",
            "core/craftbridge-core",
            vec![dev(workspace("craftbridge-effects")), dev(external("effectcraft-time"))],
            Vec::new(),
        ));
        assert_eq!(core.len(), 2, "{core:?}");
        // A test build ships nothing, so the rank and uniffi rules leave dev-dependencies alone.
        let shipping_rules =
            check_crate(&input("craftbridge-media", "core/craftbridge-media", vec![dev(workspace("craftbridge-core")), dev(external("uniffi"))], Vec::new()));
        assert!(shipping_rules.is_empty(), "{shipping_rules:?}");
        // A tool may reach every app, so a test that took one would reach the plugins and EffectCraft
        // through it: the tool rule holds for tests too.
        let tool = check_crate(&input("craftbridge-media", "core/craftbridge-media", vec![dev(workspace("craftbridge-effects-conformance"))], Vec::new()));
        assert!(tool.len() == 1 && tool[0].contains("nothing depends on a host tool, not even a crate's tests"), "{tool:?}");
    }

    #[test]
    fn a_path_dependency_outside_the_workspace_is_refused() {
        let outside = Dependency {
            name: "craftbridge-effects-helper".into(),
            workspace: None,
            outside: Some("tools/craftbridge-effects-helper".into()),
            dev: false,
            source: None,
        };
        let errors = check_crate(&input("craftbridge-core", "core/craftbridge-core", vec![outside], Vec::new()));
        assert!(
            errors.len() == 1
                && errors[0].contains("depends on craftbridge-effects-helper by a path outside the workspace (`tools/craftbridge-effects-helper`)"),
            "{errors:?}"
        );
        // The workspace's own crate under its name, taken from anywhere but its path, is a copy of it.
        let copy = check_crate(&input("craftbridge-core", "core/craftbridge-core", vec![external("craftbridge-media")], Vec::new()));
        assert!(copy.iter().any(|e| e.contains("from outside the workspace")), "{copy:?}");
    }

    #[test]
    fn nothing_depends_on_a_tool_and_a_tool_depends_on_anything() {
        let errors = check_crate(&input("craftbridge-core", "core/craftbridge-core", vec![workspace("xtask")], Vec::new()));
        assert!(errors.iter().any(|e| e.contains("nothing depends on a host tool")), "{errors:?}");
        let app_errors = check_crate(&input("craftbridge-effects", "effects/craftbridge-effects", vec![workspace("xtask")], Vec::new()));
        assert!(app_errors.iter().any(|e| e.contains("nothing depends on a host tool")), "{app_errors:?}");
        let tool = check_crate(&input(
            "craftbridge-effects-codegen",
            "effects/craftbridge-effects-codegen",
            vec![workspace("craftbridge-effects-ffi"), workspace("xtask"), external("uniffi"), external("effectcraft-host"), external("filmcraft-media")],
            Vec::new(),
        ));
        assert!(tool.is_empty(), "{tool:?}");
    }

    #[test]
    fn layers_point_down_within_a_kind() {
        let upward = check_crate(&input("craftbridge-core", "core/craftbridge-core", vec![workspace("craftbridge-core-ffi")], Vec::new()));
        assert!(upward.iter().any(|e| e.contains("layers point down")), "{upward:?}");
        // An edge to a crate of the same rank, itself included, is refused: it is a cycle in waiting.
        let same_rank = check_crate(&input("craftbridge-core-ffi", "core/craftbridge-core-ffi", vec![workspace("craftbridge-core-ffi")], Vec::new()));
        assert!(same_rank.iter().any(|e| e.contains("layers point down")), "{same_rank:?}");
        let down = check_crate(&input("craftbridge-core", "core/craftbridge-core", vec![workspace("craftbridge-media")], Vec::new()));
        assert!(down.is_empty(), "rank 1 on rank 0 points down: {down:?}");
        let compliant = check_crate(&input(
            "craftbridge-core-ffi",
            "core/craftbridge-core-ffi",
            vec![workspace("craftbridge-core"), workspace("craftbridge-media")],
            Vec::new(),
        ));
        assert!(compliant.is_empty(), "{compliant:?}");
    }

    /// A second app's plugin row, which the shipped table does not have yet.
    const FILM: Entry =
        Entry { name: "craftbridge-film", kind: Kind::App("film"), rank: 0, ffi: false, distribution: false, platform: false, build: false, upstream: &[] };

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
            check_crate(&input(
                "craftbridge-effects-ffi",
                "effects/craftbridge-effects-ffi",
                vec![workspace("craftbridge-effects")],
                vec!["staticlib", "cdylib"]
            ))
            .is_empty()
        );
    }

    #[test]
    fn a_plugin_uses_its_own_craft_app_and_no_sibling() {
        let own = check_crate(&input(
            "craftbridge-effects",
            "effects/craftbridge-effects",
            vec![external("effectcraft-host"), external("craft-docpath"), external("effectcraft")],
            Vec::new(),
        ));
        assert!(own.is_empty(), "{own:?}");
        let sibling = check_crate(&input("craftbridge-effects", "effects/craftbridge-effects", vec![external("filmcraft-media")], Vec::new()));
        assert!(sibling.iter().any(|e| e.contains("its own Craft app's crates")), "{sibling:?}");
        let bare = check_crate(&input("craftbridge-effects", "effects/craftbridge-effects", vec![external("filmcraft")], Vec::new()));
        assert!(bare.iter().any(|e| e.contains("its own Craft app's crates")), "the app's bare crate name is the app's: {bare:?}");
        let mut errors = Vec::new();
        check_external(&FILM, "effectcraft-engine", None, false, "x", &mut errors);
        assert!(errors.iter().any(|e| e.contains("its own Craft app's crates")), "{errors:?}");
    }

    #[test]
    fn the_core_uses_no_craft_app_beyond_its_rows_upstream() {
        let core_upstream = check_crate(&input("craftbridge-core", "core/craftbridge-core", vec![external("effectcraft-host")], Vec::new()));
        assert!(core_upstream.iter().any(|e| e.contains("knows nothing about any Craft app")), "{core_upstream:?}");
        // FilmCraft's decoder registry is where the core's decoders register, so the media crate reads it.
        let media =
            check_crate(&input("craftbridge-media", "core/craftbridge-media", vec![external("filmcraft-codecs"), external("filmcraft-isobmff")], Vec::new()));
        assert!(media.is_empty(), "{media:?}");
        assert!(!check_crate(&input("craftbridge-media", "core/craftbridge-media", vec![external("filmcraft-media")], Vec::new())).is_empty());
        assert!(!check_crate(&input("craftbridge-core", "core/craftbridge-core", vec![external("filmcraft-codecs")], Vec::new())).is_empty());
        assert!(!check_crate(&input("craftbridge-core", "core/craftbridge-core", vec![external("effectcraft")], Vec::new())).is_empty());
        assert!(check_crate(&input("craftbridge-core", "core/craftbridge-core", vec![external("craft-docpath"), external("serde")], Vec::new())).is_empty());
    }

    #[test]
    fn uniffi_appears_only_in_ffi_crates() {
        let outside = check_crate(&input("craftbridge-core", "core/craftbridge-core", vec![external("uniffi")], Vec::new()));
        assert!(outside.iter().any(|e| e.contains("-ffi crates")), "{outside:?}");
        assert!(check_crate(&input("craftbridge-core-ffi", "core/craftbridge-core-ffi", vec![external("uniffi"), external("serde")], Vec::new())).is_empty());
    }

    #[test]
    fn only_the_distribution_crate_becomes_a_native_library() {
        let stray = check_crate(&input("craftbridge-effects", "effects/craftbridge-effects", Vec::new(), vec!["cdylib"]));
        assert!(stray.iter().any(|e| e.contains("staticlib or cdylib")), "{stray:?}");
        assert!(check_crate(&input("craftbridge-effects-ffi", "effects/craftbridge-effects-ffi", Vec::new(), vec!["staticlib", "cdylib", "lib"])).is_empty());
        // The distribution library must build both native kinds, or the app has no native library.
        for half in [vec!["lib"], vec!["staticlib", "lib"], vec!["cdylib", "lib"]] {
            let built = check_crate(&input("craftbridge-effects-ffi", "effects/craftbridge-effects-ffi", Vec::new(), half.clone()));
            assert!(built.iter().any(|e| e.contains("builds both a staticlib")), "{half:?}: {built:?}");
        }
        // A Rust dylib carries its own runtime, which two native libraries in one app would duplicate.
        let dylib = check_crate(&input("craftbridge-core", "core/craftbridge-core", Vec::new(), vec!["dylib"]));
        assert!(dylib.iter().any(|e| e.contains("Rust dylib")), "{dylib:?}");
    }

    #[test]
    fn the_table_keeps_one_native_library_per_app() {
        let ffi = |name: &'static str, distribution: bool| Entry {
            name,
            kind: Kind::App("effects"),
            rank: 1,
            ffi: true,
            distribution,
            platform: false,
            build: false,
            upstream: &[],
        };
        let two = [ffi("craftbridge-effects-ffi", true), ffi("craftbridge-effects-extra-ffi", true)];
        assert!(table_errors(&two).iter().any(|e| e.contains("2 distribution libraries")), "{:?}", table_errors(&two));
        let none = [ffi("craftbridge-effects-ffi", false)];
        assert!(table_errors(&none).iter().any(|e| e.contains("0 distribution libraries")), "{:?}", table_errors(&none));
        let misnamed = [ffi("craftbridge-effects-extra-ffi", true)];
        assert!(table_errors(&misnamed).iter().any(|e| e.contains("is craftbridge-effects-ffi")), "{:?}", table_errors(&misnamed));
        let core =
            [Entry { name: "craftbridge-core", kind: Kind::Core, rank: 1, ffi: false, distribution: true, platform: false, build: false, upstream: &[] }];
        assert!(table_errors(&core).iter().any(|e| e.contains("only an app plugin")), "{:?}", table_errors(&core));
        for errors in [table_errors(&two), table_errors(&none), table_errors(&misnamed), table_errors(&core)] {
            assert!(errors.iter().all(|e| e.starts_with("rust/xtask/src/layers.rs: ")), "a message about a row names the table's file: {errors:?}");
        }
        let unnamed_boundary = [Entry { ffi: true, ..FILM }, Entry { name: "craftbridge-film-ffi", distribution: true, ffi: true, rank: 1, ..FILM }];
        assert!(table_errors(&unnamed_boundary).iter().any(|e| e.contains("is named -ffi")), "{:?}", table_errors(&unnamed_boundary));
        let unknown_app = [Entry { name: "craftbridge-sound-ffi", kind: Kind::App("sound"), distribution: true, ffi: true, ..FILM }];
        assert!(table_errors(&unknown_app).iter().any(|e| e.contains("not a Craft app")), "{:?}", table_errors(&unknown_app));
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
        let dependency = |name: &str, path: Option<&str>| CrateDependency { name: name.into(), path: path.map(str::to_string), source: None, dev: false };
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
