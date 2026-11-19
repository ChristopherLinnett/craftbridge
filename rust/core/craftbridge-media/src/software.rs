//! The software H.264 encoder a movie goes on in when its hardware encoder is lost (see [`crate::failover`]):
//! the clean-room encoder EffectCraft's own export encodes with, set up as that export sets it up, but asked
//! for no reordering, so packets leave in the order pictures went in and carry the time they were fed at, as
//! [`crate::movie`] holds every encoder to.
//!
//! A stream it takes over keeps the hardware's pictures before its own, in one track under one sample entry,
//! but the two encoders' parameter sets cannot be one: each writes its slice headers for its own sequence
//! parameter set (this encoder's frame numbers and picture order counts always take 8 bits, where VideoToolbox's
//! took 5 and 6 in a Mac's movie keyed every 15 frames), and neither can be told the other's. Both encoders
//! number their sets from 0, so this one's are renamed to ids the hardware's record leaves free, its slices are
//! rewritten to name them, and the movie's record holds both encoders' sets. Every set a picture needs is then
//! in the sample entry, as an `avc1` entry keeps them (ISO/IEC 14496-15), and a reader may start or seek back at
//! any key picture: sets of the same id carried in the pictures instead would replace the hardware's in a
//! decoder that keeps them across a seek, as AVPlayer does, and every picture before the switch would decode
//! from the wrong ones.
//!
//! The first key picture after the switch may follow a key picture of the hardware's, and H.264 asks two key
//! pictures in a row to differ in `idr_pic_id` so that a decoder can tell them apart: this encoder numbers its
//! key pictures 0, 1, 0, 1, so they are all flipped when the hardware's last picture is a key picture numbered 0.
//!
//! Safe code on every target; HEVC has no software encoder here, which a caller meets as
//! [`EncodeError::Unavailable`].

use craft_media::bitstream::{BitReader, BitWriter, escape_rbsp, length_prefixed_nals, unescape_rbsp};
use craft_video::h264::decode::params::{Pps, Sps};
use craft_video::h264::decode::slice::{NalHeader, SliceHeader, nal_type};
use craft_video::h264::{ColorConfig, Encoder, EncoderConfig, PacketFormat, Profile, RateControl as Rate, YuvFrame};
use filmcraft_isobmff::AvcConfig;

use crate::encode::{BitRate, EncodeError, VideoCodec, VideoSpec, Yuv420};
use crate::failover::{Handover, Software};
use crate::movie::{Backend, Packet};

/// The size of the length prefix of every NAL unit in a packet and in the record, as the movie's sample
/// entry says.
const LENGTH_SIZE: u8 = 4;

/// H.264 profiles by `profile_idc`, lowest first: Constrained Baseline, Main, High. A stream's record names
/// the highest, which every decoder of a higher profile reads.
const PROFILES: [u8; 3] = [66, 77, 100];

/// How many ids H.264 gives sequence and picture parameter sets.
const SPS_IDS: u32 = 32;
const PPS_IDS: u32 = 256;

/// The most sequence and picture parameter sets an `avcC` record lists: it counts them in 5 and 8 bits.
const RECORD_SPS: usize = 31;
const RECORD_PPS: usize = 255;

/// Why a stream whose record cannot take the software encoder's parameter sets beside its own is not continued.
const NO_ROOM: &str = "the hardware encoder's record has no room for the software encoder's parameter sets";

/// The bits of a sequence parameter set before its id: `profile_idc`, the constraint flags and `level_idc`.
const SPS_HEAD_BITS: usize = 24;

/// The software encoder of one movie.
pub struct SoftwareEncoder {
    encoder: Encoder,
    spec: VideoSpec,
    /// Set when the encoder took a stream over from another.
    continuing: Option<Continuing>,
}

/// What an encoder that took a stream over writes in place of its own: its parameter sets renamed beside the
/// first encoder's, and its slices naming them.
struct Continuing {
    /// The movie's `avcC`: the first encoder's parameter sets and this one's, under the higher profile and level.
    record: Vec<u8>,
    /// The id this encoder's picture parameter set goes by, which each of its slices names.
    pps_id: u32,
    /// What its key pictures' `idr_pic_id` is changed with (1 or 0): the encoder numbers them 0, 1, 0, 1, and the
    /// first follows a key picture of the hardware's that may be numbered 0 too.
    idr_xor: u32,
    /// Its own parameter sets as it wrote them, which its slice headers are read with.
    sps: Sps,
    pps: Pps,
}

fn failed(error: impl std::fmt::Display) -> EncodeError {
    EncodeError::Failed(format!("the software encoder failed: {error}"))
}

