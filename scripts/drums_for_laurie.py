#!/usr/bin/env python3
# Drums, For Laurie — the score is a program.
#
# Laurie Spiegel's "Drums" (GROOVE system, Bell Labs, ~1975) wasn't a
# recording of someone drumming: it was a PROCESS — pitched drum voices
# driven by algorithmic pattern logic, steered by hand-played control
# curves. The machine held the loom; the human pulled the threads.
#
# This script is that idea aimed at Patina's 909 board, and then let off
# the leash: the same process that weaves her patient, pitched lattice is
# allowed to accelerate until it becomes breakcore. Aphex Twin's
# tenderness lives in the middle of it; Venetian Snares' 7/8 violence is
# where the process ends up when nobody stops it.
#
# One seed, one theme, every layer derived:
#   - a 7-pulse bar (7/8 at 190 bpm), the whole piece on that clock
#   - a 10-note theme row stated by everything that speaks: the KICK
#     sings it first (bd_tune played per-note — Spiegel's pitched drums),
#     the acid bass chews it, the lead confesses it, the bells remember it
#   - a rimshot Euclidean clock E(5,14), rotated one step per bar, runs
#     from bar 4 to the final bar — the thread the loom never drops
#   - drill sections are generated from a per-bar violence curve:
#     kick syncopation, ghost-note fields, ratcheted snare rolls with
#     sd_tune sweeps, hat density as Euclidean k, blast bars at the peak
#
# Deterministic: same seed, same song.  python3 scripts/drums_for_laurie.py
# writes songs/drums-for-laurie.song.

import os
import random

SEED = 1975  # the year "Drums" was woven
R = random.Random(SEED)

BPM = 190
PULSE = 0.5          # one 7/8 pulse, in beats
BAR = 7 * PULSE      # 3.5 beats
N16 = 14             # 16th steps per bar
S16 = 0.25           # one 16th, in beats

# ---------------------------------------------------------------- helpers

def fmt(x):
    s = f"{x:.6f}".rstrip("0").rstrip(".")
    return s if s else "0"

NAMES = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"]

def nn(midi):
    return f"{NAMES[midi % 12]}{midi // 12 - 1}"

def euclid(k, n, rot=0):
    """k onsets in n steps, maximally even (Bresenham form), rotated."""
    return [((i + rot) * k) % n < k for i in range(n)]

def jit(v, amt=0.05):
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
    """One automation lane: absolute-beat sets and ramps, emitted sorted."""
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

# ---------------------------------------------------------------- theme

# The row: pulse offsets within a 2-bar phrase, MIDI pitch.
# A3 C4 E4 D4 B3 | G3 A3 C4 B3 A3 — rises to E, falls home, sighs.
THEME = [(0, 57), (2, 60), (3, 64), (5, 62), (6, 59),
         (7, 55), (9, 57), (10, 60), (11, 59), (12, 57)]

def kick_tune(midi):
    """Map the theme's register onto the bd_tune knob: contour, not pitch."""
    return round(0.16 + (midi - 55) / 9.0 * 0.52, 3)

AMIN = [57, 59, 60, 62, 64, 65, 67]  # one octave of A natural minor

def scale_near(midi, step):
    """Walk `step` scale degrees from the nearest scale tone to midi."""
    octave, best = 0, None
    for o in (-12, 0, 12):
        for s in AMIN:
            c = s + o
            if best is None or abs(c - midi) < abs(best - midi):
                best, octave = c, o
    idx = AMIN.index(best - octave)
    idx2 = idx + step
    oshift = octave
    while idx2 < 0:
        idx2 += 7; oshift -= 12
    while idx2 >= 7:
        idx2 -= 7; oshift += 12
    return AMIN[idx2] + oshift

