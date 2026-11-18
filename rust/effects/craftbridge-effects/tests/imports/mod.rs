//! A Lottie file, a vector file or a template in the app bundle, which the session may read and never
//! write, is imported as the command line imports it from a writable folder, with every image or medium
//! its import writes beside it and every image it names, on every way a call reaches the commands, and
//! nothing is written under the bundle, whether the disk would refuse the write (iOS) or take it
//! (Android).

use super::common::Scratch;
use super::export::entries_under;
use super::template::{FILE, make};
use super::{cli_run, scrub, steps_document, tools_call};
use craftbridge_core::{CraftApp, CraftError};
use craftbridge_effects::EffectCraftApp;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// A one pixel PNG, as the data URI of an image a file embeds.
const PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/// Where a bundle's Lottie file sits, and the images it names by path: one beside it, one a `..`
/// leads to.
const LOTTIE: &str = "anims/anim.json";
const NAMED: [&str; 2] = ["anims/img/ext.png", "shared/up.png"];

/// A Lottie file whose image assets are, as asked, one it embeds, which its import writes as
/// `<name>_<asset id>.png`, and the two `NAMED` it names by a path relative to its own folder.
fn lottie(embeds: bool, names: bool) -> String {
    let image = |id: &str, u: &str, p: &str| json!({"id": id, "w": 1, "h": 1, "u": u, "p": p});
    let mut assets = Vec::new();
    if embeds {
        assets.push(image("image_0", "", PIXEL));
    }
    if names {
        assets.extend([image("image_1", "img/", "ext.png"), image("image_2", "../shared/", "up.png")]);
    }
    let layers: Vec<Value> = assets
        .iter()
        .enumerate()
        .map(|(index, asset)| json!({"ty": 2, "refId": asset.get("id"), "ind": index + 1, "ip": 0, "op": 30, "st": 0, "nm": "pic", "ks": {}}))
        .collect();
    json!({"v": "5.7.0", "fr": 30, "ip": 0, "op": 30, "w": 64, "h": 64, "nm": "Bundled", "assets": assets, "layers": layers}).to_string()
}

/// A one-page PDF, a vector file, that draws a red square and, when `image` is set, one two pixel
/// image of its own, which Create Shapes from Vector Layer writes as `<file> Image 1.png`.
fn vector(image: bool) -> Vec<u8> {
    let content: &[u8] = if image { b"1 0 0 rg 0 0 8 8 re f q 40 0 0 20 0 0 cm /Im1 Do Q\n" } else { b"1 0 0 rg 0 0 8 8 re f\n" };
    let stream = |dict: &str, data: &[u8]| [format!("<< {dict} /Length {} >>\nstream\n", data.len()).as_bytes(), data, b"\nendstream"].concat();
    let resources = if image { "/Resources << /XObject << /Im1 5 0 R >> >>" } else { "" };
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 64 64] {resources} /Contents 4 0 R >>").into_bytes(),
        stream("", content),
    ];
    if image {
        objects.push(stream("/Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8", &[255, 0, 0, 0, 255, 0]));
    }
    let mut file = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (number, body) in objects.iter().enumerate() {
        offsets.push(file.len());
        file.extend_from_slice(format!("{} 0 obj\n", number + 1).as_bytes());
        file.extend_from_slice(body);
        file.extend_from_slice(b"\nendobj\n");
    }
    let table = file.len();
    file.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        file.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    file.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{table}\n%%EOF\n", objects.len() + 1).as_bytes());
    file
}

/// Put `bytes` at `<root>/<name>`, making its folder.
fn put(root: &Path, name: &str, bytes: impl AsRef<[u8]>) -> Result<PathBuf, String> {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().ok_or("a folder")?).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Lay a Lottie file that embeds an image and names the two `NAMED` out under `root`, and answer it.
fn lottie_in(root: &Path) -> Result<PathBuf, String> {
    for named in NAMED {
        put(root, named, "an image the Lottie file names")?;
    }
    put(root, LOTTIE, lottie(true, true))
}

/// The two ways a bundle's folder may be: one nobody may write, as iOS's is, or one the disk would let
/// anything write but the confinement lets the session only read, as Android's unpacked assets are.
const BUNDLES: [bool; 2] = [true, false];

/// Run `body` with the bundle's folders made read-only, when `read_only` says so, and writable again
/// after.
fn with_bundle<T>(bundle: &Path, read_only: bool, body: impl FnOnce() -> T) -> Result<T, String> {
    let folders: Vec<PathBuf> = std::iter::once(Ok(bundle.to_path_buf()))
        .chain(entries_under(bundle, &[])?.into_iter().map(|entry| Ok(bundle.join(entry))))
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .filter(|path| path.is_dir())
        .collect();
    let mode = |mode| {
        folders.iter().try_for_each(|folder| {
            std::fs::set_permissions(folder, std::os::unix::fs::PermissionsExt::from_mode(mode)).map_err(|e| format!("the permissions are set: {e}"))
        })
    };
    if read_only {
        mode(0o555)?;
    }
    let done = body();
    mode(0o755)?;
    Ok(done)
}

/// The steps of a comp that shows `file` as a footage layer: what a project that shows a vector file holds.
fn showing(file: &Path) -> Vec<(String, Value)> {
    let file = file.display().to_string();
    vec![
        ("comp.new".to_string(), json!({"name": "C", "width": 64, "height": 64, "duration": 1})),
        ("file.import".to_string(), json!({"paths": [file]})),
        ("layer.addItem".to_string(), json!({"comp": 1, "item": 2, "time": 0})),
    ]
}

/// The step that converts the layer of the vector file a comp shows.
fn convert() -> (String, Value) {
    convert_in(json!(1))
}

/// [`convert`], of the first layer of `comp`, named as `layer.create` takes it.
fn convert_in(comp: Value) -> (String, Value) {
    ("layer.create".to_string(), json!({"comp": comp, "op": "shapesFromVector", "layers": [1]}))
}

/// The steps of a vector file's conversion: a comp showing the file, then the conversion of its layer.
fn conversion(file: &Path) -> Vec<(String, Value)> {
    let mut steps = showing(file);
    steps.push(convert());
    steps
}

/// The step of a Lottie file's import.
fn import(file: &Path) -> (String, Value) {
    ("file.importLottie".to_string(), json!({"path": file.display().to_string()}))
}

/// The step that lists the project's footage whose file is not there.
fn missing() -> (String, Value) {
    ("footage.missing".to_string(), json!({}))
}

/// Run `steps` through `run`.
fn run(app: &mut EffectCraftApp, steps: &[(String, Value)]) -> Result<Value, String> {
    app.request("run", json!({"steps": steps_document(steps)})).map_err(|e| e.to_string())
}

/// The files under `folder`, by name, in order.
fn names(folder: &Path) -> Result<Vec<String>, String> {
    let found = entries_under(folder, &[])?;
    let mut files: Vec<String> =
        found.iter().filter(|path| folder.join(path).is_file()).filter_map(|path| path.file_name().map(|name| name.to_string_lossy().into_owned())).collect();
    files.sort();
    Ok(files)
}

/// The folders of a test: a scratch folder with a bundle, the bundle's path and the folder the command
/// line writes in.
fn folders() -> Result<(Scratch, PathBuf, PathBuf), String> {
    let scratch = Scratch::with_bundle().map_err(|e| e.to_string())?;
    let bundle = scratch.bundle().ok_or("a bundle")?.to_path_buf();
    let beside_cli = scratch.root().join("cli");
    std::fs::create_dir(&beside_cli).map_err(|e| e.to_string())?;
    Ok((scratch, bundle, beside_cli))
}

/// Run `mine` on a new app and `theirs` on the command line, whose folder holds the same files as the
/// bundle, with the bundle read-only when `read_only` says so, and answer both results with the
/// bundle's folder and the command line's spelt as one; anything written under the bundle fails it.
fn both(scratch: &Scratch, read_only: bool, mine: &[(String, Value)], theirs: &[(String, Value)]) -> Result<(Value, Value), String> {
    let (bundle, beside_cli) = (scratch.bundle().ok_or("a bundle")?, scratch.root().join("cli"));
    let before = entries_under(bundle, &[])?;
    let mut app = scratch.app().map_err(|e| e.to_string())?;
    let mut answered = with_bundle(bundle, read_only, || run(&mut app, mine))??;
    let written = entries_under(bundle, &[])?;
    if written != before {
        return Err(format!("something was written under the bundle: {written:?}"));
    }
    let mut wanted = cli_run(scratch.root(), theirs)?.doc()?;
    let dirs = [beside_cli.as_path(), bundle, scratch.path()];
    scrub(&mut answered, &dirs);
    scrub(&mut wanted, &dirs);
    Ok((answered, wanted))
}

