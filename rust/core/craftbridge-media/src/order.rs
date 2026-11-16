//! The order pictures come out of a hardware decoder in, and which pictures it never gets.
//!
//! A hardware decoder emits pictures in decoding order; FilmCraft's software decoders emit them in
//! presentation order, which every consumer relies on. [`Reorder`] puts them there, with the stream's own
//! reorder depth, and [`Leading`] leaves out the pictures a software decoder starting at the same sample
//! could not decode either. [`Sequence`] runs both around a platform's raw decoder, so the order of those
//! steps is code every target tests, not code only a phone runs.
//!
//! Safe code, so every target tests it.

use filmcraft_codecs::hw::{NalCodec, StreamInfo};

/// Holds pictures until no picture decoded later can come before them in presentation order.
pub(crate) struct Reorder<T> {
    /// In presentation order.
    held: Vec<(i64, T)>,
    /// Pictures that may precede a picture in decoding order and follow it in output order: a picture is
    /// due once more than this many are held.
    depth: usize,
}

impl<T> Reorder<T> {
    pub(crate) fn new(depth: usize) -> Self {
        Self { held: Vec::new(), depth }
    }

    /// Takes a picture with presentation time `pts`; returns those now due, earliest first.
    pub(crate) fn push(&mut self, pts: i64, picture: T) -> Vec<T> {
        let at = self.held.partition_point(|(held, _)| *held <= pts);
        self.held.insert(at, (pts, picture));
        let due = self.held.len().saturating_sub(self.depth);
        self.held.drain(..due).map(|(_, picture)| picture).collect()
    }

    /// Everything held, earliest first: the end of the stream, or a seek.
    pub(crate) fn drain(&mut self) -> Vec<T> {
        self.held.drain(..).map(|(_, picture)| picture).collect()
    }

    pub(crate) fn clear(&mut self) {
        self.held.clear();
    }
}

/// The RASL pictures of a decoding run that starts at an HEVC CRA or BLA picture. They refer to pictures
/// before it, which a decoder starting there never saw, so they are left out, as the software decoder
/// leaves them out.
pub(crate) struct Leading {
    /// No sample of the run has gone by yet. One whose decode failed has not: it is fed again.
    first: bool,
    skipping: bool,
}

impl Leading {
    pub(crate) fn new() -> Self {
        Self { first: true, skipping: false }
    }

    /// A new run begins at the next sample.
    pub(crate) fn reset(&mut self) {
        *self = Self::new();
    }

    /// Decodes the sample whose NAL unit types are `types` with `decode`, or leaves it out, with no
    /// pictures, when it is a RASL picture of a run that began at a CRA or BLA; then moves the run past it.
    /// Ask it of every sample in decoding order. A sample whose decode fails leaves the run where it was,
    /// because it is fed again and must get the same answer.
    pub(crate) fn pass<T, E>(&mut self, types: &[u8], decode: impl FnOnce() -> Result<Vec<T>, E>) -> Result<Vec<T>, E> {
        let pictures = if self.skips(types) { Vec::new() } else { decode()? };
        self.took(types);
        Ok(pictures)
    }

    /// Whether the sample whose NAL unit types are `types` is left out. It only asks.
    fn skips(&self, types: &[u8]) -> bool {
        // RASL_N and RASL_R.
        !is_irap(types) && self.skipping && types.iter().any(|kind| matches!(kind, 8 | 9))
    }

    /// Moves the run past the sample whose NAL unit types are `types`, decoded or left out.
    fn took(&mut self, types: &[u8]) {
        let first = std::mem::replace(&mut self.first, false);
        if is_irap(types) {
            // An IDR has no leading pictures that refer back, and a CRA in mid-stream has the pictures
            // before it to refer to.
            self.skipping = first && types.iter().any(|kind| matches!(kind, 16..=18 | 21));
        }
    }
}

