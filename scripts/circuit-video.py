"""The Circuit: Patina's signal path as a video, every window a real tap
of the real signal. No text. Rows are voices; columns are stages
(oscillators bare -> ladder + drive -> tape/effects); the right-hand
column is the output bus. Each stage shows its own wave in phosphor cyan
with the previous stage's wave ghosted behind it in brass, so the
difference between panels is exactly what that part of the circuit did.

    .venv-voice/bin/python scripts/circuit-video.py <logs-dir> <master.wav> <out.mp4> [--start s --end s --frame-png f.png]
Expects <logs-dir>/stems-cd-osc, stems-cd-ladder, stems-full from --render-stems.
"""
import argparse, os, subprocess, sys
import numpy as np, soundfile as sf, librosa
from scipy.ndimage import gaussian_filter
from PIL import Image, ImageDraw, ImageFilter

ap = argparse.ArgumentParser()
ap.add_argument("logs"); ap.add_argument("master"); ap.add_argument("out")
ap.add_argument("--start", type=float, default=0); ap.add_argument("--end", type=float, default=None)
ap.add_argument("--fps", type=int, default=30); ap.add_argument("--frame-png", default=None)
A = ap.parse_args()
FPS = A.fps; W, H = 1600, 900

def load(path):
    y, r = sf.read(path, dtype="float32")
    return (y.mean(axis=1) if y.ndim > 1 else y), r
master, SR = load(A.master)
DUR = len(master) / SR
taps = {}
for stage, d in [("osc", "stems-cd-osc"), ("ladder", "stems-cd-ladder"), ("full", "stems-full")]:
    for name in ["lead", "bass", "bed", "kick"]:
        p = os.path.join(A.logs, d, name + ".wav")
        if os.path.exists(p):
            y, r = load(p); assert r == SR; taps[(name, stage)] = y
print("taps:", sorted(taps), flush=True)

# pitch tracks on the bare oscillator taps (monophonic): cached
cache = os.path.join(A.logs, "circuit-f0.npz")
if os.path.exists(cache):
    F0 = dict(np.load(cache))
else:
    F0 = {}
    hop = int(SR / FPS)
    for name, lo, hi in [("lead", 150, 1400), ("bass", 40, 400)]:
        f0, v, pr = librosa.pyin(taps[(name, "osc")], fmin=lo, fmax=hi, sr=SR, frame_length=4096, hop_length=hop, fill_na=np.nan)
        F0[name] = np.where(v & (pr > 0.4), f0, np.nan); print(name, "tracked", flush=True)
    np.savez(cache, **F0)

# ---------- static panel ----------
rng = np.random.default_rng(11)
def wood(w, h):
    g = rng.normal(0, 1, (h, w)).astype(np.float32)
    g = gaussian_filter(g, (1.5, 40)) * 6 + gaussian_filter(rng.normal(0, 1, (h, w)).astype(np.float32), 8) * 3
    t = 0.5 + 0.5 * np.sin(np.linspace(0, 60, w)[None, :] + g)
    base = np.array([0.20, 0.12, 0.07], np.float32)[None, None, :]
    return np.clip(base * (0.75 + 0.35 * t[..., None]) * 0.55, 0, 1)
BG = wood(W, H)
panel = Image.fromarray((BG * 255).astype(np.uint8))
pd = ImageDraw.Draw(panel, "RGBA")

