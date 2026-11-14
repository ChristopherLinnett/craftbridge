//! Rule 3 and rule 10 — the inherited, unweakened lint table, and `panic = "unwind"`.
//!
//! Never crash outranks everything except not losing work, so the panicking shortcuts are denied once
//! in the workspace manifest and every crate inherits them; the five with no exception are `forbid`,
//! which makes the compiler itself refuse any `allow` of them, however spelt. The rule is only real if
//! nothing can weaken it: not the inheriting table (`[lints]` must say `workspace = true` and nothing else); not the
//! workspace table itself, where a lint group allowed at a later priority (`clippy::restriction` holds
//! every denied lint) switches them all off, and where a second key for the same lint (`unwrap-used`, or
//! the former name `option_unwrap_used`) is passed to the compiler as well; not a crate or module
//! attribute (`#![allow(…)]`, also when it is spread over lines or nested in `cfg_attr`, and `#[allow(…)]`
//! on a `mod`, which covers the whole module); not clippy's configuration, whose `allow-unwrap-types`
//! lets `unwrap` and `expect` through on whole types and of which clippy reads only the file nearest a
//! crate; and not Cargo's own configuration in `rust/.cargo/`, whose `rustflags` come after the table,
//! whose profiles override the manifest's, whose compiler wrappers sit between Cargo and the compiler,
//! whose target runners stand in for the test and xtask binaries, whose `[alias]` can stand in for
//! `cargo clippy` or `cargo xtask`, and whose `[env]` reaches clippy; not a build script, which runs
//! before the compiler takes the table and can hand it flags and cfgs through `cargo::rustc-env` and
//! `cargo::rustc-cfg`, so a crate has one only when the layer table marks it; and not a file the compiler
//! reads that no rule does, which the compiler's own dep-info decides (`cargo xtask compiled`, run by every
//! cargo stage of `tools/ci.sh`). A profile that aborts is
//! refused wherever it is set: the engine's last-resort guard catches a panic on a Craft thread, which
//! needs unwinding to exist at all.
//!
//! The two levels that count are `deny` and `forbid`; `warn` and `allow` do not stop a change, so a
//! weaker level is reported as missing. Both spellings of a level are accepted: the string form and the
//! `{ level = "deny", priority = N }` table form.

use crate::{attributes, machinery, scan};
use std::path::Path;

/// The clippy lints that forbid panicking, the two that name the same calls by path (and reach a macro's
/// expansion and a method taken as a value), and the one that keeps every `unsafe` block documented.
pub(crate) const DENIED_CLIPPY_LINTS: &[&str] =
    &["unwrap_used", "expect_used", "panic", "unimplemented", "todo", "unreachable", "disallowed_methods", "disallowed_macros", "undocumented_unsafe_blocks"];

/// The denied lints with no exception at all, which the table sets to `forbid`: the compiler then refuses
/// every `allow` of them in any file it reads, which no reading of the source can promise. Tests keep
/// `unwrap`, `expect` and `panic!` through clippy's own `allow-*-in-tests`, which a `forbid` does not touch.
const FORBIDDEN_CLIPPY_LINTS: &[&str] = &["unwrap_used", "panic", "unimplemented", "todo", "unreachable"];

/// The one denied lint an item may allow, on a provably infallible literal (AGENTS.md §3), and the lint
/// that names the same `.expect` by path, which the item allows beside it and never alone.
const ITEM_EXCEPTION: &str = "expect_used";
const ITEM_EXCEPTION_BY_PATH: &str = "disallowed_methods";

/// The clippy groups that hold the lints above, so switching a group off switches them off: `restriction`
/// holds the never-crash lints, `style` (and `all`, which holds it) the two by path.
const DENIED_CLIPPY_GROUPS: &[&str] = &["restriction", "style", "all"];

/// The groups of lints on by default, which `-D warnings` makes errors: a table entry that allows one
/// switches off every lint in it at once.
const DEFAULT_CLIPPY_GROUPS: &[&str] = &["all", "correctness", "suspicious", "style", "complexity", "perf"];

/// Former names of the denied lints, and the lint each now is. The compiler still applies a former name
/// to the lint it became, with only a warning, so an `allow` under the old name is an `allow` of the lint.
const FORMER_CLIPPY_NAMES: &[(&str, &str)] = &[
    ("option_unwrap_used", "unwrap_used"),
    ("result_unwrap_used", "unwrap_used"),
    ("option_expect_used", "expect_used"),
    ("result_expect_used", "expect_used"),
];

/// The lint a table key or an attribute item (without its `clippy::` prefix) reaches: the compiler reads
/// `-` in a lint name as `_`, and a former name as the lint it became.
fn canonical_lint(name: &str) -> String {
    let name = name.replace('-', "_");
    FORMER_CLIPPY_NAMES.iter().find(|(former, _)| *former == name).map_or(name, |(_, current)| (*current).to_string())
}

/// Every entry of a lint table that reaches `lint`, under whatever key it is written: Cargo passes each
/// key to the compiler as written, so `unwrap_used = "deny"` beside `unwrap-used = "allow"` at a later
/// priority is two flags for one lint, and the later one wins.
fn entries_reaching<'a>(table: Option<&'a toml::map::Map<String, toml::Value>>, lint: &str) -> Vec<(&'a str, &'a toml::Value)> {
    table.into_iter().flatten().filter(|(key, _)| canonical_lint(key) == lint).map(|(key, value)| (key.as_str(), value)).collect()
}

/// A table entry's level, in either spelling: `"deny"` or `{ level = "deny", priority = N }`.
fn level_of(value: Option<&toml::Value>) -> Option<&str> {
    match value {
        Some(toml::Value::String(level)) => Some(level.as_str()),
        Some(toml::Value::Table(table)) => table.get("level").and_then(toml::Value::as_str),
        _ => None,
    }
}

/// Is a lint level strong enough to be a gate?
fn level_is_denied(value: Option<&toml::Value>) -> bool {
    level_of(value).is_some_and(|level| level == "deny" || level == "forbid")
}

/// Does an attribute's lint item (`clippy::unwrap_used`, `clippy::restriction`) reach a denied lint? One
/// a macro fills in might, so it does.
fn reaches_denied_lint(item: &str) -> bool {
    item == attributes::UNREADABLE
        || item
            .strip_prefix("clippy::")
            .map(canonical_lint)
            .is_some_and(|name| DENIED_CLIPPY_GROUPS.contains(&name.as_str()) || DENIED_CLIPPY_LINTS.contains(&name.as_str()))
}

/// Parse a TOML file, turning a parse failure into a named error rather than a panic or an `unwrap`.
fn parse(source: &str, name: &str) -> Result<toml::Value, Vec<String>> {
    toml::from_str::<toml::Value>(source).map_err(|error| vec![format!("{name}: is not valid TOML: {error}")])
}

