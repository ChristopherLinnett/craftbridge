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
//! (`"$dir/bin/xtask"`), through `env`, `builtin`, `command`, `exec`, `nice` and `nohup`, which run the word after
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
//! - a `sed` whose script can reach a `q` or a `Q` and end sed before its input ends. The script is read
//!   where the tree holds the words that spell it — options, script and file operands alike — command by
//!   command as both seds read it, a bracket expression in a regexp holding its delimiter as both do, and an
//!   expansion inside the script stands as a hole the parser reads as an address that may match any line
//!   (`"s/^/$name: /"` and `"${served}s/^…/\1/p"` read whole), while a script taken from a file with `-f`
//!   or `--file`, held wholly in a word not written out, or spelled with an option or a command that macOS's
//!   sed or GNU's does not read, which ends that sed at once with its error, is refused unread rather than
//!   trusted. A `sed` given a file, or a word after its script that could name one, reads that file and no
//!   pipe at all — macOS's sed opens even `-` as a file named `-`, so no operand may stand where the pipe is
//!   read, and it ends its options at its first operand, so every word after a script given as one is such
//!   a file, an option word included. A `sed` whose scripts branch with `b` or `t` to a label none of them
//!   defines, or define one twice, as either sed reads its labels — GNU's to a blank or a `;`, macOS's to
//!   the line's end — ends at once with its error before reading any input; one sed reads the labels of all
//!   its scripts together;
//! - the `read` builtin, which takes one line and leaves the writer's rest unread, and a `mapfile` or
//!   `readarray` told with `-n` how many lines to copy, told with `-u` to read another descriptor rather
//!   than the pipe, or given any option this reading cannot read whole, which could be `-n` itself;
//! - an `awk`, `gawk`, `mawk` or `nawk`, whatever its program: whether it reaches an `exit` or a
//!   `nextfile` before its input ends is awk's to say, and no tool the gate runs reads awk's syntax, so an
//!   awk is handed its input whole another way (a here-string or a file) rather than read as text here;
//! - an `eval`, by the first command of the script its words spell, which is what reads the pipe, and a
//!   wrapper that runs no command at all (`command -v`), which reads none of it;
//! - a child shell — `bash`, `sh`, `dash`, `ksh` or `zsh`, named or through a wrapper — by the program its
//!   own words give it: a `-c` string, the first word past every option wherever the `-c` stands, is read
//!   through shfmt's tree exactly as an `eval`'s is and its first command is the reader, its options are
//!   read for `pipefail` through the one reading of a `set`'s, so `bash -o pipefail -c '…'` answers for the
//!   string's pipes the way a `set -o pipefail` answers for the script's; a child reading `-n` or `-D`
//!   parses what it is given and runs none of it, as a wrapper running nothing does; and a child whose
//!   program this reading cannot read whole — no `-c` program, one not written out, one shfmt cannot parse,
//!   an option this reading does not know or one held in a word whose first letter is not written out, a
//!   program read from a stream (no program or file at all, `/dev/stdin`), or, with `pipefail` on in its
//!   own words, no script file it can name — is refused: what runs below one
//!   shell's words is invisible to any reading on this side of them, and at a pipe's end the child with no
//!   program of its own reads the pipe itself.
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
//! is refused in every script, since what it does cannot be read. A `source` or `.` that reads its script
//! from the standard input — `/dev/stdin`, a `/dev/fd` or a `/proc/self/fd` name — is refused in every
//! script too: the stream carries no script of the list, and bash 3.2 was seen to take `pipefail` from
//! words run through `source /dev/stdin`. The same commands spelt through bash's own
//! stand-ins are read as the stand-in they are: `builtin set`, `command set`, `command -p set` and
//! `builtin shopt` stand in a `set` or a `shopt`, `builtin source` and `command .` a `source`, and the
//! options between them come off, as the one reading of every wrapper's options reads them, before the word
//! that stands — past `--`, while `-v` and `-V` print a name and run nothing, and bash reads an option its
//! stand-in has not got, `builtin -p` among them, as naming no command that runs. An `eval` runs the script
//! its words spell, so that script is read through shfmt's tree as this one's own commands, at the `eval`'s
//! line: `eval 'true; set -o pipefail'` turns `pipefail` on as bash does, an `eval` of a `source` sources,
//! and its pipes and variables count as the script's. An `eval` whose words are not all written out could
//! spell `set -o pipefail` itself, so its shell is read under `pipefail`, the reading's safe way, which is
//! how `tools/env.sh`'s `eval "$exports"` of `cargo ndk-env`'s output is read, in a shell under it already;
//! what does show of such an `eval` is read as far as it goes, so `eval "set -o $option"` is refused as the
//! `set` it shows. Last, a script under `pipefail` that exports `SHELLOPTS` is refused at the export: every
//! bash started below it, at any depth and by any command, inherits the running shell's own options,
//! measured in bash 3.2 and 5.2, and answers 141 by timing at its own early-quit pipes, unseen by any
//! reading of this script. The same hand crossing from one shell into the next in a command's own words is
//! refused at the hand in every script, whether the script runs under `pipefail` or not: a command given
//! `SHELLOPTS` in the environment it runs with (`env SHELLOPTS=… bash other.sh`) starts its shell with the
//! options named there, and which commands are handed one no reading can list either. Taking the export
//! attribute back is read as the taking-back it is and not as an export: `export -n SHELLOPTS` keeps the
//! value in this shell and takes it out of the environment every program below is handed, and `declare +x`
//! and `typeset +x` do the same (measured in bash 3.2, where bash keeps `SHELLOPTS` readonly and the
//! take-back succeeds all the same); `tools/env.sh` takes back an export the environment arrives with this
//! way, as it unsets the grep variables above. A child shell's own options are read where its program is:
//! `bash -o pipefail` turns the setting on for the script and marks the script file the child runs as a
//! `source` marks one, over-marking at worst — across a boundary no flow reading answers, the safe way is
//! to read the setting as made.

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
const NAMED_UNSEEN: &str = "a pipe feeds a command named by an expansion, through an escape, or past a wrapper option this reading does not follow — one that takes the command in its value, or one that macOS's wrapper or GNU's has not got — which this reading cannot tell from a reader that quits before its input ends: name its program as a plain word, after options both platforms' wrappers read";
const COMPOUND_READER: &str = "a pipe feeds a compound command, which this reading cannot tell from a reader that quits before its input ends: hand it the writer's output with `< <( … )`, or capture the output first";
const SET_UNSEEN: &str = "a `set` or `shopt` with an option that is not written out, so this reading cannot tell whether it turns pipefail on or off";
const SOURCE_UNSEEN: &str = "a `source` or `.` whose file is named by an expansion with no file name written at its end, so this reading cannot tell which script it brings under pipefail";
const SOURCE_STDIN: &str = "a `source` or `.` reads its script from the standard input — `/dev/stdin`, a `/dev/fd` or a `/proc/self/fd` name — which is no script of the list this reading reads: the words it runs stand on whatever stream carries them, able to set options, source another file or quit early at a pipe, and no reading here reaches them: source a script file that stands where this reading reads it";
const SED_SHAPE: &str = "a pipe feeds a sed whose script can reach `q` or `Q`, which end sed before its input ends: under pipefail the writer dying of SIGPIPE makes the pipe answer 141 where the reader's answer belongs, so the verdict is timing; capture the writer's whole output and let sed read it from a file or a here-string, or judge the writer itself";
const SED_FILE: &str = "a pipe feeds a sed given a file to read, or a word after its script that is not written out and could name one: the sed reads that file and never the pipe, whose writer dies of SIGPIPE, so the pipe answers with the file's verdict or with 141 where the file is not there, and never with what the pipe holds — macOS's sed reads even `-` as a file named `-` and not as the pipe: name no file after the script, or give the script with `-e`";
const SED_UNREAD: &str = "a pipe feeds a sed whose script this reading cannot read whole: read from a file with `-f` or `--file`, held wholly in a word that is not written out, or spelled with an option or a command that macOS's sed or GNU's does not read (macOS's reads no long option at all) or that edits files in place, so for all this reading can tell it reaches `q` or `Q`, or ends at once with its error, before its input ends: write the script out, and every option as both seds read it";
const SED_UNLABELED: &str = "a pipe feeds a sed whose scripts branch with `b` or `t` to a label none of that same sed's scripts defines, or define one label twice, as macOS's sed or GNU's reads its labels — GNU's ends a label at a blank or a `;`, macOS's runs it to the line's end: each sed looks every branch label up as it reads the scripts, and macOS's refuses a label defined twice, ending at once with its error before reading any input (measured), so the writer dies half-written and the pipe answers with the sed's error and never with what the pipe holds: define each label once, with a `:name` that ends its line, in one of the same sed's scripts — one sed reads the labels of all its scripts together — end each branch's line at its label, or branch to the end with a bare `b`";
const READ_SHAPE: &str = "a pipe feeds the `read` builtin, which takes one line and leaves the writer's rest unread: under pipefail the writer dying of SIGPIPE makes the pipe answer 141 where the reader's answer belongs, so the verdict is timing; capture the writer's whole output and let the shell read it from a here-string or a file";
const MAPFILE_SHAPE: &str = "a pipe feeds a mapfile or readarray told with `-n` how many lines to copy, leaving the writer's rest unread, or told with `-u` to read another descriptor and not the pipe, or given an option this reading cannot see whole, which could be `-n` itself: under pipefail the writer dying of SIGPIPE makes the pipe answer 141 where the reader's answer belongs, so the verdict is timing; capture the writer's whole output and hand it to mapfile as a here-string";
const SHELLOPTS_EXPORTED: &str = "a script that runs under pipefail exports SHELLOPTS, which hands the running shell's own options, pipefail among them, to every bash started below it — by name, by a script's `#!` line, as the `sh` macOS runs, or by any program the script starts — whose own pipes then answer 141 by timing where a reader quits early, unseen by any reading of this script: do not export SHELLOPTS";
const CHILD_UNREAD: &str = "a child shell (`bash`, `sh`, `dash`, `ksh` or `zsh`) was given a program this reading cannot read whole: no `-c` program at all, one held in a word that is not written out, one shfmt cannot parse, an option this reading does not know or one held in a word whose first letter is not written out, a program read from a stream — no program or file at all, which leaves the shell its standard input or a here-document, or `/dev/stdin` — a script file named by an expansion while the child runs with `pipefail` on, or a word naming one that stands behind a wrapper option this reading cannot follow, which may have eaten the word before it and named no command at all — such a shell runs words no reading of this script reaches, and at a pipe's end the shell with no program of its own reads the pipe itself, so its early-quit pipes answer 141 by timing unseen: write the `-c` program and every option out whole where shfmt can parse them, name the script file the shell runs with a plain word, and let no child stand behind an option only one platform's wrapper reads";
const CHILD_SHELLOPTS: &str = "a command is handed a SHELLOPTS assignment in the environment given it, which starts the shell it runs with the options named there, pipefail among them, unseen by any reading of this script or of the script that shell runs: hand no command a SHELLOPTS assignment; let each script turn on its own options, and take back an inherited export with `export -n SHELLOPTS`, which tools/env.sh does for every script this rule reads";
const EVAL_UNREAD: &str = "an `eval` whose text shfmt cannot parse, so this reading cannot tell what it runs: whether it turns pipefail on, sources a script, or feeds or reads a pipe that quits early";
const WRAPPER_RUNS_NOTHING: &str = "a pipe feeds a wrapper that runs no command and reads none of the pipe — `command -v` or `-V`, a `builtin` or `command` given an option it has not got, `env -0`, which runs a command on neither platform, `env --help` or `--version`, or a wrapper left with no command after its options — or a child shell reading `-n` or `-D`, which parses what it is given and runs none of it: its writer dies of SIGPIPE, so the pipe answers with what that prints or its error, by timing, and never with what the pipe holds: name the command to run, or let the child shell run its program";

/// The programs that run the program their words name after their own options. `builtin` runs one of the
/// shell's own builtins and `command` a builtin or a program, each passing over a function of that name, so
/// each stands in for the command that follows its options exactly as `env` and `exec` do.
const WRAPPERS: &[&str] = &["env", "builtin", "command", "exec", "nice", "nohup"];

/// The word an expansion inside an `eval`'s words stands as in the text shfmt parses: one parameter
/// expansion, which every reading here takes for a word not written out.
const EVAL_HOLE: &str = "${_}";

/// How deep `eval`s nested in `eval`s are read before the reading refuses one: each level spawns shfmt, and
/// no script nests more than a couple.
const EVAL_DEPTH: usize = 8;

/// The variables that change what a grep reads as its options, so that a script setting one cannot be read
/// by [`grep_reading`] at all: macOS's grep takes option words out of `GREP_OPTIONS`, and `POSIXLY_CORRECT`
/// makes either grep read a word after the pattern as a file rather than as an option. `tools/env.sh`, which
/// every script this rule reads sources before its first pipe, takes both out of the environment for the
/// reading to hold, and the tests hold it and every script to that.
const GREP_ENV_VARS: &[&str] = &["GREP_OPTIONS", "POSIXLY_CORRECT"];

/// The variables that hand a bash the options of the shell it was started below: a script that exports
/// `SHELLOPTS` while it runs under `pipefail` puts under `pipefail` every bash started from it, at any
/// depth — named `bash`, run by a script's `#!` line, the `sh` macOS runs, started by `xargs` or any other
/// program — measured in bash 3.2 and 5.2, so those shells' own early-quit pipes answer 141 by timing. Which
/// commands start a bash no reading of a script can list, so the export itself is what the rule refuses.
const SHELLOPTS_EXPORT: &[&str] = &["SHELLOPTS"];

/// The shells a script hands a whole program of its own: each runs its `-c` words as a script, runs the
/// script file a word names, and reads its standard input where no program is named — the pipe itself at a
/// pipe's end — so what such a shell runs stands below the boundary of this script's reading, save for what
/// its own words make readable here. Each also starts with a `SHELLOPTS` handed it in its environment,
/// which is why the hand is refused whole rather than read per shell.
const CHILD_SHELLS: &[&str] = &["bash", "sh", "dash", "ksh", "zsh"];

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
    let mut told = Vec::new();
    for script in scripts {
        let (reading, unseen) = settings_of(&script.tree);
        let mut here = Vec::new();
        for line in unseen {
            let problem = format!("early-pipes: {}:{line}", script.name);
            problems.push(problem.clone());
            here.push(problem);
        }
        told.push(here);
        settings.push(reading);
    }
    let under = stands_in(scripts, &settings, settings.iter().map(|reading| reading.pipefail).collect());
    let grep_env = stands_in(scripts, &settings, settings.iter().map(|reading| reading.grep_env).collect());
    for ((script, reading), ((on, grep_env), seen)) in scripts.iter().zip(&settings).zip(under.iter().zip(&grep_env).zip(&told)) {
        if !on {
            continue;
        }
        let mut exports = reading.shellopts.clone();
        exports.sort_unstable();
        exports.dedup();
        let mut found: Vec<(u64, String)> = exports.iter().map(|line| (*line, format!("early-pipes: {}:{line}: {SHELLOPTS_EXPORTED}", script.name))).collect();
        // The script's own pipes at their own lines, and those of what each `eval` or child shell's `-c`
        // runs at the line the command stands on.
        let texts = std::iter::once((&script.tree, script.text.as_str(), None))
            .chain(reading.evals.iter().map(|eval| (&eval.tree, eval.text.as_str(), Some(eval.line))));
        for (tree, text, at) in texts {
            let mut piped = Vec::new();
            pipelines(tree, text, &mut piped);
            for stage in piped.iter().flat_map(|stages| stages.iter().skip(1)) {
                if let Some(verdict) = judge(stage, *grep_env, 0) {
                    let line = at.unwrap_or_else(|| line_of(stage));
                    found.push((line, format!("early-pipes: {}:{line}: {verdict}", script.name)));
                }
            }
        }
        found.sort_by_key(|(line, _)| *line);
        // A command refused by both readings of it — an unreadable child at a pipe's end, unseen by the
        // settings and by the reader of the pipe alike — is one problem, told once.
        found.retain(|(_, problem)| !seen.contains(problem));
        found.dedup_by(|a, b| a.1 == b.1);
        problems.extend(found.into_iter().map(|(_, problem)| problem));
    }
    (problems, under.iter().filter(|on| **on).count())
}

/// What a script's own commands say of `pipefail`: whether any of them turns it on, the names of the scripts
/// it sources, whether it hands its greps a variable that changes what they read as their options, the lines
/// where it exports `SHELLOPTS`, and what each `eval` or child shell's `-c` spells.
struct Settings {
    pipefail: bool,
    sources: Vec<String>,
    /// Whether the script sets a variable that changes what its greps read as their options, so that no
    /// reading of a grep's words holds for it.
    grep_env: bool,
    /// The lines where the script exports a variable of [`SHELLOPTS_EXPORT`], which hands its options to
    /// every bash started below it.
    shellopts: Vec<u64>,
    /// The script each `eval` runs and each child shell's `-c` program, read as the script's own commands,
    /// at the line the command stands on.
    evals: Vec<Evaluated>,
}

/// A script's settings, and a line (`<line>: <why>`) for each command that decides them but cannot be read.
fn settings_of(tree: &Value) -> (Settings, Vec<String>) {
    let mut settings = Settings {
        pipefail: false,
        sources: Vec::new(),
        grep_env: !vars_set(tree, GREP_ENV_VARS).is_empty(),
        shellopts: vars_set(tree, SHELLOPTS_EXPORT),
        evals: Vec::new(),
    };
    let mut unseen = Vec::new();
    for call in calls_in_order(tree) {
        if handed_shellopts(call) {
            unseen.push(format!("{}: {CHILD_SHELLOPTS}", line_of(call)));
        }
        settings_reading(&words_of(call), &mut settings, &mut unseen, line_of(call), 0, false);
    }
    (settings, unseen)
}

/// Every simple command in a tree, in the order they stand, so a refusal raised by one of them is read in
/// line order; the settings themselves do not depend on the order, since one `set` turning `pipefail` on is
/// enough.
fn calls_in_order(tree: &Value) -> Vec<&Value> {
    let mut calls = Vec::new();
    calls_in(tree, &mut calls);
    calls.sort_by_key(|call| call.get("Pos").and_then(|pos| pos.get("Offset")).and_then(Value::as_u64).unwrap_or(0));
    calls
}

/// A simple command's words, its program's first.
fn words_of(call: &Value) -> Vec<&Value> {
    call.get("Args").and_then(Value::as_array).map(|args| args.iter().collect()).unwrap_or_default()
}

