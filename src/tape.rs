// src/tape.rs — the Ferric engine
//
// A depth-resolved, explicitly-biased physical simulation of a compact
// cassette. Where previous real-time tape emulations model the oxide as a
// single hysteresis curve and design the loss effects as EQ, this engine
// simulates the mechanisms and lets the effects emerge:
//
//   TRANSPORT      wow (pinch roller, 1.17 Hz at 4.76 cm/s), capstan flutter
//                  (5.05 Hz), 1/f speed drift, random jitter — plus a
//                  stick-slip friction relaxation oscillator (Stribeck curve,
//                  ~3.4 kHz) for scrape flutter: the actual mechanism of tape
//                  squeal, not filtered noise. One modulated delay shared by
//                  both channels; azimuth error skews the right channel.
//
//   RECORD HEAD    the program is pre-emphasized (120 us curve plus a
//                  thickness-compensation shelf the deck derives from its own
//                  tape geometry, like a real alignment), upsampled 8x
//                  through polyphase half-band allpass cascades, and summed
//                  with an explicit 50 kHz AC bias oscillator.
//
//   THE OXIDE      simulated as N depth sublayers, each with its own
//                  Jiles-Atherton hysteresis state (differential
//                  magnetization with true remanence and minor loops, RK2
//                  along the field path). The head field decays with depth,
//                  so deep layers run underbiased and record poorly — which
//                  is WHY tape has thickness loss; here it emerges instead of
//                  being an EQ. Barkhausen noise is generated from the
//                  physics: magnetization moves in discrete particle
//                  avalanches, so each layer contributes noise proportional
//                  to sqrt(|dM|). Bias cycling alone therefore produces the
//                  hiss floor, and program material modulates it — hiss and
//                  asperity noise from one mechanism, spectrally shaped by
//                  the same playback path as the music.
//
//   PLAYBACK HEAD  per-layer Wallace spacing loss f0 = v/(2*pi*d) at the
//                  oversampled rate (d = base spacing + layer depth + wear
//                  + dropout lift), then decimation, gap loss as literal
//                  flux averaging over the gap transit (fractional boxcar —
//                  the true sinc response), head-bump resonance, playback
//                  de-emphasis, DC servo.
//
//   THE REEL       Poisson-scheduled dropouts modeled as the tape lifting
//                  off the head (level dip AND spacing-loss rise together),
//                  print-through post-echo from adjacent winds, and
//                  low-frequency interchannel crosstalk.
//
// The deck calibrates itself: record EQ from layer geometry, makeup gain by
// recording a 1 kHz alignment tone through the full biased magnetic path and
// measuring what comes back (quadrature projection, so bias residue is
// rejected). All four knobs at zero leaves the unit transparent, and every
// parameter is freely automatable.

use rand::Rng;
use std::f32::consts::PI;

// --- Transport ---
/// Cassette tape speed in um/s (1-7/8 ips).
const TAPE_SPEED_UM_S: f32 = 47600.0;
/// Pinch-roller rotation: 47.6 mm/s over a ~13 mm roller.
const WOW_HZ: f32 = 1.17;
/// Capstan rotation: 47.6 mm/s over a ~3 mm capstan.
const FLUTTER_HZ: f32 = 5.05;
const CENTER_DELAY_S: f32 = 0.006;
/// +-3.5 ms at 1.17 Hz ~= +-44 cents of pitch swing at full wow.
const WOW_DEPTH_S: f32 = 0.0035;
const DRIFT_DEPTH_S: f32 = 0.0015;
const FLUTTER_DEPTH_S: f32 = 0.00012;
const JITTER_DEPTH_S: f32 = 0.00008;
/// Scrape-flutter delay contribution per unit of oscillator displacement.
const SCRAPE_DEPTH_S: f32 = 0.0000012;

// --- Magnetics ---
const OS: usize = 8;
const N_LAYERS: usize = 3;
/// Depth of each oxide sublayer's center below the surface, in um.
const LAYER_DEPTH_UM: [f32; N_LAYERS] = [0.4, 1.7, 3.5];
/// Head-field decay into the coating: H(d) = H0 / (1 + d / FIELD_DEPTH_UM).
const FIELD_DEPTH_UM: f32 = 1.2;
/// Internal AC bias oscillator (real decks: 60-105 kHz; representable here
/// at the 8x internal rate and fully removed by the decimation cascade).
const BIAS_FREQ_HZ: f32 = 50000.0;
const BIAS_AMP: f32 = 0.30;
/// Barkhausen noise scale: particle-avalanche noise per unit sqrt(|dM|).
const BARKHAUSEN: f32 = 0.006;

