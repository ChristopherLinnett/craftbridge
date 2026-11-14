//! The Swift stage's lint: SwiftLint's built-in rules over `ios/sdk/.swiftlint.yml`, and every Swift rule of
//! AGENTS.md §3, §4 and §10 that no built-in rule reads, asked of the compiler's own language server.
//!
//! SwiftLint's custom rules cannot carry those rules. A custom rule's `match_kinds` reads the syntax map
//! SwiftLint asks SourceKit for without the package's compiler arguments, so SourceKit lexes every file as
//! Swift 5, and the package is Swift 6, where a bare `/…/` is a regex literal: Swift 5 reads the `/*` of
//! `/\/*/` as the start of a comment and the `"""` of `/"""/` as the start of a string, and the lines after
//! either vanish from every custom rule while the compiler builds them; `/hi!/` reads as a force unwrap.
//! SwiftLint's built-in rules read swift-syntax, which lexes as the compiler does, so the one SwiftLint rule
//! the gate runs, `missing_docs`, is a built-in.
//!
//! The rest is read through SourceKit-LSP with the settings of every build of the package that ships — the
//! Debug and Release configurations, for each Apple slice the native library is built for (the device and the
//! simulator, as `rust/rust-toolchain.toml` lists them) — so every file reads as
//! every compiler that builds it reads it and every `#if` branch a shipping build compiles is read resolved,
//! not just read by name: a use of a pointer or a banned call that is live only under `#if !DEBUG` or only on
//! a device is resolved by the build that compiles it. What no build compiles — the far side of these `#if`s
//! — is read by name alone, as far as unresolved text can be. The semantic tokens say which text is code and
//! which is a comment, a string or a regex literal; the symbol information says which declaration a name
//! refers to; the document symbols say what a declaration is and how far it reaches. A verdict on one use is
//! the builds' answers merged: a build that reads it as the host's condemns it, a build that reads it as one
//! of ours clears it, and no answer is no clearance. On that reading:
//!
//! * outside test code, a use of a banned call (`fatalError`, `exit`, `.unsafelyUnwrapped`, …) fails unless
//!   every declaration it refers to is one of this repository's files: a module qualifier, backticks or the
//!   function taken as a value change nothing, and a name the server cannot resolve fails too, because a
//!   check that cannot read what a call means cannot pass it;
//! * outside test code, every postfix `!` fails: the server classifies every operator it reads, so a `!` it
//!   leaves unclassified is the punctuation of a force unwrap, an implicitly unwrapped optional, an `init!` or
//!   a key path's `!`; `try!` and `as!` are swift-format's (`NeverUseForceTry`, `NeverForceUnwrap`);
//! * outside a `Platform/` folder, a pointer API by name, a binding to a C symbol, and a type name whose
//!   alias chain ends at a pointer type, an alias of ours or of the SDK's;
//! * everywhere, a SwiftLint command, which switches SwiftLint's rules off, a swift-format ignore that
//!   switches every rule off, reaches beyond one function or property, or gives no reason, and the test
//!   frameworks swift-format exempts from its never-crash rules: XCTest anywhere, Swift Testing outside a
//!   test target.
//!
//! Test code is SwiftPM's answer — the files `swift package describe` puts in a test target — and a Platform
//! folder is a component of the path the repository names the file by: neither is a pattern over an absolute
//! path, which the folders a checkout happens to sit in could match.

use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

/// How long one language-server question may take before the check calls the server broken.
const ANSWER_WAIT: Duration = Duration::from_secs(120);

/// How long the server may take to load the package and build what its analysis needs, once, before any
/// question: a cold machine compiles the SDK's modules for the build's destination first, and that is
/// minutes, not seconds. A silence beyond this is none.
const SETTLE_WAIT: Duration = Duration::from_secs(900);

const PACKAGE: &str = "ios/sdk";
const MANIFEST: &str = "ios/sdk/Package.swift";
const CONFIGURATION: &str = "ios/sdk/.swiftlint.yml";

/// The calls AGENTS.md §3 bans outside tests, and what each does to the host app.
const BANNED_CALLS: &[(&str, &str)] = &[
    ("fatalError", "a crash instead of a thrown error"),
    ("precondition", "a crash instead of a thrown error"),
    ("preconditionFailure", "a crash instead of a thrown error"),
    ("assert", "a crash instead of a thrown error"),
    ("assertionFailure", "a crash instead of a thrown error"),
    ("exit", "terminates the host app, which owns the process"),
    ("abort", "ends the host app's process without unwinding, which the engine's panic guard never sees"),
    ("unsafelyUnwrapped", "trusts a value is not nil without checking, and an optimised build then reads undefined memory instead of failing"),
];

/// The attributes that bind a declaration to a C symbol: `@_silgen_name("abort")` is `abort` under a name of
/// ours, `@_cdecl` and `@c` hand the host a symbol of ours, `@_extern` names one of the host's. No name check
/// can follow a symbol, so like the pointer APIs they belong where the unsafe surface lives.
const C_SYMBOL_BINDINGS: &[&str] = &["@_silgen_name", "@_cdecl", "@c", "@_extern"];

/// What SwiftLint looks for in a comment to read a command (`// swiftlint:disable all`, `:next`, `:this`).
const SWIFTLINT_COMMAND: &str = "swiftlint:";

/// The LSP symbol kinds of a declaration that holds others: module, namespace (an extension), package,
/// class (an actor too), enum, interface (a protocol), object and struct.
const REGION_KINDS: &[u64] = &[2, 3, 4, 5, 10, 11, 19, 23];

/// The LSP symbol kind SourceKit-LSP gives a type alias.
const ALIAS_KIND: u64 = 26;

/// How many aliases deep a chain may go before the check stops trusting it.
const ALIAS_DEPTH: usize = 8;

/// The empty alias set a walk from a use carries: only the set-builder's own pass matches right-hand-side
/// names against names already known to reach a pointer; a walk that asks the server has the answers itself.
const NO_POINTER_ALIASES: BTreeSet<String> = BTreeSet::new();

/// The semantic-token kinds that are text, not code.
const NOT_CODE: &[&str] = &["comment", "string", "regexp", "number"];

/// One finding: the file as the repository names it, a line and a column from one, and what is wrong.
type Finding = (String, usize, usize, String);

/// What one build's server says about what a name refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Referral {
    /// Every declaration the server offers is one of this repository's files.
    Ours,
    /// A system declaration or one of anybody else's is among them.
    Theirs,
    /// The server could not place the name at all.
    Unknown,
}

/// The verdict of all builds' answers about one use of a banned name: a build that reads a host declaration
/// in it condemns the call — that is the configuration that ships and reaches for it; only builds that read
/// it as ours clear it, and no answer is no clearance.
fn merged_referral(referrals: &[Referral]) -> Referral {
    if referrals.contains(&Referral::Theirs) {
        Referral::Theirs
    } else if referrals.contains(&Referral::Ours) {
        Referral::Ours
    } else {
        Referral::Unknown
    }
}

/// What one build's server says about one occurrence of a type name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Use {
    /// Resolved, and its alias chain ends at a pointer API.
    Pointer,
    /// Resolved, and no pointer is in what it refers to.
    Clear,
    /// The build could not read it: a name in code the build does not compile, matching a pointer alias's.
    Unknown,
}

/// The verdict of all builds' answers about one use of a type name: a build that reads a pointer type in it
/// condemns it — that is the configuration that ships the pointer outside a Platform folder; a build that
/// reads what it refers to and finds no pointer clears it, for that use is in a configuration that ships
/// too; only where no build could read it does the name stand unresolved, as far as a check must fail closed.
fn merged_use(uses: &[Use]) -> Use {
    if uses.contains(&Use::Pointer) {
        Use::Pointer
    } else if uses.contains(&Use::Clear) {
        Use::Clear
    } else {
        Use::Unknown
    }
}

pub(crate) fn run(root: &Path) -> ExitCode {
    match check(root) {
        Ok(lines) => {
            for line in &lines {
                println!("{line}");
            }
            if lines.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) }
        }
        Err(failure) => {
            eprintln!("cargo xtask swift-lint: {failure}");
            ExitCode::from(1)
        }
    }
}

