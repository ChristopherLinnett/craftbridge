//! The app's preview surface: how it is configured, resized, drawn into and lost (plan §17).
//!
//! The platform hands the core a `CAMetalLayer` (iOS) or an Android `Surface` once, already made
//! into a wgpu surface on the core's own instance. This module chooses everything the surface is
//! configured with, from the platform and from what the surface reports it supports, so a surface
//! that cannot offer a choice answers an error instead of a configuration wgpu would panic on.
//!
//! The surface itself is reached through [`Swapchain`], which the platform modules implement over
//! the surface they made from the app's handle (it is the handle that knows its own size, §17), and
//! the texture it hands over for a frame through `Taken`, which `wgpu::SurfaceTexture` implements,
//! so the rules here — what is configured on a resize, what each way a frame can fail to be
//! acquired answers, how a frame is drawn and shown — run on the desktop without a window.

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

/// What an attach asks the preview to write of the composition's alpha. The ask is the app's, made
/// when the surface is attached, because the window's compositor — not the SDK — decides how a
/// surface's alpha is read under `CompositeAlphaMode::Inherit`, so the SDK cannot know it and the app
/// that owns the consumer can: an app that shows the surface in a compositor that reads premultiplied
/// alpha (a Flutter `Texture`, say) asks for [`PreviewAlpha::Premultiplied`] and the surface carries
/// the composition's transparency; every attach that does not ask is written as it always was.
///
/// The ask picks the alpha modes the surface may be configured with, and is never answered with
/// another ask's: a surface that cannot carry what was asked refuses the attach, so an app that asked
/// for the composition's alpha is never handed the frame over black, or straight colour its
/// compositor would read as premultiplied, without being told. Where the surface says itself how its
/// alpha reads — iOS, the host — both asks choose the same mode and the same output, since the window
/// settles nothing there, and refusing one would fail the same app call on one platform and not
/// another for no reason the app can act on. The bars follow the ask on every platform
/// ([`crate::blit::clear_color`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewAlpha {
    /// The default, and what every preview wrote before an attach could ask: the modes every platform
    /// has always chosen from, between opaque black bars. On Android, whose surfaces offer `Inherit`
    /// alone, that is the frame over black with alpha 1.
    #[default]
    Opaque,
    /// The frame's own colour and alpha, premultiplied as the compositor leaves it, between fully
    /// transparent bars, so a compositor that reads premultiplied alpha shows the composition's
    /// transparency and lets what is behind the surface show through the bars. On Android it is
    /// written only through a mode that carries it, and refused where the surface offers none.
    Premultiplied,
}

/// The rows of an attach that did not ask to carry the composition's alpha, the ones every platform
/// has always chosen from: `PostMultiplied` shows a transparent comp over the app's own views, written
/// `translucent`, which is what the platform's translucent surface reads; under `Opaque` and under
/// `Inherit` the pass writes [`Output::Opaque`], the frame over black with alpha 1.
const fn unasked_modes(translucent: Output) -> [(wgpu::CompositeAlphaMode, Output); 3] {
    [
        (wgpu::CompositeAlphaMode::PostMultiplied, translucent),
        (wgpu::CompositeAlphaMode::Opaque, Output::Opaque),
        (wgpu::CompositeAlphaMode::Inherit, Output::Opaque),
    ]
}

