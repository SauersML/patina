// Patina's shared visual language: the light-Frutiger-Aero design system
// and the hardware-panel widget set. Everything here is pure egui — no
// eframe, no wgpu — so the SAME widgets drive both front ends:
//   - the standalone app (src/ui.rs), which layers the WGSL aurora and
//     live-glass pipeline on top, and
//   - the plugin editor (src/editor.rs), which runs inside a host-provided
//     window (Logic's AU view, CLAP/VST3 editors) on the baked-CPU path.
//
// Moved verbatim from ui.rs — if a widget needs to change, it changes here
// for every surface at once.

use egui::epaint::{EllipseShape, Mesh};
use egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, RichText,
    Sense, Shape, Stroke, TextureHandle, TextureOptions,
};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

use crate::oscillator::Waveform;
use crate::panel_render;

// ---------------------------------------------------------------------------
// Design system: light Frutiger Aero. A luminous animated sky, white
// frosted glass with dark slate type, dark glossy "device screens" set into
// the glass (wells), warm walnut rails, amber for touch, aqua for signal.
// ---------------------------------------------------------------------------
pub const BG0: Color32 = Color32::from_rgb(0x6f, 0xa8, 0xd0); // sky fallback
pub const BG2: Color32 = Color32::from_rgb(0xd4, 0xe7, 0xef); // light controls
pub const BG2_HOVER: Color32 = Color32::from_rgb(0xe4, 0xf2, 0xf8);
pub const INSET: Color32 = Color32::from_rgb(0x0a, 0x11, 0x14); // device screens

// TOUCH is the single interaction accent (deep aqua): arcs, lit keys,
// selection gloss. Dark hairlines sit on white glass.
pub const HAIRLINE: Color32 = Color32::from_rgba_premultiplied(0x24, 0x3a, 0x46, 45);
pub const HAIRLINE_HI: Color32 = Color32::from_rgba_premultiplied(0x1d, 0x33, 0x40, 85);

pub const TXT: Color32 = Color32::from_rgb(0x24, 0x33, 0x3c);
pub const TXT_MID: Color32 = Color32::from_rgb(0x43, 0x54, 0x5e);
pub const TXT_LOW: Color32 = Color32::from_rgb(0x6b, 0x7c, 0x86);

// Text inside the dark wells needs to stay light
pub const WELL_TXT: Color32 = Color32::from_rgb(0x7f, 0x96, 0xa0);
pub const WELL_TXT_HOVER: Color32 = Color32::from_rgb(0xc6, 0xd8, 0xde);
pub const WELL_LINE: Color32 = Color32::from_rgba_premultiplied(0xff, 0xff, 0xff, 22);

pub const TOUCH: Color32 = Color32::from_rgb(0x12, 0x9e, 0xc0);
pub const TOUCH_HI: Color32 = Color32::from_rgb(0x1e, 0xc2, 0xe8);
pub const TOUCH_DEEP: Color32 = Color32::from_rgb(0x0d, 0x7c, 0x98);
pub const TOUCH_INK: Color32 = Color32::from_rgb(0x05, 0x33, 0x40);

pub const CYAN: Color32 = Color32::from_rgb(0x35, 0xdf, 0xf5);

pub const CYAN_BRIGHT: Color32 = Color32::from_rgb(0xf4, 0xfd, 0xff);

pub const IVORY: Color32 = Color32::from_rgb(0xea, 0xe6, 0xdb);
pub const IVORY_SHADE: Color32 = Color32::from_rgb(0xd6, 0xd0, 0xc1);
pub const EBONY: Color32 = Color32::from_rgb(0x15, 0x16, 0x1a);
pub const EBONY_EDGE: Color32 = Color32::from_rgb(0x2c, 0x2f, 0x36);

/// Whether the WGSL live-glass pipeline is active this frame (app only —
/// plugin editors always take the baked-CPU frost path).
pub static GPU_ON: AtomicBool = AtomicBool::new(false);

/// Card rects collected during layout; the NEXT frame paints their glass
/// into the background layer before any content, so the panes always sit
/// under the controls. One frame of lag, imperceptible at 60 Hz.
pub static GLASS_RECTS: Mutex<Vec<Rect>> = Mutex::new(Vec::new());

