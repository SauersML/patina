// src/tape.rs
//
// A physical compact-cassette model. Signal path mirrors a real deck:
//
//   transport      wow (pinch roller, ~1.17 Hz at 4.76 cm/s) + 1/f speed
//                  drift + flutter (capstan revolution, ~5.05 Hz) + random
//                  jitter, applied as one modulated delay shared by both
//                  channels — they sit on the same strip of tape. Azimuth
//                  error skews the right channel's read position slightly.
//   record side    pre-emphasis (120 µs curve), then Jiles-Atherton
//                  hysteresis — the differential magnetization model of the
//                  oxide, with true remanence and minor loops — integrated
//                  with RK2 along the field path (quasi-static, so the loop
//                  shape is rate-independent, as it physically is).
//   tape itself    bias-noise floor plus asperity (modulation) noise that
//                  rides on the magnetization level, and Poisson-scheduled
//                  oxide dropouts. A dropout is the tape lifting off the
//                  head, so it both dips the level and raises the spacing
//                  loss while it lasts.
//   playback head  head-bump resonance near 60 Hz, and spacing loss per
//                  Wallace's law: f0 = v / (2*pi*d), where the effective
//                  spacing d grows with AGE (wear, dirt) and momentarily
//                  during dropouts. All tape-domain noise passes through
//                  these same filters — that is what makes real hiss sound
//                  like tape and not like white noise.
//   playback amp   de-emphasis and a DC servo.
//
// All four knobs at zero leaves the unit transparent (the delay line still
// runs so engaging a knob never reads a stale buffer), which also makes every
// parameter freely automatable from song files.

use rand::Rng;
use std::f32::consts::PI;

