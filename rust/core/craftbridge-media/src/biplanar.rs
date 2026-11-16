//! A biplanar 4:2:0 picture as a decoder's surface holds it, read into the planar frames FilmCraft's
//! software decoders produce: tight `Yuv8` planes for 8-bit pictures and `Yuv16` planes with the samples
//! in the low bits for 10-bit ones. The surface is a luma plane and a plane of interleaved Cb, Cr pairs, a
//! row each `stride` bytes apart, and 10-bit samples sit in the high bits of 16-bit words.
//!
//! Safe code, so every target tests it; the decoder that reads a surface into these slices is the
//! platform module's.

use std::sync::Arc;

use filmcraft_frame::{Chroma, PixelData, pool};

/// One plane of a surface.
#[derive(Clone, Copy)]
pub(crate) struct Plane<'a> {
    pub(crate) data: &'a [u8],
    /// Bytes from the start of one row to the start of the next, padding included.
    pub(crate) stride: usize,
}

impl<'a> Plane<'a> {
    /// `count` rows from row `y`, each `len` bytes from byte `x`; `None` unless all of them lie in the
    /// plane and inside their own row's bytes, so a hostile size never reads, or allocates for, more than
    /// the plane holds.
    fn rows(&self, y: usize, x: usize, len: usize, count: usize) -> Option<impl Iterator<Item = &'a [u8]> + use<'a>> {
        // A zero stride would make `chunks` panic, even for rows of no bytes.
        if self.stride == 0 || x.checked_add(len)? > self.stride {
            return None;
        }
        let start = y.checked_mul(self.stride)?.checked_add(x)?;
        let last_end = y.checked_add(count.checked_sub(1)?)?.checked_mul(self.stride)?.checked_add(x)?.checked_add(len)?;
        if last_end > self.data.len() {
            return None;
        }
        let (stride, data) = (self.stride, self.data.get(start..)?);
        Some(data.chunks(stride).take(count).filter_map(move |row| row.get(..len)))
    }
}

/// The part of the surface that is the picture, in luma samples: where its top-left corner is, and its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Crop {
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) width: usize,
    pub(crate) height: usize,
}

/// Significant bits per sample the surface carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Depth {
    Eight,
    /// P010: 10 bits in the high bits of each 16-bit word.
    Ten,
}

impl Depth {
    pub(crate) fn bits(self) -> u32 {
        match self {
            Self::Eight => 8,
            Self::Ten => 10,
        }
    }
}

const SHORT: &str = "the decoded picture's planes are smaller than the picture";

