//! The iOS hardware video decoder: VideoToolbox's `VTDecompressionSession` for `avcC` (H.264), `hvcC`
//! (HEVC) and, where the device decodes it, `av1C` (AV1) streams.
//!
//! The session requires a hardware decoder, so a device or stream without one fails to start and the
//! factory declines it (the iOS Simulator has none). Samples go in one at a time and each is decoded before
//! the call returns, which keeps the pictures VideoToolbox emits, in decoding order, inside the call that fed
//! the sample: [`Sequence`] then puts them in presentation order, as FilmCraft's software decoders emit them,
//! and each picture is read out of its surface into the planar layout those decoders produce
//! ([`biplanar`]). Every status VideoToolbox, CoreMedia and CoreVideo return is checked and becomes the
//! decoder's error, which [`crate::Fallback`] answers by switching to the software decoder.
//!
//! FilmCraft's own VideoToolbox decoder, `filmcraft_platform::videotoolbox::VtDecoder`, is built for macOS
//! only, in the crate whose wrapper [`crate::Fallback`] does not use for the dependencies it brings; this one
//! follows its behaviour on iOS in code of its own: the streams it declines, a session per decoding run, the
//! RASL pictures a run leaves out, the reorder buffer and the copy out of a biplanar surface.
//!
//! The rules for this module's `unsafe` (AGENTS.md §4): every block carries a `// SAFETY:` comment, no
//! pointer leaves the module, and nothing here panics, in the output callback least of all, because
//! VideoToolbox calls it on its own thread and a panic there would cross the framework boundary.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, PoisonError};

use filmcraft_codecs::hw::{NalCodec, NalStreamInfo, StreamInfo};
use filmcraft_codecs::{CodecError, DecodedFrame, Result, VideoDecoder};
use filmcraft_frame::VideoFrame;
use filmcraft_isobmff::{CodecConfig, SampleEntry};
use objc2_core_foundation::{CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_media::{
    CMBlockBuffer, CMFormatDescription, CMSampleBuffer, CMSampleTimingInfo, CMTime, CMTimeFlags, CMVideoFormatDescriptionCreate,
    CMVideoFormatDescriptionCreateFromH264ParameterSets, CMVideoFormatDescriptionCreateFromHEVCParameterSets, kCMBlockBufferAssureMemoryNowFlag,
    kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms, kCMVideoCodecType_AV1,
};
use objc2_core_video::{
    CVImageBuffer, CVPixelBuffer, CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeight,
    CVPixelBufferGetHeightOfPlane, CVPixelBufferGetPixelFormatType, CVPixelBufferGetPlaneCount, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress, kCVPixelBufferPixelFormatTypeKey,
};
use objc2_video_toolbox::{
    VTDecodeFrameFlags, VTDecodeInfoFlags, VTDecompressionOutputCallbackRecord, VTDecompressionSession, VTIsHardwareDecodeSupported,
    kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder,
};

use crate::biplanar::{self, Plane};
use crate::layout::{Codec, Layout};
use crate::order::{RawDecoder, Sequence};

/// A failed call, with what failed.
type Failure = String;

/// `Ok` when a CoreMedia, CoreVideo or VideoToolbox `status` says success.
fn check(status: i32, call: &str) -> std::result::Result<(), Failure> {
    if status == 0 { Ok(()) } else { Err(format!("{call} failed ({status})")) }
}

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
    let CodecConfig::Av1(config) = &entry.codec else {
        return Err("the sample entry has no av1C".into());
    };
    let (width, height) = layout.size();
    let (width, height) = (i32::try_from(width), i32::try_from(height));
    let (Ok(width), Ok(height)) = (width, height) else {
        return Err("the picture is too large".into());
    };
    let record = CFData::from_bytes(&config.to_bytes());
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

    /// Plane `index`, readable while the guard lives.
    fn plane(&self, index: usize) -> std::result::Result<Plane<'_>, Failure> {
        let base = CVPixelBufferGetBaseAddressOfPlane(self.0, index).cast::<u8>().cast_const();
        let stride = CVPixelBufferGetBytesPerRowOfPlane(self.0, index);
        let rows = CVPixelBufferGetHeightOfPlane(self.0, index);
        let size = stride.checked_mul(rows).ok_or("the decoded picture's plane size overflows")?;
        if base.is_null() || size == 0 {
            return Err(format!("the decoded picture's plane {index} is not mapped"));
        }
        // SAFETY: while the base address is locked CoreVideo maps `bytes per row * rows` bytes from each
        // plane's base address, and the slice borrows the guard, so it cannot outlive the lock.
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

/// The picture in a decoded surface, as the frame FilmCraft's software decoder makes for the stream.
fn read_picture(surface: &CVPixelBuffer, layout: &Layout) -> std::result::Result<VideoFrame, Failure> {
    let format = CVPixelBufferGetPixelFormatType(surface);
    if format != layout.pixel_format || CVPixelBufferGetPlaneCount(surface) != 2 {
        return Err(format!("the decoded picture is in format {format:#x}, not {:#x}", layout.pixel_format));
    }
    let (width, height) = (CVPixelBufferGetWidth(surface), CVPixelBufferGetHeight(surface));
    let crop = layout.crop_in(width, height).ok_or_else(|| format!("the decoded picture is {width}x{height}, not the stream's size"))?;
    let locked = Locked::new(surface)?;
    let pixels = biplanar::to_pixels(locked.plane(0)?, locked.plane(1)?, crop, layout.depth)?;
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

/// What a VideoToolbox decoder feeds [`Sequence`]: the session, made again after a seek.
struct RawSession {
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

/// The VideoToolbox decoder of one `avcC`, `hvcC` or `av1C` stream. It is only ever used inside
/// [`crate::Fallback`], which answers which samples are random-access or disposable from the stream, as the
/// software decoder does, so this decoder answers neither.
pub(crate) struct VtDecoder {
    sequence: Sequence<RawSession, DecodedFrame>,
    name: &'static str,
}

impl VtDecoder {
    /// A decoder for the stream `entry` describes, whose parsed record is `stream`. It starts a session now,
    /// so a stream or device without a hardware decoder for it is refused here.
    pub(crate) fn new(entry: &SampleEntry, stream: &StreamInfo) -> Result<Self> {
        let layout = Layout::for_stream(stream).map_err(CodecError::Unsupported)?;
        if !hardware_available(layout.codec) {
            return Err(CodecError::Unsupported("this device has no hardware decoder for the stream's codec".into()));
        }
        let format = format_description(entry, stream, &layout).map_err(CodecError::Unsupported)?;
        let session = Session::new(&format, &layout).map_err(CodecError::Unsupported)?;
        let name = match layout.codec {
            Codec::H264 => "VideoToolbox H.264",
            Codec::Hevc => "VideoToolbox HEVC",
            Codec::Av1 => "VideoToolbox AV1",
        };
        let depth = layout.reorder;
        let raw = RawSession { layout, format, session: Some(session) };
        Ok(Self { sequence: Sequence::new(raw, stream.clone(), depth), name })
    }
}

impl VideoDecoder for VtDecoder {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        self.sequence.decode(sample, pts).map_err(CodecError::Decode)
    }

    fn flush(&mut self) -> Vec<DecodedFrame> {
        // Each sample's picture came out inside its own call, so only the reorder buffer holds any.
        self.sequence.flush()
    }

    fn reset(&mut self) {
        self.sequence.reset();
    }

    fn name(&self) -> &str {
        self.name
    }
}
