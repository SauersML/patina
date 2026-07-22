#!/usr/bin/env python
"""Drone base for The Prodigal Program — the picture plays the score.

Every visual event is driven by an actual note onset parsed from
songs/prodigal-program.song, quantized to the frame at 48 fps (21 ms —
tight enough to resolve the 64th-note and accelerando ratchet fills).
The stop-motion clock is the union of every percussive and melodic
onset: the image advances WHEN THE SONG HITS and holds when it
breathes — the A-section air is near-stillness, the snare ratchets
are accelerating strobes, the wormhole ladder slams six rungs.

The gated 64th clock: the subdivision grid carries the strobe, the
score gates it — while percussion strikes the clock runs; in the air
(no percussive onset within two beats) the picture freezes, advancing
only when a voice or chord enters. BREATHE, made visible.

Score events -> light:
    kicks (BD)        sidechain pump: detail brightens, zones duck,
                      both scaled by the bd_drive lane
    snare cracks      circuit flashes (Prophet-5 schematic crops),
                      glowing with sd_snappy and the snare filter
    worm ladder       two-frame flashes on each chop
    bells             one-frame specular glints
    hats              chromatic-fringe wobble
    breath chords     luminance swell, the patch's real 0.6/1.4 s
    bass root pitch   global hue lean by circle-of-fifths distance
                      from D: flatward cools, sharpward warms

Mix audio -> light (band envelopes from the rendered wav):
    RMS loudness      saturation: quiet = ashen, full mix blazes
    sub band          deeper blacks
    voice band        midtone lift
    high band         white glints, wider fringe

Automation -> light (the engineer's moves become grade moves):
    voxbass cutoff    the zones open at the bloom
    tape_wow          frame wobble as the tape dies (beat 128 on)
    rets pitch dive   exposure and hue sink with the falling pitch

Each of ~600 plots (10:00/12:00/15:00 captures, identical center
crop) spends its three times of day as three consecutive advances,
then is gone — no image ever repeats. Plots are banded by texture
drama (cached json) so the loudest land carries the wormhole; band
sizes are computed from the counted onsets per section. Grade: hue
rides low spatial frequencies (contiguous color zones per frame),
histogram equalization spreads every frame across the whole
five-anchor polychrome ramp, tanh tone curve, saturation push.

No text, no sound — this is the bed the rest of the video sits on.
Encoding is segmented mpegts with frame-accurate resume: something on
this machine reaps long renders every ~150s, so run it in a
relaunch-until-done loop and no frame is ever paid for twice.

    .venv-voice/bin/python scripts/prodigal-video.py
    -> renders/prodigal-base.mp4
"""
import json
import os
import re
import subprocess

import numpy as np
from PIL import Image

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SONG = os.path.join(REPO, "songs", "prodigal-program.song")
SRC_PNG = "/Users/user/Downloads/image sources/drone-lsr-images"
SRC_JPG = os.path.expanduser("~/Downloads/drone-lsr-images-rest")
STATS = os.path.join(REPO, "renders", "prodigal-plotstats.json")
OUT = os.path.join(REPO, "renders", "prodigal-base.mp4")

W, H, FPS = 1920, 1080, 48
BPM = 72.0
SPB = 60.0 / BPM
DUR = 130.586667            # matches renders/prodigal-program.wav
TOTAL = int(round(DUR * FPS))
TODS = ["1000", "1200", "1500"]
SECTIONS = [0.0, 16.0, 48.0, 80.0, 104.0, 128.0, 1e9]   # A B C D E F

