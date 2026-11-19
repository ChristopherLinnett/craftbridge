//! The boundary's surface, as the smoke checks' one list of what to call.
//!
//! The boundary is what the generated bindings hand an app: every function the `-ffi` crates export,
//! every constructor and method of an object, a record or an enum, every UniFFI trait (`Debug`,
//! `Display`, `Eq`, `Hash`, `Ord`) one of those exports, which the bindings turn into each language's
//! own conformance, and every method of a callback interface. UniFFI writes that surface into the
//! distribution library as metadata, and the workspace's bindgen reads it back to write the bindings
//! ([`crate::bindings`]).
//! This reads the same metadata from the same library, which is the surface's one home, and writes it
//! out as JSON, so what the device smoke checks must call comes from the library itself and never from
//! a second hand-maintained list: an export added later appears here without any check being touched,
//! and each smoke check then fails for the name it never calls (plan §20, QA-2d).
//!
//! The file sits inside the corpus's folder, in a `generated` folder of its own, because the corpus
//! folder is already the instrumented checks' assets: the check on the emulator reads the manifest
//! from them, and the one on the simulator reads it from this checkout, exactly as both read the corpus.

use std::collections::BTreeSet;
use std::path::Path;

use camino::Utf8Path;
use serde_json::Value;
use uniffi_bindgen::BindgenCrateConfigSupplier;
use uniffi_bindgen::interface::ffi::FfiFunction;
use uniffi_bindgen::interface::{ComponentInterface, Constructor, Method, UniffiTraitMethods};

/// The folder this generator owns, inside the corpus's, so the emulator's instrumented checks get the
/// manifest as an asset of the corpus folder the build already packs.
pub(crate) const OUTPUT_DIR: &str = "conformance/effects/generated";

/// The file's name, which is also the asset's path under `generated/`.
const FILE_NAME: &str = "boundary-surface.json";

/// One exported callable: what the smoke checks call, and what they record calling.
#[derive(PartialEq, Eq, Debug)]
struct Entry {
    /// The `-ffi` crate's namespace the callable belongs to, as UniFFI names it.
    namespace: String,
    /// Which kind of callable this is: `function`, `constructor`, `method`, `trait` or `callback`.
    kind: &'static str,
    /// The object, record, enum or callback interface that holds the callable, for the kinds that hang
    /// off one.
    owner: Option<String>,
    /// The callable's canonical name, the Rust name UniFFI read from the export, or for a UniFFI trait
    /// the trait's own name. The checks label a call with this, in every language, because the metadata
    /// names each callable by it alone.
    name: String,
}

impl Entry {
    /// The id a smoke check records for this callable: the owner's name before the callable's, for the
    /// kinds that have an owner. It is what an app's reader looks for in a failing run's message, so
    /// the name the bindings write for a missed callable is this id. Tested here; the device checks
    /// build the same id from the manifest's own fields.
    #[cfg(test)]
    pub(crate) fn id(&self) -> String {
        callable_id(self.owner.as_deref(), &self.name)
    }

    /// The same entry as the JSON the checks read. Built through serde's map rather than the `json!`
    /// macro, whose expansion carries an `unwrap` of its own that §3 refuses outside a test.
    fn to_json(&self) -> Value {
        let mut object = serde_json::Map::new();
        object.insert("namespace".to_string(), Value::String(self.namespace.clone()));
        object.insert("kind".to_string(), Value::String(self.kind.to_string()));
        object.insert("owner".to_string(), self.owner.clone().map_or(Value::Null, Value::String));
        object.insert("name".to_string(), Value::String(self.name.clone()));
        Value::Object(object)
    }
}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (&self.namespace, &self.owner, self.kind, &self.name).cmp(&(&other.namespace, &other.owner, other.kind, &other.name))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The surface's one read of the library: UniFFI's own loader over the metadata the library embeds.
/// The generator needs no crate configuration — the canonical names the metadata holds are what the
/// renames in `uniffi.toml` only translate into each language's spelling — so an empty supplier gives
/// the loader nothing to look up and it runs no cargo of its own.
struct NoCrateConfigs;

impl BindgenCrateConfigSupplier for NoCrateConfigs {}

/// One component's callables, read kind by kind, with the FFI function each is called through.
struct Reading<'a> {
    namespace: &'a str,
    entries: Vec<Entry>,
    /// The FFI functions the entries are called through, which [`unread`] holds UniFFI's own list to.
    through: BTreeSet<String>,
}

