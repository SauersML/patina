#!/usr/bin/env python
"""Drone base for The Prodigal Program — the land remembers the light.

48 agricultural plots were flown at 10:00, 12:00 and 15:00; this film
holds one plot per 8-beat bar and crossfades its three captures so the
daylight visibly crawls across the field while the camera drifts. The
grade is a tritone gradient map whose palette rides the song's form:

    A breath     0-16    indigo / violet          (empty grass)
    B groove    16-48    deep purple / ultramarine (tilled rows)
    C themeB    48-80    midnight / cobalt-cyan    (roads, fences)
    D wormhole  80-104   crimson snap              (tire-track loops)
    E bloom    104-128   magenta bloom             (circular scrub)
    F dissolve 128-152   violet ash, tape-stop     (back to nothing)

No text, no sound — this is only the bed the rest of the video will
sit on. Encoding is segmented mpegts (survives the kernel killing
ffmpeg under memory pressure), same as nevernot-video.py.

    .venv-voice/bin/python scripts/prodigal-video.py
    -> renders/prodigal-base.mp4
"""
import os
import subprocess

import numpy as np
from PIL import Image

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = "/Users/user/Downloads/image sources/drone-lsr-images"
OUT = os.path.join(REPO, "renders", "prodigal-base.mp4")

W, H, FPS = 1920, 1080, 24
BPM = 72.0
SPB = 60.0 / BPM
DUR = 130.586667            # matches renders/prodigal-program.wav
TOTAL = int(round(DUR * FPS))
BEATS_PER_SEG = 8
TODS = ["1000", "1200", "1500"]

# one plot per bar-pair, in the order the song tells the story
PLOTS = [
    "plot_2_18", "plot_5_1",                            # A  breath
    "plot_3_13", "plot_2_7", "plot_3_5", "plot_2_9",    # B  groove
    "plot_4_11", "plot_3_11", "plot_4_4", "plot_3_1",   # C  shibboleth
    "plot_2_2", "plot_3_2", "plot_3_9",                 # D  wormhole
    "plot_4_12", "plot_2_3", "plot_5_10",               # E  bloom
    "plot_4_20", "plot_3_20", "plot_2_17",              # F  dissolve
]
NSEG = len(PLOTS)

# (beat, (shadow, mid, highlight), exposure) — lerped in beat space
GRADE = [
    (0,   ((4, 3, 14),  (44, 32, 96),   (136, 120, 186)), 0.74),
    (15,  ((4, 3, 14),  (44, 32, 96),   (136, 120, 186)), 0.78),
    (23,  ((12, 6, 38), (52, 58, 178),  (172, 196, 240)), 0.97),
    (46,  ((12, 6, 38), (52, 58, 178),  (172, 196, 240)), 0.97),
    (55,  ((3, 10, 28), (22, 92, 170),  (150, 225, 238)), 1.00),
    (79,  ((3, 10, 28), (22, 92, 170),  (150, 225, 238)), 1.00),
    (83,  ((16, 3, 10), (158, 18, 44),  (255, 116, 72)),  1.05),
    (101, ((16, 3, 10), (158, 18, 44),  (255, 116, 72)),  1.05),
    (110, ((26, 7, 34), (198, 44, 128), (255, 182, 198)), 1.16),
    (125, ((26, 7, 34), (198, 44, 128), (255, 182, 198)), 1.12),
    (139, ((8, 6, 16),  (64, 50, 96),   (142, 134, 160)), 0.90),
    (160, ((8, 6, 16),  (64, 50, 96),   (142, 134, 160)), 0.72),
]
XFADE = 1.1                 # s, plot-to-plot dissolve
T_STOP = DUR - 3.2          # tape-stop: motion decelerates into freeze
T_FADE = DUR - 2.6          # and everything sinks to black


def smoothstep(x):
    x = np.clip(x, 0.0, 1.0)
    return x * x * (3.0 - 2.0 * x)


class PlotCache:
    """The 3 times-of-day of the last few plots, luminance-normalized."""

    def __init__(self, cap=3):
        self.cap, self.d, self.order = cap, {}, []

    def get(self, name):
        if name not in self.d:
            imgs = []
            for tod in TODS:
                p = os.path.join(SRC, f"{name}__time_{tod}.png")
                imgs.append(np.asarray(Image.open(p).convert("RGB"),
                                       dtype=np.float32) / 255.0)
            y = imgs[1] @ np.array([0.2126, 0.7152, 0.0722], np.float32)
            lo, hi = np.percentile(y, [2.0, 98.0])
            self.d[name] = (imgs, float(lo), float(hi))
            self.order.append(name)
            if len(self.order) > self.cap:
                del self.d[self.order.pop(0)]
        return self.d[name]


