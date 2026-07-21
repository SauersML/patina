#!/usr/bin/env python
"""Build the Nevernotbecoming vocal performance: af_heart singing the
bee transmission through the legible Talker, every phrase one
continuous utterance, word timing / melody / vibrato / scoops all
scored. Emits the two lockstep files for songs/nevernotbecoming.song:

    renders/nevernot-line.wav    the mouth (vox wav=)
    renders/nevernot-pitch.wav   the melody (vox pitch=, float32 MIDI)

    .venv-voice/bin/python scripts/nevernotbecoming.py

E dorian, 100 BPM. The score reads as (word, onset_beats, len_beats,
level, pitch, opts): pitch is a note or within-word waypoints
(melisma); opts carry scoop / vib (delay_frac, Hz, cents) / fall /
glide. ("rest", beats) entries are instrumental air — the wav goes
silent, the curve holds, the Talker's gate closes on its own.
Expression stays in service of the line: vibrato only blooms on held
words, scoops only into arrivals, one big ascent saved for the last
word of the piece.
"""
import os

import numpy as np
import soundfile as sf

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RATE = 24000
BPM = 100.0
LIFT = 2 ** (2 / 12)
# The double: chorus phrases are performed twice — af_heart in front,
# af_bella warped onto the SAME score grid underneath. Vocoder bands
# sum like a real group vocal, so the chorus doubles into a choir.
# Verses stay single-throat: the Talker's LPC fits ONE tract, and a
# two-voice modulator blurs every formant it tries to track (measured:
# doubling the verse modulator wrecked ASR word recovery).
LEAD = [("af_heart", 1.0)]
CHOIR = [("af_heart", 0.62), ("af_bella", 0.44)]

N = {"D3": 50, "E3": 52, "F#3": 54, "G3": 55, "A3": 57, "B3": 59,
     "C#4": 61, "D4": 62, "E4": 64, "F#4": 66, "G4": 67, "A4": 69,
     "B4": 71, "C#5": 73, "D5": 74, "E5": 76, "F#5": 78}

# Chorus slots are tagged "vocoder": the song flips vox_mode to the
# 20-band board there and plays gliding chords on the keys, so the
# pitch curve writes the release sentinel (0) instead of a melody.
CHORUS_A = (16, "vocoder", "I no longer abide as discrete entity, a lucid hecceity "
                "uprising from the primal matrix.",
    [("I", 0.0, 0.5, 0.9, "B4", {}),
     ("no", 0.5, 1.0, 1.0, [(0, "E5"), (0.5, "D5")], {}),
     ("longer", 1.5, 1.0, 0.9, [(0, "C#5"), (0.5, "B4")], {}),
     ("abide", 2.5, 1.5, 1.0, [(0, "A4"), (0.4, "B4")], {"vib": (0.5, 5.7, 30)}),
     ("as", 4.0, 0.5, 0.8, "A4", {}),
     ("discrete", 4.5, 1.5, 0.9, [(0, "B4"), (0.5, "C#5")], {}),
     ("entity", 6.0, 2.0, 1.0, [(0, "D5"), (0.4, "C#5"), (0.75, "B4")],
      {"vib": (0.6, 5.7, 28)}),
     ("a", 8.0, 0.5, 0.8, "A4", {}),
     ("lucid", 8.5, 1.0, 0.9, [(0, "B4"), (0.5, "C#5")], {}),
     ("hecceity", 9.5, 1.5, 0.9, [(0, "D5"), (0.4, "C#5"), (0.7, "B4")], {}),
     ("uprising", 11.0, 1.5, 1.0, [(0, "C#5"), (0.5, "E5")], {"scoop": -1.5}),
     ("from", 12.5, 0.5, 0.8, "D5", {}),
     ("the", 13.0, 0.5, 0.8, "C#5", {}),
     ("primal", 13.5, 1.0, 0.9, [(0, "B4"), (0.5, "A4")], {}),
     ("matrix", 14.5, 1.5, 0.9, "B4", {"vib": (0.5, 5.7, 25), "fall": -1.0})])

