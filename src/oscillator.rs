// Oscillator modeled on the 901B / ARP 2600 architecture: ONE physical core
// (a bandlimited sawtooth ramp) with every other waveform DERIVED from it,
// the way the converter circuits do it — not four unrelated ideal generators.
//
//   ramp core ── sawtooth output
//       ├─ fold ──────────── triangle (slight asymmetry, like the trim
//       │                    adjustments the service manual provides)
//       ├─ fold + rounding ─ sine ("Q3's nonlinearity rounds the peaks of
//       │                    the triangle to approximate a sine wave")
//       └─ comparator ────── pulse (per-unit duty error inside the 901B's
//                            48–52% service window)
//
// Output amplitudes follow the 901B alignment targets (saw 0.50 Vac,
// sine 0.50, triangle 0.65, pulse 1.2) converted to peak ratios rather than
// normalizing every wave to the same level — switching waveforms changes
// how hard the filter is driven, exactly as on the hardware.

use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Waveform {
    Sine,
    Square,
    Sawtooth,
    Triangle,
}

// Peak-amplitude ratios from the 901B service targets, saw = 1.0
const AMP_SAW: f32 = 1.0;
const AMP_SINE: f32 = 0.82;
const AMP_TRI: f32 = 1.3;
const AMP_PULSE: f32 = 1.38;

pub struct Oscillator {
    phase: f64,
    frequency: AtomicU32,
    /// Fixed frequency ratio for unison detune (2^(cents/1200)).
    freq_mult: f32,
    sample_rate: f32,
    waveform: Waveform,
    drift: f32,
    rng: u32,
    /// Effective comparator threshold this sample (base width + unit error).
    duty: f32,
    /// This unit's fixed comparator error: duty sits near, not at, center.
    duty_error: f32,
    /// Waveshaper asymmetry for the triangle fold / sine rounding.
    skew: f32,
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

impl Oscillator {
    pub fn new(sample_rate: f32, frequency: f32, seed: u32) -> Self {
        let mut rng = seed.wrapping_mul(0x9E37_79B9) | 1;
        // Random start phase so unison oscillators never phase-lock
        let phase = rand01(&mut rng) as f64;
        // Component-tolerance constants, fixed for the life of the "board"
        let duty_error = (rand01(&mut rng) - 0.5) * 0.02; // within +/-1%
        let skew = (rand01(&mut rng) - 0.5) * 0.06;
        Self {
            phase,
            frequency: AtomicU32::new(frequency.to_bits()),
            freq_mult: 1.0,
            sample_rate,
            waveform: Waveform::Sawtooth,
            drift: 0.0,
            rng,
            duty: 0.5 + duty_error,
            duty_error,
            skew,
        }
    }

    /// `common_drift` is the voice-shared component (the oscillators sit on
    /// one controller card and supply); each core adds its own smaller
    /// residual walk on top, so a bank beats slowly rather than wobbling.
    /// `pitch_mult` is the global modulation ratio (vibrato applied in CV
    /// space, so it is exponential — a frequency ratio, not added hertz).
    /// `pulse_width` is the comparator threshold; this unit's duty error
    /// rides on top of it.
    pub fn next_sample(&mut self, common_drift: f32, pitch_mult: f32, pulse_width: f32) -> f32 {
        let frequency = f32::from_bits(self.frequency.load(Ordering::Relaxed));

        // Small individual drift; the larger, shared component comes in from
        // the voice so all three oscillators move together
        self.drift = (self.drift + (rand01(&mut self.rng) - 0.5) * 1.2e-5) * 0.9995;
        let detuned_frequency =
            frequency * self.freq_mult * (1.0 + self.drift + common_drift) * pitch_mult;
        self.duty = (pulse_width + self.duty_error).clamp(0.03, 0.97);

        self.phase += detuned_frequency as f64 / self.sample_rate as f64;
        self.phase %= 1.0;

        let t = self.phase as f32;
        let raw_sample = match self.waveform {
            Waveform::Sawtooth => AMP_SAW * self.polyblep_saw(t, detuned_frequency),
            Waveform::Square => AMP_PULSE * self.polyblep_pulse(t, detuned_frequency),
            Waveform::Triangle => {
                AMP_TRI * self.fold_triangle(self.polyblep_triangle(t, detuned_frequency))
            }
            Waveform::Sine => {
                // Triangle through the transistor peak-rounding curve
                let tri = self.fold_triangle(self.polyblep_triangle(t, detuned_frequency));
                AMP_SINE * Self::transistor_sine(tri)
            }
        };

        self.soft_clip(raw_sample)
    }

    /// Slightly asymmetric triangle, as if the fold network's two halves
    /// aren't perfectly matched. Endpoints stay at +/-1.
    #[inline]
    fn fold_triangle(&self, tri: f32) -> f32 {
        tri + self.skew * (1.0 - tri * tri)
    }

    /// The sine shaper: a soft cubic that rounds the triangle's peaks.
    /// Not sin() — it keeps a few percent of low-order harmonics, which is
    /// exactly what the service manual's "sine purity" trim is adjusting.
    #[inline]
    fn transistor_sine(tri: f32) -> f32 {
        let x = tri.clamp(-1.0, 1.0);
        x * (1.5 - 0.5 * x * x)
    }

