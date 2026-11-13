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

/// Every crate in the workspace, lowest layer first. Add a crate here in the same change that adds it,
/// or the check fails; a row whose crate is gone fails it too.
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
    Entry { name: "craftbridge-core", kind: Kind::Core, rank: 1, ffi: false, distribution: false, platform: false, build: false, upstream: &[] },
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

/// Every app noun a table uses, so a tool's folder is checked against the apps the table knows
/// instead of a second hard-coded list.
fn app_nouns(table: &[Entry]) -> Vec<&'static str> {
    let mut nouns: Vec<&'static str> = table.iter().filter_map(|row| if let Kind::App(noun) = row.kind { Some(noun) } else { None }).collect();
    nouns.sort_unstable();
    nouns.dedup();
    nouns
}

/// A Craft app: the noun its plugin's crates carry and the prefix of its own crates.
pub(crate) struct CraftApp {
    pub(crate) noun: &'static str,
    pub(crate) crates: &'static str,
}

/// The Craft apps whose crates can reach this workspace: EffectCraft, which the effects plugin wraps, and
/// FilmCraft, whose codecs EffectCraft decodes through. A plugin uses its own app's crates and no other's.
pub(crate) const CRAFT_APPS: &[CraftApp] = &[CraftApp { noun: "effects", crates: "effectcraft-" }, CraftApp { noun: "film", crates: "filmcraft-" }];

/// The shared Craft libraries (storytold/craft-libs), which belong to no app and serve every plugin.
pub(crate) const CRAFT_LIBRARY_PREFIX: &str = "craft-";

/// The Craft app whose crate `name` is, if it is one.
fn craft_app_of(name: &str) -> Option<&'static CraftApp> {
    CRAFT_APPS.iter().find(|app| name.starts_with(app.crates))
}

/// A dependency one package declares.
#[derive(Clone, Debug)]
pub(crate) struct Dependency {
    pub(crate) name: String,
    /// The workspace crate it points at, when it points at one.
    pub(crate) workspace: Option<String>,
}

/// A package reduced to what the layer rules read. A struct rather than a tuple because the types would
/// say nothing at a call site. Owned, so a test can build one from literals and the real check can build
/// one from `cargo metadata` without either side fighting a lifetime.
#[derive(Clone, Debug)]
pub(crate) struct CrateInput {
    pub(crate) name: String,
    /// The crate's folder relative to `rust/`, `/` separated (`core/craftbridge-core`).
    pub(crate) folder: String,
    pub(crate) dependencies: Vec<Dependency>,
    /// `crate_types` of the package's library targets.
    pub(crate) crate_types: Vec<String>,
}

/// The folder the table says a crate lives in: `<app>/<name>` for an app plugin, `core/<name>` for the
/// core, and for a tool either `<app>/<name>` when it is an app's tool or `<name>` directly under
/// `rust/`. Either way the folder name is the crate's name.
fn expected_folders(row: &Entry) -> Vec<String> {
    match row.kind {
        Kind::Core => vec![format!("core/{}", row.name)],
        Kind::App(app) => vec![format!("{app}/{}", row.name)],
        Kind::Tool => {
            let mut folders = vec![row.name.to_string()];
            folders.extend(
                app_nouns(ENTRIES).into_iter().filter(|app| row.name.starts_with(&format!("craftbridge-{app}-"))).map(|app| format!("{app}/{}", row.name)),
            );
            folders
        }
    }
}

/// Check one package: its folder, its name, every edge and its native-library targets.
pub(crate) fn check_crate(input: &CrateInput) -> Vec<String> {
    let Some(row) = entry(&input.name) else {
        return vec![format!("{}: not in the layer table (rust/xtask/src/layers.rs)", input.name)];
    };
    let mut errors = Vec::new();
    let folders = expected_folders(row);
    if !folders.iter().any(|folder| folder == &input.folder) {
        errors.push(format!(
            "{}: lives in `{}`, the table says {} (a crate's folder is its name, under its kind)",
            input.name,
            input.folder,
            folders.join(" or ")
        ));
    }
    check_name(row, &mut errors);
    for dependency in &input.dependencies {
        match &dependency.workspace {
            Some(target_name) => match entry(target_name) {
                Some(target) => check_edge(row, target, &mut errors),
                None => errors.push(format!("{} depends on {target_name}: {target_name} is not in the layer table (rust/xtask/src/layers.rs)", row.name)),
            },
            None => check_external(row, &dependency.name, &mut errors),
        }
    }
    if builds_native(&input.crate_types) && !row.distribution {
        errors.push(format!("{} is built as a staticlib or cdylib: only the distribution library the table marks may be", row.name));
    }
    errors
}

