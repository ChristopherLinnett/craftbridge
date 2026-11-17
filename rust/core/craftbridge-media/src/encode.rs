//! What a hardware movie encoder is asked for, and the decisions around it that need no framework: the
//! settings a request becomes, the colour tags, the order frames reach the encoder in, and the planes
//! a picture is handed over as.
//!
//! Safe code, so every target tests it; the platform module only turns these answers into framework calls.

use std::collections::BTreeMap;
use std::fmt;

/// The codecs a phone's hardware encodes for an MP4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    Hevc,
}

/// How the encoder spends bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateControl {
    /// A target bit rate, in kbit/s.
    Bitrate { kbps: u32 },
    /// A constant quality, 1 to 100 (100 is the best the encoder offers).
    Quality { percent: u8 },
}

/// A colour description as ITU-T H.273 code points, the numbers a `colr` box and a bitstream's colour
/// description carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColourCodes {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
}

/// Everything the encoder needs to know about the movie's picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoSpec {
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    /// The frame rate is `rate_num / rate_den` frames a second.
    pub rate_num: u32,
    pub rate_den: u32,
    pub rate_control: RateControl,
    /// Frames between key frames, at least one.
    pub keyframe_interval: u32,
    pub colour: ColourCodes,
}

/// Why an encode did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EncodeError {
    /// This device or build has no hardware encoder for the request, so nothing was started: the caller
    /// encodes it another way.
    Unavailable(String),
    /// The request cannot be encoded as given.
    Invalid(String),
    /// A call failed part way through.
    Failed(String),
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(why) | Self::Invalid(why) | Self::Failed(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for EncodeError {}

impl VideoSpec {
    /// The request as the encoder can take it: a picture of even, non-zero sides an MP4 sample entry can
    /// name (16 bits each), at a frame rate that is a real number of frames.
    pub fn check(&self) -> Result<(), EncodeError> {
        let side = |name: &str, value: u32| {
            if value == 0 || !value.is_multiple_of(2) || value > u32::from(u16::MAX) {
                Err(EncodeError::Invalid(format!("the picture's {name} is {value}, which is not an even number from 2 to 65534")))
            } else {
                Ok(())
            }
        };
        side("width", self.width)?;
        side("height", self.height)?;
        if self.rate_num == 0 || self.rate_den == 0 {
            return Err(EncodeError::Invalid(format!("the frame rate {}/{} is not a rate", self.rate_num, self.rate_den)));
        }
        if self.keyframe_interval == 0 {
            return Err(EncodeError::Invalid("the key frame interval is zero frames".into()));
        }
        Ok(())
    }
}

impl VideoSpec {
    /// The presentation time of frame `index` in the movie's timescale (`rate_num` ticks a second): every
    /// frame lasts `rate_den` ticks.
    pub fn presentation_time(&self, index: u64) -> i64 {
        i64::try_from(index.saturating_mul(u64::from(self.rate_den))).unwrap_or(i64::MAX)
    }
}

/// One picture as 8-bit 4:2:0 planes, BT.709 limited range, tight (no row padding).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Yuv420 {
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

impl Yuv420 {
    /// Whether the planes are exactly the sizes of a `width` by `height` picture.
    pub fn is_picture(&self, width: u32, height: u32) -> bool {
        let (w, h) = (width as usize, height as usize);
        let chroma = w.div_ceil(2) * h.div_ceil(2);
        self.y.len() == w * h && self.u.len() == chroma && self.v.len() == chroma
    }

    /// The picture into a biplanar surface: luma rows `luma_stride` bytes apart, and one plane of
    /// interleaved Cb, Cr pairs `chroma_stride` bytes apart, which is the layout hardware encoders take.
    /// The surface's padding is left as it is. Fails, writing nothing, unless the planes are the picture's
    /// and the surface's are large enough for it.
    pub fn write_biplanar(&self, width: u32, height: u32, luma: (&mut [u8], usize), chroma: (&mut [u8], usize)) -> Result<(), EncodeError> {
        if !self.is_picture(width, height) {
            return Err(EncodeError::Invalid(format!("the frame's planes are not those of a {width}x{height} picture")));
        }
        let (w, h) = (width as usize, height as usize);
        if w == 0 || h == 0 {
            return Err(EncodeError::Invalid("the picture has no pixels".into()));
        }
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let ((luma, luma_stride), (chroma, chroma_stride)) = (luma, chroma);
        let too_small = || EncodeError::Failed("the encoder's surface is smaller than the picture".into());
        if luma_stride < w || chroma_stride < cw * 2 || luma.len() < rows_needed(luma_stride, w, h) || chroma.len() < rows_needed(chroma_stride, cw * 2, ch) {
            return Err(too_small());
        }
        for (dst, src) in luma.chunks_mut(luma_stride).zip(self.y.chunks(w)) {
            dst.get_mut(..w).ok_or_else(too_small)?.copy_from_slice(src);
        }
        let rows = chroma.chunks_mut(chroma_stride).zip(self.u.chunks(cw).zip(self.v.chunks(cw)));
        for (dst, (u, v)) in rows {
            for (pair, (&u, &v)) in dst.as_chunks_mut::<2>().0.iter_mut().zip(u.iter().zip(v)) {
                pair.copy_from_slice(&[u, v]);
            }
        }
        Ok(())
    }
}

/// The bytes a surface of `rows` rows must hold, the last row only as long as its `row_bytes`.
fn rows_needed(stride: usize, row_bytes: usize, rows: usize) -> usize {
    rows.saturating_sub(1).saturating_mul(stride).saturating_add(row_bytes)
}

/// Puts items that arrive in any order back into frame order: `push` hands back every item that is now next
/// in line.
#[derive(Debug)]
pub struct Reorder<T> {
    next: u64,
    total: u64,
    waiting: BTreeMap<u64, T>,
}

impl<T> Reorder<T> {
    /// A stream of `total` items, numbered from 0.
    pub fn new(total: u64) -> Self {
        Self { next: 0, total, waiting: BTreeMap::new() }
    }

    /// Item `index` has arrived. Returns the items now ready, in order, each with its index. An index
    /// outside the stream, or one that came before, is refused: it would otherwise wait for ever or be
    /// encoded twice.
    pub fn push(&mut self, index: u64, item: T) -> Result<Vec<(u64, T)>, EncodeError> {
        if index >= self.total {
            return Err(EncodeError::Failed(format!("frame {index} arrived in a movie of {} frames", self.total)));
        }
        if index < self.next || self.waiting.contains_key(&index) {
            return Err(EncodeError::Failed(format!("frame {index} arrived twice")));
        }
        self.waiting.insert(index, item);
        let mut ready = Vec::new();
        while let Some(item) = self.waiting.remove(&self.next) {
            ready.push((self.next, item));
            self.next += 1;
        }
        Ok(ready)
    }

    /// How many items have been passed on.
    pub fn passed(&self) -> u64 {
        self.next
    }

    /// Whether every item of the stream has been passed on.
    pub fn is_complete(&self) -> bool {
        self.next == self.total
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn spec() -> VideoSpec {
        VideoSpec {
            codec: VideoCodec::H264,
            width: 64,
            height: 48,
            rate_num: 30,
            rate_den: 1,
            rate_control: RateControl::Bitrate { kbps: 10_000 },
            keyframe_interval: 60,
            colour: ColourCodes { primaries: 1, transfer: 1, matrix: 1 },
        }
    }

    #[test]
    fn a_picture_needs_even_sides_a_sample_entry_can_name() {
        assert_eq!(spec().check(), Ok(()));
        for (width, height) in [(0, 48), (63, 48), (64, 47), (65536, 48), (64, 0)] {
            let error = VideoSpec { width, height, ..spec() }.check().unwrap_err();
            assert!(error.to_string().contains("is not an even number from 2 to 65534"), "{width}x{height}: {error}");
        }
        assert_eq!(VideoSpec { width: 65534, ..spec() }.check(), Ok(()));
    }

    #[test]
    fn a_rate_and_a_key_frame_interval_must_be_real() {
        assert_eq!(VideoSpec { rate_num: 0, ..spec() }.check().unwrap_err().to_string(), "the frame rate 0/1 is not a rate");
        assert_eq!(VideoSpec { rate_den: 0, ..spec() }.check().unwrap_err().to_string(), "the frame rate 30/0 is not a rate");
        assert_eq!(VideoSpec { keyframe_interval: 0, ..spec() }.check().unwrap_err().to_string(), "the key frame interval is zero frames");
    }

    #[test]
    fn every_frame_lasts_the_rates_denominator_in_ticks_of_its_numerator() {
        let ntsc = VideoSpec { rate_num: 30000, rate_den: 1001, ..spec() };
        assert_eq!((ntsc.presentation_time(0), ntsc.presentation_time(1), ntsc.presentation_time(30)), (0, 1001, 30030));
        assert_eq!(ntsc.presentation_time(u64::MAX), i64::MAX);
    }

    fn picture(width: usize, height: usize) -> Yuv420 {
        let chroma = width.div_ceil(2) * height.div_ceil(2);
        Yuv420 {
            y: (0..width * height).map(|i| i as u8).collect(),
            u: (0..chroma).map(|i| 100 + i as u8).collect(),
            v: (0..chroma).map(|i| 200 + i as u8).collect(),
        }
    }

    #[test]
    fn a_picture_is_one_whose_planes_are_exactly_its_size() {
        assert!(picture(4, 2).is_picture(4, 2));
        assert!(picture(5, 3).is_picture(5, 3), "odd sides round the chroma up");
        assert!(!picture(4, 2).is_picture(4, 4));
        let mut short = picture(4, 2);
        short.v.pop();
        assert!(!short.is_picture(4, 2));
    }

    #[test]
    fn a_picture_is_written_into_a_biplanar_surface_row_by_row_leaving_the_padding() {
        let frame = picture(4, 2);
        let (mut luma, mut chroma) = (vec![0xEE; 6 * 2], vec![0xEE; 6]);
        frame.write_biplanar(4, 2, (&mut luma, 6), (&mut chroma, 6)).unwrap();
        assert_eq!(luma, [0, 1, 2, 3, 0xEE, 0xEE, 4, 5, 6, 7, 0xEE, 0xEE]);
        assert_eq!(chroma, [100, 200, 101, 201, 0xEE, 0xEE]);
    }

    #[test]
    fn a_surface_too_small_for_the_picture_is_refused_and_left_alone() {
        let frame = picture(4, 2);
        for (luma_len, luma_stride, chroma_len, chroma_stride) in [(7, 4, 4, 4), (8, 3, 4, 4), (8, 4, 3, 4), (8, 4, 4, 3)] {
            let (mut luma, mut chroma) = (vec![0; luma_len], vec![0; chroma_len]);
            let error = frame.write_biplanar(4, 2, (&mut luma, luma_stride), (&mut chroma, chroma_stride)).unwrap_err();
            assert_eq!(error.to_string(), "the encoder's surface is smaller than the picture", "{luma_len} {luma_stride} {chroma_len} {chroma_stride}");
            assert!(luma.iter().chain(&chroma).all(|&byte| byte == 0));
        }
        let (mut luma, mut chroma) = (vec![0; 8], vec![0; 4]);
        assert!(frame.write_biplanar(4, 4, (&mut luma, 4), (&mut chroma, 4)).unwrap_err().to_string().contains("not those of a 4x4 picture"));
    }

    #[test]
    fn a_last_row_needs_only_its_own_bytes_in_the_surface() {
        let frame = picture(4, 2);
        let (mut luma, mut chroma) = (vec![0; 6 + 4], vec![0; 4]);
        frame.write_biplanar(4, 2, (&mut luma, 6), (&mut chroma, 4)).unwrap();
        assert_eq!(luma.get(6..), Some(&[4, 5, 6, 7][..]));
    }

    #[test]
    fn frames_that_arrive_out_of_order_leave_in_order() {
        let mut order = Reorder::new(4);
        assert!(order.push(2, 'c').unwrap().is_empty());
        assert!(order.push(3, 'd').unwrap().is_empty());
        assert_eq!(order.push(0, 'a').unwrap(), [(0, 'a')]);
        assert_eq!(order.push(1, 'b').unwrap(), [(1, 'b'), (2, 'c'), (3, 'd')]);
        assert!(order.is_complete());
        assert_eq!(order.passed(), 4);
    }

    #[test]
    fn a_frame_outside_the_movie_or_seen_before_is_refused() {
        let mut order = Reorder::new(3);
        order.push(0, 'a').unwrap();
        order.push(2, 'c').unwrap();
        assert_eq!(order.push(3, 'x').unwrap_err().to_string(), "frame 3 arrived in a movie of 3 frames");
        assert_eq!(order.push(0, 'x').unwrap_err().to_string(), "frame 0 arrived twice", "one already passed on");
        assert_eq!(order.push(2, 'x').unwrap_err().to_string(), "frame 2 arrived twice", "one still waiting");
        assert!(!order.is_complete());
    }
}
