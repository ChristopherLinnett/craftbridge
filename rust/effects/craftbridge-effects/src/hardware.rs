//! Hardware H.264 and HEVC encoding in CraftBridge's exporter (AD7, plan §18).
//!
//! EffectCraft's encoders are internal to its export crate, so for an H.264 or HEVC job whose request chose
//! hardware encoding the exporter runs the same queue item through EffectCraft's own file exporter twice, as a
//! Targa sequence and as a WAV, each handed to an in-memory sink and never written to disk. Every frame is
//! therefore still rendered by EffectCraft's pipeline, with its timing, colour and size rules. The sink
//! receives the frames from the render threads in any order; they are turned into the planes EffectCraft's
//! own encoders are fed, put back in order and encoded by the device's hardware encoder, the sound is encoded
//! as AAC by the platform's encoder, and `craftbridge-media` writes the MP4, tagged as EffectCraft tags its own.
//! A hardware encoder whose session is lost part way (the system takes it from an app that went to the
//! background) hands the rest of an H.264 movie to the software encoder, from the first frame it made no packet
//! for; a movie that cannot go on that way ends in an error naming the lost session, and `info.encoding`
//! counts which encoder made which pictures.
//!
//! A job the hardware path cannot reproduce faithfully, or a device without the encoder, is declined before
//! anything renders, and goes to EffectCraft's exporter unchanged.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Cursor};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use craftbridge_media::{AacSpec, ColourCodes, EncodeError, Movie, RateControl, Resilient, VideoCodec, VideoSpec, Yuv420};
use effectcraft_engine::project::render_queue::{
    AudioOutput, Channels, CodecProfile, OutputFormat, RateControlMode, RenderLog, RenderQueueItem, RenderSettings, sequence_path,
};
use effectcraft_engine::project::{ColorSpace, ProjectSettings};
use effectcraft_engine::render_queue::{CANCELLED, ExportJob, ExportResult, Exporter};
use effectcraft_host::FileExporter;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as MediaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::exporter::PooledExporter;

/// Whether the request being answered chose hardware encoding: set for the length of one export, and false
/// otherwise, so an export that is not a request's (the project's own queue, a command run through `exec`)
/// never uses it.
#[derive(Clone, Default)]
pub(crate) struct HardwareChoice(Arc<AtomicBool>);

/// The choice, put back to false when it goes out of scope.
pub(crate) struct ChoiceScope<'a>(&'a HardwareChoice);

impl HardwareChoice {
    pub(crate) fn chosen(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Makes the choice `on` until the returned scope is dropped.
    pub(crate) fn scope(&self, on: bool) -> ChoiceScope<'_> {
        self.0.store(on, Ordering::SeqCst);
        ChoiceScope(self)
    }
}

impl Drop for ChoiceScope<'_> {
    fn drop(&mut self) {
        self.0.0.store(false, Ordering::SeqCst);
    }
}

/// The names of the files the two sub-jobs are handed, which only the sink sees.
const FRAMES: &str = "craftbridge-frames-[#####].tga";
const SOUND: &str = "craftbridge-sound.wav";

/// What the hardware path does for one job, worked out from the queue item before anything renders.
#[derive(Debug, PartialEq)]
struct Plan {
    spec: VideoSpec,
    frames: u64,
    /// The comp frame number of output frame 0, which the sequence's file names follow.
    first_frame: i64,
    /// The sound, when the job has one.
    audio: Option<AacSpec>,
    /// The channels EffectCraft renders the movie's frames in.
    channels: Channels,
}

/// Why a job stays with EffectCraft's exporter.
type Decline = &'static str;

/// The colour code points EffectCraft tags a movie with (`effectcraft: crates/export/src/encode.rs:22-58`):
/// the matrix is BT.709 as every encoder there converts with it, and the primaries and transfer follow the
/// output space, an alpha matte being no colour at all.
fn colour_codes(settings: &ProjectSettings, channels: Channels) -> ColourCodes {
    let output = settings.working_space.and(settings.output_space).filter(|_| channels != Channels::Alpha);
    let (primaries, transfer) = match output {
        None | Some(ColorSpace::Srgb | ColorSpace::Rec709) => (1, 1),
        Some(ColorSpace::Rec2020) => (9, 1),
        Some(ColorSpace::DisplayP3) => (12, 13),
        Some(ColorSpace::Rec2100Pq) => (9, 16),
        Some(ColorSpace::Rec2100Hlg) => (9, 18),
        // H.273 has no code for the ACES primaries: unspecified, in linear light.
        Some(ColorSpace::AcesCg | ColorSpace::Aces2065) => (2, 8),
    };
    ColourCodes { primaries, transfer, matrix: 1 }
}

