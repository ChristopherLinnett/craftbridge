//! `cargo xtask early-pipes`: no pipe in a script that runs under `pipefail` has a reader that stops
//! reading early.
//!
//! Under `set -o pipefail` a pipeline answers with the last of its commands that failed, so it normally
//! answers with the reader's verdict — until a reader that quits before its input ends (`grep -q` at its
//! first match, `head` after the lines it prints, an `awk` reaching an `exit`) leaves the writer to die of
//! SIGPIPE, and the pipe answers 141 instead: a verdict made by timing, not by data, which for a check that
//! decides whether a device run leaves a test out means the test runs or not by chance. shellcheck has no
//! such check, so the shape is the gate's own, and this module is the whole of its one definition: `tools/ci.sh`'s `shell` stage runs this command over the same list of
//! scripts shellcheck reads (`cargo xtask files shell`).
//!
//! The rule reads shell through shell's own tooling, as AGENTS.md §10 has the gate read every language:
//! the syntax tree of mvdan/sh, the parser the shfmt formatter is built on, which `shfmt --to-json` prints
//! for each script whole. A script shfmt cannot parse is refused with shfmt's own words, so no script goes
//! unread. In that tree a pipeline is a chain of `|` or `|&` commands, wherever it stands — in a function,
//! a condition, a `$( … )` or a `< <( … )`, whose status the next edit may start reading — and every
//! command a pipe feeds is a reader, the last one and any in the middle alike, since a middle reader that
//! quits kills its writer just the same. A reader is judged by the program its command word names, the
//! last path part of a word written out (`/usr/bin/head`) or of the literal tail of one that is not
//! (`"$dir/bin/xtask"`), through `env`, `command`, `exec`, `nice` and `nohup`, which run the word after
//! them:
//!
//! - a `grep`, `egrep` or `fgrep` whose words stop it reading its input before the end: by quitting at a
//!   match (`-q`, `-m`, `-l`, and GNU's `-L`, whose answer for standard input one matching line settles), by
//!   reading a folder instead (`-r`, `-R`, `-d`), by ending before what would decide its answer is read
//!   (`-f` on a pattern file the reading cannot see empty, GNU's `--color` on a value it does not know,
//!   `--binary-files` and `-D` on a value neither knows, `--directories recurse`, FreeBSD's `-J`, `-M` and
//!   `-X` on input not of their compression, and its `-I` on input that is not text), or by
//!   printing its help or version and ending. The words are read as the grep itself reads them, by
//!   `getopt_long`, and the gate's scripts meet two greps whose options differ — FreeBSD's, which macOS
//!   ships as `/usr/bin/grep`, and GNU's, which Ubuntu ships — so they are read once for each, from one
//!   table of both greps' own options ([`GREP_LONG`], [`BSD_SHORT`], [`GNU_SHORT`]), which the tests hold to
//!   the grep the machine runs, and a pipe either would stop early is refused: a long option written whole
//!   is that option, and one abbreviated is the one option of that grep it begins; an option takes its value
//!   in the next word only where that grep makes the value required (`--context` takes one on GNU, and on
//!   FreeBSD only after `=`); both read an option after the pattern. A word either grep would read as none
//!   of its options or as more than one is refused, since what follows it cannot be read, and so is an
//!   option word, up to `--`, that is not written out;
//! - a `grep` given files to read and not `-`, the pipe itself, which reads those files and no pipe at all,
//!   leaving its writer to die of SIGPIPE with the pipe unread — every word after the pattern is a file, and
//!   every operand is one once `-e`, `-f` or their long names gave the pattern, wherever those stand. So too
//!   a `grep` left with nothing to search for — no word for its pattern, an option given no value, or a
//!   context or count written as a word neither grep reads as one — which ends at once and reads none of the
//!   pipe either. Both greps read the words alike in these two cases, so one reading answers for both;
//! - a `grep` in a script whose shell sets `GREP_OPTIONS` or `POSIXLY_CORRECT` — in the script itself or
//!   any script that sources it, or in anything one of those sources, however the `source` lines nest —
//!   either variable changing what the grep reads as its options from what the words say, so no reading of
//!   the words holds for it: macOS's grep takes option words out of `GREP_OPTIONS`, and `POSIXLY_CORRECT`
//!   makes either grep read a word after the pattern as a file rather than as an option. `tools/env.sh`,
//!   which every script the gate reads sources before its first pipe, takes both out of the environment for
//!   the rule to hold ([`GREP_ENV_VARS`]);
//! - `head`, which quits after the lines it prints;
//! - an `awk`, `gawk`, `mawk` or `nawk`, whatever its program: whether it reaches an `exit` or a
//!   `nextfile` before its input ends is awk's to say, and no tool the gate runs reads awk's syntax, so an
//!   awk is handed its input whole another way (a here-string or a file) rather than read as text here.
//!
//! A reader the tree holds no program word for — a compound command such as `while read`, or a command
//! named wholly by an expansion or through an escape (`\grep`) — is refused, since this reading cannot tell
//! it from one that quits. A word is read as written out only when the tree holds it as the very text the
//! shell hands the command ([`literal`]).
//!
//! A script answers to the rule when it runs under `pipefail`: some `set` in it (`set -o pipefail`, the
//! option bundled as in `set -euo pipefail`, `+o` turning it off; `shopt -s -o pipefail` and
//! `shopt -u -o pipefail` alike) turns it on, wherever in the script that `set` stands and whatever a later
//! one turns off — which of a script's pipes run while it is on is a question of its flow, which no syntax
//! tree answers, so the whole script is read under it rather than half of it read at all. The same stands
//! across `source` by name, to the fixpoint, for a whole shell at a time: a script that sources one holding
//! the setting runs those commands in its own shell, and under that shell fall everything they source in
//! turn. That is how `tools/env.sh`, sourced by every gate script and holding no `set` of its own, is read
//! under `pipefail`, and how a library that turns it on brings every script that sources it, and all they
//! source, under it too. A `set`, `shopt`, `source` or `.` whose words decide this but are not written out
//! is refused in every script, since what it does cannot be read.

use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

/// The verdicts, one per kind of reader, each naming the fix where it is needed.
const GREP_SHAPE: &str = "a pipe feeds a grep that stops reading its input before the end, on macOS's grep or on GNU's (-q, -m, -l, -L, -r, -R, -d, -D, -f, -V, macOS's -I, -J, -M and -X, or their long names, whole or abbreviated, --help, and the options that stop on their value or their file: --binary-files, --color, --devices, --directories, --file): under pipefail the writer dying of SIGPIPE makes the pipe answer 141 where the reader's answer belongs, so the verdict is timing; let the grep read the writer's whole output and send its own away, or judge the writer itself";
const HEAD_SHAPE: &str = "a pipe feeds head, which quits after the lines it prints: under pipefail the writer dying of SIGPIPE makes the pipe answer 141 where the reader's answer belongs, so the verdict is timing; capture the writer's whole output and keep its first lines in the shell, or judge the writer itself";
const AWK_SHAPE: &str = "a pipe feeds an awk, whose program can exit before its input ends and which no tool the gate runs reads: under pipefail the writer dying of SIGPIPE would make the pipe answer 141 by timing; capture the writer's whole output and hand it to awk as a here-string or a file";
const GREP_UNSEEN: &str =
    "a pipe feeds a grep with a word before `--` that is not written out, which could be an option that quits before its input ends: end its options with `--`";
const GREP_UNFOLLOWED: &str = "a pipe feeds a grep with an option that macOS's grep or GNU's reads as none of its own, as more than one, or with a value it takes none of, so what follows it cannot be read: write the option out whole, as both greps know it";
const GREP_FILE: &str = "a pipe feeds a grep given a file to read, or a word after its pattern that is not written out and could name one: the grep reads that file and never the pipe, whose writer dies of SIGPIPE, so the pipe answers with the file's verdict or with 141 where the file is not there, and never with what the pipe holds: give the pattern with `-e`, name no file, or `-` for the pipe itself";
const GREP_NOTHING: &str = "a pipe feeds a grep left with nothing to search for, no word left to be its pattern, an option given no value, or a context or count written as no number the grep can read: the grep ends at once and reads none of the pipe, whose writer dies of SIGPIPE, so the pipe answers with the grep's error and not with anything the pipe holds: name the pattern, and a value every option that takes one can read";
const GREP_ENV: &str = "a pipe feeds a grep in a script whose shell sets GREP_OPTIONS or POSIXLY_CORRECT, however its `source`s run: either variable changes what the grep reads as its options from what its words say, so this reading of them holds for nothing: macOS's grep takes option words out of GREP_OPTIONS, and POSIXLY_CORRECT makes either grep read a word after the pattern as a file: unset it, as tools/env.sh does for every script this rule reads";
const NAMED_UNSEEN: &str = "a pipe feeds a command named by an expansion or through an escape, which this reading cannot tell from a reader that quits before its input ends: name its program as a plain word";
const COMPOUND_READER: &str = "a pipe feeds a compound command, which this reading cannot tell from a reader that quits before its input ends: hand it the writer's output with `< <( … )`, or capture the output first";
const SET_UNSEEN: &str = "a `set` or `shopt` with an option that is not written out, so this reading cannot tell whether it turns pipefail on or off";
const SOURCE_UNSEEN: &str = "a `source` or `.` whose file is named by an expansion with no file name written at its end, so this reading cannot tell which script it brings under pipefail";

/// The programs that run the program their words name after their own options.
const WRAPPERS: &[&str] = &["env", "command", "exec", "nice", "nohup"];

/// The variables that change what a grep reads as its options, so that a script setting one cannot be read
/// by [`grep_reading`] at all: macOS's grep takes option words out of `GREP_OPTIONS`, and `POSIXLY_CORRECT`
/// makes either grep read a word after the pattern as a file rather than as an option. `tools/env.sh`, which
/// every script this rule reads sources before its first pipe, takes both out of the environment for the
/// reading to hold, and the tests hold it and every script to that.
const GREP_ENV_VARS: &[&str] = &["GREP_OPTIONS", "POSIXLY_CORRECT"];

/// One script as the rule reads it: the path it was listed by, its text, and shfmt's syntax tree of it.
struct Script {
    name: String,
    text: String,
    tree: Value,
}

