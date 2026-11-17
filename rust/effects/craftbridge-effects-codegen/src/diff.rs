//! The diff of two API surfaces: which commands, keys and tools were added, removed or changed, and
//! whether any change breaks an app written against the old facade.
//!
//! # What breaks an app
//!
//! A change is breaking when an app written against the old surface's facade can fail to compile, or to
//! run, once built against the new one: an app builds the facade it ships with, so what is judged is its
//! source. Where the rule leaves a case open it is decided the way that protects apps: a doubtful change
//! breaks. Every rule is a function below, named for the change it judges.
//!
//! - A removed command or key, or a removed tool an app can call (below): the app calls what is gone.
//! - A key that is new and required, or that became required: the app's call omits it.
//! - A key whose type changed, in either direction: the facade's parameter has another type, so a call
//!   written for the old one no longer compiles (`json` to `bool` narrows what is accepted; `bool` to
//!   `json` changes the parameter's type).
//! - An enum key that lost a word: the app passes it. So too when a key's whole word list appears or
//!   goes while its type stays: that is a shape the rules do not read, and doubtful changes break.
//! - Keys of one command in another order: the facade keeps the `params` text's order for its
//!   parameters, and Swift takes arguments only in their declared order. A new key, wherever it stands,
//!   is judged on its own (above): a Swift call leaves out a defaulted parameter wherever it stands, and
//!   the Kotlin facade takes every argument by name, so no call hands a value to a key that took another's
//!   place.
//! - A tool an app can call that became `bridgeOnly`, or whose input schema changed other than by a new
//!   optional property, an optional property that was required, or a description: the app's input is no
//!   longer one the tool takes. A tool that was `bridgeOnly` is none an app can call: CraftBridge runs the
//!   engine headless, which hides and refuses every such tool, so nothing that becomes of it breaks an app.
//!
//! Everything else is compatible and still listed: a new command, a new optional key wherever it stands, a
//! new enum word, a key that became optional, a new tool, a changed label, `journal` flag, `params` text or
//! description — a changed `params` text is not listed beside a command whose keys changed, since it is the
//! very text those changes came from. A new enum word stays compatible only while the facades give an enum
//! key a type no Swift `switch` or Kotlin `when` covers exhaustively, and a new optional key before another
//! only while every Kotlin call names its arguments.

use crate::model::{Command, Key, Surface, Tool};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// What happened to the thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Verb {
    Added,
    Removed,
    Changed,
}

/// What kind of thing it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Command,
    Key,
    Tool,
}

/// One difference between two surfaces. The field order is the report's sort order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Change {
    pub(crate) kind: Kind,
    /// A command's id, a key as `<command id>:<key name>`, a tool's name.
    pub(crate) subject: String,
    pub(crate) verb: Verb,
    /// What changed, for a changed thing, with the surface's text in it escaped; empty otherwise.
    pub(crate) detail: String,
    pub(crate) breaking: bool,
}

impl Change {
    fn new(kind: Kind, subject: &str, verb: Verb, detail: String, breaking: bool) -> Self {
        Self { kind, subject: subject.to_string(), verb, detail, breaking }
    }

    /// A thing that is there in one surface only; it breaks when the app loses it, or must now supply it.
    fn presence(kind: Kind, subject: &str, verb: Verb, breaking: bool) -> Self {
        Self::new(kind, subject, verb, String::new(), breaking)
    }

    fn changed(kind: Kind, subject: &str, detail: String, breaking: bool) -> Self {
        Self::new(kind, subject, Verb::Changed, detail, breaking)
    }
}

