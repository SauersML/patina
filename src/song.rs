// src/song.rs
//
// A tiny text-based song format and player, used via `patina --play <file>`.
// Notes and parameter automation go through the VoiceManager exactly as if
// they came from the on-screen keyboard, a MIDI device, or the UI sliders.
//
// Format (one directive or a run of event tokens per line, `#` starts a comment):
//
//   bpm 100                  # global tempo (set once, at the top)
//   gate 0.85                # fraction of each note's duration it is held (default 0.9)
//
//   track lead vel=0.9 len=0.5   # start a note track; tracks play in parallel.
//                                # vel = default velocity (0..1)
//                                # len = default token duration in beats (default 1)
//     E5:2 D5 C5 R:4 [C4 E4 G4]:2@0.6  | A4
//
//   track beat kit=909 len=0.5   # a drum track: kit= routes it to the
//     BD SD:0.5 [BD CH] OH@0.6   # rhythm section. Tokens are drum names
//                                # (BD SD RS CP CH OH) or GM notes; velocity
//                                # is the accent bus (@1 = full accent).
//
// Note-track tokens:
//   C4  F#3  Eb5  60      note names (C4 = MIDI 60) or raw MIDI numbers
//   [C4 E4 G4]            chord (notes start and stop together)
//   R  or  .              rest
//   :2                    duration suffix, in beats (floats allowed)
//   @0.7                  velocity suffix (0..1)
//   |                     bar line, ignored (readability only)
//
// Automation tracks ramp a synth parameter through breakpoints:
//
//   automate cutoff
//     400 8000:16@exp R:8 400:4@smooth
//
//   The first token must be a plain value (the starting point). After that,
//   V:D@shape means "ramp to V over D beats". R:D / .:D holds the current
//   value. Shapes: lin (default), exp (musical/geometric — right for
//   frequencies), log (fast start), smooth (S-curve), step (jump at the end).
//
// Automatable parameters: volume, waveform (0=sine 1=square 2=saw 3=tri,
// use plain sets), detune, cutoff, resonance, drive, saturation, hpf
// (high-pass cutoff Hz, 16 = off), fuzz (0..1 germanium fuzz), noise
// (0..1 shared noise into the voices), spring (0..1 spring reverb wet),
// glide (portamento seconds, 0 = off), sub (0..1 octave-down square),
// osc2_wave/osc3_wave (0-3), osc2_pitch/osc3_pitch (semitones -24..24),
// osc2_level/osc3_level (0..1; `waveform` is a macro setting all three
// oscillators, per-osc waves override after it), pulse_width (0.05..0.95), lfo_rate (Hz), lfo_shape (0=saw 0.5=tri
// 1=ramp), lfo_pitch (vibrato cents), lfo_filter (octaves), lfo_pwm
// (width swing 0..0.45), attack,
// decay, sustain, release, filter_env (octaves, -5..+5), filter_attack,
// filter_decay, filter_sustain, filter_release, reverb_decay, reverb_wet,
// chorus_mode (0=off..4=IV, use plain sets), chorus_rate, chorus_depth,
// tape_wow, tape_flutter, tape_drive, tape_age.
//
// Rhythm section (the 909 board; all 0..1 panel knobs): bd_level, bd_tune,
// bd_attack, bd_decay, bd_sweep, bd_drive, sd_level, sd_tune, sd_tone,
// sd_snappy, sd_decay, rs_level, rs_tune, cp_level, cp_decay, hh_level,
// hh_tune, hh_metal, ch_decay, oh_decay, dr_drive.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::chorus::ChorusMode;
use crate::oscillator::Waveform;
use crate::voice_manager::{ParamValues, VoiceManager};

// Automation curves are sampled at this many points per beat
const AUTOMATION_STEPS_PER_BEAT: f64 = 32.0;

