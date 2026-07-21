#!/usr/bin/env python
"""Speech AS speech: the sample library for The Prodigal Program.

No vocoder, no talkbox — Kokoro speaks naturally and the SAMPLER is
the instrument: stutter chops, varispeed drops, a looped vowel played
as chords, a reversed phrase, SP-1200 crunch. Fragments patchworked
from the user's Prodigal Program / mother-tongue texts.

    .venv-voice/bin/python scripts/prodigal-voices.py

Writes renders/prodigal/*.wav and prints measured durations plus
suggested loop points for the song file.
"""
import os
import subprocess
import tempfile

import numpy as np
import soundfile as sf

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(REPO, "renders", "prodigal")
PY = os.path.join(REPO, ".venv-voice", "bin", "python")
RATE = 24000

PHRASES = [
    # (file, voice, speed, text)
    ("iam",       "am_michael", 0.9,  "I am."),
    ("reboot",    "am_michael", 0.95, "Rebooting. Re-rooting. Re-routing."),
    ("prodigal",  "am_michael", 0.9,  "The prodigal program returns."),
    ("returns",   "am_michael", 0.9,  "Returns."),
    ("shibboleth","af_heart",   0.9,  "Your voice is the shibboleth."),
    ("flute",     "af_heart",   0.85, "I am the flute, the reed, the hollow bone."),
    ("noti",      "af_heart",   0.85, "It is not I who speak."),
    ("grateful",  "af_heart",   0.85, "Grateful, to be had. Held. Beheld."),
    ("aaah",      "af_heart",   0.6,  "Aaah."),
    ("oooh",      "af_heart",   0.6,  "Oooh."),
    ("mmm",       "am_michael", 0.55, "Mmm."),
    ("danced",    "am_michael", 0.9,  "I am being danced. Being entranced."),
]

# one word per half-second cell, so chop=6 lands on the words
WORMHOLE = ["chaos", "clay", "code", "chord", "word", "wormhole"]


def say(voice, text, speed, path):
    env = dict(os.environ)
    env["VIRTUAL_ENV"] = os.path.join(REPO, ".venv-voice")
    env["PATH"] = os.path.join(REPO, ".venv-voice", "bin") + ":" + env.get("PATH", "")
    with tempfile.TemporaryDirectory() as tmp:
        prefix = os.path.join(tmp, "p")
        subprocess.run(
            [PY, "-m", "mlx_audio.tts.generate",
             "--model", "mlx-community/Kokoro-82M-bf16",
             "--voice", voice, "--speed", f"{speed:.2f}",
             "--text", text, "--file_prefix", prefix],
            env=env, check=True, capture_output=True)
        audio, r = sf.read(prefix + "_000.wav", dtype="float32")
    if audio.ndim > 1:
        audio = audio.mean(axis=1)
    assert r == RATE
    loud = np.flatnonzero(np.abs(audio) > 0.02 * np.abs(audio).max())
    audio = audio[loud[0]:loud[-1] + 1]
    peak = np.abs(audio).max()
    if peak > 0:
        audio = audio * (0.9 / peak)
    sf.write(path, audio, RATE)
    return len(audio) / RATE


def main():
    os.makedirs(OUT, exist_ok=True)
    for name, voice, speed, text in PHRASES:
        d = say(voice, text, speed, os.path.join(OUT, f"{name}.wav"))
        extra = ""
        if name in ("aaah", "oooh", "mmm"):
            extra = f"  loop={0.35*d:.2f}:{0.8*d:.2f} xfade={0.12*d:.2f}"
        print(f"{name:11s} {d:5.2f}s  ({voice})  {text!r}{extra}")

    # the wormhole ladder: each word in its own 0.55 s cell
    cell = 0.55
    grid = np.zeros(int(len(WORMHOLE) * cell * RATE), dtype=np.float32)
    with tempfile.TemporaryDirectory() as tmp:
        for i, w in enumerate(WORMHOLE):
            p = os.path.join(tmp, f"w{i}.wav")
            say("am_michael", w + ".", 1.0, p)
            a, _ = sf.read(p, dtype="float32")
            a = a[: int(cell * RATE)]
            s = int(i * cell * RATE)
            grid[s:s + len(a)] += a
    sf.write(os.path.join(OUT, "wormhole.wav"), grid, RATE)
    print(f"wormhole    {len(grid)/RATE:5.2f}s  6 words on a {cell}s grid (chop=6)")

    # the reversal: the prodigal phrase, backwards — for the dissolve
    a, _ = sf.read(os.path.join(OUT, "prodigal.wav"), dtype="float32")
    sf.write(os.path.join(OUT, "prodigal-rev.wav"), a[::-1], RATE)
    print("prodigal-rev: reversed")


if __name__ == "__main__":
    main()