impl<'a> Reading<'a> {
    fn add<'f>(&mut self, kind: &'static str, owner: Option<&str>, name: &str, ffi: impl IntoIterator<Item = &'f FfiFunction>) {
        self.entries.push(Entry { namespace: self.namespace.to_string(), kind, owner: owner.map(str::to_string), name: name.to_string() });
        self.through.extend(ffi.into_iter().map(|function| function.name().to_string()));
    }

    /// What an object, a record or an enum holds: its constructors, its methods and the UniFFI traits
    /// it exports.
    fn members<'m>(
        &mut self,
        owner: &str,
        constructors: impl IntoIterator<Item = &'m Constructor>,
        methods: impl IntoIterator<Item = &'m Method>,
        traits: &UniffiTraitMethods,
    ) {
        for constructor in constructors {
            self.add("constructor", Some(owner), constructor.name(), [constructor.ffi_func()]);
        }
        for method in methods {
            self.add("method", Some(owner), method.name(), [method.ffi_func()]);
        }
        // A trait is one callable to the bindings, which make it the language's own conformance, however
        // many functions it crosses through: `Eq` answers both `==` and `!=`.
        let named = [
            ("Debug", vec![&traits.debug_fmt]),
            ("Display", vec![&traits.display_fmt]),
            ("Eq", vec![&traits.eq_eq, &traits.eq_ne]),
            ("Hash", vec![&traits.hash_hash]),
            ("Ord", vec![&traits.ord_cmp]),
        ];
        for (name, methods) in named {
            let methods: Vec<&Method> = methods.into_iter().flatten().collect();
            if !methods.is_empty() {
                self.add("trait", Some(owner), name, methods.into_iter().map(Method::ffi_func));
            }
        }
    }
}

/// The FFI functions UniFFI writes for `interface`'s bindings to call that no entry of `reading` is
/// called through: a kind of callable this reader does not know, which would otherwise reach an app's
/// bindings and no smoke check. The handle's own clone and free, and the function a callback's table
/// is handed over by, are the bindings' plumbing and no callable of an app's.
fn unread(interface: &ComponentInterface, reading: &Reading<'_>) -> Vec<String> {
    let mut plumbing = BTreeSet::new();
    for object in interface.object_definitions() {
        plumbing.insert(object.ffi_object_clone().name().to_string());
        plumbing.insert(object.ffi_object_free().name().to_string());
        if object.has_callback_interface() {
            plumbing.insert(object.ffi_init_callback().name().to_string());
        }
    }
    for callback in interface.callback_interface_definitions() {
        plumbing.insert(callback.ffi_init_callback().name().to_string());
    }
    interface
        .iter_user_ffi_function_definitions()
        .map(FfiFunction::name)
        .filter(|name| !reading.through.contains(*name) && !plumbing.contains(*name))
        .map(str::to_string)
        .collect()
}

/// Each component the distribution library's UniFFI metadata holds, with its FFI functions derived from its
/// callables, as each language's generator derives them before it writes: they are what a reading is held
/// to.
fn interfaces(library: &Path) -> Result<Vec<ComponentInterface>, String> {
    let path = Utf8Path::from_path(library).ok_or_else(|| format!("{} is not a path text can name", library.display()))?;
    let supplier = NoCrateConfigs;
    let components = uniffi_bindgen::find_components(path, &supplier)
        .map_err(|failure| format!("cannot read the UniFFI metadata out of {}: {failure:#}", library.display()))?;
    components
        .into_iter()
        .map(|component| {
            let mut interface = component.ci;
            interface.derive_ffi_funcs().map_err(|failure| format!("cannot derive the FFI functions of {}: {failure:#}", interface.namespace()))?;
            Ok(interface)
        })
        .collect()
}

/// Every callable of one component, kind by kind.
fn read(interface: &ComponentInterface) -> Reading<'_> {
    let mut reading = Reading { namespace: interface.namespace(), entries: Vec::new(), through: BTreeSet::new() };
    for function in interface.function_definitions() {
        reading.add("function", None, function.name(), [function.ffi_func()]);
    }
    for object in interface.object_definitions() {
        reading.members(object.name(), object.constructors(), object.methods(), &object.uniffi_trait_methods());
    }
    for record in interface.record_definitions() {
        reading.members(record.name(), record.constructors(), record.methods(), &record.uniffi_trait_methods());
    }
    for enumeration in interface.enum_definitions() {
        reading.members(enumeration.name(), enumeration.constructors(), enumeration.methods(), &enumeration.uniffi_trait_methods());
    }
    // A callback interface's methods are the app's own, which the library calls through the table the app
    // hands over, so they cross through no FFI function of their own.
    for callback in interface.callback_interface_definitions() {
        for method in callback.methods() {
            reading.add("callback", Some(callback.name()), method.name(), []);
        }
    }
    reading
}

