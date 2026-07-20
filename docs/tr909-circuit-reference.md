# Roland TR-909 — Analog-Voice Circuit Reference

Extracted from the Roland TR-909 documentation set: the *TR-909 Service Notes,
First Edition, Jun 15 1984* (studied primarily via the clearer of the two
service-manual scans) and the TR-909 Owner's Manual. All component designators,
values, and pot codes below are read directly from the service notes' Voicing
Board circuit diagram (board **VG 73133090 / PCB 2291084903**, sheet "3/3"),
extracted at 400 DPI and cropped per-voice. Where a value was not printed in the
manual it is labelled **DERIVED** (computed from the surrounding RC network or
taken from the widely-measured figure) or **CHOICE** (a deliberate modern
extension in our implementation).

This document is the citation source for `src/drums.rs`: constants there are
labelled SCHEMATIC / DERIVED / CHOICE and reference the sections below (e.g. the
snare shell ratio 1.47 = C69 0.01 µF / C71 0.0068 µF is section 3). Sequencer,
display, CPU, and cassette sections are deliberately out of scope. Patina's
rhythm section implements bass drum, snare, rim shot, hand clap, and a
no-samples hi-hat; toms and the ROM cymbals are documented here for completeness
but omitted from the code by design (see the per-section notes).

Global note printed on the schematic: *unless otherwise noted, all NPN
transistors are 2SC2603-F, all PNP transistors are 2SA1115-F, all diodes are
1SS133.*

---

## 1. Global / common circuitry

**Power rails.** Three analog rails on the Voicing Board: **+15 V, −15 V, +5 V**
(digital), with a split analog/digital ground (AGND). Op-amps run ±15 V. Power
Supply Board: 7815 (IC701, +15), 7915 (IC702, −15), 7805 (IC703, +5), bridge
rectifiers 1B4B41 (D702/D703), 2200 µF reservoirs. Reset generator Q701/Q702/D701
(zener RD6.8JB2 → RD5.6JB2 after S/N 393000 for brown-out reliability).

**Active-device inventory (the parts that colour the sound).**
- **2SC2603-F** — default NPN (switches, buffers, VCA transistors).
- **2SA1115-F** — default PNP.
- **2SD1469** — snare VCO core transistors (Q41, Q48).
- **2SA798** — matched **dual PNP**, used as the anti-log/exponential converter
  pair in the cymbal/hi-hat accent-VCA current control (Q68 ride, Q70 crash,
  Q84 hi-hat). This exponential converter gives accent its musical (exponential)
  loudness law.
- **2SC2878** (Q82, Q83) — master-volume output buffer transistors.
- **Op-amps:** **M5218L** (dual, the workhorse — IC11–15, 17–21, 23–28, 34–35,
  37–43, 45–52, 64–67); **AN6912** (IC29, quad comparator, = hand-clap
  multi-pulse generator); **BA662A** (IC30, Roland OTA / "vari-conductance amp",
  = hand-clap VCA).
- **CMOS:** 4069UBP / 4049 unbuffered inverters (IC16, IC22, IC36, IC44) as the
  tom & snare VCO integrator/comparator gain stages; 4006 18-bit shift registers
  (IC32, IC33) = noise source; 4070 quad XOR (IC31) = noise clock; 4013 dual D
  flip-flop, 4040 12-bit counter, 4520 dual counter, 4174 hex D latch =
  cymbal/hi-hat PCM address & sample-latch logic.

**Trigger (TRIG) and Accent (ACCENT).** From the CPU (µPD7811G, IC604):
- **TRIG:** latched into IC1/IC10, appears as a **+5 V positive-going pulse
  ~2 ms wide** on the selected voice line. It both fires the voice and (via
  steering diodes + RC nets) shapes each voice's pitch/tone/decay envelopes.
- **ACCENT:** each voice has its own accent latch (IC2–IC9) selected by decoders
  IC612/613; the digital code is converted to analog by resistor array **RM0621
  (1 kΩ×8, R-2R ladder, R=5K)** and **held (clamped) until the next note**, then
  scales that voice's VCA depth. Global **TOTAL ACCENT** pot VR1 100 kΩ(B) sums
  into the accent bus.
