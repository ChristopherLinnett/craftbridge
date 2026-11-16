//! The render runner (AD3): what the command line's `render` does with the render queue, written on
//! EffectCraft's public render-queue commands and `Session` calls, with the command line's loop
//! turned into the progress contract of the core's export call.
//!
//! Every command goes through the plugin's door ([`execute`]), so the confinement checks every output
//! path and the failures map onto their rows as for `exec`. What the command line reads from its
//! session itself — the queue's items, the render's progress and the stop — the runner reads through
//! the same `Session` calls, and it also waits on the render's thread, which no command does: only
//! once that thread has ended has a stopped exporter removed the files it opened.

use craftbridge_core::{CraftError, PROGRESS_INTERVAL, Result};
use effectcraft_automation::Backend;
use effectcraft_engine::Session;
use effectcraft_engine::project::render_queue::RenderStatus;
use serde_json::Value;

use crate::methods::{execute, lacks_capability, object};
use crate::requests::RenderRequest;

/// The caller's progress listener: `(done, total, seconds left)`, answering `false` to stop.
pub(crate) type Progress<'a> = &'a mut dyn FnMut(u64, u64, Option<f64>) -> bool;

/// The command that names a render's failures, as `exec` names the same failure.
const RENDER: &str = "renderQueue.render";

/// How the render ended as the watch saw it.
#[derive(Debug, PartialEq, Eq)]
enum Ended {
    /// The job ran to its end, finished or failed.
    Ran,
    /// The listener answered `false` and the render was stopped.
    Stopped,
}

/// `export`: queue the request (or take the project's own queue), render it in the background, report
/// its progress every [`PROGRESS_INTERVAL`] and answer `{rendered: [{comp, output, seconds}]}`, or
/// stop the render as soon as `on_progress` answers `false`.
///
/// The progress is the current item's frames, as the command line's progress lines give it: a
/// request renders one item, and a queue's items each count their own frames.
pub(crate) fn export(backend: &mut Backend, params: &Value, on_progress: Progress<'_>) -> Result<Value> {
    let request = RenderRequest::from_params(params)?;
    if let Some(queued) = request.to_queue_params()? {
        // The request is the only item the render may write, so every item the project had already
        // ticked for rendering is taken off first, as the command line's `render` takes them off.
        for index in 1..=session(backend)?.project.render_queue.len() {
            execute(backend, "renderQueue.setRender", object([("index", Value::from(index)), ("render", Value::Bool(false))]), None)?;
        }
        execute(backend, "renderQueue.add", queued, None)?;
    }
    let attempted: Vec<u64> = session(backend)?.project.render_queue.iter().filter(|item| item.is_queued()).map(|item| item.id).collect();
    if let Err(refused) = execute(backend, RENDER, object([("wait", Value::Bool(false))]), None) {
        // The engine refuses a render whose every item is refused with a sentence about an empty
        // queue; the reason each item got is the one that says why.
        return Err(match report(session(backend)?, &attempted) {
            Err(reason @ CraftError::Unsupported { .. }) => reason,
            _ => refused,
        });
    }
    let ended = watch(session(backend)?, on_progress);
    let session = session(backend)?;
    session.poll_render();
    if ended == Ended::Stopped {
        return Err(CraftError::Cancelled);
    }
    report(session, &attempted)
}

/// The in-process session behind the backend, which the plugin always builds headless.
fn session(backend: &mut Backend) -> Result<&mut Session> {
    backend.session().ok_or_else(|| CraftError::internal("the render runner needs the in-process session, and this backend has none"))
}

/// Poll the running render at each beat and hand its progress to the listener, until the job ends or
/// the listener answers `false`. A stop waits for the render's thread to wind down: the exporter
/// removes the files it opened only when it sees the stop, so returning earlier would hand back a
/// session whose output still exists and whose render a later export would find running.
fn watch(session: &mut Session, on_progress: Progress<'_>) -> Ended {
    loop {
        session.poll_render();
        let Some(state) = session.render_progress().filter(|_| session.is_rendering()) else { return Ended::Ran };
        if !on_progress(state.done, state.total, state.remaining) {
            session.stop_render();
            if let Some(job) = session.render_job.as_mut() {
                job.wait();
            }
            return Ended::Stopped;
        }
        std::thread::sleep(PROGRESS_INTERVAL);
    }
}

