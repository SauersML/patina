//! GPU chrome: raw WGSL painted through egui's wgpu callbacks.
//!
//! One pipeline, one dynamic-offset uniform buffer, two modes:
//!   mode 0 — the sky: a living Frutiger Aero field (azure → pale horizon,
//!            a breathing sun bloom, drifting cloud wisps, a green horizon
//!            glow), evaluated per-pixel every frame.
//!   mode 1 — glass: the SAME sky field evaluated with analytic blur
//!            (light sources widen, detail attenuates — Gaussians compose),
//!            whitened into frosted glass, with grain, a moving light
//!            sweep, top sheen, an SDF drop shadow and rim light.
//!
//! Because the glass re-evaluates the field rather than sampling the
//! framebuffer, it needs no blur passes and no copies — it is exact,
//! animated, and free.

use eframe::egui;
use eframe::egui_wgpu::{self, wgpu, CallbackResources, RenderState, ScreenDescriptor};

/// Shadow padding baked around glass panels, in points. Must match the
/// `pad` constant in the WGSL.
pub const PAD: f32 = 22.0;

const SLOT: u64 = 256; // min_uniform_buffer_offset_alignment-safe stride
const MAX_SLOTS: u64 = 64;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    rect_min: [f32; 2],
    rect_size: [f32; 2],
    screen: [f32; 2],
    time: f32,
    mode: f32,
    corner: f32,
    _pad: [f32; 3],
}

pub struct AuroraPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
}

/// Build the pipeline once and park it in egui's callback resources.
pub fn init(rs: &RenderState) {
    let device = &rs.device;

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("patina-aurora"),
        source: wgpu::ShaderSource::Wgsl(WGSL.into()),
    });

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("patina-aurora-bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(SLOT),
            },
            count: None,
        }],
    });

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("patina-aurora-uniforms"),
        size: SLOT * MAX_SLOTS,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("patina-aurora-bg"),
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: wgpu::BufferSize::new(SLOT),
            }),
        }],
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("patina-aurora-layout"),
        bind_group_layouts: &[&bgl],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("patina-aurora-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: rs.target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });

    rs.renderer.write().callback_resources.insert(AuroraPipeline {
        pipeline,
        bind_group,
        buffer,
    });
}

struct AuroraCallback {
    slot: u32,
    uniforms: Uniforms,
}

impl egui_wgpu::CallbackTrait for AuroraCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(p) = callback_resources.get::<AuroraPipeline>() {
            queue.write_buffer(
                &p.buffer,
                self.slot as u64 * SLOT,
                bytemuck::bytes_of(&self.uniforms),
            );
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        if let Some(p) = callback_resources.get::<AuroraPipeline>() {
            render_pass.set_pipeline(&p.pipeline);
            render_pass.set_bind_group(0, &p.bind_group, &[(self.slot as u64 * SLOT) as u32]);
            render_pass.draw(0..3, 0..1);
        }
    }
}

fn shape(rect: egui::Rect, screen: egui::Rect, time: f32, mode: f32, corner: f32, slot: u32) -> egui::Shape {
    let uniforms = Uniforms {
        rect_min: [rect.left(), rect.top()],
        rect_size: [rect.width(), rect.height()],
        screen: [screen.width().max(1.0), screen.height().max(1.0)],
        time,
        mode,
        corner,
        _pad: [0.0; 3],
    };
    egui::Shape::Callback(egui_wgpu::Callback::new_paint_callback(
        rect,
        AuroraCallback { slot, uniforms },
    ))
}

/// The animated sky, filling `screen`. Uses uniform slot 0.
pub fn sky_shape(screen: egui::Rect, time: f32) -> egui::Shape {
    shape(screen, screen, time, 0.0, 0.0, 0)
}

/// A living frosted-glass pane over `panel` (shadow pad added around it).
pub fn glass_shape(panel: egui::Rect, screen: egui::Rect, time: f32, corner: f32, slot: u32) -> egui::Shape {
    shape(panel.expand(PAD), screen, time, 1.0, corner, slot)
}

const WGSL: &str = r#"
struct U {
  rect_min: vec2<f32>,
  rect_size: vec2<f32>,
  screen: vec2<f32>,
  time: f32,
  mode: f32,
  corner: f32,
  pad0: f32,
  pad1: f32,
  pad2: f32,
};
@group(0) @binding(0) var<uniform> u: U;

struct VOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VOut {
  var o: VOut;
  let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
  o.uv = uv;
  o.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
  return o;
}

