use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, CursorIcon, FontId, Key, Pos2, Rect, RichText,
    Sense, Shape, Stroke, TextureHandle, TextureOptions, Vec2,
};
use eframe::egui::epaint::{EllipseShape, Mesh};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;

use crate::aurora_gpu;
use crate::chorus::ChorusMode;
use crate::oscillator::{CircuitModel, Waveform};
use crate::panel_render;
use crate::song::{Curve, Param};
use crate::voice_manager::VoiceManager;

const OCTAVES: usize = 3;
const WHITE_KEY_INDICES: [usize; 7] = [0, 2, 4, 5, 7, 9, 11];
const BLACK_KEY_INDICES: [usize; 5] = [1, 3, 6, 8, 10];

/// The right-hand 909 pad grid on the QWERTY keyboard, mirrored as
/// clickable pads beside the piano: (drum name, key hint, key, base
/// velocity, glyph/activity index, ghost). Top row K L ; ' sits over
/// bottom row , . / — hats and colors above, kick/snare/clap backbone
/// below. Shift is the accent line; the ' pad is a ghost snare for
/// rolls. Pads carry pictographic glyphs, not abbreviations.
const PAD_TOP: [(&str, &str, Key, f32, usize, bool); 4] = [
    ("CH", "K", Key::K, 0.7, 4, false),
    ("OH", "L", Key::L, 0.7, 5, false),
    ("RS", ";", Key::Semicolon, 0.75, 2, false),
    ("SD", "'", Key::Quote, 0.35, 1, true),
];
const PAD_BOTTOM: [(&str, &str, Key, f32, usize, bool); 3] = [
    ("BD", ",", Key::Comma, 0.85, 0, false),
    ("SD", ".", Key::Period, 0.8, 1, false),
    ("CP", "/", Key::Slash, 0.8, 3, false),
];

// ---------------------------------------------------------------------------
// Design system: light Frutiger Aero. A luminous animated sky, white
// frosted glass with dark slate type, dark glossy "device screens" set into
// the glass (wells), warm walnut rails, amber for touch, aqua for signal.
// ---------------------------------------------------------------------------
const BG0: Color32 = Color32::from_rgb(0x6f, 0xa8, 0xd0); // sky fallback
const BG2: Color32 = Color32::from_rgb(0xd4, 0xe7, 0xef); // light controls
const BG2_HOVER: Color32 = Color32::from_rgb(0xe4, 0xf2, 0xf8);
const INSET: Color32 = Color32::from_rgb(0x0a, 0x11, 0x14); // device screens

// TOUCH is the single interaction accent (deep aqua): arcs, lit keys,
// selection gloss. Dark hairlines sit on white glass.
const HAIRLINE: Color32 = Color32::from_rgba_premultiplied(0x24, 0x3a, 0x46, 45);
const HAIRLINE_HI: Color32 = Color32::from_rgba_premultiplied(0x1d, 0x33, 0x40, 85);

const TXT: Color32 = Color32::from_rgb(0x24, 0x33, 0x3c);
const TXT_MID: Color32 = Color32::from_rgb(0x43, 0x54, 0x5e);
const TXT_LOW: Color32 = Color32::from_rgb(0x6b, 0x7c, 0x86);

// Text inside the dark wells needs to stay light
const WELL_TXT: Color32 = Color32::from_rgb(0x7f, 0x96, 0xa0);
const WELL_TXT_HOVER: Color32 = Color32::from_rgb(0xc6, 0xd8, 0xde);
const WELL_LINE: Color32 = Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 22);

const TOUCH: Color32 = Color32::from_rgb(0x12, 0x9e, 0xc0);
const TOUCH_HI: Color32 = Color32::from_rgb(0x1e, 0xc2, 0xe8);
const TOUCH_DEEP: Color32 = Color32::from_rgb(0x0d, 0x7c, 0x98);
const TOUCH_INK: Color32 = Color32::from_rgb(0x05, 0x33, 0x40);

const CYAN: Color32 = Color32::from_rgb(0x35, 0xdf, 0xf5);

const CYAN_BRIGHT: Color32 = Color32::from_rgb(0xf4, 0xfd, 0xff);

const IVORY: Color32 = Color32::from_rgb(0xea, 0xe6, 0xdb);
const IVORY_SHADE: Color32 = Color32::from_rgb(0xd6, 0xd0, 0xc1);
const EBONY: Color32 = Color32::from_rgb(0x15, 0x16, 0x1a);
const EBONY_EDGE: Color32 = Color32::from_rgb(0x2c, 0x2f, 0x36);

// GPU state shared with free-function widgets: whether the WGSL pipeline is
// live, the frame time, and a per-frame uniform slot counter (slot 0 = sky).
static GPU_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static TIME_BITS: AtomicU64 = AtomicU64::new(0);

/// Card rects collected during layout; the NEXT frame paints their glass
/// into the background layer before any content, so the panes always sit
/// under the controls. One frame of lag, imperceptible at 60 Hz.
static GLASS_RECTS: Mutex<Vec<Rect>> = Mutex::new(Vec::new());

// ---------------------------------------------------------------------------
// Per-pixel material synthesis — realism without image assets. An fBm
// walnut with growth rings, a sphere-shaded gloss knob sprite, and a soft
// aurora backdrop whose low-frequency light lets the translucent panels
// above it read as frosted glass.
// ---------------------------------------------------------------------------

struct Textures {
    backdrop: TextureHandle,
    backdrop_rgb: Vec<[f32; 3]>,
    backdrop_size: [usize; 2],
    /// Baked frosted-glass panels, keyed by rounded screen rect.
    frost: HashMap<(i32, i32, i32, i32), TextureHandle>,
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


/// Graphite chrome rail: the same dark-device family as the wells and
/// scope, replacing the walnut (blue + orange + brown never resolved).
fn rail_shapes(rect: Rect) -> Vec<Shape> {
    vec![
        Shape::rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(0x14, 0x19, 0x1f)),
        gradient_quad(
            Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + rect.height() * 0.5)),
            Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 16),
            Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0),
        ),
        gradient_quad(
            Rect::from_min_max(pos2(rect.left(), rect.bottom() - 14.0), rect.max),
            Color32::from_rgba_unmultiplied(0x00, 0x00, 0x00, 0),
            Color32::from_rgba_unmultiplied(0x00, 0x00, 0x00, 90),
        ),
    ]
}

/// Frosted glass panel: stacked soft shadow, translucent cool fill, a light
/// sweep across the top, bright inner edge.
fn glass_shapes(rect: Rect, rounding: f32) -> Vec<Shape> {
    let cr = CornerRadius::same(rounding as u8);
    let mut shapes = Vec::with_capacity(10);
    for (expand, alpha) in [(6.0f32, 22), (4.0, 34), (2.0, 52), (0.5, 70)] {
        shapes.push(Shape::rect_stroke(
            rect.expand(expand),
            CornerRadius::same((rounding + expand) as u8),
            Stroke::new(2.0, Color32::from_rgba_unmultiplied(0x10, 0x2a, 0x38, alpha)),
            egui::StrokeKind::Outside,
        ));
    }
    shapes.push(Shape::rect_filled(
        rect,
        cr,
        Color32::from_rgba_unmultiplied(0xf4, 0xfa, 0xfc, 150),
    ));
    let sweep = Rect::from_min_max(
        rect.min + vec2(rounding, 1.5),
        pos2(rect.right() - rounding, rect.top() + rect.height() * 0.42),
    );
    shapes.push(gradient_quad(
        sweep,
        Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 90),
        Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0),
    ));
    shapes.push(Shape::rect_stroke(
        rect,
        cr,
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 170)),
        egui::StrokeKind::Inside,
    ));
    shapes
}

/// Aero gloss for selected segmented cells: aqua glass with a lit top half.
fn gloss_fill(painter: &egui::Painter, rect: Rect, rounding: f32) {
    let cr = CornerRadius::same(rounding as u8);
    painter.rect_filled(
        rect,
        cr,
        Color32::from_rgba_unmultiplied(0x2f, 0xc0, 0xdd, 200),
    );
    let top = Rect::from_min_max(rect.min, pos2(rect.right(), rect.center().y));
    painter.add(gradient_quad(
        top.shrink(1.0),
        Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 90),
        Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 8),
    ));
    painter.rect_stroke(
        rect,
        cr,
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xc9, 0xf2, 0xfb, 220)),
        egui::StrokeKind::Inside,
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
    glide: f32,
    sub: f32,
    osc2_wave: Waveform,
    osc2_pitch: f32,
    osc2_level: f32,
    osc3_wave: Waveform,
    osc3_pitch: f32,
    osc3_level: f32,
    circuit: CircuitModel,
    key_track: f32,
    osc_fm: f32,
    sync: bool,
    ring: f32,
    pulse_width: f32,
    lfo_rate: f32,
    lfo_shape: f32,
    lfo_pitch: f32,
    lfo_filter: f32,
    lfo_pwm: f32,
    detune: f32,
    fenv_amount: f32,
    fenv_attack: f32,
    fenv_decay: f32,
    fenv_sustain: f32,
    fenv_release: f32,
    active_mouse_note: Option<u8>,
    active_patch: Option<usize>,
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
    // The rhythm section's panel (mirrors ParamValues like everything else)
    bd_level: f32,
    bd_tune: f32,
    bd_attack: f32,
    bd_decay: f32,
    bd_sweep: f32,
    bd_drive: f32,
    sd_level: f32,
    sd_tune: f32,
    sd_tone: f32,
    sd_snappy: f32,
    sd_decay: f32,
    rs_level: f32,
    rs_tune: f32,
    cp_level: f32,
    cp_decay: f32,
    hh_level: f32,
    hh_tune: f32,
    hh_metal: f32,
    ch_decay: f32,
    oh_decay: f32,
    dr_drive: f32,
    /// QWERTY keys currently sounding, mapped to the MIDI note each one
    /// started (so note-off always matches, even if the octave changed).
    pressed_keys: HashMap<Key, u8>,
    /// Drum-pad keys currently held (edge detection only — drum voices
    /// are one-shots with no note-off).
    pressed_drum_keys: HashSet<Key>,
    /// Which on-screen pad the mouse is currently striking.
    mouse_pad_down: Option<usize>,
    /// Strike flash for the ghost-snare pad: it shares the snare VOICE,
    /// so lighting it from voice activity would double-light with the
    /// main snare pad — it flashes only on its own strikes.
    ghost_flash: f32,
    theme_applied: bool,
    notes_active: bool,
    textures: Option<Textures>,
    /// Slow-smoothed engine signals feeding the sky: loudness, filter
    /// openness, and an integrated cloud-drift phase.
    mood_energy: f32,
    mood_bright: f32,
    sky_phase: f32,
    /// Resize debounce: the backdrop rebakes once the size holds still.
    pending_size: [usize; 2],
    size_stable_frames: u32,
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
    RichText::new(tracked(text)).size(10.0).color(TXT_MID)
}