/// Every callable in the distribution library's UniFFI metadata, sorted, so two runs of one library
/// write one file. A library that exports an FFI function no kind read here accounts for is refused,
/// naming it, so a kind UniFFI gains is read here before any smoke check can be told it is done.
fn surface_items(library: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for interface in interfaces(library)? {
        let reading = read(&interface);
        let missed = unread(&interface, &reading);
        if !missed.is_empty() {
            return Err(format!(
                "{} exports {} through UniFFI, which is no kind of callable the boundary's surface reads: read its kind in rust/xtask/src/boundary_surface.rs before a smoke check can call it",
                interface.namespace(),
                missed.join(", ")
            ));
        }
        entries.extend(reading.entries);
    }
    entries.sort();
    Ok(entries)
}

/// The manifest's whole text: every entry as JSON, ending in a newline. Keys come out in the order
/// serde writes them, and the entries are sorted, so the bytes say nothing about the order the
/// metadata happened to be read in.
fn render(entries: &[Entry]) -> Result<String, String> {
    let surface: Vec<Value> = entries.iter().map(Entry::to_json).collect();
    let mut root = serde_json::Map::new();
    root.insert("surface".to_string(), Value::Array(surface));
    let pretty = serde_json::to_string_pretty(&Value::Object(root)).map_err(|failure| format!("the surface list does not write as JSON: {failure}"))?;
    Ok(format!("{pretty}\n"))
}

/// Write the manifest beside its sibling fixtures, from the library the SDK ships, built for the host.
pub(crate) fn generate(out_dir: &Path) -> Result<(), String> {
    let library = crate::bindings::distribution_library()?;
    let text = render(&surface_items(&library)?)?;
    let file = out_dir.join(FILE_NAME);
    std::fs::write(&file, text).map_err(|failure| format!("{}: {failure}", file.display()))
}

/// The id one callable is known by: the owner's name before the callable's, for the kinds that have
/// an owner. The device smoke checks build the same id from the manifest's own fields, so a name
/// stated once here and once as the checks' rule is all there is of it.
#[cfg(test)]
fn callable_id(owner: Option<&str>, name: &str) -> String {
    match owner {
        Some(owner) => format!("{owner}.{name}"),
        None => name.to_string(),
    }
}