/// Check the workspace manifest: the denied lints, no group that switches them off, and no profile that
/// aborts instead of unwinding.
///
/// `source` is the manifest text and `name` how a message should call it, so a test can feed strings.
pub(crate) fn workspace_manifest_violations(source: &str, name: &str) -> Vec<String> {
    let document = match parse(source, name) {
        Ok(document) => document,
        Err(errors) => return errors,
    };
    let mut errors = Vec::new();
    let lints = document.get("workspace").and_then(toml::Value::as_table).and_then(|workspace| workspace.get("lints"));
    let rust_lints = lints.and_then(|lints| lints.get("rust")).and_then(toml::Value::as_table);
    let clippy_lints = lints.and_then(|lints| lints.get("clippy")).and_then(toml::Value::as_table);
    let required = std::iter::once(("rust", rust_lints, "unsafe_code", "unsafe is a decision, not a default"))
        .chain(DENIED_CLIPPY_LINTS.iter().map(|lint| ("clippy", clippy_lints, *lint, "never crash: no panicking shortcuts")));
    for (tool, table, lint, why) in required {
        let entries = entries_reaching(table, lint);
        if entries.is_empty() {
            errors.push(format!("{name}: [workspace.lints.{tool}].{lint} must be deny or forbid ({why})"));
        }
        for (key, _) in entries.iter().filter(|(_, level)| !level_is_denied(Some(level))) {
            errors.push(format!("{name}: [workspace.lints.{tool}].{key} must be deny or forbid ({why})"));
        }
        if tool == "clippy" && FORBIDDEN_CLIPPY_LINTS.contains(&lint) {
            for (key, _) in entries.iter().filter(|(_, level)| level_is_denied(Some(level)) && level_of(Some(level)) != Some("forbid")) {
                errors.push(format!("{name}: [workspace.lints.clippy].{key} must be forbid: it has no exception, so no allow of it may compile"));
            }
        }
    }
    for (key, _) in entries_reaching(clippy_lints, "restriction").iter().filter(|(_, level)| !level_is_denied(Some(level))) {
        errors.push(format!("{name}: [workspace.lints.clippy].{key} below deny switches off every denied lint the group holds"));
    }
    let defaults = DEFAULT_CLIPPY_GROUPS.iter().map(|group| ("clippy", clippy_lints, *group)).chain(std::iter::once(("rust", rust_lints, "warnings")));
    for (tool, table, group) in defaults {
        for (key, _) in entries_reaching(table, group).iter().filter(|(_, level)| level_of(Some(level)) == Some("allow")) {
            errors.push(format!("{name}: [workspace.lints.{tool}].{key} = allow switches off every lint the group holds, which `-D warnings` makes errors"));
        }
    }
    errors.extend(panic_violations(&document, name));
    errors
}

/// Does any profile abort? A crate-level override counts too: `[profile.release.package.foo]` with
/// `panic = "abort"` aborts just as dead as the profile-wide setting. Only `unwind` is accepted, so a
/// strategy Cargo adds later is refused until someone decides it keeps the guard working.
fn panic_violations(document: &toml::Value, name: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(profiles) = document.get("profile").and_then(toml::Value::as_table) else { return errors };
    let mut settings: Vec<(String, &toml::Value)> = Vec::new();
    for (profile, table) in profiles {
        settings.push((format!("[profile.{profile}]"), table));
        for (package, overrides) in table.get("package").and_then(toml::Value::as_table).into_iter().flatten() {
            settings.push((format!("[profile.{profile}.package.{package}]"), overrides));
        }
    }
    for (table_name, table) in settings {
        if let Some(strategy) = table.get("panic").and_then(toml::Value::as_str).filter(|strategy| *strategy != "unwind") {
            errors.push(format!("{name}: {table_name} sets panic = \"{strategy}\": the engine's last-resort guard catches panics, which needs unwinding"));
        }
    }
    errors
}

/// Check Cargo's configuration: no profile that aborts, no `rustflags` that set a lint level or a panic
/// strategy, which would override what the workspace table and its profile say, no compiler wrapper, no
/// alias that stands in for a command the gate runs, and no `[env]` entry for clippy, which reads its lint
/// flags (`CLIPPY_ARGS`) and finds its configuration (`CLIPPY_CONF_DIR`) through the environment Cargo
/// gives it.
pub(crate) fn cargo_config_violations(source: &str, name: &str) -> Vec<String> {
    let document = match parse(source, name) {
        Ok(document) => document,
        Err(errors) => return errors,
    };
    let mut errors = panic_violations(&document, name);
    // The compiler binary and its wrappers are the toolchain's, not the repository's: a wrapper sits
    // between Cargo and rustc (and clippy-driver) and can strip `-D warnings` and every denied lint
    // before they arrive, exactly the bypass the rest of this check refuses.
    if let Some(build) = document.get("build") {
        for key in ["rustc", "rustc-wrapper", "rustc-workspace-wrapper"] {
            if build.get(key).is_some() {
                errors.push(format!("{name}: [build].{key} points the compiler or its wrapper at code no rule reads: the denied lints and `-D warnings` must reach rustc untouched"));
            }
        }
    }
    for (alias, value) in document.get("alias").and_then(toml::Value::as_table).into_iter().flatten() {
        if let Some(violation) = alias_violation(alias, value) {
            errors.push(format!("{name}: [alias].{alias} {violation}"));
        }
    }
    for variable in document.get("env").and_then(toml::Value::as_table).into_iter().flatten().map(|(variable, _)| variable) {
        if variable.starts_with("CLIPPY_") {
            errors.push(format!(
                "{name}: [env].{variable} reaches clippy: its lint flags and its configuration come only from rust/Cargo.toml and rust/clippy.toml"
            ));
        }
    }
    let mut flag_lists: Vec<(String, &toml::Value)> = Vec::new();
    if let Some(flags) = document.get("build").and_then(|build| build.get("rustflags")) {
        flag_lists.push(("[build].rustflags".to_string(), flags));
    }
    for (target, table) in document.get("target").and_then(toml::Value::as_table).into_iter().flatten() {
        if let Some(flags) = table.get("rustflags") {
            flag_lists.push((format!("[target.{target}].rustflags"), flags));
        }
        if table.get("runner").is_some() {
            errors.push(format!(
                "{name}: [target.{target}].runner runs in place of every binary Cargo builds, so `cargo test` and the rule checks would pass having run nothing"
            ));
        }
    }
    for (key, flags) in flag_lists {
        let tokens: Vec<String> = match flags {
            toml::Value::String(line) => line.split_whitespace().map(str::to_string).collect(),
            toml::Value::Array(items) => items.iter().filter_map(toml::Value::as_str).flat_map(str::split_whitespace).map(str::to_string).collect(),
            _ => Vec::new(),
        };
        for token in tokens.iter().filter(|token| overrides_the_table(token)) {
            errors.push(format!("{name}: {key} passes `{token}`: lint levels and the panic strategy come only from rust/Cargo.toml"));
        }
    }
    errors
}

/// The cargo subcommands the gate runs that Cargo does not build in. Cargo ignores an alias named after
/// one of its own commands, but an alias named after one of these runs in its place, with only a warning:
/// `clippy = "test --no-run"` turns the lint stage into a build that reads no lint.
const GATE_EXTERNAL_COMMANDS: &[&str] = &["clippy", "fmt", "deny"];

/// What is wrong with one `[alias]` entry, if anything: it may not stand in for a command the gate runs,
/// and `xtask`, the alias a person runs the rule checks through, must run the xtask package, or
/// `cargo xtask rules` would pass having checked nothing (`xtask = "test --no-run"` reads `rules` as a test
/// filter). The gate itself builds the package and runs its binary, which neither an alias nor a runner
/// can stand in for.
fn alias_violation(alias: &str, value: &toml::Value) -> Option<&'static str> {
    if GATE_EXTERNAL_COMMANDS.contains(&alias) {
        return Some("stands in for a command the gate runs, so the stage would run something else under its name");
    }
    let words: Vec<&str> = match value {
        toml::Value::String(line) => line.split_whitespace().collect(),
        toml::Value::Array(items) => items.iter().filter_map(toml::Value::as_str).collect(),
        _ => Vec::new(),
    };
    let runs_xtask = words.first() == Some(&"run") && words.windows(2).any(|pair| matches!(pair, ["--package" | "-p", "xtask"]));
    (alias == "xtask" && !runs_xtask).then_some("must run the xtask package, or `cargo xtask rules` checks nothing")
}

