//! The plug-in side of one step: an app built over the scenario's folder, the step's calls made on it, and
//! the last call's answer printed as the command line prints its own.
//!
//! The folder is the process's working folder, as it is the command line's, and the app's configuration
//! folder is the one the environment names, as the command line reads it from there. The plan arrives as
//! one JSON argument and the step's lines on standard input.

use crate::corpus::{Call, FrameFormat, Host, Progress};
use craftbridge_core::folders::Folders;
use craftbridge_core::gpu::{GpuChoice, GpuDevice};
use craftbridge_core::{CraftApp, CraftError, PixelFormat};
use craftbridge_effects::EffectCraftApp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// What a step asks of the plug-in.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub(crate) host: Host,
    pub(crate) calls: Vec<Call>,
}

/// The variant of an error, as the plug-in's callers tell failures apart. Exhaustive, so a new kind is
/// named here before the runner builds.
fn kind(error: &CraftError) -> &'static str {
    match error {
        CraftError::InvalidParams { .. } => "InvalidParams",
        CraftError::CommandFailed { .. } => "CommandFailed",
        CraftError::NotFound { .. } => "NotFound",
        CraftError::Unsupported { .. } => "Unsupported",
        CraftError::Cancelled => "Cancelled",
        CraftError::Io { .. } => "Io",
        CraftError::Gpu { .. } => "Gpu",
        CraftError::Internal { .. } => "Internal",
    }
}

/// A failed call as the document its step prints: a plug-in call that fails is an answer a scenario can
/// check, as the command line's `--json` failure is, where a step that could not run at all is not.
fn failure(error: &CraftError) -> Value {
    document([("error", Value::String(error.to_string())), ("kind", Value::String(kind(error).to_string()))])
}

/// An object of these entries. `json!` would do, but its expansion calls `unwrap`, which the gate refuses
/// everywhere outside a test.
fn document<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Object(entries.into_iter().map(|(key, value)| (key.to_string(), value)).collect())
}

/// The folders of an app whose container is `container`: the working folders sit inside it, and the
/// configuration folder is the one the environment names.
fn folders(container: &Path, config: PathBuf, host: &Host) -> Folders {
    Folders { config, plugins: host.plugins.as_ref().map(|folder| container.join(folder)), ..Folders::inside(container.to_path_buf(), ".") }
}

/// Makes the plan's calls in order on a new app over `container`, whose configuration folder is `config`,
/// and answers what the step prints: the last call's answer, or the first failure, which ends the step.
/// Lines of standard input are what `mcp` answers.
pub(crate) fn run(plan: &str, stdin: &str, container: &Path, config: PathBuf) -> Result<String, String> {
    let plan: Plan = serde_json::from_str(plan).map_err(|error| format!("the step's calls do not parse: {error}"))?;
    let mut app = match EffectCraftApp::new(&folders(container, config, &plan.host)) {
        Ok(app) => app,
        Err(error) => return Ok(failure(&error).to_string()),
    };
    if plan.host.gpu {
        let gpu = GpuDevice::request(GpuChoice::Preferred);
        // A machine with no device would run the GPU scenario on the CPU and pass it: the step fails instead.
        if gpu.parts().is_none() {
            return Err(format!("the step needs the GPU and the machine gave none: {}", gpu.summary()));
        }
        if let Err(error) = app.attach_gpu(&gpu) {
            return Ok(failure(&error).to_string());
        }
        // The app says where it renders: a step that asked for the GPU and renders on the CPU would compare
        // the CPU with itself.
        let renders_on =
            app.request("info", Value::Object(Default::default())).ok().and_then(|info| info.get("gpu").and_then(Value::as_str).map(str::to_string));
        if renders_on.as_deref() != Some(gpu.summary().as_str()) {
            return Err(format!("the app did not take the GPU: it says it renders on {renders_on:?}, not {}", gpu.summary()));
        }
    }
    let mut printed = String::new();
    for call in &plan.calls {
        match make(&mut app, call, stdin, container) {
            Ok(text) => printed = text,
            Err(error) => return Ok(failure(&error).to_string()),
        }
    }
    Ok(printed)
}

/// One call's answer as the text its step prints.
fn make(app: &mut EffectCraftApp, call: &Call, stdin: &str, container: &Path) -> Result<String, CraftError> {
    match call {
        Call::Request { method, params, files, progress } => {
            let mut params = params.clone();
            for (name, path) in files {
                let text = std::fs::read_to_string(container.join(path)).map_err(|error| CraftError::io(format!("cannot read {path}: {error}")))?;
                match params.as_object_mut() {
                    Some(object) => object.insert(name.clone(), Value::String(text)),
                    None => return Err(CraftError::invalid_params(format!("`{method}`: parameters that take a file must be an object"))),
                };
            }
            match progress {
                None => Ok(app.request(method, params)?.to_string()),
                Some(progress) => with_progress(app, method, params, progress).map(|answer| answer.to_string()),
            }
        }
        Call::Frame { params, format, out } => {
            let wanted = match format {
                FrameFormat::Png => PixelFormat::Png,
                FrameFormat::Rgba8 => PixelFormat::Rgba8Straight,
            };
            let frame = app.frame(params.clone(), wanted)?;
            let bytes = match format {
                FrameFormat::Png => frame.data,
                FrameFormat::Rgba8 => encode_png(frame.width, frame.height, &frame.data)?,
            };
            std::fs::write(container.join(out), bytes).map_err(|error| CraftError::io(format!("cannot write {out}: {error}")))?;
            Ok(document([("width", frame.width.into()), ("height", frame.height.into())]).to_string())
        }
        Call::Mcp {} => Ok(stdin.lines().filter_map(|line| app.mcp(line)).collect::<Vec<_>>().join("\n")),
    }
}

