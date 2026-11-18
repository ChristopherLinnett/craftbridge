//! The Android AAC encoder for a movie's sound: the device's own MediaCodec AAC encoder, which is what the
//! platform's audio is encoded with — audio codecs answer the platform's hardware question with software
//! almost everywhere, and EffectCraft spends its sound through a software AAC encoder too, so this one takes
//! the platform's default for the type, as the Apple build takes AudioToolbox's, and keeps EffectCraft's
//! spending: AAC-LC at 160 kbit/s a channel ([`crate::aac`]), with the encoder's own priming cut by the
//! movie's edit.

use std::cell::{Cell, RefCell};

use ndk::media::media_codec::{DequeuedOutputBufferInfoResult, MediaCodec, MediaCodecDirection};
use ndk::media::media_format::MediaFormat;

use crate::aac::{self, AacSpec};
use crate::encode::EncodeError;
use crate::mediacodec_input;
use crate::movie::{AAC_FRAME, AudioTrack};
use crate::platform::mediacodec::{self, BIT_RATE, CODEC_CONFIG, END_OF_STREAM, Failure, MIME, POLL, STALL, step};

/// The AAC encoder the platform has, by its registered type name.
const AAC_MIME: &str = "audio/mp4a-latm";
/// `KEY_SAMPLE_RATE` and `KEY_CHANNEL_COUNT`: the sound as asked.
const SAMPLE_RATE: &str = "sample-rate";
const CHANNEL_COUNT: &str = "channel-count";
/// `KEY_AAC_PROFILE`, in the number the platform gives `AacProfileLC`: the profile the export standardizes on.
const AAC_PROFILE: &str = "aac-profile";
const AAC_LC: i32 = 2;

/// Encode `samples`, 16-bit interleaved, as AAC-LC for `spec`, or say why there is no AAC encoder.
pub(crate) fn encode(spec: AacSpec, samples: &[i16]) -> Result<AudioTrack, EncodeError> {
    spec.check()?;
    if !samples.len().is_multiple_of(usize::try_from(spec.channels).unwrap_or(1)) {
        return Err(EncodeError::Invalid("the sound ends between the channels of one frame".into()));
    }
    let converter = Converter::start(spec)?;
    converter.feed(samples)
}

/// One feeding of sound through the codec: the samples in chunks of one access unit, the end of the sound,
/// and the units that come out, one per chunk, as the movie writes them.
struct Converter {
    codec: MediaCodec,
    spec: AacSpec,
    /// The samples the codec primes before the sound ([`aac::android_priming`]).
    priming: u32,
    /// The encoded units the codec has given, in order.
    units: RefCell<Vec<Vec<u8>>>,
    /// Whether the codec has said the sound is over.
    ended: Cell<bool>,
}

// SAFETY: the `ndk` bindings hold raw pointers, which are not `Send`. The codec is not tied to the thread
// that made it: Android sets thread rules only for codec callbacks, which this encoder never registers, and
// hands every call to the codec's own looper thread. Every call comes from one thread at a time.
unsafe impl Send for Converter {}

impl Converter {
    fn start(spec: AacSpec) -> Result<Self, EncodeError> {
        let codec = MediaCodec::from_encoder_type(AAC_MIME)
            .ok_or_else(|| EncodeError::Unavailable(format!("this device has no AAC encoder: nothing answers {AAC_MIME}")))?;
        let name = step(codec.name(), "naming the AAC encoder").map_err(EncodeError::Unavailable)?;
        let priming = aac::android_priming(&name)
            .ok_or_else(|| EncodeError::Unavailable(format!("the AAC encoder {name} primes an amount this export cannot line its sound up by")))?;
        let mut format = MediaFormat::new();
        format.set_str(MIME, AAC_MIME);
        let sample_rate = i32::try_from(spec.sample_rate).map_err(|_| EncodeError::Invalid("the sound's rate is past what a codec can be told".into()))?;
        let channels = i32::try_from(spec.channels).map_err(|_| EncodeError::Invalid("the sound has too many channels for a codec".into()))?;
        format.set_i32(SAMPLE_RATE, sample_rate);
        format.set_i32(CHANNEL_COUNT, channels);
        // What a second of the sound costs is EffectCraft's spend rather than the codec's own.
        format.set_i32(BIT_RATE, i32::try_from(spec.bit_rate()).unwrap_or(i32::MAX));
        format.set_i32(AAC_PROFILE, AAC_LC);
        // An encoder that cannot be set up as asked is no encoder for this movie: the caller spends the sound
        // another way, which is what every decline here means.
        step(codec.configure(&format, None, MediaCodecDirection::Encoder), "configuring the AAC encoder")
            .and_then(|()| step(codec.start(), "starting the AAC encoder"))
            .map_err(|why| EncodeError::Unavailable(format!("an encoder that cannot be set up as asked is no encoder for this movie: {why}")))?;
        Ok(Self { codec, spec, priming, units: RefCell::new(Vec::new()), ended: Cell::new(false) })
    }

