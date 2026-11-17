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
//! Code inside EffectCraft's own dispatch (an `engine.batch`'s `$N` references, a replay's `expect`
//! steps) keeps seeing the full registry: an adaptation that trimmed what that code reads would be
//! a different product, so it is an accepted difference, which the integration guides are to state
//! (plan §25).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{PoisonError, RwLock};

use craftbridge_core::folders::Folders;
use effectcraft_text::fonts::{self as text_fonts, Face, FaceData};
use serde_json::Value;

/// The command whose listings this module trims.
pub(crate) const TEXT_FONTS: &str = "text.fonts";
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
/// listing or an `engine.batch` with a `text.fonts` step. It answers in `methods::execute`, the
/// entry `exec` and every `run` step pass; a path that hands the engine's
/// answers to the app past that entry — the `mcp` line, whose tools run their commands on the backend
/// directly — must answer its listings through this function too, or the app sees faces the phone
/// hides. The render runner's calls through `methods::execute` answer no listing. No answer of a
/// `file.runScript`, whose command list the engine would run itself, reaches an app: every session
/// this crate builds is confined, and the confinement refuses every command that runs a script
/// (PD13), so no such answer is read here.
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

/// Trim the `results` of a batch's `{steps, results}` answer, pairing them by position with the
/// steps the request carried, as the engine writes them: it pushes one result per step and answers
/// an error at the first step that fails (`crates/engine/src/cmd/batch.rs`), so row i belongs to
/// step i.
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