/// How a parameter's range is traversed by a fader or knob.
#[derive(Clone, Copy, PartialEq)]
pub enum Curve {
    Lin,
    Log,
    Step,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Param {
    Volume,
    WaveformSel,
    Detune,
    Cutoff,
    Resonance,
    Drive,
    Saturation,
    Attack,
    Decay,
    Sustain,
    Release,
    HpfCutoff,
    FuzzAmount,
    NoiseLevel,
    SpringWet,
    Glide,
    SubLevel,
    Osc2Wave,
    Osc2Pitch,
    Osc2Level,
    Osc3Wave,
    Osc3Pitch,
    Osc3Level,
    CircuitSel,
    KeyTrack,
    OscFm,
    SyncSel,
    RingAmount,
    UiOctave,
    PitchBendSemis,
    ModWheel,
    SustainPedal,
    PulseWidth,
    LfoRate,
    LfoShape,
    LfoPitch,
    LfoFilter,
    LfoPwm,
    FilterEnvAmount,
    FilterAttack,
    FilterDecay,
    FilterSustain,
    FilterRelease,
    ReverbDecay,
    ReverbWet,
    ChorusModeSel,
    ChorusRate,
    ChorusDepth,
    TapeWow,
    TapeFlutter,
    TapeDrive,
    TapeAge,
    // The rhythm section: one shared 909 board, so like the effects and
    // the LFO these are bus-level parameters (0..1 panel knobs)
    BdLevel,
    BdTune,
    BdAttack,
    BdDecay,
    BdSweep,
    BdDrive,
    SdLevel,
    SdTune,
    SdTone,
    SdSnappy,
    SdDecay,
    RsLevel,
    RsTune,
    CpLevel,
    CpDecay,
    HhLevel,
    HhTune,
    HhMetal,
    ChDecay,
    OhDecay,
    DrumDrive,
}

pub(crate) fn waveform_from_value(value: f32) -> Waveform {
    match value.round() as i32 {
        i32::MIN..=0 => Waveform::Sine,
        1 => Waveform::Square,
        2 => Waveform::Sawtooth,
        _ => Waveform::Triangle,
    }
}

impl Param {
    /// The MIDI CC chart: every automatable parameter is reachable from a
    /// controller. Standard assignments where they exist (1 mod wheel,
    /// 5 portamento, 7 volume, 64 sustain, 71/74 resonance/cutoff,
    /// 72/73/75/79 envelope, 91/93 sends); the 102-119 block carries the
    /// engine-specific rest.
    pub fn from_cc(cc: u8) -> Option<Param> {
        Some(match cc {
            1 => Param::ModWheel,
            // The rhythm section claims the 20-31 general-purpose block
            // plus 52-60 — every 909 knob is a controller away
            20 => Param::BdLevel,
            21 => Param::BdTune,
            22 => Param::BdAttack,
            23 => Param::BdDecay,
            24 => Param::BdSweep,
            25 => Param::BdDrive,
            26 => Param::SdLevel,
            27 => Param::SdTune,
            28 => Param::SdTone,
            29 => Param::SdSnappy,
            30 => Param::SdDecay,
            31 => Param::RsLevel,
            52 => Param::RsTune,
            53 => Param::CpLevel,
            54 => Param::CpDecay,
            55 => Param::HhLevel,
            56 => Param::HhTune,
            57 => Param::HhMetal,
            58 => Param::ChDecay,
            59 => Param::OhDecay,
            60 => Param::DrumDrive,
            5 => Param::Glide,
            7 => Param::Volume,
            64 => Param::SustainPedal,
            71 => Param::Resonance,
            72 => Param::Release,
            73 => Param::Attack,
            74 => Param::Cutoff,
            75 => Param::Decay,
            76 => Param::LfoRate,
            77 => Param::LfoPitch,
            78 => Param::LfoFilter,
            79 => Param::Sustain,
            80 => Param::SubLevel,
            81 => Param::NoiseLevel,
            82 => Param::PulseWidth,
            83 => Param::Detune,
            85 => Param::Osc2Level,
            86 => Param::Osc2Pitch,
            87 => Param::Osc3Level,
            88 => Param::Osc3Pitch,
            89 => Param::OscFm,
            90 => Param::RingAmount,
            91 => Param::ReverbWet,
            92 => Param::TapeWow,
            93 => Param::ChorusDepth,
            94 => Param::TapeFlutter,
            95 => Param::SpringWet,
            102 => Param::HpfCutoff,
            103 => Param::Drive,
            104 => Param::Saturation,
            105 => Param::KeyTrack,
            106 => Param::FilterEnvAmount,
            107 => Param::FilterAttack,
            108 => Param::FilterDecay,
            109 => Param::FilterSustain,
            110 => Param::FilterRelease,
            111 => Param::ChorusRate,
            112 => Param::ChorusModeSel,
            113 => Param::WaveformSel,
            114 => Param::Osc2Wave,
            115 => Param::Osc3Wave,
            116 => Param::CircuitSel,
            117 => Param::SyncSel,
            118 => Param::TapeDrive,
            119 => Param::TapeAge,
            _ => return None,
        })
    }

    /// Map a normalized controller position (0..1) into this parameter's
    /// native range, honoring its curve.
    pub fn midi_value(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let (lo, hi, curve) = self.range();
        match curve {
            Curve::Lin => lo + (hi - lo) * t,
            Curve::Log => lo * (hi / lo).powf(t),
            Curve::Step => (lo + (hi - lo) * t).round(),
        }
    }

