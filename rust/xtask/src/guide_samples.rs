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
//! What a build compiles to nothing is no compiled code either, and the gate never guesses at it. A line is
//! dead when every non-blank character of it sits in a comment, a KDoc, or a string body, or in an `#if`
//! branch no shipping build compiles. Kotlin answers those of its files with `CompilesToNothing`, a rule of
//! the build's own detekt rules run by the Android builds themselves; Swift answers its files with
//! `tools/guide-swift-lines.swift`, which reads them with the pinned Xcode's swift-syntax on every build
//! `swift_build` makes of the project, under the conditions Xcode resolves for each (AGENTS §10). A block quoted wholly from
//! dead lines quotes prose dressed as code and is refused; a KDoc quoted beside the code it documents
//! stands, because the code's lines live. A line comment marks its own line, so code commented out line by
//! line is no longer the run a block quotes, and a block made only of comment lines quotes no code at all
//! and is refused; swift-format's `NoBlockComments` refuses every Swift block comment the gate reads.
//!
//! The Markdown is read only as far as its code blocks, and every block must say what it holds: a fence of
//! three or more backquotes or tildes opens one, the next fence of the same mark and at least its length
//! closes it, and the words after the opening fence name its language. A fence that names no language, or one
//! in neither list below, a fence never closed, and a block made by indentation alone, which can name none,
//! are each refused: a block the check skipped would be one nobody promised anything about.

use crate::scan;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// One file a sample build compiles: its lines, and the lines of it a build compiles to nothing — those
/// whose every non-blank character sits in a comment, a KDoc, a string body, or an `#if` branch no
/// shipping build compiles. Kotlin answers those from the gate's own detekt rule over the folders AGP's
/// variant API lists, Swift from `tools/guide-swift-lines.swift` over the files the projects' build
/// phases list, each language read by its own parser (AGENTS §10).
pub(crate) struct Source {
    pub(crate) path: PathBuf,
    pub(crate) lines: Vec<String>,
    pub(crate) dead: std::collections::BTreeSet<usize>,
}

/// A fenced language whose blocks are code a build compiles: the word its fences name it by, what compiles
/// it, and where that build's files and their dead lines are answered.
struct CodeLanguage {
    fence: &'static str,
    builds: &'static str,
    sources: fn(&Path) -> Result<Vec<Source>, String>,
}

