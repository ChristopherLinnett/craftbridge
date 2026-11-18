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

/// The seconds a MediaCodec encoder should be asked to go between key frames for a key frame at least every
/// `interval` frames at `rate_num`/`rate_den`. MediaCodec's key frame interval counts seconds, as a float
/// since Android 7.1 (API 25, below the SDK's minSdk), and an encoder turns it back into frames with the
/// rate: the platform's software encoders round to the nearest frame, while the acceptance phone's hardware
/// encoders go one frame past a whole number the float's error leaves a hair above it (5 frames at 30 a
/// second come out every 6). A quarter of a frame short of the interval lands both on it, and neither past
/// it, which is the interval's promise (VideoToolbox's is a maximum too).
#[cfg(any(target_os = "android", test))]
pub(crate) fn key_frame_seconds(interval: u32, rate_num: u32, rate_den: u32) -> f32 {
    ((f64::from(interval.max(1)) - 0.25) * f64::from(rate_den.max(1)) / f64::from(rate_num.max(1))) as f32
}

/// The microseconds a MediaCodec encoder should stamp the sample presented at `ticks` of a
/// `rate_num`-per-second movie: MediaCodec carries presentation times as whole microseconds, while the
/// movie carries them as ticks, so the tick count becomes microseconds rounded to the nearest.
#[cfg(any(target_os = "android", test))]
pub(crate) fn microseconds_at(ticks: i64, rate_num: u32) -> i64 {
    let rate = i64::from(rate_num).max(1);
    ticks.saturating_mul(1_000_000).saturating_add(rate / 2).div_euclid(rate)
}

/// The movie tick `microseconds` of a `rate_num`-per-second movie was made from, taking back exactly what
/// [`microseconds_at`] gave: rounding each way moves a time by at most half a microsecond, which is under
/// half a tick for any rate under two million a second, and so cannot reach a neighbouring tick. Beyond that
/// the round trip can land on a neighbour; a movie's rate, which a [`VideoSpec`] check bounds far below
/// that, always returns its own tick.
#[cfg(any(target_os = "android", test))]
pub(crate) fn ticks_at(microseconds: i64, rate_num: u32) -> i64 {
    microseconds.saturating_mul(i64::from(rate_num)).saturating_add(500_000).div_euclid(1_000_000)
}

/// A level of the movie's codec standard: the number its stream carries (H.264's `level_idc`, HEVC's
/// `general_level_idc`) and the one Android's `MediaCodecInfo.CodecProfileLevel` names it by.
#[cfg(any(target_os = "android", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Level {
    pub(crate) idc: u8,
    pub(crate) android: i32,
}

/// One level's limits: its numbers, the largest picture (H.264 in macroblocks, HEVC in luma samples), the
/// most of those a second, and the highest bit rate in kbit/s before the profile's factor.
#[cfg(any(target_os = "android", test))]
type Limits = (u8, i32, u64, u64, u64);

/// H.264's levels (ITU-T H.264 Table A-1, the rows Android's `MediaCodecInfo` reads them by; level 1b, which
/// only Baseline names apart, left out). The High profile's bit rates are a quarter higher (Table A-2).
#[cfg(any(target_os = "android", test))]
const H264_LEVELS: [Limits; 19] = [
    (10, 0x1, 99, 1_485, 64),
    (11, 0x4, 396, 3_000, 192),
    (12, 0x8, 396, 6_000, 384),
    (13, 0x10, 396, 11_880, 768),
    (20, 0x20, 396, 11_880, 2_000),
    (21, 0x40, 792, 19_800, 4_000),
    (22, 0x80, 1_620, 20_250, 4_000),
    (30, 0x100, 1_620, 40_500, 10_000),
    (31, 0x200, 3_600, 108_000, 14_000),
    (32, 0x400, 5_120, 216_000, 20_000),
    (40, 0x800, 8_192, 245_760, 20_000),
    (41, 0x1000, 8_192, 245_760, 50_000),
    (42, 0x2000, 8_704, 522_240, 50_000),
    (50, 0x4000, 22_080, 589_824, 135_000),
    (51, 0x8000, 36_864, 983_040, 240_000),
    (52, 0x10000, 36_864, 2_073_600, 240_000),
    (60, 0x20000, 139_264, 4_177_920, 240_000),
    (61, 0x40000, 139_264, 8_355_840, 480_000),
    (62, 0x80000, 139_264, 16_711_680, 800_000),
];

