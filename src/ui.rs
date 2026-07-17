use eframe::egui::{
    self, pos2, vec2, Align2, Color32, ColorImage, CursorIcon, FontId, Key, Pos2, Rect, RichText,
    Rounding, Sense, Shape, Stroke, TextureHandle, TextureOptions, Vec2,
};
use eframe::egui::epaint::Mesh;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;

use crate::chorus::ChorusMode;
use crate::oscillator::Waveform;
use crate::voice_manager::VoiceManager;

const OCTAVES: usize = 3;
const WHITE_KEY_INDICES: [usize; 7] = [0, 2, 4, 5, 7, 9, 11];
const BLACK_KEY_INDICES: [usize; 5] = [1, 3, 6, 8, 10];

// ---------------------------------------------------------------------------
// Design system. A charcoal ramp, hairline strokes, and exactly two hues:
// amber for anything you touch (pointers, arcs, lit keys), phosphor cyan for
// the signal itself (the scope). Depth comes from layering and a single
// top-light model — never from texture.
// ---------------------------------------------------------------------------
const BG0: Color32 = Color32::from_rgb(0x0a, 0x0b, 0x0d); // window
const BG2: Color32 = Color32::from_rgb(0x1a, 0x1c, 0x22); // controls
const BG2_HOVER: Color32 = Color32::from_rgb(0x22, 0x25, 0x2c);
const INSET: Color32 = Color32::from_rgb(0x06, 0x07, 0x09); // display wells

const HAIRLINE: Color32 = Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 12);
const HAIRLINE_HI: Color32 = Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 24);

const TXT: Color32 = Color32::from_rgb(0xec, 0xe9, 0xe2);
const TXT_MID: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xa8);
const TXT_LOW: Color32 = Color32::from_rgb(0x5c, 0x61, 0x69);

const AMBER: Color32 = Color32::from_rgb(0xe0, 0xa1, 0x54);
const AMBER_HI: Color32 = Color32::from_rgb(0xff, 0xc0, 0x69);
const AMBER_DEEP: Color32 = Color32::from_rgb(0xc2, 0x86, 0x41);
const AMBER_INK: Color32 = Color32::from_rgb(0x54, 0x3a, 0x14);

const CYAN: Color32 = Color32::from_rgb(0x6f, 0xe3, 0xf2);

const CYAN_BRIGHT: Color32 = Color32::from_rgb(0xb5, 0xef, 0xf8);

const IVORY: Color32 = Color32::from_rgb(0xea, 0xe6, 0xdb);
const IVORY_SHADE: Color32 = Color32::from_rgb(0xd6, 0xd0, 0xc1);
const EBONY: Color32 = Color32::from_rgb(0x15, 0x16, 0x1a);
const EBONY_EDGE: Color32 = Color32::from_rgb(0x2c, 0x2f, 0x36);

// ---------------------------------------------------------------------------
// Per-pixel material synthesis — realism without image assets. An fBm
// walnut with growth rings, a sphere-shaded gloss knob sprite, and a soft
// aurora backdrop whose low-frequency light lets the translucent panels
// above it read as frosted glass.
// ---------------------------------------------------------------------------

struct Textures {
    backdrop: TextureHandle,
    wood: TextureHandle,
    knob: TextureHandle,
}

/// The knob sprite's texture id (+1, 0 = unset), so the knob widget can use
/// it without threading a handle through every call site.
static KNOB_TEX_ID: AtomicU64 = AtomicU64::new(0);

fn vhash(ix: i32, iy: i32, seed: u32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(374_761_393)
        ^ (iy as u32).wrapping_mul(668_265_263)
        ^ seed.wrapping_mul(2_654_435_761);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}

fn vnoise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let a = vhash(ix, iy, seed);
    let b = vhash(ix + 1, iy, seed);
    let c = vhash(ix, iy + 1, seed);
    let d = vhash(ix + 1, iy + 1, seed);
    a + (b - a) * sx + (c - a) * sy + (a - b - c + d) * sx * sy
}

fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut amp, mut freq, mut sum, mut norm) = (0.5, 1.0, 0.0, 0.0);
    for i in 0..octaves {
        sum += vnoise(x * freq, y * freq, seed + i) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

fn px(r: f32, g: f32, b: f32) -> Color32 {
    Color32::from_rgb(
        (r.clamp(0.0, 1.0) * 255.0) as u8,
        (g.clamp(0.0, 1.0) * 255.0) as u8,
        (b.clamp(0.0, 1.0) * 255.0) as u8,
    )
}

/// Deep teal-navy field with a few soft aurora glows.
fn make_backdrop() -> ColorImage {
    let (w, h) = (640usize, 400usize);
    let mut pixels = Vec::with_capacity(w * h);
    // (x, y, radius, r, g, b, strength)
    let blobs: [(f32, f32, f32, f32, f32, f32, f32); 4] = [
        (0.16, 0.18, 0.55, 0.28, 0.75, 0.85, 0.10), // aqua, upper left
        (0.88, 0.10, 0.50, 0.35, 0.80, 0.65, 0.06), // sea green, upper right
        (0.62, 0.95, 0.65, 0.90, 0.62, 0.30, 0.07), // warm amber, low center
        (0.30, 0.78, 0.55, 0.25, 0.55, 0.75, 0.06), // deep teal, low left
    ];
    for y in 0..h {
        let fy = y as f32 / h as f32;
        for x in 0..w {
            let fx = x as f32 / w as f32;
            let base = 0.055 - fy * 0.020;
            let (mut r, mut g, mut b) = (base * 0.75, base * 1.00, base * 1.20);
            for (bx, by, rad, br, bg, bb, strength) in blobs {
                let dx = (fx - bx) / rad;
                let dy = (fy - by) / rad;
                let fall = (-(dx * dx + dy * dy) * 2.2).exp() * strength;
                r += br * fall;
                g += bg * fall;
                b += bb * fall;
            }
            // Dither breaks up gradient banding
            let d = (vhash(x as i32, y as i32, 3) - 0.5) * 0.008;
            pixels.push(px(r + d, g + d, b + d));
        }
    }
    ColorImage { size: [w, h], pixels }
}

/// Aged walnut: growth rings warped by fBm, fine along-grain streaks and
/// pores, a satin sheen at the top edge and shade at the base.
fn make_wood() -> ColorImage {
    let (w, h) = (1024usize, 128usize);
    let mut pixels = Vec::with_capacity(w * h);
    let dark = (0.115, 0.068, 0.038);
    let light = (0.400, 0.265, 0.148);
    for y in 0..h {
        let fy = y as f32 / h as f32;
        for x in 0..w {
            let (xf, yf) = (x as f32, y as f32);
            let warp = fbm(xf * 0.006, yf * 0.030, 4, 11);
            let t = xf * 0.011 + warp * 6.5;
            let ring = ((t * std::f32::consts::TAU * 0.75).sin() * 0.5 + 0.5).powf(1.7);
            let streak = fbm(xf * 0.045, yf * 0.90, 3, 47);
            let pores = fbm(xf * 0.50, yf * 0.18, 2, 83);
            let shade = 0.52 * ring + 0.26 * streak + 0.14 * warp + 0.08 * pores;
            let mut r = dark.0 + (light.0 - dark.0) * shade;
            let mut g = dark.1 + (light.1 - dark.1) * shade;
            let mut b = dark.2 + (light.2 - dark.2) * shade;
            let finish = 1.10 - 0.28 * fy;
            r *= finish;
            g *= finish;
            b *= finish;
            if fy < 0.018 {
                r = r * 0.6 + 0.28;
                g = g * 0.6 + 0.22;
                b = b * 0.6 + 0.15;
            }
            if fy > 0.965 {
                r *= 0.45;
                g *= 0.45;
                b *= 0.45;
            }
            pixels.push(px(r, g, b));
        }
    }
    ColorImage { size: [w, h], pixels }
}

/// Sphere-shaded knob cap: key light upper-left, tight Frutiger gloss lobe,
/// aqua bounce along the lower rim, fresnel edge — rendered once.
fn make_knob_sprite() -> ColorImage {
    let s = 128usize;
    let mut pixels = Vec::with_capacity(s * s);
    let c = s as f32 / 2.0;
    let radius = c - 2.0;
    let l = (-0.42f32, -0.62f32, 0.66f32);
    let llen = (l.0 * l.0 + l.1 * l.1 + l.2 * l.2).sqrt();
    let l = (l.0 / llen, l.1 / llen, l.2 / llen);
    for y in 0..s {
        for x in 0..s {
            let dx = (x as f32 - c) / radius;
            let dy = (y as f32 - c) / radius;
            let d2 = dx * dx + dy * dy;
            if d2 >= 1.0 {
                pixels.push(Color32::TRANSPARENT);
                continue;
            }
            let nz = (1.0 - d2).sqrt();
            let lambert = (dx * l.0 + dy * l.1 + nz * l.2).max(0.0);
            let angle = dy.atan2(dx);
            let brush = vnoise(angle * 14.0, d2.sqrt() * 3.0, 5) * 0.05;
            let base = 0.075 + lambert * 0.16 + brush;
            let (mut r, mut g, mut b) = (base * 0.96, base * 1.02, base * 1.12);
            let sx = dx + 0.34;
            let sy = dy + 0.44;
            let spec = (-(sx * sx + sy * sy) * 9.0).exp();
            r += spec * 0.34;
            g += spec * 0.37;
            b += spec * 0.40;
            let rim = (d2.sqrt() - 0.72).max(0.0) / 0.28;
            let below = (dy + 0.2).max(0.0);
            let bounce = rim * below * 0.14;
            g += bounce * 0.8;
            b += bounce;
            let edge = 1.0 - ((d2.sqrt() - 0.93).max(0.0) / 0.07) * 0.5;
            r *= edge;
            g *= edge;
            b *= edge;
            let alpha = ((1.0 - d2.sqrt()) * radius).clamp(0.0, 1.0);
            let cpx = px(r, g, b);
            pixels.push(Color32::from_rgba_unmultiplied(
                cpx.r(),
                cpx.g(),
                cpx.b(),
                (alpha * 255.0) as u8,
            ));
        }
    }
    ColorImage { size: [s, s], pixels }
}

/// A quad whose top and bottom edges carry different vertex colors — the
/// GPU interpolates, giving a smooth vertical gradient.
fn gradient_quad(rect: Rect, top: Color32, bottom: Color32) -> Shape {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(2, 1, 3);
    Shape::mesh(mesh)
}

/// Frosted glass panel: stacked soft shadow, translucent cool fill, a light
/// sweep across the top, bright inner edge.
fn glass_shapes(rect: Rect, rounding: f32) -> Vec<Shape> {
    let mut shapes = Vec::with_capacity(10);
    for (expand, alpha) in [(6.0, 26), (4.0, 40), (2.0, 60), (0.5, 80)] {
        shapes.push(Shape::rect_stroke(
            rect.expand(expand),
            Rounding::same(rounding + expand),
            Stroke::new(2.0, Color32::from_rgba_unmultiplied(0, 0, 0, alpha)),
        ));
    }
    shapes.push(Shape::rect_filled(
        rect,
        Rounding::same(rounding),
        Color32::from_rgba_unmultiplied(0xd2, 0xe6, 0xec, 14),
    ));
    let sweep = Rect::from_min_max(
        rect.min + vec2(rounding, 1.5),
        pos2(rect.right() - rounding, rect.top() + rect.height() * 0.42),
    );
    shapes.push(gradient_quad(
        sweep,
        Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 20),
        Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0),
    ));
    shapes.push(Shape::rect_stroke(
        rect,
        Rounding::same(rounding),
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 34)),
    ));
    shapes.push(Shape::line_segment(
        [
            pos2(rect.left() + rounding, rect.top() + 0.5),
            pos2(rect.right() - rounding, rect.top() + 0.5),
        ],
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 70)),
    ));
    shapes
}

/// Aero gloss for selected segmented cells: aqua glass with a lit top half.
fn gloss_fill(painter: &egui::Painter, rect: Rect, rounding: f32) {
    painter.rect_filled(
        rect,
        Rounding::same(rounding),
        Color32::from_rgba_unmultiplied(0x6f, 0xe3, 0xf2, 34),
    );
    let top = Rect::from_min_max(rect.min, pos2(rect.right(), rect.center().y));
    painter.add(gradient_quad(
        top.shrink(1.0),
        Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 46),
        Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 4),
    ));
    painter.rect_stroke(
        rect,
        Rounding::same(rounding),
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xa9, 0xe6, 0xf2, 120)),
    );
}

pub struct SynthUI {
    current_octave: i32,
    volume: f32,
    waveform: Waveform,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    filter_cutoff: f32,
    filter_resonance: f32,
    filter_drive: f32,
    filter_saturation: f32,
    hpf_cutoff: f32,
    fuzz: f32,
    noise: f32,
    spring: f32,
    detune: f32,
    fenv_amount: f32,
    fenv_attack: f32,
    fenv_decay: f32,
    fenv_sustain: f32,
    fenv_release: f32,
    active_mouse_note: Option<u8>,
    voice_manager: Arc<Mutex<VoiceManager>>,
    chorus_rate: f32,
    chorus_depth: f32,
    chorus_mode: ChorusMode,
    reverb_decay: f32,
    reverb_wet: f32,
    tape_wow: f32,
    tape_flutter: f32,
    tape_drive: f32,
    tape_age: f32,
    /// QWERTY keys currently sounding, mapped to the MIDI note each one
    /// started (so note-off always matches, even if the octave changed).
    pressed_keys: HashMap<Key, u8>,
    theme_applied: bool,
    notes_active: bool,
    textures: Option<Textures>,
}

// ---------------------------------------------------------------------------
// Type helpers
// ---------------------------------------------------------------------------

/// Letterspaced micro-caps, the panel-legend voice of the whole interface.
fn tracked(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for (i, c) in text.chars().enumerate() {
        if i > 0 {
            out.push('\u{2009}'); // thin space
        }
        out.push(c.to_ascii_uppercase());
    }
    out
}

fn legend(text: &str) -> RichText {
    RichText::new(tracked(text)).size(9.5).color(TXT_LOW)
}

fn fmt_hz(v: f32) -> String {
    if v >= 1000.0 {
        format!("{:.1} kHz", v / 1000.0)
    } else {
        format!("{:.0} Hz", v)
    }
}

fn fmt_time(v: f32) -> String {
    if v < 1.0 {
        format!("{:.0} ms", v * 1000.0)
    } else {
        format!("{:.2} s", v)
    }
}

fn fmt_pct(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}

fn fmt_x(v: f32) -> String {
    format!("{:.2}", v)
}

// ---------------------------------------------------------------------------
// Widgets
// ---------------------------------------------------------------------------