/// `cargo xtask early-pipes`: the shell stage's own list, every script parsed and judged even after one is
/// named (a run that reports one problem per run would make a clean-up three runs deep). No script to check
/// is a failure, as [`crate::elf_alignment`] refuses a run that would read nothing.
pub(crate) fn run(root: &Path) -> ExitCode {
    let listed = match crate::files::listed(root, "shell") {
        Ok(listed) => listed,
        Err(reason) => return crate::report::failed("early-pipes", reason),
    };
    if listed.is_empty() {
        return crate::report::failed("early-pipes", "no script to check");
    }
    let mut scripts = Vec::new();
    let mut unread = Vec::new();
    for name in &listed {
        match std::fs::read_to_string(root.join(name)).map_err(|error| error.to_string()).and_then(|text| parse(name, &text).map(|tree| (text, tree))) {
            Ok((text, tree)) => scripts.push(Script { name: name.clone(), text, tree }),
            Err(reason) => unread.push(format!("early-pipes: {name}: {reason}")),
        }
    }
    let (problems, under) = check(&scripts);
    for problem in unread.iter().chain(&problems) {
        println!("{problem}");
    }
    if !unread.is_empty() || !problems.is_empty() {
        return ExitCode::from(1);
    }
    println!("early-pipes: ok (of {} scripts read through shfmt, {under} under pipefail)", scripts.len());
    ExitCode::SUCCESS
}

/// shfmt's syntax tree of one script, read as bash, which every script here is: shfmt reads a script from
/// its input alone when it prints the tree, and names it in its own errors by `--filename`.
fn parse(name: &str, text: &str) -> Result<Value, String> {
    let mut child = Command::new("shfmt")
        .args(["--to-json", "-ln", "bash", "--filename", name])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot run shfmt, through whose syntax tree this rule reads every script: {error}; brew install shfmt (tools/doctor.sh)"))?;
    // Written from a thread of its own, so a script larger than the pipe's buffer cannot hold shfmt and
    // this command each waiting on the other.
    let writer = child.stdin.take().map(|mut stdin| {
        let text = text.to_string();
        std::thread::spawn(move || stdin.write_all(text.as_bytes()))
    });
    let output = child.wait_with_output().map_err(|error| format!("shfmt did not finish: {error}"))?;
    if let Some(writer) = writer {
        writer
            .join()
            .map_err(|_| "the script could not be handed to shfmt".to_string())?
            .map_err(|error| format!("the script could not be handed to shfmt: {error}"))?;
    }
    if !output.status.success() {
        return Err(format!("shfmt cannot read it: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("shfmt's syntax tree does not read as JSON: {error}"))
}

/// Every problem of these scripts, as lines the stage prints, and how many run under `pipefail`. The
/// stage's run and the tests both come through here, so the rule is read in exactly one place.
fn check(scripts: &[Script]) -> (Vec<String>, usize) {
    let mut problems = Vec::new();
    let mut settings = Vec::new();
    for script in scripts {
        let (reading, unseen) = settings_of(&script.tree);
        for line in unseen {
            problems.push(format!("early-pipes: {}:{line}", script.name));
        }
        settings.push(reading);
    }
    let under = stands_in(scripts, &settings, settings.iter().map(|reading| reading.pipefail).collect());
    let grep_env = stands_in(scripts, &settings, settings.iter().map(|reading| reading.grep_env).collect());
    for (script, (on, grep_env)) in scripts.iter().zip(under.iter().zip(&grep_env)) {
        if !on {
            continue;
        }
        let mut found = Vec::new();
        pipelines(&script.tree, &script.text, &mut found);
        for stages in found {
            for stage in stages.iter().skip(1) {
                if let Some(verdict) = judge(stage, *grep_env) {
                    problems.push(format!("early-pipes: {}:{}: {verdict}", script.name, line_of(stage)));
                }
            }
        }
    }
    (problems, under.iter().filter(|on| **on).count())
}

/// What a script's own commands say of `pipefail`: whether any of them turns it on, the names of the scripts
/// it sources, and whether it hands its greps a variable that changes what they read as their options.
struct Settings {
    pipefail: bool,
    sources: Vec<String>,
    /// Whether the script sets a variable that changes what its greps read as their options, so that no
    /// reading of a grep's words holds for it.
    grep_env: bool,
}

/// A script's settings, and a line (`<line>: <why>`) for each command that decides them but cannot be read.
fn settings_of(tree: &Value) -> (Settings, Vec<String>) {
    let mut calls = Vec::new();
    calls_in(tree, &mut calls);
    // The commands in the order they stand, so a refusal raised by one of them is read in line order; the
    // settings themselves do not depend on the order, since one `set` turning `pipefail` on is enough.
    calls.sort_by_key(|call| call.get("Pos").and_then(|pos| pos.get("Offset")).and_then(Value::as_u64).unwrap_or(0));
    let mut settings = Settings { pipefail: false, sources: Vec::new(), grep_env: sets_grep_env(tree) };
    let mut unseen = Vec::new();
    for call in calls {
        let words: Vec<&Value> = call.get("Args").and_then(Value::as_array).map(|args| args.iter().collect()).unwrap_or_default();
        let Some((first, rest)) = words.split_first() else { continue };
        match program(first).as_deref() {
            // One `set` or `shopt` turning `pipefail` on is enough for the whole script: no other command
            // turns it back off for the reading, since which pipes run while it is on is flow the tree does
            // not answer, and reading only up to the last one would pass the very pipe this rule is for.
            Some("set") => match pipefail_from(rest, false) {
                Some(Some(true)) => settings.pipefail = true,
                Some(Some(false)) | Some(None) => {}
                None => unseen.push(format!("{}: {SET_UNSEEN}", line_of(call))),
            },
            Some("shopt") => match pipefail_from(rest, true) {
                Some(Some(true)) => settings.pipefail = true,
                Some(Some(false)) | Some(None) => {}
                None => unseen.push(format!("{}: {SET_UNSEEN}", line_of(call))),
            },
            Some("source" | ".") => match rest.first().and_then(|file| program(file)) {
                Some(name) => settings.sources.push(name),
                None => unseen.push(format!("{}: {SOURCE_UNSEEN}", line_of(call))),
            },
            _ => {}
        }
    }
    (settings, unseen)
}

/// What one `set` (or, with `shopt`, one `shopt`) does to `pipefail`: `Some(Some(on))` when it turns it on
/// or off, `Some(None)` when it leaves it alone, and `None` when a word that could decide is not written out.
fn pipefail_from(words: &[&Value], shopt: bool) -> Option<Option<bool>> {
    let mut decided = None;
    let mut names_option = false;
    let mut turn = None;
    let mut index = 0;
    while let Some(word) = words.get(index) {
        let text = literal(word)?;
        index += 1;
        if text == "--" || text == "-" {
            break;
        }
        let Some(sign) = text.chars().next().filter(|c| *c == '-' || *c == '+') else {
            if names_option && text == "pipefail" {
                decided = turn;
            }
            continue;
        };
        let flags = text.get(1..).unwrap_or_default();
        if shopt {
            names_option |= flags.contains('o');
            turn = if flags.contains('s') {
                Some(true)
            } else if flags.contains('u') {
                Some(false)
            } else {
                turn
            };
            continue;
        }
        if flags.contains('o') {
            // The `o` of a bundle takes the next word as the name of the option it sets.
            let name = words.get(index).map(|word| literal(word));
            index += 1;
            match name {
                Some(Some(name)) if name == "pipefail" => decided = Some(sign == '-'),
                Some(None) => return None,
                _ => {}
            }
        }
    }
    Some(decided)
}

/// Whether a script sets one of [`GREP_ENV_VARS`] anywhere in it, in a command's own assignment or in an
/// `export`, `declare`, `local` or `readonly`: shfmt writes the name of each variable an assignment gives as
/// a `Name` of its own, which nothing else in the tree is, so no reading of the words is needed to find it.
/// Where in the script it stands is not weighed, an `unset` after it included: which of the script's greps
/// run while it is set is a question of the script's flow, which no syntax tree answers, so every grep the
/// script's pipes feed is refused rather than half of them read.
fn sets_grep_env(node: &Value) -> bool {
    match node {
        Value::Object(fields) => {
            let named = fields.get("Name").and_then(|name| name.get("Value")).and_then(Value::as_str).is_some_and(|name| GREP_ENV_VARS.contains(&name));
            named || fields.values().any(sets_grep_env)
        }
        Value::Array(items) => items.iter().any(sets_grep_env),
        _ => false,
    }
}

/// Which way a setting crosses a `source`: to the script sourced, whose commands run in the sourcing shell,
/// or to the scripts sourcing it, in whose shell its own setting stands from there on.
#[derive(Clone, Copy)]
enum Toward {
    Sourced,
    Sourcers,
}

/// The scripts a setting reaches from the ones marked, across `source` by name one way, to the fixpoint
/// where each script reached passes it on in turn. A name no listed script answers to is nobody's, and asks
/// nothing. A script already reached passes the setting on and is never weighed again, so each script is
/// marked at most once however the `source`s loop back: a script that sources itself, or two that source
/// each other, ends the same way a longer chain does. [`stands_in`] follows it both ways.
fn reached(scripts: &[Script], settings: &[Settings], mut marked: Vec<bool>, toward: Toward) -> Vec<bool> {
    let names: Vec<String> = scripts.iter().map(|script| base_name(&script.name)).collect();
    let sources =
        |by: usize, of: usize| settings.get(by).is_some_and(|reading| names.get(of).is_some_and(|name| reading.sources.iter().any(|sourced| sourced == name)));
    loop {
        let mut widened = false;
        for this in 0..scripts.len() {
            if marked.get(this).copied().unwrap_or(true) {
                continue;
            }
            let passed_on = (0..scripts.len()).any(|other| {
                other != this
                    && marked.get(other).copied().unwrap_or(false)
                    && match toward {
                        Toward::Sourced => sources(other, this),
                        Toward::Sourcers => sources(this, other),
                    }
            });
            if passed_on {
                if let Some(slot) = marked.get_mut(this) {
                    *slot = true;
                }
                widened = true;
            }
        }
        if !widened {
            return marked;
        }
    }
}

/// Which scripts a setting made in one of them stands in, however far it reaches across `source`. A script
/// runs what it sources in its own shell, and a sourced script runs in the shell of the script sourcing it,
/// so wherever in a shell a setting is made — in a script, in what it sources, or in a library reached
/// through any of them — it stands in that whole shell: in every script that sources one holding it, and in
/// everything those source in turn. Both steps are [`reached`], one up the `source`s and one back down them,
/// so two scripts that only source one library share what that library sets and nothing else. Where in a
/// script the setting stands is not weighed, a later turning of it off included: which of the script's
/// commands run while it holds is a question of its flow, which no syntax tree answers, so the whole script
/// is read under it rather than half of it read at all. `pipefail` and the grep environment
/// ([`GREP_ENV_VARS`]) are both read this way, and no other setting.
fn stands_in(scripts: &[Script], settings: &[Settings], made: Vec<bool>) -> Vec<bool> {
    let shells = reached(scripts, settings, made, Toward::Sourcers);
    reached(scripts, settings, shells, Toward::Sourced)
}

/// Every pipeline in a tree, as the statements of its stages in order. A pipeline is found once, at the
/// statement that holds the whole of it; what its stages hold (a substitution inside a word, a function
/// body, a condition) is searched for pipelines of its own.
fn pipelines<'a>(node: &'a Value, text: &str, found: &mut Vec<Vec<&'a Value>>) {
    match node {
        Value::Object(fields) => {
            if fields.get("Cmd").is_some_and(|cmd| is_pipe(cmd, text)) {
                let mut stages = Vec::new();
                stages_of(node, text, &mut stages);
                for stage in &stages {
                    if let Some(cmd) = stage.get("Cmd") {
                        pipelines(cmd, text, found);
                    }
                    for key in ["Redirs", "Comments"] {
                        if let Some(more) = stage.get(key) {
                            pipelines(more, text, found);
                        }
                    }
                }
                found.push(stages);
                return;
            }
            for value in fields.values() {
                pipelines(value, text, found);
            }
        }
        Value::Array(items) => {
            for item in items {
                pipelines(item, text, found);
            }
        }
        _ => {}
    }
}

/// The stages of the pipeline a statement holds, in order: shfmt nests `a | b | c` as `(a | b) | c`.
fn stages_of<'a>(statement: &'a Value, text: &str, stages: &mut Vec<&'a Value>) {
    match statement.get("Cmd") {
        Some(cmd) if is_pipe(cmd, text) => {
            for side in ["X", "Y"] {
                if let Some(part) = cmd.get(side) {
                    stages_of(part, text, stages);
                }
            }
        }
        _ => stages.push(statement),
    }
}

/// Whether a binary command is a pipe, `|` or `|&`, by the operator the parser found where its tree says it
/// stands in the script: shfmt's releases write the operator in the tree differently (3.8 as a number of its
/// own, 3.14 as its text), but every one of them names the operator's place.
fn is_pipe(cmd: &Value, text: &str) -> bool {
    if cmd.get("Type").and_then(Value::as_str) != Some("BinaryCmd") {
        return false;
    }
    let at = cmd.get("OpPos").and_then(|pos| pos.get("Offset")).and_then(Value::as_u64).and_then(|offset| usize::try_from(offset).ok());
    at.and_then(|at| text.get(at..)).is_some_and(|rest| rest.starts_with('|') && !rest.starts_with("||"))
}

/// Every simple command in a tree.
fn calls_in<'a>(node: &'a Value, calls: &mut Vec<&'a Value>) {
    match node {
        Value::Object(fields) => {
            if fields.get("Type").and_then(Value::as_str) == Some("CallExpr") {
                calls.push(node);
            }
            for value in fields.values() {
                calls_in(value, calls);
            }
        }
        Value::Array(items) => {
            for item in items {
                calls_in(item, calls);
            }
        }
        _ => {}
    }
}

