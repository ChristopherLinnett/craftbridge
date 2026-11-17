//! Running one scenario on each side it names and checking what they produced.

use crate::corpus::{Compare, Expect, Fixture, Same, Scenario};
use crate::documents::{difference, same, set_aside, stream, without_folder};
use crate::frames::{Clip, conform, pixels};
use crate::oracle::{Outcome, Side};
use crate::timings;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The two sides a scenario may run on.
pub(crate) struct Sides<'a> {
    pub(crate) oracle: &'a dyn Side,
    pub(crate) plugin: &'a dyn Side,
}

/// What a side left behind: the folder its steps ran in and what each step printed.
struct Ran {
    folder: PathBuf,
    outputs: Vec<Outcome>,
}

/// The failures of one scenario; none means it passed. Every failure is found, not only the first, so one
/// run says everything that is wrong with a scenario. A step that fails ends its side, since what it should
/// have written is not there to check; the other side still runs. `corpus` is the folder of the corpus
/// file, which the scenario's fixtures are read from.
pub(crate) fn run(sides: &Sides, scenario: &Scenario, corpus: &Path) -> Vec<String> {
    // A fresh, uniquely named folder, removed when the scenario ends, passed or failed, so a scenario starts
    // from nothing and leaves nothing behind. A run killed outright (SIGKILL, or a Ctrl-C, whose default
    // action runs no destructor) leaves its folder for the system to clear from its temporary folder, and
    // nothing sweeps it on the next run: no later run can meet it, since every name is new, while a sweep
    // of that shared folder could take the folder of a run another worktree's gate has going at the time.
    let scratch = match tempfile::Builder::new().prefix("craftbridge-conformance-").tempdir() {
        Ok(scratch) => scratch,
        Err(error) => return vec![format!("cannot make a scratch folder: {error}")],
    };
    let mut failures = Vec::new();
    let mut ran = |name: &str, side: &dyn Side, command_line: bool| match run_steps(side, command_line, scenario, &scratch.path().join(name), corpus) {
        Ok(ran) => Some(ran),
        Err(error) => {
            failures.push(format!("{name}: {error}"));
            None
        }
    };
    let oracle = scenario.sides.runs_oracle().then(|| ran("oracle", sides.oracle, true)).flatten();
    let plugin = scenario.sides.runs_plugin().then(|| ran("plug-in", sides.plugin, false)).flatten();
    for (name, side) in [("oracle", &oracle), ("plug-in", &plugin)] {
        if let Some(side) = side {
            failures.extend(scenario.expects.iter().filter_map(|expect| expected(expect, &side.outputs).map(|failure| format!("{name}: {failure}"))));
        }
    }
    // A comparison holds the plug-in's files against the oracle's; a scenario that runs on one side alone
    // holds that side's files against each other, so every comparison it states is made. A side that did
    // not run has failed already, and nothing is compared with a side that is not there.
    let (actual, reference) = match (scenario.sides.runs_oracle(), scenario.sides.runs_plugin()) {
        (true, true) => (plugin.as_ref(), oracle.as_ref()),
        (true, false) => (oracle.as_ref(), oracle.as_ref()),
        (false, _) => (plugin.as_ref(), plugin.as_ref()),
    };
    if let (Some(actual), Some(reference)) = (actual, reference) {
        failures.extend(scenario.sames.iter().flat_map(|same| compared_outputs(same, actual, reference)));
        failures.extend(scenario.compares.iter().flat_map(|compare| compared(compare, actual, reference)));
    }
    failures
}

