//! A movie written from an encoder's packets and a sound track: the MP4's tracks, sample entry, timing and
//! colour tag, and the interleaving of sound with picture.
//!
//! Safe code over a [`Backend`], so a fake encoder drives every rule on every target; the platform
//! modules supply the real ones.

use std::io::{Seek, SeekFrom, Write};

use filmcraft_isobmff::{AvcConfig, Brand, ColorInfo, HevcConfig, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};

use crate::encode::{ColourCodes, EncodeError, Reorder, VideoCodec, VideoSpec, Yuv420};

/// Samples of sound in one AAC access unit.
pub const AAC_FRAME: u32 = 1024;

/// One compressed picture, in the encoder's length-prefixed form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub data: Vec<u8>,
    /// Whether decoding can start here.
    pub key: bool,
    /// When the picture is shown, in the movie's timescale, as the encoder stamped it.
    pub pts: i64,
}

/// An encoder of pictures, which gives packets back in the order it was fed them (it is asked for no
/// reordering, so decoding order is presentation order and no packet needs a composition offset). The
/// movie holds every packet to that by its presentation time: a packet that arrived early would be shown
/// in another picture's place, and a decoder still puts its pictures in order, so nothing else would show it.
pub trait Backend {
    /// Takes picture `index` and returns the packets that are ready, which may be none yet.
    fn encode(&mut self, index: u64, frame: &Yuv420) -> Result<Vec<Packet>, EncodeError>;
    /// Ends the stream and returns every packet still held.
    fn finish(&mut self) -> Result<Vec<Packet>, EncodeError>;
    /// The `avcC` or `hvcC` record, known once the first packet has been returned.
    fn record(&self) -> Option<Vec<u8>>;
    /// Whether the stream went on in another encoder after its first packet: [`record`](Self::record) then holds
    /// both encoders' parameter sets, which the first record did not, so the movie reads it again at its end, once
    /// the stream's last picture is known, in place of the first one's.
    fn continued(&self) -> bool {
        false
    }
}

/// The sound of a movie, already encoded as AAC-LC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioTrack {
    /// The two-byte AudioSpecificConfig.
    pub config: Vec<u8>,
    pub channels: u32,
    pub sample_rate: u32,
    /// Samples the encoder adds before the first one, which the track's edit skips.
    pub priming: i64,
    /// The access units, each [`AAC_FRAME`] samples.
    pub units: Vec<Vec<u8>>,
}

/// The sample entry of the picture track: the encoder's own record, the picture's size and the colour tag
/// EffectCraft puts on its movies, limited range.
fn sample_entry(spec: &VideoSpec, record: &[u8]) -> Result<SampleEntry, EncodeError> {
    let (width, height) = (u16::try_from(spec.width), u16::try_from(spec.height));
    let (Ok(width), Ok(height)) = (width, height) else {
        return Err(EncodeError::Invalid("the picture is too large for a sample entry".into()));
    };
    let bad = |what: &str, error: &dyn std::fmt::Display| EncodeError::Failed(format!("the encoder's {what} record does not parse: {error}"));
    let mut entry = match spec.codec {
        VideoCodec::H264 => SampleEntry::avc(AvcConfig::parse(record).map_err(|error| bad("avcC", &error))?, width, height),
        VideoCodec::Hevc => SampleEntry::hevc(HevcConfig::parse(record).map_err(|error| bad("hvcC", &error))?, width, height),
    };
    let ColourCodes { primaries, transfer, matrix } = spec.colour;
    if let Some(video) = entry.video.as_mut() {
        video.color = Some(ColorInfo::Nclx { primaries: primaries.into(), transfer: transfer.into(), matrix: matrix.into(), full_range: false });
    }
    Ok(entry)
}

fn mux(error: impl std::fmt::Display) -> EncodeError {
    EncodeError::Failed(format!("writing the movie failed: {error}"))
}

/// The tracks once the picture's record is known, and what is left of the sound to write.
struct Open<W: Write + Seek> {
    writer: Mp4Writer<W>,
    video: usize,
    /// The sound track's index in the writer.
    audio: Option<usize>,
    /// Pictures written, which is the next picture's index.
    pictures: u64,
    /// Sound units written.
    units: usize,
}

