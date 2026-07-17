use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, CursorIcon, FontId, Key, Pos2, Rect, RichText,
    Sense, Shape, Stroke, TextureHandle, TextureOptions, Vec2,
};
use eframe::egui::epaint::Mesh;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;

use crate::aurora_gpu;
use crate::chorus::ChorusMode;
use crate::oscillator::Waveform;
use crate::panel_render;
use crate::voice_manager::VoiceManager;

const OCTAVES: usize = 3;
const WHITE_KEY_INDICES: [usize; 7] = [0, 2, 4, 5, 7, 9, 11];
const BLACK_KEY_INDICES: [usize; 5] = [1, 3, 6, 8, 10];

// ---------------------------------------------------------------------------
// Design system: light Frutiger Aero. A luminous animated sky, white
// frosted glass with dark slate type, dark glossy "device screens" set into
// the glass (wells), warm walnut rails, amber for touch, aqua for signal.
// ---------------------------------------------------------------------------
const BG0: Color32 = Color32::from_rgb(0x6f, 0xa8, 0xd0); // sky fallback
const BG2: Color32 = Color32::from_rgb(0xd4, 0xe7, 0xef); // light controls
const BG2_HOVER: Color32 = Color32::from_rgb(0xe4, 0xf2, 0xf8);
const INSET: Color32 = Color32::from_rgb(0x0a, 0x11, 0x14); // device screens

// Dark hairlines: these sit on white glass now
const HAIRLINE: Color32 = Color32::from_rgba_premultiplied(0x24, 0x3a, 0x46, 45);
const HAIRLINE_HI: Color32 = Color32::from_rgba_premultiplied(0x1d, 0x33, 0x40, 85);

const TXT: Color32 = Color32::from_rgb(0x24, 0x33, 0x3c);
const TXT_MID: Color32 = Color32::from_rgb(0x43, 0x54, 0x5e);
const TXT_LOW: Color32 = Color32::from_rgb(0x6b, 0x7c, 0x86);

// Text inside the dark wells needs to stay light
const WELL_TXT: Color32 = Color32::from_rgb(0x7f, 0x96, 0xa0);
const WELL_TXT_HOVER: Color32 = Color32::from_rgb(0xc6, 0xd8, 0xde);
const WELL_LINE: Color32 = Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 22);

