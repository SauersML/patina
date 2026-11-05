# Rust Analog Synth Design Document

## 1. Overview and Goals

This document outlines the design for a Rust-based analog synthesizer simulator. Our primary goals are:

1. Create a modular, extensible system that can grow from a basic synthesizer to a complex one.
2. Ensure clean separation of concerns between audio processing, UI, and MIDI handling.
3. Maintain real-time performance for audio processing.
4. Allow for easy addition of new features and components.
5. Leverage Rust's strengths in safety, concurrency, and performance.
6. Accurately emulate the sound and behavior of analog synthesizer hardware.

## 2. Architecture Design

We will use a layered architecture with clear interfaces between components:

```
+-------------------+
|    User Interface |
+-------------------+
          |
+-------------------+
|    MIDI Handler   |
+-------------------+
          |
+-------------------+
|    Synth Engine   |
+-------------------+
    |            |
+-------+    +-------+
| Voice |    |Effects|
+-------+    +-------+
    |
+---------------------------+
| Oscillator | Filter | LFO |
+---------------------------+
    |
+-------------------+
|     Envelope      |
+-------------------+
```

## 3. Component Breakdown

1. **Synth Engine**: Coordinates all audio processing.
   - Manages voices using an object pool
   - Applies global effects
   - Handles audio device interaction

2. **Voice**: Represents a single synthesizer voice.
   - Contains Oscillator, Envelope, Filter, and LFO
   - Implemented as a struct with no runtime polymorphism
   - Add slight detuning between voices for a richer sound
   - Implement voice stealing algorithm for polyphony

3. **Oscillator**: Generates raw waveforms.
   - Implemented as a struct with methods for different waveform types
   - Uses atomic types for thread-safe parameter updates
   - Implement analog-style waveform generation with subtle imperfections
   - Add hard sync capability
   - Implement FM (Frequency Modulation) between oscillators
   - Add phase modulation capabilities

4. **Envelope**: Modulates amplitude over time (ADSR).
   - Implemented as a struct with methods for each stage
   - Implement non-linear ADSR curves to mimic analog behavior
   - Add voltage-controlled amplifier (VCA) emulation

5. **Filter**: Shapes the tone of the voice.
   - Uses traits for different filter types
   - Implement multiple filter types (Lowpass, Highpass, Bandpass)
   - Add resonance control
   - Implement self-oscillation capability
   - Add filter overdrive/saturation
   - Emulate analog filter non-linearities and instabilities

6. **LFO (Low Frequency Oscillator)**:
   - Implement various LFO shapes (sine, triangle, square, random)
   - Allow LFO to modulate various parameters (pitch, filter cutoff, etc.)

7. **Effects**: Applies post-processing effects.
   - Implements a trait object-based plugin system for flexibility
   - Add analog-style chorus
   - Implement tape-style delay
   - Add reverb emulation
   - Per-track strip inserts (src/inserts.rs): a three-band EQ, a drive
     stage (tanh / hard clip / foldback, ADAA-antialiased; bitcrush and
     sample-rate reduction), a tempo-synced volume shaper, a sidechain
     duck keyed by any track; and the master clipper and look-ahead
     limiter (src/limiter.rs) before the arrangement trim. All out by
     default, and out is bit-identical. A strip with an insert that has
     memory (EQ, drive, compressor) takes its channel's summed voice
     cards once a sample; plain strips still run per card. Engaged, the
     master stage replaces the tanh soft limit, so its ceiling is the
     true peak
   - Mix-bus compressor (src/buscomp.rs): the SSL G-series bus comp,
     after the effect returns and before the tape, like a console bus
     compressor printing to cassette. Out by default and bit-identical
     when out; its sidechain listens while out so the meter reads and IN
     lands on a settled gain

8. **MIDI Handler**: Processes MIDI input.
   - Uses a custom enum for type-safe MIDI events

9. **User Interface**: Provides control over synth parameters.
   - Implemented using egui for immediate mode GUI
   - Communicates with audio thread via Arc<Mutex<>>

10. **Real-time Audio Processing:**
    - Use lock-free data structures and atomic types for parameter updates
    - Minimize allocations in the audio callback
    - Implement oversampling for alias reduction

## 4. Rust-Specific Design Considerations

