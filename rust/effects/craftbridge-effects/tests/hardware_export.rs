//! The acceptance of plan §28 T9 and T10 with the hardware option, and of "every other format unchanged":
//! an `export` that chose hardware encoding writes a movie of the same frame count and size whose every
//! decoded frame is within D-12's 40 dB of the software export's, and any job the option does not apply to
//! writes exactly what it writes without it. A device without the encoder declines the option, which the
//! same tests hold to the same bytes as the software export.
//!
//! Runs where VideoToolbox does: on a Mac, in the iOS Simulator and on an iPhone. Every export writes to the
//! run's output the most memory it held, as the platform reports it, so a run on an iPhone shows what an
//! export holds beside the app's limit; so do the two tests that make the most exports in one app, whose own
//! peak shows whether what one export holds outlives it. The helpers answer `Result` and leave the
//! unwrapping to the `#[test]` functions, as everywhere else in this repository.

#![cfg(target_vendor = "apple")]

mod scratch;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use craftbridge_core::{CraftApp, CraftError, available_memory};
use craftbridge_effects::EffectCraftApp;
use craftbridge_media::{ColourCodes, HardwareEncoder, RateControl, VideoCodec, VideoSpec};
use effectcraft_engine::project::ItemId;
use effectcraft_media::MediaPool;
use effectcraft_render::FootageSource;
use effectcraft_time::Tick;
use scratch::Scratch;
use serde_json::{Value, json};

/// D-12's threshold.
const D12_DB: f64 = 40.0;

/// A short, small slice of the demo project: half a second at a quarter of its size.
fn small(format: &str, output: &Path) -> Value {
    json!({"format": format, "output": output.display().to_string(), "end": 0.5, "resolution": "quarter"})
}

/// `request` with `key` set to `value`.
fn with(mut request: Value, key: &str, value: Value) -> Value {
    if let Value::Object(fields) = &mut request {
        fields.insert(key.to_string(), value);
    }
    request
}

fn started(scratch: &Scratch) -> Result<EffectCraftApp, String> {
    let mut app = scratch.app().map_err(|e| e.to_string())?;
    app.request("exec", json!({"command": "file.openDemoProject"})).map_err(|e| format!("the demo project opens: {e}"))?;
    Ok(app)
}

/// One engine command through the app's `exec`, its refusal worded with the command's name.
fn exec(app: &mut EffectCraftApp, command: &str, params: Value) -> Result<Value, String> {
    app.request("exec", json!({"command": command, "params": params})).map_err(|e| format!("{command}: {e}"))
}

fn export(app: &mut EffectCraftApp, request: Value) -> Result<Value, CraftError> {
    // The output's file and the folder it is in, which tell one test's exports apart.
    let output: Vec<_> = Path::new(request.get("output").and_then(Value::as_str).unwrap_or_default()).iter().collect();
    let named: PathBuf = output.iter().skip(output.len().saturating_sub(2)).collect();
    measured(&format!("the export to {}", named.display()), || app.request_with_progress("export", request, &mut |_, _, _| true, &mut |_| Ok(())))
}

/// How often the watch below reads the platform's figure: a frame's render and encode hold their buffers
/// for longer than this, so what they hold shows, and one read of the figure costs a call into the system.
const SAMPLE_EVERY: Duration = Duration::from_millis(2);

