//! The Android hardware video decoder: MediaCodec decoding `avcC` (H.264) and, where the device decodes it,
//! `hvcC` (HEVC) and `av1C` (AV1) streams into an `AImageReader`.
//!
//! Decoding to an image reader, rather than to the buffers MediaCodec hands out, is what keeps vendor-tiled and
//! other private output formats away from CraftBridge. The decoder is asked for the reader's layout
//! (`color-format`): a decoder writing to a surface and asked for none picks its own, and Qualcomm's pick a
//! compressed one. An 8-bit picture is read from a `YUV_420_888` reader's image, the one format Android
//! guarantees, through the three planes the platform describes it as, whatever format code it carries (Qualcomm's
//! decoders give their own linear NV12 code); an image it describes no planes of, such as a compressed one, is
//! an error and none of it is read. The NDK has no 10-bit YUV image format, so a 10-bit picture is decoded into a
//! reader of `AHARDWAREBUFFER_FORMAT_YCbCr_P010` images, which Android makes from 14 (API 34) and no earlier, and
//! read from the image's hardware buffer, locked for the CPU, only when that buffer is P010 laid out as P010 is:
//! before Android 14 no decoder starts for a 10-bit stream, and a device whose buffer is another (vendor-tiled)
//! format fails the picture's read; either way the software decoder takes the stream. The decoder it uses must be
//! a hardware one by the platform's own answer (a software codec is no faster than FilmCraft's), chosen from the
//! device's codec list rather than taken from the platform's default for the type, which can be a software codec;
//! a device or stream without one fails to start and the factory declines it. MediaCodec decodes on its own
//! schedule, so a picture comes out of a later call than the one that fed its sample, and
//! [`crate::order::Sequence`] runs around the codec as it does around VideoToolbox: the RASL pictures of a run
//! starting at a CRA are never fed to it, and the pictures it still holds at the end come out of
//! [`RawDecoder::drain`], which fails when the codec does or never ends the stream. Every status MediaCodec and
//! the image reader return is checked and becomes the decoder's error, which [`crate::Fallback`] answers by
//! switching to the software decoder, at the end of the stream as well.
//!
//! The official NDK bindings (`ndk`) make every call; the only `unsafe` here is the claim that the codec and
//! the reader may move between threads, and the bytes of a locked P010 buffer's planes. Which streams the
//! device takes, the codec-specific data and samples it is fed and the planes' conversion into FilmCraft's
//! frame layout are safe code every target tests ([`crate::mediacodec_input`], [`crate::biplanar`]).

use std::cell::Cell;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use filmcraft_codecs::hw::StreamInfo;
use filmcraft_codecs::{CodecError, DecodedFrame, Result};
use filmcraft_frame::{PixelData, VideoFrame};
use filmcraft_isobmff::SampleEntry;
use ndk::hardware_buffer::{HardwareBuffer, HardwareBufferUsage, LockedPlaneInfo};
use ndk::hardware_buffer_format::HardwareBufferFormat;
use ndk::media::image_reader::{AcquireResult, Image, ImageFormat, ImageReader};
use ndk::media::media_codec::{DequeuedInputBufferResult, DequeuedOutputBufferInfoResult, InputBuffer, MediaCodec, MediaCodecDirection, OutputBuffer};
use ndk::media::media_format::MediaFormat;

use crate::biplanar::{self, Crop, Depth, Locked, Plane, Strided};
use crate::layout::Layout;
use crate::mediacodec_input::{self, BACKEND_MIMES, CodecInfo, ReaderFormat};
use crate::order::{HardwareDecoder, RawDecoder};

/// A failed call, with what failed.
pub(crate) type Failure = String;

/// The backend's name, which [`crate::register`] reports and every decoder's name starts with.
pub(crate) const BACKEND: &str = "MediaCodec";

/// Makes the codec a decoder decodes a MIME type with, and makes it again after one that failed to flush:
/// [`hardware_codec`], bar in a test that feeds another of the device's decoders exactly as the hardware one
/// is fed.
pub(crate) type MakeCodec = fn(&str) -> std::result::Result<MediaCodec, Failure>;

/// How long a wait for the codec or the image reader may take before the decoder gives up on it: far
/// longer than any decoder takes to start or to hand back a picture, and short enough that a stalled one
/// is an error the software decoder answers. An encoder, whose calls are the same codec's, waits no longer.
pub(crate) const STALL: Duration = Duration::from_secs(5);

/// How long one wait for an input buffer, or for a picture, lasts before it looks again.
pub(crate) const POLL: Duration = Duration::from_millis(10);

/// `AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM`.
pub(crate) const END_OF_STREAM: u32 = 4;

/// `AMEDIACODEC_BUFFER_FLAG_CODEC_CONFIG`, set on the buffers carrying the codec's own parameter sets.
pub(crate) const CODEC_CONFIG: u32 = 2;

/// The `MediaFormat` keys the decoder and the encoders both set, by their names there (`KEY_MIME`,
/// `KEY_WIDTH`, `KEY_HEIGHT`, `KEY_BIT_RATE`, `KEY_COLOR_FORMAT`).
pub(crate) const MIME: &str = "mime";
pub(crate) const WIDTH: &str = "width";
pub(crate) const HEIGHT: &str = "height";
pub(crate) const BIT_RATE: &str = "bitrate";
pub(crate) const COLOR_FORMAT: &str = "color-format";

/// The pictures the image reader holds at once. A picture is read and released before the next is taken, so
/// the codec never waits for a free one; the margin keeps a slow release from stalling it.
const IMAGES: i32 = 4;

/// How the pictures of the decoder's output are read: by the CPU, which both readers' usage says, so the
/// buffers the codec allocates for them are ones the CPU can read.
const READ_USAGE: HardwareBufferUsage = HardwareBufferUsage::CPU_READ_OFTEN;

/// Turns a failed NDK call into the codec's error, worded for whichever of the decoder and the encoder made
/// it: both call through here.
pub(crate) fn step<T, E: std::fmt::Debug>(result: std::result::Result<T, E>, call: &str) -> std::result::Result<T, Failure> {
    result.map_err(|error| format!("{call} failed ({error:?})"))
}

/// The device's codec list, as the app handed it over ([`set_device_codecs`]).
static DEVICE_CODECS: RwLock<Option<Vec<CodecInfo>>> = RwLock::new(None);

/// Gives the decoder the device's codec list, which it chooses its hardware decoders from. The NDK lists
/// codecs, and says which are hardware, only from API 36 (`AMediaCodecStore_*` in `NdkMediaCodecStore.h`,
/// `AMediaCodecInfo_getMediaCodecInfoType` in `NdkMediaCodecInfo.h`), beyond the SDK's minSdk 29, and what it
/// answers for a type (`AMediaCodec_createDecoderByType`) is the platform's default, which can be a software
/// codec. The app's own `MediaCodecList` and `MediaCodecInfo.isHardwareAccelerated` give both at API 29.
/// Until the list is given, every stream goes to the software decoder.
pub(crate) fn set_device_codecs(codecs: Vec<CodecInfo>) {
    *DEVICE_CODECS.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(codecs);
}

/// The first of `names` the platform can create, in the device's own order; an error naming what is missing,
/// which is one wording for the decoders and the encoders alike.
fn creatable(names: &[&str], mime: &str, what: &str) -> std::result::Result<MediaCodec, Failure> {
    names.iter().find_map(|name| MediaCodec::from_codec_name(name)).ok_or_else(|| {
        if names.is_empty() { format!("this device has no {what} for {mime}") } else { format!("none of this device's {what}s for {mime} could be created") }
    })
}

/// The first of the device's hardware decoders for `mime`, in the device's own order, that the platform
/// creates; an error naming why there is none.
fn hardware_codec(mime: &str) -> std::result::Result<MediaCodec, Failure> {
    let codecs = DEVICE_CODECS.read().unwrap_or_else(std::sync::PoisonError::into_inner);
    listed_hardware_codec(mime, codecs.as_deref().ok_or("the device's codec list was not given")?)
}

/// [`hardware_codec`] from a given codec list.
fn listed_hardware_codec(mime: &str, codecs: &[CodecInfo]) -> std::result::Result<MediaCodec, Failure> {
    creatable(&mediacodec_input::hardware_decoders(codecs, mime), mime, "hardware decoder")
}

/// The first of the device's hardware encoders for `mime`, in the device's own order, that the platform
/// creates; an error naming why there is none. The export starts one exactly where the device's own answer
/// names one, and declines the movie where it does not (plan §18).
pub(crate) fn hardware_encoder(mime: &str) -> std::result::Result<MediaCodec, Failure> {
    let codecs = DEVICE_CODECS.read().unwrap_or_else(std::sync::PoisonError::into_inner);
    listed_hardware_encoder(mime, codecs.as_deref().ok_or("the device's codec list was not given")?)
}

/// [`hardware_encoder`] from a given codec list.
pub(crate) fn listed_hardware_encoder(mime: &str, codecs: &[CodecInfo]) -> std::result::Result<MediaCodec, Failure> {
    creatable(&mediacodec_input::hardware_encoders(codecs, mime), mime, "hardware encoder")
}

/// Whether this device has a hardware decoder for `mime`. It does not promise a decoder will start
/// (resources can be taken), only that starting one is worth trying.
pub(crate) fn has_hardware_decoder(mime: &str) -> bool {
    hardware_codec(mime).is_ok()
}

/// Whether this device has a hardware decoder for a type this decoder takes ([`BACKEND_MIMES`]), which is what
/// [`crate::register`] reports as its backend.
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
    /// Whether the codec has output anything, a picture or its output format, since it started.
    output: Cell<bool>,
}

// SAFETY: see the type's documentation.
unsafe impl Send for Pipeline {}

