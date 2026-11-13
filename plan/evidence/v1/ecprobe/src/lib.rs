//! C entry points FontProbe calls to run EffectCraft's own font registration inside the app.
//! Every function returns a heap string the caller frees with `ecprobe_free`.

use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

use effectcraft_text::fonts::{self, DirectorySource, FontSource};

pub mod facematch;

fn to_c(s: String) -> *mut c_char {
    CString::new(s.replace('\0', " ")).map(CString::into_raw).unwrap_or(std::ptr::null_mut())
}

/// Folders listed one per line, blank lines skipped: what FontProbe passes in, and the format of
/// the `capture/*-dir*.txt` lists the examples read.
pub fn folders_in(text: &str) -> Vec<PathBuf> {
    text.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect()
}

/// Marks a command-line argument that names a folder-list file rather than a folder.
const LIST_PREFIX: char = '@';

/// How an argument is shown in output: a folder as given, a list as `@` and its file name only, so
/// a capture reads the same whichever capture folder the list sits in.
pub fn arg_label(a: &str) -> String {
    match a.strip_prefix(LIST_PREFIX) {
        Some(list) => {
            let name = std::path::Path::new(list).file_name().map_or_else(|| list.into(), |n| n.to_string_lossy().into_owned());
            format!("{LIST_PREFIX}{name}")
        }
        None => a.to_string(),
    }
}

/// The folders named by command-line arguments: a folder, or `@file` for a file listing one
/// folder per line. Exits with status 2 when a list cannot be read or a named folder does not
/// exist, or when nothing is named: `DirectorySource` skips a missing folder without an error,
/// so a wrong path would otherwise look like a folder without fonts. For the examples only.
pub fn folders_from_args_or_exit(args: impl Iterator<Item = String>) -> Vec<PathBuf> {
    let fail = |msg: String| -> ! {
        eprintln!("{msg}");
        std::process::exit(2)
    };
    let mut dirs = Vec::new();
    for a in args {
        match a.strip_prefix(LIST_PREFIX) {
            Some(list) => match std::fs::read_to_string(list) {
                Ok(text) => dirs.extend(folders_in(&text)),
                Err(e) => fail(format!("cannot read folder list {list}: {e} (use an absolute path under simctl spawn)")),
            },
            None => dirs.push(PathBuf::from(a)),
        }
    }
    if dirs.is_empty() {
        fail("no font folder given: pass folders, or @file with one folder per line".into());
    }
    if let Some(d) = dirs.iter().find(|d| !d.is_dir()) {
        fail(format!("not a folder: {}", d.display()));
    }
    dirs
}

/// Newline-separated folder list from the caller.
///
/// # Safety
/// `list` is null or a NUL-terminated string that stays valid for this call.
unsafe fn dirs_from(list: *const c_char) -> Vec<PathBuf> {
    if list.is_null() {
        return Vec::new();
    }
    // SAFETY: guaranteed by the caller (see above).
    folders_in(&unsafe { CStr::from_ptr(list) }.to_string_lossy())
}

/// The folders `DirectorySource::system()` picks for this target, one per line.
#[no_mangle]
pub extern "C" fn ecprobe_system_dirs() -> *mut c_char {
    let dirs = DirectorySource::system().dirs;
    to_c(dirs.iter().map(|d| format!("{} (exists: {})", d.display(), d.is_dir())).collect::<Vec<_>>().join("\n"))
}

/// Faces `DirectorySource { dirs }` lists, without registering them: "<faces> faces, <files> files, <families> families".
///
/// # Safety
/// `list` is null or a NUL-terminated, newline-separated folder list valid for this call.
#[no_mangle]
pub unsafe extern "C" fn ecprobe_count_faces(list: *const c_char) -> *mut c_char {
    // SAFETY: as this function's own contract.
    let faces = DirectorySource { dirs: unsafe { dirs_from(list) } }.faces();
    let mut files: Vec<String> = Vec::new();
    let mut families: Vec<String> = Vec::new();
    for f in &faces {
        if let fonts::FaceData::File(p) = &f.data {
            files.push(p.display().to_string());
        }
        families.push(f.family.clone());
    }
    files.sort();
    files.dedup();
    families.sort();
    families.dedup();
    to_c(format!("{} faces, {} files, {} families", faces.len(), files.len(), families.len()))
}

