//! `cargo xtask guide-samples`: the code blocks of the guides are quotations of compiled code, not copies.
//!
//! Every fenced `swift` and `kotlin` block of the guides — the Markdown under `docs`, bar the reference the
//! generator writes into `docs/generated` — must be a run of lines of one file the samples' builds compile:
//! a Swift file an iOS sample's project compiles, or a Kotlin file of an Android sample's build, as
//! `crate::samples` lists them from its one definition of where the samples are. A block may move the run
//! left as a whole, so that a guide shows code from inside a type at the margin, and changes nothing else:
//! each line keeps its indentation against the others, so a block that differs from the code only in how it
//! is indented quotes nothing. `swift_build` and `android_samples` compile those files against the SDK, so a
//! block that is such a run is code the gate builds, and a block that is not is refused, naming the guide
//! and the line.
//!
//! What a comment holds is no compiled code. A line comment marks its own line, so code commented out line by
//! line is no longer the run a block quotes, and a block made only of comment lines quotes no code at all and
//! is refused; swift-format's `NoBlockComments` refuses every Swift block comment the gate reads.
//!
//! The Markdown is read only as far as its code blocks, and every block must say what it holds: a fence of
//! three or more backquotes or tildes opens one, the next fence of the same mark and at least its length
//! closes it, and the words after the opening fence name its language. A fence that names no language, or one
//! in neither list below, a fence never closed, and a block made by indentation alone, which can name none,
//! are each refused: a block the check skipped would be one nobody promised anything about.

use crate::scan;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// A fenced language whose blocks are code a build compiles: the word its fences name it by, what compiles
/// it, and where that build's files are listed.
struct CodeLanguage {
    fence: &'static str,
    builds: &'static str,
    sources: fn(&Path) -> Result<Vec<PathBuf>, String>,
}

/// The languages whose every block must be a run of a compiled file.
const CODE_LANGUAGES: [CodeLanguage; 2] = [
    CodeLanguage { fence: "swift", builds: "the iOS samples' projects", sources: crate::samples::ios_sources },
    CodeLanguage { fence: "kotlin", builds: "the Android samples' builds", sources: crate::samples::android_sources },
];

/// The fenced languages that carry guide text — terminal lines, JSON, manifests, diagrams. They are shown as
/// written and nothing outside a compiler promises anything about them; a fence naming a language that is in
/// neither list is refused, because an unknown fence is usually a misspelling of a code language and would
/// otherwise skip the check unnoticed.
const TEXT_LANGUAGES: [&str; 10] = ["bash", "sh", "text", "json", "jsonl", "toml", "yaml", "xml", "mermaid", "gradle.kts"];

/// The columns of indentation at which Markdown reads a line outside a fence as a code block of its own.
const INDENTED_CODE: usize = 4;

/// The guide files: every Markdown file under `docs`, bar the generated reference, which the docs generator
/// writes and its own check guards. The walk is the gate's one walk, so build output is no guide.
fn guide_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let generated = root.join("docs/generated");
    Ok(scan::walk(&root.join("docs"), ".md")?.into_iter().map(|file| file.path).filter(|path| !path.starts_with(&generated)).collect())
}

/// One fenced block: its language, its opening fence's line in the guide, and its lines.
struct Block {
    language: String,
    line: usize,
    lines: Vec<String>,
}

/// The columns a line's leading whitespace takes, a tab reaching the next multiple of four as Markdown counts it.
fn indentation(line: &str) -> usize {
    line.chars()
        .take_while(|character| character.is_whitespace())
        .fold(0, |columns, character| if character == '\t' { columns + 4 - columns % 4 } else { columns + 1 })
}

/// The fence `line` is, when it is one: its mark, how many of it there are, and the words after them. A run of
/// backquotes followed by another backquote is code inside a line, not a fence.
fn fence(line: &str) -> Option<(char, usize, &str)> {
    if indentation(line) >= INDENTED_CODE {
        return None;
    }
    let rest = line.trim_start();
    let mark = rest.chars().next().filter(|mark| *mark == '`' || *mark == '~')?;
    let length = rest.chars().take_while(|character| *character == mark).count();
    let words = rest.get(length..)?.trim();
    if length < 3 || (mark == '`' && words.contains('`')) {
        return None;
    }
    Some((mark, length, words))
}

