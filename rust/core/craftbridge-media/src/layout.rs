//! What a hardware decoder needs to know about a stream to set itself up, and what every picture it
//! returns carries: the surface format to ask for (VideoToolbox's; MediaCodec's is always `YUV_420_888`),
//! the part of the surface that is the picture, and the colour and pixel aspect FilmCraft's software
//! decoder reports for the same stream. Streams the hardware
//! decoders here do not take (interlaced, 4:2:2 or 4:4:4, other bit depths) are refused up front, so they
//! go to the software decoder before a session is made.
//!
//! Safe code, so every target tests it.

use filmcraft_codecs::hw::{FrameCodec, FrameStreamInfo, NalCodec, NalStreamInfo, StreamInfo};
use filmcraft_color::ColorInfo;
#[cfg(any(target_os = "ios", test))]
use filmcraft_color::Range;
use filmcraft_frame::{PixelData, VideoFrame};

use crate::biplanar::{Crop, Depth};

/// The longest side a picture may claim. A stream's size comes from its configuration record, so it is
/// bounded before any session or buffer is sized from it; 8192 is the widest picture the phones'
/// hardware decoders take.
const MAX_SIDE: u32 = 8192;

/// The most pictures that may precede a picture in decoding order and follow it in output order: H.264 and
/// HEVC both bound the decoded picture buffer at 16 pictures.
const MAX_REORDER: usize = 16;

/// The codec of a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Codec {
    H264,
    Hevc,
    Av1,
}

impl Codec {
    /// The codec as every hardware decoder's name gives it, after its backend's.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::Hevc => "HEVC",
            Self::Av1 => "AV1",
        }
    }

    /// The four-character code CoreMedia names the codec by.
    #[cfg(any(target_os = "ios", test))]
    pub(crate) fn fourcc(self) -> u32 {
        u32::from_be_bytes(match self {
            Self::H264 => *b"avc1",
            Self::Hevc => *b"hvc1",
            Self::Av1 => *b"av01",
        })
    }
}

#[cfg(any(target_os = "ios", test))]
/// A surface format: biplanar 4:2:0, 8-bit or 10-bit, video or full range. The 10-bit formats keep each
/// sample in the high bits of a 16-bit word.
fn pixel_format(depth: Depth, full_range: bool) -> u32 {
    u32::from_be_bytes(match (depth, full_range) {
        (Depth::Eight, false) => *b"420v",
        (Depth::Eight, true) => *b"420f",
        (Depth::Ten, false) => *b"x420",
        (Depth::Ten, true) => *b"xf20",
    })
}

/// How a stream is decoded, and what its pictures are.
#[derive(Clone, Debug)]
pub(crate) struct Layout {
    pub(crate) codec: Codec,
    pub(crate) depth: Depth,
    /// The surface format the decoder is asked to produce: the stream's own range, so no sample is rescaled
    /// on the way out.
    #[cfg(any(target_os = "ios", test))]
    pub(crate) pixel_format: u32,
    /// The decoded (coded) size in luma samples.
    #[cfg(any(target_os = "ios", test))]
    coded: (usize, usize),
    /// The output rectangle in luma samples.
    crop: Crop,
    /// Pictures that may precede a picture in decoding order and follow it in output order.
    #[cfg(any(target_os = "ios", test))]
    pub(crate) reorder: usize,
    color: ColorInfo,
    par: (u32, u32),
}

fn side(value: u32) -> Option<usize> {
    (1..=MAX_SIDE).contains(&value).then_some(value as usize)
}

impl Layout {
    /// The layout of `stream`, or why no hardware decoder here takes it.
    pub(crate) fn for_stream(stream: &StreamInfo) -> Result<Self, String> {
        match stream {
            StreamInfo::Nal(info) => Self::for_nal(info),
            StreamInfo::Frame(info) => Self::for_frame(info),
        }
    }