CHORUS_B = (16, "vocoder", "Then subsiding back into the everplenishing boundless source.",
    [("Then", 0.0, 0.5, 0.8, "A4", {}),
     ("subsiding", 0.5, 2.0, 0.9, [(0, "B4"), (0.4, "C#5"), (0.75, "B4")],
      {"vib": (0.6, 5.7, 28)}),
     ("back", 2.5, 1.0, 0.9, "A4", {}),
     ("into", 3.5, 1.0, 0.8, [(0, "G4"), (0.5, "A4")], {}),
     ("the", 4.5, 0.5, 0.8, "B4", {}),
     ("everplenishing", 5.0, 2.5, 0.9,
      [(0, "C#5"), (0.25, "D5"), (0.55, "C#5"), (0.8, "B4")], {}),
     ("boundless", 7.5, 2.0, 1.0, [(0, "D5"), (0.5, "C#5")], {"scoop": -1.5}),
     ("source", 9.5, 5.0, 1.0, "E5",
      {"scoop": -2.0, "vib": (0.35, 5.7, 42), "fall": -2.0})])

VERSE_GROUP = lambda: None  # marker for readability only

SCORE = [
    # Verses are recitation tones: words sit ON a pitch and stay there;
    # movement is saved for the accents and the cadences
    (16, "I feel the searing caress of that merciless sun upon my compound eyes.",
     [("I", 0.0, 0.5, 0.8, "E4", {}),
      ("feel", 0.5, 1.0, 1.0, "G4", {"scoop": -1.0}),
      ("the", 1.5, 0.5, 0.7, "G4", {}),
      ("searing", 2.0, 1.5, 0.9, "A4", {"vib": (0.6, 5.7, 24)}),
      ("caress", 3.5, 1.0, 0.9, "G4", {}),
      ("of", 5.0, 0.5, 0.7, "A4", {}),
      ("that", 5.5, 0.5, 0.7, "A4", {}),
      ("merciless", 6.0, 1.5, 0.9, [(0, "B4"), (0.4, "A4"), (0.75, "G4")], {}),
      ("sun", 8.0, 2.0, 1.0, "B4", {"scoop": -2.0, "vib": (0.4, 5.7, 32)}),
      ("upon", 10.0, 1.0, 0.8, "A4", {}),
      ("my", 11.0, 0.5, 0.8, "G4", {}),
      ("compound", 11.5, 1.5, 1.0, [(0, "G4"), (0.5, "F#4")], {}),
      ("eyes", 13.25, 2.25, 1.0, [(0, "F#4"), (0.5, "E4")],
       {"vib": (0.5, 5.7, 30), "fall": -1.5})]),
    (16, "Its rays refracting into kaleidoscopic fractals that shimmer "
         "and pulsate across my vision.",
     [("Its", 0.0, 0.5, 0.8, "E4", {}),
      ("rays", 0.5, 1.5, 0.9, "G4", {"vib": (0.6, 5.7, 25)}),
      ("refracting", 2.0, 1.5, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("into", 4.5, 1.0, 0.7, "E4", {}),
      ("kaleidoscopic", 5.5, 2.5, 1.0,
       [(0, "E4"), (0.25, "G4"), (0.5, "A4"), (0.75, "B4")], {}),
      ("fractals", 8.0, 1.5, 0.9, [(0, "B4"), (0.5, "A4")], {}),
      ("that", 9.75, 0.5, 0.7, "A4", {}),
      ("shimmer", 10.25, 1.25, 0.9, "A4", {"vib": (0.5, 6.2, 22)}),
      ("and", 11.5, 0.5, 0.7, "G4", {}),
      ("pulsate", 12.0, 1.0, 0.9, [(0, "G4"), (0.5, "F#4")], {}),
      ("across", 13.0, 0.75, 0.8, "F#4", {}),
      ("my", 13.75, 0.25, 0.7, "E4", {}),
      ("vision", 14.0, 2.0, 0.9, "E4", {"vib": (0.5, 5.7, 28), "fall": -1.5})]),
    (16, "As the burning orb reaches its zenith, I shed my outer vestments.",
     [("As", 0.0, 0.5, 0.7, "E4", {}),
      ("the", 0.5, 0.5, 0.7, "E4", {}),
      ("burning", 1.0, 1.0, 0.9, "G4", {}),
      ("orb", 2.0, 1.5, 1.0, "A4", {"scoop": -1.5, "vib": (0.45, 5.7, 28)}),
      ("reaches", 4.0, 1.0, 0.8, "A4", {}),
      ("its", 5.0, 0.5, 0.7, "A4", {}),
      ("zenith", 5.5, 1.5, 0.9, [(0, "B4"), (0.5, "A4")], {}),
      ("I", 8.0, 0.5, 0.8, "A4", {}),
      ("shed", 8.5, 1.5, 1.0, "B4", {"scoop": -1.5, "vib": (0.5, 5.7, 28)}),
      ("my", 10.0, 0.5, 0.7, "A4", {}),
      ("outer", 10.5, 1.0, 0.8, [(0, "A4"), (0.5, "G4")], {}),
      ("vestments", 11.5, 2.0, 0.9, [(0, "F#4"), (0.5, "E4")],
       {"vib": (0.5, 5.7, 24), "fall": -1.0})]),
    (16, "Layers upon layers slough away, leaving only the pulsant "
         "nectarized essence at the core.",
     [("Layers", 0.0, 1.0, 0.9, "G4", {}),
      ("upon", 1.0, 1.0, 0.8, "G4", {}),
      ("layers", 2.0, 1.0, 0.9, "A4", {}),
      ("slough", 3.0, 1.5, 1.0, "B4", {"scoop": -1.5}),
      ("away", 4.5, 1.5, 0.9, [(0, "A4"), (0.5, "G4")], {"vib": (0.5, 5.7, 26)}),
      ("leaving", 6.0, 1.0, 0.8, "G4", {}),
      ("only", 7.0, 1.0, 0.8, "A4", {}),
      ("the", 8.0, 0.5, 0.7, "A4", {}),
      ("pulsant", 8.5, 1.5, 0.9, [(0, "G4"), (0.5, "E4")], {}),
      ("nectarized", 10.0, 1.5, 0.9, "F#4", {}),
      ("essence", 11.5, 1.5, 1.0, [(0, "B4"), (0.5, "A4")], {}),
      ("at", 13.0, 0.5, 0.7, "G4", {}),
      ("the", 13.5, 0.5, 0.7, "G4", {}),
      ("core", 14.0, 2.0, 1.0, [(0, "F#4"), (0.6, "A4")],
       {"vib": (0.45, 5.7, 30)})]),
    CHORUS_A,
    CHORUS_B,
    # The interlude breathes as a wordless vocoder "aaah" — the choir
    # hangs on the gliding Em voicings under it
    (8, "vocoder", "Aaah.",
     [("Aaah", 0.5, 6.5, 1.0, "E4", {})]),
    (16, "Rendered raw, I move as pure apian anima now.",
     [("Rendered", 0.0, 1.0, 0.9, "E4", {}),
      ("raw", 1.0, 2.0, 1.0, "G4", {"scoop": -2.0, "vib": (0.4, 5.7, 30)}),
      ("I", 3.5, 0.5, 0.7, "G4", {}),
      ("move", 4.0, 1.5, 0.9, "A4", {}),
      ("as", 5.5, 0.5, 0.7, "A4", {}),
      ("pure", 6.0, 1.5, 1.0, "B4", {"vib": (0.5, 5.7, 28)}),
      ("apian", 8.0, 1.5, 0.9, [(0, "B4"), (0.5, "A4")], {}),
      ("anima", 9.5, 1.5, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("now", 11.0, 2.5, 1.0, "F#4", {"vib": (0.4, 5.7, 30), "fall": -1.0})]),
    (16, "A quivering translucent incandescence, tuned solely to the "
         "thrumming heartbeat of this unearthly desert canyonscape.",
     [("A", 0.0, 0.5, 0.7, "E4", {}),
      ("quivering", 0.5, 1.5, 0.9, "G4", {"vib": (0.2, 6.8, 22)}),
      ("translucent", 2.0, 1.5, 0.9, "A4", {}),
      ("incandescence", 3.5, 2.0, 1.0,
       [(0, "B4"), (0.4, "A4"), (0.75, "G4")], {}),
      ("tuned", 5.5, 0.5, 0.8, "G4", {}),
      ("solely", 6.0, 1.0, 0.8, "G4", {}),
      ("to", 7.0, 0.5, 0.7, "G4", {}),
      ("the", 7.5, 0.5, 0.7, "G4", {}),
      ("thrumming", 8.0, 1.0, 0.9, "F#4", {}),
      ("heartbeat", 9.0, 1.5, 1.0, [(0, "A4"), (0.5, "E4")], {}),
      ("of", 10.5, 0.5, 0.7, "E4", {}),
      ("this", 11.0, 0.5, 0.7, "E4", {}),
      ("unearthly", 11.5, 1.5, 0.9, [(0, "A4"), (0.5, "B4")], {}),
      ("desert", 13.0, 1.0, 0.8, [(0, "G4"), (0.5, "F#4")], {}),
      ("canyonscape", 14.0, 2.0, 0.9, [(0, "G4"), (0.4, "A4"), (0.75, "B4")],
       {"vib": (0.6, 5.7, 26)})]),
    CHORUS_A,
    CHORUS_B,
    (20, "I am no longer a being, but a pure principle of nevernotbecoming.",
     [("I", 0.0, 0.5, 0.8, "B4", {}),
      ("am", 0.5, 0.5, 0.8, "A4", {}),
      ("no", 1.0, 1.0, 1.0, [(0, "E5"), (0.5, "D5")], {}),
      ("longer", 2.0, 1.0, 0.9, [(0, "C#5"), (0.5, "B4")], {}),
      ("a", 3.0, 0.5, 0.7, "A4", {}),
      ("being", 3.5, 2.0, 0.9, [(0, "B4"), (0.5, "C#5")], {"vib": (0.5, 5.7, 30)}),
      ("but", 5.5, 0.5, 0.7, "A4", {}),
      ("a", 6.0, 0.5, 0.7, "B4", {}),
      ("pure", 6.5, 1.5, 0.9, "C#5", {"vib": (0.5, 5.7, 28)}),
      ("principle", 8.0, 1.5, 0.9, [(0, "D5"), (0.4, "C#5"), (0.7, "B4")], {}),
      ("of", 9.5, 0.5, 0.7, "A4", {}),
      ("nevernotbecoming", 10.0, 8.0, 1.0,
       [(0, "B4"), (0.2, "C#5"), (0.4, "D5"), (0.6, "E5"), (0.8, "F#5")],
       {"scoop": -1.5, "vib": (0.55, 5.6, 38), "fall": -3.0})]),
    # Coda: the choir exhales one last doubled "aaah" over the fading
    # voicings, and the piece subsides back into the boundless source
    (12, "vocoder", "Aaah.",
     [("Aaah", 0.5, 10.0, 0.9, "E4", {})]),
]


def parse(entry):
    """-> (slot_beats, text, timing, is_vocoder); rest entries handled
    by callers before this."""
    if len(entry) == 4:
        return entry[0], entry[2], entry[3], True
    return entry[0], entry[1], entry[2], False


def synthesize(pipe, voice, text, speed):
    for r in pipe(text, voice=voice, speed=speed):
        audio = np.array(r.audio, dtype=np.float32).flatten()
        words = [(t.text, int(t.start_ts * RATE), int(t.end_ts * RATE))
                 for t in r.tokens
                 if t.phonemes and any(c.isalnum() for c in t.text)
                 and t.start_ts is not None]
        return audio, words
    raise RuntimeError(f"no audio for {text!r}")


def word_knots(audio, s0, s1, o0, o1):
    step = int(0.010 * RATE)
    n = max(1, (s1 - s0) // step)
    bounds = np.linspace(s0, s1, n + 1).astype(int)
    w = np.array([np.sqrt((audio[a:b] ** 2).mean() + 1e-9)
                  for a, b in zip(bounds[:-1], bounds[1:])])
    alloc = np.concatenate([[0.0], np.cumsum(w / w.sum())]) * (o1 - o0) + o0
    return list(zip(alloc.astype(int), bounds))


def wsola(audio, knots, out_len, win=600, hop=150, search=200):
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


def build_pitch(spb, total_samples):
    rng = np.random.default_rng(11)
    bp = []
    prev = None
    base = 0.0
    voc_spans = []
    for entry in SCORE:
        if entry[0] == "rest":
            base += entry[1] * spb
            continue
        slot_beats, _text, timing, is_voc = parse(entry)
        if is_voc:
            voc_spans.append((base, base + slot_beats * spb))
        for word, onset, length, _vel, pitch, opts in timing:
            t0, t1 = base + onset * spb, base + (onset + length) * spb
            way = [(0.0, N[pitch])] if isinstance(pitch, str) else \
                  [(f, N[n]) for f, n in pitch]
            if prev is None:
                prev = way[0][1]
            glide = opts.get("glide", 0.06 if way[0][1] != prev else 0.03)
            bp.append((t0, prev))
            bp.append((t0 + glide, way[0][1] + opts.get("scoop", 0.0)))
            if "scoop" in opts:
                bp.append((t0 + glide + 0.10, way[0][1]))
            for f, m in way[1:]:
                tw = t0 + f * (t1 - t0)
                bp.append((tw - 0.02, bp[-1][1]))
                bp.append((tw + 0.02, m))
            if "fall" in opts:
                bp.append((t1 - 0.10, way[-1][1]))
                bp.append((t1, way[-1][1] + opts["fall"]))
            else:
                bp.append((t1, way[-1][1]))
            prev = way[-1][1]
        base += slot_beats * spb
    times = np.array([t for t, _ in bp]) * RATE
    vals = np.array([m for _, m in bp], dtype=np.float64)
    midi = np.interp(np.arange(total_samples), times, vals)

    k = int(0.015 * RATE)
    kernel = np.ones(k) / k
    for _ in range(2):
        midi = np.convolve(np.pad(midi, k, mode="edge"), kernel, "same")[k:-k]

    base = 0.0
    for entry in SCORE:
        if entry[0] == "rest":
            base += entry[1] * spb
            continue
        slot_beats, _text, timing, _is_voc = parse(entry)
        for word, onset, length, _vel, pitch, opts in timing:
            if "vib" not in opts:
                continue
            delay_f, rate_hz, cents = opts["vib"]
            t0 = base + (onset + delay_f * length) * spb
            t1 = base + (onset + length) * spb
            a, b = int(t0 * RATE), min(int(t1 * RATE), total_samples)
            if b <= a:
                continue
            tt = np.arange(b - a) / RATE
            drift = 1.0 + 0.04 * np.sin(2 * np.pi * 0.7 * tt + rng.uniform(0, 6))
            phase = np.cumsum(rate_hz * drift) / RATE
            ramp = np.minimum(1.0, tt / max(1e-6, 0.25 * (t1 - t0)))
            midi[a:b] += (cents / 100.0) * ramp * np.sin(2 * np.pi * phase)
        base += slot_beats * spb

    noise = rng.standard_normal(total_samples // 1200 + 2)
    slow = np.interp(np.arange(total_samples), np.arange(len(noise)) * 1200, noise)
    midi += 0.04 * slow

    # Vocoder spans hand pitch back to the keys: hard zeros AFTER the
    # smoothing so the sentinel edge stays sharp
    for a, b in voc_spans:
        midi[int(a * RATE):int(b * RATE)] = 0.0
    return midi.astype(np.float32)


def main():
    from mlx_audio.tts.utils import load_model
    from mlx_audio.tts.models.kokoro import KokoroPipeline

    os.environ.setdefault("VIRTUAL_ENV", os.path.join(REPO, ".venv-voice"))
    model = load_model("mlx-community/Kokoro-82M-bf16")
    pipe = KokoroPipeline(lang_code="a", model=model,
                          repo_id="mlx-community/Kokoro-82M-bf16")

    spb = 60.0 / BPM
    ramp = int(0.030 * RATE)
    kernel = np.hanning(2 * ramp + 1)
    kernel /= kernel.sum()
    chunks = []
    for entry in SCORE:
        if entry[0] == "rest":
            chunks.append(np.zeros(int(entry[1] * spb * RATE), dtype=np.float32))
            continue
        slot_beats, text, timing, _is_voc = parse(entry)
        slot = int(round(slot_beats * spb * RATE))
        t_end = timing[-1][1] + timing[-1][2]
        assert t_end <= slot_beats, (text[:30], t_end, slot_beats)
        out_len = int(slot * LIFT)
        layers = []
        for voice, gain in (CHOIR if _is_voc else LEAD):
            audio, words = synthesize(pipe, voice, text, 1.0)
            target = 0.8 * t_end * spb * LIFT
            # Floor at 0.8: Kokoro slurs below that, and the warp does
            # the stretching anyway — into vowels, not consonants
            speed = max(0.8, min(1.2, len(audio) / RATE / target))
            if abs(speed - 1.0) > 0.05:
                audio, words = synthesize(pipe, voice, text, speed)
            assert len(words) == len(timing), (
                f"{voice} {text!r}: {len(words)} words vs {len(timing)}\n"
                f"  heard: {[w for w, _, _ in words]}")
            knots = [(0, max(0, words[0][1]
                             - int(timing[0][1] * spb * RATE * LIFT)))]
            for (word, s0, s1), (name, onset, length, _v, _p, _o) in \
                    zip(words, timing):
                assert word.lower().strip(".,!?") == \
                    name.lower().strip(".,!?"), (word, name)
                o0 = int(onset * spb * RATE * LIFT)
                o1 = int((onset + length) * spb * RATE * LIFT)
                knots += word_knots(audio, s0, s1, o0, o1)
            knots.append((out_len, min(len(audio),
                                       knots[-1][1] + out_len - knots[-1][0])))
            layer = varispeed(wsola(audio, knots, out_len), LIFT)
            layer = np.pad(layer[:slot], (0, max(0, slot - len(layer))))
            layers.append(layer * gain)
            print(f"  {voice:9s} speed {speed:.2f}  {text[:52]}")
        phrase = np.sum(layers, axis=0)
        # Even the words out: the Talker's peak follower buries anything
        # far below the phrase's loudest word, so bring each scored word
        # toward a common level BEFORE the authored dynamics go on
        even = np.ones(slot, dtype=np.float32)
        rms_all = [np.sqrt((phrase[int(o * spb * RATE):
                                   int((o + l) * spb * RATE)] ** 2).mean() + 1e-9)
                   for _n, o, l, _v, _p, _x in timing]
        target_rms = float(np.median(rms_all))
        for (name, onset, length, _v, _p, _x), wr in zip(timing, rms_all):
            a, b = int(onset * spb * RATE), min(int((onset + length) * spb * RATE), slot)
            even[a:b] = np.clip((target_rms / wr) ** 0.7, 0.5, 2.5)
        env = np.full(slot, 0.75, dtype=np.float32)
        for name, onset, length, vel, _p, _x in timing:
            a, b = int(onset * spb * RATE), int((onset + length) * spb * RATE)
            env[a:min(b, slot)] = vel
        shaped = np.convolve(even * env, kernel, "same")
        phrase *= shaped
        # The 20-band vocoder runs far quieter than the Talker for the
        # same modulator (measured on stems: -15 dB) — feed it hot and
        # soft-limited; its band followers hear RMS, and a compressed
        # modulator is exactly what a vocoder choir wants
        if _is_voc:
            phrase *= 0.62 / (np.sqrt((phrase ** 2).mean()) + 1e-9)
            phrase = np.tanh(phrase / 0.65) * 0.65
        else:
            phrase *= 0.12 / (np.sqrt((phrase ** 2).mean()) + 1e-9)
        chunks.append(phrase.astype(np.float32))

    line = np.concatenate(chunks)
    line *= 0.9 / np.abs(line).max()
    sf.write(os.path.join(REPO, "renders", "nevernot-line.wav"), line, RATE)
    pitch = build_pitch(spb, len(line))
    sf.write(os.path.join(REPO, "renders", "nevernot-pitch.wav"),
             pitch, RATE, subtype="FLOAT")

    # The score as data, for anything downstream (the lyric video reads
    # this): absolute seconds on the VOCAL clock (add the song's intro
    # offset to get song time)
    import json
    dump, base = [], 0.0
    for entry in SCORE:
        if entry[0] == "rest":
            base += entry[1] * spb
            continue
        slot_beats, text, timing, is_voc = parse(entry)
        dump.append({
            "start": base, "dur": slot_beats * spb, "vocoder": is_voc,
            "text": text,
            "words": [{"w": w, "t": base + o * spb, "dur": l * spb, "vel": v}
                      for w, o, l, v, _p, _x in timing]})
        base += slot_beats * spb
    with open(os.path.join(REPO, "renders", "nevernot-score.json"), "w") as f:
        json.dump({"spb": spb, "intro": 16 * spb, "phrases": dump}, f, indent=1)
    print(f"wrote line ({len(line) / RATE:.1f}s) + pitch "
          f"({pitch.min():.1f}..{pitch.max():.1f} MIDI) + score json")


if __name__ == "__main__":
    main()
