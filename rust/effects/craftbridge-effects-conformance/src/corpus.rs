//! The corpus file, `conformance/effects/corpus.toml`: what every platform's runner reads, so a scenario is
//! written once. A scenario is a list of steps (each one invocation of the oracle, the command line, and the
//! calls that make the same step on the plug-in), run in order in one scratch folder per side, and what to
//! check afterwards.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Corpus {
    // Defaulted, like `steps`, so a corpus that is empty is reported as that and not as a missing key.
    #[serde(default, rename = "scenario")]
    pub(crate) scenarios: Vec<Scenario>,
}

/// Which sides run a scenario. Most run on both and are compared; the plug-in's own policy has no command
/// line to compare with, and a scenario the plug-in cannot answer yet runs on the oracle alone.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Sides {
    #[default]
    Both,
    Oracle,
    Plugin,
}

impl Sides {
    pub(crate) fn runs_oracle(self) -> bool {
        self != Sides::Plugin
    }

    pub(crate) fn runs_plugin(self) -> bool {
        self != Sides::Oracle
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    /// The plan's identifier (`T5`), which every failure names.
    pub(crate) id: String,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) sides: Sides,
    /// Files put in each side's folder before its first step.
    #[serde(default, rename = "fixture")]
    pub(crate) fixtures: Vec<Fixture>,
    #[serde(default, rename = "step")]
    pub(crate) steps: Vec<Step>,
    /// Values the JSON output of every side must hold.
    #[serde(default, rename = "expect")]
    pub(crate) expects: Vec<Expect>,
    /// Outputs of the two sides that must be the same document.
    #[serde(default, rename = "same")]
    pub(crate) sames: Vec<Same>,
    /// Files the steps wrote, the side under test's held against the oracle's.
    #[serde(default, rename = "compare")]
    pub(crate) compares: Vec<Compare>,
    /// What each side's folders must hold when the steps are done.
    #[serde(default, rename = "folder")]
    pub(crate) folders: Vec<Folder>,
}

/// The names a folder of every side that ran holds when its steps are done, and no others: a cancelled
/// export leaves no file, and a queue writes the items it ticked and none it did not. `path` is relative
/// to the side's scratch folder, `.` the scratch folder itself. A name that starts with a dot is the side's
/// own working file (its configuration, cache and captured output) and is not counted; a folder that is not
/// there is a failure, never an empty one, so a misspelt path cannot pass.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Folder {
    pub(crate) path: String,
    pub(crate) holds: Vec<String>,
}

/// A file the scenario starts with: `from`, relative to the corpus file's folder, is copied to `to`,
/// relative to the side's scratch folder.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fixture {
    pub(crate) from: String,
    pub(crate) to: String,
    /// The file is text that says [`CONTAINER`] where the side's own folder goes, as an argument does: a
    /// values file names a picture by the path its side reads it at, which no fixture can spell for both.
    #[serde(default)]
    pub(crate) names_container: bool,
}

/// One step: an invocation of the oracle (the arguments after its name) and the calls the plug-in makes to
/// do the same. Each runs in the scenario's scratch folder, so a relative path is relative to that folder.
/// Every step starts the side afresh, so what one step leaves for the next is in files.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    #[serde(default)]
    pub(crate) args: Vec<String>,
    /// Lines the side reads on its standard input: what `mcp` answers.
    #[serde(default)]
    pub(crate) stdin: Vec<String>,
    /// The command line ends this step with a failure status and still prints its answer, as `replay` does
    /// for a script that fails. A step that ends well then is a failure; the plug-in has no status.
    #[serde(default)]
    pub(crate) fails: bool,
    /// How the plug-in's app is built for this step.
    #[serde(default)]
    pub(crate) host: Host,
    /// The plug-in's calls, in order, on one app; the step's output is the last call's.
    #[serde(default, rename = "call")]
    pub(crate) calls: Vec<Call>,
}

/// What an app is given besides the container.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Host {
    /// The plug-in folder the app names, relative to the container; plug-ins are off without one (PD13).
    #[serde(default)]
    pub(crate) plugins: Option<String>,
    /// The app is handed the machine's GPU before its first call.
    #[serde(default)]
    pub(crate) gpu: bool,
}

