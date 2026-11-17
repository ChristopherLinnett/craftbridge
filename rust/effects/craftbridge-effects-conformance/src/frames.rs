//! Decoded frames and the D-12 comparison: an encoded export conforms when its frame count and size equal
//! the reference's and every decoded frame is within 40 dB PSNR of the reference's decoded frame (plan §28).
//! An export whose decoded frames the plan asks to be identical (a GIF's) is held to the same size and count
//! with every frame the same pixel for pixel.

use effectcraft_media::{MediaPool, probe};
use effectcraft_time::Tick;
use std::path::Path;

/// The threshold of D-12: the encoder's slices follow the core count, so encoded output differs by
/// machine, and 40 dB is where 8-bit video shows no visible difference. It is one definition: a scenario
/// cannot loosen it.
pub(crate) const MIN_PSNR_DB: f64 = 40.0;

/// More frames than this are not a conformance clip; the cap keeps a hostile or broken file's frame count
/// from becoming a loop that never ends.
const MAX_FRAMES: u32 = 100_000;

/// The frame count a probed duration and rate give; a still, with neither, is one frame. A count past
/// [`MAX_FRAMES`] is refused, never cut down to it: two clips cut to the same cap would compare as the same
/// length whatever their real counts, and every frame past the cap would go unchecked.
fn frame_count(seconds: f64, rate: f64) -> Result<u32, String> {
    if !(seconds > 0.0 && rate > 0.0) {
        return Ok(1);
    }
    let count = (seconds * rate).round();
    if count > f64::from(MAX_FRAMES) {
        return Err(format!("{count} frames, more than the {MAX_FRAMES} a conformance clip holds"));
    }
    // At most MAX_FRAMES and not negative, so `as` is exact.
    Ok((count as u32).max(1))
}

/// A still image or a movie, decoded one frame at a time by EffectCraft's own decoders.
pub(crate) struct Clip {
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// A still is one frame.
    pub(crate) frames: u32,
    decode: Box<dyn Fn(u32) -> Result<Vec<u8>, String>>,
}

impl Clip {
    pub(crate) fn open(path: &Path) -> Result<Clip, String> {
        let name = path.display().to_string();
        let footage = probe(path).map_err(|error| format!("cannot read {name}: {error}"))?;
        let (width, height, rate) = (footage.width, footage.height, footage.frame_rate);
        let frames = frame_count(footage.duration.seconds(), rate.as_f64()).map_err(|error| format!("{name}: {error}"))?;
        let pool = MediaPool::new();
        let decode = move |frame: u32| {
            let at = Tick::from_rational(i64::from(frame), rate.den, rate.num);
            pool.frame_at(&footage, at).map(|image| image.to_rgba8()).map_err(|error| format!("cannot decode frame {frame} of {name}: {error}"))
        };
        Ok(Clip { width, height, frames, decode: Box::new(decode) })
    }

    /// A clip of frames already in memory: what a movie decodes to, for the tests of the comparison.
    #[cfg(test)]
    pub(crate) fn from_frames(width: u32, height: u32, frames: Vec<Vec<u8>>) -> Clip {
        Clip {
            width,
            height,
            frames: frames.len() as u32,
            decode: Box::new(move |index| frames.get(index as usize).cloned().ok_or_else(|| "no such frame".to_string())),
        }
    }

    /// The frame's pixels, 8-bit RGBA.
    pub(crate) fn frame(&self, index: u32) -> Result<Vec<u8>, String> {
        if index >= self.frames {
            return Err(format!("frame {index} does not exist: the clip has {} frames", self.frames));
        }
        (self.decode)(index)
    }
}

/// PSNR in dB of two 8-bit RGBA frames over their colour channels: alpha is 255 in every encoded frame,
/// and counting it would raise the figure of a frame that is wrong. `None` when the frames cannot be
/// compared (different lengths, or none); frames that are the same have infinite PSNR.
pub(crate) fn psnr_db(actual: &[u8], reference: &[u8]) -> Option<f64> {
    if actual.len() != reference.len() || actual.is_empty() {
        return None;
    }
    let (mut squared, mut samples) = (0.0_f64, 0.0_f64);
    let ((ours, _), (theirs, _)) = (actual.as_chunks::<4>(), reference.as_chunks::<4>());
    for ([r1, g1, b1, _], [r2, g2, b2, _]) in ours.iter().zip(theirs) {
        for (x, y) in [(r1, r2), (g1, g2), (b1, b2)] {
            let difference = f64::from(*x) - f64::from(*y);
            squared += difference * difference;
            samples += 1.0;
        }
    }
    if samples == 0.0 {
        return None;
    }
    let mean = squared / samples;
    Some(if mean == 0.0 { f64::INFINITY } else { 10.0 * (255.0_f64 * 255.0 / mean).log10() })
}

