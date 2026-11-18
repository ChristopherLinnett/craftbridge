//! What MediaCodec is told about a stream, and the bytes it is fed: which streams the Android decoder takes,
//! which of the device's decoders count as hardware, and the Annex B byte stream MediaCodec reads where an
//! MP4 sample holds length-prefixed NAL units; and when the end of a stream has been drained.
//!
//! Safe code, so every target tests it; the platform module only makes the calls.

use std::time::{Duration, Instant};

use filmcraft_codecs::hw::{NalCodec, NalStreamInfo, StreamInfo};
use filmcraft_isobmff::SampleEntry;

use crate::biplanar::{Crop, Depth, Plane, Strided};
// Re-exported: the Android decoder and its encoders name the codec list's entry through this module.
pub(crate) use crate::codec_list::CodecInfo;
use crate::layout::{self, Codec, Layout};

/// What every NAL unit of an Annex B byte stream starts with.
const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// The NAL unit type of an H.264 picture parameter set.
const H264_PPS: u8 = 8;

/// MediaCodec's names for the codecs this module takes.
pub(crate) const H264_MIME: &str = "video/avc";
pub(crate) const HEVC_MIME: &str = "video/hevc";
pub(crate) const AV1_MIME: &str = "video/av01";

/// Every MIME type a decoder is taken for, which is what [`crate::register`] asks the device about.
#[cfg(any(target_os = "android", test))]
pub(crate) const BACKEND_MIMES: [&str; 3] = [H264_MIME, HEVC_MIME, AV1_MIME];

/// The MIME type MediaCodec names the decoder of `layout`'s stream by, or why the Android decoder does not
/// take it. A 10-bit H.264 or HEVC picture is taken too, but reaches CraftBridge only through the P010 reader
/// ([`reader_format`]): where the platform makes none, no decoder starts, and where a device decodes into any
/// other buffer the picture's read fails; either way the software decoder takes the stream. 10-bit AV1 is
/// left to the software decoder, since no 10-bit AV1 stream has been run through the P010 reader.
pub(crate) fn mime(layout: &Layout) -> Result<&'static str, String> {
    match (layout.codec, layout.depth) {
        (Codec::Av1, Depth::Ten) => Err("10-bit video".into()),
        (Codec::H264, _) => Ok(H264_MIME),
        (Codec::Hevc, _) => Ok(HEVC_MIME),
        (Codec::Av1, Depth::Eight) => Ok(AV1_MIME),
    }
}

/// The image reader a decoder renders its pictures into, which is also where their samples are read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReaderFormat {
    /// `AIMAGE_FORMAT_YUV_420_888`, which carries 8 bits a sample: a picture is read from the image's own
    /// planes.
    Yuv420,
    /// `AHARDWAREBUFFER_FORMAT_YCbCr_P010`, 10 bits in the high bits of 16-bit words, which no NDK image plane
    /// carries: a picture is read from the image's hardware buffer, locked for the CPU. The NDK's image
    /// formats have no 10-bit YUV one; `AImageReader` takes the HAL format of this number from Android 14
    /// (API 34) and refuses it before.
    P010,
}

/// The reader pictures of `depth` are decoded into. A 10-bit picture has the P010 reader alone, so before
/// Android 14 no decoder starts for it and the software decoder takes the stream from its first sample. The
/// `YUV_420_888` reader is no way round that: Android 10 and 11 hand it a producer's P010 buffer, but lock
/// such a buffer as one plane, not as P010's planes (Android 13 is the first that does), and from Android 12
/// the reader refuses the buffer, so a decoder started with it would fail at its first picture.
pub(crate) fn reader_format(depth: Depth) -> ReaderFormat {
    match depth {
        Depth::Eight => ReaderFormat::Yuv420,
        Depth::Ten => ReaderFormat::P010,
    }
}

/// The `color-format` a decoder is asked to write the pictures of `format`'s reader in, by
/// `MediaCodecInfo.CodecCapabilities`' numbers: `COLOR_FormatYUV420Flexible` for the 8-bit reader and
/// `COLOR_FormatYUVP010` for the 10-bit one. A decoder writing to a surface and asked for none writes a layout of
/// its own choosing, and the acceptance phone's Qualcomm decoders choose a compressed one the CPU cannot read,
/// whatever reader the surface is; asked for one of these, they write pictures the reader's planes describe.
pub(crate) fn color_format(format: ReaderFormat) -> i32 {
    match format {
        ReaderFormat::Yuv420 => 0x7F42_0888,
        ReaderFormat::P010 => 54,
    }
}

