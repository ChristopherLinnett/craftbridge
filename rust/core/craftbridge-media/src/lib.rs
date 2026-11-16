//! Hardware video decoders that every Craft app plugin imports.
//!
//! VideoToolbox on iOS and MediaCodec on Android, registered through FilmCraft's public decoder
//! registry so the software decoders stay the fallback. Platform code lives in `platform/`.

#[cfg(any(target_os = "ios", test))]
mod biplanar;
mod fallback;
#[cfg(any(target_os = "ios", test))]
mod layout;
#[cfg(any(target_os = "ios", test))]
mod order;
#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
mod platform;

use filmcraft_codecs::hw::{StreamInfo, hardware_decoding, note_hw_declined, set_hardware_decoding, set_hw_backend};
use filmcraft_codecs::{Result, VideoDecoder, register_video_decoder};
use filmcraft_isobmff::SampleEntry;

pub use fallback::Fallback;

/// What [`register`] left the process able to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// The app turned hardware decoding off: every stream goes to the software decoders.
    Off,
    /// There is no hardware decoder to use: the device reports none, as the iOS Simulator does, or this
    /// build has no backend for its platform. Every stream goes to the software decoders.
    Unavailable,
    /// Streams a hardware decoder supports are decoded by it; the name is the backend's.
    Hardware(&'static str),
}

/// The hardware backend this device has, if any: on iOS VideoToolbox, when it reports a hardware decoder
/// (the Simulator has none, so there every stream goes to the software decoders).
#[cfg(target_os = "ios")]
fn backend() -> Option<&'static str> {
    platform::videotoolbox::available().then_some("VideoToolbox")
}

/// This build has no hardware backend.
#[cfg(not(target_os = "ios"))]
fn backend() -> Option<&'static str> {
    None
}

/// The platform's hardware decoder for the stream `entry` describes, built from `stream` (what
/// FilmCraft's hardware-decoder support reads from `entry`) and wrapped in [`Fallback`]; an error when
/// the platform has none for it or cannot start one.
#[cfg(target_os = "ios")]
fn hardware_decoder(entry: &SampleEntry, stream: StreamInfo) -> Result<Box<dyn VideoDecoder>> {
    let decoder = platform::videotoolbox::VtDecoder::new(entry, &stream)?;
    Ok(Box::new(Fallback::new(decoder, entry, stream)))
}

#[cfg(not(target_os = "ios"))]
fn hardware_decoder(_entry: &SampleEntry, _stream: StreamInfo) -> Result<Box<dyn VideoDecoder>> {
    Err(filmcraft_codecs::CodecError::Unsupported("this build has no hardware video decoder".into()))
}

/// The factory FilmCraft asks first for every movie it opens.
fn factory(entry: &SampleEntry) -> Option<Result<Box<dyn VideoDecoder>>> {
    answer(entry, hardware_decoding(), hardware_decoder)
}

/// What the factory answers for `entry` while hardware decoding is `on`, starting a hardware decoder with
/// `start`. Declining (`None`) hands the stream to FilmCraft's software decoders. A decline is counted
/// where FilmCraft's own hardware factories count one, when a decoder for a stream they take cannot start;
/// with the switch off, or a configuration record that does not parse, they decline uncounted.
fn answer(
    entry: &SampleEntry,
    on: bool,
    start: impl FnOnce(&SampleEntry, StreamInfo) -> Result<Box<dyn VideoDecoder>>,
) -> Option<Result<Box<dyn VideoDecoder>>> {
    if !on {
        return None;
    }
    // The software decoder takes a stream no hardware decoder takes, and reports a record that does not
    // parse.
    let stream = fallback::stream_info(entry)?.ok()?;
    match start(entry, stream) {
        Ok(decoder) => Some(Ok(decoder)),
        // FilmCraft opens a stream with the first factory that answers, so answering the error would leave
        // the movie with no decoder at all.
        Err(_) => {
            note_hw_declined();
            None
        }
    }
}

