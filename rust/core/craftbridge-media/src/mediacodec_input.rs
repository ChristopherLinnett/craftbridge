//! What MediaCodec is told about a stream, and the bytes it is fed: which streams the Android decoder takes,
//! which of the device's decoders count as hardware, and the Annex B byte stream MediaCodec reads where an
//! MP4 sample holds length-prefixed NAL units.
//!
//! Safe code, so every target tests it; the platform module only makes the calls.

use filmcraft_codecs::hw::NalStreamInfo;

use crate::biplanar::{Crop, Depth, Plane, Strided};
use crate::layout::{Codec, Layout};

/// What every NAL unit of an Annex B byte stream starts with.
const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// MediaCodec's names for the codecs this module takes.
pub(crate) const H264_MIME: &str = "video/avc";
pub(crate) const HEVC_MIME: &str = "video/hevc";

/// Every MIME type a decoder is taken for, which is what [`crate::register`] asks the device about.
#[cfg(target_os = "android")]
pub(crate) const BACKEND_MIMES: [&str; 2] = [H264_MIME, HEVC_MIME];

/// The MIME type MediaCodec names the decoder of `layout`'s stream by, or why the Android decoder does not
/// take it. Only 8-bit pictures reach `YUV_420_888`, the format the decoder's output is read in: a 10-bit
/// picture would lose its low bits on the way, so it is left to the software decoder, which keeps them. AV1
/// is left to it too: its samples are OBUs, and the Android decoder feeds MediaCodec NAL units alone.
pub(crate) fn mime(layout: &Layout) -> Result<&'static str, String> {
    match (layout.codec, layout.depth) {
        (Codec::Av1, _) => Err("AV1".into()),
        (_, Depth::Ten) => Err("10-bit video".into()),
        (Codec::H264, Depth::Eight) => Ok(H264_MIME),
        (Codec::Hevc, Depth::Eight) => Ok(HEVC_MIME),
    }
}

/// Where the picture is in a decoded image whose crop rectangle is `rect` (left, top, right, bottom): the
/// rectangle must be the stream's own picture size, since the image reader was made for it.
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

/// Whether the decoder MediaCodec names `name` is one of the platform's own software codecs, which the
/// software decoder already beats: it decodes in FilmCraft's code on every device alike, and a platform
/// codec in software is no faster.
pub(crate) fn is_software_codec(name: &str) -> bool {
    name.starts_with("c2.android.") || name.starts_with("OMX.google.")
}

/// The stream's parameter sets as the codec-specific data MediaCodec starts a decoder with: every set in
/// record order (H.264 SPS then PPS; HEVC VPS, SPS, PPS), each after a start code.
pub(crate) fn codec_specific_data(info: &NalStreamInfo) -> Vec<u8> {
    info.parameter_sets.iter().flat_map(|set| START_CODE.iter().chain(set)).copied().collect()
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

#[cfg(test)]
mod tests {
    use filmcraft_codecs::hw::{FrameCodec, FrameStreamInfo, NalCodec, StreamInfo};
    use filmcraft_color::ColorInfo;

    use super::*;
    use crate::fixtures::{avc_entry, hevc_entry, pps, sps};

    fn nal() -> NalStreamInfo {
        NalStreamInfo::from_entry(&avc_entry()).unwrap().unwrap()
    }

    fn layout(change: impl FnOnce(&mut NalStreamInfo)) -> Layout {
        let mut info = nal();
        change(&mut info);
        Layout::for_stream(&StreamInfo::Nal(info)).unwrap()
    }

    #[test]
    fn h264_and_8_bit_hevc_are_named_by_the_mime_types_mediacodec_knows_them_by() {
        // MediaCodec's own words for the two.
        assert_eq!(mime(&layout(|_| {})), Ok("video/avc"));
        assert_eq!(mime(&layout(|info| info.codec = NalCodec::Hevc)), Ok("video/hevc"));
    }

    #[test]
    fn ten_bit_pictures_and_av1_are_left_to_the_software_decoder() {
        let ten = layout(|info| {
            info.bit_depth_luma = 10;
            info.bit_depth_chroma = 10;
        });
        assert_eq!(mime(&ten), Err("10-bit video".to_string()), "10-bit H.264");
        let ten_hevc = layout(|info| {
            info.codec = NalCodec::Hevc;
            info.bit_depth_luma = 10;
            info.bit_depth_chroma = 10;
        });
        assert_eq!(mime(&ten_hevc), Err("10-bit video".to_string()), "10-bit HEVC");
        let mut av1 = FrameStreamInfo::for_tests(FrameCodec::Av1, 0, 8, (1, 1), (640, 360));
        av1.color = Some(ColorInfo::REC709);
        assert_eq!(mime(&Layout::for_stream(&StreamInfo::Frame(av1)).unwrap()), Err("AV1".to_string()), "AV1");
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

    #[test]
    fn the_platforms_own_software_codecs_are_not_hardware_and_every_other_name_is() {
        for name in ["c2.android.avc.decoder", "c2.android.hevc.decoder", "OMX.google.h264.decoder", "OMX.google.hevc.decoder"] {
            assert!(is_software_codec(name), "{name}");
        }
        for name in ["c2.goldfish.h264.decoder", "c2.qti.avc.decoder", "OMX.qcom.video.decoder.avc", "c2.exynos.h264.decoder", "c2.android", "OMX.google"] {
            assert!(!is_software_codec(name), "{name}");
        }
    }

    #[test]
    fn every_parameter_set_follows_a_start_code_in_record_order() {
        let data = codec_specific_data(&nal());
        let expected: Vec<u8> = [&[0, 0, 0, 1][..], &sps(), &[0, 0, 0, 1], &pps()].concat();
        assert_eq!(data, expected);
        // An HEVC stream's VPS too, which some decoders do without and others need.
        let hevc = NalStreamInfo::from_entry(&hevc_entry()).unwrap().unwrap();
        assert_eq!(hevc.parameter_sets.iter().map(|set| (set[0] >> 1) & 0x3f).collect::<Vec<_>>(), [32, 33, 34], "the record's VPS, SPS and PPS");
        let expected: Vec<u8> = hevc.parameter_sets.iter().flat_map(|set| [&[0, 0, 0, 1][..], set].concat()).collect();
        assert_eq!(codec_specific_data(&hevc), expected);
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
}
