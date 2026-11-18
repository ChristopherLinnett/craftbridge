//! What passes between the movie and a MediaCodec encoder. A picture goes in laid out as the encoder's own
//! input format says, which is the platform's only description of a raw input buffer. What comes out is the
//! part of each output buffer the codec says it wrote; a video encoder's arrives as Annex B streams
//! (start-coded NAL units), while [`crate::Movie`] writes length-prefixed packets and needs a sample-entry
//! record before its first picture, which MediaCodec gives only as parameter-set NAL units. The units are
//! split here, a packet is made length-prefixed and keyed by what its units are, and the record is built from
//! the parameter sets the encoder emitted, as the movie export builds its own record from its own encoder's
//! settings (plan §18). Safe code, so a host tests every rule; the platform module only moves bytes.

use filmcraft_isobmff::{AvcConfig, HevcConfig, HevcNalArray};

use crate::encode::{EncodeError, VideoCodec, Yuv420};

/// `MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420SemiPlanar`, the one colour format [`Yuv420`] is
/// written in for an encoder ([`InputLayout::packed`]): a plane of luma, then one of interleaved Cb and Cr.
pub(crate) const SEMI_PLANAR: i32 = 21;

/// Where a picture's planes go in an encoder's input buffer of [`SEMI_PLANAR`]: luma rows `stride` bytes
/// apart, then the rows of interleaved chroma, as many bytes apart, from row `slice_height`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InputLayout {
    pub(crate) stride: usize,
    pub(crate) slice_height: usize,
}

impl InputLayout {
    /// The layout an encoder's input format describes for a `width` by `height` picture, from its
    /// `color-format`, `stride` and `slice-height`. The colour format must be the one the picture is written
    /// in: a codec that took no such request reads a layout of its own, and would encode the picture's bytes
    /// as some other picture. A missing or zero stride or slice height is the picture's own side, which is how
    /// Android's MediaCodec reference reads a slice height of 0; a layout narrower or shorter than the
    /// picture holds no picture, and is refused rather than written past.
    pub(crate) fn of(width: u32, height: u32, color_format: Option<i32>, stride: Option<i32>, slice_height: Option<i32>) -> Result<Self, String> {
        if color_format != Some(SEMI_PLANAR) {
            return Err(format!("the encoder reads its pictures in colour format {color_format:?}, not the semi-planar one they are written in"));
        }
        let side = |given: Option<i32>, least: u32, name: &str| -> Result<usize, String> {
            let value = match given.filter(|&value| value != 0) {
                None => least,
                Some(value) => u32::try_from(value).map_err(|_| format!("the encoder's input {name} is {value}"))?,
            };
            if value < least {
                return Err(format!("the encoder's input {name} of {value} is less than the picture's {least}"));
            }
            usize::try_from(value).map_err(|_| format!("the encoder's input {name} of {value} does not fit this device"))
        };
        Ok(Self { stride: side(stride, width, "stride")?, slice_height: side(slice_height, height, "slice height")? })
    }

    /// `frame` as one input buffer of this layout, its padding zero: a plane of `slice_height` rows, then
    /// the chroma plane of half as many, which is the whole frame size a semi-planar buffer has.
    pub(crate) fn packed(&self, frame: &Yuv420, width: u32, height: u32) -> Result<Vec<u8>, EncodeError> {
        let too_big = || EncodeError::Invalid(format!("a {width}x{height} picture in the encoder's layout is past what this device can hold"));
        let luma = self.stride.checked_mul(self.slice_height).ok_or_else(too_big)?;
        let chroma = self.stride.checked_mul(self.slice_height.div_ceil(2)).ok_or_else(too_big)?;
        let mut buffer = vec![0u8; luma.checked_add(chroma).ok_or_else(too_big)?];
        let (luma_plane, chroma_plane) = buffer.split_at_mut_checked(luma).ok_or_else(too_big)?;
        frame.write_biplanar(width, height, (luma_plane, self.stride), (chroma_plane, self.stride))?;
        Ok(buffer)
    }
}