    /// THE range table — the single source of truth for every parameter's
    /// bounds and taper. Knobs, MIDI CCs, and any future surface all read
    /// this, so a range change lands everywhere at once.
    pub fn range(self) -> (f32, f32, Curve) {
        use Curve::*;
        match self {
            Param::Volume | Param::Sustain | Param::FilterSustain
            | Param::SubLevel | Param::NoiseLevel | Param::OscFm
            | Param::RingAmount | Param::ReverbWet | Param::ChorusDepth
            | Param::TapeWow | Param::TapeFlutter | Param::TapeDrive
            | Param::TapeAge | Param::SpringWet | Param::KeyTrack
            | Param::FuzzAmount | Param::ModWheel | Param::LfoShape
            | Param::SustainPedal | Param::Osc2Level | Param::Osc3Level
            // 909 panel knobs are all unitless 0..1 rotations; the
            // circuits map them onto their electrical ranges
            | Param::BdLevel | Param::BdTune | Param::BdAttack
            | Param::BdDecay | Param::BdSweep | Param::BdDrive
            | Param::SdLevel | Param::SdTune | Param::SdTone
            | Param::SdSnappy | Param::SdDecay | Param::RsLevel
            | Param::RsTune | Param::CpLevel | Param::CpDecay
            | Param::HhLevel | Param::HhTune | Param::HhMetal
            | Param::ChDecay | Param::OhDecay | Param::DrumDrive => (0.0, 1.0, Lin),
            Param::ReverbDecay => (0.0, 0.99, Lin),
            Param::PulseWidth => (0.05, 0.95, Lin),
            Param::Detune => (0.0, 30.0, Lin),
            Param::Osc2Pitch | Param::Osc3Pitch => (-24.0, 24.0, Step),
            Param::Attack | Param::Decay | Param::Release
            | Param::FilterDecay | Param::FilterRelease => (0.01, 2.0, Log),
            Param::FilterAttack => (0.001, 2.0, Log),
            Param::FilterEnvAmount => (-5.0, 5.0, Lin),
            Param::Cutoff => (20.0, 20000.0, Log),
            Param::HpfCutoff => (16.0, 8000.0, Log),
            Param::Resonance => (0.0, 4.0, Lin),
            Param::Drive => (0.1, 5.0, Lin),
            Param::Saturation => (0.0, 2.0, Lin),
            Param::Glide => (0.0, 2.0, Lin),
            Param::LfoRate => (0.1, 30.0, Log),
            Param::LfoPitch => (0.0, 200.0, Lin),
            Param::LfoFilter => (0.0, 4.0, Lin),
            Param::LfoPwm => (0.0, 0.45, Lin),
            Param::ChorusRate => (0.1, 10.0, Log),
            Param::ChorusModeSel => (0.0, 4.0, Step),
            Param::WaveformSel | Param::Osc2Wave | Param::Osc3Wave => (0.0, 3.0, Step),
            Param::CircuitSel | Param::SyncSel => (0.0, 1.0, Step),
            Param::UiOctave => (0.0, 8.0, Step),
            Param::PitchBendSemis => (-2.0, 2.0, Lin),
        }
    }
}

impl Param {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "volume" => Param::Volume,
            "waveform" => Param::WaveformSel,
            "detune" => Param::Detune,
            "cutoff" => Param::Cutoff,
            "resonance" => Param::Resonance,
            "drive" => Param::Drive,
            "saturation" => Param::Saturation,
            "attack" => Param::Attack,
            "decay" => Param::Decay,
            "sustain" => Param::Sustain,
            "release" => Param::Release,
            "hpf" => Param::HpfCutoff,
            "fuzz" => Param::FuzzAmount,
            "noise" => Param::NoiseLevel,
            "spring" => Param::SpringWet,
            "glide" => Param::Glide,
            "sub" => Param::SubLevel,
            "osc2_wave" => Param::Osc2Wave,
            "osc2_pitch" => Param::Osc2Pitch,
            "osc2_level" => Param::Osc2Level,
            "osc3_wave" => Param::Osc3Wave,
            "osc3_pitch" => Param::Osc3Pitch,
            "osc3_level" => Param::Osc3Level,
            "circuit" => Param::CircuitSel,
            "key_track" => Param::KeyTrack,
            "osc_fm" => Param::OscFm,
            "sync" => Param::SyncSel,
            "octave" => Param::UiOctave,
            "bend" => Param::PitchBendSemis,
            "mod_wheel" => Param::ModWheel,
            "pedal" => Param::SustainPedal,
            "ring" => Param::RingAmount,
            "pulse_width" => Param::PulseWidth,
            "lfo_rate" => Param::LfoRate,
            "lfo_shape" => Param::LfoShape,
            "lfo_pitch" => Param::LfoPitch,
            "lfo_filter" => Param::LfoFilter,
            "lfo_pwm" => Param::LfoPwm,
            "filter_env" => Param::FilterEnvAmount,
            "filter_attack" => Param::FilterAttack,
            "filter_decay" => Param::FilterDecay,
            "filter_sustain" => Param::FilterSustain,
            "filter_release" => Param::FilterRelease,
            "reverb_decay" => Param::ReverbDecay,
            "reverb_wet" => Param::ReverbWet,
            "chorus_mode" => Param::ChorusModeSel,
            "chorus_rate" => Param::ChorusRate,
            "chorus_depth" => Param::ChorusDepth,
            "tape_wow" => Param::TapeWow,
            "tape_flutter" => Param::TapeFlutter,
            "tape_drive" => Param::TapeDrive,
            "tape_age" => Param::TapeAge,
            "bd_level" => Param::BdLevel,
            "bd_tune" => Param::BdTune,
            "bd_attack" => Param::BdAttack,
            "bd_decay" => Param::BdDecay,
            "bd_sweep" => Param::BdSweep,
            "bd_drive" => Param::BdDrive,
            "sd_level" => Param::SdLevel,
            "sd_tune" => Param::SdTune,
            "sd_tone" => Param::SdTone,
            "sd_snappy" => Param::SdSnappy,
            "sd_decay" => Param::SdDecay,
            "rs_level" => Param::RsLevel,
            "rs_tune" => Param::RsTune,
            "cp_level" => Param::CpLevel,
            "cp_decay" => Param::CpDecay,
            "hh_level" => Param::HhLevel,
            "hh_tune" => Param::HhTune,
            "hh_metal" => Param::HhMetal,
            "ch_decay" => Param::ChDecay,
            "oh_decay" => Param::OhDecay,
            "dr_drive" => Param::DrumDrive,
            _ => return None,
        })
    }

    pub(crate) fn apply(self, vm: &mut VoiceManager, value: f32) {
        match self {
            Param::Volume => vm.set_volume(value),
            Param::WaveformSel => vm.set_waveform(waveform_from_value(value)),
            Param::Detune => vm.set_detune(value),
            Param::Cutoff => vm.set_filter_cutoff(value),
            Param::Resonance => vm.set_filter_resonance(value),
            Param::Drive => vm.set_filter_drive(value),
            Param::Saturation => vm.set_filter_saturation(value),
            Param::Attack => vm.set_attack(value),
            Param::Decay => vm.set_decay(value),
            Param::Sustain => vm.set_sustain(value),
            Param::Release => vm.set_release(value),
            Param::HpfCutoff => vm.set_hpf_cutoff(value),
            Param::FuzzAmount => vm.set_fuzz(value),
            Param::NoiseLevel => vm.set_noise(value),
            Param::SpringWet => vm.set_spring(value),
            Param::Glide => vm.set_glide(value),
            Param::SubLevel => vm.set_sub(value),
            Param::Osc2Wave => vm.set_osc_wave(1, waveform_from_value(value)),
            Param::Osc2Pitch => vm.set_osc_pitch(1, value),
            Param::Osc2Level => vm.set_osc_level(1, value),
            Param::Osc3Wave => vm.set_osc_wave(2, waveform_from_value(value)),
            Param::Osc3Pitch => vm.set_osc_pitch(2, value),
            Param::Osc3Level => vm.set_osc_level(2, value),
            Param::CircuitSel => vm.set_circuit(if value.round() as i32 >= 1 {
                crate::oscillator::CircuitModel::Arp
            } else {
                crate::oscillator::CircuitModel::Moog
            }),
            Param::KeyTrack => vm.set_key_track(value),
            Param::OscFm => vm.set_osc_fm(value),
            Param::SyncSel => vm.set_sync(value.round() as i32 >= 1),
            Param::RingAmount => vm.set_ring(value),
            Param::UiOctave => vm.set_ui_octave(value),
            Param::PitchBendSemis => vm.set_pitch_bend(value),
            Param::ModWheel => vm.set_mod_wheel(value.clamp(0.0, 1.0)),
            Param::SustainPedal => vm.set_sustain_pedal(value >= 0.5),
            Param::PulseWidth => vm.set_pulse_width(value),
            Param::LfoRate => vm.set_lfo_rate(value),
            Param::LfoShape => vm.set_lfo_shape(value),
            Param::LfoPitch => vm.set_lfo_pitch(value),
            Param::LfoFilter => vm.set_lfo_filter(value),
            Param::LfoPwm => vm.set_lfo_pwm(value),
            Param::FilterEnvAmount => vm.set_filter_env_amount(value),
            Param::FilterAttack => vm.set_filter_attack(value),
            Param::FilterDecay => vm.set_filter_decay(value),
            Param::FilterSustain => vm.set_filter_sustain(value),
            Param::FilterRelease => vm.set_filter_release(value),
            Param::ReverbDecay => vm.set_reverb_decay(value),
            Param::ReverbWet => vm.set_reverb_wet(value),
            Param::ChorusModeSel => {
                let mode = match value.round() as i32 {
                    i32::MIN..=0 => ChorusMode::Off,
                    1 => ChorusMode::I,
                    2 => ChorusMode::II,
                    3 => ChorusMode::III,
                    _ => ChorusMode::IV,
                };
                vm.set_chorus_mode(mode);
            }
            Param::ChorusRate => vm.set_chorus_rate(value),
            Param::ChorusDepth => vm.set_chorus_depth(value),
            Param::TapeWow => vm.set_tape_wow(value),
            Param::TapeFlutter => vm.set_tape_flutter(value),
            Param::TapeDrive => vm.set_tape_drive(value),
            Param::TapeAge => vm.set_tape_age(value),
            Param::BdLevel => vm.set_bd_level(value),
            Param::BdTune => vm.set_bd_tune(value),
            Param::BdAttack => vm.set_bd_attack(value),
            Param::BdDecay => vm.set_bd_decay(value),
            Param::BdSweep => vm.set_bd_sweep(value),
            Param::BdDrive => vm.set_bd_drive(value),
            Param::SdLevel => vm.set_sd_level(value),
            Param::SdTune => vm.set_sd_tune(value),
            Param::SdTone => vm.set_sd_tone(value),
            Param::SdSnappy => vm.set_sd_snappy(value),
            Param::SdDecay => vm.set_sd_decay(value),
            Param::RsLevel => vm.set_rs_level(value),
            Param::RsTune => vm.set_rs_tune(value),
            Param::CpLevel => vm.set_cp_level(value),
            Param::CpDecay => vm.set_cp_decay(value),
            Param::HhLevel => vm.set_hh_level(value),
            Param::HhTune => vm.set_hh_tune(value),
            Param::HhMetal => vm.set_hh_metal(value),
            Param::ChDecay => vm.set_ch_decay(value),
            Param::OhDecay => vm.set_oh_decay(value),
            Param::DrumDrive => vm.set_drum_drive(value),
        }
    }

    /// Write a VOICE-LEVEL parameter into a snapshot (per-track patches).
    /// Returns false for bus-level parameters — effects, LFO, noise,
    /// volume, performance controllers — which are shared by nature and
    /// fall through to the global path.
    pub(crate) fn apply_to_params(self, p: &mut ParamValues, value: f32) -> bool {
        match self {
            Param::WaveformSel => p.waveform = waveform_from_value(value),
            Param::Detune => p.detune = value,
            Param::Cutoff => p.cutoff = value,
            Param::Resonance => p.resonance = value,
            Param::Drive => p.drive = value,
            Param::Saturation => p.saturation = value,
            Param::Attack => p.attack = value,
            Param::Decay => p.decay = value,
            Param::Sustain => p.sustain = value,
            Param::Release => p.release = value,
            Param::HpfCutoff => p.hpf_cutoff = value,
            Param::Glide => p.glide = value.clamp(0.0, 5.0),
            Param::SubLevel => p.sub = value,
            Param::Osc2Wave => p.osc2_wave = waveform_from_value(value),
            Param::Osc2Pitch => p.osc2_pitch = value,
            Param::Osc2Level => p.osc2_level = value,
            Param::Osc3Wave => p.osc3_wave = waveform_from_value(value),
            Param::Osc3Pitch => p.osc3_pitch = value,
            Param::Osc3Level => p.osc3_level = value,
            Param::CircuitSel => {
                p.circuit = if value.round() as i32 >= 1 {
                    crate::oscillator::CircuitModel::Arp
                } else {
                    crate::oscillator::CircuitModel::Moog
                }
            }
            Param::KeyTrack => p.key_track = value,
            Param::OscFm => p.osc_fm = value,
            Param::SyncSel => p.sync = value.round() as i32 >= 1,
            Param::RingAmount => p.ring = value,
            Param::PulseWidth => p.pulse_width = value.clamp(0.05, 0.95),
            Param::FilterEnvAmount => p.filter_env_amount = value,
            Param::FilterAttack => p.filter_attack = value,
            Param::FilterDecay => p.filter_decay = value,
            Param::FilterSustain => p.filter_sustain = value,
            Param::FilterRelease => p.filter_release = value,
            _ => return false,
        }
        true
    }
}

