//! The iOS hardware video decoder: VideoToolbox's `VTDecompressionSession` for `avcC` (H.264), `hvcC`
//! (HEVC) and, where the device decodes it, `av1C` (AV1) streams.
//!
//! The session requires a hardware decoder, so a device or stream without one fails to start and the
//! factory declines it (the iOS Simulator has none). Samples go in one at a time and each is decoded before
//! the call returns, which keeps the pictures VideoToolbox emits, in decoding order, inside the call that fed
//! the sample: [`crate::order::Sequence`] then puts them in presentation order, as FilmCraft's software decoders emit them,
//! and each picture is read out of its surface into the planar layout those decoders produce
//! ([`biplanar`]). A surface is the coded picture from its corner, reported at the picture's size with the rest
//! of the coded picture held to the right and below as extended pixels, so the picture is read at its crop,
//! wherever the crop starts. Every status VideoToolbox, CoreMedia and CoreVideo return is checked and becomes
//! the decoder's error, which [`crate::Fallback`] answers by switching to the software decoder.
//!
//! FilmCraft's own VideoToolbox decoder, `filmcraft_platform::videotoolbox::VtDecoder`, is built for macOS
//! only, in the crate whose wrapper [`crate::Fallback`] does not use for the dependencies it brings; this one
//! follows its behaviour on iOS in code of its own: the streams it declines, a session per decoding run, the
//! RASL pictures a run leaves out, the reorder buffer and the copy out of a biplanar surface, bar the crop of a
//! surface of the picture's size, which `VtDecoder` reads from the surface's corner: for a crop that starts off
//! the coded picture's corner, that is another window of the picture.
//!
//! The rules for this module's `unsafe` (AGENTS.md §4): every block carries a `// SAFETY:` comment, no
//! pointer leaves the module, and nothing here panics, in the output callback least of all, because
//! VideoToolbox calls it on its own thread and a panic there would cross the framework boundary.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, PoisonError};

