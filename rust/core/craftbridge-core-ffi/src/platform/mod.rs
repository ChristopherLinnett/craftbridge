//! The surface the app gives the engine to preview on: its Metal layer or its Android window, each made
//! into the core's preview surface and driven by the same rules there — a surface that is lost or outdated
//! answers `Gpu` with the session kept (plan §17, §20).
//!
//! The boundary's hand-written `unsafe` lives here and nowhere else in the crate: the layer pointer the
//! app hands over, the JNI call that takes Android's window in, and the wgpu surfaces made from them.
//! The id table that carries a window between that call and `attach_window` is plain code.
//!
//! Every attach call, and the window's address, is exported from every build, each answering
//! `Unsupported` where its platform's object does not exist. The Swift and Kotlin bindings are generated once, from the host's build of the
//! library (§20), and each binding names every call that build exports: a call compiled for one platform
//! alone would be missing from the bindings generated on another host, or named in bindings whose
//! library lacks it, and UniFFI's Kotlin binds every call it names as the library loads.

use crate::engine::Engine;
use crate::error::CraftError;
use craftbridge_core::surface::Swapchain;
use std::sync::Arc;

/// How an attach asks the preview to write the composition's alpha. Exported as the core defines it,
/// as the GPU choice is (see `gpu.rs`): the boundary keeps no second enum to keep in step with the first.
pub use craftbridge_core::PreviewAlpha;

// Exported as the core defines it, as the error and the GPU choice are (see `error.rs`, `gpu.rs`).
#[uniffi::remote(Enum)]
pub enum PreviewAlpha {
    Opaque,
    Premultiplied,
}

// The id table is the Android window's, but it is plain code: it stays compiled, and tested, everywhere.
#[cfg(any(test, target_os = "android"))]
mod ids;

#[cfg(any(target_os = "ios", target_os = "macos"))]
mod ios;

/// The layer's attach on a build with no Core Animation.
#[cfg(not(any(target_os = "ios", target_os = "macos")))]
mod ios {
    use crate::engine::Engine;
    use crate::error::CraftError;

    pub(super) async fn attach(_: &Engine, _: u64) -> Result<(), CraftError> {
        Err(super::absent("CAMetalLayer"))
    }
}

#[cfg(target_os = "android")]
mod android;

/// The window's attach on a build with no Android window.
#[cfg(not(target_os = "android"))]
mod android {
    use crate::engine::Engine;
    use crate::error::CraftError;

    pub(super) async fn attach(_: &Engine, _: i64) -> Result<(), CraftError> {
        Err(super::absent("Android window"))
    }

    pub(super) async fn attach_with_alpha(_: &Engine, _: i64, _: super::PreviewAlpha) -> Result<(), CraftError> {
        Err(super::absent("Android window"))
    }

    pub(super) fn address(_: i64) -> Result<i64, CraftError> {
        Err(super::absent("Android window"))
    }
}

/// What an attach answers on a build whose platform has no such object: the app called the other
/// platform's attach.
fn absent(object: &str) -> CraftError {
    CraftError::unsupported(format!("this platform has no {object} to preview on"))
}

/// Attach the app's `CAMetalLayer` as the engine's preview surface (plan §20): Swift passes
/// `Unmanaged.passUnretained(layer).toOpaque()` as an integer, this takes a retain of its own for as long
/// as the layer is attached, and the engine draws every later frame onto the layer for Core Animation to
/// show (see [`Engine::present`]). Attaching replaces whatever surface was previewed;
/// [`Engine::detach_preview`] gives this one up.
///
/// A pointer with no layer behind it is refused with `InvalidParams`, and on a platform with no
/// Core Animation every call answers `Unsupported`.
///
/// [`Engine::present`]: crate::Engine::present
#[uniffi::export]
pub async fn attach_metal_layer(engine: Arc<Engine>, layer: u64) -> Result<(), CraftError> {
    ios::attach(&engine, layer).await
}

#[uniffi::export]
impl Engine {
    /// Attach as the preview surface the window the `id` names — the id the JNI call handed out for the
    /// app's `Surface` (plan §20). The window leaves the table when the attach begins: after a failed one
    /// it is back under its own id, so a retry and a release both still hold; after a successful one its
    /// keep passes to the engine's surface, and [`Self::detach_preview`] gives it up. An id the table
    /// does not hold is `NotFound`, and on a platform with no Android window every call answers
    /// `Unsupported`.
    pub async fn attach_window(&self, id: i64) -> Result<(), CraftError> {
        android::attach(self, id).await
    }

