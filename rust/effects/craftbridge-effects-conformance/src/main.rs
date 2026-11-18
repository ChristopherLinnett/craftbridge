//! Runs the EffectCraft conformance corpus (`conformance/effects/corpus.toml`, plan §28) on the EffectCraft
//! command line, the oracle, and on the plug-in, `craftbridge-effects`, and compares what each produces with
//! what the corpus says it must and with each other: values in the JSON output, documents, and files byte
//! for byte or, for an encoded export, frame by frame within the 40 dB PSNR of D-12.
//!
//! ```text
//! craftbridge-effects-conformance --oracle <effectcraft-cli> [--corpus <corpus.toml>] [--scenario <id>]…
//! craftbridge-effects-conformance --list [--corpus <corpus.toml>]
//! ```
//!
//! `tools/oracle.sh` builds the oracle from the pinned official revision. The plug-in runs as this runner
//! started again for each step (`--plugin-step`, which is not for a person), so both sides are processes
//! in the one environment `oracle.rs` makes. Exit status: 0 when every scenario passed, 1 when one failed
//! (each is named), 2 for a bad command line or corpus.

// Every line goes through `say!` or `complain!`: `print!`, `dbg!` and their kin panic on a closed pipe,
// and the runner's output is piped, so a line written any other way would crash it where its reader stops
// early. `forbid`, not `deny`: an item may lift a `deny` (the workspace's own `dbg_macro` is one), and no
// item in this crate has a print to justify — the same reason §3 sets the panicking shortcuts to `forbid`
// rather than `deny`. Clippy does not look inside a macro this crate defines, so the two macros' own
// bodies are held by `tests/closed_pipes.rs`.
#![forbid(clippy::print_stdout, clippy::print_stderr, clippy::dbg_macro)]

mod check;
mod corpus;
mod documents;
mod frames;
mod hold;
mod host;
mod oracle;
mod plugin;
#[cfg(test)]
mod testing;
mod timings;

use corpus::Corpus;
use oracle::Cli;
use plugin::Plugin;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

/// A line on standard output. `println!` panics when stdout cannot be written (a reader that closed the pipe,
/// as `| head` does), and the runner never crashes (AGENTS.md §3): a line nobody reads is dropped, and the
/// scenarios still run and decide the exit status.
macro_rules! say {
    ($($t:tt)*) => {{
        let _ = writeln!(std::io::stdout().lock(), $($t)*);
    }};
}

/// A line on standard error, dropped when it cannot be written, for the same reason as [`say!`].
macro_rules! complain {
    ($($t:tt)*) => {{
        let _ = writeln!(std::io::stderr().lock(), $($t)*);
    }};
}

/// Longest a single step of either side may take. The slowest one in the corpus is an export the plug-in
/// encodes in the runner's debug build, under a minute on a loaded machine; a step this far past that is
/// hung.
const STEP_TIMEOUT: Duration = Duration::from_secs(300);

struct Options {
    oracle: Option<PathBuf>,
    corpus: PathBuf,
    only: Vec<String>,
    list: bool,
    /// The plan of one step the plug-in makes, which is what this process then does (see `plugin.rs`).
    plugin_step: Option<String>,
}

/// The corpus beside the repository's other platform-independent data: this crate is at
/// `<root>/rust/effects/<name>`, found from the manifest folder so the runner finds the same file wherever
/// it is started.
fn default_corpus() -> PathBuf {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = folder.ancestors().nth(3).unwrap_or(folder);
    root.join("conformance/effects/corpus.toml")
}