use filmcraft_codecs::hw::{NalCodec, NalStreamInfo, StreamInfo};
use filmcraft_codecs::{CodecError, DecodedFrame, Result};
use filmcraft_frame::VideoFrame;
use filmcraft_isobmff::SampleEntry;
use objc2_core_foundation::{CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_media::{
    CMBlockBuffer, CMFormatDescription, CMSampleBuffer, CMSampleTimingInfo, CMTime, CMTimeFlags, CMVideoFormatDescriptionCreate,
    CMVideoFormatDescriptionCreateFromH264ParameterSets, CMVideoFormatDescriptionCreateFromHEVCParameterSets, kCMBlockBufferAssureMemoryNowFlag,
    kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms, kCMVideoCodecType_AV1,
};
use objc2_core_video::{
    CVImageBuffer, CVPixelBuffer, CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetExtendedPixels,
    CVPixelBufferGetHeight, CVPixelBufferGetHeightOfPlane, CVPixelBufferGetPixelFormatType, CVPixelBufferGetPlaneCount, CVPixelBufferGetWidth,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress, kCVPixelBufferPixelFormatTypeKey,
};
use objc2_video_toolbox::{
    VTDecodeFrameFlags, VTDecodeInfoFlags, VTDecompressionOutputCallbackRecord, VTDecompressionSession, VTIsHardwareDecodeSupported,
    kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder,
};

use super::check;
use crate::biplanar::{self, Plane};
use crate::layout::{self, Codec, Layout};
use crate::order::{HardwareDecoder, RawDecoder};

/// A failed call, with what failed.
type Failure = String;

/// The backend's name, which [`crate::register`] reports and every decoder's name starts with.
pub(crate) const BACKEND: &str = "VideoToolbox";

/// Whether this device has a hardware decoder for `codec`. It does not promise a session will start
/// (resources can be taken), only that starting one is worth trying.
pub(crate) fn hardware_available(codec: Codec) -> bool {
    // SAFETY: a query that takes a four-character code and returns a flag.
    unsafe { VTIsHardwareDecodeSupported(codec.fourcc()) }
}

/// Whether this device has a hardware decoder for H.264 or HEVC, the codecs an iPhone's hardware decodes,
/// which is what [`crate::register`] reports as its backend.
pub(crate) fn available() -> bool {
    hardware_available(Codec::H264) || hardware_available(Codec::Hevc)
}

/// A decompression session, which moves between threads with the decoder that owns it.
///
/// A decompression session may be used from any thread (Apple: "thread safe"), and the decoder uses it
/// from one thread at a time (`&mut self`). objc2 marks the format description `Send` and `Sync` itself but
/// not the session, so this type says it for the session alone.
struct SessionRef(CFRetained<VTDecompressionSession>);

// SAFETY: see the type's documentation.
unsafe impl Send for SessionRef {}

/// The format description of a stream, from its parameter sets or its `av1C`.
fn format_description(entry: &SampleEntry, stream: &StreamInfo, layout: &Layout) -> std::result::Result<CFRetained<CMFormatDescription>, Failure> {
    let (out, status) = match stream {
        StreamInfo::Nal(info) => from_parameter_sets(info)?,
        StreamInfo::Frame(_) => from_av1c(entry, layout)?,
    };
    check(status, "creating the format description")?;
    let description = NonNull::new(out.cast_mut()).ok_or("CoreMedia returned no format description")?;
    // SAFETY: a `Create` function returns its object retained once, and `CFRetained` takes that over.
    Ok(unsafe { CFRetained::from_raw(description) })
}

/// A H.264 or HEVC format description made from the record's parameter sets, which CoreMedia copies.
fn from_parameter_sets(info: &NalStreamInfo) -> std::result::Result<(*const CMFormatDescription, i32), Failure> {
    if info.parameter_sets.is_empty() {
        return Err("the stream has no parameter sets".into());
    }
    let mut pointers: Vec<NonNull<u8>> = info.parameter_sets.iter().map(|set| NonNull::from(set.as_slice()).cast()).collect();
    let mut sizes: Vec<usize> = info.parameter_sets.iter().map(Vec::len).collect();
    let length_size = i32::try_from(info.length_size).map_err(|_| "the NAL unit length size is out of range")?;
    let mut out: *const CMFormatDescription = std::ptr::null();
    let (pointers_ptr, sizes_ptr) = (NonNull::from(pointers.as_mut_slice()).cast(), NonNull::from(sizes.as_mut_slice()).cast());
    // SAFETY: `pointers` and `sizes` each hold one entry per parameter set, every pointer addresses its
    // set's bytes and every size is that set's length, all alive for the call; CoreMedia copies them, and
    // writes the new description to `out`.
    let status = unsafe {
        match info.codec {
            NalCodec::H264 => {
                CMVideoFormatDescriptionCreateFromH264ParameterSets(None, pointers.len(), pointers_ptr, sizes_ptr, length_size, NonNull::from(&mut out))
            }
            NalCodec::Hevc => {
                CMVideoFormatDescriptionCreateFromHEVCParameterSets(None, pointers.len(), pointers_ptr, sizes_ptr, length_size, None, NonNull::from(&mut out))
            }
        }
    };
    Ok((out, status))
}

/// An AV1 format description: the codec, the picture's size and the sample entry's `av1C` record as the
/// sample description extension atom CoreMedia reads it from.
fn from_av1c(entry: &SampleEntry, layout: &Layout) -> std::result::Result<(*const CMFormatDescription, i32), Failure> {
    let record = CFData::from_bytes(&layout::av1c_record(entry)?);
    let (width, height) = layout.size_i32().ok_or("the picture is too large")?;
    let atom = CFString::from_static_str("av1C");
    let atoms = CFDictionary::<CFString, CFData>::from_slices(&[&atom], &[&record]);
    // SAFETY: reading a framework constant, which is never written.
    let atoms_key: &CFString = unsafe { kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms };
    let extensions = CFDictionary::<CFString, CFDictionary<CFString, CFData>>::from_slices(&[atoms_key], &[&atoms]);
    let mut out: *const CMFormatDescription = std::ptr::null();
    // SAFETY: `extensions` is a dictionary of the type CoreMedia reads, alive for the call, which copies it;
    // the new description is written to `out`.
    let status = unsafe { CMVideoFormatDescriptionCreate(None, kCMVideoCodecType_AV1, width, height, Some(extensions.as_ref()), NonNull::from(&mut out)) };
    Ok((out, status))
}

/// A sample for VideoToolbox: a copy of `sample` in a CoreMedia block, timed at `pts`.
fn sample_buffer(sample: &[u8], pts: i64, format: &CMFormatDescription) -> std::result::Result<CFRetained<CMSampleBuffer>, Failure> {
    let source = NonNull::new(sample.as_ptr().cast_mut().cast::<c_void>()).filter(|_| !sample.is_empty()).ok_or("the sample is empty")?;
    let mut block: *mut CMBlockBuffer = std::ptr::null_mut();
    // SAFETY: with no memory block given, CoreMedia allocates `sample.len()` bytes itself, now, and writes
    // the new block buffer to `block`.
    let status = unsafe {
        CMBlockBuffer::create_with_memory_block(
            None,
            std::ptr::null_mut(),
            sample.len(),
            None,
            std::ptr::null(),
            0,
            sample.len(),
            kCMBlockBufferAssureMemoryNowFlag,
            NonNull::from(&mut block),
        )
    };
    check(status, "creating the block buffer")?;
    let block = NonNull::new(block).ok_or("CoreMedia returned no block buffer")?;
    // SAFETY: a `Create` function returns its object retained once, and `CFRetained` takes that over.
    let block = unsafe { CFRetained::from_raw(block) };
    // SAFETY: the block holds `sample.len()` bytes and `source` addresses as many, which are only read.
    check(unsafe { CMBlockBuffer::replace_data_bytes(source, &block, 0, sample.len()) }, "copying the sample")?;
    let untimed = CMTime { value: 0, timescale: 0, flags: CMTimeFlags(0), epoch: 0 };
    let timing = CMSampleTimingInfo {
        duration: untimed,
        presentationTimeStamp: CMTime { value: pts, timescale: 1, flags: CMTimeFlags::Valid, epoch: 0 },
        decodeTimeStamp: untimed,
    };
    let size = sample.len();
    let mut buffer: *mut CMSampleBuffer = std::ptr::null_mut();
    // SAFETY: one sample, with one timing entry and one size entry, both addressing locals that outlive the
    // call; the sample buffer retains the block and the format description.
    let status = unsafe { CMSampleBuffer::create_ready(None, Some(&block), Some(format), 1, 1, &timing, 1, &size, NonNull::from(&mut buffer)) };
    check(status, "creating the sample buffer")?;
    let buffer = NonNull::new(buffer).ok_or("CoreMedia returned no sample buffer")?;
    // SAFETY: a `Create` function returns its object retained once, and `CFRetained` takes that over.
    Ok(unsafe { CFRetained::from_raw(buffer) })
}

/// A decoded picture's surface with its base address locked for reading, unlocked when dropped.
struct Locked<'a>(&'a CVPixelBuffer);

