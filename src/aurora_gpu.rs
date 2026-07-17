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
    energy: f32,
    brightness: f32,
    phase: f32,
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

fn shape(
    rect: egui::Rect,
    screen: egui::Rect,
    time: f32,
    mode: f32,
    corner: f32,
    mood: [f32; 3],
    slot: u32,
) -> egui::Shape {
    let uniforms = Uniforms {
        rect_min: [rect.left(), rect.top()],
        rect_size: [rect.width(), rect.height()],
        screen: [screen.width().max(1.0), screen.height().max(1.0)],
        time,
        mode,
        corner,
        energy: mood[0],
        brightness: mood[1],
        phase: mood[2],
    };
    egui::Shape::Callback(egui_wgpu::Callback::new_paint_callback(
        rect,
        AuroraCallback { slot, uniforms },
    ))
}

/// The animated sky, filling `screen`. Uses uniform slot 0. `mood` is
/// [energy, brightness, phase] — the slow-smoothed voice of the engine.
pub fn sky_shape(screen: egui::Rect, time: f32, mood: [f32; 3]) -> egui::Shape {
    shape(screen, screen, time, 0.0, 0.0, mood, 0)
}

/// A living frosted-glass pane over `panel` (shadow pad added around it).
pub fn glass_shape(
    panel: egui::Rect,
    screen: egui::Rect,
    time: f32,
    corner: f32,
    mood: [f32; 3],
    slot: u32,
) -> egui::Shape {
    shape(panel.expand(PAD), screen, time, 1.0, corner, mood, slot)
}

