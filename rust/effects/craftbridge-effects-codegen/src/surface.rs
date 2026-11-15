//! The API surface: every command and tool of the pinned engine, with each command's keys typed by
//! [`crate::types`], as the JSON `generated/api-surface.json` holds.
//!
//! Everything is read from EffectCraft's own registry through its public functions, never from its
//! sources: `command_specs` for the commands, `params_schema` for the keys a command takes and which of
//! them are required, and the automation crate's tool catalogue. The output is a pure function of the
//! pinned engine and the pin, so a regeneration is byte for byte the file that was committed.

use crate::model::{Command, Key, Surface, Tool};
use crate::types::{KeyType, type_key};
use effectcraft_automation::tools;
use effectcraft_engine::commands::{accepted_params, params_schema};
use effectcraft_engine::{CommandSpec, command_specs};
use serde_json::Value;
use std::collections::BTreeMap;

/// The file the surface is written to.
pub(crate) const FILE: &str = "api-surface.json";

/// A well-formed pin for the tests: the surface records the pin it is given, so they need not follow the
/// workspace's.
#[cfg(test)]
pub(crate) const TEST_PIN: &str = "0123abcd";

/// The surface of the pinned engine, `pin` being the commit the workspace's pin names.
pub(crate) fn build(pin: &str) -> Result<Surface, String> {
    check_pin(pin)?;
    surface_of(pin, effectcraft_engine::project::APP_VERSION, command_specs(), tools::catalogue())
}

/// The surface of the given registry: kept apart from [`build`] so a test can hand it a registry of its own.
fn surface_of(pin: &str, engine_version: &str, specs: &[CommandSpec], catalogue: &[tools::ToolDef]) -> Result<Surface, String> {
    let mut commands = specs.iter().map(command_of).collect::<Vec<_>>();
    commands.sort_by(|left, right| left.id.cmp(&right.id));
    let mut tools = catalogue.iter().map(tool_of).collect::<Vec<_>>();
    tools.sort_by(|left, right| left.name.cmp(&right.name));
    let surface = Surface { app: "effectcraft".to_string(), engine_version: engine_version.to_string(), pin: pin.to_string(), commands, tools };
    surface.ensure_unique()?;
    Ok(surface)
}

/// The pin as the surface's schema states it: eight to forty lowercase hexadecimal digits.
fn check_pin(pin: &str) -> Result<(), String> {
    if (8..=40).contains(&pin.len()) && pin.chars().all(|digit| matches!(digit, '0'..='9' | 'a'..='f')) {
        Ok(())
    } else {
        Err(format!("`{pin}` is no pin: a commit id is eight to forty lowercase hexadecimal digits"))
    }
}

fn command_of(spec: &CommandSpec) -> Command {
    Command { id: spec.id.to_string(), label: spec.label.to_string(), params: spec.params.to_string(), journal: spec.journal, keys: keys_of(spec) }
}

/// A command's keys in the order its `params` text lists them, which a typed method keeps for its
/// parameters. A key is required when the registry's schema says so. Every key of one piece carries the
/// piece as its description, which is all the rules read.
fn keys_of(spec: &CommandSpec) -> Vec<Key> {
    let schema = params_schema(spec);
    let required: Vec<&str> = schema.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    let properties = schema.get("properties").and_then(Value::as_object);
    let pieces: Vec<(String, &str)> = accepted_params(spec.params)
        .unwrap_or_default()
        .into_iter()
        .map(|key| {
            let piece = properties.and_then(|properties| properties.get(&key)).and_then(|property| property.get("description")).and_then(Value::as_str);
            (key, piece.unwrap_or_default())
        })
        .collect();
    pieces.iter().map(|(key, piece)| key_of(key, piece, required.contains(&key.as_str()), &type_key(key, piece))).collect()
}

fn key_of(name: &str, piece: &str, required: bool, key_type: &KeyType) -> Key {
    let values = match key_type {
        KeyType::Enum(values) => Some(values.clone()),
        _ => None,
    };
    Key { name: name.to_string(), required, key_type: key_type.name().to_string(), values, doc: piece.to_string() }
}

