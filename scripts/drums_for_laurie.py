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
# Third weaving. What changed:
#   - the drum melody is now WRITING, not a trick: the kick states the
#     theme, the snare (snap killed, sd_tune played per-note) answers
#     with a composed counter-line, then follows in canon at one pulse.
#     The snare VOICE then morphs into the drill's backbeat — sd_snappy
#     ramps open across four bars and the singer becomes the drummer.
#     In the drill the kick keeps singing: every hit cycles the theme's
#     pitches through bd_tune. The afterimage is the duet, augmented.
#   - filters and effects carry dynamics: the acid's cutoff saw-tooths
#     per phrase (reset dark at each door, opened across it), the pad
#     blooms over the whole speech, the lead's filter swells into every
#     sung phrase like breath, spring reverb spikes on the stomp's one
#     big hit, tape drive leans in across the frenzy
#   - shorter (148 bars, ~2:45), and every section moves somewhere:
#     nothing sits still without a reason
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

# The counter-line: written against the theme's gaps, contrary where the
# theme rises, consonant where they sound together. E4 G3 B3 D4 E4.
COUNTER = [(1, 64), (4, 55), (8, 59), (11, 62), (13, 64)]

def tune_of(midi):
    """Map the drum choir's register onto a tune knob: contour, not pitch."""
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
#   0-12      LOOM        theme plain, then diminished, then two voices
#   12-28     ACCUMULATE  canon; the singer becomes the drummer
#   28-60     TORRENT     enter / commit / stomp / weave / four-bar rise
#   60-84     SPEECH      walk / hold breath / gather — E7's G#
#   84-124    FRENZY      rebuild / commit / stomp / press / blasts / tear
#   124-148   AFTERIMAGE  the duet in augmentation; the clock last

A0, B0, C0, D0, E0, F0, END = 0, 12, 28, 60, 84, 124, 148

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

a_bd_tune  = Auto("bd_tune", tune_of(57))
a_bd_decay = Auto("bd_decay", 0.8)
a_bd_att   = Auto("bd_attack", 0.35)
a_bd_drive = Auto("bd_drive", 0.12)
a_sd_tune  = Auto("sd_tune", 0.42)
a_sd_snap  = Auto("sd_snappy", 0.12)   # snap killed: the snare is a VOICE
a_sd_tone  = Auto("sd_tone", 0.32)
a_sd_level = Auto("sd_level", 0.55)
a_hh_level = Auto("hh_level", 0.5)
a_hh_tune  = Auto("hh_tune", 0.5)
a_rs_tune  = Auto("rs_tune", 0.42)
a_oh_dec   = Auto("oh_decay", 0.35)
a_drum_drv = Auto("dr_drive", 0.05)
a_fuzz     = Auto("fuzz", 0.0)
a_wow      = Auto("tape_wow", 0.5)
a_tape_drv = Auto("tape_drive", 0.28)
a_spring   = Auto("spring", 0.08)
a_rev_wet  = Auto("reverb_wet", 0.16)
a_chorus   = Auto("chorus_mode", 0)
# The master volume is played, not set: Spiegel's hand-drawn control
# line over the whole form — the loudness arc IS one of the voices.
a_volume   = Auto("volume", 0.6)
a_bend     = Auto("bend", 0.0)
a_bass_cut = Auto("bass.cutoff", 340)
a_bass_res = Auto("bass.resonance", 1.15)
a_lead_cut = Auto("lead.cutoff", 2200)
a_pad_cut  = Auto("pad.cutoff", 900)

def throw(beat, wet=0.4, hold=1.5):
    """A dub throw: the reverb send opens for one gesture, then shuts."""
    a_rev_wet.set(beat, wet)
    a_rev_wet.set(beat + hold, 0.16)

# ------------------------------------------------ the drum choir (A, B, F)

