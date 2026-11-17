//! The formats a `frame` call is answered in, and the conversions between them (plan §17, D-10).
//!
//! A frame reaches the core as one of two things: PNG file bytes, which pass through untouched, or
//! straight (un-premultiplied) RGBA bytes, which the app has just rendered. From the straight bytes
//! the core answers either raw format itself: kept as is, or swizzled and premultiplied into BGRA,
//! the layout an iOS `CVPixelBuffer` (`kCVPixelFormatType_32BGRA`, premultiplied first) takes
//! directly. Android takes the straight bytes instead (D-10) and builds its premultiplied `Bitmap`
//! (`ARGB_8888`) from them on its own side (plan §17, §23), so the premultiply here serves the BGRA
//! answer alone.
//!
//! Every raw buffer that arrives is treated as hostile: its length is checked against the size it
//! was given with before one byte is read or written, and a mismatch is a [`CraftError`] naming
//! both, never a panic or a partial conversion (AGENTS.md §3). A coded PNG keeps a byte count of
//! its encoder's choosing, so its length is matched against no size and nothing of it is read.

use crate::Result;
use crate::error::CraftError;

/// The bytes one pixel of a raw frame occupies: four 8-bit channels.
const BYTES_PER_PIXEL: usize = 4;

/// The pixel format a `frame` call asks for and is answered with (plan §14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// PNG file bytes, passed through exactly as the app encoded them.
    Png,
    /// Straight (un-premultiplied) 8-bit RGBA, row-major, top-down, no padding.
    Rgba8Straight,
    /// Premultiplied 8-bit BGRA, row-major, top-down, no padding: the colour of every pixel
    /// multiplied by its own alpha, with the red and blue channels exchanged.
    Bgra8Premultiplied,
}

impl PixelFormat {
    /// Whether this format is raw pixels — four bytes for every pixel of the frame — as opposed
    /// to a coded file, whose byte count belongs to its encoder and tells its size nothing.
    const fn is_raw(&self) -> bool {
        !matches!(self, Self::Png)
    }
}

/// A rendered frame: the size it was rendered at, the format its bytes are in, and the bytes
/// (plan §14). The app crate builds one of these per `frame` call from what its session rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameBytes {
    /// The width the frame was rendered at, in pixels.
    pub width: u32,
    /// The height the frame was rendered at, in pixels.
    pub height: u32,
    /// The format `data` is in.
    pub format: PixelFormat,
    /// The PNG file, or `width * height` pixels of four bytes each in `format`'s channel order.
    pub data: Vec<u8>,
}

impl FrameBytes {
    /// Take the bytes a render of `width` x `height` produced and answer them as `format`:
    /// PNG bytes pass through unchanged, straight RGBA bytes are kept as they are or, for
    /// [`PixelFormat::Bgra8Premultiplied`], swizzled and premultiplied in place (plan §17).
    ///
    /// The bytes of a raw format must be exactly `width * height * 4`, the sender's length matching
    /// the size it names; a frame of no pixels is no frame. Either mismatch answers
    /// [`CraftError::InvalidParams`] naming the sizes, as §14 maps a parameter the core refuses.
    ///
    /// The answer is the very buffer that was handed in: the frame's bytes sit at the address
    /// `data` had. The BGRA conversion rewrites that buffer in place and the pass-through formats
    /// move it, so building the answer copies the frame to no second buffer, whatever the format.
    /// That is this call's promise, not the native boundary's: a Kotlin `ByteArray` lives on the
    /// JVM's heap, so the bytes reach Kotlin as a copy whatever is done here.
    pub fn new(width: u32, height: u32, format: PixelFormat, mut data: Vec<u8>) -> Result<Self> {
        check_frame_size(width, height)?;
        if format.is_raw() {
            check_raw_length(width, height, &data)?;
        }
        if format == PixelFormat::Bgra8Premultiplied {
            swizzle_and_premultiply_to_bgra(&mut data)?;
        }
        Ok(Self { width, height, format, data })
    }
}

