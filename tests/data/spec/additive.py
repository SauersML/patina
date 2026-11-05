"""Reference: additive voice with per-partial spectral control (hypha, 4 Nov 2026). Spec for patina-phys.

One note = up to 512 sine partials. Partial k (1..N) has
  freq_k = f0 * k * (1 + stretch * (k - 1)) * 2 ** (spread * d_k / 1200)    d_k: fixed random detune in [-1, 1] per partial
  amp_k  = tilt(k) * oddeven(k) * comb(k) * gate
     tilt(k)    = k ** (-tilt)                  tilt 0 = flat, 1 = saw-like, 2 = dark
     oddeven(k) = 1 if k odd else (1 - odd)      odd 1 = square-like (only odd partials)
     comb(k)    = 1 - comb_depth * (0.5 + 0.5 * cos(2 pi * k * comb_freq + comb_phase))   a moving comb through the spectrum
  pan_k  = width * (+1 if k even else -1) * (0.5 + 0.5 * sin(k * 2.4))   partials fan out across the stereo field
Partials above 0.95 * Nyquist are silent. Each partial keeps its own phase accumulator (no resets on parameter
changes), and every parameter is read per block of 64 samples and smoothed linearly across the block, so all of
them can be automated as fast lanes without clicks. Automatable: add_partials (N), add_tilt, add_odd, add_stretch,
add_spread (cents), add_comb_depth, add_comb_freq, add_comb_phase (moving it sweeps the comb: a phaser through the
partials), add_width. The amplitudes are normalised so total power stays constant as N, tilt and comb change.
Run this file to write test renders in specs/out/.
"""
import numpy as np, os
from modal import write, SR
BLOCK = 64

def additive(dur, f0, lanes, N_max=512, seed=3):
    """lanes: dict param -> function of time t (seconds) -> value. Missing params use defaults."""
    D = dict(add_partials=64, add_tilt=1.0, add_odd=0.0, add_stretch=0.0, add_spread=0.0, add_comb_depth=0.0,
             add_comb_freq=0.1, add_comb_phase=0.0, add_width=0.0)
    rng = np.random.default_rng(seed)
    det = rng.uniform(-1, 1, N_max)
    k = np.arange(1, N_max + 1)
    phase = np.zeros(N_max)
    n_total = int(dur * SR)
    out = np.zeros((n_total, 2))
    def params(t):
        p = {name: (lanes[name](t) if name in lanes else v) for name, v in D.items()}
        n = int(np.clip(p['add_partials'], 1, N_max))
        freq = f0 * k * (1 + p['add_stretch'] * (k - 1)) * 2 ** (p['add_spread'] * det / 1200)
        amp = k ** (-p['add_tilt']) * np.where(k % 2 == 1, 1.0, 1 - p['add_odd'])
        amp *= 1 - p['add_comb_depth'] * (0.5 + 0.5 * np.cos(2 * np.pi * k * p['add_comb_freq'] + p['add_comb_phase']))
        amp[n:] = 0; amp[freq >= SR / 2 * 0.95] = 0
        amp = amp / (np.sqrt((amp ** 2).sum()) + 1e-12) * 0.3
        pan = np.clip(p['add_width'] * np.where(k % 2 == 0, 1, -1) * (0.5 + 0.5 * np.sin(k * 2.4)), -1, 1)
        return freq, amp, pan
    f_prev, a_prev, p_prev = params(0.0)
    tt = np.arange(BLOCK)[:, None] / BLOCK
    for b0 in range(0, n_total, BLOCK):
        f_new, a_new, p_new = params((b0 + BLOCK) / SR)
        m = min(BLOCK, n_total - b0)
        f = f_prev + (f_new - f_prev) * tt[:m]; a = a_prev + (a_new - a_prev) * tt[:m]; p = p_prev + (p_new - p_prev) * tt[:m]
        ph = phase + np.cumsum(2 * np.pi * f / SR, axis=0)
        s = np.sin(ph) * a
        out[b0:b0 + m, 0] = (s * np.sqrt((1 - p) / 2)).sum(1); out[b0:b0 + m, 1] = (s * np.sqrt((1 + p) / 2)).sum(1)
        phase = ph[-1] % (2 * np.pi)
        f_prev, a_prev, p_prev = f_new, a_new, p_new
    return out

if __name__ == '__main__':
    os.makedirs('out', exist_ok=True)
    cases = {
        'static_saw64': (2.0, 110, {}),
        'open_cluster': (6.0, 110, {'add_partials': lambda t: 1 + 255 * min(1, t / 4), 'add_tilt': lambda t: 1.6 - 0.9 * min(1, t / 4),
                                     'add_spread': lambda t: 40 * min(1, t / 5), 'add_width': lambda t: min(1, t / 3)}),
        'comb_sweep': (4.0, 82.5, {'add_partials': lambda t: 256, 'add_tilt': lambda t: 0.6, 'add_comb_depth': lambda t: 0.95,
                                   'add_comb_freq': lambda t: 0.06, 'add_comb_phase': lambda t: 2 * np.pi * 0.5 * t}),
        'odd_morph_stretch': (4.0, 110, {'add_partials': lambda t: 128, 'add_odd': lambda t: 0.5 + 0.5 * np.sin(np.pi * t),
                                         'add_stretch': lambda t: 0.004 * t}),
    }
    for name, (dur, f0, lanes) in cases.items():
        y = additive(dur, f0, lanes)
        np.save(f'out/add_{name}.npy', y[:4096].astype(np.float32))
        write(f'out/add_{name}.wav', y)
        print(name, 'peak', round(float(np.abs(y).max()), 4))
