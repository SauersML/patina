// The plugin editor: Patina's front panel inside a host-provided window.
// Pure egui over the shared design system (crate::panel) — the same knobs,
// glass, and glyphs as the standalone app, laid out for the 56 host
// parameters in src/host_params.rs. Everything a host format needs to show
// this panel is the `ParamHost` trait; the AU front end (src/au/cocoa.rs)
// implements it over its parameter atomics + AUEventListener notifications,
// and a future CLAP/VST3 editor can implement it over nice-plug's
// GuiContext without touching this file.

use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Stroke};
use std::collections::HashMap;
use std::sync::Arc;

use crate::host_params::{self, ChoiceDef, Display, FloatDef, ParamDef};
use crate::panel::{
    self, card, fmt_hz, fmt_pct, fmt_time, knob_sized, rail_shapes, segmented, sublegend,
    tracked, waveform_selector, Textures, CYAN, TXT_LOW,
};

/// Logical size of the editor window; the backdrop is baked at this size.
/// The panel now lays out the full synth surface (oscillators 1-3, the
/// source mixer, unison, routing, and the extended reverb), so it is taller
/// than the original four-row layout.
pub const EDITOR_WIDTH: u32 = 1200;
pub const EDITOR_HEIGHT: u32 = 810;

/// What the editor needs from its host: parameter access by table index
/// (the index into `host_params::param_defs()`, which is also the AU
/// parameter ID), plus automation gesture brackets so hosts record touched
/// parameters correctly.
pub trait ParamHost: Send + Sync {
    fn get(&self, index: usize) -> f32;
    fn set(&self, index: usize, value: f32);
    fn begin_gesture(&self, index: usize);
    fn end_gesture(&self, index: usize);
}

pub struct EditorState {
    host: Arc<dyn ParamHost>,
    defs: Vec<ParamDef>,
    index_of: HashMap<&'static str, usize>,
    textures: Option<Textures>,
    /// Parameter currently inside a begin/end gesture bracket.
    active_gesture: Option<usize>,
    /// Whether the active parameter moved this frame.
    touched: bool,
}

impl EditorState {
    pub fn new(host: Arc<dyn ParamHost>) -> Self {
        let defs = host_params::param_defs();
        let index_of = defs.iter().enumerate().map(|(i, d)| (d.id(), i)).collect();
        Self { host, defs, index_of, textures: None, active_gesture: None, touched: false }
    }

    fn float(&self, id: &str) -> (usize, &FloatDef) {
        let idx = self.index_of[id];
        match &self.defs[idx] {
            ParamDef::Float(fd) => (idx, fd),
            ParamDef::Choice(_) => unreachable!("{id} is a selector"),
        }
    }

    fn choice(&self, id: &str) -> (usize, &ChoiceDef) {
        let idx = self.index_of[id];
        match &self.defs[idx] {
            ParamDef::Choice(cd) => (idx, cd),
            ParamDef::Float(_) => unreachable!("{id} is a float"),
        }
    }

    /// Open (or move) the gesture bracket to this parameter.
    fn touch(&mut self, idx: usize) {
        if self.active_gesture != Some(idx) {
            if let Some(prev) = self.active_gesture.take() {
                self.host.end_gesture(prev);
            }
            self.host.begin_gesture(idx);
            self.active_gesture = Some(idx);
        }
        self.touched = true;
    }

    /// A table-bound knob. `label` overrides the def's display name (the
    /// rhythm card strips voice prefixes); `compact` selects pad density.
    fn pknob(&mut self, ui: &mut egui::Ui, id: &str, label: Option<&str>, compact: bool) {
        let (idx, fd) = self.float(id);
        let (name, min, max, default, display) = (fd.name, fd.min, fd.max, fd.default, fd.display);
        let label = label.unwrap_or(name);
        let logarithmic = matches!(display, Display::Seconds | Display::Hertz);
        let mut value = self.host.get(idx);
        let fmt = move |v: f32| match display {
            Display::Percent => fmt_pct(v),
            Display::Fraction => format!("{:.2}", v),
            Display::Seconds => fmt_time(v),
            Display::Hertz => fmt_hz(v),
            Display::Plain(unit) => format!("{:.1}{}", v, unit),
        };
        if knob_sized(ui, label, &mut value, min, max, default, logarithmic, fmt, compact) {
            self.touch(idx);
            self.host.set(idx, value);
        }
    }

