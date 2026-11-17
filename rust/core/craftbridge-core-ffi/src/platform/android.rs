//! The preview surface on Android: the app's `Surface`, arriving through the one JNI call as an
//! `ANativeWindow` and held in a process-wide id table until `attach_window` takes it or `release` gives
//! it up (plan §20).
//!
//! The Vulkan swapchain Android builds reports a lost or outdated surface itself, and the core turns what
//! it reports into `Gpu` with the session kept (§17); what this module guarantees is only that the window
//! is live while its surface lives, and that ids mean what the app holds them to mean.

use crate::engine::Engine;
use crate::error::CraftError;
use crate::platform::{Held, OnSurface, ids::IdTable};
use craftbridge_core::{CraftError as CoreError, GpuDevice, Surface, SurfaceMaker};
use ndk::native_window::NativeWindow;
use std::os::raw::c_void;

/// The windows the app has handed us and not yet attached or released, each under the id the app holds.
static WINDOWS: IdTable<NativeWindow> = IdTable::new();

impl Held for NativeWindow {
    /// The size the window is sized at now: an `ANativeWindow` is always its own size, and one that has
    /// none yet — the app has not given its producer a geometry — is zero, which the core previews
    /// nothing into until it grows.
    fn pixel_size(&self) -> (u32, u32) {
        (u32::try_from(self.width()).unwrap_or(0), u32::try_from(self.height()).unwrap_or(0))
    }
}

/// The app's window as a surface target: Vulkan names the display it draws against and the window to
/// draw on; the display on Android is the whole NDK, which `AndroidDisplayHandle` says while holding no
/// pointer of its own, and the window is this one's own pointer.
fn target(window: &NativeWindow) -> wgpu::SurfaceTargetUnsafe {
    wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(wgpu::rwh::RawDisplayHandle::Android(wgpu::rwh::AndroidDisplayHandle::new())),
        raw_window_handle: wgpu::rwh::RawWindowHandle::AndroidNdk(wgpu::rwh::AndroidNdkWindowHandle::new(window.ptr().cast::<c_void>())),
    }
}

/// The preview surface on an app's window: the wgpu surface made from the window's own pointer, on top
/// of the window itself, which the surface holds — so the surface always reads its pointer from a live
/// window, and the window ends only when the engine's preview of it does (§20 L6).
fn preview(gpu: &GpuDevice, window: NativeWindow) -> craftbridge_core::Result<OnSurface<NativeWindow>> {
    let instance = gpu.instance().ok_or_else(|| CoreError::gpu("there is no device to preview on"))?;
    // SAFETY: wgpu asks that a raw window handle name a valid window and stay valid until after the
    // surface is dropped. The pointer is `window`'s, a reference to the window held across the call, and
    // the `OnSurface` made here drops the surface before that reference. Vulkan also takes one of its own
    // for the surface's life (`vkCreateAndroidSurfaceKHR` increments the window's reference count).
    let surface = unsafe { instance.create_surface_unsafe(target(&window)) }
        .map_err(|failure| CoreError::gpu(format!("the window cannot become a preview surface: {failure}")))?;
    Ok(OnSurface { surface, holder: window })
}

/// The app's one JNI function (plan §20): `NativeSurfaces.acquire(surface)`, the Kotlin
/// `io.github.christopherlinnett.craftbridge.core.NativeSurfaces`'s `external fun`. JNI knows a function
/// only by the name spelled from its class's package, so the name stands spelled out here, and the SDK's
/// Kotlin declaration names it back.
///
/// The answer is the id of the window that names `surface`, which Kotlin hands with the engine to
/// `attachWindow`; zero answers when the surface is null, gone or was never one. The window waits in the
/// table until it is attached or released, so an app that does neither before it lets go still gives the
/// window back at its `release`.
///
/// # Safety
///
/// JNI calls this with a live `JNIEnv` and with a `jobject` that names a live `android.view.Surface` or
/// is null, from a thread of the JVM's.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_io_github_christopherlinnett_craftbridge_core_NativeSurfaces_acquire(
    env: *mut c_void,
    _class: *mut c_void,
    surface: *mut c_void,
) -> i64 {
    // `from_surface` asks for a live `android.view.Surface`, and Android's `ANativeWindow_fromSurface`
    // reads a field of whatever object it is handed, so a null one is answered here, before the call.
    if surface.is_null() {
        return 0;
    }
    // SAFETY: the arguments are what JNI passed — a live `JNIEnv` and, past the check above, a live
    // Surface — which is what `from_surface` asks for, and it reads them before this returns. The window
    // it answers carries the reference `ANativeWindow_fromSurface` took, which the table's entry owns.
    match unsafe { NativeWindow::from_surface(env.cast(), surface.cast()) } {
        Some(window) => WINDOWS.insert(window),
        None => 0,
    }
}

