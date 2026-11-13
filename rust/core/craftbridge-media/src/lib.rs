//! Hardware video decoders that every Craft app plugin imports.
//!
//! VideoToolbox on iOS and MediaCodec on Android, registered through FilmCraft's public decoder
//! registry so the software decoders stay the fallback. Platform code lives in `platform/`.
