//! The shape of the API surface file, defined once: the generator builds a [`Surface`] and writes it, the
//! diff reads two back into the same types, so the shape the schema
//! (`plan/contracts/api-surface.schema.json`) describes is written in code here and nowhere else.
//!
//! A field the schema marks optional defaults when absent, so an older file still reads; a field the
//! schema requires is required here too, and a file without it is refused with the field named.

use crate::types::KeyType;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// The whole surface of one pinned engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Surface {
    pub(crate) app: String,
    pub(crate) engine_version: String,
    pub(crate) pin: String,
    pub(crate) commands: Vec<Command>,
    pub(crate) tools: Vec<Tool>,
}

/// One command of the registry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Command {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) params: String,
    pub(crate) journal: bool,
    /// In the order the `params` text lists them, which a typed method keeps for its parameters.
    pub(crate) keys: Vec<Key>,
}

/// One key a command takes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Key {
    pub(crate) name: String,
    pub(crate) required: bool,
    /// The name of the key's [`crate::types::KeyType`].
    #[serde(rename = "type")]
    pub(crate) key_type: String,
    /// The words of an `enum`; no other type has any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) values: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) doc: String,
}

impl Key {
    /// The key that carries the typed `key_type`: the file names the type as [`KeyType::name`] spells it
    /// and holds an `enum`'s words in `values`, so every reader of a surface — the diff, a facade — finds
    /// the type and its words the same one way, and a typed rule and its file shape cannot drift apart.
    pub(crate) fn typed(name: &str, required: bool, key_type: &KeyType, doc: &str) -> Self {
        let values = match key_type {
            KeyType::Enum(words) => Some(words.clone()),
            _ => None,
        };
        Key { name: name.to_string(), required, key_type: key_type.name().to_string(), values, doc: doc.to_string() }
    }
}

/// One tool of the catalogue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Tool {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) bridge_only: bool,
    pub(crate) input_schema: Value,
}

impl Surface {
    /// Read the text of a surface file.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let surface: Self = serde_json::from_str(text).map_err(|error| format!("not an API surface: {error}"))?;
        surface.ensure_unique()?;
        Ok(surface)
    }

    /// The surface as the text of its file: pretty-printed, objects with their keys in sorted order (what
    /// `serde_json` writes unless a build turns on its `preserve_order` feature, which the test of this
    /// text would then fail on), and ending in one newline. A struct would write its fields in the order
    /// it declares them, so the text goes through a [`Value`] first.
    pub(crate) fn render(&self) -> Result<String, String> {
        let value = serde_json::to_value(self).map_err(|error| format!("cannot write the surface as JSON: {error}"))?;
        let mut text = serde_json::to_string_pretty(&value).map_err(|error| format!("cannot write the surface as JSON: {error}"))?;
        text.push('\n');
        Ok(text)
    }

    /// Every command id, every tool name and every key name of a command is there once: a surface is
    /// compared and typed by those names, and a name twice would leave one of its two entries unread.
    pub(crate) fn ensure_unique(&self) -> Result<(), String> {
        first_repeat(self.commands.iter().map(|command| command.id.as_str()))
            .map_or(Ok(()), |id| Err(format!("two commands are named `{id}`: the surface needs every id once")))?;
        first_repeat(self.tools.iter().map(|tool| tool.name.as_str()))
            .map_or(Ok(()), |name| Err(format!("two tools are named `{name}`: the surface needs every name once")))?;
        for command in &self.commands {
            if let Some(name) = first_repeat(command.keys.iter().map(|key| key.name.as_str())) {
                return Err(format!("`{}` has two keys named `{name}`: a command needs every key name once", command.id));
            }
        }
        Ok(())
    }
}

/// The first name that has already come by, if any does.
fn first_repeat<'a>(names: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut seen = BTreeSet::new();
    names.into_iter().find(|name| !seen.insert(*name))
}

#[cfg(test)]
impl Command {
    /// A command as a test writes one, journalled and with no `params` text of its own: every test module
    /// builds its commands here, so a field the surface gains is given its test value once.
    pub(crate) fn of(id: &str, label: &str, keys: Vec<Key>) -> Self {
        Command { id: id.to_string(), label: label.to_string(), params: "{}".to_string(), journal: true, keys }
    }
}