    /// Attach the window the `id` names as the engine's preview surface, as [`Self::attach_window`]
    /// does, and write the composition's alpha into it as `alpha` asks. `Premultiplied` writes the
    /// frame's own alpha, premultiplied, and clears the letterbox bars transparent, so a compositor
    /// that reads premultiplied alpha — a Flutter `Texture` over the app's own content — shows the
    /// composition's transparency. It is written only through a mode of the surface that carries it,
    /// `PreMultiplied` or else Android's `Inherit`: a surface that offers neither is refused as `Gpu`,
    /// naming those modes, never drawn over black or in straight colour in its place, and the app
    /// attaches it with `Opaque` instead. `Opaque` is exactly [`Self::attach_window`]: the frame over
    /// black. The choice is made with the attach and kept with the surface until it is detached or
    /// another is attached. Every refusal and platform answer of [`Self::attach_window`] is this
    /// call's too: on a platform with no Android window it answers `Unsupported` whichever is asked.
    pub async fn attach_window_with_alpha(&self, id: i64, alpha: PreviewAlpha) -> Result<(), CraftError> {
        android::attach_with_alpha(self, id, alpha).await
    }
}

/// The address of the window the `id` names — an id the JNI call handed out for the app's `Surface` —
/// which no other live window shares (plan §20): the Kotlin SDK keeps the address of the window it
/// attached last and compares it with this one's, to tell whether the engine previews into the window a
/// `Surface` names now. The window stays under its id, for the attach or the release after. An id the
/// table does not hold is `NotFound`, and on a platform with no Android window every call answers
/// `Unsupported`.
#[uniffi::export]
pub fn window_address(id: i64) -> Result<i64, CraftError> {
    android::address(id)
}

/// The app's own object underneath a preview surface: what the wgpu surface was made from, and where its
/// live pixel size is read from — the layer's or window's size now, which wgpu's surface cannot answer
/// (§17).
pub(crate) trait Held {
    /// The object's size in pixels now; zero on an axis with no pixels yet, which previews nothing into.
    fn pixel_size(&self) -> (u32, u32);
}

/// One of the platform's preview surfaces: the wgpu surface made from the app's object, and the object
/// itself — held so the pointer the surface was made from names the live object for as long as the
/// surface lives, and so the object ends only when the engine's preview of it does (§20 L6). The
/// surface's work is the same on every platform, so it is written once here; each platform says only
/// what its object is and how it is sized.
pub(crate) struct OnSurface<T: Held> {
    /// Declared before `holder`, so it is dropped first: wgpu asks that what a surface was made from
    /// stay valid until after the surface is dropped, and fields drop in the order they are declared.
    pub(crate) surface: wgpu::Surface<'static>,
    pub(crate) holder: T,
}

impl<T: Held> Swapchain for OnSurface<T> {
    fn capabilities(&self, adapter: &wgpu::Adapter) -> wgpu::SurfaceCapabilities {
        self.surface.get_capabilities(adapter)
    }

    fn configure(&self, device: &wgpu::Device, configuration: &wgpu::SurfaceConfiguration) {
        self.surface.configure(device, configuration);
    }

    fn acquire(&self) -> wgpu::CurrentSurfaceTexture {
        self.surface.get_current_texture()
    }

    fn current_size(&self) -> (u32, u32) {
        self.holder.pixel_size()
    }
}

#[cfg(test)]
mod tests {
    use crate::CraftError;
    use crate::surface_tests::engine;

    /// The window's attach and address are in every build, so the bindings generated on any host name
    /// them, and a build with no Android window answers both `Unsupported` with the engine answering on.
    #[cfg(not(target_os = "android"))]
    #[test]
    fn a_window_where_there_are_none_is_unsupported() {
        let engine = engine();
        let absent = CraftError::unsupported("this platform has no Android window to preview on");
        assert_eq!(pollster::block_on(engine.attach_window(1)), Err(absent.clone()));
        assert_eq!(super::window_address(1), Err(absent), "and so is the address of one");
        assert_eq!(pollster::block_on(engine.request("echo".into(), "3".into())), Ok("3".to_string()));
    }

    /// The layer's attach is in every build too, and a build with no Core Animation answers it
    /// `Unsupported` with the engine answering on.
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    #[test]
    fn a_layer_where_there_are_none_is_unsupported() {
        let engine = engine();
        let answer = pollster::block_on(super::attach_metal_layer(engine.clone(), 8));
        assert_eq!(answer, Err(CraftError::unsupported("this platform has no CAMetalLayer to preview on")));
        assert_eq!(pollster::block_on(engine.request("echo".into(), "3".into())), Ok("3".to_string()));
    }

    /// The alpha's attach is in every build with it, answers `Unsupported` where there is no window
    /// to attach whichever the ask, and carries the ask past a refusal: the engine answers on.
    #[cfg(not(target_os = "android"))]
    #[test]
    fn the_alpha_attach_where_there_are_no_windows_is_unsupported() {
        use super::PreviewAlpha;
        let engine = engine();
        let absent = CraftError::unsupported("this platform has no Android window to preview on");
        assert_eq!(pollster::block_on(engine.attach_window_with_alpha(1, PreviewAlpha::Opaque)), Err(absent.clone()), "the default asks nothing");
        assert_eq!(pollster::block_on(engine.attach_window_with_alpha(1, PreviewAlpha::Premultiplied)), Err(absent), "and so does the one that asks");
        assert_eq!(pollster::block_on(engine.request("echo".into(), "4".into())), Ok("4".to_string()));
    }
}
