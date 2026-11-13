//! Rule 6 — `cargo xtask generate`: the only thing allowed to write a generated file.
//!
//! A generated file is committed and never edited by hand, so the generator that writes it has to be the
//! only writer, and the gate has to prove nobody hand-edited one. Both come from one registry: each
//! generator names the output directory it owns and writes its files there. `generate` runs them into
//! the real directories; `generate --check` runs them into a scratch copy and compares byte for byte,
//! so a hand edit or a generator that depends on the date fails the gate with the file named.
//!
//! The registry is empty today; the first generator (the codegen's API surface and its Swift and Kotlin
//! facades) registers itself here in the change that adds it.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

/// A generator's work: write its files into `out_dir`, which is either the real output directory or a
/// scratch copy of it.
pub(crate) type GenerateFn = fn(&Path) -> Result<(), String>;

/// One entry of the registry.
pub(crate) struct Generator {
    /// Named how a message would say it, e.g. `effects-api`.
    pub(crate) name: &'static str,
    /// The directory this generator owns, relative to the repository root.
    pub(crate) output_dir: &'static str,
    pub(crate) generate: GenerateFn,
}

/// The registry. Add a generator here when its files start being generated.
pub(crate) fn generators() -> Vec<Generator> {
    Vec::new()
}

/// Run `cargo xtask generate`, with or without `--check`.
pub(crate) fn run(root: &Path, check: bool) -> ExitCode {
    let registry = generators();
    if registry.is_empty() {
        // Nothing is generated yet, so there is nothing to write and nothing to compare: that is not a
        // failure, it is the state of the repository before the first generator lands.
        println!("generate: no generators registered yet");
        return ExitCode::SUCCESS;
    }
    let problems = registry_problems(&registry);
    if !problems.is_empty() {
        for problem in problems {
            println!("generate: {problem}");
        }
        return ExitCode::from(1);
    }
    if check { check_against_registry(root, &registry) } else { write_registry(root, &registry) }
}

/// What makes a registry unsafe to run. A generator's output directory is emptied before every run, so
/// it must be a `generated`/`Generated` directory inside the repository — never the root, a source folder
/// or a path that climbs out — and two generators may not share or nest one, or each run would delete
/// the other's files.
fn registry_problems(registry: &[Generator]) -> Vec<String> {
    let mut problems = Vec::new();
    for generator in registry {
        let output = Path::new(generator.output_dir);
        let stays_inside = output.components().all(|part| matches!(part, std::path::Component::Normal(_)));
        let generated = output.file_name().is_some_and(|name| name == "generated" || name == "Generated");
        if !stays_inside || !generated {
            problems.push(format!("{}: output `{}` must be a generated or Generated directory inside the repository", generator.name, generator.output_dir));
        }
        for other in registry.iter().filter(|other| other.name != generator.name) {
            if output.starts_with(other.output_dir) {
                problems.push(format!("{}: output `{}` lies in {}'s `{}`", generator.name, generator.output_dir, other.name, other.output_dir));
            }
        }
    }
    problems
}

