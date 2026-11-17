//! The hardware video encoder: VideoToolbox's `VTCompressionSession` for H.264 and HEVC.
//!
//! The session requires a hardware encoder, so a device without one fails to start and the caller encodes the
//! movie another way. Pictures go in as biplanar surfaces ([`crate::encode::Yuv420::write_biplanar`]), one at a
//! time and in order, and the session is asked for no reordering, so the packets come out in the order they went
//! in and every one is a whole picture; each carries the presentation time VideoToolbox stamped it with, which the
//! movie holds to that order. The movie's decisions are in [`crate::movie`], and what this module decides is only
//! how the framework is called. Every status VideoToolbox, CoreMedia and CoreVideo return is
//! checked and becomes the encoder's error.
//!
//! The rules for this module's `unsafe` (AGENTS.md §4): every block carries a `// SAFETY:` comment, no pointer
//! leaves the module, and nothing here panics, in the output callback least of all, because VideoToolbox calls
//! it on its own thread and a panic there would cross the framework boundary.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, PoisonError};

use objc2_core_foundation::{CFArray, CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_media::{
    CMBlockBuffer, CMFormatDescription, CMSampleBuffer, CMTime, CMTimeFlags, CMTimeRoundingMethod, kCMFormatDescriptionColorPrimaries_ITU_R_709_2,
    kCMFormatDescriptionColorPrimaries_ITU_R_2020, kCMFormatDescriptionColorPrimaries_P3_D65, kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms,
    kCMFormatDescriptionTransferFunction_ITU_R_709_2, kCMFormatDescriptionTransferFunction_ITU_R_2100_HLG, kCMFormatDescriptionTransferFunction_Linear,
    kCMFormatDescriptionTransferFunction_SMPTE_ST_2084_PQ, kCMFormatDescriptionTransferFunction_sRGB, kCMFormatDescriptionYCbCrMatrix_ITU_R_709_2,
    kCMSampleAttachmentKey_NotSync, kCMVideoCodecType_H264, kCMVideoCodecType_HEVC,
};
use objc2_core_video::{
    CVImageBuffer, CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeightOfPlane,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress, kCVPixelBufferHeightKey, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelBufferWidthKey, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
};
use objc2_video_toolbox::{
    VTCompressionSession, VTEncodeInfoFlags, VTSessionSetProperty, kVTCompressionPropertyKey_AllowFrameReordering, kVTCompressionPropertyKey_AllowOpenGOP,
    kVTCompressionPropertyKey_AverageBitRate, kVTCompressionPropertyKey_ColorPrimaries, kVTCompressionPropertyKey_DataRateLimits,
    kVTCompressionPropertyKey_ExpectedFrameRate, kVTCompressionPropertyKey_MaxKeyFrameInterval, kVTCompressionPropertyKey_ProfileLevel,
    kVTCompressionPropertyKey_Quality, kVTCompressionPropertyKey_RealTime, kVTCompressionPropertyKey_TransferFunction, kVTCompressionPropertyKey_YCbCrMatrix,
    kVTProfileLevel_H264_High_AutoLevel, kVTProfileLevel_HEVC_Main_AutoLevel, kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder,
};

use super::check;
use crate::encode::{EncodeError, VideoCodec, VideoSpec, Yuv420};
use crate::movie::{Backend, Packet};
use crate::settings::{Matrix, Primaries, Profile, Settings, Transfer};

/// What the output callback has produced and not yet handed over.
#[derive(Default)]
struct Output {
    packets: Vec<Packet>,
    /// The `avcC` or `hvcC` record, read from the first sample's format description.
    record: Option<Vec<u8>>,
    failure: Option<String>,
}

/// What the callback needs: the codec's record name, the movie's timescale its packets' times are read in,
/// and where it leaves its results.
struct CallbackState {
    record_atom: &'static str,
    timescale: i32,
    output: Mutex<Output>,
}

/// The compression session, which moves between threads with the encoder that owns it.
///
/// A compression session may be used from any thread (Apple: "thread safe"), and the encoder uses it from one
/// thread at a time (`&mut self`). objc2 does not mark the session `Send`, so this type says it for the session
/// alone.
struct SessionRef(CFRetained<VTCompressionSession>);

// SAFETY: see the type's documentation.
unsafe impl Send for SessionRef {}

/// A packet's bytes, a copy out of the sample's block buffer.
fn sample_bytes(sample: &CMSampleBuffer) -> Result<Vec<u8>, String> {
    // SAFETY: a query of a sample buffer that stays alive for the call.
    let block: CFRetained<CMBlockBuffer> = unsafe { sample.data_buffer() }.ok_or("the sample has no data")?;
    // SAFETY: a query of a block buffer that stays alive for the call.
    let length = unsafe { block.data_length() };
    let mut bytes = vec![0u8; length];
    let destination = NonNull::new(bytes.as_mut_ptr().cast::<c_void>()).ok_or("the sample has no bytes")?;
    // SAFETY: `bytes` holds `length` bytes, the whole of the block, and the call writes only inside them.
    check(unsafe { block.copy_data_bytes(0, length, destination) }, "copying the sample")?;
    Ok(bytes)
}

/// Whether decoding can start at the sample: its attachments do not say it depends on an earlier one.
fn is_key(sample: &CMSampleBuffer) -> bool {
    // SAFETY: a query of a sample buffer that stays alive for the call.
    let Some(attachments) = (unsafe { sample.sample_attachments_array(false) }) else { return true };
    // SAFETY: a sample's attachments are an array of dictionaries keyed by strings.
    let attachments = unsafe { CFRetained::cast_unchecked::<CFArray<CFDictionary<CFString, CFType>>>(attachments) };
    let Some(first) = attachments.get(0) else { return true };
    // SAFETY: reading a framework constant, which is never written.
    let key: &CFString = unsafe { kCMSampleAttachmentKey_NotSync };
    match first.get(key) {
        Some(value) => value.downcast::<CFBoolean>().map(|flag| !flag.as_bool()).unwrap_or(true),
        None => true,
    }
}

/// When the sample is shown, in ticks of `timescale`, converted by CoreMedia's own arithmetic.
fn presentation_ticks(sample: &CMSampleBuffer, timescale: i32) -> Option<i64> {
    // SAFETY: a query of a sample buffer that stays alive for the call, and a conversion of the value it
    // returned.
    let time = unsafe { sample.presentation_time_stamp().convert_scale(timescale, CMTimeRoundingMethod::Default) };
    let (flags, value) = (time.flags, time.value);
    flags.contains(CMTimeFlags::Valid).then_some(value)
}

/// The codec's configuration record, `avcC` or `hvcC`, as CoreMedia holds it in the format description's
/// sample description atoms.
fn record_of(sample: &CMSampleBuffer, atom: &str) -> Option<Vec<u8>> {
    // SAFETY: a query of a sample buffer that stays alive for the call.
    let format: CFRetained<CMFormatDescription> = unsafe { sample.format_description() }?;
    // SAFETY: reading a framework constant, which is never written.
    let key: &CFString = unsafe { kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms };
    // SAFETY: a query of a format description that stays alive for the call.
    let atoms = unsafe { format.extension(key) }?.downcast::<CFDictionary>().ok()?;
    // SAFETY: the atoms extension is a dictionary from atom names to their bytes.
    let atoms = unsafe { CFRetained::cast_unchecked::<CFDictionary<CFString, CFData>>(atoms) };
    atoms.get(&CFString::from_str(atom)).map(|record| record.to_vec())
}

/// The output callback: VideoToolbox calls it, on a thread of its own, once for each picture it was fed.
///
/// It never unwinds into VideoToolbox: a panic while reading a sample becomes a failure.
unsafe extern "C-unwind" fn output_callback(state: *mut c_void, _frame: *mut c_void, status: i32, flags: VTEncodeInfoFlags, sample: *mut CMSampleBuffer) {
    // SAFETY: `state` is the `CallbackState` the session was made with, which its `Encoder` holds a strong
    // reference to until the session is invalidated, after which no callback runs; it is only read through a
    // shared reference, and its output is behind a `Mutex`.
    let Some(state) = (unsafe { state.cast::<CallbackState>().as_ref() }) else { return };
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<Option<(Packet, Option<Vec<u8>>)>, String> {
        check(status, "encoding the picture")?;
        if flags.contains(VTEncodeInfoFlags::FrameDropped) {
            return Err("VideoToolbox dropped the picture".into());
        }
        // SAFETY: VideoToolbox passes a sample buffer that stays alive for the call, or null.
        let Some(sample) = (unsafe { sample.as_ref() }) else { return Ok(None) };
        let pts = presentation_ticks(sample, state.timescale).ok_or("VideoToolbox returned a picture with no presentation time")?;
        let packet = Packet { data: sample_bytes(sample)?, key: is_key(sample), pts };
        Ok(Some((packet, record_of(sample, state.record_atom))))
    }));
    let mut output = state.output.lock().unwrap_or_else(PoisonError::into_inner);
    match result {
        Ok(Ok(Some((packet, record)))) => {
            if output.record.is_none() {
                output.record = record;
            }
            output.packets.push(packet);
        }
        Ok(Ok(None)) => {}
        Ok(Err(failure)) => output.failure = Some(failure),
        Err(_) => output.failure = Some("reading an encoded picture panicked".into()),
    }
}