fn check(root: &Path) -> Result<Vec<String>, String> {
    let files = crate::files::listed(root, "swift")?;
    if files.is_empty() {
        return Err("no Swift files for the linter to read".to_string());
    }
    let canonical = std::fs::canonicalize(root).map_err(|error| format!("cannot resolve the repository's path: {error}"))?;
    let mut findings = run_swiftlint(root, &canonical, &files)?;
    let package = canonical.join(PACKAGE);
    let targets = crate::direction::describe_package(&package, MANIFEST)?.unwrap_or_default();
    let compiled: BTreeSet<String> = targets.iter().flat_map(|target| target.sources.iter()).map(|source| format!("{PACKAGE}/{source}")).collect();
    let tests: BTreeSet<String> = crate::direction::test_sources(&targets).into_iter().map(|source| format!("{PACKAGE}/{source}")).collect();
    let mut texts = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(canonical.join(file)).map_err(|error| format!("cannot read {file}: {error}"))?;
        findings.extend(swiftlint_commands(file, &text));
        texts.push(text);
    }
    // Where the server's answer can differ between builds: the same token sits at the same place in each
    // build's reading of a file, so every build's answer about a use is collected under that place and the
    // merge of all builds' answers, not any one build's, is what a use passes or fails on.
    let mut banned: BTreeMap<(String, usize, usize), (String, Vec<Referral>)> = BTreeMap::new();
    let mut pointers: BTreeMap<(String, usize, usize), (String, Vec<Use>)> = BTreeMap::new();
    for build in Build::all(&canonical, &package)? {
        let mut server = LanguageServer::start(&package, build)?;
        let mut read: Vec<(&str, String)> = Vec::new();
        for (file, text) in files.iter().zip(&texts) {
            let path = canonical.join(file);
            let uri = uri_of(&path);
            server.ours.insert(path);
            server.open(&uri, text)?;
            read.push((file.as_str(), uri));
        }
        server.settle()?;
        for (file, uri) in &read {
            if compiled.contains(*file) {
                server.require_build_settings(uri, file)?;
            }
        }
        let members = server.pointer_alias_names(&read);
        for (file, uri) in &read {
            let document = server.document(uri)?;
            findings.extend(ignore_directives(file, &document));
            findings.extend(test_framework_uses(file, &document, tests.contains(*file)));
            if !tests.contains(*file) {
                findings.extend(postfix_bangs(file, &document));
                for (line, column, name, referral) in server.banned_uses(uri, &document) {
                    banned.entry((file.to_string(), line, column)).or_insert_with(|| (name.clone(), Vec::new())).1.push(referral);
                }
            }
            if !in_platform(file) {
                findings.extend(pointer_names(file, &document));
                for (line, column, name, use_) in server.alias_uses(uri, &document, &members) {
                    pointers.entry((file.to_string(), line, column)).or_insert_with(|| (name.clone(), Vec::new())).1.push(use_);
                }
            }
        }
        server.shutdown();
    }
    for ((file, line, column), (name, referrals)) in &banned {
        if merged_referral(referrals) != Referral::Ours {
            let why = BANNED_CALLS.iter().find(|(banned, _)| *banned == name).map(|(_, why)| *why).unwrap_or_default();
            findings.push((file.clone(), *line, *column, format!("{name}: {why}")));
        }
    }
    for ((file, line, column), (name, uses)) in &pointers {
        if merged_use(uses) != Use::Clear {
            findings.push((
                file.clone(),
                *line,
                *column,
                format!("{name}: a pointer type reached through a type alias: pointer types appear only under a Platform/ folder"),
            ));
        }
    }
    findings.sort();
    findings.dedup();
    Ok(findings.into_iter().map(|(file, line, column, message)| format!("{file}:{line}:{column}: {message}")).collect())
}

