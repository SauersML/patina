#!/usr/bin/env python3
"""Strobe Engine — the music video.

A software-3D phosphor renderer: glowing points splatted additively,
with persistence trails and box-blur bloom, streamed frame-by-frame into
ffmpeg (flat RAM — this machine has 8 GB). The scene is cut to the
song's beat grid and pulsed by band energies measured from the actual
render, so the video and the mix can't drift apart.

Sections (beats @ 128 bpm):
    0-16    memory      amber particle drift, tape wobble
   16-32    vamp        cyan floor grid fades in
   32-48    groove      grid pulses with the kick, camera locks to tempo
   48-80    drop 1      ring tunnel + riff ribbon
   80-92    answer      bell strikes = expanding bursts
   92-96    walkdown    dim, slow
   96-128   bloom       violet nebula, camera drifts up, wide
  128-142.5 build       rings return, red shift, riser lines climb
  142.5-144 gap         black, frozen
  144-208   drop 2      full tunnel: grid + ceiling + rings + ribbon,
                        kick strobes; 176-192 lift = gold, camera rises
  208-240   outro       elements fall away, amber drift, wow warble out

Usage:
  python3 scripts/strobe-video.py                    # full render
  python3 scripts/strobe-video.py --preview 7,35,60  # PNG stills (secs)
"""

import argparse
import os
import struct
import subprocess
import sys

import numpy as np

FEATURE_CACHE = "/tmp/strobe_features.npz"

BPM = 128.0
BEAT = 60.0 / BPM
FPS = 30
W, H = 960, 540
FOCAL = 420.0
WAV = "renders/strobe-engine.wav"
OUT = "renders/strobe-engine.mp4"

BELL_STRIKES = [82, 82.75, 83.5, 90, 90.75, 91.5, 120, 124]


