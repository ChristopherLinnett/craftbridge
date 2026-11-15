//! Rule 6 — `cargo xtask generate`: the only thing allowed to write a generated file.
//!
//! A generated file is committed and never edited by hand, so the generator that writes it has to be the
//! only writer, and the gate has to prove nobody hand-edited one. Both come from one registry: each
//! generator names the output directory it owns and writes its files there. `generate` runs each into a
//! scratch folder and replaces the real directory only when it succeeded; `generate --check` runs them
//! into a scratch copy and compares it byte for byte with the working tree, so a hand edit or a generator
//! that depends on the date fails the gate with the file named, and asks git whether every file there is
//! tracked, because a comparison with the working tree alone would pass files that were never added or
//! that a `.gitignore` hides.
//!
//! Every generator registers itself in [`generators`], in the change that adds it, so the gate regenerates
//! and compares its files from that change on.

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
    vec![
        Generator { name: "effects-api-surface", output_dir: crate::effects_api::OUTPUT_DIR, generate: crate::effects_api::generate },
        Generator { name: "effects-swift-facade", output_dir: crate::effects_api::SWIFT_OUTPUT_DIR, generate: crate::effects_api::generate_swift },
    ]
}

/// Run `cargo xtask generate`, with or without `--check`.
pub(crate) fn run(root: &Path, check: bool) -> ExitCode {
    let registry = generators();
    let problems = registry_problems(&registry);
    if !problems.is_empty() {
        for problem in problems {
            println!("generate: {problem}");
        }
        return ExitCode::from(1);
    }
    if check { check_against_registry(root, &registry) } else { write_registry(root, &registry) }
}

/// What makes a registry unsafe to run. A generator's output directory is replaced by every run, so it
/// must be a `generated`/`Generated` directory inside one of the trees `rules: generated` reads — never
/// the root, a source folder, `plan/` or a path that climbs out — and two generators may neither share a
/// name, which every message goes by, nor share or nest an output, or each run would delete the other's
/// files.
fn registry_problems(registry: &[Generator]) -> Vec<String> {
    let mut problems = Vec::new();
    for (index, generator) in registry.iter().enumerate() {
        let output = Path::new(generator.output_dir);
        let stays_inside = output.components().all(|part| matches!(part, std::path::Component::Normal(_)));
        let generated = output.file_name().is_some_and(|name| name == "generated" || name == "Generated");
        if !stays_inside || !generated {
            problems.push(format!("{}: output `{}` must be a generated or Generated directory inside the repository", generator.name, generator.output_dir));
        }
        if !crate::files::CODE_TREES.iter().any(|tree| output.starts_with(tree)) {
            problems.push(format!(
                "{}: output `{}` lies outside the trees `rules: generated` reads ({})",
                generator.name,
                generator.output_dir,
                crate::files::CODE_TREES.join(", ")
            ));
        }
        if registry.iter().take(index).any(|earlier| earlier.name == generator.name) {
            problems.push(format!("{}: a second generator of that name", generator.name));
        }
        for (other_index, other) in registry.iter().enumerate().filter(|(other_index, _)| *other_index != index) {
            if output.starts_with(other.output_dir) && (output != Path::new(other.output_dir) || other_index < index) {
                problems.push(format!("{}: output `{}` lies in {}'s `{}`", generator.name, generator.output_dir, other.name, other.output_dir));
            }
        }
    }
    problems
}

/// Write every generator into the directory it owns: into a scratch folder first, and over the committed
/// directory only once the generator succeeded, so a generator that fails leaves the committed files as
/// they were rather than deleted.
fn write_registry(root: &Path, registry: &[Generator]) -> ExitCode {
    let mut failed = false;
    for generator in registry {
        let written = crate::scratch::create("generate-write").and_then(|scratch| {
            let fresh = scratch.join("out");
            let result = generate_into(generator, &fresh).and_then(|()| replace_tree(&fresh, &root.join(generator.output_dir)));
            let _ = fs::remove_dir_all(&scratch);
            result
        });
        match written {
            Ok(()) => println!("generate: {} wrote {}", generator.name, generator.output_dir),
            Err(error) => {
                failed = true;
                println!("generate: {}: {error}", generator.name);
            }
        }
    }
    if failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

/// Replace the tree at `to` with a copy of the tree at `from`.
fn replace_tree(from: &Path, to: &Path) -> Result<(), String> {
    if to.exists() {
        fs::remove_dir_all(to).map_err(|error| format!("could not clear {}: {error}", to.display()))?;
    }
    let mut stack = vec![from.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let target = to.join(dir.strip_prefix(from).unwrap_or(&dir));
        fs::create_dir_all(&target).map_err(|error| format!("could not create {}: {error}", target.display()))?;
        for entry in fs::read_dir(&dir).map_err(|error| format!("{}: {error}", dir.display()))? {
            let path = entry.map_err(|error| format!("{}: {error}", dir.display()))?.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(name) = path.file_name() {
                fs::copy(&path, target.join(name)).map_err(|error| format!("could not copy {}: {error}", path.display()))?;
            }
        }
    }
    Ok(())
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
            Ok(()) => match tree_differences(&out_dir, &committed, "the generator", "the working tree") {
                Ok(differences) if differences.is_empty() => {}
                Ok(differences) => {
                    failed = true;
                    println!("generate: {} ({}) is not what the committed files say it should be", generator.name, generator.output_dir);
                    for difference in differences {
                        println!("generate: {}/{difference}", generator.output_dir.trim_end_matches('/'));
                    }
                }
                Err(error) => {
                    failed = true;
                    println!("generate: {}: {error}", generator.name);
                }
            },
        }
        match uncommitted(root, generator.output_dir) {
            Ok(files) => {
                for file in files {
                    failed = true;
                    println!("generate: {file}: git does not track it, so no commit holds the file the gate compared");
                }
            }
            Err(error) => {
                failed = true;
                println!("generate: {}: cannot ask git which files it tracks: {error}", generator.output_dir);
            }
        }
    }
    if let Err(error) = fs::remove_dir_all(scratch) {
        println!("generate: could not remove {}: {error}", scratch.display());
        failed = true;
    }
    if failed { ExitCode::from(1) } else { ExitCode::SUCCESS }
}

