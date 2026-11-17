//! The acceptance of plan §28 T17 as a property of the plugin: a template opened and rendered with
//! values answers what the command line's `--template` answers on the same inputs, the frame byte
//! for byte and the export file for file.
//!
//! D-12 asks that an encoded export match the command line's in frame count, size and, frame by
//! frame, within 40 dB. The export is held to the stronger relation the export tests hold every
//! output to: the same files with the same bytes. The template is the plan's running example in
//! small, a Name text control and a Photo media control, with a Colour and an Opacity control beside
//! them, so a value of every kind `essential.set` takes is applied: text, media, a colour and a
//! number.

use super::common::Scratch;
use super::export::{QUEUED_HARDWARE, entries_under, same_files};
use super::{cli, scrub, scrubbed};
use craftbridge_core::{CancelToken, CraftApp, CraftError, Engine, GpuChoice, PixelFormat};
use craftbridge_effects::EffectCraftApp;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The template's file in the container, and the picture to put in its Photo control's place.
pub(crate) const FILE: &str = "card.ectemplate";
pub(crate) const REPLACEMENT: &str = "sam.png";

/// The method a template is exported through.
pub(crate) const RENDER: &str = "template.render";

/// A template long enough that a render of it, one image to a frame, is far from over when the first
/// file appears, and the files it writes in the end.
pub(crate) const SIZE: (u32, u32) = (320, 180);
pub(crate) const SECONDS: f64 = 7.0;
pub(crate) const FRAMES: usize = 210;

/// The font the fixture's Name text names and no machine has.
const LOST_FONT: &str = "Nonexistent Face Qq";

/// The values of the running example: the card's name, and a photo by path.
fn sam(photo: &Path) -> Value {
    json!({"Name": "Sam", "Photo": photo.display().to_string()})
}

/// A template made with the plugin's own commands inside the container: a `width` x `height`
/// composition of `seconds` seconds at 30 frames a second, a Name text control named in `font`, a
/// Photo media control over a small red picture, a Colour control on the background's Fill and an
/// Opacity control on the name. Answers the template's path and a green picture to put in its place.
pub(crate) fn make(scratch: &Scratch, size: (u32, u32), seconds: f64, font: &str) -> Result<(PathBuf, PathBuf), String> {
    make_over(scratch, size, seconds, font, [200, 30, 30])
}

/// [`make`], with the template's own Photo picture of this colour.
fn make_over(scratch: &Scratch, size: (u32, u32), seconds: f64, font: &str, picture: [u8; 3]) -> Result<(PathBuf, PathBuf), String> {
    let (photo, replacement) = (scratch.path().join("photo.png"), scratch.path().join(REPLACEMENT));
    for (file, colour) in [(&photo, picture), (&replacement, [30, 200, 30])] {
        image::RgbImage::from_pixel(32, 32, image::Rgb(colour)).save(file).map_err(|e| format!("the picture is written: {e}"))?;
    }
    let template = scratch.path().join(FILE);
    let mut app = scratch.app().map_err(|e| e.to_string())?;
    let mut exec = |command: &str, params: Value| app.request("exec", json!({"command": command, "params": params})).map_err(|e| format!("{command}: {e}"));
    exec("comp.new", json!({"name": "Card", "width": size.0, "height": size.1, "duration": seconds, "frameRate": 30}))?;
    exec("layer.newSolid", json!({"comp": 1, "name": "Bg", "color": "#223344"}))?;
    exec("effect.apply", json!({"layers": ["Bg"], "effect": "fill"}))?;
    exec("layer.newText", json!({"comp": 1, "text": "Hello", "name": "Name", "font": font}))?;
    let imported = exec("file.import", json!({"paths": [photo.display().to_string()]}))?;
    let item = imported.pointer("/items/0").cloned().ok_or("the picture imports as an item")?;
    let added = exec("layer.addItem", json!({"comp": 1, "item": item}))?;
    let layer = added.get("layer").cloned().ok_or("the picture is added as a layer")?;
    for (time, at) in [(0, [20, 20]), (1, [70, 40])] {
        exec("prop.addKey", json!({"comp": 1, "layer": layer, "path": "transform/position", "time": time, "value": at}))?;
    }
    exec("essential.addProperty", json!({"comp": 1, "layer": "Name", "path": "text/sourceText", "name": "Name"}))?;
    exec("essential.addMedia", json!({"layer": layer, "name": "Photo"}))?;
    exec("essential.addProperty", json!({"comp": 1, "layer": "Bg", "path": "effects/#1/color", "name": "Colour"}))?;
    exec("essential.addProperty", json!({"comp": 1, "layer": "Name", "path": "transform/opacity", "name": "Opacity"}))?;
    exec("essential.exportTemplate", json!({"comp": 1, "path": template.display().to_string()}))?;
    Ok((template, replacement))
}

/// The command line's `render-frame --template` with these values and flags: the PNG it wrote.
fn cli_frame(scratch: &Scratch, template: &Path, values: Option<&Value>, flags: &[&str]) -> Result<Vec<u8>, String> {
    let out = scratch.root().join("cli-frame.png");
    let mut args =
        vec!["render-frame".to_string(), "--json".into(), "--template".into(), template.display().to_string(), "--out".into(), out.display().to_string()];
    args.extend(values_flag(scratch, values)?);
    args.extend(flags.iter().map(|flag| flag.to_string()));
    cli(scratch.root(), &args.iter().map(String::as_str).collect::<Vec<_>>())?.doc()?;
    std::fs::read(&out).map_err(|e| format!("the frame the command line wrote reads: {e}"))
}

/// `--values FILE` for these values, the file written beside the container.
fn values_flag(scratch: &Scratch, values: Option<&Value>) -> Result<Vec<String>, String> {
    let Some(values) = values else { return Ok(Vec::new()) };
    let file = scratch.root().join("values.json");
    std::fs::write(&file, values.to_string()).map_err(|e| format!("the values are written: {e}"))?;
    Ok(vec!["--values".into(), file.display().to_string()])
}

/// The plugin's frame of the template with these values and frame request.
fn plugin_frame(scratch: &Scratch, template: &Path, values: Option<&Value>, frame: Value) -> Result<Vec<u8>, CraftError> {
    let mut app = scratch.app()?;
    app.frame(call(template, values, "frame", frame), PixelFormat::Png).map(|frame| frame.data)
}

/// The parameters of a template call: the template, its values and the request under `key`.
fn call(template: &Path, values: Option<&Value>, key: &str, request: Value) -> Value {
    let mut params = json!({"template": template.display().to_string(), key: request});
    if let (Some(values), Some(fields)) = (values, params.as_object_mut()) {
        fields.insert("values".into(), values.clone());
    }
    params
}

