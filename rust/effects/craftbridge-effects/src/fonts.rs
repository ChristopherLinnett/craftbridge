//! AD2 — the phone's fonts (plan §19): which font folders the engine starts with, and the font
//! listing every app-facing call answers through.
//!
//! Two halves. At engine start the plugin registers, through EffectCraft's one registration entry
//! (`effectcraft_text::fonts::add_source`), the folders the platform's own text system answers
//! for — iOS: the parent folders of the files CoreText reports, never a hard-coded list (§32 V1);
//! Android: `/system/fonts` — plus the app's `Folders.fonts`. Everything registered stays
//! registered and stays drawable; the filter below trims only what an app is *shown*.
//!
//! The second half is that filter. iOS lists families from its own files' name tables, which names
//! fonts Apple hid from apps' listings (and, in the simulator, fonts of the Mac's file system),
//! so an app-facing list must say what the platform's own listing says. The filter reads what is
//! registered from `effectcraft_text::fonts::all_faces()` — no second folder scan — and keeps a
//! face when its file sits in one of the app's `Folders.fonts` (origin `bundled`: the families the
//! app carries, identified by their files because the folder scan tags every face `system`), when
//! its (file, PostScript name) pair is one CoreText reported (origin `system`), or when its bytes
//! are EffectCraft's built-in statics (origin `bundled`, as they report). A kept family lists only
//! the kept faces' styles, and `count` counts what survives. Where the platform answers no face
//! list — the desktop build, Android — every listing passes through untouched.
//!
//! A script, and anything a script drives, keeps seeing the full registry: an adaptation that
//! trimmed what a script reads would be a different product, so scripts seeing the whole registry
//! is an accepted difference, which the integration guides are to state (plan §25).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{PoisonError, RwLock};

use craftbridge_core::folders::Folders;
use effectcraft_text::fonts::{self as text_fonts, Face, FaceData};
use serde_json::Value;

/// The command whose listings this module trims, and the `engine.execute` method step that carries it.
pub(crate) const TEXT_FONTS: &str = "text.fonts";
/// The command `engine.batch` steps and `file.runScript`'s `engine.execute` steps name it through.
const ENGINE_EXECUTE: &str = "engine.execute";
/// The one font folder Android is defined to have; `Folders.fonts` comes after it.
#[cfg(target_os = "android")]
const ANDROID_SYSTEM_FONTS: &str = "/system/fonts";

/// What an app-facing listing keeps: the (file, PostScript name) pairs the platform's text system
/// reports for this application, and the folders the application carries its own fonts in.
#[derive(Clone, Debug)]
struct FaceList {
    /// Each file CoreText offers, with the PostScript names it offers in it.
    coretext: HashMap<PathBuf, HashSet<String>>,
    /// The application's `Folders.fonts`, the home of its bundled fonts.
    fonts_dirs: Vec<PathBuf>,
}

impl FaceList {
    /// Take in what a later engine start answered, keeping everything known before it.
    fn absorb(&mut self, later: FaceList) {
        for (file, names) in later.coretext {
            self.coretext.entry(file).or_default().extend(names);
        }
        for dir in later.fonts_dirs {
            push_once(&mut self.fonts_dirs, dir);
        }
    }
}

/// The face list the listing filter answers through, written at engine start. `None` means the
/// platform answers no face list and listings pass through untouched. The engine's font registry
/// is one per process, shared by every engine the app starts, so one filter for the process follows
/// the registry's own life.
static FACE_LIST: RwLock<Option<FaceList>> = const { RwLock::new(None) };

/// Register the phone's font folders with EffectCraft and set the listing filter. One engine
/// start calls this once, before its first command; registering is how the phone's fonts arrive
/// (`add_source` with a `DirectorySource` is EffectCraft's only entry that takes folders).
pub(crate) fn register(folders: &Folders) {
    let (dirs, list) = platform_fonts(folders);
    if !dirs.is_empty() {
        text_fonts::add_source(&text_fonts::DirectorySource { dirs });
    }
    let mut current = FACE_LIST.write().unwrap_or_else(PoisonError::into_inner);
    let known = current.take();
    *current = grown(known, list);
}

