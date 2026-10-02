//! Picture placement and the scaler (docs/native.md "Window and scaling").
//!
//! The picture fills the largest rectangle that fits the window at the
//! frame's aspect, at any (fractional) scale. Nearest-neighbour at 7.14x
//! makes pixel columns 7 or 8 wide and shimmers when scrolling, so the
//! shader does "sharp bilinear": the equivalent of a nearest-neighbour
//! pre-scale by `ceil(scale)` followed by a linear resize to the fit size,
//! in one pass (each texel is flat but for a seam under one output pixel
//! wide that is blended).
//!
//! `--widescreen auto` picks the margin whose frame aspect is closest to the
//! window's; the leftover mismatch (at most half an 8-px tile per side, a
//! few percent) is then stretched away so the window is filled edge to edge.

use mmx_core::Widescreen;
use pixels::wgpu;

/// In `auto`, stretch to fill when the aspect error is at most this
/// (relative). Half a tile step is 8/256 = 3.1% at the narrowest frame.
pub const AUTO_FILL_TOLERANCE: f64 = 0.035;

/// Where the picture goes in the window, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The largest `frame`-aspect rectangle inside `win`, centred. With
/// `fill_tolerance > 0`, a window whose aspect is within that relative error
/// of the frame's gets the whole window (a slight non-uniform stretch).
pub fn fit_rect(win: (u32, u32), frame: (u32, u32), fill_tolerance: f64) -> Rect {
    let (ww, wh) = (f64::from(win.0.max(1)), f64::from(win.1.max(1)));
    let (fw, fh) = (f64::from(frame.0.max(1)), f64::from(frame.1.max(1)));
    let err = (ww / wh) / (fw / fh) - 1.0;
    if err.abs() <= fill_tolerance {
        return Rect { x: 0, y: 0, w: win.0, h: win.1 };
    }
    let s = (ww / fw).min(wh / fh);
    let w = ((fw * s).round() as u32).min(win.0);
    let h = ((fh * s).round() as u32).min(win.1);
    Rect { x: (win.0 - w) / 2, y: (win.1 - h) / 2, w, h }
}

/// The widescreen margin (8-px tiles per side, 0-24) whose `256 + 16n` by
/// `frame_h` frame is closest in aspect to the window.
pub fn auto_margin(win: (u32, u32), frame_h: u32) -> Widescreen {
    let want = f64::from(win.0) * f64::from(frame_h) / f64::from(win.1.max(1));
    let n = ((want - 256.0) / 16.0).round().clamp(0.0, f64::from(Widescreen::MAX_TILES));
    Widescreen(n as u8)
}

const SHADER: &str = r"
struct Locals { tex: vec2<f32>, prescale: vec2<f32> }
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@group(0) @binding(2) var<uniform> u: Locals;

struct V { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> }

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> V {
    // One triangle covering the viewport.
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var o: V;
    o.pos = vec4<f32>(p, 0.0, 1.0);
    o.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return o;
}

@fragment
fn fs_main(v: V) -> @location(0) vec4<f32> {
    // Sharp bilinear: flat inside each texel, a linear seam 1/prescale wide.
    let texel = v.uv * u.tex;
    let base = floor(texel);
    let d = fract(texel) - 0.5;
    let region = 0.5 - 0.5 / u.prescale;
    let f = (d - clamp(d, -region, region)) * u.prescale + 0.5;
    return textureSample(t, s, (base + f) / u.tex);
}
";

/// Draws the pixels texture into a [`Rect`] of the surface (black around it).
pub struct FitRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    /// Rebuilt when pixels recreates its texture (`resize_buffer`).
    bind: Option<(wgpu::Extent3d, wgpu::BindGroup)>,
}

