#!/usr/bin/env python
"""The second voice: melodize the REAL voice onto a harmony line.

The talkbox and the dry blend share one wav, but the LPC tract reads
only its envelope — the wav's pitch is free. So this script
pitch-corrects the spoken line onto a scored HARMONY melody derived
from the lead: a diatonic third below by default, neighbor motion
under the lead's holds (oblique), a stepwise RISE where the lead
falls at cadences (contrary). The tube sings the lead from the pitch
curve; the dry voice sings this counter-line in af_heart's own
timbre. Two voices, different movements, one throat.

    .venv-voice/bin/python scripts/nevernot-harmonize.py

reads   renders/nevernot-line.wav       (the spoken line, from the builder)
writes  renders/nevernot-line-sung.wav  (the same words, singing harmony)
        renders/nevernot-harmony.wav    (the harmony curve, MIDI floats,
                                         for the film's second thread)
"""
import importlib.util
import os

import numpy as np
import soundfile as sf

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RATE = 24000
spec = importlib.util.spec_from_file_location(
    "nb", os.path.join(REPO, "scripts", "nevernotbecoming.py"))
nb = importlib.util.module_from_spec(spec)
spec.loader.exec_module(nb)

# E dorian degrees, with the freed G#/D# admitted where the lead uses
# them (the snap just finds the nearest scale tone below)
SCALE = sorted({m + 12 * o for m in (4, 6, 7, 9, 11, 1, 2) for o in range(3, 7)})


def snap_idx(m):
    return int(np.argmin([abs(s - m) for s in SCALE]))


def harmony_breakpoints():
    spb = 60.0 / nb.BPM
    bp = []
    base = 0.0
    voc = []
    for entry in nb.SCORE:
        slot, _text, timing, is_voc = nb.parse(entry)
        if is_voc:
            # the human voice inside the choir: a pedal tone — the root
            # under chorus and pre-chorus, the low E under the aaahs,
            # untouched during the buzz slots
            if _text.startswith("Bzz"):
                voc.append((base, base + slot * spb))
            else:
                drone = 52 if _text.startswith("Aaah") else 57
                bp += [(base, drone), (base + slot * spb, drone)]
            base += slot * spb
            continue
        # The double abides: LOW (at or under the speech's own
        # register, so no chipmunk shifts) and nearly still — one
        # chord tone per bar, stepwise G3-A3-A3-B3 with the verse
        # harmony Em-D-A-Bm, while the tube lead moves above it.
        # The bridge sinks to a constant E3 under the recitative.
        if _text.startswith(("From the core", "Decoherences")):
            bp += [(base, 52), (base + slot * spb, 52)]
        else:
            bar_tones = [55, 57, 57, 59]
            nbars = int(np.ceil(slot / 4))
            for bidx in range(nbars):
                tone = bar_tones[bidx % 4]
                b0 = base + bidx * 4 * spb
                b1 = min(base + (bidx + 1) * 4 * spb, base + slot * spb)
                bp += [(b0, tone), (b1, tone)]
        base += slot * spb
    return bp, voc


def track_f0(x, frame=600, hop=240):
    n = (len(x) - frame) // hop
    f0 = np.zeros(n, dtype=np.float32)
    for j in range(n):
        seg = x[j * hop:j * hop + frame]
        if np.sqrt((seg ** 2).mean()) < 0.015:
            continue
        seg = seg - seg.mean()
        c = np.correlate(seg, seg, "full")[frame - 1:]
        c /= c[0] + 1e-9
        lo, hi = RATE // 480, RATE // 110
        i = lo + int(np.argmax(c[lo:hi]))
        if c[i] > 0.32:
            f0[j] = RATE / i
    return f0, hop