/// The picture at `crop` in `luma` and `chroma`, as the planar pixels FilmCraft's software decoder
/// produces for it: planes of exactly the picture's size (the chroma planes rounded up for an odd one), and
/// the 10-bit samples shifted down from the high bits. An error when the planes hold less than the
/// picture, or `crop` starts at an odd sample, which 4:2:0 chroma cannot start at.
pub(crate) fn to_pixels(luma: Plane<'_>, chroma: Plane<'_>, crop: Crop, depth: Depth) -> Result<PixelData, &'static str> {
    let Crop { x, y, width, height } = crop;
    if width == 0 || height == 0 {
        return Err("the decoded picture is empty");
    }
    if x % 2 != 0 || y % 2 != 0 {
        return Err("the decoded picture starts at an odd sample");
    }
    let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
    let bytes = match depth {
        Depth::Eight => 1,
        Depth::Ten => 2,
    };
    // The bytes of `samples` samples of `parts` values each: a chroma sample is a Cb, Cr pair.
    let scale = |samples: usize, parts: usize| samples.checked_mul(parts).and_then(|values| values.checked_mul(bytes)).ok_or(SHORT);
    // Every row is checked to lie in its plane before anything is allocated for it.
    let luma_rows = luma.rows(y, scale(x, 1)?, scale(width, 1)?, height).ok_or(SHORT)?;
    let chroma_rows = chroma.rows(y / 2, scale(x / 2, 2)?, scale(chroma_width, 2)?, chroma_height).ok_or(SHORT)?;
    let samples = chroma_width.saturating_mul(chroma_height);
    Ok(match depth {
        Depth::Eight => {
            let mut luma_plane = pool::take_u8(width.saturating_mul(height));
            luma_rows.for_each(|row| luma_plane.extend_from_slice(row));
            let (mut cb, mut cr) = (pool::take_u8(samples), pool::take_u8(samples));
            for &[b, r] in chroma_rows.flat_map(|row| row.as_chunks::<2>().0) {
                cb.push(b);
                cr.push(r);
            }
            PixelData::Yuv8 { planes: [Arc::new(luma_plane), Arc::new(cb), Arc::new(cr)], chroma: Chroma::C420, alpha: None }
        }
        Depth::Ten => {
            let shift = 16 - depth.bits();
            let mut luma_plane = pool::take_u16(width.saturating_mul(height));
            luma_plane.extend(luma_rows.flat_map(|row| row.as_chunks::<2>().0).map(|&word| u16::from_ne_bytes(word) >> shift));
            let (mut cb, mut cr) = (pool::take_u16(samples), pool::take_u16(samples));
            for &[b0, b1, r0, r1] in chroma_rows.flat_map(|row| row.as_chunks::<4>().0) {
                cb.push(u16::from_ne_bytes([b0, b1]) >> shift);
                cr.push(u16::from_ne_bytes([r0, r1]) >> shift);
            }
            PixelData::Yuv16 { planes: [Arc::new(luma_plane), Arc::new(cb), Arc::new(cr)], chroma: Chroma::C420, bits: depth.bits(), alpha: None }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A surface of `rows` luma rows of `stride` bytes, each byte its own index, and the `rows / 2` chroma
    /// rows after it, each byte 100 + its index.
    fn surface(stride: usize, rows: usize) -> (Vec<u8>, Vec<u8>) {
        ((0..stride * rows).map(|i| i as u8).collect(), (0..stride * rows.div_ceil(2)).map(|i| 100 + i as u8).collect())
    }

    fn plane(data: &[u8], stride: usize) -> Plane<'_> {
        Plane { data, stride }
    }

    fn yuv8(pixels: &PixelData) -> (&[u8], &[u8], &[u8]) {
        let PixelData::Yuv8 { planes, chroma, alpha } = pixels else { panic!("not 8-bit planar") };
        assert_eq!((*chroma, alpha.is_none()), (Chroma::C420, true));
        (&planes[0], &planes[1], &planes[2])
    }

    fn yuv16(pixels: &PixelData) -> (&[u16], &[u16], &[u16]) {
        let PixelData::Yuv16 { planes, chroma, bits, alpha } = pixels else { panic!("not 16-bit planar") };
        assert_eq!((*chroma, *bits, alpha.is_none()), (Chroma::C420, 10, true));
        (&planes[0], &planes[1], &planes[2])
    }

    #[test]
    fn eight_bit_planes_are_cropped_and_the_chroma_pairs_are_split_into_cb_and_cr() {
        // A 10-byte row holds 8 samples and 2 of padding; the picture is the 4x4 at (2, 2).
        let (luma, chroma) = surface(10, 6);
        let (luma, chroma) = (plane(&luma, 10), plane(&chroma, 10));
        let pixels = to_pixels(luma, chroma, Crop { x: 2, y: 2, width: 4, height: 4 }, Depth::Eight).unwrap();
        let (y, cb, cr) = yuv8(&pixels);
        assert_eq!(y, [22, 23, 24, 25, 32, 33, 34, 35, 42, 43, 44, 45, 52, 53, 54, 55]);
        // Chroma rows 1 and 2, pairs from byte 2: row 1 starts at 10, row 2 at 20.
        assert_eq!(cb, [112, 114, 122, 124]);
        assert_eq!(cr, [113, 115, 123, 125]);
    }

    #[test]
    fn an_odd_sized_picture_gets_chroma_planes_rounded_up() {
        let (luma, chroma) = surface(8, 6);
        let (luma, chroma) = (plane(&luma, 8), plane(&chroma, 8));
        let pixels = to_pixels(luma, chroma, Crop { x: 0, y: 0, width: 5, height: 3 }, Depth::Eight).unwrap();
        let (y, cb, cr) = yuv8(&pixels);
        assert_eq!(y, [0, 1, 2, 3, 4, 8, 9, 10, 11, 12, 16, 17, 18, 19, 20]);
        // 3 chroma samples across and 2 down: the rows at 0 and 8, three pairs each.
        assert_eq!(cb, [100, 102, 104, 108, 110, 112]);
        assert_eq!(cr, [101, 103, 105, 109, 111, 113]);
    }

    /// Words of a P010 surface, as the little-endian bytes the planes hold: the sample in the high 10 bits.
    fn p010(samples: &[u16]) -> Vec<u8> {
        samples.iter().flat_map(|sample| (sample << 6).to_ne_bytes()).collect()
    }

    #[test]
    fn ten_bit_samples_are_shifted_down_from_the_high_bits() {
        let luma = p010(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let chroma = p010(&[500, 501, 502, 503]);
        let (luma, chroma) = (plane(&luma, 8), plane(&chroma, 8));
        let pixels = to_pixels(luma, chroma, Crop { x: 0, y: 0, width: 4, height: 2 }, Depth::Ten).unwrap();
        let (y, cb, cr) = yuv16(&pixels);
        assert_eq!(y, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!((cb, cr), (&[500, 502][..], &[501, 503][..]));
    }

    #[test]
    fn an_odd_sized_ten_bit_picture_is_cropped_and_rounded_up_too() {
        // A 6-sample, 4-row surface of words (stride 12 bytes); the picture is the 3x3 at (2, 0).
        let luma = p010(&(0..24).collect::<Vec<u16>>());
        let chroma = p010(&(500..512).collect::<Vec<u16>>());
        let (luma, chroma) = (plane(&luma, 12), plane(&chroma, 12));
        let pixels = to_pixels(luma, chroma, Crop { x: 2, y: 0, width: 3, height: 3 }, Depth::Ten).unwrap();
        let (y, cb, cr) = yuv16(&pixels);
        assert_eq!(y, [2, 3, 4, 8, 9, 10, 14, 15, 16]);
        // 2 chroma samples across (from pair 1) and 2 down: pairs 1 and 2 of rows 0 and 1.
        assert_eq!(cb, [502, 504, 508, 510]);
        assert_eq!(cr, [503, 505, 509, 511]);
    }

    #[test]
    fn a_surface_smaller_than_the_picture_is_an_error_never_a_panic() {
        let (luma, chroma) = surface(8, 8);
        let (luma, chroma) = (plane(&luma, 8), plane(&chroma, 8));
        let whole = Crop { x: 0, y: 0, width: 8, height: 8 };
        assert!(to_pixels(luma, chroma, whole, Depth::Eight).is_ok());
        for crop in [
            Crop { width: 9, ..whole },
            Crop { height: 9, ..whole },
            Crop { x: 2, ..whole },
            // Rows that run past their stride into the next row, while the plane still holds the last one.
            Crop { x: 2, height: 4, ..whole },
            Crop { y: 2, ..whole },
            Crop { width: 0, ..whole },
            Crop { height: 0, ..whole },
            Crop { x: usize::MAX - 1, ..whole },
            Crop { x: 1, width: 4, ..whole },
            Crop { width: usize::MAX, ..whole },
        ] {
            assert!(to_pixels(luma, chroma, crop, Depth::Eight).is_err(), "{crop:?}");
        }
        // Ten-bit words are twice as wide, so the same surface holds only half the samples.
        assert!(to_pixels(luma, chroma, whole, Depth::Ten).is_err());
        let short = plane(&chroma.data[..8], 8);
        assert!(to_pixels(luma, short, whole, Depth::Eight).is_err(), "a chroma plane shorter than the picture");
        assert!(plane(luma.data, 0).rows(0, 0, 0, 1).is_none(), "a surface that reports no stride");
    }
}