/// Every difference between `old` and `new`, sorted, so the same two surfaces always give the same list.
pub(crate) fn compare(old: &Surface, new: &Surface) -> Vec<Change> {
    // Each struct of the surface is taken apart whole here and in the functions below, never with `..`:
    // a field the surface gains then stops the build until the diff judges it. The header (the app, the
    // engine's version, the pin) names the two surfaces, not their API, so it is no change an app sees.
    let Surface { app: _, engine_version: _, pin: _, commands: _, tools: _ } = old;
    let mut changes = Vec::new();
    let (old_commands, new_commands) = (by(&old.commands, |command| &command.id), by(&new.commands, |command| &command.id));
    for (id, pair) in pairs(&old_commands, &new_commands) {
        match pair {
            (Some(_), None) => changes.push(Change::presence(Kind::Command, id, Verb::Removed, true)),
            (None, Some(_)) => changes.push(Change::presence(Kind::Command, id, Verb::Added, false)),
            (Some(before), Some(after)) => compare_command(before, after, &mut changes),
            (None, None) => {}
        }
    }
    let (old_tools, new_tools) = (by(&old.tools, |tool| &tool.name), by(&new.tools, |tool| &tool.name));
    for (name, pair) in pairs(&old_tools, &new_tools) {
        match pair {
            (Some(before), None) => changes.push(Change::presence(Kind::Tool, name, Verb::Removed, callable(before))),
            (None, Some(_)) => changes.push(Change::presence(Kind::Tool, name, Verb::Added, false)),
            (Some(before), Some(after)) => compare_tool(before, after, &mut changes),
            (None, None) => {}
        }
    }
    changes.sort();
    changes
}

/// Does any change break an app?
pub(crate) fn is_breaking(changes: &[Change]) -> bool {
    changes.iter().any(|change| change.breaking)
}

/// The things of one list by name.
fn by<'a, T>(items: &'a [T], name: impl Fn(&'a T) -> &'a String) -> BTreeMap<&'a str, &'a T> {
    items.iter().map(|item| (name(item).as_str(), item)).collect()
}

/// Every name either side has, with what each side has under it.
fn pairs<'a, T>(old: &BTreeMap<&'a str, &'a T>, new: &BTreeMap<&'a str, &'a T>) -> Vec<(&'a str, (Option<&'a T>, Option<&'a T>))> {
    let names: BTreeSet<&str> = old.keys().chain(new.keys()).copied().collect();
    names.into_iter().map(|name| (name, (old.get(name).copied(), new.get(name).copied()))).collect()
}

fn compare_command(old: &Command, new: &Command, changes: &mut Vec<Change>) {
    let Command { id: _, label: _, params: _, journal: _, keys: _ } = old;
    let before = changes.len();
    let (old_keys, new_keys) = (by(&old.keys, |key| &key.name), by(&new.keys, |key| &key.name));
    for (name, pair) in pairs(&old_keys, &new_keys) {
        let subject = format!("{}:{name}", old.id);
        match pair {
            (Some(_), None) => changes.push(Change::presence(Kind::Key, &subject, Verb::Removed, true)),
            (None, Some(key)) => changes.push(Change::presence(Kind::Key, &subject, Verb::Added, key.required)),
            (Some(before), Some(after)) => compare_key(&subject, before, after, changes),
            (None, None) => {}
        }
    }
    if let Some(detail) = reordered(&old.keys, &new.keys) {
        changes.push(Change::changed(Kind::Command, &old.id, detail, true));
    }
    let keys_changed = changes.len() > before;
    for (what, was, is) in [("label", &old.label, &new.label), ("params", &old.params, &new.params)] {
        // The `params` text is where the keys come from, so it changes whenever they do.
        if was != is && !(what == "params" && keys_changed) {
            changes.push(Change::changed(Kind::Command, &old.id, format!("{what} {} -> {}", quoted(was), quoted(is)), false));
        }
    }
    if old.journal != new.journal {
        changes.push(Change::changed(Kind::Command, &old.id, format!("journal {} -> {}", old.journal, new.journal), false));
    }
}

