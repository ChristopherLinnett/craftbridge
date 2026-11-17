//! What the native boundary can carry, and the one check every answer that grows with what a call asks
//! for goes through: a frame, a request's answer and an MCP reply line. An error's text and the recovery
//! report, which the journal's own size bound keeps far below these limits, cross unchecked.
//!
//! UniFFI writes a length into the buffer it hands over as an `i32` with `i32::try_from(len).unwrap()`
//! (a `Vec<u8>`'s item count, a `String`'s byte count), so a longer value panics inside UniFFI, past
//! every guard of this crate, and the foreign caller gets no `CraftError`. Kotlin reads a buffer
//! through one `ByteBuffer`, which holds at most `i32::MAX` bytes whole, so what a buffer holds beside
//! the value counts against that limit too. Nothing here allocates a value to test it: the check reads
//! a length.

use crate::error::CraftError;

/// What the rest of a serialised [`crate::FrameBytes`] takes beside its `data`: the width, the height,
/// the format's tag and the vector's item count, four bytes each.
const RECORD_FIELD_BYTES: usize = 16;

/// What an `Option<String>` takes beside its text: the tag byte and the text's byte count.
const OPTION_TEXT_FIELD_BYTES: usize = 5;

/// The most bytes of `data` a frame may hold and still cross the boundary. A frame of about 23170 x
/// 23170 RGBA pixels reaches it on a CPU render.
pub const MAX_FRAME_BYTES: usize = i32::MAX as usize - RECORD_FIELD_BYTES;

/// The most bytes a `request` or `request_with_progress` answer may hold: a `String` crosses as its
/// bytes, whole.
pub const MAX_ANSWER_BYTES: usize = i32::MAX as usize;

/// The most bytes an `mcp` reply line may hold: it crosses as an `Option<String>`.
pub const MAX_MCP_LINE_BYTES: usize = i32::MAX as usize - OPTION_TEXT_FIELD_BYTES;

/// A kind of value that crosses the boundary on the way out of Rust, with the words that refuse it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Crossing {
    Frame,
    Answer,
    McpLine,
}

impl Crossing {
    /// What the refusal calls the value.
    fn noun(self) -> &'static str {
        match self {
            Self::Frame => "the frame",
            Self::Answer => "the answer",
            Self::McpLine => "the MCP reply line",
        }
    }

    /// What the caller can change to get a value the boundary carries.
    fn advice(self) -> &'static str {
        match self {
            Self::Frame => "ask for a smaller one (maxSide or scale)",
            Self::Answer | Self::McpLine => "ask for less of it",
        }
    }
}

/// How many bytes of each kind of value an engine hands over. A value so a test can lower one.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub(crate) frame: usize,
    pub(crate) answer: usize,
    pub(crate) mcp_line: usize,
}

impl Limits {
    /// What the boundary itself carries.
    pub(crate) const BOUNDARY: Self = Self { frame: MAX_FRAME_BYTES, answer: MAX_ANSWER_BYTES, mcp_line: MAX_MCP_LINE_BYTES };

    fn of(self, crossing: Crossing) -> usize {
        match crossing {
            Crossing::Frame => self.frame,
            Crossing::Answer => self.answer,
            Crossing::McpLine => self.mcp_line,
        }
    }
}

/// Refuse a value of `len` bytes when `limits` carries fewer of its kind, before UniFFI lowers it.
///
/// The caller can ask again for less, so the answer is the one for parameters the core refuses.
pub(crate) fn check_len(crossing: Crossing, len: usize, limits: Limits) -> Result<(), CraftError> {
    let limit = limits.of(crossing);
    if len > limit {
        return Err(CraftError::invalid_params(format!(
            "{} is {len} bytes, more than the {limit} the native boundary carries: {}",
            crossing.noun(),
            crossing.advice()
        )));
    }
    Ok(())
}

/// `value`, which holds `len` bytes, when the boundary carries it, or the refusal.
pub(crate) fn carry<T>(crossing: Crossing, value: T, len: usize, limits: Limits) -> Result<T, CraftError> {
    check_len(crossing, len, limits)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(crossing: Crossing, len: usize) -> String {
        match check_len(crossing, len, Limits::BOUNDARY) {
            Err(CraftError::InvalidParams { message }) => message,
            other => panic!("{crossing:?} of {len} bytes: {other:?}"),
        }
    }

    #[test]
    fn the_limits_are_what_uniffi_and_kotlin_can_hold_beside_each_value() {
        assert_eq!(MAX_FRAME_BYTES, 2_147_483_631);
        assert_eq!(MAX_ANSWER_BYTES, 2_147_483_647);
        assert_eq!(MAX_MCP_LINE_BYTES, 2_147_483_642);
    }

    #[test]
    fn a_value_beyond_its_limit_is_refused_without_building_one() {
        // These lengths are only numbers: no value of this size is allocated.
        for len in [i32::MAX as usize, i32::MAX as usize + 1, usize::MAX] {
            let message = refusal(Crossing::Frame, len);
            assert_eq!(
                message,
                format!("the frame is {len} bytes, more than the 2147483631 the native boundary carries: ask for a smaller one (maxSide or scale)")
            );
        }
        for len in [i32::MAX as usize + 1, usize::MAX] {
            assert_eq!(
                refusal(Crossing::Answer, len),
                format!("the answer is {len} bytes, more than the 2147483647 the native boundary carries: ask for less of it")
            );
        }
        for len in [i32::MAX as usize - 4, i32::MAX as usize, usize::MAX] {
            assert_eq!(
                refusal(Crossing::McpLine, len),
                format!("the MCP reply line is {len} bytes, more than the 2147483642 the native boundary carries: ask for less of it")
            );
        }
    }

    #[test]
    fn the_largest_value_of_each_kind_is_carried_and_one_more_byte_is_not() {
        for (crossing, most) in [(Crossing::Frame, MAX_FRAME_BYTES), (Crossing::Answer, MAX_ANSWER_BYTES), (Crossing::McpLine, MAX_MCP_LINE_BYTES)] {
            assert_eq!(check_len(crossing, most, Limits::BOUNDARY), Ok(()), "{crossing:?}");
            assert_eq!(check_len(crossing, 0, Limits::BOUNDARY), Ok(()), "{crossing:?}");
            assert!(refusal(crossing, most + 1).contains(&format!("{} bytes", most + 1)), "{crossing:?}");
        }
    }

    #[test]
    fn a_value_that_fits_is_carried_whole() {
        assert_eq!(carry(Crossing::Answer, "text".to_string(), 4, Limits::BOUNDARY), Ok("text".to_string()));
    }
}
