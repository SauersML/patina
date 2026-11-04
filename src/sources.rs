// Two more sound sources on every voice card, for walls and risers.
//
// SUPERSAW: seven bandlimited sawtooths around oscillator 1's pitch — the
// trance-wall oscillator. `supersaw` crossfades oscillator 1's own
// waveform into the bank (0 = the bank is off and costs nothing),
// `supersaw_detune` spreads the six side saws, `supersaw_mix` balances
// them against the centre saw. The bank rides oscillator 1's exact
// frequency every sample, so glide, vibrato, FM, the pitch envelope, a
// note bend and the card's drift all move all seven saws together. The
// side saws sit at uneven offsets (the outer pairs wider than the inner
// ones) so no two pairs beat at the same rate, and every saw free-runs
// from its own start phase, so each note's wall is a little different —
// unless `key_sync` restarts them with the card's oscillators.
//
// NOISE OSCILLATOR: a white-noise generator per card with its own
// resonant state-variable filter (lowpass, bandpass or highpass), mixed
// in ahead of the drive and the ladder. Unlike the shared `noise` bus
// (one transistor source for every voice), each card's noise is its own,
// so a unison stack's noise is wide, and a track can sweep its noise
// filter (`automate riser.noise_cutoff 200 9000:16@exp`) without
// touching anyone else's.

use crate::oscillator::PROGRAM_V;

/// Relative frequency offsets of the seven saws at full detune. Uneven on
/// purpose: equal spacing makes the pairs beat in lockstep.
const OFFSETS: [f32; 7] = [-0.110, -0.063, -0.0195, 0.0, 0.0199, 0.0622, 0.1075];

/// The detune knob's taper: fine control over the first half (tens of
/// cents, where most walls live), a steep top end to the full spread.
#[inline]
fn detune_taper(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    0.2 * x + 0.8 * x * x * x
}

/// The mixer gain of one oscillator, in volts at the ladder's summing
/// junction (`voice::MIXER_GAIN` times program level).
const SOURCE_V: f32 = 0.45 * PROGRAM_V;

pub struct SuperSaw {
    phase: [f64; 7],
    /// Where `key_sync` restarts the saws: this card's own spread of
    /// phases, fixed for the life of the board.
    home: [f64; 7],
}

impl SuperSaw {
    pub fn new(seed: u32) -> Self {
        let mut rng = crate::rng::seed(seed.wrapping_mul(0x5851_F42D).wrapping_add(7));
        let mut home = [0.0f64; 7];
        for h in &mut home {
            *h = crate::rng::unipolar(&mut rng) as f64;
        }
        Self { phase: home, home }
    }

    /// Restart every saw at this card's home phases.
    pub fn sync(&mut self) {
        self.phase = self.home;
    }

    #[inline]
    fn blep(t: f32, dt: f32) -> f32 {
        if t < dt {
            let t = t / dt;
            2.0 * t - t * t - 1.0
        } else if t > 1.0 - dt {
            let t = (t - 1.0) / dt;
            t * t + 2.0 * t + 1.0
        } else {
            0.0
        }
    }

