"""Kokoro choir on a chord loop, sustained by TD-PSOLA.

Kokoro is a speech model: it gives a spoken 'ah' of ~1.5 s at most. Each
glottal period of that vowel becomes a two-period Hann grain; grains are
overlap-added at the target pitch (vibrato, 1.5-cent jitter, shimmer) while
the read position random-walks slowly through the vowel. There is no loop
length to retrigger on, and formants are preserved because nothing is
resampled. Consecutive grains are phase-aligned by cross-correlation.

    .venv-voice/bin/python scripts/kokoro-choir.py renders/kokoro-choir-am-f.wav
    env: TAKES (voices per part, default 1)  VIB (cents, 15)  VIB_RATE (Hz, 5)  ONLY (one voice)
Vowel renders are cached in ~/.cache/patina-kokoro/.
"""
import os, subprocess, sys, tempfile
import numpy as np, soundfile as sf, librosa
from scipy.signal import butter, sosfiltfilt

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PY = f"{REPO}/.venv-voice/bin/python"
OUT = sys.argv[1]
SR = 24000
ENV = dict(os.environ, VIRTUAL_ENV=f"{REPO}/.venv-voice", PATH=f"{REPO}/.venv-voice/bin:" + os.environ["PATH"])
CACHE = os.path.expanduser("~/.cache/patina-kokoro")
rng = np.random.default_rng(7)

NOTE = {n: i for i, n in enumerate(["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"])}
def hz(name):
    return 440.0 * 2 ** ((NOTE[name[:-1]] + 12 * (int(name[-1]) + 1) - 69) / 12)

def render(voice):
    """A slow 'ah. ah. ah. ah.' — speed 0.3 gives the longest continuous vowel Kokoro will hold."""
    os.makedirs(CACHE, exist_ok=True)
    path = f"{CACHE}/{voice}-ah-slow.wav"
    if not os.path.exists(path):
        with tempfile.TemporaryDirectory() as td:
            subprocess.run([PY, "-m", "mlx_audio.tts.generate", "--model", "mlx-community/Kokoro-82M-bf16",
                            "--voice", voice, "--speed", "0.3", "--text", "ah. ah. ah. ah.",
                            "--file_prefix", f"{td}/v"], env=ENV, check=True, capture_output=True)
            os.replace(f"{td}/v_000.wav", path)
    y, r = sf.read(path, dtype="float32")
    if y.ndim > 1: y = y.mean(axis=1)
    if r != SR: y = librosa.resample(y, orig_sr=r, target_sr=SR)
    return y

def pitch_marks(y):
    """One mark per glottal period through the longest voiced run, each
    snapped to the crest of the low-passed waveform so grains share a phase."""
    hop = 64
    f0, vflag, _ = librosa.pyin(y, fmin=60, fmax=600, sr=SR, frame_length=1024, hop_length=hop)
    t = librosa.frames_to_time(np.arange(len(f0)), sr=SR, hop_length=hop)
    good = vflag & ~np.isnan(f0)
    best, cur = (0, 0), 0
    for i, g in enumerate(good):
        cur = cur + 1 if g else 0
        if cur > best[0]: best = (cur, i - cur + 1)
    n, s0 = best
    lo, hi = t[s0] + 0.02, t[s0 + n - 1] - 0.02
    f0i = lambda tt: np.interp(tt, t[good], f0[good])
    smooth = sosfiltfilt(butter(4, 1.6 * float(np.median(f0[good])), btype="low", fs=SR, output="sos"), y)
    marks, tt = [], lo
    while tt < hi:
        i = int(tt * SR); P = SR / f0i(tt); w = int(P * 0.15)
        seg = smooth[max(0, i - w): i + w]
        if len(seg): i = max(0, i - w) + int(np.argmax(seg))
        marks.append(i)
        tt = i / SR + 1.0 / f0i(tt)
    return np.array(marks)

def grains(y, marks):
    """Two-period Hann grains around each mark, level-normalized."""
    out = []
    for k in range(1, len(marks) - 1):
        a, b = marks[k - 1], marks[k + 1]
        g = y[a:b] * np.hanning(b - a)
        g = g / (np.sqrt(np.mean(g ** 2)) + 1e-6)
        out.append((g, marks[k] - a))
    return out

