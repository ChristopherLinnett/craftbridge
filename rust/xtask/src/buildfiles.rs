//! Rule 10 — what a Gradle build file of the Android SDK may say.
//!
//! A module's build file is the one piece of Gradle every contributor writes, and Gradle lets it say
//! anything: a `tasks` line rewires a check the gate relies on, a `sourceSets` line compiles another
//! folder's code into the module, and `afterEvaluate` waits until the convention plugin's read-backs
//! have run to undo their work. So the build files are not trusted to stay quiet: each module's holds
//! only the lines that describe the module — its convention plugin, its namespace, its dependencies —
//! and the root's only its plugins block. Everything else a build needs is configured once in
//! `android/sdk/build-logic`, which is the gate's machinery, so changing what a build file may say is
//! itself a gate change a reviewer sees.
//!
//! The files are read as text under a small line grammar, not as Kotlin: a Kotlin parser would accept
//! any Kotlin a file can hold, and this rule is about what the files may hold at all. Anything the
//! grammar does not know fails, because a line this rule cannot read is a line nobody decided belongs
//! in a build file. A Groovy `build.gradle` is refused too: Gradle reads it in place of the `.kts`
//! file of the same project, and a rule that cannot read it cannot allow it.

use crate::scan;
use std::path::Path;

/// Which allowlist a file is read against.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `android/sdk/<module>/build.gradle.kts`.
    Module,
    /// `android/sdk/build.gradle.kts`.
    Root,
}

/// The block whose body a line sits in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Inside {
    Outside,
    Plugins,
    Android,
    Dependencies,
}

/// The plugin every module applies, and the one the root applies: the rules the gate relies on are
/// what they bring.
const MODULE_PLUGIN: &str = "id(\"craftbridge.android.library\")";
const ROOT_PLUGIN: &str = "id(\"craftbridge.quality\")";

/// The `android` block's one line: AGP reads the namespace, and AGENTS §9 says what it is.
const NAMESPACE_PREFIX: &str = "namespace = \"io.github.christopherlinnett.craftbridge.";

/// The dependency configurations the rule allows, so a module can say what it builds and tests
/// against; every version itself comes from the catalog, never from a coordinate written here.
const DEPENDENCY_CONFIGURATIONS: &[&str] = &["api", "implementation", "testImplementation", "androidTestImplementation"];

/// Is a line the opening of `word`'s block — alone (`plugins {`) or compact with one statement inside
/// (`plugins { id("…") }`), either of which the file may be written?
fn block_open<'a>(line: &'a str, word: &str) -> Option<Option<&'a str>> {
    if line == format!("{word} {{") {
        return Some(None);
    }
    let inner = line.strip_prefix(&format!("{word} {{ "))?.strip_suffix(" }")?;
    Some(Some(inner.trim()))
}

/// Which block does this outside line open, if any, and does it close in the same line with one entry
/// inside? `android` and `dependencies` exist only in a module file.
fn open_block(line: &str, kind: Kind) -> Option<(Inside, Option<&str>)> {
    for (word, block) in [("plugins", Inside::Plugins), ("android", Inside::Android), ("dependencies", Inside::Dependencies)] {
        if kind == Kind::Root && block != Inside::Plugins {
            continue;
        }
        if let Some(inner) = block_open(line, word) {
            return Some((block, inner));
        }
    }
    None
}

/// Is a line a `plugins` entry this build allows? The module's only the convention plugin; the root
/// may also turn off the catalog's plugins for the projects that apply them.
fn plugin_entry(line: &str, kind: Kind) -> bool {
    match kind {
        Kind::Module => line == MODULE_PLUGIN,
        Kind::Root => line == ROOT_PLUGIN || catalog_plugin_alias(line),
    }
}

/// `alias(libs.plugins.<alias>) apply false`: the one way the root makes a catalog plugin available
/// without applying it — the accessor's spelling of any catalog alias.
fn catalog_plugin_alias(line: &str) -> bool {
    let Some(inner) = line.strip_prefix("alias(").and_then(|rest| rest.strip_suffix(") apply false")) else { return false };
    inner.strip_prefix("libs.plugins.").is_some_and(catalog_accessor)
}

