//! The behaviours the plugin promises, each pinned by a named test: the confinement that refuses
//! accesses outside the container, the commands that need a helper answering `Unsupported`, the
//! plug-ins switched off, the method table's four rows, and `run` stopping at the right step.

mod common;

use common::Scratch;
use craftbridge_core::{CraftApp, CraftError, Engine};
use craftbridge_effects::EffectCraftApp;
use serde_json::{Value, json};

fn request(app: &mut EffectCraftApp, method: &str, params: Value) -> Result<Value, CraftError> {
    app.request(method, params)
}

#[test]
fn info_names_the_engine_its_version_and_the_builds_state() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let info = request(&mut app, "info", json!({})).expect("info answers");
    assert_eq!(info["engine"].as_str(), Some("effectcraft"));
    assert_eq!(info["version"].as_str(), Some(effectcraft_engine::project::APP_VERSION));
    let pin = info["pin"].as_str().expect("info names the pin");
    assert!(pin.len() == 40 && pin.chars().all(|digit| matches!(digit, '0'..='9' | 'a'..='f')), "the pin is the full commit id: {pin}");
    assert!(info["commands"].as_u64().is_some_and(|n| n > 0));
    assert!(info["effects"].as_u64().is_some_and(|n| n > 0));
    assert!(info["project"].is_object());
    assert!(info["activeComp"].is_null(), "a fresh session has no comp open");
    assert!(info["gpu"].is_null(), "no GPU has been attached, so the engine says so");
    assert_eq!(info["hardwareDecoding"].as_bool(), Some(false));
    assert_eq!(
        info["project"]["renderer"]["why"].as_str(),
        Some("headless: renders on the CPU"),
        "the session states how it renders, without promising a flag a phone cannot pass"
    );
}

#[test]
fn confinement_refuses_reads_outside_the_container() {
    // Dropping the confinement step at construction lets this read succeed and this test fail (L5).
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let error = (request(&mut app, "exec", json!({"command": "file.open", "params": {"path": "/etc/hosts"}}))).expect_err("the call was meant to fail");
    match error {
        CraftError::Io { message } => assert!(message.starts_with("cannot read `/etc/hosts`"), "the refusal keeps the engine's words: {message}"),
        other => panic!("reading outside the container must be `Io`, got {other:?}"),
    }
}

#[test]
fn confinement_refuses_writes_outside_the_container() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let error = (request(&mut app, "exec", json!({"command": "file.saveAs", "params": {"path": "/tmp/craftbridge-escape.ecproj"}})))
        .expect_err("the call was meant to fail");
    match error {
        CraftError::Io { message } => assert!(message.starts_with("cannot write"), "the refusal keeps the engine's words: {message}"),
        other => panic!("writing outside the container must be `Io`, got {other:?}"),
    }
}

#[test]
fn confinement_reads_inside_the_container_are_allowed() {
    // The same call the refusal test makes, aimed inside the container: refused for being no
    // project, never by the confinement, which is what shows the roots hold and do not over-refuse.
    let scratch = Scratch::new().expect("a scratch folder");
    let inside = scratch.path().join("nothing.ecproj");
    assert!(scratch.root().join("container").is_dir(), "the container sits in the scratch folder it was prepared from");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let error = (request(&mut app, "exec", json!({"command": "file.open", "params": {"path": inside}}))).expect_err("the call was meant to fail");
    assert!(!matches!(error, CraftError::Io { .. }), "an access inside the container is not refused by the confinement: {error}");
}

#[test]
fn a_path_the_confinement_cannot_check_is_the_callers_to_fix() {
    // A parameter that names no path is refused by the confinement before any file is touched; it
    // is the caller's mistake, `InvalidParams`, never a refused file access.
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let error = (request(&mut app, "exec", json!({"command": "file.open", "params": {"path": 5}}))).expect_err("the call was meant to fail");
    match error {
        CraftError::InvalidParams { message } => assert_eq!(message, "`file.open`: `path` must be a path"),
        other => panic!("a path that is no string must be `InvalidParams`, got {other:?}"),
    }
}

