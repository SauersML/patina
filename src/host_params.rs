// THE canonical host-facing parameter table, shared by every plugin front
// end: CLAP/VST3 (src/plugin.rs) and Audio Unit (src/au/). Each front end
// derives its host parameter list, defaults, state save/restore, and
// per-block application from this table, so the formats cannot drift.
//
// Adding an engine knob = one engine setter + one line in `param_defs()`.
//
// ORDER IS ABI: the Audio Unit uses each entry's index in `param_defs()` as
// its AudioUnitParameterID, which Logic stores in automation lanes and
// project files. Append new parameters at the end of their section freely,
// but never remove or reorder existing entries.

use crate::chorus::ChorusMode;
use crate::oscillator::Waveform;
use crate::voice_manager::VoiceManager;

/// Voice count every plugin front end allocates.
pub const NUM_VOICES: usize = 8;
/// Standard pitch-wheel range, in semitones each direction.
pub const PITCH_BEND_SEMITONES: f32 = 2.0;

/// How a value should be presented to the user; front ends map this to
/// their own formatter/unit vocabulary. `Seconds` and `Hertz` are
/// perceptually logarithmic and get a skewed/log control mapping.
#[derive(Clone, Copy, PartialEq)]
pub enum Display {
    /// 0..1 shown as a percentage.
    Percent,
    /// 0..1 shown as a bare panel rotation, exactly like the hardware.
    Fraction,
    Seconds,
    Hertz,
    /// Linear, with a fixed unit suffix (" ct", " oct", ...).
    Plain(&'static str),
}

pub struct FloatDef {
    pub id: &'static str,
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub display: Display,
    /// Guarded setters are only called when the value changes — they swap
    /// voice banks, re-randomize offsets, or re-run self-calibration.
    pub guarded: bool,
    pub apply: fn(&mut VoiceManager, f32),
}

/// A selector parameter: a small set of named positions. Always guarded.
pub struct ChoiceDef {
    pub id: &'static str,
    pub name: &'static str,
    pub variants: &'static [&'static str],
    pub default: usize,
    pub apply: fn(&mut VoiceManager, usize),
}

pub enum ParamDef {
    Float(FloatDef),
    Choice(ChoiceDef),
}

impl ParamDef {
    pub fn id(&self) -> &'static str {
        match self {
            ParamDef::Float(f) => f.id,
            ParamDef::Choice(c) => c.id,
        }
    }

    pub fn default_value(&self) -> f32 {
        match self {
            ParamDef::Float(f) => f.default,
            ParamDef::Choice(c) => c.default as f32,
        }
    }
}

/// Engine order for the waveform selector; `WaveformParam` in plugin.rs
/// mirrors this order (checked by test).
pub const WAVEFORM_VARIANTS: [Waveform; 4] =
    [Waveform::Sine, Waveform::Triangle, Waveform::Sawtooth, Waveform::Square];
pub const CHORUS_VARIANTS: [ChorusMode; 5] =
    [ChorusMode::Off, ChorusMode::I, ChorusMode::II, ChorusMode::III, ChorusMode::IV];

fn f(
    id: &'static str,
    name: &'static str,
    default: f32,
    min: f32,
    max: f32,
    display: Display,
    apply: fn(&mut VoiceManager, f32),
) -> ParamDef {
    ParamDef::Float(FloatDef { id, name, min, max, default, display, guarded: false, apply })
}

fn pct(id: &'static str, name: &'static str, default: f32, apply: fn(&mut VoiceManager, f32)) -> ParamDef {
    f(id, name, default, 0.0, 1.0, Display::Percent, apply)
}

/// A 0..1 panel rotation displayed without a % sign.
fn frac(id: &'static str, name: &'static str, default: f32, apply: fn(&mut VoiceManager, f32)) -> ParamDef {
    f(id, name, default, 0.0, 1.0, Display::Fraction, apply)
}

fn secs(id: &'static str, name: &'static str, default: f32, min: f32, max: f32, apply: fn(&mut VoiceManager, f32)) -> ParamDef {
    f(id, name, default, min, max, Display::Seconds, apply)
}

fn hz(id: &'static str, name: &'static str, default: f32, min: f32, max: f32, apply: fn(&mut VoiceManager, f32)) -> ParamDef {
    f(id, name, default, min, max, Display::Hertz, apply)
}

