use std::f32::consts::PI;
use reverb::Reverb as SecondReverb;

pub struct Reverb {
    // One instance of every stage per channel: pushing left and right
    // through shared filter state interleaves the two signals and corrupts
    // both (see the same note in chorus.rs)
    early_reflections: [EarlyReflections; 2],
    late_reflections: [LateReflections; 2],
    modulation: [Modulation; 2],
    eq: [Equalizer; 2],
    wet: f32,
    dry: f32,
    second_reverb: SecondReverb,
}

struct EarlyReflections {
    delay_line: DelayLine,
    taps: Vec<(usize, f32)>,
}

struct LateReflections {
    delay_lines: Vec<DelayLine>,
    feedback_matrix: Vec<Vec<f32>>,
    filters: Vec<Biquad>,
    decay: f32,
    damping: f32,
}

struct Modulation {
    lfos: Vec<LFO>,
    depths: Vec<f32>,
}

struct Equalizer {
    low_shelf: Biquad,
    high_shelf: Biquad,
}
struct DelayLine {
    buffer: Vec<f32>,
    write_pos: usize,
    size: usize,
}

struct LFO {
    phase: f32,
    freq: f32,
}

struct Biquad {
    b0: f32, b1: f32, b2: f32,
    a1: f32, a2: f32,
    x1: f32, x2: f32,
    y1: f32, y2: f32,
}




impl LateReflections {
    fn new(sample_rate: f32, num_channels: usize) -> Self {
        let delay_times_ms = [29.0, 37.0, 43.0, 53.0];
        let delay_lines = delay_times_ms.iter()
            .map(|&ms| DelayLine::new((ms * sample_rate / 1000.0) as usize))
            .collect();

        let feedback_matrix = Self::create_feedback_matrix(num_channels);

        let filters = (0..num_channels)
            .map(|_| Biquad::new_lowpass(5000.0, 0.7, sample_rate))
            .collect();

        Self {
            delay_lines,
            feedback_matrix,
            filters,
            decay: 0.1,
            damping: 0.5,
        }
    }

    fn create_feedback_matrix(size: usize) -> Vec<Vec<f32>> {
        let mut matrix = vec![vec![0.0; size]; size];
        for i in 0..size {
            for j in 0..size {
                if i != j {
                    matrix[i][j] = 0.05 / (size as f32 - 1.0);
                }
            }
        }
        matrix
    }

