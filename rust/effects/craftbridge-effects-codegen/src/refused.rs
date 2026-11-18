//! What the pinned engine refuses while file access is confined, read through EffectCraft's own public
//! calls and never copied: the surface marks every such command and tool, so `diff` can judge what an
//! app can call.
//!
//! An app's calls all reach the engine behind a confinement, so the engine answers every call of a
//! command or tool the confinement refuses with that refusal. EffectCraft keeps both lists of refused
//! names private and answers the question through its public doors instead, and the doors are what this
//! module asks — never a list of what they refuse:
//!
//! - A command: [`Confinement::check_command`] is a pure check that runs nothing. A refused id it
//!   answers at once with one sentence, built from the id, before it looks at any parameter; every
//!   other answer, an error about a path included, is a question about parameters, not a refusal.
//! - A tool: [`tools::run`] refuses a refused tool in that same sentence, built from the name, before
//!   the tool runs. No public check answers the question without running, so a tool that is not refused
//!   runs its own code, and the reading bounds what that code can reach:
//!   - The tools are asked on a throwaway headless session, the same shape CraftBridge runs, confined
//!     to no folder at all.
//!   - Every argument a tool's input schema names is [`OUTSIDE`], a path outside every folder. A tool
//!     hands its arguments to the engine's commands, and the confinement refuses a command given such
//!     a path before the command runs, so a tool that takes an argument stops there, at an argument it
//!     cannot read, or on the empty session itself, which has no composition to render and nothing to
//!     undo. An argument a tool reads as a number never reaches the confinement: `undo` and `redo` meet
//!     only that empty history, and a test asks every tool of the pin and fails if one runs through.
//!     Asked with no arguments instead, a tool runs whole — `list_fonts` reads every font folder of the
//!     machine — which is why none is asked that way.
//!   - A tool whose schema names no argument leaves the confinement nothing to stop, so it is run only
//!     when EffectCraft's own descriptor of it says it only reads, or when it is bridge-only, which the
//!     headless backend refuses before it runs. Any other such tool is never run, and the reading fails
//!     naming it, so a pin that adds one is looked at before anything of it runs. The descriptor's word
//!     promises only that a tool changes nothing, not that it reads nothing beyond the session —
//!     `list_fonts` carries it — so a tool a later pin adds with no argument and that word does run in
//!     that pin's regeneration; it is read by a person once the update loop names this reading's
//!     adaptation for the change to the tools' code.
//!   - A bridge-only tool answers the bridge's own refusal on a headless backend and comes back not
//!     refused: such a tool is hidden from a headless engine anyway, and the surface already says so
//!     with `bridgeOnly`.
//!   - A tool is marked only by the refusal of its own name, never because the commands it runs are
//!     refused, as `run_script`'s one command `script.run` is. Which commands a tool runs is written
//!     only in its own code, which EffectCraft keeps private, and a tool's answer to the arguments it is
//!     asked with names only the command those arguments took it to: `open_project` runs `file.open`
//!     when it is given a path and `file.newProject` when it is told to start a new one. So no answer
//!     read here says that every call of a tool is refused, and such a tool is judged as one an app can
//!     call.

use effectcraft_automation::confine::Confinement;
use effectcraft_automation::tools::{self, ToolDef};
use effectcraft_automation::{Backend, Error};
use serde_json::{Map, Value, json};

/// The sentence both doors answer with for a name the confinement refuses, built from the name as
/// EffectCraft builds it: an answer that is not this is not a refusal.
pub(crate) fn refusal(name: &str) -> String {
    format!("`{name}` is not available while file access is confined (--automation-read-root/--automation-write-root)")
}

/// A confinement over no folder: confined enough to answer with its refusals, and it can name no file
/// of the machine to read or write.
pub(crate) fn confinement() -> Result<Confinement, String> {
    Confinement::new(&[], &[]).map_err(|error| format!("the confinement over no roots was itself refused: {error}"))
}

/// Is this answer of either door the refusal of `name`? Only the refusal's sentence says so: any other
/// answer, an error about a path or about an argument included, is no refusal (module docs).
fn is_refusal(answer: &Error, name: &str) -> bool {
    matches!(answer, Error::BadArgs(text) if *text == refusal(name))
}

/// Does the confinement refuse this command outright?
pub(crate) fn command_refused(confinement: &Confinement, id: &str) -> bool {
    confinement.check_command(id, &json!({})).is_err_and(|answer| is_refusal(&answer, id))
}

/// The value of every argument a tool is asked with: the root folder, an absolute path outside every
/// folder of a confinement over none, which the confinement refuses as a read and as a write, as a
/// file parameter and as any other.
const OUTSIDE: &str = "/";