#[test]
fn a_lottie_file_in_the_bundle_imports_with_every_image_as_the_command_line_imports_it() {
    for read_only in BUNDLES {
        let (scratch, bundle, beside_cli) = folders().expect("the folders");
        let (in_bundle, on_disk) = (lottie_in(&bundle).expect("the files"), lottie_in(&beside_cli).expect("the files"));
        let (mine, wanted) = both(&scratch, read_only, &[import(&in_bundle), missing()], &[import(&on_disk), missing()]).expect("both ran");
        assert_eq!(mine, wanted, "the import answers what the command line's does (read-only {read_only})");
        assert_eq!(mine[0]["result"]["warnings"], json!([]), "no image was dropped");
        assert_eq!(mine[1]["result"]["missing"], json!([]), "every image it names is found where the command line finds it");
        let theirs = names(&beside_cli).expect("the folder reads");
        assert_eq!(theirs.iter().filter(|name| name.ends_with("_image_0.png")).count(), 1, "the command line wrote the image beside the file");
        let staged = names(&scratch.folders.staged).expect("the staged folder reads");
        assert_eq!(staged, ["anim.json", "anim_image_0.png"].map(String::from).to_vec(), "the copy and the image it embeds alone are staged");
    }
}

#[test]
fn a_vector_file_in_the_bundle_creates_its_layers_as_the_command_line_does() {
    for read_only in BUNDLES {
        let (scratch, bundle, beside_cli) = folders().expect("the folders");
        let in_bundle = put(&bundle, "art.pdf", vector(true)).expect("the file");
        let on_disk = put(&beside_cli, "art.pdf", vector(true)).expect("the file");
        let (mine, wanted) = both(&scratch, read_only, &conversion(&in_bundle), &conversion(&on_disk)).expect("both ran");
        assert_eq!(mine, wanted, "the conversion answers what the command line's does (read-only {read_only})");
        assert_eq!(mine[3]["result"]["skipped"], json!([]), "no image was skipped");
        assert!(names(&beside_cli).expect("the folder reads").contains(&"art Image 1.png".to_string()), "the command line wrote the image beside the file");
        let staged = names(&scratch.folders.staged).expect("the staged folder reads");
        assert!(staged.contains(&"art Image 1.png".to_string()), "the image is beside the copy, inside the container");
    }
}

/// One way a call reaches a command: it runs `steps` in a new app, the last step that way and the
/// others before it through `run` when the way takes one command, and answers whether every step ran.
type Way = fn(&mut EffectCraftApp, &[(String, Value)]) -> Result<(), String>;

/// The steps before the last, through `run`, and the last.
fn last_alone<'a>(app: &mut EffectCraftApp, steps: &'a [(String, Value)]) -> Result<&'a (String, Value), String> {
    let (last, before) = steps.split_last().ok_or("a step")?;
    if !before.is_empty() {
        run(app, before)?;
    }
    Ok(last)
}

/// One line of `mcp`, answered as a tool call that did not fail, whose answer is read from its text.
fn tool(app: &mut EffectCraftApp, name: &str, arguments: Value) -> Result<Value, String> {
    tool_sent(app, name, arguments, false)
}

/// [`tool`], with the call sent, when `batched`, as the one message of a batch of lines, which the server
/// answers message by message, in a batch holding the one reply.
fn tool_sent(app: &mut EffectCraftApp, name: &str, arguments: Value, batched: bool) -> Result<Value, String> {
    let call = tools_call(name, &arguments);
    let line = app.mcp(&if batched { json!([call]) } else { call }.to_string()).ok_or("the line is answered")?;
    let answered: Value = serde_json::from_str(&line).map_err(|e| format!("the answer is JSON: {e}"))?;
    let reply = if batched { answered.get(0).cloned().ok_or("a reply to the one message")? } else { answered };
    if reply.pointer("/result/isError") != Some(&json!(false)) {
        return Err(format!("`{name}` failed: {reply}"));
    }
    let text = reply.pointer("/result/content/0/text").and_then(Value::as_str).ok_or("a text answer")?;
    serde_json::from_str(text).map_err(|e| format!("the text is JSON: {e}"))
}

/// The steps as an `engine.batch`'s, in turn in both the spellings with which a step names its command.
fn batch_steps(steps: &[(String, Value)]) -> Value {
    let named = |index: usize, command: &str, params: &Value| {
        let spelling = if index.is_multiple_of(2) { "command" } else { "id" };
        Value::Object([(spelling.to_string(), json!(command)), ("params".to_string(), params.clone())].into_iter().collect())
    };
    Value::Array(steps.iter().enumerate().map(|(index, (command, params))| named(index, command, params)).collect())
}

/// Every way a call reaches a command, by name: on its own and as a step of `run`; as a step of an
/// `engine.batch`, itself on its own and a step of `run`; as a line of `replay`; and through each MCP
/// tool that runs a command by its id, with the parameters given as JSON and as text holding it.
fn ways() -> Vec<(&'static str, Way)> {
    vec![
        ("exec", |app, steps| {
            let (command, params) = last_alone(app, steps)?;
            app.request("exec", json!({"command": command, "params": params})).map(drop).map_err(|e| e.to_string())
        }),
        ("run", |app, steps| run(app, steps).map(drop)),
        ("engine.batch through exec", |app, steps| {
            let batch = json!({"command": "engine.batch", "params": {"steps": batch_steps(steps)}});
            app.request("exec", batch).map(drop).map_err(|e| e.to_string())
        }),
        ("engine.batch as a step of run", |app, steps| run(app, &[("engine.batch".to_string(), json!({"steps": batch_steps(steps)}))]).map(drop)),
        ("replay", |app, steps| {
            let script: Vec<String> = steps.iter().map(|(command, params)| json!({"command": command, "params": params}).to_string()).collect();
            let report = app.request("replay", json!({"script": script.join("\n")})).map_err(|e| e.to_string())?;
            if report.get("ok") == Some(&json!(true)) { Ok(()) } else { Err(format!("the replay failed: {report}")) }
        }),
        ("mcp execute_command", |app, steps| {
            let (command, params) = last_alone(app, steps)?;
            tool(app, "execute_command", json!({"command": command, "params": params})).map(drop)
        }),
        ("mcp command_run, its params as text", |app, steps| {
            let (command, params) = last_alone(app, steps)?;
            tool(app, "command_run", json!({"id": command, "params": params.to_string()})).map(drop)
        }),
        ("mcp command_batch", |app, steps| {
            let answer = tool(app, "command_batch", json!({"steps": steps_document(steps)}))?;
            if answer.get("failed") == Some(&json!(0)) { Ok(()) } else { Err(format!("a step failed: {answer}")) }
        }),
        ("mcp batch, its steps as text", |app, steps| tool(app, "batch", json!({"steps": batch_steps(steps).to_string()})).map(drop)),
    ]
}

/// The names of the project's items.
fn item_names(app: &mut EffectCraftApp) -> Result<Vec<String>, String> {
    let summary = app.request("exec", json!({"command": "project.summary"})).map_err(|e| e.to_string())?;
    let items = summary.get("items").and_then(Value::as_array).ok_or("the items")?;
    Ok(items.iter().filter_map(|item| item.get("name").and_then(Value::as_str).map(str::to_string)).collect())
}

/// Each way a call reaches either command, with the bundle's folder one the disk would let anything
/// write, hands the call the copy: a Lottie file's import makes an item for every image, embedded or
/// named, and finds every one; a vector file imported and converted in one call, or converted in a
/// call of its own, makes the item of its image; the images are beside the copy, and nothing is
/// written under the bundle.
#[test]
fn every_way_a_call_reaches_the_imports_hands_them_the_copy() {
    for (way, through) in ways() {
        for lottie_call in [true, false] {
            let (scratch, bundle, _) = folders().expect("the folders");
            let steps = if lottie_call {
                vec![import(&lottie_in(&bundle).expect("the files"))]
            } else {
                conversion(&put(&bundle, "art.pdf", vector(true)).expect("the file"))
            };
            let before = entries_under(&bundle, &[]).expect("the bundle reads");
            let mut app = scratch.app().expect("the app starts");
            through(&mut app, &steps).unwrap_or_else(|e| panic!("{way}: {e}"));
            assert_eq!(entries_under(&bundle, &[]).expect("the bundle reads"), before, "{way}: nothing was written under the bundle");
            let made = item_names(&mut app).expect("the items");
            let (images, beside_copy) =
                if lottie_call { (vec!["image_0", "image_1", "image_2"], "anim_image_0.png") } else { (vec!["art Image 1.png"], "art Image 1.png") };
            for image in images {
                assert!(made.iter().any(|name| name == image), "{way}: the item of {image} was made: {made:?}");
            }
            let lost = app.request("exec", json!({"command": "footage.missing"})).expect("the footage is checked");
            assert_eq!(lost["missing"], json!([]), "{way}: every image is found");
            assert!(
                names(&scratch.folders.staged).expect("the staged folder reads").contains(&beside_copy.to_string()),
                "{way}: {beside_copy} is beside the copy"
            );
        }
    }
}

