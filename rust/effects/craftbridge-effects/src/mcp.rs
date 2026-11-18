//! The `mcp` call: one line of EffectCraft's MCP transport, answered by the server built at
//! construction (plan §14, §16).
//!
//! The server runs a tool's command through the confined backend, so the confinement holds on this
//! path as on every other. The model downloads and the renders and fills no one has listed the files of are
//! what the backend does not withhold, so the door that withholds them (AD11, AD3, AD20) is held here too:
//! a tool call that reaches one is answered as a failed tool call, in the words every other path
//! refuses it in, and the server never sees it.
//! The tools also run their commands past `methods::execute`, so each reply to a tool call that
//! runs a command passes through the font listing filter (AD2) before it is written, and a tool call
//! that reaches a command writing beside its source is handed the copy to read first (AD22), its
//! arguments written again where the tool reads them.

use std::borrow::Cow;
use std::path::Path;

use effectcraft_automation::server::call_result;
use effectcraft_automation::{Error, McpServer, tools};
use serde_json::{Map, Value};

use crate::fonts::{TEXT_FONTS, filtered};
use crate::imports;
use crate::methods::{empty_params, object, withheld};

/// The tools that run a command by its id, and so can reach a withheld command or a font listing, by
/// how they name it: one command, or a list of steps.
const ONE_COMMAND: [&str; 2] = ["execute_command", "command_run"];
const COMMAND_LISTS: [&str; 2] = ["command_batch", "batch"];
/// The tool that is the font listing command itself, and the arguments it passes on.
const LIST_FONTS: &str = "list_fonts";
const LIST_FONTS_ARGS: [&str; 3] = ["query", "variable", "rescan"];

/// A command a tool call runs, with its parameters.
type Step = (String, Value);

/// What a command's answer is passed through before it is written: `fonts::filtered`.
type Filter = fn(&str, &Value, Value) -> Value;

/// The commands one tool call runs, as the server's own tools read them.
struct Commands {
    /// Where the tool reads its commands' parameters from, which also says whether its answer holds
    /// one row per step.
    shape: Shape,
    /// Each step in order; `None` for one the server refuses before it runs it.
    steps: Vec<Option<Step>>,
}

/// Where a tool reads the parameters of the commands it runs.
#[derive(Clone, Copy)]
enum Shape {
    /// One command, whose parameters are the tool's `params`.
    One,
    /// An `engine.batch` the tool hands the engine whole, whose steps are the tool's `steps`.
    Batch,
    /// Steps the tool runs one by one (`command_batch`), each with the `params` of its own entry of the
    /// tool's `steps`; the answer holds one row per step.
    Rows,
    /// The font listing, whose one command the tool builds from arguments of its own.
    Fonts,
}

/// Answer one line as the server does, with the one difference that a message reaching a withheld
/// command is refused, and each tool reply's font listings are trimmed. A command a tool call runs
/// that would write beside a source in a folder the session may not write reads a copy in `folder`,
/// the app's staged folder.
pub(crate) fn answer(server: &mut McpServer, folder: &Path, line: &str) -> Option<String> {
    answer_with(server, Some(folder), line, filtered)
}

/// [`answer`] with the listing filter given, so a test can hand it the face list a phone would, and
/// with no staged folder for a server no app holds.
fn answer_with(server: &mut McpServer, folder: Option<&Path>, line: &str, filter: Filter) -> Option<String> {
    match serde_json::from_str::<Value>(line.trim()) {
        Ok(message) if !line.trim().is_empty() => handle(server, folder, &message, filter).map(|reply| reply.to_string()),
        _ => server.handle_line(line),
    }
}

/// Answer a message as the server does, a batch message by message. The server assembles a batch's
/// replies the same way: notifications answer nothing, and a batch that answers nothing is not
/// answered.
fn handle(server: &mut McpServer, folder: Option<&Path>, message: &Value, filter: Filter) -> Option<Value> {
    match message {
        Value::Array(batch) if !batch.is_empty() => {
            let replies: Vec<Value> = batch.iter().filter_map(|inner| handle(server, folder, inner, filter)).collect();
            (!replies.is_empty()).then_some(Value::Array(replies))
        }
        _ => match withheld_in(message) {
            Some(words) => refusal(message, words),
            None => handle_staged(server, folder, message, filter),
        },
    }
}

