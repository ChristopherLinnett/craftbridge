//! The `info` method: the engine's state as one document, in the command line's shape (plan §14).

use craftbridge_core::{CraftError, Result};
use craftbridge_media::Availability;
use effectcraft_automation::Backend;
use serde_json::Value;

use crate::generated::version::{ENGINE_PIN, ENGINE_VERSION};
use crate::methods::{empty_params, object, tool_json};

/// The engine this plugin answers for, named as plan §21 spells it.
const ENGINE: &str = "effectcraft";

/// `info`: `{engine, version, pin, commands, effects, project, activeComp, gpu, hardwareDecoding}`.
///
/// The project and active comp come from EffectCraft's own tools, so the document is the command
/// line's `info` with the plugin's identity beside it. `gpu` says where the engine renders
/// (plan §17): the attached device as `"<adapter> (<backend>)"`, the same words the core names the
/// device by, or — with no device attached — the session's own note of why it renders on the CPU,
/// or null only for a session this app did not build. `hardwareDecoding` reports whether this build
/// decodes on hardware: true once an engine start's registration found a hardware decoder (plan §18),
/// VideoToolbox's on an iPhone, or on Android one the codec list the app handed over names; false where it
/// found none, as in the iOS Simulator, on the desktop, and on an Android device whose list names none or
/// that was handed no list.
pub(crate) fn info(backend: &mut Backend, params: &Value) -> Result<Value> {
    if !(params.is_null() || params.as_object().is_some_and(serde_json::map::Map::is_empty)) {
        return Err(CraftError::invalid_params("`info`: takes no parameters"));
    }
    let project = tool_json(backend, "get_project", &empty_params())?;
    // The command line's info keeps an absent active comp as null: a session with no comp open is a
    // state, not a failure, and the app reads it off the same key.
    let active_comp = tool_json(backend, "get_comp", &empty_params()).unwrap_or(Value::Null);
    let gpu = match backend.session() {
        Some(session) => match (&session.accel, &session.accel_note) {
            (Some(accel), _) => Value::from(accel.name()),
            (None, Some(note)) => Value::from(note.clone()),
            (None, None) => Value::Null,
        },
        None => Value::Null,
    };
    Ok(object([
        ("engine", Value::from(ENGINE)),
        ("version", Value::from(ENGINE_VERSION)),
        ("pin", Value::from(ENGINE_PIN)),
        ("commands", Value::from(effectcraft_engine::command_specs().len())),
        ("effects", Value::from(effectcraft_engine::effects::registry().len())),
        ("project", project),
        ("activeComp", active_comp),
        ("gpu", gpu),
        ("hardwareDecoding", Value::Bool(decodes_on_hardware(craftbridge_media::availability()))),
    ]))
}

/// Whether `registered`, what the engine start's registration reported, left a hardware decoder in use: not
/// when no registration ran, nor when the device lists none or the app turned hardware decoding off.
fn decodes_on_hardware(registered: Option<Availability>) -> bool {
    matches!(registered, Some(Availability::Hardware(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_decoding_is_reported_only_for_a_registration_that_found_a_hardware_decoder() {
        assert!(decodes_on_hardware(Some(Availability::Hardware("MediaCodec"))));
        assert!(!decodes_on_hardware(Some(Availability::Unavailable)), "a device that lists no hardware decoder");
        assert!(!decodes_on_hardware(Some(Availability::Off)), "hardware decoding turned off");
        assert!(!decodes_on_hardware(None), "no registration has run");
    }
}
