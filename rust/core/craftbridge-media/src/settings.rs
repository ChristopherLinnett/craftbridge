//! The session settings a hardware encoder's request becomes, as numbers and names the platform module sets
//! one by one: the bit rate with its ceiling or the constant quality, the key frame interval, the profile and
//! the colour tags. Safe code, so a host tests every rule; only a platform with a hardware encoder reads it.

use crate::encode::{ColourCodes, RateControl, VideoCodec, VideoSpec};

/// A primaries, transfer or matrix the encoder can be told by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Primaries {
    Bt709,
    Bt2020,
    P3D65,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transfer {
    Bt709,
    Srgb,
    Pq,
    Hlg,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Matrix {
    Bt709,
}

/// The colour tags the encoder writes into the bitstream: each is `None` where the code point has no name
/// the framework knows, which leaves that tag unspecified there. The container's own tag always carries the
/// code points as they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Tags {
    pub(crate) primaries: Option<Primaries>,
    pub(crate) transfer: Option<Transfer>,
    pub(crate) matrix: Option<Matrix>,
}

impl ColourCodes {
    pub(crate) fn tags(self) -> Tags {
        Tags {
            primaries: match self.primaries {
                1 => Some(Primaries::Bt709),
                9 => Some(Primaries::Bt2020),
                12 => Some(Primaries::P3D65),
                _ => None,
            },
            transfer: match self.transfer {
                1 => Some(Transfer::Bt709),
                13 => Some(Transfer::Srgb),
                16 => Some(Transfer::Pq),
                18 => Some(Transfer::Hlg),
                8 => Some(Transfer::Linear),
                _ => None,
            },
            matrix: (self.matrix == 1).then_some(Matrix::Bt709),
        }
    }
}

/// The profile asked of the encoder: High for H.264 as EffectCraft's own encoder writes, Main for HEVC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Profile {
    H264High,
    HevcMain,
}

/// The session's properties as numbers, which the platform module sets one by one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Settings {
    /// Bits a second the encoder aims at.
    pub(crate) average_bit_rate: Option<i64>,
    /// The most bytes in a window of one second: the rate's own ceiling, one and a half times the target,
    /// as EffectCraft's H.264 variable rate has it.
    pub(crate) peak_bytes_per_second: Option<i64>,
    /// The encoder's quality scale, 0.01 to 1.
    pub(crate) quality: Option<f32>,
    pub(crate) max_key_frame_interval: i32,
    pub(crate) frame_rate: f64,
    pub(crate) profile: Profile,
    pub(crate) tags: Tags,
}

/// The lowest bit rate EffectCraft's encoders take for each codec, in kbit/s.
const MIN_H264_KBPS: u32 = 100;
const MIN_HEVC_KBPS: u32 = 50;

impl VideoSpec {
    pub(crate) fn settings(&self) -> Settings {
        let (average_bit_rate, peak_bytes_per_second, quality) = match self.rate_control {
            RateControl::Bitrate { kbps } => {
                let kbps = i64::from(kbps.max(match self.codec {
                    VideoCodec::H264 => MIN_H264_KBPS,
                    VideoCodec::Hevc => MIN_HEVC_KBPS,
                }));
                (Some(kbps * 1000), Some(kbps * 1000 * 3 / 2 / 8), None)
            }
            RateControl::Quality { percent } => (None, None, Some(f32::from(percent.clamp(1, 100)) / 100.0)),
        };
        Settings {
            average_bit_rate,
            peak_bytes_per_second,
            quality,
            max_key_frame_interval: i32::try_from(self.keyframe_interval).unwrap_or(i32::MAX),
            frame_rate: f64::from(self.rate_num) / f64::from(self.rate_den),
            profile: match self.codec {
                VideoCodec::H264 => Profile::H264High,
                VideoCodec::Hevc => Profile::HevcMain,
            },
            tags: self.colour.tags(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::tests::spec;
    use crate::encode::{RateControl, VideoCodec, VideoSpec};

    #[test]
    fn a_bit_rate_has_the_floor_of_its_codec_and_a_ceiling_of_one_and_a_half_times() {
        let settings = |codec, kbps| VideoSpec { codec, rate_control: RateControl::Bitrate { kbps }, ..spec() }.settings();
        let h264 = settings(VideoCodec::H264, 10);
        assert_eq!((h264.average_bit_rate, h264.peak_bytes_per_second, h264.quality), (Some(100_000), Some(18_750), None));
        let hevc = settings(VideoCodec::Hevc, 10);
        assert_eq!(hevc.average_bit_rate, Some(50_000));
        assert_eq!(settings(VideoCodec::H264, 8000).average_bit_rate, Some(8_000_000));
        assert_eq!(settings(VideoCodec::H264, 8000).peak_bytes_per_second, Some(1_500_000));
    }

    #[test]
    fn a_constant_quality_is_a_fraction_between_one_hundredth_and_one() {
        let quality = |percent| VideoSpec { codec: VideoCodec::Hevc, rate_control: RateControl::Quality { percent }, ..spec() }.settings();
        assert_eq!(quality(70).quality, Some(0.7));
        assert_eq!(quality(0).quality, Some(0.01));
        assert_eq!(quality(200).quality, Some(1.0));
        assert_eq!((quality(70).average_bit_rate, quality(70).peak_bytes_per_second), (None, None));
    }

    #[test]
    fn the_profile_and_key_frame_interval_follow_the_request() {
        assert_eq!(spec().settings().profile, Profile::H264High);
        assert_eq!(VideoSpec { codec: VideoCodec::Hevc, ..spec() }.settings().profile, Profile::HevcMain);
        assert_eq!(spec().settings().max_key_frame_interval, 60);
        assert_eq!(VideoSpec { keyframe_interval: u32::MAX, ..spec() }.settings().max_key_frame_interval, i32::MAX);
        assert_eq!(VideoSpec { rate_num: 30000, rate_den: 1001, ..spec() }.settings().frame_rate, 30000.0 / 1001.0);
    }

    #[test]
    fn colour_code_points_the_framework_names_become_tags_and_the_rest_stay_unspecified() {
        let tags = |primaries, transfer, matrix| ColourCodes { primaries, transfer, matrix }.tags();
        assert_eq!(tags(1, 1, 1), Tags { primaries: Some(Primaries::Bt709), transfer: Some(Transfer::Bt709), matrix: Some(Matrix::Bt709) });
        assert_eq!(tags(9, 16, 1).primaries, Some(Primaries::Bt2020));
        assert_eq!(tags(9, 16, 1).transfer, Some(Transfer::Pq));
        assert_eq!(tags(12, 13, 1), Tags { primaries: Some(Primaries::P3D65), transfer: Some(Transfer::Srgb), matrix: Some(Matrix::Bt709) });
        assert_eq!(tags(9, 18, 1).transfer, Some(Transfer::Hlg));
        assert_eq!(tags(2, 8, 1), Tags { primaries: None, transfer: Some(Transfer::Linear), matrix: Some(Matrix::Bt709) });
        assert_eq!(tags(2, 2, 2), Tags { primaries: None, transfer: None, matrix: None });
    }
}
