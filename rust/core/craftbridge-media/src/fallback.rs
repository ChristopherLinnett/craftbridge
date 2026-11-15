//! A hardware decoder that hands over to the software decoder when it fails mid-stream.
//!
//! The software decoders are the reference (plan D-14): a stream a hardware decoder cannot finish is
//! finished in software, from a sample decoding can restart at, so the pictures the caller sees never
//! skip and never repeat. FilmCraft's own wrapper, `filmcraft_platform::HybridDecoder`, does this for
//! FilmCraft's hardware decoders. It is not used because its crate always depends on `filmcraft-export`,
//! which brings its H.264 and FLAC encoders and FilmCraft's compositor, text, caption and audio crates,
//! none of which a decoder needs, into the phone build: 25 more crates in the core's own graph, and nine
//! even in a plugin library whose app already uses the rest, for about 60 KB of wrapper once linked. This
//! one follows its behaviour in code of its own.

use std::collections::{BTreeMap, BTreeSet};

use filmcraft_codecs::hw::{FrameStreamInfo, NalStreamInfo, StreamInfo, note_hw_fallback, note_hw_frames, note_hw_session};
use filmcraft_codecs::{CodecError, DecodedFrame, Result, VideoDecoder, software_video_decoder};
use filmcraft_isobmff::SampleEntry;

/// How much a replay may hold and produce. It runs inside one decode call, so its length is how long
/// that call takes: at most 600 samples, two ten-second GOPs at 30 pictures a second (after an HEVC CRA
/// the GOP before it is kept too), and 256 MiB; FilmCraft's hybrid decoder keeps the same two bounds.
/// The pictures bound is for a corrupt access unit, which can make one sample decode to many pictures,
/// and so turn a bounded replay into an unbounded queue; FilmCraft's hybrid decoder bounds that too.
const BOUNDS: Bounds = Bounds { samples: 600, bytes: 256 << 20, pictures: 4096 };

#[derive(Clone, Copy)]
struct Bounds {
    samples: usize,
    bytes: usize,
    pictures: usize,
}

/// What a sample means for the replay.
#[derive(Clone, Copy)]
enum Restart {
    /// Decoding cannot start at it.
    No,
    /// Decoding can start at it with no earlier state: an H.264 IDR, an HEVC IDR or BLA, a VP9 or AV1
    /// key frame.
    Clean,
    /// An HEVC CRA. Its leading (RASL) pictures follow it in decode order and refer to the GOP before it,
    /// which a decoder that starts at the CRA has not seen, so it leaves them out.
    KeepsPrevious,
}

impl Restart {
    fn of(stream: &StreamInfo, sample: &[u8]) -> Self {
        if !stream.is_irap(sample) {
            Self::No
        } else if stream.keeps_previous_restart(sample) {
            Self::KeepsPrevious
        } else {
            Self::Clean
        }
    }
}

/// A sample kept for a replay.
struct Kept {
    pts: i64,
    sample: Vec<u8>,
    /// The caller already has this sample's picture.
    delivered: bool,
}

/// The samples, in decode order, a software decoder needs to reach the hardware decoder's state.
struct Replay {
    /// From the latest restart sample, or from the one before it when the latest is a CRA; `None` once a
    /// bound was passed, until the next restart sample.
    kept: Option<Vec<Kept>>,
    bytes: usize,
    /// Where the latest restart sample sits in `kept`.
    latest: usize,
    bounds: Bounds,
}

impl Replay {
    /// Nothing kept yet: the first sample fed is where a replay starts, as decoding does.
    fn new(bounds: Bounds) -> Self {
        Self { kept: Some(Vec::new()), bytes: 0, latest: 0, bounds }
    }

    fn clear(&mut self) {
        *self = Self::new(self.bounds);
    }

    fn keep(&mut self, stream: &StreamInfo, sample: &[u8], pts: i64) {
        match (Restart::of(stream, sample), self.kept.as_mut()) {
            (Restart::No, _) => {}
            (Restart::KeepsPrevious, Some(kept)) => {
                let dropped = kept.drain(..self.latest.min(kept.len())).fold(0, |bytes: usize, gone| bytes.saturating_add(gone.sample.len()));
                self.bytes = self.bytes.saturating_sub(dropped);
                self.latest = kept.len();
            }
            // A clean restart needs nothing before it. A CRA past the bounds has lost the GOP before it, and
            // is still the best start left: only its leading pictures are missing from a replay.
            (Restart::Clean | Restart::KeepsPrevious, _) => self.clear(),
        }
        let Some(kept) = self.kept.as_mut() else { return };
        let bytes = self.bytes.saturating_add(sample.len());
        if kept.len() >= self.bounds.samples || bytes > self.bounds.bytes {
            self.kept = None;
            return;
        }
        self.bytes = bytes;
        kept.push(Kept { pts, sample: sample.to_vec(), delivered: false });
    }