def voice_bars(track, drum, tune_auto, line, first_bar, n_bars, vel,
               stretch=1, ornament_p=0.0, delay=0, heartbeat=False):
    """A melodic line played by one drum of the choir: tune set per note,
    velocity following the contour, passing 16ths added with probability
    ornament_p (the additive process — the line grows notes over time)."""
    phrase_pulses = 14 * stretch
    for bar in range(first_bar, first_bar + n_bars):
        t0 = bb(bar)
        p0 = ((bar - first_bar) * 7) % phrase_pulses
        hits = []
        for pulse, midi in line:
            p = pulse * stretch + delay - p0
            if 0 <= p < 7:
                hits.append((float(p), midi))
        if heartbeat:
            hits.append((3.5, None))
        hits.sort(key=lambda h: h[0])
        ev = []  # (pulse, midi_or_None, is_ornament)
        for idx, (p, midi) in enumerate(hits):
            ev.append((p, midi, False))
            if midi is None or not ornament_p or idx + 1 >= len(hits):
                continue
            np_, nm = hits[idx + 1]
            if nm is not None and np_ - p >= 1.0 and R.random() < ornament_p:
                passing = scale_near(midi, 1 if nm > midi else -1)
                ev.append((np_ - 0.5, passing, True))
        ev.sort(key=lambda e: e[0])
        toks = []
        cur = 0.0
        for p, midi, orn in ev:
            at = p * PULSE
            if at > cur + 1e-9:
                toks.append(f"R:{fmt(at - cur)}")
                cur = at
            elif at < cur - 1e-9:
                continue
            if midi is not None:
                tune_auto.set(t0 + at, tune_of(midi))
                v = vel + (midi - 59) * 0.012 + (0.08 if p == 0.0 else 0.0)
                if orn:
                    v *= 0.65
                v = jit(v, 0.03)
            else:
                tune_auto.set(t0 + at, 0.18)
                v = jit(vel - 0.2, 0.03)
            toks.append(f"{drum}:0.25@{v:.2f}")
            cur += 0.25
        track.bar(t0, toks)

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

# Even in the drill the kick keeps singing: every hit takes the next
# theme pitch, scaled down into drum range — melody as an undertow.
MELODIC = [m for _, m in THEME]
_mk_i = 0

def next_kick_tune(depth=0.3):
    global _mk_i
    m = MELODIC[_mk_i % len(MELODIC)]
    _mk_i += 1
    return round(0.12 + tune_of(m) * depth, 3)

def drill_phrase(first_bar, n_bars, v=(0.5, 0.6), gain=(0.9, 1.0), root=45,
                 style="drill", acid_oct=0, acid_dens=1.0, sd_base=0.42,
                 hats_on=True, ghosts_n=1, end="roll_up", breaths=(),
                 blasts=None, washes=(), acid_on=True, cut=None):
    """One phrase: patterns fixed at the door, dynamics arced across it,
    mutation only in the last bar. cut=(from,to) saw-tooths the acid's
    filter across the phrase — reset dark, opened wide."""
    blasts = blasts or {}
    t_start = bb(first_bar)

    kick_pat = make_kick_pat(style, v[1])
    ghost_steps = R.sample([3, 5, 9, 11, 13], ghosts_n) if ghosts_n else []
    hat_rot = R.randrange(N16)
    oh_step = R.choice([7, 13])
    riff = make_acid_riff(root, acid_oct, acid_dens, v[1]) if acid_on else []
    a_sd_tune.set(t_start, sd_base)
    if cut:
        a_bass_cut.set(t_start, cut[0])
        a_bass_cut.ramp(t_start, cut[1], n_bars * BAR, "exp")

    for i in range(n_bars):
        bar = first_bar + i
        t0 = bb(bar)
        frac = i / max(1, n_bars - 1)
        gn = gain[0] + (gain[1] - gain[0]) * frac
        vv = v[0] + (v[1] - v[0]) * frac
        last = i == n_bars - 1

        if i in breaths:   # the loom inhales: clock keeps turning, one kick
            a_bd_tune.set(t0, next_kick_tune())
            kick.bar(t0, [f"BD:0.25@{g(0.8, gn):.2f}"])
            continue

        if i in blasts:    # a Venetian bar: the snare becomes a surface
            up = blasts[i] == "up"
            n = R.choice([21, 28])
            ev = roll(t0, 0.0, BAR, n, 0.7 * gn,
                      0.2 if up else 0.85, 0.9 if up else 0.18, sd_base)
            steps_to_tokens(snare, t0, ev)
            a_bd_tune.set(t0, next_kick_tune())
            kick.bar(t0, [f"BD:0.25@{g(0.95, gn):.2f}",
                          f"R:{fmt(BAR - 0.5)}", f"BD:0.25@{g(0.9, gn):.2f}"])
            continue

        # --- kick: the phrase's riff, every hit singing the next theme pitch
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
            a_bd_tune.set(t0 + s * S16, next_kick_tune(
                0.25 if style == "halftime" else 0.3))
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
                a_spring.set(t0 + 12 * S16, 0.3)   # the stomp's hit boings
                a_spring.set(t0 + 12 * S16 + 2.0, 0.08)
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
# Three postures, four bars each: plain / diminished / two voices.

