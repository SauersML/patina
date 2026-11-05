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
//   tail 0.25                # seconds rendered after the final event;
//                            # short for an abrupt ending, long for reverb.
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
//                                # channel at bar one (every one is also a
//                                # lane: `automate lead.gain`):
//                                # gain= (linear 0..4: 0 mutes, 2 = +6 dB,
//                                # 4 = +12 dB) pan= (-1..1)
//                                # reverb_send= spring_send= chorus_send=
//                                # echo_send= (0..1, into the shared tanks at unity)
//                                # duck= (sidechain depth 0..1: 0.5 = -6 dB,
//                                # 1 = silence at the dip), keyed by every
//                                # kick, or by `duck_from=TRACK` (that
//                                # track's notes) / `duck_from=TRACK.BD`
//                                # (one drum voice, or one note, of it);
//                                # duck_attack= duck_hold= (seconds; 0 =
//                                # snap / no hold) and duck_release= (the
//                                # time constant back to full).
//                                # shape=pump|saw|ramp|sine|square|tri or
//                                # shape=x.x.xx.x (a trance gate: x open,
//                                # . shut, 0-9 ninths, one 16th a step):
//                                # a volume shaper on the song's beat;
//                                # shape_beats= (cycle length; 1 for the
//                                # curves, steps/4 for a pattern),
//                                # shape_depth= (0..1, default 1),
//                                # shape_smooth= (step edges, seconds).
//                                # Inserts, before the fader: eq_low=
//                                # eq_mid= eq_high= (dB, 0 = out) at
//                                # eq_low_freq= (100) eq_mid_freq= (1000,
//                                # Q eq_mid_q= 0.7) eq_high_freq= (8000);
//                                # dist= (0..1, 0 = out) through
//                                # dist_mode= (0 tanh, 1 hard clip, 2
//                                # foldback, 3 bitcrush, 4 sample-rate
//                                # reduce), dist_mix= (wet 0..1). The
//                                # shapers drive up to +36 dB and make up
//                                # half of it in dB: denser, peaks down,
//                                # RMS up (a -9 dBFS supersaw chord at
//                                # dist=0.5 tanh: peak -2.3 dB, RMS
//                                # +4.4 dB, crest 14.4 -> 7.7 dB); trim
//                                # with gain=. The EQ, the drive and the
//                                # compressor hear the track's summed
//                                # voices, once a sample.
//                                # comp= (strip compressor threshold dBFS,
//                                # 0 = out) and, on the strip, the bus
//                                # compressor's own knobs: comp_ratio=
//                                # comp_attack= comp_release= comp_makeup=
//                                # comp_mix= comp_sc_hpf=.
//     E5:2 D5 C5 R:4 [C4 E4 G4]:2@0.6  | A4
//
//   track beat kit=909 len=0.5   # a drum track: kit= routes it to the
//     BD SD:0.5 [BD CH] OH@0.6   # rhythm section. Tokens are drum names
//                                # (BD SD RS CP CH OH) or GM notes; velocity
//                                # is the accent bus (@1 = full accent).
//                                # Every kit=909 track is a 909 board of
//                                # its own: its knobs (`automate
//                                # beat.hh_tune`), strip and stem are its
//                                # alone. kit=<track> plays an earlier kit
//                                # track's board instead: one machine, one
//                                # strip, one stem, and a closed hat on one
//                                # track still chokes the other's open hat.
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
//                           # The throat: `psola` keeps a voice's
//                           # formants and time while notes change its
//                           # pitch. `formant` keeps the formants on the
//                           # varispeed tape instead (pitch AND time still
//                           # follow the key: an octave down is half
//                           # speed, not a monster); formant=S moves them
//                           # S semitones (-24..24) on top, formant_keep=K
//                           # (0..1) keeps that much of them. Automate
//                           # smp_formant / smp_formant_keep alike.
//
// Note-track tokens:
//   C4  F#3  Eb5  60      note names (C4 = MIDI 60) or raw MIDI numbers
//   [C4 E4 G4]            chord (notes start and stop together)
//   R  or  .              rest (`R:0`, any rest of 1e-6 beats or less,
//                         is nothing: rounding noise from a generator)
//   :2                    duration suffix, in beats (floats allowed)
//   @0.7                  velocity suffix (0..1)
//   ~+0.02  ~-0.01        microtiming, in beats: push or drag this hit
//                         off the grid (the cursor stays on it)
//   >G3  >-12  >+0.5      note bend: the note glides from its own pitch
//                         to the target over its held length, then holds
//                         it (C4:2>G3 and C4>G3:2 are the same; a note,
//                         a MIDI number, or signed semitones).
//                         [C4 E4]:2>-12 bends a whole chord;
//                         [C4>D4 G4]:2 bends one chord note. The track's
//                         bend_curve (-1..1) shapes the glide: 0 straight
//                         in pitch, + moves early, - holds and falls
//                         late. Synth tracks, vox tracks (the carrier)
//                         and sampler tracks bend; kit hits cannot.
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
//   A track prefix needs a control the track owns: its voice, its strip,
//   its tape slot, a kit track's own 909 board, the voice box on a vox
//   track. The bus circuits under every track (noise, reverb_wet, spring,
//   chorus, tape, volume) refuse one; automate them unscoped.
//
//   The first token must be a plain value (the starting point). After that,
//   V:D@shape means "ramp to V over D beats". R:D / .:D holds the current
//   value. Shapes: lin (default), exp (musical/geometric — right for
//   frequencies), log (fast start), smooth (S-curve), step (jump at the end).
//
// Automatable parameters: volume, output (post-effects arrangement trim),
// waveform (0=sine 1=square 2=saw 3=tri,
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
// vox_formant (semitones, -24..24: the voice's throat moved without its
// pitch — the formant voice's resonances scale, a wav= recording plays
// through throat-only PSOLA; -5 is a bigger mouth, +5 a smaller one),
// glide (portamento seconds, 0 = off), mono (1 = one voice per track,
// last-note priority, overlapping notes slur legato; plain sets),
// vel_amp (0..1 how far a soft strike drops the level, default 0.7),
// vel_filter (octaves the velocity swing moves the cutoff, default
// 0.8), pitch_shift (channel-local
// semitones -24..24 for independently gliding chord-voice tracks),
// sub (0..1 octave-down square),
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
// The echo (echo.rs; a clean stereo delay before the reverb, so repeats
// sit in the same room as the notes they answer): echo (0..1 insert
// level; per track use `echo_send=`), echo_time (seconds, 0.02..2.5 —
// 60/bpm is a quarter note), echo_feedback (0..0.95), echo_tone (Hz,
// the loop lowpass: each repeat a little darker), echo_pingpong (0..1,
// repeats alternate sides). A fixed 120 Hz high-pass in the loop thins
// each repeat so echoes never pile up under the music.
//
// The master peak control (limiter.rs; after the width, before the output
// trim): clip (dB of drive into a soft-knee clipper at the ceiling, 0 =
// out), limit (look-ahead brickwall limiter threshold, dBFS, -30..0, 0 =
// out: the mix comes up by -limit and is held under the ceiling),
// limit_ceiling (dBFS, -12..0, default -1), limit_release (seconds, default
// 0.08). Engaged, it replaces the engine's tanh soft limit (the ceiling is
// the bounce's true peak), and a song that sets limit < 0 or clip > 0
// anywhere runs it from the downbeat. The threshold is absolute: the
// loudest peak loses about (raw mix peak - limit - ceiling) dB.
//
// Out-of-range values are clamped to the parameter's range (`patina
// --params`) and the loader warns, one line each: "line 12: gain 3.8
// clamped to 4". So do the gate, vel=, a note's @ velocity, the sampler
// options and the values in a patch= file.
//
// The mix-bus compressor (buscomp.rs; SSL G-series bus comp, printing to
// the tape): comp_in (0 = out, 1 = in; plain sets), comp_threshold (dBFS
// peak, -48..0), comp_ratio (0 = 2:1, 1 = 4:1, 2 = 10:1), comp_attack (0..5
// = 0.1, 0.3, 1, 3, 10, 30 ms), comp_release (0..3 = 0.1, 0.3, 0.6, 1.2 s,
// 4 = AUTO) — the three switches take plain sets — comp_makeup (dB, 0..15),
// comp_mix (0..1 parallel blend, 1 = compressor alone), comp_sc_hpf
// (sidechain high-pass Hz, 20..400; 20 = the card's own coupling).
//
// Rhythm section (the 909 board; 0..1 panel knobs, except the five voice
// levels bd_ sd_ rs_ cp_ hh_level, linear gains to 4 = +12 dB over the
// panel's full 1): bd_level, bd_tune,
// bd_attack, bd_decay, bd_sweep, bd_drive, sd_level, sd_tune, sd_tone,
// sd_snappy, sd_decay, rs_level, rs_tune, cp_level, cp_decay, hh_level,
// hh_tune, hh_metal, ch_decay, oh_decay, dr_drive, dr_tone (bus lowpass:
// 1 = wire, 0 = felt blanket — the dusty-kit knob). Per kit track via
// `automate <track>.<knob>` (that track's board alone), or unscoped to
// turn every kit's board at once, as the panel does.
//
// The tape deck (per sampler track via `automate <track>.<param>`, or
// global to all slots): smp_pitch (semitones, -48..48 — four octaves of
// varispeed, enough for a tape-stop swoop), smp_start (0..1
// needle-drop point), smp_gain (0..2), smp_pan (-1..1), smp_attack,
// smp_release (seconds), smp_cutoff (Hz, 20000 = open), smp_res (0..1),
// smp_formant (semitones, -24..24: the throat moved without the pitch),
// smp_formant_keep (0..1: how much of each note's varispeed the throat
// refuses; 1 = the recording's own formants at every key and smp_pitch).
// The two need the slot's epochs, found at load for a `psola`/`formant`
// slot or any slot a smp_formant lane reaches.
//
// Kick-bass, walls and risers (voice-card knobs, so per track via
// `automate <track>.<param>` like cutoff): pitch_env (semitones, -48..96:
// every gated note STARTS this far above its own pitch and dives onto it;
// 0 = off), pitch_env_time (seconds of the dive, 0.001..4),
// pitch_env_curve (-1..1: 0 = straight in octaves, + = fast drop and a
// long settle — the kick's snap, - = slow start, fast landing). The tail
// holds the note's pitch, so the kick is also the bass, and the dive and
// the tail (decay, cutoff) shape separately. key_sync (1 = every gate
// restarts the oscillators at one phase, so every hit clicks alike).
// shaper (dB of drive, 0..48, into a waveshaper between the mixer and the
// ladder, so the ladder is its post-filter; 0 = out), shaper_mode (0
// tanh, 1 hard clip, 2 sine wavefolder). supersaw (0..1, crossfades
// oscillator 1 into seven detuned saws), supersaw_detune (0..1 spread),
// supersaw_mix (0..1, the six side saws against the centre one),
// osc1_level (0..1, oscillator 1 into the mixer). noise_osc (0..1, the
// card's own white noise ahead of the drive and ladder), noise_cutoff
// (Hz, 20..20000, its own resonant filter), noise_res (0..1), noise_mode
// (0 lowpass, 1 bandpass, 2 highpass). Switches take plain sets.
//
// Glitch inserts (glitch.rs), on one track's strip as `automate
// <track>.stutter`, or unscoped across the whole mix bus (after the tape,
// so the finished mix stutters and stops):
//   stutter N   beat repeat: from the moment it reaches N >= 1, the next
//               1/N note (8 an eighth, 16 a sixteenth, 64 a 64th; 12 an
//               eighth-note triplet) is caught and repeated until it goes
//               back to 0. Raising N while it runs shortens the repeat on
//               the caught audio, the roll into a drop:
//                 automate kick.stutter
//                   0 >12 8 >13 16 >14 32 >15 64 >16 0
//   reverse N   plays the track backwards in 1/N-note slices, one slice
//               behind (each slice is the one before it, reversed); 0 off
//   tape_stop B the tape winds down to a halt over B beats, pitch and
//               level falling together, then silence until it goes back
//               to 0, when the music returns live:
//                 automate tape_stop
//                   0 >60 4
// Lengths follow the song's tempo lane. Every seam is declicked.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::chorus::ChorusMode;
use crate::oscillator::Waveform;
use crate::voice_manager::{ParamValues, VoiceManager};

// Automation curves are sampled at this many points per beat
const AUTOMATION_STEPS_PER_BEAT: f64 = 32.0;

/// The score clock may deliberately enter audio-rate territory so a repeated
/// figure can accelerate continuously into a pitched blur. This is a song-DSL
/// limit, not the much narrower range of a conventional front-panel tempo knob.
/// 400 beats a second already puts a sixteenth-note figure at 1.6 kHz; past
/// it the tempo map's 1/32-beat grid step falls toward a single sample and
/// the clock can no longer place the notes it is asked for.
const MAX_SONG_BPM: f32 = 24_000.0;