impl FitRenderer {
    pub fn new(p: &pixels::Pixels) -> Self {
        let device = p.device();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mmx_fit"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mmx_fit"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mmx_fit"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mmx_fit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mmx_fit"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mmx_fit"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &module, entry_point: "vs_main", buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: p.render_texture_format(),
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
        });
        FitRenderer { pipeline, layout, sampler, uniforms, bind: None }
    }

    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        ctx: &pixels::PixelsContext,
        dest: Rect,
    ) {
        let ext = ctx.texture_extent;
        if self.bind.as_ref().is_none_or(|(e, _)| *e != ext) {
            let view = ctx.texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mmx_fit"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: self.uniforms.as_entire_binding() },
                ],
            });
            self.bind = Some((ext, bg));
        }
        let (tw, th) = (ext.width as f32, ext.height as f32);
        let pre = |d: u32, t: f32| (d as f32 / t).ceil().max(1.0);
        let locals = [tw, th, pre(dest.w, tw), pre(dest.h, th)];
        let bytes: Vec<u8> = locals.iter().flat_map(|f| f.to_ne_bytes()).collect();
        ctx.queue.write_buffer(&self.uniforms, 0, &bytes);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mmx_fit"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        if dest.w == 0 || dest.h == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind.as_ref().expect("bind group").1, &[]);
        pass.set_viewport(dest.x as f32, dest.y as f32, dest.w as f32, dest.h as f32, 0.0, 1.0);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: u32, y: u32, w: u32, h: u32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn auto_margin_per_display() {
        // 16:10: 358.4 px wanted -> 6 tiles (352 px, 1.8% narrow).
        assert_eq!(auto_margin((2560, 1600), 224), Widescreen(6));
        // 16:9: 398.2 px -> 9 tiles, the 16:9 preset (400 px).
        assert_eq!(auto_margin((1920, 1080), 224), Widescreen::W16_9);
        assert_eq!(auto_margin((3840, 2160), 224), Widescreen::W16_9);
        // 43:18 ultrawide: 535.1 px -> 17 tiles, the 21:9 preset (528 px).
        assert_eq!(auto_margin((3440, 1440), 224), Widescreen::W21_9);
        // Tall and narrow: no margin; super-wide: clamped to 24.
        assert_eq!(auto_margin((800, 1200), 224), Widescreen::OFF);
        assert_eq!(auto_margin((5120, 1000), 224), Widescreen(24));
        // The plain 3x window stays at 256.
        assert_eq!(auto_margin((768, 672), 224), Widescreen::OFF);
    }

    #[test]
    fn fit_letterboxes_fixed_aspect() {
        // 400x224 on 16:10: full width, bars top and bottom.
        assert_eq!(fit_rect((2560, 1600), (400, 224), 0.0), r(0, 83, 2560, 1434));
        // 256x224 on 16:9: full height, fractional 4.82x, bars left and right.
        assert_eq!(fit_rect((1920, 1080), (256, 224), 0.0), r(343, 0, 1234, 1080));
        // 528x224 on 3440x1440: 6.43x, 3394 wide.
        assert_eq!(fit_rect((3440, 1440), (528, 224), 0.0), r(23, 0, 3394, 1440));
        // Tall narrow window: full width, bars top and bottom.
        assert_eq!(fit_rect((800, 1200), (256, 224), 0.0), r(0, 250, 800, 700));
        // Exact integer fit.
        assert_eq!(fit_rect((768, 672), (256, 224), 0.0), r(0, 0, 768, 672));
    }

    #[test]
    fn auto_fills_the_window() {
        for win in [(2560, 1600), (1920, 1080), (3440, 1440), (1440, 900)] {
            let fw = auto_margin(win, 224).width() as u32;
            assert_eq!(fit_rect(win, (fw, 224), AUTO_FILL_TOLERANCE), r(0, 0, win.0, win.1), "{win:?}");
        }
        // Too tall for any margin: plain fit.
        assert_eq!(fit_rect((800, 1200), (256, 224), AUTO_FILL_TOLERANCE), r(0, 250, 800, 700));
    }
}