/// The platform's figure for what the app may still allocate (`available_memory()`, which on an iPhone is
/// `os_proc_available_memory()`), read every [`SAMPLE_EVERY`] on a thread of its own from the watch's
/// start until it finishes or is dropped. The figure falls by what the app holds, so its lowest point against its
/// value at the start is the most the app held at once on top of what it held then. On a Mac and in the iOS
/// Simulator the figure is the host's, which moves in whole percents of its memory, so the peak there is coarse.
struct Watch {
    start: Option<u64>,
    lowest: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Watch {
    fn start() -> Self {
        let start = available_memory().ok();
        let (lowest, stop) = (Arc::new(AtomicU64::new(start.unwrap_or(u64::MAX))), Arc::new(AtomicBool::new(false)));
        let (reads, stops) = (Arc::clone(&lowest), Arc::clone(&stop));
        let thread = std::thread::spawn(move || {
            while !stops.load(Ordering::SeqCst) {
                if let Ok(now) = available_memory() {
                    reads.fetch_min(now, Ordering::SeqCst);
                }
                std::thread::sleep(SAMPLE_EVERY);
            }
        });
        Self { start, lowest, stop, thread: Some(thread) }
    }

    /// The figure at the start and its lowest point since, once the watch has stopped; `None` where the
    /// platform's figure could not be read at the start.
    fn finish(mut self) -> Option<(u64, u64)> {
        self.stop_reading();
        if let Ok(now) = available_memory() {
            self.lowest.fetch_min(now, Ordering::SeqCst);
        }
        Some((self.start?, self.lowest.load(Ordering::SeqCst)))
    }

    fn stop_reading(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Watch {
    /// A test that fails mid-way leaves no thread reading behind it.
    fn drop(&mut self) {
        self.stop_reading();
    }
}

/// `body`, with the most memory it held at once written to the run's output under `what`, so a device run
/// shows each export's peak and the app's room beside it. The figure is 0 for an app at the system's limit
/// for it, so a lowest point above 0 is an app that stayed under its limit throughout.
fn measured<R>(what: &str, body: impl FnOnce() -> R) -> R {
    let watch = Watch::start();
    let answer = body();
    match watch.finish() {
        None => eprintln!("{what}: the platform's memory figure could not be read"),
        Some((start, lowest)) => {
            let mib = |bytes: u64| bytes >> 20;
            eprintln!(
                "{what}: peak {} MiB, the app's room at least {} MiB of the {} MiB at the start",
                mib(start.saturating_sub(lowest)),
                mib(lowest),
                mib(start)
            );
            assert!(lowest > 0, "{what} reached the app's memory limit");
        }
    }
    answer
}

/// Whether this device encodes `codec` in hardware, which the tests meet with the bytes they hold the option
/// to.
fn has_hardware(codec: VideoCodec) -> bool {
    let spec = VideoSpec {
        codec,
        width: 64,
        height: 64,
        rate_num: 30,
        rate_den: 1,
        rate_control: RateControl::Bitrate { kbps: 1000 },
        keyframe_interval: 30,
        colour: ColourCodes { primaries: 1, transfer: 1, matrix: 1 },
    };
    HardwareEncoder::start(spec).is_ok()
}

/// The movie at `path` decoded by EffectCraft's own media pool: its size, and each frame as 8-bit RGBA.
fn decoded(path: &Path) -> Result<(u32, u32, Vec<Vec<u8>>), String> {
    let footage = effectcraft_media::probe(path.display().to_string()).map_err(|e| e.to_string())?;
    let pool = MediaPool::new();
    let rate = footage.frame_rate;
    let count = (footage.duration.seconds() * rate.as_f64()).round() as u64;
    let mut frames = Vec::new();
    for index in 0..count {
        let at = Tick::from_seconds_f64(index as f64 * rate.den as f64 / rate.num as f64);
        let image = pool.frame(ItemId(1), &footage, at).ok_or_else(|| format!("frame {index} of {} does not decode", path.display()))?;
        frames.push(image.to_rgba8_over([0.0; 3]));
    }
    Ok((footage.width, footage.height, frames))
}

fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let error = a.iter().zip(b).map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2)).sum::<f64>() / a.len().max(1) as f64;
    if error == 0.0 { f64::INFINITY } else { 10.0 * (255.0f64 * 255.0 / error).log10() }
}