# (beat, 5 colors at y = 0 / .28 / .5 / .72 / 1, exposure) — lerped in
# beat space. Hue rotates WITH lightness inside every frame: shadows,
# mids and highlights each get their own color, not shades of one hue.
GRADE = [
    (0,   ((6, 4, 22),   (52, 34, 130),  (120, 60, 200),
           (200, 140, 230), (232, 220, 255)), 0.82),
    (15,  ((6, 4, 22),   (52, 34, 130),  (120, 60, 200),
           (200, 140, 230), (232, 220, 255)), 0.88),
    (23,  ((10, 6, 40),  (90, 30, 170),  (210, 50, 180),
           (110, 150, 250), (210, 240, 255)), 1.00),
    (46,  ((10, 6, 40),  (90, 30, 170),  (210, 50, 180),
           (110, 150, 250), (210, 240, 255)), 1.00),
    (55,  ((2, 10, 34),  (60, 50, 180),  (20, 130, 210),
           (90, 220, 190),  (225, 250, 255)), 1.05),
    (79,  ((2, 10, 34),  (60, 50, 180),  (20, 130, 210),
           (90, 220, 190),  (225, 250, 255)), 1.05),
    (83,  ((20, 2, 10),  (120, 8, 60),   (220, 30, 40),
           (255, 120, 40),  (255, 230, 180)), 1.10),
    (101, ((20, 2, 10),  (120, 8, 60),   (220, 30, 40),
           (255, 120, 40),  (255, 230, 180)), 1.10),
    (110, ((30, 6, 40),  (170, 30, 90),  (240, 60, 180),
           (150, 120, 255), (255, 220, 240)), 1.20),
    (125, ((30, 6, 40),  (170, 30, 90),  (240, 60, 180),
           (150, 120, 255), (255, 220, 240)), 1.16),
    (139, ((8, 6, 18),   (60, 40, 100),  (110, 90, 140),
           (150, 150, 190), (200, 190, 210)), 0.95),
    (160, ((8, 6, 18),   (60, 40, 100),  (110, 90, 140),
           (150, 150, 190), (200, 190, 210)), 0.78),
]
TONE_S = 1.8                # gentle S on top of the equalized histogram
TONE_GAMMA = 1.0            # equalization already owns the distribution
T_FADE = DUR - 2.6          # everything sinks to black with the tape

LUM = np.array([0.2126, 0.7152, 0.0722], np.float32)
EQ_P = np.linspace(0.0, 1.0, 65)


def smoothstep(x):
    x = np.clip(x, 0.0, 1.0)
    return x * x * (3.0 - 2.0 * x)


def tone_curve(y):
    y = np.clip(y, 0.0, 1.0) ** TONE_GAMMA
    return 0.5 + np.tanh(TONE_S * (y - 0.5)) / (2.0 * np.tanh(TONE_S / 2.0))


# ---- the score: every onset, straight from the .song file ------------

def parse_song(path):
    """(tracks, autom): tracks = {name: [(beat, dur, vel, token)]},
    autom = {lane: [(t0, t1, v0, v1) ramp segments, beats]}."""
    tracks, positions, vels = {}, {}, {}
    autom, apos, acur = {}, {}, {}
    name, mode = None, None
    with open(path) as fh:
        for raw in fh:
            line = raw.rstrip("\n")
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            if not line[0].isspace():
                head = line.split()
                if head[0] == "track":
                    name, mode = head[1], "track"
                    tracks.setdefault(name, [])
                    positions.setdefault(name, 0.0)
                    m = re.search(r"\bvel=([0-9.]+)", line)
                    vels[name] = float(m.group(1)) if m else 0.8
                elif head[0] == "automate":
                    name, mode = head[1], "autom"
                    autom.setdefault(name, [])
                    apos.setdefault(name, 0.0)
                    acur.setdefault(name, None)
                else:
                    mode = "skip"     # bpm / gate / globals
                continue
            if mode == "autom":
                pos, cur = apos[name], acur[name]
                for tok in line.split():
                    m = re.match(r"^R:([0-9.]+)$", tok)
                    if m:
                        pos += float(m.group(1))
                        continue
                    m = re.match(r"^(-?[0-9.]+):([0-9.]+)(?:@\w+)?$", tok)
                    if m:                       # ramp to v over dur
                        v, dur = float(m.group(1)), float(m.group(2))
                        autom[name].append(
                            (pos, pos + dur, cur if cur is not None
                             else v, v))
                        pos += dur
                        cur = v
                        continue
                    m = re.match(r"^(-?[0-9.]+)$", tok)
                    if m:                       # instant set
                        v = float(m.group(1))
                        autom[name].append((pos, pos, v, v))
                        cur = v
                apos[name], acur[name] = pos, cur
                continue
            if mode != "track":
                continue
            # chords keep their spaces out of the tokenizer's way
            body = re.sub(r"\[([^\]]*)\]",
                          lambda m: "[" + m.group(1).replace(" ", "_") + "]",
                          line.strip())
            # expand ( ... )xN groups textually
            while True:
                m = re.search(r"\(([^()]*)\)x(\d+)", body)
                if not m:
                    break
                body = (body[:m.start()]
                        + " ".join([m.group(1)] * int(m.group(2)))
                        + body[m.end():])
            pos = positions[name]
            for tok in body.split():
                m = re.match(r"^([^\s:]+):([0-9.]+)(?:@([0-9.]+))?$", tok)
                if not m:
                    continue
                nm, dur = m.group(1), float(m.group(2))
                vel = float(m.group(3)) if m.group(3) else vels[name]
                if nm != "R":
                    tracks[name].append((pos, dur, vel, nm))
                pos += dur
            positions[name] = pos
    return tracks, autom


