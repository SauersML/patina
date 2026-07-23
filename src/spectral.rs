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

/// Radix-2 iterative FFT, in place. `inverse` includes the 1/N scale.
fn fft(re: &mut [f32], im: &mut [f32], inverse: bool) {
    let n = re.len();
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
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        let ang = sign * std::f32::consts::TAU / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0;
        while i < n {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (ar, ai) = (re[i + k], im[i + k]);
                let (br, bi) = (re[i + k + len / 2], im[i + k + len / 2]);
                let (tr, ti) = (br * cr - bi * ci, br * ci + bi * cr);
                re[i + k] = ar + tr;
                im[i + k] = ai + ti;
                re[i + k + len / 2] = ar - tr;
                im[i + k + len / 2] = ai - ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
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
        for k in 0..N {
            mr[k] = self.in_m[k] * self.window[k];
            cr[k] = self.in_c[k] * self.window[k];
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
        // Slide inputs left by one into the frame buffers
        self.in_m.copy_within(1.., 0);
        self.in_c.copy_within(1.., 0);
        self.in_m[N - 1] = modulator;
        self.in_c[N - 1] = carrier;

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
}