/// Cassette tape speed in µm/s (1-7/8 ips).
const TAPE_SPEED_UM_S: f32 = 47600.0;
/// Pinch-roller rotation rate: 47.6 mm/s over a ~13 mm roller.
const WOW_HZ: f32 = 1.17;
/// Capstan rotation rate: 47.6 mm/s over a ~3 mm capstan.
const FLUTTER_HZ: f32 = 5.05;
/// Nominal head-to-read delay; modulation swings around this center.
const CENTER_DELAY_S: f32 = 0.006;
/// ±3.5 ms at 1.17 Hz ≈ ±44 cents of pitch swing at full wow.
const WOW_DEPTH_S: f32 = 0.0035;
const DRIFT_DEPTH_S: f32 = 0.0015;
const FLUTTER_DEPTH_S: f32 = 0.00012;
const JITTER_DEPTH_S: f32 = 0.00008;
/// Effective head-to-tape spacing in µm: fresh and clean vs worn and dirty.
const SPACING_NEW_UM: f32 = 0.05;
const SPACING_WORN_UM: f32 = 1.5;
/// Extra spacing while the tape lifts off the head during a dropout.
const SPACING_DROPOUT_UM: f32 = 8.0;

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
    azimuth_skew: f32, // right-channel read offset, in samples

    // Record side
    pre_emphasis: [OnePoleHighPass; 2],
    hysteresis: [JilesAtherton; 2],
    field_scale: f32,
    makeup: f32,

    // Tape itself
    hiss_level: f32,
    asperity_level: f32,
    level_follow: [f32; 2],
    dropout_env: f32,
    dropout_target: f32,
    dropout_remaining: f32,
    next_dropout: f32,

    // Playback head and amp
    head_bump: [PeakingFilter; 2],
    spacing_age_um: f32,
    azimuth_spacing_factor: f32,
    gap_alpha: [f32; 2],
    gap_state: [[f32; 2]; 2], // two cascaded poles per channel
    de_emphasis: [OnePoleHighPass; 2],
    dc_block: [OnePoleHighPass; 2],
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
            azimuth_skew: 0.0,
            // The standard cassette playback time constant is 120 µs
            // (~1326 Hz); pre/de-emphasis shelves pivot there.
            pre_emphasis: [OnePoleHighPass::new(sample_rate, 1326.0); 2],
            hysteresis: [JilesAtherton::new(); 2],
            field_scale: 0.0,
            makeup: 1.0,
            hiss_level: 0.0,
            asperity_level: 0.0,
            level_follow: [0.0; 2],
            dropout_env: 1.0,
            dropout_target: 1.0,
            dropout_remaining: 0.0,
            next_dropout: f32::MAX,
            head_bump: [PeakingFilter::new(); 2],
            spacing_age_um: SPACING_NEW_UM,
            azimuth_spacing_factor: 1.0,
            gap_alpha: [1.0; 2],
            gap_state: [[0.0; 2]; 2],
            de_emphasis: [OnePoleHighPass::new(sample_rate, 1326.0); 2],
            dc_block: [OnePoleHighPass::new(sample_rate, 10.0); 2],
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
        self.field_scale = 0.05 + 1.15 * self.drive;

        // Calibrate makeup gain so small signals come back at unity: run a
        // quasi-static minor loop at 10% level and measure the response.
        // Only runs on knob moves, never per sample.
        let mut ja = JilesAtherton::new();
        let amp = 0.1 * self.field_scale;
        let (mut lo, mut hi) = (0.0f32, 0.0f32);
        for i in 0..192 {
            let h = amp * (2.0 * PI * i as f32 / 64.0).sin();
            let m = ja.process(h);
            if i >= 128 {
                lo = lo.min(m);
                hi = hi.max(m);
            }
        }
        let response = (0.5 * (hi - lo)).max(1e-6);
        self.makeup = (0.1 / response).min(30.0);

        // Head bump only shows up once the tape is actually being hit
        let bump_db = 3.0 * self.drive;
        for filter in &mut self.head_bump {
            filter.set_peaking(self.sample_rate, 60.0, 1.2, bump_db);
        }
    }

    fn update_age(&mut self) {
        self.spacing_age_um =
            SPACING_NEW_UM + (SPACING_WORN_UM - SPACING_NEW_UM) * self.age;
        // Azimuth error: effective spacing is worse on the outer track
        self.azimuth_spacing_factor = 1.0 + 0.5 * self.age;
        // ...and it skews interchannel timing, up to ~0.1 ms
        self.azimuth_skew = self.age * 0.0001 * self.sample_rate;

        // Bias-noise floor and asperity (modulation) noise, in the magnetic
        // domain; the playback chain shapes their spectrum downstream
        self.hiss_level = self.age * self.age * 0.0015;
        self.asperity_level = self.age * 0.02;

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
        let max_delay = self.size as f32 - 4.0;
        let delay = (delay_seconds * self.sample_rate).clamp(2.0, max_delay);
        let delay_right = (delay + self.azimuth_skew).clamp(2.0, max_delay);

        let left = read_fractional(&self.buffer_left, self.write, delay, self.size);
        let right = read_fractional(&self.buffer_right, self.write, delay_right, self.size);

        // --- Oxide dropouts: shared, the tape lifts across its full width ---
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

        // --- Spacing loss (Wallace): f0 = v / (2*pi*d). Dropouts lift the
        // tape off the head, so they raise d while they last ---
        let lift = (1.0 - self.dropout_env) * SPACING_DROPOUT_UM;
        let spacing = self.spacing_age_um + lift;
        let f0 = TAPE_SPEED_UM_S / (2.0 * PI * spacing);
        self.gap_alpha[0] = one_pole_alpha(self.sample_rate, f0);
        self.gap_alpha[1] =
            one_pole_alpha(self.sample_rate, f0 / self.azimuth_spacing_factor);

        let mut out = [left, right];
        for (ch, sample) in out.iter_mut().enumerate() {
            *sample = self.process_channel(*sample, ch);
        }
        (out[0], out[1])
    }

    fn process_channel(&mut self, x: f32, ch: usize) -> f32 {
        // --- Record side: emphasis, then hysteresis in the magnetic domain ---
        let emphasized = x + 0.6 * self.pre_emphasis[ch].process(x);
        let m = self.hysteresis[ch].process(emphasized * self.field_scale) * self.makeup;

        // --- Tape itself: noise lives ON the tape, so it passes through the
        // playback filters with the signal. Asperity noise rides the level.
        self.level_follow[ch] += 0.004 * (m.abs() - self.level_follow[ch]);
        let noise_amp = self.hiss_level + self.asperity_level * self.level_follow[ch];
        let noise = rand::thread_rng().gen_range(-1.0..1.0f32) * noise_amp;
        let tape_signal = (m + noise) * self.dropout_env;

        // --- Playback head: bump resonance, then spacing loss ---
        let bumped = self.head_bump[ch].process(tape_signal);
        let alpha = self.gap_alpha[ch];
        self.gap_state[ch][0] += alpha * (bumped - self.gap_state[ch][0]);
        self.gap_state[ch][1] += alpha * (self.gap_state[ch][0] - self.gap_state[ch][1]);
        let dulled = self.gap_state[ch][1];

        // --- Playback amp: de-emphasis and DC servo ---
        let de_emphasized = dulled - 0.35 * self.de_emphasis[ch].process(dulled);
        self.dc_block[ch].process(de_emphasized)
    }
}

/// Jiles-Atherton magnetic hysteresis, normalized to saturation
/// magnetization Ms = 1. Parameters follow published tape-oxide fits
/// (Chowdhury 2019, scaled by Ms): `a` domain density, `k` coercivity,
/// `c` reversibility, `alpha` mean-field coupling.
///
/// Quasi-static form: dM/dH depends only on the field path, not on time,
/// which is physically exact for audio-rate fields. Integrated with RK2
/// over two substeps per sample.
#[derive(Clone, Copy)]
struct JilesAtherton {
    m: f32,
    h_prev: f32,
    a: f32,
    k: f32,
    c: f32,
    alpha: f32,
}

impl JilesAtherton {
    fn new() -> Self {
        Self {
            m: 0.0,
            h_prev: 0.0,
            a: 0.063,
            k: 0.077,
            c: 0.17,
            alpha: 1.6e-3,
        }
    }