/// A time of `value` ticks of a clock of `scale` ticks a second.
fn time(value: i64, scale: i32) -> CMTime {
    CMTime { value, timescale: scale, flags: CMTimeFlags::Valid, epoch: 0 }
}

/// `kVTPropertyNotSupportedErr`: the encoder has no such property.
const PROPERTY_NOT_SUPPORTED: i32 = -12900;

/// Sets a property the encoder may lack without the movie being any different, such as a hint, a ceiling the
/// average bit rate keeps close to, or a setting whose default is the one asked for: an encoder without it is
/// left as it is.
fn set_if_supported(session: &VTCompressionSession, key: &CFString, value: &CFType, name: &str) -> Result<(), EncodeError> {
    // SAFETY: the session is valid, and the key and value are valid for the call, which copies the value.
    match unsafe { VTSessionSetProperty(session, key, Some(value)) } {
        PROPERTY_NOT_SUPPORTED => Ok(()),
        status => check(status, &format!("setting {name}")).map_err(EncodeError::Failed),
    }
}

/// Sets a property on the session.
fn set(session: &VTCompressionSession, key: &CFString, value: &CFType, name: &str) -> Result<(), EncodeError> {
    // SAFETY: the session is valid, and the key and value are valid for the call, which copies the value.
    check(unsafe { VTSessionSetProperty(session, key, Some(value)) }, &format!("setting {name}")).map_err(EncodeError::Failed)
}

