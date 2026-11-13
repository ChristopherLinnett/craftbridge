//! Host check: the faces EffectCraft's `add_font_data` reads from each font file given.
fn main() {
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else { println!("{path}: unreadable"); continue };
        let ids = effectcraft_text::fonts::add_font_data(bytes);
        let names: Vec<String> = ids.iter().map(|&id| {
            let f = effectcraft_text::fonts::face(id);
            format!("{}/{} (index {}, full \"{}\")", f.info.family, f.info.style, f.info.index, f.info.full_name)
        }).collect();
        println!("{}: {} faces\n  {}", path.rsplit('/').next().unwrap_or(&path), ids.len(), names.join("\n  "));
    }
}