/// The row and pixel strides of the three planes of an image of the 8-bit reader, `planes` planes of `strides`,
/// when they describe planes that can be read; an error naming the image's `format` when they do not. The
/// platform describes a linear 4:2:0 picture so whatever format code it carries (Qualcomm's decoders give their own
/// linear NV12 one, not `YUV_420_888`'s). It gives a picture it cannot describe, such as the compressed one those
/// decoders write when asked for no colour format, chroma planes with no strides and planes with no memory, so
/// such a picture is refused before any plane of it is read: the platform answers a read of such a plane with no
/// address, of which the bindings would make a slice.
pub(crate) fn described_planes(format: i32, planes: i32, strides: [(i32, i32); 3]) -> Result<[(i32, i32); 3], String> {
    if planes != 3 || strides.iter().any(|&(row, pixel)| row <= 0 || pixel <= 0) {
        return Err(format!("the decoded picture, in format {format:#x}, is not planes the platform describes: {planes} planes, strides {strides:?}"));
    }
    Ok(strides)
}

/// What a decoder's description of a crop that starts off the coded picture's corner cannot be told from, in
/// words, for the decoder's error and the tests that pin it.
pub(crate) const IGNORED_OFFSET: &str = "a decoder that ignores a crop's offset describes its picture alike";

/// Where the picture is in a decoded image whose crop rectangle is `rect` (left, top, right, bottom): the
/// platform's own description of where the decoder put the picture, which must be the stream's own picture
/// size, since the image reader was made for it. It is not held to the image's own width and height, which can
/// be the picture's size alone while a crop at the stream's offset runs past them, the planes holding the coded
/// picture. For a stream whose crop starts at the coded picture's corner the rectangle is taken wherever it
/// starts: there is no offset a decoder could ignore. For one whose crop starts off the corner it is taken only
/// at that corner. A decoder that ignores a crop's offset writes the coded picture from where it starts its
/// picture and describes a picture of the stream's size there, the image's origin unless it pads the image,
/// which is also how a decoder that crops for itself describes the window it wrote, in an image of the same
/// size; so a description at any other corner cannot be told from the coded picture's corner, and reading the
/// wrong one gives another window of the picture with no error. The stream is refused, and the software
/// decoder takes it.
pub(crate) fn crop_of(rect: (i32, i32, i32, i32), layout: &Layout) -> Result<Crop, String> {
    let (left, top, right, bottom) = rect;
    let side = |from: i32, to: i32| to.checked_sub(from).and_then(|length| usize::try_from(length).ok());
    let (width, height) = (side(left, right), side(top, bottom));
    let (Some(width), Some(height), Ok(x), Ok(y)) = (width, height, usize::try_from(left), usize::try_from(top)) else {
        return Err(format!("the decoded picture's crop {rect:?} is not a rectangle in the image"));
    };
    if (width, height) != layout.size() {
        return Err(format!("the decoded picture is {width}x{height}, not the stream's size"));
    }
    let (wanted_x, wanted_y) = layout.offset();
    if (wanted_x, wanted_y) != (0, 0) && (x, y) != (wanted_x, wanted_y) {
        return Err(format!(
            "the decoded picture's crop starts at {x},{y}, not where the stream's crop starts, {wanted_x},{wanted_y}: {IGNORED_OFFSET}, so it cannot be told from the coded picture's corner"
        ));
    }
    Ok(Crop { x, y, width, height })
}

/// Plane `index` of a decoded image as the planar reader takes it, from what the image reports of it; an
/// error for a negative stride, which no plane has.
pub(crate) fn plane(index: i32, data: &[u8], row_stride: i32, pixel_stride: i32) -> Result<Strided<'_>, String> {
    match (usize::try_from(row_stride), usize::try_from(pixel_stride)) {
        (Ok(stride), Ok(pixel)) => Ok(Strided { data, stride, pixel }),
        _ => Err(format!("the decoded picture's plane {index} has a negative stride")),
    }
}

/// The luma plane as the planar reader takes it: its samples must sit one byte apart.
pub(crate) fn luma(plane: Strided<'_>) -> Result<Plane<'_>, &'static str> {
    if plane.pixel != 1 {
        return Err("the decoded picture's luma samples are not contiguous");
    }
    Ok(Plane { data: plane.data, stride: plane.stride })
}

/// How far every presentation time is moved up before MediaCodec gets it, so that none is negative. MediaCodec
/// takes times as unsigned numbers, and a decoder may hand each picture the earliest time it holds rather than
/// its own sample's (the emulator's does), so a negative time would sort after every positive one and label
/// pictures with the wrong times. A stream's times go below zero where its composition offsets do.
const TIME_OFFSET: i64 = 1 << 40;

/// The latest time MediaCodec is given: a rendered picture carries its time in nanoseconds, a thousand times
/// the number, which must not overflow.
const LATEST_TIME: i64 = i64::MAX / 1000;

/// The time MediaCodec is given for a sample presented at `pts`: positive, and in the same order as the
/// stream's own times, whether compared as signed or unsigned numbers.
pub(crate) fn codec_time(pts: i64) -> Result<u64, &'static str> {
    let time = pts.checked_add(TIME_OFFSET).filter(|time| (0..=LATEST_TIME).contains(time));
    time.and_then(|time| u64::try_from(time).ok()).ok_or("the sample's presentation time is outside what MediaCodec carries")
}