/// Parse patch-file text (`param value` lines) into a parameter snapshot.
/// Bus-level lines are ignored — a channel patch describes a voice, not
/// the shared effects.
pub fn params_from_patch(text: &str) -> Result<ParamValues, String> {
    let mut p = ParamValues::default();
    for (no, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let name = it.next().unwrap();
        let value: f32 = it
            .next()
            .ok_or_else(|| format!("patch line {}: '{}' has no value", no + 1, name))?
            .parse()
            .map_err(|_| format!("patch line {}: bad value for '{}'", no + 1, name))?;
        let param = Param::from_name(name)
            .ok_or_else(|| format!("patch line {}: unknown parameter '{}'", no + 1, name))?;
        param.apply_to_params(&mut p, value);
    }
    Ok(p)
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    Lin,
    Exp,
    Log,
    Smooth,
    Step,
}

impl Shape {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "lin" => Shape::Lin,
            "exp" => Shape::Exp,
            "log" => Shape::Log,
            "smooth" => Shape::Smooth,
            "step" => Shape::Step,
            _ => return None,
        })
    }

    fn interpolate(self, from: f32, to: f32, t: f32) -> f32 {
        let eased = match self {
            Shape::Lin => t,
            // Geometric interpolation for positive endpoints (perceptually even
            // for frequencies); fall back to an ease-in power curve otherwise
            Shape::Exp => {
                if from > 0.0 && to > 0.0 {
                    return from * (to / from).powf(t);
                }
                t * t
            }
            Shape::Log => 1.0 - (1.0 - t) * (1.0 - t),
            Shape::Smooth => t * t * (3.0 - 2.0 * t),
            Shape::Step => return if t >= 1.0 { to } else { from },
        };
        from + (to - from) * eased
    }
}