    fn process(&mut self, input: f32) -> f32 {
        let mut output = 0.0;
        let n = self.delay_lines.len().min(4);

        // Read from delay lines and apply filtering.
        // Fixed-size buffers: this runs per sample on the audio thread,
        // so heap allocation here is not acceptable.
        let mut temp_outputs = [0.0f32; 4];
        for i in 0..n {
            let delayed = self.delay_lines[i].read(0);
            temp_outputs[i] = self.filters[i].process(delayed);
        }

        // Apply feedback matrix
        let mut feedback_outputs = [0.0f32; 4];
        for i in 0..n {
            for j in 0..n {
                feedback_outputs[i] += temp_outputs[j] * self.feedback_matrix[i][j];
            }
        }

        // Update delay lines
        for i in 0..n {
            let new_sample = input + feedback_outputs[i] * self.decay;
            self.delay_lines[i].write(new_sample);
            output += new_sample;
        }

        // Apply damping
        output = output * (1.0 - self.damping) + input * self.damping;

        output * 0.25 // Attenuate output
    }

}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let num_channels = 4;
        let mut second_reverb = SecondReverb::new();

        second_reverb.bandwidth(0.8);  // Increase bandwidth to soften the sound
        second_reverb.damping(0.8);    // Increase damping to reduce high frequency resonances
        second_reverb.decay(0.8);      // Reduce decay to shorten the reverb tail
        second_reverb.diffusion(0.7, 0.7, 0.7, 0.7);  // Set diffusion to smooth out distinct echoes

        Self {
            early_reflections: [
                EarlyReflections::new(sample_rate),
                EarlyReflections::new(sample_rate),
            ],
            late_reflections: [
                LateReflections::new(sample_rate, num_channels),
                LateReflections::new(sample_rate, num_channels),
            ],
            modulation: [
                Modulation::new(sample_rate, num_channels),
                Modulation::new(sample_rate, num_channels),
            ],
            eq: [Equalizer::new(sample_rate), Equalizer::new(sample_rate)],
            wet: 0.7,
            dry: 0.3,
            second_reverb,
        }
    }

    pub fn process(&mut self, input_left: f32, input_right: f32) -> (f32, f32) {
        let mut wet = [input_left, input_right];
        for (ch, sample) in wet.iter_mut().enumerate() {
            let early = self.early_reflections[ch].process(*sample);
            let late = self.late_reflections[ch].process(early);
            let modulated = self.modulation[ch].process(late);
            *sample = self.eq[ch].process(modulated);
        }

        // The plate takes dry mono input once per sample frame and returns
        // the averaged wet stereo taps
        let plate = self
            .second_reverb
            .calc_sample((wet[0] + wet[1]) * 0.5, 0.6);

        let output_left = input_left * self.dry + (wet[0] * 0.5 + plate * 0.5) * self.wet;
        let output_right = input_right * self.dry + (wet[1] * 0.5 + plate * 0.5) * self.wet;

        (output_left, output_right)
    }

    pub fn set_decay(&mut self, decay: f32) {
        for late in &mut self.late_reflections {
            late.decay = decay.clamp(0.0, 0.98);
        }
        self.second_reverb.decay(decay);
    }

    pub fn set_wet(&mut self, wet: f32) {
        self.wet = wet.clamp(0.0, 1.0);
        self.dry = 1.0 - self.wet;
    }
}

impl EarlyReflections {
    fn new(sample_rate: f32) -> Self {
        // Calculate delay times in samples
        let delay_times = vec![
            (0.007 * sample_rate) as usize,
            (0.012243 * sample_rate) as usize,
            (0.0154443 * sample_rate) as usize,
            (0.023405 * sample_rate) as usize,
        ];

        let max_delay = *delay_times.iter().max().unwrap_or(&0);
        let delay_line = DelayLine::new(max_delay + 1);

        let taps = delay_times.into_iter()
            .enumerate()
            .map(|(i, delay)| (delay, 0.7f32.powf(i as f32)))
            .collect();

        Self {
            delay_line,
            taps,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        self.delay_line.write(input);
        let reflection = self.taps.iter()
            .map(|&(delay, gain)| self.delay_line.read(delay) * gain)
            .sum::<f32>();
        (reflection + input * 0.125) * 0.5  // Apply gain reduction
    }
}

impl DelayLine {
    fn new(size: usize) -> Self {
        let size = size.max(1);  // Ensure size is at least 1
        Self {
            buffer: vec![0.0; size],
            write_pos: 0,
            size,
        }
    }

    fn read(&self, delay: usize) -> f32 {
        let delay = delay.min(self.size - 1);
        let read_pos = (self.size + self.write_pos - delay) % self.size;
        self.buffer[read_pos]
    }

    fn write(&mut self, input: f32) {
        self.buffer[self.write_pos] = input;
        self.write_pos = (self.write_pos + 1) % self.size;
    }
}



impl Modulation {
    fn new(sample_rate: f32, num_channels: usize) -> Self {
        let lfos = (0..num_channels)
            .map(|i| LFO::new(0.1 + i as f32 * 0.05, sample_rate))
            .collect();
        let depths = vec![0.0002, 0.0003, 0.0004, 0.0005];
        Self { lfos, depths }
    }

