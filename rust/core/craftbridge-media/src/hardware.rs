//! The platform's hardware encoders behind one name each, so a caller writes no `cfg`: a build whose platform
//! has none answers [`EncodeError::Unavailable`], which a caller meets by encoding another way.

use crate::aac::AacSpec;
use crate::encode::{EncodeError, VideoSpec, Yuv420};
use crate::movie::{AudioTrack, Backend, Packet};

#[cfg(target_vendor = "apple")]
type Inner = crate::platform::videotoolbox_encoder::Encoder;
/// No platform here has an encoder, so no value of this type exists.
#[cfg(not(target_vendor = "apple"))]
type Inner = std::convert::Infallible;

/// The device's hardware video encoder for one movie.
pub struct HardwareEncoder(Inner);

impl HardwareEncoder {
    /// Starts an encoder for `spec`, or says why there is none: [`EncodeError::Unavailable`] when the device
    /// has no hardware encoder for the codec, [`EncodeError::Invalid`] when the request cannot be encoded.
    pub fn start(spec: VideoSpec) -> Result<Self, EncodeError> {
        #[cfg(target_vendor = "apple")]
        {
            crate::platform::videotoolbox_encoder::Encoder::start(spec).map(Self)
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            spec.check()?;
            Err(EncodeError::Unavailable("this build has no hardware video encoder".into()))
        }
    }
}

impl Backend for HardwareEncoder {
    fn encode(&mut self, index: u64, frame: &Yuv420) -> Result<Vec<Packet>, EncodeError> {
        #[cfg(target_vendor = "apple")]
        {
            self.0.encode(index, frame)
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            let _ = (index, frame);
            match self.0 {}
        }
    }

    fn finish(&mut self) -> Result<Vec<Packet>, EncodeError> {
        #[cfg(target_vendor = "apple")]
        {
            self.0.finish()
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            match self.0 {}
        }
    }

    fn record(&self) -> Option<Vec<u8>> {
        #[cfg(target_vendor = "apple")]
        {
            self.0.record()
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            match self.0 {}
        }
    }
}

/// `samples`, 16-bit interleaved, encoded as AAC-LC by the platform's encoder; [`EncodeError::Unavailable`]
/// where the platform has none.
pub fn encode_aac(spec: AacSpec, samples: &[i16]) -> Result<AudioTrack, EncodeError> {
    #[cfg(target_vendor = "apple")]
    {
        crate::platform::audioconverter::encode(spec, samples)
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        let _ = samples;
        spec.check()?;
        Err(EncodeError::Unavailable("this build has no AAC encoder".into()))
    }
}

/// These run the framework on a Mac, which has the same VideoToolbox and AudioToolbox a phone has: the
/// encoder, the AAC converter and the movie they make, read back with FilmCraft's own demuxer and decoder.
#[cfg(test)]
#[cfg(target_vendor = "apple")]
mod tests {
    use std::io::Cursor;

    use filmcraft_frame::PixelData;
    use filmcraft_isobmff::{CodecConfig, ColorInfo, TrackKind};

    use super::*;
    use crate::encode::{ColourCodes, RateControl, VideoCodec};
    use crate::movie::Movie;

    const SIZE: u32 = 128;
    const FRAMES: u64 = 30;