SCORE, AUTOM = parse_song(SONG)


def autom_frames(lane, default):
    """Per-frame values of an automation lane, in its native units."""
    out = np.full(TOTAL, np.float32(default))
    segs = AUTOM.get(lane, [])
    if not segs:
        return out
    beats = np.arange(TOTAL, dtype=np.float64) / FPS / SPB
    cur = np.float32(segs[0][2])
    out[:] = cur
    for t0, t1, v0, v1 in segs:
        i0, i1 = int(t0 * SPB * FPS), int(t1 * SPB * FPS)
        i0, i1 = min(i0, TOTAL), min(i1, TOTAL)
        if i1 > i0:
            out[i0:i1] = v0 + (v1 - v0) * (
                (beats[i0:i1] - t0) / max(t1 - t0, 1e-9))
        out[i1:] = v1
    return out


def onsets(track, names=None, min_vel=0.0, min_dur=0.0):
    return [(b, d, v) for b, d, v, n in SCORE.get(track, [])
            if (names is None or n in names) and v >= min_vel
            and d >= min_dur]


KICKS = sorted(onsets("beat", {"BD"}) + onsets("kickK")
               + onsets("kickB"))
SNARES = sorted(onsets("beat", {"SD"}) + onsets("snareI"))
TSS = onsets("snare")
HATS = sorted(onsets("hats") + onsets("ohK"))
SHAKER = onsets("shaker")
BELLS = sorted(onsets("bells") + onsets("bellsHi"))
WORM = onsets("worm")
BREATHS = sorted(onsets("breath") + onsets("voxbass") + onsets("sub"))
VOICES = sorted(onsets("themeA") + onsets("themeB") + onsets("owls")
                + onsets("grat") + onsets("rev") + onsets("notiS")
                + onsets("rets"))

# ---- the stop-motion clock: fast, 64th-aligned, and BREATHING --------
# The subdivision grid carries the strobe (sixteenths in the breath,
# 32nds under the groove, 64ths through the wormhole, 32nd triplets in
# the bloom — every step lands exactly on a metric subdivision, frame-
# quantized). The score gates it: while percussion strikes the clock
# runs; when the song holds its air (no percussive onset within two
# beats) the picture FREEZES, advancing only when a voice or chord
# enters. The song's first law — BREATHE — made visible.
RATES = [(16, 4), (48, 8), (80, 8), (104, 16), (128, 12), (1e9, 4)]


def step_at(beat):
    steps, prev = 0.0, 0.0
    for end, rate in RATES:
        if beat <= end:
            return int(steps + (beat - prev) * rate)
        steps += (end - prev) * rate
        prev = end
    return int(steps)


PERC_BEATS = np.array(sorted(b for b, _, _ in
                             (KICKS + SNARES + TSS + HATS + SHAKER)))
MELODIC_F = {int(round(b * SPB * FPS))
             for b in sorted(x for x, _, _ in
                             (BREATHS + VOICES + BELLS + WORM))
             if b * SPB < DUR}

STEP_F = np.zeros(TOTAL, np.int32)
_steps, _prev_sub = 0, 0
for _f in range(TOTAL):
    _b = _f / FPS / SPB
    _sub = step_at(_b)
    _i = np.searchsorted(PERC_BEATS, _b, side="right")
    _gate = _i > np.searchsorted(PERC_BEATS, _b - 2.0, side="right")
    if _gate:
        _steps += _sub - _prev_sub
    elif _f in MELODIC_F:
        _steps += 1
    _prev_sub = _sub
    STEP_F[_f] = _steps
N_ADVANCE = int(STEP_F[-1]) + 1


# ---- the catalog: every plot, its drama, and the running order -------

def plot_path(name, tod):
    p = os.path.join(SRC_PNG, f"{name}__time_{tod}.png")
    if os.path.exists(p):
        return p
    return os.path.join(SRC_JPG, f"{name}__time_{tod}.jpg")