/// The last beat a song may reach: over a day at 60 bpm, and a tempo map
/// of a few million steps.
const MAX_SONG_BEATS: f64 = 100_000.0;

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
    //
    // Envelope times run to 10 s, the Moog 911's own range (2 ms-10 s per
    // segment); a 2 s ceiling cut every slow swell and long bloom short.
    // ------------------------------------------------------------------
    Volume:          "volume",         Some(7),   (0.0, 1.0, Lin);
    Output:          "output",         None,      (0.0, 1.0, Lin);
    // Master mid/side after the tape: 0 folds the finished mix to mono,
    // 1 leaves it untouched, above 1 widens. The effects each add their
    // own decorrelation; this is the one knob that sets the image.
    Width:           "width",          None,      (0.0, 2.0, Lin);
    WaveformSel:     "waveform",       Some(113), (0.0, 3.0, Step);
    Detune:          "detune",         Some(83),  (0.0, 30.0, Lin);
    Cutoff:          "cutoff",         Some(74),  (20.0, 20000.0, Log);
    Resonance:       "resonance",      Some(71),  (0.0, 4.0, Lin);
    Drive:           "drive",          Some(103), (0.1, 5.0, Lin);
    Saturation:      "saturation",     Some(104), (0.0, 2.0, Lin);
    Attack:          "attack",         Some(73),  (0.0001, 10.0, Log);
    Decay:           "decay",          Some(75),  (0.001, 10.0, Log);
    Sustain:         "sustain",        Some(79),  (0.0, 1.0, Lin);
    Release:         "release",        Some(72),  (0.001, 10.0, Log);
    HpfCutoff:       "hpf",            Some(102), (16.0, 8000.0, Log);
    FuzzAmount:      "fuzz",           None,      (0.0, 1.0, Lin);
    NoiseLevel:      "noise",          Some(81),  (0.0, 1.0, Lin);
    SpringWet:       "spring",         Some(95),  (0.0, 1.0, Lin);
    Glide:           "glide",          Some(5),   (0.0, 2.0, Lin);
    // Mono voice mode, the SH-101's assigner: 1 = the track owns one
    // voice (or one unison stack), last-note priority, overlapped notes
    // slur without re-striking the envelopes
    MonoSel:         "mono",           None,      (0.0, 1.0, Step);
    // Channel-local pitch CV. Put chord voices on separate tracks to glide
    // each one independently to an exact destination note.
    PitchShift:      "pitch_shift",    None,      (-24.0, 24.0, Lin);
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
    FilterAttack:    "filter_attack",  Some(107), (0.001, 10.0, Log);
    FilterDecay:     "filter_decay",   Some(108), (0.01, 10.0, Log);
    FilterSustain:   "filter_sustain", Some(109), (0.0, 1.0, Lin);
    FilterRelease:   "filter_release", Some(110), (0.01, 10.0, Log);
    // Velocity sensitivity: how far a soft strike drops the VCA (0 =
    // velocity ignored), and how many octaves the full velocity swing
    // moves the cutoff
    VelAmp:          "vel_amp",        None,      (0.0, 1.0, Lin);
    VelFilter:       "vel_filter",     None,      (0.0, 4.0, Lin);
    ReverbDecay:     "reverb_decay",   None,      (0.0, 0.99, Lin);
    ReverbWet:       "reverb_wet",     Some(91),  (0.0, 1.0, Lin);
    ReverbTone:      "reverb_tone",    None,      (800.0, 12000.0, Log);
    ReverbPre:       "reverb_pre",     None,      (0.0, 0.08, Lin);
    // The echo (echo.rs): insert level, time in seconds, feedback, the
    // loop lowpass, and how far repeats cross sides
    EchoWet:         "echo",           None,      (0.0, 1.0, Lin);
    EchoTime:        "echo_time",      None,      (0.02, 2.5, Log);
    EchoFeedback:    "echo_feedback",  None,      (0.0, 0.95, Lin);
    EchoTone:        "echo_tone",      None,      (500.0, 12000.0, Log);
    EchoPingPong:    "echo_pingpong",  None,      (0.0, 1.0, Lin);
    Unison:          "unison",         None,      (1.0, 4.0, Step);
    UnisonDetune:    "unison_detune",  None,      (0.0, 40.0, Lin);
    // Stereo spread of the voice cards (0 = every note centered)
    Spread:          "spread",         None,      (0.0, 1.0, Lin);
    ChorusModeSel:   "chorus_mode",    Some(112), (0.0, 4.0, Step);
    ChorusRate:      "chorus_rate",    Some(111), (0.1, 10.0, Log);
    ChorusDepth:     "chorus_depth",   Some(93),  (0.0, 1.0, Lin);
    ChorusHiss:      "chorus_hiss",    None,      (0.0, 2.0, Lin);
    TapeWow:         "tape_wow",       Some(92),  (0.0, 1.0, Lin);
    TapeFlutter:     "tape_flutter",   Some(94),  (0.0, 1.0, Lin);
    TapeDrive:       "tape_drive",     Some(118), (0.0, 1.0, Lin);
    TapeAge:         "tape_age",       Some(119), (0.0, 1.0, Lin);
    // The mix-bus compressor (buscomp.rs): the console's centre section,
    // printing to the tape. Its three switches are stepped like the card's
    // (ratio 2/4/10:1, attack 0.1..30 ms, release 0.1..1.2 s + AUTO).
    CompIn:          "comp_in",        None,      (0.0, 1.0, Step);
    CompThreshold:   "comp_threshold", None,      (-48.0, 0.0, Lin);
    CompRatio:       "comp_ratio",     None,      (0.0, 2.0, Step);
    CompAttack:      "comp_attack",    None,      (0.0, 5.0, Step);
    CompRelease:     "comp_release",   None,      (0.0, 4.0, Step);
    CompMakeup:      "comp_makeup",    None,      (0.0, 15.0, Lin);
    CompMix:         "comp_mix",       None,      (0.0, 1.0, Lin);
    CompScHpf:       "comp_sc_hpf",    None,      (20.0, 400.0, Log);
    // The rhythm section: the 909 panel. Unscoped (the panel, MIDI CCs,
    // patches, `automate bd_tune`) a knob turns every kit's board; scoped
    // (`automate beat.bd_tune`) it turns that kit track's board alone.
    // Unitless 0..1 panel knob rotations; the circuits map them onto
    // their electrical ranges. The five voice levels are linear gains
    // reaching 4 (+12 dB over a full panel level of 1): the stock knob
    // positions (0..1) sound exactly as they did, and a hat or rim can be
    // brought up to sit against the kick
    BdLevel:         "bd_level",       Some(20),  (0.0, 4.0, Lin);
    BdTune:          "bd_tune",        Some(21),  (0.0, 1.0, Lin);
    BdAttack:        "bd_attack",      Some(22),  (0.0, 1.0, Lin);
    BdDecay:         "bd_decay",       Some(23),  (0.0, 1.0, Lin);
    BdSweep:         "bd_sweep",       Some(24),  (0.0, 1.0, Lin);
    BdDrive:         "bd_drive",       Some(25),  (0.0, 1.0, Lin);
    SdLevel:         "sd_level",       Some(26),  (0.0, 4.0, Lin);
    SdTune:          "sd_tune",        Some(27),  (0.0, 1.0, Lin);
    SdTone:          "sd_tone",        Some(28),  (0.0, 1.0, Lin);
    SdSnappy:        "sd_snappy",      Some(29),  (0.0, 1.0, Lin);
    SdDecay:         "sd_decay",       Some(30),  (0.0, 1.0, Lin);
    RsLevel:         "rs_level",       Some(31),  (0.0, 4.0, Lin);
    RsTune:          "rs_tune",        Some(52),  (0.0, 1.0, Lin);
    CpLevel:         "cp_level",       Some(53),  (0.0, 4.0, Lin);
    CpDecay:         "cp_decay",       Some(54),  (0.0, 1.0, Lin);
    HhLevel:         "hh_level",       Some(55),  (0.0, 4.0, Lin);
    HhTune:          "hh_tune",        Some(56),  (0.0, 1.0, Lin);
    HhMetal:         "hh_metal",       Some(57),  (0.0, 1.0, Lin);
    ChDecay:         "ch_decay",       Some(58),  (0.0, 1.0, Lin);
    OhDecay:         "oh_decay",       Some(59),  (0.0, 1.0, Lin);
    DrumDrive:       "dr_drive",       Some(60),  (0.0, 1.0, Lin);
    DrumTone:        "dr_tone",        Some(61),  (0.0, 1.0, Lin);
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
    SmpPitch:        "smp_pitch",      Some(9),   (-48.0, 48.0, Lin);
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
    // Linear: 0 mutes, 4 is +12 dB
    TrackGain:       "gain",           None,      (0.0, 4.0, Lin);
    TrackPan:        "pan",            None,      (-1.0, 1.0, Lin);
    ReverbSend:      "reverb_send",    None,      (0.0, 1.0, Lin);
    SpringSend:      "spring_send",    None,      (0.0, 1.0, Lin);
    ChorusSend:      "chorus_send",    None,      (0.0, 1.0, Lin);
    EchoSend:        "echo_send",      None,      (0.0, 1.0, Lin);
    DuckAmount:      "duck",           None,      (0.0, 1.0, Lin);
    DuckRelease:     "duck_release",   None,      (0.02, 2.0, Lin);
    // The duck envelope's attack (seconds to full depth; 0 = snap) and
    // hold (seconds at full depth before the release)
    DuckAttack:      "duck_attack",    None,      (0.0, 1.0, Lin);
    DuckHold:        "duck_hold",      None,      (0.0, 2.0, Lin);
    // A compressor on the track's own strip: threshold in dBFS, 0 = out.
    // The bus compressor glues the mix; this one shapes one part (a
    // drum loop) before it reaches the bus, the way a console channel does.
    TrackComp:       "comp",           None,      (-48.0, 0.0, Lin);
    // The tempo-synced volume shaper (`shape=` on the track line): how
    // far the curve pulls the strip down (0 = off, 1 = to silence), and
    // how long each step edge takes, seconds
    ShapeDepth:      "shape_depth",    None,      (0.0, 1.0, Lin);
    ShapeSmooth:     "shape_smooth",   None,      (0.0005, 0.05, Log);
    // The strip's drive stage (inserts.rs), before the fader: amount
    // (0 = out), the circuit (0 tanh, 1 hard clip, 2 foldback, 3 bitcrush,
    // 4 sample-rate reduce) and the wet/dry blend
    Dist:            "dist",           None,      (0.0, 1.0, Lin);
    DistMode:        "dist_mode",      None,      (0.0, 4.0, Step);
    DistMix:         "dist_mix",       None,      (0.0, 1.0, Lin);
    // The strip's three-band EQ, before the drive: gains in dB (0 = the
    // band is out), corners in Hz, the mid band's Q
    EqLow:           "eq_low",         None,      (-24.0, 24.0, Lin);
    EqLowFreq:       "eq_low_freq",    None,      (20.0, 1000.0, Log);
    EqMid:           "eq_mid",         None,      (-24.0, 24.0, Lin);
    EqMidFreq:       "eq_mid_freq",    None,      (100.0, 10000.0, Log);
    EqMidQ:          "eq_mid_q",       None,      (0.2, 10.0, Log);
    EqHigh:          "eq_high",        None,      (-24.0, 24.0, Lin);
    EqHighFreq:      "eq_high_freq",   None,      (1000.0, 20000.0, Log);
    // The master bus's peak control (limiter.rs), after the width and
    // before the arrangement trim: clipper drive in dB (0 = out), limiter
    // threshold in dBFS (0 = out; the mix comes up by -threshold), the
    // shared ceiling in dBFS, and the limiter's release in seconds
    Clip:            "clip",           None,      (0.0, 24.0, Lin);
    Limit:           "limit",          None,      (-30.0, 0.0, Lin);
    LimitCeiling:    "limit_ceiling",  None,      (-12.0, 0.0, Lin);
    LimitRelease:    "limit_release",  None,      (0.005, 1.0, Log);
    // Global chorus insert mix override (the mode switch re-derives it)
    ChorusMix:       "chorus_mix",     None,      (0.0, 1.0, Lin);
    // The kick-bass voice (voice.rs): a pitch envelope fired by every
    // gate — the note starts `pitch_env` semitones from its own pitch
    // and lands on it after `pitch_env_time` s along `pitch_env_curve`
    // (0 straight, + dives early, - late); the tail holds the note.
    // `key_sync` restarts the oscillators on every gate. `shaper` drives
    // a waveshaper between the mixer and the ladder (dB, 0 = out;
    // `shaper_mode` 0 tanh, 1 hard clip, 2 sine fold), so the ladder
    // is the post-filter.
    PitchEnv:        "pitch_env",      None,      (-48.0, 96.0, Lin);
    PitchEnvTime:    "pitch_env_time", None,      (0.001, 4.0, Log);
    PitchEnvCurve:   "pitch_env_curve", None,     (-1.0, 1.0, Lin);
    KeySync:         "key_sync",       None,      (0.0, 1.0, Step);
    ShaperDrive:     "shaper",         None,      (0.0, 48.0, Lin);
    ShaperModeSel:   "shaper_mode",    None,      (0.0, 2.0, Step);
    // Walls and risers (sources.rs): seven detuned saws crossfaded over
    // oscillator 1, and each card's own filtered noise (cutoff Hz,
    // resonance, mode 0 lowpass / 1 bandpass / 2 highpass)
    Supersaw:        "supersaw",       None,      (0.0, 1.0, Lin);
    // Oscillator 1's level into the mixer (osc 2 and 3 have theirs);
    // 0 leaves a card to its noise oscillator, sub or other oscillators
    Osc1Level:       "osc1_level",     None,      (0.0, 1.0, Lin);
    SupersawDetune:  "supersaw_detune", None,     (0.0, 1.0, Lin);
    SupersawMix:     "supersaw_mix",   None,      (0.0, 1.0, Lin);
    NoiseOsc:        "noise_osc",      None,      (0.0, 1.0, Lin);
    NoiseCutoff:     "noise_cutoff",   None,      (20.0, 20000.0, Log);
    NoiseRes:        "noise_res",      None,      (0.0, 1.0, Lin);
    NoiseModeSel:    "noise_mode",     None,      (0.0, 2.0, Step);
    // The shape of a note bend (`C4:2>G3`): 0 straight, + early, - late
    BendCurve:       "bend_curve",     None,      (-1.0, 1.0, Lin);
    // The throat (sampler.rs, vox.rs): formants moved without moving the
    // pitch, in semitones, on a tape-deck slot or the voice box's voice;
    // and how much of each note's varispeed a slot's throat refuses
    // (1 = the recording's own formants at every key)
    SmpFormant:      "smp_formant",    None,      (-24.0, 24.0, Lin);
    SmpFormantKeep:  "smp_formant_keep", None,    (0.0, 1.0, Lin);
    VoxFormant:      "vox_formant",    None,      (-24.0, 24.0, Lin);
    // Glitch inserts (glitch.rs): a beat repeat of 1/N notes, a reverser
    // in 1/N-note slices (below 1 = off), and a tape stop over N beats
    // (0 = running). On one track's strip as `automate <track>.stutter`;
    // unscoped, across the whole mix bus.
    Stutter:         "stutter",        None,      (0.0, 64.0, Lin);
    Reverse:         "reverse",        None,      (0.0, 64.0, Lin);
    TapeStop:        "tape_stop",      None,      (0.0, 16.0, Lin);
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
        PARAM_DEFS
            .iter()
            .find(|d| d.cc == Some(cc))
            .map(|d| d.param)
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

    /// The span a knob sweeps: a MIDI controller, a plugin's host
    /// parameter, the app's panel. The table range, except the five 909
    /// voice levels: a song line reaches 4 (+12 dB, the desk channel's
    /// trim), while a knob turns the stock panel's 0..1, as it always has
    /// (CC 55 at full is hh_level 1, a saved plugin session holds its
    /// levels, a .mid file's level CCs render as they did).
    pub fn knob_range(self) -> (f32, f32, Curve) {
        match self {
            Param::BdLevel | Param::SdLevel | Param::RsLevel | Param::CpLevel | Param::HhLevel => {
                (0.0, 1.0, Curve::Lin)
            }
            _ => self.range(),
        }
    }

    /// Map a normalized controller position (0..1) onto this parameter's
    /// knob span, honoring its curve.
    pub fn midi_value(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let (lo, hi, curve) = self.knob_range();
        match curve {
            Curve::Lin => lo + (hi - lo) * t,
            Curve::Log => lo * (hi / lo).powf(t),
            Curve::Step => (lo + (hi - lo) * t).round(),
        }
    }
}

impl Param {
    /// This parameter's setting in a snapshot, in the same units a song or
    /// patch line writes (selectors as their index). `None` = the value
    /// lives outside `ParamValues` (mixer strips, sampler slots, MIDI
    /// performance controls, the arrangement trim). Patch SAVE writes every
    /// `Some`, so a saved patch cannot leave a control behind; the sweep test
    /// drives every row through it.
    #[rustfmt::skip]
    pub fn read(self, v: &ParamValues) -> Option<f32> {
        Some(match self {
            Param::WaveformSel => v.waveform as u8 as f32,
            Param::Osc2Wave => v.osc2_wave as u8 as f32,
            Param::Osc3Wave => v.osc3_wave as u8 as f32,
            Param::CircuitSel => match v.circuit {
                crate::oscillator::CircuitModel::Moog => 0.0,
                crate::oscillator::CircuitModel::Arp => 1.0,
            },
            Param::SyncSel => if v.sync { 1.0 } else { 0.0 },
            Param::MonoSel => if v.mono { 1.0 } else { 0.0 },
            Param::ChorusModeSel => match v.chorus_mode {
                ChorusMode::Off => 0.0,
                ChorusMode::I => 1.0,
                ChorusMode::II => 2.0,
                ChorusMode::III => 3.0,
                ChorusMode::IV => 4.0,
            },
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
            Param::VelAmp => v.vel_amp,
            Param::VelFilter => v.vel_filter,
            Param::ReverbDecay => v.reverb_decay,
            Param::ReverbWet => v.reverb_wet,
            Param::ReverbTone => v.reverb_tone,
            Param::ReverbPre => v.reverb_pre,
            Param::EchoWet => v.echo,
            Param::EchoTime => v.echo_time,
            Param::EchoFeedback => v.echo_feedback,
            Param::EchoTone => v.echo_tone,
            Param::EchoPingPong => v.echo_pingpong,
            Param::Unison => v.unison,
            Param::UnisonDetune => v.unison_detune,
            Param::Spread => v.spread,
            Param::ChorusRate => v.chorus_rate,
            Param::ChorusDepth => v.chorus_depth,
            Param::ChorusHiss => v.chorus_hiss,
            Param::TapeWow => v.tape_wow,
            Param::TapeFlutter => v.tape_flutter,
            Param::TapeDrive => v.tape_drive,
            Param::TapeAge => v.tape_age,
            Param::CompIn => v.comp_in,
            Param::CompThreshold => v.comp_threshold,
            Param::CompRatio => v.comp_ratio,
            Param::CompAttack => v.comp_attack,
            Param::CompRelease => v.comp_release,
            Param::CompMakeup => v.comp_makeup,
            Param::CompMix => v.comp_mix,
            Param::CompScHpf => v.comp_sc_hpf,
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
            Param::DrumTone => v.dr_tone,
            Param::VoxLevel => v.vox_level,
            Param::VoxDry => v.vox_dry,
            Param::VoxBreath => v.vox_breath,
            Param::VoxClarity => v.vox_clarity,
            Param::VoxVibrato => v.vox_vibrato,
            Param::VoxModeSel => v.vox_mode,
            Param::VoxIntonation => v.vox_intonation,
            Param::PitchEnv => v.pitch_env,
            Param::PitchEnvTime => v.pitch_env_time,
            Param::PitchEnvCurve => v.pitch_env_curve,
            Param::KeySync => v.key_sync,
            Param::ShaperDrive => v.shaper,
            Param::ShaperModeSel => v.shaper_mode,
            Param::Supersaw => v.supersaw,
            Param::Osc1Level => v.osc1_level,
            Param::SupersawDetune => v.supersaw_detune,
            Param::SupersawMix => v.supersaw_mix,
            Param::NoiseOsc => v.noise_osc,
            Param::NoiseCutoff => v.noise_cutoff,
            Param::NoiseRes => v.noise_res,
            Param::NoiseModeSel => v.noise_mode,
            Param::BendCurve => v.bend_curve,
            _ => return None,
        })
    }

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
            Param::Output => vm.set_output(value),
            Param::Width => vm.set_width(value),
            Param::TrackGain
            | Param::TrackPan
            | Param::ReverbSend
            | Param::SpringSend
            | Param::ChorusSend
            | Param::EchoSend
            | Param::DuckAmount
            | Param::DuckRelease
            | Param::DuckAttack
            | Param::DuckHold
            | Param::TrackComp
            | Param::ShapeDepth
            | Param::ShapeSmooth
            | Param::Dist
            | Param::DistMode
            | Param::DistMix
            | Param::EqLow
            | Param::EqLowFreq
            | Param::EqMid
            | Param::EqMidFreq
            | Param::EqMidQ
            | Param::EqHigh
            | Param::EqHighFreq => vm.set_track_mix(0, self, value),
            Param::Clip => vm.set_master_clip(value),
            Param::Limit => vm.set_limit(value),
            Param::LimitCeiling => vm.set_limit_ceiling(value),
            Param::LimitRelease => vm.set_limit_release(value),
            // unscoped, the glitch inserts sit across the bus
            Param::Stutter | Param::Reverse | Param::TapeStop => vm.set_glitch(0, self, value),
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
            Param::MonoSel => vm.set_mono(value.round() as i32 >= 1),
            Param::PitchShift => vm.set_pitch_shift(value),
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
            Param::VelAmp => vm.set_vel_amp(value),
            Param::VelFilter => vm.set_vel_filter(value),
            Param::ReverbDecay => vm.set_reverb_decay(value),
            Param::ReverbWet => vm.set_reverb_wet(value),
            Param::ReverbTone => vm.set_reverb_tone(value),
            Param::ReverbPre => vm.set_reverb_pre(value),
            Param::EchoWet => vm.set_echo(value),
            Param::EchoTime => vm.set_echo_time(value),
            Param::EchoFeedback => vm.set_echo_feedback(value),
            Param::EchoTone => vm.set_echo_tone(value),
            Param::EchoPingPong => vm.set_echo_pingpong(value),
            Param::Unison => vm.set_unison(value),
            Param::UnisonDetune => vm.set_unison_detune(value),
            Param::Spread => vm.set_spread(value),
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
            Param::ChorusHiss => vm.set_chorus_hiss(value),
            Param::TapeWow => vm.set_tape_wow(value),
            Param::TapeFlutter => vm.set_tape_flutter(value),
            Param::TapeDrive => vm.set_tape_drive(value),
            Param::TapeAge => vm.set_tape_age(value),
            Param::CompIn => vm.set_comp_in(value),
            Param::CompThreshold => vm.set_comp_threshold(value),
            Param::CompRatio => vm.set_comp_ratio(value),
            Param::CompAttack => vm.set_comp_attack(value),
            Param::CompRelease => vm.set_comp_release(value),
            Param::CompMakeup => vm.set_comp_makeup(value),
            Param::CompMix => vm.set_comp_mix(value),
            Param::CompScHpf => vm.set_comp_sc_hpf(value),
            Param::BdLevel
            | Param::BdTune
            | Param::BdAttack
            | Param::BdDecay
            | Param::BdSweep
            | Param::BdDrive
            | Param::SdLevel
            | Param::SdTune
            | Param::SdTone
            | Param::SdSnappy
            | Param::SdDecay
            | Param::RsLevel
            | Param::RsTune
            | Param::CpLevel
            | Param::CpDecay
            | Param::HhLevel
            | Param::HhTune
            | Param::HhMetal
            | Param::ChDecay
            | Param::OhDecay
            | Param::DrumDrive
            | Param::DrumTone => vm.set_drum_knob(self, value),
            Param::VoxLevel => vm.set_vox_level(value),
            Param::VoxDry => vm.set_vox_dry(value),
            Param::VoxBreath => vm.set_vox_breath(value),
            Param::VoxClarity => vm.set_vox_clarity(value),
            Param::VoxVibrato => vm.set_vox_vibrato(value),
            Param::VoxModeSel => vm.set_vox_mode(value),
            Param::VoxIntonation => vm.set_vox_intonation(value),
            Param::VoxFormant => vm.set_vox_formant(value),
            // Un-addressed sampler automation reaches every slot (the
            // per-track path routes by channel before it gets here)
            Param::SmpPitch
            | Param::SmpStart
            | Param::SmpGain
            | Param::SmpPan
            | Param::SmpAttack
            | Param::SmpRelease
            | Param::SmpCutoff
            | Param::SmpRes
            | Param::SmpFormant
            | Param::SmpFormantKeep => vm.set_sampler_all(self, value),
            Param::PitchEnv
            | Param::PitchEnvTime
            | Param::PitchEnvCurve
            | Param::KeySync
            | Param::ShaperDrive
            | Param::ShaperModeSel
            | Param::Supersaw
            | Param::Osc1Level
            | Param::SupersawDetune
            | Param::SupersawMix
            | Param::NoiseOsc
            | Param::NoiseCutoff
            | Param::NoiseRes
            | Param::NoiseModeSel
            | Param::BendCurve => vm.set_source_param(self, value),
        }
    }

    /// Whether automation addressed to one track's channel lands on that
    /// track alone. Voice-level controls live in the channel's snapshot
    /// and strip controls on its desk strip; the tape deck's transport
    /// routes to its slot, and the 909 board and the voice box are whole
    /// instruments owned by their one track. Everything else -- the
    /// shared noise generator, the reverb, spring, chorus and tape, the
    /// master volume, the performance controllers -- is one circuit under
    /// every track, and `set_channel_param` hands it to the global path,
    /// so `automate buzz.noise` used to noise up the whole mix.
    pub(crate) fn reaches_track(self, channel: u16) -> bool {
        use Param as P;
        if self.apply_to_params(&mut ParamValues::default(), self.range().0) {
            return true;
        }
        if self.is_strip() || self.is_strip_comp_knob() {
            return true;
        }
        match self {
            P::Stutter | P::Reverse | P::TapeStop => true,
            P::SmpPitch
            | P::SmpStart
            | P::SmpGain
            | P::SmpPan
            | P::SmpAttack
            | P::SmpRelease
            | P::SmpCutoff
            | P::SmpRes
            | P::SmpFormant
            | P::SmpFormantKeep => crate::sampler::slot_for_channel(channel).is_some(),
            P::BdLevel
            | P::BdTune
            | P::BdAttack
            | P::BdDecay
            | P::BdSweep
            | P::BdDrive
            | P::SdLevel
            | P::SdTune
            | P::SdTone
            | P::SdSnappy
            | P::SdDecay
            | P::RsLevel
            | P::RsTune
            | P::CpLevel
            | P::CpDecay
            | P::HhLevel
            | P::HhTune
            | P::HhMetal
            | P::ChDecay
            | P::OhDecay
            | P::DrumDrive
            | P::DrumTone => crate::drums::kit_for_channel(channel).is_some(),
            P::VoxLevel
            | P::VoxDry
            | P::VoxBreath
            | P::VoxClarity
            | P::VoxVibrato
            | P::VoxModeSel
            | P::VoxIntonation
            | P::VoxFormant => channel == crate::vox::VOX_CHANNEL,
            _ => false,
        }
    }

    /// A control on a track's own mixer strip (voice_manager::ChannelMix):
    /// a track-line option (`gain=0.8`) and a scoped lane
    /// (`automate lead.gain`). Unscoped, it turns the panel channel's strip.
    pub(crate) fn is_strip(self) -> bool {
        use Param as P;
        matches!(
            self,
            P::TrackGain
                | P::TrackPan
                | P::ReverbSend
                | P::SpringSend
                | P::ChorusSend
                | P::EchoSend
                | P::DuckAmount
                | P::DuckRelease
                | P::DuckAttack
                | P::DuckHold
                | P::TrackComp
                | P::ShapeDepth
                | P::ShapeSmooth
                | P::Dist
                | P::DistMode
                | P::DistMix
                | P::EqLow
                | P::EqLowFreq
                | P::EqMid
                | P::EqMidFreq
                | P::EqMidQ
                | P::EqHigh
                | P::EqHighFreq
        )
    }

    /// The bus compressor's knobs. Unscoped they turn the mix-bus
    /// compressor; scoped to a track (`automate drums.comp_ratio`, or
    /// `comp_ratio=` on its track line) they turn that track's strip
    /// compressor, the one `comp=` puts in.
    pub(crate) fn is_strip_comp_knob(self) -> bool {
        use Param as P;
        matches!(
            self,
            P::CompThreshold
                | P::CompRatio
                | P::CompAttack
                | P::CompRelease
                | P::CompMakeup
                | P::CompMix
                | P::CompScHpf
        )
    }

    /// The values the engine takes for this parameter: the table range,
    /// except where an engine-safety limit is deliberately wider (see
    /// `clamp`). The song parser clamps to this and warns when it must.
    pub fn accepts(self) -> (f32, f32) {
        match self {
            Param::Detune => (0.0, 50.0),
            Param::Glide => (0.0, 5.0),
            Param::PitchBendSemis => (-24.0, 24.0),
            _ => {
                let (lo, hi, _) = self.range();
                (lo, hi)
            }
        }
    }

    /// Write a VOICE-LEVEL parameter into a snapshot (per-track patches).
    /// Returns false for master effects, noise, volume and performance
    /// controllers. Oscillator/filter/envelope controls and the track's
    /// own LFO live in this snapshot.
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
            Param::PitchShift => p.pitch_shift = self.clamp(value),
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
            Param::LfoRate => p.lfo_rate = value,
            Param::LfoShape => p.lfo_shape = value,
            Param::LfoPitch => p.lfo_pitch = value,
            Param::LfoFilter => p.lfo_filter = value,
            Param::LfoPwm => p.lfo_pwm = value,
            Param::FilterEnvAmount => p.filter_env_amount = value,
            Param::FilterAttack => p.filter_attack = value,
            Param::FilterDecay => p.filter_decay = value,
            Param::FilterSustain => p.filter_sustain = value,
            Param::FilterRelease => p.filter_release = value,
            Param::VelAmp => p.vel_amp = value,
            Param::VelFilter => p.vel_filter = value,
            // Mono is voice-level: it is how THIS track's notes claim cards
            Param::MonoSel => p.mono = value.round() as i32 >= 1,
            // Unison is voice-level: it decides how many cards THIS track's
            // notes claim, so it must live in the per-channel snapshot, not
            // the shared bus.
            Param::Unison => p.unison = value,
            Param::UnisonDetune => p.unison_detune = value,
            // Spread is voice-level too: where THIS track's notes sit
            Param::Spread => p.spread = value,
            // The kick-bass, wall and riser controls are the card's own
            Param::PitchEnv => p.pitch_env = self.clamp(value),
            Param::PitchEnvTime => p.pitch_env_time = self.clamp(value),
            Param::PitchEnvCurve => p.pitch_env_curve = self.clamp(value),
            Param::KeySync => p.key_sync = self.clamp(value).round(),
            Param::ShaperDrive => p.shaper = self.clamp(value),
            Param::ShaperModeSel => p.shaper_mode = self.clamp(value).round(),
            Param::Supersaw => p.supersaw = self.clamp(value),
            Param::Osc1Level => p.osc1_level = self.clamp(value),
            Param::SupersawDetune => p.supersaw_detune = self.clamp(value),
            Param::SupersawMix => p.supersaw_mix = self.clamp(value),
            Param::NoiseOsc => p.noise_osc = self.clamp(value),
            Param::NoiseCutoff => p.noise_cutoff = self.clamp(value),
            Param::NoiseRes => p.noise_res = self.clamp(value),
            Param::NoiseModeSel => p.noise_mode = self.clamp(value).round(),
            Param::BendCurve => p.bend_curve = self.clamp(value),
            _ => return false,
        }
        true
    }
}

