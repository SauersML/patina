// 904B-style voltage-controlled high-pass ladder (US 3,475,623 covers both
// the low-pass and its "electrical brother"). Four cascaded one-pole
// high-pass sections give the same 24 dB/octave slope as the low-pass
// ladder; per the AES 1965 paper the production 904B ran without a
// resonance feedback path, so none is modeled. In series with the low-pass
// this recreates the 904C filter-coupler band-pass trick.

use std::f32::consts::PI;

/// Panel slew on the cutoff control, as a TIME so stepped automation
/// ramps identically at every host rate.
const PARAM_SLEW_TAU_S: f32 = 0.004;

pub struct HighPassLadder {
    sample_rate: f32,
    target_cutoff: f32,
    cutoff: f32, // smoothed
    /// Slew coefficient for `PARAM_SLEW_TAU_S` at this rate.
    slew_k: f32,
    s: [f32; 4],
}

impl HighPassLadder {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            target_cutoff: 16.0,
            cutoff: 16.0,
            slew_k: crate::voice::smoothing_coef(PARAM_SLEW_TAU_S, sample_rate),
            s: [0.0; 4],
        }
    }

    /// At the 16 Hz minimum the filter is effectively transparent.
    ///
    /// The ceiling is the panel's 8 kHz OR what the sample rate can
    /// carry, whichever is lower. `process` takes `tan(PI * fc / sr)`,
    /// which passes through infinity and turns NEGATIVE once fc reaches
    /// Nyquist; the integrator coefficient `g / (1 + g)` then exceeds 2
    /// and the ladder diverges. The host chooses the rate, so a ceiling
    /// written in absolute Hz is not a ceiling at all. (Measured: the
    /// 909 hat bank, whose HPF sits at 5.2 kHz, went non-finite within
    /// 435 samples at an 8 kHz host rate.)
    pub fn set_cutoff(&mut self, cutoff: f32) {
        let ceiling = (0.45 * self.sample_rate).clamp(16.0, 8000.0);
        self.target_cutoff = cutoff.clamp(16.0, ceiling);
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        self.cutoff += (self.target_cutoff - self.cutoff) * self.slew_k;
        // Trapezoidal (zero-delay) one-pole integrators, so the passband
        // stays flat instead of sagging like an explicit discretization
        let g = (PI * self.cutoff / self.sample_rate).tan();
        let a = g / (1.0 + g);

        let mut x = input;
        for state in &mut self.s {
            let v = (x - *state) * a;
            let lp = v + *state;
            *state = lp + v;
            x -= lp;
        }
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    #[test]
    fn passes_highs_blocks_lows() {
        let sr = 44100.0;
        let measure = |freq: f32| -> f32 {
            let mut hpf = HighPassLadder::new(sr);
            hpf.set_cutoff(500.0);
            let mut peak = 0.0f32;
            for n in 0..44100 {
                let x = (TAU * freq * n as f32 / sr).sin();
                let y = hpf.process(x);
                if n > 22050 {
                    peak = peak.max(y.abs());
                }
            }
            peak
        };
        let low = measure(50.0);
        let high = measure(5000.0);
        // 24 dB/oct: 50 Hz is > 3 octaves below 500 Hz -> heavily attenuated
        assert!(low < 0.05, "50 Hz should be crushed, got {low}");
        assert!(high > 0.9, "5 kHz should pass, got {high}");
    }

    #[test]
    fn transparent_at_minimum_cutoff() {
        let sr = 44100.0;
        let mut hpf = HighPassLadder::new(sr);
        let mut peak = 0.0f32;
        for n in 0..44100 {
            let x = (TAU * 110.0 * n as f32 / sr).sin();
            let y = hpf.process(x);
            if n > 22050 {
                peak = peak.max(y.abs());
            }
        }
        assert!(peak > 0.9, "110 Hz should pass at 16 Hz cutoff, got {peak}");
    }

    /// The cutoff slew is a TIME, not a per-sample number: a hardcoded
    /// coefficient made automated sweeps arrive twice as fast at 96 kHz.
    #[test]
    fn the_cutoff_slew_takes_the_same_time_at_every_rate() {
        let settle_ms = |sr: f32| -> f32 {
            let mut h = HighPassLadder::new(sr);
            h.set_cutoff(100.0);
            for _ in 0..(sr as usize) {
                h.process(0.0);
            }
            h.set_cutoff(4000.0);
            let target = 100.0 + 0.632 * (4000.0 - 100.0);
            let mut n = 0usize;
            while h.cutoff < target {
                h.process(0.0);
                n += 1;
                assert!(n < sr as usize, "the cutoff slew never arrived");
            }
            n as f32 / sr * 1000.0
        };
        let a = settle_ms(44100.0);
        let b = settle_ms(96000.0);
        let want = PARAM_SLEW_TAU_S * 1000.0;
        assert!(
            (a / want - 1.0).abs() < 0.05 && (a / b - 1.0).abs() < 0.05,
            "hpf cutoff slew took {a:.2} ms at 44.1 kHz and {b:.2} ms at \
             96 kHz, expected {want:.2} ms at both"
        );
    }
}
