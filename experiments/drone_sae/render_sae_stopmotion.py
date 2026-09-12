#!/usr/bin/env python3

import argparse
import json
import os
import subprocess
from dataclasses import dataclass
from pathlib import Path

import mido
import librosa
import numpy as np
import torch
from diffusers import AutoPipelineForImage2Image
from PIL import Image, ImageEnhance

from run_feature_sweeps import (
    BLOCKS,
    MODEL_ID,
    PROMPT,
    apply_feature,
    generator,
    load_sae,
    transplant_pixel_delta,
)


TIMES = ("1000", "1200", "1500")
STRENGTH_COUNT = 25
MIN_STRENGTH_AMPLITUDE = 160.0
MAX_STRENGTH_AMPLITUDE = 240.0
MODEL_SIZE = 1024
FRAME_WIDTH = 1024
FRAME_HEIGHT = 576
OUTPUT_WIDTH = 1920
OUTPUT_HEIGHT = 1080
OUTPUT_FPS = 48
RENDER_FPS = 24
TIME_IMAGE_FPS = 12.0
START_CROP_WIDTH = 768.0
END_CROP_WIDTH = 1024.0
MIN_CHORD_SPACING_SECONDS = 8.5
AUDIO_ANALYSIS_RATE = 22050
AUDIO_HOP = 512
AUDIO_WINDOW = 2048
PITCH_CLASS_NAMES = (
    "C",
    "C#",
    "D",
    "D#",
    "E",
    "F",
    "F#",
    "G",
    "G#",
    "A",
    "A#",
    "B",
)


@dataclass(frozen=True)
class Feature:
    block: str
    index: int
    effect: float
    source: str


@dataclass(frozen=True)
class MidiOnset:
    time: float
    pitches: tuple[int, ...]
    velocity: int
    tracks: tuple[str, ...]


@dataclass(frozen=True)
class NoteSpan:
    start: float
    end: float
    pitch: int
    track: str


@dataclass(frozen=True)
class PatternWindow:
    pattern: str
    voicing: str
    start: float
    end: float


@dataclass(frozen=True)
class AudioAnalysis:
    duration: float
    times: np.ndarray
    flux: np.ndarray
    rms: np.ndarray
    centroid: np.ndarray
    sub: np.ndarray
    mid: np.ndarray
    high: np.ndarray
    chroma: np.ndarray
    bandwidth: np.ndarray
    rolloff: np.ndarray
    flatness: np.ndarray
    contrast: np.ndarray
    tonnetz: np.ndarray
    tuning: float
    tempo: float
    beat_times: np.ndarray


@dataclass(frozen=True)
class Interval:
    start: float
    end: float
    rms: float
    centroid: float
    bandwidth: float
    rolloff: float
    flatness: float
    contrast: float
    root_pitch_class: int
    midi_chord: str
    wav_chord: str
    midi_pitch_classes: tuple[int, ...]
    wav_pitch_classes: tuple[int, ...]
    tonal_agreement: float
    tonal_motion: float
    strength_start: float
    strength_end: float


@dataclass(frozen=True)
class Section:
    name: str
    motif: str
    variation: int
    start: float
    end: float


@dataclass(frozen=True)
class MotifAssignment:
    section: str
    motif_step: int
    harmonic_pattern: str
    phrase_repetition: int
    feature: Feature


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Render MIDI- and audio-synchronized SAE stop motion from drone "
            "trios with 25 strengths per time-of-day image."
        )
    )
    parser.add_argument("--input-dir", type=Path, required=True)
    parser.add_argument("--checkpoint-root", type=Path, required=True)
    parser.add_argument("--feature-manifest", type=Path, required=True)
    parser.add_argument("--plot-stats", type=Path, required=True)
    parser.add_argument("--midi", type=Path, required=True)
    parser.add_argument("--audio", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--segment-limit",
        type=int,
        help="Render only the first N plot trios without assembling.",
    )
    parser.add_argument("--steps", type=int, default=4)
    parser.add_argument("--image-strength", type=float, default=0.25)
    parser.add_argument("--seed", type=int, default=72)
    return parser.parse_args()