/// Every number in song and patch text must be finite. Rust's float
/// parser also accepts `nan`, `inf` and `infinity`, and those sail past the
/// range checks (every comparison with NaN is false; `clamp` passes NaN
/// through): `C4:inf` sized the tempo map past memory and panicked, `bpm
/// nan` timed every event at NaN, and a NaN cutoff poisons a recursive
/// filter for good.
pub(crate) trait ParseFinite {
    fn parse_finite<T: std::str::FromStr + Copy + Into<f64>>(&self) -> Result<T, ()>;
}

impl ParseFinite for str {
    fn parse_finite<T: std::str::FromStr + Copy + Into<f64>>(&self) -> Result<T, ()> {
        match self.parse::<T>() {
            Ok(v) if v.into().is_finite() => Ok(v),
            _ => Err(()),
        }
    }
}

/// Parse patch-file text (`param value` lines) into a parameter snapshot.
/// Bus-level lines are ignored — a channel patch describes a voice, not
/// the shared effects.
pub fn params_from_patch(text: &str) -> Result<ParamValues, String> {
    let mut p = ParamValues::neutral();
    for (param, value) in patch_lines(text)? {
        param.apply_to_params(&mut p, value);
    }
    Ok(p)
}

/// Every `name value` line of a patch, parsed and in file order — both
/// halves: the voice lines a snapshot keeps and the bus lines it does not.
pub(crate) fn patch_lines(text: &str) -> Result<Vec<(Param, f32)>, String> {
    let mut lines = Vec::new();
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
            .parse_finite()
            .map_err(|_| format!("patch line {}: bad value for '{}'", no + 1, name))?;
        let param = Param::from_name(name)
            .ok_or_else(|| format!("patch line {}: unknown parameter '{}'", no + 1, name))?;
        lines.push((param, value));
    }
    Ok(lines)
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
    NoteOn {
        note: u8,
        velocity: f32,
        channel: u16,
    },
    NoteOff {
        note: u8,
        channel: u16,
    },
    Param {
        param: Param,
        value: f32,
        channel: u16,
    },
    /// A syllable for the voice box, fired `onset_lead_ms` BEFORE its
    /// note-on so the onset consonants speak early and the vowel lands
    /// on the beat (sung diction: the vowel owns the beat). `note` is
    /// the pitch the onset approaches; the note-on holds the nucleus.
    VoxLead {
        syl: crate::vox::Syllable,
        note: u8,
        velocity: f32,
    },
    /// A note bend (`C4:2>G3`): the note just struck on `channel` glides
    /// `semitones` from its own pitch over `seconds` (its held length),
    /// along the track's `bend_curve`. Sorts right after its note-on.
    Bend {
        note: u8,
        channel: u16,
        semitones: f32,
        seconds: f64,
    },
    /// The song's beat clock: at this event the song is at `beat`, moving
    /// at `bpm` until the next one. Only songs whose strips ride the beat
    /// (`shape=`) carry these: one at the downbeat, then one wherever the
    /// tempo lane moves.
    Clock { beat: f64, bpm: f64 },
}

pub struct SongEvent {
    pub time: f64, // seconds from song start
    pub kind: EventKind,
}

/// A parsed song: the timed events plus each patch channel's parameter
/// snapshot (channel N+1 = channels[N]; channel 0 is the live panel).
pub struct Song {
    pub events: Vec<SongEvent>,
    /// Exact offline-render allowance after the last event.
    pub tail_seconds: f64,
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
    /// Strip routing set once at the downbeat: sidechain keys and shapers.
    pub mix: SongMix,
    /// What the parser had to change to play the song (a value clamped
    /// into its parameter's range), one line each, e.g.
    /// "line 12: gain 3.8 clamped to 4". The loader prints them.
    pub warnings: Vec<String>,
}

/// The mixer-strip routing a song's track lines declare.
#[derive(Default)]
pub struct SongMix {
    /// `duck_from=`: (keyed strip's channel, its key).
    pub duck_keys: Vec<(u16, crate::voice_manager::DuckKey)>,
    /// `shape=`: (channel, its tempo-synced volume shaper).
    pub shapers: Vec<(u16, crate::inserts::Shaper)>,
}

/// A rest this short is rounding noise from a generator, not a rest:
/// it plays as nothing (`R:0` used to be a hard error).
pub(crate) const REST_EPSILON_BEATS: f64 = 1e-6;

/// Seconds rendered after the last event unless a song says otherwise.
pub(crate) const DEFAULT_TAIL_SECONDS: f64 = 4.0;

pub fn load_song(path: &str) -> Result<Song, String> {
    load_song_with_patch(path, None)
}

/// Load a song, or import a Standard MIDI File (`.mid`/`.midi`, see
/// src/midi_file.rs) whose melodic channels all play `patch`
/// (`patches/NAME.patch`; Init when None). A song file names its voices
/// per track (`patch=`), so an override for one is refused rather than
/// silently ignored.
pub fn load_song_with_patch(path: &str, patch: Option<&str>) -> Result<Song, String> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "mid" || ext == "midi" {
        #[cfg(feature = "app")]
        return crate::midi_file::load(path, patch);
        #[cfg(not(feature = "app"))]
        return Err(format!(
            "'{}': MIDI file import is part of the app build",
            path
        ));
    }
    if let Some(name) = patch {
        return Err(format!(
            "patch '{}' applies to MIDI files; '{}' picks its voices per track (patch=)",
            name, path
        ));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read song file '{}': {}", path, e))?;
    let song = parse_song(&text)?;
    // A generator's systematic slip can clamp thousands of values: the
    // first ones say what and where, the count says how many
    const SHOWN: usize = 20;
    for w in song.warnings.iter().take(SHOWN) {
        eprintln!("warning: {}", w);
    }
    if song.warnings.len() > SHOWN {
        eprintln!(
            "warning: ... and {} more values clamped",
            song.warnings.len() - SHOWN
        );
    }
    Ok(song)
}

/// A voice from `patches/NAME.patch` — a song track's `patch=`, or the
/// `--patch` a MIDI file's channels play.
pub(crate) fn load_patch_file(name: &str) -> Result<ParamValues, String> {
    params_from_patch(&read_patch_file(name)?)
}

/// The text of `patches/NAME.patch`.
pub(crate) fn read_patch_file(name: &str) -> Result<String, String> {
    let path = format!("patches/{}.patch", name);
    std::fs::read_to_string(&path).map_err(|e| format!("patch '{}': {}", path, e))
}

/// Parse song text directly (the `--say` builder and tests use this).
pub fn parse_song_text(text: &str) -> Result<Song, String> {
    parse_song(text)
}

fn dispatch(vm: &mut VoiceManager, kind: &EventKind) {
    match kind {
        &EventKind::NoteOn {
            note,
            velocity,
            channel,
        } => vm.note_on_channel(note, velocity, channel),
        &EventKind::NoteOff { note, channel } => vm.note_off_channel(note, channel),
        &EventKind::Param {
            param,
            value,
            channel,
        } => vm.set_channel_param(channel, param, value),
        EventKind::VoxLead {
            syl,
            note,
            velocity,
        } => vm.vox_speak(syl, *note, *velocity),
        &EventKind::Bend {
            note,
            channel,
            semitones,
            seconds,
        } => vm.bend_note(channel, note, semitones, seconds as f32),
        &EventKind::Clock { beat, bpm } => vm.set_song_clock(beat, bpm),
    }
}

fn register_channels(vm: &mut VoiceManager, song: &Song) {
    for (i, p) in song.channels.iter().enumerate() {
        vm.set_channel_params((i + 1) as u16, *p);
    }
    // Every kit track's board exists from the first sample, as board 0
    // always has, so unscoped drum automation at beat 0 reaches them all
    for &(_, channel) in &song.tracks {
        vm.add_kit(channel);
    }
    // A throat lane on the voice box or a slot needs its recording's
    // glottal epochs, found once here rather than mid-song
    let throat_lane = |on: &dyn Fn(Param, u16) -> bool| {
        song.events.iter().any(|e| match e.kind {
            EventKind::Param { param, channel, .. } => on(param, channel),
            _ => false,
        })
    };
    if throat_lane(&|p, _| p == Param::VoxFormant) {
        vm.vox.prepare_formant();
    }
    if let Some((samples, rate)) = &song.vox_wav {
        vm.set_vox_wav(samples, *rate);
    }
    if let Some((samples, rate)) = &song.vox_pitch {
        vm.set_vox_pitch(samples, *rate);
    }
    for (i, slot) in song.samplers.iter().enumerate() {
        let mut slot = slot.clone();
        let ch = crate::sampler::SAMPLER_CHANNEL_BASE + i as u16;
        slot.cfg.formant |= throat_lane(&|p, c| {
            matches!(p, Param::SmpFormant | Param::SmpFormantKeep) && (c == ch || c == 0)
        });
        vm.set_sampler_slot(i, slot);
    }
    for &(channel, key) in &song.mix.duck_keys {
        vm.set_duck_key(channel, key);
    }
    for (channel, shaper) in &song.mix.shapers {
        vm.set_shaper(*channel, shaper.clone());
    }
    // A song that limits or clips anywhere runs the master peak control
    // from its downbeat (see VoiceManager::arm_master_peak)
    if song.events.iter().any(|e| {
        matches!(e.kind, EventKind::Param { param: Param::Limit, value, .. } if value < 0.0)
            || matches!(e.kind, EventKind::Param { param: Param::Clip, value, .. } if value > 0.0)
    }) {
        vm.arm_master_peak();
    }
}

/// A sample-accurate offline bounce. Audio is generated on demand; keeping
/// a long song in memory is the caller's explicit choice via `collect()`.
pub struct OfflineRender<'a> {
    vm: VoiceManager,
    events: &'a [SongEvent],
    next_event: usize,
    frame: usize,
    total: usize,
    sample_rate: f64,
    peak_voices: usize,
}

impl OfflineRender<'_> {
    /// Render this bounce's voice cards on `lanes` threads (see
    /// `VoiceManager::set_card_lanes`). Bit-identical to the serial loop.
    pub fn with_card_lanes(mut self, lanes: usize) -> Self {
        self.vm.set_card_lanes(lanes);
        self
    }

    pub fn peak_voices(&self) -> usize {
        self.peak_voices
    }
}

impl Iterator for OfflineRender<'_> {
    type Item = (f32, f32);

    fn next(&mut self) -> Option<Self::Item> {
        if self.frame == self.total {
            return None;
        }
        let t = self.frame as f64 / self.sample_rate;
        let mut fired = false;
        while self.next_event < self.events.len() && self.events[self.next_event].time <= t {
            dispatch(&mut self.vm, &self.events[self.next_event].kind);
            self.next_event += 1;
            fired = true;
        }
        if fired {
            self.peak_voices = self
                .peak_voices
                .max(self.vm.voices.iter().filter(|v| v.is_active()).count());
        }
        self.frame += 1;
        Some(self.vm.render_next())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.total - self.frame;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for OfflineRender<'_> {}
impl std::iter::FusedIterator for OfflineRender<'_> {}

/// Threads a full offline bounce renders its voice cards on: the
/// `PATINA_LANES` environment variable, default 1. Measured on an M5 Pro,
/// 4 lanes bounce ~1.3x faster for ~3x the CPU (the lanes spin through
/// every sample's serial bus work), so it is opt-in for when wall time
/// matters more than heat. Stems run serially inside and in parallel with
/// one another instead (see `render_stems`).
pub fn card_lanes() -> usize {
    std::env::var("PATINA_LANES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1)
}

/// Stream a song through the full engine, including its configured tail.
pub fn render_offline(song: &Song, sample_rate: f32) -> OfflineRender<'_> {
    render_offline_solo(song, sample_rate, None)
}

/// Stream a stem: all events and circuits still run, but only the solo
/// channel's strip reaches the bus and effect sends.
pub fn render_offline_solo(song: &Song, sample_rate: f32, solo: Option<u16>) -> OfflineRender<'_> {
    // Unison multiplies voice demand. Offline bounces use 64 distinct
    // voice boards to keep busy arrangements from stealing held notes.
    let mut vm = VoiceManager::new(sample_rate, 64);
    vm.set_solo(solo);
    vm.warm_up();
    register_channels(&mut vm, song);
    let end = song.events.last().map(|e| e.time).unwrap_or(0.0) + song.tail_seconds;
    OfflineRender {
        vm,
        events: &song.events,
        next_event: 0,
        frame: 0,
        total: (end * sample_rate as f64) as usize,
        sample_rate: sample_rate as f64,
        peak_voices: 0,
    }
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
    Notes {
        vel: f32,
        len: f64,
        channel: u16,
        swing: f64,
    },
    Automation {
        param: Param,
        current: Option<f32>,
        channel: u16,
    },
    /// `automate bpm`: tokens land on the tempo map, not the event list
    Tempo {
        current: Option<f32>,
    },
}