/// HEVC's Main tier levels (ITU-T H.265 Tables A.8 and A.9, as Android's `MediaCodecInfo` reads them).
#[cfg(any(target_os = "android", test))]
const HEVC_LEVELS: [Limits; 13] = [
    (30, 0x1, 36_864, 552_960, 128),
    (60, 0x4, 122_880, 3_686_400, 1_500),
    (63, 0x10, 245_760, 7_372_800, 3_000),
    (90, 0x40, 552_960, 16_588_800, 6_000),
    (93, 0x100, 983_040, 33_177_600, 10_000),
    (120, 0x400, 2_228_224, 66_846_720, 12_000),
    (123, 0x1000, 2_228_224, 133_693_440, 20_000),
    (150, 0x4000, 8_912_896, 267_386_880, 25_000),
    (153, 0x10000, 8_912_896, 534_773_760, 40_000),
    (156, 0x40000, 8_912_896, 1_069_547_520, 60_000),
    (180, 0x100000, 35_651_584, 1_069_547_520, 60_000),
    (183, 0x400000, 35_651_584, 2_139_095_040, 120_000),
    (186, 0x1000000, 35_651_584, 4_278_190_080, 240_000),
];

#[cfg(any(target_os = "android", test))]
impl VideoSpec {
    /// The lowest level of the movie's codec whose limits hold it: the picture and each of its sides, the
    /// pictures a second at its rate, and the bit rate it aims at; the highest where none does, which an
    /// encoder then refuses. A MediaCodec encoder asked for a profile must
    /// be asked for a level with it (`MediaFormat.KEY_PROFILE`), and this is the one VideoToolbox's
    /// automatic level picks for the same movie.
    pub(crate) fn level(&self) -> Level {
        let settings = self.settings();
        let (w, h) = (u64::from(self.width), u64::from(self.height));
        // H.264 counts 16x16 macroblocks; HEVC counts luma samples of a picture coded in 8x8 blocks.
        let (table, (width, height), bits_per_kbit): (&[Limits], (u64, u64), u64) = match self.codec {
            VideoCodec::H264 => (&H264_LEVELS, (w.div_ceil(16), h.div_ceil(16)), 1_250),
            VideoCodec::Hevc => (&HEVC_LEVELS, (w.div_ceil(8) * 8, h.div_ceil(8) * 8), 1_000),
        };
        let size = width.saturating_mul(height);
        let bits = settings.average_bit_rate.map_or(0, |bits| u64::try_from(bits).unwrap_or(u64::MAX));
        let (num, den) = (u64::from(self.rate_num), u64::from(self.rate_den.max(1)));
        let holds = |&&(_, _, most, per_second, kbits): &&Limits| {
            let side = (most.saturating_mul(8) as f64).sqrt() as u64;
            size <= most
                && width <= side
                && height <= side
                && size.saturating_mul(num) <= per_second.saturating_mul(den)
                && bits <= kbits.saturating_mul(bits_per_kbit)
        };
        let &(idc, android, ..) = table.iter().find(holds).or(table.last()).unwrap_or(&(0, 0, 0, 0, 0));
        Level { idc, android }
    }
}

/// The colour a MediaCodec encoder is told its stream is, in the numbers Android's `MediaFormat` gives its
/// `color-standard`, `color-transfer` and `color-range` keys.
#[cfg(any(target_os = "android", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AndroidColour {
    /// `COLOR_STANDARD_*`, which names the primaries and the matrix together.
    pub(crate) standard: Option<i32>,
    /// `COLOR_TRANSFER_*`.
    pub(crate) transfer: Option<i32>,
    /// `COLOR_RANGE_*`.
    pub(crate) range: i32,
}