/// What a binding call gives, or `nothing` as the codec's error where it panics instead.
///
/// The bindings panic where the platform answers a buffer or a format with null, as it does once the codec
/// has failed (a decoder that cannot decode the stream's profile, an encoder that died). The failure lands on
/// the codec's own thread, so it can come after the buffer was dequeued, and the NDK has no state to ask
/// first; the bindings' `MediaCodec::input_buffer` and `output_buffer` answer `None` instead, but only by the
/// buffer's index, which `InputBuffer` and `OutputBuffer` keep to themselves. Catching the panic is what is
/// left, and it makes the failure the codec's error, never a crash.
pub(crate) fn caught<T>(call: impl FnOnce() -> T, nothing: &str) -> std::result::Result<T, Failure> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)).map_err(|_| nothing.to_string())
}

/// The memory of the input buffer the codec handed out, which the caller fills before queueing it.
pub(crate) fn input_memory<'a>(input: &'a mut InputBuffer<'_>) -> std::result::Result<&'a mut [std::mem::MaybeUninit<u8>], Failure> {
    caught(move || input.buffer_mut(), "the codec gave no input buffer memory")
}

/// A copy of what the codec wrote into an output buffer it handed out ([`mediacodec_output::written`]).
pub(crate) fn output_bytes(output: &OutputBuffer<'_>) -> std::result::Result<Vec<u8>, Failure> {
    let info = output.info();
    let buffer = caught(|| output.buffer(), "the codec gave no output buffer memory")?;
    crate::mediacodec_output::written(buffer, info.offset(), info.size()).map(<[u8]>::to_vec)
}

/// Copies `data` into the input buffer the codec handed out.
pub(crate) fn fill(input: &mut InputBuffer<'_>, data: &[u8]) -> std::result::Result<(), Failure> {
    let room = input_memory(input)?;
    let (target, _) = room.split_at_mut_checked(data.len()).ok_or("the sample is larger than the input buffer the codec handed out")?;
    for (to, from) in target.iter_mut().zip(data) {
        to.write(*from);
    }
    Ok(())
}

