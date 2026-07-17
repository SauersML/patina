// Moog-style transistor ladder filter, Huovilainen-flavored:
// four one-pole stages with tanh nonlinearities, run at 2x oversampling,
// with per-sample parameter smoothing (no zipper noise under automation),
// transistor mismatch, and slow thermal drift.

use std::f32::consts::TAU;

pub struct LadderFilter {
    sample_rate: f32,
    target_cutoff: f32,
    cutoff: f32, // smoothed
    target_resonance: f32,
    resonance: f32, // smoothed
    drive: f32,
    saturation: f32,
    s: [f32; 4],
    mismatch: [f32; 4],
    thermal_drift: f32,
    rng: u32,
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
            s: [0.0; 4],
            mismatch,
            thermal_drift: 0.0,
            rng,
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

        let fc = (self.cutoff * cutoff_mult * (1.0 + self.thermal_drift))
            .clamp(16.0, self.sample_rate * 0.45);
        // One-pole coefficient at 2x oversampling
        let g = 1.0 - (-TAU * fc / (self.sample_rate * 2.0)).exp();
        let res = self.resonance;
        let x_in = input * self.drive;

        let mut out = 0.0;
        for _ in 0..2 {
            // Input stage: drive plus resonance feedback from the last stage
            let x = fast_tanh(x_in - res * self.s[3]);
            self.s[0] += g * self.mismatch[0] * (x - fast_tanh(self.s[0]));
            self.s[1] += g * self.mismatch[1] * (fast_tanh(self.s[0]) - fast_tanh(self.s[1]));
            self.s[2] += g * self.mismatch[2] * (fast_tanh(self.s[1]) - fast_tanh(self.s[2]));
            self.s[3] += g * self.mismatch[3] * (fast_tanh(self.s[2]) - fast_tanh(self.s[3]));
            out = self.s[3];
        }

        // Compensate the passband loss that rising resonance causes
        out *= 1.0 + res * 0.4;
        // Drive make-up gain so the knob adds grit, not just volume
        out /= self.drive.sqrt().max(0.5);

        // Output saturation stage: transparent at 0, tape-ish squash at 2
        if self.saturation > 0.02 {
            out = fast_tanh(out * self.saturation) / self.saturation;
        }
        out
    }
}