    fn for_nal(info: &NalStreamInfo) -> Result<Self, String> {
        let codec = match info.codec {
            NalCodec::H264 => Codec::H264,
            NalCodec::Hevc => Codec::Hevc,
        };
        if info.interlaced {
            return Err("field-coded video".into());
        }
        if info.chroma_format_idc != 1 {
            return Err(format!("chroma_format_idc {}", info.chroma_format_idc));
        }
        if info.reorder > MAX_REORDER {
            return Err(format!("a reorder depth of {}", info.reorder));
        }
        let depth = depth_of(info.bit_depth_luma, info.bit_depth_chroma)?;
        let (crop_x, crop_y, crop_width, crop_height) = info.crop;
        let coded = (side(info.coded.0), side(info.coded.1));
        let (Some(coded_width), Some(coded_height), Some(width), Some(height)) = (coded.0, coded.1, side(crop_width), side(crop_height)) else {
            return Err(format!("a {crop_width}x{crop_height} picture in {}x{}", info.coded.0, info.coded.1));
        };
        let (x, y) = (crop_x as usize, crop_y as usize);
        if x.checked_add(width).is_none_or(|end| end > coded_width) || y.checked_add(height).is_none_or(|end| end > coded_height) {
            return Err(format!("a {crop_width}x{crop_height} picture at {crop_x},{crop_y} outside {coded_width}x{coded_height}"));
        }
        Ok(Self {
            codec,
            depth,
            #[cfg(any(target_os = "ios", test))]
            pixel_format: pixel_format(depth, info.color.range == Range::Full),
            #[cfg(any(target_os = "ios", test))]
            coded: (coded_width, coded_height),
            crop: Crop { x, y, width, height },
            #[cfg(any(target_os = "ios", test))]
            reorder: info.reorder,
            color: info.color,
            par: info.par,
        })
    }

    fn for_frame(info: &FrameStreamInfo) -> Result<Self, String> {
        if info.codec != FrameCodec::Av1 {
            return Err("VP9".into());
        }
        if info.mono || info.subsampling != (1, 1) {
            return Err("AV1 other than 4:2:0".into());
        }
        let depth = depth_of(info.bit_depth, info.bit_depth)?;
        // The software decoder reports the colour its sequence header holds; a stream without one is
        // left to it.
        let color = info.color.ok_or("an AV1 stream with no colour description")?;
        let (Some(width), Some(height)) = (side(info.size.0), side(info.size.1)) else {
            return Err(format!("a {}x{} picture", info.size.0, info.size.1));
        };
        Ok(Self {
            codec: Codec::Av1,
            depth,
            #[cfg(any(target_os = "ios", test))]
            pixel_format: pixel_format(depth, color.range == Range::Full),
            #[cfg(any(target_os = "ios", test))]
            coded: (width, height),
            crop: Crop { x: 0, y: 0, width, height },
            // A temporal unit's one shown picture comes out in presentation order.
            #[cfg(any(target_os = "ios", test))]
            reorder: 0,
            color,
            par: (1, 1),
        })
    }

    /// Where the picture is in a surface of `width` x `height` luma samples: a surface of the picture's own
    /// size holds just it, one of the coded size holds it at its crop. Any other size is not this stream's.
    #[cfg(any(target_os = "ios", test))]
    pub(crate) fn crop_in(&self, width: usize, height: usize) -> Option<Crop> {
        if (width, height) == (self.crop.width, self.crop.height) {
            Some(Crop { x: 0, y: 0, ..self.crop })
        } else if (width, height) == self.coded {
            Some(self.crop)
        } else {
            None
        }
    }

    /// The picture's size in luma samples.
    #[cfg(any(target_os = "android", test))]
    pub(crate) fn size(&self) -> (usize, usize) {
        (self.crop.width, self.crop.height)
    }

    /// The picture's size as the signed integers the platforms' frameworks take; `None` when a side does not
    /// fit one, which a picture bounded by [`MAX_SIDE`] never is.
    pub(crate) fn size_i32(&self) -> Option<(i32, i32)> {
        Some((i32::try_from(self.crop.width).ok()?, i32::try_from(self.crop.height).ok()?))
    }

