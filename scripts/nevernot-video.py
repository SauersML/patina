#!/usr/bin/env python
"""Lyric video for Nevernotbecoming — the instrument's view.

No footage, no captions: a scrolling pitch-time plane. Every word's
letters are placed ON the sung pitch curve, so lyrics physically climb
the melismas and tremble with the real vibrato; during the vocoder
choruses the words stack into one copy per chord tone and melt as the
voicings glide; the four saw voices drift behind as amber counterpoint
threads; the wordless "aaah"s bloom as compound-eye hexagon rings
pulsing with the choir. Everything is driven by the performance data:
renders/nevernot-score.json (word times), renders/nevernot-pitch.wav
(the melody curve), and the mix itself (scope + blooms).

    .venv-voice/bin/python scripts/nevernot-video.py
    -> renders/nevernotbecoming-lyric.mp4
"""
import json
import math
import os
import subprocess

import numpy as np
import soundfile as sf
from PIL import Image, ImageDraw, ImageFont

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
W, H, FPS = 1280, 720, 30
SPB = 0.6
X_NOW = int(W * 0.36)
PX_PER_S = 118.0
M_LO, M_HI = 36.0, 85.0

N = {"D2": 38, "E2": 40, "F#2": 42, "G2": 43, "A2": 45, "B2": 47,
     "C#3": 49, "D3": 50, "E3": 52, "F#3": 54, "G3": 55, "A3": 57,
     "B3": 59, "C#4": 61, "D4": 62, "E4": 64, "F#4": 66, "G4": 67,
     "A4": 69, "B4": 71, "C#5": 73, "D5": 74, "E5": 76, "F#5": 78}

# Saw counterpoint, verbatim from the song (note, beats); glide s/oct
def expand(seq, times=1):
    return seq * times

VERSE = {"S": [("E4",5),("F#4",4),("E4",5),("D4",2)],
         "A": [("G3",3),("F#3",4),("E3",4),("D3",5)],
         "T": [("B3",6),("A3",5),("B3",5)],
         "B": [("E2",4),("D2",4),("A2",4),("B2",4)]}
PRE = {"S": [("E4",4),("F#4",4),("G4",4),("A4",4)],
       "A": [("G3",4),("A3",4),("B3",4),("C#4",4)],
       "T": [("B3",4),("B3",4),("D4",4),("E4",4)],
       "B": [("E2",4),("F#2",4),("G2",4),("A2",4)]}
CHOR = {"S": expand([("E5",5),("D5",8),("E5",3)], 2) + [("E5",4)],
        "A": expand([("A3",5),("B3",4),("A3",4),("G3",3)], 2) + [("A3",4)],
        "T": expand([("E4",5),("F#4",7),("E4",4)], 2) + [("E4",4)],
        "B": expand([("A2",4),("B2",4),("D2",4),("E2",4)], 2) + [("A2",4)]}
RISE = {"S": [("B4",4),("C#5",4),("D5",4),("E5",4),("F#5",4)],
        "A": [("B3",4),("C#4",4),("E4",4),("F#4",4),("G4",4)],
        "T": [("G3",4),("A3",4),("C#4",4),("B3",4),("E4",4)],
        "B": [("E2",4),("F#2",4),("A2",4),("B2",4),("E2",4)]}


def track_seq(k):
    intro = {"S": [("E4",6),("F#4",4),("E4",6)],
             "A": [("G3",7),("F#3",3),("G3",6)],
             "T": [("B3",16)],
             "B": [("E2",4),("E2",4),("E2",4),("D2",4)]}[k]
    il = {"S": [("E5",8)], "A": [("G3",5),("F#3",3)],
          "T": [("B3",8)], "B": [("E2",4),("D2",4)]}[k]
    br = {"S": [(None,48)], "A": [(None,48)],
          "T": expand([("B3",8),("A3",8)], 3),
          "B": [("E2",8)]*6}[k]
    conv = {"S": [("E5",20)], "A": [("G4",8),("E4",12)],
            "T": [("B3",8),("E4",12)], "B": [("A2",4),("G2",4),("E2",12)]}[k]
    tail = {"S": [("E5",12),(None,16)], "A": [("E4",12),(None,16)],
            "T": [("E4",12),(None,16)], "B": [("E2",28)]}[k]
    return (intro + VERSE[k]*7 + PRE[k] + CHOR[k] + il + VERSE[k]*4 +
            PRE[k] + CHOR[k] + br + VERSE[k] + RISE[k] + VERSE[k] +
            conv + tail)