fn tool_of(tool: &tools::ToolDef) -> Tool {
    Tool { name: tool.name.to_string(), description: tool.description.to_string(), bridge_only: tool.bridge_only, input_schema: tool.input_schema() }
}

/// How many keys the surface types each way, for the run's report.
pub(crate) fn type_counts(surface: &Surface) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for key in surface.commands.iter().flat_map(|command| &command.keys) {
        *counts.entry(key.key_type.clone()).or_default() += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn surface() -> Surface {
        build(TEST_PIN).unwrap()
    }

    fn command<'a>(surface: &'a Surface, id: &str) -> &'a Command {
        surface.commands.iter().find(|command| command.id == id).unwrap()
    }

    fn key<'a>(surface: &'a Surface, id: &str, name: &str) -> &'a Key {
        command(surface, id).keys.iter().find(|key| key.name == name).unwrap()
    }

    #[test]
    fn every_command_of_the_registry_is_in_the_surface_once_and_in_order() {
        let surface = surface();
        assert_eq!(surface.commands.len(), command_specs().len());
        assert!(surface.commands.len() > 700, "{} commands", surface.commands.len());
        assert!(surface.commands.windows(2).all(|pair| pair.first().map(|c| &c.id) < pair.get(1).map(|c| &c.id)), "sorted by id, no id twice");
    }

    #[test]
    fn every_tool_of_the_catalogue_is_in_the_surface() {
        let surface = surface();
        assert_eq!(surface.tools.len(), tools::catalogue().len());
        assert!(surface.tools.windows(2).all(|pair| pair.first().map(|t| &t.name) < pair.get(1).map(|t| &t.name)), "sorted by name");
        for tool in &surface.tools {
            assert!(tool.input_schema.is_object(), "{}", tool.name);
        }
    }

    #[test]
    fn the_header_names_the_app_the_engine_and_the_pin() {
        let surface = surface();
        assert_eq!((surface.app.as_str(), surface.pin.as_str()), ("effectcraft", TEST_PIN));
        assert_eq!(surface.engine_version, effectcraft_engine::project::APP_VERSION);
    }

    #[test]
    fn the_written_text_carries_every_field_the_schema_requires() {
        let text = surface().render().unwrap();
        let written: Value = serde_json::from_str(&text).unwrap();
        let command = written
            .get("commands")
            .and_then(Value::as_array)
            .and_then(|commands| commands.iter().find(|c| c.get("id") == Some(&json!("layer.newText"))))
            .unwrap();
        for field in ["id", "label", "params", "keys", "journal"] {
            assert!(command.get(field).is_some(), "layer.newText lacks {field}");
        }
        for key in command.get("keys").and_then(Value::as_array).unwrap() {
            for field in ["name", "required", "type", "doc"] {
                assert!(key.get(field).is_some(), "{key} lacks {field}");
            }
        }
        assert_eq!(Surface::parse(&text).unwrap(), surface());
    }

    #[test]
    fn keys_are_typed_by_the_rules_and_required_as_the_registry_says() {
        let surface = surface();
        let average = key(&surface, "effect.pickColor", "average");
        assert_eq!((average.key_type.as_str(), average.required), ("bool", false));
        let effect = key(&surface, "effect.remove", "effect");
        assert_eq!((effect.key_type.as_str(), effect.required), ("ref", true));
        let tool = key(&surface, "liquify.stroke", "tool");
        assert_eq!(tool.key_type, "enum");
        assert!(tool.values.as_ref().is_some_and(|values| values.iter().any(|value| value == "twirlClockwise")));
        let mode = key(&surface, "layer.mask.motionBlur", "mode");
        assert_eq!(mode.values, Some(vec!["sameAsLayer".to_string(), "on".to_string(), "off".to_string()]));
    }

    #[test]
    fn keys_keep_the_order_of_the_params_text() {
        let surface = surface();
        let names: Vec<&str> = command(&surface, "layer.mask.motionBlur").keys.iter().map(|key| key.name.as_str()).collect();
        assert_eq!(names, ["layer", "mask", "mode"]);
    }

    #[test]
    fn keys_that_share_a_piece_stay_json() {
        let surface = surface();
        for (command_id, name) in [("view.exposure", "stops"), ("view.exposure", "delta"), ("effect.copy", "effect"), ("effect.copy", "effects")] {
            assert_eq!(key(&surface, command_id, name).key_type, "json", "{command_id} {name}");
        }
    }

    #[test]
    fn prose_that_names_a_kind_of_value_or_offers_another_leaves_the_key_json() {
        let surface = surface();
        for (command_id, name) in [
            ("renderQueue.add", "output"),
            ("renderQueue.add", "resolution"),
            ("mask.interpolate", "keyframeRate"),
            ("file.interpretFootage", "startTimecode"),
            ("layer.style.add", "style"),
        ] {
            assert_eq!(key(&surface, command_id, name).key_type, "json", "{command_id} {name}");
        }
        assert_eq!(key(&surface, "layer.newSolid", "color").key_type, "color", "a colour written hex first");
        assert_eq!(key(&surface, "renderQueue.move", "to").key_type, "number", "`n` is a number");
    }

    #[test]
    fn a_command_with_no_key_list_has_no_keys() {
        let surface = surface();
        assert!(command(&surface, "time.forward10").keys.is_empty());
    }

    #[test]
    fn the_surface_is_the_same_every_time_it_is_built() {
        assert_eq!(surface().render().unwrap(), surface().render().unwrap());
    }

    #[test]
    fn a_pin_is_eight_to_forty_lowercase_hex_digits() {
        for good in ["10376058", "103760584a60074c9aaeef686d2d05d89f08155e"] {
            assert!(check_pin(good).is_ok(), "{good}");
        }
        for bad in ["", "1037605", "1037605G", "10376058A", "main", &"a".repeat(41)] {
            assert!(check_pin(bad).is_err(), "{bad:?}");
        }
        assert!(build("main").is_err());
    }

    fn query(id: &'static str, params: &'static str) -> CommandSpec {
        CommandSpec {
            id,
            label: "A label",
            menu: &[],
            shortcut: None,
            alt_shortcuts: &[],
            params,
            enabled: effectcraft_engine::commands::always,
            run: |_, _| Ok(json!(null)),
            journal: false,
        }
    }

    #[test]
    fn a_registry_of_its_own_gives_exactly_the_keys_its_params_texts_list() {
        let specs = [query("b.second", "{layer?: id|name, mode: on|off, time? (s) | frame?}"), query("a.first", "{}"), query("c.third", "free text")];
        let surface = surface_of(TEST_PIN, "1.2.3", &specs, &[]).unwrap();
        let ids: Vec<&str> = surface.commands.iter().map(|command| command.id.as_str()).collect();
        assert_eq!(ids, ["a.first", "b.second", "c.third"]);
        let keys: Vec<(&str, bool, &str)> =
            command(&surface, "b.second").keys.iter().map(|key| (key.name.as_str(), key.required, key.key_type.as_str())).collect();
        assert_eq!(keys, [("layer", false, "ref"), ("mode", true, "enum"), ("time", false, "json"), ("frame", false, "json")]);
        assert!(command(&surface, "c.third").keys.is_empty());
        assert!(!command(&surface, "a.first").journal);
        assert_eq!(surface.engine_version, "1.2.3");
        assert!(surface.tools.is_empty());
    }

    #[test]
    fn two_commands_of_one_id_are_refused() {
        let error = surface_of(TEST_PIN, "1.2.3", &[query("a.first", "{}"), query("a.first", "{x}")], &[]).unwrap_err();
        assert!(error.contains("`a.first`"), "{error}");
    }

    #[test]
    fn every_type_the_rules_give_is_one_the_schema_allows() {
        let allowed = ["bool", "number", "string", "enum", "vec2", "vec3", "color", "ref", "json"];
        let counts = type_counts(&surface());
        assert!(counts.keys().all(|name| allowed.contains(&name.as_str())), "{counts:?}");
        for used in ["bool", "number", "enum", "vec2", "color", "ref", "json"] {
            assert!(counts.get(used).is_some_and(|count| *count > 0), "no key is typed {used}: {counts:?}");
        }
    }
}
