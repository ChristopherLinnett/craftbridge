//! The `timings` method: the command line's `bench`, answered as a document (plan §14, AD6).
//!
//! The command line prints its benchmark as text; this measures the same things on EffectCraft's
//! public render API and answers them as JSON in the same structure: the frame's wall and CPU
//! time over several runs without the layer cache, each layer's process and composite time and each
//! effect's time (the minimum of the runs, as the command line reports), and with `play` the cost of
//! consecutive frames without and with the cache. With `gpu`, the same comp against the attached
//! accelerator, as `bench --gpu` compares them. The numbers are measurements; only the structure is
//! the command line's.

use std::time::Instant;

use craftbridge_core::{CraftError, Result};
use effectcraft_automation::{Backend, Error};
use effectcraft_engine::Session;
use effectcraft_engine::project::{ItemId, Project};
use effectcraft_render::{Accelerator, AutoPick, Backend as RenderBackend, Image, LayerCache, LayerTiming, RenderOpts, Renderer};
use effectcraft_time::Tick;
use rustix::time::{ClockId, DynamicClockId, clock_gettime_dynamic};
use serde_json::Value;
use serde_json::map::Map;

use crate::methods::{failure, number_param, object, object_params, reject_unknown};

/// The command line's defaults: the frame it times is at three seconds, full size, over ten runs.
const DEFAULT_TIME: f64 = 3.0;
const DEFAULT_RUNS: u64 = 10;
/// The most runs or frames one call times. Every run holds its layers' timings until the call ends,
/// and a caller's number must not be able to ask for more than a phone can hold or wait for.
const MOST_RUNS: u64 = 1000;
/// The largest scale timed: Full. A frame's pixels grow with the square of the scale, so a caller's
/// scale must not be able to ask for a frame larger than the comp's own, which every preview and
/// export of it already renders; past what the phone can hold, the allocation stops the process.
const MOST_SCALE: f64 = 1.0;

/// `timings`: `{comp?, time?, scale?, runs?, play?, gpu?}`.
pub(crate) fn timings(backend: &mut Backend, params: &Value) -> Result<Value> {
    let obj = object_params(params, "timings")?;
    reject_unknown(&obj, "timings", &["comp", "time", "scale", "runs", "play", "gpu"])?;
    let time = number_param(&obj, "time", "timings")?.unwrap_or(DEFAULT_TIME);
    let scale = number_param(&obj, "scale", "timings")?.unwrap_or(1.0);
    if !time.is_finite() || !scale.is_finite() || scale <= 0.0 || scale > MOST_SCALE {
        return Err(CraftError::invalid_params(format!("`timings`: `time` must be a number and `scale` a number above zero and at most {MOST_SCALE}")));
    }
    let runs = count_param(&obj, "runs")?.unwrap_or(DEFAULT_RUNS);
    let play = count_param(&obj, "play")?;
    let gpu = match obj.get("gpu") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err(CraftError::invalid_params("`timings`: `gpu` must be a bool")),
    };
    let session = backend.session().ok_or_else(|| CraftError::internal("`timings`: the backend holds no session"))?;
    let comp = session.resolve_comp(obj.get("comp")).map_err(|error| failure(Error::Engine(error), "timings", params, false))?;
    let accelerator = match (gpu, session.accel.clone()) {
        (true, None) => {
            let why = session.accel_note.clone().unwrap_or_else(|| "no GPU is attached".to_string());
            return Err(CraftError::unsupported(format!("`timings`: `gpu` needs a GPU, and this session has none: {why}")));
        }
        (true, accel) => accel,
        (false, _) => None,
    };
    let at = Tick::from_seconds_f64(time);
    let opts = RenderOpts { scale, ..Default::default() };
    let name = session.project.item(comp).map(|item| item.name.clone()).unwrap_or_default();
    let mut doc = vec![
        ("comp", Value::from(comp.0)),
        ("name", Value::from(name)),
        ("time", Value::from(time)),
        ("scale", Value::from(scale)),
        ("runs", Value::from(runs)),
        ("frame", frame_timings(session, comp, at, opts, runs, accelerator.as_deref())),
    ];
    if let Some(frames) = play {
        doc.push(("play", play_timings(session, comp, at, opts, frames)));
    }
    if let Some(accelerator) = accelerator.as_deref() {
        doc.push(("gpu", gpu_timings(session, comp, time, runs, accelerator)));
    }
    Ok(Value::Object(doc.into_iter().map(|(key, value)| (key.to_string(), value)).collect::<Map<_, _>>()))
}

