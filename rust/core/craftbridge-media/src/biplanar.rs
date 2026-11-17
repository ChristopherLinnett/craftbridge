//! A 4:2:0 picture as a decoder's surface holds it, read into the planar frames FilmCraft's software
//! decoders produce: tight `Yuv8` planes for 8-bit pictures and `Yuv16` planes with the samples in the low
//! bits for 10-bit ones. A biplanar surface is a luma plane and a plane of interleaved Cb, Cr pairs, a row
//! each `stride` bytes apart, and 10-bit samples sit in the high bits of 16-bit words; Android's
//! `YUV_420_888` image is a luma plane and a Cb and a Cr plane, whose samples sit a pixel stride apart (one
//! for planar, two for the semi-planar layout most devices use). A locked P010 hardware buffer is the biplanar
//! surface again, which Android reports as three planes: the Cb plane's samples sit four bytes apart and the
//! Cr plane starts two bytes after it, so the two are one plane of interleaved pairs.
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

#[cfg(any(target_os = "ios", target_os = "android", test))]
impl Depth {
    pub(crate) fn bits(self) -> u32 {
        match self {
            Self::Eight => 8,
            Self::Ten => 10,
        }
    }
}

/// One chroma plane of a `YUV_420_888` image: a sample every `pixel` bytes, a row each `stride` bytes apart.
#[cfg(any(target_os = "android", test))]
#[derive(Clone, Copy)]
pub(crate) struct Strided<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) stride: usize,
    /// Bytes from one sample to the next in a row.
    pub(crate) pixel: usize,
}

#[cfg(any(target_os = "android", test))]
impl Strided<'_> {
    /// Appends `count` rows of `len` samples to `out`, from the sample at (`x`, `y`) in this plane's own
    /// samples; `None` unless every one of them lies in the plane and in its own
    /// row, so a hostile size never reads, or allocates for, more than the plane holds.
    fn copy(&self, y: usize, x: usize, len: usize, count: usize, out: &mut Vec<u8>) -> Option<()> {
        let last_sample = x.checked_add(len.checked_sub(1)?)?.checked_mul(self.pixel)?;
        let last = y.checked_add(count.checked_sub(1)?)?.checked_mul(self.stride)?.checked_add(last_sample)?;
        if self.pixel == 0 || last_sample >= self.stride || last >= self.data.len() {
            return None;
        }
        let first = x.checked_mul(self.pixel)?;
        for row in 0..count {
            let start = y.checked_add(row)?.checked_mul(self.stride)?.checked_add(first)?;
            out.extend(self.data.get(start..)?.iter().step_by(self.pixel).take(len));
        }
        Some(())
    }
}

/// One plane of a locked hardware buffer as `AHardwareBuffer_lockPlanes` reports it, with the plane's
/// address as a number: the decision whether the planes are a P010 buffer's is safe code every target tests,
/// and the platform module alone turns an address into bytes.
#[cfg(any(target_os = "android", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Locked {
    pub(crate) address: usize,
    /// Bytes from the start of one row to the start of the next.
    pub(crate) row_stride: usize,
    /// Bytes from one sample to the next in a row.
    pub(crate) pixel_stride: usize,
}

/// How many bytes of a locked P010 buffer hold the picture, and how far apart its rows are: the luma plane
/// and the plane of interleaved Cb, Cr pairs that starts at the Cb plane's address.
#[cfg(any(target_os = "android", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct P010Extents {
    pub(crate) luma_len: usize,
    pub(crate) luma_stride: usize,
    pub(crate) chroma_len: usize,
    pub(crate) chroma_stride: usize,
}

#[cfg(any(target_os = "android", test))]
const NOT_P010: &str = "the decoded picture's planes are not a P010 buffer's";

/// The bytes `rows` rows of `row_bytes` bytes take when each starts `stride` bytes after the last: the last
/// row's padding is not part of the buffer.
#[cfg(any(target_os = "android", test))]
fn span(stride: usize, rows: usize, row_bytes: usize) -> Option<usize> {
    if stride < row_bytes {
        return None;
    }
    stride.checked_mul(rows.checked_sub(1)?)?.checked_add(row_bytes)
}