    /// The frame around `pixels` of the picture at `crop`, as the software decoder makes it: colour and pixel
    /// aspect from the stream, and no presentation time of its own (the decoded picture carries it).
    pub(crate) fn frame(&self, pixels: PixelData) -> VideoFrame {
        VideoFrame {
            width: u32::try_from(self.crop.width).unwrap_or(u32::MAX),
            height: u32::try_from(self.crop.height).unwrap_or(u32::MAX),
            data: pixels,
            color: self.color,
            par: self.par,
            pts: Default::default(),
        }
    }
}

fn depth_of(luma: u32, chroma: u32) -> Result<Depth, String> {
    match (luma, chroma) {
        (8, 8) => Ok(Depth::Eight),
        (10, 10) => Ok(Depth::Ten),
        _ => Err(format!("{luma}-bit luma and {chroma}-bit chroma")),
    }
}

#[cfg(test)]
mod tests {
    use filmcraft_color::ColorInfo;

    use super::*;
    use crate::fixtures::avc_entry;

    fn nal() -> NalStreamInfo {
        NalStreamInfo::from_entry(&avc_entry()).unwrap().unwrap()
    }

    fn av1() -> FrameStreamInfo {
        let mut info = FrameStreamInfo::for_tests(FrameCodec::Av1, 0, 10, (1, 1), (640, 360));
        info.color = Some(ColorInfo::REC709);
        info
    }

    fn layout(info: NalStreamInfo) -> Result<Layout, String> {
        Layout::for_stream(&StreamInfo::Nal(info))
    }

    #[test]
    fn each_codec_is_named_by_the_four_characters_coremedia_knows_it_by() {
        assert_eq!([Codec::H264, Codec::Hevc, Codec::Av1].map(Codec::fourcc), [0x61766331, 0x68766331, 0x61763031]);
    }

    #[test]
    fn each_codec_has_the_name_every_hardware_decoders_name_gives_it() {
        assert_eq!([Codec::H264, Codec::Hevc, Codec::Av1].map(Codec::name), ["H.264", "HEVC", "AV1"]);
    }

    #[test]
    fn a_progressive_420_stream_is_taken_with_its_own_range_and_size() {
        let info = nal();
        let taken = layout(info.clone()).unwrap();
        assert_eq!((taken.codec, taken.depth, taken.pixel_format), (Codec::H264, Depth::Eight, u32::from_be_bytes(*b"420v")));
        assert_eq!(taken.crop, Crop { x: info.crop.0 as usize, y: info.crop.1 as usize, width: info.crop.2 as usize, height: info.crop.3 as usize });
        assert_eq!(taken.reorder, info.reorder);
        let mut full = info.clone();
        full.color.range = Range::Full;
        assert_eq!(layout(full.clone()).unwrap().pixel_format, u32::from_be_bytes(*b"420f"));
        full.bit_depth_luma = 10;
        full.bit_depth_chroma = 10;
        let ten = layout(full.clone()).unwrap();
        assert_eq!((ten.depth, ten.pixel_format), (Depth::Ten, u32::from_be_bytes(*b"xf20")));
        full.color.range = Range::Limited;
        assert_eq!(layout(full).unwrap().pixel_format, u32::from_be_bytes(*b"x420"));
        let mut hevc = info;
        hevc.codec = NalCodec::Hevc;
        assert_eq!(layout(hevc).unwrap().codec, Codec::Hevc);
    }

    #[test]
    fn streams_no_hardware_decoder_here_takes_are_refused() {
        let info = nal();
        let refused = |change: &dyn Fn(&mut NalStreamInfo)| {
            let mut changed = info.clone();
            change(&mut changed);
            layout(changed).is_err()
        };
        assert!(refused(&|i| i.interlaced = true), "field-coded");
        assert!(refused(&|i| i.chroma_format_idc = 2), "4:2:2");
        assert!(refused(&|i| i.chroma_format_idc = 3), "4:4:4");
        assert!(refused(&|i| i.chroma_format_idc = 0), "monochrome");
        assert!(refused(&|i| i.bit_depth_luma = 12), "12-bit luma");
        assert!(refused(&|i| i.bit_depth_chroma = 10), "luma and chroma of different depths");
        assert!(refused(&|i| i.reorder = MAX_REORDER + 1), "a reorder depth no stream has");
        assert!(refused(&|i| i.crop.2 = 0), "an empty picture");
        assert!(refused(&|i| i.crop.3 = MAX_SIDE + 1), "a picture taller than the hardware decodes");
        assert!(refused(&|i| i.crop.0 = i.coded.0), "a crop outside the coded picture");
        assert!(refused(&|i| i.crop.0 = u32::MAX), "a crop that overflows");
        assert!(!refused(&|i| i.reorder = MAX_REORDER));
    }