/// What a platform's decoder does for [`Sequence`]: decodes one sample at a time.
pub(crate) trait RawDecoder<T> {
    /// Decodes `sample`, whose presentation time is `pts`, into the pictures it produced, each with its
    /// presentation time, in decoding order. An error means the sample left the decoder with no picture
    /// of it.
    fn feed(&mut self, sample: &[u8], pts: i64) -> Result<Vec<(i64, T)>, String>;

    /// The next sample starts a new decoding run, which refers to no picture an earlier one decoded.
    fn restart(&mut self);
}

/// A platform's raw decoder with the rules every hardware decoder shares: the RASL pictures of a run
/// starting at a CRA or BLA are never fed to it, and the pictures it produces come out in presentation
/// order.
pub(crate) struct Sequence<R, T> {
    raw: R,
    stream: StreamInfo,
    order: Reorder<T>,
    leading: Leading,
}

impl<R: RawDecoder<T>, T> Sequence<R, T> {
    /// `depth` is the stream's reorder depth.
    pub(crate) fn new(raw: R, stream: StreamInfo, depth: usize) -> Self {
        Self { raw, stream, order: Reorder::new(depth), leading: Leading::new() }
    }

    /// The pictures now due in presentation order after `sample`. A sample the raw decoder fails on adds
    /// nothing to the reorder buffer, which keeps what the fallback flushes, and leaves the run where it
    /// was, so fed again it is treated as it was.
    pub(crate) fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<T>, String> {
        let types = self.nal_types(sample);
        let (raw, order) = (&mut self.raw, &mut self.order);
        self.leading.pass(&types, || {
            let pictures = raw.feed(sample, pts)?;
            Ok(pictures.into_iter().flat_map(|(pts, picture)| order.push(pts, picture)).collect())
        })
    }

    /// The pictures still held, earliest first.
    pub(crate) fn flush(&mut self) -> Vec<T> {
        self.order.drain()
    }

    /// A seek: nothing decoded so far is referred to again.
    pub(crate) fn reset(&mut self) {
        self.raw.restart();
        self.order.clear();
        self.leading.reset();
    }

    /// The NAL unit types of an HEVC sample, which decide the RASL pictures a run leaves out; none for
    /// any other stream.
    fn nal_types(&self, sample: &[u8]) -> Vec<u8> {
        match &self.stream {
            StreamInfo::Nal(info) if info.codec == NalCodec::Hevc => info.nal_types(sample),
            _ => Vec::new(),
        }
    }
}