/// A call that runs long: its answer, with the listener of the call told to stop once `progress` says so.
/// The files the call may write are listed to nobody: the core's journal that reads them is not part of
/// the app a step builds.
fn with_progress(app: &mut EffectCraftApp, method: &str, params: Value, progress: &Progress) -> Result<Value, CraftError> {
    let mut listen = |done: u64, total: u64, _left: Option<f64>| !progress.cancels(done, total);
    app.request_with_progress(method, params, &mut listen, &mut |_| Ok(()))
}

/// A PNG holding straight RGBA pixels exactly as they are, which is how a file can be compared with the
/// command line's PNG without either side being converted.
fn encode_png(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, CraftError> {
    let mut bytes = Vec::new();
    image::write_buffer_with_format(&mut std::io::Cursor::new(&mut bytes), pixels, width, height, image::ExtendedColorType::Rgba8, image::ImageFormat::Png)
        .map_err(|error| CraftError::internal(format!("cannot encode the frame: {error}")))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::Corpus;
    use tempfile::TempDir;

    /// The plan of `calls`, written as a corpus's step writes them.
    fn plan(host: &str, calls: &str) -> String {
        let text = format!("[[scenario]]\nid = \"X\"\ntitle = \"x\"\nsides = \"plugin\"\n[[scenario.step]]\n{host}{calls}");
        let corpus = Corpus::parse(&text).expect("parses");
        let step = corpus.scenarios.first().and_then(|scenario| scenario.steps.first()).expect("a step");
        serde_json::to_string(&Plan { host: step.host.clone(), calls: step.calls.clone() }).expect("serialises")
    }

    /// A container with the plan's step run in it: what it printed.
    fn printed(host: &str, calls: &str, stdin: &str) -> (TempDir, Result<String, String>) {
        let scratch = tempfile::tempdir().expect("scratch");
        let config = scratch.path().join(".config");
        let answer = run(&plan(host, calls), stdin, scratch.path(), config);
        (scratch, answer)
    }

    const INFO: &str = "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"info\"\n";

    #[test]
    fn the_last_calls_answer_is_what_the_step_prints() {
        let both = format!("{INFO}[[scenario.step.call]]\nkind = \"request\"\nmethod = \"commands\"\nparams = {{ filter = \"comp.new\" }}\n");
        let (_scratch, answer) = printed("", &both, "");
        let documents = crate::documents::stream(&answer.expect("runs")).expect("JSON");
        assert!(matches!(documents.as_slice(), [Value::Array(commands)] if commands.len() == 1), "{documents:?}");
    }

    #[test]
    fn a_call_that_fails_prints_its_error_and_kind_and_ends_the_step() {
        let calls = "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"nothing\"\n".to_string() + INFO;
        let (_scratch, answer) = printed("", &calls, "");
        let document: Value = serde_json::from_str(&answer.expect("a failure is an answer")).expect("JSON");
        assert_eq!(document["kind"], "Unsupported");
        assert!(document["error"].as_str().is_some_and(|words| words.contains("nothing")), "{document}");
    }

    #[test]
    fn a_file_parameter_is_the_text_of_the_file_and_a_frame_is_written_where_it_says() {
        let scratch = tempfile::tempdir().expect("scratch");
        std::fs::write(scratch.path().join("repro.jsonl"), "{\"start\": \"new\"}\n").expect("written");
        let calls = "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"replay\"\nfiles = { script = \"repro.jsonl\" }\n";
        let answer = run(&plan("", calls), "", scratch.path(), scratch.path().join(".config")).expect("runs");
        assert_eq!(serde_json::from_str::<Value>(&answer).expect("JSON")["ok"], true, "{answer}");
        let out = scratch.path().join("frame.png");
        let frame = "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"run\"\nparams = { steps = [{ command = \"comp.new\", params = { width = 8, height = 6 } }] }\n[[scenario.step.call]]\nkind = \"frame\"\nformat = \"rgba8\"\nout = \"frame.png\"\n";
        let answer = run(&plan("", frame), "", scratch.path(), scratch.path().join(".config")).expect("runs");
        assert_eq!(serde_json::from_str::<Value>(&answer).expect("JSON"), serde_json::json!({"width": 8, "height": 6}));
        assert_eq!(image::open(&out).expect("a PNG").to_rgba8().dimensions(), (8, 6));
    }

    #[test]
    fn mcp_answers_each_line_it_is_given_and_no_reply_for_a_notification() {
        let calls = "[[scenario.step.call]]\nkind = \"mcp\"\n";
        let lines = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n";
        let (_scratch, answer) = printed("", calls, lines);
        let documents = crate::documents::stream(&answer.expect("runs")).expect("JSON");
        assert_eq!(documents.len(), 1, "{documents:?}");
        assert_eq!(documents.first().map(|reply| reply["id"].clone()), Some(serde_json::json!(1)));
    }

    #[test]
    fn a_plug_in_folder_the_host_names_turns_plug_ins_on_and_none_leaves_them_off() {
        let load = "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"exec\"\nparams = { command = \"effect.plugins.list\" }\n";
        let scratch = tempfile::tempdir().expect("scratch");
        std::fs::create_dir(scratch.path().join("plug-ins")).expect("made");
        let on = run(&plan("[scenario.step.host]\nplugins = \"plug-ins\"\n", load), "", scratch.path(), scratch.path().join(".config")).expect("runs");
        assert_eq!(serde_json::from_str::<Value>(&on).expect("JSON")["wasm"], true, "{on}");
        let off = run(&plan("", load), "", scratch.path(), scratch.path().join(".config")).expect("runs");
        assert_eq!(serde_json::from_str::<Value>(&off).expect("JSON")["wasm"], false, "{off}");
    }

    /// The app's working folders sit inside the scenario's container, each in a folder of its own so a
    /// scenario's files and the app's never meet, and the plug-in folder is the one the host names inside it.
    #[test]
    fn the_apps_working_folders_sit_inside_the_container_each_apart() {
        let container = Path::new("/scenario");
        let config = PathBuf::from("/settings");
        let named = Host { plugins: Some("plug-ins".to_string()), gpu: false };
        let app = folders(container, config.clone(), &named);
        assert_eq!((app.container.as_path(), app.config.as_path()), (container, config.as_path()));
        assert_eq!(app.plugins, Some(container.join("plug-ins")));
        let inside: Vec<&Path> = app.working().into_iter().map(|(_, folder)| folder).filter(|folder| *folder != config.as_path()).collect();
        assert_eq!(inside.len(), app.working().len() - 1, "every working folder but the configuration one is the host's own: {inside:?}");
        assert!(inside.iter().all(|folder| folder.starts_with(container) && *folder != container), "{inside:?}");
        assert_eq!(inside.iter().collect::<std::collections::BTreeSet<_>>().len(), inside.len(), "each apart: {inside:?}");
        assert_eq!(folders(container, config, &Host::default()).plugins, None);
    }

    /// The step that asks for the GPU renders on it: the app names the device where it says it renders, which
    /// a machine without a GPU cannot do, and the conformance stage needs one for T8 in any case.
    #[test]
    #[cfg_attr(target_os = "linux", ignore = "a hosted Linux runner has no GPU adapter, which this test asks for")]
    fn a_step_that_asks_for_the_gpu_has_it() {
        let (_scratch, answer) = printed("[scenario.step.host]\ngpu = true\n", INFO, "");
        let info: Value = serde_json::from_str(&answer.expect("the machine has a GPU")).expect("JSON");
        assert!(info["gpu"].as_str().is_some_and(|gpu| !gpu.starts_with("headless")), "{info}");
        let (_scratch, cpu) = printed("", INFO, "");
        let cpu: Value = serde_json::from_str(&cpu.expect("runs")).expect("JSON");
        assert!(cpu["gpu"].as_str().is_some_and(|gpu| gpu.starts_with("headless")), "{cpu}");
    }

    /// `export` runs through the call with progress, which a plain request refuses, and writes what it was
    /// asked for.
    #[test]
    fn a_call_with_progress_runs_export_and_a_plain_request_of_it_is_refused() {
        let scratch = tempfile::tempdir().expect("scratch");
        let out = scratch.path().join("x.png");
        let call = |progress: &str| {
            format!(
                "[[scenario.step.call]]\nkind = \"request\"\nmethod = \"run\"\nparams = {{ steps = [{{ command = \"comp.new\", params = {{ width = 8, height = 6, duration = 0.2, frameRate = 10 }} }}] }}\n[[scenario.step.call]]\nkind = \"request\"\nmethod = \"export\"\nparams = {{ output = \"{}\", format = \"png\" }}\n{progress}",
                out.display()
            )
        };
        let config = scratch.path().join(".config");
        let plain = run(&plan("", &call("")), "", scratch.path(), config.clone()).expect("runs");
        let refused: Value = serde_json::from_str(&plain).expect("JSON");
        assert_eq!(refused["kind"], "Unsupported", "{plain}");
        assert!(!out.exists());
        let answer = run(&plan("", &call("progress = {}\n")), "", scratch.path(), config).expect("runs");
        let rendered: Value = serde_json::from_str(&answer).expect("JSON");
        assert_eq!(rendered["rendered"].as_array().map(Vec::len), Some(1), "{answer}");
        assert!(scratch.path().join("x_00000.png").is_file(), "the export wrote its first frame");
    }
}