def load_audio(path):
    data = open(path, "rb").read()
    f = data.find(b"fmt ")
    sr = struct.unpack("<I", data[f + 12 : f + 16])[0]
    nch = struct.unpack("<H", data[f + 10 : f + 12])[0]
    i = data.find(b"data")
    n = struct.unpack("<I", data[i + 4 : i + 8])[0]
    a = np.frombuffer(data, dtype="<f4", count=n // 4, offset=i + 8)
    return a.reshape(-1, nch).mean(axis=1), sr


def band_envelopes(mono, sr, n_frames):
    """Per-video-frame energy in four bands, normalized, punchy-smoothed."""
    hop = sr / FPS
    win = 4096
    bands = {"kick": (40, 110), "bass": (110, 240), "riff": (240, 900), "high": (5000, 12000)}
    freqs = np.fft.rfftfreq(win, 1 / sr)
    masks = {k: (freqs >= lo) & (freqs < hi) for k, (lo, hi) in bands.items()}
    hann = np.hanning(win).astype(np.float32)
    out = {k: np.zeros(n_frames, np.float32) for k in bands}
    for fi in range(n_frames):
        c = int(fi * hop)
        seg = mono[max(0, c - win // 2) : c + win // 2]
        if len(seg) < win:
            seg = np.pad(seg, (0, win - len(seg)))
        spec = np.abs(np.fft.rfft(seg * hann))
        for k in bands:
            out[k][fi] = np.sqrt(np.mean(spec[masks[k]] ** 2))
    for k in out:
        e = out[k]
        e /= np.percentile(e, 97) + 1e-9
        sm = np.empty_like(e)
        acc = 0.0
        for i in range(len(e)):  # instant attack, ~150 ms release
            acc = max(e[i], acc * 0.86)
            sm[i] = acc
        out[k] = np.clip(sm, 0, 1.4)
    return out


def curve(b, xs, ys):
    return np.interp(b, xs, ys)


def color_curve(b, xs, cols):
    cols = np.array(cols, np.float32)
    return np.array([np.interp(b, xs, cols[:, i]) for i in range(3)], np.float32)


# palette keys: structure color (grid/rings) and accent (ribbon/particles)
PAL_B = [0, 14, 20, 46, 91, 97, 126, 129, 142, 144, 174, 178, 190, 194, 207, 214, 224, 240]
PAL_STRUCT = [
    (1.0, 0.55, 0.22), (1.0, 0.55, 0.22), (0.15, 0.8, 1.0), (0.15, 0.85, 1.0),
    (0.15, 0.85, 1.0), (0.6, 0.35, 1.0), (0.65, 0.35, 1.0), (1.0, 0.4, 0.18),
    (1.0, 0.32, 0.15), (0.15, 0.85, 1.0), (0.15, 0.85, 1.0), (1.0, 0.78, 0.3),
    (1.0, 0.82, 0.35), (0.15, 0.85, 1.0), (0.15, 0.85, 1.0), (0.6, 0.65, 0.9),
    (1.0, 0.55, 0.22), (1.0, 0.5, 0.2),
]
PAL_ACCENT = [
    (1.0, 0.65, 0.3), (1.0, 0.65, 0.3), (1.0, 0.6, 0.25), (1.0, 0.55, 0.2),
    (1.0, 0.55, 0.2), (1.0, 0.4, 0.85), (0.95, 0.4, 0.9), (1.0, 0.5, 0.2),
    (1.0, 0.45, 0.2), (1.0, 0.5, 0.2), (1.0, 0.5, 0.2), (1.0, 0.92, 0.6),
    (1.0, 0.95, 0.7), (1.0, 0.5, 0.2), (1.0, 0.5, 0.2), (1.0, 0.6, 0.3),
    (1.0, 0.62, 0.28), (1.0, 0.55, 0.22),
]

SPEED_B = [0, 16, 32, 48, 80, 92, 96, 128, 142.5, 143.9, 144, 176, 192, 208, 216, 240]
SPEED_V = [0.6, 1.0, 1.6, 2.2, 2.0, 1.2, 0.8, 2.2, 4.0, 0.0, 3.2, 2.6, 3.0, 2.4, 1.0, 0.4]

GAIN_B = [0, 91.5, 93, 95.5, 96, 142.5, 142.6, 143.9, 144, 222, 232, 237, 240]
GAIN_V = [1, 1, 0.55, 0.6, 1, 1, 0.07, 0.07, 1, 1, 0.55, 0.12, 0.0]

W_PART = ([0, 13, 19, 204, 214, 240], [1, 1, 0, 0, 1, 1])
W_GRID = ([0, 14, 18, 91, 95, 126, 130, 207, 215, 224, 240], [0, 0, 1, 1, 0, 0, 1, 1, 0.4, 0, 0])
W_RING = ([0, 46, 48, 78, 82, 93, 127, 130, 142.5, 142.6, 143.9, 144, 203, 209, 240],
          [0, 0, 1, 1, 0.3, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0])
W_RIBN = ([0, 47, 49, 79, 82, 143.9, 144, 204, 209, 240], [0, 0, 1, 1, 0, 0, 1, 1, 0, 0])
W_NEB = ([0, 94, 98, 126, 129, 240], [0, 0, 1, 1, 0, 0])
W_RISE = ([0, 127.5, 128, 142.5, 142.6, 240], [0, 0, 1, 1, 0, 0])
W_CEIL = ([0, 143, 144, 203, 209, 240], [0, 0, 1, 1, 0, 0])
WOBBLE = ([0, 14, 18, 92, 96, 128, 208, 216, 230, 240], [0.9, 0.7, 0.08, 0.08, 0.25, 0.08, 0.08, 0.35, 1.1, 1.5])
STROBE = ([0, 47, 48, 80, 92, 127, 128, 142.5, 143, 144, 207, 210, 240],
          [0, 0, 0.35, 0.35, 0, 0, 0.4, 0.45, 0, 0.6, 0.6, 0, 0])
CAM_Y = ([0, 175, 183, 191, 196, 240], [0, 0, 2.6, 4.0, 0, 0])


def boxblur(img, r):
    p = np.cumsum(np.pad(img, ((r + 1, r), (0, 0), (0, 0)), mode="edge"), axis=0, dtype=np.float32)
    img = (p[2 * r + 1 :] - p[: -2 * r - 1]) / (2 * r + 1)
    p = np.cumsum(np.pad(img, ((0, 0), (r + 1, r), (0, 0)), mode="edge"), axis=1, dtype=np.float32)
    return (p[:, 2 * r + 1 :] - p[:, : -2 * r - 1]) / (2 * r + 1)


class Scene:
    def __init__(self, seed=7):
        rng = np.random.default_rng(seed)
        self.part = rng.uniform([-7, -4, 0], [7, 4, 44], (1400, 3)).astype(np.float32)
        self.neb_dir = rng.normal(size=(3800, 3)).astype(np.float32)
        self.neb_dir /= np.linalg.norm(self.neb_dir, axis=1, keepdims=True)
        self.neb_r = (10 + rng.normal(0, 0.9, 3800)).astype(np.float32)
        self.rise_xz = rng.uniform([-7, 2], [7, 40], (60, 2)).astype(np.float32)

    def points(self, b, t, cz, E, kick):
        P, C = [], []
        struct_c = color_curve(b, PAL_B, PAL_STRUCT)
        accent_c = color_curve(b, PAL_B, PAL_ACCENT)

        w = curve(b, *W_PART)
        if w > 0.01:  # drifting cloud, wrapped around the camera
            p = self.part.copy()
            p[:, 0] += 0.6 * np.sin(0.3 * t + p[:, 2])
            p[:, 1] += 0.4 * np.sin(0.23 * t + p[:, 0] * 2)
            p[:, 2] = (p[:, 2] - cz) % 44 + 0.8
            P.append(p)
            C.append(np.tile(accent_c * (0.14 * w), (len(p), 1)))

        w = curve(b, *W_GRID)
        if w > 0.01:  # floor: z-rails and cross-ties
            zs = np.arange(0, 46, 0.22, dtype=np.float32) + (-cz % 0.22)
            xs = np.arange(-8, 8.01, 1.0, dtype=np.float32)
            gx, gz = np.meshgrid(xs, zs)
            rails = np.stack([gx.ravel(), np.full(gx.size, -2.5, np.float32), gz.ravel()], 1)
            zt = np.arange(0, 46, 4, dtype=np.float32) + (-cz % 4)
            xt = np.arange(-8, 8.01, 0.22, dtype=np.float32)
            tx, tz = np.meshgrid(xt, zt)
            ties = np.stack([tx.ravel(), np.full(tx.size, -2.5, np.float32), tz.ravel()], 1)
            g = np.vstack([rails, ties])
            P.append(g)
            C.append(np.tile(struct_c * (0.10 * w * (0.55 + 0.8 * kick)), (len(g), 1)))
            wc = curve(b, *W_CEIL)
            if wc > 0.01:  # drop 2: the tunnel closes in
                c2 = g.copy()
                c2[:, 1] = 3.5
                P.append(c2)
                C.append(np.tile(struct_c * (0.07 * wc * (0.55 + 0.8 * kick)), (len(c2), 1)))

        w = curve(b, *W_RING)
        if w > 0.01:  # tunnel rings, flashing with the stab/riff band
            th = np.linspace(0, 2 * np.pi, 130, endpoint=False, dtype=np.float32)
            ring = np.stack([6.5 * np.cos(th), 6.5 * np.sin(th) * 0.75 + 0.4, np.zeros_like(th)], 1)
            zs = np.arange(0, 32, 4, dtype=np.float32) + (-cz % 4)
            flash = 0.35 + 1.5 * E["riff"]
            for z in zs:
                r = ring.copy()
                r[:, 2] = z
                P.append(r)
                C.append(np.tile(struct_c * (0.22 * w * flash / (1 + 0.010 * z * z)), (len(r), 1)))

        w = curve(b, *W_RIBN)
        if w > 0.01:  # the riff as a double helix
            z = np.linspace(0.8, 24, 500, dtype=np.float32)
            amp = 1.7 + 2.8 * E["riff"]
            for ph in (0.0, np.pi):
                x = np.sin(z * 0.7 + b * 1.2 + ph) * amp
                y = np.cos(z * 0.55 + b * 0.9 + ph) * 0.8 - 0.4
                P.append(np.stack([x, y, z], 1))
                C.append(np.tile(accent_c * (0.20 * w), (len(z), 1)))

        w = curve(b, *W_NEB)
        if w > 0.01:  # the bloom nebula, slowly rolling overhead
            a = 0.12 * t
            rot = np.array([[np.cos(a), 0, np.sin(a)], [0, 1, 0], [-np.sin(a), 0, np.cos(a)]], np.float32)
            p = (self.neb_dir * self.neb_r[:, None]) @ rot.T
            p += np.array([0, 4.5, 19], np.float32)
            shimmer = 0.7 + 0.6 * np.sin(3 * t + self.neb_r * 7)
            P.append(p)
            C.append(accent_c[None, :] * (0.09 * w * shimmer[:, None]))

        w = curve(b, *W_RISE)
        if w > 0.01:  # sync-scream riser lines, growing with the sweep
            h = 0.2 + 5.8 * np.clip((b - 128) / 14.5, 0, 1)
            ys = np.linspace(0, 1, 26, dtype=np.float32)
            for x, z in self.rise_xz:
                zz = (z - cz) % 40 + 1.5
                col = np.array([1.0, 0.35 + 0.3 * (1 - h / 6), 0.15], np.float32)
                P.append(np.stack([np.full_like(ys, x), -2.5 + ys * h, np.full_like(ys, zz)], 1))
                C.append(np.tile(col * (0.15 * w), (len(ys), 1)))

        for s in BELL_STRIKES:  # bell strikes = expanding rings
            db_ = b - s
            if 0 < db_ < 2.5:
                th = np.linspace(0, 2 * np.pi, 90, endpoint=False, dtype=np.float32)
                r = 0.3 + db_ * 2.2
                fade = (1 - db_ / 2.5) ** 2
                p = np.stack([r * np.cos(th), r * np.sin(th) * 0.8 + 0.6, np.full_like(th, 9.0)], 1)
                P.append(p)
                C.append(np.tile(np.array([1, 0.9, 0.75], np.float32) * (0.5 * fade), (len(th), 1)))

        if not P:
            return np.zeros((0, 3), np.float32), np.zeros((0, 3), np.float32)
        return np.vstack(P).astype(np.float32), np.vstack(C).astype(np.float32)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--preview", help="comma-separated times (s) -> PNG stills")
    ap.add_argument("--frames", help="A:B frame range -> segment render")
    ap.add_argument("--segment", help="output path for a video-only segment")
    ap.add_argument("--features-only", action="store_true")
    args = ap.parse_args()

    mono, sr = load_audio(WAV)
    dur = len(mono) / sr
    n_frames = int(dur * FPS)
    if os.path.exists(FEATURE_CACHE) and not args.features_only:
        z = np.load(FEATURE_CACHE)
        E_all = {k: z[k] for k in ("kick", "bass", "riff", "high")}
    else:
        print(f"audio {dur:.1f}s -> {n_frames} frames; measuring bands...")
        E_all = band_envelopes(mono, sr, n_frames)
        np.savez(FEATURE_CACHE, **E_all)
        if args.features_only:
            print("features cached")
            return

    # camera path: integrate tempo-locked speed
    fb = np.arange(n_frames) / FPS / BEAT
    speed = np.interp(fb, SPEED_B, SPEED_V)
    cz_all = np.cumsum(speed * (1 / FPS / BEAT))

    yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
    d = np.sqrt(((xx - W / 2) / (W / 2)) ** 2 + ((yy - H / 2) / (H / 2)) ** 2)
    vign = (1 - 0.55 * np.clip(d, 0, 1.2) ** 2.2)[:, :, None].astype(np.float32)

    scene = Scene()
    accum = np.zeros((H, W, 3), np.float32)

    if args.preview:
        targets = [int(float(x) * FPS) for x in args.preview.split(",")]
        frames = []
        for tf in targets:  # warm the persistence buffer before each still
            frames.extend(range(max(0, tf - 18), tf + 1))
        targets = set(targets)
        proc = None
        write_from = 0
    else:
        if args.frames:
            a, b_end = (int(x) for x in args.frames.split(":"))
            b_end = min(b_end, n_frames)
            # warm the persistence buffer across the seam
            frames = range(max(0, a - 18), b_end)
            write_from = a
            out_path = args.segment
            cmd_tail = [out_path]
        else:
            frames = range(n_frames)
            write_from = 0
            out_path = OUT
            cmd_tail = ["-i", WAV, "-c:a", "aac", "-b:a", "192k",
                        "-shortest", "-movflags", "+faststart", OUT]
        proc = subprocess.Popen(
            ["ffmpeg", "-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24",
             "-s", f"{W}x{H}", "-r", str(FPS), "-i", "-"]
            + (["-i", WAV] if not args.frames else [])
            + ["-c:v", "libx264", "-preset", "veryfast", "-crf", "19", "-pix_fmt", "yuv420p"]
            + (["-c:a", "aac", "-b:a", "192k", "-shortest", "-movflags", "+faststart"]
               if not args.frames else [])
            + [out_path],
            stdin=subprocess.PIPE, stderr=subprocess.PIPE)

    for fi in frames:
        t = fi / FPS
        b = t / BEAT
        E = {k: float(E_all[k][fi]) for k in E_all}
        cz = float(cz_all[fi])
        cam_y = curve(b, *CAM_Y)

        pts, cols = scene.points(b, t, cz, E, E["kick"])
        buf = np.zeros((H, W, 3), np.float32)
        if len(pts):
            z = pts[:, 2]
            ok = z > 0.35
            pts, cols, z = pts[ok], cols[ok], z[ok]
            wob = curve(b, *WOBBLE)
            sx = W / 2 + FOCAL * pts[:, 0] / z
            sy = H / 2 - FOCAL * (pts[:, 1] - cam_y) / z + cam_y * 7
            sx = sx + np.sin(sy * 0.02 + t * 3.1) * wob * 11
            depth = 1.0 / (1 + 0.06 * z * z / 4)
            ix, iy = sx.astype(np.int32), sy.astype(np.int32)
            wgt_all = cols * depth[:, None] * 3.0
            for dx, dy, wf in ((0, 0, 1.0), (1, 0, 0.45), (0, 1, 0.45)):
                jx, jy = ix + dx, iy + dy
                ok = (jx >= 0) & (jx < W) & (jy >= 0) & (jy < H)
                flat = (jy[ok] * W + jx[ok]).astype(np.int64)
                wgt = wgt_all[ok] * wf
                for c in range(3):
                    buf[:, :, c] += np.bincount(flat, wgt[:, c], minlength=H * W).reshape(H, W)

        accum = accum * 0.80 + buf
        img = accum + boxblur(accum, 2) * 0.8 + boxblur(accum, 7) * 0.55
        gain = curve(b, GAIN_B, GAIN_V)
        flash = curve(b, *STROBE) * E["kick"] ** 2
        img = (1 - np.exp(-1.7 * img)) * gain * (1 + 0.6 * flash)
        img += flash * 0.03 * gain
        img = np.clip(img * vign, 0, 1) ** (1 / 1.9)
        frame = (img * 255).astype(np.uint8)

        if proc is None:
            if fi not in targets:
                continue
            name = f"/tmp/strobe_preview_{t:.0f}s.png"
            p2 = subprocess.run(
                ["ffmpeg", "-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24",
                 "-s", f"{W}x{H}", "-i", "-", "-frames:v", "1", name], input=frame.tobytes())
            print("wrote", name, p2.returncode)
        else:
            if fi < write_from:
                continue
            try:
                proc.stdin.write(frame.tobytes())
            except BrokenPipeError:
                err = proc.stderr.read().decode(errors="replace")
                sys.exit(f"ffmpeg died at frame {fi}: {err}")
            if fi % 300 == 0:
                print(f"  {fi}/{n_frames} ({100*fi/n_frames:.0f}%)", flush=True)

    if proc is not None:
        proc.stdin.close()
        proc.wait()
        print("wrote", OUT)


if __name__ == "__main__":
    main()
