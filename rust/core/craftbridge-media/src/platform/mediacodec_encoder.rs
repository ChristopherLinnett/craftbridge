//! The Android video encoders: the device's own MediaCodec encoder for a movie's codec, asked the way every
//! Android encoder is asked (a [`MediaFormat`] of the platform's own key names) and answered the way the movie
//! writer takes an encoder (a [`crate::movie::Backend`] of length-prefixed packets).
//!
//! An encoder starts only over the device's own answer that it has a *hardware* encoder for the codec
//! ([`mediacodec_input::hardware_encoders`], which reads the app's `isHardwareAccelerated` list), exactly as
//! the decoders choose theirs: a software MediaCodec encoder is what EffectCraft's own encoders already are,
//! so a device with no hardware encoder declines and the export runs through EffectCraft (plan §18). The
//! starting seam that names a codec itself exists for the device proof to walk the whole path once per codec
//! on a device whose only encoders are software.

use std::cell::{Cell, RefCell};

use ndk::media::media_codec::{DequeuedOutputBufferInfoResult, MediaCodec, MediaCodecDirection};
use ndk::media::media_format::MediaFormat;

use crate::encode::{EncodeError, VideoCodec, VideoSpec, Yuv420};
use crate::mediacodec_input::{self, H264_MIME, HEVC_MIME};
use crate::mediacodec_output::{InputLayout, SEMI_PLANAR};
use crate::movie::Packet;
use crate::settings::{self, AndroidColour, Profile, Settings};

use super::mediacodec::{self, CODEC_CONFIG, END_OF_STREAM, Failure, MakeCodec, POLL, STALL, step};

/// `KEY_FRAME_RATE`: what one second of the movie holds, which the rate's own quotient says, as the float
/// the key takes; a video encoder asked with any other kind of number (a double) is told no frame rate and
/// refuses to configure.
const FRAME_RATE: &str = "frame-rate";
/// `KEY_BITRATE_MODE`: aim at the bit rate rather than hold it, which is the shape of the export's request.
const BITRATE_MODE: &str = "bitrate-mode";
/// The `KEY_BITRATE_MODE` value that is variable: `MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_VBR`.
const BITRATE_MODE_VBR: i32 = 1;
/// `KEY_I_FRAME_INTERVAL`: how long between pictures decoding can start at, in seconds.
const I_FRAME_INTERVAL: &str = "i-frame-interval";
/// `KEY_PROFILE`: the profile the exporter always asks for ([`Profile`]) in the number each codec names it by
/// (`MediaCodecInfo.CodecProfileLevel.AVCProfileHigh`, `HEVCProfileMain`).
const PROFILE: &str = "profile";
const PROFILE_AVC_HIGH: i32 = 8;
const PROFILE_HEVC_MAIN: i32 = 1;
/// `KEY_LEVEL`, which a video encoder asked for a profile must be asked with ([`VideoSpec::level`]): an
/// encoder of Android's older codec framework refuses a profile that comes without one.
const LEVEL: &str = "level";
/// `KEY_MAX_B_FRAMES`: none, which is what keeps the packets coming out in the order the movie was fed them.
const MAX_B_FRAMES: &str = "max-bframes";
/// `KEY_STRIDE` and `KEY_SLICE_HEIGHT` of the codec's input format, which describe the layout its raw input
/// buffers take: the platform's only description of one, since the NDK has no `getInputImage`.
const STRIDE: &str = "stride";
const SLICE_HEIGHT: &str = "slice-height";
/// `KEY_COLOR_STANDARD`, `KEY_COLOR_TRANSFER` and `KEY_COLOR_RANGE`: the colour of the stream
/// ([`crate::settings::Tags::android`]).
const COLOR_STANDARD: &str = "color-standard";
const COLOR_TRANSFER: &str = "color-transfer";
const COLOR_RANGE: &str = "color-range";

