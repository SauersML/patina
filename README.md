# 🎹 Patina: An Analog Synth Simulator 🎛️

Patina is a realistic emulation of classic analog synthesizer sounds, built in Rust —
a Moog-inspired ladder filter feeding a Juno-style chorus, with a scriptable song
format for programmatic playback and parameter automation.

## ✨ Features

- 🎚️ 3-oscillator unison per voice built the 901B way: one bandlimited saw core with triangle folded from it, sine as a transistor-rounded triangle, pulse from a comparator with per-unit duty error — at the service manual's (non-normalized) output levels, with correlated bank drift
- 🌫️ One shared transistor noise source distributed into every voice (903A/Juno architecture)
- 🌊 Global LFO after the variable-rate-integrator patent (US 3,943,456): shape morphs saw→triangle→ramp, driving vibrato (in CV space), filter wobble, and PWM on every voice together
- 📊 Analog-style exponential ADSR amplitude envelope
- 🎯 Dedicated filter envelope (±5 octaves), velocity-to-filter, and keyboard tracking
- 🔊 Polyphonic voices with age-based stealing, spread across the stereo field
- 🎛️ Moog-inspired ladder filter: 4 tanh stages, 2× oversampled, resonance to self-oscillation, drive, saturation, transistor mismatch, thermal drift
- 🧈 Per-sample parameter smoothing — no zipper noise under automation
- 🌀 Juno-style chorus (modes I–IV), stereo reverb, a 905-style dual-spring reverb (dispersive, fixed mechanical decay, wet/dry only), and a physically-modeled cassette tape stage (wow, flutter, saturation, age)
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
   cargo run --release -- --play songs/nightdrive.song
   ```

## 🎛️ Usage

Once Patina is running, you'll see the GUI with various controls:

- Use the sliders to adjust volume, ADSR envelope parameters, filter, chorus, and reverb.
- Select different waveforms.
- Play notes using your computer keyboard (Z-M for lower octave, Q-P for higher octave).
- Click or drag on the on-screen keyboard to play notes with your mouse.
- Connect a MIDI keyboard (or enable the macOS IAC Driver for virtual MIDI).

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
- **Parameters**: `volume`, `waveform` (0–3), `detune`, `noise` (0–1), `pulse_width` (0.05–0.95), `lfo_rate` (Hz), `lfo_shape` (0 = saw, 0.5 = tri, 1 = ramp), `lfo_pitch` (cents), `lfo_filter` (octaves), `lfo_pwm` (0–0.45), `hpf` (Hz, 16 = off), `fuzz` (0–1), `spring` (0–1), `cutoff`, `resonance`, `drive`, `saturation`, `attack`, `decay`, `sustain`, `release`, `filter_env` (octaves), `filter_attack`, `filter_decay`, `filter_sustain`, `filter_release`, `reverb_decay`, `reverb_wet`, `chorus_mode` (0–4), `chorus_rate`, `chorus_depth`, `tape_wow`, `tape_flutter`, `tape_drive`, `tape_age`.

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