/// Two files of one name and the same bytes in two folders of the bundle share one copy, beside which
/// each import writes only what those bytes and that name decide, and each names the images its own
/// folder holds, whether the two are imported by two calls or by one batch: here the second folder
/// lacks the image beside its file, which the command line finds missing as the plugin does. Two vector
/// files alike share one copy too, beside which each conversion writes the same image.
#[test]
fn two_files_of_one_name_and_bytes_share_a_copy_and_each_keeps_its_own_folder() {
    let (scratch, bundle, beside_cli) = folders().expect("the folders");
    let laid = |root: &Path| -> Result<[PathBuf; 2], String> {
        let first = lottie_in(&root.join("a"))?;
        put(&root.join("b"), NAMED[1], "an image the Lottie file names")?;
        Ok([first, put(&root.join("b"), LOTTIE, lottie(true, true))?])
    };
    let (mine_files, their_files) = (laid(&bundle).expect("the files"), laid(&beside_cli).expect("the files"));
    let apart = |files: &[PathBuf; 2]| vec![import(&files[0]), import(&files[1]), missing()];
    let together =
        |files: &[PathBuf; 2]| vec![("engine.batch".to_string(), json!({"steps": steps_document(&[import(&files[0]), import(&files[1])])})), missing()];
    for steps in [apart, together] {
        let mut mine = run(&mut scratch.app().expect("the app starts"), &steps(&mine_files)).expect("the imports ran");
        let mut wanted = cli_run(scratch.root(), &steps(&their_files)).expect("the command line ran").doc().expect("a document");
        let dirs = [beside_cli.as_path(), bundle.as_path(), scratch.path()];
        scrub(&mut mine, &dirs);
        scrub(&mut wanted, &dirs);
        assert_eq!(mine, wanted, "each import names the images of its own folder");
        let lost = mine.as_array().and_then(|steps| steps.last()).map(|last| last["result"]["missing"].clone());
        assert_eq!(lost.as_ref().and_then(Value::as_array).map(Vec::len), Some(1), "the image the second folder lacks is missing");
    }
    let copies = std::fs::read_dir(&scratch.folders.staged).expect("the staged folder reads").count();
    assert_eq!(copies, 1, "one copy for two files of one name and bytes");
    assert_eq!(names(&scratch.folders.staged).expect("the staged folder reads"), ["anim.json", "anim_image_0.png"].map(String::from).to_vec());
    let (scratch, bundle, _) = folders().expect("the folders");
    for folder in ["a", "b"] {
        let file = put(&bundle.join(folder), "art.pdf", vector(true)).expect("the file");
        let answer = run(&mut scratch.app().expect("the app starts"), &conversion(&file)).expect("the conversion ran");
        assert_eq!(answer[3]["result"]["skipped"], json!([]), "no image was skipped");
    }
    assert_eq!(std::fs::read_dir(&scratch.folders.staged).expect("the staged folder reads").count(), 1, "one copy for two vector files alike");
    assert_eq!(names(&scratch.folders.staged).expect("the staged folder reads"), ["art Image 1.png", "art.pdf"].map(String::from).to_vec());
}

#[test]
fn a_source_inside_the_container_is_imported_where_it_is() {
    let (scratch, _, _) = folders().expect("the folders");
    let lottie_file = lottie_in(scratch.path()).expect("the files");
    let vector_file = put(scratch.path(), "art.pdf", vector(true)).expect("the file");
    run(&mut scratch.app().expect("the app starts"), &[import(&lottie_file), missing()]).expect("the import ran");
    run(&mut scratch.app().expect("the app starts"), &conversion(&vector_file)).expect("the conversion ran");
    let beside = names(scratch.path()).expect("the container reads");
    assert!(beside.contains(&"anim_image_0.png".to_string()) && beside.contains(&"art Image 1.png".to_string()), "the images are beside the files: {beside:?}");
    assert_eq!(names(&scratch.folders.staged).expect("the staged folder reads"), Vec::<String>::new(), "nothing was copied");
}

/// A Lottie file in the bundle that embeds no image writes nothing beside itself, so it is read where
/// it is, whether it names images by path, which are then found beside it, or holds none.
#[test]
fn a_lottie_file_that_embeds_no_image_is_read_where_it_is() {
    for names_images in [true, false] {
        let (scratch, bundle, _) = folders().expect("the folders");
        for named in NAMED {
            put(&bundle, named, "an image the Lottie file names").expect("the image");
        }
        let file = put(&bundle, LOTTIE, lottie(false, names_images)).expect("the file");
        let answer = run(&mut scratch.app().expect("the app starts"), &[import(&file), missing()]).expect("the import ran");
        assert_eq!(answer[1]["result"]["missing"], json!([]), "every image it names is found");
        assert_eq!(names(&scratch.folders.staged).expect("the staged folder reads"), Vec::<String>::new(), "nothing was copied");
    }
}

/// A call the confinement refuses as it came is handed no copy: it stays refused, in the
/// confinement's words, and nothing is copied for it. Here the call would also write into the bundle,
/// which a copy put in place of the file it names would turn into a write inside the container.
#[test]
fn a_call_the_confinement_refuses_is_handed_no_copy() {
    let (scratch, bundle, _) = folders().expect("the folders");
    let file = lottie_in(&bundle).expect("the files");
    let path = file.display().to_string();
    let steps = json!([{"command": "file.importLottie", "params": {"path": path}}, {"command": "file.exportLottie", "params": {"comp": 1, "path": path}}]);
    let refused = scratch.app().expect("the app starts").request("exec", json!({"command": "engine.batch", "params": {"steps": steps}}));
    assert!(matches!(refused, Err(CraftError::Io { .. })), "the write into the bundle is refused: {refused:?}");
    assert_eq!(names(&scratch.folders.staged).expect("the staged folder reads"), Vec::<String>::new(), "nothing was copied");
}

/// A conversion of another kind writes nothing beside a vector file, so the bundle's vector footage
/// stays where it is.
#[test]
fn a_conversion_of_another_kind_copies_nothing() {
    let (scratch, bundle, _) = folders().expect("the folders");
    let mut steps = conversion(&put(&bundle, "art.pdf", vector(true)).expect("the file"));
    if let Some((_, params)) = steps.last_mut() {
        params["op"] = json!("shapesFromText");
    }
    let mut app = scratch.app().expect("the app starts");
    drop(run(&mut app, &steps));
    assert_eq!(names(&scratch.folders.staged).expect("the staged folder reads"), Vec::<String>::new(), "nothing was copied");
}

/// An image a Lottie file names by the very path its import writes an embedded image to is that
/// written image, as on the command line, where the write lands on the file the name finds: so the
/// item stays on the image beside the copy, which the bundle, holding no such file, could not give it.
#[test]
fn an_image_named_where_the_import_writes_one_is_the_written_one() {
    let (scratch, bundle, beside_cli) = folders().expect("the folders");
    let file = |root: &Path| {
        let mut doc: Value = serde_json::from_str(&lottie(true, false)).expect("the Lottie file");
        if let Some(assets) = doc.get_mut("assets").and_then(Value::as_array_mut) {
            assets.push(json!({"id": "image_1", "w": 1, "h": 1, "u": "", "p": "anim_image_0.png"}));
        }
        put(root, LOTTIE, doc.to_string()).expect("the file")
    };
    let (in_bundle, on_disk) = (file(&bundle), file(&beside_cli));
    let (mine, wanted) = both(&scratch, false, &[import(&in_bundle), missing()], &[import(&on_disk), missing()]).expect("both ran");
    assert_eq!(mine, wanted, "the import answers what the command line's does");
    assert_eq!(mine[1]["result"]["missing"], json!([]), "the named image is the written one");
}

/// A project saved after a bundle file's import reopens with every image: the repoint of the
/// external images to the files beside the bundle's file is not just for the call that ran, but
/// for the project as it is kept — a fresh app that opens the saved file finds the external
/// images beside the file in the bundle and the embedded one beside the copy.
#[test]
fn a_project_saved_after_a_bundled_import_reopens_with_every_image() {
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let in_bundle = lottie_in(&bundle).expect("the file");
        let saved = scratch.path().join("saved.ecproj");
        let mut app = scratch.app().expect("the app starts");
        with_bundle(&bundle, read_only, || run(&mut app, &[import(&in_bundle), ("file.save".to_string(), json!({"path": saved.display().to_string()}))]))
            .expect("the import and save ran")
            .expect("the steps ran");
        let open = ("file.open".to_string(), json!({"path": saved.display().to_string()}));
        let mut reopened = scratch.app().expect("a fresh app");
        let answer = run(&mut reopened, &[open, missing()]).expect("the project reopened");
        assert_eq!(answer[1]["result"]["missing"], json!([]), "the reopened project finds every image (read-only {read_only})");
        let made = item_names(&mut reopened).expect("the items");
        for image in ["image_0", "image_1", "image_2"] {
            assert!(made.iter().any(|name| name == image), "the reopened project holds the item of {image}: {made:?}");
        }
    }
}