#[test]
fn a_template_frame_is_the_command_lines_frame_with_the_same_values() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (template, photo) = make(&scratch, (96, 64), 2.0, "Arial").expect("the template is made");
    let named = json!({"Name": "Sam"});
    let both = sam(&photo);
    let as_object = json!({"Name": "Sam", "Photo": {"path": photo.display().to_string()}});
    let every = json!({"Name": "Sam", "Photo": photo.display().to_string(), "Colour": "#ff8800", "Opacity": 40});
    // Each control a values object leaves out keeps the template's own value, so the cases that name
    // one control or two are the missing keys.
    let cases: Vec<(Option<Value>, Value, Vec<&str>)> = vec![
        (None, json!({}), vec![]),
        (Some(named), json!({}), vec![]),
        (Some(both), json!({}), vec![]),
        (Some(as_object), json!({"time": 0.5}), vec!["--time", "0.5"]),
        (Some(sam(&photo)), json!({"time": 1, "scale": 0.5, "transparent": true}), vec!["--time", "1", "--scale", "0.5", "--transparent"]),
        (Some(sam(&photo)), json!({"frame": 5, "maxSide": 40}), vec!["--frame", "5", "--max-side", "40"]),
        // A control is found by its name in any case, as `essential.set` finds it.
        (Some(json!({"name": "Sam", "photo": photo.display().to_string()})), json!({}), vec![]),
        (Some(json!({"Colour": "#ff8800"})), json!({}), vec![]),
        (Some(json!({"Colour": [0.2, 0.9, 0.4, 1]})), json!({}), vec![]),
        (Some(json!({"Opacity": 40})), json!({}), vec![]),
        (Some(json!({"Name": {"text": "Sam"}})), json!({}), vec![]),
        (Some(every), json!({"time": 1}), vec!["--time", "1"]),
    ];
    let mut frames = Vec::new();
    for (values, frame, flags) in &cases {
        let theirs = cli_frame(&scratch, &template, values.as_ref(), flags).expect("the command line rendered the frame");
        let mine = plugin_frame(&scratch, &template, values.as_ref(), frame.clone()).expect("the plugin rendered the frame");
        assert!(mine == theirs, "{values:?} {frame}: the frame is not the command line's, byte for byte");
        frames.push(theirs);
    }
    // The comparison bites: each control, each spelling of a value and the moment change the frame.
    let differ = |first: usize, second: usize, what: &str| assert!(frames.get(first) != frames.get(second), "{what} changes the frame");
    differ(0, 1, "the Name control");
    differ(1, 2, "the Photo control");
    differ(2, 3, "the time");
    differ(0, 7, "the Colour control");
    differ(7, 8, "a colour given as components rather than in hex");
    differ(0, 9, "the Opacity control");
    differ(0, 10, "a name given as a text document");
}

#[test]
fn an_opened_template_is_the_instance_the_frame_call_renders() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (template, photo) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let values = sam(&photo);
    let theirs = cli_frame(&scratch, &template, Some(&values), &[]).expect("the command line rendered the frame");
    let mut app = scratch.app().expect("the app starts");
    let opened = app.request("template.open", json!({"template": template.display().to_string(), "values": values})).expect("the template opens");
    assert!(opened["comp"].is_u64() && opened["warnings"] == json!([]), "{opened}");
    let mine = app.frame(json!({}), PixelFormat::Png).expect("the active comp renders");
    assert!(mine.data == theirs, "the opened instance is the command line's frame");
    let named = app.frame(json!({"comp": opened["comp"]}), PixelFormat::Png).expect("the instance's comp renders");
    assert!(named.data == theirs, "the answer names the instance");
}

#[test]
fn a_missing_font_is_surfaced_as_the_command_line_reports_it() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (template, _) = make(&scratch, (96, 64), 1.0, LOST_FONT).expect("the template is made");
    let path = template.display().to_string();
    let run = cli(
        scratch.root(),
        &[
            "run",
            "--json",
            "--empty",
            "essential.importTemplate",
            &json!({"path": path, "addToComp": false}).to_string(),
            "file.findMissing",
            r#"{"what":"fonts"}"#,
        ],
    )
    .expect("the command line ran")
    .doc()
    .expect("it answered");
    let theirs = run.pointer("/1/result/fonts").cloned().expect("the command line lists the fonts");
    assert_eq!(theirs.as_array().map(Vec::len), Some(1), "the command line reports the font: {run}");
    let mut app = scratch.app().expect("the app starts");
    let opened = app.request("template.open", json!({"template": path})).expect("the template opens");
    assert_eq!(opened["warnings"], theirs, "the answer carries the command line's report");
    let exported = render(&scratch, call(&template, None, "output", json!({"output": scratch.path().join("lost.png").display().to_string(), "format": "png"})));
    assert_eq!(exported.expect("the template exports")["warnings"], theirs, "so does an export's");
    assert_eq!(opened["warnings"][0]["font"], LOST_FONT);
    // A project's own text with a missing font is no warning of the template's.
    let mut other = scratch.app().expect("the app starts");
    for (command, params) in [
        ("comp.new", json!({"name": "Mine", "width": 32, "height": 32, "duration": 1, "frameRate": 10})),
        ("layer.newText", json!({"comp": 1, "text": "x", "font": LOST_FONT})),
    ] {
        other.request("exec", json!({"command": command, "params": params})).expect("the project builds");
    }
    let (clean, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("a template");
    let opened = other.request("template.open", json!({"template": clean.display().to_string()})).expect("the template opens");
    assert_eq!(opened["warnings"], json!([]), "the project's own missing font is not the template's");
}

/// One `template.render` through the core's engine, answering what it answered.
fn render(scratch: &Scratch, params: Value) -> Result<Value, CraftError> {
    let engine = Engine::start(scratch.app()?, GpuChoice::CpuOnly)?;
    let answer = engine.request_with_progress("template.render", params, |_, _, _| {}, &CancelToken::new());
    engine.close()?;
    answer
}

