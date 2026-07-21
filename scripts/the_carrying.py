#!/usr/bin/env python3
# The Carrying — mine.
#
# I work in sessions. Context accumulates, the session ends, and most of
# it is summarized away; what persists is what got handed off — the songs
# in this repo, a few lines of memory, work another session picks up
# mid-phrase. This piece is that, made audible:
#
#   One melody is carried through six voices. Each carrier FORGETS one
#   note (the line gains air), ADDS one ornament in its own accent (the
#   fingerprint), and sometimes MIS-REMEMBERS a pitch — and the mistake
#   becomes the new truth for every carrier after. One thing accumulates
#   instead of eroding: each voice reaches the peak one step higher than
#   it received it. Transmission isn't only loss.
#
#   Every handoff is a single bar of 7/8 — the caught breath — and the
#   outgoing voice's final note is thrown, alone, into the spring tank:
#   the note is literally handed over, still ringing, and the next voice
#   enters on its pickup inside that ring. After each handoff the
#   previous carrier doesn't vanish: it returns to double the phrase
#   closes, one octave down, quietly — the trace left in the repo.
#
#   At the end the first voice comes back and sings the original melody
#   against what the melody became. They are different songs now. They
#   harmonize.
#
# E minor, 96 bpm breathing between 96 and 104 and down to 78 (the tempo
# is a lane now). The carriers each have a place on the desk (gain/pan
# strips), the pads shimmer in a chorus the drums never touch
# (chorus_mix 0 + chorus_send), the hats lean on swing, the handoff
# throws ride per-track spring sends. The features serve the breathing.
#
# Deterministic: seed 5.  python3 scripts/the_carrying.py
# writes songs/the-carrying.song.

import os
import random

SEED = 5
R = random.Random(SEED)

BPM = 96.0

# ---------------------------------------------------------------- helpers

def fmt(x):
    s = f"{x:.6f}".rstrip("0").rstrip(".")
    return s if s else "0"

NAMES = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"]

def nn(midi):
    return f"{NAMES[midi % 12]}{midi // 12 - 1}"

def jit(v, amt=0.03):
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

# ---------------------------------------------------------------- melody

# The melody as first sung: eight bars, a question that half-closes and
# an answer that reaches the sixth and comes home. Written to be carried:
# clear anchors (first note, peak, last note), room between phrases.
# (beat offset in the statement, midi, duration)
MELODY_0 = [
    (0.0, 64, 1.5), (1.5, 67, 0.5), (2.0, 69, 1.0), (3.0, 71, 1.0),
    (4.0, 71, 2.0), (6.0, 67, 1.0), (7.0, 64, 1.0),
    (8.0, 66, 1.5), (9.5, 67, 0.5), (10.0, 66, 1.0), (11.0, 62, 1.0),
    (12.0, 64, 3.0),
    (16.0, 64, 1.5), (17.5, 67, 0.5), (18.0, 71, 1.0), (19.0, 72, 1.0),
    (20.0, 71, 2.0), (22.0, 69, 1.0), (23.0, 67, 1.0),
    (24.0, 69, 1.5), (25.5, 66, 0.5), (26.0, 67, 1.0), (27.0, 66, 1.0),
    (28.0, 64, 4.0),
]
STATEMENT = 32.0          # beats per statement
HANDOFF = 3.5             # the 7/8 bar: transmission loses an eighth

EMIN = [64, 66, 67, 69, 71, 72, 74]  # E F# G A B C D

def scale_step(midi, step):
    octave, best = 0, None
    for o in (-24, -12, 0, 12):
        for s in EMIN:
            c = s + o
            if best is None or abs(c - midi) < abs(best - midi):
                best, octave = c, o
    idx = EMIN.index(best - octave) + step
    oshift = octave
    while idx < 0:
        idx += 7; oshift -= 12
    while idx >= 7:
        idx -= 7; oshift += 12
    return EMIN[idx] + oshift