/// Is one `rustflags` token a lint level or a panic strategy? Levels are `-A`, `-W`, `--allow`, `--warn`,
/// `--force-warn` and `--cap-lints`, each glued to its lint or followed by it; any of them can only lower
/// what the table denies. A panic strategy appears as `panic=` after `-C` or `--codegen`.
fn overrides_the_table(token: &str) -> bool {
    ["-A", "-W", "--allow", "--warn", "--force-warn", "--cap-lints"].iter().any(|flag| token.starts_with(flag)) || token.contains("panic=")
}

/// Check clippy's configuration: `allow-unwrap-types` lets `unwrap()` and `expect()` through on every
/// value of the types it lists, which switches the denied lints off for them. Its other settings that
/// touch a denied lint allow it only in tests, or in code evaluated while compiling, where a panic is a
/// build error rather than a crash, or say where a safety comment may sit. That one configuration is
/// `rust/clippy.toml` alone: a second copy is the machinery check's to refuse, because clippy reads only
/// the file nearest a crate and a copy would replace this one below it.
pub(crate) fn clippy_config_violations(source: &str, name: &str) -> Vec<String> {
    let document = match parse(source, name) {
        Ok(document) => document,
        Err(errors) => return errors,
    };
    match document.get("allow-unwrap-types").and_then(toml::Value::as_array) {
        Some(types) if !types.is_empty() => {
            vec![format!("{name}: allow-unwrap-types lets unwrap() and expect() through on whole types, which the workspace denies")]
        }
        _ => Vec::new(),
    }
}

/// Does a package manifest inherit the workspace lints and nothing else?
pub(crate) fn package_lints_violations(source: &str, name: &str) -> Vec<String> {
    let document = match parse(source, name) {
        Ok(document) => document,
        Err(errors) => return errors,
    };
    let Some(lints) = document.get("lints").and_then(toml::Value::as_table) else {
        return vec![format!("{name}: no [lints] table, so it does not inherit the workspace lints")];
    };
    let mut errors = Vec::new();
    match lints.get("workspace") {
        Some(toml::Value::Boolean(true)) => {}
        Some(_) => errors.push(format!("{name}: [lints] must say workspace = true, nothing weaker")),
        None => errors.push(format!("{name}: [lints] must inherit the workspace lints with workspace = true")),
    }
    for key in lints.keys() {
        if key != "workspace" {
            errors.push(format!("{name}: [lints] may only inherit, and `{key}` adds or overrides a lint here"));
        }
    }
    errors
}

/// Does the manifest inherit the workspace edition the way it inherits the lint table — with
/// `edition.workspace = true`, beside `[lints] workspace = true`? What the compiler accepts, and which
/// lints even exist to apply, change between editions, so a crate may not pick one for itself; the old
/// `[project]` table inherits as well, since Cargo reads it wherever it reads the edition. Which edition
/// a crate actually settles on is Cargo's own answer, held to the workspace's by the rule that reads
/// `cargo metadata`. A target's table writes no edition at all: writing one sets the target's edition
/// even when it writes the workspace's own value, and a pin spelt that way is invisible to Cargo's
/// answer until the day the workspace edition moves — only the spelling shows it, so the spelling is
/// what is refused.
pub(crate) fn package_edition_violations(source: &str, name: &str) -> Vec<String> {
    let document = match parse(source, name) {
        Ok(document) => document,
        Err(errors) => return errors,
    };
    let mut errors = Vec::new();
    let declared: Vec<&toml::Value> =
        ["package", "project"].into_iter().filter_map(|table| document.get(table).and_then(|table| table.get("edition"))).collect();
    if !declared.iter().any(|edition| edition.get("workspace").and_then(toml::Value::as_bool) == Some(true)) {
        let spelled =
            declared.iter().filter_map(|edition| edition.as_str()).map(|edition| format!(" — it says edition = \"{edition}\"")).next().unwrap_or_default();
        errors.push(format!("{name}: does not inherit the workspace edition{spelled}: a crate says edition.workspace = true beside [lints] workspace = true"));
    }
    for (at, target) in target_tables(&document) {
        if let Some(edition) = target.get("edition") {
            let value = edition.as_str().map_or_else(|| edition.to_string(), |edition| format!("\"{edition}\""));
            errors.push(format!("{name}: {at} sets an edition of its own, {value}: a target takes its crate's inherited edition and sets none of its own"));
        }
    }
    errors
}

/// The manifest tables Cargo reads a target from: `[lib]`, and the `[[bin]]`, `[[test]]`, `[[example]]`
/// and `[[bench]]` arrays. Each answer is the table and how a message names it; a named array entry is
/// named back, so the writer sees which one to fix.
fn target_tables(document: &toml::Value) -> Vec<(String, &toml::Value)> {
    let mut tables: Vec<(String, &toml::Value)> = Vec::new();
    if let Some(lib) = document.get("lib") {
        tables.push(("`[lib]`".to_string(), lib));
    }
    for kind in ["bin", "test", "example", "bench"] {
        for (index, entry) in document.get(kind).and_then(toml::Value::as_array).into_iter().flatten().enumerate() {
            let named = entry.get("name").and_then(toml::Value::as_str).map_or_else(|| format!("entry {index}"), |target| format!("`{target}`"));
            tables.push((format!("`[[{kind}]]` {named}"), entry));
        }
    }
    tables
}

/// Every attribute that weakens a denied lint, directly or through its group, reported at the line of the
/// attribute's `#`: `#![allow(…)]`, `#![expect(…)]`, `#![warn(…)]`, one of them inside `cfg_attr`, and the
/// same as an outer attribute on a `mod`, which covers the whole module just as the inner one does. On
/// any other single item only `expect_used` may be allowed, the one exception `AGENTS.md` §3 makes (a
/// provably infallible literal); the five lints without one are `forbid` and the compiler refuses them
/// anyway. The rule is "never crash outside tests", so `cfg_attr(test, …)` is not reported, and an
/// attribute a macro merely forwards onto one item (`#[$attribute]`) is the forwarding idiom, not a level.
pub(crate) fn weakened_crate_attributes(stripped_lines: &[String], name: &str) -> Vec<String> {
    let text = stripped_lines.join("\n");
    let mut errors = Vec::new();
    for allowance in attributes::allowances(&text).into_iter().filter(|allowance| !allowance.test_only) {
        let (hash, place, item) = match (allowance.inner, attributes::declared_module(&text, allowance.after)) {
            (true, _) => ("#!", String::new(), false),
            (false, Some(module)) => ("#", format!(" on `mod {module}`"), false),
            (false, None) if allowance.level == attributes::MACRO_ATTRIBUTE => continue,
            (false, None) => ("#", " on one item".to_string(), true),
        };
        let names: Vec<String> = allowance.lints.iter().filter_map(|lint| lint.strip_prefix("clippy::").map(canonical_lint)).collect();
        let excepted = |name: &str| item && (name == ITEM_EXCEPTION || (name == ITEM_EXCEPTION_BY_PATH && names.iter().any(|other| other == ITEM_EXCEPTION)));
        for lint in allowance.lints.iter().filter(|lint| reaches_denied_lint(lint)) {
            if lint.strip_prefix("clippy::").map(canonical_lint).is_some_and(|name| excepted(&name)) {
                continue;
            }
            let why = if item {
                "weakens a lint the workspace denies: on an item only clippy::expect_used may be allowed, on a provably infallible literal"
            } else {
                "weakens a lint the workspace denies, and a crate may not switch it off"
            };
            errors.push(format!("{name}:{}: {hash}[{}({lint})]{place} {why}", allowance.line, allowance.level));
        }
    }
    errors
}