#[cfg(any(target_os = "android", test))]
impl Tags {
    /// The tags in `MediaFormat`'s numbers, `None` where the platform has no name for what they are, which is
    /// then left unset rather than guessed at. A standard names primaries and a matrix as one, so only BT.709
    /// primaries with the BT.709 matrix have one here: `COLOR_STANDARD_BT2020` would assert BT.2020's matrix,
    /// which no export converts with, and Display P3 has no standard at all. The transfers are the four
    /// `MediaFormat` names (BT.709's curve is its `COLOR_TRANSFER_SDR_VIDEO`); sRGB's has none. The range
    /// is always limited: a [`VideoSpec`]'s colour is limited-range, as the movie export's is.
    pub(crate) fn android(&self) -> AndroidColour {
        AndroidColour {
            standard: match (self.primaries, self.matrix) {
                (Some(Primaries::Bt709), Some(Matrix::Bt709)) => Some(COLOR_STANDARD_BT709),
                _ => None,
            },
            transfer: self.transfer.and_then(|tag| match tag {
                Transfer::Bt709 => Some(COLOR_TRANSFER_SDR_VIDEO),
                Transfer::Linear => Some(COLOR_TRANSFER_LINEAR),
                Transfer::Pq => Some(COLOR_TRANSFER_ST2084),
                Transfer::Hlg => Some(COLOR_TRANSFER_HLG),
                Transfer::Srgb => None,
            }),
            range: COLOR_RANGE_LIMITED,
        }
    }
}

