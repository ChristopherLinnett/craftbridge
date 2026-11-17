//! The behaviours of the `replay`, `mcp` and `timings` rows: the door that withholds the model
//! downloads (AD11) and the confinement (L5) held on every path that hands a command to the engine,
//! the shapes of the answers, and the one thing not built, `script`.

mod scratch;

use craftbridge_core::{CraftApp, CraftError};
use effectcraft_automation::confine::Confinement;
use scratch::Scratch;
use serde_json::{Value, json};

const DOWNLOAD_WORDS: &str = "is not available in this build: it downloads by starting the system `curl`, and this build starts no process";

/// The two commands that start a process, which the door withholds on every path.
const DOWNLOADS: [&str; 2] = ["roto.model.download", "face.model.download"];

/// A file outside the container, which the confinement refuses to read.
const OUTSIDE: &str = "/etc/hosts";

/// The door's refusal of `id`, in the words every path refuses it in.
fn withheld(id: &str) -> String {
    format!("`{id}` {DOWNLOAD_WORDS}")
}

/// What the confinement itself answers `command` with, built over the scratch app's own roots: the
/// words every path must refuse it in.
fn confinement_says(scratch: &Scratch, command: &str, params: &Value) -> Result<String, String> {
    let roots = [scratch.path().to_path_buf()];
    let confinement = Confinement::new(&roots, &roots).map_err(|e| format!("the confinement builds: {e}"))?;
    match confinement.check_command(command, params) {
        Err(refused) => Ok(refused.to_string()),
        Ok(_) => Err(format!("the confinement lets `{command}` {params} through")),
    }
}

fn reply(app: &mut craftbridge_effects::EffectCraftApp, message: &Value) -> Result<Value, String> {
    let line = app.mcp(&message.to_string()).ok_or("the line is answered")?;
    serde_json::from_str(&line).map_err(|e| format!("the answer is JSON: {e}"))
}

fn tool_call(id: u64, tool: &str, arguments: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": tool, "arguments": arguments}})
}

fn text_of(answer: &Value) -> Result<String, String> {
    answer.pointer("/result/content/0/text").and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("a text result: {answer}"))
}

fn comp_count(app: &mut craftbridge_effects::EffectCraftApp) -> Result<usize, String> {
    let info = app.request("info", json!({})).map_err(|e| e.to_string())?;
    Ok(info.pointer("/project/items").and_then(Value::as_array).map_or(0, Vec::len))
}

#[test]
fn a_replay_runs_its_lines_and_reports_as_the_command_line_does() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts");
    let script = [
        r#"{"start": "new"}"#,
        r##"# a comment"##,
        r#"{"command": "comp.new", "params": {"name": "R", "width": 64, "height": 64}}"#,
        r#"{"expect": {"query": "comp.info", "params": {"comp": 1}, "pointer": "/name", "equals": "R"}}"#,
        r#"{"command": "comp.info", "params": {"comp": "nope"}, "expect_error": "nope"}"#,
    ]
    .join("\n");
    let report = app.request("replay", json!({"script": script})).expect("the replay answers");
    assert_eq!(report["ok"], json!(true), "{report}");
    assert_eq!(report["steps"].as_array().map(Vec::len), Some(4), "{report}");
    // A failing step is an answer, named by line, not an error of the call.
    let report = app.request("replay", json!({"script": "{\"command\": \"comp.info\", \"params\": {\"comp\": \"nope\"}}"})).expect("answers");
    assert_eq!(report["ok"], json!(false));
    assert_eq!(report["first_failure"]["line"], json!(1));
}

