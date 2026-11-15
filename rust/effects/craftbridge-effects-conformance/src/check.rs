//! Running one scenario through the oracle and checking what it produced.

use crate::corpus::{Compare, Expect, Scenario};
use crate::frames::{Clip, conform};
use crate::oracle::Oracle;
use serde_json::{Number, Value};
use std::path::Path;

/// The failures of one scenario; none means it passed. Every failure is found, not only the first, so one
/// run says everything that is wrong with a scenario. A step that fails ends the scenario, since what it
/// should have written is not there to check.
pub(crate) fn run(oracle: &dyn Oracle, scenario: &Scenario) -> Vec<String> {
    // A fresh, uniquely named folder, removed when the scenario ends, passed or failed, so a scenario starts
    // from nothing and leaves nothing behind. A run killed outright (SIGKILL, or a Ctrl-C, whose default
    // action runs no destructor) leaves its folder for the system to clear from its temporary folder, and
    // nothing sweeps it on the next run: no later run can meet it, since every name is new, while a sweep
    // of that shared folder could take the folder of a run another worktree's gate has going at the time.
    let scratch = match tempfile::Builder::new().prefix("craftbridge-conformance-").tempdir() {
        Ok(scratch) => scratch,
        Err(error) => return vec![format!("cannot make a scratch folder: {error}")],
    };
    let mut stdouts = Vec::new();
    for (index, step) in scenario.steps.iter().enumerate() {
        match oracle.run(scratch.path(), &step.args) {
            Err(error) => return vec![format!("step {index} (`{}`): {error}", step.args.join(" "))],
            Ok(outcome) if !outcome.success => {
                return vec![format!("step {index} (`{}`) ended with {}: {}", step.args.join(" "), outcome.ended, outcome.stderr.trim())];
            }
            Ok(outcome) => stdouts.push(outcome.stdout),
        }
    }
    let mut failures: Vec<String> = scenario.expects.iter().filter_map(|expect| expected(expect, &stdouts)).collect();
    failures.extend(scenario.compares.iter().flat_map(|compare| compared(compare, scratch.path())));
    failures
}

/// The failure of one expectation, if it does not hold.
fn expected(expect: &Expect, stdouts: &[String]) -> Option<String> {
    let at = format!("step {} {}", expect.step, expect.pointer);
    let Some(stdout) = stdouts.get(expect.step) else { return Some(format!("{at}: the step did not run")) };
    let document: Value = match serde_json::from_str(stdout) {
        Ok(document) => document,
        Err(error) => return Some(format!("{at}: the step's output is not JSON: {error}")),
    };
    let wanted = match serde_json::to_value(&expect.equals) {
        Ok(wanted) => wanted,
        Err(error) => return Some(format!("{at}: the expected value is not JSON: {error}")),
    };
    match document.pointer(&expect.pointer) {
        None => Some(format!("{at}: the output has no such value")),
        Some(found) if same(found, &wanted) => None,
        Some(found) => Some(format!("{at}: found {found}, expected {wanted}")),
    }
}

/// JSON equality in which a number is its value: TOML writes `90` where the oracle prints `90.0`.
fn same(found: &Value, wanted: &Value) -> bool {
    match (found, wanted) {
        (Value::Number(a), Value::Number(b)) => same_number(a, b),
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b)),
        (Value::Object(a), Value::Object(b)) => a.len() == b.len() && a.iter().all(|(key, a)| b.get(key).is_some_and(|b| same(a, b))),
        _ => found == wanted,
    }
}

/// Two JSON numbers are the same number. serde_json's own `==` is exact between two integers and between two
/// floats, and false across the two; an integer and a float are the same only when the float is integral
/// and is that integer. Rounding the integer to a float instead would make two numbers one past 2^53,
/// where an f64 no longer holds neighbouring integers apart, and pass an expectation against a different
/// number.
fn same_number(a: &Number, b: &Number) -> bool {
    if a == b {
        return true;
    }
    let integer = |n: &Number| n.as_i64().map(i128::from).or_else(|| n.as_u64().map(i128::from));
    // A fraction or an infinity is no integer. An integral float converts exactly while it fits an i128,
    // and `as` saturates one that does not, out of reach of every integer JSON holds.
    let integral = |f: f64| (f.fract() == 0.0).then_some(f as i128);
    match (integer(a), integer(b), a.as_f64(), b.as_f64()) {
        (Some(int), None, _, Some(float)) | (None, Some(int), Some(float), _) => integral(float) == Some(int),
        _ => false,
    }
}

