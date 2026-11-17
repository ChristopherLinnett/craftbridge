//! The platform's AAC encoder: an AudioToolbox `AudioConverter` from 16-bit interleaved samples to AAC-LC, at
//! the constant bit rate [`AacSpec`] names.
//!
//! The whole sound is converted in one call, the converter pulling its input through a callback until the
//! samples run out, after which it hands over what it still holds. Every status AudioToolbox returns is
//! checked and becomes the encoder's error, and nothing here panics, in the input callback least of all,
//! because AudioToolbox calls it from inside the conversion.
//!
//! The rules for this module's `unsafe` (AGENTS.md §4): every block carries a `// SAFETY:` comment, and no
//! pointer leaves the module.

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;

use objc2_audio_toolbox::{
    AudioConverterDispose, AudioConverterFillComplexBuffer, AudioConverterGetProperty, AudioConverterNew, AudioConverterPrimeInfo, AudioConverterRef,
    AudioConverterSetProperty, kAudioCodecBitRateControlMode_Constant, kAudioCodecPropertyBitRateControlMode, kAudioConverterEncodeBitRate,
    kAudioConverterPrimeInfo,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, AudioStreamPacketDescription, kAudioFormatFlagIsPacked, kAudioFormatFlagIsSignedInteger,
    kAudioFormatLinearPCM, kAudioFormatMPEG4AAC,
};

use super::check;
use crate::aac::AacSpec;
use crate::encode::EncodeError;
use crate::movie::{AAC_FRAME, AudioTrack};

/// The AAC-LC audio object type, which an output description names in its format flags.
const OBJECT_AAC_LC: u32 = 2;

/// The most bytes one AAC access unit of up to two channels can take.
const MAX_UNIT: usize = 16 * 1024;

/// The most frames the input callback hands over at a time.
const INPUT_FRAMES: usize = 4096;

/// A converter, disposed of when dropped.
struct Converter(AudioConverterRef);

impl Drop for Converter {
    fn drop(&mut self) {
        // SAFETY: the converter was made by `AudioConverterNew` and is disposed of once, here.
        unsafe { AudioConverterDispose(self.0) };
    }
}

/// The samples left to give the converter.
struct Input<'a> {
    samples: &'a [i16],
    channels: usize,
    /// Set when the callback hit something it cannot answer with a status.
    failed: bool,
}

/// The input callback: hands the converter up to the packets it asked for from the samples that are left, and
/// none once they have run out, which ends the stream.
unsafe extern "C-unwind" fn supply(
    _converter: AudioConverterRef,
    packets: NonNull<u32>,
    data: NonNull<AudioBufferList>,
    _descriptions: *mut *mut AudioStreamPacketDescription,
    state: *mut c_void,
) -> i32 {
    // SAFETY: `state` is the `Input` the conversion was started with, which outlives the call and is used by
    // this callback alone while the call runs.
    let Some(input) = (unsafe { state.cast::<Input<'_>>().as_mut() }) else { return -1 };
    let given = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: AudioToolbox passes a valid count of the packets it wants.
        let wanted = usize::try_from(unsafe { *packets.as_ptr() }).unwrap_or(0).min(INPUT_FRAMES);
        let frames = (input.samples.len() / input.channels.max(1)).min(wanted);
        let (now, rest) = input.samples.split_at(frames * input.channels);
        input.samples = rest;
        (now.as_ptr(), frames, std::mem::size_of_val(now))
    }));
    let Ok((pointer, frames, bytes)) = given else {
        input.failed = true;
        return -1;
    };
    // SAFETY: AudioToolbox passes valid pointers for the count and the buffer list, which holds one buffer;
    // the buffer is pointed at samples that stay alive and unchanged until the next call, and the converter
    // only reads them.
    unsafe {
        *packets.as_ptr() = u32::try_from(frames).unwrap_or(0);
        let list = data.as_ptr();
        (*list).mNumberBuffers = 1;
        (*list).mBuffers = [AudioBuffer {
            mNumberChannels: u32::try_from(input.channels).unwrap_or(1),
            mDataByteSize: u32::try_from(bytes).unwrap_or(0),
            mData: pointer.cast_mut().cast::<c_void>(),
        }];
    }
    0
}

fn pcm_description(spec: &AacSpec) -> AudioStreamBasicDescription {
    let frame_bytes = 2 * spec.channels;
    AudioStreamBasicDescription {
        mSampleRate: f64::from(spec.sample_rate),
        mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked,
        mBytesPerPacket: frame_bytes,
        mFramesPerPacket: 1,
        mBytesPerFrame: frame_bytes,
        mChannelsPerFrame: spec.channels,
        mBitsPerChannel: 16,
        mReserved: 0,
    }
}