impl Plan {
    fn of(job: &ExportJob) -> Result<Self, Decline> {
        let (item, output) = (job.item, &job.item.output);
        let codec = match output.format {
            OutputFormat::H264 => VideoCodec::H264,
            OutputFormat::Hevc => VideoCodec::Hevc,
            _ => return Err("only H.264 and HEVC movies are encoded in hardware"),
        };
        if codec == VideoCodec::Hevc && (output.codec.profile != CodecProfile::Main || output.codec.level.is_some()) {
            return Err("the hardware encoder takes HEVC at the Main profile and the level it picks");
        }
        // These ask for something a sink cannot give: files that already exist, an overflow folder (which only a
        // session that reports free space on its storage ever uses), or a log that describes the frames' own job.
        if item.settings.skip_existing || (item.settings.storage_overflow && job.storage.is_some()) || item.log != RenderLog::ErrorsOnly {
            return Err("skipping existing files, storage overflow and a render log stay with EffectCraft's exporter");
        }
        let comp = job.project.comp(item.comp).ok_or("the job names no composition")?;
        // A job always renders at least one frame.
        let frames = item.settings.frame_count(comp);
        let rate = item.settings.rate(comp);
        let (rate_num, rate_den) =
            (u32::try_from(rate.num).map_err(|_| "the frame rate is out of range")?, u32::try_from(rate.den).map_err(|_| "the frame rate is out of range")?);
        let (width, height) = output.output_size(comp, &item.settings);
        let channels = match output.channels {
            Channels::Rgba => Channels::Rgb,
            channels => channels,
        };
        let rate_control = match (codec, output.codec.rate_control) {
            (VideoCodec::Hevc, RateControlMode::Quality) => RateControl::Quality { percent: output.codec.quality },
            _ => RateControl::Bitrate { kbps: output.bitrate_kbps },
        };
        let spec = VideoSpec {
            codec,
            width,
            height,
            rate_num,
            rate_den,
            rate_control,
            // EffectCraft's H.264 encoder keys every two seconds whatever the module says, and its HEVC encoder
            // follows the module.
            keyframe_interval: match codec {
                VideoCodec::H264 => (rate.as_f64() * 2.0).round().max(1.0) as u32,
                VideoCodec::Hevc => output.keyint(rate.as_f64()),
            },
            colour: colour_codes(&job.project.settings, channels),
        };
        spec.check().map_err(|_| "the picture or the frame rate is one the encoder cannot take")?;
        let sound = match output.audio {
            AudioOutput::On => true,
            AudioOutput::Off => false,
            AudioOutput::Auto => effectcraft_render::audio::comp_has_audio(job.project, item.comp),
        };
        let audio =
            sound.then(|| AacSpec { sample_rate: output.audio_sample_rate.clamp(8_000, 192_000), channels: if output.audio_channels == 1 { 1 } else { 2 } });
        if let Some(audio) = &audio {
            audio.check().map_err(|_| "the sound is one the AAC encoder cannot take")?;
        }
        Ok(Self { spec, frames, first_frame: item.settings.first_frame(comp), audio, channels })
    }

    /// The item as a Targa sequence of the movie's frames, with no sound.
    fn frames_item(&self, item: &RenderQueueItem) -> RenderQueueItem {
        let mut frames = item.clone();
        frames.output.format = OutputFormat::TargaSequence;
        frames.output.channels = self.channels;
        frames.output.audio = AudioOutput::Off;
        frames.extra_outputs.clear();
        frames
    }

    /// The item as a WAV of the movie's sound, in the movie's rate and channels, as 16-bit samples.
    fn sound_item(&self, item: &RenderQueueItem) -> RenderQueueItem {
        let mut sound = item.clone();
        sound.output.format = OutputFormat::Wav;
        sound.output.audio = AudioOutput::On;
        sound.output.audio_format = effectcraft_engine::project::render_queue::AudioFormat::S16;
        sound.extra_outputs.clear();
        sound
    }
}

/// `job`, for `item` and `path`, handed to an exporter in place of the original.
fn variant<'a>(job: &'a ExportJob, item: &'a RenderQueueItem, path: &'a str) -> ExportJob<'a> {
    ExportJob {
        project: job.project,
        footage: job.footage,
        expr: job.expr,
        item,
        accel: job.accel,
        path,
        storage: job.storage,
        label: job.label.clone(),
        nested_switches: job.nested_switches,
        caches: job.caches,
        renderer: job.renderer,
        gpu_frames: job.gpu_frames,
    }
}

/// An exporter that writes into memory: EffectCraft's own, with `sink` for its files.
fn into_memory(sink: Arc<dyn Fn(&str, Vec<u8>) + Send + Sync>) -> FileExporter {
    FileExporter { sink: Some(sink) }
}