/// The failures of one comparison of two files the steps wrote.
fn compared(compare: &Compare, folder: &Path) -> Vec<String> {
    match compare {
        Compare::Identical { actual, reference } => match (std::fs::read(folder.join(actual)), std::fs::read(folder.join(reference))) {
            (Ok(ours), Ok(theirs)) if ours == theirs => Vec::new(),
            (Ok(_), Ok(_)) => vec![format!("{actual} is not byte-identical to {reference}")],
            (Err(error), _) => vec![format!("{actual}: {error}")],
            (_, Err(error)) => vec![format!("{reference}: {error}")],
        },
        Compare::Frames { actual, reference, actual_frame, reference_frame } => match (Clip::open(&folder.join(actual)), Clip::open(&folder.join(reference))) {
            (Ok(ours), Ok(theirs)) => {
                conform(&ours, &theirs, *actual_frame, *reference_frame).into_iter().map(|f| format!("{actual} against {reference}: {f}")).collect()
            }
            (Err(error), _) | (_, Err(error)) => vec![error],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::Corpus;
    use crate::oracle::Outcome;
    use crate::testing::png_bytes;
    use std::cell::RefCell;

    /// An oracle that prints canned output and writes canned files: the comparison is tested on its own.
    struct Fake {
        stdout: &'static str,
        files: Vec<(&'static str, Vec<u8>)>,
        fail: bool,
        calls: RefCell<Vec<Vec<String>>>,
        folders: RefCell<Vec<std::path::PathBuf>>,
    }

    impl Fake {
        fn new(stdout: &'static str) -> Fake {
            Fake { stdout, files: Vec::new(), fail: false, calls: RefCell::new(Vec::new()), folders: RefCell::new(Vec::new()) }
        }
    }

    impl Oracle for Fake {
        fn run(&self, folder: &Path, args: &[String]) -> Result<Outcome, String> {
            self.calls.borrow_mut().push(args.to_vec());
            self.folders.borrow_mut().push(folder.to_path_buf());
            for (name, bytes) in &self.files {
                std::fs::write(folder.join(name), bytes).map_err(|error| error.to_string())?;
            }
            Ok(Outcome { success: !self.fail, ended: "exit status: 1".to_string(), stdout: self.stdout.to_string(), stderr: "boom\n".to_string() })
        }
    }

    fn scenario(body: &str) -> Scenario {
        let text = format!("[[scenario]]\nid = \"T5\"\ntitle = \"x\"\n[[scenario.step]]\nargs = [\"get\", \"--json\"]\n{body}");
        Corpus::parse(&text).expect("parses").scenarios.remove(0)
    }

    const EXPECT_90: &str = "[[scenario.expect]]\nstep = 0\npointer = \"/evaluated\"\nequals = 90\n";

    #[test]
    fn an_expected_value_the_oracle_prints_passes_whatever_number_form_it_uses() {
        let oracle = Fake::new("{\"evaluated\":90.0}");
        assert_eq!(run(&oracle, &scenario(EXPECT_90)), Vec::<String>::new());
        assert_eq!(oracle.calls.borrow().as_slice(), [vec!["get".to_string(), "--json".to_string()]]);
    }

    #[test]
    fn a_wrong_expected_value_fails_and_says_what_was_found() {
        let failures = run(&Fake::new("{\"evaluated\":91.0}"), &scenario(EXPECT_90));
        assert_eq!(failures, ["step 0 /evaluated: found 91.0, expected 90"]);
        assert!(run(&Fake::new("{}"), &scenario(EXPECT_90)).iter().any(|f| f.contains("no such value")));
        assert!(run(&Fake::new("not json"), &scenario(EXPECT_90)).iter().any(|f| f.contains("not JSON")));
    }

    #[test]
    fn values_compare_as_json_with_numbers_by_value() {
        let a: Value = serde_json::json!({"n": [1, 2.0], "s": "x"});
        assert!(same(&a, &serde_json::json!({"s": "x", "n": [1.0, 2]})));
        assert!(!same(&a, &serde_json::json!({"s": "x", "n": [1, 3]})));
        assert!(!same(&a, &serde_json::json!({"s": "x"})));
        // Past what an f64 holds apart, two integers still differ: a float comparison would say no.
        let far = Value::from(9007199254740993_u64);
        assert!(!same(&far, &Value::from(9007199254740992_u64)), "two different integers are never the same number");
        assert!(same(&Value::from(9007199254740993_u64), &Value::from(9007199254740993_u64)));
        // An integer and a float meet only when the float is exactly that integer, far out as near.
        let float = |text: &str| serde_json::from_str::<Value>(text).expect("a JSON float");
        assert!(!same(&far, &float("9007199254740992.0")), "an integer is never a float that is not it");
        assert!(same(&Value::from(9007199254740992_u64), &float("9007199254740992.0")));
        assert!(!same(&Value::from(u64::MAX), &float("18446744073709551616.0")), "u64::MAX is not 2^64");
        assert!(same(&Value::from(-3), &float("-3.0")) && !same(&Value::from(90), &float("90.5")));
        // The reach of the i128 between them: a u64-wide integer meets its own float, and a float past
        // every integer JSON holds saturates out of the reach of any of them. An i64 middle would sink
        // the first and raise the second as a false match on its own largest integer.
        assert!(same(&Value::from(10_000_000_000_000_000_000_u64), &float("1e19")));
        assert!(!same(&Value::from(i64::MAX), &float("1e40")), "a float past i128 is no integer");
    }

    #[test]
    fn a_failing_step_ends_the_scenario_with_the_oracles_own_message() {
        let mut oracle = Fake::new("");
        oracle.fail = true;
        let failures = run(&oracle, &scenario(EXPECT_90));
        assert_eq!(failures.len(), 1);
        assert!(failures.iter().all(|f| f.contains("step 0 (`get --json`)") && f.contains("boom")), "{failures:?}");
    }

    #[test]
    fn byte_identical_files_pass_and_a_difference_fails() {
        let compare = "[[scenario.compare]]\nkind = \"identical\"\nactual = \"a\"\nreference = \"b\"\n";
        let mut oracle = Fake::new("");
        oracle.files = vec![("a", vec![1, 2]), ("b", vec![1, 2])];
        assert!(run(&oracle, &scenario(compare)).is_empty());
        oracle.files = vec![("a", vec![1, 2]), ("b", vec![1, 3])];
        assert_eq!(run(&oracle, &scenario(compare)), ["a is not byte-identical to b"]);
        oracle.files = vec![("a", vec![1])];
        assert!(run(&oracle, &scenario(compare)).iter().any(|f| f.starts_with("b: ")));
    }

    #[test]
    fn every_scenario_runs_in_a_folder_of_its_own_that_is_gone_afterwards_whether_it_passed_or_failed() {
        let compare = "[[scenario.compare]]\nkind = \"identical\"\nactual = \"a\"\nreference = \"b\"\n";
        let mut oracle = Fake::new("");
        oracle.files = vec![("a", vec![1]), ("b", vec![2])];
        assert!(!run(&oracle, &scenario(compare)).is_empty());
        oracle.files = vec![("a", vec![1]), ("b", vec![1])];
        assert!(run(&oracle, &scenario(compare)).is_empty());
        let folders = oracle.folders.borrow();
        assert!(matches!(folders.as_slice(), [first, second] if first != second && !first.exists() && !second.exists()), "{folders:?}");
    }

    #[test]
    fn a_frames_comparison_names_the_files_and_the_frame() {
        let compare = "[[scenario.compare]]\nkind = \"frames\"\nactual = \"a.png\"\nreference = \"b.png\"\n";
        let png = |shade: u8| png_bytes(16, 16, |_, _| [shade; 3]);
        let mut oracle = Fake::new("");
        oracle.files = vec![("a.png", png(100)), ("b.png", png(100))];
        assert!(run(&oracle, &scenario(compare)).is_empty());
        oracle.files = vec![("a.png", png(100)), ("b.png", png(160))];
        let failures = run(&oracle, &scenario(compare));
        assert!(failures.iter().any(|f| f.starts_with("a.png against b.png: frame 0 is ") && f.contains("below the 40 dB")), "{failures:?}");
    }
}