/// The face list after one more engine start. The registry it filters only ever grows — two
/// engines of one app register into the same registry and nothing is unregistered — so the list
/// grows with it: an engine started with other font folders must not hide the faces an earlier
/// engine bundled, which stay registered and drawable for both.
fn grown(known: Option<FaceList>, later: Option<FaceList>) -> Option<FaceList> {
    match (known, later) {
        (Some(mut known), Some(later)) => {
            known.absorb(later);
            Some(known)
        }
        (_, later) => later,
    }
}

/// Trim one successful command's answer to what an app may see, when that command is a font
/// listing, a `text.fonts` step of an `engine.batch`, or one of a `file.runScript`. It answers in
/// `methods::run_command`, the entry `exec` and every `run` step pass; a path that hands the engine's
/// answers to the app past that entry — the `mcp` line, whose tools run their commands on the backend
/// directly — must answer its listings through this function too, or the app sees faces the phone
/// hides. The render runner's calls through `methods::execute` answer no listing.
pub(crate) fn filtered(command: &str, params: &Value, value: Value) -> Value {
    // Read once and held for the call: an engine start writes the list only before its own first
    // command, so the read never waits on one for long.
    let list = FACE_LIST.read().unwrap_or_else(PoisonError::into_inner);
    filtered_by(list.as_ref(), command, params, value)
}

/// [`filtered`] with the face list given, the one place a listing is told from any other answer.
fn filtered_by(list: Option<&FaceList>, command: &str, params: &Value, value: Value) -> Value {
    let Some(list) = list else { return value };
    match command {
        TEXT_FONTS => trimmed(list, value),
        "engine.batch" => filter_step_results(list, steps_of(params), value),
        "file.runScript" => filter_run_script(list, params, value),
        _ => value,
    }
}

/// The folders to register, and the face list to filter with: iOS answers both from CoreText,
/// Android names one folder and no list (its listing is its families, unfiltered), every other
/// platform carries only `Folders.fonts` and filters nothing.
fn platform_fonts(folders: &Folders) -> (Vec<PathBuf>, Option<FaceList>) {
    #[cfg(target_os = "ios")]
    {
        ios_fonts(folders)
    }
    #[cfg(target_os = "android")]
    {
        (android_fonts(folders), None)
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        (app_font_dirs(folders), None)
    }
}

/// iOS: the folders are the parents of CoreText's own face files, never a hard-coded list — in
/// the simulator a hard-coded `/System/Library/Fonts` lands on the Mac's fonts (V1), and on a
/// phone a hard-coded list misses fonts Apple keeps inside private frameworks.
#[cfg(target_os = "ios")]
fn ios_fonts(folders: &Folders) -> (Vec<PathBuf>, Option<FaceList>) {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut coretext: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    for face in craftbridge_core::platform::coretext_faces() {
        if let Some(dir) = face.file.parent() {
            push_once(&mut dirs, dir.to_path_buf());
        }
        coretext.entry(face.file).or_default().insert(face.postscript);
    }
    for dir in &folders.fonts {
        push_once(&mut dirs, dir.clone());
    }
    (dirs, Some(FaceList { coretext, fonts_dirs: folders.fonts.clone() }))
}

/// Android: `/system/fonts`, then the app's own folders. The emulator's listing is its families
/// as registered, so no face list is set and the listing passes through.
#[cfg(target_os = "android")]
fn android_fonts(folders: &Folders) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    push_once(&mut dirs, PathBuf::from(ANDROID_SYSTEM_FONTS));
    for dir in app_font_dirs(folders) {
        push_once(&mut dirs, dir);
    }
    dirs
}

/// The application's own font folders, each once.
#[cfg(not(target_os = "ios"))]
fn app_font_dirs(folders: &Folders) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in &folders.fonts {
        push_once(&mut dirs, dir.clone());
    }
    dirs
}

fn push_once(dirs: &mut Vec<PathBuf>, dir: PathBuf) {
    if !dirs.contains(&dir) {
        dirs.push(dir);
    }
}

