//! The app's preview surface: how it is configured, resized, drawn into and lost (plan §17).
//!
//! The platform hands the core a `CAMetalLayer` (iOS) or an Android `Surface` once, already made into
//! a wgpu surface on the core's own instance. This module chooses everything the surface is
//! configured with, from the platform and from what the surface reports it supports, so a surface
//! that cannot offer a choice answers an error instead of a configuration wgpu would panic on.
//!
//! The surface itself is reached through [`Swapchain`], which `wgpu::Surface` implements, and the
//! texture it hands over for a frame through `Taken`, which `wgpu::SurfaceTexture` implements, so
//! the rules here — what is configured on a resize, what each way a frame can fail to be acquired
//! answers, how a frame is drawn and shown — run on the desktop without a window.

use crate::Result;
use crate::blit::{Blit, Output};
use crate::error::CraftError;
use crate::gpu::{Platform, validated};

/// The formats a platform's surface is configured with, best first: the layer's native one, never
/// sRGB, because the frame's bytes are already the display's colour and an sRGB target would encode
/// them a second time. iOS takes `Bgra8Unorm` (what `CAMetalLayer` draws) and Android `Rgba8Unorm`;
/// a desktop takes either, `Bgra8Unorm` first, as its windowing systems offer it first.
const fn preferred_formats(platform: Platform) -> &'static [wgpu::TextureFormat] {
    match platform {
        Platform::Ios => &[wgpu::TextureFormat::Bgra8Unorm],
        Platform::Android => &[wgpu::TextureFormat::Rgba8Unorm],
        Platform::Host => &[wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm],
    }
}

/// The alpha modes a surface is configured with, best first, each with what the blit pass writes for
/// it (§17). `PostMultiplied` shows a transparent comp over the app's own views; `Opaque` shows the
/// frame over black. Android's surfaces offer `Inherit` alone, because its window manager, not the
/// swapchain, decides how a window is composited (the platform's Vulkan loader,
/// `frameworks/native/vulkan/libvulkan/swapchain.cpp`, reports `VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR`
/// and nothing else); there the pass writes alpha 1, which shows the same whatever the window does
/// with alpha.
const ALPHA_MODES: [(wgpu::CompositeAlphaMode, Output); 3] = [
    (wgpu::CompositeAlphaMode::PostMultiplied, Output::PostMultiplied),
    (wgpu::CompositeAlphaMode::Opaque, Output::Opaque),
    (wgpu::CompositeAlphaMode::Inherit, Output::Opaque),
];

/// How many frames the surface may hold between acquiring one and showing it: one, as the desktop
/// app's window keeps it (`effectcraft: apps/effectcraft/src/main.rs:475-482`), so a scrub shows each
/// frame one refresh sooner and the surface holds one drawable fewer. The compositor renders into
/// its own texture, not the surface's, so a smaller queue here never holds up the render, only the
/// blit's wait for a drawable.
const FRAME_LATENCY: u32 = 1;

/// What a surface is configured with, apart from its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The surface's texture format.
    pub format: wgpu::TextureFormat,
    /// How the surface is told to composite alpha.
    pub alpha_mode: wgpu::CompositeAlphaMode,
    /// What the blit pass writes, which is what that alpha mode reads.
    pub output: Output,
}

impl Choice {
    /// Choose from what a surface reports it supports. The format is the first of the platform's
    /// that the surface lists. The present mode is `Fifo`, which every surface must offer, checked
    /// anyway. The alpha mode is the first of [`ALPHA_MODES`] the surface lists. Anything the
    /// surface cannot give is [`CraftError::Gpu`] naming what was wanted and what it offered.
    pub(crate) fn from_capabilities(platform: Platform, capabilities: &wgpu::SurfaceCapabilities) -> Result<Self> {
        let format = preferred_formats(platform).iter().copied().find(|format| capabilities.formats.contains(format)).ok_or_else(|| {
            CraftError::gpu(format!("the preview surface offers none of the formats {:?}, only {:?}", preferred_formats(platform), capabilities.formats))
        })?;
        if !capabilities.present_modes.contains(&wgpu::PresentMode::Fifo) {
            return Err(CraftError::gpu(format!("the preview surface does not offer Fifo, only {:?}", capabilities.present_modes)));
        }
        let (alpha_mode, output) = ALPHA_MODES.into_iter().find(|(mode, _)| capabilities.alpha_modes.contains(mode)).ok_or_else(|| {
            CraftError::gpu(format!(
                "the preview surface offers none of the alpha modes {:?}, only {:?}",
                ALPHA_MODES.map(|(mode, _)| mode),
                capabilities.alpha_modes
            ))
        })?;
        Ok(Self { format, alpha_mode, output })
    }

