//! What the platform's AAC encoder is asked for: the stream's description as AAC-LC, its bit rate and the
//! two-byte AudioSpecificConfig an MP4 carries for it.
//!
//! Safe code, so every target tests it; the platform module runs the converter these settings describe.

use crate::encode::EncodeError;
#[cfg(any(target_vendor = "apple", target_os = "android", test))]
use crate::movie::{AAC_FRAME, AudioTrack};

/// The sample rates an AudioSpecificConfig names by index, in the index order (ISO/IEC 14496-3, table 1.18).
const RATES: [u32; 13] = [96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000, 7_350];

/// The AAC-LC audio object type.
const OBJECT_LC: u16 = 2;

/// Bits a second the encoder spends on each channel, at a constant rate, as EffectCraft's AAC encoder is set up.
#[cfg(any(target_vendor = "apple", target_os = "android", test))]
const BITS_PER_CHANNEL: u32 = 160_000;

/// The samples an Android AAC encoder primes before the sound, by its name: two frames, 2048 samples, for the
/// Fraunhofer encoder the platform carries (`c2.android.aac.encoder`, and `OMX.google.aac.encoder` before
/// Codec 2): a burst it encodes decodes exactly that much late, and the movie's own check of where its sound
/// lands holds the number on every device the suite runs on. MediaCodec reports no encoder's priming (its
/// output format names none, and `KEY_ENCODER_DELAY` describes a decoded stream), so another encoder primes
/// an amount nothing here can know, and its sound could not be lined up with the pictures: `None`.
#[cfg(any(target_os = "android", test))]
pub(crate) fn android_priming(encoder: &str) -> Option<u32> {
    matches!(encoder, "c2.android.aac.encoder" | "OMX.google.aac.encoder").then_some(2 * AAC_FRAME)
}

/// What to encode: interleaved 16-bit samples of one or two channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AacSpec {
    pub sample_rate: u32,
    pub channels: u32,
}

impl AacSpec {
    /// The request as AAC-LC can take it: mono or stereo at a rate the configuration can name.
    pub fn check(&self) -> Result<(), EncodeError> {
        if !(1..=2).contains(&self.channels) {
            return Err(EncodeError::Invalid(format!("{} channels of sound cannot be encoded as AAC here, only one or two", self.channels)));
        }
        if self.config().is_none() {
            return Err(EncodeError::Invalid(format!("a sample rate of {} Hz has no AAC-LC configuration", self.sample_rate)));
        }
        Ok(())
    }

    /// The AudioSpecificConfig: five bits of object type, four of rate index, four of channel
    /// configuration and three zero bits; `None` for a rate without an index.
    pub(crate) fn config(&self) -> Option<Vec<u8>> {
        let index = RATES.iter().position(|&rate| rate == self.sample_rate)?;
        let bits = (OBJECT_LC << 11) | (u16::try_from(index).ok()? << 7) | (u16::try_from(self.channels).ok()? << 3);
        Some(bits.to_be_bytes().to_vec())
    }

    /// The encoder's constant rate, bits a second over all channels.
    #[cfg(any(target_vendor = "apple", target_os = "android", test))]
    pub(crate) fn bit_rate(&self) -> u32 {
        BITS_PER_CHANNEL.saturating_mul(self.channels)
    }