/// The fenced blocks of a guide's text, and, by line, what of the text is a block this reading cannot take.
fn blocks(text: &str) -> (Vec<Block>, Vec<(usize, &'static str)>) {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    let mut open: Option<(char, usize, Block)> = None;
    // A line indented as code only continues a paragraph the line before it is part of; anywhere else it
    // opens a code block of its own.
    let mut in_paragraph = false;
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let fenced = fence(line);
        let closes = |(mark, length): (char, usize)| fenced.is_some_and(|(closing, count, words)| closing == mark && count >= length && words.is_empty());
        if open.as_ref().is_some_and(|(mark, length, _)| closes((*mark, *length))) {
            found.extend(open.take().map(|(_, _, block)| block));
            in_paragraph = false;
            continue;
        }
        if let Some((_, _, block)) = &mut open {
            block.lines.push(line.to_string());
            continue;
        }
        if let Some((mark, length, words)) = fenced {
            open = Some((mark, length, Block { language: words.to_string(), line: number, lines: Vec::new() }));
            in_paragraph = false;
            continue;
        }
        let blank = line.trim().is_empty();
        if !blank && !in_paragraph && indentation(line) >= INDENTED_CODE {
            problems
                .push((number, "a line indented four columns or more outside a fence is a code block that names no language; fence it, naming its language"));
        }
        in_paragraph = !blank && !line.trim_start().starts_with('#');
    }
    if let Some((_, _, block)) = open {
        problems.push((block.line, "a fence opened here is never closed, so its block runs to the guide's end"));
    }
    (found, problems)
}

/// The lines of a block with the block's own common indentation removed; a line that is empty holds nothing
/// to remove.
fn dedent<S: AsRef<str>>(lines: &[S]) -> Vec<String> {
    let common = lines.iter().map(AsRef::as_ref).filter(|line| !line.trim().is_empty()).map(|line| line.len() - line.trim_start().len()).min().unwrap_or(0);
    lines.iter().map(AsRef::as_ref).map(|line| line.get(common.min(line.len())..).unwrap_or("").trim_end().to_string()).collect()
}

/// Whether `block`, dedented, is a run of `source`: some lines of it in a row which, moved left together by
/// their own common indentation, are the block's lines exactly.
fn quoted_in(block: &[String], source: &[String]) -> bool {
    !block.is_empty() && source.windows(block.len()).any(|run| dedent(run) == block)
}

/// Whether every line of `block` that holds anything is a line comment, which both languages mark with `//`.
fn only_comments(block: &[String]) -> bool {
    block.iter().filter(|line| !line.trim().is_empty()).all(|line| line.trim_start().starts_with("//"))
}

/// Run `cargo xtask guide-samples`.
pub(crate) fn run(root: &Path) -> ExitCode {
    match check(root) {
        Ok(count) => {
            println!("guide-samples: {count} guide blocks are runs of files the samples' builds compile.");
            ExitCode::SUCCESS
        }
        Err(problems) => {
            for problem in &problems {
                eprintln!("guide-samples: {problem}");
            }
            eprintln!("guide-samples: FAILED, {} problem(s).", problems.len());
            ExitCode::FAILURE
        }
    }
}

