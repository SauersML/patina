use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CursorIcon, FontId, Key, Pos2, Rect, RichText, Rounding,
    Sense, Shape, Stroke, Vec2,
};
use eframe::egui::epaint::Mesh;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;

use crate::chorus::ChorusMode;
use crate::oscillator::Waveform;
use crate::voice_manager::VoiceManager;

const OCTAVES: usize = 3;
const WHITE_KEY_INDICES: [usize; 7] = [0, 2, 4, 5, 7, 9, 11];
const BLACK_KEY_INDICES: [usize; 5] = [1, 3, 6, 8, 10];

// ---------------------------------------------------------------------------
// Materials — old wood, aged brass, warm amber (the "old") against dark
// glass and phosphor-cyan electricity (the "new"). Everything is painted
// procedurally: gradient meshes, seeded grain, layered glow strokes.
// ---------------------------------------------------------------------------
const BG_TOP: Color32 = Color32::from_rgb(0x15, 0x16, 0x1b);
const BG_BOTTOM: Color32 = Color32::from_rgb(0x0b, 0x0c, 0x0f);
const BG_INSET: Color32 = Color32::from_rgb(0x08, 0x09, 0x0c);
const PANEL_EDGE: Color32 = Color32::from_rgb(0x2a, 0x2e, 0x38);
const HOVER: Color32 = Color32::from_rgb(0x2e, 0x33, 0x3e);

const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe4, 0xdd);
const TEXT_DIM: Color32 = Color32::from_rgb(0x8b, 0x92, 0xa0);

const ACCENT: Color32 = Color32::from_rgb(0xff, 0xb1, 0x4a);
const ACCENT_SOFT: Color32 = Color32::from_rgb(0x3a, 0x2f, 0x1d);
const ACCENT_PRESSED_SHADE: Color32 = Color32::from_rgb(0xd9, 0x8f, 0x2f);
const ACCENT_INK: Color32 = Color32::from_rgb(0x5c, 0x40, 0x12);

const ELECTRIC: Color32 = Color32::from_rgb(0x53, 0xe7, 0xff);
const ELECTRIC_DIM: Color32 = Color32::from_rgb(0x2f, 0x7c, 0x8a);

const WOOD_HI: Color32 = Color32::from_rgb(0x4a, 0x32, 0x1e);
const WOOD_LO: Color32 = Color32::from_rgb(0x27, 0x19, 0x0e);
const GRAIN_DARK: Color32 = Color32::from_rgba_premultiplied(0x14, 0x0c, 0x06, 90);
const GRAIN_LIGHT: Color32 = Color32::from_rgba_premultiplied(0x50, 0x38, 0x20, 60);

const BRASS_HI: Color32 = Color32::from_rgb(0xd9, 0xb4, 0x6a);
const BRASS_LO: Color32 = Color32::from_rgb(0x8a, 0x6a, 0x35);
const BRASS_EDGE: Color32 = Color32::from_rgb(0x54, 0x40, 0x1f);
const BRASS_INK: Color32 = Color32::from_rgb(0x2b, 0x1d, 0x0e);

const IVORY: Color32 = Color32::from_rgb(0xf1, 0xea, 0xd8);
const IVORY_SHADE: Color32 = Color32::from_rgb(0xd6, 0xcd, 0xb4);
const EBONY: Color32 = Color32::from_rgb(0x14, 0x15, 0x19);
const EBONY_EDGE: Color32 = Color32::from_rgb(0x32, 0x35, 0x3e);
const FELT: Color32 = Color32::from_rgb(0x6e, 0x1e, 0x27);

const SERIF: &str = "patina-serif";

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
    pressed_keys: HashSet<Key>,
    theme_applied: bool,
    serif_pending: bool,
    serif_loaded: bool,
    notes_active: bool,
    time: f64,
}

// ---------------------------------------------------------------------------
// Procedural material painting
// ---------------------------------------------------------------------------

fn hash01(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(2654435769);
    x ^= x >> 16;
    x = x.wrapping_mul(2246822519);
    x ^= x >> 13;
    (x & 0xffff) as f32 / 65535.0
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

/// Horizontal variant, for vignette edges.
fn gradient_quad_h(rect: Rect, left: Color32, right: Color32) -> Shape {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(2, 1, 3);
    Shape::mesh(mesh)
}

/// Walnut rail: gradient base plus seeded wavy grain strokes. The seed keeps
/// the grain identical frame to frame so it never shimmers.
fn wood_shapes(rect: Rect, seed: u32) -> Vec<Shape> {
    let mut shapes = vec![
        Shape::rect_filled(rect, 0.0, WOOD_LO),
        gradient_quad(rect, WOOD_HI, WOOD_LO),
    ];

    let n = ((rect.height() / 6.0) as i32).max(4);
    for i in 0..n {
        let h = hash01(seed.wrapping_add(i as u32).wrapping_mul(7919));
        let y = rect.top() + rect.height() * (i as f32 + 0.5) / n as f32 + (h - 0.5) * 5.0;
        let amp = 0.8 + h * 2.2;
        let phase = h * 21.0;
        let freq = 5.0 + h * 9.0;
        let color = if i % 3 == 0 { GRAIN_LIGHT } else { GRAIN_DARK };
        let points: Vec<Pos2> = (0..=40)
            .map(|k| {
                let t = k as f32 / 40.0;
                pos2(
                    rect.left() + t * rect.width(),
                    y + (t * freq + phase).sin() * amp,
                )
            })
            .collect();
        shapes.push(Shape::line(points, Stroke::new(0.7 + h * 0.9, color)));
    }

    // Bevel: lit top edge, shadowed bottom edge
    shapes.push(Shape::line_segment(
        [rect.left_top(), rect.right_top()],
        Stroke::new(1.0, Color32::from_rgba_premultiplied(0x6b, 0x50, 0x30, 70)),
    ));
    shapes.push(Shape::line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(2.0, Color32::from_rgba_premultiplied(0, 0, 0, 130)),
    ));
    shapes
}