def all_plots():
    names = set()
    for d, ext in ((SRC_PNG, ".png"), (SRC_JPG, ".jpg")):
        for f in os.listdir(d):
            if f.endswith(ext) and "__time_" in f:
                names.add(f.split("__time_")[0])
    return sorted(names)


def drama_scores():
    if os.path.exists(STATS):
        with open(STATS) as fh:
            return json.load(fh)
    scores = {}
    for name in all_plots():
        im = Image.open(plot_path(name, "1200")).convert("L")
        im.thumbnail((128, 128))
        y = np.asarray(im, dtype=np.float32) / 255.0
        gy, gx = np.gradient(y)
        scores[name] = float(y.std() + 2.0 * np.abs(gx + 1j * gy).mean())
    os.makedirs(os.path.dirname(STATS), exist_ok=True)
    with open(STATS, "w") as fh:
        json.dump(scores, fh)
    return scores


def running_order():
    """Unique plots, banded by drama, sized by the score itself: each
    section gets as many plots as its gated step clock demands."""
    counts = []
    for lo, hi in zip(SECTIONS[:-1], SECTIONS[1:]):
        f0 = min(TOTAL - 1, int(lo * SPB * FPS))
        f1 = min(TOTAL - 1, int(hi * SPB * FPS))
        counts.append(int(STEP_F[f1] - STEP_F[f0]))
    need = [int(np.ceil(c / 3)) + 2 for c in counts]

    scores = drama_scores()
    ranked = sorted(scores, key=scores.get)      # calm -> violent
    n = len(ranked)
    rng = np.random.default_rng(72)
    used = set()

    def pick(lo, hi, k, order=None):
        band = [p for p in ranked[int(lo * n):int(hi * n)] if p not in used]
        while len(band) < k:
            lo, hi = max(0.0, lo - 0.05), min(1.0, hi + 0.05)
            band = [p for p in ranked[int(lo * n):int(hi * n)]
                    if p not in used]
        sel = [band[i] for i in rng.permutation(len(band))[:k]]
        used.update(sel)
        if order == "asc":
            sel.sort(key=scores.get)
        elif order == "desc":
            sel.sort(key=scores.get, reverse=True)
        return sel

    seq = []
    seq += pick(0.05, 0.30, need[0], order="asc")    # A: waking up
    seq += pick(0.30, 0.60, need[1])                 # B: the groove works
    seq += pick(0.45, 0.78, need[2])                 # C: roads and fences
    seq += pick(0.78, 1.00, need[3])                 # D: the loudest land
    seq += pick(0.60, 0.88, need[4])                 # E: bloom
    seq += pick(0.00, 0.35, need[5], order="desc")   # F: emptying out
    return seq


SEQ = running_order()


class PlotCache:
    """Per plot: the 3 times of day, identically center-cropped to
    1920x1080, plus their equalized+curved luminance planes."""

    def __init__(self, cap=2):
        self.cap, self.d, self.order = cap, {}, []

    def get(self, name):
        if name not in self.d:
            imgs, ys = [], []
            cdf_q = None
            for tod in TODS:
                im = Image.open(plot_path(name, tod)).convert("RGB")
                im = im.crop((0, 224, 1024, 800)).resize((W, H),
                                                         Image.BILINEAR)
                a = np.asarray(im, dtype=np.float32) / 255.0
                y = a @ LUM
                if cdf_q is None:
                    # one shared equalization for the triplet: spreads
                    # pixel mass across the WHOLE ramp while the real
                    # exposure jumps between times of day stay honest
                    cdf_q = np.maximum.accumulate(np.quantile(y, EQ_P))
                    cdf_q = cdf_q + np.arange(len(EQ_P)) * 1e-7
                imgs.append(a)
                ys.append(y)
            for i in range(3):
                ys[i] = tone_curve(
                    np.interp(ys[i], cdf_q, EQ_P)).astype(np.float32)
            self.d[name] = (imgs, ys)
            self.order.append(name)
            if len(self.order) > self.cap:
                del self.d[self.order.pop(0)]
        return self.d[name]


CACHE = PlotCache()

