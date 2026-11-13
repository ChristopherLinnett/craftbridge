//! Host check: the families `DirectorySource { dirs }` finds, one per line. Arguments: folders, or
//! `@file` for a file listing one folder per line.
use effectcraft_text::fonts::{DirectorySource, FontSource};
fn main() {
    let dirs = ecprobe::folders_from_args_or_exit(std::env::args().skip(1));
    let mut fams: Vec<String> = DirectorySource { dirs }.faces().into_iter().map(|f| f.family).collect();
    fams.sort();
    fams.dedup();
    for f in fams { println!("{f}"); }
}