fn compare_key(subject: &str, old: &Key, new: &Key, changes: &mut Vec<Change>) {
    // The name is the key's identity, and `doc` is its piece of the `params` text, judged with the command.
    let Key { name: _, required: _, key_type: _, values: _, doc: _ } = old;
    if !old.required && new.required {
        changes.push(Change::changed(Kind::Key, subject, "required false -> true".to_string(), true));
    }
    if old.required && !new.required {
        changes.push(Change::changed(Kind::Key, subject, "required true -> false".to_string(), false));
    }
    if old.key_type != new.key_type {
        changes.push(Change::changed(Kind::Key, subject, format!("type {} -> {}", escaped(&old.key_type), escaped(&new.key_type)), true));
    } else {
        // A word list can come or go on its own, without the type moving: the schema leaves `values`
        // optional, so the rules must judge its whole shape, not only one list against another.
        match (old.values.as_ref(), new.values.as_ref()) {
            (Some(was), Some(is)) => {
                let (was, is): (BTreeSet<&String>, BTreeSet<&String>) = (was.iter().collect(), is.iter().collect());
                let lost: Vec<&&String> = was.difference(&is).collect();
                let gained: Vec<&&String> = is.difference(&was).collect();
                if !lost.is_empty() || !gained.is_empty() {
                    let words = |sign: char, list: &[&&String]| list.iter().map(|word| format!(" {sign}{}", escaped(word))).collect::<String>();
                    changes.push(Change::changed(Kind::Key, subject, format!("values{}{}", words('-', &lost), words('+', &gained)), !lost.is_empty()));
                }
            }
            (None, None) => {}
            (was, is) => {
                let words = |list: Option<&Vec<String>>| match list {
                    None => "none".to_string(),
                    Some(list) => format!("[{}]", list.iter().map(|word| escaped(word)).collect::<Vec<_>>().join(",")),
                };
                changes.push(Change::changed(Kind::Key, subject, format!("values {} -> {}", words(was), words(is)), true));
            }
        }
    }
}

/// The old and the new order of a command's keys, when the keys the command keeps are in another order
/// among themselves: a Swift call passes its arguments in the order the parameters are declared. A key
/// that is gone or new is judged on its own, wherever it stood or stands.
fn reordered(old: &[Key], new: &[Key]) -> Option<String> {
    let (was, is) = (names_of(old), names_of(new));
    let kept_then: Vec<&str> = was.iter().copied().filter(|name| is.contains(name)).collect();
    let kept_now: Vec<&str> = is.iter().copied().filter(|name| was.contains(name)).collect();
    let list = |names: &[&str]| names.iter().map(|name| escaped(name)).collect::<Vec<_>>().join(",");
    (kept_then != kept_now).then(|| format!("key order {} -> {}", list(&was), list(&is)))
}

fn names_of(keys: &[Key]) -> Vec<&str> {
    keys.iter().map(|key| key.name.as_str()).collect()
}

/// Can an app call the tool? CraftBridge runs the engine headless, and a headless engine hides and refuses
/// every `bridgeOnly` tool, which needs the desktop app's control channel.
fn callable(tool: &Tool) -> bool {
    !tool.bridge_only
}

/// A tool no app can call breaks no app, whatever becomes of it; its changes are still listed.
fn compare_tool(old: &Tool, new: &Tool, changes: &mut Vec<Change>) {
    let Tool { name: _, description: _, bridge_only: _, input_schema: _ } = old;
    let subject = old.name.as_str();
    let called = callable(old);
    if old.description != new.description {
        changes.push(Change::changed(Kind::Tool, subject, "description".to_string(), false));
    }
    if old.bridge_only != new.bridge_only {
        changes.push(Change::changed(Kind::Tool, subject, format!("bridgeOnly {} -> {}", old.bridge_only, new.bridge_only), called));
    }
    for (detail, breaking) in schema_changes(&old.input_schema, &new.input_schema) {
        changes.push(Change::changed(Kind::Tool, subject, format!("input {detail}"), called && breaking));
    }
}

