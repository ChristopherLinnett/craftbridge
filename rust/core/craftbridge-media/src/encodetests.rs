//! The device's encoders' checks, which every platform with hardware encoders runs against its own codecs:
//! a movie encoded by the platform, read back with FilmCraft's own demuxer and software decoder, and the
//! sound the platform made of a tone. Safe test code only; a platform passes the function that starts its
//! encoder, so the same checks run over the production hardware path and over a codec a proof names (plan §18).

use std::io::Cursor;

use filmcraft_frame::PixelData;
use filmcraft_isobmff::{CodecConfig, ColorInfo, TrackKind};

use crate::aac::AacSpec;
use crate::encode::{ColourCodes, EncodeError, RateControl, VideoCodec, VideoSpec, Yuv420};
use crate::hardware::{HardwareEncoder, encode_aac};
use crate::movie::{AudioTrack, Backend, Movie};

pub(crate) type Start = fn(VideoSpec) -> Result<HardwareEncoder, EncodeError>;

pub(crate) const SIZE: u32 = 128;
pub(crate) const FRAMES: u64 = 30;

/// D-12's threshold (plan §28): every decoded picture within 40 dB of the one it should be.
const D12_DB: f64 = 40.0;

pub(crate) fn spec(codec: VideoCodec) -> VideoSpec {
    VideoSpec {
        codec,
        width: SIZE,
        height: SIZE,
        rate_num: 30,
        rate_den: 1,
        rate_control: RateControl::Bitrate { kbps: 2000 },
        keyframe_interval: 5,
        colour: ColourCodes { primaries: 1, transfer: 1, matrix: 1 },
    }
}

/// A moving pattern of a `width` by `height` picture, so the encoder has motion to predict and every picture
/// differs. Both chroma planes move too, each its own way: a layout that puts either plane, or either half of
/// an interleaved pair, anywhere but where the encoder reads it changes the picture that comes back.
pub(crate) fn picture_of(width: u32, height: u32, index: u64) -> Yuv420 {
    let plane = |width: usize, height: usize, shade: &dyn Fn(f64, f64) -> f64| -> Vec<u8> {
        (0..width * height).map(|at| shade((at % width) as f64 + index as f64 * 2.0, (at / width) as f64 + index as f64) as u8).collect()
    };
    let (w, h) = (width as usize, height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    Yuv420 {
        y: plane(w, h, &|x, y| 128.0 + 80.0 * (x / 9.0).sin() * (y / 7.0).cos()),
        u: plane(cw, ch, &|x, y| 128.0 + 40.0 * (x / 11.0).cos() + 20.0 * (y / 5.0).sin()),
        v: plane(cw, ch, &|x, y| 128.0 - 40.0 * (y / 13.0).sin() + 20.0 * (x / 6.0).cos()),
    }
}

pub(crate) fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let error: f64 = a.iter().zip(b).map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2)).sum::<f64>() / a.len() as f64;
    if error == 0.0 { f64::INFINITY } else { 10.0 * (255.0f64 * 255.0 / error).log10() }
}

/// The frames of a `spec`'s movie, fed out of order on purpose: the movie puts them back.
pub(crate) fn movie_of(spec: VideoSpec, start: Start, frames: u64, audio: Option<AudioTrack>) -> Option<Vec<u8>> {
    let encoder = start(spec).ok()?;
    let mut movie = Movie::new(encoder, Cursor::new(Vec::new()), spec, frames, audio).unwrap();
    for index in (0..frames).rev() {
        movie.push_frame(index, picture_of(spec.width, spec.height, index)).unwrap();
    }
    Some(movie.finish().unwrap().out.into_inner())
}