CACHE = PlotCache()
RNGS = [np.random.default_rng(7000 + i) for i in range(NSEG)]
LUM = np.array([0.2126, 0.7152, 0.0722], np.float32)

# per-segment Ken Burns paths, deterministic
KB = []
for i in range(NSEG):
    r = RNGS[i]
    z0, z1 = 1.05 + r.uniform(0, 0.05), 1.13 + r.uniform(0, 0.06)
    if i % 2:
        z0, z1 = z1, z0
    cy0, cy1 = r.uniform(0.34, 0.66), r.uniform(0.34, 0.66)
    cx0, cx1 = r.uniform(0.46, 0.54), r.uniform(0.46, 0.54)
    KB.append((z0, z1, cx0, cx1, cy0, cy1))

# vignette, once
yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
rr = np.sqrt(((xx / W - 0.5) * 2) ** 2 + ((yy / H - 0.5) * 1.6) ** 2)
VIG = (1.0 - 0.42 * np.clip(rr, 0, 1.25) ** 2.2)[..., None].astype(np.float32)
del yy, xx, rr


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


def seg_frame(seg, p):
    """Raw graded-ready RGB (H,W,3 float) for segment `seg` at progress p."""
    imgs, lo, hi = CACHE.get(PLOTS[seg])
    # daylight crawls 10:00 -> 12:00 -> 15:00 across the bar-pair
    if p < 0.5:
        a, b, t = imgs[0], imgs[1], smoothstep(p * 2)
    else:
        a, b, t = imgs[1], imgs[2], smoothstep((p - 0.5) * 2)
    img = a + (b - a) * np.float32(t)

    z0, z1, cx0, cx1, cy0, cy1 = KB[seg]
    e = smoothstep(p)
    z = z0 + (z1 - z0) * e
    cx = (cx0 + (cx1 - cx0) * e) * 1024
    cy = (cy0 + (cy1 - cy0) * e) * 1024
    ww, hh = 1024.0 / z, 576.0 / z
    x0 = int(np.clip(cx - ww / 2, 0, 1024 - ww))
    y0 = int(np.clip(cy - hh / 2, 0, 1024 - hh))
    crop = img[y0:y0 + int(hh), x0:x0 + int(ww)]
    pil = Image.fromarray((np.clip(crop, 0, 1) * 255).astype(np.uint8))
    out = np.asarray(pil.resize((W, H), Image.BILINEAR), np.float32) / 255.0
    return out, lo, hi


def render_frame(f):
    t = f / FPS
    beat = t / SPB
    # tape-stop: the film decelerates into a freeze while the tape dies
    if t > T_STOP:
        u = (t - T_STOP) / (DUR - T_STOP)
        bs = T_STOP / SPB
        beat = bs + (beat - bs) * (1.0 - u) * (1.0 - u)
    seg = min(int(beat // BEATS_PER_SEG), NSEG - 1)
    p = np.clip(beat / BEATS_PER_SEG - seg, 0.0, 1.0)

    img, lo, hi = seg_frame(seg, p)
    # dissolve between plots
    tin = (beat - seg * BEATS_PER_SEG) * SPB
    if seg > 0 and tin < XFADE / 2:
        prev, plo, phi = seg_frame(seg - 1, 1.0)
        w = smoothstep(0.5 + tin / XFADE)
        img = prev + (img - prev) * np.float32(w)
        lo, hi = plo + (lo - plo) * w, phi + (hi - phi) * w
    elif seg < NSEG - 1:
        tout = BEATS_PER_SEG * SPB - tin
        if tout < XFADE / 2:
            nxt, nlo, nhi = seg_frame(seg + 1, 0.0)
            w = smoothstep(0.5 + tout / XFADE)
            img = nxt + (img - nxt) * np.float32(w)
            lo, hi = nlo + (lo - nlo) * w, nhi + (hi - nhi) * w

    # normalized field luminance -> tritone gradient map
    y = img @ LUM
    y = np.clip((y - lo) / max(hi - lo, 1e-6), 0.0, 1.0)
    y = smoothstep(y)                       # gentle S-curve
    pal, expo = grade_at(t / SPB)
    (s, m, hgh) = pal
    ramp = np.float32([0.0, 0.5, 1.0])
    graded = np.empty((H, W, 3), np.float32)
    for c in range(3):
        graded[..., c] = np.interp(
            y, ramp, [s[c] / 255.0, m[c] / 255.0, hgh[c] / 255.0])
    graded = graded * 0.9 + img * 0.1       # a breath of the real field

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