/// Premultiply one 8-bit channel by an 8-bit alpha: the nearest 8-bit value to `value * alpha / 255`.
///
/// This is the formula Apple documents for its own premultiply of 8-bit pixels
/// (`vImagePremultiplyData_RGBA8888`: `(src * alpha + 127) / 255`), so the bytes a `CVPixelBuffer`
/// takes as [`PixelFormat::Bgra8Premultiplied`] are the ones Apple's conversion would make. Its
/// rounding is also EffectCraft's, which writes premultiplied colour to 8 bits as
/// `(v * 255.0 + 0.5) as u8`, rounding half up: for the same straight value and alpha the two give
/// the same byte, because a tie would need `value * alpha` at a half-multiple of 255 and
/// `255k + 127.5` is never a whole number. That is no promise of EffectCraft's own premultiplied
/// bytes for a rendered pixel: straight bytes have already been rounded once, colour and alpha
/// apart, so premultiplying them again can land one away from what the compositor writes from its
/// floating-point pixel. Alpha 0 gives all-zero colour, alpha 255 leaves the colour unchanged, and
/// no colour byte ever exceeds its alpha.
#[must_use]
pub const fn premultiply_channel(value: u8, alpha: u8) -> u8 {
    // The product fits u32 (at most 255 * 255 + 127), so no wider type and no overflow check:
    // this is the one arithmetic this module runs per channel, and it is total.
    ((value as u32 * alpha as u32 + 127) / 255) as u8
}

/// Turn a straight RGBA buffer into premultiplied BGRA in place: every pixel's colour multiplied
/// by its own alpha, red and blue exchanged, alpha itself kept (plan §17). In place is why a 4K
/// frame converts without one further allocation: the buffer it arrives in is the only buffer.
///
/// A length that is not a whole number of pixels is refused naming the leftover bytes, and the
/// buffer is left exactly as it came: a partial last pixel would otherwise convert to bytes of its
/// own, quietly mis-sized output. A frame built through [`FrameBytes::new`] never brings one: its
/// length is already the exact size.
pub fn swizzle_and_premultiply_to_bgra(rgba: &mut [u8]) -> Result<()> {
    for [red, green, blue, alpha] in whole_pixels(rgba)? {
        // Every output byte reads its inputs before any of them is written, so one pass over the
        // buffer converts a pixel to itself and never to one of its own already-written bytes.
        let (straight_red, straight_green, straight_blue, straight_alpha) = (*red, *green, *blue, *alpha);
        *red = premultiply_channel(straight_blue, straight_alpha);
        *green = premultiply_channel(straight_green, straight_alpha);
        *blue = premultiply_channel(straight_red, straight_alpha);
    }
    Ok(())
}

/// Premultiply a straight RGBA buffer in place, keeping its order: the premultiplied `Rgba8Unorm` a
/// display frame is (plan §17), which the preview uploads a CPU frame as before the blit pass reads
/// it. A length that is not a whole number of pixels is refused as
/// [`swizzle_and_premultiply_to_bgra`] refuses it.
pub(crate) fn premultiply_rgba(rgba: &mut [u8]) -> Result<()> {
    for [red, green, blue, alpha] in whole_pixels(rgba)? {
        *red = premultiply_channel(*red, *alpha);
        *green = premultiply_channel(*green, *alpha);
        *blue = premultiply_channel(*blue, *alpha);
    }
    Ok(())
}

/// The buffer's pixels, or the refusal naming its leftover bytes. The leftover is judged before any
/// pixel is handed out, so a refused buffer is never returned half converted.
fn whole_pixels(rgba: &mut [u8]) -> Result<&mut [[u8; BYTES_PER_PIXEL]]> {
    let length = rgba.len();
    let (pixels, leftover) = rgba.as_chunks_mut::<BYTES_PER_PIXEL>();
    if !leftover.is_empty() {
        return Err(CraftError::invalid_params(format!(
            "frame buffer of {length} bytes ends with {}/{BYTES_PER_PIXEL} of a pixel, which cannot be converted",
            leftover.len()
        )));
    }
    Ok(pixels)
}

/// Refuse a frame of no pixels: both sides are call parameters, and a frame with a side of zero
/// names no pixels for any buffer to fill (plan §14's parameter row).
fn check_frame_size(width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(CraftError::invalid_params(format!("frame size {width}x{height} is not a frame: each side must be at least one pixel")));
    }
    Ok(())
}