    /// Notes that the caller has the picture with this presentation time, which is its sample's.
    fn delivered(&mut self, pts: i64) {
        if let Some(kept) = self.kept.as_mut().and_then(|kept| kept.iter_mut().rev().find(|kept| kept.pts == pts)) {
            kept.delivered = true;
        }
    }

    fn take(&mut self) -> Option<Vec<Kept>> {
        let kept = self.kept.take();
        self.clear();
        kept
    }
}

enum Engine<D> {
    Hardware(D),
    Software(Box<dyn VideoDecoder>),
}

/// Makes the software decoder the hardware one hands over to.
type SoftwareMaker = Box<dyn Fn() -> Result<Box<dyn VideoDecoder>> + Send>;

/// Wraps the hardware decoder `D`. When `D` fails, or a sample carries parameters other than the ones
/// `D` was set up with, it switches to the software decoder and replays the samples since the last
/// restart sample, so the caller sees one continuous stream: the pictures it already has are left out,
/// and the ones `D` still held come out in presentation order.
///
/// `D` reports [`CodecError::Cancelled`] only for a sample it has not taken: FilmCraft's decode loop
/// feeds a sample again after any error.
pub struct Fallback<D> {
    engine: Engine<D>,
    stream: StreamInfo,
    software: SoftwareMaker,
    replay: Replay,
    /// After the switch: presentation times the caller has that the software decoder decodes again.
    delivered: BTreeSet<i64>,
    /// After the switch: pictures the caller does not have yet, until the software decoder's output
    /// passes them: the ones the hardware decoder still held, and those of a replay whose last sample
    /// failed.
    held: BTreeMap<i64, DecodedFrame>,
    draft: bool,
}

impl<D: VideoDecoder> Fallback<D> {
    /// Wraps `hardware`, a decoder for the stream `entry` describes; `stream` is what FilmCraft's
    /// hardware-decoder support reads from `entry`, and the decoder switched to is FilmCraft's software
    /// decoder for `entry`.
    pub fn new(hardware: D, entry: &SampleEntry, stream: StreamInfo) -> Self {
        let entry = entry.clone();
        Self::with_software(hardware, stream, move || software_video_decoder(&entry), BOUNDS)
    }

    fn with_software(hardware: D, stream: StreamInfo, software: impl Fn() -> Result<Box<dyn VideoDecoder>> + Send + 'static, bounds: Bounds) -> Self {
        note_hw_session();
        Self {
            engine: Engine::Hardware(hardware),
            stream,
            software: Box::new(software),
            replay: Replay::new(bounds),
            delivered: BTreeSet::new(),
            held: BTreeMap::new(),
            draft: false,
        }
    }

    fn active(&self) -> &dyn VideoDecoder {
        match &self.engine {
            Engine::Hardware(hardware) => hardware,
            Engine::Software(software) => software.as_ref(),
        }
    }

    fn active_mut(&mut self) -> &mut dyn VideoDecoder {
        match &mut self.engine {
            Engine::Hardware(hardware) => hardware,
            Engine::Software(software) => software.as_mut(),
        }
    }

    /// Notes the hardware decoder's pictures as the caller's, so a replay leaves them out.
    fn delivered(&mut self, pictures: Vec<DecodedFrame>) -> Vec<DecodedFrame> {
        note_hw_frames(pictures.len());
        for picture in &pictures {
            self.replay.delivered(picture.pts);
        }
        pictures
    }

    /// Switches to the software decoder after `failure` on `sample`, replays the kept samples and that
    /// one, and returns the pictures the caller does not have yet.
    fn switch(&mut self, failure: CodecError, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        // Without a software decoder the hardware one stays, and the sample is not kept: the caller
        // feeds it again.
        let Ok(mut software) = (self.software)() else { return Err(failure) };
        software.set_draft(self.draft);
        note_hw_fallback();
        self.replay.keep(&self.stream, sample, pts);
        let still_held = self.active_mut().flush();
        let Some(replay) = self.replay.take() else {
            // Too far from a restart sample to replay: the software decoder starts at the next one, or
            // at the caller's next seek.
            self.engine = Engine::Software(software);
            return Err(failure);
        };
        self.engine = Engine::Software(software);
        self.delivered = replay.iter().filter(|kept| kept.delivered).map(|kept| kept.pts).collect();
        self.held = still_held.into_iter().map(|picture| (picture.pts, picture)).collect();
        let last = replay.len().saturating_sub(1);
        // A replay's pictures are merged only once the whole replay has run, as FilmCraft's hybrid does: until
        // then they are the replay's own output, so any error can drop them without touching what the caller
        // has or what the failed hardware decoder held.
        let mut frames = Vec::new();
        let mut rejected = None;
        let mut produced = 0usize;
        for (index, kept) in replay.iter().enumerate() {
            match self.active_mut().decode(&kept.sample, kept.pts) {
                Ok(out) => {
                    produced = produced.saturating_add(out.len());
                    if produced > self.replay.bounds.pictures {
                        // The replay's own pictures go with the error, as a corrupt stream's do. What the
                        // caller has, and what the failed hardware decoder held, stay: the software decoder
                        // can still return pictures the caller has, and they must not come again.
                        return Err(CodecError::Decode("the software decoder produced too many pictures replaying the stream".into()));
                    }
                    frames.extend(out);
                }
                Err(error) if index == last => rejected = Some(error),
                // The caller had this sample's answer long ago, and software decoding from the restart
                // would have gone on past it too.
                Err(_) => {}
            }
        }
        match rejected {
            None => Ok(self.merge(frames)),
            Some(error) => {
                // The failed sample's own error is this call's; the replay's pictures, except those the
                // caller already has, wait in the held map and come out in order with the next calls, among
                // the ones the failed hardware decoder held and never among the pictures the caller has.
                for picture in frames {
                    if !self.delivered.contains(&picture.pts) {
                        self.held.insert(picture.pts, picture);
                    }
                }
                Err(error)
            }
        }
    }