/// A picture of `width` by `height` as the planes EffectCraft's encoders are fed, from the Targa frame
/// EffectCraft wrote: the frame cut or padded to the coded size as EffectCraft's own movies fit theirs (the
/// top left kept, the rest black and clear), then converted with EffectCraft's own BT.709 limited-range
/// conversion.
fn planes_of(targa: &[u8], width: u32, height: u32) -> Result<Yuv420, String> {
    let frame =
        image::load_from_memory_with_format(targa, image::ImageFormat::Tga).map_err(|error| format!("a rendered frame could not be read: {error}"))?.to_rgba8();
    let (have_w, have_h) = (frame.width() as usize, frame.height() as usize);
    let (w, h) = (width as usize, height as usize);
    let rgba = if (have_w, have_h) == (w, h) {
        frame.into_raw()
    } else {
        let mut fitted = vec![0u8; w * h * 4];
        let row = have_w.min(w) * 4;
        let source = frame.as_raw();
        for (to, from) in fitted.chunks_mut(w * 4).zip(source.chunks(have_w * 4)).take(have_h.min(h)) {
            if let (Some(to), Some(from)) = (to.get_mut(..row), from.get(..row)) {
                to.copy_from_slice(from);
            }
        }
        fitted
    };
    let (mut y, mut u, mut v) = (Vec::new(), Vec::new(), Vec::new());
    effectcraft_raster::yuv::rgba_to_yuv420(&rgba, w, h, &mut y, &mut u, &mut v);
    Ok(Yuv420 { y, u, v })
}

