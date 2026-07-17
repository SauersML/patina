// src/tape.rs
//
// A physically-motivated compact-cassette model, in signal order:
//
//   transport      wow (capstan eccentricity, ~0.9 Hz) + 1/f speed drift +
//                  flutter (pinch roller, ~6.3 Hz) + random jitter, applied
//                  as one modulated delay shared by both channels — they sit
//                  on the same strip of tape.
//   record side    pre-emphasis (120 µs curve), then a Langevin-function
//                  magnetization curve — the anhysteretic B/H curve of the
//                  oxide — offset by a small record-bias asymmetry for even
//                  harmonics, then matching playback de-emphasis.
//   playback head  head-bump resonance near 90 Hz, and gap/spacing loss as a
//                  one-pole HF rolloff that worsens with AGE; azimuth error
//                  costs the right channel a little extra top end.
//   media wear     Poisson-scheduled oxide dropouts and bias hiss, both
//                  scaled by AGE.
//
// All four knobs at zero leaves the unit transparent (the delay line still
// runs so engaging a knob never reads a stale buffer), which also makes every
// parameter freely automatable from song files.

use rand::Rng;
use std::f32::consts::PI;

const WOW_HZ: f32 = 0.9;
const FLUTTER_HZ: f32 = 6.3;
/// Nominal head-to-read delay; modulation swings around this center.
const CENTER_DELAY_S: f32 = 0.006;
/// ±3.5 ms at 0.9 Hz ≈ ±34 cents of pitch swing at full wow.
const WOW_DEPTH_S: f32 = 0.0035;
const DRIFT_DEPTH_S: f32 = 0.0015;
const FLUTTER_DEPTH_S: f32 = 0.00012;
const JITTER_DEPTH_S: f32 = 0.00008;

pub struct Tape {
    sample_rate: f32,
    wow: f32,
    flutter: f32,
    drive: f32,
    age: f32,

    // Transport
    buffer_left: Vec<f32>,
    buffer_right: Vec<f32>,
    size: usize,
    write: usize,
    wow_phase: f32,
    flutter_phase: f32,
    drift: SmoothedNoise,
    jitter: SmoothedNoise,

    // Record/playback electronics (independent state per channel)
    pre_emphasis: [OnePoleHighPass; 2],
    de_emphasis: [OnePoleHighPass; 2],
    dc_block: [OnePoleHighPass; 2],
    head_bump: [PeakingFilter; 2],
    gap_loss: [OnePoleLowPass; 2],

    // Media wear
    hiss_lp: [OnePoleLowPass; 2],
    hiss_level: f32,
    dropout_env: f32,
    dropout_target: f32,
    dropout_remaining: f32,
    next_dropout: f32,
}

impl Tape {
    pub fn new(sample_rate: f32) -> Self {
        let size = (sample_rate * CENTER_DELAY_S * 2.0) as usize + 8;
        let mut tape = Self {
            sample_rate,
            wow: 0.0,
            flutter: 0.0,
            drive: 0.0,
            age: 0.0,
            buffer_left: vec![0.0; size],
            buffer_right: vec![0.0; size],
            size,
            write: 0,
            wow_phase: 0.0,
            flutter_phase: 0.0,
            drift: SmoothedNoise::new(sample_rate, 0.3),
            jitter: SmoothedNoise::new(sample_rate, 12.0),
            // The standard cassette playback time constant is 120 µs
            // (~1326 Hz); pre/de-emphasis shelves pivot there.
            pre_emphasis: [OnePoleHighPass::new(sample_rate, 1326.0); 2],
            de_emphasis: [OnePoleHighPass::new(sample_rate, 1326.0); 2],
            dc_block: [OnePoleHighPass::new(sample_rate, 10.0); 2],
            head_bump: [PeakingFilter::new(); 2],
            gap_loss: [OnePoleLowPass::new(sample_rate, 16000.0); 2],
            hiss_lp: [OnePoleLowPass::new(sample_rate, 7000.0); 2],
            hiss_level: 0.0,
            dropout_env: 1.0,
            dropout_target: 1.0,
            dropout_remaining: 0.0,
            next_dropout: f32::MAX,
        };
        tape.update_drive();
        tape.update_age();
        tape
    }

