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
//! A frame the memory cannot hold at all is different from a frame it merely holds fewer of at once:
//! the floor of one thread renders a frame larger than the phone's whole memory, and the system ends
//! the app mid-render. So before a job renders, the export compares what its frame needs — the copies
//! its compositor and its output hold of it, [`frame_need`], the same need the pool's size is built
//! from — with what the platform reports, and refuses a frame that cannot fit in words naming the
//! frame and the memory, before a pixel is written. A platform whose figure cannot be read fails the
//! export with its reason: a guess in its place could let the system end the app.
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

/// The bytes a pixel of a frame takes in the compositors: four `f32` channels, the number EffectCraft's
/// own rule for its frames in flight counts by.
const BYTES_PER_PIXEL: u64 = 16;

/// The compositor a job's frames render on, which decides how many copies of a frame are held at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Compositor {
    Cpu,
    Gpu,
}

/// The copies of a frame in flight that are alive at once on the CPU (`Compositor::copies` says why
/// four). The device checks read it, times the frame's bytes a pixel, from the refusal's own words, so no
/// other language spells it.
const CPU_COPIES: u64 = 4;

/// The copies of a frame in flight that are alive at once on the GPU.
const GPU_COPIES: u64 = 10;

impl Compositor {
    /// The compositor EffectCraft's exporter renders a job on: the GPU's when the project asks for GPU
    /// acceleration and the job has an accelerator, the CPU's otherwise. A job on the GPU may still
    /// render some frames on the CPU, as its compositor is measured; it is counted at the GPU's need,
    /// the larger.
    fn of(gpu_acceleration: bool, accelerated: bool) -> Self {
        if gpu_acceleration && accelerated { Self::Gpu } else { Self::Cpu }
    }

    /// How many times a frame's bytes one frame in flight holds, at the pin, measured as the most memory
    /// a one-frame export of a full-frame layer held above what the process held before it, for every
    /// format EffectCraft writes that was measured. On the CPU: the canvas the comp is drawn on, the
    /// frame of the layer drawn onto it, and what the output makes of the finished frame — its 8-bit
    /// pixels and the writer's or encoder's own buffers — from 2.2 frames (OpenEXR) through 2.4 (PNG,
    /// JPEG), 2.5 (TIFF) and 2.8 (Targa, H.264) to 3.3 (ProRes). On the GPU, its working textures and the
    /// frame read back from them besides: 6.5 frames (PNG at 5K and 8K) to 9.3 (ProRes at 1080p and 4K),
    /// on a Mac's Metal device, which shares the app's memory as a phone's GPU does. Each count is the
    /// whole number above the most measured; a comp with more effects can need more, so the count is a
    /// floor the refusal holds a frame to, never a promise that a frame above it fits.
    fn copies(self) -> u64 {
        match self {
            Self::Cpu => CPU_COPIES,
            Self::Gpu => GPU_COPIES,
        }
    }

    /// The compositor, as the refusal names it.
    fn words(self) -> &'static str {
        match self {
            Self::Cpu => "the CPU",
            Self::Gpu => "the GPU",
        }
    }
}

/// One frame the export holds: the size it is at, its bytes at the compositor's bytes a pixel, and the
/// compositor that renders it.
struct Frame {
    width: u32,
    height: u32,
    bytes: u64,
    on: Compositor,
}

/// What one frame in flight needs: its bytes as many times as its compositor and output hold them. The
/// pool's size divides the memory by this, and [`refusal`] compares it with the memory as it is: the one
/// definition of a frame's need, so a job the pool can hold a frame of is never refused, and a frame no
/// pool can hold is.
fn frame_need(frame: &Frame) -> u64 {
    frame.bytes.saturating_mul(frame.on.copies())
}

