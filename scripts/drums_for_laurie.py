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
# Second weaving. The first draft rolled dice every bar, and uniform
# randomness reads as gray: nothing repeats exactly, so nothing means
# anything. This version composes in PHRASES:
#   - patterns (kick, ghosts, hats, the acid riff) are chosen once per
#     8-bar phrase, repeated as riffs, and mutated only at phrase ends —
#     repetition is what makes the mutations land
#   - every phrase has a dynamic arc (gain crescendos, breath bars,
#     whole cycles with no kick) — loudness moves at three timescales:
#     hit, phrase, section
#   - the snare is punctuation, not weather: ghosts live at fixed low-
#     velocity spots near the backbeat, rolls happen at phrase ends
#     (with reverb THROWS — the send is played, not set), blasts are
#     saved for the frenzy's last quarter
#   - pitch never sits still: the kick whispers the theme root into
#     each drill phrase (bd_tune), snare-tune bases shift per phrase,
#     rim tune and hat tune drift slowly across whole sections
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

def g(x, gain):
    return min(1.0, max(0.06, x * gain))

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
#   0-16      LOOM        the kick sings the theme alone; the clock starts
#   16-40     ACCUMULATE  one thread every four bars; the process thickens
#   40-80     TORRENT     five phrases, each with its own posture
#   80-112    SPEECH      four cycles; the second holds its breath
#   112-160   FRENZY      rebuild, stomp, drill, blasts — then the tear
#   160-192   AFTERIMAGE  half-speed theme; the clock outlives the song

A0, B0, C0, D0, E0, F0, END = 0, 16, 40, 80, 112, 160, 192

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
a_sd_level = Auto("sd_level", 0.6)
a_hh_level = Auto("hh_level", 0.5)
a_hh_tune  = Auto("hh_tune", 0.5)
a_rs_tune  = Auto("rs_tune", 0.42)
a_oh_dec   = Auto("oh_decay", 0.35)
a_drum_drv = Auto("dr_drive", 0.05)
a_fuzz     = Auto("fuzz", 0.0)
a_wow      = Auto("tape_wow", 0.5)
a_spring   = Auto("spring", 0.08)
a_rev_wet  = Auto("reverb_wet", 0.16)
# The master volume is played, not set: Spiegel's hand-drawn control
# line over the whole form — the loudness arc IS one of the voices.
a_volume   = Auto("volume", 0.6)
a_bend     = Auto("bend", 0.0)
a_bass_cut = Auto("bass.cutoff", 340)
a_bass_res = Auto("bass.resonance", 1.15)

def throw(beat, wet=0.4, hold=1.5):
    """A dub throw: the reverb send opens for one gesture, then shuts."""
    a_rev_wet.set(beat, wet)
    a_rev_wet.set(beat + hold, 0.16)

# ------------------------------------------------ the kick sings (A, B, F)

