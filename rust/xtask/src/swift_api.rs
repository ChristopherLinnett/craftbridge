//! The Swift stage's check of the names hand-written Swift gives beside generated Swift, `cargo xtask
//! swift-api <graphs>…`: no public member written by hand takes the base name of a public member that
//! generated Swift declares on the same type, such as a command namespace's property on `EffectsEngine`
//! or a command's method on that namespace's type (AGENTS §9, plan §22).
//!
//! The compiler refuses only a member whose whole name another already has: a property beside a property,
//! or a call with no arguments beside a property of its name. A method with arguments stands beside a
//! property, or a method with other labels, of the same base name, and a call then reaches whichever the
//! compiler ranks first, so a hand-written `script(_:)` beside the generated `script` namespace, or a
//! hand-written `templates.list()` beside the generated `templates.list(thumbnails:extra:)`, would build
//! and quietly take the generated name's calls or stand in its place. So the names are read from the
//! compiler's own declarations: the symbol graphs every build of the `swift_build` stage emits, in which
//! each symbol names the type it is a member of by its precise identifier, whatever alias an extension
//! spelt it through, and the file it is declared in, which says whether a generator wrote it (AGENTS §6).
//!
//! Every graph of a module the package builds from its own sources is read, its test targets' aside, since
//! a test's extension ships in no app. A build folder that lacks the graph of such a module fails the check,
//! and so does a graph file that is not shaped as one: each would read as a module that declares nothing,
//! and a check that read nothing must not pass.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

/// The ending of every symbol graph file the compiler writes: `<module>.symbols.json` for the module's own
/// declarations and `<module>@<other>.symbols.json` for its extensions of another module's types.
const GRAPH_ENDING: &str = ".symbols.json";

/// The access levels a member of the API an app sees has.
const PUBLIC: &[&str] = &["public", "open"];

/// One public member, as one graph declares it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Member {
    /// The precise identifier of the type it is a member of.
    parent: String,
    /// Its name up to its argument labels: `script` for `script(_:)`.
    base: String,
    /// Its full path, as the compiler gives it: `EffectsEngine.TemplatesCommands.list()`.
    shown: String,
    /// The file it is declared in, as the repository names it, with its line and column counted from 1.
    file: String,
    line: u64,
    column: u64,
    /// Whether the file lies in a generated folder.
    generated: bool,
}

/// Check the graphs each of `folders` holds, one folder for each build, against the package's modules:
/// one line for each clash, and success only when there is none.
pub(crate) fn run(root: &Path, folders: &[&str]) -> ExitCode {
    match check(root, folders) {
        Ok(findings) => {
            for finding in &findings {
                println!("{finding}");
            }
            if findings.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) }
        }
        Err(failure) => crate::report::failed("swift-api", failure),
    }
}

/// What [`run`] prints: every clash, or why the graphs could not be read.
fn check(root: &Path, folders: &[&str]) -> Result<Vec<String>, String> {
    if folders.is_empty() {
        return Err("no folder of symbol graphs to read: name one for each build".to_string());
    }
    let canonical = std::fs::canonicalize(root).map_err(|error| format!("cannot resolve the repository's path: {error}"))?;
    let manifest = crate::direction::FILE;
    let package = canonical.join(Path::new(manifest).parent().unwrap_or_else(|| Path::new("")));
    let targets = crate::direction::describe_package(&package, manifest)?.ok_or_else(|| format!("{manifest}: no package to read"))?;
    let modules: BTreeSet<String> =
        targets.iter().filter(|target| target.kind != "test" && target.kind != "binary").map(|target| target.name.clone()).collect();
    let mut findings = BTreeSet::new();
    for folder in folders {
        let graphs = read_folder(Path::new(folder), &modules)?;
        let mut members = Vec::new();
        for graph in &graphs {
            members.extend(members_of(&canonical, graph));
        }
        findings.extend(clashes(&members));
    }
    Ok(findings.into_iter().collect())
}