/// Every picture of a movie the platform encoded is there, timed as asked, with a picture decoding can start
/// at no further apart than the interval asked for, and each of its planes within D-12 of the picture that
/// went in: an encoder that garbles, drops or retimes pictures, or was handed them in a layout other than the
/// one it reads, fails here, not downstream.
pub(crate) fn check_movie(spec: VideoSpec, start: Start) {
    let codec = spec.codec;
    let Some(bytes) = movie_of(spec, start, FRAMES, None) else { return };
    let file = filmcraft_isobmff::open(bytes.clone()).unwrap();
    let track = file.tracks.iter().find(|track| track.kind == TrackKind::Video).unwrap();
    assert_eq!((track.timescale, track.samples.len()), (spec.rate_num, FRAMES as usize));
    assert!(track.samples.first().is_some_and(|sample| sample.is_sync));
    let keys: Vec<usize> = track.samples.iter().enumerate().filter(|(_, sample)| sample.is_sync).map(|(at, _)| at).collect();
    let apart = keys.iter().zip(keys.iter().skip(1)).map(|(a, b)| b - a).chain([FRAMES as usize - keys.last().copied().unwrap_or(0)]).max();
    assert!(apart <= Some(spec.keyframe_interval as usize), "{codec:?}: key pictures at {keys:?}, asked for every {}", spec.keyframe_interval);
    assert!(
        track.samples.iter().all(|sample| sample.duration == spec.rate_den && sample.pts == sample.dts),
        "no reordering: decoding order is presentation order"
    );
    let entry = track.entries.first().unwrap();
    // The stream's level holds what the movie needs, as the level an encoder is asked for or picks itself.
    let level = match (&entry.codec, codec) {
        (CodecConfig::Avc(record), VideoCodec::H264) => record.level,
        (CodecConfig::Hevc(record), VideoCodec::Hevc) => record.general_level_idc,
        (other, _) => panic!("{codec:?}: a sample entry of {other:?}"),
    };
    assert!(level >= spec.level().idc, "{codec:?}: level {level}, where the movie needs {}", spec.level().idc);
    assert_eq!(entry.video.as_ref().and_then(|video| video.color.clone()), Some(ColorInfo::Nclx { primaries: 1, transfer: 1, matrix: 1, full_range: false }));
    // The software decoder reads every picture, near what went in.
    let mut decoder = filmcraft_codecs::software_video_decoder(entry).unwrap();
    let mut decoded = Vec::new();
    for (at, sample) in track.samples.iter().enumerate() {
        let data = file.read_sample(&bytes, file.tracks.iter().position(|t| t.id == track.id).unwrap(), at).unwrap();
        decoded.extend(decoder.decode(&data, sample.pts).unwrap());
    }
    decoded.extend(decoder.flush());
    assert_eq!(decoded.len(), FRAMES as usize);
    for (index, frame) in decoded.iter().enumerate() {
        let PixelData::Yuv8 { planes, .. } = &frame.frame.data else { panic!("8-bit planes expected") };
        let fed = picture_of(spec.width, spec.height, index as u64);
        for (plane, (name, expected)) in planes.iter().zip([("luma", &fed.y), ("Cb", &fed.u), ("Cr", &fed.v)]) {
            assert_eq!(plane.len(), expected.len(), "{codec:?} picture {index}: the {name} plane's size");
            let quality = psnr(plane, expected);
            assert!(quality >= D12_DB, "{codec:?} picture {index}: {name} PSNR {quality}");
        }
    }
}

/// One second of a tone, 48 kHz stereo.
pub(crate) fn tone(seconds: usize) -> Vec<i16> {
    (0..48_000 * seconds)
        .flat_map(|n| {
            let sample = ((n as f64 * 440.0 * std::f64::consts::TAU / 48_000.0).sin() * 8000.0) as i16;
            [sample, sample]
        })
        .collect()
}

/// The sound a platform's encoder made of a second of tone: AAC-LC at 160 kbit/s a channel, as the movie
/// export spends it, with its priming before the first sample. `tolerance` is the run's own bit-rate slack:
/// a constant-rate converter lands within a percent of what it asked for, a codec asked in bits per second
/// lands nearer ten.
pub(crate) fn check_aac(track: &AudioTrack, tolerance: f64) {
    assert_eq!((track.config.clone(), track.channels, track.sample_rate), (vec![0x11, 0x90], 2, 48_000));
    assert!(track.priming > 0, "the encoder primes before the first sample");
    // The units hold the priming and the second of sound after it, and no unit beyond the one the sound's last
    // sample falls in: the same count for every encoder, whatever it makes past the sound's end.
    let reached = (usize::try_from(track.priming).unwrap() + 48_000).div_ceil(crate::movie::AAC_FRAME as usize);
    assert_eq!(track.units.len(), reached, "{} units for a priming of {}", track.units.len(), track.priming);
    assert!(track.units.iter().all(|unit| !unit.is_empty()));
    let seconds = track.units.len() as f64 * f64::from(crate::movie::AAC_FRAME) / 48_000.0;
    let rate = track.units.iter().map(Vec::len).sum::<usize>() as f64 * 8.0 / seconds;
    assert!((rate - 320_000.0).abs() < 320_000.0 * tolerance, "{rate:.0} bit/s");
}