# ---------------------------------------------------------------- sections
#   bars      section
#   0-20      LOOM        the kick sings the theme alone; the clock starts
#   20-48     ACCUMULATE  one thread every four bars; the process thickens
#   48-88     TORRENT     the loom becomes a drill; acid takes the theme
#   88-120    SPEECH      half density; the lead confesses over chords
#   120-168   FRENZY      violence to 1.0; blast bars; the process unbound
#   168-204   AFTERIMAGE  half-speed theme; the clock outlives the song

A0, B0, C0, D0, E0, F0, END = 0, 20, 48, 88, 120, 168, 204

def bb(bar_idx):
    return bar_idx * BAR

# ---------------------------------------------------------------- tracks

kick  = Track("track kick kit=909 vel=0.9 len=0.25")
snare = Track("track snare kit=909 vel=0.8 len=0.25")
hats  = Track("track hats kit=909 vel=0.5 len=0.25")
clock = Track("track clock kit=909 vel=0.3 len=0.25")
bass  = Track("track bass patch=acidline vel=0.8 len=0.25")
lead  = Track("track lead patch=nostalgia-lead vel=0.8 len=0.5")
pad   = Track("track pad patch=dreampad vel=0.45 len=1")
bell  = Track("track bell patch=glintbell vel=0.6 len=0.5")
drone = Track("track drone patch=drone vel=0.5 len=1")

a_bd_tune  = Auto("bd_tune", kick_tune(57))
a_bd_decay = Auto("bd_decay", 0.8)
a_bd_att   = Auto("bd_attack", 0.35)
a_bd_drive = Auto("bd_drive", 0.12)
a_sd_tune  = Auto("sd_tune", 0.42)
a_sd_snap  = Auto("sd_snappy", 0.55)
a_oh_dec   = Auto("oh_decay", 0.35)
a_drum_drv = Auto("dr_drive", 0.05)
a_fuzz     = Auto("fuzz", 0.0)
a_wow      = Auto("tape_wow", 0.5)
a_spring   = Auto("spring", 0.08)
# The master volume is played, not set: Spiegel's hand-drawn control
# line over the whole form — the loudness arc IS one of the voices.
a_volume   = Auto("volume", 0.6)
a_bend     = Auto("bend", 0.0)
a_bass_cut = Auto("bass.cutoff", 340)
a_bass_res = Auto("bass.resonance", 1.15)

SD_BASE = 0.42

# ------------------------------------------------ the kick sings (A, B, F)

def kick_theme_bars(first_bar, n_bars, vel=0.8, half_speed=False,
                    heartbeat=False):
    """The theme on the kick drum, bd_tune set note-by-note."""
    stretch = 2 if half_speed else 1
    phrase_pulses = 14 * stretch
    for bar in range(first_bar, first_bar + n_bars):
        t0 = bb(bar)
        toks = []
        cur = 0.0
        phrase_pulse0 = ((bar - first_bar) * 7) % phrase_pulses
        hits = []
        for pulse, midi in THEME:
            p = pulse * stretch - phrase_pulse0
            if 0 <= p < 7:
                hits.append((p * PULSE, midi))
        if heartbeat:
            hits.append((3.5 * PULSE, None))  # offbeat heartbeat thud
            hits.sort()
        for at, midi in hits:
            if at > cur + 1e-9:
                toks.append(f"R:{fmt(at - cur)}")
                cur = at
            if midi is not None:
                a_bd_tune.set(t0 + at, kick_tune(midi))
                v = jit(vel + (0.1 if at == 0.0 else 0.0), 0.03)
            else:
                a_bd_tune.set(t0 + at, 0.18)
                v = jit(vel - 0.2, 0.03)
            toks.append(f"BD:0.25@{v:.2f}")
            cur += 0.25
        kick.bar(t0, toks)

# ------------------------------------------------ the clock (whole piece)