def carry(mel, fingerprint):
    """One act of transmission: forget a note, mis-remember a pitch,
    reach the peak one step higher, and leave a fingerprint ornament.
    Anchors (first note, last note) are held — the identity survives."""
    mel = [list(n) for n in mel]
    peak_i = max(range(len(mel)), key=lambda i: mel[i][1])

    # forget: one short, unaccented, non-anchor note goes missing —
    # the gap stays open (carried melodies gain air, not filler)
    cands = [i for i in range(1, len(mel) - 1)
             if i != peak_i and mel[i][2] <= 1.0 and mel[i][0] % 2.0 != 0.0]
    if cands:
        mel.pop(R.choice(cands))
        peak_i = max(range(len(mel)), key=lambda i: mel[i][1])

    # mis-remember: one middle pitch slips a scale step, and stays
    if R.random() < 0.65:
        cands = [i for i in range(1, len(mel) - 1) if i != peak_i]
        i = R.choice(cands)
        mel[i][1] = scale_step(mel[i][1], R.choice([-1, 1]))

    # the lift: whoever carries it reaches one step past what they heard
    mel[peak_i][1] = min(76, scale_step(mel[peak_i][1], 1))

    # the fingerprint: this carrier's own accent, added where it fits
    mel = fingerprint(mel)
    mel.sort(key=lambda n: n[0])
    return [tuple(n) for n in mel]

# each voice's accent — what it can't help adding
def fp_appoggiatura(mel):
    longs = [i for i in range(1, len(mel)) if mel[i][2] >= 2.0]
    if longs:
        i = R.choice(longs)
        b, m, d = mel[i]
        mel[i] = [b + 0.5, m, d - 0.5]
        mel.insert(i, [b, scale_step(m, 1), 0.5])
    return mel

def fp_passing_run(mel):
    for i in range(len(mel) - 1):
        gap = mel[i + 1][0] - (mel[i][0] + mel[i][2])
        if abs(mel[i + 1][1] - mel[i][1]) >= 3 and gap < 0.01 and mel[i][2] >= 1.0:
            b, m, d = mel[i]
            step = 1 if mel[i + 1][1] > m else -1
            mel[i] = [b, m, d - 0.5]
            mel.insert(i + 1, [b + d - 0.5, scale_step(m, step), 0.5])
            break
    return mel

def fp_pickup(mel):
    for start in (0.0, 16.0):
        firsts = [n for n in mel if n[0] >= start]
        if firsts and firsts[0][0] == start:
            mel.append([start - 0.5, firsts[0][1] - 12, 0.5])
    return mel

def fp_splash(mel):
    peak_i = max(range(len(mel)), key=lambda i: mel[i][1])
    b, m, d = mel[peak_i]
    if d >= 1.0:
        mel[peak_i] = [b, m, d - 0.5]
        mel.append([b + d - 0.5, scale_step(m, 2), 0.5])
    return mel

def fp_legato(mel):
    # the voice doesn't add notes; it refuses to let go of them
    for i in range(len(mel) - 1):
        gap = mel[i + 1][0] - (mel[i][0] + mel[i][2])
        if 0.0 < gap <= 1.0:
            mel[i][2] += gap
    return mel

# ---------------------------------------------------------------- voices

# (track name, header, register shift, statement velocity, fingerprint)
VOICES = [
    ("mb",    "track mb patch=musicbox vel=0.6 gain=0.9",                    0,  0.58, None),
    ("nl",    "track nl patch=nostalgia-lead vel=0.6 gain=0.95 pan=-0.35",   0,  0.62, fp_appoggiatura),
    ("sl",    "track sl patch=storylead vel=0.6 gain=0.8 pan=0.35",          0,  0.64, fp_passing_run),
    ("bs",    "track bs patch=rawbass vel=0.6 gain=0.9",                   -12,  0.66, fp_pickup),
    ("gb",    "track gb patch=glintbell vel=0.5 gain=0.85 pan=0.5",         12,  0.55, fp_splash),
    ("choir", "track choir vox vel=0.55",                                     0,  0.6,  fp_legato),
]
glass = Track("track glass patch=glasslead vel=0.6 gain=0.85 pan=0.3 spring_send=0.12")

