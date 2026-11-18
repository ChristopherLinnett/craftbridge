//! The platform's hardware encoders behind one name each, so a caller writes no `cfg`: a build whose platform
//! has none answers [`EncodeError::Unavailable`], which a caller meets by encoding another way.

use crate::aac::AacSpec;
use crate::encode::{EncodeError, VideoSpec, Yuv420};
use crate::movie::{AudioTrack, Backend, Packet};

#[cfg(target_vendor = "apple")]
type Inner = crate::platform::videotoolbox_encoder::Encoder;
/// The device's own MediaCodec encoders, chosen by the device's hardware answer.
#[cfg(target_os = "android")]
type Inner = crate::platform::mediacodec_encoder::Encoder;
/// No platform here has an encoder, so no value of this type exists.
#[cfg(not(any(target_vendor = "apple", target_os = "android")))]
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
        #[cfg(target_os = "android")]
        {
            crate::platform::mediacodec_encoder::Encoder::start(spec).map(Self)
        }
        #[cfg(not(any(target_vendor = "apple", target_os = "android")))]
        {
            spec.check()?;
            Err(EncodeError::Unavailable("this build has no hardware video encoder".into()))
        }
    }

    /// An encoder the platform module started over a codec its own proof named, under the caller's name:
    /// the device tests walk the whole path over a codec the device has, which is not what production
    /// chooses on a device that has hardware ones (plan §18).
    #[cfg(all(test, target_os = "android"))]
    pub(crate) fn started_over(encoder: Inner) -> Self {
        Self(encoder)
    }
}

impl Backend for HardwareEncoder {
    fn encode(&mut self, index: u64, frame: &Yuv420) -> Result<Vec<Packet>, EncodeError> {
        #[cfg(any(target_vendor = "apple", target_os = "android"))]
        {
            self.0.encode(index, frame)
        }
        #[cfg(not(any(target_vendor = "apple", target_os = "android")))]
        {
            let _ = (index, frame);
            match self.0 {}
        }
    }

    fn finish(&mut self) -> Result<Vec<Packet>, EncodeError> {
        #[cfg(any(target_vendor = "apple", target_os = "android"))]
        {
            self.0.finish()
        }
        #[cfg(not(any(target_vendor = "apple", target_os = "android")))]
        {
            match self.0 {}
        }
    }

    fn record(&self) -> Option<Vec<u8>> {
        #[cfg(any(target_vendor = "apple", target_os = "android"))]
        {
            self.0.record()
        }
        #[cfg(not(any(target_vendor = "apple", target_os = "android")))]
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
    #[cfg(target_os = "android")]
    {
        crate::platform::mediacodec_aac::encode(spec, samples)
    }
    #[cfg(not(any(target_vendor = "apple", target_os = "android")))]
    {
        let _ = samples;
        spec.check()?;
        Err(EncodeError::Unavailable("this build has no AAC encoder".into()))
    }
}

/// These run the framework on a Mac, which has the same VideoToolbox and AudioToolbox a phone has: the
/// encoder, the AAC converter and the movie they make. The checks are the device encoders' own
/// ([`crate::encodetests`]), shared with the Android device suite; here the platform only names its encoder
/// and what its converter spends.
#[cfg(test)]
#[cfg(target_vendor = "apple")]
mod tests {
    use super::*;
    use crate::encode::{ColourCodes, RateControl, VideoCodec};
    use crate::encodetests as device;
    use filmcraft_isobmff::ColorInfo;

    /// A Mac has a hardware encoder for each codec; the iOS Simulator has what its host lends it, so a
    /// codec it declines is left out of its run.
    fn hardware(spec: VideoSpec) -> Result<HardwareEncoder, EncodeError> {
        match HardwareEncoder::start(spec) {
            started @ Ok(_) => started,
            Err(error) if cfg!(target_os = "ios") => {
                assert!(matches!(error, EncodeError::Unavailable(_)), "{error}");
                Err(error)
            }
            Err(error) => panic!("this Mac has a hardware encoder: {error}"),
        }
    }

    #[test]
    fn an_h264_movie_the_hardware_made_decodes_to_the_pictures_that_went_in() {
        device::check_movie(device::spec(VideoCodec::H264), hardware);
    }

    #[test]
    fn an_hevc_movie_the_hardware_made_decodes_to_the_pictures_that_went_in() {
        device::check_movie(device::spec(VideoCodec::Hevc), hardware);
    }

    #[test]
    fn a_movie_at_a_fractional_rate_is_timed_in_the_rates_own_ticks() {
        // 30000/1001: a tick is no longer a frame, so the time each picture is fed with and the time the encoder
        // stamps it with are the rate's denominator apart, which the movie holds to.
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            device::check_movie(VideoSpec { rate_num: 30000, rate_den: 1001, ..device::spec(codec) }, hardware);
        }
    }

    #[test]
    fn a_constant_quality_hevc_movie_is_made_and_a_wide_gamut_one_carries_its_tags() {
        let mut spec = device::spec(VideoCodec::Hevc);
        spec.rate_control = RateControl::Quality { percent: 70 };
        spec.colour = ColourCodes { primaries: 12, transfer: 13, matrix: 1 };
        let Some(bytes) = device::movie_of(spec, hardware, 3, None) else { return };
        let file = filmcraft_isobmff::open(bytes).unwrap();
        let entry = file.tracks.first().and_then(|track| track.entries.first()).unwrap();
        assert_eq!(
            entry.video.as_ref().and_then(|video| video.color.clone()),
            Some(ColorInfo::Nclx { primaries: 12, transfer: 13, matrix: 1, full_range: false })
        );
    }

    #[test]
    fn a_picture_of_the_wrong_size_is_refused_by_the_encoder() {
        device::wrong_size_is_refused(hardware);
    }

    #[test]
    fn the_platform_encodes_sound_as_aac_lc_with_its_priming() {
        let track = encode_aac(AacSpec { sample_rate: 48_000, channels: 2 }, &device::tone(1)).unwrap();
        // A constant 160 kbit/s a channel, as EffectCraft's encoder spends: the platform's own mode would
        // spend about half that on a tone, and a constant-rate converter lands within a percent of what it asked.
        device::check_aac(&track, 0.01);
    }

    #[test]
    fn a_movie_with_sound_has_both_tracks_and_the_sound_decodes_for_as_long_as_the_picture() {
        device::check_movie_with_sound(hardware);
    }
}