def clock_bars(first_bar, n_bars, vel=0.28, fade_to=None):
    for bar in range(first_bar, first_bar + n_bars):
        t0 = bb(bar)
        frac = (bar - first_bar) / max(1, n_bars - 1)
        v = vel if fade_to is None else vel + (fade_to - vel) * frac
        pat = euclid(5, N16, rot=bar % N16)
        toks = []
        cur = 0.0
        for i in range(N16):
            if pat[i]:
                at = i * S16
                if at > cur + 1e-9:
                    toks.append(f"R:{fmt(at - cur)}")
                    cur = at
                toks.append(f"RS:0.25@{jit(v, 0.04):.2f}")
                cur += 0.25
        clock.bar(t0, toks)

# ------------------------------------------------ drill machinery (C, E)

def steps_to_tokens(track, t0, events):
    """events: sorted list of (step_beats, token_dur, name, vel)."""
    toks = []
    cur = 0.0
    for at, dur, name, vel in events:
        if at > cur + 1e-9:
            toks.append(f"R:{fmt(at - cur)}")
            cur = at
        toks.append(f"{name}:{fmt(dur)}@{vel:.2f}")
        cur += dur
    track.bar(t0, toks)

def roll(t0, at, span, n, base_vel, tune_from, tune_to):
    """A ratcheted snare roll with an sd_tune sweep, restored after."""
    d = span / n
    ev = []
    for i in range(n):
        v = base_vel * (0.42 + 0.58 * (i + 1) / n)
        ev.append((at + i * d, d, "SD", min(1.0, jit(v, 0.03))))
    a_sd_tune.set(t0 + at, tune_from)
    a_sd_tune.ramp(t0 + at, tune_to, span, "exp")
    a_sd_tune.set(t0 + at + span, SD_BASE)
    return ev