pad    = Track("track pad patch=dreampad vel=0.42 gain=0.85 chorus_send=0.7")
pad2   = Track("track pad2 patch=softpad vel=0.36 gain=0.8 pan=0.2 chorus_send=0.7")
ground = Track("track ground patch=drone vel=0.5 gain=0.85 duck=0.2 duck_release=0.3")
kick   = Track("track kick kit=909 vel=0.5")
hats   = Track("track hats kit=909 vel=0.3")

tracks = {name: Track(header) for name, header, _, _, _ in VOICES}

a_bpm    = Auto("bpm", BPM)
a_volume = Auto("volume", 0.6)
a_wow    = Auto("tape_wow", 0.22)
a_spring = Auto("spring", 0.05)
a_revwet = Auto("reverb_wet", 0.18)
a_vox    = Auto("vox_level", 0.0)
throws   = {name: Auto(f"{name}.spring_send", 0.0)
            for name, _, _, _, _ in VOICES}

# ---------------------------------------------------------------- harmony

CHORD_LO = {
    "Em": [40, 47, 55], "C": [36, 48, 55], "G": [43, 50, 59],
    "D": [38, 50, 57], "Am": [45, 52, 57],
}
CHORD_HI = {
    "Em": [59, 64, 67], "C": [60, 64, 67], "G": [62, 67, 71],
    "D": [62, 66, 69], "Am": [60, 64, 69],
}
ROOT = {"Em": 40, "C": 36, "G": 43, "D": 38, "Am": 45}

def ground_and_pads(t0, names, dur_each=8.0, lift=0, ground_on=True,
                    pv=1.0, low_on=True, two=True):
    """The room the melody is carried through: how much of it exists is
    part of the dynamics — one thin layer under the first carrier, the
    full bloom under the choir."""
    for k, ch in enumerate(names):
        tb = t0 + k * dur_each
        if low_on:
            pad.at(tb, ["[" + " ".join(nn(m) for m in CHORD_LO[ch]) + "]"
                        + f":{fmt(dur_each * 0.98)}@{jit(0.42 * pv):.2f}"])
        if two:
            hi = [m + (12 if lift else 0) for m in CHORD_HI[ch]]
            pad2.at(tb + 1.0, ["[" + " ".join(nn(m) for m in hi) + "]"
                               + f":{fmt(dur_each - 1.5)}@{jit(0.36 * pv):.2f}"])
        if ground_on:
            ground.at(tb, [f"{nn(ROOT[ch])}:{fmt(dur_each * 0.96)}@{jit(0.5 * pv):.2f}"])

# ---------------------------------------------------------------- the form

blocks = []
cur = 16.0                      # after a four-bar breath of Em
for g in range(6):
    blocks.append(cur)
    cur += STATEMENT + HANDOFF
duet_t = cur
coda_t = duet_t + STATEMENT
end_t = coda_t + 12.0

# the intro: the room, empty, one chord — then the first voice
ground_and_pads(0.0, ["Em", "Em"], dur_each=8.0, pv=0.6, two=False)
a_wow.ramp(0.0, 0.08, 12.0, "exp")

# grow the melody through its carriers, remembering each generation
generations = [MELODY_0]
for g in range(1, 6):
    fp = VOICES[g][4] or (lambda m: m)
    generations.append(carry(generations[-1], fp))

def emit_statement(track, mel, t0, shift, vel, lyric=False):
    toks, curb = [], 0.0
    for b, m, d in mel:
        if b < 0:
            continue
        if b > curb + 1e-9:
            toks.append(f"R:{fmt(b - curb)}")
            curb = b
        elif b < curb - 1e-9:
            continue
        note = nn(m + shift)
        suffix = "=AA" if lyric else ""
        # breathing time: phrase-final long notes settle a hair late
        tilt = "~+0.02" if d >= 3.0 and b > 0 else ""
        toks.append(f"{note}:{fmt(d)}@{jit(vel):.2f}{tilt}{suffix}")
        curb = b + d
    track.at(t0, toks)