/// A whole number of runs or frames, from one up to [`MOST_RUNS`].
fn count_param(obj: &Map<String, Value>, key: &str) -> Result<Option<u64>> {
    match number_param(obj, key, "timings")? {
        None => Ok(None),
        Some(number) if number.fract() == 0.0 && (1.0..=MOST_RUNS as f64).contains(&number) => Ok(Some(number as u64)),
        Some(_) => Err(CraftError::invalid_params(format!("`timings`: `{key}` must be a whole number from 1 to {MOST_RUNS}"))),
    }
}

/// One timed frame: wall milliseconds, the process's CPU milliseconds (every thread: a measure of
/// work that load does not move) and each layer's timings.
struct Timed {
    wall: f64,
    cpu: f64,
    layers: Vec<LayerTiming>,
}

/// The process's CPU time so far, in milliseconds, read from the same clock the command line's
/// `bench` reads (`CLOCK_PROCESS_CPUTIME_ID`), and like it 0 when the clock cannot be read.
fn cpu_ms() -> f64 {
    let clock = DynamicClockId::Known(ClockId::ProcessCPUTime);
    clock_gettime_dynamic(clock).map(|now| now.tv_sec as f64 * 1e3 + now.tv_nsec as f64 / 1e6).unwrap_or(0.0)
}

fn render_timed(session: &Session, comp: ItemId, at: Tick, opts: RenderOpts, cache: Option<&LayerCache>) -> Timed {
    let layers = std::sync::Mutex::new(Vec::<LayerTiming>::new());
    let mut renderer = Renderer::new(&session.project, session.footage.as_ref(), opts);
    renderer.expr = session.expr.as_deref();
    renderer.cache = cache;
    renderer.profile = Some(&layers);
    let cpu_before = cpu_ms();
    let started = Instant::now();
    let image = renderer.comp_frame(comp, at);
    let wall = started.elapsed().as_secs_f64() * 1e3;
    let cpu = cpu_ms() - cpu_before;
    std::hint::black_box(&image);
    Timed { wall, cpu, layers: layers.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner) }
}

fn minimum(values: &[f64]) -> f64 {
    values.iter().copied().fold(f64::INFINITY, f64::min)
}

/// The median as the command line's `bench` takes it: the middle value once sorted, the upper of the
/// two middle ones for an even count, and 0 for none.
fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

/// One layer across the runs: its place in the tree, and every run's process, composite and
/// per-effect milliseconds, in the order the renderer reports the layers and effects.
struct LayerRuns {
    depth: usize,
    layer: String,
    process: Vec<f64>,
    composite: Vec<f64>,
    effects: Vec<(String, Vec<f64>)>,
}

/// `frame`: the frame rendered `runs` times without the layer cache, which is the whole cost of a
/// frame, as `bench` prints it: wall minimum and median, the CPU minimum, and the minimum per layer
/// and per effect. `gpu_supported` marks, when an accelerator is attached, each effect it implements.
fn frame_timings(session: &Session, comp: ItemId, at: Tick, opts: RenderOpts, runs: u64, accelerator: Option<&dyn Accelerator>) -> Value {
    let (mut walls, mut cpus) = (Vec::new(), Vec::new());
    let mut layers: Vec<LayerRuns> = Vec::new();
    for _ in 0..runs {
        let timed = render_timed(session, comp, at, opts, None);
        walls.push(timed.wall);
        cpus.push(timed.cpu);
        for (position, timing) in timed.layers.into_iter().enumerate() {
            if layers.len() <= position {
                layers.push(LayerRuns { depth: timing.depth, layer: timing.layer.clone(), process: vec![], composite: vec![], effects: vec![] });
            }
            let Some(row) = layers.get_mut(position) else { continue };
            row.process.push(timing.process_ms);
            row.composite.push(timing.composite_ms);
            for (slot, (id, ms)) in timing.effects.into_iter().enumerate() {
                if row.effects.len() <= slot {
                    row.effects.push((id, vec![]));
                }
                if let Some((_, times)) = row.effects.get_mut(slot) {
                    times.push(ms);
                }
            }
        }
    }
    let rows: Vec<Value> = layers.iter().map(|row| layer_row(row, accelerator)).collect();
    object([
        ("wallMinMs", Value::from(minimum(&walls))),
        ("wallMedianMs", Value::from(median(&mut walls))),
        ("cpuMinMs", Value::from(minimum(&cpus))),
        ("layers", Value::Array(rows)),
    ])
}