/// The import is one undoable edit, and the repoint of its external images rides with the items
/// themselves: undoing the import leaves no footage item naming a copy, whose images a later purge
/// of the staged folder could remove, and redoing it finds every image again — the external ones
/// beside the file in the bundle, the embedded one beside the copy.
#[test]
fn undoing_a_bundled_import_leaves_no_item_naming_a_copy() {
    let (scratch, bundle, _) = folders().expect("the folders");
    let in_bundle = lottie_in(&bundle).expect("the file");
    let mut app = scratch.app().expect("the app starts");
    let answer = run(&mut app, &[import(&in_bundle), missing()]).expect("the import ran");
    assert_eq!(answer[1]["result"]["missing"], json!([]), "before the undo every image is found");
    app.request("exec", json!({"command": "edit.undo"})).expect("the import is one undo step");
    let after = run(&mut app, &[missing()]).expect("the footage is checked");
    assert_eq!(after[0]["result"]["missing"], json!([]), "no item is left naming a copy a later purge could remove");
    let made = item_names(&mut app).expect("the items");
    for image in ["image_0", "image_1", "image_2"] {
        assert!(!made.iter().any(|name| name == image), "the import's item of {image} is undone: {made:?}");
    }
    app.request("exec", json!({"command": "edit.redo"})).expect("the import is one redo step");
    let after = run(&mut app, &[missing()]).expect("the footage is checked again");
    assert_eq!(after[0]["result"]["missing"], json!([]), "after the redo every image is found where it was before the undo");
}

/// The repoint takes each shared copy's items by the path beside the copy, which only the import
/// that wrote that path can have made: an earlier call's items, naming the files of their own
/// folder, are left to name them. Here the first folder holds every image and the second none,
/// so the second import's own images are the only ones missing.
#[test]
fn a_later_import_repoints_only_the_items_its_call_made() {
    let (scratch, bundle, _) = folders().expect("the folders");
    let first = lottie_in(&bundle.join("a")).expect("the files");
    let second = put(&bundle.join("b"), LOTTIE, lottie(true, true)).expect("the file");
    let mut app = scratch.app().expect("the app starts");
    run(&mut app, &[import(&first)]).expect("the first import ran");
    let before = run(&mut app, &[missing()]).expect("the footage is checked");
    assert_eq!(before[0]["result"]["missing"], json!([]), "the first call's items name files that are there");
    let answer = run(&mut app, &[import(&second), missing()]).expect("the second import ran");
    let mut lost: Vec<String> =
        answer[1]["result"]["missing"].as_array().expect("the missing list").iter().filter_map(|image| image["path"].as_str().map(str::to_string)).collect();
    lost.sort();
    let mut wanted: Vec<String> =
        [bundle.join("b").join(NAMED[0]), bundle.join("b").join(NAMED[1])].into_iter().map(|path| path.display().to_string()).collect();
    wanted.sort();
    assert_eq!(lost, wanted, "only the second call's own images are missing, beside its own file; the first call's items still name theirs");
}

/// The steps of one call that opens `project` and converts the layer of the vector file it shows.
fn open_and_convert(project: &Path) -> Vec<(String, Value)> {
    vec![("file.open".to_string(), json!({"path": project.display().to_string()})), convert()]
}

/// The steps that make a comp showing `file`, as it was saved before a later call opened it.
fn save_showing(file: &Path, project: &Path) -> Vec<(String, Value)> {
    let mut steps = showing(file);
    steps.push(("file.save".to_string(), json!({"path": project.display().to_string()})));
    steps
}

/// A project inside the container that shows a vector file from the bundle, saved by an app of `scratch`
/// and left where it lies, so a later call has to open it.
fn project_showing(scratch: &Scratch, bundle: &Path) -> Result<PathBuf, String> {
    let file = put(bundle, "art.pdf", vector(true))?;
    let project = scratch.path().join("shows.ecproj");
    let mut app = scratch.app().map_err(|e| e.to_string())?;
    run(&mut app, &save_showing(&file, &project)).map(drop)?;
    Ok(project)
}

/// One way a single call reaches both the open and the conversion: it answers the words the call refused
/// with, or that it ran instead.
type Refusal = fn(&mut EffectCraftApp, &[(String, Value)]) -> Result<String, String>;

/// The words a method call refused with, or that it ran instead.
fn refused(answer: Result<Value, CraftError>) -> Result<String, String> {
    match answer {
        Ok(answer) => Err(format!("the call ran: {answer}")),
        Err(error) => Ok(error.to_string()),
    }
}

/// The words a tool call refused with, or that it ran instead.
fn tool_refused(answer: Result<Value, String>) -> Result<String, String> {
    match answer {
        Ok(answer) => Err(format!("the call ran: {answer}")),
        Err(words) => Ok(words),
    }
}

/// The words an `engine.batch` of `steps` is refused with when `app` runs it through `exec`, with the
/// bundle read-only when `read_only` says so, or that it ran instead.
fn batch_refused(app: &mut EffectCraftApp, bundle: &Path, read_only: bool, steps: &[(String, Value)]) -> Result<String, String> {
    let call = json!({"command": "engine.batch", "params": {"steps": batch_steps(steps)}});
    with_bundle(bundle, read_only, || refused(app.request("exec", call)))?
}

/// The words the call refuses with that reads a project into the session with `command` and `params`, in a
/// new app after the steps `earlier` that make the command able to run, and then converts.
fn refused_after(scratch: &Scratch, bundle: &Path, read_only: bool, earlier: &[(String, Value)], reads: (&str, Value)) -> Result<String, String> {
    let mut app = scratch.app().map_err(|e| e.to_string())?;
    if !earlier.is_empty() {
        run(&mut app, earlier).map_err(|e| format!("`{}`: {e}", reads.0))?;
    }
    batch_refused(&mut app, bundle, read_only, &[(reads.0.to_string(), reads.1), convert()])
}

/// The ways one call reaches both an open and a conversion, by name: an `engine.batch` through `exec`
/// and as a step of `run`; each MCP tool that runs a list of commands, whose steps the tool runs itself,
/// and one sent in a batch of lines, each message of which reaches the door on its own; and a `replay`,
/// whose `start` project the script opens before its first line.
fn open_ways() -> Vec<(&'static str, Refusal)> {
    vec![
        ("engine.batch through exec", |app, steps| refused(app.request("exec", json!({"command": "engine.batch", "params": {"steps": batch_steps(steps)}})))),
        ("engine.batch as a step of run", |app, steps| {
            refused(app.request("run", json!({"steps": [{"command": "engine.batch", "params": {"steps": batch_steps(steps)}}]})))
        }),
        ("mcp command_batch", |app, steps| tool_refused(tool(app, "command_batch", json!({"steps": steps_document(steps)})))),
        ("mcp batch, its steps as text", |app, steps| tool_refused(tool(app, "batch", json!({"steps": batch_steps(steps).to_string()})))),
        ("mcp command_batch, in a batch of lines", |app, steps| tool_refused(tool_sent(app, "command_batch", json!({"steps": steps_document(steps)}), true))),
        ("replay", |app, steps| {
            let (open, rest) = steps.split_first().ok_or("a step")?;
            let mut script = vec![json!({"start": {"project": open.1.get("path")}}).to_string()];
            script.extend(rest.iter().map(|(command, params)| json!({"command": command, "params": params}).to_string()));
            refused(app.request("replay", json!({"script": script.join("\n")})))
        }),
    ]
}

/// A call that converts vector footage of the very project it opens cannot be handed a copy of what that
/// project shows, which lies in the folder the session may read and never write: every way one call
/// reaches both is refused, its words naming the call that works, and nothing is copied or written under
/// the bundle.
#[test]
fn a_call_that_opens_a_project_and_converts_what_it_shows_names_the_call_that_works() {
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let project = project_showing(&scratch, &bundle).expect("the project");
        let before = entries_under(&bundle, &[]).expect("the bundle reads");
        for (way, through) in open_ways() {
            let mut app = scratch.app().expect("the app starts");
            let words = with_bundle(&bundle, read_only, || through(&mut app, &open_and_convert(&project)))
                .expect("the call is answered")
                .unwrap_or_else(|e| panic!("{way}: {e}"));
            assert!(words.contains("`layer.create` is not available in this build"), "{way}: the refusal says what is not available: {words}");
            assert!(words.contains("open or import in one call and convert in the next"), "{way}: the refusal names the call that works: {words}");
            assert_eq!(names(&scratch.folders.staged).expect("the staged folder reads"), Vec::<String>::new(), "{way}: nothing was copied");
        }
        assert_eq!(entries_under(&bundle, &[]).expect("the bundle reads"), before, "nothing was written under the bundle (read-only {read_only})");
    }
}

/// The call the refusal names works: the open in one call and the conversion in the next is handed the
/// copy, writes every image the conversion makes inside the container, and answers what the command
/// line's same two calls do.
#[test]
fn a_conversion_in_a_call_after_the_open_is_handed_the_copy() {
    for read_only in BUNDLES {
        let (scratch, bundle, beside_cli) = folders().expect("the folders");
        let project = project_showing(&scratch, &bundle).expect("the project");
        let on_disk = put(&beside_cli, "art.pdf", vector(true)).expect("the file");
        let saved = beside_cli.join("shows.ecproj");
        cli_run(scratch.root(), &save_showing(&on_disk, &saved)).expect("the command line saved its project");
        let (mine, wanted) = both(&scratch, read_only, &open_and_convert(&project), &open_and_convert(&saved)).expect("both ran");
        assert_eq!(mine, wanted, "the open and the conversion answer what the command line's do (read-only {read_only})");
        assert_eq!(mine[1]["result"]["skipped"], json!([]), "no image was skipped");
        let staged = names(&scratch.folders.staged).expect("the staged folder reads");
        assert!(staged.contains(&"art Image 1.png".to_string()), "the image lies beside the copy, inside the container: {staged:?}");
    }
}