#[cfg(test)]
impl Key {
    /// A key as a test writes one: its piece of the `params` text is its name and whether it may be left
    /// out, which is all the type rules and the facade read from a piece.
    pub(crate) fn of(name: &str, required: bool, key_type: KeyType) -> Self {
        Key::typed(name, required, &key_type, &format!("{name}{}", if required { "" } else { "?" }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(name: &str) -> Key {
        Key { name: name.to_string(), required: false, key_type: "json".to_string(), values: None, doc: String::new() }
    }

    fn surface(commands: Vec<Command>, tools: Vec<Tool>) -> Surface {
        Surface { app: "effectcraft".to_string(), engine_version: "1.2.3".to_string(), pin: "0123abcd".to_string(), commands, tools }
    }

    fn tool(name: &str) -> Tool {
        Tool { name: name.to_string(), description: String::new(), bridge_only: false, input_schema: json!({"type": "object"}) }
    }

    #[test]
    fn a_surface_is_written_with_its_keys_sorted_and_read_back_whole() {
        let mut typed = key("mode");
        typed.key_type = "enum".to_string();
        typed.values = Some(vec!["on".to_string(), "off".to_string()]);
        let original = surface(vec![Command::of("a.first", "A label", vec![typed, key("plain")])], vec![tool("undo")]);
        let text = original.render().unwrap();
        assert!(text.starts_with("{\n  \"app\": \"effectcraft\",\n  \"commands\": ["), "{text}");
        assert!(text.ends_with("}\n") && !text.ends_with("\n\n"));
        assert!(text.find("\"doc\"") < text.find("\"name\"") && text.find("\"name\"") < text.find("\"required\""), "keys are sorted: {text}");
        assert!(!text.contains("\"values\": null"), "a key with no words writes none: {text}");
        assert_eq!(Surface::parse(&text).unwrap(), original);
    }

    #[test]
    fn what_the_schema_leaves_optional_defaults_and_what_it_requires_is_required() {
        let text = r#"{"app":"effectcraft","engineVersion":"1","pin":"0123abcd","commands":[{"id":"a","label":"l","params":"{}","journal":false,"keys":[{"name":"k","required":true,"type":"bool"}]}],"tools":[{"name":"t","inputSchema":{}}]}"#;
        let read = Surface::parse(text).unwrap();
        assert_eq!(read.commands.first().and_then(|command| command.keys.first()).map(|key| key.doc.as_str()), Some(""));
        assert_eq!(read.tools.first().map(|tool| (tool.description.as_str(), tool.bridge_only)), Some(("", false)));
        for missing in ["engineVersion", "pin", "commands", "tools", "app"] {
            let mut value: Value = serde_json::from_str(text).unwrap();
            value.as_object_mut().unwrap().remove(missing);
            let error = Surface::parse(&value.to_string()).unwrap_err();
            assert!(error.contains(missing), "{error}");
        }
    }

    #[test]
    fn text_that_is_no_surface_is_refused_with_a_reason() {
        for text in ["", "not json", "[]", "{}", "{\"app\": 1}"] {
            assert!(Surface::parse(text).unwrap_err().starts_with("not an API surface"), "{text:?}");
        }
    }

    #[test]
    fn a_name_twice_is_refused() {
        let twice = surface(vec![Command::of("a", "A label", vec![]), Command::of("a", "A label", vec![])], vec![]);
        assert!(twice.ensure_unique().unwrap_err().contains("`a`"));
        let tools = surface(vec![], vec![tool("t"), tool("t")]);
        assert!(tools.ensure_unique().unwrap_err().contains("two tools are named `t`"));
        let keys = surface(vec![Command::of("a", "A label", vec![key("k"), key("k")])], vec![]);
        assert!(keys.ensure_unique().unwrap_err().contains("two keys named `k`"));
        assert!(Surface::parse(&twice.render().unwrap()).is_err(), "a file is checked as it is read");
        assert!(
            surface(vec![Command::of("a", "A label", vec![key("k")]), Command::of("b", "A label", vec![key("k")])], vec![tool("t")]).ensure_unique().is_ok()
        );
    }
}
