// The spectral vocoder — cross-synthesis at full resolution. vox_mode 3.
//
// Every band vocoder has a ceiling: 20 bands is a 20-pixel picture of
// the mouth, and it always sounds like one ("vocodery"). This circuit
// is the other end of the line — the FFT vocoder of the software era
// (Zynaptiq/Orange-Vocoder lineage): a 1024-point short-time spectrum,
// ~47 Hz per bin at 48 kHz, five hundred effective bands.
//
// Per frame (75% overlap, Hann):
//   1. FFT the modulator and the carrier.
//   2. Extract each one's SPECTRAL ENVELOPE by cepstral liftering: take
//      log|X|, FFT it, keep only the low quefrencies (the slow ripple =
//      the mouth), discard the high ones (the fast comb = the pitch),
//      inverse, exp. This is how the envelope and the source are
//      separated without either contaminating the other.
//   3. WHITEN the carrier (divide by its own envelope) so its formants
//      don't fight the voice's, then multiply by the modulator's
//      envelope. The carrier keeps its harmonic soul — pitch, phase,
//      buzz; the voice contributes articulation only, at a resolution
//      where every consonant survives intact.
//   4. Overlap-add back to time.
//
// The result is the sound the band vocoder only gestures at: a lead
// where the words are completely clear and the tone is completely the
// instrument's.

const N: usize = 1024;
const HOP: usize = N / 4;
/// Cepstral cutoff in seconds: ripple slower than this is envelope,
/// faster is source. 2 ms sits safely below any singing pitch period.
const LIFTER_S: f32 = 0.002;
/// Whitening floor: bins more than ~36 dB under the carrier's envelope
/// peak stop being amplified (they hold no real signal, only noise).
const WHITEN_FLOOR: f32 = 0.015;
/// Per-bin gain ceiling after whitening.
const MAX_GAIN: f32 = 10.0;

/// Twiddle factors for every radix-2 stage, computed once.
///
/// A stage's factors depend only on its length, not on the transform's,
/// so one flat table serves every power-of-two size up to N: stage `len`
/// occupies `len/2` entries starting at `len/2 - 1`. N-1 entries total,
/// ~8 KB.
///
/// This replaces recomputing each twiddle inside the butterfly loop by
/// complex multiplication — four multiplies and two adds of pure
/// bookkeeping on every one of the ~140 butterflies this circuit runs per
/// output sample, on the audio thread. It is also more accurate: the
/// recurrence drifted over the 512 iterations of the widest stage.
fn twiddles() -> &'static [(f32, f32)] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Vec<(f32, f32)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut v = Vec::with_capacity(N - 1);
        let mut len = 2usize;
        while len <= N {
            for k in 0..len / 2 {
                let a = -std::f32::consts::TAU * k as f32 / len as f32;
                v.push((a.cos(), a.sin()));
            }
            len <<= 1;
        }
        v
    })
}

/// Radix-2 iterative FFT, in place. `inverse` includes the 1/N scale.
/// `n` must be a power of two no larger than N (the twiddle table's size).
fn fft(re: &mut [f32], im: &mut [f32], inverse: bool) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && n <= N, "fft size {n}");
    let tw = twiddles();
    // Bit-reversal permutation
    let mut j = 0usize;
    for i in 0..n {
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
        let mut m = n >> 1;
        while m >= 1 && j & m != 0 {
            j ^= m;
            m >>= 1;
        }
        j |= m;
    }
    // The inverse transform is the forward one with conjugated twiddles
    let conj = if inverse { -1.0f32 } else { 1.0 };
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let stage = &tw[half - 1..half - 1 + half];
        let mut i = 0;
        while i < n {
            for (k, &(wr, wi)) in stage.iter().enumerate() {
                let (cr, ci) = (wr, conj * wi);
                let (ar, ai) = (re[i + k], im[i + k]);
                let (br, bi) = (re[i + k + half], im[i + k + half]);
                let (tr, ti) = (br * cr - bi * ci, br * ci + bi * cr);
                re[i + k] = ar + tr;
                im[i + k] = ai + ti;
                re[i + k + half] = ar - tr;
                im[i + k + half] = ai - ti;
            }
            i += len;
        }
        len <<= 1;
    }
    if inverse {
        let s = 1.0 / n as f32;
        for k in 0..n {
            re[k] *= s;
            im[k] *= s;
        }
    }
}

