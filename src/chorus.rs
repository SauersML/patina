use std::f32::consts::PI;
use rand::Rng;

pub struct Chorus {
    buffer_left: Vec<f32>,
    buffer_right: Vec<f32>,
    index: usize,
    size: usize,
    mode: ChorusMode,
    sample_rate: f32,
    // Separate filter state per channel; sharing one filter between left and
    // right corrupts both channels' state
    low_pass_left: LowPassFilter,
    low_pass_right: LowPassFilter,
    high_pass_left: HighPassFilter,
    high_pass_right: HighPassFilter,
    noise_generator: NoiseGenerator,
    saturation: Saturation,
    feedback: f32,
    voices: Vec<Voice>,
    /// Panel overrides of the mode presets. `None` = the knob has never
    /// been touched, so the selected mode's own preset stands; `Some` = a
    /// knob position that must survive a switch throw (see `set_mode`).
    rate: Option<f32>,
    depth: Option<f32>,
    wet_dry_mix: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ChorusMode {
    Off,
    I,
    II,
    III,
    IV,
}

struct LowPassFilter {
    prev: f32,
    alpha: f32,
}

struct HighPassFilter {
    prev_input: f32,
    prev_output: f32,
    cutoff: f32,
}

struct NoiseGenerator {
    level: f32,
    prev: f32,
}

struct Saturation {
    drive: f32,
}

struct Voice {
    phase_left: f32,
    phase_right: f32,
    rate_left: f32,
    rate_right: f32,
    depth: f32,
    smooth_depth: f32,
}

impl Chorus {
    pub fn new(sample_rate: f32) -> Self {
        let max_delay_ms = 40.0;
        let size = (sample_rate * max_delay_ms / 1000.0) as usize;
        Self {
            buffer_left: vec![0.0; size],
            buffer_right: vec![0.0; size],
            index: 0,
            size,
            mode: ChorusMode::Off,
            sample_rate,
            low_pass_left: LowPassFilter::new(sample_rate),
            low_pass_right: LowPassFilter::new(sample_rate),
            high_pass_left: HighPassFilter::new(sample_rate),
            high_pass_right: HighPassFilter::new(sample_rate),
            noise_generator: NoiseGenerator::new(),
            saturation: Saturation::new(),
            feedback: 0.25,
            rate: None,
            depth: None,
            voices: vec![
                Voice::new(0.513, 0.515, 0.007),
                Voice::new(0.75, 0.753, 0.006),
                Voice::new(0.95, 0.953, 0.005),
            ],
            wet_dry_mix: 0.5,
        }
    }

    pub fn set_rate(&mut self, rate: f32) {
        // House rule: automation re-asserts values every block; a setter
        // that re-randomizes (or resets) state must early-return on an
        // unchanged value, or the per-voice detune re-rolls continuously
        let rate = rate.clamp(0.1, 10.0);
        if self.rate == Some(rate) {
            return;
        }
        self.rate = Some(rate);
        self.apply_rate();
    }

    pub fn set_depth(&mut self, depth: f32) {
        let depth = depth.clamp(0.0, 1.0);
        if self.depth == Some(depth) {
            return;
        }
        self.depth = Some(depth);
        self.apply_depth();
    }

    fn apply_rate(&mut self) {
        let Some(rate) = self.rate else { return };
        for voice in &mut self.voices {
            voice.rate_left = rate * (0.9 + rand::thread_rng().gen::<f32>() * 0.2);
            voice.rate_right = rate * (0.9 + rand::thread_rng().gen::<f32>() * 0.2);
        }
    }

    fn apply_depth(&mut self) {
        let Some(depth) = self.depth else { return };
        for voice in &mut self.voices {
            // Knob is 0..1; voice depth is the LFO delay swing in seconds.
            // Full depth = 10 ms, matching the scale of the mode presets.
            voice.depth = depth * 0.010 * (0.9 + rand::thread_rng().gen::<f32>() * 0.2);
        }
    }