    fn polyblep(&self, t: f32, dt: f32) -> f32 {
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

    fn polyblep_pulse(&self, t: f32, frequency: f32) -> f32 {
        let dt = frequency / self.sample_rate;
        let naive = if t < self.duty { 1.0 } else { -1.0 };
        naive - self.polyblep(t, dt) + self.polyblep((t + 1.0 - self.duty) % 1.0, dt)
    }

    fn polyblep_saw(&self, t: f32, frequency: f32) -> f32 {
        let dt = frequency / self.sample_rate;
        let naive = 2.0 * t - 1.0;
        naive - self.polyblep(t, dt)
    }

    fn polyblep_triangle(&self, t: f32, frequency: f32) -> f32 {
        let dt = frequency / self.sample_rate;
        let naive = if t < 0.5 {
            4.0 * t - 1.0
        } else {
            3.0 - 4.0 * t
        };
        naive - self.integrate_polyblep(t, dt) + self.integrate_polyblep((t + 0.5) % 1.0, dt)
    }

    fn integrate_polyblep(&self, t: f32, dt: f32) -> f32 {
        if t < dt {
            let t = t / dt;
            dt * (t * t * t / 3.0 - t * t / 2.0 - t + 1.0 / 3.0)
        } else if t > 1.0 - dt {
            let t = (t - 1.0) / dt;
            dt * (-t * t * t / 3.0 + t * t + t + 1.0 / 3.0)
        } else {
            0.0
        }
    }

    fn soft_clip(&self, x: f32) -> f32 {
        // Cubic soft clip, transparent below |x| ~ 1.5: leaves headroom for
        // the hot 901B pulse/triangle levels so the alignment ratios survive
        let x = x.clamp(-2.25, 2.25);
        x * (1.0 - x * x / 15.1875)
    }

    pub fn set_frequency(&self, frequency: f32) {
        self.frequency.store(frequency.to_bits(), Ordering::Relaxed);
    }

    pub fn set_freq_mult(&mut self, mult: f32) {
        self.freq_mult = mult;
    }

    pub fn set_waveform(&mut self, waveform: Waveform) {
        self.waveform = waveform;
    }

    pub fn note_to_frequency(note: u8) -> f32 {
        440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(waveform: Waveform) -> f32 {
        let mut osc = Oscillator::new(44100.0, 220.0, 42);
        osc.set_waveform(waveform);
        let mut peak = 0.0f32;
        for _ in 0..44100 {
            peak = peak.max(osc.next_sample(0.0, 1.0, 0.5).abs());
        }
        peak
    }

    /// 901B alignment: outputs are NOT normalized to one level. Triangle and
    /// pulse run hotter than saw; sine runs a touch quieter.
    #[test]
    fn service_target_amplitude_ratios() {
        let saw = peak(Waveform::Sawtooth);
        let tri = peak(Waveform::Triangle);
        let sine = peak(Waveform::Sine);
        let pulse = peak(Waveform::Square);
        assert!(tri > saw * 1.1, "triangle should run hot: {tri} vs saw {saw}");
        assert!(pulse > saw * 1.2, "pulse should run hot: {pulse} vs saw {saw}");
        assert!(sine < saw, "sine should be slightly quieter: {sine} vs {saw}");
    }

    /// The derived sine is a rounded triangle, not sin(): it must carry a
    /// few percent of harmonic residue.
    #[test]
    fn sine_is_imperfect() {
        let sr = 44100.0;
        let freq = 441.0; // exactly 100 samples per period
        let mut osc = Oscillator::new(sr, freq, 7);
        osc.set_waveform(Waveform::Sine);
        // Collect one steady period and correlate against a pure sinusoid
        let mut samples = Vec::new();
        for _ in 0..4410 {
            samples.push(osc.next_sample(0.0, 1.0, 0.5));
        }
        let period = &samples[4310..4410];
        // Fundamental amplitude via DFT bin
        let n = period.len() as f32;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (k, &s) in period.iter().enumerate() {
            let a = std::f32::consts::TAU * k as f32 / n;
            re += s * a.cos();
            im += s * a.sin();
        }
        let fundamental = 2.0 * (re * re + im * im).sqrt() / n;
        let total_rms = (period.iter().map(|s| s * s).sum::<f32>() / n).sqrt();
        let fund_rms = fundamental / std::f32::consts::SQRT_2;
        let residue = (total_rms * total_rms - fund_rms * fund_rms).max(0.0).sqrt();
        let thd = residue / fund_rms;
        assert!(
            thd > 0.005 && thd < 0.12,
            "transistor sine should have a few percent THD, got {thd}"
        );
    }

    /// PWM: the mean of a pulse wave is 2*duty - 1, so the width control
    /// must move the DC balance accordingly.
    #[test]
    fn pulse_width_shifts_duty() {
        let measure = |width: f32| -> f32 {
            let mut osc = Oscillator::new(44100.0, 441.0, 3);
            osc.set_waveform(Waveform::Square);
            let mut sum = 0.0f32;
            let n = 44100;
            for _ in 0..n {
                sum += osc.next_sample(0.0, 1.0, width);
            }
            sum / n as f32
        };
        let narrow = measure(0.2);
        let center = measure(0.5);
        let wide = measure(0.8);
        assert!(narrow < -0.5, "20% duty should sit negative, got {narrow}");
        assert!(center.abs() < 0.1, "50% duty near zero mean, got {center}");
        assert!(wide > 0.5, "80% duty should sit positive, got {wide}");
    }
}