voice_bars(kick, "BD", a_bd_tune, THEME, A0, 4, vel=0.58)
voice_bars(kick, "BD", a_bd_tune, THEME, A0 + 4, 4, vel=0.6, ornament_p=0.45)
voice_bars(kick, "BD", a_bd_tune, THEME, A0 + 8, 4, vel=0.62, ornament_p=0.3)
voice_bars(snare, "SD", a_sd_tune, COUNTER, A0 + 8, 4, vel=0.38)

clock_bars(4, 8, vel=0.36)          # the thread, alone: let it be heard
clock_bars(B0, END - 8 - B0)        # ...then woven under, to bar 140

a_wow.ramp(0.0, 0.12, 8 * BAR, "exp")
a_wow.set(bb(B0), 0.09)
a_volume.ramp(0.0, 0.7, 12 * BAR)
a_volume.ramp(bb(B0), 0.8, 16 * BAR)
a_volume.set(bb(E0), 0.82)
a_volume.ramp(bb(E0), 1.0, 32 * BAR)
a_volume.set(bb(F0), 0.6)

# ---------------------------------------------------------- B: ACCUMULATE
# The theme in canon with itself; then the singer becomes the drummer.

# canon: the snare voice follows the kick one pulse behind, ornaments grow
voice_bars(kick, "BD", a_bd_tune, THEME, B0, 8, vel=0.56, ornament_p=0.25)
voice_bars(snare, "SD", a_sd_tune, THEME, B0, 8, vel=0.26, delay=1)
# second statement: counter-line proper, heartbeat under, more ornaments
voice_bars(kick, "BD", a_bd_tune, THEME, B0 + 8, 4, vel=0.58,
           ornament_p=0.45, heartbeat=True)
voice_bars(snare, "SD", a_sd_tune, COUNTER, B0 + 8, 4, vel=0.38)
# the morph: same kick line, but the snare's snap opens — the voice
# becomes a drum over four bars, landing as the drill's backbeat
voice_bars(kick, "BD", a_bd_tune, THEME, B0 + 12, 4, vel=0.6,
           ornament_p=0.55, heartbeat=True)
a_sd_snap.ramp(bb(B0 + 12), 0.68, 4 * BAR)
a_sd_tone.ramp(bb(B0 + 12), 0.45, 4 * BAR)
a_sd_tune.set(bb(B0 + 12), 0.42)
for bar in range(B0 + 12, C0):
    t0 = bb(bar)
    v = 0.35 + 0.25 * (bar - B0 - 12) / 4
    snare.bar(t0, [f"R:{fmt(10 * S16)}", f"SD:0.25@{jit(v, 0.03):.2f}"])
for bar, nroll in ((C0 - 2, 4), (C0 - 1, 6)):
    t0 = bb(bar)
    for at, d, nme, vv2 in roll(t0, 11 * S16, 3 * S16, nroll,
                                0.4 + 0.15 * (bar - C0 + 2), 0.3, 0.7, 0.42):
        snare.lines.append(f">{fmt(t0 + at)} {nme}:{fmt(d)}@{vv2:.2f}")
throw(bb(C0 - 1) + 11 * S16, 0.4, 1.5)