/// The lowest PSNR between the frames of two movies, after holding them to the same frame count and size.
fn lowest_psnr(expected: &Path, actual: &Path) -> Result<(usize, f64), String> {
    let ((sw, sh, expected), (hw, hh, actual)) = (decoded(expected)?, decoded(actual)?);
    assert_eq!((hw, hh, actual.len()), (sw, sh, expected.len()), "frame count and size");
    assert!(!expected.is_empty());
    let lowest = actual.iter().zip(&expected).map(|(a, b)| psnr(a, b)).fold(f64::INFINITY, f64::min);
    Ok((expected.len(), lowest))
}

/// `codec`'s movie from the demo, once as the command line writes it and once with hardware: the same frame
/// count and size, every frame within D-12, and (where the device has the encoder) bytes that differ.
fn hardware_matches_software(format: &str, codec: VideoCodec) -> Result<(), String> {
    let scratch = Scratch::new().map_err(|e| e.to_string())?;
    let mut app = started(&scratch)?;
    let software = scratch.path().join("software.mp4");
    let hardware = scratch.path().join("hardware.mp4");
    export(&mut app, small(format, &software)).map_err(|e| e.to_string())?;
    let answer = export(&mut app, with(small(format, &hardware), "hardware", json!(true))).map_err(|e| e.to_string())?;
    assert_eq!(answer.pointer("/rendered/0/output").and_then(Value::as_str), Some(hardware.display().to_string().as_str()), "{answer}");
    let same = std::fs::read(&software).map_err(|e| e.to_string())? == std::fs::read(&hardware).map_err(|e| e.to_string())?;
    assert_eq!(!same, has_hardware(codec), "the movie came from the hardware encoder exactly when the device has one");
    let (frames, lowest) = lowest_psnr(&software, &hardware)?;
    eprintln!("{format}: {frames} frames, the lowest PSNR against the software export is {lowest:.1} dB");
    assert!(lowest >= D12_DB, "{format}: {lowest:.1} dB is under D-12's {D12_DB}");
    Ok(())
}

#[test]
fn an_h264_export_with_hardware_is_within_d12_of_the_software_export() {
    hardware_matches_software("h264", VideoCodec::H264).unwrap();
}

#[test]
fn an_hevc_export_with_hardware_is_within_d12_of_the_software_export() {
    hardware_matches_software("hevc", VideoCodec::Hevc).unwrap();
}

/// The files `dir` holds, by name, with their bytes.
fn files(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().filter(|entry| entry.path().is_file()) {
        files.push((entry.file_name().to_string_lossy().into_owned(), std::fs::read(entry.path()).map_err(|e| e.to_string())?));
    }
    files.sort();
    Ok(files)
}

/// What `request` writes into a folder of its own without the option, and with it.
fn both_ways(request: impl Fn(&Path) -> Value) -> Result<(Vec<(String, Vec<u8>)>, Vec<(String, Vec<u8>)>), String> {
    let scratch = Scratch::new().map_err(|e| e.to_string())?;
    let mut app = started(&scratch)?;
    let mut written = Vec::new();
    for (name, hardware) in [("plain", false), ("option", true)] {
        let folder = scratch.path().join(name);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        export(&mut app, with(request(&folder), "hardware", json!(hardware))).map_err(|e| e.to_string())?;
        written.push(files(&folder)?);
    }
    match (written.pop(), written.pop()) {
        (Some(option), Some(plain)) => Ok((plain, option)),
        _ => Err("two folders were written".into()),
    }
}

#[test]
fn every_other_format_is_written_as_it_is_without_the_option() {
    measured("every_other_format_is_written_as_it_is_without_the_option", || {
        // Every format of the render queue's but the two the option encodes, the other MP4 movie among them.
        for (format, file) in [
            ("av1", "out.mp4"),
            ("prores", "out.mov"),
            ("uncompressed", "out.mov"),
            ("webm", "out.webm"),
            ("png", "out.png"),
            ("jpeg", "out.jpg"),
            ("tiff", "out.tif"),
            ("exr", "out.exr"),
            ("tga", "out.tga"),
            ("gif", "out.gif"),
            ("wav", "out.wav"),
            ("aiff", "out.aif"),
        ] {
            let (plain, option) = both_ways(|folder| small(format, &folder.join(file))).unwrap();
            assert!(!plain.is_empty(), "{format} wrote something");
            assert_eq!(plain, option, "{format}");
        }
    });
}