/// One command's words read against the script's settings: whether it turns `pipefail` on and what it
/// `source`s — and, for a wrapper, an `eval` and a child shell, of the words those hand on too: what
/// `builtin set -o pipefail`, `command set -o pipefail`, `eval 'set -o pipefail'` and
/// `env bash -o pipefail -c '…'` turn on, and what `builtin source` and `command .` source, is read through
/// this one definition rather than a second one that knows those spellings. `depth` counts the `eval`s and
/// child programs this command stands inside.
fn settings_reading(words: &[&Value], settings: &mut Settings, unseen: &mut Vec<String>, line: u64, depth: usize, handed: bool) {
    let Some((first, rest)) = words.split_first() else { return };
    match program(first).as_deref() {
        // One `set` or `shopt` turning `pipefail` on is enough for the whole script: no other command
        // turns it back off for the reading, since which pipes run while it is on is flow the tree does
        // not answer, and reading only up to the last one would pass the very pipe this rule is for.
        Some("set") => match pipefail_from(rest, false) {
            Some(Some(true)) => settings.pipefail = true,
            Some(Some(false)) | Some(None) => {}
            None => unseen.push(format!("{line}: {SET_UNSEEN}")),
        },
        Some("shopt") => match pipefail_from(rest, true) {
            Some(Some(true)) => settings.pipefail = true,
            Some(Some(false)) | Some(None) => {}
            None => unseen.push(format!("{line}: {SET_UNSEEN}")),
        },
        Some("source" | ".") => match rest.first() {
            // A script read from the standard input is no script of the list: the words it runs stand on
            // whatever stream carries them, able to set options or feed pipes unseen by any reading here.
            Some(file) if stdin_script(file) => unseen.push(format!("{line}: {SOURCE_STDIN}")),
            Some(file) => match program(file) {
                Some(name) => settings.sources.push(name),
                None => unseen.push(format!("{line}: {SOURCE_UNSEEN}")),
            },
            None => unseen.push(format!("{line}: {SOURCE_UNSEEN}")),
        },
        // The word after a wrapper's options runs in its place, so it is read through the same definition,
        // past the options as the one reading of every wrapper's options reads them; a wrapper that runs no
        // command (`command -v`) runs no setting either. `builtin` and `command` run the shell's own
        // builtins, `set` and `source` among them, while `env`, `exec`, `nice` and `nohup` start a program,
        // which no builtin is: what follows one of those is read only where it starts a child shell, or
        // another wrapper — `env bash -o pipefail -c '…'` runs its program as `bash` alone does (measured
        // 141 at the string's `grep -q` pipe).
        Some(wrapper) if WRAPPERS.contains(&wrapper) => {
            let past = past_wrapper_options(wrapper, rest, 0);
            // An option this reading cannot follow may have eaten the word that names the command, and a
            // child shell standing behind it runs its `-c` program unseen, behind every wrapper alike: GNU's
            // `env` reads every abbreviation of its long options as the option each abbreviates, eating a
            // value and naming no command this reading can name, then runs the word behind
            // (`env --un FOO bash -o pipefail -c '…'` runs the string's `grep -q` pipe at 141), and so do
            // `nice -5 bash …` on both `nice`s, GNU's `nice --adjustment=5 bash …` and bash's
            // `exec -cl bash …` (each measured 141). The settings reading must not walk past one in
            // silence: where an option hides the command and a word naming a child shell stands among the
            // rest ([`names_child_shell`]), the line is refused as an unread child. A hidden option hiding
            // a word no wrapper would run (`nohup --help bash …`) is refused too, which is the safe half:
            // it costs a script only a shape dead where it runs.
            if matches!(past, Program::Hidden) && names_child_shell(rest) {
                unseen.push(format!("{line}: {CHILD_UNREAD}"));
            }
            if let Program::At(at) | Program::BehindHand(at) = past {
                let runs = rest.get(at..).unwrap_or_default();
                let handed = handed || past.hand_unwritten();
                let named = runs.first().and_then(|word| program(word));
                let starts = |name: &String| CHILD_SHELLS.contains(&name.as_str()) || WRAPPERS.contains(&name.as_str());
                if matches!(wrapper, "builtin" | "command") || named.as_ref().is_some_and(starts) {
                    settings_reading(runs, settings, unseen, line, depth, handed);
                } else if handed {
                    // A program started behind a hand this reading cannot spell runs whatever that hand named,
                    // `SHELLOPTS=pipefail` among it: `env "$setup" tools/other.sh` puts the script's own
                    // `grep -q` pipe at 141 (measured), as a child shell's own `-o pipefail` does the file it
                    // runs. So the script it names is marked as that file is, and the setting read as made.
                    settings.pipefail = true;
                    settings.sources.extend(named);
                }
            }
        }
        // `eval` runs the script its words spell, so that script is read as this one's own commands: its
        // settings, its variables and its pipes, at the `eval`'s line. One whose words are not all written
        // out could spell anything, `set -o pipefail` among it, so its shell is read under `pipefail`, the
        // reading's safe way, and what does show — `eval "set -o $option"`, `eval "source $library"` — is
        // read as far as it goes, so a `set` or `source` it shows unread is refused as one written out would
        // be. `tools/env.sh` runs `eval "$exports"` on `cargo ndk-env`'s output in a shell every script that
        // sources it runs under `pipefail` already.
        Some("eval") => match evaluated(rest, line, depth) {
            Ok(eval) => {
                settings.pipefail |= eval.holes;
                fold_script(eval, settings, unseen, depth);
            }
            Err(_) => unseen.push(format!("{line}: {EVAL_UNREAD}")),
        },
        // A child shell runs its `-c` program or the file it names in a shell of its own: the program reads
        // as this script's own, and the options its words turn on stand for this script too, the safe way
        // across a boundary no flow reading answers — an option that turns the child on turns on the script
        // and its file alike, over-marking at worst a script whose child read its own default after all. A
        // file the child runs with `pipefail` on runs its pipes unseen by the file's own reading, measured
        // 141, and is marked as a `source` marks one, while a file named by a word with no name in it is
        // refused there; a child reading its program from a stream, which can turn the setting on itself,
        // and one whose program cannot be read whole could run anything: both refused.
        Some(name) if CHILD_SHELLS.contains(&name) => {
            let (child, child_pipefail) = child_program(rest, line, depth);
            // A wrapper before this shell read its words past one this reading cannot spell, and such a word
            // stands where `env` hands an assignment: `env "$setup" bash other.sh` with `$setup` spelling
            // `SHELLOPTS=pipefail` starts bash under the setting (measured 141 in bash 3.2), unseen by any
            // reading of this script or of the shell's own. The shell is read as turned on, the safe way
            // across a boundary no reading answers, so its program and the file it runs are read under
            // `pipefail` as an option written out would read them, and the script with them.
            let child_pipefail = child_pipefail || handed;
            settings.pipefail |= child_pipefail;
            match child {
                Child::Program(program) => fold_script(program, settings, unseen, depth),
                Child::File(Some(name)) if child_pipefail => settings.sources.push(name),
                Child::File(None) if child_pipefail => unseen.push(format!("{line}: {CHILD_UNREAD}")),
                Child::File(_) | Child::Parses => {}
                Child::Stream | Child::Unread => unseen.push(format!("{line}: {CHILD_UNREAD}")),
            }
        }
        _ => {}
    }
}

/// The script one `eval` runs: its words as the shell hands them to it, joined with a space as `eval` joins
/// them, read through shfmt's tree as every script here is, with whether any of them was not written out.
struct Evaluated {
    line: u64,
    text: String,
    tree: Value,
    holes: bool,
}

/// What an `eval` with these words runs, as [`Evaluated`]; an expansion among them stands as [`EVAL_HOLE`]. An
/// `eval` nested past [`EVAL_DEPTH`], or whose text shfmt cannot parse, is no script this reading can read.
fn evaluated(words: &[&Value], line: u64, depth: usize) -> Result<Evaluated, String> {
    if depth >= EVAL_DEPTH {
        return Err(format!("an eval nested deeper than {EVAL_DEPTH}"));
    }
    let mut holes = false;
    let mut spelled = Vec::new();
    for word in words {
        let mut text = String::new();
        holes |= spell(word, EVAL_HOLE, program_part_text, &mut text);
        spelled.push(text);
    }
    let text = spelled.join(" ");
    let tree = parse("eval", &text)?;
    Ok(Evaluated { line, text, tree, holes })
}

/// A word's text as far as `read` vouches for each of its parts, a double-quoted part read part by part, and
/// `hole` standing for each part it does not; whether any part stood as the hole. The one walk both readings
/// of a word's text take: a sed script keeps the text the tree holds, backslashes and all, and an `eval`'s
/// words or a child shell's `-c` program keep only what [`program_part_text`] vouches the shell hands on.
fn spell(word: &Value, hole: &str, read: fn(&Value) -> Option<String>, text: &mut String) -> bool {
    let mut holes = false;
    for part in word.get("Parts").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
        if let Some(held) = read(part) {
            text.push_str(&held);
        } else if part.get("Type").and_then(Value::as_str) == Some("DblQuoted") {
            holes |= spell(part, hole, read, text);
        } else {
            text.push_str(hole);
            holes = true;
        }
    }
    holes
}

/// Fold the script a command runs — an `eval`'s text, a child shell's `-c` program — into the script
/// running it: its variables, its settings and its pipes, read as the running script's own at the line the
/// command stands on, through this same walk one level deeper.
fn fold_script(program: Evaluated, settings: &mut Settings, unseen: &mut Vec<String>, depth: usize) {
    let line = program.line;
    settings.grep_env |= !vars_set(&program.tree, GREP_ENV_VARS).is_empty();
    if !vars_set(&program.tree, SHELLOPTS_EXPORT).is_empty() {
        settings.shellopts.push(line);
    }
    for call in calls_in_order(&program.tree) {
        if handed_shellopts(call) {
            unseen.push(format!("{line}: {CHILD_SHELLOPTS}"));
        }
        settings_reading(&words_of(call), settings, unseen, line, depth + 1, false);
    }
    settings.evals.push(program);
}

/// What a child shell's words hand the shell they start.
enum Child {
    /// A `-c` program written out whole and parsed: its commands are the running script's own at the
    /// child's line, its own `set` included, read through [`evaluated`] as an `eval`'s text is.
    Program(Evaluated),
    /// A script file, named by a word written out, or `None` where the word names none.
    File(Option<String>),
    /// No program and no file, which leaves the shell reading its program from its standard input — the
    /// pipe itself where the shell stands at a pipe's end, a here-document, a redirected file — or a file
    /// that is a stream ([`stdin_script`], a lone `-`): a stream carries no script of the list, and what it
    /// runs can turn `pipefail` on itself (measured: `bash <<EOF` of a `set -o pipefail` and a `grep -q`
    /// pipe answers 141 there), as what `source /dev/stdin` runs can.
    Stream,
    /// `-n` and `-D`: the shell parses what it is given and runs no command of it.
    Parses,
    /// Words this reading cannot tell what the shell runs: no `-c` program, one not written out whole, one
    /// shfmt cannot parse, an option this reading does not read, or a word before the program that is not
    /// written out at all, which the child may read as one of its own options.
    Unread,
}

/// What one child shell's words hand it, and whether its own options turn `pipefail` on. A child starts
/// with a shell's defaults however its parent stands — an inherited `SHELLOPTS` export is refused at the
/// export and a hand at the hand — so the words alone decide, read through [`pipefail_from`] like a
/// `set`'s are. The options run up to the first word that is none, or past `--`, and the word standing
/// there is the program: the `-c` string where a `-c` stood anywhere among the options, since `-c` is a
/// switch that makes the first operand the program and not an option taking the next word, and the script
/// file otherwise. `bash -c -o pipefail '…'` and `bash -co pipefail '…'` run the string under `pipefail`
/// (measured 141 in bash 3.2, and in macOS's `sh` and `zsh` alike), as POSIX's `sh -c [options] string`
/// has it. An `o` or an `O` takes the next word as its name wherever it stands in its word —
/// `bash -of pipefail -c '…'` turns `pipefail` on (measured 141) — so two of them in one word, whose names
/// this reading would have to pair up, end it unread. Options this reading does not know end it unread
/// rather than guess past them: a long option, which bash 3.2 and later bash read differently (`--posix`
/// was there in 3.2 and is gone now), and every letter but these, whatever it takes. A word not written out
/// could be one of these options rather than the program — `"$flags"` spells `-o` as easily as it spells
/// `other.sh` — so it ends the reading unread with the child read on unless its own first letter is written
/// out and no `-` ([`spells_operand`]): `bash $flags x.sh` with `-o pipefail` among the words `$flags` stands
/// for runs `x.sh` under it, and no reading here can tell that coming.
/// A file named by a word with no name in it: a child run with
/// `pipefail` off runs whatever it runs under defaults this script answers nothing for, the way any other
/// program's children do, and one run with it on runs unseen under a setting that was made, which is
/// refused at the line. No program and no file, or a stream for the file, is [`Child::Stream`].
fn child_program(args: &[&Value], line: u64, depth: usize) -> (Child, bool) {
    let mut index = 0;
    let mut parses = false;
    let mut command = false;
    while let Some(word) = args.get(index) {
        // A word not written out is one of the child's options as easily as it is its program: with `flag=c`
        // it makes `bash "-$flag" -o pipefail '…'` run the string under `pipefail`, and with `flag=-o` it
        // makes `bash "$flag" pipefail -c '…'` do the same (measured 141 in bash 3.2, in macOS's `sh` and in
        // `zsh`), and each word after it could be one more option this reading never sees. Only a word whose
        // first letter is written out and is no `-` says the child's options ended there.
        let Some(text) = literal(word) else {
            if spells_operand(word) {
                break;
            }
            return (Child::Unread, true);
        };
        // A lone `-` ends the options as `--` does, and leaves the child reading its standard input when no
        // word follows (measured: `bash -o pipefail -` runs a `grep -q` pipe it reads there at 141).
        if text == "--" || text == "-" {
            index += 1;
            break;
        }
        if text.starts_with("--") {
            return (Child::Unread, true);
        }
        let Some(flags) = text.strip_prefix(['-', '+']).filter(|flags| !flags.is_empty()) else { break };
        let mut names = 0;
        for flag in flags.chars() {
            match flag {
                'c' => command = true,
                // `-o`'s option name reads through `pipefail_from` over every option word, and `-O`'s can
                // only name a shopt and never `pipefail`: each is counted here only so its name is not
                // read as the program.
                'o' | 'O' => names += 1,
                // `-n` and `-D` parse what they are given and run none of it; these letters neither take a
                // value nor move the run off what the words give (3.2 reads `-e`, `-f`, `-i`, `-u` and `-x`
                // as plain switches; `-b`, `-g`, `-s`, `-C`, `-S` and `-E` are options this reading does
                // not know and end it unread below).
                'n' | 'D' => parses = true,
                'e' | 'f' | 'i' | 'u' | 'x' => {}
                _ => return (Child::Unread, true),
            }
        }
        if names > 1 {
            return (Child::Unread, true);
        }
        index += 1 + names;
    }
    // The safe way across a boundary no flow reading answers: what `pipefail_from` cannot tell here is read
    // on, so an option that could turn the child on turns on the script and its file alike.
    let pipefail = match pipefail_from(args.get(..index.min(args.len())).unwrap_or_default(), false) {
        None => true,
        Some(decided) => decided == Some(true),
    };
    let program_word = args.get(index);
    let child = if parses {
        Child::Parses
    } else if command {
        match program_word.map(|word| evaluated(&[word], line, depth)) {
            // Unlike an `eval`, whose opaque text runs in this very shell and is read as turning `pipefail`
            // on, a child's unreadable program is refused: the harm it could do stands below the boundary,
            // in pipes and settings no reading of this script reaches, so no safe default reads it whole.
            Some(Ok(program)) if !program.holes => Child::Program(program),
            _ => Child::Unread,
        }
    } else {
        match program_word {
            Some(word) if !stdin_script(word) => Child::File(program(word).filter(|name| !name.is_empty())),
            _ => Child::Stream,
        }
    };
    (child, pipefail)
}

/// Whether a word not written out is surely what a child shell reads as its operand rather than one of its
/// options: the whole word is text the shell hands on, with nothing an expansion stands for inside it, and
/// that text begins with a letter no option begins with. So `bash -c "printf 'y\n' | grep -q x"` counts as
/// the child's program, backslash and all, while `"-$flag"`, `"$flags"`, `"$script"`, `"run-$name.sh"` and
/// `"$@"` say nothing about their first letter and could each stand for `-c` or `-o`, with the option's own
/// word, or the program, among the words after.
fn spells_operand(word: &Value) -> bool {
    let mut text = String::new();
    !spell(word, EVAL_HOLE, program_part_text, &mut text) && matches!(text.chars().next(), Some(first) if first != '-')
}

/// Whether a command is handed a `SHELLOPTS` assignment in the environment it runs with: one given to a
/// wrapper made to hand assignments to what it runs (`env SHELLOPTS=pipefail bash other.sh`, measured to
/// answer 141 at the child's `grep -q` pipe where the parent set nothing), or one standing before the
/// command's name (`SHELLOPTS=errex bash other.sh`), which shfmt keeps off the command's own words, in its
/// `Assigns`: bash refuses that assignment to its own readonly variable and hands nothing (measured, macOS's
/// `sh` alike, which is bash), while dash, ksh or zsh running the line hands it whole. The shell started
/// then holds the options named there, unseen by any reading of this script or of that shell's own; which
/// commands are handed one no reading can list, so the hand is refused whole, in every script and whatever
/// the command names — any word of it spelling the assignment — as the export is. An assignment with no
/// command after it hands nothing: it sets this shell's own variable, which the export reading answers for.
fn handed_shellopts(call: &Value) -> bool {
    let words = words_of(call);
    let prefixed = call.get("Assigns").and_then(Value::as_array).is_some_and(|assigns| {
        assigns.iter().any(|assignment| {
            assignment.get("Name").and_then(|name| name.get("Value")).and_then(Value::as_str).is_some_and(|name| SHELLOPTS_EXPORT.contains(&name))
        })
    });
    hands_variable(&words, SHELLOPTS_EXPORT) || (prefixed && !words.is_empty())
}

