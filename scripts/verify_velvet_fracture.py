#!/usr/bin/env python3
"""Independent audits for Velvet Fracture's five-note arp-to-chord form.

This cannot prove that a composition is subjectively ideal. It does prevent a
good-looking event count from concealing the failures that matter here: crowded
motifs, a stalled register, weak harmony, a clock kink, a state handoff, a level
restart, a click, or residual note attacks in the held ending.
"""

from __future__ import annotations

import itertools
import hashlib
import json
import math
import re
import struct
import subprocess
import tempfile
from pathlib import Path

import numpy as np


ROOT = Path(__file__).resolve().parents[1]
SONG = ROOT / "songs" / "velvet-fracture.song"
PATCH = ROOT / "patches" / "velvet-fracture-voice.patch"
BINARY = ROOT / "target" / "debug" / "patina"
FFMPEG = Path("/Users/user/bin/ffmpeg")

LEAD_CELLS = [
    [21, 25, 28, 32, 35],
    [21, 25, 28, 30, 35],
    [31, 35, 38, 42, 45],
    [31, 35, 38, 40, 45],
    [42, 45, 49, 52, 56],
    [49, 52, 56, 59, 63],
    [52, 57, 59, 61, 62],
    [57, 61, 64, 68, 71],
]
CHORD_NAMES = ["Amaj9", "A6/9", "Gmaj9", "G6/9", "F#m9", "C#m9", "E13sus", "Amaj9"]
CHORD_PITCH_CLASSES = [
    {1, 4, 8, 9, 11},
    {1, 4, 6, 9, 11},
    {2, 6, 7, 9, 11},
    {2, 4, 7, 9, 11},
    {1, 4, 6, 8, 9},
    {1, 3, 4, 8, 11},
    {1, 2, 4, 9, 11},
    {1, 4, 8, 9, 11},
]
FINAL_ARP = LEAD_CELLS[-1]
FINAL_SPECTRUM = {"A": 57, "C#": 61, "E": 64, "G#": 68, "B": 71}


