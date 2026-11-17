//! The behaviours the plugin promises, each pinned by a named test: the confinement that refuses
//! accesses outside the container, the commands that need a helper answering `Unsupported`, the
//! plug-ins switched off, the method table's rows, `run` stopping at the right step, and the
//! GPU's states of plan §17 as `info.gpu` reports them — the core's device handed to EffectCraft's
//! own `Gpu::new`, and both of wgpu's failure callbacks reaching the core's notice for that device,
//! so the GPU is dropped on its first failure.

mod common;
mod scratch;
mod withheld;

use common::{EVERY_FORMAT, Scratch};
use craftbridge_core::{CraftApp, CraftError, Engine, GpuChoice, GpuDevice, GpuFailure, PixelFormat};
use craftbridge_effects::EffectCraftApp;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use withheld::assert_withheld_every_way;

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
    assert_eq!(
        info["gpu"].as_str(),
        Some("headless: renders on the CPU"),
        "with no device attached, `info.gpu` carries the session's note, as plan §17's *CPU only* state has it"
    );
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
    let outside = scratch.outside_file().expect("a file outside the container");
    let error = (request(&mut app, "exec", json!({"command": "file.open", "params": {"path": outside}}))).expect_err("the call was meant to fail");
    match error {
        CraftError::Io { message } => {
            assert!(message.starts_with(&format!("cannot read `{}`", outside.display())), "the refusal keeps the engine's words: {message}")
        }
        other => panic!("reading outside the container must be `Io`, got {other:?}"),
    }
}

#[test]
fn confinement_refuses_writes_outside_the_container() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let escape = scratch.outside().join("escape.ecproj");
    let error = (request(&mut app, "exec", json!({"command": "file.saveAs", "params": {"path": escape}}))).expect_err("the call was meant to fail");
    // The confinement's own words, path and reason: a write the disk refused is `Io` too, and opens
    // with the same `cannot write`.
    let refusal = format!("cannot write `{}`: it is outside the automation roots", escape.display());
    match error {
        CraftError::Io { message } => assert!(message.starts_with(&refusal), "the refusal keeps the confinement's words: {message}"),
        other => panic!("writing outside the container must be `Io`, got {other:?}"),
    }
    assert!(!escape.exists(), "the refused write left a file outside the container");
}

#[test]
fn the_bundle_is_read_and_never_written() {
    // The bundle is a read root and no write root: a project shipped in it opens, and nothing is
    // written into it, by name or by saving in place the project opened from it.
    let scratch = Scratch::with_bundle().expect("a scratch folder with a bundle");
    let bundle = scratch.bundle().expect("the scratch folder has a bundle").to_path_buf();
    let shipped = bundle.join("shipped.ecproj");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    request(&mut app, "exec", json!({"command": "file.saveAs", "params": {"path": scratch.path().join("made.ecproj")}})).expect("a project to ship");
    std::fs::copy(scratch.path().join("made.ecproj"), &shipped).expect("the project is shipped in the bundle");
    let shipped_bytes = std::fs::read(&shipped).expect("the shipped project reads");
    request(&mut app, "exec", json!({"command": "file.open", "params": {"path": shipped}})).expect("a project in the bundle opens");
    let copy = bundle.join("copy.ecproj");
    for (command, params) in [("file.save", json!({})), ("file.save", json!({"path": copy})), ("file.saveAs", json!({"path": copy}))] {
        match request(&mut app, "exec", json!({"command": command, "params": params})).expect_err("the bundle is never written") {
            CraftError::Io { message } => assert!(message.starts_with("cannot write `") && message.contains("it is outside the automation roots"), "{message}"),
            other => panic!("`{command}` {params} into the bundle must be the confinement's `Io`, got {other:?}"),
        }
    }
    assert_eq!(std::fs::read(&shipped).expect("the shipped project reads"), shipped_bytes, "the shipped project is as it was");
    assert!(!copy.exists(), "nothing new is written into the bundle");
}