/// Every spelling a replay line gives a command — its `command`, an `expect` query, a step of an
/// `engine.batch` by `command` or by `id`, and the `start` line's project — refused whole before any
/// line runs, in the words of the door or the confinement and on their rows: each model download,
/// a read outside the container, a script, and a command whose destination only a live lookup can
/// tell, which the confinement refuses in a batch.
#[test]
fn a_replay_is_refused_whole_by_the_door_and_the_confinement_before_any_line_runs() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts");
    let first = r#"{"command": "comp.new", "params": {"name": "Left behind"}}"#;
    let weights = json!({"id": "classical"});
    let in_a_batch = |step: Value| json!({"command": "engine.batch", "params": {"steps": [{"command": "comp.new"}, step]}});
    let mut refused: Vec<(Value, String, fn(&CraftError) -> bool)> = Vec::new();
    let unsupported: fn(&CraftError) -> bool = |e| matches!(e, CraftError::Unsupported { .. });
    let io: fn(&CraftError) -> bool = |e| matches!(e, CraftError::Io { .. });
    let invalid: fn(&CraftError) -> bool = |e| matches!(e, CraftError::InvalidParams { .. });
    for id in DOWNLOADS {
        for line in [
            json!({"command": id, "params": weights}),
            json!({"expect": {"query": id, "params": weights}}),
            in_a_batch(json!({"command": id, "params": weights})),
            in_a_batch(json!({"id": id, "params": weights})),
        ] {
            refused.push((line, withheld(id), unsupported));
        }
    }
    let read = json!({"path": OUTSIDE});
    let outside = confinement_says(&scratch, "file.open", &read).expect("the confinement's words");
    for line in [
        json!({"command": "file.open", "params": read}),
        json!({"start": {"project": OUTSIDE}}),
        json!({"expect": {"query": "file.open", "params": read}}),
        in_a_batch(json!({"command": "file.open", "params": read})),
    ] {
        refused.push((line, outside.clone(), io));
    }
    for command in ["script.run", "file.runScript"] {
        let words = confinement_says(&scratch, command, &json!({})).expect("the confinement's words");
        refused.push((json!({"command": command, "params": {"code": "1"}}), words.clone(), unsupported));
        refused.push((in_a_batch(json!({"command": command, "params": {"code": "1"}})), words, unsupported));
    }
    let lookup = "`file.save` writes to its project's or queue's files: run it on its own while file access is confined".to_string();
    refused.push((json!({"command": "file.save", "params": {}}), lookup, invalid));
    for (line, words, row) in refused {
        // A `start` line runs only first, so it is refused before a line that would run after it.
        let (script, at) = if line.get("start").is_some() { (format!("{line}\n{first}\n"), "line 1: ") } else { (format!("{first}\n{line}\n"), "line 2: ") };
        let error = app.request("replay", json!({"script": script})).expect_err("the line is refused");
        assert!(row(&error), "{line}: refused on the wrong row: {error:?}");
        assert!(error.to_string().starts_with(at) && error.to_string().contains(&words), "{line}: not in its own words `{words}`: {error}");
        assert_eq!(comp_count(&mut app).expect("a count"), 0, "{line}: a line of the refused script ran");
    }
}

#[test]
fn replay_checks_its_parameters() {
    let scratch = Scratch::new().expect("a scratch folder");
    assert!(scratch.root().exists() && scratch.path().exists());
    let mut app = scratch.app().expect("the app starts");
    for params in [json!({}), json!({"script": 1}), json!({"script": "", "keepGoing": "yes"}), json!({"script": "", "x": 1})] {
        let error = app.request("replay", params).expect_err("bad parameters");
        assert!(matches!(error, CraftError::InvalidParams { .. }), "{error:?}");
    }
}

#[test]
fn mcp_answers_on_the_session_the_methods_act_on() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts");
    let made = reply(&mut app, &tool_call(1, "execute_command", json!({"command": "comp.new", "params": {"name": "Via MCP"}}))).expect("answered");
    assert_eq!(made["result"]["isError"], json!(false), "{made}");
    let info = app.request("info", json!({})).expect("info");
    assert_eq!(info["activeComp"]["name"], json!("Via MCP"), "the line and the method share one session");
    assert!(app.mcp("").is_none(), "an empty line needs no reply");
    assert!(app.mcp(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string()).is_none());
    // `mcp` is a call of its own, never a method of `request`.
    assert!(matches!(app.request("mcp", json!({})).expect_err("refused"), CraftError::Unsupported { .. }));
}

