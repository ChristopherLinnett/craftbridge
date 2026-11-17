//! The preview surface on iOS and iPadOS: the app's `CAMetalLayer`, which it hands us as a pointer and
//! Core Animation composes on screen (plan §20).
//!
//! The layer is held for as long as the surface made from it lives (§20 L6). And the surface re-reads the
//! layer's size on every frame: Metal never says a drawable went outdated — a resized layer simply stops
//! matching what the surface was configured at — so reading the live layer is the only way the core's
//! outdated rule (§17) can fire there at all.

use crate::engine::Engine;
use crate::error::CraftError;
use crate::platform::{Held, OnSurface};
use craftbridge_core::{CraftError as CoreError, GpuDevice, Surface, SurfaceMaker};
use objc2::rc::Retained;
use objc2_quartz_core::CAMetalLayer;
use std::os::raw::c_void;

/// The app's layer, held across the thread boundary: the pointer arrives on the caller's thread, and the
/// engine thread attaches its surface.
struct AppLayer(Retained<CAMetalLayer>);

// SAFETY: a layer's retain count is atomic, so the retain this holds may be taken and given back on any
// thread, and Core Animation takes a layer changed from any thread: `CATransaction` gives a thread that
// changes the layer tree with no transaction of its own an implicit one. All this does off the thread the
// layer came from is make the surface from it and read its bounds and contents scale; a read that meets
// the app's resize halfway reads a size the next present finds changed and configures again for (§17).
// wgpu's own Metal surface holds the same layer the same way, and is `Send` (`wgpu-hal`'s
// `metal::Surface`).
unsafe impl Send for AppLayer {}

impl Held for AppLayer {
    /// The size, in pixels, the layer is drawing at now: its bounds in points, scaled by its contents
    /// scale — the same product the layer itself sizes its drawables by. A scale or a bound that is not
    /// a positive, finite number is no size yet: it answers zero, which the core previews nothing into.
    fn pixel_size(&self) -> (u32, u32) {
        let scale = self.0.contentsScale();
        let bounds = self.0.bounds();
        let side = |points: f64| -> u32 {
            let pixels = (points * scale).round();
            if pixels.is_finite() && pixels > 0.0 { if pixels >= f64::from(u32::MAX) { u32::MAX } else { pixels as u32 } } else { 0 }
        };
        (side(bounds.size.width), side(bounds.size.height))
    }
}

/// The preview surface on the app's layer: the wgpu surface made from the layer on the engine's device,
/// on top of the layer itself, which the surface holds for as long as it lives.
fn preview(gpu: &GpuDevice, layer: AppLayer) -> craftbridge_core::Result<OnSurface<AppLayer>> {
    let instance = gpu.instance().ok_or_else(|| CoreError::gpu("there is no device to preview on"))?;
    let pointer = std::ptr::from_ref(&*layer.0).cast::<c_void>();
    // SAFETY: wgpu asks for a valid object to make the surface on, and the pointer is the `CAMetalLayer`
    // `layer` holds a retain of, live for the whole call. Past the call wgpu's Metal surface takes a
    // retain of its own (`wgpu-hal`'s `metal::Surface::from_layer`), and the `OnSurface` made here drops
    // the surface before the layer's holder, so the layer outlives every read the surface makes of it.
    let surface = unsafe { instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(pointer.cast_mut())) }
        .map_err(|failure| CoreError::gpu(format!("the layer cannot become a preview surface: {failure}")))?;
    Ok(OnSurface { surface, holder: layer })
}

