//! The Android hardware video decoder: MediaCodec decoding `avcC` (H.264) and, where the device decodes it,
//! `hvcC` (HEVC) streams into an `AImageReader` of `YUV_420_888` images.
//!
//! Decoding to an image reader, rather than to the buffers MediaCodec hands out, is what keeps vendor-tiled
//! and other private output formats away from CraftBridge: the platform converts every picture into the one
//! format Android guarantees. The decoder it uses must be a hardware one (the platform's own software
//! codecs are no faster than FilmCraft's), so a device or stream without one fails to start and the factory
//! declines it. MediaCodec decodes on its own schedule, so a picture comes out of a later call than the
//! one that fed its sample, and [`crate::order::Sequence`] runs around the codec as it does around VideoToolbox: the
//! RASL pictures of a run starting at a CRA are never fed to it, and the pictures it still holds at the end
//! come out of [`RawDecoder::drain`]. Every status MediaCodec and the image reader return is checked and
//! becomes the decoder's error, which [`crate::Fallback`] answers by switching to the software decoder.
//!
//! The official NDK bindings (`ndk`) make every call; the only `unsafe` here is the claim that the codec and
//! the reader may move between threads. Which streams the device takes, the Annex B bytes it is fed and the
//! planes' conversion into FilmCraft's frame layout are safe code every target tests
//! ([`crate::mediacodec_input`], [`crate::biplanar`]).

use std::time::{Duration, Instant};

use filmcraft_codecs::hw::{NalStreamInfo, StreamInfo};
use filmcraft_codecs::{CodecError, DecodedFrame, Result};
use filmcraft_frame::VideoFrame;
use ndk::media::image_reader::{AcquireResult, Image, ImageFormat, ImageReader};
use ndk::media::media_codec::{DequeuedInputBufferResult, DequeuedOutputBufferInfoResult, MediaCodec, MediaCodecDirection};
use ndk::media::media_format::MediaFormat;

use crate::biplanar::{self, Strided};
use crate::layout::Layout;
use crate::mediacodec_input::{self, BACKEND_MIMES};
use crate::order::{HardwareDecoder, RawDecoder};

/// A failed call, with what failed.
type Failure = String;

/// The backend's name, which [`crate::register`] reports and every decoder's name starts with.
pub(crate) const BACKEND: &str = "MediaCodec";

/// Makes the codec a decoding run decodes a MIME type with: [`hardware_codec`], bar in a test that feeds
/// another of the device's decoders exactly as the hardware one is fed.
type MakeCodec = fn(&str) -> std::result::Result<MediaCodec, Failure>;

/// How long a wait for the codec or the image reader may take before the decoder gives up on it: far
/// longer than any decoder takes to start or to hand back a picture, and short enough that a stalled one
/// is an error the software decoder answers.
const STALL: Duration = Duration::from_secs(5);

/// How long one wait for an input buffer, or for a picture, lasts before it looks again.
const POLL: Duration = Duration::from_millis(10);

/// `AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM`.
const END_OF_STREAM: u32 = 4;

/// The pictures the image reader holds at once. A picture is read and released before the next is taken, so
/// the codec never waits for a free one; the margin keeps a slow release from stalling it.
const IMAGES: i32 = 4;

/// Turns a failed NDK call into the decoder's error.
fn step<T>(result: ndk::media_error::Result<T>, call: &str) -> std::result::Result<T, Failure> {
    result.map_err(|error| format!("{call} failed ({error:?})"))
}

/// The device's decoder for `mime`, when it is a hardware one; an error naming why not.
fn hardware_codec(mime: &str) -> std::result::Result<MediaCodec, Failure> {
    let codec = MediaCodec::from_decoder_type(mime).ok_or_else(|| format!("this device has no decoder for {mime}"))?;
    let name = step(codec.name(), "naming the decoder")?;
    if mediacodec_input::is_software_codec(&name) {
        return Err(format!("this device decodes {mime} only in software ({name})"));
    }
    Ok(codec)
}

/// Whether this device has a hardware decoder for `mime`. It does not promise a decoder will start
/// (resources can be taken), only that starting one is worth trying.
pub(crate) fn has_hardware_decoder(mime: &str) -> bool {
    hardware_codec(mime).is_ok()
}

/// Whether this device has a hardware decoder for H.264 or HEVC, which is what [`crate::register`] reports as
/// its backend.
pub(crate) fn available() -> bool {
    BACKEND_MIMES.into_iter().any(has_hardware_decoder)
}