def drill_bar(bar, v, root=45, blast=False, breath=False, wash=False,
              acid=True):
    """One generated bar of the drill: v is the violence curve, 0..1."""
    t0 = bb(bar)

    if breath:  # the loom inhales: clock keeps turning, one kick, nothing else
        kick.bar(t0, [f"BD:0.25@{jit(0.85):.2f}"])
        return

    if blast:   # a Venetian bar: the snare becomes a surface
        n = R.choice([21, 28])
        up = R.random() < 0.6
        ev = roll(t0, 0.0, BAR, n, 0.92,
                  0.2 if up else 0.85, 0.9 if up else 0.18)
        steps_to_tokens(snare, t0, ev)
        kick.bar(t0, [f"BD:0.25@0.95", f"R:{fmt(BAR - 0.5)}", f"BD:0.25@0.9"])
        return

    # --- kick: anchor plus violence-weighted syncopation
    ksteps = {0}
    for cand, p in ((6, 0.85), (10, 0.7), (3, 0.25 + 0.5 * v),
                    (8, 0.2 + 0.4 * v), (12, 0.15 + 0.45 * v),
                    (5, 0.3 * v), (11, 0.3 * v)):
        if R.random() < p * (0.55 + 0.45 * v):
            ksteps.add(cand)
    kev = []
    for s in sorted(ksteps):
        if R.random() < 0.12 * v:  # stutter: the tape sticks
            for i in range(3):
                kev.append((s * S16 + i * S16 / 3, S16 / 3, "BD",
                            jit(0.8 - 0.15 * i)))
        else:
            kev.append((s * S16, S16, "BD", jit(0.92 if s == 0 else 0.8)))
    steps_to_tokens(kick, t0, kev)

    # --- snare: backbeats, displacement, ghost field, end-of-bar ratchets
    sev = []
    back = [4, 10]
    if v > 0.55 and R.random() < 0.35:
        back[R.randrange(2)] += R.choice([-1, 1])
    occupied = set()
    for s in back:
        sev.append((s * S16, S16, "SD", jit(0.85)))
        occupied.add(s)
    for s in range(N16):
        if s in occupied or s in ksteps:
            continue
        if R.random() < 0.06 + 0.2 * v:
            sev.append((s * S16, S16, "SD", jit(0.14, 0.05)))
            occupied.add(s)
    if R.random() < 0.25 + 0.6 * v:
        span_steps = R.choice([2, 3]) if v < 0.6 else R.choice([2, 3, 4])
        start = N16 - span_steps
        sev = [e for e in sev if e[0] < start * S16 - 1e-9]
        n = R.choice([3, 4, 6]) if v < 0.6 else R.choice([6, 8, 10])
        up = R.random() < 0.7
        sev += roll(t0, start * S16, span_steps * S16, n, 0.55 + 0.4 * v,
                    0.25 if up else 0.75, 0.8 if up else 0.25)
    sev.sort(key=lambda e: e[0])
    steps_to_tokens(snare, t0, sev)

    # --- hats: Euclidean density rides the violence; OH opens the seams
    k = min(12, 5 + int(round(6 * v)))
    pat = euclid(k, N16, rot=(bar * 3) % N16)
    hev = []
    for i in range(N16):
        if wash and i % 2 == 0:
            hev.append((i * S16, S16, "OH", jit(0.5)))
        elif pat[i]:
            strong = i % 4 == 0
            hev.append((i * S16, S16, "CH", jit(0.5 if strong else 0.3)))
    if not wash and R.random() < 0.3 + 0.3 * v:
        oh_at = R.choice([7, 13])
        hev = [e for e in hev if abs(e[0] - oh_at * S16) > 1e-9]
        hev.append((oh_at * S16, S16, "OH", jit(0.55)))
        hev.sort(key=lambda e: e[0])
    steps_to_tokens(hats, t0, hev)

    # --- clap: seals every fourth bar
    if bar % 4 == 3:
        snare.lines.append(f">{fmt(t0 + 13 * S16)} CP:0.25@{jit(0.7):.2f}")

    # --- acid: the theme chewed into 16ths around the root
    if acid:
        theme_at = {p % 7: m for p, m in THEME}
        aev = []
        for s in range(N16):
            pulse, half = divmod(s, 2)
            r = R.random()
            if half == 0 and pulse in theme_at:
                m = theme_at[pulse] - 12 + (root - 45)
                aev.append((s, m, 0.95))
            elif r < 0.28 + 0.25 * v:
                m = root - 12 if r < 0.18 else root
                if R.random() < 0.15:
                    m = scale_near(root, R.choice([-1, 1, 2]))
                aev.append((s, m, 0.5 if half else 0.62))
        toks = []
        cur = 0.0
        for s, m, vel in aev:
            at = s * S16
            if at > cur + 1e-9:
                toks.append(f"R:{fmt(at - cur)}")
                cur = at
            toks.append(f"{nn(m)}:0.25@{jit(vel, 0.04):.2f}")
            cur += 0.25
        bass.bar(t0, toks)

# ---------------------------------------------------------------- A: LOOM

kick_theme_bars(A0, 20, vel=0.6)
clock_bars(4, 16, vel=0.36)         # the thread, alone: let it be heard
clock_bars(20, END - 8 - 20)        # ...then woven under, to bar 196

a_wow.ramp(0.0, 0.12, 8 * BAR, "exp")
a_wow.set(B0 * BAR, 0.09)
a_volume.ramp(0.0, 0.7, 20 * BAR)
a_volume.ramp(bb(B0), 0.78, 28 * BAR)
a_volume.ramp(bb(E0), 1.0, 42 * BAR)
a_volume.set(bb(F0), 0.62)

# first whispers: ghost snares find the loom in the dark
for bar in range(12, A0 + 28):
    t0 = bb(bar)
    for _ in range(R.choice([1, 1, 2])):
        s = R.randrange(N16)
        snare.lines.append(f">{fmt(t0 + s * S16)} SD:0.25@{jit(0.12, 0.04):.2f}")

# ---------------------------------------------------------- B: ACCUMULATE

kick_theme_bars(B0, 8, vel=0.66)
kick_theme_bars(B0 + 8, 20, vel=0.7, heartbeat=True)

