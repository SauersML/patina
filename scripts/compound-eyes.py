#!/usr/bin/env python
"""Build the Compound Eyes performance: continuous Kokoro speech with a
fully AUTHORED musical performance — word timing, per-word pitch,
melisma (within-word pitch waypoints), portamento, scoops, delayed
vibrato, release falls, micro-drift — rendered as two files the engine
plays in lockstep:

    renders/compound-eyes-line.wav   the mouth (vox wav= modulator)
    renders/compound-eyes-pitch.wav  the melody (vox pitch= line,
                                     float32 MIDI notes, same clock)

    .venv-voice/bin/python scripts/compound-eyes.py

The speech side is the talkbox-score method: each phrase is ONE
utterance, Kokoro's duration predictor locates every word, WSOLA warps
words onto their scored beats (holds live in vowel nuclei). The pitch
side is built from the same score, so every glide, scoop and vibrato
is placed relative to the words it belongs to.
"""
import os

import numpy as np
import soundfile as sf

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RATE = 24000
BPM = 140.0
SLOT_BEATS = 16
SPEECH_BEATS = 15.5

D = {"D3": 50, "E3": 52, "F3": 53, "G3": 55, "A3": 57, "B3": 59, "C4": 60,
     "D4": 62, "E4": 64, "F4": 65, "G4": 67, "A4": 69, "B4": 71, "C5": 72,
     "D5": 74}