fn plain(id: &'static str, name: &'static str, default: f32, min: f32, max: f32, unit: &'static str, apply: fn(&mut VoiceManager, f32)) -> ParamDef {
    f(id, name, default, min, max, Display::Plain(unit), apply)
}

fn guarded(def: ParamDef) -> ParamDef {
    match def {
        ParamDef::Float(mut fd) => {
            fd.guarded = true;
            ParamDef::Float(fd)
        }
        choice => choice,
    }
}

/// THE table. Order here is the order hosts display parameters in, and the
/// index is the Audio Unit parameter ID (see header comment).
/// Ranges and defaults mirror the engine's own clamps and ParamValues.
#[rustfmt::skip]
pub fn param_defs() -> Vec<ParamDef> {
    vec![
        // Oscillator
        ParamDef::Choice(ChoiceDef {
            id: "waveform", name: "Waveform",
            variants: &["Sine", "Triangle", "Sawtooth", "Square"],
            default: 2,
            apply: |vm, i| vm.set_waveform(WAVEFORM_VARIANTS[i.min(3)]),
        }),
        pct("volume",   "Volume", 0.5,                                  |vm, v| vm.set_volume(v)),
        plain("detune", "Detune", 7.0, 0.0, 30.0, " ct",                |vm, v| vm.set_detune(v)),
        frac("pw",      "Pulse Width", 0.5,                             |vm, v| vm.set_pulse_width(v)),
        pct("noise",    "Noise", 0.0,                                   |vm, v| vm.set_noise(v)),

        // LFO
        hz("lforate",    "LFO Rate", 1.0, 0.1, 30.0,                    |vm, v| vm.set_lfo_rate(v)),
        pct("lfoshape",  "LFO Shape", 0.5,                              |vm, v| vm.set_lfo_shape(v)),
        plain("lfopitch", "LFO > Pitch", 0.0, 0.0, 200.0, " ct",        |vm, v| vm.set_lfo_pitch(v)),
        plain("lfofilt", "LFO > Filter", 0.0, 0.0, 4.0, " oct",         |vm, v| vm.set_lfo_filter(v)),
        plain("lfopwm",  "LFO > PWM", 0.0, 0.0, 0.45, "",               |vm, v| vm.set_lfo_pwm(v)),

        // Amplitude envelope
        secs("attack",   "Attack", 0.1, 0.01, 2.0,                      |vm, v| vm.set_attack(v)),
        secs("decay",    "Decay", 0.1, 0.01, 2.0,                       |vm, v| vm.set_decay(v)),
        pct("sustain",   "Sustain", 0.7,                                |vm, v| vm.set_sustain(v)),
        secs("release",  "Release", 0.2, 0.01, 2.0,                     |vm, v| vm.set_release(v)),

        // Filter
        hz("cutoff",     "Cutoff", 15000.0, 20.0, 20000.0,              |vm, v| vm.set_filter_cutoff(v)),
        plain("reso",    "Resonance", 0.0, 0.0, 4.0, "",                |vm, v| vm.set_filter_resonance(v)),
        plain("drive",   "Drive", 1.0, 0.1, 5.0, "",                    |vm, v| vm.set_filter_drive(v)),
        plain("sat",     "Saturation", 1.0, 0.0, 2.0, "",               |vm, v| vm.set_filter_saturation(v)),
        hz("hpf",        "High-Pass", 16.0, 16.0, 8000.0,               |vm, v| vm.set_hpf_cutoff(v)),

        // Filter envelope
        plain("fenvamt", "Filter Env", 0.0, -5.0, 5.0, " oct",          |vm, v| vm.set_filter_env_amount(v)),
        secs("fenvatk",  "Filter Attack", 0.005, 0.001, 2.0,            |vm, v| vm.set_filter_attack(v)),
        secs("fenvdec",  "Filter Decay", 0.3, 0.01, 2.0,                |vm, v| vm.set_filter_decay(v)),
        pct("fenvsus",   "Filter Sustain", 0.0,                         |vm, v| vm.set_filter_sustain(v)),
        secs("fenvrel",  "Filter Release", 0.3, 0.01, 2.0,              |vm, v| vm.set_filter_release(v)),

        // Effects
        pct("fuzz",      "Fuzz", 0.0,                                   |vm, v| vm.set_fuzz(v)),
        pct("spring",    "Spring Reverb", 0.0,                          |vm, v| vm.set_spring(v)),
        frac("rvbdecay", "Reverb Decay", 0.5,                           |vm, v| vm.set_reverb_decay(v)),
        pct("rvbwet",    "Reverb Mix", 0.5,                             |vm, v| vm.set_reverb_wet(v)),
        ParamDef::Choice(ChoiceDef {
            id: "chmode", name: "Chorus Mode",
            variants: &["Off", "I", "II", "III", "IV"],
            default: 0,
            apply: |vm, i| vm.set_chorus_mode(CHORUS_VARIANTS[i.min(4)]),
        }),
        guarded(hz("chrate",    "Chorus Rate", 0.5, 0.1, 10.0,          |vm, v| vm.set_chorus_rate(v))),
        guarded(pct("chdepth",  "Chorus Depth", 0.3,                    |vm, v| vm.set_chorus_depth(v))),
        pct("tpwow",     "Tape Wow", 0.0,                               |vm, v| vm.set_tape_wow(v)),
        pct("tpflut",    "Tape Flutter", 0.0,                           |vm, v| vm.set_tape_flutter(v)),
        guarded(pct("tpdrive",  "Tape Drive", 0.0,                      |vm, v| vm.set_tape_drive(v))),
        guarded(pct("tpage",    "Tape Age", 0.0,                        |vm, v| vm.set_tape_age(v))),

        // Rhythm section (the 909 board; triggered on MIDI channel 10).
        // Panel knobs are unitless rotations, exactly like the hardware.
        pct("bdlevel",   "BD Level", 0.8,                               |vm, v| vm.set_bd_level(v)),
        frac("bdtune",   "BD Tune", 0.35,                               |vm, v| vm.set_bd_tune(v)),
        frac("bdattack", "BD Attack", 0.5,                              |vm, v| vm.set_bd_attack(v)),
        frac("bddecay",  "BD Decay", 0.45,                              |vm, v| vm.set_bd_decay(v)),
        frac("bdsweep",  "BD Sweep", 0.5,                               |vm, v| vm.set_bd_sweep(v)),
        frac("bddrive",  "BD Drive", 0.25,                              |vm, v| vm.set_bd_drive(v)),
        pct("sdlevel",   "SD Level", 0.75,                              |vm, v| vm.set_sd_level(v)),
        frac("sdtune",   "SD Tune", 0.4,                                |vm, v| vm.set_sd_tune(v)),
        frac("sdtone",   "SD Tone", 0.5,                                |vm, v| vm.set_sd_tone(v)),
        frac("sdsnappy", "SD Snappy", 0.6,                              |vm, v| vm.set_sd_snappy(v)),
        frac("sddecay",  "SD Decay", 0.5,                               |vm, v| vm.set_sd_decay(v)),
        pct("rslevel",   "RS Level", 0.7,                               |vm, v| vm.set_rs_level(v)),
        frac("rstune",   "RS Tune", 0.5,                                |vm, v| vm.set_rs_tune(v)),
        pct("cplevel",   "CP Level", 0.75,                              |vm, v| vm.set_cp_level(v)),
        frac("cpdecay",  "CP Decay", 0.5,                               |vm, v| vm.set_cp_decay(v)),
        pct("hhlevel",   "HH Level", 0.7,                               |vm, v| vm.set_hh_level(v)),
        frac("hhtune",   "HH Tune", 0.5,                                |vm, v| vm.set_hh_tune(v)),
        frac("hhmetal",  "HH Metal", 0.65,                              |vm, v| vm.set_hh_metal(v)),
        frac("chdecay",  "CH Decay", 0.35,                              |vm, v| vm.set_ch_decay(v)),
        frac("ohdecay",  "OH Decay", 0.5,                               |vm, v| vm.set_oh_decay(v)),
        pct("drdrive",   "Drum Drive", 0.0,                             |vm, v| vm.set_drum_drive(v)),
    ]
}

/// Route one MIDI note-on to the keyboard voices or, on GM channel 10
/// (0-indexed 9), the 909 board. Velocity is 0..1.
pub fn note_on(vm: &mut VoiceManager, channel: u8, note: u8, velocity: f32) {
    if channel == 9 {
        vm.note_on_channel(note, velocity, crate::drums::DRUM_CHANNEL);
    } else {
        vm.note_on(note, velocity);
    }
}

pub fn note_off(vm: &mut VoiceManager, channel: u8, note: u8) {
    if channel != 9 {
        vm.note_off(note);
    }
}
