//! The plug-in as a side of a scenario: the runner started again with the step's calls (`--plugin-step`),
//! so the plug-in runs as a process of its own under the environment every side has (`oracle.rs`), and a
//! crash or a hang of the app is that step's failure and not the runner's.

use crate::corpus::Step;
use crate::host::Plan;
use crate::oracle::{Cli, Outcome, Side};
use std::path::Path;

/// The flag that makes the runner a plug-in host for one step.
pub(crate) const FLAG: &str = "--plugin-step";

/// The runner itself, started once per step.
pub(crate) struct Plugin(pub(crate) Cli);

impl Side for Plugin {
    fn run(&self, folder: &Path, step: &Step) -> Result<Outcome, String> {
        let plan = Plan { host: step.host.clone(), calls: step.calls.clone() };
        let text = serde_json::to_string(&plan).map_err(|error| format!("cannot write the step's calls: {error}"))?;
        self.0.invoke(folder, &[FLAG.to_string(), text], &step.stdin)
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use crate::corpus::{Call, Host};

    /// The plug-in side over a script that reports how it was started: the plug-in is the runner started
    /// again, and the script stands for the runner so what reaches it can be read.
    fn side(folder: &Path, body: &str) -> Plugin {
        Plugin(Cli { binary: crate::testing::executable(folder, "runner.sh", body), timeout: crate::STEP_TIMEOUT })
    }

    fn step(plugins: Option<&str>) -> Step {
        Step {
            args: Vec::new(),
            stdin: vec!["a line in <container>".to_string()],
            fails: false,
            host: Host { plugins: plugins.map(str::to_string), gpu: true },
            calls: vec![Call::Request {
                method: "info".to_string(),
                params: serde_json::json!({"path": "<container>/x"}),
                files: Default::default(),
                progress: None,
            }],
        }
    }

    /// The plug-in side has the oracle's environment: the same cleared one, with the one variable naming
    /// its own configuration folder, and not a variable of the runner's own (HOME and cargo's are there).
    #[test]
    fn the_plug_in_is_started_in_the_environment_the_oracle_has() {
        let scratch = tempfile::tempdir().expect("scratch");
        let dir = scratch.path().to_path_buf();
        let outcome = side(&dir, "/usr/bin/env").run(&dir, &step(None)).expect("runs");
        crate::testing::assert_only_the_configuration_folder(&outcome.stdout, &dir);
    }

    #[test]
    fn the_calls_reach_the_runner_as_one_argument_and_the_lines_on_its_input_with_the_folder_written_in() {
        let scratch = tempfile::tempdir().expect("scratch");
        let dir = scratch.path().to_path_buf();
        let outcome = side(&dir, "echo \"$1\"; echo \"$#\"; echo \"$2\"; cat").run(&dir, &step(Some("plug-ins"))).expect("runs");
        let lines: Vec<&str> = outcome.stdout.lines().collect();
        let [flag, count, plan, input] = lines.as_slice() else { panic!("four lines expected: {lines:?}") };
        assert_eq!((*flag, *count), (FLAG, "2"));
        assert_eq!(*input, format!("a line in {}", dir.display()));
        let plan: Plan = serde_json::from_str(plan).expect("the plan is JSON");
        assert_eq!(plan.host.plugins.as_deref(), Some("plug-ins"));
        assert!(plan.host.gpu);
        assert!(
            matches!(plan.calls.as_slice(), [Call::Request { method, params, .. }] if method == "info" && params["path"] == format!("{}/x", dir.display())),
            "{plan:?}"
        );
    }
}