/// The arguments the tool a descriptor describes is asked with: each one its input schema names, as
/// [`OUTSIDE`]. With none to hand the bound to, the tool is asked only if it cannot run on the headless
/// backend or the descriptor says it only reads (module docs).
fn args_for(name: &str, bridge_only: bool, descriptor: &Value) -> Result<Value, String> {
    let names = descriptor.pointer("/inputSchema/properties").and_then(Value::as_object).into_iter().flat_map(|properties| properties.keys());
    let args: Map<String, Value> = names.map(|name| (name.clone(), Value::String(OUTSIDE.to_string()))).collect();
    let read_only = descriptor.pointer("/annotations/readOnlyHint").and_then(Value::as_bool) == Some(true);
    if args.is_empty() && !bridge_only && !read_only {
        return Err(format!(
            "the tool `{name}` takes no argument the confinement could stop and EffectCraft does not say it only reads, so it is not run to read whether the confinement refuses it"
        ));
    }
    Ok(Value::Object(args))
}

/// The throwaway confined backend the tools are asked on; [`Tools::refused`] answers per tool.
pub(crate) struct Tools {
    backend: Backend,
}

impl Tools {
    pub(crate) fn open() -> Result<Self, String> {
        let confinement = confinement()?;
        let mut backend = Backend::headless(effectcraft_engine::Session::default());
        backend.confine(confinement);
        Ok(Tools { backend })
    }

    /// Does the confinement refuse this tool outright? The refusal answers before the tool's own code
    /// (module docs); anything else the tool answers is its own, not a refusal. A tool the reading
    /// cannot bound is not run, and the answer is why.
    pub(crate) fn refused(&mut self, tool: &ToolDef) -> Result<bool, String> {
        self.ask(tool, &tool.descriptor())
    }