    fn process(&mut self, input: f32) -> f32 {
        self.lfos.iter_mut()
            .zip(&self.depths)
            .fold(input, |acc, (lfo, &depth)| {
                acc * (1.0 + lfo.process() * depth)
            })
    }
}

impl Equalizer {
    fn new(sample_rate: f32) -> Self {
        Self {
            low_shelf: Biquad::new_low_shelf(200.0, 0.0, sample_rate),
            high_shelf: Biquad::new_high_shelf(4000.0, -2.0, sample_rate),
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let low = self.low_shelf.process(input);
        self.high_shelf.process(low)
    }
}


impl LFO {
    fn new(freq: f32, sample_rate: f32) -> Self {
        Self {
            phase: 0.0,
            freq: freq / sample_rate,
        }
    }

    fn process(&mut self) -> f32 {
        self.phase += self.freq;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        (self.phase * 2.0 * PI).sin()
    }
}

impl Biquad {
    fn new_low_shelf(freq: f32, gain_db: f32, sample_rate: f32) -> Self {
        let (b0, b1, b2, a0, a1, a2) = Self::calc_low_shelf_coeffs(freq, gain_db, sample_rate);
        Self::from_coeffs(b0, b1, b2, a0, a1, a2)
    }

    fn new_high_shelf(freq: f32, gain_db: f32, sample_rate: f32) -> Self {
        let (b0, b1, b2, a0, a1, a2) = Self::calc_high_shelf_coeffs(freq, gain_db, sample_rate);
        Self::from_coeffs(b0, b1, b2, a0, a1, a2)
    }

    fn from_coeffs(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        assert!(a0 != 0.0, "a0 coefficient cannot be zero");
        Self {
            b0: b0 / a0, b1: b1 / a0, b2: b2 / a0,
            a1: a1 / a0, a2: a2 / a0,
            x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = input * self.b0 + self.x1 * self.b1 + self.x2 * self.b2
                     - self.y1 * self.a1 - self.y2 * self.a2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }

    fn calc_low_shelf_coeffs(freq: f32, gain_db: f32, sample_rate: f32) -> (f32, f32, f32, f32, f32, f32) {
        let a = 10.0f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / 2.0 * (2.0f32).sqrt();

        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * a.sqrt() * alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * a.sqrt() * alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * a.sqrt() * alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * a.sqrt() * alpha;

        (b0, b1, b2, a0, a1, a2)
    }

    fn calc_high_shelf_coeffs(freq: f32, gain_db: f32, sample_rate: f32) -> (f32, f32, f32, f32, f32, f32) {
        let a = 10.0f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / 2.0 * (2.0f32).sqrt();

        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * a.sqrt() * alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * a.sqrt() * alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * a.sqrt() * alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * a.sqrt() * alpha;

        (b0, b1, b2, a0, a1, a2)
    }

    fn new_lowpass(freq: f32, q: f32, sample_rate: f32) -> Self {
        let w0 = 2.0 * PI * freq / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();

        let b0 = (1.0 - cos_w0) / 2.0;
        let b1 = 1.0 - cos_w0;
        let b2 = (1.0 - cos_w0) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        Self::from_coeffs(b0, b1, b2, a0, a1, a2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_at_zero_wet() {
        let mut reverb = Reverb::new(48000.0);
        reverb.set_wet(0.0);
        for n in 0..4800 {
            let x = (n as f32 * 0.05).sin() * 0.5;
            let (l, r) = reverb.process(x, -x);
            assert_eq!(l, x);
            assert_eq!(r, -x);
        }
    }

    /// Identical inputs must produce identical outputs. When the channels
    /// shared one set of filter states this failed immediately (each call
    /// interleaved into the other's history).
    #[test]
    fn identical_inputs_stay_identical() {
        let mut reverb = Reverb::new(48000.0);
        reverb.set_wet(1.0);
        for n in 0..48000 {
            let x = (2.0 * PI * 220.0 * n as f32 / 48000.0).sin() * 0.3;
            let (l, r) = reverb.process(x, x);
            assert!(l.is_finite() && r.is_finite());
            assert!(
                (l - r).abs() < 1e-6,
                "channels diverged at sample {n}: {l} vs {r}"
            );
        }
    }
}