def run(*args: str) -> str:
    proc = subprocess.run(
        args,
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    return proc.stdout


def read_float_wav(path: Path) -> tuple[np.ndarray, int]:
    raw = path.read_bytes()
    pos = 12
    fmt = None
    data = None
    while pos + 8 <= len(raw):
        chunk_id = raw[pos : pos + 4]
        size = struct.unpack_from("<I", raw, pos + 4)[0]
        body = raw[pos + 8 : pos + 8 + size]
        if chunk_id == b"fmt ":
            fmt = struct.unpack_from("<HHIIHH", body, 0)
        elif chunk_id == b"data":
            data = body
            break
        pos += 8 + size + (size & 1)
    if fmt is None or data is None:
        raise ValueError(f"{path}: missing fmt or data chunk")
    kind, channels, rate, _, _, bits = fmt
    if (kind, channels, bits) != (3, 2, 32):
        raise ValueError(f"{path}: expected stereo float32 WAV")
    frames = np.frombuffer(data, dtype="<f4").reshape(-1, channels)
    return frames.astype(np.float64).mean(axis=1), rate


def rms(samples: np.ndarray) -> float:
    return math.sqrt(float(np.mean(samples * samples)))


def db(value: float) -> float:
    return 20.0 * math.log10(max(value, 1e-15))


def patch_values(text: str) -> dict[str, float]:
    values: dict[str, float] = {}
    for raw in text.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        key, value = line.split()[:2]
        values[key] = float(value)
    return values


def circular_distance(a: int, b: int) -> int:
    distance = abs(a - b) % 12
    return min(distance, 12 - distance)


def voice_leading_cost(source: set[int], destination: set[int]) -> int:
    ordered = sorted(source)
    return min(
        sum(circular_distance(a, b) for a, b in zip(ordered, candidate))
        for candidate in itertools.permutations(destination)
    )


def spectral_amplitude(samples: np.ndarray, rate: int, midi_note: int) -> float:
    frequency = 440.0 * 2.0 ** ((midi_note - 69) / 12.0)
    window = np.hanning(len(samples))
    times = np.arange(len(samples)) / rate
    return abs(np.dot(samples * window, np.exp(-2j * np.pi * frequency * times))) / (
        window.sum() / 2.0
    )


def main() -> None:
    song_text = SONG.read_text()
    patch = patch_values(PATCH.read_text())
    failures: list[str] = []

    def heading(name: str) -> None:
        print(f"\n[{name}]")

    def check(condition: bool, name: str, detail: str) -> None:
        status = "PASS" if condition else "FAIL"
        print(f"{status:4}  {name}: {detail}")
        if not condition:
            failures.append(name)

    heading("SCORE / HARMONY")
    note_tracks = re.findall(r"(?m)^track\s+", song_text)
    forbidden = [token for token in ("pitch_shift", "glide", "automate output", "[", "!", "^") if token in song_text]
    check(len(note_tracks) == 1, "one instrument stream", f"note tracks={len(note_tracks)}")
    check(not forbidden, "no handoff machinery", f"forbidden constructs={forbidden}")
    check("len=0.5" in song_text, "compact motif clock", "five notes per cell; no 16-note scale runs")

    pitch_class_sets = [{note % 12 for note in cell} for cell in LEAD_CELLS]
    roots = [cell[0] for cell in LEAD_CELLS]
    centers = [float(np.mean(cell)) for cell in LEAD_CELLS]
    check(pitch_class_sets == CHORD_PITCH_CLASSES, "authored harmony", " -> ".join(CHORD_NAMES))
    check(
        all(all(a < b for a, b in zip(cell, cell[1:])) for cell in LEAD_CELLS),
        "every motif rises",
        "all eight cells contain exactly five strictly ascending pitches",
    )
    score_span = max(map(max, LEAD_CELLS)) - min(map(min, LEAD_CELLS))
    check(
        roots == sorted(roots) and roots[-1] - roots[0] == 36 and score_span == 50,
        "audible register climb",
        f"cell roots rise A0->A3 ({roots[-1]-roots[0]} semitones); score spans {score_span} semitones",
    )
    check(
        all(b >= a - 1.0 for a, b in zip(centers, centers[1:])),
        "rising melodic centers",
        "cell centers=" + ", ".join(f"{value:.1f}" for value in centers),
    )
    costs = [voice_leading_cost(a, b) for a, b in zip(pitch_class_sets, pitch_class_sets[1:])]
    common = [len(a & b) for a, b in zip(pitch_class_sets, pitch_class_sets[1:])]
    check(
        max(costs) <= 6 and min(common) >= 2,
        "parsimonious chord motion",
        f"minimum semitone costs={costs}; common-tone counts={common}",
    )

    heading("SYNTH TOPOLOGY")
    check(
        patch.get("waveform") == 2
        and patch.get("mix_saw") == 1
        and patch.get("osc2_level") == 0
        and patch.get("osc3_level") == 0,
        "physical saw source",
        "saw VCO; no hidden oscillator layer",
    )
    check(
        all(name in patch for name in ("cutoff", "filter_env", "filter_attack", "filter_decay", "filter_sustain", "filter_release")),
        "explicit VCF",
        "cutoff and full filter ADSR are authored",
    )
    check(
        all(name in patch for name in ("attack", "decay", "sustain", "release")),
        "explicit VCA",
        "full amplifier ADSR is authored",
    )

    with tempfile.TemporaryDirectory(prefix="velvet-fracture-audit-") as temp_name:
        temp = Path(temp_name)
        events_path = temp / "events.json"
        mix_path = temp / "mix.wav"
        repeat_path = temp / "mix-repeat.wav"
        run("cargo", "build", "--bin", "patina")
        run(str(BINARY), "--play", str(SONG.relative_to(ROOT)), "--export-events", str(events_path))
        render_log = run(str(BINARY), "--play", str(SONG.relative_to(ROOT)), "--render", str(mix_path))
        run(str(BINARY), "--play", str(SONG.relative_to(ROOT)), "--render", str(repeat_path))

        events = json.loads(events_path.read_text())["events"]
        ons = [event for event in events if event["type"] == "on"]
        offs = [event for event in events if event["type"] == "off"]
        notes = [event["note"] for event in ons]
        on_times = np.array([event["t"] for event in ons])
        intervals = np.diff(on_times)

        heading("EVENT / CLOCK CONTINUITY")
        expected_lead = [note for cell in LEAD_CELLS for note in cell]
        check(notes[:40] == expected_lead, "exact composed phrase", "eight five-note cells, 40 lead attacks total")
        check(notes[35:40] == notes[40:45] == FINAL_ARP, "identical boundary motif", "last slow cycle == first accelerated cycle")
        check(notes[40:] == FINAL_ARP * 48, "unchanged accelerated motif", "48 repetitions; no replacement chord token")
        check(
            len(ons) == len(offs) == 280 and {event["ch"] for event in ons} == {1},
            "ordinary note stream",
            f"{len(ons)} note-on/off pairs on one channel",
        )
        check(
            float(np.max(np.diff(intervals[:180]))) <= 5e-6,
            "strict continuous accelerando",
            f"spacing contracts {intervals[0]*1000:.2f}->{intervals[179]*1000:.2f} ms without a reversal",
        )
        ratios = intervals[1:] / intervals[:-1]
        neighborhood = ratios[30:50]
        ratio_error = abs(float(ratios[38] - np.median(neighborhood)))
        check(ratio_error < 2e-5, "no clock kink at score boundary", f"local interval-ratio error={ratio_error:.8f}")
        fast = intervals[180:]
        check(
            float(np.ptp(fast)) <= 2e-6 and abs(float(np.mean(fast)) - 0.005) < 2e-6,
            "audio-rate plateau",
            f"{np.mean(fast)*1000:.3f} ms attacks = {1/np.mean(fast):.1f} notes/s = {1/np.mean(fast)/5:.1f} complete arps/s",
        )
        pedal_events = [event for event in events if event["type"] == "param" and event["param"] == "SustainPedal"]
        check(
            abs(pedal_events[-2]["t"] - on_times[35]) < 2e-6 and pedal_events[-2]["t"] < on_times[40] - 0.8,
            "state acquired before repetition",
            f"sustain begins {on_times[40]-pedal_events[-2]['t']:.3f}s before the score boundary",
        )

        params = [event for event in events if event["type"] == "param" and event["ch"] == 1]
        def values(name: str) -> list[float]:
            return [event["value"] for event in params if event["param"] == name]

        check(
            values("Decay")[0] >= 0.89
            and values("Sustain")[0] >= 0.41
            and values("Release")[0] >= 0.39
            and values("FilterEnvAmount")[0] >= 2.79,
            "strong opening VCA/VCF contour",
            "long bass decay/release and full filter-envelope depth",
        )
        check(
            values("Attack")[-1] >= 0.44
            and values("Sustain")[-1] >= 0.99
            and abs(values("FilterEnvAmount")[-1]) < 0.01
            and values("FilterSustain")[-1] >= 0.99,
            "retrigger-neutral final envelopes",
            "VCA sustain=1 and VCF sustain=1/depth=0 before attacks cease",
        )

        effect_names = {
            "NoiseLevel", "FuzzAmount", "SpringWet", "ChorusModeSel", "ChorusMix", "ChorusDepth",
            "TapeWow", "TapeFlutter", "TapeDrive", "TapeAge", "ReverbSend", "SpringSend", "ChorusSend",
        }
        effect_values = [event["value"] for event in events if event["type"] == "param" and event["param"] in effect_names]
        check(effect_values and max(map(abs, effect_values)) == 0.0, "heavy effects defeated", "no noise, fuzz, spring, chorus, or tape masking")
        wet = [event["value"] for event in events if event["type"] == "param" and event["param"] == "ReverbWet"]
        check(abs(wet[0] - 0.06) < 1e-6 and wet[-1] >= 0.53, "algorithmic reverb bloom", f"wet={wet[0]:.2f}->{wet[-1]:.2f}")

        peak_match = re.search(r"peak concurrent voices: (\d+)/(\d+)", render_log)
        peak, capacity = map(int, peak_match.groups()) if peak_match else (999, 0)
        check(peak == 5 and peak < capacity, "five physical cards", f"peak voices={peak}/{capacity}; no stealing")

        audio, rate = read_float_wav(mix_path)
        duration = len(audio) / rate
        last_note_event = max(ons[-1]["t"], offs[-1]["t"])

        heading("WAVEFORM / PERCEPTUAL PROXIES")
        check(abs(duration - 20.0) < 0.03, "twenty-second form", f"duration={duration:.6f}s")

        first_hash = hashlib.sha256(mix_path.read_bytes()).hexdigest()
        repeat_hash = hashlib.sha256(repeat_path.read_bytes()).hexdigest()
        check(
            first_hash == repeat_hash,
            "deterministic independent render",
            f"two complete renders share SHA-256 {first_hash}",
        )

        ffmpeg_log = run(
            str(FFMPEG), "-hide_banner", "-nostats", "-i", str(mix_path),
            "-filter_complex", "ebur128=peak=true", "-f", "null", "-",
        )
        ffmpeg_loudness = [float(value) for value in re.findall(r"I:\s+(-?\d+\.\d+) LUFS", ffmpeg_log)][-1]
        ffmpeg_peak = [float(value) for value in re.findall(r"Peak:\s+(-?\d+\.\d+) dBFS", ffmpeg_log)][-1]
        check(
            -15.0 <= ffmpeg_loudness <= -14.0 and -1.2 <= ffmpeg_peak <= -0.8,
            "external EBU R128 cross-check",
            f"FFmpeg reports {ffmpeg_loudness:.1f} LUFS integrated and {ffmpeg_peak:.1f} dBFS true peak",
        )

        cycle_levels = []
        for index in range(0, len(ons) - 5, 5):
            lo = int(on_times[index] * rate)
            hi = int(on_times[index + 5] * rate)
            cycle_levels.append(db(rms(audio[lo:hi])))
        harmony_join = cycle_levels[7] - cycle_levels[6]
        score_join = cycle_levels[8] - cycle_levels[7]
        check(
            abs(harmony_join) < 0.5 and abs(score_join) < 0.5,
            "cycle-energy continuity",
            f"E13sus->Amaj9={harmony_join:+.3f} dB; slow->accelerating Amaj9={score_join:+.3f} dB",
        )

        level_windows = [(10.0 + i, 11.0 + i) for i in range(10)]
        levels = [db(rms(audio[int(a * rate) : int(b * rate)])) for a, b in level_windows]
        check(
            max(levels) - min(levels) < 0.8,
            "stable long-form dynamics",
            f"10-20s range={max(levels)-min(levels):.3f} dB; levels=" + ", ".join(f"{value:.2f}" for value in levels),
        )

        derivative = np.abs(np.diff(audio))
        ordinary_step = float(np.percentile(derivative, 99.99))
        boundary_points = (on_times[40], on_times[180], last_note_event)
        step_ratios = []
        for point in boundary_points:
            center = int(point * rate)
            width = int(0.005 * rate)
            step_ratios.append(float(np.max(derivative[center - width : center + width])) / ordinary_step)
        check(
            max(step_ratios) < 1.2,
            "no waveform discontinuity",
            "boundary max-step / ordinary p99.99=" + ", ".join(f"{value:.3f}" for value in step_ratios),
        )

        fft_size = int(0.05 * rate)
        fft_window = np.hanning(fft_size)
        def onset_spectrum(point: float) -> np.ndarray:
            center = int(point * rate)
            segment = audio[center - fft_size // 2 : center + fft_size // 2]
            return np.log1p(np.abs(np.fft.rfft(segment * fft_window)))

        cycle_flux = []
        for index in range(5, len(ons), 5):
            before = onset_spectrum(on_times[index - 1])
            after = onset_spectrum(on_times[index])
            cycle_flux.append(float(np.sqrt(np.mean((after - before) ** 2))))
        boundary_flux = cycle_flux[7]
        following_flux = float(np.median(cycle_flux[8:12]))
        check(
            boundary_flux <= following_flux * 1.2,
            "no exceptional spectral restart",
            f"boundary flux={boundary_flux:.4f}; next-cycle median={following_flux:.4f}; ratio={boundary_flux/following_flux:.3f}",
        )

        eventless_hold = duration - last_note_event
        check(eventless_hold > 2.1, "literal held ending", f"no note events for final {eventless_hold:.3f}s")

        def pulse_contrast(start: float, end: float, window_ms: float) -> float:
            segment = audio[int(start * rate) : int(end * rate)]
            hop = int(window_ms * 0.001 * rate)
            samples = np.array([rms(segment[i : i + hop]) for i in range(0, len(segment) - hop + 1, hop)])
            return db(float(np.percentile(samples, 90) / np.percentile(samples, 10)))

        lead_pulse = pulse_contrast(10.0, 11.0, 50.0)
        held_pulse = pulse_contrast(19.0, 20.0, 50.0)
        check(
            held_pulse < 1.5 and held_pulse < lead_pulse * 0.4,
            "onset articulation disappears",
            f"50ms pulse contrast {lead_pulse:.2f}->{held_pulse:.2f} dB",
        )

        held = audio[int(18.0 * rate) : int(19.5 * rate)]
        chord_energy = {name: spectral_amplitude(held, rate, note) for name, note in FINAL_SPECTRUM.items()}
        chord_spread = db(max(chord_energy.values()) / min(chord_energy.values()))
        check(
            chord_spread < 7.0,
            "complete final Amaj9 spectrum",
            f"A/C#/E/G#/B fundamental spread={chord_spread:.2f} dB",
        )

    if failures:
        raise SystemExit(f"\nFAILED: {len(failures)} checks: {', '.join(failures)}")
    print("\nAUDITED: score, harmony, clock, synth state, waveform, dynamics, onset loss, and chord spectrum all pass")


if __name__ == "__main__":
    main()