    /// The whole sound, then its end, then the units: the movie's track of them, with the priming.
    fn feed(&self, samples: &[i16]) -> Result<AudioTrack, EncodeError> {
        let channels = usize::try_from(self.spec.channels).unwrap_or(1);
        let chunk = AAC_FRAME as usize * channels;
        let frames = samples.len() / channels;
        let rate = u64::from(self.spec.sample_rate).max(1);
        // The last chunk is padded out to a whole frame — the codec answers in whole frames only — and as many
        // frames of silence as the codec primes are added after the whole sound: the codec answers a frame it
        // was given and nothing more, so without them the priming edit would cut off the sound's end exactly
        // as many samples as it skips at its start.
        let count = samples.chunks(chunk).count() + self.priming.div_ceil(AAC_FRAME) as usize;
        let silence: &[i16] = &[];
        for (index, part) in samples.chunks(chunk).chain(std::iter::repeat(silence)).take(count).enumerate() {
            let mut bytes = Vec::with_capacity(chunk * 2);
            for sample in part {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
            bytes.resize(chunk * 2, 0);
            let time = u64::try_from(index).unwrap_or(0).saturating_mul(u64::from(AAC_FRAME)).saturating_mul(1_000_000) / rate;
            mediacodec::queue_one(&self.codec, time, 0, |input| mediacodec::fill(input, &bytes).map(|()| bytes.len()), || self.drain())
                .map_err(EncodeError::Failed)?;
        }
        let end_time = u64::try_from(count).unwrap_or(0).saturating_mul(u64::from(AAC_FRAME)).saturating_mul(1_000_000) / rate;
        mediacodec_input::until_the_end::<()>(
            STALL,
            "encoder",
            || {
                mediacodec::queue_one(&self.codec, end_time, END_OF_STREAM, |_| Ok(0), || self.drain())?;
                self.drain()?;
                Ok(vec![])
            },
            || {
                self.drain()?;
                Ok((vec![], self.ended.get()))
            },
        )
        .map_err(EncodeError::Failed)?;
        let units = self.units.replace(Vec::new());
        self.spec.track(i64::from(self.priming), units, frames)
    }

    /// What the codec has out right now: its configuration buffer is skipped (the movie builds the sound's
    /// description from [`AacSpec::config`], which is the same description); every other buffer is one unit.
    fn drain(&self) -> Result<(), Failure> {
        loop {
            match step(self.codec.dequeue_output_buffer(POLL), "waiting for the AAC encoder")? {
                DequeuedOutputBufferInfoResult::TryAgainLater => return Ok(()),
                DequeuedOutputBufferInfoResult::OutputFormatChanged | DequeuedOutputBufferInfoResult::OutputBuffersChanged => {}
                DequeuedOutputBufferInfoResult::Buffer(output) => {
                    let flags = output.info().flags();
                    let bytes = mediacodec::output_bytes(&output);
                    step(self.codec.release_output_buffer(output, false), "returning the AAC encoder's buffer")?;
                    let bytes = bytes?;
                    if !bytes.is_empty() && flags & CODEC_CONFIG == 0 {
                        self.units.borrow_mut().push(bytes);
                    }
                    if flags & END_OF_STREAM != 0 {
                        self.ended.set(true);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encodetests;
    use crate::hardware::encode_aac;
    use crate::platform::mediacodec::tests::device_codecs;

    #[test]
    fn the_device_encodes_sound_as_aac_lc_with_its_priming() {
        // Read before any codec is made, as every device test does: reading it starts the binder thread pool
        // the codec service calls back on, without which the encoder's calls never return.
        device_codecs();
        let sound = encode_aac(AacSpec { sample_rate: 48_000, channels: 2 }, &encodetests::tone(1)).unwrap();
        // A MediaCodec asked in bits a second lands nearer its target than a constant-rate converter, so the
        // run gives it ten percent of slack.
        encodetests::check_aac(&sound, 0.1);
        // The platform's own encoder, every device's default for the type; the movie check lines its sound up.
        assert_eq!(sound.priming, 2048, "the platform's encoder primes two frames");
    }

    #[test]
    fn sound_that_ends_between_the_channels_of_one_frame_is_refused_by_its_words() {
        // Half a sample is no frame, and no encoder can be handed it: the refusal names why, in its own words.
        let why = match encode_aac(AacSpec { sample_rate: 48_000, channels: 2 }, &[0, 0, 0]) {
            Err(EncodeError::Invalid(why)) => why,
            Ok(_) => panic!("half a frame must be refused, not encoded"),
            Err(_) => panic!("half a frame must be refused as invalid, not as another error"),
        };
        assert_eq!(why, "the sound ends between the channels of one frame");
    }
}