// --- Playback ---
/// Playback head gap length, um.
const GAP_UM: f32 = 1.2;
/// Effective head-to-tape spacing, um: fresh/clean vs worn/dirty.
const SPACING_NEW_UM: f32 = 0.05;
const SPACING_WORN_UM: f32 = 1.5;
/// Extra spacing while the tape lifts off the head during a dropout.
const SPACING_DROPOUT_UM: f32 = 8.0;
/// LF interchannel crosstalk (track fringing), linear gain below ~300 Hz.
const CROSSTALK: f32 = 0.02;
/// Print-through: adjacent-wind echo delay and base level.
const PRINT_DELAY_S: f32 = 1.35;
const PRINT_LEVEL: f32 = 0.0035;

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
    scrape: ScrapeOscillator,
    azimuth_skew: f32, // right-channel read offset, samples

    // Record side
    pre_emphasis: [OnePoleHighPass; 2],
    thickness_comp: [OnePoleHighPass; 2],
    thickness_gain: f32,
    field_scale: f32,
    makeup: f32,

    // The oxide
    bias_cos: f32,
    bias_sin: f32,
    bias_rot: (f32, f32),
    up1: [Halfband; 2],
    up2: [Halfband; 2],
    up3: [Halfband; 2],
    hysteresis: [[JilesAtherton; N_LAYERS]; 2],
    barkhausen_level: f32,
    wallace_alpha: [[f32; N_LAYERS]; 2],
    wallace_state: [[[f32; 2]; N_LAYERS]; 2],
    down1: [Halfband; 2],
    down2: [Halfband; 2],
    down3: [Halfband; 2],

    // The reel
    dropout_env: f32,
    dropout_target: f32,
    dropout_remaining: f32,
    next_dropout: f32,
    spacing_age_um: f32,
    azimuth_spacing_factor: f32,
    print_buffer: [Vec<f32>; 2],
    print_index: usize,
    print_lp: [OnePole; 2],
    print_level: f32,

    // Playback side
    gap_frac: f32,
    gap_prev: [f32; 2],
    head_bump: [PeakingFilter; 2],
    de_emphasis: [OnePoleHighPass; 2],
    dc_block: [OnePoleHighPass; 2],
    crosstalk_lp: [OnePole; 2],
    prev_out: (f32, f32),
}

impl Tape {
    pub fn new(sample_rate: f32) -> Self {
        let size = (sample_rate * CENTER_DELAY_S * 2.0) as usize + 8;
        let print_len = (sample_rate * PRINT_DELAY_S) as usize + 1;
        let os_rate = sample_rate * OS as f32;
        let w = 2.0 * PI * BIAS_FREQ_HZ / os_rate;

        // Gap loss as flux averaging over the gap transit time. The transit
        // is ~1.2 samples at 48 kHz, so a fractional two-tap boxcar realizes
        // the physical integral (and its sinc rolloff) almost exactly.
        let gap_samples = (GAP_UM / TAPE_SPEED_UM_S) * sample_rate;
        let gap_frac = (gap_samples - 1.0).clamp(0.0, 1.0);

        // Self-alignment, step 1: derive the record EQ thickness shelf from
        // this deck's own layer geometry, the way a technician trims record
        // EQ until the system measures flat. Target: unity at 10 kHz for a
        // fresh tape, given the per-layer Wallace losses.
        let response_10k: f32 = (0..N_LAYERS)
            .map(|l| {
                let d = SPACING_NEW_UM + LAYER_DEPTH_UM[l];
                let f0 = TAPE_SPEED_UM_S / (2.0 * PI * d);
                let per_pole = 1.0 / (1.0 + (10000.0 / f0).powi(2));
                per_pole // two cascaded poles => squared magnitude of one
            })
            .sum::<f32>()
            / N_LAYERS as f32;
        let boost = (1.0 / response_10k.max(1e-3)).sqrt();
        let hp_mag_10k = {
            let r = 10000.0f32 / 4000.0;
            r / (1.0 + r * r).sqrt()
        };
        let thickness_gain = ((boost - 1.0) / hp_mag_10k).clamp(0.0, 4.0);

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
            scrape: ScrapeOscillator::new(sample_rate),
            azimuth_skew: 0.0,
            // Standard cassette playback time constant: 120 us (~1326 Hz)
            pre_emphasis: [OnePoleHighPass::new(sample_rate, 1326.0); 2],
            thickness_comp: [OnePoleHighPass::new(sample_rate, 4000.0); 2],
            thickness_gain,
            field_scale: 0.0,
            makeup: 1.0,
            bias_cos: 1.0,
            bias_sin: 0.0,
            bias_rot: (w.cos(), w.sin()),
            up1: [Halfband::new(); 2],
            up2: [Halfband::new(); 2],
            up3: [Halfband::new(); 2],
            hysteresis: [[JilesAtherton::new(); N_LAYERS]; 2],
            barkhausen_level: 0.0,
            wallace_alpha: [[1.0; N_LAYERS]; 2],
            wallace_state: [[[0.0; 2]; N_LAYERS]; 2],
            down1: [Halfband::new(); 2],
            down2: [Halfband::new(); 2],
            down3: [Halfband::new(); 2],
            dropout_env: 1.0,
            dropout_target: 1.0,
            dropout_remaining: 0.0,
            next_dropout: f32::MAX,
            spacing_age_um: SPACING_NEW_UM,
            azimuth_spacing_factor: 1.0,
            print_buffer: [vec![0.0; print_len], vec![0.0; print_len]],
            print_index: 0,
            print_lp: [OnePole::new(sample_rate, 2500.0); 2],
            print_level: 0.0,
            gap_frac,
            gap_prev: [0.0; 2],
            head_bump: [PeakingFilter::new(); 2],
            de_emphasis: [OnePoleHighPass::new(sample_rate, 1326.0); 2],
            dc_block: [OnePoleHighPass::new(sample_rate, 10.0); 2],
            crosstalk_lp: [OnePole::new(sample_rate, 300.0); 2],
            prev_out: (0.0, 0.0),
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
        self.field_scale = 0.05 + 0.22 * self.drive;

        // Self-alignment, step 2: record a small 1 kHz tone through the full
        // biased multi-layer magnetic path and set makeup gain from what
        // comes back. Quadrature projection rejects the bias residue.
        // Runs only on knob moves, never per sample.
        self.makeup = calibrate_makeup(self.field_scale, self.sample_rate);

        // Head bump only shows up once the tape is actually being hit
        let bump_db = 3.0 * self.drive;
        for filter in &mut self.head_bump {
            filter.set_peaking(self.sample_rate, 60.0, 1.2, bump_db);
        }
    }