/// The device's hardware video encoder for one movie.
///
/// The `ndk` bindings hold raw pointers, which are not `Send`. Neither the codec nor its buffers is tied to
/// the thread that made it: Android sets thread rules only for codec callbacks, which this encoder never
/// registers, and the platform hands every call to the codec's own looper thread. The encoder makes every
/// call from one thread at a time, which the movie's own calls already are.
pub(crate) struct Encoder {
    codec: MediaCodec,
    spec: VideoSpec,
    /// Where each picture goes in an input buffer, as the codec's input format says.
    layout: InputLayout,
    /// How many pictures the codec holds: where the end of the stream belongs in time.
    fed: Cell<u64>,
    /// Whether the codec has said the stream is over, after which it takes nothing.
    ended: Cell<bool>,
    /// The parameter-set bytes the codec named itself, in the buffers it flagged as its configuration.
    csd: RefCell<Vec<Vec<u8>>>,
    /// The record built from them once, and why it could not be built while that is true.
    record: RefCell<Option<Vec<u8>>>,
    no_record: RefCell<Option<String>>,
}

// SAFETY: see the type's documentation: synchronous mode, every call from one thread at a time.
unsafe impl Send for Encoder {}

impl Encoder {
    /// Starts the device's hardware encoder for `spec`, or says why there is none: nothing to ask
    /// ([`EncodeError::Unavailable`]) or a request no encoder can be asked with ([`EncodeError::Invalid`]).
    pub(crate) fn start(spec: VideoSpec) -> Result<Self, EncodeError> {
        Self::start_with(spec, mediacodec::hardware_encoder)
    }

    /// [`Encoder::start`] with a codec chooser the caller names: production names
    /// [`mediacodec::hardware_encoder`], the device's own hardware answer; a device proof names a codec the
    /// device has (plan §18). A constant-quality request is declined before anything is asked, because
    /// MediaCodec encoders are asked for a bit rate and this exporter has no quality number for them.
    pub(crate) fn start_with(spec: VideoSpec, name_codec: MakeCodec) -> Result<Self, EncodeError> {
        spec.check()?;
        let settings = spec.settings();
        if settings.quality.is_some() {
            return Err(EncodeError::Unavailable("this platform asks its video encoders for a bit rate, never a quality".into()));
        }
        let mime = mime(spec.codec);
        let codec = name_codec(mime).map_err(EncodeError::Unavailable)?;
        let format = asking(&spec, &settings)?;
        // An encoder that cannot be set up as asked is no encoder for this movie: the caller encodes it another
        // way, which is what every decline here means.
        let unavailable = |why: Failure| EncodeError::Unavailable(format!("an encoder that cannot be set up as asked is no encoder for this movie: {why}"));
        step(codec.configure(&format, None, MediaCodecDirection::Encoder), "configuring the encoder").map_err(unavailable)?;
        let layout = input_layout(&codec, &spec).map_err(unavailable)?;
        step(codec.start(), "starting the encoder").map_err(unavailable)?;
        Ok(Self {
            codec,
            spec,
            layout,
            fed: Cell::new(0),
            ended: Cell::new(false),
            csd: RefCell::new(Vec::new()),
            record: RefCell::new(None),
            no_record: RefCell::new(None),
        })
    }

    /// Feeds one picture, and gives back every packet ready after it: the codec's own parameter sets first,
    /// which it names in buffers it flags as its configuration, then the pictures in the order they were fed.
    pub(crate) fn encode(&self, index: u64, frame: &Yuv420) -> Result<Vec<Packet>, EncodeError> {
        if self.ended.get() {
            return Err(EncodeError::Failed("the encoder ended the stream before the movie was over".into()));
        }
        let time = self.clock(index).map_err(EncodeError::Failed)?;
        let picture = self.layout.packed(frame, self.spec.width, self.spec.height)?;
        let packets = self.queue(time, 0, &picture).map_err(EncodeError::Failed)?;
        self.fed.set(index + 1);
        Ok(packets)
    }

    /// Hands the codec the end of the stream and takes everything still inside it out.
    pub(crate) fn finish(&self) -> Result<Vec<Packet>, EncodeError> {
        let time = self.clock(self.fed.get()).map_err(EncodeError::Failed)?;
        mediacodec_input::until_the_end(
            STALL,
            "encoder",
            || self.queue(time, END_OF_STREAM, &[]),
            || {
                let packets = self.drain()?;
                Ok((packets, self.ended.get()))
            },
        )
        .map_err(EncodeError::Failed)
    }