    /// [`Tools::refused`] of a tool as `descriptor` describes it, kept apart so a test can describe a
    /// tool of the pin as one the pin lacks; all else is the tool's own, its name and whether it is
    /// bridge-only, and its arguments are settled before anything of it runs.
    fn ask(&mut self, tool: &ToolDef, descriptor: &Value) -> Result<bool, String> {
        let args = args_for(tool.name, tool.bridge_only, descriptor)?;
        Ok(tools::run(&mut self.backend, tool.name, &args).is_err_and(|answer| is_refusal(&answer, tool.name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tool of the catalogue named `name`.
    fn tool(name: &str) -> &'static ToolDef {
        tools::find(name).unwrap()
    }

    /// The arguments the reading asks a tool of the catalogue with.
    fn bounded_args(tool: &ToolDef) -> Result<Value, String> {
        args_for(tool.name, tool.bridge_only, &tool.descriptor())
    }

    /// The refusal of a script command answers in the module's sentence: the module's words are the
    /// command door's, and a drift between them is a refusal read as no refusal.
    #[test]
    fn the_command_refusal_answers_in_this_sentence() {
        let confinement = confinement().unwrap();
        let error = confinement.check_command("script.run", &json!({})).err();
        let answer = error.map(|error| error.to_string()).unwrap_or_default();
        assert_eq!(answer, refusal("script.run"));
        assert!(command_refused(&confinement, "script.run"));
    }

    /// The refusal of a confined tool answers in the same sentence, from the other door.
    #[test]
    fn the_tool_refusal_answers_in_this_sentence() {
        let mut tools = Tools::open().unwrap();
        let answer = match tools::run(&mut tools.backend, "script_ui", &bounded_args(tool("script_ui")).unwrap()) {
            Err(error) => error.to_string(),
            Ok(_) => String::new(),
        };
        assert_eq!(answer, refusal("script_ui"));
        assert_eq!(tools.refused(tool("script_ui")), Ok(true));
    }

    /// The mark is the backend's answer, not a name held here: the same tool asked on a backend that is
    /// not confined is not refused. Only `script_ui` is asked so: unconfined, nothing bounds a tool,
    /// and `script_ui` stops at its unknown action before it reaches the engine.
    #[test]
    fn a_tool_is_refused_only_by_a_confined_backend() {
        let mut open = Tools { backend: Backend::headless(effectcraft_engine::Session::default()) };
        assert_eq!(open.refused(tool("script_ui")), Ok(false));
        assert_eq!(Tools::open().unwrap().refused(tool("script_ui")), Ok(true));
    }

    /// An engine answer that is an error but not the refusal is a question about parameters, not a
    /// refusal: the surface marks no more than this one sentence marks.
    #[test]
    fn an_error_that_is_not_the_refusal_is_no_refusal() {
        let confinement = confinement().unwrap();
        // `help.exportRepro` writes a file, and the confined check asks for a path it can check: an
        // error over empty parameters, yet the command is callable and stays unmarked.
        assert!(confinement.check_command("help.exportRepro", &json!({})).is_err(), "the door must still answer an error here");
        assert!(!command_refused(&confinement, "help.exportRepro"));
        assert!(!command_refused(&confinement, "edit.undo"));
        // A tool that is not refused fails here for its bounded arguments, and that is no refusal either.
        let mut tools = Tools::open().unwrap();
        assert!(tools::run(&mut tools.backend, "list_fonts", &bounded_args(tool("list_fonts")).unwrap()).is_err());
        assert_eq!(tools.refused(tool("list_fonts")), Ok(false));
    }

    /// The bound holds for every tool of the pin: each can be asked, one that takes an argument and is
    /// not hidden behind the bridge stops with an error — at the confinement, at an argument it cannot
    /// read or on the empty session — and each that takes none and runs whole is one EffectCraft's own
    /// descriptor says only reads.
    #[test]
    fn the_reading_stops_every_tool_that_takes_an_argument() {
        let mut tools = Tools::open().unwrap();
        for tool in tools::catalogue() {
            let args = bounded_args(tool).unwrap();
            let answer = tools::run(&mut tools.backend, tool.name, &args);
            match args.as_object().is_some_and(|args| args.is_empty()) {
                true if !tool.bridge_only => {
                    assert_eq!(tool.descriptor().pointer("/annotations/readOnlyHint"), Some(&json!(true)), "`{}` ran whole", tool.name)
                }
                _ => assert!(answer.is_err(), "`{}` ran with the bounded arguments {args}", tool.name),
            }
        }
    }

    /// A tool whose schema names no argument leaves the confinement nothing to stop: it is run only when
    /// its descriptor says it only reads, or when it is bridge-only, which the headless backend refuses
    /// before it runs. Any other is not run at all, and the reading says why, in these words; every
    /// argument a schema names is asked as the one path outside every folder.
    #[test]
    fn a_tool_with_no_argument_is_run_only_if_it_only_reads() {
        let described = |properties: Value, read_only: bool| json!({"inputSchema": {"type": "object", "properties": properties}, "annotations": {"readOnlyHint": read_only}});
        let unbounded = "the tool `purge` takes no argument the confinement could stop and EffectCraft does not say it only reads, so it is not run to read whether the confinement refuses it";
        assert_eq!(args_for("purge", false, &described(json!({}), false)), Err(unbounded.to_string()));
        assert_eq!(args_for("purge", false, &json!({"inputSchema": {"type": "object"}})), Err(unbounded.to_string()), "no annotation says nothing of reading");
        assert_eq!(args_for("purge", false, &described(json!({}), true)), Ok(json!({})));
        assert_eq!(args_for("purge", true, &described(json!({}), false)), Ok(json!({})));
        let typed = described(json!({"steps": {"type": "integer"}, "path": {"type": "string"}}), false);
        assert_eq!(args_for("purge", false, &typed), Ok(json!({"steps": OUTSIDE, "path": OUTSIDE})));
    }

    /// A tool the reading cannot bound is never run: the reading answers why before anything of the
    /// tool runs, and the session is left as it was. `list_fonts` is described here as taking no
    /// argument and saying nothing of reading; run with none, it would read every font folder of the
    /// machine and leave its step in the session's journal, which is what this test looks for. Whether
    /// a tool is bridge-only is read from the tool itself, so the same description of a bridge-only
    /// tool is asked, and the headless backend refuses it before any of its code runs.
    #[test]
    fn a_tool_the_reading_cannot_bound_is_never_run() {
        let mut tools = Tools::open().unwrap();
        let unbounded = json!({"inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": false}});
        let answer = tools.ask(tool("list_fonts"), &unbounded);
        assert!(answer.as_ref().is_err_and(|why| why.starts_with("the tool `list_fonts` takes no argument")), "{answer:?}");
        assert_eq!(tools.backend.session().map(|session| session.journal.len()), Some(0), "`list_fonts` ran");
        assert_eq!(tools.ask(tool("ui_inspect"), &unbounded), Ok(false));
        // `list_fonts`, described as it is, runs bounded and leaves the journal as empty.
        assert_eq!(tools.refused(tool("list_fonts")), Ok(false));
        assert_eq!(tools.backend.session().map(|session| session.journal.len()), Some(0));
    }
}
