//! Face-by-face match of CoreText's faces against the faces EffectCraft registers from CoreText's
//! folders, by the (file, PostScript name) pair: the identity FX-3's `list_fonts` rule needs. One
//! definition, used by the `facematch` example (the detailed capture) and by FontProbe through
//! `ecprobe_face_match` (the summary, in the simulator and on the device).
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::Hash;
use std::path::PathBuf;

use effectcraft_text::fonts::{FaceData, FaceInfo};

/// One CoreText descriptor: PostScript name (None where the attribute is absent), file, family.
pub type CtFace = (Option<String>, PathBuf, String);

/// CoreText's faces, one "PostScript name<TAB>file<TAB>family" line each ("-" where the descriptor
/// has no PostScript name), blank lines skipped.
pub fn parse_ct_faces(text: &str) -> Result<Vec<CtFace>, String> {
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() != 3 {
                return Err(format!("bad CoreText face line (want 3 tab-separated fields): {l}"));
            }
            Ok((if f[0] == "-" { None } else { Some(f[0].to_string()) }, PathBuf::from(f[1]), f[2].to_string()))
        })
        .collect()
}

type Pair = (PathBuf, String);

fn count_by<T, K: Eq + Hash>(items: impl IntoIterator<Item = T>, key: impl Fn(&T) -> K) -> HashMap<K, usize> {
    let mut m: HashMap<K, usize> = HashMap::new();
    for it in items {
        *m.entry(key(&it)).or_default() += 1;
    }
    m
}

fn dup_lines(counts: &HashMap<Pair, usize>) -> Vec<String> {
    let mut out: Vec<String> = counts.iter().filter(|(_, n)| **n > 1).map(|((f, p), n)| format!("{n} x {p} in {}", f.display())).collect();
    out.sort();
    out
}

