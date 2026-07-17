// Moog transistor ladder filter, after Huovilainen (DAFx 2004) and
// Välimäki & Huovilainen (2006) — the model behind CSound's `moogladder`,
// cross-checked against the published constants:
//
//   - four one-pole stages with tanh differential-pair nonlinearities on
//     every stage input AND state (the base-emitter curves of US 3,475,623)
//   - 2x oversampled, with a half-sample-averaged feedback tap for phase
//     compensation in the resonance loop
//   - cutoff-tuning polynomial `fcr` and resonance-tuning polynomial `acr`,
//     so the knob frequency and the self-oscillation point stay correct
//     across the audio band
//   - authentic passband loss as resonance rises: the 904A transfer function
//     is 1/((1+s/w)^4 + k), so bass genuinely thins out at high resonance.
//     Only partial make-up gain is applied — the thinning is part of the
//     Moog sound, not a defect to engineer away.
//
// The reference model runs at 16-bit signal scale with thermal = 0.000025
// (2*Vt in those units). The equations are exactly scale-invariant under
// (signal /= S, thermal *= S), so we run at +/-1.0 signal scale with an
// equivalent THERMAL = 0.4 — placing full-scale program material well into
// the differential-pair curvature, like a ladder driven at healthy level.

use std::f32::consts::TAU;

use crate::adaa::AdaaTanh;

const THERMAL: f32 = 0.4;

pub struct LadderFilter {
    sample_rate: f32,
    target_cutoff: f32,
    cutoff: f32, // smoothed
    target_resonance: f32,
    resonance: f32, // smoothed, 0..4 (4 = self-oscillation)
    drive: f32,
    saturation: f32,
    stage: [f32; 4],
    stage_tanh: [f32; 3],
    // delay[0..=3]: stage states; delay[4]: previous output;
    // delay[5]: half-sample-averaged feedback tap
    delay: [f32; 6],
    mismatch: [f32; 4],
    thermal_drift: f32,
    rng: u32,
    sat_adaa: AdaaTanh,
}