    /// The configuration for a surface `width` by `height`, with the size cut to what the device
    /// takes (`max_side`), or nothing when either side has no pixels: a surface of no size, as a
    /// minimised or not-yet-laid-out view reports, cannot be configured.
    #[must_use]
    pub fn configuration(self, (width, height): (u32, u32), max_side: u32) -> Option<wgpu::SurfaceConfiguration> {
        if width == 0 || height == 0 {
            return None;
        }
        Some(wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: self.format,
            color_space: wgpu::SurfaceColorSpace::default(),
            width: width.min(max_side),
            height: height.min(max_side),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: FRAME_LATENCY,
            alpha_mode: self.alpha_mode,
            view_formats: Vec::new(),
        })
    }
}

/// What the preview surface needs of a wgpu surface.
pub trait Swapchain {
    /// What the surface supports on this adapter.
    fn capabilities(&self, adapter: &wgpu::Adapter) -> wgpu::SurfaceCapabilities;
    /// Configure the surface; every call replaces the last configuration.
    fn configure(&self, device: &wgpu::Device, configuration: &wgpu::SurfaceConfiguration);
    /// Take the texture to draw the next frame into, or say why there is none.
    fn acquire(&self) -> wgpu::CurrentSurfaceTexture;
}

impl Swapchain for wgpu::Surface<'static> {
    fn capabilities(&self, adapter: &wgpu::Adapter) -> wgpu::SurfaceCapabilities {
        self.get_capabilities(adapter)
    }

    fn configure(&self, device: &wgpu::Device, configuration: &wgpu::SurfaceConfiguration) {
        wgpu::Surface::configure(self, device, configuration);
    }

    fn acquire(&self) -> wgpu::CurrentSurfaceTexture {
        self.get_current_texture()
    }
}

/// Why no frame could be drawn this time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    /// The surface changed under its configuration: configure it again, and the next frame draws.
    Outdated,
    /// The surface is gone; only a new one from the platform brings the preview back.
    Lost,
    /// The device reported a validation error acquiring.
    Validation,
}

impl Fault {
    /// The error the call answers, in words the app can show: `Gpu`, the row §14 gives a surface failure.
    fn error(self) -> CraftError {
        CraftError::gpu(match self {
            Self::Outdated => "the preview surface changed size or format; it is configured again, so the next frame draws",
            Self::Lost => "the preview surface was lost; attach a new surface to preview again",
            Self::Validation => "the preview surface reported a validation error",
        })
    }
}

/// What acquiring the next frame came to.
enum Acquired {
    /// A texture to draw into, and whether the surface wants configuring again for the next frame.
    Texture { texture: wgpu::SurfaceTexture, reconfigure: bool },
    /// No texture this time, and nothing wrong: the view is covered or the surface is slow. The
    /// frame is skipped and the next present tries again.
    Skipped,
}

/// Sort what wgpu answered into a texture, a skip or a fault.
fn acquire(current: wgpu::CurrentSurfaceTexture) -> std::result::Result<Acquired, Fault> {
    match current {
        wgpu::CurrentSurfaceTexture::Success(texture) => Ok(Acquired::Texture { texture, reconfigure: false }),
        wgpu::CurrentSurfaceTexture::Suboptimal(texture) => Ok(Acquired::Texture { texture, reconfigure: true }),
        wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => Ok(Acquired::Skipped),
        wgpu::CurrentSurfaceTexture::Outdated => Err(Fault::Outdated),
        wgpu::CurrentSurfaceTexture::Lost => Err(Fault::Lost),
        wgpu::CurrentSurfaceTexture::Validation => Err(Fault::Validation),
    }
}

/// A texture taken from the surface for one frame: drawn into, then shown. One dropped unshown is
/// discarded, and the surface keeps showing its last frame.
trait Taken {
    /// The texture to draw into.
    fn texture(&self) -> &wgpu::Texture;
    /// Show it at the display's next refresh.
    fn show(self, queue: &wgpu::Queue);
}

impl Taken for wgpu::SurfaceTexture {
    fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    fn show(self, queue: &wgpu::Queue) {
        queue.present(self);
    }
}

