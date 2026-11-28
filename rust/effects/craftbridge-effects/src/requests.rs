//! Two requests, each resolved the way the EffectCraft command line resolves the flags it stands
//! for (plan §14, §17, §19): [`RenderRequest`], the parameters of `export`, and `FrameRequest`, what a
//! `frame` call asks for; and the fit of a `frame` in a box, `previewRequest`.
//!
//! # `export`
//!
//! [`RenderRequest`] is one field per flag of the command line's `render`, and the render-queue
//! parameters they stand for (AD3).
//!
//! The parameters are read as every method's are (`methods.rs`): an unknown name is refused with
//! every unknown name it has, and a value of the wrong kind with the name it was given under, in
//! the words every other method refuses its own in, as the command line names the flag it refuses.
//!
//! # `frame`
//!
//! `FrameRequest` is what a `frame` call asks for, resolved the way the command line's `render-frame`
//! resolves its flags, and the render that answers it.
//!
//! The request is `{comp?, time?, frame?, maxSide?, scale?, transparent?}`. The resolution is the
//! command line's own, so a frame asked for here is the frame `render-frame` writes for the same
//! flags, byte for byte once decoded:
//!
//! * `comp` is a comp id, a name, or `-` for the active comp; a string of digits is an id, as the
//!   command line reads one, and no `comp` is the active comp;
//! * `time` is seconds in the comp, and `frame` a frame number divided by the comp's frame rate (30
//!   where the comp states none); with neither, the active comp's current time, whichever comp is
//!   named;
//! * `maxSide` is the longest side in pixels, `scale` that factor of the comp's longest side rounded
//!   to a whole number and at least 1, and with neither, or a `maxSide` of 0, the comp's full size.
//!   A number is cut to a whole number of pixels the way the command line's cast cuts it, a negative
//!   one reading 0;
//! * `transparent` keeps the frame's straight alpha instead of compositing it over the comp's
//!   background colour.
//!
//! Where the command line lets one flag quietly override another, `frame` over `time` and `scale`
//! over `maxSide`, a request that names both is refused instead: the command line's own usage gives
//! each pair as alternatives (`[--time S | --frame N] [--max-side PX | --scale K]`), as the plan's
//! table does, and a request with two answers has no one frame to give.
//!
//! A comp that is not there fails in the command line's words whatever the format, so an app that
//! asks for the same frame as PNG or as pixels hears one refusal.
//!
//! # `previewRequest`
//!
//! [`preview_request`] answers the `frame` request that draws a comp with its own alpha fitted inside a
//! box of pixels, the same request as JSON text, and the comp's pixel size, so a caller that draws
//! frames into a texture of its own need not read the comp and work the fit out itself. It stands for
//! no flag of the command line: its request is a `FrameRequest`'s own fields, and its comp is read as
//! `frame` reads one.

use std::borrow::Cow;

use craftbridge_core::{CraftError, FrameBytes, PixelFormat, Result, Surface};
use effectcraft_automation::{Backend, Error};
use effectcraft_engine::EngineError;
use effectcraft_engine::project::ItemId;
use effectcraft_engine::project::render_queue::OutputFormat;
use effectcraft_gpu::{DisplayFrame, Gpu};
use effectcraft_render::{RenderOpts, frame_size};
use effectcraft_time::Tick;
use serde_json::Value;
use serde_json::map::Map;

use crate::methods::{bool_param, failure, number_param, object, object_params, reject_unknown, string_param, tool_json, whole_number_words, whole_param};
use crate::reference::{Given, given, reference};

/// The method a refusal of these parameters names.
const EXPORT: &str = "export";

/// The two fields a request cannot set together.
const QUEUE: &str = "queue";
const HARDWARE: &str = "hardware";

/// Every parameter `export` takes, one per flag of `render`; any other name is refused.
const FIELDS: [&str; 27] = [
    "comp",
    "output",
    "format",
    "start",
    "end",
    "workArea",
    "fps",
    "resolution",
    "scale",
    "quality",
    "channels",
    "jpegQuality",
    "bitrate",
    "prores",
    "audio",
    "profile",
    "level",
    "rateControl",
    "videoQuality",
    "keyint",
    "webmCodec",
    "audioBitrate",
    "opusApp",
    "exrCompression",
    "exrDepth",
    QUEUE,
    HARDWARE,
];

/// A composition named by its id or by its name.
#[derive(Debug, Clone, PartialEq)]
pub enum CompRef {
    /// The composition's item id.
    Id(u64),
    /// The composition's name.
    Name(String),
}

impl CompRef {
    /// The `comp` parameter of a call to `method`: a whole number is an id and any text a name;
    /// anything else is refused, naming the method. The text is kept as given, for the engine to
    /// resolve as it resolves `render --comp`'s, so `-` is a name here and not the active comp as it
    /// is to [`reference`].
    pub(crate) fn param(obj: &Map<String, Value>, method: &str) -> Result<Option<Self>> {
        Ok(given(obj, "comp", method)?.map(|given| match given {
            Given::Id(id) => CompRef::Id(id),
            Given::Text(name) => CompRef::Name(name.to_string()),
        }))
    }

    /// The render queue's `comp`: an id as a number, a name as its text, which the command line
    /// passes as it was given, so the engine resolves it as it resolves the command line's.
    fn to_value(&self) -> Value {
        match self {
            CompRef::Id(id) => Value::from(*id),
            CompRef::Name(name) => Value::from(name.as_str()),
        }
    }
}

/// The output size: a factor of the composition's size, or a preset the render queue names
/// (`full`, `half`, `third`, `quarter`).
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// A factor; text that reads as a number is one, as the command line reads it.
    Factor(f64),
    /// A preset name.
    Preset(String),
}

impl Resolution {
    /// The size parameter `key`: a number, or text, which is a factor when it reads as a number and
    /// otherwise a preset the render queue checks.
    fn param(obj: &Map<String, Value>, key: &str) -> Result<Option<Self>> {
        match obj.get(key) {
            Some(Value::String(text)) => Ok(Some(text.parse::<f64>().map_or_else(|_| Resolution::Preset(text.clone()), Resolution::Factor))),
            Some(Value::Number(_)) => Ok(number_param(obj, key, EXPORT)?.map(Resolution::Factor)),
            None | Some(Value::Null) => Ok(None),
            Some(_) => Err(CraftError::invalid_params(format!("`{EXPORT}`: `{key}` must be a factor or a preset name"))),
        }
    }