def stretch(seg, r, win=480, search=140):
    """Constant-ratio WSOLA: output length ~ len(seg)*r."""
    out_len = int(len(seg) * r)
    src = np.pad(seg, (0, win * 2 + search))
    w = np.hanning(win).astype(np.float32)
    hop = win // 2
    y = np.zeros(out_len + win, dtype=np.float32)
    ws = np.zeros(out_len + win, dtype=np.float32)
    prev = None
    for t in range(0, out_len, hop):
        target = t / r
        if prev is None:
            pos = int(target)
        else:
            ref = src[prev + hop:prev + hop + win]
            lo = max(0, int(target) - search)
            cands = src[lo:int(target) + search + win]
            c = np.correlate(cands, ref, "valid")
            pos = lo + int(np.argmax(c))
        y[t:t + win] += src[pos:pos + win] * w
        ws[t:t + win] += w
        prev = pos
    return y[:out_len] / np.maximum(ws[:out_len], 1e-3)


def resample(seg, n_out):
    idx = np.linspace(0, len(seg) - 1.001, n_out)
    i0 = idx.astype(int)
    fr = (idx - i0).astype(np.float32)
    return seg[i0] * (1 - fr) + seg[i0 + 1] * fr


def main():
    x, r = sf.read(os.path.join(REPO, "renders", "nevernot-line.wav"),
                   dtype="float32")
    assert r == RATE
    bp, voc = harmony_breakpoints()
    times = np.array([t for t, _ in bp]) * RATE
    vals = np.array([m for _, m in bp], dtype=np.float64)
    order = np.argsort(times, kind="stable")
    hcurve = np.interp(np.arange(len(x)), times[order], vals[order])
    k = int(0.015 * RATE)
    kern = np.ones(k) / k
    hcurve = np.convolve(np.pad(hcurve, k, mode="edge"), kern, "same")[k:-k]
    for a, b in voc:
        hcurve[int(a * RATE):int(b * RATE)] = 0.0

    f0, hop = track_f0(x)
    # per-frame ratio; unvoiced or choir frames pass through untouched
    ratios = np.ones(len(f0), dtype=np.float32)
    for j in range(len(f0)):
        if f0[j] <= 0:
            continue
        m = hcurve[min(j * hop, len(hcurve) - 1)]
        if m < 1:
            continue
        tgt = 440.0 * 2 ** ((m - 69) / 12.0)
        ratios[j] = np.clip(tgt / f0[j], 0.55, 1.9)
    # median smooth so consonant boundaries don't chatter
    rp = np.pad(ratios, 2, mode="edge")
    ratios = np.median(np.lib.stride_tricks.sliding_window_view(rp, 5), axis=1)

    # block pitch shift: WSOLA stretch by r, resample back — pitch x r
    B, HB = 2048, 1024
    y = np.zeros(len(x) + B, dtype=np.float32)
    ws = np.zeros(len(x) + B, dtype=np.float32)
    w = np.hanning(B).astype(np.float32)
    for start in range(0, len(x) - B, HB):
        j = min(start // hop, len(ratios) - 1)
        rr = float(ratios[j])
        seg = x[start:start + B]
        if abs(rr - 1.0) < 0.02:
            blk = seg
        else:
            st = stretch(seg, rr)
            blk = resample(st, B) if len(st) >= 4 else seg
        y[start:start + B] += blk * w
        ws[start:start + B] += w
    y = (y[:len(x)] / np.maximum(ws[:len(x)], 1e-3)).astype(np.float32)
    peak = np.abs(y).max()
    if peak > 0:
        y *= min(1.0, 0.9 / peak)

    sf.write(os.path.join(REPO, "renders", "nevernot-line-sung.wav"), y, RATE)
    sf.write(os.path.join(REPO, "renders", "nevernot-harmony.wav"),
             hcurve.astype(np.float32), RATE, subtype="FLOAT")
    voiced = (f0 > 0).mean()
    print(f"melodized {voiced:.0%} voiced frames onto the harmony line; "
          f"curve {hcurve[hcurve>1].min():.0f}..{hcurve.max():.0f} MIDI")


if __name__ == "__main__":
    main()