/// What [`PreviewSurface::present`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presented {
    /// The frame was drawn and presented.
    Shown,
    /// Nothing was drawn: the surface has no size, is covered, or timed out. The app shows its last frame.
    Skipped,
}

/// The app's preview surface, configured and ready to blit into.
pub struct PreviewSurface<S: Swapchain = wgpu::Surface<'static>> {
    swapchain: S,
    choice: Choice,
    blit: Blit,
    size: (u32, u32),
    max_side: u32,
    /// Whether the surface is configured: not while it has no size.
    configured: bool,
    /// Set once the surface is lost, so every later present answers the same without asking wgpu
    /// to use a surface it said is gone.
    lost: bool,
}

impl<S: Swapchain> PreviewSurface<S> {
    /// Take the platform's surface, choose its configuration from what it supports on the core's
    /// `adapter`, make the blit pass for it, and configure it at `size`.
    pub fn attach(swapchain: S, adapter: &wgpu::Adapter, device: &wgpu::Device, size: (u32, u32)) -> Result<Self> {
        let choice = Choice::from_capabilities(Platform::current(), &swapchain.capabilities(adapter))?;
        let blit = Blit::new(device, choice.format, choice.output)?;
        let mut surface = Self { swapchain, choice, blit, size, max_side: device.limits().max_texture_dimension_2d, configured: false, lost: false };
        surface.reconfigure(device)?;
        Ok(surface)
    }

    /// What the surface is configured with, apart from its size.
    #[must_use]
    pub const fn choice(&self) -> Choice {
        self.choice
    }