/// The bytes of an output buffer the codec says it wrote: `size` of them from `offset`. A range that is not
/// inside the buffer is the codec's error, never a shorter unit and never a panic.
pub(crate) fn written(buffer: &[u8], offset: i32, size: i32) -> Result<&[u8], String> {
    let range = usize::try_from(offset).ok().zip(usize::try_from(size).ok()).and_then(|(from, size)| Some(from..from.checked_add(size)?));
    range.and_then(|range| buffer.get(range)).ok_or_else(|| format!("the codec wrote {size} bytes at {offset} of a buffer of {}", buffer.len()))
}

/// H.264 NAL unit types (ITU-T H.264, Table 7-1) this code names.
const H264_IDR: u8 = 5;
const H264_SPS: u8 = 7;
const H264_PPS: u8 = 8;

/// HEVC NAL unit types (ITU-T H.265, Table 7-1) this code names. The instantaneous decoding refresh
/// pictures run from BLA_W_LP (16) through the reserved IRAP types.
const HEVC_IRAP_FIRST: u8 = 16;
const HEVC_IRAP_LAST: u8 = 23;
const HEVC_VPS: u8 = 32;
const HEVC_SPS: u8 = 33;
const HEVC_PPS: u8 = 34;

/// The NAL length field the packets and records use: four bytes, as the movie export writes its samples.
pub(crate) const LENGTH_SIZE: u8 = 4;

/// The unit type encoded in a unit's first byte, which H.264 and HEVC number differently. `None` of a unit
/// with no byte to name itself with.
fn nal_type(codec: VideoCodec, unit: &[u8]) -> Option<u8> {
    let &first = unit.first()?;
    Some(match codec {
        VideoCodec::H264 => first & 0x1F,
        VideoCodec::Hevc => (first >> 1) & 0x3F,
    })
}

/// Where the `00 01` of the next start code at or after `from` begins: two zeros and a one, with any third
/// zero before them counted as the code's own.
fn start_code_at(data: &[u8], from: usize) -> Option<usize> {
    (from..data.len().saturating_sub(2)).find(|&i| data.get(i..i + 3) == Some(&[0, 0, 1][..]))
}

/// The bytes of an output buffer, split into its NAL units. A buffer is Annex B: every unit follows a start
/// code, and only zeros and start codes separate them; anything else before the first start code is not a
/// stream this kind of encoder was given. Trailing zeros separate units and pad the buffer, so they are no
/// unit's bytes.
pub(crate) fn nal_units(data: &[u8]) -> Result<Vec<&[u8]>, String> {
    let first = start_code_at(data, 0).ok_or("the encoder's buffer holds no start-coded NAL unit")?;
    if data.iter().take(first).any(|&byte| byte != 0) {
        return Err("the encoder's buffer does not begin with a start-coded NAL unit".into());
    }
    let mut units = Vec::new();
    let (mut at, mut next) = (first + 3, start_code_at(data, first + 3));
    loop {
        let end = next.unwrap_or(data.len());
        let mut unit = data.get(at..end).ok_or_else(|| "the encoder's buffer is shorter than its own stream".to_string())?;
        while unit.last() == Some(&0) {
            unit = unit.get(..unit.len() - 1).unwrap_or_default();
        }
        if unit.is_empty() {
            return Err("two of the encoder's start codes have no NAL unit between them".into());
        }
        units.push(unit);
        match next {
            Some(code) => (at, next) = (code + 3, start_code_at(data, code + 3)),
            None => return Ok(units),
        }
    }
}

/// Whether a packet's units let decoding start here: a picture that refreshes the whole frame.
pub(crate) fn is_key(codec: VideoCodec, units: &[&[u8]]) -> bool {
    units.iter().any(|unit| match codec {
        VideoCodec::H264 => nal_type(codec, unit) == Some(H264_IDR),
        VideoCodec::Hevc => nal_type(codec, unit).is_some_and(|kind| (HEVC_IRAP_FIRST..=HEVC_IRAP_LAST).contains(&kind)),
    })
}

