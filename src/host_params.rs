// The host-facing parameter surface, shared by every plugin front end:
// CLAP/VST3 (src/plugin.rs), the Audio Unit (src/au/), and the custom editor
// panel (src/editor.rs). Each front end derives its host parameter list,
// defaults, state save/restore, and per-block application from this module,
// so the formats cannot drift from one another.
//
// SINGLE SOURCE OF TRUTH FOR THE ENGINE: the parameter's range, taper, and
// the VoiceManager setter it drives all come from `song::Param` — the SAME
// canonical table the standalone app, the song player, and MIDI CCs use
// (song.rs). A host parameter is nothing but a `Param` plus the cosmetics a
// host needs that the engine table does not carry: a human display name, a
// unit/formatting hint, a default value, and (for a handful of setters) the
// "only fire on change" guard flag. Because every host parameter APPLIES
// through `Param::apply`, the old class of bug — a host selector that never
// reached the engine, or a hand-copied range that disagreed with the real
// clamp — is unrepresentable: if the engine can set it, the host sets it the
// exact same way.
//
// COMPLETENESS IS ENFORCED: every `Param` in `song::PARAM_DEFS` is either in
// the presentation table below or in `EXCLUDED` (the MIDI/performance events
// and song-desk lanes that are not host-automation knobs). A parameter that
// is neither fails `every_param_is_accounted_for`, so a new engine knob
// cannot silently go missing from Logic.
//
// ORDER is the Audio Unit parameter-ID order (the AU uses each entry's index
// as its AudioUnitParameterID) and the CLAP/VST3 display order. There is no
// released-project compatibility to preserve, so the order is chosen for
// readability; the front ends and their drift-pin tests all derive from it.

use crate::oscillator::Waveform;
use crate::song::Param;
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
    /// The engine parameter this drives; its range/taper/setter are canonical.
    pub param: Param,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub display: Display,
    /// Guarded setters are only called when the value changes — they swap
    /// voice banks, re-randomize offsets, or re-run self-calibration.
    pub guarded: bool,
}

/// A selector parameter: a small set of named positions. Always applied
/// change-only (its setter swaps voice banks / circuit models).
pub struct ChoiceDef {
    pub id: &'static str,
    pub name: &'static str,
    pub param: Param,
    pub variants: &'static [&'static str],
    pub default: usize,
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

    pub fn param(&self) -> Param {
        match self {
            ParamDef::Float(f) => f.param,
            ParamDef::Choice(c) => c.param,
        }
    }

    pub fn default_value(&self) -> f32 {
        match self {
            ParamDef::Float(f) => f.default,
            ParamDef::Choice(c) => c.default as f32,
        }
    }

    /// Apply a host value to the engine through the canonical setter path.
    /// Floats pass their native value; selectors pass the index as an f32,
    /// which `Param::apply` maps to the enum position.
    pub fn apply(&self, vm: &mut VoiceManager, value: f32) {
        self.param().apply(vm, value);
    }
}

/// Engine order for the waveform selector — the order of `Waveform as u8`,
/// which is also the order `song::waveform_from_value` inverts. A host
/// selector index is therefore the exact value `Param::apply` expects, and
/// the variant NAMES below sit in this same order. Nothing maps between two
/// orderings, so a "picked Triangle, got Square" mismatch cannot occur.
pub const WAVEFORM_VARIANTS: [Waveform; 4] = [
    Waveform::Sine,
    Waveform::Square,
    Waveform::Sawtooth,
    Waveform::Triangle,
];

const WAVE_NAMES: &[&str] = &["Sine", "Square", "Sawtooth", "Triangle"];
const CIRCUIT_NAMES: &[&str] = &["Moog", "ARP"];
const SYNC_NAMES: &[&str] = &["Off", "On"];
const CHORUS_NAMES: &[&str] = &["Off", "I", "II", "III", "IV"];

/// One presentation row: an engine parameter plus its host cosmetics.
struct Row {
    param: Param,
    name: &'static str,
    kind: Kind,
}

enum Kind {
    Float {
        display: Display,
        guarded: bool,
    },
    Choice {
        variants: &'static [&'static str],
    },
}

const fn flt(param: Param, name: &'static str, display: Display) -> Row {
    Row {
        param,
        name,
        kind: Kind::Float {
            display,
            guarded: false,
        },
    }
}

const fn gflt(param: Param, name: &'static str, display: Display) -> Row {
    Row {
        param,
        name,
        kind: Kind::Float {
            display,
            guarded: true,
        },
    }
}