# ---- the circuit flashes ---------------------------------------------
# Snare cracks and the wormhole ladder surface the machine's skeleton:
# Prophet-5 schematic crops (renders/prodigal-schem/*.png), inverted to
# white-on-black, graded by whatever palette owns that beat. Crop
# indices are sequential per flash event — like the fields, no crop
# ever repeats.
SCHEM_DIR = os.path.join(REPO, "renders", "prodigal-schem")
SCHEM_PAGES = (sorted(os.listdir(SCHEM_DIR))
               if os.path.isdir(SCHEM_DIR) else [])


class SchemCache:
    def __init__(self, cap=6):
        self.cap, self.d, self.order = cap, {}, []

    def page(self, i):
        name = SCHEM_PAGES[i % len(SCHEM_PAGES)]
        if name not in self.d:
            a = np.asarray(Image.open(
                os.path.join(SCHEM_DIR, name)).convert("L"),
                dtype=np.float32) / 255.0
            self.d[name] = a
            self.order.append(name)
            if len(self.order) > self.cap:
                del self.d[self.order.pop(0)]
        return self.d[name]


SCHEM = SchemCache()


def flash_y(crop_idx):
    page = SCHEM.page(crop_idx)
    ph, pw = page.shape
    rng = np.random.default_rng(9000 + crop_idx)
    cw = int(pw * rng.uniform(0.35, 0.85))
    ch = min(int(cw * 9 / 16), ph)
    x0 = rng.integers(0, max(pw - cw, 1))
    y0 = rng.integers(0, max(ph - ch, 1))
    crop = 1.0 - page[y0:y0 + ch, x0:x0 + cw]      # white lines on black
    im = Image.fromarray((np.clip(crop, 0, 1) * 255).astype(np.uint8))
    y = np.asarray(im.resize((W, H), Image.BILINEAR),
                   dtype=np.float32) / 255.0
    return np.clip((y - 0.06) / 0.88, 0.0, 1.0) ** 0.9


# flash schedule: frame -> unique crop index. Snare cracks (909 SD at
# force, TSS hits with real length) get 1-2 frames by velocity; every
# rung of the wormhole ladder gets 2.
FLASH_AT = {}
_crop = 0
_flashes = ([(b, v, 1 + (v >= 0.85)) for b, _, v in SNARES if v >= 0.72]
            + [(b, v, 1 + (v >= 0.8)) for b, d, v in TSS
               if v >= 0.6 and d >= 1.5]
            + [(b, v, 2) for b, _, v in WORM])
for b, v, nfr in sorted(_flashes):
    f0 = int(round(b * SPB * FPS))
    for k in range(nfr):
        if f0 + k < TOTAL and (f0 + k) not in FLASH_AT:
            FLASH_AT[f0 + k] = _crop
            _crop += 1

# per-frame envelopes, precomputed straight from the score
PUMP = np.zeros(TOTAL, np.float32)      # kicks: exposure
FRINGE = np.zeros(TOTAL, np.float32)    # hats+shaker: chromatic wobble
GLINT = np.zeros(TOTAL, np.float32)     # bells: specular sparkle
BREATH_ENV = np.zeros(TOTAL, np.float32)  # exhale chords: slow swell
_t = np.arange(TOTAL, dtype=np.float32) / FPS


def _decay(env, beats, tau, gain=1.0):
    for b, _, v in beats:
        t0 = b * SPB
        if t0 >= DUR:
            continue
        i0 = int(t0 * FPS)
        seg = np.exp(-(_t[i0:] - t0) / tau) * v * gain
        env[i0:] = np.maximum(env[i0:], seg)


_decay(PUMP, KICKS, 0.12)
_decay(FRINGE, HATS + SHAKER, 0.06)
_decay(GLINT, BELLS, 0.08)
for b, d, v in BREATHS:
    t0, t1 = b * SPB, min((b + d) * SPB, DUR)
    if t0 >= DUR:
        continue
    i0, i1 = int(t0 * FPS), int(t1 * FPS)
    att = np.clip((_t[i0:i1] - t0) / 0.6, 0, 1)
    BREATH_ENV[i0:i1] = np.maximum(BREATH_ENV[i0:i1], att * v)
    rel = np.exp(-(_t[i1:] - t1) / 1.4) * v
    BREATH_ENV[i1:] = np.maximum(BREATH_ENV[i1:], rel)