# hat density is the accumulation made audible: k climbs 3 -> 9
for bar in range(B0 - 2, C0):
    t0 = bb(bar)
    k = min(9, 3 + max(0, bar - (B0 - 2)) // 3 * 2)
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
            toks.append(f"CH:0.25@{jit(0.28 if strong else 0.16, 0.05):.2f}")
            cur += 0.25
    hats.bar(t0, toks)

# the drone under the loom
for bar in range(B0 + 4, C0, 4):
    drone.bar(bb(bar), [f"{nn(33)}:{fmt(4 * BAR * 0.96)}@0.4"])

# acid murmurs, waking: the filter opens audibly across eight bars
for bar in range(B0 + 8, C0):
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
a_bass_cut.ramp(bb(B0 + 8), 950, 8 * BAR, "exp")

a_drum_drv.ramp(bb(B0 + 8), 0.2, 8 * BAR)

# the loom tightens for the drill: kick becomes a drum, not a voice
a_bd_decay.ramp(bb(C0 - 1), 0.52, BAR)
a_bd_att.set(bb(C0), 0.55)
a_sd_level.set(bb(C0), 0.66)
a_hh_level.set(bb(C0), 0.56)
a_wow.set(bb(C0), 0.06)

# ------------------------------------------------------------- C: TORRENT
# enter (4) / commit (8) / stomp (8) / weave (8) / the four-bar rise.

drill_phrase(C0, 4, v=(0.32, 0.42), gain=(0.78, 0.9), style="entry",
             hats_on=False, ghosts_n=0, sd_base=0.38, acid_dens=0.7,
             end="roll_up", cut=(500, 750))
drill_phrase(C0 + 4, 8, v=(0.42, 0.58), gain=(0.9, 1.05), style="drill",
             ghosts_n=1, sd_base=0.42, end="roll_up", cut=(600, 1200))
drill_phrase(C0 + 12, 8, v=(0.5, 0.5), gain=(1.0, 1.0), style="halftime",
             root=41, acid_oct=-12, acid_dens=0.5, sd_base=0.5,
             ghosts_n=0, end="big", breaths={7}, cut=(420, 800))
drill_phrase(C0 + 20, 8, v=(0.55, 0.7), gain=(0.9, 1.08), style="drill",
             root=43, ghosts_n=2, sd_base=0.44, washes={5},
             end="roll_down", cut=(750, 1700))

# the rise: two committed bars, one rising surface, one bar of air
drill_phrase(C0 + 28, 2, v=(0.7, 0.75), gain=(1.0, 1.1), style="drill",
             ghosts_n=1, sd_base=0.46, end="none", cut=(1000, 1900))
t0 = bb(C0 + 30)
sev = roll(t0, 0.0, BAR, 21, 0.75, 0.2, 0.9, 0.42)
steps_to_tokens(snare, t0, sev)
a_bd_tune.set(t0, next_kick_tune())
kick.bar(t0, ["BD:0.25@0.95"])
hats.bar(t0, " ".join(f"OH:0.5@{jit(0.5, 0.04):.2f}" for _ in range(7)).split())
a_oh_dec.set(t0, 0.7)
a_oh_dec.set(bb(D0), 0.35)
throw(t0, 0.45, 3.0)
t0 = bb(C0 + 31)
a_bd_tune.set(t0, tune_of(57))
kick.bar(t0, ["BD:0.25@0.9"])

a_bass_res.ramp(bb(C0 + 4), 1.45, 24 * BAR)
a_drum_drv.ramp(bb(C0), 0.32, (D0 - C0) * BAR)
a_rs_tune.ramp(bb(C0), 0.55, (D0 - C0) * BAR)

# -------------------------------------------------------------- D: SPEECH
# Three cycles: walk / hold breath / gather. The pad blooms all the way.

CHORDS = [
    [45, 52, 59, 60],   # Am(add9)  A2 E3 B3 C4
    [41, 48, 52, 57],   # Fmaj7     F2 C3 E3 A3
    [43, 50, 59, 64],   # G6        G2 D3 B3 E4
    [40, 47, 55, 62],   # Em7       E2 B2 G3 D4
]
E7 = [40, 47, 56, 62]   # the piece's only G#: the ache toward home

for cyc in range(3):
    for ci in range(4):
        bar = D0 + cyc * 8 + ci * 2
        chord = E7 if (ci == 3 and cyc == 2) else CHORDS[ci]
        pad.bar(bb(bar), ["[" + " ".join(nn(m) for m in chord) + "]"
                          + f":{fmt(2 * BAR * 0.96)}@0.5"])
a_pad_cut.set(bb(D0), 700)
a_pad_cut.ramp(bb(D0), 2400, 24 * BAR, "exp")   # the bloom is the build

a_bd_decay.set(bb(D0), 0.75)    # the kick sings again under the chords
a_bd_att.set(bb(D0), 0.3)
a_bd_decay.set(bb(E0), 0.52)
a_bd_att.set(bb(E0), 0.55)
a_sd_level.set(bb(D0), 0.5)
a_hh_level.set(bb(D0), 0.42)
a_hh_tune.set(bb(D0), 0.42)
a_rs_tune.set(bb(D0), 0.32)
a_sd_snap.set(bb(D0), 0.5)
a_sd_tune.set(bb(D0), 0.38)

for cyc in range(3):
    for i in range(8):
        bar = D0 + cyc * 8 + i
        t0 = bb(bar)
        gn = 0.75 if cyc < 2 else 0.75 + 0.25 * i / 7
        if cyc == 0 and i % 2 == 0:
            pass  # sotto voce kick handled below, half-speed theme
        elif cyc == 2:
            kev = [(0, S16, "BD", g(jit(0.82, 0.03), gn)),
                   (6 * S16, S16, "BD", g(jit(0.58, 0.03), gn))]
            a_bd_tune.set(t0, next_kick_tune(0.25))
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
        if cyc == 2 and i == 7:
            for at, d, nme, vv2 in roll(t0, 11 * S16, 3 * S16, 8, 0.6,
                                        0.3, 0.8, 0.42):
                snare.lines.append(
                    f">{fmt(t0 + at)} {nme}:{fmt(d)}@{vv2:.2f}")
            throw(t0 + 11 * S16, 0.45, 2.0)

# the kick sings under the first cycle, half speed, sotto voce
voice_bars(kick, "BD", a_bd_tune, THEME, D0, 8, vel=0.34, stretch=2)

# the confession: the theme sung, the filter swelling into every phrase
def sing(first_bar, transpose, vel, ornament):
    t0 = bb(first_bar)
    a_lead_cut.set(t0, 900)
    a_lead_cut.ramp(t0, 3200, 6.5, "exp")   # the breath into the phrase
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
sing(D0 + 18, 24, 0.66, True)
# over the E7 bars the lead leans on the G#, unresolved until FRENZY
a_lead_cut.set(bb(D0 + 22), 1100)
a_lead_cut.ramp(bb(D0 + 22), 3600, 2 * BAR, "exp")
lead.bar(bb(D0 + 22), [f"{nn(68)}:2@0.7", f"{nn(69)}:1@0.6",
                       f"{nn(71)}:{fmt(2 * BAR - 3)}@0.65"])

for at_bar, m in ((D0 + 7, 76), (D0 + 15, 72)):
    bell.bar(bb(at_bar) + 12 * S16, [f"{nn(m)}:1@0.55"])

# -------------------------------------------------------------- E: FRENZY
# rebuild / commit / stomp / press / the blast phrase / the tear.

a_sd_level.set(bb(E0), 0.62)
a_sd_level.ramp(bb(E0), 0.72, 32 * BAR)
a_hh_level.set(bb(E0), 0.52)
a_hh_level.ramp(bb(E0), 0.6, 32 * BAR)
a_hh_tune.set(bb(E0), 0.5)
a_hh_tune.ramp(bb(E0), 0.64, 32 * BAR)
a_rs_tune.set(bb(E0), 0.38)
a_rs_tune.ramp(bb(E0), 0.6, 38 * BAR)
a_sd_snap.set(bb(E0), 0.7)
a_tape_drv.ramp(bb(E0), 0.5, 38 * BAR)   # the tape leans in

drill_phrase(E0, 8, v=(0.45, 0.55), gain=(0.8, 0.9), style="drill",
             hats_on=False, ghosts_n=0, sd_base=0.4, acid_dens=0.85,
             end="none", cut=(700, 1000))
drill_phrase(E0 + 8, 8, v=(0.6, 0.72), gain=(0.9, 1.0), style="drill",
             ghosts_n=1, sd_base=0.42, blasts={7: "up"}, end="none",
             cut=(900, 1800))
drill_phrase(E0 + 16, 8, v=(0.6, 0.6), gain=(1.0, 1.0), style="halftime",
             root=41, acid_oct=-12, acid_dens=0.5, sd_base=0.5,
             ghosts_n=0, washes={3, 7}, end="big", cut=(500, 1000))
drill_phrase(E0 + 24, 8, v=(0.78, 0.9), gain=(0.92, 1.02), style="drill",
             root=43, ghosts_n=2, sd_base=0.44, end="roll_down",
             cut=(1000, 2200))
drill_phrase(E0 + 32, 6, v=(0.92, 1.0), gain=(1.0, 1.1), style="drill",
             root=40, ghosts_n=1, sd_base=0.48,
             blasts={1: "up", 3: "down"}, end="none", cut=(1200, 2600))

# the tear: one full-bar rising roll, one last kick, then air
t0 = bb(E0 + 38)
sev = roll(t0, 0.0, BAR, 21, 0.85, 0.2, 0.92, 0.42)
steps_to_tokens(snare, t0, sev)
kick.bar(t0, ["BD:0.25@1"])
throw(t0, 0.5, 3.0)
t0 = bb(E0 + 39)
kick.bar(t0, ["BD:0.25@1"])
a_wow.set(t0 + 0.5, 0.3)

# the lead returns possessed: theme an octave up, then the long high cry
sing(E0 + 8, 24, 0.72, True)
a_lead_cut.set(bb(E0 + 16), 1200)
a_lead_cut.ramp(bb(E0 + 16), 4200, 4 * BAR, "exp")
lead.bar(bb(E0 + 16), [f"{nn(81)}:{fmt(4 * BAR)}@0.72"])
sing(E0 + 20, 24, 0.75, True)
sing(E0 + 28, 24, 0.78, True)
a_lead_cut.set(bb(E0 + 32), 1400)
a_lead_cut.ramp(bb(E0 + 32), 4600, 6 * BAR, "exp")
lead.bar(bb(E0 + 32), [f"{nn(80)}:{fmt(BAR)}@0.75",
                       f"{nn(81)}:{fmt(3 * BAR)}@0.78"])
lead.bar(bb(E0 + 36), [f"{nn(84)}:{fmt(BAR)}@0.75",
                       f"{nn(83)}:1@0.7", f"{nn(81)}:{fmt(BAR - 1)}@0.72"])

a_fuzz.ramp(bb(E0), 0.18, 28 * BAR)
a_fuzz.set(bb(E0 + 38), 0.0)
a_drum_drv.ramp(bb(E0), 0.42, 32 * BAR)
a_drum_drv.set(bb(F0), 0.08)
a_bass_res.ramp(bb(E0), 1.65, 38 * BAR)
a_bd_drive.ramp(bb(E0), 0.35, 32 * BAR)
a_bd_drive.set(bb(F0), 0.1)

# ---------------------------------------------------------- F: AFTERIMAGE
# The duet returns in augmentation: two drum voices, remembering.

a_bd_decay.set(bb(F0), 0.88)
a_bd_att.set(bb(F0), 0.3)
a_sd_snap.set(bb(F0), 0.12)     # the drummer becomes the singer again
a_sd_tone.set(bb(F0), 0.3)
a_sd_level.set(bb(F0), 0.45)
a_hh_level.set(bb(F0), 0.4)
a_rs_tune.set(bb(F0), 0.5)
a_rev_wet.set(bb(F0), 0.3)
a_tape_drv.set(bb(F0), 0.25)

voice_bars(kick, "BD", a_bd_tune, THEME, F0, 8, vel=0.44, stretch=2)
voice_bars(snare, "SD", a_sd_tune, COUNTER, F0, 8, vel=0.3, stretch=2,
           delay=1)
# fragments: the theme loses words, keeps only downbeats
for bar in range(F0 + 8, F0 + 14, 2):
    t0 = bb(bar)
    a_bd_tune.set(t0, tune_of(57))
    kick.bar(t0, [f"BD:0.25@{jit(0.42, 0.03):.2f}"])

pad.bar(bb(F0), ["[" + " ".join(nn(m) for m in CHORDS[0]) + "]"
                 + f":{fmt(4 * BAR)}@0.44"])
pad.bar(bb(F0 + 8), ["[" + " ".join(nn(m) for m in CHORDS[1]) + "]"
                     + f":{fmt(3 * BAR)}@0.4"])
pad.bar(bb(F0 + 14), ["[" + " ".join(nn(m) for m in CHORDS[0]) + "]"
                      + f":{fmt(8 * BAR)}@0.4"])
a_pad_cut.set(bb(F0), 1800)
a_pad_cut.ramp(bb(F0), 500, 20 * BAR, "exp")   # the light going out
a_chorus.set(bb(F0 + 8), 2)                    # the room widens as it dims

for bar in range(F0, F0 + 14, 4):
    drone.bar(bb(bar), [f"{nn(33)}:{fmt(4 * BAR * 0.96)}@0.36"])

# the bells remember three notes of it, then two, then one, then lower
bell.bar(bb(F0 + 4), [f"{nn(81)}:1@0.5", f"{nn(84)}:1@0.45", f"{nn(83)}:2@0.4"])
bell.bar(bb(F0 + 10), [f"{nn(76)}:1@0.45", f"{nn(74)}:2@0.4"])
bell.bar(bb(F0 + 16), [f"{nn(81)}:2@0.35"])
bell.bar(bb(F0 + 20), [f"{nn(69)}:2@0.3"])

# the clock alone, fading — the process outlives the song
clock_bars(END - 8, 7, vel=0.26, fade_to=0.08)

a_wow.ramp(bb(F0 + 8), 0.55, 16 * BAR, "exp")
a_spring.ramp(bb(F0), 0.35, 12 * BAR)
a_volume.ramp(bb(END - 6), 0.0, 6 * BAR, "smooth")
a_bend.ramp(bb(END - 3), -2.0, 3 * BAR, "smooth")

# ---------------------------------------------------------------- emit

HEADER = f"""# Drums, For Laurie — 190 bpm, 7/8, A minor. Generated, on purpose:
# the score is a program (scripts/drums_for_laurie.py, seed {SEED}), the
# way "Drums" was a program on the GROOVE system at Bell Labs — pitched
# drum voices driven by pattern logic, steered by hand-drawn curves.
#
# The process is allowed to accelerate until it becomes breakcore.
# One 10-note theme is the whole piece, and the drums are a CHOIR:
# the kick states the theme (bd_tune played per-note), the snare —
# snap killed — answers with a counter-line and follows in canon,
# then MORPHS into the drill's backbeat as its snap ramps open. Even
# in the drill every kick hit sings the next theme pitch. The acid
# bass chews the theme into riffs, the lead confesses it over chords
# (its filter swelling into each phrase like breath), the bells
# remember three notes of it, and a rimshot Euclidean clock E(5,14),
# rotating one step per bar, never breaks until it is all that's left.
#
#   bars   0-12    LOOM        plain / diminished / two voices
#   bars  12-28    ACCUMULATE  canon; the singer becomes the drummer
#   bars  28-60    TORRENT     enter / commit / stomp / weave / the rise
#   bars  60-84    SPEECH      walk / hold breath / gather — E7's G#
#   bars  84-124   FRENZY      rebuild / commit / stomp / press / tear
#   bars 124-148   AFTERIMAGE  the duet in augmentation; the clock last
#
# Regenerate: python3 scripts/drums_for_laurie.py

bpm {BPM}
gate 0.82

automate reverb_decay
0.62
automate tape_flutter
0.05
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
automate sd_decay
0.42
"""

parts = [HEADER]
for a in (a_bd_tune, a_bd_decay, a_bd_att, a_bd_drive, a_sd_tune, a_sd_snap,
          a_sd_tone, a_sd_level, a_hh_level, a_hh_tune, a_rs_tune, a_oh_dec,
          a_drum_drv, a_fuzz, a_wow, a_tape_drv, a_spring, a_rev_wet,
          a_chorus, a_volume, a_bend):
    parts.append(a.text())
for t in (kick, snare, hats, clock, bass, drone, pad, bell, lead):
    parts.append(t.text())
parts.append(a_bass_cut.text())
parts.append(a_bass_res.text())
parts.append(a_lead_cut.text())
parts.append(a_pad_cut.text())

out = "\n\n".join(parts) + "\n"
repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
path = os.path.join(repo, "songs", "drums-for-laurie.song")
with open(path, "w") as f:
    f.write(out)
print(f"wrote {path}: {len(out.splitlines())} lines, "
      f"{END * BAR * 60 / BPM:.0f}s before tail")