/// The companion of the acquire call: `NativeSurfaces.release(id)`, which gives up the window the id
/// holds. An attach that took the id leaves nothing to release, so an id the table does not hold answers
/// nothing — the release can be made whichever way the attach ended. It reads only the id, never the
/// `JNIEnv` or the class, so a caller has nothing to uphold and the function is safe.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_christopherlinnett_craftbridge_core_NativeSurfaces_release(_env: *mut c_void, _class: *mut c_void, id: i64) {
    // Giving up the last reference we hold to the window is what ends our keep of it; the engine's own,
    // if it attached one, lives on with its surface.
    drop(WINDOWS.take(id));
}

/// The attach `Engine::attach_window` makes on Android: take the window out of the table, and hand the
/// engine the call that makes its surface on the engine's device.
pub(super) async fn attach(engine: &Engine, id: i64) -> Result<(), CraftError> {
    let window = WINDOWS.take(id).ok_or_else(|| CraftError::not_found("no window of this process has that id"))?;
    let held = window.clone();
    let make: SurfaceMaker = Box::new(move |gpu: &GpuDevice| Surface::new(Box::new(preview(gpu, held)?), gpu));
    match engine.attach_surface(make).await {
        Ok(()) => Ok(()),
        Err(error) => {
            // The app still carries this id: give it its window back, so it can retry the attach or
            // release the id exactly as it could before the attempt.
            WINDOWS.put(id, window);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface_tests::{a_preview_follows_the_gpu, empty_preview_keeps_the_session, engine, engine_on_a_device};
    use ndk::media::image_reader::{AcquireResult, ImageFormat, ImageReader};
    use std::time::{Duration, Instant};

    /// The app's `Surface` without an app: an `AImageReader`'s window, which is a real producer-connected
    /// surface whose buffers the test can read back. The JNI call's own Kotlin path is the SDK's job;
    /// everything past the id — the table's rules, the attach, the draw — runs here for real.
    fn window_reader() -> ImageReader {
        ImageReader::new(64, 64, ImageFormat::RGBA_8888, 3).expect("the emulator gives a reader for a 64 by 64 RGBA image")
    }

    /// The pixel the drawn frame left at the middle of a buffer the reader received: proof that the blit
    /// pass' picture reached the app's window, and — with its alpha — what the alpha fallback wrote.
    fn middle_pixel(reader: &ImageReader) -> [u8; 4] {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(AcquireResult::Image(image)) = reader.acquire_next_image() {
                let plane = image.plane_data(0).expect("the reader hands CPU-readable buffers");
                let stride = image.plane_row_stride(0).expect("the plane has a row stride") as usize;
                let at = 32 * stride + 32 * 4;
                return [plane[at], plane[at + 1], plane[at + 2], plane[at + 3]];
            }
            assert!(Instant::now() < deadline, "the reader never received a presented frame");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The emulator attach test (plan §20): the attached Android surface offers only the `Inherit` alpha
    /// mode — the loader has nothing else — so the core's fallback to it is what attaches, and a real
    /// texture is acquired, drawn and presented as an opaque picture the reader reads back; attaching
    /// consumes the id, detaching empties the present with the session kept, and a fresh window attaches
    /// and draws again.
    #[test]
    fn a_window_attaches_draws_opaque_and_its_detach_leaves_the_session() {
        let engine = engine_on_a_device();
        let reader = window_reader();
        let window = reader.window().expect("the reader hands out its window");
        // The JNI call's job is `WINDOWS.insert`; this is that insert without a JVM to make the call.
        let id = WINDOWS.insert(window.clone());
        assert!(id > 0, "the window came in with an id");

        // Vulkan holds a window for one surface at a time, so the asking-about surface ends before the
        // engine makes the real one from the same window.
        {
            let instance = wgpu::Instance::default();
            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("the emulator has a device");
            // SAFETY: the test keeps this window reference alive for as long as it reads the surface.
            let probe = unsafe { instance.create_surface_unsafe(target(&window)) }.expect("the window names a surface to ask about");
            let caps = probe.get_capabilities(&adapter);
            assert_eq!(caps.alpha_modes.len(), 1, "the emulator's Vulkan loader offers exactly one alpha mode");
            assert_eq!(caps.alpha_modes.first(), Some(&wgpu::CompositeAlphaMode::Inherit), "and it is Inherit alone");
        }

        assert_eq!(pollster::block_on(engine.attach_window(id)), Ok(()), "the fallback to Inherit attaches");
        assert_eq!(WINDOWS.take(id), None, "the attach consumed the id");
        assert_eq!(pollster::block_on(engine.present("null".into())), Ok(()), "a real texture was drawn and shown");
        assert_eq!(middle_pixel(&reader), [255, 0, 0, 255], "the picture drawn through the Inherit fallback is opaque");

        pollster::block_on(engine.detach_preview()).expect("the detach answers");
        empty_preview_keeps_the_session(&engine);

        let second = window_reader();
        let second_id = WINDOWS.insert(second.window().expect("the second reader hands out its window"));
        assert_eq!(pollster::block_on(engine.attach_window(second_id)), Ok(()));
        assert_eq!(pollster::block_on(engine.present("null".into())), Ok(()));
        assert_eq!(middle_pixel(&second), [255, 0, 0, 255]);
        pollster::block_on(engine.detach_preview()).expect("the detach answers");
    }

    /// The window's preview through the GPU's life: dropped on `suspend_gpu`, which lets go of the
    /// window so Vulkan makes a surface of it again, and drawn again once the app attaches its window
    /// again after `resume_gpu`, as it does with the new id the JNI call hands out.
    #[test]
    fn a_window_is_dropped_on_suspend_and_draws_again_once_attached_after_resume() {
        let engine = engine_on_a_device();
        let reader = window_reader();
        let window = reader.window().expect("the reader hands out its window");
        let drew = || assert_eq!(middle_pixel(&reader), [255, 0, 0, 255], "the frame reached the window");
        a_preview_follows_the_gpu(&engine, || pollster::block_on(engine.attach_window(WINDOWS.insert(window.clone()))), drew);
    }

    /// A frame with soft alpha reaches the window as the blit pass is given it, premultiplied: a CPU
    /// frame's straight red at half alpha shows as half red over black on the opaque surface, where a
    /// straight upload would show it at full red.
    #[test]
    fn a_translucent_cpu_frame_reaches_the_window_premultiplied() {
        let engine = engine_on_a_device();
        let reader = window_reader();
        let id = WINDOWS.insert(reader.window().expect("the reader hands out its window"));
        assert_eq!(pollster::block_on(engine.attach_window(id)), Ok(()));
        assert_eq!(pollster::block_on(engine.present("\"translucent\"".into())), Ok(()), "the translucent frame was drawn and shown");
        assert_eq!(middle_pixel(&reader), [128, 0, 0, 255], "straight (255, 0, 0, 128) premultiplied, over black");
        pollster::block_on(engine.detach_preview()).expect("the detach answers");
    }

    /// Attaching the window that is already attached, as an app that attaches again on every change of
    /// its `Surface` does, works: Vulkan makes one surface of a window at a time, so the attached one is
    /// given up before the new one is made from the same window.
    #[test]
    fn the_attached_window_attaches_again() {
        let engine = engine_on_a_device();
        let reader = window_reader();
        let window = reader.window().expect("the reader hands out its window");
        let first = WINDOWS.insert(window.clone());
        assert_eq!(pollster::block_on(engine.attach_window(first)), Ok(()));
        let again = WINDOWS.insert(window);
        assert_eq!(pollster::block_on(engine.attach_window(again)), Ok(()), "the window attached a second time");
        assert_eq!(pollster::block_on(engine.present("null".into())), Ok(()));
        assert_eq!(middle_pixel(&reader), [255, 0, 0, 255], "and the frame reached it");
        pollster::block_on(engine.detach_preview()).expect("the detach answers");
    }

    /// A null `Surface` is no window: the acquire answers the zero that names none, without reading the
    /// JVM's arguments at all.
    #[test]
    fn a_null_surface_is_no_window() {
        let null = std::ptr::null_mut();
        // SAFETY: a null surface is answered before the `JNIEnv` or the class is read, so null for both is
        // what the call reads nothing of.
        assert_eq!(unsafe { Java_io_github_christopherlinnett_craftbridge_core_NativeSurfaces_acquire(null, null, null) }, 0);
    }

    /// An id the table does not hold — never handed out, or already taken — is refused with the engine
    /// answering on, and a release of anything answers nothing.
    #[test]
    fn an_unknown_id_is_refused_and_release_of_it_is_quiet() {
        let engine = engine();
        assert_eq!(pollster::block_on(engine.attach_window(4242)), Err(CraftError::not_found("no window of this process has that id")));
        Java_io_github_christopherlinnett_craftbridge_core_NativeSurfaces_release(std::ptr::null_mut(), std::ptr::null_mut(), 4242);
        assert_eq!(pollster::block_on(engine.request("echo".into(), "2".into())), Ok("2".to_string()));
    }
}
