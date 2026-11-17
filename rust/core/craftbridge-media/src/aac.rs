//! What the platform's AAC encoder is asked for: the stream's description as AAC-LC, its bit rate and the
//! two-byte AudioSpecificConfig an MP4 carries for it.
//!
//! Safe code, so every target tests it; the platform module runs the converter these settings describe.

use crate::encode::EncodeError;
#[cfg(any(target_vendor = "apple", test))]
use crate::movie::{AAC_FRAME, AudioTrack};

/// The sample rates an AudioSpecificConfig names by index, in the index order (ISO/IEC 14496-3, table 1.18).
const RATES: [u32; 13] = [96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000, 7_350];

/// The AAC-LC audio object type.
const OBJECT_LC: u16 = 2;

/// Bits a second the encoder spends on each channel, at a constant rate, as EffectCraft's AAC encoder is set up.
#[cfg(any(target_vendor = "apple", test))]
const BITS_PER_CHANNEL: u32 = 160_000;

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
    #[cfg(any(target_vendor = "apple", test))]
    pub(crate) fn bit_rate(&self) -> u32 {
        BITS_PER_CHANNEL.saturating_mul(self.channels)
    }

    /// The track the encoder's access units make of `frames` frames of sound, once it has produced them. The
    /// units must hold the priming and every frame after it: an encoder that stopped early, however it said so,
    /// would otherwise leave the movie's sound short without a word.
    #[cfg(any(target_vendor = "apple", test))]
    pub(crate) fn track(&self, priming: i64, units: Vec<Vec<u8>>, frames: usize) -> Result<AudioTrack, EncodeError> {
        let config = self.config().ok_or_else(|| EncodeError::Invalid(format!("a sample rate of {} Hz has no AAC-LC configuration", self.sample_rate)))?;
        let held = (units.len() as u128).saturating_mul(u128::from(AAC_FRAME));
        let needed = u128::try_from(priming).unwrap_or(0).saturating_add(frames as u128);
        if frames > 0 && held < needed {
            return Err(EncodeError::Failed(format!(
                "the AAC encoder returned {} units of {AAC_FRAME} samples, which do not hold its priming of {priming} and the sound's {frames}",
                units.len()
            )));
        }
        Ok(AudioTrack { config, channels: self.channels, sample_rate: self.sample_rate, priming, units })
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
}
