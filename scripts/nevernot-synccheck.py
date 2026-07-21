#!/usr/bin/env python
"""End-to-end A/V sync test for the Nevernotbecoming film.

Two independent checks, because the two failures we actually shipped
were independent:

1. FRAME IDENTITY vs TIMESTAMP: every rendered frame carries its frame
   number as a 20-bit strip of 4x3 px blocks in the bottom-right
   corner. Seeking the finished mp4 to time T and decoding the strip
   must yield frame round(T*30) — this catches timestamp damage from
   segment splices (mpegts clocks restarting mid-file), which pure
   audio checks cannot see.

2. AUDIO vs MIX: the mp4's audio track cross-correlated against
   renders/nevernotbecoming.wav at several times must peak within
   25 ms of zero offset.

    .venv-voice/bin/python scripts/nevernot-synccheck.py [file.mp4]
"""
import os
import subprocess
import sys
import tempfile

import numpy as np
import soundfile as sf
from PIL import Image

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FPS = 30
W = 1280
H = 720


def decode_strip(png):
    img = np.asarray(Image.open(png).convert("L"), dtype=np.float32)
    f = 0
    for bit in range(20):
        x0 = W - 4 * (bit + 1)
        block = img[H - 3:H - 1, x0:x0 + 3]
        if block.mean() > 127:
            f |= 1 << bit
    return f


def main():
    mp4 = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        REPO, "renders", "nevernotbecoming-lyric.mp4")
    mix_path = os.path.join(REPO, "renders", "nevernotbecoming.wav")

    dur = float(subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration",
         "-of", "csv=p=0", mp4], capture_output=True, text=True).stdout)
    probes = [5.0, 60.0, 120.0, 180.0, 220.0, dur - 10.0]

    print(f"== frame-identity check ({mp4.split('/')[-1]}, {dur:.1f}s)")
    worst = 0
    with tempfile.TemporaryDirectory() as tmp:
        for T in probes:
            png = os.path.join(tmp, "f.png")
            subprocess.run(
                ["ffmpeg", "-y", "-loglevel", "error", "-ss", f"{T:.3f}",
                 "-i", mp4, "-frames:v", "1", png], check=True)
            got = decode_strip(png)
            want = round(T * FPS)
            d = got - want
            worst = max(worst, abs(d))
            flag = "ok" if abs(d) <= 1 else "FAIL"
            print(f"  t={T:6.1f}s  frame {got:5d} vs expected {want:5d}"
                  f"  Δ{d:+3d} frames  {flag}")

    print("== audio-offset check")
    mix, mr = sf.read(mix_path, dtype="float32")
    if mix.ndim > 1:
        mix = mix.mean(axis=1)
    with tempfile.TemporaryDirectory() as tmp:
        aw = os.path.join(tmp, "a.wav")
        subprocess.run(
            ["ffmpeg", "-y", "-loglevel", "error", "-i", mp4, "-vn",
             "-ac", "1", "-ar", str(mr), aw], check=True)
        a, _ = sf.read(aw, dtype="float32")
    worst_a = 0.0
    for T in (30.0, 120.0, 230.0):
        n = int(10 * mr)
        sa = a[int(T * mr):int(T * mr) + n]
        pad = int(0.1 * mr)
        sb = mix[int(T * mr) - pad:int(T * mr) + n + pad]
        c = np.correlate(sb, sa, "valid")
        off = (np.argmax(c) - pad) / mr * 1000
        worst_a = max(worst_a, abs(off))
        flag = "ok" if abs(off) < 25 else "FAIL"
        print(f"  t={T:6.1f}s  offset {off:+7.1f} ms  {flag}")

    if worst <= 1 and worst_a < 25:
        print("SYNC OK: frame identity and audio both aligned")
    else:
        print("SYNC FAILURE — do not ship this file")
        sys.exit(1)


if __name__ == "__main__":
    main()
