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
//! of ours clears it, and no answer is no clearance.
//!
//! Neither the tools nor the stage may read less than the package, and both tools' own answers guard that.
//! Before the server for a build starts, SwiftPM itself builds and prepares the package for indexing for
//! that build, through the same pinned tools, into the scratch the server prepares its modules against:
//! a prepare-for-indexing plan lists a binary target's staged library as an input to every module that
//! links it but never stages that copy itself, so a package with a binary target would fail that
//! preparation and every module importing the binary would read against a missing or stale prepared
//! module. A preparation that fails fails the stage, naming the build and carrying SwiftPM's own answer,
//! and so does a file the server still cannot read, as the diagnostics it reports for the file say. Nor may
//! the server answer with less than it read: each of its own timeouts, past which it answers with what it
//! has, is set beyond the stage's waits, each question the stage asks has a deadline of its own however much
//! the server says meanwhile, and the manifest is opened only once the package has settled. A check that
//! silently reads less than the package passes code it never read. On that reading:
//!
//! * outside test code, a use of a banned call (`fatalError`, `exit`, `.unsafelyUnwrapped`, …) fails unless
//!   every declaration it refers to is one of this repository's files: a module qualifier, backticks or the
//!   function taken as a value change nothing, and a name the server cannot resolve fails too, because a
//!   check that cannot read what a call means cannot pass it;
//! * outside test code, every postfix `!` fails: the server classifies every operator it reads, so a `!` it
//!   leaves unclassified is the punctuation of a force unwrap, an implicitly unwrapped optional, an `init!` or
//!   a key path's `!`; `try!` and `as!` are swift-format's (`NeverUseForceTry`, `NeverForceUnwrap`);
//! * outside a `Platform/` folder, a pointer API by name, a binding to a C symbol, and a type name whose
//!   alias chain ends at a pointer type, an alias of ours or of the SDK's; a generic parameter is no alias,
//!   so one the compiler names a generic parameter of one of the package's own modules — the `Self` a
//!   `where` clause narrows among them — passes though nothing declares it, while an alias the check
//!   cannot read, as one from a module that ships without its source or a C header's `typedef`, still fails;
//! * everywhere, a SwiftLint command, which switches SwiftLint's rules off, a swift-format ignore that
//!   switches every rule off, reaches beyond one function or property, or gives no reason, and the test
//!   frameworks swift-format exempts from its never-crash rules: XCTest anywhere, Swift Testing outside a
//!   test target.
//!
//! Test code is SwiftPM's answer — the files `swift package describe` puts in a test target — and a Platform
//! folder is a component of the path the repository names the file by: neither is a pattern over an absolute
//! path, which the folders a checkout happens to sit in could match.
//!
//! UniFFI's Swift bindings are read like every other file, bar the rules its fixed templates cannot meet
//! ([`BINDINGS_EXEMPTIONS`]), from which their folder alone is exempt.
//!
//! The same reading of the package answers what the rules cannot ask without the native library built,
//! SwiftPM describing no package whose binary target is missing: where each regular target sits, and that
//! every target compiles only files the checks read and is built by the package scheme
//! (`crate::direction::resolved_errors`).

use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

/// How long one language-server question may take before the check calls the server broken.
const ANSWER_WAIT: Duration = Duration::from_secs(120);

/// How long the server may take to load the package and build what its analysis needs, once, before any
/// question: a cold machine compiles the SDK's modules for the build's destination first, and that is
/// minutes, not seconds. A silence beyond this is none.
const SETTLE_WAIT: Duration = Duration::from_secs(900);

/// Past every wait of the check's own: what each of the server's own timeouts is set to. SourceKit-LSP answers
/// a request that outlives one of them with what it has — a file's syntax without its semantics, no build
/// targets, an index cut short — and calls that an answer, which a check would read as the file. Set past the
/// check's waits, a stall ends as the check's own failure instead, naming what it asked.
const SERVER_PATIENCE: Duration = Duration::from_secs(SETTLE_WAIT.as_secs() * 2);

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
pub(crate) const SWIFTLINT_COMMAND: &str = "swiftlint:";

/// How the postfix-`!` rule names itself in its findings and in [`BINDINGS_EXEMPTIONS`].
const POSTFIX_BANG: &str = "postfix !";

/// The rules of this stage UniFFI's Swift cannot meet, each by the name its findings go by — SwiftLint's
/// rule id, a banned call's name, [`POSTFIX_BANG`] — with what in UniFFI's fixed templates breaks it. The
/// folder its Swift is generated into ([`crate::swift_bindings::holds`]) alone is exempt from them. Every
/// other rule holds there — the pointer confinement, which its `Platform/` folder meets, every other banned
/// call, the test frameworks, the directives and the `swiftlint:` commands, which the generator removes —
/// and every rule holds everywhere else, the command facade's generated Swift included.
const BINDINGS_EXEMPTIONS: &[(&str, &str)] = &[
    ("missing_docs", "UniFFI makes public the converters it writes for every type (`FfiConverter…`, `lift`, `lower`) and documents none of them"),
    (
        "fatalError",
        "UniFFI stops the process when the library's contract version or checksums differ from the Swift's, and in the converters and calls it writes for what the library never sends",
    ),
    (POSTFIX_BANG, "UniFFI force-unwraps the data of every buffer it reads"),
];

