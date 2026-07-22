#!/usr/bin/env python
"""Drone base for The Prodigal Program — 600 fields, none seen twice.

609 agricultural plots were flown at 10:00, 12:00 and 15:00. Each
plot spends its three captures as three consecutive stop-motion
steps — one identical center crop, hard cuts, the ground still and
the shadows crawling morning to afternoon — and then it is gone for
good: no image ever appears twice. The step clock is a musical
subdivision that rides the form:

    A breath     0-16    eighths     calm plots     violet
    B groove    16-48    sixteenths  working rows   electric purple
    C themeB    48-80    sixteenths  roads, fences  bright cobalt
    D wormhole  80-104   32nd strobe loudest plots  hot crimson
    E bloom    104-128   triplets    high texture   vivid magenta
    F dissolve 128-       eighths     calmest land   fading violet

Plots are ranked by a texture-drama score (luminance spread + edge
energy, cached in renders/prodigal-plotstats.json) so the wormhole
strobes through the most violent textures and the film ends on the
emptiest field it knows. Grade is a bright tritone gradient map with
a post-map saturation push. The tape-stop stretches the final steps
into a freeze while everything sinks to black.

No text, no sound — this is the bed the rest of the video sits on.
Encoding is segmented mpegts with frame-accurate resume: something on
this machine reaps long renders every ~150s, so run it in a
relaunch-until-done loop and no frame is ever paid for twice.

    .venv-voice/bin/python scripts/prodigal-video.py
    -> renders/prodigal-base.mp4
"""
import json
import os
import subprocess

import numpy as np
from PIL import Image

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC_PNG = "/Users/user/Downloads/image sources/drone-lsr-images"
SRC_JPG = os.path.expanduser("~/Downloads/drone-lsr-images-rest")
STATS = os.path.join(REPO, "renders", "prodigal-plotstats.json")
OUT = os.path.join(REPO, "renders", "prodigal-base.mp4")

W, H, FPS = 1920, 1080, 24
BPM = 72.0
SPB = 60.0 / BPM
DUR = 130.586667            # matches renders/prodigal-program.wav
TOTAL = int(round(DUR * FPS))
TODS = ["1000", "1200", "1500"]

# (end_beat, steps_per_beat) — the stop-motion clock per section
RATES = [(16, 2), (48, 4), (80, 4), (104, 8), (128, 6), (1e9, 2)]

# (beat, (shadow, mid, highlight), exposure) — lerped in beat space
GRADE = [
    (0,   ((10, 4, 28),  (96, 52, 190),  (198, 160, 255)), 0.82),
    (15,  ((10, 4, 28),  (96, 52, 190),  (198, 160, 255)), 0.88),
    (23,  ((18, 8, 52),  (88, 72, 230),  (200, 210, 255)), 1.00),
    (46,  ((18, 8, 52),  (88, 72, 230),  (200, 210, 255)), 1.00),
    (55,  ((4, 14, 40),  (36, 120, 225), (170, 240, 250)), 1.05),
    (79,  ((4, 14, 40),  (36, 120, 225), (170, 240, 250)), 1.05),
    (83,  ((24, 4, 14),  (200, 26, 60),  (255, 140, 90)),  1.10),
    (101, ((24, 4, 14),  (200, 26, 60),  (255, 140, 90)),  1.10),
    (110, ((36, 8, 46),  (232, 56, 160), (255, 196, 220)), 1.20),
    (125, ((36, 8, 46),  (232, 56, 160), (255, 196, 220)), 1.16),
    (139, ((12, 8, 26),  (96, 70, 150),  (176, 160, 210)), 0.95),
    (160, ((12, 8, 26),  (96, 70, 150),  (176, 160, 210)), 0.78),
]
SAT = 1.18                  # post-map saturation push
T_STOP = DUR - 3.2          # tape-stop: steps decelerate into freeze
T_FADE = DUR - 2.6          # and everything sinks to black

LUM = np.array([0.2126, 0.7152, 0.0722], np.float32)