/// A frame's format, as `craftbridge_core::PixelFormat` names the two the corpus asks for.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FrameFormat {
    Png,
    /// Straight 8-bit RGBA, written out as a PNG, which holds those pixels as they are.
    Rgba8,
}

/// One call on the plug-in's app.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum Call {
    /// `CraftApp::request`: the method and its parameters; each `files` entry puts the text of a file
    /// in the parameter of that name. With `progress` the call is `CraftApp::request_with_progress`, which
    /// is how `export` and `template.render` run.
    Request {
        method: String,
        #[serde(default = "no_params")]
        params: Value,
        #[serde(default)]
        files: BTreeMap<String, String>,
        #[serde(default)]
        progress: Option<Progress>,
    },
    /// `CraftApp::frame`: the frame written to `out`.
    Frame {
        #[serde(default = "no_params")]
        params: Value,
        format: FrameFormat,
        out: String,
    },
    /// `CraftApp::mcp`: every line of the step's standard input, each reply on a line of output.
    Mcp {},
}

/// A call that reports progress, and how its listener answers.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Progress {
    /// The fraction of the frames done at which the listener answers `false` to stop the call; the call
    /// runs to its end without one. The call's export is held at that point until the stop reaches it
    /// ([`crate::hold`]), so the stop never meets a call that has already finished, and a call whose export
    /// that stop did not release from there fails its step.
    #[serde(default)]
    pub(crate) cancel_at: Option<f64>,
}

impl Progress {
    /// Whether the listener stops the call at `done` of `total` frames. A call that has not said how many
    /// frames it has is not stopped, since half of nothing is no place to stop at.
    pub(crate) fn cancels(&self, done: u64, total: u64) -> bool {
        self.cancel_at.is_some_and(|at| total > 0 && done as f64 >= at * total as f64)
    }
}

/// What a step's arguments, calls and input say where the side's own folder goes: the plug-in takes
/// absolute paths, and the scenario's folder is not known until it runs. Both sides are given the folder
/// the same way, so a path the command line prints back is the one the plug-in prints.
pub(crate) const CONTAINER: &str = "<container>";

/// `text` with the side's folder written where it says [`CONTAINER`].
pub(crate) fn in_folder(text: &str, folder: &Path) -> String {
    text.replace(CONTAINER, &folder.display().to_string())
}

fn no_params() -> Value {
    Value::Object(serde_json::Map::new())
}

/// The value at `pointer` (RFC 6901) in a JSON document a step printed on standard output equals `equals`,
/// or holds `length` items.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Expect {
    /// The step's index, from 0.
    pub(crate) step: usize,
    /// Which document of the step's output, from 0: `mcp` prints one per reply.
    #[serde(default)]
    pub(crate) output: usize,
    pub(crate) pointer: String,
    pub(crate) equals: Option<toml::Value>,
    pub(crate) length: Option<usize>,
}

/// The JSON the string at `pointer` holds, with these pointers' values taken out of it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Within {
    pub(crate) pointer: String,
    pub(crate) omit: Vec<String>,
}

/// In every entry of the array at `pointer` whose string `key` starts with `prefix`, the values at `omit`
/// (pointers into the entry): what two sides say differently of a few entries, the rest of each entry
/// still compared.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entries {
    pub(crate) pointer: String,
    pub(crate) key: String,
    pub(crate) prefix: String,
    pub(crate) omit: Vec<String>,
}

/// The two sides' output of a step compared.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum Same {
    /// The documents are equal, numbers by value.
    Output {
        step: usize,
        /// Values taken out of every document of both sides first, by RFC 6901 pointer. One that is
        /// missing is no error: the sides may each leave out a value the other has.
        #[serde(default)]
        omit: Vec<String>,
        /// Values taken out of some entries of an array in the same way.
        #[serde(default)]
        entries: Vec<Entries>,
        /// Values taken out of a document a string holds, which a tool's reply carries as text.
        #[serde(default)]
        within: Vec<Within>,
    },
    /// The same runs, layers, effects and playback were timed. The command line prints its benchmark as
    /// text, and the plug-in answers JSON: the structure is read out of each, and no measurement is compared.
    Timings { step: usize },
}

impl Same {
    pub(crate) fn step(&self) -> usize {
        match self {
            Same::Output { step, .. } | Same::Timings { step } => *step,
        }
    }
}