SAW = {k: (g, track_seq(k))
       for k, g in (("S", 2.5), ("A", 3.5), ("T", 4.0), ("B", 0.15))}

CHORUS_CHORDS = ([("A3 E4 B4 E5",2),("A3 E4 A4 C#5",2),("B3 F#4 B4 D5",2),
                  ("G3 E4 G4 B4",2),("A3 E4 A4 C#5",2),("B3 F#4 B4 D5",2),
                  ("C#4 A4 C#5 E5",2),("B3 F#4 B4 D5",2),
                  ("D4 A4 D5",2),("C#4 A4 C#5",2),("B3 B4 E5",2),
                  ("A3 A4 C#5",2),("D4 A4 D5",2),("C#4 A4 C#5",2),
                  ("B3 B4 E5",4),("C#4 A4 E5",4)])
BUZZ_CHORD = [("E2 B2 E3 B3", 8)]


def chord_events():
    ev = []
    def add(start_beat, seq):
        b = start_beat
        for ch, dur in seq:
            ev.append((b * SPB, [N[n] for n in ch.split()]))
            b += dur
    add(8, BUZZ_CHORD)
    add(144, CHORUS_CHORDS)
    add(180, [("E3 B3 E4 G4",4),("E3 B3 E4 F#4",4)])
    add(268, CHORUS_CHORDS)
    add(424, [("A3 E4 A4 C#5",4),("G3 D4 G4 B4",4),("E3 B3 E4 G4",4)])
    add(436, BUZZ_CHORD)
    return sorted(ev)


def glide_track(seq, glide, total_s, rate=FPS):
    """Linear slew in octave space at 1/glide oct/s — the panel law."""
    n = int(total_s * rate)
    out = np.zeros(n)
    t = 0.0
    cur = N[seq[0][0]] / 12.0
    idx = 0
    tgt = cur
    changes = []
    b = 0.0
    for note, beats in seq:
        changes.append((b * SPB, None if note is None else N[note] / 12.0))
        b += beats
    ci = 0
    step = (1.0 / max(glide, 1e-3)) / rate
    for i in range(n):
        tt = i / rate
        while ci < len(changes) and changes[ci][0] <= tt:
            tgt = changes[ci][1]
            ci += 1
        if tgt is None:
            out[i] = np.nan
            continue
        d = tgt - cur
        if abs(d) <= step:
            cur = tgt
        else:
            cur += step * (1 if d > 0 else -1)
        out[i] = cur * 12.0
    return out


def m2y(m):
    return H - (m - M_LO) / (M_HI - M_LO) * (H - 90) - 50


def t2x(tt, t):
    return X_NOW + (tt - t) * PX_PER_S


def hexagon(cx, cy, r, rot=0.0):
    return [(cx + r * math.cos(rot + k * math.pi / 3),
             cy + r * math.sin(rot + k * math.pi / 3)) for k in range(7)]


def load_font(size):
    for path, idx in [("/System/Library/Fonts/Supplemental/Futura.ttc", 1),
                      ("/System/Library/Fonts/Helvetica.ttc", 0)]:
        try:
            return ImageFont.truetype(path, size, index=idx)
        except OSError:
            continue
    return ImageFont.load_default()