1. **Thread Safety and Communication**:
   - Use `Arc<Mutex<>>` for shared state that requires complex operations
   - Implement a lock-free ring buffer for audio thread communication
   - Use atomics for simple shared state (e.g., global volume)

2. **Memory Management**:
   - Implement an object pool for voice allocation to avoid runtime allocations
   - Use `const` generics for fixed-size audio buffers

3. **Zero-Cost Abstractions**:
   - Define traits for Oscillator, Envelope, and Filter interfaces
   - Use static dispatch where possible for performance

4. **Type Safety**:
   - Implement newtype patterns for units like Frequency and Amplitude
   - Use a type-safe builder pattern for synth configuration

5. **Error Handling**:
   - Define a custom `SynthError` enum that encapsulates all possible errors
   - Use `Result` types consistently, avoiding panics in audio threads

6. **Concurrency Patterns**:
   - Use channels for non-real-time communication between threads
   - Explore lock-free data structures for real-time parameter updates

7. **Optimization**:
   - Utilize SIMD instructions for audio processing where applicable
   - Avoid allocations and blocking operations in the audio thread

8. **SIMD Optimization**:
   - Use Rust's SIMD intrinsics for parallel processing of audio samples

9. **Setter Idempotence (house rule)**:
   - Song automation re-asserts parameter values every block, so **every
     setter that allocates, rebuilds filters, re-randomizes, or otherwise
     resets state MUST early-return when the value is unchanged**. A
     setter that only stores a float may skip the guard.
   - Violations are subtle and audible: `Talker::set_clarity` once rebuilt
     its output filters per call and the cascade never rang up (the voice
     lost its top end); `Chorus::set_mode` rebuilt BBD voices per set
     event. Guarded examples: `Talker::set_clarity`, `Chorus::set_mode` /
     `set_rate` / `set_depth`, `Tape::set_drive` / `set_age`,
     `VoxBox::set_mode`, `BusComp::set_makeup` (every other `BusComp`
     setter stores a value or a time constant and needs no guard;
     `buscomp::tests::reasserting_settings_is_a_no_op` pins it), the strip
     inserts' `Eq3::set` (redesigns a band only on a change) and
     `Dist::set_mode` (reseeds its ADAA memory only on a change;
     `inserts::tests::reasserting_a_setting_is_a_no_op`).
   - The next structural parameter you add will hit this trap: write the
     guard first, and a test that re-asserting the same value leaves
     internal state untouched (see `chorus::tests::reasserting_the_same_mode_is_a_no_op`).

## 5. Key Rust Patterns and Features to Utilize

1. **Traits**: For defining common interfaces (e.g., `Oscillator`, `Effect`)
2. **Enums**: For representing different types (e.g., waveforms, MIDI events)
3. **Generics** and **PhantomData**: For type-safe configurations
4. **Atomic Types**: For lock-free sharing of simple values
5. **Channels**: For communication between non-audio threads
6. **Unsafe Code**: Carefully used for performance-critical sections, with clear safety documentation
7. **Macros**: For generating repetitive code (e.g., parameter setters)
8. **Type State Pattern**: For enforcing correct usage of the synthesizer API
9. **Const Generics**: For compile-time configuration of oversampling rates and buffer sizes

## 6. Analog-Specific Considerations

1. **Oscillator Drift**:
   - Implement subtle frequency drift to emulate temperature changes in analog circuits
   - Use noise generators to add slight instability to oscillator pitch

2. **Filter Behavior**:
   - Model non-linear behavior of filter cutoff and resonance
   - Implement filter saturation for overdrive effects
   - Ladder filter

3. **Envelope Non-Linearity**:
   - Model the non-linear charging and discharging of capacitors in analog ADSR circuits

4. **Component Tolerances**:
   - Simulate variations in component values to add subtle differences between voices

5. **Analog Warmth**:
   - Implement soft clipping and saturation throughout the signal path
   - Add subtle harmonic distortion to mimic analog circuitry

6. **Noise and Imperfections**:
   - Add low-level noise to various stages of the signal path
   - Implement "zipper noise" reduction for parameter changes

7. **Power Supply Simulation**:
   - Model subtle fluctuations in power supply to affect overall sound

8. **Anti-Aliasing**:
   - Implement oversampling and appropriate anti-aliasing filters

9. **Analog-Style Modulation**:
   - Implement cross-modulation between oscillators
   - Allow audio-rate modulation of filter parameters

