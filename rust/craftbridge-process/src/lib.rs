//! Host-only process support, shared by every host tool that starts a process and must leave nothing of it
//! behind: the conformance runner's oracle steps and the pre-push tests' pushes.
//!
//! A process started in a process group of its own takes everything it starts into that group. Stopping
//! only the process leaves its children running, so the group is what gets signalled, and what is waited
//! on. A group of its own is also out of reach of the terminal's Ctrl-C, which goes to the terminal's
//! group, so a [`Group`] is led by a guard that ends it when the program that started it ends first. The
//! code lives here once, so a fix reaches every tool that stops a group. The crate depends on no
//! CraftBridge crate and holds no `unsafe`: the system's `kill`, `pgrep` and `sh` do the signalling, as a
//! shell does.

#![forbid(unsafe_code)]

mod command;
mod group;
mod poll;

pub use command::output_of;
pub use group::{Group, group_has_live_process, signal_group, stop_group};
pub use poll::poll_until;
