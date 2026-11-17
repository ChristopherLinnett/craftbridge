//! Whether an engine asks for a GPU.

pub use craftbridge_core::GpuChoice;

// Exported as the core defines it, as the error is (see `error.rs`).
#[uniffi::remote(Enum)]
pub enum GpuChoice {
    Preferred,
    CpuOnly,
}
