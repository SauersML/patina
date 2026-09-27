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
  tom         a short, high-tuned kick resampled so its pitch is exactly
              E2 (82.41 Hz): a drum that plays the harmony
  crackle     corona discharge — the fizz under high-voltage lines: clusters
              of tiny sparks, each a resonant click, crushed
  wind        eight seconds of wind across an open cut: noise through slowly
              wandering band-passes, gusting, made to loop
  break       one bar at 76 bpm built from the hits above — a swung,
              syncopated groove with ghost notes and a small room baked on,
              then destroyed: wavefolded, hard-clipped, 5-bit, ~8 kHz. The song slices it into sixteen pads and
              re-sequences it: chopped, stuttered, reversed

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


def destroy(x, drive=8.0, bits=5, hold=6):
    """Distorted until it breaks: drive into a sine wavefolder (the peaks
    fold back on themselves), hard-clip, then crush — 5-bit steps and a
    sample-and-hold that drops the rate to ~8 kHz — and saturate what's left."""
    x = drive * x / (np.abs(x).max() + 1e-12)
    x = np.sin(0.5 * np.pi * np.clip(x, -7, 7) / 1.5)          # fold
    x = np.clip(1.6 * x, -1, 1)                                 # clip
    q = 2 ** (bits - 1)
    x = np.round(x * q) / q                                     # bits
    x = np.repeat(x[::hold], hold)[:len(x)]                     # rate
    return np.tanh(2.0 * x)


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


# --- the tom: a drum in tune. Bounce a short kick, find its pitch, and
# resample it so the fundamental lands exactly on E2.
tom = bounce('BD', [('bd_tune', 0.75), ('bd_decay', 0.22), ('bd_sweep', 0.35), ('bd_attack', 0.4), ('bd_drive', 0.2)])
tom = fit(tom, 0.45)
seg = tom[int(0.04 * SR):int(0.25 * SR)] * np.hanning(int(0.25 * SR) - int(0.04 * SR))
spec = np.abs(np.fft.rfft(seg, 1 << 16))
freqs = np.fft.rfftfreq(1 << 16, 1 / SR)
band = (freqs > 45) & (freqs < 260)
f0 = freqs[band][np.argmax(spec[band])]
ratio = f0 / 82.41                       # >1: read faster, pitch goes down to E2
pos = np.arange(0, len(tom) - 1, ratio)
tom = np.interp(pos, np.arange(len(tom)), tom)
print(f'tom        {f0:.1f} Hz -> 82.41 Hz (E2)')
# folded and clipped but not decimated: the harmonics stay on the pitch
t = 6.0 * tom / np.abs(tom).max()
save('tom', fit(np.clip(np.sin(0.5 * np.pi * np.clip(t, -3, 3)) * 1.4, -1, 1), 0.4))

# --- the break: one bar at 76 bpm, sixteen steps, swung, then roomed and
# saturated. Rows are step: (hit, gain).
BPM, STEPS = 76, 16
bar = 4 * 60 / BPM
step = bar / STEPS
hits = {'K': fit(kick, 0.6), 'S': snare, 'H': fit(hat, 0.15, fade=0.01), 'C': fit(clank, 0.5)}
pattern = {
    0: [('K', 1.0), ('H', 0.55)], 1: [('H', 0.25)], 2: [('H', 0.5)], 3: [('K', 0.45)],
    4: [('S', 0.95), ('H', 0.45)], 5: [('H', 0.2)], 6: [('H', 0.5), ('K', 0.6)], 7: [('S', 0.28)],
    8: [('H', 0.55)], 9: [('K', 0.8)], 10: [('K', 0.55), ('H', 0.45)], 11: [('S', 0.22)],
    12: [('S', 1.0), ('H', 0.5)], 13: [('S', 0.3)], 14: [('H', 0.5), ('C', 0.35)], 15: [('H', 0.35), ('S', 0.2)],
}
loop = np.zeros(int(bar * SR) + SR)
for st, row in pattern.items():
    t0 = st * step + (0.12 * step if st % 2 else 0.0)    # swing the off-sixteenths
    i0 = int(t0 * SR)
    for name, g in row:
        x = hits[name][:len(loop) - i0]
        loop[i0:i0 + len(x)] += g * x
room = rng.standard_normal(int(0.35 * SR)) * np.exp(-np.arange(int(0.35 * SR)) / SR / 0.09)
room = lowpass(room, 4000) * 0.12
room[0] = 1.0
loop = np.convolve(loop, room)[:len(loop)]
brk = destroy(loop[:int(bar * SR)])     # exactly one bar: sixteen slices are sixteen steps
save('break', fit(brk, bar, fade=0.003))


# --- corona crackle: Poisson clusters of sparks; each spark a short resonant
# ping at a random height, the whole thing crushed like the drums.
n = int(1.2 * SR)
crackle = np.zeros(n)
t_spark = 0.0
while t_spark < 1.1:
    t_spark += rng.exponential(0.018) if rng.random() < 0.8 else rng.exponential(0.12)
    i0 = int(t_spark * SR)
    if i0 >= n - 2000:
        break
    f = rng.uniform(2500, 9000)
    k = np.arange(600)
    crackle[i0:i0 + 600] += rng.uniform(0.2, 1.0) * np.sin(2 * np.pi * f * k / SR) * np.exp(-k / SR / 0.0015)
save('crackle', fit(destroy(crackle, drive=3.0, bits=7, hold=3), 1.2, fade=0.05))

# --- wind: pink-ish noise through two band-passes whose centres wander, with
# a slow gust envelope; crossfaded end-to-start so it loops without a seam.
secs = 8.0
n = int(secs * SR)
white = rng.standard_normal(n + SR)
pink = lowpass(white, 900) * 3 + lowpass(white, 4000) * 0.6
tt = np.arange(n + SR) / SR
out = np.zeros(n + SR)
for c0, depth, rate in [(420, 260, 0.11), (1400, 700, 0.07)]:
    centre = c0 + depth * np.sin(2 * np.pi * rate * tt + rng.uniform(0, 6))
    y1 = y2 = 0.0
    band = np.zeros_like(pink)
    for i in range(len(pink)):                      # a state-variable band-pass, retuned per sample
        fc = 2 * np.sin(np.pi * centre[i] / SR)
        hp = pink[i] - y2 - 0.35 * y1
        y1 += fc * hp
        y2 += fc * y1
        band[i] = y1
    out += band
gust = 0.55 + 0.45 * np.sin(2 * np.pi * 0.19 * tt) * np.sin(2 * np.pi * 0.07 * tt + 1.0)
out *= gust
xf = SR
loopable = out[:n].copy()
loopable[:xf] = loopable[:xf] * np.linspace(0, 1, xf) + out[n:n + xf] * np.linspace(1, 0, xf)
save('wind', fit(loopable, secs, fade=0.001))