/// Answer a message the door lets through, its commands first handed the copy in `folder` of each
/// file they would write beside where the session may not write ([`imports::staged`]): a copy that
/// cannot be made is a failed tool call in the disk's words, and what the door leaves for after the
/// call is done once the server has answered.
fn handle_staged(server: &mut McpServer, folder: Option<&Path>, message: &Value, filter: Filter) -> Option<Value> {
    let Some(mut commands) = commands_in(message) else { return server.handle(message).map(|reply| listed(message, reply, filter)) };
    let mut calls: Vec<(&str, &mut Value)> = commands.steps.iter_mut().flatten().map(|(command, params)| (command.as_str(), params)).collect();
    let staged = match imports::staged(server.backend(), folder, &mut calls) {
        Ok(staged) => staged,
        Err(error) => return refusal(message, error.to_string()),
    };
    let message = if staged.rewritten() {
        let mut rewritten = message.clone();
        rewrite(&mut rewritten, &commands);
        Cow::Owned(rewritten)
    } else {
        Cow::Borrowed(message)
    };
    let reply = server.handle(&message).map(|reply| listed(&message, reply, filter));
    staged.finish(server.backend());
    reply
}

/// Write each command's parameters, as `commands` holds them now, where the tool reads them in
/// `message`: a value the tools take as JSON or as text holding it is written as JSON, which the
/// tools read alike (`json_of`).
fn rewrite(message: &mut Value, commands: &Commands) {
    let Some(arguments) = message.pointer_mut("/params/arguments").and_then(Value::as_object_mut) else { return };
    let first = commands.steps.first().and_then(Option::as_ref).map(|(_, params)| params);
    match commands.shape {
        Shape::One => {
            if let Some(params) = first {
                arguments.insert("params".to_string(), params.clone());
            }
        }
        Shape::Batch => {
            if let Some(steps) = first.and_then(|params| params.get("steps")) {
                arguments.insert("steps".to_string(), steps.clone());
            }
        }
        Shape::Rows => {
            let Some(Value::Array(mut steps)) = arguments.get("steps").and_then(json_of) else { return };
            for (step, given) in steps.iter_mut().zip(&commands.steps) {
                if let (Some(step), Some((_, params))) = (step.as_object_mut(), given) {
                    step.insert("params".to_string(), params.clone());
                }
            }
            arguments.insert("steps".to_string(), Value::Array(steps));
        }
        Shape::Fonts => {}
    }
}

/// The commands a `tools/call` request runs, if it is one the server runs. A notification is never
/// run by the server, so it runs nothing (`crates/automation/src/server.rs`:63-89), and nor does a
/// call whose arguments the server refuses before it runs the tool (:184-191): the server answers
/// both itself, as the command line's does. The tools are read as the server's own read them
/// (`crates/automation/src/tools.rs`:154-174, :202-206, :234-236, :496-503, :1012-1034): a command is
/// `command`, else `id`; `params` and a batch's `steps` may be given as JSON text; a call that
/// names no command, or text that is no JSON, runs nothing, and the server refuses it itself.
fn commands_in(message: &Value) -> Option<Commands> {
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
    if name == LIST_FONTS {
        let given = LIST_FONTS_ARGS.iter().filter_map(|key| Some((key.to_string(), args.get(key).filter(|value| !value.is_null())?.clone())));
        return Some(Commands { shape: Shape::Fonts, steps: vec![Some((TEXT_FONTS.to_string(), Value::Object(given.collect::<Map<_, _>>())))] });
    }
    if ONE_COMMAND.contains(&name) {
        return Some(Commands { shape: Shape::One, steps: vec![step_of(args)] });
    }
    if !COMMAND_LISTS.contains(&name) {
        return None;
    }
    let steps = json_of(args.get("steps")?)?;
    if name == "batch" {
        return Some(Commands { shape: Shape::Batch, steps: vec![Some(("engine.batch".to_string(), object([("steps", steps)])))] });
    }
    Some(Commands { shape: Shape::Rows, steps: steps.as_array()?.iter().map(step_of).collect() })
}

/// The words of the refusal of the first command a `tools/call` request reaches that the door
/// withholds, a model download or a render or a fill nothing has listed, if it reaches one.
fn withheld_in(message: &Value) -> Option<String> {
    let commands = commands_in(message)?;
    commands.steps.iter().flatten().find_map(|(command, params)| withheld(command, params))
}

/// The command one step of a tool call runs: the tool's `command`, else `id`, with its `params`.
fn step_of(step: &Value) -> Option<Step> {
    let given = |key: &str| step.get(key).filter(|value| !value.is_null());
    let command = given("command").or_else(|| given("id"))?.as_str()?;
    let params = match given("params") {
        None => empty_params(),
        Some(text) => json_of(text)?,
    };
    Some((command.to_string(), params))
}

