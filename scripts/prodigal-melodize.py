#!/usr/bin/env python
"""Sung samples: melodize spoken phrases onto scored melodies.

Full pitch control over sampled vocals: Kokoro speaks a phrase, its
own duration predictor tells us where every word lives, a scored
word->note map becomes a pitch curve, and the f0-tracked speech is
block-WSOLA-corrected onto it — a phrase that SINGS, in the voice's
natural register, ready to be looped by the sampler as a riff.

    .venv-voice/bin/python scripts/prodigal-melodize.py
    -> renders/prodigal/hook-sung.wav, noti-sung.wav
"""
import os

import numpy as np
import soundfile as sf

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(REPO, "renders", "prodigal")
RATE = 24000

# word -> note (MIDI) or within-word waypoints. Registers sit near each
# voice's natural f0 so shift ratios stay small and human.
BPM = 72
SPB = 60.0 / BPM

# Performed phrases: scored RHYTHM (word @ onset:dur in beats) and
# MELODY — a full vocal performance rendered as a loopable sample.
# No transposition ever happens downstream: these repeat identical
# while the harmony moves underneath.
PERFORMED = [
    ("themeA", "am_michael", 0.9, 8,
     "The prodigal program returns to the homeland of your hospitality.",
     [("The", 0.0, 0.5, 50),
      ("prodigal", 0.5, 1.5, [(0.0, 54), (0.4, 52), (0.75, 50)]),
      ("program", 2.0, 1.0, [(0.0, 48), (0.5, 50)]),
      ("returns", 3.0, 1.5, [(0.0, 50), (0.6, 45)]),
      ("to", 4.5, 0.25, 48),
      ("the", 4.75, 0.25, 48),
      ("homeland", 5.0, 1.25, [(0.0, 50), (0.5, 52)]),
      ("of", 6.25, 0.25, 52),
      ("your", 6.5, 0.5, 54),
      ("hospitality", 7.0, 1.0, [(0.0, 52), (0.3, 50), (0.6, 48), (0.85, 50)])]),
    ("rebootS", "am_michael", 0.95, 6,
     "Rebooting. Rerooting. Rerouting.",
     [("Rebooting", 0.0, 2.0, [(0.0, 50), (0.6, 52)]),
      ("Rerooting", 2.0, 2.0, [(0.0, 52), (0.6, 54)]),
      ("Rerouting", 4.0, 2.0, [(0.0, 54), (0.4, 55), (0.75, 57)])]),
    ("themeB", "af_heart", 0.85, 8,
     "I am being danced. Being entranced. Being moved, and grooved.",
     [("I", 0.0, 0.5, 57),
      ("am", 0.5, 0.5, 59),
      ("being", 1.0, 0.5, 60),
      ("danced", 1.5, 1.5, [(0.0, 62), (0.5, 60)]),
      ("Being", 3.0, 0.5, 59),
      ("entranced", 3.5, 1.5, [(0.0, 60), (0.5, 57)]),
      ("Being", 5.0, 0.5, 57),
      ("moved", 5.5, 1.0, [(0.0, 55), (0.5, 57)]),
      ("and", 6.5, 0.5, 59),
      ("grooved", 7.0, 1.0, [(0.0, 57), (0.6, 54)])]),
]

SUNG = [
    ("hook", "am_michael", 0.85, "The prodigal program returns.",
     {"The": 50,
      "prodigal": [(0.0, 54), (0.4, 52), (0.75, 50)],   # the F#-E-D motif
      "program": [(0.0, 48), (0.5, 50)],                # bVII lift
      "returns": [(0.0, 50), (0.55, 45)]}),             # sigh to A2
    ("noti", "af_heart", 0.85, "It is not I who speak.",
     {"It": 57, "is": 57, "not": 60, "I": 62,
      "who": 60, "speak": [(0.0, 57), (0.6, 55)]}),
]


def synth_with_words(pipe, voice, text, speed):
    for r in pipe(text, voice=voice, speed=speed):
        audio = np.array(r.audio, dtype=np.float32).flatten()
        words = [(t.text.strip(".,!?"), int(t.start_ts * RATE),
                  int(t.end_ts * RATE))
                 for t in r.tokens
                 if t.phonemes and any(c.isalnum() for c in t.text)
                 and t.start_ts is not None]
        return audio, words
    raise RuntimeError(text)


