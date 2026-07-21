#!/usr/bin/env python3
# Drums, For Steve — the loom learns to phase.
#
# Companion to Drums, For Laurie. Same premise (the score is a program,
# the 909 is a pitched drum choir), different teacher: Steve Reich.
#
# The machinery, used exactly:
#   - SUBSTITUTION (Drumming): the piece begins with two strokes of a
#     twelve-step pattern and builds it one substituted rest at a time
#   - CONTINUOUS PHASING (Piano Phase / Drumming): the second drum voice
#     repeats the same pattern with a period of 2.96875 beats against
#     3.0 — sample-accurate continuous drift, then LOCK at +1 sixteenth,
#     drift again, lock at +2. The song format's absolute-beat seeks
#     make true tape-phase possible, not stepwise approximation
#   - RESULTING PATTERNS: the bells play only what the process creates —
#     the generator computes the actual coincidences of the two phased
#     voices at each lock and scores those, nothing else
#   - PULSES (Music for 18 Musicians): a four-chord cycle breathes in
#     organ stabs and — the sonic center — the VOCODER CHOIR singing
#     "ah", velocity-arced like players' lungs, over a maraca hat grid
#   - PHASE II: a mallet pair (musicbox vs mallet patch) runs the same
#     process on a twelve-NOTE cell, one octave up, drifting to +2
#   - UNRAVELING: subtraction in reverse order while both phased voices
#     drift home to unison; the last bar is the bare pattern, once,
#     in perfect sync — the journey is the piece
#
# Deterministic: same seed, same song.  python3 scripts/drums_for_steve.py
# writes songs/drums-for-steve.song.

import os
import random

SEED = 1971  # the year Drumming premiered
R = random.Random(SEED)

BPM = 112
S = 0.25              # one 16th (the 12/8 pulse), in beats
CYCLE = 12 * S        # the 12-step pattern, 3.0 beats
BARB = CYCLE          # one "bar" = one cycle of voice A

# ---------------------------------------------------------------- helpers

def fmt(x):
    s = f"{x:.6f}".rstrip("0").rstrip(".")
    return s if s else "0"

NAMES = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"]

def nn(midi):
    return f"{NAMES[midi % 12]}{midi // 12 - 1}"

def jit(v, amt=0.04):
    return max(0.05, min(1.0, v + R.uniform(-amt, amt)))

class Track:
    def __init__(self, header):
        self.header = header
        self.lines = []
    def bar(self, beat, tokens):
        if tokens:
            self.lines.append(f">{fmt(beat)} " + " ".join(tokens))
    def text(self):
        return "\n".join([self.header] + self.lines)

class Auto:
    def __init__(self, name, initial):
        self.name = name
        self.events = [(0.0, f"{fmt(initial)}")]
    def set(self, beat, v):
        self.events.append((beat, fmt(v)))
    def ramp(self, beat, to, dur, shape="lin"):
        self.events.append((beat, f"{fmt(to)}:{fmt(dur)}@{shape}"))
    def text(self):
        ev = sorted(self.events, key=lambda e: e[0])
        lines = [f"automate {self.name}", ev[0][1]]
        for beat, tok in ev[1:]:
            lines.append(f">{fmt(beat)} {tok}")
        return "\n".join(lines)

# ---------------------------------------------------------------- material

# The twelve-step bell pattern (the 7-stroke African bell Reich carried
# home from Ghana): X . X . X X . X . X . X
PATTERN = [1, 0, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1]
STEPS = [i for i in range(12) if PATTERN[i]]

# Each stroke has a pitch — tuned bongos, D dorian, singing a cell that
# rises to A and sighs back to D.
PITCH = {0: 50, 2: 53, 4: 52, 5: 57, 7: 55, 9: 52, 11: 48}  # D3 F3 E3 A3 G3 E3 C3

# the substitution order: how the pattern assembles, two strokes first
BUILD_ORDER = [0, 7, 5, 2, 9, 4, 11]

# the mallet cell: the PATTERN COMPLETED. Strokes keep their pitches an
# octave up; every rest the drums never filled is substituted with a
# passing tone — the mallets finish the process the kick began.
CELL = [62, 64, 65, 62, 64, 69, 71, 67, 65, 64, 62, 60]