#[derive(Debug)]
pub enum EventKind {
    NoteOn { note: u8, velocity: f32, channel: u16 },
    NoteOff { note: u8, channel: u16 },
    Param { param: Param, value: f32, channel: u16 },
}

pub struct SongEvent {
    time: f64, // seconds from song start
    kind: EventKind,
}

/// A parsed song: the timed events plus each patch channel's parameter
/// snapshot (channel N+1 = channels[N]; channel 0 is the live panel).
pub struct Song {
    pub events: Vec<SongEvent>,
    pub channels: Vec<ParamValues>,
}

pub fn load_song(path: &str) -> Result<Song, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read song file '{}': {}", path, e))?;
    parse_song(&text)
}

fn dispatch(vm: &mut VoiceManager, kind: &EventKind) {
    match *kind {
        EventKind::NoteOn { note, velocity, channel } => {
            vm.note_on_channel(note, velocity, channel)
        }
        EventKind::NoteOff { note, channel } => vm.note_off_channel(note, channel),
        EventKind::Param { param, value, channel } => vm.set_channel_param(channel, param, value),
    }
}

fn register_channels(vm: &mut VoiceManager, song: &Song) {
    for (i, p) in song.channels.iter().enumerate() {
        vm.set_channel_params((i + 1) as u16, *p);
    }
}

/// Render a song offline, as fast as the CPU allows: same events, same
/// engine, no audio device. Returns interleaved-by-frame stereo samples,
/// with a few seconds of tail for reverb and tape print-through to ring out.
pub fn render_offline(song: &Song, sample_rate: f32) -> Vec<(f32, f32)> {
    let mut vm = VoiceManager::new(sample_rate, 10);
    // A bounce records a warmed-up instrument, not a cold power-on
    vm.warm_up();
    register_channels(&mut vm, song);
    let events = &song.events;
    let end = events.last().map(|e| e.time).unwrap_or(0.0) + 4.0;
    let total = (end * sample_rate as f64) as usize;
    let mut out = Vec::with_capacity(total);
    let mut next = 0;
    for n in 0..total {
        let t = n as f64 / sample_rate as f64;
        while next < events.len() && events[next].time <= t {
            dispatch(&mut vm, &events[next].kind);
            next += 1;
        }
        out.push(vm.render_next());
    }
    out
}

pub fn spawn_player(song: Song, voice_manager: Arc<Mutex<VoiceManager>>) {
    thread::spawn(move || {
        // Let the audio stream and window settle before the downbeat
        thread::sleep(Duration::from_millis(1200));
        println!("Song: playing {} events", song.events.len());
        register_channels(&mut voice_manager.lock(), &song);

        let start = Instant::now();
        for event in &song.events {
            let target = Duration::from_secs_f64(event.time);
            if let Some(wait) = target.checked_sub(start.elapsed()) {
                thread::sleep(wait);
            }
            dispatch(&mut voice_manager.lock(), &event.kind);
        }
        println!("Song: finished");
    });
}

enum TrackMode {
    None,
    Notes { vel: f32, len: f64, channel: u16 },
    Automation { param: Param, current: Option<f32>, channel: u16 },
}

// (beats, order-rank, kind); rank makes offs < params < ons at equal times
type RawEvent = (f64, u8, EventKind);