/// The attach `attach_metal_layer` makes on a platform with Core Animation: retain the layer the pointer
/// names, and hand the engine the call that makes its surface on the engine's device.
pub(super) async fn attach(engine: &Engine, layer: u64) -> Result<(), CraftError> {
    let pointer = layer as *mut CAMetalLayer;
    // SAFETY: `retain` asks for a pointer that is null or names a live object of the type it is read as.
    // The app made this one with `passUnretained` from the `CAMetalLayer` it passes, and keeps that layer
    // alive across the call (§20); `retain` takes a retain of our own from it, which the surface's holder
    // gives back when the preview ends. `passUnretained` hands over no retain, so taking ownership of
    // one (`from_raw`) would release the app's own reference when the preview ends.
    let layer = unsafe { Retained::retain(pointer) }.ok_or_else(|| CraftError::invalid_params("that pointer names no layer"))?;
    let layer = AppLayer(layer);
    let make: SurfaceMaker = Box::new(move |gpu: &GpuDevice| Surface::new(Box::new(preview(gpu, layer)?), gpu));
    engine.attach_surface(make).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::attach_metal_layer;
    use crate::surface_tests::{a_preview_follows_the_gpu, empty_preview_keeps_the_session, engine, engine_on_a_device};
    use objc2::runtime::NSObjectProtocol;
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use std::cell::Cell;
    use std::sync::{Arc, Mutex};

    /// A layer the size the test asks for, at a contents scale where points and pixels differ: that way
    /// a change of the layer is visible only through a fresh read of it, which is exactly what the
    /// outdated rule depends on Metal never reporting for itself.
    fn layer_of_side(points: f64) -> Retained<CAMetalLayer> {
        let layer = CAMetalLayer::new();
        layer.setContentsScale(2.0);
        layer.setBounds(CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: points, height: points } });
        layer
    }

    /// The iOS attach test (plan §20), against a real Metal device and a real layer: the attached layer
    /// hands a real texture to a present, resizing it answers the outdated `Gpu` whose words promise the
    /// next frame draws, the next frame does draw, and after the detach the present is the empty answer
    /// with the session kept. The layer's pointer is handed over as Swift's `passUnretained` hands it,
    /// with no retain of its own.
    #[test]
    fn a_layer_draws_and_a_resized_one_answers_gpu_until_the_next_frame_draws() {
        let engine = engine_on_a_device();
        let layer = layer_of_side(32.0);
        let attached = pollster::block_on(attach_metal_layer(engine.clone(), Retained::as_ptr(&layer) as u64));
        assert_eq!(attached, Ok(()), "the app's layer attached as the preview surface");
        assert_eq!(pollster::block_on(engine.present("null".into())), Ok(()), "a real texture was drawn and shown");
        layer.setBounds(CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: 24.0, height: 24.0 } });
        assert_eq!(
            pollster::block_on(engine.present("null".into())),
            Err(CraftError::gpu("the preview surface changed size or format; it is configured again, so the next frame draws")),
            "a resized layer is what outdated means on Metal"
        );
        assert_eq!(pollster::block_on(engine.present("null".into())), Ok(()), "and the next frame draws");
        pollster::block_on(engine.detach_preview()).expect("the detach answers");
        empty_preview_keeps_the_session(&engine);
    }

    /// The layer's preview through the GPU's life: dropped on `suspend_gpu` and drawn again once the app
    /// attaches its layer again after `resume_gpu`, which the same layer allows since the surface made
    /// from it went with the GPU.
    #[test]
    fn a_layer_is_dropped_on_suspend_and_draws_again_once_attached_after_resume() {
        let engine = engine_on_a_device();
        let layer = layer_of_side(32.0);
        // Metal never reports a resize, so a present that answers the outdated `Gpu` after the layer changed
        // size is one that read the live layer: the frame went through a real surface.
        let side = Cell::new(32.0);
        let drew = || {
            side.set(if side.get() > 28.0 { 24.0 } else { 32.0 });
            layer.setBounds(CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: side.get(), height: side.get() } });
            let outdated = CraftError::gpu("the preview surface changed size or format; it is configured again, so the next frame draws");
            assert_eq!(pollster::block_on(engine.present("null".into())), Err(outdated), "the present read the layer's live size");
            assert_eq!(pollster::block_on(engine.present("null".into())), Ok(()), "and the next frame draws");
        };
        a_preview_follows_the_gpu(&engine, || pollster::block_on(attach_metal_layer(engine.clone(), Retained::as_ptr(&layer) as u64)), drew);
    }

    /// The layer arrives as Swift's `passUnretained` hands it, with no retain of its own (§20): the attach
    /// takes one for as long as the layer is attached, and once the preview and the engine have ended,
    /// every retain the attach took is given back and none of the app's, so the app's own references are
    /// the ones left. Releasing a retain the attach never took would free the app's layer under it. The
    /// count is read after the engine stops, because wgpu lets go of its own hold on the layer only
    /// as the device that configured it goes.
    #[test]
    fn an_unretained_layer_is_held_while_attached_and_all_its_retains_come_back() {
        let engine = engine_on_a_device();
        let layer = layer_of_side(16.0);
        let before = layer.retainCount();
        let attached = pollster::block_on(attach_metal_layer(engine.clone(), Retained::as_ptr(&layer) as u64));
        assert_eq!(attached, Ok(()), "the app's layer attached as the preview surface");
        assert!(layer.retainCount() > before, "the attached layer is held by the engine, not only by the app");
        pollster::block_on(engine.detach_preview()).expect("the detach answers");
        pollster::block_on(engine.shut_down()).expect("the engine stops");
        assert_eq!(layer.retainCount(), before, "the preview gave back every retain it took, and none of the app's");
    }

    /// A holder that notes, as it lets go of its layer, how many surfaces its instance still holds.
    struct Noting {
        layer: AppLayer,
        instance: wgpu::Instance,
        surfaces_at_drop: Arc<Mutex<Option<usize>>>,
    }

    impl Held for Noting {
        fn pixel_size(&self) -> (u32, u32) {
            self.layer.pixel_size()
        }
    }

    impl Drop for Noting {
        fn drop(&mut self) {
            *self.surfaces_at_drop.lock().unwrap() = self.instance.generate_report().map(|report| report.surfaces().num_kept_from_user);
        }
    }

    /// wgpu asks that what a surface was made from stay valid until after the surface is dropped: the
    /// preview's surface goes first, so when the holder lets go of the layer no surface made from it is
    /// left.
    #[test]
    fn the_surface_is_dropped_before_the_layer_it_was_made_from() {
        let instance = wgpu::Instance::default();
        let layer = layer_of_side(8.0);
        let pointer = Retained::as_ptr(&layer).cast::<c_void>().cast_mut();
        // SAFETY: the test's own `layer` keeps the layer live across the call and past the surface's end.
        let surface = unsafe { instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(pointer)) }.unwrap();
        let noted = Arc::new(Mutex::new(None));
        let held = OnSurface { surface, holder: Noting { layer: AppLayer(layer.clone()), instance: instance.clone(), surfaces_at_drop: noted.clone() } };
        assert_eq!(instance.generate_report().map(|report| report.surfaces().num_kept_from_user), Some(1), "the surface is live");
        drop(held);
        assert_eq!(*noted.lock().unwrap(), Some(0), "no surface made from the layer was left when its holder let go");
    }

    /// A pointer with nothing behind it is the caller's mistake, answered `InvalidParams` with the engine
    /// answering on: a null layer is a call the app should not have made, not an engine in trouble.
    #[test]
    fn a_layer_that_is_nothing_is_refused() {
        let engine = engine();
        assert_eq!(pollster::block_on(attach_metal_layer(engine.clone(), 0)), Err(CraftError::invalid_params("that pointer names no layer")));
        assert_eq!(pollster::block_on(engine.request("echo".into(), "2".into())), Ok("2".to_string()));
    }
}