/// A movie being written to `W`.
pub struct Movie<B: Backend, W: Write + Seek> {
    backend: B,
    spec: VideoSpec,
    order: Reorder<Yuv420>,
    audio: Option<AudioTrack>,
    /// `Some` until the writer is opened by the first packet.
    out: Option<W>,
    open: Option<Open<W>>,
}

/// What a finished movie is.
#[derive(Debug)]
pub struct Finished<W> {
    pub out: W,
    pub frames: u64,
    pub bytes: u64,
}

impl<B: Backend, W: Write + Seek> Movie<B, W> {
    /// A movie of `frames` pictures of `spec`, with `audio` if it has sound. Nothing is written until the
    /// first packet exists, since the picture's sample entry is made from it.
    pub fn new(backend: B, out: W, spec: VideoSpec, frames: u64, audio: Option<AudioTrack>) -> Result<Self, EncodeError> {
        spec.check()?;
        Ok(Self { backend, spec, order: Reorder::new(frames), audio, out: Some(out), open: None })
    }

    /// Picture `index` is ready. Pictures may come in any order; each goes to the encoder as soon as the ones
    /// before it have.
    pub fn push_frame(&mut self, index: u64, frame: Yuv420) -> Result<(), EncodeError> {
        for (index, frame) in self.order.push(index, frame)? {
            let packets = self.backend.encode(index, &frame)?;
            self.write(packets)?;
        }
        Ok(())
    }

    /// Ends the movie: every picture must have arrived. Returns the file, closed.
    pub fn finish(mut self) -> Result<Finished<W>, EncodeError> {
        if !self.order.is_complete() {
            return Err(EncodeError::Failed(format!("the movie ended after {} of its frames", self.order.passed())));
        }
        let packets = self.backend.finish()?;
        self.write(packets)?;
        let mut open = self.open.take().ok_or_else(|| EncodeError::Failed("the encoder produced no picture".into()))?;
        if open.pictures != self.order.passed() {
            return Err(EncodeError::Failed(format!("the encoder produced {} pictures for {} frames", open.pictures, self.order.passed())));
        }
        self.write_sound(&mut open, None)?;
        if self.backend.continued() {
            let record = self.backend.record().ok_or_else(|| EncodeError::Failed("the encoder returned a picture without its record".into()))?;
            open.writer.set_sample_entry(open.video, sample_entry(&self.spec, &record)?).map_err(mux)?;
        }
        let frames = open.pictures;
        let mut out = open.writer.finish().map_err(mux)?;
        out.flush().map_err(mux)?;
        let bytes = out.seek(SeekFrom::End(0)).map_err(mux)?;
        Ok(Finished { out, frames, bytes })
    }

    fn write(&mut self, packets: Vec<Packet>) -> Result<(), EncodeError> {
        for packet in packets {
            let mut open = match self.open.take() {
                Some(open) => open,
                None => self.open_writer()?,
            };
            let written = self.write_packet(&mut open, &packet);
            self.open = Some(open);
            written?;
        }
        Ok(())
    }

    /// Opens the writer from the encoder's record: both tracks are added before any sample, as the muxer
    /// needs them.
    fn open_writer(&mut self) -> Result<Open<W>, EncodeError> {
        let record = self.backend.record().ok_or_else(|| EncodeError::Failed("the encoder returned a picture without its record".into()))?;
        let out = self.out.take().ok_or_else(|| EncodeError::Failed("the movie's file was already given up".into()))?;
        let mut writer = Mp4Writer::new(out, WriterOptions::new(Brand::Mp4)).map_err(mux)?;
        let video = writer.add_track(TrackConfig::new(sample_entry(&self.spec, &record)?, self.spec.rate_num)).map_err(mux)?;
        let audio = match &self.audio {
            Some(track) => {
                let entry = SampleEntry::aac(track.config.clone(), track.channels, track.sample_rate);
                let mut config = TrackConfig::new(entry, track.sample_rate);
                config.media_start = Some(track.priming);
                Some(writer.add_track(config).map_err(mux)?)
            }
            None => None,
        };
        Ok(Open { writer, video, audio, pictures: 0, units: 0 })
    }

