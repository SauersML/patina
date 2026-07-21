#!/usr/bin/env python3
# One Note at a Time — The Carrying, morphed into minimalism.
#
# In The Carrying the melody changed hands six times, in big statements.
# Here the same idea runs continuously: a four-beat cell repeats without
# stopping, and every third repetition ONE thing changes — a pitch slips
# a step, the peak lifts, a note is forgotten (the gap stays), an eighth
# splits into two sixteenths. Twenty changes, spread over three minutes:
# you never catch the moment, and at the end the cell is a different
# melody you never heard arrive.
#
# The carrying is made audible as a ROUND: a second voice plays the cell
# one change behind, a third plays it two behind — every mutation
# ripples through them like a rumor, two and four bars later. Where the
# leader and the follower happen to sound the same pitch at the same
# instant, a bell sparkles: the resulting pattern, found not composed.
#
# When the twentieth change lands, the followers catch up, the canon
# collapses to unison — four repetitions of the final cell, together —
# and then the cell is played once in augmentation, four times slower:
# the busy pattern, revealed as a song. It was a melody all along.
#
# No physically-realistic voices anywhere: no drums, no brass, nothing
# pretending to be wood or skin. Glass, sine, pulse and ring-mod only.
# The pulse is a dyad, the ground is a sine pedal, the harmony is three
# pad tones that also move one note at a time, every eight bars.
#
# Deterministic: seed 5.  python3 scripts/one_note_at_a_time.py
# writes songs/one-note-at-a-time.song.

import os
import random

SEED = 5
R = random.Random(SEED)

BPM = 132
CELL_SPAN = 4.0

# ---------------------------------------------------------------- helpers

def fmt(x):
    s = f"{x:.6f}".rstrip("0").rstrip(".")
    return s if s else "0"

NAMES = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"]

def nn(midi):
    return f"{NAMES[midi % 12]}{midi // 12 - 1}"

def jit(v, amt=0.02):
    return max(0.05, min(1.0, v + R.uniform(-amt, amt)))

class Track:
    def __init__(self, header):
        self.header = header
        self.lines = []
    def at(self, beat, tokens):
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

EMIN = [64, 66, 67, 69, 71, 72, 74]

def scale_step(midi, step):
    octave, best = 0, None
    for o in (-24, -12, 0, 12):
        for sc in EMIN:
            c = sc + o
            if best is None or abs(c - midi) < abs(best - midi):
                best, octave = c, o
    idx = EMIN.index(best - octave) + step
    oshift = octave
    while idx < 0:
        idx += 7; oshift -= 12
    while idx >= 7:
        idx -= 7; oshift += 12
    return EMIN[idx] + oshift

# ------------------------------------------------------------ the process

# The cell: The Carrying's opening phrase folded into four beats of
# eighths. Rise E-G-A-B, fall B-G-E, D leading back around.
# events: (offset, midi, dur)
CELL_0 = [
    (0.0, 64, 0.5), (0.5, 67, 0.5), (1.0, 69, 0.5), (1.5, 71, 0.5),
    (2.0, 71, 0.5), (2.5, 67, 0.5), (3.0, 64, 0.5), (3.5, 62, 0.5),
]

def op_slip(cell):
    cands = [i for i in range(1, len(cell))]
    i = R.choice(cands)
    o, m, d = cell[i]
    cell[i] = (o, scale_step(m, R.choice([-1, 1])), d)
    return cell

def op_lift(cell):
    i = max(range(len(cell)), key=lambda k: cell[k][1])
    o, m, d = cell[i]
    cell[i] = (o, min(79, scale_step(m, 1)), d)
    return cell

def op_forget(cell):
    cands = [i for i in range(1, len(cell)) if cell[i][2] >= 0.5]
    if len(cell) > 6 and cands:
        cell.pop(R.choice(cands))
    return cell