// ---------------------------------------------------------------------------
// Per-pixel material synthesis — realism without image assets. An fBm
// walnut with growth rings, a sphere-shaded gloss knob sprite, and a soft
// aurora backdrop whose low-frequency light lets the translucent panels
// above it read as frosted glass.
// ---------------------------------------------------------------------------

pub struct Textures {
    pub backdrop: TextureHandle,
    pub backdrop_rgb: Vec<[f32; 3]>,
    pub backdrop_size: [usize; 2],
    /// Baked frosted-glass panels, keyed by rounded screen rect.
    pub frost: HashMap<(i32, i32, i32, i32), TextureHandle>,
}

impl Textures {
    /// Bake the aurora backdrop once at the given pixel size.
    pub fn bake(ctx: &egui::Context, w: usize, h: usize) -> Self {
        let rgb = panel_render::render_backdrop(w, h);
        let image = panel_render::backdrop_image(w, h, &rgb);
        let backdrop = ctx.load_texture("panel-backdrop", image, TextureOptions::LINEAR);
        Self { backdrop, backdrop_rgb: rgb, backdrop_size: [w, h], frost: HashMap::new() }
    }
}

/// A quad whose top and bottom edges carry different vertex colors — the
/// GPU interpolates, giving a smooth vertical gradient.
pub fn gradient_quad(rect: Rect, top: Color32, bottom: Color32) -> Shape {
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
pub fn rail_shapes(rect: Rect) -> Vec<Shape> {
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
pub fn glass_shapes(rect: Rect, rounding: f32) -> Vec<Shape> {
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
pub fn gloss_fill(painter: &egui::Painter, rect: Rect, rounding: f32) {
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

// ---------------------------------------------------------------------------
// Type helpers
// ---------------------------------------------------------------------------

/// Letterspaced micro-caps, the panel-legend voice of the whole interface.
pub fn tracked(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for (i, c) in text.chars().enumerate() {
        if i > 0 {
            out.push('\u{2009}'); // thin space
        }
        out.push(c.to_ascii_uppercase());
    }
    out
}

pub fn legend(text: &str) -> RichText {
    RichText::new(tracked(text)).size(10.0).color(TXT_MID)
}

pub fn sublegend(text: &str) -> RichText {
    RichText::new(tracked(text)).size(8.5).color(TXT_LOW)
}

pub fn fmt_hz(v: f32) -> String {
    if v >= 1000.0 {
        format!("{:.1} kHz", v / 1000.0)
    } else {
        format!("{:.0} Hz", v)
    }
}

pub fn fmt_time(v: f32) -> String {
    if v < 1.0 {
        format!("{:.0} ms", v * 1000.0)
    } else {
        format!("{:.2} s", v)
    }
}

pub fn fmt_pct(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}

pub fn fmt_x(v: f32) -> String {
    format!("{:.2}", v)
}

// ---------------------------------------------------------------------------
// Widgets
// ---------------------------------------------------------------------------

/// Rotary control in the language of hardware panel legends: etched tick
/// marks, a dome-shaded cap, a thin amber value arc (grown from 12 o'clock
/// for bipolar ranges), and a quiet tabular readout. Drag vertically or
/// scroll; Shift for fine control; double-click to reset.
pub fn knob(
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

pub fn knob_sized(
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
pub fn drum_glyph(painter: &egui::Painter, rect: Rect, which: usize, color: Color32, ghost: bool) {
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

pub fn wave_glyph(painter: &egui::Painter, rect: Rect, waveform: Waveform, color: Color32) {
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
pub fn waveform_selector(ui: &mut egui::Ui, id: &str, selected: &mut Waveform) -> bool {
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
pub fn segmented(ui: &mut egui::Ui, id: &str, labels: &[&str], selected: usize) -> Option<usize> {
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
pub fn step_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
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
pub fn card<R>(
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
pub fn vseparator(ui: &mut egui::Ui, height: f32) {
    // A breath of space, not a line — the grouping reads from the gap
    let _ = ui.allocate_exact_size(vec2(9.0, height), Sense::hover());
}
