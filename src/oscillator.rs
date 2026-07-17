use std::sync::atomic::{AtomicU32, Ordering};
use std::f32::consts::PI;
use rand;

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
    sample_rate: f32,
    volume: AtomicU32,
    waveform: Waveform,
    drift: f32,
}

impl Oscillator {
    pub fn new(sample_rate: f32, frequency: f32) -> Self {
        Self {
            phase: 0.0,
            frequency: AtomicU32::new(frequency.to_bits()),
            sample_rate,
            volume: AtomicU32::new(1.0f32.to_bits()),
            waveform: Waveform::Sawtooth,
            drift: 0.0,
        }
    }

    pub fn next_sample(&mut self) -> f32 {
        let frequency = f32::from_bits(self.frequency.load(Ordering::Relaxed));
        let volume = f32::from_bits(self.volume.load(Ordering::Relaxed));
        
        // Slow bounded random walk models analog pitch drift; fresh per-sample
        // noise here acts as FM noise and adds audible hiss
        self.drift = (self.drift + (rand::random::<f32>() - 0.5) * 2e-5) * 0.9995;
        let detuned_frequency = frequency * (1.0 + self.drift);
        
        // More precise phase accumulation
        self.phase += detuned_frequency as f64 / self.sample_rate as f64;
        self.phase %= 1.0;

        let raw_sample = match self.waveform {
            Waveform::Sine => (self.phase as f32 * 2.0 * PI).sin(),
            Waveform::Square => self.polyblep_square(self.phase as f32, detuned_frequency),
            Waveform::Sawtooth => self.polyblep_saw(self.phase as f32, detuned_frequency),
            Waveform::Triangle => self.polyblep_triangle(self.phase as f32, detuned_frequency),
        };

        // Apply soft clipping for analog-like distortion
        let clipped_sample = self.soft_clip(raw_sample);

        clipped_sample * volume
    }

    fn polyblep(&self, t: f32, dt: f32) -> f32 {
        if t < dt {
            let t = t / dt;
            return 2.0 * t - t * t - 1.0;
        } else if t > 1.0 - dt {
            let t = (t - 1.0) / dt;
            return t * t + 2.0 * t + 1.0;
        } else {
            return 0.0;
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
            return dt * (t * t * t / 3.0 - t * t / 2.0 - t + 1.0 / 3.0);
        } else if t > 1.0 - dt {
            let t = (t - 1.0) / dt;
            return dt * (-t * t * t / 3.0 + t * t + t + 1.0 / 3.0);
        } else {
            return 0.0;
        }
    }

    fn soft_clip(&self, x: f32) -> f32 {
        // Cubic soft clip; the previous formula inverted polarity for |x| > ~1.7
        let x = x.clamp(-1.5, 1.5);
        x * (1.0 - x * x / 6.75)
    }


    pub fn set_frequency(&self, frequency: f32) {
        self.frequency.store(frequency.to_bits(), Ordering::Relaxed);
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume.store(volume.to_bits(), Ordering::Relaxed);
    }

    pub fn set_waveform(&mut self, waveform: Waveform) {
        self.waveform = waveform;
    }

    pub fn note_to_frequency(note: u8) -> f32 {
        440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0)
    }
}