/// Rotary control in the language of hardware panel legends: etched tick
/// marks, a dome-shaded cap, a thin amber value arc (grown from 12 o'clock
/// for bipolar ranges), and a quiet tabular readout. Drag vertically or
/// scroll; Shift for fine control; double-click to reset.
fn knob(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    logarithmic: bool,
    fmt: impl Fn(f32) -> String,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(70.0, 102.0), Sense::click_and_drag());
    let mut changed = false;

    let to_t = |v: f32| -> f32 {
        if logarithmic {
            ((v / min).ln() / (max / min).ln()).clamp(0.0, 1.0)
        } else {
            ((v - min) / (max - min)).clamp(0.0, 1.0)
        }
    };
    let from_t = |t: f32| -> f32 {
        if logarithmic {
            min * (max / min).powf(t)
        } else {
            min + (max - min) * t
        }
    };

    if response.double_clicked() {
        *value = default;
        changed = true;
    } else if response.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let sensitivity = if fine { 0.0015 } else { 0.006 };
        let dy = response.drag_delta().y;
        if dy != 0.0 {
            *value = from_t((to_t(*value) - dy * sensitivity).clamp(0.0, 1.0));
            changed = true;
        }
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.scroll_delta.y);
        if scroll != 0.0 {
            let fine = ui.input(|i| i.modifiers.shift);
            let sensitivity = if fine { 0.0004 } else { 0.0015 };
            *value = from_t((to_t(*value) + scroll * sensitivity).clamp(0.0, 1.0));
            changed = true;
        }
    }

    let response = response.on_hover_cursor(CursorIcon::ResizeVertical);
    let engaged = response.hovered() || response.dragged();

    let painter = ui.painter();
    let center = pos2(rect.center().x, rect.top() + 50.0);
    let start = 135.0_f32.to_radians();
    let sweep = 270.0_f32.to_radians();
    let t = to_t(*value);

    painter.text(
        pos2(rect.center().x, rect.top() + 4.0),
        Align2::CENTER_TOP,
        tracked(label),
        FontId::proportional(9.0),
        if engaged { TXT_MID } else { TXT_LOW },
    );

    // Etched ticks, majors slightly longer
    for i in 0..=10 {
        let a = start + sweep * i as f32 / 10.0;
        let dir = vec2(a.cos(), a.sin());
        let (r0, r1) = if i % 5 == 0 { (23.0, 27.5) } else { (23.0, 25.5) };
        painter.line_segment(
            [center + dir * r0, center + dir * r1],
            Stroke::new(1.0, HAIRLINE_HI),
        );
    }

    // Value arc — from 12 o'clock for bipolar ranges, from min otherwise
    let arc_r = 20.0;
    let t_origin = if min < 0.0 { to_t(0.0) } else { 0.0 };
    let (a0, a1) = (t_origin.min(t), t_origin.max(t));
    if a1 - a0 > 0.004 {
        let n = 1.max(((a1 - a0) * 48.0) as usize);
        let points: Vec<Pos2> = (0..=n)
            .map(|i| {
                let a = start + sweep * (a0 + (a1 - a0) * i as f32 / n as f32);
                center + vec2(a.cos(), a.sin()) * arc_r
            })
            .collect();
        painter.add(Shape::line(
            points,
            Stroke::new(2.5, if engaged { AMBER_HI } else { AMBER }),
        ));
    }
    // Arc endpoint
    let end_angle = start + sweep * t;
    painter.circle_filled(
        center + vec2(end_angle.cos(), end_angle.sin()) * arc_r,
        2.0,
        if engaged { AMBER_HI } else { AMBER },
    );

    // Cap: soft under-shadow, then the sphere-shaded gloss sprite
    painter.circle_filled(
        center + vec2(0.0, 1.6),
        15.8,
        Color32::from_rgba_unmultiplied(0, 0, 0, 100),
    );
    let knob_tex = KNOB_TEX_ID.load(AtomicOrdering::Relaxed);
    if knob_tex > 0 {
        let cap = Rect::from_center_size(center, Vec2::splat(31.0));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let tex_id = egui::TextureId::Managed(knob_tex - 1);
        painter.image(tex_id, cap, uv, Color32::WHITE);
        if engaged {
            // Second translucent pass lifts the whole cap
            painter.image(tex_id, cap, uv, Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 70));
        }
    } else {
        painter.circle_filled(center, 15.0, Color32::from_rgb(0x1a, 0x1c, 0x21));
    }
    if engaged {
        painter.circle_stroke(
            center,
            17.0,
            Stroke::new(1.5, Color32::from_rgba_unmultiplied(0x6f, 0xe3, 0xf2, 90)),
        );
    }

    // Pointer — painted indicator line, like an engraved knob skirt
    let dir = vec2(end_angle.cos(), end_angle.sin());
    painter.line_segment(
        [center + dir * 5.0, center + dir * 13.5],
        Stroke::new(2.0, TXT),
    );

    painter.text(
        pos2(rect.center().x, rect.bottom() - 3.0),
        Align2::CENTER_BOTTOM,
        fmt(*value),
        FontId::monospace(10.0),
        if engaged { AMBER_HI } else { TXT_LOW },
    );

    changed
}

fn wave_glyph(painter: &egui::Painter, rect: Rect, waveform: Waveform, color: Color32) {
    let inner = rect.shrink2(vec2(9.0, 9.0));
    let (l, r, top, bot, mid) = (
        inner.left(),
        inner.right(),
        inner.top(),
        inner.bottom(),
        inner.center().y,
    );
    let points: Vec<Pos2> = match waveform {
        Waveform::Sine => (0..=24)
            .map(|i| {
                let x = i as f32 / 24.0;
                pos2(
                    l + x * inner.width(),
                    mid - (x * std::f32::consts::TAU).sin() * inner.height() * 0.5,
                )
            })
            .collect(),
        Waveform::Square => vec![
            pos2(l, bot),
            pos2(l, top),
            pos2(inner.center().x, top),
            pos2(inner.center().x, bot),
            pos2(r, bot),
            pos2(r, top),
        ],
        Waveform::Sawtooth => vec![
            pos2(l, bot),
            pos2(inner.center().x, top),
            pos2(inner.center().x, bot),
            pos2(r, top),
        ],
        Waveform::Triangle => vec![
            pos2(l, mid),
            pos2(l + inner.width() * 0.25, top),
            pos2(l + inner.width() * 0.75, bot),
            pos2(r, mid),
        ],
    };
    painter.add(Shape::line(points, Stroke::new(1.6, color)));
}

/// Segmented waveform selector: one hairline container, four cells.
fn waveform_selector(ui: &mut egui::Ui, selected: &mut Waveform) -> bool {
    const OPTIONS: [Waveform; 4] = [
        Waveform::Sine,
        Waveform::Triangle,
        Waveform::Sawtooth,
        Waveform::Square,
    ];
    let cell = vec2(40.0, 30.0);
    let (rect, _) = ui.allocate_exact_size(
        vec2(cell.x * OPTIONS.len() as f32, cell.y),
        Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::same(7.0), INSET);

    let mut changed = false;
    for (i, wf) in OPTIONS.iter().enumerate() {
        let cell_rect = Rect::from_min_size(
            pos2(rect.left() + cell.x * i as f32, rect.top()),
            cell,
        );
        let response = ui.interact(
            cell_rect,
            ui.id().with(("wave", i)),
            Sense::click(),
        );
        let is_selected = *selected == *wf;
        if response.clicked() && !is_selected {
            *selected = *wf;
            changed = true;
        }
        if is_selected {
            gloss_fill(&painter, cell_rect.shrink(2.0), 5.0);
        }
        let color = if is_selected {
            CYAN_BRIGHT
        } else if response.hovered() {
            TXT_MID
        } else {
            TXT_LOW
        };
        wave_glyph(&painter, cell_rect, *wf, color);
        if i > 0 && !is_selected {
            painter.line_segment(
                [
                    pos2(cell_rect.left(), cell_rect.top() + 7.0),
                    pos2(cell_rect.left(), cell_rect.bottom() - 7.0),
                ],
                Stroke::new(1.0, HAIRLINE),
            );
        }
    }
    painter.rect_stroke(rect, Rounding::same(7.0), Stroke::new(1.0, HAIRLINE));
    changed
}

