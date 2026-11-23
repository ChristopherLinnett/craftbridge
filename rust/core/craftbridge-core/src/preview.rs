//! The preview a `present` draws into: the surface the engine hands the app (plan §16, §17).
//!
//! The platform module builds the surface from the app's layer or window — it is the one that
//! knows the handle's live size — and hands the engine a call that makes it on the engine's own
//! thread, on the device the engine's GPU lifecycle holds. The lifecycle keeps the surface with
//! that device: every `present` call runs the app's [`crate::app::CraftApp::present`] with it, and it is
//! dropped, with the swapchain and the platform handle it holds, on the engine thread behind the
//! same guard as a call, by `detach_preview` and wherever the device goes.

use crate::Result;
use crate::error::CraftError;
use crate::frames::{FrameBytes, PixelFormat, check_raw_length, premultiply_rgba};
use crate::gpu::{GpuDevice, validated};
use crate::surface::{Choice, Presented, PreviewAlpha, PreviewSurface, Swapchain};

/// The answer to a preview with no device to make its surface on: the engine is on the CPU, because
/// the app chose so, no adapter answered, or the GPU was given up for the background.
pub(crate) const NO_DEVICE: &str = "there is no device to preview on";

/// A platform's call to make its preview surface on the engine's device. It is built on the
/// caller's thread from the app's handle, runs on the engine thread where the device is, and
/// answers the surface to preview into, or the reason there is none.
pub type SurfaceMaker = Box<dyn FnOnce(&GpuDevice) -> Result<Surface> + Send>;

/// The preview surface an app's [`crate::app::CraftApp::present`] draws into (§16).
///
/// A frame already on the GPU goes in through [`Surface::present`]; a CPU-rendered one through
/// [`Surface::present_frame`], which uploads it to a reusable texture before the same blit
/// (§17). Either way what the surface does with the frame — acquire, aspect-fit blit, show, and
/// every way a frame fails to be acquired or shown answers [`CraftError::Gpu`] with the session
/// kept — is [`PreviewSurface`]'s, and a surface resized or lost between presents is answered
/// there.
pub struct Surface {
    surface: PreviewSurface<Box<dyn Swapchain + Send>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// The upload of the CPU frame presented last, kept for the next one of the same size (§17).
    upload: Option<wgpu::Texture>,
}

impl Surface {
    /// Draw a frame that is already on the device, aspect-fit, and show it.
    pub fn present(&mut self, frame: &wgpu::Texture) -> Result<Presented> {
        self.surface.present(&self.device, &self.queue, frame)
    }

    /// Upload a CPU-rendered frame and present it as [`Surface::present`] does (§17's CPU step).
    ///
    /// The frame is straight RGBA — the format a CPU render gives — and it is premultiplied as it
    /// is uploaded, since the blit pass reads every frame as the premultiplied display frame a GPU
    /// render leaves. The upload reuses one texture until the frame changes size. Any other format
    /// answers [`CraftError::Unsupported`]: the conversion belongs to the caller's own colour work,
    /// and the core uploads the one shape a CPU render hands over whole.
    pub fn present_frame(&mut self, frame: &FrameBytes) -> Result<Presented> {
        if frame.format != PixelFormat::Rgba8Straight {
            return Err(CraftError::unsupported("the preview uploads straight RGBA frames"));
        }
        let upload = self.upload(frame.width, frame.height, &frame.data)?;
        self.present(&upload)
    }

    /// The device the surface shows its frames with: the one the frame textures are made on.
    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The queue that carries the device's work.
    #[must_use]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// What the surface is configured with, apart from its size: the app renders its frame for
    /// the format and alpha this names.
    #[must_use]
    pub const fn choice(&self) -> Choice {
        self.surface.choice()
    }

