//! What registering only the faces CoreText offers (a filtering `FontSource`, RESULT §4) costs
//! EffectCraft's drawing, against registering every face from CoreText's folders.
//!
//! One process registers one set, because EffectCraft's font registry is process state and its
//! fallback result cache is not cleared by `add_source` (fonts.rs:396-413 at the pinned rev only
//! clears the usual-family cache), so the two sets must not share a process.
//! `review-scripts/fallback-cost.sh` runs this twice in the simulator and merges the answers.
//!
//! Sample characters: every family among the faces the rule does NOT register is asked for one
//! character that face draws and no registered face draws. Coverage comes from each face's own
//! `cmap` (skrifa's `Charmap::mappings`, the first condition of EffectCraft's own `covers`,
//! fonts.rs:1017-1024, on the same bytes EffectCraft reads). The answers — which face
//! EffectCraft's fallback picks for a sample, or none — come only from the public
//! `fonts::fallback_for_locale`, with the default Inter Regular as the preferred face, after the
//! faces are registered through the public `fonts::add_source`. A face from the bundled set (the
//! 7 faces registered at start) counts as no covering face, since nothing covers the sample.
//!
//! Arguments: `fallbackcost @folder-list coretext-faces full|filtered` — the folder list and the
//! CoreText face lines are facematch's inputs; `filtered` registers exactly the faces whose
//! (file, PostScript name) pair CoreText lists (the 266), `full` registers them all (the 633).
//! Output: `meta` and `sample` lines, tab separated, families in sorted order.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use effectcraft_text::fonts::{self, DirectorySource, FaceData, FaceInfo, FontSource};
use skrifa::{FontRef, MetadataProvider};

/// A `FontSource` that hands EffectCraft a prepared list: the rule's filtered registration.
struct Prepared {
    faces: Vec<FaceInfo>,
}

impl FontSource for Prepared {
    fn faces(&self) -> Vec<FaceInfo> {
        self.faces.clone()
    }
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(2);
    })
}

/// Characters a sample must not be: control characters and blanks (EffectCraft's own
/// `blank_by_nature`, fonts.rs:1007-1014, plus surrogates and the private-use areas), and the
/// BMP Latin block (every font has it; a script sample comes from above it).
fn no_sample(c: u32) -> bool {
    c < 0xA0
        || (0xD800..=0xDFFF).contains(&c)
        || (0xE000..=0xF8FF).contains(&c)
        || (0xF0000..=0x10FFFD).contains(&c)
        || (0xFE00..=0xFE0F).contains(&c)
        || (0xE0100..=0xE01EF).contains(&c)
        || (0x200B..=0x200F).contains(&c)
        || (0x2060..=0x206F).contains(&c)
}

/// The code points of one face's `cmap` that map to a glyph (skrifa's `mappings` skips NOTDEF).
fn face_chars(path: &std::path::Path, index: u32, files: &mut HashMap<PathBuf, Vec<u8>>) -> BTreeSet<u32> {
    let bytes = files.entry(path.to_path_buf()).or_insert_with(|| std::fs::read(path).unwrap_or_default());
    FontRef::from_index(bytes, index).map(|font| font.charmap().mappings().map(|(c, _)| c).collect()).unwrap_or_default()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 || (args[2] != "full" && args[2] != "filtered") {
        eprintln!("usage: fallbackcost @folder-list coretext-faces full|filtered");
        std::process::exit(2);
    }
    let dirs = ecprobe::folders_from_args_or_exit(args[..1].iter().cloned());
    let ct = ecprobe::facematch::parse_ct_faces(&read(&args[1])).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let mode = args[2].clone();

    // The same 633 faces facematch sees; the 266 are exactly the ones whose pair CoreText lists.
    let ec = DirectorySource { dirs: dirs.clone() }.faces();
    let ct_pairs: HashSet<(PathBuf, String)> = ct.iter().filter_map(|(ps, file, _)| ps.as_ref().map(|p| (file.clone(), p.clone()))).collect();
    let is_matched = |f: &FaceInfo| match (&f.data, &f.postscript) {
        (FaceData::File(p), Some(ps)) => ct_pairs.contains(&(p.clone(), ps.clone())),
        _ => false,
    };
    let matched: Vec<FaceInfo> = ec.iter().filter(|f| is_matched(f)).cloned().collect();

    // The bundled faces (the 7 registered at process start) count as covering, exactly as they
    // do in a phone's registry, but by EffectCraft's own has_char, not by their cmap.
    let bundled: Vec<fonts::FaceId> = fonts::all_faces().iter().map(|f| f.id).collect();
    let bundled_covers = |c: u32| bundled.iter().any(|id| fonts::face(*id).has_char(char::from_u32(c).unwrap_or(' ')));

    // Coverage of every face, by cmap, read from the files once each.
    let mut files: HashMap<PathBuf, Vec<u8>> = HashMap::new();
    let cover: Vec<BTreeSet<u32>> = ec
        .iter()
        .map(|f| match &f.data {
            FaceData::File(p) => face_chars(p, f.index, &mut files),
            _ => BTreeSet::new(),
        })
        .collect();
    let covered_by_registered: HashSet<u32> = ec.iter().zip(&cover).filter(|(f, _)| is_matched(f)).flat_map(|(_, c)| c.iter().copied()).collect();

    // One sample per family of the unregistered faces: a character its faces map that neither a
    // registered face's cmap nor a bundled face's has_char covers.
    let mut lost: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
    for (f, chars) in ec.iter().zip(&cover).filter(|(f, _)| !is_matched(f)) {
        let e = lost.entry(f.family.as_str()).or_default();
        e.extend(chars.iter().copied().filter(|c| !covered_by_registered.contains(c)));
    }
    let mut samples: Vec<(&str, u32)> = Vec::new();
    let mut no_sample_families: Vec<&str> = Vec::new();
    for (family, chars) in &lost {
        match chars.iter().copied().find(|c| !no_sample(*c) && !bundled_covers(*c)) {
            Some(c) => samples.push((family, c)),
            None => no_sample_families.push(family),
        }
    }

    // Register this run's set, then ask the public fallback about every sample.
    let added = match mode.as_str() {
        "full" => fonts::add_source(&DirectorySource { dirs }),
        _ => fonts::add_source(&Prepared { faces: matched.clone() }),
    };
    let total = fonts::all_faces().len();
    println!("meta\t{mode}\tecfaces\t{}\tmatched\t{}\tadded\t{added}\tregistered\t{total}", ec.len(), matched.len());
    let prefer = fonts::resolve(fonts::DEFAULT_FAMILY, "Regular").face;
    for (family, c) in &samples {
        let ch = char::from_u32(*c).unwrap_or('\u{FFFD}');
        let picked = fonts::face(fonts::fallback_for_locale(ch, prefer, "en-US"));
        let answer =
            if picked.info.origin == "bundled" { "none".to_string() } else { format!("{} / {}", picked.info.family, picked.info.style) };
        println!("sample\t{family}\tU+{c:04X}\t{ch}\t{answer}");
    }
    for family in no_sample_families {
        println!("nosample\t{family}");
    }
}
