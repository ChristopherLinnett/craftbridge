//! A movie that goes on in the software encoder when its hardware encoder is lost: a hardware session is
//! invalidated when the app goes to the background (VideoToolbox answers -12903) or reclaimed by the system
//! (Android's `MediaCodec` can be, for a higher priority use of the same hardware), and the export a user
//! started must not fail because they looked at another app.
//!
//! [`Failover`] stands in for the hardware encoder under [`crate::movie::Movie`]. It keeps each picture the
//! hardware took until a packet comes back for it, since a lost session never returns the pictures it held;
//! when the hardware fails, the pictures with no packet and the one being encoded go to the software encoder
//! in order, so every frame is encoded once and the packets stay in frame order. Where the software encoder
//! cannot go on with the stream the hardware began, the movie ends in an error naming the lost session.
//!
//! The one definition of which encoder made which frames is [`EncodingStats`], which `info` reports.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::encode::{EncodeError, VideoSpec, Yuv420};
use crate::movie::{Backend, Packet};

/// A software encoder that can take a stream over from a hardware one.
pub trait Software: Backend + Sized + 'static {
    /// Starts an encoder for `spec`, going on with the stream the hardware began when it had returned pictures
    /// of it: its [`record`](Backend::record) is then the record of the whole stream, both encoders' pictures.
    /// [`EncodeError::Unavailable`] when there is no software encoder for the request, or when it cannot go on
    /// with that stream.
    fn start(spec: VideoSpec, first: Option<Handover<'_>>) -> Result<Self, EncodeError>;
}

/// What the hardware encoder had made when it was lost, which the software encoder goes on from.
#[derive(Clone, Copy, Debug)]
pub struct Handover<'a> {
    /// The hardware's record of its stream.
    pub record: &'a [u8],
    /// The last picture the hardware returned, which the software's first picture follows in decoding order.
    pub last: &'a Packet,
}

/// Which encoder made the pictures of the movies written since the process started, which only grow: two
/// readings measure what happened between them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncodingStats {
    /// Pictures the hardware encoders made.
    pub hardware_pictures: u64,
    /// Pictures the software encoder made.
    pub software_pictures: u64,
    /// Movies whose hardware encoder was lost and which went on in software.
    pub fallbacks: u64,
    /// The first frame the software encoder made in the latest of those: every frame before it is the
    /// hardware's, and it and every frame after it the software's.
    pub last_fallback_frame: Option<u64>,
}

/// The counters of [`EncodingStats`].
#[derive(Debug, Default)]
pub(crate) struct Tally(Mutex<EncodingStats>);

impl Tally {
    /// The counters every movie of the process adds to.
    pub(crate) fn process() -> Arc<Self> {
        static TALLY: OnceLock<Arc<Tally>> = OnceLock::new();
        TALLY.get_or_init(Arc::default).clone()
    }

    pub(crate) fn read(&self) -> EncodingStats {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn change(&self, change: impl FnOnce(&mut EncodingStats)) {
        change(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner));
    }
}

