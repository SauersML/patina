#!/bin/sh
# Borrow a voice for the vocoder: text -> wav, ready for `wav=` on a vox
# track (or --say-style experiments). Prefers Piper, the open-source
# neural TTS, if it is on the PATH; falls back to the macOS system voice.
#
#   scripts/borrow-voice.sh "Listen to me." renders/borrowed.wav [voice]
#
# With Piper, [voice] is a model path (.onnx); with macOS say, a voice
# name like Samantha or Daniel.
set -e
TEXT=${1:?usage: borrow-voice.sh "text" out.wav [voice]}
OUT=${2:?usage: borrow-voice.sh "text" out.wav [voice]}
VOICE=${3:-}

REPO=$(cd "$(dirname "$0")/.." && pwd)
PIPER_PY="$REPO/.venv-voice/bin/python"
PIPER_MODEL="$REPO/.venv-voice/voices/en_US-lessac-medium.onnx"

if [ -x "$PIPER_PY" ] && [ -f "${VOICE:-$PIPER_MODEL}" ]; then
    # Piper: open-source neural TTS, ~60 MB voice, runs in a few hundred
    # MB of RAM. --length-scale slows delivery, --noise-scale varies it.
    echo "$TEXT" | "$PIPER_PY" -m piper -m "${VOICE:-$PIPER_MODEL}" \
        --length-scale 1.15 --noise-scale 0.75 -f "$OUT"
elif command -v piper >/dev/null 2>&1; then
    echo "$TEXT" | piper ${VOICE:+--model "$VOICE"} --output_file "$OUT"
elif [ "$(uname)" = "Darwin" ]; then
    say ${VOICE:+-v "$VOICE"} -o "$OUT" --data-format=LEI16@22050 "$TEXT"
else
    echo "no TTS found: install piper (pip install piper-tts) or run on macOS" >&2
    exit 1
fi
echo "wrote $OUT"
