//! Hardware video decoders that every Craft app plugin imports.
//!
//! VideoToolbox on iOS and MediaCodec on Android, registered through FilmCraft's public decoder
//! registry so the software decoders stay the fallback. Platform code lives in `platform/`.

mod fallback;

use filmcraft_codecs::hw::{StreamInfo, hardware_decoding, note_hw_declined, set_hardware_decoding, set_hw_backend};
use filmcraft_codecs::{CodecError, Result, VideoDecoder, register_video_decoder};
use filmcraft_isobmff::SampleEntry;

pub use fallback::Fallback;

/// What [`register`] left the process able to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// The app turned hardware decoding off: every stream goes to the software decoders.
    Off,
    /// This build has no hardware decoder, so every stream goes to the software decoders.
    Unavailable,
    /// Streams a hardware decoder supports are decoded by it; the name is the backend's.
    Hardware(&'static str),
}

/// The hardware backend this build carries, if any. Each platform backend sets it where it is built.
const fn backend() -> Option<&'static str> {
    None
}

/// The platform's hardware decoder for the stream `entry` describes, built from `stream` (what
/// FilmCraft's hardware-decoder support reads from `entry`) and wrapped in [`Fallback`]; an error when
/// the platform has none for it or cannot start one.
fn hardware_decoder(_entry: &SampleEntry, _stream: StreamInfo) -> Result<Box<dyn VideoDecoder>> {
    Err(CodecError::Unsupported("this build has no hardware video decoder".into()))
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
    let backend = backend();
    if let Some(name) = backend {
        // FilmCraft's own diagnostics name the backend registered at start.
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

    /// A stream no hardware decoder takes.
    pub(crate) fn prores_entry() -> SampleEntry {
        SampleEntry::video(FourCc(*b"apch"), CodecConfig::ProRes { fourcc: FourCc(*b"apch") }, 8, 8)
    }
}

#[cfg(test)]
mod tests {
    use filmcraft_codecs::hw::hw_stats;
    use filmcraft_codecs::{software_video_decoder, video_decoder_registered};
    use filmcraft_isobmff::AvcConfig;

    use super::*;
    use crate::fixtures::{avc_entry, pps, prores_entry};

    #[test]
    fn register_puts_the_factory_in_filmcrafts_registry_and_sets_the_switch() {
        assert_eq!(register(true), Availability::Unavailable);
        assert!(video_decoder_registered(factory));
        assert!(hardware_decoding());
        assert_eq!(register(false), Availability::Off);
        assert!(!hardware_decoding());
        assert_eq!(register(true), Availability::Unavailable);
        assert!(hardware_decoding());
    }

    #[test]
    fn a_hardware_decoder_that_cannot_start_declines_and_never_answers_its_error() {
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
}