/// Trim a `{"count", "families": [...], "added"}` listing: each kept family with its kept styles
/// and the origin the kept faces carry, and `count` recounting what survives. A value that is not
/// a listing (a step that answered something else) is returned as it came.
fn trimmed(list: &FaceList, value: Value) -> Value {
    match kept_listing(list, &value) {
        Some(listing) => listing,
        None => value,
    }
}

fn kept_listing(list: &FaceList, value: &Value) -> Option<Value> {
    let families = value.get("families").and_then(Value::as_array)?;
    let kept = kept_faces(list);
    let mut out: Vec<Value> = Vec::new();
    for entry in families {
        if let Some(entry) = keep_family(&kept, entry) {
            out.push(entry);
        }
    }
    let mut doc = value.clone();
    let root = doc.as_object_mut()?;
    let count = out.len();
    root.insert("families".to_string(), Value::Array(out));
    root.insert("count".to_string(), Value::from(count));
    Some(doc)
}

/// One family of a listing: kept, with only its kept faces' styles and origin, if any face of it
/// was kept. Its order and its other fields (the native name, the variable flag) are the listing's.
fn keep_family(kept: &HashMap<String, KeptFamily>, entry: &Value) -> Option<Value> {
    let family = kept.get(entry.get("family").and_then(Value::as_str)?)?;
    let styles: Vec<Value> = entry
        .get("styles")
        .and_then(Value::as_array)?
        .iter()
        .filter(|style| style.as_str().is_some_and(|name| family.styles.contains(&name.to_lowercase())))
        .cloned()
        .collect();
    if styles.is_empty() {
        return None;
    }
    let mut entry = entry.clone();
    let root = entry.as_object_mut()?;
    root.insert("styles".to_string(), Value::Array(styles));
    root.insert("origin".to_string(), Value::String(family.origin.to_string()));
    Some(entry)
}

/// One family's kept faces, taken together: the styles they list (a variable face lists its named
/// instances as styles, as `families()` does) and the origin the first kept face carries.
struct KeptFamily {
    styles: HashSet<String>,
    origin: &'static str,
}

/// Walk the whole registry once — `all_faces()`, no second scan — and say what survives the list:
/// a face filed under one of the app's own font folders keeps as `bundled`, which is what the app
/// lists and ships next to its LICENSE; a face's (file, PostScript name) pair CoreText reported
/// keeps as `system`; EffectCraft's built-in statics keep as the `bundled` origin they report.
fn kept_faces(list: &FaceList) -> HashMap<String, KeptFamily> {
    let mut kept: HashMap<String, KeptFamily> = HashMap::new();
    for face in text_fonts::all_faces() {
        let Some(origin) = kept_origin(list, &face) else { continue };
        let entry = kept.entry(face.info.family.clone()).or_insert(KeptFamily { styles: HashSet::new(), origin });
        entry.styles.insert(face.info.style.to_lowercase());
        for instance in &face.info.instances {
            entry.styles.insert(instance.style.to_lowercase());
        }
    }
    kept
}

/// The origin a kept face reports, or `None` to drop it from the listing. The app's own folders
/// are asked first: an app that also hands its fonts to the system (so its own views draw them)
/// makes CoreText list them too, and they are still the app's bundled fonts.
fn kept_origin(list: &FaceList, face: &Face) -> Option<&'static str> {
    match &face.info.data {
        FaceData::File(path) => {
            if list.fonts_dirs.iter().any(|dir| path.starts_with(dir)) {
                return Some("bundled");
            }
            if face.info.postscript.as_ref().is_some_and(|name| list.coretext.get(path).is_some_and(|names| names.contains(name))) {
                return Some("system");
            }
            None
        }
        FaceData::Static(_) => Some("bundled"),
        // A face added as bytes (a picker's choice this session): its own origin stands.
        FaceData::Shared(_) => Some(face.info.origin),
    }
}