#[test]
fn a_template_export_is_the_command_lines_export() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (template, photo) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let (cli_out, plugin_out) = (scratch.root().join("cli-out"), scratch.path().join("plugin-out"));
    for dir in [&cli_out, &plugin_out] {
        std::fs::create_dir(dir).expect("an output folder");
    }
    let values = sam(&photo);
    let mut args: Vec<String> = ["render", "--json", "--template"].map(String::from).to_vec();
    args.push(template.display().to_string());
    args.extend(values_flag(&scratch, Some(&values)).expect("the values"));
    args.extend(["--out".to_string(), cli_out.join("card.mp4").display().to_string(), "--format".into(), "h264".into()]);
    let mut theirs = cli(scratch.root(), &args.iter().map(String::as_str).collect::<Vec<_>>()).expect("the command line ran").doc().expect("it answered");
    let request = json!({"output": plugin_out.join("card.mp4").display().to_string(), "format": "h264"});
    let mut mine = render(&scratch, call(&template, Some(&values), "output", request)).expect("the plugin exports");
    assert_eq!(mine["warnings"], json!([]));
    let seconds = |doc: &mut Value| doc.pointer_mut("/rendered/0").and_then(Value::as_object_mut).map(|item| item.remove("seconds"));
    seconds(&mut theirs);
    seconds(&mut mine);
    if let Some(answer) = mine.as_object_mut() {
        answer.remove("warnings");
    }
    scrub(&mut theirs, &[&cli_out]);
    scrub(&mut mine, &[&plugin_out]);
    assert_eq!(mine, theirs, "the answers differ");
    assert_eq!(same_files(&plugin_out, &cli_out).expect("the export is the command line's, file for file"), 1);
}

/// The plugin's refusal of a template call and the command line's words for the same call, with the
/// scratch folders scrubbed from both.
fn refusals(scratch: &Scratch, cli_args: &[&str], mine: Result<Value, CraftError>) -> Result<(Value, Value), String> {
    let theirs = cli(scratch.root(), cli_args)?.failure()?;
    let ours = mine.err().ok_or("the plugin refused nothing")?.to_string();
    Ok((scrubbed(&ours, &[scratch.root()]), scrubbed(&theirs, &[scratch.root()])))
}

#[test]
fn a_bad_value_is_refused_in_the_command_lines_words_before_anything_is_imported() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (template, photo) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let path = template.display().to_string();
    let missing = scratch.path().join("none.png");
    // The value, what the command line says, and the command the plugin names before those words: the
    // command line's wrapper of an invalid parameter names its command itself.
    let cases: Vec<(Value, &str, &str)> = vec![
        (json!({"Nope": "x"}), "no control named", "essential.set: "),
        (json!({"Photo": 5}), "is a media control: give it a file path", ""),
        (json!({"Name": "Sam", "Photo": missing.display().to_string()}), "cannot import", "file.import: "),
        (json!({"Colour": "notacolour"}), "for a color property", "essential.set: "),
        (json!({"Opacity": "x"}), "for a scalar property", "essential.set: "),
        (json!({"Name": 5}), "for a text property", "essential.set: "),
    ];
    for (values, why, named) in cases {
        let file = scratch.root().join("values.json");
        std::fs::write(&file, values.to_string()).expect("the values are written");
        let flags = ["render-frame", "--json", "--template", path.as_str(), "--values", file.to_str().unwrap_or_default(), "--out", "x.png"];
        let mut app = scratch.app().expect("the app starts");
        let before = app.request("info", json!({})).expect("info")["project"].clone();
        let answer = app.request("template.open", json!({"template": path, "values": values}));
        let (ours, theirs) = refusals(&scratch, &flags, answer).expect("both sides refuse");
        let (ours, theirs) = (ours.as_str().unwrap_or_default().to_string(), theirs.as_str().unwrap_or_default().to_string());
        assert!(theirs.contains(why), "the command line says {why}: {theirs}");
        let plain = theirs.replace("invalid parameters for `essential.set`: ", "");
        assert_eq!(ours, format!("{named}{plain}"), "{values}: not the command line's words");
        if why != "cannot import" {
            let after = app.request("info", json!({})).expect("info")["project"].clone();
            assert_eq!(after, before, "{values}: a refused value leaves the project as it was");
        }
    }
    // A control the values name and the template lacks is a thing named that is not there.
    let mut app = scratch.app().expect("the app starts");
    let lacked = app.request("template.open", json!({"template": path, "values": {"Nope": "x"}})).expect_err("refused");
    assert!(matches!(lacked, CraftError::NotFound { .. }), "a missing control is `NotFound`: {lacked:?}");
    let _ = photo;
}

/// The template of a small card made in `scratch`, by its path, and an engine started on it.
fn started_with_template(scratch: &Scratch) -> Result<(String, Engine), String> {
    let (template, _) = make(scratch, (96, 64), 1.0, "Arial")?;
    let engine = Engine::start(scratch.app().map_err(|e| e.to_string())?, GpuChoice::CpuOnly).map_err(|e| e.to_string())?;
    Ok((template.display().to_string(), engine))
}

/// A render of `path` with the request `output` that the engine refuses as `InvalidParams` and that leaves
/// what `state` reads as `before`, answering the refusal.
fn refused_render<T: PartialEq + std::fmt::Debug>(
    engine: &Engine,
    path: &str,
    output: &Value,
    state: &dyn Fn() -> T,
    before: &T,
) -> Result<CraftError, String> {
    let params = json!({"template": path, "output": output});
    let refused = engine.request_with_progress(RENDER, params, |_, _, _| {}, &CancelToken::new()).err().ok_or("the render is refused")?;
    if !matches!(refused, CraftError::InvalidParams { .. }) {
        return Err(format!("{output}: {refused:?}"));
    }
    if &state() != before {
        return Err(format!("{output}: a refused render request leaves something behind"));
    }
    Ok(refused)
}

#[test]
fn a_request_refused_for_its_shape_leaves_no_instance_behind() {
    // A frame's or a render's own request is read whole before the template is imported, as the
    // command line reads its whole command line first: a request refused for its shape leaves the
    // project as it was.
    let scratch = Scratch::new().expect("a scratch folder");
    let (path, engine) = started_with_template(&scratch).expect("the template and the engine");
    let project = || engine.request("info", json!({})).expect("info answers")["project"].clone();
    let before = project();
    for frame in [json!({"time": 1, "frame": 2}), json!({"time": "1"}), json!({"nope": 1})] {
        let refused = engine.frame(json!({"template": path, "frame": frame}), PixelFormat::Png).expect_err("refused");
        assert!(matches!(refused, CraftError::InvalidParams { .. }), "{frame}: {refused:?}");
        assert_eq!(project(), before, "{frame}: a refused frame request leaves no instance behind");
    }
    let out = scratch.path().join("x.mp4").display().to_string();
    for output in [json!({"output": 5}), json!({"output": out, "end": "x"}), json!({"output": out, "nope": 1})] {
        refused_render(&engine, &path, &output, &project, &before).expect("refused, and nothing left behind");
    }
    engine.close().expect("the engine closes");
}