/// A core crate is `craftbridge-<area>`; an app plugin carries its app's noun, which stops one app's
/// crate being mistaken for another's. A tool keeps the name its convention gives it (`xtask`).
fn check_name(row: &Entry, errors: &mut Vec<String>) {
    match row.kind {
        Kind::Core => {
            if !row.name.starts_with("craftbridge-") {
                errors.push(format!("{}: a core crate is named craftbridge-<area>", row.name));
            }
        }
        Kind::App(app) => {
            let prefixed = format!("craftbridge-{app}-");
            if row.name != format!("craftbridge-{app}") && !row.name.starts_with(&prefixed) {
                errors.push(format!("{}: a {} plugin crate is named craftbridge-{app} or craftbridge-{app}-…", row.name, app));
            }
        }
        Kind::Tool => {}
    }
}

/// One edge between two workspace crates.
fn check_edge(row: &Entry, target: &Entry, errors: &mut Vec<String>) {
    match (&row.kind, &target.kind) {
        // A tool may depend on anything; nothing may depend on a tool.
        (Kind::Tool, _) => {}
        (_, Kind::Tool) => errors.push(edge(row, target, "nothing depends on a host tool")),
        (Kind::Core, Kind::App(app)) => errors.push(edge(row, target, format!("the universal core never depends on the {app} plugin"))),
        (Kind::App(other), Kind::App(app)) if other != app => {
            errors.push(edge(row, target, format!("an app plugin never depends on another app's plugin ({other} on {app})")))
        }
        // Within one kind, and only within it, ranks order the edges; the same rank is a cycle waiting
        // to happen, so equality is not allowed either.
        (Kind::Core, Kind::Core) | (Kind::App(_), Kind::App(_)) if target.rank >= row.rank => {
            errors.push(edge(row, target, format!("layers point down: {} is rank {} and {} is rank {}", row.name, row.rank, target.name, target.rank)));
        }
        _ => {}
    }
}

fn edge(row: &Entry, target: &Entry, reason: impl AsRef<str>) -> String {
    format!("{} depends on {}: {}", row.name, target.name, reason.as_ref())
}