- Documented common accent/trigger envelope shapes (measured on the BD accent
  shaper, IC36 region): **F1** peak 30 %, atk ≈ 2 ms, total ≈ 20 ms; **F2** peak
  15 %, atk ≈ 4.5 ms, total ≈ 25 ms; **F3** peak 30 %, atk ≈ 1 ms, total ≈ 5 ms.

**Noise generator (shared by snare, toms, hand-clap).** Quasi-random **digital**
noise — flat and bright, NOT a transistor junction (a common misconception; that
is the ARP-style source Patina reuses for the drum board as a CHOICE):
- **IC32 + IC33 = two 4006 18-bit static shift registers cascaded = 32 usable
  stages.** The long chain makes the repeat period inaudible.
- **IC31 = 4070 quad XOR** feeds the shift-register input (gates a/b/c/d) to make
  the pseudo-random sequence, clocked at a high rate for bright noise.
- Support: R187 47K, R186 100K, R185 33K, R184 10K, R182/R183 10K, C46 4.7 µF/50,
  C47 100 pF, D48/D49 (D48 injects the power-up start trigger into IC32 pin 1),
  **C48 0.1 µF ceramic**. +15 V.
- Fans out to snare snappy/tone, the Tom-Noise buffer, and the clap noise input.

**Tom-noise conditioning (feeds all three toms).** IC27a/IC27b buffer/filter:
R190 22K, C161/C51/C52 0.0047 µF, R191 10K, R192 220K, C53 0.047 µF, R195 1K,
Q34/Q33, R196 22K, R197 10K, R198 100K, R199 47, R200 100K, R193 22K, **R194
47K, C54 0.0022 µF**. Steering diodes D3/D2/D1/D45/D44/D26 route noise+trigger to
Low/Mid/Hi Tom. **After S/N 426700:** C54 0.0022 → **0.0047 µF**, R194 47K →
**100K** ("emphasizes attack of Tom-Toms").

**Output / mix.** Each voice → its LEVEL pot → individual Multi-Jack output (with
series resistor), and → the stereo master mix. Multi-out order (CN-5 pins
33–42): Bass Drum, Snare, Low Tom, Mid Tom, Hi Tom, Rim Shot, Hand Clap,
Hi-Hat, Crash, Ride. Series output resistors: BD 12K, SD 12K, Low Tom 8.2K, Mid
Tom 12K, Hi Tom 15K, Rim 12K, Clap 12K, Hi-Hat 15K, Crash 8.2K, Ride 15K
(R501–R529), each with a 0.01 µF shunt (C500–C512) added after S/N 415300 to cut
output noise. **Master Volume:** IC66a/IC66b summers, output buffers **Q82/Q83
(2SC2878)**, C153/C154 1 µF/50, C143 47 µF/35, C140/141/142 10 µF/16. Master Out
L, R/MONO, 6 Vp-p, 1 kΩ. Nominal internal level = 2 Vp-p at Multi Out with level
pots centred.

---

## 2. Bass drum (IC11–IC13, Q1–Q12)

A **self-oscillating bridged-T / twin-T resonator** (op-amps IC12a/b, IC13a/b,
labelled "VCO") *rung* by a trigger pulse, with a **separate hard click/attack
transient**, a downward **pitch sweep**, diode soft-clip, and a discrete VCA.

```
TRIG ─► pulse shaper (Q1,Q2,Q7 / R1 22K,R2 10K,R13 47K,R4/R3 47K)
                 │
                 ├─► PITCH ENV (Q5/Q3/Q4, R33 1M,R32 47K,R20 6.8K,
                 │        R17/R18 470K, C7 0.033µ) ── pulls resonator freq down
                 │
        ┌────────┴─ CV Gen (IC12a/b) ─► T-resonator (IC13a/b)
        │            C1 0.068µ, R26 330K, R27 1.5M, R29/R30 100K
        │            TUNE VR2 100K(A) + R23 47K, R57 2.7K, C9 0.33µ(T)
        │                                            │ decaying quasi-sine
   CLICK/ATTACK (Q11, R51 2.2K, R48 22K, D10/D11/D12, C4, "PULSE")   │
        │                                            ▼
        └───────────────► mix ─► VCA (Q6/Q12, gain = decay-env × ACCENT)
                            │
                 AMP IC11a ─► LEVEL VR4 100K(B) ─► BD out
```

