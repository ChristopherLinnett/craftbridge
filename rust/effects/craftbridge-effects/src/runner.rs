//! The render runner (AD3): what the command line's `render` does with the render queue, written on
//! EffectCraft's public render-queue commands and `Session` calls, with the command line's loop
//! turned into the progress contract of the core's export call.
//!
//! Every command goes through the plugin's door ([`execute`]), so the confinement checks every output
//! path and the failures map onto their rows as for `exec`. What the command line reads from its
//! session itself — the queue's items, the render's progress and the stop — the runner reads through
//! the same `Session` calls, and it also waits on the render's thread, which no command does: only
//! once that thread has ended has a stopped exporter removed the files it opened.
//!
//! The runner is also the only caller that starts a render (a fill has its own runner, `fill.rs`): it
//! lists every file the render may write before it starts, which the door refuses every other call to
//! do, so a render the system kills leaves files the next start can delete.

use craftbridge_core::{CraftError, PROGRESS_INTERVAL, Result};
use effectcraft_automation::Backend;
use effectcraft_engine::Session;
use effectcraft_engine::project::render_queue::RenderStatus;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::methods::{RENDER, execute, execute_listed, lacks_capability, object};
use crate::outputs;
use crate::requests::RenderRequest;

/// The caller's progress listener: `(done, total, seconds left)`, answering `false` to stop.
pub(crate) type Progress<'a> = &'a mut dyn FnMut(u64, u64, Option<f64>) -> bool;

/// The core's lister of the files an export may write: each call adds to the export's list.
pub(crate) type ListOutputs<'a> = &'a mut dyn FnMut(&[PathBuf]) -> Result<()>;

/// How long a wait for a stopped job sleeps between its looks: a stopped job reports at its next frame,
/// which is soon, so this is short.
const WIND_DOWN: Duration = Duration::from_millis(2);

/// How long a panic's unwinding waits for the job it stopped. A job that never reaches its next look at
/// the stop would otherwise hold the engine's thread for ever, turning one caught panic into a hang of
/// every later call. Past this the panic unwinds with the job told to stop: when it does look, the
/// exporter removes the files it opened and the fill the frames it wrote.
const PANIC_WAIT: Duration = Duration::from_secs(if cfg!(test) { 2 } else { 10 });

/// How a render or a fill ended as the watch saw it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Ended {
    /// The job ran to its end, finished or failed.
    Ran,
    /// The listener answered `false` and the job was stopped.
    Stopped,
}

/// Where an `export` call's render request sits: its parameters are the request.
pub(crate) fn export_request(params: &Value) -> Option<&Value> {
    Some(params)
}