/// How the software encoder is started once it is needed: for the spec, and the stream it goes on with, if any.
type Starter<S> = Box<dyn FnOnce(VideoSpec, Option<Handover<'_>>) -> Result<S, EncodeError> + Send>;

enum Encoder<H, S> {
    Hardware(H),
    Software(S),
}

/// The encoder of one movie: the hardware's until it fails, then the software's.
pub struct Failover<H, S> {
    spec: VideoSpec,
    encoder: Encoder<H, S>,
    /// The pictures the hardware took that no packet has come back for yet.
    pending: VecDeque<(u64, Yuv420)>,
    /// Packets handed on, which is the next frame without one.
    returned: u64,
    /// The last packet the hardware returned.
    last: Option<Packet>,
    /// Whether the stream went on in the software encoder after a first packet of the hardware's.
    continued: bool,
    tally: Arc<Tally>,
    /// Taken when the software encoder is started, which happens once.
    starter: Option<Starter<S>>,
}

/// Whether `error` is the encoder's own failure: a request that was wrong would fail the same way in software.
fn is_lost(error: &EncodeError) -> bool {
    matches!(error, EncodeError::Failed(_) | EncodeError::Unavailable(_))
}

impl<H: Backend, S: Software> Failover<H, S> {
    /// `hardware`, started for `spec`, with the software encoder behind it.
    pub fn new(hardware: H, spec: VideoSpec) -> Self {
        Self::counting(hardware, spec, Tally::process(), Box::new(S::start))
    }

    pub(crate) fn counting(hardware: H, spec: VideoSpec, tally: Arc<Tally>, starter: Starter<S>) -> Self {
        Self { spec, encoder: Encoder::Hardware(hardware), pending: VecDeque::new(), returned: 0, last: None, continued: false, tally, starter: Some(starter) }
    }

    /// Counts `packets` as made by whoever is encoding, and as handed on.
    fn handed_on(&mut self, packets: Vec<Packet>) -> Vec<Packet> {
        let made = packets.len() as u64;
        for _ in &packets {
            self.pending.pop_front();
        }
        self.returned = self.returned.saturating_add(made);
        match self.encoder {
            Encoder::Hardware(_) => {
                if let Some(last) = packets.last() {
                    self.last = Some(last.clone());
                }
                self.tally.change(|stats| stats.hardware_pictures = stats.hardware_pictures.saturating_add(made));
            }
            Encoder::Software(_) => self.tally.change(|stats| stats.software_pictures = stats.software_pictures.saturating_add(made)),
        }
        packets
    }

    /// The error of a movie that cannot go on: the lost session, named, and why the software encoder cannot
    /// take the stream over.
    fn cannot_go_on(&self, lost: &EncodeError, why: impl std::fmt::Display) -> EncodeError {
        EncodeError::Failed(format!(
            "the hardware video encoder's session was lost at frame {} ({lost}) and the movie cannot go on in the software encoder: {why}",
            self.returned
        ))
    }

    /// Takes the movie over in the software encoder after the hardware's `lost`: every picture the hardware
    /// has not returned a packet for is encoded again, in order, and the stream ended there when `finishing`.
    fn switch(&mut self, lost: &EncodeError, finishing: bool) -> Result<Vec<Packet>, EncodeError> {
        // A session that fails while the movie ends, with a packet back for every picture, has lost nothing.
        if finishing && self.pending.is_empty() {
            return Ok(Vec::new());
        }
        let stream = match (&self.encoder, &self.last) {
            (Encoder::Hardware(hardware), Some(last)) => {
                Some((hardware.record().ok_or_else(|| self.cannot_go_on(lost, "the hardware encoder has no record of its stream"))?, last.clone()))
            }
            _ => None,
        };
        let first = stream.as_ref().map(|(record, last)| Handover { record, last });
        let start = self.starter.take().ok_or_else(|| self.cannot_go_on(lost, "the software encoder was already started"))?;
        let software = start(self.spec, first).map_err(|why| self.cannot_go_on(lost, why))?;
        let frame = self.returned;
        // The hardware session is released here, with the pictures it held.
        self.encoder = Encoder::Software(software);
        self.continued = stream.is_some();
        self.tally.change(|stats| {
            stats.fallbacks = stats.fallbacks.saturating_add(1);
            stats.last_fallback_frame = Some(frame);
        });
        let mut packets = Vec::new();
        while let Some((index, picture)) = self.pending.pop_front() {
            let Encoder::Software(software) = &mut self.encoder else { break };
            packets.extend(software.encode(index, &picture)?);
        }
        if finishing && let Encoder::Software(software) = &mut self.encoder {
            packets.extend(software.finish()?);
        }
        Ok(self.handed_on(packets))
    }
}

impl<H: Backend, S: Software> Backend for Failover<H, S> {
    fn encode(&mut self, index: u64, frame: &Yuv420) -> Result<Vec<Packet>, EncodeError> {
        let result = match &mut self.encoder {
            Encoder::Hardware(hardware) => {
                self.pending.push_back((index, frame.clone()));
                hardware.encode(index, frame)
            }
            Encoder::Software(software) => software.encode(index, frame),
        };
        match (result, &self.encoder) {
            (Ok(packets), _) => Ok(self.handed_on(packets)),
            (Err(lost), Encoder::Hardware(_)) if is_lost(&lost) => self.switch(&lost, false),
            (Err(error), _) => Err(error),
        }
    }

    fn finish(&mut self) -> Result<Vec<Packet>, EncodeError> {
        let result = match &mut self.encoder {
            Encoder::Hardware(hardware) => hardware.finish(),
            Encoder::Software(software) => software.finish(),
        };
        match (result, &self.encoder) {
            (Ok(packets), _) => Ok(self.handed_on(packets)),
            (Err(lost), Encoder::Hardware(_)) if is_lost(&lost) => self.switch(&lost, true),
            (Err(error), _) => Err(error),
        }
    }

    fn record(&self) -> Option<Vec<u8>> {
        match &self.encoder {
            Encoder::Hardware(encoder) => encoder.record(),
            Encoder::Software(encoder) => encoder.record(),
        }
    }

    fn continued(&self) -> bool {
        self.continued
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::tests::spec;

    /// What an encoder was fed, shared with the test.
    type Log = Arc<Mutex<Vec<u64>>>;

    fn blank(index: u64) -> Yuv420 {
        Yuv420 { y: vec![index as u8], u: Vec::new(), v: Vec::new() }
    }

    fn packet(index: u64) -> Packet {
        Packet { data: vec![index as u8], key: index == 0, pts: index as i64 }
    }

    /// An encoder that holds the last picture back until the end, as a real one holds some, and fails when
    /// told to.
    struct Fake {
        fed: Log,
        held: Option<u64>,
        /// Whether the last picture is held back until the end.
        holds: bool,
        /// The picture whose `encode` fails, and the failure.
        fails_at: Option<(u64, EncodeError)>,
        fails_finishing: Option<EncodeError>,
        record: Option<Vec<u8>>,
    }

    impl Fake {
        fn new(fed: &Log) -> Self {
            Self { fed: fed.clone(), held: None, holds: true, fails_at: None, fails_finishing: None, record: None }
        }
    }

    impl Backend for Fake {
        fn encode(&mut self, index: u64, frame: &Yuv420) -> Result<Vec<Packet>, EncodeError> {
            assert_eq!(frame, &blank(index), "the picture fed as frame {index} is that frame's");
            self.fed.lock().unwrap().push(index);
            // The picture was taken, as VideoToolbox takes one, and its packet is never made.
            if let Some((_, error)) = self.fails_at.as_ref().filter(|(at, _)| *at == index) {
                return Err(error.clone());
            }
            if self.holds { Ok(self.held.replace(index).map(packet).into_iter().collect()) } else { Ok(vec![packet(index)]) }
        }
        fn finish(&mut self) -> Result<Vec<Packet>, EncodeError> {
            match self.fails_finishing.clone() {
                Some(error) => Err(error),
                None => Ok(self.held.take().map(packet).into_iter().collect()),
            }
        }
        fn record(&self) -> Option<Vec<u8>> {
            self.record.clone()
        }
    }

    /// The software side: the same fake, with the record it was started for.
    struct Soft {
        fake: Fake,
        record: Vec<u8>,
    }

    impl Backend for Soft {
        fn encode(&mut self, index: u64, frame: &Yuv420) -> Result<Vec<Packet>, EncodeError> {
            self.fake.encode(index, frame)
        }
        fn finish(&mut self) -> Result<Vec<Packet>, EncodeError> {
            self.fake.finish()
        }
        fn record(&self) -> Option<Vec<u8>> {
            Some(self.record.clone())
        }
    }

    impl Software for Soft {
        fn start(_: VideoSpec, _: Option<Handover<'_>>) -> Result<Self, EncodeError> {
            Err(EncodeError::Unavailable("these tests start it through their own starter".into()))
        }
    }

    const LOST: &str = "encoding the picture failed (-12903)";

    fn lost() -> EncodeError {
        EncodeError::Failed(LOST.into())
    }

    struct Run {
        hardware: Log,
        software: Log,
        movie: Failover<Fake, Soft>,
        tally: Arc<Tally>,
        /// The time of the last picture the hardware returned, as the software encoder was told when it started.
        handed_over: Arc<Mutex<Option<i64>>>,
    }

    /// A movie over the fake hardware `configure` sets up, whose software side goes on with the hardware's
    /// stream by adding `continues` to its record, or refuses to with that error, and refuses to start at all with
    /// `refuses`.
    fn run(configure: impl FnOnce(&mut Fake), continues: Result<Vec<u8>, String>, refuses: Option<EncodeError>) -> Run {
        let (hardware, software) = (Log::default(), Log::default());
        let mut fake = Fake::new(&hardware);
        fake.record = Some(b"hardware".to_vec());
        configure(&mut fake);
        let tally = Arc::new(Tally::default());
        let fed = software.clone();
        let handed_over = Arc::new(Mutex::new(None));
        let told = handed_over.clone();
        let starter: Starter<Soft> = Box::new(move |_, first| {
            *told.lock().unwrap() = first.map(|first| first.last.pts);
            let record = match (refuses, first) {
                (Some(error), _) => return Err(error),
                (None, Some(first)) => [first.record, &continues.map_err(EncodeError::Unavailable)?].concat(),
                (None, None) => b"software".to_vec(),
            };
            Ok(Soft { fake: Fake::new(&fed), record })
        });
        Run { hardware, software, movie: Failover::counting(fake, spec(), tally.clone(), starter), tally, handed_over }
    }

    fn feed(run: &mut Run, frames: std::ops::Range<u64>) -> Result<Vec<Packet>, EncodeError> {
        let mut packets = Vec::new();
        for index in frames {
            packets.extend(run.movie.encode(index, &blank(index))?);
        }
        Ok(packets)
    }

    fn indices(packets: &[Packet]) -> Vec<i64> {
        packets.iter().map(|packet| packet.pts).collect()
    }

    #[test]
    fn a_movie_the_hardware_finishes_is_never_the_softwares() {
        let mut run = run(|_| {}, Ok(Vec::new()), None);
        let mut packets = feed(&mut run, 0..5).unwrap();
        packets.extend(run.movie.finish().unwrap());
        assert_eq!(indices(&packets), [0, 1, 2, 3, 4]);
        assert!(run.software.lock().unwrap().is_empty());
        assert_eq!(run.tally.read(), EncodingStats { hardware_pictures: 5, ..EncodingStats::default() });
        assert_eq!((run.movie.record(), run.movie.continued()), (Some(b"hardware".to_vec()), false));
    }

    #[test]
    fn a_session_lost_mid_movie_goes_on_in_software_from_the_frame_after_the_last_packet_with_every_frame_once() {
        let mut run = run(|fake| fake.fails_at = Some((4, lost())), Ok(b"+merged".to_vec()), None);
        let mut packets = feed(&mut run, 0..8).unwrap();
        packets.extend(run.movie.finish().unwrap());
        assert_eq!(indices(&packets), [0, 1, 2, 3, 4, 5, 6, 7], "every frame once and in order");
        // Packets 0 to 2 came back before the loss; frame 3 was held and 4 was being encoded, so both go again.
        assert_eq!(*run.hardware.lock().unwrap(), [0, 1, 2, 3, 4]);
        assert_eq!(*run.software.lock().unwrap(), [3, 4, 5, 6, 7]);
        assert_eq!(run.tally.read(), EncodingStats { hardware_pictures: 3, software_pictures: 5, fallbacks: 1, last_fallback_frame: Some(3) });
        assert_eq!(run.movie.record(), Some(b"hardware+merged".to_vec()), "the record covers both encoders' pictures");
        assert!(run.movie.continued());
        assert_eq!(*run.handed_over.lock().unwrap(), Some(2), "the software encoder is told the last picture the hardware returned");
    }

    #[test]
    fn a_session_lost_while_the_movie_ends_finishes_the_held_pictures_in_software() {
        let mut run = run(|fake| fake.fails_finishing = Some(lost()), Ok(Vec::new()), None);
        let mut packets = feed(&mut run, 0..4).unwrap();
        packets.extend(run.movie.finish().unwrap());
        assert_eq!(indices(&packets), [0, 1, 2, 3]);
        assert_eq!(*run.software.lock().unwrap(), [3]);
        assert_eq!(run.tally.read().last_fallback_frame, Some(3));
    }

    #[test]
    fn a_session_that_fails_while_the_movie_ends_with_nothing_left_to_encode_loses_nothing() {
        let mut run = run(
            |fake| {
                fake.holds = false;
                fake.fails_finishing = Some(lost());
            },
            Err("never asked".into()),
            None,
        );
        let mut packets = feed(&mut run, 0..3).unwrap();
        packets.extend(run.movie.finish().unwrap());
        assert_eq!(indices(&packets), [0, 1, 2]);
        assert!(run.software.lock().unwrap().is_empty());
        assert_eq!(run.tally.read().fallbacks, 0);
    }

    #[test]
    fn a_session_refused_before_any_packet_makes_the_whole_movie_in_software_with_no_record_to_continue() {
        let mut run = run(|fake| fake.fails_at = Some((0, EncodeError::Unavailable("refused".into()))), Err("never asked".into()), None);
        let mut packets = feed(&mut run, 0..3).unwrap();
        packets.extend(run.movie.finish().unwrap());
        assert_eq!(indices(&packets), [0, 1, 2]);
        assert_eq!(*run.software.lock().unwrap(), [0, 1, 2]);
        assert_eq!((run.movie.record(), run.movie.continued()), (Some(b"software".to_vec()), false));
    }

    #[test]
    fn a_stream_the_software_encoder_cannot_continue_ends_in_an_error_naming_the_lost_session() {
        let mut run = run(|fake| fake.fails_at = Some((4, lost())), Err("a profile it cannot continue".into()), None);
        let error = feed(&mut run, 0..8).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "the hardware video encoder's session was lost at frame 3 ({LOST}) and the movie cannot go on in the software encoder: a profile it cannot continue"
            )
        );
        assert!(run.software.lock().unwrap().is_empty(), "nothing was encoded in a stream that cannot be kept valid");
        assert_eq!(run.tally.read().fallbacks, 0);
    }

    #[test]
    fn a_codec_with_no_software_encoder_ends_in_the_same_error() {
        let mut run =
            run(|fake| fake.fails_at = Some((2, lost())), Ok(Vec::new()), Some(EncodeError::Unavailable("there is no software encoder for HEVC here".into())));
        let error = feed(&mut run, 0..4).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "the hardware video encoder's session was lost at frame 1 ({LOST}) and the movie cannot go on in the software encoder: there is no software encoder for HEVC here"
            )
        );
    }

    #[test]
    fn a_request_the_hardware_found_wrong_would_fail_in_software_too_and_is_not_handed_over() {
        let mut run = run(|fake| fake.fails_at = Some((1, EncodeError::Invalid("not a picture".into()))), Ok(Vec::new()), None);
        assert_eq!(feed(&mut run, 0..3).unwrap_err(), EncodeError::Invalid("not a picture".into()));
        assert!(run.software.lock().unwrap().is_empty());
    }
}