/// Whether a `source`'s file argument names the standard input rather than a file: `/dev/stdin`, a
/// `/dev/fd/N` or a `/proc/self/fd/N` name, which carry whatever stream the script runs on — bash opens
/// process substitutions there, and words run through `source /dev/stdin` were measured to set `pipefail`
/// unseen — and never a script of the list this reading reads.
fn stdin_script(word: &Value) -> bool {
    let Some(text) = literal(word) else { return false };
    text == "/dev/stdin"
        || matches!(text.strip_prefix("/dev/fd/").or_else(|| text.strip_prefix("/proc/self/fd/")), Some(number) if !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
}

/// How far one long-option word shows the option: its name with the dashes off, and its value too when the
/// word writes `--name=value`. The one spelling of that split, shared by every option reading here.
fn long_option(text: &str) -> Option<(&str, Option<&str>)> {
    let long = text.strip_prefix("--")?;
    Some(long.split_once('=').map_or((long, None), |(name, value)| (name, Some(value))))
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

/// The lines where a script sets one of the given variables, in a command's own assignment or in an
/// `export`, `declare`, `local` or `readonly`: shfmt writes the name of each variable an assignment gives as
/// a `Name` of its own, so no reading of the words is needed to find it. Where in the script it stands is
/// not weighed, an `unset` after it included: which of the script's commands run while it is set is a
/// question of the script's flow, which no syntax tree answers, so the whole script is read as setting it
/// rather than half of it read at all. A declaration that takes the export attribute back names its
/// variable without exporting it ([`takes_back`]), which is no setting this reads.
fn vars_set(node: &Value, vars: &[&str]) -> Vec<u64> {
    let mut lines = Vec::new();
    vars_set_in(node, vars, &mut lines, false);
    lines
}

fn vars_set_in(node: &Value, vars: &[&str], lines: &mut Vec<u64>, taken_back: bool) {
    match node {
        Value::Object(fields) => {
            // A declaration that takes the export attribute back names its variable without exporting it:
            // `export -n SHELLOPTS` keeps the value in this shell and takes it out of the environment every
            // program below is handed, and `declare +x` and `typeset +x` do the same (measured in bash 3.2,
            // where bash keeps `SHELLOPTS` readonly and the take-back succeeds all the same). The harm of
            // an export stands in the attribute, not the name, so this is read as the taking-back it is —
            // which is how `tools/env.sh` clears an inherited `SHELLOPTS` export without setting anything.
            // Only `SHELLOPTS` is exempted this way: the grep variables change what a grep reads from where
            // they stand in the shell at all, however they are declared.
            let taken_back = taken_back || takes_back(fields);
            if let Some(name) = fields.get("Name").filter(|name| {
                name.get("Value").and_then(Value::as_str).is_some_and(|held| vars.contains(&held) && !(taken_back && SHELLOPTS_EXPORT.contains(&held)))
            }) {
                lines.push(line_of(name));
            }
            for value in fields.values() {
                vars_set_in(value, vars, lines, taken_back);
            }
        }
        Value::Array(items) => {
            for value in items {
                vars_set_in(value, vars, lines, taken_back);
            }
        }
        _ => {}
    }
}

/// Whether a declaration names its variables without exporting them: bash's `export -n NAME` and `declare`
/// or `typeset` with `+x` take the export attribute off. Only these spellings are read as taking back:
/// `declare -n` and `local -n` name a nameref in bash, which is no taking-back, and an `export` claiming
/// `-x` beside `-n` is read as an export, the worse of the two. bash itself, keeping `SHELLOPTS` readonly,
/// answers every other spelling with an error and sets nothing.
fn takes_back(fields: &serde_json::Map<String, Value>) -> bool {
    let args: Vec<&Value> = fields.get("Args").and_then(Value::as_array).map(|args| args.iter().collect()).unwrap_or_default();
    let flag = |name: &str| args.iter().any(|arg| naked_word(arg, name));
    match fields.get("Variant").and_then(|variant| variant.get("Value")).and_then(Value::as_str) {
        Some("export") => flag("-n") && !flag("-x"),
        Some("declare") | Some("typeset") => flag("+x") && !flag("-x"),
        _ => false,
    }
}

/// A declaration's bare option word, which holds its text as any word does; a variable's name stands under
/// a `Name` these flags cannot name, so only a flag reads as one here.
fn naked_word(arg: &Value, name: &str) -> bool {
    arg.get("Naked").and_then(Value::as_bool) == Some(true)
        && [Some(arg), arg.get("Value")].into_iter().flatten().any(|word| literal(word).as_deref() == Some(name))
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
/// under it. `depth` counts the `eval`s the stage stands inside.
fn judge(stage: &Value, grep_env: bool, depth: usize) -> Option<&'static str> {
    let Some(cmd) = stage.get("Cmd").filter(|cmd| cmd.get("Type").and_then(Value::as_str) == Some("CallExpr")) else {
        return Some(COMPOUND_READER);
    };
    let words = words_of(cmd);
    let mut index = 0;
    loop {
        let word = words.get(index)?;
        let Some(name) = program(word) else { return Some(NAMED_UNSEEN) };
        index += 1;
        if WRAPPERS.contains(&name.as_str()) {
            match past_wrapper_options(&name, &words, index) {
                Program::At(next) if next < words.len() => index = next,
                // A word not written out stood before the command this reading came to: it is the command as
                // easily as it is an option or an assignment before it, so which word reads the pipe is not
                // known here.
                Program::BehindHand(_) => return Some(NAMED_UNSEEN),
                Program::At(_) | Program::Nothing => return Some(WRAPPER_RUNS_NOTHING),
                // A child shell standing behind an option this reading cannot follow is the unread child
                // the settings reading refuses at this same line, so the pipe's reader answers the same,
                // and the command is one problem, told once.
                Program::Hidden if names_child_shell(words.get(index..).unwrap_or_default()) => return Some(CHILD_UNREAD),
                Program::Hidden => return Some(NAMED_UNSEEN),
            }
            continue;
        }
        let rest = words.get(index..).unwrap_or_default();
        return match name.as_str() {
            "grep" | "egrep" | "fgrep" if grep_env || hands_variable(words.get(..index).unwrap_or_default(), GREP_ENV_VARS) => Some(GREP_ENV),
            "grep" | "egrep" | "fgrep" => grep_quits(rest),
            "head" => Some(HEAD_SHAPE),
            "sed" => sed_verdict(rest),
            "read" => Some(READ_SHAPE),
            "mapfile" | "readarray" => mapfile_verdict(rest),
            "awk" | "gawk" | "mawk" | "nawk" => Some(AWK_SHAPE),
            "eval" => eval_reads(rest, grep_env, depth),
            // A child shell at a pipe's end reads the pipe through what it runs: its `-c` script is read as
            // an `eval`'s is, its first command the reader; a parse-only child runs nothing and reads
            // nothing, as a wrapper running nothing does; a script file it runs reads the pipe through
            // commands this reading does not reach, and a child with no program of its own reads the pipe
            // itself as its script — either unseen, and refused.
            name if CHILD_SHELLS.contains(&name) => match child_program(rest, 0, depth).0 {
                Child::Program(program) => script_reads(&program, grep_env, depth),
                Child::Parses => Some(WRAPPER_RUNS_NOTHING),
                Child::File(_) | Child::Stream | Child::Unread => Some(CHILD_UNREAD),
            },
            _ => None,
        };
    }
}

/// Whether a word naming a child shell stands among a wrapper's words: behind an option this reading cannot
/// follow, which may have eaten the word naming the command, any of them could be the shell the wrapper
/// runs.
fn names_child_shell(words: &[&Value]) -> bool {
    words.iter().filter_map(|word| program(word)).any(|name| CHILD_SHELLS.contains(&name.as_str()))
}

/// What an `eval` at a pipe's end reads of it: the script it runs reads the pipe with its first command, so
/// that command is the reader, judged as any other, and a script of more than one statement is read as the
/// compound command it is. Its words read as [`evaluated`] reads them, so one held in an expansion stands as
/// a reader named by an expansion.
fn eval_reads(words: &[&Value], grep_env: bool, depth: usize) -> Option<&'static str> {
    let Ok(eval) = evaluated(words, 0, depth) else { return Some(EVAL_UNREAD) };
    script_reads(&eval, grep_env, depth)
}

/// What the first command of a script read from a command's words — an `eval`'s text, a child shell's `-c`
/// program — makes of the pipe, judged as any other reader, with a script of more than one statement read
/// as the compound command it is.
fn script_reads(program: &Evaluated, grep_env: bool, depth: usize) -> Option<&'static str> {
    match program.tree.get("Stmts").and_then(Value::as_array).map(Vec::as_slice) {
        Some([statement]) => {
            let mut stages = Vec::new();
            stages_of(statement, &program.text, &mut stages);
            stages.first().and_then(|first| judge(first, grep_env, depth + 1))
        }
        _ => Some(COMPOUND_READER),
    }
}

/// The name a word written out as `NAME=value` hands, given where it stands behind `env`: the text before
/// its first `=`, whenever such a word is an assignment where it stands, and `None` when it is no
/// assignment but an option or the command. Where the word stands is all that can decide it: before `env`'s
/// options end the only name that hands nothing is one beginning with `-`, which is one of `env`'s options
/// whatever it holds, while once they have ended — at its `--`, or at its first assignment, past which
/// neither `env` reads an option — `env` reads every word holding a `=` as an assignment (measured on
/// macOS's `env` and on GNU's alike: `env -x=1 true` ends at once with its unknown-option error, where
/// `env -- -x=1 printenv -x` and `env A=1 -x=1 printenv -x` print `1`, and `env -- -x=1 bash -o pipefail
/// -c '…'` and `env A=1 -x=1 bash …` run the string's `grep -q` pipe at 141). A name may even be empty: GNU's `env` sets such a word
/// and runs the command behind it wherever it stands, and macOS's ends at 1 having run nothing (measured:
/// GNU runs the command behind `env '=x=1'` at 141, before `--` and past it alike, where macOS answers 1)
/// — the word is never a command on either, so the reading carries it as an assignment, which over-refuses
/// at worst macOS's dead end.
fn assignment_name(text: &str, options_ended: bool) -> Option<&str> {
    let (name, _) = text.split_once('=')?;
    (options_ended || !name.starts_with('-')).then_some(name)
}

/// Whether any of these words hands one of `vars` to the program a command runs, written out as
/// `NAME=value`: `env GREP_OPTIONS=-q grep -q x` changes what its grep reads as its options as a script
/// setting the variable does, and `env SHELLOPTS=pipefail bash` starts its shell under the options named.
/// Every tracked name is a plain variable name, neither empty nor beginning with `-`, so no position of a
/// word can change whether it hands one, and the words are read as if each stood where `env`'s options have
/// ended.
fn hands_variable(words: &[&Value], vars: &[&str]) -> bool {
    words.iter().filter_map(|word| literal(word)).any(|text| assignment_name(&text, true).is_some_and(|name| vars.contains(&name)))
}

/// Where the reading of a wrapper's options leaves the command it runs.
enum Program {
    /// The word at this index is the command, or stands where one would be if the word is not written out.
    At(usize),
    /// The wrapper runs no command at all, so nothing reads the pipe and its writer dies of SIGPIPE.
    Nothing,
    /// An option hides the command from this reading, in its value or by eating the word that names it.
    Hidden,
    /// The command is at this index, but a word before it is not written out, so what the wrapper hands it,
    /// and which word it runs, are not this reading's to tell: such a word stands where every wrapper reads an
    /// option and `env` an assignment, and spells one as easily as it spells the command —
    /// `env "$setup" bash other.sh` runs bash under `SHELLOPTS=pipefail` when `$setup` spells it, and
    /// `cat big | env "$setup" grep pat` reads the grep's options from `GREP_OPTIONS=-q` (each measured 141
    /// in bash 3.2), with no word of the line naming it.
    BehindHand(usize),
}

impl Program {
    /// Whether a word this reading cannot spell stood before the command, so the hand the command was given
    /// is unknown: a shell started behind one holds options no reading of this script or of its own answers.
    /// Only `env` hands a command anything, but a word before any other wrapper's command is read the same
    /// way, which over-marks at worst a child behind `nice "$n"`.
    fn hand_unwritten(&self) -> bool {
        matches!(self, Program::BehindHand(_))
    }
}

