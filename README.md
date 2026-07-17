# 🎹 Patina: An Analog Synth Simulator 🎛️

Patina is a realistic emulation of classic analog synthesizer sounds, built in Rust —
a Moog-inspired ladder filter feeding a Juno-style chorus, with a scriptable song
format for programmatic playback and parameter automation.

## ✨ Features

- 🎚️ Multiple oscillator types (Sine, Square, Sawtooth, Triangle) with polyBLEP anti-aliasing
- 📊 ADSR envelope generator
- 🔊 Polyphonic voice management with age-based voice stealing
- 🎛️ Moog-inspired ladder filter (resonance, drive, saturation, thermal drift)
- 🌀 Juno-style chorus (modes I–IV) and a stereo reverb
- 🖥️ Real-time parameter control via GUI
- ⌨️ QWERTY keyboard input for note playing
- 🖱️ Click-and-drag interface for playing notes
- 🎹 MIDI input (auto-connects to an IAC Driver if present)
- 📜 Text-based song files with per-track sequencing and parameter automation

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
- **Parameters**: `volume`, `cutoff`, `resonance`, `drive`, `saturation`, `attack`, `decay`, `sustain`, `release`, `reverb_decay`, `reverb_wet`, `chorus_rate`, `chorus_depth`.

## 🧪 Technical Details

- **Audio Engine**: CPAL (Cross-Platform Audio Library) for low-latency audio output.
- **Oscillators**: polyBLEP anti-aliasing with slow analog-style pitch drift.
- **Filter**: Moog-inspired ladder filter with resonance, drive, saturation, transistor mismatch, and thermal drift.
- **Chorus**: modeled on the Roland Juno bucket-brigade chorus, modes I–IV.
- **Envelope**: ADSR (Attack, Decay, Sustain, Release).
- **Voice Management**: polyphonic with age-based voice stealing (idle voices first, then releasing, then oldest).