    pub fn set_wow(&mut self, wow: f32) {
        self.wow = wow.clamp(0.0, 1.0);
    }

    pub fn set_flutter(&mut self, flutter: f32) {
        self.flutter = flutter.clamp(0.0, 1.0);
    }

    pub fn set_drive(&mut self, drive: f32) {
        self.drive = drive.clamp(0.0, 1.0);
        self.update_drive();
    }

    pub fn set_age(&mut self, age: f32) {
        self.age = age.clamp(0.0, 1.0);
        self.update_age();
    }

    fn update_drive(&mut self) {
        // Head bump only shows up once the tape is actually being hit
        let bump_db = 3.0 * self.drive;
        for filter in &mut self.head_bump {
            filter.set_peaking(self.sample_rate, 90.0, 1.2, bump_db);
        }
    }

    fn update_age(&mut self) {
        // Gap/spacing loss: a fresh tape reaches ~16 kHz, a worn one ~3.8 kHz.
        // Azimuth error costs the right channel a bit more.
        let cutoff = 16000.0 * (3800.0f32 / 16000.0).powf(self.age);
        self.gap_loss[0].set_cutoff(self.sample_rate, cutoff);
        self.gap_loss[1].set_cutoff(self.sample_rate, cutoff * (1.0 - 0.25 * self.age));

        self.hiss_level = self.age * self.age * 0.002;

        // Reschedule so raising AGE from zero doesn't wait on a stale
        // (effectively infinite) arrival time
        self.next_dropout = self.sample_dropout_interval();
    }

    /// Poisson arrival time for the next oxide dropout, in samples.
    fn sample_dropout_interval(&self) -> f32 {
        let rate_per_second = self.age * self.age * 0.35;
        if rate_per_second < 1e-4 {
            return f32::MAX;
        }
        let u: f32 = rand::thread_rng().gen_range(1e-6..1.0f32);
        -u.ln() * self.sample_rate / rate_per_second
    }

    pub fn process(&mut self, input_left: f32, input_right: f32) -> (f32, f32) {
        // Keep the transport rolling even when idle so turning a knob up
        // never reads stale audio out of the delay line
        self.buffer_left[self.write] = input_left;
        self.buffer_right[self.write] = input_right;
        self.write = (self.write + 1) % self.size;

        if self.wow + self.flutter + self.drive + self.age < 1e-4 {
            return (input_left, input_right);
        }

        // --- Transport: one tape speed, both channels ---
        self.wow_phase = (self.wow_phase + WOW_HZ / self.sample_rate).fract();
        self.flutter_phase = (self.flutter_phase + FLUTTER_HZ / self.sample_rate).fract();

        let wow_sq = self.wow * self.wow; // perceptual taper
        let flutter_sq = self.flutter * self.flutter;
        let delay_seconds = CENTER_DELAY_S
            + wow_sq * WOW_DEPTH_S * (2.0 * PI * self.wow_phase).sin()
            + wow_sq * DRIFT_DEPTH_S * self.drift.next()
            + flutter_sq * FLUTTER_DEPTH_S * (2.0 * PI * self.flutter_phase).sin()
            + flutter_sq * JITTER_DEPTH_S * self.jitter.next();
        let delay = (delay_seconds * self.sample_rate).clamp(2.0, self.size as f32 - 4.0);

        let read = (self.write as f32 - delay + self.size as f32) as usize % self.size;
        let frac = delay.fract();
        let left = read_cubic(&self.buffer_left, read, frac, self.size);
        let right = read_cubic(&self.buffer_right, read, frac, self.size);

        // --- Media wear: dropout envelope is shared, oxide sheds across the
        // full tape width ---
        if self.dropout_remaining > 0.0 {
            self.dropout_remaining -= 1.0;
            if self.dropout_remaining <= 0.0 {
                self.dropout_target = 1.0;
                self.next_dropout = self.sample_dropout_interval();
            }
        } else if self.next_dropout != f32::MAX {
            self.next_dropout -= 1.0;
            if self.next_dropout <= 0.0 {
                let mut rng = rand::thread_rng();
                self.dropout_target = 1.0 - rng.gen_range(0.25..0.85);
                self.dropout_remaining = rng.gen_range(0.002..0.045) * self.sample_rate;
            }
        }
        self.dropout_env += 0.004 * (self.dropout_target - self.dropout_env);

        let mut out = [left, right];
        for (ch, sample) in out.iter_mut().enumerate() {
            *sample = self.process_channel(*sample, ch);
        }
        (out[0], out[1])
    }