/// The presentation time of the picture MediaCodec gives `time`, which [`codec_time`] made.
pub(crate) fn stream_time(time: i64) -> i64 {
    time.saturating_sub(TIME_OFFSET)
}

/// The device's hardware decoders for `mime`, in the order the device lists them. A decoder counts as
/// hardware by the platform's own answer, never by its name: neither a software-only codec nor a vendor's
/// software one (not software-only, and not hardware-accelerated either: what the NDK, from API 36, calls
/// `AMediaCodecType_SOFTWARE_WITH_DEVICE_ACCESS` in `NdkMediaCodecInfo.h`) decodes faster than FilmCraft's
/// software decoder, which decodes the same way on every device. The platform's own default for a type is
/// whatever the list ranks first for it, which can be a software codec (on the API 29 emulator, which lists
/// no hardware H.264 decoder, it is), so the pick is made here, never taken from the default.
pub(crate) fn hardware_decoders<'a>(codecs: &'a [CodecInfo], mime: &str) -> Vec<&'a str> {
    hardware_codecs(codecs, mime, false)
}

/// The device's hardware encoders for `mime`, in the order the device lists them, which is what the
/// hardware export encodes with (plan §18). The rule is the decoders' own, the platform's
/// `isHardwareAccelerated()` answer and never the name, for the same kind of reason: what the export is
/// here for is what the device's silicon does better, and a software encoder is what EffectCraft's own
/// encoders already are, so a device whose answer lists none leaves the export to EffectCraft rather than
/// encoding the movie twice in the same software.
pub(crate) fn hardware_encoders<'a>(codecs: &'a [CodecInfo], mime: &str) -> Vec<&'a str> {
    hardware_codecs(codecs, mime, true)
}

fn hardware_codecs<'a>(codecs: &'a [CodecInfo], mime: &str, encoder: bool) -> Vec<&'a str> {
    // MIME types compare without regard to case (RFC 2045, section 5.1), as the platform's own lookup by type
    // compares them.
    let takes = |codec: &CodecInfo| codec.types.iter().any(|kind| kind.eq_ignore_ascii_case(mime));
    codecs.iter().filter(|codec| codec.encoder == encoder && codec.hardware_accelerated && takes(codec)).map(|codec| codec.name.as_str()).collect()
}

/// The keys of the codec-specific data MediaCodec starts a decoder with.
pub(crate) const CSD_0: &str = "csd-0";
pub(crate) const CSD_1: &str = "csd-1";

/// The stream's codec-specific data as the buffers Android's table gives the codec: for H.264 and HEVC the
/// parameter sets, each after a start code (H.264's SPS in `csd-0` and its PPS in `csd-1`; HEVC's VPS, SPS
/// and PPS together in `csd-0`), in record order; for AV1 the `av1C` record itself in `csd-0`.
pub(crate) fn codec_specific_data(entry: &SampleEntry, stream: &StreamInfo) -> Result<Vec<(&'static str, Vec<u8>)>, &'static str> {
    match stream {
        StreamInfo::Nal(info) => Ok(parameter_set_buffers(info)),
        StreamInfo::Frame(_) => Ok(vec![(CSD_0, layout::av1c_record(entry)?)]),
    }
}

fn parameter_set_buffers(info: &NalStreamInfo) -> Vec<(&'static str, Vec<u8>)> {
    let joined = |sets: &[&Vec<u8>]| -> Vec<u8> { sets.iter().flat_map(|set| START_CODE.iter().chain(set.iter())).copied().collect() };
    let sets: Vec<&Vec<u8>> = info.parameter_sets.iter().collect();
    match info.codec {
        NalCodec::Hevc => vec![(CSD_0, joined(&sets))],
        NalCodec::H264 => {
            let (pictures, sequences): (Vec<_>, Vec<_>) =
                sets.into_iter().partition(|set| set.first().is_some_and(|&header| info.nal_type(header) == H264_PPS));
            vec![(CSD_0, joined(&sequences)), (CSD_1, joined(&pictures))]
        }
    }
}

/// A sample as MediaCodec is fed it: H.264 and HEVC as Annex B ([`annex_b`]); AV1 as it is, since its
/// samples are OBU temporal units, which is the form the Android decoder reads. An error for a sample with
/// nothing in it.
pub(crate) fn input(stream: &StreamInfo, sample: &[u8]) -> Result<Vec<u8>, &'static str> {
    match stream {
        StreamInfo::Nal(info) => annex_b(info, sample),
        StreamInfo::Frame(_) if sample.is_empty() => Err("the sample is empty"),
        StreamInfo::Frame(_) => Ok(sample.to_vec()),
    }
}