    fn update_age(&mut self) {
        self.spacing_age_um =
            SPACING_NEW_UM + (SPACING_WORN_UM - SPACING_NEW_UM) * self.age;
        // Azimuth error: worse effective spacing on the outer track...
        self.azimuth_spacing_factor = 1.0 + 0.5 * self.age;
        // ...and an interchannel timing skew, up to ~0.1 ms
        self.azimuth_skew = self.age * 0.0001 * self.sample_rate;

        // Particle quality: worn oxide switches in coarser avalanches
        self.barkhausen_level = BARKHAUSEN * (0.4 + 1.6 * self.age * self.age);

        self.print_level = PRINT_LEVEL * self.age * self.age;

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
        let scrape_amp = self.flutter * (0.2 + 0.8 * self.age);
        let delay_seconds = CENTER_DELAY_S
            + wow_sq * WOW_DEPTH_S * (2.0 * PI * self.wow_phase).sin()
            + wow_sq * DRIFT_DEPTH_S * self.drift.next()
            + flutter_sq * FLUTTER_DEPTH_S * (2.0 * PI * self.flutter_phase).sin()
            + flutter_sq * JITTER_DEPTH_S * self.jitter.next()
            + scrape_amp * SCRAPE_DEPTH_S * self.scrape.next();
        let max_delay = self.size as f32 - 4.0;
        let delay = (delay_seconds * self.sample_rate).clamp(2.0, max_delay);
        let delay_right = (delay + self.azimuth_skew).clamp(2.0, max_delay);

        let left = read_fractional(&self.buffer_left, self.write, delay, self.size);
        let right = read_fractional(&self.buffer_right, self.write, delay_right, self.size);

        // Transport-only mode: nothing is being recorded, pass the wobbled
        // program straight through
        if self.drive + self.age < 5e-3 {
            self.prev_out = (left, right);
            return (left, right);
        }

        // --- Dropouts: the tape lifts across its full width ---
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

        // --- Per-layer Wallace spacing loss, f0 = v / (2 pi d). Dropout lift
        // raises d for every layer while it lasts ---
        let lift = (1.0 - self.dropout_env) * SPACING_DROPOUT_UM;
        let os_rate = self.sample_rate * OS as f32;
        for ch in 0..2 {
            let az = if ch == 1 { self.azimuth_spacing_factor } else { 1.0 };
            for l in 0..N_LAYERS {
                let d = (self.spacing_age_um + LAYER_DEPTH_UM[l] + lift) * az;
                let f0 = TAPE_SPEED_UM_S / (2.0 * PI * d);
                self.wallace_alpha[ch][l] = one_pole_alpha(os_rate, f0);
            }
        }

        // --- Bias oscillator: one strip of tape, one bias field ---
        let mut bias_sub = [0.0f32; OS];
        for b in bias_sub.iter_mut() {
            let (c, s) = (self.bias_cos, self.bias_sin);
            self.bias_cos = c * self.bias_rot.0 - s * self.bias_rot.1;
            self.bias_sin = s * self.bias_rot.0 + c * self.bias_rot.1;
            let g = 1.5 - 0.5 * (self.bias_cos * self.bias_cos + self.bias_sin * self.bias_sin);
            self.bias_cos *= g;
            self.bias_sin *= g;
            *b = self.bias_sin * BIAS_AMP;
        }

        let mut out = [left, right];
        for (ch, sample) in out.iter_mut().enumerate() {
            *sample = self.process_channel(*sample, ch, &bias_sub);
            if !sample.is_finite() {
                // Safety net for a live audio path: flush and go quiet for
                // one sample rather than emit garbage
                self.reset_channel(ch);
                *sample = 0.0;
            }
        }

        // --- LF crosstalk: track fringing bleeds bass between channels ---
        let bleed_into_l = self.crosstalk_lp[0].process(self.prev_out.1) * CROSSTALK;
        let bleed_into_r = self.crosstalk_lp[1].process(self.prev_out.0) * CROSSTALK;
        out[0] += bleed_into_l;
        out[1] += bleed_into_r;

        self.prev_out = (out[0], out[1]);
        (out[0], out[1])
    }