/// An open that comes after the conversion is no reason to refuse the call: what the conversion converts
/// was in the project before the call, so it is handed its copy as every other conversion is.
#[test]
fn a_conversion_before_the_open_is_handed_the_copy() {
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let file = put(&bundle, "art.pdf", vector(true)).expect("the file");
        let project = scratch.path().join("shows.ecproj");
        let mut app = scratch.app().expect("the app starts");
        run(&mut app, &save_showing(&file, &project)).expect("the project is saved and still open");
        let mut steps = open_and_convert(&project);
        steps.swap(0, 1);
        let call = json!({"command": "engine.batch", "params": {"steps": batch_steps(&steps)}});
        with_bundle(&bundle, read_only, || app.request("exec", call)).expect("the call is answered").expect("the call ran");
        let staged = names(&scratch.folders.staged).expect("the staged folder reads");
        assert!(staged.contains(&"art Image 1.png".to_string()), "the image lies beside the copy, inside the container: {staged:?}");
    }
}

/// A project whose footage lies where the session may write has nothing to copy, so the same call that
/// opens it and converts what it shows runs, on an app that ships a bundle as on one that ships none: it
/// answers what the command line answers and writes its image beside the file it read, exactly where the
/// command line writes it.
#[test]
fn a_call_with_nothing_to_copy_runs_as_the_command_line_runs_it() {
    for scratch in [Scratch::new(), Scratch::with_bundle()] {
        let scratch = scratch.expect("the folders");
        let file = put(scratch.path(), "art.pdf", vector(true)).expect("the file");
        let project = scratch.path().join("shows.ecproj");
        cli_run(scratch.path(), &save_showing(&file, &project)).expect("the command line saved a project");
        let steps = vec![("engine.batch".to_string(), json!({"steps": batch_steps(&open_and_convert(&project))}))];
        let mut app = scratch.app().expect("the app starts");
        let mut mine = run(&mut app, &steps).expect("the call ran");
        let mut wanted = cli_run(scratch.path(), &steps).expect("the command line ran it").doc().expect("a document");
        scrub(&mut mine, &[scratch.path()]);
        scrub(&mut wanted, &[scratch.path()]);
        let shipped = scratch.bundle().is_some();
        assert_eq!(mine, wanted, "the call that opens a project and converts what it shows answers what the command line's does (bundle {shipped})");
        assert!(names(scratch.path()).expect("the folder reads").contains(&"art Image 1.png".to_string()), "the image was written beside the file it read");
    }
}

/// Each command that reads a document into the session, with the parameters that have it read the saved
/// `project`, and the steps an earlier call takes so that it does: `file.openRecent` is there to run only
/// once the session has opened a project, `file.revert` reads the project the session has open again,
/// and `file.newProject` the template project the settings name.
fn openings(project: &Path) -> Vec<(&'static str, Value, Vec<(String, Value)>)> {
    let path = project.display().to_string();
    let named = json!({"path": path});
    let opened = vec![("file.open".to_string(), named.clone())];
    let template = json!({"values": {"project.useTemplate": true, "project.templatePath": path}});
    vec![
        ("file.open", named.clone(), Vec::new()),
        ("file.openRecent", named.clone(), opened.clone()),
        ("file.revert", json!({}), opened),
        ("file.newProject", json!({}), vec![("prefs.set".to_string(), template)]),
        ("file.importProject", named, Vec::new()),
    ]
}

/// Reading into the session a project, or another document's footage, brings what it shows from its file,
/// so a conversion after any command that does is refused when what it brings in lies where the session
/// may read and never write: it is the command that reads a file into the session that the door cannot
/// see past, not the one that replaces the project whole. `file.importAep` is held to the rule by the
/// door's own unit test: EffectCraft's export writes no vector footage into an After Effects project, so
/// no such file can be made here.
#[test]
fn a_conversion_after_any_command_that_reads_a_project_is_refused() {
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let project = project_showing(&scratch, &bundle).expect("the project");
        for (command, params, earlier) in openings(&project) {
            let words = refused_after(&scratch, &bundle, read_only, &earlier, (command, params)).unwrap_or_else(|e| panic!("`{command}`: {e}"));
            assert!(words.contains("open or import in one call and convert in the next"), "`{command}`: the refusal names the call that works: {words}");
        }
        assert_eq!(names(&scratch.folders.staged).expect("the staged folder reads"), Vec::<String>::new(), "nothing was copied");
    }
}

/// What a document read in by the call brings in is told apart from what the project showed before the
/// call: a project imported beside bundled footage the session already shows, whose own footage lies
/// where the session may write, needs no copy, so the call runs, and the conversion of the bundled
/// footage is handed its copy as every other is.
#[test]
fn an_import_that_brings_in_nothing_to_copy_runs_beside_footage_that_needs_one() {
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let project = project_showing(&scratch, &bundle).expect("the project");
        let beside = scratch.path().join("beside");
        let imported = beside.join("beside.ecproj");
        run(&mut scratch.app().expect("the app starts"), &save_showing(&put(&beside, "own.pdf", vector(true)).expect("the file"), &imported))
            .expect("the project to import is saved");
        let mut app = scratch.app().expect("the app starts");
        let mut steps = open_and_convert(&project);
        run(&mut app, steps.get(..1).unwrap_or_default()).expect("the project is open");
        steps[0] = ("file.importProject".to_string(), json!({"path": imported.display().to_string()}));
        let call = json!({"command": "engine.batch", "params": {"steps": batch_steps(&steps)}});
        let answer = with_bundle(&bundle, read_only, || app.request("exec", call)).expect("the call is answered").expect("the call ran");
        assert_eq!(answer["results"][1]["skipped"], json!([]), "no image was skipped (read-only {read_only})");
        let staged = names(&scratch.folders.staged).expect("the staged folder reads");
        assert!(staged.contains(&"art Image 1.png".to_string()), "the image lies beside the copy, inside the container: {staged:?}");
    }
}

/// A conversion after the open is refused though another comes before it: the one before converts what
/// the project showed before the call, the one after what the open brought in.
#[test]
fn a_conversion_after_the_open_is_refused_though_another_comes_before_it() {
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let project = project_showing(&scratch, &bundle).expect("the project");
        let mut app = scratch.app().expect("the app starts");
        let mut steps = open_and_convert(&project);
        run(&mut app, steps.get(..1).unwrap_or_default()).expect("the project is open, so the first conversion has a layer to convert");
        steps.insert(0, convert());
        let words = batch_refused(&mut app, &bundle, read_only, &steps).unwrap_or_else(|e| panic!("{e}"));
        assert!(words.contains("open or import in one call and convert in the next"), "the refusal names the call that works: {words}");
    }
}

/// What a command reads into the session can turn on an earlier step of the same call: here a change of
/// the settings names the template a new project loads. The door cannot read that before the call, so
/// the call is refused whatever the new project would show.
#[test]
fn an_open_an_earlier_step_decides_is_refused() {
    let (scratch, bundle, _) = folders().expect("the folders");
    let project = project_showing(&scratch, &bundle).expect("the project");
    let (command, params, mut steps) = openings(&project).into_iter().find(|(command, ..)| *command == "file.newProject").expect("the new project");
    steps.extend([(command.to_string(), params), convert()]);
    let words = batch_refused(&mut scratch.app().expect("the app starts"), &bundle, false, &steps).unwrap_or_else(|e| panic!("{e}"));
    assert!(words.contains("open or import in one call and convert in the next"), "the refusal names the call that works: {words}");
}

/// A step of `run` is refused as every other row of a `run` is: the command it names, then its failure,
/// then the step it is, so a caller reading a `run`'s failure knows which row to split.
#[test]
fn a_step_that_opens_a_project_and_converts_what_it_shows_is_refused_as_a_step() {
    let (scratch, bundle, _) = folders().expect("the folders");
    let project = project_showing(&scratch, &bundle).expect("the project");
    let batch = json!({"steps": batch_steps(&open_and_convert(&project))});
    let words = refused(scratch.app().expect("the app starts").request("run", json!({"steps": [{"command": "engine.batch", "params": batch}]})))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(words.starts_with("engine.batch: "), "the refusal names the row that failed: {words}");
    assert!(words.ends_with("(step 0)"), "the refusal gives the row's number: {words}");
}

