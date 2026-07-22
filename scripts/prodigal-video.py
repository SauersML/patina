#!/usr/bin/env python
"""Drone base for The Prodigal Program — the land as stop motion.

48 agricultural plots were flown at 10:00, 12:00 and 15:00. Every
plot's three captures share one identical center crop, hard-cut
back-to-back on the beat clock — the ground holds still and the
shadows JUMP, sun swinging like a stop-motion animator's lamp. The
sway is a ping-pong (10:00 - 12:00 - 15:00 - 12:00) so the light
rocks instead of snapping back: one full sway every two beats once
the groove lands, half speed during the opening breath and the final
dissolve, and the tape-stop stretches the last steps into a freeze.

One plot per 8-beat bar, hard cuts on the bar. The grade is a tritone
gradient map riding the song's form:

    A breath     0-16    indigo / violet          (empty grass)
    B groove    16-48    deep purple / ultramarine (tilled rows)
    C themeB    48-80    midnight / cobalt-cyan    (roads, fences)
    D wormhole  80-104   crimson snap              (tire-track loops)
    E bloom    104-128   magenta bloom             (circular scrub)
    F dissolve 128-152   violet ash, tape-stop     (back to nothing)

No text, no sound — this is only the bed the rest of the video will
sit on. Encoding is segmented mpegts with frame-accurate resume:
something on this machine reaps long renders every ~150s, so run it
in a relaunch-until-done loop and no frame is ever paid for twice.

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
SWAY = [0, 1, 2, 1]         # ping-pong through the times of day

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
T_STOP = DUR - 3.2          # tape-stop: the sway decelerates into freeze
T_FADE = DUR - 2.6          # and everything sinks to black

LUM = np.array([0.2126, 0.7152, 0.0722], np.float32)


def smoothstep(x):
    x = np.clip(x, 0.0, 1.0)
    return x * x * (3.0 - 2.0 * x)


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
                p = os.path.join(SRC, f"{name}__time_{tod}.png")
                im = Image.open(p).convert("RGB").crop((0, 224, 1024, 800))
                im = im.resize((W, H), Image.BILINEAR)
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


def sway_steps(beat):
    """Cumulative time-of-day steps: per beat in the breath, per
    half-beat under the groove, per beat again for the dissolve."""
    if beat < 16:
        return int(beat)
    if beat < 128:
        return 16 + int((beat - 16) * 2)
    return 16 + 224 + int(beat - 128)


def render_frame(f):
    t = f / FPS
    beat = t / SPB
    # tape-stop: the sway decelerates into a freeze while the tape dies
    if t > T_STOP:
        u = (t - T_STOP) / (DUR - T_STOP)
        bs = T_STOP / SPB
        beat = bs + (beat - bs) * (1.0 - u) * (1.0 - u)
    seg = min(int(beat // BEATS_PER_SEG), NSEG - 1)
    tod = SWAY[sway_steps(beat) % 4]

    imgs, ys = CACHE.get(PLOTS[seg])
    img, y = imgs[tod], ys[tod]

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


# vignette, once
yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
rr = np.sqrt(((xx / W - 0.5) * 2) ** 2 + ((yy / H - 0.5) * 1.6) ** 2)
VIG = (1.0 - 0.42 * np.clip(rr, 0, 1.25) ** 2.2)[..., None].astype(np.float32)
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