impl<'a> Locked<'a> {
    fn new(surface: &'a CVPixelBuffer) -> std::result::Result<Self, Failure> {
        // SAFETY: `surface` is a pixel buffer that stays alive for the call, and the lock is undone by the
        // guard's drop, with the same flags.
        let status = unsafe { CVPixelBufferLockBaseAddress(surface, CVPixelBufferLockFlags::ReadOnly) };
        check(status, "locking the decoded picture")?;
        Ok(Self(surface))
    }

    /// Plane `index`, readable while the guard lives, with its share of the `below` rows of extended pixels the
    /// surface has under its `height` rows: all of them for a plane of the surface's height, half for 4:2:0
    /// chroma.
    fn plane(&self, index: usize, below: usize, height: usize) -> std::result::Result<Plane<'_>, Failure> {
        let base = CVPixelBufferGetBaseAddressOfPlane(self.0, index).cast::<u8>().cast_const();
        let stride = CVPixelBufferGetBytesPerRowOfPlane(self.0, index);
        let rows = CVPixelBufferGetHeightOfPlane(self.0, index);
        let extended = below.checked_mul(rows).and_then(|share| share.checked_div(height));
        let size = extended.and_then(|extended| rows.checked_add(extended)).and_then(|rows| stride.checked_mul(rows));
        let size = size.ok_or("the decoded picture's plane size overflows")?;
        if base.is_null() || size == 0 {
            return Err(format!("the decoded picture's plane {index} is not mapped"));
        }
        // SAFETY: while the base address is locked CoreVideo maps `bytes per row` bytes for each of a plane's
        // rows and of the extended rows below them from the plane's base address, and the slice borrows the
        // guard, so it cannot outlive the lock.
        let data = unsafe { std::slice::from_raw_parts(base, size) };
        Ok(Plane { data, stride })
    }
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        // SAFETY: `new` locked this surface with the same flags, and it is still alive.
        unsafe { CVPixelBufferUnlockBaseAddress(self.0, CVPixelBufferLockFlags::ReadOnly) };
    }
}