/// Each model download, by every tool that runs a command and every way the server reads one —
/// `command` or `id`, parameters as JSON or as text, a step of `command_batch`, of `batch` (steps as
/// JSON or as text) and of an `engine.batch` run as one command — refused in the door's words, alone
/// and inside a JSON-RPC batch, where the other messages are answered as the server answers them.
#[test]
fn mcp_withholds_the_model_downloads_on_every_tool_that_runs_a_command() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts");
    let weights = json!({"id": "classical"});
    for id in DOWNLOADS {
        let step = json!({"command": id, "params": weights});
        let id_step = json!({"id": id, "params": weights});
        let in_a_batch = json!({"command": "engine.batch", "params": {"steps": [{"command": "comp.new"}, id_step]}});
        let calls = [
            tool_call(1, "execute_command", step.clone()),
            tool_call(2, "execute_command", id_step.clone()),
            tool_call(3, "command_run", id_step.clone()),
            tool_call(4, "execute_command", json!({"command": id, "params": weights.to_string()})),
            tool_call(5, "execute_command", in_a_batch),
            tool_call(6, "command_batch", json!({"steps": [{"id": "comp.new"}, id_step]})),
            tool_call(7, "command_batch", json!({"steps": [{"command": "comp.new"}, step]})),
            tool_call(8, "batch", json!({"steps": [{"command": "comp.new"}, step]})),
            tool_call(9, "batch", json!({"steps": json!([id_step]).to_string()})),
        ];
        for call in &calls {
            let answer = reply(&mut app, call).expect("answered");
            assert_eq!(answer["id"], call["id"]);
            assert_eq!(answer["result"]["isError"], json!(true), "{call}: {answer}");
            assert!(text_of(&answer).expect("text").contains(&withheld(id)), "{call}: {answer}");
            // The same call as one message of a JSON-RPC batch, beside a call the server answers
            // and a notification, which it never answers.
            let line = json!([call, tool_call(99, "get_project", json!({})), {"jsonrpc": "2.0", "method": "notifications/initialized"}]).to_string();
            let answers: Vec<Value> = serde_json::from_str(&app.mcp(&line).expect("answered")).expect("a batch of answers");
            assert_eq!(answers.len(), 2, "{line}");
            assert!(answers[0]["id"] == call["id"] && text_of(&answers[0]).expect("text").contains(&withheld(id)), "{line}: {answers:?}");
            assert_eq!((answers[1]["id"].clone(), answers[1]["result"]["isError"].clone()), (json!(99), json!(false)), "{line}");
        }
        // A notification is never run, so it is never answered, refused or not.
        let mut notification = calls[0].clone();
        notification.as_object_mut().expect("an object").remove("id");
        assert!(app.mcp(&notification.to_string()).is_none());
    }
    assert_eq!(comp_count(&mut app).expect("a count"), 0, "a refused call ran nothing, the batch's earlier steps included");
}

/// A call whose arguments the server refuses before it runs the tool runs nothing, so the door leaves
/// it to the server, which answers it as the command line does, download or not.
#[test]
fn mcp_leaves_a_call_the_server_refuses_unrun_to_the_server() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts");
    for arguments in [json!({"command": "roto.model.download", "params": {"id": "classical"}, "extra": 1}), json!("roto.model.download")] {
        let answer = reply(&mut app, &tool_call(1, "execute_command", arguments.clone())).expect("answered");
        assert!(answer["error"]["code"] == json!(-32602) && answer.get("result").is_none(), "{arguments}: {answer}");
    }
}

#[test]
fn a_refused_call_is_marked_as_the_server_marks_its_own_for_a_client_that_negotiates_per_request() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts");
    let meta = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28"});
    let mut refused = tool_call(1, "execute_command", json!({"command": "roto.model.download", "params": {"id": "classical"}}));
    refused["params"]["_meta"] = meta.clone();
    let mut server_refused = tool_call(2, "execute_command", json!({"command": "script.run", "params": {"code": "1"}}));
    server_refused["params"]["_meta"] = meta;
    let (ours, theirs) = (reply(&mut app, &refused).expect("answered"), reply(&mut app, &server_refused).expect("answered"));
    let keys = |answer: &Value| answer["result"].as_object().map(|result| result.keys().cloned().collect::<Vec<_>>());
    assert!(text_of(&ours).expect("text").contains(&withheld("roto.model.download")), "the door answered: {ours}");
    assert_eq!(keys(&ours), keys(&theirs), "the refusal carries the fields the server's own failed call carries");
    assert_eq!(ours["result"]["resultType"], json!("complete"));
}

/// A read outside the container and a script, by each tool that runs a command and as a step of
/// each batch, answered in the confinement's own words: the server runs every tool's command through
/// the confined backend.
#[test]
fn mcp_keeps_the_confinement_and_withholds_scripts() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts");
    let read = json!({"path": OUTSIDE});
    let code = json!({"code": "1 + 1"});
    for (command, params) in [("file.open", read), ("script.run", code.clone())] {
        let words = confinement_says(&scratch, command, &params).expect("the confinement's words");
        let step = json!({"command": command, "params": params});
        for call in [
            tool_call(1, "execute_command", step.clone()),
            tool_call(2, "command_run", json!({"id": command, "params": params})),
            tool_call(3, "command_batch", json!({"steps": [step]})),
            tool_call(4, "batch", json!({"steps": [step]})),
        ] {
            let answer = reply(&mut app, &call).expect("answered");
            assert_eq!(answer["result"]["isError"], json!(true), "{call}: {answer}");
            assert!(text_of(&answer).expect("text").contains(&words), "{call}: not in the confinement's words `{words}`: {answer}");
        }
    }
    let script = reply(&mut app, &tool_call(5, "run_script", code)).expect("answered");
    assert_eq!(script["result"]["isError"], json!(true));
    let words = confinement_says(&scratch, "script.run", &json!({})).expect("the confinement's words");
    assert!(text_of(&script).expect("text").contains(&words), "{script}");
}

