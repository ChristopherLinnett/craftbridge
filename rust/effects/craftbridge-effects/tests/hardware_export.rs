//! The acceptance of plan §28 T9 and T10 with the hardware option, and of "every other format unchanged":
//! an `export` that chose hardware encoding writes a movie of the same frame count and size whose every
//! decoded frame is within D-12's 40 dB of the software export's, and any job the option does not apply to
//! writes exactly what it writes without it. A device without the encoder declines the option, which the
//! same tests hold to the same bytes as the software export.
//!
//! Runs where VideoToolbox does: on a Mac and in the iOS Simulator. The helpers answer `Result` and leave
//! the unwrapping to the `#[test]` functions, as everywhere else in this repository.

#![cfg(target_vendor = "apple")]

mod scratch;

use std::path::{Path, PathBuf};

use craftbridge_core::{CraftApp, CraftError};
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

fn export(app: &mut EffectCraftApp, request: Value) -> Result<Value, CraftError> {
    app.request_with_progress("export", request, &mut |_, _, _| true, &mut |_| Ok(()))
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
}

#[test]
fn a_movie_the_hardware_does_not_reproduce_stays_with_effectcrafts_encoders() {
    // HEVC at Main 10 and at a chosen level are what the hardware encoder does not take, so the job is the
    // software one.
    for (key, value) in [("profile", "main10"), ("level", "4.1")] {
        let (plain, option) = both_ways(|folder| with(small("hevc", &folder.join("out.mp4")), key, json!(value))).unwrap();
        assert!(!plain.is_empty());
        assert_eq!(plain, option, "{key} {value}");
    }
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
    let request = with(with(small("h264", &output), "hardware", json!(true)), "end", json!(2.0));
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