def op_split(cell):
    cands = [i for i in range(len(cell)) if cell[i][2] == 0.5]
    if cands:
        i = R.choice(cands)
        o, m, d = cell[i]
        cell[i] = (o, m, 0.25)
        cell.insert(i + 1, (o + 0.25, scale_step(m, 1), 0.25))
    return cell

# twenty changes: shuffled, but the piece opens gently (slips first)
OPS = [op_slip] * 8 + [op_lift] * 4 + [op_forget] * 3 + [op_split] * 5
tail = OPS[2:]
R.shuffle(tail)
OPS = OPS[:2] + tail

VERSIONS = [list(CELL_0)]
for op in OPS:
    VERSIONS.append(sorted(op([tuple(n) for n in VERSIONS[-1]]),
                           key=lambda n: n[0]))
N_V = len(VERSIONS) - 1   # 20

# ---------------------------------------------------------------- timeline

A_START = 16.0            # the pulse establishes the grid first
B_START = 48.0 + 2.0      # follower, half a cell behind, one change behind
C_START = 80.0 + 1.0      # third voice, two changes behind, high and faint

def version_at(beat):
    """Which version the LEADER sings in the cell starting at `beat`."""
    r = int((beat - A_START) // CELL_SPAN)
    if r < 2:
        return 0
    return min(N_V, (r - 2) // 3 + 1)

FREEZE_R = 2 + 3 * N_V                     # leader rep where changes end
FREEZE_T = A_START + FREEZE_R * CELL_SPAN  # beat 264 + 16 = ...
CATCH_T = FREEZE_T + 6 * CELL_SPAN         # followers reach the final cell
UNISON_T = CATCH_T + 2 * CELL_SPAN
AUG_T = UNISON_T + 4 * CELL_SPAN           # four unison reps, then...
END_T = AUG_T + 16.0 + 14.0                # ...the augmentation and the rest

# ---------------------------------------------------------------- tracks

lead = Track("track lead patch=glasslead vel=0.55 gain=0.9 reverb_send=0.12")
low  = Track("track low patch=sunlead vel=0.5 gain=0.85 pan=-0.4 reverb_send=0.1")
high = Track("track high patch=glasshigh vel=0.4 gain=0.7 pan=0.45 reverb_send=0.2")
spark = Track("track spark patch=glintbell vel=0.35 gain=0.8 pan=0.25 spring_send=0.2")
pulse = Track("track pulse patch=pulsechord vel=0.3 gain=0.8 pan=0.15")
pad   = Track("track pad patch=dreampad vel=0.4 gain=0.85 chorus_send=0.7")
pad2  = Track("track pad2 patch=softpad vel=0.34 gain=0.8 pan=-0.2 chorus_send=0.7")
ground = Track("track ground patch=deepsine vel=0.5 gain=0.9")

a_bpm    = Auto("bpm", BPM)
a_volume = Auto("volume", 0.62)
a_spring = Auto("spring", 0.05)
a_revwet = Auto("reverb_wet", 0.16)
a_wow    = Auto("tape_wow", 0.15)

def emit_cell(track, cell, t0, shift, vel):
    toks, cur = [], 0.0
    for o, m, d in cell:
        if o > cur + 1e-9:
            toks.append(f"R:{fmt(o - cur)}")
            cur = o
        toks.append(f"{nn(m + shift)}:{fmt(d)}@{jit(vel):.2f}")
        cur = o + d
    track.at(t0, toks)

# ------------------------------------------------------- the three voices

def breathe(beat, period=32.0, depth=0.12):
    import math
    return 1.0 + depth * math.sin(2 * math.pi * beat / period)

# leader: continuous from A_START to the augmentation
t = A_START
while t < UNISON_T + 4 * CELL_SPAN - 1e-9:
    vi = min(N_V, version_at(t))
    emit_cell(lead, VERSIONS[vi], t, 0, 0.55 * breathe(t)
              * (1.1 if t >= UNISON_T else 1.0))
    t += CELL_SPAN

# low follower: one change behind, half a cell offset — until the
# collapse, where it waits one cell and re-enters ON the grid
t = B_START
while t < CATCH_T - 1e-9:
    vi = max(0, min(N_V, version_at(t) - 1))
    emit_cell(low, VERSIONS[vi], t, -12, 0.5 * breathe(t + 8))
    t += CELL_SPAN
t = CATCH_T
while t < UNISON_T + 4 * CELL_SPAN - 1e-9:
    emit_cell(low, VERSIONS[N_V], t, -12,
              0.5 * (1.15 if t >= UNISON_T else 1.0))
    t += CELL_SPAN

# high follower: two changes behind, a beat late, faint — the far echo
t = C_START
while t < CATCH_T + CELL_SPAN - 1e-9:
    vi = max(0, min(N_V, version_at(t) - 2))
    emit_cell(high, VERSIONS[vi], t, 12, 0.4 * breathe(t + 16))
    t += CELL_SPAN
t = UNISON_T                        # shed the 1-beat lateness, join the grid
while t < UNISON_T + 3 * CELL_SPAN - 1e-9:
    emit_cell(high, VERSIONS[N_V], t, 12, 0.42)
    t += CELL_SPAN

# ------------------------------------------- the found bells (emergence)

# where leader and low follower sound the SAME pitch at the same instant,
# on a quarter-note, a bell agrees — computed, not composed
t = B_START
seen = 0
while t < FREEZE_T - 1e-9 and seen < 60:
    vb = VERSIONS[max(0, min(N_V, version_at(t) - 1))]
    for o, m, d in vb:
        abs_t = t + o
        r_a = int((abs_t - A_START) // CELL_SPAN)
        ta = A_START + r_a * CELL_SPAN
        va = VERSIONS[min(N_V, version_at(ta))]
        for oa, ma, da in va:
            if abs(ta + oa - abs_t) < 1e-6 and ma == m and (abs_t % 1.0) == 0.0:
                spark.at(abs_t, [f"{nn(m + 12)}:0.5@{jit(0.32):.2f}"])
                seen += 1
    t += CELL_SPAN

# ---------------------------------------------------------- pulse & pads

# the pulse: an E-B dyad in eighths, accents rotating three-against-four,
# breathing in eight-bar waves; it is the first and last thing heard
t = 0.0
bar = 0
while t < AUG_T - 1e-9:
    toks = []
    for e in range(8):
        acc = (e % 3) == (bar % 3)
        v = (0.34 if acc else 0.2) * breathe(t, 64.0, 0.18)
        toks.append(f"[E3 B3]:0.5@{jit(v):.2f}")
    pulse.at(t, toks)
    t += CELL_SPAN
    bar += 1

# the harmony is carried too: three pad tones, one moving every 8 bars
WALK = [
    [40, 47, 55], [40, 48, 55], [40, 48, 57], [40, 50, 57],
    [40, 50, 59], [40, 52, 59], [40, 52, 60], [40, 50, 60],
    [40, 50, 59], [40, 48, 59], [40, 47, 59], [40, 47, 55],
]
seg = 32.0
for k, chord in enumerate(WALK):
    tb = k * seg
    if tb >= AUG_T - 30:
        break
    pad.at(tb, ["[" + " ".join(nn(m) for m in chord) + "]"
                + f":{fmt(seg * 0.98)}@{jit(0.4):.2f}"])
    pad2.at(tb + 1.0, ["[" + " ".join(nn(m + 12) for m in chord[1:]) + "]"
                       + f":{fmt(seg - 2.0)}@{jit(0.34):.2f}"])
    ground.at(tb, [f"E2:{fmt(seg * 0.96)}@{jit(0.5):.2f}"])

# ------------------------------------------------------- the augmentation
# The final cell, four times slower: the pattern revealed as a song.

aug = [(o * 4.0, m, d * 4.0) for o, m, d in VERSIONS[N_V]]
emit_cell(lead, aug, AUG_T, 0, 0.52)
emit_cell(low, [(o, m, d) for o, m, d in aug], AUG_T, -12, 0.3)
pad.at(AUG_T, [f"[{nn(40)} {nn(47)} {nn(55)}]:{fmt(16.0)}@0.4"])
pad2.at(AUG_T + 1.0, [f"[{nn(59)} {nn(64)} {nn(67)}]:{fmt(14.0)}@0.32"])
ground.at(AUG_T, [f"E2:{fmt(16.0)}@0.48"])

# the close: one bell on the tonic, the springs keep the rest
spark.at(AUG_T + 16.0, [f"E6:4@0.4"])
pad.at(AUG_T + 16.0, [f"[{nn(40)} {nn(47)} {nn(55)}]:{fmt(10.0)}@0.36"])
ground.at(AUG_T + 16.0, [f"E1:{fmt(10.0)}@0.45"])
a_spring.ramp(AUG_T, 0.2, 12.0)
a_revwet.ramp(AUG_T, 0.26, 12.0)
a_wow.ramp(AUG_T + 12.0, 0.28, 12.0, "exp")

# dynamics of the whole: accumulation is the crescendo; the desk only
# helps it breathe
a_volume.ramp(A_START, 0.7, 64.0)
a_volume.ramp(B_START + 32.0, 0.78, 96.0)
a_volume.set(AUG_T, 0.68)
a_volume.ramp(AUG_T + 18.0, 0.0, 10.0, "smooth")

# steadiness is the aesthetic: one imperceptible lean forward across the
# whole process, and a small letting-go for the augmentation only
a_bpm.ramp(A_START, 136.0, FREEZE_T - A_START)
a_bpm.set(AUG_T, 126.0)
a_bpm.ramp(AUG_T + 8.0, 112.0, 18.0, "smooth")

# ---------------------------------------------------------------- emit

HEADER = f"""# One Note at a Time — E minor, 132 bpm, seed {SEED}. The Carrying,
# morphed into minimalism (scripts/one_note_at_a_time.py).
#
# A four-beat cell repeats continuously; every third repetition ONE
# thing changes — a pitch slips, the peak lifts, a note is forgotten,
# an eighth splits. Twenty changes. You never catch the moment, and
# by the end the cell is a different melody you never heard arrive.
# A low voice sings one change behind, a high voice two behind: every
# mutation ripples through the round like a rumor. Where leader and
# follower sound the same pitch at the same instant, a bell agrees —
# the resulting pattern, found not composed. The three pad tones are
# carried too: one moves every eight bars. When the last change
# lands, the canon collapses to unison, and the final cell is played
# once in augmentation, four times slower — the busy pattern revealed
# as a song. It was a melody all along.
#
# No physically-realistic voices: no drums, no brass, nothing
# pretending to be wood or skin. Glass, sine, pulse, ring-mod.
#
# Regenerate: python3 scripts/one_note_at_a_time.py

bpm {BPM}
gate 0.86

automate chorus_mode
2
automate chorus_mix
0
automate chorus_depth
0.3
automate chorus_rate
0.35
automate reverb_decay
0.68
automate tape_flutter
0.03
automate tape_age
0.22
automate tape_drive
0.18
automate bd_level
0
automate sd_level
0
automate hh_level
0
automate rs_level
0
automate cp_level
0
"""

parts = [HEADER]
for a in (a_bpm, a_volume, a_spring, a_revwet, a_wow):
    parts.append(a.text())
for t in (lead, low, high, spark, pulse, pad, pad2, ground):
    parts.append(t.text())

out = "\n\n".join(parts) + "\n"
repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
path = os.path.join(repo, "songs", "one-note-at-a-time.song")
with open(path, "w") as f:
    f.write(out)
print(f"wrote {path}: {len(out.splitlines())} lines, ~{END_T:.0f} beats, "
      f"{N_V} changes, freeze at beat {FREEZE_T:.0f}")