/// The 16-bit interleaved samples of the WAV EffectCraft wrote, which must be the rate and channels the
/// movie's sound is made in.
fn samples_of(wav: Vec<u8>, audio: &AacSpec) -> Result<Vec<i16>, String> {
    let wrong = |error: MediaError| format!("the rendered sound could not be read: {error}");
    let source = MediaSourceStream::new(Box::new(Cursor::new(wav)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("wav");
    let probed = symphonia::default::get_probe().format(&hint, source, &FormatOptions::default(), &MetadataOptions::default()).map_err(wrong)?;
    let mut reader = probed.format;
    let track = reader.default_track().ok_or("the rendered sound has no track")?;
    let (id, parameters) = (track.id, track.codec_params.clone());
    if parameters.sample_rate != Some(audio.sample_rate) || parameters.channels.map(|channels| channels.count()) != usize::try_from(audio.channels).ok() {
        return Err("the rendered sound is not in the rate and channels the movie asks for".into());
    }
    let mut decoder = symphonia::default::get_codecs().make(&parameters, &DecoderOptions::default()).map_err(wrong)?;
    let mut samples = Vec::new();
    loop {
        let packet = match reader.next_packet() {
            Ok(packet) => packet,
            Err(MediaError::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(wrong(error)),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = decoder.decode(&packet).map_err(wrong)?;
        let mut buffer = SampleBuffer::<i16>::new(decoded.capacity() as u64, *decoded.spec());
        buffer.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buffer.samples());
    }
    Ok(samples)
}

/// A failure the sink met on a render thread, where it cannot be returned.
#[derive(Default)]
struct Failure(Mutex<Option<String>>);

impl Failure {
    /// Keeps the first failure.
    fn set(&self, why: String) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(why);
    }

    fn is_set(&self) -> bool {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).is_some()
    }

    fn take(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

fn describe(error: EncodeError) -> String {
    error.to_string()
}

/// The movie being written: shared by the sink's render threads, and taken back when the sub-job ends.
type SharedMovie = Arc<Mutex<Option<Movie<Resilient, BufWriter<File>>>>>;

/// The file the movie is written to, its folder made as EffectCraft makes it.
fn create(path: &str) -> Result<BufWriter<File>, String> {
    if let Some(folder) = std::path::Path::new(path).parent().filter(|folder| !folder.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    File::create(path).map(BufWriter::new).map_err(|error| error.to_string())
}

/// EffectCraft's own exporter, with the hardware encoder for the movies a request chose it for.
pub(crate) struct HardwareExporter {
    pooled: PooledExporter,
    choice: HardwareChoice,
}

impl HardwareExporter {
    pub(crate) fn new(choice: HardwareChoice) -> Self {
        Self::over(PooledExporter::new(), choice)
    }

    fn over(pooled: PooledExporter, choice: HardwareChoice) -> Self {
        Self { pooled, choice }
    }

    /// The job through the hardware encoder; `None` when it stays with EffectCraft's exporter, in which case
    /// nothing was written and nothing rendered for it.
    fn hardware(&self, job: &ExportJob, progress: &mut dyn FnMut(u64, u64) -> bool) -> Option<Result<ExportResult, String>> {
        let plan = Plan::of(job).ok()?;
        let started = Instant::now();
        let encoder = Resilient::start(plan.spec).ok()?;
        // The sound first, which is cheap beside the frames: a sound the platform cannot encode declines the
        // job before any frame has rendered.
        let track = match &plan.audio {
            Some(audio) => match self.sound(&plan, audio, job, progress) {
                Ok(Some(track)) => Some(track),
                Ok(None) => return None,
                Err(stopped) => return Some(Err(stopped)),
            },
            None => None,
        };
        Some(self.movie(&plan, encoder, track, job, progress, started))
    }

    /// The movie's sound, encoded; `Ok(None)` where the sound cannot be had here (the platform has no AAC
    /// encoder, or EffectCraft's own export of it failed, which its exporter then reports in its own words), and
    /// the job is declined; an error only for a stop, which ends the job.
    fn sound(
        &self,
        plan: &Plan,
        audio: &AacSpec,
        job: &ExportJob,
        progress: &mut dyn FnMut(u64, u64) -> bool,
    ) -> Result<Option<craftbridge_media::AudioTrack>, String> {
        let wav = Arc::new(Mutex::new(None));
        let held = wav.clone();
        let exporter = into_memory(Arc::new(move |_, bytes| *held.lock().unwrap_or_else(PoisonError::into_inner) = Some(bytes)));
        let item = plan.sound_item(job.item);
        let frames = plan.frames;
        // The sound renders in one pass reporting frames; the movie's progress is its pictures', so this only
        // asks the caller whether to go on.
        let rendered = self.pooled.run(&exporter, &variant(job, &item, SOUND), &mut |_, _| progress(0, frames));
        match rendered {
            Err(stopped) if stopped == CANCELLED => return Err(stopped),
            Err(_) => return Ok(None),
            Ok(_) => {}
        }
        let bytes = wav.lock().unwrap_or_else(PoisonError::into_inner).take();
        Ok(bytes.and_then(|bytes| samples_of(bytes, audio).ok()).and_then(|samples| craftbridge_media::encode_aac(*audio, &samples).ok()))
    }

    fn movie(
        &self,
        plan: &Plan,
        encoder: Resilient,
        track: Option<craftbridge_media::AudioTrack>,
        job: &ExportJob,
        progress: &mut dyn FnMut(u64, u64) -> bool,
        started: Instant,
    ) -> Result<ExportResult, String> {
        let file = create(job.path)?;
        let movie = Movie::new(encoder, file, plan.spec, plan.frames, track).map_err(describe)?;
        let movie: SharedMovie = Arc::new(Mutex::new(Some(movie)));
        let failure = Arc::new(Failure::default());
        let names: HashMap<String, u64> =
            (0..plan.frames).map(|index| (sequence_path(FRAMES, plan.first_frame.saturating_add_unsigned(index)), index)).collect();
        let sink = {
            let (movie, failure, spec) = (movie.clone(), failure.clone(), plan.spec);
            move |path: &str, bytes: Vec<u8>| {
                if failure.is_set() {
                    return;
                }
                let Some(&index) = names.get(path) else { return failure.set(format!("EffectCraft wrote a file the movie did not ask for: {path}")) };
                let pushed =
                    planes_of(&bytes, spec.width, spec.height).and_then(|planes| match movie.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
                        Some(movie) => movie.push_frame(index, planes).map_err(describe),
                        None => Err("the movie was already finished".into()),
                    });
                if let Err(why) = pushed {
                    failure.set(why);
                }
            }
        };
        let item = plan.frames_item(job.item);
        let rendered =
            self.pooled.run(&into_memory(Arc::new(sink)), &variant(job, &item, FRAMES), &mut |done, total| progress(done, total) && !failure.is_set());
        let finished = match (failure.take(), rendered) {
            (Some(why), _) => Err(why),
            (None, Err(why)) => Err(why),
            (None, Ok(_)) => match movie.lock().unwrap_or_else(PoisonError::into_inner).take() {
                Some(movie) => movie.finish().map_err(describe),
                None => Err("the movie was already finished".into()),
            },
        };
        match finished {
            Ok(done) => Ok(ExportResult {
                path: job.path.to_string(),
                frames: done.frames,
                width: plan.spec.width,
                height: plan.spec.height,
                bytes: done.bytes,
                seconds: started.elapsed().as_secs_f64(),
                audio: plan.audio.is_some(),
                ..ExportResult::default()
            }),
            Err(why) => {
                // This path removes its own half-written movie when it stops or fails. EffectCraft's exporter
                // does not: it leaves its file after a failure, so what makes a failed export leave no
                // output on either path is the core, which deletes every file the export listed.
                let _ = std::fs::remove_file(job.path);
                Err(if why == CANCELLED { CANCELLED.to_string() } else { why })
            }
        }
    }
}

impl Exporter for HardwareExporter {
    fn formats(&self) -> Vec<OutputFormat> {
        self.pooled.formats()
    }

    fn supports_skip_existing(&self, settings: &RenderSettings) -> bool {
        self.pooled.supports_skip_existing(settings)
    }

    fn export(&self, job: &ExportJob, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<ExportResult, String> {
        // The memory is asked before either path starts anything: the hardware path starts the device's
        // encoder, encodes the sound and makes the movie's file and folder before its frames reach the
        // pool, so a frame the memory refuses is refused here, while nothing of the export exists.
        self.pooled.admit(job)?;
        if self.choice.chosen()
            && let Some(done) = self.hardware(job, progress)
        {
            return done;
        }
        self.pooled.export(job, progress)
    }
}

#[cfg(test)]
mod tests {
    use effectcraft_engine::project::render_queue::{AudioFormat, OutputModule};
    use image::ImageEncoder;

    use super::*;
    use crate::exporter::tests::{project_with_comp, with_job, with_job_at};

    /// The plan the job for `item` over a `width` by `height` composition gets.
    fn plan_of(width: u32, height: u32, change: impl FnOnce(&mut RenderQueueItem)) -> Result<Plan, Decline> {
        let (project, mut item) = project_with_comp(width, height);
        change(&mut item);
        with_job(&project, &item, Plan::of)
    }

    #[test]
    fn an_h264_job_is_planned_from_its_item_as_effectcrafts_encoder_is_set_up() {
        let plan = plan_of(1920, 1080, |_| {}).unwrap();
        assert_eq!(
            plan.spec,
            VideoSpec {
                codec: VideoCodec::H264,
                width: 1920,
                height: 1080,
                rate_num: 10,
                rate_den: 1,
                rate_control: RateControl::Bitrate { kbps: 10_000 },
                keyframe_interval: 20,
                colour: ColourCodes { primaries: 1, transfer: 1, matrix: 1 },
            }
        );
        assert_eq!((plan.frames, plan.first_frame, plan.audio, plan.channels), (10, 0, None, Channels::Rgb));
    }

    #[test]
    fn a_frame_is_planned_at_the_coded_size_which_rounds_odd_sides_down() {
        let plan = plan_of(1921, 1081, |_| {}).unwrap();
        assert_eq!((plan.spec.width, plan.spec.height), (1920, 1080));
        let plan = plan_of(1920, 1080, |item| {
            item.settings.resolution = 0.5;
        })
        .unwrap();
        assert_eq!((plan.spec.width, plan.spec.height), (960, 540));
    }

    #[test]
    fn an_hevc_job_follows_its_rate_control_bitrate_and_key_frame_interval() {
        let bitrate = plan_of(64, 64, |item| {
            item.output.format = OutputFormat::Hevc;
            item.output.bitrate_kbps = 4000;
            item.output.keyframe_interval = 7;
        })
        .unwrap();
        assert_eq!((bitrate.spec.codec, bitrate.spec.rate_control, bitrate.spec.keyframe_interval), (VideoCodec::Hevc, RateControl::Bitrate { kbps: 4000 }, 7));
        let quality = plan_of(64, 64, |item| {
            item.output.format = OutputFormat::Hevc;
            item.output.codec.rate_control = RateControlMode::Quality;
            item.output.codec.quality = 80;
        })
        .unwrap();
        assert_eq!(quality.spec.rate_control, RateControl::Quality { percent: 80 });
        // H.264 has no constant-quality mode, so its rate control is the bit rate whatever the module says.
        let h264 = plan_of(64, 64, |item| {
            item.output.codec.rate_control = RateControlMode::Quality;
            item.output.keyframe_interval = 7;
        })
        .unwrap();
        assert_eq!(h264.spec.keyframe_interval, 20, "H.264 keys every two seconds, as EffectCraft's encoder does");
        assert_eq!(h264.spec.rate_control, RateControl::Bitrate { kbps: 10_000 });
    }

    #[test]
    fn every_format_but_h264_and_hevc_stays_with_effectcrafts_exporter() {
        for format in OutputFormat::ALL {
            match plan_of(64, 64, |item| item.output.format = format) {
                Ok(_) => assert!(matches!(format, OutputFormat::H264 | OutputFormat::Hevc), "{format:?}"),
                Err(words) => {
                    assert!(!matches!(format, OutputFormat::H264 | OutputFormat::Hevc), "{format:?}");
                    assert_eq!(words, "only H.264 and HEVC movies are encoded in hardware");
                }
            }
        }
    }

    #[test]
    fn what_the_hardware_encoder_does_not_take_stays_with_effectcrafts_exporter() {
        let hevc = |change: fn(&mut OutputModule)| {
            plan_of(64, 64, move |item| {
                item.output.format = OutputFormat::Hevc;
                change(&mut item.output);
            })
        };
        let words = "the hardware encoder takes HEVC at the Main profile and the level it picks";
        assert_eq!(hevc(|output| output.codec.profile = CodecProfile::Main10).unwrap_err(), words);
        assert_eq!(hevc(|output| output.codec.level = Some(41)).unwrap_err(), words);
        assert!(hevc(|_| {}).is_ok());
        // The Main 10 and level rules are HEVC's: H.264 reads neither.
        assert!(plan_of(64, 64, |item| item.output.codec.profile = CodecProfile::Main10).is_ok());
    }

    #[test]
    fn what_a_memory_sink_cannot_give_stays_with_effectcrafts_exporter() {
        let words = "skipping existing files, storage overflow and a render log stay with EffectCraft's exporter";
        assert_eq!(plan_of(64, 64, |item| item.settings.skip_existing = true).unwrap_err(), words);
        assert_eq!(plan_of(64, 64, |item| item.log = RenderLog::PlusSettings).unwrap_err(), words);
        assert_eq!(plan_of(64, 64, |item| item.log = RenderLog::PlusPerFrameInfo).unwrap_err(), words);
        // Overflow is on by default and only a session with a storage hook ever uses it.
        assert!(plan_of(64, 64, |item| item.settings.storage_overflow = true).is_ok());
    }

    #[test]
    fn a_job_that_asks_for_overflow_through_a_storage_hook_stays_with_effectcrafts_exporter() {
        struct Quota;
        impl effectcraft_engine::project::render_queue::StorageQuota for Quota {
            fn has_room(&self, _: &str, _: u64) -> bool {
                true
            }
        }
        let (project, item) = project_with_comp(64, 64);
        let footage = effectcraft_media::MediaPool::new();
        let job = ExportJob {
            project: &project,
            footage: &footage,
            expr: None,
            item: &item,
            accel: None,
            path: "out.mp4",
            storage: Some(&Quota),
            label: String::new(),
            nested_switches: false,
            caches: None,
            renderer: None,
            gpu_frames: None,
        };
        assert_eq!(Plan::of(&job).unwrap_err(), "skipping existing files, storage overflow and a render log stay with EffectCraft's exporter");
    }

    #[test]
    fn a_job_without_a_composition_stays_with_effectcrafts_exporter() {
        assert_eq!(plan_of(64, 64, |item| item.comp = effectcraft_engine::project::ItemId(9999)).unwrap_err(), "the job names no composition");
    }

    #[test]
    fn the_sound_is_planned_from_the_module_and_the_comp() {
        let on = plan_of(64, 64, |item| item.output.audio = AudioOutput::On).unwrap();
        assert_eq!(on.audio, Some(AacSpec { sample_rate: 48_000, channels: 2 }));
        let mono = plan_of(64, 64, |item| {
            item.output.audio = AudioOutput::On;
            item.output.audio_channels = 1;
            item.output.audio_sample_rate = 44_100;
        })
        .unwrap();
        assert_eq!(mono.audio, Some(AacSpec { sample_rate: 44_100, channels: 1 }));
        assert_eq!(plan_of(64, 64, |item| item.output.audio = AudioOutput::Off).unwrap().audio, None);
        assert_eq!(plan_of(64, 64, |item| item.output.audio = AudioOutput::Auto).unwrap().audio, None, "a comp with no sound has none");
        // A rate is held to EffectCraft's own range before it is checked.
        let low = plan_of(64, 64, |item| {
            item.output.audio = AudioOutput::On;
            item.output.audio_sample_rate = 100;
        })
        .unwrap();
        assert_eq!(low.audio, Some(AacSpec { sample_rate: 8_000, channels: 2 }));
    }

    #[test]
    fn a_sound_the_aac_encoder_cannot_take_stays_with_effectcrafts_exporter() {
        let odd = plan_of(64, 64, |item| {
            item.output.audio = AudioOutput::On;
            item.output.audio_sample_rate = 45_000;
        });
        assert_eq!(odd.unwrap_err(), "the sound is one the AAC encoder cannot take");
    }

    #[test]
    fn an_rgba_movie_is_rendered_as_rgb_and_an_alpha_matte_stays_one() {
        assert_eq!(plan_of(64, 64, |item| item.output.channels = Channels::Rgba).unwrap().channels, Channels::Rgb);
        assert_eq!(plan_of(64, 64, |item| item.output.channels = Channels::Alpha).unwrap().channels, Channels::Alpha);
    }

    #[test]
    fn the_colour_tags_follow_the_output_space_as_effectcrafts_movies_do() {
        let tagged = |output_space: Option<ColorSpace>, channels| {
            let settings = ProjectSettings { working_space: output_space.map(|_| ColorSpace::Srgb), output_space, ..ProjectSettings::default() };
            let codes = colour_codes(&settings, channels);
            (codes.primaries, codes.transfer, codes.matrix)
        };
        assert_eq!(tagged(None, Channels::Rgb), (1, 1, 1));
        for (space, expected) in [
            (ColorSpace::Srgb, (1, 1, 1)),
            (ColorSpace::Rec709, (1, 1, 1)),
            (ColorSpace::Rec2020, (9, 1, 1)),
            (ColorSpace::DisplayP3, (12, 13, 1)),
            (ColorSpace::Rec2100Pq, (9, 16, 1)),
            (ColorSpace::Rec2100Hlg, (9, 18, 1)),
            (ColorSpace::AcesCg, (2, 8, 1)),
            (ColorSpace::Aces2065, (2, 8, 1)),
        ] {
            assert_eq!(tagged(Some(space), Channels::Rgb), expected, "{space:?}");
        }
        assert_eq!(tagged(Some(ColorSpace::Rec2100Pq), Channels::Alpha), (1, 1, 1), "an alpha matte is not in the output space");
        let unmanaged = ProjectSettings { working_space: None, output_space: Some(ColorSpace::Rec2020), ..ProjectSettings::default() };
        assert_eq!(colour_codes(&unmanaged, Channels::Rgb).primaries, 1, "only a working space converts to the output space");
    }

    #[test]
    fn the_frames_job_is_the_item_as_a_targa_sequence_of_the_movies_channels_with_no_sound() {
        let (_, mut item) = project_with_comp(64, 64);
        item.output.audio = AudioOutput::On;
        item.output.channels = Channels::Rgba;
        item.extra_outputs.push(OutputModule::default());
        let plan = plan_of(64, 64, |_| {}).unwrap();
        let frames = plan.frames_item(&item);
        assert_eq!(frames.output.format, OutputFormat::TargaSequence);
        assert_eq!((frames.output.channels, frames.output.audio), (Channels::Rgb, AudioOutput::Off));
        assert!(frames.extra_outputs.is_empty());
        assert_eq!(
            (frames.comp, frames.settings.clone(), frames.output.crop, frames.output.resize),
            (item.comp, item.settings.clone(), item.output.crop, item.output.resize)
        );
    }

    #[test]
    fn the_sound_job_is_the_item_as_a_wav_of_sixteen_bit_samples_in_the_movies_rate_and_channels() {
        let (_, mut item) = project_with_comp(64, 64);
        item.output.audio_format = AudioFormat::F32;
        item.output.audio_sample_rate = 44_100;
        item.output.audio_channels = 1;
        item.output.audio = AudioOutput::Off;
        let sound = plan_of(64, 64, |_| {}).unwrap().sound_item(&item);
        assert_eq!((sound.output.format, sound.output.audio_format, sound.output.audio), (OutputFormat::Wav, AudioFormat::S16, AudioOutput::On));
        assert_eq!((sound.output.audio_sample_rate, sound.output.audio_channels), (44_100, 1));
    }

    #[test]
    fn a_sub_job_is_the_job_with_another_item_and_path() {
        let (project, item) = project_with_comp(64, 64);
        let other = RenderQueueItem::new(2, item.comp);
        with_job(&project, &item, |job| {
            let sub = variant(job, &other, "elsewhere.wav");
            assert_eq!((sub.item.id, sub.path, sub.label.as_str(), sub.nested_switches), (2, "elsewhere.wav", "#1 A", false));
        });
    }

    fn targa(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut file = Cursor::new(Vec::new());
        image::codecs::tga::TgaEncoder::new(&mut file).disable_rle().write_image(rgba, width, height, image::ExtendedColorType::Rgba8).unwrap();
        file.into_inner()
    }

    /// A picture whose every pixel is its own colour.
    fn colourful(width: u32, height: u32) -> Vec<u8> {
        (0..width * height).flat_map(|n| [(n * 37 % 256) as u8, (n * 91 % 256) as u8, (n * 13 % 256) as u8, 255]).collect()
    }

    fn planes_of_rgba(rgba: &[u8], w: usize, h: usize) -> Yuv420 {
        let (mut y, mut u, mut v) = (Vec::new(), Vec::new(), Vec::new());
        effectcraft_raster::yuv::rgba_to_yuv420(rgba, w, h, &mut y, &mut u, &mut v);
        Yuv420 { y, u, v }
    }

    #[test]
    fn a_frame_of_the_coded_size_becomes_the_planes_effectcrafts_encoders_are_fed() {
        let rgba = colourful(4, 4);
        assert_eq!(planes_of(&targa(4, 4, &rgba), 4, 4).unwrap(), planes_of_rgba(&rgba, 4, 4));
    }

    #[test]
    fn a_frame_one_pixel_too_large_is_cut_to_the_coded_size_keeping_the_top_left() {
        let rgba = colourful(5, 3);
        let mut cut = Vec::new();
        for row in 0..2usize {
            cut.extend_from_slice(rgba.get(row * 5 * 4..row * 5 * 4 + 4 * 4).unwrap());
        }
        assert_eq!(planes_of(&targa(5, 3, &rgba), 4, 2).unwrap(), planes_of_rgba(&cut, 4, 2));
    }

    #[test]
    fn a_frame_smaller_than_the_coded_size_is_padded_with_clear_black() {
        let rgba = colourful(1, 1);
        let mut padded = vec![0u8; 2 * 2 * 4];
        padded.get_mut(..4).unwrap().copy_from_slice(&rgba);
        assert_eq!(planes_of(&targa(1, 1, &rgba), 2, 2).unwrap(), planes_of_rgba(&padded, 2, 2));
    }

    #[test]
    fn a_frame_that_is_not_a_targa_file_is_refused_in_words() {
        let error = planes_of(b"not a picture", 2, 2).unwrap_err();
        assert!(error.starts_with("a rendered frame could not be read: "), "{error}");
    }

    /// A WAV of 16-bit samples.
    fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|sample| sample.to_le_bytes()).collect();
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        out.extend_from_slice(&(channels * 2).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn the_samples_of_a_wav_are_read_back_exactly() {
        let samples: Vec<i16> = (0..4000).map(|n| (n * 7 % 20_000) as i16 - 10_000).collect();
        let spec = AacSpec { sample_rate: 48_000, channels: 2 };
        assert_eq!(samples_of(wav(48_000, 2, &samples), &spec).unwrap(), samples);
    }

    #[test]
    fn a_wav_in_another_rate_or_channels_than_the_movies_is_refused() {
        let spec = AacSpec { sample_rate: 48_000, channels: 2 };
        let words = "the rendered sound is not in the rate and channels the movie asks for";
        assert_eq!(samples_of(wav(44_100, 2, &[0; 8]), &spec).unwrap_err(), words);
        assert_eq!(samples_of(wav(48_000, 1, &[0; 8]), &spec).unwrap_err(), words);
    }

    #[test]
    fn a_wav_that_is_not_one_is_refused_in_words() {
        let error = samples_of(b"RIFF nonsense".to_vec(), &AacSpec { sample_rate: 48_000, channels: 2 }).unwrap_err();
        assert!(error.starts_with("the rendered sound could not be read: "), "{error}");
    }

    #[test]
    fn a_frame_the_memory_cannot_hold_is_refused_before_the_hardware_path_makes_anything() {
        // A 1080p frame in flight on the CPU needs 132710400 bytes; the device here has one megabyte.
        let (project, item) = project_with_comp(1920, 1080);
        let scratch = tempfile::tempdir().expect("a scratch folder");
        let folder = scratch.path().join("made by the export");
        let path = folder.join("movie.mp4");
        let choice = HardwareChoice::default();
        let exporter = HardwareExporter::over(PooledExporter::over(Arc::new(FileExporter::default()), || Ok(1 << 20)), choice.clone());
        let _chosen = choice.scope(true);
        let refused = with_job_at(&project, &item, None, path.to_str().expect("a scratch path is text"), |job| exporter.export(job, &mut |_, _| true))
            .expect_err("the frame cannot fit");
        assert_eq!(refused, "a frame of 1920x1080 needs 132710400 bytes to render on the CPU and write, and the device reports 1048576 bytes available");
        assert!(!folder.exists(), "the refused export made its movie's folder, so something of it was written first");
    }

    #[test]
    fn the_choice_lasts_for_its_scope_and_is_off_after_it() {
        let choice = HardwareChoice::default();
        assert!(!choice.chosen());
        {
            let _scope = choice.scope(true);
            assert!(choice.chosen());
        }
        assert!(!choice.chosen());
        let _scope = choice.scope(false);
        assert!(!choice.chosen());
        // A copy is the same choice, as the session's exporter and the app share it.
        let copy = choice.clone();
        let _on = copy.scope(true);
        assert!(choice.chosen());
    }

    #[test]
    fn the_first_failure_is_the_one_kept() {
        let failure = Failure::default();
        assert!(!failure.is_set());
        failure.set("first".into());
        failure.set("second".into());
        assert!(failure.is_set());
        assert_eq!(failure.take(), Some("first".to_string()));
        assert!(!failure.is_set());
    }

    #[test]
    fn the_exporter_writes_the_formats_and_skips_as_effectcrafts_does() {
        let exporter = HardwareExporter::new(HardwareChoice::default());
        assert_eq!(exporter.formats(), FileExporter::default().formats());
        assert_eq!(exporter.supports_skip_existing(&RenderSettings::default()), FileExporter::default().supports_skip_existing(&RenderSettings::default()));
    }

    #[test]
    fn the_names_the_sub_jobs_are_handed_name_their_frames() {
        assert_eq!(sequence_path(FRAMES, 0), "craftbridge-frames-00000.tga");
        assert_eq!(sequence_path(FRAMES, 123), "craftbridge-frames-00123.tga");
        assert_eq!(sequence_path(FRAMES, 123_456), "craftbridge-frames-123456.tga");
    }
}