    fn wave_selector(&mut self, ui: &mut egui::Ui, id: &str) {
        let (idx, _) = self.choice(id);
        let current = (self.host.get(idx).round().max(0.0) as usize).min(3);
        let mut selected = host_params::WAVEFORM_VARIANTS[current];
        if waveform_selector(ui, id, &mut selected) {
            let new_index = host_params::WAVEFORM_VARIANTS
                .iter()
                .position(|w| *w == selected)
                .unwrap_or(current);
            self.touch(idx);
            self.host.set(idx, new_index as f32);
        }
    }

    fn choice_selector(&mut self, ui: &mut egui::Ui, id: &str) {
        let (idx, cd) = self.choice(id);
        let variants = cd.variants;
        let current = (self.host.get(idx).round().max(0.0) as usize).min(variants.len() - 1);
        if let Some(new_index) = segmented(ui, id, variants, current) {
            self.touch(idx);
            self.host.set(idx, new_index as f32);
        }
    }

    /// One drum voice: a sublegend over its compact knobs. Each entry is an
    /// (id, short label) pair — the group header carries the instrument name
    /// (spelled out in the host list), so the knob shows just the parameter.
    fn drum_group(&mut self, ui: &mut egui::Ui, title: &str, controls: &[(&str, &str)]) {
        ui.vertical(|ui| {
            ui.add_space(2.0);
            ui.label(sublegend(title));
            ui.horizontal(|ui| {
                for (id, label) in controls {
                    self.pknob(ui, id, Some(label), true);
                }
            });
        });
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        if self.textures.is_none() {
            self.textures = Some(Textures::bake(
                ctx,
                EDITOR_WIDTH as usize,
                EDITOR_HEIGHT as usize,
            ));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let screen = ui.max_rect();
                let painter = ui.painter();

                // Baked aurora backdrop, edge to edge
                if let Some(t) = &self.textures {
                    painter.image(
                        t.backdrop.id(),
                        screen,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }

                // Graphite header rail with the wordmark
                let rail = Rect::from_min_size(screen.min, vec2(screen.width(), 40.0));
                for shape in rail_shapes(rail) {
                    painter.add(shape);
                }
                painter.text(
                    pos2(rail.left() + 16.0, rail.center().y),
                    Align2::LEFT_CENTER,
                    tracked("Patina"),
                    FontId::proportional(17.0),
                    Color32::from_rgb(0xee, 0xf4, 0xf6),
                );
                painter.circle_filled(pos2(rail.left() + 96.0, rail.center().y), 3.0, CYAN);
                painter.text(
                    pos2(rail.left() + 108.0, rail.center().y),
                    Align2::LEFT_CENTER,
                    tracked("circuit-modeled polyphonic synthesizer"),
                    FontId::proportional(9.0),
                    TXT_LOW,
                );
                painter.line_segment(
                    [pos2(rail.left(), rail.bottom()), pos2(rail.right(), rail.bottom())],
                    Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x35, 0xdf, 0xf5, 60)),
                );

                let content = Rect::from_min_max(
                    pos2(screen.left() + 14.0, rail.bottom() + 10.0),
                    screen.max - vec2(14.0, 10.0),
                );
                let mut panel_ui = ui.new_child(
                    egui::UiBuilder::new().max_rect(content).layout(
                        egui::Layout::top_down(egui::Align::LEFT),
                    ),
                );
                self.panel_body(&mut panel_ui);
            });

        // Close the gesture bracket once the pointer lets go and nothing
        // moved this frame (scroll/double-click settle one frame later).
        if let Some(idx) = self.active_gesture {
            let pointer_down = ctx.input(|i| i.pointer.any_down());
            if !pointer_down && !self.touched {
                self.host.end_gesture(idx);
                self.active_gesture = None;
            }
        }
        self.touched = false;

        // Hosts automate parameters while the panel is open — keep readouts
        // live even without local input.
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }

    fn panel_body(&mut self, ui: &mut egui::Ui) {
        // Cards borrow the frost cache mutably one at a time; take the
        // textures out for the frame.
        let mut tex = self.textures.take();

        // Row 1 — Oscillator 1 (circuit, waveform, core knobs) + LFO
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| card(ui, "Oscillator", tex.as_mut(), None, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.add_space(6.0);
                        self.choice_selector(ui, "circuit");
                        ui.add_space(6.0);
                        self.wave_selector(ui, "waveform");
                    });
                    self.pknob(ui, "volume", None, false);
                    self.pknob(ui, "detune", None, false);
                    self.pknob(ui, "pulse_width", Some("Pulse W"), false);
                    self.pknob(ui, "noise", None, false);
                    self.pknob(ui, "sub", Some("Sub"), false);
                    self.pknob(ui, "glide", Some("Glide"), false);
                });
            }));
            ui.vertical(|ui| card(ui, "LFO", tex.as_mut(), None, |ui| {
                ui.horizontal(|ui| {
                    self.pknob(ui, "lfo_rate", Some("Rate"), false);
                    self.pknob(ui, "lfo_shape", Some("Shape"), false);
                    self.pknob(ui, "lfo_pitch", Some("Pitch"), false);
                    self.pknob(ui, "lfo_filter", Some("Filter"), false);
                    self.pknob(ui, "lfo_pwm", Some("PWM"), false);
                });
            }));
        });
        ui.add_space(8.0);

        // Row 2 — Oscillators 2 & 3 + routing (sync, ring, FM, key track)
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| card(ui, "Oscillator 2", tex.as_mut(), None, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.add_space(14.0);
                        self.wave_selector(ui, "osc2_wave");
                    });
                    self.pknob(ui, "osc2_pitch", Some("Pitch"), false);
                    self.pknob(ui, "osc2_level", Some("Level"), false);
                });
            }));
            ui.vertical(|ui| card(ui, "Oscillator 3", tex.as_mut(), None, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.add_space(14.0);
                        self.wave_selector(ui, "osc3_wave");
                    });
                    self.pknob(ui, "osc3_pitch", Some("Pitch"), false);
                    self.pknob(ui, "osc3_level", Some("Level"), false);
                });
            }));
            ui.vertical(|ui| card(ui, "Routing", tex.as_mut(), None, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.add_space(14.0);
                        self.choice_selector(ui, "sync");
                    });
                    self.pknob(ui, "ring", Some("Ring"), false);
                    self.pknob(ui, "osc_fm", Some("FM"), false);
                    self.pknob(ui, "key_track", Some("Key Trk"), false);
                });
            }));
        });
        ui.add_space(8.0);

        // Row 3 — Oscillator 1 source mixer + unison
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| card(ui, "Osc 1 Mixer", tex.as_mut(), None, |ui| {
                ui.horizontal(|ui| {
                    self.pknob(ui, "mix_saw", Some("Sawtooth"), false);
                    self.pknob(ui, "mix_pulse", Some("Pulse"), false);
                    self.pknob(ui, "mix_tri", Some("Triangle"), false);
                    self.pknob(ui, "mix_sine", Some("Sine"), false);
                });
            }));
            ui.vertical(|ui| card(ui, "Unison", tex.as_mut(), None, |ui| {
                ui.horizontal(|ui| {
                    self.pknob(ui, "unison", Some("Voices"), false);
                    self.pknob(ui, "unison_detune", Some("Detune"), false);
                });
            }));
        });
        ui.add_space(8.0);

        // Row 4 — Amplitude envelope + filter + filter envelope
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| card(ui, "Amp Envelope", tex.as_mut(), None, |ui| {
                ui.horizontal(|ui| {
                    self.pknob(ui, "attack", Some("Attack"), false);
                    self.pknob(ui, "decay", Some("Decay"), false);
                    self.pknob(ui, "sustain", Some("Sustain"), false);
                    self.pknob(ui, "release", Some("Release"), false);
                });
            }));
            ui.vertical(|ui| card(ui, "Filter", tex.as_mut(), None, |ui| {
                ui.horizontal(|ui| {
                    self.pknob(ui, "cutoff", Some("Cutoff"), false);
                    self.pknob(ui, "resonance", Some("Resonance"), false);
                    self.pknob(ui, "drive", Some("Drive"), false);
                    self.pknob(ui, "saturation", Some("Saturation"), false);
                    self.pknob(ui, "hpf", Some("High-Pass"), false);
                });
            }));
            ui.vertical(|ui| card(ui, "Filter Envelope", tex.as_mut(), None, |ui| {
                ui.horizontal(|ui| {
                    self.pknob(ui, "filter_env", Some("Amount"), false);
                    self.pknob(ui, "filter_attack", Some("Attack"), false);
                    self.pknob(ui, "filter_decay", Some("Decay"), false);
                    self.pknob(ui, "filter_sustain", Some("Sustain"), false);
                    self.pknob(ui, "filter_release", Some("Release"), false);
                });
            }));
        });
        ui.add_space(8.0);

        // Row 5 — Space (fuzz/spring/reverb) + chorus + tape
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| card(ui, "Space", tex.as_mut(), None, |ui| {
                ui.horizontal(|ui| {
                    self.pknob(ui, "fuzz", Some("Fuzz"), false);
                    self.pknob(ui, "spring", Some("Spring"), false);
                    self.pknob(ui, "reverb_decay", Some("Rvb Decay"), false);
                    self.pknob(ui, "reverb_wet", Some("Rvb Mix"), false);
                    self.pknob(ui, "reverb_tone", Some("Rvb Tone"), false);
                    self.pknob(ui, "reverb_pre", Some("Rvb Pre"), false);
                });
            }));
            ui.vertical(|ui| card(ui, "Chorus", tex.as_mut(), None, |ui| {
                ui.vertical(|ui| {
                    ui.add_space(4.0);
                    self.choice_selector(ui, "chorus_mode");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        self.pknob(ui, "chorus_rate", Some("Rate"), false);
                        self.pknob(ui, "chorus_depth", Some("Depth"), false);
                    });
                });
            }));
            ui.vertical(|ui| card(ui, "Tape", tex.as_mut(), None, |ui| {
                ui.horizontal(|ui| {
                    self.pknob(ui, "tape_wow", Some("Wow"), false);
                    self.pknob(ui, "tape_flutter", Some("Flutter"), false);
                    self.pknob(ui, "tape_drive", Some("Drive"), false);
                    self.pknob(ui, "tape_age", Some("Age"), false);
                });
            }));
        });
        ui.add_space(8.0);

        // Row 6 — the 909 rhythm section
        card(ui, "Rhythm Section · 909 · MIDI CH 10", tex.as_mut(), None, |ui| {
            // 23 pads don't fit on one line at panel density — the last of
            // them used to run off the right edge — so the board wraps onto
            // two rows the way the hardware splits drums from cymbals.
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.horizontal(|ui| {
                self.drum_group(ui, "Kick", &[
                    ("bd_level", "Level"), ("bd_tune", "Tune"), ("bd_attack", "Attack"),
                    ("bd_decay", "Decay"), ("bd_sweep", "Sweep"), ("bd_drive", "Drive"),
                ]);
                panel::vseparator(ui, 90.0);
                self.drum_group(ui, "Snare", &[
                    ("sd_level", "Level"), ("sd_tune", "Tune"), ("sd_tone", "Tone"),
                    ("sd_snappy", "Snappy"), ("sd_decay", "Decay"),
                ]);
                panel::vseparator(ui, 90.0);
                self.drum_group(ui, "Rim · Clap", &[
                    ("rs_level", "Rim Lvl"), ("rs_tune", "Rim Tune"),
                    ("cp_level", "Clap Lvl"), ("cp_decay", "Clap Dec"),
                ]);
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                self.drum_group(ui, "Hats", &[
                    ("hh_level", "Level"), ("hh_tune", "Tune"), ("hh_metal", "Metal"),
                    ("ch_decay", "Closed"), ("oh_decay", "Open"),
                ]);
                panel::vseparator(ui, 90.0);
                self.drum_group(ui, "Bus", &[("dr_drive", "Drive"), ("dr_tone", "Tone")]);
            });
        });

        self.textures = tex;
    }
}

/// The waveform selector widget draws exactly four cells; pin the table's
/// variant count to that expectation.
const _: () = assert!(host_params::WAVEFORM_VARIANTS.len() == 4);