# hat density is the accumulation made audible: k climbs 3 -> 11
for bar in range(B0 - 4, C0):
    t0 = bb(bar)
    k = 3 + max(0, (bar - (B0 - 4))) // 4 * 2
    k = min(11, k)
    pat = euclid(k, N16, rot=(bar * 5) % N16)
    toks = []
    cur = 0.0
    for i in range(N16):
        if pat[i]:
            at = i * S16
            if at > cur + 1e-9:
                toks.append(f"R:{fmt(at - cur)}")
                cur = at
            toks.append(f"CH:0.25@{jit(0.32 if i % 4 else 0.45, 0.05):.2f}")
            cur += 0.25
    hats.bar(t0, toks)

# the drone under the loom
for bar in range(B0, C0, 4):
    drone.bar(bb(bar), [f"{nn(33)}:{fmt(4 * BAR * 0.96)}@0.45"])

# acid murmurs: fragments of the theme's floor, half asleep
for bar in range(36, C0):
    t0 = bb(bar)
    toks = []
    cur = 0.0
    for s in sorted(R.sample([0, 3, 6, 8, 10, 12], R.choice([2, 3]))):
        at = s * S16
        if at > cur + 1e-9:
            toks.append(f"R:{fmt(at - cur)}")
            cur = at
        m = 33 if R.random() < 0.7 else R.choice([36, 40, 45])
        toks.append(f"{nn(m)}:0.25@{jit(0.5, 0.05):.2f}")
        cur += 0.25
    bass.bar(t0, toks)

# backbeat learns to stand: snare enters soft and rises
for bar in range(40, C0):
    t0 = bb(bar)
    v = 0.3 + 0.3 * (bar - 40) / (C0 - 40)
    snare.bar(t0, [f"R:1", f"SD:0.25@{jit(v):.2f}", f"R:{fmt(10 * S16 - 0.25)}",
                   f"SD:0.25@{jit(v + 0.05):.2f}"])
    if bar % 4 == 3 and bar > 42:
        sev = roll(t0, 12 * S16, 2 * S16, 3, 0.5, 0.3, 0.6)
        for at, d, nme, vv in sev:
            snare.lines.append(f">{fmt(t0 + at)} {nme}:{fmt(d)}@{vv:.2f}")

a_drum_drv.ramp(bb(40), 0.18, 8 * BAR)
a_bass_cut.ramp(bb(36), 520, 12 * BAR, "exp")

# the loom tightens for the drill: kick becomes a drum, not a voice
a_bd_decay.ramp(bb(C0 - 1), 0.52, BAR)
a_bd_att.set(bb(C0), 0.55)
a_bd_tune.set(bb(C0), 0.24)
a_sd_snap.set(bb(C0), 0.72)
a_wow.set(bb(C0), 0.06)

# ------------------------------------------------------------- C: TORRENT

for bar in range(C0, D0):
    frac = (bar - C0) / (D0 - C0)
    v = 0.35 + 0.35 * frac
    breath = bar in (C0 + 7, C0 + 23)
    blast = bar == C0 + 31
    wash = bar == C0 + 15
    drill_bar(bar, v, root=45, blast=blast, breath=breath, wash=wash)

# C's seam into SPEECH: one rising ratchet, then the floor drops
t0 = bb(D0 - 1)
sev = roll(t0, 0.0, BAR - 0.5, 10, 0.85, 0.25, 0.88)
steps_to_tokens(snare, t0, sev)
kick.bar(t0, ["BD:0.25@0.95"])

a_bass_cut.ramp(bb(C0), 1700, (D0 - C0 - 2) * BAR, "exp")
a_bass_res.ramp(bb(C0 + 8), 1.5, 24 * BAR)
a_drum_drv.ramp(bb(C0), 0.3, (D0 - C0) * BAR)
a_oh_dec.set(bb(C0 + 15), 0.7)
a_oh_dec.set(bb(C0 + 16), 0.35)