def audio_duration(path: Path) -> float:
    result = subprocess.run(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            str(path),
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    return float(result.stdout.strip())


def analyze_audio(path: Path) -> AudioAnalysis:
    samples, sample_rate = librosa.load(
        path,
        sr=AUDIO_ANALYSIS_RATE,
        mono=True,
    )
    magnitude = np.abs(
        librosa.stft(
            samples,
            n_fft=AUDIO_WINDOW,
            hop_length=AUDIO_HOP,
        )
    )
    power = magnitude * magnitude
    frequencies = librosa.fft_frequencies(
        sr=sample_rate,
        n_fft=AUDIO_WINDOW,
    )
    flux = librosa.onset.onset_strength(
        y=samples,
        sr=sample_rate,
        hop_length=AUDIO_HOP,
    )
    rms = librosa.feature.rms(
        S=magnitude,
        frame_length=AUDIO_WINDOW,
        hop_length=AUDIO_HOP,
    )[0]
    centroid = librosa.feature.spectral_centroid(
        S=magnitude,
        sr=sample_rate,
    )[0]
    bandwidth = librosa.feature.spectral_bandwidth(
        S=magnitude,
        sr=sample_rate,
    )[0]
    rolloff = librosa.feature.spectral_rolloff(
        S=magnitude,
        sr=sample_rate,
        roll_percent=0.85,
    )[0]
    flatness = librosa.feature.spectral_flatness(S=magnitude)[0]
    contrast = librosa.feature.spectral_contrast(
        S=magnitude,
        sr=sample_rate,
    ).mean(axis=0)
    harmonic = librosa.effects.harmonic(samples, margin=4.0)
    chroma = librosa.feature.chroma_cqt(
        y=harmonic,
        sr=sample_rate,
        hop_length=AUDIO_HOP,
    )
    tonnetz = librosa.feature.tonnetz(
        chroma=chroma,
        sr=sample_rate,
    )
    sub_mask = (frequencies >= 30) & (frequencies < 180)
    mid_mask = (frequencies >= 180) & (frequencies < 1000)
    high_mask = (frequencies >= 1000) & (frequencies < 8000)
    sub = power[sub_mask].mean(axis=0)
    mid = power[mid_mask].mean(axis=0)
    high = power[high_mask].mean(axis=0)
    frame_count = min(
        len(flux),
        len(rms),
        len(centroid),
        len(bandwidth),
        len(rolloff),
        len(flatness),
        len(contrast),
        len(sub),
        chroma.shape[1],
        tonnetz.shape[1],
    )
    times = librosa.frames_to_time(
        np.arange(frame_count),
        sr=sample_rate,
        hop_length=AUDIO_HOP,
    )
    tempo_values = librosa.feature.tempo(
        onset_envelope=flux[:frame_count],
        sr=sample_rate,
        hop_length=AUDIO_HOP,
        aggregate=np.median,
    )
    _, beat_frames = librosa.beat.beat_track(
        onset_envelope=flux[:frame_count],
        sr=sample_rate,
        hop_length=AUDIO_HOP,
    )
    return AudioAnalysis(
        duration=audio_duration(path),
        times=times,
        flux=flux[:frame_count],
        rms=rms[:frame_count],
        centroid=centroid[:frame_count],
        sub=sub[:frame_count],
        mid=mid[:frame_count],
        high=high[:frame_count],
        chroma=chroma[:, :frame_count],
        bandwidth=bandwidth[:frame_count],
        rolloff=rolloff[:frame_count],
        flatness=flatness[:frame_count],
        contrast=contrast[:frame_count],
        tonnetz=tonnetz[:, :frame_count],
        tuning=float(librosa.estimate_tuning(y=harmonic, sr=sample_rate)),
        tempo=float(np.asarray(tempo_values).reshape(-1)[0]),
        beat_times=librosa.frames_to_time(
            beat_frames,
            sr=sample_rate,
            hop_length=AUDIO_HOP,
        ),
    )


def midi_tick_converter(mid: mido.MidiFile):
    tempo_events = []
    for track in mid.tracks:
        tick = 0
        for message in track:
            tick += message.time
            if message.type == "set_tempo":
                tempo_events.append((tick, message.tempo))
    tempo_events = sorted(set(tempo_events))

    def tick_to_seconds(target_tick: int) -> float:
        tempo = 500000
        prior_tick = 0
        seconds = 0.0
        for event_tick, new_tempo in tempo_events:
            if target_tick < event_tick:
                break
            seconds += mido.tick2second(
                event_tick - prior_tick,
                mid.ticks_per_beat,
                tempo,
            )
            prior_tick = event_tick
            tempo = new_tempo
        return seconds + mido.tick2second(
            target_tick - prior_tick,
            mid.ticks_per_beat,
            tempo,
        )

    return tick_to_seconds


def parse_midi_onsets(path: Path) -> list[MidiOnset]:
    mid = mido.MidiFile(path)
    tick_to_seconds = midi_tick_converter(mid)
    raw = []
    for track in mid.tracks:
        tick = 0
        track_name = ""
        for message in track:
            tick += message.time
            if message.type == "track_name":
                track_name = message.name
            elif message.type == "note_on" and message.velocity > 0:
                raw.append(
                    (
                        tick_to_seconds(tick),
                        int(message.note),
                        int(message.velocity),
                        track_name,
                    )
                )
    raw.sort()
    groups: list[list[tuple[float, int, int, str]]] = []
    for event in raw:
        if not groups or event[0] - groups[-1][0][0] > 0.025:
            groups.append([event])
        else:
            groups[-1].append(event)
    return [
        MidiOnset(
            time=group[0][0],
            pitches=tuple(sorted({event[1] for event in group})),
            velocity=max(event[2] for event in group),
            tracks=tuple(sorted({event[3] for event in group})),
        )
        for group in groups
    ]


def parse_midi_note_spans(path: Path) -> list[NoteSpan]:
    mid = mido.MidiFile(path)
    tick_to_seconds = midi_tick_converter(mid)
    spans = []
    for track in mid.tracks:
        tick = 0
        track_name = ""
        active = {}
        for message in track:
            tick += message.time
            if message.type == "track_name":
                track_name = message.name
            elif message.type == "note_on" and message.velocity > 0:
                active[(message.channel, message.note)] = tick
            elif (
                message.type == "note_off"
                or (message.type == "note_on" and message.velocity == 0)
            ):
                key = (message.channel, message.note)
                if key not in active:
                    continue
                start_tick = active.pop(key)
                spans.append(
                    NoteSpan(
                        start=tick_to_seconds(start_tick),
                        end=tick_to_seconds(tick),
                        pitch=int(message.note),
                        track=track_name,
                    )
                )
    return spans


def contiguous_span_groups(
    spans: list[NoteSpan],
    maximum_gap: float,
) -> list[list[NoteSpan]]:
    ordered = sorted(spans, key=lambda span: span.start)
    groups: list[list[NoteSpan]] = []
    group_end = 0.0
    for span in ordered:
        if not groups or span.start - group_end > maximum_gap:
            groups.append([span])
            group_end = span.end
        else:
            groups[-1].append(span)
            group_end = max(group_end, span.end)
    return groups


def detect_pattern_windows(
    spans: list[NoteSpan],
    midi_lead: float,
) -> list[PatternWindow]:
    windows = []
    pad_spans = [
        span
        for span in spans
        if span.track in ("Smooth Synth", "Stone In Focus corrected")
    ]
    for group in contiguous_span_groups(pad_spans, maximum_gap=4.0):
        start = min(span.start for span in group) - midi_lead
        end = max(span.end for span in group) - midi_lead
        if end - start >= 5.0:
            windows.append(
                PatternWindow(
                    pattern="three-chord",
                    voicing="synth-stone",
                    start=max(start, 0.0),
                    end=end,
                )
            )

    ocarina_one = [span for span in spans if span.track == "Ocarina1"]
    for group in contiguous_span_groups(ocarina_one, maximum_gap=4.0):
        start = min(span.start for span in group) - midi_lead
        end = max(span.end for span in group) - midi_lead
        if end - start >= 5.0:
            windows.append(
                PatternWindow(
                    pattern="five-chord",
                    voicing="ocarina1",
                    start=start,
                    end=end,
                )
            )

    bass_spans = [span for span in spans if span.track == "SineBass"]
    for group in contiguous_span_groups(bass_spans, maximum_gap=5.0):
        pitch_classes = {span.pitch % 12 for span in group}
        if not {6, 11, 9}.issubset(pitch_classes):
            continue
        windows.append(
            PatternWindow(
                pattern="three-chord",
                voicing="late-bass",
                start=min(span.start for span in group) - midi_lead,
                end=max(span.end for span in group) - midi_lead,
            )
        )
    return sorted(windows, key=lambda window: window.start)


def interval_pattern(
    start: float,
    end: float,
    windows: list[PatternWindow],
) -> str:
    overlaps = []
    for window in windows:
        overlap = max(0.0, min(end, window.end) - max(start, window.start))
        overlaps.append((overlap, window.pattern))
    best_overlap, best_pattern = max(overlaps, default=(0.0, "transition"))
    if best_overlap / max(end - start, 1e-9) < 0.5:
        return "transition-or-ambient"
    return best_pattern


def normalized(values: np.ndarray) -> np.ndarray:
    return (values - np.median(values)) / (np.std(values) + 1e-9)


def estimate_midi_lead(
    onsets: list[MidiOnset],
    analysis: AudioAnalysis,
) -> tuple[float, float]:
    standardized_flux = normalized(analysis.flux)
    first_note = onsets[0].time
    candidates = np.arange(first_note - 2.0, first_note + 2.0001, 0.005)
    frame_step = float(np.median(np.diff(analysis.times)))
    radius = max(1, int(round(0.05 / frame_step)))

    def score(offset: float) -> float:
        values = []
        weights = []
        for onset in onsets:
            audio_time = onset.time - offset
            if not 0.2 < audio_time < analysis.duration - 0.2:
                continue
            center = int(np.searchsorted(analysis.times, audio_time))
            if not radius <= center < len(standardized_flux) - radius:
                continue
            values.append(
                float(
                    standardized_flux[
                        center - radius : center + radius + 1
                    ].max()
                )
            )
            weights.append(
                np.sqrt(len(onset.pitches)) * onset.velocity / 127.0
            )
        return float(np.average(values, weights=weights))

    scores = np.asarray([score(candidate) for candidate in candidates])
    best_index = int(scores.argmax())
    correlation_lead = float(candidates[best_index])
    rms_floor, rms_peak = np.quantile(analysis.rms, [0.05, 0.95])
    audible = np.flatnonzero(
        analysis.rms > rms_floor + 0.08 * (rms_peak - rms_floor)
    )
    if len(audible):
        onset_anchor_lead = first_note - float(analysis.times[audible[0]])
        if abs(onset_anchor_lead - correlation_lead) <= 0.15:
            correlation_lead = onset_anchor_lead
    return correlation_lead, float(scores[best_index])


def strongest_novelty_time(
    analysis: AudioAnalysis,
    start: float,
    end: float,
) -> float:
    log_rms = np.log(analysis.rms + 1e-6)
    novelty = (
        np.maximum(normalized(analysis.flux), 0)
        + 0.45 * np.abs(np.gradient(normalized(log_rms)))
        + 0.25 * np.abs(np.gradient(normalized(analysis.centroid)))
        + 0.20 * np.abs(np.gradient(normalized(analysis.flatness)))
        + 0.20
        * np.linalg.norm(
            np.gradient(analysis.tonnetz, axis=1),
            axis=0,
        )
    )
    mask = (analysis.times >= start) & (analysis.times <= end)
    indices = np.flatnonzero(mask)
    if len(indices) == 0:
        raise RuntimeError(f"No audio analysis frames between {start} and {end}")
    return float(analysis.times[indices[int(novelty[indices].argmax())]])


def chord_boundaries(
    onsets: list[MidiOnset],
    midi_lead: float,
    analysis: AudioAnalysis,
) -> list[float]:
    shifted = [
        MidiOnset(
            time=onset.time - midi_lead,
            pitches=onset.pitches,
            velocity=onset.velocity,
            tracks=onset.tracks,
        )
        for onset in onsets
    ]
    chord_candidates = [
        onset
        for onset in shifted
        if 0 <= onset.time < analysis.duration and len(onset.pitches) >= 2
    ]
    anchors = [0.0]
    for onset in chord_candidates:
        if onset.time - anchors[-1] >= MIN_CHORD_SPACING_SECONDS:
            anchors.append(onset.time)

    enriched = list(anchors)
    for left, right in zip(anchors, anchors[1:]):
        gap = right - left
        if gap <= 30.0:
            continue
        first = strongest_novelty_time(
            analysis,
            left + 0.15 * gap,
            left + 0.48 * gap,
        )
        second = strongest_novelty_time(
            analysis,
            left + 0.52 * gap,
            left + 0.85 * gap,
        )
        enriched.extend((first, second))
    enriched = sorted(set(enriched))
    boundaries = [*enriched, analysis.duration]

    while (len(boundaries) - 1) % len(TIMES) != 0:
        gaps = np.diff(boundaries)
        index = int(gaps.argmax())
        left, right = boundaries[index], boundaries[index + 1]
        split = strongest_novelty_time(
            analysis,
            left + 0.25 * (right - left),
            left + 0.75 * (right - left),
        )
        boundaries.insert(index + 1, split)
    return boundaries


def musical_sections(
    onsets: list[MidiOnset],
    midi_lead: float,
    duration: float,
) -> list[Section]:
    shifted = [
        MidiOnset(
            time=onset.time - midi_lead,
            pitches=onset.pitches,
            velocity=onset.velocity,
            tracks=onset.tracks,
        )
        for onset in onsets
    ]

    def first_time(predicate, after: float) -> float:
        matches = [
            onset.time
            for onset in shifted
            if onset.time >= after and predicate(onset)
        ]
        if not matches:
            raise RuntimeError(f"No MIDI section event found after {after}s")
        return matches[0]

    ocarina_bass = first_time(
        lambda onset: (
            "Ocarina1" in onset.tracks and "SineBass" in onset.tracks
        ),
        60.0,
    )
    stone_interlock = first_time(
        lambda onset: "Stone In Focus corrected" in onset.tracks,
        120.0,
    )
    ocarina_transition = first_time(
        lambda onset: (
            "Ocarina2" in onset.tracks and "SineBass" in onset.tracks
        ),
        240.0,
    )
    final_stone = first_time(
        lambda onset: "Stone In Focus corrected" in onset.tracks,
        480.0,
    )
    prior_to_ambient = [
        onset.time
        for onset in shifted
        if onset.time < final_stone and onset.time >= ocarina_transition
    ]
    if not prior_to_ambient:
        raise RuntimeError("Could not locate the pre-ambient MIDI section")
    ambient = max(prior_to_ambient)

    points = [
        0.0,
        ocarina_bass,
        stone_interlock,
        ocarina_transition,
        ambient,
        duration,
    ]
    identities = [
        ("A1-synth", "A", 1),
        ("B1-ocarina", "B", 1),
        ("A2-stone-interlock", "A", 2),
        ("B2-ocarina-pattern", "B", 2),
        ("A3-ambient-return", "A", 3),
    ]
    return [
        Section(
            name=name,
            motif=motif,
            variation=variation,
            start=start,
            end=end,
        )
        for (name, motif, variation), start, end in zip(
            identities,
            points,
            points[1:],
        )
    ]


def interval_audio_value(
    values: np.ndarray,
    times: np.ndarray,
    start: float,
    end: float,
) -> float:
    mask = (times >= start) & (times < end)
    if not mask.any():
        raise RuntimeError(f"No audio frames in interval {start} to {end}")
    return float(values[mask].mean())


def root_at_time(
    onsets: list[MidiOnset],
    midi_lead: float,
    time: float,
) -> int:
    prior = [
        onset
        for onset in onsets
        if onset.time - midi_lead <= time + 0.05 and onset.pitches
    ]
    if not prior:
        return 0
    return min(prior[-1].pitches) % 12


def chord_name(chroma: np.ndarray) -> str:
    vector = np.asarray(chroma, dtype=np.float64)
    if not vector.any():
        return "none"
    vector = vector / (np.linalg.norm(vector) + 1e-9)
    candidates = []
    for root in range(12):
        for quality, intervals in (
            ("maj", (0, 4, 7)),
            ("min", (0, 3, 7)),
        ):
            template = np.zeros(12, dtype=np.float64)
            template[(root + np.asarray(intervals)) % 12] = (1.0, 0.82, 0.78)
            template[root] += 0.15
            template /= np.linalg.norm(template)
            candidates.append((float(vector @ template), root, quality))
    _score, root, quality = max(candidates)
    return f"{PITCH_CLASS_NAMES[root]}{quality}"


def interval_wav_chroma(
    analysis: AudioAnalysis,
    start: float,
    end: float,
) -> np.ndarray:
    mask = (analysis.times >= start) & (analysis.times < end)
    if not mask.any():
        raise RuntimeError(f"No WAV chroma frames in {start} to {end}")
    weights = 0.35 + np.maximum(normalized(analysis.flux)[mask], 0.0)
    return np.average(analysis.chroma[:, mask], axis=1, weights=weights)


def interval_midi_chroma(
    spans: list[NoteSpan],
    midi_lead: float,
    start: float,
    end: float,
) -> np.ndarray:
    vector = np.zeros(12, dtype=np.float64)
    for span in spans:
        shifted_start = span.start - midi_lead
        shifted_end = span.end - midi_lead
        overlap = max(
            0.0,
            min(end, shifted_end) - max(start, shifted_start),
        )
        if overlap > 0:
            vector[span.pitch % 12] += overlap
    return vector


def top_pitch_classes(chroma: np.ndarray, count: int = 3) -> tuple[int, ...]:
    vector = np.asarray(chroma)
    if not vector.any():
        return ()
    return tuple(
        int(value)
        for value in np.argsort(vector)[-count:][::-1]
    )


def chroma_agreement(left: np.ndarray, right: np.ndarray) -> float:
    return float(
        np.asarray(left) @ np.asarray(right)
        / (
            np.linalg.norm(left) * np.linalg.norm(right)
            + 1e-9
        )
    )


def build_intervals(
    boundaries: list[float],
    onsets: list[MidiOnset],
    note_spans: list[NoteSpan],
    midi_lead: float,
    analysis: AudioAnalysis,
) -> list[Interval]:
    raw_rms = np.asarray(
        [
            interval_audio_value(
                analysis.rms,
                analysis.times,
                start,
                end,
            )
            for start, end in zip(boundaries, boundaries[1:])
        ]
    )
    raw_centroid = np.asarray(
        [
            interval_audio_value(
                analysis.centroid,
                analysis.times,
                start,
                end,
            )
            for start, end in zip(boundaries, boundaries[1:])
        ]
    )
    raw_bandwidth = np.asarray(
        [
            interval_audio_value(
                analysis.bandwidth,
                analysis.times,
                start,
                end,
            )
            for start, end in zip(boundaries, boundaries[1:])
        ]
    )
    raw_rolloff = np.asarray(
        [
            interval_audio_value(
                analysis.rolloff,
                analysis.times,
                start,
                end,
            )
            for start, end in zip(boundaries, boundaries[1:])
        ]
    )
    raw_flatness = np.asarray(
        [
            interval_audio_value(
                analysis.flatness,
                analysis.times,
                start,
                end,
            )
            for start, end in zip(boundaries, boundaries[1:])
        ]
    )
    raw_contrast = np.asarray(
        [
            interval_audio_value(
                analysis.contrast,
                analysis.times,
                start,
                end,
            )
            for start, end in zip(boundaries, boundaries[1:])
        ]
    )
    low, high = np.quantile(raw_rms, [0.1, 0.9])
    loudness = np.clip((raw_rms - low) / max(high - low, 1e-9), 0, 1)
    amplitudes = (
        MIN_STRENGTH_AMPLITUDE
        + (MAX_STRENGTH_AMPLITUDE - MIN_STRENGTH_AMPLITUDE) * loudness
    )

    intervals = []
    current_strength = -float(amplitudes[0])
    for index, (start, end) in enumerate(
        zip(boundaries, boundaries[1:])
    ):
        wav_chroma = interval_wav_chroma(analysis, start, end)
        midi_chroma = interval_midi_chroma(
            note_spans,
            midi_lead,
            start,
            end,
        )
        tonal_mask = (analysis.times >= start) & (analysis.times < end)
        local_tonnetz = analysis.tonnetz[:, tonal_mask]
        tonal_motion = float(
            np.linalg.norm(np.diff(local_tonnetz, axis=1), axis=0).mean()
            if local_tonnetz.shape[1] > 1
            else 0.0
        )
        target = float(amplitudes[index]) * (1.0 if index % 2 == 0 else -1.0)
        intervals.append(
            Interval(
                start=start,
                end=end,
                rms=float(raw_rms[index]),
                centroid=float(raw_centroid[index]),
                bandwidth=float(raw_bandwidth[index]),
                rolloff=float(raw_rolloff[index]),
                flatness=float(raw_flatness[index]),
                contrast=float(raw_contrast[index]),
                root_pitch_class=root_at_time(onsets, midi_lead, start),
                midi_chord=chord_name(midi_chroma),
                wav_chord=chord_name(wav_chroma),
                midi_pitch_classes=top_pitch_classes(midi_chroma),
                wav_pitch_classes=top_pitch_classes(wav_chroma),
                tonal_agreement=chroma_agreement(
                    midi_chroma,
                    wav_chroma,
                ),
                tonal_motion=tonal_motion,
                strength_start=current_strength,
                strength_end=target,
            )
        )
        current_strength = target
    return intervals


def complete_plot(input_dir: Path, plot: str) -> bool:
    return all(
        (input_dir / f"{plot}__time_{time_name}.jpg").is_file()
        for time_name in TIMES
    )


def select_plots(
    input_dir: Path,
    plot_stats: Path,
    count: int,
    seed: int,
) -> list[str]:
    scores = json.loads(plot_stats.read_text())
    ranked = [
        name
        for name, _score in sorted(scores.items(), key=lambda item: item[1])
        if complete_plot(input_dir, name)
    ]
    if len(ranked) < count:
        raise RuntimeError(
            f"Only {len(ranked)} complete plots are available; requested {count}"
        )
    rng = np.random.default_rng(seed)
    bins = np.array_split(np.asarray(ranked, dtype=object), count)
    return [str(rng.choice(plot_bin)) for plot_bin in bins]


def load_features(manifest_path: Path) -> dict[str, list[Feature]]:
    manifest = json.loads(manifest_path.read_text())
    by_block = {block: [] for block in BLOCKS}
    for item in manifest["features"]:
        by_block[item["block"]].append(
            Feature(
                block=item["block"],
                index=int(item["feature_index"]),
                effect=float(item["screen_effect_mean_absolute_pixels"]),
                source=item["source"],
            )
        )
    for block in by_block:
        by_block[block].sort(key=lambda feature: feature.effect, reverse=True)
    return by_block


def assign_motifs(
    intervals: list[Interval],
    features: dict[str, list[Feature]],
    sections: list[Section],
    pattern_windows: list[PatternWindow],
) -> list[MotifAssignment]:
    plot_count = len(intervals) // len(TIMES)
    section_patterns = {
        "A1-synth": (
            ("style", 1),
            ("style", 2),
            ("style", 1),
        ),
        "B1-ocarina": (
            ("composition", 0),
            ("detail", 0),
            ("composition", 0),
            ("detail", 1),
        ),
        "A2-stone-interlock": (
            ("style", 0),
            ("style", 1),
            ("style", 0),
            ("style", 2),
        ),
        "B2-ocarina-pattern": (
            ("detail", 0),
            ("style", 2),
            ("detail", 0),
            ("style", 3),
        ),
        "A3-ambient-return": (
            ("style", 1),
            ("style", 0),
            ("style", 1),
        ),
    }
    harmonic_patterns = {
        "three-chord": (
            ("style", 1),
            ("style", 0),
            ("style", 1),
            ("style", 2),
        ),
        "five-chord": (
            ("composition", 0),
            ("detail", 0),
            ("composition", 0),
            ("detail", 1),
        ),
    }
    section_counts = {section.name: 0 for section in sections}
    phrase_counts = {
        "three-chord": 0,
        "five-chord": 0,
        "transition-or-ambient": 0,
    }
    assigned = []
    for plot_index in range(plot_count):
        trio = intervals[plot_index * 3 : plot_index * 3 + 3]
        midpoint = (trio[0].start + trio[-1].end) / 2.0
        section = next(
            section
            for section in sections
            if section.start <= midpoint <= section.end
        )
        motif_step = section_counts[section.name]
        section_counts[section.name] += 1
        trio_patterns = [
            interval_pattern(interval.start, interval.end, pattern_windows)
            for interval in trio
        ]
        harmonic_pattern = max(
            dict.fromkeys(trio_patterns),
            key=trio_patterns.count,
        )
        phrase_repetition = phrase_counts[harmonic_pattern]
        phrase_counts[harmonic_pattern] += 1
        pattern = harmonic_patterns.get(
            harmonic_pattern,
            section_patterns[section.name],
        )
        block, feature_index = pattern[phrase_repetition % len(pattern)]
        generation = phrase_repetition // len(pattern)
        timbral_mutation = int(
            np.mean([interval.tonal_motion for interval in trio])
            > np.median([interval.tonal_motion for interval in intervals])
        )
        bucket = features[block]
        evolved_feature_index = feature_index + generation + timbral_mutation
        assigned.append(
            MotifAssignment(
                section=section.name,
                motif_step=motif_step,
                harmonic_pattern=harmonic_pattern,
                phrase_repetition=phrase_repetition,
                feature=bucket[evolved_feature_index % len(bucket)],
            )
        )
    return assigned


def load_plot(input_dir: Path, plot: str) -> list[Image.Image]:
    images = []
    for time_name in TIMES:
        path = input_dir / f"{plot}__time_{time_name}.jpg"
        image = (
            Image.open(path)
            .convert("RGB")
            .resize((MODEL_SIZE, MODEL_SIZE), Image.Resampling.LANCZOS)
        )
        images.append(image)
    return images


@torch.inference_mode()
def render_baseline(
    pipeline: AutoPipelineForImage2Image,
    image: Image.Image,
    steps: int,
    image_strength: float,
    seed: int,
) -> Image.Image:
    return pipeline(
        prompt=PROMPT,
        image=image,
        strength=image_strength,
        num_inference_steps=steps,
        guidance_scale=0.0,
        generator=generator(seed),
    ).images[0]


def smoothstep(value: float) -> float:
    clipped = min(max(value, 0.0), 1.0)
    return clipped * clipped * (3.0 - 2.0 * clipped)


def zoomed_frame(image: Image.Image, progress: float) -> Image.Image:
    crop_width = START_CROP_WIDTH + (
        END_CROP_WIDTH - START_CROP_WIDTH
    ) * smoothstep(progress)
    crop_height = crop_width * FRAME_HEIGHT / FRAME_WIDTH
    left = (MODEL_SIZE - crop_width) / 2.0
    top = (MODEL_SIZE - crop_height) / 2.0
    return image.transform(
        (FRAME_WIDTH, FRAME_HEIGHT),
        Image.Transform.EXTENT,
        (left, top, left + crop_width, top + crop_height),
        Image.Resampling.BICUBIC,
    )


def sound_reactive_grade(
    image: Image.Image,
    mid_signal: float,
    high_signal: float,
    flux_signal: float,
    contrast_signal: float,
    tonal_x: float,
    tonal_y: float,
) -> Image.Image:
    brightness = 1.0 + float(np.clip(0.035 * mid_signal, -0.05, 0.08))
    saturation = 1.0 + float(np.clip(0.06 * high_signal, -0.08, 0.15))
    graded = ImageEnhance.Brightness(image).enhance(brightness)
    graded = ImageEnhance.Color(graded).enhance(saturation)
    graded = ImageEnhance.Contrast(graded).enhance(
        1.0 + float(np.clip(0.025 * contrast_signal, -0.04, 0.07))
    )
    pixels_float = np.asarray(graded, dtype=np.float32)
    gains = np.asarray(
        [
            1.0 + 0.035 * tonal_x,
            1.0 + 0.020 * tonal_y,
            1.0 - 0.035 * tonal_x,
        ],
        dtype=np.float32,
    )
    graded = Image.fromarray(
        np.clip(pixels_float * gains, 0, 255).astype(np.uint8)
    )
    fringe = int(
        round(
            np.clip(
                0.65 * max(high_signal, 0.0)
                + 0.35 * max(flux_signal, 0.0),
                0.0,
                2.0,
            )
        )
    )
    if fringe == 0:
        return graded
    pixels = np.asarray(graded, dtype=np.uint8).copy()
    pixels[..., 0] = np.roll(pixels[..., 0], fringe, axis=1)
    pixels[..., 2] = np.roll(pixels[..., 2], -fringe, axis=1)
    return Image.fromarray(pixels)


def segment_duration(path: Path) -> float:
    if not path.is_file():
        return 0.0
    result = subprocess.run(
        [
            "ffprobe",
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            str(path),
        ],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0 or not result.stdout.strip():
        return 0.0
    return float(result.stdout.strip())


def segment_is_complete(path: Path, expected_duration: float) -> bool:
    return abs(segment_duration(path) - expected_duration) < 0.15


def open_encoder(path: Path) -> subprocess.Popen:
    return subprocess.Popen(
        [
            "ffmpeg",
            "-y",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-s",
            f"{FRAME_WIDTH}x{FRAME_HEIGHT}",
            "-r",
            str(RENDER_FPS),
            "-i",
            "-",
            "-vf",
            (
                f"framerate=fps={OUTPUT_FPS}:interp_start=0:"
                "interp_end=255:scene=100,"
                f"scale={OUTPUT_WIDTH}:{OUTPUT_HEIGHT}:flags=lanczos"
            ),
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "20",
            "-pix_fmt",
            "yuv420p",
            "-f",
            "mpegts",
            str(path),
        ],
        stdin=subprocess.PIPE,
    )


def interpolate_strength_image(
    rendered: list[Image.Image],
    strengths: np.ndarray,
    strength: float,
) -> Image.Image:
    coordinate = float(
        np.interp(
            strength,
            strengths,
            np.arange(len(strengths), dtype=np.float64),
        )
    )
    lower = int(np.floor(coordinate))
    upper = min(lower + 1, len(rendered) - 1)
    fraction = coordinate - lower
    if lower == upper:
        return rendered[lower]
    return Image.blend(
        rendered[lower],
        rendered[upper],
        smoothstep(fraction),
    )


def render_plot_trio(
    pipeline: AutoPipelineForImage2Image,
    loaded_sae,
    images: list[Image.Image],
    feature: Feature,
    intervals: list[Interval],
    analysis: AudioAnalysis,
    full_duration: float,
    segment_path: Path,
    steps: int,
    image_strength: float,
    seed: int,
) -> None:
    start = intervals[0].start
    end = intervals[-1].end
    duration = end - start
    strength_start = intervals[0].strength_start
    strength_end = intervals[-1].strength_end
    grid_padding = 20.0
    strength_grid = np.linspace(
        min(strength_start, strength_end) - grid_padding,
        max(strength_start, strength_end) + grid_padding,
        STRENGTH_COUNT,
    )
    rendered_grid: list[list[Image.Image]] = []
    for image in images:
        baseline = render_baseline(
            pipeline,
            image,
            steps,
            image_strength,
            seed,
        )
        rendered_time = []
        for strength in strength_grid:
            if abs(float(strength)) < 1e-9:
                rendered_time.append(image)
                continue
            steered = apply_feature(
                pipeline,
                loaded_sae,
                BLOCKS[feature.block],
                feature.index,
                float(strength),
                image,
                steps,
                image_strength,
                seed,
            )
            rendered_time.append(
                transplant_pixel_delta(image, baseline, steered)
            )
        rendered_grid.append(rendered_time)

    frame_count = int(round(duration * RENDER_FPS))
    sample_times = start + np.arange(frame_count) / RENDER_FPS
    mask = (
        (analysis.times >= start)
        & (analysis.times <= end)
    )
    local_times = analysis.times[mask]
    local_flux = np.maximum(normalized(analysis.flux)[mask], 0)
    local_rms = np.maximum(normalized(np.log(analysis.rms + 1e-6))[mask], 0)
    motion_density = 0.35 + local_rms + 0.65 * local_flux
    cumulative = np.cumsum(motion_density)
    cumulative = (cumulative - cumulative[0]) / max(
        cumulative[-1] - cumulative[0],
        1e-9,
    )
    musical_phase = np.interp(sample_times, local_times, cumulative)
    easing = musical_phase * musical_phase * (3.0 - 2.0 * musical_phase)
    base_strengths = strength_start + (
        strength_end - strength_start
    ) * easing
    sub_signal = np.interp(
        sample_times,
        analysis.times,
        normalized(np.log(analysis.sub + 1e-9)),
    )
    mid_signal = np.interp(
        sample_times,
        analysis.times,
        normalized(np.log(analysis.mid + 1e-9)),
    )
    high_signal = np.interp(
        sample_times,
        analysis.times,
        normalized(np.log(analysis.high + 1e-9)),
    )
    flux_signal = np.interp(
        sample_times,
        analysis.times,
        normalized(analysis.flux),
    )
    contrast_signal = np.interp(
        sample_times,
        analysis.times,
        normalized(analysis.contrast),
    )
    sampled_chroma = np.vstack(
        [
            np.interp(
                sample_times,
                analysis.times,
                analysis.chroma[pitch_class],
            )
            for pitch_class in range(12)
        ]
    )
    pitch_angles = 2.0 * np.pi * np.arange(12) / 12.0
    chroma_totals = sampled_chroma.sum(axis=0) + 1e-9
    tonal_x = np.cos(pitch_angles) @ sampled_chroma / chroma_totals
    tonal_y = np.sin(pitch_angles) @ sampled_chroma / chroma_totals
    endpoint_window = np.sin(np.pi * musical_phase)
    strength_modulation = np.clip(
        5.0 * sub_signal + 3.0 * flux_signal,
        -10.0,
        10.0,
    ) * endpoint_window
    strengths = base_strengths + strength_modulation
    encoder = open_encoder(segment_path)
    if encoder.stdin is None:
        raise RuntimeError("FFmpeg encoder did not expose stdin")
    try:
        for index, strength in enumerate(strengths):
            time_position = (
                (sample_times[index] - start) * TIME_IMAGE_FPS
            )
            time_lower = int(np.floor(time_position)) % len(TIMES)
            time_upper = (time_lower + 1) % len(TIMES)
            time_fraction = smoothstep(
                time_position - np.floor(time_position)
            )
            lower_time_image = interpolate_strength_image(
                rendered_grid[time_lower],
                strength_grid,
                float(strength),
            )
            upper_time_image = interpolate_strength_image(
                rendered_grid[time_upper],
                strength_grid,
                float(strength),
            )
            edited = Image.blend(
                lower_time_image,
                upper_time_image,
                time_fraction,
            )
            frame_time = float(sample_times[index])
            frame = zoomed_frame(edited, frame_time / full_duration)
            frame = sound_reactive_grade(
                frame,
                float(mid_signal[index]),
                float(high_signal[index]),
                float(flux_signal[index]),
                float(contrast_signal[index]),
                float(tonal_x[index]),
                float(tonal_y[index]),
            )
            encoder.stdin.write(np.asarray(frame, dtype=np.uint8).tobytes())
    finally:
        encoder.stdin.close()
        return_code = encoder.wait()
    if return_code != 0:
        raise RuntimeError(
            f"FFmpeg failed for plot trio at {start:.3f}s"
        )


def assemble_video(
    segments: list[Path],
    audio: Path,
    output_dir: Path,
    output: Path,
) -> None:
    concat_path = output_dir / "segments.txt"
    concat_path.write_text(
        "".join(f"file '{path.resolve()}'\n" for path in segments)
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            "ffmpeg",
            "-y",
            "-loglevel",
            "error",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            str(concat_path),
            "-i",
            str(audio),
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            "256k",
            "-shortest",
            "-movflags",
            "+faststart",
            str(output),
        ],
        check=True,
    )


def main() -> None:
    args = parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    analysis = analyze_audio(args.audio)
    onsets = parse_midi_onsets(args.midi)
    note_spans = parse_midi_note_spans(args.midi)
    midi_lead, alignment_score = estimate_midi_lead(onsets, analysis)
    boundaries = chord_boundaries(onsets, midi_lead, analysis)
    intervals = build_intervals(
        boundaries,
        onsets,
        note_spans,
        midi_lead,
        analysis,
    )
    pattern_windows = detect_pattern_windows(note_spans, midi_lead)
    sections = musical_sections(onsets, midi_lead, analysis.duration)
    plot_count = len(intervals) // len(TIMES)
    if args.segment_limit is not None and not (
        1 <= args.segment_limit <= plot_count
    ):
        raise ValueError(
            "--segment-limit must be between 1 and the plot-trio count"
        )

    plots = select_plots(
        args.input_dir,
        args.plot_stats,
        plot_count,
        args.seed,
    )
    feature_buckets = load_features(args.feature_manifest)
    motif_assignments = assign_motifs(
        intervals,
        feature_buckets,
        sections,
        pattern_windows,
    )
    assignments = []
    for plot_index, (plot, motif) in enumerate(
        zip(plots, motif_assignments)
    ):
        feature = motif.feature
        assignments.append(
            {
                "plot": plot,
                "section": motif.section,
                "motif_step": motif.motif_step,
                "harmonic_pattern": motif.harmonic_pattern,
                "phrase_repetition": motif.phrase_repetition,
                "feature": {
                    "source": feature.source,
                    "block": feature.block,
                    "index": feature.index,
                    "screen_effect": feature.effect,
                },
                "intervals": [
                    {
                        "phrase_interval": time_index + 1,
                        "start": interval.start,
                        "end": interval.end,
                        "rms": interval.rms,
                        "centroid": interval.centroid,
                        "bandwidth": interval.bandwidth,
                        "rolloff": interval.rolloff,
                        "flatness": interval.flatness,
                        "spectral_contrast": interval.contrast,
                        "root_pitch_class": interval.root_pitch_class,
                        "midi_chord": interval.midi_chord,
                        "wav_chord": interval.wav_chord,
                        "midi_pitch_classes": [
                            PITCH_CLASS_NAMES[value]
                            for value in interval.midi_pitch_classes
                        ],
                        "wav_pitch_classes": [
                            PITCH_CLASS_NAMES[value]
                            for value in interval.wav_pitch_classes
                        ],
                        "midi_wav_tonal_agreement": (
                            interval.tonal_agreement
                        ),
                        "tonal_motion": interval.tonal_motion,
                        "harmonic_pattern": interval_pattern(
                            interval.start,
                            interval.end,
                            pattern_windows,
                        ),
                        "strength_start": interval.strength_start,
                        "strength_end": interval.strength_end,
                    }
                    for time_index, interval in enumerate(
                        intervals[
                            plot_index * 3 : plot_index * 3 + 3
                        ]
                    )
                ],
            }
        )
    manifest = {
        "audio": str(args.audio),
        "midi": str(args.midi),
        "audio_duration": analysis.duration,
        "measured_midi_lead_seconds": midi_lead,
        "alignment_score": alignment_score,
        "wav_analysis": {
            "sample_rate": AUDIO_ANALYSIS_RATE,
            "hop_length": AUDIO_HOP,
            "estimated_tuning_semitones": analysis.tuning,
            "estimated_tempo_bpm": analysis.tempo,
            "beat_count": len(analysis.beat_times),
            "descriptors": [
                "CQT chroma",
                "tonal centroid",
                "onset strength",
                "RMS dynamics",
                "spectral centroid",
                "spectral bandwidth",
                "85-percent rolloff",
                "spectral flatness",
                "spectral contrast",
                "sub/mid/high band energy",
            ],
        },
        "model": MODEL_ID,
        "resolution": [OUTPUT_WIDTH, OUTPUT_HEIGHT],
        "output_fps": OUTPUT_FPS,
        "composited_fps": RENDER_FPS,
        "strengths_per_image": STRENGTH_COUNT,
        "time_of_day_animation": {
            "source_times": list(TIMES),
            "source_image_rate_fps": TIME_IMAGE_FPS,
            "sequence": "1000 -> 1200 -> 1500 -> repeat",
            "interpolation": (
                "Every output frame blends consecutive time-of-day images "
                "and consecutive SAE strength anchors."
            ),
        },
        "strength_amplitude_range": [
            MIN_STRENGTH_AMPLITUDE,
            MAX_STRENGTH_AMPLITUDE,
        ],
        "zoom_crop_width": [START_CROP_WIDTH, END_CROP_WIDTH],
        "chord_boundaries": boundaries,
        "harmonic_pattern_windows": [
            {
                "pattern": window.pattern,
                "voicing": window.voicing,
                "start": window.start,
                "end": window.end,
                "cycle_starts": [
                    boundary
                    for boundary in boundaries[:-1]
                    if window.start - 0.1
                    <= boundary
                    <= window.end + 0.1
                ],
            }
            for window in pattern_windows
        ],
        "sections": [
            {
                "name": section.name,
                "motif": section.motif,
                "variation": section.variation,
                "start": section.start,
                "end": section.end,
            }
            for section in sections
        ],
        "motif_patterns": (
            "Two recurring musical/visual families in the form "
            "A1-B1-A2-B2-A3. Each return mutates an established SAE feature "
            "pattern. Plots and feature generations change on three-interval "
            "phrase units; strength phase is warped by WAV energy and novelty."
        ),
        "wav_signal_mapping": {
            "sub_band": "endpoint-safe SAE strength modulation",
            "mid_band": "exposure",
            "high_band": "saturation and restrained RGB fringe",
            "spectral_flux": "morph speed and SAE strength modulation",
            "spectral_contrast": "image contrast",
            "CQT_chroma": "continuous harmony-linked color balance",
            "tonal_centroid_motion": "feature mutation across repetitions",
        },
        "assignments": assignments,
    }
    (args.output_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n"
    )
    print(
        f"MIDI leads WAV by {midi_lead:.3f}s "
        f"(alignment score {alignment_score:.3f}); "
        f"{len(intervals)} intervals, {plot_count} trios.",
        flush=True,
    )

    pipeline = AutoPipelineForImage2Image.from_pretrained(
        MODEL_ID,
        torch_dtype=torch.float16,
        variant="fp16",
        use_safetensors=True,
    ).to("cuda")
    pipeline.set_progress_bar_config(disable=True)
    pipeline.enable_vae_slicing()
    saes = {
        name: load_sae(args.checkpoint_root, module_path, "cuda")
        for name, module_path in BLOCKS.items()
    }

    segments = []
    for plot_index, plot in enumerate(plots):
        if (
            args.segment_limit is not None
            and plot_index >= args.segment_limit
        ):
            break
        motif = motif_assignments[plot_index]
        feature = motif.feature
        trio = intervals[
            plot_index * len(TIMES) : (plot_index + 1) * len(TIMES)
        ]
        segment_path = args.output_dir / (
            f"segment_{plot_index + 1:03d}.ts"
        )
        segments.append(segment_path)
        duration = trio[-1].end - trio[0].start
        if segment_is_complete(segment_path, duration):
            print(
                f"[{plot_index + 1}/{plot_count}] reuse "
                f"{segment_path.name}",
                flush=True,
            )
            continue
        if segment_path.exists():
            os.remove(segment_path)
        loaded_images = load_plot(args.input_dir, plot)
        print(
            f"[{plot_index + 1}/{plot_count}] "
            f"{trio[0].start:.3f}-{trio[-1].end:.3f}s | "
            f"{motif.harmonic_pattern} | "
            f"{plot} 1000/1200/1500 at {TIME_IMAGE_FPS:g} fps | "
            f"{motif.section} motif {motif.motif_step} "
            f"repeat {motif.phrase_repetition} | "
            f"{feature.block}:{feature.index} | "
            f"{trio[0].strength_start:+.1f}"
            f"->{trio[-1].strength_end:+.1f}",
            flush=True,
        )
        render_plot_trio(
            pipeline,
            saes[feature.block],
            loaded_images,
            feature,
            trio,
            analysis,
            analysis.duration,
            segment_path,
            args.steps,
            args.image_strength,
            args.seed,
        )

    if args.segment_limit is not None and args.segment_limit < plot_count:
        print(
            f"Stopped after {args.segment_limit} plot trios as requested.",
            flush=True,
        )
        return
    assemble_video(segments, args.audio, args.output_dir, args.output)
    print(args.output, flush=True)


if __name__ == "__main__":
    main()