/// Every step of the scenario on one side, in a folder of its own that holds the scenario's fixtures
/// first; the first step that cannot run, or ends badly, is the side's failure.
fn run_steps(side: &dyn Side, command_line: bool, scenario: &Scenario, folder: &Path, corpus: &Path) -> Result<Ran, String> {
    std::fs::create_dir_all(folder).map_err(|error| format!("cannot make {}: {error}", folder.display()))?;
    for fixture in &scenario.fixtures {
        place(fixture, corpus, folder)?;
    }
    let mut outputs = Vec::new();
    for (index, step) in scenario.steps.iter().enumerate() {
        let shown = if step.args.is_empty() { format!("{} calls", step.calls.len()) } else { step.args.join(" ") };
        match side.run(folder, step) {
            Err(error) => return Err(format!("step {index} (`{shown}`): {error}")),
            // A step the corpus says fails, on the command line, must: its status is part of what it does.
            Ok(outcome) if outcome.success && command_line && step.fails => {
                return Err(format!("step {index} (`{shown}`) ended well, where the scenario says it fails"));
            }
            Ok(outcome) if !outcome.success && !(command_line && step.fails) => {
                return Err(format!("step {index} (`{shown}`) ended with {}: {}", outcome.ended, outcome.stderr.trim()));
            }
            Ok(outcome) => outputs.push(outcome),
        }
    }
    Ok(Ran { folder: folder.to_path_buf(), outputs })
}

/// A fixture copied from the corpus's folder into a side's.
fn place(fixture: &Fixture, corpus: &Path, folder: &Path) -> Result<(), String> {
    let (from, to) = (corpus.join(&fixture.from), folder.join(&fixture.to));
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("cannot make {}: {error}", parent.display()))?;
    }
    std::fs::copy(&from, &to).map(drop).map_err(|error| format!("cannot copy the fixture {} to {}: {error}", from.display(), to.display()))
}

/// The failure of one expectation, if it does not hold.
fn expected(expect: &Expect, outputs: &[Outcome]) -> Option<String> {
    let at = format!("step {} {}", expect.step, expect.pointer);
    let Some(output) = outputs.get(expect.step) else { return Some(format!("{at}: the step did not run")) };
    let documents = match stream(&output.stdout) {
        Ok(documents) => documents,
        Err(error) => return Some(format!("{at}: the step's {error}")),
    };
    let Some(document) = documents.get(expect.output) else {
        return Some(format!("{at}: the step printed {} documents, none at {}", documents.len(), expect.output));
    };
    let Some(found) = document.pointer(&expect.pointer) else { return Some(format!("{at}: the output has no such value")) };
    if let Some(length) = expect.length {
        let held = match found {
            Value::Array(items) => Some(items.len()),
            Value::Object(entries) => Some(entries.len()),
            _ => None,
        };
        return match held {
            Some(held) if held == length => None,
            Some(held) => Some(format!("{at}: holds {held} items, expected {length}")),
            None => Some(format!("{at}: found {found}, which holds no items")),
        };
    }
    let wanted = match serde_json::to_value(&expect.equals) {
        Ok(wanted) => wanted,
        Err(error) => return Some(format!("{at}: the expected value is not JSON: {error}")),
    };
    if same(found, &wanted) { None } else { Some(format!("{at}: found {found}, expected {wanted}")) }
}

/// The failures of one comparison of the sides' output of a step.
fn compared_outputs(compare: &Same, under_test: &Ran, oracle: &Ran) -> Vec<String> {
    let step = compare.step();
    let (Some(ours), Some(theirs)) = (under_test.outputs.get(step), oracle.outputs.get(step)) else {
        return vec![format!("step {step}: the step did not run")];
    };
    match compare {
        Same::Timings { .. } => {
            // The command line prints its benchmark on both outputs, one after the other.
            let text = format!("{}\n{}", theirs.stderr, theirs.stdout);
            let answer = stream(&ours.stdout).map(|documents| documents.first().map(timings::from_json).unwrap_or_default());
            match answer {
                Ok(timed) => timings::failures(&timings::from_text(&text), &timed).into_iter().map(|failure| format!("step {step}: {failure}")).collect(),
                Err(error) => vec![format!("step {step}: the plug-in's {error}")],
            }
        }
        Same::Output { omit, entries, within, .. } => {
            let read = |ran: &Ran, output: &Outcome| {
                stream(&without_folder(&output.stdout, &ran.folder)).map(|mut documents| {
                    documents.iter_mut().for_each(|document| set_aside(document, omit, entries, within));
                    documents
                })
            };
            match (read(under_test, ours), read(oracle, theirs)) {
                (Ok(ours), Ok(theirs)) => {
                    if ours.len() != theirs.len() {
                        return vec![format!("step {step}: the plug-in printed {} documents, the command line {}", ours.len(), theirs.len())];
                    }
                    ours.iter()
                        .zip(&theirs)
                        .enumerate()
                        .filter_map(|(index, (a, b))| difference(a, b).map(|at| format!("step {step} document {index}, {at}")))
                        .collect()
                }
                (Err(error), _) => vec![format!("step {step}: the plug-in's {error}")],
                (_, Err(error)) => vec![format!("step {step}: the command line's {error}")],
            }
        }
    }
}