    pub fn set_mode(&mut self, mode: ChorusMode) {
        // Re-asserting the current mode (song automation does, every
        // block) must not rebuild the BBD voices — that resets their
        // delay-line state mid-note — and must not stomp a chorus_mix
        // override. Only an actual switch throw re-derives anything.
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        match mode {
            ChorusMode::Off => {
                self.voices.clear();
                self.wet_dry_mix = 0.0;
            },
            ChorusMode::I => {
                self.voices = vec![Voice::new(0.513, 0.515, 0.00535)];
                self.wet_dry_mix = 0.5;
            },
            ChorusMode::II => {
                self.voices = vec![Voice::new(0.863, 0.865, 0.00535)];
                self.wet_dry_mix = 0.8;
            },
            ChorusMode::III => {
                self.voices = vec![
                    Voice::new(0.513, 0.515, 0.0037),
                    Voice::new(0.863, 0.865, 0.0037),
                ];
                self.wet_dry_mix = 0.5;
            },
            ChorusMode::IV => {
                self.voices = vec![
                    Voice::new(0.5, 0.502, 0.007),
                    Voice::new(0.75, 0.752, 0.006),
                    Voice::new(1.0, 1.002, 0.005),
                    Voice::new(1.25, 1.252, 0.004),
                ];
                self.wet_dry_mix = 0.6;
            },
        }
        // A switch throw rebuilds the BBD voices from the mode's own
        // preset, which used to silently DISCARD the rate and depth knobs:
        // the setters early-return on an unchanged value (they must, they
        // re-roll per-voice detune), so re-asserting the same knob position
        // afterwards -- which is exactly what song automation and the UI's
        // apply-all do, every block -- was a no-op and the panel controls
        // stayed dead until someone physically moved them. Re-derive the
        // overrides here so the knobs survive the switch.
        self.apply_rate();
        self.apply_depth();
    }


    /// Override the insert mix set by the mode switch: `0` keeps the bus
    /// completely dry while the BBD still runs, so per-channel sends can
    /// be chorused alone. Selecting a mode afterwards re-derives its
    /// default mix, matching the hardware's switch behavior.
    pub fn set_mix(&mut self, mix: f32) {
        self.wet_dry_mix = mix.clamp(0.0, 1.0);
    }

    pub fn process(&mut self, input_left: f32, input_right: f32) -> (f32, f32) {
        self.process_with_send(input_left, input_right, 0.0, 0.0)
    }

    /// The BBD line is fed by the global bus scaled by the insert mix
    /// PLUS the per-channel send bus at unity — so `chorus_mode` on with
    /// `chorus_mix 0` chews only what the tracks send it.
    pub fn process_with_send(
        &mut self,
        input_left: f32,
        input_right: f32,
        send_left: f32,
        send_right: f32,
    ) -> (f32, f32) {
        if self.mode == ChorusMode::Off {
            return (input_left, input_right);
        }
        // The BBD line feeds back on itself, so one non-finite sample
        // circulates forever and the chorus never produces audio again.
        // Screening the input is O(1) and makes it a one-sample dropout.
        let input_left = if input_left.is_finite() { input_left } else { 0.0 };
        let input_right = if input_right.is_finite() { input_right } else { 0.0 };
        let send_left = if send_left.is_finite() { send_left } else { 0.0 };
        let send_right = if send_right.is_finite() { send_right } else { 0.0 };

        let m = self.wet_dry_mix.clamp(0.0, 1.0);
        let fed_left = input_left * m + send_left;
        let fed_right = input_right * m + send_right;

        let high_passed_left = self.high_pass_left.process(fed_left);
        let high_passed_right = self.high_pass_right.process(fed_right);
        let filtered_input_left = self.low_pass_left.process(high_passed_left);
        let filtered_input_right = self.low_pass_right.process(high_passed_right);

        let feedback_left = self.buffer_left[self.index];
        let feedback_right = self.buffer_right[self.index];
        let feedback = (feedback_left + feedback_right) * 0.5;
        let input_with_feedback_left = filtered_input_left + (self.feedback * feedback).clamp(-1.0, 1.0);
        let input_with_feedback_right = filtered_input_right + (self.feedback * feedback).clamp(-1.0, 1.0);

        self.buffer_left[self.index] = input_with_feedback_left;
        self.buffer_right[self.index] = input_with_feedback_right;
        self.index = (self.index + 1) % self.size;

        let (left_output, right_output) = self.calculate_delay_samples(input_with_feedback_left, input_with_feedback_right);

        // BBD hiss rides the line at the level the line is actually fed
        let n_gain = if send_left != 0.0 || send_right != 0.0 { m.max(0.25) } else { m };
        let noise = self.noise_generator.generate() * n_gain;
        let left_output = left_output + noise;
        let right_output = right_output + noise;

        let left_output = self.saturation.process(left_output);
        let right_output = self.saturation.process(right_output);

        let left = (1.0 - m) * input_left + left_output;
        let right = (1.0 - m) * input_right + right_output;

        (left.clamp(-1.0, 1.0), right.clamp(-1.0, 1.0))
    }