    /// The render queue's `resolution`: a factor as a number, a preset as its name.
    fn to_value(&self) -> Value {
        match self {
            Resolution::Factor(factor) => Value::from(*factor),
            Resolution::Preset(name) => Value::from(name.as_str()),
        }
    }
}

/// What `export` renders and how: one field per flag of the command line's `render`, with the
/// command line's defaults, so a field left out is the flag left off.
///
/// Every number is a JSON number and every other value is the text the flag takes, checked by the
/// render queue exactly as the command line's are (`format`, `quality`, `channels` and the rest).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderRequest {
    /// `--comp`: the composition to render, the active one when left out.
    pub comp: Option<CompRef>,
    /// `--out`: where to write, an absolute path inside the container. The command line resolves a
    /// relative path against its working folder; a session has none the app knows, and the
    /// confinement refuses a relative path.
    pub output: Option<String>,
    /// `--format`: any name the render queue knows a format by — `h264`, `hevc`, `av1`, `prores`,
    /// `uncompressed`, `webm`, `png`, `jpeg`, `tiff`, `exr`, `tga`, `gif`, `wav` or `aiff`, or an
    /// alias such as `mp4` — and the extension of `output` when left out.
    pub format: Option<String>,
    /// `--start`: the first second to render.
    pub start: Option<f64>,
    /// `--end`: the last second to render.
    pub end: Option<f64>,
    /// `--work-area`: render the work area; with neither `start` nor `end`, the whole composition.
    pub work_area: bool,
    /// `--fps`: the output frame rate.
    pub fps: Option<f64>,
    /// `--resolution`.
    pub resolution: Option<Resolution>,
    /// `--scale`: the command line's other name for `resolution`; `resolution` wins when both are
    /// given.
    pub scale: Option<Resolution>,
    /// `--quality`: `best` or `draft`.
    pub quality: Option<String>,
    /// `--channels`: `rgb` or `rgba`.
    pub channels: Option<String>,
    /// `--jpeg-quality`; it sets the same render-queue quality as `quality`, and wins over it.
    pub jpeg_quality: Option<f64>,
    /// `--bitrate` in kbit/s; for WebM it also asks the encoder to target the bitrate.
    pub bitrate: Option<f64>,
    /// `--prores`: the ProRes profile.
    pub prores: Option<String>,
    /// `--audio`: `auto`, `on` or `off`.
    pub audio: Option<String>,
    /// `--profile`: HEVC and AV1 profile.
    pub profile: Option<String>,
    /// `--level`: HEVC and AV1 level.
    pub level: Option<String>,
    /// `--rate-control`: `bitrate` or `quality`.
    pub rate_control: Option<String>,
    /// `--video-quality`; it sets the same render-queue quality as `jpegQuality`, and wins over it.
    pub video_quality: Option<f64>,
    /// `--keyint`: frames between keyframes.
    pub keyint: Option<f64>,
    /// `--webm-codec`: `vp9` or `av1`.
    pub webm_codec: Option<String>,
    /// `--audio-bitrate` in kbit/s.
    pub audio_bitrate: Option<f64>,
    /// `--opus-app`: `audio` or `voice`.
    pub opus_app: Option<String>,
    /// `--exr-compression`.
    pub exr_compression: Option<String>,
    /// `--exr-depth`: `half` or `float`.
    pub exr_depth: Option<String>,
    /// `--queue`: render the project's own queue; every other field is then left unread, as the
    /// command line leaves its flags, so a value it would refuse is not refused either. The one it is
    /// refused beside is a `hardware` of `true`, since a queued render encodes through the render
    /// queue's encoders.
    pub queue: bool,
    /// CraftBridge's own option, no flag of the command line: encode an H.264 or HEVC movie with the
    /// device's hardware encoder where it has one (plan §18). Every other format, and a device without the
    /// encoder, export through EffectCraft's encoders as the command line does.
    pub hardware: bool,
}