/// What a pipe's reader is, when it is one the rule refuses: the stage's verdict, or `None` for a reader
/// that reads its input whole. A grep that runs with one of [`GREP_ENV_VARS`] set ([`stands_in`]) is
/// refused whole, by the name of the variable rather than of an option, since no reading of its words holds
/// under it.
fn judge(stage: &Value, grep_env: bool) -> Option<&'static str> {
    let Some(cmd) = stage.get("Cmd").filter(|cmd| cmd.get("Type").and_then(Value::as_str) == Some("CallExpr")) else {
        return Some(COMPOUND_READER);
    };
    let words: Vec<&Value> = cmd.get("Args").and_then(Value::as_array).map(|args| args.iter().collect()).unwrap_or_default();
    let mut index = 0;
    loop {
        let word = words.get(index)?;
        let Some(name) = program(word) else { return Some(NAMED_UNSEEN) };
        index += 1;
        if WRAPPERS.contains(&name.as_str()) {
            index = past_wrapper_options(&name, &words, index)?;
            continue;
        }
        let rest = words.get(index..).unwrap_or_default();
        return match name.as_str() {
            "grep" | "egrep" | "fgrep" if grep_env || sets_grep_env_in(words.get(..index).unwrap_or_default()) => Some(GREP_ENV),
            "grep" | "egrep" | "fgrep" => grep_quits(rest),
            "head" => Some(HEAD_SHAPE),
            "awk" | "gawk" | "mawk" | "nawk" => Some(AWK_SHAPE),
            _ => None,
        };
    }
}

/// Whether a command hands one of [`GREP_ENV_VARS`] to the program it runs, as `env GREP_OPTIONS=-q grep -q x`
/// does, which is the same change to what its grep reads as its options as a script setting it.
fn sets_grep_env_in(words: &[&Value]) -> bool {
    words.iter().filter_map(|word| literal(word)).any(|text| text.split_once('=').is_some_and(|(name, _)| GREP_ENV_VARS.contains(&name)))
}

/// Where the program a wrapper runs is named: past the wrapper's options (and `env`'s assignments), the
/// options that take a value taking theirs. `None` when an option there is not written out.
fn past_wrapper_options(wrapper: &str, words: &[&Value], mut index: usize) -> Option<usize> {
    while let Some(word) = words.get(index) {
        let Some(text) = literal(word) else { return Some(index) };
        let assignment = wrapper == "env" && text.split_once('=').is_some_and(|(name, _)| !name.is_empty() && !name.starts_with('-'));
        if text == "--" {
            return Some(index + 1);
        }
        if !text.starts_with('-') && !assignment {
            return Some(index);
        }
        let takes_value = matches!((wrapper, text.as_str()), ("env", "-u" | "-C" | "-P" | "-S") | ("nice", "-n"));
        index += if takes_value { 2 } else { 1 };
    }
    Some(index)
}

/// What one of a grep's options takes: nothing, a value (after `=` or in the next word), or a value only after `=`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Takes {
    Nothing,
    Value,
    Optional,
}

/// One long option of the greps the gate's scripts run, as each grep's own `getopt_long` table has it: what it
/// takes on FreeBSD's grep, which macOS ships as `/usr/bin/grep`, and on GNU's, which Ubuntu ships, `None`
/// where that grep has no such option; and whether it stops grep reading its input before the end. What each
/// takes is what that grep's `getopt_long` says of every abbreviation of every name, which the tests ask the
/// grep the machine runs; whether it stops was read off each grep's runs at a pipe's end, a writer handing it
/// more than a pipe's buffer holds, and with its output a file, a pipe and `/dev/null`, since GNU's reads to the
/// end for `-l` and `-L` only when its output is `/dev/null`.
struct GrepLong {
    name: &'static str,
    bsd: Option<Takes>,
    gnu: Option<Takes>,
    stops: bool,
}

const fn long(name: &'static str, bsd: Option<Takes>, gnu: Option<Takes>, stops: bool) -> GrepLong {
    GrepLong { name, bsd, gnu, stops }
}

const N: Option<Takes> = Some(Takes::Nothing);
const V: Option<Takes> = Some(Takes::Value);
const O: Option<Takes> = Some(Takes::Optional);

/// The one table of both greps' long options. Five of them stop where a value or a file decides it —
/// `--directories` where the value is `recurse`, `--color` and `--colour` where GNU reads a value it does
/// not know, `--binary-files` and `--devices` where either grep reads a word it does not know as the value
/// (GNU's knows each only in lower case), ending at once, `--file` where the file holds no pattern — and the
/// table reads no word given as a value, nor any file, so those five stand among the options that stop, the
/// values both greps read whole refused with the rest; so do FreeBSD's decompressing options but
/// `--decompress`, which end at the first bytes that are not of their format. Only a count is read as a
/// value ([`GREP_COUNTS`]).
const GREP_LONG: &[GrepLong] = &[
    long("after-context", V, V, false),
    long("basic-regexp", N, N, false),
    long("before-context", V, V, false),
    long("binary", N, N, false),
    long("binary-files", V, V, true),
    long("byte-offset", N, N, false),
    long("bz2decompress", N, None, true),
    long("color", O, O, true),
    long("colour", O, O, true),
    long("context", O, V, false),
    long("count", N, N, false),
    long("decompress", N, None, false),
    long("dereference-recursive", None, N, true),
    long("devices", V, V, true),
    long("directories", V, V, true),
    long("exclude", V, V, false),
    long("exclude-dir", V, V, false),
    long("exclude-from", None, V, false),
    long("extended-regexp", N, N, false),
    long("file", V, V, true),
    long("files-with-matches", N, N, true),
    long("files-without-match", N, N, true),
    long("fixed-regexp", None, N, false),
    long("fixed-strings", N, N, false),
    long("group-separator", None, V, false),
    long("help", N, N, true),
    long("ignore-case", N, N, false),
    long("include", V, V, false),
    long("include-dir", V, None, false),
    long("initial-tab", None, N, false),
    long("invert-match", N, N, false),
    long("label", V, V, false),
    long("line-buffered", N, N, false),
    long("line-number", N, N, false),
    long("line-regexp", N, N, false),
    long("lzma", N, None, true),
    long("max-count", V, V, true),
    long("mmap", N, None, false),
    long("no-filename", N, N, false),
    long("no-group-separator", None, N, false),
    long("no-ignore-case", None, N, false),
    long("no-messages", N, N, false),
    long("null", N, N, false),
    long("null-data", N, N, false),
    long("only-matching", N, N, false),
    long("perl-regexp", None, N, false),
    long("quiet", N, N, true),
    long("recursive", N, N, true),
    long("regexp", V, V, false),
    long("silent", N, N, true),
    long("text", N, N, false),
    long("unix-byte-offsets", N, N, false),
    long("version", N, N, true),
    long("with-filename", N, N, false),
    long("word-regexp", N, N, false),
    long("xz", N, None, true),
];