/// A brass screw head with a randomly rotated slot.
fn screw_shapes(center: Pos2, radius: f32, seed: u32) -> Vec<Shape> {
    let angle = hash01(seed) * std::f32::consts::PI;
    let dir = vec2(angle.cos(), angle.sin()) * radius * 0.72;
    vec![
        Shape::circle_filled(
            center + vec2(0.6, 1.0),
            radius,
            Color32::from_rgba_premultiplied(0, 0, 0, 110),
        ),
        Shape::circle_filled(center, radius, BRASS_LO),
        Shape::circle_stroke(center, radius, Stroke::new(1.0, BRASS_EDGE)),
        Shape::circle_stroke(
            center - vec2(radius * 0.25, radius * 0.25),
            radius * 0.55,
            Stroke::new(1.0, Color32::from_rgba_premultiplied(0xd9, 0xb4, 0x6a, 140)),
        ),
        Shape::line_segment([center - dir, center + dir], Stroke::new(1.2, BRASS_INK)),
    ]
}

/// Dark translucent glass: tinted fill, top sheen gradient, bright inner
/// edge over a dark outer edge.
fn glass_shapes(rect: Rect, rounding: f32) -> Vec<Shape> {
    let sheen = Rect::from_min_max(
        rect.min + vec2(rounding, 2.0),
        pos2(rect.right() - rounding, rect.top() + rect.height() * 0.45),
    );
    vec![
        Shape::rect_stroke(
            rect.expand(1.0),
            Rounding::same(rounding + 1.0),
            Stroke::new(1.5, Color32::from_rgba_premultiplied(0, 0, 0, 140)),
        ),
        Shape::rect_filled(
            rect,
            Rounding::same(rounding),
            Color32::from_rgba_premultiplied(0x10, 0x14, 0x1a, 205),
        ),
        gradient_quad(
            sheen,
            Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 11),
            Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 0),
        ),
        Shape::rect_stroke(
            rect,
            Rounding::same(rounding),
            Stroke::new(1.0, Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 20)),
        ),
    ]
}

/// Small status lamp. `on` lamps glow electric cyan and breathe slowly.
fn draw_led(painter: &egui::Painter, center: Pos2, on: bool, time: f64, seed: f32) {
    if on {
        let pulse = 0.7 + 0.3 * ((time * 2.4 + seed as f64).sin() as f32 * 0.5 + 0.5);
        painter.circle_filled(
            center,
            5.5,
            Color32::from_rgba_premultiplied(0x53, 0xe7, 0xff, (36.0 * pulse) as u8),
        );
        painter.circle_filled(center, 2.4, ELECTRIC);
        painter.circle_filled(
            center - vec2(0.6, 0.6),
            0.8,
            Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 200),
        );
    } else {
        painter.circle_filled(center, 2.4, Color32::from_rgb(0x1a, 0x20, 0x26));
        painter.circle_stroke(center, 2.4, Stroke::new(1.0, PANEL_EDGE));
    }
}

// ---------------------------------------------------------------------------
// Widgets
// ---------------------------------------------------------------------------

