//! CraftBridge's exporter (AD7): EffectCraft's own file exporter, run on a thread pool the phone's
//! memory can feed.
//!
//! An export renders frames in parallel and holds every frame in flight at once. EffectCraft keeps
//! that number under what the system has free, but it reads the free memory through a call with no
//! branch for iOS or Android, so on a phone it has no figure and takes the frames in flight from the
//! pool's thread count alone: a long export at a large size then outgrows the app's memory and the
//! system ends the app. The exporter here gives each job a pool of the size the figure the core reads
//! ([`available_memory`]) can hold, and EffectCraft's own rule does the rest, since it follows the
//! pool's size.
//!
//! A job the memory does not hold back runs on the threads its caller already has, exactly as
//! EffectCraft's exporter alone runs it. A smaller pool is kept from job to job rather than started
//! for each: EffectCraft keeps state on every thread it renders on (each builds an expression runtime
//! once and never frees it, as it expects its threads to live on), so a pool started for every job
//! would build that state again for each job and leave it behind when its threads end.

use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};

use craftbridge_core::available_memory;
use effectcraft_engine::project::render_queue::{OutputFormat, RenderSettings};
use effectcraft_engine::render_queue::{ExportJob, ExportResult, Exporter};

/// The bytes a pixel of a frame in flight takes: the compositors keep four `f32` channels, the number
/// EffectCraft's own rule for its frames in flight counts by.
const BYTES_PER_PIXEL: u64 = 16;

/// The number of threads an export may run on: as many as half of `available` bytes hold frames of
/// `frame` bytes (the other half is for the layer caches, the decoded footage, the encoder and the
/// figure being low), at least one and at most `cores`.
///
/// Without a figure for the memory, or for the frame, there is nothing to size by and the export keeps
/// every core: the exports of a desktop, whose figures EffectCraft reads itself, run as they did.
pub(crate) fn pool_size(available: Option<u64>, frame: Option<u64>, cores: usize) -> usize {
    let cores = cores.max(1);
    let (Some(available), Some(frame)) = (available, frame.filter(|&bytes| bytes > 0)) else { return cores };
    usize::try_from(available / 2 / frame).unwrap_or(usize::MAX).clamp(1, cores)
}

/// The bytes of the largest frame the job holds at once: the frame the compositor renders, or the frame
/// the job's output module writes after its crop and resize, whichever is larger. A queue item's other
/// output modules are not counted: EffectCraft hands each module to the exporter as a job of its own,
/// which writes only its own. `None` when the job names no composition, which the export itself refuses
/// in its own words.
fn frame_bytes(job: &ExportJob) -> Option<u64> {
    let comp = job.project.comp(job.item.comp)?;
    let pixels = |(width, height): (u32, u32)| u64::from(width).saturating_mul(u64::from(height));
    let rendered = pixels(job.item.settings.output_size(comp));
    let written = pixels(job.item.output.output_size(comp, &job.item.settings));
    Some(rendered.max(written).saturating_mul(BYTES_PER_PIXEL))
}

/// The pool an export runs on: `threads` threads, named so a debugger or a crash report shows whose they are.
fn build_pool(threads: usize) -> Result<rayon::ThreadPool, String> {
    started(rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|index| format!("craftbridge-export-{index}")).build())
}

/// What a pool's start comes to: a pool that cannot start fails the export, which would otherwise run on
/// more threads than the memory holds.
fn started(pool: Result<rayon::ThreadPool, rayon::ThreadPoolBuildError>) -> Result<rayon::ThreadPool, String> {
    pool.map_err(|error| format!("the export's threads could not start: {error}"))
}

/// The exporter a session holds: every job goes to `inner` inside a pool of the size [`pool_size`]
/// gives for the memory `memory` reports as the job starts.
pub(crate) struct PooledExporter {
    inner: Arc<dyn Exporter>,
    memory: fn() -> Option<u64>,
    /// The pool the last job the memory held back ran on, for the next job that needs one of its size.
    /// Only the last is kept, so at most one pool's threads wait beside the caller's own.
    kept: Mutex<Option<Arc<rayon::ThreadPool>>>,
}