    fn process_channel(&mut self, x: f32, ch: usize, bias_sub: &[f32; OS]) -> f32 {
        // --- Record side: 120 us emphasis + the deck's own thickness shelf ---
        let emphasized = x
            + 0.6 * self.pre_emphasis[ch].process(x)
            + self.thickness_gain * self.thickness_comp[ch].process(x);

        // --- Upsample 8x (polyphase half-band allpass cascade) ---
        let (a, b) = self.up1[ch].up(emphasized);
        let (c, d) = self.up2[ch].up(a);
        let (e, f) = self.up2[ch].up(b);
        let mut sub = [0.0f32; OS];
        for (i, s) in [c, d, e, f].into_iter().enumerate() {
            let (p, q) = self.up3[ch].up(s);
            sub[2 * i] = p;
            sub[2 * i + 1] = q;
        }

        // --- The oxide: bias + program through depth-resolved hysteresis ---
        let mut rng = rand::thread_rng();
        let mut mag = [0.0f32; OS];
        for s in 0..OS {
            let h_surface = sub[s] * self.field_scale + bias_sub[s];
            let mut acc = 0.0;
            for l in 0..N_LAYERS {
                let depth_factor = 1.0 / (1.0 + LAYER_DEPTH_UM[l] / FIELD_DEPTH_UM);
                let ja = &mut self.hysteresis[ch][l];
                let m_prev = ja.m;
                let m = ja.step(h_surface * depth_factor);
                // Barkhausen: magnetization moves in particle avalanches, so
                // the noise power rides on how much switching just happened
                let avalanche = (m - m_prev).abs().sqrt();
                let noise: f32 = rng.gen_range(-1.0f32..1.0);
                let m_noisy = m + noise * self.barkhausen_level * avalanche;
                // This layer's spacing loss, at the oversampled rate
                let st = &mut self.wallace_state[ch][l];
                let alpha = self.wallace_alpha[ch][l];
                st[0] += alpha * (m_noisy - st[0]);
                st[1] += alpha * (st[0] - st[1]);
                acc += st[1];
            }
            mag[s] = acc * (self.makeup / N_LAYERS as f32);
        }

        // --- Decimate back to base rate (bias vanishes in the cascade) ---
        let d4 = [
            self.down3[ch].down(mag[0], mag[1]),
            self.down3[ch].down(mag[2], mag[3]),
            self.down3[ch].down(mag[4], mag[5]),
            self.down3[ch].down(mag[6], mag[7]),
        ];
        let d2 = [
            self.down2[ch].down(d4[0], d4[1]),
            self.down2[ch].down(d4[2], d4[3]),
        ];
        let recorded = self.down1[ch].down(d2[0], d2[1]);

        // --- The reel: print-through from adjacent winds, then dropouts ---
        let echo = self.print_buffer[ch][self.print_index];
        self.print_buffer[ch][self.print_index] = recorded;
        if ch == 1 {
            self.print_index = (self.print_index + 1) % self.print_buffer[0].len();
        }
        let with_echo =
            recorded + self.print_lp[ch].process(echo) * self.print_level;
        let tape_signal = with_echo * self.dropout_env;

        // --- Playback head: gap flux averaging, bump; playback amp: EQ, DC ---
        let gapped =
            (tape_signal + self.gap_frac * self.gap_prev[ch]) / (1.0 + self.gap_frac);
        self.gap_prev[ch] = tape_signal;
        let bumped = self.head_bump[ch].process(gapped);
        let de_emphasized = bumped - 0.35 * self.de_emphasis[ch].process(bumped);
        self.dc_block[ch].process(de_emphasized)
    }

    fn reset_channel(&mut self, ch: usize) {
        self.hysteresis[ch] = [JilesAtherton::new(); N_LAYERS];
        self.wallace_state[ch] = [[0.0; 2]; N_LAYERS];
        self.up1[ch] = Halfband::new();
        self.up2[ch] = Halfband::new();
        self.up3[ch] = Halfband::new();
        self.down1[ch] = Halfband::new();
        self.down2[ch] = Halfband::new();
        self.down3[ch] = Halfband::new();
        self.gap_prev[ch] = 0.0;
        self.head_bump[ch] = {
            let mut f = PeakingFilter::new();
            f.set_peaking(self.sample_rate, 60.0, 1.2, 3.0 * self.drive);
            f
        };
        self.de_emphasis[ch] = OnePoleHighPass::new(self.sample_rate, 1326.0);
        self.dc_block[ch] = OnePoleHighPass::new(self.sample_rate, 10.0);
    }
}