# the pulse cycle: four chords, eight bars each (Music for 18's breath)
PULSE_CHORDS = [
    [38, 50, 57, 62, 64],    # D2 D3 A3 D4 E4
    [36, 48, 55, 60, 64],    # C2 C3 G3 C4 E4
    [40, 52, 59, 62, 67],    # E2 E3 B3 D4 G4
    [43, 50, 57, 60, 64],    # G2 D3 A3 C4 E4
]
VOX_CHORDS = [
    [57, 62, 64],            # A3 D4 E4
    [55, 60, 64],            # G3 C4 E4
    [59, 62, 67],            # B3 D4 G4
    [57, 60, 64],            # A3 C4 E4
]

def tune_of(midi):
    """The drum choir's register: D3-A3 across the tune knob."""
    return round(min(0.9, 0.14 + (midi - 48) / 26.0 * 0.75), 3)

# ---------------------------------------------------------------- sections
# bars are cycles of 3 beats (1.607 s at 112)
#
#   0-20     BUILD      substitution: the pattern assembles, stroke by stroke
#   20-52    PHASE I    voice B drifts ahead; locks at +1, drifts, locks at +2
#   52-84    PULSE      the four-chord cycle breathes; voices sing "ah"
#   84-116   PHASE II   the mallet pair runs the process an octave up
#   116-140  WEAVE      everything; the soprano sings the resulting melody
#   140-164  UNRAVEL    subtraction; both phases drift home; unison, once

A0, B0, C0, D0, E0, F0, END = 0, 20, 52, 84, 116, 140, 164

def bb(bar):
    return bar * BARB

# ---------------------------------------------------------------- tracks

kick  = Track("track kick kit=909 vel=0.8 len=0.25")
snare = Track("track snare kit=909 vel=0.7 len=0.25")
hats  = Track("track hats kit=909 vel=0.4 len=0.25")
m1    = Track("track m1 patch=musicbox vel=0.6 len=0.25")
m2    = Track("track m2 patch=mallet vel=0.55 len=0.25")
bell  = Track("track bell patch=glintbell vel=0.5 len=0.5")
organ = Track("track organ patch=pulsechord vel=0.55 len=0.5")
sop   = Track("track sop patch=glasslead vel=0.6 len=0.5")
drone = Track("track drone patch=drone vel=0.45 len=1")
pad   = Track("track pad patch=dreampad vel=0.4 len=1")
pad2  = Track("track pad2 patch=softpad vel=0.35 len=1")
choir = Track("track choir vox vel=0.5 len=0.5")

a_bd_tune = Auto("bd_tune", tune_of(50))
a_bd_dec  = Auto("bd_decay", 0.6)
a_bd_att  = Auto("bd_attack", 0.4)
a_sd_tune = Auto("sd_tune", 0.4)
a_sd_snap = Auto("sd_snappy", 0.08)   # the snare is the second tuned drum
a_sd_tone = Auto("sd_tone", 0.28)
a_sd_dec  = Auto("sd_decay", 0.55)
a_sd_lvl  = Auto("sd_level", 0.6)
a_hh_lvl  = Auto("hh_level", 0.0)
a_vox_lvl = Auto("vox_level", 0.0)
a_volume  = Auto("volume", 0.72)
a_m1_cut  = Auto("m1.cutoff", 3200)
a_m2_cut  = Auto("m2.cutoff", 3000)
a_org_cut = Auto("organ.cutoff", 1500)
a_pad_cut = Auto("pad.cutoff", 700)
a_pad_det = Auto("pad.detune", 5)
a_pad2_cut = Auto("pad2.cutoff", 900)
a_rev_wet = Auto("reverb_wet", 0.22)
a_sop_cut = Auto("sop.cutoff", 1800)
a_dr_drv  = Auto("dr_drive", 0.04)

# ------------------------------------------------ the phasing machinery

# the harmony the drums FOLLOW: chord-cycle root offsets by beat
CYCLE_ROOTS = [0, -2, 2, 5]      # D, C, E, G

