//! The `props`, `get` and `set` methods of the plan's §14 table: a layer's property tree, one
//! property, and a property after a change.
//!
//! Each goes through the same EffectCraft automation tool the command line's command of that name
//! goes through (`get_layer`, `get_property`, `set_property`), with the arguments the command line
//! builds from its own: a comp or layer is an id or a name, `-` or nothing is the active comp, and a
//! time is the decimal the command line parses. A depth is the whole number the tool reads, which
//! the command line's `props` loses (see `whole`).

use craftbridge_core::{CraftError, Result};
use effectcraft_automation::Backend;
use serde_json::Value;
use serde_json::map::Map;

use crate::methods::{bool_param, object, object_params, reject_unknown, string_param, tool_json};
use crate::reference::reference;

/// `props`: the layer's property tree, as a flat list of leaf properties when `flat` is set.
pub(crate) fn props(backend: &mut Backend, params: &Value) -> Result<Value> {
    let obj = object_params(params, "props")?;
    reject_unknown(&obj, "props", &["comp", "layer", "flat", "time", "depth"])?;
    let arguments = object([
        ("comp", reference(&obj, "comp", "props")?),
        ("layer", reference(&obj, "layer", "props")?),
        ("flat", Value::Bool(bool_param(&obj, "flat", "props")?.unwrap_or(false))),
        ("time", number(&obj, "time", "props")?),
        ("depth", whole(&obj, "depth", "props")?),
    ]);
    tool_json(backend, "get_layer", &arguments)
}

/// `get`: one property, at `time` when it is given.
pub(crate) fn get(backend: &mut Backend, params: &Value) -> Result<Value> {
    let obj = object_params(params, "get")?;
    reject_unknown(&obj, "get", &["comp", "layer", "path", "time"])?;
    let arguments = object([
        ("comp", reference(&obj, "comp", "get")?),
        ("layer", reference(&obj, "layer", "get")?),
        ("path", text(&obj, "path", "get")?),
        ("time", number(&obj, "time", "get")?),
    ]);
    tool_json(backend, "get_property", &arguments)
}

/// `set`: a value and/or an expression on one property, answered as `get` answers it. The value is
/// the JSON the command line would have parsed from its text, so it goes to the tool as it came.
pub(crate) fn set(backend: &mut Backend, params: &Value) -> Result<Value> {
    let obj = object_params(params, "set")?;
    reject_unknown(&obj, "set", &["comp", "layer", "path", "value", "time", "expression"])?;
    let arguments = object([
        ("comp", reference(&obj, "comp", "set")?),
        ("layer", reference(&obj, "layer", "set")?),
        ("path", text(&obj, "path", "set")?),
        ("value", obj.get("value").cloned().unwrap_or(Value::Null)),
        ("time", number(&obj, "time", "set")?),
        ("expression", text(&obj, "expression", "set")?),
    ]);
    tool_json(backend, "set_property", &arguments)
}

/// A string parameter, as the value the tool reads it in: absent is none.
fn text(obj: &Map<String, Value>, key: &str, method: &str) -> Result<Value> {
    Ok(string_param(obj, key, method)?.map_or(Value::Null, Value::from))
}

/// A time, which the command line parses as a decimal and hands the tool as one, a whole number
/// included.
fn number(obj: &Map<String, Value>, key: &str, method: &str) -> Result<Value> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::Number(number)) => Ok(number.as_f64().map_or(Value::Null, Value::from)),
        Some(_) => Err(CraftError::invalid_params(format!("`{method}`: `{key}` must be a number"))),
    }
}

/// A count of levels, which the tool reads only from a whole number and ignores in any other form.
/// It goes over as the whole number it is, so it limits the tree as the tool says. The command
/// line's `props --depth` parses its text as a decimal and so limits nothing; a library has no text
/// to parse, and a parameter that is taken and does nothing would be a false promise. Any other
/// number is refused rather than ignored for the same reason.
fn whole(obj: &Map<String, Value>, key: &str, method: &str) -> Result<Value> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::Number(count)) if count.is_u64() => Ok(Value::Number(count.clone())),
        Some(_) => Err(CraftError::invalid_params(format!("`{method}`: `{key}` must be a whole number"))),
    }
}