/// Refuse a raw buffer whose length is not the exact size it was given with. The product is taken
/// in u128, where it is total: two u32 sides of four-byte pixels need up to 66 bits, past what u64
/// holds, and a product that wrapped would answer a huge size with a length a small buffer matches.
/// No buffer of the size named is allocated to answer it: only the length already in hand is compared.
pub(crate) fn check_raw_length(width: u32, height: u32, data: &[u8]) -> Result<()> {
    // A buffer's length and the pixel size are `usize`, which no target makes wider than 64 bits, so
    // widening them to u128 is exact.
    let expected = u128::from(width) * u128::from(height) * BYTES_PER_PIXEL as u128;
    let actual = data.len() as u128;
    if actual != expected {
        return Err(CraftError::invalid_params(format!(
            "frame {width}x{height} needs {expected} raw bytes (width x height x {BYTES_PER_PIXEL}), the buffer holds {actual}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One pixel's four straight-RGBA bytes, assembled without indexing to match the module's rule.
    fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> [u8; 4] {
        [red, green, blue, alpha]
    }

    /// A one-pixel frame in the raw format given, built from one straight RGBA pixel.
    fn one_pixel_frame(format: PixelFormat, pixel: [u8; 4]) -> Result<FrameBytes> {
        FrameBytes::new(1, 1, format, pixel.to_vec())
    }

    /// The byte EffectCraft's rounding writes for this straight value and alpha premultiplied in
    /// floating point, an independent route to the one [`premultiply_channel`] takes in integers:
    /// the value and alpha as fractions, multiplied, and written as the compositor writes
    /// premultiplied colour, `(v.clamp(0.0, a) * 255.0 + 0.5) as u8`.
    fn reference_premultiply(value: u8, alpha: u8) -> u8 {
        let alpha_fraction = f32::from(alpha) / 255.0;
        let premultiplied = f32::from(value) / 255.0 * alpha_fraction;
        (premultiplied.clamp(0.0, alpha_fraction) * 255.0 + 0.5) as u8
    }

    /// The premultiplied BGRA pixel the reference makes of one straight RGBA pixel.
    fn reference_bgra([red, green, blue, alpha]: [u8; 4]) -> [u8; 4] {
        [reference_premultiply(blue, alpha), reference_premultiply(green, alpha), reference_premultiply(red, alpha), alpha]
    }

    #[test]
    fn png_bytes_pass_through_unchanged() {
        let png: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 1, 2, 3, 4, 5];
        let frame = FrameBytes::new(1920, 1080, PixelFormat::Png, png.clone()).expect("PNG passes through");
        assert_eq!(frame.data, png);
        assert_eq!((frame.width, frame.height), (1920, 1080));
        assert_eq!(frame.format, PixelFormat::Png);
    }

    #[test]
    fn png_length_is_its_own_and_not_the_size() {
        // A coded PNG has a byte count of its encoder's choosing; the raw length rule is the raw
        // formats' only, so even a "wrong" PNG length for the size passes through.
        let frame = FrameBytes::new(2, 2, PixelFormat::Png, vec![0x89, b'P', b'N', b'G']).expect("PNG size is metadata");
        assert_eq!(frame.data.len(), 4);
    }

    #[test]
    fn straight_rgba_is_kept_as_is() {
        let pixels = [rgba(255, 0, 128, 1), rgba(3, 4, 5, 254), rgba(6, 7, 8, 0), rgba(9, 10, 11, 128)];
        let straight: Vec<u8> = pixels.iter().flatten().copied().collect();
        let frame = FrameBytes::new(4, 1, PixelFormat::Rgba8Straight, straight.clone()).expect("straight RGBA is kept");
        assert_eq!(frame.data, straight);
    }

    #[test]
    fn every_format_answers_in_the_buffer_it_was_handed() {
        // The contract's buffer claim pinned without a clock: every format answers at the address
        // it was handed, so no format copies the frame or makes a second buffer for it. A
        // two-pixel frame stands for every size: a frame's size decides only whether it is refused
        // and how many pixels the conversion visits, never which buffer the answer is in. Only the
        // address is asserted: capacity is the allocator's detail, and a copy can hand back a
        // matching capacity at a different address.
        for format in [PixelFormat::Png, PixelFormat::Rgba8Straight, PixelFormat::Bgra8Premultiplied] {
            let straight: Vec<u8> = [rgba(200, 100, 50, 128), rgba(1, 2, 3, 4)].into_iter().flatten().collect();
            let address = straight.as_ptr();
            let frame = FrameBytes::new(2, 1, format, straight).expect("two whole pixels");
            assert_eq!(frame.data.as_ptr(), address, "{format:?} answered in another buffer");
        }
    }

    #[test]
    fn premultiply_of_every_value_against_every_alpha_matches_the_reference() {
        // All 65,536 (value, alpha) pairs against the independent floating-point route: stronger
        // than a per-channel sweep at a fixed alpha, and it proves no pair rounds the other way.
        for alpha in 0_u8..=255 {
            for value in 0_u8..=255 {
                assert_eq!(premultiply_channel(value, alpha), reference_premultiply(value, alpha), "value {value} at alpha {alpha}");
            }
        }
    }

    #[test]
    fn every_channel_sweeps_at_each_interesting_alpha() {
        // Every 8-bit value of one channel at a fixed alpha, the other two at zero, through the whole
        // conversion: each lands premultiplied in the BGRA slot the swizzle gives it, alpha kept.
        for alpha in [0_u8, 1, 128, 254, 255] {
            for channel in 0..3 {
                let straight: Vec<[u8; 4]> = (0_u8..=255)
                    .map(|value| match channel {
                        0 => rgba(value, 0, 0, alpha),
                        1 => rgba(0, value, 0, alpha),
                        _ => rgba(0, 0, value, alpha),
                    })
                    .collect();
                let mut buffer: Vec<u8> = straight.iter().flatten().copied().collect();
                swizzle_and_premultiply_to_bgra(&mut buffer).expect("a whole number of pixels converts");
                let (converted, leftover) = buffer.as_chunks::<4>();
                assert_eq!((converted.len(), leftover.len()), (256, 0));
                for (pixel, bgra) in straight.iter().zip(converted) {
                    assert_eq!(*bgra, reference_bgra(*pixel), "channel {channel} of {pixel:?}");
                }
            }
        }
    }

    #[test]
    fn known_pixels_premultiply_and_swizzle() {
        // Hand-worked against (c * a + 127) / 255, the fractions floored by hand:
        //   (255,255,255,128): (255*128+127)/255 = 32767/255 = 128 each
        //   (1,127,128,1):     128/255 = 0, 254/255 = 0, 255/255 = 1
        //   (64,128,200,128):  25727/255 = 100, 16511/255 = 64, 8319/255 = 32
        //   (128,64,32,254):   (32*254+127)/255 = 8255/255 = 32, (64*254+127)/255 = 16383/255 = 64, (128*254+127)/255 = 32639/255 = 127
        let source = [rgba(255, 255, 255, 128), rgba(1, 127, 128, 1), rgba(64, 128, 200, 128), rgba(128, 64, 32, 254)];
        let expected: Vec<u8> = [[128, 128, 128, 128], [1, 0, 0, 1], [100, 64, 32, 128], [32, 64, 127, 254]].into_iter().flatten().collect();
        let straight: Vec<u8> = source.into_iter().flatten().collect();
        let frame = FrameBytes::new(4, 1, PixelFormat::Bgra8Premultiplied, straight).expect("four whole pixels convert");
        assert_eq!(frame.data, expected);
    }

    #[test]
    fn alpha_zero_gives_all_zero_colour() {
        for value in 0_u8..=255 {
            let frame = one_pixel_frame(PixelFormat::Bgra8Premultiplied, rgba(value, value, value, 0)).expect("one whole pixel");
            let bgra: [u8; 4] = frame.data.clone().try_into().expect("a 1x1 frame is one pixel");
            assert_eq!(bgra, [0, 0, 0, 0], "colour survived at alpha 0: {bgra:?}");
        }
    }

    #[test]
    fn alpha_255_leaves_colour_unchanged_apart_from_the_swizzle() {
        for value in 0_u8..=255 {
            let frame = one_pixel_frame(PixelFormat::Bgra8Premultiplied, rgba(value, 7, value, 255)).expect("one whole pixel");
            let bgra: [u8; 4] = frame.data.clone().try_into().expect("a 1x1 frame is one pixel");
            assert_eq!(bgra, [value, 7, value, 255], "colour changed at alpha 255: {bgra:?}");
        }
    }

    #[test]
    fn premultiplied_colour_never_exceeds_its_alpha() {
        // What a premultiplied CVPixelBuffer consumer relies on, for every value at every alpha: a
        // channel's result depends on its own value and the alpha alone, so that covers every
        // straight pixel.
        for alpha in 0_u8..=255 {
            for value in 0_u8..=255 {
                let frame = one_pixel_frame(PixelFormat::Bgra8Premultiplied, rgba(value, value, value, alpha)).expect("one whole pixel");
                assert!(frame.data.iter().take(3).all(|colour| *colour <= alpha), "{value} at alpha {alpha} exceeded it: {:?}", frame.data);
            }
        }
    }

    #[test]
    fn two_conversions_at_full_alpha_return_the_original_bytes() {
        // Where a round trip applies: at alpha 255 premultiplication is the identity and the
        // swizzle is its own inverse, so converting the BGRA bytes again restores the RGBA bytes.
        let straight: Vec<u8> =
            [(9_u8, 1, 200_u8, 255_u8), (250, 3, 4, 255), (0, 255, 128, 255)].into_iter().flat_map(|(r, g, b, a)| rgba(r, g, b, a)).collect();
        let once = FrameBytes::new(3, 1, PixelFormat::Bgra8Premultiplied, straight.clone()).expect("whole pixels");
        let twice = FrameBytes::new(3, 1, PixelFormat::Bgra8Premultiplied, once.data).expect("whole pixels again");
        assert_eq!(twice.data, straight);
    }

    #[test]
    fn a_truncated_buffer_names_its_leftover_bytes_and_is_left_as_it_came() {
        // The leftover path of the conversion itself: one whole pixel, then three bytes of a
        // second that needs a fourth. The whole pixel ahead of the leftover must not be converted
        // either.
        let straight = vec![10_u8, 20, 30, 255, 1, 2, 3];
        let mut buffer = straight.clone();
        let error = swizzle_and_premultiply_to_bgra(&mut buffer).expect_err("three leftover bytes are refused");
        assert!(matches!(error, CraftError::InvalidParams { .. }), "{error:?}");
        assert_eq!(error.to_string(), "frame buffer of 7 bytes ends with 3/4 of a pixel, which cannot be converted");
        assert_eq!(buffer, straight, "a refused buffer was converted in part");
    }

    #[test]
    fn a_buffer_too_short_for_its_size_names_both_sizes() {
        let error = FrameBytes::new(2, 3, PixelFormat::Rgba8Straight, vec![0_u8; 20]).expect_err("20 bytes are not 2x3 RGBA");
        assert!(matches!(error, CraftError::InvalidParams { .. }), "{error:?}");
        assert_eq!(error.to_string(), "frame 2x3 needs 24 raw bytes (width x height x 4), the buffer holds 20");
    }

    #[test]
    fn a_buffer_too_long_for_its_size_names_both_sizes() {
        let error = FrameBytes::new(1, 1, PixelFormat::Bgra8Premultiplied, vec![0_u8; 5]).expect_err("5 bytes are not one pixel");
        assert!(matches!(error, CraftError::InvalidParams { .. }), "{error:?}");
        assert_eq!(error.to_string(), "frame 1x1 needs 4 raw bytes (width x height x 4), the buffer holds 5");
    }

    #[test]
    fn a_zero_side_is_no_frame() {
        // Each raw buffer is empty, the length a zero side names, so the zero rule alone refuses it.
        for (width, height) in [(0_u32, 2), (2, 0), (0, 0)] {
            for (format, data) in
                [(PixelFormat::Rgba8Straight, Vec::new()), (PixelFormat::Bgra8Premultiplied, Vec::new()), (PixelFormat::Png, vec![0x89, b'P', b'N', b'G'])]
            {
                let error = FrameBytes::new(width, height, format, data).expect_err("a side of zero is refused");
                assert!(matches!(error, CraftError::InvalidParams { .. }), "{format:?}: {error:?}");
                assert_eq!(error.to_string(), format!("frame size {width}x{height} is not a frame: each side must be at least one pixel"));
            }
        }
    }

    #[test]
    fn a_geometry_too_large_for_any_buffer_is_refused_not_wrapped() {
        // 65536 x 65536 is 2^34 bytes, past what a u32 can count; 2^31 x 2^31 is exactly 2^64
        // bytes, which a u64 product wraps to 0, the length of the empty buffer each is given;
        // u32::MAX on both sides needs 66 bits. Each must name its true size and refuse the buffer.
        for (side, needed) in [(65536_u32, "17179869184"), (1 << 31, "18446744073709551616"), (u32::MAX, "73786976260478468100")] {
            for format in [PixelFormat::Rgba8Straight, PixelFormat::Bgra8Premultiplied] {
                let error = FrameBytes::new(side, side, format, Vec::new()).expect_err("no empty buffer holds such a frame");
                assert!(matches!(error, CraftError::InvalidParams { .. }), "{format:?}: {error:?}");
                assert_eq!(error.to_string(), format!("frame {side}x{side} needs {needed} raw bytes (width x height x 4), the buffer holds 0"));
            }
        }
    }

    #[test]
    fn an_empty_buffer_for_a_real_size_is_refused() {
        let error = FrameBytes::new(1920, 1080, PixelFormat::Bgra8Premultiplied, Vec::new()).expect_err("no bytes for a 1920x1080 frame");
        assert!(matches!(error, CraftError::InvalidParams { .. }), "{error:?}");
        assert_eq!(error.to_string(), "frame 1920x1080 needs 8294400 raw bytes (width x height x 4), the buffer holds 0");
    }
}