/// Segmented text selector; returns the newly selected index if it changed.
fn segmented(ui: &mut egui::Ui, id: &str, labels: &[&str], selected: usize) -> Option<usize> {
    let cell = vec2(34.0, 24.0);
    let (rect, _) = ui.allocate_exact_size(
        vec2(cell.x * labels.len() as f32, cell.y),
        Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, Rounding::same(7.0), INSET);

    let mut result = None;
    for (i, label) in labels.iter().enumerate() {
        let cell_rect = Rect::from_min_size(
            pos2(rect.left() + cell.x * i as f32, rect.top()),
            cell,
        );
        let response = ui.interact(cell_rect, ui.id().with((id, i)), Sense::click());
        let is_selected = i == selected;
        if response.clicked() && !is_selected {
            result = Some(i);
        }
        if is_selected {
            gloss_fill(&painter, cell_rect.shrink(2.0), 5.0);
        }
        let color = if is_selected {
            CYAN_BRIGHT
        } else if response.hovered() {
            TXT_MID
        } else {
            TXT_LOW
        };
        painter.text(
            cell_rect.center(),
            Align2::CENTER_CENTER,
            *label,
            FontId::proportional(10.5),
            color,
        );
        if i > 0 && !is_selected {
            painter.line_segment(
                [
                    pos2(cell_rect.left(), cell_rect.top() + 6.0),
                    pos2(cell_rect.left(), cell_rect.bottom() - 6.0),
                ],
                Stroke::new(1.0, HAIRLINE),
            );
        }
    }
    painter.rect_stroke(rect, Rounding::same(7.0), Stroke::new(1.0, HAIRLINE));
    result
}

/// A quiet square button for the octave stepper.
fn step_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
    let painter = ui.painter();
    let fill = if response.is_pointer_button_down_on() {
        INSET
    } else if response.hovered() {
        BG2_HOVER
    } else {
        BG2
    };
    painter.rect_filled(rect, Rounding::same(6.0), fill);
    painter.rect_stroke(rect, Rounding::same(6.0), Stroke::new(1.0, HAIRLINE));
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(13.0),
        if response.hovered() { TXT } else { TXT_MID },
    );
    response
}

/// Card: one surface, hairline edge, a lit top edge, and a legend header
/// with a rule. The background paints after layout via a placeholder so it
/// sits under the contents.
fn card<R>(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui) -> R) {
    let bg_idx = ui.painter().add(Shape::Noop);
    let inner = egui::Frame::none()
        .inner_margin(egui::style::Margin {
            left: 16.0,
            right: 16.0,
            top: 12.0,
            bottom: 14.0,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(legend(title));
                let (rule, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
                ui.painter().line_segment(
                    [
                        pos2(rule.left() + 4.0, rule.center().y),
                        pos2(rule.right(), rule.center().y),
                    ],
                    Stroke::new(1.0, HAIRLINE),
                );
            });
            ui.add_space(8.0);
            add_contents(ui);
        });
    let rect = inner.response.rect;
    ui.painter().set(bg_idx, Shape::Vec(glass_shapes(rect, 12.0)));
}

/// Vertical hairline between subgroups inside a card.
fn vseparator(ui: &mut egui::Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(17.0, height), Sense::hover());
    ui.painter().line_segment(
        [
            pos2(rect.center().x, rect.top() + 2.0),
            pos2(rect.center().x, rect.bottom() - 2.0),
        ],
        Stroke::new(1.0, HAIRLINE),
    );
}

impl SynthUI {
    pub fn new(voice_manager: Arc<Mutex<VoiceManager>>) -> Self {
        let ui = Self {
            voice_manager,
            current_octave: 4,
            volume: 0.5,
            waveform: Waveform::Sawtooth,
            attack: 0.1,
            decay: 0.1,
            sustain: 0.7,
            release: 0.2,
            filter_cutoff: 15000.0,
            filter_resonance: 0.0,
            filter_drive: 1.0,
            filter_saturation: 1.0,
            hpf_cutoff: 16.0,
            fuzz: 0.0,
            noise: 0.0,
            spring: 0.0,
            detune: 7.0,
            fenv_amount: 0.0,
            fenv_attack: 0.005,
            fenv_decay: 0.3,
            fenv_sustain: 0.0,
            fenv_release: 0.3,
            active_mouse_note: None,
            chorus_rate: 0.5,
            chorus_depth: 0.3,
            chorus_mode: ChorusMode::Off,
            reverb_decay: 0.5,
            reverb_wet: 0.5,
            tape_wow: 0.0,
            tape_flutter: 0.0,
            tape_drive: 0.0,
            tape_age: 0.0,
            pressed_keys: HashMap::new(),
            theme_applied: false,
            notes_active: false,
            textures: None,
        };
        // Push the UI defaults into the engine so what you see is what you hear
        ui.apply_all_settings();
        ui
    }

    fn apply_all_settings(&self) {
        let mut vm = self.voice_manager.lock();
        vm.set_waveform(self.waveform);
        vm.set_volume(self.volume);
        vm.set_detune(self.detune);
        vm.set_attack(self.attack);
        vm.set_decay(self.decay);
        vm.set_sustain(self.sustain);
        vm.set_release(self.release);
        vm.set_filter_env_amount(self.fenv_amount);
        vm.set_filter_attack(self.fenv_attack);
        vm.set_filter_decay(self.fenv_decay);
        vm.set_filter_sustain(self.fenv_sustain);
        vm.set_filter_release(self.fenv_release);
        vm.set_filter_cutoff(self.filter_cutoff);
        vm.set_filter_resonance(self.filter_resonance);
        vm.set_filter_drive(self.filter_drive);
        vm.set_filter_saturation(self.filter_saturation);
        vm.set_hpf_cutoff(self.hpf_cutoff);
        vm.set_fuzz(self.fuzz);
        vm.set_noise(self.noise);
        vm.set_spring(self.spring);
        vm.set_reverb_decay(self.reverb_decay);
        vm.set_reverb_wet(self.reverb_wet);
        vm.set_chorus_mode(self.chorus_mode);
        vm.set_chorus_rate(self.chorus_rate);
        vm.set_chorus_depth(self.chorus_depth);
        vm.set_tape_wow(self.tape_wow);
        vm.set_tape_flutter(self.tape_flutter);
        vm.set_tape_drive(self.tape_drive);
        vm.set_tape_age(self.tape_age);
    }