impl RenderRequest {
    /// The parameters of an `export` call as an object of the fields it takes, refused otherwise: what
    /// the command line's own parser refuses of its flags, an option it does not know, before it reads
    /// any of them. A request that queues and chooses hardware is refused here too, as the one place
    /// every render request is read first: the project's own queue encodes through the render queue's
    /// encoders, so the choice would be dropped without a word.
    pub(crate) fn fields(params: &Value) -> Result<Cow<'_, Map<String, Value>>> {
        let obj = object_params(params, EXPORT)?;
        reject_unknown(&obj, EXPORT, &FIELDS)?;
        let is_true = |key: &str| obj.get(key) == Some(&Value::Bool(true));
        if is_true(QUEUE) && is_true(HARDWARE) {
            return Err(CraftError::invalid_params(format!(
                "`{EXPORT}`: `{QUEUE}` cannot be set with `{HARDWARE}`: a queued render encodes through the render queue and its own encoders, never the device's hardware encoder"
            )));
        }
        Ok(obj)
    }

    /// Whether the render request `request` chooses hardware encoding, for every method that renders
    /// one: each names where its request sits in its own parameters, and this reads it. A request that
    /// does not read chooses nothing, and the method that renders it refuses it, in the request's own
    /// words, before anything renders.
    pub(crate) fn chooses_hardware(request: &Value) -> bool {
        Self::from_params(request).is_ok_and(|request| request.hardware)
    }

    /// Read the parameters of an `export` call. Absent parameters are an empty request, which
    /// [`RenderRequest::to_queue_params`] then refuses for naming no output.
    pub fn from_params(params: &Value) -> Result<Self> {
        let obj = Self::fields(params)?;
        if bool_param(&obj, QUEUE, EXPORT)?.unwrap_or(false) {
            return Ok(Self { queue: true, ..Self::default() });
        }
        let text = |key: &str| -> Result<Option<String>> { Ok(string_param(&obj, key, EXPORT)?.map(str::to_string)) };
        let number = |key: &str| number_param(&obj, key, EXPORT);
        Ok(Self {
            comp: CompRef::param(&obj, EXPORT)?,
            output: text("output")?,
            format: text("format")?,
            start: number("start")?,
            end: number("end")?,
            work_area: bool_param(&obj, "workArea", EXPORT)?.unwrap_or(false),
            fps: number("fps")?,
            resolution: Resolution::param(&obj, "resolution")?,
            scale: Resolution::param(&obj, "scale")?,
            quality: text("quality")?,
            channels: text("channels")?,
            jpeg_quality: number("jpegQuality")?,
            bitrate: number("bitrate")?,
            prores: text("prores")?,
            audio: text("audio")?,
            profile: text("profile")?,
            level: text("level")?,
            rate_control: text("rateControl")?,
            video_quality: number("videoQuality")?,
            keyint: number("keyint")?,
            webm_codec: text("webmCodec")?,
            audio_bitrate: number("audioBitrate")?,
            opus_app: text("opusApp")?,
            exr_compression: text("exrCompression")?,
            exr_depth: text("exrDepth")?,
            queue: false,
            hardware: bool_param(&obj, HARDWARE, EXPORT)?.unwrap_or(false),
        })
    }

    /// The parameters of the `renderQueue.add` that queues this request, set as the command line
    /// sets them from its flags, in its order; `None` when the request renders the project's own
    /// queue and adds nothing.
    pub fn to_queue_params(&self) -> Result<Option<Value>> {
        if self.queue {
            return Ok(None);
        }
        let output = self.output.as_deref().ok_or_else(|| CraftError::invalid_params("`export`: `output` is required (or `queue`)"))?;
        let mut params = Map::new();
        let mut set = |key: &str, value: Value| {
            params.insert(key.to_string(), value);
        };
        set("output", Value::from(output));
        if let Some(comp) = &self.comp {
            set("comp", comp.to_value());
        }
        let extension = std::path::Path::new(output).extension().map(|extension| extension.to_string_lossy().into_owned());
        let format = self.format.clone().or(extension);
        if let Some(format) = &format {
            set("format", Value::from(format.as_str()));
        }
        match (self.start, self.end) {
            (None, None) => set("timeSpan", Value::from(if self.work_area { "workArea" } else { "comp" })),
            (start, end) => {
                set("start", Value::from(start.unwrap_or(0.0)));
                if let Some(end) = end {
                    set("end", Value::from(end));
                }
            }
        }
        if let Some(fps) = self.fps {
            set("frameRate", Value::from(fps));
        }
        if let Some(resolution) = self.resolution.as_ref().or(self.scale.as_ref()) {
            set("resolution", resolution.to_value());
        }
        for (key, value) in [("quality", &self.quality), ("channels", &self.channels), ("proresProfile", &self.prores), ("audio", &self.audio)] {
            if let Some(value) = value {
                set(key, Value::from(value.as_str()));
            }
        }
        if let Some(quality) = self.jpeg_quality {
            set("quality", Value::from(quality));
        }
        if let Some(bitrate) = self.bitrate {
            set("bitrate", Value::from(bitrate));
            // A WebM's VP9 encoder aims at a quality and ignores a bitrate it is not told to keep
            // to. Which names mean WebM is `OutputFormat::from_name`'s call, so every name the
            // render queue takes for it, the extension's included, turns the bitrate on.
            if format.as_deref().and_then(OutputFormat::from_name) == Some(OutputFormat::WebM) {
                set("webmBitrate", Value::Bool(true));
            }
        }
        for (key, value) in [
            ("profile", &self.profile),
            ("level", &self.level),
            ("rateControl", &self.rate_control),
            ("webmCodec", &self.webm_codec),
            ("opusApplication", &self.opus_app),
            ("exrCompression", &self.exr_compression),
            ("exrBitDepth", &self.exr_depth),
        ] {
            if let Some(value) = value {
                set(key, Value::from(value.as_str()));
            }
        }
        for (key, value) in [("quality", self.video_quality), ("keyframeInterval", self.keyint), ("audioBitrate", self.audio_bitrate)] {
            if let Some(value) = value {
                set(key, Value::from(value));
            }
        }
        Ok(Some(Value::Object(params)))
    }
}

/// The name a failure of the render is reported under.
const METHOD: &str = "frame";

/// The frame rate the command line divides a frame number by when the comp states none.
const DEFAULT_FRAME_RATE: f64 = 30.0;

/// Render the frame `params` ask for and answer it as `format`: the PNG file `render-frame` writes,
/// or the raw pixels the compositor rendered, with the size check and the premultiplied conversion
/// those take in the core's own [`FrameBytes::new`].
pub(crate) fn frame(backend: &mut Backend, params: &Value, format: PixelFormat) -> Result<FrameBytes> {
    FrameRequest::parse(params)?.frame(backend, format)
}

/// Draw the frame `params` ask for into the preview `surface` (plan §17): the request of a `frame` call,
/// resolved the same way, so a refusal of it is the `frame` call's, and drawn as [`picture`] makes it.
/// `gpu` is the compositor's device when the app holds one.
pub(crate) fn present(backend: &mut Backend, gpu: Option<&Gpu>, params: &Value, surface: &mut Surface) -> Result<()> {
    match picture(backend, gpu, params, surface.device().limits().max_texture_dimension_2d)? {
        Picture::Display(display) => surface.present(&display.texture)?,
        Picture::Pixels(frame) => surface.present_frame(&frame)?,
    };
    Ok(())
}

/// What a preview draws of a frame: the compositor's display frame where it renders on the GPU, which
/// the surface blits without a copy through the CPU, or the CPU's straight pixels, which the core
/// uploads premultiplied, as the blit reads every frame (plan §17).
pub(crate) enum Picture {
    Display(DisplayFrame),
    Pixels(FrameBytes),
}

/// The picture of the frame `params` ask for, the same whichever compositor makes it.
///
/// The comp is rendered with its own alpha, as the display frame keeps it, whatever `transparent` says:
/// the core shows a frame's alpha over black, or, on a surface that composites it, over the app's own
/// views, so a frame composited over the comp's background on the CPU alone would show that colour only
/// when the GPU declined the frame. It is rendered at the comp's own size, whatever `maxSide` or `scale`
/// say, since the blit scales it to the surface, and no larger than `max_side`, the device's texture
/// limit, which a CPU frame is uploaded under: a comp too large for the device is one the GPU declines,
/// and its picture still reaches the surface. The GPU renders it when the project asks for GPU
/// acceleration, as EffectCraft's viewer asks the renderer's own accelerator before it renders a display
/// frame; a project on Mercury Software Only previews from the CPU, as its frames render.
pub(crate) fn picture(backend: &mut Backend, gpu: Option<&Gpu>, params: &Value, max_side: u32) -> Result<Picture> {
    FrameRequest::parse(params)?.resolve(backend)?.picture(backend, gpu, max_side)
}

/// The method a refusal of a preview request's parameters names.
const PREVIEW_REQUEST: &str = "previewRequest";

