//! Run inside the simulator (`simctl spawn`): registers the folders the arguments name (folders, or
//! `@file` for a file listing one folder per line), then prints the face EffectCraft's fallback
//! picks for Han text in each language.
use effectcraft_text::fonts::{self, DirectorySource};
fn main() {
    // A wrong path must fail loudly: `simctl spawn` starts the child with a different working
    // directory, so a relative path would register nothing and every fallback answer would be fake.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dirs = ecprobe::folders_from_args_or_exit(args.iter().cloned());
    let added = fonts::add_source(&DirectorySource { dirs });
    println!("registered {added} faces from {}", args.iter().map(|a| ecprobe::arg_label(a)).collect::<Vec<_>>().join(" "));
    // Why a Chinese face is or is not picked: what each PingFang SC face reports for 汉.
    for f in fonts::all_faces().iter().filter(|f| f.info.family == "PingFang SC") {
        let path = match &f.info.data { fonts::FaceData::File(p) => p.display().to_string(), _ => "-".into() };
        let file = path.rsplit('/').next().unwrap_or("-").to_string();
        println!("  PingFang SC/{} index {} in {file}: parses {}, glyph(汉) {:?}, has_char(汉) {}", f.info.style, f.info.index, f.font().is_some(), f.glyph('汉'), f.has_char('汉'));
    }
    for line in ecprobe::fallback_faces().lines() {
        let [c, tag, face] = line.splitn(3, '\t').collect::<Vec<_>>()[..] else { continue };
        println!("  {c} ({tag}) -> {face}");
    }
}