# -------------------------------------------------------------- D: SPEECH

CHORDS = [
    [45, 52, 59, 60],   # Am(add9)  A2 E3 B3 C4
    [41, 48, 52, 57],   # Fmaj7     F2 C3 E3 A3
    [43, 50, 59, 64],   # G6        G2 D3 B3 E4
    [40, 47, 55, 62],   # Em7       E2 B2 G3 D4
]
E7 = [40, 47, 56, 62]   # the piece's only G#: the ache toward home

for cyc in range(4):
    for ci in range(4):
        bar = D0 + cyc * 8 + ci * 2
        chord = CHORDS[ci]
        if ci == 3 and cyc >= 2:
            chord = E7
        pad.bar(bb(bar), ["[" + " ".join(nn(m) for m in chord) + "]"
                          + f":{fmt(2 * BAR * 0.96)}@0.5"])

# drums breathe: skeleton at low violence, no acid, clock carries
for bar in range(D0, E0 - 2):
    t0 = bb(bar)
    kick.bar(t0, [f"BD:0.25@{jit(0.8):.2f}", f"R:{fmt(6 * S16 - 0.25)}",
                  f"BD:0.25@{jit(0.6):.2f}"])
    snare.bar(t0, [f"R:1", f"SD:0.25@{jit(0.62):.2f}",
                   f"R:{fmt(10 * S16 - 5 * S16)}",
                   f"SD:0.25@{jit(0.66):.2f}"])
    pat = euclid(5, N16, rot=(bar * 3) % N16)
    toks = []
    cur = 0.0
    for i in range(N16):
        if pat[i]:
            at = i * S16
            if at > cur + 1e-9:
                toks.append(f"R:{fmt(at - cur)}")
                cur = at
            toks.append(f"CH:0.25@{jit(0.25, 0.04):.2f}")
            cur += 0.25
    hats.bar(t0, toks)
    if bar % 8 == (D0 + 7) % 8:
        sev = roll(t0, 12 * S16, 2 * S16, R.choice([3, 4]), 0.45, 0.3, 0.55)
        for at, d, nme, vv in sev:
            snare.lines.append(f">{fmt(t0 + at)} {nme}:{fmt(d)}@{vv:.2f}")

# the confession: the theme sung plainly, then ornamented, then higher
def sing(first_bar, transpose, vel, ornament):
    t0 = bb(first_bar)
    toks = []
    cur = 0.0
    for i, (pulse, midi) in enumerate(THEME):
        at = pulse * PULSE
        m = midi + transpose
        nxt = THEME[i + 1][0] * PULSE if i + 1 < len(THEME) else 14 * PULSE
        if at > cur + 1e-9:
            toks.append(f"R:{fmt(at - cur)}")
            cur = at
        dur = nxt - at
        if ornament and dur >= 1.0 and R.random() < 0.5:
            grace = scale_near(m, R.choice([-1, 1]))
            toks.append(f"{nn(m)}:{fmt(dur - 0.5)}@{vel:.2f}")
            toks.append(f"{nn(grace)}:0.25@{vel - 0.15:.2f}")
            toks.append(f"{nn(m)}:0.25@{vel - 0.2:.2f}")
        else:
            toks.append(f"{nn(m)}:{fmt(dur)}@{vel:.2f}")
        cur = nxt
    lead.bar(t0, toks)

sing(D0 + 2, 12, 0.62, False)
sing(D0 + 10, 12, 0.68, True)
sing(D0 + 18, 24, 0.6, True)
sing(D0 + 26, 12, 0.72, True)
# over the E7 bars the lead leans on the G#, unresolved until FRENZY
lead.bar(bb(D0 + 30), [f"{nn(68)}:2@0.7", f"{nn(69)}:1@0.6",
                       f"{nn(71)}:{fmt(2 * BAR - 3)}@0.65"])

