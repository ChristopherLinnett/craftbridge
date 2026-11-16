//! `FrameRequest`: what a `frame` call asks for, resolved the way the command line's `render-frame`
//! resolves its flags, and the render that answers it (plan §14, §17, §19).
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

use craftbridge_core::{CraftError, FrameBytes, PixelFormat, Result};
use effectcraft_automation::{Backend, Error};
use effectcraft_engine::EngineError;
use effectcraft_time::Tick;
use serde_json::Value;

use crate::methods::{bool_param, failure, number_param, object, object_params, reject_unknown, tool_json};
use crate::reference::reference;

/// The name a failure of the render is reported under.
const METHOD: &str = "frame";

/// The frame rate the command line divides a frame number by when the comp states none.
const DEFAULT_FRAME_RATE: f64 = 30.0;

/// Render the frame `params` ask for and answer it as `format`: the PNG file `render-frame` writes,
/// or the raw pixels the compositor rendered, with the size check and the premultiplied conversion
/// those take in the core's own [`FrameBytes::new`].
pub(crate) fn frame(backend: &mut Backend, params: &Value, format: PixelFormat) -> Result<FrameBytes> {
    let request = FrameRequest::parse(params)?.resolve(backend)?;
    request.render(backend, format)
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
struct FrameRequest {
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

impl FrameRequest {
    fn parse(params: &Value) -> Result<Self> {
        let obj = object_params(params, METHOD)?;
        reject_unknown(&obj, METHOD, &["comp", "time", "frame", "maxSide", "scale", "transparent"])?;
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

    /// Settle the figures that depend on the comp, asking the comp for them the way the command line
    /// does: only a frame number or a scale needs the comp's own figures.
    fn resolve(self, backend: &mut Backend) -> Result<Resolved> {
        let Self { comp, moment, size, transparent } = self;
        let needs_comp = matches!(moment, Moment::Frame(_)) || matches!(size, Size::Scale(_));
        let info = if needs_comp { tool_json(backend, "get_comp", &object([("comp", comp.clone().unwrap_or(Value::Null))]))? } else { Value::Null };
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
    /// Render the frame: as a PNG through the backend's own `render_with`, the path of the command
    /// line's `render-frame`, or as raw pixels through the session's `render_rgba8_alpha`, which that
    /// call renders its PNG from. Both resolve the comp and the time the same way.
    fn render(&self, backend: &mut Backend, format: PixelFormat) -> Result<FrameBytes> {
        let Self { comp, time, max_side, transparent } = self;
        let comp = comp.as_ref();
        if format == PixelFormat::Png {
            let rendered = backend.render_with(comp, *time, *max_side, *transparent).map_err(render_failed)?;
            return FrameBytes::new(rendered.width, rendered.height, format, rendered.png);
        }
        let session = backend.session().ok_or_else(|| CraftError::internal("`frame`: the session is not headless"))?;
        let id = match session.resolve_comp(comp) {
            Ok(id) => id,
            Err(unresolved) => return Err(missing_comp(backend, comp, unresolved)),
        };
        let at = time.map_or_else(|| session.time(), Tick::from_seconds_f64);
        let (width, height, pixels) = session.render_rgba8_alpha(id, at, *max_side, *transparent).map_err(|error| render_failed(Error::from(error)))?;
        FrameBytes::new(width, height, format, pixels)
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