/// The match and why each face did or did not match. `ec` is what `DirectorySource { dirs }.faces()`
/// gives for CoreText's folders, `ui` is `UIFont.familyNames`. With `detailed` false only the
/// counts and the family comparison are written (what FontProbe prints); with it true also every
/// unmatched face, the instance names, the matched families and where the unmatched faces live.
pub fn report(ec: &[FaceInfo], ct: &[CtFace], ui: &BTreeSet<String>, detailed: bool) -> String {
    let mut out = String::new();
    let mut line = |s: String| {
        out.push_str(&s);
        out.push('\n');
    };

    // CoreText keyed by the (file, PostScript name) pair.
    let ct_pairs: HashMap<Pair, usize> = count_by(ct.iter().filter_map(|(ps, file, _)| ps.as_ref().map(|p| (file.clone(), p.clone()))), |p| p.clone());
    let ct_files: BTreeSet<PathBuf> = ct.iter().map(|(_, f, _)| f.clone()).collect();
    let ct_families: BTreeSet<String> = ct.iter().map(|(_, _, fam)| fam.clone()).collect();
    let ct_no_ps = ct.iter().filter(|(ps, _, _)| ps.is_none()).count();
    let ct_dups = dup_lines(&ct_pairs);

    // EffectCraft faces: files and pair counts, and which pairs matched.
    let mut ec_files: Vec<String> = ec.iter().filter_map(|f| match &f.data { FaceData::File(p) => Some(p.display().to_string()), _ => None }).collect();
    ec_files.sort();
    ec_files.dedup();
    let ec_no_ps = ec.iter().filter(|f| f.postscript.is_none()).count();
    let ec_not_file = ec.iter().filter(|f| !matches!(f.data, FaceData::File(_))).count();
    let ec_pairs: HashMap<Pair, usize> = count_by(
        ec.iter().filter_map(|f| match (&f.data, &f.postscript) {
            (FaceData::File(p), Some(ps)) => Some((p.clone(), ps.clone())),
            _ => None,
        }),
        |p| p.clone(),
    );
    let ec_dups = dup_lines(&ec_pairs);

    let is_ct = |f: &FaceInfo| match (&f.data, &f.postscript) {
        (FaceData::File(p), Some(ps)) => ct_pairs.contains_key(&(p.clone(), ps.clone())),
        _ => false,
    };
    let matched_families: BTreeSet<String> = ec.iter().filter(|f| is_ct(f)).map(|f| f.family.clone()).collect();
    let matched = ec.iter().filter(|f| is_ct(f)).count();
    let unmatched_ec = ec.len() - matched;

    // CoreText pairs no EffectCraft face carries (under any file).
    let mut ec_ps_files: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for f in ec {
        if let (FaceData::File(p), Some(ps)) = (&f.data, &f.postscript) {
            ec_ps_files.entry(ps.clone()).or_default().insert(p.display().to_string());
        }
    }
    // A CoreText face that is not one of EffectCraft's faces may still be a named instance of one:
    // CoreText gives every fvar instance its own descriptor, EffectCraft keeps them in
    // FaceInfo.instances (public, each with its own PostScript name).
    let instance_of = |file: &PathBuf, ps: &str| -> Option<String> {
        ec.iter().find_map(|f| match &f.data {
            FaceData::File(p) if p == file => f
                .instances
                .iter()
                .find(|i| i.postscript == ps)
                .map(|i| format!("{} / {} (index {}, variable: {}): instance style {}", f.family, f.style, f.index, f.variable, i.style)),
            _ => None,
        })
    };
    let mut ct_unmatched: Vec<String> = Vec::new();
    let mut instance_matched = 0usize;
    // Per-file EffectCraft instance names, collected only for files with a face CoreText names that matches nothing.
    let mut file_instances: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (ps, file, family) in ct {
        let pair = ps.as_ref().map(|p| (file.clone(), p.clone()));
        let got = pair.as_ref().and_then(|p| ec_pairs.get(p)).copied().unwrap_or(0);
        if got > 0 {
            continue;
        }
        let need = pair.as_ref().and_then(|p| ct_pairs.get(p)).copied().unwrap_or(0);
        match ps.as_ref().and_then(|p| instance_of(file, p)) {
            Some(face) => {
                instance_matched += 1;
                ct_unmatched.push(format!(
                    "  {ps}\t{file}\t{family}: no face, but a named instance of the face in that file ({face})",
                    ps = ps.as_deref().unwrap_or("-"),
                    file = file.display()
                ));
            }
            None => {
                let elsewhere = ps
                    .as_ref()
                    .and_then(|p| ec_ps_files.get(p))
                    .map(|v| v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
                    .unwrap_or_else(|| "nowhere: no EffectCraft face or instance has this PostScript name".into());
                ct_unmatched.push(format!(
                    "  {}\t{}\t{}: CoreText lists this face {}; EffectCraft has {got} of {need} in that file; the name is in: {elsewhere}",
                    ps.as_deref().unwrap_or("-"),
                    file.display(),
                    family,
                    if need > 1 { format!("{need} times") } else { "once".into() }
                ));
                // What EffectCraft's own named instances of that file's faces are called, so a
                // reader can see how the same instance is named on each side.
                file_instances.entry(file.display().to_string()).or_insert_with(|| {
                    let mut v: Vec<String> = ec
                        .iter()
                        .filter(|f| matches!(&f.data, FaceData::File(p) if p == file))
                        .flat_map(|f| f.instances.iter().map(|i| format!("{}={}", i.style, i.postscript)))
                        .collect();
                    v.sort();
                    v.dedup();
                    v
                });
            }
        }
    }
    ct_unmatched.sort();

    line("## 3. the match: keep each EffectCraft face whose (file, PostScript name) pair is one CoreText lists".into());
    line(format!(
        "##    CoreText: {} descriptors, {} distinct files, {} families (as CoreText names them); faces without a PostScript name: {}; duplicate (file, PostScript name) pairs: {}",
        ct.len(),
        ct_files.len(),
        ct_families.len(),
        ct_no_ps,
        ct_dups.len()
    ));
    for l in &ct_dups {
        line(format!("##      {l}"));
    }
    line(format!(
        "##    EffectCraft, DirectorySource over CoreText's folders: {} faces from {} files; faces without a PostScript name: {}; faces not read from a file: {}; duplicate (file, PostScript name) pairs: {}",
        ec.len(),
        ec_files.len(),
        ec_no_ps,
        ec_not_file,
        ec_dups.len()
    ));
    for l in &ec_dups {
        line(format!("##      {l}"));
    }
    line(format!(
        "##    matched: {} of {} EffectCraft faces; CoreText's {} faces: {} matched an EffectCraft face, {} matched a named instance of one, {} matched neither; {} EffectCraft faces matched nothing",
        matched,
        ec.len(),
        ct.len(),
        ct.len() - ct_unmatched.len(),
        instance_matched,
        ct_unmatched.len() - instance_matched,
        unmatched_ec
    ));
    if detailed {
        if !ct_unmatched.is_empty() {
            line("## CoreText faces no EffectCraft face in that file carries (named instances found are listed and counted as covered):".into());
            for l in &ct_unmatched {
                line(l.clone());
            }
        }
        for (file, insts) in &file_instances {
            line(format!("##    EffectCraft names the instances of {file}: {}", if insts.is_empty() { "none".into() } else { insts.join(", ") }));
        }
    }

    line(format!("## 4. the families a list_fonts filtered this way would show: {}", matched_families.len()));
    if detailed {
        for f in &matched_families {
            line(f.clone());
        }
    }
    let only_fams: Vec<&String> = matched_families.difference(ui).collect();
    let only_ui: Vec<&String> = ui.difference(&matched_families).collect();
    // Where the matched families' names differ from CoreText's own family names for the same face.
    let mut renamed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let ct_family_of: HashMap<Pair, String> = ct.iter().filter_map(|(ps, file, fam)| ps.as_ref().map(|p| ((file.clone(), p.clone()), fam.clone()))).collect();
    for f in ec {
        if let (FaceData::File(p), Some(ps)) = (&f.data, &f.postscript) {
            if let Some(ctf) = ct_family_of.get(&(p.clone(), ps.clone())) {
                if ctf != &f.family {
                    renamed.entry(ctf.clone()).or_default().insert(f.family.clone());
                }
            }
        }
    }
    line(format!("##    against UIFont.familyNames ({}): {} only in the filtered list, {} only in UIFont.familyNames", ui.len(), only_fams.len(), only_ui.len()));
    if !renamed.is_empty() {
        line("##    faces whose EffectCraft family differs from CoreText's family name for the same face:".into());
        for (ct_f, ec_fs) in &renamed {
            line(format!("##      CoreText \"{ct_f}\" is EffectCraft \"{}\"", ec_fs.iter().cloned().collect::<Vec<_>>().join("\" / \"")));
        }
    }
    let unexplained_fams = only_fams.len() + only_ui.len();
    line(format!(
        "##    of that difference, the {} renames above explain {} family names; unexplained: {}",
        renamed.len(),
        renamed.len() * 2,
        unexplained_fams.saturating_sub(renamed.len() * 2)
    ));
    if unexplained_fams > renamed.len() * 2 {
        line("##    families only in the filtered list:".into());
        for f in only_fams {
            line(format!("##      {f}"));
        }
        line("##    families only in UIFont.familyNames:".into());
        for f in only_ui {
            line(format!("##      {f}"));
        }
    }

    // Why each EffectCraft face matched nothing.
    let mut in_listed: Vec<String> = Vec::new();
    let mut in_unlisted: BTreeMap<String, usize> = BTreeMap::new();
    for f in ec.iter().filter(|f| !is_ct(f)) {
        let path = match &f.data { FaceData::File(p) => p.display().to_string(), _ => "<not a file>".to_string() };
        if ct_files.iter().any(|p| p.display().to_string() == path) {
            in_listed.push(format!(
                "  {}\t{}\t{}/{} (index {}): CoreText lists this file but not this PostScript name",
                f.postscript.as_deref().unwrap_or("-"),
                path,
                f.family,
                f.style,
                f.index
            ));
        } else {
            *in_unlisted.entry(path).or_default() += 1;
        }
    }
    line(format!("## 5. why the unmatched EffectCraft faces matched nothing: {unmatched_ec} in all ({ec_no_ps} without a PostScript name, {ec_not_file} not from a file)"));
    line(format!("##    in files CoreText lists but not this face ({}):", in_listed.len()));
    if detailed {
        for l in &in_listed {
            line(l.clone());
        }
    }
    line(format!("##    in files CoreText does not list ({} faces in {} files):", in_unlisted.values().sum::<usize>(), in_unlisted.len()));
    if detailed {
        for (p, n) in &in_unlisted {
            line(format!("##    {n} in {p}"));
        }
    }
    out
}
