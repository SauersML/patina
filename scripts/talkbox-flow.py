#!/usr/bin/env python
"""Build renders/talkbox-flow.wav: two Kokoro voices trading phrases,
each phrase ONE continuous utterance — coarticulated, human — for the
Talker (vox_mode 2).

    .venv-voice/bin/python scripts/talkbox-flow.py

No syllable chopping: the mouth flows through the whole line the way a
talkbox player sings into the tube, and the riff's note changes ride on
top. Each phrase is spoken naturally, its delivery speed nudged once so
it fills its 8-beat slot, then af_heart takes a small varispeed lift
into the reference's high register while am_michael stays put (his low
f0 tracks cleanest through LPC). The engine has a single vox_wav slot,
so the duet is cut into the wav, phrase by phrase.
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
SLOT_BEATS = 8
SPEECH_BEATS = 7.5  # leave half a beat of breath before the next voice

# (voice, varispeed factor, one whole spoken line). The lifts matter
# beyond style: the Talker was tuned against a high-register modulator,
# and a low-register one renders ~3x too bright above 3 kHz (measured).
# +5/+2 st splits the difference with the song's tape_age, keeping both
# voices recognizably human. The lifted voice needs longer lines: its
# slot holds more source-seconds of speech, and words fill time better
# than Kokoro's speed floor can
PHRASES = [
    ("af_heart", 2 ** (5 / 12), "Give me one more night, one more night to remember."),
    ("am_michael", 2 ** (2 / 12), "We are wide awake in the neon."),
    ("af_heart", 2 ** (5 / 12), "Every heartbeat turning over into thunder and light."),
    ("am_michael", 2 ** (2 / 12), "Drive it home and never surrender."),
]


def say(voice, text, speed, prefix):
    env = dict(os.environ)
    env["VIRTUAL_ENV"] = os.path.join(REPO, ".venv-voice")
    env["PATH"] = os.path.join(REPO, ".venv-voice", "bin") + ":" + env.get("PATH", "")
    subprocess.run(
        [PY, "-m", "mlx_audio.tts.generate",
         "--model", "mlx-community/Kokoro-82M-bf16",
         "--voice", voice, "--speed", f"{speed:.2f}",
         "--text", text, "--file_prefix", prefix],
        env=env, check=True, capture_output=True,
    )
    audio, r = sf.read(prefix + "_000.wav", dtype="float32")
    assert r == RATE
    if audio.ndim > 1:
        audio = audio.mean(axis=1)
    loud = np.flatnonzero(np.abs(audio) > 0.02 * np.abs(audio).max())
    return audio[loud[0]:loud[-1] + 1]


def main():
    spb = 60.0 / BPM
    slot = int(round(SLOT_BEATS * spb * RATE))
    chunks = []
    with tempfile.TemporaryDirectory() as tmp:
        for i, (voice, factor, text) in enumerate(PHRASES):
            prefix = os.path.join(tmp, f"phrase{i}")
            # speech must fill the slot BEFORE the varispeed lift shortens it
            target = SPEECH_BEATS * spb * factor
            audio = say(voice, text, 1.0, prefix)
            # one calibration pass: Kokoro's speed knob stretches delivery
            # without touching pitch, so the phrase stays human
            speed = max(0.5, min(1.3, len(audio) / RATE / target))
            audio = say(voice, text, speed, prefix)
            # overruns get varispeed-fit into the slot rather than
            # truncated — never clip the tail off the last word
            fit = factor * max(1.0, len(audio) / (slot * factor))
            if fit != 1.0:
                # varispeed: read faster -> pitch and formants lift together
                idx = np.arange(0, len(audio) - 1, fit)
                i0 = idx.astype(int)
                frac = (idx - i0).astype(np.float32)
                audio = audio[i0] * (1 - frac) + audio[i0 + 1] * frac
            print(f"phrase {i}: {voice} x{fit:.3f} speed {speed:.2f} "
                  f"-> {len(audio) / RATE:.2f}s of {slot / RATE:.2f}s slot")
            audio = audio[:slot]
            chunks.append(np.pad(audio, (0, slot - len(audio))))

    line = np.concatenate(chunks)
    line = np.tile(line, 2)  # the song states the 32-beat form twice
    line *= 0.9 / np.abs(line).max()
    out = os.path.join(REPO, "renders", "talkbox-flow.wav")
    sf.write(out, line, RATE)
    print(f"wrote {out} ({len(line) / RATE:.1f}s)")


if __name__ == "__main__":
    main()