/// The most pixels one side of a preview's box may be: a texture's side is a `u32`.
const MOST_BOX_SIDE: u64 = u32::MAX as u64;

/// The parameters `previewRequest` takes.
const PREVIEW_REQUEST_FIELDS: [&str; 4] = ["comp", "time", "width", "height"];

/// `previewRequest`: `{comp?, time?, width, height}`, the box a preview fills in pixels, answered as
/// `{request, requestText, contentWidth, contentHeight}` (plan §14).
///
/// `request` is the `frame` request that draws the comp with its own alpha, as a preview shows it,
/// as large as fits inside the box and never above the comp's own pixels, and `contentWidth` and
/// `contentHeight` are the comp's pixel size, which sets the texture or buffer the frame is drawn into
/// before the app scales it. The comp is the request's as a `frame` reads it and the time as given,
/// each left out when the caller left it out, and the size is read through [`comp_figures`], the read a
/// `frame` makes for a scale, so a comp that is not there is refused in the words a `frame` refuses it
/// in. `requestText` is `request` written here as JSON, so every platform answers one text for one
/// request: each SDK's own writer lays out an object's keys and a number in its own way, such as a
/// time under a millisecond, which Kotlin writes with an exponent and Swift without.
pub(crate) fn preview_request(backend: &mut Backend, params: &Value) -> Result<Value> {
    let obj = object_params(params, PREVIEW_REQUEST)?;
    reject_unknown(&obj, PREVIEW_REQUEST, &PREVIEW_REQUEST_FIELDS)?;
    // The time is echoed as the caller wrote it, once it is known to be a number.
    let time = number_param(&obj, "time", PREVIEW_REQUEST)?.and_then(|_| obj.get("time")).cloned();
    let side = |key: &str| {
        let pixels = whole_param(&obj, key, PREVIEW_REQUEST, MOST_BOX_SIDE)?.ok_or_else(|| whole_number_words(key, PREVIEW_REQUEST, MOST_BOX_SIDE))?;
        u32::try_from(pixels).map_err(|_| whole_number_words(key, PREVIEW_REQUEST, MOST_BOX_SIDE))
    };
    let within = (side("width")?, side("height")?);
    let comp = Some(reference(&obj, "comp", PREVIEW_REQUEST)?).filter(|comp| !comp.is_null());
    let info = comp_figures(backend, comp.as_ref())?;
    let figure = |key: &str| {
        info.get(key)
            .and_then(Value::as_u64)
            .and_then(|pixels| u32::try_from(pixels).ok())
            .ok_or_else(|| CraftError::internal(format!("`{PREVIEW_REQUEST}`: the comp's figures hold no `{key}` in pixels")))
    };
    let content = (figure("width")?, figure("height")?);
    let mut request = Map::new();
    request.extend(comp.map(|comp| ("comp".to_string(), comp)));
    request.extend(time.map(|time| ("time".to_string(), time)));
    request.insert("maxSide".to_string(), Value::from(fitted_side(content, within)));
    request.insert("transparent".to_string(), Value::Bool(true));
    let request = Value::Object(request);
    let text = serde_json::to_string(&request)
        .map_err(|error| CraftError::internal(format!("`{PREVIEW_REQUEST}`: the request cannot be written as JSON: {error}")))?;
    Ok(object([
        ("request", request),
        ("requestText", Value::String(text)),
        ("contentWidth", Value::from(content.0)),
        ("contentHeight", Value::from(content.1)),
    ]))
}

/// The longest side, in pixels, of the largest frame of a comp `content` pixels across that fits inside
/// a box `within` pixels across and is no longer on any side than the comp: the comp's own aspect
/// kept, its longest side bounded by whichever of its sides meets the box first.
///
/// A frame is `frame_size` of the comp at the scale a side makes of its longest, which rounds each side
/// to a whole pixel, so the side is worked out from that rounding and then checked against
/// `frame_size` itself, the function every renderer sizes its frame with, and moved by a pixel where
/// the arithmetic and the floating-point scale part. The result is always at least 1.
fn fitted_side(content: (u32, u32), within: (u32, u32)) -> u32 {
    let (width, height) = (content.0.max(1), content.1.max(1));
    let longest = width.max(height);
    let fits = |side: u32| {
        let (frame_width, frame_height) = frame_size(width, height, f64::from(side) / f64::from(longest));
        frame_width <= within.0 && frame_height <= within.1
    };
    // A side of the comp that is `own` pixels across reaches `limit` pixels when `side` pixels of the
    // longest side round to `limit`, which is the largest `side` for which `own * side / longest` is
    // under `limit + 0.5`.
    let reach = |own: u32, limit: u32| ((2 * u128::from(limit) + 1) * u128::from(longest) - 1) / (2 * u128::from(own));
    let first = reach(width, within.0).min(reach(height, within.1)).min(u128::from(longest)).max(1);
    let mut side = u32::try_from(first).unwrap_or(longest);
    while side > 1 && !fits(side) {
        side -= 1;
    }
    while side < longest && fits(side + 1) {
        side += 1;
    }
    side
}

/// The moment a frame is rendered at.
enum Moment {
    /// The active comp's current time.
    Current,
    Seconds(f64),
    /// A frame number, which the comp's frame rate turns into seconds.
    Frame(f64),
}

/// How big a frame is rendered.
enum Size {
    Full,
    MaxSide(f64),
    /// A factor of the comp's longest side.
    Scale(f64),
}

/// The request as the app wrote it, checked for its shape.
pub(crate) struct FrameRequest {
    comp: Option<Value>,
    moment: Moment,
    size: Size,
    transparent: bool,
}

/// The request with every default and relative figure settled: the arguments of one render.
struct Resolved {
    comp: Option<Value>,
    time: Option<f64>,
    max_side: u32,
    transparent: bool,
}

/// The fields a frame request takes.
const FRAME_FIELDS: [&str; 6] = ["comp", "time", "frame", "maxSide", "scale", "transparent"];

