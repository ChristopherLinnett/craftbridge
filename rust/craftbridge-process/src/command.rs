use std::process::{Command, Output};

/// Run `command` to its end and keep what it printed. A program that cannot be started is named in the
/// error, from the command itself, so a missing tool says which one it is wherever it is started.
pub fn output_of(command: &mut Command) -> Result<Output, String> {
    command.output().map_err(|error| format!("could not run {}: {error}", command.get_program().to_string_lossy()))
}
