#!/usr/bin/env python3
# Already There — subtraction first.
#
# The other pieces built. This one begins COMPLETE: downbeat one is
# everything at once — arpeggio, canon and its echo, a counter-line, a
# pulse, two pads, a sine ground, tape hiss, light fuzz glue — and
# buried inside the wall, already playing at bar zero, a slow melody
# nobody can hear yet. Nothing is ever added after the first beat.
#
# The composition is deletion. Every few bars, accelerating, one thing
# is removed: the arpeggio loses its offbeats, the echo goes, the pulse
# loses a note, a layer is deleted from below (its high-pass rises until
# it isn't there), voices leave — and every removal RINGS: the deleted
# part's last note is thrown into the spring tank, so subtraction has a
# sound. The room grows as the furniture leaves (reverb widens with
# every deletion), the tape wow rises as the mass empties (memory warps
# once there is less of it), the hiss is cut in one instant — the most
# audible deletion of all — and the melody is NEVER played louder: the
# desk uncovers it (gain, filter, chorus bloom) while everything around
# it is carved away. By the end, the melody itself is subtracted, bar by
# bar, down to its kernel: a falling third, F# to D, the call-home
# interval. The piece's only borrowed note — one Bb, in one Gm chord —
# is spent on the final cadence, and the melody lands on the tonic for
# the first time in the whole piece as the last thing that happens.
#
# It was already there. You just couldn't hear it until enough was gone.
#
# No physically-realistic voices: glass, sine, pulse, triangle, wash.
#
# Deterministic: seed 5.  python3 scripts/already_there.py
# writes songs/already-there.song.

import os
import random

SEED = 5
R = random.Random(SEED)

BPM = 92
BAR = 4.0

def bb(bar):
    return bar * BAR

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

# ------------------------------------------------------------ the melody
# Slow, mostly pentatonic, built on the falling third F#->D. It avoids
# the tonic chord's ground the whole piece: phrases land on E, on A, on
# B — near home, never in it — until the final bar. Bars (4 beats each):

MEL_BARS = [
    [(0.0, 66, 2.0), (2.0, 62, 2.0)],                    # the call
    [(0.0, 64, 2.0), (2.0, 59, 2.0)],                    # step down, hang
    [(0.0, 57, 2.0), (2.0, 59, 1.0), (3.0, 62, 1.0)],    # gather
    [(0.0, 64, 4.0)],                                    # rest on 2, wistful
    [(0.0, 66, 2.0), (2.0, 62, 2.0)],                    # the call again
    [(0.0, 67, 2.0), (2.0, 64, 2.0)],                    # the call, up a step
    [(0.0, 66, 1.0), (1.0, 64, 1.0), (2.0, 62, 1.0), (3.0, 59, 1.0)],
    [(0.0, 57, 4.0)],                                    # land on the 5th
]

# ------------------------------------------------------------- the layers
# Everything derives from the melody's pitch world: D A B F# E G.

ARP16 = [62, 69, 71, 66]                 # D4 A4 B4 F#4, rolling 16ths
CANON8 = [66, 62, 64, 59, 57, 59, 62, 64]  # the melody, folded to 8ths
COUNTER8 = [50, 54, 57, 54, 59, 57, 54, 52]  # D3 F#3 A3 ... E3, mid wheel
PULSE_DYAD = [50, 57]                    # D3 A3

CHORDS = [  # two bars each, rotating: D G Bm A — home only implied
    ("D",  [38, 45, 54], [66, 69, 73], 26),
    ("G",  [43, 50, 59], [67, 71, 74], 31),
    ("Bm", [47, 54, 62], [66, 71, 74], 35),
    ("A",  [45, 52, 61], [64, 69, 73], 33),
]
GM_LO, GM_HI = [43, 50, 58], [58, 62, 67]   # the one Bb, saved for the end

# ---------------------------------------------------------------- tracks