/// The alpha modes a surface may be configured with for what its attach asked, best first, each with
/// what the blit pass writes for it (§17). The surface gets the first row it offers and nothing
/// outside the ask's own rows: one that offers none of them refuses the attach.
///
/// An attach that did not ask ([`PreviewAlpha::Opaque`]) chooses from [`unasked_modes`] on every
/// platform. Android's surfaces offer `Inherit` alone, because its window manager, not the swapchain,
/// decides how a window is composited (the platform's Vulkan loader,
/// `frameworks/native/vulkan/libvulkan/swapchain.cpp`, reports `VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR`
/// and nothing else), so there such an attach is written [`Output::Opaque`]: the SDK cannot know how
/// that window reads alpha, and alpha 1 asks nothing of it.
///
/// An Android attach that asked to carry the composition's alpha ([`PreviewAlpha::Premultiplied`])
/// takes `PreMultiplied` where the surface offers it, else `Inherit`, and under either the pass writes
/// [`Output::Premultiplied`], the frame's alpha as it is held. Under `Inherit` that is right only for a
/// window whose compositor reads premultiplied alpha, which the app that asked knows. It never takes
/// `PostMultiplied`, whose straight colour such a compositor would add at full strength, nor `Opaque`,
/// which would hand the frame over black to an app that asked not to have it.
///
/// On iOS the translucent mode is still wgpu's `PostMultiplied`, the one non-opaque mode its Metal
/// backend offers, which only clears the layer's opaque flag. Core Animation then reads the layer as
/// premultiplied, whatever the name says (measured on the simulator, composing a half-transparent red over
/// blue), so the pass writes the frame as it is held, premultiplied: written straight, every translucent
/// pixel would show too bright. iOS and the host choose from the same rows whichever was asked: a
/// `CAMetalLayer` and a desktop window say themselves how their alpha reads, so the ask settles nothing
/// there.
const fn alpha_modes(platform: Platform, alpha: PreviewAlpha) -> &'static [(wgpu::CompositeAlphaMode, Output)] {
    match (platform, alpha) {
        (Platform::Android, PreviewAlpha::Premultiplied) => {
            &[(wgpu::CompositeAlphaMode::PreMultiplied, Output::Premultiplied), (wgpu::CompositeAlphaMode::Inherit, Output::Premultiplied)]
        }
        (Platform::Ios, _) => &const { unasked_modes(Output::Premultiplied) },
        (Platform::Android | Platform::Host, _) => &const { unasked_modes(Output::PostMultiplied) },
    }
}

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
    /// Choose from what a surface reports it supports for an attach that did not ask to carry the
    /// composition's alpha: [`from_capabilities_with_alpha`][Self::from_capabilities_with_alpha] at the
    /// default. Every attach keeps its ask from its own call, so nothing that ships asks twice; this
    /// asks-with-nothing shape stands for the tests, which weigh a choice made without an ask against
    /// one made with the default and so hold the default to it.
    #[cfg(test)]
    pub(crate) fn from_capabilities(platform: Platform, capabilities: &wgpu::SurfaceCapabilities) -> Result<Self> {
        Self::from_capabilities_with_alpha(platform, capabilities, PreviewAlpha::default())
    }

    /// Choose from what a surface reports it supports, for an attach that asked for `alpha`. The
    /// format is the first of the platform's that the surface lists. The present mode is `Fifo`,
    /// which every surface must offer, checked anyway. The alpha mode is the first row of
    /// [`alpha_modes`] for that ask that the surface lists, and what the pass writes is that row's.
    /// Anything the surface cannot give is [`CraftError::Gpu`] naming what was wanted and what it
    /// offered: for the alpha mode, the modes of the ask's own rows, so an attach that asked to carry
    /// the composition's alpha reads which modes would have carried it and can attach without the ask.
    pub(crate) fn from_capabilities_with_alpha(platform: Platform, capabilities: &wgpu::SurfaceCapabilities, alpha: PreviewAlpha) -> Result<Self> {
        let format = preferred_formats(platform).iter().copied().find(|format| capabilities.formats.contains(format)).ok_or_else(|| {
            CraftError::gpu(format!("the preview surface offers none of the formats {:?}, only {:?}", preferred_formats(platform), capabilities.formats))
        })?;
        if !capabilities.present_modes.contains(&wgpu::PresentMode::Fifo) {
            return Err(CraftError::gpu(format!("the preview surface does not offer Fifo, only {:?}", capabilities.present_modes)));
        }
        let rows = alpha_modes(platform, alpha);
        let (alpha_mode, output) = rows.iter().copied().find(|(mode, _)| capabilities.alpha_modes.contains(mode)).ok_or_else(|| {
            CraftError::gpu(format!(
                "the preview surface offers none of the alpha modes {:?}, only {:?}",
                rows.iter().map(|(mode, _)| mode).collect::<Vec<_>>(),
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

/// What the preview surface needs of a wgpu surface. The platform modules implement this over the
/// wgpu surface they made from the app's layer or window, because only that handle knows the
/// surface's current size: a `wgpu::Surface` on its own can answer nothing about the CAMetalLayer's
/// bounds or the ANativeWindow's size, which is what §17 sizes and resizes by.
pub trait Swapchain {
    /// What the surface supports on this adapter.
    fn capabilities(&self, adapter: &wgpu::Adapter) -> wgpu::SurfaceCapabilities;
    /// Configure the surface; every call replaces the last configuration.
    fn configure(&self, device: &wgpu::Device, configuration: &wgpu::SurfaceConfiguration);
    /// Take the texture to draw the next frame into, or say why there is none.
    fn acquire(&self) -> wgpu::CurrentSurfaceTexture;
    /// The surface's current size in pixels: the app's layer or window as it is now, zero on an
    /// axis with no pixels yet (§17). [`PreviewSurface::present`] compares every present against
    /// this and configures again when it has changed.
    fn current_size(&self) -> (u32, u32);
}

/// A platform's surface with its type erased, which is how the engine holds one: the message that
/// attaches a surface is built in the platform module, whose concrete wrapper type no engine code
/// names, and the surface lives on the engine's thread beside the lifecycle that owns it.
impl Swapchain for Box<dyn Swapchain + Send> {
    fn capabilities(&self, adapter: &wgpu::Adapter) -> wgpu::SurfaceCapabilities {
        (**self).capabilities(adapter)
    }

    fn configure(&self, device: &wgpu::Device, configuration: &wgpu::SurfaceConfiguration) {
        (**self).configure(device, configuration);
    }

    fn acquire(&self) -> wgpu::CurrentSurfaceTexture {
        (**self).acquire()
    }

    fn current_size(&self) -> (u32, u32) {
        (**self).current_size()
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

/// The app's preview surface, configured and ready to blit into. The engine holds one with the
/// platform's wrapper type erased ([`Box<dyn Swapchain + Send>`]); tests hold a script.
pub struct PreviewSurface<S: Swapchain = Box<dyn Swapchain + Send>> {
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
    /// `adapter`, make the blit pass for it, and configure it at the size the surface has now.
    /// This is an attach that did not ask to carry the composition's alpha.
    pub fn attach(swapchain: S, adapter: &wgpu::Adapter, device: &wgpu::Device) -> Result<Self> {
        Self::attach_with_alpha(swapchain, adapter, device, PreviewAlpha::default())
    }

    /// Take the platform's surface as [`PreviewSurface::attach`] does, for an attach that asked for
    /// `alpha`. The ask chooses the surface's alpha mode and what the pass writes for it, and a
    /// surface that offers no mode the ask takes is refused here, as [`CraftError::Gpu`], before it
    /// is configured. The surface keeps the ask in the pass it makes, which clears as
    /// [`crate::blit::clear_color`] says: an attach that asked to carry the composition's alpha
    /// leaves the bars transparent, and one that did not is exactly [`PreviewSurface::attach`].
    pub fn attach_with_alpha(swapchain: S, adapter: &wgpu::Adapter, device: &wgpu::Device, alpha: PreviewAlpha) -> Result<Self> {
        Self::attach_on(Platform::current(), swapchain, adapter, device, alpha)
    }

    /// The attach, choosing by `platform`'s rules: the build's own platform for every attach that
    /// ships, and each platform in turn for the tests, which hold a phone's whole attach — its choice,
    /// its refusal and what it draws — on the machine they run on.
    fn attach_on(platform: Platform, swapchain: S, adapter: &wgpu::Adapter, device: &wgpu::Device, alpha: PreviewAlpha) -> Result<Self> {
        let size = swapchain.current_size();
        let choice = Choice::from_capabilities_with_alpha(platform, &swapchain.capabilities(adapter), alpha)?;
        let blit = Blit::new_with_alpha(device, choice.format, choice.output, alpha)?;
        let mut surface = Self { swapchain, choice, blit, size, max_side: device.limits().max_texture_dimension_2d, configured: false, lost: false };
        surface.reconfigure(device)?;
        Ok(surface)
    }

    /// What the surface is configured with, apart from its size.
    #[must_use]
    pub const fn choice(&self) -> Choice {
        self.choice
    }

    /// The size the surface is configured at, which is the size its swapchain last reported; a
    /// surface of no pixels is unconfigured until it has some.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        self.size
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
        // §17: a layer or window resized between presents is found here first, from the handle's
        // live size — a CAMetalLayer resized with no error ever reaching wgpu, and a window resized
        // before an acquire could say so: configure again for the new size, answer `Gpu`, and the
        // next present draws. A surface that only now has a size is not a resize: it configures and
        // draws this frame.
        let size = self.swapchain.current_size();
        if size != self.size {
            let was_configured = self.configured;
            self.size = size;
            self.reconfigure(device)?;
            if !self.configured {
                // A surface that has no size, whether it never had one or has just lost it: nothing
                // there can show a frame, so this present skips, as any present on it does.
                return Ok(Presented::Skipped);
            }
            if was_configured {
                return Err(Fault::Outdated.error());
            }
        } else if !self.configured {
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
                    // The resize wgpu itself saw: take the live size with it, so the frame the
                    // next present draws goes into a surface configured for the size it has.
                    Fault::Outdated => {
                        self.size = self.swapchain.current_size();
                        self.reconfigure(device)?;
                    }
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
pub(crate) mod tests {
    use super::*;
    use crate::blit::tests::{assert_matches, frame as texels, ramp_frame, reference, target};
    use crate::testing::{BOTH, EVERYTHING, MODES, Rig, Scripted, caps, read_back, refuse, rig, texture};
    use std::cell::Cell;

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
    /// `PostMultiplied`, an opaque picture for the other two, except on iOS, where Core Animation reads
    /// every non-opaque layer as premultiplied and the pass writes the frame as it is held.
    /// `PreMultiplied` alone is refused.
    #[test]
    fn the_alpha_mode_is_post_multiplied_else_opaque_else_inherited() {
        let choose = |alpha: &[wgpu::CompositeAlphaMode]| Choice::from_capabilities(Platform::Android, &caps(&BOTH, &MODES, alpha));
        let configured = |choice: Choice| choice.configuration((8, 8), 64).map(|c| c.alpha_mode);
        let post = choose(&EVERYTHING).expect("post-multiplied is offered");
        assert_eq!((post.alpha_mode, post.output), (wgpu::CompositeAlphaMode::PostMultiplied, Output::PostMultiplied));
        assert_eq!(configured(post), Some(wgpu::CompositeAlphaMode::PostMultiplied));
        let ios = Choice::from_capabilities(Platform::Ios, &caps(&BOTH, &MODES, &EVERYTHING)).expect("post-multiplied is offered");
        assert_eq!((ios.alpha_mode, ios.output), (wgpu::CompositeAlphaMode::PostMultiplied, Output::Premultiplied));
        let ios_opaque = Choice::from_capabilities(Platform::Ios, &caps(&BOTH, &MODES, &[wgpu::CompositeAlphaMode::Opaque])).expect("opaque is offered");
        assert_eq!((ios_opaque.alpha_mode, ios_opaque.output), (wgpu::CompositeAlphaMode::Opaque, Output::Opaque));
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

    /// What each ask chooses from what a surface may offer, on each platform's rows. An attach that did
    /// not ask chooses what it always did, refusals included, and a choice made with no ask is one made
    /// with the default. An Android attach that asked to carry the composition's alpha takes
    /// `PreMultiplied`, else `Inherit`, written premultiplied under either, whatever order the surface
    /// lists them in; it never takes `PostMultiplied` or `Opaque`, so a surface that offers neither of
    /// its own two modes is refused, naming them. iOS and the host choose the same for either ask.
    #[test]
    fn the_ask_picks_the_alpha_mode_and_never_falls_back() {
        use wgpu::CompositeAlphaMode as Mode;
        let choose = |platform: Platform, offered: &[Mode], alpha: PreviewAlpha| {
            Choice::from_capabilities_with_alpha(platform, &caps(&BOTH, &MODES, offered), alpha).map(|choice| (choice.alpha_mode, choice.output))
        };
        let android = |offered: &[Mode], alpha: PreviewAlpha| choose(Platform::Android, offered, alpha);
        let refused =
            |wanted: &str, offered: &str| Err(CraftError::gpu(format!("the preview surface offers none of the alpha modes {wanted}, only {offered}")));

        // An Android attach that did not ask: the rows, and the order, it always chose by.
        let unasked = "[PostMultiplied, Opaque, Inherit]";
        assert_eq!(android(&[Mode::Inherit], PreviewAlpha::Opaque), Ok((Mode::Inherit, Output::Opaque)));
        assert_eq!(android(&[Mode::PreMultiplied], PreviewAlpha::Opaque), refused(unasked, "[PreMultiplied]"));
        assert_eq!(android(&[Mode::Inherit, Mode::PostMultiplied], PreviewAlpha::Opaque), Ok((Mode::PostMultiplied, Output::PostMultiplied)));
        assert_eq!(android(&[Mode::Opaque], PreviewAlpha::Opaque), Ok((Mode::Opaque, Output::Opaque)));
        assert_eq!(android(&[], PreviewAlpha::Opaque), refused(unasked, "[]"));

        // An Android attach that asked: a mode that carries premultiplied alpha, or no attach at all.
        let asked = "[PreMultiplied, Inherit]";
        assert_eq!(android(&[Mode::Inherit], PreviewAlpha::Premultiplied), Ok((Mode::Inherit, Output::Premultiplied)));
        assert_eq!(android(&[Mode::PreMultiplied], PreviewAlpha::Premultiplied), Ok((Mode::PreMultiplied, Output::Premultiplied)));
        assert_eq!(
            android(&[Mode::Inherit, Mode::PreMultiplied], PreviewAlpha::Premultiplied),
            Ok((Mode::PreMultiplied, Output::Premultiplied)),
            "the surface's own premultiplied mode comes before the window's"
        );
        assert_eq!(
            android(&[Mode::PostMultiplied, Mode::Inherit], PreviewAlpha::Premultiplied),
            Ok((Mode::Inherit, Output::Premultiplied)),
            "a mode that reads straight colour is never taken"
        );
        assert_eq!(android(&EVERYTHING, PreviewAlpha::Premultiplied), Ok((Mode::PreMultiplied, Output::Premultiplied)));
        assert_eq!(
            android(&[Mode::PostMultiplied, Mode::Opaque], PreviewAlpha::Premultiplied),
            refused(asked, "[PostMultiplied, Opaque]"),
            "neither the straight mode nor the opaque one stands in for what was asked"
        );
        assert_eq!(android(&[Mode::Opaque], PreviewAlpha::Premultiplied), refused(asked, "[Opaque]"));
        assert_eq!(android(&[], PreviewAlpha::Premultiplied), refused(asked, "[]"));

        // Whatever a surface offers: no ask is the default ask on every platform, and iOS and the host
        // answer one ask as they answer the other.
        let offers: [&[Mode]; 7] = [
            &[Mode::Inherit],
            &[Mode::PreMultiplied],
            &[Mode::PostMultiplied, Mode::Inherit],
            &[Mode::Opaque],
            &[Mode::PostMultiplied, Mode::Opaque],
            &EVERYTHING,
            &[],
        ];
        for offered in offers {
            for platform in [Platform::Ios, Platform::Android, Platform::Host] {
                let quiet = Choice::from_capabilities(platform, &caps(&BOTH, &MODES, offered)).map(|choice| (choice.alpha_mode, choice.output));
                assert_eq!(quiet, choose(platform, offered, PreviewAlpha::Opaque), "{platform:?} offered {offered:?}: no ask is the default ask");
            }
            for platform in [Platform::Ios, Platform::Host] {
                assert_eq!(
                    choose(platform, offered, PreviewAlpha::Premultiplied),
                    choose(platform, offered, PreviewAlpha::Opaque),
                    "{platform:?} offered {offered:?}: the ask settles nothing here"
                );
            }
        }
        // And what they answer is what they always chose: iOS's translucent layer written premultiplied,
        // the host's straight, `Opaque` and `Inherit` opaque on both, and a refusal in the rows every
        // platform had.
        assert_eq!(choose(Platform::Ios, &EVERYTHING, PreviewAlpha::Premultiplied), Ok((Mode::PostMultiplied, Output::Premultiplied)));
        assert_eq!(choose(Platform::Host, &EVERYTHING, PreviewAlpha::Premultiplied), Ok((Mode::PostMultiplied, Output::PostMultiplied)));
        for platform in [Platform::Ios, Platform::Host] {
            assert_eq!(choose(platform, &[Mode::Opaque], PreviewAlpha::Premultiplied), Ok((Mode::Opaque, Output::Opaque)), "{platform:?}");
            assert_eq!(choose(platform, &[Mode::Inherit], PreviewAlpha::Premultiplied), Ok((Mode::Inherit, Output::Opaque)), "{platform:?}");
            assert_eq!(choose(platform, &[Mode::PreMultiplied], PreviewAlpha::Premultiplied), refused(unasked, "[PreMultiplied]"), "{platform:?}");
        }
    }

    /// The ask an attach makes reaches the surface it attaches, on each platform's rows, from a surface
    /// that offers what Android's do. The choice is the ask's own, and what is drawn is what the ask
    /// says: a frame of another aspect, with soft alpha, reaches an Android attach that asked to carry
    /// the composition's alpha as its own premultiplied bytes between transparent bars, and one that
    /// did not as the frame over black between opaque black bars. iOS and the host write the frame
    /// over black under `Inherit` whichever was asked, and their bars alone follow the ask. The attach
    /// every platform's maker makes is this build's platform with the ask it was handed, and an attach
    /// that names no ask is this build's platform at the default.
    #[test]
    fn the_ask_reaches_the_attached_surface_its_picture_and_its_bars() {
        let rig = rig();
        // Premultiplied (64, 32, 0) at alpha 128, twice as wide as it is tall: bars above and below.
        let source = texels(&rig, 8, 4, |_, _| [64, 32, 0, 128]);
        let inherit_alone = || {
            let scripted = Scripted::new((8, 8), []);
            scripted.set_caps(caps(&BOTH, &MODES, &[wgpu::CompositeAlphaMode::Inherit]));
            scripted
        };
        let drawn = |surface: PreviewSurface<Scripted>| bar_and_picture(&rig, surface, &source);
        let attached = |platform: Platform, alpha: PreviewAlpha| {
            let scripted = inherit_alone();
            let surface = PreviewSurface::attach_on(platform, scripted.clone(), &rig.adapter, &rig.device, alpha).expect("every ask takes Inherit");
            let choice = Choice::from_capabilities_with_alpha(platform, &scripted.capabilities(&rig.adapter), alpha).expect("every ask takes Inherit");
            assert_eq!(surface.choice(), choice, "{platform:?} {alpha:?}: the choice is the ask's");
            drawn(surface)
        };
        let (black_bar, over_black) = (Some([0, 0, 0, 255]), Some([64, 32, 0, 255]));
        let (clear_bar, carried) = (Some([0, 0, 0, 0]), Some([64, 32, 0, 128]));
        assert_eq!(attached(Platform::Android, PreviewAlpha::Premultiplied), (clear_bar, carried), "an Android attach that asked");
        assert_eq!(attached(Platform::Android, PreviewAlpha::Opaque), (black_bar, over_black), "an Android attach that did not ask");
        for platform in [Platform::Ios, Platform::Host] {
            assert_eq!(attached(platform, PreviewAlpha::Premultiplied), (clear_bar, over_black), "{platform:?}: only the bars follow the ask");
            assert_eq!(attached(platform, PreviewAlpha::Opaque), (black_bar, over_black), "{platform:?}");
        }
        for alpha in [PreviewAlpha::Premultiplied, PreviewAlpha::Opaque] {
            let named = PreviewSurface::attach_with_alpha(inherit_alone(), &rig.adapter, &rig.device, alpha).expect("every ask takes Inherit");
            assert_eq!(drawn(named), attached(Platform::current(), alpha), "{alpha:?}: a maker's attach is this build's platform with its ask");
        }
        let quiet = PreviewSurface::attach(inherit_alone(), &rig.adapter, &rig.device).expect("an attach that names no ask takes Inherit");
        assert_eq!(drawn(quiet), attached(Platform::current(), PreviewAlpha::Opaque), "an attach that names no ask is the default ask");
    }

    /// A surface that offers neither mode an Android attach takes to carry the composition's alpha
    /// fails that attach, naming both, and is never configured; the same surface attaches for an
    /// attach that did not ask, as it always did, which is the other way the refusal leaves the app.
    #[test]
    fn attaching_to_carry_alpha_where_no_mode_carries_it_is_a_gpu_error() {
        let rig = rig();
        let scripted = Scripted::new((16, 16), []);
        scripted.set_caps(caps(&BOTH, &MODES, &[wgpu::CompositeAlphaMode::PostMultiplied, wgpu::CompositeAlphaMode::Opaque]));
        let refused = PreviewSurface::attach_on(Platform::Android, scripted.clone(), &rig.adapter, &rig.device, PreviewAlpha::Premultiplied).err();
        assert_eq!(
            refused,
            Some(CraftError::gpu("the preview surface offers none of the alpha modes [PreMultiplied, Inherit], only [PostMultiplied, Opaque]"))
        );
        assert!(scripted.configured().is_empty(), "a refused surface is never configured");
        let unasked = PreviewSurface::attach_on(Platform::Android, scripted.clone(), &rig.adapter, &rig.device, PreviewAlpha::Opaque)
            .expect("an attach that did not ask takes PostMultiplied");
        assert_eq!((unasked.choice().alpha_mode, unasked.choice().output), (wgpu::CompositeAlphaMode::PostMultiplied, Output::PostMultiplied));
        assert_eq!(scripted.configured(), [(16, 16)]);
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

    /// The corner and a pixel of the picture of the 8 by 8 texture `surface` hands over, once `frame`, twice
    /// as wide as it is tall, is drawn into it: the corner is a bar, which follows what the attach asked of
    /// the composition's alpha on every platform, so it tells one ask's surface from the other's.
    pub(crate) fn bar_and_picture<S: Swapchain>(rig: &Rig, mut surface: PreviewSurface<S>, frame: &wgpu::Texture) -> (Option<[u8; 4]>, Option<[u8; 4]>) {
        let texture = target(rig, surface.choice().format, (8, 8));
        let shown = Cell::new(0);
        let taken = Offscreen { texture: texture.clone(), shown: &shown, refuse: None };
        assert_eq!(surface.show(&rig.device, &rig.queue, frame, taken, false), Ok(Presented::Shown));
        let pixels = read_back(rig, &texture);
        (pixels.first().copied(), pixels.get(4 * 8 + 4).copied())
    }

    /// A display frame for the tests that only need one to be there.
    fn frame(rig: &Rig) -> wgpu::Texture {
        texture(&rig.device, (4, 4), 1, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::TEXTURE_BINDING)
    }

    /// Attaching configures at the size the surface has, and the present that finds a new size
    /// configures again for it and answers `Gpu`, so the next frame draws at it (§17).
    #[test]
    fn attach_and_a_resized_surface_configure_the_surface() {
        let rig = rig();
        let scripted = Scripted::new((320, 240), []);
        let mut surface = PreviewSurface::attach(scripted.clone(), &rig.adapter, &rig.device).expect("the scripted surface attaches");
        assert_eq!(surface.size(), (320, 240), "the surface attaches at the size it reports");
        assert_eq!(scripted.configured(), [(320, 240)]);
        let outdated = CraftError::gpu("the preview surface changed size or format; it is configured again, so the next frame draws");
        scripted.set_size((640, 480));
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Err(outdated.clone()));
        scripted.set_size((240, 320));
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Err(outdated));
        assert_eq!(scripted.configured(), [(320, 240), (640, 480), (240, 320)], "one configuration per size");
        assert_eq!(scripted.acquired(), 0, "a present that finds a resize does not draw");
        // The platform's own choice from what the script offers, by the rule the attach uses: the
        // tests above pin that rule for each platform, and this one that attaching applies it.
        let choice = Choice::from_capabilities(Platform::current(), &scripted.capabilities(&rig.adapter)).expect("the script offers the platform's format");
        assert_eq!(surface.choice(), choice);
    }

    /// A surface with no size is not configured, presents skip without asking the surface for a
    /// frame, and its first size brings it back.
    #[test]
    fn a_surface_with_no_size_skips_until_it_has_one() {
        let rig = rig();
        let scripted = Scripted::new((0, 0), [wgpu::CurrentSurfaceTexture::Occluded]);
        let mut surface = PreviewSurface::attach(scripted.clone(), &rig.adapter, &rig.device).expect("a surface of no size attaches");
        assert!(scripted.configured().is_empty());
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Ok(Presented::Skipped));
        assert_eq!(scripted.acquired(), 0);
        scripted.set_size((16, 16));
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Ok(Presented::Skipped));
        assert_eq!(scripted.configured(), [(16, 16)], "the first size configures the surface");
        // Now it asks, and an occluded surface is a skip, not an error.
        assert_eq!(scripted.acquired(), 1);
        scripted.set_size((0, 16));
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Ok(Presented::Skipped));
        assert_eq!(scripted.acquired(), 1, "a surface of no size is never asked for a frame");
    }

    /// A timeout skips the frame and the session goes on.
    #[test]
    fn a_timeout_skips_the_frame() {
        let rig = rig();
        let scripted = Scripted::new((16, 16), [wgpu::CurrentSurfaceTexture::Timeout]);
        let mut surface = PreviewSurface::attach(scripted.clone(), &rig.adapter, &rig.device).expect("attached");
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Ok(Presented::Skipped));
    }

    /// An outdated surface answers the `Gpu` error and is configured again at once, so the next
    /// present is on a fresh configuration; the surface is never marked lost by it.
    #[test]
    fn an_outdated_surface_is_configured_again_and_answers_gpu() {
        let rig = rig();
        let scripted = Scripted::new((16, 16), [wgpu::CurrentSurfaceTexture::Outdated, wgpu::CurrentSurfaceTexture::Outdated]);
        let mut surface = PreviewSurface::attach(scripted.clone(), &rig.adapter, &rig.device).expect("attached");
        let error = surface.present(&rig.device, &rig.queue, &frame(&rig)).expect_err("outdated is an error");
        assert_eq!(error, CraftError::gpu("the preview surface changed size or format; it is configured again, so the next frame draws"));
        assert_eq!(scripted.configured(), [(16, 16), (16, 16)], "configured again after the fault");
        // It asks again on the next present: outdated is not lost.
        assert!(surface.present(&rig.device, &rig.queue, &frame(&rig)).is_err());
        assert_eq!(scripted.acquired(), 2);
        assert_eq!(scripted.configured().len(), 3);
    }

    /// A lost surface answers the `Gpu` error, never a panic, and keeps answering it without asking
    /// the surface for another frame or configuring it, until the app attaches a new one.
    #[test]
    fn a_lost_surface_answers_gpu_and_stays_lost() {
        let rig = rig();
        let scripted = Scripted::new((16, 16), [wgpu::CurrentSurfaceTexture::Lost]);
        let mut surface = PreviewSurface::attach(scripted.clone(), &rig.adapter, &rig.device).expect("attached");
        let lost = CraftError::gpu("the preview surface was lost; attach a new surface to preview again");
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Err(lost.clone()));
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Err(lost));
        assert_eq!(scripted.acquired(), 1, "a lost surface is not asked again");
        assert_eq!(scripted.configured(), [(16, 16)], "a lost surface is not configured again");
        // The session is unharmed: a new surface attaches on the same device and presents.
        let fresh = Scripted::new((16, 16), [wgpu::CurrentSurfaceTexture::Timeout]);
        let mut surface = PreviewSurface::attach(fresh.clone(), &rig.adapter, &rig.device).expect("a new surface attaches");
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Ok(Presented::Skipped));
    }

    /// A validation error acquiring is the `Gpu` error with its own words; the surface is not lost by it.
    #[test]
    fn a_validation_error_answers_gpu_without_losing_the_surface() {
        let rig = rig();
        let scripted = Scripted::new((16, 16), [wgpu::CurrentSurfaceTexture::Validation, wgpu::CurrentSurfaceTexture::Timeout]);
        let mut surface = PreviewSurface::attach(scripted.clone(), &rig.adapter, &rig.device).expect("attached");
        let error = surface.present(&rig.device, &rig.queue, &frame(&rig)).expect_err("a validation error");
        assert_eq!(error, CraftError::gpu("the preview surface reported a validation error"));
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Ok(Presented::Skipped));
        assert_eq!(scripted.configured(), [(16, 16)]);
    }

    /// The device error wgpu raises when it refuses an acquire is the `Gpu` error with the device's
    /// words: with no error scope open, wgpu's default handler would panic instead.
    #[test]
    fn a_device_error_while_acquiring_is_a_gpu_error_not_a_panic() {
        let rig = rig();
        let scripted = Scripted::new((16, 16), [wgpu::CurrentSurfaceTexture::Validation, wgpu::CurrentSurfaceTexture::Timeout]);
        scripted.refuse(rig.device.clone());
        let mut surface = PreviewSurface::attach(scripted.clone(), &rig.adapter, &rig.device).expect("attached");
        let Err(CraftError::Gpu { message }) = surface.present(&rig.device, &rig.queue, &frame(&rig)) else {
            panic!("a refused acquire must answer a Gpu error");
        };
        assert!(message.starts_with("acquiring the preview frame: "), "{message}");
        // The surface is not lost by it: the next present asks again.
        assert_eq!(surface.present(&rig.device, &rig.queue, &frame(&rig)), Err(CraftError::Gpu { message }));
        assert_eq!(scripted.acquired(), 2);
    }

    /// A texture the surface hands over is drawn into aspect-fit at its own size, matching the
    /// reference, and shown once; a frame the surface marks suboptimal is shown, then the surface is
    /// configured again.
    #[test]
    fn a_frame_is_drawn_aspect_fit_into_the_texture_the_surface_hands_over_and_shown() {
        let rig = rig();
        let scripted = Scripted::new((48, 48), []);
        let mut surface = PreviewSurface::attach(scripted.clone(), &rig.adapter, &rig.device).expect("attached");
        let source = ramp_frame(&rig, 32, 16);
        for (size, suboptimal) in [((48, 48), false), ((24, 48), true)] {
            let drawn = target(&rig, surface.choice().format, size);
            let shown = Cell::new(0);
            let taken = Offscreen { texture: drawn.clone(), shown: &shown, refuse: None };
            assert_eq!(surface.show(&rig.device, &rig.queue, &source, taken, suboptimal), Ok(Presented::Shown));
            assert_eq!(shown.get(), 1, "{size:?}: shown once");
            assert_matches(&read_back(&rig, &drawn), &reference((32, 16), size), size.0, &format!("{size:?}"));
        }
        assert_eq!(scripted.configured(), [(48, 48), (48, 48)], "configured again after the suboptimal frame only");
    }

    /// A frame the pass cannot draw is the `Gpu` error and is not shown, so the surface keeps its last frame.
    #[test]
    fn a_frame_that_fails_to_draw_is_not_shown() {
        let rig = rig();
        let scripted = Scripted::new((16, 16), []);
        let mut surface = PreviewSurface::attach(scripted.clone(), &rig.adapter, &rig.device).expect("attached");
        let unbindable = texture(&rig.device, (4, 4), 1, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::COPY_DST);
        let shown = Cell::new(0);
        let taken = Offscreen { texture: target(&rig, surface.choice().format, (16, 16)), shown: &shown, refuse: None };
        let Err(CraftError::Gpu { message }) = surface.show(&rig.device, &rig.queue, &unbindable, taken, true) else {
            panic!("a frame that cannot be bound must answer a Gpu error");
        };
        assert!(message.starts_with("the blit draw: "), "{message}");
        assert_eq!(shown.get(), 0, "an undrawn frame is not shown");
        assert_eq!(scripted.configured(), [(16, 16)], "nothing was shown, so nothing is configured again");
    }

    /// The device error wgpu raises when it refuses to present is the `Gpu` error with the device's
    /// words, not wgpu's default panic.
    #[test]
    fn a_device_error_while_showing_is_a_gpu_error_not_a_panic() {
        let rig = rig();
        let scripted = Scripted::new((16, 16), []);
        let mut surface = PreviewSurface::attach(scripted.clone(), &rig.adapter, &rig.device).expect("attached");
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
        let scripted = Scripted::new((16, 16), []);
        scripted.set_caps(caps(&[wgpu::TextureFormat::Rgba16Float], &MODES, &EVERYTHING));
        let Err(CraftError::Gpu { message }) = PreviewSurface::attach(scripted.clone(), &rig.adapter, &rig.device) else {
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