/// The failures of one comparison of two files the steps wrote: `actual` in the side under test's folder,
/// `reference` in the oracle's.
fn compared(compare: &Compare, under_test: &Ran, oracle: &Ran) -> Vec<String> {
    let [name_ours, name_theirs] = compare.files();
    let (ours, theirs) = (under_test.folder.join(name_ours), oracle.folder.join(name_theirs));
    match compare {
        Compare::Identical { actual, reference } => match (std::fs::read(&ours), std::fs::read(&theirs)) {
            (Ok(ours), Ok(theirs)) if ours == theirs => Vec::new(),
            (Ok(_), Ok(_)) => vec![format!("{actual} is not byte-identical to {reference}")],
            (Err(error), _) => vec![format!("{actual}: {error}")],
            (_, Err(error)) => vec![format!("{reference}: {error}")],
        },
        Compare::Frames { actual, reference, actual_frame, reference_frame } => match (Clip::open(&ours), Clip::open(&theirs)) {
            (Ok(ours), Ok(theirs)) => {
                conform(&ours, &theirs, *actual_frame, *reference_frame).into_iter().map(|f| format!("{actual} against {reference}: {f}")).collect()
            }
            (Err(error), _) | (_, Err(error)) => vec![error],
        },
        Compare::Pixels { actual, reference, tolerance, allowed_fraction, cap } => {
            pixels(&ours, &theirs, *tolerance, *allowed_fraction, cap.unwrap_or(*tolerance))
                .into_iter()
                .map(|f| format!("{actual} against {reference}: {f}"))
                .collect()
        }
        Compare::Json { actual, reference } => {
            let read = |path: &Path, folder: &Path| -> Result<Value, String> {
                let text = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
                serde_json::from_str(&without_folder(&text, folder)).map_err(|error| format!("{}: not JSON: {error}", path.display()))
            };
            match (read(&ours, &under_test.folder), read(&theirs, &oracle.folder)) {
                (Ok(ours), Ok(theirs)) => difference(&ours, &theirs).map(|at| format!("{actual} against {reference}: {at}")).into_iter().collect(),
                (Err(error), _) | (_, Err(error)) => vec![error],
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{Corpus, Step};
    use crate::testing::png_bytes;
    use std::cell::RefCell;

    /// A side that prints canned output and writes canned files: the comparison is tested on its own.
    struct Fake {
        stdout: String,
        stderr: String,
        files: Vec<(&'static str, Vec<u8>)>,
        fail: bool,
        calls: RefCell<Vec<Vec<String>>>,
        folders: RefCell<Vec<PathBuf>>,
    }

    impl Fake {
        fn new(stdout: &str) -> Fake {
            Fake {
                stdout: stdout.to_string(),
                stderr: String::new(),
                files: Vec::new(),
                fail: false,
                calls: RefCell::new(Vec::new()),
                folders: RefCell::new(Vec::new()),
            }
        }
    }

    impl Side for Fake {
        fn run(&self, folder: &Path, step: &Step) -> Result<Outcome, String> {
            self.calls.borrow_mut().push(step.args.clone());
            self.folders.borrow_mut().push(folder.to_path_buf());
            for (name, bytes) in &self.files {
                // A file that is text names the folder it was written in where it says <HERE>, as a side's own does.
                let bytes = match std::str::from_utf8(bytes) {
                    Ok(text) => text.replace("<HERE>", &folder.display().to_string()).into_bytes(),
                    Err(_) => bytes.clone(),
                };
                std::fs::write(folder.join(name), bytes).map_err(|error| error.to_string())?;
            }
            // The folder the step ran in is what a side's own output spells paths with.
            let stdout = self.stdout.replace("<HERE>", &folder.display().to_string());
            Ok(Outcome {
                success: !self.fail,
                ended: "exit status: 1".to_string(),
                stdout,
                stderr: if self.fail { "boom\n".to_string() } else { self.stderr.clone() },
            })
        }
    }

    /// The scenario `body` finishes: one step that runs on `sides`, with a call for the plug-in.
    fn scenario(sides: &str, body: &str) -> Scenario {
        Corpus::parse(&scenario_text(sides, body)).expect("parses").scenarios.remove(0)
    }

    fn scenario_text(sides: &str, body: &str) -> String {
        format!(
            "[[scenario]]\nid = \"T5\"\ntitle = \"x\"\nsides = \"{sides}\"\n[[scenario.step]]\n{}{body}",
            match sides {
                "oracle" => "args = [\"get\", \"--json\"]\n",
                "plugin" => "[[scenario.step.call]]\nkind = \"mcp\"\n",
                _ => "args = [\"get\", \"--json\"]\n[[scenario.step.call]]\nkind = \"mcp\"\n",
            }
        )
    }

    fn both(oracle: &Fake, plugin: &Fake, scenario: &Scenario) -> Vec<String> {
        run(&Sides { oracle, plugin }, scenario, Path::new("/nowhere"))
    }

    fn alone(oracle: &Fake, scenario: &Scenario) -> Vec<String> {
        both(oracle, &Fake::new(""), scenario)
    }

    const EXPECT_90: &str = "[[scenario.expect]]\nstep = 0\npointer = \"/evaluated\"\nequals = 90\n";

    #[test]
    fn an_expected_value_the_oracle_prints_passes_whatever_number_form_it_uses() {
        let oracle = Fake::new("{\"evaluated\":90.0}");
        assert_eq!(alone(&oracle, &scenario("oracle", EXPECT_90)), Vec::<String>::new());
        assert_eq!(oracle.calls.borrow().as_slice(), [vec!["get".to_string(), "--json".to_string()]]);
    }

    #[test]
    fn a_wrong_expected_value_fails_and_says_what_was_found() {
        let failures = alone(&Fake::new("{\"evaluated\":91.0}"), &scenario("oracle", EXPECT_90));
        assert_eq!(failures, ["oracle: step 0 /evaluated: found 91.0, expected 90"]);
        assert!(alone(&Fake::new("{}"), &scenario("oracle", EXPECT_90)).iter().any(|f| f.contains("no such value")));
        assert!(alone(&Fake::new("not json"), &scenario("oracle", EXPECT_90)).iter().any(|f| f.contains("not JSON")));
    }

    #[test]
    fn an_expectation_reads_the_document_it_names_and_counts_items() {
        let replies = Fake::new("{\"a\":1}\n{\"tools\":[1,2,3],\"n\":{\"x\":1}}");
        let body = |output: usize, pointer: &str, check: &str| format!("[[scenario.expect]]\nstep = 0\noutput = {output}\npointer = \"{pointer}\"\n{check}\n");
        assert!(alone(&replies, &scenario("oracle", &body(1, "/tools", "length = 3"))).is_empty());
        assert!(alone(&replies, &scenario("oracle", &body(1, "/n", "length = 1"))).is_empty());
        assert_eq!(alone(&replies, &scenario("oracle", &body(1, "/tools", "length = 4"))), ["oracle: step 0 /tools: holds 3 items, expected 4"]);
        assert!(alone(&replies, &scenario("oracle", &body(1, "/n/x", "length = 1"))).iter().any(|f| f.contains("holds no items")));
        assert!(alone(&replies, &scenario("oracle", &body(2, "/a", "equals = 1"))).iter().any(|f| f.contains("printed 2 documents, none at 2")));
        assert!(alone(&replies, &scenario("oracle", &body(0, "/a", "equals = 1"))).is_empty());
    }

    #[test]
    fn a_failing_step_ends_its_side_with_the_sides_own_message_and_the_other_still_runs() {
        let mut oracle = Fake::new("");
        oracle.fail = true;
        let plugin = Fake::new("{\"evaluated\":91}");
        let failures = both(&oracle, &plugin, &scenario("both", EXPECT_90));
        assert!(failures.iter().any(|f| f.starts_with("oracle: step 0 (`get --json`) ended with exit status: 1: boom")), "{failures:?}");
        assert!(failures.iter().any(|f| f == "plug-in: step 0 /evaluated: found 91, expected 90"), "{failures:?}");
        assert_eq!(failures.len(), 2, "{failures:?}");
    }

    #[test]
    fn a_step_the_corpus_says_fails_must_fail_on_the_command_line_and_still_has_its_output_read() {
        let with_fails = scenario_text("both", EXPECT_90).replace("args = [\"get\", \"--json\"]\n", "args = [\"get\", \"--json\"]\nfails = true\n");
        let failing = Corpus::parse(&with_fails).expect("parses").scenarios.remove(0);
        let mut oracle = Fake::new("{\"evaluated\":90}");
        oracle.fail = true;
        // The command line ends with a failure and prints its answer; the plug-in has no status to fail with.
        assert_eq!(both(&oracle, &Fake::new("{\"evaluated\":90}"), &failing), Vec::<String>::new());
        let ended_well = both(&Fake::new("{\"evaluated\":90}"), &Fake::new("{\"evaluated\":90}"), &failing);
        assert_eq!(ended_well, ["oracle: step 0 (`get --json`) ended well, where the scenario says it fails"]);
        // Without `fails` the same failure ends the step as any other.
        assert!(
            both(&oracle, &Fake::new("{\"evaluated\":90}"), &scenario("both", EXPECT_90))
                .iter()
                .any(|f| f.starts_with("oracle: step 0 (`get --json`) ended with"))
        );
    }

    #[test]
    fn every_expectation_holds_on_each_side_and_a_failure_names_its_side() {
        let (good, bad) = (Fake::new("{\"evaluated\":90}"), Fake::new("{\"evaluated\":89}"));
        assert!(both(&good, &good, &scenario("both", EXPECT_90)).is_empty());
        assert_eq!(both(&good, &bad, &scenario("both", EXPECT_90)), ["plug-in: step 0 /evaluated: found 89, expected 90"]);
        assert_eq!(both(&bad, &good, &scenario("both", EXPECT_90)), ["oracle: step 0 /evaluated: found 89, expected 90"]);
        // A scenario of one side leaves the other alone entirely.
        let plugin = Fake::new("{\"evaluated\":90}");
        assert!(both(&Fake::new("garbage"), &plugin, &scenario("plugin", EXPECT_90)).is_empty());
        assert!(plugin.calls.borrow().len() == 1);
    }

    #[test]
    fn the_sides_outputs_are_one_document_apart_from_what_the_scenario_sets_aside() {
        let oracle = Fake::new("{\"a\":1.0,\"mode\":\"cli\",\"dir\":\"<HERE>/x\"}\n{\"b\":[2]}");
        let plugin = Fake::new("{\"a\":1,\"engine\":\"effectcraft\",\"dir\":\"<HERE>/x\"}\n{\"b\":[2]}");
        let aside = "[[scenario.same]]\nkind = \"output\"\nstep = 0\nomit = [\"/engine\", \"/mode\"]\n";
        assert_eq!(both(&oracle, &plugin, &scenario("both", aside)), Vec::<String>::new(), "folders differ by side and are one marker");
        // Without the set-aside the two differ, and the failure says where.
        let found = both(&oracle, &plugin, &scenario("both", "[[scenario.same]]\nkind = \"output\"\nstep = 0\n"));
        assert_eq!(found, ["step 0 document 0, the document: only the actual has `engine`"]);
        let other = Fake::new("{\"a\":2,\"engine\":\"effectcraft\",\"dir\":\"<HERE>/x\"}\n{\"b\":[2]}");
        assert_eq!(both(&oracle, &other, &scenario("both", aside)), ["step 0 document 0, /a: 2 against 1.0"]);
        let fewer = Fake::new("{\"a\":1}");
        assert!(both(&oracle, &fewer, &scenario("both", aside)).iter().any(|f| f.contains("printed 1 documents, the command line 2")));
        assert!(both(&oracle, &Fake::new("nope"), &scenario("both", aside)).iter().any(|f| f.contains("the plug-in's the output is not JSON")));
        let entries =
            "[[scenario.same]]\nkind = \"output\"\nstep = 0\n[[scenario.same.entries]]\npointer = \"/list\"\nkey = \"id\"\nprefix = \"h.\"\nomit = [\"/on\"]\n";
        let a = Fake::new("{\"list\":[{\"id\":\"h.x\",\"on\":true,\"label\":\"X\"},{\"id\":\"c\",\"on\":true}]}");
        let b = Fake::new("{\"list\":[{\"id\":\"h.x\",\"on\":false,\"label\":\"X\"},{\"id\":\"c\",\"on\":true}]}");
        assert!(both(&a, &b, &scenario("both", entries)).is_empty());
        // The rest of a named entry is still compared, and an entry the rule does not name is compared whole.
        let relabelled = Fake::new("{\"list\":[{\"id\":\"h.x\",\"on\":false,\"label\":\"Y\"},{\"id\":\"c\",\"on\":true}]}");
        assert_eq!(both(&a, &relabelled, &scenario("both", entries)), ["step 0 document 0, /list/0/label: \"Y\" against \"X\""]);
        let other = Fake::new("{\"list\":[{\"id\":\"h.x\",\"on\":false,\"label\":\"X\"},{\"id\":\"c\",\"on\":false}]}");
        assert_eq!(both(&a, &other, &scenario("both", entries)), ["step 0 document 0, /list/1/on: false against true"]);
    }

    #[test]
    fn timings_are_the_same_rows_whatever_the_numbers() {
        let mut oracle = Fake::new("");
        oracle.stderr = "bench (no cache): 2 runs\n  Title process 1.0 ms\n    fx ec.blur 0.3 ms\n".to_string();
        let same = "[[scenario.same]]\nkind = \"timings\"\nstep = 0\n";
        let good = Fake::new("{\"runs\":2,\"frame\":{\"layers\":[{\"layer\":\"Title\",\"depth\":0,\"effects\":[{\"id\":\"ec.blur\"}]}]}}");
        assert!(both(&oracle, &good, &scenario("both", same)).is_empty());
        let wrong = Fake::new("{\"runs\":2,\"frame\":{\"layers\":[{\"layer\":\"Title\",\"depth\":0,\"effects\":[]}]}}");
        assert!(both(&oracle, &wrong, &scenario("both", same)).iter().any(|f| f.starts_with("step 0: the layers and effects timed differ")));
        assert!(both(&oracle, &Fake::new("nope"), &scenario("both", same)).iter().any(|f| f.contains("the plug-in's the output is not JSON")));
    }

    #[test]
    fn byte_identical_files_pass_and_a_difference_fails() {
        let compare = "[[scenario.compare]]\nkind = \"identical\"\nactual = \"a\"\nreference = \"b\"\n";
        let mut oracle = Fake::new("");
        oracle.files = vec![("a", vec![1, 2]), ("b", vec![1, 2])];
        assert!(alone(&oracle, &scenario("oracle", compare)).is_empty());
        oracle.files = vec![("a", vec![1, 2]), ("b", vec![1, 3])];
        assert_eq!(alone(&oracle, &scenario("oracle", compare)), ["a is not byte-identical to b"]);
        oracle.files = vec![("a", vec![1])];
        assert!(alone(&oracle, &scenario("oracle", compare)).iter().any(|f| f.starts_with("b: ")));
    }

    #[test]
    fn a_comparison_reads_the_actual_file_from_the_plug_ins_folder_and_the_reference_from_the_oracles() {
        let compare = "[[scenario.compare]]\nkind = \"identical\"\nactual = \"out\"\nreference = \"out\"\n";
        let (mut oracle, mut plugin) = (Fake::new(""), Fake::new(""));
        oracle.files = vec![("out", vec![1])];
        plugin.files = vec![("out", vec![1])];
        assert!(both(&oracle, &plugin, &scenario("both", compare)).is_empty());
        plugin.files = vec![("out", vec![2])];
        assert_eq!(both(&oracle, &plugin, &scenario("both", compare)), ["out is not byte-identical to out"]);
    }

    /// A scenario that runs on the plug-in alone compares the files it wrote with each other, as one that
    /// runs on the oracle alone does, and a scenario of both sides compares nothing once a side has failed.
    #[test]
    fn a_scenario_of_one_side_compares_that_sides_files_and_one_of_both_needs_both() {
        let compare = "[[scenario.compare]]\nkind = \"identical\"\nactual = \"a\"\nreference = \"b\"\n";
        let mut plugin = Fake::new("");
        plugin.files = vec![("a", vec![1]), ("b", vec![2])];
        assert_eq!(both(&Fake::new(""), &plugin, &scenario("plugin", compare)), ["a is not byte-identical to b"]);
        plugin.files = vec![("a", vec![1]), ("b", vec![1])];
        assert!(both(&Fake::new(""), &plugin, &scenario("plugin", compare)).is_empty());
        // The oracle's own two files differ, which only a comparison of the oracle with itself would see.
        let mut oracle = Fake::new("");
        oracle.files = vec![("a", vec![1]), ("b", vec![2])];
        let mut failing = Fake::new("");
        failing.fail = true;
        let failures = both(&oracle, &failing, &scenario("both", compare));
        assert!(matches!(failures.as_slice(), [only] if only.starts_with("plug-in: step 0")), "{failures:?}");
    }

    #[test]
    fn every_scenario_runs_in_a_folder_of_its_own_that_is_gone_afterwards_whether_it_passed_or_failed() {
        let compare = "[[scenario.compare]]\nkind = \"identical\"\nactual = \"a\"\nreference = \"b\"\n";
        let mut oracle = Fake::new("");
        oracle.files = vec![("a", vec![1]), ("b", vec![2])];
        assert!(!alone(&oracle, &scenario("oracle", compare)).is_empty());
        oracle.files = vec![("a", vec![1]), ("b", vec![1])];
        assert!(alone(&oracle, &scenario("oracle", compare)).is_empty());
        let folders = oracle.folders.borrow();
        assert!(matches!(folders.as_slice(), [first, second] if first != second && !first.exists() && !second.exists()), "{folders:?}");
    }

    #[test]
    fn each_side_runs_in_a_folder_of_its_own_with_the_scenarios_fixtures_in_it() {
        let corpus = tempfile::tempdir().expect("scratch");
        std::fs::write(corpus.path().join("seed.txt"), "seed").expect("written");
        let body = "[[scenario.fixture]]\nfrom = \"seed.txt\"\nto = \"deep/seed.txt\"\n";
        let (oracle, plugin) = (Fake::new(""), Fake::new(""));
        let scenario = scenario("both", body);
        let sides = Sides { oracle: &oracle, plugin: &plugin };
        assert!(run(&sides, &scenario, corpus.path()).is_empty());
        let (a, b) = (oracle.folders.borrow()[0].clone(), plugin.folders.borrow()[0].clone());
        assert!(a != b && a.ends_with("oracle") && b.ends_with("plug-in"), "{a:?} {b:?}");
        // The copy is made before the first step: a missing fixture is a failure of the side that needed it.
        let missing = run(&sides, &scenario, &corpus.path().join("absent"));
        assert!(missing.iter().any(|f| f.starts_with("oracle: cannot copy the fixture") && f.contains("seed.txt")), "{missing:?}");
        assert!(missing.iter().any(|f| f.starts_with("plug-in: cannot copy the fixture")), "{missing:?}");
    }

    #[test]
    fn a_frames_comparison_names_the_files_and_the_frame() {
        let compare = "[[scenario.compare]]\nkind = \"frames\"\nactual = \"a.png\"\nreference = \"b.png\"\n";
        let png = |shade: u8| png_bytes(16, 16, |_, _| [shade; 3]);
        let mut oracle = Fake::new("");
        oracle.files = vec![("a.png", png(100)), ("b.png", png(100))];
        assert!(alone(&oracle, &scenario("oracle", compare)).is_empty());
        oracle.files = vec![("a.png", png(100)), ("b.png", png(160))];
        let failures = alone(&oracle, &scenario("oracle", compare));
        assert!(failures.iter().any(|f| f.starts_with("a.png against b.png: frame 0 is ") && f.contains("below the 40 dB")), "{failures:?}");
    }

    #[test]
    fn a_pixels_comparison_holds_the_files_to_its_tolerance() {
        let compare = |extra: &str| format!("[[scenario.compare]]\nkind = \"pixels\"\nactual = \"a.png\"\nreference = \"b.png\"\n{extra}");
        let png = |shade: u8| png_bytes(8, 8, |_, _| [shade; 3]);
        let mut oracle = Fake::new("");
        oracle.files = vec![("a.png", png(101)), ("b.png", png(100))];
        assert!(alone(&oracle, &scenario("oracle", &compare("tolerance = 1"))).is_empty());
        let failures = alone(&oracle, &scenario("oracle", &compare("")));
        assert!(failures.iter().any(|f| f.starts_with("a.png against b.png: 64 of 64 pixels are more than 0 levels")), "{failures:?}");
        // One pixel in 64 is four levels out: the fraction allows it and `cap` says how far, which is the
        // tolerance when the comparison names none.
        oracle.files = vec![("a.png", png_bytes(8, 8, |x, y| if (x, y) == (0, 0) { [104; 3] } else { [100; 3] })), ("b.png", png(100))];
        let held = |extra: &str| alone(&oracle, &scenario("oracle", &compare(extra)));
        assert!(held("tolerance = 1\nallowed_fraction = 0.02\ncap = 4").is_empty());
        assert!(held("tolerance = 1\nallowed_fraction = 0.02\ncap = 3").iter().any(|f| f.contains("4 levels from the reference's, past the 3 allowed")));
        assert!(held("tolerance = 1\nallowed_fraction = 0.02").iter().any(|f| f.contains("past the 1 allowed")));
        assert!(held("tolerance = 1\nallowed_fraction = 0.01\ncap = 4").iter().any(|f| f.contains("1 of 64 pixels")));
    }

    #[test]
    fn a_json_comparison_reads_each_file_as_its_sides_folder_spells_it() {
        let compare = "[[scenario.compare]]\nkind = \"json\"\nactual = \"p.json\"\nreference = \"p.json\"\n";
        let mut oracle = Fake::new("");
        oracle.files = vec![("p.json", br#"{"n": 1.0}"#.to_vec())];
        assert!(alone(&oracle, &scenario("oracle", compare)).is_empty());
        let (mut a, mut b) = (Fake::new(""), Fake::new(""));
        a.files = vec![("p.json", br#"{"n": [1], "t": "x"}"#.to_vec())];
        b.files = vec![("p.json", br#"{"n": [2], "t": "x"}"#.to_vec())];
        assert_eq!(both(&b, &a, &scenario("both", compare)), ["p.json against p.json: /n/0: 1 against 2"]);
        // Each file spells its own side's folder, which the comparison reads as one marker.
        (a.files, b.files) = (vec![("p.json", br#"{"at": "<HERE>/p.json"}"#.to_vec())], vec![("p.json", br#"{"at": "<HERE>/p.json"}"#.to_vec())]);
        assert!(both(&b, &a, &scenario("both", compare)).is_empty());
        b.files = vec![("p.json", b"not json".to_vec())];
        assert!(both(&b, &a, &scenario("both", compare)).iter().any(|f| f.contains("not JSON")));
    }
}