fn number_i64(value: i64) -> CFRetained<CFNumber> {
    CFNumber::new_i64(value)
}

/// A pixel buffer's plane `index` with its base address locked for writing, unlocked when dropped.
struct Locked<'a>(&'a CVPixelBuffer);

impl<'a> Locked<'a> {
    fn new(surface: &'a CVPixelBuffer) -> Result<Self, EncodeError> {
        // SAFETY: `surface` is a pixel buffer that stays alive for the call, and the lock is undone by the
        // guard's drop, with the same flags.
        check(unsafe { CVPixelBufferLockBaseAddress(surface, CVPixelBufferLockFlags(0)) }, "locking the picture's surface").map_err(EncodeError::Failed)?;
        Ok(Self(surface))
    }

    /// Both planes of the biplanar surface as bytes with their strides: luma, then the interleaved chroma.
    fn planes(&mut self) -> Result<((&mut [u8], usize), (&mut [u8], usize)), EncodeError> {
        let describe = |index: usize| -> Result<(*mut u8, usize, usize), EncodeError> {
            let base = CVPixelBufferGetBaseAddressOfPlane(self.0, index).cast::<u8>();
            let stride = CVPixelBufferGetBytesPerRowOfPlane(self.0, index);
            let size = stride
                .checked_mul(CVPixelBufferGetHeightOfPlane(self.0, index))
                .ok_or_else(|| EncodeError::Failed("the surface's plane size overflows".into()))?;
            if base.is_null() || size == 0 {
                return Err(EncodeError::Failed(format!("the surface's plane {index} is not mapped")));
            }
            Ok((base, size, stride))
        };
        let ((luma, luma_size, luma_stride), (chroma, chroma_size, chroma_stride)) = (describe(0)?, describe(1)?);
        // SAFETY: while the base address is locked CoreVideo maps `bytes per row * rows` bytes from each plane's
        // base address; the two planes are separate allocations of one surface, so the slices do not overlap,
        // and both borrow the guard mutably, so neither can outlive the lock.
        unsafe { Ok(((std::slice::from_raw_parts_mut(luma, luma_size), luma_stride), (std::slice::from_raw_parts_mut(chroma, chroma_size), chroma_stride))) }
    }
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        // SAFETY: `new` locked this surface with the same flags, and it is still alive.
        unsafe { CVPixelBufferUnlockBaseAddress(self.0, CVPixelBufferLockFlags(0)) };
    }
}