def trans_at(beat):
    """Semitone offset of the pulse cycle's current chord — the drum
    choir retunes with the harmony instead of singing one key forever."""
    bar = beat / BARB
    if bar < C0:
        return 0
    return CYCLE_ROOTS[int((bar - C0) // 8) % 4]

def cycles(track, drum, tune_auto, t_start, t_end, period, vel_base,
           pattern=None, swell=None, follow=True, lift=0):
    """Repeat the 12-step pattern from t_start until t_end at the given
    period. period < 3.0 drifts the voice ahead (Reich's accelerating
    player); period == 3.0 holds. The accented stroke ROTATES one step
    per cycle (Clapping Music's moving downbeat), the pattern's pitches
    follow the pulse harmony, and `lift` raises chosen strokes an
    octave. Returns the beat where it stopped."""
    pattern = pattern if pattern is not None else PATTERN
    steps_here = [i for i in range(12) if pattern[i]]
    t = t_start
    c = 0
    while t < t_end - 1e-9:
        toks, cur = [], 0.0
        sw = swell(c, t) if swell else 1.0
        acc = steps_here[c % len(steps_here)] if steps_here else 0
        for i in range(12):
            if not pattern[i]:
                continue
            at = i * S
            if at > cur + 1e-9:
                toks.append(f"R:{fmt(at - cur)}")
                cur = at
            m = PITCH.get(i, 50) + (trans_at(t + at) if follow else 0)
            if lift and i == acc and c % 4 >= 2:
                m += 12
            tune_auto.set(t + at, tune_of(m))
            v = vel_base * (1.3 if i == acc else 1.0) * sw
            toks.append(f"{drum}:0.25@{jit(min(1.0, v)):.2f}")
            cur += 0.25
        track.bar(t, toks)
        t += period
        c += 1
    return t

def note_cycles(track, t_start, t_end, period, vel_base, transpose=0,
                swell=None):
    """The mallet cell, cycled the same way (Piano Phase's process)."""
    t = t_start
    c = 0
    while t < t_end - 1e-9:
        toks = []
        sw = swell(c, t) if swell else 1.0
        for i, m in enumerate(CELL):
            v = vel_base * (1.3 if i == 0 else 1.0) * sw
            toks.append(f"{nn(m + transpose)}:0.25@{jit(min(1.0, v), 0.03):.2f}")
        track.bar(t, toks)
        t += period
        c += 1
    return t

def resulting(offset_steps):
    """What the process creates: steps where BOTH phased voices strike.
    Scored for bells at the coincidence pitch, an octave up."""
    out = []
    for i in range(12):
        j = (i - offset_steps) % 12
        if PATTERN[i] and PATTERN[j]:
            out.append((i, max(PITCH[i], PITCH[j]) + 12))
    return out

# ---------------------------------------------------------------- A: BUILD

# substitution: begin with two strokes, add one every two bars
pat = [0] * 12
order = list(BUILD_ORDER)
pat[order.pop(0)] = 1
pat[order.pop(0)] = 1
bar = 0
while bar < A0 + 20:
    grow = bar >= 4 and bar % 2 == 0 and order
    if grow:
        pat[order.pop(0)] = 1
    vel = 0.5 + 0.28 * min(1.0, bar / 16)
    cycles(kick, "BD", a_bd_tune, bb(bar), bb(bar + 1), BARB, vel,
           pattern=list(pat))
    bar += 1

a_volume.ramp(0.0, 0.8, 20 * BARB)

# the maraca grid wakes under the finished pattern
a_hh_lvl.ramp(bb(12), 0.42, 8 * BARB)
for b in range(12, F0 + 16):
    toks = []
    breathe = 0.85 + 0.3 * (0.5 - abs((b % 8) / 8 - 0.5))
    for i in range(12):
        strong = i % 3 == b % 3 if b % 16 < 8 else i % 4 == b % 4
        v = (0.38 if strong else 0.2) * breathe
        toks.append(f"CH:0.25@{jit(v, 0.03):.2f}")
    if b % 8 == 7:
        toks[-1] = f"OH:0.25@{jit(0.4, 0.03):.2f}"
    hats.bar(bb(b), toks)

# the D pedal, breathing in eight-bar lungs
for b in range(8, C0, 8):
    drone.bar(bb(b), [f"D1:{fmt(8 * BARB * 0.96)}@0.45"])

# --------------------------------------------------------------- B: PHASE I

DRIFT8 = BARB - S / 8      # eight cycles to gain one sixteenth

# voice A holds; voice B enters in unison, drifts, locks, drifts, locks
cycles(kick, "BD", a_bd_tune, bb(B0), bb(E0), BARB, 0.76,
       swell=lambda c, t: 1.0 + 0.1 * (0.5 - abs(((t / BARB) % 16) / 16 - 0.5)))
cycles(kick, "BD", a_bd_tune, bb(E0), bb(F0), BARB, 0.82, lift=1,
       swell=lambda c, t: 1.0 + 0.15 * min(1.0, (t / BARB - E0) / 12))

tB = bb(B0)
tB = cycles(snare, "SD", a_sd_tune, tB, tB + 4 * BARB, BARB, 0.55)      # unison
tB = cycles(snare, "SD", a_sd_tune, tB, tB + 8 * DRIFT8, DRIFT8, 0.6)   # drift
lock1_start = tB
tB = cycles(snare, "SD", a_sd_tune, tB, bb(B0 + 20) + S, BARB, 0.62)    # lock +1
tB = cycles(snare, "SD", a_sd_tune, tB, tB + 8 * DRIFT8, DRIFT8, 0.64)  # drift
lock2_start = tB
# from here voice B holds +2 through PULSE, PHASE II and WEAVE
tB = cycles(snare, "SD", a_sd_tune, tB, bb(F0) + 2 * S, BARB, 0.62,
            swell=lambda c, t: 1.0 + 0.12 * (0.5 - abs(((t / BARB) % 8) / 8 - 0.5)))

# at each lock, the bells play what the phasing MADE — nothing else
for start, off, nbars, vel in ((lock1_start, 1, 6, 0.4),
                               (lock2_start, 2, 8, 0.45)):
    res = resulting(off)
    for b in range(0, nbars, 2):
        t0 = start + b * BARB
        toks, cur = [], 0.0
        for i, m in res:
            at = i * S
            if at > cur + 1e-9:
                toks.append(f"R:{fmt(at - cur)}")
                cur = at
            toks.append(f"{nn(m)}:0.25@{jit(vel, 0.03):.2f}")
            cur += 0.25
        bell.bar(t0, toks)

# ---------------------------------------------------------------- C: PULSE

# the four-chord cycle: organ stabs on the pattern's strong steps,
# the vocoder choir breathing "ah" in velocity arcs — players' lungs
a_vox_lvl.ramp(bb(C0), 0.55, 4 * BARB)
a_org_cut.ramp(bb(C0), 2600, 32 * BARB)

def pulse_cycle(first_bar, n_cycles, sung=True, stab_vel=0.5):
    import math
    for cyc in range(n_cycles):
        ci = cyc % 4
        t0 = bb(first_bar + cyc * 8)
        chord = PULSE_CHORDS[ci]
        vox = VOX_CHORDS[ci]
        # the held pad, in two staggered layers that ROLL open: the low
        # dyad arrives, the upper tones follow a bar later, and at the
        # chord's middle a high added voice joins (18 Musicians' singers
        # stepping forward), while the unison detune blooms and settles
        pad.bar(t0, ["[" + " ".join(nn(m) for m in chord[1:3])
                     + f"]:{fmt(8 * BARB * 0.96)}@0.42"])
        pad2.bar(t0 + BARB, ["[" + " ".join(nn(m) for m in chord[3:])
                             + f"]:{fmt(3 * BARB)}@0.36"])
        pad2.bar(t0 + 4 * BARB + 0.5,
                 ["[" + " ".join(nn(m) for m in chord[3:])
                  + " " + nn(chord[-1] + 7)
                  + f"]:{fmt(3.5 * BARB)}@0.4"])
        a_pad_cut.set(t0, 650)
        a_pad_cut.ramp(t0, 2100, 4 * BARB, "exp")
        a_pad_cut.ramp(t0 + 4 * BARB, 750, 4 * BARB, "exp")
        a_pad2_cut.set(t0 + BARB, 800)
        a_pad2_cut.ramp(t0 + BARB, 2600, 4 * BARB, "exp")
        a_pad_det.ramp(t0, 15, 4 * BARB, "smooth")
        a_pad_det.ramp(t0 + 4 * BARB, 5, 4 * BARB, "smooth")
        # the bass walks the cycle: root motion at last
        drone.bar(t0, [f"{nn(chord[0])}:{fmt(8 * BARB * 0.92)}@0.5"])
        a_org_cut.set(t0, 1100)
        a_org_cut.ramp(t0, 2800, 4 * BARB, "exp")
        a_org_cut.ramp(t0 + 4 * BARB, 1300, 4 * BARB, "exp")
        # organ: stabs whose VOICING climbs then falls across the chord,
        # velocity arced, with a soft echo stab answering off the beat
        for b in range(8):
            arc = math.sin(math.pi * (b + 0.5) / 8)
            voicing = list(chord[1:])
            if b % 4 >= 2:
                voicing = voicing[1:] + [voicing[0] + 12]   # first inversion
            if b >= 6:
                voicing = [voicing[0] - 12] + voicing[1:]   # settling low
            vch = "[" + " ".join(nn(m) for m in voicing) + "]"
            toks, cur = [], 0.0
            hits = ((0, stab_vel * (0.7 + 0.5 * arc)),
                    (5, stab_vel * (0.55 + 0.4 * arc)),
                    (7, stab_vel * (0.6 + 0.45 * arc)),
                    (9, stab_vel * 0.35)) if b % 2 == 0 else (
                    (2, stab_vel * 0.4), (7, stab_vel * (0.5 + 0.35 * arc)))
            for i, v in hits:
                at = i * S
                if at > cur + 1e-9:
                    toks.append(f"R:{fmt(at - cur)}")
                    cur = at
                toks.append(f"{vch}:0.25@{jit(min(1.0, v), 0.03):.2f}")
                cur += 0.25
            organ.bar(t0 + b * BARB, toks)
        # the lungs: vox level itself breathes with each chord
        a_vox_lvl.ramp(t0, 0.66, 4 * BARB, "smooth")
        a_vox_lvl.ramp(t0 + 4 * BARB, 0.4, 4 * BARB, "smooth")
        if sung:
            for half in range(2):
                tb = t0 + half * 4 * BARB
                toks = []
                for pu in range(8):
                    arc2 = math.sin(math.pi * (pu + 0.5) / 8)
                    v = 0.25 + 0.5 * arc2
                    toks.append("[" + " ".join(nn(m) for m in vox)
                                + f"]:1.5@{v:.2f}=AA")
                choir.bar(tb, toks)

pulse_cycle(C0, 4)

# ------------------------------------------------------------- D: PHASE II

# the mallet pair, one octave up, same process, faster to lock
m1_end = bb(F0 + 8)
note_cycles(m1, bb(D0), m1_end, BARB, 0.5,
            swell=lambda c, t: 0.85 + 0.15 * min(1.0, c / 8))
tM = bb(D0)
tM = note_cycles(m2, tM, tM + 8 * BARB, BARB, 0.42)                   # unison
DRIFT16 = BARB - S / 8
tM = note_cycles(m2, tM, tM + 8 * DRIFT16, DRIFT16, 0.46)             # drift +1
tM = note_cycles(m2, tM, tM + 8 * BARB + S, BARB, 0.48)               # lock +1
tM = note_cycles(m2, tM, tM + 8 * DRIFT16, DRIFT16, 0.5)              # drift +2
m2_lock2 = tM
tM = note_cycles(m2, tM, bb(F0) + 2 * S, BARB, 0.48)                  # lock +2

pulse_cycle(D0, 4, sung=True, stab_vel=0.45)
a_m2_cut.ramp(bb(D0), 4200, 32 * BARB)

# the resulting patterns never stop being true: bells keep scoring them,
# denser as the weave approaches
RESD = resulting(2)
for b in range(D0 + 4, F0 - 2, 4):
    t0 = bb(b)
    dense = b >= E0
    picks = RESD if dense else RESD[::2]
    toks, cur = [], 0.0
    for i, m in picks:
        at = i * S
        if at > cur + 1e-9:
            toks.append(f"R:{fmt(at - cur)}")
            cur = at
        toks.append(f"{nn(m + trans_at(t0))}:0.25@{jit(0.4 if dense else 0.32, 0.03):.2f}")
        cur += 0.25
    bell.bar(t0, toks)

# ----------------------------------------------------------------- E: WEAVE

# the soprano sings the +2 resulting melody in long notes over everything
RES2 = resulting(2)
for b in range(E0, F0 - 4, 4):
    t0 = bb(b)
    toks, cur = [], 0.0
    picks = RES2[:: 2] if (b // 4) % 2 == 0 else RES2[1:: 2]
    for i, m in picks:
        at = i * S * 4          # augmentation: the melody at quarter speed
        if at - 0.5 > cur + 1e-9:
            toks.append(f"R:{fmt(at - 0.5 - cur)}")
            toks.append(f"{nn(m + 14)}:0.5@{jit(0.42, 0.03):.2f}")
            cur = at
        elif at > cur + 1e-9:
            toks.append(f"R:{fmt(at - cur)}")
            cur = at
        dur = 2.0
        toks.append(f"{nn(m + 12)}:{fmt(dur)}@{jit(0.62, 0.03):.2f}")
        cur = at + dur
    sop.bar(t0, toks)

pulse_cycle(E0, 3)
a_volume.ramp(bb(E0), 0.9, 12 * BARB)
a_dr_drv.ramp(bb(E0), 0.22, 16 * BARB)
a_dr_drv.ramp(bb(F0), 0.05, 8 * BARB)
a_bd_dec.ramp(bb(E0), 0.68, 8 * BARB)
a_sop_cut.ramp(bb(E0), 3400, 20 * BARB, "exp")
a_sd_lvl.ramp(bb(E0), 0.68, 12 * BARB)

# --------------------------------------------------------------- F: UNRAVEL

# subtraction, in reverse; voice B and m2 drift HOME (period > 3.0)
pat = list(PATTERN)
unorder = list(reversed(BUILD_ORDER))
bar = F0
while bar < END - 2:
    if bar % 2 == 0 and len(unorder) > 2:
        pat[unorder.pop(0)] = 0
    vel = 0.75 - 0.35 * (bar - F0) / (END - F0)
    cycles(kick, "BD", a_bd_tune, bb(bar), bb(bar + 1), BARB, vel,
           pattern=list(pat))
    bar += 1

HOME8 = BARB + S / 4       # eight cycles to give back two sixteenths
tB2 = bb(F0) + 2 * S
tB2 = cycles(snare, "SD", a_sd_tune, tB2, tB2 + 8 * HOME8, HOME8, 0.5)
cycles(snare, "SD", a_sd_tune, tB2, bb(END - 2), BARB, 0.42)

tM2 = bb(F0) + 2 * S
tM2 = note_cycles(m2, tM2, tM2 + 8 * HOME8, HOME8, 0.4)
note_cycles(m2, tM2, bb(F0 + 8), BARB, 0.35)

a_rev_wet.ramp(bb(E0), 0.3, 16 * BARB)
a_rev_wet.ramp(bb(F0), 0.4, 16 * BARB)
a_vox_lvl.ramp(bb(F0), 0.3, 12 * BARB)
a_bd_dec.ramp(bb(F0), 0.55, 8 * BARB)
a_pad_cut.ramp(bb(F0), 500, 16 * BARB, "exp")
a_hh_lvl.ramp(bb(F0 + 8), 0.0, 8 * BARB)
a_volume.ramp(bb(F0 + 8), 0.6, 12 * BARB)

# the last breath: one chord, then the bare pattern, once, in unison
choir.bar(bb(END - 6), ["[" + " ".join(nn(m) for m in VOX_CHORDS[0])
                        + f"]:9=AA@0.45"])
a_vox_lvl.ramp(bb(END - 4), 0.0, 4 * BARB)

t_final = bb(END - 2)
toks, cur = [], 0.0
for i in STEPS:
    at = i * S
    if at > cur + 1e-9:
        toks.append(f"R:{fmt(at - cur)}")
        cur = at
    a_bd_tune.set(t_final + at, tune_of(PITCH[i]))
    a_sd_tune.set(t_final + at, tune_of(PITCH[i]))
    toks.append(f"BD:0.25@0.6")
    cur += 0.25
kick.bar(t_final, toks)
toks2 = toks[:]
snare.bar(t_final, [t.replace("BD", "SD").replace("@0.6", "@0.45")
                    for t in toks2])
drone.bar(t_final, [f"D1:{fmt(2 * BARB)}@0.4"])
bell.bar(bb(END - 1), [f"D6:{fmt(2 * BARB)}@0.5"])
a_volume.ramp(bb(END - 1) + 0.5, 0.0, 2.5 * BARB, "smooth")

# ---------------------------------------------------------------- emit

HEADER = f"""# Drums, For Steve — 112 bpm, 12/8, D dorian. Companion to Drums,
# For Laurie: the same pitched 909 drum choir, taught by Steve Reich.
# Generated by scripts/drums_for_steve.py (seed {SEED} — Drumming's year).
#
# The processes, used exactly: SUBSTITUTION assembles a 7-stroke
# twelve-step bell pattern two strokes at a time; a second tuned drum
# CONTINUOUSLY PHASES against the first (period 2.96875 vs 3.0 —
# sample-accurate tape-phase, then LOCKS at +1 and +2 sixteenths);
# the bells play only the RESULTING PATTERNS the phasing actually
# creates; a four-chord PULSE cycle breathes in organ stabs and the
# vocoder choir's "ah" (velocity arcs = players' lungs); a mallet
# pair runs the same phase process an octave up; then subtraction
# unbuilds the pattern while both drifted voices come HOME, and the
# last bar is the bare pattern, once, in perfect unison.
#
#   bars   0-20    BUILD      substitution, stroke by stroke
#   bars  20-52    PHASE I    drift / lock +1 / drift / lock +2
#   bars  52-84    PULSE      four chords breathing, voices enter
#   bars  84-116   PHASE II   the mallet pair, an octave up
#   bars 116-140   WEAVE      soprano sings the resulting melody, augmented
#   bars 140-164   UNRAVEL    subtraction; the phases come home; unison
#
# Regenerate: python3 scripts/drums_for_steve.py

bpm {BPM}
gate 0.9

automate reverb_decay
0.7
automate spring
0.06
automate tape_wow
0.04
automate tape_flutter
0.03
automate tape_age
0.22
automate tape_drive
0.2
automate bd_level
0.85
automate cp_level
0
automate rs_level
0
automate hh_tune
0.55
automate hh_metal
0.4
automate vox_mode
1
automate vox_intonation
0.05
automate vox_breath
0.35
automate vox_vibrato
0.25
# the vox carrier is the panel: give it something worth singing through
automate waveform
2
automate detune
7
automate attack
0.06
automate release
0.35
automate cutoff
2200
automate sustain
0.8
"""

parts = [HEADER]
for a in (a_bd_tune, a_bd_dec, a_bd_att, a_sd_tune, a_sd_snap, a_sd_tone,
          a_sd_dec, a_sd_lvl, a_hh_lvl, a_vox_lvl, a_volume, a_dr_drv, a_rev_wet):
    parts.append(a.text())
for t in (kick, snare, hats, m1, m2, bell, organ, sop, drone, pad, pad2, choir):
    parts.append(t.text())
parts.append(a_m1_cut.text())
parts.append(a_m2_cut.text())
parts.append(a_org_cut.text())
parts.append(a_pad_cut.text())
parts.append(a_pad_det.text())
parts.append(a_pad2_cut.text())
parts.append(a_sop_cut.text())

out = "\n\n".join(parts) + "\n"
repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
path = os.path.join(repo, "songs", "drums-for-steve.song")
with open(path, "w") as f:
    f.write(out)
print(f"wrote {path}: {len(out.splitlines())} lines, "
      f"{END * BARB * 60 / BPM:.0f}s before tail")
