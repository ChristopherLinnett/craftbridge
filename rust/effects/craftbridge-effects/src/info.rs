//! The `info` method: the engine's state as one document, in the command line's shape (plan §14).

use craftbridge_core::{CraftError, Result};
use effectcraft_automation::Backend;
use serde_json::Value;

use crate::methods::{empty_params, object, tool_json};

/// The engine this plugin answers for, named as plan §21 spells it.
const ENGINE: &str = "effectcraft";

/// The EffectCraft revision the workspace pins, which is what this plugin builds. It is written here
/// once, and the crate's tests hold `info`'s `pin` to the pin in the workspace manifest, so the two
/// cannot drift; plan §21 has the generator write it into `src/generated/` instead.
const ENGINE_PIN: &str = "10376058";

/// `info`: `{engine, version, pin, commands, effects, project, activeComp, gpu, hardwareDecoding}`.
///
/// The project and active comp come from EffectCraft's own tools, so the document is the command
/// line's `info` with the plugin's identity beside it. `gpu` is null while no GPU has been attached
/// — the engine renders on the CPU, which is its state until a device is attached — and
/// `hardwareDecoding` reports whether this build decodes on hardware, which it does not yet.
pub(crate) fn info(backend: &mut Backend, params: &Value) -> Result<Value> {
    if !(params.is_null() || params.as_object().is_some_and(serde_json::map::Map::is_empty)) {
        return Err(CraftError::invalid_params("`info`: takes no parameters"));
    }
    let project = tool_json(backend, "get_project", &empty_params())?;
    // The command line's info keeps an absent active comp as null: a session with no comp open is a
    // state, not a failure, and the app reads it off the same key.
    let active_comp = tool_json(backend, "get_comp", &empty_params()).unwrap_or(Value::Null);
    Ok(object([
        ("engine", Value::from(ENGINE)),
        ("version", Value::from(effectcraft_engine::project::APP_VERSION)),
        ("pin", Value::from(ENGINE_PIN)),
        ("commands", Value::from(effectcraft_engine::command_specs().len())),
        ("effects", Value::from(effectcraft_engine::effects::registry().len())),
        ("project", project),
        ("activeComp", active_comp),
        ("gpu", Value::Null),
        ("hardwareDecoding", Value::Bool(false)),
    ]))
}