/// Run SwiftLint itself and read its JSON. Anything the tool could only warn about — a configuration
/// section it refused to read, rules falling back to defaults — is fatal here: a lint that quietly stopped
/// being a lint would pass. `--no-cache`: SwiftLint's cache keys a file by its modification time, so a file
/// whose text changed under the same time would be served the last run's verdict.
fn run_swiftlint(root: &Path, canonical: &Path, files: &[String]) -> Result<Vec<Finding>, String> {
    let output = Command::new("swiftlint")
        .args(["lint", "--no-cache", "--quiet", "--config", CONFIGURATION, "--reporter", "json"])
        .args(files)
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot run swiftlint: {error} (tools/doctor.sh says where the pinned copy belongs)"))?;
    if !matches!(output.status.code(), Some(0) | Some(2)) {
        return Err(format!("swiftlint exited with {}:\n{}", output.status, String::from_utf8_lossy(&output.stderr).trim()));
    }
    let noise = String::from_utf8_lossy(&output.stderr).to_lowercase();
    for marker in ["invalid configuration", "invalid rule", "unknown rule"] {
        if noise.contains(marker) {
            return Err(format!(
                "swiftlint refused part of its configuration ({marker}); the gate would run a rule set its own file does not name:\n{}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    }
    let report: Value = serde_json::from_slice(&output.stdout).map_err(|error| format!("swiftlint's JSON is not readable here: {error}"))?;
    let entries = report.as_array().ok_or("swiftlint's JSON is not a list of violations")?;
    // A violation this check cannot read is refused rather than dropped: dropping it would pass it.
    entries.iter().map(|entry| violation(entry, canonical).ok_or_else(|| format!("swiftlint reported a violation this check cannot read: {entry}"))).collect()
}

/// One violation of SwiftLint's JSON reporter as a finding. The reporter prints canonical absolute paths;
/// the gate prints paths as the repository names them. A violation of a whole line has no column.
fn violation(entry: &Value, canonical: &Path) -> Option<Finding> {
    let reported = entry.get("file")?.as_str()?;
    let file = Path::new(reported).strip_prefix(canonical).map(|relative| relative.display().to_string()).unwrap_or_else(|_| reported.to_string());
    let message = format!("{}: {} ({})", entry.get("severity")?.as_str()?, entry.get("reason")?.as_str()?, entry.get("rule_id")?.as_str()?);
    Some((file, entry.get("line")?.as_u64()? as usize, entry.get("character").and_then(Value::as_u64).unwrap_or(1) as usize, message))
}

/// Every SwiftLint command in a file. SwiftLint reads one from any line comment holding `swiftlint:` and
/// switches its rules off for the line, the next or the rest of the file, so none may appear. Looked for in
/// the whole text, strings included, so no reader can disagree with SwiftLint about where a comment is.
fn swiftlint_commands(file: &str, text: &str) -> Vec<Finding> {
    lsp_lines(text)
        .into_iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let at = line.to_ascii_lowercase().find(SWIFTLINT_COMMAND)?;
            Some((
                file.to_string(),
                index + 1,
                column(line, at),
                "a SwiftLint command: SwiftLint switches its rules off where one says so, and nothing a file says switches the gate off (AGENTS.md §10)"
                    .to_string(),
            ))
        })
        .collect()
}

/// A swift-format ignore directive, read as swift-format 6.3 reads one (each spelling probed against
/// `swift format lint`): a `//` line comment — not a `///` documentation comment — holding only
/// `swift-format-ignore` or `swift-format-ignore-file`, alone or followed by `:` (spaces around it or not)
/// and a comma-separated list of rule names. Anything else after the word, `because …` included, makes it
/// no directive at all, and a list entry silences only the rule it names exactly.
#[derive(Debug, PartialEq)]
enum Directive {
    /// `swift-format-ignore-file`, with the rules it names, or none for every rule.
    File(Option<Vec<String>>),
    /// `swift-format-ignore`, applied to the declaration or statement that follows it.
    Node(Option<Vec<String>>),
}

impl Directive {
    fn read(comment: &str) -> Option<Self> {
        // A `///` documentation comment leaves a `/` before the word, so it is no directive, as swift-format reads it.
        let body = comment.strip_prefix("//")?.trim_start();
        let (file, rest) = match body.strip_prefix("swift-format-ignore-file") {
            Some(rest) => (true, rest),
            None => (false, body.strip_prefix("swift-format-ignore")?),
        };
        let rules =
            if rest.trim().is_empty() { None } else { Some(rest.trim_start().strip_prefix(':')?.split(',').map(|rule| rule.trim().to_string()).collect()) };
        Some(if file { Self::File(rules) } else { Self::Node(rules) })
    }
}

/// The swift-format ignores AGENTS.md §10 leaves no room for: a file-wide one that switches a never-crash rule
/// off (alone it switches every rule off), a bare declaration ignore, which does the same for what follows,
/// a declaration ignore with no reason on the comment line below it, and one before a declaration that holds
/// others — an extension, a type — or an `#if` block, which reaches every member it holds: only one function
/// or property may follow one.
fn ignore_directives(file: &str, document: &Document) -> Vec<Finding> {
    let mut found = Vec::new();
    for (index, token) in document.tokens.iter().enumerate() {
        if token.kind != "comment" {
            continue;
        }
        let at = |message: String| (file.to_string(), token.line + 1, document.column(token), message);
        match Directive::read(document.text(token)) {
            Some(Directive::File(None)) => {
                found.push(at("swift-format-ignore-file switches every rule off for the whole file, the never-crash ones included".to_string()))
            }
            Some(Directive::File(Some(rules))) => {
                for rule in rules.iter().filter(|rule| crate::sources::SWIFT_FORMAT_NEVER_CRASH_RULES.contains(&rule.as_str())) {
                    found.push(at(format!("swift-format-ignore-file switches {rule} off for the whole file")));
                }
            }
            Some(Directive::Node(rules)) => {
                if rules.is_none() {
                    found.push(at(
                        "a bare swift-format-ignore switches every rule off, the never-crash ones included: name the rule it switches off".to_string()
                    ));
                } else if !document.reasoned(index) {
                    found.push(at("a swift-format-ignore carries its reason on the comment line below it (AGENTS.md §10): swift-format silences nothing when the reason shares the ignore's line".to_string()));
                }
                if let Some(region) = document.region_after(index) {
                    found.push(at(format!(
                        "swift-format-ignore before {region} switches its rules off for everything inside it; only a single function or property may follow one"
                    )));
                }
            }
            None => {}
        }
    }
    found
}

/// The doors swift-format would let test code through: it exempts from its never-crash rules a whole file
/// that imports XCTest and every function marked `@Test`. So XCTest is refused everywhere (new tests use
/// Swift Testing), and Swift Testing outside the files SwiftPM compiles into a test target. An `import` in
/// any form (`@testable`, `public`, `import class XCTest.XCTestCase`, spread over lines) is the keyword and
/// the module the first name after its kind keyword names.
fn test_framework_uses(file: &str, document: &Document, test: bool) -> Vec<Finding> {
    let mut found = Vec::new();
    for (index, token) in document.tokens.iter().enumerate() {
        let written = document.text(token);
        let message = if token.kind == "keyword" && written == "import" {
            let module = document.tokens.iter().skip(index + 1).find(|next| next.kind != "keyword" && next.kind != "comment").map(|next| document.text(next));
            match module {
                Some("XCTest") => "import XCTest: swift-format exempts a file that imports XCTest from its never-crash rules, and new tests use Swift Testing",
                Some("Testing") if !test => "import Testing outside a test target: swift-format exempts test code from its never-crash rules",
                _ => continue,
            }
        } else if written == "@Test" && !test && !NOT_CODE.contains(&token.kind.as_str()) {
            "@Test outside a test target: swift-format exempts a @Test function from its never-crash rules"
        } else {
            continue;
        };
        found.push((file.to_string(), token.line + 1, document.column(token), message.to_string()));
    }
    found
}

/// Every postfix `!` outside test code: a `!` no token covers. SourceKit-LSP classifies every operator it
/// reads, a prefix `!`, `!=` and an operator of our own included, in code the build compiles and in code it
/// does not, so the `!` it leaves unclassified is the postfix punctuation the lexer makes of a `!` with no
/// space before it. `try!` and `as!` are swift-format's own rules and stay theirs alone.
fn postfix_bangs(file: &str, document: &Document) -> Vec<Finding> {
    let mut found = Vec::new();
    for (line, text) in document.lines.iter().enumerate() {
        let on_line: Vec<&Token> = document.tokens.iter().filter(|token| token.line == line).collect();
        for (at, _) in text.match_indices('!') {
            if on_line.iter().any(|token| token.start <= at && at < token.end) {
                continue;
            }
            let forced = on_line.iter().any(|token| token.end == at && token.kind == "keyword" && matches!(document.text(token), "try" | "as"));
            if !forced {
                found.push((
                    file.to_string(),
                    line + 1,
                    column(text, at),
                    "postfix !: a force unwrap or an implicitly unwrapped optional, a crash on nil".to_string(),
                ));
            }
        }
    }
    found
}

/// Every pointer API named outside a Platform folder, by its spelling, qualified or not, and every binding
/// of a declaration to a C symbol: `Darwin.OpaquePointer` and a member named `withUnsafe…` touch memory the
/// compiler is not tracking, and a method of ours may claim such a name only under Platform/ for the same
/// reason.
fn pointer_names(file: &str, document: &Document) -> Vec<Finding> {
    document
        .tokens
        .iter()
        .filter(|token| !NOT_CODE.contains(&token.kind.as_str()))
        .filter_map(|token| {
            let written = document.text(token);
            let message = if names_pointer(written.trim_matches('`')) {
                format!("{written}: a pointer API only under a Platform/ folder: the unsafe surface lives where a reviewer can find it")
            } else if C_SYMBOL_BINDINGS.contains(&written) {
                format!("{written}: a binding to a C symbol only under a Platform/ folder: it hands the host's symbols to code no name check can follow")
            } else {
                return None;
            };
            Some((file.to_string(), token.line + 1, document.column(token), message))
        })
        .collect()
}

/// Whether a name is one of the pointer APIs AGENTS.md §4 confines: every `Unsafe…Pointer`, `withUnsafe…`,
/// `Unmanaged`, `OpaquePointer`, `unsafeBitCast` and `unsafeDowncast`.
fn names_pointer(name: &str) -> bool {
    (name.contains("Unsafe") && name.ends_with("Pointer"))
        || name.contains("withUnsafe")
        || matches!(name, "Unmanaged" | "OpaquePointer" | "unsafeBitCast" | "unsafeDowncast")
}

/// Whether a file, as the repository names it, sits under a folder named `Platform`.
fn in_platform(file: &str) -> bool {
    let mut components: Vec<&str> = file.split('/').collect();
    components.pop();
    components.contains(&"Platform")
}

/// The lines of a file as the language server counts them: `\n`, `\r\n` and a lone `\r` each end a line,
/// as the LSP spec says and as Swift compiles — a file of lone carriage returns still compiles. Splitting
/// any other way would read as one line what the server reads as many: its tokens would answer about lines
/// the check does not have, no name would resolve, and a banned call there would pass for want of a reading.
fn lsp_lines(text: &str) -> Vec<&str> {
    text.split('\n').flat_map(|line| line.strip_suffix('\r').unwrap_or(line).split('\r')).collect()
}

/// A column from one, in UTF-16 code units, of a byte offset in a line: the unit LSP counts positions in
/// unless a client negotiates another, which is none here, so a message names the same column the server
/// reported and an editor moves its cursor through. A byte-order mark opening a file is no character an
/// editor shows, so it is not counted: the language server and this check both keep it in the first line,
/// which keeps their offsets in step, but a message names the column a reader sees.
fn column(line: &str, at: usize) -> usize {
    let units = |before: &str| before.chars().map(char::len_utf16).sum::<usize>();
    line.get(..at).map(|before| units(crate::scan::without_bom(before))).unwrap_or_default() + 1
}

/// One semantic token: where it sits, as the language server counts (a line from zero and a UTF-16 column)
/// and as the line's bytes do, and its kind from the server's legend.
#[derive(Clone, Debug)]
struct Token {
    line: usize,
    character: usize,
    start: usize,
    end: usize,
    kind: String,
}

impl Token {
    fn position(&self) -> (usize, usize) {
        (self.line, self.character)
    }
}

/// A declaration as the document symbols give it: its kind, the whole of it, and its name.
#[derive(Clone, Debug)]
struct Symbol {
    kind: u64,
    start: (usize, usize),
    end: (usize, usize),
    name_start: (usize, usize),
    name_end: (usize, usize),
}

/// One file as the language server reads it.
#[derive(Debug, Default)]
struct Document {
    lines: Vec<String>,
    tokens: Vec<Token>,
    symbols: Vec<Symbol>,
}

impl Document {
    fn new(text: &str, tokens: &[Value], legend: &[String], symbols: Option<&Value>) -> Self {
        let lines: Vec<String> = lsp_lines(text).into_iter().map(str::to_string).collect();
        let tokens = decode_tokens(tokens, legend, &lines);
        let mut flat = Vec::new();
        if let Some(Value::Array(items)) = symbols {
            flatten_symbols(items, &mut flat);
        }
        Self { lines, tokens, symbols: flat }
    }

    fn text(&self, token: &Token) -> &str {
        self.lines.get(token.line).and_then(|line| line.get(token.start..token.end)).unwrap_or_default()
    }

    fn column(&self, token: &Token) -> usize {
        self.lines.get(token.line).map(|line| column(line, token.start)).unwrap_or(1)
    }

    /// Whether the comment token at `index` has a comment of its own on the line directly below it.
    fn reasoned(&self, index: usize) -> bool {
        let (Some(ignore), Some(next)) = (self.tokens.get(index), self.tokens.get(index + 1)) else { return false };
        next.kind == "comment" && next.line == ignore.line + 1 && self.text(next).trim_start_matches('/').trim().chars().next().is_some()
    }

    /// What a declaration ignore at token `index` reaches, when that is more than one item: the first code after
    /// the comments that follow it opens an `#if` block, or starts a declaration — its attributes and
    /// modifiers come first — that holds others.
    fn region_after(&self, index: usize) -> Option<&'static str> {
        let next = self.tokens.iter().skip(index + 1).find(|token| token.kind != "comment")?;
        if self.text(next) == "#if" {
            return Some("an #if block");
        }
        let at = next.position();
        self.symbols
            .iter()
            .find(|symbol| REGION_KINDS.contains(&symbol.kind) && symbol.start <= at && at <= symbol.name_start)
            .map(|symbol| if symbol.kind == 3 { "an extension" } else { "a declaration that holds others (a type)" })
    }

    /// The tokens between two positions, the first included.
    fn tokens_between(&self, from: (usize, usize), to: (usize, usize)) -> impl Iterator<Item = &Token> {
        self.tokens.iter().filter(move |token| token.position() >= from && token.position() < to)
    }
}

/// The server's relative encoding (each token five numbers: lines down, then columns along from the last
/// token's start when on its line, the length, the kind's index in the legend, and modifiers), as tokens.
fn decode_tokens(data: &[Value], legend: &[String], lines: &[String]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let (mut line, mut character) = (0usize, 0usize);
    for chunk in data.chunks(5) {
        let numbers: Vec<usize> = chunk.iter().filter_map(Value::as_u64).map(|number| number as usize).collect();
        let [down, along, length, kind, _] = numbers.as_slice() else { break };
        if *down > 0 {
            line = line.saturating_add(*down);
            character = *along;
        } else {
            character = character.saturating_add(*along);
        }
        let text = lines.get(line).map(String::as_str).unwrap_or_default();
        tokens.push(Token {
            line,
            character,
            start: byte_of(text, character),
            end: byte_of(text, character.saturating_add(*length)),
            kind: legend.get(*kind).cloned().unwrap_or_default(),
        });
    }
    tokens
}

/// The byte offset of a UTF-16 column in a line, the line's end for one past it.
fn byte_of(line: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (index, character) in line.char_indices() {
        if units >= utf16 {
            return index;
        }
        units += character.len_utf16();
    }
    line.len()
}

fn flatten_symbols(items: &[Value], into: &mut Vec<Symbol>) {
    for item in items {
        let range = |key: &str, end: &str| -> Option<(usize, usize)> {
            let point = item.get(key)?.get(end)?;
            Some((point.get("line")?.as_u64()? as usize, point.get("character")?.as_u64()? as usize))
        };
        if let (Some(kind), Some(start), Some(end), Some(name_start), Some(name_end)) = (
            item.get("kind").and_then(Value::as_u64),
            range("range", "start"),
            range("range", "end"),
            range("selectionRange", "start"),
            range("selectionRange", "end"),
        ) {
            into.push(Symbol { kind, start, end, name_start, name_end });
        }
        if let Some(Value::Array(children)) = item.get("children") {
            flatten_symbols(children, into);
        }
    }
}

/// The Apple slices the native library is built for, as the Rust toolchain names them, and how the Swift
/// package is built for each: the target triple's architecture and environment, the SDK and how a message
/// names it. The Swift package is compiled only for the slices its binary target links, so the slices the
/// toolchain lists (`rust/rust-toolchain.toml`) are the destinations the analysis reads, whatever processor
/// the machine running the gate has. A slice of the toolchain's this table does not know stops the stage
/// rather than going unread.
const APPLE_SLICES: &[(&str, &str, &str, &str, &str)] = &[
    ("aarch64-apple-ios", "arm64", "", "iphoneos", "iOS device"),
    ("aarch64-apple-ios-sim", "arm64", "-simulator", "iphonesimulator", "iOS Simulator"),
    ("x86_64-apple-ios", "x86_64", "-simulator", "iphonesimulator", "Intel iOS Simulator"),
];

/// The toolchain's targets, read from `rust/rust-toolchain.toml`: the one list of what the native library is
/// built for.
const TOOLCHAIN: &str = "rust/rust-toolchain.toml";

/// The Apple targets the toolchain lists, each with its row of [`APPLE_SLICES`].
fn apple_slices(root: &Path) -> Result<Vec<&'static (&'static str, &'static str, &'static str, &'static str, &'static str)>, String> {
    let source = std::fs::read_to_string(root.join(TOOLCHAIN)).map_err(|error| format!("{TOOLCHAIN}: {error}"))?;
    let toolchain = toml::from_str::<toml::Value>(&source).map_err(|error| format!("{TOOLCHAIN}: is not valid TOML: {error}"))?;
    let targets: Vec<&str> = toolchain
        .get("toolchain")
        .and_then(|table| table.get("targets"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .filter(|target| target.contains("-apple-"))
        .collect();
    if targets.is_empty() {
        return Err(format!("{TOOLCHAIN}: lists no Apple target, so the Swift package ships for no destination the analysis could read"));
    }
    targets
        .into_iter()
        .map(|target| {
            APPLE_SLICES.iter().find(|slice| slice.0 == target).ok_or_else(|| {
                format!("{TOOLCHAIN}: the native library is built for `{target}`, a slice the Swift analysis has no build for (rust/xtask/src/swift_lint.rs)")
            })
        })
        .collect()
}

/// One build of the package that ships: the slice it builds for and the SwiftPM configuration it compiles
/// with, Debug and Release for each slice. An `#if` whose condition differs between these compiles different
/// code, and the gate reads every branch a shipping build compiles with the server resolving it; what none of
/// them compiles ships in no build and is read by name alone.
struct Build {
    /// How the stage and its errors name this build: its configuration and destination.
    label: String,
    /// SwiftPM's configuration: `debug` or `release`.
    configuration: &'static str,
    /// The SDK the destination builds against.
    sdk: String,
    /// The destination triple, as SwiftPM's `--triple` takes it.
    triple: String,
}

/// The SDK path `xcrun` gives for a platform SDK: the analysis reads files as a build against that SDK does.
fn xcrun_sdk(name: &str) -> Result<String, String> {
    let output =
        Command::new("xcrun").args(["--sdk", name, "--show-sdk-path"]).output().map_err(|error| format!("cannot ask xcrun for the {name} SDK: {error}"))?;
    let sdk = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || sdk.is_empty() {
        return Err(format!("xcrun cannot find the {name} SDK the analysis builds against (tools/doctor.sh names the fix)"));
    }
    Ok(sdk)
}

impl Build {
    /// The builds the shipping package compiles: both configurations for each slice the toolchain lists, at
    /// the package's own minimum iOS version.
    fn all(root: &Path, package: &Path) -> Result<Vec<Build>, String> {
        let manifest = crate::direction::dump_package(package, MANIFEST)?.ok_or_else(|| format!("{MANIFEST}: no package for the analysis to read"))?;
        let version = manifest
            .get("platforms")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|platform| platform.get("platformName").and_then(Value::as_str) == Some("ios"))
            .and_then(|platform| platform.get("version").and_then(Value::as_str))
            .ok_or_else(|| format!("{MANIFEST}: names no iOS version for the analysis to build against"))?
            .to_string();
        let mut builds = Vec::new();
        for (_, architecture, environment, sdk_name, destination) in apple_slices(root)? {
            let sdk = xcrun_sdk(sdk_name)?;
            for configuration in ["debug", "release"] {
                builds.push(Build {
                    label: format!("{configuration} {destination}"),
                    configuration,
                    triple: format!("{architecture}-apple-ios{version}{environment}"),
                    sdk: sdk.clone(),
                });
            }
        }
        Ok(builds)
    }

    /// SourceKit-LSP's options for this build: SwiftPM's build of the package for the destination and
    /// configuration, waited for rather than guessed at (a file the server has no build settings for yet
    /// would be read with settings for no platform at all), and the request that says which settings a file
    /// was read with. The `-sdk` flags come last so they are the last word whatever the environment (an
    /// `SDKROOT` that `xcrun` exports, say) makes SwiftPM name first.
    fn options(&self) -> Value {
        let swift = vec![
            Value::from("-sdk"),
            Value::from(self.sdk.as_str()),
            Value::from("-Xcc"),
            Value::from("-isysroot"),
            Value::from("-Xcc"),
            Value::from(self.sdk.as_str()),
        ];
        object([
            ("defaultWorkspaceType", Value::from("swiftPM")),
            ("backgroundIndexing", Value::from(true)),
            ("buildSettingsTimeout", Value::from(SETTLE_WAIT.as_millis() as u64)),
            ("experimentalFeatures", Value::from(vec!["sourcekit-options-request"])),
            (
                "swiftPM",
                object([
                    ("triple", Value::from(self.triple.as_str())),
                    ("configuration", Value::from(self.configuration)),
                    ("swiftCompilerFlags", Value::from(swift)),
                    ("cCompilerFlags", Value::from(vec![Value::from("-isysroot"), Value::from(self.sdk.as_str())])),
                ]),
            ),
        ])
    }
}

/// A language server over its JSON-RPC pipe. One server answers every question of one build's reading, with
/// each file opened once; messages are read on a side thread so a silent server fails the stage loudly, not
/// silently and not forever.
struct LanguageServer {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Value>,
    next_id: u64,
    legend: Vec<String>,
    build: Build,
    opened: BTreeSet<String>,
    documents: BTreeMap<String, Rc<Document>>,
    pointers: BTreeMap<(String, usize, usize), Option<bool>>,
    /// This repository's Swift files, as canonical paths: a declaration in one of them is ours.
    ours: BTreeSet<PathBuf>,
}

impl LanguageServer {
    fn start(package: &Path, build: Build) -> Result<Self, String> {
        let located =
            Command::new("xcrun").args(["--find", "sourcekit-lsp"]).output().map_err(|error| format!("cannot ask xcrun for sourcekit-lsp: {error}"))?;
        if !located.status.success() {
            return Err("xcrun cannot find sourcekit-lsp: the language server this stage reads Swift through comes with Xcode".to_string());
        }
        let binary = String::from_utf8_lossy(&located.stdout).trim().to_string();
        refuse_global_configuration()?;
        let mut child = Command::new(&binary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("cannot start {binary}: {error}"))?;
        let stdin = child.stdin.take().ok_or("sourcekit-lsp's stdin is not a pipe this check can write")?;
        let stdout = child.stdout.take().ok_or("sourcekit-lsp's stdout is not a pipe this check can read")?;
        let (sender, messages) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(message) = read_message(&mut reader) {
                if sender.send(message).is_err() {
                    return;
                }
            }
        });
        let mut server = Self {
            child,
            stdin,
            messages,
            next_id: 0,
            legend: Vec::new(),
            build,
            opened: BTreeSet::new(),
            documents: BTreeMap::new(),
            pointers: BTreeMap::new(),
            ours: BTreeSet::new(),
        };
        let capabilities = object([
            (
                "textDocument",
                object([
                    ("definition", object([("linkSupport", Value::from(false))])),
                    ("documentSymbol", object([("hierarchicalDocumentSymbolSupport", Value::from(true))])),
                    (
                        "semanticTokens",
                        object([
                            ("requests", object([("full", Value::from(true))])),
                            ("tokenTypes", Value::from(Vec::<Value>::new())),
                            ("tokenModifiers", Value::from(Vec::<Value>::new())),
                            ("formats", Value::from(vec!["relative"])),
                        ]),
                    ),
                ]),
            ),
            // The SDK's declarations then come as documents the server itself reads with the package's
            // settings, so an alias chain walks through them as through our own files.
            ("experimental", object([("workspace/getReferenceDocument", Value::from(true))])),
        ]);
        let initialized = server.request(
            "initialize",
            object([
                ("processId", Value::from(std::process::id())),
                ("clientInfo", object([("name", Value::from("craftbridge-gate"))])),
                ("rootUri", Value::from(uri_of(package))),
                ("initializationOptions", server.build.options()),
                ("capabilities", capabilities),
            ]),
            ANSWER_WAIT,
        )?;
        server.legend = initialized
            .pointer("/capabilities/semanticTokensProvider/legend/tokenTypes")
            .and_then(Value::as_array)
            .map(|kinds| kinds.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .ok_or("sourcekit-lsp offers no semantic tokens, which this check reads Swift through")?;
        server.notify("initialized", object([]))?;
        Ok(server)
    }

    fn open(&mut self, uri: &str, text: &str) -> Result<(), String> {
        if self.opened.insert(uri.to_string()) {
            self.notify(
                "textDocument/didOpen",
                object([(
                    "textDocument",
                    object([("uri", Value::from(uri)), ("languageId", Value::from("swift")), ("version", Value::from(1)), ("text", Value::from(text))]),
                )]),
            )?;
        }
        Ok(())
    }

    /// Wait until the server has loaded the package and prepared what its analysis needs, so no answer after
    /// this one is read from a half-built package.
    fn settle(&mut self) -> Result<(), String> {
        self.request("workspace/synchronize", object([("index", Value::from(true))]), SETTLE_WAIT).map(|_| ())
    }

    /// A file a target compiles must be read with SwiftPM's settings for this build, never with the fallback
    /// the server uses for a file it knows nothing of: a fallback reading resolves nothing outside the file
    /// and would leave every alias of ours unread.
    fn require_build_settings(&mut self, uri: &str, file: &str) -> Result<(), String> {
        let settings = self
            .request(
                "workspace/_sourceKitOptions",
                object([
                    ("textDocument", object([("uri", Value::from(uri))])),
                    ("prepareTarget", Value::from(false)),
                    ("allowFallbackSettings", Value::from(false)),
                ]),
                ANSWER_WAIT,
            )
            .map_err(|error| format!("{file}: the language server has no build settings for it: {error}"))?;
        let arguments: Vec<&str> = settings.get("compilerArguments").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        if settings.get("kind").and_then(Value::as_str) != Some("normal") || !reads_the_build(&arguments, &self.build) {
            return Err(format!(
                "{file}: the language server reads it without the {} build's settings, so its answers would not be the build's",
                self.build.label
            ));
        }
        Ok(())
    }

    /// One file as the server reads it, fetched once: a file of ours from the disk, a declaration of the SDK's
    /// from the server, which generates it.
    fn document(&mut self, uri: &str) -> Result<Rc<Document>, String> {
        if let Some(document) = self.documents.get(uri) {
            return Ok(Rc::clone(document));
        }
        let text = match path_of(uri) {
            Some(path) if uri.starts_with("file://") => std::fs::read_to_string(&path).map_err(|error| format!("cannot read {path}: {error}"))?,
            _ => self
                .request("workspace/getReferenceDocument", object([("uri", Value::from(uri))]), ANSWER_WAIT)?
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("the language server sent no text for {uri}"))?
                .to_string(),
        };
        self.open(uri, &text)?;
        let document = object([("uri", Value::from(uri))]);
        let tokens = self.request("textDocument/semanticTokens/full", object([("textDocument", document.clone())]), ANSWER_WAIT)?;
        let symbols = self.request("textDocument/documentSymbol", object([("textDocument", document)]), ANSWER_WAIT)?;
        let data = tokens.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
        let read = Rc::new(Document::new(&text, &data, &self.legend, Some(&symbols)));
        self.documents.insert(uri.to_string(), Rc::clone(&read));
        Ok(read)
    }

    /// Every use of a banned name and what this build's server makes of it. A raw identifier is one token,
    /// so `` `stop abort` `` is not `abort`; a backticked `` `abort` `` is.
    fn banned_uses(&mut self, uri: &str, document: &Document) -> Vec<(usize, usize, String, Referral)> {
        let mut found = Vec::new();
        for token in document.tokens.iter().filter(|token| !NOT_CODE.contains(&token.kind.as_str())) {
            let name = document.text(token).trim_matches('`');
            let Some((_, _)) = BANNED_CALLS.iter().find(|(banned, _)| *banned == name) else { continue };
            found.push((token.line + 1, document.column(token), name.to_string(), self.referral(uri, token.position())));
        }
        found
    }

    /// What this build's server says a name refers to. No answer, an error or no candidate is no answer; a
    /// system declaration or one in a file that is not ours is theirs; every candidate in one of ours is ours.
    fn referral(&mut self, uri: &str, at: (usize, usize)) -> Referral {
        let Ok(answer) = self.request("textDocument/symbolInfo", position_params(uri, at), ANSWER_WAIT) else { return Referral::Unknown };
        let details = answer.as_array().cloned().unwrap_or_default();
        if details.is_empty() {
            return Referral::Unknown;
        }
        let all_ours = details.iter().all(|detail| {
            detail.get("isSystem").and_then(Value::as_bool) != Some(true)
                && detail
                    .pointer("/bestLocalDeclaration/uri")
                    .and_then(Value::as_str)
                    .and_then(path_of)
                    .and_then(|path| std::fs::canonicalize(path).ok())
                    .is_some_and(|path| self.ours.contains(&path))
        });
        if all_ours { Referral::Ours } else { Referral::Theirs }
    }

    /// The names of this repository's aliases of a pointer type, read from each declaration's own right-hand
    /// side: in code this build does not compile the server classifies names without resolving them, and one
    /// of these names there is the alias all the same. A declaration this build does not compile is read for
    /// what its right-hand side spells, never taken for a pointer for want of an answer about its own name.
    fn pointer_alias_names(&mut self, read: &[(&str, String)]) -> BTreeSet<String> {
        // Grown to a fixed point, each pass seeing the names the ones before found: an alias in code no
        // build compiles is read by what its right-hand side spells, and one that spells another of our
        // alias names reaches a pointer when that one does — whichever order the two declarations sit in,
        // and however long the chain of unresolved names between them.
        let mut names = BTreeSet::new();
        loop {
            let mut grew = false;
            for (_, uri) in read {
                let Ok(document) = self.document(uri) else { continue };
                for symbol in document.symbols.iter().filter(|symbol| symbol.kind == ALIAS_KIND) {
                    if self.right_hand_side_reaches_pointer(uri, &document, symbol, 0, &names) {
                        let line = document.lines.get(symbol.name_start.0).map(String::as_str).unwrap_or_default();
                        grew |= names.insert(line.get(byte_of(line, symbol.name_start.1)..byte_of(line, symbol.name_end.1)).unwrap_or_default().to_string());
                    }
                }
            }
            if !grew {
                return names;
            }
        }
    }

    /// Every use of a type name this build reads as reaching a pointer type, or cannot rule out: a resolved
    /// one whose alias chain ends at a pointer API; a name this build cannot read at all, in code it does not
    /// compile, that names one of this build's pointer aliases; and a type name the server cannot place,
    /// which no build gets to call innocent.
    fn alias_uses(&mut self, uri: &str, document: &Document, members: &BTreeSet<String>) -> Vec<(usize, usize, String, Use)> {
        let mut found = Vec::new();
        for token in &document.tokens {
            let name = document.text(token);
            let use_ = match token.kind.as_str() {
                "typeParameter" => match self.alias_verdict(uri, token.position(), 0) {
                    Some(true) => Use::Pointer,
                    Some(false) => Use::Clear,
                    None => Use::Unknown,
                },
                "identifier" if members.contains(name) && !document.symbols.iter().any(|symbol| symbol.name_start == token.position()) => Use::Unknown,
                _ => continue,
            };
            found.push((token.line + 1, document.column(token), name.to_string(), use_));
        }
        found
    }

    /// Whether the type named at a position is a pointer type through aliases: the declaration it refers to is
    /// an alias whose right-hand side names a pointer API, or names an alias that is one. A resolved type name
    /// the server cannot place or read the declaration of, and a chain deeper than [`ALIAS_DEPTH`], are not
    /// proven innocent.
    fn is_pointer_type(&mut self, uri: &str, at: (usize, usize), depth: usize) -> bool {
        self.alias_verdict(uri, at, depth).unwrap_or(true)
    }

    /// The memoised walk: `Some` when the chain was read to its end, `None` when the server could not say what
    /// the name at `at` refers to.
    fn alias_verdict(&mut self, uri: &str, at: (usize, usize), depth: usize) -> Option<bool> {
        if depth >= ALIAS_DEPTH {
            return Some(true);
        }
        let key = (uri.to_string(), at.0, at.1);
        if let Some(known) = self.pointers.get(&key) {
            return *known;
        }
        let verdict = self.walk_alias(uri, at, depth);
        self.pointers.insert(key, verdict);
        verdict
    }

    fn walk_alias(&mut self, uri: &str, at: (usize, usize), depth: usize) -> Option<bool> {
        let targets = locations(&self.request("textDocument/definition", position_params(uri, at), ANSWER_WAIT).ok()?);
        if targets.is_empty() {
            return None;
        }
        for (target, position) in targets {
            let declared = self.document(&target).ok()?;
            let aliases: Vec<Symbol> = declared
                .symbols
                .iter()
                .filter(|symbol| symbol.kind == ALIAS_KIND && symbol.name_start <= position && position <= symbol.name_end)
                .cloned()
                .collect();
            for alias in aliases {
                if self.right_hand_side_reaches_pointer(&target, &declared, &alias, depth, &NO_POINTER_ALIASES) {
                    return Some(true);
                }
            }
        }
        Some(false)
    }

    /// Whether an alias declaration's right-hand side names a pointer API, names one of the alias names
    /// already known to reach one (`known`, which the set-builder grows to a fixed point), or names a type
    /// the walk follows to one.
    fn right_hand_side_reaches_pointer(&mut self, uri: &str, declared: &Document, alias: &Symbol, depth: usize, known: &BTreeSet<String>) -> bool {
        let right: Vec<Token> = declared.tokens_between(alias.name_end, alias.end).cloned().collect();
        for token in right {
            let written = declared.text(&token).trim_matches('`');
            if names_pointer(written) || known.contains(written) {
                return true;
            }
            // The server resolves the names in our own files, so an alias there is a `typeParameter` whose
            // chain must be read to its end. In a declaration the SDK's interface holds, or in code this build
            // does not compile, it classifies names without resolving them, so an alias there is an
            // `identifier`, which a function type's argument label is too: one the server cannot place is no
            // type to follow — only its spelled name, matched against `known`, carries a chain of them.
            let followed = match token.kind.as_str() {
                "typeParameter" => self.is_pointer_type(uri, token.position(), depth + 1),
                "identifier" => self.alias_verdict(uri, token.position(), depth + 1).unwrap_or(false),
                _ => false,
            };
            if followed {
                return true;
            }
        }
        false
    }

    /// Ask the server to stop, as the protocol does; [`Drop`] ends it whatever it answers.
    fn shutdown(mut self) {
        let _ = self.request("shutdown", Value::Null, ANSWER_WAIT);
        let _ = self.notify("exit", object([]));
    }

    /// Ask, and wait for this request's answer. A request the server sends meanwhile (a progress token, a
    /// refresh) is answered at once, so the server never waits on the check while the check waits on it.
    fn request(&mut self, method: &str, params: Value, wait: Duration) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&object([("jsonrpc", Value::from("2.0")), ("id", Value::from(id)), ("method", Value::from(method)), ("params", params)]))?;
        loop {
            let message = self.messages.recv_timeout(wait).map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => format!(
                    "the language server gave no answer to the `{method}` request within {} seconds: a check cannot read a verdict from a silent analyser",
                    wait.as_secs()
                ),
                mpsc::RecvTimeoutError::Disconnected => {
                    format!("the language server closed its output before it answered the `{method}` request: a check cannot read a verdict from an analyser that is gone")
                }
            })?;
            if message.get("method").is_some() {
                if let Some(asked) = message.get("id") {
                    self.send(&object([("jsonrpc", Value::from("2.0")), ("id", asked.clone()), ("result", Value::Null)]))?;
                }
                continue;
            }
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(format!(
                    "the language server refused the `{method}` request: {}",
                    error.get("message").and_then(Value::as_str).unwrap_or("no message")
                ));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&object([("jsonrpc", Value::from("2.0")), ("method", Value::from(method)), ("params", params)]))
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        let body = message.to_string();
        self.stdin
            .write_all(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes())
            .map_err(|error| format!("cannot write to the language server: {error}"))?;
        self.stdin.flush().map_err(|error| format!("cannot flush the language server's stdin: {error}"))
    }
}