#[test]
fn a_movie_the_hardware_does_not_reproduce_stays_with_effectcrafts_encoders() {
    measured("a_movie_the_hardware_does_not_reproduce_stays_with_effectcrafts_encoders", || {
        // HEVC at Main 10 and at a chosen level are what the hardware encoder does not take, so the job is
        // the software one.
        for (key, value) in [("profile", "main10"), ("level", "4.1")] {
            let (plain, option) = both_ways(|folder| with(small("hevc", &folder.join("out.mp4")), key, json!(value))).unwrap();
            assert!(!plain.is_empty());
            assert_eq!(plain, option, "{key} {value}");
        }
    });
}

#[test]
fn the_option_lasts_for_the_one_export_that_chose_it() {
    let scratch = Scratch::new().unwrap();
    let mut app = started(&scratch).unwrap();
    let paths = ["a.mp4", "b.mp4", "c.mp4"].map(|name| scratch.path().join(name));
    let [first, second, third] = &paths;
    export(&mut app, small("h264", first)).unwrap();
    export(&mut app, with(small("h264", second), "hardware", json!(true))).unwrap();
    export(&mut app, small("h264", third)).unwrap();
    let read = |path: &Path| std::fs::read(path).unwrap();
    assert_eq!(read(first), read(third), "an export that did not choose hardware after one that did is the software export");
    assert_eq!(read(first) != read(second), has_hardware(VideoCodec::H264));
    assert!(scratch.path().starts_with(scratch.root()));
}

#[test]
fn a_stopped_hardware_export_leaves_no_movie() {
    let scratch = Scratch::new().unwrap();
    let mut app = started(&scratch).unwrap();
    let output = scratch.path().join("stopped.mp4");
    // The whole demo at full size: the stop lands only on a progress beat, and EffectCraft, optimised in
    // the tests as in its own workspace, renders a short quarter-size export's frames between two beats.
    // The export stops at its first frames, so its length costs nothing.
    let request = json!({"format": "h264", "output": output.display().to_string(), "hardware": true, "end": 10, "resolution": "full"});
    let answer = app.request_with_progress("export", request, &mut |done, _, _| done < 2, &mut |_| Ok(()));
    assert!(matches!(answer, Err(CraftError::Cancelled)), "{answer:?}");
    assert!(!output.exists(), "no half-written movie is left");
}

#[test]
fn the_option_is_a_boolean() {
    let scratch = Scratch::new().unwrap();
    let mut app = started(&scratch).unwrap();
    let request = with(small("h264", &scratch.path().join("x.mp4")), "hardware", json!("yes"));
    assert_eq!(export(&mut app, request).unwrap_err().to_string(), "`export`: `hardware` must be a bool");
}

/// An export that chose hardware encoding is still run by the render runner, so it lists every file
/// the export may write — the movie and the render log beside it — through the progress call's
/// lister before it writes any of them (AD19).
#[test]
fn a_hardware_export_lists_the_files_it_may_write_before_writing_them() {
    let scratch = Scratch::new().unwrap();
    let mut app = started(&scratch).unwrap();
    let movie = scratch.path().join("listed.mp4");
    let mut listed = Vec::new();
    let mut written_while_listing = false;
    app.request_with_progress("export", with(small("h264", &movie), "hardware", json!(true)), &mut |_, _, _| true, &mut |paths: &[PathBuf]| {
        written_while_listing |= movie.exists();
        listed.extend_from_slice(paths);
        Ok(())
    })
    .unwrap();
    assert!(listed.contains(&movie), "the movie was not listed: {listed:?}");
    let log = movie.with_file_name("listed_RenderLog.txt");
    assert!(listed.contains(&log), "the render log was not listed: {listed:?}");
    assert!(!written_while_listing, "the movie stood written before the listing was over");
    assert!(movie.exists(), "and the export did write the movie");
}