# ---- sound -> light: band energies from the actual mix --------------
# RMS loudness owns saturation (quiet = ashen, loud = blazing), the
# high band throws white glints and widens the fringe, the voice band
# lifts the midtones, the sub band deepens the blacks.
WAV = os.path.join(REPO, "renders", "prodigal-program.wav")


def band_envelopes():
    import soundfile as sf
    audio, sr = sf.read(WAV, dtype="float32")
    if audio.ndim > 1:
        audio = audio.mean(axis=1)
    nfft = 2048
    rms = np.zeros(TOTAL, np.float32)
    bands = np.zeros((3, TOTAL), np.float32)     # sub / voice / high
    freqs = np.fft.rfftfreq(nfft, 1.0 / sr)
    masks = [(freqs >= 20) & (freqs < 120),
             (freqs >= 250) & (freqs < 2000),
             (freqs >= 4000) & (freqs < 12000)]
    win = np.hanning(nfft).astype(np.float32)
    for f in range(TOTAL):
        c = int(f / FPS * sr)
        seg = audio[max(0, c - nfft // 2):c + nfft // 2]
        if len(seg) < nfft:
            seg = np.pad(seg, (0, nfft - len(seg)))
        rms[f] = np.sqrt(np.mean(seg ** 2))
        mag = np.abs(np.fft.rfft(seg * win))
        for i, m in enumerate(masks):
            bands[i, f] = mag[m].mean()

    def smooth_norm(x, release):
        out = np.copy(x)
        k = np.exp(-1.0 / (FPS * release))
        for f in range(1, len(out)):             # fast up, slow down
            out[f] = max(out[f], out[f - 1] * k)
        return np.clip(out / (np.percentile(out, 95.0) + 1e-9), 0, 1.2)

    return (smooth_norm(rms, 0.25), smooth_norm(bands[0], 0.15),
            smooth_norm(bands[1], 0.12), smooth_norm(bands[2], 0.08))


RMS_N, BASS_N, MID_N, HIGH_N = band_envelopes()

# ---- pitch -> hue: the bass root leans the whole palette ------------
# Circle-of-fifths distance from D (the tonic): flatward roots cool
# the frame, sharpward roots warm it. Chord changes recolor the world.
FIFTHS = {"D": 0, "A": 1, "E": 2, "B": 3, "F#": 4, "C#": 5, "G#": 6,
          "G": -1, "C": -2, "F": -3, "A#": -4, "D#": -5}
HUE_LEAN = np.zeros(TOTAL, np.float32)
_roots = sorted((b, re.match(r"([A-G]#?)", n).group(1))
                for b, d, v, n in
                (SCORE.get("voxbass", []) + SCORE.get("sub", []))
                if re.match(r"([A-G]#?)", n))
for _i, (_b, _root) in enumerate(_roots):
    _t0 = _b * SPB
    _t1 = _roots[_i + 1][0] * SPB if _i + 1 < len(_roots) else DUR
    _f0, _f1 = int(_t0 * FPS), min(int(_t1 * FPS), TOTAL)
    HUE_LEAN[_f0:_f1] = FIFTHS.get(_root, 0) / 60.0   # ~6 deg per fifth
# quarter-second slew so chord changes sweep instead of snapping
_k = np.exp(-1.0 / (FPS * 0.25))
for _f in range(1, TOTAL):
    HUE_LEAN[_f] = HUE_LEAN[_f] * (1 - _k) + HUE_LEAN[_f - 1] * _k

# ---- the engineer's moves: automation lanes become grade moves ------
BD_DRIVE = autom_frames("bd_drive", 0.85)       # kick punch per section
SD_SNAP = autom_frames("sd_snappy", 0.4)        # flash snap per section
SN_CUT = autom_frames("snare.smp_cutoff", 20000.0)  # flash open/dark
VB_CUT = autom_frames("voxbass.smp_cutoff", 700.0)  # the bed opens
TAPE_WOW = autom_frames("tape_wow", 0.5)        # the tape dying
RETS_P = autom_frames("rets.smp_pitch", 0.0)    # the final pitch dive
LFO_RATE = float(autom_frames("lfo_rate", 0.12)[0])

# flashes glow with the snare channel: snap adds bite, the closed
# filter of section C smolders instead of blazing
FLASH_GAIN = ((0.75 + 0.55 * SD_SNAP)
              * (0.45 + 0.55 * np.clip(SN_CUT / 20000.0, 0.0, 1.0)))
# the sub's slow LFO drifts the whole palette a few degrees; the tape
# dive at the end drags every hue cold in proportion to its pitch
HUE_LEAN += (0.006 * np.sin(2 * np.pi * LFO_RATE * _t)
             + 0.10 * (RETS_P / 24.0)).astype(np.float32)
# and the dying tape sinks the exposure with the pitch
RETS_EXPO = (1.0 + 0.45 * (RETS_P / 24.0)).astype(np.float32)
WOW_AMP = 22.0 * np.maximum(0.0, TAPE_WOW - 0.5)   # px, 0 until beat 128
ZONE_OPEN = (0.9 + 0.2 * np.clip((VB_CUT - 700.0) / 700.0, 0, 1)
             ).astype(np.float32)


def rotate_hue(anchors, lean):
    """Rotate the palette's hues by `lean` (fraction of the wheel)."""
    if abs(lean) < 1e-4:
        return anchors
    import colorsys
    out = np.empty_like(anchors)
    for i, (r, g, b) in enumerate(anchors):
        h, s, v = colorsys.rgb_to_hsv(r, g, b)
        out[i] = colorsys.hsv_to_rgb((h + lean) % 1.0, s, v)
    return out


def grade_at(beat):
    bs = [g[0] for g in GRADE]
    beat = min(max(beat, bs[0]), bs[-1])
    j = max(1, np.searchsorted(bs, beat))
    j = min(j, len(bs) - 1)
    b0, p0, e0 = GRADE[j - 1]
    b1, p1, e1 = GRADE[j]
    t = smoothstep((beat - b0) / max(b1 - b0, 1e-9))
    pal = [tuple(a + (b - a) * t for a, b in zip(c0, c1))
           for c0, c1 in zip(p0, p1)]
    return pal, e0 + (e1 - e0) * t


def render_frame(f):
    t = f / FPS
    # the gated 64th clock: hits advance the picture, air holds it
    idx = int(STEP_F[f])
    plot = min(idx // 3, len(SEQ) - 1)

    crop_idx = FLASH_AT.get(f)
    if crop_idx is None:
        imgs, ys = CACHE.get(SEQ[plot])
        img, y = imgs[idx % 3], ys[idx % 3]
    else:
        img, y = None, flash_y(crop_idx)

    pal, expo = grade_at(t / SPB)
    anchors = np.float32(pal) / 255.0           # (5, 3): hue-by-lightness
    anchors = rotate_hue(anchors, float(HUE_LEAN[f]))
    ramp = np.float32([0.0, 0.28, 0.5, 0.72, 1.0])
    # hue rides the LOW frequencies (shadow banks, bright patches form
    # contiguous color zones), detail rides lightness inside each zone
    ylow = np.asarray(
        Image.fromarray((y * 255).astype(np.uint8))
        .resize((60, 34), Image.BILINEAR)
        .resize((W, H), Image.BILINEAR), np.float32) / 255.0
    zone = np.empty((H, W, 3), np.float32)
    for c in range(3):
        zone[..., c] = np.interp(ylow, ramp, anchors[:, c])
    zone *= ZONE_OPEN[f]        # the bed's filter opens at the bloom
    # the visible sidechain: on each kick the color zones DUCK while
    # the detail layer pumps, both scaled by the kick's drive lane;
    # the sub band keeps the blacks deep
    pump = PUMP[f] * (BD_DRIVE[f] / 0.85)
    base = (0.26 - 0.12 * BASS_N[f]) * (1.0 - 0.30 * pump)
    detail = 1.05 * (1.0 + 0.35 * pump)
    graded = zone * (base + detail * y)[..., None]
    graded += (y ** 4)[..., None] * (anchors[-1] * 0.45)  # specular lift
    if img is not None:
        graded = graded * 0.93 + img * 0.07  # a breath of the real field
    else:
        graded *= FLASH_GAIN[f]  # flashes glow with the snare channel
    # the voice band lights the midtones
    if MID_N[f] > 0.05:
        graded *= (1.0 + 0.14 * MID_N[f]
                   * np.exp(-((y - 0.55) ** 2) / 0.045))[..., None]
    # bells and the high band throw white glints off the top
    glint = max(0.9 * GLINT[f], 0.7 * HIGH_N[f])
    if glint > 0.02:
        graded += (y ** 6)[..., None] * (anchors[-1] * glint)

    lum = (graded @ LUM)[..., None]
    # loudness owns saturation: quiet = ashen, the full mix blazes
    graded = lum + (graded - lum) * (0.75 + 0.60 * RMS_N[f])

    # the exhale chords swell the exposure; the dying tape sinks it
    graded *= expo * RETS_EXPO[f] * (1.0 + 0.06 * BREATH_ENV[f])
    graded *= VIG

    # tape wow: the frame wobbles as the tape dies
    if WOW_AMP[f] >= 0.5:
        wob = int(round(WOW_AMP[f] * np.sin(2 * np.pi * 1.1 * t)))
        if wob:
            graded = np.roll(graded, wob, axis=0)

    # analog fringe: red and blue drift apart, hats and hiss knock
    # them wider
    px = 2 + int(round(3.0 * FRINGE[f] + 2.0 * HIGH_N[f]))
    graded[..., 0] = np.roll(graded[..., 0], px, axis=1)
    graded[..., 2] = np.roll(graded[..., 2], -px, axis=1)

    rng = np.random.default_rng(f)
    ga = 0.016 if t < T_FADE else 0.016 + 0.02 * (t - T_FADE) / (DUR - T_FADE)
    graded += rng.normal(0.0, ga, (H, W, 1)).astype(np.float32)

    if t > T_FADE:
        graded *= float(1.0 - smoothstep((t - T_FADE) / (DUR - T_FADE)))

    return (np.clip(graded, 0.0, 1.0) * 255).astype(np.uint8)


# vignette, once — lighter than before, the colors carry the depth now
yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
rr = np.sqrt(((xx / W - 0.5) * 2) ** 2 + ((yy / H - 0.5) * 1.6) ** 2)
VIG = (1.0 - 0.34 * np.clip(rr, 0, 1.25) ** 2.2)[..., None].astype(np.float32)
del yy, xx, rr


def seg_frames(path):
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "v:0",
         "-count_packets", "-show_entries", "stream=nb_read_packets",
         "-of", "csv=p=0", path], capture_output=True, text=True)
    try:
        return int(out.stdout.split()[0])
    except (ValueError, IndexError):
        return 0


def main():
    os.makedirs(os.path.join(REPO, "renders"), exist_ok=True)
    segdir = os.path.join(REPO, "renders", "prodigal-base-segs")
    os.makedirs(segdir, exist_ok=True)
    # resume: keep whatever earlier (killed) runs already landed
    segments = []
    for name in sorted(os.listdir(segdir)):
        if name.endswith(".ts"):
            path = os.path.join(segdir, name)
            n = seg_frames(path)
            if n > 0:
                segments.append((path, n))
            else:
                os.remove(path)

    def open_segment(idx):
        path = os.path.join(segdir, f"seg{idx:03d}.ts")
        while os.path.exists(path):
            idx += 1
            path = os.path.join(segdir, f"seg{idx:03d}.ts")
        ff = subprocess.Popen(
            ["ffmpeg", "-y", "-loglevel", "error",
             "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}",
             "-r", str(FPS), "-i", "-",
             "-c:v", "libx264", "-preset", "veryfast", "-crf", "18",
             "-pix_fmt", "yuv420p", "-f", "mpegts", path],
            stdin=subprocess.PIPE)
        return path, ff

    f = sum(n for _, n in segments)
    if f:
        print(f"resuming at frame {f}", flush=True)
    seg_path, ff = open_segment(len(segments))
    landed = 0
    while f < TOTAL:
        frame = render_frame(f)
        try:
            ff.stdin.write(frame.tobytes())
            f += 1
            landed += 1
        except BrokenPipeError:
            print(f"ffmpeg died at frame {f}; resuming", flush=True)
            segments.append((seg_path, landed))
            seg_path, ff = open_segment(len(segments))
            landed = 0
        if f % 480 == 0:
            print(f"{f}/{TOTAL} frames ({f / FPS:.1f}s)", flush=True)
    ff.stdin.close()
    ff.wait()
    segments.append((seg_path, landed))

    concat = "concat:" + "|".join(p for p, _ in segments)
    subprocess.run(
        ["ffmpeg", "-y", "-loglevel", "error", "-i", concat,
         "-c", "copy", OUT], check=True)
    for p, _ in segments:
        os.remove(p)
    os.rmdir(segdir)
    print(OUT)


if __name__ == "__main__":
    main()