/// May this crate have a build script? A build script runs before the compiler reads the lint table and
/// can hand the compiler flags and cfgs around it through `cargo::rustc-env` and `cargo::rustc-cfg`, so a
/// crate has one only when its row in the layer table marks it. `script` is the build script Cargo found
/// (`cargo metadata`'s own answer, whatever the manifest's spelling), as a message names it.
pub(crate) fn build_script_violations(name: &str, marked: bool, script: Option<&str>) -> Option<String> {
    let script = script.filter(|_| !marked)?;
    Some(format!("{name}: has a build script, `{script}`, which reaches the compiler around the lint table: only a crate the layer table marks may have one"))
}

/// The edition the workspace manifest declares for its crates to inherit, if it declares one. Cargo
/// resolves a crate's own edition itself, so this is the one half of the edition rule the workspace
/// manifest answers.
pub(crate) fn declared_edition(source: &str) -> Option<String> {
    let document = parse(source, "the workspace manifest").ok()?;
    Some(document.get("workspace")?.get("package")?.get("edition")?.as_str()?.to_string())
}

/// The other half of the edition rule ([`package_edition_violations`] says why there is one edition):
/// Cargo's own answer to which edition the code compiles under, so no spelling hides a pinned one.
/// `package` is `cargo metadata`'s edition for the crate — the old `[project]` table included, which Cargo
/// still reads for editions before 2024 — and `targets` its edition for each target, which a target's own
/// table can set apart from the package's with no more than a warning from Cargo; a target is named only
/// where it parts from the package, whose own message covers the rest. `workspace` is the edition every
/// one had to settle to.
pub(crate) fn resolved_edition_violations(name: &str, package: &str, targets: &[(String, String)], workspace: &str) -> Vec<String> {
    let mut errors = Vec::new();
    if package != workspace {
        errors.push(format!(
            "{name}: compiles under edition \"{package}\", not the workspace's \"{workspace}\": a crate inherits the workspace edition with edition.workspace = true"
        ));
    }
    for (target, edition) in targets.iter().filter(|(_, edition)| edition != workspace && edition != package) {
        errors.push(format!(
            "{name}: target `{target}` compiles under edition \"{edition}\", not the workspace's \"{workspace}\": a target takes its crate's inherited edition and sets none of its own"
        ));
    }
    errors
}

/// Run the check against the repository: the workspace manifest, Cargo's configuration, clippy's
/// configuration, every package manifest, build script and edition, and every `.rs` file's attributes.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let rust_dir = root.join("rust");
    let workspace_source = scan::read_text(&rust_dir.join("Cargo.toml"))?;
    let mut errors = workspace_manifest_violations(&workspace_source, "rust/Cargo.toml");
    // The one edition every crate inherits. With none declared, inheritance has nothing to inherit and
    // a crate could pin any edition for itself under cover of the rule below.
    let workspace_edition = declared_edition(&workspace_source);
    if workspace_edition.is_none() {
        errors.push(
            "rust/Cargo.toml: [workspace.package] declares no edition: the workspace sets the one edition every crate inherits with edition.workspace = true"
                .to_string(),
        );
    }

    // The one Cargo configuration the gate's commands read; the machinery check refuses any other.
    let config = root.join(machinery::CARGO_CONFIGURATION);
    if config.is_file() {
        errors.extend(cargo_config_violations(&scan::read_text(&config)?, machinery::CARGO_CONFIGURATION));
    }

    // Clippy reads its configuration from one file only, the one nearest a crate; the machinery check
    // refuses any copy that would be nearer elsewhere, so this check reads exactly the listed one.
    let clippy_config = root.join(machinery::CLIPPY_CONFIGURATION);
    if clippy_config.is_file() {
        errors.extend(clippy_config_violations(&scan::read_text(&clippy_config)?, machinery::CLIPPY_CONFIGURATION));
    }

    for package in crate::meta::read(&rust_dir, crate::meta::Graph::Workspace)? {
        let manifest = Path::new(&package.manifest_path);
        let shown = crate::report::shown(root, manifest);
        let source = scan::read_text(manifest)?;
        errors.extend(package_lints_violations(&source, &shown));
        errors.extend(package_edition_violations(&source, &shown));
        if let Some(edition) = workspace_edition.as_deref() {
            errors.extend(resolved_edition_violations(&shown, &package.edition, &package.target_editions, edition));
        }
        let marked = crate::layers::entry(&package.name).is_some_and(|row| row.build);
        let script = package.build_script.as_deref().map(|script| crate::report::shown(root, Path::new(script)));
        errors.extend(build_script_violations(&shown, marked, script.as_deref()));
    }

    // A generator's output is checked by regenerating it, not here.
    for file in scan::walk(&rust_dir, ".rs")?.into_iter().filter(|file| !crate::generated::in_generated(&file.components)) {
        let shown = crate::report::shown(root, &file.path);
        let stripped = scan::strip_rust(&scan::read_text(&file.path)?);
        errors.extend(weakened_crate_attributes(&stripped, &shown));
    }
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_MANIFEST: &str = r#"
[workspace]
members = ["core/*"]

[workspace.lints.rust]
unsafe_code = "deny"

[workspace.lints.clippy]
unwrap_used = "forbid"
expect_used = "deny"
panic = "forbid"
unimplemented = "forbid"
todo = "forbid"
unreachable = "forbid"
disallowed_methods = "deny"
disallowed_macros = "deny"
undocumented_unsafe_blocks = "deny"