/// Queues one sample: waits up to [`STALL`] for an input buffer, lets `write` fill it and say how many bytes
/// it wrote, and queues them at `time` with `flags`. While the codec wants no input, `look` gets a turn to
/// take what has already come out, because a codec that is not read stops taking input.
pub(crate) fn queue_one(
    codec: &MediaCodec,
    time: u64,
    flags: u32,
    write: impl FnOnce(&mut InputBuffer<'_>) -> std::result::Result<usize, Failure>,
    mut look: impl FnMut() -> std::result::Result<(), Failure>,
) -> std::result::Result<(), Failure> {
    let deadline = Instant::now() + STALL;
    loop {
        match step(codec.dequeue_input_buffer(POLL), "waiting for an input buffer")? {
            DequeuedInputBufferResult::Buffer(mut input) => {
                let size = write(&mut input)?;
                step(codec.queue_input_buffer(input, 0, size, time, flags), "queueing the sample")?;
                return Ok(());
            }
            DequeuedInputBufferResult::TryAgainLater => {
                look()?;
                if Instant::now() >= deadline {
                    return Err("the codec took no input".into());
                }
            }
        }
    }
}

/// The image format the NDK makes a reader of `format` with. The P010 one is the HAL format of that name,
/// which the NDK's `AIMAGE_FORMAT_*` list has no entry for.
fn image_format(format: ReaderFormat) -> ImageFormat {
    match format {
        ReaderFormat::Yuv420 => ImageFormat::YUV_420_888,
        ReaderFormat::P010 => ImageFormat::from(i32::from(HardwareBufferFormat::YCbCr_P010)),
    }
}

/// The reader pictures of `depth` are decoded into ([`mediacodec_input::reader_format`]); an error naming its
/// format where the platform makes no such reader, which keeps the decoder from starting.
fn image_reader(width: i32, height: i32, depth: Depth) -> std::result::Result<ImageReader, Failure> {
    let format = mediacodec_input::reader_format(depth);
    let reader = ImageReader::new_with_usage(width, height, image_format(format), READ_USAGE, IMAGES);
    step(reader, &format!("creating the {format:?} image reader"))
}

impl Pipeline {
    /// `codec`, a decoder of `mime`, started for a picture of `layout`'s size from the codec-specific data
    /// `config` and decoding into a reader, in the layout that reader's planes describe
    /// ([`mediacodec_input::color_format`]).
    fn start(codec: MediaCodec, mime: &str, layout: &Layout, config: &[(&str, Vec<u8>)]) -> std::result::Result<Self, Failure> {
        let (width, height) = layout.size_i32().ok_or("the picture is too large")?;
        let reader = image_reader(width, height, layout.depth)?;
        let window = step(reader.window(), "getting the image reader's surface")?;
        let mut format = MediaFormat::new();
        format.set_str(MIME, mime);
        format.set_i32(WIDTH, width);
        format.set_i32(HEIGHT, height);
        format.set_i32(COLOR_FORMAT, mediacodec_input::color_format(mediacodec_input::reader_format(layout.depth)));
        for (key, data) in config {
            format.set_buffer(key, data);
        }
        step(codec.configure(&format, Some(&window), MediaCodecDirection::Decoder), "configuring the decoder")?;
        step(codec.start(), "starting the decoder")?;
        Ok(Self { codec, reader, output: Cell::new(false) })
    }

    /// Queues `data` as one input sample at the codec's `time` ([`mediacodec_input::codec_time`]), or the end
    /// of the stream when `end` is set (with no data). Pictures come out while it waits for a free input
    /// buffer, which it returns: the codec takes no input until its output is read.
    fn queue(&self, data: &[u8], time: u64, end: bool, layout: &Layout) -> std::result::Result<Vec<(i64, VideoFrame)>, Failure> {
        let mut pictures = Vec::new();
        queue_one(
            &self.codec,
            time,
            if end { END_OF_STREAM } else { 0 },
            |input| fill(input, data).map(|()| data.len()),
            || {
                pictures.extend(self.pictures(Duration::ZERO, layout)?.0);
                Ok(())
            },
        )?;
        Ok(pictures)
    }

    /// Discards every sample and picture the codec holds, for the codec to take the stream again from a
    /// sample decoding can start at. The codec takes input again at once and is not started again: Android
    /// asks for `start()` after `flush()` only of a codec in asynchronous mode, and this decoder registers no
    /// callback, so it runs in synchronous mode. A codec that has output nothing yet is not flushed, and the
    /// call fails, so the decoder drops it and the next sample starts a fresh one: Android's MediaCodec
    /// reference says a codec flushed that soon after its start may lose its codec-specific data, which a
    /// decoder that has lost it shows only as pictures that never come, with no error.
    fn flush(&self) -> std::result::Result<(), Failure> {
        if !self.output.get() {
            return Err("the decoder has output nothing to flush after".into());
        }
        step(self.codec.flush(), "flushing the decoder")
    }

    /// Every picture the codec has ready, waiting up to `wait` for the first; and whether the end of the
    /// stream came out, which no picture follows.
    fn pictures(&self, wait: Duration, layout: &Layout) -> std::result::Result<(Vec<(i64, VideoFrame)>, bool), Failure> {
        let mut pictures = Vec::new();
        loop {
            match step(self.codec.dequeue_output_buffer(wait), "waiting for a picture")? {
                DequeuedOutputBufferInfoResult::TryAgainLater => return Ok((pictures, false)),
                DequeuedOutputBufferInfoResult::OutputFormatChanged => self.output.set(true),
                DequeuedOutputBufferInfoResult::OutputBuffersChanged => {}
                DequeuedOutputBufferInfoResult::Buffer(output) => {
                    self.output.set(true);
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
    // The platform's own description of where the picture is in the image: the only answer there is, and
    // [`mediacodec_input::crop_of`] says when it cannot be trusted.
    let rect = step(image.crop_rect(), "reading the picture's crop")?;
    let crop = mediacodec_input::crop_of((rect.left, rect.top, rect.right, rect.bottom), layout)?;
    let pixels = match mediacodec_input::reader_format(layout.depth) {
        ReaderFormat::Yuv420 => yuv_420_888_pixels(image, crop)?,
        ReaderFormat::P010 => p010_pixels(&step(image.hardware_buffer(), "getting the picture's hardware buffer")?, crop)?,
    };
    Ok(layout.frame(pixels))
}

/// The 8-bit picture at `crop` in an image of the `YUV_420_888` reader, read from the three planes the platform
/// describes it as, and only once it does ([`mediacodec_input::described_planes`]).
fn yuv_420_888_pixels(image: &Image, crop: Crop) -> std::result::Result<PixelData, Failure> {
    let format = step(image.format(), "reading the picture's format")?;
    let planes = step(image.number_of_planes(), "counting the picture's planes")?;
    let strides = |index: i32| -> std::result::Result<(i32, i32), Failure> {
        Ok((step(image.plane_row_stride(index), "reading a plane's row stride")?, step(image.plane_pixel_stride(index), "reading a plane's pixel stride")?))
    };
    let described = if planes == 3 { [strides(0)?, strides(1)?, strides(2)?] } else { [(0, 0); 3] };
    let [luma, cb, cr] = mediacodec_input::described_planes(i32::from(format), planes, described)?;
    let plane = |index: i32, (stride, pixel): (i32, i32)| -> std::result::Result<Strided<'_>, Failure> {
        mediacodec_input::plane(index, step(image.plane_data(index), "reading a plane")?, stride, pixel)
    };
    Ok(biplanar::planar_to_pixels(mediacodec_input::luma(plane(0, luma)?)?, plane(1, cb)?, plane(2, cr)?, crop)?)
}

/// The 10-bit picture at `crop` in a decoded image's hardware `buffer`: an error unless the buffer is P010.
/// The buffer is locked for the CPU only while its planes are read into FilmCraft's own planes.
fn p010_pixels(buffer: &HardwareBuffer, crop: Crop) -> std::result::Result<PixelData, Failure> {
    let description = buffer.describe();
    if description.format != HardwareBufferFormat::YCbCr_P010 {
        return Err(format!("the decoded picture is in hardware buffer format {:?}, not P010", description.format));
    }
    let size = usize::try_from(description.width).and_then(|width| Ok((width, usize::try_from(description.height)?)));
    let (width, height) = size.map_err(|_| "the picture is too large".to_string())?;
    let planes: Vec<_> = step(buffer.lock_planes(READ_USAGE, None, None), "locking the picture's planes")?.collect();
    // SAFETY: the planes are the ones locking this buffer gave, and the buffer is `width` x `height` samples
    // by its own description; it stays locked for CPU reads, its planes mapped, until after the call returns.
    let pixels = unsafe { locked_pixels(&planes, width, height, crop) };
    let unlocked = step(buffer.unlock(), "unlocking the picture's planes");
    let pixels = pixels?;
    unlocked?;
    Ok(pixels)
}

/// The picture at `crop` in the planes of a locked P010 buffer of `width` x `height` samples. Planes that are
/// not P010's biplanar layout are an error, and nothing of them is read.
///
/// # Safety
///
/// The first two planes' addresses must each be readable, for as long as the call lasts, for the bytes
/// [`biplanar::p010_extents`] gives them from their strides and `width` x `height`, as the planes of a P010
/// buffer of that size are while it is locked for CPU reads.
unsafe fn locked_pixels(planes: &[LockedPlaneInfo], width: usize, height: usize, crop: Crop) -> std::result::Result<PixelData, Failure> {
    let locked = planes
        .iter()
        .map(|plane| {
            let strides = (usize::try_from(plane.bytes_per_stride), usize::try_from(plane.bytes_per_pixel));
            let (Ok(row_stride), Ok(pixel_stride)) = strides else { return Err("a plane has strides no address space holds".to_string()) };
            Ok(Locked { address: plane.virtual_address.addr(), row_stride, pixel_stride })
        })
        .collect::<std::result::Result<Vec<_>, Failure>>()?;
    let extents = biplanar::p010_extents(&locked, width, height)?;
    let (Some(luma), Some(chroma)) = (planes.first(), planes.get(1)) else { return Err("the picture has no planes".into()) };
    // SAFETY: the caller keeps both planes readable for these extents for the call, and the slices are read
    // into FilmCraft's own planes before it returns. `lockPlanes` reports no length; `p010_extents` bounds each
    // as Android bounds a P010 plane's data (`NdkImage.cpp`): the row stride times the rows before the last,
    // plus the last row's samples.
    let (luma, chroma) = unsafe {
        (
            std::slice::from_raw_parts(luma.virtual_address.cast::<u8>().cast_const(), extents.luma_len),
            std::slice::from_raw_parts(chroma.virtual_address.cast::<u8>().cast_const(), extents.chroma_len),
        )
    };
    let luma = Plane { data: luma, stride: extents.luma_stride };
    let chroma = Plane { data: chroma, stride: extents.chroma_stride };
    Ok(biplanar::to_pixels(luma, chroma, crop, Depth::Ten)?)
}

/// What a MediaCodec decoder feeds [`crate::order::Sequence`]: one codec, flushed at each seek and at the end
/// of the stream.
pub(crate) struct RawCodec {
    layout: Layout,
    stream: StreamInfo,
    mime: &'static str,
    make: MakeCodec,
    /// The codec-specific data a codec starts with.
    config: Vec<(&'static str, Vec<u8>)>,
    /// The codec the decoder started with, kept for its life: a seek or the end of a stream flushes it, which
    /// leaves it referring to no picture of the run before and holding none of its pictures. A codec that
    /// cannot flush, or has output nothing yet ([`Pipeline::flush`]), is dropped (`None`), and the next sample
    /// gets a fresh one.
    pipeline: Option<Pipeline>,
}

impl RawCodec {
    /// Ends the run: the codec is flushed, and dropped when it cannot be, so the next sample starts a fresh
    /// one.
    fn end_run(&mut self) {
        if self.pipeline.as_ref().is_some_and(|pipeline| pipeline.flush().is_err()) {
            self.pipeline = None;
        }
    }

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
        let data = mediacodec_input::input(&self.stream, sample)?;
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
        self.end_run();
    }

    fn drain(&mut self) -> std::result::Result<Vec<(i64, DecodedFrame)>, Failure> {
        let layout = self.layout.clone();
        // The end of the stream is queued as an input of its own; the codec outputs what it holds, then
        // the end. A stream that does not end in time, or any failed call, loses the tail the caller has no
        // other way to know of, so each is an error. A codec that has seen the end takes no more samples
        // until it is flushed, which is how a run ends and keeps the one codec; a codec that failed while
        // draining is never used again: the next run gets a codec of its own, whatever this drain answers.
        let drained = match &self.pipeline {
            Some(pipeline) => mediacodec_input::until_the_end(STALL, "decoder", || pipeline.queue(&[], 0, true, &layout), || pipeline.pictures(POLL, &layout)),
            None => Ok(Vec::new()),
        };
        match drained {
            Ok(pictures) => {
                self.end_run();
                Ok(decoded(pictures))
            }
            Err(why) => {
                self.pipeline = None;
                Err(why)
            }
        }
    }
}

/// Pictures as the decoder returns them.
fn decoded(pictures: Vec<(i64, VideoFrame)>) -> Vec<(i64, DecodedFrame)> {
    pictures.into_iter().map(|(pts, frame)| (pts, DecodedFrame { pts, frame, draft: false })).collect()
}

/// The MediaCodec decoder of one `avcC`, `hvcC` or `av1C` stream.
pub(crate) type McDecoder = HardwareDecoder<RawCodec>;

/// A decoder for the stream `entry` describes, whose parsed record is `stream`. It starts a codec now, so a stream or device
/// without a hardware decoder for it is refused here.
pub(crate) fn decoder(entry: &SampleEntry, stream: &StreamInfo) -> Result<McDecoder> {
    decoder_with(entry, stream, hardware_codec)
}

/// [`decoder`], with its codec made by `make`.
fn decoder_with(entry: &SampleEntry, stream: &StreamInfo, make: MakeCodec) -> Result<McDecoder> {
    let raw = raw_codec(entry, stream, make)?;
    let codec = raw.layout.codec;
    // MediaCodec outputs pictures in presentation order itself, so the reorder buffer holds none back.
    Ok(HardwareDecoder::new(raw, stream.clone(), 0, BACKEND, codec))
}

/// What [`decoder_with`] runs [`crate::order::Sequence`] around, its codec made by `make` and started.
fn raw_codec(entry: &SampleEntry, stream: &StreamInfo, make: MakeCodec) -> Result<RawCodec> {
    let layout = Layout::for_stream(stream).map_err(CodecError::Unsupported)?;
    let mime = mediacodec_input::mime(&layout).map_err(CodecError::Unsupported)?;
    let config = mediacodec_input::codec_specific_data(entry, stream).map_err(|error| CodecError::Unsupported(error.into()))?;
    let pipeline = make(mime).and_then(|codec| Pipeline::start(codec, mime, &layout, &config)).map_err(CodecError::Unsupported)?;
    Ok(RawCodec { layout, stream: stream.clone(), mime, make, config, pipeline: Some(pipeline) })
}

/// These run where a device's media codecs are: on an Android phone or emulator, which is where this module
/// builds. The device's codec list is the one the platform gives, handed over as the app hands it: a file the
/// harness writes before the run (see [`tests::device_codecs`]).
#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;
    use std::ops::Range;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, OnceLock};

    use filmcraft_codecs::hw::{FrameCodec, FrameStreamInfo, hw_stats};
    use filmcraft_codecs::{VideoDecoder, make_video_decoder, software_video_decoder};
    use filmcraft_color::ColorInfo;
    use filmcraft_frame::Chroma;
    use ndk::hardware_buffer::{HardwareBufferDesc, HardwareBufferRef};

    use super::*;
    use crate::fallback::Fallback;
    use crate::fixtures::{
        self, Branch, Stream, av1_entry, av1_samples, counters, decoded, h264_with_b_pictures_from_before_zero, hevc_entry, hevc_samples, hevc10_entry,
        hevc10_samples, pictures, reordered_entry, reordered_samples, stream, text, through_a_failed_drain, tiny_entry, tiny_samples, two_runs,
    };
    use crate::mediacodec_input::{AV1_MIME, H264_MIME, HEVC_MIME};
    use crate::platform::mediacodec_encoder::tests::{through_a_software_codec, through_the_hardware};

    /// Names the file the harness writes from `MediaCodecList(MediaCodecList.REGULAR_CODECS)`, one codec a line
    /// in the list's order, tab-separated: `getName()`, `isHardwareAccelerated()`, `isEncoder()`, then
    /// `getSupportedTypes()` joined by commas.
    const LIST_FILE: &str = "CRAFTBRIDGE_CODEC_LIST";

    /// The call `name` of the device's own `library`, looked up as the process runs: the way to a platform call
    /// the NDK's headers and stubs leave out. An error naming both where the device's library has no such call.
    ///
    /// # Safety
    ///
    /// `name` must be a call of `library` that takes no arguments and returns nothing.
    unsafe fn platform_call(library: &std::ffi::CStr, name: &std::ffi::CStr) -> std::result::Result<extern "C" fn(), String> {
        unsafe extern "C" {
            fn dlopen(file: *const std::ffi::c_char, mode: std::ffi::c_int) -> *mut std::ffi::c_void;
            fn dlsym(library: *mut std::ffi::c_void, name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        }
        /// Bionic's `RTLD_NOW`.
        const NOW: std::ffi::c_int = 2;
        // SAFETY: both names are NUL-terminated strings that outlive the calls, and a library dlopen gives is
        // never closed, so the address dlsym finds stays valid for the process's life.
        let address = unsafe {
            let opened = dlopen(library.as_ptr(), NOW);
            if opened.is_null() { std::ptr::null_mut() } else { dlsym(opened, name.as_ptr()) }
        };
        if address.is_null() {
            return Err(format!("the device's {} has no {}", library.to_string_lossy(), name.to_string_lossy()));
        }
        // SAFETY: the address is the call's, which the caller promises takes no arguments and returns nothing.
        Ok(unsafe { std::mem::transmute::<*mut std::ffi::c_void, extern "C" fn()>(address) })
    }

    /// Starts this process's binder thread pool, which an app's process has from its start and a test binary
    /// run from a shell has not. A hardware decoder runs in a vendor process that calls back into this one over
    /// binder: with no thread here to take those calls, the acceptance phone's decoders gave no picture and their
    /// release never returned. `ABinderProcess_startThreadPool` is `libbinder_ndk`'s call for it, which AOSP
    /// declares in a platform header (`binder_process.h`) the NDK leaves out.
    fn binder_threads() -> std::result::Result<(), String> {
        // SAFETY: `binder_process.h` declares `void ABinderProcess_startThreadPool(void)`.
        let start = unsafe { platform_call(c"libbinder_ndk.so", c"ABinderProcess_startThreadPool") }?;
        start();
        Ok(())
    }

    #[test]
    fn a_platform_call_the_devices_library_does_not_have_is_an_error_naming_both() {
        // SAFETY: the call is never made: the device's library has none of that name.
        let missing = unsafe { platform_call(c"libbinder_ndk.so", c"ABinderProcess_noSuchCall") };
        assert_eq!(missing.err(), Some("the device's libbinder_ndk.so has no ABinderProcess_noSuchCall".to_string()));
        // SAFETY: as above; no library of this name is on the device.
        let no_library = unsafe { platform_call(c"libcraftbridge_none.so", c"ABinderProcess_startThreadPool") };
        assert_eq!(no_library.err(), Some("the device's libcraftbridge_none.so has no ABinderProcess_startThreadPool".to_string()));
    }

    /// The device's codec list, as the app hands it over. A run without it would compare nothing, so the tests
    /// that need it fail instead. Every test that makes a codec reads it, so reading it first starts the binder
    /// thread pool an app's process has ([`binder_threads`]).
    pub(crate) fn device_codecs() -> &'static [CodecInfo] {
        static LIST: OnceLock<std::result::Result<Vec<CodecInfo>, String>> = OnceLock::new();
        let list = LIST.get_or_init(|| {
            binder_threads()?;
            let path = std::env::var_os(LIST_FILE).ok_or(format!("{LIST_FILE} names no file of the device's codec list"))?;
            let text = std::fs::read_to_string(&path).map_err(|error| format!("{LIST_FILE}: {error}"))?;
            text.lines()
                .map(|line| match line.split('\t').collect::<Vec<_>>()[..] {
                    [name, hardware_accelerated, encoder, types] => Ok(CodecInfo {
                        name: name.to_string(),
                        types: types.split(',').map(str::to_string).collect(),
                        encoder: encoder.parse().map_err(|error| format!("{line:?}: {error}"))?,
                        hardware_accelerated: hardware_accelerated.parse().map_err(|error| format!("{line:?}: {error}"))?,
                    }),
                    _ => Err(format!("{line:?} is not four tab-separated fields")),
                })
                .collect()
        });
        list.as_deref().unwrap_or_else(|why| panic!("{why}"))
    }

    /// Gives the decoder the device's codec list, as the app does before it registers the decoders.
    pub(crate) fn give_device_codecs() {
        set_device_codecs(device_codecs().to_vec());
    }

    /// The device's hardware decoder for `mime`, chosen from the device's list.
    fn listed(mime: &str) -> std::result::Result<MediaCodec, Failure> {
        listed_hardware_codec(mime, device_codecs())
    }

    /// The name of the first hardware decoder for `mime` the device's codec list names, the one [`listed`] makes
    /// where the platform creates it; `None` on a device whose list names none for the type, which declines every
    /// stream of it. Read from the list, not from a codec made: a listed decoder the platform could not create
    /// compared nothing either.
    fn hardware_name(mime: &str) -> Option<String> {
        mediacodec_input::hardware_decoders(device_codecs(), mime).first().map(|name| name.to_string())
    }

    /// [`crate::fixtures::hardware_compared`] for a `mime` stream on this device, whose hardware decoder for the
    /// type is the first its codec list names.
    fn hardware_compared(mime: &str, compared: usize) -> std::result::Result<(), String> {
        crate::fixtures::hardware_compared(mime, hardware_name(mime).as_deref(), compared)
    }

    /// [`crate::fixtures::hardware_start`] for a `mime` stream on this device, whose hardware decoder for the
    /// type is the first its codec list names.
    fn hardware_start<T>(mime: &str, start: std::result::Result<T, impl std::fmt::Debug>) -> std::result::Result<Option<T>, String> {
        crate::fixtures::hardware_start(mime, hardware_name(mime).as_deref(), start)
    }

    /// The device's hardware decoder for `entry`'s stream, which the factory declines the stream without: none
    /// where the device has no hardware decoder for its codec, and then there is nothing to compare, and a
    /// failed test where it has one that refuses the stream ([`hardware_start`]). The platform's H.264 software
    /// decoder is no stand-in: on the API 29 emulator, read through the image reader, a picture of it now and
    /// then carries an earlier picture's pixels, in about one run of these tests in ten and in most runs with
    /// the tests in parallel.
    fn device_decoders(entry: &SampleEntry, mime: &str) -> Vec<McDecoder> {
        hardware_start(mime, decoder_with(entry, &stream(entry), listed)).unwrap().into_iter().collect()
    }

    fn software(entry: &SampleEntry) -> Box<dyn VideoDecoder> {
        software_video_decoder(entry).unwrap()
    }

    #[test]
    fn the_decoder_picked_is_the_devices_first_hardware_one_for_the_type_and_never_a_software_default() {
        for mime in BACKEND_MIMES {
            let hardware = |name: &str| device_codecs().iter().any(|codec| codec.name == name && !codec.encoder && codec.hardware_accelerated);
            let first = device_codecs().iter().find(|codec| hardware(&codec.name) && codec.types.iter().any(|kind| kind == mime));
            match (listed(mime), first) {
                (Ok(codec), Some(first)) => assert_eq!(codec.name().unwrap(), first.name, "{mime}"),
                (Err(error), None) => assert_eq!(error, format!("this device has no hardware decoder for {mime}"), "{mime}"),
                (picked, first) => panic!("{mime}: picked {:?}, the platform lists {first:?} first", picked.map(|codec| codec.name())),
            }
            // The platform's default for the type is taken only where it is a hardware decoder itself.
            if let Some(default) = MediaCodec::from_decoder_type(mime).map(|codec| codec.name().unwrap()) {
                let picked = listed(mime).map(|codec| codec.name().unwrap());
                assert_eq!(picked.as_ref() == Ok(&default), hardware(&default), "{mime}: the default is {default}, the pick {picked:?}");
            }
        }
    }

    #[test]
    fn a_listed_hardware_decoder_the_platform_cannot_create_is_passed_over_for_the_next() {
        let missing = CodecInfo { name: "c2.craftbridge.missing.decoder".into(), types: vec![H264_MIME.into()], encoder: false, hardware_accelerated: true };
        assert_eq!(
            listed_hardware_codec(H264_MIME, std::slice::from_ref(&missing)).map(|_| ()),
            Err(format!("none of this device's hardware decoders for {H264_MIME} could be created"))
        );
        let after = [&[missing][..], device_codecs()].concat();
        assert_eq!(listed_hardware_codec(H264_MIME, &after).map(|codec| codec.name().unwrap()).ok(), listed(H264_MIME).map(|codec| codec.name().unwrap()).ok());
    }

    #[test]
    fn the_decoder_chooses_from_the_list_the_app_gave_and_declines_every_stream_until_it_is_given() {
        let _shared = counters();
        let given = |mime: &str| hardware_codec(mime).map(|_| ());
        let before = DEVICE_CODECS.write().unwrap().take();
        assert_eq!(given(H264_MIME), Err("the device's codec list was not given".to_string()));
        assert!(!available());
        assert!(matches!(decoder(&tiny_entry(), &stream(&tiny_entry())), Err(CodecError::Unsupported(_))));
        give_device_codecs();
        let expected = BACKEND_MIMES.map(|mime| listed(mime).map(|_| ()));
        assert_eq!(BACKEND_MIMES.map(given), expected);
        assert_eq!(available(), expected.iter().any(|answer| answer.is_ok()));
        assert_eq!(has_hardware_decoder(H264_MIME), expected[0].is_ok());
        *DEVICE_CODECS.write().unwrap() = before;
    }

    #[test]
    fn a_failed_call_is_the_decoders_error_naming_the_call_and_what_the_platform_said() {
        let failed: ndk::media_error::Result<()> = Err(ndk::media_error::MediaError::ErrorMalformed);
        assert_eq!(step(failed, "queueing the sample"), Err("queueing the sample failed (ErrorMalformed)".to_string()));
        assert_eq!(step(Ok::<_, ndk::media_error::MediaError>(3), "queueing the sample"), Ok(3));
    }

    #[test]
    fn the_device_decodes_h264_to_the_pictures_the_software_decoder_gives() {
        // The last stream starts two pictures before time zero, where negative composition offsets can put a
        // stream's first pictures: the codec hands every presentation time back as it was given.
        let before_zero = h264_with_b_pictures_from_before_zero().samples;
        for (entry, samples) in [(tiny_entry(), tiny_samples()), (reordered_entry(), reordered_samples()), (reordered_entry(), before_zero)] {
            for mut decoder in device_decoders(&entry, H264_MIME) {
                assert_eq!(decoder.name(), "MediaCodec H.264");
                assert_eq!(decoded(&mut decoder, &samples), decoded(software(&entry).as_mut(), &samples));
            }
        }
    }

    #[test]
    fn pictures_of_a_stream_with_b_pictures_come_out_in_presentation_order() {
        for mut decoder in device_decoders(&reordered_entry(), H264_MIME) {
            let order: Vec<i64> = pictures(&mut decoder, &reordered_samples()).iter().map(|picture| picture.pts).collect();
            assert_eq!(order, [0, 1, 2, 3, 4, 5]);
        }
    }

    #[test]
    fn a_flush_ends_the_run_and_the_decoder_takes_the_stream_again_from_its_start() {
        let entry = tiny_entry();
        let samples = tiny_samples();
        let wanted = decoded(software(&entry).as_mut(), &samples);
        for mut decoder in device_decoders(&entry, H264_MIME) {
            assert_eq!(decoded(&mut decoder, &samples), wanted);
            assert_eq!(decoded(&mut decoder, &samples), wanted, "after the end of the stream");
            assert!(decoder.flush().is_empty(), "nothing is held after a flush");
        }
    }

    /// `decoder` and the software decoder each fed samples `before` of `samples`, sought to the run of `length`
    /// samples that starts at `to`, and that run decoded: the decoder's pictures must be the software
    /// decoder's from that point, on the one codec.
    fn assert_seek_matches_software(
        decoder: &mut McDecoder,
        entry: &SampleEntry,
        samples: &[(i64, Vec<u8>)],
        (before, to, length): (Range<usize>, usize, usize),
    ) {
        let mut software = software(entry);
        for (pts, sample) in &samples[before.clone()] {
            decoder.decode(sample, *pts).unwrap();
            software.decode(sample, *pts).unwrap();
        }
        decoder.reset();
        software.reset();
        let seeked = &samples[to..to + length];
        let wanted = decoded(software.as_mut(), seeked);
        assert_eq!(decoded(decoder, seeked), wanted, "a seek to {to} after samples {before:?}");
    }

    #[test]
    fn a_seek_mid_stream_decodes_the_pictures_the_software_decoder_gives_from_that_point() {
        let entry = tiny_entry();
        let samples = two_runs();
        for mut decoder in device_decoders(&entry, H264_MIME) {
            assert_eq!(decoded(&mut decoder, &samples), decoded(software(&entry).as_mut(), &samples), "the whole stream");
            for (before, to) in [(0..3, 4), (4..7, 0), (0..2, 0), (4..8, 4)] {
                assert_seek_matches_software(&mut decoder, &entry, &samples, (before, to, 4));
            }
        }
    }

    #[test]
    fn a_seek_in_an_av1_stream_decodes_the_pictures_the_software_decoder_gives_from_that_point() {
        // The device's hardware AV1 decoder where it has one, and its own AV1 decoder, a software one included,
        // fed exactly as the hardware one is, so the seek is run on every device that decodes AV1.
        let entry = av1_entry();
        let samples = av1_samples();
        let own = decoder_with(&entry, &stream(&entry), any_codec).ok();
        for mut decoder in device_decoders(&entry, AV1_MIME).into_iter().chain(own) {
            assert_eq!(decoded(&mut decoder, &samples), decoded(software(&entry).as_mut(), &samples), "the whole stream");
            for before in [0..0, 0..1, 0..samples.len()] {
                assert_seek_matches_software(&mut decoder, &entry, &samples, (before, 0, samples.len()));
            }
        }
    }

    /// How many codecs [`counted`] has made.
    static MADE: AtomicUsize = AtomicUsize::new(0);

    /// [`listed`], counted in [`MADE`]: only the test that reads it uses it.
    fn counted(mime: &str) -> std::result::Result<MediaCodec, Failure> {
        MADE.fetch_add(1, Ordering::SeqCst);
        listed(mime)
    }

    #[test]
    fn a_decoder_keeps_its_one_codec_through_seeks_and_the_ends_of_streams() {
        let entry = reordered_entry();
        let samples = reordered_samples();
        let wanted = decoded(software(&entry).as_mut(), &samples);
        let Some(mut decoder) = hardware_start(H264_MIME, decoder_with(&entry, &stream(&entry), counted)).unwrap() else { return };
        assert_eq!(decoded(&mut decoder, &samples), wanted);
        for (pts, sample) in samples.iter().take(3) {
            decoder.decode(sample, *pts).unwrap();
        }
        decoder.reset();
        assert_eq!(decoded(&mut decoder, &samples), wanted, "after a seek");
        decoder.reset();
        assert_eq!(decoded(&mut decoder, &samples), wanted, "after the end of the stream and a seek");
        assert_eq!(MADE.load(Ordering::SeqCst), 1, "one codec for the decoder's life");
    }

    /// `samples` fed to `raw` and drained, as text: what [`decoded`] gives for a decoder, for a stream with no
    /// reordering.
    fn raw_pictures(raw: &mut RawCodec, samples: &[(i64, Vec<u8>)]) -> Vec<String> {
        let mut out: Vec<DecodedFrame> = samples.iter().flat_map(|(pts, sample)| raw.feed(sample, *pts).unwrap()).map(|(_, picture)| picture).collect();
        out.extend(raw.drain().unwrap().into_iter().map(|(_, picture)| picture));
        text(&out)
    }

    /// The device's hardware decoder for the tiny stream, outside [`crate::order::Sequence`]; none where the
    /// device has no hardware H.264 decoder ([`hardware_start`]).
    fn tiny_raw() -> Option<RawCodec> {
        hardware_start(H264_MIME, raw_codec(&tiny_entry(), &stream(&tiny_entry()), listed)).unwrap()
    }

    #[test]
    fn a_start_after_a_synchronous_flush_is_refused_and_the_codec_decodes_on() {
        // Android asks for `start()` after `flush()` only of a codec in asynchronous mode: in synchronous mode
        // the flushed codec is running already, and a start is an invalid operation. A decoder that started it
        // and minded the answer would lose its codec at every seek.
        let samples = tiny_samples();
        let wanted = decoded(software(&tiny_entry()).as_mut(), &samples);
        let Some(mut raw) = tiny_raw() else { return };
        assert_eq!(raw_pictures(&mut raw, &samples), wanted);
        assert!(raw.pipeline.as_ref().unwrap().codec.start().is_err(), "a start after the flush at the end of the stream");
        assert_eq!(raw_pictures(&mut raw, &samples), wanted, "after the refused start");
    }

    #[test]
    fn a_seek_before_the_codec_has_output_anything_decodes_the_run_after_it() {
        // A seek as soon as the decoder is made, as opening a movie part of the way in makes, and one straight
        // after the stream's first sample, as scrubbing makes.
        let entry = tiny_entry();
        let samples = tiny_samples();
        let wanted = decoded(software(&entry).as_mut(), &samples);
        for fed in [0, 1] {
            for mut decoder in device_decoders(&entry, H264_MIME) {
                for (pts, sample) in samples.iter().take(fed) {
                    decoder.decode(sample, *pts).unwrap();
                }
                decoder.reset();
                assert_eq!(decoded(&mut decoder, &samples), wanted, "a seek after {fed} samples");
            }
        }
    }

    #[test]
    fn a_codec_that_has_output_nothing_is_replaced_at_a_seek_and_one_that_has_is_flushed_and_kept() {
        // Android's MediaCodec reference: a codec flushed before it returns any output buffer or output format
        // may lose its codec-specific data, which an emulator's decoder does not show, so the rule is checked by
        // what the decoder keeps.
        let samples = tiny_samples();
        let wanted = decoded(software(&tiny_entry()).as_mut(), &samples);
        let Some(mut raw) = tiny_raw() else { return };
        raw.restart();
        assert!(raw.pipeline.is_none(), "a codec that has output nothing is not flushed");
        assert_eq!(raw_pictures(&mut raw, &samples), wanted, "a fresh codec takes the stream");
        raw.feed(&samples[0].1, samples[0].0).unwrap();
        raw.restart();
        assert!(raw.pipeline.is_some(), "a codec that has output is flushed and kept");
        assert_eq!(raw_pictures(&mut raw, &samples), wanted, "after the flush");
    }

    #[test]
    fn a_codec_that_cannot_flush_is_dropped_and_the_next_sample_gets_a_fresh_one() {
        let samples = tiny_samples();
        let wanted = decoded(software(&tiny_entry()).as_mut(), &samples);
        let Some(mut raw) = tiny_raw() else { return };
        assert_eq!(raw_pictures(&mut raw, &samples), wanted);
        raw.feed(&samples[0].1, samples[0].0).unwrap();
        // A stopped codec refuses a flush, as a codec that has failed does.
        raw.pipeline.as_ref().unwrap().codec.stop().unwrap();
        raw.restart();
        assert!(raw.pipeline.is_none(), "the codec that could not flush is gone");
        assert_eq!(raw_pictures(&mut raw, &samples), wanted, "a fresh codec takes the stream");
    }

    #[test]
    fn a_reset_forgets_the_run_so_far_and_the_next_sample_starts_another() {
        // The pictures the first samples of a stream with B pictures leave in the codec must not come out of the
        // next run.
        for (entry, samples) in [(tiny_entry(), tiny_samples()), (reordered_entry(), reordered_samples())] {
            for mut decoder in device_decoders(&entry, H264_MIME) {
                for (pts, sample) in samples.iter().take(3) {
                    decoder.decode(sample, *pts).unwrap();
                }
                decoder.reset();
                assert_eq!(decoded(&mut decoder, &samples), decoded(software(&entry).as_mut(), &samples));
            }
        }
    }

    #[test]
    fn a_sample_that_is_not_nal_units_is_an_error_the_decoder_survives() {
        let entry = tiny_entry();
        for mut decoder in device_decoders(&entry, H264_MIME) {
            assert!(matches!(decoder.decode(&[0, 0, 0, 9, 0x65], 0), Err(CodecError::Decode(_))));
            assert!(matches!(decoder.decode(&[], 0), Err(CodecError::Decode(_))));
            assert_eq!(decoded(&mut decoder, &tiny_samples()), decoded(software(&entry).as_mut(), &tiny_samples()));
        }
    }

    #[test]
    fn the_device_decodes_hevc_through_its_hardware_decoder_where_it_has_one() {
        let entry = hevc_entry();
        for mut decoder in device_decoders(&entry, HEVC_MIME) {
            assert_eq!(decoder.name(), "MediaCodec HEVC");
            assert_eq!(decoded(&mut decoder, &hevc_samples()), decoded(software(&entry).as_mut(), &hevc_samples()));
        }
    }

    #[test]
    fn the_device_decodes_av1_only_where_it_has_a_hardware_decoder_for_it() {
        let entry = av1_entry();
        let Some(mut decoder) = device_decoders(&entry, AV1_MIME).pop() else { return };
        assert_eq!(decoder.name(), "MediaCodec AV1");
        assert_eq!(decoded(&mut decoder, &av1_samples()), decoded(software(&entry).as_mut(), &av1_samples()));
    }

    #[test]
    fn an_av1_stream_fed_as_the_hardware_decoder_is_fed_decodes_to_the_pictures_the_software_decoder_gives() {
        // Where the device's AV1 decoder is software the factory declines the stream, so its own decoder takes
        // it with the same `av1C` as `csd-0`, the same temporal units and the same image reader.
        let entry = av1_entry();
        let decoder = decoder_with(&entry, &stream(&entry), any_codec);
        assert_eq!(decoder.is_ok(), any_codec(AV1_MIME).is_ok(), "a decoder starts wherever the device decodes AV1");
        let Ok(mut decoder) = decoder else { return };
        assert_eq!(decoder.name(), "MediaCodec AV1");
        let wanted = decoded(software(&entry).as_mut(), &av1_samples());
        assert_eq!(decoded(&mut decoder, &av1_samples()), wanted);
        assert_eq!(decoded(&mut decoder, &av1_samples()), wanted, "after the end of the stream");
        assert!(matches!(decoder.decode(&[], 0), Err(CodecError::Decode(_))), "an empty sample");
    }

    /// The device's own decoder for `mime`, a software one included, the first the list gives.
    fn any_codec(mime: &str) -> std::result::Result<MediaCodec, Failure> {
        let name = device_codecs().iter().find(|codec| !codec.encoder && codec.types.iter().any(|kind| kind == mime)).map(|codec| codec.name.as_str());
        name.and_then(MediaCodec::from_codec_name).ok_or_else(|| format!("this device has no decoder for {mime}"))
    }

    #[test]
    fn an_hevc_stream_fed_as_the_hardware_decoder_is_fed_decodes_to_the_pictures_the_software_decoder_gives() {
        // An emulator has no hardware HEVC decoder, so its own decoder, software or not, takes the stream with
        // the same codec-specific data, the same Annex B samples and the same image reader.
        let entry = hevc_entry();
        let decoder = decoder_with(&entry, &stream(&entry), any_codec);
        assert_eq!(decoder.is_ok(), any_codec(HEVC_MIME).is_ok(), "a decoder starts wherever the device decodes HEVC");
        let Ok(mut decoder) = decoder else { return };
        assert_eq!(decoder.name(), "MediaCodec HEVC");
        assert_eq!(decoded(&mut decoder, &hevc_samples()), decoded(software(&entry).as_mut(), &hevc_samples()));
    }

    #[test]
    fn an_input_buffer_a_codec_has_no_memory_for_is_the_decoders_error_never_a_panic() {
        // A codec stopped after handing out an input buffer has no memory for it, as a codec that has failed
        // on its own thread has none: the same call, met every time rather than in a race.
        let entry = tiny_entry();
        let layout = Layout::for_stream(&stream(&entry)).unwrap();
        let config = mediacodec_input::codec_specific_data(&entry, &stream(&entry)).unwrap();
        let pipeline = Pipeline::start(any_codec(H264_MIME).unwrap(), H264_MIME, &layout, &config).unwrap();
        let DequeuedInputBufferResult::Buffer(mut input) = pipeline.codec.dequeue_input_buffer(STALL).unwrap() else { panic!("no input buffer") };
        pipeline.codec.stop().unwrap();
        assert_eq!(fill(&mut input, &[0, 0, 0, 1]), Err("the codec gave no input buffer memory".to_string()));
    }

    /// Stops `raw`'s codec, if it has one, as a codec that fails on its own thread is stopped: every call a drain
    /// then makes fails, every time rather than in a race.
    fn stop_codec(raw: &RawCodec) {
        if let Some(pipeline) = &raw.pipeline {
            pipeline.codec.stop().unwrap();
        }
    }

    #[test]
    fn a_codec_that_fails_while_draining_is_the_decoders_error_and_is_never_used_again() {
        let (entry, samples) = (tiny_entry(), tiny_samples());
        let mut raw = raw_codec(&entry, &stream(&entry), any_codec).unwrap();
        for (pts, sample) in &samples {
            raw.feed(sample, *pts).unwrap();
        }
        stop_codec(&raw);
        assert!(raw.drain().is_err(), "the pictures the codec held are lost with it, which only an error tells");
        assert!(raw.pipeline.is_none(), "the codec whose drain failed is gone");
        // The next run gets a codec of its own, which decodes the stream whole.
        let codec = raw.layout.codec;
        let mut decoder = HardwareDecoder::new(raw, stream(&entry), 0, BACKEND, codec);
        assert_eq!(decoded(&mut decoder, &samples), decoded(software(&entry).as_mut(), &samples));
    }

    /// A MediaCodec raw decoder whose codec holds back the last `holds` pictures it decoded, as a codec with that
    /// latency does, and is stopped just before the sample at index `stop_at` is fed or, with none, just before
    /// the end of the stream is drained: that call fails, and every picture still held is lost with the codec,
    /// however soon the codec itself hands pictures back.
    struct Dies {
        raw: RawCodec,
        holds: usize,
        stop_at: Option<usize>,
        fed: usize,
        held: VecDeque<(i64, DecodedFrame)>,
    }

    impl Dies {
        fn new(raw: RawCodec, holds: usize, stop_at: Option<usize>) -> Self {
            Self { raw, holds, stop_at, fed: 0, held: VecDeque::new() }
        }

        fn stop(&mut self) {
            stop_codec(&self.raw);
            self.held.clear();
        }
    }

    impl RawDecoder<DecodedFrame> for Dies {
        fn feed(&mut self, sample: &[u8], pts: i64) -> std::result::Result<Vec<(i64, DecodedFrame)>, Failure> {
            if Some(self.fed) == self.stop_at {
                self.stop();
            }
            self.fed += 1;
            self.held.extend(self.raw.feed(sample, pts)?);
            let due = self.held.len().saturating_sub(self.holds);
            Ok(self.held.drain(..due).collect())
        }

        fn restart(&mut self) {
            self.held.clear();
            self.raw.restart();
        }

        fn drain(&mut self) -> std::result::Result<Vec<(i64, DecodedFrame)>, Failure> {
            if self.stop_at.is_none() {
                self.stop();
            }
            let drained = self.raw.drain()?;
            Ok(self.held.drain(..).chain(drained).collect())
        }
    }

    #[test]
    fn a_codec_that_fails_while_draining_gives_the_streams_whole_tail_through_fallback_from_the_software_decoder() {
        let _counters = counters();
        // The codec holds every picture of streams this short until the end, as the API 29 emulator's did, so the
        // failed drain loses the whole stream, the first run of two included.
        for (entry, samples) in [(tiny_entry(), tiny_samples()), (reordered_entry(), reordered_samples()), (tiny_entry(), two_runs())] {
            let raw = raw_codec(&entry, &stream(&entry), any_codec).unwrap();
            let codec = raw.layout.codec;
            through_a_failed_drain(&entry, &samples, HardwareDecoder::new(Dies::new(raw, usize::MAX, None), stream(&entry), 0, BACKEND, codec));
        }
    }

    #[test]
    fn a_codec_stopped_mid_stream_gives_the_whole_stream_back_through_fallback_from_the_software_decoder() {
        let _counters = counters();
        // The same codec dying mid-stream instead of at its end, three pictures late, with the pictures it held
        // gone: the replay reaches back to the restart sample before the oldest picture the caller lacks, and the
        // rest of the stream comes through the one switch, the same pictures the software decoder gives.
        let entry = tiny_entry();
        let samples = two_runs();
        let wanted = decoded(software(&entry).as_mut(), &samples);
        for stop_at in [1, 5, 7] {
            let fallbacks = hw_stats().fallbacks;
            let raw = raw_codec(&entry, &stream(&entry), any_codec).unwrap();
            let codec = raw.layout.codec;
            let hardware = HardwareDecoder::new(Dies::new(raw, 3, Some(stop_at)), stream(&entry), 0, BACKEND, codec);
            let mut decoder = Fallback::new(hardware, &entry, stream(&entry));
            assert_eq!(text(&pictures(&mut decoder, &samples)), wanted, "stopped before sample {stop_at}");
            assert_eq!(hw_stats().fallbacks, fallbacks + 1, "one switch, stopped before sample {stop_at}");
        }
    }

    #[test]
    fn streams_the_device_decoder_does_not_take_are_refused_before_it_is_asked() {
        let mut ten_bit_av1 = FrameStreamInfo::for_tests(FrameCodec::Av1, 0, 10, (1, 1), (640, 360));
        ten_bit_av1.color = Some(ColorInfo::REC709);
        assert!(matches!(decoder(&av1_entry(), &StreamInfo::Frame(ten_bit_av1)), Err(CodecError::Unsupported(_))), "10-bit AV1");
    }

    /// The 10-bit sample of [`p010_buffer`] at (x, y): luma, then the Cb and the Cr of the chroma pair at
    /// (x, y). The buffer is taller than it is wide, so a width and a height taken for each other read wrong
    /// samples.
    fn luma_at(x: usize, y: usize) -> u16 {
        ((y * 32 + x) % 1024) as u16
    }

    fn cb_at(x: usize, y: usize) -> u16 {
        ((300 + y * 16 + x) % 1024) as u16
    }

    fn cr_at(x: usize, y: usize) -> u16 {
        ((700 + y * 16 + x) % 1024) as u16
    }

    /// A P010 buffer of 32 x 48 samples holding [`luma_at`], [`cb_at`] and [`cr_at`], written through the plane
    /// layout the platform reports; `None` where the device allocates no such buffer. The samples are the
    /// picture's own 10-bit values, so the buffer holds each shifted up by six bits, as P010 does.
    fn p010_buffer() -> Option<HardwareBufferRef> {
        let description = HardwareBufferDesc {
            width: 32,
            height: 48,
            layers: 1,
            format: HardwareBufferFormat::YCbCr_P010,
            usage: HardwareBufferUsage::CPU_READ_OFTEN | HardwareBufferUsage::CPU_WRITE_OFTEN,
            stride: 0,
        };
        if !HardwareBuffer::is_supported(description) {
            return None;
        }
        let buffer = HardwareBuffer::allocate(description).ok()?;
        let planes: Vec<_> = buffer.lock_planes(HardwareBufferUsage::CPU_WRITE_OFTEN, None, None).ok()?.collect();
        let [luma, cb, cr] = planes.as_slice() else { panic!("a P010 buffer locks as three planes, not {}", planes.len()) };
        let word = |at: *mut std::ffi::c_void, plane: &LockedPlaneInfo, row: u32, column: u32, value: u16| {
            let offset = (row * plane.bytes_per_stride + column * plane.bytes_per_pixel) as usize;
            // SAFETY: the sample lies inside the locked buffer, 32 x 48 luma samples and 16 x 24 chroma pairs.
            unsafe { at.cast::<u8>().add(offset).cast::<u16>().write_unaligned(value << 6) };
        };
        for y in 0..48 {
            for x in 0..32 {
                word(luma.virtual_address, luma, y, x, luma_at(x as usize, y as usize));
            }
        }
        for y in 0..24 {
            for x in 0..16 {
                word(cb.virtual_address, cb, y, x, cb_at(x as usize, y as usize));
                word(cr.virtual_address, cr, y, x, cr_at(x as usize, y as usize));
            }
        }
        buffer.unlock().ok()?;
        Some(buffer)
    }

    /// The 10-bit planes of [`p010_buffer`] at `crop`: the luma and the Cb and Cr samples the crop holds.
    fn p010_planes(crop: Crop) -> [Vec<u16>; 3] {
        let (mut luma, mut cb, mut cr) = (Vec::new(), Vec::new(), Vec::new());
        for y in crop.y..crop.y + crop.height {
            luma.extend((crop.x..crop.x + crop.width).map(|x| luma_at(x, y)));
        }
        for y in crop.y / 2..(crop.y + crop.height).div_ceil(2) {
            cb.extend((crop.x / 2..(crop.x + crop.width).div_ceil(2)).map(|x| cb_at(x, y)));
            cr.extend((crop.x / 2..(crop.x + crop.width).div_ceil(2)).map(|x| cr_at(x, y)));
        }
        [luma, cb, cr]
    }

    /// The crops of [`p010_buffer`] the tests read, each with the pixels its picture is.
    fn p010_pictures() -> [(Crop, PixelData); 3] {
        [Crop { x: 0, y: 0, width: 32, height: 48 }, Crop { x: 2, y: 4, width: 15, height: 9 }, Crop { x: 0, y: 30, width: 31, height: 18 }]
            .map(|crop| (crop, PixelData::Yuv16 { planes: p010_planes(crop).map(Arc::new), chroma: Chroma::C420, bits: 10, alpha: None }))
    }

    #[test]
    fn a_p010_hardware_buffer_is_read_into_the_ten_bit_planes_the_software_decoder_makes() {
        let Some(buffer) = p010_buffer() else { return };
        for (crop, wanted) in p010_pictures() {
            // The device lays the planes out as it likes. The buffer's own size and the planes it locks to are
            // what the read must take, so reading them by hand gives the same answer: a layout other than
            // P010's biplanar one is declined by both, and the one this reads is read exactly.
            let planes: Vec<_> = buffer.lock_planes(READ_USAGE, None, None).unwrap().collect();
            // SAFETY: the planes are the ones locking this 32 x 48 buffer gave, and it stays locked until the
            // call has returned.
            let by_hand = unsafe { locked_pixels(&planes, 32, 48, crop) };
            buffer.unlock().unwrap();
            let read = p010_pixels(&buffer, crop);
            assert_eq!(format!("{read:?}"), format!("{by_hand:?}"), "{crop:?}");
            match read {
                Ok(pixels) => assert_eq!(format!("{pixels:?}"), format!("{wanted:?}"), "{crop:?}"),
                // Another layout is declined, and nothing else fails.
                Err(why) => assert_eq!(why, "the decoded picture's planes are not a P010 buffer's", "{crop:?}"),
            }
        }
    }

    #[test]
    fn the_planes_of_a_locked_buffer_in_plain_memory_are_read_with_their_padding_and_nothing_past_their_end() {
        // A P010 buffer of 32 x 48 samples with 8 words of padding after each luma row and 4 after each chroma
        // row, in memory whose end is the last chroma sample's: a read past the extents would leave it.
        let (luma_stride, chroma_stride) = (32 * 2 + 16, 16 * 4 + 8);
        let sample = |value: u16| (value << 6).to_ne_bytes();
        let mut memory: Vec<u8> = Vec::new();
        for y in 0..48 {
            memory.extend((0..luma_stride / 2).flat_map(|x| sample(if x < 32 { luma_at(x, y) } else { 1023 })));
        }
        let chroma_at = memory.len();
        for y in 0..24 {
            for x in 0..chroma_stride / 4 {
                let (cb, cr) = if x < 16 { (cb_at(x, y), cr_at(x, y)) } else { (1023, 1023) };
                memory.extend(sample(cb).into_iter().chain(sample(cr)));
            }
        }
        memory.truncate(chroma_at + chroma_stride * 23 + 16 * 4);
        let at = |offset: usize, row_stride: usize, pixel_stride: u32| LockedPlaneInfo {
            virtual_address: memory.get(offset..).unwrap().as_ptr().cast_mut().cast(),
            bytes_per_pixel: pixel_stride,
            bytes_per_stride: row_stride as u32,
        };
        let planes = [at(0, luma_stride, 2), at(chroma_at, chroma_stride, 4), at(chroma_at + 2, chroma_stride, 4)];
        // SAFETY: every plane points into `memory`, which holds the extents of a 32 x 48 buffer with these
        // strides. A call with other planes or the size turned round is refused before anything is read.
        let read = |planes: &[LockedPlaneInfo], (width, height): (usize, usize), crop| unsafe { locked_pixels(planes, width, height, crop) };
        for (crop, wanted) in p010_pictures() {
            assert_eq!(format!("{:?}", read(&planes, (32, 48), crop).unwrap()), format!("{wanted:?}"), "{crop:?}");
        }
        let planar_chroma = [planes[0], LockedPlaneInfo { bytes_per_pixel: 2, ..planes[1] }, planes[2]];
        let whole = Crop { x: 0, y: 0, width: 32, height: 48 };
        assert!(read(&planar_chroma, (32, 48), whole).is_err(), "planar chroma");
        assert!(read(&planes[..2], (32, 48), whole).is_err(), "two planes");
        assert!(read(&planes, (32, 48), Crop { width: 33, ..whole }).is_err(), "a crop wider than the buffer");
        assert!(read(&planes, (48, 32), whole).is_err(), "a width and a height taken for each other");
    }

    /// The device's API level, as the platform's own property names it.
    fn api_level() -> u32 {
        let property = std::process::Command::new("getprop").arg("ro.build.version.sdk").output().unwrap();
        String::from_utf8(property.stdout).unwrap().trim().parse().unwrap()
    }

    /// Whether the platform makes the reader a 10-bit picture is decoded into.
    fn p010_reader_made() -> bool {
        image_reader(32, 32, Depth::Ten).is_ok()
    }

    #[test]
    fn an_8_bit_picture_gets_a_yuv_420_888_reader_and_a_10_bit_one_the_p010_reader_from_android_14_and_none_before() {
        // The NDK header's AHARDWAREBUFFER_FORMAT_YCbCr_P010.
        assert_eq!(i32::from(image_format(ReaderFormat::P010)), 0x36);
        assert_eq!(image_reader(32, 32, Depth::Eight).unwrap().format().unwrap(), ImageFormat::YUV_420_888);
        match image_reader(32, 32, Depth::Ten) {
            Ok(reader) => assert_eq!(reader.format().unwrap(), image_format(ReaderFormat::P010)),
            Err(error) => assert_eq!(error, "creating the P010 image reader failed (ErrorInvalidParameter)"),
        }
        assert_eq!(p010_reader_made(), api_level() >= 34, "AImageReader takes the P010 format from Android 14 (API 34)");
    }

    #[test]
    fn a_hardware_buffer_that_is_not_p010_is_an_error_naming_its_format() {
        let description = HardwareBufferDesc {
            width: 32,
            height: 32,
            layers: 1,
            format: HardwareBufferFormat::R8G8B8A8_UNORM,
            usage: HardwareBufferUsage::CPU_READ_OFTEN | HardwareBufferUsage::CPU_WRITE_OFTEN,
            stride: 0,
        };
        let buffer = HardwareBuffer::allocate(description).unwrap();
        let error = p010_pixels(&buffer, Crop { x: 0, y: 0, width: 32, height: 32 }).unwrap_err();
        assert_eq!(error, "the decoded picture is in hardware buffer format R8G8B8A8_UNORM, not P010");
    }

    /// A decoder of the 10-bit HEVC stream, made over the device's hardware HEVC decoder where its codec list
    /// names one, and otherwise over its own HEVC decoder, a software one included; or the error that started
    /// none.
    fn ten_bit_decoder() -> (SampleEntry, std::result::Result<McDecoder, CodecError>) {
        let entry = hevc10_entry();
        let make: MakeCodec = if hardware_name(HEVC_MIME).is_some() { listed } else { any_codec };
        let decoder = decoder_with(&entry, &stream(&entry), make);
        (entry, decoder)
    }

    /// How many of the device decoder's own pictures of the 10-bit stream one run through `start` compared with
    /// the software decoder's (`wanted`), each equal to it. A start that was refused compares none, and is the
    /// error [`hardware_start`] makes of it where the device has a hardware HEVC decoder. A run whose decoder
    /// failed on a sample compares none either: the registry's decoder answers the failure by replaying the stream
    /// in software, whose pictures must be the software decoder's, byte for byte, and none of them is the device
    /// decoder's.
    fn ten_bit_run(start: std::result::Result<McDecoder, CodecError>, samples: &[(i64, Vec<u8>)], wanted: &[String]) -> std::result::Result<usize, String> {
        let Some(mut decoder) = hardware_start(HEVC_MIME, start)? else { return Ok(0) };
        let outcome: std::result::Result<Vec<_>, _> = samples.iter().map(|(pts, sample)| decoder.decode(sample, *pts)).collect();
        let Ok(decoded_samples) = outcome else {
            drop(decoder);
            let (entry, again) = ten_bit_decoder();
            if let Some(again) = hardware_start(HEVC_MIME, again)? {
                let mut declined = Fallback::new(again, &entry, stream(&entry));
                assert_eq!(decoded(&mut declined, samples), wanted, "a decline is byte-identical");
            }
            return Ok(0);
        };
        let mut pictures: Vec<_> = decoded_samples.into_iter().flatten().collect();
        pictures.extend(decoder.flush());
        if !pictures.is_empty() {
            assert_eq!(text(&pictures), wanted, "the device gave YCbCr_P010");
        }
        Ok(pictures.len())
    }

    #[test]
    fn a_start_that_was_refused_compares_no_picture_and_fails_where_the_device_has_a_hardware_decoder_for_the_type() {
        let refused = || decoder_with(&hevc10_entry(), &stream(&hevc10_entry()), |_| Err("the codec refused the stream".into()));
        // Where the rule applies is read from the device's codec list itself, not through the rule's own helpers.
        let rule = |mime: &str| {
            let first = mediacodec_input::hardware_decoders(device_codecs(), mime).first().copied();
            first.map(|name| format!("{mime}: the device's hardware decoder {name} gave no picture compared with the software decoder's"))
        };
        let why = "Unsupported(\"the codec refused the stream\")";
        let refusal = |mime: &str| rule(mime).map(|rule| format!("{rule}: its start was refused ({why})"));
        for mime in BACKEND_MIMES {
            assert_eq!(hardware_compared(mime, 0), rule(mime).map_or(Ok(()), Err), "{mime}");
            assert_eq!(hardware_compared(mime, 1), Ok(()), "{mime}");
            let start = hardware_start(mime, refused()).map(|decoder| decoder.is_some());
            assert_eq!(start, refusal(mime).map_or(Ok(false), Err), "{mime}");
        }
        assert_eq!(ten_bit_run(refused(), &hevc10_samples(), &[]), refusal(HEVC_MIME).map_or(Ok(0), Err), "a refused 10-bit start compares nothing");
    }

    #[test]
    fn a_10_bit_hevc_stream_decodes_to_the_pictures_the_software_decoder_gives_or_is_declined() {
        // Before Android 14 there is no reader for a 10-bit picture, so no decoder starts, which the factory
        // answers by declining: the software decoder takes the stream from its first sample.
        if !p010_reader_made() {
            let Err(CodecError::Unsupported(why)) = ten_bit_decoder().1 else { panic!("a 10-bit decoder started without a P010 reader") };
            assert_eq!(why, "creating the P010 image reader failed (ErrorInvalidParameter)");
            return;
        }
        // From it, the device's decoder takes the stream with the same codec-specific data, the same Annex B
        // samples and the P010 reader. Where that decoder gives P010 buffers, its pictures are the software
        // decoder's, frame by frame. Where it gives none (a decoder that does not decode 10-bit video, or that
        // makes another buffer format), a sample is an error, which the registry's decoder answers by replaying
        // the stream in software. A codec that has failed has no input buffer memory to give, and that must be
        // the same error, so the run repeats to meet a codec in that state. A codec that fails only after the
        // last sample was fed loses its pictures without an error, which a run cannot tell from no pictures, so
        // a run that ends so compares nothing. Where the device has a hardware HEVC decoder, its pictures must
        // be compared; a device with none may decline the stream at its start, as before Android 14.
        // A failed run is replayed through `Fallback`, whose switch a process-wide counter counts.
        let _counters = counters();
        let samples = hevc10_samples();
        let wanted = decoded(software(&hevc10_entry()).as_mut(), &samples);
        let compared: std::result::Result<usize, String> = (0..6).map(|_| ten_bit_run(ten_bit_decoder().1, &samples, &wanted)).sum();
        hardware_compared(HEVC_MIME, compared.unwrap()).unwrap();
    }

    /// Why the device's hardware decoder refuses a picture of `stream`: the error of the first sample it fails,
    /// `None` where it decodes every sample.
    fn first_refusal(stream: &Stream) -> Option<String> {
        let mut decoder = decoder_with(&stream.entry, &fixtures::stream(&stream.entry), listed).unwrap();
        let refused = stream.samples.iter().find_map(|(pts, sample)| decoder.decode(sample, *pts).err());
        decoder.flush();
        refused.map(|error| error.to_string())
    }

    /// `stream` opened through FilmCraft's registry, as an engine's start leaves it ([`crate::register`] over the
    /// device's list), every picture held to the software decoder's: the branch it took. It is the device's: the
    /// first hardware decoder the list names for the stream's type, with no picture from another decoder and no
    /// switch to the software one, wherever the list names one and the platform can make the stream's pictures
    /// (a 10-bit one needs the P010 reader, [`p010_reader_made`]); the software decoder, the decline counted,
    /// everywhere else. One more is allowed: a decoder that describes the window of a crop that starts off the
    /// coded picture's corner as a decoder that ignores the offset does ([`mediacodec_input::IGNORED_OFFSET`])
    /// is refused at the stream's first picture, naming the corner the stream's own record gives its crop, and
    /// the stream goes to the software decoder, the switch counted; any other refusal fails the test. The branch
    /// is held to what the registry did, so the branch the test says is the one taken.
    fn through_the_registry(stream: &Stream) -> Branch {
        let layout = Layout::for_stream(&fixtures::stream(&stream.entry)).unwrap();
        let mime = mediacodec_input::mime(&layout).unwrap();
        let hardware = hardware_name(mime).filter(|_| layout.depth == Depth::Eight || p010_reader_made());
        let refusal = hardware.as_ref().and_then(|_| first_refusal(stream));
        let backend = format!("{BACKEND} {}", layout.codec.name());
        let (declined, fallbacks) = (hw_stats().declined, hw_stats().fallbacks);
        let mut decoder = make_video_decoder(&stream.entry).unwrap();
        let opened = decoder.name().to_string();
        let compared = fixtures::compared_with_software(stream.name, decoder.as_mut(), &stream.entry, &stream.samples);
        assert_eq!(compared, stream.samples.len(), "{}: a picture for each sample", stream.name);
        let did = (opened.as_str(), decoder.name(), hw_stats().declined, hw_stats().fallbacks);
        let branch = match (hardware, refusal) {
            (Some(name), None) => Branch::Hardware(format!("{name} ({backend})")),
            (Some(name), Some(why)) => {
                // The corner the stream's own record gives its crop, not the layout's reading of it.
                let corner = match fixtures::stream(&stream.entry) {
                    StreamInfo::Nal(info) => (info.crop.0, info.crop.1),
                    StreamInfo::Frame(_) => (0, 0),
                };
                let reason = format!("not where the stream's crop starts, {},{}: {}", corner.0, corner.1, mediacodec_input::IGNORED_OFFSET);
                assert!(why.contains(&reason), "{}: {name} refused a picture, and not with {reason:?}: {why}", stream.name);
                Branch::Switched(format!("{name} ({backend})"), why)
            }
            (None, _) => Branch::Declined(
                decoder_with(&stream.entry, &fixtures::stream(&stream.entry), listed).err().map_or_else(|| "no hardware decoder".into(), |why| why.to_string()),
            ),
        };
        // What the registry does on each branch: the decoder it opens the stream with and the one it ends with,
        // and the declines and switches it counts.
        let software = software_video_decoder(&stream.entry).unwrap();
        let does = match &branch {
            Branch::Hardware(_) => (backend.as_str(), backend.as_str(), declined, fallbacks),
            Branch::Switched(..) => (backend.as_str(), software.name(), declined, fallbacks + 1),
            Branch::Declined(_) => (software.name(), software.name(), declined + 1, fallbacks),
        };
        let said = branch.says(stream.name, compared);
        assert_eq!(did, does, "the registry took another branch than this: {said}");
        eprintln!("{said}");
        branch
    }

    /// T23's footage on this device: the running example's movie as the device's hardware encoder writes it where
    /// its list names one for H.264, and its own software encoder where not, as an emulator's list leaves none.
    fn running_example_footage() -> Stream {
        let hardware = mediacodec_input::hardware_encoders(device_codecs(), H264_MIME).first().copied();
        let start = if hardware.is_some() { through_the_hardware } else { through_a_software_codec };
        eprintln!(
            "T23's footage: encoded by {}",
            hardware.map_or("the device's software encoder, its list naming no hardware one".into(), |name| format!("hardware encoder {name}"))
        );
        fixtures::running_example_footage(start).unwrap()
    }

    /// T23 on this device: the running example's footage and the streams of the set, each opened through the
    /// registry and held to the device's branch, the two offset crops among them: each is the window the software
    /// decoder shows or is refused for its decoder's description of it, and says which.
    #[test]
    fn t23_the_running_examples_footage_and_the_set_take_the_devices_branch_through_the_registry() {
        let _counters = counters();
        give_device_codecs();
        let footage = running_example_footage();
        assert_eq!(footage.samples.len() as u64, fixtures::T23_FRAMES);
        crate::register(true);
        let set = [
            fixtures::h264(),
            fixtures::h264_with_b_pictures(),
            fixtures::h264_with_b_pictures_from_before_zero(),
            fixtures::hevc(),
            fixtures::hevc10(),
            fixtures::av1(),
            fixtures::h264_with_an_offset_crop(),
            fixtures::hevc_with_an_offset_crop(),
        ];
        for stream in set.iter().chain([&footage]) {
            through_the_registry(stream);
        }
    }
}