// (beats, order-rank, kind); rank makes offs < params < ons at equal times
type RawEvent = (f64, u8, EventKind);

fn parse_song(text: &str) -> Result<Song, String> {
    let mut bpm = 120.0_f64;
    let mut gate = 0.9_f64;
    let mut tail_seconds = DEFAULT_TAIL_SECONDS;
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
    // `kit=909` tracks so far: each one is a 909 board of its own
    let mut kit_boards = 0usize;
    // Named beat ranges (`section CH1 144..180`), for seeks and `during`
    let mut sections: std::collections::HashMap<String, (f64, f64)> =
        std::collections::HashMap::new();
    // Values the parser clamped into range, reported line by line
    let mut warnings: Vec<String> = Vec::new();
    // Strip routing: shapers as they come, `duck_from=` keys resolved
    // once every track is known (line, keyed channel, key spec)
    let mut mix = SongMix::default();
    let mut duck_from: Vec<(usize, u16, String)> = Vec::new();

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
                bpm = line[3..]
                    .trim()
                    .parse_finite::<f64>()
                    .map_err(|_| err("invalid bpm".into()))?;
                if !(20.0..=MAX_SONG_BPM as f64).contains(&bpm) {
                    return Err(err(format!("bpm must be 20-{MAX_SONG_BPM}")));
                }
            }
            "gate" => {
                gate = line[4..]
                    .trim()
                    .parse_finite::<f64>()
                    .map_err(|_| err("invalid gate".into()))?;
                warnings.extend(range_warning("gate", gate as f32, 0.05, 1.0).map(&err));
                gate = gate.clamp(0.05, 1.0);
            }
            "tail" => {
                tail_seconds = line[4..]
                    .trim()
                    .parse_finite::<f64>()
                    .map_err(|_| err("invalid tail duration".into()))?;
                if !(0.0..=30.0).contains(&tail_seconds) {
                    return Err(err("tail must be between 0 and 30 seconds".into()));
                }
            }
            "section" => {
                let mut it = line.split_whitespace().skip(1);
                let name = it.next().ok_or_else(|| {
                    err("section needs a name and a range: section CH1 144..180".into())
                })?;
                let range = it
                    .next()
                    .ok_or_else(|| err(format!("section {} needs a range, e.g. 144..180", name)))?;
                let (a, b) = range
                    .split_once("..")
                    .ok_or_else(|| err(format!("section range must be A..B, got '{}'", range)))?;
                let a: f64 = a
                    .parse_finite()
                    .map_err(|_| err(format!("invalid section start '{}'", a)))?;
                let b: f64 = b
                    .parse_finite()
                    .map_err(|_| err(format!("invalid section end '{}'", b)))?;
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
                let mut track_duck_from: Option<String> = None;
                let mut shape_spec: Option<String> = None;
                let mut shape_beats: Option<f64> = None;
                for opt in line.split_whitespace().skip(2) {
                    if let Some(v) = opt.strip_prefix("vel=") {
                        vel = v
                            .parse_finite::<f32>()
                            .map_err(|_| err(format!("invalid vel '{}'", v)))?;
                        warnings.extend(range_warning("vel", vel, 0.0, 1.0).map(&err));
                        vel = vel.clamp(0.0, 1.0);
                    } else if let Some(v) = opt.strip_prefix("len=") {
                        len = v
                            .parse_finite::<f64>()
                            .map_err(|_| err(format!("invalid len '{}'", v)))?;
                    } else if let Some(v) = opt.strip_prefix("swing=") {
                        swing = v
                            .parse_finite::<f64>()
                            .map_err(|_| err(format!("invalid swing '{}'", v)))?;
                        if !(0.4..=0.8).contains(&swing) {
                            return Err(err("swing must be 0.4-0.8 (0.5 = straight)".into()));
                        }
                    } else if let Some(v) = opt.strip_prefix("duck_from=") {
                        // Key this strip's duck off another track's notes
                        // (resolved below, once every track is known)
                        track_duck_from = Some(v.to_string());
                    } else if let Some(v) = opt.strip_prefix("shape=") {
                        shape_spec = Some(v.to_string());
                    } else if let Some(v) = opt.strip_prefix("shape_beats=") {
                        shape_beats = Some(
                            v.parse_finite::<f64>()
                                .map_err(|_| err(format!("invalid shape_beats '{}'", v)))?,
                        );
                    } else if let Some((k, v)) = opt.split_once('=').filter(|(k, _)| {
                        Param::from_name(k).map_or(false, |p| p.is_strip() || p.is_strip_comp_knob())
                    }) {
                        let value = v
                            .parse_finite::<f32>()
                            .map_err(|_| err(format!("invalid {} '{}'", k, v)))?;
                        let param = Param::from_name(k).unwrap();
                        if let Some(w) = clamp_warning(param, value) {
                            warnings.push(err(w));
                        }
                        mix_opts.push((param, value));
                    } else if let Some(kit) = opt.strip_prefix("kit=") {
                        // A drum track: `kit=909` gives the track a 909
                        // board of its own on its own channel (the first
                        // on DRUM_CHANNEL), so its knobs, strip and stem
                        // are its alone. `kit=<track>` plays an earlier
                        // kit track's board instead: one machine, one
                        // strip, one stem, and the closed hat still
                        // chokes the open one across the two tracks. Any
                        // other name used to play the 909 too, so
                        // `kit=808` sounded like it had been honored.
                        channel = if kit == "909" {
                            if kit_boards == crate::drums::MAX_KITS {
                                return Err(err(format!(
                                    "too many kit=909 tracks (max {}); kit=<track> \
                                     shares an earlier kit track's board",
                                    crate::drums::MAX_KITS
                                )));
                            }
                            kit_boards += 1;
                            crate::drums::kit_channel(kit_boards - 1)
                        } else {
                            track_channels
                                .iter()
                                .find(|(t, c)| {
                                    t == kit && crate::drums::kit_for_channel(*c).is_some()
                                })
                                .map(|(_, c)| *c)
                                .ok_or_else(|| {
                                    err(format!(
                                        "unknown kit '{}': kit=909 gives the track its own \
                                         909 board, kit=<track> shares the board of a kit \
                                         track defined above",
                                        kit
                                    ))
                                })?
                        };
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
                            .parse_finite::<f64>()
                            .map_err(|_| err(format!("invalid wav_at '{}'", v)))?;
                        if b < 0.0 {
                            return Err(err("wav_at must be >= 0".into()));
                        }
                        vox_wav_at = Some(b);
                    } else if let Some(v) = opt.strip_prefix("patch=") {
                        // A private patch for this track: the file's
                        // voice-level parameters become this channel
                        let text = read_patch_file(v).map_err(err)?;
                        for (param, value) in patch_lines(&text).map_err(err)? {
                            warnings.extend(
                                clamp_warning(param, value)
                                    .map(|w| err(format!("patch '{}': {}", v, w))),
                            );
                        }
                        channels.push(params_from_patch(&text).map_err(err)?);
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
                            .parse_finite()
                            .map_err(|_| err(format!("invalid beats '{}'", v)))?;
                        if n <= 0.0 {
                            return Err(err("beats must be positive".into()));
                        }
                        smp_beats = Some(n);
                    } else if let Some(cfg) = smp.as_mut() {
                        let mut clamped = Vec::new();
                        parse_sampler_option(cfg, opt, &mut smp_mode_set, &mut clamped)
                            .map_err(err)?;
                        warnings.extend(clamped.into_iter().map(err));
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
                        data =
                            std::sync::Arc::new(crate::sampler::crunch(&data, smp_bits, smp_rate));
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
                        let speed = secs / (beats * 60.0 / bpm);
                        warnings.extend(
                            range_warning(&format!("beats={} needs playback speed", beats), speed as f32, 0.03, 32.0)
                                .map(&err),
                        );
                        cfg.speed = speed.clamp(0.03, 32.0) as f32;
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
                // A bare synth track gets a PRIVATE channel seeded from
                // the panel's musical default. It used to play on channel
                // 0 — the live panel itself — where per-track voice-level
                // automation silently lost to (or fought over) the shared
                // panel state: `automate sub.waveform 0` left three
                // detuned saws playing. Same sound at rest, but now the
                // track's automation actually lands on its own voice.
                if channel == 0 {
                    channels.push(ParamValues::default());
                    channel = channels.len() as u16;
                }
                if let Some(spec) = shape_spec {
                    let shaper = crate::inserts::Shaper::parse(&spec, shape_beats).map_err(err)?;
                    // one strip per channel: a later shape on a shared
                    // board replaces the earlier one
                    mix.shapers.retain(|(c, _)| *c != channel);
                    mix.shapers.push((channel, shaper));
                } else if shape_beats.is_some() {
                    return Err(err("shape_beats= needs a shape= on the same track".into()));
                }
                if let Some(spec) = track_duck_from {
                    duck_from.push((line_no, channel, spec));
                }
                for (param, value) in mix_opts {
                    events.push((
                        0.0,
                        1,
                        param_event(param, value, channel),
                    ));
                }
                track_channels.push((name, channel));
                mode = TrackMode::Notes {
                    vel,
                    len,
                    channel,
                    swing,
                };
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
                        if let Some(param) = Param::from_name(pname) {
                            if !param.reaches_track(ch) {
                                return Err(err(format!(
                                    "automate '{}': {} is shared by every track, so a \
                                     track cannot own it; automate it unscoped \
                                     (`automate {}`){}",
                                    name,
                                    pname,
                                    pname,
                                    match param {
                                        Param::ReverbWet => ", or this track's reverb_send",
                                        Param::SpringWet => ", or this track's spring_send",
                                        Param::EchoWet => ", or this track's echo_send",
                                        _ => "",
                                    }
                                )));
                            }
                        }
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
                        .parse_finite()
                        .map_err(|_| err(format!("invalid value '{}'", toks[2])))?;
                    if let Some(w) = clamp_warning(param, value) {
                        warnings.push(err(w));
                    }
                    let names = toks.get(4).copied().ok_or_else(|| {
                        err("`during` needs section names, e.g. during CH1,CH2".into())
                    })?;
                    let base: Option<f32> = match toks.get(5).copied() {
                        Some("base") => Some(
                            toks.get(6)
                                .copied()
                                .ok_or_else(|| err("`base` needs a value".into()))?
                                .parse_finite()
                                .map_err(|_| err("invalid base value".into()))?,
                        ),
                        Some(t) => return Err(err(format!("unexpected token '{}'", t))),
                        None => None,
                    };
                    if toks.len() > 7 {
                        return Err(err(format!("unexpected token '{}'", toks[7])));
                    }
                    if let Some(w) = base.and_then(|b| clamp_warning(param, b)) {
                        warnings.push(err(w));
                    }
                    if let Some(base) = base {
                        events.push((
                            0.0,
                            1,
                            param_event(param, base, channel),
                        ));
                    }
                    for sname in names.split(',').filter(|s| !s.is_empty()) {
                        let &(a, b) = sections.get(sname).ok_or_else(|| {
                            err(format!("unknown section '{}' (define it above)", sname))
                        })?;
                        events.push((
                            a,
                            1,
                            param_event(param, value, channel),
                        ));
                        if let Some(base) = base {
                            events.push((
                                b,
                                1,
                                param_event(param, base, channel),
                            ));
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
                mode = TrackMode::Automation {
                    param,
                    current: None,
                    channel,
                };
            }
            _ => match &mut mode {
                TrackMode::None => {
                    return Err(err(
                        "event tokens before any 'track' or 'automate' line".into()
                    ));
                }
                TrackMode::Notes {
                    vel,
                    len,
                    channel,
                    swing,
                } => {
                    let swing = *swing;
                    let (vel, len, channel) = (*vel, *len, *channel);
                    let drums = crate::drums::kit_for_channel(channel).is_some();
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
                        // `>target` note bends ride the note (`C4:2>G3`,
                        // `[C4>D4 G4]`, `[..]>-12`); split them off too
                        let (token, bends) = split_bends(&token)
                            .map_err(|m| err(format!("token '{}': {}", token, m)))?;
                        let (notes, dur, vel, shift) = parse_note_token(&token, vel, len, drums)
                            .map_err(|m| err(format!("token '{}': {}", token, m)))?;
                        if !bends.is_empty() {
                            if notes.is_empty() {
                                return Err(err(format!(
                                    "token '{}': a rest has no pitch to bend",
                                    token
                                )));
                            }
                            if drums {
                                return Err(err(format!(
                                    "token '{}': a drum hit has no pitch to bend",
                                    token
                                )));
                            }
                        }
                        // the track's own vel= is clamped already: this is an
                        // explicit `@` past the range
                        warnings.extend(
                            range_warning(&format!("token '{}': velocity", token), vel, 0.0, 1.0)
                                .map(&err),
                        );
                        let vel = vel.clamp(0.0, 1.0);
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
                            // The vowel owns the beat: the syllable
                            // fires early by its onset length, so the
                            // consonants speak into the note and the
                            // nucleus lands on the note-on
                            let lead_beats =
                                crate::vox::onset_lead_ms(&syl) as f64 * 0.001 * bpm / 60.0;
                            let note = *notes.iter().min().unwrap();
                            events.push((
                                (sound_beat - lead_beats).max(0.0),
                                1,
                                EventKind::VoxLead {
                                    syl,
                                    note,
                                    velocity: vel,
                                },
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
                                EventKind::NoteOn {
                                    note,
                                    velocity: vel,
                                    channel,
                                },
                            ));
                        }
                        for &note in &notes {
                            events.push((off_beat, 0, EventKind::NoteOff { note, channel }));
                        }
                        // Each bend glides over its note's held length;
                        // `seconds` holds the off beat until the tempo map
                        // turns it into a length below
                        for (k, &note) in notes.iter().enumerate() {
                            let Some(target) = bends.target(k) else {
                                continue;
                            };
                            let semitones = target.semitones_from(note);
                            if semitones != 0.0 {
                                events.push((
                                    sound_beat,
                                    3,
                                    EventKind::Bend {
                                        note,
                                        channel,
                                        semitones,
                                        seconds: off_beat,
                                    },
                                ));
                            }
                        }
                        track_beat += dur;
                    }
                }
                TrackMode::Automation {
                    param,
                    current,
                    channel,
                } => {
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
                                if let Some(w) = clamp_warning(param, value) {
                                    warnings.push(err(w));
                                }
                                events.push((
                                    track_beat,
                                    1,
                                    param_event(param, value, channel),
                                ));
                                *current = Some(value);
                            }
                            AutoToken::Ramp { to, dur, shape } => {
                                if let Some(w) = clamp_warning(param, to) {
                                    warnings.push(err(w));
                                }
                                let from = current.ok_or_else(|| {
                                    err(format!(
                                        "token '{}': first token of an automate track must be a plain starting value",
                                        token
                                    ))
                                })?;
                                emit_ramp(
                                    &mut events,
                                    param,
                                    channel,
                                    from,
                                    to,
                                    track_beat,
                                    dur,
                                    shape,
                                );
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
                                if !(20.0..=MAX_SONG_BPM).contains(&value) {
                                    return Err(err(format!("bpm must be 20-{MAX_SONG_BPM}")));
                                }
                                tempo_lane.push((track_beat, AutoToken::Set(value)));
                                *current = Some(value);
                            }
                            AutoToken::Ramp { to, dur, shape } => {
                                if current.is_none() {
                                    return Err(
                                        err("first bpm token must be a plain value".into()),
                                    );
                                }
                                if !(20.0..=MAX_SONG_BPM).contains(&to) {
                                    return Err(err(format!("bpm must be 20-{MAX_SONG_BPM}")));
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
            return Err(format!(
                "wav_at={}: the song has no vox notes to start the wav",
                at
            ));
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
    end_notes_at_restrike(&mut events);

    let max_beat = events.iter().map(|e| e.0).fold(0.0f64, f64::max);
    // The tempo map holds one step per 1/32 beat to the last event, so a
    // stray seek like `>1e9` would size it past memory
    if max_beat > MAX_SONG_BEATS {
        return Err(format!(
            "the song runs to beat {max_beat}, past the {MAX_SONG_BEATS}-beat limit"
        ));
    }

    // `duck_from=NAME` keys a strip off every note of track NAME;
    // `duck_from=NAME.X` off one drum voice (BD SD RS CP CH OH) or one
    // note of it
    for (line_no, channel, spec) in duck_from {
        let e = |m: String| format!("line {}: duck_from={}: {}", line_no + 1, spec, m);
        let (name, part) = match spec.split_once('.') {
            Some((n, p)) => (n, Some(p)),
            None => (spec.as_str(), None),
        };
        let source = track_channels
            .iter()
            .find(|(t, _)| t == name)
            .map(|(_, c)| *c)
            .ok_or_else(|| e(format!("no track named '{}'", name)))?;
        use crate::voice_manager::{DuckKey, KeyFilter};
        let filter = match part {
            None => KeyFilter::Any,
            Some(p) => match crate::drums::kit_for_channel(source)
                .and_then(|_| crate::drums::DrumVoice::from_name(p))
            {
                Some(voice) => KeyFilter::Drum(voice),
                None => KeyFilter::Note(parse_note(p).map_err(e)?),
            },
        };
        mix.duck_keys.push((
            channel,
            DuckKey {
                channel: source,
                filter,
            },
        ));
    }

    // Shapers ride the beat and glitch lengths follow the tempo: give the
    // engine the song's clock, once at the downbeat and again at every
    // grid step where the tempo moves
    let glitched = events.iter().any(|(_, _, k)| {
        matches!(
            k,
            EventKind::Param {
                param: Param::Stutter | Param::Reverse | Param::TapeStop,
                ..
            }
        )
    });
    if !mix.shapers.is_empty() || glitched {
        let grid = bpm_grid(bpm, &tempo_lane, max_beat);
        let mut last = f64::NAN;
        for i in 0..grid.len() - 1 {
            let beat = i as f64 / AUTOMATION_STEPS_PER_BEAT;
            if beat > max_beat {
                break;
            }
            // the tempo map advances each step at the mean of its ends
            let mid = 0.5 * (grid[i] + grid[i + 1]) as f64;
            if mid != last {
                events.push((beat, 1, EventKind::Clock { beat, bpm: mid }));
                last = mid;
            }
        }
        events.sort_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        });
    }

    let time_of = tempo_map(bpm, &tempo_lane, max_beat);
    Ok(Song {
        events: events
            .into_iter()
            .map(|(beats, _, mut kind)| {
                // A bend's off beat becomes its length in seconds
                if let EventKind::Bend { seconds, .. } = &mut kind {
                    *seconds = (time_of(*seconds) - time_of(beats)).max(0.0);
                }
                SongEvent {
                    time: time_of(beats),
                    kind,
                }
            })
            .collect(),
        tail_seconds,
        channels,
        vox_wav,
        vox_pitch,
        samplers,
        tracks: track_channels,
        mix,
        warnings,
    })
}

/// The warning for a value outside what `param` takes, if it is.
fn clamp_warning(param: Param, value: f32) -> Option<String> {
    clamp_warning_as(param.name(), param, value)
}

/// The same, naming the value as the song wrote it (`res=` on a sampler
/// track is the smp_res parameter).
fn clamp_warning_as(label: &str, param: Param, value: f32) -> Option<String> {
    let (lo, hi) = param.accepts();
    range_warning(label, value, lo, hi)
}

/// The warning for a value the song wrote outside `lo..=hi`, if it is.
fn range_warning(label: &str, value: f32, lo: f32, hi: f32) -> Option<String> {
    (value < lo || value > hi)
        .then(|| format!("{} {} clamped to {}", label, value, value.clamp(lo, hi)))
}

/// A parameter event, its value clamped to what the engine takes (and
/// so to what any warning said it would be).
fn param_event(param: Param, value: f32, channel: u16) -> EventKind {
    let (lo, hi) = param.accepts();
    EventKind::Param {
        param,
        value: value.clamp(lo, hi),
        channel,
    }
}

/// One key cannot be held twice: striking it again ends the earlier press.
/// A note whose off lands after the next strike of the same key on the
/// same channel -- a swung offbeat leaning into the next grid note, a
/// `~+` drag, a long note under a repeated one -- would otherwise release
/// the NEW note (the engine retriggers the held card on the re-strike, and
/// the stale off then lifts it), cutting it to a click. The earlier note's
/// off moves to the re-strike's beat, where it sorts before the on exactly
/// as a gate-1 repeat does. `events` must already be sorted.
fn end_notes_at_restrike(events: &mut Vec<(f64, u8, EventKind)>) {
    let mut held: std::collections::HashMap<(u16, u8), u32> = std::collections::HashMap::new();
    let mut i = 0;
    while i < events.len() {
        match events[i].2 {
            EventKind::NoteOn { note, channel, .. } => {
                let count = held.entry((channel, note)).or_insert(0);
                if *count > 0 {
                    let stale = events[i + 1..].iter().position(|e| {
                        matches!(e.2, EventKind::NoteOff { note: n, channel: c }
                            if n == note && c == channel)
                    });
                    if let Some(k) = stale {
                        let (_, _, off) = events.remove(i + 1 + k);
                        events.insert(i, (events[i].0, 0, off));
                        i += 1;
                    }
                } else {
                    *count = 1;
                }
            }
            EventKind::NoteOff { note, channel } => {
                if let Some(count) = held.get_mut(&(channel, note)) {
                    *count = count.saturating_sub(1);
                }
            }
            _ => {}
        }
        i += 1;
    }
}

/// Compile the tempo lane into beat -> seconds. The lane is sampled at
/// automation resolution and 60/bpm integrated cumulatively, so ramps
/// (ritardando, accelerando, with any shape) land sample-accurately —
/// events keep their musical positions and time stretches around them.
fn tempo_map(base_bpm: f64, lane: &[(f64, AutoToken)], max_beat: f64) -> impl Fn(f64) -> f64 {
    let res = AUTOMATION_STEPS_PER_BEAT;
    let bpm_at = bpm_grid(base_bpm, lane, max_beat);
    let n = bpm_at.len();
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

/// The tempo lane sampled at automation resolution: bpm at every 1/32
/// beat from 0 to 8 beats past `max_beat`.
fn bpm_grid(base_bpm: f64, lane: &[(f64, AutoToken)], max_beat: f64) -> Vec<f32> {
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
    bpm_at
}

/// One tape-deck track option. Times are seconds on the source recording;
/// `root=` takes a note name, `loop=a:b` a pair of times.
fn parse_sampler_option(
    cfg: &mut crate::sampler::SlotConfig,
    opt: &str,
    mode_set: &mut bool,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    use crate::sampler::PlayMode;
    let secs = |v: &str, what: &str| -> Result<f32, String> {
        v.parse_finite::<f32>()
            .map_err(|_| format!("invalid {} '{}'", what, v))
    };
    let key = opt.split_once('=').map_or(opt, |(k, _)| k);
    let mut clamp = |param: Param, v: f32| -> f32 {
        warnings.extend(clamp_warning_as(key, param, v));
        param.clamp(v)
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
        let x = secs(v, "xfade")?;
        warnings.extend(range_warning("xfade", x, 0.0, 2.0));
        cfg.xfade = x.clamp(0.0, 2.0);
    } else if let Some(v) = opt.strip_prefix("chop=") {
        let n: usize = v
            .parse()
            .map_err(|_| format!("invalid chop count '{}'", v))?;
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
    } else if opt == "psola" {
        cfg.psola = true;
    } else if opt == "formant" {
        // the recording's own throat at every key
        cfg.formant = true;
        cfg.formant_keep = 1.0;
    } else if let Some(v) = opt.strip_prefix("formant=") {
        cfg.formant = true;
        cfg.formant_semis = Param::SmpFormant.clamp(secs(v, "formant")?);
    } else if let Some(v) = opt.strip_prefix("formant_keep=") {
        cfg.formant = true;
        cfg.formant_keep = Param::SmpFormantKeep.clamp(secs(v, "formant_keep")?);
    } else if opt == "reverse" {
        cfg.reverse = true;
    } else if opt == "fixed" {
        cfg.keytrack = false;
    } else if opt == "mono" || opt == "choke" {
        cfg.mono = true;
    } else if let Some(v) = opt.strip_prefix("gain=") {
        cfg.gain = clamp(Param::SmpGain, secs(v, "gain")?);
    } else if let Some(v) = opt.strip_prefix("pan=") {
        cfg.pan = clamp(Param::SmpPan, secs(v, "pan")?);
    } else if let Some(v) = opt.strip_prefix("pitch=") {
        cfg.pitch_semis = clamp(Param::SmpPitch, secs(v, "pitch")?);
    } else if let Some(v) = opt.strip_prefix("attack=") {
        cfg.attack = clamp(Param::SmpAttack, secs(v, "attack")?);
    } else if let Some(v) = opt.strip_prefix("release=") {
        cfg.release = clamp(Param::SmpRelease, secs(v, "release")?);
    } else if let Some(v) = opt.strip_prefix("vel_amt=") {
        let x = secs(v, "vel_amt")?;
        warnings.extend(range_warning("vel_amt", x, 0.0, 1.0));
        cfg.vel_amt = x.clamp(0.0, 1.0);
    } else if let Some(v) = opt.strip_prefix("cutoff=") {
        cfg.cutoff = clamp(Param::SmpCutoff, secs(v, "cutoff")?);
    } else if let Some(v) = opt.strip_prefix("res=") {
        cfg.res = clamp(Param::SmpRes, secs(v, "res")?);
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
        events.push((
            start_beat + dur,
            1,
            param_event(param, to, channel),
        ));
        return;
    }
    let steps = ((dur * AUTOMATION_STEPS_PER_BEAT).ceil() as usize).clamp(1, 4096);
    for k in 1..=steps {
        let t = k as f64 / steps as f64;
        let value = shape.interpolate(from, to, t as f32);
        events.push((
            start_beat + dur * t,
            1,
            param_event(param, value, channel),
        ));
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
    if let Ok(beat) = spec.parse_finite::<f64>() {
        if beat < 0.0 {
            return Err(format!("invalid seek '>{}' (before the song starts)", spec));
        }
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
        let d = s[i + 1..]
            .parse_finite::<f64>()
            .map_err(|_| "invalid duration".to_string())?;
        let hold = &s[..i] == "." || s[..i].eq_ignore_ascii_case("r");
        // A hold within rounding of zero (`R:0`) is nothing
        if hold && d.abs() <= REST_EPSILON_BEATS {
            return Ok(AutoToken::Hold(0.0));
        }
        if d <= 0.0 {
            return Err("duration must be positive".into());
        }
        dur = Some(d);
        s = &s[..i];
    }

    if s == "." || s.eq_ignore_ascii_case("r") {
        return Ok(AutoToken::Hold(
            dur.ok_or("hold needs a duration, e.g. R:4")?,
        ));
    }

    let value = s
        .parse_finite::<f32>()
        .map_err(|_| "invalid value".to_string())?;
    match dur {
        Some(dur) => Ok(AutoToken::Ramp {
            to: value,
            dur,
            shape,
        }),
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

/// Parse one note-track token into
/// (notes, duration-in-beats, velocity, timing-shift).
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
            .parse_finite::<f64>()
            .map_err(|_| "invalid timing shift".to_string())?;
        if shift.abs() > 0.5 {
            return Err("timing shift must be within +/-0.5 beats".into());
        }
        s = &s[..i];
    }

    if let Some(i) = s.rfind('@') {
        vel = s[i + 1..]
            .parse_finite::<f32>()
            .map_err(|_| "invalid velocity".to_string())?;
        s = &s[..i];
    }
    if let Some(i) = s.rfind(':') {
        dur = s[i + 1..]
            .parse_finite::<f64>()
            .map_err(|_| "invalid duration".to_string())?;
        s = &s[..i];
    }
    let rest = s == "." || s.eq_ignore_ascii_case("r");
    // A rest within rounding of zero (`R:0` from a generator) is nothing
    if rest && dur.abs() <= REST_EPSILON_BEATS {
        return Ok((Vec::new(), 0.0, vel, 0.0));
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
    let notes = if rest {
        Vec::new()
    } else if let Some(inner) = s.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        inner
            .split_whitespace()
            .map(one)
            .collect::<Result<Vec<u8>, String>>()?
    } else {
        vec![one(s)?]
    };

    // unclamped: the caller clamps it, and warns when it must
    Ok((notes, dur, vel, shift))
}

/// Where a note bend goes: a note (`>G3`, `>55`) or a signed interval in
/// semitones (`>-12`, `>+0.5`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum BendTarget {
    Note(u8),
    By(f32),
}

impl BendTarget {
    fn parse(s: &str) -> Result<Self, String> {
        if s.starts_with('+') || s.starts_with('-') {
            let by = s
                .parse_finite::<f32>()
                .map_err(|_| format!("invalid bend interval '{}'", s))?;
            if by.abs() > 48.0 {
                return Err(format!("bend interval {} is past +/-48 semitones", s));
            }
            return Ok(BendTarget::By(by));
        }
        parse_note(s)
            .map(BendTarget::Note)
            .map_err(|m| format!("bend target: {}", m))
    }

    fn semitones_from(self, note: u8) -> f32 {
        match self {
            BendTarget::Note(n) => n as f32 - note as f32,
            BendTarget::By(by) => by,
        }
    }
}

/// The bends a note token carries: one for the whole token, or one per
/// chord note (empty when the token bends nothing).
#[derive(Debug, Default, PartialEq)]
struct Bends {
    all: Option<BendTarget>,
    each: Vec<Option<BendTarget>>,
}

impl Bends {
    fn is_empty(&self) -> bool {
        self.all.is_none() && self.each.iter().all(|b| b.is_none())
    }

    fn target(&self, k: usize) -> Option<BendTarget> {
        self.all.or_else(|| self.each.get(k).copied().flatten())
    }
}

/// Split note bends off a note token, returning the plain token for
/// `parse_note_token`. `C4:2>G3` and `C4>G3:2` bend the note to G3 over
/// its held length; `[C4 E4]:2>-12` bends a whole chord down an octave;
/// `[C4>D4 G4]:2` bends one chord note and holds the other.
fn split_bends(token: &str) -> Result<(String, Bends), String> {
    if !token.contains('>') {
        return Ok((token.to_string(), Bends::default()));
    }
    // A target runs from its '>' to the next suffix (':', '@', '~') or the end
    let take_target = |rest: &str| -> Result<(String, Option<BendTarget>), String> {
        let Some(i) = rest.find('>') else {
            return Ok((rest.to_string(), None));
        };
        let after = &rest[i + 1..];
        let end = after.find([':', '@', '~']).unwrap_or(after.len());
        if after[end..].contains('>') {
            return Err("a note bends to one target".into());
        }
        let target = BendTarget::parse(&after[..end])?;
        Ok((format!("{}{}", &rest[..i], &after[end..]), Some(target)))
    };
    let mut bends = Bends::default();
    if let Some(inner) = token.strip_prefix('[') {
        let close = inner.find(']').ok_or("unmatched '['")?;
        let mut notes = Vec::new();
        for part in inner[..close].split_whitespace() {
            let (note, target) = match part.split_once('>') {
                Some((n, t)) => (n, Some(BendTarget::parse(t)?)),
                None => (part, None),
            };
            notes.push(note);
            bends.each.push(target);
        }
        let (rest, all) = take_target(&inner[close + 1..])?;
        if all.is_some() && bends.each.iter().any(|b| b.is_some()) {
            return Err("a chord bends as a whole (`]>`) or note by note, not both".into());
        }
        bends.all = all;
        return Ok((format!("[{}]{}", notes.join(" "), rest), bends));
    }
    let (plain, all) = take_target(token)?;
    bends.all = all;
    Ok((plain, bends))
}

/// Parse a note name like C4, F#3, Eb5 (C4 = MIDI 60), or a raw MIDI number.
fn parse_note(s: &str) -> Result<u8, String> {
    if s.chars().all(|c| c.is_ascii_digit()) {
        let n = s
            .parse::<u8>()
            .map_err(|_| format!("invalid MIDI number '{}'", s))?;
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

    /// Every key's ons and offs must alternate, each off after its on: an
    /// off that arrives while a LATER strike holds the key releases that
    /// later note instead of its own.
    fn assert_keys_alternate(song: &Song) {
        let mut held = std::collections::HashMap::new();
        for e in &song.events {
            match e.kind {
                EventKind::NoteOn { note, channel, .. } => {
                    let was = held.insert((channel, note), true);
                    assert_ne!(was, Some(true), "note {note} struck again while held");
                }
                EventKind::NoteOff { note, channel } => {
                    let was = held.insert((channel, note), false);
                    assert_eq!(was, Some(true), "note {note} released twice");
                }
                _ => {}
            }
        }
    }

    #[test]
    fn restriking_a_key_ends_the_earlier_note_first() {
        // Swung late, the offbeat 16th's off lands after the next grid C4
        let swung = parse_song("track a swing=0.66\n(C4:0.25)x8\n").unwrap();
        assert_keys_alternate(&swung);
        // Swung early, the offbeat strikes before the onbeat has let go
        let early = parse_song("track a swing=0.4\n(C4:0.25)x8\n").unwrap();
        assert_keys_alternate(&early);
        // A dragged hit and a long note under a repeat
        let dragged = parse_song("track a\nC4~+0.3 C4 C4\n").unwrap();
        assert_keys_alternate(&dragged);
        let under = parse_song("track a\nC4:4 >1 C4\n").unwrap();
        assert_keys_alternate(&under);
        // The moved off sits at the re-strike's time, just before its on
        let ev = &dragged.events;
        let second_on = ev
            .iter()
            .filter(|e| matches!(e.kind, EventKind::NoteOn { .. }))
            .nth(1)
            .unwrap()
            .time;
        assert!(ev
            .iter()
            .any(|e| matches!(e.kind, EventKind::NoteOff { .. }) && e.time == second_on));
    }

    #[test]
    fn non_finite_and_runaway_numbers_are_refused() {
        for text in [
            "track a\nC4:inf\n",
            "track a\nC4:nan\n",
            "track a\nC4@nan\n",
            "track a\nC4~nan\n",
            "track a\n>nan C4\n",
            "track a\n>-2 C4\n",
            "track a\n>1e9 C4\n",
            "track a\nC4:1e12\n",
            "track a swing=nan\nC4\n",
            "track a gain=nan\nC4\n",
            "bpm nan\ntrack a\nC4\n",
            "bpm 1e9\ntrack a\nC4\n",
            "gate nan\ntrack a\nC4\n",
            "tail inf\ntrack a\nC4\n",
            "section A 0..inf\ntrack a\nC4\n",
            "track a\nC4\nautomate cutoff\nnan\n",
            "track a\nC4\nautomate cutoff\n500 inf:1\n",
            "track a\nC4\nautomate cutoff\n500 R:nan\n",
            "track a\nC4\nautomate bpm\n120 nan:1\n",
        ] {
            assert!(parse_song(text).is_err(), "accepted {text:?}");
        }
        assert!(params_from_patch("cutoff nan\n").is_err());
        assert!(params_from_patch("cutoff inf\n").is_err());
        let mut vm = VoiceManager::new(44100.0, 4);
        assert!(crate::patch::apply(&mut vm, "resonance NaN\n").is_err());
    }

    #[test]
    fn scoped_automation_must_land_on_its_track() {
        let head = "track lead\nC4\ntrack beat kit=909\nBD\ntrack sing vox\nC4\n";
        // Voice-level, strip and board controls belong to the track
        for lane in [
            "lead.cutoff",
            "lead.unison",
            "lead.reverb_send",
            "beat.bd_tune",
            "sing.vox_mode",
        ] {
            let text = format!("{head}automate {lane}\n0.5\n");
            assert!(parse_song(&text).is_ok(), "{lane} refused");
        }
        // Bus circuits under every track would silently go global
        for lane in [
            "lead.noise",
            "lead.reverb_wet",
            "lead.spring",
            "lead.tape_wow",
            "lead.volume",
            "lead.bd_tune",
            "beat.vox_level",
            "lead.smp_pitch",
        ] {
            let text = format!("{head}automate {lane}\n0.5\n");
            assert!(parse_song(&text).is_err(), "{lane} accepted");
            let during = format!("{head}section A 0..1\nautomate {lane}: 0.5 during A\n");
            assert!(parse_song(&during).is_err(), "{lane} during accepted");
        }
        // Unscoped, the bus controls are fine
        assert!(parse_song(&format!("{head}automate noise\n0.1\n")).is_ok());
    }

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

    /// Note bends ride their note tokens and glide over the held length:
    /// a note to a note, a chord note by note, a whole chord by an
    /// interval, a fractional interval; each sorts right after its note-on.
    #[test]
    fn note_bends_ride_their_notes() {
        let song = parse_song(
            "bpm 120\ngate 1\ntrack a\nC4:2>G3 [C4>D4 G4]:1 [C4 E4]:1>-12 A3>+0.5:1 C4>C4\n",
        )
        .unwrap();
        let bends: Vec<(f64, u8, f32, f64)> = song
            .events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::Bend {
                    note,
                    semitones,
                    seconds,
                    ..
                } => Some((e.time, note, semitones, seconds)),
                _ => None,
            })
            .collect();
        let want = [
            (0.0, 60, -5.0, 1.0),
            (1.0, 60, 2.0, 0.5),
            (1.5, 60, -12.0, 0.5),
            (1.5, 64, -12.0, 0.5),
            (2.0, 57, 0.5, 0.5),
        ];
        assert_eq!(bends.len(), want.len(), "{bends:?}");
        for (got, want) in bends.iter().zip(want) {
            assert!((got.0 - want.0).abs() < 1e-9, "{got:?} vs {want:?}");
            assert_eq!((got.1, got.2), (want.1, want.2));
            assert!((got.3 - want.3).abs() < 1e-6, "{got:?} vs {want:?}");
        }
        // every bend follows its own note-on
        for (i, e) in song.events.iter().enumerate() {
            if let EventKind::Bend { note, channel, .. } = e.kind {
                let on = song.events[..i].iter().rev().find(|p| {
                    matches!(p.kind, EventKind::NoteOn { note: n, channel: c, .. } if n == note && c == channel)
                });
                assert!(on.map_or(false, |p| p.time == e.time), "bend before its note");
            }
        }
        // the plain token survives for the note parser
        let (plain, bends) = split_bends("[C4>D4 G4]:1@0.5").unwrap();
        assert_eq!(plain, "[C4 G4]:1@0.5");
        assert_eq!(bends.target(0), Some(BendTarget::Note(62)));
        assert_eq!(bends.target(1), None);
        for bad in [
            "track a\nR>C4\n",
            "track b kit=909\nBD>C4\n",
            "track a\n[C4>D4 E4]>E4\n",
            "track a\nC4>G3>A3\n",
            "track a\nC4>+60\n",
            "track a\nC4>H9\n",
        ] {
            assert!(parse_song(bad).is_err(), "should reject {bad:?}");
        }
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
        let events = parse_song("bpm 120\ntrack a vel=0.9\nC4 E4:1 | R:2 [C3 G3]:2\n")
            .unwrap()
            .events;
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
    fn tail_directive_sets_exact_offline_render_allowance() {
        let song = parse_song("bpm 60\ngate 1\ntail 0.25\ntrack a\nC4:1\n").unwrap();
        let mut frames = render_offline(&song, 48000.0);
        let total = (1.25 * 48000.0) as usize;
        assert_eq!(frames.len(), total);
        assert_eq!(frames.peak_voices(), 0);
        assert!(frames.next().is_some());
        assert_eq!(frames.peak_voices(), 1);
        assert_eq!(frames.len(), total - 1);
        assert_eq!(frames.by_ref().count(), total - 1);
        assert_eq!(frames.size_hint(), (0, Some(0)));
        assert_eq!(frames.next(), None);
        assert_eq!(frames.next(), None);
        assert!((song.tail_seconds - 0.25).abs() < f64::EPSILON);
        assert!(parse_song("tail -1\ntrack a\nC4\n").is_err());
    }

    #[test]
    fn automation() {
        let events = parse_song("bpm 60\ntrack a\nC4:8\nautomate cutoff\n400 R:2 8000:4@exp\n")
            .unwrap()
            .events;
        let params: Vec<(f64, f32)> = events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::Param {
                    param: Param::Cutoff,
                    value,
                    ..
                } => Some((e.time, value)),
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
    fn tempo_lane_can_accelerate_continuously_into_audio_rate() {
        let song = parse_song(
            "bpm 56\ngate 1\ntrack arp\n(C4:0.75 E4:0.75 G4:0.75 B4:0.75)x26\n\
             automate bpm\n56 21600:78@exp\n",
        )
        .unwrap();
        let onsets: Vec<_> = song
            .events
            .iter()
            .filter_map(|event| match event.kind {
                EventKind::NoteOn { .. } => Some(event.time),
                _ => None,
            })
            .collect();
        let gaps: Vec<_> = onsets.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert!(gaps.windows(2).all(|pair| pair[1] < pair[0]));
        assert!(gaps.first().unwrap() > &0.7);
        assert!(gaps.last().unwrap() < &0.003);
        assert!(parse_song("track a\nC4\nautomate bpm\n120 24001:1\n").is_err());
    }

    #[test]
    fn scoped_pitch_shift_automation_reaches_its_track() {
        let song = parse_song(
            "bpm 60\ntrack voice\nC4:4\nautomate voice.pitch_shift\n0 R:1 -12:2@smooth\n",
        )
        .unwrap();
        let spread: Vec<_> = song
            .events
            .iter()
            .filter_map(|event| match event.kind {
                EventKind::Param {
                    param: Param::PitchShift,
                    channel,
                    value,
                } => Some((channel, value)),
                _ => None,
            })
            .collect();
        assert!(!spread.is_empty());
        assert!(spread.iter().all(|(channel, _)| *channel == 1));
        assert!((spread.last().unwrap().1 + 12.0).abs() < 1e-6);
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
    /// suffixes emit VoxLead events ahead of their note-ons (the vowel
    /// owns the beat), and the whole path — DSL to voice box to vocoder
    /// to bus — makes sound.
    #[test]
    fn vox_tracks_sing() {
        use crate::vox::{Phoneme, VOX_CHANNEL};
        let song = parse_song(
            "bpm 120\n\
             track choir vox vel=0.9\n\
             R:1 [A2 E3]:2=HH-EH R:1 A2:1=S-IH-NG-Z:200@0.8\n",
        )
        .unwrap();
        let lyrics: Vec<_> = song
            .events
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::VoxLead {
                    syl,
                    note,
                    velocity,
                } => Some((e.time, &syl.phones, *note, *velocity)),
                _ => None,
            })
            .collect();
        assert_eq!(lyrics.len(), 2);
        assert_eq!(
            lyrics[0].2, 45,
            "the onset approaches the chord's lowest note (A2)"
        );
        assert!((lyrics[0].3 - 0.9).abs() < 1e-6);
        assert_eq!(lyrics[0].1[0].ph, Phoneme::HH);
        assert_eq!(lyrics[0].1[1].ph, Phoneme::EH);
        // per-phoneme overrides survive the trip
        let z = lyrics[1].1.last().unwrap();
        assert_eq!(z.ph, Phoneme::Z);
        assert_eq!(z.ms, Some(200.0));
        assert!((z.amp - 0.8).abs() < 1e-6);
        // the lyric leads its note-on by exactly the onset length
        let first_on = song
            .events
            .iter()
            .find(|e| matches!(e.kind, EventKind::NoteOn { .. }))
            .unwrap();
        let lead = crate::vox::onset_lead_ms(&crate::vox::parse_lyric("HH-EH").unwrap());
        // event times are seconds; constant tempo maps the lead exactly
        let expect = first_on.time - lead as f64 * 0.001;
        assert!(
            (lyrics[0].0 - expect).abs() < 1e-9 && lead > 0.0,
            "VoxLead at {} should lead the note-on at {} by {} ms",
            lyrics[0].0,
            first_on.time,
            lead
        );
        assert!(
            matches!(first_on.kind, EventKind::NoteOn { channel, .. } if channel == VOX_CHANNEL)
        );

        // lyric grammar errors
        assert!(
            parse_song("bpm 120\ntrack a\nC4=AA\n").is_err(),
            "lyrics need a vox track"
        );
        assert!(
            parse_song("bpm 120\ntrack a vox\nR:2=AA\n").is_err(),
            "no lyric on a rest"
        );
        assert!(
            parse_song("bpm 120\ntrack a vox\nC4=QX\n").is_err(),
            "unknown phoneme"
        );
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
        let frames: Vec<_> = render_offline(&song, 48000.0).collect();
        let peak = frames
            .iter()
            .fold(0.0f32, |a, &(l, r)| a.max(l.abs()).max(r.abs()));
        assert!(peak > 0.05, "the choir should be audible, peak={peak}");
        assert!(frames.iter().all(|&(l, r)| l.is_finite() && r.is_finite()));
    }

    /// The vowel owns the beat: a lyric's onset consonants are scheduled
    /// ahead of the note (VoxLead), so the sung nucleus opens the
    /// vocoder AT the note-on, with the onset audible before it.
    #[test]
    fn vox_vowel_lands_on_the_beat() {
        let sr = 48000.0;
        let song = parse_song(
            "bpm 120\n\
             track choir vox\n\
             R:2 [A2]:4=S-T-R-AA\n",
        )
        .unwrap();
        // The lead event must sit before the beat-2 note-on
        let lead = song
            .events
            .iter()
            .find(|e| matches!(e.kind, EventKind::VoxLead { .. }))
            .expect("a lyric compiles to a VoxLead event");
        assert!(
            lead.time < 1.0 && lead.time > 0.5,
            "VoxLead fires ahead of its 1.0 s note-on: {} s",
            lead.time
        );
        let frames: Vec<_> = render_offline(&song, sr).collect();
        let mono: Vec<f32> = frames.iter().map(|&(l, r)| (l + r) * 0.5).collect();
        // 120 bpm: the note-on is at 1.0 s
        let win = |a: f32, b: f32| {
            let s = &mono[(a * sr) as usize..(b * sr) as usize];
            (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
        };
        let vowel = win(1.05, 1.6);
        assert!(
            win(1.0, 1.06) > 0.5 * vowel,
            "the nucleus must be open right at the beat: {} vs vowel {}",
            win(1.0, 1.06),
            vowel
        );
        assert!(
            win(0.8, 0.99) > 0.02 * vowel,
            "the onset consonants must speak before the beat, rms={}",
            win(0.8, 0.99)
        );
        assert!(
            win(0.2, 0.7) < 0.05 * vowel,
            "silence before the syllable starts, rms={}",
            win(0.2, 0.7)
        );
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

    /// The throat options parse into the slot, and a smp_formant lane on a
    /// slot with none of them still reaches it: the lane alone makes the
    /// slot a formant slot at load, so +12 moves its sound up.
    #[test]
    fn throat_options_and_lanes_reach_the_slot() {
        let wav = write_test_wav("patina-throat-test.wav");
        let song = parse_song(&format!(
            "track a sample={wav} formant\nC4\n\
             track b sample={wav} formant=-5 formant_keep=0.5\nC4\n\
             track c sample={wav}\nC4\n"
        ))
        .unwrap();
        let cfg = |i: usize| song.samplers[i].cfg;
        assert!(cfg(0).formant && cfg(0).formant_keep == 1.0 && cfg(0).formant_semis == 0.0);
        assert!(cfg(1).formant && cfg(1).formant_keep == 0.5 && cfg(1).formant_semis == -5.0);
        assert!(!cfg(2).formant);
        let centroid = |lane: &str| {
            let song = parse_song(&format!(
                "track keys sample={wav} root=C4 loop\nC4:2\nautomate keys.{lane}\n"
            ))
            .unwrap();
            let x: Vec<f32> = render_offline(&song, 48000.0).map(|(l, _)| l).collect();
            let seg = &x[24000..24000 + 4096];
            let (mut num, mut den) = (0.0f64, 0.0f64);
            for k in 1..400 {
                let f = k as f64 * 20.0;
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (i, &v) in seg.iter().enumerate() {
                    let ph = std::f64::consts::TAU * f * i as f64 / 48000.0;
                    re += v as f64 * ph.cos();
                    im += v as f64 * ph.sin();
                }
                num += f * (re * re + im * im);
                den += re * re + im * im;
            }
            num / den
        };
        let (plain, moved) = (centroid("smp_gain\n0.8"), centroid("smp_formant\n12"));
        assert!(moved > 1.4 * plain, "a +12 throat lane: centroid {plain:.0} -> {moved:.0} Hz");
    }

    /// The glitch lanes follow the song's tempo: at 150 bpm a track's
    /// 1/16 stutter repeats every 0.1 s (a rising sine, which never
    /// repeats by itself, correlates with itself one repeat back), and an
    /// unscoped tape stop over a beat leaves the whole mix silent.
    #[test]
    fn glitch_lanes_follow_the_tempo_on_a_track_and_the_bus() {
        let render = |lanes: &str| -> Vec<f32> {
            let song = parse_song(&format!(
                "bpm 150\ngate 1\nautomate reverb_wet\n0\n\
                 track a\nA3:4>A5\nautomate a.waveform\n0\n{lanes}"
            ))
            .unwrap();
            render_offline(&song, 48000.0).map(|(l, _)| l).collect()
        };
        // normalized correlation of x[k] and x[k - lag] over a window
        let corr = |x: &[f32], at: usize, lag: usize| {
            let (mut xy, mut xx, mut yy) = (0.0f64, 0.0f64, 0.0f64);
            for k in at..at + 9600 {
                let (a, b) = (x[k] as f64, x[k - lag] as f64);
                xy += a * b;
                xx += a * a;
                yy += b * b;
            }
            xy / (xx * yy).sqrt().max(1e-30)
        };
        // beat 1 is 0.4 s (sample 19200); a 1/16 is 4800 samples
        let plain = render("");
        let stut = render("automate a.stutter\n0 >1 16\n");
        assert!(corr(&plain, 30000, 4800) < 0.5, "the sweep alone repeats");
        assert!(
            corr(&stut, 30000, 4800) > 0.99,
            "a 1/16 stutter at 150 bpm: correlation {:.3} one repeat back",
            corr(&stut, 30000, 4800)
        );
        // the bus: a one-beat stop from beat 1 is over by beat 2 (0.8 s)
        let rms = |x: &[f32], a: usize, b: usize| {
            (x[a..b].iter().map(|v| (v * v) as f64).sum::<f64>() / (b - a) as f64).sqrt()
        };
        let stop = render("automate tape_stop\n0 >1 1\n");
        assert!(rms(&plain, 42000, 72000) > 0.01);
        assert!(rms(&stop, 42000, 72000) < 1e-4, "a stopped bus: rms {:.6}", rms(&stop, 42000, 72000));
        // before it engages, the mix is untouched
        assert_eq!(&stop[..19000], &plain[..19000]);
        // a reverse lane's opening 0 sets the reverser listening, so its
        // first slice (beat 1 to 2) is beat 0 to 1 played backwards
        let rev = render("automate a.reverse\n0 >1 4\n");
        let (mut xy, mut xx, mut yy) = (0.0f64, 0.0f64, 0.0f64);
        for u in 500..18700 {
            let (a, b) = (rev[19200 + u] as f64, plain[19199 - u] as f64);
            xy += a * b;
            xx += a * a;
            yy += b * b;
        }
        let c = xy / (xx * yy).sqrt().max(1e-30);
        assert!(c > 0.95, "the first reversed slice against beat 0-1 backwards: {c:.3}");
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

        let frames: Vec<_> = render_offline(&song, 48000.0).collect();
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
        assert!(
            (cfg.speed - 1.0).abs() < 0.02,
            "beats fit speed {}",
            cfg.speed
        );

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
        let events = parse_song("bpm 120\ntrack a\nF#4 [G2 F#3]:2 # a comment\n")
            .unwrap()
            .events;
        let ons = events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::NoteOn { .. }))
            .count();
        assert_eq!(ons, 3);
    }

    /// Cards rendered on several threads and summed in voice order must be
    /// the serial render, sample for sample: the pool is a speedup, never a
    /// different instrument.
    #[test]
    fn parallel_cards_are_bit_identical() {
        let song = parse_song(
            "bpm 120\ntail 0.5\n\
             track pad patch=tidewater\n[C3 E3 G3 B3]:4 [F3 A3 C4]:4\n\
             track lead patch=init echo_send=0.3\n(C5:0.5 G4:0.5 E5:1)x4\n\
             automate cutoff\n400 3000:8@exp\n",
        )
        .unwrap();
        let serial: Vec<(f32, f32)> = render_offline(&song, 48000.0).collect();
        let parallel: Vec<(f32, f32)> =
            render_offline(&song, 48000.0).with_card_lanes(3).collect();
        assert_eq!(serial.len(), parallel.len());
        for (i, (s, p)) in serial.iter().zip(&parallel).enumerate() {
            assert!(
                s.0.to_bits() == p.0.to_bits() && s.1.to_bits() == p.1.to_bits(),
                "sample {i}: serial {s:?} vs parallel {p:?}"
            );
        }
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
                EventKind::Param {
                    param: Param::Cutoff,
                    value,
                    ..
                } => Some((e.time, value)),
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
        assert!(
            matches!(p.osc2_wave, Waveform::Sine),
            "macro sets all three"
        );
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
        // patch track -> channel 1; the bare pad now gets its own
        // private channel too (2) instead of squatting on the panel
        assert_eq!(song.channels.len(), 2, "patch channel + bare-track channel");
        // lead notes carry channel 1, pad notes channel 0
        let lead_on = song.events.iter().any(|e| {
            matches!(
                e.kind,
                EventKind::NoteOn {
                    note: 72,
                    channel: 1,
                    ..
                }
            )
        });
        let pad_on = song.events.iter().any(|e| {
            matches!(
                e.kind,
                EventKind::NoteOn {
                    note: 48,
                    channel: 2,
                    ..
                }
            )
        });
        assert!(lead_on && pad_on);
        // dotted automation tagged to channel 1, plain to channel 0
        let tagged = song.events.iter().any(|e| {
            matches!(
                e.kind,
                EventKind::Param {
                    param: Param::Cutoff,
                    channel: 1,
                    ..
                }
            )
        });
        let global = song.events.iter().any(|e| {
            matches!(
                e.kind,
                EventKind::Param {
                    param: Param::Cutoff,
                    channel: 0,
                    ..
                }
            )
        });
        assert!(tagged && global);
        // unknown track name in dotted automation is an error
        assert!(parse_song("track a\nC4\nautomate ghost.cutoff\n400\n").is_err());
    }

    /// Velocity response and mono mode are voice-level patch lines: the
    /// neutral base keeps the original response, a patch sets them, a
    /// saved patch carries them, and per-track automation reaches them.
    #[test]
    fn velocity_and_mono_are_patch_lines() {
        let p = params_from_patch("").unwrap();
        assert_eq!((p.vel_amp, p.vel_filter, p.mono), (0.7, 0.8, false));
        let p = params_from_patch("vel_amp 0.25\nvel_filter 2.5\nmono 1\n").unwrap();
        assert_eq!((p.vel_amp, p.vel_filter, p.mono), (0.25, 2.5, true));
        let back = params_from_patch(&crate::patch::serialize(&p)).unwrap();
        assert_eq!(
            (back.vel_amp, back.vel_filter, back.mono),
            (0.25, 2.5, true)
        );

        let song = parse_song(
            "bpm 120\n\
             track lead\n\
             C4 D4\n\
             automate lead.mono\n\
             1\n\
             automate lead.vel_filter\n\
             2\n",
        )
        .unwrap();
        let lead = song.tracks.iter().find(|(n, _)| n == "lead").unwrap().1;
        for want in [Param::MonoSel, Param::VelFilter] {
            assert!(
                song.events.iter().any(|e| matches!(
                    e.kind,
                    EventKind::Param { param, channel, .. } if param == want && channel == lead
                )),
                "`automate lead.{}` did not land on the track",
                want.name()
            );
        }
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
        // Another kit name is refused, not ignored
        assert!(parse_song("bpm 120\ntrack beat kit=808\nBD\n").is_err());

        let frames: Vec<_> = render_offline(&song, 48000.0).collect();
        let peak = frames
            .iter()
            .fold(0.0f32, |a, &(l, r)| a.max(l.abs()).max(r.abs()));
        assert!(peak > 0.05, "the beat should be audible, peak={peak}");
    }

    /// Drum knobs are ordinary parameters: unscoped automation lands on
    /// the live panel (and every kit's board) like a patch line or a knob.
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

    /// One stem's samples, bit for bit: `channel` soloed through the
    /// full engine.
    fn stem_bits(song: &Song, channel: u16) -> Vec<(u32, u32)> {
        render_offline_solo(song, 48000.0, Some(channel))
            .map(|(l, r)| (l.to_bits(), r.to_bits()))
            .collect()
    }

    fn channel_of(song: &Song, track: &str) -> u16 {
        song.tracks.iter().find(|(t, _)| t == track).unwrap().1
    }

    /// Every `kit=909` track is a 909 board of its own: the first on the
    /// drum channel, the next on the kit block. A track's drum knob
    /// automation, pan and level reach its own board and strip and no
    /// other: track b's hat tuning leaves track a's stem bit-identical,
    /// while unscoped automation still turns every board.
    #[test]
    fn each_kit_track_owns_its_board_and_strip() {
        let song = |a_tune: f32, b_tune: f32, b_gain: f32, unscoped: Option<f32>| {
            let mut text = format!(
                "bpm 120\ntail 0.5\n\
                 track a kit=909 len=0.25 pan=-0.8\nR (CH CH OH CH)x4\n\
                 track b kit=909 len=0.25 pan=0.8 gain={b_gain}\nR (CH CH OH CH)x4\n\
                 automate a.hh_tune\n{a_tune}\n\
                 automate b.hh_tune\n{b_tune}\n"
            );
            if let Some(v) = unscoped {
                text += &format!("automate hh_tune\n{v}\n");
            }
            parse_song(&text).unwrap()
        };
        let base = song(0.2, 0.2, 1.0, None);
        let (a, b) = (channel_of(&base, "a"), channel_of(&base, "b"));
        assert_eq!(a, crate::drums::DRUM_CHANNEL, "the first kit plays board 0");
        assert_eq!(
            b,
            crate::drums::kit_channel(1),
            "the second kit has a board of its own"
        );
        // Each track's notes and knob lane carry its own channel
        for e in &base.events {
            match e.kind {
                EventKind::Param {
                    param: Param::HhTune,
                    channel,
                    ..
                } => {
                    assert!(channel == a || channel == b)
                }
                EventKind::NoteOn { channel, .. } => assert!(channel == a || channel == b),
                _ => {}
            }
        }
        let retuned_b = song(0.2, 0.9, 1.0, None);
        let retuned_a = song(0.9, 0.2, 1.0, None);
        assert!(
            stem_bits(&base, a) == stem_bits(&retuned_b, a),
            "b.hh_tune reached track a's board"
        );
        assert!(
            stem_bits(&base, b) != stem_bits(&retuned_b, b),
            "b.hh_tune did not retune track b"
        );
        assert!(
            stem_bits(&base, a) != stem_bits(&retuned_a, a),
            "a.hh_tune did not retune track a"
        );
        assert!(
            stem_bits(&base, b) == stem_bits(&retuned_a, b),
            "a.hh_tune reached track b's board"
        );
        // Two kits tuned apart render a different mix than the swap
        let mix = |s: &Song| -> Vec<(u32, u32)> {
            render_offline(s, 48000.0)
                .map(|(l, r)| (l.to_bits(), r.to_bits()))
                .collect()
        };
        assert!(mix(&retuned_a) != mix(&retuned_b));
        // Unscoped hh_tune turns both boards (it lands after the scoped
        // lanes at beat 0, so both stems move)
        let global = song(0.2, 0.2, 1.0, Some(0.9));
        assert!(stem_bits(&base, a) != stem_bits(&global, a));
        assert!(stem_bits(&base, b) != stem_bits(&global, b));
        // Pan and level are the track's own: a sits left, b right, and
        // b's gain=0 silences b alone
        let balance = |bits: &[(u32, u32)]| {
            bits.iter().fold((0.0f64, 0.0f64), |(l, r), &(x, y)| {
                let (x, y) = (f32::from_bits(x) as f64, f32::from_bits(y) as f64);
                (l + x * x, r + y * y)
            })
        };
        let (al, ar) = balance(&stem_bits(&base, a));
        let (bl, br) = balance(&stem_bits(&base, b));
        assert!(al > 4.0 * ar, "track a should sit left: L {al} R {ar}");
        assert!(br > 4.0 * bl, "track b should sit right: L {bl} R {br}");
        let muted = song(0.2, 0.2, 0.0, None);
        let (ml, mr) = balance(&stem_bits(&muted, b));
        assert!(
            ml + mr < (bl + br) * 1e-4,
            "gain=0 must silence track b: {}",
            ml + mr
        );
        assert!(
            stem_bits(&muted, a) == stem_bits(&base, a),
            "b's gain reached track a"
        );
    }

    /// Board 0 is the live panel's board: a knob addressed to the first
    /// kit shows on the panel, one addressed to a later kit does not, and
    /// an unaddressed knob moves the panel and every board.
    #[test]
    fn kit_knobs_reach_the_panel_only_from_board_zero() {
        let mut vm = VoiceManager::new(48000.0, 4);
        let second = crate::drums::kit_channel(1);
        vm.add_kit(second);
        vm.set_channel_param(second, Param::BdTune, 0.9);
        assert_eq!(
            vm.params.bd_tune, 0.35,
            "a later kit's knob moved the panel"
        );
        vm.set_channel_param(crate::drums::DRUM_CHANNEL, Param::BdTune, 0.7);
        assert_eq!(vm.params.bd_tune, 0.7);
        Param::BdTune.apply(&mut vm, 0.1);
        assert_eq!(vm.params.bd_tune, 0.1);
        assert_eq!(vm.kit_boards(), 2);
    }

    /// A one-kit song is the old single shared board: its kit plays board
    /// 0 on the drum channel, no further board is built, and the render is
    /// bit-identical to sending all its drum-knob automation unscoped (on
    /// the shared board, scoped and unscoped were the same knob).
    #[test]
    fn a_one_kit_song_renders_as_the_one_shared_board() {
        let text = "bpm 118\ntail 1\n\
             track bass len=0.5 duck=0.6\n(C2 C2 Eb2 G2)x4\n\
             track beat kit=909 len=0.25 gain=0.8 pan=0.3 reverb_send=0.35 echo_send=0.2 comp=-18\n\
             (BD CH CH CH SD CH [CH BD] OH RS CH CH CP SD CH OH CH)x2\n\
             automate beat.bd_tune\n0.2 0.8:8@lin\n\
             automate beat.oh_decay\n0.3 0.9:8@smooth\n\
             automate beat.pan\n0.3 -0.3:8@lin\n\
             automate dr_drive\n0.0 0.6:8@lin\n";
        let song = parse_song(text).unwrap();
        assert_eq!(channel_of(&song, "beat"), crate::drums::DRUM_CHANNEL);
        let mut vm = VoiceManager::new(48000.0, 8);
        register_channels(&mut vm, &song);
        assert_eq!(vm.kit_boards(), 1, "a one-kit song built a second board");
        let unscoped = parse_song(
            &text
                .replace("automate beat.bd_tune", "automate bd_tune")
                .replace("automate beat.oh_decay", "automate oh_decay"),
        )
        .unwrap();
        let mix = |s: &Song| -> Vec<(u32, u32)> {
            render_offline(s, 48000.0)
                .map(|(l, r)| (l.to_bits(), r.to_bits()))
                .collect()
        };
        let scoped = mix(&song);
        assert!(scoped.iter().any(|&(l, _)| f32::from_bits(l).abs() > 1e-3));
        assert!(
            scoped == mix(&unscoped),
            "the one kit no longer sounds like the shared board"
        );
    }

    /// What the old shared board did, `kit=<track>` still does: a sharing
    /// track's strip options land on the one shared strip (wherever they
    /// are written, the render is the same), and a closed hat on one track
    /// chokes the open hat ringing on the other. On separate boards the
    /// open hat rings on through the closed hit.
    #[test]
    fn a_shared_board_keeps_the_strip_and_the_hat_choke() {
        let mix = |text: &str| -> Vec<(u32, u32)> {
            render_offline(&parse_song(text).unwrap(), 48000.0)
                .map(|(l, r)| (l.to_bits(), r.to_bits()))
                .collect()
        };
        let on_second = mix("bpm 120\ntail 0.5\ntrack a kit=909\nBD R\n\
                             track b kit=a reverb_send=0.3 pan=0.4\nR SD\n");
        let on_first = mix(
            "bpm 120\ntail 0.5\ntrack a kit=909 reverb_send=0.3 pan=0.4\nBD R\n\
             track b kit=a\nR SD\n",
        );
        assert!(
            on_second == on_first,
            "a sharing track's strip options missed the shared strip"
        );
        // A long open hat at 0 on one track, a short closed hat at 0.1 s
        // on another, the reverb dry: 0.2-0.5 s holds whatever the open
        // hat still rings
        let tail = |second: &str| {
            let song = parse_song(&format!(
                "bpm 120\ntail 0.5\nautomate reverb_wet\n0\nautomate oh_decay\n1\n\
                 automate ch_decay\n0\ntrack open kit=909\nOH:2\n\
                 track closed {second}\nR:0.2 CH:1.8\n"
            ))
            .unwrap();
            render_offline(&song, 48000.0)
                .skip(9600)
                .take(14400)
                .map(|(l, r)| (l as f64).powi(2) + (r as f64).powi(2))
                .sum::<f64>()
        };
        let (choked, ringing) = (tail("kit=open"), tail("kit=909"));
        assert!(
            ringing > 100.0 * choked,
            "the closed hat should choke the shared board's open hat: \
             choked {choked:.3e} vs separate boards {ringing:.3e}"
        );
    }

    /// `kit=<track>` plays an earlier kit track's board: same channel (so
    /// the closed hat on one track still chokes the open hat on the
    /// other), one strip, one stem. It must name a kit track above.
    #[test]
    fn kit_names_an_earlier_kit_track_to_share_its_board() {
        let song = parse_song(
            "bpm 120\ntrack hats kit=909 len=0.25\nCH CH CH CH\n\
             track lead\nC4\n\
             track open kit=hats len=0.5\nR OH\n\
             track perc kit=909\nRS\n",
        )
        .unwrap();
        assert_eq!(channel_of(&song, "open"), channel_of(&song, "hats"));
        assert_eq!(channel_of(&song, "perc"), crate::drums::kit_channel(1));
        assert!(
            parse_song("bpm 120\ntrack open kit=hats\nOH\n").is_err(),
            "no such track"
        );
        assert!(
            parse_song("bpm 120\ntrack lead\nC4\ntrack open kit=lead\nOH\n").is_err(),
            "lead is not a kit track"
        );
        // Sixteen boards, then a clear refusal
        let many: String = (0..crate::drums::MAX_KITS)
            .map(|i| format!("track k{i} kit=909\nBD\n"))
            .collect();
        assert!(parse_song(&format!("bpm 120\n{many}")).is_ok());
        let err = parse_song(&format!("bpm 120\n{many}track one_more kit=909\nBD\n"))
            .err()
            .unwrap();
        assert!(err.contains("too many kit=909 tracks"), "{err}");
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
            .map(|i| {
                if (i as f32 * 200.0 / 48000.0) % 1.0 < 0.5 {
                    0.8
                } else {
                    -0.8
                }
            })
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
        let frames: Vec<_> = render_offline(&song, 48000.0).collect();
        let before = frames_rms(&frames, 0.0, 1.9, 48000.0);
        let after = frames_rms(&frames, 2.0, 2.3, 48000.0);
        assert!(
            after > 0.01,
            "the recording should speak at its beat, rms={after}"
        );
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
            .map(|i| {
                if (i as f32 * 200.0 / 48000.0) % 1.0 < 0.5 {
                    0.8
                } else {
                    -0.8
                }
            })
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
        assert!(
            (samples[100] - 62.0).abs() < 1e-3,
            "values pass through unnormalized"
        );

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
        assert_eq!(
            ons,
            vec![(4.0, 60), (6.0, 62)],
            "section seeks land on its edges"
        );
        let modes: Vec<(f64, f32)> = song
            .events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::Param {
                    param: Param::VoxModeSel,
                    value,
                    ..
                } => Some((e.time, value)),
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
                if let Some(got) = def.param.read(&vm.params) {
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
        assert!(matches!(
            vm.params.circuit,
            crate::oscillator::CircuitModel::Arp
        ));
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
        let frames: Vec<_> = render_offline(&song, 48000.0).collect();
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

    #[test]
    fn bare_tracks_get_private_channels() {
        // Two bare synth tracks must not share the live panel (channel
        // 0) — per-track automation must land on a private channel.
        let src = "bpm 120\ntrack a vel=0.8\n  C4:1\ntrack b vel=0.8\n  E4:1\n";
        let dir = std::env::temp_dir().join("patina_bare_ch_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.song");
        std::fs::write(&path, src).unwrap();
        let song = load_song(path.to_str().unwrap()).unwrap();
        let mut chans: Vec<u16> = song
            .events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::NoteOn { channel, .. } => Some(channel),
                _ => None,
            })
            .collect();
        chans.sort_unstable();
        chans.dedup();
        assert_eq!(chans.len(), 2, "two tracks, two channels: {chans:?}");
        assert!(
            !chans.contains(&0),
            "no bare track may sit on the live panel"
        );
        assert_eq!(song.channels.len(), 2);
    }

    #[test]
    fn sampler_tracks_have_independent_strips() {
        // Two sample tracks; the second at gain=0 must be silent while
        // the first sounds — per-slot strips, not one strip per deck.
        let dir = std::env::temp_dir().join("patina_slotstrip_test");
        std::fs::create_dir_all(&dir).unwrap();
        // a tiny mono wav: 0.2s of 440 Hz
        let n = 9600usize;
        let mut bytes: Vec<u8> = Vec::new();
        let data_len = (n * 2) as u32;
        bytes.extend(b"RIFF");
        bytes.extend((36 + data_len).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48000u32.to_le_bytes());
        bytes.extend(96000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(data_len.to_le_bytes());
        for i in 0..n {
            let s = (std::f32::consts::TAU * 440.0 * i as f32 / 48000.0).sin();
            bytes.extend(((s * 20000.0) as i16).to_le_bytes());
        }
        let wav = dir.join("tone.wav");
        std::fs::write(&wav, bytes).unwrap();
        let song_src = format!(
            "bpm 120\ntrack a sample={p} root=C3 mode=oneshot gain=1\n  C3:1\n\
             track b sample={p} root=C3 mode=oneshot gain=0\n  C3:1\n",
            p = wav.display()
        );
        let sp = dir.join("t.song");
        std::fs::write(&sp, song_src).unwrap();
        let song = load_song(sp.to_str().unwrap()).unwrap();
        let mut vm = crate::voice_manager::VoiceManager::new(48000.0, 8);
        register_channels(&mut vm, &song);
        // fire both notes and render a beat; then solo channel of track b
        use crate::sampler::SAMPLER_CHANNEL_BASE;
        for e in &song.events {
            if e.time <= 0.0 {
                dispatch(&mut vm, &e.kind);
            }
        }
        for (slot, _) in song.samplers.iter().enumerate() {
            vm.sampler.note_on(slot, 48, 1.0);
        }
        let mut a_rms = 0.0f64;
        for _ in 0..24000 {
            let (l, _r) = vm.render_next();
            a_rms += (l as f64) * (l as f64);
        }
        let combined = (a_rms / 24000.0).sqrt();
        assert!(combined > 1e-4, "track a must sound: {combined}");
        // now solo track b's channel on a FRESH engine (no reverb tail
        // from phase one): must be (near) silent
        let mut vm = crate::voice_manager::VoiceManager::new(48000.0, 8);
        register_channels(&mut vm, &song);
        for e in &song.events {
            if e.time <= 0.0 {
                dispatch(&mut vm, &e.kind);
            }
        }
        vm.set_solo(Some(SAMPLER_CHANNEL_BASE + 1));
        for (slot, _) in song.samplers.iter().enumerate() {
            vm.sampler.note_on(slot, 48, 1.0);
        }
        let mut b_rms = 0.0f64;
        for _ in 0..24000 {
            let (l, _r) = vm.render_next();
            b_rms += (l as f64) * (l as f64);
        }
        let solo_b = (b_rms / 24000.0).sqrt();
        // a third engine, soloing track a (gain=1) for the reference:
        // idle voice-card bleed (-60 dB VCA floor, volts) is common to
        // both solos, so the RATIO is the honest strip test
        let mut vm = crate::voice_manager::VoiceManager::new(48000.0, 8);
        register_channels(&mut vm, &song);
        for e in &song.events {
            if e.time <= 0.0 {
                dispatch(&mut vm, &e.kind);
            }
        }
        vm.set_solo(Some(SAMPLER_CHANNEL_BASE));
        for (slot, _) in song.samplers.iter().enumerate() {
            vm.sampler.note_on(slot, 48, 1.0);
        }
        let mut a2 = 0.0f64;
        for _ in 0..24000 {
            let (l, _r) = vm.render_next();
            a2 += (l as f64) * (l as f64);
        }
        let solo_a = (a2 / 24000.0).sqrt();
        assert!(
            solo_b < solo_a * 0.35,
            "gain=0 track must be far quieter than gain=1 track through \
             per-slot strips: b={solo_b} a={solo_a}"
        );
    }


    /// The peak of each of a render's hits, `every` seconds apart.
    fn hit_peaks(frames: &[(f32, f32)], every: f64, hits: usize) -> Vec<f32> {
        let w = (every * 48000.0) as usize;
        (0..hits)
            .map(|k| {
                frames[k * w..((k + 1) * w).min(frames.len())]
                    .iter()
                    .fold(0f32, |m, &(l, r)| m.max(l.abs()).max(r.abs()))
            })
            .collect()
    }

    /// hypha's report: a kit track's `gain` barely moved its level
    /// (G=0 -14.5 dBFS, G=0.2 -14.4, G=1 -13.1). The fader itself was
    /// linear; the strip was born at unity and glided to G over ~26 ms,
    /// and a drum's peak lives in its first millisecond, so the first hit
    /// always played at full level and set the song's peak. A strip set
    /// before its channel's first note now starts where it is set.
    #[test]
    fn kit_gain_is_a_true_fader_from_the_first_hit() {
        let take = |gain: &str| {
            let text = format!(
                "automate reverb_wet\n0\nautomate echo\n0\n\
                 track s kit=909 gain={gain}\n(SD:0.25@0.8)x8\n"
            );
            let song = parse_song(&text).unwrap();
            let frames: Vec<(f32, f32)> = render_offline(&song, 48000.0).collect();
            hit_peaks(&frames, 0.125, 8)
        };
        let unity = take("1");
        let db = |x: f32, r: f32| 20.0 * (x / r).log10();
        for (g, want) in [("0.2", -13.98f32), ("0.5", -6.02)] {
            let peaks = take(g);
            for (k, (&p, &u)) in peaks.iter().zip(&unity).enumerate() {
                assert!(
                    (db(p, u) - want).abs() < 0.3,
                    "gain={g}, hit {k}: {:.2} dB from unity, want {want}",
                    db(p, u)
                );
            }
        }
        // 0 mutes, the first hit included
        assert!(take("0").iter().all(|&p| p == 0.0), "gain=0 must mute");
        // and the lane form: a ramp from 0.2 to 2 spans 20 dB, first hit on
        let text = "automate reverb_wet\n0\nautomate echo\n0\ntrack s kit=909\n\
                    (SD:0.25@0.8)x17\nautomate s.gain\n0.2 2:4\n";
        let frames: Vec<(f32, f32)> =
            render_offline(&parse_song(text).unwrap(), 48000.0).collect();
        let peaks = hit_peaks(&frames, 0.125, 17);
        let span = db(peaks[16], peaks[0]);
        // the master's tanh soft limit takes ~0.5 dB off the loudest hit
        assert!(span > 19.0, "a 0.2 -> 2 gain lane moved the hits {span:.1} dB");
    }

    /// A strip first set after its channel has played still glides there
    /// from unity, as it always did: no step under a sounding note.
    #[test]
    fn a_fader_set_mid_note_still_glides() {
        let text = "track a\nC4:4\nautomate a.gain\n>1 0.25\n";
        let song = parse_song(text).unwrap();
        let frames: Vec<(f32, f32)> = render_offline(&song, 48000.0).collect();
        // 1 beat at 120 bpm = 0.5 s; for the first 8 ms after the move
        // the strip is still near unity, not already at a quarter
        let before = frames_rms(&frames, 0.40, 0.5, 48000.0);
        let just_after = frames_rms(&frames, 0.5, 0.508, 48000.0);
        let later = frames_rms(&frames, 0.7, 0.75, 48000.0);
        assert!(just_after > 0.6 * before, "the fader stepped: {just_after} vs {before}");
        assert!(later < 0.35 * before, "the fader never arrived: {later} vs {before}");
    }

    /// The range is 0..4 now (+12 dB), and anything outside a parameter's
    /// range is clamped with a warning that names the line.
    #[test]
    fn out_of_range_values_are_clamped_with_a_warning() {
        let text = "track a gain=5 pan=-3\nC4\nautomate a.gain\n0 6:4 3.8:2\n\
                    automate detune\n40\nautomate cutoff\n90000\n";
        let song = parse_song(text).unwrap();
        assert_eq!(
            song.warnings,
            vec![
                "line 1: gain 5 clamped to 4",
                "line 1: pan -3 clamped to -1",
                "line 4: gain 6 clamped to 4",
                "line 8: cutoff 90000 clamped to 20000",
            ],
            "detune 40 is inside its engine range (0..50) and stays quiet"
        );
        for e in &song.events {
            if let EventKind::Param { param, value, .. } = e.kind {
                let (lo, hi) = param.accepts();
                assert!((lo..=hi).contains(&value), "{} {}", param.name(), value);
            }
        }
        // a 3.8 swell reaches 3.8, not the old ceiling of 2
        let top = song
            .events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::Param {
                    param: Param::TrackGain,
                    value,
                    ..
                } => Some(value),
                _ => None,
            })
            .last()
            .unwrap();
        assert!((top - 3.8).abs() < 1e-6, "{top}");
        // sampler options warn the same way
        let wav = write_test_wav("clamp_warn");
        let song = parse_song(&format!("track k sample={wav} res=3\nC4\n")).unwrap();
        assert_eq!(song.warnings, vec!["line 1: res 3 clamped to 1"]);
        // values that are not table parameters but are clamped all the same:
        // the gate, a track's vel=, a note's @ velocity, sampler options
        let song = parse_song(&format!(
            "gate 1.5\ntrack a vel=1.3\nC4 D4@1.2 E4@0.5\ntrack k sample={wav} xfade=3 vel_amt=-1\nC4\n"
        ))
        .unwrap();
        assert_eq!(
            song.warnings,
            vec![
                "line 1: gate 1.5 clamped to 1",
                "line 2: vel 1.3 clamped to 1",
                "line 3: token 'D4@1.2': velocity 1.2 clamped to 1",
                "line 4: xfade 3 clamped to 2",
                "line 4: vel_amt -1 clamped to 0",
            ]
        );
        let a = channel_of(&song, "a");
        let vels: Vec<f32> = song
            .events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::NoteOn {
                    velocity, channel, ..
                } if channel == a => Some(velocity),
                _ => None,
            })
            .collect();
        assert_eq!(vels, vec![1.0, 1.0, 0.5]);
    }

    /// The five 909 voice levels reach 4 (+12 dB) from a song, while a
    /// knob (a MIDI controller, the plugin's host parameter, the app's
    /// panel) still turns the stock 0..1: CC 55 at full is hh_level 1.
    #[test]
    fn drum_levels_reach_four_in_a_song_but_a_knob_turns_the_panel() {
        for p in [Param::BdLevel, Param::SdLevel, Param::RsLevel, Param::CpLevel, Param::HhLevel] {
            assert_eq!(p.range().1, 4.0, "{}", p.name());
            assert_eq!(p.midi_value(1.0), 1.0, "{}", p.name());
            assert_eq!(p.knob_range().1, 1.0, "{}", p.name());
        }
        let song = parse_song("track d kit=909\nCH\nautomate d.hh_level\n4\n").unwrap();
        assert!(song.warnings.is_empty(), "{:?}", song.warnings);
    }

    /// Generated scores round tiny rests to `R:0`; those play as nothing.
    #[test]
    fn zero_length_rests_are_nothing() {
        let song = parse_song("track a\nC4 R:0 D4 .:0.0000004 R:-0.0000001 E4\n").unwrap();
        let ons: Vec<f64> = song
            .events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::NoteOn { .. }))
            .map(|e| e.time)
            .collect();
        assert_eq!(ons, vec![0.0, 0.5, 1.0]);
        assert!(parse_song("track a\nC4\nautomate cutoff\n500 R:0 800:1\n").is_ok());
        assert!(parse_song("bpm 100\nautomate bpm\n100 R:0 90:4\ntrack a\nC4\n").is_ok());
        // real negative rests and zero-length notes are still errors
        assert!(parse_song("track a\nC4 R:-1 D4\n").is_err());
        assert!(parse_song("track a\nC4:0\n").is_err());
        assert!(parse_song("track a\nC4\nautomate cutoff\n500 R:-0.5\n").is_err());
    }

    #[test]
    fn duck_from_keys_a_strip_off_another_track() {
        let text = "track pad duck=0.5 duck_from=kick\nC4:4\n\
                    track hats duck=0.3 duck_from=beat.CH\nC5\n\
                    track keyed duck=0.3 duck_from=lead.C4\nE4\n\
                    track kick kit=909\nBD\ntrack beat kit=909\nCH\ntrack lead\nC4\n";
        let song = parse_song(text).unwrap();
        use crate::voice_manager::{DuckKey, KeyFilter};
        let key = |t: &str| {
            song.mix
                .duck_keys
                .iter()
                .find(|(c, _)| *c == channel_of(&song, t))
                .unwrap()
                .1
        };
        assert_eq!(
            key("pad"),
            DuckKey {
                channel: channel_of(&song, "kick"),
                filter: KeyFilter::Any
            }
        );
        assert_eq!(
            key("hats").filter,
            KeyFilter::Drum(crate::drums::DrumVoice::ClosedHat)
        );
        assert_eq!(key("keyed").filter, KeyFilter::Note(60));
        assert!(parse_song("track pad duck_from=nobody\nC4\n").is_err());
        assert!(parse_song("track pad duck_from=lead.Q9\nC4\ntrack lead\nC4\n").is_err());
    }

    /// The duck keyed off a track dips the strip by exactly its depth
    /// while it holds, in the bounce: duck=0.5 is -6 dB. Judged against
    /// the same bounce unducked, since a detuned saw stack beats by a
    /// couple of dB on its own.
    #[test]
    fn duck_from_dips_by_the_set_depth() {
        let take = |depth: &str| {
            let text = format!(
                "automate reverb_wet\n0\nautomate echo\n0\n\
                 track pad duck={depth} duck_from=key duck_hold=0.2 duck_release=0.05\n\
                 C4:8\ntrack key kit=909 gain=0\nR:2 SD\n"
            );
            let song = parse_song(&text).unwrap();
            let pad = channel_of(&song, "pad");
            render_offline_solo(&song, 48000.0, Some(pad)).collect::<Vec<(f32, f32)>>()
        };
        let (dry, ducked) = (take("0"), take("0.5"));
        let energy = |f: &[(f32, f32)], t0: f64, t1: f64| {
            f[(t0 * 48000.0) as usize..(t1 * 48000.0) as usize]
                .iter()
                .map(|&(l, r)| (l * l + r * r) as f64)
                .sum::<f64>()
        };
        let db = |t0: f64, t1: f64| {
            10.0 * (energy(&ducked, t0, t1) / energy(&dry, t0, t1)).log10()
        };
        // the key track's snare at beat 2 (1.0 s) keys it: full depth for
        // the 0.2 s hold, then a 50 ms release back to open
        assert!(db(0.5, 0.99).abs() < 0.05, "open before the key: {:.2} dB", db(0.5, 0.99));
        let held = db(1.001, 1.199);
        assert!((held + 6.02).abs() < 0.1, "duck=0.5 held at {held:.2} dB");
        assert!(db(1.6, 1.9).abs() < 0.05, "released: {:.2} dB", db(1.6, 1.9));
    }

    #[test]
    fn shapers_bring_the_song_clock_and_others_do_not() {
        let plain = parse_song("track a\nC4:4\n").unwrap();
        assert!(!plain
            .events
            .iter()
            .any(|e| matches!(e.kind, EventKind::Clock { .. })));
        let gated = parse_song("bpm 100\ntrack a shape=x.\nC4:4\n").unwrap();
        let clocks: Vec<(f64, f64)> = gated
            .events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::Clock { beat, bpm } => Some((beat, bpm)),
                _ => None,
            })
            .collect();
        assert_eq!(clocks, vec![(0.0, 100.0)], "a steady tempo needs one clock");
        let ramp = parse_song("bpm 100\nautomate bpm\n100 R:1 140:1\ntrack a shape=pump\nC4:4\n")
            .unwrap();
        let n = ramp
            .events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::Clock { .. }))
            .count();
        assert!(n > 30, "a tempo ramp re-clocks every grid step: {n}");
        assert!(parse_song("track a shape=x?\nC4\n").is_err());
        assert!(parse_song("track a shape_beats=2\nC4\n").is_err());
    }

    /// A trance gate opens and shuts on the beat grid.
    #[test]
    fn a_trance_gate_follows_the_beat() {
        // 120 bpm: a sixteenth is 0.125 s; `x.` opens the first and shuts
        // the second of each pair
        let text = "automate reverb_wet\n0\nautomate echo\n0\ntrack a shape=x.\nC4:8\n";
        let song = parse_song(text).unwrap();
        let frames: Vec<(f32, f32)> = render_offline(&song, 48000.0).collect();
        let open = frames_rms(&frames, 0.51, 0.62, 48000.0);
        let shut = frames_rms(&frames, 0.64, 0.74, 48000.0);
        assert!(
            20.0 * (shut / open).log10() < -40.0,
            "gate shut only {:.1} dB",
            20.0 * (shut / open).log10()
        );
    }

    #[test]
    fn strip_inserts_and_the_strip_compressor_are_addressable() {
        let head = "track d kit=909 comp=-20 comp_ratio=2 comp_makeup=3 dist=0.3 dist_mode=2 \
                    eq_low=-4 eq_mid_freq=400\nBD\n";
        for lane in [
            "d.comp_ratio",
            "d.comp_attack",
            "d.dist",
            "d.eq_high",
            "d.shape_depth",
            "d.duck_hold",
        ] {
            assert!(
                parse_song(&format!("{head}automate {lane}\n1\n")).is_ok(),
                "{lane} refused"
            );
        }
        // the bus compressor's IN switch has no strip meaning
        assert!(parse_song(&format!("{head}automate d.comp_in\n1\n")).is_err());
        // unscoped, master controls
        assert!(parse_song(&format!("{head}automate limit\n-8\nautomate clip\n3\n")).is_ok());
    }

    /// A strip insert with memory hears the track's summed voices once a
    /// sample. Run per voice card, a sample-rate reducer at 29.6 kHz held
    /// one card's sample for all eight cards of a chord and raised the
    /// track 2.5 dB; on the summed chord it is all but transparent, as it
    /// is on one signal.
    #[test]
    fn strip_inserts_hear_the_whole_chord() {
        let take = |opts: &str| -> Vec<(f32, f32)> {
            let text = format!(
                "tail 0.1\nautomate reverb_wet\n0\nautomate echo\n0\n\
                 track p patch=superwall {opts}\n[F3 G#3 C4 F4]:4\n"
            );
            render_offline(&parse_song(&text).unwrap(), 48000.0).collect()
        };
        let rms = |f: &[(f32, f32)]| {
            (f.iter().map(|&(l, r)| (l * l + r * r) as f64).sum::<f64>() / f.len() as f64).sqrt()
        };
        let plain = rms(&take(""));
        let held = rms(&take("dist=0.1 dist_mode=4"));
        let db = 20.0 * (held / plain).log10();
        assert!(db.abs() < 0.2, "a near-transparent hold moved the chord {db:+.2} dB");
    }

    /// A strip's EQ on a chord is the plain chord put through that EQ once
    /// a sample: the filter hears the channel's summed voice cards, not
    /// each card in turn. Only the master's tanh, 40 dB down, separates the
    /// two, over the first quarter second (later the supply's load, which
    /// follows the output level, drifts the voices apart).
    #[test]
    fn a_strip_eq_on_a_chord_is_the_eq_run_once_on_the_sum() {
        use crate::inserts::{Eq3, EqKnob};
        let take = |opts: &str| -> Vec<(f32, f32)> {
            let text = format!(
                "automate reverb_wet\n0\nautomate echo\n0\n\
                 track p patch=superwall gain=0.1 chorus_send=0 {opts}\n[F3 G#3 C4 F4]:4\n"
            );
            render_offline(&parse_song(&text).unwrap(), 48000.0)
                .take(12_000)
                .collect()
        };
        let plain = take("");
        let heard = take("eq_low=-12 eq_mid=12 eq_mid_q=2 eq_high=9");
        let mut eq = Eq3::new(48000.0);
        eq.set(EqKnob::LowGain, -12.0);
        eq.set(EqKnob::MidGain, 12.0);
        eq.set(EqKnob::MidQ, 2.0);
        eq.set(EqKnob::HighGain, 9.0);
        let (mut err, mut sig) = (0f64, 0f64);
        for (&(l, r), &(hl, hr)) in plain.iter().zip(&heard) {
            let (el, er) = eq.process(l, r);
            err += ((hl - el) as f64).powi(2) + ((hr - er) as f64).powi(2);
            sig += (hl as f64).powi(2) + (hr as f64).powi(2);
        }
        let db = 10.0 * (err / sig).log10();
        assert!(
            sig > 0.0 && db < -50.0,
            "the chord's EQ is {db:.1} dB off one EQ on the sum"
        );
    }

    /// The master limiter's ceiling is the bounce's true peak (the DC
    /// blocker runs before it and no tanh after), it lowers a dense loop's
    /// crest, and a threshold first set mid-song lands without a dropout:
    /// the song is armed at its downbeat.
    #[test]
    fn the_master_limiter_holds_its_ceiling_in_a_bounce() {
        let body = "track k kit=909\n(BD)x16\ntrack h kit=909\n(R:0.5 OH:0.5)x16\n\
                    track b patch=deepsine gain=2\n(F1:0.5 F2:0.5)x16\n";
        let take = |head: &str| -> Vec<(f32, f32)> {
            let song = parse_song(&format!("bpm 128\ntail 0.2\n{head}{body}")).unwrap();
            render_offline(&song, 48000.0).collect()
        };
        let peak = |f: &[(f32, f32)]| f.iter().fold(0f32, |m, &(l, r)| m.max(l.abs()).max(r.abs()));
        let rms = |f: &[(f32, f32)]| {
            (f.iter().map(|&(l, r)| (l * l + r * r) as f64).sum::<f64>() / (2 * f.len()) as f64)
                .sqrt() as f32
        };
        let crest = |f: &[(f32, f32)]| 20.0 * (peak(f) / rms(f)).log10();
        let plain = take("");
        let limited = take("automate limit\n-16\nautomate limit_ceiling\n-2\n");
        let ceiling = 10f32.powf(-2.0 / 20.0);
        assert!(peak(&limited) <= ceiling, "peak {} over {ceiling}", peak(&limited));
        assert!(peak(&limited) > ceiling * 0.99, "the ceiling is reached: {}", peak(&limited));
        assert!(
            crest(&limited) < crest(&plain) - 4.0,
            "crest {:.1} -> {:.1}",
            crest(&plain),
            crest(&limited)
        );
        // set at beat 4 (1.875 s): no run of silence where it lands
        let late = take("automate limit\n0 R:4 -16:0.01@step\n");
        let at = (4.0 * 60.0 / 128.0 * 48000.0) as usize;
        let quiet = late[at - 200..at + 400]
            .windows(24)
            .any(|w| w.iter().all(|&(l, r)| l == 0.0 && r == 0.0));
        assert!(!quiet, "engaging the limiter dropped a run of silence into the mix");
    }
}
