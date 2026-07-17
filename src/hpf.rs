// 904B-style voltage-controlled high-pass ladder (US 3,475,623 covers both
// the low-pass and its "electrical brother"). Four cascaded one-pole
// high-pass sections give the same 24 dB/octave slope as the low-pass
// ladder; per the AES 1965 paper the production 904B ran without a
// resonance feedback path, so none is modeled. In series with the low-pass
// this recreates the 904C filter-coupler band-pass trick.

use std::f32::consts::PI;

pub struct HighPassLadder {
    sample_rate: f32,
    target_cutoff: f32,
    cutoff: f32, // smoothed
    s: [f32; 4],
}

impl HighPassLadder {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            target_cutoff: 16.0,
            cutoff: 16.0,
            s: [0.0; 4],
        }
    }

    /// At the 16 Hz minimum the filter is effectively transparent.
    pub fn set_cutoff(&mut self, cutoff: f32) {
        self.target_cutoff = cutoff.clamp(16.0, 8000.0);
    }

    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        self.cutoff += (self.target_cutoff - self.cutoff) * 0.006;
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
}