    /// The surface changed size: configure it again at `size`. A size of no pixels leaves it
    /// unconfigured, and presents skip until it has one.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) -> Result<()> {
        self.size = size;
        self.reconfigure(device)
    }

    /// Configure the swapchain for the current size, or note it has none.
    fn reconfigure(&mut self, device: &wgpu::Device) -> Result<()> {
        let Some(configuration) = self.choice.configuration(self.size, self.max_side) else {
            self.configured = false;
            return Ok(());
        };
        validated(device, "configuring the preview surface", || self.swapchain.configure(device, &configuration))?;
        self.configured = true;
        Ok(())
    }

    /// Draw `frame` into the surface, aspect-fit, and present it.
    ///
    /// A surface that is outdated is configured again and answers [`CraftError::Gpu`], so the app's
    /// next present works; a lost one answers the same error until the app attaches a new surface.
    /// Any error the device reports while the frame is acquired, drawn or shown answers it too.
    /// None of these stops the session or panics, and the frame is simply not shown.
    pub fn present(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &wgpu::Texture) -> Result<Presented> {
        if self.lost {
            return Err(Fault::Lost.error());
        }
        if !self.configured {
            return Ok(Presented::Skipped);
        }
        // wgpu reports an acquire that fails validation to the device's error scopes, and panics
        // when none is open.
        match acquire(validated(device, "acquiring the preview frame", || self.swapchain.acquire())?) {
            Ok(Acquired::Texture { texture, reconfigure }) => self.show(device, queue, frame, texture, reconfigure),
            Ok(Acquired::Skipped) => Ok(Presented::Skipped),
            Err(fault) => {
                match fault {
                    Fault::Lost => self.lost = true,
                    Fault::Outdated => self.reconfigure(device)?,
                    Fault::Validation => {}
                }
                Err(fault.error())
            }
        }
    }

    /// Draw `frame` into `taken`, aspect-fit, and show it, then configure the surface again when it
    /// asked to be (`reconfigure`). A frame that fails to draw is not shown: it is discarded, and the
    /// surface keeps its last frame instead of showing one the pass never finished. Showing and
    /// discarding report to the device's error scopes too, so both happen inside one.
    fn show(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &wgpu::Texture, taken: impl Taken, reconfigure: bool) -> Result<Presented> {
        let blit = &self.blit;
        validated(device, "presenting the preview frame", || {
            let target = taken.texture();
            let drawn = blit.draw(device, queue, frame, &target.create_view(&wgpu::TextureViewDescriptor::default()), (target.width(), target.height()));
            if drawn.is_ok() {
                taken.show(queue);
            }
            drawn
        })??;
        if reconfigure {
            self.reconfigure(device)?;
        }
        Ok(Presented::Shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blit::tests::{assert_matches, ramp_frame, reference, target};
    use crate::testing::{Rig, read_back, rig, texture};
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    /// What a surface reports, written out: formats, present modes, alpha modes.
    fn caps(formats: &[wgpu::TextureFormat], modes: &[wgpu::PresentMode], alpha: &[wgpu::CompositeAlphaMode]) -> wgpu::SurfaceCapabilities {
        wgpu::SurfaceCapabilities { formats: formats.to_vec(), present_modes: modes.to_vec(), alpha_modes: alpha.to_vec(), ..Default::default() }
    }

    const EVERYTHING: [wgpu::CompositeAlphaMode; 4] = [
        wgpu::CompositeAlphaMode::Opaque,
        wgpu::CompositeAlphaMode::PostMultiplied,
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::Inherit,
    ];
    const MODES: [wgpu::PresentMode; 2] = [wgpu::PresentMode::Immediate, wgpu::PresentMode::Fifo];
    const BOTH: [wgpu::TextureFormat; 4] =
        [wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Bgra8UnormSrgb, wgpu::TextureFormat::Bgra8Unorm];

    /// §17's formats: `Bgra8Unorm` on iOS, `Rgba8Unorm` on Android, never sRGB, each only when the
    /// surface lists it, whatever order it lists them in.
    #[test]
    fn the_format_is_the_platforms_and_never_srgb() {
        let ios = Choice::from_capabilities(Platform::Ios, &caps(&BOTH, &MODES, &EVERYTHING)).expect("iOS lists Bgra8Unorm");
        assert_eq!(ios.format, wgpu::TextureFormat::Bgra8Unorm);
        let android = Choice::from_capabilities(Platform::Android, &caps(&BOTH, &MODES, &EVERYTHING)).expect("Android lists Rgba8Unorm");
        assert_eq!(android.format, wgpu::TextureFormat::Rgba8Unorm);
        // A desktop takes Bgra8Unorm first and Rgba8Unorm if that is all it has.
        let host = Choice::from_capabilities(Platform::Host, &caps(&BOTH, &MODES, &EVERYTHING)).expect("a host lists both");
        assert_eq!(host.format, wgpu::TextureFormat::Bgra8Unorm);
        let rgba_only = caps(&[wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Rgba8Unorm], &MODES, &EVERYTHING);
        assert_eq!(Choice::from_capabilities(Platform::Host, &rgba_only).expect("a host takes Rgba8Unorm").format, wgpu::TextureFormat::Rgba8Unorm);
    }

    /// A surface that offers the platform's format only as sRGB, or not at all, answers `Gpu`
    /// naming both sides, never a configuration it did not offer.
    #[test]
    fn a_format_the_surface_does_not_offer_is_a_gpu_error() {
        let srgb_only = caps(&[wgpu::TextureFormat::Bgra8UnormSrgb], &MODES, &EVERYTHING);
        let error = Choice::from_capabilities(Platform::Ios, &srgb_only).expect_err("sRGB is not the format");
        assert_eq!(error, CraftError::gpu("the preview surface offers none of the formats [Bgra8Unorm], only [Bgra8UnormSrgb]"));
        // The other platform's format is no substitute.
        let error =
            Choice::from_capabilities(Platform::Android, &caps(&[wgpu::TextureFormat::Bgra8Unorm], &MODES, &EVERYTHING)).expect_err("Android wants Rgba8Unorm");
        assert_eq!(error, CraftError::gpu("the preview surface offers none of the formats [Rgba8Unorm], only [Bgra8Unorm]"));
    }

    /// Fifo is what is configured, whatever else the surface offers, and its absence is an error.
    #[test]
    fn the_present_mode_is_fifo_and_checked() {
        let choice = Choice::from_capabilities(Platform::Ios, &caps(&BOTH, &[wgpu::PresentMode::Mailbox, wgpu::PresentMode::Fifo], &EVERYTHING))
            .expect("Fifo is offered");
        let configuration = choice.configuration((100, 50), 16_384).expect("a surface with a size");
        assert_eq!(configuration.present_mode, wgpu::PresentMode::Fifo);
        let error = Choice::from_capabilities(Platform::Ios, &caps(&BOTH, &[wgpu::PresentMode::Mailbox], &EVERYTHING)).expect_err("no Fifo");
        assert_eq!(error, CraftError::gpu("the preview surface does not offer Fifo, only [Mailbox]"));
    }

    /// `PostMultiplied` where offered, else `Opaque`, else `Inherit`, the one mode Android's surfaces
    /// offer, and what is configured is what the pass writes for it: straight colour for
    /// `PostMultiplied`, an opaque picture for the other two. `PreMultiplied` alone is refused.
    #[test]
    fn the_alpha_mode_is_post_multiplied_else_opaque_else_inherited() {
        let choose = |alpha: &[wgpu::CompositeAlphaMode]| Choice::from_capabilities(Platform::Android, &caps(&BOTH, &MODES, alpha));
        let configured = |choice: Choice| choice.configuration((8, 8), 64).map(|c| c.alpha_mode);
        let post = choose(&EVERYTHING).expect("post-multiplied is offered");
        assert_eq!((post.alpha_mode, post.output), (wgpu::CompositeAlphaMode::PostMultiplied, Output::PostMultiplied));
        assert_eq!(configured(post), Some(wgpu::CompositeAlphaMode::PostMultiplied));
        let opaque =
            choose(&[wgpu::CompositeAlphaMode::Inherit, wgpu::CompositeAlphaMode::PreMultiplied, wgpu::CompositeAlphaMode::Opaque]).expect("opaque is offered");
        assert_eq!((opaque.alpha_mode, opaque.output), (wgpu::CompositeAlphaMode::Opaque, Output::Opaque));
        assert_eq!(configured(opaque), Some(wgpu::CompositeAlphaMode::Opaque));
        // What Android's Vulkan loader reports for every surface.
        let android = choose(&[wgpu::CompositeAlphaMode::Inherit]).expect("an Android surface attaches");
        assert_eq!((android.alpha_mode, android.output), (wgpu::CompositeAlphaMode::Inherit, Output::Opaque));
        assert_eq!(configured(android), Some(wgpu::CompositeAlphaMode::Inherit));
        let error = choose(&[wgpu::CompositeAlphaMode::PreMultiplied]).expect_err("none of the three");
        assert_eq!(error, CraftError::gpu("the preview surface offers none of the alpha modes [PostMultiplied, Opaque, Inherit], only [PreMultiplied]"));
    }

    /// The whole configuration at a size: the size is the surface's, cut to the device's limit, a
    /// size with no pixels configures nothing, and nothing else about it moves with the size.
    #[test]
    fn the_configuration_follows_the_size_and_nothing_else() {
        let choice = Choice { format: wgpu::TextureFormat::Bgra8Unorm, alpha_mode: wgpu::CompositeAlphaMode::PostMultiplied, output: Output::PostMultiplied };
        let first = choice.configuration((640, 480), 16_384).expect("a size");
        assert_eq!((first.width, first.height), (640, 480));
        let second = choice.configuration((1280, 720), 16_384).expect("a size");
        assert_eq!((second.width, second.height), (1280, 720));
        let same = wgpu::SurfaceConfiguration { width: 640, height: 480, ..second.clone() };
        assert_eq!(same, first, "only the size differs between sizes");
        assert_eq!(first.usage, wgpu::TextureUsages::RENDER_ATTACHMENT);
        assert_eq!(first.format, wgpu::TextureFormat::Bgra8Unorm);
        // One frame in flight, as the desktop app's window keeps it.
        assert_eq!(first.desired_maximum_frame_latency, 1);
        let cut = choice.configuration((40_000, 100), 16_384).expect("a size");
        assert_eq!((cut.width, cut.height), (16_384, 100));
        assert_eq!(choice.configuration((0, 480), 16_384), None);
        assert_eq!(choice.configuration((640, 0), 16_384), None);
    }

    /// Raise a validation error on `device`, as wgpu does on a refused acquire or present: a
    /// texture with no width.
    fn refuse(device: &wgpu::Device) {
        let _ = texture(device, (0, 4), 1, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::TEXTURE_BINDING);
    }

    /// A surface that is a script: it reports what it is told, records every configuration, and
    /// answers acquisitions from a queue, raising a device error while it acquires when told to.
    struct Scripted {
        capabilities: wgpu::SurfaceCapabilities,
        configured: RefCell<Vec<(u32, u32)>>,
        acquisitions: RefCell<VecDeque<wgpu::CurrentSurfaceTexture>>,
        acquired: RefCell<u32>,
        refuse: Option<wgpu::Device>,
    }

    impl Scripted {
        fn new(acquisitions: impl IntoIterator<Item = wgpu::CurrentSurfaceTexture>) -> Self {
            Self {
                capabilities: caps(&BOTH, &MODES, &EVERYTHING),
                configured: RefCell::new(Vec::new()),
                acquisitions: RefCell::new(acquisitions.into_iter().collect()),
                acquired: RefCell::new(0),
                refuse: None,
            }
        }
    }

    impl Swapchain for &Scripted {
        fn capabilities(&self, _: &wgpu::Adapter) -> wgpu::SurfaceCapabilities {
            caps(&self.capabilities.formats, &self.capabilities.present_modes, &self.capabilities.alpha_modes)
        }

        fn configure(&self, _: &wgpu::Device, configuration: &wgpu::SurfaceConfiguration) {
            self.configured.borrow_mut().push((configuration.width, configuration.height));
        }

        fn acquire(&self) -> wgpu::CurrentSurfaceTexture {
            *self.acquired.borrow_mut() += 1;
            if let Some(device) = &self.refuse {
                refuse(device);
            }
            self.acquisitions.borrow_mut().pop_front().unwrap_or(wgpu::CurrentSurfaceTexture::Lost)
        }
    }

    /// A surface texture made offscreen, as a real surface's would be handed over: drawn into and
    /// read back, counting how often it was shown, and raising a device error as it is shown when told to.
    struct Offscreen<'a> {
        texture: wgpu::Texture,
        shown: &'a Cell<u32>,
        refuse: Option<&'a wgpu::Device>,
    }

    impl Taken for Offscreen<'_> {
        fn texture(&self) -> &wgpu::Texture {
            &self.texture
        }

        fn show(self, _: &wgpu::Queue) {
            self.shown.set(self.shown.get() + 1);
            if let Some(device) = self.refuse {
                refuse(device);
            }
        }
    }

    /// A display frame for the tests that only need one to be there.
    fn frame(rig: &Rig) -> wgpu::Texture {
        texture(&rig.device, (4, 4), 1, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::TEXTURE_BINDING)
    }

    /// Attaching configures at the size it was given, and every resize configures again at the new one.
    #[test]
    fn attach_and_every_resize_configure_the_surface() {
        let rig = rig();
        let scripted = Scripted::new([]);
        let mut surface = PreviewSurface::attach(&scripted, &rig.adapter, &rig.device, (320, 240)).expect("the scripted surface attaches");
        assert_eq!(*scripted.configured.borrow(), [(320, 240)]);
        surface.resize(&rig.device, (640, 480)).expect("a resize");
        surface.resize(&rig.device, (240, 320)).expect("a resize");
        assert_eq!(*scripted.configured.borrow(), [(320, 240), (640, 480), (240, 320)]);
        // The platform's choice, from what the script offers: post-multiplied BGRA, as the host takes it.
        let choice = Choice { format: wgpu::TextureFormat::Bgra8Unorm, alpha_mode: wgpu::CompositeAlphaMode::PostMultiplied, output: Output::PostMultiplied };
        assert_eq!(surface.choice(), choice);
    }

    /// A surface with no size is not configured, presents skip without asking the surface for a
    /// frame, and its first size brings it back.
    #[test]
    fn a_surface_with_no_size_skips_until_it_has_one() {
        let rig = rig();
        let scripted = Scripted::new([wgpu::CurrentSurfaceTexture::Occluded]);
        let mut surface = PreviewSurface::attach(&scripted, &rig.adapter, &rig.device, (0, 0)).expect("a surface of no size attaches");
        assert!(scripted.configured.borrow().is_empty());
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Ok(Presented::Skipped));
        assert_eq!(*scripted.acquired.borrow(), 0);
        surface.resize(&rig.device, (16, 16)).expect("a first size");
        assert_eq!(*scripted.configured.borrow(), [(16, 16)]);
        // Now it asks, and an occluded surface is a skip, not an error.
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Ok(Presented::Skipped));
        assert_eq!(*scripted.acquired.borrow(), 1);
        surface.resize(&rig.device, (0, 16)).expect("a resize to nothing");
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Ok(Presented::Skipped));
        assert_eq!(*scripted.acquired.borrow(), 1);
    }

    /// A timeout skips the frame and the session goes on.
    #[test]
    fn a_timeout_skips_the_frame() {
        let rig = rig();
        let scripted = Scripted::new([wgpu::CurrentSurfaceTexture::Timeout]);
        let mut surface = PreviewSurface::attach(&scripted, &rig.adapter, &rig.device, (16, 16)).expect("attached");
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Ok(Presented::Skipped));
    }

    /// An outdated surface answers the `Gpu` error and is configured again at once, so the next
    /// present is on a fresh configuration; the surface is never marked lost by it.
    #[test]
    fn an_outdated_surface_is_configured_again_and_answers_gpu() {
        let rig = rig();
        let scripted = Scripted::new([wgpu::CurrentSurfaceTexture::Outdated, wgpu::CurrentSurfaceTexture::Outdated]);
        let mut surface = PreviewSurface::attach(&scripted, &rig.adapter, &rig.device, (16, 16)).expect("attached");
        let error = surface.present(&rig.device, &rig.queue, &frame(&rig)).expect_err("outdated is an error");
        assert_eq!(error, CraftError::gpu("the preview surface changed size or format; it is configured again, so the next frame draws"));
        assert_eq!(*scripted.configured.borrow(), [(16, 16), (16, 16)], "configured again after the fault");
        // It asks again on the next present: outdated is not lost.
        assert!(surface.present(&rig.device, &rig.queue, &frame(&rig)).is_err());
        assert_eq!(*scripted.acquired.borrow(), 2);
        assert_eq!(scripted.configured.borrow().len(), 3);
    }

    /// A lost surface answers the `Gpu` error, never a panic, and keeps answering it without asking
    /// the surface for another frame or configuring it, until the app attaches a new one.
    #[test]
    fn a_lost_surface_answers_gpu_and_stays_lost() {
        let rig = rig();
        let scripted = Scripted::new([wgpu::CurrentSurfaceTexture::Lost]);
        let mut surface = PreviewSurface::attach(&scripted, &rig.adapter, &rig.device, (16, 16)).expect("attached");
        let lost = CraftError::gpu("the preview surface was lost; attach a new surface to preview again");
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Err(lost.clone()));
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Err(lost));
        assert_eq!(*scripted.acquired.borrow(), 1, "a lost surface is not asked again");
        assert_eq!(*scripted.configured.borrow(), [(16, 16)], "a lost surface is not configured again");
        // The session is unharmed: a new surface attaches on the same device and presents.
        let fresh = Scripted::new([wgpu::CurrentSurfaceTexture::Timeout]);
        let mut surface = PreviewSurface::attach(&fresh, &rig.adapter, &rig.device, (16, 16)).expect("a new surface attaches");
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Ok(Presented::Skipped));
    }

    /// A validation error acquiring is the `Gpu` error with its own words; the surface is not lost by it.
    #[test]
    fn a_validation_error_answers_gpu_without_losing_the_surface() {
        let rig = rig();
        let scripted = Scripted::new([wgpu::CurrentSurfaceTexture::Validation, wgpu::CurrentSurfaceTexture::Timeout]);
        let mut surface = PreviewSurface::attach(&scripted, &rig.adapter, &rig.device, (16, 16)).expect("attached");
        let error = surface.present(&rig.device, &rig.queue, &frame(&rig)).expect_err("a validation error");
        assert_eq!(error, CraftError::gpu("the preview surface reported a validation error"));
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Ok(Presented::Skipped));
        assert_eq!(*scripted.configured.borrow(), [(16, 16)]);
    }

    /// The device error wgpu raises when it refuses an acquire is the `Gpu` error with the device's
    /// words: with no error scope open, wgpu's default handler would panic instead.
    #[test]
    fn a_device_error_while_acquiring_is_a_gpu_error_not_a_panic() {
        let rig = rig();
        let mut scripted = Scripted::new([wgpu::CurrentSurfaceTexture::Validation, wgpu::CurrentSurfaceTexture::Timeout]);
        scripted.refuse = Some(rig.device.clone());
        let mut surface = PreviewSurface::attach(&scripted, &rig.adapter, &rig.device, (16, 16)).expect("attached");
        let Err(CraftError::Gpu { message }) = surface.present(&rig.device, &rig.queue, &frame(&rig)) else {
            panic!("a refused acquire must answer a Gpu error");
        };
        assert!(message.starts_with("acquiring the preview frame: "), "{message}");
        // The surface is not lost by it: the next present asks again.
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Err(CraftError::Gpu { message }));
        assert_eq!(*scripted.acquired.borrow(), 2);
    }

    /// A texture the surface hands over is drawn into aspect-fit at its own size, matching the
    /// reference, and shown once; a frame the surface marks suboptimal is shown, then the surface is
    /// configured again.
    #[test]
    fn a_frame_is_drawn_aspect_fit_into_the_texture_the_surface_hands_over_and_shown() {
        let rig = rig();
        let scripted = Scripted::new([]);
        let mut surface = PreviewSurface::attach(&scripted, &rig.adapter, &rig.device, (48, 48)).expect("attached");
        let source = ramp_frame(&rig, 32, 16);
        for (size, suboptimal) in [((48, 48), false), ((24, 48), true)] {
            let drawn = target(&rig, surface.choice().format, size);
            let shown = Cell::new(0);
            let taken = Offscreen { texture: drawn.clone(), shown: &shown, refuse: None };
            assert_eq!(surface.show(&rig.device, &rig.queue, &source, taken, suboptimal), Ok(Presented::Shown));
            assert_eq!(shown.get(), 1, "{size:?}: shown once");
            assert_matches(&read_back(&rig, &drawn), &reference((32, 16), size), size.0, &format!("{size:?}"));
        }
        assert_eq!(*scripted.configured.borrow(), [(48, 48), (48, 48)], "configured again after the suboptimal frame only");
    }

    /// A frame the pass cannot draw is the `Gpu` error and is not shown, so the surface keeps its last frame.
    #[test]
    fn a_frame_that_fails_to_draw_is_not_shown() {
        let rig = rig();
        let scripted = Scripted::new([]);
        let mut surface = PreviewSurface::attach(&scripted, &rig.adapter, &rig.device, (16, 16)).expect("attached");
        let unbindable = texture(&rig.device, (4, 4), 1, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::COPY_DST);
        let shown = Cell::new(0);
        let taken = Offscreen { texture: target(&rig, surface.choice().format, (16, 16)), shown: &shown, refuse: None };
        let Err(CraftError::Gpu { message }) = surface.show(&rig.device, &rig.queue, &unbindable, taken, true) else {
            panic!("a frame that cannot be bound must answer a Gpu error");
        };
        assert!(message.starts_with("the blit draw: "), "{message}");
        assert_eq!(shown.get(), 0, "an undrawn frame is not shown");
        assert_eq!(*scripted.configured.borrow(), [(16, 16)], "nothing was shown, so nothing is configured again");
    }

    /// The device error wgpu raises when it refuses to present is the `Gpu` error with the device's
    /// words, not wgpu's default panic.
    #[test]
    fn a_device_error_while_showing_is_a_gpu_error_not_a_panic() {
        let rig = rig();
        let scripted = Scripted::new([]);
        let mut surface = PreviewSurface::attach(&scripted, &rig.adapter, &rig.device, (16, 16)).expect("attached");
        let shown = Cell::new(0);
        let taken = Offscreen { texture: target(&rig, surface.choice().format, (16, 16)), shown: &shown, refuse: Some(&rig.device) };
        let Err(CraftError::Gpu { message }) = surface.show(&rig.device, &rig.queue, &ramp_frame(&rig, 4, 4), taken, false) else {
            panic!("a refused present must answer a Gpu error");
        };
        assert!(message.starts_with("presenting the preview frame: "), "{message}");
    }

    /// A surface that offers nothing the core can use fails to attach with the `Gpu` error and no
    /// panic, so the app learns at attach and not at its first frame.
    #[test]
    fn attaching_to_a_surface_with_no_usable_format_is_a_gpu_error() {
        let rig = rig();
        let mut scripted = Scripted::new([]);
        scripted.capabilities = caps(&[wgpu::TextureFormat::Rgba16Float], &MODES, &EVERYTHING);
        let Err(CraftError::Gpu { message }) = PreviewSurface::attach(&scripted, &rig.adapter, &rig.device, (16, 16)) else {
            panic!("a surface with no usable format must not attach");
        };
        assert!(message.starts_with("the preview surface offers none of the formats"), "{message}");
    }

    /// Each platform's pick is the plan's, by the target names the gate builds.
    #[test]
    fn each_platform_takes_its_own_formats() {
        assert_eq!(preferred_formats(Platform::of("ios")), [wgpu::TextureFormat::Bgra8Unorm]);
        assert_eq!(preferred_formats(Platform::of("android")), [wgpu::TextureFormat::Rgba8Unorm]);
        assert_eq!(preferred_formats(Platform::of("macos")), [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm]);
    }
}
