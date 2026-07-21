#!/usr/bin/env python3
"""Drums, For Laurie — the music video: THE LOOM.

Not a visualization of the audio: a weaving of the score. The video is
generated from the same .song file the synth plays — every knot in the
cloth is an event, placed at its exact beat. The fabric is the piece:

  - each bar of 7/8 is one weft row of 14 warp columns (the 16th grid)
  - the KICK is one continuous amber thread; its horizontal wander
    inside each column is the bd_tune melody — the drum choir's tune,
    literally visible as a thread's path
  - the SNARE voice is a cream thread; in the accumulation you can see
    it running one pulse behind the kick — the canon, woven
  - snare rolls stitch diagonals as sd_tune sweeps; blast bars become
    dense stitched bands
  - hats are brass dots on the warp, the rim clock is electric cyan
    and never stops; claps are red double-knots; bells are starbursts
  - the lead draws a wide gold line across the whole warp — the theme
    roaming free of the columns
  - the control curves (volume, tape wow, acid cutoff, snare tune) run
    in the side gutters as thin cyan hand-drawn lines — GROOVE's
    interface was exactly this
  - below the shuttle line there is NOTHING but bare warp: the future
    has not been woven yet
  - tape wow bends the image: the opening is a wobbling memory that
    snaps into focus as the wow curve does; the ending dissolves
  - in the afterimage the view pulls back and the entire piece is
    revealed as one finished tapestry, six panels, the cyan clock
    still ticking down the last strip

Usage:
  python3 scripts/laurie-video.py                    # full render
  python3 scripts/laurie-video.py --preview 8,45,120 # PNG stills (secs)
"""

import argparse
import os
import re
import struct
import subprocess
import sys

import numpy as np

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SONG = os.path.join(REPO, "songs", "drums-for-laurie.song")
WAV = os.path.join(REPO, "renders", "drums-for-laurie.wav")
OUT = os.path.join(REPO, "renders", "drums-for-laurie.mp4")

FPS = 30
W, H = 1280, 720
BPM = 190.0
BAR = 3.5
N16 = 14

# cloth geometry
COL_W = 50                    # px per warp column
CLOTH_W = COL_W * N16         # 700
PX_BEAT = 24                  # px per beat of weft
CLOTH_X0 = (W - CLOTH_W) // 2
SHUTTLE_Y = int(H * 0.62)     # the present moment, in frame rows

# palette (RGB, 0..1) — Patina's design language: warm materials for the
# voices, electric cyan for the readouts, near-black warm ground
BG      = np.array([0.020, 0.015, 0.012])
WARP    = np.array([0.10, 0.08, 0.05])
CYAN    = np.array([0.10, 0.80, 0.90])
AMBER_L = np.array([0.55, 0.18, 0.05])   # kick thread, low tune
AMBER_H = np.array([1.00, 0.72, 0.20])   # kick thread, high tune
CREAM   = np.array([0.95, 0.88, 0.70])   # snare voice thread
BRASS   = np.array([0.55, 0.45, 0.28])   # closed hat
BRASS_O = np.array([0.75, 0.62, 0.35])   # open hat
RED     = np.array([0.90, 0.25, 0.15])   # clap
GOLD    = np.array([1.00, 0.85, 0.45])   # lead thread
EMBER   = np.array([0.45, 0.12, 0.06])   # bass ticks / drone
STAR    = np.array([0.80, 0.95, 1.00])   # bells
PADW    = np.array([0.10, 0.06, 0.035])  # chord wash
PADACHE = np.array([0.13, 0.045, 0.055]) # the E7 wash, tinted rose

# ------------------------------------------------------------ song parser

NOTE_RE = re.compile(r"^([A-Ga-g])([#b]?)(-?\d+)$")
SEMI = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}

def parse_note(s):
    m = NOTE_RE.match(s)
    if not m:
        return None
    letter, acc, octv = m.groups()
    v = SEMI[letter.upper()] + (1 if acc == "#" else -1 if acc == "b" else 0)
    return v + (int(octv) + 1) * 12

def tokenize(line):
    return re.findall(r"\[[^\]]*\]\S*|\S+", line)

def strip_comment(line):
    """`#` starts a comment only at line start or after whitespace, so
    sharp note names (F#2) survive — mirrors song.rs strip_comment."""
    for i, ch in enumerate(line):
        if ch == "#" and (i == 0 or line[i - 1].isspace()):
            return line[:i]
    return line