/// The languages whose every block must be a run of a compiled file.
const CODE_LANGUAGES: [CodeLanguage; 2] = [
    CodeLanguage { fence: "swift", builds: "the iOS samples' projects", sources: crate::samples::swift_guide_sources },
    CodeLanguage { fence: "kotlin", builds: "the Android samples' builds", sources: crate::samples::android_guide_sources },
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

/// The lines of `block`, dedented, are a run of `source` at each returned start: some lines of it in a row
/// which, moved left together by their own common indentation, are the block's lines exactly. A run starts
/// where its first line sits, counted from one; several runs may match, and a quotation that sits in one
/// file wholly is the same quotation wherever else its lines might line up.
fn quoted_runs(block: &[String], source: &Source) -> Vec<usize> {
    if block.is_empty() {
        return Vec::new();
    }
    source.lines.as_slice().windows(block.len()).enumerate().filter(|(_, run)| dedent(run) == block).map(|(start, _)| start + 1).collect()
}

/// Whether every line of a run from `start` is a line the build compiles to nothing: a block quoted wholly
/// from such lines quotes no code, only what the compiler never sees.
fn wholly_dead(block: &[String], source: &Source, start: usize) -> bool {
    (start..start + block.len()).all(|line| source.dead.contains(&line))
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

/// Each code language's compiled files, each with the lines its build compiles to nothing, from the samples
/// of the tree at `root`.
type Sources = Vec<(&'static str, Vec<Source>)>;

/// The refusal when a language's builds compile no source at all: with nothing compiled to match a guide's
/// blocks against, no block could be proved, and a quiet pass would be the wrong answer.
fn no_sources(language: &CodeLanguage) -> String {
    format!("no {} sources found: {} compile none to match {} blocks against", language.fence, language.builds, language.fence)
}

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
            return Err(vec![no_sources(language)]);
        }
        sources.push((language.fence, files));
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
            } else {
                // A file that holds the block as a run over lines the build compiles vouches for it; a file
                // that holds it only where the build compiles nothing says what the block really quotes.
                let mut quoted = false;
                let mut dead_in: Option<&Path> = None;
                for source in sources.iter().filter(|(fence, _)| *fence == language.fence).flat_map(|(_, files)| files) {
                    let runs = quoted_runs(&wanted, source);
                    if runs.is_empty() {
                        continue;
                    }
                    if runs.iter().all(|start| wholly_dead(&wanted, source, *start)) {
                        dead_in.get_or_insert(source.path.as_path());
                    } else {
                        quoted = true;
                    }
                }
                if quoted {
                    count += 1;
                } else if let Some(file) = dead_in {
                    problems.push(format!(
                        "{}: this {name} block quotes only lines that compile to nothing: all of them lie in a comment, a KDoc, a string, or a branch no shipping build compiles, in {}",
                        at(block.line),
                        file.display()
                    ));
                } else {
                    problems.push(format!(
                        "{}: this {name} block is no run of a file {} compile: its lines must appear in order in one such file, each indented against the others as it is there",
                        at(block.line),
                        language.builds
                    ));
                }
            }
        }
    }
    if problems.is_empty() { Ok(count) } else { Err(problems) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    /// The compiled sources the guides below are checked against: one Swift file and one Kotlin file,
    /// each with a line its build compiles to nothing — the comment, and the KDoc, that head them.
    fn sources() -> Sources {
        let source = |path: &str, text: &str, dead: &[usize]| Source {
            path: PathBuf::from(path),
            lines: text.lines().map(str::to_string).collect(),
            dead: dead.iter().copied().collect(),
        };
        vec![
            ("swift", vec![source("App/Studio.swift", "// The studio.\nstruct Studio {\n    func start() -> Int {\n        42\n    }\n}\n", &[1])]),
            ("kotlin", vec![source("src/main/kotlin/Studio.kt", "/** The studio. */\nfun start(): Int =\n    42\n", &[1])]),
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

    /// One compiled Kotlin file for the tests below: the two-line studio, with whatever lines its build
    /// compiles to nothing.
    fn studio(dead: &[usize]) -> Source {
        Source {
            path: PathBuf::from("sdk/core/src/main/kotlin/Studio.kt"),
            lines: vec!["fun start(): Int =".to_string(), "    42".to_string()],
            dead: dead.iter().copied().collect(),
        }
    }

    /// A block quoted wholly from lines a build compiles to nothing quotes prose dressed as code, and is
    /// refused naming the file the quotation really lies in. Which lines are dead is the language's own
    /// parser's answer — KDoc, comment, string body, dead branch alike — so the check asks none of the
    /// kinds itself; the gate's plants cover each kind against the real parsers.
    #[test]
    fn a_block_quoted_wholly_from_dead_lines_names_the_file_it_lies_in() {
        let sources: Sources = vec![("kotlin", vec![studio(&[1])])];
        assert_eq!(
            check_guides(&[(PathBuf::from("docs/guide.md"), "# Guide\n\n```kotlin\nfun start(): Int =\n```\n".to_string())], &sources),
            Err(vec![
                "docs/guide.md:3: this kotlin block quotes only lines that compile to nothing: all of them lie in a comment, a KDoc, a string, or a branch no shipping build compiles, in sdk/core/src/main/kotlin/Studio.kt"
                    .to_string()
            ])
        );
    }

    /// One live line in the run is enough: a quotation lies dead only when all of it does.
    #[test]
    fn a_block_with_one_live_line_is_no_dead_quotation() {
        let sources: Sources = vec![("kotlin", vec![studio(&[1])])];
        assert_eq!(check_guides(&[(PathBuf::from("docs/guide.md"), "# Guide\n\n```kotlin\nfun start(): Int =\n    42\n```\n".to_string())], &sources), Ok(1));
    }

    /// A KDoc quoted beside the code it documents stands: the comment's own line is dead, the code's are
    /// not, and the block as a whole quotes code.
    #[test]
    fn a_kdoc_quoted_with_the_code_it_documents_passes() {
        let source = Source {
            path: PathBuf::from("sdk/core/src/main/kotlin/Studio.kt"),
            lines: vec!["/** The studio. */".to_string(), "fun start(): Int =".to_string(), "    42".to_string()],
            dead: [1usize].into(),
        };
        let sources: Sources = vec![("kotlin", vec![source])];
        assert_eq!(
            check_guides(
                &[(PathBuf::from("docs/guide.md"), "# Guide\n\n```kotlin\n/** The studio. */\nfun start(): Int =\n    42\n```\n".to_string())],
                &sources
            ),
            Ok(1)
        );
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

    /// A tree that lists no samples answers no source — read as nothing, refusing the guide rather than
    /// passing it — and answering costs no build, because the providers start one only where samples are.
    #[test]
    fn a_tree_without_samples_answers_no_sources_and_is_refused() {
        let root = Scratch::new("guide-samples-no-samples");
        root.write("docs/guide.md", "# Guide\n");
        assert!(crate::samples::swift_guide_sources(&root.0).is_ok_and(|sources| sources.is_empty()));
        assert!(crate::samples::android_guide_sources(&root.0).is_ok_and(|sources| sources.is_empty()));
        assert_eq!(check(&root.0), Err(vec!["no swift sources found: the iOS samples' projects compile none to match swift blocks against".to_string()]));
    }

    /// The empty answer names the language and the builds that should have compiled something, for both
    /// languages alike; the Kotlin one is refused before any build starts, so only its wording can be the pin.
    #[test]
    fn an_empty_answer_names_the_language_and_the_builds_that_compiled_nothing() {
        assert_eq!(no_sources(&CODE_LANGUAGES[0]), "no swift sources found: the iOS samples' projects compile none to match swift blocks against");
        assert_eq!(no_sources(&CODE_LANGUAGES[1]), "no kotlin sources found: the Android samples' builds compile none to match kotlin blocks against");
    }

    /// The check reads what the samples compile, through `crate::samples`: a Swift file that only sits
    /// in a sample's folder, which its project's Sources phase does not list, is compiled by no build and
    /// is no source a block may quote, while the listed one passes.
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
        assert_eq!(
            crate::samples::project_build(&root.0.join("ios/samples/App/App.xcodeproj")).unwrap().sources,
            [root.0.join("ios/samples/App/App/Listed.swift")]
        );
        let listed = Source { path: root.0.join("ios/samples/App/App/Listed.swift"), lines: vec!["func listed() {}".to_string()], dead: Default::default() };
        let kotlin = Source { path: PathBuf::from("App.kt"), lines: vec!["fun app() {}".to_string()], dead: Default::default() };
        let sources: Sources = vec![("swift", vec![listed]), ("kotlin", vec![kotlin])];
        let guide = root.0.join("docs/guide.md");
        root.write("docs/guide.md", "# Guide\n\n```swift\nfunc listed() {}\n```\n\n```swift\nfunc unlisted() {}\n```\n");
        assert_eq!(
            check_guides(&[(guide.clone(), scan::read_text(&guide).unwrap())], &sources),
            Err(vec![format!(
                "{}:7: this swift block is no run of a file the iOS samples' projects compile: its lines must appear in order in one such file, each indented against the others as it is there",
                guide.display()
            )])
        );
    }
}
