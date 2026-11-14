//! The API surface: every command and tool of the pinned engine, with each command's keys typed by
//! [`crate::types`], as the JSON `generated/api-surface.json` holds.
//!
//! Everything is read from EffectCraft's own registry through its public functions, never from its
//! sources: `command_specs` for the commands, `params_schema` for the keys a command takes and which of
//! them are required, and the automation crate's tool catalogue. The output is a pure function of the
//! pinned engine and the pin, so a regeneration is byte for byte the file that was committed.

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
pub(crate) fn build(pin: &str) -> Result<Value, String> {
    check_pin(pin)?;
    surface_of(pin, effectcraft_engine::project::APP_VERSION, command_specs(), tools::catalogue())
}

/// The surface of the given registry: kept apart from [`build`] so a test can hand it a registry of its own.
fn surface_of(pin: &str, engine_version: &str, specs: &[CommandSpec], catalogue: &[tools::ToolDef]) -> Result<Value, String> {
    let mut commands = specs.iter().map(command_of).collect::<Vec<_>>();
    commands.sort_by(|left, right| id_of(left).cmp(id_of(right)));
    if let Some(pair) = commands.windows(2).find(|pair| pair.first().map(id_of) == pair.get(1).map(id_of)) {
        return Err(format!("two commands are named `{}`: the surface needs every id once", pair.first().map(id_of).unwrap_or_default()));
    }
    let mut tool_list = catalogue.iter().map(tool_of).collect::<Vec<_>>();
    tool_list.sort_by(|left, right| name_of(left).cmp(name_of(right)));
    Ok(object([
        ("app", "effectcraft".into()),
        ("engineVersion", engine_version.into()),
        ("pin", pin.into()),
        ("commands", Value::Array(commands)),
        ("tools", Value::Array(tool_list)),
    ]))
}

/// The pin as the surface's schema states it: eight to forty lowercase hexadecimal digits.
fn check_pin(pin: &str) -> Result<(), String> {
    if (8..=40).contains(&pin.len()) && pin.chars().all(|digit| matches!(digit, '0'..='9' | 'a'..='f')) {
        Ok(())
    } else {
        Err(format!("`{pin}` is no pin: a commit id is eight to forty lowercase hexadecimal digits"))
    }
}

fn id_of(command: &Value) -> &str {
    command.get("id").and_then(Value::as_str).unwrap_or_default()
}

fn name_of(tool: &Value) -> &str {
    tool.get("name").and_then(Value::as_str).unwrap_or_default()
}

fn command_of(spec: &CommandSpec) -> Value {
    object([
        ("id", spec.id.into()),
        ("label", spec.label.into()),
        ("params", spec.params.into()),
        ("journal", spec.journal.into()),
        ("keys", Value::Array(keys_of(spec))),
    ])
}

/// A command's keys in the order its `params` text lists them, which a typed method keeps for its
/// parameters. A key is required when the registry's schema says so. Every key of one piece carries the
/// piece as its description, which is all the rules read.
fn keys_of(spec: &CommandSpec) -> Vec<Value> {
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

fn key_of(name: &str, piece: &str, required: bool, key_type: &KeyType) -> Value {
    let mut fields = vec![("name", name.into()), ("required", required.into()), ("type", key_type.name().into())];
    if let KeyType::Enum(values) = key_type {
        fields.push(("values", Value::Array(values.iter().map(|value| value.as_str().into()).collect())));
    }
    fields.push(("doc", piece.into()));
    object(fields)
}

fn tool_of(tool: &tools::ToolDef) -> Value {
    object([
        ("name", tool.name.into()),
        ("description", tool.description.into()),
        ("bridgeOnly", tool.bridge_only.into()),
        ("inputSchema", tool.input_schema()),
    ])
}

/// A JSON object of the given fields. Built by hand: `json!` expands to an `unwrap`, which the never-crash
/// lints refuse in shipped code.
fn object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(name, value)| (name.to_string(), value)).collect())
}

/// The surface as the text of its file: pretty-printed, objects with their keys in sorted order (what
/// `serde_json` writes unless a build turns on its `preserve_order` feature, which the test of this text
/// would then fail on), and ending in one newline.
pub(crate) fn render(surface: &Value) -> Result<String, String> {
    let mut text = serde_json::to_string_pretty(surface).map_err(|error| format!("cannot write the surface as JSON: {error}"))?;
    text.push('\n');
    Ok(text)
}