/// The files under `output_dir` git does not track, ignored ones included (`git ls-files --others` applies
/// no ignore rules unless asked to), as paths from the root, bar the names no rule counts: Finder's
/// `.DS_Store`, which the comparison skips too. Outside a git checkout there is no commit to compare with,
/// so there is nothing to report; inside one, a git that cannot answer is an error, never an empty answer,
/// or the check would pass files nobody looked at.
fn uncommitted(root: &Path, output_dir: &str) -> Result<Vec<String>, String> {
    if !crate::git::is_checkout(root) {
        return Ok(Vec::new());
    }
    let files = crate::git::listed(root, &["ls-files", "-z", "--others", "--", output_dir])?;
    Ok(files.into_iter().filter(|file| !file.rsplit('/').next().is_some_and(|name| crate::layout::IGNORED_NAMES.contains(&name))).collect())
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
/// written shows every file as added. `writer` and `tree` are what a line calls the one that wrote the
/// fresh tree and the one it is compared against, so the check that rebuilds something says so in its
/// own words: the same comparison stands behind `generate --check` and `plan-views --check`.
pub(crate) fn tree_differences(regenerated: &Path, committed: &Path, writer: &str, tree: &str) -> Result<Vec<String>, String> {
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
            (Some(_), None) => differences.push(format!("{name}: {writer} writes it, and {tree} does not hold it")),
            (None, Some(_)) => differences.push(format!("{name}: in {tree}, but not written by {writer}")),
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
        assert!(tree_differences(&fresh.0, &committed.0, "the generator", "the working tree").unwrap().is_empty());
    }

    #[test]
    fn a_changed_file_is_listed() {
        let fresh = Scratch::new("changed-fresh");
        let committed = Scratch::new("changed-committed");
        fake_generator(&fresh.0).unwrap();
        fake_generator(&committed.0).unwrap();
        committed.write("nested/second.txt", "two, edited by hand\n");
        let differences = tree_differences(&fresh.0, &committed.0, "the generator", "the working tree").unwrap();
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
        let differences = tree_differences(&fresh.0, &committed.0, "the generator", "the working tree").unwrap();
        assert_eq!(
            differences,
            vec![
                "nested/second.txt: the generator writes it, and the working tree does not hold it",
                "stale.txt: in the working tree, but not written by the generator",
                "surprise.txt: the generator writes it, and the working tree does not hold it"
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
        let differences = tree_differences(&fresh.0, &committed.0, "the generator", "the working tree").unwrap();
        assert_eq!(differences, vec!["nested/second.txt: in the working tree, but not written by the generator"]);
    }

    #[test]
    fn a_finder_file_in_the_committed_tree_is_no_difference() {
        let fresh = Scratch::new("finder-fresh");
        let committed = Scratch::new("finder-committed");
        fake_generator(&fresh.0).unwrap();
        fake_generator(&committed.0).unwrap();
        committed.write("nested/.DS_Store", [0u8, 1]);
        assert!(tree_differences(&fresh.0, &committed.0, "the generator", "the working tree").unwrap().is_empty());
    }

    #[test]
    fn a_missing_output_directory_counts_as_everything_added() {
        let fresh = Scratch::new("missing-fresh");
        let missing = Scratch::new("missing-committed");
        fs::remove_dir_all(&missing.0).unwrap();
        fake_generator(&fresh.0).unwrap();
        let differences = tree_differences(&fresh.0, &missing.0, "the generator", "the working tree").unwrap();
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
    fn a_registry_with_two_names_alike_or_an_output_outside_the_trees_is_refused() {
        let generator = |name: &'static str, output_dir: &'static str| Generator { name, output_dir, generate: fake_generator };
        let twice = registry_problems(&[generator("a", "rust/x/generated"), generator("a", "rust/y/generated")]);
        assert!(twice.iter().any(|problem| problem == "a: a second generator of that name"), "{twice:?}");
        let shared = registry_problems(&[generator("a", "rust/x/generated"), generator("b", "rust/x/generated")]);
        assert_eq!(shared.len(), 1, "one output shared by two generators is reported once: {shared:?}");
        let plan = registry_problems(&[generator("a", "plan/generated")]);
        assert!(plan.iter().any(|problem| problem.contains("outside the trees")), "{plan:?}");
    }

    #[test]
    fn a_failing_write_leaves_the_committed_files_in_place() {
        fn failing(out_dir: &Path) -> Result<(), String> {
            fs::write(out_dir.join("half.txt"), "half\n").map_err(|error| error.to_string())?;
            Err("the engine's registry could not be read".to_string())
        }
        let root = Scratch::new("write-failing");
        root.write("conformance/generated/first.txt", "committed\n");
        let registry = vec![Generator { name: "failing", output_dir: "conformance/generated", generate: failing }];
        assert_eq!(code(write_registry(&root.0, &registry)), 1);
        assert_eq!(fs::read_to_string(root.0.join("conformance/generated/first.txt")).unwrap(), "committed\n");
        assert!(!root.0.join("conformance/generated/half.txt").exists());
        // A generator that succeeds replaces the directory with exactly what it wrote.
        let registry = vec![Generator { name: "fake", output_dir: "conformance/generated", generate: fake_generator }];
        assert_eq!(code(write_registry(&root.0, &registry)), 0);
        assert!(root.0.join("conformance/generated/nested/second.txt").exists() && !root.0.join("conformance/generated/half.txt").exists());
    }

    #[test]
    fn generated_files_git_does_not_track_fail_the_check() {
        let root = Scratch::new("check-untracked");
        let git = |args: &[&str]| crate::git::output(&root.0, args).unwrap();
        git(&["init", "-q"]);
        fake_generator(&root.0.join("conformance/generated")).unwrap();
        let registry = vec![Generator { name: "fake", output_dir: "conformance/generated", generate: fake_generator }];
        assert_eq!(uncommitted(&root.0, "conformance/generated").unwrap(), ["conformance/generated/first.txt", "conformance/generated/nested/second.txt"]);
        assert_eq!(code(check_against_registry(&root.0, &registry)), 1, "the files match but no commit holds them");
        // An ignored file is no more committed than an untracked one.
        root.write(".gitignore", "conformance/generated/\n");
        assert_eq!(uncommitted(&root.0, "conformance/generated").unwrap().len(), 2);
        fs::remove_file(root.0.join(".gitignore")).unwrap();
        git(&["add", "conformance"]);
        assert!(uncommitted(&root.0, "conformance/generated").unwrap().is_empty());
        assert_eq!(code(check_against_registry(&root.0, &registry)), 0);
        // Finder's file in a tracked output is no difference and nothing git should hold.
        root.write("conformance/generated/.DS_Store", [0u8, 1]);
        root.write("conformance/generated/nested/.DS_Store", [0u8, 1]);
        assert!(uncommitted(&root.0, "conformance/generated").unwrap().is_empty());
        assert_eq!(code(check_against_registry(&root.0, &registry)), 0);
    }

    #[test]
    fn a_git_that_cannot_answer_in_a_checkout_fails_the_check() {
        let root = Scratch::new("check-git-broken");
        root.write(".git", "not a gitdir line\n");
        fake_generator(&root.0.join("conformance/generated")).unwrap();
        assert!(uncommitted(&root.0, "conformance/generated").is_err(), "a checkout whose git fails is not a tree with nothing untracked");
        let registry = vec![Generator { name: "fake", output_dir: "conformance/generated", generate: fake_generator }];
        assert_eq!(code(check_against_registry(&root.0, &registry)), 1);
        // A scratch tree with no `.git` at all has no commit to compare with.
        let scratch = Scratch::new("check-no-git");
        fake_generator(&scratch.0.join("conformance/generated")).unwrap();
        assert!(uncommitted(&scratch.0, "conformance/generated").unwrap().is_empty());
    }

    #[test]
    fn the_registry_is_safe_to_run_and_holds_the_effects_surface_and_its_swift_facade() {
        let registry = generators();
        assert!(registry_problems(&registry).is_empty(), "{:?}", registry_problems(&registry));
        assert!(registry.iter().any(|generator| generator.name == "effects-api-surface" && generator.output_dir.ends_with("/generated")));
        assert!(registry.iter().any(|generator| generator.name == "effects-swift-facade" && generator.output_dir.ends_with("/CraftBridgeEffects/Generated")));
    }
}