    fn process_channel(&mut self, x: f32, ch: usize) -> f32 {
        // --- Record side: emphasis, bias, magnetization ---
        let emphasized = x + 0.6 * self.pre_emphasis[ch].process(x);

        let a = 0.5 + 4.5 * self.drive;
        let bias = 0.08 * self.drive;
        let norm = langevin(a);
        let saturated = langevin(a * (emphasized + bias)) / norm - langevin(a * bias) / norm;
        let trimmed = saturated / (1.0 + 0.6 * self.drive);

        let de_emphasized = trimmed - 0.35 * self.de_emphasis[ch].process(trimmed);
        let centered = self.dc_block[ch].process(de_emphasized);

        // --- Playback head ---
        let bumped = self.head_bump[ch].process(centered);
        let dulled = self.gap_loss[ch].process(bumped);

        // --- Media wear ---
        let hiss = self.hiss_lp[ch].process(rand::thread_rng().gen_range(-1.0..1.0f32));
        dulled * self.dropout_env + hiss * self.hiss_level
    }
}

/// Langevin function L(x) = coth(x) - 1/x: the anhysteretic magnetization
/// curve of an ideal paramagnet, and the standard model for tape oxide.
fn langevin(x: f32) -> f32 {
    if x.abs() < 1e-3 {
        x / 3.0
    } else {
        1.0 / x.tanh() - 1.0 / x
    }
}

fn read_cubic(buffer: &[f32], index: usize, frac: f32, size: usize) -> f32 {
    cubic_interpolate(
        &[
            buffer[(index + size - 1) % size],
            buffer[index],
            buffer[(index + 1) % size],
            buffer[(index + 2) % size],
        ],
        frac,
    )
}

fn cubic_interpolate(y: &[f32; 4], mu: f32) -> f32 {
    let mu2 = mu * mu;
    let a0 = y[3] - y[2] - y[0] + y[1];
    let a1 = y[0] - y[1] - a0;
    let a2 = y[2] - y[0];
    let a3 = y[1];
    a0 * mu * mu2 + a1 * mu2 + a2 * mu + a3
}

/// White noise through two cascaded one-pole lowpasses, normalized to roughly
/// ±1: the slow 1/f-ish component of transport speed error.
struct SmoothedNoise {
    stage1: f32,
    stage2: f32,
    alpha: f32,
    gain: f32,
}

impl SmoothedNoise {
    fn new(sample_rate: f32, cutoff: f32) -> Self {
        let alpha = 1.0 - (-2.0 * PI * cutoff / sample_rate).exp();
        Self {
            stage1: 0.0,
            stage2: 0.0,
            alpha,
            // Restores unit-ish variance after the heavy lowpassing
            gain: 1.0 / (0.577 * (alpha / 2.0).sqrt().max(1e-6)),
        }
    }

    fn next(&mut self) -> f32 {
        let white: f32 = rand::thread_rng().gen_range(-1.0..1.0);
        self.stage1 += self.alpha * (white - self.stage1);
        self.stage2 += self.alpha * (self.stage1 - self.stage2);
        (self.stage2 * self.gain).clamp(-1.0, 1.0)
    }
}

#[derive(Clone, Copy)]
struct OnePoleHighPass {
    prev_input: f32,
    prev_output: f32,
    alpha: f32,
}

impl OnePoleHighPass {
    fn new(sample_rate: f32, cutoff: f32) -> Self {
        let rc = 1.0 / (2.0 * PI * cutoff);
        let dt = 1.0 / sample_rate;
        Self {
            prev_input: 0.0,
            prev_output: 0.0,
            alpha: rc / (rc + dt),
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.alpha * (self.prev_output + input - self.prev_input);
        self.prev_input = input;
        self.prev_output = output;
        output
    }
}

#[derive(Clone, Copy)]
struct OnePoleLowPass {
    state: f32,
    alpha: f32,
}

impl OnePoleLowPass {
    fn new(sample_rate: f32, cutoff: f32) -> Self {
        let mut lp = Self { state: 0.0, alpha: 0.0 };
        lp.set_cutoff(sample_rate, cutoff);
        lp
    }