[profile.release]
panic = "unwind"
"#;

    fn weakened(source: &str) -> Vec<String> {
        weakened_crate_attributes(&scan::strip_rust(source), "x.rs")
    }

    #[test]
    fn a_strong_table_passes_in_both_level_spellings() {
        assert!(workspace_manifest_violations(GOOD_MANIFEST, "rust/Cargo.toml").is_empty());
        let table_form = GOOD_MANIFEST.replace("unwrap_used = \"forbid\"", "unwrap_used = { level = \"forbid\", priority = -2 }");
        assert!(workspace_manifest_violations(&table_form, "rust/Cargo.toml").is_empty());
    }

    #[test]
    fn a_missing_or_weak_lint_is_named() {
        let without_unwrap = GOOD_MANIFEST.replace("unwrap_used = \"forbid\"", "");
        let errors = workspace_manifest_violations(&without_unwrap, "rust/Cargo.toml");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("unwrap_used"), "{errors:?}");

        let weak = GOOD_MANIFEST.replace("expect_used = \"deny\"", "expect_used = \"warn\"");
        let errors = workspace_manifest_violations(&weak, "rust/Cargo.toml");
        assert!(errors.iter().any(|e| e.contains("expect_used")), "{errors:?}");

        let no_unsafe = GOOD_MANIFEST.replace("unsafe_code = \"deny\"", "");
        let errors = workspace_manifest_violations(&no_unsafe, "rust/Cargo.toml");
        assert!(errors.iter().any(|e| e.contains("unsafe_code")), "{errors:?}");
    }

    #[test]
    fn a_second_key_for_a_denied_lint_is_read_as_that_lint() {
        // Cargo passes every key on as written; the compiler reads `-` as `_` and a former name as the lint
        // it became, so each of these switches a denied lint off beside its strong spelling.
        for (table, weak, lint) in [
            ("[workspace.lints.clippy]", "unwrap-used = { level = \"allow\", priority = 1 }", "unwrap-used"),
            ("[workspace.lints.clippy]", "option_unwrap_used = { level = \"allow\", priority = 1 }", "option_unwrap_used"),
            ("[workspace.lints.clippy]", "result_expect_used = \"warn\"", "result_expect_used"),
            ("[workspace.lints.clippy]", "Restriction = \"allow\"", ""),
            ("[workspace.lints.rust]", "unsafe-code = { level = \"allow\", priority = 1 }", "unsafe-code"),
        ] {
            let manifest = GOOD_MANIFEST.replace(table, &format!("{table}\n{weak}"));
            let errors = workspace_manifest_violations(&manifest, "rust/Cargo.toml");
            if lint.is_empty() {
                // An upper-case name is no lint at all to the compiler, so it weakens nothing.
                assert!(errors.is_empty(), "{weak}: {errors:?}");
            } else {
                assert!(errors.iter().any(|e| e.contains(&format!("].{lint} must be deny"))), "{weak}: {errors:?}");
            }
        }
        // A second strong spelling of a denied lint is still a denied lint.
        let doubled = GOOD_MANIFEST.replace("unwrap_used = \"forbid\"", "unwrap_used = \"forbid\"\nunwrap-used = \"forbid\"");
        assert!(workspace_manifest_violations(&doubled, "rust/Cargo.toml").is_empty());
        let hyphen_only = GOOD_MANIFEST.replace("unwrap_used = \"forbid\"", "unwrap-used = \"forbid\"");
        assert!(workspace_manifest_violations(&hyphen_only, "rust/Cargo.toml").is_empty(), "the hyphen spelling alone forbids the lint");
    }

    #[test]
    fn a_lint_with_no_exception_must_be_forbidden() {
        // `deny` lets an `allow` compile; `forbid` makes the compiler refuse it however it is written.
        for lint in FORBIDDEN_CLIPPY_LINTS {
            let denied = GOOD_MANIFEST.replace(&format!("{lint} = \"forbid\""), &format!("{lint} = \"deny\""));
            let errors = workspace_manifest_violations(&denied, "rust/Cargo.toml");
            assert!(errors.len() == 1 && errors[0].contains(&format!("].{lint} must be forbid")), "{lint}: {errors:?}");
        }
        let expect_forbidden = GOOD_MANIFEST.replace("expect_used = \"deny\"", "expect_used = \"forbid\"");
        assert!(workspace_manifest_violations(&expect_forbidden, "rust/Cargo.toml").is_empty(), "expect_used may be stricter than its exception needs");
    }

    #[test]
    fn a_group_that_switches_the_denied_lints_off_is_named() {
        let allowed = GOOD_MANIFEST.replace("[profile.release]", "restriction = { level = \"allow\", priority = 1 }\n\n[profile.release]");
        let errors = workspace_manifest_violations(&allowed, "rust/Cargo.toml");
        assert!(errors.iter().any(|e| e.contains("restriction")), "{errors:?}");
        let denied = GOOD_MANIFEST.replace("[profile.release]", "restriction = \"deny\"\n\n[profile.release]");
        assert!(workspace_manifest_violations(&denied, "rust/Cargo.toml").is_empty());
    }

    #[test]
    fn an_aborting_profile_is_reported_wherever_it_is_set() {
        let profile_wide = GOOD_MANIFEST.replace("panic = \"unwind\"", "panic = \"abort\"");
        let errors = workspace_manifest_violations(&profile_wide, "rust/Cargo.toml");
        assert!(errors.iter().any(|e| e.contains("[profile.release]") && e.contains("unwinding")), "{errors:?}");

        let per_crate = format!("{GOOD_MANIFEST}\n[profile.release.package.craftbridge-core]\npanic = \"abort\"\n");
        let errors = workspace_manifest_violations(&per_crate, "rust/Cargo.toml");
        assert!(errors.iter().any(|e| e.contains("package.craftbridge-core")), "{errors:?}");

        let unknown = GOOD_MANIFEST.replace("panic = \"unwind\"", "panic = \"immediate-abort\"");
        assert!(!workspace_manifest_violations(&unknown, "rust/Cargo.toml").is_empty());

        let with_lto = GOOD_MANIFEST.replace("panic = \"unwind\"", "lto = true\npanic = \"unwind\"");
        assert!(workspace_manifest_violations(&with_lto, "rust/Cargo.toml").is_empty(), "other profile settings are none of our business");
    }

    #[test]
    fn cargo_configuration_may_not_override_the_table_or_the_profile() {
        assert!(cargo_config_violations("[alias]\nxtask = \"run --package xtask --locked --quiet --\"\n", "c").is_empty());
        let profile = cargo_config_violations("[profile.release]\npanic = \"abort\"\n", "c");
        assert!(profile.iter().any(|e| e.contains("[profile.release]")), "{profile:?}");
        let glued = cargo_config_violations("[build]\nrustflags = [\"-Aclippy::unwrap_used\"]\n", "c");
        assert!(glued.iter().any(|e| e.contains("-Aclippy::unwrap_used")), "{glued:?}");
        let spread = cargo_config_violations("[target.aarch64-linux-android]\nrustflags = \"--cap-lints warn -C panic=abort\"\n", "c");
        assert_eq!(spread.len(), 2, "{spread:?}");
        let harmless = cargo_config_violations("[build]\nrustflags = [\"-C\", \"link-arg=-Wl,-z,max-page-size=16384\"]\n", "c");
        assert!(harmless.is_empty(), "{harmless:?}");
        let clippy_env = cargo_config_violations(
            "[env]\nCLIPPY_ARGS = { value = \"-Aclippy::unwrap_used\", force = true }\nCLIPPY_CONF_DIR = \"/elsewhere\"\nIPHONEOS_DEPLOYMENT_TARGET = \"17.0\"\n",
            "c",
        );
        assert_eq!(clippy_env.len(), 2, "{clippy_env:?}");
        assert!(clippy_env.iter().all(|e| e.contains("[env].CLIPPY_")), "{clippy_env:?}");
        // A wrapper around rustc (or clippy-driver) strips the denied lints before they arrive.
        let wrapper =
            cargo_config_violations("[build]\nrustc-wrapper = \"./wrap.sh\"\nrustc-workspace-wrapper = \"./wrap.sh\"\nrustc = \"./fake-rustc\"\n", "c");
        assert_eq!(wrapper.len(), 3, "{wrapper:?}");
        // An alias named after an external command the gate runs runs in its place; `xtask` must run xtask.
        // A runner stands in for every binary Cargo builds, the test binaries and xtask included.
        let runner = cargo_config_violations("[target.aarch64-apple-darwin]\nrunner = \"true\"\n[target.'cfg(all())']\nrunner = [\"env\"]\n", "c");
        assert!(runner.len() == 2 && runner.iter().all(|e| e.contains("].runner runs in place of every binary")), "{runner:?}");
        let aliases = cargo_config_violations("[alias]\nxtask = \"test --no-run\"\nclippy = \"test --no-run\"\nfmt = [\"version\"]\nb = \"build\"\n", "c");
        assert_eq!(aliases.len(), 3, "{aliases:?}");
        assert!(aliases.iter().any(|e| e.contains("[alias].xtask must run the xtask package")), "{aliases:?}");
        assert!(aliases.iter().any(|e| e.contains("[alias].clippy stands in")) && aliases.iter().any(|e| e.contains("[alias].fmt stands in")), "{aliases:?}");
        assert!(cargo_config_violations("[alias]\nxtask = [\"run\", \"-p\", \"xtask\", \"--\"]\n", "c").is_empty(), "the list form of the gate's alias");
        assert!(wrapper.iter().all(|e| e.contains("must reach rustc untouched")), "{wrapper:?}");
    }

    #[test]
    fn clippy_configuration_may_not_let_unwrap_through_on_a_type() {
        let ours = "allow-unwrap-in-tests = true\nallow-expect-in-tests = true\nallow-panic-in-tests = true\ncognitive-complexity-threshold = 30\n";
        assert!(clippy_config_violations(ours, "rust/clippy.toml").is_empty());
        assert!(clippy_config_violations(&format!("{ours}allow-unwrap-types = []\n"), "rust/clippy.toml").is_empty(), "an empty list lets nothing through");
        let typed = clippy_config_violations(&format!("{ours}allow-unwrap-types = [\"core::option::Option\"]\n"), "rust/clippy.toml");
        assert!(typed.len() == 1 && typed[0].contains("allow-unwrap-types"), "{typed:?}");
    }

    #[test]
    fn a_build_script_needs_a_mark_in_the_layer_table() {
        let found = build_script_violations("rust/core/craftbridge-core/Cargo.toml", false, Some("rust/core/craftbridge-core/gen.rs"));
        assert!(found.as_deref().is_some_and(|e| e.contains("`rust/core/craftbridge-core/gen.rs`")), "the message names the script: {found:?}");
        assert!(build_script_violations("craftbridge-core", true, Some("build.rs")).is_none(), "a marked crate may have one");
        assert!(build_script_violations("craftbridge-core", false, None).is_none());
        // The table marks no crate today, which is what the AGENTS.md rule says.
        assert!(crate::layers::ENTRIES.iter().all(|row| !row.build), "no crate is marked for a build script yet");
    }

    /// The real tree's copy, checked: a check that cannot run here is a broken copy or a broken check,
    /// and says which instead of panicking with no rule's name.
    fn checked(copy: &crate::scratch::Scratch) -> Vec<String> {
        check(&copy.0)
            .unwrap_or_else(|failure| panic!("the lints check could not run on a copy of rust/ (a path outside rust/ is refused by rules: layers): {failure}"))
    }

    #[test]
    fn a_cargo_configuration_planted_in_a_real_tree_is_caught() {
        let copy = crate::scratch::rust_tree_copy("lints-config");
        let config = copy.0.join(machinery::CARGO_CONFIGURATION);
        let original = std::fs::read_to_string(&config).unwrap();
        std::fs::write(&config, format!("{original}\n[build]\nrustflags = [\"-Aclippy::unwrap_used\"]\n")).unwrap();
        let errors = checked(&copy);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("rust/.cargo/config.toml: [build].rustflags passes `-Aclippy::unwrap_used`")), "{errors:?}");
        std::fs::write(&config, &original).unwrap();
        assert!(checked(&copy).is_empty(), "the copy with the file restored is the clean tree");
    }

    #[test]
    fn a_build_script_planted_in_a_real_tree_is_caught_however_the_manifest_names_it() {
        let copy = crate::scratch::rust_tree_copy("lints-build-script");
        let crate_dir = copy.0.join("rust/core/craftbridge-core");
        let manifest = crate_dir.join("Cargo.toml");
        let original = std::fs::read_to_string(&manifest).unwrap();
        let script = "fn main() { println!(\"cargo::rustc-env=CLIPPY_ARGS=-Aclippy::unwrap_used__CLIPPY_HACKERY__\"); }\n";
        // A build.rs Cargo picks up beside the manifest.
        std::fs::write(crate_dir.join("build.rs"), script).unwrap();
        let errors = checked(&copy);
        assert!(errors.len() == 1 && errors[0].contains("has a build script, `rust/core/craftbridge-core/build.rs`"), "{errors:?}");
        // `build = false` turns that detection off, so the file is no build script.
        std::fs::write(&manifest, original.replacen("[package]", "[package]\nbuild = false", 1)).unwrap();
        assert!(checked(&copy).is_empty(), "{:?}", checked(&copy));
        std::fs::remove_file(crate_dir.join("build.rs")).unwrap();
        // One `package.build` names.
        std::fs::write(crate_dir.join("gen.rs"), script).unwrap();
        std::fs::write(&manifest, original.replacen("[package]", "[package]\nbuild = \"gen.rs\"", 1)).unwrap();
        let errors = checked(&copy);
        assert!(errors.len() == 1 && errors[0].contains("`rust/core/craftbridge-core/gen.rs`"), "{errors:?}");
        // And the old `[project]` table, which Cargo still reads for editions before 2024. Such a manifest
        // pins an old edition too, so the edition rule names it twice beside the build script: the
        // spelling inherits nothing, and the edition Cargo settles on is not the workspace's.
        let project = original.replacen("[package]", "[project]\nbuild = \"gen.rs\"", 1).replace("edition.workspace = true", "edition = \"2021\"");
        std::fs::write(&manifest, project).unwrap();
        let errors = checked(&copy);
        assert!(
            errors.len() == 3
                && errors.iter().any(|e| e.contains("`rust/core/craftbridge-core/gen.rs`"))
                && errors.iter().filter(|e| e.contains("edition.workspace = true")).count() == 2,
            "{errors:?}"
        );
        std::fs::write(&manifest, &original).unwrap();
        std::fs::remove_file(crate_dir.join("gen.rs")).unwrap();
        assert!(checked(&copy).is_empty());
    }

    #[test]
    fn a_package_inherits_and_only_inherits() {
        assert!(package_lints_violations("[package]\nname = \"x\"\n[lints]\nworkspace = true\n", "x/Cargo.toml").is_empty());
        assert!(package_lints_violations("[package]\nname = \"x\"\n", "x/Cargo.toml").iter().any(|e| e.contains("no [lints] table")));
        let weakened = package_lints_violations("[package]\nname = \"x\"\n[lints]\nworkspace = true\n\"clippy::unwrap_used\" = \"allow\"\n", "x/Cargo.toml");
        assert!(weakened.iter().any(|e| e.contains("clippy::unwrap_used") && e.contains("only inherit")), "{weakened:?}");
        let not_inherited = package_lints_violations("[package]\nname = \"x\"\n[lints]\nworkspace = false\n", "x/Cargo.toml");
        assert!(not_inherited.iter().any(|e| e.contains("workspace = true")), "{not_inherited:?}");
    }

    #[test]
    fn a_manifest_inherits_the_workspace_edition_like_the_lint_table() {
        assert!(package_edition_violations("[package]\nname = \"x\"\nedition.workspace = true\n[lints]\nworkspace = true\n", "x/Cargo.toml").is_empty());
        assert!(
            package_edition_violations("[project]\nname = \"x\"\nedition = { workspace = true }\n", "x/Cargo.toml").is_empty(),
            "the old table inherits the same way"
        );
        let pinned = package_edition_violations("[package]\nname = \"x\"\nedition = \"2021\"\n[lints]\nworkspace = true\n", "x/Cargo.toml");
        assert!(pinned.len() == 1 && pinned[0].starts_with("x/Cargo.toml:") && pinned[0].contains("edition = \"2021\""), "{pinned:?}");
        let missing = package_edition_violations("[package]\nname = \"x\"\n", "x/Cargo.toml");
        assert!(missing.iter().any(|e| e.contains("does not inherit the workspace edition")), "{missing:?}");
        // A target table writing the workspace's own edition sets one of its own all the same: the pin is
        // the writing, and Cargo's answer only agrees with the workspace until the workspace moves.
        let lib = package_edition_violations("[package]\nname = \"x\"\nedition.workspace = true\n[lib]\nedition = \"2024\"\n", "x/Cargo.toml");
        assert!(
            lib.len() == 1 && lib[0].starts_with("x/Cargo.toml: `[lib]` sets an edition of its own, \"2024\"") && lib[0].contains("sets none of its own"),
            "{lib:?}"
        );
        let bin = package_edition_violations("[package]\nname = \"x\"\nedition.workspace = true\n[[bin]]\nname = \"t\"\nedition = \"2021\"\n", "x/Cargo.toml");
        assert!(bin.len() == 1 && bin[0].starts_with("x/Cargo.toml: `[[bin]]` `t` sets an edition of its own"), "{bin:?}");
        let unset =
            package_edition_violations("[package]\nname = \"x\"\nedition.workspace = true\n[[bin]]\nname = \"t\"\npath = \"src/main.rs\"\n", "x/Cargo.toml");
        assert!(unset.is_empty(), "a target table says many things; only the edition is the pin: {unset:?}");
        let mentioned = package_edition_violations(
            "# [lib]\n# edition = \"2021\"\n[package]\nname = \"x\"\ndescription = \"[lib] edition = \\\"2021\\\"\"\nedition.workspace = true\n",
            "x/Cargo.toml",
        );
        assert!(mentioned.is_empty(), "the manifest is read as TOML, so a comment or a string that spells a target edition is none: {mentioned:?}");
        assert_eq!(declared_edition("[workspace]\n[workspace.package]\nname = \"x\"\nedition = \"2024\"\n").as_deref(), Some("2024"));
        assert_eq!(declared_edition("[workspace]\n[workspace.package]\nname = \"x\"\n"), None);
        let inherited = [("x".to_string(), "2024".to_string()), ("it".to_string(), "2024".to_string())];
        assert!(resolved_edition_violations("x/Cargo.toml", "2024", &inherited, "2024").is_empty());
        let settled = resolved_edition_violations("x/Cargo.toml", "2021", &[("x".to_string(), "2021".to_string())], "2024");
        assert!(
            settled.len() == 1 && settled[0].starts_with("x/Cargo.toml:") && settled[0].contains("\"2021\"") && settled[0].contains("edition.workspace = true"),
            "the package's message covers the targets that follow it: {settled:?}"
        );
        let target =
            resolved_edition_violations("x/Cargo.toml", "2024", &[("x".to_string(), "2024".to_string()), ("it".to_string(), "2018".to_string())], "2024");
        assert!(target.len() == 1 && target[0].starts_with("x/Cargo.toml: target `it` compiles under edition \"2018\""), "{target:?}");
    }

    #[test]
    fn a_real_crate_pinning_an_old_edition_is_refused_however_it_spells_the_table() {
        let copy = crate::scratch::rust_tree_copy("lints-edition");
        let manifest = copy.0.join("rust/core/craftbridge-core/Cargo.toml");
        let original = std::fs::read_to_string(&manifest).unwrap();
        // An old edition, pinned in the crate's own table.
        std::fs::write(&manifest, original.replace("edition.workspace = true", "edition = \"2021\"")).unwrap();
        let errors = checked(&copy);
        assert!(errors.len() == 2, "{errors:?}");
        assert!(errors.iter().all(|e| e.starts_with("rust/core/craftbridge-core/Cargo.toml:")) && errors.iter().all(|e| e.contains("2021")), "{errors:?}");
        // The old `[project]` table too: Cargo reads it for editions before 2024, and its answer shows.
        std::fs::write(&manifest, original.replace("[package]", "[project]").replace("edition.workspace = true", "edition = \"2021\"")).unwrap();
        let errors = checked(&copy);
        assert!(errors.len() == 2 && errors.iter().all(|e| e.contains("\"2021\"")), "{errors:?}");
        // A target's own table, under a package that inherits: it pins that one target's edition and
        // Cargo only warns, so the writing shows the pin and Cargo's answer for the target follows it.
        std::fs::write(&manifest, format!("{original}\n[lib]\nedition = \"2021\"\n")).unwrap();
        let errors = checked(&copy);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("rust/core/craftbridge-core/Cargo.toml: `[lib]` sets an edition of its own")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("target `craftbridge_core` compiles under edition \"2021\"")), "{errors:?}");
        // The pin holds even when the table writes the workspace's own edition: Cargo's answer is equal
        // today and shows nothing, so only the spelling refuses the table that sets one of its own.
        std::fs::write(&manifest, format!("{original}\n[lib]\nedition = \"2024\"\n")).unwrap();
        let errors = checked(&copy);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("rust/core/craftbridge-core/Cargo.toml: `[lib]` sets an edition of its own, \"2024\""), "{errors:?}");
        std::fs::write(&manifest, &original).unwrap();
        assert!(checked(&copy).is_empty(), "the tree itself inherits, and passes");
    }

    /// Every table Cargo reads a target from, each written in a TOML form of its own (a dotted key, an
    /// inline array, array-of-tables headers), held against Cargo's own reading of the same manifest: Cargo
    /// compiles each of the five targets under the edition its table writes, and the rule names each table.
    #[test]
    fn every_target_table_cargo_reads_is_named_however_the_toml_spells_it() {
        let copy = crate::scratch::rust_tree_copy("lints-target-tables");
        let crate_dir = copy.0.join("rust/core/craftbridge-core");
        let manifest = crate_dir.join("Cargo.toml");
        let original = std::fs::read_to_string(&manifest).unwrap();
        for file in ["src/bin/probe-bin.rs", "tests/probe-test.rs", "examples/probe-example.rs", "benches/probe-bench.rs"] {
            std::fs::create_dir_all(crate_dir.join(file).parent().unwrap()).unwrap();
            std::fs::write(crate_dir.join(file), "//! A probe target.\nfn main() {}\n").unwrap();
        }
        let ahead = "lib.edition = \"2021\"\nbin = [{ name = \"probe-bin\", edition = \"2021\" }]\n";
        let arrays = ["test", "example", "bench"].map(|kind| format!("\n[[{kind}]]\nname = \"probe-{kind}\"\nedition = \"2021\"\n")).concat();
        std::fs::write(&manifest, format!("{ahead}{original}{arrays}")).unwrap();
        let errors = checked(&copy);
        for table in ["`[lib]`", "`[[bin]]` `probe-bin`", "`[[test]]` `probe-test`", "`[[example]]` `probe-example`", "`[[bench]]` `probe-bench`"] {
            assert!(errors.iter().any(|e| e.contains(&format!(": {table} sets an edition of its own, \"2021\""))), "{table}: {errors:?}");
        }
        for target in ["craftbridge_core", "probe-bin", "probe-test", "probe-example", "probe-bench"] {
            assert!(
                errors.iter().any(|e| e.contains(&format!("target `{target}` compiles under edition \"2021\""))),
                "Cargo reads {target}'s table: {errors:?}"
            );
        }
        assert_eq!(errors.len(), 10, "{errors:?}");
    }

    #[test]
    fn a_workspace_that_declares_no_edition_is_refused() {
        let copy = crate::scratch::rust_tree_copy("lints-no-edition");
        let workspace = copy.0.join("rust/Cargo.toml");
        let source = std::fs::read_to_string(&workspace).unwrap();
        std::fs::write(&workspace, source.replacen("edition = \"2024\"\n", "", 1)).unwrap();
        // With nothing to inherit, Cargo reads the tree only once every crate pins an edition of its own.
        for member in crate::scan::walk(&copy.0.join("rust"), "Cargo.toml").unwrap().into_iter().filter(|file| file.relative != "Cargo.toml") {
            let manifest = std::fs::read_to_string(&member.path).unwrap();
            std::fs::write(&member.path, manifest.replace("edition.workspace = true", "edition = \"2024\"")).unwrap();
        }
        let errors = checked(&copy);
        assert!(errors.iter().any(|e| e.starts_with("rust/Cargo.toml: [workspace.package] declares no edition")), "{errors:?}");
        assert!(errors.iter().filter(|e| e.contains("does not inherit the workspace edition")).count() > 1, "{errors:?}");
    }

    #[test]
    fn a_crate_level_allow_is_reported_with_its_line_and_level() {
        let errors = weakened("//! docs\n#![allow(clippy::unwrap_used)]\nfn f() {}\n");
        assert_eq!(errors, ["x.rs:2: #![allow(clippy::unwrap_used)] weakens a lint the workspace denies, and a crate may not switch it off"]);
    }

    #[test]
    fn every_spelling_of_a_crate_level_weakening_is_caught() {
        let multiline = weakened("#![expect(\n    clippy::expect_used\n)]\n");
        assert!(multiline.len() == 1 && multiline[0].contains("x.rs:1: #![expect(clippy::expect_used)]"), "{multiline:?}");
        assert!(weakened("#![allow(clippy::all, clippy::todo)]\n").iter().any(|e| e.contains("clippy::todo")));
        assert!(weakened("#![allow(clippy::restriction)]\n").iter().any(|e| e.contains("clippy::restriction")), "the group holds every denied lint");
        assert!(weakened("#![cfg_attr(not(test), allow(clippy::panic))]\n").iter().any(|e| e.contains("clippy::panic")));
        assert!(weakened("#![cfg_attr(any(test, feature = \"x\"), allow(clippy::panic))]\n").iter().any(|e| e.contains("clippy::panic")));
        assert!(weakened("#![allow(clippy::undocumented_unsafe_blocks)]\n").iter().any(|e| e.contains("undocumented_unsafe_blocks")));
        assert!(weakened("#![allow(clippy::option_unwrap_used)]\n").iter().any(|e| e.contains("clippy::option_unwrap_used")), "a former name");
        // Spellings the compiler reads as the same attribute: a raw identifier, and `#` apart from `!`.
        assert!(weakened("#![allow(clippy::r#expect_used)]\n").iter().any(|e| e.contains("x.rs:1: #![allow(clippy::expect_used)]")));
        assert!(weakened("#![allow(r#clippy::undocumented_unsafe_blocks)]\n").iter().any(|e| e.contains("clippy::undocumented_unsafe_blocks")));
        assert!(weakened("# ![allow(clippy::panic)]\n").iter().any(|e| e.contains("x.rs:1: #![allow(clippy::panic)]")));
        assert!(weakened("#\n![allow(clippy::panic)]\n").iter().any(|e| e.contains("x.rs:1:")));
        // A lint or level a macro fills in might be any of them.
        let macro_lint = weakened("macro_rules! quiet { ($lint:ident) => { #[allow(clippy::$lint)] pub mod inner {} }; }\n");
        assert!(!macro_lint.is_empty(), "{macro_lint:?}");
        let macro_level = weakened("macro_rules! level { ($l:ident) => { #[$l(clippy::expect_used)] mod inner {} }; }\n");
        assert!(!macro_level.is_empty(), "{macro_level:?}");
    }

    #[test]
    fn an_outer_attribute_on_a_module_weakens_the_whole_module() {
        let errors = weakened("#[allow(clippy::unwrap_used, clippy::panic)]\nmod inner {\n    pub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n}\n");
        assert_eq!(
            errors,
            [
                "x.rs:1: #[allow(clippy::unwrap_used)] on `mod inner` weakens a lint the workspace denies, and a crate may not switch it off",
                "x.rs:1: #[allow(clippy::panic)] on `mod inner` weakens a lint the workspace denies, and a crate may not switch it off",
            ]
        );
        assert!(!weakened("#[allow(clippy::r#panic)] pub mod inner;\n").is_empty(), "a raw identifier on a module");
        let declared = weakened("#[cfg(feature = \"x\")]\n#[expect(clippy::todo)]\npub(crate) mod later;\n");
        assert!(declared.len() == 1 && declared[0].contains("x.rs:2: #[expect(clippy::todo)] on `mod later`"), "{declared:?}");
        assert!(weakened("#[cfg_attr(test, allow(clippy::unwrap_used))]\nmod tests;\n").is_empty(), "a module's tests may unwrap");
        assert!(weakened("#[allow(clippy::needless_range_loop)]\nmod grid;\n").is_empty(), "a lint the workspace does not deny");
    }

    #[test]
    fn compliant_attributes_and_comments_pass() {
        assert!(weakened("#![deny(missing_docs)]\n#![allow(clippy::needless_range_loop)]\n// #![allow(clippy::panic)]\nfn f() {}\n").is_empty());
        assert!(weakened("#![cfg_attr(test, allow(clippy::unwrap_used))]\n").is_empty(), "tests may unwrap");
        // On one item only `expect_used` may be allowed (AGENTS.md §3), and a macro forwarding attributes
        // onto an item is the forwarding idiom.
        assert!(weakened("#[allow(clippy::expect_used)]\nfn f() -> i32 { 1 }\n").is_empty());
        assert!(weakened("macro_rules! m { ($(#[$a:meta])* $i:item) => { $(#[$a])* $i }; }\n").is_empty());
        assert!(weakened("#[allow(clippy::needless_range_loop)]\nfn f() {}\n").is_empty(), "a lint the workspace does not deny");
    }

    #[test]
    fn an_item_may_allow_only_expect_used() {
        for (source, lint) in [
            ("#[allow(clippy::undocumented_unsafe_blocks)]\nfn f() {}\n", "clippy::undocumented_unsafe_blocks"),
            ("#[allow(clippy::unwrap_used)]\nimpl X {}\n", "clippy::unwrap_used"),
            ("#[expect(clippy::restriction)]\nfn f() {}\n", "clippy::restriction"),
        ] {
            let errors = weakened(source);
            assert!(errors.len() == 1 && errors[0].contains(&format!("({lint})] on one item")) && errors[0].contains("only clippy::expect_used"), "{errors:?}");
        }
        assert!(weakened("#[allow(clippy::r#expect_used)]\nconst X: u8 = 1;\n").is_empty());
    }

    #[test]
    fn a_default_group_switched_off_in_the_table_is_refused() {
        for (table, key) in [("clippy", "all"), ("clippy", "style"), ("clippy", "correctness"), ("clippy", "perf"), ("rust", "warnings")] {
            let weakened = GOOD_MANIFEST
                .replace(&format!("[workspace.lints.{table}]\n"), &format!("[workspace.lints.{table}]\n{key} = {{ level = \"allow\", priority = 1 }}\n"));
            let errors = workspace_manifest_violations(&weakened, "rust/Cargo.toml");
            assert!(errors.iter().any(|e| e.contains(&format!("[workspace.lints.{table}].{key} = allow switches off"))), "{key}: {errors:?}");
        }
        let warned = GOOD_MANIFEST.replace("[workspace.lints.clippy]\n", "[workspace.lints.clippy]\npedantic = \"warn\"\nstyle = \"warn\"\n");
        assert!(workspace_manifest_violations(&warned, "rust/Cargo.toml").is_empty());
        let without = GOOD_MANIFEST.replace("disallowed_methods = \"deny\"\n", "");
        assert!(workspace_manifest_violations(&without, "rust/Cargo.toml").iter().any(|e| e.contains("disallowed_methods must be deny or forbid")));
    }

    #[test]
    fn the_calls_by_path_are_allowed_on_an_item_only_beside_the_expect_exception() {
        assert!(weakened("#[allow(clippy::expect_used, clippy::disallowed_methods)]\nfn f() {}\n").is_empty());
        let alone = weakened("#[allow(clippy::disallowed_methods)]\nfn f() {}\n");
        assert!(alone.len() == 1 && alone[0].contains("disallowed_methods"), "{alone:?}");
        assert_eq!(weakened("#![allow(clippy::disallowed_macros)]\n").len(), 1);
        assert_eq!(weakened("#![allow(clippy::all)]\n").len(), 1, "all holds style, which holds the calls by path");
        assert!(weakened("#![cfg_attr(test, allow(clippy::disallowed_methods))]\n").is_empty());
    }

    #[test]
    fn the_check_reads_the_workspace_on_disk() {
        let errors = check(&crate::repo_root()).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
