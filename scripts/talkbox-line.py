#!/usr/bin/env python
"""Assemble a talk-box line: each syllable synthesized SEPARATELY with
Kokoro and placed at its exact note position, so the mouth locks to the
riff instead of talking over it.

    .venv-voice/bin/python scripts/talkbox-line.py renders/line.wav 96 am_michael \
        turn:1 it:1 up:1 and:1 feel:1 the:1 sir:1 kit:1 breathe:5

Each token is syllable:beats (spell syllables phonetically — "sir kit"
reads better than "cir cuit"). The output wav starts at the first
syllable; give the song's vox track the same rhythm.
"""
import os
import subprocess
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PY = os.path.join(REPO, ".venv-voice", "bin", "python")


def main():
    if len(sys.argv) < 5:
        raise SystemExit(__doc__)
    out_path, bpm, voice = sys.argv[1], float(sys.argv[2]), sys.argv[3]
    tokens = []
    for t in sys.argv[4:]:
        word, _, beats = t.rpartition(":")
        tokens.append((word, float(beats)))

    import numpy as np
    import soundfile as sf

    env = dict(os.environ)
    env["VIRTUAL_ENV"] = os.path.join(REPO, ".venv-voice")
    env["PATH"] = os.path.join(REPO, ".venv-voice", "bin") + ":" + env.get("PATH", "")

    rate = 24000
    spb = 60.0 / bpm  # seconds per beat
    # Exact length: sum of the slots, no padding — so `chop=N` slice
    # boundaries in the sampler land exactly on the syllable slots
    total = sum(b for _, b in tokens) * spb
    line = np.zeros(int(total * rate), dtype=np.float32)

    with tempfile.TemporaryDirectory() as tmp:
        t = 0.0
        for i, (word, beats) in enumerate(tokens):
            prefix = os.path.join(tmp, f"syl{i}")
            # Stretch the mouthing toward the note length: longer notes
            # get slower delivery (Kokoro's floor is ~0.5)
            speed = max(0.5, min(1.0, 0.9 / (beats * spb)))
            subprocess.run(
                [PY, "-m", "mlx_audio.tts.generate",
                 "--model", "mlx-community/Kokoro-82M-bf16",
                 "--voice", voice, "--speed", f"{speed:.2f}",
                 "--text", word, "--file_prefix", prefix],
                env=env, check=True, capture_output=True,
            )
            audio, r = sf.read(prefix + "_000.wav", dtype="float32")
            if audio.ndim > 1:
                audio = audio.mean(axis=1)
            assert r == rate, f"unexpected rate {r}"
            # Trim Kokoro's leading/trailing silence so the syllable's
            # onset lands ON the note
            loud = np.flatnonzero(np.abs(audio) > 0.02 * np.abs(audio).max())
            audio = audio[loud[0]:loud[-1] + 1]
            start = int(t * rate)
            end = min(start + len(audio), len(line))
            line[start:end] += audio[: end - start]
            t += beats * spb
            print(f"  {word:>10s} at beat offset {t / spb - beats:.2f} "
                  f"({len(audio) / rate:.2f}s, speed {speed:.2f})")

    peak = np.abs(line).max()
    if peak > 0:
        line *= 0.9 / peak
    sf.write(out_path, line, rate)
    print(f"wrote {out_path} ({len(line) / rate:.1f}s at {rate} Hz)")


if __name__ == "__main__":
    main()