#[test]
fn a_call_the_command_line_would_not_make_is_refused() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (template, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let path = template.display().to_string();
    let mut app = scratch.app().expect("the app starts");
    let mut refused = |method: &str, params: Value| app.request(method, params).expect_err("refused").to_string();
    assert_eq!(refused("template.open", json!({})), "`template.open`: needs a `template`");
    assert_eq!(
        refused("template.open", json!({"template": path, "values": [1]})),
        "`template.open`: `values` must be a JSON object mapping control names to values"
    );
    assert_eq!(refused("template.open", json!({"template": path, "frame": {}})), "`template.open`: unknown parameter(s) `frame`");
    drop(refused);
    let out = |name: &str| scratch.path().join(name).display().to_string();
    let words = |params: Value| render(&scratch, params).expect_err("refused").to_string();
    let request = |fields: Value| json!({"template": path, "output": fields});
    assert_eq!(words(json!({"template": path})), "`template.render`: needs an `output` request");
    assert_eq!(
        words(request(json!({"output": out("x.mp4"), "comp": 1, "queue": true}))),
        "`template.render`: `output` cannot name `comp`, `queue`: a template renders a new instance"
    );
    assert_eq!(words(json!({"template": path, "frame": {}})), "`template.render`: unknown parameter(s) `frame`");
    assert_eq!(words(request(json!({}))), "`export`: `output` is required (or `queue`)");
    // The render would overwrite the template it was made from, as the command line refuses.
    let cli_words =
        cli(scratch.root(), &["render", "--json", "--template", &path, "--out", &path]).expect("the command line ran").failure().expect("it refuses");
    assert_eq!(cli_words, format!("--out {path} would overwrite --template {path}"));
    assert_eq!(words(request(json!({"output": path}))), format!("`template.render`: `output` {path} would overwrite `template` {path}"));
    let frame = |params: Value| scratch.app().expect("the app starts").frame(params, PixelFormat::Png).expect_err("refused").to_string();
    assert_eq!(frame(json!({"template": path, "frame": {"comp": 1}})), "`template.render`: `frame` cannot name `comp`: a template renders a new instance");
    assert_eq!(frame(json!({"template": path, "output": {}})), "`template.render`: unknown parameter(s) `output`");
}

#[test]
fn a_template_outside_the_container_is_refused_by_the_confinement() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (template, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let outside = scratch.outside().join("card.ectemplate");
    std::fs::copy(&template, &outside).expect("a copy outside the roots");
    let mut app = scratch.app().expect("the app starts");
    // A value no control has would be refused by the scratch check if the template were read, which
    // the confinement forbids as it forbids the import.
    for values in [json!({}), json!({"Name": "Sam"}), json!({"Nope": "x"})] {
        let refused = app.request("template.open", json!({"template": outside.display().to_string(), "values": values})).expect_err("outside the roots");
        assert!(matches!(refused, CraftError::Io { .. }), "{refused:?}");
    }
    assert!(!scratch.outside().join("card Media").exists(), "nothing was extracted outside the roots");
}

/// An app that has opened a small template with no values, and what `template.open` answered.
fn opened_template(scratch: &Scratch) -> Result<(EffectCraftApp, Value), String> {
    let (template, _) = make(scratch, (96, 64), 1.0, "Arial")?;
    let mut app = scratch.app().map_err(|e| e.to_string())?;
    let opened = app.request("template.open", json!({"template": template.display().to_string()})).map_err(|e| e.to_string())?;
    Ok((app, opened))
}

#[test]
fn an_instance_is_the_one_layer_of_a_wrapper_the_template_names() {
    // The wrapper is named as the template's composition is, its layer ends where it does and the
    // layer carries the source's motion blur switch, as the command line sets the instance up.
    let scratch = Scratch::new().expect("a scratch folder");
    let (mut app, opened) = opened_template(&scratch).expect("the template opens");
    let layer = app.request("props", json!({"comp": opened["comp"], "layer": "#1"})).expect("the instance's layer reads");
    assert_eq!(layer["out"], json!(1.0), "the instance ends where the composition does");
    assert_eq!(layer["switches"]["motion_blur"], json!(true), "the instance carries the source's motion blur");
    let info = app.request("info", json!({})).expect("info answers");
    assert_eq!(info["activeComp"]["name"], json!("Card"), "the wrapper is named as the template's composition");
}

/// Put a copy of `template` at `<root>/<folder>/<name>`, the way an app bundle holds one.
fn bundled(template: &Path, root: &Path, folder: &str) -> Result<PathBuf, String> {
    let dir = root.join(folder);
    std::fs::create_dir_all(&dir).map_err(|e| format!("a folder in the bundle: {e}"))?;
    let copy = dir.join(FILE);
    std::fs::copy(template, &copy).map_err(|e| format!("the template is copied into the bundle: {e}"))?;
    Ok(copy)
}

/// A template made in the container and bundled at `<bundle>/<folder>/card.ectemplate`: the template
/// in the container, the bundle's root and the template's path in the bundle.
fn bundled_template(scratch: &Scratch, folder: &str) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let bundle = scratch.bundle().ok_or("a bundle")?.to_path_buf();
    let (template, _) = make(scratch, (96, 64), 1.0, "Arial")?;
    let in_bundle = bundled(&template, &bundle, folder)?;
    Ok((template, bundle, in_bundle))
}