/// `MediaFormat`'s colour numbers, by their names there.
#[cfg(any(target_os = "android", test))]
const COLOR_STANDARD_BT709: i32 = 1;
#[cfg(any(target_os = "android", test))]
const COLOR_TRANSFER_LINEAR: i32 = 1;
#[cfg(any(target_os = "android", test))]
const COLOR_TRANSFER_SDR_VIDEO: i32 = 3;
#[cfg(any(target_os = "android", test))]
const COLOR_TRANSFER_ST2084: i32 = 6;
#[cfg(any(target_os = "android", test))]
const COLOR_TRANSFER_HLG: i32 = 7;
#[cfg(any(target_os = "android", test))]
const COLOR_RANGE_LIMITED: i32 = 2;

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

    #[test]
    fn the_tags_a_media_codec_encoder_names_get_its_numbers_and_the_rest_stay_unset() {
        let android = |primaries, transfer, matrix| {
            let AndroidColour { standard, transfer, range } = ColourCodes { primaries, transfer, matrix }.tags().android();
            (standard, transfer, range)
        };
        // `MediaFormat.COLOR_STANDARD_BT709`, `COLOR_TRANSFER_SDR_VIDEO` and `COLOR_RANGE_LIMITED`.
        assert_eq!(android(1, 1, 1), (Some(1), Some(3), 2));
        // `COLOR_TRANSFER_ST2084`, `COLOR_TRANSFER_HLG` and `COLOR_TRANSFER_LINEAR`: the platform's own numbers,
        // not the H.273 code points (16, 18 and 8) the request carries.
        assert_eq!(android(9, 16, 1), (None, Some(6), 2), "BT.2020 primaries with the BT.709 matrix have no standard");
        assert_eq!(android(9, 18, 1).1, Some(7), "the hybrid log gamma transfer");
        assert_eq!(android(2, 8, 1).1, Some(1), "linear light");
        assert_eq!(android(12, 13, 1), (None, None, 2), "Display P3 and sRGB's curve have no MediaFormat names");
        assert_eq!(android(1, 1, 0), (None, Some(3), 2), "BT.709 primaries with no matrix tag name no standard");
        assert_eq!(android(0, 0, 0), (None, None, 2), "always limited range: the request's colour is limited");
    }

    #[test]
    fn the_level_is_the_lowest_whose_picture_rate_and_bit_rate_limits_hold_the_movie() {
        let level = |codec, width, height, (rate_num, rate_den), kbps| {
            let spec = VideoSpec {
                codec,
                width,
                height,
                rate_num,
                rate_den,
                rate_control: RateControl::Bitrate { kbps },
                keyframe_interval: 30,
                colour: ColourCodes { primaries: 1, transfer: 1, matrix: 1 },
            };
            let Level { idc, android } = spec.level();
            (idc, android)
        };
        // 128 by 128 at 2 Mbit/s: 1.3 holds the picture and the rate but only 0.96 Mbit/s at High, and 2.0
        // holds 2.5 (`AVCLevel2`); HEVC's 2.0 holds 1.5 Mbit/s and 2.1 holds 3.
        assert_eq!(level(VideoCodec::H264, 128, 128, (30, 1), 2000), (20, 0x20));
        assert_eq!(level(VideoCodec::Hevc, 128, 128, (30, 1), 2000), (63, 0x10));
        assert_eq!(level(VideoCodec::H264, 128, 128, (30, 1), 2400).0, 20, "2.0's 2 Mbit/s are 2.5 at High");
        // 1080p: 8160 macroblocks are level 4's at thirty a second and 4.2's at sixty.
        assert_eq!(level(VideoCodec::H264, 1920, 1080, (30, 1), 10_000), (40, 0x800));
        assert_eq!(level(VideoCodec::H264, 1920, 1080, (60, 1), 10_000), (42, 0x2000));
        assert_eq!(level(VideoCodec::H264, 1920, 1080, (30000, 1001), 30_000), (41, 0x1000), "30 Mbit/s is past level 4's 25 at High");
        // HEVC counts luma samples of 8x8 blocks: 1920 by 1088 is level 4's, and 15 Mbit/s past its 12.
        assert_eq!(level(VideoCodec::Hevc, 1920, 1080, (30, 1), 10_000), (120, 0x400));
        assert_eq!(level(VideoCodec::Hevc, 1920, 1080, (30, 1), 15_000), (123, 0x1000));
        // A side holds a level too: a sliver of 16 by 4096 is few samples, but 4096 is past level 3.1's longest
        // side (the square root of eight times its 983040 samples, 2804) and within level 4's (4222).
        assert_eq!(level(VideoCodec::Hevc, 16, 4096, (30, 1), 100), (120, 0x400));
        assert_eq!(level(VideoCodec::H264, 8192, 8192, (120, 1), 2_000_000), (62, 0x80000), "past every level: the highest");
    }

    #[test]
    fn a_key_frame_interval_of_frames_becomes_its_seconds_a_quarter_frame_short() {
        // Back in frames at the rate, each is a quarter short: nearest is the interval, and so is up.
        for (interval, num, den) in [(60u32, 30u32, 1u32), (5, 30, 1), (1, 30000, 1001), (48, 24000, 1001), (25, 25, 1), (30000, 30000, 1001)] {
            let frames = f64::from(key_frame_seconds(interval, num, den)) * f64::from(num) / f64::from(den);
            assert!((frames - (f64::from(interval) - 0.25)).abs() < 1e-3 * f64::from(interval), "{interval} at {num}/{den}: {frames}");
        }
        assert_eq!(key_frame_seconds(60, 30, 1), (59.75f64 / 30.0) as f32, "EffectCraft's own two-second spacing at thirty");
        assert_eq!(key_frame_seconds(0, 30, 1), key_frame_seconds(1, 30, 1), "an interval of none is every frame, never past it");
        assert!(key_frame_seconds(u32::MAX, 1, 1).is_finite());
    }

    #[test]
    fn sample_times_survive_the_trip_through_microseconds() {
        for (rate_num, rate_den) in [(30u32, 1u32), (25, 1), (30000, 1001), (24000, 1001), (60, 1), (1, 1)] {
            for index in [0u64, 1, 2, 599, 30_000, 3_600_000] {
                let ticks = i64::try_from(index * u64::from(rate_den)).unwrap();
                let us = microseconds_at(ticks, rate_num);
                assert_eq!(ticks_at(us, rate_num), ticks, "tick {ticks} of {rate_num}/{rate_den} through its microseconds {us}");
                if index > 0 {
                    let previous = i64::try_from((index - 1) * u64::from(rate_den)).unwrap();
                    assert!(us > microseconds_at(previous, rate_num), "and strictly between neighbours: a movie never stamps two samples the same");
                }
            }
        }
    }
}