    /// The record the movie's sample entry is built from: the encoder's own parameter sets, read as the
    /// description the codec's stream standard gives them. It is there once the codec has named its sets,
    /// which is with or before its first picture.
    pub(crate) fn record(&self) -> Option<Vec<u8>> {
        self.record.borrow().clone()
    }

    /// Queues `data` at `time` with `flags`, and gives back every packet out by then: those the codec gave
    /// while it had no input buffer free, which waiting on it reads, then those ready after.
    fn queue(&self, time: u64, flags: u32, data: &[u8]) -> Result<Vec<Packet>, Failure> {
        let mut packets = Vec::new();
        mediacodec::queue_one(
            &self.codec,
            time,
            flags,
            |input| mediacodec::fill(input, data).map(|()| data.len()),
            || {
                packets.extend(self.drain()?);
                Ok(())
            },
        )?;
        packets.extend(self.drain()?);
        Ok(packets)
    }

    /// The codec's clock for the movie time of the picture at `index`: microseconds, which round-trip with
    /// the movie's ticks at any rate an export runs at ([`settings::microseconds_at`]).
    fn clock(&self, index: u64) -> Result<u64, Failure> {
        u64::try_from(settings::microseconds_at(self.spec.presentation_time(index), self.spec.rate_num))
            .map_err(|_| "the movie is longer than the codec's clock".into())
    }

    /// Every packet ready at the codec right now, read within [`POLL`] of the first.
    fn drain(&self) -> Result<Vec<Packet>, Failure> {
        let mut packets = Vec::new();
        loop {
            match step(self.codec.dequeue_output_buffer(POLL), "waiting for the encoder")? {
                DequeuedOutputBufferInfoResult::TryAgainLater => return Ok(packets),
                DequeuedOutputBufferInfoResult::OutputFormatChanged | DequeuedOutputBufferInfoResult::OutputBuffersChanged => {}
                DequeuedOutputBufferInfoResult::Buffer(output) => {
                    let (flags, time) = (output.info().flags(), output.info().presentation_time_us());
                    let bytes = mediacodec::output_bytes(&output);
                    step(self.codec.release_output_buffer(output, false), "returning the encoder's buffer")?;
                    let bytes = bytes?;
                    if !bytes.is_empty() {
                        let units = crate::mediacodec_output::nal_units(&bytes)?;
                        if flags & CODEC_CONFIG != 0 {
                            self.name_parameters(&units);
                        } else {
                            packets.push(Packet {
                                data: crate::mediacodec_output::length_prefixed(&units),
                                key: crate::mediacodec_output::is_key(self.spec.codec, &units),
                                pts: settings::ticks_at(time, self.spec.rate_num),
                            });
                            self.require_record()?;
                        }
                    }
                    if flags & END_OF_STREAM != 0 {
                        self.ended.set(true);
                    }
                }
            }
        }
    }

    /// The codec's own name for its parameter sets: kept as it gave them, and read once into the record. A
    /// codec that splits its sets over buffers is read when the set is whole, which the read itself shows.
    fn name_parameters(&self, units: &[&[u8]]) {
        self.csd.borrow_mut().extend(units.iter().map(|unit| unit.to_vec()));
        let held = self.csd.borrow().clone();
        let refs: Vec<&[u8]> = held.iter().map(Vec::as_slice).collect();
        match crate::mediacodec_output::record(self.spec.codec, &refs) {
            Ok(record) => {
                *self.record.borrow_mut() = Some(record);
                *self.no_record.borrow_mut() = None;
            }
            Err(why) => *self.no_record.borrow_mut() = Some(why),
        }
    }

    /// A picture with no record yet is a stream the movie cannot describe: the codec broke its own rule of
    /// naming its parameter sets first, or named them unreadably.
    fn require_record(&self) -> Result<(), Failure> {
        if self.record.borrow().is_some() {
            return Ok(());
        }
        Err(match self.no_record.borrow().as_deref() {
            Some(why) => format!("the encoder's own parameter sets are not a record: {why}"),
            None => "the encoder sent a picture before naming its parameter sets".into(),
        })
    }
}