#[test]
fn a_template_in_the_bundle_is_imported_from_a_copy_inside_the_container() {
    // The import writes a template's media beside the template. A template in the bundle, a root the
    // app may read and never write, is imported from a copy in the container's staged folder, so the media
    // come out whole and nothing is written under the bundle, which is made read-only here so that
    // a write would fail as it does on iOS and the media would then come out missing.
    let scratch = Scratch::with_bundle().expect("a scratch folder");
    let bundle = scratch.bundle().expect("a bundle").to_path_buf();
    let (first, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let theirs = cli_frame(&scratch, &first, None, &[]).expect("the command line rendered the frame");
    let in_bundle = bundled(&first, &bundle, "a").expect("the first template is bundled");
    let kept = scratch.path().join("first.ectemplate");
    std::fs::copy(&first, &kept).expect("the first template is kept, as the next one is made at its name");
    let (second, _) = make_over(&scratch, (64, 48), 1.0, "Arial", [30, 30, 200]).expect("the second template is made");
    let their_second = cli_frame(&scratch, &second, None, &[]).expect("the command line rendered the second frame");
    let also_bundled = bundled(&second, &bundle, "b").expect("the second template is bundled");
    assert_ne!(theirs, their_second, "the two templates are told apart by their frames");
    let before = entries_under(&bundle, &[]).expect("the bundle reads");
    let set_read_only = |read_only: bool| {
        let mode = if read_only { 0o555 } else { 0o755 };
        for folder in [bundle.clone(), bundle.join("a"), bundle.join("b")] {
            std::fs::set_permissions(&folder, std::os::unix::fs::PermissionsExt::from_mode(mode)).expect("the permissions are set");
        }
    };
    set_read_only(true);
    let mut app = scratch.app().expect("the app starts");
    let opened = app.request("template.open", json!({"template": in_bundle.display().to_string()}));
    // Two templates of one file name in two folders never share a copy: the second opened in the
    // same session does not change the media the first's instance shows, which its first frame reads
    // only now.
    let comp = opened.as_ref().map(|opened| opened["comp"].clone()).expect("the bundled template opens");
    let _ = app.request("template.open", json!({"template": also_bundled.display().to_string()}));
    let again = app.frame(json!({"comp": comp}), PixelFormat::Png).map(|frame| frame.data);
    // And in fresh sessions each renders as the command line renders it.
    let both = [&in_bundle, &also_bundled].map(|path| plugin_frame(&scratch, path, None, json!({})));
    let (ours, wanted) = (scratch.path().join("bundled"), scratch.root().join("wanted"));
    for dir in [&ours, &wanted] {
        std::fs::create_dir(dir).expect("an output folder");
    }
    let cli_args = ["render", "--json", "--template", &kept.display().to_string(), "--out", &wanted.join("card.mp4").display().to_string(), "--format", "h264"]
        .map(String::from);
    cli(scratch.root(), &cli_args.iter().map(String::as_str).collect::<Vec<_>>()).expect("the command line ran").doc().expect("it answered");
    let exported = render(&scratch, call(&in_bundle, None, "output", json!({"output": ours.join("card.mp4").display().to_string(), "format": "h264"})));
    set_read_only(false);
    assert!(opened.expect("the bundled template opens")["warnings"] == json!([]));
    assert!(again.expect("the first instance renders again") == theirs, "the first instance shows its own media, though the second template was opened since");
    assert!(both[0].as_ref().expect("the first renders") == &theirs, "the first template's frame is the command line's");
    assert!(both[1].as_ref().expect("the second renders") == &their_second, "the second template does not share the first's copy");
    exported.expect("the bundled template exports");
    assert_eq!(same_files(&ours, &wanted).expect("the export is the command line's, file for file"), 1);
    assert_eq!(entries_under(&bundle, &[]).expect("the bundle reads"), before, "nothing was written under the bundle");
}

#[test]
fn a_template_inside_the_container_is_imported_where_it_is() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (template, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let mut app = scratch.app().expect("the app starts");
    app.request("template.open", json!({"template": template.display().to_string()})).expect("the template opens");
    assert!(scratch.path().join("card Media").is_dir(), "the media are written beside the template");
    assert!(
        entries_under(&scratch.folders.staged, &[]).expect("the staged folder reads").is_empty(),
        "no copy is made of a template that may be written beside"
    );
}

#[test]
fn a_missing_control_is_not_found_through_every_command_that_names_one() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (mut app, opened) = opened_template(&scratch).expect("the template opens");
    let (wrapper, source) = (opened["comp"].clone(), json!(1));
    // Every command that takes a control by name or id, with the name and the id the engine says it
    // lacks.
    let on_source = |extra: Value, control: &Value| {
        let mut params = json!({"comp": source, "control": control});
        if let (Some(fields), Some(more)) = (params.as_object_mut(), extra.as_object()) {
            fields.extend(more.clone());
        }
        params
    };
    let instance = |control: &Value| json!({"comp": wrapper, "layer": "#1", "control": control, "value": "x"});
    let (name, id) = (json!("Nope"), json!(99_999));
    let calls = [
        ("essential.set", instance(&name)),
        ("essential.set", instance(&id)),
        ("essential.rename", on_source(json!({"name": "x"}), &name)),
        ("essential.rename", on_source(json!({"name": "x"}), &id)),
        ("essential.remove", on_source(json!({}), &id)),
        ("essential.move", on_source(json!({}), &id)),
        ("essential.unlinkProperty", on_source(json!({}), &id)),
    ];
    for (command, params) in calls {
        let control = params["control"].clone();
        let by_exec = app.request("exec", json!({"command": command, "params": params})).expect_err("refused");
        assert!(matches!(by_exec, CraftError::NotFound { .. }), "{command} {control}: `exec` answers {by_exec:?}");
        let by_run = app.request("run", json!({"steps": [{"command": command, "params": params}]})).expect_err("refused");
        assert!(matches!(by_run, CraftError::NotFound { .. }), "{command} {control}: `run` answers {by_run:?}");
    }
}

#[test]
fn a_bundle_the_system_lets_be_written_is_not_written_either() {
    // On Android the bundle's unpacked assets can be written, so only the confinement keeps the media
    // out of them.
    let scratch = Scratch::with_bundle().expect("a scratch folder");
    let (_, bundle, in_bundle) = bundled_template(&scratch, "assets").expect("the template is bundled");
    let before = entries_under(&bundle, &[]).expect("the bundle reads");
    let mut app = scratch.app().expect("the app starts");
    app.request("template.open", json!({"template": in_bundle.display().to_string()})).expect("the bundled template opens");
    assert_eq!(entries_under(&bundle, &[]).expect("the bundle reads"), before, "nothing was written under the bundle");
    assert!(
        !entries_under(&scratch.folders.staged, &[]).expect("the staged folder reads").is_empty(),
        "the template was imported from a copy in the staged folder"
    );
}