/// The rendered items in the command line's shape, or its failure: every attempted item that did not
/// finish, `name: reason` joined as the command line joins them.
fn report(session: &Session, attempted: &[u64]) -> Result<Value> {
    let (mut rendered, mut failed, mut unavailable) = (Vec::new(), Vec::new(), true);
    for item in session.project.render_queue.iter().filter(|item| attempted.contains(&item.id)) {
        let name = session.project.item(item.comp).map_or_else(|| "?".to_string(), |comp| comp.name.clone());
        match &item.status {
            RenderStatus::Done => rendered.push(object([
                ("comp", Value::from(name)),
                ("output", Value::from(item.last_output.clone())),
                ("seconds", Value::from(item.render_time)),
            ])),
            RenderStatus::Failed(reason) => {
                // An export format the build lacks is `Unsupported`, as the same refusal is anywhere.
                unavailable &= lacks_capability(reason);
                failed.push(format!("{name}: {reason}"));
            }
            other => {
                unavailable = false;
                failed.push(format!("{name}: {}", other.label()));
            }
        }
    }
    match (failed.is_empty(), unavailable) {
        (true, _) => Ok(object([("rendered", Value::Array(rendered))])),
        (false, true) => Err(CraftError::unsupported(failed.join("; "))),
        (false, false) => Err(CraftError::command_failed(RENDER, failed.join("; "), None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use effectcraft_engine::project::render_queue::OutputFormat;
    use effectcraft_engine::render_queue::{CANCELLED, ExportJob, ExportResult, Exporter};

    /// How long the fake exporter takes over a frame: the code under test is slowed, never the
    /// machine, so the runner's beats fall inside the export however fast the machine is.
    const FRAME: std::time::Duration = std::time::Duration::from_millis(30);

    /// An exporter that writes its output first, takes its time over each frame and removes the
    /// output when stopped, as the real one does, so the runner is tested on its own logic.
    struct Slow {
        frames: u64,
        written: Arc<AtomicBool>,
        started: Arc<AtomicU64>,
    }

    impl Exporter for Slow {
        fn formats(&self) -> Vec<OutputFormat> {
            vec![OutputFormat::PngSequence, OutputFormat::H264]
        }
        fn export(&self, job: &ExportJob, progress: &mut dyn FnMut(u64, u64) -> bool) -> std::result::Result<ExportResult, String> {
            self.started.fetch_add(1, Ordering::SeqCst);
            std::fs::write(job.path, b"partial").map_err(|error| error.to_string())?;
            self.written.store(true, Ordering::SeqCst);
            for done in 0..self.frames {
                std::thread::sleep(FRAME);
                if !progress(done, self.frames) {
                    let _ = std::fs::remove_file(job.path);
                    return Err(CANCELLED.into());
                }
            }
            Ok(ExportResult { path: job.path.to_string(), frames: self.frames, ..Default::default() })
        }
    }

    struct Rig {
        backend: Backend,
        written: Arc<AtomicBool>,
        started: Arc<AtomicU64>,
        dir: tempfile::TempDir,
    }

    fn rig(frames: u64) -> Rig {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let (written, started) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU64::new(0)));
        let exporter = Slow { frames, written: written.clone(), started: started.clone() };
        let session = Session { exporter: Some(Arc::new(exporter)), ..Default::default() };
        let mut backend = Backend::headless(session);
        for (command, params) in [
            ("comp.new", serde_json::json!({"name": "A", "width": 16, "height": 16, "duration": 1, "frameRate": 10})),
            ("layer.newSolid", serde_json::json!({"comp": 1, "name": "Bg"})),
        ] {
            backend.exec(command, params).expect("the project builds");
        }
        Rig { backend, written, started, dir }
    }

    fn out(rig: &Rig, name: &str) -> String {
        rig.dir.path().join(name).display().to_string()
    }

    fn queue(rig: &mut Rig) -> Vec<(u64, bool, String)> {
        let session = rig.backend.session().expect("a session");
        session.project.render_queue.iter().map(|item| (item.id, item.render, item.status.label().to_string())).collect()
    }

    #[test]
    fn the_listener_hears_the_frames_and_the_item_renders_to_its_end() {
        let mut rig = rig(5);
        let mut heard = Vec::new();
        let output = out(&rig, "a.mp4");
        let answer = export(&mut rig.backend, &serde_json::json!({"output": output, "format": "h264"}), &mut |done, total, _| {
            heard.push((done, total));
            true
        })
        .expect("the export renders");
        assert_eq!(answer["rendered"][0]["comp"], "A");
        assert_eq!(answer["rendered"][0]["output"], output.as_str());
        assert!(heard.iter().all(|(_, total)| *total == 5 || *total == 0) && !heard.is_empty(), "{heard:?}");
    }

    #[test]
    fn the_projects_other_items_are_unqueued_and_this_one_is_the_only_one_rendered() {
        let mut rig = rig(1);
        let other = out(&rig, "other.mp4");
        rig.backend.exec("renderQueue.add", serde_json::json!({"output": other, "format": "h264"})).expect("another item is queued");
        let output = out(&rig, "mine.mp4");
        export(&mut rig.backend, &serde_json::json!({"output": output, "format": "h264"}), &mut |_, _, _| true).expect("the export renders");
        assert_eq!(rig.started.load(Ordering::SeqCst), 1, "only the requested item reached the exporter");
        let items = queue(&mut rig);
        assert_eq!(items, [(1, false, "Unqueued".to_string()), (2, true, "Done".to_string())]);
        assert!(!Path::new(&other).exists());
    }

    #[test]
    fn a_queue_request_renders_the_projects_own_queue_and_adds_nothing() {
        let mut rig = rig(1);
        let own = out(&rig, "own.mp4");
        rig.backend.exec("renderQueue.add", serde_json::json!({"output": own, "format": "h264"})).expect("an item is queued");
        let answer = export(&mut rig.backend, &serde_json::json!({"queue": true, "output": "ignored"}), &mut |_, _, _| true).expect("the queue renders");
        assert_eq!(answer["rendered"].as_array().map(Vec::len), Some(1));
        assert_eq!(queue(&mut rig).len(), 1, "nothing was added");
    }

    #[test]
    fn the_first_false_stops_the_render_and_leaves_no_output() {
        let mut rig = rig(1000);
        let written = rig.written.clone();
        let output = out(&rig, "c.mp4");
        let mut calls = 0;
        let stopped = export(&mut rig.backend, &serde_json::json!({"output": output, "format": "h264"}), &mut |_, _, _| {
            calls += 1;
            // Stop only once the exporter has written its output, so the stop has something to remove.
            !(calls >= 3 && written.load(Ordering::SeqCst))
        });
        assert_eq!(stopped, Err(CraftError::Cancelled));
        assert!(!Path::new(&output).exists(), "a stopped render leaves no output");
        assert!(!rig.backend.session().expect("a session").is_rendering(), "the render's thread has wound down on return");
        assert_eq!(queue(&mut rig), [(1, true, "User Stopped".to_string())]);
    }

    #[test]
    fn a_failed_item_fails_the_export_in_the_command_lines_words() {
        let mut rig = rig(1);
        // The fake exporter cannot write inside a folder that is not there.
        let output = out(&rig, "missing/x.mp4");
        let error = export(&mut rig.backend, &serde_json::json!({"output": output, "format": "h264"}), &mut |_, _, _| true).expect_err("the item fails");
        match error {
            CraftError::CommandFailed { command, message, step } => {
                assert_eq!((command.as_str(), step), (RENDER, None));
                assert!(message.starts_with("A: "), "{message}");
            }
            other => panic!("a failed item is `CommandFailed`, got {other:?}"),
        }
    }

    #[test]
    fn a_format_the_exporter_lacks_is_unsupported() {
        let mut rig = rig(1);
        let output = out(&rig, "x.gif");
        let error = export(&mut rig.backend, &serde_json::json!({"output": output, "format": "gif"}), &mut |_, _, _| true).expect_err("not offered");
        match error {
            CraftError::Unsupported { message } => assert!(message.starts_with("A: ") && message.ends_with("export is not available"), "{message}"),
            other => panic!("an export format the build lacks is `Unsupported`, got {other:?}"),
        }
    }
}