/// `export`: queue the request (or take the project's own queue), list every file the render may
/// write with `list_outputs`, render it in the background, report
/// its progress every [`PROGRESS_INTERVAL`] and answer `{rendered: [{comp, output, seconds}]}`, or
/// stop the render as soon as `on_progress` answers `false`. A queue's failure and its stop alike list
/// again the files of the items it finished, so the core keeps them.
///
/// The progress is the current item's frames, as the command line's progress lines give it: a
/// request renders one item, and a queue's items each count their own frames.
pub(crate) fn export(backend: &mut Backend, params: &Value, on_progress: Progress<'_>, list_outputs: ListOutputs<'_>) -> Result<Value> {
    let request = RenderRequest::from_params(params)?;
    let queued = request.to_queue_params()?;
    let single = queued.is_some();
    if let Some(queued) = queued {
        // The request is the only item the render may write, so every item the project had already
        // ticked for rendering is taken off first, as the command line's `render` takes them off.
        for index in 1..=session(backend)?.project.render_queue.len() {
            execute(backend, None, "renderQueue.setRender", object([("index", Value::from(index)), ("render", Value::Bool(false))]), None)?;
        }
        execute(backend, None, "renderQueue.add", queued, None)?;
    }
    let attempted: Vec<u64> = session(backend)?.project.render_queue.iter().filter(|item| item.is_queued()).map(|item| item.id).collect();
    // The file names are known only now: the render writes `card_00000.png` on for a PNG sequence asked
    // for at `card.png`. A refusal to list stops the export before any file is written.
    //
    // The listing of a sequence of thousands of files is the export's longest step before its first
    // frame, and the render's own watch reports only once it is over, so the listing keeps the beat
    // itself: each chunk is reported before the journal is handed it, the first before the journal
    // looks at a single file, and a listener that answers `false` stops the export there, where
    // nothing is written and nothing has to be taken back. Nothing is done yet and no frame has been
    // timed, so each of those reports says so and carries no estimate.
    outputs::list_queued(session(backend)?, &mut |paths| {
        if on_progress(0, 0, None) { list_outputs(paths) } else { Err(CraftError::Cancelled) }
    })?;
    if let Err(refused) = execute_listed(backend, RENDER, object([("wait", Value::Bool(false))])) {
        // The engine refuses a render whose every item is refused with a sentence about an empty
        // queue; the reason each item got is the one that says why.
        return Err(match report(session(backend)?, &attempted) {
            Err(reason @ CraftError::Unsupported { .. }) => reason,
            _ => refused,
        });
    }
    let ended = watching(session(backend)?, stop_and_wait, |session| watch(session, on_progress));
    let session = session(backend)?;
    session.poll_render();
    let answer = if ended == Ended::Stopped { Err(CraftError::Cancelled) } else { report(session, &attempted) };
    // A single export has no finished part to keep: a stop that lands as its one item finishes, which
    // the engine then reports `Done`, leaves nothing like any other stop.
    if answer.is_err() && !single {
        // A queue renders on past a failed item and stops at a cancel, and its status says which items
        // finished and where, so their files stay. Should this listing be refused they are deleted with
        // the rest, and the caller still hears the render's own end, which is what it needs.
        let _ = outputs::list_finished(session, &attempted, list_outputs);
    }
    answer
}

/// Run `watch` over a job running on `session`, and `stop` it should `watch` panic, waiting for it to end
/// for at most [`PANIC_WAIT`]. The answer to a panic deletes what the export listed, so a job left
/// running would write its next files after that, where no list names them.
pub(crate) fn watching<T>(session: &mut Session, stop: impl FnMut(&mut Session, Option<Duration>), watch: impl FnOnce(&mut Session) -> T) -> T {
    struct StopOnPanic<'s, F: FnMut(&mut Session, Option<Duration>)> {
        session: &'s mut Session,
        stop: F,
    }
    impl<F: FnMut(&mut Session, Option<Duration>)> Drop for StopOnPanic<'_, F> {
        fn drop(&mut self) {
            if std::thread::panicking() {
                // A stop that panics as well while this one unwinds would abort the process, so it is caught
                // here and what it panicked with is leaked rather than dropped, since that is app code too.
                if let Err(again) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.stop)(self.session, Some(PANIC_WAIT)))) {
                    std::mem::forget(again);
                }
            }
        }
    }
    let guard = StopOnPanic { session, stop };
    watch(&mut *guard.session)
}

/// Look until `ended` answers yes, for at most `limit` when there is one.
pub(crate) fn wait_for_end(limit: Option<Duration>, mut ended: impl FnMut() -> bool) {
    let started = Instant::now();
    while !ended() {
        if limit.is_some_and(|limit| started.elapsed() >= limit) {
            return;
        }
        std::thread::sleep(WIND_DOWN);
    }
}

/// Stop the render and wait for its thread to end, for at most `limit` when there is one: the exporter
/// removes the files it opened only when it sees the stop, and holds them open until then.
fn stop_and_wait(session: &mut Session, limit: Option<Duration>) {
    session.stop_render();
    let Some(job) = session.render_job.as_mut() else { return };
    if limit.is_some() {
        wait_for_end(limit, || job.is_finished());
        if !job.is_finished() {
            return;
        }
    }
    job.wait();
}