fn layer_row(row: &LayerRuns, accelerator: Option<&dyn Accelerator>) -> Value {
    let effects = row
        .effects
        .iter()
        .map(|(id, times)| {
            let mut effect = vec![("id", Value::from(id.as_str())), ("ms", Value::from(minimum(times)))];
            if let Some(accelerator) = accelerator {
                effect.push(("gpuSupported", Value::Bool(accelerator.supports_effect(id))));
            }
            Value::Object(effect.into_iter().map(|(key, value)| (key.to_string(), value)).collect())
        })
        .collect();
    object([
        ("depth", Value::from(row.depth)),
        ("layer", Value::from(row.layer.as_str())),
        ("processMs", Value::from(minimum(&row.process))),
        ("compositeMs", Value::from(minimum(&row.composite))),
        ("effects", Value::Array(effects)),
    ])
}

/// `play`: `frames` consecutive frames from `at`, as playback renders them, without the layer cache
/// and then with it, and the cache's own counts afterwards, as `bench-play` prints them.
fn play_timings(session: &Session, comp: ItemId, at: Tick, opts: RenderOpts, frames: u64) -> Value {
    let Some(item) = session.project.comp(comp) else { return Value::Null };
    let first = item.frame_rate.frame_at(at);
    let cache = LayerCache::default();
    let pass = |cache: Option<&LayerCache>| {
        let timed: Vec<Timed> =
            (0..frames).map(|index| render_timed(session, comp, item.frame_rate.tick_of(first.saturating_add(index as i64)), opts, cache)).collect();
        let mut walls: Vec<f64> = timed.iter().map(|frame| frame.wall).collect();
        let cpus: Vec<f64> = timed.iter().map(|frame| frame.cpu).collect();
        object([
            ("frames", Value::from(frames)),
            ("wallMeanMs", Value::from(mean(&walls))),
            ("wallMedianMs", Value::from(median(&mut walls))),
            ("wallMinMs", Value::from(minimum(&walls))),
            ("cpuMeanMs", Value::from(mean(&cpus))),
        ])
    };
    let without = pass(None);
    let with = pass(Some(&cache));
    let stats = cache.stats();
    object([
        ("noCache", without),
        ("layerCache", with),
        (
            "cache",
            object([
                ("hits", Value::from(stats.hits)),
                ("misses", Value::from(stats.misses)),
                ("entries", Value::from(stats.entries)),
                ("bytes", Value::from(stats.bytes)),
            ]),
        ),
    ])
}

/// Median wall milliseconds of `runs` runs of `work` after one warm-up run.
fn time_ms(runs: u64, mut work: impl FnMut()) -> f64 {
    work();
    let mut times: Vec<f64> = (0..runs)
        .map(|_| {
            let started = Instant::now();
            work();
            started.elapsed().as_secs_f64() * 1e3
        })
        .collect();
    median(&mut times)
}

/// `gpu`: the comp at full and half size on the CPU against the attached accelerator, as `bench --gpu`
/// compares them: a frame with nothing cached ("cold") and with the layer cache filled ("warm"),
/// what Auto picks once both sides have warmed up, the speedup of warm frames, and the share of
/// pixels where the GPU frame differs from the CPU's by more than one 8-bit level. The frame is the
/// one `bench --gpu` picks, at `time` but no later than half the comp's duration, and its time is
/// reported. The command line's display-path and transfer columns need its own GPU type, which the
/// render API does not carry, so they are not here.
fn gpu_timings(session: &Session, comp: ItemId, time: f64, runs: u64, accelerator: &dyn Accelerator) -> Value {
    let duration = session.project.comp(comp).map_or(0.0, |item| item.duration.seconds());
    let seconds = time.min(duration * 0.5);
    let at = Tick::from_seconds_f64(seconds);
    let mut project: Project = (*session.project).clone();
    project.settings.gpu_acceleration = true;
    let render = |scale: f64, backend: RenderBackend, cache: Option<&LayerCache>| -> Image {
        let mut renderer = Renderer::new(&project, session.footage.as_ref(), RenderOpts { scale, backend, ..Default::default() });
        renderer.expr = session.expr.as_deref();
        renderer.cache = cache;
        renderer.accel = Some(accelerator);
        renderer.comp_frame(comp, at)
    };
    let mut rows = Vec::new();
    for (label, scale) in [("Full", 1.0), ("Half", 0.5)] {
        let cpu_cold = time_ms(runs, || drop(std::hint::black_box(render(scale, RenderBackend::Cpu, None))));
        let gpu_cold = time_ms(runs, || drop(std::hint::black_box(render(scale, RenderBackend::Gpu, None))));
        let (cpu_cache, gpu_cache) = (LayerCache::default(), LayerCache::default());
        let cpu_warm = time_ms(runs, || drop(std::hint::black_box(render(scale, RenderBackend::Cpu, Some(&cpu_cache)))));
        let gpu_warm = time_ms(runs, || drop(std::hint::black_box(render(scale, RenderBackend::Gpu, Some(&gpu_cache)))));
        for _ in 0..2 * AutoPick::WARMUP {
            drop(std::hint::black_box(render(scale, RenderBackend::Auto, Some(&gpu_cache))));
        }
        let auto = time_ms(runs, || drop(std::hint::black_box(render(scale, RenderBackend::Auto, Some(&gpu_cache)))));
        let (cpu_frame, gpu_frame) = (render(scale, RenderBackend::Cpu, Some(&cpu_cache)), render(scale, RenderBackend::Gpu, Some(&gpu_cache)));
        rows.push(object([
            ("resolution", Value::from(label)),
            ("width", Value::from(cpu_frame.width)),
            ("height", Value::from(cpu_frame.height)),
            ("cpuColdMs", Value::from(cpu_cold)),
            ("gpuColdMs", Value::from(gpu_cold)),
            ("cpuWarmMs", Value::from(cpu_warm)),
            ("gpuWarmMs", Value::from(gpu_warm)),
            ("autoMs", Value::from(auto)),
            ("speedup", Value::from(cpu_warm / gpu_warm.max(1e-9))),
            ("differsPercent", Value::from(differing_percent(&cpu_frame, &gpu_frame))),
        ]));
    }
    object([("adapter", Value::from(accelerator.name())), ("time", Value::from(seconds)), ("frames", Value::Array(rows))])
}