/// How many keys the surface types each way, for the run's report.
pub(crate) fn type_counts(surface: &Value) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for command in surface.get("commands").and_then(Value::as_array).into_iter().flatten() {
        for key in command.get("keys").and_then(Value::as_array).into_iter().flatten() {
            if let Some(name) = key.get("type").and_then(Value::as_str) {
                *counts.entry(name.to_string()).or_default() += 1;
            }
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn surface() -> Value {
        build(TEST_PIN).unwrap()
    }

    fn command<'a>(surface: &'a Value, id: &str) -> &'a Value {
        surface.get("commands").and_then(Value::as_array).unwrap().iter().find(|command| id_of(command) == id).unwrap()
    }

    fn key<'a>(surface: &'a Value, id: &str, name: &str) -> &'a Value {
        command(surface, id).get("keys").and_then(Value::as_array).unwrap().iter().find(|key| key.get("name").and_then(Value::as_str) == Some(name)).unwrap()
    }

    #[test]
    fn every_command_of_the_registry_is_in_the_surface_once_and_in_order() {
        let surface = surface();
        let commands = surface.get("commands").and_then(Value::as_array).unwrap();
        assert_eq!(commands.len(), command_specs().len());
        assert!(commands.len() > 700, "{} commands", commands.len());
        assert!(commands.windows(2).all(|pair| id_of(pair.first().unwrap()) < id_of(pair.get(1).unwrap())), "sorted by id, no id twice");
    }

    #[test]
    fn every_tool_of_the_catalogue_is_in_the_surface() {
        let surface = surface();
        let listed = surface.get("tools").and_then(Value::as_array).unwrap();
        assert_eq!(listed.len(), tools::catalogue().len());
        assert!(listed.windows(2).all(|pair| name_of(pair.first().unwrap()) < name_of(pair.get(1).unwrap())), "sorted by name");
        for tool in listed {
            assert!(tool.get("inputSchema").is_some_and(Value::is_object), "{tool}");
        }
    }

    #[test]
    fn the_header_names_the_app_the_engine_and_the_pin() {
        let surface = surface();
        assert_eq!(surface.get("app"), Some(&json!("effectcraft")));
        assert_eq!(surface.get("pin"), Some(&json!(TEST_PIN)));
        assert_eq!(surface.get("engineVersion"), Some(&json!(effectcraft_engine::project::APP_VERSION)));
    }

    #[test]
    fn a_command_carries_what_the_schema_requires() {
        let surface = surface();
        let command = command(&surface, "layer.newText");
        for field in ["id", "label", "params", "keys", "journal"] {
            assert!(command.get(field).is_some(), "layer.newText lacks {field}");
        }
        for key in command.get("keys").and_then(Value::as_array).unwrap() {
            for field in ["name", "required", "type", "doc"] {
                assert!(key.get(field).is_some(), "{key} lacks {field}");
            }
        }
    }

    #[test]
    fn keys_are_typed_by_the_rules_and_required_as_the_registry_says() {
        let surface = surface();
        let average = key(&surface, "effect.pickColor", "average");
        assert_eq!((average.get("type"), average.get("required")), (Some(&json!("bool")), Some(&json!(false))));
        let effect = key(&surface, "effect.remove", "effect");
        assert_eq!((effect.get("type"), effect.get("required")), (Some(&json!("ref")), Some(&json!(true))));
        let tool = key(&surface, "liquify.stroke", "tool");
        assert_eq!(tool.get("type"), Some(&json!("enum")));
        assert!(tool.get("values").and_then(Value::as_array).is_some_and(|values| values.contains(&json!("twirlClockwise"))));
        let mode = key(&surface, "layer.mask.motionBlur", "mode");
        assert_eq!(mode.get("values"), Some(&json!(["sameAsLayer", "on", "off"])));
    }

    #[test]
    fn keys_keep_the_order_of_the_params_text() {
        let surface = surface();
        let names: Vec<&str> = command(&surface, "layer.mask.motionBlur")
            .get("keys")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(|key| key.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(names, ["layer", "mask", "mode"]);
    }

    #[test]
    fn keys_that_share_a_piece_stay_json() {
        let surface = surface();
        for (command_id, name) in [("view.exposure", "stops"), ("view.exposure", "delta"), ("effect.copy", "effect"), ("effect.copy", "effects")] {
            assert_eq!(key(&surface, command_id, name).get("type"), Some(&json!("json")), "{command_id} {name}");
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
            assert_eq!(key(&surface, command_id, name).get("type"), Some(&json!("json")), "{command_id} {name}");
        }
        assert_eq!(key(&surface, "layer.newSolid", "color").get("type"), Some(&json!("color")), "a colour written hex first");
        assert_eq!(key(&surface, "renderQueue.move", "to").get("type"), Some(&json!("number")), "`n` is a number");
    }

    #[test]
    fn a_command_with_no_key_list_has_no_keys() {
        let surface = surface();
        assert_eq!(command(&surface, "time.forward10").get("keys"), Some(&json!([])));
    }

    #[test]
    fn the_surface_is_the_same_every_time_it_is_built() {
        assert_eq!(render(&surface()).unwrap(), render(&surface()).unwrap());
    }

    #[test]
    fn the_text_is_sorted_pretty_and_ends_in_one_newline() {
        let text = render(&json!({"b": 1, "a": {"d": [{"z": 1, "y": 2}], "c": null}})).unwrap();
        assert_eq!(text, "{\n  \"a\": {\n    \"c\": null,\n    \"d\": [\n      {\n        \"y\": 2,\n        \"z\": 1\n      }\n    ]\n  },\n  \"b\": 1\n}\n");
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
        let ids: Vec<&str> = surface.get("commands").and_then(Value::as_array).unwrap().iter().map(id_of).collect();
        assert_eq!(ids, ["a.first", "b.second", "c.third"]);
        let keys: Vec<(&str, bool, &str)> = command(&surface, "b.second")
            .get("keys")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|key| {
                (key.get("name").and_then(Value::as_str).unwrap(), key.get("required") == Some(&json!(true)), key.get("type").and_then(Value::as_str).unwrap())
            })
            .collect();
        assert_eq!(keys, [("layer", false, "ref"), ("mode", true, "enum"), ("time", false, "json"), ("frame", false, "json")]);
        assert_eq!(command(&surface, "c.third").get("keys"), Some(&json!([])));
        assert_eq!(command(&surface, "a.first").get("journal"), Some(&json!(false)));
        assert_eq!(surface.get("engineVersion"), Some(&json!("1.2.3")));
        assert_eq!(surface.get("tools"), Some(&json!([])));
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
