//! The Android stage's judgement of Gradle's task outcomes, read from Gradle's plain-console log.
//!
//! A task a module's build file disables, or guards with `onlyIf`, checks nothing and still lets the
//! build succeed; Gradle reports it `SKIPPED`, whatever the reason or however late it was set, so any
//! skipped task of the SDK's projects fails the stage. The included `build-logic` is the gate's own
//! machinery, where the Kotlin plugin skips one of its checks by design. `tools/ci.sh` runs Gradle and
//! hands its log here, so the rule is Rust with tests rather than a `grep` nothing tests.

use std::process::ExitCode;

/// The prefix of the included build's tasks, which may skip.
const MACHINERY_TASKS: &str = ":build-logic:";

/// The tasks a plain-console Gradle log reports as `SKIPPED`, outside the included build: each line
/// `> Task :<path> SKIPPED`, exactly as `--console=plain` prints it. A task Gradle found up to date, or
/// took from the cache, did its checking and is not skipped.
pub(crate) fn skipped_tasks(log: &str) -> Vec<String> {
    log.lines()
        .filter_map(|line| line.trim_end_matches('\r').strip_prefix("> Task ")?.strip_suffix(" SKIPPED").map(str::to_string))
        .filter(|task| task.starts_with(':') && !task.contains(' ') && !task.starts_with(MACHINERY_TASKS))
        .collect()
}

/// Does the log report any task at all? A log with none (empty, cut short, or written in a format this
/// reader does not know) would show no skipped task and pass, having been read for nothing.
pub(crate) fn reports_tasks(log: &str) -> bool {
    log.lines().any(|line| line.trim_end_matches('\r').starts_with("> Task :"))
}

/// `cargo xtask skipped-tasks <log>`: fail, naming each, when the log reports a skipped task, and fail
/// when it reports no task at all.
pub(crate) fn run(log_path: &str) -> ExitCode {
    let log = match std::fs::read_to_string(log_path) {
        Ok(log) => log,
        Err(error) => {
            println!("ci: cannot read the Gradle log {log_path}: {error}");
            return ExitCode::from(1);
        }
    };
    if !reports_tasks(&log) {
        println!("ci: the Gradle log {log_path} reports no `> Task :` line, so no task outcome could be read from it");
        return ExitCode::from(1);
    }
    let skipped = skipped_tasks(&log);
    if skipped.is_empty() {
        return ExitCode::SUCCESS;
    }
    println!("ci: these tasks were skipped, so they checked nothing:");
    for task in skipped {
        println!("> Task {task} SKIPPED");
    }
    ExitCode::from(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_that_reports_no_task_is_read_for_nothing() {
        assert!(!reports_tasks(""));
        assert!(!reports_tasks("Task :core:lintRelease skipped\nBUILD SUCCESSFUL\n"), "another format reports nothing this reader knows");
        assert!(reports_tasks("> Task :core:preBuild UP-TO-DATE\r\n"));
    }

    #[test]
    fn a_skipped_task_of_the_sdk_is_reported_and_the_machinery_s_is_not() {
        let log = "> Task :build-logic:convention:checkKotlinGradlePluginConfigurationErrors SKIPPED\n\
                   > Task :core:lintAnalyzeRelease SKIPPED\n\
                   > Task :effects:spotlessKotlin UP-TO-DATE\n\
                   > Task :core:compileReleaseKotlin FROM-CACHE\n\
                   > Task :effects:compileReleaseKotlin\r\n\
                   > Task :effects:lintRelease SKIPPED\r\n\
                   BUILD SUCCESSFUL in 4s\n";
        assert_eq!(skipped_tasks(log), [":core:lintAnalyzeRelease", ":effects:lintRelease"]);
    }

    #[test]
    fn prose_that_mentions_skipping_is_not_a_task_outcome() {
        let log = "Task :core:lint was SKIPPED by a rule\n  > Task :core:lint SKIPPED\n> Task  SKIPPED\n> Task :core:lint SKIPPED because\n";
        assert!(skipped_tasks(log).is_empty(), "{:?}", skipped_tasks(log));
    }

    #[test]
    fn a_clean_log_passes() {
        assert!(skipped_tasks("> Task :core:assembleRelease\nBUILD SUCCESSFUL\n").is_empty());
    }
}