/// A value the tools take as JSON or as text holding it: a blank text is none, `{}`.
fn json_of(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) if text.trim().is_empty() => Some(empty_params()),
        Value::String(text) => serde_json::from_str(text).ok(),
        other => Some(other.clone()),
    }
}

/// The reply to a tool call with each command's answer passed through the font listing filter, in
/// place: the answer is the JSON text of the reply's first content item, and a `command_batch`'s
/// rows are each filtered by the step they answer, matched by position. A reply that is no such
/// text, such as a failed call's message, is left as it is. The filter is a parameter so a test can
/// give it the face list a phone would.
fn listed(message: &Value, mut reply: Value, filter: Filter) -> Value {
    let Some(commands) = commands_in(message) else { return reply };
    let Some(text) = reply.pointer_mut("/result/content/0/text") else { return reply };
    let Some(mut answer) = text.as_str().and_then(|written| serde_json::from_str::<Value>(written).ok()) else { return reply };
    if matches!(commands.shape, Shape::Rows) {
        let rows = answer.get_mut("results").and_then(Value::as_array_mut).into_iter().flatten();
        for (row, step) in rows.zip(&commands.steps) {
            let Some((command, params)) = step else { continue };
            let Some(result) = row.get_mut("result") else { continue };
            *result = filter(command, params, result.take());
        }
    } else if let Some(Some((command, params))) = commands.steps.first() {
        answer = filter(command, params, answer);
    }
    *text = Value::from(answer.to_string());
    reply
}