/// A comparison of two files the steps wrote: `reference` the oracle's, which it is taken to have right, and
/// `actual` the side under test's. A scenario that runs on one side alone has both from that side's folder.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum Compare {
    /// The bytes are the same; of two folders, they hold the same files with the same bytes.
    Identical { actual: String, reference: String },
    /// Decoded frames agree: the same size and, for two movies, the same frame count, and every frame at
    /// least 40 dB PSNR from its counterpart (D-12), or, with `exact`, every decoded frame the same pixel
    /// for pixel. A frame index picks one frame of a movie or a still to compare with one frame of the
    /// other (a still is frame 0); without either index both are movies.
    Frames {
        actual: String,
        reference: String,
        actual_frame: Option<u32>,
        reference_frame: Option<u32>,
        #[serde(default)]
        exact: bool,
    },
    /// Two PNGs hold the pixels they say, straight RGBA, each channel within `tolerance` levels of the
    /// other's, in all but `allowed_fraction` of the pixels, and no pixel's channel further than `cap`
    /// levels: the compositor's own rule for a render on the GPU against the CPU's.
    Pixels {
        actual: String,
        reference: String,
        #[serde(default)]
        tolerance: u8,
        #[serde(default)]
        allowed_fraction: f64,
        cap: Option<u8>,
    },
    /// Two files are the same JSON document, the scratch folders they sit in aside, numbers by value.
    Json { actual: String, reference: String },
}

impl Compare {
    pub(crate) fn files(&self) -> [&str; 2] {
        match self {
            Compare::Identical { actual, reference }
            | Compare::Frames { actual, reference, .. }
            | Compare::Pixels { actual, reference, .. }
            | Compare::Json { actual, reference, .. } => [actual, reference],
        }
    }
}

impl Corpus {
    pub(crate) fn parse(text: &str) -> Result<Corpus, String> {
        let corpus: Corpus = toml::from_str(text).map_err(|error| format!("the corpus does not parse: {error}"))?;
        let errors = corpus.errors();
        if errors.is_empty() { Ok(corpus) } else { Err(errors.join("\n")) }
    }

    /// What makes a corpus unusable, found before anything runs: a scenario the failures could not name, a
    /// check of a step that does not exist, a step a side cannot run, a comparison of sides that did not
    /// both run, or a path that leaves the scratch folder.
    fn errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.scenarios.is_empty() {
            errors.push("the corpus holds no scenario".to_string());
        }
        let mut seen = BTreeSet::new();
        for scenario in &self.scenarios {
            let id = &scenario.id;
            if id.trim().is_empty() || !seen.insert(id.as_str()) {
                errors.push(format!("scenario id {id:?} is empty or repeated: every failure names its scenario by id"));
            }
            if scenario.steps.is_empty() {
                errors.push(format!("{id}: no step"));
            }
            errors.extend(scenario_errors(scenario));
        }
        errors
    }
}