const WGSL: &str = r#"
struct U {
  rect_min: vec2<f32>,
  rect_size: vec2<f32>,
  screen: vec2<f32>,
  time: f32,
  mode: f32,
  corner: f32,
  energy: f32,
  brightness: f32,
  phase: f32,
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

// A drifting orb of light: everything is a Gaussian — a soft luminous
// shell whose density peaks near (not at) the boundary, an interior haze,
// and a wide specular bloom. No hard edges anywhere; the form breathes,
// and its radius wobbles slowly so the silhouette never reads as a circle.
fn bubble(w: vec2<f32>, c: vec2<f32>, r: f32, t: f32, blur: f32) -> vec3<f32> {
  let v = w - c;
  let ang = atan2(v.y, v.x);
  // Organic silhouette: radius modulated around the circumference
  let wob = 1.0 + 0.10 * sin(ang * 2.0 + t * 0.21) + 0.05 * sin(ang * 5.0 - t * 0.13);
  let d = length(v) / (r * wob);
  // Gaussian shell centred just inside the boundary; blur fattens it
  let shell_w = 0.16 + blur * 0.55;
  let shell = exp(-(d - 0.86) * (d - 0.86) / (shell_w * shell_w));
  // Thin-film tint drifting around the shell
  let film = vec3<f32>(
    0.72 + 0.24 * sin(ang * 3.0 + t * 0.3),
    0.84 + 0.16 * sin(ang * 3.0 + 2.1 + t * 0.3),
    0.96
  );
  var glow = film * shell * (0.26 - blur * 0.13) * (0.75 + u.energy * 0.55);
  // Interior haze, densest at the middle, vanishing before the shell
  glow = glow + vec3<f32>(0.85, 0.94, 1.0) * exp(-d * d * 2.2) * 0.055;
  // Broad specular bloom toward the sun, upper-left
  let bead = v / (r * wob) + vec2<f32>(0.34, 0.38);
  glow = glow + vec3<f32>(1.0, 1.0, 1.0) * exp(-dot(bead, bead) * (14.0 - blur * 9.0)) * (0.28 - blur * 0.16);
  return glow;
}

// Domain-warped cloud density: fbm displaced by fbm gives billowing
// cumulus shapes instead of flat threshold smudges.
fn cloud_density(p: vec2<f32>, ph: f32) -> f32 {
  let q = vec2<f32>(
    fbm(p + vec2<f32>(ph * 1.4, 0.0)),
    fbm(p + vec2<f32>(5.2, 1.3) - vec2<f32>(ph * 1.1, 0.0))
  );
  return fbm(p + q * 1.5 + vec2<f32>(ph * 0.8, 0.0));
}

// The Frutiger Aero sky. `blur` widens every light source analytically —
// this IS the glass blur, computed in closed form.
fn sky(w: vec2<f32>, t: f32, blur: f32) -> vec3<f32> {
  let y = clamp(w.y, 0.0, 1.0);
  // Three-stop gradient: deep zenith, luminous mid, warm pale horizon
  var col = mix(vec3<f32>(0.13, 0.38, 0.76), vec3<f32>(0.36, 0.66, 0.91), smoothstep(0.0, 0.70, y));
  col = mix(col, vec3<f32>(0.84, 0.94, 0.96), smoothstep(0.40, 1.05, y));
  // Sun bloom, breathing
  let sun_pos = vec2<f32>(0.20 + 0.015 * sin(t * 0.11), 0.14 + 0.010 * sin(t * 0.07 + 1.7));
  let sr = 0.26 * (1.0 + blur * 1.4) * (1.0 + 0.03 * sin(t * 0.23));
  let sd = (w - sun_pos) / sr;
  col = col + vec3<f32>(1.00, 0.97, 0.88) * exp(-dot(sd, sd)) * (0.58 + u.energy * 0.34);
  // Aqua counter-glow low right
  let gd = (w - vec2<f32>(0.86, 0.80)) / (0.45 * (1.0 + blur));
  col = col + vec3<f32>(0.28, 0.82, 0.72) * exp(-dot(gd, gd)) * (0.14 + u.energy * 0.14);
  // Cumulus with volume: warped-fbm coverage, self-shaded by comparing
  // density toward the sun — lit crowns, shaded bases. Blur flattens it.
  let cp = w * vec2<f32>(2.5, 3.4);
  let d = cloud_density(cp, u.phase);
  let d_lit = cloud_density(cp + vec2<f32>(-0.09, -0.11), u.phase);
  let cov = smoothstep(0.40 + u.brightness * 0.06, 0.86, d) * (1.0 - blur * 0.55);
  let lit = clamp(0.78 + (d - d_lit) * 2.2, 0.58, 1.06);
  let cloud_col = vec3<f32>(0.86, 0.90, 0.96) * lit;
  col = mix(col, cloud_col, cov * 0.75);
  // High cirrus streaks — the crisp detail the frost erases entirely
  let cir = smoothstep(0.52, 0.92, fbm(w * vec2<f32>(5.5, 11.0) + vec2<f32>(t * 0.021, 7.7)));
  col = mix(col, vec3<f32>(1.0, 1.0, 1.0), cir * 0.06 * (1.0 - blur));
  // Bright haze band settling on the horizon
  col = col + vec3<f32>(0.90, 0.96, 1.00) * exp(-(y - 0.90) * (y - 0.90) * 55.0) * (0.05 + (1.0 - u.brightness) * 0.09);
  // Green horizon glow
  col = col + vec3<f32>(0.25, 0.52, 0.22) * 0.12 * smoothstep(0.84, 1.0, y);
  // Aero bubbles drifting upward; size sets speed, so depth reads as
  // parallax. The frost diffuses whichever drift behind a pane.
  col = col + bubble(w, vec2<f32>(0.70 + 0.030 * sin(t * 0.10), fract(0.90 - t * 0.0060)), 0.058, t, blur);
  col = col + bubble(w, vec2<f32>(0.34 + 0.020 * sin(t * 0.13 + 2.0), fract(0.55 - t * 0.0034)), 0.034, t, blur);
  col = col + bubble(w, vec2<f32>(0.91 + 0.020 * sin(t * 0.08 + 4.0), fract(0.30 - t * 0.0050)), 0.046, t, blur);
  col = col + bubble(w, vec2<f32>(0.12 + 0.015 * sin(t * 0.11 + 1.1), fract(0.72 - t * 0.0026)), 0.024, t, blur);
  col = col + bubble(w, vec2<f32>(0.55 + 0.025 * sin(t * 0.09 + 5.3), fract(0.40 - t * 0.0019)), 0.017, t, blur);
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
    col = col + (hash2(p) - 0.5) * 0.016;
    return vec4<f32>(col, 1.0);
  }