/// `add_source(&DirectorySource { dirs })`, then `families()` as `list_fonts` reads it: first line
/// "faces before <n>; add_source added <n>; families() now <m>", then "<family> [<styles>]" per family.
///
/// # Safety
/// `list` is null or a NUL-terminated, newline-separated folder list valid for this call.
#[no_mangle]
pub unsafe extern "C" fn ecprobe_register_dirs(list: *const c_char) -> *mut c_char {
    let before = fonts::all_faces().len();
    // SAFETY: as this function's own contract.
    let added = fonts::add_source(&DirectorySource { dirs: unsafe { dirs_from(list) } });
    let fams = fonts::families();
    let mut out = format!("faces before {before}; add_source added {added}; families() now {}", fams.len());
    for (family, styles) in &fams {
        out.push_str(&format!("\n{family} [{}]", styles.len()));
    }
    to_c(out)
}

/// `add_font_data(bytes)`: "<n> faces: family/style, ...".
///
/// # Safety
/// `ptr` is null or points to `len` readable bytes that stay valid for this call.
#[no_mangle]
pub unsafe extern "C" fn ecprobe_add_font_data(ptr: *const u8, len: usize) -> *mut c_char {
    if ptr.is_null() {
        return to_c("0 faces (no data)".into());
    }
    // SAFETY: as this function's own contract.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
    let ids = fonts::add_font_data(bytes);
    let names: Vec<String> = ids.iter().map(|&id| {
        let f = fonts::face(id);
        format!("{}/{}", f.info.family, f.info.style)
    }).collect();
    to_c(format!("{} faces: {}", ids.len(), names.join(", ")))
}

/// The characters whose fallback face is compared: Han read as Simplified Chinese, Traditional
/// Chinese and Japanese, and Hangul. FontProbe asks CoreText about the same characters.
const FALLBACK_SAMPLES: [(char, &str); 4] = [('汉', "zh-Hans"), ('漢', "zh-Hant"), ('漢', "ja"), ('한', "ko")];

/// The face EffectCraft's fallback picks for each sample with the fonts registered so far, starting
/// from the default face (Inter): one "<char>\t<language>\t<family>/<style>" line per sample.
pub fn fallback_faces() -> String {
    let prefer = fonts::resolve(fonts::DEFAULT_FAMILY, "Regular").face;
    FALLBACK_SAMPLES
        .iter()
        .map(|&(c, tag)| {
            let f = fonts::face(fonts::fallback_for_locale(c, prefer, tag));
            format!("{c}\t{tag}\t{}/{}", f.info.family, f.info.style)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// [`fallback_faces`] for FontProbe.
#[no_mangle]
pub extern "C" fn ecprobe_fallback_faces() -> *mut c_char {
    to_c(fallback_faces())
}

/// The face-by-face match of [`facematch::report`], summary only: `dirs` are CoreText's folders
/// (newline-separated), `ct_faces` CoreText's faces ("PostScript name<TAB>file<TAB>family" per line),
/// `ui_families` `UIFont.familyNames` (one per line).
///
/// # Safety
/// Each pointer is null or a NUL-terminated string that stays valid for this call.
#[no_mangle]
pub unsafe extern "C" fn ecprobe_face_match(dirs: *const c_char, ct_faces: *const c_char, ui_families: *const c_char) -> *mut c_char {
    // SAFETY: as this function's own contract.
    let text = |p: *const c_char| if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() };
    let ct = match facematch::parse_ct_faces(&text(ct_faces)) {
        Ok(ct) => ct,
        Err(e) => return to_c(e),
    };
    let ui: std::collections::BTreeSet<String> = text(ui_families).lines().filter(|l| !l.is_empty()).map(str::to_string).collect();
    // SAFETY: as this function's own contract.
    let ec = DirectorySource { dirs: unsafe { dirs_from(dirs) } }.faces();
    to_c(facematch::report(&ec, &ct, &ui, false))
}

/// Frees a string an `ecprobe_` function returned.
///
/// # Safety
/// `p` is null or a pointer an `ecprobe_` function returned, not freed yet.
#[no_mangle]
pub unsafe extern "C" fn ecprobe_free(p: *mut c_char) {
    if !p.is_null() {
        // SAFETY: as this function's own contract: `p` came from `CString::into_raw` in this crate.
        drop(unsafe { CString::from_raw(p) });
    }
}