/// Where the program a wrapper runs is named: past the wrapper's options (and `env`'s assignments), the
/// options that take a value taking theirs — `exec -a NAME CMD` runs CMD with NAME as its argv0 (measured on
/// bash 3.2) — bundled in one cluster as the getopt of each reads them, and `--` ending them, though `env`
/// reads its assignments on past its `--` and runs the first word after that ends none (measured on macOS's
/// `env` and on GNU's), and a bare `-` counting as one of `env`'s options up to its first assignment (measured
/// on both); a word that is not written out is read past, and the command after it is [`Program::BehindHand`]. [`Program::Nothing`] when the wrapper
/// runs no command at all: `command -v` and
/// `-V`, bundled or not, print what a name is, an option `builtin` or `command` has not got ends either at
/// once with its error, `env -0` refuses a command and `env --help` and `--version` end, and a wrapper left
/// with no command after its options runs nothing either (`env` alone prints the environment; measured on
/// macOS: each ends without reading the pipe, whose writer dies of SIGPIPE). [`Program::Hidden`] when an
/// option hides the command from this reading: one that takes the command itself in its value —
/// `env -S 'head -n 1'` runs `head`, which quits early (measured on macOS's `env`; GNU's spells it
/// `--split-string` too) — or one the reading does not follow, which one platform's wrapper has not got and
/// ends at once on, or which may take a value the reading would mistake for the command (GNU's `env` reads
/// every abbreviation of `--unset` as taking one, so `env --un FOO head` runs head, where macOS's `env` reads
/// no long option). The one reading of every wrapper's options, for the pipe's reader and the settings alike.
fn past_wrapper_options(wrapper: &str, words: &[&Value], mut index: usize) -> Program {
    let mut behind = false;
    // Whether `env`'s options have ended: at its `--`, and at its first assignment, past which neither
    // `env` reads an option — `env A=1 -i …` runs a program named `-i` and `env A=1 -- cat` one named `--`
    // (measured on macOS's `env` and on GNU's). Past that point even a `-word` is no option, only an
    // assignment or the word to run.
    let mut options_ended = false;
    // Whether `env` has read its first assignment. A bare `-` keeps an option's standing up to it, past
    // `--` too, and is a word to run after it (all measured; read below where it stands): `--` ends every
    // other option's standing, but only an assignment ends this one.
    let mut assigned = false;
    let at = |index: usize, behind: bool| if behind { Program::BehindHand(index) } else { Program::At(index) };
    while let Some(word) = words.get(index) {
        // A word not written out stands where every wrapper reads an option and `env` an assignment too, so it
        // is one of those as easily as it is the command, for each wrapper alike: `env "$V=1" bash …`,
        // `nice "$o" bash …` with `-n5`, `exec "$o" bash …` with `-l` and `command "$o" bash …` with `-p`
        // each run the child's `-o pipefail -c` string (measured 141), and `command "$o" set -o pipefail`
        // turns the setting on. So the words after it are still read, the first written out that is no
        // option taken as the command, behind a word this reading cannot spell. When that word is an option
        // whose value comes next (`env "$o" FOO bash …` with `-u`), the value is taken for the command, and
        // what runs after it stays unread.
        let Some(text) = literal(word) else {
            behind = true;
            index += 1;
            continue;
        };
        if text == "--" && !options_ended {
            if wrapper != "env" {
                return at(index + 1, behind);
            }
            // `--` ends `env`'s options but not its assignments: past it `env` still reads every word
            // holding a `=` as one and runs the first that is none — `env -- FOO=1 grep -q pat` runs that
            // grep on the pipe and `env -- SHELLOPTS=pipefail bash other.sh` starts bash under the handed
            // options (measured 141 on macOS's `env` and on GNU's alike) — so an unread word there is the
            // same hand this reading cannot spell, and the words after are read as before, only without options.
            options_ended = true;
            index += 1;
            continue;
        }
        // A bare `-` on `env` is an option, not the command it spells, until its first assignment: each
        // `env` consumes one in its options and runs the word behind it (measured: `env - grep -q x`
        // answers 141 on both), macOS's reading every such word there and GNU's one, its `--` included
        // (`env -- - grep -q x` answers 141 on GNU, where macOS runs a program named `-`; `env -- A=1 -
        // cat` runs the `-`, measured on both). The reading follows the env that runs a command; where
        // that runs one env's dead end instead — a second `-`, which GNU runs where macOS runs the first
        // — the refusal falls on a word no env finds, refusing a dead end being the safe half of this rule.
        if wrapper == "env" && text == "-" && !assigned {
            index += 1;
            continue;
        }
        // Before `env`'s options end a word beginning with `-` is one of them, whatever it holds; once they
        // have ended — or where a word this reading cannot spell may have ended them, as an assignment it
        // could be would (`env "$v" -x=1 bash …` with `v=B=2` runs the child, measured 141) — every word
        // holding a `=` is an assignment, which ends them in turn: [`assignment_name`] reads the name each
        // one hands.
        if wrapper == "env" && assignment_name(&text, options_ended || behind).is_some() {
            options_ended = true;
            assigned = true;
            index += 1;
            continue;
        }
        if options_ended || !text.starts_with('-') {
            return at(index, behind);
        }
        let takes_value = match wrapper {
            // Any option ends them: `builtin -p` lists the builtins and runs no command, pipe-fed or not
            // (measured: `builtin -p set -o pipefail` turns nothing on, and a pipe into `builtin -p` answers
            // its usage error), and an option either has not got (`command -V`, `builtin -x`) ends it with
            // its error; `command -p` alone goes on, to the command it still runs by its path.
            "builtin" | "command" => {
                if wrapper == "builtin" || text.chars().skip(1).any(|flag| flag != 'p') {
                    return Program::Nothing;
                }
                false
            }
            // The options both `env`s read, alone or bundled in one cluster as the getopt of each reads
            // them (measured on both: `env -iu NOPE cat` and `env -iuNOPE cat` each run the cat behind)
            // — every flag of the cluster, then a value-taking option taking the rest of the cluster
            // glued to it or, where the cluster ends with it, the next word, and the command after.
            // `-P` takes the path on the one of the two that has it, and is read taking one because
            // only the env that has it runs a command, behind which the path stands spent (measured:
            // macOS runs the command behind `env -P bin cat`; GNU's 9.11 answers "invalid option --
            // 'P'", a dead end either reading passes only by refusing). `-0` refuses a command on both
            // (measured: 125), and GNU's `--help` and `--version` print and end where macOS's ends at
            // once on them, so neither runs a command either way. Any other word — `-S`, which takes
            // the command itself in its value, or a word carrying a long option, macOS ending at once
            // on each while GNU reads every abbreviation of every one as the option it abbreviates and
            // runs what stands behind it (measured on GNU: `env --un FOO head` runs head) — hides the
            // command from this reading, which is refused rather than guessed at, live on one env and
            // dead on the other.
            "env" => match text.as_str() {
                "-0" | "--null" | "--help" | "--version" => return Program::Nothing,
                _ => {
                    let mut cluster = text.chars().skip(1).peekable();
                    loop {
                        match cluster.next() {
                            None => break false,
                            Some('i') | Some('v') => {}
                            Some('u') | Some('C') | Some('P') => break cluster.peek().is_none(),
                            _ => return Program::Hidden,
                        }
                    }
                }
            },
            // bash 3.2 and 5.2 both: `-c` runs the command with a null environment and `-l` prefixes argv0
            // with a dash, neither taking a value; `-a` takes the argv0 it sets.
            "exec" => match text.as_str() {
                "-c" | "-l" => false,
                "-a" => true,
                _ => return Program::Hidden,
            },
            "nice" => match text.as_str() {
                "-n" => true,
                t if t.starts_with("-n") => false,
                _ => return Program::Hidden,
            },
            _ => return Program::Hidden,
        };
        index += if takes_value { 2 } else { 1 };
    }
    at(index, behind)
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
        if let Some((name, value)) = long_option(&text) {
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

/// What one `sed` at a pipe does with it, read the way both seds a script may run — macOS's, which is
/// FreeBSD's, and GNU's — read it: it quits where its script can reach a `q` or a `Q`, reads a file in place
/// of the pipe where it is given one, and cannot be read whole where the script comes from a file, from a
/// word not written out, or with an option one of the two seds reads as none of its own.
fn sed_verdict(words: &[&Value]) -> Option<&'static str> {
    let mut scripts: Vec<SedRead> = Vec::new();
    // One sed reads the labels of all its scripts together, so every script word of one sed is read
    // against the pool the others leave: a `b` or `t` names a label no script of that sed defines, or a
    // label is defined twice.
    let mut labels = SedLabels::default();
    let mut unread = false;
    let mut file = false;
    // Whether an `-e`, an `-f` or their long names has handed a script: after one, every operand is a file,
    // before it the first operand is the script itself.
    let mut script_given = false;
    // Whether the script came from an operand: macOS's sed ends its options at its first operand, as its
    // `getopt` does, so every word after that one is a file it opens in place of the pipe, an option word
    // and `--` included — `sed p -n` opens a file named `-n`, and `sed ':x' -e 'b x'` files named `-e` and
    // `b x` (measured) — while GNU's reads such a word as the option it spells, and with an `-e` given takes
    // the first operand for a file in turn (`sed ':x' -e 'b x'` takes `:x` for a file and ends at once,
    // finding no label for its `b x`, measured on GNU's sed 4.10): either way one of the two reads no pipe.
    let mut script_operand = false;
    let mut ends_options = false;
    let mut index = 0;
    while let Some(word) = words.get(index) {
        index += 1;
        if script_operand {
            file = true;
            continue;
        }
        // An operand: every word past `--`; a word that is not written out, whose holes show no option of
        // its own, so that whatever it could say as one, nothing after it can be read and it ends refused or
        // read as the script it stands for; and a word that is no option, `-` among them. The first operand
        // is the script, and every operand after a script was given is a file that is or could be named —
        // `-` included, which macOS's sed opens as a file named `-` while GNU's reads the pipe (the writer's
        // death by the first, measured on both).
        let text = literal(word).filter(|text| !ends_options && text.starts_with('-') && text != "-");
        let Some(text) = text else {
            if script_given {
                file = true;
            } else {
                scripts.push(sed_script(Some(*word), &mut labels));
                script_given = true;
                script_operand = true;
            }
            continue;
        };
        if text == "--" {
            ends_options = true;
            continue;
        }
        if let Some((name, value)) = long_option(&text) {
            // macOS's sed reads no long option at all and ends at once on one, so every long word is refused;
            // GNU's spellings of a script are still read, so a script that also quits says so.
            unread = true;
            match name {
                "expression" => {
                    script_given = true;
                    match value {
                        Some(value) => scripts.push(sed_read_of(value, &mut labels)),
                        // The next word is its script, whatever word it is.
                        None => {
                            scripts.push(sed_script(words.get(index).copied(), &mut labels));
                            index += 1;
                        }
                    }
                }
                // A script read from a file is a script this reading cannot see; its operand is consumed.
                "file" if value.is_none() => index += 1,
                _ => {}
            }
            continue;
        }
        let flags = text.get(1..).unwrap_or_default();
        for (at, flag) in flags.char_indices() {
            match flag {
                // The options both seds read alike, none of which moves the reading off the pipe.
                'n' | 'E' | 'r' | 'u' => {}
                'e' => {
                    script_given = true;
                    let attached = flags.get(at + 1..).unwrap_or_default();
                    if attached.is_empty() {
                        scripts.push(sed_script(words.get(index).copied(), &mut labels));
                        index += 1;
                    } else {
                        scripts.push(sed_read_of(attached, &mut labels));
                    }
                    break;
                }
                'f' => {
                    unread = true;
                    if flags.get(at + 1..).unwrap_or_default().is_empty() {
                        index += 1;
                    }
                    break;
                }
                // An option one of the two seds reads as none of its own (`-z` and `-s` GNU's alone, `-a`,
                // `-H` and `-I` macOS's alone, `-l` a count on GNU's and none on macOS's, anything unknown)
                // ends that sed at once with its error, and `-i` edits files and ends either at once on a
                // pipe: what stands after it is read by neither.
                _ => {
                    unread = true;
                    break;
                }
            }
        }
    }
    // The priority the grep reading keeps too: a script that can quit decides first, a file read in place
    // of the pipe next, and a script that cannot be read last of the shapes a script alone can show; a
    // branch to a label none of this sed's scripts defines is the last word of the whole set, as both seds
    // look every branch up only once every script reads whole, and a quit or an unread script is refused
    // for what it does before what it branches to.
    if scripts.contains(&SedRead::Quits) {
        Some(SED_SHAPE)
    } else if file {
        Some(SED_FILE)
    } else if unread || scripts.contains(&SedRead::Unread) {
        Some(SED_UNREAD)
    } else if labels.unresolved() {
        Some(SED_UNLABELED)
    } else {
        None
    }
}

/// The labels one `sed`'s whole set of scripts holds: every `:name` that names one, and every `b` or `t`
/// that branches to one. One sed reads the scripts of all its `-e`s and its first operand together, so a
/// branch defined in one script is defined for them all — measured, `sed -e 'b x' -e ':x'` answers 0 —
/// while a branch none of them defines ends both seds at once with their error before any input is read,
/// and a label defined twice ends macOS's the same way (`duplicate label`; GNU's 4.10 reads it whole).
///
/// Each label is held as macOS's sed reads it, from the text after the `:`, `b` or `t` and its blanks to
/// the line's end, blanks and all, where GNU's ends it at the first blank, `;` or `}` of that same text: so
/// `:x;p`, `:x ` and `b x ` name `x` on GNU's sed and `x;p`, `x ` and `x ` on macOS's (measured:
/// `sed -e 'b x' -e ':x '` and `sed -e 'b x ' -e ':x'` end macOS's sed with `undefined label`, and GNU's
/// 4.10 reads both whole). macOS's reading is the finer one — two labels it reads alike are one text, which
/// GNU's cuts alike — so a branch it finds defined GNU's finds defined too.
#[derive(Default)]
struct SedLabels {
    branches: Vec<String>,
    defines: Vec<String>,
}

impl SedLabels {
    /// Whether either sed would end at once on these labels: a branch to a label no `:` of the same sed
    /// defines, or a label defined twice.
    fn unresolved(&self) -> bool {
        let undefined = self.branches.iter().any(|branch| !self.defines.contains(branch));
        let twice = self.defines.iter().enumerate().any(|(at, label)| self.defines.iter().skip(at + 1).any(|other| other == label));
        undefined || twice
    }
}

/// The script an `-e` or a `--expression` takes from a word: whatever word stands there is sed's script,
/// written out or not, and where no word stands there sed was given no script and ends at once.
fn sed_script(word: Option<&Value>, labels: &mut SedLabels) -> SedRead {
    word.map_or(SedRead::Unread, |word| sed_read_of(&script_text(word), labels))
}

/// What one sed script does when sed reads it to the last line: whether it can reach a `q` or a `Q`,
/// whether it cannot be read whole, or whether it reads the input to the end.
#[derive(PartialEq, Eq)]
enum SedRead {
    Whole,
    Quits,
    Unread,
}

/// The hole an expansion leaves in a script: at a command's address position it stands for an address that
/// may match any line, since `"${served}s/^…/\1/p"` is such a script whole, and at the command position
/// itself it names no command either sed reads, which is no script this reading can vouch for. A `?` a
/// script spells itself lands in the same two places: an address may be any line all the same, and `?` is
/// no command either sed has, so a script that spells one is refused as an unread one either way.
const SED_HOLE: char = '?';

/// Whether one sed script can reach a `q` or a `Q` before its input ends. Commands are read as both seds
/// read them, one after another down a line or after a `;`, and a script either sed ends at once with its
/// error — a command, a flag or an address only one of them reads, or an `a`, `i` or `c` whose text does not
/// follow a `\` at its line's end, which macOS's sed asks for — is refused unread. Where the two disagree
/// about where a command's own text ends — macOS's reads a label to its line's end, GNU's to a blank or a
/// `;` (GNU's `sed ':x;q'` and `sed ':x q'` quit) — the text ends at the earlier place, which reads the more
/// commands and so is the safe one. A `q` whose one address is `$` is reached only with the last line read,
/// so it ends nothing early. A script that is empty, or holds addresses with no command, ends either sed at
/// once with its error, and is refused; so does one that chains commands the way only GNU reads, since
/// macOS's sed asks every command but a label, a file's and an appended text's for a `;`, a line's end or
/// the script's end after it — of an `s` this reading asks for it at once after its flags, a blank there
/// being past what it can vouch GNU reads alike — reads a branch's label to its line's end, and asks its
/// braces to balance: `sed 'x d'`, `sed '{p}'`, `sed '1{s/x/y/}'`,
/// `sed 'y/a/b/ p'`, `sed '/x/b;p'` and `sed '}'` are its errors before a line is read, GNU's whole reads
/// or quits. Such a script is read on GNU's way to its end all the same, so a `q` GNU's sed reaches after
/// it is still the quit it is. The `:` labels it names and the `b` and `t` branches it carries are gathered
/// into [`SedLabels`], so a caller reading a whole set of one sed's scripts pools them as the sed does.
fn sed_read_of(text: &str, labels: &mut SedLabels) -> SedRead {
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    let mut commands = 0;
    let mut braces = 0usize;
    // Whether one sed ends at once on something the other reads on past: macOS's on GNU's chaining, or an
    // `s`'s end this reading does not vouch both read alike.
    let mut one_sed_refuses = false;
    loop {
        while chars.get(at).is_some_and(|c| matches!(c, ' ' | '\t' | ';' | '\n' | '\r')) {
            at += 1;
        }
        if at >= chars.len() {
            return if commands > 0 && braces == 0 && !one_sed_refuses { SedRead::Whole } else { SedRead::Unread };
        }
        // An address: a line number, the last line, a regex (`I` after it both seds' case flag), a range of
        // two, then maybe `!`.
        let mut addresses = 0;
        let mut last_line_only = false;
        loop {
            let began = at;
            match chars.get(at).copied() {
                Some(c) if c.is_ascii_digit() || c == '$' || c == SED_HOLE => {
                    at += 1;
                    while chars.get(at).is_some_and(|c| c.is_ascii_digit()) {
                        at += 1;
                    }
                }
                Some(open) if open == '/' || open == '\\' => {
                    let delimiter = if open == '/' { Some('/') } else { chars.get(at + 1).copied() };
                    // A backslash with nothing after it leaves no command to read at all, and both seds refuse a
                    // newline or a backslash as a delimiter.
                    let Some(delimiter) = delimiter.filter(|delimiter| !matches!(delimiter, '\n' | '\\')) else { return SedRead::Unread };
                    at += if open == '/' { 1 } else { 2 };
                    if !sed_regex_to(&chars, &mut at, delimiter) {
                        return SedRead::Unread;
                    }
                    if chars.get(at) == Some(&'I') {
                        at += 1;
                    }
                }
                _ => {}
            }
            if at == began {
                break;
            }
            addresses += 1;
            last_line_only = addresses == 1 && at == began + 1 && chars.get(began) == Some(&'$');
            if addresses < 2 && chars.get(at) == Some(&',') {
                at += 1;
                continue;
            }
            break;
        }
        let negated = chars.get(at) == Some(&'!');
        if negated {
            at += 1;
        }
        while chars.get(at).is_some_and(|c| matches!(c, ' ' | '\t')) {
            at += 1;
        }
        let Some(&command) = chars.get(at) else { return SedRead::Unread };
        at += 1;
        match command {
            // The two commands that end sed before its input ends, bar a `q` at the last line alone, which
            // ends it with all of it read; macOS's sed has no `Q` at all.
            'q' if last_line_only && !negated => one_sed_refuses |= !sed_bare_end(&chars, at, true),
            'Q' if last_line_only && !negated => return SedRead::Unread,
            'q' | 'Q' => return SedRead::Quits,
            // `a`, `i` and `c` take their text after a `\` that ends the line, and macOS's sed ends at once
            // on any other spelling; the text then runs, on both, to a line end no `\` escapes.
            'a' | 'i' | 'c' => {
                while chars.get(at).is_some_and(|c| matches!(c, ' ' | '\t')) {
                    at += 1;
                }
                if chars.get(at) != Some(&'\\') || !matches!(chars.get(at + 1), None | Some('\n')) {
                    return SedRead::Unread;
                }
                at += 2;
                while let Some(&c) = chars.get(at) {
                    if c == '\n' {
                        break;
                    }
                    at += if c == '\\' { 2 } else { 1 };
                }
            }
            // A file argument, which both seds read to the end of the line, `;` included: GNU reads
            // `sed 'r /dev/null;q'` as one command reading a file named `/dev/null;q` and reads the input
            // whole (measured), so nothing hides in the argument and no `q` either.
            'r' | 'w' => sed_run_to(&chars, &mut at, |c| c == '\n'),
            // A branch and a branch-on-success, and a label, each named after any blanks, which both seds
            // pass over: GNU ends a label at a blank, a `;` or the line's end, macOS's at the line's end
            // alone, so a branch with more after its label on its line names a label macOS's sed has not
            // got, and it ends at once (`undefined label`, measured on `sed '/x/b;p'`); a label so followed
            // is only a longer label, which [`SedLabels`] holds as macOS's sed reads it. A `:` naming no label
            // ends both seds at once (`empty label`, `":" lacks a label`, measured).
            'b' | 't' | ':' => {
                while chars.get(at).is_some_and(|c| matches!(c, ' ' | '\t')) {
                    at += 1;
                }
                let began = at;
                sed_run_to(&chars, &mut at, |c| c.is_whitespace() || c == ';' || c == '}');
                let mut line_end = at;
                sed_run_to(&chars, &mut line_end, |c| c == '\n');
                let label: String = chars.get(began..line_end).map(|named| named.iter().collect()).unwrap_or_default();
                if command == ':' {
                    if at == began {
                        return SedRead::Unread;
                    }
                    labels.defines.push(label);
                } else if at > began {
                    labels.branches.push(label);
                }
                if command != ':' {
                    let mut end = at;
                    while chars.get(end).is_some_and(|c| matches!(c, ' ' | '\t')) {
                        end += 1;
                    }
                    one_sed_refuses |= !matches!(chars.get(end), None | Some('\n') | Some('\r'));
                }
            }
            // A `y` ends as an argument-less command does; an `s` ends at once after its flags, since a blank
            // there, which macOS's sed reads past, is a spelling this reading does not vouch GNU's reads alike.
            's' | 'y' => {
                if !sed_substitute(&chars, &mut at, command == 'y') {
                    return SedRead::Unread;
                }
                one_sed_refuses |= !sed_bare_end(&chars, at, command != 's');
            }
            '#' => sed_run_to(&chars, &mut at, |c| c == '\n'),
            // The braces, kept balanced: macOS's sed ends at once on a `}` with no `{`, or a `{` whose `}`
            // never comes, and this reading does not say which it is reading.
            '{' => braces += 1,
            '}' if braces == 0 => return SedRead::Unread,
            '}' => {
                braces -= 1;
                one_sed_refuses |= !sed_bare_end(&chars, at, true);
            }
            // The commands both seds read that take no argument (`l` takes a width on GNU's sed alone):
            // macOS's sed asks each for a `;`, a line's end or the script's end after it, and ends at once
            // with its error on anything else, GNU's blanks and a command after it included.
            '=' | 'd' | 'D' | 'p' | 'P' | 'n' | 'N' | 'x' | 'h' | 'H' | 'g' | 'G' | 'l' => one_sed_refuses |= !sed_bare_end(&chars, at, true),
            // Anything else is no command both seds read — GNU's own `e`, `F`, `R`, `T`, `W`, `v` and `z`
            // among them, and [`SED_HOLE`]: one ends its sed at once with its error, or could spell a `q`
            // this reading cannot see.
            _ => return SedRead::Unread,
        }
        commands += 1;
    }
}

/// Whether a command ends where macOS's sed makes it end: where `blanks` allows them, blanks, then a `;`,
/// a line's end or the script's end. `sed 'x d'`, `sed '{p}'`, `sed '= p'`, `sed '1{s/x/y/}'` and
/// `sed 'y/a/b/ p'` are its "extra characters", "bad flag" and "extra text" errors, which end sed before it
/// reads a line, though GNU's reads each whole (measured on macOS's sed). An `s` ends at the strict end,
/// `blanks` false: macOS's sed reads on past a blank after its flags, whether GNU's ends as macOS's does
/// there could not be measured, and an `s` so followed is refused rather than vouched — the `s` arm of
/// [`sed_read_of`] sets it so.
fn sed_bare_end(chars: &[char], at: usize, blanks: bool) -> bool {
    let mut end = at;
    if blanks {
        while chars.get(end).is_some_and(|c| matches!(c, ' ' | '\t')) {
            end += 1;
        }
    }
    matches!(chars.get(end), None | Some(';') | Some('\n') | Some('\r'))
}

/// Where an address's regex ends: at its delimiter, unescaped, outside a bracket expression and on this line.
fn sed_regex_to(chars: &[char], at: &mut usize, delimiter: char) -> bool {
    while let Some(&c) = chars.get(*at) {
        match c {
            '\\' => *at += 2,
            c if c == delimiter => {
                *at += 1;
                return true;
            }
            '\n' => return false,
            '[' => {
                if !sed_bracket_to(chars, at) {
                    return false;
                }
            }
            _ => *at += 1,
        }
    }
    false
}

/// Past one bracket expression of a regexp, from its `[` to the `]` that closes it, as both seds read one: a
/// `]` first in it (after any `^`) is one of its characters, and so is the `]` that ends a `[:class:]`,
/// `[=e=]` or `[.c.]` inside it, while the delimiter and a backslash stand in it as themselves — a `/` in
/// `/[/#]/q` ends no regexp, and both seds quit there (measured). One left open on its line ends either sed
/// at once with its error.
fn sed_bracket_to(chars: &[char], at: &mut usize) -> bool {
    *at += 1;
    if chars.get(*at) == Some(&'^') {
        *at += 1;
    }
    if chars.get(*at) == Some(&']') {
        *at += 1;
    }
    while let Some(&c) = chars.get(*at) {
        match c {
            ']' => {
                *at += 1;
                return true;
            }
            '\n' => return false,
            '[' if matches!(chars.get(*at + 1), Some('.' | ':' | '=')) => {
                let Some(&kind) = chars.get(*at + 1) else { return false };
                *at += 2;
                loop {
                    match chars.get(*at) {
                        None | Some('\n') => return false,
                        Some(&inner) if inner == kind && chars.get(*at + 1) == Some(&']') => {
                            *at += 2;
                            break;
                        }
                        Some(_) => *at += 1,
                    }
                }
            }
            _ => *at += 1,
        }
    }
    false
}

/// Past a command's line tail, up to the character that ends it.
fn sed_run_to(chars: &[char], at: &mut usize, ends: impl Fn(char) -> bool) {
    while chars.get(*at).is_some_and(|&c| !ends(c)) {
        *at += 1;
    }
}

/// An `s` and its regexp, replacement and flags, or a `y` and its two parts and no flags. The delimiter is
/// any one character but a newline or a backslash, which both seds refuse, a `;`, a `}` and a blank among
/// those both read; both end at once at a missing one or a newline inside a part. The command's end after
/// it is [`sed_bare_end`]'s to read. An `s`'s regexp reads a
/// bracket expression as one, the delimiter inside it included; its replacement and a `y`'s parts do not, on
/// either sed (measured). An empty regexp is refused unread: GNU reuses the last regexp for one and macOS's
/// sed reads no such thing, so what either does with `s//x/` is not a reading this rule vouches for. A `y`
/// whose two sets are not both written and of the same length ends either sed at once with its error. After
/// an `s`'s flags — a count, `g`, `p`, and the case flags `i` and `I`, the ones both seds read — any other
/// letter is no flag both have, GNU's `m`, `M` and run-command `e` among them, or the `w` this reading does
/// not follow.
fn sed_substitute(chars: &[char], at: &mut usize, y: bool) -> bool {
    let Some(&delimiter) = chars.get(*at) else { return false };
    if matches!(delimiter, '\n' | '\\') {
        return false;
    }
    *at += 1;
    let Some(regexp) = sed_part_to(chars, at, delimiter, !y) else { return false };
    if regexp.is_empty() {
        return false;
    }
    let Some(replacement) = sed_part_to(chars, at, delimiter, false) else { return false };
    if y && (replacement.is_empty() || regexp.chars().count() != replacement.chars().count()) {
        return false;
    }
    if !y {
        while chars.get(*at).is_some_and(|&c| matches!(c, '0'..='9' | 'g' | 'p' | 'i' | 'I')) {
            *at += 1;
        }
    }
    !chars.get(*at).is_some_and(|c| c.is_ascii_alphabetic())
}

/// One delimited part of an `s` or a `y`, to its delimiter unescaped, and its text as the sed reads it, a
/// bracket expression read whole where `brackets` says the part is a regexp: a part unterminated, or one
/// broken by a newline, is a script neither sed reads to its end.
fn sed_part_to(chars: &[char], at: &mut usize, delimiter: char, brackets: bool) -> Option<String> {
    let mut part = String::new();
    while let Some(&c) = chars.get(*at) {
        match c {
            '\\' => {
                part.extend(chars.get(*at + 1).copied());
                *at += 2;
            }
            c if c == delimiter => {
                *at += 1;
                return Some(part);
            }
            '\n' => return None,
            '[' if brackets => {
                let began = *at;
                if !sed_bracket_to(chars, at) {
                    return None;
                }
                part.extend(chars.get(began..*at).unwrap_or_default());
            }
            _ => {
                part.push(c);
                *at += 1;
            }
        }
    }
    None
}

/// A word the shell hands sed as its script, as far as this reading vouches: quotes come off as the shell
/// takes them and what an expansion holds stands as [`SED_HOLE`]. Unlike an `eval`'s words a backslash is
/// kept as written: it matters to sed inside a regexp and a replacement, where the parser reads it as the
/// escape of the character after it, and a backslashed hole hides nothing the parser would not refuse
/// anyway; a `$'…'` translates its escapes at the shell, before any of this, so it stands as the hole.
fn script_text(word: &Value) -> String {
    let mut text = String::new();
    spell(word, &SED_HOLE.to_string(), as_written, &mut text);
    text
}

/// A part's text as the tree holds it, where the part is text written out: a bare word or a plain `'…'`.
fn as_written(part: &Value) -> Option<String> {
    match part.get("Type").and_then(Value::as_str) {
        Some("Lit") => Some(part.get("Value").and_then(Value::as_str).unwrap_or_default().to_string()),
        Some("SglQuoted") if part.get("Dollar").and_then(Value::as_bool) != Some(true) => {
            Some(part.get("Value").and_then(Value::as_str).unwrap_or_default().to_string())
        }
        _ => None,
    }
}

/// What a `mapfile` or a `readarray` at a pipe does with it: `-n` counts the lines it copies and lets the
/// writer's rest go unread, and `-u` reads another descriptor and never the pipe. bash writes mapfile's
/// options in single letters alone — a long word is one it reads as none of its own and ends at once, and
/// so is an option this reading does not name, and a word not written out could be `-n` itself. A value an
/// option takes is not weighed: an `-n 0` copies every line after all, and a script wanting the whole of
/// the pipe gives it to mapfile another way.
fn mapfile_verdict(words: &[&Value]) -> Option<&'static str> {
    let mut index = 0;
    let mut ends_options = false;
    while let Some(word) = words.get(index) {
        index += 1;
        let text = literal(word);
        if ends_options {
            // The array's name: no name changes when mapfile reads.
            continue;
        }
        let Some(text) = text else { return Some(MAPFILE_SHAPE) };
        if text == "--" {
            ends_options = true;
            continue;
        }
        if text.starts_with("--") {
            return Some(MAPFILE_SHAPE);
        }
        let Some(flags) = text.strip_prefix('-').filter(|flags| !flags.is_empty()) else {
            continue;
        };
        for (at, flag) in flags.char_indices() {
            match flag {
                't' => {}
                'd' | 'O' | 's' | 'c' => {
                    // Their values stand attached or in the next word; none stops mapfile where `-n` does.
                    if flags.get(at + 1..).unwrap_or_default().is_empty() {
                        index += 1;
                    }
                    break;
                }
                _ => return Some(MAPFILE_SHAPE),
            }
        }
    }
    None
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
        "DblQuoted" => double_quoted_text(part)?,
        _ => return None,
    };
    let plain_single_quotes = kind == "SglQuoted" && part.get("Dollar").and_then(Value::as_bool) != Some(true);
    (plain_single_quotes || !text.contains('\\')).then_some(text)
}