/// The number of threads an export may run on: as many as `available` bytes hold frames in flight of
/// `need` bytes each ([`frame_need`]), at least one and at most `cores`. Every thread may hold a frame in
/// flight; on the GPU fewer render at once, so the count errs towards fewer threads, the side that keeps
/// the app alive.
pub(crate) fn pool_size(available: u64, need: u64, cores: usize) -> usize {
    let cores = cores.max(1);
    // A need of no bytes, which no frame has, holds nothing back.
    available.checked_div(need).map_or(cores, |frames| usize::try_from(frames).unwrap_or(usize::MAX).clamp(1, cores))
}

/// The refusal of a job whose frames do not fit: `None` when `threads` frames in flight of `frame`, one
/// per thread of its pool, fit in `available` bytes at [`frame_need`] each, and the words that say why
/// it does not when they do not, naming the frame's size, its compositor and the memory the platform
/// reports. The number a platform reports is never more than `u64::MAX`, so a frame whose need passes
/// that number, from a size too large to count, fits nowhere and is refused everywhere.
fn refusal(available: u64, frame: &Frame, threads: usize) -> Option<String> {
    let need = frame_need(frame);
    (need.saturating_mul(u64::try_from(threads).unwrap_or(u64::MAX)) > available).then(|| {
        format!(
            "a frame of {}x{} needs {need} bytes to render on {} and write, and the device reports {available} bytes available",
            frame.width,
            frame.height,
            frame.on.words()
        )
    })
}

/// The failure of a job whose frame cannot be held to the memory, because the platform's figure could
/// not be read: `why` is the platform's reason.
fn unread(frame: &Frame, why: &str) -> String {
    format!("a frame of {}x{} cannot be checked against the device's memory: {why}", frame.width, frame.height)
}