/// Is this step of a batch a `text.fonts` call, named as the engine reads a step: by its
/// `command`, or else its `id`?
fn step_is_text_fonts(step: &Value) -> bool {
    step.get("command").or_else(|| step.get("id")).and_then(Value::as_str) == Some(TEXT_FONTS)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;
    use std::sync::Once;

    // The tests hand the filter their own face list through `filtered_by` and never write the
    // process-wide one, which every session build in this binary rewrites, bar the one test that
    // runs alone in a child process of its own.

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

    /// The families a bundled-only face list keeps: EffectCraft's built-in statics and the region
    /// subsets the crate bundles.
    const KEPT_BY_A_BUNDLED_ONLY_LIST: [&str; 5] = ["Inter", "JetBrains Mono", "Noto Serif", "Noto Sans SC", "Noto Sans TC"];

    /// Whether a bundled-only list trimmed this row: only families that list keeps, each reporting
    /// `bundled`, the bundled Simplified face among them, and `count` recounted. An untrimmed row
    /// fails it by a family the list drops or, where a query left only kept families, by the
    /// `system` origin the folder scan gives the bundled files.
    fn trimmed_to_the_bundled(row: &Value) -> bool {
        let each_kept_as_bundled = row.get("families").and_then(Value::as_array).into_iter().flatten().all(|entry| {
            entry.get("family").and_then(Value::as_str).is_some_and(|name| KEPT_BY_A_BUNDLED_ONLY_LIST.contains(&name))
                && entry.get("origin").and_then(Value::as_str) == Some("bundled")
        });
        each_kept_as_bundled && family_of(row, "Noto Sans SC").is_some() && count_agrees(row)
    }

    /// A `{steps, results}` answer whose first row is the registry's listing.
    fn answer_of_one_listing() -> Value {
        json!({ "steps": 1, "results": [raw_listing()] })
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
        let allowed: HashSet<String> = KEPT_BY_A_BUNDLED_ONLY_LIST.into_iter().map(str::to_string).collect();
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
    fn a_step_is_found_as_the_engine_reads_it() {
        assert!(step_is_text_fonts(&json!({ "command": TEXT_FONTS })));
        assert!(step_is_text_fonts(&json!({ "id": TEXT_FONTS })));
        assert!(step_is_text_fonts(&json!({ "command": TEXT_FONTS, "id": "comp.new" })), "a `command` is read first, as the engine reads it");
        assert!(!step_is_text_fonts(&json!({ "command": "comp.new", "id": TEXT_FONTS })), "an `id` beside a `command` is never read");
        assert!(!step_is_text_fonts(&json!({ "command": "comp.new" })));
        // A batch step has no `engine.execute` method form: the engine refuses one before it runs.
        assert!(!step_is_text_fonts(&json!({ "method": "engine.execute", "params": { "id": TEXT_FONTS } })));
    }

    /// Every entry an app's font listing comes back through, driven as an app drives it, with the
    /// bundled-only face list where a phone's engine start writes its own, so what is weighed is each
    /// door's own call of [`filtered`]: the desktop answers no face list, and without one a door that
    /// lost its filter answers exactly as one that kept it. The face list is one per process and
    /// every session build rewrites it, so the child runs alone in a process of its own, where no
    /// other test's build can rewrite it between the child's write and its reads.
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    #[test]
    fn every_app_facing_entry_answers_through_the_face_list() {
        craftbridge_process::rerun(
            "fonts::tests::child_lists_through_every_app_facing_entry",
            &[],
            "an app-facing entry answered a listing the face list does not trim",
        )
        .unwrap();
    }

    /// The listing of `exec` of `text.fonts`, of a `run` step, of an `engine.batch` row and of each
    /// MCP tool that runs a command, through the app's `mcp` line; each step spelling the batch reads
    /// is one path. Every failing path is named at once, so one plant shows which paths a door's
    /// filter covers.
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    #[test]
    #[ignore = "run by every_app_facing_entry_answers_through_the_face_list, alone in a process of its own, since it writes the process-wide face list"]
    fn child_lists_through_every_app_facing_entry() {
        use craftbridge_core::{CraftApp, Platform};
        // The app as an app builds it, confined, delivering the bundled folder as one of its
        // `Folders.fonts`: every path below is one the shipped app answers.
        let mut scratch = crate::session::tests::Scratch::new(false, None);
        scratch.folders.fonts = vec![bundled_fonts_dir()];
        let mut app = scratch.app(Platform::current());
        // The build above wrote the desktop's answer, none; a phone's engine start writes its own.
        *FACE_LIST.write().unwrap_or_else(PoisonError::into_inner) = Some(bundled_only());

        let comp = json!({ "command": "comp.new", "params": { "name": "C", "width": 100, "height": 100, "duration": 1, "frameRate": 30 } });
        let mut untrimmed: Vec<String> = Vec::new();
        let mut weigh = |what: &str, answer: &Value, pointer: &str| {
            let listing = answer.pointer(pointer).unwrap_or(&Value::Null);
            if !trimmed_to_the_bundled(listing) {
                untrimmed.push(format!("{what} lists {:?}", family_names(listing)));
            }
        };
        let exec = app.request("exec", json!({ "command": TEXT_FONTS })).expect("`exec` answers");
        weigh("`exec` of `text.fonts`", &exec, "");
        // A listing that is not the first step, so a row is matched to its step by position.
        let run = app.request("run", json!({ "steps": [comp, { "command": TEXT_FONTS }] })).expect("`run` answers");
        weigh("a `run` step", &run, "/1/result");
        let batch = json!({ "command": "engine.batch", "params": { "steps": [comp, { "command": TEXT_FONTS }, { "id": TEXT_FONTS }] } });
        let batch = app.request("exec", batch).expect("the batch answers");
        weigh("an `engine.batch` row naming the command", &batch, "/results/1");
        weigh("an `engine.batch` row naming it by `id`", &batch, "/results/2");
        for (tool, request, _, pointer) in crate::mcp::tests::paths() {
            let reply = app.mcp(&request.to_string()).expect("the `mcp` line answers");
            let reply: Value = serde_json::from_str(&reply).expect("the reply is JSON");
            weigh(&format!("the `mcp` line's {tool}"), &crate::mcp::tests::answer_of(&reply), pointer);
        }
        assert!(untrimmed.is_empty(), "these entries answered listings the face list does not trim: {untrimmed:#?}");
    }
}