/// The extended pixels to the right of and below a surface's own size, as columns and rows: where VideoToolbox
/// holds the rest of the coded picture. Those to the left and above lie before the surface's origin and are never
/// read. CoreVideo documents extended pixels only as padding the surface's memory holds, and the clean aperture
/// (`CVImageBufferGetCleanRect`) as the part of a surface that is the picture; VideoToolbox's surface attaches no
/// clean aperture, so its clean rectangle is the whole surface, wherever the crop starts. The crop therefore comes
/// from the stream, and the extended pixels say whether the surface holds the coded picture it is a window of.
fn extended_pixels(surface: &CVPixelBuffer) -> (usize, usize) {
    let (mut left, mut right, mut top, mut bottom) = (0, 0, 0, 0);
    // SAFETY: `surface` is a pixel buffer that stays alive for the call, which writes the four counts.
    unsafe { CVPixelBufferGetExtendedPixels(surface, &mut left, &mut right, &mut top, &mut bottom) };
    (right, bottom)
}

/// The picture in a decoded surface, as the frame FilmCraft's software decoder makes for the stream.
fn read_picture(surface: &CVPixelBuffer, layout: &Layout) -> std::result::Result<VideoFrame, Failure> {
    let format = CVPixelBufferGetPixelFormatType(surface);
    if format != layout.pixel_format || CVPixelBufferGetPlaneCount(surface) != 2 {
        return Err(format!("the decoded picture is in format {format:#x}, not {:#x}", layout.pixel_format));
    }
    let (width, height) = (CVPixelBufferGetWidth(surface), CVPixelBufferGetHeight(surface));
    let extended = extended_pixels(surface);
    let crop = layout.crop_in((width, height), extended)?;
    let below = extended.1;
    let locked = Locked::new(surface)?;
    let pixels = biplanar::to_pixels(locked.plane(0, below, height)?, locked.plane(1, below, height)?, crop, layout.depth)?;
    Ok(layout.frame(pixels))
}

/// What VideoToolbox's output callback produced for one sample.
enum Outcome {
    Picture(DecodedFrame),
    Failed(Failure),
}

/// What the output callback needs, and where it leaves its results.
struct CallbackState {
    layout: Layout,
    outcomes: Mutex<Vec<Outcome>>,
}

/// The output callback: VideoToolbox calls it, on a thread of its own, once for each sample it was fed.
/// `frame_tag` is the sample's presentation time, which the decoder passed as the frame's reference.
///
/// It never unwinds into VideoToolbox: a panic while reading a picture becomes a failed outcome.
unsafe extern "C-unwind" fn output_callback(
    state: *mut c_void,
    frame_tag: *mut c_void,
    status: i32,
    flags: VTDecodeInfoFlags,
    image: *mut CVImageBuffer,
    _pts: CMTime,
    _duration: CMTime,
) {
    // SAFETY: `state` is the `CallbackState` the session was made with, which its `Session` holds a strong
    // reference to until the session is invalidated, after which no callback runs; it is only read through
    // a shared reference, and its outcomes are behind a `Mutex`.
    let Some(state) = (unsafe { state.cast::<CallbackState>().as_ref() }) else { return };
    let pts = frame_tag.addr() as i64;
    let read = catch_unwind(AssertUnwindSafe(|| {
        check(status, "decoding the sample")?;
        if flags.contains(VTDecodeInfoFlags::FrameDropped) {
            return Err("VideoToolbox dropped the picture".into());
        }
        // SAFETY: VideoToolbox passes a pixel buffer that stays alive for the call, or null.
        let surface = unsafe { image.as_ref() }.ok_or("VideoToolbox returned no picture")?;
        read_picture(surface, &state.layout)
    }));
    let outcome = match read {
        Ok(Ok(frame)) => Outcome::Picture(DecodedFrame { pts, frame, draft: false }),
        Ok(Err(failure)) => Outcome::Failed(failure),
        Err(_) => Outcome::Failed("reading a decoded picture panicked".into()),
    };
    state.outcomes.lock().unwrap_or_else(PoisonError::into_inner).push(outcome);
}