/// The names `getopt_long` reads as one option, since each grep's table gives them the same value: an
/// abbreviation that begins only names of one option is that option, where one that begins two options is
/// refused by both greps.
const GREP_ALIASES: &[(&str, &str)] = &[("colour", "color"), ("fixed-regexp", "fixed-strings"), ("silent", "quiet")];

/// One grep's short options: the letters it knows, those that take a value (in the rest of their word or the
/// next one), and those that stop its reading, `-D` among them for the reason `--devices`, its long name,
/// stands among the long ones that stop, and FreeBSD's `-I`, which ends at the first byte of its input that
/// is not text, as its `--binary-files=without-match` does, where GNU's reads its standard input to the end.
/// Both greps' `getopt_long` read an option after the pattern.
struct GrepShort {
    known: &'static str,
    valued: &'static str,
    stops: &'static str,
}

/// The options whose value both greps read as a count, and end at once on one they cannot read, each by its
/// short letter and its long name, since a script may write either: the context lines after, before and
/// around a match. `-m` and `--max-count` ask for a count too, and stop reading their input on their own
/// name, so the reading never weighs their value.
const GREP_COUNTS: &[(char, &str)] = &[('A', "after-context"), ('B', "before-context"), ('C', "context")];

const BSD_SHORT: GrepShort = GrepShort { known: "0123456789abcdefhilmnopqrsuvwxyzABCDEFGHIJLMORSUVXZ", valued: "ABCDdefm", stops: "dDIJlmMqrRVX" };

const GNU_SHORT: GrepShort = GrepShort { known: "0123456789abcdefhilmnoqrsuvwxyzABCDEFGHILPRTUVXZ", valued: "ABCDXdefm", stops: "dDflLmqrRV" };

/// The two greps a gate script may run under.
#[derive(Clone, Copy)]
enum Grep {
    Bsd,
    Gnu,
}

impl Grep {
    fn takes(self, option: &GrepLong) -> Option<Takes> {
        match self {
            Grep::Bsd => option.bsd,
            Grep::Gnu => option.gnu,
        }
    }

    fn short(self) -> &'static GrepShort {
        match self {
            Grep::Bsd => &BSD_SHORT,
            Grep::Gnu => &GNU_SHORT,
        }
    }
}

/// What a grep's `getopt_long` reads one long option name as.
enum Resolved {
    /// The option of that name, or the one option it begins: its name, what it takes, and whether it stops
    /// the reading.
    Option(&'static str, Takes, bool),
    /// No option of that grep begins with it.
    Unknown,
    /// It begins two options or more.
    Ambiguous,
}

/// What a grep's `getopt_long` says of the words it is given, one grep's reading of them.
#[derive(PartialEq, Eq)]
enum Reading {
    /// Reads the pipe from first byte to last.
    Whole,
    /// Stops before the end of what it is handed.
    Stops,
    /// Reads a file rather than the pipe at all, or a word that could name one.
    File,
    /// Has nothing to search for, so ends before reading anything.
    Nothing,
    /// A word that could be an option or a file is not written out.
    Unseen,
    /// A word neither grep reads as one option, so what follows it cannot be read.
    Unfollowed,
}

/// What one grep reads of a pipe's words: every word that may be an option read as its `getopt_long` reads
/// it, and every word after the options as an operand — the pattern, then the files. Both greps read an
/// option after the pattern, so `-e`, `-f` and their long names give the pattern wherever they stand, and
/// every operand after one of them is a file. Files with no `-`, the pipe itself, among them are read instead
/// of the pipe, which is the same failure as quitting early.
fn grep_reading(grep: Grep, words: &[&Value]) -> Reading {
    let short = grep.short();
    let mut index = 0;
    let mut operands: Vec<Option<String>> = Vec::new();
    let mut pattern_is_option = false;
    let mut ends_options = false;
    let mut valued_waits = false;
    while let Some(word) = words.get(index) {
        index += 1;
        let text = literal(word);
        if ends_options {
            operands.push(text);
            continue;
        }
        let Some(text) = text else {
            // A word that is not written out could be an option that quits, or one that names no option at
            // all, so nothing after it can be read.
            return Reading::Unseen;
        };
        if text == "--" {
            ends_options = true;
            continue;
        }
        if let Some(long) = text.strip_prefix("--") {
            let (name, value) = long.split_once('=').map_or((long, None), |(name, value)| (name, Some(value)));
            match resolve(grep, name) {
                Resolved::Unknown | Resolved::Ambiguous => return Reading::Unfollowed,
                Resolved::Option(.., true) => return Reading::Stops,
                Resolved::Option(_, Takes::Nothing, false) if value.is_some() => return Reading::Unfollowed,
                // A context or count written as no number either grep can read ends the grep at once, its
                // writer dying with the pipe unread, whichever word the value stands in.
                Resolved::Option(of, ..) if value.is_some_and(|given| needs_number(of) && !reads_as_number(given)) => return Reading::Nothing,
                Resolved::Option(of, Takes::Value, false) if value.is_none() => {
                    // The next word is the option's value, whatever word it is: both greps take one that
                    // begins with `-` as the option's own, and end at a value that is not there.
                    if next_is_no_number(needs_number(of), words, index) {
                        return Reading::Nothing;
                    }
                    pattern_is_option |= gives_pattern(of);
                    valued_waits = index >= words.len();
                    index += 1;
                }
                Resolved::Option(of, ..) => pattern_is_option |= gives_pattern(of),
            }
            continue;
        }
        let Some(flags) = text.strip_prefix('-').filter(|flags| !flags.is_empty()) else {
            operands.push(Some(text));
            continue;
        };
        for (at, flag) in flags.char_indices() {
            if !short.known.contains(flag) {
                return Reading::Unfollowed;
            }
            pattern_is_option |= flag == 'e' || flag == 'f';
            if short.stops.contains(flag) {
                return Reading::Stops;
            }
            if short.valued.contains(flag) {
                // The rest of the word is the option's value, or the next word is when nothing is left.
                let rest = flags.get(at + flag.len_utf8()..).unwrap_or_default();
                let count = GREP_COUNTS.iter().any(|(letter, _)| *letter == flag);
                if !rest.is_empty() {
                    if count && !reads_as_number(rest) {
                        return Reading::Nothing;
                    }
                } else if next_is_no_number(count, words, index) {
                    return Reading::Nothing;
                } else {
                    valued_waits = index >= words.len();
                    index += 1;
                }
                break;
            }
        }
    }
    if valued_waits {
        return Reading::Nothing;
    }
    // Every operand after the pattern is a file the grep reads in place of the pipe it stands at, and with
    // the pattern from an option every operand is one, unless `-`, the pipe itself, stands among them, when
    // the grep reads the pipe whole beside them. A path that names the pipe another way is no `-`: GNU's
    // grep, its output sent to `/dev/null`, stops at the first match of a `/dev/stdin` it opened itself,
    // where it reads its own standard input to the end. None left at all and the grep has no pattern.
    let files: &[Option<String>] = if pattern_is_option {
        operands.as_slice()
    } else if let Some((_, files)) = operands.split_first() {
        files
    } else {
        return Reading::Nothing;
    };
    if files.is_empty() || files.iter().any(|file| file.as_deref() == Some("-")) { Reading::Whole } else { Reading::File }
}

/// Whether a long option is one of the two that give the grep its pattern, `--regexp` and `--file`, from
/// whichever name the word began it with.
fn gives_pattern(option: &str) -> bool {
    option == "regexp" || option == "file"
}

/// Whether a long option is one whose value both greps read as a count ([`GREP_COUNTS`]).
fn needs_number(option: &str) -> bool {
    GREP_COUNTS.iter().any(|(_, name)| *name == option)
}

/// Whether a word is a number both greps read as the context or count they asked for: an optional `+`,
/// which both allow, then digits and nothing else. Both end at once on `5x`, on `1e2` and on the empty
/// word, and macOS's grep on a count past what it can count (`9223372036854775807`), so the digits stop at
/// eighteen, every count of which both read. A count of nineteen digits macOS's grep still reads, and a
/// word with a space before its digits, which both read as a number, are refused with them: no script
/// writes one, and a word the reading must guess at is refused anyway.
fn reads_as_number(value: &str) -> bool {
    let digits = value.strip_prefix('+').unwrap_or(value);
    !digits.is_empty() && digits.len() <= 18 && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether the word a valued option takes its value from is one it cannot read as the number it asked for.
/// A word that is not written out is taken as its value unread: a count held in a variable is how a script
/// passes one, and one that is no count makes the grep fail with its own error, which the pipe answers
/// whatever the timing.
fn next_is_no_number(needs: bool, words: &[&Value], at: usize) -> bool {
    needs && words.get(at).and_then(|word| literal(word)).is_some_and(|text| !reads_as_number(&text))
}

/// Whether a grep with these words stops reading its input before the end, on either grep a script may run.
fn grep_quits(words: &[&Value]) -> Option<&'static str> {
    let readings = [Grep::Bsd, Grep::Gnu].map(|grep| grep_reading(grep, words));
    if readings.contains(&Reading::Stops) {
        Some(GREP_SHAPE)
    } else if readings.contains(&Reading::File) {
        Some(GREP_FILE)
    } else if readings.contains(&Reading::Nothing) {
        Some(GREP_NOTHING)
    } else if readings.contains(&Reading::Unseen) {
        Some(GREP_UNSEEN)
    } else if readings.contains(&Reading::Unfollowed) {
        Some(GREP_UNFOLLOWED)
    } else {
        None
    }
}

/// The long option a grep reads a name as, as `getopt_long` resolves it: the option of that name, or else the
/// one option it begins, the names of one option counting as one ([`GREP_ALIASES`]: `--col` is `--color` and
/// `--colour` both).
fn resolve(grep: Grep, name: &str) -> Resolved {
    if name.is_empty() {
        return Resolved::Unknown;
    }
    let mine = || GREP_LONG.iter().filter_map(|option| grep.takes(option).map(|takes| (option.name, Resolved::Option(option.name, takes, option.stops))));
    if let Some((_, exact)) = mine().find(|(known, _)| *known == name) {
        return exact;
    }
    let one = |known: &'static str| GREP_ALIASES.iter().find(|(alias, _)| *alias == known).map_or(known, |(_, of)| *of);
    let mut begun = mine().filter(|(known, _)| known.starts_with(name));
    let Some((first, resolved)) = begun.next() else { return Resolved::Unknown };
    if begun.all(|(other, _)| one(other) == one(first)) { resolved } else { Resolved::Ambiguous }
}

/// A word's text when every part of it is written out: what the shell hands the command, quotes taken off
/// and no escape left for the shell to read ([`part_text`]).
fn literal(word: &Value) -> Option<String> {
    let mut text = String::new();
    for part in word.get("Parts").and_then(Value::as_array)? {
        text.push_str(&part_text(part)?);
    }
    Some(text)
}

/// What one part of a word hands the command, where this reading can vouch for it. The tree holds a part as
/// the script spells it, so a part whose text holds a backslash — an escape the shell takes off (`\grep`,
/// `\-V`), or in a `$'…'` one it translates (`$'\x2dq'`) — is none it vouches for, bar a plain `'…'`, whose
/// backslashes the shell hands on as they stand; such a word is refused like any other the reading cannot
/// see, whatever it would read as. shfmt writes no `Value` for an empty `''`, which is the empty word.
fn part_text(part: &Value) -> Option<String> {
    let kind = part.get("Type").and_then(Value::as_str)?;
    let text = match kind {
        "Lit" => part.get("Value").and_then(Value::as_str)?.to_string(),
        "SglQuoted" => part.get("Value").and_then(Value::as_str).unwrap_or_default().to_string(),
        "DblQuoted" => {
            let mut text = String::new();
            for inner in part.get("Parts").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
                if inner.get("Type").and_then(Value::as_str) != Some("Lit") {
                    return None;
                }
                text.push_str(inner.get("Value").and_then(Value::as_str)?);
            }
            text
        }
        _ => return None,
    };
    let plain_single_quotes = kind == "SglQuoted" && part.get("Dollar").and_then(Value::as_bool) != Some(true);
    (plain_single_quotes || !text.contains('\\')).then_some(text)
}