fn scenario_errors(scenario: &Scenario) -> Vec<String> {
    let id = &scenario.id;
    let mut errors = Vec::new();
    for (index, step) in scenario.steps.iter().enumerate() {
        if scenario.sides.runs_oracle() && step.args.is_empty() {
            errors.push(format!("{id}: step {index} has no `args` for the oracle"));
        }
        if scenario.sides.runs_plugin() && step.calls.is_empty() {
            errors.push(format!("{id}: step {index} has no `call` for the plug-in"));
        }
        if !scenario.sides.runs_plugin() && !step.calls.is_empty() {
            errors.push(format!("{id}: step {index} has a `call`, but the scenario runs on the oracle alone"));
        }
        // The host builds the plug-in's app and nothing else: on a scenario without the plug-in it would be
        // a GPU or a plug-in folder the scenario reads as asked for and nothing gives.
        if !scenario.sides.runs_plugin() && (step.host.gpu || step.host.plugins.is_some()) {
            errors.push(format!("{id}: step {index} has a `host`, but the scenario runs on the oracle alone"));
        }
        if step.fails && !scenario.sides.runs_oracle() {
            errors.push(format!("{id}: step {index} says the command line fails, but the scenario does not run on it"));
        }
        if !scenario.sides.runs_oracle() && !step.args.is_empty() {
            errors.push(format!("{id}: step {index} has `args`, but the scenario runs on the plug-in alone"));
        }
        for call in &step.calls {
            if let Call::Request { progress: Some(Progress { cancel_at: Some(at) }), .. } = call
                && !(*at > 0.0 && *at <= 1.0)
            {
                errors.push(format!("{id}: step {index} cancels at {at}, which must be a fraction of the frames above 0 and up to 1"));
            }
        }
        let mut inside = step.host.plugins.iter().map(String::as_str).chain(step.calls.iter().flat_map(call_paths));
        if let Some(path) = inside.find(|path| !stays_inside(path)) {
            errors.push(format!("{id}: {path:?} must be a relative path that stays in the scenario's folder"));
        }
    }
    let reads = |step: usize| step < scenario.steps.len();
    for expect in &scenario.expects {
        if !reads(expect.step) {
            errors.push(format!("{id}: an expectation reads step {}, but the scenario has {} steps", expect.step, scenario.steps.len()));
        }
        if expect.equals.is_some() == expect.length.is_some() {
            errors.push(format!("{id}: the expectation of step {} {} needs exactly one of `equals` and `length`", expect.step, expect.pointer));
        }
    }
    for same in &scenario.sames {
        if !reads(same.step()) {
            errors.push(format!("{id}: a comparison reads step {}, but the scenario has {} steps", same.step(), scenario.steps.len()));
        }
    }
    if (!scenario.sames.is_empty()) && scenario.sides != Sides::Both {
        errors.push(format!("{id}: a comparison of the two sides' output needs a scenario that runs on both"));
    }
    for path in
        scenario.fixtures.iter().flat_map(|fixture| [fixture.from.as_str(), fixture.to.as_str()]).chain(scenario.compares.iter().flat_map(Compare::files))
    {
        if !stays_inside(path) {
            errors.push(format!("{id}: {path:?} must be a relative path that stays in the scenario's folder"));
        }
    }
    for folder in &scenario.folders {
        if folder.path != "." && !stays_inside(&folder.path) {
            errors.push(format!("{id}: {:?} must be `.` or a relative path that stays in the scenario's folder", folder.path));
        }
    }
    errors
}

/// The paths of a call inside the scenario's folder.
fn call_paths(call: &Call) -> Vec<&str> {
    match call {
        Call::Request { files, .. } => files.values().map(String::as_str).collect(),
        Call::Frame { out, .. } => vec![out.as_str()],
        Call::Mcp {} => Vec::new(),
    }
}