for at_bar, m in ((D0 + 7, 76), (D0 + 15, 72), (D0 + 23, 79)):
    bell.bar(bb(at_bar) + 12 * S16, [f"{nn(m)}:1@0.55"])

# -------------------------------------------------------------- E: FRENZY

ROOTS = [45, 45, 41, 43,  45, 45, 41, 40,  45, 45, 41, 43]  # 4-bar groups
BLASTS = {E0 + 14, E0 + 22, E0 + 30, E0 + 38, E0 + 42, E0 + 44, E0 + 45}
BREATHS = {E0 + 15, E0 + 31}

for bar in range(E0, F0 - 2):
    g = (bar - E0) // 4
    frac = (bar - E0) / (F0 - 2 - E0)
    v = 0.55 + 0.45 * frac
    drill_bar(bar, min(1.0, v), root=ROOTS[min(g, len(ROOTS) - 1)],
              blast=bar in BLASTS, breath=bar in BREATHS,
              wash=bar in (E0 + 19, E0 + 35))

# collapse: two bars of air before the afterimage
t0 = bb(F0 - 2)
kick.bar(t0, ["BD:0.25@1"])
a_wow.set(t0 + 0.5, 0.3)

# the lead returns possessed: theme an octave up, then the long high cry
sing(E0 + 8, 24, 0.7, True)
sing(E0 + 16, 24, 0.75, True)
lead.bar(bb(E0 + 24), [f"{nn(81)}:{fmt(4 * BAR)}@0.72"])
sing(E0 + 28, 24, 0.78, True)
lead.bar(bb(E0 + 36), [f"{nn(80)}:{fmt(BAR)}@0.75",
                       f"{nn(81)}:{fmt(3 * BAR)}@0.78"])
lead.bar(bb(E0 + 40), [f"{nn(84)}:{fmt(2 * BAR)}@0.75",
                       f"{nn(83)}:1@0.7", f"{nn(81)}:{fmt(2 * BAR - 1)}@0.72"])

a_fuzz.ramp(bb(E0), 0.22, 32 * BAR)
a_fuzz.set(bb(F0 - 2), 0.0)
a_drum_drv.ramp(bb(E0), 0.5, 40 * BAR)
a_drum_drv.set(bb(F0), 0.08)
a_bass_cut.set(bb(E0), 900)
a_bass_cut.ramp(bb(E0), 2600, 44 * BAR, "exp")
a_bass_res.ramp(bb(E0), 1.7, 44 * BAR)
a_bd_drive.ramp(bb(E0), 0.4, 40 * BAR)
a_bd_drive.set(bb(F0), 0.1)
a_oh_dec.set(bb(E0 + 19), 0.75); a_oh_dec.set(bb(E0 + 20), 0.35)
a_oh_dec.set(bb(E0 + 35), 0.75); a_oh_dec.set(bb(E0 + 36), 0.35)

# ---------------------------------------------------------- F: AFTERIMAGE

a_bd_decay.set(bb(F0), 0.88)
a_bd_att.set(bb(F0), 0.3)
a_sd_snap.set(bb(F0), 0.5)

kick_theme_bars(F0, 16, vel=0.46, half_speed=True)
# fragments: the theme loses words, keeps only downbeats
for bar in range(F0 + 16, F0 + 24, 2):
    t0 = bb(bar)
    a_bd_tune.set(t0, kick_tune(57))
    kick.bar(t0, [f"BD:0.25@{jit(0.45, 0.03):.2f}"])

pad.bar(bb(F0), ["[" + " ".join(nn(m) for m in CHORDS[0]) + "]"
                 + f":{fmt(4 * BAR)}@0.44"])
pad.bar(bb(F0 + 8), ["[" + " ".join(nn(m) for m in CHORDS[1]) + "]"
                     + f":{fmt(4 * BAR)}@0.4"])
pad.bar(bb(F0 + 16), ["[" + " ".join(nn(m) for m in CHORDS[0]) + "]"
                      + f":{fmt(8 * BAR)}@0.4"])