def parse_song(path):
    """Parse the generated .song subset: returns (events, autos).
    events: list of dicts {beat, dur, vel, track, kind, midi(s)/name}
    autos: {param: list of (beat, kind, value, dur, shape)}"""
    events, autos = [], {}
    mode = None            # ("track", name, vel, len) | ("auto", param)
    track_beat = 0.0
    cur_auto = None
    defaults = {}
    for raw in open(path):
        line = strip_comment(raw).strip()
        if not line:
            continue
        head = line.split()[0]
        if head in ("bpm", "gate"):
            continue
        if head == "track":
            name = line.split()[1]
            vel, ln = 0.8, 1.0
            for opt in line.split()[2:]:
                if opt.startswith("vel="):
                    vel = float(opt[4:])
                elif opt.startswith("len="):
                    ln = float(opt[4:])
            defaults[name] = (vel, ln)
            mode = ("track", name)
            track_beat = 0.0
            continue
        if head == "automate":
            param = line.split()[1]
            autos.setdefault(param, [])
            mode = ("auto", param)
            cur_auto = autos[param]
            track_beat = 0.0
            continue
        if mode is None:
            continue
        if mode[0] == "track":
            name = mode[1]
            dvel, dlen = defaults[name]
            for tok in tokenize(line):
                if tok.startswith(">"):
                    track_beat = float(tok[1:])
                    continue
                vel, dur = dvel, dlen
                body = tok
                if "@" in body:
                    body, v = body.rsplit("@", 1)
                    vel = float(v)
                if ":" in body and not body.startswith("["):
                    body, d = body.rsplit(":", 1)
                    dur = float(d)
                elif body.startswith("[") and "]:" in tok:
                    body, rest = tok.split("]:", 1)
                    body += "]"
                    dur = float(rest.split("@")[0])
                if body in (".", "R", "r"):
                    track_beat += dur
                    continue
                if body.startswith("["):
                    midis = [parse_note(x) for x in body[1:-1].split()]
                    events.append(dict(beat=track_beat, dur=dur, vel=vel,
                                       track=name, kind="chord", midis=midis))
                else:
                    m = parse_note(body)
                    if m is None:
                        events.append(dict(beat=track_beat, dur=dur, vel=vel,
                                           track=name, kind="drum", name=body))
                    else:
                        events.append(dict(beat=track_beat, dur=dur, vel=vel,
                                           track=name, kind="note", midi=m))
                track_beat += dur
        else:
            for tok in tokenize(line):
                if tok.startswith(">"):
                    track_beat = float(tok[1:])
                    continue
                shape, body = "lin", tok
                if "@" in body:
                    body, shape = body.rsplit("@", 1)
                if ":" in body:
                    v, d = body.rsplit(":", 1)
                    d = float(d)
                    if v in (".", "R", "r"):
                        track_beat += d
                        continue
                    cur_auto.append((track_beat, "ramp", float(v), d, shape))
                    track_beat += d
                else:
                    cur_auto.append((track_beat, "set", float(body), 0.0, "lin"))
    return events, autos

RES = 16  # automation lanes sampled per beat, once, for O(1) lookup

def sample_auto(segs, total_beats):
    """Sample an automation lane into an array at RES samples per beat."""
    n = int(total_beats * RES) + RES
    arr = np.zeros(n, dtype=np.float32)
    cur = segs[0][2] if segs else 0.0
    arr[:] = cur
    for beat, kind, v, d, shape in segs:
        i0 = min(n - 1, max(0, int(round(beat * RES))))
        if kind == "set":
            arr[i0:] = v
            cur = v
        else:
            i1 = min(n, max(i0 + 1, int(round((beat + d) * RES))))
            t = np.linspace(0, 1, i1 - i0, endpoint=False)
            v0 = arr[i0]
            if shape == "exp" and v0 > 0 and v > 0:
                seg_vals = v0 * (v / v0) ** t
            elif shape == "smooth":
                tt = t * t * (3 - 2 * t)
                seg_vals = v0 + (v - v0) * tt
            else:
                seg_vals = v0 + (v - v0) * t
            arr[i0:i1] = seg_vals
            arr[i1:] = v
            cur = v
    return arr