/// IRAP pictures are types 16 to 23: BLA (16 to 18), IDR (19, 20) and CRA (21), then two reserved.
fn is_irap(types: &[u8]) -> bool {
    types.iter().any(|kind| (16..=23).contains(kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::avc_entry;
    use filmcraft_codecs::hw::NalStreamInfo;

    /// Passes a sample whose decode succeeds through the run, as the decoder does; whether it was left out.
    fn feeds(leading: &mut Leading, types: &[u8]) -> bool {
        leading.pass(types, || Ok::<_, ()>(vec!["picture"])).unwrap().is_empty()
    }

    #[test]
    fn pictures_come_out_in_presentation_order_once_the_reorder_depth_is_held() {
        // I P B B in decoding order is presentation times 0 3 1 2, with two pictures that may be reordered.
        let mut order = Reorder::new(2);
        assert_eq!(order.push(0, "I"), Vec::<&str>::new());
        assert_eq!(order.push(3, "P"), Vec::<&str>::new());
        assert_eq!(order.push(1, "B1"), vec!["I"]);
        assert_eq!(order.push(2, "B2"), vec!["B1"]);
        assert_eq!(order.drain(), vec!["B2", "P"]);
        assert_eq!(order.drain(), Vec::<&str>::new());
    }

    #[test]
    fn a_stream_without_reordering_passes_every_picture_straight_through() {
        let mut order = Reorder::new(0);
        assert_eq!(order.push(5, "a"), vec!["a"]);
        assert_eq!(order.push(6, "b"), vec!["b"]);
    }

    #[test]
    fn pictures_of_one_presentation_time_keep_their_decoding_order_and_a_clear_forgets_them() {
        let mut order = Reorder::new(3);
        order.push(1, "first");
        order.push(1, "second");
        assert_eq!(order.drain(), vec!["first", "second"]);
        order.push(1, "gone");
        order.clear();
        assert_eq!(order.drain(), Vec::<&str>::new());
    }

    #[test]
    fn a_run_starting_at_a_cra_or_bla_leaves_out_the_rasl_pictures_after_it() {
        for start in [21, 16, 17, 18] {
            let mut leading = Leading::new();
            assert!(!feeds(&mut leading, &[start]), "the starting picture itself decodes");
            assert!(feeds(&mut leading, &[8]), "RASL_N after {start}");
            assert!(feeds(&mut leading, &[9]), "RASL_R after {start}");
            assert!(!feeds(&mut leading, &[6]), "a RADL or trailing picture decodes");
            assert!(!feeds(&mut leading, &[1]));
            assert!(!feeds(&mut leading, &[19]), "the next IRAP ends the skipping");
            assert!(!feeds(&mut leading, &[8]), "and its RASL pictures decode");
        }
    }

    #[test]
    fn an_idr_start_or_a_cra_in_mid_stream_leaves_nothing_out() {
        let mut idr = Leading::new();
        assert!(!feeds(&mut idr, &[19]));
        assert!(!feeds(&mut idr, &[9]));
        let mut mid = Leading::new();
        assert!(!feeds(&mut mid, &[19]));
        assert!(!feeds(&mut mid, &[1]));
        assert!(!feeds(&mut mid, &[21]), "a CRA that is not the run's first sample");
        assert!(!feeds(&mut mid, &[8]));
        // A reset starts a new run.
        mid.reset();
        assert!(!feeds(&mut mid, &[21]));
        assert!(feeds(&mut mid, &[8]));
    }

    #[test]
    fn a_leading_picture_before_any_irap_is_not_skipped_unless_a_cra_started_the_run() {
        let mut leading = Leading::new();
        assert!(!feeds(&mut leading, &[8]), "no CRA or BLA started this run");
    }

    #[test]
    fn a_sample_asked_about_twice_gets_the_same_answer_until_the_run_moves_on() {
        // The first sample of a run fails and is fed again: it is still the run's first.
        let mut leading = Leading::new();
        assert!(!leading.skips(&[21]));
        assert!(!leading.skips(&[21]));
        leading.took(&[21]);
        assert!(leading.skips(&[8]));
        assert!(leading.skips(&[8]), "asking changes nothing");
    }

    #[test]
    fn a_sample_whose_decode_fails_leaves_the_run_where_it_was() {
        // The run's first sample, a CRA, fails and is fed again: it is still the run's first, so the RASL
        // pictures after it are left out.
        let mut leading = Leading::new();
        assert_eq!(leading.pass(&[21], || Err::<Vec<()>, _>("the session failed")), Err("the session failed"));
        assert!(!feeds(&mut leading, &[21]), "fed again, the CRA is decoded");
        assert!(feeds(&mut leading, &[8]), "and its RASL pictures are left out");
        // A picture left out is never decoded, and a failure after a run is under way changes nothing either.
        assert_eq!(leading.pass(&[9], || -> Result<Vec<()>, &str> { panic!("decoded a RASL picture") }), Ok(Vec::new()));
        assert_eq!(leading.pass(&[1], || Err::<Vec<()>, _>("the session failed")), Err("the session failed"));
        assert!(feeds(&mut leading, &[8]), "still in the CRA's leading pictures");
    }

    /// A raw decoder that records what it is fed and fails on the samples it is told to; each sample's
    /// picture is its own last byte, at its presentation time.
    #[derive(Default)]
    struct Fake {
        fed: Vec<u8>,
        restarts: usize,
        failing: bool,
    }

    impl RawDecoder<u8> for Fake {
        fn feed(&mut self, sample: &[u8], pts: i64) -> Result<Vec<(i64, u8)>, String> {
            if self.failing {
                return Err("the session failed".into());
            }
            let marker = sample.last().copied().unwrap_or_default();
            self.fed.push(marker);
            Ok(vec![(pts, marker)])
        }

        fn restart(&mut self) {
            self.restarts += 1;
        }
    }

    /// An HEVC sample of one NAL unit of `kind`, led by its length, with `marker` as the picture's tag.
    fn hevc_sample(kind: u8, marker: u8) -> Vec<u8> {
        vec![0, 0, 0, 3, kind << 1, 1, marker]
    }

    fn hevc(depth: usize) -> Sequence<Fake, u8> {
        let mut info = NalStreamInfo::from_entry(&avc_entry()).unwrap().unwrap();
        info.codec = NalCodec::Hevc;
        Sequence::new(Fake::default(), StreamInfo::Nal(info), depth)
    }

    #[test]
    fn a_sequence_never_feeds_the_raw_decoder_a_rasl_picture_of_a_run_that_starts_at_a_cra() {
        let mut sequence = hevc(0);
        assert_eq!(sequence.decode(&hevc_sample(21, 1), 0), Ok(vec![1]));
        assert_eq!(sequence.decode(&hevc_sample(8, 2), 1), Ok(Vec::new()));
        assert_eq!(sequence.decode(&hevc_sample(1, 3), 2), Ok(vec![3]));
        assert_eq!(sequence.raw.fed, vec![1, 3]);
        sequence.reset();
        assert_eq!(sequence.raw.restarts, 1, "a seek restarts the raw decoder");
        assert_eq!(sequence.decode(&hevc_sample(21, 4), 0), Ok(vec![4]));
        assert_eq!(sequence.decode(&hevc_sample(9, 5), 1), Ok(Vec::new()), "a new run, started at a CRA again");
    }

    #[test]
    fn a_sequence_whose_raw_decoder_fails_the_first_cra_still_leaves_out_its_rasl_pictures_when_it_is_fed_again() {
        let mut sequence = hevc(0);
        sequence.raw.failing = true;
        assert_eq!(sequence.decode(&hevc_sample(21, 1), 0), Err("the session failed".into()));
        sequence.raw.failing = false;
        assert_eq!(sequence.decode(&hevc_sample(21, 1), 0), Ok(vec![1]));
        assert_eq!(sequence.decode(&hevc_sample(8, 2), 1), Ok(Vec::new()));
        assert_eq!(sequence.raw.fed, vec![1]);
    }

    #[test]
    fn a_sequence_puts_pictures_in_presentation_order_and_a_failed_sample_adds_none() {
        let mut sequence = hevc(1);
        assert_eq!(sequence.decode(&hevc_sample(19, 10), 0), Ok(Vec::new()));
        assert_eq!(sequence.decode(&hevc_sample(1, 30), 3), Ok(vec![10]));
        sequence.raw.failing = true;
        assert!(sequence.decode(&hevc_sample(1, 20), 1).is_err());
        sequence.raw.failing = false;
        assert_eq!(sequence.flush(), vec![30], "the failed sample left nothing in the buffer");
        sequence.decode(&hevc_sample(1, 40), 4).unwrap();
        sequence.reset();
        assert_eq!(sequence.flush(), Vec::<u8>::new(), "a seek forgets what was held");
    }

    #[test]
    fn a_sequence_reads_nal_types_only_in_an_hevc_stream() {
        let mut info = NalStreamInfo::from_entry(&avc_entry()).unwrap().unwrap();
        info.codec = NalCodec::H264;
        let mut sequence = Sequence::new(Fake::default(), StreamInfo::Nal(info), 0);
        // The H.264 NAL unit types 21 and 8 are the numbers HEVC gives a CRA and a RASL picture. An H.264 stream has
        // neither, so none is left out.
        sequence.decode(&[0, 0, 0, 3, 21, 1, 1], 0).unwrap();
        assert_eq!(sequence.decode(&[0, 0, 0, 3, 8, 1, 2], 1), Ok(vec![2]));
    }
}