    #[test]
    fn an_av1_stream_is_taken_when_it_is_420_with_a_colour_description_and_vp9_is_not() {
        let taken = Layout::for_stream(&StreamInfo::Frame(av1())).unwrap();
        assert_eq!((taken.codec, taken.depth, taken.reorder, taken.par), (Codec::Av1, Depth::Ten, 0, (1, 1)));
        assert_eq!(taken.crop, Crop { x: 0, y: 0, width: 640, height: 360 });
        let refused = |change: &dyn Fn(&mut FrameStreamInfo)| {
            let mut changed = av1();
            change(&mut changed);
            Layout::for_stream(&StreamInfo::Frame(changed)).is_err()
        };
        assert!(refused(&|i| i.codec = FrameCodec::Vp9), "VP9");
        assert!(refused(&|i| i.mono = true), "monochrome");
        assert!(refused(&|i| i.subsampling = (1, 0)), "4:2:2");
        assert!(refused(&|i| i.bit_depth = 12), "12-bit");
        assert!(refused(&|i| i.color = None), "no colour description");
        assert!(refused(&|i| i.size = (0, 360)), "an empty picture");
    }

    #[test]
    fn a_surface_is_the_pictures_own_size_or_the_coded_size_and_nothing_else() {
        let mut info = nal();
        info.coded = (1920, 1088);
        info.crop = (0, 0, 1920, 1080);
        let taken = layout(info).unwrap();
        assert_eq!(taken.crop_in(1920, 1080), Some(Crop { x: 0, y: 0, width: 1920, height: 1080 }));
        assert_eq!(taken.crop_in(1920, 1088), Some(Crop { x: 0, y: 0, width: 1920, height: 1080 }));
        assert_eq!(taken.crop_in(1280, 720), None);
        let mut offset = nal();
        offset.coded = (64, 64);
        offset.crop = (8, 4, 32, 32);
        let taken = layout(offset).unwrap();
        assert_eq!(taken.crop_in(32, 32), Some(Crop { x: 0, y: 0, width: 32, height: 32 }));
        assert_eq!(taken.crop_in(64, 64), Some(Crop { x: 8, y: 4, width: 32, height: 32 }));
    }

    #[test]
    fn a_pictures_size_is_given_as_the_signed_integers_the_frameworks_take_when_it_fits_them() {
        let mut taken = layout(nal()).unwrap();
        assert_eq!(taken.size_i32(), Some((nal().crop.2 as i32, nal().crop.3 as i32)));
        taken.crop.width = i32::MAX as usize + 1;
        assert_eq!(taken.size_i32(), None);
        taken.crop.width = 1;
        taken.crop.height = usize::MAX;
        assert_eq!(taken.size_i32(), None);
    }

    #[test]
    fn a_frame_carries_the_streams_colour_and_pixel_aspect_and_the_pictures_size() {
        let mut info = nal();
        info.par = (4, 3);
        info.color.range = Range::Full;
        let taken = layout(info.clone()).unwrap();
        let pixels = PixelData::Rgba8(Default::default());
        let frame = taken.frame(pixels);
        assert_eq!((frame.width, frame.height, frame.par, frame.color), (info.crop.2, info.crop.3, (4, 3), info.color));
        assert_eq!(frame.pts, Default::default());
        assert_eq!(taken.size(), (info.crop.2 as usize, info.crop.3 as usize));
    }
}