#[test]
fn a_missing_layer_is_not_found_in_each_of_the_engines_phrasings() {
    // The engine says a layer is missing as "no layer …" and as "no such layer", as a command's own
    // refusal and, for a list of layers, as a plain one; each is the §14 `NotFound` row, never a
    // command failing for its own reasons.
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    request(&mut app, "exec", json!({"command": "comp.new", "params": {"name": "A"}})).expect("a comp to look in");
    for (command, params, words) in [
        ("prop.set", json!({"comp": 1, "layer": 99, "path": "transform/opacity", "value": 5}), "prop.set: no layer 99"),
        ("camera.orbitNull", json!({"comp": 1, "layer": 99}), "camera.orbitNull: no such layer"),
        ("effect.apply", json!({"comp": 1, "layers": [99], "effect": "Gaussian Blur"}), "no layer 99 (use a layer id"),
    ] {
        let error = (request(&mut app, "exec", json!({"command": command, "params": params}))).expect_err("the call was meant to fail");
        match error {
            CraftError::NotFound { message } => assert!(message.starts_with(words), "the refusal keeps the engine's words: {message}"),
            other => panic!("`{command}` naming a missing layer must be `NotFound`, got {other:?}"),
        }
    }
}

#[test]
fn run_names_the_failed_step_whatever_row_it_maps_onto() {
    // Only `CommandFailed` carries a step field; every other row of a failed step carries the
    // step in its words, as the command line words every failed step of its `run`.
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let missing =
        (request(&mut app, "run", json!({"steps": [{"command": "comp.new", "params": {"name": "A"}}, {"command": "comp.info", "params": {"comp": "nope"}}]})))
            .expect_err("the call was meant to fail");
    match missing {
        CraftError::NotFound { message } => assert_eq!(message, "comp.info: no composition \"nope\" (step 1)"),
        other => panic!("a missing comp in a step must be `NotFound` naming the step, got {other:?}"),
    }
    let refused =
        (request(&mut app, "run", json!({"steps": [{"command": "file.open", "params": {"path": "/etc/hosts"}}]}))).expect_err("the call was meant to fail");
    match refused {
        CraftError::Io { message } => {
            assert!(message.starts_with("file.open: cannot read `/etc/hosts`") && message.ends_with(" (step 0)"), "the step is named: {message}")
        }
        other => panic!("a refused read in a step must be `Io` naming the step, got {other:?}"),
    }
}

#[test]
fn helper_needing_command_answers_unsupported_and_starts_nothing() {
    // The acceptance: a command that wants a helper is refused, and the session holds no helper
    // hooks at all, so no process the hooks would have started can be started by anything.
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    // Every way a call reaches a helper command: on its own, as a step of `run`, and folded into
    // one engine batch, whose steps the engine runs itself.
    let scan = json!({"command": "effect.helpers.scan", "params": {}});
    let load = json!({"command": "effect.helpers.load", "params": {"path": scratch.path().join("helper")}});
    for (method, params) in [
        ("exec", scan.clone()),
        ("exec", load.clone()),
        ("run", json!({"steps": [scan.clone(), load]})),
        ("exec", json!({"command": "engine.batch", "params": {"steps": [scan]}})),
    ] {
        match (request(&mut app, method, params.clone())).expect_err("the call was meant to fail") {
            CraftError::Unsupported { message } => {
                assert!(message.contains("`effect.helpers."), "the refusal names the withheld command: {message}")
            }
            other => panic!("a helper-needing command must be `Unsupported` through `{method}` {params}, got {other:?}"),
        }
    }
    // The engine's own report of the session it runs, through the two commands the confinement
    // lets through: no helper hooks, so no helper is listed, none is available, and none can start.
    let helpers = request(&mut app, "exec", json!({"command": "effect.helpers.list"})).expect("the helper list answers");
    assert_eq!(helpers, json!({"helpers": [], "blocked": [], "available": false}), "the engine sees no helper hooks");
    let plug_ins = request(&mut app, "exec", json!({"command": "effect.plugins.list"})).expect("the plug-in list answers");
    assert_eq!(plug_ins.get("helpers"), Some(&json!(false)), "the engine sees no helper hooks: {plug_ins}");
    assert_eq!(plug_ins.get("wasm"), Some(&json!(false)), "the engine sees no plug-in loader without a plug-in folder: {plug_ins}");
}