    fn write_packet(&mut self, open: &mut Open<W>, packet: &Packet) -> Result<(), EncodeError> {
        let due = self.spec.presentation_time(open.pictures);
        if packet.pts != due {
            return Err(EncodeError::Failed(format!(
                "the encoder returned the picture shown at tick {} where the one at tick {due} was next: a picture came back out of its turn or not at all",
                packet.pts
            )));
        }
        let sample = WriteSample { data: &packet.data, duration: self.spec.rate_den, composition_offset: 0, is_sync: packet.key };
        open.writer.write_sample(open.video, sample).map_err(mux)?;
        open.pictures += 1;
        // The sound that starts before this picture ends goes in beside it, as EffectCraft's own movies
        // interleave theirs.
        self.write_sound(open, Some(open.pictures))
    }

    /// Writes the sound units that start before the end of the first `pictures` pictures, or all that are
    /// left.
    fn write_sound(&self, open: &mut Open<W>, pictures: Option<u64>) -> Result<(), EncodeError> {
        let (Some(track), Some(index)) = (&self.audio, open.audio) else { return Ok(()) };
        while let Some(unit) = track.units.get(open.units) {
            if let Some(pictures) = pictures {
                // Unit `n` starts at n * 1024 / rate seconds; the pictures end at pictures * den / num.
                let starts = u128::from(open.units as u64) * u128::from(AAC_FRAME) * u128::from(self.spec.rate_num);
                let ends = u128::from(pictures) * u128::from(self.spec.rate_den) * u128::from(track.sample_rate);
                if starts >= ends {
                    break;
                }
            }
            let sample = WriteSample { data: unit, duration: AAC_FRAME, composition_offset: 0, is_sync: true };
            open.writer.write_sample(index, sample).map_err(mux)?;
            open.units += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io::Cursor;
    use std::rc::Rc;

    use filmcraft_isobmff::{CodecConfig, FourCc, Mp4File, Track, TrackKind};

    use super::*;
    use crate::encode::RateControl;
    use crate::fixtures;

    fn spec(codec: VideoCodec) -> VideoSpec {
        VideoSpec {
            codec,
            width: 1280,
            height: 720,
            rate_num: 30,
            rate_den: 1,
            rate_control: RateControl::Bitrate { kbps: 1000 },
            keyframe_interval: 3,
            colour: ColourCodes { primaries: 1, transfer: 1, matrix: 1 },
        }
    }

    /// An encoder that turns each picture into a packet naming its index, a key frame every third, and
    /// holds the last packet back until the end, as a real one holds some.
    struct Fake {
        record: Option<Vec<u8>>,
        held: Option<Packet>,
        fed: Rc<RefCell<Vec<u64>>>,
    }

    impl Fake {
        fn new(record: Vec<u8>) -> (Self, Rc<RefCell<Vec<u64>>>) {
            let fed = Rc::new(RefCell::new(Vec::new()));
            (Self { record: Some(record), held: None, fed: fed.clone() }, fed)
        }
    }

    impl Backend for Fake {
        fn encode(&mut self, index: u64, _: &Yuv420) -> Result<Vec<Packet>, EncodeError> {
            self.fed.borrow_mut().push(index);
            // A frame lasts one tick at these tests' whole frame rate.
            let packet = Packet { data: vec![0, 0, 0, 1, index as u8], key: index.is_multiple_of(3), pts: index as i64 };
            Ok(self.held.replace(packet).into_iter().collect())
        }
        fn finish(&mut self) -> Result<Vec<Packet>, EncodeError> {
            Ok(self.held.take().into_iter().collect())
        }
        fn record(&self) -> Option<Vec<u8>> {
            self.record.clone()
        }
    }

    fn avcc() -> Vec<u8> {
        let (sps, pps) = (fixtures::sps(), fixtures::pps());
        let mut record = vec![1, sps[1], sps[2], sps[3], 0xFF, 0xE1];
        record.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        record.extend_from_slice(&sps);
        record.push(1);
        record.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        record.extend_from_slice(&pps);
        record
    }

    fn blank() -> Yuv420 {
        Yuv420 { y: Vec::new(), u: Vec::new(), v: Vec::new() }
    }

    fn movie(frames: u64, audio: Option<AudioTrack>) -> (Movie<Fake, Cursor<Vec<u8>>>, Rc<RefCell<Vec<u64>>>) {
        let (fake, fed) = Fake::new(avcc());
        (Movie::new(fake, Cursor::new(Vec::new()), spec(VideoCodec::H264), frames, audio).unwrap(), fed)
    }

    fn movie_of_two() -> (Movie<Fake, Cursor<Vec<u8>>>, Rc<RefCell<Vec<u64>>>) {
        movie(2, None)
    }

    fn track(units: usize) -> AudioTrack {
        AudioTrack { config: vec![0x11, 0x90], channels: 2, sample_rate: 48_000, priming: 2112, units: (0..units).map(|n| vec![n as u8; 4]).collect() }
    }

    /// The file read back.
    fn read(file: Vec<u8>) -> Mp4File {
        filmcraft_isobmff::open(file).unwrap()
    }

    fn of_kind(file: &Mp4File, kind: TrackKind) -> &Track {
        file.tracks.iter().find(|track| track.kind == kind).unwrap()
    }

    #[test]
    fn pictures_reach_the_encoder_in_order_whatever_order_they_came_in() {
        let (mut movie, fed) = movie(4, None);
        for index in [2, 0, 3, 1] {
            movie.push_frame(index, blank()).unwrap();
        }
        assert_eq!(*fed.borrow(), [0, 1, 2, 3]);
        assert_eq!(movie.finish().unwrap().frames, 4);
    }

    #[test]
    fn a_movie_with_a_missing_picture_is_not_finished() {
        let (mut movie, _) = movie(3, None);
        movie.push_frame(0, blank()).unwrap();
        movie.push_frame(2, blank()).unwrap();
        assert_eq!(movie.finish().unwrap_err().to_string(), "the movie ended after 1 of its frames");
    }

    #[test]
    fn a_picture_the_movie_has_no_room_for_is_refused() {
        let (mut movie, _) = movie(2, None);
        assert_eq!(movie.push_frame(2, blank()).unwrap_err().to_string(), "frame 2 arrived in a movie of 2 frames");
    }

    #[test]
    fn a_request_that_cannot_be_encoded_is_refused_before_anything_starts() {
        let (fake, _) = Fake::new(avcc());
        let odd = VideoSpec { width: 1281, ..spec(VideoCodec::H264) };
        assert!(Movie::new(fake, Cursor::new(Vec::new()), odd, 1, None).is_err());
    }

    #[test]
    fn the_movie_has_the_picture_track_the_request_describes() {
        let (mut movie, _) = movie(5, None);
        for index in 0..5 {
            movie.push_frame(index, blank()).unwrap();
        }
        let finished = movie.finish().unwrap();
        assert_eq!(finished.bytes, finished.out.get_ref().len() as u64);
        let file = read(finished.out.into_inner());
        let video = of_kind(&file, TrackKind::Video);
        assert_eq!(video.timescale, 30, "a tick is a frame at a whole rate");
        assert_eq!(video.samples.len(), 5);
        assert!(video.samples.iter().all(|sample| sample.duration == 1 && sample.pts == sample.dts));
        let entry = video.entries.first().unwrap();
        let params = entry.video.as_ref().unwrap();
        assert_eq!((params.width, params.height), (1280, 720));
        assert_eq!(params.color, Some(ColorInfo::Nclx { primaries: 1, transfer: 1, matrix: 1, full_range: false }));
        assert!(matches!(entry.codec, CodecConfig::Avc(_)));
        let keys: Vec<bool> = video.samples.iter().map(|sample| sample.is_sync).collect();
        assert_eq!(keys, [true, false, false, true, false]);
        assert!(video.edits.is_empty(), "no reordering, so no edit");
    }

    #[test]
    fn a_stream_that_went_on_in_another_encoder_is_written_with_the_record_that_covers_the_whole_of_it() {
        /// Gives a record at the end that is not the one it gave for the first packet.
        struct Continued {
            fake: Fake,
            finished: bool,
        }
        impl Backend for Continued {
            fn encode(&mut self, index: u64, frame: &Yuv420) -> Result<Vec<Packet>, EncodeError> {
                self.fake.encode(index, frame)
            }
            fn finish(&mut self) -> Result<Vec<Packet>, EncodeError> {
                self.finished = true;
                self.fake.finish()
            }
            fn record(&self) -> Option<Vec<u8>> {
                let record = self.fake.record()?;
                let mut config = AvcConfig::parse(&record).ok()?;
                config.level = config.level.saturating_add(if self.finished { 9 } else { 0 });
                Some(config.to_bytes())
            }
            fn continued(&self) -> bool {
                true
            }
        }
        let level = |bytes: Vec<u8>| match read(bytes).tracks.first().and_then(|track| track.entries.first()).map(|entry| (entry.format, entry.codec.clone())) {
            Some((format, CodecConfig::Avc(record))) => Some((format, record.level)),
            _ => None,
        };
        let (fake, _) = Fake::new(avcc());
        let first = AvcConfig::parse(&avcc()).unwrap().level;
        let mut movie = Movie::new(Continued { fake, finished: false }, Cursor::new(Vec::new()), spec(VideoCodec::H264), 2, None).unwrap();
        (0..2).for_each(|index| movie.push_frame(index, blank()).unwrap());
        assert_eq!(level(movie.finish().unwrap().out.into_inner()), Some((FourCc(*b"avc1"), first + 9)), "the record read at the end is the entry's");
        let (mut movie, _) = movie_of_two();
        (0..2).for_each(|index| movie.push_frame(index, blank()).unwrap());
        assert_eq!(level(movie.finish().unwrap().out.into_inner()), Some((FourCc(*b"avc1"), first)), "a stream of one encoder keeps its first record");
    }

    #[test]
    fn the_colour_tag_carries_the_requests_code_points_in_limited_range() {
        let (fake, _) = Fake::new(avcc());
        let wide = VideoSpec { colour: ColourCodes { primaries: 12, transfer: 13, matrix: 1 }, ..spec(VideoCodec::H264) };
        let mut movie = Movie::new(fake, Cursor::new(Vec::new()), wide, 1, None).unwrap();
        movie.push_frame(0, blank()).unwrap();
        let file = read(movie.finish().unwrap().out.into_inner());
        let video = of_kind(&file, TrackKind::Video);
        assert_eq!(
            video.entries.first().and_then(|entry| entry.video.as_ref()).unwrap().color,
            Some(ColorInfo::Nclx { primaries: 12, transfer: 13, matrix: 1, full_range: false })
        );
    }

    #[test]
    fn a_record_that_does_not_parse_fails_the_movie_in_words() {
        let (fake, _) = Fake::new(vec![1, 2]);
        let mut movie = Movie::new(fake, Cursor::new(Vec::new()), spec(VideoCodec::H264), 2, None).unwrap();
        movie.push_frame(0, blank()).unwrap();
        let error = movie.push_frame(1, blank()).unwrap_err().to_string();
        assert!(error.starts_with("the encoder's avcC record does not parse: "), "{error}");
        let (fake, _) = Fake::new(vec![1, 2]);
        let mut movie = Movie::new(fake, Cursor::new(Vec::new()), spec(VideoCodec::Hevc), 2, None).unwrap();
        movie.push_frame(0, blank()).unwrap();
        assert!(movie.push_frame(1, blank()).unwrap_err().to_string().starts_with("the encoder's hvcC record does not parse: "));
    }

    #[test]
    fn a_movie_the_encoder_gave_too_few_pictures_for_is_not_finished() {
        struct Drops(Fake);
        impl Backend for Drops {
            fn encode(&mut self, index: u64, frame: &Yuv420) -> Result<Vec<Packet>, EncodeError> {
                self.0.encode(index, frame)
            }
            fn finish(&mut self) -> Result<Vec<Packet>, EncodeError> {
                Ok(Vec::new())
            }
            fn record(&self) -> Option<Vec<u8>> {
                self.0.record()
            }
        }
        let (fake, _) = Fake::new(avcc());
        let mut movie = Movie::new(Drops(fake), Cursor::new(Vec::new()), spec(VideoCodec::H264), 3, None).unwrap();
        for index in 0..3 {
            movie.push_frame(index, blank()).unwrap();
        }
        assert_eq!(movie.finish().unwrap_err().to_string(), "the encoder produced 2 pictures for 3 frames");
    }

    #[test]
    fn a_picture_the_encoder_returns_before_its_turn_fails_the_movie() {
        // An encoder that reordered: it holds the first picture and returns the second before it, as an
        // encoder allowed B pictures returns the reference they are predicted from.
        struct Reorders(Option<Packet>);
        impl Backend for Reorders {
            fn encode(&mut self, index: u64, _: &Yuv420) -> Result<Vec<Packet>, EncodeError> {
                let packet = Packet { data: vec![0, 0, 0, 1, index as u8], key: index == 0, pts: index as i64 };
                Ok(match self.0.take() {
                    Some(held) => vec![packet, held],
                    None => {
                        self.0 = Some(packet);
                        Vec::new()
                    }
                })
            }
            fn finish(&mut self) -> Result<Vec<Packet>, EncodeError> {
                Ok(self.0.take().into_iter().collect())
            }
            fn record(&self) -> Option<Vec<u8>> {
                Some(avcc())
            }
        }
        let mut movie = Movie::new(Reorders(None), Cursor::new(Vec::new()), spec(VideoCodec::H264), 2, None).unwrap();
        movie.push_frame(0, blank()).unwrap();
        assert_eq!(
            movie.push_frame(1, blank()).unwrap_err().to_string(),
            "the encoder returned the picture shown at tick 1 where the one at tick 0 was next: a picture came back out of its turn or not at all"
        );
    }

    #[test]
    fn a_movie_with_no_picture_at_all_is_not_finished() {
        let (movie, _) = movie(0, None);
        assert_eq!(movie.finish().unwrap_err().to_string(), "the encoder produced no picture");
    }

    #[test]
    fn the_sound_track_carries_the_encoders_priming_as_its_edit_and_every_unit() {
        let (mut movie, _) = movie(30, Some(track(47)));
        for index in 0..30 {
            movie.push_frame(index, blank()).unwrap();
        }
        let file = read(movie.finish().unwrap().out.into_inner());
        let audio = of_kind(&file, TrackKind::Audio);
        assert_eq!(audio.timescale, 48_000);
        assert_eq!(audio.samples.len(), 47);
        assert_eq!(audio.edits.first().map(|edit| edit.media_time), Some(2112));
    }

    #[test]
    fn sound_is_written_beside_the_picture_it_plays_with() {
        // 30 pictures a second and 48 kHz: unit n starts at n*1024/48000 s, and picture p ends at (p+1)/30 s.
        // Unit 23 starts at 0.4907 s, inside picture 14 (0.4667 to 0.5 s), so it is written after picture 14 and
        // before picture 15; unit 0 goes in with the first picture, and the last unit (46, at 0.981 s) with the
        // last pictures.
        let (mut movie, _) = movie(30, Some(track(47)));
        for index in 0..30 {
            movie.push_frame(index, blank()).unwrap();
        }
        let file = read(movie.finish().unwrap().out.into_inner());
        let (video, audio) = (of_kind(&file, TrackKind::Video), of_kind(&file, TrackKind::Audio));
        let at = |track: &Track, n: usize| track.samples.get(n).map(|sample| sample.offset).unwrap();
        assert!(at(audio, 0) > at(video, 0) && at(audio, 0) < at(video, 1), "unit 0 follows the first picture");
        assert!(at(audio, 23) > at(video, 14) && at(audio, 23) < at(video, 15), "unit 23 follows picture 14");
        // Unit 25 starts at 25*1024/48000 = 16/30 s, exactly where picture 15 ends: it belongs to picture 16.
        assert!(at(audio, 25) > at(video, 16) && at(audio, 25) < at(video, 17), "a unit that starts as a picture ends goes after the next one");
        assert!(at(audio, 46) > at(video, 28), "unit 46 goes in with the last pictures");
    }

    #[test]
    fn sound_that_outlasts_the_pictures_is_still_written() {
        let (mut movie, _) = movie(1, Some(track(10)));
        movie.push_frame(0, blank()).unwrap();
        let file = read(movie.finish().unwrap().out.into_inner());
        assert_eq!(of_kind(&file, TrackKind::Audio).samples.len(), 10);
    }
}