/// A length-prefixed sample as an Annex B byte stream. An error when the sample does not hold whole NAL
/// units end to end, which FilmCraft's own reader of them would stop at without saying so.
pub(crate) fn annex_b(info: &NalStreamInfo, sample: &[u8]) -> Result<Vec<u8>, &'static str> {
    let units = info.nals(sample);
    let covered = units.iter().try_fold(0usize, |sum, unit| sum.checked_add(unit.len())?.checked_add(info.length_size));
    if units.is_empty() || units.iter().any(|unit| unit.is_empty()) || covered != Some(sample.len()) {
        return Err("the sample is not whole NAL units");
    }
    Ok(units.into_iter().flat_map(|unit| START_CODE.iter().chain(unit)).copied().collect())
}

/// The pictures a codec gives at the end of a stream: what `queue_the_end` returns (the pictures ready
/// when the end of the stream is queued), then what each `read` returns (the pictures ready, and whether the
/// end of the stream came out with them), until the end comes. A codec that has not ended the stream within
/// `stall` of its end being queued may still hold pictures nothing will ask it for again, so that is an
/// error, as a failed call is: the decoder's caller then replays the stream in software rather than end it
/// short. Every failure of a drain is decided here, so the host tests pin them all.
pub(crate) fn until_the_end<T>(
    stall: Duration,
    who: &str,
    queue_the_end: impl FnOnce() -> Result<Vec<T>, String>,
    mut read: impl FnMut() -> Result<(Vec<T>, bool), String>,
) -> Result<Vec<T>, String> {
    let mut pictures = queue_the_end()?;
    let deadline = Instant::now() + stall;
    loop {
        let (more, end) = read()?;
        pictures.extend(more);
        if end {
            return Ok(pictures);
        }
        if Instant::now() >= deadline {
            return Err(format!("the {who} never ended the stream"));
        }
    }
}

#[cfg(test)]
mod tests {
    use filmcraft_codecs::hw::{FrameCodec, FrameStreamInfo, NalCodec};
    use filmcraft_codecs::software_video_decoder;
    use filmcraft_color::ColorInfo;
    use filmcraft_frame::PixelData;

    use super::*;
    use crate::fallback::stream_info;
    use crate::fixtures::{av1_entry, av1_samples, avc_entry, hevc_entry, hevc10_entry, hevc10_samples, pictures, pps, sps};

    fn nal() -> NalStreamInfo {
        NalStreamInfo::from_entry(&avc_entry()).unwrap().unwrap()
    }

    fn av1() -> StreamInfo {
        FrameStreamInfo::from_entry(&av1_entry()).unwrap().unwrap().into()
    }

    fn layout(change: impl FnOnce(&mut NalStreamInfo)) -> Layout {
        let mut info = nal();
        change(&mut info);
        Layout::for_stream(&StreamInfo::Nal(info)).unwrap()
    }

    #[test]
    fn each_codec_taken_is_named_by_the_mime_type_mediacodec_knows_it_by() {
        // MediaCodec's own words for each.
        assert_eq!(mime(&layout(|_| {})), Ok("video/avc"));
        assert_eq!(mime(&layout(|info| info.codec = NalCodec::Hevc)), Ok("video/hevc"));
        assert_eq!(mime(&Layout::for_stream(&av1()).unwrap()), Ok("video/av01"));
        let taken = [layout(|_| {}), layout(|info| info.codec = NalCodec::Hevc), Layout::for_stream(&av1()).unwrap()];
        assert_eq!(taken.iter().map(|layout| mime(layout).unwrap()).collect::<Vec<_>>(), BACKEND_MIMES, "the device is asked about every MIME type taken");
    }

    #[test]
    fn ten_bit_h264_and_hevc_are_taken_with_their_codecs_mime_type_and_ten_bit_av1_is_left_to_the_software_decoder() {
        let ten = layout(|info| {
            info.bit_depth_luma = 10;
            info.bit_depth_chroma = 10;
        });
        assert_eq!(mime(&ten), Ok("video/avc"), "10-bit H.264");
        let ten_hevc = layout(|info| {
            info.codec = NalCodec::Hevc;
            info.bit_depth_luma = 10;
            info.bit_depth_chroma = 10;
        });
        assert_eq!(mime(&ten_hevc), Ok("video/hevc"), "10-bit HEVC");
        let mut ten_av1 = FrameStreamInfo::for_tests(FrameCodec::Av1, 0, 10, (1, 1), (640, 360));
        ten_av1.color = Some(ColorInfo::REC709);
        assert_eq!(mime(&Layout::for_stream(&StreamInfo::Frame(ten_av1)).unwrap()), Err("10-bit video".to_string()), "10-bit AV1");
    }