/// A call the confinement refuses keeps the confinement's own words even when it is also a call that opens
/// a project and converts what it shows: the door's advice is for a call this build could run at all.
/// It holds where the door reads the document first, and where the call's own steps leave nothing to read:
/// here a second document read in before the conversion.
#[test]
fn a_call_the_confinement_refuses_is_not_told_to_split_its_open_from_its_conversion() {
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let project = project_showing(&scratch, &bundle).expect("the project");
        let outside = ("file.open".to_string(), json!({"path": scratch.outside_file().expect("a file outside the container")}));
        let mut read_first = open_and_convert(&project);
        read_first[0] = outside.clone();
        let mut undecided = open_and_convert(&project);
        undecided.insert(0, outside);
        for steps in [read_first, undecided] {
            let mut app = scratch.app().expect("the app starts");
            let call = json!({"command": "engine.batch", "params": {"steps": batch_steps(&steps)}});
            let refused = with_bundle(&bundle, read_only, || app.request("exec", call)).expect("the call is answered");
            assert!(matches!(refused, Err(CraftError::Io { .. })), "the confinement answers in its own words: {refused:?}");
        }
        assert_eq!(names(&scratch.folders.staged).expect("the staged folder reads"), Vec::<String>::new(), "nothing was copied");
    }
}

/// Each way an import is given a bundled project: `file.import`, `file.importMultiple` and
/// `file.importRecent`, which the confinement has name its path and which imports it through `file.import`
/// as the others do, after an earlier call's own import made the project a recent file it can import.
/// `essential.importTemplate` reads its own kind of file, a template, which a test of its own makes.
fn project_imports(project: &Path) -> Vec<(&'static str, Value, Vec<(String, Value)>)> {
    let path = project.display().to_string();
    let imported = vec![("file.import".to_string(), json!({"paths": [path]}))];
    vec![
        ("file.import", json!({"paths": [path]}), Vec::new()),
        ("file.importMultiple", json!({"paths": [path]}), Vec::new()),
        ("file.importRecent", json!({"path": path}), imported),
    ]
}

/// EffectCraft sends a project given to an import to `file.importProject` inside the command, so the
/// footage it shows is in no string of the call: a conversion after such an import is refused, naming the
/// call that works, with nothing copied or written under the bundle.
#[test]
fn a_conversion_after_an_import_of_a_bundled_project_is_refused() {
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let project = project_showing(&scratch, &bundle).expect("the project");
        let before = entries_under(&bundle, &[]).expect("the bundle reads");
        for (command, params, earlier) in project_imports(&project) {
            let words =
                refused_after(&scratch, &bundle, read_only, &earlier, (command, params.clone())).unwrap_or_else(|e| panic!("`{command}` {params}: {e}"));
            assert!(words.contains("`layer.create` is not available in this build in a call that opens or imports a project"), "`{command}`: {words}");
            assert!(words.contains("open or import in one call and convert in the next"), "`{command}`: the refusal names the call that works: {words}");
        }
        assert_eq!(names(&scratch.folders.staged).expect("the staged folder reads"), Vec::<String>::new(), "nothing was copied");
        assert_eq!(entries_under(&bundle, &[]).expect("the bundle reads"), before, "nothing was written under the bundle (read-only {read_only})");
    }
}

/// An import that names the vector file itself shows footage the call names, which the door hands a copy of
/// as it always did: the call that imports it and converts it runs and writes its images inside the
/// container, as the command line's does, through each command that imports a path.
#[test]
fn a_conversion_after_an_import_that_names_the_file_is_handed_the_copy() {
    for read_only in BUNDLES {
        for command in ["file.import", "file.importMultiple"] {
            let (scratch, bundle, beside_cli) = folders().expect("the folders");
            let (in_bundle, on_disk) = (put(&bundle, "art.pdf", vector(true)).expect("the file"), put(&beside_cli, "art.pdf", vector(true)).expect("the file"));
            let importing = |file: &Path| {
                let mut steps = conversion(file);
                if let Some(step) = steps.get_mut(1) {
                    step.0 = command.to_string();
                }
                vec![("engine.batch".to_string(), json!({"steps": batch_steps(&steps)}))]
            };
            let (mine, wanted) = both(&scratch, read_only, &importing(&in_bundle), &importing(&on_disk)).expect("both ran");
            assert_eq!(mine, wanted, "`{command}` and the conversion answer what the command line's do (read-only {read_only})");
            assert_eq!(mine[0]["result"]["results"][3]["skipped"], json!([]), "no image was skipped");
            let staged = names(&scratch.folders.staged).expect("the staged folder reads");
            assert!(staged.contains(&"art Image 1.png".to_string()), "`{command}`: the image is beside the copy, inside the container: {staged:?}");
        }
    }
}

/// A project that shows no footage the session may not write, imported by a call that converts the
/// vector file it shows after, is no reason to refuse: only footage that needs a copy counts, so the call
/// runs and the file it names is handed its copy.
#[test]
fn an_import_of_a_project_with_nothing_to_copy_runs_beside_footage_that_needs_one() {
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let file = put(&bundle, "art.pdf", vector(true)).expect("the file");
        let own = put(&scratch.path().join("beside"), "own.pdf", vector(true)).expect("the file");
        let beside = scratch.path().join("beside").join("beside.ecproj");
        run(&mut scratch.app().expect("the app starts"), &save_showing(&own, &beside)).expect("the project is saved");
        let mut steps = conversion(&file);
        steps.insert(3, ("file.import".to_string(), json!({"paths": [beside.display().to_string()]})));
        let mut app = scratch.app().expect("the app starts");
        let call = json!({"command": "engine.batch", "params": {"steps": batch_steps(&steps)}});
        let answer = with_bundle(&bundle, read_only, || app.request("exec", call)).expect("the call is answered").expect("the call ran");
        assert!(
            names(&scratch.folders.staged).expect("the staged folder reads").contains(&"art Image 1.png".to_string()),
            "the named file's image is inside the container"
        );
        assert!(answer.to_string().contains("\"skipped\":[]"), "no image was skipped: {answer}");
    }
}

/// An import whose parameters hold a reference to an earlier step's result cannot be read before the call
/// runs, so it may import a project: the call is refused, as an open an earlier step decides is.
#[test]
fn an_import_an_earlier_step_decides_is_refused() {
    let (scratch, bundle, _) = folders().expect("the folders");
    let file = put(&bundle, "art.pdf", vector(true)).expect("the file");
    let mut steps = conversion(&file);
    if let Some(step) = steps.get_mut(1) {
        step.1 = json!({"paths": [file.display().to_string()], "page": "$1.page"});
    }
    let words = batch_refused(&mut scratch.app().expect("the app starts"), &bundle, false, &steps).unwrap_or_else(|e| panic!("{e}"));
    assert!(words.contains("open or import in one call and convert in the next"), "the refusal names the call that works: {words}");
}

/// What an import brings in from a project lies in no file the call names, even where the same import
/// also names the file that footage shows, and even where only an earlier step of the same call, an
/// import or a change of the settings, lets `file.importRecent` run at all: each such call is refused,
/// naming the call that works, and nothing is written under the bundle. The conversion names the
/// project's own comp, so a call let through would convert the layer the project shows.
#[test]
fn a_conversion_after_an_import_of_a_project_is_refused_whatever_else_the_call_imports() {
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let project = project_showing(&scratch, &bundle).expect("the project").display().to_string();
        let art = bundle.join("art.pdf").display().to_string();
        let other = put(scratch.path(), "other.csv", "a,b\n1,2\n").expect("a file to import").display().to_string();
        let before = entries_under(&bundle, &[]).expect("the bundle reads");
        let calls = [
            vec![("file.import", json!({"paths": [project, art]}))],
            vec![("file.importMultiple", json!({"paths": [art, project]}))],
            vec![("file.import", json!({"paths": [other]})), ("file.importRecent", json!({"path": project}))],
            vec![("prefs.set", json!({"key": "recentFootage", "value": [other]})), ("file.importRecent", json!({"path": project}))],
        ];
        for reads in calls {
            let mut steps: Vec<(String, Value)> = reads.into_iter().map(|(command, params)| (command.to_string(), params)).collect();
            steps.push(convert_in(json!("C")));
            let words = batch_refused(&mut scratch.app().expect("the app starts"), &bundle, read_only, &steps).unwrap_or_else(|e| panic!("{steps:?}: {e}"));
            assert!(words.contains("in a call that opens or imports a project"), "{steps:?}: {words}");
            assert!(words.contains("open or import in one call and convert in the next"), "{steps:?}: the refusal names the call that works: {words}");
        }
        assert_eq!(entries_under(&bundle, &[]).expect("the bundle reads"), before, "nothing was written under the bundle (read-only {read_only})");
    }
}

