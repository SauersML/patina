"""Reference: modal resonator bank (hypha, 4 Nov 2026). Spec for patina-phys to port and test against.

A modal bank is a sum of independent damped resonators (two-pole filters), one per mode. Each mode m has
  freq_m = f0 * ratio_m * stretch(ratio_m)        (Hz; clipped below Nyquist*0.95, modes above are dropped)
  decay_m = t60 seconds (time to fall 60 dB)      -> pole radius r_m = 10 ** (-3 / (t60_m * sr))
  gain_m  linear
Excitation x[n] (any audio: an impulse, a noise burst, a bow, another track) drives every mode:
  y_m[n] = 2 r cos(w) y_m[n-1] - r^2 y_m[n-2] + g_m * (1 - r) * x[n]      (w = 2 pi freq_m / sr)
  out[n] = sum_m y_m[n] * pan gains
The (1 - r) factor keeps loudness roughly independent of decay. Parameters that must be automatable:
  modal_f0 (from the note), modal_decay (scales every t60), modal_damp (high modes decay faster:
  t60_m = decay * t60_base_m / (1 + damp * (ratio_m - 1))), modal_stretch (inharmonicity: ratio -> ratio * (1 + stretch * (ratio - 1))),
  modal_bright (gain tilt: g_m *= ratio_m ** (-1 + bright)), modal_spread (stereo: odd modes left, even right, by spread),
  modal_modes (how many of the preset's modes sound, from the lowest).
Presets (ratios from the classic physics: bars, plates, bowls):
  bar     free-free bar, asymptotic formula ratio_k = ((2k+1)/3)^2 for k = 2..12 after the fundamental:
          1, 2.78, 5.44, 9.0, 13.4 ... (the exact values are 1, 2.756, 5.404, 8.933, 13.344; either is fine)
  glass   wine glass: 1, 2.32, 4.25, 6.63, 9.38 ... with long decays
  plate   square plate: sqrt(m^2 + n^2) normalised, many dense modes
  bell    church bell partials: 0.5 (hum), 1 (prime), 1.2 (minor third), 1.5, 2, 2.5, 3, 4, 5.3 ...
  string  harmonic 1..N (with stretch > 0 it becomes a stiff piano wire)
Excitations: strike (1-sample impulse filtered by a hardness lowpass), noise (filtered burst, length), bow (continuous
  noise times a slow envelope, for sustain), and 'excite=<track>' (another track's audio is x).
Run this file to write the test renders in specs/out/ (wav at 48 kHz, and npy of the first 4096 samples per case).
"""
import numpy as np, os, wave
SR = 48000
PRESETS = {
    'bar': [1.0] + [((2 * k + 1) / 3) ** 2 / ((3 / 3) ** 2) for k in range(2, 13)],
    'glass': [1.0, 2.32, 4.25, 6.63, 9.38, 12.5, 16.0, 19.9],
    'plate': sorted({np.sqrt(m * m + n * n) / np.sqrt(2) for m in range(1, 8) for n in range(1, 8)})[:40],
    'bell': [0.5, 1.0, 1.2, 1.5, 2.0, 2.5, 3.0, 4.0, 5.33, 6.67, 8.0],
    'string': [float(k) for k in range(1, 41)],
}
BASE_T60 = {'bar': 2.5, 'glass': 6.0, 'plate': 3.0, 'bell': 5.0, 'string': 3.0}

def modal(x, f0, preset='bar', decay=1.0, damp=0.5, stretch=0.0, bright=0.0, spread=0.0, modes=None):
    ratios = np.array(PRESETS[preset][:modes] if modes else PRESETS[preset], float)
    ratios = ratios * (1 + stretch * (ratios - 1))
    freqs = f0 * ratios
    keep = freqs < SR / 2 * 0.95
    ratios, freqs = ratios[keep], freqs[keep]
    t60 = decay * BASE_T60[preset] / (1 + damp * (ratios - 1))
    r = 10 ** (-3 / (t60 * SR))
    w = 2 * np.pi * freqs / SR
    g = ratios ** (-1 + bright)
    g = g / np.sqrt((g ** 2).sum())
    a1, a2 = 2 * r * np.cos(w), -r * r
    y1 = np.zeros(len(ratios)); y2 = np.zeros(len(ratios))
    pans = np.where(np.arange(len(ratios)) % 2 == 0, -spread, spread)
    gl, gr = np.sqrt((1 - pans) / 2), np.sqrt((1 + pans) / 2)
    out = np.zeros((len(x), 2))
    gin = g * (1 - r)
    for n in range(len(x)):          # reference, not fast: clarity over speed
        y = a1 * y1 + a2 * y2 + gin * x[n]
        y2, y1 = y1, y
        out[n, 0] = (y * gl).sum(); out[n, 1] = (y * gr).sum()
    return out

def strike(n, hardness=0.7):
    x = np.zeros(n); x[0] = 1.0
    a = np.exp(-2 * np.pi * (500 + 15000 * hardness) / SR)   # one-pole lowpass: soft mallet vs hard
    for i in range(1, 64): x[i] = x[i - 1] * a
    return x * (1 - a) / (1 - a ** 64) * 40
def bow(n, level=0.02, attack=0.4):
    rng = np.random.default_rng(1)
    env = np.minimum(1, np.arange(n) / (attack * SR))
    return rng.standard_normal(n) * env * level

def write(path, y):
    y = y / (np.abs(y).max() + 1e-12) * 0.9
    with wave.open(path, 'wb') as w:
        w.setnchannels(2); w.setsampwidth(2); w.setframerate(SR)
        w.writeframes((y * 32767).astype('<i2').tobytes())

if __name__ == '__main__':
    os.makedirs('out', exist_ok=True)
    cases = [('bar_strike', dict(preset='bar'), strike, 220.0),
             ('glass_strike', dict(preset='glass', decay=1.0), strike, 660.0),
             ('bell_strike', dict(preset='bell', bright=0.3, spread=0.6), strike, 330.0),
             ('plate_strike', dict(preset='plate', damp=1.0), strike, 180.0),
             ('wire_bow', dict(preset='string', stretch=0.002, damp=0.2, spread=0.4), bow, 110.0),
             ('bar_stretch', dict(preset='bar', stretch=0.15), strike, 220.0)]
    for name, kw, exc, f0 in cases:
        x = exc(int(SR * 3))
        y = modal(x, f0, **kw)
        np.save(f'out/modal_{name}.npy', y[:4096].astype(np.float32))
        write(f'out/modal_{name}.wav', y)
        print(name, 'peak', round(float(np.abs(y).max()), 4))