class Vibrato:
    """A singer's vibrato: the rate wanders slowly around its centre, the depth
    fades in over the first second and follows the dynamics, every cycle is a
    little different, and a touch of tremolo rides on the pitch wobble."""
    def __init__(self, rate, cents, onset=0.5, wander=0.35):
        self.rate, self.cents, self.onset, self.wander = rate, cents, onset, wander
        self.phase = rng.uniform(0, 2 * np.pi)
        self.rate_dev, self.depth_dev = 0.0, 0.0

    def step(self, t, dt, level):
        # slow random walks (Ornstein-Uhlenbeck) on rate and depth
        self.rate_dev += (-self.rate_dev * 0.6 + rng.normal(0, 1.2)) * dt
        self.depth_dev += (-self.depth_dev * 0.8 + rng.normal(0, 0.9)) * dt
        rate = self.rate + self.wander * self.rate_dev
        fade = min(1.0, max(0.0, (t - self.onset) / 0.9)) ** 1.5
        depth = self.cents * fade * (0.55 + 0.6 * level) * (1 + 0.3 * self.depth_dev)
        self.phase += 2 * np.pi * rate * dt
        wobble = np.sin(self.phase)
        cents = depth * wobble + rng.normal(0, 1.5)
        tremolo = 1 + 0.06 * fade * (0.5 + level) * np.sin(self.phase - 0.4)
        return cents, tremolo

def swell(t, dur, peak_at=0.55, floor=0.4):
    """Messa di voce: grow into the note, ease out of it."""
    x = t / max(dur, 1e-6)
    hump = np.sin(np.pi * min(1.0, x / peak_at) * 0.5) if x < peak_at else np.cos(np.pi * (x - peak_at) / (1 - peak_at) * 0.5)
    return floor + (1 - floor) * max(0.0, hump)