def auto_fn(segs, total_beats=600.0):
    arr = sample_auto(segs, total_beats)
    n = len(arr)
    def f(beat):
        return float(arr[min(n - 1, max(0, int(beat * RES)))])
    f.arr = arr
    return f

# ------------------------------------------------------------ tiny font

GLYPHS = {
    "A": "01110100011000111111100011000110001",
    "C": "01110100011000010000100001000101110",
    "D": "11110100011000110001100011000111110",
    "E": "11111100001000011110100001000011111",
    "F": "11111100001000011110100001000010000",
    "G": "01110100011000010111100011000101111",
    "H": "10001100011000111111100011000110001",
    "I": "01110001000010000100001000010001110",
    "L": "10000100001000010000100001000011111",
    "M": "10001110111010110001100011000110001",
    "O": "01110100011000110001100011000101110",
    "P": "11110100011000111110100001000010000",
    "R": "11110100011000111110101001001010001",
    "S": "01111100001000001110000010000111110",
    "T": "11111001000010000100001000010000100",
    "U": "10001100011000110001100011000101110",
    ",": "00000000000000000000001100010001000",
    "1": "00100011000010000100001000010001110",
    "9": "01110100011000101111000010001001100",
    "7": "11111000010001000100010001000100010",
    "5": "11111100001111000001000011000101110",
    " ": "00000000000000000000000000000000000",
    "-": "00000000000000001111000000000000000",
}

def stamp_text(img, text, cx, cy, scale, color, alpha):
    tw = len(text) * 6 * scale
    x = int(cx - tw / 2)
    for ch in text:
        gl = GLYPHS.get(ch.upper())
        if gl:
            for r in range(7):
                for c in range(5):
                    if gl[r * 5 + c] == "1":
                        y0, x0 = cy + r * scale, x + c * scale
                        img[y0:y0 + scale, x0:x0 + scale] = (
                            img[y0:y0 + scale, x0:x0 + scale] * (1 - alpha)
                            + color * alpha)
        x += 6 * scale

# ------------------------------------------------------------ the cloth

DRUM_STEP_X = {}   # column centers
for i in range(N16):
    DRUM_STEP_X[i] = CLOTH_X0 + i * COL_W + COL_W // 2