# layout: rows = lead, bass, bed(+drums); columns = osc, ladder, tape; bus at right
ROWS = {"lead": 150, "bass": 410, "bed": 670}
COLS = {"osc": (70, 400), "ladder": (520, 850), "tape": (970, 1300)}
SC_H = 190
BUS = (1400, 60, 1560, 840)
def scope_rect(row, col):
    x0, x1 = COLS[col]; cy = ROWS[row]
    return (x0, cy - SC_H // 2, x1, cy + SC_H // 2)
def brass_plate(box, r=14):
    x0, y0, x1, y1 = box
    pd.rounded_rectangle((x0 - 10, y0 - 10, x1 + 10, y1 + 10), r + 6, fill=(120, 92, 48, 255), outline=(160, 128, 70, 255), width=2)
    pd.rounded_rectangle((x0 - 6, y0 - 6, x1 + 6, y1 + 6), r + 2, fill=(70, 54, 30, 255))
    pd.rounded_rectangle(box, r, fill=(10, 14, 16, 255), outline=(40, 60, 62, 255), width=2)
    for k in range(1, 4):   # faint reticle
        yy = y0 + (y1 - y0) * k / 4; pd.line((x0 + 8, yy, x1 - 8, yy), fill=(30, 48, 50, 255))
    for k in range(1, 6):
        xx = x0 + (x1 - x0) * k / 6; pd.line((xx, y0 + 8, xx, y1 - 8), fill=(30, 48, 50, 255))
SCOPES = {}
for row in ROWS:
    for col in COLS:
        if row == "bed" and col == "ladder": continue          # the tape head has no ladder
        SCOPES[(row, col)] = scope_rect(row, col); brass_plate(SCOPES[(row, col)])
SCOPES[("drums", "osc")] = (520, ROWS["bed"] - SC_H // 2, 680, ROWS["bed"] + SC_H // 2); brass_plate(SCOPES[("drums", "osc")])
SCOPES[("drums", "tape")] = (700, ROWS["bed"] - SC_H // 2, 850, ROWS["bed"] + SC_H // 2); brass_plate(SCOPES[("drums", "tape")])
brass_plate(BUS, 18)
# screws
for (x0, y0, x1, y1) in list(SCOPES.values()) + [BUS]:
    for (sx, sy) in [(x0 - 4, y0 - 4), (x1 + 4, y0 - 4), (x0 - 4, y1 + 4), (x1 + 4, y1 + 4)]:
        pd.ellipse((sx - 4, sy - 4, sx + 4, sy + 4), fill=(190, 160, 95, 255), outline=(60, 45, 20, 255))
# dials: cutoff dial beside each ladder scope, VU on the bus
DIALS = {"lead": (905, ROWS["lead"] - 40), "bass": (905, ROWS["bass"] - 40)}
for (dx, dy) in DIALS.values():
    pd.ellipse((dx - 34, dy - 34, dx + 34, dy + 34), fill=(120, 92, 48, 255), outline=(180, 150, 90, 255), width=2)
    pd.ellipse((dx - 28, dy - 28, dx + 28, dy + 28), fill=(18, 16, 12, 255))
    for k in range(11):
        a = np.pi * (0.75 + 1.5 * k / 10); pd.line((dx + 22 * np.cos(a), dy + 22 * np.sin(a), dx + 27 * np.cos(a), dy + 27 * np.sin(a)), fill=(200, 170, 100, 255), width=2)
# drive knob (pinned) beside each ladder scope, below the dial
for (dx, dy) in DIALS.values():
    ky = dy + 85
    pd.ellipse((dx - 22, ky - 22, dx + 22, ky + 22), fill=(28, 24, 18, 255), outline=(150, 120, 70, 255), width=2)
    a = np.pi * 0.25; pd.line((dx, ky, dx + 18 * np.cos(a), ky + 18 * np.sin(a)), fill=(80, 230, 240, 255), width=3)
    for k in range(11):
        a = np.pi * (0.75 + 1.5 * k / 10); pd.ellipse((dx + 30 * np.cos(a) - 1.5, ky + 30 * np.sin(a) - 1.5, dx + 30 * np.cos(a) + 1.5, ky + 30 * np.sin(a) + 1.5), fill=(200, 170, 100, 255))
VU = (BUS[0] + 80, BUS[1] + 70)
pd.ellipse((VU[0] - 60, VU[1] - 60, VU[0] + 60, VU[1] + 60), fill=(230, 215, 170, 255), outline=(120, 92, 48, 255), width=3)
for k in range(13):
    a = np.pi * (1.15 + 0.7 * k / 12); c = (200, 40, 30, 255) if k >= 10 else (60, 45, 25, 255)
    pd.line((VU[0] + 44 * np.cos(a), VU[1] + 44 * np.sin(a), VU[0] + 52 * np.cos(a), VU[1] + 52 * np.sin(a)), fill=c, width=2)
# tape reels above each tape scope
REELS = {row: [(COLS["tape"][0] + 90, ROWS[row] - SC_H // 2 - 40), (COLS["tape"][1] - 90, ROWS[row] - SC_H // 2 - 40)] for row in ROWS}
for pair in REELS.values():
    for (rx, ry) in pair:
        pd.ellipse((rx - 30, ry - 30, rx + 30, ry + 30), fill=(40, 36, 30, 255), outline=(150, 120, 70, 255), width=2)
    pd.line((pair[0][0] + 30, pair[0][1] + 8, pair[1][0] - 30, pair[1][1] + 8), fill=(80, 70, 55, 255), width=3)
PANEL = np.asarray(panel).astype(np.float32) / 255.0

# wires as masks (drawn once), lit per frame by that stage's level
def wire_mask(points, width=4):
    im = Image.new("L", (W, H), 0); d = ImageDraw.Draw(im)
    d.line(points, fill=255, width=width, joint="curve")
    return np.asarray(im).astype(np.float32) / 255.0
WIRES = {}
for row in ["lead", "bass"]:
    y = ROWS[row]
    WIRES[(row, "a")] = wire_mask([(COLS["osc"][1] + 10, y), (COLS["ladder"][0] - 10, y)])
    WIRES[(row, "b")] = wire_mask([(COLS["ladder"][1] + 10, y), (COLS["tape"][0] - 10, y)])
    WIRES[(row, "c")] = wire_mask([(COLS["tape"][1] + 10, y), (1360, y), (BUS[0] - 10, (BUS[1] + BUS[3]) // 2)])
yb = ROWS["bed"]; WIRES[("bed", "a")] = wire_mask([(COLS["osc"][1] + 10, yb), (470, yb), (470, yb + 130), (910, yb + 130), (910, yb), (COLS["tape"][0] - 10, yb)])
WIRES[("bed", "c")] = wire_mask([(COLS["tape"][1] + 10, ROWS["bed"]), (1360, ROWS["bed"]), (BUS[0] - 10, (BUS[1] + BUS[3]) // 2)])
WIRES[("drums", "a")] = wire_mask([(680 + 4, ROWS["bed"] + 60), (700 - 4, ROWS["bed"] + 60)])
WIRES[("drums", "c")] = wire_mask([(850 + 10, yb + 60), (910, yb + 60), (910, yb + 20)])
WIRE_SUM = sum(WIRES.values())

# ---------- per-frame signal windows ----------
def rising_zero(y, s, span):
    seg = y[s: s + span]
    z = np.where((seg[:-1] <= 0) & (seg[1:] > 0))[0]
    return s + int(z[0]) if len(z) else s
def window(y, s, n):
    seg = y[s: s + n]
    if len(seg) < n: seg = np.pad(seg, (0, n - len(seg)))
    return seg
def trace(glow, box, seg, col, amp=1.0, ghost=False):
    """Rasterise a waveform into the glow layer, vectorised."""
    x0, y0, x1, y1 = box; w, h = x1 - x0 - 16, y1 - y0 - 16
    n = len(seg); m = max(2, int(w * 3))
    xs = np.linspace(0, n - 1, m); ys = np.interp(xs, np.arange(n), seg)
    px = x0 + 8 + xs / (n - 1) * w; py = (y0 + y1) / 2 - np.clip(ys * amp, -1, 1) * (h / 2 - 4)
    # connect samples so steep edges (the saw's flyback) are solid lines
    dpx = np.diff(px); dpy = np.diff(py); steps = np.maximum(1, np.ceil(np.abs(dpy) / 1.5)).astype(int)
    if ghost: steps = np.minimum(steps, 6)
    idx = np.repeat(np.arange(len(dpx)), steps); frac = (np.arange(steps.sum()) - np.repeat(np.cumsum(steps) - steps, steps)) / np.repeat(steps, steps)
    X = np.clip((px[idx] + dpx[idx] * frac).astype(int), 0, W - 1); Y = np.clip((py[idx] + dpy[idx] * frac).astype(int), 0, H - 1)
    wgt = 1.0 / np.repeat(steps, steps) ** 0.5
    for c in range(3):
        np.add.at(glow[..., c], (Y, X), col[c] * wgt * (0.35 if ghost else 1.0))
CYAN = np.array([0.30, 0.95, 1.00], np.float32); BRASS = np.array([0.95, 0.72, 0.30], np.float32); HEAT = np.array([1.0, 0.45, 0.12], np.float32)
def level(y, s, n=2048):
    return float(np.sqrt(np.mean(window(y, s, n) ** 2)))
def flatness(seg):
    return np.sqrt(np.mean(seg ** 2)) / (np.abs(seg).max() + 1e-6)
def centroid(seg):
    sp = np.abs(np.fft.rfft(seg * np.hanning(len(seg)))); f = np.fft.rfftfreq(len(seg), 1 / SR)
    return float((sp * f).sum() / (sp.sum() + 1e-9))
reel_angle = 0.0
vu_needle = 0.0
heat_state = {"lead": 0.0, "bass": 0.0}

def render_frame(i):
    global reel_angle, vu_needle
    t = i / FPS; s = int(t * SR)
    glow = np.zeros((W, H, 3), np.float32).transpose(1, 0, 2).copy() if False else np.zeros((H, W, 3), np.float32)
    heat = np.zeros((H, W), np.float32)
    lit = np.zeros((H, W), np.float32)
    ov = Image.new("RGBA", (W, H), (0, 0, 0, 0)); od = ImageDraw.Draw(ov)
    # voices: lead and bass through osc -> ladder -> tape
    for row in ["lead", "bass"]:
        f0 = F0[row][min(i, len(F0[row]) - 1)]
        osc, lad, ful = taps[(row, "osc")], taps[(row, "ladder")], taps[(row, "full")]
        lv = level(osc, s)
        if np.isfinite(f0) and lv > 0.003:
            per = int(SR / f0); n = 3 * per; s0 = rising_zero(osc, s, per + 2)
        else:
            n = int(0.02 * SR); s0 = s
        segs = {"osc": window(osc, s0, n), "ladder": window(lad, s0, n), "tape": window(ful, s0, n)}
        prev = None
        for col in ["osc", "ladder", "tape"]:
            seg = segs[col]
            pk = max(np.abs(seg).max(), np.abs(prev).max() if prev is not None else 0.0)
            norm = 0.9 / (pk + 1e-4) if lv > 0.003 else 0.0
            if prev is not None: trace(glow, SCOPES[(row, col)], prev * norm, BRASS, ghost=True)
            trace(glow, SCOPES[(row, col)], seg * norm, CYAN)
            prev = seg
        # heat = how much the ladder flattened the wave, times how loud it is
        h = np.clip((flatness(segs["ladder"]) - flatness(segs["osc"])) * 3.0, 0, 1) * min(1.0, lv * 10) if lv > 0.003 else 0.0
        heat_state[row] = heat_state[row] * 0.85 + h * 0.15
        x0, y0, x1, y1 = SCOPES[(row, "ladder")]
        heat[y0 - 30:y1 + 30, x0 - 30:x1 + 30] += heat_state[row]
        # cutoff dial follows the ladder's brightness
        c = centroid(segs["ladder"]) if lv > 0.003 else 200.0
        a = np.pi * (0.75 + 1.5 * np.clip(np.log2(max(c, 200) / 200) / np.log2(6000 / 200), 0, 1))
        dx, dy = DIALS[row]; od.line((dx, dy, dx + 24 * np.cos(a), dy + 24 * np.sin(a)), fill=(80, 230, 240, 255), width=3)
        # wires lit by stage level
        for key, y in [("a", osc), ("b", lad), ("c", ful)]:
            lit += WIRES[(row, key)] * min(1.0, level(y, s) * 14)
    # the tape head: the sung loop, bare and through the tape
    raw, ful = taps[("bed", "osc")], taps[("bed", "full")]
    n = int(0.03 * SR); s0 = rising_zero(raw, s, int(0.012 * SR))
    lvb = level(raw, s); wr, wf = window(raw, s0, n), window(ful, s0, n)
    nb = 0.9 / (np.abs(wr).max() + 1e-4) if lvb > 0.002 else 0.0
    nf = 0.9 / (max(np.abs(wr).max(), np.abs(wf).max()) + 1e-4) if lvb > 0.002 else 0.0
    trace(glow, SCOPES[("bed", "osc")], wr * nb, CYAN)
    trace(glow, SCOPES[("bed", "tape")], wr * nf, BRASS, ghost=True)
    trace(glow, SCOPES[("bed", "tape")], wf * nf, CYAN)
    lit += WIRES[("bed", "a")] * min(1.0, lvb * 14) + WIRES[("bed", "c")] * min(1.0, level(ful, s) * 14)
    # the 909: bare and driven, a scrolling 60 ms
    draw_, drv = taps[("kick", "osc")], taps[("kick", "full")]
    n = int(0.06 * SR); wr, wd = window(draw_, s, n), window(drv, s, n)
    nd = 0.9 / (np.abs(wr).max() + 1e-4) if level(draw_, s) > 0.002 else 0.0
    nf = 0.9 / (max(np.abs(wr).max(), np.abs(wd).max()) + 1e-4) if level(draw_, s) > 0.002 else 0.0
    trace(glow, SCOPES[("drums", "osc")], wr * nd, CYAN)
    trace(glow, SCOPES[("drums", "tape")], wr * nf, BRASS, ghost=True)
    trace(glow, SCOPES[("drums", "tape")], wd * nf, CYAN)
    lit += WIRES[("drums", "a")] * min(1.0, level(draw_, s) * 14) + WIRES[("drums", "c")] * min(1.0, level(drv, s) * 14)
    # the bus: the master, vertical (time runs downward), 80 ms
    n = int(0.08 * SR); segm = window(master, s, n)
    x0, y0, x1, y1 = BUS; m = int((y1 - y0 - 160) * 2)
    ys = np.linspace(0, n - 1, m); xs = np.interp(ys, np.arange(n), segm)
    PY = np.clip((y0 + 150 + ys / (n - 1) * (y1 - y0 - 160)).astype(int), 0, H - 1)
    PX = np.clip(((x0 + x1) / 2 + np.clip(xs, -1, 1) * ((x1 - x0) / 2 - 12)).astype(int), 0, W - 1)
    for c in range(3): np.add.at(glow[..., c], (PY, PX), CYAN[c] * 0.8)
    # VU needle (ballistic)
    db = 20 * np.log10(level(master, s, 4096) + 1e-6); target = np.clip((db + 30) / 30, 0, 1)
    vu_needle += (target - vu_needle) * 0.25
    a = np.pi * (1.15 + 0.7 * vu_needle)
    od.line((VU[0], VU[1] + 10, VU[0] + 50 * np.cos(a), VU[1] + 50 * np.sin(a)), fill=(30, 20, 15, 255), width=3)
    # reels turn
    reel_angle += 0.09
    for pair in REELS.values():
        for (rx, ry) in pair:
            for k in range(3):
                ang = reel_angle + k * 2 * np.pi / 3
                od.line((rx, ry, rx + 26 * np.cos(ang), ry + 26 * np.sin(ang)), fill=(150, 120, 70, 255), width=3)
    # composite
    img = PANEL.copy()
    core = np.clip(glow, 0, 2)
    soft = gaussian_filter(core[::2, ::2], (3, 3, 0)); soft = np.repeat(np.repeat(soft, 2, axis=0), 2, axis=1)[:H, :W]
    img += core * 0.9 + soft * 0.7
    img += (gaussian_filter(heat[::4, ::4], 10).repeat(4, axis=0).repeat(4, axis=1)[:H, :W, None]) * HEAT * 0.9
    img += (lit * 0.9 + gaussian_filter(lit, 3) * 0.6)[..., None] * CYAN * 0.7 + (WIRE_SUM * 0.12)[..., None] * BRASS
    o = np.asarray(ov).astype(np.float32) / 255.0
    img = img * (1 - o[..., 3:4]) + o[..., :3] * o[..., 3:4]
    img += rng.normal(0, 0.004, (H, W, 1)).astype(np.float32)
    img = 1 - np.exp(-1.5 * np.clip(img, 0, None))
    return (np.clip(img, 0, 1) ** (1 / 1.8) * 255).astype(np.uint8)

if A.frame_png:
    i0 = int(A.start * FPS)
    for i in range(max(0, i0 - 20), i0 + 1): frame = render_frame(i)
    Image.fromarray(frame).save(A.frame_png); print("wrote", A.frame_png); sys.exit(0)
t_end = A.end if A.end else DUR
i0, i1 = int(A.start * FPS), int(t_end * FPS)
cmd = ["ffmpeg", "-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}", "-r", str(FPS), "-i", "-",
       "-ss", str(A.start), "-t", str(t_end - A.start), "-i", A.master,
       "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-shortest", A.out]
proc = subprocess.Popen(cmd, stdin=subprocess.PIPE)
for i in range(i0, i1):
    proc.stdin.write(render_frame(i).tobytes())
    if (i - i0) % (FPS * 10) == 0: print(f"  {i / FPS:6.1f}s", flush=True)
proc.stdin.close(); proc.wait(); print("wrote", A.out)