/// A surface of the session's size holding `frame`.
fn surface(spec: &VideoSpec, frame: &Yuv420) -> Result<CFRetained<CVPixelBuffer>, EncodeError> {
    let (width, height) = (usize::try_from(spec.width).unwrap_or(0), usize::try_from(spec.height).unwrap_or(0));
    let mut buffer: *mut CVPixelBuffer = std::ptr::null_mut();
    // SAFETY: the sizes and format are valid, no attributes are given, and the new buffer is written to
    // `buffer`.
    let status = unsafe { CVPixelBufferCreate(None, width, height, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, None, NonNull::from(&mut buffer)) };
    check(status, "creating the picture's surface").map_err(EncodeError::Failed)?;
    let buffer = NonNull::new(buffer).ok_or_else(|| EncodeError::Failed("CoreVideo returned no surface".into()))?;
    // SAFETY: a `Create` function returns its object retained once, and `CFRetained` takes that over.
    let buffer = unsafe { CFRetained::from_raw(buffer) };
    {
        let mut locked = Locked::new(&buffer)?;
        let (luma, chroma) = locked.planes()?;
        frame.write_biplanar(spec.width, spec.height, luma, chroma)?;
    }
    Ok(buffer)
}

/// The VideoToolbox encoder of one movie.
pub(crate) struct Encoder {
    session: SessionRef,
    spec: VideoSpec,
    state: Arc<CallbackState>,
}