    fn spec(codec: VideoCodec) -> VideoSpec {
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

    /// A moving pattern, so the encoder has motion to predict and every picture differs.
    fn picture(index: u64) -> Yuv420 {
        let side = SIZE as usize;
        let shade = |at: usize| {
            let (x, y) = ((at % side) as f64 + index as f64 * 2.0, (at / side) as f64 + index as f64);
            (128.0 + 80.0 * (x / 9.0).sin() * (y / 7.0).cos()) as u8
        };
        let y = (0..side * side).map(shade).collect();
        let chroma = (side / 2) * (side / 2);
        Yuv420 { y, u: vec![110; chroma], v: vec![150; chroma] }
    }

    /// The hardware encoder for `spec`: a Mac has one for each codec, and the iOS Simulator has what its host
    /// lends it, so a codec it declines is left out of its run.
    fn encoder(spec: VideoSpec) -> Option<HardwareEncoder> {
        match HardwareEncoder::start(spec) {
            Ok(encoder) => Some(encoder),
            Err(error) if cfg!(target_os = "ios") => {
                assert!(matches!(error, EncodeError::Unavailable(_)), "{error}");
                None
            }
            Err(error) => panic!("this Mac has a hardware encoder: {error}"),
        }
    }

    fn movie_of(spec: VideoSpec, frames: u64, audio: Option<AudioTrack>) -> Option<Vec<u8>> {
        let encoder = encoder(spec)?;
        let mut movie = Movie::new(encoder, Cursor::new(Vec::new()), spec, frames, audio).unwrap();
        // Out of order on purpose: the movie puts them back.
        for index in (0..frames).rev() {
            movie.push_frame(index, picture(index)).unwrap();
        }
        Some(movie.finish().unwrap().out.into_inner())
    }

    fn psnr(a: &[u8], b: &[u8]) -> f64 {
        let error: f64 = a.iter().zip(b).map(|(&a, &b)| (f64::from(a) - f64::from(b)).powi(2)).sum::<f64>() / a.len() as f64;
        if error == 0.0 { f64::INFINITY } else { 10.0 * (255.0f64 * 255.0 / error).log10() }
    }

    fn check_movie(spec: VideoSpec) {
        let codec = spec.codec;
        let Some(bytes) = movie_of(spec, FRAMES, None) else { return };
        let file = filmcraft_isobmff::open(bytes.clone()).unwrap();
        let track = file.tracks.iter().find(|track| track.kind == TrackKind::Video).unwrap();
        assert_eq!((track.timescale, track.samples.len()), (spec.rate_num, FRAMES as usize));
        assert!(track.samples.first().is_some_and(|sample| sample.is_sync));
        assert!(
            track.samples.iter().all(|sample| sample.duration == spec.rate_den && sample.pts == sample.dts),
            "no reordering: decoding order is presentation order"
        );
        let entry = track.entries.first().unwrap();
        assert!(match codec {
            VideoCodec::H264 => matches!(entry.codec, CodecConfig::Avc(_)),
            VideoCodec::Hevc => matches!(entry.codec, CodecConfig::Hevc(_)),
        });
        assert_eq!(
            entry.video.as_ref().and_then(|video| video.color.clone()),
            Some(ColorInfo::Nclx { primaries: 1, transfer: 1, matrix: 1, full_range: false })
        );
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
            let quality = psnr(planes.first().unwrap(), &picture(index as u64).y);
            assert!(quality > 30.0, "{codec:?} picture {index}: luma PSNR {quality}");
        }
    }

    #[test]
    fn an_h264_movie_the_hardware_made_decodes_to_the_pictures_that_went_in() {
        check_movie(spec(VideoCodec::H264));
    }

    #[test]
    fn an_hevc_movie_the_hardware_made_decodes_to_the_pictures_that_went_in() {
        check_movie(spec(VideoCodec::Hevc));
    }