/// Where the burst of [`burst`] first passes [`HEARD`]: its second sample.
const ONSET: usize = 4801;
/// A level a sample of the burst passes and the encoder's noise in its silence does not.
const HEARD: f32 = 0.0458;

/// A second of 48 kHz stereo, silent but for a tenth of a second of a 1 kHz tone from sample 4800.
fn burst() -> Vec<i16> {
    (0..48_000usize)
        .flat_map(|n| {
            let sample = if (4800..9600).contains(&n) { ((n as f64 * 1000.0 * std::f64::consts::TAU / 48_000.0).sin() * 12_000.0) as i16 } else { 0 };
            [sample, sample]
        })
        .collect()
}

/// A movie with both tracks: the sound the platform's AAC encoder made rides beside the pictures, with its
/// priming cut off by an edit that is the encoder's own, so the sound decodes with every sample where it went
/// in: an edit that skipped more or less than the encoder primed would put the sound early or late against
/// the pictures, and an encoder that kept the sound's end would show in its units.
pub(crate) fn check_movie_with_sound(start: Start) {
    let sound = encode_aac(AacSpec { sample_rate: 48_000, channels: 2 }, &burst()).unwrap();
    let units = sound.units.len();
    let Some(movie) = movie_of(spec(VideoCodec::H264), start, FRAMES, Some(sound)) else { return };
    let file = filmcraft_isobmff::open(movie.clone()).unwrap();
    let audio = file.tracks.iter().find(|track| track.kind == TrackKind::Audio).unwrap();
    assert_eq!((audio.timescale, audio.samples.len()), (48_000, units));
    assert!(audio.edits.first().is_some_and(|edit| edit.media_time > 0));
    let decoded = filmcraft_codecs::open_bytes("movie.mp4", movie.into()).unwrap().audio(0, 48_000, 48_000).unwrap();
    let left = decoded.channels.first().unwrap();
    let onset = left.iter().position(|sample| sample.abs() > HEARD).unwrap();
    assert!(onset.abs_diff(ONSET) <= 8, "the sound starts at sample {onset}, where it went in at {ONSET}");
}

/// A picture that is not one of the movie's is refused by the encoder where the encoder is asked for one:
/// a wrongly sized plane is no picture at all, and an encoder that encoded it would write a frame the
/// movie never asked for. The start may decline the codec (a platform with none), which skips the check.
pub(crate) fn wrong_size_is_refused(start: Start) {
    let Ok(mut encoder) = start(spec(VideoCodec::H264)) else { return };
    let wrong = Yuv420 { y: vec![0; 4], u: vec![0], v: vec![0] };
    assert_eq!(encoder.encode(0, &wrong).unwrap_err().to_string(), "the frame's planes are not those of a 128x128 picture");
}

#[test]
fn a_request_that_cannot_be_encoded_never_starts_a_session() {
    let odd = VideoSpec { width: 63, ..spec(VideoCodec::H264) };
    assert!(matches!(HardwareEncoder::start(odd), Err(EncodeError::Invalid(_))));
}

#[test]
fn sound_samples_that_are_not_whole_frames_are_refused() {
    assert!(matches!(encode_aac(AacSpec { sample_rate: 48_000, channels: 2 }, &[0, 0, 0]), Err(EncodeError::Invalid(_))));
    assert!(matches!(encode_aac(AacSpec { sample_rate: 45_000, channels: 2 }, &[0, 0]), Err(EncodeError::Invalid(_))));
}