impl Encoder {
    /// Starts a session for `spec` that requires a hardware encoder. A device without one, or without one for
    /// the codec, answers [`EncodeError::Unavailable`] and nothing is left running.
    pub(crate) fn start(spec: VideoSpec) -> Result<Self, EncodeError> {
        spec.check()?;
        let settings = spec.settings();
        let (codec, record_atom) = match spec.codec {
            VideoCodec::H264 => (kCMVideoCodecType_H264, "avcC"),
            VideoCodec::Hevc => (kCMVideoCodecType_HEVC, "hvcC"),
        };
        // SAFETY: reading a framework constant, which is never written.
        let require: &CFString = unsafe { kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder };
        let specification = CFDictionary::<CFString, CFType>::from_slices(&[require], &[CFBoolean::new(true).as_ref()]);
        // SAFETY: reading framework constants, which are never written.
        let (format_key, width_key, height_key): (&CFString, &CFString, &CFString) =
            unsafe { (kCVPixelBufferPixelFormatTypeKey, kCVPixelBufferWidthKey, kCVPixelBufferHeightKey) };
        let format = CFNumber::new_i32(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange as i32);
        let (width, height) = (CFNumber::new_i32(spec.width as i32), CFNumber::new_i32(spec.height as i32));
        let source = CFDictionary::<CFString, CFType>::from_slices(&[format_key, width_key, height_key], &[format.as_ref(), width.as_ref(), height.as_ref()]);
        let timescale = i32::try_from(spec.rate_num).map_err(|_| EncodeError::Invalid("the frame rate's numerator is too large".into()))?;
        let state = Arc::new(CallbackState { record_atom, timescale, output: Mutex::new(Output::default()) });
        let mut session: *mut VTCompressionSession = std::ptr::null_mut();
        // SAFETY: every argument is valid for the call, which copies the dictionaries; the callback's state is
        // the `Arc` kept in the `Encoder`, which keeps it alive until the session is invalidated.
        let status = unsafe {
            VTCompressionSession::create(
                None,
                spec.width as i32,
                spec.height as i32,
                codec,
                Some(specification.as_ref()),
                Some(source.as_ref()),
                None,
                Some(output_callback),
                Arc::as_ptr(&state).cast_mut().cast::<c_void>(),
                NonNull::from(&mut session),
            )
        };
        if status != 0 {
            return Err(EncodeError::Unavailable(format!("this device has no hardware encoder for the movie ({status})")));
        }
        let session = NonNull::new(session).ok_or_else(|| EncodeError::Unavailable("VideoToolbox returned no session".into()))?;
        // SAFETY: a `Create` function returns its object retained once, and `CFRetained` takes that over.
        let session = SessionRef(unsafe { CFRetained::from_raw(session) });
        let encoder = Self { session, spec, state };
        // An encoder that cannot be set up as asked is no encoder for this movie: the caller encodes it another way.
        encoder.configure(&settings).map_err(|error| EncodeError::Unavailable(error.to_string()))?;
        // SAFETY: the session is valid.
        check(unsafe { encoder.session.0.prepare_to_encode_frames() }, "preparing the encoder").map_err(EncodeError::Unavailable)?;
        Ok(encoder)
    }

    fn configure(&self, settings: &Settings) -> Result<(), EncodeError> {
        let session: &VTCompressionSession = &self.session.0;
        // SAFETY: reading framework constants, which are never written.
        let (profile, reordering, open_gop, real_time, frame_rate, key_interval) = unsafe {
            (
                match settings.profile {
                    Profile::H264High => kVTProfileLevel_H264_High_AutoLevel,
                    Profile::HevcMain => kVTProfileLevel_HEVC_Main_AutoLevel,
                },
                kVTCompressionPropertyKey_AllowFrameReordering,
                kVTCompressionPropertyKey_AllowOpenGOP,
                kVTCompressionPropertyKey_RealTime,
                kVTCompressionPropertyKey_ExpectedFrameRate,
                kVTCompressionPropertyKey_MaxKeyFrameInterval,
            )
        };
        // SAFETY: reading a framework constant, which is never written.
        set(session, unsafe { kVTCompressionPropertyKey_ProfileLevel }, profile.as_ref(), "the profile")?;
        // No reordering and no open GOP, so packets leave in the order pictures went in and each stands alone
        // (an encoder with no open GOP setting has none).
        set(session, reordering, CFBoolean::new(false).as_ref(), "frame reordering")?;
        set_if_supported(session, open_gop, CFBoolean::new(false).as_ref(), "open GOP")?;
        set_if_supported(session, real_time, CFBoolean::new(false).as_ref(), "real time")?;
        set(session, frame_rate, CFNumber::new_f64(settings.frame_rate).as_ref(), "the frame rate")?;
        set(session, key_interval, CFNumber::new_i32(settings.max_key_frame_interval).as_ref(), "the key frame interval")?;
        if let Some(rate) = settings.average_bit_rate {
            // SAFETY: reading a framework constant, which is never written.
            set(session, unsafe { kVTCompressionPropertyKey_AverageBitRate }, number_i64(rate).as_ref(), "the bit rate")?;
        }
        if let Some(peak) = settings.peak_bytes_per_second {
            // SAFETY: reading a framework constant, which is never written.
            let key: &CFString = unsafe { kVTCompressionPropertyKey_DataRateLimits };
            let limits = CFArray::<CFNumber>::from_retained_objects(&[number_i64(peak), CFNumber::new_i64(1)]);
            set_if_supported(session, key, limits.as_ref(), "the data rate limit")?;
        }
        if let Some(quality) = settings.quality {
            // SAFETY: reading a framework constant, which is never written.
            set(session, unsafe { kVTCompressionPropertyKey_Quality }, CFNumber::new_f32(quality).as_ref(), "the quality")?;
        }
        // SAFETY: reading framework constants, which are never written.
        unsafe {
            if let Some(primaries) = settings.tags.primaries {
                let value = match primaries {
                    Primaries::Bt709 => kCMFormatDescriptionColorPrimaries_ITU_R_709_2,
                    Primaries::Bt2020 => kCMFormatDescriptionColorPrimaries_ITU_R_2020,
                    Primaries::P3D65 => kCMFormatDescriptionColorPrimaries_P3_D65,
                };
                set(session, kVTCompressionPropertyKey_ColorPrimaries, value.as_ref(), "the colour primaries")?;
            }
            if let Some(transfer) = settings.tags.transfer {
                let value = match transfer {
                    Transfer::Bt709 => kCMFormatDescriptionTransferFunction_ITU_R_709_2,
                    Transfer::Srgb => kCMFormatDescriptionTransferFunction_sRGB,
                    Transfer::Pq => kCMFormatDescriptionTransferFunction_SMPTE_ST_2084_PQ,
                    Transfer::Hlg => kCMFormatDescriptionTransferFunction_ITU_R_2100_HLG,
                    Transfer::Linear => kCMFormatDescriptionTransferFunction_Linear,
                };
                set(session, kVTCompressionPropertyKey_TransferFunction, value.as_ref(), "the transfer function")?;
            }
            if let Some(Matrix::Bt709) = settings.tags.matrix {
                set(session, kVTCompressionPropertyKey_YCbCrMatrix, kCMFormatDescriptionYCbCrMatrix_ITU_R_709_2.as_ref(), "the matrix")?;
            }
        }
        Ok(())
    }

