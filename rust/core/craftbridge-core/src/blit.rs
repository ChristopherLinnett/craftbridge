//! The blit pass: a display frame scaled into the preview surface, aspect-fit (plan §17).
//!
//! The frame is the premultiplied `Rgba8Unorm` texture the compositor leaves on the GPU, with a full
//! mip chain. The pass clears the whole target to the bar colour, then draws one full-screen
//! triangle into the viewport [`fit`] gives, so the bars are never touched by the picture and the
//! shader needs no rule of its own for them. The sampler is linear and reads across the mip chain,
//! so a frame shown smaller than it was rendered is filtered, not aliased.
//!
//! The shader is the string below rather than a `blit.wgsl` read with `include_str!`: AGENTS.md §3
//! refuses any file a crate's compilation reads that is not one of its own `.rs` files. A test reads
//! it with naga, the WGSL front end wgpu compiles it with, so a broken shader fails the gate on any
//! machine rather than on the first device that makes the pass.

use crate::Result;
use crate::gpu::validated;
use crate::surface::PreviewAlpha;

/// What the target's alpha channel means to whatever composites the surface, which decides what the
/// pass writes there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// An opaque picture: the frame's premultiplied colour is already the frame composited over black,
    /// which is what is written, with alpha 1. It is written for a surface that ignores alpha
    /// (`CompositeAlphaMode::Opaque`), and for one that leaves alpha to the window (`Inherit`) when its
    /// attach did not ask to carry the composition's alpha ([`PreviewAlpha::Opaque`]). An Android attach
    /// that asked ([`PreviewAlpha::Premultiplied`]) is written [`Output::Premultiplied`] under `Inherit`
    /// instead.
    Opaque,
    /// The surface reads straight colour with alpha (`CompositeAlphaMode::PostMultiplied` off iOS, where
    /// [`Output::Premultiplied`] takes that mode): the frame is un-premultiplied before it is written, or
    /// every soft edge would be darkened twice.
    PostMultiplied,
    /// The surface reads colour already multiplied by its alpha, as Core Animation reads every
    /// non-opaque `CAMetalLayer` whatever alpha mode wgpu was told (wgpu's Metal backend only clears the
    /// layer's opaque flag for `PostMultiplied`), and as the window of an Android attach that asked to
    /// carry the composition's alpha ([`PreviewAlpha::Premultiplied`]) reads its surface, under
    /// `PreMultiplied` or `Inherit`: the frame is premultiplied already, so it is written as it is.
    /// Written straight, a translucent pixel would be added to what is behind it at its full colour and
    /// show too bright.
    Premultiplied,
}

impl Output {
    /// The fragment entry point of [`SHADER`] that writes this output.
    const fn entry_point(self) -> &'static str {
        match self {
            Self::Opaque => "fs_opaque",
            Self::PostMultiplied => "fs_straight",
            Self::Premultiplied => "fs_premultiplied",
        }
    }
}

/// The colour of the letterbox bars of every attach that did not ask to carry the composition's alpha:
/// opaque black, whatever the output. The bars are not picture, so they never take the frame's alpha,
/// and such a preview has the same shape on every platform whichever alpha mode its surface offers.
pub const BAR_COLOR: wgpu::Color = wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

/// Nothing at all: what an attach that asked to carry the composition's alpha clears the bars to,
/// so what is behind the surface shows through them.
pub const TRANSPARENT_COLOR: wgpu::Color = wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

/// The colour the pass clears the target to before the aspect-fit draw, which is what the bars are
/// then: [`BAR_COLOR`] for every attach that did not ask to carry the composition's alpha, and
/// [`TRANSPARENT_COLOR`] for one that did — a surface attached to carry alpha shows the app's own
/// content in the bars, as it shows the composition's own alpha in the picture.
#[must_use]
pub const fn clear_color(alpha: PreviewAlpha) -> wgpu::Color {
    match alpha {
        PreviewAlpha::Opaque => BAR_COLOR,
        PreviewAlpha::Premultiplied => TRANSPARENT_COLOR,
    }
}