/// The units as one packet: every unit's byte count in four bytes before the unit, which is what
/// [`crate::Movie`] writes and what the sample-entry record says the stream is laid out with.
pub(crate) fn length_prefixed(units: &[&[u8]]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(units.iter().map(|unit| unit.len() + LENGTH_SIZE as usize).sum());
    for unit in units {
        packet.extend(u32::try_from(unit.len()).unwrap_or(u32::MAX).to_be_bytes());
        packet.extend_from_slice(unit);
    }
    packet
}

/// The sample-entry record the encoder's parameter sets describe: an `avcC` naming its SPS and PPS, or an
/// `hvcC` naming its VPS, SPS and PPS and carrying what its SPS says about the stream, as the movie export's
/// own record does. An error, naming what is missing, when sets the codec needs never came.
pub(crate) fn record(codec: VideoCodec, csd: &[&[u8]]) -> Result<Vec<u8>, String> {
    let sets = |wanted: &[u8]| -> Vec<Vec<u8>> {
        csd.iter().filter(|unit| nal_type(codec, unit).is_some_and(|kind| wanted.contains(&kind))).map(|unit| unit.to_vec()).collect()
    };
    match codec {
        VideoCodec::H264 => {
            let (sps, pps) = (sets(&[H264_SPS]), sets(&[H264_PPS]));
            if sps.is_empty() || pps.is_empty() {
                return Err("the encoder never gave its H.264 SPS and PPS".into());
            }
            Ok(AvcConfig::new(sps, pps, LENGTH_SIZE).to_bytes())
        }
        VideoCodec::Hevc => {
            let (vps, sps, pps) = (sets(&[HEVC_VPS]), sets(&[HEVC_SPS]), sets(&[HEVC_PPS]));
            if vps.is_empty() || sps.is_empty() || pps.is_empty() {
                return Err("the encoder never gave its HEVC VPS, SPS and PPS".into());
            }
            hevc_record(&vps, &sps, &pps)
        }
    }
}

/// The `hvcC` its first SPS describes: the general profile, level and constraints stream pictures fall
/// under, and the stream's chroma format and bit depths, with the three arrays the sets themselves make.
/// EffectCraft's movie export writes these fields from its own encoder's settings; here the encoder's own
/// SPS is those settings (plan §18), and an SPS that cannot be read is refused, never guessed at.
fn hevc_record(vps: &[Vec<u8>], sps: &[Vec<u8>], pps: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let array = |nal_type: u8, sets: &[Vec<u8>]| HevcNalArray { completeness: true, nal_type, nalus: sets.to_vec() };
    let read = |sps_unit: &[u8]| {
        // HEVC NAL headers are two bytes; the payload starts after them, emulation prevention bytes gone.
        let body = no_emulation(sps_unit.get(2..)?);
        let mut bits = Bits { data: &body, at: 0 };
        let _vps_id = bits.u(4)?;
        let max_sub_layers = bits.u(3)?;
        let temporal_id_nesting = bits.u(1)? == 1;
        let profile_space = bits.u(2)? as u8;
        let tier_flag = bits.u(1)? == 1;
        let profile_idc = bits.u(5)? as u8;
        let compat_flags = bits.u(32)? as u32;
        let constraints = bits.u(48)?;
        let level_idc = bits.u(8)? as u8;
        let mut sub_layers = Vec::new();
        for _ in 0..max_sub_layers {
            sub_layers.push((bits.u(1)? == 1, bits.u(1)? == 1));
        }
        if max_sub_layers > 0 {
            bits.u(2 * (8 - max_sub_layers))?;
        }
        for &(profile_present, level_present) in &sub_layers {
            // A sub-layer's own profile_tier_level: 88 bits of profile and constraints and 8 of level,
            // whichever its flags ask for (ITU-T H.265 7.3.3).
            if profile_present {
                bits.u(88)?;
            }
            if level_present {
                bits.u(8)?;
            }
        }
        let _sps_id = bits.ue()?;
        let chroma = bits.ue()?;
        if chroma == 3 {
            bits.u(1)?;
        }
        bits.ue()?; // pic_width_in_luma_samples
        bits.ue()?; // pic_height_in_luma_samples
        if bits.u(1)? == 1 {
            for _ in 0..4 {
                bits.ue()?;
            }
        }
        let bit_depth_luma = bits.ue()? + 8;
        let bit_depth_chroma = bits.ue()? + 8;
        Some(HevcConfig {
            general_profile_space: profile_space,
            general_tier_flag: tier_flag,
            general_profile_idc: profile_idc,
            general_profile_compatibility_flags: compat_flags,
            general_constraint_indicator_flags: constraints,
            general_level_idc: level_idc,
            min_spatial_segmentation_idc: 0,
            parallelism_type: 0,
            chroma_format_idc: chroma.min(3) as u8,
            bit_depth_luma: bit_depth_luma.min(16) as u8,
            bit_depth_chroma: bit_depth_chroma.min(16) as u8,
            avg_frame_rate: 0,
            constant_frame_rate: 0,
            num_temporal_layers: (max_sub_layers + 1).min(8) as u8,
            temporal_id_nested: temporal_id_nesting,
            length_size: LENGTH_SIZE,
            arrays: vec![array(HEVC_VPS, vps), array(HEVC_SPS, sps), array(HEVC_PPS, pps)],
        })
    };
    let config = sps.first().and_then(|unit| read(unit)).ok_or("the encoder's HEVC SPS cannot be read: it is shorter or shaped unlike one")?;
    Ok(config.to_bytes())
}