/// Registers CraftBridge's decoders in FilmCraft's decoder registry, the one every app that decodes
/// through FilmCraft opens movies through, and sets whether they may be used (`on`). Call it once per
/// process at engine start; calling it again only changes the switch. A source picks the switch up when
/// it is reopened.
pub fn register(on: bool) -> Availability {
    register_video_decoder(factory);
    set_hardware_decoding(on);
    report(on, backend())
}

/// What [`register`] reports with hardware decoding `on` and the device's `backend`, which FilmCraft's own
/// diagnostics are told the name of, as they name the backend registered at start.
fn report(on: bool, backend: Option<&'static str>) -> Availability {
    if let Some(name) = backend {
        set_hw_backend(name);
    }
    match (on, backend) {
        (false, _) => Availability::Off,
        (true, None) => Availability::Unavailable,
        (true, Some(name)) => Availability::Hardware(name),
    }
}

/// Sample entries the crate's tests share.
#[cfg(test)]
pub(crate) mod fixtures {
    use filmcraft_isobmff::{AvcConfig, CodecConfig, FourCc, SampleEntry};

    pub(crate) fn sps() -> Vec<u8> {
        vec![
            0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9, 0x40, 0x50, 0x05, 0xBB, 0x01, 0x10, 0x00, 0x00, 0x03, 0x00, 0x10, 0x00, 0x00, 0x03, 0x03, 0xC0, 0xF1, 0x83,
            0x19, 0x60,
        ]
    }

    pub(crate) fn pps() -> Vec<u8> {
        vec![0x68, 0xEB, 0xE3, 0xCB, 0x22, 0xC0]
    }

    /// An `avcC` entry FilmCraft's own parser accepts.
    pub(crate) fn avc_entry() -> SampleEntry {
        SampleEntry::avc(AvcConfig::new(vec![sps()], vec![pps()], 4), 1280, 720)
    }

    const TINY_SPS: [u8; 22] =
        [0x67, 0x42, 0xC0, 0x0A, 0xDA, 0x25, 0xB0, 0x11, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x00, 0x03, 0x00, 0x08, 0x8F, 0x12, 0x26, 0xA0];
    const TINY_PPS: [u8; 5] = [0x68, 0xCE, 0x01, 0x37, 0x20];
    /// A real 32x32 H.264 stream, an IDR picture and three P pictures, encoded by x264 at its smallest and
    /// baseline profile: short enough to decode in a test, long enough to have a sample after a restart one.
    pub(crate) fn tiny_entry() -> SampleEntry {
        SampleEntry::avc(AvcConfig::new(vec![TINY_SPS.to_vec()], vec![TINY_PPS.to_vec()], 4), 32, 32)
    }