/// A brass-bezel rotary knob with a dark glass face and a phosphor readout.
/// Drag vertically, Shift for fine control, double-click to reset.
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
    let desired = vec2(64.0, 88.0);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());
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
            let t = (to_t(*value) - dy * sensitivity).clamp(0.0, 1.0);
            *value = from_t(t);
            changed = true;
        }
    }

    let response = response.on_hover_cursor(CursorIcon::ResizeVertical);
    let engaged = response.hovered() || response.dragged();

    let painter = ui.painter();
    let center = pos2(rect.center().x, rect.top() + 42.0);
    let arc_radius = 19.0;
    let start = 135.0_f32.to_radians();
    let sweep = 270.0_f32.to_radians();
    let t = to_t(*value);

    painter.text(
        pos2(rect.center().x, rect.top() + 6.0),
        Align2::CENTER_TOP,
        label,
        FontId::proportional(10.0),
        TEXT_DIM,
    );

    let arc_points = |t0: f32, t1: f32, radius: f32| -> Vec<Pos2> {
        let n = 40;
        (0..=n)
            .map(|i| {
                let a = start + sweep * (t0 + (t1 - t0) * i as f32 / n as f32);
                center + vec2(a.cos(), a.sin()) * radius
            })
            .collect()
    };

    // Track and amber value arc
    painter.add(Shape::line(
        arc_points(0.0, 1.0, arc_radius),
        Stroke::new(3.0, PANEL_EDGE),
    ));
    if t > 0.001 {
        if engaged {
            painter.add(Shape::line(
                arc_points(0.0, t, arc_radius),
                Stroke::new(6.0, Color32::from_rgba_premultiplied(0xff, 0xb1, 0x4a, 50)),
            ));
        }
        painter.add(Shape::line(
            arc_points(0.0, t, arc_radius),
            Stroke::new(3.0, ACCENT),
        ));
    }

    // Brass bezel over a drop shadow, dark glass face
    painter.circle_filled(
        center + vec2(0.0, 1.5),
        14.5,
        Color32::from_rgba_premultiplied(0, 0, 0, 110),
    );
    painter.circle_filled(center, 14.0, BRASS_LO);
    painter.circle_stroke(center, 14.0, Stroke::new(1.0, BRASS_EDGE));
    // Top-left glint on the bezel
    let glint: Vec<Pos2> = (0..=20)
        .map(|i| {
            let a = (-200.0 + 160.0 * i as f32 / 20.0).to_radians();
            center + vec2(a.cos(), a.sin()) * 13.0
        })
        .collect();
    painter.add(Shape::line(
        glint,
        Stroke::new(1.4, Color32::from_rgba_premultiplied(0xd9, 0xb4, 0x6a, 150)),
    ));
    painter.circle_filled(center, 10.0, Color32::from_rgb(0x0e, 0x12, 0x16));
    painter.circle_filled(
        center - vec2(0.0, 3.0),
        6.0,
        Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 7),
    );
    if engaged {
        painter.circle_stroke(
            center,
            15.5,
            Stroke::new(2.0, Color32::from_rgba_premultiplied(0x53, 0xe7, 0xff, 80)),
        );
    }

    let angle = start + sweep * t;
    let dir = vec2(angle.cos(), angle.sin());
    painter.line_segment(
        [center + dir * 3.5, center + dir * 9.0],
        Stroke::new(2.0, ACCENT),
    );

    // Phosphor readout — dim cyan at rest, lit while touched
    painter.text(
        pos2(rect.center().x, rect.bottom() - 4.0),
        Align2::CENTER_BOTTOM,
        fmt(*value),
        FontId::monospace(10.5),
        if engaged { ELECTRIC } else { ELECTRIC_DIM },
    );

    changed
}

/// Waveform selector with a painted glyph on dark glass.
fn wave_button(ui: &mut egui::Ui, waveform: Waveform, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(48.0, 34.0), Sense::click());
    let painter = ui.painter();

    let bg = if selected {
        ACCENT_SOFT
    } else if response.hovered() {
        HOVER
    } else {
        BG_INSET
    };
    painter.rect_filled(rect, Rounding::same(6.0), bg);
    painter.rect_stroke(
        rect,
        Rounding::same(6.0),
        if selected {
            Stroke::new(1.0, ACCENT)
        } else {
            Stroke::new(1.0, PANEL_EDGE)
        },
    );

    let inner = rect.shrink2(vec2(11.0, 11.0));
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
    let stroke_color = if selected { ACCENT } else { TEXT_DIM };
    painter.add(Shape::line(points, Stroke::new(1.8, stroke_color)));

    response.on_hover_text(format!("{:?}", waveform))
}

/// Pill toggle for the chorus modes.
fn pill_button(ui: &mut egui::Ui, text: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(36.0, 22.0), Sense::click());
    let painter = ui.painter();
    let (bg, fg) = if selected {
        (ACCENT_SOFT, ACCENT)
    } else if response.hovered() {
        (HOVER, TEXT)
    } else {
        (BG_INSET, TEXT_DIM)
    };
    painter.rect_filled(rect, Rounding::same(11.0), bg);
    if selected {
        painter.rect_stroke(rect, Rounding::same(11.0), Stroke::new(1.0, ACCENT));
    }
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(11.0),
        fg,
    );
    response
}

/// A small machined-brass button.
fn brass_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 22.0), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(
        rect.translate(vec2(0.0, 1.0)),
        Rounding::same(5.0),
        Color32::from_rgba_premultiplied(0, 0, 0, 120),
    );
    painter.rect_filled(rect, Rounding::same(5.0), BRASS_LO);
    let (hi, lo) = if response.is_pointer_button_down_on() {
        (BRASS_LO, BRASS_LO)
    } else if response.hovered() {
        (BRASS_HI, BRASS_LO)
    } else {
        (
            Color32::from_rgb(0xb8, 0x93, 0x50),
            Color32::from_rgb(0x77, 0x5a, 0x2c),
        )
    };
    painter.add(gradient_quad(rect.shrink(1.5), hi, lo));
    painter.rect_stroke(rect, Rounding::same(5.0), Stroke::new(1.0, BRASS_EDGE));
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(11.0),
        BRASS_INK,
    );
    response
}

fn mini_header(text: &str) -> RichText {
    RichText::new(text).size(10.0).color(TEXT_DIM)
}

