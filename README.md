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
- 🗜️ A mix-bus "glue" compressor modeled on the SSL 4000 G-series centre-section bus compressor, printing to the tape: feed-forward, stereo-linked, a Blackmer (dbx 202-family) VCA with its real even-order colour, a log-converter soft knee, a ratio switch (2/4/10:1; a song or patch sets any ratio from 1 to 20:1), attack (0.1–30 ms) and release (0.1–1.2 s) switches with a dual-time-constant program-dependent AUTO, make-up, parallel mix, a sidechain high-pass, and a moving-coil gain-reduction meter on the panel. Out by default — and out means bit-identical (`comp_in`, `comp_threshold`, … in songs and patches)
- 🥁 A TR-909 rhythm section built from the service-manual circuits — swept bridged-T kick with click path and waveshaper, twin-shell snare with snappy noise, three-mode rim knock, flam-envelope clap, and a no-samples hi-hat (six-oscillator metal bank through the 909's swept high-pass and choke VCAs) — living on the same volt bus, effects, and power rail as the keyboard voices. Modern range extensions: kick DRIVE from clean sub to full grit, SWEEP depth, rumble-length decays, and an antialiased bus drive. Triggered from drum tracks in songs (`track beat kit=909` with `BD SD RS CP CH OH`; every kit track is a board of its own), MIDI channel 10 (GM map), pads on the panel, or the plugin
- 🗣️ A voice box: a Klatt-style formant speech synthesizer (glottal pulse + cascade resonators, ARPAbet phonemes) driving a 20-band channel vocoder whose carrier is the synth's own voices, with a VSM-201-style voiced/unvoiced switch that snaps the carrier to noise on consonant frames so words stay legible. Two circuits on the mode switch: the '97 DigiTech Talker "TalkBox" voicing (tube-choked lows, honky mids, instant articulation, amp grit) and a full-range studio board. Lyrics ride the notes in songs (`track choir vox`, `[A2 E3 A3]:2=HH-EH-L-OW`) with per-phoneme duration and dynamics; `wav=` feeds any recorded voice through the same circuit, and `patina --say "HH-AH-L-OW"` speaks from the command line
- 📼 A tape-deck sampler: any WAV becomes an instrument on the keys (`track keys sample=tape.wav`) — band-limited windowed-sinc playback (Kaiser β=8.6, kernel stretched by the read speed so varispeed doesn't alias in *either* direction), sustain loops with equal-power crossfades (a Mellotron whose tape never runs out), `chop=N` slice pads mapped chromatically (the MPC workflow), `beats=N` tempo-fitting (break-matching solved at parse), a per-slot resonant ZDF lowpass (the SP/Akai filter-sweep sound), vintage converter emulation (`bits=12 rate=26040` is the SP-1200: band-limited decimation at load, truncation, and a zero-order-hold DAC whose imaging spray is the point), reverse transport, gate/one-shot modes, per-note choke (`mono`), and a live-automatable transport: `smp_pitch` is a varispeed knob (tape-stops on demand), `smp_start` scrubs the needle-drop, `smp_cutoff`/`smp_res` sweep the filter, `smp_gain`/`smp_pan`/`smp_attack`/`smp_release` reshape it mid-song. The heads mix onto the same volt bus as everything else — sampler playback sags the power rail, runs through the fuzz/spring/reverb/chorus/tape chain, and bends with the pitch wheel
- 🥊 Kick-bass on any voice: a per-note pitch envelope (start, time, curve) diving onto the note, key sync, and an antialiased drive stage (tanh, clip, wavefolder) ahead of the ladder as post-filter; plus a seven-saw supersaw bank and a filtered noise oscillator per card for walls and risers
- 🫦 Note bends (`C4:2>G3` glides to the target over the note), and the throat: pitch-synchronous grains that move a voice's formants apart from its pitch, so a sampled voice drops an octave without turning into a monster (`formant`, `smp_formant`, `vox_formant`)
- 💥 Glitch inserts on any track, any bus or the whole mix: beat repeat from 1/8 to 1/64 that can halve itself into a roll, reverse in note slices, and a tape stop with a curve (`stutter`, `stutter_halve`, `reverse`, `tape_stop`, `tape_stop_curve`)
- 🌀 Generative automation lanes: `{expr}` segments of time at control rate, with noise, step sequences and chaotic systems (Lorenz, Rössler, logistic and Hénon maps, a Kuramoto swarm of coupled oscillators, a double pendulum)
- 🔀 Audio-rate cross-modulation between tracks: any track's signal as another's FM, filter FM or ring modulator, and routed feedback loops between tracks through a delay, a tone filter and a safety limiter (`fm_from=`, `filter_fm_from=`, `ringmod_from=`, `feed_from=`)
- 🌾 A granular engine on any track or the whole mix bus: a live capture buffer played back as a cloud of grains with size, density, position, pitch and pan scatter, freeze and reverse probability (`gran`, `gran_size`, `gran_freeze`, ...)
- 🌈 Spectral inserts on any track or the mix bus: freeze, blur, frequency shift and pitch shift, spectral gate and tilt, and a morph that crossfades one track's spectrum into another's (`spec_freeze`, `spec_morph` with `spec_from=`, ...)
- 🎧 Binaural placement on any track, for headphones: azimuth, elevation and distance around a modelled head (a spherical head with pinna and shoulder echoes, generated, no measured dataset), motion paths, and the Doppler bend of a moving source (`azimuth`, `distance`, `orbit`, `pos_x`, `doppler`, ...)
- 🔔 Bodies: any track or the whole mix bus heard inside a wine glass, a steel box, a wooden crate, a tube, a room, a hall or a cathedral, by convolution with impulse responses generated from each body's physics, and morphs from one body into another (`body=glass..cathedral`, `body_morph`)
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

- **Notes**: names (`C4`, `F#3`, `Eb5`, with C4 = MIDI 60) or raw MIDI numbers; `[..]` for chords; `R` or `.` for rests (a rest of 1e-6 beats or less, like a generator's `R:0`, is nothing); `:beats` duration; `@vel` velocity; `|` bar lines (ignored); `>16` as a token of its own seeks the lane to beat 16, forwards or back, on every kind of track and lane alike (`->16` is not a seek; its error says to write `>16`).
- **Note bends**: `>target` makes a note glide from its own pitch to the target over its held length, then hold it: `C4:2>G3` (or `C4>G3:2`), `A1:4>-12` (signed semitones, fractions allowed), `[C4 E4 G4]:2>+2` (the whole chord), `[C4>D4 G4]:2` (one chord note). `automate <track>.bend_curve` (−1..1) shapes the glide: 0 is a straight line in pitch, positive moves early and settles, negative holds and falls late. Synth, vox (the carrier), sampler and instrument tracks bend.
- **Automation**: `automate <param>` starts a curve track. The first token is the starting value; `V:D@shape` ramps to `V` over `D` beats; `R:D` holds. Shapes: `lin`, `exp` (geometric — right for frequencies), `log`, `smooth`, `step`.
- **Phasing**: `automate <track>.phase` moves one track's notes off where they are written, in beats: a note written at beat `b` plays at `b + phase(b)` (negative = ahead). A linear ramp is a constant-rate drift and a hold is a lock, so `0 R:96 -0.5:36 R:96` is one Reich phase step: one eighth ahead over six 6-beat figures, then locked. Only that track's notes move (their bends and lyrics with them); automation stays on the song clock. A ramp that falls one beat per beat or faster is refused (the track would stop, then run backwards). `songs/tessera.song` is the demo.
- **Generative lanes**: `{expr}:D` in any lane follows an expression for D beats at control rate (128 steps a beat, fine enough that fast lanes leave no zipper; `{expr}:D@N` takes N steps), and `{expr}` alone runs to the end of the song. It can open a lane or follow any token, and the next token starts from its last value. An expression reads the clock — `t` (seconds, through the tempo lane), `b` (beats), `s` (beats into the segment), `p` (0..1 through it), `v` (the lane's value where it began) — with `+ - * / % ^`, comparisons, `&& || !`, `c ? a : b`, the usual math (`sin cos exp log sqrt abs floor round min max clamp mix pow smoothstep snap mtof db` …), cycle shapes one period a unit (`sine tri saw square`), `step(x)`, `seq(i, a, b, …)` (a step sequence), `noise(x[,seed])` (smooth) and `rand(x[,seed])` (a new 0..1 each integer), and chaotic systems as pure functions of their time argument: `lorenz(time[,axis,rho,sigma,beta])`, `rossler(time[,axis,a,b,c])`, `logistic(n, r[,x0])` (period doubling from r = 3, chaos past 3.57; a map swept slowly through a doubling lingers on the old cycle, since nothing nudges it off, so step `r` for clean regimes), `henon(n[,axis,a,b])`, `kuramoto(time, k[,which,n,spread])` (n coupled oscillators: their mean field, their coherence with `which=-1`, or one of them) and `pendulum(time[,axis,damping])` (a double pendulum). `automate x: {expr} during CH1` runs it through each section. The tempo and phase lanes take expressions of `b` with a duration (not `t`, since they lay out the clock and the song's end); a phase expression is held to the phase ramp rule.
  ```
  automate pad.cutoff
  800 {2000 + 1500 * lorenz(t / 2)}:32 R:4 600:4@exp
  automate bass.filter_env
  {2 * logistic(b * 4, seq(b / 8, 2.8, 3.2, 3.5, 3.9))}:32   # 1, 2, 4 values, then chaos
  automate lead.pan
  {kuramoto(t, 2 + 6 * p, 3)}              # one voice of a swarm falling into step
  ```
  A value that holds across steps (a `seq`, a stepped map) sends one event, not many, and every jump lands through the engine's knob smoothing, so stepped and chaotic lanes move without clicks. Demo: `songs/generative.song` (a Kuramoto swarm panning three arpeggios into step and apart, an acid line on the logistic map's period doubling, hats accented by the map's chaos, a bell FM'd through a double pendulum).
- **Parameters**: `patina --params` prints the complete chart — every automatable name with its range, taper, and MIDI CC, straight from the single source-of-truth table in `src/song.rs` (`param_table!`). Highlights: `waveform` (0–3), `vox_mode` (0 TalkBox / 1 vocoder / 2 Talker / 3 spectral), `vox_level` (0–2: the band vocoder's per-band tanh caps its own level, so post-makeup headroom is the only push), `chorus_mode` (0–4), the 909's `bd_*`/`sd_*`/`hh_*` knobs (per kit track via `automate <track>.hh_tune`, or unscoped to every kit), and the tape deck's `smp_*` transport (per sampler track via `automate <track>.smp_pitch`, or global to all slots).
- **Mixer strips**: every track owns a desk channel. On the track line or as a lane (`automate bass.gain`): `gain=` (linear 0–4: 0 mutes, 2 is +6 dB, 4 is +12 dB), `pan=`, the sends (`reverb_send=` `spring_send=` `chorus_send=` `echo_send=`), and in signal order:
  - **EQ** `eq_low=` `eq_mid=` `eq_high=` (dB, 0 = out) at `eq_low_freq=` (100 Hz) `eq_mid_freq=` (1 kHz, `eq_mid_q=` 0.7) `eq_high_freq=` (8 kHz) — low shelf, peak, high shelf.
  - **Drive** `dist=` (0–1, 0 = out) through `dist_mode=` 0 tanh · 1 hard clip · 2 foldback (all three antialiased) · 3 bitcrush (16 → 2 bits) · 4 sample-rate reduce (down 7 octaves), blended by `dist_mix=`. The shapers drive up to +36 dB and give half of it back, so they thicken rather than just get louder: a −9 dBFS supersaw chord at `dist=0.5` (tanh) loses 2.3 dB of peak, gains 4.4 dB of RMS, and its crest goes from 14.4 to 7.7 dB. The two lo-fi modes keep the level. The EQ, drive and compressor hear the whole track (a chord, not each note), once a sample.
  - **Fader**, times the **duck** and the **shaper**. `duck=` (depth 0–1: 0.5 = −6 dB) dips the strip on every kick, or on the notes of one track with `duck_from=kick` (`duck_from=beat.BD` keys off one drum voice, or one note, of a track), on the audio of one track with `duck_from=loop.onset` (an onset detector on its raw output: a fast peak follower jumping 6 dB over a 100 ms one, above −50 dBFS, at most one per 40 ms; a `gain=0` track or a bus keys it too), or on a list of beats with `duck_at=4,8,12.5,16..32/0.5,DROP..DROP.end` (beats, sections, and `A..B/step` ranges up to B; a listed strip skips the kick unless `duck_from=` asks), shaped by `duck_attack=` `duck_hold=` `duck_release=` (seconds). `shape=` is a tempo-synced volume shaper: a curve (`pump` `saw` `ramp` `sine` `square` `tri`, one beat per cycle) or a trance gate (`shape=x.x.xx.x`: `x` open, `.` shut, `0`–`9` ninths, a sixteenth per step), with `shape_beats=` (cycle length), `shape_depth=` and `shape_smooth=` (step edges, seconds).
  - **Compressor** `comp=` (threshold dBFS, 0 = out; 4:1, 0.1 ms, auto release, auto make-up), voiced by the bus compressor's own knobs on the strip: `comp_ratio=` `comp_attack=` `comp_release=` `comp_makeup=` `comp_mix=` `comp_sc_hpf=` (`automate drums.comp_ratio`; unscoped they turn the bus compressor). `comp_ratio` is the ratio itself, N:1 from 1 to 20 (`comp_ratio=6` is 6:1; the panel's switch has 2, 4 and 10). Before dd3d1c8 (11-05) it was the switch's position: 0 = 2:1, 1 = 4:1, 2 or more = 10:1. To convert an old song or patch, write 2 for 0, 4 for 1 and 10 for 2 or more, in `comp_ratio=` options and `automate comp_ratio` lanes alike; unconverted, an old 1 plays 1:1, a 2 plays 2:1, a 4 plays 4:1, and a 0 clamps to 1 with a warning.
  ```
  track kick kit=909
  track bass patch=deepsine gain=1.6 duck=0.7 duck_from=kick duck_hold=0.05 duck_release=0.12
  track pad patch=aurora shape=x.x.xx.x. shape_depth=0.9 eq_low=-6 eq_high=3
  track drums kit=909 dist=0.4 dist_mode=1 comp=-18 comp_ratio=6
  ```
- **Cross-modulation**: one track's signal drives another at audio rate. The route sits on the track it drives and names its source (any track, anywhere in the song, itself included); every depth is a lane (`automate lead.fm`):
  - `fm_from=SRC fm=` linear FM: the track's oscillators run at f0·(1 + fm·m), m being SRC's signal (1 = 0 dBFS). A sine of level A at f_m puts sidebands at f0 ± k·f_m with Bessel amplitudes J_k(fm·A·f0/f_m); the frequency stops at 0 Hz (no through-zero).
  - `filter_fm_from=SRC filter_fm=` the cutoff moves `filter_fm` octaves (−8..8) per unit of m.
  - `ringmod_from=SRC ringmod=` the track times (1 − d + d·m): 1 is a ring modulator (the carrier is gone, f ± f_m are left), 0.5 is AM.
  - `feed_from=SRC feed=` routed feedback: SRC's signal comes into this track's strip ahead of its glitch, EQ and drive, through `feed_time=` (delay, 0–2 s), `feed_tone=` (lowpass Hz, 20000 = open), a DC blocker, the gain `feed=` (0–2) and a safety limiter at `feed_limit=` (dBFS, default −6). The limiter is a saturator, as a real feedback loop's amplifier is: untouched below its knee (4.4 dB under the ceiling), a smooth shoulder above it, run 2x oversampled so a saturated loop's harmonics don't fold back and circulate, behind a gain rider that keeps it from being driven past the ceiling however high `feed=` goes. A loop (a track feeding itself, or two tracks each other) with gain over 1 settles into a steady tone just under the ceiling and never passes it; tune a loop to a note's period and only that note's harmonics ring (the loop runs a sample or two longer than its `feed_time`: the route reads a sample late and the tone filter lags).

  Sound: an FM'd synth card runs its oscillators 4x oversampled and the ring modulator multiplies at 2x, so neither folds its sidebands back off Nyquist (a sawtooth swept 0–2x by FM: the folded lines about 65 dB under the true ones, against about 40 at the base rate; a ring modulator's sum tones above 29 kHz filtered instead of folding); every depth, the feed's tone and its ceiling glide over 20 ms, and `feed_time` glides like tape. FM and filter FM need a synth, vox (its carrier) or sampler track; ring and feed work on every track. The source is SRC's strip after its inserts and before its fader, so `gain=0` silences a modulator you only want to hear through the track it drives, and a track with no notes can take a feed as an effects bus. Routes read their sources one sample late, so any graph runs in one pass whatever the track order, and stems keep every route.
  ```
  track mod patch=deepsine gain=0                  # heard only through lead
  track lead fm_from=mod fm=3 ringmod_from=mod ringmod=0.2
  track loop feed_from=loop feed=1.2 feed_time=0.0023 feed_tone=3000 dist=0.3
  ```
  Demo: `songs/cross-mod.song` (a wire ringing on its own feedback, a bass filter driven by the kick, an FM bell, a ring-modulated pad, two leads feeding each other).
- **Master**: `automate limit` sets a look-ahead brickwall limiter's threshold (dBFS, 0 = out — the mix comes up by −threshold and is held under `limit_ceiling`, default −1 dBFS, released over `limit_release` seconds); `automate clip` drives a soft-knee clipper into the same ceiling first (dB, 0 = out). Engaged, this stage replaces the engine's tanh soft limit, so the ceiling is the bounce's true peak, and a song that limits or clips anywhere runs it from the downbeat. The threshold is absolute, so how much it takes off depends on how hot the raw mix runs (a normalized bounce prints it: `Normalized: peak 0.447`, about −7 dBFS): the loudest peak loses about raw peak − threshold − ceiling dB. On a dense 128 bpm loop with a −7 dBFS raw peak and 15.6 dB crest: `limit -12` gives 11.9 dB crest, `limit -16` 9.6, `limit -20` 8.3, and `clip 8` with `limit -12` 8.2 (−7.8 LUFS).
- **Range warnings**: a value the loader has to clamp is reported, one line each, at load: any parameter outside its range (`patina --params`) on a track line, in a lane, a `during` block or a `patch=` file, and the gate, `vel=`, a note's `@` velocity and the sampler's options. `warning: line 12: gain 6 clamped to 4`.
- **Kits**: every `track NAME kit=909` is a 909 board of its own on its own channel, so two kit tracks can carry different tuning, decay, pan, level, track compressor and sends, and each gets its own stem. `automate NAME.sd_tune` (any `bd_`/`sd_`/`rs_`/`cp_`/`hh_`/`ch_`/`oh_`/`dr_` knob) turns that track's board alone; the five voice levels (`bd_level` `sd_level` `rs_level` `cp_level` `hh_level`) are linear gains to 4 (+12 dB over the panel's full level of 1) from a song, `hh_level` to 16 (+24 dB), while a MIDI CC or plugin knob still sweeps the stock 0–1 (a closed hat is a short burst of high noise, so its peak says little about how loud it sounds: at the default `hh_level` 0.7 its hit carries about 28 dB less K-weighted energy than a default kick, at 4 about 13 dB less though it peaks only 1 dB under, and at 16 it is within about 1 dB of the kick); unscoped, a drum knob turns every board. `kit=OTHER` plays the board of an earlier kit track `OTHER` instead (one machine, strip and stem; its closed hat still chokes the other track's open hat).
- **Sections**: `section NAME A..B` names a beat range. `>NAME` / `>NAME.end` seek any track there, and `automate <param>: <value> during NAME,NAME2 [base <b>]` brackets sections without hand-done beat arithmetic.
- **Bounces**: `--play x.song --render out.wav` (add `--no-normalize` to keep the engine's exact gain; peak/RMS/LUFS are reported either way). `--render-stems <dir>` solos each track through the full engine, un-normalized, and prints a per-stem level table — measured mixing without text-splitting song files. `--bits 16` writes TPDF-dithered 16-bit PCM instead of the default 32-bit float (half the size, plays anywhere), for `--render`, `--render-stems` and `--say`. Every offline render also reports its peak concurrent voice count, and warns once per track when a note found all 64 voice cards (or the tape deck's 24 heads) sounding and had to cut one of that track's, with when it first happened and how many it cut in all: `warning: voice cap: track 'pad' lost a held note at 12.40 s to a note on 'lead' (all 64 voice cards were sounding); 3 held notes and 11 release tails cut in all`. `patina --help` lists every command.

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

## 🪕 Strings and struck bodies

Physical models play on their own tracks (`src/physical.rs`), each with its own strip and stem:

```
track guitar string str_stiff=0.0003 str_decay=4   # plucked
track piano string=strike str_hard=0.7 str_stiff=0.0005
track cello string=bow str_pressure=0.6 str_pos=0.11 attack=0.15
track bell modal=bell modal_bright=0.3 modal_spread=0.6
modes chime 1 2.76:2 5.4:1:0.5                      # a body of your own: ratio:t60:gain
track chimes modal=chime
track plate modal=plate excite=drums modal_decay=2  # the kit rings the plate
track wire modal=string modal_excite=2 modal_stretch=0.002 modal_damp=0.2 modal_spread=0.4 attack=0.4
```

- **String** (`string=pluck|strike|bow|none`): a digital waveguide whose loop is solved to the note's period exactly (every filter's delay counted; the tests hold it within 0.05 cents from 41 Hz to 2 kHz). `str_decay` is the fundamental's T60 in seconds and `str_damp` shortens partial f to T60/(1 + damp·(f/f1 − 1)). `str_stiff` is the inharmonicity B: an allpass cascade (8 to 32 sections, fitted per note) puts partial k at k·f0·√(1 + B·k²): within 2 cents through the 16th partial for B up to 0.001 (piano-like), about 8 cents at 0.002; a stiffer low string runs out of loop and stretches less than asked. `str_pos` is where the pluck, hammer or bow touches (harmonics with a node there drop out), `str_hard` the plectrum or felt, `str_pressure` the bow (velocity is bow speed, so loudness), `str_mute` how hard a note-off damps. A bow speaks in clean Helmholtz motion near the bridge (`str_pos` 0.08 to 0.15) and breaks into multi-slip further out.
- **Modal bank** (`modal=bar|glass|plate|bell|string|gong|cymbal`, or a `modes` line): up to 512 two-pole resonators, a line-for-line port of Hypha's reference (`tests/data/spec/modal.py`, matched sample for sample by the tests). `modal_decay` scales every T60, `modal_damp` shortens the high modes, `modal_stretch` makes ratios inharmonic, `modal_bright` tilts the gains, `modal_spread` puts odd modes left and even right, `modal_modes` sounds the lowest N. `modal_excite` picks a mallet strike (`modal_hard`), a noise burst (`modal_burst` seconds), a noise bow that sustains while the note is held, or none.
- **Excite**: `excite=TRACK` drives the body with another track's audio (its raw output, so a source at `gain=0` still drives it), scaled by `excite_gain`. The instrument's own notes tune the body and hold it open; `attack=`/`release=` on the track line shape that gate. A pitched source on a body tuned to it resonates hard: turn `excite_gain` down.
- Every knob is a track option and a lane (`automate wire.str_decay`); unscoped, it turns every instrument that has it. Notes bend (`C4:2>G3`).