    fn calculate_delay_samples(&mut self, input_left: f32, input_right: f32) -> (f32, f32) {
        let mut left_output = 0.0;
        let mut right_output = 0.0;

        for voice in &mut self.voices {
            voice.phase_left += voice.rate_left / self.sample_rate;
            voice.phase_right += voice.rate_right / self.sample_rate;
            if voice.phase_left >= 1.0 { voice.phase_left -= 1.0; }
            if voice.phase_right >= 1.0 { voice.phase_right -= 1.0; }

            voice.smooth_depth += (voice.depth - voice.smooth_depth) * 0.001;

            let lfo_left = ((2.0 * PI * voice.phase_left).sin() * 0.51 + 0.5) * 0.5 +
                           ((2.0 * PI * voice.phase_left * 1.101).sin() * 0.5 + 0.5) * 0.5;
            let lfo_right = ((2.0 * PI * voice.phase_right).sin() * 0.5 + 0.51) * 0.5 +
                            ((2.0 * PI * voice.phase_right * 1.1).sin() * 0.5 + 0.5) * 0.5;

            let max_delay = self.size as f32 - 3.0;
            let delay_left =
                (voice.smooth_depth * self.sample_rate * lfo_left).clamp(1.0, max_delay);
            let delay_right =
                (voice.smooth_depth * self.sample_rate * lfo_right).clamp(1.0, max_delay);

            // The newest sample sits at index-1 (the write pointer has
            // already advanced). Interpolate at the true fractional position
            // so a sweeping delay never jumps at integer boundaries.
            let pos_left = self.index as f32 - 1.0 - delay_left + 2.0 * self.size as f32;
            let pos_right = self.index as f32 - 1.0 - delay_right + 2.0 * self.size as f32;

            let index_left = pos_left as usize % self.size;
            let index_right = pos_right as usize % self.size;

            let frac_left = pos_left.fract();
            let frac_right = pos_right.fract();

            let sample_left = cubic_interpolate(&[
                self.buffer_left[(index_left + self.size - 1) % self.size],
                self.buffer_left[index_left],
                self.buffer_left[(index_left + 1) % self.size],
                self.buffer_left[(index_left + 2) % self.size],
            ], frac_left);

            let sample_right = cubic_interpolate(&[
                self.buffer_right[(index_right + self.size - 1) % self.size],
                self.buffer_right[index_right],
                self.buffer_right[(index_right + 1) % self.size],
                self.buffer_right[(index_right + 2) % self.size],
            ], frac_right);

            left_output += sample_left;
            right_output += sample_right;
        }

        if !self.voices.is_empty() {
            left_output = left_output / self.voices.len() as f32 + input_left * 0.5;
            right_output = right_output / self.voices.len() as f32 + input_right * 0.5;
        } else {
            left_output = input_left;
            right_output = input_right;
        }

        (left_output, right_output)
    }
}

fn cubic_interpolate(y: &[f32; 4], mu: f32) -> f32 {
    let mu2 = mu * mu;
    let a0 = y[3] - y[2] - y[0] + y[1];
    let a1 = y[0] - y[1] - a0;
    let a2 = y[2] - y[0];
    let a3 = y[1];
    a0 * mu * mu2 + a1 * mu2 + a2 * mu + a3
}

impl LowPassFilter {
    fn new(sample_rate: f32) -> Self {
        // BBD-style darkening of the wet path: one pole at 8 kHz
        Self {
            prev: 0.0,
            alpha: 1.0 - (-2.0 * PI * 8000.0 / sample_rate).exp(),
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        self.prev += self.alpha * (input - self.prev);
        self.prev
    }
}

impl HighPassFilter {
    fn new(sample_rate: f32) -> Self {
        Self {
            prev_input: 0.0,
            prev_output: 0.0,
            cutoff: 20.0 / sample_rate,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let alpha = 1.0 / (1.0 + self.cutoff);
        let output = alpha * (self.prev_output + input - self.prev_input);
        self.prev_input = input;
        self.prev_output = output;
        output
    }
}

impl NoiseGenerator {
    fn new() -> Self {
        Self {
            level: 0.0005,
            prev: 0.0,
        }
    }

    fn generate(&mut self) -> f32 {
        let mut rng = rand::thread_rng();
        let new_noise = rng.gen_range(-self.level..self.level);
        let output = (self.prev + new_noise) * 0.5;
        self.prev = new_noise;
        output
    }
}

impl Saturation {
    fn new() -> Self {
        Self {
            drive: 1.2,
        }
    }