    /// What the callback produced since the last call, or the failure it hit.
    fn take(&self) -> Result<Vec<Packet>, EncodeError> {
        let mut output = self.state.output.lock().unwrap_or_else(PoisonError::into_inner);
        match output.failure.take() {
            Some(failure) => Err(EncodeError::Failed(failure)),
            None => Ok(std::mem::take(&mut output.packets)),
        }
    }
}

impl Backend for Encoder {
    fn encode(&mut self, index: u64, frame: &Yuv420) -> Result<Vec<Packet>, EncodeError> {
        let image = surface(&self.spec, frame)?;
        let scale = self.state.timescale;
        let (pts, duration) = (time(self.spec.presentation_time(index), scale), time(i64::from(self.spec.rate_den), scale));
        let mut info = VTEncodeInfoFlags(0);
        // SAFETY: the session is valid; the surface is a pixel buffer, which VideoToolbox retains while it
        // encodes it, so it need not outlive the call; no per-frame properties or reference are given.
        let status = unsafe {
            let image: &CVImageBuffer = &image;
            self.session.0.encode_frame(image, pts, duration, None, std::ptr::null_mut(), &mut info)
        };
        check(status, "encoding the picture").map_err(EncodeError::Failed)?;
        self.take()
    }

    fn finish(&mut self) -> Result<Vec<Packet>, EncodeError> {
        // SAFETY: the session is valid; an invalid time asks for every picture still pending.
        let status = unsafe { self.session.0.complete_frames(CMTime { value: 0, timescale: 0, flags: CMTimeFlags(0), epoch: 0 }) };
        check(status, "finishing the movie").map_err(EncodeError::Failed)?;
        self.take()
    }

    fn record(&self) -> Option<Vec<u8>> {
        self.state.output.lock().unwrap_or_else(PoisonError::into_inner).record.clone()
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: the session is valid until it is released. Completing lets a callback that is running finish,
        // and invalidating ends them, so the state they read can be freed with the `Arc` afterwards.
        unsafe {
            self.session.0.complete_frames(CMTime { value: 0, timescale: 0, flags: CMTimeFlags(0), epoch: 0 });
            self.session.0.invalidate();
        }
    }
}