/// Whether `file`, as the repository names it, is exempt from `rule`: the bindings' folder from
/// [`BINDINGS_EXEMPTIONS`], nothing else from anything.
fn exempt(file: &str, rule: &str) -> bool {
    crate::swift_bindings::holds(file) && BINDINGS_EXEMPTIONS.iter().any(|(exempted, _)| *exempted == rule)
}

/// The LSP symbol kinds of a declaration that holds others: module, namespace (an extension), package,
/// class (an actor too), enum, interface (a protocol), object and struct.
const REGION_KINDS: &[u64] = &[2, 3, 4, 5, 10, 11, 19, 23];

/// LSP's type-parameter symbol kind, which SourceKit-LSP gives two ways. Its document symbols give it to
/// every declaration that names a type it does not define — an alias, an associated type, a generic
/// parameter — so the walk reads a declaration of this kind as an alias. Its symbol information gives it
/// only where the compiler reports a generic type parameter, `Self` among them, and reports an alias or an
/// associated type with no kind at all.
const TYPE_PARAMETER_KIND: u64 = 26;

/// How many aliases deep a chain may go before the check stops trusting it.
const ALIAS_DEPTH: usize = 8;

/// Whether a `textDocument/symbolInfo` answer names only generic parameters of `modules`, the package's
/// own source modules. The compiler declares some generic parameters itself — the `Self` of a protocol or
/// of an extension narrowed with `where`, the parameter of a type that a constrained extension takes
/// over — so a jump to their definition finds nothing, yet a generic parameter is no alias and reaches no
/// pointer type. Both halves are the server's own word: the kind is the compiler's, and the module says
/// the name is one of ours, whose every declaration the server holds. A name with no declaration in reach
/// that is anything else stays unread: an alias from a module that ships without its source is placed at
/// no declaration too, and is still an alias, so it must still fail.
fn names_generic_parameters(answer: &Value, modules: &BTreeSet<String>) -> bool {
    let Some(candidates) = answer.as_array() else {
        return false;
    };
    !candidates.is_empty()
        && candidates.iter().all(|candidate| {
            candidate.get("kind").and_then(Value::as_u64) == Some(TYPE_PARAMETER_KIND)
                && candidate.pointer("/systemModule/moduleName").and_then(Value::as_str).is_some_and(|module| modules.contains(module))
        })
}

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

/// The repository's resolved path, the package as SwiftPM describes it, and what that description breaks.
fn described(root: &Path) -> Result<(PathBuf, Vec<crate::direction::SwiftTarget>, Vec<String>), String> {
    let canonical = std::fs::canonicalize(root).map_err(|error| format!("cannot resolve the repository's path: {error}"))?;
    let package = canonical.join(PACKAGE);
    let targets = crate::direction::describe_package(&package, MANIFEST)?.unwrap_or_default();
    let resolved = crate::direction::resolved_errors(&package, &targets)?;
    Ok((canonical, targets, resolved))
}

pub(crate) fn run(root: &Path) -> ExitCode {
    let (canonical, targets, resolved) = match described(root) {
        Ok(described) => described,
        Err(failure) => return crate::report::failed("swift-lint", failure),
    };
    // What SwiftPM resolved — which only a package whose binary target is built can answer, so it is judged
    // here rather than by the rules, which read the manifest alone — is printed before the analysis runs: a
    // package the analysis cannot read, such as one with a target no build prepares, still has it judged.
    for line in &resolved {
        println!("{line}");
    }
    match check(root, &canonical, &targets) {
        Ok(lines) => {
            for line in &lines {
                println!("{line}");
            }
            if lines.is_empty() && resolved.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) }
        }
        Err(failure) => crate::report::failed("swift-lint", failure),
    }
}