/// A decompression session with the state its callback reads.
struct Session {
    session: SessionRef,
    format: CFRetained<CMFormatDescription>,
    /// The callback's state, which the session points to: it outlives the session. An `Arc`, not a `Box`,
    /// because a `Box` asserts unique access to its contents each time it moves, which would invalidate
    /// the pointer the session holds; and `Arc` makes the compiler hold the state to `Sync`, as the callback
    /// reads it from VideoToolbox's thread.
    state: Arc<CallbackState>,
}

impl Session {
    /// A session decoding `format` into `layout`'s surface format, which must be a hardware one.
    fn new(format: &CFRetained<CMFormatDescription>, layout: &Layout) -> std::result::Result<Self, Failure> {
        // SAFETY: reading a framework constant, which is never written.
        let require: &CFString = unsafe { kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder };
        let specification = CFDictionary::<CFString, CFType>::from_slices(&[require], &[CFBoolean::new(true).as_ref()]);
        // SAFETY: reading a framework constant, which is never written.
        let format_key: &CFString = unsafe { kCVPixelBufferPixelFormatTypeKey };
        let surface = CFNumber::new_i32(layout.pixel_format as i32);
        let attributes = CFDictionary::<CFString, CFType>::from_slices(&[format_key], &[surface.as_ref()]);
        let state = Arc::new(CallbackState { layout: layout.clone(), outcomes: Mutex::new(Vec::new()) });
        let callback = VTDecompressionOutputCallbackRecord {
            decompressionOutputCallback: Some(output_callback),
            decompressionOutputRefCon: Arc::as_ptr(&state).cast_mut().cast::<c_void>(),
        };
        let mut session: *mut VTDecompressionSession = std::ptr::null_mut();
        // SAFETY: every argument is valid for the call, which copies the callback record and the
        // dictionaries; the record's state is the `Arc` returned in the `Session`, which keeps it alive until
        // the session is invalidated.
        let status = unsafe {
            VTDecompressionSession::create(None, format, Some(specification.as_ref()), Some(attributes.as_ref()), &callback, NonNull::from(&mut session))
        };
        check(status, "creating the decompression session")?;
        let session = NonNull::new(session).ok_or("VideoToolbox returned no session")?;
        // SAFETY: a `Create` function returns its object retained once, and `CFRetained` takes that over.
        let session = SessionRef(unsafe { CFRetained::from_raw(session) });
        Ok(Self { session, format: format.clone(), state })
    }

    /// Decodes `sample`, whose presentation time is `pts`, and returns once the callback has run for it.
    fn decode(&self, sample: &[u8], pts: i64) -> std::result::Result<(), Failure> {
        let buffer = sample_buffer(sample, pts, &self.format)?;
        let mut info = VTDecodeInfoFlags(0);
        // Without the asynchronous flag the call returns once the sample is decoded; the pts travels as the
        // frame's reference, which is an integer here and never dereferenced.
        let tag = std::ptr::without_provenance_mut::<c_void>(pts as usize);
        // SAFETY: the session and the sample buffer are valid, and `info` is a flag word the call writes.
        let status = unsafe { self.session.0.decode_frame(&buffer, VTDecodeFrameFlags::empty(), tag, &mut info) };
        check(status, "decoding the sample")?;
        // SAFETY: the session is valid.
        check(unsafe { self.session.0.wait_for_asynchronous_frames() }, "waiting for the decoded picture")
    }