#[test]
fn confinement_reads_inside_the_container_are_allowed() {
    // The same call the refusal test makes, aimed inside the container: refused for being no
    // project, never by the confinement, which is what shows the roots hold and do not over-refuse.
    // The file is there and reads, so the one refusal left is the engine's own of what it holds; a
    // file the disk would not read is `Io` like the confinement's refusal, and would hide one.
    let scratch = Scratch::new().expect("a scratch folder");
    let inside = scratch.path().join("nothing.ecproj");
    std::fs::write(&inside, "no project").expect("a file inside the container");
    assert!(scratch.root().join("container").is_dir(), "the container sits in the scratch folder it was prepared from");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let error = (request(&mut app, "exec", json!({"command": "file.open", "params": {"path": inside}}))).expect_err("the call was meant to fail");
    match error {
        CraftError::CommandFailed { command, message, .. } => {
            assert_eq!(command, "file.open");
            assert!(message.contains(" is not a project EffectCraft can open: "), "the engine read the file: {message}");
        }
        other => panic!("an access inside the container is not refused by the confinement: {other:?}"),
    }
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
    let outside = scratch.outside_file().expect("a file outside the container");
    let refused =
        (request(&mut app, "run", json!({"steps": [{"command": "file.open", "params": {"path": outside}}]}))).expect_err("the call was meant to fail");
    match refused {
        CraftError::Io { message } => {
            assert!(
                message.starts_with(&format!("file.open: cannot read `{}`", outside.display())) && message.ends_with(" (step 0)"),
                "the step is named: {message}"
            )
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
    // `Unsupported` row. Nothing runs either way: the refusal is the plug-in's, before the engine
    // sees the call.
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let weights = json!({"id": "classical"});
    for command in ["roto.model.download", "face.model.download"] {
        let words = format!("`{command}` is not available in this build: it downloads by starting the system `curl`, and this build starts no process");
        assert_withheld_every_way(&mut app, command, &weights, &words).expect("every way a call reaches the download is refused");
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

/// The project both timeline-export rules are checked on: one composition holding one text layer,
/// which is a layer a rendering Export Timeline would pre-render to a movie.
fn text_layer_project() -> craftbridge_core::Result<(Scratch, EffectCraftApp)> {
    let scratch = Scratch::new()?;
    let mut app = scratch.app()?;
    for (command, params) in [
        ("comp.new", json!({"name": "Card", "width": 32, "height": 24, "duration": 0.5, "frameRate": 10})),
        ("layer.newText", json!({"comp": 1, "text": "Hi"})),
    ] {
        request(&mut app, "exec", json!({"command": command, "params": params})).map_err(|error| CraftError::internal(format!("{command}: {error}")))?;
    }
    Ok((scratch, app))
}

#[test]
fn a_render_nothing_has_listed_is_refused_however_a_call_reaches_it() {
    // The pending-export journal learns a render's files only from `export`, which lists them before
    // the render starts; a render started any other way would leave files a restart cannot delete.
    // The commands that have the exporter write one are withheld on every way a call reaches them,
    // each with parameters it would render with, and `export` is the way that renders.
    let (scratch, mut app) = text_layer_project().expect("a project with one text layer");
    let at = |name: &str| scratch.path().join(name).display().to_string();
    // A queued item: what a render the runner has not listed would render.
    request(&mut app, "exec", json!({"command": "renderQueue.add", "params": {"comp": 1, "format": "h264", "output": at("queued.mp4")}}))
        .expect("an item queues");
    let before = std::fs::read_dir(scratch.path()).expect("the container reads").count();
    for (command, params) in [
        ("renderQueue.render", json!({})),
        ("file.watchFolder", json!({"folder": scratch.path()})),
        ("file.watchFolder.poll", json!({"folder": scratch.path()})),
        ("render.saveCurrentPreview", json!({"comp": 1, "path": at("preview.mp4")})),
        ("file.exportTimeline", json!({"comp": 1, "path": at("cut.xml")})),
    ] {
        let words = format!(
            "`{command}` is not available in this build: it starts a render whose files the pending-export journal cannot list, and `export` lists them"
        );
        assert_withheld_every_way(&mut app, command, &params, &words).expect("every way a call reaches the render is refused");
        // Behind a step the door reads past, an Export Timeline that renders nothing, the batch is
        // still refused for this later step, and the quiet step does not run either.
        let quiet = json!({"command": "file.exportTimeline", "params": {"comp": 1, "path": at("quiet.xml"), "prerender": "none"}});
        let batched = json!({"command": "engine.batch", "params": {"steps": [quiet, {"command": command, "params": params}]}});
        match request(&mut app, "exec", batched) {
            Err(CraftError::Unsupported { message }) => assert_eq!(message, words, "the later step is the one refused"),
            other => panic!("`{command}` after a quiet step must be `Unsupported`, got {other:?}"),
        }
    }
    // Withheld before the engine saw it: nothing rendered or was written.
    let queue = request(&mut app, "exec", json!({"command": "renderQueue.list"})).expect("the queue lists");
    assert_eq!(queue["items"][0]["status"], json!("Queued"), "{queue}");
    assert_eq!(queue["rendering"], json!(false), "{queue}");
    assert_eq!(std::fs::read_dir(scratch.path()).expect("the container reads").count(), before, "a withheld render wrote nothing");
}

/// The door's one exception, pinned from both sides. An Export Timeline whose own `prerender` reads
/// as the mode that renders nothing answers in every way a call reaches it — on its own, as a step
/// of `run`, and as a step of an engine batch by either spelling — and pre-renders nothing, exactly
/// as the pin reads the parameter: the bool `false`, and `none`, `off`, `false` and `no` however
/// normalized. Every Export Timeline that can render — no `prerender`, a rendering spelling, a
/// `prerender` an `engine.batch` only fills in as it runs — is refused before the engine sees the
/// call, and nothing renders: no movie appears beside the export folder.
#[test]
fn a_timeline_export_that_renders_nothing_answers_and_every_other_timeline_export_is_refused() {
    let (scratch, mut app) = text_layer_project().expect("a project with one text layer");
    let at = |name: &str| scratch.path().join(name).display().to_string();
    let pre_rendered = |answer: &Value| match answer.get("prerendered").and_then(Value::as_array) {
        Some(list) => !list.is_empty(),
        None => panic!("the answer says nothing about pre-renders: {answer}"),
    };
    for (index, spelling) in [json!("none"), json!("NONE"), json!(" Off "), json!("no"), json!("false"), json!(false)].into_iter().enumerate() {
        let call = |name: &str| json!({"comp": 1, "path": at(name), "prerender": spelling});
        // On its own: answered, and its answer says nothing was pre-rendered.
        let answer = request(&mut app, "exec", json!({"command": "file.exportTimeline", "params": call(&format!("own-{index}.xml"))}))
            .expect("a call that renders nothing is answered");
        assert!(!pre_rendered(&answer), "nothing was pre-rendered: {answer}");
        // As a step of `run`, and as a step of an engine batch in both spellings.
        let stepped = request(&mut app, "run", json!({"steps": [{"command": "file.exportTimeline", "params": call(&format!("run-{index}.xml"))}]}))
            .expect("a step that renders nothing is answered");
        assert!(!pre_rendered(&stepped[0]["result"]), "the step rendered: {stepped}");
        for spelling_of in ["command", "id"] {
            let step = json!({spelling_of: "file.exportTimeline", "params": call(&format!("batch-{spelling_of}-{index}.xml"))});
            let answer = request(&mut app, "exec", json!({"command": "engine.batch", "params": {"steps": [step]}}))
                .expect("a batch step that renders nothing is answered");
            assert!(!pre_rendered(&answer["results"][0]), "the batch step rendered: {answer}");
        }
    }
    for file in std::fs::read_dir(scratch.path()).expect("the container reads") {
        let path = file.expect("an entry reads").path();
        assert_ne!(path.extension().unwrap_or_default(), "mov", "a no-render Export Timeline left a movie: {}", path.display());
    }
    // Every other Export Timeline can render — this project holds a layer no timeline file can
    // carry — so the door refuses it on every way a call reaches it, and a `$N` is not written
    // until the batch runs it, which is after the door has read.
    let words =
        "`file.exportTimeline` is not available in this build: it starts a render whose files the pending-export journal cannot list, and `export` lists them";
    for extra in [json!({}), json!({"prerender": "all"}), json!({"prerender": "unsupported"}), json!({"prerender": true}), json!({"prerender": 5})] {
        let mut params = json!({"comp": 1, "path": at("may-render.xml")});
        params.as_object_mut().expect("an object of parameters").extend(extra.as_object().expect("more parameters").clone());
        assert_withheld_every_way(&mut app, "file.exportTimeline", &params, words).expect("a call that may render is refused every way");
    }
    match request(
        &mut app,
        "exec",
        json!({"command": "engine.batch", "params": {"steps": [
            {"command": "comp.new", "params": {"name": "Later"}},
            {"command": "file.exportTimeline", "params": {"comp": 1, "path": at("filled.xml"), "prerender": "$0.prerender"}},
        ]}}),
    ) {
        Err(CraftError::Unsupported { message }) => assert!(message.starts_with("`file.exportTimeline` is not available"), "{message}"),
        other => panic!("a `prerender` a batch fills in as it runs is not the door's to read, got {other:?}"),
    }
    assert!(!std::path::Path::new(&at("filled.xml")).exists(), "the refused batch step ran nothing");
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
    // The four names §14's table carries that `request` does not answer, and two names the table
    // carries not at all, `script` (PD13: an app composes the engine's methods instead) and one no
    // method is: every name answers `Unsupported` in the one refusal, whose exact words are pinned
    // so they claim only what is true however the method was asked — that `request` lacks it, and
    // which methods `request` answers.
    for method in ["frame", "export", "template.render", "script", "mcp", "no.such.method"] {
        let error = (request(&mut app, method, json!({}))).expect_err("the call was meant to fail");
        match error {
            CraftError::Unsupported { message } => assert_eq!(
                message,
                format!(
                    "`{method}` is not answered by `request` in this build, which answers `info`, `commands`, `exec`, `run`, `props`, `get`, `set`, `open`, `save`, `saveAs`, `new`, `replay`, `timings` and `template.open`"
                ),
                "the refusal names the method and what `request` answers"
            ),
            other => panic!("an unwired method must be `Unsupported`, got {other:?}"),
        }
    }
}

#[test]
fn a_call_with_progress_answers_its_methods_and_refuses_every_other_name() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    // Every name but `export` and `template.render`, the `request` methods included, is refused in the one refusal that
    // names what the call answers.
    for method in ["info", "exec", "template.open", "frame", "script", "no.such.method"] {
        let error = app.request_with_progress(method, json!({}), &mut |_, _, _| true, &mut |_| Ok(())).expect_err("the call was meant to fail");
        match error {
            CraftError::Unsupported { message } => assert_eq!(
                message,
                format!("`{method}` is not answered by `request_with_progress` in this build, which answers `export` and `template.render`"),
                "the refusal names the method and what the call answers"
            ),
            other => panic!("a name the call does not answer must be `Unsupported`, got {other:?}"),
        }
    }
    // `export` reads its parameters like every method: an unknown field and a missing output are the caller's to fix.
    for params in [json!({"output": "/x", "widht": 1}), json!({})] {
        let error = app.request_with_progress("export", params, &mut |_, _, _| true, &mut |_| Ok(())).expect_err("the call was meant to fail");
        assert!(matches!(error, CraftError::InvalidParams { .. }), "{error:?}");
    }
    // An output outside the container is the confinement's to refuse, before anything renders.
    app.request("exec", json!({"command": "comp.new", "params": {"name": "A", "width": 16, "height": 16, "duration": 0.2, "frameRate": 10}})).expect("a comp");
    let error =
        app.request_with_progress("export", json!({"output": "/tmp/outside.mp4"}), &mut |_, _, _| true, &mut |_| Ok(())).expect_err("outside the container");
    assert!(matches!(error, CraftError::Io { .. }), "{error:?}");
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
fn the_property_and_file_methods_check_their_own_parameters() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    app.request("exec", json!({"command": "comp.new", "params": {"name": "A"}})).expect("a comp to ask about");
    // Each is the caller's to fix: a key the method does not take, a value of the wrong kind, or
    // parameters that are no object, refused by the method itself before any tool or command is
    // asked, in one sentence naming the method and, where there is one, the key.
    let (not_an_object, unknown, not_a_string, not_a_number) =
        ("its parameters must be a JSON object", "unknown parameter(s) `", "must be a string", "must be a number");
    for (method, params, sentence) in [
        ("props", json!({"comp": 1, "layer": 1, "bogus": 1}), format!("`props`: {unknown}bogus`")),
        ("props", json!({"comp": 1, "layer": 1, "flat": "yes"}), "`props`: `flat` must be a bool".to_string()),
        ("props", json!({"comp": 1, "layer": 1, "time": "now"}), format!("`props`: `time` {not_a_number}")),
        ("props", json!({"comp": 1, "layer": 1, "depth": "deep"}), "`props`: `depth` must be a whole number".to_string()),
        // The tool ignores a depth that is no whole number, so it is refused rather than ignored.
        ("props", json!({"comp": 1, "layer": 1, "depth": 1.5}), "`props`: `depth` must be a whole number".to_string()),
        ("props", json!({"comp": 1, "layer": 1, "depth": -1}), "`props`: `depth` must be a whole number".to_string()),
        ("props", json!([1, 2]), format!("`props`: {not_an_object}")),
        ("get", json!({"comp": 1, "layer": 1, "path": 5}), format!("`get`: `path` {not_a_string}")),
        ("get", json!({"comp": 1, "layer": 1, "path": "transform/opacity", "time": "now"}), format!("`get`: `time` {not_a_number}")),
        ("get", json!({"comp": 1, "layer": 1, "path": "transform/opacity", "value": 5}), format!("`get`: {unknown}value`")),
        ("set", json!({"comp": 1, "layer": 1, "path": "transform/opacity", "expression": 5}), format!("`set`: `expression` {not_a_string}")),
        ("set", json!({"comp": 1, "layer": 1, "path": "transform/opacity", "value": 5, "timeBase": "comp"}), format!("`set`: {unknown}timeBase`")),
        ("open", json!({"path": 5}), format!("`open`: `path` {not_a_string}")),
        ("open", json!({"file": "a.ecproj"}), format!("`open`: {unknown}file`")),
        ("save", json!({"path": ["a"]}), format!("`save`: `path` {not_a_string}")),
        ("saveAs", json!("a.ecproj"), format!("`saveAs`: {not_an_object}")),
        ("new", json!({"path": "a.ecproj"}), format!("`new`: {unknown}path`")),
    ] {
        let error = request(&mut app, method, params.clone()).expect_err("the call was meant to fail");
        match error {
            CraftError::InvalidParams { message } => assert_eq!(message, sentence, "{method} {params}"),
            other => panic!("`{method}` {params} must be `InvalidParams`, got {other:?}"),
        }
    }
}

#[test]
fn a_comp_or_layer_that_names_nothing_is_refused_in_one_sentence() {
    // A value that is no id, no name and not `-` names nothing the command line could read: the
    // refusal says what a comp or layer may be, in the same words for each method and key.
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    app.request("exec", json!({"command": "comp.new", "params": {"name": "A"}})).expect("a comp to ask about");
    for (method, key, params) in [
        ("props", "comp", json!({"comp": true, "layer": 1})),
        ("props", "layer", json!({"comp": 1, "layer": 1.5})),
        ("props", "layer", json!({"comp": 1, "layer": -1})),
        ("get", "comp", json!({"comp": {"id": 1}, "layer": 1, "path": "transform/opacity"})),
        ("set", "layer", json!({"comp": 1, "layer": [1], "path": "transform/opacity", "value": 5})),
    ] {
        match request(&mut app, method, params.clone()).expect_err("the call was meant to fail") {
            CraftError::InvalidParams { message } => assert_eq!(message, format!("`{method}`: `{key}` must be an id, a name or `-`"), "{params}"),
            other => panic!("`{method}` {params} must be `InvalidParams`, got {other:?}"),
        }
    }
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
    let engine = Engine::start(app, GpuChoice::CpuOnly).expect("the engine starts with the app");
    let info = engine.request("info", json!({})).expect("info answers on the engine thread");
    assert_eq!(info["engine"].as_str(), Some("effectcraft"));
    let unsupported = engine.request("frame", json!({})).expect_err("a name `request` does not answer");
    assert!(matches!(unsupported, CraftError::Unsupported { .. }));
    // The frame is answered through its own call, which reaches the same session.
    engine.request("run", json!({"steps": [{"command": "comp.new", "params": {"name": "C", "width": 8, "height": 6}}]})).expect("a comp is made");
    let frame = engine.frame(json!({}), PixelFormat::Rgba8Straight).expect("the frame renders on the engine thread");
    assert_eq!((frame.width, frame.height, frame.data.len()), (8, 6, 8 * 6 * 4));
    engine.close().expect("the engine closes clean");
}

#[test]
fn a_frame_request_is_checked_before_anything_renders() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    app.request("exec", json!({"command": "comp.new", "params": {"name": "C", "width": 8, "height": 6}})).expect("a comp is made");
    // Each of these names no one frame, or no frame at all, whichever format is asked for, and the
    // refusal says what to give instead: a comp is an id, a name or `-`, whatever else it was given as.
    let not_a_comp = "`frame`: `comp` must be an id, a name or `-`";
    for (params, words) in [
        (json!({"time": 1, "frame": 2}), "`frame`: give `time` or `frame`, not both"),
        (json!({"maxSide": 4, "scale": 0.5}), "`frame`: give `maxSide` or `scale`, not both"),
        (json!({"extra": 1}), "`frame`: unknown parameter(s) `extra`"),
        (json!({"time": "1"}), "`frame`: `time` must be a number"),
        (json!({"frame": true}), "`frame`: `frame` must be a number"),
        (json!({"maxSide": "4"}), "`frame`: `maxSide` must be a number"),
        (json!({"transparent": 1}), "`frame`: `transparent` must be a bool"),
        (json!({"comp": 1.5}), not_a_comp),
        (json!({"comp": -1}), not_a_comp),
        (json!({"comp": [1]}), not_a_comp),
        (json!({"comp": true}), not_a_comp),
        (json!({"comp": {"id": 1}}), not_a_comp),
        (json!([1]), "`frame`: its parameters must be a JSON object"),
    ] {
        for format in EVERY_FORMAT {
            match app.frame(params.clone(), format).expect_err("the request is refused") {
                CraftError::InvalidParams { message } => assert_eq!(message, words, "{params} as {format:?}"),
                other => panic!("{params} as {format:?} is `InvalidParams`, got {other:?}"),
            }
        }
    }
    // Absent parameters are the defaults, and null fields count as absent.
    let frame = app.frame(Value::Null, PixelFormat::Rgba8Straight).expect("no parameters is the active comp");
    assert_eq!((frame.width, frame.height), (8, 6));
    let frame = app.frame(json!({"comp": null, "time": null, "maxSide": null}), PixelFormat::Rgba8Straight).expect("null fields are absent");
    assert_eq!((frame.width, frame.height), (8, 6));
}

#[test]
fn a_frame_is_never_answered_by_request() {
    // `frame` has no row of the method table, so `request` refuses the name even when the frame itself
    // renders: the refusal names what `request` answers.
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    app.request("exec", json!({"command": "comp.new", "params": {"name": "C", "width": 8, "height": 6}})).expect("a comp is made");
    assert!(app.frame(json!({}), PixelFormat::Png).is_ok());
    match request(&mut app, "frame", json!({})).expect_err("`request` does not answer `frame`") {
        CraftError::Unsupported { message } => assert!(
            message
                .ends_with("which answers `info`, `commands`, `exec`, `run`, `props`, `get`, `set`, `open`, `save`, `saveAs`, `new`, `replay`, `timings` and `template.open`"),
            "{message}"
        ),
        other => panic!("`frame` through `request` must be `Unsupported`, got {other:?}"),
    }
}

/// What `info.gpu` answers right now, in one string for the assertions.
fn gpu_of(app: &mut EffectCraftApp) -> Result<String, String> {
    let info = app.request("info", json!({})).map_err(|e| format!("`info` fails: {e}"))?;
    info.pointer("/gpu").and_then(Value::as_str).map(str::to_string).ok_or("the `info` answer carries no `gpu` string".to_string())
}

/// The desktop device, attached, and `info` already naming it as `"<adapter> (<backend>)"`, with the
/// device itself for the test to act on.
fn attach_the_device(app: &mut EffectCraftApp) -> Result<(GpuDevice, wgpu::Device), String> {
    let gpu = GpuDevice::request(GpuChoice::Preferred);
    let GpuDevice::Device { device, .. } = &gpu else {
        return Err(format!("a desktop must answer a device for this test to mean anything: {}", gpu.summary()));
    };
    let device = device.clone();
    app.attach_gpu(&gpu).map_err(|e| format!("the desktop device must attach: {e}"))?;
    let name = gpu_of(app)?;
    if name != gpu.summary() {
        return Err(format!("the attached device must be named as `\"<adapter> (<backend>)\"` (plan §17); `info` says {name}"));
    }
    Ok((gpu, device))
}

/// The §17 states in order: the construction note, then the reason a `CpuOnly` choice gives, then
/// the adapter's name once a device is attached, then the note a deliberate detach leaves.
#[test]
fn the_states_of_the_gpu_read_through_info() {
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    assert_eq!(gpu_of(&mut app).expect("`info` answers"), "headless: renders on the CPU", "the session's own note before any device");

    let cpu = GpuDevice::request(GpuChoice::CpuOnly);
    app.attach_gpu(&cpu).expect("attaching a CPU-only answer is not a failure");
    assert_eq!(gpu_of(&mut app).expect("`info` answers"), cpu.summary(), "the reason of the `CpuOnly` choice is what `info.gpu` reports (plan §17)");

    attach_the_device(&mut app).expect("the desktop device attaches and is named");
    app.drop_gpu();
    assert_eq!(gpu_of(&mut app).expect("`info` answers"), "the GPU was detached; renders run on the CPU", "the detach says so through `info.gpu`");
}

/// What an engine's app saw of the GPU, kept by [`Spy`] for the test to read and to act on: the
/// calls in the order the app made them, and for each device it was handed, its handles and notice.
#[derive(Default)]
struct Seen {
    events: Mutex<Vec<String>>,
    devices: Mutex<Vec<wgpu::Device>>,
    queues: Mutex<Vec<wgpu::Queue>>,
    notices: Mutex<Vec<GpuFailure>>,
}

/// The list behind `list`, read through a poisoned lock too: a panic in the engine thread must not
/// hide what the app had seen.
fn held<T>(list: &Mutex<Vec<T>>) -> std::sync::MutexGuard<'_, Vec<T>> {
    list.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The EffectCraft app as an engine drives it, with a record of every call and of each device it is
/// handed, since the engine makes the device itself and a test has no other way to reach it.
struct Spy {
    inner: EffectCraftApp,
    seen: Arc<Seen>,
}

impl Spy {
    fn note(&self, event: impl Into<String>) {
        held(&self.seen.events).push(event.into());
    }
}

impl CraftApp for Spy {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, CraftError> {
        self.note(format!("call {method}"));
        self.inner.request(method, params)
    }

    fn frame(&mut self, params: Value, format: PixelFormat) -> Result<craftbridge_core::FrameBytes, CraftError> {
        self.note("call frame");
        self.inner.frame(params, format)
    }

    fn attach_gpu(&mut self, gpu: &GpuDevice) -> Result<(), CraftError> {
        self.note("attach_gpu");
        if let Some((_, device, queue)) = gpu.parts() {
            held(&self.seen.devices).push(device.clone());
            held(&self.seen.queues).push(queue.clone());
        }
        held(&self.seen.notices).push(gpu.device_failure_notifier());
        self.inner.attach_gpu(gpu)
    }

    fn drop_gpu(&mut self) {
        self.note("drop_gpu");
        self.inner.drop_gpu();
    }
}

/// An engine on a scratch session that has one 8 by 6 comp with a blurred solid, which the
/// compositor draws on the GPU, started as `choice` asks through the start an app's constructor
/// makes, with what its app sees.
fn engine_on(choice: GpuChoice) -> Result<(Scratch, Engine, Arc<Seen>), String> {
    let scratch = Scratch::new().map_err(|e| e.to_string())?;
    let seen = Arc::new(Seen::default());
    let app = Spy { inner: scratch.app().map_err(|e| e.to_string())?, seen: seen.clone() };
    let engine = Engine::start_in(&scratch.folders, app, choice).map_err(|e| e.to_string())?;
    let steps = json!([
        {"command": "comp.new", "params": {"name": "C", "width": 8, "height": 6}},
        {"command": "layer.newSolid", "params": {"comp": 1, "name": "s"}},
        {"command": "effect.apply", "params": {"comp": 1, "layer": "s", "effect": "Gaussian Blur"}}
    ]);
    engine.request("run", json!({ "steps": steps })).map_err(|e| format!("a blurred solid is made: {e}"))?;
    Ok((scratch, engine, seen))
}

/// What `info.gpu` answers right now, in one string for the assertions.
fn gpu_of_engine(engine: &Engine) -> Result<String, String> {
    let info = engine.request("info", json!({})).map_err(|e| format!("`info` fails: {e}"))?;
    info.pointer("/gpu").and_then(Value::as_str).map(str::to_string).ok_or("the `info` answer carries no `gpu` string".to_string())
}

/// How many batches of work the device had been given when this one was handed to it. wgpu counts
/// the submissions of a device itself and keeps the count private, but a submission's index prints
/// in its `Debug` text, so a test asks for it by handing the queue an empty batch: the difference of
/// two answers, less the one asked for, is every other batch submitted between them.
fn submissions(queue: &wgpu::Queue) -> Result<u64, String> {
    let text = format!("{:?}", queue.submit([]));
    text.chars().filter(char::is_ascii_digit).collect::<String>().parse().map_err(|e| format!("no submission index in `{text}`: {e}"))
}

/// The GPU's work between two probes of `queue`, by the device's own count.
fn submitted_during(queue: &wgpu::Queue, work: impl FnOnce() -> Result<(), String>) -> Result<u64, String> {
    let before = submissions(queue)?;
    work()?;
    Ok(submissions(queue)? - before - 1)
}

fn render(engine: &Engine) -> Result<(), String> {
    let frame = engine.frame(json!({}), PixelFormat::Rgba8Straight).map_err(|e| format!("the frame renders: {e}"))?;
    if (frame.width, frame.height) == (8, 6) { Ok(()) } else { Err(format!("a frame of {}x{}", frame.width, frame.height)) }
}

/// The GPU's work, by the device's own count on `queue`, over four renders of frames the compositor
/// has not drawn before: the solid's blur is set to a new radius, from `radius` on, before each. A
/// frame drawn again can come from the compositor's caches with no GPU work on the GPU too, and the
/// work of one new frame reaches the device over more than one call, so one render alone can count
/// none where four on the GPU always count some.
fn submitted_rendering_new_frames(engine: &Engine, queue: &wgpu::Queue, radius: u32) -> Result<u64, String> {
    submitted_during(queue, || {
        (radius..radius + 4).try_for_each(|radius| {
            let blur = json!({"comp": 1, "layer": "s", "path": "effects/#1/blurriness", "value": radius});
            engine.request("set", blur).map_err(|e| format!("the blur is set: {e}"))?;
            render(engine)
        })
    })
}

/// An engine started with a GPU attaches it, and `info.gpu` names the adapter as the compositor does
/// (§17); one that asks for none is handed the reason and keeps it.
#[test]
fn an_engine_started_with_a_gpu_attaches_it_and_info_names_it() {
    let (_scratch, engine, seen) = engine_on(GpuChoice::Preferred).expect("the engine starts");
    let named = GpuDevice::request(GpuChoice::Preferred).summary();
    assert_eq!(gpu_of_engine(&engine).expect("`info` answers"), named, "the attached device is named as `<adapter> (<backend>)`");
    assert_eq!(held(&seen.devices).len(), 1, "the engine handed the app one device");

    let (_scratch, engine, seen) = engine_on(GpuChoice::CpuOnly).expect("the engine starts");
    assert_eq!(gpu_of_engine(&engine).expect("`info` answers"), "the app asked for CPU-only rendering");
    assert!(held(&seen.devices).is_empty(), "a CpuOnly engine is handed no device");
}

/// Both of wgpu's failure callbacks reach the notice of the engine's device, in EffectCraft's words,
/// and the engine drops the GPU before the next call, which `info.gpu` carries as EffectCraft's
/// desktop app notes a retired device, and every render after it answers on the CPU (§17, L7).
#[test]
fn a_failure_drops_the_gpu_before_the_engines_next_call_and_renders_on_the_cpu() {
    let causes: [(&str, &str, fn(&wgpu::Device)); 2] = [
        ("a lost device", "GPU device lost (Destroyed): ", |device| {
            device.destroy();
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
        }),
        ("an uncaptured error", "uncaptured GPU error: ", |device| {
            let oversized = wgpu::BufferDescriptor {
                label: Some("craftbridge uncaptured-error probe"),
                size: device.limits().max_buffer_size + 4096,
                usage: wgpu::BufferUsages::UNIFORM,
                mapped_at_creation: false,
            };
            let _ = device.create_buffer(&oversized);
        }),
    ];
    for (name, words, cause) in causes {
        let (_scratch, engine, seen) = engine_on(GpuChoice::Preferred).expect("the engine starts");
        render(&engine).expect("the GPU renders");
        let device = held(&seen.devices).first().cloned().expect("the app was handed a device");
        cause(&device);
        let reason = held(&seen.notices).first().and_then(GpuFailure::reason).unwrap_or_else(|| panic!("{name}: the notice has no report"));
        assert!(reason.starts_with(words), "{name}: the notice keeps the failure in EffectCraft's words: {reason}");
        assert_eq!(gpu_of_engine(&engine).expect("`info` answers after the failure"), format!("GPU preview retired: {reason}"), "{name}");
        render(&engine).expect("the CPU renders after the failure");
        let events = held(&seen.events).clone();
        let dropped = events.iter().position(|event| event == "drop_gpu").unwrap_or_else(|| panic!("{name}: the GPU was never dropped: {events:?}"));
        assert_eq!(events.iter().filter(|event| *event == "drop_gpu").count(), 1, "{name}: the GPU is dropped once");
        assert_eq!(events.get(dropped + 1).map(String::as_str), Some("call info"), "{name}: the drop comes before the next call: {events:?}");
    }
}

/// A suspended engine submits no work to the device it had, by the device's own count, and renders
/// on the CPU; a resumed one holds a device again, named in `info.gpu`, and renders on it. Every
/// count is of new frames, so no cache answers a render that would have run on the GPU.
#[test]
fn a_suspended_engine_submits_no_gpu_work_and_a_resumed_one_holds_a_device_again() {
    let (_scratch, engine, seen) = engine_on(GpuChoice::Preferred).expect("the engine starts");
    let first = held(&seen.queues).first().cloned().expect("the app was handed a device");
    let on_the_gpu = submitted_rendering_new_frames(&engine, &first, 1).expect("the GPU renders are counted");
    assert!(on_the_gpu > 0, "the counter sees GPU renders, so a count of none means something");

    engine.suspend_gpu().expect("the engine suspends");
    let idle = submitted_rendering_new_frames(&engine, &first, 10).expect("the renders are counted");
    assert_eq!(idle, 0, "no GPU work is submitted while suspended");
    assert_eq!(gpu_of_engine(&engine).expect("`info` answers"), "the GPU was detached; renders run on the CPU");

    assert_eq!(engine.resume_gpu(), Ok(None), "a resume that got a device answers no CPU note");
    assert_eq!(gpu_of_engine(&engine).expect("`info` answers"), GpuDevice::request(GpuChoice::Preferred).summary(), "the app holds a device again");
    let second = held(&seen.queues).get(1).cloned().expect("the engine handed the app a second device");
    assert!(submitted_rendering_new_frames(&engine, &second, 20).expect("the renders are counted") > 0, "the resumed device renders");
}

/// A resume that finds no device hands the app the CPU note: an engine that asked for none, resumed,
/// is handed the reason again and never a device.
#[test]
fn a_resume_with_no_device_answers_the_cpu_note() {
    let (_scratch, engine, seen) = engine_on(GpuChoice::CpuOnly).expect("the engine starts");
    engine.suspend_gpu().expect("the engine suspends");
    let note = "the app asked for CPU-only rendering".to_string();
    assert_eq!(engine.resume_gpu(), Ok(Some(note.clone())), "no device is no failure, and the caller hears why");
    assert_eq!(gpu_of_engine(&engine).expect("`info` answers"), note);
    assert!(held(&seen.devices).is_empty(), "no device ever came");
    render(&engine).expect("the CPU renders");
}