/// Record a 1 kHz alignment tone through the full biased multi-layer
/// magnetic path and return the makeup gain that brings it back to unity.
fn calibrate_makeup(field_scale: f32, sample_rate: f32) -> f32 {
    let os_rate = sample_rate * OS as f32;
    let w = 2.0 * PI * BIAS_FREQ_HZ / os_rate;
    let rot = (w.cos(), w.sin());
    let (mut bc, mut bs) = (1.0f32, 0.0f32);
    let mut ja = [JilesAtherton::new(); N_LAYERS];

    let period = (os_rate / 1000.0) as usize;
    let total = period * 6;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    let mut count = 0usize;
    for n in 0..total {
        let c = bc * rot.0 - bs * rot.1;
        bs = bs * rot.0 + bc * rot.1;
        bc = c;
        let phase = 2.0 * PI * (n % period) as f32 / period as f32;
        let h_surface = 0.1 * phase.sin() * field_scale + bs * BIAS_AMP;
        let mut acc = 0.0;
        for l in 0..N_LAYERS {
            let depth_factor = 1.0 / (1.0 + LAYER_DEPTH_UM[l] / FIELD_DEPTH_UM);
            acc += ja[l].step(h_surface * depth_factor);
        }
        acc /= N_LAYERS as f32;
        if n >= total - 2 * period {
            re += (acc * phase.sin()) as f64;
            im += (acc * phase.cos()) as f64;
            count += 1;
        }
    }
    let amp = 2.0 * ((re * re + im * im).sqrt() as f32) / count as f32;
    (0.1 / amp.max(1e-5)).min(60.0)
}

// ---------------------------------------------------------------------------
// Jiles-Atherton hysteresis
// ---------------------------------------------------------------------------

/// Jiles-Atherton magnetic hysteresis, normalized to saturation
/// magnetization Ms = 1. Parameters follow published tape-oxide fits
/// (Chowdhury 2019, scaled by Ms): `a` domain density, `k` coercivity,
/// `c` reversibility, `alpha` mean-field coupling.
///
/// Quasi-static form: dM/dH depends only on the field path, not on time,
/// which is physically exact for audio-rate (and bias-rate) fields.
/// One RK2 step per oversampled sub-sample.
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

    #[inline]
    fn dm_dh(&self, m: f32, h: f32, delta: f32) -> f32 {
        let he = h + self.alpha * m;
        let x = he / self.a;
        let (man, lang_deriv) = langevin_pair(x);
        let dman_dh = lang_deriv / self.a;
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

    #[inline]
    fn step(&mut self, h: f32) -> f32 {
        let dh = h - self.h_prev;
        if dh.abs() < 1e-9 {
            self.h_prev = h;
            return self.m;
        }
        let delta = dh.signum();
        let k1 = self.dm_dh(self.m, self.h_prev, delta);
        let k2 = self.dm_dh(self.m + 0.5 * dh * k1, self.h_prev + 0.5 * dh, delta);
        self.m = (self.m + dh * k2).clamp(-1.2, 1.2);
        self.h_prev = h;
        self.m
    }
}