/// The bytes with NAL emulation prevention gone: a `03` that follows two zeros goes; the test is on the
/// raw bytes before it, as the codec standards write it (ITU-T H.264 7.4.2.3), so a removed `03` still
/// stands in the pair a later byte is tested against.
fn no_emulation(nal: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(nal.len());
    for (i, &byte) in nal.iter().enumerate() {
        if byte == 3 && i >= 2 && nal.get(i - 2) == Some(&0) && nal.get(i - 1) == Some(&0) {
            continue;
        }
        bytes.push(byte);
    }
    bytes
}

/// Bits of a byte string, most significant first.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    /// The next `n` bits as one number, `None` when the string runs out.
    fn u(&mut self, n: u64) -> Option<u64> {
        let mut value = 0u64;
        for _ in 0..n {
            let byte = *self.data.get(self.at / 8)?;
            let bit = (byte >> (7 - (self.at % 8))) & 1;
            value = (value << 1) | u64::from(bit);
            self.at += 1;
        }
        Some(value)
    }

    /// The next Exp-Golomb-coded number (ITU-T H.265 5.2.5): a run of zeros, a one, and that many bits.
    fn ue(&mut self) -> Option<u64> {
        let zeros = (0..64).find(|_| self.u(1) == Some(1))?;
        let rest = self.u(zeros)?;
        Some((1u64 << zeros) - 1 + rest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(bytes: &[u8]) -> Vec<&[u8]> {
        nal_units(bytes).unwrap()
    }

    #[test]
    fn the_layout_is_the_input_formats_and_a_missing_side_is_the_pictures() {
        let nv12 = Some(SEMI_PLANAR);
        assert_eq!(InputLayout::of(160, 120, nv12, Some(192), Some(128)), Ok(InputLayout { stride: 192, slice_height: 128 }));
        assert_eq!(InputLayout::of(160, 120, nv12, None, None), Ok(InputLayout { stride: 160, slice_height: 120 }));
        assert_eq!(
            InputLayout::of(160, 120, nv12, Some(0), Some(0)),
            Ok(InputLayout { stride: 160, slice_height: 120 }),
            "a slice height of 0 is the picture's"
        );
        assert_eq!(InputLayout::of(160, 120, nv12, Some(158), None), Err("the encoder's input stride of 158 is less than the picture's 160".into()));
        assert_eq!(InputLayout::of(160, 120, nv12, None, Some(118)), Err("the encoder's input slice height of 118 is less than the picture's 120".into()));
        assert_eq!(InputLayout::of(160, 120, nv12, Some(-1), None), Err("the encoder's input stride is -1".into()));
        // `COLOR_FormatYUV420Flexible`, which a codec asked for nothing reads, and none at all.
        assert_eq!(
            InputLayout::of(160, 120, Some(0x7F42_0888), None, None),
            Err("the encoder reads its pictures in colour format Some(2135033992), not the semi-planar one they are written in".into())
        );
        assert!(InputLayout::of(160, 120, None, None, None).is_err());
    }

    #[test]
    fn a_picture_is_packed_luma_first_then_its_chroma_from_the_slice_height_with_zero_padding() {
        let frame = Yuv420 { y: vec![1, 2, 3, 4, 5, 6, 7, 8], u: vec![9, 10], v: vec![11, 12] };
        let layout = InputLayout { stride: 6, slice_height: 4 };
        // Two picture rows and two of padding to the slice height, then the chroma row and its own padding row.
        let rows: [[u8; 6]; 6] = [[1, 2, 3, 4, 0, 0], [5, 6, 7, 8, 0, 0], [0; 6], [0; 6], [9, 11, 10, 12, 0, 0], [0; 6]];
        assert_eq!(layout.packed(&frame, 4, 2).unwrap(), rows.concat());
        let tight = InputLayout { stride: 4, slice_height: 2 };
        assert_eq!(tight.packed(&frame, 4, 2).unwrap(), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 10, 12]);
        assert_eq!(
            tight.packed(&Yuv420 { y: vec![0; 4], u: vec![0], v: vec![0] }, 4, 2),
            Err(EncodeError::Invalid("the frame's planes are not those of a 4x2 picture".into()))
        );
    }

    #[test]
    fn the_bytes_a_codec_wrote_are_its_range_of_the_buffer_and_a_range_outside_it_is_an_error() {
        let buffer = [0u8, 1, 2, 3, 4, 5];
        assert_eq!(written(&buffer, 2, 3), Ok(&[2u8, 3, 4][..]));
        assert_eq!(written(&buffer, 0, 0), Ok(&[][..]));
        assert_eq!(written(&buffer, 4, 3), Err("the codec wrote 3 bytes at 4 of a buffer of 6".into()));
        assert_eq!(written(&buffer, -1, 2), Err("the codec wrote 2 bytes at -1 of a buffer of 6".into()));
        assert_eq!(written(&buffer, 0, -2), Err("the codec wrote -2 bytes at 0 of a buffer of 6".into()));
        assert!(written(&buffer, i32::MAX, i32::MAX).is_err());
    }

    #[test]
    fn an_annex_b_buffer_splits_at_its_start_codes_of_either_size() {
        assert_eq!(units(&[0, 0, 0, 1, 9, 8, 0, 0, 1, 7]), vec![&[9u8, 8][..], &[7u8][..]]);
        assert_eq!(units(&[0, 0, 1, 5]), vec![&[5u8][..]]);
    }

    #[test]
    fn padding_zeros_and_the_gaps_between_are_not_units() {
        // A trailing zero pads the buffer; a four-byte start code's leading zero is the code's own.
        assert_eq!(units(&[0, 0, 0, 1, 65, 66, 0, 0, 0, 1, 67, 0, 0]), vec![&[65u8, 66][..], &[67u8][..]]);
        assert_eq!(units(&[0, 0, 0, 0, 1, 7]), vec![&[7u8][..]], "a run of leading zeros is one start code");
    }

    #[test]
    fn a_buffer_that_is_not_a_start_coded_stream_is_an_error_not_a_unit_of_garbage() {
        assert!(nal_units(&[]).is_err(), "nothing");
        assert!(nal_units(&[0, 0]).is_err(), "not even a code");
        assert!(nal_units(&[65, 0, 0, 1, 7]).is_err(), "bytes before the first code: a length-prefixed stream mislabelled");
        assert!(nal_units(&[0, 0, 1, 0, 0, 1, 7]).is_err(), "nothing between two codes");
    }

    #[test]
    fn an_idr_picture_keys_h264_and_an_instantaneous_refresh_picture_keys_hevc() {
        // H.264 types are the low five bits of the first byte, HEVC's bits two to seven.
        assert!(is_key(VideoCodec::H264, &[&[0x65, 1, 2], &[7u8][..]]));
        assert!(!is_key(VideoCodec::H264, &[&[1u8, 2, 3]]));
        assert!(!is_key(VideoCodec::H264, &[&[9u8][..]]), "a parameter set on its own starts nothing");
        let irap = [0x26u8, 1]; // nal type 19 (IDR_W_RADL)
        let trail = [0x02u8, 1]; // nal type 1, a trailing picture
        assert!(is_key(VideoCodec::Hevc, &[&trail[..], &irap[..]]));
        assert!(!is_key(VideoCodec::Hevc, &[&trail[..]]));
        // The refresh types run from BLA_W_LP (16) to the last reserved one (23): the ends and the CRA between
        // are keys, and the types on either side are not.
        for (kind, key) in [(15u8, false), (16, true), (20, true), (21, true), (23, true), (24, false)] {
            assert_eq!(is_key(VideoCodec::Hevc, &[&[kind << 1, 1][..]]), key, "HEVC NAL type {kind}");
        }
    }

    #[test]
    fn a_packet_puts_each_unit_behind_its_own_four_byte_count() {
        assert_eq!(length_prefixed(&[&[1u8, 2], &[3][..]]), vec![0, 0, 0, 2, 1, 2, 0, 0, 0, 1, 3]);
    }

    #[test]
    fn the_h264_record_names_the_sps_and_pps_the_encoder_gave_and_round_trips() {
        let sps = [0x67u8, 100, 0, 30, 1, 2];
        let pps = [0x68u8, 200];
        let record = record(VideoCodec::H264, &[&sps, &pps]).unwrap();
        let parsed = AvcConfig::parse(&record).unwrap();
        assert_eq!(
            (parsed.profile, parsed.level, parsed.length_size, parsed.sps.as_slice(), parsed.pps.as_slice()),
            (100, 30, 4, &[sps.to_vec()][..], &[pps.to_vec()][..])
        );
    }

    #[test]
    fn a_record_without_its_parameter_sets_is_refused_naming_what_is_missing() {
        let sps = [0x67u8, 66, 0, 12];
        assert!(record(VideoCodec::H264, &[&sps]).unwrap_err().to_string().contains("SPS and PPS"));
        assert!(record(VideoCodec::H264, &[]).is_err());
        let vps = [0x40u8, 1];
        assert!(record(VideoCodec::Hevc, &[&vps]).unwrap_err().to_string().contains("VPS, SPS and PPS"));
    }

    /// Writes bits, most significant first, the way the SPS under test is meant to be read.
    struct Writer(Vec<u8>, usize);

    impl Writer {
        fn new() -> Self {
            Writer(Vec::new(), 0)
        }
        fn bits(&mut self, value: u64, n: u64) {
            for i in (0..n).rev() {
                let at = self.1 / 8;
                if self.0.len() <= at {
                    self.0.push(0);
                }
                if value & (1 << i) != 0 {
                    self.0[at] |= 1 << (7 - self.1 % 8);
                }
                self.1 += 1;
            }
        }
        fn ue(&mut self, value: u64) {
            let v = value + 1;
            let n = 64 - v.leading_zeros();
            self.bits(0, u64::from(n) - 1);
            self.bits(v, u64::from(n));
        }
    }

    /// An HEVC SPS NAL: Main, level 3.1 (93), one temporal layer, 4:2:0 8-bit, with a conformance window and
    /// trailing bits to prove the reader walks past them.
    fn hevc_sps() -> Vec<u8> {
        let mut w = Writer::new();
        w.bits(0, 4); // sps_video_parameter_set_id
        w.bits(0, 3); // sps_max_sub_layers_minus1
        w.bits(1, 1); // sps_temporal_id_nesting_flag
        w.bits(0, 2); // general_profile_space
        w.bits(0, 1); // general_tier_flag
        w.bits(1, 5); // general_profile_idc: Main
        w.bits(0b0100_0000_0000_0000_0000_0000_0000_0001, 32); // compatibility flags
        w.bits(0x9000_0000_0000, 48); // constraint indicators
        w.bits(93, 8); // general_level_idc
        w.ue(7); // sps_seq_parameter_set_id
        w.ue(1); // chroma_format_idc: 4:2:0
        w.ue(64); // pic_width_in_luma_samples
        w.ue(48); // pic_height_in_luma_samples
        w.bits(1, 1); // conformance_window_flag
        w.ue(1);
        w.ue(1);
        w.ue(1);
        w.ue(1);
        w.ue(0); // bit_depth_luma_minus8
        w.ue(0); // bit_depth_chroma_minus8
        w.bits(0b0000_0011, 8); // trailing bits and a padding byte that must not trip the reader
        let mut nal = vec![0x42, 0x01]; // nal_unit_type 33 (SPS): (0x42 >> 1) & 0x3F
        nal.extend(w.0);
        nal.push(0);
        nal
    }

    #[test]
    fn the_hevc_record_carries_what_its_sps_says_and_names_its_three_sets() {
        let (vps, sps, pps) = ([0x40u8, 1, 0, 0], hevc_sps(), [0x44u8, 7, 0]);
        let record = record(VideoCodec::Hevc, &[&vps, &sps, &pps]).unwrap();
        let parsed = HevcConfig::parse(&record).unwrap();
        assert_eq!(
            (parsed.general_profile_idc, parsed.general_level_idc, parsed.general_profile_compatibility_flags, parsed.general_constraint_indicator_flags),
            (1, 93, 0b0100_0000_0000_0000_0000_0000_0000_0001, 0x9000_0000_0000)
        );
        assert_eq!(
            (
                parsed.chroma_format_idc,
                parsed.bit_depth_luma,
                parsed.bit_depth_chroma,
                parsed.num_temporal_layers,
                parsed.temporal_id_nested,
                parsed.length_size
            ),
            (1, 8, 8, 1, true, 4)
        );
        assert_eq!(
            parsed.arrays.iter().map(|a| (a.completeness, a.nal_type, a.nalus.len())).collect::<Vec<_>>(),
            vec![(true, 32, 1), (true, 33, 1), (true, 34, 1)]
        );
    }

    #[test]
    fn an_sps_that_runs_out_or_is_not_one_is_refused() {
        assert!(record(VideoCodec::Hevc, &[&[0x40, 1], &[0x42, 0x01], &[0x44, 1]]).unwrap_err().to_string().contains("cannot be read"));
    }

    #[test]
    fn emulation_prevention_bytes_go_before_bits_are_read() {
        assert_eq!(no_emulation(&[0, 0, 3, 1]), vec![0u8, 0, 1]);
        // The second `03` follows `03 00`, not two zeros, on the raw bytes: it is data and stays.
        assert_eq!(no_emulation(&[0, 0, 0, 3, 0, 3, 2]), vec![0u8, 0, 0, 0, 3, 2]);
        assert_eq!(no_emulation(&[3, 0, 3]), vec![3u8, 0, 3], "a three away from a pair of zeros stays");
        assert_eq!(no_emulation(&[0, 1, 3, 5]), vec![0u8, 1, 3, 5], "a three after one zero and a one is data");
        assert_eq!(no_emulation(&[7, 0, 3, 9]), vec![7u8, 0, 3, 9], "and so is one after a single zero");
    }

    #[test]
    fn exp_golomb_numbers_decode_their_own_coding() {
        // The bits 0 1 0 | 0 0 0 1 1 1 0 | ...: "010" is 1, "0001110" is 13.
        let mut bits = Bits { data: &[0b0100_0011, 0b1000_0000], at: 0 };
        assert_eq!(bits.ue(), Some(1));
        assert_eq!(bits.ue(), Some(13));
        let mut short = Bits { data: &[0b1000_0000], at: 1 };
        assert_eq!(short.ue(), None, "a run of zeros that never reaches its one");
    }
}