fn sublegend(text: &str) -> RichText {
    RichText::new(tracked(text)).size(8.5).color(TXT_LOW)
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
/// A knob bound to a `Param`: range and taper come from THE range table
/// (`Param::range`), and changes flow through `Param::apply` — the same
/// path songs, patches, and MIDI use. The UI is just another performer.
fn param_knob(
    ui: &mut egui::Ui,
    vm: &Arc<Mutex<VoiceManager>>,
    label: &str,
    param: Param,
    value: &mut f32,
    default: f32,
    fmt: impl Fn(f32) -> String,
) {
    let (lo, hi, curve) = param.range();
    if knob(ui, label, value, lo, hi, default, curve == Curve::Log, fmt) {
        param.apply(&mut vm.lock(), *value);
    }
}

/// Compact knob bound to a `Param` — the rhythm card's 21 controls fit a
/// single row only at this density.
fn param_knob_sm(
    ui: &mut egui::Ui,
    vm: &Arc<Mutex<VoiceManager>>,
    label: &str,
    param: Param,
    value: &mut f32,
    default: f32,
) {
    let (lo, hi, curve) = param.range();
    if knob_sized(
        ui,
        label,
        value,
        lo,
        hi,
        default,
        curve == Curve::Log,
        fmt_pct,
        true,
    ) {
        param.apply(&mut vm.lock(), *value);
    }
}

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
    knob_sized(ui, label, value, min, max, default, logarithmic, fmt, false)
}

fn knob_sized(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    logarithmic: bool,
    fmt: impl Fn(f32) -> String,
    compact: bool,
) -> bool {
    // Geometry per density: (w, h, center_y, tick r0, tick major, tick
    // minor, arc r, disc r, pointer in/out, label pt, value pt)
    let g = if compact {
        (48.0, 74.0, 34.0, 16.0, 20.0, 18.5, 13.5, 10.5, 3.5, 9.5, 7.6, 8.5)
    } else {
        (59.0, 78.0, 38.0, 20.0, 24.0, 22.0, 17.0, 13.0, 4.5, 11.5, 9.0, 10.0)
    };
    let (rect, response) = ui.allocate_exact_size(vec2(g.0, g.1), Sense::click_and_drag());
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
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            let fine = ui.input(|i| i.modifiers.shift);
            let sensitivity = if fine { 0.0004 } else { 0.0015 };
            *value = from_t((to_t(*value) + scroll * sensitivity).clamp(0.0, 1.0));
            changed = true;
        }
    }

    let response = response
        .on_hover_cursor(CursorIcon::ResizeVertical)
        .on_hover_text("drag or scroll · shift for fine · double-click resets");
    let engaged = response.hovered() || response.dragged();

    let painter = ui.painter();
    let center = pos2(rect.center().x, rect.top() + g.2);
    let start = 135.0_f32.to_radians();
    let sweep = 270.0_f32.to_radians();
    let t = to_t(*value);

    painter.text(
        pos2(rect.center().x, rect.top() + 3.0),
        Align2::CENTER_TOP,
        tracked(label),
        FontId::proportional(g.10),
        if engaged { TXT_MID } else { TXT_LOW },
    );

    // Etched ticks, majors slightly longer
    for i in 0..=10 {
        let a = start + sweep * i as f32 / 10.0;
        let dir = vec2(a.cos(), a.sin());
        let (r0, r1) = if i % 5 == 0 { (g.3, g.4) } else { (g.3, g.5) };
        painter.line_segment(
            [center + dir * r0, center + dir * r1],
            Stroke::new(1.0, HAIRLINE_HI),
        );
    }

    // Value arc — from 12 o'clock for bipolar ranges, from min otherwise
    let arc_r = g.6;
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
            Stroke::new(2.5, if engaged { TOUCH_HI } else { TOUCH }),
        ));
    }
    // Arc endpoint
    let end_angle = start + sweep * t;
    painter.circle_filled(
        center + vec2(end_angle.cos(), end_angle.sin()) * arc_r,
        2.0,
        if engaged { TOUCH_HI } else { TOUCH },
    );

    // Committed 2D: a flat disc; the arc, ticks, and pointer carry it
    let disc = if engaged {
        Color32::from_rgb(0x28, 0x33, 0x3d)
    } else {
        Color32::from_rgb(0x20, 0x29, 0x31)
    };
    painter.circle_filled(center, g.7, disc);
    painter.circle_stroke(
        center,
        g.7,
        if engaged {
            Stroke::new(1.2, Color32::from_rgba_unmultiplied(0x1e, 0xc2, 0xe8, 150))
        } else {
            Stroke::new(1.0, HAIRLINE_HI)
        },
    );

    let dir = vec2(end_angle.cos(), end_angle.sin());
    painter.line_segment(
        [center + dir * g.8, center + dir * g.9],
        Stroke::new(
            2.0,
            if engaged { TOUCH_HI } else { Color32::from_rgb(0xee, 0xf4, 0xf6) },
        ),
    );

    painter.text(
        pos2(rect.center().x, rect.bottom() - 2.0),
        Align2::CENTER_BOTTOM,
        fmt(*value),
        FontId::monospace(g.11),
        if engaged { TOUCH_HI } else { TXT_LOW },
    );

    changed
}

