//! The device's codec list, as the app hands it over.

/// One entry of the device's codec list, `MediaCodecList(MediaCodecList.REGULAR_CODECS)`, with what its
/// `MediaCodecInfo` answers. The regular codecs are the ones the platform's own lookup by type chooses from;
/// the others work only with special surfaces, secure-only or tunnelled-only ones, and an image reader's
/// surface is neither.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodecInfo {
    /// `getName()`: the name the codec is created by.
    pub name: String,
    /// `getSupportedTypes()`: the MIME types the codec takes.
    pub types: Vec<String>,
    /// `isEncoder()`.
    pub encoder: bool,
    /// `isHardwareAccelerated()` (API 29, the SDK's minSdk): the platform's own answer to whether the codec
    /// runs in hardware.
    pub hardware_accelerated: bool,
}
