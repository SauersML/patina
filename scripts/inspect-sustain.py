"""Level ripple, envelope periodicity and a zoomed spectrogram PNG of a wav
region: inspect-sustain.py file.wav t0 t1. A steady tone reads ~0.5 dB."""
import sys, zlib, struct, numpy as np, soundfile as sf, librosa
path, t0, t1 = sys.argv[1], float(sys.argv[2]), float(sys.argv[3])
y, sr = sf.read(path, dtype="float32")
if y.ndim > 1: y = y.mean(axis=1)
seg = y[int(t0 * sr): int(t1 * sr)]
n = int(0.05 * sr)
env = np.sqrt(np.convolve(seg ** 2, np.ones(n) / n, mode="valid"))[::int(0.01 * sr)]
e = env - env.mean(); ac = np.correlate(e, e, "full")[len(e) - 1:]; ac /= ac[0] + 1e-9
lags = np.arange(len(ac)) / 100.0
m = (lags > 0.12) & (lags < 2.0); k = np.argmax(ac[m])
print(f"{path.split('/')[-1]} {t0}-{t1}s: level ripple {20*np.log10(env.max()/max(env.min(),1e-6)):.1f} dB, "
      f"strongest envelope repeat every {lags[m][k]:.2f}s strength {ac[m][k]:.2f}")
hop = int(0.002 * sr)
D = librosa.amplitude_to_db(np.abs(librosa.stft(seg, n_fft=2048, hop_length=hop)), ref=np.max)
freqs = librosa.fft_frequencies(sr=sr, n_fft=2048)
rows = np.where((freqs >= 60) & (freqs <= 1500))[0]
img = np.clip((D[rows][::-1] + 60) / 60, 0, 1); h, w = img.shape
g = (img * 255).astype(np.uint8)
raw = b"".join(b"\x00" + g[i].tobytes() for i in range(h))
def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 0, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
out = path.rsplit(".", 1)[0] + f"-{t0:g}-{t1:g}.png"; open(out, "wb").write(png)
print(f"wrote {out} ({w}x{h}: {t1-t0:g}s wide, 1500 Hz top to 60 Hz bottom)")