/// A template exported while its footage could not be read keeps naming that footage where it lay, here
/// a vector file in the bundle, so its import brings in footage no file of the call names, as a project's
/// does: a conversion after it is refused, naming the call that works, with nothing copied or written
/// under the bundle.
#[test]
fn a_conversion_after_an_import_of_a_template_showing_bundled_footage_is_refused() {
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let art = put(&bundle, "art.pdf", vector(true)).expect("the file");
        let template = scratch.path().join("shows.ectemplate").display().to_string();
        let mut app = scratch.app().expect("the app starts");
        let shown = run(&mut app, &showing(&art)).expect("the comp shows the file");
        let control = json!({"comp": 1, "layer": shown[2]["result"]["layer"], "path": "transform/opacity", "name": "Opacity"});
        run(&mut app, &[("essential.addProperty".to_string(), control)]).expect("the comp has a control to export");
        let away = bundle.join("away.pdf");
        std::fs::rename(&art, &away).expect("the file is moved away");
        let exported = run(&mut app, &[("essential.exportTemplate".to_string(), json!({"comp": 1, "path": template}))]);
        std::fs::rename(&away, &art).expect("the file is back");
        exported.expect("the template is exported, its footage left named where it lay");
        let before = entries_under(&bundle, &[]).expect("the bundle reads");
        let steps = [("essential.importTemplate".to_string(), json!({"path": template, "addToComp": false})), convert_in(json!("C"))];
        let words = batch_refused(&mut scratch.app().expect("the app starts"), &bundle, read_only, &steps).unwrap_or_else(|e| panic!("{e}"));
        assert!(words.contains("open or import in one call and convert in the next"), "the refusal names the call that works: {words}");
        assert_eq!(names(&scratch.folders.staged).expect("the staged folder reads"), Vec::<String>::new(), "nothing was copied");
        assert_eq!(entries_under(&bundle, &[]).expect("the bundle reads"), before, "nothing was written under the bundle (read-only {read_only})");
    }
}

/// An import the scratch session cannot run leaves the imports after it to be read all the same, since
/// a call may still run them: `command_batch`, told not to stop at an error, runs every step, so a project
/// an import brings in after one that fails is refused as any other is.
#[test]
fn an_import_after_one_that_fails_is_still_read() {
    let (scratch, bundle, _) = folders().expect("the folders");
    let project = project_showing(&scratch, &bundle).expect("the project").display().to_string();
    let other = put(scratch.path(), "other.csv", "a,b\n1,2\n").expect("a file to import").display().to_string();
    let steps = [
        ("file.import".to_string(), json!({"paths": [other], "importAs": "neither"})),
        ("file.import".to_string(), json!({"paths": [project]})),
        convert_in(json!("C")),
    ];
    let before = entries_under(&bundle, &[]).expect("the bundle reads");
    let arguments = json!({"steps": steps_document(&steps), "stop_on_error": false});
    let words = tool_refused(tool(&mut scratch.app().expect("the app starts"), "command_batch", arguments)).unwrap_or_else(|e| panic!("{e}"));
    assert!(words.contains("open or import in one call and convert in the next"), "the refusal names the call that works: {words}");
    assert_eq!(entries_under(&bundle, &[]).expect("the bundle reads"), before, "nothing was written under the bundle");
}

/// A folder's files are named by no string of the call, as a project's footage is not: a conversion after
/// an import of a folder of the bundle that holds a vector file is refused, in words that name both and
/// the call that works, with nothing copied or written under the bundle. The import finds the file in the
/// folder only through the session's importer, which the scratch session shares.
#[test]
fn a_conversion_after_an_import_of_a_bundled_folder_is_refused() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="red"/></svg>"#;
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let folder = bundle.join("art");
        put(&folder, "art.svg", svg).expect("the file");
        let before = entries_under(&bundle, &[]).expect("the bundle reads");
        let words = batch_refused(&mut scratch.app().expect("the app starts"), &bundle, read_only, &conversion(&folder)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            words,
            "`layer.create` is not available in this build in a call that opens or imports a project or a folder: what it brings in is not there before the call, so no copy can be handed over of a file it shows in a folder this build may read and never write, beside which its images would land; open or import in one call and convert in the next"
        );
        assert_eq!(names(&scratch.folders.staged).expect("the staged folder reads"), Vec::<String>::new(), "nothing was copied");
        assert_eq!(entries_under(&bundle, &[]).expect("the bundle reads"), before, "nothing was written under the bundle (read-only {read_only})");
    }
}

/// A template that holds a medium, made inside the container and put at `<root>/<folder>/card.ectemplate`,
/// where `root` is a bundle's or the command line's folder: its path there.
fn template_in(scratch: &Scratch, root: &Path, folder: &str) -> Result<PathBuf, String> {
    let made = scratch.path().join(FILE);
    if !made.is_file() {
        make(scratch, (96, 64), 1.0, "Arial")?;
    }
    put(root, &format!("{folder}/{FILE}"), std::fs::read(&made).map_err(|e| e.to_string())?)
}

/// The step that imports the template at `file` and adds it to no comp.
fn import_template(file: &Path) -> (String, Value) {
    ("essential.importTemplate".to_string(), json!({"path": file.display().to_string(), "addToComp": false}))
}

/// Every file under `folder` with its bytes, by its path under `folder`, in order.
fn contents(folder: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let mut found = Vec::new();
    for entry in entries_under(folder, &[])? {
        if folder.join(&entry).is_file() {
            found.push((entry.clone(), std::fs::read(folder.join(&entry)).map_err(|e| e.to_string())?));
        }
    }
    Ok(found)
}

/// The folder of the one copy the staged folder holds.
fn the_copy(scratch: &Scratch) -> Result<PathBuf, String> {
    let copies: Vec<PathBuf> = std::fs::read_dir(&scratch.folders.staged).map_err(|e| e.to_string())?.flatten().map(|entry| entry.path()).collect();
    match copies.as_slice() {
        [copy] => Ok(copy.clone()),
        other => Err(format!("one copy was made: {other:?}")),
    }
}

/// A template in the bundle, imported plainly, has every medium it holds written inside the container,
/// beside its copy, as the command line writes them beside the template in a writable folder: the same
/// answer, the same media file for file with the same bytes, every item found, and nothing written under
/// the bundle, whether the disk would refuse the write (iOS, where the media would come out missing) or
/// take it (Android, where they would escape the confinement).
#[test]
fn a_template_in_the_bundle_imports_with_every_medium_inside_the_container_as_the_command_line_imports_it() {
    for read_only in BUNDLES {
        let (scratch, bundle, beside_cli) = folders().expect("the folders");
        let (in_bundle, on_disk) =
            (template_in(&scratch, &bundle, "tpl").expect("the template"), template_in(&scratch, &beside_cli, "tpl").expect("the template"));
        let steps = |file: &Path| vec![import_template(file), missing()];
        let (mine, wanted) = both(&scratch, read_only, &steps(&in_bundle), &steps(&on_disk)).expect("both ran");
        assert_eq!(mine, wanted, "the import answers what the command line's does (read-only {read_only})");
        assert_eq!(mine[1]["result"]["missing"], json!([]), "every medium is found (read-only {read_only})");
        let copy = the_copy(&scratch).expect("the copy");
        let (ours, theirs) = (contents(&copy).expect("the copy's folder reads"), contents(&beside_cli.join("tpl")).expect("the command line's folder reads"));
        assert!(
            ours.iter().any(|(path, _)| path.starts_with("card Media")),
            "the media are beside the copy: {:?}",
            ours.iter().map(|(path, _)| path).collect::<Vec<_>>()
        );
        assert_eq!(ours, theirs, "the template and its media, file for file, as the command line writes them (read-only {read_only})");
    }
}

/// Every way a call reaches the template's import hands it the copy: the media are inside the container,
/// every item is found, and nothing is written under the bundle.
#[test]
fn every_way_a_call_reaches_the_template_import_hands_it_the_copy() {
    for (way, through) in ways() {
        let (scratch, bundle, _) = folders().expect("the folders");
        let template = template_in(&scratch, &bundle, "tpl").expect("the template");
        let before = entries_under(&bundle, &[]).expect("the bundle reads");
        let mut app = scratch.app().expect("the app starts");
        with_bundle(&bundle, true, || through(&mut app, &[import_template(&template)]))
            .expect("the bundle is read-only")
            .unwrap_or_else(|e| panic!("{way}: {e}"));
        assert_eq!(entries_under(&bundle, &[]).expect("the bundle reads"), before, "{way}: nothing was written under the bundle");
        let lost = app.request("exec", json!({"command": "footage.missing"})).expect("the footage is checked");
        assert_eq!(lost["missing"], json!([]), "{way}: every medium is found");
        let media = contents(&the_copy(&scratch).expect("the copy").join("card Media")).expect("the media folder reads");
        assert!(!media.is_empty(), "{way}: the media are beside the copy, inside the container");
    }
}