impl FrameRequest {
    /// The request as an object of the fields a frame takes, and of those in `also`, which a caller
    /// refuses in words of its own, refused otherwise: what the command line's own parser refuses of
    /// its flags before it reads any of them.
    pub(crate) fn fields<'a>(params: &'a Value, also: &[&str]) -> Result<Cow<'a, Map<String, Value>>> {
        let obj = object_params(params, METHOD)?;
        let known: Vec<&str> = FRAME_FIELDS.iter().chain(also).copied().collect();
        reject_unknown(&obj, METHOD, &known)?;
        Ok(obj)
    }

    pub(crate) fn parse(params: &Value) -> Result<Self> {
        let obj = Self::fields(params, &[])?;
        let moment = match (number_param(&obj, "time", METHOD)?, number_param(&obj, "frame", METHOD)?) {
            (Some(_), Some(_)) => return Err(CraftError::invalid_params("`frame`: give `time` or `frame`, not both")),
            (Some(seconds), None) => Moment::Seconds(seconds),
            (None, Some(number)) => Moment::Frame(number),
            (None, None) => Moment::Current,
        };
        let size = match (number_param(&obj, "maxSide", METHOD)?, number_param(&obj, "scale", METHOD)?) {
            (Some(_), Some(_)) => return Err(CraftError::invalid_params("`frame`: give `maxSide` or `scale`, not both")),
            (Some(side), None) => Size::MaxSide(side),
            (None, Some(factor)) => Size::Scale(factor),
            (None, None) => Size::Full,
        };
        // `render-frame` hands its render `None` for the active comp (`main.rs:797`), so the render is
        // called as it calls it; `get_comp` is given null, as the command line gives it.
        let comp = Some(reference(&obj, "comp", METHOD)?).filter(|comp| !comp.is_null());
        Ok(Self { comp, moment, size, transparent: bool_param(&obj, "transparent", METHOD)?.unwrap_or(false) })
    }

    /// Render the frame asked for and answer it as `format`, the figures that depend on the comp
    /// settled first.
    pub(crate) fn frame(self, backend: &mut Backend, format: PixelFormat) -> Result<FrameBytes> {
        self.resolve(backend)?.render(backend, format)
    }

    /// Settle the figures that depend on the comp, asking the comp for them the way the command line
    /// does: only a frame number or a scale needs the comp's own figures.
    fn resolve(self, backend: &mut Backend) -> Result<Resolved> {
        let Self { comp, moment, size, transparent } = self;
        let needs_comp = matches!(moment, Moment::Frame(_)) || matches!(size, Size::Scale(_));
        let info = if needs_comp { comp_figures(backend, comp.as_ref())? } else { Value::Null };
        let time = match moment {
            Moment::Current => None,
            Moment::Seconds(seconds) => Some(seconds),
            Moment::Frame(number) => Some(seconds_of_frame(number, &info)),
        };
        let max_side = match size {
            Size::Full => 0,
            // `as` saturates, which is the command line's own cast: a figure past `u32` reads as the
            // largest, a negative one or NaN as 0.
            Size::MaxSide(side) => side as u32,
            Size::Scale(factor) => side_of_scale(factor, &info),
        };
        Ok(Resolved { comp, time, max_side, transparent })
    }
}

/// The figures of the comp `comp` names (the active one when none), read as the command line reads them
/// for a frame number or a scale: from `get_comp`, whose refusal of a comp that is not there is the
/// command line's own. The one read every request that must know the comp's size makes.
fn comp_figures(backend: &mut Backend, comp: Option<&Value>) -> Result<Value> {
    tool_json(backend, "get_comp", &object([("comp", comp.cloned().unwrap_or(Value::Null))]))
}

/// The seconds frame `number` falls at in the comp `info` describes (`comp.info`'s document): the
/// number over the comp's frame rate, or over 30 where the document states none, as the command
/// line divides it.
fn seconds_of_frame(number: f64, info: &Value) -> f64 {
    number / info.get("frameRate").and_then(Value::as_f64).unwrap_or(DEFAULT_FRAME_RATE)
}

/// The longest side a `scale` asks of the comp `info` describes: that factor of the comp's longest
/// side, rounded to a whole pixel and at least one, a side the document states none of reading 0, as
/// the command line works it out; `as` saturates, as the command line's cast does.
fn side_of_scale(factor: f64, info: &Value) -> u32 {
    let side = |key: &str| info.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    (side("width").max(side("height")) * factor).round().max(1.0) as u32
}

impl Resolved {
    /// The comp the request names and the moment it asks for, both settled the one way every render and
    /// every preview of this request settles them: the comp through the session, whose refusal of one
    /// that is not there is the command line's own words, and no time meaning the comp's current one.
    fn comp_and_time(&self, backend: &mut Backend) -> Result<(ItemId, Tick)> {
        let comp = self.comp.as_ref();
        let session = backend.session().ok_or_else(|| CraftError::internal("`frame`: the session is not headless"))?;
        let id = match session.resolve_comp(comp) {
            Ok(id) => id,
            Err(unresolved) => return Err(missing_comp(backend, comp, unresolved)),
        };
        Ok((id, self.time.map_or_else(|| session.time(), Tick::from_seconds_f64)))
    }

    /// Render the frame: as a PNG through the backend's own `render_with`, the path of the command
    /// line's `render-frame`, or as raw pixels through the session's `render_rgba8_alpha`, which that
    /// call renders its PNG from. Both resolve the comp and the time the same way.
    fn render(&self, backend: &mut Backend, format: PixelFormat) -> Result<FrameBytes> {
        if format == PixelFormat::Png {
            let rendered = backend.render_with(self.comp.as_ref(), self.time, self.max_side, self.transparent).map_err(render_failed)?;
            return FrameBytes::new(rendered.width, rendered.height, format, rendered.png);
        }
        let (id, at) = self.comp_and_time(backend)?;
        let session = backend.session().ok_or_else(|| CraftError::internal("`frame`: the session is not headless"))?;
        let (width, height, pixels) = session.render_rgba8_alpha(id, at, self.max_side, self.transparent).map_err(|error| render_failed(Error::from(error)))?;
        FrameBytes::new(width, height, format, pixels)
    }

    /// The preview's [`Picture`] of the frame, as [`picture`] says: the display frame the compositor
    /// leaves on the GPU, through the renderer the session's own frames render with, or, where the
    /// project or the app has no GPU for it or the compositor declines the frame, the CPU's.
    fn picture(&self, backend: &mut Backend, gpu: Option<&Gpu>, max_side: u32) -> Result<Picture> {
        let (id, at) = self.comp_and_time(backend)?;
        let session = backend.session().ok_or_else(|| CraftError::internal("`present`: the session is not headless"))?;
        // `Auto` is the backend the session's own frames render with (`render_rgba8_alpha`), under which
        // the renderer offers its accelerator only when the project asks for GPU acceleration.
        let renderer = session.renderer(RenderOpts { backend: effectcraft_render::Backend::Auto, ..RenderOpts::default() });
        if renderer.active_accel().is_some()
            && let Some(display) = gpu.and_then(|gpu| gpu.render_display(&renderer, id, at))
        {
            return Ok(Picture::Display(display));
        }
        let (width, height, pixels) = session.render_rgba8_alpha(id, at, max_side, true).map_err(|error| render_failed(Error::from(error)))?;
        Ok(Picture::Pixels(FrameBytes::new(width, height, PixelFormat::Rgba8Straight, pixels)?))
    }
}