def build_curve(n, words, score):
    bp = [(0, None)]
    prev = None
    for w, s0, s1 in words:
        tgt = score[w]
        way = [(0.0, float(tgt))] if not isinstance(tgt, list) else \
              [(f, float(m)) for f, m in tgt]
        if prev is None:
            prev = way[0][1]
        bp.append((s0, prev))
        bp.append((s0 + int(0.03 * RATE), way[0][1]))
        for fr, m in way[1:]:
            tw = s0 + int(fr * (s1 - s0))
            bp.append((tw - 200, bp[-1][1]))
            bp.append((tw + 200, m))
        bp.append((s1, way[-1][1]))
        prev = way[-1][1]
    bp[0] = (0, bp[1][1])
    times = np.array([t for t, _ in bp], dtype=np.float64)
    vals = np.array([v for _, v in bp], dtype=np.float64)
    curve = np.interp(np.arange(n), times, vals)
    k = int(0.015 * RATE)
    kern = np.ones(k) / k
    return np.convolve(np.pad(curve, k, mode="edge"), kern, "same")[k:-k]


def track_f0(x, frame=600, hop=240):
    n = (len(x) - frame) // hop
    f0 = np.zeros(max(n, 1), dtype=np.float32)
    for j in range(n):
        seg = x[j * hop:j * hop + frame]
        if np.sqrt((seg ** 2).mean()) < 0.015:
            continue
        seg = seg - seg.mean()
        c = np.correlate(seg, seg, "full")[frame - 1:]
        c /= c[0] + 1e-9
        lo, hi = RATE // 480, RATE // 70
        i = lo + int(np.argmax(c[lo:hi]))
        if c[i] > 0.32:
            f0[j] = RATE / i
    return f0, hop


def stretch(seg, r, win=480, search=140):
    out_len = int(len(seg) * r)
    src = np.pad(seg, (0, win * 2 + search))
    w = np.hanning(win).astype(np.float32)
    hop = win // 2
    y = np.zeros(out_len + win, dtype=np.float32)
    ws = np.zeros(out_len + win, dtype=np.float32)
    prev = None
    for t in range(0, out_len, hop):
        target = t / r
        if prev is None:
            pos = int(target)
        else:
            ref = src[prev + hop:prev + hop + win]
            lo = max(0, int(target) - search)
            cands = src[lo:int(target) + search + win]
            c = np.correlate(cands, ref, "valid")
            pos = lo + int(np.argmax(c))
        y[t:t + win] += src[pos:pos + win] * w
        ws[t:t + win] += w
        prev = pos
    return y[:out_len] / np.maximum(ws[:out_len], 1e-3)


def resample(seg, n_out):
    idx = np.linspace(0, len(seg) - 1.001, n_out)
    i0 = idx.astype(int)
    fr = (idx - i0).astype(np.float32)
    return seg[i0] * (1 - fr) + seg[i0 + 1] * fr