/// Only the import of a template is handed a copy of it, only of the file its `path` names, and only a
/// template the session may not write beside: a command that only reads one, a template of the bundle
/// named under a parameter the import does not read, and a template in a folder the session may write,
/// are read where they are, and the folder holding it is no more written for it than it is.
#[test]
fn only_the_import_of_a_template_in_the_bundle_is_handed_a_copy() {
    let (scratch, bundle, _) = folders().expect("the folders");
    let in_bundle = template_in(&scratch, &bundle, "tpl").expect("the template");
    let mut app = scratch.app().expect("the app starts");
    let info = ("essential.templateInfo".to_string(), json!({"path": in_bundle.display().to_string()}));
    run(&mut app, &[info]).expect("the template's manifest is read");
    assert_eq!(
        names(&scratch.folders.staged).expect("the staged folder reads"),
        Vec::<String>::new(),
        "a command that only reads a template is handed no copy"
    );
    let inside = scratch.path().join(FILE);
    let (path, other) = (inside.display().to_string(), in_bundle.display().to_string());
    let named_elsewhere = json!({"path": path, "addToComp": false, "template": other, "file": other, "source": other});
    let words = run(&mut app, &[("essential.importTemplate".to_string(), named_elsewhere)]).expect_err("a parameter the import does not read is refused");
    assert!(words.contains("unknown parameter(s) `file`, `source`, `template`"), "the command's own words: {words}");
    assert_eq!(
        names(&scratch.folders.staged).expect("the staged folder reads"),
        Vec::<String>::new(),
        "a template named under a parameter the import does not read is handed no copy"
    );
    run(&mut app, &[import_template(&inside)]).expect("the template inside the container imports");
    assert_eq!(
        names(&scratch.folders.staged).expect("the staged folder reads"),
        Vec::<String>::new(),
        "a template that may be written beside is read where it is"
    );
    assert!(scratch.path().join("card Media").is_dir(), "its media are written beside it");
    let words = run(&mut app, &[import_template(&bundle.join("tpl").join("absent.ectemplate"))]).expect_err("a template that is not there is refused");
    assert!(words.contains("cannot read"), "the command's own words: {words}");
    assert_eq!(names(&scratch.folders.staged).expect("the staged folder reads"), Vec::<String>::new(), "no copy of a file that is not there");
}

/// A template that holds a medium and shows a vector file of the bundle that could not be read when it was
/// exported, which keeps naming the file where it lay: its import before a conversion is refused for that
/// footage, as a project's is, and it holds a medium of its own, which the import's media folders are made
/// for.
fn template_with_media_and_bundled_footage(scratch: &Scratch, bundle: &Path) -> Result<PathBuf, String> {
    let art = put(bundle, "art.pdf", vector(true))?;
    let photo = scratch.path().join("photo.png");
    image::RgbImage::from_pixel(8, 8, image::Rgb([200, 30, 30])).save(&photo).map_err(|e| e.to_string())?;
    let template = scratch.path().join("shows.ectemplate");
    let mut app = scratch.app().map_err(|e| e.to_string())?;
    let shown = run(&mut app, &showing(&art))?;
    let imported = run(&mut app, &[("file.import".to_string(), json!({"paths": [photo.display().to_string()]}))])?;
    let item = imported.pointer("/0/result/items/0").ok_or("the photo's item")?;
    run(&mut app, &[("layer.addItem".to_string(), json!({"comp": 1, "item": item, "time": 0}))])?;
    let layer = shown.pointer("/2/result/layer").ok_or("the layer showing the file")?;
    let control = json!({"comp": 1, "layer": layer, "path": "transform/opacity", "name": "Opacity"});
    run(&mut app, &[("essential.addProperty".to_string(), control)])?;
    let away = bundle.join("away.pdf");
    std::fs::rename(&art, &away).map_err(|e| e.to_string())?;
    let exported = run(&mut app, &[("essential.exportTemplate".to_string(), json!({"comp": 1, "path": template.display().to_string()}))]);
    std::fs::rename(&away, &art).map_err(|e| e.to_string())?;
    exported?;
    put(bundle, "tpl/shows.ectemplate", std::fs::read(&template).map_err(|e| e.to_string())?)
}

/// The scratch session's probe of a template's import, which makes the folders of the template's media on
/// the disk itself, leaves none behind: a call refused for its conversion, though the probe imported a
/// template holding a medium, leaves the copy's folder holding the copy alone, the staged folder holding
/// that folder alone, and the bundle as it was; and one that follows an import of the same template, whose
/// media are there, leaves those as they were.
#[test]
fn a_refused_call_leaves_no_empty_media_folder_behind() {
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let template = template_with_media_and_bundled_footage(&scratch, &bundle).expect("the template");
        let refused_call = [import_template(&template), convert_in(json!("C"))];
        let before = entries_under(&bundle, &[]).expect("the bundle reads");
        let words = batch_refused(&mut scratch.app().expect("the app starts"), &bundle, read_only, &refused_call).unwrap_or_else(|e| panic!("{e}"));
        assert!(words.contains("open or import in one call and convert in the next"), "the call is refused for its conversion: {words}");
        let copy = the_copy(&scratch).expect("the copy was made before the call was refused, and is all the staged folder holds");
        assert!(alone(&copy.join("shows.ectemplate")).expect("the copy's folder reads"), "nothing beside the copy: {:?}", entries_under(&copy, &[]));
        assert_eq!(entries_under(&bundle, &[]).expect("the bundle reads"), before, "nothing was written under the bundle (read-only {read_only})");
        let mut app = scratch.app().expect("the app starts");
        with_bundle(&bundle, read_only, || run(&mut app, &[import_template(&template)])).expect("the bundle is read").expect("the plain import runs");
        let written = contents(&copy).expect("the copy's folder reads");
        assert!(copy.join("shows Media").is_dir(), "the import wrote its media beside the copy: {written:?}");
        let words = batch_refused(&mut app, &bundle, read_only, &refused_call).unwrap_or_else(|e| panic!("{e}"));
        assert!(words.contains("open or import in one call and convert in the next"), "refused again: {words}");
        assert_eq!(contents(&copy).expect("the copy's folder reads"), written, "the media an earlier import wrote stay as they were (read-only {read_only})");
        assert_eq!(the_copy(&scratch).expect("the staged folder holds the copy's folder alone"), copy, "nothing else in the staged folder");
    }
}

/// [`template_with_media_and_bundled_footage`], put in a folder of the container the session may write,
/// with the call that imports it and converts, which is refused for its conversion.
fn in_a_writable_folder(scratch: &Scratch, bundle: &Path) -> Result<(PathBuf, [(String, Value); 2]), String> {
    let made = template_with_media_and_bundled_footage(scratch, bundle)?;
    let template = put(scratch.path(), "docs/shows.ectemplate", std::fs::read(&made).map_err(|e| e.to_string())?)?;
    let call = [import_template(&template), convert_in(json!("C"))];
    Ok((template, call))
}

/// Whether the folder holding `template` holds it alone.
fn alone(template: &Path) -> Result<bool, String> {
    let folder = template.parent().ok_or("a folder")?;
    Ok(entries_under(folder, &[])? == [PathBuf::from(template.file_name().ok_or("a name")?)])
}

/// A template in a folder the session may write is imported where it is, so the call hands no copy of it,
/// and the probe of a call refused for its conversion still writes nothing beside it: the folder holds the
/// template alone afterwards, as before, and the staged folder holds nothing, the probe's own folder gone.
#[test]
fn a_refused_call_leaves_nothing_beside_a_template_in_a_folder_the_session_may_write() {
    for read_only in BUNDLES {
        let (scratch, bundle, _) = folders().expect("the folders");
        let (template, refused_call) = in_a_writable_folder(&scratch, &bundle).expect("the template");
        let words = batch_refused(&mut scratch.app().expect("the app starts"), &bundle, read_only, &refused_call).unwrap_or_else(|e| panic!("{e}"));
        assert!(words.contains("open or import in one call and convert in the next"), "the call is refused for its conversion: {words}");
        assert!(
            alone(&template).expect("the folder reads"),
            "nothing beside the template (read-only {read_only}): {:?}",
            template.parent().map(|folder| entries_under(folder, &[]))
        );
        assert_eq!(entries_under(&scratch.folders.staged, &[]).expect("the staged folder reads"), Vec::<PathBuf>::new(), "nothing in the staged folder");
    }
}

/// A probe the disk will not make its own folder for refuses the call in the disk's words, as a copy that
/// cannot be made does, before anything of it runs, rather than run its template's import where its
/// folders would land beside the template.
#[test]
fn a_probe_the_disk_will_not_make_a_folder_for_refuses_the_call() {
    let (scratch, bundle, _) = folders().expect("the folders");
    let (template, refused_call) = in_a_writable_folder(&scratch, &bundle).expect("the template");
    let mut app = scratch.app().expect("the app starts");
    let call = json!({"command": "engine.batch", "params": {"steps": batch_steps(&refused_call)}});
    let mode = |mode| std::fs::set_permissions(&scratch.folders.staged, std::os::unix::fs::PermissionsExt::from_mode(mode)).expect("the mode is set");
    mode(0o555);
    let answer = app.request("exec", call);
    mode(0o755);
    match answer {
        Err(CraftError::Io { message }) => {
            assert_eq!(message, "cannot make the link a probe of a template's import reads in the container's staged folder: Permission denied (os error 13)")
        }
        other => panic!("the disk's refusal: {other:?}"),
    }
    assert!(alone(&template).expect("the folder reads"), "nothing beside the template");
    assert_eq!(item_names(&mut app).expect("the items"), Vec::<String>::new(), "nothing of the call ran");
}
