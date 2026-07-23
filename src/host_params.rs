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
pub const WAVEFORM_VARIANTS: [Waveform; 4] =
    [Waveform::Sine, Waveform::Square, Waveform::Sawtooth, Waveform::Triangle];

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
    Float { display: Display, default: f32, guarded: bool },
    Choice { variants: &'static [&'static str], default: usize },
}

const fn flt(param: Param, name: &'static str, display: Display, default: f32) -> Row {
    Row { param, name, kind: Kind::Float { display, default, guarded: false } }
}

const fn gflt(param: Param, name: &'static str, display: Display, default: f32) -> Row {
    Row { param, name, kind: Kind::Float { display, default, guarded: true } }
}

const fn sel(
    param: Param,
    name: &'static str,
    variants: &'static [&'static str],
    default: usize,
) -> Row {
    Row { param, name, kind: Kind::Choice { variants, default } }
}

use Display::{Fraction, Hertz, Percent, Plain, Seconds};

/// THE host presentation table. Range/taper/setter for each row come from
/// `Param`; only the human name, formatting, default, and guard live here.
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

    sel (Param::WaveformSel, "Waveform",   WAVE_NAMES,    2),
    flt (Param::Volume,     "Volume",      Percent,       0.5),
    flt (Param::Detune,     "Detune",      Plain(" ct"),  7.0),
    flt (Param::PulseWidth, "Pulse Width", Fraction,      0.5),
    flt (Param::NoiseLevel, "Noise",       Percent,       0.0),
    flt (Param::LfoRate,    "LFO Rate",           Hertz,        1.0),
    flt (Param::LfoShape,   "LFO Shape",          Percent,      0.5),
    flt (Param::LfoPitch,   "LFO to Pitch",       Plain(" ct"), 0.0),
    flt (Param::LfoFilter,  "LFO to Filter",      Plain(" oct"), 0.0),
    flt (Param::LfoPwm,     "LFO to Pulse Width", Plain(""),    0.0),
    flt (Param::Attack,     "Amp Attack",  Seconds, 0.1),
    flt (Param::Decay,      "Amp Decay",   Seconds, 0.1),
    flt (Param::Sustain,    "Amp Sustain", Percent, 0.7),
    flt (Param::Release,    "Amp Release", Seconds, 0.2),
    flt (Param::Cutoff,     "Filter Cutoff",     Hertz,     15000.0),
    flt (Param::Resonance,  "Filter Resonance",  Plain(""), 0.0),
    flt (Param::Drive,      "Filter Drive",      Plain(""), 1.0),
    flt (Param::Saturation, "Filter Saturation", Plain(""), 1.0),
    flt (Param::HpfCutoff,  "High-Pass Filter",  Hertz,     16.0),
    flt (Param::FilterEnvAmount, "Filter Envelope Amount", Plain(" oct"), 0.0),
    flt (Param::FilterAttack,    "Filter Attack",  Seconds, 0.005),
    flt (Param::FilterDecay,     "Filter Decay",   Seconds, 0.3),
    flt (Param::FilterSustain,   "Filter Sustain", Percent, 0.0),
    flt (Param::FilterRelease,   "Filter Release", Seconds, 0.3),
    flt (Param::FuzzAmount,  "Fuzz",            Percent,     0.0),
    flt (Param::SpringWet,   "Spring Reverb",   Percent,     0.0),
    flt (Param::ReverbDecay, "Reverb Decay",    Fraction,    0.5),
    flt (Param::ReverbWet,   "Reverb Mix",      Percent,     0.5),
    sel (Param::ChorusModeSel, "Chorus Mode",   CHORUS_NAMES, 0),
    gflt(Param::ChorusRate,  "Chorus Rate",     Hertz,       0.5),
    gflt(Param::ChorusDepth, "Chorus Depth",    Percent,     0.3),
    flt (Param::TapeWow,     "Tape Wow",        Percent,     0.0),
    flt (Param::TapeFlutter, "Tape Flutter",    Percent,     0.0),
    gflt(Param::TapeDrive,   "Tape Drive",      Percent,     0.0),
    gflt(Param::TapeAge,     "Tape Age",        Percent,     0.0),
    flt (Param::BdLevel,   "Kick Level",      Percent,  0.8),
    flt (Param::BdTune,    "Kick Tune",       Fraction, 0.35),
    flt (Param::BdAttack,  "Kick Attack",     Fraction, 0.5),
    flt (Param::BdDecay,   "Kick Decay",      Fraction, 0.45),
    flt (Param::BdSweep,   "Kick Sweep",      Fraction, 0.5),
    flt (Param::BdDrive,   "Kick Drive",      Fraction, 0.25),
    flt (Param::SdLevel,   "Snare Level",     Percent,  0.75),
    flt (Param::SdTune,    "Snare Tune",      Fraction, 0.4),
    flt (Param::SdTone,    "Snare Tone",      Fraction, 0.5),
    flt (Param::SdSnappy,  "Snare Snappy",    Fraction, 0.6),
    flt (Param::SdDecay,   "Snare Decay",     Fraction, 0.5),
    flt (Param::RsLevel,   "Rim Shot Level",  Percent,  0.7),
    flt (Param::RsTune,    "Rim Shot Tune",   Fraction, 0.5),
    flt (Param::CpLevel,   "Clap Level",      Percent,  0.75),
    flt (Param::CpDecay,   "Clap Decay",      Fraction, 0.5),
    flt (Param::HhLevel,   "Hi-Hat Level",    Percent,  0.7),
    flt (Param::HhTune,    "Hi-Hat Tune",     Fraction, 0.5),
    flt (Param::HhMetal,   "Hi-Hat Metal",    Fraction, 0.65),
    flt (Param::ChDecay,   "Closed Hat Decay", Fraction, 0.35),
    flt (Param::OhDecay,   "Open Hat Decay",  Fraction, 0.5),
    flt (Param::DrumDrive, "Drum Bus Drive",  Percent,  0.0),

    // --- Appended after the frozen block (new controls) ------------------
    sel (Param::CircuitSel, "Circuit",     CIRCUIT_NAMES, 0),
    flt (Param::SubLevel,   "Sub Oscillator", Percent,    0.0),
    flt (Param::Glide,      "Glide",       Plain(" s"),   0.0),
    sel (Param::Osc2Wave,   "Oscillator 2 Waveform", WAVE_NAMES, 2),
    flt (Param::Osc2Pitch,  "Oscillator 2 Pitch",    Plain(" st"), 0.0),
    flt (Param::Osc2Level,  "Oscillator 2 Level",    Percent,      0.72),
    sel (Param::Osc3Wave,   "Oscillator 3 Waveform", WAVE_NAMES, 2),
    flt (Param::Osc3Pitch,  "Oscillator 3 Pitch",    Plain(" st"), 0.0),
    flt (Param::Osc3Level,  "Oscillator 3 Level",    Percent,      0.72),
    sel (Param::SyncSel,    "Oscillator Sync", SYNC_NAMES, 0),
    flt (Param::RingAmount, "Ring Modulation", Percent,    0.0),
    flt (Param::OscFm,      "Oscillator FM",   Percent,    0.0),
    flt (Param::KeyTrack,   "Key Tracking",    Percent,    0.4),
    flt (Param::MixSaw,     "Oscillator 1 Mix Sawtooth", Percent, 0.0),
    flt (Param::MixPulse,   "Oscillator 1 Mix Pulse",    Percent, 0.0),
    flt (Param::MixTri,     "Oscillator 1 Mix Triangle", Percent, 0.0),
    flt (Param::MixSine,    "Oscillator 1 Mix Sine",     Percent, 0.0),
    flt (Param::Unison,        "Unison Voices", Plain(""),    1.0),
    flt (Param::UnisonDetune,  "Unison Detune", Plain(" ct"), 12.0),
    flt (Param::ReverbTone,  "Reverb Tone",     Hertz,       5500.0),
    flt (Param::ReverbPre,   "Reverb Predelay", Plain(" s"), 0.012),
    flt (Param::DrumTone,  "Drum Bus Tone",   Fraction, 1.0),
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
    // Mixer desk (per-track strips) + the song-only chorus insert override
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
            match row.kind {
                Kind::Float { display, default, guarded } => ParamDef::Float(FloatDef {
                    id,
                    name: row.name,
                    param: row.param,
                    min,
                    max,
                    default,
                    display,
                    guarded,
                }),
                Kind::Choice { variants, default } => ParamDef::Choice(ChoiceDef {
                    id,
                    name: row.name,
                    param: row.param,
                    variants,
                    default,
                }),
            }
        })
        .collect()
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