/// The graphs in `folder` that modules of `modules` emitted, refusing a folder that lacks any module's own.
fn read_folder(folder: &Path, modules: &BTreeSet<String>) -> Result<Vec<Value>, String> {
    let entries = std::fs::read_dir(folder).map_err(|error| format!("{}: cannot read the symbol graphs: {error}", folder.display()))?;
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let path = entry.map_err(|error| format!("{}: cannot read the symbol graphs: {error}", folder.display()))?.path();
        if path.file_name().and_then(|name| name.to_str()).and_then(emitting_module).is_some_and(|module| modules.contains(module)) {
            files.push(path);
        }
    }
    let missing = missing_graphs(&files, modules);
    if !missing.is_empty() {
        return Err(format!(
            "{}: no symbol graph of {}, so the build read none of its names: build again from a clean build folder, which compiles every module and emits its graph",
            folder.display(),
            missing.join(", ")
        ));
    }
    files.sort();
    files
        .iter()
        .map(|file| {
            let text = crate::scan::read_text(file)?;
            let graph: Value =
                serde_json::from_str(&text).map_err(|error| format!("{}: is not a symbol graph this check can read: {error}", file.display()))?;
            // The names of a module are weighed through the two lists every graph the compiler writes
            // holds, even a module that declares no name: a file of some other shape would read as such a
            // module, and a check that read none of a module's names must not pass.
            for key in ["symbols", "relationships"] {
                if !graph.get(key).is_some_and(Value::is_array) {
                    return Err(format!("{}: is not a symbol graph this check can read: it holds no `{key}` list", file.display()));
                }
            }
            Ok(graph)
        })
        .collect()
}

/// The module that wrote a graph file named `name`: what comes before `@` or the ending, or `None` for a
/// file that is no graph.
fn emitting_module(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(GRAPH_ENDING)?;
    Some(stem.split_once('@').map_or(stem, |(module, _)| module))
}

/// The modules of `modules` whose own graph, `<module>.symbols.json`, none of `files` is.
fn missing_graphs(files: &[PathBuf], modules: &BTreeSet<String>) -> Vec<String> {
    let present: BTreeSet<&str> = files.iter().filter_map(|file| file.file_name()?.to_str()?.strip_suffix(GRAPH_ENDING)).collect();
    modules.iter().filter(|module| !present.contains(module.as_str())).cloned().collect()
}

/// The public members `graph` declares in this repository's files, each with the type it is a member of.
/// A member the compiler declares itself, which no file of ours holds — one a protocol's extension lends, a
/// synthesized conformance — has no name of ours to weigh.
fn members_of(root: &Path, graph: &Value) -> Vec<Member> {
    let symbols: BTreeMap<&str, &Value> =
        list(graph, "symbols").filter_map(|symbol| Some((symbol.get("identifier")?.get("precise")?.as_str()?, symbol))).collect();
    list(graph, "relationships")
        .filter(|relationship| relationship.get("kind").and_then(Value::as_str) == Some("memberOf"))
        .filter_map(|relationship| {
            let parent = relationship.get("target")?.as_str()?;
            let symbol = symbols.get(relationship.get("source")?.as_str()?)?;
            member(root, parent, symbol)
        })
        .collect()
}

/// The items of `graph`'s array `key`.
fn list<'a>(graph: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    graph.get(key).and_then(Value::as_array).into_iter().flatten()
}

/// `symbol` as a public member of `parent`, when it is public and one of this repository's files declares it.
fn member(root: &Path, parent: &str, symbol: &Value) -> Option<Member> {
    if !PUBLIC.contains(&symbol.get("accessLevel")?.as_str()?) {
        return None;
    }
    let title = symbol.get("names")?.get("title")?.as_str()?;
    let location = symbol.get("location")?;
    let path = PathBuf::from(crate::swift_lint::path_of(location.get("uri")?.as_str()?)?);
    let relative = match path.strip_prefix(root) {
        Ok(relative) => relative.to_path_buf(),
        Err(_) => std::fs::canonicalize(&path).ok()?.strip_prefix(root).ok()?.to_path_buf(),
    };
    let folders: Vec<String> = relative.parent().into_iter().flat_map(Path::components).map(|part| part.as_os_str().to_string_lossy().into_owned()).collect();
    let position = location.get("position")?;
    let shown: Vec<&str> = symbol.get("pathComponents")?.as_array()?.iter().filter_map(Value::as_str).collect();
    Some(Member {
        parent: parent.to_string(),
        base: title.split_once('(').map_or(title, |(base, _)| base).to_string(),
        shown: shown.join("."),
        file: relative.to_string_lossy().replace('\\', "/"),
        line: position.get("line")?.as_u64()?.saturating_add(1),
        column: position.get("character")?.as_u64()?.saturating_add(1),
        generated: crate::generated::in_generated(&folders),
    })
}