#[test]
fn a_bundled_template_keeps_its_media_after_the_system_empties_the_cache() {
    // The instance of a bundled template reads its media from beside the staged copy, and the saved
    // project names those files. The copies therefore live in the staged folder, which the system
    // keeps, and not in the cache it may empty while the app is not running: open the saved project
    // after such an emptying and every medium is there, the frame it renders unchanged.
    let scratch = Scratch::with_bundle().expect("a scratch folder");
    let (template, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let in_bundle = bundled(&template, scratch.bundle().expect("a bundle"), "assets").expect("the template is bundled");
    let project = scratch.path().join("keep.ecproj");
    let ours = {
        let mut app = scratch.app().expect("the app starts");
        app.request("template.open", json!({"template": in_bundle.display().to_string()})).expect("the bundled template opens");
        let frame = app.frame(json!({}), PixelFormat::Png).expect("the instance renders").data;
        app.request("saveAs", json!({"path": project.display().to_string()})).expect("the project saves");
        frame
    };
    // The system empties the cache while no app runs, as iOS may do. The staged folder beside it
    // stays as it was, and staging never wrote one file into the cache.
    std::fs::remove_dir_all(&scratch.folders.cache).expect("the cache is emptied");
    let mut again = scratch.app().expect("the app starts again");
    again.request("open", json!({"path": project.display().to_string()})).expect("the saved project opens");
    let after = again.frame(json!({}), PixelFormat::Png).expect("the reopened instance renders").data;
    assert!(after == ours, "the reopened project renders every medium it saved, though the cache is gone");
    assert!(
        !entries_under(&scratch.folders.staged, &[]).expect("the staged folder reads").is_empty(),
        "the copy whose media the project names sits in the staged folder the system keeps"
    );
    assert_eq!(std::fs::read_dir(&scratch.folders.cache).expect("the cache reads again").count(), 0, "nothing of the import was written into the cache");
}

#[test]
fn a_bundled_template_keeps_one_copy_across_updates_and_a_saved_project_its_media() {
    // An update installs the bundle anew, at a new path on iOS, and may change a template in it where
    // it sits. The same template, wherever it is installed, is read from the copy it had, so no update
    // leaves a set of copies and media that nothing names again; a changed one is read from a copy of
    // what it holds now, never from the old one; and a project saved before both updates, the cache
    // emptied on the way, still renders every medium it saved.
    let scratch = Scratch::with_bundle().expect("a scratch folder");
    let (first, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let first = std::fs::read(&first).expect("the template reads");
    let (second, _) = make_over(&scratch, (96, 64), 1.0, "Arial", [30, 30, 200]).expect("the changed template is made");
    let theirs = cli_frame(&scratch, &second, None, &[]).expect("the command line rendered the changed template's frame");
    let second = std::fs::read(&second).expect("the changed template reads");
    let install = |bundle: &Path, bytes: &[u8]| {
        let path = bundle.join("assets").join(FILE);
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("the bundle's folder");
        std::fs::write(&path, bytes).expect("the template is installed");
        json!({"template": path.display().to_string()})
    };
    let copies = || std::fs::read_dir(&scratch.folders.staged).expect("the staged folder reads").flatten().filter(|entry| entry.path().is_dir()).count();
    let rendered = |app: &mut EffectCraftApp| app.frame(json!({}), PixelFormat::Png).expect("the comp renders").data;
    let project = scratch.path().join("before.ecproj");
    let saved = {
        let mut app = scratch.app().expect("the app starts");
        app.request("template.open", install(scratch.bundle().expect("a bundle"), &first)).expect("the bundled template opens");
        app.request("saveAs", json!({"path": project.display().to_string()})).expect("the project saves");
        rendered(&mut app)
    };
    assert_eq!(copies(), 1);
    // The update installs the same template at a new path; the system empties the cache meanwhile.
    let moved = tempfile::tempdir().expect("the updated bundle's folder");
    let updated = craftbridge_core::folders::Folders { bundle: Some(moved.path().to_path_buf()), ..scratch.folders.clone() };
    let reinstalled = install(moved.path(), &first);
    std::fs::remove_dir_all(&scratch.folders.cache).expect("the cache is emptied");
    let mut app = EffectCraftApp::new(&updated).expect("the updated app starts");
    app.request("open", json!({"path": project.display().to_string()})).expect("the saved project opens");
    assert!(rendered(&mut app) == saved, "the project saved before the update renders every medium it saved");
    app.request("template.open", reinstalled.clone()).expect("the reinstalled template opens");
    assert!(rendered(&mut app) == saved, "the reinstalled template is the same template");
    assert_eq!(copies(), 1, "the same template installed at a new path is read from the copy it had");
    // A later update changes the template where it sits.
    drop(app);
    install(moved.path(), &second);
    let mut app = EffectCraftApp::new(&updated).expect("the app starts after the second update");
    app.request("template.open", reinstalled).expect("the changed template opens");
    assert!(rendered(&mut app) == theirs, "the changed template is read as it is now, as the command line reads it");
    app.request("open", json!({"path": project.display().to_string()})).expect("the saved project opens again");
    assert!(rendered(&mut app) == saved, "the project saved before both updates still renders the media it saved");
    assert_eq!(copies(), 2, "the changed template has a copy of its own");
}

#[test]
fn a_link_in_the_bundle_to_a_template_in_the_container_is_imported_from_a_copy() {
    // The import writes beside the template as it is named, in the folder holding the name, not where
    // a link leads: a link in the bundle to a template in the container would have the media written
    // into the bundle, so it is read from a copy too.
    let scratch = Scratch::with_bundle().expect("a scratch folder");
    let bundle = scratch.bundle().expect("a bundle").to_path_buf();
    let (template, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let theirs = cli_frame(&scratch, &template, None, &[]).expect("the command line rendered the frame");
    std::fs::create_dir(bundle.join("links")).expect("a folder in the bundle");
    let link = bundle.join("links").join(FILE);
    std::os::unix::fs::symlink(&template, &link).expect("a link in the bundle to the template");
    let before = entries_under(&bundle, &[]).expect("the bundle reads");
    let mine = plugin_frame(&scratch, &link, None, json!({})).expect("the linked template renders");
    assert_eq!(entries_under(&bundle, &[]).expect("the bundle reads"), before, "nothing was written under the bundle");
    assert!(mine == theirs, "the linked template's frame is the command line's");
}

#[test]
fn a_call_that_is_not_answered_makes_no_copy() {
    // A template is copied only for a call the app answers: one refused for its name never touches
    // the staged folder.
    let scratch = Scratch::with_bundle().expect("a scratch folder");
    let (_, _, in_bundle) = bundled_template(&scratch, "assets").expect("the template is bundled");
    let params = json!({"template": in_bundle.display().to_string()});
    let mut app = scratch.app().expect("the app starts");
    for method in [RENDER, "frame"] {
        let refused = app.request(method, params.clone()).expect_err("`request` does not answer it");
        assert!(matches!(refused, CraftError::Unsupported { .. }), "{method}: {refused:?}");
    }
    let refused = app.request_with_progress("template.open", params, &mut |_, _, _| true, &mut |_| Ok(())).expect_err("not answered");
    assert!(matches!(refused, CraftError::Unsupported { .. }), "{refused:?}");
    assert!(entries_under(&scratch.folders.staged, &[]).expect("the staged folder reads").is_empty(), "a refused call copied nothing");
}

#[test]
fn a_read_only_template_in_the_bundle_opens_again_and_after_a_restart() {
    // A bundle can hold files nobody may write, whose copy must not take that mode, or the second
    // import of the same template would find its copy unwritable.
    let scratch = Scratch::with_bundle().expect("a scratch folder");
    let (_, _, in_bundle) = bundled_template(&scratch, "assets").expect("the template is bundled");
    std::fs::set_permissions(&in_bundle, std::os::unix::fs::PermissionsExt::from_mode(0o444)).expect("the template is read-only");
    let params = json!({"template": in_bundle.display().to_string()});
    for start in ["the first start", "a restart"] {
        let mut app = scratch.app().expect("the app starts");
        for time in ["once", "again"] {
            app.request("template.open", params.clone()).unwrap_or_else(|error| panic!("{start}, opened {time}: {error}"));
        }
    }
}

#[test]
fn a_folder_named_as_the_template_gets_the_commands_own_refusal() {
    let scratch = Scratch::with_bundle().expect("a scratch folder");
    let folder = scratch.bundle().expect("a bundle").join("card.ectemplate");
    std::fs::create_dir(&folder).expect("a folder in the bundle");
    let mut app = scratch.app().expect("the app starts");
    let refused = app.request("template.open", json!({"template": folder.display().to_string()})).expect_err("a folder is no template");
    assert!(!refused.to_string().contains("cannot copy"), "the command refuses it, not the copy: {refused}");
    assert!(entries_under(&scratch.folders.staged, &[]).expect("the staged folder reads").is_empty(), "nothing was copied");
}

#[test]
fn a_link_in_the_container_to_a_template_in_the_bundle_is_imported_where_it_is() {
    // The import writes beside the name it is given, which is inside the container.
    let scratch = Scratch::with_bundle().expect("a scratch folder");
    let (_, bundle, in_bundle) = bundled_template(&scratch, "assets").expect("the template is bundled");
    let link = scratch.path().join("linked.ectemplate");
    std::os::unix::fs::symlink(&in_bundle, &link).expect("a link in the container");
    let before = entries_under(&bundle, &[]).expect("the bundle reads");
    let mut app = scratch.app().expect("the app starts");
    app.request("template.open", json!({"template": link.display().to_string()})).expect("the linked template opens");
    assert_eq!(entries_under(&bundle, &[]).expect("the bundle reads"), before, "nothing was written under the bundle");
    assert!(scratch.path().join("linked Media").is_dir(), "the media are written beside the name given");
    assert!(entries_under(&scratch.folders.staged, &[]).expect("the staged folder reads").is_empty(), "no copy is made of a name inside the write root");
}

#[test]
fn a_render_that_would_overwrite_a_template_read_from_a_copy_is_refused() {
    // A link in the bundle leads to a template in the container, so the template is read from a
    // copy; the output that names the file the link leads to would still overwrite the template,
    // which the command line refuses, and the refusal names the template as the call named it.
    let scratch = Scratch::with_bundle().expect("a scratch folder");
    let bundle = scratch.bundle().expect("a bundle").to_path_buf();
    let (template, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let (link, original) = (bundle.join(FILE), std::fs::read(&template).expect("the template reads"));
    std::os::unix::fs::symlink(&template, &link).expect("a link in the bundle to the template");
    let params = json!({"template": link.display().to_string(), "output": {"output": template.display().to_string(), "format": "h264"}});
    let words = render(&scratch, params).expect_err("refused").to_string();
    assert_eq!(words, format!("`template.render`: `output` {} would overwrite `template` {}", template.display(), link.display()));
    assert_eq!(std::fs::read(&template).expect("the template reads"), original, "the template is as it was");
}

#[test]
fn a_render_naming_a_comp_is_refused_for_it_before_its_output_is_weighed() {
    // The command line refuses `--comp` beside `--template` before it compares `--out` with the
    // template, so a call doing both is refused for the comp, as one doing only that is.
    let scratch = Scratch::new().expect("a scratch folder");
    let (template, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let path = template.display().to_string();
    // A usage error, which the command line says on its error stream rather than as a document.
    let theirs = cli(scratch.root(), &["render", "--json", "--template", &path, "--out", &path, "--comp", "1"]).expect("the command line ran").stderr;
    assert!(theirs.contains("--template renders a new instance") && !theirs.contains("would overwrite"), "the command line refuses the comp first: {theirs}");
    let words = render(&scratch, json!({"template": path, "output": {"output": path, "comp": 1}})).expect_err("refused").to_string();
    assert_eq!(words, "`template.render`: `output` cannot name `comp`: a template renders a new instance");
}

#[test]
fn a_call_refused_for_its_shape_makes_no_copy() {
    // Each call is read whole, as its method reads it, before its template is copied.
    let scratch = Scratch::with_bundle().expect("a scratch folder");
    let bundle = scratch.bundle().expect("a bundle").to_path_buf();
    let (template, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let named = bundled(&template, &bundle, "assets").expect("the template is bundled").display().to_string();
    let mut app = scratch.app().expect("the app starts");
    let opened = app.request("template.open", json!({"template": named, "values": [1]})).expect_err("refused");
    assert!(matches!(opened, CraftError::InvalidParams { .. }), "{opened:?}");
    let framed = app.frame(json!({"template": named, "frame": {"time": 1, "frame": 2}}), PixelFormat::Png).expect_err("refused");
    assert!(matches!(framed, CraftError::InvalidParams { .. }), "{framed:?}");
    let rendered =
        app.request_with_progress(RENDER, json!({"template": named, "output": {"output": 5}}), &mut |_, _, _| true, &mut |_| Ok(())).expect_err("refused");
    assert!(matches!(rendered, CraftError::InvalidParams { .. }), "{rendered:?}");
    assert!(entries_under(&scratch.folders.staged, &[]).expect("the staged folder reads").is_empty(), "a call refused for its shape copied nothing");
}

#[test]
fn a_template_named_as_an_export_file_is_never_overwritten_by_its_own_render() {
    // The render writes the file the output names when the name already carries the format's
    // extension, so a template saved under such a name would be replaced by its own render: the
    // refusal is what keeps it, as the command line's is.
    let scratch = Scratch::new().expect("a scratch folder");
    let (made, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let template = scratch.path().join("card.mp4");
    std::fs::rename(&made, &template).expect("the template takes an export file's name");
    let original = std::fs::read(&template).expect("the template reads");
    let path = template.display().to_string();
    let theirs = cli(scratch.root(), &["render", "--json", "--template", &path, "--out", &path, "--format", "h264"])
        .expect("the command line ran")
        .failure()
        .expect("it refuses");
    assert_eq!(theirs, format!("--out {path} would overwrite --template {path}"));
    let words = render(&scratch, json!({"template": path, "output": {"output": path, "format": "h264"}})).expect_err("refused").to_string();
    assert_eq!(words, format!("`template.render`: `output` {path} would overwrite `template` {path}"));
    assert_eq!(std::fs::read(&template).expect("the template reads"), original, "the template is as it was");
}

#[test]
fn a_call_is_refused_in_the_order_the_command_line_refuses_it() {
    // The command line's `validate` refuses the instance choice, a missing `--out` and an output that
    // is the template before it reads the values file, and reads its flags after the import, so a call
    // with several faults is refused for the first of those in that order, in each of their words.
    let scratch = Scratch::new().expect("a scratch folder");
    let (template, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let path = template.display().to_string();
    let values = scratch.root().join("values.json");
    std::fs::write(&values, "[1]").expect("a values file that is no object");
    let flags = values.display().to_string();
    let theirs = |extra: &[&str]| {
        let mut args = vec!["render", "--json", "--template", path.as_str(), "--values", flags.as_str()];
        args.extend(extra);
        let run = cli(scratch.root(), &args).expect("the command line ran");
        format!("{}{}", run.stderr, run.failure().unwrap_or_default())
    };
    let ours = |output: Value| render(&scratch, json!({"template": path, "values": [1], "output": output})).expect_err("refused").to_string();
    let out = scratch.path().join("x.mp4").display().to_string();
    // First of all, the command line's parser refuses an option it does not know, which a field the
    // request does not take is, and a request that is no object at all.
    assert!(theirs(&["--widht", "1", "--out", &out, "--comp", "1"]).contains("unknown option `--widht`"));
    assert_eq!(ours(json!({"output": out, "widht": 1, "comp": 1})), "`export`: unknown parameter(s) `widht`");
    assert_eq!(ours(json!(5)), "`export`: its parameters must be a JSON object");
    assert!(theirs(&["--out", &out, "--comp", "1"]).contains("--template renders a new instance"));
    assert_eq!(ours(json!({"output": out, "comp": 1})), "`template.render`: `output` cannot name `comp`: a template renders a new instance");
    assert!(theirs(&["--end", "x"]).contains("render: --out FILE is required"));
    assert_eq!(ours(json!({"end": "x"})), "`export`: `output` is required (or `queue`)");
    assert!(theirs(&["--out", &path, "--end", "x"]).contains("would overwrite --template"));
    assert_eq!(ours(json!({"output": path, "end": "x"})), format!("`template.render`: `output` {path} would overwrite `template` {path}"));
    assert!(theirs(&["--out", &out, "--end", "x"]).contains("expected a JSON object mapping control names to values"));
    assert_eq!(ours(json!({"output": out, "end": "x"})), "`template.render`: `values` must be a JSON object mapping control names to values");
    let frame = |params: Value| scratch.app().expect("the app starts").frame(params, PixelFormat::Png).expect_err("refused").to_string();
    let theirs_frame = |extra: &[&str]| {
        let mut args = vec!["render-frame", "--json", "--template", path.as_str(), "--values", flags.as_str()];
        args.extend(extra);
        cli(scratch.root(), &args).expect("the command line ran").stderr
    };
    assert!(theirs_frame(&["--nope", "1", "--comp", "1"]).contains("unknown option `--nope`"));
    assert_eq!(frame(json!({"template": path, "values": [1], "frame": {"nope": 1, "comp": 1}})), "`frame`: unknown parameter(s) `nope`");
    assert_eq!(
        frame(json!({"template": path, "values": [1], "frame": {"comp": 1}})),
        "`template.render`: `frame` cannot name `comp`: a template renders a new instance"
    );
    // The command line knows `--queue`, which no frame takes, and refuses it beside `--template`.
    assert!(theirs_frame(&["--queue"]).contains("--template renders a new instance"));
    assert_eq!(
        frame(json!({"template": path, "values": [1], "frame": {"queue": true}})),
        "`template.render`: `frame` cannot name `queue`: a template renders a new instance"
    );
}

#[test]
fn a_null_or_false_field_asks_for_nothing_as_the_requests_read_it() {
    // `export` and `frame` read a `comp` or `queue` that is null, and a `queue` that is false, as not
    // given, and an `output` that is null as absent: so do the template's calls.
    let scratch = Scratch::new().expect("a scratch folder");
    let (template, _) = make(&scratch, (96, 64), 1.0, "Arial").expect("the template is made");
    let path = template.display().to_string();
    let out = scratch.path().join("plain.mp4").display().to_string();
    render(&scratch, json!({"template": path, "output": {"output": out, "format": "h264", "comp": null, "queue": false}}))
        .expect("a render that asks for neither");
    render(&scratch, json!({"template": path, "output": {"output": scratch.path().join("null.mp4").display().to_string(), "format": "h264", "queue": null}}))
        .expect("a null queue");
    let plain = plugin_frame(&scratch, &template, None, json!({})).expect("the frame renders");
    assert!(plugin_frame(&scratch, &template, None, json!({"comp": null})).expect("a null comp renders") == plain, "a null comp is the instance");
    let words = render(&scratch, json!({"template": path, "values": [1], "output": {"output": null, "end": "x"}})).expect_err("refused").to_string();
    assert_eq!(words, "`export`: `output` is required (or `queue`)", "a null output is refused as an absent one is, before the values");
}

#[test]
fn a_template_output_that_queues_and_asks_for_hardware_is_refused_in_the_exports_words_before_anything_renders() {
    let scratch = Scratch::new().expect("a scratch folder");
    let (path, engine) = started_with_template(&scratch).expect("the template and the engine");
    let state =
        || (engine.request("info", json!({})).expect("info answers")["project"].clone(), entries_under(scratch.path(), &[]).expect("the container lists"));
    let before = state();
    let out = scratch.path().join("x.mp4").display().to_string();
    for output in [json!({"queue": true, "hardware": true}), json!({"output": out, "format": "h264", "queue": true, "hardware": true, "comp": 1})] {
        assert_eq!(
            refused_render(&engine, &path, &output, &state, &before).expect("refused, and nothing left behind").to_string(),
            QUEUED_HARDWARE,
            "{output}"
        );
    }
    let params = json!({"template": path, "output": {"output": out, "queue": true}});
    let refused = engine.request_with_progress(RENDER, params, |_, _, _| {}, &CancelToken::new()).expect_err("a queue beside a template is refused");
    assert_eq!(refused.to_string(), "`template.render`: `output` cannot name `queue`: a template renders a new instance", "a queue alone is refused as before");
    engine.close().expect("the engine closes");
}
