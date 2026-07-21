#!/bin/sh
# Borrow a voice for the voice box: text -> wav, ready for `wav=` on a
# vox track. The house voice is Kokoro-82M on MLX (Apache 2.0, ~300 MB,
# runs happily on an 8 GB machine). [voice] picks any of its ~50 voices:
# af_heart (default, warm) for songs, am_michael (low, steady) for
# Talker-circuit leads — a low source gives LPC the cleanest formant
# tracking. Falls back to the macOS system voice if the venv is absent.
#
#   scripts/borrow-voice.sh "Listen to me." renders/borrowed.wav am_michael
#
# One-time setup:
#   uv venv --python 3.12 .venv-voice
#   uv pip install --python .venv-voice/bin/python mlx-audio "misaki[en]" torch
set -e
TEXT=${1:?usage: borrow-voice.sh "text" out.wav [voice]}
OUT=${2:?usage: borrow-voice.sh "text" out.wav [voice]}
VOICE=${3:-af_heart}

REPO=$(cd "$(dirname "$0")/.." && pwd)
PY="$REPO/.venv-voice/bin/python"

if [ -x "$PY" ]; then
    TMP=$(mktemp -d)
    # mlx-audio shells out to `uv pip` for stragglers; point it at the venv
    VIRTUAL_ENV="$REPO/.venv-voice" PATH="$REPO/.venv-voice/bin:$PATH" \
        "$PY" -m mlx_audio.tts.generate \
        --model mlx-community/Kokoro-82M-bf16 \
        --voice "$VOICE" --text "$TEXT" --file_prefix "$TMP/say" >/dev/null
    mv "$TMP/say_000.wav" "$OUT"
    rm -rf "$TMP"
elif [ "$(uname)" = "Darwin" ]; then
    say ${3:+-v "$3"} -o "$OUT" --data-format=LEI16@22050 "$TEXT"
else
    echo "no TTS found: set up .venv-voice (see header) or run on macOS" >&2
    exit 1
fi
echo "wrote $OUT"