/// The reply to a refused tool call, in the shape of every other failed tool call: a result
/// whose `isError` is set and whose text is the refusal. A client that negotiates per request names
/// its revision in `_meta`, and the server then marks results `resultType: complete`
/// (`crates/automation/src/server.rs`:281-304), so the refusal is marked the same way.
fn refusal(message: &Value, words: String) -> Option<Value> {
    let id = message.get("id")?.clone();
    let mut result = call_result(Err(Error::BadArgs(words)));
    let modern =
        message.pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion").and_then(Value::as_str).is_some_and(|version| version >= "2026-07-28");
    if let Some(fields) = result.as_object_mut().filter(|_| modern) {
        fields.insert("resultType".to_string(), Value::from("complete"));
    }
    Some(object([("jsonrpc", Value::from("2.0")), ("id", id), ("result", result)]))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::fonts::tests::{count_agrees, family_names, filtered_by_the_bundled_list, raw_listing};
    use effectcraft_automation::Backend;
    use serde_json::json;

    fn passes_through(_command: &str, _params: &Value, value: Value) -> Value {
        value
    }

    /// The request of a tool call, and the reply the server writes for the same tool when it answers `answer`.
    fn call(tool: &str, arguments: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": tool, "arguments": arguments } })
    }

    fn reply_with(answer: &Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "result": { "content": [{ "type": "text", "text": answer.to_string() }], "isError": false } })
    }

    pub(crate) fn answer_of(reply: &Value) -> Value {
        let text = reply.pointer("/result/content/0/text").and_then(Value::as_str).unwrap_or("null");
        serde_json::from_str(text).unwrap_or(Value::Null)
    }

    /// An app's MCP server, built as the app builds it, on folders that need not exist.
    fn server() -> McpServer {
        let folders = crate::session::tests::folders(None);
        McpServer::new(Backend::headless(crate::session::build(craftbridge_core::Platform::current(), &folders, &crate::hardware::HardwareChoice::default())))
    }

    /// The listing the server answers a line with, read from the reply at `pointer`.
    fn listing_through(filter: Filter, request: &Value, pointer: &str) -> Value {
        let reply = answer_with(&mut server(), None, &request.to_string(), filter).unwrap_or_default();
        let reply: Value = serde_json::from_str(&reply).unwrap_or(Value::Null);
        answer_of(&reply).pointer(pointer).cloned().unwrap_or(Value::Null)
    }

    fn steps() -> Value {
        json!([{ "command": TEXT_FONTS }])
    }

    /// Every tool path that carries a listing: the request, the server's answer to it holding the
    /// registry's raw listing, and where the listing sits in that answer.
    pub(crate) fn paths() -> Vec<(&'static str, Value, Value, &'static str)> {
        let raw = raw_listing();
        let named = json!({ "command": TEXT_FONTS });
        let by_id = json!({ "id": TEXT_FONTS, "params": "{}" });
        vec![
            ("list_fonts", call("list_fonts", json!({ "query": "Noto" })), raw.clone(), ""),
            ("execute_command", call("execute_command", named.clone()), raw.clone(), ""),
            ("command_run by id", call("command_run", by_id), raw.clone(), ""),
            ("batch", call("batch", json!({ "steps": steps() })), json!({ "steps": 1, "results": [raw.clone()] }), "/results/0"),
            ("batch given as text", call("batch", json!({ "steps": steps().to_string() })), json!({ "steps": 1, "results": [raw.clone()] }), "/results/0"),
            (
                "command_batch",
                call("command_batch", json!({ "steps": steps() })),
                json!({ "completed": 1, "failed": 0, "results": [{ "ok": true, "result": raw }] }),
                "/results/0/result",
            ),
        ]
    }

    #[test]
    fn every_tool_path_trims_its_listing_where_a_face_list_is_supplied() {
        for (name, request, answer, pointer) in paths() {
            let before = answer.pointer(pointer).cloned().unwrap_or(Value::Null);
            let reply = listed(&request, reply_with(&answer), filtered_by_the_bundled_list);
            let after = answer_of(&reply).pointer(pointer).cloned().unwrap_or(Value::Null);
            assert!(family_names(&after).len() < family_names(&before).len(), "{name}: the listing is trimmed");
            assert!(family_names(&after).iter().any(|family| family == "Noto Sans SC"), "{name}: the bundled face stays");
            assert!(count_agrees(&after), "{name}: `count` is recounted");
        }
    }

    #[test]
    fn every_tool_path_leaves_its_listing_where_no_face_list_is_supplied() {
        for (name, request, answer, _) in paths() {
            let reply = reply_with(&answer);
            assert_eq!(answer_of(&listed(&request, reply.clone(), passes_through)), answer, "{name}: nothing changes without a face list");
        }
    }

    #[test]
    fn the_server_answers_each_tool_through_the_filter() {
        // The wiring, end to end: the server runs the command and the line's reply is the trimmed one.
        for (name, request, _, pointer) in paths() {
            let trimmed = listing_through(filtered_by_the_bundled_list, &request, pointer);
            let whole = listing_through(passes_through, &request, pointer);
            assert!(family_names(&trimmed).len() < family_names(&whole).len(), "{name}: the server's own answer is trimmed on its way out");
            assert!(count_agrees(&trimmed), "{name}: `count` is recounted");
        }
    }

    #[test]
    fn a_batch_of_lines_is_trimmed_message_by_message() {
        let lines = json!([call("list_fonts", json!({})), call("list_fonts", json!({}))]);
        let reply = answer_with(&mut server(), None, &lines.to_string(), filtered_by_the_bundled_list).unwrap_or_default();
        let replies: Vec<Value> = serde_json::from_str(&reply).unwrap_or_default();
        assert_eq!(replies.len(), 2);
        for reply in &replies {
            assert!(family_names(&answer_of(reply)).len() < family_names(&raw_listing()).len(), "each reply of a batch is trimmed");
        }
    }

    #[test]
    fn a_command_batch_row_is_filtered_by_its_own_step() {
        let raw = raw_listing();
        let request = call("command_batch", json!({ "steps": [{ "command": "comp.new" }, { "command": TEXT_FONTS }, { "command": "comp.new" }] }));
        let answer =
            json!({ "completed": 3, "failed": 0, "results": [{ "ok": true, "result": raw }, { "ok": true, "result": raw }, { "ok": true, "result": raw }] });
        let reply = answer_of(&listed(&request, reply_with(&answer), filtered_by_the_bundled_list));
        assert_eq!(reply.pointer("/results/0/result"), Some(&raw), "a row of another command is not a listing");
        assert!(family_names(reply.pointer("/results/1/result").unwrap_or(&Value::Null)).len() < family_names(&raw).len(), "the `text.fonts` row is");
        assert_eq!(reply.pointer("/results/2/result"), Some(&raw));
    }

    #[test]
    fn another_commands_answer_and_a_failed_reply_are_left_alone() {
        let raw = raw_listing();
        let other = call("execute_command", json!({ "command": "comp.list" }));
        assert_eq!(answer_of(&listed(&other, reply_with(&raw), filtered_by_the_bundled_list)), raw, "a command that is no listing answers as it did");
        let failed = json!({ "jsonrpc": "2.0", "id": 1, "result": { "content": [{ "type": "text", "text": "no such command" }], "isError": true } });
        assert_eq!(listed(&call("list_fonts", json!({})), failed.clone(), filtered_by_the_bundled_list), failed, "a failed call's message is not a listing");
        let not_a_call = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        assert_eq!(answer_of(&listed(&not_a_call, reply_with(&raw), filtered_by_the_bundled_list)), raw);
    }
}