    /// The samples of [`tiny_entry`] in decoding order, each with its presentation time.
    pub(crate) fn tiny_samples() -> Vec<(i64, Vec<u8>)> {
        vec![
            (
                0,
                [
                    0x00, 0x00, 0x00, 0xC5, 0x65, 0x88, 0x84, 0x3A, 0x0C, 0x60, 0x02, 0x9C, 0xFD, 0x83, 0x2B, 0xA1, 0xDF, 0xDB, 0x01, 0x1E, 0xBD, 0xB6, 0x7E,
                    0x4C, 0x19, 0x52, 0xFD, 0x93, 0xC9, 0x8C, 0x7C, 0xF1, 0xE4, 0x26, 0x77, 0x3F, 0x0D, 0x06, 0x34, 0x30, 0xE2, 0xB2, 0xD4, 0x6A, 0x6E, 0x8B,
                    0xB0, 0x24, 0xE3, 0x65, 0xEB, 0x4A, 0xB9, 0xED, 0x49, 0x3E, 0x05, 0x87, 0xC0, 0xB0, 0x18, 0xC0, 0x14, 0x79, 0xD9, 0x42, 0x3B, 0x10, 0x03,
                    0x1F, 0xF8, 0xAA, 0xE5, 0xD7, 0xD3, 0xC0, 0x26, 0xEB, 0x4D, 0xF0, 0x3E, 0xD7, 0xB4, 0x75, 0xE6, 0xF8, 0x64, 0x18, 0x00, 0x78, 0x44, 0xC3,
                    0xC3, 0xE8, 0x9D, 0x1A, 0x56, 0xEC, 0x54, 0xDB, 0x91, 0x01, 0xE7, 0xEB, 0x43, 0x7B, 0xDD, 0xAF, 0x81, 0x61, 0xF0, 0x2C, 0x05, 0x9C, 0x02,
                    0x4A, 0x14, 0xF4, 0x8A, 0x5F, 0xDD, 0xEF, 0x00, 0xBE, 0xAF, 0x3D, 0xFE, 0x44, 0x95, 0xCC, 0x19, 0x2A, 0x2C, 0xC7, 0x77, 0x48, 0x29, 0x96,
                    0x0E, 0x00, 0x45, 0x41, 0x00, 0x03, 0x42, 0x46, 0x0C, 0x49, 0x4C, 0x00, 0x7E, 0x8E, 0x06, 0xC8, 0x7A, 0x81, 0x06, 0x33, 0xEE, 0x02, 0xCE,
                    0x00, 0x25, 0xA1, 0xFA, 0x86, 0x4A, 0x2C, 0x3C, 0xDF, 0xC0, 0x8F, 0x6F, 0x3C, 0xB7, 0xE9, 0xF7, 0xED, 0x83, 0x57, 0x2C, 0x24, 0x0F, 0x0C,
                    0x63, 0x85, 0xD0, 0x57, 0x41, 0x21, 0xEE, 0x00, 0xD8, 0x3E, 0x83, 0x13, 0xA6, 0x10, 0xA6, 0xAC, 0x80,
                ]
                .to_vec(),
            ),
            (1, [0x00, 0x00, 0x00, 0x09, 0x41, 0x9A, 0x20, 0xC9, 0xF5, 0x4A, 0xBD, 0x70, 0x65].to_vec()),
            (2, [0x00, 0x00, 0x00, 0x0E, 0x41, 0x9A, 0x41, 0x27, 0xD4, 0xA2, 0xF8, 0x6E, 0x2B, 0xE5, 0xCF, 0xE6, 0xD2, 0x52].to_vec()),
            (
                3,
                [
                    0x00, 0x00, 0x00, 0x20, 0x41, 0x9A, 0x61, 0xA7, 0xC3, 0x71, 0x77, 0x25, 0xE0, 0x1C, 0x01, 0x71, 0x2D, 0x05, 0xBC, 0x40, 0xE8, 0x96, 0x9E,
                    0x18, 0xD4, 0xE4, 0x79, 0xBE, 0x02, 0x22, 0xE5, 0x8E, 0x1A, 0x0B, 0x5F, 0xC0,
                ]
                .to_vec(),
            ),
        ]
    }

    /// Held by a test that reads FilmCraft's process-wide decline counter, or makes a decoder that may bump it,
    /// so another test's decline does not land between its two readings.
    pub(crate) fn counters() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A stream no hardware decoder takes.
    pub(crate) fn prores_entry() -> SampleEntry {
        SampleEntry::video(FourCc(*b"apch"), CodecConfig::ProRes { fourcc: FourCc(*b"apch") }, 8, 8)
    }
}

#[cfg(test)]
mod tests {
    use filmcraft_codecs::hw::{hw_backend, hw_stats};
    use filmcraft_codecs::{CodecError, make_video_decoder, software_video_decoder, video_decoder_registered};
    use filmcraft_isobmff::AvcConfig;

    use super::*;
    use crate::fixtures::{avc_entry, counters, pps, prores_entry, tiny_entry, tiny_samples};

    /// Every build but an iPhone's has no hardware decoder: the iOS Simulator has none, and VideoToolbox is
    /// the only backend this crate carries.
    const NO_HARDWARE: bool = cfg!(any(not(target_os = "ios"), target_abi = "sim"));