- **Resonator caps/Rs:** C1 **0.068 µF**, R26 330K, R27 **1.5 M**, R29/R30 100K;
  IC13a/b feedback R31, R25 100K, R24 22K. Tune cap C9 **0.33 µF (T)**, plus
  C5 0.0068, C4 0.0033, C2 100 pF, C3 0.0047 µF.
- **TUNE:** VR2 **100 kΩ(A)** varies the resonator charge current →
  fundamental ≈ **20–90 Hz** (no explicit Hz printed; DERIVED). **After
  S/N 381500, C9 0.22 → 0.33 µF "for expanding the TUNE range."**
- **ATTACK:** VR3 **500 Ω(B)** — amount of the fast click/beater transient
  (click path Q11 + R51/R48 + D10–D12).
- **DECAY:** VR5 **1 MΩ(A)** with R58/R35/R36 47K, D8/D9, C8 0.33 µF(T) — the
  resonator ring-down time.
- **LEVEL:** VR4 **100 kΩ(B)**. Output amp IC11a; R53 2.2K, R54 470K.
- **VCA:** discrete Q6/Q12 (matched, "s"), control = decay envelope × accent
  (Q4 gated by TRIG). Waveshaping is the natural soft-clip of the VCA transistor
  plus the clamp diodes (D8/D9/D10/D11/D12), rounding the tone into the 909 kick.
- Waveform (sheet 9): 200 mV/div, 5 ms/div — pitch-swept decaying quasi-sine;
  accent raises initial amplitude and click.

---

## 3. Snare drum (IC35–IC40, Q40–Q51)

Two sub-voices: **"Drum"** (two tuned oscillators = the shell) + **"Snappy"**
(split-filtered noise = the wires). Both tunable with independent envelopes.

```
DRUM:
  CV Gen IC35a/b ─► VCO-1 (IC37a/b, 2SD1469 Q41, C69 0.01µ, R269 100K) ┐ triangle
                 ─► VCO-2 (IC38a/b, C71 0.0068µ, R273 100K) ───────────┤ triangle (higher)
   IC36 (4069UBP) = inverting buffer in both loops                      │
   VCA Q50 (drum amount, gain = ENV3 × ACCENT via Q4/TRIG) ─────────────┤
SNAPPY:
  NOISE ─► HPF IC39a (R285 3.3K, R286 10K, C85 0.015, 0.0022µ×2, C84 220pF,
            C81 0.0033µ) ─► articulate highs                            │
  NOISE ─► LPF IC39b (C67 0.47µ/50, C80 0.022) ─► noise body            │
  VCA Q47/Q48; SNAPPY pot VR9 sets HF amount; gain = ENV5               │
   Sum ─► AMP IC40b ─► LEVEL VR8 50K(B) ─► Snare out
```

- **VCO-1 (lower):** comparator IC37a + integrator IC37b, core **2SD1469 (Q41)**,
  timing cap **C69 = 0.01 µF**, R269 = 100K.
- **VCO-2 (higher):** IC38a/b, timing cap **C71 = 0.0068 µF**, R273 = 100K.
- **Frequency ratio = C69/C71 = 0.01/0.0068 = 1.47** (equal 100K integrator
  resistors, so f ∝ 1/C). Fundamentals ≈ **180 Hz and ~265 Hz**. On TRIG the
  VCOs reset to a common start (Q44 discharges C69; IC37a forced low) so the
  attack phase-locks; the CV gen adds a **~20 ms downward pitch bend** at onset
  (IC36 pulse +5 → −5 V, **2 ms** wide, decaying over ~20 ms).
- **VCA (drum):** Q50 (matched), gain = ENV3 (ACCENT gated by Q4/TRIG). Q51 in
  the output stage.
- **Snappy filters:** IC39a = high-pass ("snap"), IC39b = low-pass (noise body);
  outputs recombine. ENV5 (gated by Q41) sets the snap duration.