/// Writes `seconds` of a 440 Hz tone as a 48 kHz 16-bit stereo WAV.
fn write_tone(path: &Path, seconds: usize) -> Result<(), String> {
    let samples = 48_000 * seconds;
    let mut data = Vec::with_capacity(samples * 4);
    for n in 0..samples {
        let sample = ((n as f64 * 440.0 * std::f64::consts::TAU / 48_000.0).sin() * 12_000.0) as i16;
        data.extend_from_slice(&sample.to_le_bytes());
        data.extend_from_slice(&sample.to_le_bytes());
    }
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&48_000u32.to_le_bytes());
    wav.extend_from_slice(&192_000u32.to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    std::fs::write(path, wav).map_err(|e| e.to_string())
}

/// The demo project with a second of the tone laid in its active composition, from its start.
fn with_sound(scratch: &Scratch) -> Result<EffectCraftApp, String> {
    let mut app = started(scratch)?;
    let tone = scratch.path().join("tone.wav");
    write_tone(&tone, 1)?;
    let imported = app.request("exec", json!({"command": "file.import", "params": {"paths": [tone.display().to_string()]}})).map_err(|e| e.to_string())?;
    let item = imported.pointer("/items/0").and_then(Value::as_u64).ok_or("the tone is imported")?;
    let info = app.request("info", json!({})).map_err(|e| e.to_string())?;
    let comp = info.pointer("/activeComp/id").and_then(Value::as_u64).ok_or("an active composition")?;
    app.request("exec", json!({"command": "layer.addItem", "params": {"comp": comp, "item": item, "time": 0.0}})).map_err(|e| e.to_string())?;
    Ok(app)
}

/// The loudness of the first `seconds` of the movie's sound, in dB below full scale, or `None` without sound.
fn loudness(path: &Path, seconds: f64) -> Option<f64> {
    let footage = effectcraft_media::probe(path.display().to_string()).ok()?;
    if !footage.has_audio {
        return None;
    }
    let samples = MediaPool::new().audio(ItemId(1), &footage, Tick::from_seconds_f64(0.0), (48_000.0 * seconds) as usize, 48_000)?;
    let energy = samples.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / samples.len().max(1) as f64;
    Some(10.0 * energy.log10())
}

#[test]
fn a_movie_with_sound_has_the_sound_and_the_pictures_of_the_software_export() {
    let scratch = Scratch::new().unwrap();
    let mut app = with_sound(&scratch).unwrap();
    let (software, hardware) = (scratch.path().join("software.mp4"), scratch.path().join("hardware.mp4"));
    let request = with(small("h264", &software), "end", json!(1.0));
    export(&mut app, request.clone()).unwrap();
    let chosen = with(with(request, "output", json!(hardware.display().to_string())), "hardware", json!(true));
    assert!(export(&mut app, chosen).unwrap().get("rendered").is_some());
    // The sound is the hardware path's too: a movie whose sound it could not make would fall back whole, and
    // the levels below would hold for that software export as well.
    let same = std::fs::read(&software).unwrap() == std::fs::read(&hardware).unwrap();
    assert_eq!(!same, has_hardware(VideoCodec::H264), "the movie with sound came from the hardware encoder exactly when the device has one");
    let (expected, actual) =
        (loudness(&software, 0.8).expect("the software export has sound"), loudness(&hardware, 0.8).expect("the hardware export has sound"));
    eprintln!("the sound's level: {expected:.1} dB in the software export, {actual:.1} dB in the hardware export");
    assert!((expected - actual).abs() < 1.5, "{expected} against {actual}");
    assert!(lowest_psnr(&software, &hardware).unwrap().1 >= D12_DB);
    let (a, b) = (effectcraft_media::probe(software.display().to_string()).unwrap(), effectcraft_media::probe(hardware.display().to_string()).unwrap());
    assert!((a.duration.seconds() - b.duration.seconds()).abs() < 0.05, "{} against {}", a.duration.seconds(), b.duration.seconds());
}