def build_cloth(events, autos, total_beats):
    Hc = int(total_beats * PX_BEAT) + 200
    cloth = np.zeros((Hc, W, 3), dtype=np.float32)

    def y_of(beat):
        return int(beat * PX_BEAT)

    def x_in_col(beat, tune):
        step = int(round((beat % BAR) / 0.25)) % N16
        return DRUM_STEP_X[step] + (tune - 0.45) * COL_W * 0.75

    def dot(x, y, size, color, gain=1.0):
        x, y = int(x), int(y)
        r = size
        yy, xx = np.mgrid[-r:r + 1, -r:r + 1]
        k = np.exp(-(xx * xx + yy * yy) / (0.35 * r * r + 0.6))
        y0, y1 = max(0, y - r), min(Hc, y + r + 1)
        x0, x1 = max(0, x - r), min(W, x + r + 1)
        if y1 <= y0 or x1 <= x0:
            return
        kk = k[y0 - (y - r):y1 - (y - r), x0 - (x - r):x1 - (x - r)]
        cloth[y0:y1, x0:x1] += kk[..., None] * color * gain

    def line(x0, y0, x1, y1, color, gain=0.3):
        n = int(max(abs(x1 - x0), abs(y1 - y0), 1))
        for t in np.linspace(0, 1, n):
            xi, yi = int(x0 + (x1 - x0) * t), int(y0 + (y1 - y0) * t)
            if 0 <= yi < Hc and 0 <= xi < W:
                cloth[yi, xi] += color * gain

    f_bd = auto_fn(autos["bd_tune"])
    f_sd = auto_fn(autos["sd_tune"])

    # pads first: broad washes behind everything
    for e in events:
        if e["track"] == "pad" and e["kind"] == "chord":
            y0, y1 = y_of(e["beat"]), y_of(e["beat"] + e["dur"])
            col = PADACHE if 56 in e["midis"] else PADW
            cloth[y0:y1, CLOTH_X0:CLOTH_X0 + CLOTH_W] += col * 1.5
        if e["track"] == "drone":
            y0, y1 = y_of(e["beat"]), y_of(e["beat"] + e["dur"])
            cloth[y0:y1, CLOTH_X0 - 26:CLOTH_X0 - 12] += EMBER * 0.5

    # threads: kick (amber, tune-wandering) and snare (cream)
    prev = {}
    for e in sorted(events, key=lambda e: e["beat"]):
        tr, kd = e["track"], e["kind"]
        b, v = e["beat"], e["vel"]
        if kd == "drum":
            nm = e["name"]
            if nm == "BD":
                tune = f_bd(b + 1e-4)
                x, y = x_in_col(b, tune), y_of(b)
                c = AMBER_L + (AMBER_H - AMBER_L) * min(1.0, tune / 0.75)
                if "BD" in prev and b - prev["BD"][2] < 2.5:
                    px, py, _ = prev["BD"]
                    line(px, py, x, y, c, 0.14 + 0.14 * v)
                dot(x, y, 2 + int(4 * v), c, 0.5 + 0.9 * v)
                prev["BD"] = (x, y, b)
            elif nm == "SD":
                tune = f_sd(b + 1e-4)
                x, y = x_in_col(b, tune), y_of(b)
                if "SD" in prev and b - prev["SD"][2] < 1.5:
                    px, py, _ = prev["SD"]
                    line(px, py, x, y, CREAM, 0.1 + 0.12 * v)
                dot(x, y, 2 + int(3.5 * v), CREAM, 0.35 + 1.0 * v)
                prev["SD"] = (x, y, b)
            elif nm == "CH":
                dot(x_in_col(b, 0.45), y_of(b), 2, BRASS, 0.55 + 0.7 * v)
            elif nm == "OH":
                dot(x_in_col(b, 0.45), y_of(b), 4, BRASS_O, 0.5 + 0.5 * v)
            elif nm == "RS":
                dot(x_in_col(b, 0.45), y_of(b), 2, CYAN, 0.6 + 0.7 * v)
            elif nm == "CP":
                x, y = x_in_col(b, 0.4), y_of(b)
                dot(x - 4, y, 2, RED, 0.7)
                dot(x + 4, y, 2, RED, 0.7)
        elif kd == "note" and tr == "bass":
            x = CLOTH_X0 + ((e["midi"] - 21) % 24) / 24 * CLOTH_W
            cloth[y_of(b):y_of(b) + 3, int(x):int(x) + 4] += EMBER * (0.9 + 1.4 * v)
        elif kd == "note" and tr == "lead":
            x = CLOTH_X0 + 30 + (e["midi"] - 57) / 40 * (CLOTH_W - 60)
            y = y_of(b)
            if "LD" in prev and b - prev["LD"][2] < 4:
                px, py, _ = prev["LD"]
                line(px, py, x, y, GOLD, 0.35)
                line(px + 1, py, x + 1, y, GOLD, 0.2)
            dot(x, y, 3, GOLD, 0.8)
            prev["LD"] = (x, y, b)
        elif kd == "note" and tr == "bell":
            x = CLOTH_X0 + 30 + (e["midi"] - 57) / 40 * (CLOTH_W - 60)
            y = y_of(b)
            dot(x, y, 6, STAR, 0.5)
            dot(x, y, 2, STAR, 1.2)
            line(x - 10, y, x + 10, y, STAR, 0.25)
            line(x, y - 10, x, y + 10, STAR, 0.25)
    np.clip(cloth, 0, 6.0, out=cloth)
    return cloth

# ------------------------------------------------------------ audio bands

def load_audio(path):
    data = open(path, "rb").read()
    i = data.find(b"data")
    a = np.frombuffer(data, dtype="<f4", offset=i + 8)
    return a.reshape(-1, 2).mean(axis=1), 48000

def band_env(audio, sr, n_frames):
    hop = sr // FPS
    win = 2048
    sub, high = np.zeros(n_frames), np.zeros(n_frames)
    hann = np.hanning(win)
    freqs = np.fft.rfftfreq(win, 1 / sr)
    m_sub = (freqs > 25) & (freqs < 130)
    m_high = (freqs > 3000) & (freqs < 11000)
    for i in range(n_frames):
        s = i * hop
        seg = audio[s:s + win]
        if len(seg) < win:
            seg = np.pad(seg, (0, win - len(seg)))
        sp = np.abs(np.fft.rfft(seg * hann))
        sub[i] = sp[m_sub].mean()
        high[i] = sp[m_high].mean()
    def norm(x):
        x = x / (np.percentile(x, 96) + 1e-9)
        y = np.copy(x)
        for i in range(1, len(y)):        # fast attack, slow release
            y[i] = max(x[i], y[i - 1] * 0.88)
        return np.clip(y, 0, 1.6)
    return norm(sub), norm(high)