def main():
    score = json.load(open(os.path.join(REPO, "renders/nevernot-score.json")))
    intro = score["intro"]
    words = []
    voc_spans = []
    aah_spans = []
    for ph in score["phrases"]:
        if ph["vocoder"]:
            voc_spans.append((intro + ph["start"], intro + ph["start"] + ph["dur"]))
            if ph["text"].startswith("Aaah"):
                aah_spans.append((intro + ph["start"], intro + ph["start"] + ph["dur"]))
        for w in ph["words"]:
            words.append({"w": w["w"], "t": intro + w["t"], "d": w["dur"],
                          "vel": w["vel"], "voc": ph["vocoder"],
                          "aah": ph["text"].startswith("Aaah")})

    curve, crate = sf.read(os.path.join(REPO, "renders/nevernot-pitch.wav"),
                           dtype="float32")
    mix, mrate = sf.read(os.path.join(REPO, "renders/nevernotbecoming.wav"),
                         dtype="float32")
    if mix.ndim > 1:
        mix = mix.mean(axis=1)
    dur = len(mix) / mrate
    nframes = int(dur * FPS)

    # audio RMS at frame rate, for the blooms
    hop = mrate // FPS
    nrm = len(mix) // hop
    rms = np.sqrt((mix[:nrm * hop].reshape(nrm, hop) ** 2).mean(axis=1))
    rms = rms / (rms.max() + 1e-9)

    saws = {k: glide_track(seq, g, dur) for k, (g, seq) in SAW.items()}
    chords = chord_events()

    def curve_at(tt):
        i = int((tt - intro) * crate)
        if i < 0 or i >= len(curve):
            return 0.0
        return float(curve[i])

    def chord_at(tt):
        cur = None
        for ct, notes in chords:
            if ct <= tt:
                cur = (ct, notes)
            else:
                break
        return cur

    fonts = {s: load_font(s) for s in (22, 26, 30, 36, 44, 54)}
    probe_only = os.environ.get("PROBE") is not None
    out_path = os.path.join(REPO, "renders/nevernotbecoming-lyric.mp4")
    ff = None if probe_only else subprocess.Popen(
        ["ffmpeg", "-y", "-loglevel", "error",
         "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}",
         "-r", str(FPS), "-i", "-",
         "-i", os.path.join(REPO, "renders/nevernotbecoming.wav"),
         "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18",
         "-c:a", "aac", "-b:a", "192k", "-shortest", out_path],
        stdin=subprocess.PIPE)

    bg = Image.new("RGB", (W, H), (8, 7, 6))
    d0 = ImageDraw.Draw(bg)
    # faint standing hex lattice — the compound eye at rest
    for gy in range(-1, 9):
        for gx in range(-1, 13):
            cx = gx * 118 + (59 if gy % 2 else 0)
            cy = gy * 100
            d0.line(hexagon(cx, cy, 56), fill=(14, 15, 14), width=1)

    probes = {int(s * FPS): s for s in (15.0, 95.0, 109.0, 228.0)}

    for f in (sorted(probes) if probe_only else range(nframes)):
        t = f / FPS
        img = bg.copy()
        dr = ImageDraw.Draw(img, "RGBA")
        t0w, t1w = t - X_NOW / PX_PER_S, t + (W - X_NOW) / PX_PER_S

        # amber counterpoint threads: the four saw voices, portamento
        # and all — song starts them at beat 0
        for k, arr in saws.items():
            pts = []
            wgt = 3 if k in ("B",) else 2
            for px in range(0, W, 4):
                tt = t0w + px / PX_PER_S
                i = int(tt * FPS)
                v = arr[i] if 0 <= i < len(arr) else np.nan
                if np.isnan(v):
                    if len(pts) > 1:
                        dr.line(pts, fill=(196, 132, 45, 110), width=wgt)
                    pts = []
                else:
                    pts.append((px, m2y(v)))
            if len(pts) > 1:
                dr.line(pts, fill=(196, 132, 45, 110), width=wgt)

        # phosphor scope: the mix itself, folded along the bottom
        i0 = int(t * mrate)
        seg = mix[max(0, i0 - mrate // 30):i0]
        if len(seg) > 4:
            pts = [(int(W * j / len(seg)), H - 34 + int(seg[j] * 26))
                   for j in range(0, len(seg), 8)]
            dr.line(pts, fill=(0, 150, 130, 90), width=1)

        # the sung pitch line, phosphor cyan with distance dimming
        pts, last = [], None
        for px in range(0, W, 2):
            tt = t0w + px / PX_PER_S
            m = curve_at(tt)
            if m > 1.0:
                pts.append((px, m2y(m)))
            else:
                if len(pts) > 1:
                    dr.line(pts, fill=(0, 190, 170, 150), width=2)
                pts = []
        if len(pts) > 1:
            dr.line(pts, fill=(0, 190, 170, 150), width=2)

        # chord tones as breathing rungs while the vocoder holds them
        ch = chord_at(t)
        in_voc = any(a <= t < b for a, b in voc_spans)
        if in_voc and ch:
            age = t - ch[0]
            for m in ch[1]:
                y = m2y(m)
                a = int(120 * math.exp(-age * 0.6) + 40)
                dr.line([(X_NOW - 130, y), (X_NOW + 150, y)],
                        fill=(0, 160, 150, a), width=1)

        # the words
        for wd in words:
            wt, wdur = wd["t"], max(wd["d"], 0.25)
            if wt > t1w or wt + wdur < t0w - 3.0:
                continue
            active = wt <= t <= wt + wdur
            past = t > wt + wdur
            base_size = 44 if wd["voc"] else (36 if wd["vel"] >= 1.0 else 30)
            if wd["aah"]:
                base_size = 54
            size = base_size if (active or past) else 26
            font = fonts[size]
            if active:
                col = (225, 255, 245, 255)
            elif past:
                fade = max(0.0, 1.0 - (t - wt - wdur) / 2.8)
                col = (0, int(200 * fade), int(180 * fade), int(230 * fade))
                if fade <= 0:
                    continue
            else:
                col = (90, 130, 125, 160)
            text = wd["w"]
            n = len(text)
            if wd["voc"] and not wd["aah"] and ch:
                # chordal voice: the ACTIVE word stacks, one copy per
                # held tone; neighbors sit small on the top tone only
                if active:
                    for ci, m in enumerate(ch[1]):
                        a = 255 if ci == len(ch[1]) - 1 else 110
                        dr.text((t2x(wt, t), m2y(m) - size * 0.6), text,
                                font=font,
                                fill=(col[0], col[1], col[2], a))
                else:
                    top = max(ch[1])
                    dr.text((t2x(wt, t), m2y(top) - 30), text,
                            font=fonts[22], fill=col)
            elif wd["aah"]:
                # the long exhale: letters spread across the whole hold
                for li, chch in enumerate(text):
                    tt = wt + wdur * (li / max(n - 1, 1)) * 0.85
                    m = curve_at(tt)
                    if m <= 1.0:
                        m = 64.0
                    dr.text((t2x(tt, t), m2y(m) - size * 1.05), chch,
                            font=font, fill=col)
            else:
                # letters advance at type width but RIDE the sung curve
                # — melismas climb, vibrato trembles the baseline
                x0 = t2x(wt, t)
                xa = x0
                for chch in text:
                    tt = wt + max(0.0, min((xa - x0) / PX_PER_S,
                                           wdur * 0.92))
                    m = curve_at(tt)
                    if m <= 1.0:
                        m = curve_at(wt + 0.05) or 64.0
                    dr.text((xa, m2y(m) - size * 1.05), chch,
                            font=font, fill=col)
                    xa += dr.textlength(chch, font=font)

        # compound-eye bloom on the wordless choir
        for a0, b0 in aah_spans:
            if a0 - 1 <= t <= b0 + 2:
                k = rms[min(f, len(rms) - 1)]
                phase = (t - a0) * 0.5
                for ring in range(1, 7):
                    r = 36 + ring * 46 * (0.5 + 0.8 * k) + 10 * math.sin(phase + ring)
                    alpha = int(180 * k * math.exp(-ring * 0.3))
                    if alpha > 3:
                        dr.line(hexagon(X_NOW, m2y(64), r, rot=phase * 0.3),
                                fill=(0, 200, 180, alpha), width=2)

        # the now-line, brass
        dr.line([(X_NOW, 40), (X_NOW, H - 46)], fill=(170, 120, 60, 70), width=1)

        if ff is not None:
            ff.stdin.write(img.tobytes())
        if f in probes:
            img.save(f"{os.environ.get('PROBE_DIR', '/tmp')}/probe-{probes[f]:.0f}.png")

    if ff is not None:
        ff.stdin.close()
        ff.wait()
        print(f"wrote {out_path} ({nframes} frames, {dur:.1f}s)")


if __name__ == "__main__":
    main()