# One entry per word: (onset_beats, len_beats, level, pitch, opts).
# pitch is a note name or [(frac, note), ...] waypoints inside the word
# (melisma / within-syllable glide). opts: vib=(delay_frac, rate_hz,
# depth_cents), scoop=semitones, fall=semitones, glide=seconds.
SCORE = [
    ("af_heart", 2 ** (2 / 12),
     "I feel the searing caress of that merciless sun upon my compound eyes.",
     [("I", 0.0, 0.5, 0.8, "D4", {}),
      ("feel", 0.5, 1.0, 1.0, "F4", {"scoop": -1.5}),
      ("the", 1.5, 0.5, 0.7, "E4", {}),
      ("searing", 2.0, 1.5, 1.0, [(0, "A4"), (0.6, "G4")], {"vib": (0.5, 6.0, 30)}),
      ("caress", 3.5, 1.5, 0.9, [(0, "F4"), (0.5, "G4")], {}),
      ("of", 5.0, 0.5, 0.7, "E4", {}),
      ("that", 5.5, 0.5, 0.7, "F4", {}),
      ("merciless", 6.0, 1.5, 0.9, [(0, "E4"), (0.35, "D4"), (0.7, "C4")], {}),
      ("sun", 7.5, 2.0, 1.0, "A4", {"scoop": -2.0, "vib": (0.35, 6.0, 40)}),
      ("upon", 9.5, 1.0, 0.8, [(0, "G4"), (0.5, "A4")], {}),
      ("my", 10.5, 0.5, 0.8, "B4", {}),
      ("compound", 11.0, 1.5, 1.0, [(0, "C5"), (0.5, "B4")], {}),
      ("eyes", 12.5, 3.0, 1.0, "D5", {"scoop": -2.0, "vib": (0.3, 5.8, 45),
                                      "fall": -2.0})]),
    ("af_heart", 2 ** (2 / 12),
     "Its rays refracting into kaleidoscopic fractals that shimmer and "
     "pulsate across my vision.",
     [("Its", 0.0, 0.5, 0.8, "D4", {}),
      ("rays", 0.5, 1.5, 1.0, [(0, "F4"), (0.5, "G4")], {"vib": (0.5, 6.0, 30)}),
      ("refracting", 2.0, 1.5, 0.9, [(0, "A4"), (0.4, "G4"), (0.75, "F4")], {}),
      ("into", 3.5, 1.0, 0.7, [(0, "E4"), (0.5, "F4")], {}),
      ("kaleidoscopic", 4.5, 2.5, 1.0,
       [(0, "D4"), (0.2, "E4"), (0.4, "F4"), (0.6, "G4"), (0.8, "A4")], {}),
      ("fractals", 7.0, 1.5, 0.9, [(0, "B4"), (0.5, "A4")], {}),
      ("that", 8.5, 0.5, 0.7, "G4", {}),
      ("shimmer", 9.0, 1.5, 0.9, [(0, "A4"), (0.5, "G4")], {"vib": (0.6, 6.4, 25)}),
      ("and", 10.5, 0.5, 0.7, "F4", {}),
      ("pulsate", 11.0, 1.5, 0.9, [(0, "G4"), (0.5, "E4")], {}),
      ("across", 12.5, 1.0, 0.8, [(0, "D4"), (0.5, "E4")], {}),
      ("my", 13.5, 0.5, 0.7, "C4", {}),
      ("vision", 14.0, 2.0, 1.0, "D4", {"vib": (0.4, 5.8, 35), "fall": -1.5})]),
    ("am_michael", 1.0,
     "The relentless heat becomes a roar in my tendril antennae.",
     [("The", 0.0, 0.5, 0.8, "D3", {}),
      ("relentless", 0.5, 1.5, 0.9, [(0, "F3"), (0.4, "E3"), (0.75, "D3")], {}),
      ("heat", 2.0, 1.5, 1.0, "A3", {"scoop": -2.0, "vib": (0.4, 5.6, 35)}),
      ("becomes", 3.5, 1.5, 0.8, [(0, "G3"), (0.5, "F3")], {}),
      ("a", 5.0, 0.5, 0.7, "E3", {}),
      ("roar", 5.5, 2.5, 1.0, "B3", {"scoop": -3.0, "vib": (0.3, 5.6, 45)}),
      ("in", 8.0, 0.5, 0.7, "A3", {}),
      ("my", 8.5, 0.5, 0.7, "G3", {}),
      ("tendril", 9.0, 1.5, 0.9, [(0, "F3"), (0.5, "G3")], {}),
      ("antennae", 10.5, 3.5, 1.0, [(0, "A3"), (0.4, "G3"), (0.7, "E3")],
       {"vib": (0.5, 5.6, 35), "fall": -2.0})]),
    ("am_michael", 1.0,
     "An atonal throbbing that vibrates through my very being.",
     [("An", 0.0, 0.5, 0.8, "D3", {}),
      ("atonal", 0.5, 1.5, 0.9, [(0, "E3"), (0.35, "F3"), (0.7, "G3")], {}),
      ("throbbing", 2.0, 1.5, 1.0, [(0, "A3"), (0.5, "G3")], {"vib": (0.6, 5.6, 30)}),
      ("that", 3.5, 0.5, 0.7, "F3", {}),
      ("vibrates", 4.0, 1.5, 0.9, [(0, "G3"), (0.5, "B3")], {"vib": (0.5, 6.6, 55)}),
      ("through", 5.5, 1.0, 0.8, "A3", {}),
      ("my", 6.5, 0.5, 0.7, "G3", {}),
      ("very", 7.0, 1.0, 0.8, [(0, "F3"), (0.5, "E3")], {}),
      ("being", 8.0, 4.0, 1.0, [(0, "A3"), (0.35, "B3"), (0.65, "D4")],
       {"scoop": -2.0, "vib": (0.45, 5.8, 40), "fall": -3.0})]),
]


def synthesize(pipe, voice, text, speed):
    for r in pipe(text, voice=voice, speed=speed):
        audio = np.array(r.audio, dtype=np.float32).flatten()
        words = [(t.text, int(t.start_ts * RATE), int(t.end_ts * RATE))
                 for t in r.tokens
                 if t.phonemes and any(c.isalnum() for c in t.text)
                 and t.start_ts is not None]
        return audio, words
    raise RuntimeError(f"no audio for {text!r}")