    /// The software decoder's pictures after the switch, with the ones the caller has left out and each
    /// held one put before the first picture that follows it in presentation order (a software picture
    /// replaces a held one with its presentation time).
    fn merge(&mut self, pictures: Vec<DecodedFrame>) -> Vec<DecodedFrame> {
        let mut out = Vec::with_capacity(pictures.len());
        for picture in pictures {
            if self.delivered.remove(&picture.pts) {
                continue;
            }
            while let Some(held) = self.held.first_entry() {
                if *held.key() >= picture.pts {
                    break;
                }
                out.push(held.remove());
            }
            self.held.remove(&picture.pts);
            out.push(picture);
        }
        out
    }
}

/// What FilmCraft's hardware-decoder support reads from the stream `entry` describes; `None` for a
/// stream no hardware decoder takes.
pub(crate) fn stream_info(entry: &SampleEntry) -> Option<Result<StreamInfo>> {
    match NalStreamInfo::from_entry(entry) {
        Some(nal) => Some(nal.map(StreamInfo::from)),
        None => FrameStreamInfo::from_entry(entry).map(|frame| frame.map(StreamInfo::from)),
    }
}

impl<D: VideoDecoder> VideoDecoder for Fallback<D> {
    fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
        let outcome = match &mut self.engine {
            Engine::Software(software) => {
                let pictures = software.decode(sample, pts)?;
                return Ok(self.merge(pictures));
            }
            // The hardware session was set up from the sample entry's parameters and cannot decode past
            // in-band ones that differ.
            Engine::Hardware(_) if self.stream.parameters_changed(sample) => {
                Err(CodecError::Decode("in-band parameters differ from the sample entry's".into()))
            }
            Engine::Hardware(hardware) => hardware.decode(sample, pts),
        };
        match outcome {
            Ok(pictures) => {
                self.replay.keep(&self.stream, sample, pts);
                Ok(self.delivered(pictures))
            }
            // The caller no longer wants the picture: nothing failed, and the sample is not kept, since
            // the caller feeds it again when it does.
            Err(CodecError::Cancelled) => Err(CodecError::Cancelled),
            Err(failure) => self.switch(failure, sample, pts),
        }
    }

    fn flush(&mut self) -> Vec<DecodedFrame> {
        let pictures = self.active_mut().flush();
        if matches!(self.engine, Engine::Hardware(_)) {
            return self.delivered(pictures);
        }
        let mut out = self.merge(pictures);
        out.extend(std::mem::take(&mut self.held).into_values());
        out
    }

    fn reset(&mut self) {
        self.replay.clear();
        self.delivered.clear();
        self.held.clear();
        self.active_mut().reset();
    }

    fn name(&self) -> &str {
        self.active().name()
    }

    fn intra_only(&self) -> bool {
        self.active().intra_only()
    }

    // Answered from the stream, as the software decoder answers them, so seeking and catching up treat
    // the stream alike in hardware and in software.
    fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
        self.stream.is_random_access(sample)
    }

    fn is_disposable(&self, sample: &[u8]) -> bool {
        self.stream.is_disposable(sample)
    }

    fn set_draft(&mut self, on: bool) {
        self.draft = on;
        self.active_mut().set_draft(on);
    }

    fn thread_limited(&self) -> bool {
        self.active().thread_limited()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use filmcraft_codecs::hw::NalCodec;
    use filmcraft_isobmff::{CodecConfig, FourCc};

    use super::*;
    use crate::fixtures::{avc_entry, pps, prores_entry, sps};

    // The last byte of a test sample tells the fakes what to do with it.
    const PLAIN: u8 = 0;
    /// The hardware fake fails.
    const FAIL: u8 = 0xEE;
    /// The software fake rejects it.
    const REJECT: u8 = 0xDD;
    /// Both fakes fail.
    const BOTH: u8 = 0xBB;
    /// The hardware fake reports it cancelled the first time it is fed.
    const CANCEL: u8 = 0xCC;

    // H.264 NAL headers: an IDR slice, a slice other pictures refer to, and one none refers to.
    const IDR: &[u8] = &[0x65];
    const P: &[u8] = &[0x41];
    const B: &[u8] = &[0x01];
    // HEVC NAL headers (type, then TemporalId + 1): IDR_W_RADL, CRA, TRAIL_R.
    const HEVC_IDR: &[u8] = &[19 << 1, 1];
    const CRA: &[u8] = &[21 << 1, 1];
    const TRAIL: &[u8] = &[1 << 1, 1];

    /// A length-prefixed access unit of the NAL units `nals`.
    fn access_unit(nals: &[&[u8]]) -> Vec<u8> {
        nals.iter().flat_map(|nal| u32::try_from(nal.len()).unwrap().to_be_bytes().into_iter().chain(nal.iter().copied())).collect()
    }

    /// A one-NAL sample with this header and last byte, at presentation time `pts`.
    fn at(pts: i64, header: &[u8], tag: u8) -> (i64, Vec<u8>) {
        (pts, access_unit(&[&[header, &[tag]].concat()]))
    }

    /// A one-picture Motion-JPEG stream: FilmCraft's own decoder turns it into a real `DecodedFrame`,
    /// the one thing the fakes below cannot build themselves.
    fn tiny_jpeg() -> Vec<u8> {
        let huffman = |class: u8| [&[0xFF, 0xC4, 0x00, 0x14, class, 1][..], &[0; 15], &[0]].concat();
        [
            &[0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43, 0x00][..],
            &[1; 64],
            &[0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x08, 0x00, 0x08, 0x01, 0x01, 0x11, 0x00],
            &huffman(0x00),
            &huffman(0x10),
            &[0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0x3F, 0xFF, 0xD9],
        ]
        .concat()
    }

    fn picture(pts: i64) -> DecodedFrame {
        let entry = SampleEntry::video(FourCc(*b"jpeg"), CodecConfig::Jpeg { fourcc: FourCc(*b"jpeg") }, 8, 8);
        let mut decoder = software_video_decoder(&entry).unwrap();
        let mut frame = decoder.decode(&tiny_jpeg(), pts).unwrap().remove(0);
        frame.pts = pts;
        frame
    }

    fn avc() -> StreamInfo {
        NalStreamInfo::from_entry(&avc_entry()).unwrap().unwrap().into()
    }

    /// FilmCraft's HEVC rules over the same record: only the samples' NAL unit types matter here.
    fn hevc() -> StreamInfo {
        let mut info = NalStreamInfo::from_entry(&avc_entry()).unwrap().unwrap();
        info.codec = NalCodec::Hevc;
        info.into()
    }

    type Fed = Arc<Mutex<Vec<i64>>>;

    /// Returns one picture per sample, `latency` samples late (a flush returns the rest), and fails on a
    /// sample whose last byte is in `fails_on`.
    struct Fake {
        name: &'static str,
        fails_on: &'static [u8],
        latency: usize,
        cancels: bool,
        /// Loses the pictures it held when it fails, as a hardware decoder whose session is lost does.
        forgets: bool,
        pending: VecDeque<i64>,
        fed: Fed,
        drafted: Arc<Mutex<Vec<bool>>>,
    }

    impl VideoDecoder for Fake {
        fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
            self.fed.lock().unwrap().push(pts);
            let tag = sample.last().copied().unwrap_or(PLAIN);
            if self.fails_on.contains(&tag) {
                if self.forgets {
                    self.pending.clear();
                }
                return Err(CodecError::Decode("injected".into()));
            }
            if tag == CANCEL && self.cancels {
                self.cancels = false;
                return Err(CodecError::Cancelled);
            }
            self.pending.push_back(pts);
            let ready = self.pending.len().saturating_sub(self.latency);
            Ok(self.pending.drain(..ready).map(picture).collect())
        }
        fn flush(&mut self) -> Vec<DecodedFrame> {
            self.pending.drain(..).map(picture).collect()
        }
        fn reset(&mut self) {
            self.pending.clear();
        }
        fn set_draft(&mut self, on: bool) {
            self.drafted.lock().unwrap().push(on);
        }
        fn name(&self) -> &str {
            self.name
        }
    }

    fn fake(name: &'static str, fails_on: &'static [u8], latency: usize) -> Fake {
        Fake { name, fails_on, latency, cancels: name == "hardware", forgets: false, pending: VecDeque::new(), fed: Fed::default(), drafted: Arc::default() }
    }

    /// A rig: the fakes' latencies, the stream, the attempt from which the software decoder can be made,
    /// and the replay's bounds.
    struct Setup {
        hardware: usize,
        software: usize,
        stream: StreamInfo,
        available_from: u32,
        bounds: Bounds,
        /// The hardware fake loses the pictures it held when it fails.
        forgets: bool,
    }

    impl Default for Setup {
        fn default() -> Self {
            Self { hardware: 0, software: 0, stream: avc(), available_from: 1, bounds: BOUNDS, forgets: false }
        }
    }

    impl Setup {
        fn rig(self) -> Rig {
            let hardware = Fake { forgets: self.forgets, ..fake("hardware", &[FAIL, BOTH], self.hardware) };
            let software = fake("software", &[REJECT, BOTH], self.software);
            let (hardware_fed, software_fed) = (Arc::clone(&hardware.fed), Arc::clone(&software.fed));
            let software_draft = Arc::clone(&software.drafted);
            let software = Mutex::new(Some(software));
            let made = Arc::new(Mutex::new(0));
            let attempts = Arc::clone(&made);
            let available_from = self.available_from;
            let make = move || {
                let mut attempts = attempts.lock().unwrap();
                *attempts += 1;
                if *attempts < available_from {
                    return Err(CodecError::Unsupported("no software decoder yet".into()));
                }
                let software = software.lock().unwrap().take().expect("the software decoder is made once");
                Ok(Box::new(software) as Box<dyn VideoDecoder>)
            };
            Rig {
                decoder: Fallback::with_software(hardware, self.stream, make, self.bounds),
                hardware: hardware_fed,
                software: software_fed,
                software_draft,
                made,
            }
        }
    }

    /// What a call returned: its pictures' presentation times, or that it failed or was cancelled.
    type Outcome = std::result::Result<Vec<i64>, &'static str>;

    struct Rig {
        decoder: Fallback<Fake>,
        hardware: Fed,
        software: Fed,
        software_draft: Arc<Mutex<Vec<bool>>>,
        made: Arc<Mutex<u32>>,
    }

    impl Rig {
        fn feed(&mut self, samples: &[(i64, Vec<u8>)]) -> Vec<Outcome> {
            samples
                .iter()
                .map(|(pts, sample)| match self.decoder.decode(sample, *pts) {
                    Ok(pictures) => Ok(pictures.iter().map(|p| p.pts).collect()),
                    Err(CodecError::Cancelled) => Err("cancelled"),
                    Err(_) => Err("failed"),
                })
                .collect()
        }
        fn flush(&mut self) -> Vec<i64> {
            self.decoder.flush().iter().map(|p| p.pts).collect()
        }
        fn fed_to_hardware(&self) -> Vec<i64> {
            self.hardware.lock().unwrap().clone()
        }
        fn fed_to_software(&self) -> Vec<i64> {
            self.software.lock().unwrap().clone()
        }
        fn software_drafts(&self) -> Vec<bool> {
            self.software_draft.lock().unwrap().clone()
        }
        fn made(&self) -> u32 {
            *self.made.lock().unwrap()
        }
        /// Feeds two GOPs whose fifth sample fails the hardware decoder, and returns the calls with
        /// every picture the caller ended up with. A hardware decoder two samples late still holds 2 and 3
        /// when it fails.
        fn fail_in_the_second_gop(&mut self) -> (Vec<Outcome>, Vec<i64>) {
            let outcomes = self.feed(&[at(0, IDR, PLAIN), at(1, P, PLAIN), at(2, IDR, PLAIN), at(3, P, PLAIN), at(4, P, FAIL), at(5, P, PLAIN)]);
            let mut seen = pictures(&outcomes);
            seen.extend(self.flush());
            (outcomes, seen)
        }
    }

    /// Every picture the calls returned, in order.
    fn pictures(outcomes: &[Outcome]) -> Vec<i64> {
        outcomes.iter().flat_map(|outcome| outcome.clone().unwrap_or_default()).collect()
    }

    #[test]
    fn an_error_switches_to_software_and_replays_from_the_last_random_access_sample() {
        let mut rig = Setup::default().rig();
        let out = rig.feed(&[at(0, IDR, PLAIN), at(1, P, PLAIN), at(2, P, PLAIN), at(3, IDR, PLAIN), at(4, P, PLAIN), at(5, P, FAIL), at(6, P, PLAIN)]);
        // The samples since the IDR at 3, the failing one included, and none from before it.
        assert_eq!(rig.fed_to_software(), vec![3, 4, 5, 6]);
        // Pictures 3 and 4 came from hardware already, so the switch returns only the failed sample's.
        assert_eq!(out, (0..7).map(|pts| Ok(vec![pts])).collect::<Vec<_>>());
        assert_eq!(rig.decoder.name(), "software");
        assert_eq!(rig.made(), 1);
        assert_eq!(rig.fed_to_hardware(), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn without_an_error_hardware_decodes_every_sample_and_software_is_never_made() {
        let mut rig = Setup::default().rig();
        let out = rig.feed(&[at(0, P, PLAIN), at(1, P, PLAIN), at(2, IDR, PLAIN), at(3, P, PLAIN)]);
        assert_eq!(pictures(&out), vec![0, 1, 2, 3]);
        assert_eq!(rig.fed_to_hardware().len(), 4);
        assert_eq!(rig.made(), 0);
        assert!(rig.fed_to_software().is_empty());
        assert_eq!(rig.decoder.name(), "hardware");
    }

    #[test]
    fn after_an_hevc_cra_the_replay_starts_at_the_restart_before_it() {
        let mut rig = Setup { stream: hevc(), ..Setup::default() }.rig();
        let out = rig.feed(&[
            at(0, HEVC_IDR, PLAIN),
            at(1, TRAIL, PLAIN),
            at(2, CRA, PLAIN),
            at(3, TRAIL, PLAIN),
            at(4, CRA, PLAIN),
            at(5, TRAIL, PLAIN),
            at(6, TRAIL, FAIL),
        ]);
        // The CRA at 4 restarts nothing by itself: its leading pictures refer to the GOP of the CRA at 2.
        // That one restarts the replay, so a stream of CRAs keeps two GOPs, never the whole movie.
        assert_eq!(rig.fed_to_software(), vec![2, 3, 4, 5, 6]);
        assert_eq!(pictures(&out), vec![0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn pictures_the_software_decoder_returns_late_are_left_out_when_the_caller_has_them() {
        let mut rig = Setup { software: 2, ..Setup::default() }.rig();
        let out = rig.feed(&[at(0, IDR, PLAIN), at(1, P, PLAIN), at(2, P, PLAIN), at(3, P, FAIL), at(4, P, PLAIN), at(5, P, PLAIN)]);
        let mut seen = pictures(&out);
        seen.extend(rig.flush());
        // Every picture once, in order, though the software decoder returns 2 again after the switch.
        assert_eq!(seen, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn pictures_the_hardware_decoder_still_held_come_out_in_presentation_order() {
        let mut rig = Setup { hardware: 2, ..Setup::default() }.rig();
        let out = rig.feed(&[at(0, IDR, PLAIN), at(1, P, PLAIN), at(2, IDR, PLAIN), at(3, P, FAIL)]);
        // The replay starts at the IDR at 2: picture 1 was only ever the hardware decoder's.
        assert_eq!(rig.fed_to_software(), vec![2, 3]);
        assert_eq!(out, vec![Ok(vec![]), Ok(vec![]), Ok(vec![0]), Ok(vec![1, 2, 3])]);
    }

    #[test]
    fn the_replay_goes_on_past_a_sample_the_software_decoder_rejects() {
        let mut rig = Setup::default().rig();
        let out = rig.feed(&[at(0, IDR, PLAIN), at(1, P, REJECT), at(2, P, PLAIN), at(3, P, FAIL), at(4, P, PLAIN)]);
        assert_eq!(rig.fed_to_software(), vec![0, 1, 2, 3, 4]);
        assert_eq!(out, (0..5).map(|pts| Ok(vec![pts])).collect::<Vec<_>>());
    }

    #[test]
    fn when_the_software_decoder_rejects_the_failed_sample_too_its_error_comes_back_and_the_pictures_follow() {
        // Picture 1 follows whether the failed hardware decoder still held it or lost it with its session,
        // when the replay's is the only one left.
        for forgets in [false, true] {
            let mut rig = Setup { hardware: 1, forgets, ..Setup::default() }.rig();
            let out = rig.feed(&[at(0, IDR, PLAIN), at(1, P, PLAIN), at(2, P, BOTH), at(3, P, PLAIN)]);
            assert_eq!(out, vec![Ok(vec![]), Ok(vec![0]), Err("failed"), Ok(vec![1, 3])], "forgets {forgets}");
        }
    }

    #[test]
    fn a_failure_on_a_restart_sample_replays_that_sample_alone() {
        let mut rig = Setup::default().rig();
        let out = rig.feed(&[at(0, IDR, PLAIN), at(1, P, PLAIN), at(2, IDR, FAIL), at(3, P, PLAIN)]);
        assert_eq!(rig.fed_to_software(), vec![2, 3]);
        assert_eq!(pictures(&out), vec![0, 1, 2, 3]);
    }

    #[test]
    fn a_failure_on_the_first_sample_replays_it_whatever_it_is() {
        let mut rig = Setup::default().rig();
        assert_eq!(rig.feed(&[at(0, P, FAIL), at(1, P, PLAIN)]), vec![Ok(vec![0]), Ok(vec![1])]);
        assert_eq!(rig.fed_to_software(), vec![0, 1]);
    }

    #[test]
    fn after_the_switch_a_software_error_passes_through_and_nothing_switches_again() {
        let mut rig = Setup::default().rig();
        let out = rig.feed(&[at(0, IDR, PLAIN), at(1, P, FAIL), at(2, P, REJECT), at(3, P, FAIL)]);
        // FAIL is the hardware fake's failure, which the software decoder decodes.
        assert_eq!(out, vec![Ok(vec![0]), Ok(vec![1]), Err("failed"), Ok(vec![3])]);
        assert_eq!(rig.made(), 1);
        assert_eq!(rig.fed_to_hardware(), vec![0, 1]);
    }

    #[test]
    fn a_cancelled_decode_is_not_a_failure_and_its_sample_is_kept_once() {
        let mut rig = Setup::default().rig();
        let out = rig.feed(&[at(0, IDR, PLAIN), at(1, P, CANCEL)]);
        assert_eq!(out, vec![Ok(vec![0]), Err("cancelled")]);
        assert_eq!((rig.made(), rig.decoder.name()), (0, "hardware"));
        // FilmCraft feeds a cancelled sample again when the picture is wanted again.
        let out = rig.feed(&[at(1, P, CANCEL), at(2, P, FAIL)]);
        assert_eq!(out, vec![Ok(vec![1]), Ok(vec![2])]);
        assert_eq!(rig.fed_to_software(), vec![0, 1, 2]);
    }

    #[test]
    fn without_a_software_decoder_the_error_comes_back_the_hardware_decoder_stays_and_the_sample_is_not_kept() {
        let mut rig = Setup { available_from: 2, ..Setup::default() }.rig();
        assert_eq!(rig.feed(&[at(0, IDR, PLAIN), at(1, P, FAIL)]), vec![Ok(vec![0]), Err("failed")]);
        assert_eq!(rig.decoder.name(), "hardware");
        // Fed again, the sample hands over, and the replay has it once.
        assert_eq!(rig.feed(&[at(1, P, FAIL)]), vec![Ok(vec![1])]);
        assert_eq!(rig.fed_to_software(), vec![0, 1]);
        assert_eq!(rig.made(), 2);
    }

    #[test]
    fn past_either_replay_bound_the_switch_has_nothing_to_replay_and_reports_the_error() {
        let length = at(0, IDR, PLAIN).1.len();
        for bounds in [Bounds { samples: 2, bytes: usize::MAX, ..BOUNDS }, Bounds { bytes: 2 * length, ..BOUNDS }] {
            let mut rig = Setup { bounds, ..Setup::default() }.rig();
            let out = rig.feed(&[at(0, IDR, PLAIN), at(1, P, PLAIN), at(2, P, PLAIN), at(3, P, FAIL), at(4, IDR, PLAIN)]);
            // The software decoder takes over at the next restart sample.
            assert_eq!(out, vec![Ok(vec![0]), Ok(vec![1]), Ok(vec![2]), Err("failed"), Ok(vec![4])]);
            assert_eq!(rig.fed_to_software(), vec![4]);
            assert_eq!(rig.decoder.name(), "software");
        }
    }

    #[test]
    fn a_replay_that_produces_too_many_pictures_is_an_error_and_the_stream_goes_on_in_software() {
        let mut rig = Setup { bounds: Bounds { pictures: 2, ..BOUNDS }, ..Setup::default() }.rig();
        let out = rig.feed(&[at(0, IDR, PLAIN), at(1, P, PLAIN), at(2, P, PLAIN), at(3, P, FAIL), at(4, P, PLAIN)]);
        assert_eq!(out, vec![Ok(vec![0]), Ok(vec![1]), Ok(vec![2]), Err("failed"), Ok(vec![4])]);
        assert_eq!(rig.decoder.name(), "software");
    }

    #[test]
    fn the_replay_bounds_are_the_amounts_filmcrafts_hybrid_decoder_keeps() {
        // The other bound tests set amounts of their own; these are the ones AD8 records as FilmCraft's
        // hybrid decoder's.
        assert_eq!((BOUNDS.samples, BOUNDS.bytes, BOUNDS.pictures), (600, 256 << 20, 4096));
    }

    #[test]
    fn each_replay_bound_holds_exactly_its_amount_as_filmcrafts_hybrid_decoder_does() {
        let length = at(0, IDR, PLAIN).1.len();
        // With the failed sample the replay is four samples of `length` bytes, decoding to four pictures.
        let stream = [at(0, IDR, PLAIN), at(1, P, PLAIN), at(2, P, PLAIN), at(3, P, FAIL)];
        for (fits, short) in [
            (Bounds { samples: 4, ..BOUNDS }, Bounds { samples: 3, ..BOUNDS }),
            (Bounds { bytes: 4 * length, ..BOUNDS }, Bounds { bytes: 4 * length - 1, ..BOUNDS }),
            (Bounds { pictures: 4, ..BOUNDS }, Bounds { pictures: 3, ..BOUNDS }),
        ] {
            assert_eq!(Setup { bounds: fits, ..Setup::default() }.rig().feed(&stream).last(), Some(&Ok(vec![3])));
            assert_eq!(Setup { bounds: short, ..Setup::default() }.rig().feed(&stream).last(), Some(&Err("failed")));
        }
    }

    #[test]
    fn after_a_replay_that_produces_too_many_pictures_none_repeats_and_the_held_ones_still_come() {
        let mut rig = Setup { software: 2, bounds: Bounds { pictures: 1, ..BOUNDS }, ..Setup::default() }.rig();
        let out = rig.feed(&[at(0, IDR, PLAIN), at(1, P, PLAIN), at(2, P, PLAIN), at(3, P, PLAIN), at(4, P, FAIL), at(5, P, PLAIN), at(6, P, PLAIN)]);
        assert_eq!(out.get(4), Some(&Err("failed")));
        let mut seen = pictures(&out);
        seen.extend(rig.flush());
        // The software decoder returns 2 and 3 after the error; the caller had them from hardware. Picture 4
        // goes with the error.
        assert_eq!(seen, vec![0, 1, 2, 3, 5, 6]);
        // The hardware decoder still held 2 and 3 when it failed: they come after the error, whether the
        // cap trips before the software decoder returned a picture or after it returned one during the
        // replay. FilmCraft's hybrid merges after the whole replay, so an error drops only the replay's
        // own output: a replay merged as it runs would lose the software picture for 2 at the cap of 1,
        // and the hardware one has already been taken out of the held map to make room for it.
        for cap in [0, 1] {
            let mut rig = Setup { hardware: 2, bounds: Bounds { pictures: cap, ..BOUNDS }, ..Setup::default() }.rig();
            let (out, seen) = rig.fail_in_the_second_gop();
            assert_eq!(out.get(4), Some(&Err("failed")), "cap {cap}");
            assert_eq!(seen, vec![0, 1, 2, 3, 5], "cap {cap}");
        }
    }

    #[test]
    fn an_in_band_parameter_change_hands_over_before_the_hardware_decoder_is_fed_it() {
        let mut rig = Setup::default().rig();
        let repeated = (1, access_unit(&[&sps(), &pps(), &[0x65, PLAIN]]));
        let changed = (2, access_unit(&[&[0x67, 0x42, 0x00, 0x0A], &[0x65, PLAIN]]));
        assert_eq!(rig.feed(&[at(0, IDR, PLAIN), repeated, changed]), vec![Ok(vec![0]), Ok(vec![1]), Ok(vec![2])]);
        // The entry's own parameter sets in band change nothing; another SPS does.
        assert_eq!(rig.fed_to_hardware(), vec![0, 1]);
        assert_eq!(rig.fed_to_software(), vec![2]);
    }

    #[test]
    fn the_switch_hands_the_software_decoder_the_draft_mode_the_caller_had_set() {
        // Draft mode decodes faster, cheaper pictures: dropping the caller's choice at the handover
        // would quietly change what it sees mid-stream. FilmCraft's hybrid carries it across in its
        // fall-back, and so does this switch.
        // Both ways, and the caller's latest choice: FilmCraft's decode loop sets it before every run.
        for draft in [true, false] {
            let mut rig = Setup::default().rig();
            rig.decoder.set_draft(!draft);
            rig.decoder.set_draft(draft);
            rig.feed(&[at(0, IDR, PLAIN), at(1, P, FAIL)]);
            // A decoder never told is out of draft mode, as FilmCraft's are until they are.
            assert_eq!(rig.software_drafts().last().copied().unwrap_or(false), draft, "draft {draft}");
        }
    }

    #[test]
    fn random_access_and_disposable_samples_are_answered_as_the_software_decoder_answers_them() {
        let rig = Setup::default().rig();
        let software = software_video_decoder(&avc_entry()).unwrap();
        // The fake answers neither, as a platform decoder that leaves them to the wrapper.
        for (_, sample) in [at(0, B, PLAIN), at(0, P, PLAIN), at(0, IDR, PLAIN)] {
            assert_eq!(rig.decoder.is_disposable(&sample), software.is_disposable(&sample));
            assert_eq!(rig.decoder.is_random_access(&sample), software.is_random_access(&sample));
        }
        assert!(rig.decoder.is_disposable(&at(0, B, PLAIN).1));
    }

    #[test]
    fn a_reset_forgets_the_replay_and_the_pictures_the_caller_had() {
        // Before the switch: the replay starts afresh at the first sample after the reset.
        let mut rig = Setup::default().rig();
        rig.feed(&[at(0, IDR, PLAIN), at(1, P, PLAIN)]);
        rig.decoder.reset();
        assert_eq!(rig.feed(&[at(2, P, FAIL)]), vec![Ok(vec![2])]);
        assert_eq!(rig.fed_to_software(), vec![2]);
        // After it: a seek back wants every picture again, the ones the hardware decoder returned included.
        let mut rig = Setup { software: 2, ..Setup::default() }.rig();
        rig.feed(&[at(0, IDR, PLAIN), at(1, P, PLAIN), at(2, P, FAIL)]);
        rig.decoder.reset();
        let mut seen = pictures(&rig.feed(&[at(0, IDR, PLAIN), at(1, P, PLAIN), at(2, P, PLAIN)]));
        seen.extend(rig.flush());
        assert_eq!(seen, vec![0, 1, 2]);
    }

    #[test]
    fn a_stream_no_hardware_decoder_takes_has_no_stream_info() {
        assert!(stream_info(&avc_entry()).is_some_and(|info| info.is_ok()));
        assert!(stream_info(&prores_entry()).is_none());
    }
}