/// Map a failure of the render onto its row, under the method's name.
fn render_failed(error: Error) -> CraftError {
    failure(error, METHOD, &Value::Null, false)
}

/// The refusal of a comp the session could not resolve, in the words the PNG path's render refuses
/// it with, which are the command line's: the automation layer says them only inside its render,
/// whose first step resolves the comp as the session just did, so it refuses this one before it
/// renders anything. Asking it for them keeps one wording of a missing comp for every format,
/// without a copy of the automation layer's sentences here.
fn missing_comp(backend: &mut Backend, comp: Option<&Value>, unresolved: EngineError) -> CraftError {
    match backend.render_with(comp, None, 1, false) {
        Err(refused) => render_failed(refused),
        // A render that found the comp the session did not would mean the two resolve differently;
        // the session's own refusal is then the one that describes what this call tried.
        Ok(_) => render_failed(Error::from(unresolved)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The largest longest side whose frame fits the box, found by trying every side: what `fitted_side`
    /// must answer, however it is worked out.
    fn largest_fit(content: (u32, u32), within: (u32, u32)) -> u32 {
        let longest = content.0.max(content.1);
        (1..=longest)
            .rev()
            .find(|&side| {
                let (width, height) = frame_size(content.0, content.1, f64::from(side) / f64::from(longest));
                width <= within.0 && height <= within.1
            })
            .unwrap_or(1)
    }

    #[test]
    fn a_box_of_the_comps_own_aspect_or_larger_asks_for_the_comps_longest_side_at_most() {
        assert_eq!(fitted_side((1280, 720), (640, 640)), 640, "a square box takes the comp's longer side");
        assert_eq!(fitted_side((1280, 720), (1280, 720)), 1280, "a box of the comp's own size takes the comp's longer side");
        assert_eq!(fitted_side((1280, 720), (4000, 4000)), 1280, "a larger box never asks for more than the comp's pixels");
        assert_eq!(fitted_side((720, 1280), (640, 640)), 640, "a portrait comp is bound by its height");
        assert_eq!(fitted_side((720, 1280), (4000, 4000)), 1280);
        assert_eq!(fitted_side((1280, 720), (500, 300)), 500, "a box of another aspect keeps the comp's aspect inside it");
        assert_eq!(fitted_side((1280, 720), (4000, 100)), 178, "a short box is met by the shorter side, as the frame's rounding allows");
        assert_eq!(fitted_side((1280, 720), (1, 1)), 1, "a box of one pixel is a frame of one");
        assert_eq!(fitted_side((1, 1), (9, 9)), 1);
        assert_eq!(fitted_side((100_000, 1), (1000, 1)), 1000, "a comp far wider than tall");
        assert_eq!(fitted_side((u32::MAX, u32::MAX), (u32::MAX, 1)), 1, "no figure the box or the comp holds overflows");
    }

    #[test]
    fn the_fitted_side_is_the_largest_side_whose_frame_fits_the_box() {
        for width in 1..=13 {
            for height in 1..=13 {
                for box_width in 1..=15 {
                    for box_height in 1..=15 {
                        let (content, within) = ((width, height), (box_width, box_height));
                        assert_eq!(fitted_side(content, within), largest_fit(content, within), "a comp of {content:?} in a box of {within:?}");
                    }
                }
            }
        }
    }

    /// Where the compositor's floating-point scale rounds a side otherwise than exact arithmetic, the side
    /// is the compositor's: the frame it renders is the one that must fit.
    #[test]
    fn the_fit_follows_the_compositors_rounding_where_it_parts_from_exact_arithmetic() {
        let fits = |content: (u32, u32), within: (u32, u32), side: u32| {
            let (width, height) = frame_size(content.0, content.1, f64::from(side) / f64::from(content.0.max(content.1)));
            width <= within.0 && height <= within.1
        };
        // 11 by 22 at a side of 15 is 7.5 pixels wide exactly, which rounds to 8, but the compositor's
        // floating-point scale makes it a hair under 7.5, so it renders 7: a frame of 7 by 15 the box takes.
        let (content, within) = ((11, 22), (7, 15));
        assert_eq!(fitted_side(content, within), 15, "the compositor's 7 by 15 fits");
        assert!(fits(content, within, 15));
        // At these figures the float's error is wider than the distance to a half pixel: a side the exact
        // arithmetic lets through rounds over the box in the compositor, so the side is one less.
        let (content, within) = ((196_327_744, 405_334_033), (189_842_424, 391_944_582));
        assert_eq!(fitted_side(content, within), 391_944_581, "one side less than exact arithmetic gives");
        assert!(fits(content, within, 391_944_581) && !fits(content, within, 391_944_582));
    }

    fn params(request: Value) -> Value {
        RenderRequest::from_params(&request).expect("the request reads").to_queue_params().expect("the request maps").expect("it queues an item")
    }

    #[test]
    fn a_request_with_an_output_is_the_command_lines_defaults() {
        // `--out x.mp4`: the extension names the format and the whole composition is rendered.
        assert_eq!(params(json!({"output": "/c/x.mp4"})), json!({"output": "/c/x.mp4", "format": "mp4", "timeSpan": "comp"}));
        assert_eq!(params(json!({"output": "/c/x", "workArea": true})), json!({"output": "/c/x", "timeSpan": "workArea"}));
        assert_eq!(params(json!({"output": "/c/x.mp4", "format": "h264"}))["format"], "h264", "a named format wins over the extension");
    }

    #[test]
    fn a_start_or_an_end_replaces_the_time_span() {
        assert_eq!(params(json!({"output": "/c/x", "end": 2.0, "workArea": true})), json!({"output": "/c/x", "start": 0.0, "end": 2.0}));
        assert_eq!(params(json!({"output": "/c/x", "start": 1.5})), json!({"output": "/c/x", "start": 1.5}));
    }

    #[test]
    fn each_field_sets_the_parameter_its_flag_sets() {
        let mapped = params(json!({
            "output": "/c/x", "format": "h264", "comp": "Main", "fps": 12, "channels": "rgba", "prores": "hq", "audio": "off",
            "profile": "main10", "level": "4.1", "rateControl": "quality", "webmCodec": "av1", "opusApp": "voice",
            "exrCompression": "zip", "exrDepth": "half", "keyint": 30, "audioBitrate": 96, "bitrate": 800
        }));
        assert_eq!(
            mapped,
            json!({
                "output": "/c/x", "format": "h264", "comp": "Main", "timeSpan": "comp", "frameRate": 12.0, "channels": "rgba",
                "proresProfile": "hq", "audio": "off", "bitrate": 800.0, "profile": "main10", "level": "4.1", "rateControl": "quality",
                "webmCodec": "av1", "opusApplication": "voice", "exrCompression": "zip", "exrBitDepth": "half",
                "keyframeInterval": 30.0, "audioBitrate": 96.0
            })
        );
        assert_eq!(params(json!({"output": "/c/x", "comp": 3}))["comp"], 3);
    }

    #[test]
    fn the_three_quality_flags_share_one_parameter_and_the_last_wins() {
        assert_eq!(params(json!({"output": "/c/x", "quality": "draft"}))["quality"], "draft");
        assert_eq!(params(json!({"output": "/c/x", "quality": "draft", "jpegQuality": 80}))["quality"], 80.0);
        assert_eq!(params(json!({"output": "/c/x", "quality": "draft", "jpegQuality": 80, "videoQuality": 55}))["quality"], 55.0);
    }

    #[test]
    fn a_resolution_is_a_number_when_it_reads_as_one() {
        assert_eq!(params(json!({"output": "/c/x", "resolution": "half"}))["resolution"], "half");
        assert_eq!(params(json!({"output": "/c/x", "resolution": "0.25"}))["resolution"], 0.25);
        assert_eq!(params(json!({"output": "/c/x", "resolution": 0.5}))["resolution"], 0.5);
        assert_eq!(params(json!({"output": "/c/x", "scale": "third"}))["resolution"], "third");
        assert_eq!(params(json!({"output": "/c/x", "scale": "third", "resolution": "half"}))["resolution"], "half");
    }

    #[test]
    fn a_bitrate_targets_the_bitrate_of_a_webm_by_any_name_of_the_format() {
        for format in ["webm", "vp9", "webmvp9opus"] {
            assert_eq!(params(json!({"output": "/c/x", "format": format, "bitrate": 500}))["webmBitrate"], true, "{format}");
        }
        assert_eq!(params(json!({"output": "/c/x.webm", "bitrate": 500}))["webmBitrate"], true, "the format of the extension counts too");
        assert!(params(json!({"output": "/c/x", "format": "h264", "bitrate": 500})).get("webmBitrate").is_none());
        assert!(params(json!({"output": "/c/x", "format": "webm"})).get("webmBitrate").is_none(), "no bitrate, nothing to target");
    }

    #[test]
    fn a_queue_request_adds_nothing_and_leaves_every_other_field_unread() {
        // The command line reads none of its other flags with `--queue`, so it refuses none of them.
        let request = RenderRequest::from_params(&json!({"queue": true, "output": "/c/x", "bitrate": 1, "start": "soon"})).expect("reads");
        assert_eq!(request, RenderRequest { queue: true, ..RenderRequest::default() });
        assert_eq!(request.to_queue_params().expect("maps"), None);
        // An unknown name is refused whatever else is given, `queue` with `hardware` included, as the
        // command line refuses an unknown option before it reads any.
        for request in [json!({"queue": true, "widht": 1}), json!({"queue": true, "hardware": true, "widht": 1})] {
            let error = RenderRequest::from_params(&request).expect_err("an unknown name");
            assert_eq!(error, CraftError::invalid_params("`export`: unknown parameter(s) `widht`"), "{request}");
        }
    }

    #[test]
    fn a_request_without_an_output_or_a_queue_is_refused() {
        for request in [json!({}), Value::Null, json!({"format": "png"}), json!({"queue": false, "format": "png"})] {
            let error = RenderRequest::from_params(&request).and_then(|request| request.to_queue_params()).expect_err("no output");
            assert_eq!(error, CraftError::invalid_params("`export`: `output` is required (or `queue`)"));
        }
    }

    /// Each refusal is in the words every method refuses its parameters in, naming every unknown
    /// name and the name of a value of the wrong kind, as the command line names the flag.
    #[test]
    fn a_bad_parameter_is_refused_by_its_name() {
        for (bad, words) in [
            (json!({"output": "/c/x", "widht": 1, "hieght": 2}), "`export`: unknown parameter(s) `hieght`, `widht`"),
            (json!([1]), "`export`: its parameters must be a JSON object"),
            (json!("/c/x.mp4"), "`export`: its parameters must be a JSON object"),
            (json!({"output": 7}), "`export`: `output` must be a string"),
            (json!({"output": "/c/x", "start": "soon"}), "`export`: `start` must be a number"),
            (json!({"output": "/c/x", "quality": 80}), "`export`: `quality` must be a string"),
            (json!({"output": "/c/x", "workArea": "yes"}), "`export`: `workArea` must be a bool"),
            (json!({"output": "/c/x", "queue": "yes"}), "`export`: `queue` must be a bool"),
            (json!({"output": "/c/x", "comp": 1.5}), "`export`: `comp` must be an id, a name or `-`"),
            (json!({"output": "/c/x", "comp": -1}), "`export`: `comp` must be an id, a name or `-`"),
            (json!({"output": "/c/x", "scale": true}), "`export`: `scale` must be a factor or a preset name"),
        ] {
            assert_eq!(RenderRequest::from_params(&bad), Err(CraftError::invalid_params(words)), "{bad}");
        }
    }

    /// Every field refuses a value of no kind it takes (an array) by its own name and nobody
    /// else's, so a caller is never pointed at the wrong field.
    #[test]
    fn every_parameter_refuses_a_wrong_value_by_its_own_name() {
        for name in FIELDS {
            let bad = Value::Object(Map::from_iter([(name.to_string(), json!([1]))]));
            let refusal = RenderRequest::from_params(&bad).expect_err("an array is no kind of value").to_string();
            assert!(refusal.starts_with(&format!("`export`: `{name}` must be ")), "{name}: {refusal}");
        }
    }

    /// Every name the request accepts is a field it reads: a request carrying any one of them is
    /// not the request that left it out.
    #[test]
    fn every_parameter_it_accepts_is_read() {
        let values = [
            ("comp", json!("Main")),
            ("output", json!("/c/x")),
            ("format", json!("png")),
            ("start", json!(1)),
            ("end", json!(2)),
            ("workArea", json!(true)),
            ("fps", json!(12)),
            ("resolution", json!("half")),
            ("scale", json!(0.5)),
            ("quality", json!("draft")),
            ("channels", json!("rgba")),
            ("jpegQuality", json!(70)),
            ("bitrate", json!(900)),
            ("prores", json!("hq")),
            ("audio", json!("off")),
            ("profile", json!("main")),
            ("level", json!("4.1")),
            ("rateControl", json!("quality")),
            ("videoQuality", json!(60)),
            ("keyint", json!(12)),
            ("webmCodec", json!("av1")),
            ("audioBitrate", json!(96)),
            ("opusApp", json!("voice")),
            ("exrCompression", json!("zip")),
            ("exrDepth", json!("half")),
            ("queue", json!(true)),
            ("hardware", json!(true)),
        ];
        assert_eq!(values.iter().map(|(name, _)| *name).collect::<Vec<_>>(), FIELDS, "a value for each accepted name");
        let left_out = RenderRequest::from_params(&json!({})).expect("reads");
        for (name, value) in values {
            let given = RenderRequest::from_params(&Value::Object(Map::from_iter([(name.to_string(), value)]))).expect("reads");
            assert_ne!(given, left_out, "`{name}` is accepted but never read");
        }
    }

    #[test]
    fn hardware_is_an_option_of_the_request_and_never_a_render_queue_parameter() {
        let request = |extra: Value| {
            let mut request = json!({"output": "/c/x.mp4", "format": "h264"});
            request.as_object_mut().expect("an object").extend(extra.as_object().expect("an object").clone());
            request
        };
        assert!(!RenderRequest::from_params(&request(json!({}))).expect("reads").hardware, "off unless asked for");
        assert!(!RenderRequest::from_params(&request(json!({"hardware": false}))).expect("reads").hardware);
        assert!(RenderRequest::from_params(&request(json!({"hardware": true}))).expect("reads").hardware);
        assert_eq!(params(request(json!({"hardware": true}))), params(request(json!({}))), "the queue item is the same either way");
        assert_eq!(RenderRequest::from_params(&request(json!({"hardware": "yes"}))), Err(CraftError::invalid_params("`export`: `hardware` must be a bool")));
    }

    /// The refusal of a queued render that asks for hardware, in its exact words, from the one reader of
    /// the fields; each field alone, a `false` or a null beside the other, and a value that is no bool
    /// (which the field's own reader answers) are not it.
    #[test]
    fn a_queued_request_that_asks_for_hardware_is_refused_in_one_set_of_words() {
        let words = "`export`: `queue` cannot be set with `hardware`: a queued render encodes through the render queue and its own encoders, never the device's hardware encoder";
        for request in [json!({"queue": true, "hardware": true}), json!({"hardware": true, "queue": true, "output": "/c/x.mp4"})] {
            assert_eq!(RenderRequest::fields(&request).unwrap_err(), CraftError::invalid_params(words));
            assert_eq!(RenderRequest::from_params(&request), Err(CraftError::invalid_params(words)));
        }
        for request in [
            json!({"queue": true}),
            json!({"hardware": true, "output": "/c/x.mp4"}),
            json!({"queue": true, "hardware": false}),
            json!({"queue": false, "hardware": true, "output": "/c/x.mp4"}),
            json!({"queue": true, "hardware": null}),
        ] {
            assert!(RenderRequest::from_params(&request).is_ok(), "{request}");
        }
        assert!(!RenderRequest::from_params(&json!({"queue": true, "hardware": "yes"})).expect("queue leaves the rest unread").hardware);
    }

    #[test]
    fn a_request_chooses_hardware_only_when_it_says_true_and_reads() {
        assert!(RenderRequest::chooses_hardware(&json!({"output": "/c/x.mp4", "hardware": true})));
        assert!(!RenderRequest::chooses_hardware(&json!({"output": "/c/x.mp4", "hardware": false})));
        assert!(!RenderRequest::chooses_hardware(&json!({"output": "/c/x.mp4"})));
        assert!(!RenderRequest::chooses_hardware(&json!({"output": "/c/x.mp4", "hardware": "yes"})), "a value that does not read chooses nothing");
        assert!(!RenderRequest::chooses_hardware(&json!({"output": "/c/x.mp4", "hardware": true, "widht": 1})), "nor does a call that is refused");
        assert!(!RenderRequest::chooses_hardware(&json!({"queue": true, "hardware": true})), "a queued request that asks for hardware is refused");
        assert!(!RenderRequest::chooses_hardware(&json!({"queue": true})), "the project's own queue renders no request");
        assert!(!RenderRequest::chooses_hardware(&Value::Null));
    }

    /// A frame number is seconds at the comp's own rate, and at 30 frames a second where the comp
    /// states none; every comp `comp.info` describes at the pin states one, so only this test can
    /// reach the fallback.
    #[test]
    fn a_frame_number_is_seconds_at_the_comps_rate_or_at_30() {
        assert_eq!(seconds_of_frame(15.0, &json!({"frameRate": 25.0})), 0.6);
        assert_eq!(seconds_of_frame(15.0, &json!({"width": 64})), 0.5);
        assert_eq!(seconds_of_frame(15.0, &Value::Null), 0.5);
    }

    /// A scale is a factor of the longest side, rounded to the nearest pixel, never below one, and
    /// as large as `u32` holds at most.
    #[test]
    fn a_scale_is_the_longest_side_rounded_and_at_least_one_pixel() {
        let (wide, tall) = (json!({"width": 64, "height": 48}), json!({"width": 30, "height": 50}));
        assert_eq!(side_of_scale(0.34, &wide), 22, "21.76 rounds up, where a cut would give 21");
        assert_eq!(side_of_scale(0.3, &wide), 19, "19.2 rounds down");
        assert_eq!(side_of_scale(0.5, &tall), 25, "the height is the longest side of a tall comp");
        assert_eq!(side_of_scale(0.0, &wide), 1);
        assert_eq!(side_of_scale(-2.0, &wide), 1);
        assert_eq!(side_of_scale(0.5, &Value::Null), 1, "a comp of no stated size is a side of 0");
        assert_eq!(side_of_scale(1e12, &wide), u32::MAX, "a side past `u32` is the largest one");
    }
}