for bar in range(F0, F0 + 22, 4):
    drone.bar(bb(bar), [f"{nn(33)}:{fmt(4 * BAR * 0.96)}@0.36"])

# the bells remember three notes of it, then one, then one lower
bell.bar(bb(F0 + 4), [f"{nn(81)}:1@0.5", f"{nn(84)}:1@0.45", f"{nn(83)}:2@0.4"])
bell.bar(bb(F0 + 12), [f"{nn(76)}:1@0.45", f"{nn(74)}:2@0.4"])
bell.bar(bb(F0 + 20), [f"{nn(81)}:2@0.35"])
bell.bar(bb(F0 + 28), [f"{nn(69)}:2@0.3"])

# the clock alone, fading — the process outlives the song
clock_bars(END - 8, 7, vel=0.26, fade_to=0.08)

a_wow.ramp(bb(F0 + 12), 0.55, 20 * BAR, "exp")
a_spring.ramp(bb(F0), 0.35, 16 * BAR)
a_volume.ramp(bb(END - 6), 0.0, 6 * BAR, "smooth")
a_bend.ramp(bb(END - 3), -2.0, 3 * BAR, "smooth")

# ---------------------------------------------------------------- emit

HEADER = f"""# Drums, For Laurie — 190 bpm, 7/8, A minor. Generated, on purpose:
# the score is a program (scripts/drums_for_laurie.py, seed {SEED}), the
# way "Drums" was a program on the GROOVE system at Bell Labs — pitched
# drum voices driven by pattern logic, steered by hand-drawn curves.
#
# Here the process is allowed to accelerate until it becomes breakcore.
# One 10-note theme is the whole piece: the KICK sings it first (bd_tune
# played per-note — the pitched drum as melodic voice), the acid bass
# chews it into 16ths, the lead confesses it over chords, the bells
# remember three notes of it, and a rimshot Euclidean clock E(5,14),
# rotating one step per bar, runs from bar 4 to the last bar without
# ever breaking — the thread the loom never drops.
#
#   bars   0-20    LOOM        the kick alone, tuned like a drum choir
#   bars  20-48    ACCUMULATE  a thread every 4 bars; hats climb E(3..11,14)
#   bars  48-88    TORRENT     the drill: ratchet rolls, sd_tune sweeps
#   bars  88-120   SPEECH      half density; Am9 F G6 Em7 — then E7's G#
#   bars 120-168   FRENZY      violence to 1.0, blast bars, the long cry
#   bars 168-204   AFTERIMAGE  half-speed theme, wow flood, the clock last
#
# Regenerate: python3 scripts/drums_for_laurie.py

bpm {BPM}
gate 0.82

automate reverb_wet
0.16
automate reverb_decay
0.62
automate tape_flutter
0.05
automate tape_drive
0.28
automate tape_age
0.3
automate bd_level
0.92
automate sd_level
0.88
automate hh_level
0.58
automate rs_level
0.7
automate cp_level
0.68
automate hh_metal
0.55
automate sd_decay
0.5
"""

parts = [HEADER]
for a in (a_bd_tune, a_bd_decay, a_bd_att, a_bd_drive, a_sd_tune, a_sd_snap,
          a_oh_dec, a_drum_drv, a_fuzz, a_wow, a_spring, a_volume, a_bend):
    parts.append(a.text())
for t in (kick, snare, hats, clock, bass, drone, pad, bell, lead):
    parts.append(t.text())
parts.append(a_bass_cut.text())
parts.append(a_bass_res.text())

out = "\n\n".join(parts) + "\n"
repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
path = os.path.join(repo, "songs", "drums-for-laurie.song")
with open(path, "w") as f:
    f.write(out)
print(f"wrote {path}: {len(out.splitlines())} lines, "
      f"{END * BAR * 60 / BPM:.0f}s before tail")