def word_knots(audio, s0, s1, o0, o1):
    step = int(0.010 * RATE)
    n = max(1, (s1 - s0) // step)
    bounds = np.linspace(s0, s1, n + 1).astype(int)
    w = np.array([np.sqrt((audio[a:b] ** 2).mean() + 1e-9)
                  for a, b in zip(bounds[:-1], bounds[1:])])
    alloc = np.concatenate([[0.0], np.cumsum(w / w.sum())]) * (o1 - o0) + o0
    return list(zip(alloc.astype(int), bounds))


def wsola(audio, knots, out_len, win=600, hop=150, search=200):
    ko = np.array([k[0] for k in knots], dtype=np.float64)
    ks = np.array([k[1] for k in knots], dtype=np.float64)
    src = np.pad(audio, (0, 2 * win + search))
    w = np.hanning(win).astype(np.float32)
    y = np.zeros(out_len + win, dtype=np.float32)
    wsum = np.zeros(out_len + win, dtype=np.float32)
    prev = None
    for t in range(0, out_len, hop):
        target = np.interp(t, ko, ks)
        if prev is None:
            pos = int(target)
        else:
            ref = src[prev + hop:prev + hop + win]
            lo = max(0, int(target) - search)
            cands = src[lo:int(target) + search + win]
            corr = np.correlate(cands, ref, "valid")
            pos = lo + int(np.argmax(corr))
        y[t:t + win] += src[pos:pos + win] * w
        wsum[t:t + win] += w
        prev = pos
    return y[:out_len] / np.maximum(wsum[:out_len], 1e-3)


def varispeed(audio, factor):
    idx = np.arange(0, len(audio) - 1, factor)
    i0 = idx.astype(int)
    frac = (idx - i0).astype(np.float32)
    return audio[i0] * (1 - frac) + audio[i0 + 1] * frac


def note_of(p):
    return D[p] if isinstance(p, str) else D[p[0][1]]


def build_pitch(spb, slot_s):
    """The whole performance's pitch line, in MIDI notes at RATE."""
    total = int(len(SCORE) * slot_s * RATE)
    rng = np.random.default_rng(7)

    # Breakpoints (time_s, midi), linearly interpolated then smoothed
    bp = []
    prev = note_of(SCORE[0][3][0][4])
    for pi, (_v, _l, _t, timing) in enumerate(SCORE):
        base = pi * slot_s
        for word, onset, length, _vel, pitch, opts in timing:
            t0, t1 = base + onset * spb, base + (onset + length) * spb
            way = [(0.0, D[pitch])] if isinstance(pitch, str) else \
                  [(f, D[n]) for f, n in pitch]
            glide = opts.get("glide", 0.06 if way[0][1] != prev else 0.03)
            bp.append((t0, prev))
            start = way[0][1] + opts.get("scoop", 0.0)
            bp.append((t0 + glide, start))
            if "scoop" in opts:
                bp.append((t0 + glide + 0.10, way[0][1]))
            for f, m in way[1:]:
                tw = t0 + f * (t1 - t0)
                bp.append((tw - 0.02, bp[-1][1]))
                bp.append((tw + 0.02, m))
            if "fall" in opts:
                bp.append((t1 - 0.08, way[-1][1]))
                bp.append((t1, way[-1][1] + opts["fall"]))
                prev = way[-1][1]  # the fall is a gesture, not a new pitch
            else:
                bp.append((t1, way[-1][1]))
                prev = way[-1][1]
    times = np.array([t for t, _ in bp]) * RATE
    vals = np.array([m for _, m in bp], dtype=np.float64)
    midi = np.interp(np.arange(total), times, vals)

    # Soften every corner like a real CV lag (15 ms box, twice);
    # edge-pad so the ends smooth toward themselves, not toward zero
    k = int(0.015 * RATE)
    kernel = np.ones(k) / k
    for _ in range(2):
        midi = np.convolve(np.pad(midi, k, mode="edge"), kernel, "same")[k:-k]

    # Vibrato: per word, delayed onset, ramped depth, drifting rate
    for pi, (_v, _l, _t, timing) in enumerate(SCORE):
        base = pi * slot_s
        for word, onset, length, _vel, pitch, opts in timing:
            if "vib" not in opts:
                continue
            delay_f, rate_hz, cents = opts["vib"]
            t0 = base + onset * spb + delay_f * length * spb
            t1 = base + (onset + length) * spb
            a, b = int(t0 * RATE), int(t1 * RATE)
            if b <= a:
                continue
            n = b - a
            tt = np.arange(n) / RATE
            drift = 1.0 + 0.04 * np.sin(2 * np.pi * 0.7 * tt + rng.uniform(0, 6))
            phase = np.cumsum(rate_hz * drift) / RATE
            ramp = np.minimum(1.0, tt / max(1e-6, 0.25 * (t1 - t0)))
            midi[a:b] += (cents / 100.0) * ramp * np.sin(2 * np.pi * phase)

    # Micro-drift: +/- 4 cents of slow noise — a hand, not a quartz clock
    noise = rng.standard_normal(total // 1200 + 2)
    slow = np.interp(np.arange(total), np.arange(len(noise)) * 1200, noise)
    midi += 0.04 * slow
    return midi.astype(np.float32)


def main():
    from mlx_audio.tts.utils import load_model
    from mlx_audio.tts.models.kokoro import KokoroPipeline

    os.environ.setdefault("VIRTUAL_ENV", os.path.join(REPO, ".venv-voice"))
    model = load_model("mlx-community/Kokoro-82M-bf16")
    pipe = KokoroPipeline(lang_code="a", model=model,
                          repo_id="mlx-community/Kokoro-82M-bf16")

    spb = 60.0 / BPM
    slot = int(round(SLOT_BEATS * spb * RATE))
    chunks = []
    for voice, lift, text, timing in SCORE:
        audio, words = synthesize(pipe, voice, text, 1.0)
        target = SPEECH_BEATS * spb * lift
        speed = max(0.5, min(1.3, len(audio) / RATE / target))
        audio, words = synthesize(pipe, voice, text, speed)
        assert len(words) == len(timing), (
            f"{text!r}: {len(words)} words vs {len(timing)} scored\n"
            f"  heard: {[w for w, _, _ in words]}")
        out_len = int(slot * lift)
        knots = [(0, max(0, words[0][1] - int(timing[0][1] * spb * RATE * lift)))]
        for (word, s0, s1), (name, onset, length, _vel, _p, _o) in zip(words, timing):
            assert word.lower().strip(".,!?") == name.lower().strip(".,!?"), \
                (word, name)
            o0 = int(onset * spb * RATE * lift)
            o1 = int((onset + length) * spb * RATE * lift)
            knots += word_knots(audio, s0, s1, o0, o1)
        knots.append((out_len, min(len(audio),
                                   knots[-1][1] + out_len - knots[-1][0])))
        phrase = varispeed(wsola(audio, knots, out_len), lift)
        phrase = np.pad(phrase[:slot], (0, max(0, slot - len(phrase))))
        env = np.full(slot, 0.75, dtype=np.float32)
        for name, onset, length, vel, _p, _o in timing:
            a, b = int(onset * spb * RATE), int((onset + length) * spb * RATE)
            env[a:min(b, slot)] = vel
        ramp = int(0.030 * RATE)
        kernel = np.hanning(2 * ramp + 1)
        env = np.convolve(env, kernel / kernel.sum(), "same")
        phrase *= env
        phrase *= 0.12 / (np.sqrt((phrase ** 2).mean()) + 1e-9)
        print(f"{voice:12s} x{lift:.3f} speed {speed:.2f}  {text[:52]}")
        chunks.append(phrase)

    line = np.concatenate(chunks)
    line *= 0.9 / np.abs(line).max()
    sf.write(os.path.join(REPO, "renders", "compound-eyes-line.wav"), line, RATE)

    pitch = build_pitch(spb, SLOT_BEATS * spb)[:len(line)]
    sf.write(os.path.join(REPO, "renders", "compound-eyes-pitch.wav"),
             pitch, RATE, subtype="FLOAT")
    print(f"wrote line ({len(line) / RATE:.1f}s) + pitch "
          f"({pitch.min():.1f}..{pitch.max():.1f} MIDI)")


if __name__ == "__main__":
    main()