  // -------------------------------------------------------------------
  // Physically-based glass slab. No painted highlights: everything below
  // is Snell refraction, Fresnel-Schlick reflectance, Beer-Lambert
  // absorption, micro-facet scattering, and a Blinn sun glint — all lit
  // by the SAME sun that renders in the sky.
  // -------------------------------------------------------------------
  let pad = 22.0;
  let inner = u.rect_size - vec2<f32>(pad * 2.0, pad * 2.0);
  let lp = in.uv * u.rect_size - vec2<f32>(pad, pad);
  let sdf = panel_sdf(lp, inner, u.corner);

  // Two-lobe penumbra: contact-sharp plus distance-soft, like an area light
  let dsh = max(sdf, 0.0);
  let sh = (0.20 * exp(-dsh * 0.30) + 0.11 * exp(-dsh * 0.055)) * step(0.0, sdf);

  // Slab geometry: flat interior, beveled rim. Normal from the SDF gradient.
  let eps = 1.5;
  let gx = panel_sdf(lp + vec2<f32>(eps, 0.0), inner, u.corner)
         - panel_sdf(lp - vec2<f32>(eps, 0.0), inner, u.corner);
  let gy = panel_sdf(lp + vec2<f32>(0.0, eps), inner, u.corner)
         - panel_sdf(lp - vec2<f32>(0.0, eps), inner, u.corner);
  let bevel = 9.0;
  let rim = clamp(1.0 + sdf / bevel, 0.0, 1.0); // 1 at the edge, 0 inside
  let tilt = rim * rim * 0.85;
  let n = normalize(vec3<f32>(vec2<f32>(gx, gy) / (2.0 * eps) * tilt, 1.0));

  // Snell: refract the straight-on view ray through IOR 1.52
  let rdir = refract(vec3<f32>(0.0, 0.0, -1.0), n, 1.0 / 1.52);
  let gap = 16.0;
  let thick = mix(0.35, 1.0, 1.0 - rim);
  let ofs = rdir.xy / max(-rdir.z, 0.2) * gap * thick;

  // Frost is micro-facet scattering: jittered refracted samples of the
  // analytically blurred field (Gaussians compose, so the blur is exact)
  let j = vec2<f32>(
    vnoise(p * 0.85 + vec2<f32>(13.7, 0.0)) - 0.5,
    vnoise(p * 0.85 + vec2<f32>(0.0, 57.3)) - 0.5
  ) * 7.0;
  let c0 = sky((p + ofs) / u.screen, u.time, 1.0);
  let c1 = sky((p + ofs + j) / u.screen, u.time, 1.0);
  var col = c0 * 0.6 + c1 * 0.4;

  // Beer-Lambert: extinction along the refracted path (cool-tinted glass);
  // longer oblique paths through the bevel tint denser
  let path = (0.35 + thick) / max(-rdir.z, 0.35);
  col = col * exp(-vec3<f32>(0.050, 0.020, 0.009) * path);

  // Fresnel-Schlick, n = 1.52: the grazing rim mirrors the upper sky
  let cosv = clamp(n.z, 0.0, 1.0);
  let fres = 0.042 + 0.958 * pow(1.0 - cosv, 5.0);
  let refl = sky(vec2<f32>((p.x + ofs.x) / u.screen.x, w.y * 0.22), u.time, 0.6);
  col = mix(col, refl * 1.12, clamp(fres, 0.0, 0.85));

  // Blinn sun glint off the slab — the same sun the sky renders
  let sun = normalize(vec3<f32>(-0.45, -0.55, 0.70));
  let hlf = normalize(sun + vec3<f32>(0.0, 0.0, 1.0));
  let spec = pow(clamp(dot(n, hlf), 0.0, 1.0), 170.0);
  col = col + vec3<f32>(1.0, 0.97, 0.88) * spec * (0.10 + fres * 6.0);

  // Diffuse scattering lift — the milkiness of etched glass
  col = mix(col, vec3<f32>(1.0, 1.0, 1.0), 0.35);
  col = col + (hash2(p) - 0.5) * 0.012;

  // Contact occlusion where the pane seats into its shadow
  let fy = lp.y / max(inner.y, 1.0);
  col = col - clamp((fy - 0.88) / 0.12, 0.0, 1.0) * 0.05;

  let cover = clamp(0.5 - sdf, 0.0, 1.0);
  let alpha = clamp(cover + sh, 0.0, 1.0);
  return vec4<f32>(col * cover, alpha);
}
"#;
