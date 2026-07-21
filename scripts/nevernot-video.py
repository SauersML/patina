#!/usr/bin/env python
"""Lyric film for Nevernotbecoming — the transmission seen from inside.

Not a lyric video: a pitch-time plane that EXPANDS and DISSOLVES with
the text it carries. Every word's letters ride the actual sung pitch
curve (melismas climb, vibrato trembles the baselines); when a word is
over it does not fade — it MOLTS, sublimating into hex-dust that
drifts up and out. The standing hexagonal lattice is a compound eye
that sees the music: each cell pulses with its own frequency band of
the mix. The bee is drawn — a small hex-bodied thing with flickering
wings that flies in on the opening buzz along the buzz-synth's own
pitch path, and recedes at the end. Chorus words are BORN as one and
split apart onto the held chord tones (the egregore reconstituting);
the view slowly widens through every choir span and contracts to
intimacy for the bridge; the final ascent floods the frame with light
before the coda lets everything dissolve. The title assembles itself
from dust in the first bar — becoming — and is gone by the time the
voice arrives.

Everything is driven by performance data: renders/nevernot-score.json,
renders/nevernot-pitch.wav, and the mix itself. Encoding is segmented
mpegts (survives the kernel killing ffmpeg under memory pressure).

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
M_LO, M_HI = 36.0, 85.0

N = {"C2": 36, "D2": 38, "E2": 40, "F#2": 42, "G2": 43, "A2": 45,
     "B2": 47, "C#3": 49, "D3": 50, "E3": 52, "F#3": 54, "G3": 55,
     "G#3": 56, "A3": 57, "A#3": 58, "B3": 59, "C4": 60, "C#4": 61,
     "D4": 62, "D#4": 63, "E4": 64, "F#4": 66, "G4": 67, "G#4": 68,
     "A4": 69, "B4": 71, "C#5": 73, "D5": 74, "D#5": 75, "E5": 76,
     "F#5": 78, "G#5": 80}


def expand(seq, times=1):
    return seq * times

VERSE = {"S": [("E4",5),("F#4",4),("E4",5),("D4",2)],
         "A": [("G3",3),("F#3",4),("E3",4),("D3",5)],
         "T": [("B3",6),("A3",5),("B3",5)],
         "B": [("E2",4),("D2",4),("A2",4),("B2",4)]}
PRE = {"S": [("G4",6),("A4",6),("B4",4)],
       "A": [("E3",6),("F#3",6),("G#3",4)],
       "T": [("C3",6),("D3",6),("E3",4)],
       "B": [("C2",6),("D2",6),("E2",4)]}
N["C3"] = 48
CHOR = {"S": [("E5",6),("C#5",2),("E5",4),("F#5",2),("E5",2),
              ("C#5",4),("D5",4),("C#5",4),("E5",8)],
        "A": expand([("A3",5),("B3",4),("A3",4),("G#3",3)], 2) + [("B3",4)],
        "T": expand([("E3",5),("F#3",7),("E3",4)], 2) + [("E3",4)],
        "B": expand([("A2",4),("B2",4),("D2",4),("E2",4)], 2) + [("E2",4)]}
RISE = {"S": [("B4",4),("C#5",4),("D#5",4),("E5",4),("F#5",4)],
        "A": [("B3",4),("C#4",4),("E4",4),("F#4",4),("G#4",4)],
        "T": [("G#3",4),("A3",4),("C#4",4),("B3",4),("E4",4)],
        "B": [("E2",4),("F#2",4),("A2",4),("B2",4),("E2",4)]}


def track_seq(k):
    intro = {"S": [("E4",6),("F#4",4),("E4",6)],
             "A": [("G3",7),("F#3",3),("G3",6)],
             "T": [("B3",16)],
             "B": [("E2",4),("E2",4),("E2",4),("D2",4)]}[k]
    il = {"S": [("E5",8)], "A": [("G3",5),("F#3",3)],
          "T": [("B3",8)], "B": [("E2",4),("D2",4)]}[k]
    br = {"S": [(None,48)], "A": [(None,48)],
          "T": [("B3",8),("A#3",8),("A3",8),("G#3",8),("A3",8),("A#3",8)],
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

# The committed chorus: A | Bm | D | E, E5 pedal tops, F#5 escalation
CHORUS_CHORDS = ([("A2 A3 E4 C#5 E5",2),("B2 B3 F#4 B4 E5",2),
                  ("D3 D4 F#4 A4 E5",2),("E3 E4 G#4 B4 C#5",2),
                  ("A2 A3 E4 C#5 E5",2),("B2 B3 F#4 D5 F#5",2),
                  ("D3 D4 F#4 A4 E5",2),("E3 E4 G#4 B4 D5",2),
                  ("A2 A3 E4 C#5",2),("B2 B3 F#4 D5",2),
                  ("D3 D4 A4 C#5",2),("E3 E4 G#4 B4",2),
                  ("A2 A3 E4 C#5",2),("B2 B3 F#4 D5",2),
                  ("D3 D4 A4 D5",4),("B2 B3 G#4 E5",4)])

PRE_TRIO = [("G3 C4 E4",2),("G3 D4 G4",2),("E3 C4 A4",1),
            ("E3 G4 A4",1),("F#3 G4 B4",2),("F#3 F#4 B4",1),
            ("A3 F#4 A4",1.5),("A3 E4 B4",1.5),("G#3 F#4 C#5",2),
            ("G#3 G#4 B4",2)]

# The bee's own line (the buzz synth track, verbatim)
BEE_IN = [(None,1),("A3",0.75),("G#3",0.25),("A3",0.5),("A#3",0.25),
          ("A3",0.75),("G#3",0.5),("A3",0.5),("B3",0.25),("A3",0.75),
          ("G#3",0.25),("A3",0.5),("A#3",0.25),("A3",0.75),("G#3",0.5),
          ("A3",1.25)]
BEE_OUT = [("A3",0.5),("G#3",0.5),("A3",0.75),("G3",0.25),("F#3",0.75),
           ("G3",0.25),("F#3",0.5),("E3",0.5),("F#3",0.25),("E3",0.75),
           ("E3",1),("D3",0.5),("E3",1.5)]

# section map: (start_beat, name). Zoom + tint + lattice glow per name.
SECTIONS = [(0,"intro"),(16,"verse"),(128,"pre"),(144,"chorus"),
            (180,"aaah"),(188,"verse"),(252,"pre"),(268,"chorus"),
            (304,"bridge"),(352,"verse"),(368,"ascent"),(388,"verse"),
            (424,"aaah"),(436,"buzz"),(452,"end")]
LOOK = {  # px_per_s (zoom out = expansive), tint rgb, lattice glow
    "intro":  (118, (1.00, 1.00, 1.00), 0.5),
    "verse":  (118, (1.00, 1.00, 1.00), 0.6),
    "pre":    (106, (0.92, 1.05, 1.10), 1.0),
    "chorus": (94,  (1.05, 1.12, 1.20), 1.6),
    "aaah":   (88,  (1.10, 1.18, 1.18), 2.0),
    "bridge": (134, (0.68, 0.80, 1.00), 0.25),
    "ascent": (86,  (1.22, 1.15, 0.95), 2.4),
    "buzz":   (100, (0.90, 0.95, 0.90), 0.7),
    "end":    (100, (0.8, 0.8, 0.8), 0.3),
}


def section_at(beat):
    cur = "intro"
    for b, name in SECTIONS:
        if b <= beat:
            cur = name
        else:
            break
    return cur


def glide_track(seq, glide, total_s, rate=FPS):
    n = int(total_s * rate)
    out = np.zeros(n)
    changes = []
    b = 0.0
    for note, beats in seq:
        changes.append((b * SPB, None if note is None else N[note] / 12.0))
        b += beats
    first = next((v for _, v in changes if v is not None), 57 / 12.0)
    cur, tgt = first, changes[0][1]
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


def dust(seed, n):
    """Deterministic particle kinematics for one molted word."""
    rng = np.random.default_rng(seed)
    ang = rng.uniform(-2.6, -0.5, n)          # up and outward
    spd = rng.uniform(18, 70, n)
    return (rng.uniform(0, 1, n), np.cos(ang) * spd, np.sin(ang) * spd,
            rng.uniform(1.4, 3.2, n))


def main():
    score = json.load(open(os.path.join(REPO, "renders/nevernot-score.json")))
    intro = score["intro"]
    words, voc_spans, aah_spans = [], [], []
    for ph in score["phrases"]:
        if ph["vocoder"]:
            span = (intro + ph["start"], intro + ph["start"] + ph["dur"])
            voc_spans.append(span)
            if ph["text"].startswith("Aaah"):
                aah_spans.append(span)
        for w in ph["words"]:
            words.append({"w": w["w"], "t": intro + w["t"], "d": w["dur"],
                          "vel": w["vel"], "voc": ph["vocoder"],
                          "aah": ph["text"].startswith("Aaah"),
                          "buzz": ph["text"].startswith("Bzz")})

    curve, crate = sf.read(os.path.join(REPO, "renders/nevernot-pitch.wav"),
                           dtype="float32")
    mix, mrate = sf.read(os.path.join(REPO, "renders/nevernotbecoming.wav"),
                         dtype="float32")
    if mix.ndim > 1:
        mix = mix.mean(axis=1)
    dur = len(mix) / mrate
    nframes = int(dur * FPS)

    hop = mrate // FPS
    nrm = len(mix) // hop
    rms = np.sqrt((mix[:nrm * hop].reshape(nrm, hop) ** 2).mean(axis=1))
    rms = rms / (rms.max() + 1e-9)

    # eight log bands per frame — the compound eye's retina
    fft_n = 2048
    edges = np.geomspace(60, 9000, 9)
    bands = np.zeros((nframes, 8), dtype=np.float32)
    win = np.hanning(fft_n)
    for f in range(nframes):
        i0 = int(f / FPS * mrate)
        seg = mix[i0:i0 + fft_n]
        if len(seg) < fft_n:
            break
        sp = np.abs(np.fft.rfft(seg * win)) ** 2
        fr = np.fft.rfftfreq(fft_n, 1 / mrate)
        for b in range(8):
            m = (fr >= edges[b]) & (fr < edges[b + 1])
            bands[f, b] = sp[m].sum()
    bands /= np.percentile(bands, 99, axis=0, keepdims=True) + 1e-12
    bands = np.clip(bands, 0, 1)

    saws = {k: glide_track(seq, g, dur) for k, (g, seq) in SAW.items()}
    bee_in = glide_track(BEE_IN, 0.08, 10.0)
    bee_out = glide_track(BEE_OUT, 0.08, 9.0)

    chords = []
    def add_ch(start_beat, seq):
        b = start_beat
        for ch, d in seq:
            chords.append((b * SPB, [N[n] for n in ch.split()]))
            b += d
    add_ch(128, PRE_TRIO)
    add_ch(144, CHORUS_CHORDS)
    add_ch(180, [("E3 B3 E4 G4",4),("E3 B3 E4 F#4",4)])
    add_ch(252, PRE_TRIO)
    add_ch(268, CHORUS_CHORDS)
    add_ch(424, [("A3 E4 A4 C#5",4),("G#3 E4 B4 E5",4),("E3 B3 E4 G#4",4)])
    chords.sort()

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

    fonts = {s: load_font(s) for s in (20, 22, 26, 30, 36, 44, 54, 64)}
    probe_only = os.environ.get("PROBE") is not None
    out_path = os.path.join(REPO, "renders/nevernotbecoming-lyric.mp4")

    seg_dir = os.path.join(REPO, "renders", ".nn-segs")
    os.makedirs(seg_dir, exist_ok=True)
    segments = []

    def open_segment(idx):
        path = os.path.join(seg_dir, f"seg{idx:03d}.ts")
        proc = subprocess.Popen(
            ["ffmpeg", "-y", "-loglevel", "error",
             "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}",
             "-r", str(FPS), "-i", "-",
             "-c:v", "libx264", "-preset", "veryfast", "-crf", "19",
             "-pix_fmt", "yuv420p", "-f", "mpegts", path],
            stdin=subprocess.PIPE)
        return path, proc

    def seg_frames(path):
        out = subprocess.run(
            ["ffprobe", "-v", "error", "-count_packets",
             "-select_streams", "v", "-show_entries",
             "stream=nb_read_packets", "-of", "csv=p=0", path],
            capture_output=True, text=True)
        try:
            return int(out.stdout.split()[0])
        except (ValueError, IndexError):
            return 0

    ff = None
    seg_path = None

    # hex lattice cell centers, row-indexed for the retina bands
    cells = []
    for gy in range(-1, 9):
        for gx in range(-1, 13):
            cx = gx * 118 + (59 if gy % 2 else 0)
            cy = gy * 100
            cells.append((cx, cy, min(max(gy, 0), 7)))

    title = "nevernotbecoming"
    tfont = fonts[64]

    probes = {int(s * FPS): s for s in (2.0, 15.0, 90.0, 110.0, 195.0, 228.0)}
    px_per_s = 118.0

    f = 0
    frame_iter = sorted(probes) if probe_only else None
    while True:
        if probe_only:
            if not frame_iter:
                break
            f = frame_iter.pop(0)
        elif f >= nframes:
            break
        if not probe_only and ff is None:
            seg_path, ff = open_segment(len(segments))
        t = f / FPS
        beat = t / SPB
        sec = section_at(beat)
        zoom_target, tint, glow = LOOK[sec]
        px_per_s += (zoom_target - px_per_s) * (1.0 if probe_only else 0.02)
        k_now = rms[min(f, len(rms) - 1)]

        def col(rgb, a):
            return (min(255, int(rgb[0] * tint[0])),
                    min(255, int(rgb[1] * tint[1])),
                    min(255, int(rgb[2] * tint[2])), a)

        def t2x(tt):
            return X_NOW + (tt - t) * px_per_s

        img = Image.new("RGB", (W, H), (8, 7, 6))
        dr = ImageDraw.Draw(img, "RGBA")
        t0w = t - X_NOW / px_per_s
        t1w = t + (W - X_NOW) / px_per_s

        # the compound eye: each cell is a retina pixel for one band
        bnow = bands[min(f, len(bands) - 1)]
        for cx, cy, row in cells:
            a = int(10 + 70 * glow * bnow[row])
            if sec == "ascent":
                a = min(160, a + int(40 * (beat - 368) / 20))
            dr.line(hexagon(cx, cy, 56), fill=col((0, 190, 170), a), width=1)

        # amber counterpoint threads (silent voices draw nothing)
        for k, arr in saws.items():
            pts = []
            wgt = 3 if k == "B" else 2
            for px in range(0, W, 4):
                tt = t0w + px / px_per_s
                i = int(tt * FPS)
                v = arr[i] if 0 <= i < len(arr) else np.nan
                if np.isnan(v):
                    if len(pts) > 1:
                        dr.line(pts, fill=col((196, 132, 45), 110), width=wgt)
                    pts = []
                else:
                    pts.append((px, m2y(v)))
            if len(pts) > 1:
                dr.line(pts, fill=col((196, 132, 45), 110), width=wgt)

        # the sung pitch line
        pts = []
        for px in range(0, W, 2):
            tt = t0w + px / px_per_s
            m = curve_at(tt)
            if m > 1.0:
                pts.append((px, m2y(m)))
            else:
                if len(pts) > 1:
                    dr.line(pts, fill=col((0, 190, 170), 150), width=2)
                pts = []
        if len(pts) > 1:
            dr.line(pts, fill=col((0, 190, 170), 150), width=2)

        ch = chord_at(t)
        in_voc = any(a0 <= t < b0 for a0, b0 in voc_spans)
        if in_voc and ch:
            age = t - ch[0]
            for m in ch[1]:
                y = m2y(m)
                a = int(120 * math.exp(-age * 0.6) + 40)
                dr.line([(X_NOW - 130, y), (X_NOW + 150, y)],
                        fill=col((0, 160, 150), a), width=1)

        # words: future faint, active bright, past MOLTING into dust
        for wd in words:
            wt, wdur = wd["t"], max(wd["d"], 0.25)
            if wt > t1w or wt + wdur < t0w - 3.2:
                continue
            active = wt <= t <= wt + wdur
            past = t > wt + wdur
            base_size = 44 if wd["voc"] else (36 if wd["vel"] >= 1.0 else 30)
            if wd["aah"] or wd["buzz"]:
                base_size = 54
            size = base_size if (active or past) else 26
            font = fonts[size]
            text = wd["w"]
            n = len(text)
            if past:
                # the molt: letters sublimate into rising hex-dust
                age = t - wt - wdur
                fade = max(0.0, 1.0 - age / 2.6)
                if fade <= 0:
                    continue
                fr0, vx, vy, life = dust(hash(wd["w"]) & 0xffff ^ int(wt * 7), 12)
                x0 = t2x(wt)
                m0 = curve_at(wt + wdur * 0.5)
                y0 = m2y(m0 if m0 > 1 else 64.0)
                dr.text((x0, y0 - size), text, font=font,
                        fill=col((0, 170, 155), int(150 * fade ** 2)))
                for j in range(len(fr0)):
                    if age > life[j]:
                        continue
                    px_ = x0 + fr0[j] * size * n * 0.55 + vx[j] * age
                    py_ = y0 + vy[j] * age
                    aa = int(200 * fade * (1 - age / life[j]))
                    if aa > 4:
                        dr.line(hexagon(px_, py_, 2.6), fill=col((90, 220, 200), aa))
                continue
            col_w = (col((225, 255, 245), 255) if active
                     else col((90, 130, 125), 150))
            if wd["voc"] and not (wd["aah"] or wd["buzz"]) and ch:
                if active:
                    # born as one, splitting onto the chord tones
                    grow = min(1.0, (t - wt) / 0.35)
                    ease = grow * grow * (3 - 2 * grow)
                    yc = sum(m2y(m) for m in ch[1]) / len(ch[1])
                    for ci, m in enumerate(ch[1]):
                        yy = yc + (m2y(m) - yc) * ease
                        a = 255 if ci == len(ch[1]) - 1 else 110
                        dr.text((t2x(wt), yy - size * 0.6), text, font=font,
                                fill=(col_w[0], col_w[1], col_w[2], a))
                else:
                    dr.text((t2x(wt), m2y(max(ch[1])) - 30), text,
                            font=fonts[20], fill=col_w)
            elif wd["aah"] or wd["buzz"]:
                for li, chch in enumerate(text):
                    tt = wt + wdur * (li / max(n - 1, 1)) * 0.85
                    m = curve_at(tt)
                    y = m2y(m if m > 1 else 64.0)
                    dr.text((t2x(tt), y - size * 1.05), chch, font=font,
                            fill=col_w)
            else:
                x0 = t2x(wt)
                xa = x0
                for chch in text:
                    tt = wt + max(0.0, min((xa - x0) / px_per_s, wdur * 0.92))
                    m = curve_at(tt)
                    if m <= 1.0:
                        m = curve_at(wt + 0.05) or 64.0
                    dr.text((xa, m2y(m) - size * 1.05), chch, font=font,
                            fill=col_w)
                    xa += dr.textlength(chch, font=font)

        # aaah blooms: the eye dilating
        for a0, b0 in aah_spans:
            if a0 - 1 <= t <= b0 + 2:
                phase = (t - a0) * 0.5
                for ring in range(1, 7):
                    r = 36 + ring * 46 * (0.5 + 0.8 * k_now) + 10 * math.sin(phase + ring)
                    alpha = int(180 * k_now * math.exp(-ring * 0.3))
                    if alpha > 3:
                        dr.line(hexagon(X_NOW, m2y(64), r, rot=phase * 0.3),
                                fill=col((0, 200, 180), alpha), width=2)

        # the bee, on its own pitch path
        bee = None
        if 0.5 <= t < 9.7:
            i = int(t * FPS)
            if i < len(bee_in) and not np.isnan(bee_in[i]):
                prog = (t - 0.5) / 9.2
                bee = (80 + prog * (W - 160), m2y(bee_in[i]) - 60)
        elif 261.0 <= t < 269.5:
            i = int((t - 261.0) * FPS)
            if i < len(bee_out) and not np.isnan(bee_out[i]):
                prog = (t - 261.0) / 8.5
                bee = (W * 0.6 - prog * (W * 0.55), m2y(bee_out[i]) - 40 + prog * 90)
        if bee:
            bx, by = bee
            bx += 6 * math.sin(t * 7.3)
            by += 4 * math.sin(t * 9.1)
            dr.polygon(hexagon(bx, by, 7), fill=col((230, 190, 60), 230))
            dr.polygon(hexagon(bx - 9, by + 2, 5), fill=col((60, 50, 20), 230))
            if f % 2 == 0:
                dr.ellipse([bx - 4, by - 14, bx + 10, by - 4],
                           outline=col((220, 240, 235), 150))
            else:
                dr.ellipse([bx - 10, by - 12, bx + 4, by - 2],
                           outline=col((220, 240, 235), 150))
            for k3 in range(1, 7):
                dr.line(hexagon(bx - k3 * 16, by + k3 * 3, 1.6),
                        fill=col((230, 190, 60), max(0, 120 - k3 * 20)))

        # title: assembles from dust, dissolves as the bee arrives
        if t < 5.4:
            a_in = min(1.0, t / 2.2)
            a_out = max(0.0, min(1.0, (5.4 - t) / 1.2))
            aa = a_in * a_out
            tw = dr.textlength(title, font=tfont)
            x0 = (W - tw) / 2
            rng = np.random.default_rng(99)
            xa = x0
            for li, chch in enumerate(title):
                conv = min(1.0, max(0.0, (t - li * 0.09) / 1.6))
                e = conv * conv * (3 - 2 * conv)
                ox, oy = rng.uniform(-260, 260), rng.uniform(-200, 200)
                dr.text((xa + ox * (1 - e), H * 0.42 + oy * (1 - e)),
                        chch, font=tfont,
                        fill=col((200, 250, 240), int(255 * aa * (0.25 + 0.75 * e))))
                xa += dr.textlength(chch, font=tfont)

        # scope along the floor
        i0 = int(t * mrate)
        segm = mix[max(0, i0 - mrate // 30):i0]
        if len(segm) > 8:
            pts = [(int(W * j / len(segm)), H - 34 + int(segm[j] * 26))
                   for j in range(0, len(segm), 8)]
            dr.line(pts, fill=col((0, 150, 130), 90), width=1)

        dr.line([(X_NOW, 40), (X_NOW, H - 46)], fill=col((170, 120, 60), 70), width=1)

        # the coda whiteout, then the dark
        if sec == "aaah" and beat >= 424:
            wash = int(60 * k_now)
            dr.rectangle([0, 0, W, H], fill=(255, 255, 250, wash))
        if sec in ("buzz", "end") and beat >= 436:
            dark = int(180 * min(1.0, (beat - 436) / 14))
            dr.rectangle([0, 0, W, H], fill=(0, 0, 0, dark))

        if ff is not None:
            try:
                ff.stdin.write(img.tobytes())
            except BrokenPipeError:
                ff.wait()
                landed = seg_frames(seg_path)
                start = sum(nn for _, nn in segments)
                segments.append((seg_path, landed))
                f = start + landed
                print(f"ffmpeg died at frame {f}; resuming", flush=True)
                ff = None
                continue
        if f in probes:
            img.save(f"{os.environ.get('PROBE_DIR', '/tmp')}/probe-{probes[f]:.0f}.png")
        if not probe_only:
            f += 1

    if ff is not None:
        ff.stdin.close()
        ff.wait()
        segments.append((seg_path, seg_frames(seg_path)))
    if not probe_only:
        concat = "concat:" + "|".join(p for p, _ in segments)
        subprocess.run(
            ["ffmpeg", "-y", "-loglevel", "error", "-i", concat,
             "-i", os.path.join(REPO, "renders/nevernotbecoming.wav"),
             "-c:v", "copy", "-c:a", "aac", "-b:a", "192k",
             "-shortest", out_path], check=True)
        for p_, _ in segments:
            os.remove(p_)
        os.rmdir(seg_dir)
        total = sum(nn for _, nn in segments)
        print(f"wrote {out_path} ({total} frames in {len(segments)} "
              f"segment(s), {dur:.1f}s)")


if __name__ == "__main__":
    main()