/// Trim the `results` of a `{steps, results}` answer, pairing them by position as the engine
/// writes them: it pushes one result per step and answers an error at the first step that fails
/// (`crates/engine/src/cmd/batch.rs`, `file_more.rs`), so row i belongs to step i. Steps come from
/// the request when it carried them (a batch, inline script steps), or from the command-list file
/// this module read.
fn filter_step_results(list: &FaceList, steps: Option<&Vec<Value>>, value: Value) -> Value {
    let Some(steps) = steps else { return value };
    let Some(results) = value.get("results").and_then(Value::as_array).cloned() else { return value };
    let out: Vec<Value> = results.into_iter().zip(steps).map(|(result, step)| if step_is_text_fonts(step) { trimmed(list, result) } else { result }).collect();
    let mut doc = value;
    if let Some(root) = doc.as_object_mut() {
        root.insert("results".to_string(), Value::Array(out));
    }
    doc
}

/// The steps array a request carried, if it carried one.
fn steps_of(params: &Value) -> Option<&Vec<Value>> {
    params.get("steps").and_then(Value::as_array)
}

/// Is this step of a batch or script command list a `text.fonts` call, however it is written: by
/// its `command`/`id`, or as an `engine.execute` method step (`file.runScript`'s second form)?
fn step_is_text_fonts(step: &Value) -> bool {
    if step.get("command").or_else(|| step.get("id")).and_then(Value::as_str) == Some(TEXT_FONTS) {
        return true;
    }
    step.get("method").and_then(Value::as_str) == Some(ENGINE_EXECUTE)
        && step.get("params").and_then(|params| params.get("id")).and_then(Value::as_str) == Some(TEXT_FONTS)
}

/// Trim a `file.runScript` answer, reading its request in the order the engine does
/// (`crates/engine/src/cmd/file_more.rs`, `run_script`): `recent` first; then a named script, when
/// neither a `path` nor `steps` comes with it; then a JavaScript file, a `path` with no `steps`;
/// then the command list — the inline `steps` array, or else the file at `path`, read here the way
/// the engine reads it, because the engine runs those steps itself and never brings their ids back
/// through this door. Script code answers from the full registry (the accepted difference), so its
/// answer passes through. So does a `recent` re-run: the engine resolves it from the session's own
/// list of recent scripts, which this door does not read; no session this crate builds runs one,
/// since each is confined and the confinement refuses every script runner.
fn filter_run_script(list: &FaceList, params: &Value, value: Value) -> Value {
    if params.get("recent").is_some() {
        return value;
    }
    let path = params.get("path").and_then(Value::as_str);
    let steps = params.get("steps");
    if params.get("name").and_then(Value::as_str).is_some() && params.get("path").is_none() && steps.is_none() {
        return value;
    }
    if steps.is_none() && path.is_some_and(is_js_script) {
        return value;
    }
    match (steps, path) {
        (Some(Value::Array(steps)), _) => filter_step_results(list, Some(steps), value),
        (_, Some(path)) => match read_command_list(path) {
            Some(steps) => filter_step_results(list, Some(&steps), value),
            None => value,
        },
        _ => value,
    }
}

/// JavaScript script files, as `file.runScript` tells them from command lists
/// (`crates/engine/src/cmd/file_more.rs`: `.js`/`.jsx`, and `.jsxbin` is a script file too — its
/// refusal comes from the engine).
fn is_js_script(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".js") || lower.ends_with(".jsx") || lower.ends_with(".jsxbin")
}

