#!/usr/bin/env python
"""Generate speech with Resemble AI's Chatterbox (open-source, MIT) for
the vocoder's wav= modulator input.

    .venv-voice/bin/python scripts/chatterbox-say.py \
        "Listen to me." renders/chatterbox.wav --exaggeration 0.7

--exaggeration is Chatterbox's emotion intensity (0.25 flat .. ~1 wild);
--cfg-weight trades adherence vs pacing (lower = slower, more deliberate);
--voice clones the speaker of a reference wav instead of the default.
First run downloads the model weights (~2-3 GB) from Hugging Face.
"""
import argparse
import os

# Ops without Metal kernels fall back to CPU instead of erroring
os.environ.setdefault("PYTORCH_ENABLE_MPS_FALLBACK", "1")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("text")
    p.add_argument("out")
    p.add_argument("--exaggeration", type=float, default=0.5)
    p.add_argument("--cfg-weight", type=float, default=0.5)
    p.add_argument("--voice", default=None, help="reference wav to clone")
    p.add_argument("--force", action="store_true", help="run even on a small-RAM machine")
    a = p.parse_args()

    # Chatterbox wants ~8 GB for itself (three fp32 model stacks + torch).
    # On a small machine it swap-storms the whole system — refuse unless
    # forced, and point at Piper (scripts/borrow-voice.sh), which runs in
    # a few hundred MB.
    ram_gb = ram_gigabytes()
    if ram_gb is not None and ram_gb < 12 and not a.force:
        raise SystemExit(
            f"this machine has {ram_gb:.0f} GB RAM; Chatterbox needs ~8 GB for "
            "itself and will thrash the system. Use scripts/borrow-voice.sh "
            "(Piper) instead, or pass --force if you really mean it."
        )

    import time

    import torch
    import torchaudio
    from chatterbox.tts import ChatterboxTTS

    device = "mps" if torch.backends.mps.is_available() else "cpu"
    print(f"loading Chatterbox on {device}...")
    try:
        model = ChatterboxTTS.from_pretrained(device=device)
        t0 = time.time()
        wav = generate(model, a)
    except Exception as e:  # some ops lack Metal kernels; CPU always works
        if device == "cpu":
            raise
        print(f"mps failed ({e}); retrying on cpu")
        device = "cpu"
        model = ChatterboxTTS.from_pretrained(device="cpu")
        t0 = time.time()
        wav = generate(model, a)
    elapsed = time.time() - t0

    torchaudio.save(a.out, wav.cpu(), model.sr)
    secs = wav.shape[-1] / model.sr
    print(
        f"wrote {a.out}: {secs:.1f}s of speech at {model.sr} Hz, "
        f"generated in {elapsed:.1f}s on {device} ({secs / elapsed:.2f}x realtime)"
    )


def generate(model, a):
    kwargs = dict(exaggeration=a.exaggeration, cfg_weight=a.cfg_weight)
    if a.voice:
        kwargs["audio_prompt_path"] = a.voice
    return model.generate(a.text, **kwargs)


def ram_gigabytes():
    try:
        import subprocess

        out = subprocess.run(
            ["sysctl", "-n", "hw.memsize"], capture_output=True, text=True
        )
        return int(out.stdout.strip()) / 2**30
    except Exception:
        return None


if __name__ == "__main__":
    main()
