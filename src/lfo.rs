// LFO after US 3,943,456 ("Signal Generator ... Employing Variable Rate
// Integrator", Luce/Moog Music, used for the Crumar Spirit's second LFO):
// one integrator whose charge and discharge rates are rebalanced by a
// single shape control, morphing the output continuously from falling
// sawtooth through triangle to rising ramp. Range follows the Juno-106
// service endpoints, 0.1-30 Hz.
//
// The LFO is GLOBAL — one per instrument, shared by every voice, the way
// a modular's 901 in low range or the Juno's single LFO drives everything
// together. Correlated modulation is part of the sound.

pub struct Lfo {
    sample_rate: f32,
    phase: f32,
    rate: f32,  // Hz
    shape: f32, // 0 = falling saw, 0.5 = triangle, 1 = rising ramp
}

impl Lfo {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            rate: 1.0,
            shape: 0.5,
        }
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate.clamp(0.1, 30.0);
    }

    pub fn set_shape(&mut self, shape: f32) {
        self.shape = shape.clamp(0.0, 1.0);
    }

    /// Bipolar output, -1..+1.
    #[inline]
    pub fn next(&mut self) -> f32 {
        self.phase += self.rate / self.sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        // Rise for `d` of the cycle, fall for the rest; the integrator's
        // two rates trade off so the period stays constant
        let d = self.shape.clamp(0.02, 0.98);
        if self.phase < d {
            -1.0 + 2.0 * self.phase / d
        } else {
            1.0 - 2.0 * (self.phase - d) / (1.0 - d)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_at_center_shape() {
        let sr = 1000.0;
        let mut lfo = Lfo::new(sr);
        lfo.set_rate(1.0);
        lfo.set_shape(0.5);
        let samples: Vec<f32> = (0..1000).map(|_| lfo.next()).collect();
        let max = samples.iter().cloned().fold(f32::MIN, f32::max);
        let min = samples.iter().cloned().fold(f32::MAX, f32::min);
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        assert!(max > 0.98 && min < -0.98, "full swing: {min}..{max}");
        assert!(mean.abs() < 0.02, "triangle should be centered, mean={mean}");
    }

    #[test]
    fn shape_morphs_rise_fraction() {
        // At shape 0.9 the LFO spends ~90% of the cycle rising
        let sr = 1000.0;
        let mut lfo = Lfo::new(sr);
        lfo.set_rate(1.0);
        lfo.set_shape(0.9);
        let samples: Vec<f32> = (0..1000).map(|_| lfo.next()).collect();
        let rising = samples
            .windows(2)
            .filter(|w| w[1] > w[0])
            .count() as f32
            / 999.0;
        assert!(
            (0.85..=0.95).contains(&rising),
            "rise fraction should track shape, got {rising}"
        );
    }

    #[test]
    fn rate_sets_period() {
        let sr = 44100.0;
        let mut lfo = Lfo::new(sr);
        lfo.set_rate(5.0);
        lfo.set_shape(0.5);
        // Count positive peaks over 2 seconds
        let samples: Vec<f32> = (0..(2 * 44100)).map(|_| lfo.next()).collect();
        let mut peaks = 0;
        for w in samples.windows(3) {
            if w[1] > 0.9 && w[1] >= w[0] && w[1] > w[2] {
                peaks += 1;
            }
        }
        assert!((9..=11).contains(&peaks), "expected ~10 peaks at 5 Hz, got {peaks}");
    }
}