/// How a tool's input schema changed, each change with whether it breaks a caller. Prose (`description`)
/// is no part of what a schema accepts and is dropped first. A property or the required list is judged
/// by its own rule; whatever else differs (`type`, `additionalProperties`, a bound) is a change the rule
/// cannot judge, so it breaks.
fn schema_changes(old: &Value, new: &Value) -> Vec<(String, bool)> {
    let (old, new) = (without_prose(old), without_prose(new));
    let mut changes = Vec::new();
    let none = Map::new();
    let (old_properties, new_properties) = (properties_of(&old).unwrap_or(&none), properties_of(&new).unwrap_or(&none));
    let (old_required, new_required) = (required_of(&old).unwrap_or_default(), required_of(&new).unwrap_or_default());
    let names: BTreeSet<&str> = old_properties
        .keys()
        .chain(new_properties.keys())
        .map(String::as_str)
        .chain(old_required.iter().copied())
        .chain(new_required.iter().copied())
        .collect();
    for name in names {
        let (was, is) = (old_properties.get(name), new_properties.get(name));
        let shown = escaped(name);
        match (was, is) {
            (Some(_), None) => changes.push((format!("`{shown}` removed"), true)),
            (None, Some(_)) => changes.push((format!("`{shown}` added"), new_required.contains(name))),
            (Some(before), Some(after)) if before != after => changes.push((format!("`{shown}` changed"), true)),
            _ => {}
        }
        // A property that came or went is judged above; a name the list requires with no property behind it
        // still asks the caller for it.
        if was.is_some() == is.is_some() {
            match (old_required.contains(name), new_required.contains(name)) {
                (false, true) => changes.push((format!("`{shown}` required"), true)),
                (true, false) => changes.push((format!("`{shown}` optional"), false)),
                _ => {}
            }
        }
    }
    if rest_of(&old) != rest_of(&new) {
        changes.push(("schema changed".to_string(), true));
    }
    changes
}