/// A codec and the reader its pictures arrive in, which move between threads with the decoder that owns
/// them.
///
/// Neither is tied to the thread that made it. Android sets thread rules only for their callbacks, which
/// this decoder never registers; the platform hands each codec call to the codec's own looper thread and
/// takes the reader's lock in each call on its queue. The decoder makes every call from one thread at a
/// time (`&mut self`). The `ndk` bindings hold raw pointers, which are not `Send`, so this type says it for
/// the pair alone.
struct Pipeline {
    /// Dropped before `reader`: the codec stops writing to the reader's surface before the surface goes.
    codec: MediaCodec,
    reader: ImageReader,
}

// SAFETY: see the type's documentation.
unsafe impl Send for Pipeline {}

impl Pipeline {
    /// `codec`, a decoder of `mime`, started for a picture of `layout`'s size from `config` and decoding into
    /// a reader.
    fn start(codec: MediaCodec, mime: &str, layout: &Layout, config: &[u8]) -> std::result::Result<Self, Failure> {
        let (width, height) = layout.size_i32().ok_or("the picture is too large")?;
        let reader = step(ImageReader::new(width, height, ImageFormat::YUV_420_888, IMAGES), "creating the image reader")?;
        let window = step(reader.window(), "getting the image reader's surface")?;
        let mut format = MediaFormat::new();
        format.set_str("mime", mime);
        format.set_i32("width", width);
        format.set_i32("height", height);
        format.set_buffer("csd-0", config);
        step(codec.configure(&format, Some(&window), MediaCodecDirection::Decoder), "configuring the decoder")?;
        step(codec.start(), "starting the decoder")?;
        Ok(Self { codec, reader })
    }

    /// Queues `data` as one input sample at the codec's `time` ([`mediacodec_input::codec_time`]), or the end
    /// of the stream when `end` is set (with no data). Pictures come out while it waits for a free input
    /// buffer, which it returns: the codec takes no input until its output is read.
    fn queue(&self, data: &[u8], time: u64, end: bool, layout: &Layout) -> std::result::Result<Vec<(i64, VideoFrame)>, Failure> {
        let mut pictures = Vec::new();
        let deadline = Instant::now() + STALL;
        loop {
            match step(self.codec.dequeue_input_buffer(POLL), "waiting for an input buffer")? {
                DequeuedInputBufferResult::Buffer(mut input) => {
                    let room = input.buffer_mut();
                    let (target, _) = room.split_at_mut_checked(data.len()).ok_or("the sample is larger than the decoder's input buffer")?;
                    for (to, from) in target.iter_mut().zip(data) {
                        to.write(*from);
                    }
                    let flags = if end { END_OF_STREAM } else { 0 };
                    step(self.codec.queue_input_buffer(input, 0, data.len(), time, flags), "queueing the sample")?;
                    return Ok(pictures);
                }
                DequeuedInputBufferResult::TryAgainLater => {
                    pictures.extend(self.pictures(Duration::ZERO, layout)?.0);
                    if Instant::now() >= deadline {
                        return Err("the decoder took no input".into());
                    }
                }
            }
        }
    }

    /// Every picture the codec has ready, waiting up to `wait` for the first; and whether the end of the
    /// stream came out, which no picture follows.
    fn pictures(&self, wait: Duration, layout: &Layout) -> std::result::Result<(Vec<(i64, VideoFrame)>, bool), Failure> {
        let mut pictures = Vec::new();
        loop {
            match step(self.codec.dequeue_output_buffer(wait), "waiting for a picture")? {
                DequeuedOutputBufferInfoResult::TryAgainLater => return Ok((pictures, false)),
                DequeuedOutputBufferInfoResult::OutputFormatChanged | DequeuedOutputBufferInfoResult::OutputBuffersChanged => {}
                DequeuedOutputBufferInfoResult::Buffer(output) => {
                    let info = output.info();
                    let (pts, flags, shown) = (mediacodec_input::stream_time(info.presentation_time_us()), info.flags(), info.size() > 0);
                    // Rendering hands the picture to the reader; releasing without it drops one that is not.
                    step(self.codec.release_output_buffer(output, shown), "releasing the decoded picture")?;
                    if shown {
                        pictures.push((pts, self.read(layout)?));
                    }
                    if flags & END_OF_STREAM != 0 {
                        return Ok((pictures, true));
                    }
                }
            }
        }
    }