/// Each hand-written member that takes the base name of a generated member of the same type.
fn clashes(members: &[Member]) -> Vec<String> {
    let mut generated: BTreeMap<(&str, &str), &Member> = BTreeMap::new();
    for member in members.iter().filter(|member| member.generated) {
        generated.entry((member.parent.as_str(), member.base.as_str())).or_insert(member);
    }
    members
        .iter()
        .filter(|member| !member.generated)
        .filter_map(|member| {
            let taken = generated.get(&(member.parent.as_str(), member.base.as_str()))?;
            Some(format!(
                "{}:{}:{}: `{}` is written by hand under the name `{}`, which the generated `{}` ({}) already gives its type: a call the generated Swift names stands beside it and the compiler picks between them, so a hand-written call takes a name of its own, as an extension of a namespace's generated type when it belongs to that namespace (plan §22)",
                member.file, member.line, member.column, member.shown, member.base, taken.shown, taken.file
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/repo";
    const ENGINE: &str = "s:18CraftBridgeEffects0B6EngineC";
    const TEMPLATES: &str = "s:18CraftBridgeEffects0B6EngineC17TemplatesCommandsV";

    /// A graph of `symbols`, each `(id, parent, title, path, file, access)`, the parent named by a `memberOf`.
    fn graph(symbols: &[(&str, &str, &str, &str, &str, &str)]) -> Value {
        let declared: Vec<Value> = symbols
            .iter()
            .map(|(id, _, title, path, file, access)| {
                serde_json::json!({
                    "identifier": {"precise": id, "interfaceLanguage": "swift"},
                    "names": {"title": title},
                    "pathComponents": path.split('.').collect::<Vec<_>>(),
                    "accessLevel": access,
                    "location": {"uri": format!("file://{ROOT}/{file}"), "position": {"line": 4, "character": 15}},
                })
            })
            .collect();
        let relationships: Vec<Value> =
            symbols.iter().map(|(id, parent, ..)| serde_json::json!({"kind": "memberOf", "source": id, "target": parent})).collect();
        serde_json::json!({"symbols": declared, "relationships": relationships})
    }

    fn findings(symbols: &[(&str, &str, &str, &str, &str, &str)]) -> Vec<String> {
        clashes(&members_of(Path::new(ROOT), &graph(symbols)))
    }

    const NAMESPACE: (&str, &str, &str, &str, &str, &str) =
        ("s:ns", ENGINE, "script", "EffectsEngine.script", "ios/sdk/Sources/CraftBridgeEffects/Generated/ScriptCommands.swift", "public");
    const COMMAND: (&str, &str, &str, &str, &str, &str) = (
        "s:list",
        TEMPLATES,
        "list(thumbnails:extra:)",
        "EffectsEngine.TemplatesCommands.list(thumbnails:extra:)",
        "ios/sdk/Sources/CraftBridgeEffects/Generated/TemplatesCommands.swift",
        "public",
    );

    #[test]
    fn a_hand_written_call_named_like_a_namespace_of_its_type_is_refused_naming_both() {
        let script = ("s:call", ENGINE, "script(_:)", "EffectsEngine.script(_:)", "ios/sdk/Sources/CraftBridgeEffects/EngineCalls.swift", "public");
        assert_eq!(
            findings(&[NAMESPACE, script]),
            vec![
                "ios/sdk/Sources/CraftBridgeEffects/EngineCalls.swift:5:16: `EffectsEngine.script(_:)` is written by hand under the name `script`, which the generated `EffectsEngine.script` (ios/sdk/Sources/CraftBridgeEffects/Generated/ScriptCommands.swift) already gives its type: a call the generated Swift names stands beside it and the compiler picks between them, so a hand-written call takes a name of its own, as an extension of a namespace's generated type when it belongs to that namespace (plan §22)"
            ]
        );
    }

    #[test]
    fn a_hand_written_call_named_like_a_command_of_its_namespace_is_refused_whatever_its_labels() {
        for title in ["list()", "list(_:)", "list"] {
            let list = ("s:mine", TEMPLATES, title, "EffectsEngine.TemplatesCommands.list", "ios/sdk/Sources/CraftBridgeEffects/Templates.swift", "public");
            let found = findings(&[COMMAND, list]);
            assert_eq!(found.len(), 1, "{title}: {found:?}");
            assert!(found.iter().all(|line| line.starts_with("ios/sdk/Sources/CraftBridgeEffects/Templates.swift:5:16: ")), "{found:?}");
        }
    }

    #[test]
    fn a_name_of_its_own_another_types_name_and_two_generated_names_pass() {
        let open = (
            "s:open",
            TEMPLATES,
            "open(_:values:)",
            "EffectsEngine.TemplatesCommands.open(_:values:)",
            "ios/sdk/Sources/CraftBridgeEffects/Templates.swift",
            "public",
        );
        // The engine's own `list` is no member of the templates namespace, whose `list` it cannot stand beside.
        let elsewhere = ("s:else", ENGINE, "list(_:)", "EffectsEngine.list(_:)", "ios/sdk/Sources/CraftBridgeEffects/EngineCalls.swift", "public");
        // A command and the namespace below it may share a name: both are the generator's, which weighs its own.
        let pair = (
            "s:pair",
            TEMPLATES,
            "list",
            "EffectsEngine.TemplatesCommands.list",
            "ios/sdk/Sources/CraftBridgeEffects/Generated/TemplatesListCommands.swift",
            "public",
        );
        assert_eq!(findings(&[NAMESPACE, COMMAND, open, elsewhere, pair]), Vec::<String>::new());
    }

    #[test]
    fn a_member_no_app_sees_or_no_file_of_ours_declares_is_not_weighed() {
        let internal = ("s:int", ENGINE, "script(_:)", "EffectsEngine.script(_:)", "ios/sdk/Sources/CraftBridgeEffects/EngineCalls.swift", "internal");
        let package = ("s:pkg", ENGINE, "script(_:)", "EffectsEngine.script(_:)", "ios/sdk/Sources/CraftBridgeEffects/EngineCalls.swift", "package");
        assert_eq!(findings(&[NAMESPACE, internal, package]), Vec::<String>::new());
        let mut lent = graph(&[NAMESPACE, ("s:lent", ENGINE, "script(_:)", "EffectsEngine.script(_:)", "x.swift", "public")]);
        if let Some(fields) = lent.pointer_mut("/symbols/1").and_then(Value::as_object_mut) {
            fields.remove("location");
        }
        assert_eq!(clashes(&members_of(Path::new(ROOT), &lent)), Vec::<String>::new());
        assert!(members_of(Path::new("/elsewhere"), &graph(&[NAMESPACE])).is_empty(), "a file outside the repository is no file of ours");
    }

    #[test]
    fn an_open_member_is_public_and_a_percent_encoded_path_is_read_as_its_file() {
        let mut open =
            graph(&[NAMESPACE, ("s:open", ENGINE, "script(_:)", "EffectsEngine.script(_:)", "ios/sdk/Sources/Craft%42ridgeEffects/A.swift", "open")]);
        let found = clashes(&members_of(Path::new(ROOT), &open));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found.iter().all(|line| line.starts_with("ios/sdk/Sources/CraftBridgeEffects/A.swift:5:16: ")), "{found:?}");
        open = graph(&[NAMESPACE]);
        assert_eq!(members_of(Path::new(ROOT), &open).first().map(|member| member.generated), Some(true));
    }

    #[test]
    fn a_graph_file_names_the_module_that_wrote_it_and_a_folder_must_hold_each_modules_own() {
        assert_eq!(emitting_module("CraftBridgeEffects.symbols.json"), Some("CraftBridgeEffects"));
        assert_eq!(emitting_module("CraftBridge@CraftBridgeFFI.symbols.json"), Some("CraftBridge"));
        assert_eq!(emitting_module("CraftBridge.swiftmodule"), None);
        let modules: BTreeSet<String> = ["CraftBridge", "CraftBridgeEffects"].into_iter().map(str::to_string).collect();
        let only_an_extension = vec![PathBuf::from("/g/CraftBridge.symbols.json"), PathBuf::from("/g/CraftBridgeEffects@CraftBridge.symbols.json")];
        assert_eq!(missing_graphs(&only_an_extension, &modules), vec!["CraftBridgeEffects".to_string()]);
        let scratch = crate::scratch::Scratch::new("swift-api");
        let empty_graph = "{\"symbols\": [], \"relationships\": []}";
        scratch.write("CraftBridge.symbols.json", empty_graph);
        let error = read_folder(&scratch.0, &modules).unwrap_err();
        assert!(error.contains("no symbol graph of CraftBridgeEffects, so the build read none of its names"), "{error}");
        scratch.write("CraftBridgeEffects.symbols.json", empty_graph);
        scratch.write("CraftBridgeEffectsTests.symbols.json", "not JSON: a test target's graph is not read");
        assert_eq!(read_folder(&scratch.0, &modules).unwrap().len(), 2);
        // A file that is JSON but no symbol graph is no graph: read as a module that declares nothing,
        // it would let a hand-written member of that module take a generated name with nothing to weigh.
        for no_graph in ["{}", "[]", "null", "42"] {
            scratch.write("CraftBridgeEffects.symbols.json", no_graph);
            let error = read_folder(&scratch.0, &modules).unwrap_err();
            assert!(error.contains("is not a symbol graph this check can read: it holds no `symbols` list"), "{no_graph}: {error}");
        }
        scratch.write("CraftBridgeEffects.symbols.json", "{\"symbols\": [], \"relationships\": {}}");
        let error = read_folder(&scratch.0, &modules).unwrap_err();
        assert!(error.contains("it holds no `relationships` list"), "{error}");
    }
}