    /// What the callback produced since the last call.
    fn take(&self) -> Vec<Outcome> {
        std::mem::take(&mut *self.state.outcomes.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: the session is valid until it is released. Waiting lets a callback that is running
        // finish, and invalidating ends them, so the state they read can be freed with the `Arc` afterwards.
        unsafe {
            self.session.0.wait_for_asynchronous_frames();
            self.session.0.invalidate();
        }
    }
}

/// What a VideoToolbox decoder feeds [`crate::order::Sequence`]: the session, made again after a seek.
pub(crate) struct RawSession {
    layout: Layout,
    format: CFRetained<CMFormatDescription>,
    /// `None` until the first sample of a run: each run, after a seek, gets a session of its own, so no
    /// reference picture of an earlier one survives.
    session: Option<Session>,
}

impl RawDecoder<DecodedFrame> for RawSession {
    fn feed(&mut self, sample: &[u8], pts: i64) -> std::result::Result<Vec<(i64, DecodedFrame)>, Failure> {
        if self.session.is_none() {
            self.session = Some(Session::new(&self.format, &self.layout)?);
        }
        // A session that failed is kept, with its reference pictures, as the software decoder keeps its state
        // after an error: a fresh one fed the next sample would decode it from references it never saw.
        let session = self.session.as_ref().ok_or("there is no decompression session")?;
        let decoded = session.decode(sample, pts);
        let outcomes = session.take();
        decoded?;
        outcomes
            .into_iter()
            .map(|outcome| match outcome {
                Outcome::Picture(picture) => Ok((picture.pts, picture)),
                Outcome::Failed(failure) => Err(failure),
            })
            .collect()
    }

    fn restart(&mut self) {
        self.session = None;
    }
}

/// The VideoToolbox decoder of one `avcC`, `hvcC` or `av1C` stream.
pub(crate) type VtDecoder = HardwareDecoder<RawSession>;

/// A decoder for the stream `entry` describes, whose parsed record is `stream`. It starts a session now, so a
/// stream or device without a hardware decoder for it is refused here.
pub(crate) fn decoder(entry: &SampleEntry, stream: &StreamInfo) -> Result<VtDecoder> {
    let layout = Layout::for_stream(stream).map_err(CodecError::Unsupported)?;
    if !hardware_available(layout.codec) {
        return Err(CodecError::Unsupported("this device has no hardware decoder for the stream's codec".into()));
    }
    let format = format_description(entry, stream, &layout).map_err(CodecError::Unsupported)?;
    let session = Session::new(&format, &layout).map_err(CodecError::Unsupported)?;
    let (depth, codec) = (layout.reorder, layout.codec);
    let raw = RawSession { layout, format, session: Some(session) };
    Ok(HardwareDecoder::new(raw, stream.clone(), depth, BACKEND, codec))
}

/// These run where VideoToolbox decodes, on an iPhone and in the iOS Simulator: each stream of the media tests'
/// set, and T23's footage, through a real session, frame by frame against FilmCraft's software decoder. Each
/// test says which branch it took. Where VideoToolbox has a hardware decoder for the codec, every picture is
/// the session's; where it has none (the Simulator, which has none at all, and an iPhone whose chip decodes no
/// AV1), the start is refused, the factory declines the stream, and the registry's software decoder must give
/// the same pictures.
#[cfg(test)]
mod tests {
    use filmcraft_codecs::hw::hw_stats;
    use filmcraft_codecs::{CodecError, VideoDecoder, make_video_decoder, software_video_decoder};

    use super::*;
    use crate::HardwareEncoder;
    use crate::fixtures::{self, Branch, Stream, counters};

    /// The type of a stream of `codec` as CoreMedia names it: its four-character code.
    fn kind(codec: Codec) -> String {
        String::from_utf8_lossy(&codec.fourcc().to_be_bytes()).into_owned()
    }

    /// The name of this device's hardware decoder for `codec`, the name its decoders carry; `None` where
    /// VideoToolbox says the device has none.
    fn hardware_name(codec: Codec) -> Option<String> {
        hardware_available(codec).then(|| format!("{BACKEND} {}", codec.name()))
    }

