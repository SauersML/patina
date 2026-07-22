#!/usr/bin/env python
"""Drone base for The Prodigal Program — 600 fields, none seen twice.

609 agricultural plots were flown at 10:00, 12:00 and 15:00. Each
plot spends its three captures as three consecutive stop-motion
steps — one identical center crop, hard cuts, the ground still and
the shadows crawling morning to afternoon — and then it is gone for
good: no image ever appears twice. The step clock is a musical
subdivision that rides the form:

    A breath     0-16    sixteenths   calm plots     violet
    B groove    16-48    32nds        working rows   electric purple
    C themeB    48-80    32nds        roads, fences  bright cobalt
    D wormhole  80-104   64th strobe  loudest plots  hot crimson
    E bloom    104-128   32nd trips   high texture   vivid magenta
    F dissolve 128-       sixteenths   calmest land   fading violet

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
RATES = [(16, 4), (48, 8), (80, 8), (104, 16), (128, 12), (1e9, 4)]

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
SAT = 1.22                  # post-map saturation push
TONE_S = 1.8                # gentle S on top of the equalized histogram
TONE_GAMMA = 1.0            # equalization already owns the distribution
T_STOP = DUR - 3.2          # tape-stop: steps decelerate into freeze
T_FADE = DUR - 2.6          # and everything sinks to black

LUM = np.array([0.2126, 0.7152, 0.0722], np.float32)
EQ_P = np.linspace(0.0, 1.0, 65)


def smoothstep(x):
    x = np.clip(x, 0.0, 1.0)
    return x * x * (3.0 - 2.0 * x)


def tone_curve(y):
    """Levels + curves: gamma opens the mids, then a tanh S-curve with
    a steep mid slope crushes the toe and rolls the shoulder."""
    y = np.clip(y, 0.0, 1.0) ** TONE_GAMMA
    return 0.5 + np.tanh(TONE_S * (y - 0.5)) / (2.0 * np.tanh(TONE_S / 2.0))


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
    seq += pick(0.05, 0.30, 22, order="asc")     # A: waking up
    seq += pick(0.30, 0.60, 86)                  # B: the groove works
    seq += pick(0.45, 0.78, 86)                  # C: roads and fences
    seq += pick(0.78, 1.00, 128)                 # D: the loudest land
    seq += pick(0.60, 0.88, 96)                  # E: bloom
    seq += pick(0.00, 0.35, 60, order="desc")    # F: emptying out
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
            cdf_q = None
            for tod in TODS:
                im = Image.open(plot_path(name, tod)).convert("RGB")
                im = im.crop((0, 224, 1024, 800)).resize((W, H),
                                                         Image.BILINEAR)
                a = np.asarray(im, dtype=np.float32) / 255.0
                y = a @ LUM
                if cdf_q is None:
                    # one shared histogram equalization for the triplet:
                    # spreads pixel mass across the WHOLE ramp so every
                    # hue zone owns real area in every frame, while the
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
# Prophet-5 schematic pages (rendered by scripts/prodigal-schem.sh into
# renders/prodigal-schem/*.png, dark-on-light) flash for the first two
# frames after every plot change: two different crops, inverted to
# white-on-black, graded through whatever palette owns that beat.
# Crop index = 2*plot or 2*plot+1, so like the fields, none repeats.
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
    """Inverted, contrast-shaped luminance plane of schematic crop
    `crop_idx` — deterministic, unique per index."""
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


def warped_beat(f):
    t = f / FPS
    beat = t / SPB
    # tape-stop: the step clock decelerates into a freeze
    if t > T_STOP:
        u = (t - T_STOP) / (DUR - T_STOP)
        bs = T_STOP / SPB
        beat = bs + (beat - bs) * (1.0 - u) * (1.0 - u)
    return beat


def plot_at(f):
    return min(step_at(warped_beat(f)) // 3, len(SEQ) - 1)


def render_frame(f):
    t = f / FPS
    step = step_at(warped_beat(f))
    plot = min(step // 3, len(SEQ) - 1)

    # the first two frames after a plot change are circuit flashes:
    # two different schematic crops, never the same crop twice
    flash = None
    if SCHEM_PAGES and f >= 2 and plot > 0:
        if plot != plot_at(f - 1):
            flash = flash_y(2 * plot)
        elif plot != plot_at(f - 2):
            flash = flash_y(2 * plot + 1)
    if flash is None:
        imgs, ys = CACHE.get(SEQ[plot])
        img, y = imgs[step % 3], ys[step % 3]
    else:
        img, y = None, flash

    pal, expo = grade_at(t / SPB)
    anchors = np.float32(pal) / 255.0           # (5, 3): hue-by-lightness
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
    graded = zone * (0.22 + 1.05 * y)[..., None]
    graded += (y ** 4)[..., None] * (anchors[-1] * 0.45)  # specular lift
    if img is not None:
        graded = graded * 0.93 + img * 0.07  # a breath of the real field

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