/// The accessor Gradle generates for a version-catalog alias, after `libs.` or `libs.plugins.`: Gradle's
/// own rule for an alias — it starts with a lowercase ASCII letter, and its parts are ASCII letters and
/// digits — with each `-`, `_` or `.` separator written as a dot (`kotlin-android` and `kotlinAndroid`
/// become `kotlin.android` and `kotlinAndroid`). The root's plugin lines and a module's dependencies read
/// the same notion through this one definition.
fn catalog_accessor(accessor: &str) -> bool {
    accessor.starts_with(|c: char| c.is_ascii_lowercase())
        && accessor.split('.').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// Does a line name the module's namespace, and what segment after the fixed prefix?
fn namespace_segment(line: &str) -> Option<&str> {
    let quoted = line.strip_prefix(NAMESPACE_PREFIX)?;
    quoted.strip_suffix('"').filter(|segment| !segment.is_empty())
}

/// Is a line one dependency of the allowed shapes: an allowed configuration, and either a module of
/// this build or an alias of the catalog? A written-out coordinate is refused: versions come from the
/// catalog once, so no module pins one to itself.
fn dependency_entry(line: &str) -> bool {
    DEPENDENCY_CONFIGURATIONS
        .iter()
        .filter_map(|configuration| line.strip_prefix(&format!("{configuration}(")).and_then(|rest| rest.strip_suffix(')')))
        .any(|target| project_target(target) || libs_target(target))
}

/// `project(":core")`, `:module` and `:group:module` spellings alike.
fn project_target(target: &str) -> bool {
    let Some(path) = target.strip_prefix("project(\"").and_then(|rest| rest.strip_suffix("\")")) else { return false };
    let Some(path) = path.strip_prefix(':') else { return false };
    !path.is_empty() && path.split(':').all(|segment| !segment.is_empty() && segment.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

/// `libs.kotlin`, `libs.kotlinx.coroutines.android`, `libs.kotlinAndroid`: a catalog accessor.
fn libs_target(target: &str) -> bool {
    target.strip_prefix("libs.").is_some_and(catalog_accessor)
}

/// Refusing a line says what the file may hold and where what it tried to say lives instead.
fn shape_message(at: &str, kind: Kind) -> String {
    match kind {
        Kind::Module => format!(
            "{at}: a module's build file holds only comments, plugins with id(\"craftbridge.android.library\"), android with its namespace, and dependencies of project(\":…\") or libs.<alias>; anything else a build needs is configured once in android/sdk/build-logic"
        ),
        Kind::Root => format!(
            "{at}: the root build file holds only comments and a plugins block of alias(libs.plugins.<alias>) apply false lines and id(\"craftbridge.quality\"); anything else a build needs is configured once in android/sdk/build-logic"
        ),
    }
}

/// What is wrong with one block entry — nothing, or why it fails. The counters record the lines the
/// presence rules read.
fn entry_error(at: &str, line: &str, block: Inside, kind: Kind, module: Option<&str>, applied: &mut bool, namespaces: &mut usize) -> Option<String> {
    match block {
        Inside::Plugins => {
            if !plugin_entry(line, kind) {
                return Some(shape_message(at, kind));
            }
            let expected = match kind {
                Kind::Module => MODULE_PLUGIN,
                Kind::Root => ROOT_PLUGIN,
            };
            *applied |= line == expected;
            None
        }
        Inside::Android => match (namespace_segment(line), module) {
            (Some(segment), Some(folder)) if segment == folder => {
                *namespaces += 1;
                if *namespaces > 1 {
                    return Some(format!("{at}: a namespace already names this module: once is enough"));
                }
                None
            }
            (Some(segment_is), Some(folder)) => {
                Some(format!("{at}: the namespace's last segment must be `{folder}`, not `{segment_is}`: the module's folder names it (AGENTS §9)"))
            }
            _ => Some(shape_message(at, kind)),
        },
        Inside::Dependencies => {
            if dependency_entry(line) {
                None
            } else {
                Some(shape_message(at, kind))
            }
        }
        Inside::Outside => None,
    }
}

/// Read one build file's shape. `name` is how a message names the file; `module` is the folder whose
/// name the namespace's last segment must repeat, `None` for the root file.
fn violations(name: &str, source: &str, kind: Kind, module: Option<&str>) -> Vec<String> {
    let mut errors = Vec::new();
    // A carriage return that ends no line is a line break to some readers and not to others; no legal
    // line needs one, so the question of which reading Gradle takes never arises.
    for (index, line) in source.split('\n').enumerate() {
        if line.trim_end_matches('\r').contains('\r') {
            errors.push(format!("{name}:{}: a carriage return inside a line: end lines with a newline alone", index + 1));
        }
    }
    let mut inside = Inside::Outside;
    let mut opened = 0usize;
    let mut applied = false;
    let mut namespaces = 0usize;
    // Blocks a refused line opened and has not closed: their `}` closes them, not the allowed block
    // around them, so one refused block is reported once and not again at every brace after it. A
    // refused line has already failed the file, so this only keeps the messages pointing at it.
    let mut refused_open = 0usize;
    // Comments blanked, strings kept: the grammar reads a quoted path exactly as written, and a line
    // hidden in a comment is nobody's statement — and nobody's hole.
    for (index, stripped) in scan::strip_comments(source, scan::Language::Kotlin).lines().enumerate() {
        let line = stripped.trim();
        if line.is_empty() {
            continue;
        }
        let at = format!("{name}:{}", index + 1);
        if line == "}" && refused_open > 0 {
            refused_open -= 1;
            continue;
        }
        let errors_before = errors.len();
        match inside {
            Inside::Outside => {
                if line == "}" {
                    errors.push(format!("{at}: a `}}` with no block to close"));
                    continue;
                }
                match open_block(line, kind) {
                    Some((block, inner)) => {
                        inside = block;
                        opened = index + 1;
                        if let Some(inner) = inner {
                            errors.extend(entry_error(&at, inner, block, kind, module, &mut applied, &mut namespaces));
                            inside = Inside::Outside;
                        }
                    }
                    None => errors.push(shape_message(&at, kind)),
                }
            }
            block => {
                if line == "}" {
                    inside = Inside::Outside;
                    continue;
                }
                errors.extend(entry_error(&at, line, block, kind, module, &mut applied, &mut namespaces));
            }
        }
        if errors.len() > errors_before {
            refused_open += line.matches('{').count().saturating_sub(line.matches('}').count());
        }
    }
    if inside != Inside::Outside {
        errors.push(format!("{name}:{opened}: this block is never closed"));
    }
    // The lines the rules arrive on must be present, not merely permitted: a file that holds none of
    // them is an empty file where a module's rules should be, and Gradle would say so only later and
    // less clearly.
    if !applied {
        errors.push(match kind {
            Kind::Module => format!("{name}:1: a module's build file must apply its convention plugin ({MODULE_PLUGIN}): the rules the gate relies on live in android/sdk/build-logic"),
            Kind::Root => format!("{name}:1: the root build file must apply {ROOT_PLUGIN}: it formats and checks the shared Gradle scripts (android/sdk/build-logic)"),
        });
    }
    if kind == Kind::Module && namespaces != 1 {
        errors.push(format!("{name}:1: a module's build file must name its namespace once: {NAMESPACE_PREFIX}{}\"", module.unwrap_or("<module>")));
    }
    errors
}

/// The project paths `include("…")` statements name, as folder paths. The settings file is the
/// machinery's own and spells its includes as string literals; any other spelling is simply not seen
/// here — a module whose file exists is still read by shape, and one that skips the convention plugin
/// fails the settings guard at build time whatever this read.
fn included_folders(settings: &Path) -> Result<Vec<String>, String> {
    let text = scan::strip_comments(&scan::read_text(settings)?, scan::Language::Kotlin);
    let mut folders = Vec::new();
    for line in text.lines() {
        let Some(from) = line.find("include(") else { continue };
        let rest = line.get(from + "include(".len()..).unwrap_or_default();
        let rest = rest.get(..rest.find(')').unwrap_or(rest.len())).unwrap_or(rest);
        for value in rest.split('"').skip(1).step_by(2) {
            if let Some(path) = value.strip_prefix(':')
                && !path.is_empty()
            {
                folders.push(path.replace(':', "/"));
            }
        }
    }
    folders.sort();
    folders.dedup();
    Ok(folders)
}

/// The file Gradle reads a project's Kotlin build from.
const BUILD_FILE: &str = "build.gradle.kts";

/// The included build that holds the convention plugins: machinery, read by review, not by this rule.
const BUILD_LOGIC: &str = "build-logic";

/// Run the check over `android/sdk` when the tree has it: every module's and the root's build file by
/// shape, the root file and every included module's file present, no Groovy build file, and no other
/// place a build's setup could run from — a `buildSrc`, which Gradle builds and whose scripts it runs on
/// every run, or a settings file but the build's and `build-logic`'s. Names are compared exactly:
/// `rules: hygiene` refuses every other spelling of a name Gradle opens.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, String> {
    let sdk = root.join("android").join("sdk");
    if !sdk.join("settings.gradle.kts").is_file() {
        return Ok(Vec::new());
    }
    let mut errors = Vec::new();
    let files = scan::walk(&sdk, "")?;
    let mut build_sources = std::collections::BTreeSet::new();
    for found in &files {
        let Some(name) = found.path.file_name().and_then(|name| name.to_str()) else { continue };
        if found.components.first().map(String::as_str) == Some(BUILD_LOGIC) {
            continue;
        }
        let shown = crate::report::shown(root, &found.path);
        if let Some(depth) = found.components.iter().position(|component| component == "buildSrc") {
            build_sources.insert(format!("android/sdk/{}", found.components.get(..=depth).unwrap_or_default().join("/")));
            continue;
        }
        if (name == "settings.gradle.kts" || name == "settings.gradle") && !found.components.is_empty() {
            errors.push(format!(
                "{shown}: a settings file of a build of its own, whose scripts run outside the convention plugins: the SDK's one build is android/sdk, and shared setup lives in android/sdk/build-logic"
            ));
            continue;
        }
        if name.ends_with(".gradle") {
            errors.push(format!(
                "{shown}: Gradle reads this Groovy file in place of the `.kts` file of the same project, and this rule cannot read Groovy: the build is written in Kotlin DSL, and shared setup lives in android/sdk/build-logic"
            ));
            continue;
        }
        if name != BUILD_FILE {
            continue;
        }
        let source = scan::read_text(&found.path)?;
        let kind = if found.components.is_empty() { Kind::Root } else { Kind::Module };
        let module = match kind {
            Kind::Module => found.path.parent().and_then(|parent| parent.file_name()).and_then(|name| name.to_str()),
            Kind::Root => None,
        };
        errors.extend(violations(&shown, &source, kind, module));
    }
    errors.extend(build_sources.into_iter().map(|folder| {
        format!("{folder}: Gradle builds a buildSrc and runs its scripts on every run of the build, outside the convention plugins: shared setup lives in android/sdk/build-logic")
    }));
    if !sdk.join(BUILD_FILE).is_file() {
        errors.push("android/sdk/build.gradle.kts: the root build file is missing: it applies id(\"craftbridge.quality\"), which formats and checks the shared Gradle scripts".to_string());
    }
    for folder in included_folders(&sdk.join("settings.gradle.kts"))? {
        if !sdk.join(&folder).join(BUILD_FILE).is_file() {
            errors.push(format!("android/sdk/{folder}/build.gradle.kts: the settings include `:{}` but there is no build file: every module has one, applying the convention plugin (android/sdk/build-logic)", folder.replace('/', ":")));
        }
    }
    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    fn module(name: &str, text: &str) -> Vec<String> {
        violations(name, text, Kind::Module, name.rsplit('/').nth(1))
    }
    fn root(text: &str) -> Vec<String> {
        violations("android/sdk/build.gradle.kts", text, Kind::Root, None)
    }

    const LEGAL_MODULE: &str = r#"// The effects plugin: the core, and nothing else.

plugins {
    id("craftbridge.android.library")
}

android {
    namespace = "io.github.christopherlinnett.craftbridge.effects"
}

dependencies {
    // the engine's public surface arrives through the core alone
    api(project(":core"))
}
"#;

    const LEGAL_ROOT: &str = r#"// The root holds nothing but the plugins, so every module gets them the same way.
plugins {
    alias(libs.plugins.android.library) apply false
    alias(libs.plugins.kotlin.android) apply false
    id("craftbridge.quality")
}
"#;

    #[test]
    fn a_legal_module_file_passes() {
        let errors = module("android/sdk/effects/build.gradle.kts", LEGAL_MODULE);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_legal_root_file_passes() {
        let errors = root(LEGAL_ROOT);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn every_shape_the_rule_names_passes() {
        let compact = module(
            "android/sdk/core/build.gradle.kts",
            "plugins { id(\"craftbridge.android.library\") }\nandroid { namespace = \"io.github.christopherlinnett.craftbridge.core\" }\ndependencies { api(project(\":core\")) }\n",
        );
        assert!(compact.is_empty(), "{compact:?}");
    }

    #[test]
    fn a_trailing_comma_is_not_a_statement_and_is_refused() {
        // Gradle's script compiler rejects `id("…"),` and `api(…),` outright: the Kotlin DSL has no
        // statement-list commas. The grammar allows only what would compile.
        let errors = module(
            "android/sdk/core/build.gradle.kts",
            "plugins {\n    id(\"craftbridge.android.library\"),\n}\nandroid {\n    namespace = \"io.github.christopherlinnett.craftbridge.core\"\n}\n",
        );
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/core/build.gradle.kts:2:")), "{errors:?}");
        let errors = module(
            "android/sdk/core/build.gradle.kts",
            "plugins { id(\"craftbridge.android.library\") }\nandroid { namespace = \"io.github.christopherlinnett.craftbridge.core\" }\ndependencies { api(project(\":core\")), }\n",
        );
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/core/build.gradle.kts:3:")), "{errors:?}");
    }

    #[test]
    fn a_kotlin_char_literal_hides_nothing_from_the_grammar() {
        // Read the Rust way, `'\u0041'` ran on to the next `}'` and blanked every line between, so the
        // `afterEvaluate`, `gradle.` and `tasks` lines below passed unread. A char literal is kept, and a
        // line holding one is no line the grammar allows.
        let plant = "'\\u0041'.let(::plant)\n\nfun plant(c: Char) = afterEvaluate(::late)\n\nfun late(project: Project) = gradle.taskGraph.whenReady(::ready)\n\n\
                     fun ready(graph: org.gradle.api.execution.TaskExecutionGraph) = tasks.configureEach(::quiet)\n\nval close = '}'\n";
        let file = "android/sdk/core/build.gradle.kts";
        let errors = module(file, &format!("{LEGAL_MODULE}{plant}").replace("effects", "core"));
        for line in [15, 17, 19, 21, 23] {
            assert!(errors.iter().any(|e| e.starts_with(&format!("{file}:{line}:"))), "line {line}: {errors:?}");
        }
    }

    #[test]
    fn a_carriage_return_inside_a_line_is_refused() {
        let file = "android/sdk/effects/build.gradle.kts";
        let errors = module(file, &LEGAL_MODULE.replace("// the engine's public surface", "// the engine's\rpublic surface"));
        assert!(errors.len() == 1 && errors[0].starts_with(&format!("{file}:12: a carriage return")), "{errors:?}");
        assert!(module(file, &LEGAL_MODULE.replace('\n', "\r\n")).is_empty(), "CRLF line endings are line endings");
    }

    #[test]
    fn each_line_the_rule_forbids_fails_naming_the_line() {
        for forbidden in [
            "tasks.withType<KotlinCompilationTask<*>>().configureEach { compilerOptions.allWarningsAsErrors.set(false) }",
            "tasks.named(\"lintDebug\") { onlyIf { false } }",
            "afterEvaluate { tasks.named(\"spotlessCheck\") { setDependsOn(emptyList()) } }",
            "gradle.projectsEvaluated { tasks.withType<SpotlessTask>().configureEach { setTarget(files()) } }",
            "sourceSets { getByName(\"main\") { assets.srcDir(\"/tmp/elsewhere\") } }",
            "lint { disable += \"MissingClass\" }",
            "kotlin { explicitApi = null }",
        ] {
            let file = "android/sdk/effects/build.gradle.kts";
            let errors = module(file, &format!("{LEGAL_MODULE}\n{forbidden}\n"));
            assert!(errors.iter().any(|e| e.starts_with(&format!("{file}:16:"))), "{forbidden} not refused at its line: {errors:?}");
            assert!(errors.iter().any(|e| e.contains("build-logic")), "{forbidden}: {errors:?}");
        }
    }

    #[test]
    fn trouble_reached_for_from_inside_the_allowed_blocks_fails_there() {
        // the shapes an author might reach for within android{} and dependencies{}
        let errors = module(
            "android/sdk/effects/build.gradle.kts",
            "plugins {\n    id(\"craftbridge.android.library\")\n}\nandroid {\n    namespace = \"io.github.christopherlinnett.craftbridge.effects\"\n    lint { disable += \"MissingClass\" }\n}\ndependencies {\n    implementation(\"androidx.annotation:annotation:1.9.1\")\n}\n",
        );
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/effects/build.gradle.kts:6:")), "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/effects/build.gradle.kts:9:")), "{errors:?}");
    }

    #[test]
    fn the_root_file_refuses_any_statement_outside_its_plugins() {
        let errors = root(&format!("{LEGAL_ROOT}\nprintln(\"hello\")\n"));
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/build.gradle.kts:8:")), "{errors:?}");
        let errors = root(&format!("android {{ namespace = \"x\" }}\n{LEGAL_ROOT}"));
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/build.gradle.kts:1:")), "{errors:?}");
    }

    #[test]
    fn the_lines_that_bring_the_rules_must_be_present() {
        // an empty or comment-only file says nothing and applies nothing: the shape requires the lines
        let errors = module("android/sdk/core/build.gradle.kts", "// nothing here\n");
        assert!(errors.iter().any(|e| e.contains("convention plugin")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("namespace")), "{errors:?}");
        let errors = root("");
        assert!(errors.iter().any(|e| e.contains("id(\"craftbridge.quality\")")), "{errors:?}");
    }

    #[test]
    fn the_namespace_follows_the_folder() {
        let errors = module("android/sdk/core/build.gradle.kts", &LEGAL_MODULE.replace("craftbridge.effects", "craftbridge.wrong"));
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/core/build.gradle.kts:8:") && e.contains("must be `core`")), "{errors:?}");
        // and the prefix must be the one AGENTS §9 names
        let errors = module("android/sdk/core/build.gradle.kts", &LEGAL_MODULE.replace("io.github.christopherlinnett.craftbridge.effects", "com.other.core"));
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/core/build.gradle.kts:8:")), "{errors:?}");
        // twice is as wrong as never
        let twice = format!("android {{ namespace = \"io.github.christopherlinnett.craftbridge.core\" }}\n{LEGAL_MODULE}")
            .replace("craftbridge.effects", "craftbridge.core");
        let errors = module("android/sdk/core/build.gradle.kts", &twice);
        assert!(errors.iter().any(|e| e.contains("once is enough")), "{errors:?}");
    }

    #[test]
    fn blocks_must_close_and_nothing_closes_on_its_own() {
        let errors = module("android/sdk/core/build.gradle.kts", "plugins {\n    id(\"craftbridge.android.library\")\n");
        // the file ends inside the plugins block
        assert!(errors.iter().any(|e| e.contains(":1: this block is never closed")), "{errors:?}");
        let errors = module("android/sdk/core/build.gradle.kts", &format!("{LEGAL_MODULE}\n}}\n"));
        assert!(errors.iter().any(|e| e.contains(":16: a `}` with no block")), "{errors:?}");
    }

    #[test]
    fn the_grammar_reads_statements_not_prose() {
        // commented-out trouble is still comments; commented-in too
        let commented = module("android/sdk/effects/build.gradle.kts", &format!("{LEGAL_MODULE}// sourceSets {{ ... }}\n"));
        assert!(commented.is_empty(), "{commented:?}");
        let quoted = module("android/sdk/effects/build.gradle.kts", &format!("{LEGAL_MODULE}val note = \"tasks\"\n"));
        assert!(quoted.iter().any(|e| e.starts_with("android/sdk/effects/build.gradle.kts:15:")), "{quoted:?}");
    }

    #[test]
    fn a_tree_without_an_android_build_has_nothing_to_say() {
        let scratch = Scratch::new("no-android");
        assert!(check(&scratch.0).unwrap().is_empty());
    }

    #[test]
    fn build_logic_is_the_machinery_and_is_not_read_as_a_module() {
        let scratch = setup();
        scratch.write("android/sdk/build-logic/convention/build.gradle.kts", "tasks.register(\"anything\") { }\n");
        let errors = check(&scratch.0).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn a_missing_module_build_file_fails_the_rules_stage() {
        // the hole that needed only the gradle stage to catch: an include with no file
        let scratch = setup();
        std::fs::remove_file(scratch.0.join("android/sdk/core/build.gradle.kts")).unwrap();
        let errors = check(&scratch.0).unwrap();
        assert_eq!(
            errors,
            [
                "android/sdk/core/build.gradle.kts: the settings include `:core` but there is no build file: every module has one, applying the convention plugin (android/sdk/build-logic)"
            ]
        );
    }

    #[test]
    fn a_missing_root_build_file_is_an_error() {
        let scratch = setup();
        std::fs::remove_file(scratch.0.join("android/sdk/build.gradle.kts")).unwrap();
        let errors = check(&scratch.0).unwrap();
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/build.gradle.kts: the root build file is missing")), "{errors:?}");
    }

    #[test]
    fn a_groovy_build_file_is_refused_before_gradle_can_prefer_it() {
        let scratch = setup();
        scratch.write("android/sdk/effects/build.gradle", "tasks.named(\"lintDebug\") { onlyIf { false } }\n");
        scratch.write("android/sdk/settings.gradle", "rootProject.name = \"away\"\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().all(|e| e.contains("Groovy")), "{errors:?}");
    }

    /// A case-insensitive disk hands Gradle `Build.gradle.kts`, `BUILD.GRADLE`, or a name carrying the Kelvin
    /// sign or a long s, where it opens `build.gradle.kts` or `build.gradle` (running Gradle on this APFS
    /// volume proved the folds). `rules: hygiene` refuses every such spelling the disk folds, and every
    /// ASCII-case one on any disk, so this rule reads the exact names only. Where the disk does not fold the
    /// name, Gradle does not open the file either, and the module simply has no build file.
    #[test]
    fn a_build_file_spelled_otherwise_is_refused_by_hygiene() {
        for spelling in ["Build.gradle.kts", "build.gradle.\u{212A}ts", "build.gradle.kt\u{017F}", "BUILD.GRADLE"] {
            let scratch = setup();
            std::fs::remove_file(scratch.0.join("android/sdk/effects/build.gradle.kts")).unwrap();
            scratch.write(&format!("android/sdk/effects/{spelling}"), format!("{LEGAL_MODULE}tasks.named(\"lintDebug\") {{ onlyIf {{ false }} }}\n"));
            let plain = if spelling.eq_ignore_ascii_case("BUILD.GRADLE") { "build.gradle" } else { "build.gradle.kts" };
            let folded = std::fs::canonicalize(scratch.0.join("android/sdk/effects").join(plain)).ok()
                == std::fs::canonicalize(scratch.0.join("android/sdk/effects").join(spelling)).ok();
            let refused = crate::hygiene::check(&scratch.0).unwrap().iter().any(|e| e.starts_with(&format!("android/sdk/effects/{spelling}: spell")));
            assert_eq!(refused, folded || spelling.eq_ignore_ascii_case(plain), "{spelling}");
            if !folded && plain == "build.gradle.kts" {
                let errors = check(&scratch.0).unwrap();
                assert!(
                    errors.iter().any(|e| e.contains("effects/build.gradle.kts: the settings include") && e.contains("no build file")),
                    "{spelling}: {errors:?}"
                );
            }
        }
    }

    #[test]
    fn a_build_source_or_another_settings_file_is_refused() {
        let scratch = setup();
        scratch.write("android/sdk/buildSrc/settings.gradle.kts", "gradle.parent?.let { it.projectsEvaluated { } }\n");
        scratch.write("android/sdk/buildSrc/src/main/kotlin/Late.kt", "\n");
        scratch.write("android/sdk/core/settings.gradle.kts", "rootProject.name = \"apart\"\n");
        scratch.write("android/sdk/build-logic/settings.gradle.kts", "rootProject.name = \"build-logic\"\n");
        let errors = check(&scratch.0).unwrap();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/buildSrc: Gradle builds a buildSrc")), "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("android/sdk/core/settings.gradle.kts: a settings file of a build of its own")), "{errors:?}");
    }

    #[test]
    fn a_catalog_accessor_is_read_by_gradle_s_alias_rule_in_the_root_and_in_a_module() {
        for accessor in ["kotlin.android", "kotlinAndroid", "spotless.gradle.plugin", "androidx.core.ktx2"] {
            assert!(catalog_plugin_alias(&format!("alias(libs.plugins.{accessor}) apply false")), "{accessor}");
            assert!(libs_target(&format!("libs.{accessor}")), "{accessor}");
        }
        for accessor in ["", "Kotlin", "kotlin..android", "kotlin.", "kotlin-android", "kotlin_android", "4kotlin"] {
            assert!(!catalog_plugin_alias(&format!("alias(libs.plugins.{accessor}) apply false")), "{accessor:?}");
            assert!(!libs_target(&format!("libs.{accessor}")), "{accessor:?}");
        }
    }

    #[test]
    fn a_refused_block_is_reported_at_its_lines_and_its_braces_close_it() {
        let nested = LEGAL_MODULE.replace(
            "    namespace = \"io.github.christopherlinnett.craftbridge.effects\"\n",
            "    namespace = \"io.github.christopherlinnett.craftbridge.effects\"\n    sourceSets {\n        getByName(\"main\") {\n            assets.directories.add(\"../x\")\n        }\n    }\n",
        );
        let errors = module("android/sdk/effects/build.gradle.kts", &nested);
        let lines: Vec<&str> = errors.iter().map(|e| e.split(": ").next().unwrap_or_default()).collect();
        assert_eq!(
            lines,
            ["android/sdk/effects/build.gradle.kts:9", "android/sdk/effects/build.gradle.kts:10", "android/sdk/effects/build.gradle.kts:11"],
            "{errors:?}"
        );
    }

    #[test]
    fn a_missing_namespace_is_named_with_the_line_to_write() {
        let errors = module("android/sdk/core/build.gradle.kts", "plugins {\n    id(\"craftbridge.android.library\")\n}\n");
        assert_eq!(
            errors,
            [
                "android/sdk/core/build.gradle.kts:1: a module's build file must name its namespace once: namespace = \"io.github.christopherlinnett.craftbridge.core\""
            ]
        );
    }

    #[test]
    fn a_module_hiding_as_build_logic_is_no_hole() {
        // the exclusion is for the included build beside the modules, not for any folder that borrows
        // its name: one at a module's depth is read as a module, and its junk is refused
        let scratch = setup();
        scratch.write("android/sdk/elsewhere/build-logic/build.gradle.kts", "tasks.register(\"anything\") { }\n");
        let errors = check(&scratch.0).unwrap();
        assert!(errors.iter().any(|e| e.contains("android/sdk/elsewhere/build-logic/build.gradle.kts:1:")), "{errors:?}");
    }

    /// A small sdk tree shaped like the repository's.
    fn setup() -> Scratch {
        let scratch = Scratch::new("buildfiles");
        scratch.write("android/sdk/settings.gradle.kts", "include(\":core\", \":effects\")\n");
        scratch.write("android/sdk/build.gradle.kts", LEGAL_ROOT);
        scratch.write(
            "android/sdk/core/build.gradle.kts",
            "plugins {\n    id(\"craftbridge.android.library\")\n}\n\nandroid {\n    namespace = \"io.github.christopherlinnett.craftbridge.core\"\n}\n",
        );
        scratch.write("android/sdk/effects/build.gradle.kts", LEGAL_MODULE);
        scratch
    }

    #[test]
    fn the_check_reads_a_real_shaped_tree() {
        let scratch = setup();
        let errors = check(&scratch.0).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn the_repository_passes() {
        let errors = check(&crate::repo_root()).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
    }
}
