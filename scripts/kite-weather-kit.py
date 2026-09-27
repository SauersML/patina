#!/usr/bin/env python3
"""Build the drum samples for songs/kite-weather.song into renders/kite-kit/.

Every hit starts life on Patina's own 909 board, bounced one at a time, and
is then broken on purpose:

  kick        the kick tuned into the floor, driven, then saturated hard
  snare       wire-heavy snare through an asymmetric clipper
  snare_verb  that snare with a long, dark room baked onto it — the song
              plays it reversed, so it swells up INTO the downbeat
  hat         a closed hat, hard-clipped (the song crushes it further)
  clank       the rimshot ring-modulated against E (164.81 Hz): a metal
              sound centred on the song's key

    python scripts/kite-weather-kit.py [--patina target/release/patina]

Needs numpy and soundfile.
"""
import argparse, os, subprocess, tempfile
import numpy as np, soundfile as sf

ap = argparse.ArgumentParser()
ap.add_argument('--patina', default='target/release/patina')
ap.add_argument('--out', default='renders/kite-kit')
A = ap.parse_args()
os.makedirs(A.out, exist_ok=True)
SR = 48000


def bounce(hit, knobs):
    """One 909 hit, dry, straight off the board."""
    lines = ['bpm 120', 'tail 1.5']
    for k, v in [('reverb_wet', 0), ('spring', 0), ('chorus_mode', 0), ('tape_age', 0),
                 ('tape_wow', 0), ('tape_flutter', 0), ('tape_drive', 0), ('fuzz', 0), ('dr_tone', 1)] + knobs:
        lines += [f'automate {k}', str(v)]
    lines += ['track k kit=909', f'{hit}@1']
    with tempfile.TemporaryDirectory() as d:
        song, wav = f'{d}/hit.song', f'{d}/hit.wav'
        open(song, 'w').write('\n'.join(lines) + '\n')
        subprocess.run([A.patina, '--play', song, '--render', wav], check=True, capture_output=True)
        x, sr = sf.read(wav, always_2d=True)
    assert sr == SR
    m = x.mean(1)
    start = np.argmax(np.abs(m) > 1e-3 * np.abs(m).max())
    return m[start:]


def fit(x, seconds, fade=0.02):
    x = x[:int(seconds * SR)].copy()
    n = int(fade * SR)
    x[-n:] *= np.linspace(1, 0, n)
    return x / (np.abs(x).max() + 1e-12) * 0.89


def lowpass(x, hz):
    a = np.exp(-2 * np.pi * hz / SR)
    y = np.zeros_like(x)
    acc = 0.0
    for i, v in enumerate(x):
        acc = (1 - a) * v + a * acc
        y[i] = acc
    return y


def save(name, x):
    sf.write(f'{A.out}/{name}.wav', np.stack([x, x], 1).astype(np.float32), SR, subtype='FLOAT')
    print(f'{name:10s} {len(x) / SR:5.2f} s')


kick = bounce('BD', [('bd_tune', 0.12), ('bd_decay', 0.55), ('bd_sweep', 0.6), ('bd_drive', 0.5), ('bd_attack', 0.7)])
kick = np.tanh(3.5 * kick / np.abs(kick).max()) / np.tanh(3.5)
save('kick', fit(lowpass(kick, 6500), 0.6))

snare = bounce('SD', [('sd_tone', 0.25), ('sd_snappy', 0.85), ('sd_decay', 0.6)])
s = snare / np.abs(snare).max()
snare = np.tanh(4 * s + 0.3) - np.tanh(0.3)
snare = fit(snare, 0.5)
save('snare', snare)

# A long, dark room baked on: exponentially decaying noise, lowpassed, as an
# impulse response. Reversed in the song, the tail becomes a swell.
rng = np.random.default_rng(88)
t = np.arange(int(1.3 * SR)) / SR
ir = rng.standard_normal(len(t)) * np.exp(-t / 0.42)
ir = lowpass(ir, 3200)
ir[0] = 6.0
wet = np.convolve(np.pad(snare, (0, len(ir))), ir)[:len(snare) + len(ir)]
save('snare_verb', fit(wet, 1.4, fade=0.05))

hat = bounce('CH', [('hh_metal', 0.8), ('ch_decay', 0.3)])
hat = np.clip(2.5 * hat / np.abs(hat).max(), -1, 1)
save('hat', fit(hat, 0.15, fade=0.01))

rim = bounce('RS', [('rs_tune', 0.3)])
ring = rim * np.sin(2 * np.pi * 164.81 * np.arange(len(rim)) / SR)   # E3: the ring sits on the key
clank = np.tanh(3 * (0.6 * ring + 0.4 * rim) / np.abs(rim).max())
save('clank', fit(clank, 0.5))