/// Cepstrally-smoothed spectral envelope of a magnitude spectrum.
fn envelope(mag: &[f32; N], lifter_bins: usize, out: &mut [f32; N]) {
    let mut re = [0.0f32; N];
    let mut im = [0.0f32; N];
    for k in 0..N {
        re[k] = mag[k].max(1e-9).ln();
    }
    fft(&mut re, &mut im, false);
    // Keep low quefrencies (both ends: the cepstrum is symmetric)
    for q in lifter_bins..N - lifter_bins {
        re[q] = 0.0;
        im[q] = 0.0;
    }
    fft(&mut re, &mut im, true);
    for k in 0..N {
        out[k] = re[k].exp();
    }
}

pub struct Spectral {
    lifter_bins: usize,
    // Input accumulation and output overlap-add rings
    in_m: [f32; N],
    in_c: [f32; N],
    ola: [f32; N],
    /// Where the next input sample lands — and therefore where the OLDEST
    /// held sample currently sits, which is where a frame starts reading.
    ///
    /// These two buffers used to be shifted left by one every sample
    /// (`copy_within(1.., 0)`), which is 8 KB of memmove per sample: at
    /// 48 kHz that is ~390 MB/s of pure copying on the audio thread, for
    /// a circuit whose actual work happens once every HOP samples. The
    /// host is entitled to kill a plugin that behaves like that, and this
    /// one has been reported for exactly that. A write cursor costs one
    /// store instead.
    in_pos: usize,
    fill: usize,
    out_pos: usize,
    window: [f32; N],
    // SHAPE-FREEZE: the held, smoothed mouth shape. The voice's envelope
    // is normalized to unit mean (pure shape, no loudness) and slewed at
    // two rates — fast on syllable onsets, slow inside vowels — and when
    // the voice goes quiet the shape FREEZES instead of tracking
    // silence. A held note keeps the mouth open; every trace of the
    // voice's amplitude micro-flutter (glottal ripple, breath shimmer,
    // frame-rate wobble — the "vocodery" instability) is erased, because
    // the carrier's own envelope is the only thing that moves the level.
    shape: [f32; N],
    have_shape: bool,
    prev_energy: f32,
}