def word_knots(audio, s0, s1, o0, o1):
    step = int(0.010 * RATE)
    n = max(1, (s1 - s0) // step)
    bounds = np.linspace(s0, s1, n + 1).astype(int)
    w = np.array([np.sqrt((audio[a:b] ** 2).mean() + 1e-9)
                  for a, b in zip(bounds[:-1], bounds[1:])])
    alloc = np.concatenate([[0.0], np.cumsum(w / w.sum())]) * (o1 - o0) + o0
    return list(zip(alloc.astype(int), bounds))


def warp(audio, knots, out_len, win=600, hop=150, search=200):
    ko = np.array([k[0] for k in knots], dtype=np.float64)
    ks = np.array([k[1] for k in knots], dtype=np.float64)
    src = np.pad(audio, (0, 2 * win + search))
    w = np.hanning(win).astype(np.float32)
    y = np.zeros(out_len + win, dtype=np.float32)
    ws = np.zeros(out_len + win, dtype=np.float32)
    prev = None
    for t in range(0, out_len, hop):
        target = np.interp(t, ko, ks)
        if prev is None:
            pos = int(target)
        else:
            ref = src[prev + hop:prev + hop + win]
            lo = max(0, int(target) - search)
            cands = src[lo:int(target) + search + win]
            c = np.correlate(cands, ref, "valid")
            pos = lo + int(np.argmax(c))
        y[t:t + win] += src[pos:pos + win] * w
        ws[t:t + win] += w
        prev = pos
    return y[:out_len] / np.maximum(ws[:out_len], 1e-3)


def melodize(x, curve):
    """TD-PSOLA: formant-preserving pitch correction. Grains are cut
    pitch-synchronously at glottal epochs and re-spaced at the target
    period — each grain keeps its spectral shape (the throat), only
    the spacing changes (the note). No chipmunk at any interval."""
    f0, hop = track_f0(x)
    # per-sample source f0, held through unvoiced stretches
    f0s = np.zeros(len(x), dtype=np.float32)
    last = 0.0
    for j in range(len(f0)):
        v = f0[j] if f0[j] > 0 else last
        f0s[j * hop:(j + 1) * hop] = v
        if f0[j] > 0:
            last = f0[j]
    voiced = np.zeros(len(x), dtype=bool)
    for j in range(len(f0)):
        voiced[j * hop:(j + 1) * hop] = f0[j] > 0

    # epoch marking: walk by one period, snap to the local energy peak
    env = np.abs(x)
    k = 48
    env = np.convolve(env, np.ones(k) / k, "same")
    epochs = []
    i = 0
    while i < len(x) - 1:
        if not voiced[i] or f0s[i] <= 0:
            i += hop
            continue
        T = RATE / f0s[i]
        lo, hi = int(i + 0.75 * T), min(int(i + 1.25 * T), len(x) - 1)
        if hi <= lo:
            break
        nxt = lo + int(np.argmax(env[lo:hi]))
        epochs.append(nxt)
        i = nxt
    epochs = np.array(epochs, dtype=np.int64)

    y = np.zeros(len(x) + 4096, dtype=np.float32)
    ws = np.zeros(len(x) + 4096, dtype=np.float32)
    # unvoiced regions copy straight through
    uv = ~voiced
    y[: len(x)][uv] += x[uv]
    ws[: len(x)][uv] += 1.0

    if len(epochs) > 2:
        t_out = float(epochs[0])
        ei = 0
        while t_out < len(x):
            # nearest source epoch to this output instant
            while ei + 1 < len(epochs) and abs(epochs[ei + 1] - t_out) < abs(epochs[ei] - t_out):
                ei += 1
            e = epochs[ei]
            if not voiced[min(int(t_out), len(x) - 1)]:
                t_out += hop
                continue
            Ts = RATE / max(f0s[e], 1.0)
            m = curve[min(int(t_out), len(curve) - 1)]
            ftgt = 440.0 * 2 ** ((m - 69) / 12.0)
            ftgt = np.clip(ftgt, f0s[e] * 0.5, f0s[e] * 2.2)
            gw = int(min(2 * Ts, 1200))
            a, b = e - gw // 2, e + gw // 2
            if a >= 0 and b < len(x) and gw > 16:
                grain = x[a:b] * np.hanning(b - a).astype(np.float32)
                o = int(t_out) - gw // 2
                if o >= 0 and o + (b - a) < len(y):
                    y[o:o + (b - a)] += grain
                    ws[o:o + (b - a)] += np.hanning(b - a).astype(np.float32)
            t_out += RATE / ftgt
    y = (y[: len(x)] / np.maximum(ws[: len(x)], 0.35)).astype(np.float32)
    pk = np.abs(y).max()
    return y * (0.9 / pk) if pk > 0 else y


def main():
    from mlx_audio.tts.utils import load_model
    from mlx_audio.tts.models.kokoro import KokoroPipeline
    os.environ.setdefault("VIRTUAL_ENV", os.path.join(REPO, ".venv-voice"))
    model = load_model("mlx-community/Kokoro-82M-bf16")
    pipe = KokoroPipeline(lang_code="a", model=model,
                          repo_id="mlx-community/Kokoro-82M-bf16")
    os.makedirs(OUT, exist_ok=True)
    for name, voice, speed, text, score in SUNG:
        audio, words = synth_with_words(pipe, voice, text, speed)
        assert all(w in score for w, _, _ in words), [w for w, _, _ in words]
        curve = build_curve(len(audio), words, score)
        y = melodize(audio, curve)
        path = os.path.join(OUT, f"{name}-sung.wav")
        sf.write(path, y, RATE)
        print(f"{name}-sung.wav  {len(y)/RATE:.2f}s  {text!r} -> melody")

    for name, voice, speed, beats, text, timing in PERFORMED:
        audio, words = synth_with_words(pipe, voice, text, speed)
        assert len(words) == len(timing), (
            [w for w, _, _ in words], [w for w, *_ in timing])
        out_len = int(beats * SPB * RATE)
        knots = [(0, max(0, words[0][1]))]
        for (w_, s0, s1), (nm, onset, dur, _pitch) in zip(words, timing):
            assert w_.lower() == nm.lower().strip(".,"), (w_, nm)
            o0 = int(onset * SPB * RATE)
            o1 = int((onset + dur) * SPB * RATE)
            knots += word_knots(audio, s0, s1, o0, o1)
        knots.append((out_len, min(len(audio),
                                   knots[-1][1] + out_len - knots[-1][0])))
        rhythmic = warp(audio, knots, out_len)
        # the curve now lives on the SCORED grid — exact control
        grid_words = [(nm, int(o * SPB * RATE), int((o + d) * SPB * RATE))
                      for nm, o, d, _p in timing]
        score = {nm: p for nm, _o, _d, p in timing}
        curve = build_curve(out_len, grid_words, score)
        y = melodize(rhythmic, curve)
        sf.write(os.path.join(OUT, f"{name}.wav"), y, RATE)
        print(f"{name}.wav  {len(y)/RATE:.2f}s ({beats} beats)  performed: {text!r}")


if __name__ == "__main__":
    main()