/// Pictographic drum-voice glyphs, drawn in the same hand as the wave
/// glyphs: 0 kick head, 1 snare shell with wires, 2 stick on the rim,
/// 3 clap burst, 4 closed hats kissed on the stand, 5 open hats lifted,
/// 6 the bus-drive saturation curve. `ghost` fades the glyph (the soft
/// snare pad for rolls).
fn drum_glyph(painter: &egui::Painter, rect: Rect, which: usize, color: Color32, ghost: bool) {
    let color = if ghost { color.gamma_multiply(0.5) } else { color };
    let s = Stroke::new(1.7, color);
    let thin = Stroke::new(1.2, color);
    let c = rect.center();
    let w = rect.width().min(rect.height() * 1.6);
    match which {
        0 => {
            // Kick: a front-facing bass drum — big round shell on its two
            // floor spurs, dot for the beater pad
            let r = w * 0.36;
            let dc = c - vec2(0.0, 1.0);
            painter.circle_stroke(dc, r, s);
            painter.circle_filled(dc, 1.8, color);
            for sx in [-1.0f32, 1.0] {
                painter.line_segment(
                    [
                        dc + vec2(sx * r * 0.6, r * 0.75),
                        dc + vec2(sx * r * 0.95, r + 3.0),
                    ],
                    thin,
                );
            }
        }
        1 => {
            // Snare: crossed sticks over the shallow shell — the classic
            // percussion mark, unmistakable at pad size
            let shell = Rect::from_center_size(
                c + vec2(0.0, w * 0.22),
                vec2(w * 0.85, w * 0.30),
            );
            painter.rect_stroke(shell, CornerRadius::same(2), s, egui::StrokeKind::Inside);
            let top = shell.top() - 1.0;
            painter.line_segment(
                [pos2(c.x - w * 0.42, c.y - w * 0.44), pos2(c.x + w * 0.20, top)],
                Stroke::new(1.8, color),
            );
            painter.line_segment(
                [pos2(c.x + w * 0.42, c.y - w * 0.44), pos2(c.x - w * 0.20, top)],
                Stroke::new(1.8, color),
            );
        }
        2 => {
            // Rim shot: the stick striking down onto the drum's edge
            let shell = Rect::from_center_size(
                c + vec2(0.0, w * 0.18),
                vec2(w * 0.9, w * 0.34),
            );
            painter.rect_stroke(shell, CornerRadius::same(2), s, egui::StrokeKind::Inside);
            painter.line_segment(
                [
                    pos2(c.x + w * 0.45, c.y - w * 0.42),
                    pos2(c.x - w * 0.05, shell.top() + 1.0),
                ],
                Stroke::new(2.2, color),
            );
            painter.circle_filled(pos2(c.x - w * 0.05, shell.top() + 1.0), 1.8, color);
        }
        3 => {
            // Clap: a bold eight-ray burst, long and short rays alternating
            for k in 0..8 {
                let a = std::f32::consts::TAU * k as f32 / 8.0 + std::f32::consts::FRAC_PI_8;
                let d = vec2(a.cos(), a.sin());
                let reach = if k % 2 == 0 { w * 0.42 } else { w * 0.27 };
                painter.line_segment([c + d * (w * 0.10), c + d * reach], s);
            }
        }
        4 => {
            // Closed hat: two cymbal lenses kissed together on the stand
            let rx = w * 0.42;
            let ry = (w * 0.10).max(2.0);
            painter.line_segment([c + vec2(0.0, 2.0), c + vec2(0.0, w * 0.42)], thin);
            for dy in [-2.5f32, 2.0] {
                painter.add(Shape::Ellipse(EllipseShape {
                    center: c + vec2(0.0, dy),
                    radius: vec2(rx, ry),
                    fill: color,
                    stroke: Stroke::NONE,
                }));
            }
        }
        5 => {
            // Open hat: the top cymbal lifted clear off the bottom one
            let rx = w * 0.42;
            let ry = (w * 0.10).max(2.0);
            painter.line_segment([c + vec2(0.0, 4.0), c + vec2(0.0, w * 0.46)], thin);
            for dy in [-w * 0.34, 3.5] {
                painter.add(Shape::Ellipse(EllipseShape {
                    center: c + vec2(0.0, dy),
                    radius: vec2(rx, ry),
                    fill: color,
                    stroke: Stroke::NONE,
                }));
            }
        }
        _ => {
            // Bus drive: a tanh curve pressed into the rails
            let n = 14;
            let pts: Vec<Pos2> = (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32;
                    let x = (t - 0.5) * w * 0.9;
                    let y = -(x * 0.24).tanh() * w * 0.36;
                    pos2(c.x + x, c.y + y)
                })
                .collect();
            painter.add(Shape::line(pts, s));
        }
    }
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
/// `id` distinguishes the three oscillator sections' selectors.
fn waveform_selector(ui: &mut egui::Ui, id: &str, selected: &mut Waveform) -> bool {
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
    painter.rect_filled(rect, CornerRadius::same(7), INSET);

    let mut changed = false;
    for (i, wf) in OPTIONS.iter().enumerate() {
        let cell_rect = Rect::from_min_size(
            pos2(rect.left() + cell.x * i as f32, rect.top()),
            cell,
        );
        let response = ui.interact(
            cell_rect,
            ui.id().with((id, "wave", i)),
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
            WELL_TXT_HOVER
        } else {
            WELL_TXT
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
    painter.rect_stroke(rect, CornerRadius::same(7), Stroke::new(1.0, HAIRLINE), egui::StrokeKind::Inside);
    changed
}

/// Segmented text selector; returns the newly selected index if it changed.
fn segmented(ui: &mut egui::Ui, id: &str, labels: &[&str], selected: usize) -> Option<usize> {
    // Cells size to their text so long labels (MOOG) never overflow
    let widths: Vec<f32> = labels
        .iter()
        .map(|l| (l.len() as f32 * 9.0 + 26.0).max(36.0))
        .collect();
    let cell_h = 24.0;
    let total: f32 = widths.iter().sum();
    let (rect, _) = ui.allocate_exact_size(vec2(total, cell_h), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(7), INSET);

    let mut result = None;
    let mut cx = rect.left();
    for (i, label) in labels.iter().enumerate() {
        let cell_rect = Rect::from_min_size(pos2(cx, rect.top()), vec2(widths[i], cell_h));
        cx += widths[i];
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
            WELL_TXT_HOVER
        } else {
            WELL_TXT
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
                Stroke::new(1.0, WELL_LINE),
            );
        }
    }
    painter.rect_stroke(rect, CornerRadius::same(7), Stroke::new(1.0, HAIRLINE), egui::StrokeKind::Inside);
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
    painter.rect_filled(rect, CornerRadius::same(6), fill);
    painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, HAIRLINE), egui::StrokeKind::Inside);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(13.0),
        if response.hovered() { TXT } else { TXT_MID },
    );
    response
}

/// Frosted glass card with legend header and hairline rule. Given the
/// baked layers, the glass is real: the backdrop behind this rect, Gaussian
/// blurred, masked, and edge-lit — with a soft baked drop shadow.
fn card<R>(
    ui: &mut egui::Ui,
    title: &str,
    tex: Option<&mut Textures>,
    fill_width: Option<f32>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) {
    let bg_idx = ui.painter().add(Shape::Noop);
    let inner = egui::Frame::NONE
        .inner_margin(egui::Margin {
            left: 12,
            right: 12,
            top: 7,
            bottom: 8,
        })
        .show(ui, |ui| {
            // The card sizes to its content — measured widths only, never
            // available_width (unbounded inside rows in egui 0.31). A row's
            // LAST card passes the measured remainder to run flush right.
            if let Some(w) = fill_width {
                ui.set_min_width((w - 28.0).max(60.0));
            }
            ui.label(legend(title));
            ui.add_space(6.0);
            add_contents(ui);
        });
    let rect = inner.response.rect;
    if GPU_ON.load(AtomicOrdering::Relaxed) {
        // Living glass: record the rect; next frame's background pass
        // paints the pane underneath everything.
        GLASS_RECTS.lock().push(rect);
        ui.painter().set(bg_idx, Shape::Noop);
    } else if let Some(t) = tex {
        let key = (
            rect.left().round() as i32,
            rect.top().round() as i32,
            rect.width().round() as i32,
            rect.height().round() as i32,
        );
        if !t.frost.contains_key(&key) {
            if t.frost.len() > 64 {
                t.frost.clear();
            }
            let img = panel_render::frost_panel(
                &t.backdrop_rgb,
                t.backdrop_size[0],
                t.backdrop_size[1],
                (rect.left(), rect.top(), rect.width(), rect.height()),
                12.0,
            );
            let handle = ui
                .ctx()
                .load_texture(format!("frost-{:?}", key), img, TextureOptions::LINEAR);
            t.frost.insert(key, handle);
        }
        let handle = &t.frost[&key];
        ui.painter().set(
            bg_idx,
            Shape::image(
                handle.id(),
                rect.expand(panel_render::frost_pad()),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            ),
        );
    } else {
        ui.painter().set(bg_idx, Shape::Vec(glass_shapes(rect, 12.0)));
    }
}