def kick_theme_bars(first_bar, n_bars, vel=0.6, half_speed=False,
                    heartbeat=False, vary=False):
    """The theme on the kick drum, bd_tune set note-by-note. Velocity
    follows the contour the way a hand would; every 4 bars the statement
    changes posture (full / sparse / full / lifted) when vary is on."""
    stretch = 2 if half_speed else 1
    phrase_pulses = 14 * stretch
    for bar in range(first_bar, first_bar + n_bars):
        t0 = bb(bar)
        mode = "full"
        if vary:
            mode = ("full", "sparse", "full", "lift")[((bar - first_bar) // 4) % 4]
        toks = []
        cur = 0.0
        phrase_pulse0 = ((bar - first_bar) * 7) % phrase_pulses
        hits = []
        for pulse, midi in THEME:
            p = pulse * stretch - phrase_pulse0
            if 0 <= p < 7:
                hits.append((p * PULSE, midi))
        if heartbeat:
            hits.append((3.5 * PULSE, None))
            hits.sort()
        for at, midi in hits:
            if mode == "sparse" and at > 0.0 and midi is not None \
                    and R.random() < 0.35:
                continue
            if at > cur + 1e-9:
                toks.append(f"R:{fmt(at - cur)}")
                cur = at
            if midi is not None:
                tune = kick_tune(midi) + (0.12 if mode == "lift" else 0.0)
                a_bd_tune.set(t0 + at, min(0.78, tune))
                v = vel + (midi - 59) * 0.012 + (0.08 if at == 0.0 else 0.0)
                v = jit(v, 0.03)
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

def roll(t0, at, span, n, base_vel, tune_from, tune_to, restore):
    """A ratcheted snare roll with an sd_tune sweep, restored after."""
    d = span / n
    ev = []
    for i in range(n):
        v = base_vel * (0.4 + 0.6 * (i + 1) / n)
        ev.append((at + i * d, d, "SD", min(1.0, jit(v, 0.03))))
    a_sd_tune.set(t0 + at, tune_from)
    a_sd_tune.ramp(t0 + at, tune_to, span, "exp")
    a_sd_tune.set(t0 + at + span, restore)
    return ev

def make_kick_pat(style, v):
    if style == "entry":
        return {0, 6, 10}
    if style == "halftime":
        return {0, 12}
    base = {0, 6, 10}
    for cand, p in ((3, 0.6), (8, 0.45), (12, 0.5), (5, 0.25), (11, 0.25)):
        if R.random() < p * (0.4 + 0.6 * v):
            base.add(cand)
    return base

def make_acid_riff(root, oct_shift, dens, v):
    """One 2-bar riff on a 28-step grid: the theme's bones plus motion.
    Chosen once per phrase — a riff, not weather."""
    theme_at = {p % 7: m for p, m in THEME}
    riff = []
    for s in range(28):
        half_bar, s14 = divmod(s, 14)
        pulse, half = divmod(s14, 2)
        if half == 0 and pulse in theme_at and \
                (half_bar == 0 or R.random() < 0.6):
            riff.append((s, theme_at[pulse] - 12 + (root - 45) + oct_shift, True))
        elif R.random() < (0.2 + 0.2 * v) * dens:
            m = root - 12 if R.random() < 0.5 else root
            if R.random() < 0.15:
                m = scale_near(root, R.choice([-1, 1, 2]))
            riff.append((s, m + oct_shift, False))
    return riff

WHISPER = [57, 60, 64, 62, 59, 55]  # theme pitches the drill kick borrows
_whisper_i = 0

def drill_phrase(first_bar, n_bars, v=(0.5, 0.6), gain=(0.9, 1.0), root=45,
                 style="drill", acid_oct=0, acid_dens=1.0, sd_base=0.42,
                 hats_on=True, ghosts_n=1, end="roll_up", breaths=(),
                 blasts=None, washes=(), acid_on=True):
    """One 8-bar phrase: patterns fixed at the door, dynamics arced
    across it, mutation only in the last bar."""
    global _whisper_i
    blasts = blasts or {}
    t_start = bb(first_bar)

    # the phrase's identity, chosen once
    kick_pat = make_kick_pat(style, v[1])
    ghost_steps = R.sample([3, 5, 9, 11, 13], ghosts_n) if ghosts_n else []
    hat_rot = R.randrange(N16)
    oh_step = R.choice([7, 13])
    riff = make_acid_riff(root, acid_oct, acid_dens, v[1]) if acid_on else []
    a_sd_tune.set(t_start, sd_base)
    # the kick whispers the theme into the drill, one root per phrase
    a_bd_tune.set(t_start, kick_tune(WHISPER[_whisper_i % len(WHISPER)]) * 0.55)
    _whisper_i += 1

    for i in range(n_bars):
        bar = first_bar + i
        t0 = bb(bar)
        frac = i / max(1, n_bars - 1)
        gn = gain[0] + (gain[1] - gain[0]) * frac
        vv = v[0] + (v[1] - v[0]) * frac
        last = i == n_bars - 1

        if i in breaths:   # the loom inhales: clock keeps turning, one kick
            kick.bar(t0, [f"BD:0.25@{g(0.8, gn):.2f}"])
            continue

        if i in blasts:    # a Venetian bar: the snare becomes a surface
            up = blasts[i] == "up"
            n = R.choice([21, 28])
            ev = roll(t0, 0.0, BAR, n, 0.7 * gn,
                      0.2 if up else 0.85, 0.9 if up else 0.18, sd_base)
            steps_to_tokens(snare, t0, ev)
            kick.bar(t0, [f"BD:0.25@{g(0.95, gn):.2f}",
                          f"R:{fmt(BAR - 0.5)}", f"BD:0.25@{g(0.9, gn):.2f}"])
            continue

        # --- kick: the phrase's riff, mutated only in the last bar
        pat = set(kick_pat)
        if last and style != "halftime":
            c = R.choice([3, 5, 8, 11, 12])
            pat.symmetric_difference_update({c})
            pat.add(0)
        kev = []
        for s in sorted(pat):
            if style == "halftime":
                base = 1.0 if s == 0 else 0.85
            else:
                base = 0.95 if s == 0 else (0.82 if s in (6, 10) else 0.68)
            if style == "drill" and s != 0 and R.random() < 0.08 * vv:
                for k3 in range(3):
                    kev.append((s * S16 + k3 * S16 / 3, S16 / 3, "BD",
                                g(jit(0.75 - 0.13 * k3, 0.03), gn)))
            else:
                kev.append((s * S16, S16, "BD", g(jit(base, 0.03), gn)))
        steps_to_tokens(kick, t0, kev)

        # --- snare: backbeat by posture, fixed ghosts, phrase-end gesture
        sev = []
        occupied = set(pat)
        backs = {"drill": [4, 10], "entry": [10], "halftime": [8]}[style]
        for s in backs:
            sev.append((s * S16, S16, "SD",
                        g(jit(0.92 if style == "halftime" else 0.78, 0.04), gn)))
            occupied.add(s)
        for s2 in ghost_steps:
            if s2 not in occupied and R.random() < 0.8:
                sev.append((s2 * S16, S16, "SD", g(jit(0.1, 0.03), gn)))
                occupied.add(s2)
        if last and end != "none":
            if end == "big":
                sev = [e for e in sev if e[0] < 12 * S16 - 1e-9]
                sev.append((12 * S16, 2 * S16, "SD", g(0.95, gn)))
                throw(t0 + 12 * S16, 0.45, 2.0)
            else:
                up = end == "roll_up"
                span = 3 * S16
                start = (N16 - 3) * S16
                sev = [e for e in sev if e[0] < start - 1e-9]
                n = 6 if vv < 0.6 else R.choice([8, 10])
                sev += roll(t0, start, span, n, (0.42 + 0.3 * vv) * gn,
                            0.25 if up else 0.75, 0.8 if up else 0.25, sd_base)
                throw(t0 + start, 0.4, 1.5)
        sev.sort(key=lambda e: e[0])
        steps_to_tokens(snare, t0, sev)

        # --- hats: one rotation per phrase; strong steps are the 3+2+2 heads
        if hats_on:
            hev = []
            if i in washes:
                for s in range(0, N16, 2):
                    hev.append((s * S16, S16, "OH", g(jit(0.48, 0.04), gn)))
            elif style == "halftime":
                for s in (0, 4, 8, 12):
                    strong = s in (0, 8)
                    hev.append((s * S16, S16, "CH",
                                g(jit(0.45 if strong else 0.26, 0.04), gn)))
                if i % 2 == 1:
                    hev[-1] = (12 * S16, S16, "OH", g(jit(0.5, 0.04), gn))
            else:
                k = min(11, 5 + int(round(5 * vv)))
                hpat = euclid(k, N16, rot=hat_rot)
                for s in range(N16):
                    if hpat[s]:
                        strong = s in (0, 6, 10)
                        hev.append((s * S16, S16, "CH",
                                    g(jit(0.48 if strong else 0.26, 0.04), gn)))
                if i % 2 == 1:
                    hev = [e for e in hev if abs(e[0] - oh_step * S16) > 1e-9]
                    hev.append((oh_step * S16, S16, "OH", g(jit(0.5, 0.04), gn)))
                    hev.sort(key=lambda e: e[0])
            steps_to_tokens(hats, t0, hev)

        # --- clap seals the phrase's midpoint and end
        if i in (3, 7) and style != "entry":
            snare.lines.append(
                f">{fmt(t0 + 13 * S16)} CP:0.25@{g(jit(0.6, 0.04), gn):.2f}")

        # --- acid: the phrase's riff, register fixed, last bar mutated
        if acid_on and riff:
            half = i % 2
            toks = []
            cur = 0.0
            for s, m, accent in riff:
                hb, s14 = divmod(s, 14)
                if hb != half:
                    continue
                if last and not accent and R.random() < 0.3:
                    m = scale_near(m, R.choice([-2, 2]))
                at = s14 * S16
                if at > cur + 1e-9:
                    toks.append(f"R:{fmt(at - cur)}")
                    cur = at
                vel = (0.9 if accent else 0.52) * gn
                toks.append(f"{nn(m)}:0.25@{jit(min(1.0, vel), 0.04):.2f}")
                cur += 0.25
            bass.bar(t0, toks)

# ---------------------------------------------------------------- A: LOOM

kick_theme_bars(A0, 16, vel=0.58)
clock_bars(4, 12, vel=0.36)         # the thread, alone: let it be heard
clock_bars(16, END - 8 - 16)        # ...then woven under, to bar 184

a_wow.ramp(0.0, 0.12, 8 * BAR, "exp")
a_wow.set(bb(B0), 0.09)
a_volume.ramp(0.0, 0.7, 16 * BAR)
a_volume.ramp(bb(B0), 0.78, 24 * BAR)
a_volume.set(bb(E0), 0.82)
a_volume.ramp(bb(E0), 1.0, 40 * BAR)
a_volume.set(bb(F0), 0.6)

# first whispers: a ghost snare finds the loom in the dark, sometimes
for bar in range(10, 36):
    if R.random() < 0.55:
        t0 = bb(bar)
        s = R.choice([3, 5, 9, 11, 13])
        snare.lines.append(f">{fmt(t0 + s * S16)} SD:0.25@{jit(0.1, 0.03):.2f}")

# ---------------------------------------------------------- B: ACCUMULATE

kick_theme_bars(B0, 8, vel=0.58, vary=True)
kick_theme_bars(B0 + 8, 16, vel=0.62, heartbeat=True, vary=True)

# hat density is the accumulation made audible: k climbs 3 -> 11
for bar in range(B0 - 4, C0):
    t0 = bb(bar)
    k = min(9, 3 + max(0, bar - (B0 - 4)) // 4 * 2)
    pat = euclid(k, N16, rot=(bar * 5) % N16)
    toks = []
    cur = 0.0
    for i in range(N16):
        if pat[i]:
            at = i * S16
            if at > cur + 1e-9:
                toks.append(f"R:{fmt(at - cur)}")
                cur = at
            strong = i in (0, 6, 10)
            toks.append(f"CH:0.25@{jit(0.32 if strong else 0.18, 0.05):.2f}")
            cur += 0.25
    hats.bar(t0, toks)

# the drone under the loom
for bar in range(B0, C0, 4):
    drone.bar(bb(bar), [f"{nn(33)}:{fmt(4 * BAR * 0.96)}@0.4"])

# acid murmurs: fragments of the theme's floor, half asleep
for bar in range(28, C0):
    t0 = bb(bar)
    toks = []
    cur = 0.0
    for s in sorted(R.sample([0, 3, 6, 8, 10, 12], R.choice([2, 3]))):
        at = s * S16
        if at > cur + 1e-9:
            toks.append(f"R:{fmt(at - cur)}")
            cur = at
        m = 33 if R.random() < 0.7 else R.choice([36, 40, 45])
        toks.append(f"{nn(m)}:0.25@{jit(0.45, 0.05):.2f}")
        cur += 0.25
    bass.bar(t0, toks)

# the backbeat learns to stand: one limb first, quietly
for bar in range(32, C0):
    t0 = bb(bar)
    v = 0.28 + 0.2 * (bar - 32) / (C0 - 32)
    snare.bar(t0, [f"R:{fmt(10 * S16)}", f"SD:0.25@{jit(v, 0.03):.2f}"])
    if bar in (35, 39):
        for at, d, nme, vv2 in roll(t0, 12 * S16, 2 * S16, 3, 0.4,
                                    0.3, 0.6, 0.42):
            snare.lines.append(f">{fmt(t0 + at)} {nme}:{fmt(d)}@{vv2:.2f}")

a_drum_drv.ramp(bb(32), 0.18, 8 * BAR)
a_bass_cut.ramp(bb(28), 520, 12 * BAR, "exp")

# the loom tightens for the drill: kick becomes a drum, not a voice
a_bd_decay.ramp(bb(C0 - 1), 0.52, BAR)
a_bd_att.set(bb(C0), 0.55)
a_sd_snap.set(bb(C0), 0.68)
a_sd_level.set(bb(C0), 0.66)
a_hh_level.set(bb(C0), 0.56)
a_wow.set(bb(C0), 0.06)

# ------------------------------------------------------------- C: TORRENT
# Five phrases, five postures: enter / commit / stomp / weave / peak.

drill_phrase(C0, 8, v=(0.3, 0.4), gain=(0.75, 0.9), style="entry",
             hats_on=False, ghosts_n=0, sd_base=0.38, acid_dens=0.7,
             end="roll_up", breaths={6})
drill_phrase(C0 + 8, 8, v=(0.4, 0.55), gain=(0.9, 1.05), style="drill",
             ghosts_n=1, sd_base=0.42, end="roll_up")
drill_phrase(C0 + 16, 8, v=(0.5, 0.5), gain=(1.0, 1.0), style="halftime",
             root=41, acid_oct=-12, acid_dens=0.5, sd_base=0.5,
             ghosts_n=0, end="big", breaths={7})
drill_phrase(C0 + 24, 8, v=(0.5, 0.62), gain=(0.9, 1.05), style="drill",
             ghosts_n=2, sd_base=0.4, washes={5}, end="roll_down")
drill_phrase(C0 + 32, 7, v=(0.6, 0.72), gain=(0.95, 1.1), style="drill",
             ghosts_n=2, sd_base=0.44, blasts={4: "up"}, end="none")

# C's seam into SPEECH: one rising ratchet, then the floor drops
t0 = bb(D0 - 1)
sev = roll(t0, 0.0, BAR - 0.5, 10, 0.8, 0.25, 0.88, 0.42)
steps_to_tokens(snare, t0, sev)
kick.bar(t0, ["BD:0.25@0.95"])
throw(t0, 0.45, 2.5)

a_bass_cut.ramp(bb(C0), 1500, 36 * BAR, "exp")
a_bass_res.ramp(bb(C0 + 8), 1.45, 24 * BAR)
a_drum_drv.ramp(bb(C0), 0.3, (D0 - C0) * BAR)
a_rs_tune.ramp(bb(C0), 0.55, (D0 - C0) * BAR)
a_oh_dec.set(bb(C0 + 29), 0.7)
a_oh_dec.set(bb(C0 + 30), 0.35)

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

a_sd_level.set(bb(D0), 0.5)
a_hh_level.set(bb(D0), 0.42)
a_hh_tune.set(bb(D0), 0.42)
a_rs_tune.set(bb(D0), 0.32)
a_sd_snap.set(bb(D0), 0.5)
a_sd_tune.set(bb(D0), 0.38)

# four cycles, four postures: walk / hold breath / tick / gather
for cyc in range(4):
    for i in range(8):
        bar = D0 + cyc * 8 + i
        t0 = bb(bar)
        gn = 0.75 if cyc < 3 else 0.75 + 0.25 * i / 7
        if cyc != 1:
            kev = [(0, S16, "BD", g(jit(0.82, 0.03), gn)),
                   (6 * S16, S16, "BD", g(jit(0.58, 0.03), gn))]
            steps_to_tokens(kick, t0, kev)
        sv = 0.42 if cyc == 1 else 0.6
        snare.bar(t0, [f"R:1", f"SD:0.25@{g(jit(sv, 0.03), gn):.2f}",
                       f"R:1.25", f"SD:0.25@{g(jit(sv + 0.05, 0.03), gn):.2f}"])
        if cyc == 2:
            toks = []
            cur = 0.0
            for s in range(0, N16, 2):
                at = s * S16
                if at > cur + 1e-9:
                    toks.append(f"R:{fmt(at - cur)}")
                    cur = at
                toks.append(f"CH:0.25@{jit(0.16, 0.03):.2f}")
                cur += 0.25
            hats.bar(t0, toks)
        elif cyc != 1:
            pat = euclid(5, N16, rot=(D0 * 3 + cyc * 5) % N16)
            toks = []
            cur = 0.0
            for s in range(N16):
                if pat[s]:
                    at = s * S16
                    if at > cur + 1e-9:
                        toks.append(f"R:{fmt(at - cur)}")
                        cur = at
                    toks.append(f"CH:0.25@{jit(0.22, 0.03):.2f}")
                    cur += 0.25
            hats.bar(t0, toks)
        if cyc == 3 and i == 7:
            for at, d, nme, vv2 in roll(t0, 11 * S16, 3 * S16, 8, 0.6,
                                        0.3, 0.8, 0.42):
                snare.lines.append(
                    f">{fmt(t0 + at)} {nme}:{fmt(d)}@{vv2:.2f}")
            throw(t0 + 11 * S16, 0.45, 2.0)

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
# Six phrases: rebuild / commit / stomp / weave / press / tear.

a_sd_level.set(bb(E0), 0.62)
a_sd_level.ramp(bb(E0), 0.72, 40 * BAR)
a_hh_level.set(bb(E0), 0.52)
a_hh_level.ramp(bb(E0), 0.6, 40 * BAR)
a_hh_tune.set(bb(E0), 0.5)
a_hh_tune.ramp(bb(E0), 0.64, 40 * BAR)
a_rs_tune.set(bb(E0), 0.38)
a_rs_tune.ramp(bb(E0), 0.6, 44 * BAR)
a_sd_snap.set(bb(E0), 0.7)

drill_phrase(E0, 8, v=(0.45, 0.55), gain=(0.8, 0.9), style="drill",
             hats_on=False, ghosts_n=0, sd_base=0.4, acid_dens=0.85,
             end="none")
drill_phrase(E0 + 8, 8, v=(0.6, 0.72), gain=(0.9, 1.0), style="drill",
             ghosts_n=1, sd_base=0.42, blasts={7: "up"}, end="none")
drill_phrase(E0 + 16, 8, v=(0.6, 0.6), gain=(1.0, 1.0), style="halftime",
             root=41, acid_oct=-12, acid_dens=0.5, sd_base=0.5,
             ghosts_n=0, washes={3, 7}, end="big")
drill_phrase(E0 + 24, 8, v=(0.75, 0.85), gain=(0.9, 1.0), style="drill",
             root=43, ghosts_n=2, sd_base=0.44, end="roll_down")
drill_phrase(E0 + 32, 8, v=(0.85, 0.95), gain=(0.95, 1.05), style="drill",
             ghosts_n=2, sd_base=0.46, blasts={6: "down"}, end="roll_up")
drill_phrase(E0 + 40, 6, v=(0.95, 1.0), gain=(1.0, 1.1), style="drill",
             root=40, ghosts_n=1, sd_base=0.48,
             blasts={1: "up", 3: "down"}, end="none")

# the tear: one full-bar rising roll, one last kick, then air
t0 = bb(E0 + 46)
sev = roll(t0, 0.0, BAR, 21, 0.85, 0.2, 0.92, 0.42)
steps_to_tokens(snare, t0, sev)
kick.bar(t0, ["BD:0.25@1"])
throw(t0, 0.5, 3.0)
t0 = bb(E0 + 47)
kick.bar(t0, ["BD:0.25@1"])
a_wow.set(t0 + 0.5, 0.3)

# the lead returns possessed: theme an octave up, then the long high cry
sing(E0 + 8, 24, 0.7, True)
lead.bar(bb(E0 + 16), [f"{nn(81)}:{fmt(4 * BAR)}@0.72"])
sing(E0 + 20, 24, 0.74, True)
sing(E0 + 28, 24, 0.78, True)
lead.bar(bb(E0 + 36), [f"{nn(80)}:{fmt(BAR)}@0.75",
                       f"{nn(81)}:{fmt(3 * BAR)}@0.78"])
lead.bar(bb(E0 + 42), [f"{nn(84)}:{fmt(2 * BAR)}@0.75",
                       f"{nn(83)}:1@0.7", f"{nn(81)}:{fmt(2 * BAR - 1)}@0.72"])

a_fuzz.ramp(bb(E0), 0.18, 32 * BAR)
a_fuzz.set(bb(E0 + 46), 0.0)
a_drum_drv.ramp(bb(E0), 0.42, 40 * BAR)
a_drum_drv.set(bb(F0), 0.08)
a_bass_cut.set(bb(E0), 900)
a_bass_cut.ramp(bb(E0), 2400, 44 * BAR, "exp")
a_bass_res.ramp(bb(E0), 1.65, 44 * BAR)
a_bd_drive.ramp(bb(E0), 0.35, 40 * BAR)
a_bd_drive.set(bb(F0), 0.1)
a_oh_dec.set(bb(E0 + 19), 0.75); a_oh_dec.set(bb(E0 + 20), 0.35)
a_oh_dec.set(bb(E0 + 23), 0.75); a_oh_dec.set(bb(E0 + 24), 0.35)

# ---------------------------------------------------------- F: AFTERIMAGE

a_bd_decay.set(bb(F0), 0.88)
a_bd_att.set(bb(F0), 0.3)
a_sd_snap.set(bb(F0), 0.5)
a_sd_level.set(bb(F0), 0.48)
a_hh_level.set(bb(F0), 0.4)
a_rs_tune.set(bb(F0), 0.5)
a_rev_wet.set(bb(F0), 0.3)

kick_theme_bars(F0, 12, vel=0.44, half_speed=True)
# fragments: the theme loses words, keeps only downbeats
for bar in range(F0 + 12, F0 + 18, 2):
    t0 = bb(bar)
    a_bd_tune.set(t0, kick_tune(57))
    kick.bar(t0, [f"BD:0.25@{jit(0.42, 0.03):.2f}"])

pad.bar(bb(F0), ["[" + " ".join(nn(m) for m in CHORDS[0]) + "]"
                 + f":{fmt(4 * BAR)}@0.44"])
pad.bar(bb(F0 + 8), ["[" + " ".join(nn(m) for m in CHORDS[1]) + "]"
                     + f":{fmt(4 * BAR)}@0.4"])
pad.bar(bb(F0 + 16), ["[" + " ".join(nn(m) for m in CHORDS[0]) + "]"
                      + f":{fmt(8 * BAR)}@0.4"])

for bar in range(F0, F0 + 18, 4):
    drone.bar(bb(bar), [f"{nn(33)}:{fmt(4 * BAR * 0.96)}@0.36"])

# the bells remember three notes of it, then two, then one, then lower
bell.bar(bb(F0 + 4), [f"{nn(81)}:1@0.5", f"{nn(84)}:1@0.45", f"{nn(83)}:2@0.4"])
bell.bar(bb(F0 + 12), [f"{nn(76)}:1@0.45", f"{nn(74)}:2@0.4"])
bell.bar(bb(F0 + 20), [f"{nn(81)}:2@0.35"])
bell.bar(bb(F0 + 26), [f"{nn(69)}:2@0.3"])

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
# chews it into riffs, the lead confesses it over chords, the bells
# remember three notes of it, and a rimshot Euclidean clock E(5,14),
# rotating one step per bar, runs from bar 4 to the last bar without
# ever breaking — the thread the loom never drops.
#
# Second weaving: composed in 8-bar phrases (patterns fixed at each
# phrase's door, mutated only at its close), dynamics at three
# timescales (hit, phrase arc, section), the snare demoted from weather
# to punctuation, reverb played as dub throws, and every drum's pitch
# knob moving — the kick whispers the theme root into each drill phrase.
#
#   bars   0-16    LOOM        the kick alone, tuned like a drum choir
#   bars  16-40    ACCUMULATE  a thread every 4 bars; hats climb E(3..11,14)
#   bars  40-80    TORRENT     enter / commit / stomp / weave / peak
#   bars  80-112   SPEECH      walk / hold breath / tick / gather — E7's G#
#   bars 112-160   FRENZY      rebuild / commit / stomp / weave / press / tear
#   bars 160-192   AFTERIMAGE  half-speed theme, wow flood, the clock last
#
# Regenerate: python3 scripts/drums_for_laurie.py

bpm {BPM}
gate 0.82

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
automate rs_level
0.7
automate cp_level
0.62
automate hh_metal
0.55
automate sd_tone
0.45
automate sd_decay
0.42
"""

parts = [HEADER]
for a in (a_bd_tune, a_bd_decay, a_bd_att, a_bd_drive, a_sd_tune, a_sd_snap,
          a_sd_level, a_hh_level, a_hh_tune, a_rs_tune, a_oh_dec, a_drum_drv,
          a_fuzz, a_wow, a_spring, a_rev_wet, a_volume, a_bend):
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