/// How many failing frames or files a comparison names; the rest are counted.
pub(crate) const NAMED: usize = 3;

/// How a frame pair is held: within D-12's 40 dB, or, for `exact`, the same pixel for pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Closeness {
    D12,
    Exact,
}

impl Closeness {
    /// The failure of one frame `a` of the actual clip against frame `r` of the reference's, if it is not
    /// close enough; the figure of a D-12 failure is what the count of failing frames names.
    fn judge(self, a: u32, r: u32, ours: &[u8], theirs: &[u8]) -> Result<Option<String>, String> {
        match self {
            Closeness::Exact => Ok((ours != theirs).then(|| format!("frame {a} is not the reference's frame {r}, pixel for pixel"))),
            Closeness::D12 => match psnr_db(ours, theirs) {
                None => Err(format!("frame {a} cannot be compared with the reference's frame {r}")),
                Some(db) if db < MIN_PSNR_DB => Ok(Some(format!("frame {a} is {db:.2} dB from the reference's frame {r}, below the {MIN_PSNR_DB} dB of D-12"))),
                Some(_) => Ok(None),
            },
        }
    }
}

/// The failures of `actual` against `reference` under D-12, or under `Exact`; none means they conform. With
/// no frame index the two are movies and every frame pair is compared; an index picks the one frame compared
/// (an index on one side alone leaves the other at frame 0).
pub(crate) fn conform(actual: &Clip, reference: &Clip, actual_frame: Option<u32>, reference_frame: Option<u32>, closeness: Closeness) -> Vec<String> {
    let mut failures = Vec::new();
    if (actual.width, actual.height) != (reference.width, reference.height) {
        failures.push(format!("size {}x{} is not the reference's {}x{}", actual.width, actual.height, reference.width, reference.height));
    }
    let pairs: Vec<(u32, u32)> = if actual_frame.is_none() && reference_frame.is_none() {
        if actual.frames != reference.frames {
            failures.push(format!("{} frames are not the reference's {}", actual.frames, reference.frames));
        }
        (0..actual.frames.min(reference.frames)).map(|frame| (frame, frame)).collect()
    } else {
        vec![(actual_frame.unwrap_or(0), reference_frame.unwrap_or(0))]
    };
    if !failures.is_empty() {
        return failures;
    }
    let mut below = 0_usize;
    for (a, r) in pairs {
        let verdict = actual.frame(a).and_then(|ours| reference.frame(r).and_then(|theirs| closeness.judge(a, r, &ours, &theirs)));
        match verdict {
            Err(error) => failures.push(error),
            Ok(Some(failure)) => {
                below += 1;
                if below <= NAMED {
                    failures.push(failure);
                }
            }
            Ok(None) => {}
        }
    }
    if below > NAMED {
        failures.push(match closeness {
            Closeness::D12 => format!("{} more frames are below {MIN_PSNR_DB} dB", below - NAMED),
            Closeness::Exact => format!("{} more frames are not the reference's", below - NAMED),
        });
    }
    failures
}

/// A PNG read as the straight 8-bit RGBA it holds, with its size: the pixels as the file has them, where
/// a [`Clip`] decodes a still the way the engine does.
fn straight_rgba(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let name = path.display().to_string();
    let image = image::open(path).map_err(|error| format!("cannot read {name}: {error}"))?.to_rgba8();
    Ok((image.width(), image.height(), image.into_raw()))
}