fn hash2(p: vec2<f32>) -> f32 {
  return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn vnoise(p: vec2<f32>) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let s = f * f * (3.0 - 2.0 * f);
  let a = hash2(i);
  let b = hash2(i + vec2<f32>(1.0, 0.0));
  let c = hash2(i + vec2<f32>(0.0, 1.0));
  let d = hash2(i + vec2<f32>(1.0, 1.0));
  return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

fn fbm(p: vec2<f32>) -> f32 {
  var v = 0.0;
  var a = 0.5;
  var q = p;
  for (var i = 0; i < 4; i = i + 1) {
    v = v + a * vnoise(q);
    q = q * 2.03;
    a = a * 0.5;
  }
  return v;
}

// The Frutiger Aero sky. `blur` widens every light source analytically —
// this IS the glass blur, computed in closed form.
fn sky(w: vec2<f32>, t: f32, blur: f32) -> vec3<f32> {
  var col = mix(
    vec3<f32>(0.22, 0.50, 0.83),
    vec3<f32>(0.72, 0.90, 0.95),
    pow(clamp(w.y, 0.0, 1.0), 1.25)
  );
  // Sun bloom, breathing
  let sun_pos = vec2<f32>(0.20 + 0.015 * sin(t * 0.11), 0.14 + 0.010 * sin(t * 0.07 + 1.7));
  let sr = 0.26 * (1.0 + blur * 1.4) * (1.0 + 0.03 * sin(t * 0.23));
  let sd = (w - sun_pos) / sr;
  col = col + vec3<f32>(1.00, 0.95, 0.78) * exp(-dot(sd, sd)) * 0.75;
  // Aqua counter-glow low right
  let gd = (w - vec2<f32>(0.86, 0.80)) / (0.45 * (1.0 + blur));
  col = col + vec3<f32>(0.28, 0.82, 0.72) * exp(-dot(gd, gd)) * 0.20;
  // Drifting cloud wisps (detail dies under blur)
  let cl = fbm(w * vec2<f32>(3.0, 6.5) + vec2<f32>(t * 0.012, 0.0));
  let clouds = smoothstep(0.52, 0.80, cl) * (1.0 - blur * 0.6);
  col = mix(col, vec3<f32>(1.0, 1.0, 1.0), clouds * 0.24);
  // Green horizon glow
  col = col + vec3<f32>(0.25, 0.52, 0.22) * 0.12 * smoothstep(0.84, 1.0, w.y);
  return col;
}

fn panel_sdf(lp: vec2<f32>, size: vec2<f32>, r: f32) -> f32 {
  let half = size * 0.5;
  let q = abs(lp - half) - (half - vec2<f32>(r, r));
  return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
  let p = u.rect_min + in.uv * u.rect_size;
  let w = p / u.screen;

  if (u.mode < 0.5) {
    var col = sky(w, u.time, 0.0);
    col = col + (hash2(p) - 0.5) * 0.008;
    return vec4<f32>(col, 1.0);
  }

  // Glass pane; the callback rect carries `pad` of shadow around the panel
  let pad = 22.0;
  let inner = u.rect_size - vec2<f32>(pad * 2.0, pad * 2.0);
  let lp = in.uv * u.rect_size - vec2<f32>(pad, pad);
  let sdf = panel_sdf(lp, inner, u.corner);

  // Soft drop shadow, biased downward
  let sh = exp(-max(sdf - 2.0, 0.0) * 0.14) * 0.28 * step(0.0, sdf);

  var col = sky(w, u.time, 1.0);
  // Frosted white glass
  col = mix(col, vec3<f32>(1.0, 1.0, 1.0), 0.44);
  col = col + vec3<f32>(-0.005, 0.006, 0.014);
  // Frost grain
  col = col + (vnoise(p * 1.9) - 0.5) * 0.018;
  // A light sweep slowly crossing the pane
  let swp = fract(u.time * 0.02);
  let dband = (in.uv.x * 0.9 - in.uv.y * 0.45) - (swp * 2.6 - 0.8);
  col = col + vec3<f32>(1.0, 1.0, 1.0) * exp(-dband * dband * 34.0) * 0.07;
  // Top sheen, bottom seat
  let fy = lp.y / max(inner.y, 1.0);
  let sheen = clamp(1.0 - fy * 2.2, 0.0, 1.0);
  col = col + sheen * sheen * 0.10;
  col = col - clamp((fy - 0.85) / 0.15, 0.0, 1.0) * 0.06;
  // Rim light just inside the edge
  let rim = exp(-abs(sdf + 1.2) * 1.1);
  col = col + rim * 0.16;

  let cover = clamp(0.5 - sdf, 0.0, 1.0);
  let alpha = clamp(cover + sh, 0.0, 1.0);
  return vec4<f32>(col * cover, alpha);
}
"#;