    fn dm_dh(&self, m: f32, h: f32, delta: f32) -> f32 {
        let he = h + self.alpha * m;
        let man = langevin(he / self.a);
        let dman_dh = langevin_deriv(he / self.a) / self.a;
        let dm = man - m;
        // The irreversible term only acts when the field pushes the
        // magnetization toward the anhysteretic curve; the guard removes the
        // non-physical negative-susceptibility branch
        let irreversible = if dm * delta > 0.0 {
            let denom = (1.0 - self.c) * delta * self.k - self.alpha * dm;
            if denom.abs() < 1e-9 {
                0.0
            } else {
                (1.0 - self.c) * dm / denom
            }
        } else {
            0.0
        };
        irreversible + self.c * dman_dh
    }

    fn process(&mut self, h: f32) -> f32 {
        let dh = h - self.h_prev;
        if dh.abs() < 1e-12 {
            self.h_prev = h;
            return self.m;
        }
        let delta = dh.signum();
        let sub = dh * 0.5;
        let mut h_cur = self.h_prev;
        for _ in 0..2 {
            let k1 = self.dm_dh(self.m, h_cur, delta);
            let k2 = self.dm_dh(self.m + 0.5 * sub * k1, h_cur + 0.5 * sub, delta);
            self.m = (self.m + sub * k2).clamp(-1.0, 1.0);
            h_cur += sub;
        }
        self.h_prev = h;
        self.m
    }
}

/// Langevin function L(x) = coth(x) - 1/x: the anhysteretic magnetization
/// curve of the oxide particles.
fn langevin(x: f32) -> f32 {
    if x.abs() < 1e-3 {
        x / 3.0
    } else {
        1.0 / x.tanh() - 1.0 / x
    }
}

/// L'(x) = 1/x^2 - csch^2(x), with series and asymptotic guards.
fn langevin_deriv(x: f32) -> f32 {
    let ax = x.abs();
    if ax < 1e-2 {
        1.0 / 3.0 - x * x / 15.0
    } else if ax > 20.0 {
        1.0 / (x * x)
    } else {
        let s = x.sinh();
        1.0 / (x * x) - 1.0 / (s * s)
    }
}

fn one_pole_alpha(sample_rate: f32, cutoff: f32) -> f32 {
    (1.0 - (-2.0 * PI * cutoff / sample_rate).exp()).min(1.0)
}

fn read_fractional(buffer: &[f32], write: usize, delay: f32, size: usize) -> f32 {
    let read = (write as f32 - delay + size as f32) as usize % size;
    let frac = delay.fract();
    cubic_interpolate(
        &[
            buffer[(read + size - 1) % size],
            buffer[read],
            buffer[(read + 1) % size],
            buffer[(read + 2) % size],
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
    fn hysteresis_has_remanence() {
        // The signature of real magnetic tape: sweep the field up to
        // saturation and back to zero, and magnetization remains
        let mut ja = JilesAtherton::new();
        for i in 0..=1000 {
            ja.process(i as f32 / 1000.0);
        }
        for i in 0..=1000 {
            ja.process(1.0 - i as f32 / 1000.0);
        }
        assert!(ja.m > 0.05, "no remanence: M = {}", ja.m);
        assert!(ja.m < 1.0);
    }

    #[test]
    fn small_signals_pass_at_unity() {
        let mut tape = Tape::new(FS);
        tape.set_drive(0.4);
        let mut in_e = 0.0f32;
        let mut out_e = 0.0f32;
        for (i, x) in sine(96000, 440.0).enumerate() {
            let (l, _) = tape.process(x * 0.2, x * 0.2);
            if i > 24000 {
                in_e += (x * 0.2) * (x * 0.2);
                out_e += l * l;
            }
        }
        let gain = (out_e / in_e).sqrt();
        assert!(
            (0.6..1.6).contains(&gain),
            "small-signal gain should be near unity, got {}",
            gain
        );
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
        assert!(peak < 1.5, "magnetization must stay bounded, peak {}", peak);
    }

    #[test]
    fn aged_tape_hisses_and_dulls() {
        let mut tape = Tape::new(FS);
        tape.set_age(1.0);
        // Hiss: silence in, a noise floor out — but a dull one, since tape
        // noise passes through the playback losses too
        let mut energy = 0.0f32;
        let mut high_energy = 0.0f32;
        let mut hp = OnePoleHighPass::new(FS, 9000.0);
        for _ in 0..48000 {
            let (l, _) = tape.process(0.0, 0.0);
            energy += l * l;
            let h = hp.process(l);
            high_energy += h * h;
        }
        let hiss_rms = (energy / 48000.0).sqrt();
        assert!(hiss_rms > 1e-5 && hiss_rms < 3e-3, "hiss rms {}", hiss_rms);
        assert!(
            high_energy < energy * 0.2,
            "hiss should be dull, not white: high fraction {}",
            high_energy / energy
        );

        // Spacing loss: a 10 kHz tone loses far more level than a 200 Hz tone
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