## 7. Performance Optimizations

1. Implement vectorized operations using SIMD for core DSP functions
2. Optimize memory access patterns for cache-friendly processing
3. Implement multi-threading for parallel voice processing

## 8. Future Considerations

1. Explore async Rust for non-audio tasks if beneficial
2. Consider FFI for integrating with existing audio or MIDI libraries
3. Investigate using WebAssembly for a web-based version of the synthesizer
4. Implement a flexible audio graph using traits and generics
## 9. Feel: what the floor does (`patina feel`)

`patina feel <song.song | file.wav | file.mid>` plays a piece through a
modelled big sound system and reports what a body in the floor or soil
right under the subwoofer would feel, over time (src/feel.rs). It is
analysis only: it reads the same offline bounce `--render` makes, in
memory, so a render is bit-identical whether or not anyone feels it.

```
mix -> mono -> LR4 crossover -> sub's low-frequency limit       (sound system)
    -> pressure at 1 m on the ground (the --level calibration)  (level)
    -> soil | slab | timber floor                               (venue)
    -> vertical acceleration a(t) at the body, m/s²
    -> ISO 2631-1 Wk -> running r.m.s., MTVV, VDV               (human feel)
    -> hits, tempo, sections -> report, JSON
```

Every stage is a linear filter given as an analog prototype (second-order
sections), applied as f64 biquads by the bilinear transform at an 8 kHz
analysis rate. The exact analog response is what the JSON and
`examples/feel_venues.rs` print, and the tests hold the digital filters to
it (within 1% at 50 Hz for the whole chain, 0.2% for Wk).

### 9.1 Sound system

- **Mono**: (L + R) / 2. A song is first normalized the way `--render`
  normalizes a bounce (peak to −1 dBFS), since that is the file a sound
  engineer would get; `--no-normalize` keeps the engine's gain. A WAV
  plays at its own level.
- **Crossover**: 4th-order Linkwitz-Riley low-pass (two cascaded
  Butterworth 2nd-order sections, −6 dB at the crossover), 90 Hz for the
  PA rigs, 80 Hz for the house. The tops are ignored: what they put into
  the floor is not modelled. The crossover is also the anti-alias filter
  for the step down to 8 kHz (more than 130 dB down at 4 kHz).
- **The sub's own limit**: a vented or horn box rolls off as a 4th-order
  Butterworth high-pass (the B4 alignment), at 32 Hz for the PA rigs (an
  L-Acoustics KS28 is −10 dB at 25 Hz); the house sub is a sealed box,
  2nd order (Qtc 0.707) at 25 Hz. `--sub-hz`, `--sealed`, `--vented`.

### 9.2 Level: digital full scale to pressure and force

`--level` names the peak SPL at 1 m (half space, sub on the ground) that a
full-scale sub-band signal reaches, i.e. where the system's limiter sits:

| level | dB peak at 1 m | Pa | basis |
|---|---|---|---|
| festival | 143 | 283 | one KS28-class cabinet at its rated maximum (data sheet: 143 dB peak at 1 m, half space, pink noise of crest factor 4) |
| club | 135 | 112 | a club double-18 run near its limit (estimate) |
| home | 112 | 8.0 | a large home subwoofer near its limit (estimate) |

or any number (`--level 128`). Assumptions, all stated in every report:

- Full scale in the sub band is the limiter. The absolute scale is an
  estimate; it is the same in every run, so runs compare.
- A sub on the ground radiates as a monopole on a rigid half space,
  p(r) = ρ0 Q′(t − r/c) / (2π r). At full scale and 143 dB the cones'
  volume acceleration is Q′ = 2π (1 m) p / ρ0 = 1,480 m³/s², and the
  motors push 2.35 kg/m² × Q′ = 3.5 kN on the cones (an 18" woofer moves
  about 0.27 kg over 0.115 m²). The report prints both.
- What loads the ground is the sound pressure on it:
  q(r) = p1 × (1 m) / max(r, 0.5 m) around the sub's centre, held at its
  cabinet-face value inside the cabinet's half-depth. The cabinet passes
  the pressure on its top through to the ground under it. At festival
  level that is 1.33 kN on the soil within 1 m of the sub.