    #[test]
    fn the_ten_bit_fixture_is_a_main_10_hevc_stream_whose_pictures_the_software_decoder_gives_in_ten_bits() {
        let entry = hevc10_entry();
        let stream = stream_info(&entry).unwrap().unwrap();
        let layout = Layout::for_stream(&stream).unwrap();
        assert_eq!((layout.codec, layout.depth, mime(&layout)), (Codec::Hevc, Depth::Ten, Ok("video/hevc")));
        let StreamInfo::Nal(info) = &stream else { panic!("an hvcC stream") };
        let samples = hevc10_samples();
        assert!(samples.iter().all(|(_, sample)| annex_b(info, sample).is_ok()), "whole NAL units, as MediaCodec is fed them");
        let mut software = software_video_decoder(&entry).unwrap();
        let frames = pictures(software.as_mut(), &samples);
        let bits: Vec<_> = frames.iter().map(|frame| matches!(frame.frame.data, PixelData::Yuv16 { bits: 10, .. })).collect();
        assert_eq!(bits, vec![true; samples.len()]);
    }

    #[test]
    fn an_8_bit_picture_is_decoded_into_a_yuv_420_888_reader_and_a_10_bit_one_into_the_p010_reader_alone() {
        assert_eq!(reader_format(Depth::Eight), ReaderFormat::Yuv420);
        assert_eq!(reader_format(Depth::Ten), ReaderFormat::P010);
    }

    #[test]
    fn each_reader_asks_its_decoder_for_the_colour_format_its_planes_describe() {
        // `MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible` and `COLOR_FormatYUVP010`.
        assert_eq!(color_format(ReaderFormat::Yuv420), 0x7F42_0888);
        assert_eq!(color_format(ReaderFormat::P010), 54);
    }

    #[test]
    fn an_8_bit_image_is_read_only_as_three_planes_the_platform_describes_whatever_its_format_code() {
        // The acceptance phone's Qualcomm decoders write their own linear NV12 (0x7FA30C04): chroma interleaved two
        // bytes apart, every row 256 bytes. A planar YUV_420_888 image is read as well.
        let nv12 = [(256, 1), (256, 2), (256, 2)];
        assert_eq!(described_planes(0x7FA3_0C04, 3, nv12), Ok(nv12));
        let planar = [(160, 1), (80, 1), (80, 1)];
        assert_eq!(described_planes(0x23, 3, planar), Ok(planar));
        // Their compressed layout (0x7FA30C06), which they write when asked for no colour format, has chroma planes
        // the platform gives no strides.
        assert_eq!(
            described_planes(0x7FA3_0C06, 3, [(256, 1), (0, 0), (0, 0)]),
            Err("the decoded picture, in format 0x7fa30c06, is not planes the platform describes: 3 planes, strides [(256, 1), (0, 0), (0, 0)]".into())
        );
        for (planes, strides) in
            [(1, nv12), (4, nv12), (3, [(256, 1), (256, 0), (256, 2)]), (3, [(-256, 1), (256, 2), (256, 2)]), (3, [(256, 1), (256, 2), (0, 2)])]
        {
            assert!(described_planes(0x23, planes, strides).is_err(), "{planes} planes, strides {strides:?}");
        }
    }

    #[test]
    fn a_decoded_images_crop_is_the_streams_size_at_a_non_negative_offset_and_nothing_else() {
        let info = nal();
        let taken = layout(|_| {});
        let (width, height) = (info.crop.2 as i32, info.crop.3 as i32);
        assert_eq!(crop_of((0, 0, width, height), &taken), Ok(Crop { x: 0, y: 0, width: width as usize, height: height as usize }));
        assert_eq!(crop_of((4, 2, 4 + width, 2 + height), &taken), Ok(Crop { x: 4, y: 2, width: width as usize, height: height as usize }));
        let size = |width: i32, height: i32| format!("the decoded picture is {width}x{height}, not the stream's size");
        let outside = |rect: (i32, i32, i32, i32)| format!("the decoded picture's crop {rect:?} is not a rectangle in the image");
        for (rect, message) in [
            ((0, 0, width - 1, height), size(width - 1, height)),
            ((0, 0, width, height + 1), size(width, height + 1)),
            ((-2, 0, width - 2, height), outside((-2, 0, width - 2, height))),
            ((0, -2, width, height - 2), outside((0, -2, width, height - 2))),
            ((width, 0, 0, height), outside((width, 0, 0, height))),
            ((i32::MIN, 0, i32::MAX, height), outside((i32::MIN, 0, i32::MAX, height))),
        ] {
            assert_eq!(crop_of(rect, &taken), Err(message), "{rect:?}");
        }
    }