/// The largest frame the job holds at once: the frame the compositor renders, or the frame the job's
/// output module writes after its crop and resize, whichever is larger. A queue item's other output
/// modules are not counted: EffectCraft hands each module to the exporter as a job of its own, which
/// writes only its own. `None` when the job renders no frame: one that names no composition, which the
/// export itself refuses in its own words, and one whose output is sound alone, which EffectCraft mixes
/// without rendering a picture.
fn frame_of(job: &ExportJob) -> Option<Frame> {
    if job.item.output.format.is_audio_only() {
        return None;
    }
    let comp = job.project.comp(job.item.comp)?;
    let pixels = |(width, height): (u32, u32)| u64::from(width).saturating_mul(u64::from(height));
    let rendered = job.item.settings.output_size(comp);
    let written = job.item.output.output_size(comp, &job.item.settings);
    let (width, height) = if pixels(written) > pixels(rendered) { written } else { rendered };
    let on = Compositor::of(job.project.settings.gpu_acceleration, job.accel.is_some());
    Some(Frame { width, height, bytes: pixels((width, height)).saturating_mul(BYTES_PER_PIXEL), on })
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
/// gives for the memory `memory` reports as the job starts, or is refused before it starts.
pub(crate) struct PooledExporter {
    inner: Arc<dyn Exporter>,
    memory: fn() -> Result<u64, String>,
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
    pub(crate) fn over(inner: Arc<dyn Exporter>, memory: fn() -> Result<u64, String>) -> Self {
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
    /// The threads `job` may run on, or why it may not run at all, decided before anything of it renders
    /// or is written: the frame must fit the memory the platform reports before a pixel of it is painted,
    /// since a floor of one thread would otherwise render a frame larger than the phone's whole memory,
    /// and the system would end the app mid-render — losing the user's work in the app, not just the
    /// export. A job that renders no frame has nothing to hold to the memory and keeps every core; for one
    /// that does, a figure the platform cannot give fails the job with the platform's reason.
    pub(crate) fn admit(&self, job: &ExportJob) -> Result<usize, String> {
        // The pool's ceiling is the size of the pool a thread outside any pool would run on, which is
        // what EffectCraft's own rule counts threads in: the machine's cores, or the count rayon's
        // environment variables set.
        let cores = rayon::current_num_threads();
        let Some(frame) = frame_of(job) else { return Ok(cores) };
        let available = (self.memory)().map_err(|why| unread(&frame, &why))?;
        let threads = pool_size(available, frame_need(&frame), cores);
        refusal(available, &frame, threads).map_or(Ok(threads), Err)
    }

    /// `job` through `inner`, on the pool the memory allows: the one place a job's pool is chosen, so the
    /// exports CraftBridge makes of its own (plan §18) are held to the memory as EffectCraft's are.
    pub(crate) fn run(&self, inner: &dyn Exporter, job: &ExportJob, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<ExportResult, String> {
        let threads = self.admit(job)?;
        if threads == rayon::current_num_threads() {
            // The caller's own pool already has that many threads, so the job runs there, as
            // EffectCraft's exporter alone runs it: a job the memory does not hold back keeps its
            // threads and what EffectCraft keeps on them.
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

    /// What a 1080p frame in flight needs on the CPU: four copies of its bytes.
    const FULL_HD_NEED: u64 = 4 * FULL_HD;

    /// What a 16 by 16 frame in flight needs on the CPU: four copies of its 4096 bytes.
    const SMALLEST_NEED: u64 = 4 * 16 * 16 * BYTES_PER_PIXEL;

    /// Memory that holds 256 1080p frames in flight: more than any machine the tests run on has cores,
    /// and few enough that an export test with a broken ceiling starts a pool the machine can bear, where
    /// the largest figure would ask for billions of threads.
    const PLENTY: u64 = 256 * FULL_HD_NEED;

    #[test]
    fn the_pool_holds_as_many_frames_in_flight_as_the_memory_holds_needs() {
        assert_eq!(pool_size(4 * GIB, 32 * MIB, 256), 128);
        assert_eq!(pool_size(4 * GIB, 512 * MIB, 128), 8);
        // A 1080p frame in flight needs 126.6 MiB on the CPU: 6 GiB holds 48 of them.
        assert_eq!(pool_size(6 * GIB, FULL_HD_NEED, 128), 48);
    }

    #[test]
    fn the_pool_rounds_down_at_the_edge_of_a_frame() {
        assert_eq!(pool_size(3 * MIB, MIB, 16), 3);
        assert_eq!(pool_size(3 * MIB - 1, MIB, 16), 2);
        assert_eq!(pool_size(4 * MIB - 1, MIB, 16), 3);
    }

    #[test]
    fn the_pool_never_has_fewer_than_one_thread() {
        assert_eq!(pool_size(0, MIB, 8), 1);
        assert_eq!(pool_size(MIB, GIB, 8), 1);
        assert_eq!(pool_size(GIB - 1, GIB, 8), 1);
        assert_eq!(pool_size(GIB, 1, 0), 1, "a count of no cores is one");
        assert_eq!(pool_size(GIB, u64::MAX, 8), 1, "a frame whose need passes every number is the floor of one");
        assert_eq!(pool_size(0, 0, 0), 1);
    }

    #[test]
    fn the_pool_never_has_more_threads_than_cores() {
        assert_eq!(pool_size(64 * GIB, MIB, 6), 6);
        assert_eq!(pool_size(u64::MAX, 1, 6), 6);
        assert_eq!(pool_size(6 * MIB, MIB, 6), 6, "exactly as many frames as cores");
        assert_eq!(pool_size(7 * MIB, MIB, 6), 6);
        assert_eq!(pool_size(GIB, 0, 8), 8, "a need of no bytes holds nothing back");
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

    /// `body` with a job for `item` over `project`, which writes to `out.mp4` and has no accelerator.
    pub(crate) fn with_job<R>(project: &Project, item: &RenderQueueItem, body: impl FnOnce(&ExportJob) -> R) -> R {
        with_job_at(project, item, None, "out.mp4", body)
    }

    /// `body` with a job for `item` over `project`, which writes to `path` and renders with `accel`.
    pub(crate) fn with_job_at<R>(
        project: &Project,
        item: &RenderQueueItem,
        accel: Option<&dyn effectcraft_render::Accelerator>,
        path: &str,
        body: impl FnOnce(&ExportJob) -> R,
    ) -> R {
        let footage = MediaPool::new();
        let job = ExportJob {
            project,
            footage: &footage,
            expr: None,
            item,
            accel,
            path,
            storage: None,
            label: "#1 A".into(),
            nested_switches: false,
            caches: None,
            renderer: None,
            gpu_frames: None,
        };
        body(&job)
    }

    fn frame_in(project: &Project, item: &RenderQueueItem) -> Option<(u32, u32, u64, Compositor)> {
        with_job(project, item, |job| frame_of(job).map(|frame| (frame.width, frame.height, frame.bytes, frame.on)))
    }

    fn bytes_of(project: &Project, item: &RenderQueueItem) -> Option<u64> {
        frame_in(project, item).map(|(_, _, bytes, _)| bytes)
    }

    #[test]
    fn a_frame_is_the_size_the_comp_renders_at_sixteen_bytes_a_pixel() {
        let (project, item) = project_with_comp(1920, 1080);
        assert_eq!(frame_in(&project, &item), Some((1920, 1080, FULL_HD, Compositor::Cpu)), "the frame is the size of the comp");
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
        assert_eq!(frame_in(&project, &item), Some((3840, 2160, 3840 * 2160 * 16, Compositor::Cpu)), "an enlarging resize holds the larger frame");
        let (project, mut item) = project_with_comp(3840, 2160);
        item.output.resize.enabled = true;
        item.output.resize.lock_aspect = false;
        item.output.resize.width = 640;
        item.output.resize.height = 360;
        assert_eq!(frame_in(&project, &item), Some((3840, 2160, 3840 * 2160 * 16, Compositor::Cpu)), "a shrinking resize still renders the full frame");
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
    fn a_job_without_its_comp_has_no_frame() {
        let (project, _) = project_with_comp(16, 16);
        let missing = RenderQueueItem::new(1, ItemId(9999));
        assert_eq!(bytes_of(&project, &missing), None);
    }

    #[test]
    fn a_job_that_writes_sound_alone_renders_no_frame() {
        for format in OutputFormat::ALL {
            let (project, mut item) = project_with_comp(7680, 4320);
            item.output.format = format;
            let sound_alone = matches!(format, OutputFormat::Wav | OutputFormat::Aiff);
            assert_eq!(bytes_of(&project, &item).is_none(), sound_alone, "{format:?}");
        }
    }

    #[test]
    fn a_frame_renders_on_the_gpu_only_where_the_project_asks_for_it_and_the_job_has_an_accelerator() {
        assert_eq!(Compositor::of(true, true), Compositor::Gpu);
        assert_eq!(Compositor::of(true, false), Compositor::Cpu, "no accelerator, nothing to render on but the CPU");
        assert_eq!(Compositor::of(false, true), Compositor::Cpu, "a Software Only project");
        assert_eq!(Compositor::of(false, false), Compositor::Cpu);
        // The job's own accelerator, where the machine has an adapter to make one on.
        let Ok(gpu) = effectcraft_gpu::Gpu::try_headless() else {
            eprintln!("no GPU adapter here: the job with an accelerator is not built");
            return;
        };
        let (project, item) = project_with_comp(1920, 1080);
        assert!(project.settings.gpu_acceleration, "a new project asks for GPU acceleration");
        assert_eq!(with_job_at(&project, &item, Some(&gpu), "out.mp4", |job| frame_of(job).map(|frame| frame.on)), Some(Compositor::Gpu));
    }

    #[test]
    fn a_frame_in_flight_needs_its_bytes_as_often_as_its_compositor_holds_them() {
        let frame = |on| Frame { width: 1920, height: 1080, bytes: FULL_HD, on };
        assert_eq!(frame_need(&frame(Compositor::Cpu)), 4 * FULL_HD);
        assert_eq!(frame_need(&frame(Compositor::Gpu)), 10 * FULL_HD);
        assert_eq!(frame_need(&Frame { width: u32::MAX, height: u32::MAX, bytes: u64::MAX, on: Compositor::Cpu }), u64::MAX, "a need too large to count");
    }

    #[test]
    fn an_8k_export_on_a_phone_that_cannot_hold_the_frame_is_refused_in_words_before_anything_is_written() {
        // A two-gigabyte emulator with 450 MiB free, as the low-memory run leaves it: one 8K frame in
        // flight on the CPU needs four times its 531 MB, which the phone does not have to give.
        let inner = recording(1);
        let exporter = PooledExporter::over(inner.clone(), || Ok(450 * MIB));
        let (project, item) = project_with_comp(7680, 4320);
        let refused = with_job(&project, &item, |job| exporter.export(job, &mut |_, _| true)).expect_err("the frame cannot fit");
        assert_eq!(
            refused, "a frame of 7680x4320 needs 2123366400 bytes to render on the CPU and write, and the device reports 471859200 bytes available",
            "the refusal names the frame's size, its compositor and the memory the platform gave"
        );
        assert!(inner.runs.lock().unwrap().is_empty(), "a refused export reached nothing that could write");
    }

    #[test]
    fn the_refusal_is_pinned_to_its_exact_words_at_the_smallest_size_on_either_compositor() {
        // A 16x16 frame is 4096 bytes; on the CPU it needs 16384, one more than this memory has.
        let (project, item) = project_with_comp(16, 16);
        let exporter = PooledExporter::over(recording(1), || Ok(SMALLEST_NEED - 1));
        let refused = with_job(&project, &item, |job| exporter.export(job, &mut |_, _| true)).expect_err("one byte short of the frame's need");
        assert_eq!(refused, "a frame of 16x16 needs 16384 bytes to render on the CPU and write, and the device reports 16383 bytes available");
        let on_the_gpu = Frame { width: 16, height: 16, bytes: 4096, on: Compositor::Gpu };
        assert_eq!(
            refusal(40_959, &on_the_gpu, 1).as_deref(),
            Some("a frame of 16x16 needs 40960 bytes to render on the GPU and write, and the device reports 40959 bytes available")
        );
        assert_eq!(refusal(40_960, &on_the_gpu, 1), None);
    }

    #[test]
    fn a_figure_the_platform_cannot_give_fails_the_export_with_its_reason_before_anything_is_written() {
        let inner = recording(1);
        let exporter = PooledExporter::over(inner.clone(), || Err("/proc/meminfo could not be read: permission denied".to_string()));
        let (project, item) = project_with_comp(1920, 1080);
        let failed = with_job(&project, &item, |job| exporter.export(job, &mut |_, _| true)).expect_err("no figure, no export");
        assert_eq!(failed, "a frame of 1920x1080 cannot be checked against the device's memory: /proc/meminfo could not be read: permission denied");
        assert!(inner.runs.lock().unwrap().is_empty(), "an export with no figure reached nothing that could write");
    }

    #[test]
    fn what_fits_is_not_refused_and_a_job_with_no_frame_asks_for_no_figure() {
        // Memory exactly one 8K frame's need holds the export, on a pool of one thread.
        let inner = recording(1);
        let exporter = PooledExporter::over(inner.clone(), || Ok(4 * 7680 * 4320 * BYTES_PER_PIXEL));
        assert_eq!(run_on(&exporter, &inner, 7680, 4320).0, 1, "memory exactly a frame's need runs it");
        // A job whose comp is gone is the export's to answer in its own words, and a job of sound alone
        // renders no picture: neither has a frame to hold to the memory, so neither asks for a figure,
        // and both run on the caller's own threads.
        let caller = std::thread::current().id();
        let inner = recording(1);
        let unreadable = PooledExporter::over(inner.clone(), || Err("no figure".to_string()));
        let (project, mut sound) = project_with_comp(7680, 4320);
        sound.output.format = OutputFormat::Wav;
        with_job(&project, &sound, |job| unreadable.export(job, &mut |_, _| true)).expect("a job of sound alone is not held to the memory");
        assert_eq!(inner.last(), (rayon::current_num_threads(), caller));
        let missing = RenderQueueItem::new(1, ItemId(9999));
        with_job(&project, &missing, |job| unreadable.export(job, &mut |_, _| true)).expect("a job with no frame is not refused by the check");
    }

    #[test]
    fn a_frame_too_large_to_count_fits_no_figure_and_each_thread_holds_a_frame() {
        let uncountable = Frame { width: u32::MAX, height: u32::MAX, bytes: u64::MAX, on: Compositor::Cpu };
        for available in [0, 1024, u64::MAX - 1] {
            assert_eq!(
                refusal(available, &uncountable, 1).as_deref(),
                Some(
                    format!(
                        "a frame of {}x{} needs {} bytes to render on the CPU and write, and the device reports {available} bytes available",
                        u32::MAX,
                        u32::MAX,
                        u64::MAX
                    )
                    .as_str()
                ),
                "a need past the largest figure fits no memory, even the largest"
            );
        }
        let small = Frame { width: 4, height: 4, bytes: 256, on: Compositor::Cpu };
        assert_eq!(refusal(1024, &small, 1), None, "memory exactly one need holds one frame in flight");
        assert!(refusal(1024, &small, 2).is_some(), "two frames in flight need two needs");
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
    fn threads_for(memory: fn() -> Result<u64, String>) -> usize {
        let inner = recording(1);
        run_on(&PooledExporter::over(inner.clone(), memory), &inner, 1920, 1080).0
    }

    #[test]
    fn the_export_runs_on_a_pool_that_follows_the_reported_memory() {
        let cores = rayon::current_num_threads();
        assert_eq!(threads_for(|| Ok(FULL_HD_NEED)), 1, "memory exactly one frame's need holds one frame in flight");
        assert_eq!(threads_for(|| Ok(3 * FULL_HD_NEED)), 3.min(cores));
        assert_eq!(threads_for(|| Ok(3 * FULL_HD_NEED - 1)), 2.min(cores));
        // A pool between one thread and the cores is its own size: a ceiling below the cores would show.
        assert_eq!(threads_for(|| Ok(10 * FULL_HD_NEED)), 10.min(cores));
        assert_eq!(threads_for(|| Ok(PLENTY)), cores);
    }

    #[test]
    fn a_job_the_memory_does_not_hold_back_runs_on_its_callers_own_threads() {
        let caller = std::thread::current().id();
        let inner = recording(1);
        assert_eq!(run_on(&PooledExporter::over(inner.clone(), || Ok(PLENTY)), &inner, 1920, 1080), (rayon::current_num_threads(), caller));
        let inner = recording(1);
        let held_back = run_on(&PooledExporter::over(inner.clone(), || Ok(FULL_HD_NEED)), &inner, 1920, 1080);
        assert_ne!(held_back.1, caller, "a job held back runs on its pool");
        assert_eq!(held_back.0, 1);
    }

    #[test]
    fn jobs_of_one_size_share_one_pool_and_another_size_replaces_it() {
        // This figure holds one 4K frame in flight, and two of the half-width comp, so the 4K job gets
        // the floor of one thread and the other a pool of two; the machines the tests run on have more
        // than two cores, so both jobs are held back.
        let inner = recording(1);
        let exporter = PooledExporter::over(inner.clone(), || Ok(4 * FULL_HD_NEED));
        let (one, first) = run_on(&exporter, &inner, 3840, 2160);
        assert_eq!((one, run_on(&exporter, &inner, 3840, 2160)), (1, (1, first)), "the second 4K job runs on the first one's thread");
        assert_eq!(run_on(&exporter, &inner, 1920, 2160).0, 2, "the half-width job needs a pool of two");
        let (one, again) = run_on(&exporter, &inner, 3840, 2160);
        assert_eq!(one, 1);
        assert_ne!(again, first, "the pool of one gave way to the pool of two");
    }

    /// The pool follows the figure this platform really gives, for frames from 1080p to 8K. On a phone's
    /// simulator or emulator this is the run that shows it; each figure and size goes to the run's output
    /// for the person reading it. A phone's figure moves while the test runs, so the pool is held to the
    /// sizes the figures just before and just after its export give. Where the figure cannot hold a frame
    /// in flight at all the export is refused instead, in words: that is what the phone shows when it is
    /// short, and the run shows the words and the figure they name.
    #[test]
    fn the_pool_follows_the_figure_the_platform_gives() {
        let cores = rayon::current_num_threads();
        let inner = recording(1);
        let exporter = PooledExporter::over(inner.clone(), available_memory);
        for (width, height) in [(1920, 1080), (3840, 2160), (5120, 2880), (7680, 4320)] {
            let frame = Frame { width, height, bytes: u64::from(width) * u64::from(height) * BYTES_PER_PIXEL, on: Compositor::Cpu };
            let need = frame_need(&frame);
            let (project, item) = project_with_comp(width, height);
            let before = available_memory();
            let outcome = with_job(&project, &item, |job| exporter.export(job, &mut |_, _| true));
            let after = available_memory();
            let (before, after) = match (outcome, before, after) {
                (Ok(_), Ok(before), Ok(after)) => (before, after),
                (Err(failed), Err(why), _) => {
                    eprintln!("available_memory() = {why:?}: a {width}x{height} export fails: {failed}");
                    assert_eq!(failed, unread(&frame, &why), "a platform with no figure fails the export with its reason");
                    continue;
                }
                (Err(refused), before, after) => {
                    eprintln!("available_memory() = {before:?} then {after:?}: a {width}x{height} export is refused: {refused}");
                    let said = refused
                        .strip_prefix(&format!("a frame of {width}x{height} needs {need} bytes to render on the CPU and write, and the device reports "))
                        .and_then(|said| said.strip_suffix(" bytes available"))
                        .and_then(|said| said.parse::<u64>().ok())
                        .expect("a refusal is in the promised words and names the figure it read");
                    assert!(said < need, "the refused figure {said} is not under one frame's need of {need}");
                    continue;
                }
                other => panic!("a {width}x{height} export with figures read but none used: {other:?}"),
            };
            let (threads, _) = inner.last();
            eprintln!("available_memory() = {before} then {after}, cores = {cores}, a {width}x{height} export's pool = {threads}");
            let (one, other) = (pool_size(before, need, cores), pool_size(after, need, cores));
            assert!((one.min(other)..=one.max(other)).contains(&threads), "{threads} threads, against {one} and {other}");
        }
    }

    #[test]
    fn the_hook_is_called_from_the_callers_thread_and_can_stop_the_export() {
        let (project, item) = project_with_comp(16, 16);
        let caller = std::thread::current().id();
        // A job held back runs on the pool and a job that is not runs on the caller's own threads; the
        // hook is the caller's either way. The held-back figure is exactly what the 16x16 frame needs.
        for (memory, on_the_pool) in [(|| Ok(SMALLEST_NEED)) as fn() -> Result<u64, String>, || Ok(PLENTY)].into_iter().zip([true, false]) {
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
        let exporter = PooledExporter::over(inner.clone(), || Ok(SMALLEST_NEED));
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
        let exporter = PooledExporter::over(Arc::new(PanicsOnce::default()), || Ok(SMALLEST_NEED));
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
        // Once on the caller's threads, and once held back to a pool of one.
        for memory in [(|| Ok(PLENTY)) as fn() -> Result<u64, String>, || Ok(SMALLEST_NEED)] {
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