- **Knobs:** TUNE VR6 **10 kΩ(B)** (both VCOs together); TONE VR7 **500 kΩ(B)**
  (drum-tone/brightness tilt via Q42/D60); SNAPPY VR9 **10 kΩ(B)** (HF noise
  amount); LEVEL VR8 **50 kΩ(B)**.
- Waveform (sheet 9): 0.5 V/div, 2 ms/div — pitched decaying tone + noise.

---

## 4. Toms — Low / Mid / Hi (identical topology; omitted from Patina by design)

Low Tom IC14–IC19, Q13–Q22 · Mid Tom IC20–IC25 · Hi Tom IC42–IC47, Q53–Q62.
Each tom is a **cluster of three triangle-core VCOs** (VCO-1/2/3) tuned slightly
apart and summed for the "body," plus added tom-noise for the skin attack, a
downward pitch envelope, and a VCA + decay envelope.

```
CV Gen (op-amp) + TUNE ─┬─► VCO-1 (cap A) ─┐
                        ├─► VCO-2 (cap B) ─┤ sum (triangles)
                        └─► VCO-3 (cap C) ─┘
   PITCH ENV1 (downward) modulates CV ; NOISE mixed for attack
   ─► VCA (Q19/20/21) gain = ENV × ACCENT ─► AMP ─► LEVEL ─► DECAY ─► out
```

| Tom | VCO-1 cap | VCO-2 cap | VCO-3 cap | Pitch-env cap | Decay cap | Level | Tune | Decay |
|-----|-----------|-----------|-----------|---------------|-----------|-------|------|-------|
| **Low** | C18 **0.022 µF** | C19 **0.033 µF** | C20 **0.012 µF** | C22 0.22 µF(T) | C25 0.056 µF | VR12 50K(B) | VR10 10K(B) | VR11 500K(B) |
| **Mid** | C32 **0.018 µF** | C33 **0.027 µF** | C34 **0.01 µF** | C36 0.15 µF(T) | C35 0.056 µF | VR15 50K(B) | VR13 10K(B) | VR14 500K(B) |
| **Hi**  | C97 **0.015 µF** | C102 **0.022 µF** | C103 **0.0082 µF** | C99 0.15 µF(T) | C106 0.047 µF | VR18 50K(B) | VR16 10K(B) | VR17 500K(B) |

- Cap sizes shrink Low → Mid → Hi, so pitch rises. Per-tom the VCO-1/2/3 caps
  are ~1 : 1.5 : 0.55, the fixed internal detune that makes each tom a small
  chord rather than a pure tone.
- Common per-VCO resistors mostly **100K**; VCO output gain Rs **1 M**; series
  47K; 4069UBP supplies the loop gain (IC16 low, IC22 mid, IC44 hi).
- **TUNE (10 kΩ(B))** shifts all three VCOs together; **DECAY (500 kΩ(B))** sets
  the ENV ring-down; **LEVEL (50 kΩ(B))** into the output amp (IC18a/IC23a/IC45a).
- Waveform (sheet 9): 0.5 V/div, 5 ms/div — pitch-swept, noise-tinged decay.

---

## 5. Rim shot (IC48–IC50, Q63–Q68)

A **triple bandpass "ping":** the trigger rings three tuned multiple-feedback
bandpass filters whose sum makes the metallic rim tone, then diode clip + output
high-pass. All attack, no body.

```
TRIG ─► pulse (IC48a: Q64/Q63, R400 47K, R402 1M, R398 10K, C111 0.018, ENV C119 0.047µ)
          ├─► F1  IC48b : C112/C113 0.01,  Rfb 470K (R407), Rin 2.2K (R394) ─┐
          ├─► F2  IC49a : C115/C116 0.027, Rfb 330K (R414), Rin 2.2K (R411) ─┤ mix
          └─► F3  IC49b : C117/C118 0.0047, Rfb 470K (R416), Rin 2.2K (R404) ┘
          ─► CLIPPER (D91/D92) ─► VCA/LEVEL (IC50a, Q65, C120 220pF, VR19 100K(B))
          ─► HIGH-PASS (IC50b, C121/C122 0.01µ, R422 22K, R421/R419 4.7K)
          ─► Rim out (also drives TRIG OUT jack, +14 V 20 ms pulse)
```