/// The share of pixels, in percent, where two frames differ by more than one 8-bit level on any
/// channel; a NaN counts as a difference.
fn differing_percent(cpu: &Image, gpu: &Image) -> f64 {
    let differing = cpu
        .data
        .iter()
        .zip(&gpu.data)
        .filter(|(left, right)| left.iter().zip(right.iter()).any(|(a, b)| (a - b).abs().is_nan() || (a - b).abs() > 1.0 / 255.0 + 1e-6))
        .count();
    100.0 * differing as f64 / cpu.data.len().max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The median `bench` reports: the middle value, the upper of the two middle ones for an even
    /// count.
    #[test]
    fn the_median_is_the_one_the_command_line_takes() {
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), 3.0);
        assert_eq!(median(&mut [2.0, 9.0, 1.0]), 2.0);
        assert_eq!(median(&mut []), 0.0);
    }

    /// The GPU half, on the machine's own adapter: the document carries the CPU against the GPU for
    /// both sizes at the frame `bench --gpu` picks, no later than half the comp's duration, and each
    /// effect says whether the accelerator implements it. A machine with no adapter cannot attach
    /// one, and then the call's refusal for want of a GPU is what is checked.
    #[test]
    fn the_gpu_section_compares_both_sizes_when_an_accelerator_is_attached() {
        let folders = crate::session::tests::folders(None);
        let mut backend = Backend::headless(crate::session::build(&folders));
        backend.exec("file.openDemoProject", json!({})).expect("the demo opens");
        let Ok(gpu) = effectcraft_gpu::Gpu::try_headless() else {
            let refused = timings(&mut backend, &json!({"gpu": true, "runs": 1}));
            assert!(matches!(refused, Err(CraftError::Unsupported { .. })), "{refused:?}");
            return;
        };
        let session = backend.session().expect("a session");
        session.accel = Some(std::sync::Arc::new(gpu));
        let active = session.active_comp_id().expect("the demo's comp");
        let duration = session.project.comp(active).expect("a comp").duration.seconds();
        let doc = timings(&mut backend, &json!({"gpu": true, "runs": 1, "scale": 0.25, "time": duration})).expect("timings answer");
        assert_eq!(doc["time"], json!(duration), "the frame is timed where it was asked");
        assert_eq!(doc["gpu"]["time"], json!(duration * 0.5), "the GPU compares the frame `bench --gpu` picks");
        let rows = doc["gpu"]["frames"].as_array().expect("frames");
        assert_eq!(rows.iter().map(|row| row["resolution"].clone()).collect::<Vec<_>>(), [json!("Full"), json!("Half")]);
        for key in ["cpuColdMs", "gpuColdMs", "cpuWarmMs", "gpuWarmMs", "autoMs", "speedup", "differsPercent"] {
            assert!(rows.iter().all(|row| row[key].is_f64()), "{key}: {doc}");
        }
        let effect = doc["frame"]["layers"].as_array().into_iter().flatten().flat_map(|l| l["effects"].as_array().into_iter().flatten()).next();
        assert!(effect.is_none_or(|e| e["gpuSupported"].is_boolean()), "{effect:?}");
    }
}