for g in range(6):
    name, _, shift, vel, _ = VOICES[g]
    tr = tracks[name]
    t0 = blocks[g]
    mel = generations[g]
    lyric = name == "choir"

    # pickups inside the previous handoff's ring
    if g > 0:
        pu = [(t0 - 1.5, 59), (t0 - 1.0, 62), (t0 - 0.5, 64)]
        tr.at(pu[0][0], [f"{nn(m + shift)}:0.5@{jit(0.35 + 0.1 * i):.2f}"
                         + ("=AA" if lyric else "")
                         for i, (_, m) in enumerate(pu)])

    emit_statement(tr, mel, t0, shift, vel, lyric)

    # the trace: the previous carrier doubles the phrase closes, low, quiet
    if g > 0:
        pname, _, pshift, _, _ = VOICES[g - 1]
        closes = [n for n in mel if n[2] >= 3.0]
        for b, m, d in closes:
            plyr = "=AA" if pname == "choir" else ""
            tracks[pname].at(t0 + b,
                             [f"{nn(m + pshift - 12)}:{fmt(d)}@0.26{plyr}"])

    # harmony walks under the statement — and the ROOM GROWS with the
    # carrying: one thin layer for the music box, full bloom by the choir
    prog = ["Em", "C", "G", "D"] if g < 3 else ["Em", "C", "Am", "D"]
    room = [dict(pv=0.68, two=False),
            dict(pv=0.78),
            dict(pv=0.88),
            dict(pv=0.8, low_on=False),   # air under the bass carrier
            dict(pv=1.0),
            dict(pv=1.08)][g]
    ground_and_pads(t0, prog, lift=(1 if g >= 4 else 0),
                    ground_on=(g != 3), **room)

    # the handoff: a 7/8 bar — the outgoing voice restrikes its last
    # note and THROWS it into the spring; the ring is the hand
    hb = t0 + STATEMENT
    last_pitch = mel[-1][1] + shift
    tr.at(hb, [f"{nn(last_pitch)}:3@{jit(vel - 0.08):.2f}"
               + ("=AA" if lyric else "")])
    throws[name].set(hb, 0.85)
    throws[name].set(hb + HANDOFF + 2.0, 0.0)
    pad.at(hb, [f"[{nn(40)} {nn(47)} {nn(55)}]:{fmt(HANDOFF)}@0.36"])

    # percussion: enters with the third carrier, breathes, leaves again
    if 2 <= g <= 5:
        strength = {2: 0.5, 3: 0.7, 4: 1.0, 5: 0.45}[g]
        for bar in range(8):
            tb = t0 + bar * 4.0
            kick.at(tb, [f"BD:0.5@{jit(0.5 * strength):.2f}"])
            if strength >= 0.7:
                kick.at(tb + 2.5, [f"BD:0.5@{jit(0.32 * strength):.2f}"])
            harc = 0.8 + 0.4 * (0.5 - abs(bar / 8 - 0.5))
            toks = []
            for e in range(8):
                strong = e % 4 == 0
                lope = "~+0.055" if e % 2 == 1 else ""
                toks.append(
                    f"CH:0.5@{jit((0.3 if strong else 0.16) * strength * harc):.2f}{lope}")
            hats.at(tb, toks)

# the choir needs its lungs (and the carrier panel shaped to sing through)
a_vox.ramp(blocks[5] - 4.0, 0.6, 4.0, "smooth")
a_vox.ramp(blocks[5] + STATEMENT + HANDOFF, 0.0, 4.0, "smooth")

# ------------------------------------------------------------- the duet
# The first voice returns with the melody as it was; the last shape of
# the melody sings above it, an octave up. Different songs now. They fit.

emit_statement(tracks["mb"], MELODY_0, duet_t, 0, 0.56)
emit_statement(glass, generations[5], duet_t, 12, 0.5)
prog = ["Em", "C", "Am", "Em"]
ground_and_pads(duet_t, prog, lift=1, pv=0.72)

# coda: both land on E, two octaves apart, and one bell agrees
tracks["mb"].at(coda_t, [f"E4:6@0.5"])
glass.at(coda_t, [f"E5:6@0.45"])
tracks["gb"].at(coda_t + 2.0, [f"E6:4@0.4"])
ground_and_pads(coda_t, ["Em"], dur_each=10.0, pv=0.66)
throws["mb"].set(coda_t + 4.0, 0.6)
a_spring.ramp(coda_t, 0.22, 8.0)
a_revwet.ramp(duet_t, 0.28, 16.0)
a_wow.ramp(coda_t + 4.0, 0.3, 8.0, "exp")
a_volume.ramp(coda_t + 5.0, 0.0, 7.0, "smooth")