def psola(gr, target, seconds, vib, dyn, detune_cents=0.0):
    """Lay grains at the target period through `seconds`, reading slowly and a
    little randomly through the source. Pitch = target * vibrato * jitter;
    level follows dyn(t) (0..1) with the vibrato's tremolo on top."""
    n = int(seconds * SR)
    out = np.zeros(n + 4096, dtype=np.float32)
    n_src = len(gr)
    pos, t, read = 0.0, 0.0, 0.0
    read_speed = n_src / (seconds * target) * 0.9
    prev, prev_k = None, -1
    while t < seconds:
        level = dyn(t)
        cents, tremolo = vib.step(t, 1.0 / target, level)
        cents += detune_cents
        f = target * 2 ** (cents / 1200)
        k = int(round(read)) % n_src
        g, c = gr[k]
        if k != prev_k and prev is not None:
            P = int(SR / f); w = max(1, P // 3); m = min(len(g), len(prev))
            best, best_lag = -1e9, 0
            for lag in range(-w, w + 1):
                a0, a1 = max(0, lag), min(m, m + lag)
                if a1 - a0 < m // 2: continue
                score = float(np.dot(g[a0:a1], prev[a0 - lag:a1 - lag]))
                if score > best: best, best_lag = score, lag
            c = c + best_lag
        prev, prev_k = g, k
        i = int(pos) - c
        amp = (0.2 + 0.8 * level) * tremolo * (1.0 + rng.normal(0, 0.03))
        a, b = max(0, i), min(len(out), i + len(g))
        out[a:b] += g[a - i: b - i] * amp
        pos += SR / f
        t = pos / SR
        read = max(0.0, min(n_src - 1.0, read + read_speed + rng.normal(0, 0.2)))
    return out[:n]

def colour(y, dyn_curve):
    """Quieter = darker: crossfade toward a 1.4 kHz low-pass as the level drops."""
    lp = sosfiltfilt(butter(2, 1400, btype="low", fs=SR, output="sos"), y)
    bright = 0.35 + 0.65 * dyn_curve
    return (lp + (y - lp) * bright).astype(np.float32)

def envelope(y, a=0.12, r=0.25):
    n = len(y); env = np.ones(n, dtype=np.float32)
    na, nr = int(a * SR), int(r * SR)
    env[:na] = np.linspace(0, 1, na) ** 1.5; env[-nr:] *= np.linspace(1, 0, nr) ** 1.5
    return y * env

def f0_acf(y, fmin=60.0, fmax=600.0):
    y = y - y.mean()
    n = 1 << int(np.ceil(np.log2(2 * len(y))))
    ac = np.fft.irfft(np.abs(np.fft.rfft(y, n)) ** 2)[: len(y)]
    lo, hi = int(SR / fmax), int(SR / fmin)
    seg = ac[lo:hi]; thresh = seg.max() * 0.85
    k = next(i for i in range(1, len(seg) - 1) if seg[i] >= thresh and seg[i] >= seg[i - 1] and seg[i] >= seg[i + 1])
    a, b, c = seg[k - 1], seg[k], seg[k + 1]
    return SR / (lo + k + 0.5 * (a - c) / (a - 2 * b + c))

# --- the loop the user played: open fifths, D then C over A; A then D over F ---
BAR = float(os.environ.get('BAR', '4.70588'))   # one bar of 4 at 51 bpm; the user's take was 4.8 / 4.6
AM, F = BAR, BAR
CYCLE = AM + F
PARTS = {  # voice: [(note, start, dur, gain)]
    "am_michael": [("A2", 0.0, AM, 0.8), ("F2", AM, F, 0.8)],
    "am_adam":    [("E3", 0.0, AM, 0.5), ("C3", AM, F, 0.5)],
    "bf_emma":    [("A3", 0.0, AM, 0.4), ("C4", AM, F, 0.4)],
    "af_heart":   [("D4", 0.0, 1.0, 0.7), ("C4", 1.0, AM - 1.0, 0.7),
                   ("A3", AM, 2.0, 0.7), ("D4", AM + 2.0, F - 2.0, 0.7)],
}
# vibrato (rate Hz, depth cents, onset s): bass slow and narrow, melody freest
VIBRATO = {"am_michael": (4.6, 10, 0.7), "am_adam": (4.9, 13, 0.6), "bf_emma": (5.2, 15, 0.5), "af_heart": (5.4, 20, 0.35)}

def phrase_level(t_cycle):
    """The A bar builds toward the F bar; the F bar settles back to where the loop began."""
    if t_cycle < AM:
        return 0.5 + 0.5 * (t_cycle / AM) ** 1.3
    x = (t_cycle - AM) / F
    return 1.0 - 0.45 * x ** 1.6
CYCLES = 2
TAKES = int(os.environ.get("TAKES", "1"))
VIB = float(os.environ.get("VIB", "15"))
VIB_RATE = float(os.environ.get("VIB_RATE", "5.0"))
ONLY = os.environ.get("ONLY")

N = int(CYCLE * CYCLES * SR)
mix = np.zeros(N, dtype=np.float32)
for voice, events in PARTS.items():
    if ONLY and voice != ONLY: continue
    y = render(voice)
    gr = grains(y, pitch_marks(y))
    print(f"{voice}: {len(gr)} grains", flush=True)
    v_rate, v_cents, v_onset = VIBRATO[voice]
    for take in range(TAKES):
        detune = rng.uniform(-6, 6) if take else 0.0
        for c in range(CYCLES):
            for note, start, dur, gain in events:
                vib = Vibrato(v_rate * VIB_RATE / 5.0 + rng.uniform(-0.2, 0.2), v_cents * VIB / 15.0, v_onset)
                melody = voice == "af_heart"
                dyn = lambda t, start=start, dur=dur, melody=melody: swell(t, dur, floor=0.35 if melody else 0.5) * phrase_level((start + t) % CYCLE)
                seg = psola(gr, hz(note), dur + 0.12, vib, dyn, detune)
                curve = np.array([dyn(i / SR) for i in range(0, len(seg), 240)], dtype=np.float32)
                curve = np.interp(np.arange(len(seg)), np.arange(0, len(seg), 240), curve)
                seg = envelope(colour(seg, curve))
                wanted = float(np.mean(0.2 + 0.8 * curve))          # the loudness this note was given
                seg *= gain / TAKES / (np.sqrt(np.mean(seg ** 2)) + 1e-6) * 0.15 * wanted
                if c == 0 and take == 0:
                    sung = f0_acf(seg[int(0.15 * SR): int(0.4 * SR)])
                    print(f"   {note} target {hz(note):.1f} Hz -> sung {sung:.1f} Hz ({1200*np.log2(sung/hz(note)):+.0f} cents)", flush=True)
                i = int((c * CYCLE + start) * SR)
                idx = (np.arange(len(seg)) + i) % N      # tails wrap: the file is a seamless loop
                np.add.at(mix, idx, seg)
mix /= max(1e-6, np.abs(mix).max()) / 0.85
sf.write(OUT, mix, SR)
print("wrote", OUT, f"{len(mix)/SR:.1f}s")
