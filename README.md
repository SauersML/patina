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
- 🔊 Polyphonic voices with age-based stealing; the STEREO knob fans successive notes across the field the way a rotating assigner over pan-potted voice cards does (Oberheim-style), so chords spread and balance while a held note never moves
- 🎛️ Moog-inspired ladder filter: 4 tanh stages, 2× oversampled, resonance to self-oscillation, drive, saturation, transistor mismatch, thermal drift
- 🧈 Per-sample parameter smoothing — no zipper noise under automation
- 🌀 Juno-style chorus (modes I–IV), stereo reverb, a 905-style dual-spring reverb (dispersive, fixed mechanical decay, wet/dry only), and a physically-modeled cassette tape stage (wow, flutter, saturation, age)
- 🗜️ A mix-bus "glue" compressor modeled on the SSL 4000 G-series centre-section bus compressor, printing to the tape: feed-forward, stereo-linked, a Blackmer (dbx 202-family) VCA with its real even-order colour, a log-converter soft knee, stepped ratio (2/4/10:1), attack (0.1–30 ms) and release (0.1–1.2 s) switches with a dual-time-constant program-dependent AUTO, make-up, parallel mix, a sidechain high-pass, and a moving-coil gain-reduction meter on the panel. Out by default — and out means bit-identical (`comp_in`, `comp_threshold`, … in songs and patches)
- 🥁 A TR-909 rhythm section built from the service-manual circuits — swept bridged-T kick with click path and waveshaper, twin-shell snare with snappy noise, three-mode rim knock, flam-envelope clap, and a no-samples hi-hat (six-oscillator metal bank through the 909's swept high-pass and choke VCAs) — living on the same volt bus, effects, and power rail as the keyboard voices. Modern range extensions: kick DRIVE from clean sub to full grit, SWEEP depth, rumble-length decays, and an antialiased bus drive. Triggered from drum tracks in songs (`track beat kit=909` with `BD SD RS CP CH OH`; every kit track is a board of its own), MIDI channel 10 (GM map), pads on the panel, or the plugin
- 🗣️ A voice box: a Klatt-style formant speech synthesizer (glottal pulse + cascade resonators, ARPAbet phonemes) driving a 20-band channel vocoder whose carrier is the synth's own voices, with a VSM-201-style voiced/unvoiced switch that snaps the carrier to noise on consonant frames so words stay legible. Two circuits on the mode switch: the '97 DigiTech Talker "TalkBox" voicing (tube-choked lows, honky mids, instant articulation, amp grit) and a full-range studio board. Lyrics ride the notes in songs (`track choir vox`, `[A2 E3 A3]:2=HH-EH-L-OW`) with per-phoneme duration and dynamics; `wav=` feeds any recorded voice through the same circuit, and `patina --say "HH-AH-L-OW"` speaks from the command line
- 📼 A tape-deck sampler: any WAV becomes an instrument on the keys (`track keys sample=tape.wav`) — band-limited windowed-sinc playback (Kaiser β=8.6, kernel stretched by the read speed so varispeed doesn't alias in *either* direction), sustain loops with equal-power crossfades (a Mellotron whose tape never runs out), `chop=N` slice pads mapped chromatically (the MPC workflow), `beats=N` tempo-fitting (break-matching solved at parse), a per-slot resonant ZDF lowpass (the SP/Akai filter-sweep sound), vintage converter emulation (`bits=12 rate=26040` is the SP-1200: band-limited decimation at load, truncation, and a zero-order-hold DAC whose imaging spray is the point), reverse transport, gate/one-shot modes, per-note choke (`mono`), and a live-automatable transport: `smp_pitch` is a varispeed knob (tape-stops on demand), `smp_start` scrubs the needle-drop, `smp_cutoff`/`smp_res` sweep the filter, `smp_gain`/`smp_pan`/`smp_attack`/`smp_release` reshape it mid-song. The heads mix onto the same volt bus as everything else — sampler playback sags the power rail, runs through the fuzz/spring/reverb/chorus/tape chain, and bends with the pitch wheel
- 🥊 Kick-bass on any voice: a per-note pitch envelope (start, time, curve) diving onto the note, key sync, and an antialiased drive stage (tanh, clip, wavefolder) ahead of the ladder as post-filter; plus a seven-saw supersaw bank and a filtered noise oscillator per card for walls and risers
- 🫦 Note bends (`C4:2>G3` glides to the target over the note), and the throat: pitch-synchronous grains that move a voice's formants apart from its pitch, so a sampled voice drops an octave without turning into a monster (`formant`, `smp_formant`, `vox_formant`)
- 💥 Glitch inserts on any track or the whole mix bus: beat repeat from 1/8 to 1/64, reverse in note slices, and tape stop (`stutter`, `reverse`, `tape_stop`)
- 🔀 Audio-rate cross-modulation between tracks: any track's signal as another's FM, filter FM or ring modulator, and routed feedback loops between tracks through a delay, a tone filter and a safety limiter (`fm_from=`, `filter_fm_from=`, `ringmod_from=`, `feed_from=`)
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
   cargo run --release            # opens the live synth, silent until played
   cargo run --release -- --play songs/ferris-wheel.song
   cargo run --release -- --play file.mid --patch ember --render out.wav   # MIDI file, every melodic channel on patches/ember.patch (Init if omitted)
   ```

## 🔌 Plugins

The same engine ships as a plugin for every major host, all three front ends
driven by one shared parameter table (`src/host_params.rs`):

- **CLAP / VST3** — `cargo xtask bundle patina --release --no-default-features --features plugin`
- **Audio Unit (Logic Pro, GarageBand)** — a native AUv2 MusicDevice written
  directly against the AudioComponent ABI, no wrapper frameworks:
  `scripts/bundle-au.sh --validate` builds `Patina.component`, installs it to
  `~/Library/Audio/Plug-Ins/Components`, and runs Apple's `auval` suite
  against it. In Logic, it appears as an instrument under
  **Sauers → Patina**; keyboard voices on any MIDI channel, the 909 board on
  channel 10, and every knob automatable with Logic's parameter curves. It
  carries the **same panel as the standalone app** — the exact knobs, glass,
  and glyphs — software-rendered (`src/editor.rs`, `src/au/raster.rs`) and
  vended as a CocoaUI view (`src/au/cocoa.rs`) that survives Logic's
  out-of-process view host by drawing through AppKit's `drawRect:` cycle.
  Build with `--no-default-features --features au`; drop `editor` from the
  feature set to fall back to the host's generic parameter view.

On macOS the git hooks install all three automatically after every commit
(`scripts/install-plugins.sh`).

## 🎚️ Patches

The strip at the top of the panel holds the factory bank (US 3,981,218 style — one click retunes every block at once, live, even under held notes, and every patch lands over Init so nothing leaks from the last one): **Init · Ember · Tidewater · Glasswing · Sea of Dials · Aurora · Vellum · Choir · Thunder · Lantern · Tears · Fathom · Warehouse** — sustained, dimensional voices, level-matched to −20 LUFS, each with its own keyboard register. Init is one dry sawtooth with every effect out, and it is also every control's reset position and every plugin parameter's default. `SAVE` snapshots every control to `patches/user-N.patch` — plain text, same parameter names as song automation, edit at will.

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
gate 0.85                      # note separation: (1-gate) of the note, capped at 80 ms
section CH1 16..32             # name a beat range: `>CH1` / `>CH1.end` seek to it

track lead vel=0.9 len=0.5     # default velocity and duration (beats)
E5:1 D5 C5 | [C4 E4 G4]:2@0.6 R:2

automate cutoff                # ramp any parameter through breakpoints
500 7000:16@exp R:8 600:8@smooth

automate vox_mode: 1 during CH1 base 0   # one-line section automation
```

- **Notes**: names (`C4`, `F#3`, `Eb5`, with C4 = MIDI 60) or raw MIDI numbers; `[..]` for chords; `R` or `.` for rests (a rest of 1e-6 beats or less, like a generator's `R:0`, is nothing); `:beats` duration; `@vel` velocity; `|` bar lines (ignored).
- **Note bends**: `>target` makes a note glide from its own pitch to the target over its held length, then hold it: `C4:2>G3` (or `C4>G3:2`), `A1:4>-12` (signed semitones, fractions allowed), `[C4 E4 G4]:2>+2` (the whole chord), `[C4>D4 G4]:2` (one chord note). `automate <track>.bend_curve` (−1..1) shapes the glide: 0 is a straight line in pitch, positive moves early and settles, negative holds and falls late. Synth, vox (the carrier) and sampler tracks bend.
- **Automation**: `automate <param>` starts a curve track. The first token is the starting value; `V:D@shape` ramps to `V` over `D` beats; `R:D` holds. Shapes: `lin`, `exp` (geometric — right for frequencies), `log`, `smooth`, `step`.
- **Parameters**: `patina --params` prints the complete chart — every automatable name with its range, taper, and MIDI CC, straight from the single source-of-truth table in `src/song.rs` (`param_table!`). Highlights: `waveform` (0–3), `vox_mode` (0 TalkBox / 1 vocoder / 2 Talker / 3 spectral), `vox_level` (0–2: the band vocoder's per-band tanh caps its own level, so post-makeup headroom is the only push), `chorus_mode` (0–4), the 909's `bd_*`/`sd_*`/`hh_*` knobs (per kit track via `automate <track>.hh_tune`, or unscoped to every kit), and the tape deck's `smp_*` transport (per sampler track via `automate <track>.smp_pitch`, or global to all slots).
- **Mixer strips**: every track owns a desk channel. On the track line or as a lane (`automate bass.gain`): `gain=` (linear 0–4: 0 mutes, 2 is +6 dB, 4 is +12 dB), `pan=`, the sends (`reverb_send=` `spring_send=` `chorus_send=` `echo_send=`), and in signal order:
  - **EQ** `eq_low=` `eq_mid=` `eq_high=` (dB, 0 = out) at `eq_low_freq=` (100 Hz) `eq_mid_freq=` (1 kHz, `eq_mid_q=` 0.7) `eq_high_freq=` (8 kHz) — low shelf, peak, high shelf.
  - **Drive** `dist=` (0–1, 0 = out) through `dist_mode=` 0 tanh · 1 hard clip · 2 foldback (all three antialiased) · 3 bitcrush (16 → 2 bits) · 4 sample-rate reduce (down 7 octaves), blended by `dist_mix=`. The shapers drive up to +36 dB and give half of it back, so they thicken rather than just get louder: a −9 dBFS supersaw chord at `dist=0.5` (tanh) loses 2.3 dB of peak, gains 4.4 dB of RMS, and its crest goes from 14.4 to 7.7 dB. The two lo-fi modes keep the level. The EQ, drive and compressor hear the whole track (a chord, not each note), once a sample.
  - **Fader**, times the **duck** and the **shaper**. `duck=` (depth 0–1: 0.5 = −6 dB) dips the strip on every kick, or on the notes of one track with `duck_from=kick` (`duck_from=beat.BD` keys off one drum voice, or one note, of a track), shaped by `duck_attack=` `duck_hold=` `duck_release=` (seconds). `shape=` is a tempo-synced volume shaper: a curve (`pump` `saw` `ramp` `sine` `square` `tri`, one beat per cycle) or a trance gate (`shape=x.x.xx.x`: `x` open, `.` shut, `0`–`9` ninths, a sixteenth per step), with `shape_beats=` (cycle length), `shape_depth=` and `shape_smooth=` (step edges, seconds).
  - **Compressor** `comp=` (threshold dBFS, 0 = out; 4:1, 0.1 ms, auto release, auto make-up), voiced by the bus compressor's own knobs on the strip: `comp_ratio=` `comp_attack=` `comp_release=` `comp_makeup=` `comp_mix=` `comp_sc_hpf=` (`automate drums.comp_ratio`; unscoped they turn the bus compressor).
  ```
  track kick kit=909
  track bass patch=deepsine gain=1.6 duck=0.7 duck_from=kick duck_hold=0.05 duck_release=0.12
  track pad patch=aurora shape=x.x.xx.x. shape_depth=0.9 eq_low=-6 eq_high=3
  track drums kit=909 dist=0.4 dist_mode=1 comp=-18 comp_ratio=2
  ```
- **Cross-modulation**: one track's signal drives another at audio rate. The route sits on the track it drives and names its source (any track, anywhere in the song, itself included); every depth is a lane (`automate lead.fm`):
  - `fm_from=SRC fm=` linear FM: the track's oscillators run at f0·(1 + fm·m), m being SRC's signal (1 = 0 dBFS). A sine of level A at f_m puts sidebands at f0 ± k·f_m with Bessel amplitudes J_k(fm·A·f0/f_m); the frequency stops at 0 Hz (no through-zero).
  - `filter_fm_from=SRC filter_fm=` the cutoff moves `filter_fm` octaves (−8..8) per unit of m.
  - `ringmod_from=SRC ringmod=` the track times (1 − d + d·m): 1 is a ring modulator (the carrier is gone, f ± f_m are left), 0.5 is AM.
  - `feed_from=SRC feed=` routed feedback: SRC's signal comes into this track's strip ahead of its glitch, EQ and drive, through `feed_time=` (delay, 0–2 s), `feed_tone=` (lowpass Hz, 20000 = open), a DC blocker, the gain `feed=` (0–2) and a safety limiter at `feed_limit=` (dBFS, default −6). A loop (a track feeding itself, or two tracks each other) with gain over 1 rings up to the limiter's ceiling and stays there, never past it.

  FM and filter FM need a synth, vox (its carrier) or sampler track; ring and feed work on every track. The source is SRC's strip after its inserts and before its fader, so `gain=0` silences a modulator you only want to hear through the track it drives, and a track with no notes can take a feed as an effects bus. Routes read their sources one sample late, so any graph runs in one pass whatever the track order, and stems keep every route.
  ```
  track mod patch=deepsine gain=0                  # heard only through lead
  track lead fm_from=mod fm=3 ringmod_from=mod ringmod=0.2
  track loop feed_from=loop feed=1.2 feed_time=0.0023 feed_tone=3000 dist=0.3
  ```
- **Master**: `automate limit` sets a look-ahead brickwall limiter's threshold (dBFS, 0 = out — the mix comes up by −threshold and is held under `limit_ceiling`, default −1 dBFS, released over `limit_release` seconds); `automate clip` drives a soft-knee clipper into the same ceiling first (dB, 0 = out). Engaged, this stage replaces the engine's tanh soft limit, so the ceiling is the bounce's true peak, and a song that limits or clips anywhere runs it from the downbeat. The threshold is absolute, so how much it takes off depends on how hot the raw mix runs (a normalized bounce prints it: `Normalized: peak 0.447`, about −7 dBFS): the loudest peak loses about raw peak − threshold − ceiling dB. On a dense 128 bpm loop with a −7 dBFS raw peak and 15.6 dB crest: `limit -12` gives 11.9 dB crest, `limit -16` 9.6, `limit -20` 8.3, and `clip 8` with `limit -12` 8.2 (−7.8 LUFS).
- **Range warnings**: a value the loader has to clamp is reported, one line each, at load: any parameter outside its range (`patina --params`) on a track line, in a lane, a `during` block or a `patch=` file, and the gate, `vel=`, a note's `@` velocity and the sampler's options. `warning: line 12: gain 6 clamped to 4`.
- **Kits**: every `track NAME kit=909` is a 909 board of its own on its own channel, so two kit tracks can carry different tuning, decay, pan, level, track compressor and sends, and each gets its own stem. `automate NAME.sd_tune` (any `bd_`/`sd_`/`rs_`/`cp_`/`hh_`/`ch_`/`oh_`/`dr_` knob) turns that track's board alone; the five voice levels (`bd_level` `sd_level` `rs_level` `cp_level` `hh_level`) are linear gains to 4 (+12 dB over the panel's full level of 1) from a song, while a MIDI CC or plugin knob still sweeps the stock 0–1 (at the default `hh_level` 0.7 a closed hat peaks about 17 dB under the kick; at 4, about 2 dB under); unscoped, a drum knob turns every board. `kit=OTHER` plays the board of an earlier kit track `OTHER` instead (one machine, strip and stem; its closed hat still chokes the other track's open hat).
- **Sections**: `section NAME A..B` names a beat range. `>NAME` / `>NAME.end` seek any track there, and `automate <param>: <value> during NAME,NAME2 [base <b>]` brackets sections without hand-done beat arithmetic.
- **Bounces**: `--play x.song --render out.wav` (add `--no-normalize` to keep the engine's exact gain; peak/RMS/LUFS are reported either way). `--render-stems <dir>` solos each track through the full engine, un-normalized, and prints a per-stem level table — measured mixing without text-splitting song files. Every offline render also reports its peak concurrent voice count.

## 🗣️ The voice box

A vox track's notes are the vocoder's **carrier**; its `=lyrics` drive the
**modulator** — the built-in formant voice, or any recording:

```
track choir vox
[A2 E3 A3]:2=HH-OW1-L-D [G2 D3 G3]:2=AA-N | [A2 E3 A3 C4]:6=HH-OW1-M.

track voice vox wav=renders/borrowed.wav wav_at=0   # a recording played on the keys
[D3 A3 D4 F4]:2 [C3 G3 C4 E4]:2
```

The recording starts at the **first vox note-on**. `wav_at=<beat>` pins that
anchor: if editing ever moves the first vox note, the song errors instead of
silently shifting the vocal against every other clock. A `pitch=curve.wav`
performance line (float32-only, MIDI note numbers on the modulator's clock)
rides the same anchor.

Lyrics are dash-joined ARPAbet phonemes riding their note. Onsets speak at
note-on, the vowel sustains while held (pitch = lowest held key), codas
land on the release. Per phoneme: `:ms` fixed length, `@amp` loudness,
stress digits on vowels (`OW1` primary, `AH0` reduced) shaping loudness,
length, and pitch accents. A trailing `.` or `?` ends the phrase with a
fall or rise. `vox_intonation` scales the voice's own prosody (accents,
declination, final falls): keep it low when singing, high when speaking.
`vox_formant` (−24..24 semitones) moves the voice's throat without moving
its pitch: the formant voice scales every resonance, and a `wav=`
recording plays through throat-only PSOLA (its pitch and clock untouched).
Negative is a bigger mouth, positive a smaller one; automate it like any
vox knob (`automate choir.vox_formant`).

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

- **Options**: `root=` (key of natural speed), `start=`/`end=` (trim), `loop` or `loop=a:b` + `xfade=` (sustain loop, equal-power crossfade), `chop=N` (slice pads, natural speed, one-shot), `beats=N` (varispeed the region/loop to span exactly N beats at the song tempo — drop a breakbeat in and it locks), `bits=`/`rate=` (vintage converters + un-reconstructed ZOH playback; `bits=12 rate=26040` is the SP-1200, `bits=8` the early Fairlight school), `cutoff=`/`res=` (the slot's resonant lowpass), `mode=gate|oneshot`, `reverse`, `fixed` (no keytracking), `mono`/`choke`, `gain=`, `pan=`, `pitch=`, `attack=`, `release=`, `vel_amt=`, and the throat options below.
- **The throat (formants)**: varispeed moves a voice's formants with its pitch, so an octave down sounds like a monster and an octave up like a chipmunk. Three ways out, all on pitch-synchronous grains cut at the recording's glottal pulses:
  - `formant` keeps the recording's own formants at every key while the tape keeps its varispeed time (an octave down is half speed with the same mouth). `formant_keep=K` keeps that fraction of them (0..1).
  - `psola` keeps the formants AND the time: notes change only the pitch (loop and reverse do not apply).
  - `formant=S` (−24..24 semitones) moves the formants on top of either, or alone: `formant=-5` is a bigger throat on the same notes.
  - Automate them as `smp_formant` and `smp_formant_keep` (`automate vox.smp_formant`). Formant heads loop and reverse like tape heads.

```
track vox sample=renders/throat.wav root=C4 formant      # a melody across the keys, one throat
C4:2 G3:2 D#3:4 C3:8                                     # C3 is half speed, not a monster
automate vox.smp_formant                                 # then shrink the throat over 8 beats
0 R:8 7:8@smooth
```
- `songs/magnetic-memory.song` is the demo: the instrument sampling its own bounce of *Hold On* — looped tape choir, vocal chops, reverse swells, and a tape-stop ending.

## 🥊 Kick-bass, walls and risers

Every voice card has a pitch envelope, a drive stage, a supersaw bank and
its own noise oscillator. All of them are knobs like `cutoff`: set them in
a patch, or automate them per track (`automate kick.pitch_env_time`).

```
track kick patch=hardkick          # dives 4 octaves onto each note, driven, filtered
(F1 F1 G#1 F1 | C2 C2 G#1 D#1)     # ...and the tail is the bassline

automate wall.supersaw_detune      # open a supersaw wall
0.2 0.55:32@smooth
automate riser.noise_cutoff        # sweep a track's own noise
300 9000:16@exp
```

- **Pitch envelope**: `pitch_env` (semitones above the note where each gated note starts, −48..96, 0 = off), `pitch_env_time` (seconds to land, 0.001..4), `pitch_env_curve` (−1..1: 0 is straight in octaves, positive drops fast and settles long). The tail sits on the note's own pitch, so the dive and the tail shape separately. Legato moves glide without re-diving. `key_sync 1` restarts the oscillators at one phase on every gate so every hit clicks the same.
- **Drive stage**: `shaper` (0..48 dB) into `shaper_mode` 0 tanh, 1 hard clip, 2 sine wavefolder, between the mixer and the ladder, so the ladder is the post-filter. All three curves are antialiased (ADAA); 0 dB is out and costs nothing.
- **Supersaw**: `supersaw` (0..1) crossfades oscillator 1 into seven polyBLEP saws riding its exact pitch (glide, vibrato, the pitch envelope and bends move all seven), `supersaw_detune` spreads them, `supersaw_mix` brings the six side saws up against the centre at constant loudness. `osc1_level` sets oscillator 1's level in the mixer.
- **Noise oscillator**: `noise_osc` (0..1) is the card's own white noise through its own resonant filter, `noise_cutoff` (Hz), `noise_res`, `noise_mode` (0 lowpass, 1 bandpass, 2 highpass). Unlike `noise` (one shared source into every voice), each card's noise is its own.
- Patches: `hardkick` (hardstyle kick-bass), `rawkick` (wavefolded screech), `kick808` (long 808), `superwall`, `riser`. `songs/kick-bass.song` is the demo.

## 💥 Glitch

Three glitch inserts: a beat repeat, a reverser and a tape stop. Automate them on one track (`automate kick.stutter`, on that track's strip) or with no track across the whole mix bus (`automate stutter`, after the tape, so the finished mix catches and stops). Lengths are in notes and beats and follow the tempo lane.

```
automate kick.stutter              # the roll into a drop: the kick at beat 12 is
0 >12 8 >13 16 >14 32 >15 64 >16 0 # caught and repeated as 1/8, 1/16, 1/32, 1/64
automate stab.reverse              # the stab backwards, a beat behind
0 >8 4 >16 0
automate tape_stop                 # the whole mix winds down over 3 beats
0 >25 3
```

- **Beat repeat**: `stutter N` catches the next 1/N note from the moment it reaches N ≥ 1 (8 an eighth, 16 a sixteenth, 64 a 64th, 12 an eighth-note triplet) and repeats it until it returns to 0. The first pass is live, so the repeat keeps the downbeat it caught. Raising N while it runs shortens the repeat on the caught audio.
- **Reverse**: `reverse N` plays the track backwards in 1/N-note slices, one slice behind, so it sounds the moment it engages. (The tape deck's `reverse` option flips a reel; this flips whatever the strip hears.)
- **Tape stop**: `tape_stop B` winds the tape down to a halt over B beats at a constant rate, pitch and level falling together, then holds silence until it returns to 0 and the music comes back live.
- Every seam (each repeat, each reversed slice, switching in and out) is declicked. A song that never touches them renders exactly as before. `songs/glitch.song` is the demo.

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

See [engine and bounce performance](docs/performance.md) for streaming-render
memory behavior, measured CPU costs, and benchmark commands.
