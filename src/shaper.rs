// The voice's drive stage: a waveshaper between the oscillator mixer and
// the ladder, so the filter after it is a POST-filter. This is the
// hardstyle/808 kick chain (a pitch-swept sine, distorted hard, then
// filtered) and the dirty-bass chain in general.
//
// Three transfer curves, all with first-order antiderivative antialiasing
// (Parker et al., DAFx-16 — the same treatment the ladder's tanh and the
// fuzz get, see adaa.rs), because a shaper 40 dB into its curve on a
// pitch envelope starting kilohertz up would otherwise fold its harmonics
// straight back into the band:
//
//   0 tanh  soft saturation, odd harmonics, the sine squares off
//   1 clip  hard clip at the reference level, the rawest square
//   2 fold  sine wavefolder: past the reference the wave folds back on
//           itself, spraying harmonics that move with the drive (the
//           rawstyle "screech")
//
// The input is in program volts; `SHAPER_REF_V` is where the curve bends.
// The first 3 dB of drive crossfade from the dry wire into the curve, so
// automation can bring the stage in from 0 dB without a level step.

/// Where the transfer curve bends, in volts: a single oscillator at the
/// mixer swings about +/-2 V, so 0 dB of drive already touches the knee.
pub const SHAPER_REF_V: f32 = 2.5;

/// Drive at which the stage is fully in (below it, dry and shaped blend).
const BLEND_DB: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShaperMode {
    Tanh,
    Clip,
    Fold,
}

impl ShaperMode {
    pub fn from_value(v: f32) -> Self {
        match v.round() as i32 {
            i32::MIN..=0 => ShaperMode::Tanh,
            1 => ShaperMode::Clip,
            _ => ShaperMode::Fold,
        }
    }

    #[inline]
    fn curve(self, u: f32) -> f32 {
        match self {
            ShaperMode::Tanh => u.tanh(),
            ShaperMode::Clip => u.clamp(-1.0, 1.0),
            ShaperMode::Fold => u.sin(),
        }
    }

    /// The antiderivative of `curve`.
    #[inline]
    fn integral(self, u: f32) -> f32 {
        match self {
            ShaperMode::Tanh => {
                // ln(cosh(u)), numerically stable
                let a = u.abs();
                a + (-2.0 * a).exp().ln_1p() - std::f32::consts::LN_2
            }
            ShaperMode::Clip => {
                let a = u.abs();
                if a <= 1.0 {
                    0.5 * u * u
                } else {
                    a - 0.5
                }
            }
            ShaperMode::Fold => -u.cos(),
        }
    }
}

pub struct Shaper {
    /// Target and smoothed drive in dB (0 = out).
    target_db: f32,
    db: f32,
    smooth_k: f32,
    mode: ShaperMode,
    /// Previous normalized input and its antiderivative.
    u1: f32,
    f1: f32,
    /// Whether the last sample went through the curve: a stage coming
    /// back in seeds its antialiasing state on the input it meets, so
    /// the first quotient spans one real step (see `AdaaTanh::seed`).
    was_active: bool,
}