#[test]
fn model_downloads_answer_unsupported_and_start_no_process() {
    // The second promise of "no process can start": the model downloads are the commands that fetch
    // something outside the app by starting the system `curl` — the engine's own specs mark both
    // "desktop; uses the system curl". Every way a call reaches one is refused with the §14
    // `Unsupported` row: on its own, as a step of `run`, and as a step of an engine batch, in both
    // the spellings with which a batch step names its command. Nothing runs either way: the refusal
    // is the plug-in's, before the engine sees the call.
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let weights = json!({"id": "classical"});
    for command in ["roto.model.download", "face.model.download"] {
        match (request(&mut app, "exec", json!({"command": command, "params": weights.clone()}))).expect_err("the download was meant to be refused") {
            CraftError::Unsupported { message } => assert_eq!(
                message,
                format!("`{command}` is not available in this build: it downloads by starting the system `curl`, and this build starts no process"),
                "the refusal names the command and what it would start"
            ),
            other => panic!("a model download must be `Unsupported`, got {other:?}"),
        }
        let stepped = json!({"steps": [{"command": "comp.new", "params": {"name": "A"}}, {"command": command, "params": weights.clone()}]});
        match (request(&mut app, "run", stepped)).expect_err("the step was meant to be refused") {
            CraftError::Unsupported { message } => {
                assert!(
                    message.starts_with(&format!("{command}: `{command}` is not available")) && message.ends_with("(step 1)"),
                    "the failed step is named and numbered: {message}"
                )
            }
            other => panic!("a model download as a step of `run` must be `Unsupported`, got {other:?}"),
        }
        // A step of an engine batch: the engine runs its steps itself, so the id reaches nothing.
        for step in [json!({"command": command}), json!({"id": command})] {
            let batched = json!({"command": "engine.batch", "params": {"steps": [json!({"command": "comp.new", "params": {"name": "B"}}), step]}});
            match (request(&mut app, "exec", batched.clone())).expect_err("the batch step was meant to be refused") {
                CraftError::Unsupported { message } => {
                    assert!(message.starts_with(&format!("`{command}` is not available")), "the refusal names the withheld step: {message}")
                }
                other => panic!("a model download as a batch step must be `Unsupported`, got {other:?}"),
            }
            // The batch as a step of `run`: the step is named by its own command, as every row is.
            match (request(&mut app, "run", json!({"steps": [batched]}))).expect_err("the batch step was meant to be refused") {
                CraftError::Unsupported { message } => assert!(
                    message.starts_with(&format!("engine.batch: `{command}` is not available")) && message.ends_with("(step 0)"),
                    "the failed step is named by its command and numbered: {message}"
                ),
                other => panic!("a model download in a batch step of `run` must be `Unsupported`, got {other:?}"),
            }
        }
        // The one other command that runs the steps a call names, a command script read from a file
        // in the container: the engine runs its steps itself, so it is the confinement that withholds
        // the whole script before any step is read.
        let script = scratch.path().join("download.jsonl");
        std::fs::write(&script, format!("{}\n", json!({"command": command, "params": weights}))).expect("a command script to name");
        match (request(&mut app, "exec", json!({"command": "file.runScript", "params": {"path": script}})))
            .expect_err("the command script was meant to be refused")
        {
            CraftError::Unsupported { message } => assert!(message.contains("`file.runScript` is not available"), "the script is withheld: {message}"),
            other => panic!("a model download in a command script must be `Unsupported`, got {other:?}"),
        }
    }
    // And the phone's own way to get weights stays open: Install from File reads a file, starts
    // nothing — here it fails for the file not being weights, never for a withheld capability.
    let module = scratch.path().join("weights.bin");
    std::fs::write(&module, b"not weights").expect("a file to name");
    let error = (request(&mut app, "exec", json!({"command": "roto.model.install", "params": {"path": module}}))).expect_err("the file is no weights");
    assert!(!matches!(error, CraftError::Unsupported { .. }), "installing weights from a file inside the container is not withheld: {error:?}");
}