    /// [`fixtures::hardware_start`] for a stream of `codec` on this device, whose hardware decoder for the
    /// codec is the one VideoToolbox says it has.
    fn hardware_start(codec: Codec, start: Result<VtDecoder>) -> std::result::Result<Option<VtDecoder>, String> {
        fixtures::hardware_start(&kind(codec), hardware_name(codec).as_deref(), start)
    }

    fn codec_of(entry: &SampleEntry) -> Codec {
        Layout::for_stream(&fixtures::stream(entry)).unwrap().codec
    }

    /// The decoder a test compares `entry`'s stream through, and its branch: the device's hardware decoder, in a
    /// session of its own; or, where the start is refused on a device with no hardware decoder for the codec,
    /// the decoder FilmCraft's registry opens the stream with once the factory has declined it, the decline
    /// counted. On a device with a hardware decoder for the codec a refused start fails the test, naming the
    /// type and the decoder. The caller holds [`counters`], since the decline counter is process-wide.
    fn on_this_device(entry: &SampleEntry) -> (Box<dyn VideoDecoder>, Branch) {
        let codec = codec_of(entry);
        let start = decoder(entry, &fixtures::stream(entry));
        let why = start.as_ref().err().map(ToString::to_string).unwrap_or_default();
        if let Some(hardware) = hardware_start(codec, start).unwrap() {
            let name = hardware.name().to_string();
            return (Box::new(hardware), Branch::Hardware(name));
        }
        crate::register(true);
        let declined = hw_stats().declined;
        let through_registry = make_video_decoder(entry).unwrap();
        let software = software_video_decoder(entry).unwrap();
        assert_eq!((through_registry.name(), hw_stats().declined), (software.name(), declined + 1), "the factory declined the stream");
        (through_registry, Branch::Declined(why))
    }

    /// The pictures of `stream` on this device, frame by frame against the software decoder's: the branch it
    /// took, which [`settle`] says and holds to the device's.
    fn check(stream: &Stream) -> Branch {
        let (mut decoder, branch) = on_this_device(&stream.entry);
        let compared = fixtures::compared_with_software(stream.name, decoder.as_mut(), &stream.entry, &stream.samples);
        settle(stream, &branch, compared);
        branch
    }

    /// Says `branch` of `stream`, `compared` pictures compared, and fails a run that compared none where the
    /// device has a hardware decoder for the codec, or one whose branch is not the device's: the hardware one,
    /// through the device's hardware decoder, exactly where VideoToolbox says it has one for the codec, so what a
    /// test says of its branch is what it took.
    fn settle(stream: &Stream, branch: &Branch, compared: usize) {
        let codec = codec_of(&stream.entry);
        let hardware = hardware_name(codec);
        fixtures::hardware_compared(&kind(codec), hardware.as_deref(), compared).unwrap();
        let taken = match branch {
            Branch::Hardware(decoder) => Some(decoder.as_str()),
            Branch::Declined(_) | Branch::Switched(..) => None,
        };
        assert_eq!(taken, hardware.as_deref(), "{}: the branch is the device's for the codec", stream.name);
        eprintln!("{}", branch.says(stream.name, compared));
    }

    #[test]
    fn a_refused_start_fails_where_videotoolbox_has_a_hardware_decoder_for_the_codec_naming_the_type_and_the_decoder() {
        let why = "the session refused the stream";
        let refused = || Err(CodecError::Unsupported(why.into()));
        for (codec, fourcc, decoder) in
            [(Codec::H264, *b"avc1", "VideoToolbox H.264"), (Codec::Hevc, *b"hvc1", "VideoToolbox HEVC"), (Codec::Av1, *b"av01", "VideoToolbox AV1")]
        {
            // Where the rule applies is VideoToolbox's own answer, asked here, not through the tests' helpers.
            // SAFETY: a query that takes a four-character code and returns a flag.
            let device_has_one = unsafe { VTIsHardwareDecodeSupported(u32::from_be_bytes(fourcc)) };
            let kind = String::from_utf8_lossy(&fourcc).into_owned();
            let rule = format!(
                "{kind}: the device's hardware decoder {decoder} gave no picture compared with the software decoder's: its start was refused (Unsupported({why:?}))"
            );
            let start = hardware_start(codec, refused()).map(|decoder| decoder.is_some());
            assert_eq!(start, if device_has_one { Err(rule) } else { Ok(false) }, "{kind}");
        }
    }