impl Spectral {
    pub fn new(sample_rate: f32) -> Self {
        let mut window = [0.0f32; N];
        for (i, w) in window.iter_mut().enumerate() {
            *w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / N as f32).cos();
        }
        Self {
            lifter_bins: ((LIFTER_S * sample_rate) as usize).clamp(8, N / 4),
            in_m: [0.0; N],
            in_c: [0.0; N],
            ola: [0.0; N],
            in_pos: 0,
            fill: 0,
            out_pos: 0,
            window,
            shape: [0.0; N],
            have_shape: false,
            prev_energy: 0.0,
        }
    }

    fn frame(&mut self) {
        let mut mr = [0.0f32; N];
        let mut mi = [0.0f32; N];
        let mut cr = [0.0f32; N];
        let mut ci = [0.0f32; N];
        // Read the rings in time order: oldest sample first. `in_pos` is
        // both the next write slot and the oldest sample, so the frame is
        // [in_pos..N] followed by [0..in_pos].
        let (m_recent, m_oldest) = self.in_m.split_at(self.in_pos);
        let (c_recent, c_oldest) = self.in_c.split_at(self.in_pos);
        for (k, ((&m, &c), &w)) in m_oldest
            .iter()
            .chain(m_recent)
            .zip(c_oldest.iter().chain(c_recent))
            .zip(self.window.iter())
            .enumerate()
        {
            mr[k] = m * w;
            cr[k] = c * w;
        }
        fft(&mut mr, &mut mi, false);
        fft(&mut cr, &mut ci, false);

        let mut m_mag = [0.0f32; N];
        let mut c_mag = [0.0f32; N];
        for k in 0..N {
            m_mag[k] = (mr[k] * mr[k] + mi[k] * mi[k]).sqrt();
            c_mag[k] = (cr[k] * cr[k] + ci[k] * ci[k]).sqrt();
        }
        let mut m_env = [0.0f32; N];
        let mut c_env = [0.0f32; N];
        envelope(&m_mag, self.lifter_bins, &mut m_env);
        envelope(&c_mag, self.lifter_bins, &mut c_env);

        // Update the held mouth shape only while the voice is actually
        // speaking; silence freezes it (the mouth stays open)
        let energy = m_mag.iter().map(|v| v * v).sum::<f32>() / N as f32;
        if energy > 1e-4 {
            let mean = m_env.iter().sum::<f32>() / N as f32;
            // Onset (energy doubled since last frame): snap in one frame.
            // Inside a vowel: glide at ~40 ms so nothing can flutter.
            let k_slew = if energy > 2.0 * self.prev_energy || !self.have_shape {
                1.0
            } else {
                0.12
            };
            for k in 0..N {
                let target = m_env[k] / mean.max(1e-9);
                self.shape[k] += (target - self.shape[k]) * k_slew;
            }
            self.have_shape = true;
        }
        self.prev_energy = energy;

        // Whiten the carrier, dress it in the HELD shape: per-bin real
        // gain, phases untouched — the carrier's harmonic core, level,
        // and envelope are the only things that move
        let c_peak = c_env.iter().fold(1e-9f32, |a, &v| a.max(v));
        for k in 0..N {
            let g = (self.shape[k] * 0.35 / (c_env[k].max(WHITEN_FLOOR * c_peak) / c_peak))
                .min(MAX_GAIN);
            cr[k] *= g;
            ci[k] *= g;
        }
        fft(&mut cr, &mut ci, true);

        // Overlap-add (Hann analysis + Hann synthesis at N/4 hop sums to
        // 1.5; fold that into the output scale)
        for k in 0..N {
            let idx = (self.out_pos + k) % N;
            self.ola[idx] += cr[k] * self.window[k] / 1.5;
        }
    }

    /// One sample in, one sample out (N-sample latency, constant).
    #[inline]
    pub fn process(&mut self, modulator: f32, carrier: f32) -> f32 {
        // One store per input, not a 4 KB shift each (see `in_pos`)
        self.in_m[self.in_pos] = modulator;
        self.in_c[self.in_pos] = carrier;
        self.in_pos = (self.in_pos + 1) % N;

        let y = self.ola[self.out_pos];
        self.ola[self.out_pos] = 0.0;
        self.out_pos = (self.out_pos + 1) % N;

        self.fill += 1;
        if self.fill >= HOP {
            self.fill = 0;
            self.frame();
        }
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1 kHz-resonant voice must shape the carrier near 1 kHz and
    /// leave the rest shut; BEFORE any voice has spoken the output is
    /// silent, and AFTER the voice stops the frozen shape holds — the
    /// carrier keeps sounding through the open mouth.
    #[test]
    fn envelope_transfers_at_high_resolution() {
        let sr = 48000.0;
        let mut s = Spectral::new(sr);
        // Phase 0: carrier plays, but no voice has EVER spoken — no
        // shape exists yet, so nothing may pass
        let mut virgin = 0.0f32;
        for n in 0..(sr as usize / 4) {
            let carrier = (((n as f32 * 110.0 / sr) % 1.0) * 2.0 - 1.0) * 5.0;
            let v = s.process(0.0, carrier).abs();
            if n > (0.1 * sr) as usize {
                virgin = virgin.max(v);
            }
        }
        assert!(virgin < 0.05, "no shape yet: carrier must stay shut, got {virgin}");

        let mut noise = crate::noise::NoiseSource::new(sr);
        let (mut y1, mut y2) = (0.0f32, 0.0f32);
        let c = -(-std::f32::consts::TAU * 100.0 / sr).exp();
        let b = 2.0 * (-std::f32::consts::PI * 100.0 / sr).exp()
            * (std::f32::consts::TAU * 1000.0 / sr).cos();
        let a = 1.0 - b - c;
        let mut out = Vec::with_capacity(sr as usize);
        for n in 0..(sr as usize) {
            let m = {
                let y = a * noise.next() + b * y1 + c * y2;
                y2 = y1;
                y1 = y;
                y * 0.5
            };
            let carrier = (((n as f32 * 110.0 / sr) % 1.0) * 2.0 - 1.0) * 5.0;
            out.push(s.process(m, carrier));
        }
        assert!(out.iter().all(|v| v.is_finite()));
        let goertzel = |freq: f32| -> f32 {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, &v) in out[out.len() / 2..].iter().enumerate() {
                let ph = std::f32::consts::TAU * freq * i as f32 / sr;
                re += v * ph.cos();
                im += v * ph.sin();
            }
            (re * re + im * im).sqrt()
        };
        // 990 Hz = carrier harmonic 9*110 inside the voice's resonance;
        // 2970 Hz = harmonic 27 far outside it. High resolution should
        // separate them far more sharply than 20 bands ever could.
        let near = goertzel(990.0);
        let far = goertzel(2970.0);
        assert!(
            near > 6.0 * far,
            "1 kHz voice energy must shape the carrier sharply: near={near}, far={far}"
        );

        // Phase 2: the voice stops but the carrier holds — the FROZEN
        // shape must keep the carrier sounding (the mouth stays open;
        // the note's own envelope owns the dynamics now)
        let mut held = 0.0f32;
        for n in 0..(sr as usize / 4) {
            let carrier = (((n as f32 * 110.0 / sr) % 1.0) * 2.0 - 1.0) * 5.0;
            let v = s.process(0.0, carrier).abs();
            if n > (0.1 * sr) as usize {
                held = held.max(v);
            }
        }
        assert!(
            held > 0.2,
            "the frozen mouth must keep the held note sounding, got {held}"
        );
    }

    /// The transform itself, pinned independently of the vocoder around
    /// it: the twiddles come from a table now, not from a per-butterfly
    /// complex-multiply recurrence, and a wrong table would be a subtle
    /// spectral smear rather than an obvious failure.
    #[test]
    fn fft_round_trips_and_lands_on_the_right_bin() {
        // A pure cosine at bin 37 must put all its energy in bins 37 and
        // N-37, and nowhere else
        let bin = 37usize;
        let mut re = [0.0f32; N];
        let mut im = [0.0f32; N];
        for k in 0..N {
            re[k] = (std::f32::consts::TAU * bin as f32 * k as f32 / N as f32).cos();
        }
        let original = re;
        fft(&mut re, &mut im, false);
        for k in 0..N {
            let mag = (re[k] * re[k] + im[k] * im[k]).sqrt();
            if k == bin || k == N - bin {
                assert!((mag - N as f32 / 2.0).abs() < 0.5, "bin {k} magnitude {mag}");
            } else {
                assert!(mag < 0.05, "bin {k} should be empty, got {mag}");
            }
        }
        // ...and the inverse must give the signal back
        fft(&mut re, &mut im, true);
        for k in 0..N {
            assert!(
                (re[k] - original[k]).abs() < 1e-4,
                "round trip drifted at {k}: {} vs {}",
                re[k],
                original[k]
            );
            assert!(im[k].abs() < 1e-4, "round trip grew an imaginary part at {k}");
        }
        // Linearity across a non-trivial signal, at the sizes the
        // envelope path also uses
        for n in [16usize, 256, N] {
            let mut r: Vec<f32> = (0..n).map(|i| ((i * 7919) % 101) as f32 / 50.0 - 1.0).collect();
            let mut i_ = vec![0.0f32; n];
            let want = r.clone();
            fft(&mut r, &mut i_, false);
            fft(&mut r, &mut i_, true);
            for k in 0..n {
                assert!((r[k] - want[k]).abs() < 1e-4, "size {n} round trip at {k}");
            }
        }
    }

    /// Audio-thread cost. This circuit runs per sample inside the host's
    /// callback, and the AU has already been reported by Logic as
    /// destabilising the system on CPU. Run by hand:
    ///   cargo test --release perf_spectral -- --ignored --nocapture
    #[test]
    #[ignore]
    fn perf_spectral() {
        let sr = 48000.0;
        let mut s = Spectral::new(sr);
        let n = 48000 * 10;
        let t = std::time::Instant::now();
        let mut acc = 0.0f32;
        for k in 0..n {
            let m = (std::f32::consts::TAU * 220.0 * k as f32 / sr).sin() * 0.4;
            let c = (((k as f32 * 110.0 / sr) % 1.0) * 2.0 - 1.0) * 5.0;
            acc += s.process(m, c);
        }
        let secs = t.elapsed().as_secs_f64();
        println!(
            "spectral: {:.1}x realtime, {:.2}% of one core at 48 kHz (sink {acc})",
            (n as f64 / 48000.0) / secs,
            100.0 * secs / (n as f64 / 48000.0)
        );
    }
}