/// The program a command word names, or the file a `source` names: the last path part of the word when it
/// is written out, or of its written tail when that holds a `/` (`"$dir/bin/xtask"`, `"$(dirname "$0")/env.sh"`).
fn program(word: &Value) -> Option<String> {
    if let Some(text) = literal(word) {
        return Some(base_name(&text));
    }
    let last = word.get("Parts").and_then(Value::as_array)?.last()?;
    let tail = match last.get("Type").and_then(Value::as_str)? {
        "DblQuoted" => part_text(last.get("Parts").and_then(Value::as_array)?.last()?)?,
        _ => part_text(last)?,
    };
    tail.contains('/').then(|| base_name(&tail)).filter(|name| !name.is_empty())
}

fn base_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn line_of(node: &Value) -> u64 {
    node.get("Pos").and_then(|pos| pos.get("Line")).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(name: &str, text: &str) -> Script {
        Script { name: name.to_string(), text: text.to_string(), tree: parse(name, text).unwrap() }
    }

    fn one(text: &str) -> Vec<String> {
        check(&[script("s.sh", text)]).0
    }

    fn under(text: &str) -> usize {
        one(&format!("set -o pipefail\n{text}\n")).len()
    }

    /// Every message is pinned whole, wording and all: a rewording must be a decision, not drift. Each
    /// shape is planted here, because a message that never fires proves nothing.
    #[test]
    fn every_message_is_pinned_to_its_exact_words() {
        let cases = [
            ("set -euo pipefail\nlisted | grep -Fxq -- \"$test\"\n", format!("early-pipes: s.sh:2: {GREP_SHAPE}")),
            ("set -o pipefail\nlisted | grep x pats\n", format!("early-pipes: s.sh:2: {GREP_FILE}")),
            ("set -o pipefail\nlisted | grep pat -e\n", format!("early-pipes: s.sh:2: {GREP_NOTHING}")),
            ("set -euo pipefail\nfind tree | head -n 1\n", format!("early-pipes: s.sh:2: {HEAD_SHAPE}")),
            ("set -o pipefail\nvtool b | awk '$1 == \"minos\" { print $2 }'\n", format!("early-pipes: s.sh:2: {AWK_SHAPE}")),
            ("set -o pipefail\nlisted | grep $flags pat\n", format!("early-pipes: s.sh:2: {GREP_UNSEEN}")),
            ("set -o pipefail\nlisted | grep --fi pat\n", format!("early-pipes: s.sh:2: {GREP_UNFOLLOWED}")),
            ("set -euo pipefail\nexport GREP_OPTIONS=-q\nlisted | grep -c pat\n", format!("early-pipes: s.sh:3: {GREP_ENV}")),
            ("set -o pipefail\nlisted | \"$reader\" -n 1\n", format!("early-pipes: s.sh:2: {NAMED_UNSEEN}")),
            ("set -o pipefail\nlisted | while read -r line; do :; done\n", format!("early-pipes: s.sh:2: {COMPOUND_READER}")),
            ("set -o pipefail\nset $options\n", format!("early-pipes: s.sh:2: {SET_UNSEEN}")),
            ("set -o pipefail\nset -o \"$option\"\n", format!("early-pipes: s.sh:2: {SET_UNSEEN}")),
            ("set -o pipefail\nshopt -s -o \"$option\"\n", format!("early-pipes: s.sh:2: {SET_UNSEEN}")),
            ("set -o pipefail\nsource \"$library\"\n", format!("early-pipes: s.sh:2: {SOURCE_UNSEEN}")),
        ];
        for (text, message) in cases {
            assert_eq!(one(text), [message], "{text}");
        }
        assert_eq!(
            [
                GREP_SHAPE,
                GREP_FILE,
                GREP_NOTHING,
                HEAD_SHAPE,
                AWK_SHAPE,
                GREP_UNSEEN,
                GREP_UNFOLLOWED,
                GREP_ENV,
                NAMED_UNSEEN,
                COMPOUND_READER,
                SET_UNSEEN,
                SOURCE_UNSEEN
            ],
            [
                "a pipe feeds a grep that stops reading its input before the end, on macOS's grep or on GNU's (-q, -m, -l, -L, -r, -R, -d, -D, -f, -V, macOS's -I, -J, -M and -X, or their long names, whole or abbreviated, --help, and the options that stop on their value or their file: --binary-files, --color, --devices, --directories, --file): under pipefail the writer dying of SIGPIPE makes the pipe answer 141 where the reader's answer belongs, so the verdict is timing; let the grep read the writer's whole output and send its own away, or judge the writer itself",
                "a pipe feeds a grep given a file to read, or a word after its pattern that is not written out and could name one: the grep reads that file and never the pipe, whose writer dies of SIGPIPE, so the pipe answers with the file's verdict or with 141 where the file is not there, and never with what the pipe holds: give the pattern with `-e`, name no file, or `-` for the pipe itself",
                "a pipe feeds a grep left with nothing to search for, no word left to be its pattern, an option given no value, or a context or count written as no number the grep can read: the grep ends at once and reads none of the pipe, whose writer dies of SIGPIPE, so the pipe answers with the grep's error and not with anything the pipe holds: name the pattern, and a value every option that takes one can read",
                "a pipe feeds head, which quits after the lines it prints: under pipefail the writer dying of SIGPIPE makes the pipe answer 141 where the reader's answer belongs, so the verdict is timing; capture the writer's whole output and keep its first lines in the shell, or judge the writer itself",
                "a pipe feeds an awk, whose program can exit before its input ends and which no tool the gate runs reads: under pipefail the writer dying of SIGPIPE would make the pipe answer 141 by timing; capture the writer's whole output and hand it to awk as a here-string or a file",
                "a pipe feeds a grep with a word before `--` that is not written out, which could be an option that quits before its input ends: end its options with `--`",
                "a pipe feeds a grep with an option that macOS's grep or GNU's reads as none of its own, as more than one, or with a value it takes none of, so what follows it cannot be read: write the option out whole, as both greps know it",
                "a pipe feeds a grep in a script whose shell sets GREP_OPTIONS or POSIXLY_CORRECT, however its `source`s run: either variable changes what the grep reads as its options from what its words say, so this reading of them holds for nothing: macOS's grep takes option words out of GREP_OPTIONS, and POSIXLY_CORRECT makes either grep read a word after the pattern as a file: unset it, as tools/env.sh does for every script this rule reads",
                "a pipe feeds a command named by an expansion or through an escape, which this reading cannot tell from a reader that quits before its input ends: name its program as a plain word",
                "a pipe feeds a compound command, which this reading cannot tell from a reader that quits before its input ends: hand it the writer's output with `< <( … )`, or capture the output first",
                "a `set` or `shopt` with an option that is not written out, so this reading cannot tell whether it turns pipefail on or off",
                "a `source` or `.` whose file is named by an expansion with no file name written at its end, so this reading cannot tell which script it brings under pipefail"
            ]
        );
    }

    /// Every spelling of a grep that stops reading early is the shape — bundled or long, whole or abbreviated,
    /// on either grep, through a wrapper — and reading the whole output is not. Where the two greps part —
    /// FreeBSD's `--context` takes its value only after `=`, some options only one of them has, and some stop
    /// on one alone — the words were read off each at a pipe's end, FreeBSD's grep as macOS ships it and
    /// GNU's as Ubuntu does, and a word is refused where either stops.
    #[test]
    fn grep_stops_where_either_grep_reads_its_options_so() {
        for planted in [
            "grep -q pat",
            "grep -Fxq pat",
            "grep --quiet pat",
            "grep --silent pat",
            "egrep -iq pat",
            "fgrep -sq pat",
            "grep -m 1 pat",
            "grep -m1 pat",
            "grep --max-count=1 pat",
            "grep -l pat",
            "grep --qui pat",
            "grep --max 1 pat",
            "grep --max-count 1 pat",
            "grep --files-with-matches pat",
            "grep pat -q",
            "grep --context -q pat",
            "grep --binary -q pat",
            "grep -r pat",
            "grep --recursive pat",
            "grep --dereference-recursive pat",
            "grep -d recurse pat",
            "grep -L pat",
            "grep pat -L",
            "grep --files-without-match pat",
            "grep --files-without pat",
            "grep -f pats pat",
            "grep -f empty",
            "grep -q -f empty",
            "grep --file pats pat",
            "grep --file=empty pat",
            "grep --color=always pat",
            "grep --colour=never pat",
            "grep --col always pat",
            "grep --binary-files=bogus pat",
            "grep pat --binary-files bogus",
            "grep --binary-f bogus pat",
            "grep -D bogus pat",
            "grep -Dbogus pat",
            "grep pat -D READ",
            "grep --devices=bogus pat",
            "grep --dev bogus pat",
            // Values both greps read whole are refused with the rest: the table reads no word given as a
            // value, so these stand among the options that stop, as `--color` and `--directories` do. `-I`
            // reads text whole on both, and is refused for macOS's grep ending at the first byte that is not.
            "grep --binary-files=text pat",
            "grep -D read pat",
            "grep -D skip pat",
            "grep --devices=skip pat",
            "grep -d skip pat",
            "grep -I pat",
            "grep --lzma pat",
            "grep -M pat",
            "grep --bz pat",
            "grep --bz2decompress pat",
            "grep -J pat",
            "grep --xz pat",
            "grep -X pat",
            "grep pat -X",
            "grep --help",
            "grep -V",
            "/usr/bin/grep -q pat",
            "env LC_ALL=C grep -q pat",
            "command grep -q pat",
        ] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {planted}\n")), [format!("early-pipes: s.sh:2: {GREP_SHAPE}")], "{planted} stops");
        }
        for unfollowed in [
            "grep --fi pat",
            "grep --f pat",
            "grep --initial-tab pat",
            "grep --mmap pat",
            "grep -P pat",
            "grep --count=1 pat",
            "grep --files-with pat",
            "grep --li pat",
            "grep --line- pat",
            "grep --no- pat",
        ] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {unfollowed}\n")), [format!("early-pipes: s.sh:2: {GREP_UNFOLLOWED}")], "{unfollowed}");
        }
        for whole in [
            "grep -Fx pat >/dev/null",
            "grep -e q",
            "grep -e \"$pat\"",
            "grep --regexp pat",
            "grep -E pat",
            "grep -c pat",
            "grep -Fx -- \"$test\"",
            "grep --line-buffered pat",
            "grep --null pat",
            "grep --exclude -q pat",
            "grep --regexp=pat",
            "grep --with-filename pat",
            "grep --context=2 pat",
            "grep --fixed pat",
            "grep --unix-byte-offsets pat",
            "grep pat -c",
            "grep pat -A 5",
            "grep pat -A +5",
            "grep pat -C 0",
            "grep pat --context=5",
            "grep pat --context=+5",
            "grep pat -",
            "grep - -",
            "grep pat --",
            "grep -- -q",
            "grep -x -- pat -",
            "grep pat - pats",
            "grep pat pats -",
            "grep -e q - pats",
            "grep -c pat missing -",
            "grep -c ''",
            "grep -e ''",
            "grep pat --after-context=5",
            "grep pat --before-context 2",
            "grep pat --bef 3",
            "grep pat -A 999999999999999999",
        ] {
            assert_eq!(under(&format!("listed | {whole}")), 0, "{whole} reads whole");
        }
        // A file operand is read instead of the pipe, wherever it stands. The first operand is the pattern,
        // unless `-e`, `-f` or their long names gave it — and they may stand after the pattern, which both
        // greps read — so from there on every operand is a file. `-` is the pipe itself, so it is none, and
        // among the files it has the grep read the pipe whole beside them; `/dev/stdin` is no `-`, since
        // GNU's grep, its output sent to `/dev/null`, stops at the first match of a `/dev/stdin` it opens.
        for file in [
            "grep pat pats",
            "grep pat -- -q",
            "grep -e q pat",
            "grep -eq pat",
            "grep pat -c pats",
            "grep pat --context x",
            "grep -e q -- pats",
            "grep pat -- \"$file\"",
            "grep pat pats >/dev/null",
            "grep x pats -c",
            "egrep pat pats",
            "fgrep pat pats",
            "/usr/bin/grep pat pats",
            "command grep pat pats",
            "grep pat /dev/stdin",
            "grep -c pat ''",
        ] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {file}\n")), [format!("early-pipes: s.sh:2: {GREP_FILE}")], "{file} reads a file");
        }
        // Nothing to search for, because no word is left to be the pattern or an option is given no value:
        // the grep ends before reading any of the pipe, on either grep, which is its writer dying the same.
        for nothing in [
            "grep",
            "grep -e",
            "grep pat -e",
            "grep -B pat",
            "grep --context",
            "grep --regexp",
            "grep pat -A x",
            "grep pat -Ax",
            "grep pat -C x -B 2",
            "grep pat -A 5x",
            "grep pat --context=x",
            "grep pat --contex=x",
            "grep pat -A ''",
            "grep pat --after-context=x",
            "grep pat --after-context x",
            "grep pat --before-context=x",
            "grep pat --before-context y",
            "grep pat --after=x",
            "grep pat --aft x",
            "grep pat --before x",
            // Counts both greps read, refused with the counts they cannot: a space before the digits, and a
            // count of nineteen digits macOS's grep still reads, past which the reading does not go.
            "grep pat -A ' 5'",
            "grep pat -A 1000000000000000000",
        ] {
            assert_eq!(
                one(&format!("set -o pipefail\nlisted | {nothing}\n")),
                [format!("early-pipes: s.sh:2: {GREP_NOTHING}")],
                "{nothing} searches for nothing"
            );
        }
        // A word the tree holds with a backslash still in it is not one this reading vouches for: the shell
        // takes an escape off (`\-V` is `-V`, `$'\x2dq'` is `-q`), and inside double quotes off some
        // characters and not others, so each is refused unread, as any word not written out is. A plain
        // single-quoted backslash is handed on as it stands.
        for escaped in ["grep \\-V", "grep $'\\x2dV'", "grep \\-q pat", "grep $'\\x2dq' pat", "grep -c \"\\-\" pat"] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {escaped}\n")), [format!("early-pipes: s.sh:2: {GREP_UNSEEN}")], "{escaped}");
        }
        assert_eq!(under("listed | grep -c '\\-'"), 0, "a plain single-quoted backslash is handed on as it stands");
        assert_eq!(under("grep -q pat file"), 0, "a grep no pipe feeds stops no writer");
    }

    /// A grep a pipe feeds in a script that sets one of the variables changing what its grep reads as its
    /// options is refused whole, however its own words read: under `GREP_OPTIONS` macOS's grep quits at a
    /// word the script never wrote, and under `POSIXLY_CORRECT` either grep takes the word after the
    /// pattern as a file. The very same words, with the variable gone, are read as the rule reads them.
    #[test]
    fn a_script_that_sets_the_grep_environment_is_refused_at_its_greps() {
        for planted in
            ["export GREP_OPTIONS=-q", "GREP_OPTIONS=-q", "export POSIXLY_CORRECT=1", "POSIXLY_CORRECT=", "declare -x GREP_OPTIONS=-q", "local GREP_OPTIONS"]
        {
            assert_eq!(one(&format!("set -euo pipefail\n{planted}\nlisted | grep pat -c\n")), [format!("early-pipes: s.sh:3: {GREP_ENV}")], "{planted}");
        }
        // The setting can stand anywhere in the script, an `unset` after it too, and every grep is refused,
        // those whose words both greps read alike under it among them: which greps run while it is set is
        // the script's flow, which the tree does not hold.
        for (text, line) in [
            ("set -euo pipefail\nlisted | grep pat -c\nexport POSIXLY_CORRECT=1\n", 2),
            ("set -euo pipefail\nexport GREP_OPTIONS=-q\nunset GREP_OPTIONS\nlisted | grep pat -c\n", 4),
            ("set -euo pipefail\nexport POSIXLY_CORRECT=1\nlisted | grep -c pat\n", 3),
        ] {
            assert_eq!(one(text), [format!("early-pipes: s.sh:{line}: {GREP_ENV}")], "{text}");
        }
        // Handing it to the command is the same change made by `env` rather than by the shell.
        assert_eq!(one("set -o pipefail\nlisted | env POSIXLY_CORRECT=1 grep pat -c\n"), [format!("early-pipes: s.sh:2: {GREP_ENV}")]);
        // Naming the variable is no setting of it, and no reader but a grep asks about it.
        assert_eq!(one("set -o pipefail\necho \"$GREP_OPTIONS\"\nunset POSIXLY_CORRECT\nlisted | grep pat -c\n"), Vec::<String>::new());
        assert_eq!(one("set -o pipefail\nexport GREP_OPTIONS=-q\nlisted | cat\n"), Vec::<String>::new());
    }

    /// A setting crosses `source` both ways: a library that sets the variable leaves it set in every shell
    /// that sources it, and a script that sets it runs every library it sources with it set. Two scripts
    /// that only source the same library share what it sets, and nothing either of them sets itself.
    #[test]
    fn the_grep_environment_crosses_source_both_ways() {
        let library = |sets: &str| script("tools/lib.sh", &format!("# shellcheck shell=bash\n{sets}\nlib_greps() {{ listed | grep pat -c; }}\n"));
        let sourcer = |sets: &str| script("tools/user.sh", &format!("set -euo pipefail\n{sets}\nsource \"$(dirname \"$0\")/lib.sh\"\nlisted | grep pat -c\n"));
        let sibling = script("tools/sibling.sh", "set -euo pipefail\nsource \"$(dirname \"$0\")/lib.sh\"\nlisted | grep pat -c\n");
        let (from_library, _) = check(&[library("export GREP_OPTIONS=-q"), sourcer(":"), sibling]);
        assert_eq!(
            from_library,
            [
                format!("early-pipes: tools/lib.sh:3: {GREP_ENV}"),
                format!("early-pipes: tools/user.sh:4: {GREP_ENV}"),
                format!("early-pipes: tools/sibling.sh:3: {GREP_ENV}")
            ]
        );
        let sibling = script("tools/sibling.sh", "set -euo pipefail\nsource \"$(dirname \"$0\")/lib.sh\"\nlisted | grep pat -c\n");
        let (from_sourcer, _) = check(&[library(":"), sourcer("export POSIXLY_CORRECT=1"), sibling]);
        assert_eq!(from_sourcer, [format!("early-pipes: tools/lib.sh:3: {GREP_ENV}"), format!("early-pipes: tools/user.sh:4: {GREP_ENV}")]);
    }

    /// The rule reads a grep's words under none of [`GREP_ENV_VARS`], which is what the gate's own
    /// environment stands for: `tools/env.sh` takes each out of the environment with an `unset` of its own
    /// top level, which runs whenever it is sourced, once, and every other script the rule reads sources it
    /// at its own top level before its first pipe. Both are read through shfmt's tree, as the rule reads
    /// them, since the rule's answer is only as good as what it assumes.
    #[test]
    fn the_gate_clears_what_the_grep_reading_assumes() {
        let root = crate::repo_root();
        let listed = crate::files::listed(&root, "shell").unwrap();
        let read: Vec<Script> = listed.iter().map(|name| script(name, &std::fs::read_to_string(root.join(name)).unwrap())).collect();
        // Each of a script's own top-level commands: where it starts, and its words, the program's first.
        fn top_level(script: &Script) -> Vec<(u64, Vec<&Value>)> {
            script.tree["Stmts"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|statement| statement["Cmd"]["Type"] == "CallExpr")
                .map(|statement| {
                    (statement["Pos"]["Offset"].as_u64().unwrap(), statement["Cmd"]["Args"].as_array().map(|words| words.iter().collect()).unwrap_or_default())
                })
                .collect()
        }
        let named = |words: &[&Value], names: &[&str]| words.first().and_then(|word| program(word)).is_some_and(|name| names.contains(&name.as_str()));
        let env = read.iter().find(|script| script.name == "tools/env.sh").expect("tools/env.sh is among the scripts the rule reads");
        let unsets: Vec<Vec<String>> = top_level(env)
            .into_iter()
            .filter(|(_, words)| named(words, &["unset"]))
            .map(|(_, words)| words.iter().map(|word| literal(word).unwrap_or_default()).collect())
            .collect();
        for variable in GREP_ENV_VARS {
            assert_eq!(
                unsets.iter().filter(|words| words.contains(&variable.to_string())).count(),
                1,
                "tools/env.sh must unset {variable} once, at its own top level"
            );
        }
        for script in read.iter().filter(|script| script.name != "tools/env.sh") {
            let sourced_at = top_level(script)
                .into_iter()
                .find(|(_, words)| named(words, &["source", "."]) && words.get(1).and_then(|word| program(word)).as_deref() == Some("env.sh"))
                .map(|(at, _)| at);
            let mut found = Vec::new();
            pipelines(&script.tree, &script.text, &mut found);
            let first_pipe = found.iter().filter_map(|stages| stages.first()).map(|stage| stage["Pos"]["Offset"].as_u64().unwrap()).min();
            assert!(
                sourced_at.is_some_and(|at| first_pipe.is_none_or(|pipe| at < pipe)),
                "{} must source tools/env.sh at its own top level before its first pipe",
                script.name
            );
        }
    }

    /// A `head` a pipe feeds quits early, at the end or in the middle; a `tail` reads to the end.
    #[test]
    fn head_quits_wherever_a_pipe_feeds_it() {
        assert_eq!(under("java -version 2>&1 | sed -n s/x/y/p | head -n 1"), 1);
        assert_eq!(under("find tree | head -n 5 | sort"), 1, "a reader in the middle kills its writer too");
        assert_eq!(under("sed -n 's/x/y/p' log | tail -n 1"), 0);
    }

    /// An awk a pipe feeds is refused whatever its program, since only awk could say whether it exits;
    /// one handed its input whole as a here-string is no pipe's reader.
    #[test]
    fn an_awk_a_pipe_feeds_is_refused_whatever_its_program() {
        for program in ["'{ print; exit }'", "'$2 == test'", "-F'\\t' '$2 == \"*\" { found = 1 } END { exit !found }'", "-f early.awk"] {
            assert_eq!(under(&format!("rows | awk {program}")), 1, "{program}");
        }
        assert_eq!(under("rows | gawk '{ print }'"), 1);
        assert_eq!(under("awk '{ print; exit }' <<<\"$rows\""), 0);
    }

    /// Quotes, comments and `||` hold no pipes; a pipe inside a command or process substitution, a
    /// function, a condition, a negation or a background command is one the rule answers for.
    #[test]
    fn pipes_are_read_wherever_the_tree_holds_them() {
        assert_eq!(under("echo 'a | head -n 1'\necho \"b | grep -q c\"\n# x | head -n 1\nj | jq 'select(.x) | .y'\nfalse || head -n 2 file"), 0);
        for planted in [
            "readelf=\"$(find tree | head -n 1)\"",
            "while read -r x; do :; done < <(find tree | head -n 1)",
            "f() { listed | grep -q x; }",
            "if listed | grep -q x; then :; fi",
            "if ! listed | grep -q x; then :; fi",
            "FLAG=1 listed | head -n 1",
            "listed | head -n 1 &",
            "listed |& grep -q x",
            "[ \"$(a | head -n 1)\" = b ]",
            "listed | grep -c -- \"$(find tree | head -n 1)\"",
            "printf '%s\\n' \"$(a | head -n 1)\" | sort",
        ] {
            assert_eq!(under(planted), 1, "{planted}");
        }
    }

    /// A pipe continued over a line break, after `|` or after `\`, is one pipe, and the message names the
    /// reader's own line, not the writer's.
    #[test]
    fn a_pipe_continues_onto_the_next_line_and_names_its_reader() {
        for (text, line) in [("listed \\\n  | grep -q x", 3), ("listed |\n  head -n 1", 3), ("vtool b |\n  awk '{ print }'", 3)] {
            assert!(one(&format!("set -o pipefail\n{text}\n")).first().is_some_and(|found| found.starts_with(&format!("early-pipes: s.sh:{line}:"))), "{text}");
        }
    }

    /// A reader the tree holds no program word for is refused; one named by a written tail holding a `/`
    /// is read by that tail.
    #[test]
    fn a_reader_is_named_by_its_program_word() {
        assert_eq!(under("listed | \"$xtask_run/debug/xtask\" compiled"), 0);
        assert_eq!(under("listed | \"$tools/head\" -n 1"), 1);
        assert_eq!(under("listed | $reader"), 1);
        assert_eq!(under("listed | { read -r first; }"), 1);
        assert_eq!(under("listed | env -u X head -n 1"), 1);
        // An escaped program word is the program the shell runs once the escape is off, which this reading
        // does not vouch for, so it is refused whether that program would quit or read whole.
        for escaped in ["listed | \\grep -q x", "listed | \\grep -c x", "listed | \\head -n 1"] {
            assert_eq!(one(&format!("set -o pipefail\n{escaped}\n")), [format!("early-pipes: s.sh:2: {NAMED_UNSEEN}")], "{escaped}");
        }
    }

    /// Only scripts that run under `pipefail` answer for their pipes, in every spelling of turning it on
    /// and off: one `set` turning it on is enough, whatever a later one turns off, since where each pipe
    /// stands in the flow no syntax tree reads; a library sourced by one (the shape of `tools/env.sh`)
    /// answers too, through the sourcing script's `source` line, and so does what it sources in turn.
    #[test]
    fn pipefail_answers_for_its_script_and_for_what_it_sources() {
        for on in [
            "set -euo pipefail",
            "set -o pipefail",
            "set -e; set -o pipefail",
            "set +o pipefail; set -o pipefail",
            "shopt -s -o pipefail",
            "shopt -os pipefail",
            "set -o pipefail\nset +o pipefail",
            "set -o pipefail\nshopt -u -o pipefail",
        ] {
            assert_eq!(one(&format!("{on}\nlisted | grep -q x\n")).len(), 1, "{on}");
        }
        for off in ["", "set -e", "# set -o pipefail", "set -- -o pipefail", "set +o pipefail", "shopt -u -o pipefail"] {
            assert!(one(&format!("{off}\nlisted | grep -q x\n")).is_empty(), "{off}");
        }
        // A pipe between the setting and a later turning-off runs under `pipefail`, so a reading that took
        // the script's last `set` for the whole script would pass it.
        assert_eq!(one("set -euo pipefail\nif listed | grep -q x; then :; fi\nset +o pipefail\n"), [format!("early-pipes: s.sh:2: {GREP_SHAPE}")]);
        // Listed deepest first, so the library sourced by a library is reached only once the first is.
        let chain = [
            script("tools/more.sh", "# shellcheck shell=bash\nlisted | grep -q x\n"),
            script("tools/env.sh", "# shellcheck shell=bash\n. \"$CRAFTBRIDGE_ROOT/tools/more.sh\"\nmajor() { java -version 2>&1 | head -n 1; }\n"),
            script("tools/doctor.sh", "set -euo pipefail\n# shellcheck source=tools/env.sh\nsource \"$(dirname \"$0\")/env.sh\"\n"),
            script("tools/alone.sh", "listed | grep -q x\n"),
        ];
        let (problems, under) = check(&chain);
        assert_eq!(under, 3);
        assert_eq!(problems.len(), 2);
        assert!(problems.first().is_some_and(|line| line.starts_with("early-pipes: tools/more.sh:2:")), "{problems:?}");
        assert!(problems.get(1).is_some_and(|line| line.starts_with("early-pipes: tools/env.sh:3:")), "{problems:?}");
    }

    /// A library that turns `pipefail` on stands in every script that sources it, and in what they source in
    /// turn, the same closure the grep environment crosses; and a `source` that loops back on itself — a
    /// script sourcing itself, two sourcing each other — is read once and ends.
    #[test]
    fn pipefail_stands_in_the_scripts_that_source_it_and_a_cycle_ends() {
        let library = [
            script("tools/lib.sh", "set -o pipefail\n"),
            script("tools/caller.sh", "# shellcheck source=tools/lib.sh\nsource \"$(dirname \"$0\")/lib.sh\"\nlisted | grep -q x\n"),
        ];
        let (problems, under) = check(&library);
        assert_eq!(under, 2);
        assert_eq!(problems, [format!("early-pipes: tools/caller.sh:3: {GREP_SHAPE}")]);

        // A deeper way round: the setting reaches the sourcer's own sourcer, and the library it sources.
        let up = [
            script("tools/lib.sh", "set -o pipefail\n"),
            script(
                "tools/caller.sh",
                "# shellcheck source=tools/lib.sh\nsource \"$(dirname \"$0\")/lib.sh\"\n# shellcheck source=tools/more.sh\nsource \"$(dirname \"$0\")/more.sh\"\n",
            ),
            script("tools/more.sh", "listed | head -n 1\n"),
            script("tools/top.sh", "# shellcheck source=tools/caller.sh\nsource \"$(dirname \"$0\")/caller.sh\"\nlisted | grep -q x\n"),
            script("tools/alone.sh", "listed | grep -q x\n"),
        ];
        let (problems, under) = check(&up);
        assert_eq!(under, 4);
        assert_eq!(problems, [format!("early-pipes: tools/more.sh:1: {HEAD_SHAPE}"), format!("early-pipes: tools/top.sh:3: {GREP_SHAPE}")]);

        // A script sourcing itself, and a setting made in one of two scripts sourcing each other: each script
        // is marked at most once, so the walk ends however the `source`s loop back, with the setting standing
        // in the setters, in the cycle, in a third script sourcing the cycle, and in nothing else. The third
        // is listed before the cycle, so the setting climbs to it only on a second pass up the `source`s.
        let cycle = [
            script("tools/three.sh", "# shellcheck source=tools/two.sh\nsource \"$(dirname \"$0\")/two.sh\"\nlisted | grep -q x\n"),
            script("tools/self.sh", "set -o pipefail\n# shellcheck source=tools/self.sh\nsource \"$(dirname \"$0\")/self.sh\"\nlisted | grep -q x\n"),
            script("tools/two.sh", "# shellcheck source=tools/one.sh\nsource \"$(dirname \"$0\")/one.sh\"\nlisted | head -n 1\n"),
            script("tools/one.sh", "set -o pipefail\n# shellcheck source=tools/two.sh\nsource \"$(dirname \"$0\")/two.sh\"\n"),
            script("tools/aside.sh", "listed | grep -q x\n"),
        ];
        let (problems, under) = check(&cycle);
        assert_eq!(under, 4);
        assert_eq!(
            problems,
            [
                format!("early-pipes: tools/three.sh:3: {GREP_SHAPE}"),
                format!("early-pipes: tools/self.sh:4: {GREP_SHAPE}"),
                format!("early-pipes: tools/two.sh:3: {HEAD_SHAPE}")
            ]
        );
    }

    /// A `source` naming no script listed answers to nothing, and asks nothing: neither way of the closure
    /// carries a setting through a name no file of the release answers to.
    #[test]
    fn a_source_naming_no_script_of_the_list_carries_nothing() {
        let scripts = [script("tools/lib.sh", "set -o pipefail\n"), script("tools/caller.sh", "source \"$HOME/outside/gate.sh\"\nlisted | grep -q x\n")];
        let (problems, under) = check(&scripts);
        assert_eq!(under, 1);
        assert!(problems.is_empty(), "{problems:?}");
    }

    /// A script shfmt cannot parse is refused in shfmt's own words, never passed unread.
    #[test]
    fn a_script_shfmt_cannot_parse_is_refused() {
        let refused = parse("broken.sh", "set -o pipefail\nlisted |\n");
        assert!(refused.as_ref().is_err_and(|why| why.starts_with("shfmt cannot read it: broken.sh:")), "{refused:?}");
    }

    /// What a grep's `getopt_long` says of one option word, in its own diagnostics: both greps word them alike,
    /// bar FreeBSD's for an unknown letter, which it calls illegal where POSIX asks it to.
    #[derive(PartialEq, Eq, Debug)]
    enum Said {
        Unknown,
        Ambiguous,
        Takes(Takes),
    }

    /// What the grep on `PATH` prints for these words, in an empty folder with nothing to read: in the C
    /// locale, which words its diagnostics as [`said_of`] reads them, and with no variable that adds words or
    /// changes how they are read.
    fn grep_says(words: &[String], folder: &Path) -> std::process::Output {
        let mut grep = Command::new("grep");
        grep.args(words).current_dir(folder).env("LC_ALL", "C").stdin(Stdio::null());
        for variable in GREP_ENV_VARS {
            grep.env_remove(variable);
        }
        grep.output().expect("grep runs")
    }

    fn diagnostics(words: &[String], folder: &Path) -> String {
        String::from_utf8_lossy(&grep_says(words, folder).stderr).into_owned()
    }

    /// What the grep says of a word, alone or after the pattern, and, for a long option that takes no
    /// required value, of it with one after `=`.
    fn said_of(before: &[&str], word: &str, folder: &Path) -> Said {
        let alone = diagnostics(&before.iter().map(|word| word.to_string()).chain([word.to_string()]).collect::<Vec<_>>(), folder);
        if ["unrecognized option", "invalid option", "illegal option"].iter().any(|unknown| alone.contains(unknown)) {
            Said::Unknown
        } else if alone.contains("is ambiguous") {
            Said::Ambiguous
        } else if alone.contains("requires an argument") {
            Said::Takes(Takes::Value)
        } else if !word.starts_with("--") || diagnostics(&[format!("{word}=1")], folder).contains("doesn't allow an argument") {
            Said::Takes(Takes::Nothing)
        } else {
            Said::Takes(Takes::Optional)
        }
    }

    /// The table is held to the grep the machine runs, which the gate's scripts run here, by what its own
    /// `getopt_long` says of every word the reading takes for an option: every abbreviation of every long
    /// name either grep has, a long option of every letter no name begins with, every letter and digit as a
    /// short option, and an option after the pattern. A release that changes no option reads every word as
    /// before and passes, whatever its number; one that adds, drops or changes an option, and a grep the
    /// table was never read from, is named here by the word it reads otherwise. An option it adds where a
    /// name the table holds begins it, or after a letter that begins two, is no word the reading passes, so
    /// the reading refuses it unasked. Whether an option stops the reading shows in no diagnostic: the
    /// table's `stops` are each grep's runs at a pipe's end, read again when a release's notes name a changed one.
    #[test]
    fn the_grep_on_path_reads_every_option_word_as_the_table_does() {
        let folder = crate::scratch::Scratch::new("grep-options");
        let version = grep_says(&["--version".to_string()], &folder.0).stdout;
        let line = String::from_utf8_lossy(&version).lines().next().unwrap_or_default().to_string();
        let grep = if line.contains("(GNU grep)") {
            Grep::Gnu
        } else if line.contains("BSD grep") {
            Grep::Bsd
        } else {
            panic!("the grep on PATH is neither GNU's nor FreeBSD's, the two the table was read from: `{line}`");
        };
        let mut names: Vec<String> =
            GREP_LONG.iter().flat_map(|option| (1..=option.name.len()).filter_map(|end| option.name.get(..end).map(str::to_string))).collect();
        names.extend(('a'..='z').chain('0'..='9').map(String::from).filter(|letter| !GREP_LONG.iter().any(|option| option.name.starts_with(letter.as_str()))));
        names.sort();
        names.dedup();
        let mut misread = Vec::new();
        for name in &names {
            let table = match resolve(grep, name) {
                Resolved::Option(_, takes, _) => Said::Takes(takes),
                Resolved::Unknown => Said::Unknown,
                Resolved::Ambiguous => Said::Ambiguous,
            };
            let said = said_of(&[], &format!("--{name}"), &folder.0);
            if said != table {
                misread.push(format!("--{name}: the table reads {table:?}, the grep {said:?}"));
            }
        }
        let short = grep.short();
        for letter in ('a'..='z').chain('A'..='Z').chain('0'..='9') {
            let table = match (short.known.contains(letter), short.valued.contains(letter)) {
                (false, _) => Said::Unknown,
                (true, true) => Said::Takes(Takes::Value),
                (true, false) => Said::Takes(Takes::Nothing),
            };
            let said = said_of(&[], &format!("-{letter}"), &folder.0);
            if said != table {
                misread.push(format!("-{letter}: the table reads {table:?}, the grep {said:?}"));
            }
        }
        let after = said_of(&["pat"], "-e", &folder.0);
        if after != Said::Takes(Takes::Value) {
            misread.push(format!("pat -e: the reading takes an option after the pattern, the grep {after:?}"));
        }
        assert!(
            misread.is_empty(),
            "`{line}` reads these option words otherwise than the table: read `GREP_LONG`, `BSD_SHORT` and `GNU_SHORT` again against it, and what its changed options stop at a pipe's end:\n{}",
            misread.join("\n")
        );
    }

    /// The names of one option take the same and stop alike on each grep that has both, as one option does.
    #[test]
    fn the_names_of_one_option_read_alike() {
        for (alias, of) in GREP_ALIASES {
            let row = |name: &str| GREP_LONG.iter().find(|option| option.name == name).unwrap_or_else(|| panic!("{name} is a row of the table"));
            let (alias_row, of_row) = (row(alias), row(of));
            for grep in [Grep::Bsd, Grep::Gnu] {
                if let (Some(alias_takes), Some(of_takes)) = (grep.takes(alias_row), grep.takes(of_row)) {
                    assert_eq!((alias_takes, alias_row.stops), (of_takes, of_row.stops), "{alias} and {of}");
                }
            }
        }
    }

    /// The real tree, through the shell stage's own list: no early pipe anywhere, and every listed script
    /// either turns `pipefail` on itself or is the sourced library the rule still covers.
    #[test]
    fn the_tree_holds_no_early_pipe() {
        let root = crate::repo_root();
        let listed = crate::files::listed(&root, "shell").unwrap();
        assert!(!listed.is_empty());
        let read: Vec<Script> = listed.iter().map(|name| script(name, &std::fs::read_to_string(root.join(name)).unwrap())).collect();
        let (problems, under) = check(&read);
        assert_eq!(problems, Vec::<String>::new());
        assert_eq!(under, read.len());
    }
}