    fn apply_theme(ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = vec2(10.0, 10.0);
        style.spacing.button_padding = vec2(10.0, 4.0);

        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = Color32::TRANSPARENT;
        v.window_fill = BG0;
        v.override_text_color = Some(TXT);
        v.widgets.inactive.bg_fill = BG2;
        v.widgets.hovered.bg_fill = BG2_HOVER;
        v.widgets.active.bg_fill = INSET;
        v.widgets.inactive.rounding = Rounding::same(6.0);
        v.widgets.hovered.rounding = Rounding::same(6.0);
        v.widgets.active.rounding = Rounding::same(6.0);
        v.widgets.open.rounding = Rounding::same(6.0);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, TXT_MID);
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, TXT);
        v.widgets.active.fg_stroke = Stroke::new(1.0, AMBER);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, HAIRLINE);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, HAIRLINE_HI);
        v.selection.bg_fill = Color32::from_rgb(0x33, 0x2a, 0x1a);
        v.selection.stroke = Stroke::new(1.0, AMBER);

        ctx.set_style(style);
    }

    fn ensure_textures(&mut self, ctx: &egui::Context) {
        if self.textures.is_none() {
            let textures = Textures {
                backdrop: ctx.load_texture("patina-backdrop", make_backdrop(), TextureOptions::LINEAR),
                wood: ctx.load_texture("patina-wood", make_wood(), TextureOptions::LINEAR),
                knob: ctx.load_texture("patina-knob", make_knob_sprite(), TextureOptions::LINEAR),
            };
            if let egui::TextureId::Managed(id) = textures.knob.id() {
                KNOB_TEX_ID.store(id + 1, AtomicOrdering::Relaxed);
            }
            self.textures = Some(textures);
        }
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        if !self.theme_applied {
            Self::apply_theme(ctx);
            self.theme_applied = true;
        }
        self.ensure_textures(ctx);

        // Pull the engine's canonical parameter values so the controls follow
        // song automation (and any other source) live
        {
            let vm = self.voice_manager.lock();
            let p = vm.params;
            self.volume = p.volume;
            self.waveform = p.waveform;
            self.attack = p.attack;
            self.decay = p.decay;
            self.sustain = p.sustain;
            self.release = p.release;
            self.filter_cutoff = p.cutoff;
            self.filter_resonance = p.resonance;
            self.filter_drive = p.drive;
            self.filter_saturation = p.saturation;
            self.hpf_cutoff = p.hpf_cutoff;
            self.fuzz = p.fuzz;
            self.noise = p.noise;
            self.spring = p.spring;
            self.detune = p.detune;
            self.fenv_amount = p.filter_env_amount;
            self.fenv_attack = p.filter_attack;
            self.fenv_decay = p.filter_decay;
            self.fenv_sustain = p.filter_sustain;
            self.fenv_release = p.filter_release;
            self.reverb_decay = p.reverb_decay;
            self.reverb_wet = p.reverb_wet;
            self.chorus_mode = p.chorus_mode;
            self.chorus_rate = p.chorus_rate;
            self.chorus_depth = p.chorus_depth;
            self.tape_wow = p.tape_wow;
            self.tape_flutter = p.tape_flutter;
            self.tape_drive = p.tape_drive;
            self.tape_age = p.tape_age;
            self.notes_active = vm.held_note_states().iter().any(|&held| held);
        }

        // Keep repainting so key lights and the scope stay live
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        // Aurora backdrop, painted on the panels' shared layer before they
        // run so it sits under their transparent frames
        if let Some(tex) = &self.textures {
            ctx.layer_painter(egui::LayerId::background()).image(
                tex.backdrop.id(),
                ctx.screen_rect(),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        self.handle_keyboard_input(ctx);

        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::none()
                    .inner_margin(egui::style::Margin::symmetric(20.0, 12.0)),
            )
            .show(ctx, |ui| self.draw_header(ui));

        egui::TopBottomPanel::bottom("keyboard")
            .frame(egui::Frame::none().inner_margin(egui::style::Margin {
                left: 20.0,
                right: 20.0,
                top: 9.0,
                bottom: 10.0,
            }))
            .show(ctx, |ui| {
                // Walnut shelf under the keys, painted after layout
                let bg_idx = ui.painter().add(Shape::Noop);
                self.draw_keyboard(ui);
                ui.add_space(7.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Z–M  ·  Q–U  play      ↑ ↓  octave      drag / scroll knobs  ·  ⇧ fine  ·  double-click reset")
                            .size(9.5)
                            .color(Color32::from_rgba_unmultiplied(0xf0, 0xe4, 0xcd, 150)),
                    );
                });
                let shelf = ui.min_rect().expand2(vec2(20.0, 9.5));
                if let Some(tex) = &self.textures {
                    let uv_w = (shelf.width() / 1024.0).clamp(0.35, 1.0);
                    ui.painter().set(
                        bg_idx,
                        Shape::Vec(vec![
                            Shape::image(
                                tex.wood.id(),
                                shelf,
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(uv_w, 1.0)),
                                Color32::WHITE,
                            ),
                            Shape::line_segment(
                                [shelf.left_top(), shelf.right_top()],
                                Stroke::new(1.5, Color32::from_rgba_unmultiplied(0, 0, 0, 150)),
                            ),
                        ]),
                    );
                }
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().inner_margin(egui::style::Margin {
                left: 20.0,
                right: 20.0,
                top: 8.0,
                bottom: 8.0,
            }))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
                ui.horizontal(|ui| {
                    self.draw_oscillator_card(ui);
                    self.draw_envelope_card(ui);
                    self.draw_filter_card(ui);
                });
                ui.horizontal(|ui| {
                    self.draw_filter_env_card(ui);
                    self.draw_effects_card(ui);
                });
                self.draw_scope(ui);
            });
    }

    /// Wordmark on the walnut top rail, octave stepper right.
    fn draw_header(&mut self, ui: &mut egui::Ui) {
        let bg_idx = ui.painter().add(Shape::Noop);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(tracked("Patina"))
                    .size(14.0)
                    .strong()
                    .color(TXT),
            );
            ui.add_space(2.0);
            ui.label(
                RichText::new(tracked("polyphonic synthesizer"))
                    .size(8.5)
                    .color(Color32::from_rgba_unmultiplied(0xf0, 0xe4, 0xcd, 130)),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if step_button(ui, "+").on_hover_text("Octave up  (↑)").clicked() {
                    self.shift_octave(1);
                }
                let (chip, _) = ui.allocate_exact_size(vec2(58.0, 24.0), Sense::hover());
                let painter = ui.painter();
                painter.rect_filled(chip, Rounding::same(6.0), INSET);
                painter.rect_stroke(chip, Rounding::same(6.0), Stroke::new(1.0, HAIRLINE));
                painter.text(
                    chip.center(),
                    Align2::CENTER_CENTER,
                    format!("OCT {}", self.current_octave),
                    FontId::monospace(10.5),
                    CYAN,
                );
                if step_button(ui, "−").on_hover_text("Octave down  (↓)").clicked() {
                    self.shift_octave(-1);
                }
            });
        });
        ui.add_space(10.0);
        let rail = ui.min_rect().expand2(vec2(20.0, 12.0));
        if let Some(tex) = &self.textures {
            let uv_w = (rail.width() / 1024.0).clamp(0.35, 1.0);
            ui.painter().set(
                bg_idx,
                Shape::Vec(vec![
                    Shape::image(
                        tex.wood.id(),
                        rail,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(uv_w, 1.0)),
                        Color32::WHITE,
                    ),
                    Shape::line_segment(
                        [rail.left_bottom(), rail.right_bottom()],
                        Stroke::new(1.5, Color32::from_rgba_unmultiplied(0, 0, 0, 160)),
                    ),
                ]),
            );
        }
    }

    fn draw_oscillator_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "Oscillator", |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.add_space(10.0);
                    if waveform_selector(ui, &mut self.waveform) {
                        self.voice_manager.lock().set_waveform(self.waveform);
                    }
                });
                if knob(ui, "Level", &mut self.volume, 0.0, 1.0, 0.5, false, fmt_pct) {
                    self.voice_manager.lock().set_volume(self.volume);
                }
                if knob(ui, "Detune", &mut self.detune, 0.0, 30.0, 7.0, false, |v| {
                    format!("{:.0} ct", v)
                }) {
                    self.voice_manager.lock().set_detune(self.detune);
                }
                if knob(ui, "Noise", &mut self.noise, 0.0, 1.0, 0.0, false, fmt_pct) {
                    self.voice_manager.lock().set_noise(self.noise);
                }
            });
        });
    }

    fn draw_envelope_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "Envelope", |ui| {
            ui.horizontal(|ui| {
                if knob(ui, "Attack", &mut self.attack, 0.01, 2.0, 0.1, true, fmt_time) {
                    self.voice_manager.lock().set_attack(self.attack);
                }
                if knob(ui, "Decay", &mut self.decay, 0.01, 2.0, 0.1, true, fmt_time) {
                    self.voice_manager.lock().set_decay(self.decay);
                }
                if knob(ui, "Sustain", &mut self.sustain, 0.0, 1.0, 0.7, false, fmt_pct) {
                    self.voice_manager.lock().set_sustain(self.sustain);
                }
                if knob(ui, "Release", &mut self.release, 0.01, 2.0, 0.2, true, fmt_time) {
                    self.voice_manager.lock().set_release(self.release);
                }
            });
        });
    }

    fn draw_filter_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "Filter", |ui| {
            ui.horizontal(|ui| {
                if knob(ui, "Cutoff", &mut self.filter_cutoff, 20.0, 20000.0, 15000.0, true, fmt_hz) {
                    self.voice_manager.lock().set_filter_cutoff(self.filter_cutoff);
                }
                if knob(ui, "Reso", &mut self.filter_resonance, 0.0, 4.0, 0.0, false, fmt_x) {
                    self.voice_manager.lock().set_filter_resonance(self.filter_resonance);
                }
                if knob(ui, "Drive", &mut self.filter_drive, 0.1, 5.0, 1.0, false, fmt_x) {
                    self.voice_manager.lock().set_filter_drive(self.filter_drive);
                }
                if knob(ui, "Shape", &mut self.filter_saturation, 0.0, 2.0, 1.0, false, fmt_x) {
                    self.voice_manager.lock().set_filter_saturation(self.filter_saturation);
                }
                if knob(ui, "Hi-Pass", &mut self.hpf_cutoff, 16.0, 8000.0, 16.0, true, fmt_hz) {
                    self.voice_manager.lock().set_hpf_cutoff(self.hpf_cutoff);
                }
            });
        });
    }

    fn draw_filter_env_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "Filter Envelope", |ui| {
            ui.horizontal(|ui| {
                if knob(ui, "Amount", &mut self.fenv_amount, -5.0, 5.0, 0.0, false, |v| {
                    format!("{:+.1} oct", v)
                }) {
                    self.voice_manager.lock().set_filter_env_amount(self.fenv_amount);
                }
                if knob(ui, "Attack", &mut self.fenv_attack, 0.001, 2.0, 0.005, true, fmt_time) {
                    self.voice_manager.lock().set_filter_attack(self.fenv_attack);
                }
                if knob(ui, "Decay", &mut self.fenv_decay, 0.01, 2.0, 0.3, true, fmt_time) {
                    self.voice_manager.lock().set_filter_decay(self.fenv_decay);
                }
                if knob(ui, "Sustain", &mut self.fenv_sustain, 0.0, 1.0, 0.0, false, fmt_pct) {
                    self.voice_manager.lock().set_filter_sustain(self.fenv_sustain);
                }
                if knob(ui, "Release", &mut self.fenv_release, 0.01, 2.0, 0.3, true, fmt_time) {
                    self.voice_manager.lock().set_filter_release(self.fenv_release);
                }
            });
        });
    }

    fn draw_effects_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "Effects", |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(legend("Chorus"));
                    ui.add_space(4.0);
                    let modes = ["OFF", "I", "II", "III", "IV"];
                    let selected = match self.chorus_mode {
                        ChorusMode::Off => 0,
                        ChorusMode::I => 1,
                        ChorusMode::II => 2,
                        ChorusMode::III => 3,
                        ChorusMode::IV => 4,
                    };
                    if let Some(i) = segmented(ui, "chorus", &modes, selected) {
                        self.chorus_mode = [
                            ChorusMode::Off,
                            ChorusMode::I,
                            ChorusMode::II,
                            ChorusMode::III,
                            ChorusMode::IV,
                        ][i];
                        self.voice_manager.lock().set_chorus_mode(self.chorus_mode);
                    }
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        if knob(ui, "Rate", &mut self.chorus_rate, 0.1, 10.0, 0.5, true, |v| {
                            format!("{:.1} Hz", v)
                        }) {
                            self.voice_manager.lock().set_chorus_rate(self.chorus_rate);
                        }
                        if knob(ui, "Depth", &mut self.chorus_depth, 0.0, 1.0, 0.3, false, fmt_pct) {
                            self.voice_manager.lock().set_chorus_depth(self.chorus_depth);
                        }
                    });
                });
                vseparator(ui, 150.0);
                ui.vertical(|ui| {
                    ui.label(legend("Reverb"));
                    ui.add_space(34.0);
                    ui.horizontal(|ui| {
                        if knob(ui, "Decay", &mut self.reverb_decay, 0.0, 0.99, 0.5, false, fmt_pct) {
                            self.voice_manager.lock().set_reverb_decay(self.reverb_decay);
                        }
                        if knob(ui, "Mix", &mut self.reverb_wet, 0.0, 1.0, 0.5, false, fmt_pct) {
                            self.voice_manager.lock().set_reverb_wet(self.reverb_wet);
                        }
                        if knob(ui, "Spring", &mut self.spring, 0.0, 1.0, 0.0, false, fmt_pct) {
                            self.voice_manager.lock().set_spring(self.spring);
                        }
                    });
                });
                vseparator(ui, 150.0);
                ui.vertical(|ui| {
                    ui.label(legend("Tape"));
                    ui.add_space(34.0);
                    ui.horizontal(|ui| {
                        if knob(ui, "Wow", &mut self.tape_wow, 0.0, 1.0, 0.0, false, fmt_pct) {
                            self.voice_manager.lock().set_tape_wow(self.tape_wow);
                        }
                        if knob(ui, "Flutter", &mut self.tape_flutter, 0.0, 1.0, 0.0, false, fmt_pct) {
                            self.voice_manager.lock().set_tape_flutter(self.tape_flutter);
                        }
                        if knob(ui, "Drive", &mut self.tape_drive, 0.0, 1.0, 0.0, false, fmt_pct) {
                            self.voice_manager.lock().set_tape_drive(self.tape_drive);
                        }
                        if knob(ui, "Age", &mut self.tape_age, 0.0, 1.0, 0.0, false, fmt_pct) {
                            self.voice_manager.lock().set_tape_age(self.tape_age);
                        }
                    });
                });
                vseparator(ui, 150.0);
                ui.vertical(|ui| {
                    ui.label(legend("Fuzz"));
                    ui.add_space(34.0);
                    if knob(ui, "Germanium", &mut self.fuzz, 0.0, 1.0, 0.0, false, fmt_pct) {
                        self.voice_manager.lock().set_fuzz(self.fuzz);
                    }
                });
            });
        });
    }

    /// Output oscilloscope. The one place the interface goes cyan: the
    /// signal itself, trigger-stabilized on a rising zero crossing.
    fn draw_scope(&self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, 64.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, Rounding::same(10.0), INSET);
        painter.rect_stroke(rect, Rounding::same(10.0), Stroke::new(1.0, HAIRLINE));

        let inner = rect.shrink2(vec2(14.0, 10.0));
        painter.line_segment(
            [
                pos2(inner.left(), inner.center().y),
                pos2(inner.right(), inner.center().y),
            ],
            Stroke::new(1.0, HAIRLINE),
        );
        painter.text(
            pos2(rect.left() + 12.0, rect.top() + 6.0),
            Align2::LEFT_TOP,
            tracked("out"),
            FontId::proportional(8.5),
            TXT_LOW,
        );
        if self.notes_active {
            painter.circle_filled(pos2(rect.left() + 40.0, rect.top() + 10.5), 2.0, CYAN);
        }

        let samples: Vec<f32> = self.voice_manager.lock().scope.iter().copied().collect();
        if samples.len() < 32 {
            return;
        }

        // Show half the buffer, starting at a rising zero crossing when one
        // exists in the first half
        let window = samples.len() / 2;
        let mut start = 0;
        for i in 1..window {
            if samples[i - 1] <= 0.0 && samples[i] > 0.0 {
                start = i;
                break;
            }
        }

        let n = 256usize;
        let points: Vec<Pos2> = (0..n)
            .map(|i| {
                let sample = samples[start + i * (window - 1) / (n - 1)];
                pos2(
                    inner.left() + inner.width() * i as f32 / (n - 1) as f32,
                    inner.center().y - sample.clamp(-1.0, 1.0) * inner.height() * 0.5,
                )
            })
            .collect();

        painter.add(Shape::line(
            points.clone(),
            Stroke::new(3.5, Color32::from_rgba_premultiplied(0x6f, 0xe3, 0xf2, 36)),
        ));
        painter.add(Shape::line(points, Stroke::new(1.2, CYAN)));
    }

    // -----------------------------------------------------------------------
    // Keyboard
    // -----------------------------------------------------------------------

    /// Computer-key hint for a note, if the note is reachable from the
    /// QWERTY mapping at the current octave.
    fn key_hint(&self, visual_octave: usize, key_index: usize) -> Option<&'static str> {
        const LOWER: [&str; 12] = ["Z", "S", "X", "D", "C", "V", "G", "B", "H", "N", "J", "M"];
        const UPPER: [&str; 12] = ["Q", "2", "W", "3", "E", "R", "5", "T", "6", "Y", "7", "U"];
        match visual_octave {
            0 => Some(LOWER[key_index]),
            1 => Some(UPPER[key_index]),
            _ => None,
        }
    }

    fn draw_keyboard(&mut self, ui: &mut egui::Ui) {
        let available_width = ui.available_width();
        let white_key_width = available_width / (7.0 * OCTAVES as f32);
        let white_key_height = 126.0;
        let black_key_width = white_key_width * 0.6;
        let black_key_height = white_key_height * 0.6;

        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(available_width, white_key_height),
            egui::Sense::click_and_drag(),
        );
        self.handle_mouse_input(ui, rect, &response);

        // Light keys from the engine's live voice state, so song playback,
        // MIDI, QWERTY, and mouse input all show up on the keyboard
        let key_states = self.voice_manager.lock().held_note_states();

        let painter = ui.painter();
        painter.rect_filled(rect.expand(3.0), Rounding::same(6.0), INSET);

        // White keys
        for visual_octave in 0..OCTAVES {
            for (i, &key_index) in WHITE_KEY_INDICES.iter().enumerate() {
                if let Some(note) = self.calculate_midi_note(visual_octave as i32, key_index) {
                    let x = (visual_octave * 7 + i) as f32 * white_key_width;
                    let pressed = key_states[note as usize];
                    // Pressed keys sit down into the bed
                    let sink = if pressed { 2.0 } else { 0.0 };
                    let key_rect = Rect::from_min_size(
                        rect.min + Vec2::new(x + 1.0, sink),
                        Vec2::new(white_key_width - 2.0, white_key_height - sink),
                    );
                    let rounding = Rounding {
                        nw: 0.0,
                        ne: 0.0,
                        sw: 3.0,
                        se: 3.0,
                    };
                    if pressed {
                        painter.rect_filled(
                            key_rect.expand(2.0),
                            rounding,
                            Color32::from_rgba_premultiplied(0xe0, 0xa1, 0x54, 40),
                        );
                    }
                    painter.rect_filled(key_rect, rounding, if pressed { AMBER } else { IVORY });
                    if !pressed {
                        // Ivory sheen: light falls from the top
                        painter.add(gradient_quad(
                            Rect::from_min_max(
                                key_rect.min,
                                pos2(key_rect.right(), key_rect.top() + 30.0),
                            ),
                            Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 60),
                            Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0),
                        ));
                    }
                    let shade = Rect::from_min_max(
                        pos2(key_rect.min.x, key_rect.max.y - 6.0),
                        key_rect.max,
                    );
                    painter.rect_filled(
                        shade,
                        rounding,
                        if pressed { AMBER_DEEP } else { IVORY_SHADE },
                    );
                    painter.line_segment(
                        [key_rect.right_top(), key_rect.right_bottom()],
                        Stroke::new(1.0, Color32::from_rgba_premultiplied(0, 0, 0, 30)),
                    );

                    if key_index == 0 {
                        painter.text(
                            pos2(key_rect.center().x, key_rect.max.y - 28.0),
                            Align2::CENTER_CENTER,
                            format!("C{}", self.current_octave + visual_octave as i32),
                            FontId::proportional(8.5),
                            if pressed {
                                AMBER_INK
                            } else {
                                Color32::from_rgb(0xb8, 0xb1, 0x9e)
                            },
                        );
                    }
                    if let Some(hint) = self.key_hint(visual_octave, key_index) {
                        painter.text(
                            pos2(key_rect.center().x, key_rect.max.y - 15.0),
                            Align2::CENTER_CENTER,
                            hint,
                            FontId::proportional(10.0),
                            if pressed {
                                AMBER_INK
                            } else {
                                Color32::from_rgb(0xa3, 0x9c, 0x88)
                            },
                        );
                    }
                }
            }
        }

        // Black keys
        for visual_octave in 0..OCTAVES {
            for (i, &key_index) in BLACK_KEY_INDICES.iter().enumerate() {
                if let Some(note) = self.calculate_midi_note(visual_octave as i32, key_index) {
                    let x = match i {
                        0 => white_key_width * 0.75,
                        1 => white_key_width * 1.75,
                        2 => white_key_width * 3.75,
                        3 => white_key_width * 4.75,
                        4 => white_key_width * 5.75,
                        _ => unreachable!(),
                    };
                    let pressed = key_states[note as usize];
                    let sink = if pressed { 2.0 } else { 0.0 };
                    let key_rect = Rect::from_min_size(
                        rect.min
                            + Vec2::new(x + visual_octave as f32 * 7.0 * white_key_width, sink),
                        Vec2::new(black_key_width, black_key_height - sink),
                    );
                    let rounding = Rounding {
                        nw: 0.0,
                        ne: 0.0,
                        sw: 3.0,
                        se: 3.0,
                    };
                    painter.rect_filled(
                        key_rect,
                        rounding,
                        if pressed { AMBER_DEEP } else { EBONY },
                    );
                    if !pressed {
                        // Glossy ebony: lit top face
                        painter.add(gradient_quad(
                            Rect::from_min_max(
                                key_rect.min + vec2(1.0, 0.0),
                                pos2(key_rect.right() - 1.0, key_rect.top() + 16.0),
                            ),
                            Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 34),
                            Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0),
                        ));
                        let edge = Rect::from_min_max(
                            pos2(key_rect.min.x, key_rect.max.y - 4.0),
                            key_rect.max,
                        );
                        painter.rect_filled(edge, rounding, EBONY_EDGE);
                    }
                    painter.rect_stroke(key_rect, rounding, Stroke::new(1.0, INSET));

                    if let Some(hint) = self.key_hint(visual_octave, key_index) {
                        painter.text(
                            pos2(key_rect.center().x, key_rect.max.y - 12.0),
                            Align2::CENTER_CENTER,
                            hint,
                            FontId::proportional(9.5),
                            if pressed { AMBER_INK } else { TXT_LOW },
                        );
                    }
                }
            }
        }
    }

    fn get_note_from_pointer(&self, pos: egui::Pos2, rect: Rect) -> Option<u8> {
        let rel_pos = pos - rect.min;
        let octave_width = rect.width() / (OCTAVES as f32);
        let x_in_keyboard = rel_pos.x;
        let y = rel_pos.y;

        let white_key_width = octave_width / 7.0;
        let black_key_width = white_key_width * 0.6;
        let black_key_height = rect.height() * 0.6;

        // Calculate the visual octave and key within the keyboard
        let visual_octave = (x_in_keyboard / octave_width) as i32;
        let x_in_octave = x_in_keyboard % octave_width;

        // Check black keys first
        for (i, &key_index) in BLACK_KEY_INDICES.iter().enumerate() {
            let x = match i {
                0 => white_key_width * 0.75,
                1 => white_key_width * 1.75,
                2 => white_key_width * 3.75,
                3 => white_key_width * 4.75,
                4 => white_key_width * 5.75,
                _ => unreachable!(),
            };
            if x_in_octave >= x && x_in_octave < x + black_key_width && y < black_key_height {
                return self.calculate_midi_note(visual_octave, key_index);
            }
        }

        // If not a black key, it must be a white key
        let white_key_index = (x_in_octave / white_key_width) as usize;
        if white_key_index < WHITE_KEY_INDICES.len() {
            let key_index = WHITE_KEY_INDICES[white_key_index];
            return self.calculate_midi_note(visual_octave, key_index);
        }

        None
    }

    fn handle_keyboard_input(&mut self, ctx: &egui::Context) {
        const KEYS: [Key; 24] = [
            Key::Z, Key::S, Key::X, Key::D, Key::C, Key::V, Key::G, Key::B, Key::H, Key::N, Key::J, Key::M,
            Key::Q, Key::Num2, Key::W, Key::Num3, Key::E, Key::R, Key::Num5, Key::T, Key::Num6, Key::Y, Key::Num7, Key::U,
        ];

        if ctx.input(|i| i.key_pressed(Key::ArrowUp)) {
            self.shift_octave(1);
        }
        if ctx.input(|i| i.key_pressed(Key::ArrowDown)) {
            self.shift_octave(-1);
        }

        // Keys pressed as part of an OS shortcut (Cmd/Alt chords) are not
        // note presses
        let chord = ctx.input(|i| i.modifiers.command || i.modifiers.alt);

        for &key in KEYS.iter() {
            if ctx.input(|i| i.key_pressed(key))
                && !chord
                && !self.pressed_keys.contains_key(&key)
            {
                if let Some(note) = self.key_to_note(key) {
                    self.play_note(note);
                    self.pressed_keys.insert(key, note);
                }
            }
            if ctx.input(|i| i.key_released(key)) {
                // Stop the note this key actually started, not whatever it
                // would map to now
                if let Some(note) = self.pressed_keys.remove(&key) {
                    self.stop_note(note);
                }
            }
        }

        // Release events can be lost (focus loss, OS dialogs, shortcuts).
        // Reconcile with the live key state so no note drones on and no key
        // is left "pressed" and refusing to retrigger.
        let stale: Vec<Key> = ctx.input(|i| {
            self.pressed_keys
                .keys()
                .copied()
                .filter(|k| !i.keys_down.contains(k))
                .collect()
        });
        for key in stale {
            if let Some(note) = self.pressed_keys.remove(&key) {
                self.stop_note(note);
            }
        }
    }

    /// Change octave, releasing any held notes first so nothing gets stuck
    /// (note-off would otherwise map to a different MIDI note).
    fn shift_octave(&mut self, delta: i32) {
        let held: Vec<u8> = self.pressed_keys.drain().map(|(_, note)| note).collect();
        for note in held {
            self.stop_note(note);
        }
        if let Some(note) = self.active_mouse_note.take() {
            self.stop_note(note);
        }
        self.current_octave = (self.current_octave + delta).clamp(0, 8);
    }

    fn handle_mouse_input(&mut self, ui: &egui::Ui, rect: Rect, response: &egui::Response) {
        // Hold the note for as long as the mouse button is down on the keyboard,
        // gliding to a new note when the pointer drags across key boundaries
        if response.is_pointer_button_down_on() || response.dragged() {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                if let Some(note) = self.get_note_from_pointer(pos, rect) {
                    if Some(note) != self.active_mouse_note {
                        if let Some(old_note) = self.active_mouse_note.take() {
                            self.stop_note(old_note);
                        }
                        self.play_note(note);
                        self.active_mouse_note = Some(note);
                    }
                }
            }
        } else if let Some(old_note) = self.active_mouse_note.take() {
            self.stop_note(old_note);
        }
    }

    fn key_to_note(&self, key: Key) -> Option<u8> {
        let base_index = match key {
            Key::Z => 0, Key::S => 1, Key::X => 2, Key::D => 3, Key::C => 4, Key::V => 5,
            Key::G => 6, Key::B => 7, Key::H => 8, Key::N => 9, Key::J => 10, Key::M => 11,
            Key::Q => 12, Key::Num2 => 13, Key::W => 14, Key::Num3 => 15, Key::E => 16, Key::R => 17,
            Key::Num5 => 18, Key::T => 19, Key::Num6 => 20, Key::Y => 21, Key::Num7 => 22, Key::U => 23,
            _ => return None,
        };

        let octave_offset = base_index / 12;
        let note_index = base_index % 12;
        self.calculate_midi_note(octave_offset, note_index.try_into().unwrap())
    }

    fn calculate_midi_note(&self, visual_octave: i32, key_index: usize) -> Option<u8> {
        let base_note = (self.current_octave + visual_octave) * 12 + key_index as i32;
        if base_note >= 0 && base_note <= 127 {
            Some(base_note as u8)
        } else {
            None
        }
    }

    fn play_note(&mut self, note: u8) {
        self.voice_manager.lock().note_on(note, 0.9);
    }

    fn stop_note(&mut self, note: u8) {
        self.voice_manager.lock().note_off(note);
    }
}