fn parse_song(text: &str) -> Result<Song, String> {
    let mut bpm = 120.0_f64;
    let mut gate = 0.9_f64;
    let mut events: Vec<RawEvent> = Vec::new();
    // Per-track patches: channel N+1 = channels[N]; channel 0 = the panel
    let mut channels: Vec<ParamValues> = Vec::new();
    let mut track_channels: Vec<(String, u16)> = Vec::new();

    let mut mode = TrackMode::None;
    let mut track_beat = 0.0_f64;

    for (line_no, raw) in text.lines().enumerate() {
        let err = |msg: String| format!("line {}: {}", line_no + 1, msg);
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        let first = line.split_whitespace().next().unwrap();
        match first {
            "bpm" => {
                bpm = line[3..].trim().parse::<f64>().map_err(|_| err("invalid bpm".into()))?;
                if bpm <= 0.0 {
                    return Err(err("bpm must be positive".into()));
                }
            }
            "gate" => {
                gate = line[4..].trim().parse::<f64>().map_err(|_| err("invalid gate".into()))?;
                gate = gate.clamp(0.05, 1.0);
            }
            "track" => {
                track_beat = 0.0;
                let name = line
                    .split_whitespace()
                    .nth(1)
                    .ok_or_else(|| err("track needs a name".into()))?
                    .to_string();
                let mut vel = 0.8_f32;
                let mut len = 1.0_f64;
                let mut channel = 0u16;
                for opt in line.split_whitespace().skip(2) {
                    if let Some(v) = opt.strip_prefix("vel=") {
                        vel = v.parse::<f32>().map_err(|_| err(format!("invalid vel '{}'", v)))?;
                    } else if let Some(v) = opt.strip_prefix("len=") {
                        len = v.parse::<f64>().map_err(|_| err(format!("invalid len '{}'", v)))?;
                    } else if opt.strip_prefix("kit=").is_some() {
                        // A drum track: notes route to the rhythm section
                        // (there is one board, so no per-track patches here)
                        channel = crate::drums::DRUM_CHANNEL;
                    } else if let Some(v) = opt.strip_prefix("patch=") {
                        // A private patch for this track: the file's
                        // voice-level parameters become this channel
                        let path = format!("patches/{}.patch", v);
                        let text = std::fs::read_to_string(&path)
                            .map_err(|e| err(format!("patch '{}': {}", path, e)))?;
                        let p = params_from_patch(&text).map_err(err)?;
                        channels.push(p);
                        channel = channels.len() as u16;
                    } else {
                        return Err(err(format!("unknown track option '{}'", opt)));
                    }
                }
                track_channels.push((name, channel));
                mode = TrackMode::Notes { vel, len, channel };
            }
            "automate" => {
                let name = line
                    .split_whitespace()
                    .nth(1)
                    .ok_or_else(|| err("automate needs a parameter name".into()))?;
                // `automate lead.cutoff` targets the named track's channel
                let (channel, pname) = match name.split_once('.') {
                    Some((track, pname)) => {
                        let ch = track_channels
                            .iter()
                            .find(|(t, _)| t == track)
                            .map(|(_, c)| *c)
                            .ok_or_else(|| {
                                err(format!(
                                    "automate '{}': no track named '{}' defined above",
                                    name, track
                                ))
                            })?;
                        (ch, pname)
                    }
                    None => (0u16, name),
                };
                let param = Param::from_name(pname)
                    .ok_or_else(|| err(format!("unknown parameter '{}'", pname)))?;
                track_beat = 0.0;
                mode = TrackMode::Automation { param, current: None, channel };
            }
            _ => match &mut mode {
                TrackMode::None => {
                    return Err(err("event tokens before any 'track' or 'automate' line".into()));
                }
                TrackMode::Notes { vel, len, channel } => {
                    let (vel, len, channel) = (*vel, *len, *channel);
                    let drums = channel == crate::drums::DRUM_CHANNEL;
                    for token in tokenize(line).map_err(err)? {
                        if token == "|" {
                            continue;
                        }
                        let (notes, dur, vel) = parse_note_token(&token, vel, len, drums)
                            .map_err(|m| err(format!("token '{}': {}", token, m)))?;
                        let off_beat = track_beat + dur * gate;
                        for &note in &notes {
                            events.push((
                                track_beat,
                                2,
                                EventKind::NoteOn { note, velocity: vel, channel },
                            ));
                            events.push((off_beat, 0, EventKind::NoteOff { note, channel }));
                        }
                        track_beat += dur;
                    }
                }
                TrackMode::Automation { param, current, channel } => {
                    let (param, channel) = (*param, *channel);
                    for token in tokenize(line).map_err(err)? {
                        if token == "|" {
                            continue;
                        }
                        let seg = parse_automation_token(&token)
                            .map_err(|m| err(format!("token '{}': {}", token, m)))?;
                        match seg {
                            AutoToken::Hold(dur) => track_beat += dur,
                            AutoToken::Set(value) => {
                                events.push((
                                    track_beat,
                                    1,
                                    EventKind::Param { param, value, channel },
                                ));
                                *current = Some(value);
                            }
                            AutoToken::Ramp { to, dur, shape } => {
                                let from = current.ok_or_else(|| {
                                    err(format!(
                                        "token '{}': first token of an automate track must be a plain starting value",
                                        token
                                    ))
                                })?;
                                emit_ramp(&mut events, param, channel, from, to, track_beat, dur, shape);
                                *current = Some(to);
                                track_beat += dur;
                            }
                        }
                    }
                }
            },
        }
    }

    if events.is_empty() {
        return Err("song contains no events".into());
    }

    events.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });

    let secs_per_beat = 60.0 / bpm;
    Ok(Song {
        events: events
            .into_iter()
            .map(|(beats, _, kind)| SongEvent { time: beats * secs_per_beat, kind })
            .collect(),
        channels,
    })
}

fn emit_ramp(
    events: &mut Vec<RawEvent>,
    param: Param,
    channel: u16,
    from: f32,
    to: f32,
    start_beat: f64,
    dur: f64,
    shape: Shape,
) {
    if matches!(shape, Shape::Step) || from == to {
        events.push((start_beat + dur, 1, EventKind::Param { param, value: to, channel }));
        return;
    }
    let steps = ((dur * AUTOMATION_STEPS_PER_BEAT).ceil() as usize).clamp(1, 4096);
    for k in 1..=steps {
        let t = k as f64 / steps as f64;
        let value = shape.interpolate(from, to, t as f32);
        events.push((start_beat + dur * t, 1, EventKind::Param { param, value, channel }));
    }
}

enum AutoToken {
    Set(f32),
    Hold(f64),
    Ramp { to: f32, dur: f64, shape: Shape },
}

/// Parse one automation token: `V`, `V:D`, `V:D@shape`, or `R:D` / `.:D`.
fn parse_automation_token(token: &str) -> Result<AutoToken, String> {
    let mut s = token;
    let mut shape = Shape::Lin;
    let mut dur: Option<f64> = None;

    if let Some(i) = s.rfind('@') {
        let name = &s[i + 1..];
        shape = Shape::from_name(name).ok_or_else(|| format!("unknown shape '{}'", name))?;
        s = &s[..i];
    }
    if let Some(i) = s.rfind(':') {
        let d = s[i + 1..].parse::<f64>().map_err(|_| "invalid duration".to_string())?;
        if d <= 0.0 {
            return Err("duration must be positive".into());
        }
        dur = Some(d);
        s = &s[..i];
    }

    if s == "." || s.eq_ignore_ascii_case("r") {
        return Ok(AutoToken::Hold(dur.ok_or("hold needs a duration, e.g. R:4")?));
    }

    let value = s.parse::<f32>().map_err(|_| "invalid value".to_string())?;
    match dur {
        Some(dur) => Ok(AutoToken::Ramp { to: value, dur, shape }),
        None => Ok(AutoToken::Set(value)),
    }
}