/// Read every listed Swift file as the stage's rules say, `canonical` the repository's resolved path and
/// `targets` the package as SwiftPM describes it.
fn check(root: &Path, canonical: &Path, targets: &[crate::direction::SwiftTarget]) -> Result<Vec<String>, String> {
    let files = crate::files::listed(root, "swift")?;
    if files.is_empty() {
        return Err("no Swift files for the linter to read".to_string());
    }
    let mut findings = run_swiftlint(root, canonical, &files)?;
    let package = canonical.join(PACKAGE);
    let compiled: BTreeSet<String> = targets.iter().flat_map(|target| target.sources.iter()).map(|source| format!("{PACKAGE}/{source}")).collect();
    let tests: BTreeSet<String> = crate::direction::test_sources(targets).into_iter().map(|source| format!("{PACKAGE}/{source}")).collect();
    // A binary target is a module too, but one built from no source of ours, so nothing says what it declares.
    let modules: BTreeSet<String> = targets.iter().filter(|target| !target.sources.is_empty()).map(|target| target.name.clone()).collect();
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
    for build in Build::all(canonical, &package)? {
        prepare_for_index(&package, &build)?;
        let mut server = LanguageServer::start(&package, build)?;
        server.modules.clone_from(&modules);
        let mut read: Vec<(&str, String)> = Vec::new();
        // The files a target compiles are opened, and the package settles on them, before the rest: the
        // manifest above all. Opened while the server is still loading a package that links a binary target
        // with C headers, the manifest stalls the server's semantic service, which then answers every file
        // with its syntax alone, and a type name read that way is no name the checks resolve.
        let (built, unbuilt): (Vec<_>, Vec<_>) = files.iter().zip(&texts).partition(|(file, _)| compiled.contains(file.as_str()));
        for opened in [built, unbuilt] {
            for (file, text) in opened {
                let path = canonical.join(file);
                let uri = uri_of(&path);
                server.ours.insert(path);
                server.open(&uri, text)?;
                read.push((file.as_str(), uri));
            }
            server.settle()?;
        }
        let mut prepared: Vec<(&str, String)> = Vec::new();
        for (file, uri) in &read {
            if compiled.contains(*file) {
                server.require_build_settings(uri, file)?;
                prepared.push((*file, uri.clone()));
            }
        }
        let members = server.pointer_alias_names(&read);
        for (file, uri) in &read {
            let document = server.document(uri)?;
            findings.extend(ignore_directives(file, &document));
            findings.extend(test_framework_uses(file, &document, tests.contains(*file)));
            if !tests.contains(*file) {
                if !exempt(file, POSTFIX_BANG) {
                    findings.extend(postfix_bangs(file, &document));
                }
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
        server.refuse_unread_reads(&prepared)?;
        server.shutdown();
    }
    for ((file, line, column), (name, referrals)) in &banned {
        if merged_referral(referrals) != Referral::Ours && !exempt(file, name) {
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
    let mut findings = Vec::new();
    for entry in entries {
        // A violation this check cannot read is refused rather than dropped: dropping it would pass it.
        let (finding, rule) = violation(entry, canonical).ok_or_else(|| format!("swiftlint reported a violation this check cannot read: {entry}"))?;
        if !exempt(&finding.0, rule) {
            findings.push(finding);
        }
    }
    Ok(findings)
}

/// One violation of SwiftLint's JSON reporter as a finding, with the id of the rule it breaks. The reporter
/// prints canonical absolute paths; the gate prints paths as the repository names them. A violation of a
/// whole line has no column.
fn violation<'a>(entry: &'a Value, canonical: &Path) -> Option<(Finding, &'a str)> {
    let reported = entry.get("file")?.as_str()?;
    let file = Path::new(reported).strip_prefix(canonical).map(|relative| relative.display().to_string()).unwrap_or_else(|_| reported.to_string());
    let rule = entry.get("rule_id")?.as_str()?;
    let message = format!("{}: {} ({rule})", entry.get("severity")?.as_str()?, entry.get("reason")?.as_str()?);
    Some(((file, entry.get("line")?.as_u64()? as usize, entry.get("character").and_then(Value::as_u64).unwrap_or(1) as usize, message), rule))
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
                    format!("{POSTFIX_BANG}: a force unwrap or an implicitly unwrapped optional, a crash on nil"),
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

    /// The aliases declared at the position a jump to a definition lands on, or `None` where no Swift
    /// declaration stands there: a C header, say, whose `typedef void *` the importer makes an alias of a
    /// pointer type, so a walk that reads no declaration there has not read what the name is.
    fn aliases_at(&self, position: (usize, usize)) -> Option<Vec<Symbol>> {
        let named: Vec<&Symbol> = self.symbols.iter().filter(|symbol| symbol.name_start <= position && position <= symbol.name_end).collect();
        if named.is_empty() {
            return None;
        }
        Some(named.into_iter().filter(|symbol| symbol.kind == TYPE_PARAMETER_KIND).cloned().collect())
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
        let version = crate::direction::ios_version(&manifest)?;
        let mut builds = Vec::new();
        for slice in crate::pins::apple_slices(root)? {
            let sdk = xcrun_sdk(slice.sdk)?;
            for configuration in ["debug", "release"] {
                builds.push(Build {
                    label: format!("{configuration} {}", slice.message),
                    configuration,
                    triple: format!("{}-apple-ios{version}{}", slice.architecture, slice.environment),
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
        let patience = SERVER_PATIENCE.as_secs();
        object([
            ("defaultWorkspaceType", Value::from("swiftPM")),
            ("backgroundIndexing", Value::from(true)),
            ("buildSettingsTimeout", Value::from(SETTLE_WAIT.as_millis() as u64)),
            ("sourcekitdRequestTimeout", Value::from(patience)),
            ("semanticServiceRestartTimeout", Value::from(patience)),
            ("buildServerWorkspaceRequestsTimeout", Value::from(patience)),
            ("index", object([("updateIndexStoreTimeout", Value::from(patience))])),
            ("experimentalFeatures", Value::from(vec!["sourcekit-options-request"])),
            (
                "swiftPM",
                object([
                    ("triple", Value::from(self.triple.as_str())),
                    ("configuration", Value::from(self.configuration)),
                    ("scratchPath", Value::from(INDEX_SCRATCH)),
                    ("swiftCompilerFlags", Value::from(self.swift_sdk_flags().to_vec())),
                    ("cCompilerFlags", Value::from(self.c_sdk_flags().to_vec())),
                ]),
            ),
        ])
    }

    /// The Swift compiler's flags that point it, and its C importer, at this build's SDK.
    fn swift_sdk_flags(&self) -> [&str; 6] {
        ["-sdk", self.sdk.as_str(), "-Xcc", "-isysroot", "-Xcc", self.sdk.as_str()]
    }

    /// The C compiler's flag that points it at this build's SDK.
    fn c_sdk_flags(&self) -> [&str; 2] {
        ["-isysroot", self.sdk.as_str()]
    }
}

/// The scratch path, relative to the package, that SwiftPM prepares the package into for the index: the
/// check's own builds below write it, and the server is told it as `swiftPM.scratchPath`, so its preparation
/// and its analysis read the very modules those builds made.
const INDEX_SCRATCH: &str = ".build/index-build";

/// Build what the language server's index preparation needs, through the same pinned tools, before its
/// server starts for one build: SwiftPM's own package build, then the package prepared for indexing, for
/// the same destination and configuration, into the scratch the server prepares against.
///
/// A prepare-for-indexing plan lists a binary target's staged library as an input to every module that
/// links it, but a build prepared for indexing never stages that copy itself, so with a binary target in
/// the package a preparation from an empty scratch fails on the missing library, the server's own
/// preparation fails the same way, and every module that imports the binary reads against a missing or
/// stale prepared module and loses its findings. Building the package itself first stages what the
/// preparation lacks, so the preparation — this one and the server's — succeeds. SwiftPM's own answers
/// are the verdict: a nonzero exit status fails the stage, naming the build and carrying what SwiftPM
/// itself said, the module it could not build or the dependency it could not resolve; the stage never
/// carries on past a package it could not build for the index to read.
fn prepare_for_index(package: &Path, build: &Build) -> Result<(), String> {
    let scratch = package.join(INDEX_SCRATCH);
    for preparation in [false, true] {
        let output = swift_pm_build(package, &scratch, build, preparation)?;
        if let Some(failure) = preparation_failure(build, preparation, output.status.code(), &build_output(&output)) {
            return Err(failure);
        }
    }
    Ok(())
}

/// Run one of the two SwiftPM builds the index needs, with the destination, configuration and SDK
/// arguments the server compiles with, so one build plan serves the server's own preparation as well.
fn swift_pm_build(package: &Path, scratch: &Path, build: &Build, preparation: bool) -> Result<std::process::Output, String> {
    let mut command = Command::new("xcrun");
    command.current_dir(package).args(["swift", "build", "--scratch-path"]).arg(scratch).args([
        "--disable-index-store",
        "-c",
        build.configuration,
        "--triple",
        build.triple.as_str(),
    ]);
    for flag in build.swift_sdk_flags() {
        command.args(["-Xswiftc", flag]);
    }
    for flag in build.c_sdk_flags() {
        command.args(["-Xcc", flag]);
    }
    if preparation {
        command.arg("--experimental-prepare-for-indexing");
    }
    command.output().map_err(|error| format!("cannot run SwiftPM to prepare the {} build for the index: {error}", build.label))
}

/// The stage's verdict on one preparation build: no verdict while SwiftPM's own exit status is zero, and
/// a failure naming the build and what was asked of it, carrying SwiftPM's own output — which is what
/// names the module that could not be built. Only the tool's own exit status decides; no spelling of a
/// message is matched on.
fn preparation_failure(build: &Build, preparation: bool, status: Option<i32>, output: &str) -> Option<String> {
    (status != Some(0)).then(|| {
        format!(
            "{}: SwiftPM could not {} (exit status {}); every module must be read against a prepared build, SwiftPM's own answer: {}",
            build.label,
            if preparation { "prepare the package for indexing" } else { "build the package to stage what indexing needs" },
            status.map(|code| code.to_string()).unwrap_or_else(|| "killed by a signal".to_string()),
            tail(output.trim(), 4000),
        )
    })
}

/// The last characters of a build's own output, which is where its verdict sits: the whole of a short one,
/// the tail of a long one, so a failure carries what the tool said without drowning the report.
fn tail(text: &str, limit: usize) -> String {
    let length = text.chars().count();
    if length <= limit {
        return text.to_string();
    }
    format!("…{}", text.chars().skip(length.saturating_sub(limit)).collect::<String>())
}

/// What SwiftPM answered with, both of its streams, so nothing it said is lost.
fn build_output(output: &std::process::Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
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
    /// The modules SwiftPM builds from this package's own sources, each named as its target is.
    modules: BTreeSet<String>,
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
            modules: BTreeSet::new(),
        };
        let capabilities = object([
            (
                "textDocument",
                object([
                    ("definition", object([("linkSupport", Value::from(false))])),
                    // Diagnostics are asked for, so each file's are an answer the check waits for, never a
                    // notification that may not have come yet; the server registers the request for itself.
                    ("diagnostic", object([("dynamicRegistration", Value::from(true))])),
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

    /// Open a document once, and wait for the server's answer to a question about it before anything else is
    /// sent. SourceKit-LSP stops answering every request once about ninety `didOpen` notifications reach it
    /// back to back (Xcode 26.6's: with ninety of the package's files opened at once not even `semanticTokens`
    /// answered, while the same ninety opened one at a time each answered at once), so a package grown past
    /// that would leave the stage waiting on a server that never answers. The question is the document's
    /// symbols, which the server reads from the text alone, so the wait is for the open and for no build.
    fn open(&mut self, uri: &str, text: &str) -> Result<(), String> {
        if self.opened.insert(uri.to_string()) {
            self.notify(
                "textDocument/didOpen",
                object([(
                    "textDocument",
                    object([("uri", Value::from(uri)), ("languageId", Value::from("swift")), ("version", Value::from(1)), ("text", Value::from(text))]),
                )]),
            )?;
            self.request("textDocument/documentSymbol", object([("textDocument", object([("uri", Value::from(uri))]))]), ANSWER_WAIT)?;
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

    /// Fail on a compiled file this build could not read against the package, on the server's own answer:
    /// the diagnostics it reports for the file when asked. SwiftPM's preparation built every module before the
    /// server started, so a file the server still cannot read — one that errors on a missing module, say —
    /// means its reading is not backed by that prepared build, and the stage may not carry on past it.
    fn refuse_unread_reads(&mut self, read: &[(&str, String)]) -> Result<(), String> {
        for (file, uri) in read {
            let report = self.request("textDocument/diagnostic", object([("textDocument", object([("uri", Value::from(uri.as_str()))]))]), ANSWER_WAIT)?;
            if let Some(diagnostic) = unread_read(&report) {
                return Err(format!(
                    "{file}: the language server cannot read this file as the {} build compiles it, so the checks would read less than the file; the server's own diagnostic: {diagnostic}",
                    self.build.label
                ));
            }
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
                for symbol in document.symbols.iter().filter(|symbol| symbol.kind == TYPE_PARAMETER_KIND) {
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
    /// the name at `at` refers to or the walk could not read the declaration it names.
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
            // Nothing to jump to: either nothing declares the name, as with the `Self` of a protocol
            // extension, which the compiler declares itself, or its declaration is out of the server's
            // reach, as with an alias from a module that ships without its source. Only the server's
            // word that the name is a generic parameter of ours tells the two apart; anything else keeps
            // no verdict, which no build may call innocent.
            let named = self.request("textDocument/symbolInfo", position_params(uri, at), ANSWER_WAIT).ok()?;
            return names_generic_parameters(&named, &self.modules).then_some(false);
        }
        for (target, position) in targets {
            let declared = self.document(&target).ok()?;
            for alias in declared.aliases_at(position)? {
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
        let stdin = &mut self.stdin;
        await_answer(&self.messages, id, method, wait, &mut |reply| send_message(stdin, reply))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&object([("jsonrpc", Value::from("2.0")), ("method", Value::from(method)), ("params", params)]))
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        send_message(&mut self.stdin, message)
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

/// What the server said it could not read in a file, from the diagnostic report it answered with: the message
/// of any error-severity diagnostic (LSP's severity 1), which is how a missing or stale prepared module reaches
/// the stage — the server's own answer names the module it cannot find. A report with no diagnostics list is
/// no reading of the file at all, so it fails too; a warning is no failure, since the server read the file.
fn unread_read(report: &Value) -> Option<String> {
    let Some(diagnostics) = report.get("items").and_then(Value::as_array) else {
        return Some("it reported no diagnostics for the file".to_string());
    };
    diagnostics
        .iter()
        .find(|diagnostic| diagnostic.get("severity").and_then(Value::as_u64) == Some(1))
        .map(|diagnostic| diagnostic.get("message").and_then(Value::as_str).unwrap_or("an error it gave no message for").to_string())
}

/// Wait for the answer to request `id`, until `wait` has passed since it was asked. A request the server sends
/// meanwhile (a progress token, a capability it registers) is answered at once through `reply`, so the server
/// never waits on the check while the check waits on it, and a notification is passed over. The deadline is
/// the request's, not the last message's: a server that keeps reporting progress on a request it never
/// answers must not keep the check waiting forever.
fn await_answer(
    messages: &Receiver<Value>,
    id: u64,
    method: &str,
    wait: Duration,
    reply: &mut dyn FnMut(&Value) -> Result<(), String>,
) -> Result<Value, String> {
    let deadline = Instant::now() + wait;
    loop {
        let message = messages.recv_timeout(deadline.saturating_duration_since(Instant::now())).map_err(|error| match error {
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
                reply(&object([("jsonrpc", Value::from("2.0")), ("id", asked.clone()), ("result", Value::Null)]))?;
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

/// One JSON-RPC message onto the server's stdin, framed as the protocol frames it.
fn send_message(stdin: &mut impl Write, message: &Value) -> Result<(), String> {
    let body = message.to_string();
    stdin
        .write_all(format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes())
        .map_err(|error| format!("cannot write to the language server: {error}"))?;
    stdin.flush().map_err(|error| format!("cannot flush the language server's stdin: {error}"))
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
    fn only_the_bindings_folder_is_exempt_and_only_from_the_rules_its_templates_break() {
        let bindings = "ios/sdk/Sources/CraftBridgeFFI/Platform/Generated/craftbridge_core_ffi.swift";
        for rule in ["missing_docs", "fatalError", POSTFIX_BANG] {
            assert!(exempt(bindings, rule), "{rule}");
            for written in [
                "ios/sdk/Sources/CraftBridgeFFI/Platform/Written.swift",
                "ios/sdk/Sources/CraftBridgeEffects/Generated/LayerCommands.swift",
                "ios/sdk/Sources/CraftBridge/CraftBridge.swift",
            ] {
                assert!(!exempt(written, rule), "{written}: {rule}");
            }
        }
        for (name, _) in BANNED_CALLS.iter().filter(|(name, _)| *name != "fatalError") {
            assert!(!exempt(bindings, name), "{name} still holds in the bindings");
        }
        assert!(!exempt(bindings, "unsafeBitCast") && !exempt(bindings, "XCTest"));
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
    fn only_a_generic_parameter_of_the_package_s_own_modules_passes_with_nothing_declaring_it() {
        // The `symbolInfo` answers sourcekit-lsp 6.3.3 gives, in every build the stage reads, where
        // `definition` has nothing to jump to; the fields the check reads are as the server sent them.
        let ours: BTreeSet<String> = ["CraftBridge", "CraftBridgeEffects"].map(str::to_string).into();
        let generic_parameter = |name: &str, module: &str, usr: &str| {
            json!([{"name": name, "usr": usr, "kind": 26, "isSystem": false, "isDynamic": name == "Self", "receiverUsrs": [],
                    "systemModule": {"moduleName": module}}])
        };
        // The compiler declares these itself: the `Self` of `extension JSONRepresentable where Self:
        // RawRepresentable, RawValue == String`, the `Self` of an extension one module makes of another's
        // protocol, and the parameter `Value` that `extension Box where Value == Int` takes over.
        for (name, module, usr) in [
            ("Self", "CraftBridge", "s:11CraftBridge17JSONRepresentablePAASYRzSS8RawValueSYRtzrlE4Selfxmfp"),
            ("Self", "CraftBridgeEffects", "s:11CraftBridge8PointingP0aB7EffectsE4Selfxmfp"),
            ("Value", "CraftBridge", "s:11CraftBridge3BoxVAASvRszlE5Valuexmfp"),
        ] {
            assert!(names_generic_parameters(&generic_parameter(name, module, usr), &ours), "{name} of {module} is a generic parameter, no alias");
        }
        // An alias from a module that ships without its source, `public typealias Handle =
        // UnsafeMutableRawPointer` in a binary target: no declaration in reach either, and still an alias.
        let binary_alias = json!([{"name": "Handle", "usr": "s:6Vendor6Handlea", "isSystem": false, "isDynamic": false, "receiverUsrs": [],
                                   "systemModule": {"moduleName": "Vendor"}}]);
        assert!(!names_generic_parameters(&binary_alias, &ours), "an alias the server cannot read must still fail");
        // Each half alone is no clearance: an alias of ours whose declaration the server lost is still an
        // alias, a server that marked a binary target's alias with the type-parameter kind still names a
        // module of no source of ours, and a generic parameter no module claims is unplaced.
        let mut lost = binary_alias.clone();
        lost[0]["systemModule"]["moduleName"] = json!("CraftBridge");
        assert!(!names_generic_parameters(&lost, &ours), "a name of ours that is no generic parameter may be an alias");
        let mut marked = binary_alias.clone();
        marked[0]["kind"] = json!(26);
        assert!(!names_generic_parameters(&marked, &ours), "a module of no source of ours says nothing about what it declares");
        let mut unplaced = generic_parameter("Self", "CraftBridge", "s:11CraftBridge8PointingPAAE4Selfxmfp");
        unplaced[0].as_object_mut().unwrap().remove("systemModule");
        assert!(!names_generic_parameters(&unplaced, &ours), "a name no module of ours claims is not known to be ours");
        // Every candidate must be one: a reading that also offers an alias offers a pointer it cannot rule out.
        let both = json!([generic_parameter("Self", "CraftBridge", "s:11CraftBridge8PointingPAAE4Selfxmfp")[0], binary_alias[0]]);
        assert!(!names_generic_parameters(&both, &ours), "an alias among the candidates keeps the use unread");
        assert!(!names_generic_parameters(&json!([]), &ours), "a name no answer places keeps no verdict, which is no clearance");
        assert!(!names_generic_parameters(&Value::Null, &ours));
    }

    #[test]
    fn a_jump_to_where_no_swift_declaration_stands_reads_nothing() {
        // A jump lands on a declaration's name: an alias, whose right-hand side the walk then reads, or a
        // type that is no alias. A C header the walk opens holds no Swift declaration at the place the
        // server jumped to, so what the importer made of its `typedef` stays unread.
        let swift = document(
            "typealias Handle = Platform\nstruct Box {}\n",
            &[],
            json!([
                {"kind": 26, "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 27}},
                 "selectionRange": {"start": {"line": 0, "character": 10}, "end": {"line": 0, "character": 16}}},
                {"kind": 23, "range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 13}},
                 "selectionRange": {"start": {"line": 1, "character": 7}, "end": {"line": 1, "character": 10}}}
            ]),
        );
        assert_eq!(swift.aliases_at((0, 10)).map(|aliases| aliases.len()), Some(1), "an alias is read");
        assert_eq!(swift.aliases_at((1, 7)).map(|aliases| aliases.len()), Some(0), "a type that is no alias is no pointer alias");
        let header = document("typedef void *Handle;\n", &[], json!([]));
        assert!(header.aliases_at((0, 14)).is_none(), "no declaration read is no verdict");
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

    fn preparation_build() -> Build {
        Build { label: "debug iOS device".into(), configuration: "debug", sdk: "/sdk/iphoneos".into(), triple: "arm64-apple-ios17.0".into() }
    }

    #[test]
    fn a_preparation_that_succeeds_is_no_failure() {
        // What the pinned SwiftPM answers when the index scratch is staged and its modules prepare.
        assert_eq!(preparation_failure(&preparation_build(), true, Some(0), "Build complete! (10.41s)"), None);
    }

    #[test]
    fn a_preparation_that_cannot_build_fails_naming_the_module_swiftpm_named() {
        // Captured verbatim from the pinned SwiftPM 6.3.3 (Xcode 26.6) preparing a package whose binary
        // target had not been staged into the index scratch: a prepare-for-indexing build lists the
        // binary's library as an input but stages nothing, so it fails where the stage must not carry on.
        let captured = "error: couldn't build /private/tmp/b1-ops9s/pkg/.build/index-build/arm64-apple-ios/debug/Modules/CraftBridgeEffects.swiftmodule because of missing inputs: /private/tmp/b1-ops9s/pkg/.build/index-build/arm64-apple-ios/debug/libB1Bin.a";
        let failure = preparation_failure(&preparation_build(), true, Some(1), captured).expect("a preparation that exits nonzero fails the stage");
        assert!(failure.contains("debug iOS device"), "the failure names the build: {failure}");
        assert!(failure.contains("CraftBridgeEffects.swiftmodule"), "the failure carries the module SwiftPM could not build: {failure}");
        // The same answer from the staging build fails too: the stage reads SwiftPM's exit status, not a
        // spelling of one step's message.
        assert!(preparation_failure(&preparation_build(), false, Some(1), captured).is_some());
    }

    #[test]
    fn a_preparation_killed_counts_as_a_failure() {
        assert!(preparation_failure(&preparation_build(), true, None, "").is_some(), "a build the system killed read no module");
    }

    #[test]
    fn a_failure_carries_the_tools_own_answer_whole_when_it_is_short() {
        assert_eq!(tail("the tool said it", 4000), "the tool said it");
        assert_eq!(tail("", 4000), "", "an output that said nothing is carried whole");
        // The limit counts characters, not bytes, and one at exactly the limit is still whole.
        let exact = "x".repeat(100);
        assert_eq!(tail(&exact, 100), exact, "exactly the limit is no cut");
        assert_eq!(tail(&"é".repeat(100), 100), "é".repeat(100), "the limit counts characters, not bytes");
        // One past the limit is the first cut, and the cut eats from the front: what is kept is the end,
        // where the tool's verdict sits. Text whose halves differ says the cut kept the right one.
        assert_eq!(tail(&format!("{exact}y"), 100), format!("…{}y", "x".repeat(99)), "one character past the limit keeps its last and drops its first");
        assert_eq!(tail(&format!("{}verdict", "h".repeat(500)), 7), "…verdict", "a long answer keeps its end");
    }

    #[test]
    fn a_file_the_server_cannot_read_fails_naming_the_module_the_server_named() {
        // The diagnostic the pinned SourceKit-LSP (sourcekit-lsp 6.3.3) gives a file whose module's
        // preparation had failed, while `workspace/synchronize` itself answered success: the server's own
        // reading of the file is where a missing prepared module shows up. Asked for, it comes as a full report.
        let report = json!({"kind": "full", "items": [{"severity": 1, "source": "SourceKit", "message": "No such module 'CraftBridge'",
            "range": {"start": {"character": 7, "line": 1}, "end": {"character": 7, "line": 1}}, "tags": []}]});
        assert_eq!(unread_read(&report).as_deref(), Some("No such module 'CraftBridge'"));
        assert_eq!(unread_read(&json!({"kind": "full", "items": []})), None, "a clean report is no failure");
        assert_eq!(
            unread_read(&json!({"kind": "full", "items": [{"severity": 2, "message": "a warning"}]})),
            None,
            "a warning is not what the server cannot read"
        );
        assert!(unread_read(&Value::Null).is_some(), "no report is no reading of the file");
        assert!(unread_read(&json!({"kind": "unchanged", "resultId": "1"})).is_some(), "a report with no diagnostics read nothing the check asked for");
    }

    #[test]
    fn a_question_has_its_own_deadline_however_much_the_server_says_meanwhile() {
        // A server that reports progress every few milliseconds and never answers: the question must still
        // fail once its own wait is up, not wait on for as long as the server keeps talking.
        let (sender, messages) = mpsc::channel();
        let talker = thread::spawn(move || {
            for _ in 0..300 {
                if sender.send(json!({"jsonrpc": "2.0", "method": "window/logMessage", "params": {"type": 3, "message": "Indexing"}})).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        let asked = Instant::now();
        let answer = await_answer(&messages, 7, "textDocument/semanticTokens/full", Duration::from_millis(300), &mut |_| Ok(()));
        let waited = asked.elapsed();
        drop(messages);
        let _ = talker.join();
        assert!(answer.is_err_and(|error| error.contains("gave no answer to the `textDocument/semanticTokens/full` request")));
        assert!(waited < Duration::from_secs(2), "the question waited {waited:?}, past its own deadline, while the server talked on");
    }

    #[test]
    fn a_question_answers_what_the_server_asks_and_reads_its_own_answer() {
        let (sender, messages) = mpsc::channel();
        for message in [
            json!({"jsonrpc": "2.0", "id": 1, "method": "client/registerCapability", "params": {}}),
            json!({"jsonrpc": "2.0", "id": 6, "result": "another question's"}),
            json!({"jsonrpc": "2.0", "id": 7, "result": {"data": [1]}}),
        ] {
            sender.send(message).unwrap();
        }
        let mut replies = Vec::new();
        let answer = await_answer(&messages, 7, "textDocument/semanticTokens/full", Duration::from_secs(5), &mut |reply| {
            replies.push(reply.clone());
            Ok(())
        });
        assert_eq!(answer, Ok(json!({"data": [1]})));
        assert_eq!(replies, [json!({"jsonrpc": "2.0", "id": 1, "result": null})], "the server's own request is answered at once");
        sender.send(json!({"jsonrpc": "2.0", "id": 8, "error": {"message": "no such document"}})).unwrap();
        assert_eq!(
            await_answer(&messages, 8, "textDocument/diagnostic", Duration::from_secs(5), &mut |_| Ok(())),
            Err("the language server refused the `textDocument/diagnostic` request: no such document".to_string())
        );
    }

    #[test]
    fn the_server_waits_longer_than_the_check_and_prepares_where_the_check_builds() {
        // Every timeout of the server's own outlasts the check's waits, so a stall ends as the check's failure
        // rather than as an answer with less in it; and the server is told the scratch the check's own builds
        // write, so it reads the modules they made.
        let options = preparation_build().options();
        let patience = SERVER_PATIENCE.as_secs();
        assert!(patience > SETTLE_WAIT.as_secs() && patience > ANSWER_WAIT.as_secs());
        for timeout in ["/sourcekitdRequestTimeout", "/semanticServiceRestartTimeout", "/buildServerWorkspaceRequestsTimeout", "/index/updateIndexStoreTimeout"]
        {
            assert_eq!(options.pointer(timeout).and_then(Value::as_u64), Some(patience), "{timeout} is the check's to set");
        }
        assert_eq!(options.pointer("/swiftPM/scratchPath").and_then(Value::as_str), Some(INDEX_SCRATCH));
        assert_eq!(options.pointer("/swiftPM/swiftCompilerFlags"), Some(&json!(["-sdk", "/sdk/iphoneos", "-Xcc", "-isysroot", "-Xcc", "/sdk/iphoneos"])));
        assert_eq!(options.pointer("/swiftPM/cCompilerFlags"), Some(&json!(["-isysroot", "/sdk/iphoneos"])));
        // This one timeout is milliseconds where the others are seconds: written in the wrong unit, or set
        // shorter than the check's own wait, the server answers a file with no build settings — a degraded
        // read that looks like an answer — before the check has stopped waiting for the package to settle.
        assert_eq!(
            options.pointer("/buildSettingsTimeout").and_then(Value::as_u64),
            Some(SETTLE_WAIT.as_millis() as u64),
            "the server waits for a file's build settings as long as the check waits for the package"
        );
    }
}
