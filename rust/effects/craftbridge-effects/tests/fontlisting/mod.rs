//! The app-facing font listing's shape (AD2), read the one way for every platform's font test:
//! what `exec` answers for `text.fonts`, the family names it answers, and whether its `count`
//! says what the list holds; and the refusal every confined session gives the script runner.
//! Only the font tests compile this module; its answers stay `Result` or `Vec` and the panics sit
//! in the `#[test]` functions, where the lint rules let them.

use std::path::{Path, PathBuf};

use craftbridge_core::{CraftApp, CraftError, Result};
use craftbridge_effects::EffectCraftApp;
use serde_json::{Value, json};

use crate::scratch::Scratch;

/// What the engine's confinement answers a `file.runScript` — any of its forms, a command list
/// holding `text.fonts` among them — in its own words: it refuses every script runner while file
/// access is confined, and every session the plug-in builds is.
pub const SCRIPT_RUNNER_REFUSED: &str = "`file.runScript` is not available while file access is confined (--automation-read-root/--automation-write-root)";

/// The command every font test reads the listing of.
pub const TEXT_FONTS: &str = "text.fonts";

/// The scratch folders of an app that carries its own font folders.
pub fn scratch_with_fonts(fonts: Vec<PathBuf>) -> Result<Scratch> {
    Scratch::build(None, fonts)
}

/// The scratch folders of an app that carries the crate's bundled fonts from where they sit in the
/// source tree. That path is live on the desktop and in the simulator, which reads the Mac's file
/// system; an Android device has no such path, so its test hands over a pushed copy instead.
#[cfg(not(target_os = "android"))]
pub fn scratch_with_bundled_fonts() -> Result<Scratch> {
    scratch_with_fonts(vec![PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts"))])
}

/// What `exec` answers for `text.fonts`.
pub fn font_listing(app: &mut EffectCraftApp) -> Result<Value> {
    app.request("exec", json!({ "command": TEXT_FONTS }))
}

/// What `exec` answers for a `file.runScript` of a command list, written into `folder`, that lists
/// the fonts: a request the command takes, so only the confinement can refuse it.
pub fn script_listing(app: &mut EffectCraftApp, folder: &Path) -> Result<Value> {
    let list = folder.join("fonts.json");
    std::fs::write(&list, json!([{ "command": TEXT_FONTS }]).to_string()).map_err(|e| CraftError::io(format!("a command list: {e}")))?;
    app.request("exec", json!({ "command": "file.runScript", "params": { "path": list } }))
}

/// Each MCP tool that carries a font listing, with what it is called with and where in its reply
/// the listing sits (a JSON pointer into the tool's JSON): the one table every platform's test reads
/// the `mcp` line's listings through.
const MCP_PATHS: [(&str, &str); 5] =
    [("list_fonts", ""), ("execute_command", ""), ("command_run", ""), ("batch", "/results/0"), ("command_batch", "/results/0/result")];

/// The listing each MCP tool answers for `text.fonts`, read from a reply to a `tools/call` line sent
/// through the app's `mcp` line, in the order of `MCP_PATHS`, each named by its tool.
pub fn mcp_listings(app: &mut EffectCraftApp) -> Result<Vec<(&'static str, Value)>> {
    let command = json!({ "command": TEXT_FONTS });
    let mut listings = Vec::new();
    for (id, (tool, pointer)) in MCP_PATHS.into_iter().enumerate() {
        let arguments = match tool {
            "list_fonts" => json!({}),
            "command_run" => json!({ "id": TEXT_FONTS }),
            "batch" | "command_batch" => json!({ "steps": [command] }),
            _ => command.clone(),
        };
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": tool, "arguments": arguments } });
        let reply = app.mcp(&line.to_string()).ok_or_else(|| CraftError::internal(format!("`{tool}` was not answered")))?;
        let reply: Value = serde_json::from_str(&reply).map_err(|e| CraftError::internal(format!("`{tool}` answered no JSON: {e}")))?;
        let text = reply
            .pointer("/result/content/0/text")
            .and_then(Value::as_str)
            .ok_or_else(|| CraftError::internal(format!("`{tool}` answered no text: {reply}")))?;
        let answer: Value = serde_json::from_str(text).map_err(|e| CraftError::internal(format!("`{tool}`'s text is no JSON: {e}")))?;
        let listing = answer.pointer(pointer).ok_or_else(|| CraftError::internal(format!("`{tool}` carries no listing at `{pointer}`: {answer}")))?;
        listings.push((tool, listing.clone()));
    }
    Ok(listings)
}

/// The family names of a listing, in the order it answers them; a value that answers no
/// families gives an empty list, which the tests' own comparisons catch.
pub fn family_names(listing: &Value) -> Vec<String> {
    listing
        .get("families")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("family").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// Whether the listing's `count` says what its `families` array holds.
pub fn count_agrees(listing: &Value) -> bool {
    listing.get("count").and_then(Value::as_u64) == listing.get("families").and_then(Value::as_array).map(Vec::len).and_then(|n| u64::try_from(n).ok())
}