/// A path that names something below the scenario's scratch folder and nowhere else.
pub(crate) fn stays_inside(path: &str) -> bool {
    let path = Path::new(path);
    path.components().next().is_some() && path.components().all(|part| matches!(part, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
[[scenario]]
id = "T5"
title = "an expression"
[[scenario.step]]
args = ["info", "--json"]
[[scenario.step.call]]
kind = "request"
method = "info"
[[scenario.expect]]
step = 0
pointer = "/commands"
equals = 763
[[scenario.same]]
kind = "output"
step = 0
omit = ["/engine"]
[[scenario.compare]]
kind = "frames"
actual = "a.mp4"
reference = "b.png"
reference_frame = 0
"#;

    fn errors(text: &str) -> String {
        Corpus::parse(text).err().unwrap_or_default()
    }

    #[test]
    fn a_scenario_with_steps_calls_expectations_and_comparisons_reads() {
        let corpus = Corpus::parse(GOOD).expect("the corpus parses");
        let scenario = corpus.scenarios.first().expect("one scenario");
        assert_eq!((scenario.id.as_str(), scenario.steps.len(), scenario.expects.len(), scenario.sames.len(), scenario.compares.len()), ("T5", 1, 1, 1, 1));
        assert_eq!(scenario.sides, Sides::Both);
        assert!(matches!(scenario.steps.first().and_then(|step| step.calls.first()), Some(Call::Request { method, .. }) if method == "info"));
        assert!(matches!(scenario.compares.first(), Some(Compare::Frames { reference_frame: Some(0), actual_frame: None, exact: false, .. })));
    }

    #[test]
    fn an_unknown_key_or_kind_is_refused_rather_than_ignored() {
        assert!(errors(&GOOD.replace("title =", "titel =")).contains("does not parse"));
        assert!(errors(&GOOD.replace("kind = \"frames\"", "kind = \"similar\"")).contains("does not parse"));
        assert!(errors(&GOOD.replace("kind = \"request\"", "kind = \"launch\"")).contains("does not parse"));
        assert!(errors(&GOOD.replace("kind = \"output\"", "kind = \"output\"\nsurplus = 1")).contains("does not parse"));
        assert!(errors(&GOOD.replace("method = \"info\"", "method = \"info\"\nsurplus = 1")).contains("does not parse"));
        let mcp = GOOD.replace("kind = \"request\"\nmethod = \"info\"", "kind = \"mcp\"");
        assert!(Corpus::parse(&mcp).is_ok() && errors(&mcp.replace("kind = \"mcp\"", "kind = \"mcp\"\nsurplus = 1")).contains("does not parse"));
    }

    #[test]
    fn a_scenario_no_failure_could_name_or_check_is_refused() {
        assert!(errors(&format!("{GOOD}{GOOD}")).contains("empty or repeated"));
        assert!(errors(&GOOD.replace("id = \"T5\"", "id = \" \"")).contains("empty or repeated"));
        assert!(errors(&GOOD.replace("step = 0\npointer", "step = 1\npointer")).contains("an expectation reads step 1"));
        assert!(errors(&GOOD.replace("kind = \"output\"\nstep = 0", "kind = \"output\"\nstep = 4")).contains("a comparison reads step 4"));
        assert!(
            errors(&GOOD.replace("[[scenario.step]]\nargs = [\"info\", \"--json\"]\n[[scenario.step.call]]\nkind = \"request\"\nmethod = \"info\"\n", ""))
                .contains("no step")
        );
        assert!(errors("").contains("no scenario"));
    }

    #[test]
    fn an_expectation_names_a_value_or_a_length_never_neither_or_both() {
        assert!(errors(&GOOD.replace("equals = 763", "")).contains("exactly one of"));
        assert!(errors(&GOOD.replace("equals = 763", "equals = 763\nlength = 2")).contains("exactly one of"));
        assert!(Corpus::parse(&GOOD.replace("equals = 763", "length = 2")).is_ok());
    }

    #[test]
    fn each_side_a_scenario_runs_on_needs_what_it_runs_with() {
        assert!(errors(&GOOD.replace("args = [\"info\", \"--json\"]\n", "")).contains("step 0 has no `args` for the oracle"));
        assert!(errors(&GOOD.replace("[[scenario.step.call]]\nkind = \"request\"\nmethod = \"info\"\n", "")).contains("step 0 has no `call` for the plug-in"));
        let oracle_alone = GOOD.replace("title = \"an expression\"", "title = \"x\"\nsides = \"oracle\"");
        assert!(errors(&oracle_alone).contains("has a `call`, but the scenario runs on the oracle alone"), "{}", errors(&oracle_alone));
        let callless = oracle_alone
            .replace("[[scenario.step.call]]\nkind = \"request\"\nmethod = \"info\"\n", "")
            .replace("[[scenario.same]]\nkind = \"output\"\nstep = 0\nomit = [\"/engine\"]\n", "");
        assert!(Corpus::parse(&callless).is_ok(), "{}", errors(&callless));
        for host in ["gpu = true", "plugins = \"plug-ins\""] {
            let hosted = callless.replace("args = [\"info\", \"--json\"]\n", &format!("args = [\"info\", \"--json\"]\n[scenario.step.host]\n{host}\n"));
            assert!(errors(&hosted).contains("step 0 has a `host`, but the scenario runs on the oracle alone"), "{host}: {}", errors(&hosted));
        }
        let plugin_alone = GOOD.replace("title = \"an expression\"", "title = \"x\"\nsides = \"plugin\"");
        let found = errors(&plugin_alone);
        assert!(found.contains("runs on the plug-in alone") && found.contains("needs a scenario that runs on both"), "{found}");
        let fails = plugin_alone.replace("args = [\"info\", \"--json\"]", "fails = true");
        assert!(errors(&fails).contains("says the command line fails, but the scenario does not run on it"), "{}", errors(&fails));
    }

    #[test]
    fn the_folder_is_written_wherever_the_text_names_the_container() {
        assert_eq!(in_folder("a <container>/x <container>", Path::new("/c")), "a /c/x /c");
        assert_eq!(in_folder("nothing", Path::new("/c")), "nothing");
    }

    #[test]
    fn a_path_must_stay_in_the_scratch_folder() {
        for outside in ["/etc/passwd", "../x.png", "a/../../x", ""] {
            assert!(errors(&GOOD.replace("b.png", outside)).contains("stays in the scenario's folder"), "{outside:?}");
            let fixture = format!("{GOOD}\n[[scenario.fixture]]\nfrom = \"{outside}\"\nto = \"x\"\n");
            assert!(errors(&fixture).contains("stays in the scenario's folder"), "{outside:?}");
            let out = GOOD.replace("method = \"info\"", &format!("method = \"info\"\nfiles = {{ script = \"{outside}\" }}"));
            assert!(errors(&out).contains("stays in the scenario's folder"), "{outside:?}");
            let frame = format!("{GOOD}\n[[scenario.step.call]]\nkind = \"frame\"\nformat = \"png\"\nout = \"{outside}\"\n");
            assert!(errors(&frame).contains("stays in the scenario's folder"), "{outside:?}");
            let plugins = GOOD.replace("[[scenario.step.call]]", &format!("[scenario.step.host]\nplugins = \"{outside}\"\n[[scenario.step.call]]"));
            assert!(errors(&plugins).contains("stays in the scenario's folder"), "{outside:?}");
        }
        assert!(stays_inside("sub/out.mp4"));
    }

    #[test]
    fn a_call_that_reports_progress_may_name_the_fraction_it_is_stopped_at() {
        let with = |progress: &str| GOOD.replace("method = \"info\"", &format!("method = \"info\"\nprogress = {progress}"));
        let progress = |text: &str| match Corpus::parse(&with(text)).expect("parses").scenarios.remove(0).steps.remove(0).calls.remove(0) {
            Call::Request { progress, .. } => progress,
            other => panic!("a request expected: {other:?}"),
        };
        assert!(matches!(progress("{}"), Some(Progress { cancel_at: None })));
        assert!(matches!(progress("{ cancel_at = 0.5 }"), Some(Progress { cancel_at: Some(at) }) if at == 0.5));
        assert!(matches!(progress("{ cancel_at = 1 }"), Some(Progress { cancel_at: Some(at) }) if at == 1.0));
        assert!(
            Corpus::parse(GOOD).is_ok_and(|corpus| matches!(
                corpus.scenarios.first().and_then(|s| s.steps.first()?.calls.first()),
                Some(Call::Request { progress: None, .. })
            ))
        );
        for outside in ["0", "-0.5", "1.5", "nan", "inf"] {
            assert!(errors(&with(&format!("{{ cancel_at = {outside} }}"))).contains("which must be a fraction of the frames above 0 and up to 1"), "{outside}");
        }
        assert!(errors(&with("{ cancel_after = 0.5 }")).contains("does not parse"));
    }

    #[test]
    fn the_listener_stops_a_call_at_the_fraction_and_never_one_that_names_none_or_has_no_frames() {
        let at = |fraction: Option<f64>| Progress { cancel_at: fraction };
        assert!(!at(None).cancels(100, 100));
        assert!(!at(Some(0.5)).cancels(49, 100) && at(Some(0.5)).cancels(50, 100) && at(Some(0.5)).cancels(51, 100));
        assert!(!at(Some(0.5)).cancels(0, 0), "a call that has not said how many frames it has is not stopped");
        assert!(at(Some(1.0)).cancels(10, 10) && !at(Some(1.0)).cancels(9, 10));
    }

    #[test]
    fn a_folder_rule_names_a_path_in_the_scenarios_folder_or_the_folder_itself() {
        let rule = |path: &str| format!("{GOOD}\n[[scenario.folder]]\npath = \"{path}\"\nholds = [\"a\"]\n");
        for inside in [".", "out", "out/deeper"] {
            assert!(Corpus::parse(&rule(inside)).is_ok(), "{inside}");
        }
        for outside in ["/etc", "..", "../x", "a/../../x", ""] {
            assert!(errors(&rule(outside)).contains("must be `.` or a relative path that stays in the scenario's folder"), "{outside:?}");
        }
        assert!(errors(&format!("{GOOD}\n[[scenario.folder]]\npath = \".\"\n")).contains("does not parse"), "a rule without its names is no rule");
    }
}