#[test]
fn plug_ins_switched_off_answer_unsupported() {
    let scratch = Scratch::new().expect("a scratch folder");
    let module = scratch.path().join("ghost.wasm");
    std::fs::write(&module, b"\0asm\x01\0\0\0").expect("a module file to name");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let error = (request(&mut app, "exec", json!({"command": "effect.plugins.load", "params": {"path": module}}))).expect_err("the call was meant to fail");
    match error {
        CraftError::Unsupported { message } => assert!(message.contains("plug-ins are not available"), "the refusal says why: {message}"),
        other => panic!("a plug-in with the switch off must be `Unsupported`, got {other:?}"),
    }
}

#[test]
fn exec_reports_an_unknown_parameter_as_invalid_params() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let error = (request(&mut app, "exec", json!({"command": "comp.new", "params": {"widht": 100}}))).expect_err("the call was meant to fail");
    match error {
        CraftError::InvalidParams { message } => {
            assert!(message.starts_with("comp.new: unknown parameter(s) `widht`"), "the refusal keeps the engine's words: {message}")
        }
        other => panic!("a refused parameter must be `InvalidParams`, got {other:?}"),
    }
}

#[test]
fn exec_reports_an_unknown_command_as_invalid_params() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let error = (request(&mut app, "exec", json!({"command": "no.such.command"}))).expect_err("the call was meant to fail");
    assert!(matches!(error, CraftError::InvalidParams { .. }), "an unknown command must be `InvalidParams`, got {error:?}");
}

#[test]
fn exec_reports_a_missing_comp_as_not_found() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let error = (request(&mut app, "exec", json!({"command": "comp.info", "params": {"comp": "nope"}}))).expect_err("the call was meant to fail");
    match error {
        CraftError::NotFound { message } => assert!(message.contains("nope"), "the refusal names what is missing: {message}"),
        other => panic!("a comp the call named that is not there must be `NotFound`, got {other:?}"),
    }
}

#[test]
fn run_returns_after_the_last_step_with_every_result() {
    let scratch = Scratch::new().expect("a scratch folder");
    let saved = scratch.path().join("built.ecproj");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let results = request(
        &mut app,
        "run",
        json!({"steps": [
            {"command": "comp.new", "params": {"name": "Built", "width": 320, "height": 240, "duration": 1}},
            {"command": "layer.newSolid", "params": {"comp": 1, "name": "Bg"}},
            {"command": "file.save", "params": {"path": saved}},
            {"command": "file.open", "params": {"path": saved}},
        ]}),
    )
    .expect("every step succeeds");
    let steps = results.as_array().expect("run answers an array");
    assert_eq!(steps.len(), 4, "one entry per step");
    assert_eq!(steps[3]["command"].as_str(), Some("file.open"));
    assert!(steps[3]["result"].is_object(), "each entry carries the command's result");
}

#[test]
fn run_fails_at_the_first_failure_with_its_step() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let error = (request(
        &mut app,
        "run",
        json!({"steps": [
            {"command": "comp.new", "params": {"name": "A"}},
            {"command": "layer.newSolid", "params": {"comp": 1, "name": "s"}},
            {"command": "effect.apply", "params": {"comp": 1, "layer": "s", "effect": "Gausian Blur"}},
            {"command": "comp.new", "params": {"name": "Never"}},
        ]}),
    ))
    .expect_err("the call was meant to fail");
    match error {
        CraftError::CommandFailed { command, message, step } => {
            assert_eq!(command, "effect.apply");
            assert_eq!(message, "unknown effect `Gausian Blur`");
            assert_eq!(step, Some(2), "the failed step is numbered from zero, as the command line numbers it");
        }
        other => panic!("a failed step must be `CommandFailed` with its step, got {other:?}"),
    }
    // Stopping is not only reported, it happened: the step after the failure never ran.
    let info = request(&mut app, "info", json!({})).expect("info answers after a failed run");
    let names: Vec<&str> = info["project"]["items"].as_array().map(|items| items.iter().filter_map(|i| i["name"].as_str()).collect()).unwrap_or_default();
    assert!(names.contains(&"A"), "the first step's work stays");
    assert!(!names.contains(&"Never"), "the step after the failure did not run: {names:?}");
}