/// The extents of a P010 buffer of `width` x `height` samples from the three planes locking it gave (Y, Cb,
/// Cr). `AHardwareBuffer_lockPlanes` reports no length, so the picture's own size bounds every read, the way
/// Android's own reader bounds its plane data: a row stride times the rows before the last, plus the last
/// row's samples. An error unless the planes are P010's biplanar layout: 16-bit luma, and Cb and Cr words
/// interleaved in one plane, which locking reports as two planes four bytes per sample apart, the Cr plane
/// two bytes after the Cb plane. Any other layout the buffer's format name allows is left to the software
/// decoder, never read as if it were this one.
#[cfg(any(target_os = "android", test))]
pub(crate) fn p010_extents(planes: &[Locked], width: usize, height: usize) -> Result<P010Extents, &'static str> {
    let [luma, cb, cr] = planes else { return Err(NOT_P010) };
    let interleaved = cb.address.checked_add(2) == Some(cr.address) && cb.row_stride == cr.row_stride;
    if (luma.pixel_stride, cb.pixel_stride, cr.pixel_stride) != (2, 4, 4) || !interleaved || luma.address == 0 || cb.address == 0 {
        return Err(NOT_P010);
    }
    if width == 0 || height == 0 {
        return Err("the decoded picture is empty");
    }
    let luma_len = span(luma.row_stride, height, width.checked_mul(2).ok_or(NOT_P010)?);
    let chroma_len = span(cb.row_stride, height.div_ceil(2), width.div_ceil(2).checked_mul(4).ok_or(NOT_P010)?);
    let (luma_len, chroma_len) = luma_len.zip(chroma_len).ok_or(NOT_P010)?;
    Ok(P010Extents { luma_len, luma_stride: luma.row_stride, chroma_len, chroma_stride: cb.row_stride })
}

const SHORT: &str = "the decoded picture's planes are smaller than the picture";

/// The picture's size in chroma samples, rounded up for an odd one; an error for an empty picture, or one
/// at `crop`'s odd offset, where 4:2:0 chroma cannot start.
fn chroma_size(crop: Crop) -> Result<(usize, usize), &'static str> {
    let Crop { x, y, width, height } = crop;
    if width == 0 || height == 0 {
        return Err("the decoded picture is empty");
    }
    if x % 2 != 0 || y % 2 != 0 {
        return Err("the decoded picture starts at an odd sample");
    }
    Ok((width.div_ceil(2), height.div_ceil(2)))
}

/// 8-bit planar pixels around the three planes.
fn yuv8(luma: Vec<u8>, cb: Vec<u8>, cr: Vec<u8>) -> PixelData {
    PixelData::Yuv8 { planes: [Arc::new(luma), Arc::new(cb), Arc::new(cr)], chroma: Chroma::C420, alpha: None }
}