    /// One sample of the bank around `hz`, in program volts (one saw's
    /// level: the mix law holds the bank's loudness steady as the side
    /// saws come up).
    #[inline]
    pub fn render(&mut self, hz: f32, sample_rate: f32, detune: f32, mix: f32) -> f32 {
        let d = detune_taper(detune);
        let mix = mix.clamp(0.0, 1.0);
        let centre = 1.0 - 0.4 * mix;
        let side = mix;
        let norm = 1.0 / (centre * centre + 6.0 * side * side).sqrt();
        let mut out = 0.0f32;
        for (k, off) in OFFSETS.iter().enumerate() {
            let f = (hz * (1.0 + off * d)).max(0.0);
            let dt = (f / sample_rate).min(0.5);
            let mut p = self.phase[k] + dt as f64;
            p -= p.floor();
            self.phase[k] = p;
            let t = p as f32;
            let saw = 2.0 * t - 1.0 - Self::blep(t, dt.max(1e-9));
            out += saw * if k == 3 { centre } else { side };
        }
        out * norm * PROGRAM_V
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoiseMode {
    Lowpass,
    Bandpass,
    Highpass,
}

impl NoiseMode {
    pub fn from_value(v: f32) -> Self {
        match v.round() as i32 {
            i32::MIN..=0 => NoiseMode::Lowpass,
            1 => NoiseMode::Bandpass,
            _ => NoiseMode::Highpass,
        }
    }
}

pub struct NoiseOsc {
    rng: u32,
    level: f32,
    target_cutoff: f32,
    /// Smoothed cutoff (Hz) and the coefficient it was last computed at.
    cutoff: f32,
    coef_at: f32,
    res: f32,
    mode: NoiseMode,
    g: f32,
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: f32,
    ic2: f32,
    smooth_k: f32,
    sample_rate: f32,
}

impl NoiseOsc {
    pub fn new(sample_rate: f32, seed: u32) -> Self {
        let mut n = Self {
            rng: crate::rng::seed(seed.wrapping_mul(0x2545_F491).wrapping_add(3)),
            level: 0.0,
            target_cutoff: 20000.0,
            cutoff: 20000.0,
            coef_at: -1.0,
            res: 0.0,
            mode: NoiseMode::Lowpass,
            g: 0.0,
            k: 2.0,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            ic1: 0.0,
            ic2: 0.0,
            smooth_k: crate::smoothing::approach(0.004, sample_rate),
            sample_rate,
        };
        n.update_coefs();
        n
    }

    pub fn set_level(&mut self, v: f32) {
        self.level = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    }

    pub fn set_cutoff(&mut self, hz: f32) {
        if hz.is_finite() {
            self.target_cutoff = hz.clamp(20.0, 20000.0);
        }
    }

    pub fn set_res(&mut self, r: f32) {
        if r.is_finite() && r != self.res {
            self.res = r.clamp(0.0, 1.0);
            self.coef_at = -1.0;
        }
    }

    pub fn set_mode(&mut self, mode: NoiseMode) {
        self.mode = mode;
    }

    #[inline]
    pub fn active(&self) -> bool {
        self.level > 0.0
    }

    fn update_coefs(&mut self) {
        let fc = self.cutoff.min(0.49 * self.sample_rate);
        self.g = (std::f32::consts::PI * fc / self.sample_rate).tan();
        // res 0 = Q 0.5 (no peak); res 1 = Q ~16, a whistling band
        self.k = 2.0 * (1.0 - 0.97 * self.res);
        self.a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
        self.a2 = self.g * self.a1;
        self.a3 = self.g * self.a2;
        self.coef_at = self.cutoff;
    }

    /// One sample of filtered noise, in volts at the ladder's input.
    #[inline]
    pub fn render(&mut self) -> f32 {
        // log-domain slew so an exponential sweep stays exponential
        if self.cutoff != self.target_cutoff {
            let l = self.cutoff.ln() + (self.target_cutoff.ln() - self.cutoff.ln()) * self.smooth_k;
            self.cutoff = l.exp();
            if (self.cutoff / self.target_cutoff - 1.0).abs() < 1e-4 {
                self.cutoff = self.target_cutoff;
            }
        }
        if self.cutoff != self.coef_at {
            self.update_coefs();
        }
        let v0 = crate::rng::bipolar(&mut self.rng) * SOURCE_V;
        // Simper's trapezoidal SVF: stable under any sweep speed
        let v3 = v0 - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        let y = match self.mode {
            NoiseMode::Lowpass => v2,
            // unity at the centre whatever the resonance
            NoiseMode::Bandpass => self.k * v1,
            NoiseMode::Highpass => v0 - self.k * v1 - v2,
        };
        y * self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Magnitude spectrum by direct DFT at the given frequencies.
    fn dft(x: &[f32], sr: f32, freqs: &[f32]) -> Vec<f32> {
        freqs
            .iter()
            .map(|&f| {
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (i, &v) in x.iter().enumerate() {
                    let ph = std::f64::consts::TAU * f as f64 * i as f64 / sr as f64;
                    re += v as f64 * ph.cos();
                    im += v as f64 * ph.sin();
                }
                ((re * re + im * im).sqrt() / x.len() as f64) as f32
            })
            .collect()
    }

    /// The bank's fundamental region holds seven distinct partials at the
    /// detuned frequencies, not one.
    #[test]
    fn seven_saws_at_seven_frequencies() {
        let sr = 48000.0;
        let mut s = SuperSaw::new(1);
        let hz = 220.0;
        let x: Vec<f32> = (0..(4.0 * sr) as usize)
            .map(|_| s.render(hz, sr, 1.0, 1.0))
            .collect();
        let d = detune_taper(1.0);
        let at: Vec<f32> = OFFSETS.iter().map(|o| hz * (1.0 + o * d)).collect();
        let peaks = dft(&x, sr, &at);
        // between two partials there should be far less energy
        let gaps: Vec<f32> = at.windows(2).map(|w| 0.5 * (w[0] + w[1])).collect();
        let valleys = dft(&x, sr, &gaps);
        let floor = valleys.iter().fold(0.0f32, |a, &v| a.max(v));
        for (f, p) in at.iter().zip(&peaks) {
            assert!(*p > 8.0 * floor, "partial at {f:.1} Hz: {p} vs valley {floor}");
        }
    }

    #[test]
    fn the_mix_law_holds_the_level() {
        let sr = 48000.0;
        let rms = |mix: f32| {
            let mut s = SuperSaw::new(2);
            let x: Vec<f32> = (0..48000).map(|_| s.render(110.0, sr, 0.5, mix)).collect();
            (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
        };
        let (a, b) = (rms(0.0), rms(1.0));
        assert!((a / b - 1.0).abs() < 0.15, "mix 0 {a} vs mix 1 {b}");
    }

    #[test]
    fn the_noise_filter_sets_the_band() {
        let sr = 48000.0;
        let band = |mode: NoiseMode, fc: f32| {
            let mut n = NoiseOsc::new(sr, 9);
            n.set_level(1.0);
            n.set_mode(mode);
            n.set_cutoff(fc);
            n.set_res(0.5);
            for _ in 0..4800 {
                n.render();
            }
            let x: Vec<f32> = (0..48000).map(|_| n.render()).collect();
            let lo = dft(&x, sr, &[200.0, 230.0, 260.0]).iter().sum::<f32>();
            let hi = dft(&x, sr, &[8000.0, 8300.0, 8600.0]).iter().sum::<f32>();
            (lo, hi)
        };
        let (lo, hi) = band(NoiseMode::Lowpass, 500.0);
        assert!(lo > 10.0 * hi, "lowpass at 500 Hz: low {lo} high {hi}");
        let (lo, hi) = band(NoiseMode::Highpass, 4000.0);
        assert!(hi > 10.0 * lo, "highpass at 4 kHz: low {lo} high {hi}");
    }
}