# ------------------------------------------------------------ the render

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--preview", type=str, default=None)
    ap.add_argument("--frames", type=str, default=None,
                    help="a:b — render only this frame range, video-only")
    ap.add_argument("--out", type=str, default=None)
    args = ap.parse_args()

    events, autos = parse_song(SONG)
    total_beats = max(e["beat"] + e["dur"] for e in events) + 8
    print(f"parsed {len(events)} events, {len(autos)} automation lanes")

    audio, sr = load_audio(WAV)
    dur_s = len(audio) / sr
    n_frames = int(dur_s * FPS)
    sub_env, high_env = band_env(audio, sr, n_frames)
    print(f"audio {dur_s:.1f}s -> {n_frames} frames")

    cloth = build_cloth(events, autos, total_beats)
    Hc = cloth.shape[0]
    print(f"cloth woven: {Hc}x{W}")

    f_wow = auto_fn(autos["tape_wow"])
    f_vol = auto_fn(autos["volume"])
    f_cut = auto_fn(autos["bass.cutoff"])
    f_sdt = auto_fn(autos["sd_tune"])

    # birth blooms: every drum/note event, position + color + beat
    births = []
    f_bd = auto_fn(autos["bd_tune"])
    f_sd = auto_fn(autos["sd_tune"])
    for e in events:
        b, v = e["beat"], e["vel"]
        if e["kind"] == "drum":
            tune = f_bd(b + 1e-4) if e["name"] == "BD" else \
                   f_sd(b + 1e-4) if e["name"] == "SD" else 0.45
            step = int(round((b % BAR) / 0.25)) % N16
            x = DRUM_STEP_X[step] + (tune - 0.45) * COL_W * 0.75
            col = {"BD": AMBER_H, "SD": CREAM, "CH": BRASS, "OH": BRASS_O,
                   "RS": CYAN, "CP": RED}[e["name"]]
            births.append((b, x, col * (0.4 + v)))
        elif e["kind"] == "note" and e["track"] in ("lead", "bell"):
            x = CLOTH_X0 + 30 + (e["midi"] - 57) / 40 * (CLOTH_W - 60)
            births.append((b, x, (GOLD if e["track"] == "lead" else STAR) * 0.8))
    births.sort(key=lambda x: x[0])
    birth_beats = np.array([b[0] for b in births])

    # warp background: column lines, precomputed one bar tall, tiled
    warp_row = np.zeros((W,), dtype=np.float32)
    for i in range(N16 + 1):
        x = CLOTH_X0 + i * COL_W
        warp_row[x] = 1.0
    section_beats = [0, 12 * BAR, 28 * BAR, 60 * BAR, 84 * BAR, 124 * BAR, 1e9]

    reveal_t0 = 132 * BAR * 60 / BPM        # bar 132: the pull-back begins
    reveal_t1 = reveal_t0 + 9.0
    NSTRIP = 6
    woven_beats = 148 * BAR                 # the cloth that actually exists
    seg_rows = int(woven_beats * PX_BEAT / NSTRIP)
    strip_scale = min(560 / seg_rows, 182 / CLOTH_W)
    strip_w = int(CLOTH_W * strip_scale)
    strip_h = int(seg_rows * strip_scale)
    gap = (W - NSTRIP * strip_w) // (NSTRIP + 1)
    # precompute the six scaled strips once, on first use
    strips_cache = []
    def get_strips():
        if not strips_cache:
            # 4x4 block-mean first so 1px threads survive, then gather
            pooled_h = Hc // 4
            pooled = cloth[:pooled_h * 4, CLOTH_X0:CLOTH_X0 + (CLOTH_W // 4) * 4]
            pooled = pooled.reshape(pooled_h, 4, CLOTH_W // 4, 4, 3).mean((1, 3))
            ps = strip_scale * 4
            ys = (np.arange(strip_h) / ps).astype(int)
            xs = (np.arange(strip_w) / ps).astype(int).clip(0, pooled.shape[1] - 1)
            seg_p = seg_rows // 4
            for si in range(NSTRIP):
                block = pooled[si * seg_p:(si + 1) * seg_p]
                yy = ys.clip(0, block.shape[0] - 1)
                strips_cache.append(np.clip(block[yy][:, xs] * 3.2, 0, 1.2))
        return strips_cache

    def view_frame(fi):
        t = fi / FPS
        beat = t * BPM / 60.0
        frame = np.zeros((H, W, 3), dtype=np.float32)
        frame += BG

        wow = f_wow(beat)
        rv = 0.0
        if t > reveal_t0:
            rv = min(1.0, (t - reveal_t0) / (reveal_t1 - reveal_t0))
            rv = rv * rv * (3 - 2 * rv)

        if rv < 1.0:
            # live loom view
            yc = int(beat * PX_BEAT)
            y0 = yc - SHUTTLE_Y
            view = np.zeros((H, W, 3), dtype=np.float32)
            sy0, sy1 = max(0, y0), min(Hc, y0 + H)
            dy0 = sy0 - y0
            view[dy0:dy0 + (sy1 - sy0)] = cloth[sy0:sy1]
            # mask the unwoven future: bare warp below the shuttle
            view[SHUTTLE_Y + 1:] = 0.0
            # warp columns, whole height, faint; barely brighter below
            wr = warp_row[None, :, None] * WARP * 1.7
            view += wr
            # section tint: the fabric darkens/warms per era (subtle)
            # birth blooms: events of the last 1.6 beats flare at the shuttle
            i0 = np.searchsorted(birth_beats, beat - 1.6)
            i1 = np.searchsorted(birth_beats, beat + 1e-9)
            flash = 0.0
            for bi in range(i0, i1):
                bb_, bx, bc = births[bi]
                age = (beat - bb_) / 1.6
                gain = (1 - age) ** 2 * 2.2
                if beat - bb_ < 0.22:
                    flash = max(flash, (1 - (beat - bb_) / 0.22) * float(bc.max()))
                by = SHUTTLE_Y - int((beat - bb_) * PX_BEAT)
                r = 9
                ys, xs = int(by), int(bx)
                yy, xx = np.mgrid[-r:r + 1, -r:r + 1]
                k = np.exp(-(xx * xx + yy * yy) / 26.0)
                ya, yb = max(0, ys - r), min(H, ys + r + 1)
                xa, xb = max(0, xs - r), min(W, xs + r + 1)
                if yb > ya and xb > xa:
                    kk = k[ya - (ys - r):yb - (ys - r), xa - (xs - r):xb - (xs - r)]
                    view[ya:yb, xa:xb] += kk[..., None] * bc * gain
            # sub-bass breathes the woven cloth
            view[:SHUTTLE_Y] *= 1.0 + 0.34 * sub_env[min(fi, n_frames - 1)]
            # the shuttle line: cyan readout, shimmering with the highs
            sl = 0.3 + 0.55 * high_env[min(fi, n_frames - 1)] + 0.6 * flash
            view[SHUTTLE_Y - 1:SHUTTLE_Y + 1, CLOTH_X0 - 40:CLOTH_X0 + CLOTH_W + 40] += CYAN * sl * 0.8
            # gutters: the hand-drawn control lines (drawn, then masked)
            for fx, x0, wgut, lo, hi, logmap in (
                    (f_vol, 60, 180, 0.0, 1.0, False),
                    (f_wow, 60, 180, 0.0, 0.6, False),
                    (f_cut, 1040, 180, 300.0, 2800.0, True),
                    (f_sdt, 1040, 180, 0.1, 0.95, False)):
                pts_y = np.arange(0, SHUTTLE_Y + 1, 3)
                for py in pts_y:
                    bb2 = beat + (py - SHUTTLE_Y) / PX_BEAT
                    if bb2 < 0:
                        continue
                    val = fx(bb2)
                    if logmap:
                        u = (np.log(max(val, lo)) - np.log(lo)) / (np.log(hi) - np.log(lo))
                    else:
                        u = (val - lo) / (hi - lo)
                    u = min(1.0, max(0.0, u))
                    px = int(x0 + u * wgut)
                    view[py:py + 2, px:px + 2] += CYAN * 0.28
                # present value: a brighter bead on the line
                val = fx(beat)
                u = ((np.log(max(val, lo)) - np.log(lo)) / (np.log(hi) - np.log(lo))
                     if logmap else (val - lo) / (hi - lo))
                u = min(1.0, max(0.0, u))
                px = int(x0 + u * wgut)
                view[SHUTTLE_Y - 2:SHUTTLE_Y + 3, px - 1:px + 3] += CYAN * 0.9
            # tape wow: the fabric of the image itself wavers
            if wow > 0.08:
                amp = (wow - 0.06) * 26
                rows = np.arange(H)
                shift = (amp * np.sin(rows * 0.045 + t * 2.6)
                         + amp * 0.4 * np.sin(rows * 0.013 - t * 1.7)).astype(int)
                idx = (np.arange(W)[None, :] - shift[:, None]) % W
                view = np.take_along_axis(view, idx[..., None].repeat(3, 2), 1)
            frame = frame * 0 + view
        if rv > 0.0:
            # the reveal: the whole piece as one six-panel tapestry
            tap = np.zeros((H, W, 3), dtype=np.float32)
            tap += BG
            y0 = (H - strip_h) // 2
            for si, strip in enumerate(get_strips()):
                x0 = gap + si * (strip_w + gap)
                tap[y0:y0 + strip_h, x0:x0 + strip_w] += strip * 0.9
            # the clock still ticks: a cyan bead travels the strips
            prog = min(1.0, beat / woven_beats)
            si = min(NSTRIP - 1, int(prog * NSTRIP))
            within = prog * NSTRIP - si
            bx = gap + si * (strip_w + gap)
            by = y0 + int(within * strip_h)
            tap[by:by + 2, bx - 5:bx + strip_w + 5] += CYAN * 0.55
            if t > reveal_t1 + 3:
                stamp_text(tap, "DRUMS, FOR LAURIE", W // 2, H - 64, 2,
                           CREAM * 0.8, min(1.0, (t - reveal_t1 - 3) / 3) * 0.75)
                stamp_text(tap, "THE SCORE IS A PROGRAM - SEED 1975",
                           W // 2, H - 38, 1, CYAN * 0.7,
                           min(1.0, (t - reveal_t1 - 3) / 3) * 0.6)
            frame = frame * (1 - rv) + tap * rv

        # title at the open, woven in and out
        if t < 7.0:
            al = min(1.0, t / 2.5) * min(1.0, (7.0 - t) / 2.0)
            stamp_text(frame, "DRUMS, FOR LAURIE", W // 2, 56, 2, CREAM, al * 0.8)
        # master volume: the hand on the fader dims the light itself
        frame *= 0.35 + 0.65 * f_vol(beat)
        return np.clip(frame, 0, 1)

    if args.preview:
        os.makedirs(os.path.join(REPO, "renders"), exist_ok=True)
        for ts in args.preview.split(","):
            t = float(ts)
            img = (view_frame(int(t * FPS)) * 255).astype(np.uint8)
            p = os.path.join(REPO, "renders", f"loom_{int(t):03d}.png")
            write_png(p, img)
            print("still:", p)
        return

    f0, f1 = 0, n_frames
    out = args.out or OUT
    audio_args = ["-i", WAV, "-c:a", "aac", "-b:a", "192k", "-shortest"]
    if args.frames:
        a, b = args.frames.split(":")
        f0, f1 = int(a), min(n_frames, int(b))
        audio_args = []  # chunks are video-only; audio muxed at concat
    cmd = (["ffmpeg", "-y",
            "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}",
            "-r", str(FPS), "-i", "-"]
           + audio_args
           + ["-c:v", "libx264", "-preset", "medium", "-crf", "18",
              "-pix_fmt", "yuv420p", out])
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    import time
    t_start = time.time()
    for fi in range(f0, f1):
        frame = (view_frame(fi) * 255).astype(np.uint8)
        proc.stdin.write(frame.tobytes())
        if fi % 300 == 0:
            el = time.time() - t_start
            print(f"frame {fi}/{f1}  {(fi - f0) / max(el, 1e-9):.1f} fps", flush=True)
    proc.stdin.close()
    proc.wait()
    print("wrote", out, "frames", f0, "to", f1)

def write_png(path, img):
    import zlib
    h, w = img.shape[:2]
    raw = b"".join(b"\x00" + img[r].tobytes() for r in range(h))
    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d))
    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw))
           + chunk(b"IEND", b""))
    open(path, "wb").write(png)

if __name__ == "__main__":
    main()