    fn set_cutoff(&mut self, sample_rate: f32, cutoff: f32) {
        self.alpha = 1.0 - (-2.0 * PI * cutoff / sample_rate).exp();
    }

    fn process(&mut self, input: f32) -> f32 {
        self.state += self.alpha * (input - self.state);
        self.state
    }
}

/// RBJ peaking EQ biquad, used for the playback-head bump.
#[derive(Clone, Copy)]
struct PeakingFilter {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl PeakingFilter {
    fn new() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    fn set_peaking(&mut self, sample_rate: f32, freq: f32, q: f32, gain_db: f32) {
        let amp = 10.0f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha / amp;
        self.b0 = (1.0 + alpha * amp) / a0;
        self.b1 = (-2.0 * w0.cos()) / a0;
        self.b2 = (1.0 - alpha * amp) / a0;
        self.a1 = (-2.0 * w0.cos()) / a0;
        self.a2 = (1.0 - alpha / amp) / a0;
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        self.y1 = output;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48000.0;

    fn sine(n: usize, freq: f32) -> impl Iterator<Item = f32> {
        (0..n).map(move |i| (2.0 * PI * freq * i as f32 / FS).sin() * 0.5)
    }

    #[test]
    fn transparent_when_idle() {
        let mut tape = Tape::new(FS);
        for x in sine(4800, 440.0) {
            let (l, r) = tape.process(x, x);
            assert_eq!(l, x);
            assert_eq!(r, x);
        }
    }

    #[test]
    fn saturation_is_bounded_and_finite() {
        let mut tape = Tape::new(FS);
        tape.set_drive(1.0);
        let mut peak = 0.0f32;
        for x in sine(48000, 220.0) {
            let (l, r) = tape.process(x * 2.0, x * 2.0); // hit it hard
            assert!(l.is_finite() && r.is_finite());
            peak = peak.max(l.abs());
        }
        assert!(peak > 0.05, "signal should pass through, peak {}", peak);
        assert!(peak < 1.2, "magnetization curve should compress, peak {}", peak);
    }

    #[test]
    fn aged_tape_hisses_and_dulls() {
        let mut tape = Tape::new(FS);
        tape.set_age(1.0);
        // Hiss: silence in, noise floor out
        let mut energy = 0.0f32;
        for _ in 0..48000 {
            let (l, _) = tape.process(0.0, 0.0);
            energy += l * l;
        }
        let hiss_rms = (energy / 48000.0).sqrt();
        assert!(hiss_rms > 1e-5 && hiss_rms < 0.01, "hiss rms {}", hiss_rms);

        // Gap loss: a 10 kHz tone loses far more level than a 200 Hz tone
        let gain_at = |freq: f32| {
            let mut tape = Tape::new(FS);
            tape.set_age(1.0);
            let mut in_e = 0.0f32;
            let mut out_e = 0.0f32;
            for x in sine(48000, freq) {
                let (l, _) = tape.process(x, x);
                in_e += x * x;
                out_e += l * l;
            }
            (out_e / in_e).sqrt()
        };
        assert!(gain_at(10000.0) < 0.5 * gain_at(200.0));
    }

    #[test]
    fn wow_modulates_pitch() {
        // Count samples between rising zero crossings; with wow the period
        // must wander, without it the spread stays tight
        let period_spread = |wow: f32| {
            let mut tape = Tape::new(FS);
            tape.set_wow(wow);
            tape.set_drive(0.01); // keep the delay path engaged
            let mut periods = vec![];
            let mut prev = 0.0f32;
            let mut last_cross = 0usize;
            for (i, x) in sine(FS as usize * 4, 440.0).enumerate() {
                let (l, _) = tape.process(x, x);
                if prev <= 0.0 && l > 0.0 && i > last_cross + 10 {
                    if last_cross > 0 {
                        periods.push((i - last_cross) as f32);
                    }
                    last_cross = i;
                }
                prev = l;
            }
            let mean = periods.iter().sum::<f32>() / periods.len() as f32;
            let var = periods.iter().map(|p| (p - mean).powi(2)).sum::<f32>()
                / periods.len() as f32;
            var.sqrt()
        };
        assert!(period_spread(1.0) > 2.0 * period_spread(0.0));
    }
}