/// A run that stops on an error still ends its server: a language server left behind keeps the package's
/// index build locked, and the next run would wait on it.
impl Drop for LanguageServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// SourceKit-LSP merges a configuration file of the machine's over the options the check starts it with, so
/// one could hand the analysis another platform or other compiler flags. The check runs only where there is
/// none; a workspace's own `.sourcekit-lsp` folder is the tree's, which `rules: machinery` refuses.
fn refuse_global_configuration() -> Result<(), String> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut candidates = vec![
        home.join(".sourcekit-lsp/config.json"),
        home.join("Library/Application Support/org.swift.sourcekit-lsp/config.json"),
        PathBuf::from("/Library/Application Support/org.swift.sourcekit-lsp/config.json"),
    ];
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        candidates.push(PathBuf::from(xdg).join("sourcekit-lsp/config.json"));
    }
    match candidates.iter().find(|path| path.is_file()) {
        Some(found) => Err(format!(
            "{}: SourceKit-LSP merges this configuration over the gate's own options, so the Swift checks would not read the build the gate makes; move it away",
            found.display()
        )),
        None => Ok(()),
    }
}

/// Whether compiler arguments build the given build: the last word of each setting is the one the compiler
/// keeps, so the target, the SDK and the sysroot the C importer reads must each end on this destination's,
/// and the arguments must carry this configuration's marker — SwiftPM defines `DEBUG` for its debug builds
/// and nothing like it for its release ones, which is what the server was probed to answer for both. An
/// `SDKROOT` in the environment (which `xcrun` exports to what it starts) makes SwiftPM name the macOS SDK
/// first; this build's must come after it.
fn reads_the_build(arguments: &[&str], build: &Build) -> bool {
    let last = |flag: &str| arguments.windows(2).filter(|pair| pair.first() == Some(&flag)).filter_map(|pair| pair.get(1).copied()).next_back();
    let sysroot =
        arguments.windows(4).filter(|run| run.get(..3) == Some(["-Xcc", "-isysroot", "-Xcc"].as_slice())).filter_map(|run| run.get(3).copied()).next_back();
    last("-target") == Some(build.triple.as_str())
        && last("-sdk") == Some(build.sdk.as_str())
        && sysroot.is_none_or(|root| root == build.sdk)
        && defines(arguments, "DEBUG") == (build.configuration == "debug")
}