impl Shaper {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            target_db: 0.0,
            db: 0.0,
            smooth_k: crate::smoothing::approach(0.005, sample_rate),
            mode: ShaperMode::Tanh,
            u1: 0.0,
            f1: 0.0,
            was_active: false,
        }
    }

    pub fn set_drive_db(&mut self, db: f32) {
        self.target_db = if db.is_finite() { db.clamp(0.0, 48.0) } else { 0.0 };
    }

    pub fn set_mode(&mut self, mode: ShaperMode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        // the stored antiderivative belongs to the old curve
        self.f1 = mode.integral(self.u1);
    }

    /// True while the stage colours the signal (or is still fading out).
    #[inline]
    pub fn active(&self) -> bool {
        self.target_db > 0.0 || self.db > 1e-4
    }

    /// One sample, volts in, volts out. Call only while `active()`; a
    /// bypassed stage costs nothing, and `bypassed` marks it so it comes
    /// back in without a step.
    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.db += (self.target_db - self.db) * self.smooth_k;
        if self.target_db == 0.0 && self.db < 1e-4 {
            self.db = 0.0;
        }
        let gain = (self.db * (std::f32::consts::LN_10 / 20.0)).exp();
        let u = x * gain / SHAPER_REF_V;
        let f0 = self.mode.integral(u);
        if !self.was_active {
            self.u1 = u;
            self.f1 = f0;
            self.was_active = true;
        }
        let du = u - self.u1;
        let shaped = if du.abs() < 1e-3 {
            self.mode.curve(0.5 * (u + self.u1))
        } else {
            (f0 - self.f1) / du
        };
        self.u1 = u;
        self.f1 = f0;
        let y = shaped * SHAPER_REF_V;
        let wet = (self.db / BLEND_DB).min(1.0);
        x + (y - x) * wet
    }

    /// The stage sat out this sample.
    #[inline]
    pub fn bypassed(&mut self) {
        self.was_active = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_through(mode: ShaperMode, db: f32, hz: f32, amp: f32) -> Vec<f32> {
        let sr = 48000.0;
        let mut s = Shaper::new(sr);
        s.set_mode(mode);
        s.set_drive_db(db);
        // settle the drive smoother
        for _ in 0..4800 {
            s.process(0.0);
        }
        (0..4800)
            .map(|i| s.process(amp * (std::f32::consts::TAU * hz * i as f32 / sr).sin()))
            .collect()
    }

    /// Harmonic `k`'s amplitude by single-bin DFT over whole periods.
    fn harmonic(x: &[f32], hz: f32, k: f32) -> f32 {
        let sr = 48000.0;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &v) in x.iter().enumerate() {
            let ph = std::f64::consts::TAU * (k * hz) as f64 * i as f64 / sr as f64;
            re += v as f64 * ph.cos();
            im += v as f64 * ph.sin();
        }
        (2.0 * (re * re + im * im).sqrt() / x.len() as f64) as f32
    }

    #[test]
    fn drive_squares_a_sine_and_bounds_it() {
        // 100 Hz: 4800 samples are exactly 10 periods
        let clean = sine_through(ShaperMode::Tanh, 0.0, 100.0, 1.85);
        let hot = sine_through(ShaperMode::Tanh, 30.0, 100.0, 1.85);
        let h3_clean = harmonic(&clean, 100.0, 3.0) / harmonic(&clean, 100.0, 1.0);
        let h3_hot = harmonic(&hot, 100.0, 3.0) / harmonic(&hot, 100.0, 1.0);
        assert!(h3_clean < 1e-3, "0 dB must be the dry wire: h3 {h3_clean}");
        // a square's third harmonic is 1/3 of its fundamental
        assert!(h3_hot > 0.25, "30 dB of tanh should square the sine: h3 {h3_hot}");
        let peak = hot.iter().fold(0.0f32, |a, &v| a.max(v.abs()));
        assert!(peak <= SHAPER_REF_V * 1.001, "tanh is bounded by its knee: {peak}");
    }

    #[test]
    fn every_curve_is_antialiased_against_its_naive_form() {
        // A 3.1 kHz sine driven 36 dB: the naive curve's high harmonics
        // fold back to non-harmonic frequencies; ADAA must leave less
        // energy off the harmonic series than the naive shaper does.
        for mode in [ShaperMode::Tanh, ShaperMode::Clip, ShaperMode::Fold] {
            let sr = 48000.0f32;
            let hz = 3100.0f32;
            let g = (36.0f32 / 20.0 * std::f32::consts::LN_10).exp();
            let n = 48000;
            let x: Vec<f32> = (0..n)
                .map(|i| 1.85 * (std::f32::consts::TAU * hz * i as f32 / sr).sin())
                .collect();
            let mut s = Shaper::new(sr);
            s.set_mode(mode);
            s.set_drive_db(36.0);
            s.db = 36.0;
            let adaa: Vec<f32> = x.iter().map(|&v| s.process(v)).collect();
            let naive: Vec<f32> = x
                .iter()
                .map(|&v| mode.curve(v * g / SHAPER_REF_V) * SHAPER_REF_V)
                .collect();
            // energy at a frequency that is NOT a harmonic of 3.1 kHz
            // (aliases of odd harmonics land at |k*3100 - m*48000|)
            let off = |y: &[f32]| -> f32 {
                let mut e = 0.0;
                for f in [700.0f32, 1300.0, 1900.0, 2500.0] {
                    let (mut re, mut im) = (0.0f64, 0.0f64);
                    for (i, &v) in y.iter().enumerate() {
                        let ph = std::f64::consts::TAU * f as f64 * i as f64 / sr as f64;
                        re += v as f64 * ph.cos();
                        im += v as f64 * ph.sin();
                    }
                    e += (re * re + im * im) as f32;
                }
                e
            };
            let (a, b) = (off(&adaa), off(&naive));
            assert!(a < b, "{mode:?}: ADAA {a} vs naive {b}");
        }
    }

    #[test]
    fn the_stage_fades_in_without_a_step() {
        let sr = 48000.0;
        let mut s = Shaper::new(sr);
        s.set_drive_db(0.0);
        let mut prev = 0.0f32;
        let mut worst = 0.0f32;
        for i in 0..9600 {
            if i == 2400 {
                s.set_drive_db(24.0);
            }
            let x = 1.5 * (std::f32::consts::TAU * 50.0 * i as f32 / sr).sin();
            let y = if s.active() {
                s.process(x)
            } else {
                s.bypassed();
                x
            };
            if i > 0 {
                worst = worst.max((y - prev).abs());
            }
            prev = y;
        }
        // a 50 Hz sine at 1.5 V moves ~0.01 V per sample; a step from the
        // dry wire into a 24 dB curve would jump by ~1 V
        assert!(worst < 0.2, "largest sample step {worst}");
    }
}