- The cone's own recoil is not a vertical load: a front-firing box
  recoils horizontally, as a shear force and an overturning moment at its
  base, and both are antisymmetric about the box's centre, so on the
  centreline under it they cancel in the vertical. They do shake the
  ground sideways, which this model does not report (that is ISO
  2631-1's Wd axis, not Wk).
- No limiter or power compression is modelled (the LR4 can overshoot full
  scale slightly on a transient), and the system is linear throughout:
  soil strains here are around 10⁻⁶, far below where soil softens.

### 9.3 Venues

Each venue reduces to a static compliance C at the body (metres per
pascal of the sub's 1 m pressure), one resonance f0 and its Q:

a / p1 = C ω0² s² / (s² + (ω0/Q) s + ω0²)

so acceleration rises 12 dB/oct below f0 (stiffness-controlled), peaks by
about Q at f0, and is flat above (mass-controlled). A free ring falls
20 dB in ln 10 / σ, σ = ω0/(2Q) (the slow root when overdamped).

**field** (`--venue field`, the default): a festival field, the sub on
soft soil, you buried `--depth` (0.3 m) under its centre, `--distance` to
the side (0-2 m).
- Soil: soft layer Vs 150 m/s, ρ 1,800 kg/m³, ν 0.35, material Q 10
  (damping ratio 5%), H = 1.5 m deep, over firmer ground Vs 450 m/s,
  ρ 2,000 kg/m³ (typical topsoil-over-subsoil values).
- Static: Boussinesq's displacement under a surface point load,
  u_z = P [2(1−ν) + z²/R²] / (4π G R), summed over the near-field load
  within 1 m (about a quarter shear wavelength at 40 Hz; beyond it the
  loads arrive out of phase and the quasi-static sum stops), then divided
  by Gazetas' (1991) stratum factor 1 + 1.28 a/H for the firmer ground
  below. C = 1.30 × 10⁻⁸ m/Pa.
- Resonance: the soft layer's vertical (compression-extension)
  resonance, f0 = V_La / 4H, with Lysmer's analog speed
  V_La = 3.4 Vs / (π(1 − ν)) = 250 m/s (Gazetas 1991): f0 = 41.6 Hz, right
  under a kick's fundamental. The layer's shear resonance Vs/4H = 25 Hz is
  the one that shakes the ground sideways, so it does not appear here.
- Q: the peak amplification of a damped layer on an elastic base,
  1 / (α + πζ/2) (Kramer 1996, sec. 7.2), α the impedance ratio
  ρ1Vs1/ρ2Vs2 = 0.30: Q = 2.6.
- The material loss along the 0.3 m path, exp(−π f z / (Q Vs)), is under
  6% below 90 Hz and is left out (the damping is in Q).
- Check against measurement: at 50 Hz the model moves the soil
  6.4 µm/s per pascal; outdoor acoustic-to-seismic coupling ratios of
  soils measure about 1 to 13 µm/s per pascal (the Sabatier, Bass and
  Bolen line of work, JASA 80, 646, 1986, and later site surveys).

**club** (`--venue club`): a concrete slab on grade, you lying on it
`--distance` (1.0 m, up to 3 m) from the sub.
- Slab 150 mm, E 30 GPa, ν 0.2, 2,400 kg/m³, on a compacted subgrade of
  modulus k = 100 MN/m³ (ρ 1,900, Vs 200 m/s, ν 0.3).
- Static: Westergaard's plate on a dense-liquid (Winkler) foundation, a
  point load deflecting w(ρ) = −(ℓ²/2πD) kei(ρ/ℓ), radius of relative
  stiffness ℓ = (D/k)^¼ = 0.55 m, summed over the sub's direct field out
  to 4 m. kei is the Kelvin function (series of Abramowitz & Stegun
  9.9.12, its asymptote beyond 10, checked against the integral for
  K0). C = 1.05 × 10⁻⁸ m/Pa.
- Resonance: the slab's mass on the subgrade spring, f0 = √(k/m)/2π =
  84 Hz, damped by radiation into the ground with Lysmer's dashpot ρV_La
  per area: damping ratio 1.55, Q = 0.32, overdamped. The slab does not
  ring (12 ms to −20 dB); it is the dead floor.

**house** (`--venue house`): a suspended timber floor, you in it at
midspan under the sub, `--distance` along the span from midspan.
- 47 × 220 mm C24 joists (E 11 GPa) at 400 mm, 3.0 m span, floor width
  4 m, 45 kg/m² with boards, ceiling below and some furnishing.
- Resonance: a one-way simply supported floor, f1 = (π / 2L²) √(EI/m) =
  27.9 Hz. Damping 3% (Q 17): Eurocode 5 designs residential timber
  floors on 1%, measured furnished floors run 1.5-4%. It rings 438 ms to
  −20 dB: the boomy floor.
- Static: the first mode, w = φ(x) ∫ q φ dA / (M1 ω1²), with
  ∫ φ dA = (2/π) L B and M1 = m L B / 2, driven by the sub's direct field
  weighted by the mode shape (0.98 p1 on average over the 3 × 4 m bay).
  C = 9.1 × 10⁻⁷ m/Pa.
  Room gain below the room's first mode would raise the pressure further
  and is not modelled.

Transfer functions (`cargo run --release --example feel_venues`), the
venue alone in (m/s²)/Pa, and the whole chain at each venue's own rig and
level, a full-scale sine in m/s² peak:

| Hz | field /Pa | field FS | club /Pa | club FS | house /Pa | house FS |
|---|---|---|---|---|---|---|
| 20 | 2.6e-4 | 0.011 | 1.4e-4 | 0.0023 | 2.9e-2 | 0.13 |
| 25 | 4.7e-4 | 0.046 | 2.0e-4 | 0.0078 | 1.1e-1 | 0.62 |
| 31.5 | 9.9e-4 | 0.19 | 2.8e-4 | 0.022 | 1.2e-1 | 0.82 |
| 40 | 2.2e-3 | 0.56 | 4.0e-4 | 0.040 | 5.4e-2 | 0.38 |
| 50 | 2.0e-3 | 0.51 | 5.3e-4 | 0.054 | 4.0e-2 | 0.27 |
| 63 | 1.4e-3 | 0.33 | 6.9e-4 | 0.063 | 3.5e-2 | 0.20 |
| 80 | 1.2e-3 | 0.20 | 8.9e-4 | 0.062 | 3.2e-2 | 0.13 |
| 100 | 1.1e-3 | 0.12 | 1.1e-3 | 0.050 | 3.0e-2 | 0.070 |

The field boosts its kick band around its 42 Hz layer; the slab barely
moves at any frequency (a full-scale 50 Hz sine at 1 m stays under the
perception threshold once weighted); per pascal the timber floor moves 20
to 550 times more than either (most near its 28 Hz resonance) and rings
nine times longer than the field.

### 9.4 Human feel

- **Wk** (ISO 2631-1:1997, vertical whole-body), built from the
  standard's Annex A: band limiting (Butterworth high-pass 0.4 Hz,
  low-pass 100 Hz), acceleration-velocity transition (f3 = f4 = 12.5 Hz,
  Q4 = 0.63) and upward step (f5 = 2.37 Hz, Q5 = 0.91, f6 = 3.35 Hz,
  Q6 = 0.91). Against the standard's Table 3 at 1, 2, 4, 6.3, 8, 12.5,
  16, 31.5, 50, 63, 80 and 100 Hz it is within 0.3% everywhere
  (`wk_matches_the_standards_table`).
- **Running r.m.s.** a_w(t0) over the τ seconds up to t0 (linear
  integration, ISO 2631-1 6.3.1), τ = 1 s and a fast 125 ms; **MTVV** is
  the largest 1 s value. **VDV** (∫a_w⁴ dt)^¼ and the crest factor are
  reported because kicks are shocks (the standard's basic r.m.s. method
  underestimates above a crest factor of 9).
- **Perception**: half of alert, fit people detect a Wk-weighted vertical
  vibration of 0.015 m/s² peak (Annex C.3; interquartile range about 0.01
  to 0.02). It is compared with the Wk peak.
- **Comfort**: Annex C.2.3, on the r.m.s.: under 0.315 m/s² not
  uncomfortable; 0.315-0.63 a little; 0.5-1 fairly; 0.8-1.6
  uncomfortable; 1.25-2.5 very; over 2 extremely uncomfortable. The bands
  overlap on purpose; a value in two is reported as between them.

### 9.5 What the report reads

- **Hits** (a kick or a bass note's onset): on the Wk signal's 25 ms
  r.m.s. at 5 ms hops, a rise of 6 dB over the quietest point 15-60 ms
  before (after a hit, measured from the dip after its top), in the whole
  signal or in one of four bands (below 40, 40-63, 63-100, above 100 Hz,
  Linkwitz-Riley slopes) that carries some share of it (within 20 dB). A
  held bass sits in one or two bands, so a kick on top of it, whose
  falling pitch sweeps through the others, still shows there when the
  whole signal barely rises. A rise's crest is where it stops (1.5 dB
  under the running top), and it must reach half the perception
  threshold. Rises inside a hit's first 80 ms, or before its top over its
  first 90 ms (a kick's tail beating against a held bass swells again
  within 90 ms when the two are 11 Hz or more apart), belong to it; a
  ripple in a ring just before a real hit is replaced by it. A hit brings
  new energy: in a signal that rose, the 50 ms and the 160 ms after its
  onset carry twice the energy of the 160 ms before (in a band, while the
  whole signal also gains a quarter); two notes beating against a kick's
  tail do not. The hit's time is the earliest, among the signals rising
  within 20 ms of it, of where the r.m.s. last comes up within 20 dB of
  its top (or 3 dB over the dip it rose from): the steep front of the
  attack, within a millisecond or two of the note on a kick alone (the
  modelled 909's analog drift moves it about ±1.5 ms; over a held sine
  bass about as strong once weighted, within 15 ms). Per hit: raw peak (m/s², g) in its first 100 ms, Wk peak,
  dominant frequency of its first 160 ms (Hann-windowed DFT, 15-200 Hz,
  parabolic peak), ring-down from the 40 ms envelope's peak to −20 dB
  (extrapolated from the decay rate when the next hit comes first), what
  is felt over the last 40% of the gap to the next hit (Wk r.m.s., peak,
  frequency), and for songs the tracks whose notes start on it, among
  those that can reach the sub: kick drums, samples, and notes at E3
  (165 Hz) or below. A section names the tracks on 90% of its hits, or
  failing that on half of them.
- **Tempo**: the most common hit spacing (the spacing with the most
  others within 4% of it, refined to their mean), reported when at least
  half the spacings fit it.
- **Sections**: an optimal piecewise-constant fit of the 2 s Wk level
  (four values a second, everything under a quarter of the threshold
  counted as still; at least 4 s each, or 2 s for a still break; a new
  section costs what a 3 dB step between two 8 s halves would),
  neighbours closer than 3 dB merged, each boundary moved to the sharpest
  step within 2 s (and an upward step onto its first hit). A section is
  still when no hit in it reaches the threshold and at most 2% of it
  does; under a tenth of the threshold the report says nothing reaches
  the ground, above that that the ground moves below the threshold (or,
  when a moment of it crosses, that only the tail of what came before
  does).

### 9.6 Not modelled

The sound on the body through the air (chest and trouser-leg buzz, which
at a festival is most of what is felt), horizontal ground motion, room
modes and room gain, the crowd (jumping crowds shake floors at 2-3 Hz
far harder than any PA), limiters and power compression, the body's own
mass loading the soil (it moves with the ground here), and anything from
the tops.

### 9.7 Sources

- ISO 2631-1:1997, Mechanical vibration and shock: evaluation of human
  exposure to whole-body vibration, Annex A (weightings), Table 3, Annex C.
- G. Gazetas, "Foundation vibrations", ch. 15 of the Foundation
  Engineering Handbook (2nd ed., 1991): Lysmer's analog, stratum factors.
- S. L. Kramer, Geotechnical Earthquake Engineering (1996), sec. 7.2:
  amplification of a damped layer on an elastic base.
- J. Boussinesq (1885); K. L. Johnson, Contact Mechanics (1985), 3.2.
- H. M. Westergaard, "Stresses in concrete pavements computed by
  theoretical analysis", Public Roads 7 (1926).
- M. Abramowitz and I. Stegun, Handbook of Mathematical Functions, 9.9-9.10.
- J. M. Sabatier, H. E. Bass, L. N. Bolen and K. Attenborough, JASA 80,
  646 (1986), and later outdoor acoustic-to-seismic coupling surveys.
- EN 1995-1-1 (Eurocode 5) 7.3: timber floor vibration, 1% damping.
- L-Acoustics KS28 data sheet: 143 dB peak at 1 m.
