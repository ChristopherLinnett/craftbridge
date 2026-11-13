//! The EffectCraft distribution library: the one native library an app links.
//!
//! Two Rust static libraries in one app collide on the standard library's symbols, so the core's and
//! the plugin's exports are linked here, once.