/// The in-process session behind the backend, which the plugin always builds headless.
pub(crate) fn session(backend: &mut Backend) -> Result<&mut Session> {
    backend.session().ok_or_else(|| CraftError::internal("the runners need the in-process session, and this backend has none"))
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
            stop_and_wait(session, None);
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
    use std::collections::BTreeSet;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    use effectcraft_engine::project::render_queue::OutputFormat;
    use effectcraft_engine::render_queue::{CANCELLED, ExportJob, ExportResult, Exporter};

    use crate::outputs::tests::files_below;

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

    /// A lister for the tests that are not about the listing.
    fn unlisted(_: &[PathBuf]) -> Result<()> {
        Ok(())
    }

    struct Rig {
        backend: Backend,
        written: Arc<AtomicBool>,
        started: Arc<AtomicU64>,
        dir: tempfile::TempDir,
    }

    fn rig(frames: u64) -> Rig {
        rig_on(frames, |slow| Arc::new(slow))
    }

    /// The rig, with the session exporting through what `exporter` makes of its slow exporter.
    fn rig_on(frames: u64, exporter: impl FnOnce(Slow) -> Arc<dyn Exporter>) -> Rig {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let (written, started) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU64::new(0)));
        let exporter = exporter(Slow { frames, written: written.clone(), started: started.clone() });
        let session = Session { exporter: Some(exporter), ..Default::default() };
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
        let answer = export(
            &mut rig.backend,
            &serde_json::json!({"output": output, "format": "h264"}),
            &mut |done, total, _| {
                heard.push((done, total));
                true
            },
            &mut unlisted,
        )
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
        export(&mut rig.backend, &serde_json::json!({"output": output, "format": "h264"}), &mut |_, _, _| true, &mut unlisted).expect("the export renders");
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
        let answer =
            export(&mut rig.backend, &serde_json::json!({"queue": true, "output": "ignored"}), &mut |_, _, _| true, &mut unlisted).expect("the queue renders");
        assert_eq!(answer["rendered"].as_array().map(Vec::len), Some(1));
        assert_eq!(queue(&mut rig).len(), 1, "nothing was added");
    }

    /// The files of a long sequence's export: more than one chunk of the listing, so a cancel can
    /// fall inside the listing, and the count the acceptance names.
    const MANY: u32 = 18_000;

    /// The comp of [`MANY`] frames, queued as a sequence of files: the export's own listing is what
    /// takes the time before its first frame, so a test of the cancel reaching it needs a sequence,
    /// not the short ones the other tests of this module export.
    fn queue_many(rig: &mut Rig) -> String {
        rig.backend
            .exec("comp.new", serde_json::json!({"name": "Long", "width": 16, "height": 16, "duration": 1800, "frameRate": 10}))
            .expect("the long comp builds");
        out(rig, "many.png")
    }

    /// The export's start answers a cancel however many files it writes: the listing reports as it
    /// goes, and a listener that answers `false` on one of those reports stops the export there —
    /// the render never starts, so there is nothing to take back out again. The listing of a
    /// sequence of thousands of files is the longest step between the export's start and its first
    /// frame, and an export deaf through it can be ended by the system, a background task's time
    /// run out, before any cancel reaches it.
    #[test]
    fn an_export_cancelled_while_its_files_are_listed_never_starts_the_render() {
        let mut rig = rig(1);
        let output = queue_many(&mut rig);
        // The listing counts itself, and the listener stops at the first report that finds files
        // listed. A report can find some files listed only while they are being listed: the render's
        // reports come after the last of them, by when the render has already started. The stop
        // stays stopped, as a cancel does, so a build that hears the answer and goes on regardless
        // is stopped by the next report too.
        let (listed, stopped) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
        let (told, asked) = (listed.clone(), stopped.clone());
        let answer = export(
            &mut rig.backend,
            &serde_json::json!({"comp": "Long", "output": output, "format": "png"}),
            &mut |_, _, _| {
                if told.load(Ordering::SeqCst) > 0 {
                    asked.store(true, Ordering::SeqCst);
                }
                !asked.load(Ordering::SeqCst)
            },
            &mut |paths| {
                told.fetch_add(paths.len(), Ordering::SeqCst);
                Ok(())
            },
        );
        assert_eq!(answer, Err(CraftError::Cancelled), "the cancel taken in the listing ends the export");
        assert_eq!(rig.started.load(Ordering::SeqCst), 0, "a cancel taken in the listing never starts the render");
        let listed = listed.load(Ordering::SeqCst);
        assert!(listed > 0 && listed < MANY as usize, "the listing stopped where the cancel came, not ran out: {listed} of {MANY} files were listed");
        assert_eq!(files_below(rig.dir.path()), BTreeSet::new(), "an export stopped in its listing leaves no file behind");
    }

    /// The listing keeps the export's beat: the first report comes before the journal is handed a
    /// single file, and every chunk after it is reported before it is handed over, so a listener has
    /// a report to answer `false` to before every look the journal takes.
    #[test]
    fn the_listing_reports_before_the_journal_is_handed_a_file_and_before_every_chunk() {
        let mut rig = rig(1);
        let output = queue_many(&mut rig);
        // Each report, as `None`, and each chunk handed to the journal, as its count of files.
        let heard = std::cell::RefCell::new(Vec::new());
        export(
            &mut rig.backend,
            &serde_json::json!({"comp": "Long", "output": output, "format": "png"}),
            &mut |_, _, _| {
                heard.borrow_mut().push(None);
                true
            },
            &mut |paths| {
                heard.borrow_mut().push(Some(paths.len()));
                Ok(())
            },
        )
        .expect("the export renders");
        let heard = heard.into_inner();
        let chunks: Vec<usize> = heard.iter().flatten().copied().collect();
        assert!(chunks.len() > 1 && chunks.iter().sum::<usize>() > MANY as usize, "every file was listed, in chunks: {chunks:?}");
        // One report per chunk, not two: a second report with no chunk between says nothing the
        // listener has not just heard, and a start that reported twice before its first file was such a noise.
        let last_chunk = heard.iter().rposition(Option::is_some).expect("the export handed over a chunk");
        let expected: Vec<Option<usize>> = chunks.iter().flat_map(|files| [None, Some(*files)]).collect();
        assert_eq!(&heard[..=last_chunk], &expected[..], "the listing reported once before every chunk, and only once: {heard:?}");
    }

    /// Whether the memory of [`too_little_memory`] has been asked for its figure.
    static MEMORY_ASKED: AtomicBool = AtomicBool::new(false);

    /// A device whose memory holds no frame at all, which notes that it was asked.
    fn too_little_memory() -> std::result::Result<u64, String> {
        MEMORY_ASKED.store(true, Ordering::SeqCst);
        Ok(1)
    }

    /// An export whose frame the device's memory cannot hold is refused by the exporter as each job
    /// starts, from the job the render makes, which exists only once the render has begun: so the
    /// listing comes first, and every file of the export is listed while the memory has not yet been
    /// asked. The refusal is the export's answer, the exporter that writes is never reached, and
    /// nothing is written; the core ends the entries of an export that fails, so nothing of it stays
    /// listed either.
    #[test]
    fn an_export_the_memory_cannot_hold_is_listed_before_it_is_refused_and_writes_nothing() {
        let mut rig = rig_on(1, |slow| Arc::new(crate::exporter::PooledExporter::over(Arc::new(slow), too_little_memory)));
        let output = out(&rig, "card.png");
        let mut listed = 0;
        let answer = export(&mut rig.backend, &serde_json::json!({"output": output, "format": "png"}), &mut |_, _, _| true, &mut |paths| {
            // Time for a render started before the listing ended to have asked the memory.
            std::thread::sleep(std::time::Duration::from_millis(200));
            assert!(!MEMORY_ASKED.load(Ordering::SeqCst), "the memory was asked before every file of the export was listed");
            listed += paths.len();
            Ok(())
        });
        assert!(listed > 0 && MEMORY_ASKED.load(Ordering::SeqCst), "the export was listed, {listed} files, and then held to the memory");
        assert_eq!(
            answer,
            Err(CraftError::command_failed(
                RENDER,
                "A: a frame of 16x16 needs 16384 bytes to render on the CPU and write, and the device reports 1 bytes available",
                None
            ))
        );
        assert_eq!(rig.started.load(Ordering::SeqCst), 0, "the refused export reached the exporter that writes");
        assert_eq!(files_below(rig.dir.path()), BTreeSet::new(), "a refused export writes nothing");
    }

    #[test]
    fn the_first_false_stops_the_render_and_leaves_no_output() {
        let mut rig = rig(1000);
        let written = rig.written.clone();
        let output = out(&rig, "c.mp4");
        let mut calls = 0;
        let stopped = export(
            &mut rig.backend,
            &serde_json::json!({"output": output, "format": "h264"}),
            &mut |_, _, _| {
                calls += 1;
                // Stop only once the exporter has written its output, so the stop has something to remove.
                !(calls >= 3 && written.load(Ordering::SeqCst))
            },
            &mut unlisted,
        );
        assert_eq!(stopped, Err(CraftError::Cancelled));
        assert!(!Path::new(&output).exists(), "a stopped render leaves no output");
        assert!(!rig.backend.session().expect("a session").is_rendering(), "the render's thread has wound down on return");
        assert_eq!(queue(&mut rig), [(1, true, "User Stopped".to_string())]);
    }

    /// A panic in the watch, as one in a post-render import that `poll_render` runs on this thread,
    /// stops the render and waits for it: the answer to a panic deletes what the export listed, so a
    /// render left running would write its next files after that, where no list names them.
    #[test]
    fn a_panic_in_the_watch_stops_the_render_before_the_call_unwinds() {
        let mut rig = rig(1000);
        let written = rig.written.clone();
        let output = out(&rig, "p.mp4");
        let request = serde_json::json!({"output": output, "format": "h264"});
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            export(
                &mut rig.backend,
                &request,
                &mut |_, _, _| {
                    assert!(!written.load(Ordering::SeqCst), "boom in the beat");
                    true
                },
                &mut unlisted,
            )
        }));
        assert!(unwound.is_err(), "the panic did not reach the call");
        assert!(!rig.backend.session().expect("a session").is_rendering(), "the render ran on after the call had unwound");
        assert!(!Path::new(&output).exists(), "the stopped render left its file");
    }

    /// An exporter that writes its whole file at once, then holds until `let_go`, asked with the engine's
    /// progress call, answers yes, so a test holds it at the point it names on any machine. Let go, it
    /// looks at the stop once more when `looks` says so; otherwise it is past its last look and finishes.
    struct Held {
        let_go: Box<dyn Fn(&mut dyn FnMut(u64, u64) -> bool) -> bool + Send + Sync>,
        looks: bool,
    }

    impl Held {
        /// Past its last look at the stop: it waits for the stop, then finishes whatever it was told.
        fn finishing() -> Self {
            Self { let_go: Box::new(|progress| !progress(1, 2)), looks: false }
        }

        /// Stuck on a frame: it takes no look at the stop until `released` is set.
        fn stuck(released: Arc<AtomicBool>) -> Self {
            Self { let_go: Box::new(move |_| released.load(Ordering::SeqCst)), looks: true }
        }
    }

    impl Exporter for Held {
        fn formats(&self) -> Vec<OutputFormat> {
            vec![OutputFormat::H264]
        }
        fn export(&self, job: &ExportJob, progress: &mut dyn FnMut(u64, u64) -> bool) -> std::result::Result<ExportResult, String> {
            std::fs::write(job.path, b"the movie").map_err(|error| error.to_string())?;
            let _ = progress(1, 2);
            let held = Instant::now();
            while !(self.let_go)(progress) && held.elapsed() < Duration::from_secs(30) {
                std::thread::sleep(FRAME);
            }
            if self.looks && !progress(2, 2) {
                let _ = std::fs::remove_file(job.path);
                return Err(CANCELLED.into());
            }
            Ok(ExportResult { path: job.path.to_string(), frames: 2, ..Default::default() })
        }
    }

    /// A rig whose queue holds an item at each of `outputs`, over an exporter that finishes each item after
    /// its last look at the stop.
    fn finishing_queue(outputs: &[&str]) -> (Rig, Vec<String>) {
        let mut rig = rig_on(1, |_| Arc::new(Held::finishing()));
        let outputs: Vec<String> = outputs.iter().map(|name| out(&rig, name)).collect();
        for output in &outputs {
            rig.backend.exec("renderQueue.add", serde_json::json!({"output": output, "format": "h264"})).expect("the item is queued");
        }
        (rig, outputs)
    }

    /// Export `request`, stopped once its first frame is reported, after the exporter's last look at the
    /// stop, and answer what the export listed, in order, the engine's own word on its items and how many
    /// times it listed `output`.
    fn stopped_as_it_finishes(rig: &mut Rig, request: &Value) -> (Vec<PathBuf>, Vec<String>) {
        let mut listed = Vec::new();
        let stopped = export(&mut rig.backend, request, &mut |done, _, _| done == 0, &mut |paths: &[PathBuf]| {
            listed.extend_from_slice(paths);
            Ok(())
        });
        assert_eq!(stopped, Err(CraftError::Cancelled));
        (listed, queue(rig).into_iter().map(|item| item.2).collect())
    }

    /// How many times `listed` names `output`.
    fn listings(listed: &[PathBuf], output: &str) -> usize {
        listed.iter().filter(|path| path.as_path() == Path::new(output)).count()
    }

    /// A single export stopped just as its render finishes is told `Cancelled`, and leaves nothing, though
    /// the engine reports its one item `Done`: its file is not listed again as finished.
    #[test]
    fn a_single_export_stopped_as_it_finishes_keeps_nothing() {
        let (mut rig, _) = finishing_queue(&[]);
        let output = out(&rig, "single.mp4");
        let (listed, statuses) = stopped_as_it_finishes(&mut rig, &serde_json::json!({"output": output, "format": "h264"}));
        assert_eq!(statuses, ["Done"], "the stop landed too early to test this");
        assert_eq!(listings(&listed, &output), 1, "a cancelled single export kept its file");
    }

    /// A queue stopped just as its last item finishes keeps that item, which it reports `Done`.
    #[test]
    fn a_queue_stopped_as_its_last_item_finishes_keeps_it() {
        let (mut rig, outputs) = finishing_queue(&["last.mp4"]);
        let (listed, statuses) = stopped_as_it_finishes(&mut rig, &serde_json::json!({"queue": true}));
        assert_eq!(statuses, ["Done"], "the stop landed too early to test this");
        assert_eq!(listings(&listed, &outputs[0]), 2, "the finished item was not listed again");
    }

    /// A queue stopped between two items, as the first finishes, keeps the first and never starts the
    /// second.
    #[test]
    fn a_queue_stopped_between_two_items_keeps_the_first_and_starts_no_other() {
        let (mut rig, outputs) = finishing_queue(&["first.mp4", "second.mp4"]);
        let (listed, statuses) = stopped_as_it_finishes(&mut rig, &serde_json::json!({"queue": true}));
        assert_eq!(statuses, ["Done", "User Stopped"]);
        assert_eq!((listings(&listed, &outputs[0]), listings(&listed, &outputs[1])), (2, 1), "only the finished item is listed again");
        assert!(!Path::new(&outputs[1]).exists(), "the item the stop kept from starting wrote a file");
    }

    /// A panic in the watch unwinds though the render it stopped never ends, and the render, told to
    /// stop, takes its file away when it next looks.
    #[test]
    fn a_panic_unwinds_though_the_render_does_not_end() {
        let released = Arc::new(AtomicBool::new(false));
        let stuck = Held::stuck(released.clone());
        let mut rig = rig_on(1, |_| Arc::new(stuck));
        let output = out(&rig, "stuck.mp4");
        let request = serde_json::json!({"output": output, "format": "h264"});
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            export(
                &mut rig.backend,
                &request,
                &mut |done, _, _| {
                    assert_eq!(done, 0, "boom in the beat");
                    true
                },
                &mut unlisted,
            )
        }));
        assert!(unwound.is_err(), "the panic did not reach the call");
        let session = rig.backend.session().expect("a session");
        assert!(session.is_rendering(), "the call waited for a render that does not end");
        released.store(true, Ordering::SeqCst);
        wait_for_end(Some(Duration::from_secs(30)), || {
            session.poll_render();
            !session.is_rendering()
        });
        assert!(!session.is_rendering(), "the render never saw its stop");
        assert!(!Path::new(&output).exists(), "the stopped render left its file");
    }

    /// A stop that panics as well, while the call unwinds from a panic in the watch, must not end the
    /// process: the first panic still reaches the caller.
    #[test]
    fn a_stop_that_panics_again_leaves_the_first_panic_to_unwind() {
        let mut session = Session::default();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            watching(&mut session, |_, _| panic!("boom in the stop"), |_| -> () { panic!("boom in the watch") })
        }));
        let said = unwound.err().and_then(|payload| payload.downcast_ref::<&str>().map(|text| (*text).to_string()));
        assert_eq!(said.as_deref(), Some("boom in the watch"));
    }

    #[test]
    fn a_failed_item_fails_the_export_in_the_command_lines_words() {
        let mut rig = rig(1);
        // The fake exporter cannot write inside a folder that is not there.
        let output = out(&rig, "missing/x.mp4");
        let error =
            export(&mut rig.backend, &serde_json::json!({"output": output, "format": "h264"}), &mut |_, _, _| true, &mut unlisted).expect_err("the item fails");
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
        let mut asked = Vec::new();
        let error = export(&mut rig.backend, &serde_json::json!({"output": output, "format": "gif"}), &mut |_, _, _| true, &mut |paths| {
            asked.extend(paths.iter().cloned());
            Ok(())
        })
        .expect_err("not offered");
        assert!(asked.is_empty(), "a render that writes nothing lists nothing: {asked:?}");
        match error {
            CraftError::Unsupported { message } => assert!(message.starts_with("A: ") && message.ends_with("export is not available"), "{message}"),
            other => panic!("an export format the build lacks is `Unsupported`, got {other:?}"),
        }
    }

    #[test]
    fn a_refused_listing_stops_the_export_before_the_exporter_is_reached() {
        let mut rig = rig(3);
        let output = out(&rig, "listed.mp4");
        let mut asked = Vec::new();
        let refused = export(&mut rig.backend, &serde_json::json!({"output": output, "format": "h264"}), &mut |_, _, _| true, &mut |paths| {
            asked.extend(paths.iter().cloned());
            Err(CraftError::io("the journal cannot be written"))
        });
        assert_eq!(refused, Err(CraftError::io("the journal cannot be written")));
        assert_eq!(rig.started.load(Ordering::SeqCst), 0, "the exporter was never reached");
        assert!(!Path::new(&output).exists());
        assert_eq!(asked, [PathBuf::from(&output), PathBuf::from(out(&rig, "listed_RenderLog.txt"))]);
    }
}
