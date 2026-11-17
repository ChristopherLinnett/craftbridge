//! The `mcp` call: one line of EffectCraft's MCP transport, answered by the server built at
//! construction (plan §14, §16).
//!
//! The server runs a tool's command through the confined backend, so the confinement holds on this
//! path as on every other. The model downloads are the one thing the backend does not withhold, so
//! the door that withholds them (AD11) is held here too: a tool call that reaches one is answered
//! as a failed tool call, in the words every other path refuses it in, and the server never sees it.

use effectcraft_automation::server::call_result;
use effectcraft_automation::{Error, McpServer, tools};
use serde_json::Value;

use crate::methods::{empty_params, object, starts_a_process, wants_a_process};

/// The tools that run a command by its id, and so can reach a model download, by how they name it:
/// one command, or a list of steps.
const ONE_COMMAND: [&str; 2] = ["execute_command", "command_run"];
const COMMAND_LISTS: [&str; 2] = ["command_batch", "batch"];

/// Answer one line: the server's own answer unless a message in it reaches a model download.
pub(crate) fn answer(server: &mut McpServer, line: &str) -> Option<String> {
    let guarded = serde_json::from_str::<Value>(line.trim()).ok().filter(reaches_a_download);
    match guarded {
        None => server.handle_line(line),
        Some(message) => handle(server, &message).map(|reply| reply.to_string()),
    }
}

/// Whether a message, or any message of a batch of them, reaches a model download.
fn reaches_a_download(message: &Value) -> bool {
    match message {
        Value::Array(batch) => batch.iter().any(reaches_a_download),
        _ => download_in(message).is_some(),
    }
}

/// Answer a message as the server does, a batch message by message, with the one difference that a
/// message reaching a download is refused. The server assembles a batch's replies the same way:
/// notifications answer nothing, and a batch that answers nothing is not answered.
fn handle(server: &mut McpServer, message: &Value) -> Option<Value> {
    match message {
        Value::Array(batch) if !batch.is_empty() => {
            let replies: Vec<Value> = batch.iter().filter_map(|inner| handle(server, inner)).collect();
            (!replies.is_empty()).then_some(Value::Array(replies))
        }
        _ => match download_in(message) {
            Some(command) => refusal(message, &command),
            None => server.handle(message),
        },
    }
}

/// The model download a `tools/call` request reaches, if it does. A notification is never run by the
/// server, so it reaches nothing (`crates/automation/src/server.rs`:63-89), and nor does a call whose
/// arguments the server refuses before it runs the tool (:184-191): the server answers both itself,
/// as the command line's does. The tools are read as the server's own read them
/// (`crates/automation/src/tools.rs`:154-174, :202-206, :496-503, :1012-1034): a command is
/// `command`, else `id`; `params` and a batch's `steps` may be given as JSON text; a call that
/// names no command, or text that is no JSON, runs nothing, and the server refuses it itself.
fn download_in(message: &Value) -> Option<String> {
    message.get("id")?;
    if message.get("method").and_then(Value::as_str) != Some("tools/call") {
        return None;
    }
    let call = message.get("params")?;
    let name = call.get("name").and_then(Value::as_str)?;
    let empty = empty_params();
    let args = call.get("arguments").unwrap_or(&empty);
    if tools::find(name).is_none_or(|tool| tool.unknown_arg(args).is_some()) {
        return None;
    }
    if ONE_COMMAND.contains(&name) {
        return download_of_step(args);
    }
    if !COMMAND_LISTS.contains(&name) {
        return None;
    }
    let steps = json_of(args.get("steps")?)?;
    if name == "batch" {
        return wants_a_process("engine.batch", &object([("steps", steps)])).map(str::to_string);
    }
    steps.as_array()?.iter().find_map(download_of_step)
}

/// The download one command of a tool call reaches: the tool's `command`, else `id`, with its `params`.
fn download_of_step(step: &Value) -> Option<String> {
    let given = |key: &str| step.get(key).filter(|value| !value.is_null());
    let command = given("command").or_else(|| given("id"))?.as_str()?;
    let params = match given("params") {
        None => empty_params(),
        Some(text) => json_of(text)?,
    };
    wants_a_process(command, &params).map(str::to_string)
}

/// A value the tools take as JSON or as text holding it: a blank text is none, `{}`.
fn json_of(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) if text.trim().is_empty() => Some(empty_params()),
        Value::String(text) => serde_json::from_str(text).ok(),
        other => Some(other.clone()),
    }
}

/// The reply to a refused tool call, in the shape of every other failed tool call: a result
/// whose `isError` is set and whose text is the refusal. A client that negotiates per request names
/// its revision in `_meta`, and the server then marks results `resultType: complete`
/// (`crates/automation/src/server.rs`:281-304), so the refusal is marked the same way.
fn refusal(message: &Value, command: &str) -> Option<Value> {
    let id = message.get("id")?.clone();
    let mut result = call_result(Err(Error::BadArgs(starts_a_process(command))));
    let modern =
        message.pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion").and_then(Value::as_str).is_some_and(|version| version >= "2026-07-28");
    if let Some(fields) = result.as_object_mut().filter(|_| modern) {
        fields.insert("resultType".to_string(), Value::from("complete"));
    }
    Some(object([("jsonrpc", Value::from("2.0")), ("id", id), ("result", result)]))
}