const AMBER: Color32 = Color32::from_rgb(0x12, 0x9e, 0xc0);
const AMBER_HI: Color32 = Color32::from_rgb(0x1e, 0xc2, 0xe8);
const AMBER_DEEP: Color32 = Color32::from_rgb(0x0d, 0x7c, 0x98);
const AMBER_INK: Color32 = Color32::from_rgb(0x05, 0x33, 0x40);

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
    /// QWERTY keys currently sounding, mapped to the MIDI note each one
    /// started (so note-off always matches, even if the octave changed).
    pressed_keys: HashMap<Key, u8>,
    theme_applied: bool,
    notes_active: bool,
    textures: Option<Textures>,
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
    let (rect, response) = ui.allocate_exact_size(vec2(63.0, 88.0), Sense::click_and_drag());
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
    let center = pos2(rect.center().x, rect.top() + 42.0);
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
        let (r0, r1) = if i % 5 == 0 { (21.0, 25.5) } else { (21.0, 23.5) };
        painter.line_segment(
            [center + dir * r0, center + dir * r1],
            Stroke::new(1.0, HAIRLINE_HI),
        );
    }

    // Value arc — from 12 o'clock for bipolar ranges, from min otherwise
    let arc_r = 18.0;
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

    // Committed 2D: a flat disc; the arc, ticks, and pointer carry it
    let disc = if engaged {
        Color32::from_rgb(0x28, 0x33, 0x3d)
    } else {
        Color32::from_rgb(0x20, 0x29, 0x31)
    };
    painter.circle_filled(center, 14.0, disc);
    painter.circle_stroke(
        center,
        14.0,
        if engaged {
            Stroke::new(1.2, Color32::from_rgba_unmultiplied(0x1e, 0xc2, 0xe8, 150))
        } else {
            Stroke::new(1.0, HAIRLINE_HI)
        },
    );

    let dir = vec2(end_angle.cos(), end_angle.sin());
    painter.line_segment(
        [center + dir * 5.0, center + dir * 12.5],
        Stroke::new(
            2.0,
            if engaged { AMBER_HI } else { Color32::from_rgb(0xee, 0xf4, 0xf6) },
        ),
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
    painter.rect_filled(rect, CornerRadius::same(7), INSET);

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
    let cell = vec2(34.0, 24.0);
    let (rect, _) = ui.allocate_exact_size(
        vec2(cell.x * labels.len() as f32, cell.y),
        Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(7), INSET);

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
            left: 14,
            right: 14,
            top: 9,
            bottom: 10,
        })
        .show(ui, |ui| {
            // The card sizes to its content — measured widths only, never
            // available_width (unbounded inside rows in egui 0.31). A row's
            // LAST card passes the measured remainder to run flush right.
            if let Some(w) = fill_width {
                ui.set_min_width((w - 28.0).max(60.0));
            }
            ui.label(legend(title));
            ui.add_space(8.0);
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
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, height), Sense::hover());
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
            glide: 0.0,
            sub: 0.0,
            osc2_wave: Waveform::Sawtooth,
            osc2_pitch: 0.0,
            osc2_level: 0.72,
            osc3_wave: Waveform::Sawtooth,
            osc3_pitch: 0.0,
            osc3_level: 0.72,
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
            pressed_keys: HashMap::new(),
            theme_applied: false,
            notes_active: false,
            textures: None,
            pending_size: [0, 0],
            size_stable_frames: 0,
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
        vm.set_glide(self.glide);
        vm.set_sub(self.sub);
        vm.set_osc_wave(1, self.osc2_wave);
        vm.set_osc_pitch(1, self.osc2_pitch);
        vm.set_osc_level(1, self.osc2_level);
        vm.set_osc_wave(2, self.osc3_wave);
        vm.set_osc_pitch(2, self.osc3_pitch);
        vm.set_osc_level(2, self.osc3_level);
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
        v.widgets.active.fg_stroke = Stroke::new(1.0, AMBER);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, HAIRLINE);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, HAIRLINE_HI);
        v.selection.bg_fill = Color32::from_rgb(0x10, 0x3a, 0x46);
        v.selection.stroke = Stroke::new(1.0, AMBER);

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
            self.notes_active = vm.held_note_states().iter().any(|&held| held);
        }

        // The sky is alive: repaint at display cadence
        ctx.request_repaint_after(std::time::Duration::from_millis(16));

        let time = ctx.input(|i| i.time) as f32;
        TIME_BITS.store(time.to_bits() as u64, AtomicOrdering::Relaxed);

        // Backdrop on the panels' shared layer, before they run: the WGSL
        // sky when the GPU pipeline is live, the baked image otherwise.
        // Glass panes paint here too, from last frame's rects, so they are
        // always under the controls.
        if GPU_ON.load(AtomicOrdering::Relaxed) {
            let painter = ctx.layer_painter(egui::LayerId::background());
            painter.add(aurora_gpu::sky_shape(ctx.screen_rect(), time));
            let rects: Vec<Rect> = std::mem::take(&mut *GLASS_RECTS.lock());
            for (i, rect) in rects.into_iter().enumerate() {
                let slot = (i as u32 + 1) % 64;
                painter.add(aurora_gpu::glass_shape(
                    rect,
                    ctx.screen_rect(),
                    time,
                    12.0,
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
                let mut shapes = rail_shapes(shelf);
                shapes.push(Shape::line_segment(
                    [shelf.left_top(), shelf.right_top()],
                    Stroke::new(1.5, Color32::from_rgba_unmultiplied(0, 0, 0, 150)),
                ));
                painter.set(bg_idx, Shape::Vec(shapes));
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
                ui.spacing_mut().item_spacing = vec2(11.0, 10.0);
                self.draw_preset_strip(ui);
                let top = egui::Layout::left_to_right(egui::Align::TOP);
                ui.with_layout(top, |ui| {
                    self.draw_oscillator_card(ui, tex.as_mut(), None);
                    let rest = ui.available_width();
                    self.draw_envelope_card(ui, tex.as_mut(), Some(rest));
                });
                ui.with_layout(top, |ui| {
                    self.draw_filter_card(ui, tex.as_mut(), None);
                    let rest = ui.available_width();
                    self.draw_filter_env_card(ui, tex.as_mut(), Some(rest));
                });
                ui.with_layout(top, |ui| {
                    self.draw_lfo_card(ui, tex.as_mut(), None);
                    let rest = ui.available_width();
                    self.draw_effects_card(ui, tex.as_mut(), Some(rest));
                });
                self.draw_scope(ui);
            });
        self.textures = tex;
    }

    /// Preset strip, in the spirit of the Minitmoog's preset panel
    /// (US 3,981,218): one click retunes every functional block at once.
    /// Patches apply live, so you can morph a held chord between them.
    fn draw_preset_strip(&mut self, ui: &mut egui::Ui) {
        let height = 30.0;
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
        let color = if response.hovered() { AMBER_HI } else { AMBER };
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
        let mut shapes = rail_shapes(rail);
        shapes.push(Shape::line_segment(
            [rail.left_bottom(), rail.right_bottom()],
            Stroke::new(1.5, Color32::from_rgba_unmultiplied(0, 0, 0, 160)),
        ));
        ui.painter()
            .with_clip_rect(screen)
            .set(bg_idx, Shape::Vec(shapes));
    }

    fn draw_oscillator_card(&mut self, ui: &mut egui::Ui, tex: Option<&mut Textures>, fill: Option<f32>) {
        card(ui, "Oscillator", tex, fill, |ui| {
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
                if knob(ui, "Sub", &mut self.sub, 0.0, 1.0, 0.0, false, fmt_pct) {
                    self.voice_manager.lock().set_sub(self.sub);
                }
                if knob(ui, "Noise", &mut self.noise, 0.0, 1.0, 0.0, false, fmt_pct) {
                    self.voice_manager.lock().set_noise(self.noise);
                }
                if knob(ui, "Width", &mut self.pulse_width, 0.05, 0.95, 0.5, false, fmt_pct) {
                    self.voice_manager.lock().set_pulse_width(self.pulse_width);
                }
                if knob(ui, "Glide", &mut self.glide, 0.0, 2.0, 0.0, false, |v| {
                    if v < 0.001 {
                        "off".into()
                    } else {
                        fmt_time(v)
                    }
                }) {
                    self.voice_manager.lock().set_glide(self.glide);
                }
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
                    let selected = match *wave {
                        Waveform::Sine => 0,
                        Waveform::Square => 1,
                        Waveform::Sawtooth => 2,
                        Waveform::Triangle => 3,
                    };
                    let id = if which == 1 { "osc2wave" } else { "osc3wave" };
                    if let Some(i) = segmented(ui, id, &["SIN", "SQR", "SAW", "TRI"], selected) {
                        *wave = [
                            Waveform::Sine,
                            Waveform::Square,
                            Waveform::Sawtooth,
                            Waveform::Triangle,
                        ][i];
                        self.voice_manager.lock().set_osc_wave(which, *wave);
                    }
                    if knob(ui, "Pitch", pitch, -24.0, 24.0, if which == 1 { 0.0 } else { -12.0 }, false, |v| {
                        format!("{v:+.0} st")
                    }) {
                        self.voice_manager.lock().set_osc_pitch(which, *pitch);
                    }
                    if knob(ui, "Level", level, 0.0, 1.0, 0.72, false, fmt_pct) {
                        self.voice_manager.lock().set_osc_level(which, *level);
                    }
                    if which == 1 {
                        ui.add_space(10.0);
                    }
                }
            });
        });
    }

    fn draw_envelope_card(&mut self, ui: &mut egui::Ui, tex: Option<&mut Textures>, fill: Option<f32>) {
        card(ui, "Envelope", tex, fill, |ui| {
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

    fn draw_filter_card(&mut self, ui: &mut egui::Ui, tex: Option<&mut Textures>, fill: Option<f32>) {
        card(ui, "Filter", tex, fill, |ui| {
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

    fn draw_filter_env_card(&mut self, ui: &mut egui::Ui, tex: Option<&mut Textures>, fill: Option<f32>) {
        card(ui, "Filter Envelope", tex, fill, |ui| {
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

    fn draw_lfo_card(&mut self, ui: &mut egui::Ui, tex: Option<&mut Textures>, fill: Option<f32>) {
        card(ui, "LFO", tex, fill, |ui| {
            ui.horizontal(|ui| {
                if knob(ui, "Rate", &mut self.lfo_rate, 0.1, 30.0, 1.0, true, |v| {
                    format!("{:.2} Hz", v)
                }) {
                    self.voice_manager.lock().set_lfo_rate(self.lfo_rate);
                }
                if knob(ui, "Shape", &mut self.lfo_shape, 0.0, 1.0, 0.5, false, |v| {
                    if v < 0.15 {
                        "saw".into()
                    } else if v > 0.85 {
                        "ramp".into()
                    } else if (0.4..=0.6).contains(&v) {
                        "tri".into()
                    } else {
                        format!("{:.2}", v)
                    }
                }) {
                    self.voice_manager.lock().set_lfo_shape(self.lfo_shape);
                }
                if knob(ui, "Pitch", &mut self.lfo_pitch, 0.0, 200.0, 0.0, false, |v| {
                    format!("{:.0} ct", v)
                }) {
                    self.voice_manager.lock().set_lfo_pitch(self.lfo_pitch);
                }
                if knob(ui, "Filter", &mut self.lfo_filter, 0.0, 4.0, 0.0, false, |v| {
                    format!("{:.2} oct", v)
                }) {
                    self.voice_manager.lock().set_lfo_filter(self.lfo_filter);
                }
                if knob(ui, "PWM", &mut self.lfo_pwm, 0.0, 0.45, 0.0, false, fmt_pct) {
                    self.voice_manager.lock().set_lfo_pwm(self.lfo_pwm);
                }
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
                    ui.label(sublegend("Reverb"));
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
                    ui.label(sublegend("Tape"));
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
                    ui.label(sublegend("Fuzz"));
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
        let height = ui.available_height().clamp(58.0, 170.0);
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
        let white_key_height = 118.0;
        let black_key_width = white_key_width * 0.6;
        let black_key_height = white_key_height * 0.6;

        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(available_width, white_key_height),
            egui::Sense::click_and_drag(),
        );
        // The piano is pointer-only: surrendering focus kills egui's focus
        // ring and keeps arrow keys free for octave shifting
        response.surrender_focus();
        self.handle_mouse_input(ui, rect, &response);

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
                            Color32::from_rgba_premultiplied(0xe0, 0xa1, 0x54, 40),
                        );
                    }
                    let hovered = hovered_note == Some(note) && !pressed;
                    painter.rect_filled(key_rect, rounding, if pressed { AMBER } else { IVORY });
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
                        if pressed { AMBER_DEEP } else { EBONY },
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

        if ctx.input(|i| i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals)) {
            self.shift_octave(1);
        }
        if ctx.input(|i| i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::Minus)) {
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