    #[test]
    fn a_crop_that_starts_off_the_corner_is_taken_only_where_the_decoder_describes_that_corner() {
        let offset = |x: u32, y: u32| {
            layout(|info| {
                info.coded = (64, 64);
                info.crop = (x, y, 32, 32);
            })
        };
        let ambiguous = |at: (i32, i32), wanted: (u32, u32)| {
            format!(
                "the decoded picture's crop starts at {},{}, not where the stream's crop starts, {},{}: \
                 a decoder that ignores a crop's offset describes its picture alike, so it cannot be told from the coded picture's corner",
                at.0, at.1, wanted.0, wanted.1
            )
        };
        assert!(ambiguous((0, 0), (8, 4)).contains(IGNORED_OFFSET));
        // A crop 8 in and 4 down, one only across and one only down.
        for (x, y) in [(8, 4), (8, 0), (0, 4)] {
            let window = Crop { x: x as usize, y: y as usize, width: 32, height: 32 };
            let (left, top) = (x as i32, y as i32);
            assert_eq!(crop_of((left, top, left + 32, top + 32), &offset(x, y)), Ok(window), "the corner the stream's crop starts at, {x},{y}");
            // A decoder that ignores the offset describes the picture's size where it starts its picture, the origin
            // unless it pads the image, as one that crops for itself does.
            for at in [(0, 0), (4, 4), (left, 0), (0, top), (left + 2, top)] {
                if at != (left, top) {
                    let rect = (at.0, at.1, at.0 + 32, at.1 + 32);
                    assert_eq!(crop_of(rect, &offset(x, y)), Err(ambiguous(at, (x, y))), "{rect:?} for a crop at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn a_decoded_images_planes_take_their_strides_from_the_image_and_luma_must_be_contiguous() {
        let data = [1u8, 2, 3, 4];
        let chroma = plane(1, &data, 4, 2).unwrap();
        assert_eq!((chroma.data, chroma.stride, chroma.pixel), (&data[..], 4, 2));
        for (row, pixel) in [(-1, 1), (1, -1), (i32::MIN, 1)] {
            assert_eq!(plane(2, &data, row, pixel).err(), Some("the decoded picture's plane 2 has a negative stride".to_string()), "{row} {pixel}");
        }
        let contiguous = luma(plane(0, &data, 4, 1).unwrap()).unwrap();
        assert_eq!((contiguous.data, contiguous.stride), (&data[..], 4));
        assert_eq!(luma(chroma).err(), Some("the decoded picture's luma samples are not contiguous"));
    }

    #[test]
    fn every_presentation_time_reaches_mediacodec_positive_and_in_its_order_and_comes_back_as_it_was() {
        let times = [-TIME_OFFSET, -1001, -2, -1, 0, 1, 3, 90_000 * 3600 * 24, LATEST_TIME - TIME_OFFSET];
        let given: Vec<u64> = times.iter().map(|&pts| codec_time(pts).unwrap()).collect();
        assert!(given.windows(2).all(|pair| pair[0] < pair[1]), "in the stream's order as unsigned numbers: {given:?}");
        let signed: Vec<i64> = given.iter().map(|&time| i64::try_from(time).unwrap()).collect();
        assert!(signed.windows(2).all(|pair| pair[0] < pair[1]), "and as signed ones");
        assert_eq!(signed.iter().map(|&time| stream_time(time)).collect::<Vec<_>>(), times, "each picture's own time back");
        assert!(signed.iter().all(|&time| time.checked_mul(1000).is_some()), "in nanoseconds too");
        for pts in [-TIME_OFFSET - 1, LATEST_TIME - TIME_OFFSET + 1, i64::MIN, i64::MAX] {
            assert_eq!(codec_time(pts), Err("the sample's presentation time is outside what MediaCodec carries"), "{pts}");
        }
    }

    fn codec(name: &str, types: &[&str], encoder: bool, hardware_accelerated: bool) -> CodecInfo {
        CodecInfo { name: name.into(), types: types.iter().map(|kind| kind.to_string()).collect(), encoder, hardware_accelerated }
    }

    #[test]
    fn the_hardware_decoders_are_the_ones_the_platform_says_are_hardware_accelerated_in_the_devices_order() {
        // The API 29 emulator's two H.264 decoders lead, both software, as its list ranks them: the platform's
        // default for the type is the first.
        let list = [
            codec("c2.android.avc.decoder", &["video/avc"], false, false),
            codec("OMX.google.h264.decoder", &["video/avc"], false, false),
            codec("c2.vendor.avc.decoder", &["video/avc"], false, false),
            codec("c2.vendor.avc.encoder", &["video/avc"], true, true),
            codec("c2.android.hevc.decoder", &["video/hevc"], false, false),
            codec("c2.vendor.hevc.decoder", &["Video/HEVC", "video/avc"], false, true),
            codec("c2.vendor.vp9.decoder", &["video/x-vnd.on2.vp9"], false, true),
            codec("hw.avc.decoder", &["video/avc"], false, true),
            codec("c2.android.av1.decoder", &["video/av01"], false, false),
            codec("c2.android.av1-dav1d.decoder", &["video/av01"], false, false),
        ];
        // The vendor's software decoder is left out although its name is a vendor's, and the last decoder is
        // taken although its name starts with neither `OMX.` nor `c2.`: only the platform's answer counts.
        assert_eq!(hardware_decoders(&list, "video/avc"), ["c2.vendor.hevc.decoder", "hw.avc.decoder"]);
        assert_eq!(hardware_decoders(&list, "video/hevc"), ["c2.vendor.hevc.decoder"], "a type's case is not the platform's concern");
        assert_eq!(hardware_decoders(&list, "VIDEO/AVC"), ["c2.vendor.hevc.decoder", "hw.avc.decoder"]);
        assert_eq!(hardware_decoders(&list, "video/av01"), Vec::<&str>::new(), "the platform's software AV1 decoders are declined by its answer, not a name");
        assert_eq!(hardware_decoders(&list[..4], "video/avc"), Vec::<&str>::new(), "software decoders and a hardware encoder only");
        assert_eq!(hardware_decoders(&[], "video/avc"), Vec::<&str>::new());
    }

    #[test]
    fn the_hardware_encoders_are_the_ones_the_platform_says_are_hardware_accelerated_in_the_devices_order() {
        let list = [
            codec("c2.android.avc.decoder", &["video/avc"], false, false),
            codec("c2.android.avc.encoder", &["video/avc"], true, false),
            codec("c2.vendor.avc.encoder", &["video/avc", "video/hevc"], true, true),
            codec("c2.android.hevc.decoder", &["video/hevc"], false, false),
            codec("c2.vendor.hevc.encoder", &["Video/HEVC"], true, true),
            codec("c2.android.aac.encoder", &["audio/mp4a-latm"], true, false),
        ];
        // Only the platform's answer counts, and only of codecs that encode: the decoders and the software
        // encoder are left out although the device lists them first for the type, as it does its decoders.
        assert_eq!(hardware_encoders(&list, "video/avc"), ["c2.vendor.avc.encoder"]);
        assert_eq!(hardware_encoders(&list, "video/hevc"), ["c2.vendor.avc.encoder", "c2.vendor.hevc.encoder"], "in the list's order, a type's case aside");
        assert_eq!(hardware_encoders(&list, "audio/mp4a-latm"), Vec::<&str>::new(), "the AAC encoder is software by the platform's answer");
        assert_eq!(hardware_encoders(&[], "video/avc"), Vec::<&str>::new(), "a device with no list encodes none in hardware");
    }

    #[test]
    fn every_parameter_set_follows_a_start_code_in_record_order_in_the_buffer_androids_table_gives_it() {
        // H.264: the SPS is csd-0 and the PPS csd-1.
        let entry = avc_entry();
        let data = codec_specific_data(&entry, &StreamInfo::Nal(nal())).unwrap();
        let expected = vec![("csd-0", [&[0, 0, 0, 1][..], &sps()].concat()), ("csd-1", [&[0, 0, 0, 1][..], &pps()].concat())];
        assert_eq!(data, expected);
        // Several of either, in the record's order, each after a start code.
        let mut many = nal();
        let (sps2, pps2) = (vec![0x67, 1, 2], vec![0x68, 3]);
        many.parameter_sets = vec![sps(), sps2.clone(), pps(), pps2.clone()];
        let with_code = |sets: &[&[u8]]| -> Vec<u8> { sets.iter().flat_map(|set| [&[0, 0, 0, 1][..], set].concat()).collect() };
        assert_eq!(
            codec_specific_data(&entry, &StreamInfo::Nal(many)).unwrap(),
            vec![("csd-0", with_code(&[&sps(), &sps2])), ("csd-1", with_code(&[&pps(), &pps2]))]
        );
        // HEVC: the VPS, SPS and PPS together in csd-0, which some decoders do without the VPS of and others need.
        let hevc = NalStreamInfo::from_entry(&hevc_entry()).unwrap().unwrap();
        assert_eq!(hevc.parameter_sets.iter().map(|set| (set[0] >> 1) & 0x3f).collect::<Vec<_>>(), [32, 33, 34], "the record's VPS, SPS and PPS");
        let expected: Vec<u8> = hevc.parameter_sets.iter().flat_map(|set| [&[0, 0, 0, 1][..], set].concat()).collect();
        assert_eq!(codec_specific_data(&hevc_entry(), &StreamInfo::Nal(hevc)), Ok(vec![("csd-0", expected)]));
    }

    #[test]
    fn an_av1_streams_codec_specific_data_is_its_av1c_record_and_its_samples_are_fed_as_they_are() {
        let entry = av1_entry();
        let buffers = codec_specific_data(&entry, &av1()).unwrap();
        let [("csd-0", record)] = buffers.as_slice() else { panic!("one buffer, csd-0: {buffers:?}") };
        // Pinned here because the emulator's AV1 decoder reads no codec-specific data (it decodes from the key
        // frame's own sequence header), so only a phone's hardware decoder reads these bytes. The record's
        // marker and version byte, then its profile and level, then the sequence header OBU.
        assert_eq!(record[..4], [0x81, 0x00, 0x0C, 0x00]);
        assert_eq!(record[4..], [0x0A, 0x0D, 0x00, 0x00, 0x00, 0x03, 0xB4, 0xFF, 0x73, 0x6B, 0xE6, 0x40, 0x40, 0x40, 0x41]);
        assert_eq!(codec_specific_data(&avc_entry(), &av1()), Err("the sample entry has no av1C"), "an entry with no record");
        for (_, sample) in av1_samples() {
            assert_eq!(input(&av1(), &sample), Ok(sample.clone()), "no start codes and no length prefixes");
        }
        assert_eq!(input(&av1(), &[]), Err("the sample is empty"));
    }

    #[test]
    fn an_h264_or_hevc_sample_is_fed_as_annex_b() {
        let sample = [&[0, 0, 0, 2][..], &[0x65, 0x88]].concat();
        assert_eq!(input(&StreamInfo::Nal(nal()), &sample), Ok(vec![0, 0, 0, 1, 0x65, 0x88]));
        assert_eq!(input(&StreamInfo::Nal(nal()), &[0, 0, 0, 2, 0x65]), Err("the sample is not whole NAL units"));
    }

    #[test]
    fn length_prefixed_nal_units_become_an_annex_b_stream() {
        let info = nal();
        assert_eq!(info.length_size, 4);
        let sample = [&[0, 0, 0, 2][..], &[0x65, 0x88], &[0, 0, 0, 3], &[0x41, 0x9A, 0x20]].concat();
        assert_eq!(annex_b(&info, &sample), Ok(vec![0, 0, 0, 1, 0x65, 0x88, 0, 0, 0, 1, 0x41, 0x9A, 0x20]));
        let mut short = info;
        short.length_size = 2;
        assert_eq!(annex_b(&short, &[0, 1, 0x65, 0, 2, 0x41, 0x9A]), Ok(vec![0, 0, 0, 1, 0x65, 0, 0, 0, 1, 0x41, 0x9A]));
    }

    #[test]
    fn a_sample_that_is_not_whole_nal_units_is_an_error_never_a_shortened_stream() {
        let info = nal();
        for sample in [
            &[][..],
            &[0, 0, 0][..],
            &[0, 0, 0, 2, 0x65][..],
            &[0, 0, 0, 1, 0x65, 0, 0][..],
            &[0, 0, 0, 0][..],
            &[0, 0, 0, 1, 0x65, 0, 0, 0, 0][..],
            &[0xFF, 0xFF, 0xFF, 0xFF, 0x65][..],
        ] {
            assert_eq!(annex_b(&info, sample), Err("the sample is not whole NAL units"), "{sample:?}");
        }
        let mut none = nal();
        none.length_size = 0;
        assert_eq!(annex_b(&none, &[0, 0, 0, 1, 0x65]), Err("the sample is not whole NAL units"), "no length size");
    }

    #[test]
    fn a_drain_gives_what_the_end_and_each_read_gives_until_the_stream_ends_and_every_failure_is_an_error() {
        let long = Duration::from_secs(3600);
        let mut reads = [(vec![2], false), (Vec::new(), false), (vec![3, 4], true)].into_iter();
        assert_eq!(until_the_end(long, "decoder", || Ok(vec![1]), || Ok(reads.next().unwrap())), Ok(vec![1, 2, 3, 4]), "every picture, in order, then the end");
        // An end read in time is the end, however late the clock is by then; with no end in time, the pictures
        // read so far are not the stream's whole tail.
        assert_eq!(until_the_end(Duration::ZERO, "decoder", || Ok(vec![]), || Ok((vec![5], true))), Ok(vec![5]));
        assert_eq!(until_the_end(Duration::ZERO, "decoder", || Ok(vec![]), || Ok((vec![6], false))), Err("the decoder never ended the stream".into()));
        let failed = "waiting for a picture failed (ErrorUnknown)";
        assert_eq!(until_the_end(long, "decoder", || Ok(vec![]), || Err::<(Vec<u8>, bool), _>(failed.into())), Err(failed.into()));
        // A codec that cannot take the end is not read from.
        let refused = "waiting for an input buffer failed (ErrorUnknown)";
        let mut reads_made = 0;
        let read = || -> Result<(Vec<u8>, bool), String> {
            reads_made += 1;
            Ok((vec![7], true))
        };
        assert_eq!(until_the_end(long, "decoder", || Err(refused.into()), read), Err(refused.into()));
        assert_eq!(reads_made, 0, "no read after a refused end");
        // The time allowed runs from the end being queued, however long the queue took: a codec that waited for
        // an input buffer still has all of it to end the stream.
        let slow_queue = || {
            std::thread::sleep(Duration::from_millis(300));
            Ok(Vec::new())
        };
        let mut slow_reads = [(vec![8], false), (vec![9], true)].into_iter();
        assert_eq!(until_the_end(Duration::from_millis(200), "decoder", slow_queue, || Ok(slow_reads.next().unwrap())), Ok(vec![8, 9]));
    }
}