/// A template of a solid's colour, made inside the container with the app's own commands: a 256 x 144
/// composition of one second, big enough for the hardware encoder to take.
fn a_template(scratch: &Scratch) -> Result<PathBuf, String> {
    let template = scratch.path().join("solid.ectemplate");
    let mut app = scratch.app().map_err(|e| e.to_string())?;
    let mut exec = |command: &str, params: Value| exec(&mut app, command, params);
    exec("comp.new", json!({"name": "Solid", "width": 256, "height": 144, "duration": 1.0, "frameRate": 30}))?;
    exec("layer.newSolid", json!({"comp": 1, "name": "Bg", "color": "#223344"}))?;
    exec("effect.apply", json!({"layers": ["Bg"], "effect": "fill"}))?;
    exec("essential.addProperty", json!({"comp": 1, "layer": "Bg", "path": "effects/#1/color", "name": "Colour"}))?;
    exec("essential.exportTemplate", json!({"comp": 1, "path": template.display().to_string()}))?;
    Ok(template)
}

/// `template.render` of the template, writing `output` as `request` says.
fn render_template(app: &mut EffectCraftApp, template: &Path, request: Value) -> Result<Value, CraftError> {
    let params = json!({"template": template.display().to_string(), "values": {"Colour": "#aa3300"}, "output": request});
    app.request_with_progress("template.render", params, &mut |_, _, _| true, &mut |_| Ok(()))
}

/// A template render's `output` is an export's request, so its choice of hardware is read the same way:
/// the movie is the hardware encoder's exactly when `output` sets `hardware` and the device has the
/// encoder, and the render after it, which does not set it, is the software one.
#[test]
fn a_template_render_whose_output_sets_hardware_is_encoded_in_hardware() {
    let scratch = Scratch::new().unwrap();
    let template = a_template(&scratch).unwrap();
    let mut app = scratch.app().unwrap();
    let paths = ["a.mp4", "b.mp4", "c.mp4"].map(|name| scratch.path().join(name));
    let [software, hardware, after] = &paths;
    let request = |output: &Path| json!({"format": "h264", "output": output.display().to_string(), "end": 0.5});
    render_template(&mut app, &template, request(software)).unwrap();
    render_template(&mut app, &template, with(request(hardware), "hardware", json!(true))).unwrap();
    render_template(&mut app, &template, request(after)).unwrap();
    let read = |path: &Path| std::fs::read(path).unwrap();
    assert_eq!(read(software), read(after), "a render that did not set hardware after one that did is the software render");
    assert_eq!(read(software) != read(hardware), has_hardware(VideoCodec::H264), "the movie came from the hardware encoder exactly when the device has one");
    assert!(lowest_psnr(software, hardware).unwrap().1 >= D12_DB);
}

/// The choice is the `output`'s, not the call's: a `hardware` beside `output` is no field of the call.
#[test]
fn a_template_render_reads_its_hardware_choice_from_output_alone() {
    let scratch = Scratch::new().unwrap();
    let template = a_template(&scratch).unwrap();
    let mut app = scratch.app().unwrap();
    let params =
        json!({"template": template.display().to_string(), "hardware": true, "output": {"output": scratch.path().join("x.mp4").display().to_string()}});
    let refused = app.request_with_progress("template.render", params, &mut |_, _, _| true, &mut |_| Ok(())).unwrap_err();
    assert_eq!(refused.to_string(), "`template.render`: unknown parameter(s) `hardware`");
}

#[test]
fn a_template_render_whose_hardware_is_not_a_boolean_is_refused() {
    let scratch = Scratch::new().unwrap();
    let template = a_template(&scratch).unwrap();
    let mut app = scratch.app().unwrap();
    let request = json!({"output": scratch.path().join("x.mp4").display().to_string(), "hardware": "yes"});
    assert_eq!(render_template(&mut app, &template, request).unwrap_err().to_string(), "`export`: `hardware` must be a bool");
}