#[test]
fn the_catalogue_reports_the_phones_capabilities() {
    // The engine's own capability voice: a session built without helper hooks says so in the
    // catalogue, so the app can see what answers before it asks. (Plug-ins answer here only at
    // run time — `plug_ins_switched_off_answer_unsupported` pins that refusal.)
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let catalogue = request(&mut app, "commands", json!({})).expect("`commands` answers");
    let items = catalogue.as_array().expect("the catalogue is an array");
    let find = |wanted: &str| items.iter().find(|item| item["id"].as_str() == Some(wanted));
    let helpers = find("effect.helpers.scan").expect("the helper command is listed");
    assert_eq!(helpers["enabled"].as_bool(), Some(false));
    assert_eq!(helpers["why"].as_str(), Some("effect helpers are not available in this build"));
}

#[test]
fn the_method_table_answers_unwired_methods_as_unsupported() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    // The fifteen names §14's table carries that `request` does not answer, and one name the table
    // carries not at all: every name answers `Unsupported` in the one refusal, whose exact words
    // are pinned so they claim only what is true however the method was asked — that `request`
    // lacks it, and which methods `request` answers.
    for method in [
        "props",
        "get",
        "set",
        "open",
        "save",
        "saveAs",
        "new",
        "frame",
        "export",
        "template.render",
        "template.open",
        "script",
        "replay",
        "timings",
        "mcp",
        "no.such.method",
    ] {
        let error = (request(&mut app, method, json!({}))).expect_err("the call was meant to fail");
        match error {
            CraftError::Unsupported { message } => assert_eq!(
                message,
                format!("`{method}` is not answered by `request` in this build, which answers `info`, `commands`, `exec` and `run`"),
                "the refusal names the method and what `request` answers"
            ),
            other => panic!("an unwired method must be `Unsupported`, got {other:?}"),
        }
    }
}

#[test]
fn the_methods_check_their_own_parameters() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    assert!(matches!(
        (request(&mut app, "exec", json!({"command": "comp.new", "extra": 1}))).expect_err("the call was meant to fail"),
        CraftError::InvalidParams { .. }
    ));
    assert!(matches!((request(&mut app, "exec", json!({}))).expect_err("the call was meant to fail"), CraftError::InvalidParams { .. }));
    assert!(matches!((request(&mut app, "run", json!({"steps": []}))).expect_err("the call was meant to fail"), CraftError::InvalidParams { .. }));
    assert!(matches!(
        (request(&mut app, "run", json!({"steps": [{"command": "comp.new"}, {"params": {}}]}))).expect_err("the call was meant to fail"),
        CraftError::InvalidParams { .. }
    ));
    assert!(matches!((request(&mut app, "info", json!({"x": 1}))).expect_err("the call was meant to fail"), CraftError::InvalidParams { .. }));
    assert!(matches!((request(&mut app, "commands", json!({"bogus": true}))).expect_err("the call was meant to fail"), CraftError::InvalidParams { .. }));
}

#[test]
fn the_confinement_withholds_scripts_executables_and_helper_loads() {
    // The confined session withholds the commands that run code or start helpers: confined file
    // access is what the engine cannot let a script or an agent open by itself (L5, AD9).
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    for command in ["script.run", "file.executeFile", "effect.helpers.load"] {
        let error =
            (request(&mut app, "exec", json!({"command": command, "params": {"path": scratch.path().join("x")}}))).expect_err("the call was meant to fail");
        assert!(matches!(error, CraftError::Unsupported { .. }), "`{command}` must be `Unsupported` while confined, got {error:?}");
    }
}

#[test]
fn the_app_drives_through_the_core_actor() {
    // The construction survives the move onto the engine thread (the session is `Send`), and the
    // method table answers there exactly as it does off it.
    let scratch = Scratch::new().expect("a scratch folder");
    let app = scratch.app().expect("the app starts from prepared folders");
    let engine = Engine::start(app).expect("the engine starts with the app");
    let info = engine.request("info", json!({})).expect("info answers on the engine thread");
    assert_eq!(info["engine"].as_str(), Some("effectcraft"));
    let unsupported = engine.request("frame", json!({})).expect_err("a name `request` does not answer");
    assert!(matches!(unsupported, CraftError::Unsupported { .. }));
    engine.close().expect("the engine closes clean");
}