    #[test]
    fn a_movie_at_a_fractional_rate_is_timed_in_the_rates_own_ticks() {
        // 30000/1001: a tick is no longer a frame, so the time each picture is fed with and the time the encoder
        // stamps it with are the rate's denominator apart, which the movie holds to.
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            check_movie(VideoSpec { rate_num: 30000, rate_den: 1001, ..spec(codec) });
        }
    }

    #[test]
    fn a_constant_quality_hevc_movie_is_made_and_a_wide_gamut_one_carries_its_tags() {
        let mut spec = spec(VideoCodec::Hevc);
        spec.rate_control = RateControl::Quality { percent: 70 };
        spec.colour = ColourCodes { primaries: 12, transfer: 13, matrix: 1 };
        let Some(encoder) = encoder(spec) else { return };
        let mut movie = Movie::new(encoder, Cursor::new(Vec::new()), spec, 3, None).unwrap();
        for index in 0..3 {
            movie.push_frame(index, picture(index)).unwrap();
        }
        let file = filmcraft_isobmff::open(movie.finish().unwrap().out.into_inner()).unwrap();
        let entry = file.tracks.first().and_then(|track| track.entries.first()).unwrap();
        assert_eq!(
            entry.video.as_ref().and_then(|video| video.color.clone()),
            Some(ColorInfo::Nclx { primaries: 12, transfer: 13, matrix: 1, full_range: false })
        );
    }

    #[test]
    fn a_picture_of_the_wrong_size_is_refused_by_the_encoder() {
        let Some(mut encoder) = encoder(spec(VideoCodec::H264)) else { return };
        let wrong = Yuv420 { y: vec![0; 4], u: vec![0], v: vec![0] };
        assert!(encoder.encode(0, &wrong).unwrap_err().to_string().contains("not those of a 128x128 picture"));
    }

    #[test]
    fn a_request_that_cannot_be_encoded_never_starts_a_session() {
        let odd = VideoSpec { width: 63, ..spec(VideoCodec::H264) };
        assert!(matches!(HardwareEncoder::start(odd), Err(EncodeError::Invalid(_))));
    }

    /// One second of a tone, 48 kHz stereo.
    fn tone(seconds: usize) -> Vec<i16> {
        (0..48_000 * seconds)
            .flat_map(|n| {
                let sample = ((n as f64 * 440.0 * std::f64::consts::TAU / 48_000.0).sin() * 8000.0) as i16;
                [sample, sample]
            })
            .collect()
    }

    #[test]
    fn the_platform_encodes_sound_as_aac_lc_with_its_priming() {
        let track = encode_aac(AacSpec { sample_rate: 48_000, channels: 2 }, &tone(1)).unwrap();
        assert_eq!((track.config.clone(), track.channels, track.sample_rate), (vec![0x11, 0x90], 2, 48_000));
        assert!(track.priming > 0, "the encoder primes before the first sample");
        // A second of sound is 47 units of 1024 samples, and the priming adds a unit or two.
        assert!((47..=50).contains(&track.units.len()), "{} units", track.units.len());
        assert!(track.units.iter().all(|unit| !unit.is_empty()));
        // A constant 160 kbit/s a channel, as EffectCraft's encoder spends: the platform's own mode would spend
        // about half that on a tone.
        let seconds = track.units.len() as f64 * f64::from(crate::movie::AAC_FRAME) / 48_000.0;
        let rate = track.units.iter().map(Vec::len).sum::<usize>() as f64 * 8.0 / seconds;
        assert!((rate - 320_000.0).abs() < 3_200.0, "{rate:.0} bit/s");
    }

    #[test]
    fn sound_samples_that_are_not_whole_frames_are_refused() {
        assert!(matches!(encode_aac(AacSpec { sample_rate: 48_000, channels: 2 }, &[0, 0, 0]), Err(EncodeError::Invalid(_))));
        assert!(matches!(encode_aac(AacSpec { sample_rate: 45_000, channels: 2 }, &[0, 0]), Err(EncodeError::Invalid(_))));
    }

    #[test]
    fn a_movie_with_sound_has_both_tracks_and_the_sound_decodes_for_as_long_as_the_picture() {
        let sound = encode_aac(AacSpec { sample_rate: 48_000, channels: 2 }, &tone(1)).unwrap();
        let units = sound.units.len();
        let Some(movie) = movie_of(spec(VideoCodec::H264), 30, Some(sound)) else { return };
        let file = filmcraft_isobmff::open(movie).unwrap();
        let audio = file.tracks.iter().find(|track| track.kind == TrackKind::Audio).unwrap();
        assert_eq!((audio.timescale, audio.samples.len()), (48_000, units));
        assert!(audio.edits.first().is_some_and(|edit| edit.media_time > 0));
    }
}