/// The manifest as its tests read it: every entry's namespace, kind and id, in the file's order —
/// the three together say the whole entry, so a test comparing two lists compares everything.
#[cfg(test)]
fn manifest_rows(manifest: &Path) -> Result<Vec<(String, String, String)>, String> {
    let text = std::fs::read_to_string(manifest).map_err(|failure| format!("{}: {failure}", manifest.display()))?;
    let value: Value = serde_json::from_str(&text).map_err(|failure| format!("{}: {failure}", manifest.display()))?;
    let items = value.get("surface").and_then(Value::as_array).ok_or_else(|| format!("{} holds no `surface` list", manifest.display()))?;
    items
        .iter()
        .map(|item| {
            let field = |key: &str| {
                item.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("{} holds an entry with no {key}", manifest.display()))
            };
            let namespace = field("namespace")?;
            let kind = field("kind")?;
            let name = field("name")?;
            let owner = match item.get("owner") {
                Some(Value::Null) | None => None,
                Some(owner) => {
                    Some(owner.as_str().ok_or_else(|| format!("{} holds an entry whose owner is neither a name nor nothing", manifest.display()))?.to_string())
                }
            };
            Ok((namespace, kind, callable_id(owner.as_deref(), &name)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(namespace: &str, kind: &'static str, owner: Option<&str>, name: &str) -> Entry {
        Entry { namespace: namespace.into(), kind, owner: owner.map(str::to_string), name: name.into() }
    }

    #[test]
    fn entries_sort_by_namespace_owner_kind_and_name() {
        let mut entries = [
            entry("b", "function", None, "one"),
            entry("a", "constructor", Some("Obj"), "new"),
            entry("a", "function", None, "z"),
            entry("a", "method", Some("Obj"), "go"),
        ];
        entries.sort();
        let ids: Vec<String> = entries.iter().map(Entry::id).collect();
        assert_eq!(
            ids,
            ["z", "Obj.new", "Obj.go", "one"],
            "top-level callables sort before the objects of their namespace, and each object's constructors before its methods"
        );
    }

    #[test]
    fn the_manifest_is_json_an_app_can_read_with_its_names_whole() {
        let entries =
            vec![entry("craftbridge_core_ffi", "function", None, "craft_error_message"), entry("craftbridge_core_ffi", "method", Some("Engine"), "recovery")];
        let text = render(&entries).expect("render");
        assert!(text.ends_with("}\n"), "the file ends in a newline, like every text file");
        let read: Value = serde_json::from_str(&text).expect("what the generator writes reads back as JSON");
        let surface = read.get("surface").and_then(Value::as_array).expect("the list the checks read");
        assert_eq!(surface.len(), 2);
        let method = surface.iter().find(|item| item.get("kind").and_then(Value::as_str) == Some("method")).expect("the method entry");
        assert_eq!(method.get("owner").and_then(Value::as_str), Some("Engine"));
        let function = surface.iter().find(|item| item.get("kind").and_then(Value::as_str) == Some("function")).expect("the function entry");
        assert!(function.get("owner").map(Value::is_null).unwrap_or(false), "a top-level callable owns nothing");
    }

    #[test]
    fn the_committed_manifest_holds_the_librarys_surface_exactly_once_and_in_order() {
        // The committed file and a fresh read of the library must name the same callables in the same
        // order. This is where a planted export fails before any device runs: the manifest no longer
        // matches the library, and the difference names it. The smoke checks lean on that manifest.
        let manifest = crate::repo_root().join(OUTPUT_DIR).join(FILE_NAME);
        let committed = manifest_rows(&manifest).expect("the committed manifest is readable");
        let library = crate::bindings::distribution_library().expect("the host build of the distribution library");
        let live: Vec<(String, String, String)> = surface_items(&library)
            .expect("the library's own metadata reads")
            .into_iter()
            .map(|entry| {
                let id = entry.id();
                (entry.namespace, entry.kind.to_string(), id)
            })
            .collect();
        assert_eq!(
            committed, live,
            "the committed manifest and a fresh read of the library name different callables, or name them in another order: run cargo xtask generate"
        );
        for pair in live.windows(2) {
            assert_ne!(pair[0].2, pair[1].2, "two callables share the id {}, and a check could call one and count the other", pair[0].2);
        }
    }

    #[test]
    fn the_guard_names_each_ffi_function_a_reading_leaves_out_and_none_of_the_handles_plumbing() {
        // The library's surface reads whole today, so nothing but this test shows the guard refuses
        // anything: a reading of the core's functions alone leaves its objects' constructors and methods
        // unread, and the guard must name each of them, and no handle's clone, free or callback table.
        let library = crate::bindings::distribution_library().expect("the host build of the distribution library");
        let interfaces = interfaces(&library).expect("the library's own metadata reads");
        let core = interfaces.iter().find(|interface| interface.namespace() == "craftbridge_core_ffi").expect("the core's component");
        let mut functions_only = Reading { namespace: core.namespace(), entries: Vec::new(), through: BTreeSet::new() };
        for function in core.function_definitions() {
            functions_only.add("function", None, function.name(), [function.ffi_func()]);
        }
        let missed = unread(core, &functions_only);
        for left_out in ["uniffi_craftbridge_core_ffi_fn_constructor_canceltoken_new", "uniffi_craftbridge_core_ffi_fn_method_engine_recovery"] {
            assert!(missed.iter().any(|name| name == left_out), "the guard did not name {left_out}: {missed:?}");
        }
        assert!(
            missed.iter().all(|name| !name.contains("_fn_clone_") && !name.contains("_fn_free_") && !name.contains("init_callback")),
            "the guard named a handle's plumbing, which is no callable of an app's: {missed:?}"
        );
        assert_eq!(unread(core, &read(core)), Vec::<String>::new(), "a whole reading leaves nothing unread");
    }

    #[test]
    fn sorting_holds_whatever_order_the_metadata_arrives_in() {
        // The same entries, built in two orders, render byte for byte the same: the device checks read
        // the file whole, and a regenerated one must not differ for the order UniFFI happened to give.
        let mut first = vec![entry("n", "method", Some("B"), "m"), entry("n", "function", None, "a")];
        let mut second = vec![entry("n", "function", None, "a"), entry("n", "method", Some("B"), "m")];
        first.sort();
        second.sort();
        assert_eq!(render(&first).expect("render"), render(&second).expect("render"));
    }
}
