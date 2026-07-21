#!/usr/bin/env python
"""Generate speech with Resemble AI's Chatterbox (open-source, MIT) for
the vocoder's wav= modulator input.

    .venv-voice/bin/python scripts/chatterbox-say.py \
        "Listen to me." renders/chatterbox.wav

Two engines:
  --turbo (default): Chatterbox Turbo — GPT2-medium backbone + meanflow
      decoder, ~2 GB of weights. Runs comfortably on an 8 GB machine on
      CPU. Expressiveness via --temperature; voice cloning via --voice.
      (Turbo ignores exaggeration/cfg by design.)
  --full: the original 0.5B Chatterbox with the --exaggeration emotion
      knob. Needs a 12+ GB machine; refuses below that unless --force.

A watchdog aborts cleanly if this process's RSS crosses --max-rss-gb
(default 5.5) so a bad estimate can never swap-storm the machine again.
First run downloads weights from Hugging Face (~2-3 GB).
"""
import argparse
import os
import subprocess
import threading
import time

# Ops without Metal kernels fall back to CPU instead of erroring
os.environ.setdefault("PYTORCH_ENABLE_MPS_FALLBACK", "1")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("text")
    p.add_argument("out")
    p.add_argument("--turbo", action="store_true", default=True)
    p.add_argument("--full", dest="turbo", action="store_false",
                   help="original Chatterbox (needs 12+ GB RAM)")
    p.add_argument("--temperature", type=float, default=0.8)
    p.add_argument("--exaggeration", type=float, default=0.5, help="--full only")
    p.add_argument("--cfg-weight", type=float, default=0.5, help="--full only")
    p.add_argument("--voice", default=None, help="reference wav to clone")
    p.add_argument("--force", action="store_true", help="skip the RAM check")
    p.add_argument("--max-rss-gb", type=float, default=5.5)
    a = p.parse_args()

    ram_gb = ram_gigabytes()
    if not a.turbo and ram_gb is not None and ram_gb < 12 and not a.force:
        raise SystemExit(
            f"this machine has {ram_gb:.0f} GB RAM; full Chatterbox will thrash "
            "it. Use the default --turbo engine, or --force if you insist."
        )
    start_watchdog(a.max_rss_gb)

    import torch
    import torchaudio

    # Small machine -> CPU: on 8 GB unified memory, MPS's extra buffer
    # allocations are the difference between fitting and thrashing
    small = ram_gb is not None and ram_gb < 12
    device = "cpu" if small else ("mps" if torch.backends.mps.is_available() else "cpu")
    engine = "Turbo" if a.turbo else "full"
    print(f"loading Chatterbox {engine} on {device} ({ram_gb:.0f} GB machine)...")

    if a.turbo:
        from chatterbox.tts_turbo import ChatterboxTurboTTS

        model = ChatterboxTurboTTS.from_pretrained(device=device)
        kwargs = dict(temperature=a.temperature)
    else:
        from chatterbox.tts import ChatterboxTTS

        model = ChatterboxTTS.from_pretrained(device=device)
        kwargs = dict(exaggeration=a.exaggeration, cfg_weight=a.cfg_weight,
                      temperature=a.temperature)
    if a.voice:
        kwargs["audio_prompt_path"] = a.voice

    t0 = time.time()
    with torch.inference_mode():
        wav = model.generate(a.text, **kwargs)
    elapsed = time.time() - t0

    torchaudio.save(a.out, wav.cpu(), model.sr)
    secs = wav.shape[-1] / model.sr
    print(
        f"wrote {a.out}: {secs:.1f}s of speech at {model.sr} Hz, "
        f"generated in {elapsed:.1f}s on {device} ({secs / elapsed:.2f}x realtime)"
    )


def start_watchdog(max_gb):
    """Kill this process before it can take the machine down with it."""
    pid = os.getpid()

    def watch():
        while True:
            try:
                rss_kb = int(subprocess.run(
                    ["ps", "-o", "rss=", "-p", str(pid)],
                    capture_output=True, text=True).stdout.strip())
            except (ValueError, OSError):
                return
            if rss_kb / 1048576 > max_gb:
                print(f"\nwatchdog: RSS passed {max_gb} GB — aborting before "
                      "the machine swaps. Try --turbo or shorter text.")
                os._exit(1)
            time.sleep(0.5)

    threading.Thread(target=watch, daemon=True).start()


def ram_gigabytes():
    try:
        out = subprocess.run(["sysctl", "-n", "hw.memsize"],
                             capture_output=True, text=True)
        return int(out.stdout.strip()) / 2**30
    except Exception:
        return None


if __name__ == "__main__":
    main()
