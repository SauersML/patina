#!/usr/bin/env python
"""Build renders/talkbox-score.wav: two Kokoro voices trading phrases,
each phrase ONE continuous utterance whose word timing is AUTHORED —
when each word lands and how long it holds — without ever cutting the
speech into pieces.

    .venv-voice/bin/python scripts/talkbox-score.py

How: Kokoro's duration predictor gives exact per-word timestamps in
its own audio (KokoroPipeline.join_timestamps). A WSOLA time-warp then
maps each word span onto its scored onset/length. WSOLA repeats or
drops whole pitch periods, so pitch and formants never move: a held
word sustains its vowel like a singer, because stretch inside a word
is allocated by local energy — the nucleus absorbs the hold while the
consonants stay crisp. Register lifts happen by varispeed AFTER the
warp (the warp targets are pre-scaled so scored beats still land).
The engine has a single vox_wav slot, so the duet is cut into the
wav, phrase by phrase.
"""
import os

import numpy as np
import soundfile as sf

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RATE = 24000
BPM = 120.0
SLOT_BEATS = 8

# The score: each phrase is one sentence spoken whole, plus one
# (onset_beats, length_beats, level) entry PER WORD. Held words are the
# talkbox move: "night" rides across the note change, "home" and
# "neon" sustain like a fist on the keys. Gentle lifts only: +2 st on
# af_heart, none on am_michael — bigger lifts shift vowel formants
# past what LPC can track (measured by ASR word recovery).
SCORE = [
    ("af_heart", 2 ** (2 / 12), "Give me one more night, one more night to remember.",
     [("Give", 0.00, 0.50, 1.0), ("me", 0.50, 0.50, 0.8),
      ("one", 1.00, 0.50, 0.9), ("more", 1.50, 0.50, 0.8),
      ("night", 2.00, 1.50, 1.0), ("one", 4.00, 0.50, 0.9),
      ("more", 4.50, 0.50, 0.8), ("night", 5.00, 1.00, 1.0),
      ("to", 6.00, 0.50, 0.7), ("remember", 6.50, 1.50, 1.0)]),
    ("am_michael", 1.0, "We are wide awake in the neon.",
     [("We", 0.00, 0.50, 0.9), ("are", 0.50, 0.50, 0.8),
      ("wide", 1.00, 1.00, 1.0), ("awake", 2.00, 1.50, 1.0),
      ("in", 3.50, 0.25, 0.7), ("the", 3.75, 0.25, 0.7),
      ("neon", 4.00, 2.50, 1.0)]),
    ("af_heart", 2 ** (2 / 12), "Every heartbeat turning over into thunder and light.",
     [("Every", 0.00, 0.75, 1.0), ("heartbeat", 0.75, 1.25, 1.0),
      ("turning", 2.00, 1.00, 0.9), ("over", 3.00, 1.00, 0.9),
      ("into", 4.00, 0.50, 0.8), ("thunder", 4.50, 1.50, 1.0),
      ("and", 6.00, 0.25, 0.7), ("light", 6.25, 1.75, 1.0)]),
    ("am_michael", 1.0, "Drive it home and never surrender.",
     [("Drive", 0.00, 0.75, 1.0), ("it", 0.75, 0.25, 0.7),
      ("home", 1.00, 2.00, 1.0), ("and", 3.00, 0.50, 0.7),
      ("never", 3.50, 1.00, 0.9), ("surrender", 4.50, 2.50, 1.0)]),
]


def synthesize(pipe, voice, text, speed):
    """One continuous utterance -> (audio, [(word, s0, s1)] in samples)."""
    for r in pipe(text, voice=voice, speed=speed):
        audio = np.array(r.audio, dtype=np.float32).flatten()
        words = [(t.text, int(t.start_ts * RATE), int(t.end_ts * RATE))
                 for t in r.tokens
                 if t.phonemes and any(c.isalnum() for c in t.text)
                 and t.start_ts is not None]
        return audio, words
    raise RuntimeError(f"no audio for {text!r}")


def word_knots(audio, s0, s1, o0, o1):
    """Warp knots inside one word: output time allocated by local
    energy, so holds live in the vowel nucleus, not the consonants."""
    step = int(0.010 * RATE)
    n = max(1, (s1 - s0) // step)
    bounds = np.linspace(s0, s1, n + 1).astype(int)
    w = np.array([np.sqrt((audio[a:b] ** 2).mean() + 1e-9)
                  for a, b in zip(bounds[:-1], bounds[1:])])
    alloc = np.concatenate([[0.0], np.cumsum(w / w.sum())]) * (o1 - o0) + o0
    return list(zip(alloc.astype(int), bounds))


def wsola(audio, knots, out_len, win=600, hop=150, search=200):
    """Time-warp audio along piecewise-linear (out, src) knots. Repeats
    or drops pitch periods; pitch and formants stay put."""
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
            # the natural continuation of the last copied frame
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
        audio, words = synthesize(pipe, voice, text, 0.8)
        assert len(words) == len(timing), (
            f"{text!r}: {len(words)} words vs {len(timing)} scored entries\n"
            f"  heard: {[w for w, _, _ in words]}")
        # the warp works in the pre-lift domain: everything scored gets
        # scaled by `lift` now, and the varispeed after brings it back
        out_len = int(slot * lift)
        knots = [(0, max(0, words[0][1] - int(timing[0][1] * spb * RATE * lift)))]
        for (word, s0, s1), (name, onset, length, _vel) in zip(words, timing):
            assert word.lower().strip(".,!?") == name.lower().strip(".,!?")
            o0 = int(onset * spb * RATE * lift)
            o1 = int((onset + length) * spb * RATE * lift)
            knots += word_knots(audio, s0, s1, o0, o1)
        knots.append((out_len, min(len(audio),
                                   knots[-1][1] + out_len - knots[-1][0])))
        warped = wsola(audio, knots, out_len)
        phrase = varispeed(warped, lift)
        phrase = np.pad(phrase[:slot], (0, max(0, slot - len(phrase))))
        # scored dynamics: per-word level, 30 ms cosine ramps
        env = np.full(slot, 0.75, dtype=np.float32)
        ramp = int(0.030 * RATE)
        for name, onset, length, vel in timing:
            a, b = int(onset * spb * RATE), int((onset + length) * spb * RATE)
            env[a:min(b, slot)] = vel
        kernel = np.hanning(2 * ramp + 1)
        env = np.convolve(env, kernel / kernel.sum(), "same")
        phrase *= env
        # common loudness so the two throats sit at one level
        phrase *= 0.12 / (np.sqrt((phrase ** 2).mean()) + 1e-9)
        print(f"{voice:12s} x{lift:.3f}  {text}")
        chunks.append(phrase)

    line = np.concatenate(chunks)
    line = np.tile(line, 2)  # the song states the 32-beat form twice
    line *= 0.9 / np.abs(line).max()
    out = os.path.join(REPO, "renders", "talkbox-score.wav")
    sf.write(out, line, RATE)
    print(f"wrote {out} ({len(line) / RATE:.1f}s)")


if __name__ == "__main__":
    main()