def smoothstep(x):
    x = np.clip(x, 0.0, 1.0)
    return x * x * (3.0 - 2.0 * x)


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
    """~260 plots, unique, banded by drama to match the sections."""
    scores = drama_scores()
    ranked = sorted(scores, key=scores.get)      # calm -> violent
    n = len(ranked)
    rng = np.random.default_rng(72)
    used = set()

    def pick(lo, hi, k, order=None):
        band = [p for p in ranked[int(lo * n):int(hi * n)] if p not in used]
        while len(band) < k:                     # widen if exhausted
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
    seq += pick(0.05, 0.25, 11, order="asc")     # A: waking up
    seq += pick(0.35, 0.60, 43)                  # B: the groove works
    seq += pick(0.50, 0.75, 42)                  # C: roads and fences
    seq += pick(0.85, 1.00, 64)                  # D: the loudest land
    seq += pick(0.65, 0.88, 48)                  # E: bloom
    seq += pick(0.00, 0.30, 52, order="desc")    # F: emptying out
    return seq


SEQ = running_order()


def step_at(beat):
    """Cumulative stop-motion steps at `beat` under the section clock."""
    steps, prev = 0.0, 0.0
    for end, rate in RATES:
        if beat <= end:
            return int(steps + (beat - prev) * rate)
        steps += (end - prev) * rate
        prev = end
    return int(steps)


class PlotCache:
    """Per plot: the 3 times of day, identically center-cropped to
    1920x1080, plus their normalized+curved luminance planes."""

    def __init__(self, cap=2):
        self.cap, self.d, self.order = cap, {}, []

    def get(self, name):
        if name not in self.d:
            imgs, ys = [], []
            lo = hi = None
            for tod in TODS:
                im = Image.open(plot_path(name, tod)).convert("RGB")
                im = im.crop((0, 224, 1024, 800)).resize((W, H),
                                                         Image.BILINEAR)
                a = np.asarray(im, dtype=np.float32) / 255.0
                y = a @ LUM
                if lo is None:
                    # one shared normalization so the flicker keeps the
                    # real exposure jumps between times of day
                    lo, hi = np.percentile(y, [2.0, 98.0])
                imgs.append(a)
                ys.append(y)
            for i in range(3):
                ys[i] = smoothstep(np.clip(
                    (ys[i] - lo) / max(hi - lo, 1e-6), 0.0, 1.0))
            self.d[name] = (imgs, ys)
            self.order.append(name)
            if len(self.order) > self.cap:
                del self.d[self.order.pop(0)]
        return self.d[name]


CACHE = PlotCache()


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
    beat = t / SPB
    # tape-stop: the step clock decelerates into a freeze
    if t > T_STOP:
        u = (t - T_STOP) / (DUR - T_STOP)
        bs = T_STOP / SPB
        beat = bs + (beat - bs) * (1.0 - u) * (1.0 - u)
    step = step_at(beat)
    plot = min(step // 3, len(SEQ) - 1)
    imgs, ys = CACHE.get(SEQ[plot])
    img, y = imgs[step % 3], ys[step % 3]

    pal, expo = grade_at(t / SPB)
    (s, m, hgh) = pal
    ramp = np.float32([0.0, 0.5, 1.0])
    graded = np.empty((H, W, 3), np.float32)
    for c in range(3):
        graded[..., c] = np.interp(
            y, ramp, [s[c] / 255.0, m[c] / 255.0, hgh[c] / 255.0])
    graded = graded * 0.93 + img * 0.07     # a breath of the real field

    lum = (graded @ LUM)[..., None]
    graded = lum + (graded - lum) * SAT     # the color curve push

    # bar-length luminance breathing, deeper while the drone bed is alone
    breathe = 0.05 if (t / SPB) < 16 or (t / SPB) >= 128 else 0.025
    graded *= expo * (1.0 + breathe * np.sin(2 * np.pi * (t / SPB) / 8.0))
    graded *= VIG

    # analog fringe: red and blue drift a hair apart
    graded[..., 0] = np.roll(graded[..., 0], 2, axis=1)
    graded[..., 2] = np.roll(graded[..., 2], -2, axis=1)

    rng = np.random.default_rng(f)
    ga = 0.016 if t < T_STOP else 0.016 + 0.02 * (t - T_STOP) / (DUR - T_STOP)
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
        if f % 240 == 0:
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