    /// The size the surface is configured at, as its layer or window last reported it.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.surface.size()
    }

    /// Make the preview surface on the `gpu`'s device from a platform's swapchain: the one device
    /// the surface and every frame it shows are made on (§20). A CPU-only engine has no device,
    /// and answers why ([`NO_DEVICE`]); a surface the engine cannot configure answers at this call, not at the
    /// first frame. This is an attach that did not ask to carry the composition's alpha.
    pub fn new(swapchain: Box<dyn Swapchain + Send>, gpu: &GpuDevice) -> Result<Self> {
        Self::new_with_alpha(swapchain, gpu, PreviewAlpha::default())
    }

    /// Make the preview surface as [`Surface::new`] does, for an attach that asked for `alpha`: the
    /// surface keeps the ask in the pass it draws frames with, which writes and clears as the ask says
    /// (§17). One that asked nothing is what [`Surface::new`] makes.
    pub fn new_with_alpha(swapchain: Box<dyn Swapchain + Send>, gpu: &GpuDevice, alpha: PreviewAlpha) -> Result<Self> {
        let (adapter, device, queue) = gpu.parts().ok_or_else(|| CraftError::gpu(NO_DEVICE))?;
        Ok(Self::from_parts(PreviewSurface::attach_with_alpha(swapchain, adapter, device, alpha)?, device.clone(), queue.clone()))
    }

    /// Take a surface the platform made, with the device its frames show with.
    pub(crate) fn from_parts(surface: PreviewSurface<Box<dyn Swapchain + Send>>, device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self { surface, device, queue, upload: None }
    }

    /// Upload straight-RGBA `data`, premultiplied, to the reusable texture sized for a `width` by
    /// `height` frame, and answer that texture. The texture is replaced only when the frame changed
    /// size. Rows go in as they come, tightly packed: `Queue::write_texture`, unlike a copy from a
    /// buffer, asks no alignment of a row's bytes.
    fn upload(&mut self, width: u32, height: u32, data: &[u8]) -> Result<wgpu::Texture> {
        // A frame's fields are public, so its length is checked against its size here rather than
        // trusted to have come through `FrameBytes::new`.
        check_raw_length(width, height, data)?;
        let reusable = self.upload.as_ref().filter(|upload| upload.width() == width && upload.height() == height).cloned();
        let upload = match reusable {
            Some(upload) => upload,
            None => {
                // Made inside the device's error scopes, as all the core's work on a device is: a frame
                // past the device's texture limit, or one the device has no memory for, answers `Gpu`
                // here, where wgpu would otherwise panic.
                let device = &self.device;
                let upload = validated(device, "making the preview upload", || {
                    device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("craftbridge preview upload"),
                        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    })
                })?;
                self.upload = Some(upload.clone());
                upload
            }
        };
        // The device made a texture this wide, so a row's bytes fit a u32.
        let bytes_per_row = width.checked_mul(4).ok_or_else(|| CraftError::internal("a preview frame's row cannot be measured"))?;
        let mut bytes = data.to_vec();
        premultiply_rgba(&mut bytes)?;
        let queue = self.queue.clone();
        validated(&self.device, "uploading the preview frame", || {
            queue.write_texture(
                upload.as_image_copy(),
                &bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes_per_row), rows_per_image: None },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );
        })?;
        Ok(upload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blit::Output;
    use crate::blit::tests::{frame, render};
    use crate::gpu::GpuChoice;
    use crate::surface::tests::bar_and_picture;
    use crate::testing::{Rig, Scripted, rig};

    /// A CPU frame the width and height say, every pixel the colour `rgba`.
    fn cpu_frame(width: u32, height: u32, rgba: [u8; 4]) -> FrameBytes {
        let pixels = usize::try_from(width).unwrap() * usize::try_from(height).unwrap();
        FrameBytes::new(width, height, PixelFormat::Rgba8Straight, rgba.repeat(pixels)).unwrap()
    }

    /// A surface on the engine's device, made the way a platform maker makes one.
    fn surface(device: &wgpu::Device, queue: &wgpu::Queue, scripted: Scripted) -> Surface {
        let swapchain: Box<dyn Swapchain + Send> = Box::new(scripted);
        Surface::from_parts(PreviewSurface::attach(swapchain, &rig().adapter, device).unwrap(), device.clone(), queue.clone())
    }

    /// A straight-RGBA frame uploads and presents whatever it is, whatever its width, an odd
    /// one included, and one size reuses one texture until the frame changes size, in either side.
    #[test]
    fn a_cpu_frame_uploads_at_any_size_and_reuses_its_texture() {
        let rig = rig();
        let scripted = Scripted::new((16, 16), (0..5).map(|_| wgpu::CurrentSurfaceTexture::Timeout));
        let mut preview = surface(&rig.device, &rig.queue, scripted);
        let upload = |preview: &Surface| preview.upload.clone().unwrap();
        // A frame of an odd width: a row of 12 bytes, no validation error.
        assert_eq!(preview.present_frame(&cpu_frame(3, 3, [255, 0, 0, 255])), Ok(Presented::Skipped));
        let first = upload(&preview);
        // The same size: the same texture, written again.
        assert_eq!(preview.present_frame(&cpu_frame(3, 3, [0, 255, 0, 255])), Ok(Presented::Skipped));
        assert_eq!(upload(&preview), first, "one size reuses one texture");
        // Another height alone replaces it, and so does another width alone.
        assert_eq!(preview.present_frame(&cpu_frame(3, 5, [0, 0, 255, 255])), Ok(Presented::Skipped));
        let taller = upload(&preview);
        assert_ne!(taller, first, "a new height is a new texture");
        assert_eq!((taller.width(), taller.height()), (3, 5));
        assert_eq!(preview.present_frame(&cpu_frame(4, 5, [0, 0, 255, 255])), Ok(Presented::Skipped));
        let wider = upload(&preview);
        assert_ne!(wider, taller, "a new width is a new texture");
        assert_eq!((wider.width(), wider.height()), (4, 5));
    }

    /// A CPU frame reaches the blit pass as the pass reads every frame: premultiplied, and row by row
    /// as the frame lays its pixels out. The frame is three wide, a row of 12 bytes, no multiple of
    /// any copy alignment, and each pixel is its own, so a row shifted, padded or premultiplied
    /// wrong shows. Drawn opaque at its own size, each pixel is its premultiplied colour exactly.
    #[test]
    fn a_cpu_frame_is_uploaded_premultiplied_row_by_row() {
        let rig = rig();
        let mut preview = surface(&rig.device, &rig.queue, Scripted::new((16, 16), [wgpu::CurrentSurfaceTexture::Timeout]));
        let straight: [[u8; 4]; 6] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [200, 100, 50, 128], [10, 20, 30, 0], [255, 255, 255, 255]];
        let frame = FrameBytes::new(3, 2, PixelFormat::Rgba8Straight, straight.concat()).unwrap();
        assert_eq!(preview.present_frame(&frame), Ok(Presented::Skipped));
        let shown = render(&rig, wgpu::TextureFormat::Rgba8Unorm, Output::Opaque, preview.upload.as_ref().unwrap(), (3, 2));
        // (200, 100, 50) at alpha 128 is (100, 50, 25) premultiplied, and any colour at alpha 0 is black.
        let premultiplied = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [100, 50, 25, 255], [0, 0, 0, 255], [255, 255, 255, 255]];
        assert_eq!(shown, premultiplied);
    }

    /// A frame whose bytes are not its size times four is refused before a byte is written: a
    /// frame's fields are public, so its length is not taken on trust.
    #[test]
    fn a_frame_whose_bytes_do_not_match_its_size_is_refused() {
        let rig = rig();
        let mut preview = surface(&rig.device, &rig.queue, Scripted::new((16, 16), []));
        let short = FrameBytes { width: 2, height: 2, format: PixelFormat::Rgba8Straight, data: vec![0; 12] };
        assert!(matches!(preview.present_frame(&short), Err(CraftError::InvalidParams { .. })));
        assert!(preview.upload.is_none(), "a refused frame writes nothing");
    }

    /// A frame wider than the device's texture limit is refused as `Gpu`, naming the upload the device
    /// refused, and writes nothing: wgpu would raise its validation error as a panic were the
    /// texture made outside the device's error scopes.
    #[test]
    fn a_cpu_frame_past_the_texture_limit_is_refused() {
        let rig = rig();
        let mut preview = surface(&rig.device, &rig.queue, Scripted::new((16, 16), []));
        let width = rig.device.limits().max_texture_dimension_2d + 1;
        let Err(CraftError::Gpu { message }) = preview.present_frame(&cpu_frame(width, 1, [1, 2, 3, 255])) else {
            panic!("a frame past the texture limit must answer Gpu");
        };
        assert!(message.starts_with("making the preview upload: "), "{message}");
        assert!(message.contains(&width.to_string()), "the refusal names the width: {message}");
        assert!(preview.upload.is_none(), "a refused frame writes nothing");
    }

    /// A frame in a format the preview does not upload is refused before a byte is written.
    #[test]
    fn only_straight_rgba_uploads() {
        let rig = rig();
        let scripted = Scripted::new((16, 16), []);
        let mut preview = surface(&rig.device, &rig.queue, scripted);
        let premultiplied = FrameBytes::new(2, 1, PixelFormat::Bgra8Premultiplied, vec![0u8; 8]).unwrap();
        let Err(CraftError::Unsupported { message }) = preview.present_frame(&premultiplied) else {
            panic!("a premultiplied frame must not upload");
        };
        assert_eq!(message, "the preview uploads straight RGBA frames");
        assert!(preview.upload.is_none(), "a refused frame writes nothing");
    }

    /// What a platform's maker asks of the composition's alpha reaches the surface it makes: made with
    /// an ask, the surface draws as the attach with that ask draws, and made with none, as the default
    /// does. The bars follow the ask on every platform, so the two asks draw apart wherever this runs
    /// and a maker that dropped its ask would draw the other's.
    #[test]
    fn a_surface_is_made_with_what_its_maker_asked() {
        // The maker is handed the whole device, so the frame is drawn and read back on that same one.
        let gpu = GpuDevice::request(GpuChoice::Preferred);
        let (adapter, device, queue) = gpu.parts().expect("the machine the tests run on answers a device");
        let rig = Rig { adapter: adapter.clone(), device: device.clone(), queue: queue.clone() };
        let source = frame(&rig, 8, 4, |_, _| [64, 32, 0, 128]);
        let swapchain = || -> Box<dyn Swapchain + Send> { Box::new(Scripted::new((8, 8), [])) };
        let attached = |alpha: PreviewAlpha| {
            let surface = PreviewSurface::attach_with_alpha(swapchain(), adapter, device, alpha).expect("the scripted surface attaches");
            bar_and_picture(&rig, surface, &source)
        };
        assert_ne!(attached(PreviewAlpha::Premultiplied), attached(PreviewAlpha::Opaque), "the bars tell the two asks apart");
        for alpha in [PreviewAlpha::Premultiplied, PreviewAlpha::Opaque] {
            let made = Surface::new_with_alpha(swapchain(), &gpu, alpha).expect("the scripted surface is made");
            assert_eq!(bar_and_picture(&rig, made.surface, &source), attached(alpha), "{alpha:?}: the maker's ask is the attach's");
        }
        let quiet = Surface::new(swapchain(), &gpu).expect("the scripted surface is made");
        assert_eq!(bar_and_picture(&rig, quiet.surface, &source), attached(PreviewAlpha::default()), "a maker that asks nothing makes the default");
    }
}