fn words(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn length_prefixed(nal: &[u8], into: &mut Vec<u8>) {
    into.extend_from_slice(&u32::try_from(nal.len()).unwrap_or(u32::MAX).to_be_bytes());
    into.extend_from_slice(nal);
}

/// A NAL unit's header byte and its payload with the emulation-prevention bytes taken out.
fn rbsp_of(nal: &[u8]) -> Result<(u8, Vec<u8>), String> {
    let (&header, payload) = nal.split_first().ok_or("a NAL unit with no header")?;
    Ok((header, unescape_rbsp(payload)))
}

/// The NAL unit of `header` and `rbsp`, its emulation prevention put back as the encoder puts it.
fn nal_of(header: u8, rbsp: &[u8]) -> Vec<u8> {
    [&[header][..], &escape_rbsp(rbsp)].concat()
}

/// Where `rbsp_trailing_bits` begins: its stop bit, the last bit set.
fn stop_bit(rbsp: &[u8]) -> Result<usize, String> {
    let (at, byte) = rbsp.iter().enumerate().rev().find(|(_, byte)| **byte != 0).ok_or("a parameter set with no stop bit")?;
    Ok(at.saturating_mul(8).saturating_add(7).saturating_sub(byte.trailing_zeros() as usize))
}

/// Copies bits `from..to` of what `reader` reads to `writer`.
fn copy_bits(reader: &mut BitReader, from: usize, to: usize, writer: &mut BitWriter) -> Result<(), String> {
    reader.seek_bits(from);
    let mut left = to.saturating_sub(from);
    while left > 0 {
        let count = left.min(32);
        writer.write_bits(reader.read_bits(count as u32).map_err(words)?, count as u32);
        left -= count;
    }
    Ok(())
}

/// The parameter set `set` (a NAL unit) with the ids it names after its first `head` bits, one Exp-Golomb code
/// each, given as `ids`, and the rest of it as it was.
fn renamed(set: &[u8], head: usize, ids: &[u32]) -> Result<Vec<u8>, String> {
    let (header, rbsp) = rbsp_of(set)?;
    let mut reader = BitReader::new(&rbsp);
    let mut writer = BitWriter::new();
    copy_bits(&mut reader, 0, head, &mut writer)?;
    for &id in ids {
        reader.read_ue().map_err(words)?;
        writer.write_ue(id);
    }
    let rest = reader.position();
    copy_bits(&mut reader, rest, stop_bit(&rbsp)?, &mut writer)?;
    writer.rbsp_trailing();
    Ok(nal_of(header, &writer.finish()))
}

fn parsed_sps(nal: &[u8]) -> Result<Sps, String> {
    Sps::parse(&rbsp_of(nal)?.1).map_err(|error| format!("a sequence parameter set does not parse: {error}"))
}

/// `nal`, a picture parameter set, read over the sequence parameter sets `spss`, whichever ids they take.
fn parsed_pps(nal: &[u8], spss: &[Sps]) -> Result<Pps, String> {
    let mut table: Vec<Option<Sps>> = vec![None; SPS_IDS as usize];
    for sps in spss {
        if let Some(slot) = table.get_mut(sps.id as usize) {
            *slot = Some(sps.clone());
        }
    }
    Pps::parse(&rbsp_of(nal)?.1, &table).map_err(|error| format!("a picture parameter set does not parse: {error}"))
}

/// Where the `idr_pic_id` of an IDR slice begins, once `reader` has read its `pic_parameter_set_id`: after the
/// fields whose widths `sps` gives.
fn idr_code_at(reader: &mut BitReader, sps: &Sps) -> Result<usize, String> {
    if sps.separate_colour_plane {
        reader.read_bits(2).map_err(words)?;
    }
    reader.read_bits(sps.log2_max_frame_num).map_err(words)?;
    if !sps.frame_mbs_only && reader.read_flag().map_err(words)? {
        reader.read_flag().map_err(words)?;
    }
    Ok(reader.position())
}

/// The `idr_pic_id` of `packet`'s key picture, read with the sets of the stream it is in, or nothing where it holds
/// no IDR slice or the header does not read: a picture whose number is unknown gives none to avoid, and two
/// key pictures that share one are read by every decoder alike.
fn key_picture_id(packet: &Packet, spss: &[Sps], ppss: &[Pps]) -> Option<u32> {
    let unit = length_prefixed_nals(&packet.data, usize::from(LENGTH_SIZE))
        .ok()?
        .into_iter()
        .find(|unit| unit.first().is_some_and(|header| header & 0x1f == nal_type::IDR))?;
    let (header, rbsp) = rbsp_of(unit).ok()?;
    let mut reader = BitReader::new(&rbsp);
    reader.read_ue().ok()?;
    reader.read_ue().ok()?;
    let pps_id = reader.read_ue().ok()?;
    let pps = ppss.iter().find(|pps| pps.id == pps_id)?;
    let sps = spss.iter().find(|sps| sps.id == pps.sps_id)?;
    let (slice, ..) = SliceHeader::parse(&rbsp, NalHeader::parse(header).ok()?, |_| Ok((pps, sps))).ok()?;
    Some(slice.idr_pic_id)
}

impl Continuing {
    /// What `encoder` writes when it takes over the stream `first` made, or why it cannot.
    fn over(encoder: &Encoder, first: Handover<'_>) -> Result<Self, String> {
        let hardware = AvcConfig::parse(first.record).map_err(|error| format!("the hardware encoder's record does not parse: {error}"))?;
        let own = AvcConfig::parse(&encoder.avcc()).map_err(|error| format!("the software encoder's record does not parse: {error}"))?;
        if hardware.length_size != LENGTH_SIZE || own.length_size != LENGTH_SIZE {
            return Err("the two encoders' packets do not carry the same length prefix".into());
        }
        let rank = |profile: u8| PROFILES.iter().position(|&known| known == profile);
        let (Some(have), Some(make)) = (rank(hardware.profile), rank(own.profile)) else {
            return Err(format!("H.264 profile {} or {} is one the software encoder cannot continue", hardware.profile, own.profile));
        };
        let (sps_nal, pps_nal) = encoder.sps_pps();
        let sps = parsed_sps(&sps_nal)?;
        let pps = parsed_pps(&pps_nal, std::slice::from_ref(&sps))?;
        // A record that lists fewer sets than it can holds fewer ids than there are, so an id is left for each.
        if hardware.sps.len() >= RECORD_SPS || hardware.pps.len() >= RECORD_PPS {
            return Err(NO_ROOM.into());
        }
        let theirs = hardware.sps.iter().map(|nal| parsed_sps(nal)).collect::<Result<Vec<_>, _>>()?;
        let their_pps = hardware.pps.iter().map(|nal| parsed_pps(nal, &theirs)).collect::<Result<Vec<_>, _>>()?;
        let sps_id = (0..SPS_IDS).find(|id| theirs.iter().all(|sps| sps.id != *id)).ok_or(NO_ROOM)?;
        let pps_id = (0..PPS_IDS).find(|id| their_pps.iter().all(|pps| pps.id != *id)).ok_or(NO_ROOM)?;
        let idr_xor = u32::from(key_picture_id(first.last, &theirs, &their_pps) == Some(0));
        let top = if make > have { &own } else { &hardware };
        let record = AvcConfig {
            profile: top.profile,
            // A compatibility flag stands only where every parameter set of the stream sets it.
            compatibility: hardware.compatibility & own.compatibility,
            level: hardware.level.max(own.level),
            length_size: LENGTH_SIZE,
            sps: hardware.sps.iter().cloned().chain([renamed(&sps_nal, SPS_HEAD_BITS, &[sps_id])?]).collect(),
            pps: hardware.pps.iter().cloned().chain([renamed(&pps_nal, 0, &[pps_id, sps_id])?]).collect(),
            ext: top.ext.clone(),
        };
        Ok(Self { record: record.to_bytes(), pps_id, idr_xor, sps, pps })
    }

    /// `slice`, a NAL unit of this encoder's, naming the renamed picture parameter set.
    fn renamed_slice(&self, slice: &[u8]) -> Result<Vec<u8>, String> {
        let (header, rbsp) = rbsp_of(slice)?;
        let kind = NalHeader::parse(header).map_err(words)?;
        let (parsed, ..) = SliceHeader::parse(&rbsp, kind, |_| Ok((&self.pps, &self.sps))).map_err(words)?;
        // CABAC slice data starts on a byte boundary, after alignment bits that a header of another length
        // needs a different number of: what follows them is copied whole. High is CABAC-coded, always.
        if !self.pps.entropy_coding_mode {
            return Err("its pictures are not CABAC-coded".into());
        }
        let mut reader = BitReader::new(&rbsp);
        // `first_mb_in_slice` and `slice_type` come before the picture parameter set's id.
        reader.read_ue().map_err(words)?;
        reader.read_ue().map_err(words)?;
        let id_at = reader.position();
        reader.read_ue().map_err(words)?;
        let rest = reader.position();
        let mut writer = BitWriter::new();
        copy_bits(&mut reader, 0, id_at, &mut writer)?;
        writer.write_ue(self.pps_id);
        let mut from = rest;
        if parsed.idr {
            reader.seek_bits(rest);
            let idr_at = idr_code_at(&mut reader, &self.sps)?;
            copy_bits(&mut reader, from, idr_at, &mut writer)?;
            writer.write_ue(parsed.idr_pic_id ^ self.idr_xor);
            reader.seek_bits(idr_at);
            reader.read_ue().map_err(words)?;
            from = reader.position();
        }
        copy_bits(&mut reader, from, parsed.header_bits, &mut writer)?;
        while !writer.is_byte_aligned() {
            writer.write_bit(true);
        }
        writer.write_bytes(rbsp.get(parsed.header_bits.div_ceil(8)..).unwrap_or_default());
        Ok(nal_of(header, &writer.finish()))
    }

    /// A packet of this encoder's with each of its slices renamed, and its other NAL units as they were.
    fn renamed_packet(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        let mut renamed = Vec::with_capacity(data.len().saturating_add(8));
        for unit in length_prefixed_nals(data, usize::from(LENGTH_SIZE)).map_err(words)? {
            let kind = unit.first().map_or(0, |header| header & 0x1f);
            if kind == nal_type::SLICE || kind == nal_type::IDR {
                length_prefixed(&self.renamed_slice(unit)?, &mut renamed);
            } else {
                length_prefixed(unit, &mut renamed);
            }
        }
        Ok(renamed)
    }
}

/// EffectCraft's own settings for `spec`, with no reordering and no key picture the movie did not ask for.
fn configured(spec: &VideoSpec) -> Result<EncoderConfig, EncodeError> {
    spec.check()?;
    if spec.codec != VideoCodec::H264 {
        return Err(EncodeError::Unavailable("there is no software encoder for HEVC here".into()));
    }
    let Some(BitRate { target, peak }) = spec.bit_rate() else {
        return Err(EncodeError::Unavailable("the software H.264 encoder has no constant quality mode".into()));
    };
    let kbps = |bits: i64| u32::try_from(bits / 1000).unwrap_or(u32::MAX);
    let mut config = EncoderConfig::new(spec.width, spec.height, spec.rate_num, spec.rate_den);
    config.profile = Profile::High;
    config.format = PacketFormat::LengthPrefixed;
    config.aud = false;
    config.bframes = 0;
    // A key picture where the movie asked for one and nowhere else.
    config.scenecut = false;
    config.keyint = spec.keyframe_interval;
    config.rate = Rate::Vbr { target_kbps: kbps(target), max_kbps: kbps(peak) };
    let colour = spec.colour;
    config.color = ColorConfig { primaries: colour.primaries, transfer: colour.transfer, matrix: colour.matrix, full_range: false };
    Ok(config)
}

impl SoftwareEncoder {
    /// An encoder of `config` for `spec`, going on with the stream `first` made when there is one.
    fn over(config: EncoderConfig, spec: VideoSpec, first: Option<Handover<'_>>) -> Result<Self, EncodeError> {
        let encoder = Encoder::new(config).map_err(failed)?;
        let continuing = first.map(|first| Continuing::over(&encoder, first)).transpose().map_err(EncodeError::Unavailable)?;
        Ok(Self { encoder, spec, continuing })
    }

    fn packets(&self, packets: Vec<craft_video::h264::Packet>) -> Result<Vec<Packet>, EncodeError> {
        packets
            .into_iter()
            .map(|packet| {
                let data = match &self.continuing {
                    Some(continuing) => continuing.renamed_packet(&packet.data).map_err(failed)?,
                    None => packet.data,
                };
                Ok(Packet { data, key: packet.keyframe, pts: packet.pts })
            })
            .collect()
    }
}

impl Software for SoftwareEncoder {
    fn start(spec: VideoSpec, first: Option<Handover<'_>>) -> Result<Self, EncodeError> {
        Self::over(configured(&spec)?, spec, first)
    }
}

impl Backend for SoftwareEncoder {
    fn encode(&mut self, index: u64, frame: &Yuv420) -> Result<Vec<Packet>, EncodeError> {
        let (width, height) = (self.spec.width, self.spec.height);
        frame.check_picture(width, height)?;
        let (w, chroma_w) = (usize::try_from(width).unwrap_or(0), usize::try_from(width.div_ceil(2)).unwrap_or(0));
        let picture = YuvFrame { y: &frame.y, u: &frame.u, v: &frame.v, y_stride: w, uv_stride: chroma_w };
        let packets = self.encoder.try_encode(&picture, self.spec.presentation_time(index)).map_err(failed)?;
        self.packets(packets)
    }

    fn finish(&mut self) -> Result<Vec<Packet>, EncodeError> {
        let packets = self.encoder.flush();
        self.packets(packets)
    }

    fn record(&self) -> Option<Vec<u8>> {
        Some(self.continuing.as_ref().map_or_else(|| self.encoder.avcc(), |continuing| continuing.record.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::tests::spec;
    use crate::encodetests::{self, picture_of};

    fn software(spec: VideoSpec, first: Option<Handover<'_>>) -> SoftwareEncoder {
        SoftwareEncoder::start(spec, first).unwrap()
    }

    /// A hardware stream that ended in a picture nothing is asked of, with the record `record`.
    fn after(record: &[u8]) -> Option<Handover<'_>> {
        static LAST: Packet = Packet { data: Vec::new(), key: false, pts: 0 };
        Some(Handover { record, last: &LAST })
    }

    /// Every packet `encoder` makes of `frames` pictures, the end of the stream's included.
    fn encoded(encoder: &mut SoftwareEncoder, spec: VideoSpec, frames: std::ops::Range<u64>) -> Vec<Packet> {
        let mut packets = Vec::new();
        for index in frames {
            packets.extend(encoder.encode(index, &picture_of(spec.width, spec.height, index)).unwrap());
        }
        packets.extend(encoder.finish().unwrap());
        packets
    }

    /// The NAL units of a packet or of a record's list.
    fn units(data: &[u8]) -> Vec<&[u8]> {
        length_prefixed_nals(data, 4).unwrap()
    }

    #[test]
    fn hevc_and_a_constant_quality_have_no_software_encoder() {
        let hevc = VideoSpec { codec: VideoCodec::Hevc, ..spec() };
        assert_eq!(SoftwareEncoder::start(hevc, None).err().map(|e| e.to_string()), Some("there is no software encoder for HEVC here".into()));
        let quality = VideoSpec { rate_control: crate::encode::RateControl::Quality { percent: 70 }, ..spec() };
        assert_eq!(SoftwareEncoder::start(quality, None).err().map(|e| e.to_string()), Some("the software H.264 encoder has no constant quality mode".into()));
    }

    #[test]
    fn a_request_that_cannot_be_encoded_is_refused_before_a_session_starts() {
        assert!(matches!(SoftwareEncoder::start(VideoSpec { width: 63, ..spec() }, None), Err(EncodeError::Invalid(_))));
    }

    #[test]
    fn the_bit_rate_is_the_one_the_hardware_encoders_are_asked_for() {
        // Each side of the codec's floor and a rate far above it: whatever the one rule says, the encoder is told.
        for kbps in [10, 99, 100, 101, 8001] {
            let spec = VideoSpec { rate_control: crate::encode::RateControl::Bitrate { kbps }, ..spec() };
            let BitRate { target, peak } = spec.bit_rate().unwrap();
            let Rate::Vbr { target_kbps, max_kbps } = configured(&spec).unwrap().rate else { panic!("a variable bit rate") };
            assert_eq!((i64::from(target_kbps), i64::from(max_kbps)), (target / 1000, peak / 1000), "{kbps} kbit/s asked");
        }
    }

    #[test]
    fn a_picture_that_is_not_the_movies_is_refused() {
        let wrong = Yuv420 { y: vec![0; 4], u: vec![0], v: vec![0] };
        let error = software(spec(), None).encode(0, &wrong).unwrap_err();
        assert_eq!(error.to_string(), "the frame's planes are not those of a 64x48 picture");
    }

    #[test]
    fn packets_leave_in_the_order_they_were_fed_at_the_times_they_were_fed_with() {
        let spec = VideoSpec { rate_num: 30000, rate_den: 1001, ..spec() };
        let packets = encoded(&mut software(spec, None), spec, 0..8);
        let times: Vec<i64> = packets.iter().map(|packet| packet.pts).collect();
        assert_eq!(times, (0..8).map(|index| spec.presentation_time(index)).collect::<Vec<_>>());
        assert!(packets.first().is_some_and(|packet| packet.key));
    }

    /// A record another encoder could have written at `profile` and `level`, its sequence parameter sets the
    /// software encoder's own under `sps_ids`, and its picture parameter sets its own under each pair of `pps_ids`
    /// (the set's id, then the sequence parameter set's).
    fn record_naming(of: &SoftwareEncoder, profile: u8, level: u8, sps_ids: &[u32], pps_ids: &[(u32, u32)]) -> Vec<u8> {
        let (sps, pps) = of.encoder.sps_pps();
        let own = AvcConfig::parse(&of.encoder.avcc()).unwrap();
        AvcConfig {
            profile,
            level,
            sps: sps_ids.iter().map(|&id| renamed(&sps, SPS_HEAD_BITS, &[id]).unwrap()).collect(),
            pps: pps_ids.iter().map(|&(id, sps_id)| renamed(&pps, 0, &[id, sps_id]).unwrap()).collect(),
            ..own
        }
        .to_bytes()
    }

    /// `value`'s fields as text, with `blank` run over it first: two sets that differ only where `blank` writes
    /// read alike.
    fn read_without<T: Clone + std::fmt::Debug>(value: &T, blank: impl FnOnce(&mut T)) -> String {
        let mut value = value.clone();
        blank(&mut value);
        format!("{value:?}")
    }

    #[test]
    fn a_stream_taken_over_keeps_the_first_encoders_sets_and_names_its_own_by_ids_they_leave_free() {
        let spec = VideoSpec { keyframe_interval: 3, ..spec() };
        let first = software(spec, None);
        let taken = record_naming(&first, 100, 30, &[0, 1], &[(0, 0), (1, 1)]);
        let mut config = configured(&spec).unwrap();
        // Several slices to a picture, as the encoder makes on a phone with its threads.
        config.slices = 3;
        let mut encoder = SoftwareEncoder::over(config.clone(), spec, after(&taken)).unwrap();
        let record = AvcConfig::parse(&encoder.record().unwrap()).unwrap();
        let theirs = AvcConfig::parse(&taken).unwrap();
        assert_eq!((record.sps.get(..2), record.pps.get(..2)), (Some(&theirs.sps[..]), Some(&theirs.pps[..])), "the first encoder's sets, as they were");
        let (sps, pps) = (record.sps.last().unwrap(), record.pps.last().unwrap());
        let (own_sps, own_pps) = encoder.encoder.sps_pps();
        // A set renamed and renamed back is the set the encoder wrote, byte for byte, its trailing bits too: the
        // codes of other lengths move every bit after them.
        let back = |set: &[u8], head, there: &[u32], again: &[u32]| renamed(&renamed(set, head, there).unwrap(), head, again).unwrap();
        assert_eq!((back(&own_sps, SPS_HEAD_BITS, &[7], &[0]), back(&own_pps, 0, &[9, 7], &[0, 0])), (own_sps.clone(), own_pps.clone()));
        let (renamed_sps, original_sps) = (parsed_sps(sps).unwrap(), parsed_sps(&own_sps).unwrap());
        assert_eq!((renamed_sps.id, original_sps.id), (2, 0), "the lowest id the first encoder's sets leave free");
        assert_eq!(read_without(&renamed_sps, |sps| sps.id = 0), format!("{original_sps:?}"), "the same set under another id");
        let renamed_pps = parsed_pps(pps, std::slice::from_ref(&renamed_sps)).unwrap();
        assert_eq!((renamed_pps.id, renamed_pps.sps_id), (2, 2));
        let blank_ids = |pps: &mut Pps| (pps.id, pps.sps_id) = (0, 0);
        let original_pps = parsed_pps(&own_pps, std::slice::from_ref(&original_sps)).unwrap();
        assert_eq!(read_without(&renamed_pps, blank_ids), format!("{original_pps:?}"));
        let packets = encoded(&mut encoder, spec, 0..7);
        assert_eq!(packets.iter().filter(|packet| packet.key).count(), 3, "a key picture every three");
        // The same pictures from an encoder that takes nothing over, which writes the same stream under its own ids.
        let own = encoded(&mut SoftwareEncoder::over(config, spec, None).unwrap(), spec, 0..7);
        let mut slices = 0;
        for (packet, own) in packets.iter().zip(&own) {
            assert_eq!(units(&packet.data).len(), units(&own.data).len());
            for (unit, own) in units(&packet.data).into_iter().zip(units(&own.data)) {
                let ((header, rbsp), (_, own_rbsp)) = (rbsp_of(unit).unwrap(), rbsp_of(own).unwrap());
                let kind = header & 0x1f;
                assert!(kind != nal_type::SPS && kind != nal_type::PPS, "every parameter set is in the record, none in a picture");
                if kind != nal_type::SLICE && kind != nal_type::IDR {
                    assert_eq!(unit, own, "a unit that is no slice is left as it was");
                    continue;
                }
                let nal = NalHeader::parse(header).unwrap();
                let (slice, ..) = SliceHeader::parse(&rbsp, nal, |_| Ok((&renamed_pps, &renamed_sps))).unwrap();
                let (own_slice, ..) = SliceHeader::parse(&own_rbsp, nal, |_| Ok((&original_pps, &original_sps))).unwrap();
                assert_eq!(slice.pps_id, 2, "each slice names the renamed set");
                let blank = |slice: &mut SliceHeader| (slice.pps_id, slice.header_bits) = (0, 0);
                assert_eq!(read_without(&slice, blank), read_without(&own_slice, blank), "the header is the slice's own but for the set it names");
                // CABAC's alignment bits are ones up to the byte its slice data starts on, and the data is the slice's.
                let start = slice.header_bits.div_ceil(8);
                let mut alignment = BitReader::new(&rbsp);
                alignment.seek_bits(slice.header_bits);
                let ones = (start * 8 - slice.header_bits) as u32;
                assert_eq!(alignment.read_bits(ones).unwrap(), (1u32 << ones) - 1, "cabac_alignment_one_bit");
                assert_eq!(rbsp.get(start..), own_rbsp.get(own_slice.header_bits.div_ceil(8)..), "the slice data, byte for byte");
                slices += 1;
            }
        }
        assert_eq!(slices, 3 * 7, "three slices a picture, each renamed");
    }

    #[test]
    fn a_stream_taken_over_decodes_from_the_record_alone_with_every_picture_within_d12() {
        // The hardware's pictures are another software encoder's here, so the switch runs on every host: one of
        // Main profile, whose parameter sets differ from the software encoder's as a hardware encoder's do, and
        // take the ids the software encoder's must leave alone.
        encodetests::check_session_lost_midway(|spec| {
            let mut config = configured(&spec)?;
            config.profile = Profile::Main;
            SoftwareEncoder::over(config, spec, None)
        });
    }

    #[test]
    fn the_record_of_a_continued_stream_names_the_highest_profile_and_level_and_the_flags_both_sets_raise() {
        let encoder = software(spec(), None);
        let own = AvcConfig::parse(&encoder.encoder.avcc()).unwrap();
        let merged = |profile, level, compatibility| {
            let mut first = AvcConfig::parse(&record_naming(&encoder, profile, level, &[0], &[(0, 0)])).unwrap();
            first.compatibility = compatibility;
            AvcConfig::parse(&SoftwareEncoder::start(spec(), after(&first.to_bytes())).unwrap().record().unwrap()).unwrap()
        };
        let same = merged(own.profile, own.level.saturating_add(10), 0x0c);
        assert_eq!((same.profile, same.level), (own.profile, own.level.saturating_add(10)), "the higher level is the record's");
        assert_eq!(same.compatibility, own.compatibility & 0x0c, "a flag the software's set does not raise is not the stream's");
        // A Constrained Baseline encoder raises constraint_set0 and 1 (0xc0), a hardware record 1 and 2 (0x60): the stream
        // stands only on the one both raise, which no flag of either encoder alone gives.
        let mut config = configured(&spec()).unwrap();
        config.profile = Profile::Baseline;
        let baseline = SoftwareEncoder::over(config.clone(), spec(), None).unwrap();
        let mut first = AvcConfig::parse(&record_naming(&baseline, 100, 30, &[0], &[(0, 0)])).unwrap();
        first.compatibility = 0x60;
        let both = SoftwareEncoder::over(config, spec(), after(&first.to_bytes())).unwrap();
        assert_eq!(AvcConfig::parse(&both.record().unwrap()).unwrap().compatibility, 0x40, "the flags both encoders raise");
        let lower = merged(66, own.level.saturating_sub(1), 0xc0);
        assert_eq!((lower.profile, lower.level), (own.profile, own.level), "the software encoder's profile and level are the higher");
    }

    /// The `idr_pic_id` of every key picture in `packets`, read with the sets of `record`.
    fn key_ids(packets: &[Packet], record: &[u8]) -> Vec<Option<u32>> {
        let record = AvcConfig::parse(record).unwrap();
        let spss: Vec<Sps> = record.sps.iter().map(|nal| parsed_sps(nal).unwrap()).collect();
        let ppss: Vec<Pps> = record.pps.iter().map(|nal| parsed_pps(nal, &spss).unwrap()).collect();
        packets.iter().map(|packet| key_picture_id(packet, &spss, &ppss)).collect()
    }

    /// The pictures of a movie that went on in `SoftwareEncoder` after `before` pictures of another stream that
    /// keys every `interval`: that stream's and the software's key picture numbers, and the first software
    /// picture's index.
    fn key_ids_across_a_switch(interval: u32, before: u64) -> (Vec<Option<u32>>, usize) {
        let spec = VideoSpec { keyframe_interval: interval, ..spec() };
        let mut config = configured(&spec).unwrap();
        config.profile = Profile::Main;
        let mut hardware = SoftwareEncoder::over(config, spec, None).unwrap();
        let first = encoded(&mut hardware, spec, 0..before);
        let handover = Handover { record: &hardware.record().unwrap(), last: first.last().unwrap() };
        let mut software = software(spec, Some(handover));
        let then = encoded(&mut software, spec, before..before + 4);
        let ids = key_ids(&first.iter().chain(&then).cloned().collect::<Vec<_>>(), &software.record().unwrap());
        (ids, first.len())
    }

    #[test]
    fn two_key_pictures_in_a_row_never_share_an_idr_pic_id_across_the_switch() {
        // Every picture a key picture: the hardware's last is numbered 0 after one picture and 1 after two.
        for before in [1, 2] {
            let (ids, switch) = key_ids_across_a_switch(1, before);
            assert!(ids.iter().all(Option::is_some), "{ids:?}");
            assert!(ids.windows(2).all(|pair| pair[0] != pair[1]), "{before} pictures before the switch (at {switch}): {ids:?}");
        }
    }

    #[test]
    fn a_hardware_picture_that_is_no_key_picture_leaves_the_softwares_numbers_as_they_are() {
        let (ids, switch) = key_ids_across_a_switch(3, 2);
        assert_eq!(ids.get(switch..).and_then(|ids| ids.first()), Some(&Some(0)), "{ids:?}");
    }

    #[test]
    fn a_stream_the_software_encoder_cannot_name_or_read_is_not_continued() {
        let encoder = software(spec(), None);
        let refusal =
            |first: &[u8]| SoftwareEncoder::start(spec(), after(first)).err().map(|error| (matches!(error, EncodeError::Unavailable(_)), error.to_string()));
        let (unavailable, words) = refusal(&record_naming(&encoder, 110, 40, &[0], &[(0, 0)])).unwrap();
        assert!(unavailable);
        assert_eq!(words, "H.264 profile 110 or 100 is one the software encoder cannot continue");
        assert!(refusal(&[1, 2]).unwrap().1.starts_with("the hardware encoder's record does not parse: "));
        let mut other = AvcConfig::parse(&encoder.encoder.avcc()).unwrap();
        other.length_size = 2;
        assert_eq!(refusal(&other.to_bytes()).unwrap().1, "the two encoders' packets do not carry the same length prefix");
        // The record lists at most 31 sequence parameter sets and 255 picture parameter sets: one short of that,
        // the software's take the record's last places and the ids the first encoder's leave.
        let ids = |sets: usize| (0..sets as u32).collect::<Vec<_>>();
        let pps_over = |sets: usize| ids(sets).into_iter().map(|id| (id, 0)).collect::<Vec<_>>();
        let first = record_naming(&encoder, 100, 30, &ids(RECORD_SPS - 1), &pps_over(RECORD_PPS - 1));
        let record = AvcConfig::parse(&SoftwareEncoder::start(spec(), after(&first)).unwrap().record().unwrap()).unwrap();
        assert_eq!((record.sps.len(), record.pps.len()), (RECORD_SPS, RECORD_PPS));
        let sps = parsed_sps(record.sps.last().unwrap()).unwrap();
        assert_eq!((sps.id, parsed_pps(record.pps.last().unwrap(), &[sps]).unwrap().id), (RECORD_SPS as u32 - 1, RECORD_PPS as u32 - 1));
        assert_eq!(refusal(&record_naming(&encoder, 100, 30, &ids(RECORD_SPS), &[(0, 0)])).unwrap().1, NO_ROOM);
        assert_eq!(refusal(&record_naming(&encoder, 100, 30, &[0], &pps_over(RECORD_PPS))).unwrap().1, NO_ROOM);
    }

    #[test]
    fn a_slice_that_is_not_cabac_coded_is_not_renamed() {
        let first = record_naming(&software(spec(), None), 100, 30, &[0], &[(0, 0)]);
        let mut config = configured(&spec()).unwrap();
        config.profile = Profile::Baseline;
        let mut encoder = SoftwareEncoder::over(config, spec(), after(&first)).unwrap();
        let error = encoder.encode(0, &picture_of(64, 48, 0)).unwrap_err();
        assert_eq!(error.to_string(), "the software encoder failed: its pictures are not CABAC-coded");
    }
}