/// Whether compiler arguments define a flag: SwiftPM writes its definitions joined (`-DDEBUG`); the split
/// two-argument form is taken too, so a different spelling cannot let a debug reading pass as a release one.
fn defines(arguments: &[&str], flag: &str) -> bool {
    let joined = format!("-D{flag}");
    arguments.iter().any(|argument| *argument == joined) || arguments.windows(2).any(|pair| pair.first() == Some(&"-D") && pair.get(1) == Some(&flag))
}

/// One JSON-RPC message from the server's stdout, or `None` once the pipe closes or stops making sense.
fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    loop {
        let mut length = None;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).ok()? == 0 {
                return None;
            }
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some(value) = header.strip_prefix("Content-Length:") {
                length = value.trim().parse::<usize>().ok();
            }
        }
        let mut body = vec![0u8; length?];
        reader.read_exact(&mut body).ok()?;
        if let Ok(message) = serde_json::from_slice::<Value>(&body) {
            return Some(message);
        }
    }
}

/// The locations a definition answer names, whichever of the shapes the protocol allows it sent.
fn locations(answer: &Value) -> Vec<(String, (usize, usize))> {
    let items = match answer {
        Value::Array(items) => items.clone(),
        Value::Null => Vec::new(),
        single => vec![single.clone()],
    };
    items
        .iter()
        .filter_map(|location| {
            let uri = location.get("uri").or_else(|| location.get("targetUri")).and_then(Value::as_str)?.to_string();
            let start = location.get("range").or_else(|| location.get("targetSelectionRange"))?.get("start")?;
            Some((uri, (start.get("line")?.as_u64()? as usize, start.get("character")?.as_u64()? as usize)))
        })
        .collect()
}

