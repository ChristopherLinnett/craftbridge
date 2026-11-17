//! A Lottie file or a vector file in the app bundle, which the session may read and never write, is
//! imported as the command line imports it from a writable folder, with every image its import writes
//! beside it and every image it names, on every way a call reaches either command, and nothing is
//! written under the bundle, whether the disk would refuse the write (iOS) or take it (Android).

use super::common::Scratch;
use super::export::entries_under;
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

/// The steps of a vector file's conversion: a comp, the file as footage, a layer of it and the
/// conversion of that layer.
fn conversion(file: &Path) -> Vec<(String, Value)> {
    let file = file.display().to_string();
    vec![
        ("comp.new".to_string(), json!({"name": "C", "width": 64, "height": 64, "duration": 1})),
        ("file.import".to_string(), json!({"paths": [file]})),
        ("layer.addItem".to_string(), json!({"comp": 1, "item": 2, "time": 0})),
        ("layer.create".to_string(), json!({"comp": 1, "op": "shapesFromVector", "layers": [1]})),
    ]
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
    let line = app.mcp(&tools_call(name, &arguments).to_string()).ok_or("the line is answered")?;
    let reply: Value = serde_json::from_str(&line).map_err(|e| format!("the answer is JSON: {e}"))?;
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