impl PooledExporter {
    /// EffectCraft's file exporter, the one the command line exports through, on the memory the phone
    /// reports.
    pub(crate) fn new() -> Self {
        Self::over(Arc::new(effectcraft_host::FileExporter::default()), available_memory)
    }

    /// `inner` on the memory `memory` reports.
    fn over(inner: Arc<dyn Exporter>, memory: fn() -> Option<u64>) -> Self {
        Self { inner, memory, kept: Mutex::new(None) }
    }

    /// A pool of `threads` threads: the kept one when it has that many, or a new one, kept in its place.
    fn pool(&self, threads: usize) -> Result<Arc<rayon::ThreadPool>, String> {
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(pool) = kept.as_ref().filter(|pool| pool.current_num_threads() == threads) {
            return Ok(Arc::clone(pool));
        }
        let pool = Arc::new(build_pool(threads)?);
        *kept = Some(Arc::clone(&pool));
        Ok(pool)
    }
}

impl PooledExporter {
    /// `job` through `inner`, on the pool the memory allows: the one place a job's pool is chosen, so the
    /// exports CraftBridge makes of its own (plan §18) are held to the memory as EffectCraft's are.
    pub(crate) fn run(&self, inner: &dyn Exporter, job: &ExportJob, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<ExportResult, String> {
        // The pool's ceiling is the size of the pool a thread outside any pool would run on, which is
        // what EffectCraft's own rule counts threads in: the machine's cores, or the count rayon's
        // environment variables set.
        let cores = rayon::current_num_threads();
        let threads = pool_size((self.memory)(), frame_bytes(job), cores);
        if threads == cores {
            // The caller's own pool already has that many threads, so the job runs there, as
            // EffectCraft's exporter alone runs it: a desktop's exports, which report no figure, keep
            // their threads and what EffectCraft keeps on them.
            return inner.export(job, progress);
        }
        let pool = self.pool(threads)?;
        // The caller's progress hook is not `Send`, so it cannot move onto the pool. The export runs on
        // the pool and asks this thread to call the hook, waiting for the answer, which is the one
        // thread the hook has always been called on.
        let (asks, hears) = mpsc::channel::<(u64, u64, mpsc::Sender<bool>)>();
        let mut outcome = None;
        pool.in_place_scope(|scope| {
            let slot = &mut outcome;
            scope.spawn(move |_| {
                *slot = Some(inner.export(job, &mut |done, total| {
                    let (answer, reply) = mpsc::channel();
                    // A hook that cannot be reached cannot say to go on: the export stops.
                    asks.send((done, total, answer)).is_ok() && reply.recv().unwrap_or(false)
                }));
            });
            while let Ok((done, total, answer)) = hears.recv() {
                let _ = answer.send(progress(done, total));
            }
        });
        outcome.unwrap_or_else(|| Err("the export's threads ended without an answer".into()))
    }
}

impl Exporter for PooledExporter {
    fn formats(&self) -> Vec<OutputFormat> {
        self.inner.formats()
    }

    fn supports_skip_existing(&self, settings: &RenderSettings) -> bool {
        self.inner.supports_skip_existing(settings)
    }

