//! The corpus file, `conformance/effects/corpus.toml`: what every platform's runner reads, so a scenario is
//! written once. A scenario is a list of steps (each one command-line invocation of the oracle, run in
//! order in one scratch folder per side) and what to check afterwards.

use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Component, Path};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Corpus {
    // Defaulted, like `steps`, so a corpus that is empty is reported as that and not as a missing key.
    #[serde(default, rename = "scenario")]
    pub(crate) scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    /// The plan's identifier (`T5`), which every failure names.
    pub(crate) id: String,
    pub(crate) title: String,
    #[serde(default, rename = "step")]
    pub(crate) steps: Vec<Step>,
    /// Values the JSON output of every side must hold.
    #[serde(default, rename = "expect")]
    pub(crate) expects: Vec<Expect>,
    /// Files the steps wrote, the side under test's held against the oracle's.
    #[serde(default, rename = "compare")]
    pub(crate) compares: Vec<Compare>,
}

/// One invocation of the oracle: the arguments after its name, run in the scenario's scratch folder, so a
/// relative path in them is relative to that folder.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    pub(crate) args: Vec<String>,
}

/// The value at `pointer` (RFC 6901) in the JSON a step printed on standard output equals `equals`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Expect {
    /// The step's index, from 0.
    pub(crate) step: usize,
    pub(crate) pointer: String,
    pub(crate) equals: toml::Value,
}

/// A comparison of two files the steps wrote: `reference` the oracle's, which it is taken to have right, and
/// `actual` the side under test's. While the oracle is the only side the runner has, both are its own.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum Compare {
    /// The bytes are the same.
    Identical { actual: String, reference: String },
    /// Decoded frames agree: the same size and, for two movies, the same frame count, and every frame at
    /// least 40 dB PSNR from its counterpart (D-12). A frame index picks one frame of a movie or a still
    /// to compare with one frame of the other (a still is frame 0); without either index both are movies.
    Frames { actual: String, reference: String, actual_frame: Option<u32>, reference_frame: Option<u32> },
}

impl Compare {
    fn files(&self) -> [&str; 2] {
        match self {
            Compare::Identical { actual, reference } | Compare::Frames { actual, reference, .. } => [actual, reference],
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
    /// check of a step that does not exist, or a path that leaves the scratch folder.
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
            for expect in &scenario.expects {
                if expect.step >= scenario.steps.len() {
                    errors.push(format!("{id}: an expectation reads step {}, but the scenario has {} steps", expect.step, scenario.steps.len()));
                }
            }
            for file in scenario.compares.iter().flat_map(Compare::files) {
                if !stays_inside(file) {
                    errors.push(format!("{id}: {file:?} must be a relative path that stays in the scenario's folder"));
                }
            }
        }
        errors
    }
}

/// A path that names something below the scenario's scratch folder and nowhere else.
fn stays_inside(path: &str) -> bool {
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
[[scenario.expect]]
step = 0
pointer = "/commands"
equals = 763
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
    fn a_scenario_with_steps_expectations_and_comparisons_reads() {
        let corpus = Corpus::parse(GOOD).expect("the corpus parses");
        let scenario = corpus.scenarios.first().expect("one scenario");
        assert_eq!((scenario.id.as_str(), scenario.steps.len(), scenario.expects.len(), scenario.compares.len()), ("T5", 1, 1, 1));
        assert!(matches!(scenario.compares.first(), Some(Compare::Frames { reference_frame: Some(0), actual_frame: None, .. })));
    }

    #[test]
    fn an_unknown_key_or_comparison_kind_is_refused_rather_than_ignored() {
        assert!(errors(&GOOD.replace("title =", "titel =")).contains("does not parse"));
        assert!(errors(&GOOD.replace("kind = \"frames\"", "kind = \"similar\"")).contains("does not parse"));
    }

    #[test]
    fn a_scenario_no_failure_could_name_or_check_is_refused() {
        assert!(errors(&format!("{GOOD}{GOOD}")).contains("empty or repeated"));
        assert!(errors(&GOOD.replace("id = \"T5\"", "id = \" \"")).contains("empty or repeated"));
        assert!(errors(&GOOD.replace("step = 0", "step = 1")).contains("reads step 1"));
        assert!(errors(&GOOD.replace("[[scenario.step]]\nargs = [\"info\", \"--json\"]\n", "")).contains("no step"));
        assert!(errors("").contains("no scenario"));
    }

    #[test]
    fn a_path_must_stay_in_the_scratch_folder() {
        for outside in ["/etc/passwd", "../x.png", "a/../../x", ""] {
            assert!(errors(&GOOD.replace("b.png", outside)).contains("stays in the scenario's folder"), "{outside:?}");
        }
        assert!(stays_inside("sub/out.mp4"));
    }
}
