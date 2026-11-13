//! Face-by-face match of CoreText's faces against the faces EffectCraft registers from CoreText's
//! folders, by the (file, PostScript name) pair, in full detail (`ecprobe::facematch::report`).
//! Arguments: `@folder-list` (CoreText's folders, as `review-coretext-dirs.txt`), a file of
//! CoreText's faces (one "PostScript name\tfile\tfamily" line per descriptor, "-" where no
//! PostScript name), and a file of `UIFont.familyNames`, one per line.
use std::collections::BTreeSet;

use effectcraft_text::fonts::{DirectorySource, FontSource};

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(2);
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        eprintln!("usage: facematch @folder-list coretext-faces ui-family-names");
        std::process::exit(2);
    }
    let dirs = ecprobe::folders_from_args_or_exit(args[..1].iter().cloned());
    let ct = ecprobe::facematch::parse_ct_faces(&read(&args[1])).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let ui: BTreeSet<String> = read(&args[2]).lines().filter(|l| !l.is_empty()).map(str::to_string).collect();
    let ec = DirectorySource { dirs }.faces();
    print!("{}", ecprobe::facematch::report(&ec, &ct, &ui, true));
}