#[inline]
fn xorshift(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

#[inline]
fn rand01(state: &mut u32) -> f32 {
    (xorshift(state) >> 8) as f32 / (1u32 << 24) as f32
}

// Fast tanh approximation, accurate in the audio range and clamped where the
// rational form would diverge.
#[inline]
fn fast_tanh(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

impl LadderFilter {
    pub fn new(sample_rate: f32, seed: u32) -> Self {
        let mut rng = seed.wrapping_mul(0x85EB_CA6B) | 1;
        let mut mismatch = [1.0f32; 4];
        for m in &mut mismatch {
            // Subtle per-stage component tolerance, within 0.4%
            *m = 1.0 + (rand01(&mut rng) - 0.5) * 0.004;
        }
        Self {
            sample_rate,
            target_cutoff: 15000.0,
            cutoff: 15000.0,
            target_resonance: 0.0,
            resonance: 0.0,
            drive: 1.0,
            saturation: 1.0,
            stage: [0.0; 4],
            stage_tanh: [0.0; 3],
            delay: [0.0; 6],
            mismatch,
            thermal_drift: 0.0,
            rng,
            sat_adaa: AdaaTanh::new(),
        }
    }

    pub fn set_cutoff(&mut self, cutoff: f32) {
        self.target_cutoff = cutoff.clamp(16.0, self.sample_rate * 0.45);
    }

    pub fn set_resonance(&mut self, resonance: f32) {
        self.target_resonance = resonance.clamp(0.0, 4.0);
    }

    pub fn set_drive(&mut self, drive: f32) {
        self.drive = drive.clamp(0.1, 10.0);
    }

    pub fn set_saturation(&mut self, saturation: f32) {
        self.saturation = saturation.clamp(0.0, 2.0);
    }

    /// Process one sample. `cutoff_mult` is a per-sample modulation multiplier
    /// on top of the (smoothed) base cutoff — filter envelope, key tracking,
    /// and velocity all arrive through it.
    pub fn process(&mut self, input: f32, cutoff_mult: f32) -> f32 {
        // Slow thermal drift, bounded random walk
        self.thermal_drift =
            (self.thermal_drift + (rand01(&mut self.rng) - 0.5) * 1e-4) * 0.9995;

        // ~4 ms parameter slew removes zipper noise from stepped automation
        self.cutoff += (self.target_cutoff - self.cutoff) * 0.006;
        self.resonance += (self.target_resonance - self.resonance) * 0.006;

        let fc_hz = (self.cutoff * cutoff_mult * (1.0 + self.thermal_drift))
            .clamp(16.0, self.sample_rate * 0.49);
        let fc = fc_hz / self.sample_rate;
        let fc2 = fc * fc;
        let fc3 = fc2 * fc;

        // Empirical tuning corrections (Välimäki & Huovilainen 2006)
        let fcr = 1.8730 * fc3 + 0.4955 * fc2 - 0.6490 * fc + 0.9988;
        let acr = -3.9364 * fc2 + 1.8409 * fc + 0.9968;

        // One-pole coefficient per 2x-oversampled step, in thermal units
        let f = fc * 0.5;
        let tune = (1.0 - (-TAU * f * fcr).exp()) / THERMAL;
        // Resonance knob is already the classic k in 0..4
        let res_quad = self.resonance * acr;

        let x_in = input * self.drive;

        for _ in 0..2 {
            let mut inp = x_in - res_quad * self.delay[5];
            self.stage[0] = self.delay[0]
                + tune * self.mismatch[0] * (fast_tanh(inp * THERMAL) - self.stage_tanh[0]);
            self.delay[0] = self.stage[0];
            for k in 1..4 {
                inp = self.stage[k - 1];
                self.stage_tanh[k - 1] = fast_tanh(inp * THERMAL);
                let stage_out = if k != 3 {
                    self.stage_tanh[k]
                } else {
                    fast_tanh(self.delay[k] * THERMAL)
                };
                self.stage[k] = self.delay[k]
                    + tune * self.mismatch[k] * (self.stage_tanh[k - 1] - stage_out);
                self.delay[k] = self.stage[k];
            }
            // Half-sample delay for phase compensation in the feedback loop
            self.delay[5] = (self.stage[3] + self.delay[4]) * 0.5;
            self.delay[4] = self.stage[3];
        }

        let mut out = self.delay[5];

        // Partial make-up for the authentic 1/(1+k) passband loss — keep most
        // of the thinning, just soften how hard high resonance ducks the level
        out *= 1.0 + self.resonance * 0.3;
        // Drive make-up gain so the knob adds grit, not just volume
        out /= self.drive.sqrt().max(0.5);

        // Output saturation stage: transparent at 0, tape-ish squash at 2.
        // Antiderivative-antialiased tanh (Paschou et al. 2017) keeps the
        // added harmonics from folding back into the audio band.
        if self.saturation > 0.02 {
            out = self.sat_adaa.process(out * self.saturation) / self.saturation;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// US 3,475,623: at k = 4 the two rightmost poles reach the imaginary
    /// axis and the filter oscillates. Ping it and it must keep ringing.
    #[test]
    fn self_oscillates_at_max_resonance() {
        let sr = 44100.0;
        let mut filter = LadderFilter::new(sr, 7);
        filter.set_cutoff(1500.0);
        filter.set_resonance(4.0);

        // Let parameter smoothing settle, then ping
        for _ in 0..8000 {
            filter.process(0.0, 1.0);
        }
        filter.process(0.5, 1.0);

        let mut tail = 0.0f32;
        for i in 0..44100 {
            let y = filter.process(0.0, 1.0);
            assert!(y.is_finite());
            if i > 44100 - 4410 {
                tail = tail.max(y.abs());
            }
        }
        assert!(
            tail > 0.01,
            "filter should self-oscillate at resonance 4, tail peak = {tail}"
        );
    }

    /// At zero resonance the passband gain must be ~unity (the stage
    /// equilibrium of the thermal-scaled model reproduces the input).
    #[test]
    fn passband_is_transparent_at_zero_resonance() {
        let sr = 44100.0;
        let mut filter = LadderFilter::new(sr, 3);
        filter.set_cutoff(12000.0);
        filter.set_resonance(0.0);
        filter.set_saturation(0.0);

        // 220 Hz sine, well below cutoff
        let freq = 220.0;
        let mut peak_in = 0.0f32;
        let mut peak_out = 0.0f32;
        for n in 0..44100 {
            let x = 0.5 * (TAU * freq * n as f32 / sr).sin();
            let y = filter.process(x, 1.0);
            if n > 22050 {
                peak_in = peak_in.max(x.abs());
                peak_out = peak_out.max(y.abs());
            }
        }
        let gain = peak_out / peak_in;
        assert!(
            (0.7..=1.3).contains(&gain),
            "passband gain should be near unity, got {gain}"
        );
    }

    /// The defining behavior of the ladder topology: rising resonance thins
    /// the passband (1/((1+s/w)^4 + k) at s→0 is 1/(1+k)).
    #[test]
    fn resonance_thins_the_passband() {
        let sr = 44100.0;
        let measure = |resonance: f32| -> f32 {
            let mut filter = LadderFilter::new(sr, 11);
            filter.set_cutoff(8000.0);
            filter.set_resonance(resonance);
            filter.set_saturation(0.0);
            let mut peak = 0.0f32;
            for n in 0..44100 {
                let x = 0.25 * (TAU * 110.0 * n as f32 / sr).sin();
                let y = filter.process(x, 1.0);
                if n > 22050 {
                    peak = peak.max(y.abs());
                }
            }
            peak
        };
        let quiet = measure(3.5);
        let loud = measure(0.0);
        assert!(
            quiet < loud * 0.75,
            "high resonance should thin the passband: res0={loud}, res3.5={quiet}"
        );
    }
}
