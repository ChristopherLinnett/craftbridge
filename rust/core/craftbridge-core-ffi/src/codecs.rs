//! The device's codec list an app hands over before it starts an engine.

pub use craftbridge_core::CodecInfo;

// Exported as the core defines it, as the error is (see `error.rs`): the record Android's SDK fills from
// `MediaCodecList`, one entry for each codec, so the boundary has no second copy of its fields.
#[uniffi::remote(Record)]
pub struct CodecInfo {
    pub name: String,
    pub types: Vec<String>,
    pub encoder: bool,
    pub hardware_accelerated: bool,
}

/// Hand over the device's codec list, `MediaCodecList(MediaCodecList.REGULAR_CODECS)` in its order, before
/// an engine is started on Android: the start registers the hardware decoders from it, and
/// `info.hardwareDecoding` reports whether it named one; an export that asks for hardware encoding takes its
/// encoder from it too. There an engine started with no list decodes every stream, and encodes every export,
/// in software. The NDK lists codecs, and says which are hardware, only from API 36, so Android's SDK reads
/// the list itself. A platform with no codec list ignores it: on iOS the start asks VideoToolbox.
#[uniffi::export]
pub fn set_device_codecs(codecs: Vec<CodecInfo>) {
    craftbridge_core::set_device_codecs(codecs);
}