    fn export(&self, job: &ExportJob, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<ExportResult, String> {
        self.run(&*self.inner, job, progress)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::thread::ThreadId;

    use effectcraft_automation::Backend;
    use effectcraft_engine::Session;
    use effectcraft_engine::project::render_queue::{OutputModule, RenderQueueItem};
    use effectcraft_engine::project::{ItemId, Project};
    use effectcraft_engine::render_queue::CANCELLED;
    use effectcraft_media::MediaPool;

    const MIB: u64 = 1 << 20;
    const GIB: u64 = 1 << 30;

    /// A 1920 by 1080 frame at 16 bytes a pixel.
    const FULL_HD: u64 = 1920 * 1080 * BYTES_PER_PIXEL;

    /// Memory whose half holds 256 1080p frames: more than any machine the tests run on has cores, and
    /// few enough that an export test with a broken ceiling starts a pool the machine can bear, where
    /// the largest figure would ask for billions of threads.
    const PLENTY: u64 = 2 * 256 * FULL_HD;

    #[test]
    fn the_pool_holds_as_many_frames_as_half_the_memory() {
        // Half of 4 GiB is 2 GiB, which holds 64 frames of 32 MiB, and 4 of 512 MiB.
        assert_eq!(pool_size(Some(4 * GIB), Some(32 * MIB), 128), 64);
        assert_eq!(pool_size(Some(4 * GIB), Some(512 * MIB), 128), 4);
        // A 1080p frame is 31.6 MiB: 6 GiB free holds 97 of them in its half.
        assert_eq!(pool_size(Some(6 * GIB), Some(FULL_HD), 128), 97);
    }

    #[test]
    fn the_pool_rounds_down_at_the_edge_of_a_frame() {
        assert_eq!(pool_size(Some(2 * 3 * MIB), Some(MIB), 16), 3);
        assert_eq!(pool_size(Some(2 * 3 * MIB - 1), Some(MIB), 16), 2);
        assert_eq!(pool_size(Some(2 * 3 * MIB + 2 * MIB - 1), Some(MIB), 16), 3);
    }

    #[test]
    fn the_pool_never_has_fewer_than_one_thread() {
        assert_eq!(pool_size(Some(0), Some(MIB), 8), 1);
        assert_eq!(pool_size(Some(MIB), Some(GIB), 8), 1);
        assert_eq!(pool_size(Some(2 * GIB - 1), Some(GIB), 8), 1);
        assert_eq!(pool_size(Some(GIB), Some(1), 0), 1, "a count of no cores is one");
        assert_eq!(pool_size(None, None, 0), 1);
    }

    #[test]
    fn the_pool_never_has_more_threads_than_cores() {
        assert_eq!(pool_size(Some(64 * GIB), Some(MIB), 6), 6);
        assert_eq!(pool_size(Some(u64::MAX), Some(1), 6), 6);
        assert_eq!(pool_size(Some(2 * 6 * MIB), Some(MIB), 6), 6, "exactly as many frames as cores");
        assert_eq!(pool_size(Some(2 * 7 * MIB), Some(MIB), 6), 6);
    }

    #[test]
    fn without_a_figure_the_pool_has_every_core() {
        assert_eq!(pool_size(None, Some(FULL_HD), 8), 8, "a memory figure that is missing");
        assert_eq!(pool_size(Some(GIB), None, 8), 8, "a frame that is not known");
        assert_eq!(pool_size(Some(GIB), Some(0), 8), 8, "a frame of no bytes");
        assert_eq!(pool_size(None, None, 8), 8);
    }

    /// The pieces of an export job: a project holding one composition `width` by `height`, and a queue
    /// item that renders it, for the test to change before it builds the job.
    pub(crate) fn project_with_comp(width: u32, height: u32) -> (Arc<Project>, RenderQueueItem) {
        let mut backend = Backend::headless(Session::default());
        let params = serde_json::json!({"name": "A", "width": width, "height": height, "duration": 1, "frameRate": 10});
        backend.exec("comp.new", params).expect("the composition is made");
        let project = backend.session().expect("a session").project.clone();
        let comp = project.items.keys().copied().next().expect("the composition's id");
        (project, RenderQueueItem::new(1, comp))
    }

    /// `body` with a job for `item` over `project`, which writes to `path`.
    pub(crate) fn with_job<R>(project: &Project, item: &RenderQueueItem, body: impl FnOnce(&ExportJob) -> R) -> R {
        let footage = MediaPool::new();
        let job = ExportJob {
            project,
            footage: &footage,
            expr: None,
            item,
            accel: None,
            path: "out.mp4",
            storage: None,
            label: "#1 A".into(),
            nested_switches: false,
            caches: None,
            renderer: None,
            gpu_frames: None,
        };
        body(&job)
    }

    fn bytes_of(project: &Project, item: &RenderQueueItem) -> Option<u64> {
        with_job(project, item, frame_bytes)
    }

    #[test]
    fn a_frame_is_the_size_the_comp_renders_at_sixteen_bytes_a_pixel() {
        let (project, item) = project_with_comp(1920, 1080);
        assert_eq!(bytes_of(&project, &item), Some(FULL_HD));
        let (project, mut item) = project_with_comp(1920, 1080);
        item.settings.resolution = 0.5;
        assert_eq!(bytes_of(&project, &item), Some(960 * 540 * 16), "half the render scale is a quarter of the pixels");
    }

    #[test]
    fn a_frame_is_the_larger_of_what_is_rendered_and_what_is_written() {
        let (project, mut item) = project_with_comp(640, 360);
        item.output.resize.enabled = true;
        item.output.resize.lock_aspect = false;
        item.output.resize.width = 3840;
        item.output.resize.height = 2160;
        assert_eq!(bytes_of(&project, &item), Some(3840 * 2160 * 16), "an enlarging resize holds the larger frame");
        let (project, mut item) = project_with_comp(3840, 2160);
        item.output.resize.enabled = true;
        item.output.resize.lock_aspect = false;
        item.output.resize.width = 640;
        item.output.resize.height = 360;
        assert_eq!(bytes_of(&project, &item), Some(3840 * 2160 * 16), "a shrinking resize still renders the full frame");
    }

    #[test]
    fn another_output_module_is_another_jobs_frame() {
        let (project, mut item) = project_with_comp(640, 360);
        let mut larger = OutputModule::default();
        larger.resize.enabled = true;
        larger.resize.lock_aspect = false;
        larger.resize.width = 1280;
        larger.resize.height = 720;
        item.extra_outputs.push(larger.clone());
        assert_eq!(bytes_of(&project, &item), Some(640 * 360 * 16), "the main module's job writes only its own frames");
        // EffectCraft gives the other module a job of its own: the item, with that module as its output.
        item.output = larger;
        assert_eq!(bytes_of(&project, &item), Some(1280 * 720 * 16));
    }

    #[test]
    fn a_job_without_its_comp_has_no_frame_size() {
        let (project, _) = project_with_comp(16, 16);
        let missing = RenderQueueItem::new(1, ItemId(9999));
        assert_eq!(bytes_of(&project, &missing), None);
    }

    /// An exporter that records, for each export, the size of the pool it ran on and the thread it ran
    /// on, and reports `frames` frames of progress.
    struct Recording {
        runs: Mutex<Vec<(usize, ThreadId)>>,
        frames: u64,
    }

    impl Recording {
        /// The pool size and thread of the last export.
        fn last(&self) -> (usize, ThreadId) {
            *self.runs.lock().unwrap().last().expect("an export ran")
        }
    }

    impl Exporter for Recording {
        fn formats(&self) -> Vec<OutputFormat> {
            vec![OutputFormat::PngSequence]
        }
        fn supports_skip_existing(&self, _: &RenderSettings) -> bool {
            true
        }
        fn export(&self, job: &ExportJob, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<ExportResult, String> {
            self.runs.lock().unwrap().push((rayon::current_num_threads(), std::thread::current().id()));
            for done in 0..self.frames {
                if !progress(done, self.frames) {
                    return Err(CANCELLED.into());
                }
            }
            Ok(ExportResult { path: job.path.to_string(), frames: self.frames, ..Default::default() })
        }
    }

    fn recording(frames: u64) -> Arc<Recording> {
        Arc::new(Recording { runs: Mutex::new(Vec::new()), frames })
    }

    /// The pool size and thread a `width` by `height` export ran on through `exporter`, whose inner
    /// exporter is `inner`.
    fn run_on(exporter: &PooledExporter, inner: &Recording, width: u32, height: u32) -> (usize, ThreadId) {
        let (project, item) = project_with_comp(width, height);
        with_job(&project, &item, |job| exporter.export(job, &mut |_, _| true)).expect("the export runs");
        inner.last()
    }

    /// The pool a 1080p export ran on, given the memory the phone reports (a function, as the exporter
    /// reads it, so each figure is its own).
    fn threads_for(memory: fn() -> Option<u64>) -> usize {
        let inner = recording(1);
        run_on(&PooledExporter::over(inner.clone(), memory), &inner, 1920, 1080).0
    }

    #[test]
    fn the_export_runs_on_a_pool_that_follows_the_reported_memory() {
        let cores = rayon::current_num_threads();
        // A 1080p frame is 31.6 MiB, so 2 GiB holds 32 frames in its half, and 100 MiB holds 1.
        assert_eq!(threads_for(|| Some(100 * MIB)), 1);
        assert_eq!(threads_for(|| Some(0)), 1);
        assert_eq!(threads_for(|| Some(2 * 3 * FULL_HD)), 3.min(cores));
        assert_eq!(threads_for(|| Some(2 * 3 * FULL_HD - 1)), 2.min(cores));
        // A pool between one thread and the cores is its own size: a ceiling below the cores would show.
        assert_eq!(threads_for(|| Some(2 * 10 * FULL_HD)), 10.min(cores));
        assert_eq!(threads_for(|| Some(PLENTY)), cores);
        assert_eq!(threads_for(|| None), cores, "a phone that reports nothing, and a desktop, keep every core");
    }

    #[test]
    fn a_job_the_memory_does_not_hold_back_runs_on_its_callers_own_threads() {
        let caller = std::thread::current().id();
        for memory in [(|| None) as fn() -> Option<u64>, || Some(PLENTY)] {
            let inner = recording(1);
            assert_eq!(run_on(&PooledExporter::over(inner.clone(), memory), &inner, 1920, 1080), (rayon::current_num_threads(), caller));
        }
        let inner = recording(1);
        assert_ne!(run_on(&PooledExporter::over(inner.clone(), || Some(0)), &inner, 1920, 1080).1, caller, "a job held back runs on its pool");
    }

    #[test]
    fn jobs_of_one_size_share_one_pool_and_another_size_replaces_it() {
        // Half of this holds two 1080p frames and no 4K frame, so a 4K job gets the floor of one thread;
        // the machines the tests run on have more than two cores, so both jobs are held back.
        let inner = recording(1);
        let exporter = PooledExporter::over(inner.clone(), || Some(2 * 2 * FULL_HD));
        let (one, first) = run_on(&exporter, &inner, 3840, 2160);
        assert_eq!((one, run_on(&exporter, &inner, 3840, 2160)), (1, (1, first)), "the second 4K job runs on the first one's thread");
        assert_eq!(run_on(&exporter, &inner, 1920, 1080).0, 2, "a 1080p job needs a pool of two");
        let (one, again) = run_on(&exporter, &inner, 3840, 2160);
        assert_eq!(one, 1);
        assert_ne!(again, first, "the pool of one gave way to the pool of two");
    }

    /// The pool follows the figure this platform really gives, for frames from 1080p to 8K. On a phone's
    /// simulator or emulator this is the run that shows it; each figure and size goes to the run's output
    /// for the person reading it. A phone's figure moves while the test runs, so the pool is held to the
    /// sizes the figures just before and just after its export give.
    #[test]
    fn the_pool_follows_the_figure_the_platform_gives() {
        let cores = rayon::current_num_threads();
        let inner = recording(1);
        let exporter = PooledExporter::over(inner.clone(), available_memory);
        for (width, height) in [(1920, 1080), (3840, 2160), (5120, 2880), (7680, 4320)] {
            let frame = Some(u64::from(width) * u64::from(height) * BYTES_PER_PIXEL);
            let before = available_memory();
            let (threads, _) = run_on(&exporter, &inner, width, height);
            let after = available_memory();
            eprintln!("available_memory() = {before:?} then {after:?}, cores = {cores}, a {width}x{height} export's pool = {threads}");
            let (one, other) = (pool_size(before, frame, cores), pool_size(after, frame, cores));
            assert!((one.min(other)..=one.max(other)).contains(&threads), "{threads} threads, against {one} and {other}");
        }
    }

    #[test]
    fn the_hook_is_called_from_the_callers_thread_and_can_stop_the_export() {
        let (project, item) = project_with_comp(16, 16);
        let caller = std::thread::current().id();
        // A job held back runs on the pool and a job that is not runs on the caller's own threads; the
        // hook is the caller's either way.
        for (memory, on_the_pool) in [(|| Some(0)) as fn() -> Option<u64>, || None].into_iter().zip([true, false]) {
            let inner = recording(5);
            let exporter = PooledExporter::over(inner.clone(), memory);
            let mut heard = Vec::new();
            let result = with_job(&project, &item, |job| {
                exporter.export(job, &mut |done, total| {
                    heard.push((done, total, std::thread::current().id() == caller));
                    done < 2
                })
            });
            assert_eq!(result.expect_err("the third answer stops it"), CANCELLED);
            assert_eq!(heard, [(0, 5, true), (1, 5, true), (2, 5, true)]);
            assert_eq!(inner.last().1 != caller, on_the_pool, "the export ran on the pool only when the memory held it back");
        }
    }

    /// An exporter that goes on reporting until the hook says to stop, counting the reports it made.
    #[derive(Default)]
    struct Counting {
        reports: std::sync::atomic::AtomicU64,
    }

    impl Exporter for Counting {
        fn formats(&self) -> Vec<OutputFormat> {
            Vec::new()
        }
        fn export(&self, job: &ExportJob, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<ExportResult, String> {
            for done in 0..1000 {
                self.reports.fetch_add(1, Ordering::SeqCst);
                if !progress(done, 1000) {
                    return Err(CANCELLED.into());
                }
            }
            Ok(ExportResult { path: job.path.to_string(), ..Default::default() })
        }
    }

    #[test]
    fn a_hook_that_fails_stops_the_export_on_the_pool_instead_of_letting_it_run_on() {
        let (project, item) = project_with_comp(16, 16);
        let inner = Arc::new(Counting::default());
        let exporter = PooledExporter::over(inner.clone(), || Some(0));
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_job(&project, &item, |job| exporter.export(job, &mut |_, _| panic!("the hook fails")))
        }));
        assert!(caught.is_err(), "the hook's panic reaches the caller");
        assert_eq!(inner.reports.load(Ordering::SeqCst), 1, "the export stopped at the first report the hook could not answer");
    }

    /// An exporter whose first export stops the thread it runs on by panicking, saying which thread that
    /// was, and whose later exports write nothing and succeed.
    #[derive(Default)]
    struct PanicsOnce {
        panicked: std::sync::atomic::AtomicBool,
    }

    impl Exporter for PanicsOnce {
        fn formats(&self) -> Vec<OutputFormat> {
            Vec::new()
        }
        fn export(&self, job: &ExportJob, _: &mut dyn FnMut(u64, u64) -> bool) -> Result<ExportResult, String> {
            if !self.panicked.swap(true, Ordering::SeqCst) {
                std::panic::panic_any(std::thread::current().name().unwrap_or_default().to_string())
            }
            Ok(ExportResult { path: job.path.to_string(), ..Default::default() })
        }
    }

    #[test]
    fn a_panic_in_the_export_reaches_the_caller_on_its_own_thread_and_the_next_export_runs() {
        let (project, item) = project_with_comp(16, 16);
        let exporter = PooledExporter::over(Arc::new(PanicsOnce::default()), || Some(0));
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| with_job(&project, &item, |job| exporter.export(job, &mut |_, _| true))));
        let payload = caught.expect_err("the panic comes back to the caller, the process is not ended");
        assert_eq!(payload.downcast_ref::<String>().map(String::as_str), Some("craftbridge-export-0"), "it was raised on the pool's thread");
        with_job(&project, &item, |job| exporter.export(job, &mut |_, _| true)).expect("an export after the failed one runs, on the pool it kept");
    }

    #[test]
    fn a_pool_that_cannot_start_fails_the_export_in_words() {
        let failing = rayon::ThreadPoolBuilder::new().num_threads(2).spawn_handler(|_| Err(std::io::Error::other("no thread for you")));
        let error = started(failing.build()).expect_err("no thread can start");
        assert_eq!(error, "the export's threads could not start: no thread for you");
    }

    #[test]
    fn the_result_and_the_inner_exporters_answers_come_back_whole() {
        let (project, item) = project_with_comp(16, 16);
        for memory in [(|| None) as fn() -> Option<u64>, || Some(0)] {
            let exporter = PooledExporter::over(recording(3), memory);
            let result = with_job(&project, &item, |job| exporter.export(job, &mut |_, _| true)).expect("it exports");
            assert_eq!((result.path.as_str(), result.frames), ("out.mp4", 3));
            assert_eq!(exporter.formats(), [OutputFormat::PngSequence]);
            assert!(exporter.supports_skip_existing(&RenderSettings::default()));
        }
    }

    #[test]
    fn the_session_exports_in_the_formats_of_effectcrafts_exporter() {
        assert_eq!(PooledExporter::new().formats(), effectcraft_host::FileExporter::default().formats());
    }
}