arp    = Track("track arp patch=arpwash vel=0.4 gain=0.75 pan=0.3 spring_send=0")
canon  = Track("track canon patch=quartz vel=0.45 gain=0.8 pan=-0.25")
echo   = Track("track echo patch=glasshigh vel=0.35 gain=0.55 pan=0.45")
count  = Track("track count patch=sunlead vel=0.4 gain=0.7 pan=-0.45")
pulse  = Track("track pulse patch=pulsechord vel=0.32 gain=0.8 pan=0.15")
pad1   = Track("track pad1 patch=dreampad vel=0.42 gain=0.9")
pad2   = Track("track pad2 patch=softpad vel=0.36 gain=0.8 pan=0.25 chorus_send=0.3")
ground = Track("track ground patch=deepsine vel=0.5 gain=0.9")
mel    = Track("track mel patch=nostalgia-lead vel=0.55 gain=0.5 chorus_send=0.15")

a_bpm    = Auto("bpm", BPM)
a_volume = Auto("volume", 0.78)
a_rev    = Auto("reverb_wet", 0.12)
a_dec    = Auto("reverb_decay", 0.6)
a_spring = Auto("spring", 0.05)
a_wow    = Auto("tape_wow", 0.04)
a_fuzz   = Auto("fuzz", 0.1)
a_noise  = Auto("noise", 0.08)
a_mgain  = Auto("mel.gain", 0.5)
a_mcut   = Auto("mel.cutoff", 1100)
a_mcho   = Auto("mel.chorus_send", 0.15)
a_arp_hp = Auto("arp.hpf", 16)

springs = {name: Auto(f"{name}.spring_send", 0.0)
           for name in ("arp", "canon", "echo", "count", "pulse",
                        "pad2", "ground")}

def ring(name, beat, level=0.8, hold=6.0):
    """The sound of removal: the deleted part's tail thrown to the springs."""
    springs[name].set(beat, level)
    springs[name].set(beat + hold, 0.0)

def widen(beat):
    """Every deletion makes the room larger around what remains."""
    a_rev.ramp(beat, min(0.36, 0.12 + widen.k * 0.02), 4.0)
    widen.k += 1
widen.k = 1

# ------------------------------------------------------ deletion schedule
# (bar, what) — accelerating. Nothing is ever added.

E_ARP_HALF, E_ECHO, E_PULSE_A, E_ARP_HPF = 8, 14, 19, 24
E_COUNT, E_PAD2, E_ARP, E_PULSE_Q = 29, 33, 37, 40
E_NOISE, E_CANON, E_GROUND, E_PULSE = 43, 46, 49, 52
E_MEL = [56, 60, 64, 68]     # the melody subtracts itself
CADENCE = 73                 # G | Gm(the Bb) | D — first real home
LAST = 77
END = 81

# ---------------------------------------------------------------- layers

# the arpeggio: 16ths, then 8ths (offbeats deleted), thinned from below,
# then gone
for b in range(0, E_ARP):
    toks = []
    for k in range(16):
        m = ARP16[k % 4]
        toks.append(f"{nn(m)}:0.25@{jit(0.4 - 0.06 * (k % 2)):.2f}")
    arp.at(bb(b), toks)
for b in range(E_ARP_HALF, E_ARP):
    pass  # (halving handled below by re-looping; see next block)
arp.lines = arp.lines[:E_ARP_HALF]   # keep bars 0..7 as 16ths
for b in range(E_ARP_HALF, E_ARP):
    toks = [f"{nn(ARP16[k % 4])}:0.5@{jit(0.36):.2f}" for k in range(8)]
    arp.at(bb(b), toks)
ring("arp", bb(E_ARP) - 0.5)
a_arp_hp.ramp(bb(E_ARP_HPF), 900, 8 * BAR, "exp")   # deleted from below
widen(bb(E_ARP_HALF)); widen(bb(E_ARP_HPF)); widen(bb(E_ARP))

# the canon and its echo (echo two beats behind, an octave up)
for b in range(0, E_CANON):
    toks = [f"{nn(CANON8[k])}:0.5@{jit(0.45):.2f}" for k in range(8)]
    canon.at(bb(b), toks)
ring("canon", bb(E_CANON) - 0.5)
widen(bb(E_CANON))
for b in range(0, E_ECHO):
    toks = [f"{nn(CANON8[k] + 12)}:0.5@{jit(0.33):.2f}" for k in range(8)]
    echo.at(bb(b) + 2.0, toks)