/// The luma plane's rows at `crop` as one tight plane, every row checked to lie in the plane before
/// anything is allocated for it.
fn luma_8(luma: Plane<'_>, crop: Crop) -> Result<Vec<u8>, &'static str> {
    let rows = luma.rows(crop.y, crop.x, crop.width, crop.height).ok_or(SHORT)?;
    let mut plane = pool::take_u8(crop.width.saturating_mul(crop.height));
    rows.for_each(|row| plane.extend_from_slice(row));
    Ok(plane)
}

/// The picture at `crop` in `luma` and the two chroma planes of a `YUV_420_888` image, as the planar pixels
/// FilmCraft's software decoder produces for an 8-bit picture. The luma samples sit one byte apart. An error
/// when the planes hold less than the picture, or `crop` starts at an odd sample.
#[cfg(any(target_os = "android", test))]
pub(crate) fn planar_to_pixels(luma: Plane<'_>, cb: Strided<'_>, cr: Strided<'_>, crop: Crop) -> Result<PixelData, &'static str> {
    let (chroma_width, chroma_height) = chroma_size(crop)?;
    let luma_plane = luma_8(luma, crop)?;
    let samples = chroma_width.saturating_mul(chroma_height);
    let (mut cb_plane, mut cr_plane) = (pool::take_u8(samples), pool::take_u8(samples));
    let (x, y) = (crop.x / 2, crop.y / 2);
    cb.copy(y, x, chroma_width, chroma_height, &mut cb_plane).ok_or(SHORT)?;
    cr.copy(y, x, chroma_width, chroma_height, &mut cr_plane).ok_or(SHORT)?;
    Ok(yuv8(luma_plane, cb_plane, cr_plane))
}

/// The picture at `crop` in `luma` and `chroma`, as the planar pixels FilmCraft's software decoder
/// produces for it: planes of exactly the picture's size (the chroma planes rounded up for an odd one), and
/// the 10-bit samples shifted down from the high bits. An error when the planes hold less than the
/// picture, or `crop` starts at an odd sample, which 4:2:0 chroma cannot start at.
#[cfg(any(target_os = "ios", target_os = "android", test))]
pub(crate) fn to_pixels(luma: Plane<'_>, chroma: Plane<'_>, crop: Crop, depth: Depth) -> Result<PixelData, &'static str> {
    let Crop { x, y, width, height } = crop;
    let (chroma_width, chroma_height) = chroma_size(crop)?;
    let bytes = match depth {
        Depth::Eight => 1,
        Depth::Ten => 2,
    };
    // The bytes of `samples` samples of `parts` values each: a chroma sample is a Cb, Cr pair.
    let scale = |samples: usize, parts: usize| samples.checked_mul(parts).and_then(|values| values.checked_mul(bytes)).ok_or(SHORT);
    // Every row is checked to lie in its plane before anything is allocated for it.
    let chroma_rows = chroma.rows(y / 2, scale(x / 2, 2)?, scale(chroma_width, 2)?, chroma_height).ok_or(SHORT)?;
    let samples = chroma_width.saturating_mul(chroma_height);
    Ok(match depth {
        Depth::Eight => {
            let luma_plane = luma_8(luma, crop)?;
            let (mut cb, mut cr) = (pool::take_u8(samples), pool::take_u8(samples));
            for &[b, r] in chroma_rows.flat_map(|row| row.as_chunks::<2>().0) {
                cb.push(b);
                cr.push(r);
            }
            yuv8(luma_plane, cb, cr)
        }
        Depth::Ten => {
            let luma_rows = luma.rows(y, scale(x, 1)?, scale(width, 1)?, height).ok_or(SHORT)?;
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

    /// The chroma of `surface(stride, rows)` as a `YUV_420_888` image holds it: a Cb plane and a Cr plane,
    /// each sample `pixel` bytes after the last. With `pixel` 2 the two planes share the surface's interleaved
    /// bytes, the Cr plane starting one byte after the Cb plane, as a semi-planar image's do; with `pixel` 1 they
    /// are separate planes of half the stride.
    fn chroma_planes(chroma: &[u8], stride: usize, pixel: usize) -> (Vec<u8>, Vec<u8>, usize) {
        if pixel == 2 {
            (chroma.to_vec(), chroma.get(1..).unwrap().to_vec(), stride)
        } else {
            let pick = |from: usize| chroma.iter().skip(from).step_by(2).copied().collect::<Vec<u8>>();
            (pick(0), pick(1), stride / 2)
        }
    }

    #[test]
    fn a_yuv_420_888_image_in_either_layout_gives_the_pixels_a_biplanar_surface_of_it_does() {
        let (luma, chroma) = surface(10, 6);
        for crop in [Crop { x: 2, y: 2, width: 4, height: 4 }, Crop { x: 0, y: 0, width: 5, height: 3 }, Crop { x: 0, y: 0, width: 8, height: 6 }] {
            let wanted = to_pixels(plane(&luma, 10), plane(&chroma, 10), crop, Depth::Eight).unwrap();
            for pixel in [1, 2] {
                let (cb, cr, stride) = chroma_planes(&chroma, 10, pixel);
                let (cb, cr) = (Strided { data: &cb, stride, pixel }, Strided { data: &cr, stride, pixel });
                let planar = planar_to_pixels(plane(&luma, 10), cb, cr, crop).unwrap();
                assert_eq!(yuv8(&planar), yuv8(&wanted), "{crop:?} with a pixel stride of {pixel}");
            }
        }
    }

    #[test]
    fn a_yuv_420_888_image_smaller_than_the_picture_is_an_error_never_a_panic() {
        let (luma, chroma) = surface(8, 8);
        let (cb, cr, stride) = chroma_planes(&chroma, 8, 2);
        let planes = |cb: &Strided<'_>, cr: &Strided<'_>, crop: Crop| planar_to_pixels(plane(&luma, 8), *cb, *cr, crop);
        let (cb, cr) = (Strided { data: &cb, stride, pixel: 2 }, Strided { data: &cr, stride, pixel: 2 });
        let whole = Crop { x: 0, y: 0, width: 8, height: 8 };
        assert!(planes(&cb, &cr, whole).is_ok());
        for crop in [
            Crop { width: 9, ..whole },
            Crop { height: 9, ..whole },
            Crop { x: 2, ..whole },
            Crop { y: 2, ..whole },
            Crop { x: 1, width: 4, ..whole },
            Crop { y: 1, height: 4, ..whole },
            Crop { width: 0, ..whole },
            Crop { height: 0, ..whole },
            Crop { x: usize::MAX - 1, ..whole },
            Crop { width: usize::MAX, ..whole },
            Crop { height: usize::MAX, ..whole },
        ] {
            assert!(planes(&cb, &cr, crop).is_err(), "{crop:?}");
        }
        // A chroma plane that stops a sample short, a row that does not hold the samples, a plane that has no
        // pixel stride and one that has no rows.
        let short = Strided { data: cb.data.get(..cb.data.len() - 2).unwrap(), ..cb };
        assert!(planes(&short, &cr, whole).is_err(), "a Cb plane a sample short");
        assert!(planes(&cb, &Strided { stride: 6, ..cr }, whole).is_err(), "a row shorter than its samples");
        assert!(planes(&Strided { pixel: 0, ..cb }, &cr, whole).is_err(), "a pixel stride of nothing");
        assert!(planes(&cb, &Strided { stride: 0, ..cr }, whole).is_err(), "a row stride of nothing");
        assert!(planar_to_pixels(plane(&luma, 8), cb, cr, Crop { x: 0, y: 0, width: 8, height: 8 }).is_ok());
        assert!(planar_to_pixels(plane(&luma, 0), cb, cr, whole).is_err(), "a luma plane with no stride");
    }
    /// A P010 buffer of `rows` luma rows, `luma_stride` bytes apart, and the `rows / 2` rows of Cb, Cr pairs
    /// after it, `chroma_stride` bytes apart: the sample at (x, y) is `y * 100 + x` in luma, and a pair's Cb
    /// is `500 + y * 100 + pair` and its Cr 50 more, all as P010 words, with a filler byte in every padding.
    fn buffer(luma_stride: usize, chroma_stride: usize, rows: usize) -> (Vec<u8>, usize) {
        let mut bytes = Vec::new();
        for y in 0..rows {
            let row: Vec<u16> = (0..luma_stride / 2).map(|x| (y * 100 + x) as u16).collect();
            bytes.extend(p010(&row));
        }
        let chroma_at = bytes.len();
        for y in 0..rows.div_ceil(2) {
            let row: Vec<u16> = (0..chroma_stride / 2).map(|i| (500 + y * 100 + i / 2) as u16 + if i % 2 == 1 { 50 } else { 0 }).collect();
            bytes.extend(p010(&row));
        }
        (bytes, chroma_at)
    }

    /// The planes locking `bytes` reports, the Cb plane at `chroma_at` and the Cr plane two bytes after it.
    fn locked(chroma_at: usize, luma_stride: usize, chroma_stride: usize) -> [Locked; 3] {
        [
            Locked { address: 4096, row_stride: luma_stride, pixel_stride: 2 },
            Locked { address: 4096 + chroma_at, row_stride: chroma_stride, pixel_stride: 4 },
            Locked { address: 4096 + chroma_at + 2, row_stride: chroma_stride, pixel_stride: 4 },
        ]
    }

    /// What reading `bytes` as the P010 buffer of `width` x `height` samples with `planes` gives at `crop`.
    fn read(bytes: &[u8], planes: &[Locked], (width, height): (usize, usize), crop: Crop) -> Result<PixelData, &'static str> {
        let extents = p010_extents(planes, width, height)?;
        let at = |plane: &Locked, len: usize, stride: usize| {
            let from = plane.address - 4096;
            bytes.get(from..from + len).map(|data| Plane { data, stride }).ok_or("the test's buffer is shorter than its extents")
        };
        let luma = at(planes.first().unwrap(), extents.luma_len, extents.luma_stride)?;
        let chroma = at(planes.get(1).unwrap(), extents.chroma_len, extents.chroma_stride)?;
        to_pixels(luma, chroma, crop, Depth::Ten)
    }

    #[test]
    fn a_locked_p010_buffer_with_padded_rows_gives_the_pictures_ten_bit_planes() {
        // 6 samples of 12 bytes in a row of 20 bytes (4 words of padding), 4 rows.
        let (bytes, chroma_at) = buffer(20, 24, 4);
        let planes = locked(chroma_at, 20, 24);
        let whole = Crop { x: 0, y: 0, width: 6, height: 4 };
        let pixels = read(&bytes, &planes, (6, 4), whole).unwrap();
        let (y, cb, cr) = yuv16(&pixels);
        assert_eq!(y, [0, 1, 2, 3, 4, 5, 100, 101, 102, 103, 104, 105, 200, 201, 202, 203, 204, 205, 300, 301, 302, 303, 304, 305]);
        assert_eq!(cb, [500, 501, 502, 600, 601, 602]);
        assert_eq!(cr, [550, 551, 552, 650, 651, 652]);
    }

    #[test]
    fn a_locked_p010_buffer_is_cropped_and_odd_sizes_round_the_chroma_up() {
        let (bytes, chroma_at) = buffer(20, 24, 4);
        let planes = locked(chroma_at, 20, 24);
        let pixels = read(&bytes, &planes, (6, 4), Crop { x: 2, y: 2, width: 3, height: 2 }).unwrap();
        let (y, cb, cr) = yuv16(&pixels);
        assert_eq!(y, [202, 203, 204, 302, 303, 304]);
        // Chroma row 1 from pair 1, two pairs across (3 samples round up) and one down.
        assert_eq!((cb, cr), (&[601, 602][..], &[651, 652][..]));
        // A buffer of 5 x 3 samples, whose chroma rows are 3 pairs across and 2 down.
        let pixels = read(&bytes, &locked(chroma_at, 20, 24), (5, 3), Crop { x: 0, y: 0, width: 5, height: 3 }).unwrap();
        let (y, cb, _) = yuv16(&pixels);
        assert_eq!((y.len(), cb), (15, &[500, 501, 502, 600, 601, 602][..]));
    }

    #[test]
    fn a_locked_p010_buffers_extents_stop_at_the_last_samples_so_planes_that_end_there_are_enough_and_one_byte_less_is_not() {
        let (bytes, chroma_at) = buffer(20, 24, 4);
        let planes = locked(chroma_at, 20, 24);
        let extents = p010_extents(&planes, 6, 4).unwrap();
        assert_eq!(extents, P010Extents { luma_len: 20 * 3 + 12, luma_stride: 20, chroma_len: 24 + 12, chroma_stride: 24 });
        let whole = Crop { x: 0, y: 0, width: 6, height: 4 };
        let (luma, chroma) = (plane(bytes.get(..extents.luma_len).unwrap(), 20), plane(bytes.get(chroma_at..chroma_at + extents.chroma_len).unwrap(), 24));
        assert!(to_pixels(luma, chroma, whole, Depth::Ten).is_ok());
        let short_luma = plane(bytes.get(..extents.luma_len - 1).unwrap(), 20);
        assert!(to_pixels(short_luma, chroma, whole, Depth::Ten).is_err(), "a luma plane a byte short");
        let short_chroma = plane(bytes.get(chroma_at..chroma_at + extents.chroma_len - 1).unwrap(), 24);
        assert!(to_pixels(luma, short_chroma, whole, Depth::Ten).is_err(), "a chroma plane a byte short");
    }

    #[test]
    fn planes_that_are_not_a_p010_buffers_biplanar_layout_are_refused() {
        let planes = locked(1000, 20, 24);
        assert!(p010_extents(&planes, 6, 4).is_ok());
        let refused = |change: &dyn Fn(&mut [Locked; 3])| {
            let mut planes = planes;
            change(&mut planes);
            assert_eq!(p010_extents(&planes, 6, 4), Err(NOT_P010));
        };
        refused(&|p| p[0].pixel_stride = 1);
        refused(&|p| p[1].pixel_stride = 2);
        refused(&|p| p[2].pixel_stride = 2);
        refused(&|p| p[2].address = p[1].address + 4);
        refused(&|p| p[2].address = p[1].address - 2);
        refused(&|p| p[2].row_stride = 28);
        refused(&|p| p[0].address = 0);
        refused(&|p| p[1].address = 0);
        refused(&|p| p[0].row_stride = 11);
        refused(&|p| (p[1].row_stride, p[2].row_stride) = (11, 11));
        assert_eq!(p010_extents(planes.get(..2).unwrap(), 6, 4), Err(NOT_P010), "two planes");
        assert_eq!(p010_extents(&[planes[0], planes[1], planes[2], planes[2]], 6, 4), Err(NOT_P010), "four planes");
        assert_eq!(p010_extents(&planes, 0, 4), Err("the decoded picture is empty"));
        assert_eq!(p010_extents(&planes, 6, 0), Err("the decoded picture is empty"));
        assert_eq!(p010_extents(&planes, usize::MAX, 4), Err(NOT_P010));
        let huge = Locked { row_stride: usize::MAX, ..planes[0] };
        assert_eq!(p010_extents(&[huge, planes[1], planes[2]], 6, 4), Err(NOT_P010), "a stride times the rows overflows");
    }
    #[test]
    fn a_real_ten_bit_pictures_planes_laid_out_as_a_p010_buffer_read_back_to_the_picture() {
        // The software decoder's 10-bit picture of a real stream, written the way a P010 buffer holds it (each
        // sample six bits up, every row padded, the Cb and Cr words interleaved) and read through the locked
        // planes' route.
        let entry = crate::fixtures::hevc10_entry();
        let mut software = filmcraft_codecs::software_video_decoder(&entry).unwrap();
        let picture = crate::fixtures::pictures(software.as_mut(), &crate::fixtures::hevc10_samples()).remove(1);
        let PixelData::Yuv16 { planes: [luma, cb, cr], .. } = &picture.frame.data else { panic!("a 10-bit picture") };
        let (width, height) = (picture.frame.width as usize, picture.frame.height as usize);
        let (luma_stride, chroma_stride) = (width * 2 + 12, width * 2 + 8);
        let mut bytes = Vec::new();
        for row in luma.chunks(width) {
            bytes.extend(p010(row));
            bytes.extend([0xAB; 12]);
        }
        let chroma_at = bytes.len();
        for (cb_row, cr_row) in cb.chunks(width / 2).zip(cr.chunks(width / 2)) {
            bytes.extend(cb_row.iter().zip(cr_row).flat_map(|(&b, &r)| p010(&[b, r])));
            bytes.extend([0xCD; 8]);
        }
        let planes = locked(chroma_at, luma_stride, chroma_stride);
        let whole = Crop { x: 0, y: 0, width, height };
        let read_back = read(&bytes, &planes, (width, height), whole).unwrap();
        assert_eq!(format!("{read_back:?}"), format!("{:?}", picture.frame.data));
    }
}