/// The failures of two PNGs' pixels, `actual` against `reference`: each pixel is over when any channel is
/// further than `tolerance` levels from the reference's, no more than `allowed_fraction` of the pixels may
/// be, and none may be further than `cap` levels. With tolerance 0 and no fraction the pixels are the same.
pub(crate) fn pixels(actual: &Path, reference: &Path, tolerance: u8, allowed_fraction: f64, cap: u8) -> Vec<String> {
    let ((aw, ah, ours), (rw, rh, theirs)) = match (straight_rgba(actual), straight_rgba(reference)) {
        (Ok(ours), Ok(theirs)) => (ours, theirs),
        (Err(error), _) | (_, Err(error)) => return vec![error],
    };
    if (aw, ah) != (rw, rh) {
        return vec![format!("size {aw}x{ah} is not the reference's {rw}x{rh}")];
    }
    let (mut over, mut furthest, mut total) = (0_usize, 0_u8, 0_usize);
    let (ours, _) = ours.as_chunks::<4>();
    let (theirs, _) = theirs.as_chunks::<4>();
    for (a, b) in ours.iter().zip(theirs) {
        let worst = a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        furthest = furthest.max(worst);
        over += usize::from(worst > tolerance);
        total += 1;
    }
    let fraction = if total == 0 { 0.0 } else { over as f64 / total as f64 };
    let mut failures = Vec::new();
    if fraction > allowed_fraction {
        failures.push(format!(
            "{over} of {total} pixels are more than {tolerance} levels from the reference's: {:.3}% where {:.3}% is allowed",
            fraction * 100.0,
            allowed_fraction * 100.0
        ));
    }
    if furthest > cap {
        failures.push(format!("a pixel is {furthest} levels from the reference's, past the {cap} allowed"));
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::png_bytes;
    use tempfile::TempDir;

    /// A smooth image: two flat colour channels' worth of detail, so a change to it shows in PSNR.
    fn gradient(x: u32, y: u32) -> [u8; 3] {
        [(x * 4) as u8, (y * 4) as u8, 80]
    }

    /// The PNG `shade` describes, written into the scratch folder and opened.
    fn clip(scratch: &TempDir, name: &str, width: u32, height: u32, shade: impl Fn(u32, u32) -> [u8; 3]) -> Clip {
        let path = scratch.path().join(name);
        std::fs::write(&path, png_bytes(width, height, shade)).expect("written");
        Clip::open(&path).expect("opens")
    }

    /// The gradient with its red channel one level up: far above 40 dB from it, and not the same pixels.
    fn near_gradient(scratch: &TempDir) -> Clip {
        clip(scratch, "near.png", 32, 32, |x, y| [(x * 4) as u8 + 1, (y * 4) as u8, 80])
    }

    /// Every colour sample raised by `by`, alpha as it was.
    fn shifted(frame: &[u8], by: u8) -> Vec<u8> {
        frame.iter().enumerate().map(|(i, value)| if i % 4 == 3 { *value } else { value + by }).collect()
    }

    #[test]
    fn psnr_follows_its_definition() {
        let flat = vec![100_u8; 16];
        assert_eq!(psnr_db(&flat, &flat), Some(f64::INFINITY));
        // Every colour sample off by one: 10 log10(255^2 / 1) = 48.13 dB.
        let db = psnr_db(&flat, &shifted(&flat, 1)).expect("comparable");
        assert!((db - 48.130_803_6).abs() < 1e-6, "{db}");
        // Alpha is not a colour sample.
        let alpha_only: Vec<u8> = flat.iter().enumerate().map(|(i, value)| if i % 4 == 3 { 0 } else { *value }).collect();
        assert_eq!(psnr_db(&flat, &alpha_only), Some(f64::INFINITY));
        assert_eq!(psnr_db(&flat, flat.get(..8).unwrap_or_default()), None);
        assert_eq!(psnr_db(&[], &[]), None);
    }

    #[test]
    fn the_threshold_is_d12s_forty_decibels() {
        assert_eq!(MIN_PSNR_DB, 40.0);
        // A flat error of e gives 20 log10(255 / e) dB: 2.55 is 40 dB, so 3 sits below it and 2 above.
        let flat = vec![100_u8; 4 * 64];
        assert!(psnr_db(&flat, &shifted(&flat, 2)).is_some_and(|db| db >= MIN_PSNR_DB));
        assert!(psnr_db(&flat, &shifted(&flat, 3)).is_some_and(|db| db < MIN_PSNR_DB));
    }

    #[test]
    fn identical_images_conform_and_a_small_difference_stays_above_the_threshold() {
        let scratch = tempfile::tempdir().expect("scratch");
        let reference = clip(&scratch, "ref.png", 32, 32, gradient);
        let same = clip(&scratch, "same.png", 32, 32, gradient);
        assert_eq!((reference.width, reference.height, reference.frames), (32, 32, 1));
        assert!(conform(&same, &reference, None, None, Closeness::D12).is_empty());
        let near = near_gradient(&scratch);
        assert!(conform(&near, &reference, None, None, Closeness::D12).is_empty());
    }

    #[test]
    fn an_image_below_forty_decibels_or_of_another_size_fails_and_says_why() {
        let scratch = tempfile::tempdir().expect("scratch");
        let reference = clip(&scratch, "ref.png", 32, 32, gradient);
        let far = clip(&scratch, "far.png", 32, 32, |x, y| [(x * 4) as u8, (y * 4) as u8, 120]);
        let failures = conform(&far, &reference, None, None, Closeness::D12);
        assert!(failures.iter().any(|f| f.contains("frame 0") && f.contains("dB") && f.contains("D-12")), "{failures:?}");
        let wide = clip(&scratch, "wide.png", 48, 32, gradient);
        assert!(conform(&wide, &reference, None, None, Closeness::D12).iter().any(|f| f.contains("48x32")));
    }

    /// A movie of `count` flat 2x2 frames, each `step` brighter than the last.
    fn movie(count: u8, step: u8) -> Clip {
        Clip::from_frames(2, 2, (0..count).map(|n| shifted(&[100; 16], n * step)).collect())
    }

    #[test]
    fn two_movies_conform_when_their_frame_counts_match_and_every_frame_is_close() {
        assert!(conform(&movie(5, 1), &movie(5, 1), None, None, Closeness::D12).is_empty());
        assert!(conform(&movie(5, 1), &movie(4, 1), None, None, Closeness::D12).iter().any(|f| f.contains("5 frames are not the reference's 4")));
    }

    #[test]
    fn every_frame_of_a_movie_is_compared_not_only_the_first_and_the_worst_are_counted() {
        // Frame 0 is the same in both; the movies drift apart from frame 1 on.
        let failures = conform(&movie(8, 0), &movie(8, 20), None, None, Closeness::D12);
        let named = failures.iter().filter(|f| f.starts_with("frame ")).count();
        assert_eq!(named, NAMED, "{failures:?}");
        assert!(failures.iter().any(|f| f.contains("frame 1 is")) && !failures.iter().any(|f| f.starts_with("frame 0 ")), "{failures:?}");
        assert!(failures.iter().any(|f| f.contains("4 more frames are below 40 dB")), "{failures:?}");
    }

    #[test]
    fn a_frame_count_is_what_duration_and_rate_give_and_past_the_cap_an_error_never_a_cut() {
        assert_eq!(frame_count(2.0, 30.0), Ok(60));
        assert_eq!(frame_count(0.01, 30.0), Ok(1), "a movie shorter than a frame is still one frame");
        // A negative duration and rate multiply to a positive count: only this check keeps a broken
        // probe a still, so every unusable spelling — zero, negative, missing, non-finite — is named.
        for still in [(0.0, 30.0), (2.0, 0.0), (f64::NAN, 30.0), (2.0, f64::NAN), (-2.0, 30.0), (2.0, -30.0), (-2.0, -30.0)] {
            assert_eq!(frame_count(still.0, still.1), Ok(1), "{still:?}");
        }
        assert_eq!(frame_count(f64::from(MAX_FRAMES) / 25.0, 25.0), Ok(MAX_FRAMES));
        assert!(frame_count(f64::from(MAX_FRAMES) + 1.0, 1.0).is_err_and(|error| error.contains("more than")), "one frame past the cap");
        // 150 000 and 200 000 frames would both have been cut to the cap and compared as one length.
        for past in [6000.0, 8000.0] {
            assert!(frame_count(past, 25.0).is_err_and(|error| error.contains("more than")), "{past}");
        }
        assert!(frame_count(f64::INFINITY, 25.0).is_err());
    }

    #[test]
    fn an_exact_comparison_names_every_pixel_where_d12_allows_a_difference_it_cannot_see() {
        let scratch = tempfile::tempdir().expect("scratch");
        let reference = clip(&scratch, "ref.png", 32, 32, gradient);
        let near = near_gradient(&scratch);
        assert!(conform(&near, &reference, None, None, Closeness::D12).is_empty());
        let failures = conform(&near, &reference, None, None, Closeness::Exact);
        assert_eq!(failures, ["frame 0 is not the reference's frame 0, pixel for pixel"]);
        let same = clip(&scratch, "same.png", 32, 32, gradient);
        assert!(conform(&same, &reference, None, None, Closeness::Exact).is_empty());
        // Every frame of a movie is held to it, and the ones past the named few are counted.
        let failures = conform(&movie(8, 0), &movie(8, 1), None, None, Closeness::Exact);
        assert_eq!(failures.iter().filter(|f| f.starts_with("frame ")).count(), NAMED, "{failures:?}");
        assert!(failures.iter().any(|f| f == "4 more frames are not the reference's"), "{failures:?}");
        assert!(conform(&movie(5, 1), &movie(4, 1), None, None, Closeness::Exact).iter().any(|f| f.contains("5 frames are not the reference's 4")));
    }

    #[test]
    fn a_frame_index_beyond_the_clip_is_an_error() {
        let scratch = tempfile::tempdir().expect("scratch");
        let still = clip(&scratch, "a.png", 8, 8, |_, _| [1, 2, 3]);
        assert!(conform(&still, &still, Some(1), None, Closeness::D12).iter().any(|f| f.contains("does not exist")));
        assert!(conform(&still, &still, Some(0), Some(0), Closeness::D12).is_empty());
    }

    #[test]
    fn a_file_that_is_no_media_is_an_error_naming_it() {
        let scratch = tempfile::tempdir().expect("scratch");
        let path = scratch.path().join("not.png");
        std::fs::write(&path, b"not a png").expect("written");
        assert!(Clip::open(&path).err().is_some_and(|error| error.contains("not.png")));
        assert!(Clip::open(&scratch.path().join("absent.png")).is_err());
    }

    /// A PNG file of the pixels `shade` gives, in the scratch folder.
    fn png_file(scratch: &TempDir, name: &str, width: u32, height: u32, shade: impl Fn(u32, u32) -> [u8; 3]) -> std::path::PathBuf {
        let path = scratch.path().join(name);
        std::fs::write(&path, png_bytes(width, height, shade)).expect("written");
        path
    }

    #[test]
    fn pixels_pass_within_the_tolerance_and_name_what_is_over_it() {
        let scratch = tempfile::tempdir().expect("scratch");
        let flat = |level: u8| move |_: u32, _: u32| [level; 3];
        let reference = png_file(&scratch, "ref.png", 10, 10, flat(100));
        assert!(pixels(&png_file(&scratch, "same.png", 10, 10, flat(100)), &reference, 0, 0.0, 0).is_empty());
        let one_off = png_file(&scratch, "one.png", 10, 10, flat(101));
        assert!(pixels(&one_off, &reference, 1, 0.0, 1).is_empty());
        let exact = pixels(&one_off, &reference, 0, 0.0, 0);
        assert!(exact.iter().any(|f| f.contains("100 of 100 pixels are more than 0 levels")) && exact.iter().any(|f| f.contains("1 levels")), "{exact:?}");
        // Three pixels in a hundred are four levels out: the fraction allows them, and the cap sets how far.
        let flips = png_file(&scratch, "flips.png", 10, 10, |x, y| if y == 0 && x < 3 { [104; 3] } else { [100; 3] });
        assert!(pixels(&flips, &reference, 1, 0.03, 4).is_empty());
        assert!(pixels(&flips, &reference, 1, 0.02, 4).iter().any(|f| f.contains("3 of 100 pixels") && f.contains("3.000% where 2.000% is allowed")));
        assert!(pixels(&flips, &reference, 1, 0.03, 3).iter().any(|f| f.contains("4 levels from the reference's, past the 3 allowed")));
        assert!(pixels(&png_file(&scratch, "wide.png", 12, 10, flat(100)), &reference, 9, 1.0, 9).iter().any(|f| f.contains("12x10")));
        assert!(pixels(&scratch.path().join("absent.png"), &reference, 0, 0.0, 0).iter().any(|f| f.contains("absent.png")));
    }
}