/// The codec a movie of this codec runs on.
fn mime(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => H264_MIME,
        VideoCodec::Hevc => HEVC_MIME,
    }
}

/// Where a picture goes in the configured `codec`'s input buffers: the colour format, stride and slice height
/// its input format gives, the last two of which a hardware encoder may align past the picture's own sides.
fn input_layout(codec: &MediaCodec, spec: &VideoSpec) -> Result<InputLayout, Failure> {
    let format = mediacodec::caught(|| codec.input_format(), "the encoder gave no input format")?;
    InputLayout::of(spec.width, spec.height, format.i32(mediacodec::COLOR_FORMAT), format.i32(STRIDE), format.i32(SLICE_HEIGHT))
}

/// What the movie asks the encoder with: its size, rate, bit rate, key-frame spacing, profile and colour,
/// each in the key the platform gives that request, and the semi-planar layout the exporter writes.
fn asking(spec: &VideoSpec, settings: &Settings) -> Result<MediaFormat, EncodeError> {
    let too_big = |size: u32| EncodeError::Invalid(format!("the movie is {size} wide, which is past what a codec can be asked for"));
    let mut format = MediaFormat::new();
    format.set_str(mediacodec::MIME, mime(spec.codec));
    let (width, height) = (i32::try_from(spec.width).map_err(|_| too_big(spec.width))?, i32::try_from(spec.height).map_err(|_| too_big(spec.height))?);
    format.set_i32(mediacodec::WIDTH, width);
    format.set_i32(mediacodec::HEIGHT, height);
    format.set_f32(FRAME_RATE, settings.frame_rate as f32);
    if let Some(bits) = settings.average_bit_rate {
        format.set_i32(mediacodec::BIT_RATE, i32::try_from(bits).unwrap_or(i32::MAX));
    }
    format.set_i32(BITRATE_MODE, BITRATE_MODE_VBR);
    format.set_f32(I_FRAME_INTERVAL, settings::key_frame_seconds(u32::try_from(settings.max_key_frame_interval).unwrap_or(1), spec.rate_num, spec.rate_den));
    format.set_i32(mediacodec::COLOR_FORMAT, SEMI_PLANAR);
    format.set_i32(
        PROFILE,
        match settings.profile {
            Profile::H264High => PROFILE_AVC_HIGH,
            Profile::HevcMain => PROFILE_HEVC_MAIN,
        },
    );
    format.set_i32(LEVEL, spec.level().android);
    format.set_i32(MAX_B_FRAMES, 0);
    let AndroidColour { standard, transfer, range } = settings.tags.android();
    for (key, value) in [(COLOR_STANDARD, standard), (COLOR_TRANSFER, transfer)] {
        if let Some(value) = value {
            format.set_i32(key, value);
        }
    }
    format.set_i32(COLOR_RANGE, range);
    Ok(format)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::encodetests::{self, Start};
    use crate::hardware::HardwareEncoder;
    use crate::platform::mediacodec::tests::{device_codecs, give_device_codecs};

    /// The production path: the device's list given as the app gives it, then [`HardwareEncoder::start`].
    /// Where the list names a hardware encoder for the codec, the start must succeed: a refusal there is the
    /// platform's answer to how this encoder asks, and a check that skipped it would pass while no hardware
    /// encoded anything. Where it names none, the decline is the answer, which the checks take as nothing to
    /// encode ([`the_export_declines_a_movie_the_device_has_no_hardware_encoder_for`] pins its words).
    pub(crate) fn through_the_hardware(spec: VideoSpec) -> Result<HardwareEncoder, EncodeError> {
        give_device_codecs();
        let mime = mime(spec.codec);
        match (HardwareEncoder::start(spec), mediacodec_input::hardware_encoders(device_codecs(), mime).first()) {
            (Err(why), Some(name)) => panic!("{mime}: the device's hardware encoder {name} did not start: {why}"),
            (started, _) => started,
        }
    }

    /// The device's first *software* encoder for `mime`: every device lists one for H.264 and HEVC, the
    /// emulator's only encoders, so the whole path runs on a device with no hardware encoder too.
    fn software_codec(mime: &str) -> Result<MediaCodec, Failure> {
        device_codecs()
            .iter()
            .find(|codec| codec.encoder && !codec.hardware_accelerated && codec.types.iter().any(|kind| kind == mime))
            .and_then(|codec| MediaCodec::from_codec_name(&codec.name))
            .ok_or(format!("this device has no software encoder for {mime}"))
    }

    /// The whole production path over the device's software encoder: the same [`HardwareEncoder`] a caller
    /// gets, started over a named codec instead of the hardware answer.
    pub(crate) fn through_a_software_codec(spec: VideoSpec) -> Result<HardwareEncoder, EncodeError> {
        // A codec the device itself named must start: a refusal here is a broken device or a broken path,
        // and a shared check that skips rather than fails would hide exactly that.
        Ok(HardwareEncoder::started_over(Encoder::start_with(spec, software_codec).unwrap()))
    }

    /// Both ways a device encodes, each check run over each: its hardware encoder where it lists one, and its
    /// software one.
    const STARTS: [Start; 2] = [through_the_hardware, through_a_software_codec];

    #[test]
    fn an_h264_encode_is_a_movie_within_d12() {
        for start in STARTS {
            encodetests::check_movie(encodetests::spec(VideoCodec::H264), start);
        }
    }

    #[test]
    fn an_hevc_encode_is_a_movie_within_d12() {
        for start in STARTS {
            encodetests::check_movie(encodetests::spec(VideoCodec::Hevc), start);
        }
    }

    #[test]
    fn an_encode_at_a_fractional_rate_is_timed_in_the_rates_own_ticks() {
        for (start, codec) in STARTS.into_iter().flat_map(|start| [VideoCodec::H264, VideoCodec::Hevc].map(|codec| (start, codec))) {
            encodetests::check_movie(VideoSpec { rate_num: 30000, rate_den: 1001, ..encodetests::spec(codec) }, start);
        }
    }

    #[test]
    fn a_picture_of_a_size_other_than_the_checks_is_laid_out_as_the_encoder_reads_it() {
        // Neither side a multiple of 16, and the height a phone's H.264 hardware takes.
        for (start, codec) in STARTS.into_iter().flat_map(|start| [VideoCodec::H264, VideoCodec::Hevc].map(|codec| (start, codec))) {
            encodetests::check_movie(VideoSpec { width: 200, height: 150, ..encodetests::spec(codec) }, start);
        }
    }

    #[test]
    fn a_movie_with_sound_has_both_tracks() {
        // The sound is encoded before either start reads the device's list, which starts the binder thread pool
        // the codec service calls back on.
        device_codecs();
        for start in STARTS {
            encodetests::check_movie_with_sound(start);
        }
    }

    #[test]
    fn a_picture_of_the_wrong_size_is_refused_by_the_codec() {
        for start in STARTS {
            encodetests::wrong_size_is_refused(start);
        }
    }

    #[test]
    fn the_export_declines_a_movie_the_device_has_no_hardware_encoder_for() {
        // The production path reads the device's own list, which the SDK installs; here it is the list the
        // device handed the test, installed the same way (AGENTS.md §3: nothing hidden from the check).
        give_device_codecs();
        for codec in [VideoCodec::H264, VideoCodec::Hevc] {
            let spec = encodetests::spec(codec);
            let listed = !mediacodec_input::hardware_encoders(device_codecs(), mime(codec)).is_empty();
            match (HardwareEncoder::start(spec), listed) {
                // What the platform's own answer says is what the export sees: nothing hardware means the
                // movie goes to EffectCraft's encoders, and the error names exactly that.
                (Err(EncodeError::Unavailable(why)), false) => assert_eq!(why, format!("this device has no hardware encoder for {}", mime(codec))),
                (Ok(_), true) => {}
                (other, listed) => panic!(
                    "{codec:?}: the start answered {} with the device listing hardware: {listed}",
                    match other {
                        Ok(_) => "a started encoder".to_string(),
                        Err(why) => format!("an error: {why}"),
                    }
                ),
            }
        }
    }

    #[test]
    fn a_picture_after_the_stream_ended_is_refused_as_being_past_the_movies_end() {
        use crate::movie::Backend;
        for start in STARTS {
            let Ok(mut encoder) = start(encodetests::spec(VideoCodec::H264)) else { continue };
            encoder.encode(0, &encodetests::picture_of(encodetests::SIZE, encodetests::SIZE, 0)).unwrap();
            encoder.finish().unwrap();
            let why = match encoder.encode(1, &encodetests::picture_of(encodetests::SIZE, encodetests::SIZE, 1)) {
                Err(EncodeError::Failed(why)) => why,
                Ok(_) => panic!("a picture after the end must fail, not encode"),
                Err(_) => panic!("a picture after the end must fail, not as another error"),
            };
            assert_eq!(why, "the encoder ended the stream before the movie was over");
        }
    }

    #[test]
    fn an_output_buffer_a_codec_has_no_memory_for_is_the_encoders_error_never_a_panic() {
        give_device_codecs();
        let encoder = Encoder::start_with(encodetests::spec(VideoCodec::H264), software_codec).unwrap();
        let picture = encoder.layout.packed(&encodetests::picture_of(encodetests::SIZE, encodetests::SIZE, 0), encodetests::SIZE, encodetests::SIZE).unwrap();
        mediacodec::queue_one(&encoder.codec, 0, 0, |input| mediacodec::fill(input, &picture).map(|()| picture.len()), || Ok(())).unwrap();
        let output = loop {
            match encoder.codec.dequeue_output_buffer(STALL).unwrap() {
                DequeuedOutputBufferInfoResult::Buffer(output) => break output,
                DequeuedOutputBufferInfoResult::TryAgainLater => panic!("the encoder gave nothing for a picture"),
                DequeuedOutputBufferInfoResult::OutputFormatChanged | DequeuedOutputBufferInfoResult::OutputBuffersChanged => {}
            }
        };
        // A stopped codec answers its buffers with null, as a failed one does.
        encoder.codec.stop().unwrap();
        assert_eq!(mediacodec::output_bytes(&output), Err("the codec gave no output buffer memory".to_string()));
    }

    #[test]
    fn the_packets_read_while_the_end_waits_for_an_input_buffer_are_all_kept() {
        give_device_codecs();
        let encoder = Encoder::start_with(encodetests::spec(VideoCodec::H264), software_codec).unwrap();
        // Pictures go in with nothing read out until the codec takes no more input, which it does once its
        // output is full: the end of the stream then waits for an input buffer, and what that wait reads out
        // is pictures of the movie.
        let mut fed = 0u64;
        while fed < 500 {
            let picture =
                encoder.layout.packed(&encodetests::picture_of(encodetests::SIZE, encodetests::SIZE, fed), encodetests::SIZE, encodetests::SIZE).unwrap();
            let time = encoder.clock(fed).unwrap();
            if mediacodec::queue_one(&encoder.codec, time, 0, |input| mediacodec::fill(input, &picture).map(|()| picture.len()), || Ok(())).is_err() {
                break;
            }
            fed += 1;
        }
        assert!(fed < 500, "the codec took 500 pictures with none read out, so the end never waited for it");
        encoder.fed.set(fed);
        let packets = encoder.finish().unwrap();
        assert_eq!(packets.len() as u64, fed, "a picture read while the end waited was lost");
    }

    #[test]
    fn a_quality_request_never_starts_a_codec() {
        let mut spec = encodetests::spec(VideoCodec::H264);
        spec.rate_control = crate::encode::RateControl::Quality { percent: 70 };
        let why = match Encoder::start_with(spec, software_codec) {
            Err(EncodeError::Unavailable(why)) => why,
            _other => panic!("a quality request must be declined as unavailable, not answered another way"),
        };
        assert_eq!(why, "this platform asks its video encoders for a bit rate, never a quality");
    }
}