## 💥 Glitch

Three glitch inserts: a beat repeat, a reverser and a tape stop. Automate them on one track (`automate kick.stutter`, on that track's strip), on a bus (`automate music.stutter`) or with no track across the whole mix bus (`automate stutter` or `automate mix.stutter`, after the tape, so the finished mix catches and stops). Lengths are in notes and beats and follow the tempo lane. `stutter_halve H` halves a running repeat every H beats, down to a 1/256 note (`stutter 16` with `stutter_halve 0.5` is two 16ths, four 32nds and eight 64ths, an eighth note each). `tape_stop_curve C` shapes the next stop: the speed falls as (1 − t/T)^C, so 1 is the constant rate and 2 drops fast and drags a long tail (0.25 to 4).

```
automate kick.stutter              # the roll into a drop: the kick at beat 12 is
0 >12 8 >13 16 >14 32 >15 64 >16 0 # caught and repeated as 1/8, 1/16, 1/32, 1/64
automate stab.reverse              # the stab backwards, a beat behind
0 >8 4 >16 0
automate tape_stop                 # the whole mix winds down over 3 beats
0 >25 3
automate tape_stop_curve           # ...front-loaded: speed (1 - t/T)^1.6
1.6
```

- **Beat repeat**: `stutter N` catches the next 1/N note from the moment it reaches N ≥ 1 (8 an eighth, 16 a sixteenth, 64 a 64th, 12 an eighth-note triplet) and repeats it until it returns to 0. The first pass is live, so the repeat keeps the downbeat it caught. Raising N while it runs shortens the repeat on the caught audio.
- **Reverse**: `reverse N` plays the track backwards in 1/N-note slices, one slice behind, so it sounds the moment it engages. (The tape deck's `reverse` option flips a reel; this flips whatever the strip hears.)
- **Tape stop**: `tape_stop B` winds the tape down to a halt over B beats at a constant rate, pitch and level falling together, then holds silence until it returns to 0 and the music comes back live.
- Every seam (each repeat, each reversed slice, switching in and out) is declicked. A song that never touches them renders exactly as before. `songs/glitch.song` is the demo; `songs/buses.song` rolls and curves them on buses and the mix.

## 🎚️ Buses

`bus NAME tracks=a,b,c` mixes those tracks' strips, after their faders and pans, into one strip of its own. The bus takes every strip option and lane a track takes: `gain=` `pan=` the sends, `duck=` and `duck_from=`, `shape=` (so `shape=xx.x.xx.x.xx....` is a sixteenth-step gate on the group), the EQ, the drive, `comp=`, and the glitch lanes (`automate music.stutter`). The members' own sends still reach the rooms around the bus, as on a console's group, so a gated bus leaves its reverb tails ungated. A track sits on one bus at most; a bus takes tracks, not other buses, and plays no notes. Its stem (`--render-stems`) is the group through it, and a member's stem goes through its bus.

`bus mix` is the whole mix, after the tape and the bus glitch: it takes `gain=`, the duck (`duck=` `duck_from=` `duck_at=` and its times) and the shape, so `bus mix shape=xx.x.xx.x.xx.... shape_depth=0` with an `automate mix.shape_depth` lane gates the finished two-track, rooms and all, for one bar. `automate mix.stutter` is the same insert as unscoped `automate stutter`.

```
bus music tracks=pad,bass,lead duck=0.7 duck_from=kick.BD duck_release=0.07
bus mix shape=xx.x.xx.x.xx.x.x. shape_depth=0
automate mix.shape_depth           # gate the whole mix for bar 8
0 >28 1 >32 0
automate music.stutter             # roll the group into the drop
0 >31 16 >32 0
automate music.stutter_halve
0.25
```

## 🫨 Feel

`patina feel <song.song | file.wav | file.mid>` plays a song (rendered in memory) or a bounce through a modelled festival rig and reports what a body in the ground right under the subwoofer would feel: the sub feed (mono, 90 Hz Linkwitz-Riley crossover, the cabinet's roll-off), its force on the floor at a calibrated level, the floor's vertical acceleration where you are, and that acceleration weighted the way a body feels it (ISO 2631-1 Wk), against the perception threshold (0.015 m/s²) and the standard's comfort bands.

```
patina feel songs/under-the-sub.song                    # buried 30 cm under a festival sub
patina feel mix.wav --venue house                       # in a boomy timber floor, at home volume
patina feel songs/kick-bass.song --venue club --distance 2 --out /tmp/kb
```

It prints a time-ordered report (still stretches, each section's hits with their peak in g, frequency, ring-down and tempo, and what hums between them) and writes `PREFIX.feel.json` (the 50 fps Wk envelope, every hit, the sections and the settings) and `PREFIX.feel.png`, a seismograph: the Wk peak, 125 ms and 1 s r.m.s. on a log scale against the perception threshold and the comfort bands, every hit ticked above it, still stretches shaded.

- `--venue field|club|house`: soft soil over firmer ground (you buried, `--depth` 0.3 m), a 150 mm concrete slab on grade (`--distance` 1 m), a suspended timber floor that booms at 28 Hz.
- `--level festival|club|home|<dB>`: the sub's peak SPL at 1 m for a full-scale signal: festival 143 dB (the field's default), club 135 (the slab's), home 112 (the house's). Songs and WAVs are normalized to −1 dBFS first, like a mastered track; `--no-normalize` keeps a song's engine gain.
- `--wav PATH` writes the ground motion itself (the vertical acceleration at you, not weighted) as mono 32-bit float audio at 8 kHz, its peak at −1 dBFS, with the scale (what 1.0 is in m/s²) in the report and the JSON: play it through a tactile transducer (a bass shaker on a chair or floor) and feel what the report describes.
- `--crossover HZ`, `--sub-hz HZ`, `--sealed`/`--vented`, `--out PREFIX`, `--json PATH`, `--png PATH`, `--patch NAME` (the voice a MIDI file plays). `patina feel --help` lists them.
- The model, every parameter and its sources: design.md §9. `cargo run --release --example feel_venues` prints each venue's transfer function. `songs/under-the-sub.song` is the demo.

## 🌾 Granular

A grain cloud on one track (`automate pad.gran`, or `gran=0.7` on its track line) or with no track across the whole mix bus (`automate gran`). From the downbeat it records what it hears into a 12-second capture buffer, and plays that back as overlapping Hann-windowed grains, each with its own position, speed, direction and pan.

```
track pad patch=aurora gran=0.7 gran_size=150 gran_density=40 gran_spray=0.4 gran_pan_scatter=0.8
[C3 G3 D4]:16 [A2 E3 B3]:16
automate pad.gran_pos              # read further and further back
0.1 3:16@smooth
automate pad.gran_freeze           # catch the chord at beat 16 and hold it
0 >16 1
```

- `gran` is the wet/dry blend (0 is out, 1 is grains only). `gran_size` (ms, 2 to 2000) and `gran_density` (grains a second, 0.5 to 500) set the texture: density x size grains overlap at once.
- `gran_pos` is how far behind the live input a grain reads (seconds, 0 to 10), and `gran_spray` spreads it at random. Frozen (`gran_freeze 1`), the buffer stops recording, and `gran_pos` scrubs through what it caught.
- `gran_pitch` (semitones) sets each grain's playback speed: a grain an octave up reads twice as much of the recording in the same `gran_size`, so the pitch moves while the cloud keeps the music's tempo. `gran_pitch_scatter` spreads it at random, `gran_pan_scatter` (0 to 1) spreads the grains across the field, `gran_reverse` is the chance a grain plays backwards, and `gran_jitter` moves the onsets from a steady clock (0) to a random Poisson stream (1).
- Level: at density x size = 2 with no scatter, the windows sum to exactly one and the cloud is the input itself, `gran_pos` late. Denser clouds scale by sqrt(2/N), so a thick cloud stays near the input's loudness. A grain never reads past the recording head: pitched-up grains start far enough back not to overtake it.
- Clean at any setting: a grain read faster than the recording reads a copy filtered and kept at half the rate (or a quarter, down to a sixteenth), so a cloud pitched up two octaves does not fold its treble back down as aliases. `gran_freeze` can switch every sixteenth note without a click: a grain that would run into the stopped head, or into audio the restarted head overwrites, fades out over 5 ms, and the seam in the recording is faded too.
- A song that never touches the engine builds none and renders exactly as before; at `gran 0` it is a wire that keeps recording.

## 🌈 Spectral inserts

One short-time FFT processor (2048 points, 75% overlap, ~23 Hz bins at 48 kHz) on a track (`automate pad.spec_freeze`, or `spec_blur=0.5` on its track line) or with no track across the whole mix bus. Every stage at rest passes its bins untouched.

```
track voice patch=cassette-choir gain=0              # muted, but still feeding the morph
[D3 A3 F4]:32
track pad patch=aurora spec_from=voice spec_blur=0.4
[D3 A3 E4]:16 [C3 G3 D4]:16
automate pad.spec_morph                     # the pad turns into the choir
0 1:16@smooth
automate pad.spec_freeze                    # ...and is caught at beat 24
0 >24 1
```

- `spec_morph` (0 to 1) crossfades the magnitudes toward the sound of the track named by `spec_from=NAME` on this track's line. Each bin keeps the phase of whichever side gives it more, so 0 is this track and 1 is NAME exactly. The source is heard before its own strip, so `gain=0` mutes it in the mix while it keeps feeding the morph.
- `spec_blur` (seconds, 0 to 10) smears each bin's level over that time constant: transients soften and endings ring on, as tones at their own pitch rather than as noise.
- `spec_freeze` (0 to 1) catches the frame the moment it leaves 0 and holds it, each bin turning at the frequency it had, so a frozen chord rings steadily instead of buzzing at the frame rate. Between 0 and 1 the held frame and the live one crossfade; back at 0 it lets go, and the next rise catches a new frame.
- `spec_shift` (Hz, -4000 to 4000) adds to every partial's frequency, so harmonics go inharmonic; `spec_pitch` (semitones, -24 to 24) multiplies, a pitch shift that keeps time. Each spectral peak moves with its whole lobe and the bins keep their phases locked to the peak (the Laroche-Dolson phase vocoder), so a shifted tone stays one tone.
- `spec_gate` (dB, 0 is out) keeps only the bins within that many dB of the frame's loudest, so noise and room fall away and the strongest partials stay. `spec_tilt` is dB per octave about 1 kHz. `spec_mix` blends the processed spectrum with the input's.
- The insert delays its track by one frame, 2048 samples (43 ms at 48 kHz). Any track a spectral lane or option touches gets its insert at the downbeat, so the delay is the same all song and never jumps in mid-song.

## 🎧 Binaural

For headphones: a track placed at a point around the listener's head (`azimuth=-60 distance=3` on its track line, or `automate bird.azimuth`), in place of its pan. The head is modelled, not measured: an 8.75 cm sphere with Brown and Duda's head shadow, five pinna echoes that move with elevation and a shoulder reflection. Each ear hears the source along its own path through the air (around the head where the head is in the way), so the time and level differences between the ears, and the Doppler bend of a moving source, come out of the geometry.

```
track bird patch=glintbell azimuth=-60 distance=3
(A5 . E5 . C6 . .)x4
automate bird.azimuth            # once around the head, ahead to behind and back
-60 300:16@smooth
track car patch=sunlead distance=3
A3:8
automate car.pos_x               # passes 3 m ahead, left to right
-40 40:6
```

- `azimuth` is degrees: 0 ahead, 90 right, -90 left, 180 behind, and past 180 it keeps turning. `elevation` is degrees from -90 (below) to 90 (overhead). `distance` is metres from the head's centre (0.15 to 100, default 1): the level falls as 1/d, the air takes the treble (0.15 dB a metre at 10 kHz), and the sound arrives its travel time late.
- Paths: `orbit` keeps the azimuth turning (degrees a second). `pos_x`, `pos_y` and `pos_z` add a straight-line offset in metres (x right, y ahead, z up), for fly-bys. `doppler` (0 to 1, default 1) is how much of the travel time is heard: 1 is physical, so a source closing at 30 m/s rises about 10%; 0 keeps only the head's own time differences.
- Sizes, as measured on real heads: at 90 degrees the far ear hears the sound 0.65 ms late, and up to 16 dB quieter at 6 kHz (under 2 dB at 300 Hz). Straight ahead both ears hear the track at unity.
- The reverb, spring, echo and chorus sends hear the source at one level at any distance, as a room fills the same however far back a source stands, so pulling a track back makes it wetter: the strongest distance cue there is.
- A placement belongs to one track, so an unscoped `automate azimuth` is an error. A song that never places a track renders exactly as before. `songs/binaural.song` is the demo.

## 🔔 Bodies

A track heard inside a modelled object or room: `body=glass` on its track line, or a `body cathedral` line for the whole mix bus. The sound is convolved with the body's impulse response, generated from its physics when the song loads, so nothing is sampled.

```
track bell patch=mallet body=glass..cathedral body_mix=0.8
(E5:0.5 G5:0.5 B5:0.5 D6:0.5)x8
automate bell.body_morph         # from inside a wine glass out into a cathedral
0 1:16@smooth
body hall                        # and the whole mix in a hall, a third wet
automate body_mix
0.3
```

- The bodies: `glass` (a wine glass: the ring modes of a thin shell at 760 Hz and 2.83, 5.42, 8.77... times it, each a split pair that beats slowly), `metal` (a box of 1.2 mm steel panels: their bending modes and the box's cavity), `wood` (a plywood crate with a port), `tube` (a 2 m steel tube's axial resonances), `room`, `hall` and `cathedral`.
- Each response is built from three parts: the body's modes (a frequency, decay time and level each); the cavity's early reflections, traced off its walls by the image-source method; and a diffuse tail at the level the room's absorption gives, with every frequency decaying at its own reverberation time (a cathedral holds 8 s in the bass, 6.5 s in the middle and 3.5 s in the treble). Every response is scaled to unit energy, so a broadband track keeps its loudness inside any body. A steady tone right on a long-ringing mode builds up far louder, as it does in a real glass (a 4-second glass mode is half a hertz wide); the glass's modes sit between the equal-tempered notes.
- `body=room..cathedral` morphs: `body_morph` (0 to 1) runs through 17 bodies, the two ends and 15 interpolated between them, dimensions and decay times moving geometrically and the modes moving in pairs, so halfway between a room and a cathedral is a plausible mid-sized hall, not a crossfade of the two. All of them share the track's input history, so each rings with everything already played into it.
- `body_mix` is the wet share (default 1, all inside the body). The convolution adds no latency. A body lane on a track without `body=`, or an unscoped one without a `body` line, is an error. A song that names no body renders exactly as before. `songs/bodies.song` is the demo.

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