- Bandpass centres, f0 = 1/(2π·C·√(Rin·Rfb)) (DERIVED): **F2 ≈ 219 Hz**,
  **F1 ≈ 496 Hz**, **F3 ≈ 1054 Hz**.
- **Clip:** diodes D91/D92 (asymmetric) add the sharp odd-harmonic attack.
- **Envelope:** very short (ENV ~ C119 0.047 µF); whole event a few ms.
- **LEVEL:** VR19 100 kΩ(B). **After S/N 415300, R417 12K → 3.3K** ("more
  realistic sound"; sheet 9 shows the R417 3.3K vs 12K decay comparison).
- Rim Shot also sources the rear-panel TRIG OUT (+14 V, 20 ms).

---

## 6. Hand clap (IC26, IC28, IC29 AN6912, IC30 BA662A, Q35–Q40)

Noise through a bandpass filter, gated by a **multi-pulse burst envelope** (~3
fast pulses) plus a longer **reverb tail**, gain-controlled by a **BA662 OTA**.

```
NOISE ─► buffers (Q35/Q36, R215 22K, R216 10K, R210 22K, R211 10K, IC26a, R496 100K)
      ─► BANDPASS (IC26b: C42/C43 0.0047, R207 150K, R208 47K, R209 10K;
                    C55/C57 0.47µ/50, R204 1K, R205 3.3K)
      ─► VCA = BA662A (IC30): C56 0.022, R202 39K, R237 82K, C58 0.001, VR20 50K(B)
                    ▲
  MULTI-PULSE ENV (IC29 AN6912 relaxation osc: C62 0.027µ, C64 0.001, R227 2.7K,
     R228 5.6K, R226 68K; D54/D55; Q40 out) ─► ~3 pulses, ~8–10 ms spacing
  REVERB TAIL: C65 0.47µ/50 + R225 1M (longer decay)
      ─► AMP IC28a ─► Hand Clap out (Adjustment: Level centre → 2 Vp-p)
```

- **Multi-pulse generator:** AN6912 (IC29) as a fast RC relaxation network emits
  the retrigger burst (~3 pulses, ~8–10 ms apart); C65 0.47 µF/50 + R225 1 M is
  the longer reverb-tail decay. The two combine into the clap "shhht."
- **VCA:** BA662A (IC30) — a genuine Roland OTA (rare among 909 voices, most of
  which use discrete/transistor VCAs).
- **Bandpass:** IC26b around C42/C43 0.0047 µF + R207 150K; the perceived clap
  band sits ~1 kHz.
- **LEVEL:** VR20 50 kΩ(B). Q37, R234 470K, R203 47, C59/C61 0.1 µF at output.
- Waveform (sheet 9): 500 mV/div, 10 ms/div — burst train + decaying noise tail.

---

## 7. Hi-hat (PCM sample + analog post-processing)

6-bit PCM in mask ROM; everything after the DAC is analog and is the character.
Patina replaces the PCM with a six-oscillator metal bank (a CHOICE) but keeps the
analog post-processing modelled below.

**Digital front end.** ROM **IC69 = HN61256P (mask "C43")** (256 kbit); address
counters **IC70 (4520) + IC71 (4040)**; on TRIG the counters reset to 0, gate
IC72a swings to "run," a ~60 kHz osc (IC72c/d) ÷2 by IC73 clocks them. **CLOSED
vs OPEN** loads a different start address via diode-OR D196–D199 (Closed = tail
only → short; Open = full sample): OPEN `000…`, COMMON `110…`, CLOSED `111…`.
Sample latched by **IC68 (4174)**, converted by ladder DAC **RA9 (RM0621, R=5K)**.

```
DAC (RA9) ─► VCA Q85 (R483 47K, R480 5.6K) ─► IC67b
   ▲ gain = decay-env × accent (anti-log Q84 = 2SA798, R488 2.2K, C157/C158 10µ/16)
   ─► LPF (fixed multi-pole): Q80/Q81, R475/R477/R479 5.6K, C148 1200pF,
        C150 2700pF, C151 390pF, R476/R478 10K
   ─► LEVEL VR22 50K(B), C134 10µ/16, R444 22K, IC65b ─► Hi-Hat out
```

- **Decay (discrete Q72/Q73/Q74):** CLOSED path R451 **10K** + VR21 **100 kΩ(B)**
  into cap **C135 1 µF/50** (fast, ~1/10 R); OPEN path R452 **100K** + VR23
  **1 MΩ(A)** into the same cap (slow). A high on Q72 base (CLOSED) lets Q73
  charge C135 through R451; low = OPEN through R452/VR23.
- The fixed LPF shapes tone; the sample was amplitude-compressed for S/N, so the
  DAC envelope is *restored* by this decay VCA.
- **LEVEL:** VR22 50 kΩ(B). Front panel: **CH DECAY (VR21), OH DECAY (VR23)**.
- Waveforms (sheet 9): Closed 500 mV/div, 20 ms/div; Open 500 mV/div, 0.1 s/div.
- **After S/N 415300, C134 10 µF → 0.01 µF** (roll off lows).

---

## 8. Crash & ride cymbals (PCM + analog post-processing; omitted from Patina by design)

Same architecture as hi-hat, one PCM voice each, each with a **TUNE** control
(which changes the sample playback clock rate).

**Digital.** Crash ROM **IC62 = HN61256P "C42"**; Ride ROM **IC54 = HN61256P
"C44"** (256 kbit each). Counters IC55 (4040), IC58 (4520); latches **IC63 (4174)
crash, IC53 (4174) ride**; DACs **RA10 (crash), RA11/RA12 (ride)**. **TUNE**
(Crash VR25 10 kΩ(B), Ride VR27 10 kΩ(B)) varies the address-clock oscillator
(IC52b, C126 1000 pF, R432 270K, C130 470 pF, R441 6.8K) → playback pitch. The
ROM-address stream is also fed through the DAC (RA11, anti-log IC52b + Q70) to
**reconstruct the decay envelope** (samples were amplitude-compressed).

```
DAC ─► VCA Q71 (crash) / Q69 (ride) ─► IC51b / IC51a
   ▲ gain = reconstructed decay-env × accent (anti-log Q70 crash / Q68 ride = 2SA798)
   ─► LPF: Q75/Q76 (crash) / Q77/Q78 (ride); R456/457/459/461 5.6K;
        C136 1200pF, C137 1000pF, C138 2700pF, C139 390pF
        (ride: R462/463/465/467 5.6K; C144 1200pF, C145 1000pF, C146 2700pF, C147 390pF)
   ─► LEVEL crash VR24 50K(B) / ride VR26 50K(B); C132/C133 10µ/16, R445/R446 22K
   ─► IC65a (crash) / IC64a (ride) ─► out
```

- Accent anti-log converters: **Q70 (crash), Q68 (ride) = 2SA798 dual PNP**;
  R426 4.7K, R427 2.2K, R429 100, R434 100, R435 4.7K, R436/R437 2.2K, R439 470K.
- Front panel: CRASH TUNE, RIDE TUNE, CRASH LEVEL, RIDE LEVEL. No separate
  decay — decay is baked into the sample + the auto-reconstructed envelope.
- Waveforms (sheet 9): Crash & Ride both 500 mV/div, 0.1 s/div — long dense decay.

---

## 9. Front-panel pot / knob summary

| Voice | Control | Designator | Value / taper | Electrical effect |
|-------|---------|-----------|---------------|-------------------|
| Global | Total Accent | VR1 | 100 kΩ(B) | DC accent voltage on the shared bus (scales all VCAs) |
| Global | Tempo | VR601 | 50 kΩ(B) | CPU tempo ADC |
| Global | Volume | — | 50 kΩ(B) | master amp gain |
| **Bass Drum** | Tune | VR2 | 100 kΩ(A) | resonator charge current → 20–~90 Hz |
| | Attack | VR3 | 500 Ω(B) | click/beater transient amount |
| | Decay | VR5 | 1 MΩ(A) | resonator ring-down time |
| | Level | VR4 | 100 kΩ(B) | output level |
| **Snare** | Tune | VR6 | 10 kΩ(B) | both VCO frequencies together |
| | Tone | VR7 | 500 kΩ(B) | drum-vs-noise / brightness tilt |
| | Snappy | VR9 | 10 kΩ(B) | HF noise (wire) amount |
| | Level | VR8 | 50 kΩ(B) | output level |
| **Low Tom** | Tune / Decay / Level | VR10 / VR11 / VR12 | 10K(B) / 500K(B) / 50K(B) | 3-VCO pitch / env decay / level |
| **Mid Tom** | Tune / Decay / Level | VR13 / VR14 / VR15 | 10K(B) / 500K(B) / 50K(B) | same |
| **Hi Tom** | Tune / Decay / Level | VR16 / VR17 / VR18 | 10K(B) / 500K(B) / 50K(B) | same |
| **Rim Shot** | Level | VR19 | 100 kΩ(B) | output level |
| **Hand Clap** | Level | VR20 | 50 kΩ(B) | BA662 VCA output level |
| **Hi-Hat** | CH Decay / OH Decay / Level | VR21 / VR23 / VR22 | 100K(B) / 1M(A) / 50K(B) | closed & open decay-cap charge times / level |
| **Crash** | Tune / Level | VR25 / VR24 | 10K(B) / 50K(B) | sample clock rate / level |
| **Ride** | Tune / Level | VR27 / VR26 | 10K(B) / 50K(B) | sample clock rate / level |

Pot part numbers (parts list): 100K(A)=EVJFDAF30A15, 1M(A)=EVJFDAF30A16,
500K(B)=EVJFDAF30B55/B52, 50K(B)=EVJFDAF30B54, 100K(B)=EVJFDAF30B15,
10K(B)=EVJFDAF30B14, 500Ω(B)=EVJFDAF30B52.

---

## 10. Voice-by-voice modelling takeaways

- **BD** = triggered self-resonant twin-T (C1 0.068 µF / R27 1.5 M), exponential
  pitch drop over tens of ms + separate click transient + diode soft-clip + VCA
  with 1 MΩ decay. Non-linearity lives in the VCA transistor + clamp diodes.
- **SD** = two reset-locked triangle VCOs (0.01 & 0.0068 µF → **1.47:1**,
  ~180/265 Hz) + ~20 ms pitch bend, summed with split HPF/LPF noise (snappy),
  transistor VCAs; TONE tilts, SNAPPY sets noise.
- **Toms** = three triangle VCOs per drum (cap triads above), downward pitch
  env, noise attack, exponential decay; L/M/H differ only in cap scaling.
- **Rim** = 3 parallel bandpasses (0.027/0.01/0.0047 µF → 219/496/1054 Hz) +
  diode clip + very short env + output HPF.
- **Clap** = bandpassed noise (0.0047 µF/150K) × AN6912 ~3-pulse burst
  (~8–10 ms) + 0.47 µF/1 M reverb tail, BA662 OTA VCA.
- **HH/Crash/Ride** = 6-bit PCM (HN61256P ROMs) → R-2R DAC → transistor/OTA VCA
  whose gain is an externally-reconstructed exponential decay (samples were
  amplitude-compressed) → fixed multi-pole LPF (5.6K / 1200pF / 2700pF / 390pF)
  → level. HH decay user-set (CH 10K+100K(B), OH 100K+1M(A) into 1 µF C135);
  crash/ride TUNE = sample clock rate. Accent applied through 2SA798 dual-PNP
  anti-log (exponential) converters.

---

## 11. Serial-number changes affecting the sound

- **S/N 381500:** BD C9 0.22 → **0.33 µF** (wider Tune range). Tape-sync tweaks
  (not voice).
- **S/N 393000:** Reset zener D701 RD6.8JB2 → RD5.6JB2 (reliability, not tone).
- **S/N 415300:** Rim Shot R417 **12K → 3.3K** (tighter/more realistic);
  Hi-Hat C134 **10 µF → 0.01 µF** (roll off lows); 0.01 µF shunts C500–C512 on
  the multi-outs.
- **S/N 426700:** Tom Noise C54 0.0022 → **0.0047 µF**, R194 47K → **100K**
  (emphasise tom attack).
- ROM Ver.1 vs Ver.2 = sequencer firmware only (no voice change).