/// Vertical hairline between subgroups inside a card.
fn vseparator(ui: &mut egui::Ui, height: f32) {
    // A breath of space, not a line — the grouping reads from the gap
    let _ = ui.allocate_exact_size(vec2(9.0, height), Sense::hover());
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
            glide: 0.0,
            sub: 0.0,
            osc2_wave: Waveform::Sawtooth,
            osc2_pitch: 0.0,
            osc2_level: 0.72,
            osc3_wave: Waveform::Sawtooth,
            osc3_pitch: 0.0,
            osc3_level: 0.72,
            circuit: CircuitModel::Moog,
            key_track: 0.4,
            osc_fm: 0.0,
            sync: false,
            ring: 0.0,
            pulse_width: 0.5,
            lfo_rate: 1.0,
            lfo_shape: 0.5,
            lfo_pitch: 0.0,
            lfo_filter: 0.0,
            lfo_pwm: 0.0,
            detune: 7.0,
            fenv_amount: 0.0,
            fenv_attack: 0.005,
            fenv_decay: 0.3,
            fenv_sustain: 0.0,
            fenv_release: 0.3,
            active_mouse_note: None,
            active_patch: None,
            chorus_rate: 0.5,
            chorus_depth: 0.3,
            chorus_mode: ChorusMode::Off,
            reverb_decay: 0.5,
            reverb_wet: 0.5,
            tape_wow: 0.0,
            tape_flutter: 0.0,
            tape_drive: 0.0,
            tape_age: 0.0,
            // 909 panel defaults, mirroring ParamValues::default()
            bd_level: 0.8,
            bd_tune: 0.35,
            bd_attack: 0.5,
            bd_decay: 0.45,
            bd_sweep: 0.5,
            bd_drive: 0.25,
            sd_level: 0.75,
            sd_tune: 0.4,
            sd_tone: 0.5,
            sd_snappy: 0.6,
            sd_decay: 0.5,
            rs_level: 0.7,
            rs_tune: 0.5,
            cp_level: 0.75,
            cp_decay: 0.5,
            hh_level: 0.7,
            hh_tune: 0.5,
            hh_metal: 0.65,
            ch_decay: 0.35,
            oh_decay: 0.5,
            dr_drive: 0.0,
            pressed_keys: HashMap::new(),
            pressed_drum_keys: HashSet::new(),
            mouse_pad_down: None,
            ghost_flash: 0.0,
            theme_applied: false,
            notes_active: false,
            textures: None,
            mood_energy: 0.0,
            mood_bright: 1.0,
            sky_phase: 0.0,
            pending_size: [0, 0],
            size_stable_frames: 0,
        };
        // Push the UI defaults into the engine so what you see is what you hear
        ui.apply_all_settings();
        // Hardware powers on IN a state — the Polymoog test sheet notes
        // "Preset 8 always comes on first". Patina comes on in Init.
        let mut ui = ui;
        if crate::patch::apply(&mut ui.voice_manager.lock(), crate::patch::FACTORY[0].1).is_ok() {
            ui.active_patch = Some(0);
        }
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
        vm.set_glide(self.glide);
        vm.set_sub(self.sub);
        vm.set_osc_wave(1, self.osc2_wave);
        vm.set_osc_pitch(1, self.osc2_pitch);
        vm.set_osc_level(1, self.osc2_level);
        vm.set_osc_wave(2, self.osc3_wave);
        vm.set_osc_pitch(2, self.osc3_pitch);
        vm.set_osc_level(2, self.osc3_level);
        vm.set_circuit(self.circuit);
        vm.set_key_track(self.key_track);
        vm.set_osc_fm(self.osc_fm);
        vm.set_sync(self.sync);
        vm.set_ring(self.ring);
        vm.set_pulse_width(self.pulse_width);
        vm.set_lfo_rate(self.lfo_rate);
        vm.set_lfo_shape(self.lfo_shape);
        vm.set_lfo_pitch(self.lfo_pitch);
        vm.set_lfo_filter(self.lfo_filter);
        vm.set_lfo_pwm(self.lfo_pwm);
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
        v.widgets.inactive.corner_radius = CornerRadius::same(6);
        v.widgets.hovered.corner_radius = CornerRadius::same(6);
        v.widgets.active.corner_radius = CornerRadius::same(6);
        v.widgets.open.corner_radius = CornerRadius::same(6);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, TXT_MID);
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, TXT);
        v.widgets.active.fg_stroke = Stroke::new(1.0, TOUCH);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, HAIRLINE);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, HAIRLINE_HI);
        v.selection.bg_fill = Color32::from_rgb(0x10, 0x3a, 0x46);
        v.selection.stroke = Stroke::new(1.0, TOUCH);

        ctx.set_style(style);
    }

    /// Announce that the WGSL sky/glass pipeline is installed; the CPU
    /// frost path then stays dormant as a fallback.
    pub fn set_gpu_available(&mut self, on: bool) {
        GPU_ON.store(on, AtomicOrdering::Relaxed);
    }

    /// Bake (or rebake) the raster layers. The backdrop renders at window
    /// size; frost panels derive from it, so they are dropped with it. A
    /// resize debounce keeps drag-resizing cheap. When the GPU pipeline is
    /// live, the backdrop bake shrinks to a placeholder — only the wood and
    /// knob sprites are needed.
    fn ensure_textures(&mut self, ctx: &egui::Context) {
        if GPU_ON.load(AtomicOrdering::Relaxed) {
            if self.textures.is_none() {
                let rgb = panel_render::render_backdrop(8, 8);
                let backdrop = ctx.load_texture(
                    "patina-backdrop",
                    panel_render::backdrop_image(8, 8, &rgb),
                    TextureOptions::LINEAR,
                );
                self.textures = Some(Textures {
                    backdrop,
                    backdrop_rgb: rgb,
                    backdrop_size: [8, 8],
                    frost: HashMap::new(),
                });
            }
            return;
        }
        let screen = ctx.screen_rect();
        let size = [
            (screen.width().max(320.0)) as usize,
            (screen.height().max(240.0)) as usize,
        ];
        let stale = match &self.textures {
            None => true,
            Some(t) => {
                (t.backdrop_size[0] as i32 - size[0] as i32).abs() > 8
                    || (t.backdrop_size[1] as i32 - size[1] as i32).abs() > 8
            }
        };
        if !stale {
            return;
        }
        if self.pending_size == size {
            self.size_stable_frames += 1;
        } else {
            self.pending_size = size;
            self.size_stable_frames = 0;
        }
        // First bake happens immediately; rebakes wait for a stable size
        if self.textures.is_some() && self.size_stable_frames < 12 {
            return;
        }

        let rgb = panel_render::render_backdrop(size[0], size[1]);
        let backdrop = ctx.load_texture(
            "patina-backdrop",
            panel_render::backdrop_image(size[0], size[1], &rgb),
            TextureOptions::LINEAR,
        );
        self.textures = Some(Textures {
            backdrop,
            backdrop_rgb: rgb,
            backdrop_size: size,
            frost: HashMap::new(),
        });
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        if !self.theme_applied {
            Self::apply_theme(ctx);
            // Claim keyboard focus at launch: apps started from a
            // terminal otherwise leave key events with the terminal
            // until the window is clicked — "the keys don't work"
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
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
            self.glide = p.glide;
            self.sub = p.sub;
            self.osc2_wave = p.osc2_wave;
            self.osc2_pitch = p.osc2_pitch;
            self.osc2_level = p.osc2_level;
            self.osc3_wave = p.osc3_wave;
            self.osc3_pitch = p.osc3_pitch;
            self.osc3_level = p.osc3_level;
            self.circuit = p.circuit;
            self.key_track = p.key_track;
            self.osc_fm = p.osc_fm;
            self.sync = p.sync;
            self.ring = p.ring;
            self.pulse_width = p.pulse_width;
            self.lfo_rate = p.lfo_rate;
            self.lfo_shape = p.lfo_shape;
            self.lfo_pitch = p.lfo_pitch;
            self.lfo_filter = p.lfo_filter;
            self.lfo_pwm = p.lfo_pwm;
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
            self.bd_level = p.bd_level;
            self.bd_tune = p.bd_tune;
            self.bd_attack = p.bd_attack;
            self.bd_decay = p.bd_decay;
            self.bd_sweep = p.bd_sweep;
            self.bd_drive = p.bd_drive;
            self.sd_level = p.sd_level;
            self.sd_tune = p.sd_tune;
            self.sd_tone = p.sd_tone;
            self.sd_snappy = p.sd_snappy;
            self.sd_decay = p.sd_decay;
            self.rs_level = p.rs_level;
            self.rs_tune = p.rs_tune;
            self.cp_level = p.cp_level;
            self.cp_decay = p.cp_decay;
            self.hh_level = p.hh_level;
            self.hh_tune = p.hh_tune;
            self.hh_metal = p.hh_metal;
            self.ch_decay = p.ch_decay;
            self.oh_decay = p.oh_decay;
            self.dr_drive = p.dr_drive;
            self.current_octave = p.ui_octave.round() as i32;
            self.notes_active = vm.held_note_states().iter().any(|&held| held);

            // The sky listens, slowly: loudness and filter openness ease in
            // over seconds, and cloud drift accelerates as an integral so
            // speed changes never jump
            let rms = {
                let n = vm.scope.len().max(1);
                let sum: f32 = vm.scope.iter().rev().take(512).map(|s| s * s).sum();
                (sum / n.min(512) as f32).sqrt()
            };
            let target_energy = (rms * 5.0).clamp(0.0, 1.0);
            let target_bright = ((p.cutoff / 20.0).ln() / (1000.0f32).ln()).clamp(0.0, 1.0);
            self.mood_energy += (target_energy - self.mood_energy) * 0.012;
            self.mood_bright += (target_bright - self.mood_bright) * 0.008;
        }
        let dt = ctx.input(|i| i.stable_dt).min(0.05);
        self.sky_phase += dt * (0.008 + self.mood_energy * 0.030);

        // The sky is alive: repaint at display cadence
        ctx.request_repaint_after(std::time::Duration::from_millis(16));

        let time = ctx.input(|i| i.time) as f32;
        TIME_BITS.store(time.to_bits() as u64, AtomicOrdering::Relaxed);

        // Backdrop on the panels' shared layer, before they run: the WGSL
        // sky when the GPU pipeline is live, the baked image otherwise.
        // Glass panes paint here too, from last frame's rects, so they are
        // always under the controls.
        if GPU_ON.load(AtomicOrdering::Relaxed) {
            let mood = [self.mood_energy, self.mood_bright, self.sky_phase];
            let painter = ctx.layer_painter(egui::LayerId::background());
            painter.add(aurora_gpu::sky_shape(ctx.screen_rect(), time, mood));
            let rects: Vec<Rect> = std::mem::take(&mut *GLASS_RECTS.lock());
            for (i, rect) in rects.into_iter().enumerate() {
                let slot = (i as u32 + 1) % 64;
                painter.add(aurora_gpu::glass_shape(
                    rect,
                    ctx.screen_rect(),
                    time,
                    12.0,
                    mood,
                    slot,
                ));
            }
        } else if let Some(tex) = &self.textures {
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
                egui::Frame::NONE
                    .inner_margin(egui::Margin {
                        left: 86,
                        right: 20,
                        top: 12,
                        bottom: 10,
                    }),
            )
            .show(ctx, |ui| self.draw_header(ui));

        egui::TopBottomPanel::bottom("keyboard")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                left: 20,
                right: 20,
                top: 9,
                bottom: 10,
            }))
            .show(ctx, |ui| {
                // Graphite shelf under the keys, painted after layout with
                // an unclipped painter so it reaches the window edges
                let screen = ui.ctx().screen_rect();
                let painter = ui.painter().with_clip_rect(screen);
                let bg_idx = painter.add(Shape::Noop);
                self.draw_keyboard(ui);
                let core = ui.min_rect();
                let shelf = Rect::from_min_max(
                    pos2(screen.left(), core.top() - 9.5),
                    pos2(screen.right(), screen.bottom()),
                );
                painter.set(bg_idx, Shape::Vec(rail_shapes(shelf)));
            });

        let mut tex = self.textures.take();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                left: 20,
                right: 20,
                top: 8,
                bottom: 8,
            }))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(11.0, 7.0);
                self.draw_preset_strip(ui);
                let top = egui::Layout::left_to_right(egui::Align::TOP);
                let full = ui.available_width();
                self.draw_oscillator_card(ui, tex.as_mut(), Some(full + 28.0));
                ui.with_layout(top, |ui| {
                    self.draw_envelope_card(ui, tex.as_mut(), None);
                    self.draw_filter_card(ui, tex.as_mut(), None);
                    let rest = ui.available_width();
                    self.draw_filter_env_card(ui, tex.as_mut(), Some(rest));
                });
                ui.with_layout(top, |ui| {
                    self.draw_lfo_card(ui, tex.as_mut(), None);
                    let rest = ui.available_width();
                    self.draw_effects_card(ui, tex.as_mut(), Some(rest));
                });
                self.draw_rhythm_card(ui, tex.as_mut(), Some(full + 28.0));
                self.draw_scope(ui);
            });
        self.textures = tex;
    }

    /// Preset strip, in the spirit of the Minitmoog's preset panel
    /// (US 3,981,218): one click retunes every functional block at once.
    /// Patches apply live, so you can morph a held chord between them.
    fn draw_preset_strip(&mut self, ui: &mut egui::Ui) {
        let height = 26.0;
        let (bar, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(bar, CornerRadius::same(8), INSET);

        painter.text(
            pos2(bar.left() + 12.0, bar.center().y),
            Align2::LEFT_CENTER,
            tracked("patch"),
            FontId::proportional(9.0),
            TXT_LOW,
        );

        let mut x = bar.left() + 72.0;
        for (i, (name, text)) in crate::patch::FACTORY.iter().enumerate() {
            let w = 22.0 + name.len() as f32 * 7.2;
            let cell = Rect::from_min_size(pos2(x, bar.top() + 4.0), vec2(w, height - 8.0));
            let response = ui.interact(cell, ui.id().with(("preset", i)), Sense::click());
            let selected = self.active_patch == Some(i);
            if response.clicked() && !selected {
                if crate::patch::apply(&mut self.voice_manager.lock(), text).is_ok() {
                    self.active_patch = Some(i);
                }
            }
            if selected {
                gloss_fill(&painter, cell, 6.0);
            }
            let color = if selected {
                CYAN_BRIGHT
            } else if response.hovered() {
                TXT
            } else {
                TXT_MID
            };
            painter.text(
                cell.center(),
                Align2::CENTER_CENTER,
                *name,
                FontId::proportional(10.5),
                color,
            );
            x += w + 5.0;
        }

        // SAVE: snapshot the current knobs to patches/user-N.patch
        let w = 54.0;
        let cell = Rect::from_min_size(
            pos2(bar.right() - w - 6.0, bar.top() + 4.0),
            vec2(w, height - 8.0),
        );
        let response = ui.interact(cell, ui.id().with("preset-save"), Sense::click());
        if response.clicked() {
            let params = self.voice_manager.lock().params;
            match crate::patch::save_user_patch(&params) {
                Ok(path) => println!("Saved patch to {path}"),
                Err(e) => eprintln!("Could not save patch: {e}"),
            }
        }
        let color = if response.hovered() { TOUCH_HI } else { TOUCH };
        painter.rect_stroke(
            cell,
            CornerRadius::same(6),
            Stroke::new(1.0, color),
            egui::StrokeKind::Inside,
        );
        painter.text(
            cell.center(),
            Align2::CENTER_CENTER,
            tracked("save"),
            FontId::proportional(9.5),
            color,
        );

        painter.rect_stroke(
            bar,
            CornerRadius::same(8),
            Stroke::new(1.0, HAIRLINE),
            egui::StrokeKind::Inside,
        );
    }

    /// Wordmark on the walnut top rail, octave stepper right.
    fn draw_header(&mut self, ui: &mut egui::Ui) {
        let bg_idx = ui
            .painter()
            .with_clip_rect(ui.ctx().screen_rect())
            .add(Shape::Noop);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(tracked("Patina"))
                    .size(14.0)
                    .strong()
                    .color(Color32::from_rgb(0xe8, 0xf2, 0xf5)),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if step_button(ui, "+").on_hover_text("octave up · arrow-up or +").clicked() {
                    self.shift_octave(1);
                }
                let (chip, chip_resp) = ui.allocate_exact_size(vec2(58.0, 24.0), Sense::hover());
                chip_resp.on_hover_text("octave · arrow keys or + / -");
                let painter = ui.painter();
                painter.rect_filled(chip, CornerRadius::same(6), INSET);
                painter.rect_stroke(chip, CornerRadius::same(6), Stroke::new(1.0, WELL_LINE), egui::StrokeKind::Inside);
                painter.text(
                    chip.center(),
                    Align2::CENTER_CENTER,
                    format!("OCT {}", self.current_octave),
                    FontId::monospace(10.5),
                    CYAN,
                );
                if step_button(ui, "-").on_hover_text("octave down · arrow-down or -").clicked() {
                    self.shift_octave(-1);
                }
            });
        });
        ui.add_space(10.0);
        let screen = ui.ctx().screen_rect();
        let core = ui.min_rect().expand2(vec2(0.0, 12.0));
        let rail = Rect::from_min_max(
            pos2(screen.left(), screen.top()),
            pos2(screen.right(), core.bottom()),
        );
        ui.painter()
            .with_clip_rect(screen)
            .set(bg_idx, Shape::Vec(rail_shapes(rail)));
    }

    fn draw_oscillator_card(&mut self, ui: &mut egui::Ui, tex: Option<&mut Textures>, fill: Option<f32>) {
        card(ui, "Oscillator", tex, fill, |ui| {
            ui.horizontal(|ui| {
                ui.label(legend("osc 1"));
                ui.vertical(|ui| {
                    ui.add_space(10.0);
                    if waveform_selector(ui, "osc1wave", &mut self.waveform) {
                        Param::WaveformSel
                            .apply(&mut self.voice_manager.lock(), self.waveform as u8 as f32);
                    }
                });
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Level",
                    Param::Volume,
                    &mut self.volume,
                    0.5,
                    fmt_pct,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Detune",
                    Param::Detune,
                    &mut self.detune,
                    7.0,
                    |v| {
                    format!("{:.0} ct", v)
                },
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Sub",
                    Param::SubLevel,
                    &mut self.sub,
                    0.0,
                    fmt_pct,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Noise",
                    Param::NoiseLevel,
                    &mut self.noise,
                    0.0,
                    fmt_pct,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Width",
                    Param::PulseWidth,
                    &mut self.pulse_width,
                    0.5,
                    fmt_pct,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Glide",
                    Param::Glide,
                    &mut self.glide,
                    0.0,
                    |v| {
                    if v < 0.001 {
                        "off".into()
                    } else {
                        fmt_time(v)
                    }
                },
                );
            });
            // The other two oscillator sections: a voice is three
            // independent oscillators (waveform / interval / level each),
            // not three clones
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                for which in 1..=2usize {
                    ui.label(legend(if which == 1 { "osc 2" } else { "osc 3" }));
                    let (wave, pitch, level) = if which == 1 {
                        (&mut self.osc2_wave, &mut self.osc2_pitch, &mut self.osc2_level)
                    } else {
                        (&mut self.osc3_wave, &mut self.osc3_pitch, &mut self.osc3_level)
                    };
                    let id = if which == 1 { "osc2wave" } else { "osc3wave" };
                    ui.vertical(|ui| {
                        ui.add_space(10.0);
                        if waveform_selector(ui, id, wave) {
                            let p = if which == 1 { Param::Osc2Wave } else { Param::Osc3Wave };
                            p.apply(&mut self.voice_manager.lock(), *wave as u8 as f32);
                        }
                    });
                    let (p_level, p_pitch) = if which == 1 {
                        (Param::Osc2Level, Param::Osc2Pitch)
                    } else {
                        (Param::Osc3Level, Param::Osc3Pitch)
                    };
                    param_knob(ui, &self.voice_manager, "Level", p_level, level, 0.72, fmt_pct);
                    param_knob(
                        ui,
                        &self.voice_manager,
                        "Pitch",
                        p_pitch,
                        pitch,
                        if which == 1 { 0.0 } else { -12.0 },
                        |v| format!("{v:+.0} st"),
                    );
                    if which == 1 {
                        ui.add_space(10.0);
                    }
                }
                ui.add_space(10.0);
                ui.label(legend("circuit"));
                let circ_sel = if self.circuit == CircuitModel::Arp { 1 } else { 0 };
                if let Some(i) = segmented(ui, "circuit", &["MOOG", "ARP"], circ_sel) {
                    self.circuit = if i == 1 { CircuitModel::Arp } else { CircuitModel::Moog };
                    Param::CircuitSel.apply(&mut self.voice_manager.lock(), i as f32);
                }
                param_knob(ui, &self.voice_manager, "FM", Param::OscFm, &mut self.osc_fm, 0.0, fmt_pct);
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Ring",
                    Param::RingAmount,
                    &mut self.ring,
                    0.0,
                    fmt_pct,
                );
                if let Some(i) =
                    segmented(ui, "sync", &["FREE", "SYNC"], if self.sync { 1 } else { 0 })
                {
                    self.sync = i == 1;
                    Param::SyncSel.apply(&mut self.voice_manager.lock(), i as f32);
                }
            });
        });
    }

    fn draw_envelope_card(&mut self, ui: &mut egui::Ui, tex: Option<&mut Textures>, fill: Option<f32>) {
        card(ui, "Envelope", tex, fill, |ui| {
            ui.horizontal(|ui| {
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Attack",
                    Param::Attack,
                    &mut self.attack,
                    0.1,
                    fmt_time,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Decay",
                    Param::Decay,
                    &mut self.decay,
                    0.1,
                    fmt_time,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Sustain",
                    Param::Sustain,
                    &mut self.sustain,
                    0.7,
                    fmt_pct,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Release",
                    Param::Release,
                    &mut self.release,
                    0.2,
                    fmt_time,
                );
            });
        });
    }

    fn draw_filter_card(&mut self, ui: &mut egui::Ui, tex: Option<&mut Textures>, fill: Option<f32>) {
        card(ui, "Filter", tex, fill, |ui| {
            ui.horizontal(|ui| {
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Cutoff",
                    Param::Cutoff,
                    &mut self.filter_cutoff,
                    15000.0,
                    fmt_hz,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Reso",
                    Param::Resonance,
                    &mut self.filter_resonance,
                    0.0,
                    fmt_x,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Drive",
                    Param::Drive,
                    &mut self.filter_drive,
                    1.0,
                    fmt_x,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Shape",
                    Param::Saturation,
                    &mut self.filter_saturation,
                    1.0,
                    fmt_x,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Hi-Pass",
                    Param::HpfCutoff,
                    &mut self.hpf_cutoff,
                    16.0,
                    fmt_hz,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Track",
                    Param::KeyTrack,
                    &mut self.key_track,
                    0.4,
                    fmt_pct,
                );
            });
        });
    }

    fn draw_filter_env_card(&mut self, ui: &mut egui::Ui, tex: Option<&mut Textures>, fill: Option<f32>) {
        card(ui, "Filter Env", tex, fill, |ui| {
            ui.horizontal(|ui| {
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Amount",
                    Param::FilterEnvAmount,
                    &mut self.fenv_amount,
                    0.0,
                    |v| {
                    format!("{:+.1} oct", v)
                },
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Attack",
                    Param::FilterAttack,
                    &mut self.fenv_attack,
                    0.005,
                    fmt_time,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Decay",
                    Param::FilterDecay,
                    &mut self.fenv_decay,
                    0.3,
                    fmt_time,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Sustain",
                    Param::FilterSustain,
                    &mut self.fenv_sustain,
                    0.0,
                    fmt_pct,
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Release",
                    Param::FilterRelease,
                    &mut self.fenv_release,
                    0.3,
                    fmt_time,
                );
            });
        });
    }

    fn draw_lfo_card(&mut self, ui: &mut egui::Ui, tex: Option<&mut Textures>, fill: Option<f32>) {
        card(ui, "LFO", tex, fill, |ui| {
            ui.horizontal(|ui| {
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Rate",
                    Param::LfoRate,
                    &mut self.lfo_rate,
                    1.0,
                    |v| {
                    format!("{:.2} Hz", v)
                },
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Shape",
                    Param::LfoShape,
                    &mut self.lfo_shape,
                    0.5,
                    |v| {
                    if v < 0.15 {
                        "saw".into()
                    } else if v > 0.85 {
                        "ramp".into()
                    } else if (0.4..=0.6).contains(&v) {
                        "tri".into()
                    } else {
                        format!("{:.2}", v)
                    }
                },
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Pitch",
                    Param::LfoPitch,
                    &mut self.lfo_pitch,
                    0.0,
                    |v| {
                    format!("{:.0} ct", v)
                },
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "Filter",
                    Param::LfoFilter,
                    &mut self.lfo_filter,
                    0.0,
                    |v| {
                    format!("{:.2} oct", v)
                },
                );
                param_knob(
                    ui,
                    &self.voice_manager,
                    "PWM",
                    Param::LfoPwm,
                    &mut self.lfo_pwm,
                    0.0,
                    fmt_pct,
                );
            });
        });
    }

    fn draw_effects_card(&mut self, ui: &mut egui::Ui, tex: Option<&mut Textures>, fill: Option<f32>) {
        card(ui, "Effects", tex, fill, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(sublegend("Chorus"));
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
                        Param::ChorusModeSel.apply(&mut self.voice_manager.lock(), i as f32);
                    }
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        param_knob(
                            ui,
                            &self.voice_manager,
                            "Rate",
                            Param::ChorusRate,
                            &mut self.chorus_rate,
                            0.5,
                            |v| {
                            format!("{:.1} Hz", v)
                        },
                        );
                        param_knob(
                            ui,
                            &self.voice_manager,
                            "Depth",
                            Param::ChorusDepth,
                            &mut self.chorus_depth,
                            0.3,
                            fmt_pct,
                        );
                    });
                });
                vseparator(ui, 150.0);
                ui.vertical(|ui| {
                    ui.label(sublegend("Reverb"));
                    ui.add_space(34.0);
                    ui.horizontal(|ui| {
                        param_knob(
                            ui,
                            &self.voice_manager,
                            "Decay",
                            Param::ReverbDecay,
                            &mut self.reverb_decay,
                            0.5,
                            fmt_pct,
                        );
                        param_knob(
                            ui,
                            &self.voice_manager,
                            "Mix",
                            Param::ReverbWet,
                            &mut self.reverb_wet,
                            0.5,
                            fmt_pct,
                        );
                        param_knob(
                            ui,
                            &self.voice_manager,
                            "Spring",
                            Param::SpringWet,
                            &mut self.spring,
                            0.0,
                            fmt_pct,
                        );
                    });
                });
                vseparator(ui, 150.0);
                ui.vertical(|ui| {
                    ui.label(sublegend("Tape"));
                    ui.add_space(34.0);
                    ui.horizontal(|ui| {
                        param_knob(
                            ui,
                            &self.voice_manager,
                            "Wow",
                            Param::TapeWow,
                            &mut self.tape_wow,
                            0.0,
                            fmt_pct,
                        );
                        param_knob(
                            ui,
                            &self.voice_manager,
                            "Flutter",
                            Param::TapeFlutter,
                            &mut self.tape_flutter,
                            0.0,
                            fmt_pct,
                        );
                        param_knob(
                            ui,
                            &self.voice_manager,
                            "Drive",
                            Param::TapeDrive,
                            &mut self.tape_drive,
                            0.0,
                            fmt_pct,
                        );
                        param_knob(
                            ui,
                            &self.voice_manager,
                            "Age",
                            Param::TapeAge,
                            &mut self.tape_age,
                            0.0,
                            fmt_pct,
                        );
                    });
                });
                vseparator(ui, 150.0);
                ui.vertical(|ui| {
                    ui.label(sublegend("Fuzz"));
                    ui.add_space(34.0);
                    param_knob(
                        ui,
                        &self.voice_manager,
                        "Germanium",
                        Param::FuzzAmount,
                        &mut self.fuzz,
                        0.0,
                        fmt_pct,
                    );
                });
            });
        });
    }

    /// The rhythm section: the 909 board's 21 knobs in one dense row
    /// (voice names ride the knob labels; the pads live on the keyboard
    /// shelf, under the right hand where the QWERTY cluster is).
    fn draw_rhythm_card(&mut self, ui: &mut egui::Ui, tex: Option<&mut Textures>, fill: Option<f32>) {
        card(ui, "Rhythm 909", tex, fill, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            // Full-word group headers; the pictographic glyphs live only
            // on the pads by the keyboard
            let vm = self.voice_manager.clone();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(sublegend("Kick"));
                    ui.horizontal(|ui| {
                        param_knob_sm(ui, &vm, "Level", Param::BdLevel, &mut self.bd_level, 0.8);
                        param_knob_sm(ui, &vm, "Tune", Param::BdTune, &mut self.bd_tune, 0.35);
                        param_knob_sm(ui, &vm, "Attack", Param::BdAttack, &mut self.bd_attack, 0.5);
                        param_knob_sm(ui, &vm, "Decay", Param::BdDecay, &mut self.bd_decay, 0.45);
                        param_knob_sm(ui, &vm, "Sweep", Param::BdSweep, &mut self.bd_sweep, 0.5);
                        param_knob_sm(ui, &vm, "Drive", Param::BdDrive, &mut self.bd_drive, 0.25);
                    });
                });
                vseparator(ui, 74.0);
                ui.vertical(|ui| {
                    ui.label(sublegend("Snare"));
                    ui.horizontal(|ui| {
                        param_knob_sm(ui, &vm, "Level", Param::SdLevel, &mut self.sd_level, 0.75);
                        param_knob_sm(ui, &vm, "Tune", Param::SdTune, &mut self.sd_tune, 0.4);
                        param_knob_sm(ui, &vm, "Tone", Param::SdTone, &mut self.sd_tone, 0.5);
                        param_knob_sm(ui, &vm, "Snappy", Param::SdSnappy, &mut self.sd_snappy, 0.6);
                        param_knob_sm(ui, &vm, "Decay", Param::SdDecay, &mut self.sd_decay, 0.5);
                    });
                });
                vseparator(ui, 74.0);
                ui.vertical(|ui| {
                    ui.label(sublegend("Rim"));
                    ui.horizontal(|ui| {
                        param_knob_sm(ui, &vm, "Level", Param::RsLevel, &mut self.rs_level, 0.7);
                        param_knob_sm(ui, &vm, "Tune", Param::RsTune, &mut self.rs_tune, 0.5);
                    });
                });
                vseparator(ui, 74.0);
                ui.vertical(|ui| {
                    ui.label(sublegend("Clap"));
                    ui.horizontal(|ui| {
                        param_knob_sm(ui, &vm, "Level", Param::CpLevel, &mut self.cp_level, 0.75);
                        param_knob_sm(ui, &vm, "Decay", Param::CpDecay, &mut self.cp_decay, 0.5);
                    });
                });
                vseparator(ui, 74.0);
                ui.vertical(|ui| {
                    ui.label(sublegend("Hi-Hat"));
                    ui.horizontal(|ui| {
                        param_knob_sm(ui, &vm, "Level", Param::HhLevel, &mut self.hh_level, 0.7);
                        param_knob_sm(ui, &vm, "Tune", Param::HhTune, &mut self.hh_tune, 0.5);
                        param_knob_sm(ui, &vm, "Metal", Param::HhMetal, &mut self.hh_metal, 0.65);
                        param_knob_sm(ui, &vm, "Closed", Param::ChDecay, &mut self.ch_decay, 0.35);
                        param_knob_sm(ui, &vm, "Open", Param::OhDecay, &mut self.oh_decay, 0.5);
                    });
                });
                vseparator(ui, 74.0);
                ui.vertical(|ui| {
                    ui.label(sublegend("Bus"));
                    param_knob_sm(ui, &vm, "Drive", Param::DrumDrive, &mut self.dr_drive, 0.0);
                });
            });
        });
    }

    /// Output oscilloscope. The one place the interface goes cyan: the
    /// signal itself, trigger-stabilized on a rising zero crossing.
    fn draw_scope(&self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let height = ui.available_height().clamp(48.0, 170.0);
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(10), INSET);
        painter.rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0, HAIRLINE), egui::StrokeKind::Inside);

        let inner = rect.shrink2(vec2(14.0, 10.0));
        painter.line_segment(
            [
                pos2(inner.left(), inner.center().y),
                pos2(inner.right(), inner.center().y),
            ],
            Stroke::new(1.0, WELL_LINE),
        );
        painter.text(
            pos2(rect.left() + 12.0, rect.top() + 6.0),
            Align2::LEFT_TOP,
            tracked("out"),
            FontId::proportional(8.5),
            WELL_TXT,
        );
        if self.notes_active {
            painter.circle_filled(pos2(rect.left() + 40.0, rect.top() + 10.5), 2.0, CYAN);
        }

        let samples: Vec<f32> = self.voice_manager.lock().scope.iter().copied().collect();
        if samples.len() < 64 {
            return;
        }

        // Trigger on the rising zero crossing with the steepest slope in
        // the first half — locks onto the fundamental instead of whichever
        // harmonic crosses first, so the trace holds still
        let window = samples.len() / 2;
        let mut start = 0;
        let mut best_slope = 0.0f32;
        for i in 1..window {
            if samples[i - 1] <= 0.0 && samples[i] > 0.0 {
                let slope = samples[i] - samples[i - 1];
                if slope > best_slope {
                    best_slope = slope;
                    start = i;
                }
            }
        }

        // Accurate rendering: per-pixel-column min/max envelope, so no
        // peak between columns is ever lost to subsampling, plus the mean
        // as a crisp centre trace
        let cols = inner.width().floor().max(32.0) as usize;
        let y_of = |s: f32| inner.center().y - s.clamp(-1.0, 1.0) * inner.height() * 0.5;
        let mut band = Vec::with_capacity(cols * 2);
        let mut mean_pts = Vec::with_capacity(cols);
        for cx in 0..cols {
            let s0 = start + cx * window / cols;
            let s1 = (start + (cx + 1) * window / cols).max(s0 + 1);
            let (mut lo, mut hi, mut sum) = (f32::MAX, f32::MIN, 0.0f32);
            for s in &samples[s0..s1.min(samples.len())] {
                lo = lo.min(*s);
                hi = hi.max(*s);
                sum += *s;
            }
            let x = inner.left() + inner.width() * cx as f32 / (cols - 1) as f32;
            band.push((x, y_of(hi)));
            mean_pts.push(pos2(x, y_of(sum / (s1 - s0) as f32)));
            // Envelope must cover at least one pixel so silence stays visible
            let (top_y, bot_y) = (y_of(hi), y_of(lo));
            let bot_y = if bot_y - top_y < 1.0 { top_y + 1.0 } else { bot_y };
            band.push((x, bot_y));
        }
        // Filled min/max envelope as a translucent band
        let mut mesh = Mesh::default();
        let band_color = Color32::from_rgba_unmultiplied(0x6f, 0xe3, 0xf2, 70);
        for (i, chunk) in band.chunks_exact(2).enumerate() {
            let ((x, ty), (_, by)) = (chunk[0], chunk[1]);
            mesh.colored_vertex(pos2(x, ty), band_color);
            mesh.colored_vertex(pos2(x, by), band_color);
            if i > 0 {
                let b = (i * 2) as u32;
                mesh.add_triangle(b - 2, b - 1, b);
                mesh.add_triangle(b, b - 1, b + 1);
            }
        }
        painter.add(Shape::mesh(mesh));
        painter.add(Shape::line(
            mean_pts.clone(),
            Stroke::new(3.0, Color32::from_rgba_unmultiplied(0x6f, 0xe3, 0xf2, 40)),
        ));
        painter.add(Shape::line(mean_pts, Stroke::new(1.3, CYAN)));
    }

    // -----------------------------------------------------------------------
    // Keyboard
    // -----------------------------------------------------------------------

    /// Computer-key hint for a note, if the note is reachable from the
    /// QWERTY mapping at the current octave. The whole keyboard is an
    /// instrument now: the Z row (with home-row sharps) is the lower
    /// manual, the Q row (with number-row sharps) runs a full octave and
    /// a fifth above it, and the right-hand cluster K L ; ' , . / is the
    /// 909 pad grid (drawn beside the piano, not on it).
    fn key_hint(&self, visual_octave: usize, key_index: usize) -> Option<&'static str> {
        const LOWER: [&str; 12] = ["Z", "S", "X", "D", "C", "V", "G", "B", "H", "N", "J", "M"];
        const UPPER1: [&str; 12] = ["Q", "2", "W", "3", "E", "R", "5", "T", "6", "Y", "7", "U"];
        const UPPER2: [Option<&str>; 12] = [
            Some("I"), Some("9"), Some("O"), Some("0"), Some("P"), Some("["),
            Some("="), Some("]"), None, None, None, None,
        ];
        match visual_octave {
            0 => Some(LOWER[key_index]),
            1 => Some(UPPER1[key_index]),
            2 => UPPER2[key_index],
            _ => None,
        }
    }

    fn draw_keyboard(&mut self, ui: &mut egui::Ui) {
        let available_width = ui.available_width();
        let white_key_height = 94.0;
        // The shelf is split: piano on the left, the 909 pad grid under
        // the right hand — exactly the way the QWERTY layer is arranged
        let pads_width = 300.0f32.min(available_width * 0.26);
        let gap = 12.0;
        let piano_width = available_width - pads_width - gap;
        let white_key_width = piano_width / (7.0 * OCTAVES as f32);
        let black_key_width = white_key_width * 0.6;
        let black_key_height = white_key_height * 0.6;

        let (full_rect, response) = ui.allocate_exact_size(
            Vec2::new(available_width, white_key_height),
            egui::Sense::click_and_drag(),
        );
        // The piano is pointer-only: surrendering focus kills egui's focus
        // ring and keeps arrow keys free for octave shifting
        response.surrender_focus();
        let rect = Rect::from_min_size(full_rect.min, Vec2::new(piano_width, white_key_height));
        let pads_rect = Rect::from_min_max(
            pos2(rect.right() + gap, full_rect.top()),
            full_rect.max,
        );
        self.handle_mouse_input(ui, rect, &response);
        self.draw_drum_pads(ui, pads_rect, &response);

        // Light keys from the engine's live voice state, so song playback,
        // MIDI, QWERTY, and mouse input all show up on the keyboard
        let key_states = self.voice_manager.lock().held_note_states();

        // Hover preview: tint the key under the pointer and show a hand
        // cursor, so the keyboard invites playing before the first click
        let hovered_note = if response.hovered() {
            ui.input(|i| i.pointer.hover_pos())
                .and_then(|pos| self.get_note_from_pointer(pos, rect))
        } else {
            None
        };
        if hovered_note.is_some() {
            ui.output_mut(|o| o.cursor_icon = CursorIcon::PointingHand);
        }

        let painter = ui.painter();
        painter.rect_filled(rect.expand(3.0), CornerRadius::same(6), INSET);

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
                    let rounding = CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: 3,
                        se: 3,
                    };
                    if pressed {
                        painter.rect_filled(
                            key_rect.expand(2.0),
                            rounding,
                            Color32::from_rgba_unmultiplied(0x2b, 0xc6, 0xe6, 50),
                        );
                    }
                    let hovered = hovered_note == Some(note) && !pressed;
                    painter.rect_filled(key_rect, rounding, if pressed { TOUCH } else { IVORY });
                    if hovered {
                        painter.rect_filled(
                            key_rect,
                            rounding,
                            Color32::from_rgba_unmultiplied(0x2b, 0xc6, 0xe6, 34),
                        );
                    }
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
                        if pressed { TOUCH_DEEP } else { IVORY_SHADE },
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
                                TOUCH_INK
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
                                TOUCH_INK
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
                    let rounding = CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: 3,
                        se: 3,
                    };
                    let hovered = hovered_note == Some(note) && !pressed;
                    painter.rect_filled(
                        key_rect,
                        rounding,
                        if pressed { TOUCH_DEEP } else { EBONY },
                    );
                    if hovered {
                        painter.rect_filled(
                            key_rect,
                            rounding,
                            Color32::from_rgba_unmultiplied(0x2b, 0xc6, 0xe6, 60),
                        );
                    }
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
                    painter.rect_stroke(key_rect, rounding, Stroke::new(1.0, INSET), egui::StrokeKind::Inside);

                    if let Some(hint) = self.key_hint(visual_octave, key_index) {
                        painter.text(
                            pos2(key_rect.center().x, key_rect.max.y - 12.0),
                            Align2::CENTER_CENTER,
                            hint,
                            FontId::proportional(9.5),
                            if pressed { TOUCH_INK } else { TXT_LOW },
                        );
                    }
                }
            }
        }
    }

    /// The 909 pad grid beside the piano: seven strike pads mirroring the
    /// K L ; ' / , . / QWERTY cluster, lit by the board's actual VCA
    /// envelopes. Click velocity follows strike depth, like the keys.
    fn draw_drum_pads(&mut self, ui: &egui::Ui, rect: Rect, response: &egui::Response) {
        let activity = {
            let vm = self.voice_manager.lock();
            vm.drums.activity()
        };
        self.ghost_flash *= 0.88;
        let painter = ui.painter();
        painter.rect_filled(rect.expand(3.0), CornerRadius::same(6), INSET);

        let gap = 5.0;
        let row_h = (rect.height() - gap) / 2.0 - 2.0;
        let top_w = (rect.width() - 3.0 * gap) / 4.0;
        let bot_w = (rect.width() - 2.0 * gap) / 3.0;

        let pointer = ui.input(|i| i.pointer.interact_pos());
        let down = response.is_pointer_button_down_on();
        let mut struck: Option<usize> = None;

        let mut draw_pad = |painter: &egui::Painter,
                            pad_rect: Rect,
                            glyph: usize,
                            ghost: bool,
                            hint: &str,
                            act: f32,
                            idx: usize| {
            let hovered = pointer.map_or(false, |p| pad_rect.contains(p));
            let lit = act.min(1.0);
            // Graphite pad, sinking as its voice speaks
            let sink = lit * 1.5;
            let r = Rect::from_min_max(pad_rect.min + vec2(0.0, sink), pad_rect.max);
            painter.rect_filled(r, CornerRadius::same(5), EBONY);
            if lit > 0.01 {
                painter.rect_filled(
                    r,
                    CornerRadius::same(5),
                    Color32::from_rgba_unmultiplied(0x2b, 0xc6, 0xe6, (lit * 110.0) as u8),
                );
            }
            if hovered {
                painter.rect_filled(
                    r,
                    CornerRadius::same(5),
                    Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 14),
                );
            }
            // Lit top face, like the ebony keys
            painter.add(gradient_quad(
                Rect::from_min_max(r.min + vec2(1.0, 0.0), pos2(r.right() - 1.0, r.top() + 12.0)),
                Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 26),
                Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 0),
            ));
            painter.rect_stroke(
                r,
                CornerRadius::same(5),
                Stroke::new(1.0, if lit > 0.3 { TOUCH } else { EBONY_EDGE }),
                egui::StrokeKind::Inside,
            );
            let glyph_rect = Rect::from_center_size(
                pos2(r.center().x, r.top() + r.height() * 0.42),
                vec2(24.0, r.height() * 0.55),
            );
            drum_glyph(
                painter,
                glyph_rect,
                glyph,
                if lit > 0.25 {
                    CYAN_BRIGHT
                } else {
                    Color32::from_rgb(0xd8, 0xdd, 0xe2)
                },
                ghost,
            );
            painter.text(
                pos2(r.center().x, r.bottom() - 8.0),
                Align2::CENTER_CENTER,
                hint,
                FontId::monospace(8.5),
                if lit > 0.25 { CYAN } else { WELL_TXT },
            );
            if hovered && down {
                struck = Some(idx);
            }
        };

        for (i, &(_, hint, _, _, act_idx, ghost)) in PAD_TOP.iter().enumerate() {
            let pad_rect = Rect::from_min_size(
                pos2(rect.left() + i as f32 * (top_w + gap), rect.top() + 2.0),
                vec2(top_w, row_h),
            );
            let act = if ghost { self.ghost_flash } else { activity[act_idx] };
            draw_pad(painter, pad_rect, act_idx, ghost, hint, act, i);
        }
        for (i, &(_, hint, _, _, act_idx, ghost)) in PAD_BOTTOM.iter().enumerate() {
            let pad_rect = Rect::from_min_size(
                pos2(
                    rect.left() + i as f32 * (bot_w + gap),
                    rect.top() + 2.0 + row_h + gap,
                ),
                vec2(bot_w, row_h),
            );
            draw_pad(painter, pad_rect, act_idx, ghost, hint, activity[act_idx], 4 + i);
        }

        // Strike on press edge; dragging across pads re-strikes, drummily
        match struck {
            Some(idx) if self.mouse_pad_down != Some(idx) => {
                let (name, _, _, base_vel, _, _) = if idx < 4 {
                    PAD_TOP[idx]
                } else {
                    PAD_BOTTOM[idx - 4]
                };
                let depth = pointer
                    .map(|p| ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0))
                    .unwrap_or(0.7);
                let vel = (base_vel * (0.6 + 0.6 * depth)).clamp(0.1, 1.0);
                if let Some(note) = crate::drums::note_from_name(name) {
                    self.voice_manager
                        .lock()
                        .note_on_channel(note, vel, crate::drums::DRUM_CHANNEL);
                    if idx == 3 {
                        self.ghost_flash = 1.0;
                    }
                }
                self.mouse_pad_down = Some(idx);
            }
            Some(_) => {}
            None => self.mouse_pad_down = None,
        }
    }

    fn get_note_from_pointer(&self, pos: egui::Pos2, rect: Rect) -> Option<u8> {
        // The pointer may be over the pad grid to the right of the piano
        if pos.x >= rect.right() || pos.y < rect.top() {
            return None;
        }
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
        // The two manuals: Z row + home-row sharps (one octave), Q row +
        // number-row sharps (an octave and a fifth, one octave up)
        const KEYS: [Key; 32] = [
            Key::Z, Key::S, Key::X, Key::D, Key::C, Key::V, Key::G, Key::B, Key::H, Key::N, Key::J, Key::M,
            Key::Q, Key::Num2, Key::W, Key::Num3, Key::E, Key::R, Key::Num5, Key::T, Key::Num6, Key::Y, Key::Num7, Key::U,
            Key::I, Key::Num9, Key::O, Key::Num0, Key::P, Key::OpenBracket, Key::Equals, Key::CloseBracket,
        ];

        // Octave switching lives on the arrows alone now — every other
        // key is an instrument
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
                    self.play_note(note, 0.85);
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

        // The right-hand 909 pads: K L ; ' over , . / — one-shots on the
        // drum channel, Shift is the accent line
        let shift = ctx.input(|i| i.modifiers.shift);
        for &(name, _, key, base_vel, _, _) in PAD_TOP.iter().chain(PAD_BOTTOM.iter()) {
            if ctx.input(|i| i.key_pressed(key)) && !chord && !self.pressed_drum_keys.contains(&key)
            {
                self.pressed_drum_keys.insert(key);
                if let Some(note) = crate::drums::note_from_name(name) {
                    let vel = if shift { (base_vel + 0.3).min(1.0) } else { base_vel };
                    self.voice_manager
                        .lock()
                        .note_on_channel(note, vel, crate::drums::DRUM_CHANNEL);
                    if key == Key::Quote {
                        self.ghost_flash = 1.0;
                    }
                }
            }
            if ctx.input(|i| i.key_released(key)) {
                self.pressed_drum_keys.remove(&key);
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
        let stale_pads: Vec<Key> = ctx.input(|i| {
            self.pressed_drum_keys
                .iter()
                .copied()
                .filter(|k| !i.keys_down.contains(k))
                .collect()
        });
        for key in stale_pads {
            self.pressed_drum_keys.remove(&key);
        }
    }

    /// Change octave like a performance switch: notes you are holding
    /// jump to the new register instead of dying (they could never
    /// re-trigger while held — egui ignores OS key repeats).
    fn shift_octave(&mut self, delta: i32) {
        let new_octave = (self.current_octave + delta).clamp(0, 8);
        if new_octave == self.current_octave {
            return;
        }
        let semis = (new_octave - self.current_octave) * 12;
        {
            let mut vm = self.voice_manager.lock();
            for note in self.pressed_keys.values_mut() {
                let moved = (*note as i32 + semis).clamp(0, 127) as u8;
                vm.note_off(*note);
                vm.note_on(moved, 0.9);
                *note = moved;
            }
        }
        // The mouse note re-triggers naturally next frame from the pointer
        if let Some(note) = self.active_mouse_note.take() {
            self.stop_note(note);
        }
        self.current_octave = new_octave;
        self.voice_manager
            .lock()
            .set_ui_octave(self.current_octave as f32);
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
                        // Strike position is velocity: the base of the key
                        // plays hard, the top plays soft — like a real key
                        // has leverage
                        let depth = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
                        let velocity = 0.45 + 0.55 * depth;
                        self.play_note(note, velocity);
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
            Key::I => 24, Key::Num9 => 25, Key::O => 26, Key::Num0 => 27, Key::P => 28,
            Key::OpenBracket => 29, Key::Equals => 30, Key::CloseBracket => 31,
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

    fn play_note(&mut self, note: u8, velocity: f32) {
        self.voice_manager.lock().note_on(note, velocity);
    }

    fn stop_note(&mut self, note: u8) {
        self.voice_manager.lock().note_off(note);
    }
}
