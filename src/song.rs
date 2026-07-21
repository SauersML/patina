// src/song.rs
//
// A tiny text-based song format and player, used via `patina --play <file>`.
// Notes and parameter automation go through the VoiceManager exactly as if
// they came from the on-screen keyboard, a MIDI device, or the UI sliders.
//
// Format (one directive or a run of event tokens per line, `#` starts a comment):
//
//   bpm 100                  # global tempo (set once, at the top)
//   gate 0.85                # fraction of each note's duration it is held
//                            # (default 0.9). The musical intent is "a
//                            # small separation": the gap is (1-gate) of
//                            # the note but never more than 80 ms, so a
//                            # long pad doesn't end with beats of silence.
//   section CH1 144..180     # name a beat range. Sections make the beat
//                            # arithmetic the parser's job: `>CH1` seeks
//                            # any track to the section start, `>CH1.end`
//                            # to its end, and one-line automation can
//                            # target sections directly:
//                            #   automate vox_mode: 1 during CH1,CH2 base 0
//                            # (set 1 at each section start; `base` is the
//                            # value everywhere else — asserted at beat 0
//                            # and restored at each section end).
//
//   track lead vel=0.9 len=0.5   # start a note track; tracks play in parallel.
//                                # vel = default velocity (0..1)
//                                # len = default token duration in beats (default 1)
//                                # swing=0.56 leans every offbeat 16th late
//                                # by (swing-0.5) of the pair (0.5 = straight).
//                                # Mixer-strip options set the track's desk
//                                # channel at bar one: gain= pan= (-1..1)
//                                # reverb_send= spring_send= chorus_send=
//                                # (0..1, into the shared tanks at unity)
//                                # duck= (kick-keyed sidechain depth) and
//                                # duck_release= (seconds back to full).
//     E5:2 D5 C5 R:4 [C4 E4 G4]:2@0.6  | A4
//
//   track beat kit=909 len=0.5   # a drum track: kit= routes it to the
//     BD SD:0.5 [BD CH] OH@0.6   # rhythm section. Tokens are drum names
//                                # (BD SD RS CP CH OH) or GM notes; velocity
//                                # is the accent bus (@1 = full accent).
//
//   track choir vox              # a vox track: notes play the CARRIER
//     [A2 E3 A3]:2=HH-EH-L-OW    # (the synth chord) while the lyric drives
//     [F2 C3 F3]:2=W-ER-L-D      # the formant voice through the vocoder.
//                                # Lyrics are dash-joined ARPAbet phonemes
//                                # on the note: `=S-IH-NG`. Each phoneme
//                                # takes optional `:ms` (fixed length, ms)
//                                # and `@amp`: `=S:200@0.6-IH-NG-Z`.
//                                # Onsets speak at note-on, the vowel
//                                # sustains while held (pitch = lowest held
//                                # note), the coda speaks at note-off.
//                                # `wav=file.wav` on the track replaces the
//                                # built-in voice with a recording as the
//                                # vocoder's modulator (any voice you like).
//                                # The recording's clock is anchored to the
//                                # FIRST vox note-on; `wav_at=<beat>` makes
//                                # that anchor explicit and errors if the
//                                # first vox note is anywhere else — so
//                                # editing other tracks can never silently
//                                # shift the vocal. `pitch=curve.wav` (a
//                                # float32 wav of MIDI note numbers on the
//                                # modulator's clock) rides the same anchor.
//
//   track keys sample=tape.wav root=C3 loop=0.5:2.4 xfade=0.08
//     C3:4 [C3 Eb3 G3]:8    # a sampler track: the recording becomes an
//                           # instrument on the keys (sampler.rs). Notes
//                           # repitch the tape around root= (varispeed).
//                           # Options: start=/end= trim the region (secs),
//                           # loop or loop=a:b sustains it (equal-power
//                           # xfade= crossfade), chop=N slices the region
//                           # into N pads mapped chromatically up from
//                           # root (natural speed, one-shot), mode=gate|
//                           # oneshot, reverse, fixed (no keytracking),
//                           # mono/choke (new note cuts the last),
//                           # gain= pan= pitch= attack= release= vel_amt=,
//                           # cutoff=/res= (the slot's resonant lowpass),
//                           # beats=N (varispeed the region/loop to span
//                           # exactly N beats at the song tempo — break
//                           # matching), bits=/rate= (vintage converter:
//                           # resample+truncate at load and play through
//                           # the un-reconstructed ZOH DAC; bits=12
//                           # rate=26040 is the SP-1200's converter).
//                           # Playback is band-limited windowed-sinc with
//                           # the kernel widened when pitching up, so
//                           # varispeed doesn't alias in either direction.
//                           # Automate the transport per track:
//                           # `automate keys.smp_pitch` is a varispeed
//                           # knob in semitones; smp_start scrubs where
//                           # notes drop the needle; smp_cutoff/smp_res
//                           # sweep the filter; smp_gain, smp_pan,
//                           # smp_attack, smp_release reshape it live.
//
// Note-track tokens:
//   C4  F#3  Eb5  60      note names (C4 = MIDI 60) or raw MIDI numbers
//   [C4 E4 G4]            chord (notes start and stop together)
//   R  or  .              rest
//   :2                    duration suffix, in beats (floats allowed)
//   @0.7                  velocity suffix (0..1)
//   ~+0.02  ~-0.01        microtiming, in beats: push or drag this hit
//                         off the grid (the cursor stays on it)
//   |                     bar line, ignored (readability only)
//
// Automation tracks ramp a synth parameter through breakpoints:
//
//   automate cutoff
//     400 8000:16@exp R:8 400:4@smooth
//
//   automate bpm          # the tempo itself is a lane: ritardando,
//     170 90:16@smooth    # accelerando, half-time snaps — beats keep
//                         # their musical positions, time stretches
//
//   Per-track mixer lanes ride the same syntax: `automate lead.gain`,
//   `automate pad.pan`, `automate snare.reverb_send` (dub throws that
//   touch only the snare), `automate bass.duck`. `automate chorus_mix`
//   overrides the mode switch's insert mix (0 = bus dry, sends only).
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
// vox_level (0..1 vocoder into the bus), vox_dry (0..1 raw formant voice),
// vox_breath (0..1 aspiration), vox_vibrato (0..1 voice vibrato depth),
// vox_mode (0 = TalkBox-voiced band vocoder, 1 = full-range band
// vocoder, 2 = true Talker: LPC formant tracking, one continuous
// filter, no bands — the real talk-box circuit, 3 = spectral
// cross-synthesis: ~500-band FFT envelopes, words fully clear over the
// carrier's tone; plain sets),
// vox_intonation (0..1 autonomous pitch prosody: accents, declination,
// final falls — keep low when singing, high when speaking),
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
//
// The tape deck (per sampler track via `automate <track>.<param>`, or
// global to all slots): smp_pitch (semitones, -24..24), smp_start (0..1
// needle-drop point), smp_gain (0..2), smp_pan (-1..1), smp_attack,
// smp_release (seconds), smp_cutoff (Hz, 20000 = open), smp_res (0..1).

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::chorus::ChorusMode;
use crate::oscillator::Waveform;
use crate::voice_manager::{ParamValues, VoiceManager};

// Automation curves are sampled at this many points per beat
const AUTOMATION_STEPS_PER_BEAT: f64 = 32.0;

/// The longest silence `gate` may carve off a note's end, in seconds:
/// enough to articulate a separation, never enough to eat a word.
const GATE_GAP_MAX_S: f64 = 0.08;

/// How a parameter's range is traversed by a fader or knob.
#[derive(Clone, Copy, PartialEq)]
pub enum Curve {
    Lin,
    Log,
    Step,
}

/// One row of THE parameter table (see `param_table!` below).
pub struct ParamDef {
    pub name: &'static str,
    pub param: Param,
    pub cc: Option<u8>,
}