/// Glass panel section with a brass title and a status lamp. The background
/// is painted after layout via a placeholder shape, so the glass sits under
/// the contents.
fn section<R>(
    ui: &mut egui::Ui,
    title: &str,
    title_font: FontId,
    led: Option<bool>,
    time: f64,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) {
    let bg_idx = ui.painter().add(Shape::Noop);
    let inner = egui::Frame::none().inner_margin(12.0).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).font(title_font).color(BRASS_HI));
            if let Some(on) = led {
                let (led_rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                draw_led(ui.painter(), led_rect.center(), on, time, title.len() as f32);
            }
        });
        ui.add_space(4.0);
        add_contents(ui);
    });
    let rect = inner.response.rect;
    ui.painter().set(bg_idx, Shape::Vec(glass_shapes(rect, 10.0)));
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
            pressed_keys: HashSet::new(),
            theme_applied: false,
            serif_pending: false,
            serif_loaded: false,
            notes_active: false,
            time: 0.0,
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

    /// Try to install a real serif for the brand type — old-world print
    /// against the phosphor readouts. Falls back to the default font.
    fn install_fonts(&mut self, ctx: &egui::Context) {
        let candidates = [
            "/System/Library/Fonts/Supplemental/Georgia Italic.ttf",
            "/System/Library/Fonts/Supplemental/Georgia.ttf",
            "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
        ];
        for path in candidates {
            if let Ok(bytes) = std::fs::read(path) {
                let mut fonts = egui::FontDefinitions::default();
                fonts
                    .font_data
                    .insert(SERIF.to_owned(), egui::FontData::from_owned(bytes));
                let mut family = fonts
                    .families
                    .get(&egui::FontFamily::Proportional)
                    .cloned()
                    .unwrap_or_default();
                family.insert(0, SERIF.to_owned());
                fonts
                    .families
                    .insert(egui::FontFamily::Name(SERIF.into()), family);
                ctx.set_fonts(fonts);
                // set_fonts takes effect at the NEXT frame's begin; using the
                // named family this same frame panics in epaint. Mark pending
                // and start using it one frame later.
                self.serif_pending = true;
                return;
            }
        }
    }

    fn display_font(&self, size: f32) -> FontId {
        if self.serif_loaded {
            FontId::new(size, egui::FontFamily::Name(SERIF.into()))
        } else {
            FontId::proportional(size)
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        self.install_fonts(ctx);

        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = vec2(8.0, 8.0);
        style.spacing.button_padding = vec2(10.0, 4.0);

        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = Color32::TRANSPARENT;
        v.window_fill = BG_BOTTOM;
        v.override_text_color = Some(TEXT);
        v.widgets.inactive.bg_fill = BG_INSET;
        v.widgets.hovered.bg_fill = HOVER;
        v.widgets.active.bg_fill = ACCENT_SOFT;
        v.widgets.inactive.rounding = Rounding::same(6.0);
        v.widgets.hovered.rounding = Rounding::same(6.0);
        v.widgets.active.rounding = Rounding::same(6.0);
        v.widgets.open.rounding = Rounding::same(6.0);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
        v.widgets.hovered.fg_stroke = Stroke::new(1.2, TEXT);
        v.widgets.active.fg_stroke = Stroke::new(1.2, ACCENT);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, PANEL_EDGE);
        v.selection.bg_fill = ACCENT_SOFT;
        v.selection.stroke = Stroke::new(1.0, ACCENT);

        ctx.set_style(style);
    }

    /// Full-window backdrop: vertical gradient with a soft vignette, painted
    /// on the background layer beneath the transparent panels.
    fn paint_backdrop(&self, ctx: &egui::Context) {
        let rect = ctx.screen_rect();
        // Paint onto the panels' own layer, before the panels run, so the
        // gradient sits under their (transparent) frames — a separate
        // Background-order layer would draw over the panel content.
        let painter = ctx.layer_painter(egui::LayerId::background());
        painter.add(gradient_quad(rect, BG_TOP, BG_BOTTOM));
        let shade = Color32::from_rgba_premultiplied(0, 0, 0, 70);
        let clear = Color32::from_rgba_premultiplied(0, 0, 0, 0);
        let edge = 90.0_f32.min(rect.width() * 0.2);
        painter.add(gradient_quad_h(
            Rect::from_min_max(rect.left_top(), pos2(rect.left() + edge, rect.bottom())),
            shade,
            clear,
        ));
        painter.add(gradient_quad_h(
            Rect::from_min_max(pos2(rect.right() - edge, rect.top()), rect.right_bottom()),
            clear,
            shade,
        ));
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        if !self.theme_applied {
            self.apply_theme(ctx);
            self.theme_applied = true;
        } else if self.serif_pending {
            self.serif_pending = false;
            self.serif_loaded = true;
        }

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
        self.time = ctx.input(|i| i.time);

        // Keep repainting so keyboard lights, lamps, and the scope animate
        // even when the user isn't interacting
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        self.paint_backdrop(ctx);
        self.handle_keyboard_input(ctx);

        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::none()
                    .inner_margin(egui::style::Margin::symmetric(18.0, 12.0)),
            )
            .show(ctx, |ui| self.draw_header(ui));

        egui::TopBottomPanel::bottom("keyboard")
            .frame(egui::Frame::none().inner_margin(egui::style::Margin {
                left: 18.0,
                right: 18.0,
                top: 10.0,
                bottom: 12.0,
            }))
            .show(ctx, |ui| self.draw_keyboard_shelf(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::none().inner_margin(16.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    self.draw_oscillator_section(ui);
                    self.draw_envelope_section(ui);
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    self.draw_filter_section(ui);
                    self.draw_filter_env_section(ui);
                });
                ui.add_space(4.0);
                self.draw_effects_section(ui);
                ui.add_space(4.0);
                self.draw_scope(ui);
            });
    }

    /// Wood header rail: brand type stamped into walnut, screws in the
    /// corners, octave controls machined in brass with a phosphor readout.
    fn draw_header(&mut self, ui: &mut egui::Ui) {
        let bg_idx = ui.painter().add(Shape::Noop);

        ui.horizontal(|ui| {
            let title_pos = ui.cursor().min;
            let painter = ui.painter();
            // Stamped type: dark press shadow below-right, cream face on top
            painter.text(
                title_pos + vec2(1.5, 3.5),
                Align2::LEFT_TOP,
                "Patina",
                self.display_font(27.0),
                Color32::from_rgba_premultiplied(0, 0, 0, 160),
            );
            let title_rect = painter.text(
                title_pos + vec2(0.0, 2.0),
                Align2::LEFT_TOP,
                "Patina",
                self.display_font(27.0),
                IVORY,
            );
            ui.allocate_exact_size(vec2(title_rect.width() + 12.0, 34.0), Sense::hover());
            ui.label(
                RichText::new("POLYPHONIC · EST. 2026")
                    .font(FontId::monospace(9.0))
                    .color(ELECTRIC_DIM),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if brass_button(ui, "▶").on_hover_text("Octave up (↑)").clicked() {
                    self.shift_octave(1);
                }
                // Octave readout on a little dark glass screen
                let (chip, _) = ui.allocate_exact_size(vec2(58.0, 22.0), Sense::hover());
                let painter = ui.painter();
                painter.rect_filled(chip, Rounding::same(4.0), Color32::from_rgb(0x04, 0x07, 0x09));
                painter.rect_stroke(chip, Rounding::same(4.0), Stroke::new(1.0, PANEL_EDGE));
                painter.text(
                    chip.center(),
                    Align2::CENTER_CENTER,
                    format!("OCT {}", self.current_octave),
                    FontId::monospace(12.0),
                    ELECTRIC,
                );
                if brass_button(ui, "◀").on_hover_text("Octave down (↓)").clicked() {
                    self.shift_octave(-1);
                }
            });
        });

        let rail = ui.min_rect().expand2(vec2(18.0, 12.0));
        let mut shapes = wood_shapes(rail, 11);
        for (i, corner) in [
            rail.left_top() + vec2(10.0, 10.0),
            rail.right_top() + vec2(-10.0, 10.0),
            rail.left_bottom() + vec2(10.0, -10.0),
            rail.right_bottom() + vec2(-10.0, -10.0),
        ]
        .iter()
        .enumerate()
        {
            shapes.extend(screw_shapes(*corner, 3.2, 100 + i as u32));
        }
        ui.painter().set(bg_idx, Shape::Vec(shapes));
    }

    fn draw_oscillator_section(&mut self, ui: &mut egui::Ui) {
        let (font, led, time) = (self.display_font(13.0), Some(self.notes_active), self.time);
        section(ui, "Oscillator", font, led, time, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(mini_header("WAVE"));
                    ui.add_space(2.0);
                    egui::Grid::new("waveforms").spacing(vec2(6.0, 6.0)).show(ui, |ui| {
                        for (i, wf) in [
                            Waveform::Sine,
                            Waveform::Square,
                            Waveform::Sawtooth,
                            Waveform::Triangle,
                        ]
                        .iter()
                        .enumerate()
                        {
                            if wave_button(ui, *wf, self.waveform == *wf).clicked() {
                                self.waveform = *wf;
                                self.voice_manager.lock().set_waveform(self.waveform);
                            }
                            if i == 1 {
                                ui.end_row();
                            }
                        }
                    });
                });
                ui.add_space(8.0);
                if knob(ui, "VOLUME", &mut self.volume, 0.0, 1.0, 0.5, false, fmt_pct) {
                    self.voice_manager.lock().set_volume(self.volume);
                }
                if knob(ui, "DETUNE", &mut self.detune, 0.0, 30.0, 7.0, false, |v| {
                    format!("{:.0} ct", v)
                }) {
                    self.voice_manager.lock().set_detune(self.detune);
                }
            });
        });
    }

    fn draw_envelope_section(&mut self, ui: &mut egui::Ui) {
        let (font, led, time) = (self.display_font(13.0), Some(self.notes_active), self.time);
        section(ui, "Envelope", font, led, time, |ui| {
            ui.horizontal(|ui| {
                if knob(ui, "ATTACK", &mut self.attack, 0.01, 2.0, 0.1, true, fmt_time) {
                    self.voice_manager.lock().set_attack(self.attack);
                }
                if knob(ui, "DECAY", &mut self.decay, 0.01, 2.0, 0.1, true, fmt_time) {
                    self.voice_manager.lock().set_decay(self.decay);
                }
                if knob(ui, "SUSTAIN", &mut self.sustain, 0.0, 1.0, 0.7, false, fmt_pct) {
                    self.voice_manager.lock().set_sustain(self.sustain);
                }
                if knob(ui, "RELEASE", &mut self.release, 0.01, 2.0, 0.2, true, fmt_time) {
                    self.voice_manager.lock().set_release(self.release);
                }
                ui.add_space(6.0);
                self.draw_adsr_graph(ui);
            });
        });
    }

    fn draw_filter_env_section(&mut self, ui: &mut egui::Ui) {
        let led = self.fenv_amount.abs() > 0.05;
        let (font, time) = (self.display_font(13.0), self.time);
        section(ui, "Filter Env", font, Some(led), time, |ui| {
            ui.horizontal(|ui| {
                if knob(ui, "AMOUNT", &mut self.fenv_amount, -5.0, 5.0, 0.0, false, |v| {
                    format!("{:+.1} oct", v)
                }) {
                    self.voice_manager.lock().set_filter_env_amount(self.fenv_amount);
                }
                if knob(ui, "ATTACK", &mut self.fenv_attack, 0.001, 2.0, 0.005, true, fmt_time) {
                    self.voice_manager.lock().set_filter_attack(self.fenv_attack);
                }
                if knob(ui, "DECAY", &mut self.fenv_decay, 0.01, 2.0, 0.3, true, fmt_time) {
                    self.voice_manager.lock().set_filter_decay(self.fenv_decay);
                }
                if knob(ui, "SUSTAIN", &mut self.fenv_sustain, 0.0, 1.0, 0.0, false, fmt_pct) {
                    self.voice_manager.lock().set_filter_sustain(self.fenv_sustain);
                }
                if knob(ui, "RELEASE", &mut self.fenv_release, 0.01, 2.0, 0.3, true, fmt_time) {
                    self.voice_manager.lock().set_filter_release(self.fenv_release);
                }
            });
        });
    }

    fn draw_adsr_graph(&self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(vec2(160.0, 88.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, Rounding::same(6.0), BG_INSET);
        painter.rect_stroke(rect, Rounding::same(6.0), Stroke::new(1.0, PANEL_EDGE));

        let inner = rect.shrink(10.0);
        // Fixed visual fraction for the sustain plateau; A/D/R share the rest
        let hold = 0.3_f32;
        let total = (self.attack + self.decay + self.release).max(1e-6);
        let wa = self.attack / total * (1.0 - hold);
        let wd = self.decay / total * (1.0 - hold);

        let x0 = inner.left();
        let xa = x0 + wa * inner.width();
        let xd = xa + wd * inner.width();
        let xs = xd + hold * inner.width();
        let (top, bottom) = (inner.top(), inner.bottom());
        let ys = bottom - self.sustain * (bottom - top);

        let points = vec![
            pos2(x0, bottom),
            pos2(xa, top),
            pos2(xd, ys),
            pos2(xs, ys),
            pos2(inner.right(), bottom),
        ];
        painter.add(Shape::line(
            points.clone(),
            Stroke::new(4.0, Color32::from_rgba_premultiplied(0xff, 0xb1, 0x4a, 45)),
        ));
        painter.add(Shape::line(points.clone(), Stroke::new(2.0, ACCENT)));
        for pt in [points[1], points[2], points[3]] {
            painter.circle_filled(pt, 2.5, ACCENT);
        }
    }

    /// CRT-style oscilloscope: dark glass, faint phosphor grid, and the
    /// engine output as a glowing cyan trace, trigger-stabilized on a rising
    /// zero crossing so periodic waveforms hold still.
    fn draw_scope(&self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, 76.0), Sense::hover());
        let painter = ui.painter();
        for shape in glass_shapes(rect, 10.0) {
            painter.add(shape);
        }

        let inner = rect.shrink2(vec2(14.0, 11.0));
        let grid = Color32::from_rgba_premultiplied(0x53, 0xe7, 0xff, 14);
        for i in 0..=12 {
            let x = inner.left() + inner.width() * i as f32 / 12.0;
            painter.line_segment(
                [pos2(x, inner.top()), pos2(x, inner.bottom())],
                Stroke::new(0.5, grid),
            );
        }
        for i in 0..=4 {
            let y = inner.top() + inner.height() * i as f32 / 4.0;
            painter.line_segment(
                [pos2(inner.left(), y), pos2(inner.right(), y)],
                Stroke::new(0.5, grid),
            );
        }
        painter.text(
            pos2(rect.left() + 12.0, rect.top() + 6.0),
            Align2::LEFT_TOP,
            "SCOPE",
            FontId::monospace(9.0),
            ELECTRIC_DIM,
        );

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

        // Phosphor: wide soft bloom, tighter halo, crisp trace
        painter.add(Shape::line(
            points.clone(),
            Stroke::new(7.0, Color32::from_rgba_premultiplied(0x53, 0xe7, 0xff, 18)),
        ));
        painter.add(Shape::line(
            points.clone(),
            Stroke::new(3.5, Color32::from_rgba_premultiplied(0x53, 0xe7, 0xff, 60)),
        ));
        painter.add(Shape::line(points, Stroke::new(1.4, ELECTRIC)));
    }

    fn draw_filter_section(&mut self, ui: &mut egui::Ui) {
        let led = self.filter_cutoff < 19_000.0 || self.filter_resonance > 0.05;
        let (font, time) = (self.display_font(13.0), self.time);
        section(ui, "Filter", font, Some(led), time, |ui| {
            ui.horizontal(|ui| {
                if knob(ui, "CUTOFF", &mut self.filter_cutoff, 20.0, 20000.0, 15000.0, true, fmt_hz) {
                    self.voice_manager.lock().set_filter_cutoff(self.filter_cutoff);
                }
                if knob(ui, "RESONANCE", &mut self.filter_resonance, 0.0, 4.0, 0.0, false, fmt_x) {
                    self.voice_manager.lock().set_filter_resonance(self.filter_resonance);
                }
                if knob(ui, "DRIVE", &mut self.filter_drive, 0.1, 5.0, 1.0, false, fmt_x) {
                    self.voice_manager.lock().set_filter_drive(self.filter_drive);
                }
                if knob(ui, "SATURATE", &mut self.filter_saturation, 0.0, 2.0, 1.0, false, fmt_x) {
                    self.voice_manager.lock().set_filter_saturation(self.filter_saturation);
                }
                if knob(ui, "HIGH-PASS", &mut self.hpf_cutoff, 16.0, 8000.0, 16.0, true, fmt_hz) {
                    self.voice_manager.lock().set_hpf_cutoff(self.hpf_cutoff);
                }
            });
        });
    }

    fn draw_effects_section(&mut self, ui: &mut egui::Ui) {
        let led = self.chorus_mode != ChorusMode::Off
            || self.reverb_wet > 0.01
            || self.fuzz > 0.01
            || self.tape_wow > 0.01
            || self.tape_flutter > 0.01
            || self.tape_drive > 0.01
            || self.tape_age > 0.01;
        let (font, time) = (self.display_font(13.0), self.time);
        section(ui, "Effects", font, Some(led), time, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(mini_header("CHORUS"));
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (mode, label) in [
                            (ChorusMode::Off, "OFF"),
                            (ChorusMode::I, "I"),
                            (ChorusMode::II, "II"),
                            (ChorusMode::III, "III"),
                            (ChorusMode::IV, "IV"),
                        ] {
                            if pill_button(ui, label, self.chorus_mode == mode).clicked() {
                                self.chorus_mode = mode;
                                self.voice_manager.lock().set_chorus_mode(self.chorus_mode);
                            }
                        }
                    });
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        if knob(ui, "RATE", &mut self.chorus_rate, 0.1, 10.0, 0.5, true, |v| {
                            format!("{:.1} Hz", v)
                        }) {
                            self.voice_manager.lock().set_chorus_rate(self.chorus_rate);
                        }
                        if knob(ui, "DEPTH", &mut self.chorus_depth, 0.0, 1.0, 0.3, false, fmt_pct) {
                            self.voice_manager.lock().set_chorus_depth(self.chorus_depth);
                        }
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.label(mini_header("REVERB"));
                    ui.add_space(28.0);
                    ui.horizontal(|ui| {
                        if knob(ui, "DECAY", &mut self.reverb_decay, 0.0, 0.99, 0.5, false, fmt_pct) {
                            self.voice_manager.lock().set_reverb_decay(self.reverb_decay);
                        }
                        if knob(ui, "MIX", &mut self.reverb_wet, 0.0, 1.0, 0.5, false, fmt_pct) {
                            self.voice_manager.lock().set_reverb_wet(self.reverb_wet);
                        }
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.label(mini_header("FUZZ"));
                    ui.add_space(28.0);
                    ui.horizontal(|ui| {
                        if knob(ui, "GERMANIUM", &mut self.fuzz, 0.0, 1.0, 0.0, false, fmt_pct) {
                            self.voice_manager.lock().set_fuzz(self.fuzz);
                        }
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.label(mini_header("TAPE"));
                    ui.add_space(28.0);
                    ui.horizontal(|ui| {
                        if knob(ui, "WOW", &mut self.tape_wow, 0.0, 1.0, 0.0, false, fmt_pct) {
                            self.voice_manager.lock().set_tape_wow(self.tape_wow);
                        }
                        if knob(ui, "FLUTTER", &mut self.tape_flutter, 0.0, 1.0, 0.0, false, fmt_pct) {
                            self.voice_manager.lock().set_tape_flutter(self.tape_flutter);
                        }
                        if knob(ui, "DRIVE", &mut self.tape_drive, 0.0, 1.0, 0.0, false, fmt_pct) {
                            self.voice_manager.lock().set_tape_drive(self.tape_drive);
                        }
                        if knob(ui, "AGE", &mut self.tape_age, 0.0, 1.0, 0.0, false, fmt_pct) {
                            self.voice_manager.lock().set_tape_age(self.tape_age);
                        }
                    });
                });
            });
        });
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

    /// The keyboard sits on a walnut shelf with a red felt strip along the
    /// fallboard, like an old upright.
    fn draw_keyboard_shelf(&mut self, ui: &mut egui::Ui) {
        let bg_idx = ui.painter().add(Shape::Noop);

        self.draw_keyboard(ui);
        ui.add_space(6.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(
                    "play: Z–M lower · Q–U upper  |  ↑ ↓ octave  |  knobs: drag · shift = fine · double-click = reset",
                )
                .size(10.5)
                .color(Color32::from_rgb(0xc9, 0xb4, 0x8e)),
            );
        });

        let shelf = ui.min_rect().expand2(vec2(18.0, 11.0));
        let mut shapes = wood_shapes(shelf, 47);
        shapes.extend(screw_shapes(shelf.left_bottom() + vec2(10.0, -9.0), 3.2, 200));
        shapes.extend(screw_shapes(shelf.right_bottom() + vec2(-10.0, -9.0), 3.2, 201));
        ui.painter().set(bg_idx, Shape::Vec(shapes));
    }

    fn draw_keyboard(&mut self, ui: &mut egui::Ui) {
        let available_width = ui.available_width();
        let white_key_width = available_width / (7.0 * OCTAVES as f32);
        let white_key_height = 130.0;
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
        painter.rect_filled(rect.expand(4.0), Rounding::same(4.0), BG_INSET);

        // White keys — aged ivory
        for visual_octave in 0..OCTAVES {
            for (i, &key_index) in WHITE_KEY_INDICES.iter().enumerate() {
                if let Some(note) = self.calculate_midi_note(visual_octave as i32, key_index) {
                    let x = (visual_octave * 7 + i) as f32 * white_key_width;
                    let key_rect = Rect::from_min_size(
                        rect.min + Vec2::new(x + 1.0, 0.0),
                        Vec2::new(white_key_width - 2.0, white_key_height),
                    );
                    let pressed = key_states[note as usize];
                    let rounding = Rounding {
                        nw: 0.0,
                        ne: 0.0,
                        sw: 4.0,
                        se: 4.0,
                    };
                    if pressed {
                        // Warm glow bleeding around the lit key
                        painter.rect_filled(
                            key_rect.expand(3.0),
                            rounding,
                            Color32::from_rgba_premultiplied(0xff, 0xb1, 0x4a, 55),
                        );
                    }
                    painter.rect_filled(key_rect, rounding, if pressed { ACCENT } else { IVORY });
                    // Front-edge shading gives the keys a little depth
                    let shade = Rect::from_min_max(
                        pos2(key_rect.min.x, key_rect.max.y - 7.0),
                        key_rect.max,
                    );
                    painter.rect_filled(
                        shade,
                        rounding,
                        if pressed { ACCENT_PRESSED_SHADE } else { IVORY_SHADE },
                    );
                    // Side shadow keeps the keys reading as separate slats
                    painter.line_segment(
                        [key_rect.right_top(), key_rect.right_bottom()],
                        Stroke::new(1.0, Color32::from_rgba_premultiplied(0, 0, 0, 40)),
                    );

                    if key_index == 0 {
                        painter.text(
                            pos2(key_rect.center().x, key_rect.max.y - 30.0),
                            Align2::CENTER_CENTER,
                            format!("C{}", self.current_octave + visual_octave as i32),
                            FontId::proportional(9.0),
                            if pressed {
                                ACCENT_INK
                            } else {
                                Color32::from_rgb(0xb5, 0xac, 0x93)
                            },
                        );
                    }
                    if let Some(hint) = self.key_hint(visual_octave, key_index) {
                        painter.text(
                            pos2(key_rect.center().x, key_rect.max.y - 16.0),
                            Align2::CENTER_CENTER,
                            hint,
                            FontId::proportional(11.0),
                            if pressed {
                                ACCENT_INK
                            } else {
                                Color32::from_rgb(0x9a, 0x91, 0x79)
                            },
                        );
                    }
                }
            }
        }

        // Black keys — ebony with a lit front edge
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
                    let key_rect = Rect::from_min_size(
                        rect.min + Vec2::new(x + visual_octave as f32 * 7.0 * white_key_width, 0.0),
                        Vec2::new(black_key_width, black_key_height),
                    );
                    let pressed = key_states[note as usize];
                    let rounding = Rounding {
                        nw: 0.0,
                        ne: 0.0,
                        sw: 3.0,
                        se: 3.0,
                    };
                    painter.rect_filled(
                        key_rect,
                        rounding,
                        if pressed { ACCENT_PRESSED_SHADE } else { EBONY },
                    );
                    if !pressed {
                        let edge = Rect::from_min_max(
                            pos2(key_rect.min.x, key_rect.max.y - 5.0),
                            key_rect.max,
                        );
                        painter.rect_filled(edge, rounding, EBONY_EDGE);
                    }
                    painter.rect_stroke(key_rect, rounding, Stroke::new(1.0, BG_INSET));

                    if let Some(hint) = self.key_hint(visual_octave, key_index) {
                        painter.text(
                            pos2(key_rect.center().x, key_rect.max.y - 13.0),
                            Align2::CENTER_CENTER,
                            hint,
                            FontId::proportional(10.0),
                            if pressed { ACCENT_INK } else { TEXT_DIM },
                        );
                    }
                }
            }
        }

        // Red felt strip along the fallboard
        painter.rect_filled(
            Rect::from_min_max(rect.left_top(), pos2(rect.right(), rect.top() + 3.5)),
            0.0,
            FELT,
        );
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

        for &key in KEYS.iter() {
            if ctx.input(|i| i.key_pressed(key)) && !self.pressed_keys.contains(&key) {
                if let Some(note) = self.key_to_note(key) {
                    self.play_note(note);
                    self.pressed_keys.insert(key);
                }
            }
            if ctx.input(|i| i.key_released(key)) {
                if let Some(note) = self.key_to_note(key) {
                    self.stop_note(note);
                    self.pressed_keys.remove(&key);
                }
            }
        }
    }

    /// Change octave, releasing any held notes first so nothing gets stuck
    /// (note-off would otherwise map to a different MIDI note).
    fn shift_octave(&mut self, delta: i32) {
        let held: Vec<Key> = self.pressed_keys.iter().copied().collect();
        for key in held {
            if let Some(note) = self.key_to_note(key) {
                self.stop_note(note);
            }
        }
        self.pressed_keys.clear();
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
