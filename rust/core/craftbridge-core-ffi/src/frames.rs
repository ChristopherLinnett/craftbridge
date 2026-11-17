//! A rendered frame and the format it is asked for.

pub use craftbridge_core::{FrameBytes, PixelFormat};

// Exported as the core defines them, as the error is (see `error.rs`).
#[uniffi::remote(Enum)]
pub enum PixelFormat {
    Png,
    Rgba8Straight,
    Bgra8Premultiplied,
}

#[uniffi::remote(Record)]
pub struct FrameBytes {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub data: Vec<u8>,
}