fn position_params(uri: &str, (line, character): (usize, usize)) -> Value {
    object([
        ("textDocument", object([("uri", Value::from(uri))])),
        ("position", object([("line", Value::from(line as u64)), ("character", Value::from(character as u64))])),
    ])
}

fn uri_of(path: &Path) -> String {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut uri = String::from("file://");
    for byte in absolute.to_string_lossy().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'.' | b'-' | b'_' => uri.push(*byte as char),
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// The path a URI names, percent-decoded, whatever its scheme: a caller asks the scheme itself when it matters.
fn path_of(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::new();
    let rest_bytes = rest.as_bytes();
    let mut index = 0;
    while index < rest_bytes.len() {
        if rest_bytes.get(index) == Some(&b'%') && index + 2 < rest_bytes.len() {
            let hex = std::str::from_utf8(rest_bytes.get(index + 1..index + 3)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            bytes.push(*rest_bytes.get(index)?);
            index += 1;
        }
    }
    String::from_utf8(bytes).ok()
}

/// A JSON object from ordered key-value pairs; `json!` is not usable here because its macro expands
/// through `unwrap`, which the workspace forbids outside tests.
fn object<const N: usize>(items: [(&str, Value); N]) -> Value {
    let mut fields = Map::new();
    for (key, value) in items {
        fields.insert(key.to_string(), value);
    }
    Value::Object(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LEGEND: &[&str] = &["keyword", "identifier", "comment", "operator", "string", "regexp", "macro", "typeParameter", "struct"];

    /// A document from text and tokens written as (line, UTF-16 column, length, kind) from zero.
    fn document(text: &str, tokens: &[(usize, usize, usize, &str)], symbols: Value) -> Document {
        let legend: Vec<String> = LEGEND.iter().map(|kind| kind.to_string()).collect();
        let (mut line, mut character, mut data) = (0usize, 0usize, Vec::new());
        for (at_line, at, length, kind) in tokens {
            let along = if *at_line == line { at - character } else { *at };
            data.extend([at_line - line, along, *length, LEGEND.iter().position(|known| known == kind).unwrap()].map(|n| json!(n)));
            data.push(json!(0));
            (line, character) = (*at_line, *at);
        }
        Document::new(text, &data, &legend, Some(&symbols))
    }

    #[test]
    fn tokens_decode_from_the_relative_encoding_into_bytes() {
        // `é` is two UTF-8 bytes and one UTF-16 unit: the second token's byte offsets move, its column does not.
        let read =
            document("let é = x\nlet y = 1\n", &[(0, 0, 3, "keyword"), (0, 4, 1, "identifier"), (0, 8, 1, "identifier"), (1, 4, 1, "identifier")], json!([]));
        let shown: Vec<(&str, usize, usize, &str)> = read.tokens.iter().map(|t| (read.text(t), t.line, t.character, t.kind.as_str())).collect();
        assert_eq!(shown, [("let", 0, 0, "keyword"), ("é", 0, 4, "identifier"), ("x", 0, 8, "identifier"), ("y", 1, 4, "identifier")]);
        assert_eq!(read.column(&read.tokens[2]), 9);
    }

    #[test]
    fn a_column_is_the_one_an_editor_shows() {
        assert_eq!(column("let x = y!", 9), 10);
        assert_eq!(column("\u{FEFF}let x = y!", 12), 10, "a byte-order mark is not a character on screen");
        assert_eq!(column("let é = y!", 10), 10, "characters, not bytes");
        assert_eq!(column("let \u{1F600} = y!", 12), 11, "a character outside the basic plane is two UTF-16 units, the unit the language server counts");
    }

    #[test]
    fn lines_end_where_the_language_server_sees_them() {
        assert_eq!(lsp_lines("a\nb\n"), ["a", "b", ""], "line feeds, as split on them reads them");
        assert_eq!(lsp_lines("a\r\nb\r\n"), ["a", "b", ""], "a carriage return before a line feed ends one line, not two");
        assert_eq!(lsp_lines("a\rb\rc"), ["a", "b", "c"], "a lone carriage return is a line ending too");
        assert_eq!(lsp_lines("a\r\r\nb"), ["a", "", "b"], "one ending directly after another");
        assert_eq!(lsp_lines("a\n\r"), ["a", ""], "a file whose last line ends on a lone carriage return");
    }

    #[test]
    fn directives_read_as_swift_format_reads_them() {
        let file = |rules: &[&str]| Some(Directive::File(Some(rules.iter().map(|r| r.to_string()).collect())));
        let node = |rules: &[&str]| Some(Directive::Node(Some(rules.iter().map(|r| r.to_string()).collect())));
        // Each verdict was checked against `swift format lint` 6.3: what silences a rule, and what does not.
        for (comment, read) in [
            ("// swift-format-ignore-file", Some(Directive::File(None))),
            ("//swift-format-ignore-file:NeverForceUnwrap", file(&["NeverForceUnwrap"])),
            ("// swift-format-ignore-file : NeverForceUnwrap", file(&["NeverForceUnwrap"])),
            ("// swift-format-ignore", Some(Directive::Node(None))),
            ("// swift-format-ignore: NeverForceUnwrap, NeverUseForceTry", node(&["NeverForceUnwrap", "NeverUseForceTry"])),
            ("// swift-format-ignore : NeverForceUnwrap", node(&["NeverForceUnwrap"])),
            ("// swift-format-ignore: NeverForceUnwrap because x", node(&["NeverForceUnwrap because x"])),
            ("// swift-format-ignore NeverForceUnwrap", None),
            ("// swift-format-ignore-file NeverForceUnwrap", None),
            ("// swift-format-ignore because reasons", None),
            ("/// swift-format-ignore: NeverForceUnwrap", None),
            ("// note swift-format-ignore: NeverForceUnwrap", None),
            ("// swift-format-ignored", None),
        ] {
            assert_eq!(Directive::read(comment), read, "{comment}");
        }
    }

    /// The symbols of a `struct` whose head starts at an attribute, a function inside it, and an extension.
    fn symbols() -> Value {
        let symbol = |kind: u64, start: (u64, u64), end: (u64, u64), name: (u64, u64, u64)| {
            json!({"kind": kind, "range": {"start": {"line": start.0, "character": start.1}, "end": {"line": end.0, "character": end.1}},
                   "selectionRange": {"start": {"line": name.0, "character": name.1}, "end": {"line": name.0, "character": name.2}}})
        };
        json!([
            {"kind": 23, "range": {"start": {"line": 1, "character": 0}, "end": {"line": 6, "character": 1}},
             "selectionRange": {"start": {"line": 2, "character": 7}, "end": {"line": 2, "character": 8}},
             "children": [symbol(12, (5, 4), (5, 15), (5, 9, 12))]},
            symbol(3, (8, 0), (8, 14), (8, 10, 11)),
        ])
    }

    const IGNORES: &str = "// swift-format-ignore: NeverUseForceTry\n@available(iOS 17, *)\nstruct S {\n    // swift-format-ignore: NeverUseForceTry\n    // The fixture is compiled in.\n    func f() {}\n}\n// swift-format-ignore\nextension S {}\n// swift-format-ignore-file: OrderedImports, NeverForceUnwrap\n// swift-format-ignore: NeverForceUnwrap\n#if DEBUG\n";

    fn ignores() -> Document {
        document(
            IGNORES,
            &[
                (0, 0, 40, "comment"),
                (1, 0, 10, "keyword"),
                (2, 0, 6, "keyword"),
                (3, 4, 40, "comment"),
                (4, 4, 31, "comment"),
                (5, 4, 4, "keyword"),
                (7, 0, 22, "comment"),
                (8, 0, 9, "keyword"),
                (9, 0, 61, "comment"),
                (10, 0, 40, "comment"),
                (11, 0, 3, "macro"),
            ],
            symbols(),
        )
    }

    #[test]
    fn an_ignore_reaching_a_region_or_without_its_reason_is_refused_and_one_on_a_function_with_one_is_not() {
        let found = ignore_directives("A.swift", &ignores());
        let lines: Vec<(usize, &str)> = found.iter().map(|(_, line, _, message)| (*line, message.as_str())).collect();
        // Line 1: before a struct whose head is its attribute, and with no reason below it (the next comment
        // line is not right under it).
        assert!(lines.iter().any(|(line, m)| *line == 1 && m.contains("a declaration that holds others")), "{lines:?}");
        assert!(lines.iter().any(|(line, m)| *line == 1 && m.contains("its reason on the comment line below")), "{lines:?}");
        // Line 4: a function inside, with its reason: allowed.
        assert!(!lines.iter().any(|(line, _)| *line == 4), "{lines:?}");
        // Line 8: bare, before an extension.
        assert!(lines.iter().any(|(line, m)| *line == 8 && m.contains("a bare swift-format-ignore")), "{lines:?}");
        assert!(lines.iter().any(|(line, m)| *line == 8 && m.contains("an extension")), "{lines:?}");
        // Line 10: a file-wide ignore naming a never-crash rule among others; line 11: before #if.
        assert!(lines.iter().any(|(line, m)| *line == 10 && m.contains("switches NeverForceUnwrap off for the whole file")), "{lines:?}");
        assert!(lines.iter().any(|(line, m)| *line == 11 && m.contains("an #if block")), "{lines:?}");
        assert!(lines.iter().any(|(line, m)| *line == 11 && m.contains("its reason on the comment line below")), "{lines:?}");
        assert_eq!(found.len(), 7, "{lines:?}");
    }

    #[test]
    fn a_bang_no_token_covers_is_postfix_and_try_as_and_operators_are_not() {
        let text =
            "let a = x! + f()!.y\nif !ok && a != b {}\nlet b = try! g()\nlet c = d as! Int\nlet k = \\H.v!\ninit!(t: Int) {}\nlet s = \"!\"; let r = /hi!/\n";
        let read = document(
            text,
            &[
                (0, 0, 3, "keyword"),
                (0, 8, 1, "identifier"),
                (0, 11, 1, "operator"),
                (0, 13, 1, "identifier"),
                (1, 0, 2, "keyword"),
                (1, 3, 1, "operator"),
                (1, 4, 2, "identifier"),
                (1, 7, 2, "operator"),
                (1, 12, 2, "operator"),
                (2, 8, 3, "keyword"),
                (3, 10, 2, "keyword"),
                (4, 10, 1, "identifier"),
                (5, 0, 4, "keyword"),
                (6, 8, 3, "string"),
                (6, 21, 5, "regexp"),
            ],
            json!([]),
        );
        let found: Vec<(usize, usize)> = postfix_bangs("A.swift", &read).into_iter().map(|(_, line, column, _)| (line, column)).collect();
        assert_eq!(found, [(1, 10), (1, 17), (5, 13), (6, 5)], "x!, f()!, a key path's ! and init!; not !ok, !=, try!, as!, a string or a regex");
    }

    #[test]
    fn pointer_names_and_c_bindings_are_found_by_their_spelling_in_code_only() {
        let text = "let p: UnsafeMutableRawPointer = q.withUnsafeBytes { $0 }\n@_silgen_name(\"abort\") func stop()\n// OpaquePointer in prose\n";
        let read =
            document(text, &[(0, 7, 23, "struct"), (0, 35, 15, "identifier"), (1, 0, 13, "keyword"), (1, 14, 7, "string"), (2, 0, 26, "comment")], json!([]));
        let found: Vec<(usize, usize)> = pointer_names("A.swift", &read).into_iter().map(|(_, line, column, _)| (line, column)).collect();
        assert_eq!(found, [(1, 8), (1, 36), (2, 1)]);
        for name in ["UnsafeMutablePointer", "AutoreleasingUnsafeMutablePointer", "withUnsafeBytes", "Unmanaged", "OpaquePointer", "unsafeBitCast"] {
            assert!(names_pointer(name), "{name}");
        }
        for name in ["Session", "UnsafeThing", "Int", "withData", "Pointer", "unsafeFlags"] {
            assert!(!names_pointer(name), "{name}");
        }
    }

    #[test]
    fn xctest_is_refused_everywhere_and_swift_testing_outside_a_test_target() {
        let text = "@testable import XCTest\nimport class XCTest.XCTestCase\nimport Testing\n@Test func f() {}\n// import XCTest\nimport Foundation\n";
        let read = document(
            text,
            &[
                (0, 0, 9, "keyword"),
                (0, 10, 6, "keyword"),
                (0, 17, 6, "identifier"),
                (1, 0, 6, "keyword"),
                (1, 7, 5, "keyword"),
                (1, 13, 6, "identifier"),
                (1, 20, 11, "identifier"),
                (2, 0, 6, "keyword"),
                (2, 7, 7, "identifier"),
                (3, 0, 5, "keyword"),
                (3, 6, 4, "keyword"),
                (4, 0, 16, "comment"),
                (5, 0, 6, "keyword"),
                (5, 7, 10, "identifier"),
            ],
            json!([]),
        );
        let lines = |test| test_framework_uses("A.swift", &read, test).into_iter().map(|(_, line, _, _)| line).collect::<Vec<_>>();
        assert_eq!(lines(false), [1, 2, 3, 4], "XCTest twice, Testing and @Test, but not the comment or Foundation");
        assert_eq!(lines(true), [1, 2], "a test target keeps Swift Testing and still may not import XCTest");
    }

    #[test]
    fn a_swiftlint_command_is_found_in_any_line() {
        let found = swiftlint_commands(
            "A.swift",
            "let a = 1\nabort() // swiftlint:disable:this all\n  // SwiftLint:disable all\n// swiftlint mentioned\n// swiftlint:enable all\n",
        );
        assert_eq!(found.iter().map(|(_, line, column, _)| (*line, *column)).collect::<Vec<_>>(), [(2, 12), (3, 6), (5, 4)]);
    }

    #[test]
    fn a_platform_folder_is_a_component_of_the_repository_path() {
        assert!(in_platform("ios/sdk/Sources/CraftBridge/Platform/Bytes.swift"));
        assert!(!in_platform("ios/sdk/Sources/CraftBridge/Bytes.swift"));
        assert!(!in_platform("ios/sdk/Sources/PlatformKit/Bytes.swift"));
        assert!(!in_platform("ios/sdk/Sources/CraftBridge/platform/Bytes.swift"));
    }

    #[test]
    fn a_build_is_the_last_word_of_every_setting_and_its_configuration_marker() {
        let (sim, mac, triple) = ("/S/iPhoneSimulator.sdk", "/S/MacOSX.sdk", "arm64-apple-ios17.0-simulator");
        let debug = Build { label: "debug iOS Simulator".into(), configuration: "debug", sdk: sim.into(), triple: triple.into() };
        let release = Build { label: "release iOS Simulator".into(), configuration: "release", sdk: sim.into(), triple: triple.into() };
        let mut good = vec!["-target", triple, "-sdk", mac, "-Xcc", "-isysroot", "-Xcc", mac, "-sdk", sim, "-Xcc", "-isysroot", "-Xcc", sim, "-DDEBUG"];
        assert!(reads_the_build(&good, &debug));
        assert!(!reads_the_build(&good, &release), "a debug reading is not a release one");
        good.retain(|argument| *argument != "-DDEBUG");
        assert!(reads_the_build(&good, &release));
        assert!(!reads_the_build(&good, &debug), "a release reading is not a debug one");
        assert!(!reads_the_build(&good[..10], &release), "the macOS sysroot is the last");
        assert!(!reads_the_build(&["-target", triple, "-sdk", sim, "-sdk", mac], &release), "the macOS SDK is the last");
        assert!(!reads_the_build(&["-target", "arm64-apple-macosx14.0", "-sdk", sim, "-DDEBUG"], &debug), "another platform");
        assert!(defines(&["-D", "DEBUG"], "DEBUG"), "the split form defines it too");
    }

    #[test]
    fn one_build_reading_the_host_condemns_a_use_and_none_is_no_clearance() {
        assert_eq!(merged_referral(&[Referral::Ours, Referral::Theirs, Referral::Unknown]), Referral::Theirs);
        assert_eq!(merged_referral(&[Referral::Ours, Referral::Unknown]), Referral::Ours);
        assert_eq!(merged_referral(&[Referral::Unknown]), Referral::Unknown);
        assert_eq!(merged_referral(&[]), Referral::Unknown);
    }

    #[test]
    fn one_build_reading_a_pointer_condemns_a_use_and_a_reading_through_clears_it() {
        assert_eq!(merged_use(&[Use::Clear, Use::Pointer]), Use::Pointer);
        assert_eq!(merged_use(&[Use::Unknown, Use::Clear]), Use::Clear);
        assert_eq!(merged_use(&[Use::Unknown]), Use::Unknown);
        assert_eq!(merged_use(&[]), Use::Unknown);
    }

    #[test]
    fn the_analysis_reads_the_slices_the_toolchain_builds_and_refuses_one_it_has_no_build_for() {
        let shipped: Vec<(&str, &str, &str)> = apple_slices(&crate::repo_root()).unwrap().into_iter().map(|slice| (slice.1, slice.2, slice.3)).collect();
        assert_eq!(shipped, [("arm64", "", "iphoneos"), ("arm64", "-simulator", "iphonesimulator")], "the device and the simulator, both arm64");
        let scratch = crate::scratch::Scratch::new("apple-slices");
        scratch.write(TOOLCHAIN, "[toolchain]\ntargets = [\"x86_64-apple-ios\", \"aarch64-linux-android\"]\n");
        assert_eq!(
            apple_slices(&scratch.0).unwrap().into_iter().map(|slice| slice.1).collect::<Vec<_>>(),
            ["x86_64"],
            "an Intel simulator slice is read as one"
        );
        scratch.write(TOOLCHAIN, "[toolchain]\ntargets = [\"aarch64-apple-ios\", \"aarch64-apple-tvos\"]\n");
        assert!(apple_slices(&scratch.0).is_err_and(|error| error.contains("`aarch64-apple-tvos`")), "a slice with no build stops the stage");
        scratch.write(TOOLCHAIN, "[toolchain]\ntargets = [\"aarch64-linux-android\"]\n");
        assert!(apple_slices(&scratch.0).is_err(), "no Apple slice at all");
    }

    #[test]
    fn the_doctor_checks_the_sdk_of_every_slice_row() {
        // A missing SDK stops the stage with a message that sends the reader to tools/doctor.sh, so the
        // doctor must check every SDK any row of the table could build against: a row added without its
        // SDK in the doctor's loop would fail with a promise the doctor cannot keep.
        let doctor = std::fs::read_to_string(crate::repo_root().join("tools/doctor.sh")).unwrap();
        let mut sdks: Vec<&str> = APPLE_SLICES.iter().map(|slice| slice.3).collect();
        sdks.sort_unstable();
        sdks.dedup();
        for sdk in sdks {
            assert!(doctor.contains(&format!("\"{sdk}:")), "tools/doctor.sh checks no {sdk} SDK, which a row of APPLE_SLICES builds against");
        }
    }

    #[test]
    fn file_uris_survive_the_round_trip() {
        assert_eq!(path_of(&uri_of(Path::new("/tmp/a b#c/dé.swift"))).as_deref(), Some("/tmp/a b#c/dé.swift"));
        assert_eq!(path_of("sourcekit-lsp://generated-swift-interface/Swift.swiftinterface"), None);
    }

    #[test]
    fn definition_answers_of_every_shape_are_read() {
        let location = json!({"uri": "file:///a.swift", "range": {"start": {"line": 3, "character": 4}}});
        let link = json!({"targetUri": "file:///b.swift", "targetSelectionRange": {"start": {"line": 1, "character": 2}}});
        assert_eq!(locations(&location), [("file:///a.swift".to_string(), (3, 4))]);
        assert_eq!(locations(&json!([location, link])).len(), 2);
        assert!(locations(&Value::Null).is_empty());
    }

    #[test]
    fn messages_are_read_off_the_pipe_one_at_a_time() {
        let body = r#"{"id":1,"result":null}"#;
        let pipe = format!("Content-Length: {}\r\n\r\n{body}Content-Length: 2\r\n\r\n{{}}", body.len());
        let mut reader = std::io::Cursor::new(pipe.into_bytes());
        assert_eq!(read_message(&mut reader), Some(json!({"id": 1, "result": null})));
        assert_eq!(read_message(&mut reader), Some(json!({})));
        assert_eq!(read_message(&mut reader), None);
    }
}
