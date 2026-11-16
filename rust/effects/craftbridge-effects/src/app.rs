//! [`EffectCraftApp`]: the construction of plan §19 and the dispatch of the §14 method table.

use craftbridge_core::folders::Folders;
use craftbridge_core::{CraftApp, CraftError, Result};
// The core and the engine both call their type `Confinement`: the core's carries the roots the app's
// folders make, the engine's checks every access against those same roots. They meet in this file
// alone, so the engine's is renamed here to keep the two apart.
use effectcraft_automation::confine::Confinement as EngineConfinement;
use effectcraft_automation::{Backend, McpServer};
use serde_json::Value;

use crate::{info, methods, session};

/// A hosted EffectCraft session: the MCP server built at construction, which owns the headless
/// [`Backend`] and the confined session behind it (plan §19).
///
/// The core moves the app onto the engine thread and reaches it only through [`CraftApp::request`],
/// so nothing here needs a lock: one call at a time is what the engine thread already gives it.
pub struct EffectCraftApp {
    /// Every method reaches the session through this server's backend, so the app keeps the server
    /// itself rather than the backend inside it: the `mcp` method will answer its lines from here,
    /// and an app that wired the line transport around a copy of the backend would fork the session.
    server: McpServer,
}

impl EffectCraftApp {
    /// Build the app from the folders the app named (plan §19, steps 1 to 4): prepare the folders
    /// and make the confinement they derive, build the session, run it headless and confined, and
    /// serve it through the MCP server every method reaches it by.
    ///
    /// The folders are prepared first because the engine's confinement resolves its roots on disk,
    /// and a root that is not there yet would be refused before the folder that would be it exists.
    pub fn new(folders: &Folders) -> Result<Self> {
        let roots = folders.prepare()?;
        let mut backend = Backend::headless(session::build(folders));
        let confinement = EngineConfinement::new(roots.read_roots(), &[roots.write_root().to_path_buf()]).map_err(|error| CraftError::io(error.to_string()))?;
        backend.confine(confinement);
        Ok(Self { server: McpServer::new(backend) })
    }
}

/// One method's answer: the backend the session sits behind, and the parameters the call carried.
type Method = fn(&mut Backend, &Value) -> Result<Value>;

/// The methods of the plan's §14 table `request` answers, by name. It is the one list of them: the
/// dispatch reads it, and the refusal of every other name recites it, so the refusal names what
/// `request` answers truly whatever a build adds to it.
const METHODS: [(&str, Method); 4] = [("info", info::info), ("commands", methods::commands), ("exec", methods::exec), ("run", methods::run)];

impl CraftApp for EffectCraftApp {
    /// Answer one method of the plan's §14 table through the function [`METHODS`] names for it.
    /// Every other name — a §14 method `request` does not answer, or no method at all — answers
    /// `Unsupported`, the row of a capability this build lacks, naming what was asked and the
    /// methods `request` does answer: a newer app asking an older build for a method it lacks is
    /// the case the refusal is for. It speaks of `request` alone because the core reaches the app
    /// through calls of its own for the binary `frame`, the line `mcp` and a call with progress
    /// (plan §16), so a name refused here can be one the app answers through another call.
    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        match METHODS.iter().find(|(name, _)| *name == method) {
            Some((_, answer)) => answer(self.server.backend(), &params),
            None => Err(CraftError::unsupported(not_answered(method))),
        }
    }
}

/// The refusal of a name `request` does not answer: true of any name in any build, since it says
/// only that `request` lacks it and which methods `request` has, read from [`METHODS`].
fn not_answered(method: &str) -> String {
    let names: Vec<String> = METHODS.iter().map(|(name, _)| format!("`{name}`")).collect();
    let answered = match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => "nothing".to_string(),
    };
    format!("`{method}` is not answered by `request` in this build, which answers {answered}")
}