/// Every tool the server offers, each argument it takes given a model download in the shape that
/// argument takes — one at a time and all at once, alone and inside a JSON-RPC batch — reaches no
/// download: the engine's own refusal of a model id it does not know, which it makes before it
/// starts anything, is how a call that got past the door shows itself. A pin that adds a tool which
/// runs a command, under any argument name, fails here until the door reads it.
#[test]
fn no_tool_reaches_a_model_download_past_the_door() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts");
    let unknown = json!({"id": "no-such-model"});
    for tool in effectcraft_automation::tools::available(false) {
        let schema = tool.input_schema();
        let keys: Vec<String> = schema.get("properties").and_then(Value::as_object).map(|p| p.keys().cloned().collect()).unwrap_or_default();
        for id in DOWNLOADS {
            let given = |key: &str| match key {
                "params" => unknown.clone(),
                "steps" => json!([{"command": id, "params": unknown}, {"id": id, "params": unknown}]),
                _ => json!(id),
            };
            let mut argument_sets: Vec<Value> = keys.iter().map(|key| json!({key.as_str(): given(key)})).collect();
            argument_sets.push(Value::Object(keys.iter().map(|key| (key.clone(), given(key))).collect()));
            for arguments in argument_sets {
                let call = tool_call(1, tool.name, arguments);
                for line in [call.to_string(), json!([call]).to_string()] {
                    let answer = app.mcp(&line).unwrap_or_default();
                    for reached in [format!("invalid parameters for `{id}`: unknown model"), format!("invalid parameters for `{id}`: missing `id`")] {
                        assert!(!answer.contains(&reached), "{line} reached the download: {answer}");
                    }
                }
            }
        }
    }
}

#[test]
fn script_is_not_answered_while_the_confinement_cannot_hold_for_it() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts");
    assert!(matches!(app.request("script", json!({"code": "1"})).expect_err("refused"), CraftError::Unsupported { .. }));
    for method in ["exec", "run"] {
        let params = if method == "exec" {
            json!({"command": "script.run", "params": {"code": "1"}})
        } else {
            json!({"steps": [{"command": "script.run", "params": {"code": "1"}}]})
        };
        assert!(matches!(app.request(method, params).expect_err("refused"), CraftError::Unsupported { .. }));
    }
}

#[test]
fn timings_have_the_structure_of_the_benchmark() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts");
    app.request("run", json!({"steps": [{"command": "file.openDemoProject"}]})).expect("the demo opens");
    let t = app.request("timings", json!({"time": 1, "scale": 0.25, "runs": 2, "play": 2})).expect("timings answer");
    assert_eq!(t["runs"], json!(2));
    let frame = &t["frame"];
    for key in ["wallMinMs", "wallMedianMs", "cpuMinMs"] {
        assert!(frame[key].as_f64().is_some_and(|ms| ms >= 0.0), "{key}: {frame}");
    }
    let layers = frame["layers"].as_array().expect("layers");
    assert!(!layers.is_empty());
    for layer in layers {
        assert!(layer["layer"].is_string() && layer["depth"].is_u64() && layer["processMs"].is_f64() && layer["compositeMs"].is_f64(), "{layer}");
        for effect in layer["effects"].as_array().expect("effects") {
            assert!(effect["id"].is_string() && effect["ms"].is_f64() && effect.get("gpuSupported").is_none(), "{effect}");
        }
    }
    for pass in ["noCache", "layerCache"] {
        assert_eq!(t["play"][pass]["frames"], json!(2));
        assert!(t["play"][pass]["wallMeanMs"].is_f64());
    }
    assert!(t["play"]["cache"]["hits"].is_u64());
    assert!(t.get("gpu").is_none());
}

#[test]
fn timings_refuse_what_they_cannot_time() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts");
    assert!(matches!(app.request("timings", json!({})).expect_err("no comp"), CraftError::NotFound { .. }));
    app.request("run", json!({"steps": [{"command": "comp.new", "params": {"name": "T"}}]})).expect("a comp");
    for params in [
        json!({"runs": 0}),
        json!({"runs": 1001}),
        json!({"runs": 1.5}),
        json!({"play": 0}),
        json!({"scale": 0}),
        json!({"scale": 2, "runs": 1}),
        json!({"scale": "x"}),
        json!({"gpu": 1}),
        json!({"x": 1}),
    ] {
        let error = app.request("timings", params.clone()).expect_err("bad parameters");
        assert!(matches!(error, CraftError::InvalidParams { .. }), "{params}: {error:?}");
    }
    assert!(matches!(app.request("timings", json!({"comp": "nope"})).expect_err("no such comp"), CraftError::NotFound { .. }));
    let error = app.request("timings", json!({"gpu": true, "runs": 1})).expect_err("no GPU attached");
    assert!(matches!(&error, CraftError::Unsupported { message } if message.contains("headless: renders on the CPU")), "{error:?}");
}