/// The structural unification: a single macro invocation declares every
/// parameter's variant, song-file name, MIDI CC, range, and taper in ONE
/// row, and generates the `Param` enum, `PARAM_DEFS`, and `range()` from
/// it. A variant cannot exist without its name/cc/range, and they cannot
/// disagree — the mismatch is unrepresentable, not merely tested-for.
macro_rules! param_table {
    ($( $(#[$meta:meta])* $variant:ident : $name:literal, $cc:expr, ($lo:expr, $hi:expr, $curve:ident); )+) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub enum Param {
            $( $(#[$meta])* $variant, )+
        }

        /// Every parameter's name/cc row, in table order (`--params`,
        /// the parser, the CC chart, and the sweep test all walk this).
        pub const PARAM_DEFS: &[ParamDef] = &[
            $( ParamDef { name: $name, param: Param::$variant, cc: $cc }, )+
        ];

        impl Param {
            /// Bounds and taper, straight from the table row. Knobs,
            /// MIDI CCs, and setter clamps (via `Param::clamp`) all read
            /// this, so a range change lands everywhere at once.
            pub fn range(self) -> (f32, f32, Curve) {
                match self {
                    $( Param::$variant => ($lo, $hi, Curve::$curve), )+
                }
            }
        }
    };
}

#[rustfmt::skip]
param_table! {
    // ------------------------------------------------------------------
    // THE parameter table. One row = one parameter: enum variant, song-
    // file name, MIDI CC, range and taper. The macro generates the Param
    // enum, PARAM_DEFS, and Param::range() from these SAME rows, so a
    // parameter cannot exist with a missing or contradictory name/cc/
    // range — the old three-place-edit class of bug is unrepresentable.
    // Setter clamps derive their bounds via Param::clamp (backstopped by
    // the no_silently_clamped_params sweep test).
    //
    // The CC chart keeps its standard assignments where they exist
    // (1 mod wheel, 5 portamento, 7 volume, 64 sustain, 71/74
    // resonance/cutoff, 72/73/75/79 envelope, 91/93 sends); the 102-119
    // block carries the engine-specific rest, the drums claim 20-31 and
    // 52-60, the tape deck the leftover low block.
    // ------------------------------------------------------------------
    Volume:          "volume",         Some(7),   (0.0, 1.0, Lin);
    WaveformSel:     "waveform",       Some(113), (0.0, 3.0, Step);
    Detune:          "detune",         Some(83),  (0.0, 30.0, Lin);
    Cutoff:          "cutoff",         Some(74),  (20.0, 20000.0, Log);
    Resonance:       "resonance",      Some(71),  (0.0, 4.0, Lin);
    Drive:           "drive",          Some(103), (0.1, 5.0, Lin);
    Saturation:      "saturation",     Some(104), (0.0, 2.0, Lin);
    Attack:          "attack",         Some(73),  (0.01, 2.0, Log);
    Decay:           "decay",          Some(75),  (0.01, 2.0, Log);
    Sustain:         "sustain",        Some(79),  (0.0, 1.0, Lin);
    Release:         "release",        Some(72),  (0.01, 2.0, Log);
    HpfCutoff:       "hpf",            Some(102), (16.0, 8000.0, Log);
    FuzzAmount:      "fuzz",           None,      (0.0, 1.0, Lin);
    NoiseLevel:      "noise",          Some(81),  (0.0, 1.0, Lin);
    SpringWet:       "spring",         Some(95),  (0.0, 1.0, Lin);
    Glide:           "glide",          Some(5),   (0.0, 2.0, Lin);
    SubLevel:        "sub",            Some(80),  (0.0, 1.0, Lin);
    Osc2Wave:        "osc2_wave",      Some(114), (0.0, 3.0, Step);
    Osc2Pitch:       "osc2_pitch",     Some(86),  (-24.0, 24.0, Step);
    Osc2Level:       "osc2_level",     Some(85),  (0.0, 1.0, Lin);
    Osc3Wave:        "osc3_wave",      Some(115), (0.0, 3.0, Step);
    Osc3Pitch:       "osc3_pitch",     Some(88),  (-24.0, 24.0, Step);
    Osc3Level:       "osc3_level",     Some(87),  (0.0, 1.0, Lin);
    CircuitSel:      "circuit",        Some(116), (0.0, 1.0, Step);
    KeyTrack:        "key_track",      Some(105), (0.0, 1.0, Lin);
    OscFm:           "osc_fm",         Some(89),  (0.0, 1.0, Lin);
    SyncSel:         "sync",           Some(117), (0.0, 1.0, Step);
    RingAmount:      "ring",           Some(90),  (0.0, 1.0, Lin);
    // Keyboard register the UI should sit at; patches set it so a bass
    // preset arrives with the keys already down where it lives
    UiOctave:        "octave",         None,      (0.0, 8.0, Step);
    PitchBendSemis:  "bend",           None,      (-2.0, 2.0, Lin);
    ModWheel:        "mod_wheel",      Some(1),   (0.0, 1.0, Lin);
    SustainPedal:    "pedal",          Some(64),  (0.0, 1.0, Lin);
    PulseWidth:      "pulse_width",    Some(82),  (0.05, 0.95, Lin);
    // Oscillator 1's source mixer: four 0..1 converter levels
    MixSaw:          "mix_saw",        None,      (0.0, 1.0, Lin);
    MixPulse:        "mix_pulse",      None,      (0.0, 1.0, Lin);
    MixTri:          "mix_tri",        None,      (0.0, 1.0, Lin);
    MixSine:         "mix_sine",       None,      (0.0, 1.0, Lin);
    LfoRate:         "lfo_rate",       Some(76),  (0.1, 30.0, Log);
    LfoShape:        "lfo_shape",      None,      (0.0, 1.0, Lin);
    LfoPitch:        "lfo_pitch",      Some(77),  (0.0, 200.0, Lin);
    LfoFilter:       "lfo_filter",     Some(78),  (0.0, 4.0, Lin);
    LfoPwm:          "lfo_pwm",        None,      (0.0, 0.45, Lin);
    FilterEnvAmount: "filter_env",     Some(106), (-5.0, 5.0, Lin);
    FilterAttack:    "filter_attack",  Some(107), (0.001, 2.0, Log);
    FilterDecay:     "filter_decay",   Some(108), (0.01, 2.0, Log);
    FilterSustain:   "filter_sustain", Some(109), (0.0, 1.0, Lin);
    FilterRelease:   "filter_release", Some(110), (0.01, 2.0, Log);
    ReverbDecay:     "reverb_decay",   None,      (0.0, 0.99, Lin);
    ReverbWet:       "reverb_wet",     Some(91),  (0.0, 1.0, Lin);
    ChorusModeSel:   "chorus_mode",    Some(112), (0.0, 4.0, Step);
    ChorusRate:      "chorus_rate",    Some(111), (0.1, 10.0, Log);
    ChorusDepth:     "chorus_depth",   Some(93),  (0.0, 1.0, Lin);
    TapeWow:         "tape_wow",       Some(92),  (0.0, 1.0, Lin);
    TapeFlutter:     "tape_flutter",   Some(94),  (0.0, 1.0, Lin);
    TapeDrive:       "tape_drive",     Some(118), (0.0, 1.0, Lin);
    TapeAge:         "tape_age",       Some(119), (0.0, 1.0, Lin);
    // The rhythm section: one shared 909 board, so like the effects and
    // the LFO these are bus-level parameters — unitless 0..1 panel knob
    // rotations; the circuits map them onto their electrical ranges
    BdLevel:         "bd_level",       Some(20),  (0.0, 1.0, Lin);
    BdTune:          "bd_tune",        Some(21),  (0.0, 1.0, Lin);
    BdAttack:        "bd_attack",      Some(22),  (0.0, 1.0, Lin);
    BdDecay:         "bd_decay",       Some(23),  (0.0, 1.0, Lin);
    BdSweep:         "bd_sweep",       Some(24),  (0.0, 1.0, Lin);
    BdDrive:         "bd_drive",       Some(25),  (0.0, 1.0, Lin);
    SdLevel:         "sd_level",       Some(26),  (0.0, 1.0, Lin);
    SdTune:          "sd_tune",        Some(27),  (0.0, 1.0, Lin);
    SdTone:          "sd_tone",        Some(28),  (0.0, 1.0, Lin);
    SdSnappy:        "sd_snappy",      Some(29),  (0.0, 1.0, Lin);
    SdDecay:         "sd_decay",       Some(30),  (0.0, 1.0, Lin);
    RsLevel:         "rs_level",       Some(31),  (0.0, 1.0, Lin);
    RsTune:          "rs_tune",        Some(52),  (0.0, 1.0, Lin);
    CpLevel:         "cp_level",       Some(53),  (0.0, 1.0, Lin);
    CpDecay:         "cp_decay",       Some(54),  (0.0, 1.0, Lin);
    HhLevel:         "hh_level",       Some(55),  (0.0, 1.0, Lin);
    HhTune:          "hh_tune",        Some(56),  (0.0, 1.0, Lin);
    HhMetal:         "hh_metal",       Some(57),  (0.0, 1.0, Lin);
    ChDecay:         "ch_decay",       Some(58),  (0.0, 1.0, Lin);
    OhDecay:         "oh_decay",       Some(59),  (0.0, 1.0, Lin);
    DrumDrive:       "dr_drive",       Some(60),  (0.0, 1.0, Lin);
    // The voice box (vox.rs). vox_level reaches 2: the band vocoder's
    // per-band tanh caps its own output (see vocoder.rs's gain-staging
    // contract), so headroom above unity is the only push a song can
    // give a vocoder chorus.
    VoxLevel:        "vox_level",      Some(12),  (0.0, 2.0, Lin);
    VoxDry:          "vox_dry",        Some(13),  (0.0, 1.0, Lin);
    // CC2 is the MIDI breath controller -- it belongs to the voice
    VoxBreath:       "vox_breath",     Some(2),   (0.0, 1.0, Lin);
    // Talker circuit only: 0 = reference-matched caricature, 1 = legible
    VoxClarity:      "vox_clarity",    None,      (0.0, 1.0, Lin);
    VoxVibrato:      "vox_vibrato",    Some(14),  (0.0, 1.0, Lin);
    // 0 TalkBox / 1 studio vocoder / 2 Talker (LPC) / 3 spectral
    VoxModeSel:      "vox_mode",       Some(15),  (0.0, 3.0, Step);
    VoxIntonation:   "vox_intonation", Some(16),  (0.0, 1.0, Lin);
    // The tape deck (sampler.rs): per-slot transport controls, routed to
    // a slot by the track's channel (global automation reaches all slots)
    SmpPitch:        "smp_pitch",      Some(9),   (-24.0, 24.0, Lin);
    SmpStart:        "smp_start",      Some(17),  (0.0, 1.0, Lin);
    SmpGain:         "smp_gain",       Some(8),   (0.0, 2.0, Lin);
    // CC10 is the standard pan, which here pans the sampler
    SmpPan:          "smp_pan",        Some(10),  (-1.0, 1.0, Lin);
    SmpAttack:       "smp_attack",     Some(18),  (0.001, 4.0, Log);
    SmpRelease:      "smp_release",    Some(19),  (0.003, 8.0, Log);
    SmpCutoff:       "smp_cutoff",     Some(3),   (60.0, 20000.0, Log);
    SmpRes:          "smp_res",        Some(4),   (0.0, 1.0, Lin);
    // The mixer desk (voice_manager::ChannelMix): every track owns a
    // strip. Channel-scoped: address them as `automate <track>.<param>`.
    TrackGain:       "gain",           None,      (0.0, 2.0, Lin);
    TrackPan:        "pan",            None,      (-1.0, 1.0, Lin);
    ReverbSend:      "reverb_send",    None,      (0.0, 1.0, Lin);
    SpringSend:      "spring_send",    None,      (0.0, 1.0, Lin);
    ChorusSend:      "chorus_send",    None,      (0.0, 1.0, Lin);
    DuckAmount:      "duck",           None,      (0.0, 1.0, Lin);
    DuckRelease:     "duck_release",   None,      (0.02, 2.0, Lin);
    // Global chorus insert mix override (the mode switch re-derives it)
    ChorusMix:       "chorus_mix",     None,      (0.0, 1.0, Lin);
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
    /// Look a parameter up by its MIDI CC (the chart lives in PARAM_DEFS).
    pub fn from_cc(cc: u8) -> Option<Param> {
        PARAM_DEFS.iter().find(|d| d.cc == Some(cc)).map(|d| d.param)
    }

    /// Clamp a value to this parameter's documented range. Setters use
    /// this instead of hand-written bounds, so a setter clamp can never
    /// drift from the table (the vox_mode disease). Engine-safety limits
    /// that are DELIBERATELY wider than the musical range (detune,
    /// glide, pitch bend) keep their own literals, with a comment.
    pub fn clamp(self, v: f32) -> f32 {
        let (lo, hi, _) = self.range();
        v.clamp(lo, hi)
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

}

impl Param {
    /// Look a parameter up by its song-file name (the table is PARAM_DEFS).
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        PARAM_DEFS.iter().find(|d| d.name == name).map(|d| d.param)
    }

    /// The song-file name of this parameter, from the same table.
    pub fn name(self) -> &'static str {
        PARAM_DEFS
            .iter()
            .find(|d| d.param == self)
            .map(|d| d.name)
            .unwrap_or("?")
    }

    pub(crate) fn apply(self, vm: &mut VoiceManager, value: f32) {
        match self {
            Param::Volume => vm.set_volume(value),
            Param::TrackGain | Param::TrackPan | Param::ReverbSend
            | Param::SpringSend | Param::ChorusSend | Param::DuckAmount
            | Param::DuckRelease => vm.set_track_mix(0, self, value),
            Param::ChorusMix => vm.set_chorus_mix(value),
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
            Param::MixSaw => vm.set_osc1_mix_component(0, value),
            Param::MixPulse => vm.set_osc1_mix_component(1, value),
            Param::MixTri => vm.set_osc1_mix_component(2, value),
            Param::MixSine => vm.set_osc1_mix_component(3, value),
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
            Param::VoxLevel => vm.set_vox_level(value),
            Param::VoxDry => vm.set_vox_dry(value),
            Param::VoxBreath => vm.set_vox_breath(value),
            Param::VoxClarity => vm.set_vox_clarity(value),
            Param::VoxVibrato => vm.set_vox_vibrato(value),
            Param::VoxModeSel => vm.set_vox_mode(value),
            Param::VoxIntonation => vm.set_vox_intonation(value),
            // Un-addressed sampler automation reaches every slot (the
            // per-track path routes by channel before it gets here)
            Param::SmpPitch | Param::SmpStart | Param::SmpGain | Param::SmpPan
            | Param::SmpAttack | Param::SmpRelease | Param::SmpCutoff
            | Param::SmpRes => vm.set_sampler_all(self, value),
        }
    }

    /// Write a VOICE-LEVEL parameter into a snapshot (per-track patches).
    /// Returns false for bus-level parameters — effects, LFO, noise,
    /// volume, performance controllers — which are shared by nature and
    /// fall through to the global path.
    pub(crate) fn apply_to_params(self, p: &mut ParamValues, value: f32) -> bool {
        match self {
            Param::WaveformSel => {
                // Same macro semantics as the live panel: `waveform` sets
                // all three oscillators; per-osc lines override after it
                let w = waveform_from_value(value);
                p.waveform = w;
                p.osc2_wave = w;
                p.osc3_wave = w;
            }
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
            Param::MixSaw => p.mix_saw = value.clamp(0.0, 1.0),
            Param::MixPulse => p.mix_pulse = value.clamp(0.0, 1.0),
            Param::MixTri => p.mix_tri = value.clamp(0.0, 1.0),
            Param::MixSine => p.mix_sine = value.clamp(0.0, 1.0),
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
    let mut p = ParamValues::neutral();
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
    /// A syllable for the voice box, landing just before its note-on.
    Lyric { syl: crate::vox::Syllable, channel: u16 },
}

pub struct SongEvent {
    pub time: f64, // seconds from song start
    pub kind: EventKind,
}

/// A parsed song: the timed events plus each patch channel's parameter
/// snapshot (channel N+1 = channels[N]; channel 0 is the live panel).
pub struct Song {
    pub events: Vec<SongEvent>,
    pub channels: Vec<ParamValues>,
    /// A recorded vocoder modulator (`wav=` on a vox track): mono samples
    /// and their source rate, resampled by the engine on registration.
    pub vox_wav: Option<(Vec<f32>, u32)>,
    /// The performance pitch line (`pitch=` on a vox track): MIDI note
    /// numbers in a float32 wav, on the modulator's clock.
    pub vox_pitch: Option<(Vec<f32>, u32)>,
    /// The tape deck: slot i (= channel SAMPLER_CHANNEL_BASE + i) holds a
    /// loaded reel and its transport, from `sample=` tracks.
    pub samplers: Vec<crate::sampler::SamplerSlot>,
    /// Every `track` line's name and the channel it landed on, in file
    /// order — the map that stem bounces and event exports speak.
    pub tracks: Vec<(String, u16)>,
}

pub fn load_song(path: &str) -> Result<Song, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read song file '{}': {}", path, e))?;
    parse_song(&text)
}

/// Parse song text directly (the `--say` builder and tests use this).
pub fn parse_song_text(text: &str) -> Result<Song, String> {
    parse_song(text)
}

fn dispatch(vm: &mut VoiceManager, kind: &EventKind) {
    match kind {
        &EventKind::NoteOn { note, velocity, channel } => {
            vm.note_on_channel(note, velocity, channel)
        }
        &EventKind::NoteOff { note, channel } => vm.note_off_channel(note, channel),
        &EventKind::Param { param, value, channel } => {
            vm.set_channel_param(channel, param, value)
        }
        EventKind::Lyric { syl, channel } => vm.set_lyric(*channel, syl.clone()),
    }
}

fn register_channels(vm: &mut VoiceManager, song: &Song) {
    for (i, p) in song.channels.iter().enumerate() {
        vm.set_channel_params((i + 1) as u16, *p);
    }
    if let Some((samples, rate)) = &song.vox_wav {
        vm.set_vox_wav(samples, *rate);
    }
    if let Some((samples, rate)) = &song.vox_pitch {
        vm.set_vox_pitch(samples, *rate);
    }
    for (i, slot) in song.samplers.iter().enumerate() {
        vm.set_sampler_slot(i, slot.clone());
    }
}

/// Render a song offline, as fast as the CPU allows: same events, same
/// engine, no audio device. Returns interleaved-by-frame stereo samples,
/// with a few seconds of tail for reverb and tape print-through to ring out.
pub fn render_offline(song: &Song, sample_rate: f32) -> Vec<(f32, f32)> {
    render_offline_solo(song, sample_rate, None)
}

/// Render with one channel soloed (stem bounces): everything still runs —
/// oscillators free-run, tempo and automation march — but only the solo
/// channel's strip reaches the bus and the sends.
pub fn render_offline_solo(
    song: &Song,
    sample_rate: f32,
    solo: Option<u16>,
) -> Vec<(f32, f32)> {
    let mut vm = VoiceManager::new(sample_rate, 10);
    vm.set_solo(solo);
    // A bounce records a warmed-up instrument, not a cold power-on
    vm.warm_up();
    register_channels(&mut vm, song);
    let events = &song.events;
    let end = events.last().map(|e| e.time).unwrap_or(0.0) + 4.0;
    let total = (end * sample_rate as f64) as usize;
    let mut out = Vec::with_capacity(total);
    let mut next = 0;
    // Allocation-pressure telemetry: the deepest simultaneous claim on
    // the voice cards, so chorus voicings can be checked against the
    // card limit without hand-counting note-ons
    let mut peak_voices = 0usize;
    for n in 0..total {
        let t = n as f64 / sample_rate as f64;
        let mut fired = false;
        while next < events.len() && events[next].time <= t {
            dispatch(&mut vm, &events[next].kind);
            next += 1;
            fired = true;
        }
        if fired {
            let active = vm.voices.iter().filter(|v| v.is_active()).count();
            peak_voices = peak_voices.max(active);
        }
        out.push(vm.render_next());
    }
    println!("peak concurrent voices: {}/{}", peak_voices, vm.voices.len());
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
    Notes { vel: f32, len: f64, channel: u16, swing: f64 },
    Automation { param: Param, current: Option<f32>, channel: u16 },
    /// `automate bpm`: tokens land on the tempo map, not the event list
    Tempo { current: Option<f32> },
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
    // `automate bpm` breakpoints, kept in beat-space until the end
    let mut tempo_lane: Vec<(f64, AutoToken)> = Vec::new();
    let mut vox_wav: Option<(Vec<f32>, u32)> = None;
    let mut vox_pitch: Option<(Vec<f32>, u32)> = None;
    // `wav_at=`: the beat the vox recording's clock is anchored to
    let mut vox_wav_at: Option<f64> = None;
    let mut samplers: Vec<crate::sampler::SamplerSlot> = Vec::new();
    // Named beat ranges (`section CH1 144..180`), for seeks and `during`
    let mut sections: std::collections::HashMap<String, (f64, f64)> =
        std::collections::HashMap::new();

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
            "section" => {
                let mut it = line.split_whitespace().skip(1);
                let name = it
                    .next()
                    .ok_or_else(|| err("section needs a name and a range: section CH1 144..180".into()))?;
                let range = it
                    .next()
                    .ok_or_else(|| err(format!("section {} needs a range, e.g. 144..180", name)))?;
                let (a, b) = range
                    .split_once("..")
                    .ok_or_else(|| err(format!("section range must be A..B, got '{}'", range)))?;
                let a: f64 = a.parse().map_err(|_| err(format!("invalid section start '{}'", a)))?;
                let b: f64 = b.parse().map_err(|_| err(format!("invalid section end '{}'", b)))?;
                if b <= a || a < 0.0 {
                    return Err(err(format!("section {}: end must be after start", name)));
                }
                if sections.insert(name.to_string(), (a, b)).is_some() {
                    return Err(err(format!("section '{}' defined twice", name)));
                }
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
                // A sampler track: `sample=` loads a reel and the other
                // tape-deck options refine its transport, in any order
                let mut smp: Option<crate::sampler::SlotConfig> =
                    if line.split_whitespace().any(|o| o.starts_with("sample=")) {
                        Some(Default::default())
                    } else {
                        None
                    };
                let mut smp_data: Option<std::sync::Arc<crate::sampler::SampleData>> = None;
                let mut smp_mode_set = false;
                // Reel-preparation options, applied once the file is known
                let mut smp_bits: Option<u32> = None;
                let mut smp_rate: Option<u32> = None;
                let mut smp_beats: Option<f64> = None;
                let mut swing = 0.5f64;
                // mixer-strip options (gain= pan= reverb_send= ... duck=)
                // become Param events at beat 0 on this track's channel
                let mut mix_opts: Vec<(Param, f32)> = Vec::new();
                for opt in line.split_whitespace().skip(2) {
                    if let Some(v) = opt.strip_prefix("vel=") {
                        vel = v.parse::<f32>().map_err(|_| err(format!("invalid vel '{}'", v)))?;
                    } else if let Some(v) = opt.strip_prefix("len=") {
                        len = v.parse::<f64>().map_err(|_| err(format!("invalid len '{}'", v)))?;
                    } else if let Some(v) = opt.strip_prefix("swing=") {
                        swing = v
                            .parse::<f64>()
                            .map_err(|_| err(format!("invalid swing '{}'", v)))?;
                        if !(0.4..=0.8).contains(&swing) {
                            return Err(err("swing must be 0.4-0.8 (0.5 = straight)".into()));
                        }
                    } else if let Some((k, v)) = opt.split_once('=').filter(|(k, _)| {
                        matches!(
                            Param::from_name(k),
                            Some(
                                Param::TrackGain
                                    | Param::TrackPan
                                    | Param::ReverbSend
                                    | Param::SpringSend
                                    | Param::ChorusSend
                                    | Param::DuckAmount
                                    | Param::DuckRelease
                            )
                        )
                    }) {
                        let value = v
                            .parse::<f32>()
                            .map_err(|_| err(format!("invalid {} '{}'", k, v)))?;
                        mix_opts.push((Param::from_name(k).unwrap(), value));
                    } else if opt.strip_prefix("kit=").is_some() {
                        // A drum track: notes route to the rhythm section
                        // (there is one board, so no per-track patches here)
                        channel = crate::drums::DRUM_CHANNEL;
                    } else if opt == "vox" {
                        // The voice box: notes play the vocoder's carrier,
                        // `=lyric` suffixes drive the formant voice
                        channel = crate::vox::VOX_CHANNEL;
                    } else if let Some(v) = opt.strip_prefix("wav=") {
                        // A recorded modulator for the vocoder, instead of
                        // the built-in formant voice
                        vox_wav = Some(crate::vox::load_wav_mono(v).map_err(err)?);
                        channel = crate::vox::VOX_CHANNEL;
                    } else if let Some(v) = opt.strip_prefix("pitch=") {
                        // The performance line: sample-accurate carrier
                        // pitch (portamento, scoops, vibrato), authored
                        // as MIDI notes in a float32 wav on the
                        // modulator's clock. Float32 ONLY: a PCM curve
                        // would arrive normalized and transpose the
                        // melody to nonsense.
                        vox_pitch = Some(crate::vox::load_wav_mono_float(v).map_err(err)?);
                        channel = crate::vox::VOX_CHANNEL;
                    } else if let Some(v) = opt.strip_prefix("wav_at=") {
                        // The modulator clock, made explicit: the wav
                        // (and any pitch= curve) starts at the FIRST vox
                        // note-on, and this declares which beat that is —
                        // checked after parsing, so removing some other
                        // note can never silently shift the vocal
                        let b = v
                            .parse::<f64>()
                            .map_err(|_| err(format!("invalid wav_at '{}'", v)))?;
                        if b < 0.0 {
                            return Err(err("wav_at must be >= 0".into()));
                        }
                        vox_wav_at = Some(b);
                    } else if let Some(v) = opt.strip_prefix("patch=") {
                        // A private patch for this track: the file's
                        // voice-level parameters become this channel
                        let path = format!("patches/{}.patch", v);
                        let text = std::fs::read_to_string(&path)
                            .map_err(|e| err(format!("patch '{}': {}", path, e)))?;
                        let p = params_from_patch(&text).map_err(err)?;
                        channels.push(p);
                        channel = channels.len() as u16;
                    } else if let Some(v) = opt.strip_prefix("sample=") {
                        smp_data = Some(std::sync::Arc::new(
                            crate::sampler::load_wav_stereo(v).map_err(err)?,
                        ));
                    } else if smp.is_some() && opt.starts_with("bits=") {
                        let v = &opt[5..];
                        let n: u32 = v
                            .parse()
                            .map_err(|_| err(format!("invalid bits '{}'", v)))?;
                        if !(4..=16).contains(&n) {
                            return Err(err("bits must be 4-16".into()));
                        }
                        smp_bits = Some(n);
                    } else if smp.is_some() && opt.starts_with("rate=") {
                        let v = &opt[5..];
                        let n: u32 = v
                            .parse()
                            .map_err(|_| err(format!("invalid rate '{}'", v)))?;
                        if !(2000..=96000).contains(&n) {
                            return Err(err("rate must be 2000-96000 Hz".into()));
                        }
                        smp_rate = Some(n);
                    } else if smp.is_some() && opt.starts_with("beats=") {
                        let v = &opt[6..];
                        let n: f64 = v
                            .parse()
                            .map_err(|_| err(format!("invalid beats '{}'", v)))?;
                        if n <= 0.0 {
                            return Err(err("beats must be positive".into()));
                        }
                        smp_beats = Some(n);
                    } else if let Some(cfg) = smp.as_mut() {
                        parse_sampler_option(cfg, opt, &mut smp_mode_set).map_err(err)?;
                    } else {
                        return Err(err(format!("unknown track option '{}'", opt)));
                    }
                }
                if let Some(mut cfg) = smp {
                    let mut data = smp_data
                        .ok_or_else(|| err("sampler track needs sample=file.wav".into()))?;
                    // Chop pads are drum pads: fire-and-forget unless the
                    // track says otherwise
                    if cfg.chop > 1 && !smp_mode_set {
                        cfg.mode = crate::sampler::PlayMode::OneShot;
                    }
                    // Vintage converters: resample/truncate the reel once
                    // at load, and play it back through the ZOH DAC
                    if smp_bits.is_some() || smp_rate.is_some() {
                        data = std::sync::Arc::new(crate::sampler::crunch(
                            &data, smp_bits, smp_rate,
                        ));
                        cfg.zoh = true;
                    }
                    // beats=N: fit the playback region (the loop if there
                    // is one) to exactly N beats at the song's tempo
                    if let Some(beats) = smp_beats {
                        let rate = data.rate.max(1) as f64;
                        let total = data.frames() as f64 / rate;
                        let (a, b) = match cfg.loop_pts {
                            Some((a, b)) => {
                                ((a as f64).max(cfg.start as f64), (b as f64).min(total))
                            }
                            None => (
                                cfg.start as f64,
                                if cfg.end > 0.0 { cfg.end as f64 } else { total },
                            ),
                        };
                        let secs = (b - a).max(1e-3);
                        cfg.speed = (secs / (beats * 60.0 / bpm)).clamp(0.03, 32.0) as f32;
                    }
                    if samplers.len() >= crate::sampler::MAX_SLOTS {
                        return Err(err(format!(
                            "too many sampler tracks (max {})",
                            crate::sampler::MAX_SLOTS
                        )));
                    }
                    channel = crate::sampler::SAMPLER_CHANNEL_BASE + samplers.len() as u16;
                    samplers.push(crate::sampler::SamplerSlot { data, cfg });
                }
                for (param, value) in mix_opts {
                    events.push((0.0, 1, EventKind::Param { param, value, channel }));
                }
                track_channels.push((name, channel));
                mode = TrackMode::Notes { vel, len, channel, swing };
            }
            "automate" => {
                let toks: Vec<&str> = line.split_whitespace().collect();
                let name = toks
                    .get(1)
                    .copied()
                    .ok_or_else(|| err("automate needs a parameter name".into()))?
                    .trim_end_matches(':');
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
                // One-line section automation:
                //   automate vox_mode: 1 during CH1,CH2 [base 0]
                // sets the value at each section's start; `base` is the
                // value everywhere else (asserted at beat 0, restored at
                // each section end). The beat arithmetic that used to be
                // hand-built R:.../value token chains is the parser's job.
                if toks.get(3).copied() == Some("during") {
                    if pname == "bpm" {
                        return Err(err("`during` cannot drive the tempo lane".into()));
                    }
                    let param = Param::from_name(pname)
                        .ok_or_else(|| err(format!("unknown parameter '{}'", pname)))?;
                    let value: f32 = toks[2]
                        .parse()
                        .map_err(|_| err(format!("invalid value '{}'", toks[2])))?;
                    let names = toks.get(4).copied().ok_or_else(|| {
                        err("`during` needs section names, e.g. during CH1,CH2".into())
                    })?;
                    let base: Option<f32> = match toks.get(5).copied() {
                        Some("base") => Some(
                            toks.get(6)
                                .copied()
                                .ok_or_else(|| err("`base` needs a value".into()))?
                                .parse()
                                .map_err(|_| err("invalid base value".into()))?,
                        ),
                        Some(t) => return Err(err(format!("unexpected token '{}'", t))),
                        None => None,
                    };
                    if toks.len() > 7 {
                        return Err(err(format!("unexpected token '{}'", toks[7])));
                    }
                    if let Some(base) = base {
                        events.push((0.0, 1, EventKind::Param { param, value: base, channel }));
                    }
                    for sname in names.split(',').filter(|s| !s.is_empty()) {
                        let &(a, b) = sections.get(sname).ok_or_else(|| {
                            err(format!("unknown section '{}' (define it above)", sname))
                        })?;
                        events.push((a, 1, EventKind::Param { param, value, channel }));
                        if let Some(base) = base {
                            events.push((b, 1, EventKind::Param { param, value: base, channel }));
                        }
                    }
                    mode = TrackMode::None;
                    continue;
                }
                if pname == "bpm" && channel == 0 {
                    track_beat = 0.0;
                    mode = TrackMode::Tempo { current: None };
                    continue;
                }
                let param = Param::from_name(pname)
                    .ok_or_else(|| err(format!("unknown parameter '{}'", pname)))?;
                track_beat = 0.0;
                mode = TrackMode::Automation { param, current: None, channel };
            }
            _ => match &mut mode {
                TrackMode::None => {
                    return Err(err("event tokens before any 'track' or 'automate' line".into()));
                }
                TrackMode::Notes { vel, len, channel, swing } => {
                    let swing = *swing;
                    let (vel, len, channel) = (*vel, *len, *channel);
                    let drums = channel == crate::drums::DRUM_CHANNEL;
                    let line = expand_groups(line).map_err(err)?;
                    for token in tokenize(&line).map_err(err)? {
                        if token == "|" {
                            continue;
                        }
                        if let Some(beat) = token.strip_prefix('>') {
                            track_beat = resolve_seek(beat, &sections).map_err(err)?;
                            continue;
                        }
                        // `=lyric` rides the note it belongs to; split it
                        // off before note parsing (phoneme durations use
                        // ':' and '@' of their own)
                        let (token, lyric) = match token.rfind('=') {
                            Some(i) => (token[..i].to_string(), Some(&token[i + 1..])),
                            None => (token.clone(), None),
                        };
                        let (notes, dur, vel, shift) =
                            parse_note_token(&token, vel, len, drums)
                                .map_err(|m| err(format!("token '{}': {}", token, m)))?;
                        // swing: every offbeat 16th in the pair leans late
                        // by (swing - 0.5) of the pair; the cursor stays
                        // on the grid so durations never accumulate error
                        let ph = track_beat.rem_euclid(0.5);
                        let lean = if swing != 0.5 && (ph - 0.25).abs() < 1e-6 {
                            (swing - 0.5) * 0.5
                        } else {
                            0.0
                        };
                        let sound_beat = (track_beat + shift + lean).max(0.0);
                        if let Some(lyric) = lyric {
                            if channel != crate::vox::VOX_CHANNEL {
                                return Err(err(format!(
                                    "token '{}': lyrics need a vox track (add `vox` to the track line)",
                                    token
                                )));
                            }
                            if notes.is_empty() {
                                return Err(err(format!(
                                    "token '{}': a lyric needs a note to ride on, not a rest",
                                    token
                                )));
                            }
                            let syl = crate::vox::parse_lyric(lyric)
                                .map_err(|m| err(format!("token '{}': {}", token, m)))?;
                            let ph = track_beat.rem_euclid(0.5);
                            let lean = if swing != 0.5 && (ph - 0.25).abs() < 1e-6 {
                                (swing - 0.5) * 0.5
                            } else {
                                0.0
                            };
                            events.push((
                                (track_beat + lean).max(0.0),
                                1,
                                EventKind::Lyric { syl, channel },
                            ));
                        }
                        // gate means "a small separation", not a fraction
                        // of ANY note: proportional for short notes,
                        // capped at 80 ms for long ones — gate 0.98 on a
                        // 128-beat pad must not cut beats of held keys.
                        // (The cap uses the base bpm; a tempo lane
                        // stretches it with everything else.)
                        let gap = ((1.0 - gate) * dur).min(GATE_GAP_MAX_S * bpm / 60.0);
                        let off_beat = sound_beat + dur - gap;
                        for &note in &notes {
                            events.push((
                                sound_beat,
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
                    let line = expand_groups(line).map_err(err)?;
                    for token in tokenize(&line).map_err(err)? {
                        if token == "|" {
                            continue;
                        }
                        if let Some(beat) = token.strip_prefix('>') {
                            track_beat = resolve_seek(beat, &sections).map_err(err)?;
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
                TrackMode::Tempo { current } => {
                    let line = expand_groups(line).map_err(err)?;
                    for token in tokenize(&line).map_err(err)? {
                        if token == "|" {
                            continue;
                        }
                        if let Some(beat) = token.strip_prefix('>') {
                            track_beat = resolve_seek(beat, &sections).map_err(err)?;
                            continue;
                        }
                        let seg = parse_automation_token(&token)
                            .map_err(|m| err(format!("token '{}': {}", token, m)))?;
                        match seg {
                            AutoToken::Hold(dur) => track_beat += dur,
                            AutoToken::Set(value) => {
                                if !(20.0..=400.0).contains(&value) {
                                    return Err(err("bpm must be 20-400".into()));
                                }
                                tempo_lane.push((track_beat, AutoToken::Set(value)));
                                *current = Some(value);
                            }
                            AutoToken::Ramp { to, dur, shape } => {
                                if current.is_none() {
                                    return Err(err(
                                        "first bpm token must be a plain value".into(),
                                    ));
                                }
                                if !(20.0..=400.0).contains(&to) {
                                    return Err(err("bpm must be 20-400".into()));
                                }
                                tempo_lane.push((track_beat, AutoToken::Ramp { to, dur, shape }));
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

    // wav_at: the vox recording (and its pitch curve) starts playing at
    // the first vox note-on — an implicit anchor that once let a removed
    // chord silently shift the whole vocal 4.8 s against every other
    // clock. When declared, the anchor is enforced.
    if let Some(at) = vox_wav_at {
        if vox_wav.is_none() && vox_pitch.is_none() {
            return Err("wav_at= needs a wav= or pitch= vox track".into());
        }
        let first = events
            .iter()
            .filter(|(_, _, k)| {
                matches!(k, EventKind::NoteOn { channel, .. }
                    if *channel == crate::vox::VOX_CHANNEL)
            })
            .map(|(b, _, _)| *b)
            .fold(f64::INFINITY, f64::min);
        if !first.is_finite() {
            return Err(format!("wav_at={}: the song has no vox notes to start the wav", at));
        }
        if (first - at).abs() > 1e-6 {
            return Err(format!(
                "wav_at={}: the vox wav starts at the FIRST vox note-on, which is at beat {} — \
                 move the note or the anchor so the modulator clock stays explicit",
                at, first
            ));
        }
    }

    events.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });

    let max_beat = events.iter().map(|e| e.0).fold(0.0f64, f64::max);
    let time_of = tempo_map(bpm, &tempo_lane, max_beat);
    Ok(Song {
        events: events
            .into_iter()
            .map(|(beats, _, kind)| SongEvent { time: time_of(beats), kind })
            .collect(),
        channels,
        vox_wav,
        vox_pitch,
        samplers,
        tracks: track_channels,
    })
}

/// Compile the tempo lane into beat -> seconds. The lane is sampled at
/// automation resolution and 60/bpm integrated cumulatively, so ramps
/// (ritardando, accelerando, with any shape) land sample-accurately —
/// events keep their musical positions and time stretches around them.
fn tempo_map(base_bpm: f64, lane: &[(f64, AutoToken)], max_beat: f64) -> impl Fn(f64) -> f64 {
    let res = AUTOMATION_STEPS_PER_BEAT;
    let n = ((max_beat + 8.0) * res).ceil() as usize + 2;
    let mut bpm_at = vec![base_bpm as f32; n];
    for &(beat, ref tok) in lane {
        let i0 = ((beat * res).round() as usize).min(n - 1);
        match *tok {
            AutoToken::Set(v) => {
                for x in &mut bpm_at[i0..] {
                    *x = v;
                }
            }
            AutoToken::Ramp { to, dur, shape } => {
                let i1 = (((beat + dur) * res).round() as usize).clamp(i0 + 1, n);
                let from = bpm_at[i0];
                for k in i0..i1 {
                    let t = (k - i0) as f32 / (i1 - i0) as f32;
                    bpm_at[k] = shape.interpolate(from, to, t);
                }
                for x in &mut bpm_at[i1.min(n)..] {
                    *x = to;
                }
            }
            AutoToken::Hold(_) => {}
        }
    }
    let mut time = vec![0.0f64; n];
    for i in 1..n {
        let mid = 0.5 * (bpm_at[i - 1] + bpm_at[i]) as f64;
        time[i] = time[i - 1] + (60.0 / mid) / res;
    }
    move |beat: f64| {
        let x = (beat * res).max(0.0);
        let i = (x as usize).min(n - 2);
        let frac = x - i as f64;
        time[i] + (time[i + 1] - time[i]) * frac
    }
}

/// One tape-deck track option. Times are seconds on the source recording;
/// `root=` takes a note name, `loop=a:b` a pair of times.
fn parse_sampler_option(
    cfg: &mut crate::sampler::SlotConfig,
    opt: &str,
    mode_set: &mut bool,
) -> Result<(), String> {
    use crate::sampler::PlayMode;
    let secs = |v: &str, what: &str| -> Result<f32, String> {
        v.parse::<f32>().map_err(|_| format!("invalid {} '{}'", what, v))
    };
    if let Some(v) = opt.strip_prefix("root=") {
        cfg.root = parse_note(v)?;
    } else if let Some(v) = opt.strip_prefix("start=") {
        cfg.start = secs(v, "start")?.max(0.0);
    } else if let Some(v) = opt.strip_prefix("end=") {
        cfg.end = secs(v, "end")?;
    } else if opt == "loop" {
        // Loop the whole region; note_on clamps the sentinel to the tape
        cfg.loop_pts = Some((0.0, f32::MAX));
    } else if let Some(v) = opt.strip_prefix("loop=") {
        let (a, b) = v
            .split_once(':')
            .ok_or_else(|| format!("loop needs two times, e.g. loop=0.1:0.9, got '{}'", v))?;
        let (a, b) = (secs(a, "loop start")?, secs(b, "loop end")?);
        if b <= a {
            return Err(format!("loop end must be after loop start ('{}')", v));
        }
        cfg.loop_pts = Some((a, b));
    } else if let Some(v) = opt.strip_prefix("xfade=") {
        cfg.xfade = secs(v, "xfade")?.clamp(0.0, 2.0);
    } else if let Some(v) = opt.strip_prefix("chop=") {
        let n: usize = v.parse().map_err(|_| format!("invalid chop count '{}'", v))?;
        if !(2..=128).contains(&n) {
            return Err("chop count must be 2-128".into());
        }
        cfg.chop = n;
    } else if let Some(v) = opt.strip_prefix("mode=") {
        cfg.mode = match v {
            "gate" => PlayMode::Gate,
            "oneshot" => PlayMode::OneShot,
            _ => return Err(format!("mode must be gate or oneshot, got '{}'", v)),
        };
        *mode_set = true;
    } else if opt == "reverse" {
        cfg.reverse = true;
    } else if opt == "fixed" {
        cfg.keytrack = false;
    } else if opt == "mono" || opt == "choke" {
        cfg.mono = true;
    } else if let Some(v) = opt.strip_prefix("gain=") {
        cfg.gain = secs(v, "gain")?.clamp(0.0, 2.0);
    } else if let Some(v) = opt.strip_prefix("pan=") {
        cfg.pan = secs(v, "pan")?.clamp(-1.0, 1.0);
    } else if let Some(v) = opt.strip_prefix("pitch=") {
        cfg.pitch_semis = secs(v, "pitch")?.clamp(-48.0, 48.0);
    } else if let Some(v) = opt.strip_prefix("attack=") {
        cfg.attack = secs(v, "attack")?.clamp(0.001, 4.0);
    } else if let Some(v) = opt.strip_prefix("release=") {
        cfg.release = secs(v, "release")?.clamp(0.003, 8.0);
    } else if let Some(v) = opt.strip_prefix("vel_amt=") {
        cfg.vel_amt = secs(v, "vel_amt")?.clamp(0.0, 1.0);
    } else if let Some(v) = opt.strip_prefix("cutoff=") {
        cfg.cutoff = secs(v, "cutoff")?.clamp(60.0, 20000.0);
    } else if let Some(v) = opt.strip_prefix("res=") {
        cfg.res = secs(v, "res")?.clamp(0.0, 1.0);
    } else {
        return Err(format!("unknown track option '{}'", opt));
    }
    Ok(())
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

/// Resolve a `>` seek target: a plain beat number, a section name (its
/// start), or `NAME.end` (its end).
fn resolve_seek(
    spec: &str,
    sections: &std::collections::HashMap<String, (f64, f64)>,
) -> Result<f64, String> {
    if let Ok(beat) = spec.parse::<f64>() {
        return Ok(beat);
    }
    let (name, end) = match spec.strip_suffix(".end") {
        Some(n) => (n, true),
        None => (spec, false),
    };
    sections
        .get(name)
        .map(|&(a, b)| if end { b } else { a })
        .ok_or_else(|| format!("invalid seek '>{}' (not a beat or a defined section)", spec))
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

/// Expand `( tokens )xN` repeat groups textually, innermost first, so
/// loops read as loops instead of walls of copy-paste. `(...)` with no
/// suffix is a plain grouping. Nesting works: `((C4)x2 D4)x2`.
fn expand_groups(line: &str) -> Result<String, String> {
    let mut s = line.to_string();
    let mut guard = 0;
    while let Some(open) = s.rfind('(') {
        guard += 1;
        if guard > 64 {
            return Err("too many nested/expanded groups".into());
        }
        let close = s[open..]
            .find(')')
            .ok_or_else(|| "unmatched '('".to_string())?
            + open;
        let content = s[open + 1..close].trim().to_string();
        let rest = &s[close + 1..];
        let (n, rest_start) = if let Some(r) = rest.strip_prefix('x') {
            let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                return Err("repeat group needs a count, e.g. (C4 E4)x4".into());
            }
            let n: usize = digits.parse().map_err(|_| "bad repeat count".to_string())?;
            if n == 0 || n > 256 {
                return Err("repeat count must be 1-256".into());
            }
            (n, close + 1 + 1 + digits.len())
        } else {
            (1, close + 1)
        };
        let expanded = vec![content; n].join(" ");
        s = format!("{} {} {}", &s[..open], expanded, &s[rest_start..]);
    }
    Ok(s)
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
) -> Result<(Vec<u8>, f64, f32, f64), String> {
    let mut s = token;
    let mut vel = default_vel;
    let mut dur = default_len;
    // `~+0.02` / `~-0.01`: push or drag this hit by beats without moving
    // the grid — feel, at last, without absolute-seek gymnastics
    let mut shift = 0.0f64;
    if let Some(i) = s.rfind('~') {
        shift = s[i + 1..]
            .parse::<f64>()
            .map_err(|_| "invalid timing shift".to_string())?;
        if shift.abs() > 0.5 {
            return Err("timing shift must be within +/-0.5 beats".into());
        }
        s = &s[..i];
    }

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

    Ok((notes, dur, vel.clamp(0.0, 1.0), shift))
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
        let (notes, dur, vel, _) = parse_note_token("C4:2@0.7", 0.8, 1.0, false).unwrap();
        assert_eq!(notes, vec![60]);
        assert_eq!(dur, 2.0);
        assert_eq!(vel, 0.7);

        let (notes, dur, _, _) = parse_note_token("[C4 E4 G4]:0.5", 0.8, 1.0, false).unwrap();
        assert_eq!(notes, vec![60, 64, 67]);
        assert_eq!(dur, 0.5);

        let (notes, dur, _, _) = parse_note_token("R:4", 0.8, 1.0, false).unwrap();
        assert!(notes.is_empty());
        assert_eq!(dur, 4.0);

        // default duration comes from the track's len option
        let (_, dur, _, _) = parse_note_token("C4", 0.8, 0.5, false).unwrap();
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

    /// Vox tracks: `vox` routes notes to the voice channel, `=lyric`
    /// suffixes emit Lyric events just before their note-ons, and the
    /// whole path — DSL to voice box to vocoder to bus — makes sound.
    #[test]
    fn vox_tracks_sing() {
        use crate::vox::{Phoneme, VOX_CHANNEL};
        let song = parse_song(
            "bpm 120\n\
             track choir vox vel=0.9\n\
             [A2 E3]:2=HH-EH R:1 A2:1=S-IH-NG-Z:200@0.8\n",
        )
        .unwrap();
        let lyrics: Vec<_> = song
            .events
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::Lyric { syl, channel } => Some((e.time, &syl.phones, *channel)),
                _ => None,
            })
            .collect();
        assert_eq!(lyrics.len(), 2);
        assert_eq!(lyrics[0].2, VOX_CHANNEL);
        assert_eq!(lyrics[0].1[0].ph, Phoneme::HH);
        assert_eq!(lyrics[0].1[1].ph, Phoneme::EH);
        // per-phoneme overrides survive the trip
        let z = lyrics[1].1.last().unwrap();
        assert_eq!(z.ph, Phoneme::Z);
        assert_eq!(z.ms, Some(200.0));
        assert!((z.amp - 0.8).abs() < 1e-6);
        // the lyric lands with (just before) its note-on
        let first_on = song
            .events
            .iter()
            .find(|e| matches!(e.kind, EventKind::NoteOn { .. }))
            .unwrap();
        assert_eq!(lyrics[0].0, first_on.time);
        assert!(matches!(first_on.kind, EventKind::NoteOn { channel, .. } if channel == VOX_CHANNEL));

        // lyric grammar errors
        assert!(parse_song("bpm 120\ntrack a\nC4=AA\n").is_err(), "lyrics need a vox track");
        assert!(parse_song("bpm 120\ntrack a vox\nR:2=AA\n").is_err(), "no lyric on a rest");
        assert!(parse_song("bpm 120\ntrack a vox\nC4=QX\n").is_err(), "unknown phoneme");
    }

    /// The full render path: a vox chord singing a vowel must be audible
    /// through the offline bounce (voice -> vocoder -> bus -> effects).
    #[test]
    fn vox_song_renders_audibly() {
        let song = parse_song(
            "bpm 120\n\
             track choir vox\n\
             [A2 E3 A3]:6=AA\n",
        )
        .unwrap();
        let frames = render_offline(&song, 48000.0);
        let peak = frames
            .iter()
            .fold(0.0f32, |a, &(l, r)| a.max(l.abs()).max(r.abs()));
        assert!(peak > 0.05, "the choir should be audible, peak={peak}");
        assert!(frames.iter().all(|&(l, r)| l.is_finite() && r.is_finite()));
    }

    /// Write a small PCM16 mono WAV to the temp dir for sampler tests.
    fn write_test_wav(name: &str) -> String {
        let rate = 22050u32;
        let n = rate as usize / 2; // half a second of 440 Hz
        let mut data = Vec::with_capacity(44 + n * 2);
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&(36 + n as u32 * 2).to_le_bytes());
        data.extend_from_slice(b"WAVEfmt ");
        data.extend_from_slice(&16u32.to_le_bytes());
        for v in [1u16, 1] {
            data.extend_from_slice(&v.to_le_bytes()); // PCM, mono
        }
        data.extend_from_slice(&rate.to_le_bytes());
        data.extend_from_slice(&(rate * 2).to_le_bytes());
        for v in [2u16, 16] {
            data.extend_from_slice(&v.to_le_bytes()); // block align, bits
        }
        data.extend_from_slice(b"data");
        data.extend_from_slice(&(n as u32 * 2).to_le_bytes());
        let w = std::f32::consts::TAU * 440.0 / rate as f32;
        for i in 0..n {
            let s = ((i as f32 * w).sin() * 0.5 * 32767.0) as i16;
            data.extend_from_slice(&s.to_le_bytes());
        }
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, data).unwrap();
        path.to_string_lossy().into_owned()
    }

    /// A `sample=` track becomes a sampler slot on its own channel, its
    /// notes route there, `automate <track>.smp_*` follows, and the whole
    /// path — DSL to tape deck to bus — makes sound.
    #[test]
    fn sampler_tracks_play_the_tape_deck() {
        use crate::sampler::SAMPLER_CHANNEL_BASE;
        let wav = write_test_wav("patina-sampler-test.wav");
        let song = parse_song(&format!(
            "bpm 120\n\
             track keys sample={wav} root=C4 loop xfade=0.02 attack=0.01 release=0.3\n\
             C4:2 G4:2\n\
             automate keys.smp_pitch\n\
             0 -12:2@lin\n"
        ))
        .unwrap();
        assert_eq!(song.samplers.len(), 1);
        assert_eq!(song.samplers[0].cfg.root, 60);
        assert!(song.samplers[0].cfg.loop_pts.is_some());

        let ons: Vec<u16> = song
            .events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::NoteOn { channel, .. } => Some(channel),
                _ => None,
            })
            .collect();
        assert_eq!(ons, vec![SAMPLER_CHANNEL_BASE, SAMPLER_CHANNEL_BASE]);
        assert!(song.events.iter().any(|e| matches!(
            e.kind,
            EventKind::Param { param: Param::SmpPitch, channel, .. }
                if channel == SAMPLER_CHANNEL_BASE
        )));

        let frames = render_offline(&song, 48000.0);
        let peak = frames
            .iter()
            .fold(0.0f32, |a, &(l, r)| a.max(l.abs()).max(r.abs()));
        assert!(peak > 0.05, "the tape deck should be audible, peak={peak}");
        assert!(frames.iter().all(|&(l, r)| l.is_finite() && r.is_finite()));

        // Vintage converters, tempo-fit, and the slot filter parse into
        // the slot config; beats= solves the varispeed for the tempo
        let song = parse_song(&format!(
            "bpm 120\n\
             track chop sample={wav} chop=4 bits=12 rate=26040 cutoff=1200 res=0.4 beats=1\n\
             C4:1\n"
        ))
        .unwrap();
        let cfg = &song.samplers[0].cfg;
        assert!(cfg.zoh, "bits=/rate= should enable the ZOH DAC");
        assert_eq!(song.samplers[0].data.rate, 26040);
        assert!((cfg.cutoff - 1200.0).abs() < 1e-3);
        assert!((cfg.res - 0.4).abs() < 1e-3);
        // reel is 0.5 s; 1 beat at 120 bpm is 0.5 s -> unity speed
        assert!((cfg.speed - 1.0).abs() < 0.02, "beats fit speed {}", cfg.speed);

        // grammar errors
        assert!(
            parse_song("bpm 120\ntrack a root=C3\nC4\n").is_err(),
            "sampler options need sample="
        );
        assert!(
            parse_song(&format!("bpm 120\ntrack a sample={wav} bits=2\nC4\n")).is_err(),
            "bits out of range"
        );
        assert!(
            parse_song(&format!("bpm 120\ntrack a sample={wav} loop=0.5:0.1\nC4\n")).is_err(),
            "backwards loop points"
        );
        assert!(
            parse_song(&format!("bpm 120\ntrack a sample={wav} mode=maybe\nC4\n")).is_err(),
            "unknown play mode"
        );
    }

    #[test]
    fn bundled_songs_parse() {
        for text in [
            include_str!("../songs/vox-humana.song"),
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

    /// `( ... )xN` groups expand to N repetitions; `>B` seeks the track
    /// cursor to absolute beat B, in both note and automation tracks.
    #[test]
    fn repeat_groups_and_seek_tokens() {
        let song = parse_song("bpm 120\ntrack a\n(C4:1)x4\n>10 E4:1\n").unwrap();
        let ons: Vec<(f64, u8)> = song
            .events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::NoteOn { note, .. } => Some((e.time, note)),
                _ => None,
            })
            .collect();
        assert_eq!(ons.len(), 5);
        assert_eq!(ons[0], (0.0, 60));
        assert_eq!(ons[3], (1.5, 60)); // beat 3 at 120 bpm
        assert_eq!(ons[4], (5.0, 64)); // sought to beat 10
        // nesting expands innermost-first
        let song = parse_song("bpm 120\ntrack a\n((C4:1)x2 D4:1)x2\n").unwrap();
        let pitches: Vec<u8> = song
            .events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::NoteOn { note, .. } => Some(note),
                _ => None,
            })
            .collect();
        assert_eq!(pitches, vec![60, 60, 62, 60, 60, 62]);
        // seek works in automation too
        let song = parse_song("bpm 60\ntrack a\nC4:20\nautomate cutoff\n400 >16 800:2\n").unwrap();
        let last = song
            .events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::Param { param: Param::Cutoff, value, .. } => Some((e.time, value)),
                _ => None,
            })
            .last()
            .unwrap();
        assert_eq!(last.0, 18.0); // ramp ends at beat 18 = 18 s at 60 bpm
        assert!((last.1 - 800.0).abs() < 0.5);
        assert!(parse_song("track a\n(C4:1\n").is_err());
    }

    /// Channel patches build on the NEUTRAL base — nothing inherited from
    /// the live panel's musical defaults — and `waveform` acts as the
    /// same all-three-oscillators macro it is everywhere else.
    #[test]
    fn patch_base_is_neutral_and_waveform_is_a_macro() {
        let p = params_from_patch("waveform 0\n").unwrap();
        assert_eq!(p.osc2_level, 0.0, "no inherited second oscillator");
        assert_eq!(p.osc3_level, 0.0);
        assert_eq!(p.detune, 0.0);
        assert_eq!(p.sub, 0.0);
        assert_eq!(p.saturation, 0.0, "no inherited saturation stage");
        assert!(matches!(p.waveform, Waveform::Sine));
        assert!(matches!(p.osc2_wave, Waveform::Sine), "macro sets all three");
        assert!(matches!(p.osc3_wave, Waveform::Sine));
        // per-osc override after the macro still wins
        let p = params_from_patch("waveform 0\nosc2_wave 2\n").unwrap();
        assert!(matches!(p.osc2_wave, Waveform::Sawtooth));
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
            dispatch(&mut vm, &e.kind);
        }
        assert!(
            (vm.params.bd_drive - 0.9).abs() < 1e-3,
            "bd_drive automation should land on the shared panel, got {}",
            vm.params.bd_drive
        );
    }

    /// Write a WAV fixture (PCM16 or float32, mono) to the temp dir.
    fn write_fixture_wav(name: &str, rate: u32, float32: bool, samples: &[f32]) -> String {
        let bytes_per: usize = if float32 { 4 } else { 2 };
        let n = samples.len();
        let mut d = Vec::with_capacity(44 + n * bytes_per);
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&(36 + (n * bytes_per) as u32).to_le_bytes());
        d.extend_from_slice(b"WAVE");
        d.extend_from_slice(b"fmt ");
        d.extend_from_slice(&16u32.to_le_bytes());
        d.extend_from_slice(&(if float32 { 3u16 } else { 1u16 }).to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&rate.to_le_bytes());
        d.extend_from_slice(&(rate * bytes_per as u32).to_le_bytes());
        d.extend_from_slice(&(bytes_per as u16).to_le_bytes());
        d.extend_from_slice(&(if float32 { 32u16 } else { 16u16 }).to_le_bytes());
        d.extend_from_slice(b"data");
        d.extend_from_slice(&((n * bytes_per) as u32).to_le_bytes());
        for &s in samples {
            if float32 {
                d.extend_from_slice(&s.to_le_bytes());
            } else {
                d.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
            }
        }
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, d).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn frames_rms(frames: &[(f32, f32)], t0: f64, t1: f64, sr: f64) -> f32 {
        let a = (t0 * sr) as usize;
        let b = ((t1 * sr) as usize).min(frames.len());
        let ms = frames[a..b]
            .iter()
            .map(|&(l, r)| {
                let m = (l + r) * 0.5;
                m * m
            })
            .sum::<f32>()
            / (b - a).max(1) as f32;
        ms.sqrt()
    }

    /// `wav_at=` makes the vox recording's clock explicit: the anchor
    /// must MATCH the first vox note-on at parse time, and in a render
    /// the recording's first sample lands at exactly that beat.
    #[test]
    fn wav_at_anchors_the_vox_clock() {
        // A modulator with energy from its very first sample
        let sq: Vec<f32> = (0..48000)
            .map(|i| if (i as f32 * 200.0 / 48000.0) % 1.0 < 0.5 { 0.8 } else { -0.8 })
            .collect();
        let wav = write_fixture_wav("patina-wavat.wav", 48000, false, &sq);

        let song = parse_song(&format!(
            "bpm 120\ntrack v vox wav={wav} wav_at=4\n>4 A2:4\n"
        ))
        .unwrap();
        let first_on = song
            .events
            .iter()
            .find(|e| matches!(e.kind, EventKind::NoteOn { .. }))
            .unwrap();
        assert_eq!(first_on.time, 2.0, "beat 4 at 120 bpm");

        // The render: silence until the anchored beat, the recording
        // articulating the carrier right on it
        let frames = render_offline(&song, 48000.0);
        let before = frames_rms(&frames, 0.0, 1.9, 48000.0);
        let after = frames_rms(&frames, 2.0, 2.3, 48000.0);
        assert!(after > 0.01, "the recording should speak at its beat, rms={after}");
        assert!(
            before < 0.1 * after,
            "nothing may sound before the anchor: before={before}, after={after}"
        );

        // A moved first note is exactly the 4.8-second-silent-shift bug:
        // it must fail loudly at parse time, naming both beats
        let e = parse_song(&format!(
            "bpm 120\ntrack v vox wav={wav} wav_at=4\n>3 A2:4\n"
        ))
        .err()
        .unwrap();
        assert!(e.contains("wav_at=4") && e.contains("beat 3"), "{e}");
        // An anchor with no recording is meaningless
        assert!(parse_song("bpm 120\ntrack v vox wav_at=4\n>4 A2:1=AA\n")
            .err()
            .unwrap()
            .contains("wav_at"));
    }

    /// `pitch=` curves carry MIDI note numbers, so they must be float32:
    /// a PCM16 curve would decode normalized to ±1 and silently transpose
    /// the melody to nonsense.
    #[test]
    fn pitch_curves_must_be_float32() {
        let sq: Vec<f32> = (0..24000)
            .map(|i| if (i as f32 * 200.0 / 48000.0) % 1.0 < 0.5 { 0.8 } else { -0.8 })
            .collect();
        let modwav = write_fixture_wav("patina-pitch-mod.wav", 48000, false, &sq);
        let curve = vec![62.0f32; 24000];
        let f32wav = write_fixture_wav("patina-pitch-f32.wav", 48000, true, &curve);
        let pcmwav = write_fixture_wav("patina-pitch-pcm.wav", 48000, false, &curve);

        let song = parse_song(&format!(
            "bpm 120\ntrack v vox wav={modwav} pitch={f32wav} wav_at=0\nA2:2\n"
        ))
        .unwrap();
        let (samples, _) = song.vox_pitch.as_ref().unwrap();
        assert!((samples[100] - 62.0).abs() < 1e-3, "values pass through unnormalized");

        let e = parse_song(&format!(
            "bpm 120\ntrack v vox wav={modwav} pitch={pcmwav}\nA2:2\n"
        ))
        .err()
        .unwrap();
        assert!(e.contains("float32"), "{e}");
    }

    /// gate carves "a small separation": proportional on short notes,
    /// capped at 80 ms on long ones — gate 0.9 on a 64-beat pad must not
    /// cut 6.4 beats of held keys.
    #[test]
    fn gate_gap_is_proportional_short_and_capped_long() {
        let song = parse_song("bpm 120\ngate 0.9\ntrack a\nC4:64 C4:1\n").unwrap();
        let offs: Vec<f64> = song
            .events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::NoteOff { .. } => Some(e.time),
                _ => None,
            })
            .collect();
        // long note: the gap is 80 ms (0.16 beats at 120), not 6.4 beats
        assert!(
            (offs[0] - (64.0 - 0.16) * 0.5).abs() < 1e-6,
            "long-note gap must cap at 80 ms, off at {}",
            offs[0]
        );
        // short note: the proportional 0.1-beat gap is under the cap
        assert!(
            (offs[1] - (65.0 - 0.1) * 0.5).abs() < 1e-6,
            "short-note gap stays proportional, off at {}",
            offs[1]
        );
    }

    /// Sections make the beat arithmetic the parser's job: `>NAME` and
    /// `>NAME.end` seek any track, and one-line `during` automation
    /// brackets sections with set/restore pairs.
    #[test]
    fn sections_name_the_arithmetic() {
        let song = parse_song(
            "bpm 120\n\
             section A 8..12\n\
             track t\n\
             >A C4:1 >A.end D4:1\n\
             automate vox_mode: 1 during A base 0\n",
        )
        .unwrap();
        let ons: Vec<(f64, u8)> = song
            .events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::NoteOn { note, .. } => Some((e.time, note)),
                _ => None,
            })
            .collect();
        assert_eq!(ons, vec![(4.0, 60), (6.0, 62)], "section seeks land on its edges");
        let modes: Vec<(f64, f32)> = song
            .events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::Param { param: Param::VoxModeSel, value, .. } => {
                    Some((e.time, value))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            modes,
            vec![(0.0, 0.0), (4.0, 1.0), (6.0, 0.0)],
            "during = base at 0, value at start, base restored at end"
        );

        // grammar errors: backwards range, unknown section in seek and during
        assert!(parse_song("bpm 120\nsection A 8..4\ntrack t\nC4\n").is_err());
        assert!(parse_song("bpm 120\ntrack t\n>NOPE C4\n").is_err());
        assert!(parse_song("bpm 120\ntrack t\nC4\nautomate cutoff: 1 during NOPE\n").is_err());
        assert!(parse_song("bpm 120\ntrack t\nC4\nautomate bpm: 1 during A\n").is_err());
    }

    /// Read a parameter's canonical engine value back, for the sweep
    /// test below. None = the value lives outside ParamValues (mixer
    /// strips, sampler slots, performance controllers, enums — those are
    /// asserted separately or by their own tests).
    #[rustfmt::skip]
    fn readback(vm: &VoiceManager, p: Param) -> Option<f32> {
        let v = &vm.params;
        Some(match p {
            Param::Volume => v.volume,
            Param::Detune => v.detune,
            Param::Cutoff => v.cutoff,
            Param::Resonance => v.resonance,
            Param::Drive => v.drive,
            Param::Saturation => v.saturation,
            Param::HpfCutoff => v.hpf_cutoff,
            Param::FuzzAmount => v.fuzz,
            Param::NoiseLevel => v.noise,
            Param::SpringWet => v.spring,
            Param::Glide => v.glide,
            Param::SubLevel => v.sub,
            Param::Osc2Pitch => v.osc2_pitch,
            Param::Osc2Level => v.osc2_level,
            Param::Osc3Pitch => v.osc3_pitch,
            Param::Osc3Level => v.osc3_level,
            Param::KeyTrack => v.key_track,
            Param::OscFm => v.osc_fm,
            Param::RingAmount => v.ring,
            Param::PulseWidth => v.pulse_width,
            Param::MixSaw => v.mix_saw,
            Param::MixPulse => v.mix_pulse,
            Param::MixTri => v.mix_tri,
            Param::MixSine => v.mix_sine,
            Param::UiOctave => v.ui_octave,
            Param::LfoRate => v.lfo_rate,
            Param::LfoShape => v.lfo_shape,
            Param::LfoPitch => v.lfo_pitch,
            Param::LfoFilter => v.lfo_filter,
            Param::LfoPwm => v.lfo_pwm,
            Param::Attack => v.attack,
            Param::Decay => v.decay,
            Param::Sustain => v.sustain,
            Param::Release => v.release,
            Param::FilterEnvAmount => v.filter_env_amount,
            Param::FilterAttack => v.filter_attack,
            Param::FilterDecay => v.filter_decay,
            Param::FilterSustain => v.filter_sustain,
            Param::FilterRelease => v.filter_release,
            Param::ReverbDecay => v.reverb_decay,
            Param::ReverbWet => v.reverb_wet,
            Param::ChorusRate => v.chorus_rate,
            Param::ChorusDepth => v.chorus_depth,
            Param::TapeWow => v.tape_wow,
            Param::TapeFlutter => v.tape_flutter,
            Param::TapeDrive => v.tape_drive,
            Param::TapeAge => v.tape_age,
            Param::BdLevel => v.bd_level,
            Param::BdTune => v.bd_tune,
            Param::BdAttack => v.bd_attack,
            Param::BdDecay => v.bd_decay,
            Param::BdSweep => v.bd_sweep,
            Param::BdDrive => v.bd_drive,
            Param::SdLevel => v.sd_level,
            Param::SdTune => v.sd_tune,
            Param::SdTone => v.sd_tone,
            Param::SdSnappy => v.sd_snappy,
            Param::SdDecay => v.sd_decay,
            Param::RsLevel => v.rs_level,
            Param::RsTune => v.rs_tune,
            Param::CpLevel => v.cp_level,
            Param::CpDecay => v.cp_decay,
            Param::HhLevel => v.hh_level,
            Param::HhTune => v.hh_tune,
            Param::HhMetal => v.hh_metal,
            Param::ChDecay => v.ch_decay,
            Param::OhDecay => v.oh_decay,
            Param::DrumDrive => v.dr_drive,
            Param::VoxLevel => v.vox_level,
            Param::VoxDry => v.vox_dry,
            Param::VoxBreath => v.vox_breath,
            Param::VoxClarity => v.vox_clarity,
            Param::VoxVibrato => v.vox_vibrato,
            Param::VoxModeSel => v.vox_mode,
            Param::VoxIntonation => v.vox_intonation,
            _ => return None,
        })
    }

    /// The "no silently clamped params" sweep: every parameter in THE
    /// table, driven to both documented extremes, must actually land in
    /// the engine. This is the test that would have caught the historic
    /// vox_mode clamp (stuck at 1.0 while the range said 3.0, silently
    /// rerouting circuits 2 and 3 for weeks) and the vox_level 0..2
    /// extension needing edits in three places.
    #[test]
    fn no_silently_clamped_params() {
        let mut vm = VoiceManager::new(48000.0, 4);
        for def in PARAM_DEFS {
            let (lo, hi, _) = def.param.range();
            for target in [lo, hi] {
                def.param.apply(&mut vm, target);
                if let Some(got) = readback(&vm, def.param) {
                    assert!(
                        (got - target).abs() < 1e-4,
                        "param '{}' set to {} but the engine recorded {} — a stale clamp?",
                        def.name,
                        target,
                        got
                    );
                }
            }
        }
        // The enum-valued selectors, by hand: their range tops must
        // reach the last variant
        Param::WaveformSel.apply(&mut vm, 3.0);
        assert!(matches!(vm.params.waveform, Waveform::Triangle));
        Param::ChorusModeSel.apply(&mut vm, 4.0);
        assert!(matches!(vm.params.chorus_mode, ChorusMode::IV));
        Param::CircuitSel.apply(&mut vm, 1.0);
        assert!(matches!(vm.params.circuit, crate::oscillator::CircuitModel::Arp));
        Param::SyncSel.apply(&mut vm, 1.0);
        assert!(vm.params.sync);
        // ...and every table name must round-trip through the parser
        for def in PARAM_DEFS {
            assert_eq!(Param::from_name(def.name), Some(def.param), "{}", def.name);
            assert_eq!(def.param.name(), def.name);
            if let Some(cc) = def.cc {
                assert_eq!(Param::from_cc(cc), Some(def.param), "cc {}", cc);
            }
        }
    }

    /// The render-side word-audibility check, as a Rust test: every
    /// sung syllable must be audible in the offline bounce, standing
    /// clear of the gaps between notes.
    #[test]
    fn every_syllable_lands_audibly() {
        let song = parse_song(
            "bpm 120\n\
             track v vox\n\
             A2:1=S-AH R:1 A2:1=B-AA R:1 A2:1=M-IY\n\
             automate reverb_wet\n\
             0\n",
        )
        .unwrap();
        let frames = render_offline(&song, 48000.0);
        for k in 0..3 {
            let on = k as f64; // notes at beats 0, 2, 4 -> 0, 1, 2 s
            let sung = frames_rms(&frames, on + 0.05, on + 0.4, 48000.0);
            let gap = frames_rms(&frames, on + 0.7, on + 0.95, 48000.0);
            // Absolute floor sized to the engine's un-normalized gain
            // staging (a dark vowel like M-IY sits near 0.008 rms)
            assert!(sung > 0.004, "syllable {k} must be audible, rms={sung}");
            assert!(
                sung > 3.0 * gap,
                "syllable {k} must stand clear of the gap: {sung} vs {gap}"
            );
        }
    }
}