ring("echo", bb(E_ECHO) + 1.5)
widen(bb(E_ECHO))

# the counter-line
for b in range(0, E_COUNT):
    toks = [f"{nn(COUNTER8[k])}:0.5@{jit(0.4):.2f}" for k in range(8)]
    count.at(bb(b), toks)
ring("count", bb(E_COUNT) - 0.5)
widen(bb(E_COUNT))

# the pulse: dyad 8ths -> loses D -> quarters -> gone
for b in range(0, E_PULSE):
    if b < E_PULSE_A:
        tok = "[" + " ".join(nn(m) for m in PULSE_DYAD) + "]"
        toks = [f"{tok}:0.5@{jit(0.3):.2f}" for _ in range(8)]
    elif b < E_PULSE_Q:
        toks = [f"{nn(57)}:0.5@{jit(0.28):.2f}" for _ in range(8)]
    else:
        toks = [f"{nn(57)}:1@{jit(0.26):.2f}" for _ in range(4)]
    pulse.at(bb(b), toks)
ring("pulse", bb(E_PULSE) - 1.0)
widen(bb(E_PULSE_A)); widen(bb(E_PULSE_Q)); widen(bb(E_PULSE))

# the pads: chords rotate two bars each; pad2 is deleted whole, pad1
# stays to the end (someone has to hold the room)
for b in range(0, CADENCE, 2):
    _, lo, hi, _ = CHORDS[(b // 2) % 4]
    pad1.at(bb(b), ["[" + " ".join(nn(m) for m in lo) + "]"
                    + f":{fmt(2 * BAR * 0.98)}@{jit(0.42):.2f}"])
    if b < E_PAD2:
        pad2.at(bb(b) + 1.0, ["[" + " ".join(nn(m) for m in hi) + "]"
                              + f":{fmt(2 * BAR - 2.0)}@{jit(0.36):.2f}"])
ring("pad2", bb(E_PAD2) - 2.0, hold=10.0)
widen(bb(E_PAD2))

# the ground
for b in range(0, E_GROUND, 2):
    root = CHORDS[(b // 2) % 4][3]
    ground.at(bb(b), [f"{nn(root)}:{fmt(2 * BAR * 0.96)}@{jit(0.5):.2f}"])
ring("ground", bb(E_GROUND) - 1.0, hold=8.0)
widen(bb(E_GROUND))

# the hiss stops mid-phrase: the most audible deletion of all
a_noise.set(bb(E_NOISE), 0.0)
widen(bb(E_NOISE))
# the old-recording glue dissolves as the wall comes down
a_fuzz.ramp(bb(E_COUNT), 0.0, 8 * BAR)

# --------------------------------------------------------- the melody
# It plays from bar zero, buried. It is never played louder — the desk
# uncovers it while the world is carved away. Then it subtracts itself.

def mel_bar_tokens(bar_idx, vel=0.55):
    toks, cur = [], 0.0
    for o, m, d in MEL_BARS[bar_idx]:
        if o > cur + 1e-9:
            toks.append(f"R:{fmt(o - cur)}")
            cur = o
        toks.append(f"{nn(m)}:{fmt(d)}@{jit(vel):.2f}")
        cur = o + d
    return toks

removed = set()
for b in range(0, CADENCE - 1):   # bar 72 stays empty: one breath
    if b == E_MEL[0]:
        removed |= {2, 3}
    if b == E_MEL[1]:
        removed |= {5, 6}
    if b == E_MEL[2]:
        removed |= {1, 7}
    if b == E_MEL[3]:
        removed |= {4}
    idx = b % 8
    if idx in removed:
        continue
    if b >= E_MEL[3] and idx != 0:
        continue
    mel.at(bb(b), mel_bar_tokens(idx))

a_mgain.ramp(bb(E_ARP_HALF), 1.0, (E_MEL[0] - E_ARP_HALF) * BAR)
a_mcut.ramp(bb(E_ARP_HALF), 3000, (E_MEL[0] - E_ARP_HALF) * BAR, "exp")
a_mcho.ramp(bb(E_PAD2), 0.7, 20 * BAR)

# ------------------------------------------------------------ the cadence
# G | Gm — the piece's only Bb — | D: the first true home, at the last
# possible moment, under the last falling third.

pad1.at(bb(CADENCE), ["[" + " ".join(nn(m) for m in CHORDS[1][1]) + "]"
                      + f":{fmt(2 * BAR)}@0.4"])
pad1.at(bb(CADENCE + 2), ["[" + " ".join(nn(m) for m in GM_LO) + "]"
                          + f":{fmt(2 * BAR)}@0.38"])
pad2.at(bb(CADENCE + 2) + 0.5, ["[" + " ".join(nn(m) for m in GM_HI) + "]"
                                + f":{fmt(2 * BAR - 1.0)}@0.3"])
pad1.at(bb(LAST), [f"[{nn(38)} {nn(45)} {nn(54)} {nn(62)}]:{fmt(4 * BAR)}@0.4"])

mel.at(bb(CADENCE), mel_bar_tokens(0, vel=0.52))
mel.at(bb(CADENCE + 2), mel_bar_tokens(0, vel=0.5))
# the landing: F# held, then D held long — on the tonic chord, finally
mel.at(bb(LAST), [f"F#4:3@0.52", f"D4:{fmt(4 * BAR - 3)}@0.5~+0.03"])
springs["pad2"].set(bb(LAST) + 3.0, 0.5)
springs["pad2"].set(bb(LAST) + 10.0, 0.0)

# ------------------------------------------------------------- the room
# grows as it empties; the tape warps once there is less holding it

a_dec.ramp(bb(E_PAD2), 0.85, 30 * BAR)
a_wow.ramp(bb(E_ARP), 0.14, 20 * BAR, "exp")
a_wow.ramp(bb(E_MEL[0]), 0.26, 20 * BAR, "exp")
a_spring.ramp(bb(E_PULSE), 0.16, 16 * BAR)
a_volume.ramp(bb(E_MEL[0]), 0.7, 12 * BAR)
a_volume.ramp(bb(LAST) + 6.0, 0.0, 10.0, "smooth")
a_bpm.ramp(bb(CADENCE), 76.0, (END - CADENCE) * BAR, "smooth")

# ---------------------------------------------------------------- emit

HEADER = f"""# Already There — D major leaning everywhere but home, 92 bpm easing
# to 76. Generated by scripts/already_there.py, seed {SEED}.
#
# Subtraction first: downbeat one is EVERYTHING — arpeggio, canon and
# echo, counter-line, pulse, two pads, sine ground, tape hiss, fuzz
# glue — and buried in the wall, already playing, a slow melody built
# on the falling third F#->D. Nothing is ever added after the first
# beat. The composition is deletion, accelerating: offbeats deleted,
# the echo deleted, a note deleted from the pulse, the arpeggio
# deleted from below (its high-pass rises until it isn't there),
# voices deleted — every removal thrown to the springs so subtraction
# has a sound; the reverb widens with each loss; the hiss is cut in
# one instant; the melody is never played louder, only uncovered
# (gain, filter, chorus bloom on its strip). Then the melody
# subtracts itself, bar by bar, to its kernel — the call — and the
# piece's only Bb (one Gm chord) buys the final cadence: the melody
# lands on the tonic for the first time as the last thing that
# happens. It was already there.
#
# Regenerate: python3 scripts/already_there.py

bpm {BPM}
gate 0.9

automate chorus_mode
2
automate chorus_mix
0
automate chorus_depth
0.32
automate chorus_rate
0.3
automate tape_flutter
0.05
automate tape_age
0.34
automate tape_drive
0.24
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
for a in (a_bpm, a_volume, a_rev, a_dec, a_spring, a_wow, a_fuzz, a_noise):
    parts.append(a.text())
for t in (arp, canon, echo, count, pulse, pad1, pad2, ground, mel):
    parts.append(t.text())
for a in springs.values():
    parts.append(a.text())
parts.append(a_mgain.text())
parts.append(a_mcut.text())
parts.append(a_mcho.text())
parts.append(a_arp_hp.text())

out = "\n\n".join(parts) + "\n"
repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
path = os.path.join(repo, "songs", "already-there.song")
with open(path, "w") as f:
    f.write(out)
print(f"wrote {path}: {len(out.splitlines())} lines, {END} bars")