/// The arguments as the system gives them: a path is any bytes the system allows, and a flag or a scenario
/// id that is no UTF-8 is an error the runner names, never the panic `std::env::args` would raise.
fn parse_options(args: impl IntoIterator<Item = OsString>) -> Result<Options, String> {
    let mut options = Options { oracle: None, corpus: default_corpus(), only: Vec::new(), list: false, plugin_step: None };
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let flag = flag.into_string().map_err(|flag| format!("unknown argument {flag:?}"))?;
        let mut value = |what: &str| args.next().ok_or_else(|| format!("{flag} needs {what}"));
        match flag.as_str() {
            "--oracle" => options.oracle = Some(PathBuf::from(value("the path of the EffectCraft command line")?)),
            "--corpus" => options.corpus = PathBuf::from(value("the path of corpus.toml")?),
            "--scenario" => {
                let id = value("a scenario id")?.into_string().map_err(|id| format!("scenario id {id:?} is not UTF-8, as every corpus id is"))?;
                options.only.push(id);
            }
            "--list" => options.list = true,
            plugin::FLAG => {
                let plan = value("the step's calls")?.into_string().map_err(|_| "the step's calls are not UTF-8".to_string())?;
                options.plugin_step = Some(plan);
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    if options.oracle.is_none() && !options.list && options.plugin_step.is_none() {
        return Err("--oracle <effectcraft-cli> is required (tools/oracle.sh builds it from the pin)".to_string());
    }
    Ok(options)
}

fn run(options: &Options) -> Result<bool, String> {
    let text = std::fs::read_to_string(&options.corpus).map_err(|error| format!("cannot read {}: {error}", options.corpus.display()))?;
    let corpus = Corpus::parse(&text).map_err(|error| format!("{}: {error}", options.corpus.display()))?;
    if let Some(unknown) = options.only.iter().find(|id| !corpus.scenarios.iter().any(|scenario| &scenario.id == *id)) {
        return Err(format!("the corpus has no scenario {unknown}"));
    }
    let chosen = corpus.scenarios.iter().filter(|scenario| options.only.is_empty() || options.only.contains(&scenario.id));
    if options.list {
        chosen.for_each(|scenario| say!("{}  {}", scenario.id, scenario.title));
        return Ok(true);
    }
    let Some(binary) = &options.oracle else { return Err("--oracle is required".to_string()) };
    // Absolute, because every step runs in its own scratch folder, where a relative path leads nowhere.
    let binary = std::fs::canonicalize(binary).map_err(|error| format!("cannot find the oracle {}: {error}", binary.display()))?;
    let oracle = Cli { binary, timeout: STEP_TIMEOUT };
    let me = std::env::current_exe().map_err(|error| format!("cannot find the runner's own binary: {error}"))?;
    let plugin = Plugin(Cli { binary: me, timeout: STEP_TIMEOUT });
    let sides = check::Sides { oracle: &oracle, plugin: &plugin };
    // The corpus's own folder, which a scenario's fixtures are read from.
    let folder = options.corpus.parent().unwrap_or(Path::new("."));
    let (mut ran, mut failed) = (0_usize, Vec::new());
    for scenario in chosen {
        ran += 1;
        let failures = check::run(&sides, scenario, folder);
        if failures.is_empty() {
            say!("conformance: {} ok: {}", scenario.id, scenario.title);
        } else {
            say!("conformance: {} FAILED: {}", scenario.id, scenario.title);
            failures.iter().for_each(|failure| say!("  {}: {failure}", scenario.id));
            failed.push(scenario.id.as_str());
        }
    }
    if failed.is_empty() {
        say!("conformance: ok ({ran} scenarios)");
    } else {
        say!("conformance: {} of {ran} scenarios failed: {}", failed.len(), failed.join(", "));
    }
    Ok(failed.is_empty())
}

/// One step of the plug-in's: the calls in `plan` made on an app, what the step prints written to standard
/// output. The step's lines are read from standard input only when a call answers them.
fn step_host(plan: &str) -> Result<(), String> {
    let stdin = std::io::read_to_string(std::io::stdin()).map_err(|error| format!("cannot read the step's input: {error}"))?;
    let container = std::env::current_dir().map_err(|error| format!("no working folder: {error}"))?;
    let config = std::env::var_os(oracle::CONFIG_VARIABLE).map(PathBuf::from).ok_or_else(|| format!("{} is not set", oracle::CONFIG_VARIABLE))?;
    say!("{}", host::run(plan, &stdin, &container, config)?);
    Ok(())
}

fn main() -> ExitCode {
    let options = match parse_options(std::env::args_os().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            complain!("craftbridge-effects-conformance: {error}");
            return ExitCode::from(2);
        }
    };
    if let Some(plan) = &options.plugin_step {
        return match step_host(plan) {
            Ok(_) => ExitCode::SUCCESS,
            Err(error) => {
                complain!("craftbridge-effects-conformance: {error}");
                ExitCode::from(2)
            }
        };
    }
    match run(&options) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            complain!("craftbridge-effects-conformance: {error}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<Options, String> {
        parse_options(args.iter().map(OsString::from))
    }

    #[test]
    fn the_command_line_names_an_oracle_unless_it_only_lists() {
        assert!(parsed(&[]).err().is_some_and(|error| error.contains("--oracle")));
        assert!(parsed(&["--list"]).is_ok_and(|options| options.list));
        let options = parsed(&["--oracle", "cli", "--scenario", "T1", "--scenario", "T5", "--corpus", "c.toml"]).expect("parses");
        assert_eq!((options.only, options.corpus), (vec!["T1".to_string(), "T5".to_string()], PathBuf::from("c.toml")));
    }

    #[test]
    fn a_plug_in_step_needs_its_calls_and_no_oracle() {
        let options = parsed(&["--plugin-step", "{\"host\":{},\"calls\":[]}"]).expect("parses without an oracle");
        assert_eq!(options.plugin_step.as_deref(), Some("{\"host\":{},\"calls\":[]}"));
        assert!(parsed(&["--plugin-step"]).err().is_some_and(|error| error.contains("needs the step's calls")));
    }

    #[test]
    fn a_flag_without_its_value_or_unknown_is_refused() {
        assert!(parsed(&["--oracle"]).err().is_some_and(|error| error.contains("needs")));
        assert!(parsed(&["--oracle", "cli", "--bogus"]).err().is_some_and(|error| error.contains("--bogus")));
    }

    /// A byte string no UTF-8 decoder accepts, as a shell passes one through.
    #[cfg(unix)]
    fn not_utf8(prefix: &str) -> OsString {
        use std::os::unix::ffi::OsStringExt;
        let mut bytes = prefix.as_bytes().to_vec();
        bytes.push(0xff);
        OsString::from_vec(bytes)
    }

    #[test]
    #[cfg(unix)]
    fn an_argument_that_is_no_utf8_is_an_error_for_a_flag_or_an_id_and_still_a_path() {
        let args = |list: Vec<OsString>| parse_options(list);
        let id = args(vec!["--oracle".into(), "cli".into(), "--scenario".into(), not_utf8("T")]);
        assert!(id.err().is_some_and(|error| error.contains("not UTF-8")));
        assert!(args(vec![not_utf8("--oracle")]).err().is_some_and(|error| error.contains("unknown argument")));
        let path = args(vec!["--oracle".into(), not_utf8("/opt/cli")]).expect("a path is any bytes");
        assert_eq!(path.oracle.map(PathBuf::into_os_string), Some(not_utf8("/opt/cli")));
    }

    #[test]
    fn the_default_corpus_is_the_repositorys() {
        assert!(default_corpus().ends_with("conformance/effects/corpus.toml") && default_corpus().is_file(), "{}", default_corpus().display());
    }
}