    fn process(&self, input: f32) -> f32 {
        (input * self.drive).tanh()
    }
}

impl Voice {
    fn new(rate_left: f32, rate_right: f32, depth: f32) -> Self {
        Self {
            phase_left: rand::thread_rng().gen(),
            phase_right: rand::thread_rng().gen(),
            rate_left,
            rate_right,
            depth,
            smooth_depth: depth,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Song automation re-asserts chorus_mode with every plain set; the
    /// switch must treat a no-op re-assert as nothing — not rebuild the
    /// BBD voices (resetting their state mid-note) and not stomp a
    /// chorus_mix override. The setter-idempotence house rule.
    #[test]
    fn reasserting_the_same_mode_is_a_no_op() {
        let mut chorus = Chorus::new(48000.0);
        chorus.set_mode(ChorusMode::II);
        chorus.set_mix(0.12);
        for n in 0..4800 {
            let x = (n as f32 * 0.05).sin() * 0.4;
            chorus.process(x, x);
        }
        let phase_before = chorus.voices[0].phase_left;
        chorus.set_mode(ChorusMode::II);
        assert_eq!(chorus.wet_dry_mix, 0.12, "mix override must survive");
        assert_eq!(chorus.voices[0].phase_left, phase_before, "voices must not rebuild");
    }

    /// Regression: a mode switch rebuilds the voices from the mode preset,
    /// and the rate/depth setters early-return on an unchanged value. The
    /// combination used to leave both knobs DEAD after every switch throw —
    /// re-asserting the same knob position (which song automation and the
    /// UI's apply-all do every block) changed nothing, so `chorus_depth 1.0`
    /// silently played at the preset's 0.4-0.7 ms.
    #[test]
    fn rate_and_depth_knobs_survive_a_mode_switch() {
        let mut chorus = Chorus::new(48000.0);
        chorus.set_mode(ChorusMode::II);
        chorus.set_rate(4.0);
        chorus.set_depth(0.9);

        chorus.set_mode(ChorusMode::IV);
        // ...automation re-asserts the same values on the next block
        chorus.set_rate(4.0);
        chorus.set_depth(0.9);

        assert_eq!(chorus.voices.len(), 4, "mode IV still builds four voices");
        for v in &chorus.voices {
            // 9 ms +-10% per-voice detune, NOT the 4-7 ms mode preset
            assert!(
                (0.0081..=0.0099).contains(&v.depth),
                "depth knob was discarded by the switch: {}",
                v.depth
            );
            // 4 Hz +-10%, NOT the mode's 0.5/0.75/1.0/1.25 Hz preset
            assert!(
                (3.6..=4.4).contains(&v.rate_left) && (3.6..=4.4).contains(&v.rate_right),
                "rate knob was discarded by the switch: {} / {}",
                v.rate_left,
                v.rate_right
            );
        }
    }

    /// ...but a knob nobody has touched must not override anything: the
    /// mode presets are the hardware's character and have to survive.
    #[test]
    fn untouched_knobs_leave_the_mode_preset_alone() {
        let mut chorus = Chorus::new(48000.0);
        chorus.set_mode(ChorusMode::III);
        assert_eq!(chorus.voices.len(), 2);
        for v in &chorus.voices {
            assert_eq!(v.depth, 0.0037, "mode III's own depth preset");
        }
        assert_eq!(chorus.voices[0].rate_left, 0.513);
        assert_eq!(chorus.voices[1].rate_left, 0.863);
    }

    /// A single non-finite sample used to circulate in the BBD feedback
    /// line forever — the chorus never produced audio again for the life of
    /// the process. It must cost one sample, not the session.
    #[test]
    fn a_nan_does_not_kill_the_bbd_line() {
        let mut chorus = Chorus::new(48000.0);
        chorus.set_mode(ChorusMode::III);
        chorus.process(f32::NAN, f32::INFINITY);
        chorus.process_with_send(0.0, 0.0, f32::NAN, f32::NAN);
        let mut energy = 0.0f32;
        for n in 0..48000 {
            let x = (2.0 * PI * 220.0 * n as f32 / 48000.0).sin() * 0.5;
            let (l, r) = chorus.process(x, x);
            assert!(l.is_finite() && r.is_finite(), "poisoned at sample {n}");
            if n > 4800 {
                energy += l * l;
            }
        }
        assert!(energy > 1.0, "chorus should be passing audio again: {energy}");
    }

    #[test]
    fn off_is_transparent() {
        let mut chorus = Chorus::new(48000.0);
        chorus.set_mode(ChorusMode::Off);
        for n in 0..1000 {
            let x = (n as f32 * 0.03).sin() * 0.8;
            let (l, r) = chorus.process(x, x);
            assert_eq!(l, x);
            assert_eq!(r, x);
        }
    }

    /// The depth knob is 0..1 and must map to a few milliseconds of LFO
    /// swing, never past the delay buffer (it used to be taken as seconds).
    #[test]
    fn full_depth_stays_bounded() {
        let mut chorus = Chorus::new(48000.0);
        chorus.set_mode(ChorusMode::IV);
        chorus.set_rate(6.0);
        chorus.set_depth(1.0);
        let mut peak = 0.0f32;
        for n in 0..96000 {
            let x = (2.0 * PI * 440.0 * n as f32 / 48000.0).sin() * 0.5;
            let (l, r) = chorus.process(x, x);
            assert!(l.is_finite() && r.is_finite());
            peak = peak.max(l.abs()).max(r.abs());
        }
        assert!(peak > 0.1 && peak <= 1.0, "peak out of range: {peak}");
    }
}