/// What one part of a word contributes to a text that will be read again as a program: an `eval`'s words, a
/// child shell's `-c` string. That program is the text the shell hands on, while the tree holds each part as
/// the script spells it, so a part counts only where the two are one text: a part [`part_text`] vouches for,
/// or a `"…"` whose every backslash the shell hands on as it stands ([`double_quoted_text`]), which keeps
/// `printf 'y\n'` inside a `"…"` the program `printf 'y\n'`. A backslash the shell takes off or translates
/// makes the spelling another program than the one that runs: `bash -o pipefail -c listed\ \|\ grep\ -q\ x`
/// runs a pipe its spelling holds as one escaped word, and `"echo \\'; listed | grep -q x; echo \\'"` hands
/// on `\'`, where the spelling's `\\'` opens a quote that hides the pipe (both measured 141 in bash 3.2). Such
/// a part stands in the spelling as a hole, as an expansion does.
fn program_part_text(part: &Value) -> Option<String> {
    match part.get("Type").and_then(Value::as_str)? {
        "DblQuoted" => double_quoted_text(part),
        _ => part_text(part),
    }
}

/// The text a `"…"` hands on, where its spelling in the tree is that text: its literal parts joined. Inside
/// the quotes the shell takes a backslash off before `$`, `` ` ``, `"`, `\` and a newline alone and hands
/// every other on as it stands (measured in bash 3.2: `"\$ \` \" \\ \a \n"` hands on `$`, `` ` ``, `"`, `\`,
/// `\a` and `\n`), so a backslash before one of those five makes the spelling another text than the one
/// handed on, and no text this reading vouches for; nor is an expansion inside the quotes any text at all.
fn double_quoted_text(part: &Value) -> Option<String> {
    let mut text = String::new();
    for inner in part.get("Parts").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
        if inner.get("Type").and_then(Value::as_str) != Some("Lit") {
            return None;
        }
        text.push_str(inner.get("Value").and_then(Value::as_str)?);
    }
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.next().is_none_or(|escaped| matches!(escaped, '$' | '`' | '"' | '\\' | '\n')) {
            return None;
        }
    }
    Some(text)
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
            ("source /dev/stdin\n", format!("early-pipes: s.sh:1: {SOURCE_STDIN}")),
            ("set -o pipefail\nlisted | sed 1q\n", format!("early-pipes: s.sh:2: {SED_SHAPE}")),
            ("set -o pipefail\nlisted | sed 's/x/y/' names.txt\n", format!("early-pipes: s.sh:2: {SED_FILE}")),
            ("set -o pipefail\nlisted | sed -f prog\n", format!("early-pipes: s.sh:2: {SED_UNREAD}")),
            ("set -o pipefail\nlisted | sed 'b x'\n", format!("early-pipes: s.sh:2: {SED_UNLABELED}")),
            ("set -o pipefail\nlisted | read line\n", format!("early-pipes: s.sh:2: {READ_SHAPE}")),
            ("set -o pipefail\nlisted | mapfile -n 1 lines\n", format!("early-pipes: s.sh:2: {MAPFILE_SHAPE}")),
            ("set -euo pipefail\nexport SHELLOPTS\nbash other.sh\n", format!("early-pipes: s.sh:2: {SHELLOPTS_EXPORTED}")),
            ("bash -c \"$program\"\n", format!("early-pipes: s.sh:1: {CHILD_UNREAD}")),
            ("env SHELLOPTS=pipefail bash other.sh\n", format!("early-pipes: s.sh:1: {CHILD_SHELLOPTS}")),
            ("set -o pipefail\neval 'if then'\n", format!("early-pipes: s.sh:2: {EVAL_UNREAD}")),
            ("set -o pipefail\nlisted | command -v grep\n", format!("early-pipes: s.sh:2: {WRAPPER_RUNS_NOTHING}")),
            ("set -o pipefail\nlisted | bash -n -c 'cat'\n", format!("early-pipes: s.sh:2: {WRAPPER_RUNS_NOTHING}")),
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
                SOURCE_UNSEEN,
                SOURCE_STDIN,
                SED_SHAPE,
                SED_FILE,
                SED_UNREAD,
                SED_UNLABELED,
                READ_SHAPE,
                MAPFILE_SHAPE,
                SHELLOPTS_EXPORTED,
                CHILD_UNREAD,
                CHILD_SHELLOPTS,
                EVAL_UNREAD,
                WRAPPER_RUNS_NOTHING
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
                "a pipe feeds a command named by an expansion, through an escape, or past a wrapper option this reading does not follow — one that takes the command in its value, or one that macOS's wrapper or GNU's has not got — which this reading cannot tell from a reader that quits before its input ends: name its program as a plain word, after options both platforms' wrappers read",
                "a pipe feeds a compound command, which this reading cannot tell from a reader that quits before its input ends: hand it the writer's output with `< <( … )`, or capture the output first",
                "a `set` or `shopt` with an option that is not written out, so this reading cannot tell whether it turns pipefail on or off",
                "a `source` or `.` whose file is named by an expansion with no file name written at its end, so this reading cannot tell which script it brings under pipefail",
                "a `source` or `.` reads its script from the standard input — `/dev/stdin`, a `/dev/fd` or a `/proc/self/fd` name — which is no script of the list this reading reads: the words it runs stand on whatever stream carries them, able to set options, source another file or quit early at a pipe, and no reading here reaches them: source a script file that stands where this reading reads it",
                "a pipe feeds a sed whose script can reach `q` or `Q`, which end sed before its input ends: under pipefail the writer dying of SIGPIPE makes the pipe answer 141 where the reader's answer belongs, so the verdict is timing; capture the writer's whole output and let sed read it from a file or a here-string, or judge the writer itself",
                "a pipe feeds a sed given a file to read, or a word after its script that is not written out and could name one: the sed reads that file and never the pipe, whose writer dies of SIGPIPE, so the pipe answers with the file's verdict or with 141 where the file is not there, and never with what the pipe holds — macOS's sed reads even `-` as a file named `-` and not as the pipe: name no file after the script, or give the script with `-e`",
                "a pipe feeds a sed whose script this reading cannot read whole: read from a file with `-f` or `--file`, held wholly in a word that is not written out, or spelled with an option or a command that macOS's sed or GNU's does not read (macOS's reads no long option at all) or that edits files in place, so for all this reading can tell it reaches `q` or `Q`, or ends at once with its error, before its input ends: write the script out, and every option as both seds read it",
                "a pipe feeds a sed whose scripts branch with `b` or `t` to a label none of that same sed's scripts defines, or define one label twice, as macOS's sed or GNU's reads its labels — GNU's ends a label at a blank or a `;`, macOS's runs it to the line's end: each sed looks every branch label up as it reads the scripts, and macOS's refuses a label defined twice, ending at once with its error before reading any input (measured), so the writer dies half-written and the pipe answers with the sed's error and never with what the pipe holds: define each label once, with a `:name` that ends its line, in one of the same sed's scripts — one sed reads the labels of all its scripts together — end each branch's line at its label, or branch to the end with a bare `b`",
                "a pipe feeds the `read` builtin, which takes one line and leaves the writer's rest unread: under pipefail the writer dying of SIGPIPE makes the pipe answer 141 where the reader's answer belongs, so the verdict is timing; capture the writer's whole output and let the shell read it from a here-string or a file",
                "a pipe feeds a mapfile or readarray told with `-n` how many lines to copy, leaving the writer's rest unread, or told with `-u` to read another descriptor and not the pipe, or given an option this reading cannot see whole, which could be `-n` itself: under pipefail the writer dying of SIGPIPE makes the pipe answer 141 where the reader's answer belongs, so the verdict is timing; capture the writer's whole output and hand it to mapfile as a here-string",
                "a script that runs under pipefail exports SHELLOPTS, which hands the running shell's own options, pipefail among them, to every bash started below it — by name, by a script's `#!` line, as the `sh` macOS runs, or by any program the script starts — whose own pipes then answer 141 by timing where a reader quits early, unseen by any reading of this script: do not export SHELLOPTS",
                "a child shell (`bash`, `sh`, `dash`, `ksh` or `zsh`) was given a program this reading cannot read whole: no `-c` program at all, one held in a word that is not written out, one shfmt cannot parse, an option this reading does not know or one held in a word whose first letter is not written out, a program read from a stream — no program or file at all, which leaves the shell its standard input or a here-document, or `/dev/stdin` — a script file named by an expansion while the child runs with `pipefail` on, or a word naming one that stands behind a wrapper option this reading cannot follow, which may have eaten the word before it and named no command at all — such a shell runs words no reading of this script reaches, and at a pipe's end the shell with no program of its own reads the pipe itself, so its early-quit pipes answer 141 by timing unseen: write the `-c` program and every option out whole where shfmt can parse them, name the script file the shell runs with a plain word, and let no child stand behind an option only one platform's wrapper reads",
                "a command is handed a SHELLOPTS assignment in the environment given it, which starts the shell it runs with the options named there, pipefail among them, unseen by any reading of this script or of the script that shell runs: hand no command a SHELLOPTS assignment; let each script turn on its own options, and take back an inherited export with `export -n SHELLOPTS`, which tools/env.sh does for every script this rule reads",
                "an `eval` whose text shfmt cannot parse, so this reading cannot tell what it runs: whether it turns pipefail on, sources a script, or feeds or reads a pipe that quits early",
                "a pipe feeds a wrapper that runs no command and reads none of the pipe — `command -v` or `-V`, a `builtin` or `command` given an option it has not got, `env -0`, which runs a command on neither platform, `env --help` or `--version`, or a wrapper left with no command after its options — or a child shell reading `-n` or `-D`, which parses what it is given and runs none of it: its writer dies of SIGPIPE, so the pipe answers with what that prints or its error, by timing, and never with what the pipe holds: name the command to run, or let the child shell run its program"
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
        // A hand the reading cannot spell hides the reader too: with `$setup` spelling `GREP_OPTIONS=-q`,
        // `cat big | env "$setup" grep pat` quits at once and the writer dies (measured 141 in bash 3.2),
        // and the word behind the unread one is the command as easily as an assignment before it.
        assert_eq!(one("set -o pipefail\nlisted | env \"$setup\" grep pat -c\n"), [format!("early-pipes: s.sh:2: {NAMED_UNSEEN}")]);
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
    /// them, since the rule's answer is only as good as what it assumes. An inherited `SHELLOPTS` export is
    /// taken out the same way, by one top-level `export -n SHELLOPTS` of env.sh's own.
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
        // The same guarantee for the option hand: env.sh takes back the export attribute on `SHELLOPTS`
        // with one `export -n` of its own top level, so no bash started below the gate runs with options no
        // script's words turned on, and the rule reads that command as the take-back it is.
        let take_backs = env.tree["Stmts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|statement| statement["Cmd"]["Type"] == "DeclClause" && statement["Cmd"]["Variant"]["Value"] == "export")
            .filter(|statement| {
                let fields = statement["Cmd"].as_object().unwrap();
                takes_back(fields)
                    && fields.get("Args").and_then(Value::as_array).is_some_and(|args| {
                        args.iter().any(|arg| arg.get("Name").and_then(|name| name.get("Value")).and_then(Value::as_str) == Some("SHELLOPTS"))
                    })
            })
            .count();
        assert_eq!(take_backs, 1, "tools/env.sh must take SHELLOPTS out of the environment with one top-level `export -n` of its own");
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

    /// What `q` and `Q` do at a pipe, measured in macOS's bash 3.2 and `/usr/bin/sed` and in ubuntu:24.04's
    /// GNU bash and sed: a script that reaches either leaves the writer dead of SIGPIPE and the pipe answering
    /// 141, on either sed or on the ones only GNU reads (`sed 'Q'` and `sed 'q 0'` make macOS's sed error at
    /// once, which kills its writer just the same, so both seds' verdicts on a pipe are the refused one).
    /// Where the two disagree about where a command's text ends, the reading takes the early end too: GNU's
    /// `sed ':x;q'`, `sed ':x q'` and a label ended by a tab quit, and macOS's reads each as one long label.
    /// A bracket expression holds the regexp's delimiter as one of its characters on both seds, so the `q`
    /// after `/[/#]/` is a command, and both quit there.
    #[test]
    fn a_sed_quits_where_its_script_can_reach_q() {
        for quits in [
            "sed 1q",
            "sed q",
            "sed Q",
            "sed 's/a/b/;q'",
            "sed -n '/x/{p;q;}'",
            "sed -eq",
            "sed -e 's/x/y/' -e q",
            "sed -e p -e q",
            "sed --expression q",
            "sed --expression=1q",
            "sed -z q",
            "sed -x q",
            "sed '1!q'",
            "sed '$!q'",
            "sed 'q 0'",
            "sed ':x;q'",
            "sed ':x q'",
            "sed ':x\tq'",
            "sed '/y/b;q'",
            "sed 'x d;q'",
            "sed -n 's/x/y/ p;q'",
            "sed '1{p;}q'",
            "sed '$qx;q'",
            "sed -e '/y/b end' -e q -e ':end'",
            "sed '\\%x%q'",
            "sed -- 's/x/y/;q'",
            "sed \"$lines\"q",
            "sed 5q",
            "sed '/x/Iq'",
            "command sed 1q",
            "sed -n '/[/#]/q'",
            "sed -n '/^[/#]/q'",
            "sed -n '/[/r x]/q'",
            "sed -n '/[/aX]/q'",
            "sed -n '/[]/]/q'",
            "sed -n '/[^]/]/q'",
            "sed -n '/[[:alpha:]/]/q'",
            "sed -n '\\%[%]%q'",
            "sed 's/[/]/X/;q'",
        ] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {quits}\n")), [format!("early-pipes: s.sh:2: {SED_SHAPE}")], "{quits}");
        }
    }

    /// The seds the gate's own scripts pipe through, and their neighbours: a `q` inside a regexp, a
    /// replacement, a `y`'s two sets, a branch label or printed text names no command, and a `q` whose one
    /// address is `$` ends sed only with its input read. Every shape here was read the same whole way by
    /// macOS's sed and by GNU's, and the first four are the gate's own lines (`tools/env.sh` and
    /// `tools/ios-device-test.sh`, whose sed reads the pipe of a `grep` that reads a file, and
    /// `tools/doctor.sh`).
    #[test]
    fn a_sed_whose_script_never_reaches_q_reads_the_pipe_whole() {
        for whole in [
            "sed \"s/^/$name: /\"",
            "sed -n \"${served}s/^.* for \\([0-9][0-9]*\\) seconds[[:space:]]*\\$/\\1/p\"",
            "sed 's#^\\([0-9a-f]*\\) ios/sdk/#\\1 #'",
            "sed 's/^.*\"version\": \"\\([0-9.]*\\)\".*$/\\1/p'",
            "sed 's/q/x/g'",
            "sed '/q/d'",
            "sed 'y/a/q/'",
            "sed 'y/abc/ABC/'",
            "sed -n '/x/{s/x/y/;p;}'",
            "sed '1{p;}'",
            "sed '1{d;}'",
            "sed 'x;d'",
            "sed 'p ; d'",
            "sed -n 'y/x/y/ ; p'",
            "sed 's/x/y/;p'",
            "sed '1{s/x/y/;}'",
            "sed 'p;# c'",
            "sed '1{p;};p'",
            "sed '1{2{p;};}'",
            "sed -n '/x/b'",
            "sed 'r /dev/null;q'",
            "sed 'w /dev/null'",
            "sed 'a\\\nq'",
            "sed 'b'",
            "sed -n '20p'",
            "sed '/x/,/y/d'",
            "sed '$d'",
            "sed '$q'",
            "sed -n '$q'",
            "sed -n '/x/ s/y/z/p'",
            "sed \"${lines}s/x/y/\"",
            "sed -E 's/x/y/'",
            "sed -r 's/x/y/'",
            "sed -u 's/x/y/'",
            "sed 's;x;y;'",
            "sed 's}x}y}'",
            "sed 's x y '",
            "sed 's/x/y/I'",
            "sed '='",
            "sed '/x/Ip'",
            "sed 's,[,],X,'",
            "sed 's/[/#]/Y/'",
            "sed -n '/[/#]/{p;}'",
            "sed -e '/x/b end' -e p -e ':end'",
            "sed -e '/x/b end' -e 's/a/b/' -e ':end'",
            "sed -e 's/a/b/;t done' -e 's/c/d/' -e ':done'",
        ] {
            assert!(one(&format!("set -o pipefail\nlisted | {whole}\n")).is_empty(), "{whole}");
        }
        // A `q` the parser only sees through delimiters, labels and addresses would be missed; a `y`'s two
        // sets of one letter read whole while their unequal lengths (`y/ab/c/`, which errors on both seds)
        // are refused unread rather than trusted.
        assert_eq!(one("set -o pipefail\nlisted | sed 'y/ab/c/'\n"), [format!("early-pipes: s.sh:2: {SED_UNREAD}")]);
    }

    /// A script the reading cannot finish is refused unread, and a file named where the pipe would be read
    /// is refused as read in place of it — `-` included, which macOS's sed opens as a file named `-` while
    /// GNU's reads the pipe (measured: the writer dies of SIGPIPE before the first one either way). So is a
    /// script one of the two seds ends at once with its error, measured on both: every long option, which
    /// macOS's sed reads none of; GNU's own `-z`, `-s`, `Q`, `R`, `W`, `l 5`, `1~2`, `s///m` and one-line
    /// `a text`; macOS's own `-a` and `-H`; a command chained after another without a `;` (`sed 'x d'`,
    /// `sed '= p'`, `sed '{p}'`, `sed '1{s/x/y/}'`, `sed 'y/x/y/ p'`), a branch with more after its label on
    /// its line (`sed '/x/b;p'`, whose label macOS's sed reads as `;p` and has not got); unbalanced braces
    /// (`sed '{'`, `sed '}'`); and what neither reads (`&`, `s///u`).
    #[test]
    fn a_sed_read_from_a_file_an_expansion_or_no_known_option_is_refused_unread_and_a_named_file_as_read() {
        for unread in [
            "sed -f prog",
            "sed --file prog",
            "sed --file=prog",
            "sed -e 's/x/y/' -f prog",
            "sed \"$script\"",
            "sed ''",
            "sed -",
            "sed -i",
            "sed -z 's/x/y/'",
            "sed -s 's/x/y/'",
            "sed -l 's/x/y/'",
            "sed -a 's/x/y/'",
            "sed -H 's/x/y/'",
            "sed --help",
            "sed --version",
            "sed --quiet 's/x/y/p'",
            "sed --silent 's/x/y/p'",
            "sed --posix 's/x/y/'",
            "sed --regexp-extended 's/x/y/'",
            "sed --null-data 's/x/y/'",
            "sed --expression='s/x/y/'",
            "sed --expression 's/x/y/'",
            "sed '&'",
            "sed 'R /dev/null'",
            "sed 'W /dev/null'",
            "sed 'l 5'",
            "sed '1~2d'",
            "sed 'aq'",
            "sed 'a foo;q'",
            "sed 'a\\foo;q'",
            "sed 's/x/y/m'",
            "sed 's/x/y/u'",
            "sed '$Q'",
            "sed '$q5'",
            "sed '$q }'",
            "sed 'x d'",
            "sed '= p'",
            "sed '{p}'",
            "sed '}'",
            "sed '{'",
            "sed 'p 3'",
            "sed 'p # c'",
            "sed '1{2{p;}}'",
            "sed '1{s/x/y/}'",
            "sed 's/x/y/#c'",
            "sed 'y/x/y/ p'",
            "sed -n '/x/b;p'",
            "sed -n '/x/t;p'",
            "sed -e '/x/b end;p' -e ':end'",
            "sed '1{p;b}'",
            "sed 's/x/y/ p'",
            "sed 's/x/y/ ; p'",
            "sed -n '/x[/q'",
            "sed 's/x/[/]/'",
            "sed 'y/[/]/abc/'",
            "sed '\\\nx\nq'",
        ] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {unread}\n")), [format!("early-pipes: s.sh:2: {SED_UNREAD}")], "{unread}");
        }
        // macOS's sed ends its options at its first operand, so every word after a script given as one is a
        // file it opens in place of the pipe, an option word and `--` among them, and GNU's, given an `-e`
        // after it, opens that first operand as a file in turn (each measured: 141 1 on macOS's, and
        // `can't read …` or no branch target on GNU's 4.10 where it is no option to it).
        for file in [
            "sed 's/x/y/' names.txt",
            "sed -e 's/x/y/' names.txt",
            "sed 's/x/y/' -",
            "sed 's/x/y/' \"$file\"",
            "sed \"$script\" names.txt",
            "sed p -n",
            "sed p --",
            "sed ':x' -e 'b x'",
            "sed 's/a/b/;t done' -e ':done'",
            "sed \"$script\" -n",
            "sed -- 's/a/b/' -n",
        ] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {file}\n")), [format!("early-pipes: s.sh:2: {SED_FILE}")], "{file}");
        }
        // After `--` both seds read what follows as an operand, so a word spelling an option there is the
        // script, and one neither sed can run ends it at once (`invalid command code -`, `unknown command:
        // -`, measured on macOS's sed and GNU's 4.10 alike, before either reads the pipe).
        for after in ["sed -- -n", "sed -- -x"] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {after}\n")), [format!("early-pipes: s.sh:2: {SED_UNREAD}")], "{after}");
        }
        // A sed reading the pipe itself stays whole even with odd words around it, and an unquoted expansion
        // standing wholly where the script goes is the unread one, not a silent pass.
        assert!(one("set -o pipefail\nlisted | sed 's/x/y/'\n").is_empty());
    }

    /// `read` takes one line of a pipe and leaves the writer to die of SIGPIPE, and a `mapfile` or
    /// `readarray` does the same when `-n` counts its lines or `-u` points it at another descriptor;
    /// measured in bash 3.2 (which has no mapfile at all) and GNU bash 5.2, and read whole otherwise:
    /// the option table is bash 5.2's own, written with single letters only, and a word that is not one of
    /// them either fails (`--help` exits 2) or could be `-n` itself.
    #[test]
    fn read_and_mapfile_leave_their_writers_unread() {
        for read in ["read line", "read -r a b", "builtin read line", "builtin -- read line"] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {read}\n")), [format!("early-pipes: s.sh:2: {READ_SHAPE}")], "{read}");
        }
        for mapfile in [
            "mapfile -n 1 lines",
            "readarray -n 1 lines",
            "mapfile -n",
            "mapfile -u 3",
            "mapfile \"$count\" lines",
            "mapfile --help",
            "mapfile -q",
            "mapfile -C cb arr",
        ] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {mapfile}\n")), [format!("early-pipes: s.sh:2: {MAPFILE_SHAPE}")], "{mapfile}");
        }
        for whole in [
            "mapfile lines",
            "mapfile -t lines",
            "readarray -t lines",
            "mapfile -s 2 lines",
            "mapfile -O 1 -t lines",
            "mapfile -- lines",
            "command mapfile -t lines",
        ] {
            assert!(one(&format!("set -o pipefail\nlisted | {whole}\n")).is_empty(), "{whole}");
        }
    }

    /// A wrapper that runs no command reads none of the pipe it is fed: `command -v` exits at once (measured:
    /// the writer dies of SIGPIPE and the pipe answers 141 in bash 3.2 and 5.2), and bash reads `builtin -p`
    /// as an option `builtin` has not got. So does a wrapper left with no command after its options: `env`
    /// prints the environment, `exec` does nothing, `command` and `nice` print their usage, each without
    /// reading a line (measured: the writer dies and the pipe answers by timing). The same reading of a
    /// wrapper's options finds the command past `--` and `command -p`, past `exec -a` — which runs the
    /// command after its argv0 — and past attached values like `nice -n1` and `env -uFOO`; options bundled
    /// in one cluster the way getopt reads it, `env -iu NOPE`; a bare `-`, which both `env`s consume as
    /// one of their own options and run the word behind (measured); and macOS's `env -P` past the path it
    /// takes. And an option that takes the command in its value hides it: `env -S 'head -n 1'` runs head
    /// on macOS's env and GNU's alike, measured at 141, as does GNU's `--split-string`.
    #[test]
    fn a_wrapper_is_read_past_its_options_and_one_that_runs_nothing_is_refused() {
        for nothing in [
            "command -v grep",
            "command -V cat",
            "command -pv cat",
            "builtin -p cat",
            "command -x cat",
            "env",
            "exec",
            "command",
            "nice -n 5",
            "nohup",
            "env --help",
            "env -0 cat",
            "env --null cat",
            "env --help cat",
            "env --version cat",
        ] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {nothing}\n")), [format!("early-pipes: s.sh:2: {WRAPPER_RUNS_NOTHING}")], "{nothing}");
        }
        for hidden in [
            "env -S 'head -n 1'",
            "env -S 'sed 1q'",
            "env --split-string 'head -n 1'",
            "env --split-string=head -n 1",
            "env --split 'head -n 1'",
            "env --un FOO head -n 1",
            "env --unset=FOO head -n 1",
            "env --unset=FOO cat",
            "env --unset FOO cat",
            "env --chdir=/ cat",
            "env --ignore-environment cat",
            "exec -x head -n 1",
            "nice --adjustment=5 head -n 1",
        ] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {hidden}\n")), [format!("early-pipes: s.sh:2: {NAMED_UNSEEN}")], "{hidden}");
        }
        for (reader, verdict) in [
            ("command -- head -n 1", HEAD_SHAPE),
            ("command -p head -n 1", HEAD_SHAPE),
            ("command -p -- read x", READ_SHAPE),
            ("exec -a zzz head -n 1", HEAD_SHAPE),
            ("exec -a zzz sed 1q", SED_SHAPE),
            ("exec -l head -n 1", HEAD_SHAPE),
            ("nice -n 1 head -n 1", HEAD_SHAPE),
            ("nice -n1 head -n 1", HEAD_SHAPE),
            ("nice head -n 1", HEAD_SHAPE),
            ("nohup -- head -n 1", HEAD_SHAPE),
            ("env -i head -n 1", HEAD_SHAPE),
            ("env -uFOO head -n 1", HEAD_SHAPE),
            ("env -C / head -n 1", HEAD_SHAPE),
            ("env -u FOO head -n 1", HEAD_SHAPE),
            ("env -C/ head -n 1", HEAD_SHAPE),
            ("env -iu NOPE head -n 1", HEAD_SHAPE),
            ("env -iuNOPE head -n 1", HEAD_SHAPE),
            // A bare `-` runs the word behind it on both `env`s, its `--` included: GNU's consumes one
            // even past `--` and runs `head` at 141 (measured) where macOS runs a program named `-`,
            // and the reading follows the env that runs a command; macOS's dead end over-refused is the
            // safe half. `-P` takes the path on the one env that has it and runs the rest behind.
            ("env - head -n 1", HEAD_SHAPE),
            ("env -- - head -n 1", HEAD_SHAPE),
            ("env -P /bin head -n 1", HEAD_SHAPE),
        ] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {reader}\n")), [format!("early-pipes: s.sh:2: {verdict}")], "{reader}");
        }
        // Read whole past options both platforms' wrappers read, each measured 0 on macOS; a bare `-`
        // is read as an option and runs the command behind on both, and `-P` is read taking its path
        // because macOS runs `cat` behind it (GNU 9.11 has no `-P` at all, a dead end either reading
        // can only pass), while a cluster of flags is getopt's on both and ends at the command.
        for whole in [
            "command -p cat",
            "exec -a zzz cat",
            "env -C / cat",
            "env -C/ cat",
            "env -u FOO cat",
            "env -uFOO cat",
            "env -i cat",
            "env -v cat",
            "env -- cat",
            "env - cat",
            "env -P /bin cat",
            "env -iu NOPE cat",
            "env -iuNOPE cat",
            "nice -n 5 cat",
            "nohup cat",
        ] {
            assert!(one(&format!("set -o pipefail\nlisted | {whole}\n")).is_empty(), "{whole}");
        }
    }

    /// bash's own stand-ins stand in the settings commands: `builtin set`, `command set`, `command -p set`,
    /// either past `--`, and an `eval` of a `set` each turn `pipefail` on, and `builtin source`,
    /// `command .` and an `eval` of a `source` each source. An `eval` is read as the script its words spell,
    /// through shfmt as every script is, so a `set` after a `;`, inside braces or an `if`, or with its option
    /// quoted turns it on as bash turns it on. What runs nothing runs no setting: `command -v` and `-V`
    /// print, and bash reads `builtin -p` as naming no command that runs. All measured in bash 3.2 and 5.2.
    #[test]
    fn the_settings_stand_through_the_shell_standins_and_eval() {
        for on in [
            "builtin set -o pipefail",
            "command set -o pipefail",
            "command -p set -o pipefail",
            "command -- set -o pipefail",
            "builtin -- set -o pipefail",
            "command -p -- set -o pipefail",
            "builtin shopt -s -o pipefail",
            "eval 'set -o pipefail'",
            "eval set -o pipefail",
            "eval 'set -euo pipefail'",
            "eval 'command set -o pipefail'",
            "eval 'builtin set -o pipefail'",
            "eval 'true; set -o pipefail'",
            "eval 'set -o pipefail; true'",
            "eval \"set -o 'pipefail'\"",
            "eval '{ set -o pipefail; }'",
            "eval 'if true; then set -o pipefail; fi'",
            "eval 'x=1 set -o pipefail'",
            "eval eval set -o pipefail",
            "builtin eval 'set -o pipefail'",
        ] {
            assert_eq!(one(&format!("{on}\nlisted | grep -q x\n")).len(), 1, "{on}");
        }
        for off in [
            "command -v set -o pipefail",
            "command -V set -o pipefail",
            "command -pv set -o pipefail",
            "builtin -p set -o pipefail",
            "eval true set -o pipefail",
            "eval 'echo set -o pipefail'",
            "command echo set -o pipefail",
        ] {
            assert!(one(&format!("{off}\nlisted | grep -q x\n")).is_empty(), "{off}");
        }
        // An `eval` whose words are not all written out could spell `set -o pipefail` itself, so its shell
        // is read under `pipefail`; what does show of it is read as far as it goes, and a `set` or `source`
        // it shows unread is refused as that command is. `tools/env.sh` runs `eval "$exports"` in a script
        // already under `pipefail`, where it changes nothing.
        assert_eq!(one("eval \"$x\"\nlisted | grep -q x\n"), [format!("early-pipes: s.sh:2: {GREP_SHAPE}")]);
        // So is one whose spelling holds a backslash the shell takes off before `eval` reads the text: the
        // text handed on is not the one the tree spells (measured in bash 3.2: `eval \$x` and `eval "\$x"`
        // of `x='set -o pipefail'` both put the next `grep -q` pipe at 141), while a backslash the shell
        // hands on inside `"…"` leaves the spelling the text, and reads whole.
        for escaped in ["eval \\$x", "eval \"\\$x\"", "eval $'\\x24x'"] {
            assert_eq!(one(&format!("{escaped}\nlisted | grep -q x\n")), [format!("early-pipes: s.sh:2: {GREP_SHAPE}")], "{escaped}");
        }
        assert!(one("eval \"printf 'y\\n'\"\nlisted | grep -q x\n").is_empty());
        assert_eq!(one("set -o pipefail\neval \"set -o $option\"\n"), [format!("early-pipes: s.sh:2: {SET_UNSEEN}")]);
        assert_eq!(one("set -o pipefail\neval \"source $library\"\n"), [format!("early-pipes: s.sh:2: {SOURCE_UNSEEN}")]);
        assert!(one("set -o pipefail\neval \"$exports\"\nlisted | grep -c pat\n").is_empty());
        let opaque = script("tools/lib.sh", "# shellcheck shell=bash\nlib_env() { eval \"$exports\"; }\n");
        let caller = script("tools/caller.sh", "# shellcheck source=tools/lib.sh\nsource \"$(dirname \"$0\")/lib.sh\"\nlisted | grep -q x\n");
        let (problems, under) = check(&[opaque, caller]);
        assert_eq!(under, 2);
        assert_eq!(problems, [format!("early-pipes: tools/caller.sh:3: {GREP_SHAPE}")]);
        // What an `eval` runs is read as the script's own: its pipes are judged at the `eval`'s line, an
        // `eval` at a pipe's end is read by the command it runs first, and the variables it sets are set.
        assert_eq!(one("set -o pipefail\neval 'listed | head -n 1'\n"), [format!("early-pipes: s.sh:2: {HEAD_SHAPE}")]);
        assert_eq!(one("set -o pipefail\nlisted | eval 'head -n 1'\n"), [format!("early-pipes: s.sh:2: {HEAD_SHAPE}")]);
        assert_eq!(one("set -o pipefail\nlisted | eval \"$reader\"\n"), [format!("early-pipes: s.sh:2: {NAMED_UNSEEN}")]);
        assert_eq!(one("set -o pipefail\nlisted | eval 'read a; read b'\n"), [format!("early-pipes: s.sh:2: {COMPOUND_READER}")]);
        assert!(one("set -o pipefail\nlisted | eval 'cat'\n").is_empty());
        assert_eq!(one("set -o pipefail\neval 'export GREP_OPTIONS=-q'\nlisted | grep -c pat\n"), [format!("early-pipes: s.sh:3: {GREP_ENV}")]);
        // `eval`s nested in `eval`s are read to a depth no script reaches, and one nested past it is refused
        // rather than read without end.
        let nested = |evals: usize| format!("{}set -o pipefail\nlisted | grep -q x\n", "eval ".repeat(evals));
        assert_eq!(one(&nested(EVAL_DEPTH)), [format!("early-pipes: s.sh:2: {GREP_SHAPE}")]);
        assert_eq!(one(&nested(EVAL_DEPTH + 1)), [format!("early-pipes: s.sh:1: {EVAL_UNREAD}")]);
        // The stand-ins source through the one definition too: a library reached by `builtin source`,
        // `command .`, either past `--`, or an `eval` of a `source` stands under `pipefail` and answers for its
        // own greps.
        for (by, line) in [
            ("builtin source", "builtin source \"$(dirname \"$0\")/lib.sh\""),
            ("command .", "command . \"$(dirname \"$0\")/lib.sh\""),
            ("command -- .", "command -- . \"$(dirname \"$0\")/lib.sh\""),
            ("builtin -- source", "builtin -- source \"$(dirname \"$0\")/lib.sh\""),
            ("eval of a source", "eval 'true; source \"$(dirname \"$0\")/lib.sh\"'"),
        ] {
            let lib = script("tools/lib.sh", "# shellcheck shell=bash\nset -o pipefail\nlib_greps() { listed | grep -q x; }\n");
            let sourcer = script("tools/sourcer.sh", &format!("# shellcheck source=tools/lib.sh\n{line}\nlisted | head -n 1\n"));
            let (problems, under) = check(&[lib, sourcer]);
            assert_eq!(under, 2, "{by}");
            assert_eq!(problems, [format!("early-pipes: tools/lib.sh:3: {GREP_SHAPE}"), format!("early-pipes: tools/sourcer.sh:3: {HEAD_SHAPE}")], "{by}");
        }
    }

    /// A script that runs under `pipefail` and exports `SHELLOPTS` hands the setting to every bash started
    /// below it, at any depth: measured in bash 3.2 and 5.2, a `grep -q` pipe answers 141 inside a bash
    /// started by name, by a script's `#!` line, as `sh` on macOS and through `xargs`, while the same bash
    /// below a script that never exported it answers 0. Which commands start a bash no reading of a script
    /// can list, so the export is refused at its own line; a bare assignment marks it as an `export` does, as
    /// `GREP_OPTIONS` and friends already do.
    #[test]
    fn a_script_handing_its_shell_options_to_every_bash_below_it_is_refused_at_its_export() {
        for export in
            ["export SHELLOPTS", "export PATH SHELLOPTS", "declare -x SHELLOPTS", "SHELLOPTS=$x", "f() { export SHELLOPTS; }", "eval 'export SHELLOPTS'"]
        {
            assert_eq!(one(&format!("set -o pipefail\n{export}\n")), [format!("early-pipes: s.sh:2: {SHELLOPTS_EXPORTED}")], "{export}");
        }
        // Not without the setting, and not without the export; the line is named once however often it
        // names the variable.
        for off in ["export SHELLOPTS\nbash other.sh", "set -o pipefail\nbash other.sh", "set -o pipefail\nexport BASHOPTS"] {
            assert!(one(&format!("{off}\n")).is_empty(), "{off}");
        }
        assert_eq!(one("set -o pipefail\nexport SHELLOPTS SHELLOPTS\n"), [format!("early-pipes: s.sh:2: {SHELLOPTS_EXPORTED}")]);
        // The export is refused wherever its shell runs under `pipefail`, however the `source`s between them
        // run: a library exporting it in a caller's shell, and a caller exporting it for a library.
        let lib = script("tools/lib.sh", "# shellcheck shell=bash\nexport SHELLOPTS\nbash lib.sh\n");
        let caller = script("tools/caller.sh", "set -o pipefail\n# shellcheck source=tools/lib.sh\nsource \"$(dirname \"$0\")/lib.sh\"\n");
        let (problems, under) = check(&[lib, caller]);
        assert_eq!(under, 2);
        assert_eq!(problems, [format!("early-pipes: tools/lib.sh:2: {SHELLOPTS_EXPORTED}")]);
        let lib = script("tools/lib.sh", "# shellcheck shell=bash\nbash lib.sh\n");
        let caller = script("tools/caller.sh", "set -o pipefail\nexport SHELLOPTS\n# shellcheck source=tools/lib.sh\nsource \"$(dirname \"$0\")/lib.sh\"\n");
        let (problems, under) = check(&[lib, caller]);
        assert_eq!(under, 2);
        assert_eq!(problems, [format!("early-pipes: tools/caller.sh:2: {SHELLOPTS_EXPORTED}")]);
    }

    /// A child shell's own words are read through the one tree: its `-c` script's pipes answer at the
    /// shell's line as the running script's own, and the options its words turn on stand for the script too.
    /// Measured in bash 3.2: the same `grep -q` pipe answers 141 inside `bash -o pipefail -c` and 0 inside
    /// a bare `bash -c`, and only the child's own words cross the boundary — what the parent sets stays in
    /// the parent and what the child sets stays in the child — so reading an option that turns the child on
    /// as turning the script on over-marks at worst, never wrongly passes.
    #[test]
    fn a_child_shells_program_is_read_through_the_one_tree() {
        for shell in ["bash", "sh", "dash", "ksh", "zsh"] {
            assert_eq!(under(&format!("{shell} -c 'listed | grep -q x'")), 1, "{shell}");
            assert_eq!(one(&format!("set -o pipefail\n{shell} -c 'listed | grep -q x'\n")), [format!("early-pipes: s.sh:2: {GREP_SHAPE}")], "{shell}");
        }
        // The child's `-o` answers for the string's pipes and for the script's own alike.
        assert_eq!(one("bash -o pipefail -c 'listed | grep -q x'\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        assert_eq!(one("bash -eo pipefail -c \"listed | grep -q x\"\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        assert_eq!(one("bash +o pipefail -c 'listed | grep -q x'\n").len(), 0, "a child turned off hands the string to the script's own reading");
        // A `"…"` string stands before the child exactly as the child reads it: the quotes fell to the outer
        // shell, and every backslash in what they held — one it takes off or one it hands on — is text of
        // that program, so a `printf 'y\n'` inside a `"…"` is as read whole as one written in `'…'`.
        assert_eq!(one("set -o pipefail\nbash -c \"printf 'y\\n' | grep -q x\"\n"), [format!("early-pipes: s.sh:2: {GREP_SHAPE}")]);
        // An option this reading cannot decide reads the child on, and its readable string is judged.
        assert_eq!(one("bash -o \"$option\" -c 'listed | grep -q x'\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        // The string's own `set` stands for the script and its pipes, as an `eval`'s does.
        assert_eq!(one("bash -c 'set -o pipefail\nlisted | grep -q x'\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        // A child at a pipe's end is read through its program: the string's first command reads the pipe,
        // a string of two statements is the compound command it is, and a parse-only child runs nothing.
        assert_eq!(under("listed | bash -c 'grep -q x'"), 1);
        assert_eq!(under("listed | bash -c 'cat x; head -1'"), 1);
        assert_eq!(one("set -o pipefail\nlisted | bash -c 'cat x; head -1'\n"), [format!("early-pipes: s.sh:2: {COMPOUND_READER}")]);
        assert_eq!(one("set -o pipefail\nlisted | bash -n -c 'cat'\n"), [format!("early-pipes: s.sh:2: {WRAPPER_RUNS_NOTHING}")]);
        assert_eq!(one("set -o pipefail\nlisted | bash -D\n"), [format!("early-pipes: s.sh:2: {WRAPPER_RUNS_NOTHING}")]);
        // `-c` is a switch, and the program the first word past every option, wherever the `-c` stands; an
        // `o` takes the next word as its name wherever it stands in its word. Measured in bash 3.2, and in
        // macOS's `sh` and `zsh` for the first: each runs the string's `grep -q` pipe under `pipefail`, 141.
        for options in ["-c -o pipefail", "-co pipefail", "-of pipefail -c", "-c -e -o pipefail --", "-ec -o pipefail"] {
            assert_eq!(one(&format!("bash {options} 'listed | grep -q x'\n")), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")], "{options}");
        }
        assert!(one("bash -c -e 'listed | grep -q x'\n").is_empty(), "the string runs with the child's own defaults");
        assert_eq!(one("set -o pipefail\nlisted | bash -c -e 'head -n 1'\n"), [format!("early-pipes: s.sh:2: {HEAD_SHAPE}")]);
        // A child started through a wrapper runs its program as one started by name does (measured 141 for
        // each of `env`, `exec`, `nice -n 5` and `nohup`), and only the wrappers that run a builtin read a
        // `set` past them: `env set -o pipefail` runs no `set` of this shell.
        for wrapper in ["env", "env -i", "exec", "nice -n 5", "nohup", "command", "env nice"] {
            assert_eq!(one(&format!("{wrapper} bash -o pipefail -c 'listed | grep -q x'\n")), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")], "{wrapper}");
            assert_eq!(one(&format!("{wrapper} bash -c \"$program\"\n")), [format!("early-pipes: s.sh:1: {CHILD_UNREAD}")], "{wrapper}");
        }
        assert!(one("env set -o pipefail\nlisted | grep -q x\n").is_empty());
        // A script file the child runs with `pipefail` on runs its pipes unseen by the file's own reading,
        // measured 141, so the file is marked as a `source` marks one; without the option nothing stands
        // across the boundary, and the file keeps its own reading, the gate's own runners unchanged.
        for start in ["bash -o pipefail tools/other.sh", "nice -n 5 bash -o pipefail tools/other.sh"] {
            let (problems, marked) = check(&[script("tools/other.sh", "listed | grep -q x\n"), script("s.sh", &format!("{start}\n"))]);
            assert_eq!(marked, 2, "{start}");
            assert_eq!(problems, [format!("early-pipes: tools/other.sh:1: {GREP_SHAPE}")], "{start}");
        }
    }

    /// A word not written out says nothing about itself: past any wrapper it stands for one of the wrapper's
    /// options, or an `env` assignment, as easily as for the command, and where a child's options are read it
    /// stands for one of them as easily as for the program. Measured in bash 3.2: with `V=1`, and with the
    /// assignment among the words `$vars` stands for, `env "$V=1" bash -o pipefail -c '…'`, `env "${vars[@]}" …`
    /// and `env "$V=1" nice -n 5 bash -o pipefail -c '…'` each answer 141 at the string's `grep -q` pipe, and
    /// so do `nice "$o" bash …` with `-n5`, `exec "$o" bash …` with `-l` and `command "$o" bash …` with `-p`,
    /// so every wrapper's words are read on past such a word; a child's own unread word ends them unread, and
    /// the script is read under `pipefail`, the safe way, with no file marked past the unread words.
    #[test]
    fn a_child_option_or_wrapper_assignment_held_in_a_word_not_written_out_is_read_as_turning_the_child_on() {
        // A wrapper's own words go on past one that is not written out, which stands for an `env` assignment
        // or an option's value as easily as it stands for the command itself.
        for hidden in ["env \"$V=1\"", "env \"${vars[@]}\"", "env \"$V=1\" nice -n 5", "nice \"$o\"", "exec \"$o\"", "command \"$o\"", "nohup \"$o\""] {
            assert_eq!(one(&format!("{hidden} bash -o pipefail -c 'listed | grep -q x'\n")), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")], "{hidden}");
        }
        // `command "$o" set -o pipefail` with `-p` turns the setting on (measured 141 at the next pipe).
        assert_eq!(one("command \"$o\" set -o pipefail\nlisted | grep -q x\n"), [format!("early-pipes: s.sh:2: {GREP_SHAPE}")]);
        // A script started behind an unread `env` word runs under whatever that word hands it, and with
        // `SHELLOPTS=pipefail` its own `grep -q` pipe answers 141 (measured, with macOS's `env` and GNU's), so
        // the script it names is marked as a child shell's file is.
        let (problems, marked) = check(&[script("tools/other.sh", "listed | grep -q x\n"), script("s.sh", "env \"$setup\" tools/other.sh\n")]);
        assert_eq!((problems, marked), (vec![format!("early-pipes: tools/other.sh:1: {GREP_SHAPE}")], 2));
        // At a pipe's end the reader behind such a word is unknown, behind every wrapper alike.
        for reader in ["nohup \"$dir/bin/cat\"", "command \"$o\" cat", "nice \"$o\" cat", "exec \"$tool\""] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {reader}\n")), [format!("early-pipes: s.sh:2: {NAMED_UNSEEN}")], "{reader}");
        }
        // Past `env`'s `--`, which ends its options but not its assignments, an unread word is the same
        // hand: `env -- SHELLOPTS=pipefail bash other.sh` runs what it names under the handed options
        // (measured 141 on macOS's `env` and on GNU's alike), and `env -- "$setup" …` could spell that
        // assignment as easily as anything else, so the words after it are read as the words before are.
        assert_eq!(one("env -- \"$setup\" bash -o pipefail -c 'listed | grep -q x'\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        let (problems, marked) = check(&[script("tools/other.sh", "listed | grep -q x\n"), script("s.sh", "env -- \"$setup\" tools/other.sh\n")]);
        assert_eq!((problems, marked), (vec![format!("early-pipes: tools/other.sh:1: {GREP_SHAPE}")], 2));
        // A written assignment stands as one behind `--` on both `env`s too, so the command after it is
        // read: `env -- FOO=1 grep -q x` at a pipe's end is that grep (measured 141 on both `env`s), and
        // the written child after an assignment that hands no option reads its own defaults as if the
        // assignment were not there at all.
        assert_eq!(one("set -o pipefail\nlisted | env -- FOO=1 grep -q x\n"), [format!("early-pipes: s.sh:2: {GREP_SHAPE}")]);
        // Past `--` a name beginning with `-` is an assignment on both `env`s too (measured: `env -- -x=1`
        // sets `-x`, and the child after it runs its string at 141), where before `--` it is an option.
        assert_eq!(one("env -- -x=1 bash -o pipefail -c 'listed | grep -q x'\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        assert_eq!(one("set -o pipefail\nlisted | env -- -x=1 grep -q x\n"), [format!("early-pipes: s.sh:2: {GREP_SHAPE}")]);
        // And past `--` a word beginning with `-` that holds no `=` is the command's own name, never an option:
        // `env -- -S 'head -n 1'` runs no `head` but a program named `-S`, which is not there, and the pipe
        // answers that 127 on every run (measured on both `env`s), a verdict of the reader's own failure.
        assert!(one("set -o pipefail\nlisted | env -- -S 'head -n 1'\n").is_empty());
        // Every other wrapper takes no assignment, so its `--` hands over the command at once: the word after
        // it is the command whatever it holds, and `nice -- "$dir/bin/cat"` at a pipe's end is the `cat` its
        // tail names, where `nice "$dir/bin/cat"`, with no `--`, could stand for an option and is refused.
        assert!(one("set -o pipefail\nlisted | nice -- \"$dir/bin/cat\"\n").is_empty());
        assert!(one("env -- FOO=1 bash -c 'listed | grep -q x'\n").is_empty());
        // A name before the `=` may even be empty: GNU's `env` sets such a word and runs the command behind
        // it wherever it stands — before `--` and past it alike, at a pipe's end and in a child alike
        // (measured 141 in all four) — where macOS's `env` answers 1 having run nothing. So the word is no
        // command on either, and the command after it is read: the refusal stands for what GNU runs and
        // over-refuses macOS's dead end at worst. A bare `=` hands the same empty name (measured 141 too).
        assert_eq!(one("set -o pipefail\nlisted | env -- =x=1 grep -q x\n"), [format!("early-pipes: s.sh:2: {GREP_SHAPE}")]);
        assert_eq!(one("set -o pipefail\nlisted | env =x=1 grep -q x\n"), [format!("early-pipes: s.sh:2: {GREP_SHAPE}")]);
        assert_eq!(one("set -o pipefail\nlisted | env = grep -q x\n"), [format!("early-pipes: s.sh:2: {GREP_SHAPE}")]);
        assert_eq!(one("env -- =x=1 bash -o pipefail -c 'listed | grep -q x'\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        assert_eq!(one("env =x=1 bash -o pipefail -c 'listed | grep -q x'\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        // `env`'s first assignment ends its options as its `--` does, on both `env`s (measured): past it
        // `-x=1` is an assignment and the child after it runs at 141, while `-i`, `-S` and `--` are the
        // command's own name, a program that is not there, whose pipe answers 127 on every run.
        assert_eq!(one("env A=1 -x=1 bash -o pipefail -c 'listed | grep -q x'\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        assert_eq!(one("env \"$v\" -x=1 bash -o pipefail -c 'listed | grep -q x'\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        for named in ["env A=1 -S 'head -n 1'", "env A=1 -- head -n 1"] {
            assert!(one(&format!("set -o pipefail\nlisted | {named}\n")).is_empty(), "{named}");
        }
        assert!(one("env A=1 -i bash -o pipefail -c 'listed | grep -q x'\n").is_empty(), "the shell is an argument of a program named -i");
        env_bare_dash_cluster_and_hidden_children();
        // Where a child's options are read, a word not written out says nothing about the option it could be
        // and ends them unread: whatever follows is the child's own, however the word is quoted.
        for hidden in ["bash \"$script\"", "bash \"$script\" \"${run_args[@]}\"", "bash \"$script\" -- \"$@\"", "bash \"${dir}other.sh\""] {
            assert_eq!(one(&format!("{hidden}\n")), [format!("early-pipes: s.sh:1: {CHILD_UNREAD}")], "{hidden}");
            let (problems, marked) = check(&[script("tools/other.sh", "listed | grep -q x\n"), script("s.sh", &format!("{hidden}\n"))]);
            assert_eq!(problems, [format!("early-pipes: s.sh:1: {CHILD_UNREAD}")], "{hidden}");
            assert_eq!(marked, 1, "{hidden}: the script is read under pipefail, the safe way, and nothing past the words this reading cannot read is marked");
        }
    }

    /// The bare `-`, the cluster of options, and the children a hidden option hides, all measured on
    /// both `env`s; kept apart from the test above so each stays readable at its own length.
    fn env_bare_dash_cluster_and_hidden_children() {
        // A bare `-` keeps an option's standing on both `env`s, GNU's past its `--` included: each
        // consumes one and runs the child behind it (measured: `env - bash …` and `env -- - bash …`
        // answer 141 on GNU, the first on macOS too, where past `--` macOS runs a program named `-`),
        // so the child's string is read. An assignment ends the standing on both: past it `-` is the
        // command and the shell after it is its argument (measured: 127, macOS likewise).
        assert_eq!(one("env - bash -o pipefail -c 'listed | grep -q x'\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        assert_eq!(one("env -- - bash -o pipefail -c 'listed | grep -q x'\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        assert!(one("env A=1 - bash -o pipefail -c 'listed | grep -q x'\n").is_empty(), "the shell is an argument of a program named -");
        assert!(one("env -- A=1 - bash -o pipefail -c 'listed | grep -q x'\n").is_empty(), "the shell is an argument of a program named -");
        // A cluster of options is getopt's on both `env`s and both read it whole: `-iu NOPE` is `-i`
        // and `-u` with its value, and the child behind runs at 141 (measured on both; glued too).
        assert_eq!(one("env -iu NOPE bash -o pipefail -c 'listed | grep -q x'\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        assert_eq!(one("env -iuNOPE bash -o pipefail -c 'listed | grep -q x'\n"), [format!("early-pipes: s.sh:1: {GREP_SHAPE}")]);
        // Where an option this reading cannot follow may have eaten the word naming the command, the
        // settings reading refuses the line unread as one reading a child it cannot reach, rather than
        // walk past it in silence: GNU's `env` reads every abbreviation of `--unset` as taking a value
        // and runs the child behind it at 141 (measured), where macOS ends unread on the same word.
        for unread in [
            "env --un FOO bash -o pipefail -c 'listed | grep -q x'",
            "env --ignore-environment bash -o pipefail -c 'listed | grep -q x'",
            "env -S bash -o pipefail -c 'listed | grep -q x'",
            // Behind every wrapper alike: `nice -5`, GNU's `nice --adjustment=5` and bash's `exec -cl` each
            // run the child's string at 141 (measured), and `nohup --help` runs nothing, refused as the safe
            // half.
            "nice -5 bash -o pipefail -c 'listed | grep -q x'",
            "nice --adjustment=5 bash -o pipefail -c 'listed | grep -q x'",
            "exec -cl bash -o pipefail -c 'listed | grep -q x'",
            "nohup --help bash -o pipefail -c 'listed | grep -q x'",
        ] {
            assert_eq!(one(&format!("{unread}\n")), [format!("early-pipes: s.sh:1: {CHILD_UNREAD}")], "{unread}");
        }
        // At a pipe's end the same child is the pipe's reader, and both readings refuse the one command at
        // its line: told once, as the unread child, while a hidden option with no child behind it is the
        // command named past an option this reading does not follow.
        assert_eq!(one("set -o pipefail\nlisted | env --un FOO bash -c 'cat'\n"), [format!("early-pipes: s.sh:2: {CHILD_UNREAD}")]);
        assert_eq!(one("set -o pipefail\nlisted | env --un FOO cat\n"), [format!("early-pipes: s.sh:2: {NAMED_UNSEEN}")]);
    }

    /// A child shell whose program this reading cannot read whole is refused in every script, turned on or
    /// not: what such a shell runs — a `-c` string not written out or unparseable, an option this reading
    /// does not know, a `-c` glued to its letters, a file named only by an expansion or a stream under
    /// `pipefail` — stands below the boundary, and at a pipe's end the shell with no program reads the pipe
    /// itself. A `-c` string whose spelling holds a backslash the shell takes off or translates is another
    /// program than the one the child runs (measured 141 in bash 3.2 under the child's `-o pipefail`, each
    /// spelling hiding a `grep -q` pipe its tree does not show), and an option in a word whose first letter
    /// is not written out could turn the child on unseen, whether the word can stand for several words or only
    /// for one.
    #[test]
    fn a_child_shell_reading_a_program_this_rule_cannot_read_is_refused() {
        for unread in [
            "bash -c",
            "bash -c \"$program\"",
            "bash -c 'if then'",
            "bash -c\"echo hi\"",
            "bash --posix -c 'cat'",
            "bash -Q -c 'cat'",
            "bash -o pipefail -c listed\\ \\|\\ grep\\ -q\\ x",
            "bash -o pipefail -c \"echo \\\\'; listed | grep -q x; echo \\\\'\"",
            "bash -o pipefail -c $'listed \\x7c grep -q x'",
            "bash -c \"\\$reader\"",
            "bash -oo pipefail errexit -c 'cat'",
            "bash $flags tools/other.sh",
            "bash \"${options[@]}\" tools/other.sh",
            "bash \"$@\"",
            "bash -c -o pipefail",
            // Options held in a word the reading cannot spell, however it is quoted: each of these runs the
            // `grep -q` pipe under the child's `pipefail` (measured 141 in bash 3.2, in macOS's `sh` and in
            // `zsh`), because the word stands for `-c` or `-o` and the words after it are the child's options.
            "bash \"-$flag\" -o pipefail 'listed | grep -q x'",
            "bash \"$flag\" -o pipefail 'listed | grep -q x'",
            "bash \"$flag\" pipefail -c 'listed | grep -q x'",
            "bash \"$bundle\" pipefail 'listed | grep -q x'",
            "bash \"$s\" \"$f\" \"$c\" 'listed | grep -q x'",
            "sh \"-$flag\" -o pipefail 'listed | grep -q x'",
            // bash reads a word beginning with `-` as its options wherever it stands before the program, so a
            // spelled word beginning so is never the program (measured: `bash -c "-\x"` answers
            // `-\: invalid option`, running nothing).
            "bash -c \"-\\x\"",
        ] {
            assert_eq!(one(&format!("{unread}\n")), [format!("early-pipes: s.sh:1: {CHILD_UNREAD}")], "{unread}");
            assert_eq!(one(&format!("set -o pipefail\n{unread}\n")).len(), 1, "{unread}: refused once, by the settings, not twice");
        }
        // A file named by an expansion runs unseen only when the child's words put it under `pipefail`, and
        // so does a stream, which carries no script of the list.
        assert_eq!(one("bash -o pipefail \"$library\"\n"), [format!("early-pipes: s.sh:1: {CHILD_UNREAD}")]);
        assert_eq!(one("bash -o pipefail -- \"$library\"\n"), [format!("early-pipes: s.sh:1: {CHILD_UNREAD}")]);
        // A child reading its program from a stream is refused under its defaults too: the stream is no
        // script of the list, and what it runs can turn `pipefail` on itself (measured: `bash <<EOF` and
        // `bash < file` of a `set -o pipefail` and a `grep -q` pipe answer 141, as `bash -o pipefail -`
        // and `bash -o pipefail /dev/stdin` do of the pipe alone).
        for stream in [
            "bash -o pipefail /dev/stdin",
            "bash -o pipefail -",
            "bash -o pipefail /dev/fd/3 3<tools/other.sh",
            "bash <<'EOF'\nset -o pipefail\nEOF",
            "bash <tools/other.sh",
            "sh -e",
            "bash /dev/stdin",
            "env bash -",
        ] {
            assert_eq!(one(&format!("{stream}\n")), [format!("early-pipes: s.sh:1: {CHILD_UNREAD}")], "{stream}");
        }
        // A lone `-` ends the options, and the file after it is the child's.
        assert!(one("bash - tools/other.sh\n").is_empty());
        // At a pipe's end both readings refuse the same unreadable child at the same line: one problem.
        assert_eq!(one("set -o pipefail\nlisted | bash -c \"$program\"\n"), [format!("early-pipes: s.sh:2: {CHILD_UNREAD}")]);
        assert_eq!(one("set -o pipefail\nlisted | bash --posix\n"), [format!("early-pipes: s.sh:2: {CHILD_UNREAD}")]);
    }

    /// A command handed a `SHELLOPTS` assignment in the environment given it starts its shell with the
    /// options named there: measured in bash 3.2, `env SHELLOPTS=pipefail bash -c 'big | grep -q x'`
    /// answers 141 where the same child handed nothing answers 0. Bash refuses a prefix assignment to its
    /// own readonly variable and hands nothing (measured, and in macOS's `sh`, which is bash), but dash,
    /// ksh or zsh running the line hands it whole (measured 141 under zsh), and which commands a hand
    /// reaches no reading can list: the hand is refused whole, whatever command its word is given to, as the
    /// export is.
    #[test]
    fn a_command_handed_shell_options_is_refused_at_the_hand() {
        for (handed, line) in [
            ("env SHELLOPTS=pipefail bash other.sh", 1),
            ("env SHELLOPTS=pipefail sh -c 'cat'", 1),
            ("env SHELLOPTS=pipefail ./gate.sh", 1),
            ("env -i SHELLOPTS=pipefail bash other.sh", 1),
            ("sudo SHELLOPTS=pipefail bash other.sh", 1),
            ("SHELLOPTS=errex bash other.sh", 1),
            ("SHELLOPTS=errex ./gate.sh", 1),
            ("set -o pipefail\nenv SHELLOPTS=errexit bash -c 'cat'", 2),
        ] {
            assert_eq!(one(&format!("{handed}\n")), [format!("early-pipes: s.sh:{line}: {CHILD_SHELLOPTS}")], "{handed}");
        }
        // A word not written out before the command stands where an assignment stands and spells one as
        // easily: with `$setup` spelling `SHELLOPTS=pipefail`, `env "$setup" bash -c 'big | grep -q x'` and
        // `env "$setup" bash other.sh` both answer 141 (measured in bash 3.2), with no word of the line
        // naming the hand. No reading can tell that word from an option's value or the command, so the shell
        // it starts is read under `pipefail`, and what it runs with it: the pipe is answered where it lives.
        // A word an option has taken as its value is no hand: `-u` and `-C` name what they unset and where
        // they go, and run the command still.
        for (hidden, holds_the_pipe, marks) in [
            ("env \"$setup\" bash -c 'listed | grep -q x'", "s.sh:1", 1),
            ("env -i \"$setup\" bash other.sh", "tools/other.sh:1", 2),
            ("env \"$setup\" nice -n 5 bash other.sh", "tools/other.sh:1", 2),
        ] {
            let (problems, marked) = check(&[script("tools/other.sh", "listed | grep -q x\n"), script("s.sh", &format!("{hidden}\n"))]);
            assert_eq!(problems, [format!("early-pipes: {holds_the_pipe}: {GREP_SHAPE}")], "{hidden}");
            assert_eq!(marked, marks, "{hidden}: the shell is read as turned on, and with it the script and whatever it runs");
        }
        for not_hand in ["env -u \"$name\" bash other.sh", "env -C \"$dir\" bash other.sh", "nice -n \"$n\" bash other.sh"] {
            let (problems, marked) = check(&[script("tools/other.sh", "listed | grep -q x\n"), script("s.sh", &format!("{not_hand}\n"))]);
            assert_eq!((problems.as_slice(), marked), ([].as_slice(), 0), "{not_hand}");
        }
        // A hand taken back is not a hand: `env -u` keeps the name out of the child's environment, and the
        // readings take-back forms stand for the environment every script this rule reads runs in.
        for taken in [
            "env -u SHELLOPTS bash other.sh",
            "set -o pipefail\nexport -n SHELLOPTS\nbash other.sh",
            "set -o pipefail\ndeclare +x SHELLOPTS\nbash other.sh",
            "set -o pipefail\ntypeset +x SHELLOPTS\nbash other.sh",
            "set -o pipefail\nexport -n PATH SHELLOPTS\nbash other.sh",
        ] {
            assert!(one(&format!("{taken}\n")).is_empty(), "{taken}");
        }
        // What claims the export attribute stands as the export it claims: `-x` beside `-n` reads as an
        // export, and bash's nameref spelling is no taking-back.
        assert_eq!(one("set -o pipefail\nexport -n -x SHELLOPTS\n"), [format!("early-pipes: s.sh:2: {SHELLOPTS_EXPORTED}")]);
        assert_eq!(one("set -o pipefail\ndeclare -n SHELLOPTS\nbash other.sh\n"), [format!("early-pipes: s.sh:2: {SHELLOPTS_EXPORTED}")]);
    }

    /// A `source` or `.` that reads its script from the standard input runs words no reading reaches:
    /// measured in bash 3.2, `echo 'set -o pipefail' | source /dev/stdin` turns the setting on unseen, and
    /// bash opens process substitutions under `/dev/fd` and `/proc/self/fd` alike. No script of the list
    /// is on a stream; every other file a `source` names reads as before, written out or unseen.
    #[test]
    fn a_source_reading_its_script_from_the_stream_is_refused() {
        for stream in ["/dev/stdin", "/dev/fd/0", "/dev/fd/3", "/proc/self/fd/0", "/proc/self/fd/1"] {
            assert_eq!(one(&format!("source {stream}\n")), [format!("early-pipes: s.sh:1: {SOURCE_STDIN}")], "{stream}");
            assert_eq!(one(&format!(". {stream}\n")), [format!("early-pipes: s.sh:1: {SOURCE_STDIN}")], "{stream}");
        }
        // Wherever the shell that reads it runs, the reading included: the words stand at the line of the
        // command that runs them, as an `eval`'s do.
        assert_eq!(one("set -o pipefail\nbash -c 'source /dev/stdin'\n"), [format!("early-pipes: s.sh:2: {SOURCE_STDIN}")]);
        assert_eq!(one("eval 'source /dev/stdin'\n"), [format!("early-pipes: s.sh:1: {SOURCE_STDIN}")]);
        // Files read as before: unseen whole, or marked by a name its expansion leaves at the end.
        assert_eq!(one("source \"$file\"\n"), [format!("early-pipes: s.sh:1: {SOURCE_UNSEEN}")]);
        assert!(one("source \"$HOME/outside/gate.sh\"\n").is_empty());
        assert!(one("source \"$(dirname \"$0\")/lib.sh\"\n").is_empty());
    }

    /// A `sed` branch to a label none of the same sed's scripts defines ends both seds at once with their
    /// error before either reads a line: measured, `sed 'b x'` and `sed 't x'` end macOS's sed with 1 and
    /// GNU's 4.10 with 4 at once, and leave the writer to die half-written. One sed pools the labels of all
    /// its scripts, in either order and before the definition, measured `sed -e 'b x' -e ':x'` answering 0
    /// on both; a bare `b` and `t` branch to the end of the script, where no label stands. Each sed reads a
    /// label its own way — GNU's to a blank, a `;` or a `}`, macOS's to the line's end, blanks and all — and
    /// macOS's refuses a label defined twice, so a set of scripts passes only where both readings resolve
    /// every branch: `-e 'b x' -e ':x;p'`, `-e 'b x' -e ':x '`, `-e 'b x ' -e ':x'` and
    /// `-e 'b x' -e ':x' -e ':x'` each end macOS's sed at once (measured 141 1) while GNU's reads them whole.
    #[test]
    fn a_sed_branch_to_a_label_no_script_of_the_sed_defines_is_refused() {
        assert_eq!(under("listed | sed 'b x'"), 1);
        assert_eq!(one("set -o pipefail\nlisted | sed 'b x'\n"), [format!("early-pipes: s.sh:2: {SED_UNLABELED}")]);
        assert_eq!(one("set -o pipefail\nlisted | sed -n 't x'\n"), [format!("early-pipes: s.sh:2: {SED_UNLABELED}")]);
        assert_eq!(
            one("set -o pipefail\nlisted | sed -e 'b x' -e q\n"),
            [format!("early-pipes: s.sh:2: {SED_SHAPE}")],
            "a script that quits is refused for that first"
        );
        for macos_alone in [
            "sed -n -e 'b x' -e ':x;p'",
            "sed -e 'b x' -e ':x '",
            "sed -e 'b x' -e ':x\t'",
            "sed -e 'b x ' -e ':x'",
            "sed -e 'b x' -e ':x' -e ':x'",
            "sed 'b x\n:x\n:x'",
        ] {
            assert_eq!(one(&format!("set -o pipefail\nlisted | {macos_alone}\n")), [format!("early-pipes: s.sh:2: {SED_UNLABELED}")], "{macos_alone}");
        }
        // The label of one script of the sed stands for all of them, in any order and in one script whole,
        // and a label both seds read alike stands however its blanks fall: both pass over the blanks before
        // it, and a branch whose line ends in the same blanks as the label's names it on macOS's sed too.
        for defined in [
            "sed -e 'b x' -e ':x'",
            "sed -e '/zz/b x' -e p -e ':x'",
            "sed 'b x\n:x'",
            "sed -e 's/a/b/;t done' -e ':done'",
            "sed -e 'b x' -e ': x'",
            "sed -e 'b  x' -e ':x'",
            "sed -e 'b x ' -e ':x '",
            "sed -n -e 'b x' -e ':x' -e ':y'",
        ] {
            assert!(one(&format!("set -o pipefail\nlisted | {defined}\n")).is_empty(), "{defined}");
        }
        // A `:` naming no label ends both seds at once (`empty label`, `":" lacks a label`, measured).
        assert_eq!(one("set -o pipefail\nlisted | sed ':'\n"), [format!("early-pipes: s.sh:2: {SED_UNREAD}")]);
        // A bare branch needs no label; an unread script is refused unread, labels and all.
        assert!(one("set -o pipefail\nlisted | sed -n '/x/b'\n").is_empty());
        assert!(one("set -o pipefail\nlisted | sed 's/a/b/;t'\n").is_empty());
        assert_eq!(one("set -o pipefail\nlisted | sed -e '/x/b end;p' -e ':end'\n"), [format!("early-pipes: s.sh:2: {SED_UNREAD}")]);
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
