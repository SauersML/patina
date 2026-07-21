#!/usr/bin/env python
"""Build renders/talkbox-duet.wav: two Kokoro voices trading 8-beat
phrases on one beat grid, ready for the Talker (vox_mode 2).

    .venv-voice/bin/python scripts/talkbox-duet.py

af_heart phrases are generated slow and varispeed-pitched up +3 st
(the faster-higher recipe); am_michael stays natural — his low f0
gives LPC the cleanest formant tracking, and the contrast between the
two throats is the point. The engine has a single vox_wav slot, so
the duet is cut into the wav itself, phrase by phrase.
"""
import os
import subprocess
import tempfile

import numpy as np
import soundfile as sf

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PY = os.path.join(REPO, ".venv-voice", "bin", "python")
RATE = 24000
BPM = 120.0

# (voice, varispeed factor, tokens) — 8 beats each, syllable per beat
PHRASES = [
    ("af_heart", 2 ** (3 / 12),
     ["hold:1", "on:1", "ty:1", "ter:1", "chase:1", "the:1", "light:1", "now:1"]),
    ("am_michael", 1.0,
     ["we:1", "could:1", "run:1", "all:1", "night:1", "for:1", "ev:1", "er:1"]),
    ("af_heart", 2 ** (3 / 12),
     ["ev:1", "ry:1", "co:1", "lor:1", "turns:1", "e:1", "lec:1", "tric:1"]),
    ("am_michael", 1.0,
     ["hold:1", "the:1", "fee:1", "ling:1", "ne:1", "ver:1", "let:1", "go:1"]),
]


def main():
    spb = 60.0 / BPM
    chunks = []
    with tempfile.TemporaryDirectory() as tmp:
        for i, (voice, factor, tokens) in enumerate(PHRASES):
            path = os.path.join(tmp, f"phrase{i}.wav")
            # generate slower by `factor` so the varispeed-up lands
            # back on the song's grid
            subprocess.run(
                [PY, os.path.join(REPO, "scripts", "talkbox-line.py"),
                 path, f"{BPM / factor}", voice, *tokens],
                check=True,
            )
            audio, r = sf.read(path, dtype="float32")
            assert r == RATE
            if factor != 1.0:
                # varispeed: read faster -> pitch and formants up together
                idx = np.arange(0, len(audio) - 1, factor)
                i0 = idx.astype(int)
                frac = (idx - i0).astype(np.float32)
                audio = audio[i0] * (1 - frac) + audio[i0 + 1] * frac
            # pin to exactly 8 beats so phrase boundaries stay on the grid
            want = int(round(8 * spb * RATE))
            if len(audio) < want:
                audio = np.pad(audio, (0, want - len(audio)))
            else:
                audio = audio[:want]
            print(f"phrase {i}: {voice} x{factor:.3f} -> {len(audio) / RATE:.2f}s")
            chunks.append(audio)

    line = np.concatenate(chunks)
    line = np.tile(line, 2)  # the song states the 32-beat form twice
    line *= 0.9 / np.abs(line).max()
    out = os.path.join(REPO, "renders", "talkbox-duet.wav")
    sf.write(out, line, RATE)
    print(f"wrote {out} ({len(line) / RATE:.1f}s)")


if __name__ == "__main__":
    main()
