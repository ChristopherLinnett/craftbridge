//! The EffectCraft distribution library: the one native library an app links.
//!
//! Two Rust static libraries in one app collide on the standard library's symbols, so the core's and
//! the plugin's exports are linked here, once.

// Naming the core's boundary here is what links its scaffolding into this library: a dependency no
// code refers to is left out of a native library, and its exports with it.
pub use craftbridge_core_ffi;