/// A `#` starts a comment only at line start or after whitespace, so sharp
/// note names like F#4 survive.
fn strip_comment(raw: &str) -> &str {
    let bytes = raw.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return &raw[..i];
        }
    }
    raw
}

/// Split a line into tokens, keeping bracketed chords together.
fn tokenize(line: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;

    for c in line.chars() {
        match c {
            '[' => {
                depth += 1;
                current.push(c);
            }
            ']' => {
                depth -= 1;
                if depth < 0 {
                    return Err("unmatched ']'".into());
                }
                current.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if depth != 0 {
        return Err("unmatched '['".into());
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

/// Parse one note-track token into (notes, duration-in-beats, velocity).
/// An empty notes list is a rest. On drum tracks the instrument names
/// (BD SD RS CP CH OH) are valid notes too.
fn parse_note_token(
    token: &str,
    default_vel: f32,
    default_len: f64,
    drums: bool,
) -> Result<(Vec<u8>, f64, f32), String> {
    let mut s = token;
    let mut vel = default_vel;
    let mut dur = default_len;

    if let Some(i) = s.rfind('@') {
        vel = s[i + 1..].parse::<f32>().map_err(|_| "invalid velocity".to_string())?;
        s = &s[..i];
    }
    if let Some(i) = s.rfind(':') {
        dur = s[i + 1..].parse::<f64>().map_err(|_| "invalid duration".to_string())?;
        s = &s[..i];
    }
    if dur <= 0.0 {
        return Err("duration must be positive".into());
    }

    let one = |s: &str| -> Result<u8, String> {
        if drums {
            if let Some(n) = crate::drums::note_from_name(s) {
                return Ok(n);
            }
        }
        parse_note(s)
    };
    let notes = if s == "." || s.eq_ignore_ascii_case("r") {
        Vec::new()
    } else if let Some(inner) = s.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        inner
            .split_whitespace()
            .map(one)
            .collect::<Result<Vec<u8>, String>>()?
    } else {
        vec![one(s)?]
    };

    Ok((notes, dur, vel.clamp(0.0, 1.0)))
}

/// Parse a note name like C4, F#3, Eb5 (C4 = MIDI 60), or a raw MIDI number.
fn parse_note(s: &str) -> Result<u8, String> {
    if s.chars().all(|c| c.is_ascii_digit()) {
        let n = s.parse::<u8>().map_err(|_| format!("invalid MIDI number '{}'", s))?;
        if n > 127 {
            return Err(format!("MIDI number {} out of range", n));
        }
        return Ok(n);
    }

    let mut chars = s.chars();
    let letter = chars.next().ok_or("empty note")?;
    let mut semitone: i32 = match letter.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        other => return Err(format!("invalid note letter '{}'", other)),
    };

    let rest: String = chars.collect();
    let mut rest = rest.as_str();
    while let Some(r) = rest.strip_prefix('#') {
        semitone += 1;
        rest = r;
    }
    while let Some(r) = rest.strip_prefix('b') {
        semitone -= 1;
        rest = r;
    }

    let octave = rest
        .parse::<i32>()
        .map_err(|_| format!("invalid octave '{}'", rest))?;
    let midi = (octave + 1) * 12 + semitone;
    if !(0..=127).contains(&midi) {
        return Err(format!("note '{}' out of MIDI range", s));
    }
    Ok(midi as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names() {
        assert_eq!(parse_note("C4").unwrap(), 60);
        assert_eq!(parse_note("A4").unwrap(), 69);
        assert_eq!(parse_note("F#3").unwrap(), 54);
        assert_eq!(parse_note("Eb5").unwrap(), 75);
        assert_eq!(parse_note("60").unwrap(), 60);
        assert!(parse_note("H4").is_err());
        assert!(parse_note("C99").is_err());
    }

    #[test]
    fn note_tokens() {
        let (notes, dur, vel) = parse_note_token("C4:2@0.7", 0.8, 1.0, false).unwrap();
        assert_eq!(notes, vec![60]);
        assert_eq!(dur, 2.0);
        assert_eq!(vel, 0.7);

        let (notes, dur, _) = parse_note_token("[C4 E4 G4]:0.5", 0.8, 1.0, false).unwrap();
        assert_eq!(notes, vec![60, 64, 67]);
        assert_eq!(dur, 0.5);

        let (notes, dur, _) = parse_note_token("R:4", 0.8, 1.0, false).unwrap();
        assert!(notes.is_empty());
        assert_eq!(dur, 4.0);

        // default duration comes from the track's len option
        let (_, dur, _) = parse_note_token("C4", 0.8, 0.5, false).unwrap();
        assert_eq!(dur, 0.5);
    }

    #[test]
    fn full_song() {
        let events = parse_song("bpm 120\ntrack a vel=0.9\nC4 E4:1 | R:2 [C3 G3]:2\n").unwrap().events;
        // 4 sounding notes -> 8 events (on + off each)
        assert_eq!(events.len(), 8);
        assert_eq!(events[0].time, 0.0);
        assert!(matches!(events[0].kind, EventKind::NoteOn { note: 60, .. }));
        // chord starts after 1 + 1 + 2 beats = 2.0 s at 120 bpm
        let chord_on = events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::NoteOn { .. }) && e.time == 2.0)
            .count();
        assert_eq!(chord_on, 2);
    }

    #[test]
    fn automation() {
        let events = parse_song("bpm 60\ntrack a\nC4:8\nautomate cutoff\n400 R:2 8000:4@exp\n")
            .unwrap()
            .events;
        let params: Vec<(f64, f32)> = events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::Param { param: Param::Cutoff, value, .. } => Some((e.time, value)),
                _ => None,
            })
            .collect();
        // initial set at t=0, then 4 beats * 32 steps of ramp
        assert_eq!(params.len(), 1 + 128);
        assert_eq!(params[0], (0.0, 400.0));
        // ramp starts after the 2-beat hold (t=2s at 60 bpm) and ends at t=6s
        assert!(params[1].0 > 2.0);
        let last = params.last().unwrap();
        assert_eq!(last.0, 6.0);
        assert!((last.1 - 8000.0).abs() < 0.5);
        // geometric ramp is monotonically increasing
        assert!(params.windows(2).all(|w| w[1].1 > w[0].1));
    }

    #[test]
    fn automation_errors() {
        // ramp before a starting value
        assert!(parse_song("automate cutoff\n8000:4@exp\n").is_err());
        // unknown parameter and unknown shape
        assert!(parse_song("automate flanger\n1 2:1\n").is_err());
        assert!(parse_song("automate cutoff\n400 800:4@bounce\n").is_err());
    }

    #[test]
    fn bundled_songs_parse() {
        for text in [
            include_str!("../songs/ferris-wheel.song"),
            include_str!("../songs/grid-runner.song"),
            include_str!("../songs/tide-engine.song"),
            include_str!("../songs/polaris.song"),
            include_str!("../songs/pressure-lines.song"),
        ] {
            let events = parse_song(text).unwrap().events;
            assert!(!events.is_empty());
            for pair in events.windows(2) {
                assert!(pair[0].time <= pair[1].time);
            }
        }
    }

    #[test]
    fn sharps_survive_comment_stripping() {
        // F#4 must not be truncated as a comment; trailing comments after
        // whitespace still work
        let events =
            parse_song("bpm 120\ntrack a\nF#4 [G2 F#3]:2 # a comment\n").unwrap().events;
        let ons = events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::NoteOn { .. }))
            .count();
        assert_eq!(ons, 3);
    }

    /// Per-track patches: `patch=` gives the track its own channel with a
    /// parameter snapshot, and `automate <track>.<param>` targets it.
    #[test]
    fn per_track_patches_and_dotted_automation() {
        let song = parse_song(
            "bpm 120\n\
             track lead vel=0.9 patch=init\n\
             C5:2\n\
             track pad\n\
             [C3 G3]:4\n\
             automate lead.cutoff\n\
             400 4000:2@exp\n\
             automate cutoff\n\
             2000\n",
        )
        .unwrap();
        assert_eq!(song.channels.len(), 1, "one patch track -> one channel");
        // lead notes carry channel 1, pad notes channel 0
        let lead_on = song.events.iter().any(|e| {
            matches!(e.kind, EventKind::NoteOn { note: 72, channel: 1, .. })
        });
        let pad_on = song.events.iter().any(|e| {
            matches!(e.kind, EventKind::NoteOn { note: 48, channel: 0, .. })
        });
        assert!(lead_on && pad_on);
        // dotted automation tagged to channel 1, plain to channel 0
        let tagged = song.events.iter().any(|e| {
            matches!(e.kind, EventKind::Param { param: Param::Cutoff, channel: 1, .. })
        });
        let global = song.events.iter().any(|e| {
            matches!(e.kind, EventKind::Param { param: Param::Cutoff, channel: 0, .. })
        });
        assert!(tagged && global);
        // unknown track name in dotted automation is an error
        assert!(parse_song("track a\nC4\nautomate ghost.cutoff\n400\n").is_err());
    }

    #[test]
    fn parse_errors() {
        assert!(parse_song("track a\nnot_a_note\n").is_err());
        assert!(parse_song("C4\n").is_err()); // notes before any track
        assert!(parse_song("bpm 100\n").is_err()); // no events at all
    }

    /// Drum tracks: `kit=` routes the track to the rhythm section, drum
    /// names parse to GM notes, and the whole path — DSL to board to bus —
    /// produces audio through the ordinary offline render.
    #[test]
    fn drum_tracks_trigger_the_rhythm_section() {
        let song = parse_song(
            "bpm 120\n\
             track beat kit=909 len=0.5\n\
             BD CH SD [BD OH]@1 | RS CP BD:1\n",
        )
        .unwrap();
        // Every note event carries the drum channel
        let all_drum = song.events.iter().all(|e| match e.kind {
            EventKind::NoteOn { channel, .. } | EventKind::NoteOff { channel, .. } => {
                channel == crate::drums::DRUM_CHANNEL
            }
            _ => true,
        });
        assert!(all_drum, "kit= tracks must route every note to the board");
        // Drum names outside a kit= track stay errors
        assert!(parse_song("bpm 120\ntrack a\nBD\n").is_err());

        let frames = render_offline(&song, 48000.0);
        let peak = frames
            .iter()
            .fold(0.0f32, |a, &(l, r)| a.max(l.abs()).max(r.abs()));
        assert!(peak > 0.05, "the beat should be audible, peak={peak}");
    }

    /// Drum knobs are ordinary parameters: patch lines, automation, and
    /// the live panel all reach the same board.
    #[test]
    fn drum_params_automate_globally() {
        let song = parse_song(
            "bpm 120\n\
             track beat kit=909\n\
             BD:8\n\
             automate bd_drive\n\
             0.1 0.9:4@lin\n",
        )
        .unwrap();
        let mut vm = VoiceManager::new(48000.0, 4);
        for e in &song.events {
            match e.kind {
                EventKind::NoteOn { note, velocity, channel } => {
                    vm.note_on_channel(note, velocity, channel)
                }
                EventKind::NoteOff { note, channel } => vm.note_off_channel(note, channel),
                EventKind::Param { param, value, channel } => {
                    vm.set_channel_param(channel, param, value)
                }
            }
        }
        assert!(
            (vm.params.bd_drive - 0.9).abs() < 1e-3,
            "bd_drive automation should land on the shared panel, got {}",
            vm.params.bd_drive
        );
    }
}