/// Fast rational tanh (accurate within the +-3 range it is used in).
#[inline]
fn fast_tanh(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// Langevin function L(x) = coth(x) - 1/x and its derivative, computed
/// together (they share coth via csch^2 = coth^2 - 1). L is the
/// anhysteretic magnetization curve of the oxide particles.
#[inline]
fn langevin_pair(x: f32) -> (f32, f32) {
    let ax = x.abs();
    if ax < 0.6 {
        // Series: no cancellation trouble near zero
        let x2 = x * x;
        (
            x * (1.0 / 3.0 - x2 / 45.0 + 2.0 * x2 * x2 / 945.0),
            1.0 / 3.0 - x2 / 15.0 + 2.0 * x2 * x2 / 189.0,
        )
    } else if ax > 6.0 {
        // Asymptotic: coth -> sign(x)
        (x.signum() - 1.0 / x, 1.0 / (x * x))
    } else {
        let coth = 1.0 / fast_tanh(x);
        let inv_x = 1.0 / x;
        let l = coth - inv_x;
        // L'(x) = 1/x^2 - csch^2 = 1/x^2 + 1 - coth^2
        let lp = inv_x * inv_x + 1.0 - coth * coth;
        (l, lp.max(0.0))
    }
}

// ---------------------------------------------------------------------------
// Stick-slip scrape-flutter oscillator
// ---------------------------------------------------------------------------

/// Tape squeal is a friction relaxation oscillation: the tape element at the
/// head sticks and slips against a friction force that falls with sliding
/// speed (Stribeck curve), pumping energy into the tape's longitudinal
/// resonance (~3.4 kHz for cassette). This models exactly that: a resonator
/// with velocity-dependent friction whose negative damping self-excites and
/// whose nonlinearity limits the amplitude. Output is the position error of
/// the tape at the head, in normalized units.
struct ScrapeOscillator {
    u: f32, // displacement
    w: f32, // velocity, normalized to tape speed
    dt: f32,
    w0_sq: f32,
    friction_at_rest: f32,
}

const SCRAPE_HZ: f32 = 3400.0;
const MU_DELTA: f32 = 0.3; // static-over-kinetic friction excess
const STRIBECK_V: f32 = 0.35; // friction decay speed, in tape speeds
const SCRAPE_FORCE: f32 = 12500.0;
const SCRAPE_DAMPING: f32 = 400.0;

impl ScrapeOscillator {
    fn new(sample_rate: f32) -> Self {
        let w0 = 2.0 * PI * SCRAPE_HZ;
        Self {
            u: 0.0,
            w: 0.0,
            dt: 1.0 / sample_rate,
            w0_sq: w0 * w0,
            friction_at_rest: MU_DELTA * (-1.0 / STRIBECK_V).exp(),
        }
    }

    fn next(&mut self) -> f32 {
        // Sliding speed of tape over head, normalized (1 = nominal)
        let v_rel = (1.0 + self.w).max(0.0);
        let friction = MU_DELTA * (-v_rel / STRIBECK_V).exp() - self.friction_at_rest;
        let seed: f32 = rand::thread_rng().gen_range(-1.0f32..1.0) * 1e-3;
        // Semi-implicit Euler: stable for the ~3.4 kHz resonance at audio rates
        self.w += self.dt
            * (-self.w0_sq * self.u - SCRAPE_FORCE * friction - SCRAPE_DAMPING * self.w)
            + seed;
        self.w = self.w.clamp(-0.9, 0.9);
        self.u = (self.u + self.dt * self.w * self.w0_sq.sqrt()).clamp(-3.0, 3.0);
        self.u
    }
}

// ---------------------------------------------------------------------------
// Polyphase half-band resampler (allpass cascade)
// ---------------------------------------------------------------------------

/// Classic elliptic half-band built from two allpass branches, two
/// first-order (in the branch domain) sections each: ~69 dB stopband.
/// The same structure serves as a 2x interpolator (`up`) and a 2x
/// decimator (`down`).
#[derive(Clone, Copy)]
struct Halfband {
    a: [AllpassSection; 2],
    b: [AllpassSection; 2],
}

#[derive(Clone, Copy, Default)]
struct AllpassSection {
    c: f32,
    x1: f32,
    y1: f32,
}

impl AllpassSection {
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.c * (x - self.y1) + self.x1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

impl Halfband {
    fn new() -> Self {
        let mk = |c: f32| AllpassSection { c, x1: 0.0, y1: 0.0 };
        Self {
            a: [mk(0.079_866_43), mk(0.545_353_65)],
            b: [mk(0.283_829_34), mk(0.834_411_89)],
        }
    }

    /// One low-rate sample in, two high-rate samples out.
    #[inline]
    fn up(&mut self, x: f32) -> (f32, f32) {
        let mut p = x;
        for s in &mut self.a {
            p = s.process(p);
        }
        let mut q = x;
        for s in &mut self.b {
            q = s.process(q);
        }
        (q, p)
    }

    /// Two high-rate samples in, one low-rate sample out.
    #[inline]
    fn down(&mut self, x0: f32, x1: f32) -> f32 {
        let mut p = x0;
        for s in &mut self.a {
            p = s.process(p);
        }
        let mut q = x1;
        for s in &mut self.b {
            q = s.process(q);
        }
        0.5 * (p + q)
    }
}

// ---------------------------------------------------------------------------
// Small shared pieces
// ---------------------------------------------------------------------------

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
/// +-1: the slow 1/f-ish component of transport speed error.
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
struct OnePole {
    state: f32,
    alpha: f32,
}

impl OnePole {
    fn new(sample_rate: f32, cutoff: f32) -> Self {
        Self {
            state: 0.0,
            alpha: one_pole_alpha(sample_rate, cutoff),
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        self.state += self.alpha * (input - self.state);
        self.state
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

    #[inline]
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

// ---------------------------------------------------------------------------
// Tests: every physical claim above gets measured
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48000.0;

    fn sine(n: usize, freq: f32) -> impl Iterator<Item = f32> {
        (0..n).map(move |i| (2.0 * PI * freq * i as f32 / FS).sin() * 0.5)
    }

    /// Project a signal onto harmonics 1..=n_harm of f0 and return
    /// (fundamental rms, residual rms after removing all n_harm harmonics).
    fn harmonic_split(signal: &[f32], f0: f32, n_harm: usize) -> (f32, f32) {
        let n = signal.len();
        let mut residual: Vec<f32> = signal.to_vec();
        let mut fundamental_rms = 0.0f32;
        for k in 1..=n_harm {
            let wk = 2.0 * PI * f0 * k as f32 / FS;
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, &x) in signal.iter().enumerate() {
                let ph = wk * i as f32;
                re += (x * ph.sin()) as f64;
                im += (x * ph.cos()) as f64;
            }
            let (a, b) = ((2.0 * re / n as f64) as f32, (2.0 * im / n as f64) as f32);
            for (i, r) in residual.iter_mut().enumerate() {
                let ph = wk * i as f32;
                *r -= a * ph.sin() + b * ph.cos();
            }
            if k == 1 {
                fundamental_rms = ((a * a + b * b) / 2.0).sqrt();
            }
        }
        let res_rms =
            (residual.iter().map(|x| (x * x) as f64).sum::<f64>() / n as f64).sqrt() as f32;
        (fundamental_rms, res_rms)
    }

    fn rms(signal: &[f32]) -> f32 {
        (signal.iter().map(|x| (x * x) as f64).sum::<f64>() / signal.len() as f64).sqrt()
            as f32
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
        // The signature of magnetic recording: sweep the field up and back
        // to zero, and magnetization remains
        let mut ja = JilesAtherton::new();
        for i in 0..=1000 {
            ja.step(i as f32 / 1000.0);
        }
        for i in 0..=1000 {
            ja.step(1.0 - i as f32 / 1000.0);
        }
        assert!(ja.m > 0.05, "no remanence: M = {}", ja.m);
        assert!(ja.m < 1.0);
    }

    #[test]
    fn bias_linearizes_recording() {
        // The whole point of AC bias: recording through raw hysteresis is
        // grossly distorted, but with bias the transfer goes clean. Distortion
        // must be low at moderate drive and rise sharply when slammed.
        let thd_at = |drive: f32, amp: f32| {
            let mut tape = Tape::new(FS);
            tape.set_drive(drive);
            let mut out = Vec::with_capacity(24000);
            for (i, x) in sine(48000, 1000.0).enumerate() {
                let (l, _) = tape.process(x * amp * 2.0, x * amp * 2.0);
                if i >= 24000 {
                    out.push(l);
                }
            }
            let (fund, resid) = harmonic_split(&out, 1000.0, 20);
            resid / fund.max(1e-9)
        };
        let clean = thd_at(0.3, 0.5);
        let slammed = thd_at(1.0, 0.9);
        assert!(clean < 0.2, "biased recording should be clean, THD+N {}", clean);
        assert!(
            slammed > clean * 1.5,
            "drive must saturate: clean {} slammed {}",
            clean,
            slammed
        );
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
            (0.5..1.8).contains(&gain),
            "small-signal gain should be near unity, got {}",
            gain
        );
    }

    #[test]
    fn saturation_is_bounded_and_finite() {
        let mut tape = Tape::new(FS);
        tape.set_drive(1.0);
        tape.set_age(0.7);
        tape.set_wow(0.8);
        tape.set_flutter(0.8);
        let mut peak = 0.0f32;
        for x in sine(48000, 220.0) {
            let (l, r) = tape.process(x * 2.0, x * 2.0); // hit it hard
            assert!(l.is_finite() && r.is_finite());
            peak = peak.max(l.abs());
        }
        assert!(peak > 0.05, "signal should pass through, peak {}", peak);
        assert!(peak < 2.5, "magnetization must stay bounded, peak {}", peak);
    }

    #[test]
    fn barkhausen_noise_modulates_with_signal() {
        // Real tape noise is not a constant hiss bed: program material
        // increases particle switching, so the noise floor breathes with the
        // signal. Measure residual noise with and without a tone.
        let residual = |with_tone: bool| {
            let mut tape = Tape::new(FS);
            tape.set_drive(0.4);
            tape.set_age(0.6);
            let mut out = Vec::with_capacity(24000);
            for (i, x) in sine(72000, 1000.0).enumerate() {
                let inp = if with_tone { x * 0.6 } else { 0.0 };
                let (l, _) = tape.process(inp, inp);
                if i >= 48000 {
                    out.push(l);
                }
            }
            let (_, resid) = harmonic_split(&out, 1000.0, 23);
            resid
        };
        let silent_floor = residual(false);
        let modulated = residual(true);
        assert!(silent_floor > 1e-6, "bias cycling must produce a hiss floor");
        assert!(
            modulated > silent_floor * 1.3,
            "noise must ride the signal: silent {} modulated {}",
            silent_floor,
            modulated
        );
    }

    #[test]
    fn aged_tape_hisses_and_dulls() {
        let mut tape = Tape::new(FS);
        tape.set_age(1.0);
        tape.set_drive(0.3);
        let mut samples = Vec::with_capacity(48000);
        let mut hp = OnePoleHighPass::new(FS, 9000.0);
        let mut high_energy = 0.0f32;
        for _ in 0..48000 {
            let (l, _) = tape.process(0.0, 0.0);
            let h = hp.process(l);
            high_energy += h * h;
            samples.push(l);
        }
        let hiss = rms(&samples);
        let energy = samples.iter().map(|x| x * x).sum::<f32>();
        assert!(hiss > 1e-5 && hiss < 5e-3, "hiss rms {}", hiss);
        assert!(
            high_energy < energy * 0.2,
            "hiss must be dull, not white: high fraction {}",
            high_energy / energy.max(1e-12)
        );

        // Emergent frequency response: fresh tape holds its top end (the
        // deck's self-derived record EQ compensates layer losses), worn
        // tape loses it
        let hf_ratio = |age: f32| {
            let gain_at = |freq: f32| {
                let mut tape = Tape::new(FS);
                tape.set_age(age);
                tape.set_drive(0.3);
                let mut in_e = 0.0f32;
                let mut out_e = 0.0f32;
                for (i, x) in sine(48000, freq).enumerate() {
                    let (l, _) = tape.process(x, x);
                    if i > 12000 {
                        in_e += x * x;
                        out_e += l * l;
                    }
                }
                (out_e / in_e).sqrt()
            };
            gain_at(10000.0) / gain_at(400.0)
        };
        let fresh = hf_ratio(0.0);
        let worn = hf_ratio(1.0);
        assert!(fresh > 0.25, "fresh tape should keep its top end: {}", fresh);
        assert!(worn < fresh * 0.5, "worn tape must dull: fresh {} worn {}", fresh, worn);
    }

    #[test]
    fn wow_modulates_pitch() {
        let period_spread = |wow: f32| {
            let mut tape = Tape::new(FS);
            tape.set_wow(wow);
            tape.set_drive(0.05);
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

    #[test]
    fn scrape_oscillator_self_excites_near_resonance() {
        let mut scrape = ScrapeOscillator::new(FS);
        let mut prev = 0.0f32;
        let mut crossings = 0usize;
        let mut sum_sq = 0.0f32;
        let n = 48000;
        for i in 0..n * 2 {
            let u = scrape.next();
            assert!(u.is_finite());
            if i >= n {
                sum_sq += u * u;
                if prev <= 0.0 && u > 0.0 {
                    crossings += 1;
                }
                prev = u;
            }
        }
        let u_rms = (sum_sq / n as f32).sqrt();
        assert!(u_rms > 1e-4, "oscillator must self-excite, rms {}", u_rms);
        assert!(u_rms < 3.0, "and stay bounded, rms {}", u_rms);
        assert!(
            (1800..6000).contains(&crossings),
            "should oscillate near the tape resonance, got {} Hz",
            crossings
        );
    }

    #[test]
    fn crosstalk_bleeds_bass_between_channels() {
        let mut tape = Tape::new(FS);
        tape.set_drive(0.4);
        let mut l_e = 0.0f32;
        let mut r_e = 0.0f32;
        for (i, x) in sine(48000, 150.0).enumerate() {
            let (l, r) = tape.process(x, 0.0); // left only
            if i > 12000 {
                l_e += l * l;
                r_e += r * r;
            }
        }
        let bleed = (r_e / l_e.max(1e-12)).sqrt();
        assert!(
            (0.002..0.2).contains(&bleed),
            "LF crosstalk should sit tens of dB down, got {}",
            bleed
        );
    }

    #[test]
    fn print_through_echoes_after_the_wind_delay() {
        let mut tape = Tape::new(FS);
        tape.set_drive(0.5);
        tape.set_age(0.9);
        let burst_len = (0.3 * FS) as usize;
        let total = (2.6 * FS) as usize;
        let mut out = Vec::with_capacity(total);
        for i in 0..total {
            let x = if i < burst_len {
                (2.0 * PI * 300.0 * i as f32 / FS).sin() * 0.5
            } else {
                0.0
            };
            let (l, _) = tape.process(x, x);
            out.push(l);
        }
        // Echo of the burst arrives PRINT_DELAY_S after the burst itself
        let echo_start = (PRINT_DELAY_S * FS) as usize;
        let echo = rms(&out[echo_start..echo_start + burst_len]);
        let quiet = rms(&out[echo_start - burst_len..echo_start]);
        assert!(
            echo > quiet * 1.5,
            "print-through echo should rise above the floor: echo {} floor {}",
            echo,
            quiet
        );
        assert!(echo < 0.05, "but stay a ghost, got {}", echo);
    }

    #[test]
    fn runs_faster_than_realtime() {
        let mut tape = Tape::new(FS);
        tape.set_wow(0.7);
        tape.set_flutter(0.7);
        tape.set_drive(0.7);
        tape.set_age(0.7);
        let n = if cfg!(debug_assertions) { 4800 } else { 96000 };
        let start = std::time::Instant::now();
        let mut acc = 0.0f32;
        for x in sine(n, 440.0) {
            let (l, r) = tape.process(x, x);
            acc += l + r;
        }
        assert!(acc.is_finite());
        let elapsed = start.elapsed().as_secs_f32();
        let audio_time = n as f32 / FS;
        if !cfg!(debug_assertions) {
            assert!(
                elapsed < audio_time * 0.5,
                "full engine must run well under realtime: {}s for {}s of audio",
                elapsed,
                audio_time
            );
        }
    }
}