/// Read a command-list file the way `file.runScript` does: its bytes as text, invalid UTF-8
/// replaced; one JSON array, or else one JSON object per line with `//` and `#` comments skipped.
/// Unreadable, empty, or a line that is no JSON is answered `None` — the engine failed on that
/// file, or it is something else; nothing is trimmed.
fn read_command_list(path: &str) -> Option<Vec<Value>> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    if let Ok(Value::Array(steps)) = serde_json::from_str::<Value>(&text) {
        return Some(steps);
    }
    let mut steps: Vec<Value> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        steps.push(serde_json::from_str(line).ok()?);
    }
    (!steps.is_empty()).then_some(steps)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;
    use std::sync::Once;

    // The tests hand the filter their own face list through `filtered_by` and never write the
    // process-wide one, which every session build in this binary rewrites.

    /// The crate's own bundled fonts, added to the registry once for the tests that filter against them.
    static ADD_BUNDLED: Once = Once::new();
    #[cfg(target_os = "macos")]
    static ADD_SYSTEM: Once = Once::new();

    fn bundled_fonts_dir() -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts"));
        ADD_BUNDLED.call_once(|| {
            text_fonts::add_source(&text_fonts::DirectorySource { dirs: vec![dir.clone()] });
        });
        dir
    }

    /// The bundled OTFs as CoreText would report them: each file with its face's PostScript name.
    fn bundled_pairs() -> HashMap<PathBuf, HashSet<String>> {
        let dir = bundled_fonts_dir();
        let mut pairs: HashMap<PathBuf, HashSet<String>> = HashMap::new();
        for face in text_fonts::all_faces() {
            if let FaceData::File(path) = &face.info.data
                && path.starts_with(&dir)
                && let Some(postscript) = &face.info.postscript
            {
                pairs.entry(path.clone()).or_default().insert(postscript.clone());
            }
        }
        pairs
    }

    /// The face list of an app that carries the crate's bundled folder, with no CoreText pairs.
    fn bundled_only() -> FaceList {
        FaceList { coretext: HashMap::new(), fonts_dirs: vec![bundled_fonts_dir()] }
    }

    /// The listing filter of an app that carries the crate's bundled folder and has a face list with
    /// no CoreText pairs: what `filtered` does where a platform supplies one, for the tests of the
    /// paths that call `filtered` to hand their commands' answers through.
    pub(crate) fn filtered_by_the_bundled_list(command: &str, params: &Value, value: Value) -> Value {
        filtered_by(Some(&bundled_only()), command, params, value)
    }

    /// A listing as `text.fonts` writes one, straight off EffectCraft's registry, with the crate's
    /// bundled fonts registered in it.
    pub(crate) fn raw_listing() -> Value {
        bundled_fonts_dir();
        let families: Vec<Value> = text_fonts::families()
            .into_iter()
            .map(|(family, styles)| json!({ "family": family, "styles": styles, "origin": "system", "variable": false }))
            .collect();
        json!({ "count": families.len(), "families": families, "added": 0 })
    }

    fn family_of(listing: &Value, name: &str) -> Option<Value> {
        listing.get("families").and_then(Value::as_array).into_iter().flatten().find(|entry| entry.get("family").and_then(Value::as_str) == Some(name)).cloned()
    }

    pub(crate) fn family_names(listing: &Value) -> Vec<String> {
        let Some(families) = listing.get("families").and_then(Value::as_array) else { return Vec::new() };
        let mut names: Vec<String> = Vec::new();
        for entry in families {
            if let Some(name) = entry.get("family").and_then(Value::as_str) {
                names.push(name.to_string());
            }
        }
        names
    }

    pub(crate) fn count_agrees(listing: &Value) -> bool {
        listing.get("count").and_then(Value::as_u64) == listing.get("families").and_then(Value::as_array).map(Vec::len).map(u64::try_from).and_then(Result::ok)
    }

    /// Whether a bundled-only list trimmed this row: fewer families than the registry lists, the
    /// bundled Simplified face among them, and `count` recounted. An untrimmed row fails it.
    fn trimmed_to_the_bundled(row: &Value) -> bool {
        family_names(row).len() < family_names(&raw_listing()).len() && family_of(row, "Noto Sans SC").is_some() && count_agrees(row)
    }

    /// A `{steps, results}` answer whose first row is the registry's listing.
    fn answer_of_one_listing() -> Value {
        json!({ "steps": 1, "results": [raw_listing()] })
    }

    fn first_row(answer: &Value) -> Value {
        answer.get("results").and_then(Value::as_array).and_then(|rows| rows.first()).cloned().unwrap_or(Value::Null)
    }

    #[test]
    fn no_face_list_passes_the_listing_through() {
        bundled_fonts_dir();
        let raw = raw_listing();
        assert_eq!(filtered_by(None, TEXT_FONTS, &json!({}), raw.clone()), raw, "without a platform face list the listing is the registry's own");
        let batch = json!({ "steps": [{ "command": TEXT_FONTS }] });
        assert_eq!(filtered_by(None, "engine.batch", &batch, answer_of_one_listing()), answer_of_one_listing(), "and so is a batch's row");
    }

    #[test]
    fn the_face_list_grows_with_every_engine_start() {
        let first = FaceList {
            coretext: [(PathBuf::from("/fonts/a.ttc"), ["A-Regular".to_string()].into_iter().collect())].into_iter().collect(),
            fonts_dirs: vec![PathBuf::from("/one/fonts")],
        };
        let second = FaceList {
            coretext: [
                (PathBuf::from("/fonts/a.ttc"), ["A-Bold".to_string()].into_iter().collect()),
                (PathBuf::from("/fonts/b.ttf"), ["B-Regular".to_string()].into_iter().collect()),
            ]
            .into_iter()
            .collect(),
            fonts_dirs: vec![PathBuf::from("/two/fonts"), PathBuf::from("/one/fonts")],
        };
        let both = grown(Some(first), Some(second)).expect("two lists make one");
        assert_eq!(both.fonts_dirs, vec![PathBuf::from("/one/fonts"), PathBuf::from("/two/fonts")], "every engine's font folders, each once");
        let a: HashSet<String> = ["A-Regular", "A-Bold"].into_iter().map(str::to_string).collect();
        assert_eq!(both.coretext.get(Path::new("/fonts/a.ttc")), Some(&a), "a file keeps every name either start reported");
        assert!(both.coretext.contains_key(Path::new("/fonts/b.ttf")));
        assert!(grown(None, None).is_none(), "a platform that answers no face list never gains one");
    }

    #[test]
    fn a_later_engine_without_font_folders_keeps_the_earlier_ones_bundled_faces() {
        // The first engine carried the bundled folder; the second carries none. Both draw the
        // bundled faces, which stay registered, so both list them.
        let empty = FaceList { coretext: HashMap::new(), fonts_dirs: Vec::new() };
        let list = grown(Some(bundled_only()), Some(empty)).expect("two lists make one");
        let trimmed = filtered_by(Some(&list), TEXT_FONTS, &json!({}), raw_listing());
        assert!(trimmed_to_the_bundled(&trimmed), "the second start must not hide the first's bundled faces: {}", trimmed);
    }

    #[test]
    fn bundled_folders_report_bundled_origin() {
        let trimmed = filtered_by(Some(&bundled_only()), TEXT_FONTS, &json!({}), raw_listing());
        assert!(count_agrees(&trimmed));
        let sc = family_of(&trimmed, "Noto Sans SC").expect("the bundled Simplified face stays listed");
        assert_eq!(sc.get("origin").and_then(Value::as_str), Some("bundled"), "a face from Folders.fonts reports bundled, not system");
        assert!(sc.get("styles").and_then(Value::as_array).is_some_and(|styles| styles.iter().any(|s| s.as_str() == Some("Regular"))));
        let tc = family_of(&trimmed, "Noto Sans TC").expect("the bundled Traditional face stays listed");
        assert_eq!(tc.get("origin").and_then(Value::as_str), Some("bundled"));
        for static_family in ["Inter", "JetBrains Mono", "Noto Serif"] {
            let entry = family_of(&trimmed, static_family).expect("EffectCraft's built-in faces stay listed");
            assert_eq!(entry.get("origin").and_then(Value::as_str), Some("bundled"));
        }
        let names: HashSet<String> = family_names(&trimmed).into_iter().collect();
        let allowed: HashSet<String> = ["Inter", "JetBrains Mono", "Noto Serif", "Noto Sans SC", "Noto Sans TC"].into_iter().map(str::to_string).collect();
        assert!(names.is_subset(&allowed), "only statics and bundled faces survive an empty face list: {names:?}");
        assert!(names.len() < family_names(&raw_listing()).len(), "unlisted families must actually be dropped");
    }

    #[test]
    fn coretext_pairs_keep_their_faces_as_system() {
        let list = FaceList { coretext: bundled_pairs(), fonts_dirs: Vec::new() };
        let trimmed = filtered_by(Some(&list), TEXT_FONTS, &json!({}), raw_listing());
        assert!(count_agrees(&trimmed));
        let sc = family_of(&trimmed, "Noto Sans SC").expect("a face CoreText reports stays listed");
        assert_eq!(sc.get("origin").and_then(Value::as_str), Some("system"), "a CoreText file keeps its system origin");
    }

    /// An app that also hands its bundled fonts to the system makes CoreText list them: they are
    /// still the app's own, and report `bundled`.
    #[test]
    fn a_bundled_file_coretext_also_lists_stays_bundled() {
        let list = FaceList { coretext: bundled_pairs(), fonts_dirs: vec![bundled_fonts_dir()] };
        let trimmed = filtered_by(Some(&list), TEXT_FONTS, &json!({}), raw_listing());
        let sc = family_of(&trimmed, "Noto Sans SC").expect("the bundled Simplified face stays listed");
        assert_eq!(sc.get("origin").and_then(Value::as_str), Some("bundled"), "a file in Folders.fonts is bundled whoever else lists it");
    }

    /// A family whose faces are half listed: only the listed face's style may show. Menlo ships on
    /// every macOS the gate runs on, in one `.ttc`, with more than one face in it.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_family_shows_only_its_listed_faces_styles() {
        let system_dir = PathBuf::from("/System/Library/Fonts");
        ADD_SYSTEM.call_once(|| {
            text_fonts::add_source(&text_fonts::DirectorySource { dirs: vec![system_dir.clone()] });
        });
        let listed =
            text_fonts::all_faces().into_iter().find(|face| face.info.family == "Menlo" && face.info.style == "Regular").expect("macOS ships Menlo Regular");
        let (FaceData::File(path), Some(postscript)) = (&listed.info.data, &listed.info.postscript) else {
            panic!("Menlo Regular is a file face with a PostScript name");
        };
        let mut coretext: HashMap<PathBuf, HashSet<String>> = HashMap::new();
        coretext.insert(path.clone(), [postscript.clone()].into_iter().collect());
        let trimmed = filtered_by(Some(&FaceList { coretext, fonts_dirs: Vec::new() }), TEXT_FONTS, &json!({}), raw_listing());
        let menlo = family_of(&trimmed, "Menlo").expect("Menlo stays listed, one of its faces is");
        let styles: Vec<String> = menlo.get("styles").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        assert_eq!(styles, vec!["Regular".to_string()], "only the listed face's style shows");
    }

    #[test]
    fn a_batch_trims_only_its_text_fonts_rows() {
        let raw = raw_listing();
        let answer = json!({ "steps": 3, "results": [raw.clone(), { "family": "Inter", "style": "Regular" }, raw.clone()] });
        let request = json!({ "steps": [{ "command": TEXT_FONTS }, { "command": "layer.newText" }, { "id": TEXT_FONTS }] });
        let trimmed = filtered_by(Some(&bundled_only()), "engine.batch", &request, answer.clone());
        let results = trimmed.get("results").and_then(Value::as_array).expect("a batch answers results");
        assert_eq!(results.len(), 3);
        assert!(trimmed_to_the_bundled(&results[0]), "the `command` step's row is trimmed: {}", results[0]);
        assert_eq!(results[0].get("families"), results[2].get("families"), "`command` and `id` steps are both found");
        assert_eq!(results[1], json!({ "family": "Inter", "style": "Regular" }), "a step that is no listing is left alone");
        assert_eq!(trimmed.get("steps"), answer.get("steps"), "the batch's own count of steps stands");
    }

    #[test]
    fn a_script_run_sees_the_full_registry() {
        let list = bundled_only();
        let answer = answer_of_one_listing();
        // A JavaScript file's answer is the script's own, and named/recent scripts run script code.
        for params in [
            json!({ "path": "/app/Scripts/scan.js" }),
            json!({ "name": "Scan" }),
            json!({ "recent": 1 }),
            json!({ "recent": 0, "steps": [{ "command": TEXT_FONTS }] }),
        ] {
            assert_eq!(filtered_by(Some(&list), "file.runScript", &params, answer.clone()), answer, "{params} runs script code");
        }
    }

    /// The engine reads a `file.runScript` request in one order: a name runs only when neither a
    /// path nor steps come with it, a `.js` path only when no steps do, and inline steps win.
    #[test]
    fn a_script_request_is_read_in_the_engines_order() {
        let list = bundled_only();
        let dir = tempfile::tempdir().expect("a scratch folder");
        let list_file = dir.path().join("listing.json");
        std::fs::write(&list_file, "[{\"command\": \"text.fonts\"}]").expect("write the list");
        let path = list_file.to_string_lossy().to_string();
        for params in [
            json!({ "name": "Scan", "steps": [{ "command": TEXT_FONTS }] }),
            json!({ "name": "Scan", "path": path }),
            json!({ "path": "/app/Scripts/scan.js", "steps": [{ "command": TEXT_FONTS }] }),
            json!({ "steps": "not an array", "path": path }),
        ] {
            let trimmed = filtered_by(Some(&list), "file.runScript", &params, answer_of_one_listing());
            assert!(trimmed_to_the_bundled(&first_row(&trimmed)), "{params} runs a command list, whose listing row is trimmed");
        }
    }

    #[test]
    fn a_script_commands_list_is_read_as_effectcraft_reads_it() {
        let list = bundled_only();
        let dir = tempfile::tempdir().expect("a scratch folder");
        let answer = json!({ "steps": 2, "results": [raw_listing(), { "ok": true }] });
        let files: [(&str, &[u8]); 4] = [
            ("listing.json", b"[{\"command\": \"text.fonts\"}, {\"command\": \"comp.new\"}]"),
            ("listing.jsonl", b"// step one\n\n{\"command\": \"text.fonts\"}\n# not a step\n{\"id\": \"project.save\"}\n"),
            // An `engine.execute` method step is the same call wearing another coat.
            ("method.jsonl", b"{\"method\": \"engine.execute\", \"params\": {\"id\": \"text.fonts\"}}\n{\"command\": \"comp.new\"}\n"),
            // The engine reads the file's bytes with invalid UTF-8 replaced, so a stray byte in a comment runs.
            ("latin1.jsonl", b"// caf\xe9\n{\"command\": \"text.fonts\"}\n{\"command\": \"comp.new\"}\n"),
        ];
        for (name, bytes) in files {
            let file = dir.path().join(name);
            std::fs::write(&file, bytes).expect("write the list");
            let trimmed = filtered_by(Some(&list), "file.runScript", &json!({ "path": file }), answer.clone());
            assert!(trimmed_to_the_bundled(&first_row(&trimmed)), "{name}: the command list's text.fonts row is trimmed");
            assert_eq!(trimmed.get("results").and_then(Value::as_array).map(Vec::len), Some(2), "{name}: every row stays");
        }
    }

    #[test]
    fn a_step_is_found_however_the_list_writes_it() {
        assert!(step_is_text_fonts(&json!({ "command": TEXT_FONTS })));
        assert!(step_is_text_fonts(&json!({ "id": TEXT_FONTS })));
        assert!(step_is_text_fonts(&json!({ "method": "engine.execute", "params": { "id": TEXT_FONTS } })));
        assert!(
            step_is_text_fonts(&json!({ "command": TEXT_FONTS, "method": "engine.execute", "params": { "id": "comp.new" } })),
            "a `command` is read first, as the engine reads it"
        );
        assert!(!step_is_text_fonts(&json!({ "command": "comp.new" })));
        assert!(!step_is_text_fonts(&json!({ "method": "engine.execute", "params": { "id": "comp.new" } })));
        assert!(!step_is_text_fonts(&json!({ "method": "layer.setText" })));
    }

    #[test]
    fn a_js_script_is_not_a_command_list() {
        assert!(is_js_script("/app/Scripts/run.JS"));
        assert!(is_js_script("scan.jsx"));
        assert!(is_js_script("packed.jsxbin"));
        assert!(!is_js_script("commands.json"));
        assert!(!is_js_script("commands.jsonl"));
    }
}