fn aac_description(spec: &AacSpec) -> AudioStreamBasicDescription {
    AudioStreamBasicDescription {
        mSampleRate: f64::from(spec.sample_rate),
        mFormatID: kAudioFormatMPEG4AAC,
        mFormatFlags: OBJECT_AAC_LC,
        mBytesPerPacket: 0,
        mFramesPerPacket: AAC_FRAME,
        mBytesPerFrame: 0,
        mChannelsPerFrame: spec.channels,
        mBitsPerChannel: 0,
        mReserved: 0,
    }
}

/// `samples`, interleaved 16-bit, as an AAC-LC track.
pub(crate) fn encode(spec: AacSpec, samples: &[i16]) -> Result<AudioTrack, EncodeError> {
    spec.check()?;
    let channels = usize::try_from(spec.channels).unwrap_or(1);
    if !samples.len().is_multiple_of(channels) {
        return Err(EncodeError::Invalid("the sound's samples do not make whole frames".into()));
    }
    let (source, destination) = (pcm_description(&spec), aac_description(&spec));
    let mut converter: AudioConverterRef = std::ptr::null_mut();
    // SAFETY: both descriptions are valid for the call, which copies them, and the new converter is written to
    // `converter`.
    check(unsafe { AudioConverterNew(NonNull::from(&source), NonNull::from(&destination), NonNull::from(&mut converter)) }, "creating the AAC converter")
        .map_err(EncodeError::Unavailable)?;
    let converter = Converter(converter);
    // The encoder's own mode is a long-term average, which spends far less than the rate on simple sound;
    // EffectCraft's AAC encoder keeps a constant rate, so this one is asked to as well.
    let mode = kAudioCodecBitRateControlMode_Constant;
    // SAFETY: the converter is valid and the property is a 32-bit mode, passed with its size.
    let status = unsafe { AudioConverterSetProperty(converter.0, kAudioCodecPropertyBitRateControlMode, 4, NonNull::from(&mode).cast()) };
    check(status, "setting a constant AAC bit rate").map_err(EncodeError::Failed)?;
    let rate = spec.bit_rate();
    // SAFETY: the converter is valid and the property is a 32-bit bit rate, passed with its size.
    check(unsafe { AudioConverterSetProperty(converter.0, kAudioConverterEncodeBitRate, 4, NonNull::from(&rate).cast()) }, "setting the AAC bit rate")
        .map_err(EncodeError::Failed)?;
    let mut input = Input { samples, channels, failed: false };
    let mut units = Vec::new();
    loop {
        let mut unit = vec![0u8; MAX_UNIT];
        let mut packets = 1u32;
        let mut list = AudioBufferList {
            mNumberBuffers: 1,
            mBuffers: [AudioBuffer { mNumberChannels: spec.channels, mDataByteSize: MAX_UNIT as u32, mData: unit.as_mut_ptr().cast::<c_void>() }],
        };
        let mut description = AudioStreamPacketDescription { mStartOffset: 0, mVariableFramesInPacket: 0, mDataByteSize: 0 };
        // SAFETY: the converter is valid; the callback and its state (`input`, which outlives the call) are the
        // ones above; the packet count, the buffer list (one buffer over `unit`, which holds `MAX_UNIT` bytes)
        // and the description are valid for the call, which writes at most `MAX_UNIT` bytes.
        let status = unsafe {
            AudioConverterFillComplexBuffer(
                converter.0,
                Some(supply),
                std::ptr::from_mut(&mut input).cast::<c_void>(),
                NonNull::from(&mut packets),
                NonNull::from(&mut list),
                &mut description,
            )
        };
        if input.failed {
            return Err(EncodeError::Failed("the sound's input could not be read".into()));
        }
        // The input callback ends the stream by handing over no frames, which the converter answers with success
        // and no packet; a failure that produced no packet would otherwise end the sound short without a word.
        check(status, "converting the sound to AAC").map_err(EncodeError::Failed)?;
        if packets == 0 {
            break;
        }
        let size = usize::try_from(list.mBuffers.first().map_or(0, |buffer| buffer.mDataByteSize)).unwrap_or(0);
        unit.truncate(size);
        units.push(unit);
    }
    let mut prime = AudioConverterPrimeInfo { leadingFrames: 0, trailingFrames: 0 };
    let mut size = std::mem::size_of::<AudioConverterPrimeInfo>() as u32;
    // SAFETY: the converter is valid and the property is the prime info structure, passed with its size.
    let status = unsafe { AudioConverterGetProperty(converter.0, kAudioConverterPrimeInfo, NonNull::from(&mut size), NonNull::from(&mut prime).cast()) };
    check(status, "reading the AAC priming").map_err(EncodeError::Failed)?;
    spec.track(i64::from(prime.leadingFrames), units, samples.len() / channels)
}