/// Write every generator into the directory it owns.
fn write_registry(root: &Path, registry: &[Generator]) -> ExitCode {
    let mut failed = false;
    for generator in registry {
        let out_dir = root.join(generator.output_dir);
        match generate_into(generator, &out_dir) {
            Ok(()) => println!("generate: {} wrote {}", generator.name, generator.output_dir),
            Err(error) => {
                failed = true;
                println!("generate: {}: {error}", generator.name);
            }
        }
    }
    if failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

/// Regenerate everything into a scratch directory and compare it with what is committed.
fn check_against_registry(root: &Path, registry: &[Generator]) -> ExitCode {
    match crate::scratch::create("generate") {
        Ok(scratch) => check_in(root, registry, &scratch),
        Err(error) => {
            println!("generate: {error}");
            ExitCode::from(1)
        }
    }
}

/// The comparison, regenerating into `scratch`, which it removes whatever happens, so a run that fails
/// leaves nothing behind. Given the directory rather than making it, so a test can watch that one
/// directory go instead of counting a temporary folder that other runs write to as well.
fn check_in(root: &Path, registry: &[Generator], scratch: &Path) -> ExitCode {
    let mut failed = false;
    for generator in registry {
        let out_dir = scratch.join(generator.output_dir);
        let committed = root.join(generator.output_dir);
        match generate_into(generator, &out_dir) {
            Err(error) => {
                failed = true;
                println!("generate: {}: {error}", generator.name);
            }
            Ok(()) => match tree_differences(&out_dir, &committed) {
                Ok(differences) if differences.is_empty() => {}
                Ok(differences) => {
                    failed = true;
                    println!("generate: {} is not what the committed files say it should be", generator.name);
                    for difference in differences {
                        println!("generate: {difference}");
                    }
                }
                Err(error) => {
                    failed = true;
                    println!("generate: {}: {error}", generator.name);
                }
            },
        }
    }
    if let Err(error) = fs::remove_dir_all(scratch) {
        println!("generate: could not remove {}: {error}", scratch.display());
        failed = true;
    }
    if failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

/// A generator's own output, so a generator that left a file from a previous run in place cannot hide it
/// from the comparison.
fn generate_into(generator: &Generator, out_dir: &Path) -> Result<(), String> {
    if out_dir.exists() {
        fs::remove_dir_all(out_dir).map_err(|error| format!("could not clear {}: {error}", out_dir.display()))?;
    }
    fs::create_dir_all(out_dir).map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;
    (generator.generate)(out_dir)
}

/// Compare two directory trees byte for byte, listing every file that was added, removed or changed,
/// one line per file. A tree that does not exist is empty, so a generator whose output was never
/// committed shows every file as added.
pub(crate) fn tree_differences(regenerated: &Path, committed: &Path) -> Result<Vec<String>, String> {
    let fresh = collect_tree(regenerated)?;
    let existing = collect_tree(committed)?;
    let mut names: BTreeSet<&String> = BTreeSet::new();
    names.extend(fresh.keys());
    names.extend(existing.keys());
    let mut differences = Vec::new();
    for name in names {
        match (fresh.get(name), existing.get(name)) {
            (Some(bytes), Some(old)) if bytes == old => {}
            (Some(_), Some(_)) => differences.push(format!("{name}: changed")),
            (Some(_), None) => differences.push(format!("{name}: added (not committed)")),
            (None, Some(_)) => differences.push(format!("{name}: removed (committed, the generator no longer writes it)")),
            (None, None) => {}
        }
    }
    Ok(differences)
}

/// Every file under `root`, keyed by its path relative to `root` so two trees can be matched. A file or
/// directory that cannot be read fails the comparison: reading it as empty could match an empty file.
fn collect_tree(root: &Path) -> Result<Tree, String> {
    let mut files = Tree::new();
    if !root.exists() {
        return Ok(files);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|error| format!("{}: {error}", dir.display()))? {
            let path = entry.map_err(|error| format!("{}: {error}", dir.display()))?.path();
            // A committed tree on macOS gains a `.DS_Store` once Finder opens it; it was never generated.
            if path.file_name().is_some_and(|name| crate::layout::IGNORED_NAMES.iter().any(|ignored| name == *ignored)) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            files.insert(relative, bytes);
        }
    }
    Ok(files)
}

/// Relative path to bytes: one tree's files.
type Tree = std::collections::BTreeMap<String, Vec<u8>>;

/// A small generator used only by the tests below: it writes the files its name describes.
#[cfg(test)]
pub(crate) fn fake_generator(out_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(out_dir.join("nested")).map_err(|error| error.to_string())?;
    fs::write(out_dir.join("first.txt"), format!("// {} by the test generator\none\n", crate::generated::MARKER)).map_err(|error| error.to_string())?;
    fs::write(out_dir.join("nested/second.txt"), "two\n").map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn an_identical_tree_has_no_differences() {
        let fresh = Scratch::new("equal-fresh");
        let committed = Scratch::new("equal-committed");
        fake_generator(&fresh.0).unwrap();
        fake_generator(&committed.0).unwrap();
        assert!(tree_differences(&fresh.0, &committed.0).unwrap().is_empty());
    }

    #[test]
    fn a_changed_file_is_listed() {
        let fresh = Scratch::new("changed-fresh");
        let committed = Scratch::new("changed-committed");
        fake_generator(&fresh.0).unwrap();
        fake_generator(&committed.0).unwrap();
        committed.write("nested/second.txt", "two, edited by hand\n");
        let differences = tree_differences(&fresh.0, &committed.0).unwrap();
        assert_eq!(differences, vec!["nested/second.txt: changed"]);
    }

    #[test]
    fn an_added_and_a_removed_file_are_both_listed() {
        let fresh = Scratch::new("added-fresh");
        let committed = Scratch::new("added-committed");
        fake_generator(&fresh.0).unwrap();
        // The committed tree kept `first.txt` byte-identical and a file the generator no longer writes,
        // and lacks `surprise.txt` the generator now writes. Ordered by name: the removal sits between
        // the two additions, each difference on its own side of the comparison.
        committed.write("first.txt", fs::read_to_string(fresh.0.join("first.txt")).unwrap());
        committed.write("stale.txt", "the generator dropped this\n");
        fresh.write("surprise.txt", "new in the generator\n");
        let differences = tree_differences(&fresh.0, &committed.0).unwrap();
        assert_eq!(
            differences,
            vec![
                "nested/second.txt: added (not committed)",
                "stale.txt: removed (committed, the generator no longer writes it)",
                "surprise.txt: added (not committed)"
            ]
        );
    }

    #[test]
    fn a_folder_only_on_one_side_is_reported_by_its_files() {
        let fresh = Scratch::new("dir-fresh");
        let committed = Scratch::new("dir-committed");
        fake_generator(&fresh.0).unwrap();
        fs::remove_dir_all(fresh.0.join("nested")).unwrap();
        fake_generator(&committed.0).unwrap();
        let differences = tree_differences(&fresh.0, &committed.0).unwrap();
        assert_eq!(differences, vec!["nested/second.txt: removed (committed, the generator no longer writes it)"]);
    }

    #[test]
    fn a_finder_file_in_the_committed_tree_is_no_difference() {
        let fresh = Scratch::new("finder-fresh");
        let committed = Scratch::new("finder-committed");
        fake_generator(&fresh.0).unwrap();
        fake_generator(&committed.0).unwrap();
        committed.write("nested/.DS_Store", [0u8, 1]);
        assert!(tree_differences(&fresh.0, &committed.0).unwrap().is_empty());
    }

    #[test]
    fn a_missing_output_directory_counts_as_everything_added() {
        let fresh = Scratch::new("missing-fresh");
        let missing = Scratch::new("missing-committed");
        fs::remove_dir_all(&missing.0).unwrap();
        fake_generator(&fresh.0).unwrap();
        let differences = tree_differences(&fresh.0, &missing.0).unwrap();
        assert_eq!(differences.len(), 2, "{differences:?}");
    }

    /// ExitCode is opaque to read, so the tests compare it against the code the command promises.
    fn code(exit: std::process::ExitCode) -> u8 {
        if exit == std::process::ExitCode::SUCCESS {
            0
        } else if exit == std::process::ExitCode::from(1) {
            1
        } else {
            2
        }
    }

    #[test]
    fn generate_into_clears_the_output_before_writing() {
        let scratch = Scratch::new("clear");
        scratch.write("left-over.txt", "from a previous run\n");
        fake_generator(&scratch.0).unwrap();
        assert!(scratch.0.join("left-over.txt").exists(), "fake_generator does not clear; generate_into does");
        let generator = Generator { name: "fake", output_dir: "gen/generated", generate: fake_generator };
        generate_into(&generator, &scratch.0).unwrap();
        assert!(!scratch.0.join("left-over.txt").exists(), "a file from an earlier run cannot survive into the comparison");
        assert!(scratch.0.join("first.txt").exists());
    }

    #[test]
    fn the_check_path_matches_and_reports_a_committed_tree() {
        let root = Scratch::new("check-ok");
        root.write("gen/generated/first.txt", format!("// {} by the test generator\none\n", crate::generated::MARKER));
        root.write("gen/generated/nested/second.txt", "two\n");
        let registry = vec![Generator { name: "fake", output_dir: "gen/generated", generate: fake_generator }];
        assert_eq!(code(check_against_registry(&root.0, &registry)), 0);
    }

    #[test]
    fn the_check_path_fails_on_a_hand_edit_and_removes_its_scratch() {
        let root = Scratch::new("check-bad");
        root.write("gen/generated/first.txt", format!("// {} by the test generator\nchanged by hand\n", crate::generated::MARKER));
        let registry = vec![Generator { name: "fake", output_dir: "gen/generated", generate: fake_generator }];
        let scratch = crate::scratch::create("generate-test").unwrap();
        assert_eq!(code(check_in(&root.0, &registry, &scratch)), 1);
        assert!(!scratch.exists(), "the scratch directory must not outlive the run");
        assert!(
            fs::read_to_string(root.0.join("gen/generated/first.txt")).unwrap().contains("changed by hand"),
            "the check must not touch the committed files"
        );
    }

    #[test]
    fn a_failing_generator_is_reported_and_leaves_nothing_behind() {
        fn failing(_out_dir: &Path) -> Result<(), String> {
            Err("the engine's registry could not be read".to_string())
        }
        let root = Scratch::new("failing");
        let registry = vec![Generator { name: "failing", output_dir: "gen/generated", generate: failing }];
        assert_eq!(code(check_against_registry(&root.0, &registry)), 1);
        assert!(!root.0.join("gen").exists(), "a failed generator must not write into the repository");
        assert_eq!(code(write_registry(&root.0, &registry)), 1);
    }

    #[test]
    fn an_output_directory_that_is_not_a_generated_one_is_refused() {
        let generator = |name: &'static str, output_dir: &'static str| Generator { name, output_dir, generate: fake_generator };
        assert!(registry_problems(&[generator("a", "ios/sdk/Sources/CraftBridgeFFI/Generated"), generator("b", "rust/x/src/generated")]).is_empty());
        for unsafe_output in ["", ".", "rust", "../generated", "rust/../generated", "/tmp/generated"] {
            assert!(!registry_problems(&[generator("a", unsafe_output)]).is_empty(), "{unsafe_output:?} must be refused");
        }
        let nested = registry_problems(&[generator("a", "x/generated"), generator("b", "x/generated/y/generated")]);
        assert!(nested.iter().any(|problem| problem.starts_with("b:")), "{nested:?}");
    }

    #[test]
    fn the_registry_is_empty_until_a_generator_lands() {
        // The registry being empty is the repository's state today: `generate` and `generate --check`
        // both say so and succeed.
        let root = Scratch::new("empty");
        assert!(generators().is_empty());
        assert_eq!(code(run(&root.0, false)), 0);
        assert_eq!(code(run(&root.0, true)), 0);
    }
}