    /// The track the encoder's access units make of `frames` frames of sound, once it has produced them. The
    /// units must hold the priming and every frame after it: an encoder that stopped early, however it said so,
    /// would otherwise leave the movie's sound short without a word. The units after the last one the sound
    /// reaches are left out: they hold only what the encoder made past the sound's end, and an encoder that
    /// drains its own priming at the end of the stream as well (Android 17's platform encoder gives two more
    /// units than it was fed frames) would otherwise run the movie's sound on in silence past the pictures.
    #[cfg(any(target_vendor = "apple", target_os = "android", test))]
    pub(crate) fn track(&self, priming: i64, mut units: Vec<Vec<u8>>, frames: usize) -> Result<AudioTrack, EncodeError> {
        let config = self.config().ok_or_else(|| EncodeError::Invalid(format!("a sample rate of {} Hz has no AAC-LC configuration", self.sample_rate)))?;
        let held = (units.len() as u128).saturating_mul(u128::from(AAC_FRAME));
        let needed = u128::try_from(priming).unwrap_or(0).saturating_add(frames as u128);
        if frames > 0 && held < needed {
            return Err(EncodeError::Failed(format!(
                "the AAC encoder returned {} units of {AAC_FRAME} samples, which do not hold its priming of {priming} and the sound's {frames}",
                units.len()
            )));
        }
        if frames > 0 {
            units.truncate(usize::try_from(needed.div_ceil(u128::from(AAC_FRAME))).unwrap_or(usize::MAX));
        }
        Ok(AudioTrack { config, channels: self.channels, sample_rate: self.sample_rate, priming, units, frames })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_of_the_usual_rates_is_the_two_bytes_every_encoder_writes() {
        // 44.1 kHz stereo is 0x12 0x10 and 48 kHz stereo 0x11 0x90, the configurations seen in every AAC-LC file.
        assert_eq!(AacSpec { sample_rate: 44_100, channels: 2 }.config(), Some(vec![0x12, 0x10]));
        assert_eq!(AacSpec { sample_rate: 48_000, channels: 2 }.config(), Some(vec![0x11, 0x90]));
        assert_eq!(AacSpec { sample_rate: 44_100, channels: 1 }.config(), Some(vec![0x12, 0x08]));
        assert_eq!(AacSpec { sample_rate: 8_000, channels: 1 }.config(), Some(vec![0x15, 0x88]));
    }

    #[test]
    fn a_rate_without_an_index_has_no_config() {
        assert_eq!(AacSpec { sample_rate: 45_000, channels: 2 }.config(), None);
        assert_eq!(AacSpec { sample_rate: 45_000, channels: 2 }.check().unwrap_err().to_string(), "a sample rate of 45000 Hz has no AAC-LC configuration");
    }

    #[test]
    fn only_mono_and_stereo_are_encoded() {
        for channels in [0, 3, 6] {
            let error = AacSpec { sample_rate: 48_000, channels }.check().unwrap_err().to_string();
            assert_eq!(error, format!("{channels} channels of sound cannot be encoded as AAC here, only one or two"));
        }
        assert!(AacSpec { sample_rate: 48_000, channels: 1 }.check().is_ok());
        assert!(AacSpec { sample_rate: 48_000, channels: 2 }.check().is_ok());
    }

    #[test]
    fn the_target_is_a_hundred_and_sixty_kilobits_for_each_channel() {
        assert_eq!(AacSpec { sample_rate: 48_000, channels: 2 }.bit_rate(), 320_000);
        assert_eq!(AacSpec { sample_rate: 48_000, channels: 1 }.bit_rate(), 160_000);
    }

    #[test]
    fn the_platforms_own_aac_encoder_primes_two_frames_and_another_an_amount_unknown() {
        assert_eq!(android_priming("c2.android.aac.encoder"), Some(2048));
        assert_eq!(android_priming("OMX.google.aac.encoder"), Some(2048));
        assert_eq!(android_priming("c2.android.xheaac.encoder"), None);
        assert_eq!(android_priming("c2.vendor.aac.encoder"), None);
    }

    #[test]
    fn the_track_carries_the_spec_the_priming_and_the_units() {
        let track = AacSpec { sample_rate: 48_000, channels: 2 }.track(2112, vec![vec![1]; 4], 1000).unwrap();
        assert_eq!((track.config, track.channels, track.sample_rate, track.priming, track.units), (vec![0x11, 0x90], 2, 48_000, 2112, vec![vec![1]; 4]));
    }

    #[test]
    fn a_track_whose_units_stop_before_the_sound_ends_is_refused() {
        let spec = AacSpec { sample_rate: 48_000, channels: 2 };
        // A second at 48 kHz after 2112 samples of priming is 50112 samples: 49 units hold them, 48 do not.
        assert_eq!(spec.track(2112, vec![vec![1]; 49], 48_000).map(|track| track.units.len()), Ok(49));
        assert_eq!(
            spec.track(2112, vec![vec![1]; 48], 48_000).unwrap_err().to_string(),
            "the AAC encoder returned 48 units of 1024 samples, which do not hold its priming of 2112 and the sound's 48000"
        );
        assert_eq!(
            spec.track(2112, Vec::new(), 1).unwrap_err().to_string(),
            "the AAC encoder returned 0 units of 1024 samples, which do not hold its priming of 2112 and the sound's 1"
        );
        assert!(spec.track(2112, Vec::new(), 0).is_ok(), "no sound needs no unit");
    }

    #[test]
    fn a_track_keeps_the_units_the_sound_reaches_and_none_after() {
        let spec = AacSpec { sample_rate: 48_000, channels: 2 };
        // 2048 samples of priming and a second of sound reach into unit 49 (50048 samples): the two an encoder
        // drains past the end are no part of the sound.
        let units: Vec<Vec<u8>> = (0..51u8).map(|n| vec![n]).collect();
        let track = spec.track(2048, units.clone(), 48_000).unwrap();
        assert_eq!(track.units, units.get(..49).unwrap().to_vec());
        assert_eq!(spec.track(2048, units.get(..49).unwrap().to_vec(), 48_000).unwrap().units.len(), 49, "a track with no more than it needs is kept whole");
    }
}