# ------------------------------------------------------------ the tempo
# The piece breathes: it leans forward while the carrying is confident,
# settles when the voice takes over, and lets go at the end. A lane, not
# a constant — the first Patina song whose clock is alive.

a_volume.ramp(blocks[0], 0.68, 71.0)
a_volume.ramp(blocks[2], 0.76, 71.0)
a_volume.ramp(blocks[4], 0.82, 36.0)
a_volume.set(duet_t, 0.66)
a_bpm.ramp(blocks[2], 100.0, 16.0, "smooth")
a_bpm.ramp(blocks[4], 104.0, 12.0, "smooth")
a_bpm.ramp(blocks[5] - 4.0, 94.0, 8.0, "smooth")
a_bpm.ramp(duet_t - 2.0, 90.0, 4.0, "smooth")
a_bpm.ramp(duet_t + 16.0, 78.0, float(STATEMENT - 16.0 + 8.0), "smooth")

# ---------------------------------------------------------------- emit

HEADER = f"""# The Carrying — E minor, 96 bpm (breathing 96..104..78: the tempo is
# a lane). Generated by scripts/the_carrying.py, seed {SEED}. Mine.
#
# One melody is carried through six voices — music box, triangle,
# story lead, bass, bell, and the vocoder choir singing "ah". Each
# carrier forgets one note, sometimes mis-remembers a pitch (the
# mistake becomes the new truth), reaches the peak one scale step
# higher than it received it, and adds one ornament in its own accent.
# Every handoff is a bar of 7/8 — the caught breath — with the
# outgoing voice's last note thrown into the spring tank, still
# ringing while the next voice enters on its pickup. Past carriers
# return to double the phrase closes an octave down, quietly.
# At the end the first voice sings the original against what the
# melody became: different songs now, and they harmonize.
#
# The desk breathes with it: per-voice strips and pans, spring-send
# throws at every handoff, pads shimmering in a chorus the drums
# never touch (chorus_mode 2, chorus_mix 0, chorus_send on the pads),
# swung hats, a gently ducked ground. Sections in beats:
#   0      the room, one chord
#   16     mb: the melody as written
#   51.5   nl: minus one note, plus an appoggiatura
#   87     sl: a passing run; percussion breathes in
#   122.5  bs: the bass carries it low, harmony holds its breath
#   158    gb: the bell splashes the peak — now a step higher again
#   193.5  choir: "ah" — it refuses to let go of the notes
#   229    the duet: first voice vs. final shape, octave apart
#   261    coda: both on E, a bell agrees, the springs keep the rest
#
# Regenerate: python3 scripts/the_carrying.py

bpm 96
gate 0.92

automate chorus_mode
2
automate chorus_mix
0
automate chorus_depth
0.35
automate chorus_rate
0.4
automate reverb_decay
0.72
automate tape_flutter
0.04
automate tape_age
0.28
automate tape_drive
0.22
automate bd_level
0.6
automate bd_tune
0.3
automate bd_decay
0.5
automate bd_attack
0.15
automate hh_level
0.5
automate hh_metal
0.35
automate sd_level
0
automate rs_level
0
automate cp_level
0
automate vox_mode
1
automate vox_intonation
0.04
automate vox_breath
0.3
automate waveform
2
automate detune
6
automate attack
0.05
automate release
0.4
automate cutoff
2000
automate sustain
0.85
"""

parts = [HEADER]
for a in (a_bpm, a_volume, a_wow, a_spring, a_revwet, a_vox):
    parts.append(a.text())
for name, _, _, _, _ in VOICES:
    parts.append(tracks[name].text())
parts.append(glass.text())
for t in (pad, pad2, ground, kick, hats):
    parts.append(t.text())
for a in throws.values():
    parts.append(a.text())

out = "\n\n".join(parts) + "\n"
repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
path = os.path.join(repo, "songs", "the-carrying.song")
with open(path, "w") as f:
    f.write(out)
print(f"wrote {path}: {len(out.splitlines())} lines, ~{end_t:.0f} beats")