    /// The picture the codec just rendered, read out of the reader and released.
    fn read(&self, layout: &Layout) -> std::result::Result<VideoFrame, Failure> {
        let deadline = Instant::now() + STALL;
        loop {
            match step(self.reader.acquire_next_image(), "acquiring the decoded picture")? {
                AcquireResult::Image(image) => return read_picture(&image, layout),
                AcquireResult::NoBufferAvailable | AcquireResult::MaxImagesAcquired => {
                    // The codec renders asynchronously: the picture reaches the reader a moment after the
                    // codec released it.
                    if Instant::now() >= deadline {
                        return Err("the decoded picture never reached the image reader".into());
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
}

/// The picture in a decoded image, as the frame FilmCraft's software decoder makes for the stream.
fn read_picture(image: &Image, layout: &Layout) -> std::result::Result<VideoFrame, Failure> {
    let format = step(image.format(), "reading the picture's format")?;
    if format != ImageFormat::YUV_420_888 || step(image.number_of_planes(), "counting the picture's planes")? != 3 {
        return Err(format!("the decoded picture is in format {format:?}, not YUV_420_888"));
    }
    let rect = step(image.crop_rect(), "reading the picture's crop")?;
    let crop = mediacodec_input::crop_of((rect.left, rect.top, rect.right, rect.bottom), layout)?;
    let plane = |index: i32| -> std::result::Result<Strided<'_>, Failure> {
        let data = step(image.plane_data(index), "reading a plane")?;
        let stride = step(image.plane_row_stride(index), "reading a plane's row stride")?;
        let pixel = step(image.plane_pixel_stride(index), "reading a plane's pixel stride")?;
        mediacodec_input::plane(index, data, stride, pixel)
    };
    let pixels = biplanar::planar_to_pixels(mediacodec_input::luma(plane(0)?)?, plane(1)?, plane(2)?, crop)?;
    Ok(layout.frame(pixels))
}

/// What a MediaCodec decoder feeds [`crate::order::Sequence`]: the codec, made again after a seek.
pub(crate) struct RawCodec {
    layout: Layout,
    info: NalStreamInfo,
    mime: &'static str,
    make: MakeCodec,
    /// The codec-specific data a codec starts with.
    config: Vec<u8>,
    /// `None` until the first sample of a run: each run, after a seek or the end of a stream, gets a codec of
    /// its own. A fresh codec refers to no picture of an earlier run and holds none of its pictures, and a
    /// flushed one is no substitute: the emulator's decoder takes no sample after a flush and a restart.
    pipeline: Option<Pipeline>,
}

impl RawCodec {
    /// The codec of the run under way, started if the run has none yet.
    fn pipeline(&mut self) -> std::result::Result<&Pipeline, Failure> {
        if self.pipeline.is_none() {
            self.pipeline = Some(Pipeline::start((self.make)(self.mime)?, self.mime, &self.layout, &self.config)?);
        }
        self.pipeline.as_ref().ok_or_else(|| "there is no decoder".into())
    }
}

impl RawDecoder<DecodedFrame> for RawCodec {
    fn feed(&mut self, sample: &[u8], pts: i64) -> std::result::Result<Vec<(i64, DecodedFrame)>, Failure> {
        let data = mediacodec_input::annex_b(&self.info, sample)?;
        let time = mediacodec_input::codec_time(pts)?;
        let layout = self.layout.clone();
        let pipeline = self.pipeline()?;
        // A codec that failed is kept, with its reference pictures, as the software decoder keeps its state
        // after an error: a fresh one fed the next sample would decode it from references it never saw.
        let mut pictures = pipeline.queue(&data, time, false, &layout)?;
        pictures.extend(pipeline.pictures(Duration::ZERO, &layout)?.0);
        Ok(decoded(pictures))
    }

    fn restart(&mut self) {
        self.pipeline = None;
    }

    fn drain(&mut self) -> Vec<(i64, DecodedFrame)> {
        let layout = self.layout.clone();
        let mut pictures = Vec::new();
        if let Some(pipeline) = &self.pipeline {
            // The end of the stream is queued as an input of its own; the codec outputs what it holds, then
            // the end. A failure on the way keeps the pictures read so far: the caller has no more to ask for.
            let queued = pipeline.queue(&[], 0, true, &layout).map(|held| pictures.extend(held));
            let deadline = Instant::now() + STALL;
            while queued.is_ok() && Instant::now() < deadline {
                let Ok((more, end)) = pipeline.pictures(POLL, &layout) else { break };
                pictures.extend(more);
                if end {
                    break;
                }
            }
        }
        // A codec that has seen the end takes no more samples.
        self.pipeline = None;
        decoded(pictures)
    }
}

/// Pictures as the decoder returns them.
fn decoded(pictures: Vec<(i64, VideoFrame)>) -> Vec<(i64, DecodedFrame)> {
    pictures.into_iter().map(|(pts, frame)| (pts, DecodedFrame { pts, frame, draft: false })).collect()
}

/// The MediaCodec decoder of one `avcC` or `hvcC` stream.
pub(crate) type McDecoder = HardwareDecoder<RawCodec>;

/// A decoder for the stream whose parsed record is `stream`. It starts a codec now, so a stream or device
/// without a hardware decoder for it is refused here.
pub(crate) fn decoder(stream: &StreamInfo) -> Result<McDecoder> {
    decoder_with(stream, hardware_codec)
}

/// [`decoder`], with the codec each decoding run gets made by `make`.
fn decoder_with(stream: &StreamInfo, make: MakeCodec) -> Result<McDecoder> {
    let layout = Layout::for_stream(stream).map_err(CodecError::Unsupported)?;
    let mime = mediacodec_input::mime(&layout).map_err(CodecError::Unsupported)?;
    let StreamInfo::Nal(info) = stream else {
        return Err(CodecError::Unsupported("MediaCodec takes H.264 and HEVC".into()));
    };
    let config = mediacodec_input::codec_specific_data(info);
    let pipeline = make(mime).and_then(|codec| Pipeline::start(codec, mime, &layout, &config)).map_err(CodecError::Unsupported)?;
    let codec = layout.codec;
    // MediaCodec outputs pictures in presentation order itself, so the reorder buffer holds none back.
    let raw = RawCodec { layout, info: info.clone(), mime, make, config, pipeline: Some(pipeline) };
    Ok(HardwareDecoder::new(raw, stream.clone(), 0, BACKEND, codec))
}

/// These run where a device's media codecs are: on an Android phone or emulator, which is where this module
/// builds.
#[cfg(test)]
mod tests {
    use filmcraft_codecs::hw::NalCodec;
    use filmcraft_codecs::{VideoDecoder, software_video_decoder};
    use filmcraft_isobmff::SampleEntry;

    use super::*;
    use crate::fallback::stream_info;
    use crate::fixtures::{decoded, hevc_entry, hevc_samples, pictures, reordered_entry, reordered_samples, tiny_entry, tiny_samples};
    use crate::mediacodec_input::{H264_MIME, HEVC_MIME};

    fn stream(entry: &SampleEntry) -> StreamInfo {
        stream_info(entry).unwrap().unwrap()
    }

    /// The device's decoder for `entry`'s stream; `None` where the device has no hardware decoder for its codec,
    /// which is the decline the factory turns into the software decoder.
    fn device_decoder(entry: &SampleEntry, mime: &str) -> Option<McDecoder> {
        let decoder = decoder(&stream(entry));
        assert_eq!(decoder.is_ok(), has_hardware_decoder(mime), "a decoder starts exactly where the device has a hardware one for {mime}");
        decoder.ok()
    }

    fn software(entry: &SampleEntry) -> Box<dyn VideoDecoder> {
        software_video_decoder(entry).unwrap()
    }

    #[test]
    fn a_decoder_is_hardware_only_when_the_device_names_one_that_is_not_a_platform_software_codec() {
        for mime in BACKEND_MIMES {
            let name = MediaCodec::from_decoder_type(mime).map(|codec| codec.name().unwrap());
            let expected = match name {
                None => Err(format!("this device has no decoder for {mime}")),
                Some(name) if mediacodec_input::is_software_codec(&name) => Err(format!("this device decodes {mime} only in software ({name})")),
                Some(_) => Ok(()),
            };
            assert_eq!(hardware_codec(mime).map(|_| ()), expected, "{mime}");
            assert_eq!(has_hardware_decoder(mime), expected.is_ok(), "{mime}");
        }
        assert_eq!(available(), BACKEND_MIMES.into_iter().any(has_hardware_decoder));
    }

    #[test]
    fn a_failed_call_is_the_decoders_error_naming_the_call_and_what_the_platform_said() {
        let failed: ndk::media_error::Result<()> = Err(ndk::media_error::MediaError::ErrorMalformed);
        assert_eq!(step(failed, "queueing the sample"), Err("queueing the sample failed (ErrorMalformed)".to_string()));
        assert_eq!(step(Ok(3), "queueing the sample"), Ok(3));
    }

    #[test]
    fn the_device_decodes_h264_to_the_pictures_the_software_decoder_gives() {
        // The last stream starts two pictures before time zero, where negative composition offsets can put a
        // stream's first pictures: the codec hands every presentation time back as it was given.
        let before_zero = reordered_samples().into_iter().map(|(pts, sample)| (pts - 2, sample)).collect();
        for (entry, samples) in [(tiny_entry(), tiny_samples()), (reordered_entry(), reordered_samples()), (reordered_entry(), before_zero)] {
            let Some(mut decoder) = device_decoder(&entry, H264_MIME) else { return };
            assert_eq!(decoder.name(), "MediaCodec H.264");
            assert_eq!(decoded(&mut decoder, &samples), decoded(software(&entry).as_mut(), &samples));
        }
    }

    #[test]
    fn pictures_of_a_stream_with_b_pictures_come_out_in_presentation_order() {
        let entry = reordered_entry();
        let Some(mut decoder) = device_decoder(&entry, H264_MIME) else { return };
        let order: Vec<i64> = pictures(&mut decoder, &reordered_samples()).iter().map(|picture| picture.pts).collect();
        assert_eq!(order, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_flush_ends_the_run_and_the_decoder_takes_the_stream_again_from_its_start() {
        let entry = tiny_entry();
        let Some(mut decoder) = device_decoder(&entry, H264_MIME) else { return };
        let samples = tiny_samples();
        let wanted = decoded(software(&entry).as_mut(), &samples);
        assert_eq!(decoded(&mut decoder, &samples), wanted);
        assert_eq!(decoded(&mut decoder, &samples), wanted, "after the end of the stream");
        assert!(decoder.flush().is_empty(), "nothing is held after a flush");
    }

    #[test]
    fn a_reset_forgets_the_run_so_far_and_the_next_sample_starts_another() {
        // The pictures the first samples of a stream with B pictures leave in the codec must not come out of the
        // next run.
        for (entry, samples) in [(tiny_entry(), tiny_samples()), (reordered_entry(), reordered_samples())] {
            let Some(mut decoder) = device_decoder(&entry, H264_MIME) else { return };
            for (pts, sample) in samples.iter().take(3) {
                decoder.decode(sample, *pts).unwrap();
            }
            decoder.reset();
            assert_eq!(decoded(&mut decoder, &samples), decoded(software(&entry).as_mut(), &samples));
        }
    }

    #[test]
    fn a_sample_that_is_not_nal_units_is_an_error_the_decoder_survives() {
        let entry = tiny_entry();
        let Some(mut decoder) = device_decoder(&entry, H264_MIME) else { return };
        assert!(matches!(decoder.decode(&[0, 0, 0, 9, 0x65], 0), Err(CodecError::Decode(_))));
        assert!(matches!(decoder.decode(&[], 0), Err(CodecError::Decode(_))));
        assert_eq!(decoded(&mut decoder, &tiny_samples()), decoded(software(&entry).as_mut(), &tiny_samples()));
    }

    #[test]
    fn the_device_decodes_hevc_only_where_it_has_a_hardware_decoder_for_it() {
        let entry = hevc_entry();
        let Some(mut decoder) = device_decoder(&entry, HEVC_MIME) else { return };
        assert_eq!(decoder.name(), "MediaCodec HEVC");
        assert_eq!(decoded(&mut decoder, &hevc_samples()), decoded(software(&entry).as_mut(), &hevc_samples()));
    }

    /// The device's own decoder for `mime`, a software one included.
    fn any_codec(mime: &str) -> std::result::Result<MediaCodec, Failure> {
        MediaCodec::from_decoder_type(mime).ok_or_else(|| format!("this device has no decoder for {mime}"))
    }

    #[test]
    fn an_hevc_stream_fed_as_the_hardware_decoder_is_fed_decodes_to_the_pictures_the_software_decoder_gives() {
        // An emulator has no hardware HEVC decoder, so its own decoder, software or not, takes the stream with
        // the same codec-specific data, the same Annex B samples and the same image reader.
        let entry = hevc_entry();
        let decoder = decoder_with(&stream(&entry), any_codec);
        assert_eq!(decoder.is_ok(), MediaCodec::from_decoder_type("video/hevc").is_some(), "a decoder starts wherever the device decodes HEVC");
        let Ok(mut decoder) = decoder else { return };
        assert_eq!(decoder.name(), "MediaCodec HEVC");
        assert_eq!(decoded(&mut decoder, &hevc_samples()), decoded(software(&entry).as_mut(), &hevc_samples()));
    }

    #[test]
    fn streams_the_device_decoder_does_not_take_are_refused_before_it_is_asked() {
        let mut ten_bit = stream(&tiny_entry());
        let StreamInfo::Nal(info) = &mut ten_bit else { panic!("an avcC stream") };
        assert_eq!(info.codec, NalCodec::H264);
        (info.bit_depth_luma, info.bit_depth_chroma) = (10, 10);
        assert!(matches!(decoder(&ten_bit), Err(CodecError::Unsupported(_))), "10-bit video");
    }
}
