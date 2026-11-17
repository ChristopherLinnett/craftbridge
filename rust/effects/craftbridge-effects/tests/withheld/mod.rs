//! The check that a command the plug-in or the confinement withholds is refused on every path a call
//! can reach it by. The behaviours of the model downloads (the plug-in's door) and of the commands
//! that run a script (the engine's confinement) both use it, so the paths are listed once.

use craftbridge_core::{CraftApp, CraftError};
use craftbridge_effects::EffectCraftApp;
use serde_json::{Value, json};

/// Every way a call reaches one withheld command — on its own, as a step of `run`, and as a step of
/// an engine batch in both the spellings with which a batch step names its command, alone and as a
/// step of `run` — answers the §14 `Unsupported` row in `words` exactly, with only the command and
/// index a failed step of `run` is named by around them, before the engine sees the call, so nothing
/// runs. Whole words, never a prefix: a refusal by any other check, in its own words, would share the
/// opening of these. The answer says what was not refused.
pub fn assert_withheld_every_way(app: &mut EffectCraftApp, command: &str, params: &Value, words: &str) -> Result<(), String> {
    let refused_as = |app: &mut EffectCraftApp, method: &str, params: Value, expected: &str| match app.request(method, params) {
        Err(CraftError::Unsupported { message }) if message == expected => Ok(()),
        Err(CraftError::Unsupported { message }) => Err(format!("`{command}` through `{method}` must be refused in `{expected}`, got `{message}`")),
        other => Err(format!("`{command}` through `{method}` must be `Unsupported`, got {other:?}")),
    };
    let given = json!({"command": command, "params": params.clone()});
    refused_as(app, "exec", given.clone(), words)?;
    let stepped = json!({"steps": [{"command": "comp.new", "params": {"name": "A"}}, given]});
    refused_as(app, "run", stepped, &format!("{command}: {words} (step 1)"))?;
    // A step of an engine batch: the engine runs its steps itself, so the id reaches nothing.
    for step in [json!({"command": command, "params": params.clone()}), json!({"id": command, "params": params.clone()})] {
        let batched = json!({"command": "engine.batch", "params": {"steps": [json!({"command": "comp.new", "params": {"name": "B"}}), step]}});
        refused_as(app, "exec", batched.clone(), words)?;
        // The batch as a step of `run`: the step is named by its own command, as every row is.
        refused_as(app, "run", json!({"steps": [batched]}), &format!("engine.batch: {words} (step 0)"))?;
    }
    Ok(())
}