/// The vertex entry point of [`SHADER`].
const VERTEX_ENTRY: &str = "vs";

/// The pass's shader. One triangle large enough to cover the viewport, its texture coordinates
/// running 0 to 1 across the viewport from the top left, which is the frame's own orientation.
const SHADER: &str = "
struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;

@vertex
fn vs(@builtin(vertex_index) index: u32) -> Varying {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    var out: Varying;
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

@fragment
fn fs_opaque(in: Varying) -> @location(0) vec4<f32> {
    let color = textureSample(frame, frame_sampler, in.uv);
    return vec4<f32>(color.rgb, 1.0);
}

@fragment
fn fs_straight(in: Varying) -> @location(0) vec4<f32> {
    let color = textureSample(frame, frame_sampler, in.uv);
    if (color.a <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return vec4<f32>(min(color.rgb / color.a, vec3<f32>(1.0)), color.a);
}

@fragment
fn fs_premultiplied(in: Varying) -> @location(0) vec4<f32> {
    return textureSample(frame, frame_sampler, in.uv);
}
";

/// A rectangle of a target, in pixels from its top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// The left edge.
    pub x: u32,
    /// The top edge.
    pub y: u32,
    /// The width.
    pub width: u32,
    /// The height.
    pub height: u32,
}

/// Where a `source`-sized picture falls in a `target`-sized one, scaled by one factor on both axes
/// until it touches the target on its tighter axis and centred on the other, or nothing when either
/// has no pixels. The picture's side on the looser axis is rounded to the nearest pixel and kept at
/// one pixel or more, and the bars on a side differ by a pixel when they cannot be equal.
#[must_use]
pub fn fit(source: (u32, u32), target: (u32, u32)) -> Option<Rect> {
    let ((sw, sh), (tw, th)) = (source, target);
    if sw == 0 || sh == 0 || tw == 0 || th == 0 {
        return None;
    }
    let (wide, high, across, down) = (u128::from(sw), u128::from(sh), u128::from(tw), u128::from(th));
    let (width, height) =
        if across * high <= down * wide { (tw, narrow(rounded_div(high * across, wide), th)) } else { (narrow(rounded_div(wide * down, high), tw), th) };
    Some(Rect { x: (tw - width) / 2, y: (th - height) / 2, width, height })
}

/// `numerator / denominator` rounded to the nearest, halves up.
const fn rounded_div(numerator: u128, denominator: u128) -> u128 {
    (2 * numerator + denominator) / (2 * denominator)
}

/// `value` as a side of at least one pixel and at most `limit`.
fn narrow(value: u128, limit: u32) -> u32 {
    u32::try_from(value).unwrap_or(limit).clamp(1, limit)
}

/// The blit pass on one device, for one target format and output: its pipeline, sampler and clear
/// colour, made once.
#[derive(Debug)]
pub struct Blit {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// What the attach asked of the alpha, as the colour it clears to: [`clear_color`] answers it,
    /// so the pass remembers the ask for every frame it draws.
    clear: wgpu::Color,
}

impl Blit {
    /// The pass for targets of `format`, writing `output`, cleared to [`BAR_COLOR`]: the pass for an
    /// attach that did not ask to carry the composition's alpha.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, output: Output) -> Result<Self> {
        Self::new_with_alpha(device, format, output, PreviewAlpha::default())
    }

    /// The pass for targets of `format`, writing `output`, cleared as [`clear_color`] says for an
    /// attach that asked for `alpha`. A device that rejects the pipeline — a format it cannot render
    /// to — answers [`CraftError::Gpu`](crate::CraftError::Gpu) and nothing else.
    pub fn new_with_alpha(device: &wgpu::Device, format: wgpu::TextureFormat, output: Output, alpha: PreviewAlpha) -> Result<Self> {
        validated(device, "the blit pass", || {
            let module =
                device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("craftbridge blit"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("craftbridge blit"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("craftbridge blit"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("craftbridge blit"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some(VERTEX_ENTRY), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(output.entry_point()),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("craftbridge blit"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            });
            Self { pipeline, layout, sampler, clear: clear_color(alpha) }
        })
    }

    /// Draw `frame`, aspect-fit, into `target`, a view of a texture `target_size` pixels across,
    /// and submit the work. The whole target is cleared first, to the colour [`clear_color`] gave the
    /// pass — [`BAR_COLOR`] for one made without the ask, fully transparent for one made with it — so
    /// a frame with no pixels leaves a target of bars. A `frame` the pass cannot sample — without
    /// `TEXTURE_BINDING`, or a format that is not filterable colour — answers [`CraftError::Gpu`](crate::CraftError::Gpu).
    pub fn draw(&self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &wgpu::Texture, target: &wgpu::TextureView, target_size: (u32, u32)) -> Result<()> {
        let rect = fit((frame.width(), frame.height()), target_size);
        validated(device, "the blit draw", || {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("craftbridge blit"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&frame.create_view(&wgpu::TextureViewDescriptor::default())),
                    },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("craftbridge blit") });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("craftbridge blit"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(self.clear), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if let Some(rect) = rect {
                    #[expect(clippy::cast_precision_loss, reason = "pixel sizes under 2^24 are exact in f32; a surface is never larger")]
                    pass.set_viewport(rect.x as f32, rect.y as f32, rect.width as f32, rect.height as f32, 0.0, 1.0);
                    pass.set_pipeline(&self.pipeline);
                    pass.set_bind_group(0, &bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
            }
            queue.submit([encoder.finish()]);
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::error::CraftError;
    use crate::testing::{Rig, read_back, rig, texture};

    /// The numbers `fit` is pinned to, worked by hand: each is the one-factor scale of the picture
    /// to its tighter axis, its other side rounded, and the bars split.
    #[test]
    fn fit_scales_to_the_tighter_axis_and_centres() {
        // Wide into square: full width, bars above and below.
        assert_eq!(fit((32, 16), (48, 48)), Some(Rect { x: 0, y: 12, width: 48, height: 24 }));
        // Tall into square: full height, bars left and right.
        assert_eq!(fit((16, 32), (48, 48)), Some(Rect { x: 12, y: 0, width: 24, height: 48 }));
        // Exact fit and an exact multiple fill the target.
        assert_eq!(fit((24, 24), (24, 24)), Some(Rect { x: 0, y: 0, width: 24, height: 24 }));
        assert_eq!(fit((16, 16), (32, 32)), Some(Rect { x: 0, y: 0, width: 32, height: 32 }));
        // Same aspect at another size: full target.
        assert_eq!(fit((1920, 1080), (640, 360)), Some(Rect { x: 0, y: 0, width: 640, height: 360 }));
        // Wide target, square picture: the picture is as tall as the target and centred across.
        assert_eq!(fit((10, 10), (30, 10)), Some(Rect { x: 10, y: 0, width: 10, height: 10 }));
    }

    /// A side that does not divide is rounded to the nearest pixel, halves up, and an odd bar
    /// leaves its spare pixel at the far side.
    #[test]
    fn fit_rounds_to_the_nearest_pixel_and_splits_bars() {
        // 30x10 into 64x64: height 64 * 10 / 30 = 21.33 -> 21, bars 43 split 21 and 22.
        assert_eq!(fit((30, 10), (64, 64)), Some(Rect { x: 0, y: 21, width: 64, height: 21 }));
        // 3x2 into 10x10: height 10 * 2 / 3 = 6.67 -> 7, bars 3 split 1 and 2.
        assert_eq!(fit((3, 2), (10, 10)), Some(Rect { x: 0, y: 1, width: 10, height: 7 }));
        // Exactly one half rounds up: 4x1 into 10x10 is 2.5 -> 3.
        assert_eq!(fit((4, 1), (10, 10)), Some(Rect { x: 0, y: 3, width: 10, height: 3 }));
    }

    /// A picture thinner than a pixel on its looser axis keeps one, and sizes near the top of `u32`
    /// neither overflow nor leave the target.
    #[test]
    fn fit_keeps_one_pixel_and_survives_huge_sizes() {
        assert_eq!(fit((1000, 1), (10, 10)), Some(Rect { x: 0, y: 4, width: 10, height: 1 }));
        let huge = fit((u32::MAX, 1), (u32::MAX, u32::MAX));
        assert_eq!(huge, Some(Rect { x: 0, y: u32::MAX / 2, width: u32::MAX, height: 1 }));
        let tall = fit((1, u32::MAX), (u32::MAX, u32::MAX));
        assert_eq!(tall, Some(Rect { x: u32::MAX / 2, y: 0, width: 1, height: u32::MAX }));
    }

    /// No pixels on either side is nothing to draw.
    #[test]
    fn fit_of_an_empty_size_is_nothing() {
        assert_eq!(fit((0, 4), (8, 8)), None);
        assert_eq!(fit((4, 0), (8, 8)), None);
        assert_eq!(fit((4, 4), (0, 8)), None);
        assert_eq!(fit((4, 4), (8, 0)), None);
    }

    /// The bars are opaque black, whatever the output.
    #[test]
    fn bars_are_opaque_black() {
        assert_eq!(BAR_COLOR, wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 });
    }

    /// The shader is valid WGSL, as naga — the front end wgpu compiles it with — reads it, with no
    /// device and no optional capability, so a broken shader fails here and not on the first phone
    /// that makes the pass. Its entry points are the ones the pipeline names, each in its own stage.
    #[test]
    fn the_shader_is_valid_wgsl_with_the_entry_points_the_pass_names() {
        use wgpu::naga;
        let module = naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|error| panic!("{}", error.emit_to_string(SHADER)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(SHADER)));
        let stage_of = |name: &str| module.entry_points.iter().find(|entry| entry.name == name).map(|entry| entry.stage);
        assert_eq!(stage_of(VERTEX_ENTRY), Some(naga::ShaderStage::Vertex));
        for output in [Output::Opaque, Output::PostMultiplied, Output::Premultiplied] {
            assert_eq!(stage_of(output.entry_point()), Some(naga::ShaderStage::Fragment), "{output:?}");
        }
    }

    // ---- The pass on a real device, against references computed apart from the code ----

    /// A display frame as the compositor leaves it: `Rgba8Unorm`, one mip level, filled by `texel`.
    pub(crate) fn frame(rig: &Rig, width: u32, height: u32, texel: impl Fn(u32, u32) -> [u8; 4]) -> wgpu::Texture {
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let texture = texture(&rig.device, (width, height), 1, wgpu::TextureFormat::Rgba8Unorm, usage);
        let bytes: Vec<u8> = (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).flat_map(|(x, y)| texel(x, y)).collect();
        fill(rig, &texture, 0, (width, height), &bytes);
        texture
    }

    /// Write `bytes`, four to a pixel, row by row, into mip `level` of `texture`, which is `size` pixels there.
    fn fill(rig: &Rig, texture: &wgpu::Texture, level: u32, (width, height): (u32, u32), bytes: &[u8]) {
        rig.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
    }

    /// A target texture of `format` and `size` with a surface texture's usage, that the tests can read back.
    pub(crate) fn target(rig: &Rig, format: wgpu::TextureFormat, size: (u32, u32)) -> wgpu::Texture {
        texture(&rig.device, size, 1, format, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC)
    }

    /// Draw `frame` into a fresh offscreen texture of `format` and `size`, the surface's own usage,
    /// and read it back as straight RGBA bytes, whatever channel order the format stores.
    pub(crate) fn render(rig: &Rig, format: wgpu::TextureFormat, output: Output, frame: &wgpu::Texture, size: (u32, u32)) -> Vec<[u8; 4]> {
        let blit = Blit::new(&rig.device, format, output).expect("the blit pass is made");
        render_with(rig, &blit, format, frame, size)
    }

    fn render_with(rig: &Rig, blit: &Blit, format: wgpu::TextureFormat, frame: &wgpu::Texture, size: (u32, u32)) -> Vec<[u8; 4]> {
        let target = target(rig, format, size);
        blit.draw(&rig.device, &rig.queue, frame, &target.create_view(&Default::default()), size).expect("the draw is answered");
        read_back(rig, &target)
    }

    /// The tolerance on a colour channel (0 to 255) of a sampled pixel against the reference.
    ///
    /// The reference is exact arithmetic on a linear ramp, which linear filtering reproduces away
    /// from the picture's edges; what it cannot be is bit-exact, because the source ramp is stored in
    /// 8 bits (half a step of error), the sampler blends its two texels with weights of about 8 bits
    /// (a step of the ramp between neighbours times 1/256, under 0.1 for these sizes), and the
    /// result is rounded to 8 bits again (half a step). Their sum is under 1.2, so 2 holds with a
    /// margin and still fails a pass sampling the wrong texel, which is off by a whole ramp step
    /// (255 / (side - 1), 8 or more for the sizes used) or the wrong end of the ramp.
    const TOLERANCE: i32 = 2;

    /// The source image: red ramps left to right 0 to 255, green top to bottom, blue constant, opaque.
    /// Linear sampling reproduces a linear ramp, so the picture's value at any point is known without
    /// running the pass.
    const BLUE: u8 = 77;
    fn ramp(extent: u32, at: u32) -> u8 {
        u8::try_from((u64::from(at) * 255 + u64::from(extent - 1) / 2) / u64::from(extent - 1)).expect("a ramp step is at most 255")
    }

    /// The reference for a target of `size` showing a `source`-sized ramp frame, computed from the
    /// geometry with real numbers: the scale that touches the tighter axis, the picture centred, and
    /// at each pixel centre the point of the picture it shows. A pixel within one pixel of an edge
    /// between the picture and a bar is `None`: there the integer viewport and the exact edge may
    /// differ by half a pixel, which is a boundary of the rule rather than a value of it. The
    /// picture's edges on the target's own edges border no bar, so they are compared like the rest.
    /// Every other pixel is bar black or the ramp's own colour.
    pub(crate) fn reference(source: (u32, u32), size: (u32, u32)) -> Vec<Option<[u8; 4]>> {
        let (sw, sh) = (f64::from(source.0), f64::from(source.1));
        let (tw, th) = (f64::from(size.0), f64::from(size.1));
        let scale = (tw / sw).min(th / sh);
        let (pw, ph) = (sw * scale, sh * scale);
        let (left, top) = ((tw - pw) / 2.0, (th - ph) / 2.0);
        let (right, bottom) = (left + pw, top + ph);
        // The picture's edges that a bar lies beyond: none on a side where the picture fills the target.
        let mut edges_x = Vec::new();
        let mut edges_y = Vec::new();
        if left > 0.001 {
            edges_x.extend([left, right]);
        }
        if top > 0.001 {
            edges_y.extend([top, bottom]);
        }
        let near = |v: f64, edges: &[f64]| edges.iter().any(|edge| (v - edge).abs() < 1.0);
        let mut expected = Vec::new();
        for y in 0..size.1 {
            for x in 0..size.0 {
                let (cx, cy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let inside = cx > left && cx < right && cy > top && cy < bottom;
                let along_x = cx > left - 1.0 && cx < right + 1.0;
                let along_y = cy > top - 1.0 && cy < bottom + 1.0;
                let on_edge = (along_y && near(cx, &edges_x)) || (along_x && near(cy, &edges_y));
                expected.push(if on_edge {
                    None
                } else if inside {
                    let tx = ((cx - left) / pw * sw - 0.5).clamp(0.0, sw - 1.0);
                    let ty = ((cy - top) / ph * sh - 0.5).clamp(0.0, sh - 1.0);
                    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a value in 0..=255, rounded")]
                    Some([(tx / (sw - 1.0) * 255.0).round() as u8, (ty / (sh - 1.0) * 255.0).round() as u8, BLUE, 255])
                } else {
                    Some([0, 0, 0, 255])
                });
            }
        }
        expected
    }

    /// Compare every pixel the reference decides, and say which pixel on a miss.
    pub(crate) fn assert_matches(got: &[[u8; 4]], want: &[Option<[u8; 4]>], width: u32, what: &str) {
        assert_eq!(got.len(), want.len(), "{what}: pixel count");
        let mut compared = 0;
        for (i, (got, want)) in got.iter().zip(want).enumerate() {
            let Some(want) = want else { continue };
            compared += 1;
            let (x, y) = (i as u32 % width, i as u32 / width);
            for channel in 0..4 {
                assert!(
                    (i32::from(got[channel]) - i32::from(want[channel])).abs() <= TOLERANCE,
                    "{what}: pixel ({x}, {y}) channel {channel} is {got:?}, the reference says {want:?}"
                );
            }
        }
        // Most of the target is decided: a reference that excused everything would prove nothing.
        assert!(compared * 10 >= got.len() * 8, "{what}: only {compared} of {} pixels were compared", got.len());
    }

    const FORMATS: [wgpu::TextureFormat; 2] = [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm];

    pub(crate) fn ramp_frame(rig: &Rig, width: u32, height: u32) -> wgpu::Texture {
        frame(rig, width, height, |x, y| [ramp(width, x), ramp(height, y), BLUE, 255])
    }

    /// A wide frame in a square target: bars above and below, in both formats the surface can take.
    #[test]
    fn a_wide_frame_gets_bars_above_and_below() {
        let rig = rig();
        let source = ramp_frame(&rig, 32, 16);
        for format in FORMATS {
            let got = render(&rig, format, Output::Opaque, &source, (48, 48));
            assert_matches(&got, &reference((32, 16), (48, 48)), 48, &format!("wide, {format:?}"));
            // The bars are where the plan puts them, read straight off the picture.
            assert_eq!(got.first(), Some(&[0, 0, 0, 255]), "the top left is a bar");
            assert_eq!(got.get(47 * 48 + 47), Some(&[0, 0, 0, 255]), "the bottom right is a bar");
        }
    }

    /// A tall frame in a square target: bars left and right.
    #[test]
    fn a_tall_frame_gets_bars_left_and_right() {
        let rig = rig();
        let source = ramp_frame(&rig, 16, 32);
        for format in FORMATS {
            let got = render(&rig, format, Output::Opaque, &source, (48, 48));
            assert_matches(&got, &reference((16, 32), (48, 48)), 48, &format!("tall, {format:?}"));
            assert_eq!(got.get(24 * 48), Some(&[0, 0, 0, 255]), "the left edge is a bar");
        }
    }

    /// A frame whose aspect is the target's fills it with no bars at all, at its own size and scaled.
    #[test]
    fn an_exact_fit_has_no_bars() {
        let rig = rig();
        for (source, size) in [((24, 24), (24, 24)), ((16, 12), (32, 24)), ((32, 16), (16, 8))] {
            let frame = ramp_frame(&rig, source.0, source.1);
            for format in FORMATS {
                let got = render(&rig, format, Output::Opaque, &frame, size);
                assert_matches(&got, &reference(source, size), size.0, &format!("{source:?} into {size:?}, {format:?}"));
                // No pixel of an exact fit is bar black: blue is constant 77 across the picture.
                assert!(got.iter().all(|px| px[2].abs_diff(BLUE) <= 2), "{source:?} into {size:?} shows a bar");
            }
        }
    }

    /// One pass drawn into targets of two sizes in turn, as the app resizes: each draw fits the size it
    /// is given, so the bars move from above and below to none.
    #[test]
    fn a_resize_fits_the_new_size() {
        let rig = rig();
        let format = wgpu::TextureFormat::Bgra8Unorm;
        let source = ramp_frame(&rig, 32, 16);
        let blit = Blit::new(&rig.device, format, Output::Opaque).expect("the blit pass is made");
        for size in [(48, 48), (64, 32), (24, 48), (48, 48)] {
            let got = render_with(&rig, &blit, format, &source, size);
            assert_matches(&got, &reference((32, 16), size), size.0, &format!("resized to {size:?}"));
        }
    }

    /// A frame shown at half size reads the next mip level, not level 0: the compositor leaves a full
    /// chain for exactly this. Level 0 is red and level 1 green, so which was sampled is plain.
    #[test]
    fn a_smaller_view_reads_the_mip_chain() {
        let rig = rig();
        let source = texture(&rig.device, (16, 16), 2, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        for (level, side, color) in [(0_u32, 16_u32, [255_u8, 0, 0, 255]), (1, 8, [0, 255, 0, 255])] {
            let bytes: Vec<u8> = (0..side * side).flat_map(|_| color).collect();
            fill(&rig, &source, level, (side, side), &bytes);
        }
        let full = render(&rig, wgpu::TextureFormat::Rgba8Unorm, Output::Opaque, &source, (16, 16));
        assert!(full.iter().all(|px| *px == [255, 0, 0, 255]), "a frame at its own size reads level 0");
        let half = render(&rig, wgpu::TextureFormat::Rgba8Unorm, Output::Opaque, &source, (8, 8));
        assert!(half.iter().all(|px| *px == [0, 255, 0, 255]), "a frame at half size reads level 1");
    }

    /// A frame with soft alpha, premultiplied as the compositor leaves it, reaches an opaque surface as
    /// itself over black, a post-multiplied one as straight colour with its alpha, and a premultiplied one
    /// as it is.
    #[test]
    fn alpha_follows_the_output() {
        let rig = rig();
        // Premultiplied (64, 32, 0) at alpha 128: straight (127.5, 63.75, 0).
        let source = frame(&rig, 8, 8, |_, _| [64, 32, 0, 128]);
        let opaque = render(&rig, wgpu::TextureFormat::Rgba8Unorm, Output::Opaque, &source, (8, 8));
        let straight = render(&rig, wgpu::TextureFormat::Rgba8Unorm, Output::PostMultiplied, &source, (8, 8));
        for px in &opaque {
            assert_eq!(px[3], 255, "an opaque surface is written alpha 1");
            assert!((i32::from(px[0]) - 64).abs() <= TOLERANCE && (i32::from(px[1]) - 32).abs() <= TOLERANCE && px[2] <= TOLERANCE as u8, "{px:?}");
        }
        for px in &straight {
            assert!((i32::from(px[0]) - 128).abs() <= TOLERANCE && (i32::from(px[1]) - 64).abs() <= TOLERANCE && px[2] <= TOLERANCE as u8, "{px:?}");
            assert!((i32::from(px[3]) - 128).abs() <= TOLERANCE, "{px:?}");
        }
        let premultiplied = render(&rig, wgpu::TextureFormat::Rgba8Unorm, Output::Premultiplied, &source, (8, 8));
        for px in &premultiplied {
            assert_eq!(px, &[64, 32, 0, 128], "a premultiplied surface takes the frame's own bytes");
        }
        // Where the frame has no alpha the post-multiplied picture is clear, and the bars stay opaque black.
        let clear = frame(&rig, 8, 4, |_, _| [0, 0, 0, 0]);
        let got = render(&rig, wgpu::TextureFormat::Rgba8Unorm, Output::PostMultiplied, &clear, (8, 8));
        assert_eq!(got.first(), Some(&[0, 0, 0, 255]), "a bar");
        assert_eq!(got.get(4 * 8 + 4), Some(&[0, 0, 0, 0]), "a clear picture");
    }

    /// The clear under the picture follows what the attach asked, and needs no device to say so: the
    /// bar colour for an attach that did not ask to carry the composition's alpha, the default among
    /// them, and fully transparent for one that did.
    #[test]
    fn the_clear_follows_what_the_attach_asked() {
        assert_eq!(clear_color(PreviewAlpha::default()), BAR_COLOR, "the default of every attach clears the bars opaque black");
        assert_eq!(clear_color(PreviewAlpha::Opaque), BAR_COLOR);
        assert_eq!(clear_color(PreviewAlpha::Premultiplied), TRANSPARENT_COLOR);
        assert_eq!(TRANSPARENT_COLOR, wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 });
    }

    /// A pass made with the ask draws its bars as the ask says and its picture the same either way:
    /// the colour the pass was given is the one it clears to on a real device.
    #[test]
    fn a_pass_clears_its_bars_as_its_attach_asked() {
        let rig = rig();
        let source = frame(&rig, 8, 4, |_, _| [64, 32, 0, 128]);
        for (alpha, bar) in [(PreviewAlpha::Opaque, [0, 0, 0, 255]), (PreviewAlpha::Premultiplied, [0, 0, 0, 0])] {
            let blit = Blit::new_with_alpha(&rig.device, wgpu::TextureFormat::Rgba8Unorm, Output::Premultiplied, alpha).expect("the blit pass is made");
            let got = render_with(&rig, &blit, wgpu::TextureFormat::Rgba8Unorm, &source, (8, 8));
            assert_eq!(got.first(), Some(&bar), "{alpha:?}: the bar follows the ask");
            assert_eq!(got.get(4 * 8 + 4), Some(&[64, 32, 0, 128]), "{alpha:?}: the picture is the frame's own bytes");
        }
    }

    /// A frame of no pixels, and a target of no pixels, are answered, not drawn and not a panic.
    #[test]
    fn an_empty_target_draws_nothing_and_answers() {
        let rig = rig();
        let blit = Blit::new(&rig.device, wgpu::TextureFormat::Bgra8Unorm, Output::Opaque).expect("the blit pass is made");
        let source = ramp_frame(&rig, 4, 4);
        let view = target(&rig, wgpu::TextureFormat::Bgra8Unorm, (4, 4)).create_view(&Default::default());
        assert_eq!(blit.draw(&rig.device, &rig.queue, &source, &view, (0, 4)), Ok(()));
    }

    /// A frame the pass cannot sample is a `Gpu` error with the device's words, not a panic: here a
    /// texture made without `TEXTURE_BINDING`.
    #[test]
    fn a_frame_that_cannot_be_sampled_is_a_gpu_error() {
        let rig = rig();
        let blit = Blit::new(&rig.device, wgpu::TextureFormat::Bgra8Unorm, Output::Opaque).expect("the blit pass is made");
        let view = target(&rig, wgpu::TextureFormat::Bgra8Unorm, (4, 4)).create_view(&Default::default());
        let unbindable = texture(&rig.device, (4, 4), 1, wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::COPY_DST);
        let Err(CraftError::Gpu { message }) = blit.draw(&rig.device, &rig.queue, &unbindable, &view, (4, 4)) else {
            panic!("a texture that cannot be bound must answer a Gpu error");
        };
        assert!(message.starts_with("the blit draw: "), "{message}");
    }

    /// A target format the device cannot render to is a `Gpu` error from `new`, in the pass's own words.
    #[test]
    fn a_format_the_device_cannot_render_to_is_a_gpu_error() {
        let rig = rig();
        let Err(CraftError::Gpu { message }) = Blit::new(&rig.device, wgpu::TextureFormat::Depth32Float, Output::Opaque) else {
            panic!("a depth format is no colour target");
        };
        assert!(message.starts_with("the blit pass: "), "{message}");
    }
}