    #[test]
    fn register_puts_the_factory_in_filmcrafts_registry_and_sets_the_switch() {
        let _counters = counters();
        let on = if NO_HARDWARE { Availability::Unavailable } else { Availability::Hardware("VideoToolbox") };
        assert_eq!(backend().is_none(), NO_HARDWARE, "the device's own answer");
        assert_eq!(register(true), on);
        assert!(video_decoder_registered(factory));
        assert!(hardware_decoding());
        assert_eq!(register(false), Availability::Off);
        assert!(!hardware_decoding());
        assert_eq!(register(true), on);
        assert!(hardware_decoding());
    }

    #[test]
    fn a_backend_is_named_to_filmcraft_and_reported_only_while_hardware_decoding_is_on() {
        let _counters = counters();
        assert_eq!(report(true, None), Availability::Unavailable);
        assert_eq!(report(false, None), Availability::Off);
        assert_eq!(report(false, Some("a backend")), Availability::Off);
        assert_eq!(hw_backend(), Some("a backend"), "FilmCraft's diagnostics name it even while it is off");
        assert_eq!(report(true, Some("a backend")), Availability::Hardware("a backend"));
    }

    #[test]
    fn a_hardware_decoder_that_cannot_start_declines_and_never_answers_its_error() {
        let _counters = counters();
        let entry = avc_entry();
        let declined = || hw_stats().declined;
        let before = declined();
        assert!(matches!(answer(&entry, true, |entry, _| software_video_decoder(entry)), Some(Ok(_))));
        assert_eq!(declined(), before, "a started decoder is no decline");
        assert!(answer(&entry, true, |_, _| Err(CodecError::Unsupported("no hardware session".into()))).is_none());
        assert_eq!(declined(), before + 1, "a decoder that cannot start is a counted decline");
        // Nothing is started, and nothing counted, while the switch is off, for a stream no hardware decoder
        // takes, or for a record that does not parse: FilmCraft's own factories count none of those.
        let broken = SampleEntry::avc(AvcConfig::new(vec![vec![0x67]], vec![pps()], 4), 16, 16);
        assert!(fallback::stream_info(&broken).is_some_and(|info| info.is_err()));
        assert!(answer(&entry, false, |_, _| panic!("started while hardware decoding is off")).is_none());
        assert!(answer(&prores_entry(), true, |_, _| panic!("started for a stream no hardware decoder takes")).is_none());
        assert!(answer(&broken, true, |_, _| panic!("started for a record that does not parse")).is_none());
        assert_eq!(declined(), before + 1, "only a start that failed is counted");
    }

    /// Every picture a decoder returns for the tiny stream, with all it carries, as text: Debug prints each
    /// plane's every sample, so equal text is equal bytes.
    fn decode_tiny(mut decoder: Box<dyn VideoDecoder>) -> Vec<String> {
        let mut pictures = Vec::new();
        for (pts, sample) in tiny_samples() {
            pictures.extend(decoder.decode(&sample, pts).unwrap());
        }
        pictures.extend(decoder.flush());
        assert_eq!(pictures.len(), 4, "a picture for each sample");
        pictures.iter().map(|picture| format!("{} {} {:?}", picture.pts, picture.draft, picture.frame)).collect()
    }

    /// Where no hardware decoder is available, such as the iOS Simulator, a stream FilmCraft opens through
    /// the registry reaches its software decoder, and decodes to the same bytes it does when asked directly;
    /// on an iPhone the hardware decoder takes it, and decodes to the same bytes too.
    #[test]
    fn a_stream_opened_through_the_registry_decodes_to_the_bytes_the_software_decoder_gives() {
        let _counters = counters();
        register(true);
        let entry = tiny_entry();
        let declined = hw_stats().declined;
        let through_registry = make_video_decoder(&entry).unwrap();
        if NO_HARDWARE {
            assert_eq!(hw_stats().declined, declined + 1, "the stream is one the factory is responsible for and could not start a decoder for");
            assert_eq!(through_registry.name(), software_video_decoder(&entry).unwrap().name(), "the factory declined");
        } else {
            assert_eq!(hw_stats().declined, declined, "the hardware decoder started");
            assert_eq!(through_registry.name(), "VideoToolbox H.264");
        }
        assert_eq!(decode_tiny(through_registry), decode_tiny(software_video_decoder(&entry).unwrap()));
    }
}