#[cfg(test)]
mod tests {

    /// A parameter's position in PRESENTATION IS its AudioUnitParameterID,
    /// and hosts record automation curves against that number. Reordering the
    /// shipped block silently re-points every curve in every saved project —
    /// this pins the original 56 so new parameters can only be appended.
    #[test]
    fn shipped_parameter_ids_never_move() {
        const FROZEN: [Param; 56] = [
        Param::WaveformSel, Param::Volume, Param::Detune, Param::PulseWidth,
        Param::NoiseLevel, Param::LfoRate, Param::LfoShape, Param::LfoPitch,
        Param::LfoFilter, Param::LfoPwm, Param::Attack, Param::Decay,
        Param::Sustain, Param::Release, Param::Cutoff, Param::Resonance,
        Param::Drive, Param::Saturation, Param::HpfCutoff, Param::FilterEnvAmount,
        Param::FilterAttack, Param::FilterDecay, Param::FilterSustain, Param::FilterRelease,
        Param::FuzzAmount, Param::SpringWet, Param::ReverbDecay, Param::ReverbWet,
        Param::ChorusModeSel, Param::ChorusRate, Param::ChorusDepth, Param::TapeWow,
        Param::TapeFlutter, Param::TapeDrive, Param::TapeAge, Param::BdLevel,
        Param::BdTune, Param::BdAttack, Param::BdDecay, Param::BdSweep,
        Param::BdDrive, Param::SdLevel, Param::SdTune, Param::SdTone,
        Param::SdSnappy, Param::SdDecay, Param::RsLevel, Param::RsTune,
        Param::CpLevel, Param::CpDecay, Param::HhLevel, Param::HhTune,
        Param::HhMetal, Param::ChDecay, Param::OhDecay, Param::DrumDrive,
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