/// An external crate: `uniffi` only at a UniFFI boundary; no Craft app's crates in the universal core
/// beyond the row's `upstream`; and in an app plugin, its own Craft app's crates and no sibling's. A tool
/// may depend on anything.
fn check_external(row: &Entry, name: &str, errors: &mut Vec<String>) {
    if row.kind == Kind::Tool {
        return;
    }
    if name.starts_with("uniffi") && !row.ffi {
        errors.push(format!("{} depends on {name}: uniffi appears only in -ffi crates (rust/xtask/src/layers.rs)", row.name));
    }
    let Some(app) = craft_app_of(name) else { return };
    match row.kind {
        Kind::Core if !row.upstream.contains(&name) => {
            errors.push(format!("{} depends on {name}: the universal core knows nothing about any Craft app", row.name));
        }
        Kind::App(noun) if noun != app.noun => {
            errors
                .push(format!("{} depends on {name}: an app plugin uses only its own Craft app's crates; code two plugins need moves into the core", row.name));
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
    for row in table {
        if row.ffi != row.name.ends_with("-ffi") {
            errors.push(format!("{}: a UniFFI boundary, and only one, is named -ffi", row.name));
        }
        if !row.upstream.is_empty() && row.kind != Kind::Core {
            errors.push(format!("{}: only a core crate lists the Craft app crates it may use; a plugin uses its own app's", row.name));
        }
        match row.kind {
            Kind::App(noun) if row.distribution && row.name != format!("craftbridge-{noun}-ffi") => {
                errors.push(format!("{}: the {noun} plugin's distribution library is craftbridge-{noun}-ffi", row.name));
            }
            Kind::App(_) => {}
            Kind::Core | Kind::Tool if row.distribution => errors.push(format!("{}: only an app plugin has a distribution library", row.name)),
            Kind::Core | Kind::Tool => {}
        }
    }
    for noun in app_nouns(table) {
        if !CRAFT_APPS.iter().any(|app| app.noun == noun) {
            errors.push(format!("{noun}: not a Craft app this table knows (CRAFT_APPS in rust/xtask/src/layers.rs)"));
        }
        let libraries = table.iter().filter(|row| row.kind == Kind::App(noun) && row.distribution).count();
        if libraries != 1 {
            errors.push(format!("the {noun} plugin has {libraries} distribution libraries: an app has exactly one native library"));
        }
    }
    errors
}

/// A table row whose folder is not on disk, so a crate renamed or deleted cannot leave a dead row.
fn missing_folders(rust_dir: &Path) -> Vec<String> {
    ENTRIES
        .iter()
        .flat_map(|row| {
            let folders = expected_folders(row);
            let found = folders.iter().any(|folder| rust_dir.join(folder).is_dir());
            if found { Vec::new() } else { vec![format!("{}: the table says {}, but there is no such folder under rust/", row.name, folders.join(" or "))] }
        })
        .collect()
}

/// A manifest's folder relative to `rust/`, `/` separated, or `None` outside it.
fn manifest_folder(rust_dir: &Path, manifest: &str) -> Option<String> {
    let folder = Path::new(manifest).parent()?.strip_prefix(rust_dir).ok()?;
    Some(folder.to_string_lossy().replace('\\', "/"))
}

/// Reduce `cargo metadata` to the rules' input. A dependency belongs to the workspace when its declared
/// path is the folder of a workspace package's manifest; a dependency without a path, or with one
/// outside the workspace, is external and is judged by its name.
fn inputs_from(crates: &[Crate], rust_dir: &Path) -> Vec<CrateInput> {
    let owner_at = |path: &str| {
        crates.iter().find(|candidate| Path::new(&candidate.manifest_path).parent() == Some(Path::new(path))).map(|candidate| candidate.name.clone())
    };
    crates
        .iter()
        .map(|package| CrateInput {
            name: package.name.clone(),
            folder: manifest_folder(rust_dir, &package.manifest_path).unwrap_or_default(),
            dependencies: package
                .dependencies
                .iter()
                .map(|dependency| Dependency { name: dependency.name.clone(), workspace: dependency.path.as_deref().and_then(owner_at) })
                .collect(),
            crate_types: package.library_targets.iter().flat_map(|target| target.crate_types.iter().cloned()).collect(),
        })
        .collect()
}

/// Run the check against the workspace on disk: read the graph, apply every rule, and make sure the
/// table still matches the crates that exist.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let rust_dir = root.join("rust");
    let crates = crate::meta::read(&rust_dir, true)?;
    let mut errors = table_errors(ENTRIES);
    for input in &inputs_from(&crates, &rust_dir) {
        errors.extend(check_crate(input));
    }
    // A crate in the workspace but not in the table is reported by check_crate; a table row with no
    // crate is reported here.
    errors.extend(missing_folders(&rust_dir));
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{CrateDependency, CrateTarget};
    use crate::scratch::Scratch;

    fn workspace(name: &str) -> Dependency {
        Dependency { name: name.to_string(), workspace: Some(name.to_string()) }
    }

    fn external(name: &str) -> Dependency {
        Dependency { name: name.to_string(), workspace: None }
    }

    fn input(name: &str, folder: &str, dependencies: Vec<Dependency>, crate_types: Vec<&str>) -> CrateInput {
        CrateInput { name: name.to_string(), folder: folder.to_string(), dependencies, crate_types: crate_types.iter().map(|kind| kind.to_string()).collect() }
    }

    /// The table as its rows must satisfy it with no edges at all: this is what a fresh skeleton must pass.
    #[test]
    fn every_table_row_satisfies_its_own_rules() {
        assert!(table_errors(ENTRIES).is_empty(), "{:?}", table_errors(ENTRIES));
        for row in ENTRIES {
            let folder = expected_folders(row).remove(0);
            let errors = check_crate(&input(row.name, &folder, Vec::new(), vec!["lib"]));
            assert!(errors.is_empty(), "{}: {errors:?}", row.name);
        }
    }

    /// Every table row has its folder on disk.
    #[test]
    fn the_table_matches_the_workspace_on_disk() {
        let errors = missing_folders(&crate::repo_root().join("rust"));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn an_unlisted_crate_is_reported() {
        let errors = check_crate(&input("craftbridge-mystery", "core/craftbridge-mystery", Vec::new(), Vec::new()));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("not in the layer table"), "{errors:?}");
    }

    #[test]
    fn a_table_row_without_a_folder_is_reported() {
        // A folder tree missing one row's folder reports that row, and only it.
        let scratch = Scratch::new("complete");
        for row in ENTRIES.iter().skip(1) {
            scratch.mkdir(&expected_folders(row).remove(0));
        }
        let errors = missing_folders(&scratch.0);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains(ENTRIES[0].name), "{errors:?}");
        // And a folder tree holding every row finds nothing, which is the compliant case.
        scratch.mkdir(&expected_folders(&ENTRIES[0]).remove(0));
        assert!(missing_folders(&scratch.0).is_empty());
    }

    #[test]
    fn a_crate_in_the_wrong_folder_is_reported() {
        let errors = check_crate(&input("craftbridge-core", "effects/craftbridge-core", Vec::new(), Vec::new()));
        assert!(errors.iter().any(|e| e.contains("core/craftbridge-core")), "{errors:?}");
        // An app's tool may live under its app; a tool that is not an app's tool must sit at the top.
        assert!(check_crate(&input("craftbridge-effects-codegen", "effects/craftbridge-effects-codegen", Vec::new(), Vec::new())).is_empty());
        assert!(!check_crate(&input("xtask", "core/xtask", Vec::new(), Vec::new())).is_empty());
    }

    #[test]
    fn an_app_crate_must_carry_its_app_noun() {
        let effects = entry("craftbridge-effects").unwrap();
        assert!(check_name_of(effects, "craftbridge-effects").is_empty() && check_name_of(effects, "craftbridge-effects-ffi").is_empty());
        assert!(check_name_of(effects, "craftbridge-other").iter().any(|e| e.contains("effects plugin crate is named")));
    }

    #[test]
    fn a_core_crate_must_carry_the_craftbridge_prefix() {
        let core = entry("craftbridge-core").unwrap();
        assert!(check_name_of(core, "craftbridge-new-thing").is_empty());
        assert!(check_name_of(core, "core-mystery").iter().any(|e| e.contains("craftbridge-<area>")));
    }

    #[test]
    fn core_to_app_is_forbidden_and_app_to_core_is_allowed() {
        let upward = check_crate(&input("craftbridge-core", "core/craftbridge-core", vec![workspace("craftbridge-effects")], Vec::new()));
        assert!(upward.iter().any(|e| e.contains("the universal core never depends on the effects plugin")), "{upward:?}");
        let downward = check_crate(&input("craftbridge-effects", "effects/craftbridge-effects", vec![workspace("craftbridge-core")], Vec::new()));
        assert!(downward.is_empty(), "{downward:?}");
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
        check_edge(effects, &FILM, &mut errors);
        check_edge(&FILM, effects, &mut errors);
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
            vec![external("effectcraft-host"), external("craft-docpath")],
            Vec::new(),
        ));
        assert!(own.is_empty(), "{own:?}");
        let sibling = check_crate(&input("craftbridge-effects", "effects/craftbridge-effects", vec![external("filmcraft-media")], Vec::new()));
        assert!(sibling.iter().any(|e| e.contains("its own Craft app's crates")), "{sibling:?}");
        let mut errors = Vec::new();
        check_external(&FILM, "effectcraft-engine", &mut errors);
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
        let unnamed_boundary = [Entry { ffi: true, ..FILM }, Entry { name: "craftbridge-film-ffi", distribution: true, ffi: true, rank: 1, ..FILM }];
        assert!(table_errors(&unnamed_boundary).iter().any(|e| e.contains("is named -ffi")), "{:?}", table_errors(&unnamed_boundary));
        let unknown_app = [Entry { name: "craftbridge-sound-ffi", kind: Kind::App("sound"), distribution: true, ffi: true, ..FILM }];
        assert!(table_errors(&unknown_app).iter().any(|e| e.contains("not a Craft app")), "{:?}", table_errors(&unknown_app));
    }

    /// `check_name` against a row and a name that may differ from the row's own, so a test can ask
    /// what the naming rule says about a crate the table does not contain verbatim.
    fn check_name_of(row: &'static Entry, name: &'static str) -> Vec<String> {
        let mut errors = Vec::new();
        check_name(&Entry { name, ..*row }, &mut errors);
        errors
    }

    #[test]
    fn folders_and_workspace_edges_come_from_manifest_paths() {
        let crates = vec![
            Crate {
                name: "craftbridge-core".into(),
                id: "id1".into(),
                source: None,
                manifest_path: "/repo/rust/core/craftbridge-core/Cargo.toml".into(),
                dependencies: vec![
                    CrateDependency { name: "serde".into(), path: None },
                    CrateDependency { name: "craftbridge-media".into(), path: Some("/repo/rust/core/craftbridge-media".into()) },
                    CrateDependency { name: "effectcraft-time".into(), path: Some("/elsewhere/effectcraft/crates/time".into()) },
                ],
                library_targets: vec![CrateTarget { crate_types: vec!["lib".into()] }],
            },
            Crate {
                name: "craftbridge-media".into(),
                id: "id2".into(),
                source: None,
                manifest_path: "/repo/rust/core/craftbridge-media/Cargo.toml".into(),
                dependencies: Vec::new(),
                library_targets: Vec::new(),
            },
        ];
        let inputs = inputs_from(&crates, Path::new("/repo/rust"));
        assert_eq!(inputs[0].folder, "core/craftbridge-core");
        assert_eq!(inputs[0].dependencies[0].workspace, None, "an external dependency points at no workspace crate");
        assert_eq!(inputs[0].dependencies[1].workspace.as_deref(), Some("craftbridge-media"));
        assert_eq!(inputs[0].dependencies[2].workspace, None, "a path outside the workspace is external");
        assert_eq!(inputs[0].crate_types, ["lib"]);
        assert_eq!(manifest_folder(Path::new("/repo/rust"), "/repo/rust-other/x/Cargo.toml"), None);
    }

    #[test]
    fn app_nouns_come_from_the_rows() {
        assert_eq!(app_nouns(ENTRIES), vec!["effects"]);
    }
}