/// Each code language's compiled files, as lines, from the samples of the tree at `root`.
type Sources = Vec<(&'static str, Vec<Vec<String>>)>;

fn check(root: &Path) -> Result<usize, Vec<String>> {
    let mut guides = Vec::new();
    for guide in guide_files(root).map_err(|problem| vec![problem])? {
        let text = scan::read_text(&guide).map_err(|problem| vec![format!("cannot read a guide: {problem}")])?;
        guides.push((guide, text));
    }
    if guides.is_empty() {
        return Err(vec!["no guide files found under docs/".to_string()]);
    }
    let mut sources: Sources = Vec::new();
    for language in &CODE_LANGUAGES {
        let files = (language.sources)(root).map_err(|problem| vec![problem])?;
        if files.is_empty() {
            return Err(vec![format!("no {} sources found: {} compile none to match {} blocks against", language.fence, language.builds, language.fence)]);
        }
        let texts = files
            .iter()
            .map(|file| scan::read_text(file).map(|text| text.lines().map(str::to_string).collect()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|problem| vec![format!("cannot read a {} source: {problem}", language.fence)])?;
        sources.push((language.fence, texts));
    }
    check_guides(&guides, &sources)
}

/// Check every block of `guides`, each a guide's path and text, against `sources`, answering how many code
/// blocks are compiled runs, or every problem found.
fn check_guides(guides: &[(PathBuf, String)], sources: &Sources) -> Result<usize, Vec<String>> {
    let mut problems = Vec::new();
    let mut count = 0usize;
    for (guide, text) in guides {
        let at = |line: usize| format!("{}:{line}", guide.display());
        let (blocks, unread) = blocks(text);
        problems.extend(unread.into_iter().map(|(line, problem)| format!("{}: {problem}", at(line))));
        for block in blocks {
            let name = block.language.as_str();
            let Some(language) = CODE_LANGUAGES.iter().find(|language| language.fence == name) else {
                if name.is_empty() {
                    problems
                        .push(format!("{}: a fence names no language, so nothing says whether its block is code to check; name its language", at(block.line)));
                } else if !TEXT_LANGUAGES.contains(&name) {
                    problems.push(format!("{}: a fence names {name:?}, which is neither a compiled language nor a listed text one", at(block.line)));
                }
                continue;
            };
            let wanted = dedent(&block.lines);
            if wanted.iter().all(|line| line.is_empty()) {
                problems.push(format!("{}: an empty {name} block teaches nothing", at(block.line)));
            } else if only_comments(&wanted) {
                problems.push(format!("{}: this {name} block holds only comment lines, which no build compiles", at(block.line)));
            } else if sources.iter().filter(|(fence, _)| *fence == language.fence).flat_map(|(_, files)| files).any(|source| quoted_in(&wanted, source)) {
                count += 1;
            } else {
                problems.push(format!(
                    "{}: this {name} block is no run of a file {} compile: its lines must appear in order in one such file, each indented against the others as it is there",
                    at(block.line),
                    language.builds
                ));
            }
        }
    }
    if problems.is_empty() { Ok(count) } else { Err(problems) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    /// The compiled sources the guides below are checked against: one Swift file and one Kotlin file.
    fn sources() -> Sources {
        let lines = |text: &str| text.lines().map(str::to_string).collect::<Vec<_>>();
        vec![
            ("swift", vec![lines("// The studio.\nstruct Studio {\n    func start() -> Int {\n        42\n    }\n}\n")]),
            ("kotlin", vec![lines("/** The studio. */\nfun start(): Int =\n    42\n")]),
        ]
    }

    /// The problems `text`, as `docs/guide.md`, has against [`sources`], or how many blocks it quotes.
    fn checked(text: &str) -> Result<usize, Vec<String>> {
        check_guides(&[(PathBuf::from("docs/guide.md"), text.to_string())], &sources())
    }

    #[test]
    fn blocks_that_are_runs_of_the_compiled_sources_pass_at_their_own_margin_or_the_sources() {
        let guide = "# Guide\n\n```swift\nfunc start() -> Int {\n    42\n}\n```\n\n```swift\n    func start() -> Int {\n        42\n    }\n```\n\n~~~kotlin\nfun start(): Int =\n    42\n~~~\n";
        assert_eq!(checked(guide), Ok(3));
    }

    #[test]
    fn a_block_that_is_no_run_names_the_guide_the_line_and_what_compiles_its_language() {
        let wrong = checked("# Guide\n\n```swift\nfunc start() -> Int {\n    41\n}\n```\n\n```kotlin\nfun start(): Int = 41\n```\n");
        assert_eq!(
            wrong,
            Err(vec![
                "docs/guide.md:3: this swift block is no run of a file the iOS samples' projects compile: its lines must appear in order in one such file, each indented against the others as it is there".to_string(),
                "docs/guide.md:9: this kotlin block is no run of a file the Android samples' builds compile: its lines must appear in order in one such file, each indented against the others as it is there".to_string(),
            ])
        );
    }

    /// Indentation is part of the quotation: a block whose lines match the source's only once each is trimmed
    /// on its own is no run of it.
    #[test]
    fn a_block_indented_otherwise_than_its_source_is_no_run() {
        let problems = checked("# Guide\n\n```swift\nfunc start() -> Int {\n42\n}\n```\n").unwrap_err();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems.iter().all(|problem| problem.starts_with("docs/guide.md:3: this swift block is no run")), "{problems:?}");
    }

    /// A comment holds no compiled code: a block of comment lines alone is refused even where the source holds
    /// those very lines, while a comment quoted with the code it sits on is a run like any other.
    #[test]
    fn a_block_of_comment_lines_alone_quotes_no_code() {
        assert_eq!(
            checked("# Guide\n\n```swift\n// The studio.\n```\n"),
            Err(vec!["docs/guide.md:3: this swift block holds only comment lines, which no build compiles".to_string()])
        );
        assert_eq!(checked("# Guide\n\n```swift\n// The studio.\nstruct Studio {\n```\n"), Ok(1));
    }

    #[test]
    fn every_block_the_reading_cannot_take_is_refused_by_its_line() {
        let guide = "# Guide\n\n```swift\n\n```\n\n```swiftt\nx\n```\n\n```\nfunc start() -> Int {\n```\n\nprose\n\n    func start() -> Int {\n\n````swift\nfunc start() -> Int {\n```\n";
        assert_eq!(
            checked(guide),
            Err(vec![
                "docs/guide.md:17: a line indented four columns or more outside a fence is a code block that names no language; fence it, naming its language"
                    .to_string(),
                "docs/guide.md:19: a fence opened here is never closed, so its block runs to the guide's end".to_string(),
                "docs/guide.md:3: an empty swift block teaches nothing".to_string(),
                "docs/guide.md:7: a fence names \"swiftt\", which is neither a compiled language nor a listed text one".to_string(),
                "docs/guide.md:11: a fence names no language, so nothing says whether its block is code to check; name its language".to_string(),
            ])
        );
    }

    /// A line that only continues a paragraph is no code block, however it is indented, and text fences pass
    /// as written.
    #[test]
    fn listed_text_fences_and_indented_paragraph_lines_pass() {
        assert_eq!(checked("# Guide\n\n```bash\n./tools/ci.sh rules\n```\n\n~~~text\nany prose at all\n~~~\n\nA paragraph\n    that goes on.\n"), Ok(0));
    }

    #[test]
    fn the_generated_reference_is_no_guide() {
        let root = Scratch::new("guide-samples-generated");
        root.write("docs/guide.md", "# Guide\n");
        root.write("docs/generated/commands.md", "<!-- @generated -->\n\n```kotlin\nnot a quotation at all\n```\n");
        assert_eq!(guide_files(&root.0).unwrap(), [root.0.join("docs/guide.md")]);
    }

    #[test]
    fn a_tree_with_no_guides_is_refused() {
        let root = Scratch::new("guide-samples-no-guides");
        root.write("docs/notes.txt", "not a guide\n");
        assert_eq!(check(&root.0), Err(vec!["no guide files found under docs/".to_string()]));
    }

    /// The check reads what the samples' builds compile, through `crate::samples`: a Swift file in a sample's
    /// folder that its project does not compile is no source a block may quote, and a tree whose samples compile
    /// no Kotlin is refused rather than passed.
    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "reads the sample's project through plutil, which only macOS has")]
    fn only_what_the_samples_compile_may_be_quoted() {
        let root = Scratch::new("guide-samples-compiled");
        root.write(
            "ios/samples/App/App.xcodeproj/project.pbxproj",
            "{ objects = { ROOT = { isa = PBXProject; mainGroup = MAIN; targets = (APP); }; \
             MAIN = { isa = PBXGroup; children = (A); sourceTree = \"<group>\"; }; \
             A = { isa = PBXFileReference; path = App/Listed.swift; sourceTree = \"<group>\"; }; \
             BA = { isa = PBXBuildFile; fileRef = A; }; SOURCES = { isa = PBXSourcesBuildPhase; files = (BA); }; \
             APP = { isa = PBXNativeTarget; name = App; buildPhases = (SOURCES); }; }; rootObject = ROOT; }",
        );
        root.write("ios/samples/App/App/Listed.swift", "func listed() {}\n");
        root.write("ios/samples/App/App/Unlisted.swift", "func unlisted() {}\n");
        root.write("android/samples/app/settings.gradle.kts", "");
        root.write("docs/guide.md", "# Guide\n\n```swift\nfunc listed() {}\n```\n\n```swift\nfunc unlisted() {}\n```\n");
        assert_eq!(check(&root.0), Err(vec!["no kotlin sources found: the Android samples' builds compile none to match kotlin blocks against".to_string()]));
        root.write("android/samples/app/app/src/main/kotlin/App.kt", "fun listed() {}\n");
        let guide = root.0.join("docs/guide.md");
        assert_eq!(
            check(&root.0),
            Err(vec![format!(
                "{}:7: this swift block is no run of a file the iOS samples' projects compile: its lines must appear in order in one such file, each indented against the others as it is there",
                guide.display()
            )])
        );
    }
}