/// A schema without its `description`s, anywhere in it. A property named `description` stays: only a
/// string under that key is prose, and a property's own schema is an object.
fn without_prose(schema: &Value) -> Value {
    match schema {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(name, value)| !(*name == "description" && value.is_string()))
                .map(|(name, value)| (name.clone(), without_prose(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(without_prose).collect()),
        other => other.clone(),
    }
}

/// The schema's properties, when it gives them in the shape their rule reads: an object.
fn properties_of(schema: &Value) -> Option<&Map<String, Value>> {
    schema.get("properties")?.as_object()
}

/// The names the schema requires, when it gives them in the shape their rule reads: an array of strings.
fn required_of(schema: &Value) -> Option<BTreeSet<&str>> {
    schema.get("required")?.as_array()?.iter().map(Value::as_str).collect()
}

/// The schema without the two fields judged by their own rules, each left out only where it has the shape
/// its rule reads, so that every other difference, one in a schema that is no object too, is seen here.
fn rest_of(schema: &Value) -> Value {
    let judged = |name: &str| match name {
        "properties" => properties_of(schema).is_some(),
        "required" => required_of(schema).is_some(),
        _ => false,
    };
    match schema {
        Value::Object(fields) => Value::Object(fields.iter().filter(|(name, _)| !judged(name)).map(|(name, value)| (name.clone(), value.clone())).collect()),
        other => other.clone(),
    }
}

/// Text from a surface for a report line: escaped, so that no character of it can end the line or the
/// field.
fn escaped(text: &str) -> String {
    text.escape_debug().to_string()
}

fn quoted(text: &str) -> String {
    format!("\"{}\"", escaped(text))
}

/// The report: one line per change, then one line that counts them. Fields are separated by a tab, in the
/// order `breaking` or `compatible`, `added`, `removed` or `changed`, `command`, `key` or `tool`, the
/// subject (a key as `<command id>:<key name>`) and the detail, which is empty unless the thing changed.
/// A line that starts with `#` is a comment, the count; every other line is a change.
pub(crate) fn report(changes: &[Change]) -> String {
    let mut text = String::new();
    for change in changes {
        let severity = if change.breaking { "breaking" } else { "compatible" };
        let verb = match change.verb {
            Verb::Added => "added",
            Verb::Removed => "removed",
            Verb::Changed => "changed",
        };
        let kind = match change.kind {
            Kind::Command => "command",
            Kind::Key => "key",
            Kind::Tool => "tool",
        };
        // The detail is escaped where it is made: it quotes text of its own (`label "…"`).
        text.push_str(&format!("{severity}\t{verb}\t{kind}\t{}\t{}\n", escaped(&change.subject), change.detail));
    }
    let count = |verb: Verb| changes.iter().filter(|change| change.verb == verb).count();
    let breaking = changes.iter().filter(|change| change.breaking).count();
    text.push_str(&format!("# {} added, {} removed, {} changed, {breaking} breaking\n", count(Verb::Added), count(Verb::Removed), count(Verb::Changed)));
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The changes between a surface of one command and the same surface with `edit` applied to it.
    fn one_command(edit: impl FnOnce(&mut Command)) -> Vec<Change> {
        let old = Surface::of(
            vec![Command::of(
                "a.b",
                "A label",
                vec![Key::raw("one", false, "bool", None), Key::raw("two", true, "string", None), Key::raw("mode", false, "enum", Some(&["on", "off"]))],
            )],
            vec![],
        );
        let mut new = old.clone();
        edit(new.commands.first_mut().unwrap());
        compare(&old, &new)
    }

    /// The key at `at` of the command being edited.
    fn key_at(command: &mut Command, at: usize) -> &mut Key {
        command.keys.get_mut(at).unwrap()
    }

    fn one_tool(edit: impl FnOnce(&mut Tool)) -> Vec<Change> {
        let old = Surface::of(
            vec![],
            vec![Tool::of(
                "t",
                json!({"type": "object", "properties": {"x": {"type": "integer", "description": "d"}, "y": {"type": "string"}}, "required": ["x"]}),
            )],
        );
        let mut new = old.clone();
        edit(new.tools.first_mut().unwrap());
        compare(&old, &new)
    }

    /// The changes when the one tool's input schema has `value` at the JSON pointer `at`.
    fn input_set(at: &str, value: Value) -> Vec<Change> {
        one_tool(|t| {
            let (parent, last) = at.rsplit_once('/').unwrap();
            let slot = if parent.is_empty() { Some(&mut t.input_schema) } else { t.input_schema.pointer_mut(parent) };
            slot.unwrap().as_object_mut().unwrap().insert(last.to_string(), value);
        })
    }

    fn only(changes: &[Change]) -> (&str, Verb, &str, bool) {
        assert_eq!(changes.len(), 1, "{changes:?}");
        let change = changes.first().unwrap();
        (change.subject.as_str(), change.verb, change.detail.as_str(), change.breaking)
    }

    #[test]
    fn the_same_surface_has_no_change() {
        let same = Surface::of(vec![Command::of("a", "A label", vec![Key::raw("k", true, "bool", None)])], vec![Tool::of("t", json!({}))]);
        assert!(compare(&same, &same).is_empty());
        assert!(!is_breaking(&[]));
    }

    #[test]
    fn a_surface_whose_header_moved_has_no_change() {
        // `Surface::of` gives every test the same header, so this test moves it on purpose: the app, the
        // engine's version and the pin name the two surfaces, not the API an app calls.
        let old = Surface::of(vec![Command::of("a", "A label", vec![Key::raw("k", true, "bool", None)])], vec![Tool::of("t", json!({}))]);
        let mut moved = old.clone();
        moved.app = "othercraft".to_string();
        moved.engine_version = "9.9.9".to_string();
        moved.pin = "fedcba98".to_string();
        assert!(compare(&old, &moved).is_empty());
    }

    #[test]
    fn a_removed_command_or_tool_breaks_and_an_added_one_does_not() {
        let both = Surface::of(
            vec![Command::of("a", "A label", vec![]), Command::of("b", "A label", vec![])],
            vec![Tool::of("t", json!({})), Tool::of("u", json!({}))],
        );
        let fewer = Surface::of(vec![Command::of("a", "A label", vec![])], vec![Tool::of("t", json!({}))]);
        let removed = compare(&both, &fewer);
        assert_eq!(
            removed.iter().map(|c| (c.kind, c.subject.as_str(), c.verb, c.breaking)).collect::<Vec<_>>(),
            [(Kind::Command, "b", Verb::Removed, true), (Kind::Tool, "u", Verb::Removed, true)]
        );
        let added = compare(&fewer, &both);
        assert_eq!(
            added.iter().map(|c| (c.kind, c.subject.as_str(), c.verb, c.breaking)).collect::<Vec<_>>(),
            [(Kind::Command, "b", Verb::Added, false), (Kind::Tool, "u", Verb::Added, false)]
        );
        assert!(is_breaking(&removed) && !is_breaking(&added));
    }

    #[test]
    fn a_key_that_is_gone_breaks_and_a_new_one_breaks_only_when_required() {
        assert_eq!(only(&one_command(|c| drop(c.keys.remove(0)))), ("a.b:one", Verb::Removed, "", true));
        assert_eq!(only(&one_command(|c| c.keys.push(Key::raw("three", false, "bool", None)))), ("a.b:three", Verb::Added, "", false));
        assert_eq!(only(&one_command(|c| c.keys.push(Key::raw("three", true, "bool", None)))), ("a.b:three", Verb::Added, "", true));
    }

    #[test]
    fn a_key_that_became_required_breaks_and_one_that_became_optional_does_not() {
        assert_eq!(only(&one_command(|c| key_at(c, 0).required = true)), ("a.b:one", Verb::Changed, "required false -> true", true));
        assert_eq!(only(&one_command(|c| key_at(c, 1).required = false)), ("a.b:two", Verb::Changed, "required true -> false", false));
    }

    #[test]
    fn a_key_whose_type_changed_breaks_in_either_direction() {
        assert_eq!(only(&one_command(|c| key_at(c, 0).key_type = "json".to_string())), ("a.b:one", Verb::Changed, "type bool -> json", true));
        assert_eq!(only(&one_command(|c| key_at(c, 1).key_type = "number".to_string())), ("a.b:two", Verb::Changed, "type string -> number", true));
    }

    #[test]
    fn an_enum_word_that_is_gone_breaks_and_one_that_is_new_does_not() {
        assert_eq!(only(&one_command(|c| *key_at(c, 2) = Key::raw("mode", false, "enum", Some(&["on"])))), ("a.b:mode", Verb::Changed, "values -off", true));
        assert_eq!(
            only(&one_command(|c| *key_at(c, 2) = Key::raw("mode", false, "enum", Some(&["on", "off", "auto"])))),
            ("a.b:mode", Verb::Changed, "values +auto", false)
        );
        assert_eq!(
            only(&one_command(|c| *key_at(c, 2) = Key::raw("mode", false, "enum", Some(&["on", "auto"])))),
            ("a.b:mode", Verb::Changed, "values -off +auto", true)
        );
        assert!(one_command(|c| *key_at(c, 2) = Key::raw("mode", false, "enum", Some(&["off", "on"]))).is_empty(), "the order of the words is no change");
    }

    #[test]
    fn a_key_that_gains_or_loses_its_whole_word_list_breaks() {
        // The schema leaves `values` optional, so a file may list no words for a key at all:
        // then the rules read no shape there, and such a change breaks.
        assert_eq!(only(&one_command(|c| key_at(c, 2).values = None)), ("a.b:mode", Verb::Changed, "values [on,off] -> none", true));
        let bare = Surface::of(vec![Command::of("a.b", "A label", vec![Key::raw("mode", false, "enum", None)])], vec![]);
        let with_words = Surface::of(vec![Command::of("a.b", "A label", vec![Key::raw("mode", false, "enum", Some(&["on", "off"]))])], vec![]);
        assert_eq!(only(&compare(&bare, &with_words)), ("a.b:mode", Verb::Changed, "values none -> [on,off]", true));
        let plain = Surface::of(vec![Command::of("a.b", "A label", vec![Key::raw("mode", false, "json", None)])], vec![]);
        let mut on_json = plain.clone();
        on_json.commands.first_mut().unwrap().keys.first_mut().unwrap().values = Some(vec!["x".to_string()]);
        assert_eq!(only(&compare(&plain, &on_json)), ("a.b:mode", Verb::Changed, "values none -> [x]", true));
        assert!(compare(&plain, &plain).is_empty(), "no word list on either side is no change");
    }

    #[test]
    fn keys_in_another_order_break_and_a_new_optional_key_anywhere_does_not() {
        assert_eq!(only(&one_command(|c| c.keys.swap(0, 1))), ("a.b", Verb::Changed, "key order one,two,mode -> two,one,mode", true));
        // Every call names its arguments, so a new optional key takes no other's place wherever it goes:
        // first, between two, or after all of them.
        for at in [0, 2, 3] {
            let added = one_command(|c| c.keys.insert(at, Key::raw("new", false, "bool", None)));
            assert_eq!(only(&added), ("a.b:new", Verb::Added, "", false), "a new key at {at}");
        }
        // The keys the command keeps are weighed among themselves, a new key between them or not.
        let both = one_command(|c| {
            c.keys.swap(0, 1);
            c.keys.insert(1, Key::raw("new", false, "bool", None));
        });
        assert_eq!(
            both.iter().map(|change| (change.subject.as_str(), change.detail.as_str(), change.breaking)).collect::<Vec<_>>(),
            [("a.b", "key order one,two,mode -> two,new,one,mode", true), ("a.b:new", "", false)]
        );
        assert!(one_command(|c| drop(c.keys.remove(0))).iter().all(|change| change.kind == Kind::Key), "a key that is gone is judged on its own");
    }

    #[test]
    fn a_new_label_journal_flag_or_params_text_is_compatible() {
        assert_eq!(only(&one_command(|c| c.label = "Another".to_string())), ("a.b", Verb::Changed, "label \"A label\" -> \"Another\"", false));
        assert_eq!(only(&one_command(|c| c.journal = false)), ("a.b", Verb::Changed, "journal true -> false", false));
        assert_eq!(only(&one_command(|c| c.params = "{x}\ttext\n".to_string())), ("a.b", Verb::Changed, "params \"{}\" -> \"{x}\\ttext\\n\"", false));
    }

    #[test]
    fn the_params_text_is_not_listed_again_beside_the_key_change_it_carries() {
        let changes = one_command(|c| {
            c.keys.remove(0);
            c.params = "{two, mode}".to_string();
        });
        assert_eq!(only(&changes).0, "a.b:one");
    }

    #[test]
    fn a_key_whose_doc_moved_has_no_change_of_its_own() {
        // A key's `doc` is its piece of the command's `params` text, judged with the command and never as
        // a change of its own. Every test builder fixes the piece the same on both sides, so this is the
        // one test that moves it: a piece that moved while its text stands is no change an app sees.
        let changes = one_command(|c| key_at(c, 0).doc = "another piece".to_string());
        assert!(changes.is_empty(), "{changes:?}");
    }

    #[test]
    fn a_tool_that_became_bridge_only_breaks_and_one_that_stopped_does_not() {
        assert_eq!(only(&one_tool(|t| t.bridge_only = true)), ("t", Verb::Changed, "bridgeOnly false -> true", true));
        let mut only_bridge = Surface::of(vec![], vec![Tool::of("t", json!({}))]);
        only_bridge.tools.first_mut().unwrap().bridge_only = true;
        let open = Surface::of(vec![], vec![Tool::of("t", json!({}))]);
        assert_eq!(only(&compare(&only_bridge, &open)), ("t", Verb::Changed, "bridgeOnly true -> false", false));
    }

    #[test]
    fn nothing_that_becomes_of_a_tool_no_app_can_call_breaks() {
        let mut old = Surface::of(vec![], vec![Tool::of("ui", json!({"type": "object", "properties": {"x": {"type": "integer"}}, "required": ["x"]}))]);
        old.tools.first_mut().unwrap().bridge_only = true;
        assert_eq!(only(&compare(&old, &Surface::of(vec![], vec![]))), ("ui", Verb::Removed, "", false));
        let mut changed = old.clone();
        changed.tools.first_mut().unwrap().input_schema = json!({"type": "object", "properties": {}, "required": ["y"], "additionalProperties": false});
        let changes = compare(&old, &changed);
        assert_eq!(changes.len(), 3, "each change is still listed: {changes:?}");
        assert!(!is_breaking(&changes), "{changes:?}");
    }

    #[test]
    fn a_tool_input_that_it_no_longer_takes_breaks() {
        let removed = one_tool(|t| {
            t.input_schema.pointer_mut("/properties").unwrap().as_object_mut().unwrap().remove("y");
        });
        assert_eq!(only(&removed), ("t", Verb::Changed, "input `y` removed", true));
        assert_eq!(only(&input_set("/properties/x/type", json!("string"))), ("t", Verb::Changed, "input `x` changed", true));
        assert_eq!(only(&input_set("/required", json!(["x", "y"]))), ("t", Verb::Changed, "input `y` required", true));
        assert_eq!(
            only(&input_set("/required", json!(["x", "w"]))),
            ("t", Verb::Changed, "input `w` required", true),
            "a required name with no property still asks for it"
        );
        let required_new = one_tool(|t| {
            t.input_schema.pointer_mut("/properties").unwrap().as_object_mut().unwrap().insert("z".to_string(), json!({"type": "boolean"}));
            t.input_schema.as_object_mut().unwrap().insert("required".to_string(), json!(["x", "z"]));
        });
        assert_eq!(only(&required_new), ("t", Verb::Changed, "input `z` added", true));
        assert_eq!(only(&input_set("/additionalProperties", json!(false))), ("t", Verb::Changed, "input schema changed", true));
    }

    #[test]
    fn a_schema_in_a_shape_the_rules_do_not_read_is_compared_whole() {
        for (was, is) in [
            (json!({}), json!(false)),
            (json!({"required": ["x"]}), json!({"required": "x"})),
            (json!({"required": ["x"]}), json!({"required": ["x", 1]})),
            (json!({"properties": {}}), json!({"properties": true})),
        ] {
            let changes = compare(&Surface::of(vec![], vec![Tool::of("t", was.clone())]), &Surface::of(vec![], vec![Tool::of("t", is.clone())]));
            assert!(is_breaking(&changes), "{was} -> {is}: {changes:?}");
        }
    }

    #[test]
    fn a_tool_input_that_it_takes_more_freely_or_describes_differently_does_not_break() {
        assert_eq!(only(&input_set("/required", json!([]))), ("t", Verb::Changed, "input `x` optional", false));
        assert_eq!(only(&input_set("/properties/z", json!({"type": "boolean"}))), ("t", Verb::Changed, "input `z` added", false));
        assert!(input_set("/properties/x/description", json!("another")).is_empty(), "prose is no part of the schema");
        assert_eq!(only(&one_tool(|t| t.description = "Other.".to_string())), ("t", Verb::Changed, "description", false));
    }

    #[test]
    fn the_report_is_sorted_one_line_per_change_and_ends_in_a_count() {
        let old = Surface::of(
            vec![Command::of("b", "A label", vec![]), Command::of("a", "A label", vec![Key::raw("k", false, "bool", None)])],
            vec![Tool::of("t", json!({}))],
        );
        let new = Surface::of(vec![Command::of("c", "A label", vec![]), Command::of("a", "A label", vec![Key::raw("k", true, "bool", None)])], vec![]);
        let text = report(&compare(&old, &new));
        assert_eq!(
            text,
            "breaking\tremoved\tcommand\tb\t\ncompatible\tadded\tcommand\tc\t\nbreaking\tchanged\tkey\ta:k\trequired false -> true\nbreaking\tremoved\ttool\tt\t\n# 1 added, 2 removed, 1 changed, 3 breaking\n"
        );
        assert_eq!(report(&[]), "# 0 added, 0 removed, 0 changed, 0 breaking\n");
        assert_eq!(text, report(&compare(&old, &new)), "the same surfaces, the same report");
    }

    #[test]
    fn text_from_a_surface_is_escaped_so_each_change_is_one_line_of_five_fields() {
        let odd = "a\tb\nc\rd";
        let old = Surface::of(
            vec![
                Command::of(
                    odd,
                    "A label",
                    vec![Key::raw(odd, false, odd, None), Key::raw("w", false, "enum", Some(&[odd, "v"])), Key::raw("k", false, "bool", None)],
                ),
                Command::of("gone", "A label", vec![]),
            ],
            vec![Tool::of(odd, json!({"properties": {odd: {}}, "required": []}))],
        );
        let mut new = old.clone();
        let edited = new.commands.first_mut().unwrap();
        edited.label = odd.to_string();
        edited.keys = vec![Key::raw("k", false, "bool", None), Key::raw(odd, false, "json", None), Key::raw("w", false, "enum", Some(&["v"]))];
        new.commands.pop();
        new.tools = vec![Tool::of(odd, json!({"properties": {odd: {}}, "required": [odd]}))];
        let changes = compare(&old, &new);
        let text = report(&changes);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), changes.len() + 1, "{text}");
        assert!(lines.iter().filter(|line| !line.starts_with('#')).all(|line| line.split('\t').count() == 5), "{text}");
        for kind in ["key order", "type ", "values -", "label ", "required"] {
            assert!(changes.iter().any(|change| change.detail.contains(kind)), "{kind} is among them: {text}");
        }
    }
}