    #[test]
    fn h264_through_a_real_session_gives_the_pictures_the_software_decoder_gives() {
        let _counters = counters();
        for stream in [fixtures::h264(), fixtures::h264_with_b_pictures(), fixtures::h264_with_b_pictures_from_before_zero()] {
            check(&stream);
        }
    }

    #[test]
    fn hevc_with_b_pictures_through_a_real_session_gives_the_pictures_the_software_decoder_gives() {
        let _counters = counters();
        check(&fixtures::hevc());
    }

    #[test]
    fn ten_bit_hevc_with_b_pictures_through_a_real_session_gives_the_pictures_the_software_decoder_gives() {
        let _counters = counters();
        check(&fixtures::hevc10());
    }

    #[test]
    fn a_crop_that_starts_off_the_corner_gives_the_window_the_software_decoder_gives() {
        let _counters = counters();
        for stream in [fixtures::h264_with_an_offset_crop(), fixtures::hevc_with_an_offset_crop()] {
            check(&stream);
        }
    }

    #[test]
    fn a_seek_to_an_hevc_cra_gives_the_pictures_the_software_decoder_gives_from_it() {
        let _counters = counters();
        let stream = fixtures::hevc();
        let StreamInfo::Nal(info) = fixtures::stream(&stream.entry) else { panic!("HEVC is a stream of NAL units") };
        // A CRA in mid-stream (21), whose RASL picture refers back past it and is left out of a run that starts there.
        let cra = stream.samples.iter().position(|(_, sample)| info.nal_types(sample).contains(&21)).filter(|&at| at > 0).unwrap();
        let (mut decoder, branch) = on_this_device(&stream.entry);
        let from_the_cra = Stream { name: "HEVC from its CRA", entry: stream.entry.clone(), samples: stream.samples[cra..].to_vec() };
        // Sought to from the stream's start, from part way through the run before it, and from its end.
        for before in [0..0, 0..3, 0..stream.samples.len()] {
            for (pts, sample) in &stream.samples[before.clone()] {
                decoder.decode(sample, *pts).unwrap();
            }
            decoder.reset();
            let compared = fixtures::compared_with_software(from_the_cra.name, decoder.as_mut(), &stream.entry, &from_the_cra.samples);
            assert_eq!(compared, from_the_cra.samples.len() - 1, "after samples {before:?}: every picture from the CRA on but its RASL picture");
            settle(&from_the_cra, &branch, compared);
        }
    }

    #[test]
    fn av1_takes_the_decline_path_where_the_chip_has_no_av1_decoder() {
        let _counters = counters();
        // `check` holds the branch to VideoToolbox's answer for AV1, so a declined stream is one on a chip with none.
        if let Branch::Declined(why) = check(&fixtures::av1()) {
            assert_eq!(why, "unsupported codec: this device has no hardware decoder for the stream's codec");
        }
    }

    /// T23's footage, made by the device's hardware encoder. `None` on the Simulator when its Mac lends it no
    /// H.264 encoder; a phone has one.
    fn running_example_footage() -> Option<Stream> {
        match fixtures::running_example_footage(HardwareEncoder::start) {
            Ok(footage) => Some(footage),
            Err(error) if cfg!(target_abi = "sim") => {
                eprintln!("T23: the Simulator's Mac lends it no H.264 encoder to make the footage with ({error})");
                None
            }
            Err(error) => panic!("a phone has a hardware H.264 encoder: {error}"),
        }
    }

    #[test]
    fn t23_the_running_examples_footage_gives_the_pictures_the_software_decoder_gives() {
        let _counters = counters();
        let Some(footage) = running_example_footage() else { return };
        assert_eq!(footage.samples.len() as u64, fixtures::T23_FRAMES);
        let (mut decoder, branch) = on_this_device(&footage.entry);
        let compared = fixtures::compared_with_software(footage.name, decoder.as_mut(), &footage.entry, &footage.samples);
        assert_eq!(compared as u64, fixtures::T23_FRAMES, "every frame");
        settle(&footage, &branch, compared);
    }
}