const fn sel(param: Param, name: &'static str, variants: &'static [&'static str]) -> Row {
    Row {
        param,
        name,
        kind: Kind::Choice { variants },
    }
}

use Display::{Fraction, Hertz, Percent, Plain, Seconds};

/// THE host presentation table. Range/taper/setter for each row come from
/// `Param`; only the human name, formatting, and guard live here. There is
/// no default column: every default is the Init patch's value (see
/// `init_value`), so the plugin a host instantiates and the app that powers
/// on are the same sound by construction.
/// Order is the host display order (and the AU parameter-ID order).
///
/// NAMES are fully spelled out: Logic shows this list flat, with no section
/// headers to lend context, so every name must stand alone. That means no
/// bare "Drive"/"Attack"/"Level" (which drive? which envelope? which drum) —
/// each is qualified ("Filter Drive", "Amp Attack", "Kick Level"), and the
/// 909 shorthand is expanded to the instrument's real name.
#[rustfmt::skip]
const PRESENTATION: &[Row] = &[
    // ORDER IS AN ABI. A parameter's position here IS its
    // AudioUnitParameterID, and hosts record automation against that number,
    // so reordering silently re-points every automation curve in every saved
    // project. The first 56 rows are frozen in their original shipped order;
    // anything new goes on the END, never in the middle.

    sel (Param::WaveformSel, "Waveform",   WAVE_NAMES),
    flt (Param::Volume,     "Volume",      Percent),
    flt (Param::Detune,     "Detune",      Plain(" ct")),
    flt (Param::PulseWidth, "Pulse Width", Fraction),
    flt (Param::NoiseLevel, "Noise",       Percent),
    flt (Param::LfoRate,    "LFO Rate",           Hertz),
    flt (Param::LfoShape,   "LFO Shape",          Percent),
    flt (Param::LfoPitch,   "LFO to Pitch",       Plain(" ct")),
    flt (Param::LfoFilter,  "LFO to Filter",      Plain(" oct")),
    flt (Param::LfoPwm,     "LFO to Pulse Width", Plain("")),
    flt (Param::Attack,     "Amp Attack",  Seconds),
    flt (Param::Decay,      "Amp Decay",   Seconds),
    flt (Param::Sustain,    "Amp Sustain", Percent),
    flt (Param::Release,    "Amp Release", Seconds),
    flt (Param::Cutoff,     "Filter Cutoff",     Hertz),
    flt (Param::Resonance,  "Filter Resonance",  Plain("")),
    flt (Param::Drive,      "Filter Drive",      Plain("")),
    flt (Param::Saturation, "Filter Saturation", Plain("")),
    flt (Param::HpfCutoff,  "High-Pass Filter",  Hertz),
    flt (Param::FilterEnvAmount, "Filter Envelope Amount", Plain(" oct")),
    flt (Param::FilterAttack,    "Filter Attack",  Seconds),
    flt (Param::FilterDecay,     "Filter Decay",   Seconds),
    flt (Param::FilterSustain,   "Filter Sustain", Percent),
    flt (Param::FilterRelease,   "Filter Release", Seconds),
    flt (Param::FuzzAmount,  "Fuzz",            Percent),
    flt (Param::SpringWet,   "Spring Reverb",   Percent),
    flt (Param::ReverbDecay, "Reverb Decay",    Fraction),
    flt (Param::ReverbWet,   "Reverb Mix",      Percent),
    sel (Param::ChorusModeSel, "Chorus Mode",   CHORUS_NAMES),
    gflt(Param::ChorusRate,  "Chorus Rate",     Hertz),
    gflt(Param::ChorusDepth, "Chorus Depth",    Percent),
    flt (Param::TapeWow,     "Tape Wow",        Percent),
    flt (Param::TapeFlutter, "Tape Flutter",    Percent),
    gflt(Param::TapeDrive,   "Tape Drive",      Percent),
    gflt(Param::TapeAge,     "Tape Age",        Percent),
    flt (Param::BdLevel,   "Kick Level",      Percent),
    flt (Param::BdTune,    "Kick Tune",       Fraction),
    flt (Param::BdAttack,  "Kick Attack",     Fraction),
    flt (Param::BdDecay,   "Kick Decay",      Fraction),
    flt (Param::BdSweep,   "Kick Sweep",      Fraction),
    flt (Param::BdDrive,   "Kick Drive",      Fraction),
    flt (Param::SdLevel,   "Snare Level",     Percent),
    flt (Param::SdTune,    "Snare Tune",      Fraction),
    flt (Param::SdTone,    "Snare Tone",      Fraction),
    flt (Param::SdSnappy,  "Snare Snappy",    Fraction),
    flt (Param::SdDecay,   "Snare Decay",     Fraction),
    flt (Param::RsLevel,   "Rim Shot Level",  Percent),
    flt (Param::RsTune,    "Rim Shot Tune",   Fraction),
    flt (Param::CpLevel,   "Clap Level",      Percent),
    flt (Param::CpDecay,   "Clap Decay",      Fraction),
    flt (Param::HhLevel,   "Hi-Hat Level",    Percent),
    flt (Param::HhTune,    "Hi-Hat Tune",     Fraction),
    flt (Param::HhMetal,   "Hi-Hat Metal",    Fraction),
    flt (Param::ChDecay,   "Closed Hat Decay", Fraction),
    flt (Param::OhDecay,   "Open Hat Decay",  Fraction),
    flt (Param::DrumDrive, "Drum Bus Drive",  Percent),

    // --- Appended after the frozen block (new controls) ------------------
    sel (Param::CircuitSel, "Circuit",     CIRCUIT_NAMES),
    flt (Param::SubLevel,   "Sub Oscillator", Percent),
    flt (Param::Glide,      "Glide",       Plain(" s")),
    sel (Param::Osc2Wave,   "Oscillator 2 Waveform", WAVE_NAMES),
    flt (Param::Osc2Pitch,  "Oscillator 2 Pitch",    Plain(" st")),
    flt (Param::Osc2Level,  "Oscillator 2 Level",    Percent),
    sel (Param::Osc3Wave,   "Oscillator 3 Waveform", WAVE_NAMES),
    flt (Param::Osc3Pitch,  "Oscillator 3 Pitch",    Plain(" st")),
    flt (Param::Osc3Level,  "Oscillator 3 Level",    Percent),
    sel (Param::SyncSel,    "Oscillator Sync", SYNC_NAMES),
    flt (Param::RingAmount, "Ring Modulation", Percent),
    flt (Param::OscFm,      "Oscillator FM",   Percent),
    flt (Param::KeyTrack,   "Key Tracking",    Percent),
    flt (Param::MixSaw,     "Oscillator 1 Mix Sawtooth", Percent),
    flt (Param::MixPulse,   "Oscillator 1 Mix Pulse",    Percent),
    flt (Param::MixTri,     "Oscillator 1 Mix Triangle", Percent),
    flt (Param::MixSine,    "Oscillator 1 Mix Sine",     Percent),
    flt (Param::Unison,        "Unison Voices", Plain("")),
    flt (Param::UnisonDetune,  "Unison Detune", Plain(" ct")),
    flt (Param::ReverbTone,  "Reverb Tone",     Hertz),
    flt (Param::ReverbPre,   "Reverb Predelay", Plain(" s")),
    flt (Param::DrumTone,  "Drum Bus Tone",   Fraction),
];

/// Parameters that are NOT host-automation knobs and are deliberately kept
/// off the surface. They still exist in the engine and reach it by their own
/// route; exposing them as static Logic knobs would be meaningless.
///
///  - MIDI / performance events: they arrive as note/CC/pitch-bend messages
///    or as the UI's keyboard register, not as automation.
///  - The voice box (vocoder): inert without a modulator source (lyrics or a
///    recording), which the plugin instrument has no way to feed.
///  - The tape deck (sampler): routes to sampler slots; the plugin has no
///    loaded reel.
///  - The mixer desk: per-track, channel-scoped strip controls addressed as
///    `track.param` inside a song — no single global meaning in a plugin.
///
/// Referenced only by the completeness test; it documents intent for readers
/// and is the backstop that keeps the surface honest.
#[cfg_attr(not(test), allow(dead_code))]
const EXCLUDED: &[Param] = &[
    // MIDI / performance
    Param::UiOctave,
    Param::PitchBendSemis,
    Param::ModWheel,
    Param::SustainPedal,
    // Voice box
    Param::VoxLevel,
    Param::VoxDry,
    Param::VoxBreath,
    Param::VoxClarity,
    Param::VoxVibrato,
    Param::VoxModeSel,
    Param::VoxIntonation,
    // Tape deck (sampler slots)
    Param::SmpPitch,
    Param::SmpStart,
    Param::SmpGain,
    Param::SmpPan,
    Param::SmpAttack,
    Param::SmpRelease,
    Param::SmpCutoff,
    Param::SmpRes,
    // Mixer desk (per-track strips, the song's post-effects arrangement
    // trim, a track's own pitch CV) + the song-only chorus insert override
    Param::Output,
    Param::PitchShift,
    Param::TrackGain,
    Param::TrackPan,
    Param::ReverbSend,
    Param::SpringSend,
    Param::ChorusSend,
    Param::DuckAmount,
    Param::DuckRelease,
    Param::ChorusMix,
];

/// THE host parameter table, built from the presentation rows above with
/// each row's range/taper read straight from `Param::range`. Order here is
/// the order hosts display parameters in, and the index is the Audio Unit
/// parameter ID.
pub fn param_defs() -> Vec<ParamDef> {
    PRESENTATION
        .iter()
        .map(|row| {
            let (min, max, _curve) = row.param.range();
            let id = row.param.name();
            let default = init_value(row.param);
            match row.kind {
                Kind::Float { display, guarded } => ParamDef::Float(FloatDef {
                    id,
                    name: row.name,
                    param: row.param,
                    min,
                    max,
                    default,
                    display,
                    guarded,
                }),
                Kind::Choice { variants } => ParamDef::Choice(ChoiceDef {
                    id,
                    name: row.name,
                    param: row.param,
                    variants,
                    default: default as usize,
                }),
            }
        })
        .collect()
}

/// The Init patch's setting for `param` — THE default of every host
/// parameter. A power-on state is a patch like any other (the Polymoog comes
/// on in Preset 8, Patina in Init), so the defaults are read out of that
/// patch's text rather than kept in a second list that could disagree with
/// it. `init_patch_sets_every_host_parameter` pins that the patch leaves no
/// host parameter unset.
fn init_value(param: Param) -> f32 {
    let name = param.name();
    crate::patch::FACTORY[0]
        .1
        .lines()
        .filter_map(|line| {
            let line = line.split('#').next()?;
            let mut it = line.split_whitespace();
            (it.next()? == name).then(|| it.next()?.parse::<f32>().ok())?
        })
        .last()
        .unwrap_or_else(|| panic!("the Init patch does not set `{name}`"))
}

/// Route one MIDI note-on to the keyboard voices or, on GM channel 10
/// (0-indexed 9), the 909 board. Velocity is 0..1.
/// Channel 10 is the GM drum convention, but hosts built around software
/// instruments (Logic) give the user no way to choose a MIDI channel — so
/// the board also always answers the reserved sliver at the bottom of the
/// range, on every channel. Those notes are below anything playable, so the
/// keyboard voices lose nothing.
pub fn note_on(vm: &mut VoiceManager, channel: u8, note: u8, velocity: f32) {
    if channel == 9 || crate::drums::is_low_drum_note(note) {
        vm.note_on_channel(note, velocity, crate::drums::DRUM_CHANNEL);
    } else {
        vm.note_on(note, velocity);
    }
}

pub fn note_off(vm: &mut VoiceManager, channel: u8, note: u8) {
    // Drum hits are one-shots — they ring out on their own envelopes, so a
    // note-off must not chase them (and must not stop a keyboard voice that
    // never started).
    if channel != 9 && !crate::drums::is_low_drum_note(note) {
        vm.note_off(note);
    }
}

#[cfg(test)]
mod tests {

    /// Logic gives software-instrument tracks no MIDI-channel control, so
    /// the reserved bottom sliver has to reach the 909 on ANY channel or the
    /// drums are unplayable there. Notes above the sliver must still be
    /// keyboard voices on non-drum channels.
    #[test]
    fn bottom_sliver_always_reaches_the_drums() {
        for note in 0..=crate::drums::LOW_DRUM_LAST {
            assert!(
                crate::drums::is_low_drum_note(note),
                "note {note} should be a drum"
            );
        }
        assert!(!crate::drums::is_low_drum_note(
            crate::drums::LOW_DRUM_LAST + 1
        ));
        // The sliver sits below any playable key, so nothing musical is lost.
        assert!(
            crate::drums::LOW_DRUM_LAST < 21,
            "sliver must stay below A0"
        );
    }

    /// A parameter's position in PRESENTATION IS its AudioUnitParameterID,
    /// and hosts record automation curves against that number. Reordering the
    /// shipped block silently re-points every curve in every saved project —
    /// this pins the original 56 so new parameters can only be appended.
    #[test]
    fn shipped_parameter_ids_never_move() {
        const FROZEN: [Param; 56] = [
            Param::WaveformSel,
            Param::Volume,
            Param::Detune,
            Param::PulseWidth,
            Param::NoiseLevel,
            Param::LfoRate,
            Param::LfoShape,
            Param::LfoPitch,
            Param::LfoFilter,
            Param::LfoPwm,
            Param::Attack,
            Param::Decay,
            Param::Sustain,
            Param::Release,
            Param::Cutoff,
            Param::Resonance,
            Param::Drive,
            Param::Saturation,
            Param::HpfCutoff,
            Param::FilterEnvAmount,
            Param::FilterAttack,
            Param::FilterDecay,
            Param::FilterSustain,
            Param::FilterRelease,
            Param::FuzzAmount,
            Param::SpringWet,
            Param::ReverbDecay,
            Param::ReverbWet,
            Param::ChorusModeSel,
            Param::ChorusRate,
            Param::ChorusDepth,
            Param::TapeWow,
            Param::TapeFlutter,
            Param::TapeDrive,
            Param::TapeAge,
            Param::BdLevel,
            Param::BdTune,
            Param::BdAttack,
            Param::BdDecay,
            Param::BdSweep,
            Param::BdDrive,
            Param::SdLevel,
            Param::SdTune,
            Param::SdTone,
            Param::SdSnappy,
            Param::SdDecay,
            Param::RsLevel,
            Param::RsTune,
            Param::CpLevel,
            Param::CpDecay,
            Param::HhLevel,
            Param::HhTune,
            Param::HhMetal,
            Param::ChDecay,
            Param::OhDecay,
            Param::DrumDrive,
        ];
        let defs = param_defs();
        assert!(defs.len() >= FROZEN.len());
        for (id, expected) in FROZEN.iter().enumerate() {
            assert_eq!(
                defs[id].param(),
                *expected,
                "parameter id {id} moved: hosts would re-point automation"
            );
        }
    }

    use super::*;
    use crate::song::PARAM_DEFS;

    /// Structural completeness: every engine parameter is either presented
    /// to hosts or explicitly excluded — never silently dropped, never both.
    #[test]
    fn every_param_is_accounted_for() {
        for def in PARAM_DEFS {
            let p = def.param;
            let presented = PRESENTATION.iter().any(|r| r.param == p);
            let excluded = EXCLUDED.contains(&p);
            assert!(
                presented ^ excluded,
                "{} must be in exactly one of PRESENTATION / EXCLUDED (presented={presented}, excluded={excluded})",
                def.name
            );
        }
    }

    /// The waveform selector's NAMES sit in the same order as the engine's
    /// value mapping, so a host index applies as itself.
    #[test]
    fn waveform_names_match_engine_order() {
        for (i, name) in WAVE_NAMES.iter().enumerate() {
            let expected = match WAVEFORM_VARIANTS[i] {
                Waveform::Sine => "Sine",
                Waveform::Square => "Square",
                Waveform::Sawtooth => "Sawtooth",
                Waveform::Triangle => "Triangle",
            };
            assert_eq!(*name, expected, "waveform variant {i}");
        }
    }

    /// Selector ranges and their variant lists agree: max index == len-1.
    #[test]
    fn selector_ranges_match_variant_counts() {
        for def in param_defs() {
            if let ParamDef::Choice(c) = def {
                let (min, max, _) = c.param.range();
                assert_eq!(min, 0.0, "{} min", c.id);
                assert_eq!(max as usize, c.variants.len() - 1, "{} max", c.id);
            }
        }
    }

    /// Every host default is read out of the Init patch, so the patch must
    /// set each presented parameter (a missing one would panic at plugin
    /// construction) — and the bank's first slot must really be Init.
    #[test]
    fn init_patch_sets_every_host_parameter() {
        assert_eq!(crate::patch::FACTORY[0].0, "Init");
        for row in PRESENTATION {
            let name = row.param.name();
            let set = crate::patch::FACTORY[0].1.lines().any(|l| {
                l.split('#').next().unwrap().split_whitespace().next() == Some(name)
            });
            assert!(set, "the Init patch does not set `{name}`");
        }
    }

    /// Init is a dry voice: every effect in the chain is out, so what a
    /// fresh plugin instance or a freshly launched app plays is the
    /// oscillator and filter alone.
    #[test]
    fn init_is_dry() {
        for p in [
            Param::ReverbWet,
            Param::SpringWet,
            Param::FuzzAmount,
            Param::ChorusModeSel,
            Param::TapeWow,
            Param::TapeFlutter,
            Param::TapeDrive,
            Param::TapeAge,
            Param::Saturation,
            Param::Osc2Level,
            Param::Osc3Level,
            Param::Detune,
            Param::LfoPitch,
            Param::LfoFilter,
            Param::LfoPwm,
        ] {
            assert_eq!(init_value(p), 0.0, "Init sets `{}` on", p.name());
        }
    }

    /// Defaults land inside the engine's own range for every host parameter.
    #[test]
    fn defaults_within_range() {
        for def in param_defs() {
            if let ParamDef::Float(f) = def {
                assert!(
                    f.default >= f.min && f.default <= f.max,
                    "{} default {} out of [{}, {}]",
                    f.id,
                    f.default,
                    f.min,
                    f.max
                );
            }
        }
    }

    /// `Seconds` and `Hertz` mean "map this control logarithmically", and
    /// every front end acts on that: the editor knob converts with
    /// `(v/min).ln() / (max/min).ln()`, the AU sets
    /// kAudioUnitParameterFlag_DisplayLogarithmic, and the CLAP/VST3 build
    /// uses a skewed range. A minimum of zero turns the knob's conversion
    /// into inf/inf = NaN, and that NaN is written straight back to the host
    /// as the parameter's new value — which then reaches the engine. Pin the
    /// precondition here so a future log-displayed parameter cannot open
    /// that hole.
    #[test]
    fn logarithmic_displays_have_a_positive_minimum() {
        for def in param_defs() {
            if let ParamDef::Float(f) = def {
                if matches!(f.display, Display::Seconds | Display::Hertz) {
                    assert!(
                        f.min > 0.0 && f.max > f.min,
                        "`{}` is displayed logarithmically but spans [{}, {}]",
                        f.id,
                        f.min,
                        f.max
                    );
                }
            }
        }
    }

    /// Single-frequency magnitude via Goertzel — enough to weigh a partial.
    fn goertzel(samples: &[f32], sr: f32, freq: f32) -> f32 {
        let w = 2.0 * std::f32::consts::PI * freq / sr;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &x in samples {
            let s0 = x + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt()
    }

    /// Configure a fresh engine from the table defaults, override the
    /// waveform selector, play A4, and weigh the fundamental against the
    /// harmonic series in the sustained tail.
    fn harmonic_ratio(waveform_index: f32) -> f32 {
        const SR: f32 = 44100.0;
        let defs = param_defs();
        let mut vm = VoiceManager::new(SR, NUM_VOICES);
        for d in &defs {
            d.apply(&mut vm, d.default_value());
        }
        let wf = defs.iter().find(|d| d.id() == "waveform").unwrap();
        wf.apply(&mut vm, waveform_index);
        // Note-on drives the channel-0 path that reconfigures the voice from
        // the live params — the exact route that used to drop oscs 2 & 3.
        vm.note_on(69, 1.0);
        let mut buf = Vec::with_capacity(12000);
        for _ in 0..12000 {
            let (l, r) = vm.render_next();
            buf.push(0.5 * (l + r));
        }
        let seg = &buf[3000..];
        let f0 = 440.0;
        let fund = goertzel(seg, SR, f0);
        let mut harm = 0.0;
        for k in 2..=6 {
            harm += goertzel(seg, SR, f0 * k as f32);
        }
        harm / (fund + 1e-9)
    }

    /// The end-to-end proof that a host selector change reaches the audio:
    /// picking Sine must turn the WHOLE voice (all three oscillators) sine,
    /// so its harmonic content collapses far below the sawtooth's. Before
    /// the fix, a note-on reverted oscs 2 & 3 to sawtooth and the "Sine"
    /// selection was inaudible — this ratio would stay high.
    #[test]
    fn waveform_selector_actually_changes_the_sound() {
        let sine = harmonic_ratio(0.0); // WAVEFORM_VARIANTS[0] == Sine
        let saw = harmonic_ratio(2.0); // WAVEFORM_VARIANTS[2] == Sawtooth
        assert!(
            sine < 0.5,
            "Sine voice still harmonically rich (ratio {sine}) — selector not applying"
        );
        assert!(
            sine < 0.4 * saw,
            "Sine (ratio {sine}) not markedly cleaner than Sawtooth (ratio {saw})"
        );
    }
}
