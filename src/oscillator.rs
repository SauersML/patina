use std::sync::atomic::{AtomicU32, Ordering};
use std::f32::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Waveform {
    Sine,
    Square,
    Sawtooth,
    Triangle,
}

pub struct Oscillator {
    phase: f64,
    frequency: AtomicU32,
    /// Fixed frequency ratio for unison detune (2^(cents/1200)).
    freq_mult: f32,
    sample_rate: f32,
    waveform: Waveform,
    drift: f32,
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

impl Oscillator {
    pub fn new(sample_rate: f32, frequency: f32, seed: u32) -> Self {
        let mut rng = seed.wrapping_mul(0x9E37_79B9) | 1;
        // Random start phase so unison oscillators never phase-lock
        let phase = rand01(&mut rng) as f64;
        Self {
            phase,
            frequency: AtomicU32::new(frequency.to_bits()),
            freq_mult: 1.0,
            sample_rate,
            waveform: Waveform::Sawtooth,
            drift: 0.0,
            rng,
        }
    }

    pub fn next_sample(&mut self) -> f32 {
        let frequency = f32::from_bits(self.frequency.load(Ordering::Relaxed));

        // Slow bounded random walk models analog pitch drift; fresh per-sample
        // noise here would act as FM noise and add audible hiss
        self.drift = (self.drift + (rand01(&mut self.rng) - 0.5) * 2e-5) * 0.9995;
        let detuned_frequency = frequency * self.freq_mult * (1.0 + self.drift);

        self.phase += detuned_frequency as f64 / self.sample_rate as f64;
        self.phase %= 1.0;

        let raw_sample = match self.waveform {
            Waveform::Sine => (self.phase as f32 * 2.0 * PI).sin(),
            Waveform::Square => self.polyblep_square(self.phase as f32, detuned_frequency),
            Waveform::Sawtooth => self.polyblep_saw(self.phase as f32, detuned_frequency),
            Waveform::Triangle => self.polyblep_triangle(self.phase as f32, detuned_frequency),
        };

        self.soft_clip(raw_sample)
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

    fn polyblep_square(&self, t: f32, frequency: f32) -> f32 {
        let dt = frequency / self.sample_rate;
        let naive = if t < 0.5 { 1.0 } else { -1.0 };
        naive - self.polyblep(t, dt) + self.polyblep((t + 0.5) % 1.0, dt)
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
        // Cubic soft clip, transparent below |x| ~ 1
        let x = x.clamp(-1.5, 1.5);
        x * (1.0 - x * x / 6.75)
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
