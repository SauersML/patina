# 🎹 Patina: An Analog Synth Simulator 🎛️

Patina is a realistic emulation of classic analog synthesizer sounds, built in Rust —
a Moog-inspired ladder filter feeding a Juno-style chorus, with a scriptable song
format for programmatic playback and parameter automation.

## ✨ Features

- 🎚️ 3-oscillator unison per voice built the 901B way: one bandlimited saw core with triangle folded from it, sine as a transistor-rounded triangle, pulse from a comparator with per-unit duty error — at the service manual's (non-normalized) output levels, with correlated bank drift
- 🌫️ One shared transistor noise source distributed into every voice (903A/Juno architecture)
- 🌊 Global LFO after the variable-rate-integrator patent (US 3,943,456): shape morphs saw→triangle→ramp, driving vibrato (in CV space), filter wobble, and PWM on every voice together
- 🛝 Glide per US 3,991,645: the keyboard CV lags through an RC before the exponential converter, so portamento settles exponentially in octave space — the authentic Minimoog/303 swoop
- 📊 Analog-style exponential ADSR amplitude envelope
- 🎯 Dedicated filter envelope (±5 octaves), velocity-to-filter, and keyboard tracking
- 🔊 Polyphonic voices with age-based stealing, spread across the stereo field
- 🎛️ Moog-inspired ladder filter: 4 tanh stages, 2× oversampled, resonance to self-oscillation, drive, saturation, transistor mismatch, thermal drift
- 🧈 Per-sample parameter smoothing — no zipper noise under automation
- 🌀 Juno-style chorus (modes I–IV), stereo reverb, a 905-style dual-spring reverb (dispersive, fixed mechanical decay, wet/dry only), and a physically-modeled cassette tape stage (wow, flutter, saturation, age)
- 🥁 A TR-909 rhythm section built from the service-manual circuits — swept bridged-T kick with click path and waveshaper, twin-shell snare with snappy noise, three-mode rim knock, flam-envelope clap, and a no-samples hi-hat (six-oscillator metal bank through the 909's swept high-pass and choke VCAs) — living on the same volt bus, effects, and power rail as the keyboard voices. Modern range extensions: kick DRIVE from clean sub to full grit, SWEEP depth, rumble-length decays, and an antialiased bus drive. Triggered from drum tracks in songs (`track beat kit=909` with `BD SD RS CP CH OH`), MIDI channel 10 (GM map), pads on the panel, or the plugin
- 🗣️ A voice box: a Klatt-style formant speech synthesizer (glottal pulse + cascade resonators, ARPAbet phonemes) driving a 20-band channel vocoder whose carrier is the synth's own voices, with a VSM-201-style voiced/unvoiced switch that snaps the carrier to noise on consonant frames so words stay legible. Two circuits on the mode switch: the '97 DigiTech Talker "TalkBox" voicing (tube-choked lows, honky mids, instant articulation, amp grit) and a full-range studio board. Lyrics ride the notes in songs (`track choir vox`, `[A2 E3 A3]:2=HH-EH-L-OW`) with per-phoneme duration and dynamics; `wav=` feeds any recorded voice through the same circuit, and `patina --say "HH-AH-L-OW"` speaks from the command line
- 📼 A tape-deck sampler: any WAV becomes an instrument on the keys (`track keys sample=tape.wav`) — band-limited windowed-sinc playback (Kaiser β=8.6, kernel stretched by the read speed so varispeed doesn't alias in *either* direction), sustain loops with equal-power crossfades (a Mellotron whose tape never runs out), `chop=N` slice pads mapped chromatically (the MPC workflow), `beats=N` tempo-fitting (break-matching solved at parse), a per-slot resonant ZDF lowpass (the SP/Akai filter-sweep sound), vintage converter emulation (`bits=12 rate=26040` is the SP-1200: band-limited decimation at load, truncation, and a zero-order-hold DAC whose imaging spray is the point), reverse transport, gate/one-shot modes, per-note choke (`mono`), and a live-automatable transport: `smp_pitch` is a varispeed knob (tape-stops on demand), `smp_start` scrubs the needle-drop, `smp_cutoff`/`smp_res` sweep the filter, `smp_gain`/`smp_pan`/`smp_attack`/`smp_release` reshape it mid-song. The heads mix onto the same volt bus as everything else — sampler playback sags the power rail, runs through the fuzz/spring/reverb/chorus/tape chain, and bends with the pitch wheel
- 🖥️ Studio-hardware GUI: knobs, ADSR graph, live oscilloscope, keys that light from the engine's real voice state
- ⌨️ QWERTY keyboard input · 🖱️ click-and-drag keys · 🎹 MIDI input
- 📜 Text-based song files with per-track sequencing and full parameter automation

## 🚀 Getting Started

### Prerequisites

- Rust (latest stable version)

### Installation

1. Clone the repository:
   ```
   git clone https://github.com/SauersML/patina.git
   cd patina
   ```

2. Build and run:
   ```
   cargo run --release
   ```

3. Or play a song file:
   ```
   cargo run --release            # shuffles to a random song
   cargo run --release -- --play songs/ferris-wheel.song
   ```

## 🎚️ Patches

The strip at the top of the panel holds the factory bank (US 3,981,218 style — one click retunes every block at once, live, even under held notes): **Init · Glasswing · Rust Engine · Peppermint · Sea of Dials · Fathom · Tears · Moths · Anemone · Thunder · Choir** — each with its own keyboard register (a bass patch arrives at octave 2). `SAVE` snapshots your current knobs to `patches/user-N.patch` — plain text, same parameter names as song automation, edit at will.

## 🎛️ Usage

Once Patina is running, you'll see the GUI with various controls:

- Use the sliders to adjust volume, ADSR envelope parameters, filter, chorus, and reverb.
- Select different waveforms.
- Play notes using your whole computer keyboard: the Z row (with home-row sharps S D G H J) is the lower manual; the Q row runs from one octave up through `]`, with sharps on 2 3 5 6 7 9 0 and `=`. Arrow keys shift the octave.
- The right-hand cluster is the 909 pad grid, mirrored as glowing pads beside the on-screen keys: `,` kick, `.` snare, `/` clap on the bottom row; `K` closed hat, `L` open hat, `;` rim, `'` ghost snare above. Hold Shift for accent.
- Click or drag on the on-screen keyboard to play notes with your mouse — pads click too, with strike depth as velocity.
- Connect a MIDI keyboard (or enable the macOS IAC Driver for virtual MIDI).

## 🎹 MIDI

Every automatable parameter answers to a controller, scaled exactly like
its on-screen knob (log where the knob is log). One chart, defined once in
`Param::from_cc`:

| CC | Parameter | CC | Parameter |
|----|-----------|----|-----------|
| 1 | mod wheel (vibrato) | 85/86 | osc 2 level / pitch |
| 5 | glide time | 87/88 | osc 3 level / pitch |
| 7 | volume | 89/90 | FM / ring |
| 64 | sustain pedal | 91/93/95 | reverb / chorus / spring |
| 71/74 | resonance / cutoff | 92/94 | tape wow / flutter |
| 72/73/75/79 | release / attack / decay / sustain | 102–111 | hpf, drive, saturation, key track, filter env (A/D/S/R), chorus rate |
| 76/77/78 | LFO rate / pitch / filter | 112–119 | chorus mode, waveforms, circuit, sync, tape drive/age |
| 8/9/10 | sampler gain / varispeed / pan | 17/18/19 | sampler start / attack / release |
| 3/4 | sampler cutoff / resonance | | |

Pitch bend is ±2 semitones. **Program change** switches factory patches.
Songs speak the same language — `automate bend`, `automate mod_wheel`, and
`automate pedal` work like any other parameter.

## 📜 Song files

`patina --play <file>` plays a plain-text song. Tracks run in parallel;
`#` starts a comment. Full reference at the top of `src/song.rs`.

```
bpm 100
gate 0.85                      # note length as a fraction of its duration

track lead vel=0.9 len=0.5     # default velocity and duration (beats)
E5:1 D5 C5 | [C4 E4 G4]:2@0.6 R:2

automate cutoff                # ramp any parameter through breakpoints
500 7000:16@exp R:8 600:8@smooth
```

- **Notes**: names (`C4`, `F#3`, `Eb5`, with C4 = MIDI 60) or raw MIDI numbers; `[..]` for chords; `R` or `.` for rests; `:beats` duration; `@vel` velocity; `|` bar lines (ignored).
- **Automation**: `automate <param>` starts a curve track. The first token is the starting value; `V:D@shape` ramps to `V` over `D` beats; `R:D` holds. Shapes: `lin`, `exp` (geometric — right for frequencies), `log`, `smooth`, `step`.
- **Parameters**: `volume`, `waveform` (0–3), `detune`, `noise` (0–1), `glide` (seconds, 0 = off), `pulse_width` (0.05–0.95), `lfo_rate` (Hz), `lfo_shape` (0 = saw, 0.5 = tri, 1 = ramp), `lfo_pitch` (cents), `lfo_filter` (octaves), `lfo_pwm` (0–0.45), `hpf` (Hz, 16 = off), `fuzz` (0–1), `spring` (0–1), `cutoff`, `resonance`, `drive`, `saturation`, `attack`, `decay`, `sustain`, `release`, `filter_env` (octaves), `filter_attack`, `filter_decay`, `filter_sustain`, `filter_release`, `reverb_decay`, `reverb_wet`, `chorus_mode` (0–4), `chorus_rate`, `chorus_depth`, `tape_wow`, `tape_flutter`, `tape_drive`, `tape_age`, `vox_level`, `vox_dry`, `vox_breath`, `vox_vibrato`, `vox_mode` (0 = TalkBox, 1 = vocoder), `vox_intonation`, and the tape deck's `smp_pitch` (semitones), `smp_start` (0–1), `smp_gain`, `smp_pan`, `smp_attack`, `smp_release`, `smp_cutoff` (Hz), `smp_res` (0–1) (per sampler track via `automate <track>.smp_pitch`, or global to all slots).

## 🗣️ The voice box

A vox track's notes are the vocoder's **carrier**; its `=lyrics` drive the
**modulator** — the built-in formant voice, or any recording:

```
track choir vox
[A2 E3 A3]:2=HH-OW1-L-D [G2 D3 G3]:2=AA-N | [A2 E3 A3 C4]:6=HH-OW1-M.

track voice vox wav=renders/borrowed.wav    # a recording played on the keys
[D3 A3 D4 F4]:2 [C3 G3 C4 E4]:2
```

Lyrics are dash-joined ARPAbet phonemes riding their note. Onsets speak at
note-on, the vowel sustains while held (pitch = lowest held key), codas
land on the release. Per phoneme: `:ms` fixed length, `@amp` loudness,
stress digits on vowels (`OW1` primary, `AH0` reduced) shaping loudness,
length, and pitch accents. A trailing `.` or `?` ends the phrase with a
fall or rise. `vox_intonation` scales the voice's own prosody (accents,
declination, final falls): keep it low when singing, high when speaking.

- `patina --say "HH-AH-L-OW1. AY1 K-AE-N S-P-IY1-K." [--out say.wav]` speaks from the command line.
- `scripts/borrow-voice.sh "text" out.wav [voice]` renders a recording for `wav=` with the house voice: **Kokoro-82M** on MLX (Apache 2.0, ~300 MB, #1 on TTS Arena at 82M params — chosen by shootout over Piper and Chatterbox Turbo, which have been retired from the repo). `[voice]` picks any of Kokoro's ~50 voices: `af_heart` (default, warm) for songs, `am_michael` (low, steady) for Talker-circuit leads, `bf_emma`/`bm_george` for British color, plus Spanish/French/Hindi/Italian/Japanese/Portuguese/Chinese sets. One-time setup: `uv venv --python 3.12 .venv-voice && uv pip install --python .venv-voice/bin/python mlx-audio "misaki[en]" torch`. Falls back to the macOS system voice when the venv is absent.

## 📼 The tape deck

A `sample=` track puts a recording on the keys. Everything is optional but
the file; times are seconds on the source recording:

```
track keys sample=renders/hold-on.wav root=A3 loop=46.5:52.5 xfade=1.5 attack=0.9
A3:8 F3:8 G3:8 A3:8              # varispeed repitch around root=

track pads sample=break.wav chop=8 root=C4 mono
(C4 . D#4 . F4 C4 . G4)x4        # 8 slice pads up from C4, each choking the last

automate keys.smp_pitch          # a varispeed knob, in semitones:
0 R:24 -24:6@smooth              # ...a two-octave tape-stop to end the song
```

- **Options**: `root=` (key of natural speed), `start=`/`end=` (trim), `loop` or `loop=a:b` + `xfade=` (sustain loop, equal-power crossfade), `chop=N` (slice pads, natural speed, one-shot), `beats=N` (varispeed the region/loop to span exactly N beats at the song tempo — drop a breakbeat in and it locks), `bits=`/`rate=` (vintage converters + un-reconstructed ZOH playback; `bits=12 rate=26040` is the SP-1200, `bits=8` the early Fairlight school), `cutoff=`/`res=` (the slot's resonant lowpass), `mode=gate|oneshot`, `reverse`, `fixed` (no keytracking), `mono`/`choke`, `gain=`, `pan=`, `pitch=`, `attack=`, `release=`, `vel_amt=`.
- `songs/magnetic-memory.song` is the demo: the instrument sampling its own bounce of *Hold On* — looped tape choir, vocal chops, reverse swells, and a tape-stop ending.

## 🔌 The Substrate

Patina models the *chassis*, not just the modules. Three shared physical states couple everything, with magnitudes taken from the service specs rather than tuned by ear:

- **The power rail** — a regulated source (±0.075%, 5 mV ripple) behind the 10Ω/100µF local filter drawn on the 904A blueprint. Summed voice current sags it; the rail feeds every expo converter, so heavy playing microscopically flattens *everything together*, and mains ripple adds correlated micro-FM.
- **Chassis heat** — the instrument powers on slightly flat with the filters low and warms up over minutes (the manuals' 30-minute alignment warm-up), each voice card converging at its own thermal rate. Playing hard adds dissipation heat. Offline bounces record a warmed instrument.
- **The board** — adjacent voice cards leak their *differentiated* pre-filter signal into each other at ~−64 dB (trace capacitance differentiates); the 902-style VCA control feedthrough makes fast attacks physically thump (post-trim residue, per-unit); and the summing amp's finite slew rate (0.5 V/µs) shaves only the hottest multi-voice transients.

No knobs — it's the chassis. It is simply *on*, the way gravity is.

## 🧪 Technical Details

- **Audio Engine**: CPAL (Cross-Platform Audio Library) for low-latency audio output.
- **Oscillators**: 3-oscillator unison, polyBLEP anti-aliasing, free-running phases, bounded-random-walk pitch drift.
- **Filter**: the Huovilainen model of the Moog transistor ladder (US 3,475,623) — four one-pole stages with tanh differential-pair nonlinearities at thermal-voltage signal scale, 2× oversampled with half-sample-averaged feedback, published cutoff/resonance tuning-compensation polynomials, authentic passband thinning at high resonance, self-oscillation at k = 4, per-sample cutoff modulation, smoothed parameters, transistor mismatch, thermal drift.
- **VCO circuit tolerances**: per-oscillator V/octave scaling error (±1.5 cents/octave from the calibration point) and finite integrator-reset time that flattens high notes — the imperfections that make analog chords bloom.
- **High-pass**: 904B-style 24 dB/oct high-pass ladder per voice (trapezoidal zero-delay one-poles) — in series with the low-pass it recreates the 904C band-pass coupling.
- **Fuzz**: germanium Fuzz-Face-style stage on the bus — biased soft-knee saturation with even-harmonic asymmetry (after the DAFx-17 GBJT study), AC-coupled, antialiased.
- **Antialiasing**: first-order antiderivative antialiasing (ln cosh form, Parker et al.) on the filter's saturation stage and the fuzz nonlinearity.
- **Envelopes**: exponential RC-curve ADSR for amplitude and filter, click-free retriggering.
- **Chorus**: modeled on the Roland Juno bucket-brigade chorus, modes I–IV.
- **Tape**: cassette model with wow/flutter/drift transport, Langevin magnetization curve, head bump, gap loss, dropouts, and hiss.
- **Voice Management**: polyphonic with age-based voice stealing (idle voices first, then releasing, then oldest), equal-power stereo voice spread, DC-blocked and soft-limited master bus.
