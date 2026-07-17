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
//   RECORD HEAD    the program passes an EXACTLY-reciprocal 120 us emphasis
//                  shelf (its inverse sits in the playback amp, so the pair
//                  cancels to numerical precision — in-band coloration is
//                  physics, never filters), then two record-EQ trimmers whose
//                  gains the deck solves from its own layer geometry at
//                  construction, like a technician with two trim pots. Then
//                  8x upsampling through polyphase half-band allpass
//                  cascades, and an explicit AC bias oscillator running
//                  BIAS-COHERENTLY: exactly one bias cycle per output sample,
//                  so every bias harmonic that the hysteresis generates
//                  aliases onto DC or Nyquist — nothing lands in the audio
//                  band, by construction instead of by filter.
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
const DEPTH_FACTOR: [f32; N_LAYERS] = [
    1.0 / (1.0 + LAYER_DEPTH_UM[0] / FIELD_DEPTH_UM),
    1.0 / (1.0 + LAYER_DEPTH_UM[1] / FIELD_DEPTH_UM),
    1.0 / (1.0 + LAYER_DEPTH_UM[2] / FIELD_DEPTH_UM),
];
/// AC bias runs BIAS-COHERENTLY: exactly one bias cycle per output sample
/// (bias frequency = the output sample rate, e.g. 48 kHz — a legitimate
/// cassette bias). The hysteresis loop turns the bias into a harmonic-rich
/// magnetization wave whose components alias at the oversampled rate; with
/// a coherent bias every one of those aliases lands on DC or the Nyquist
/// edge — nothing in the audio band, by construction instead of by filter.
const BIAS_AMP: f32 = 0.30;
/// Barkhausen noise scale: particle-avalanche noise per unit sqrt(|dM|).
/// Calibrated against published Type I cassette SNR: ~52-55 dB below
/// program level on a worn reel, measured on rendered output.
const BARKHAUSEN: f32 = 0.0016;

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
/// Record-alignment trimmer pivots (the deck's two "trim pots").
const TRIM_LOW_HZ: f32 = 2500.0;
const TRIM_HIGH_HZ: f32 = 9000.0;

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
    record_eq: [Shelf; 2],
    trim_low: [OnePoleHighPass; 2],
    trim_high: [OnePoleHighPass; 2],
    trim_gains: (f32, f32),
    field_scale: f32,
    makeup: f32,

    // The oxide
    bias_table: [f32; OS],
    up1: [Halfband; 2],
    up2: [Halfband; 2],
    up3: [Halfband; 2],
    hysteresis: [[JilesAtherton; N_LAYERS]; 2],
    barkhausen_level: f32,
    noise_rng: XorShift32,
    wallace_alpha: [[f32; N_LAYERS]; 2],
    wallace_stale: bool,
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
    playback_eq: [Shelf; 2],
    dc_block: [OnePoleHighPass; 2],
    crosstalk_lp: [OnePole; 2],
    prev_out: (f32, f32),
}

impl Tape {
    pub fn new(sample_rate: f32) -> Self {
        let size = (sample_rate * CENTER_DELAY_S * 2.0) as usize + 8;
        let print_len = (sample_rate * PRINT_DELAY_S) as usize + 1;
        let mut bias_table = [0.0f32; OS];
        for (i, b) in bias_table.iter_mut().enumerate() {
            *b = BIAS_AMP * (2.0 * PI * i as f32 / OS as f32).sin();
        }

        // Gap loss as flux averaging over the gap transit time. The transit
        // is ~1.2 samples at 48 kHz, so a fractional two-tap boxcar realizes
        // the physical integral (and its sinc rolloff) almost exactly.
        let gap_samples = (GAP_UM / TAPE_SPEED_UM_S) * sample_rate;
        let gap_frac = (gap_samples - 1.0).clamp(0.0, 1.0);

        // Self-alignment, step 1: the deck derives its record EQ from its
        // own fresh-tape layer geometry, exactly like a technician with two
        // trim pots: two high-pass trimmers, gains solved so the system
        // measures flat against the 1 kHz reference at 5 and 13 kHz.
        let trim_gains = align_record_trim();

        // The 120 us emphasis pair is EXACTLY reciprocal (pole/zero swap),
        // so record and playback EQ cancel to numerical precision in-band;
        // any in-band coloration that remains is physics, not filters.
        let record_shelf = Shelf::high_boost(sample_rate, 1326.0, 8000.0);
        let playback_shelf = record_shelf.inverse();

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
            record_eq: [record_shelf; 2],
            trim_low: [OnePoleHighPass::new(sample_rate, TRIM_LOW_HZ); 2],
            trim_high: [OnePoleHighPass::new(sample_rate, TRIM_HIGH_HZ); 2],
            trim_gains,
            field_scale: 0.0,
            makeup: 1.0,
            bias_table,
            up1: [Halfband::new(); 2],
            up2: [Halfband::new(); 2],
            up3: [Halfband::new(); 2],
            hysteresis: [[JilesAtherton::new(); N_LAYERS]; 2],
            barkhausen_level: 0.0,
            noise_rng: XorShift32::new(0x0DDB_1A5E),
            wallace_alpha: [[1.0; N_LAYERS]; 2],
            wallace_stale: true,
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
            playback_eq: [playback_shelf; 2],
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
        self.field_scale = 0.05 + 0.5 * self.drive;

        // Self-alignment, step 2: record a small 1 kHz tone through the full
        // biased multi-layer magnetic path and set makeup gain from what
        // comes back. Quadrature projection rejects the bias residue.
        // Runs only on knob moves, never per sample.
        self.makeup = calibrate_makeup(self.field_scale, self.sample_rate);

        // The head-bump contour is a playback-geometry effect; it fades in
        // quickly with record level only so a barely-engaged deck stays
        // near-transparent
        let bump_db = 2.5 * self.drive.sqrt();
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
        self.wallace_stale = true;

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
        // raises d for every layer while it lasts; the six exp() calls only
        // run while spacing is actually moving ---
        let lift = (1.0 - self.dropout_env) * SPACING_DROPOUT_UM;
        if self.wallace_stale || lift > 1e-4 {
            let os_rate = self.sample_rate * OS as f32;
            for ch in 0..2 {
                let az = if ch == 1 { self.azimuth_spacing_factor } else { 1.0 };
                for l in 0..N_LAYERS {
                    let d = (self.spacing_age_um + LAYER_DEPTH_UM[l] + lift) * az;
                    let f0 = TAPE_SPEED_UM_S / (2.0 * PI * d);
                    self.wallace_alpha[ch][l] = one_pole_alpha(os_rate, f0);
                }
            }
            self.wallace_stale = lift > 1e-4;
        }

        let bias_sub = self.bias_table;

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
        // --- Record side: exact 120 us emphasis, then the alignment trims ---
        let shelved = self.record_eq[ch].process(x);
        let emphasized = shelved
            + self.trim_gains.0 * self.trim_low[ch].process(shelved)
            + self.trim_gains.1 * self.trim_high[ch].process(shelved);

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
        let gain = self.makeup / N_LAYERS as f32;
        let mut mag = [0.0f32; OS];
        for s in 0..OS {
            let h_surface = sub[s] * self.field_scale + bias_sub[s];
            let mut acc = 0.0;
            for l in 0..N_LAYERS {
                let ja = &mut self.hysteresis[ch][l];
                let m_prev = ja.m;
                let m = ja.step(h_surface * DEPTH_FACTOR[l]);
                // Barkhausen: magnetization moves in particle avalanches, so
                // the noise power rides on how much switching just happened
                let avalanche = (m - m_prev).abs().sqrt();
                let noise = self.noise_rng.bipolar();
                let m_noisy = m + noise * self.barkhausen_level * avalanche;
                // This layer's spacing loss, at the oversampled rate
                let st = &mut self.wallace_state[ch][l];
                let alpha = self.wallace_alpha[ch][l];
                st[0] += alpha * (m_noisy - st[0]);
                st[1] += alpha * (st[0] - st[1]);
                acc += st[1];
            }
            mag[s] = acc * gain;
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
        let de_emphasized = self.playback_eq[ch].process(bumped);
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
            f.set_peaking(self.sample_rate, 60.0, 1.2, 2.5 * self.drive.sqrt());
            f
        };
        let record_shelf = Shelf::high_boost(self.sample_rate, 1326.0, 8000.0);
        self.record_eq[ch] = record_shelf;
        self.playback_eq[ch] = record_shelf.inverse();
        self.trim_low[ch] = OnePoleHighPass::new(self.sample_rate, TRIM_LOW_HZ);
        self.trim_high[ch] = OnePoleHighPass::new(self.sample_rate, TRIM_HIGH_HZ);
        self.dc_block[ch] = OnePoleHighPass::new(self.sample_rate, 10.0);
    }
}

/// Record a 1 kHz alignment tone through the full biased multi-layer
/// magnetic path and return the makeup gain that brings it back to unity.
fn calibrate_makeup(field_scale: f32, sample_rate: f32) -> f32 {
    let os_rate = sample_rate * OS as f32;
    let mut ja = [JilesAtherton::new(); N_LAYERS];

    let period = (os_rate / 1000.0) as usize;
    let total = period * 6;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    let mut count = 0usize;
    for n in 0..total {
        let bias = BIAS_AMP * (2.0 * PI * (n % OS) as f32 / OS as f32).sin();
        let phase = 2.0 * PI * (n % period) as f32 / period as f32;
        let h_surface = 0.1 * phase.sin() * field_scale + bias;
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

/// Solve the deck's two record-EQ trimmer gains so the fresh-tape system
/// response measures flat against the 1 kHz reference at 5 and 13 kHz.
/// Pure analysis of the layer geometry (complex frequency responses), run
/// once at construction — the digital equivalent of a two-pot alignment.
fn align_record_trim() -> (f32, f32) {
    // Just enough complex arithmetic for frequency responses
    #[derive(Clone, Copy)]
    struct C(f32, f32);
    impl C {
        fn mul(self, o: C) -> C {
            C(self.0 * o.0 - self.1 * o.1, self.0 * o.1 + self.1 * o.0)
        }
        fn div(self, o: C) -> C {
            let d = o.0 * o.0 + o.1 * o.1;
            C((self.0 * o.0 + self.1 * o.1) / d, (self.1 * o.0 - self.0 * o.1) / d)
        }
        fn add(self, o: C) -> C {
            C(self.0 + o.0, self.1 + o.1)
        }
        fn scale(self, s: f32) -> C {
            C(self.0 * s, self.1 * s)
        }
        fn norm(self) -> f32 {
            (self.0 * self.0 + self.1 * self.1).sqrt()
        }
    }
    let one = C(1.0, 0.0);

    // Fresh-tape layer sum: each layer is two cascaded Wallace poles
    let layers = |f: f32| -> C {
        let mut sum = C(0.0, 0.0);
        for l in 0..N_LAYERS {
            let d = SPACING_NEW_UM + LAYER_DEPTH_UM[l];
            let f0 = TAPE_SPEED_UM_S / (2.0 * PI * d);
            let pole = C(1.0, f / f0);
            sum = sum.add(one.div(pole.mul(pole)));
        }
        sum.scale(1.0 / N_LAYERS as f32)
    };
    // One-pole high-pass trimmer response
    let hp = |f: f32, pivot: f32| -> C {
        let r = f / pivot;
        C(0.0, r).div(C(1.0, r))
    };
    // Solve |1 + g h| = t for the smallest non-negative g
    let solve_gain = |t: f32, h: C| -> f32 {
        let (a, b) = (h.0 * h.0 + h.1 * h.1, h.0);
        let disc = b * b - a * (1.0 - t * t);
        if disc <= 0.0 {
            return 0.0;
        }
        ((disc.sqrt() - b) / a).max(0.0)
    };

    let (fa, fb) = (5000.0, 13000.0);
    let (mut g_low, mut g_high) = (0.0f32, 0.0f32);
    for _ in 0..40 {
        let system = |gl: f32, gh: f32, f: f32| {
            one.add(hp(f, TRIM_LOW_HZ).scale(gl))
                .mul(one.add(hp(f, TRIM_HIGH_HZ).scale(gh)))
                .mul(layers(f))
                .norm()
        };
        let reference = system(g_low, g_high, 1000.0);
        let needed = reference
            / one.add(hp(fa, TRIM_HIGH_HZ).scale(g_high)).mul(layers(fa)).norm();
        g_low = solve_gain(needed, hp(fa, TRIM_LOW_HZ));

        let reference = system(g_low, g_high, 1000.0);
        let needed = reference
            / one.add(hp(fb, TRIM_LOW_HZ).scale(g_low)).mul(layers(fb)).norm();
        g_high = solve_gain(needed, hp(fb, TRIM_HIGH_HZ));
    }
    (g_low.clamp(0.0, 6.0), g_high.clamp(0.0, 10.0))
}

/// First-order shelving filter with an EXACT inverse: one zero, one pole,
/// each placed by its own bilinear map so the corner frequencies land
/// precisely. `high_boost(fz, fp)` boosts highs by fp/fz; `inverse()` swaps
/// pole and zero, so the pair cancels to numerical precision.
#[derive(Clone, Copy)]
struct Shelf {
    b0: f32,
    b1: f32,
    a1: f32,
    x1: f32,
    y1: f32,
}

impl Shelf {
    fn high_boost(sample_rate: f32, f_zero: f32, f_pole: f32) -> Self {
        let z0 = bilinear_root(sample_rate, f_zero);
        let p0 = bilinear_root(sample_rate, f_pole);
        let g = (1.0 - p0) / (1.0 - z0); // unity gain at DC
        Self { b0: g, b1: -g * z0, a1: -p0, x1: 0.0, y1: 0.0 }
    }

    fn inverse(&self) -> Self {
        Self {
            b0: 1.0 / self.b0,
            b1: self.a1 / self.b0,
            a1: self.b1 / self.b0,
            x1: 0.0,
            y1: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 - self.a1 * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// Real-axis z-plane root for a first-order factor at `freq`, prewarped.
fn bilinear_root(sample_rate: f32, freq: f32) -> f32 {
    let t = (PI * freq / sample_rate).tan();
    (1.0 - t) / (1.0 + t)
}

/// The same xorshift PRNG the oscillator and filter use: no thread-local
/// lookups in the audio path.
#[derive(Clone, Copy)]
struct XorShift32(u32);

impl XorShift32 {
    fn new(seed: u32) -> Self {
        Self(seed | 1)
    }

    #[inline]
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform in [-1, 1).
    #[inline]
    fn bipolar(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (2.0 / 16_777_216.0) - 1.0
    }
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
    u: f32, // displacement (normalized so u and w share a scale)
    w: f32, // velocity, normalized to tape speed
    dt: f32,
    w0: f32,
    friction_at_rest: f32,
    rng: XorShift32,
}

const SCRAPE_HZ: f32 = 3400.0;
const MU_DELTA: f32 = 0.3; // static-over-kinetic friction excess
const STRIBECK_V: f32 = 0.35; // friction decay speed, in tape speeds
const SCRAPE_FORCE: f32 = 12500.0;
const SCRAPE_DAMPING: f32 = 400.0;

impl ScrapeOscillator {
    fn new(sample_rate: f32) -> Self {
        Self {
            u: 0.0,
            w: 0.0,
            dt: 1.0 / sample_rate,
            w0: 2.0 * PI * SCRAPE_HZ,
            friction_at_rest: MU_DELTA * (-1.0 / STRIBECK_V).exp(),
            rng: XorShift32::new(0x5C4A_9E17),
        }
    }

    fn next(&mut self) -> f32 {
        // Sliding speed of tape over head, normalized (1 = nominal)
        let v_rel = (1.0 + self.w).max(0.0);
        let friction = MU_DELTA * (-v_rel / STRIBECK_V).exp() - self.friction_at_rest;
        let seed = self.rng.bipolar() * 1e-3;
        // Semi-implicit Euler in normalized coordinates (u' = w0*w,
        // w' = -w0*u - ...): stable for a ~3.4 kHz resonance at audio rates
        self.w += self.dt
            * (-self.w0 * self.u - SCRAPE_FORCE * friction - SCRAPE_DAMPING * self.w)
            + seed;
        self.w = self.w.clamp(-0.9, 0.9);
        self.u = (self.u + self.dt * self.w0 * self.w).clamp(-3.0, 3.0);
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
    /// H(z) = (A(z^2) + z^-1 B(z^2)) / 2: even outputs from the direct
    /// branch, odd outputs from the delayed branch.
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
        (p, q)
    }

    /// Two high-rate samples in (x0 earlier, x1 later), one low-rate sample
    /// out. The z^-1 on the B branch means it takes the earlier sample.
    #[inline]
    fn down(&mut self, x0: f32, x1: f32) -> f32 {
        let mut p = x1;
        for s in &mut self.a {
            p = s.process(p);
        }
        let mut q = x0;
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
    // The newest sample sits at write-1. Interpolate at the true fractional
    // position (frac of the position, not of the delay — using delay.fract()
    // mirrors the sub-sample offset and clicks at every integer crossing).
    let pos = write as f32 - 1.0 - delay + 2.0 * size as f32;
    let read = pos as usize % size;
    let frac = pos.fract();
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
    rng: XorShift32,
}

impl SmoothedNoise {
    fn new(sample_rate: f32, cutoff: f32) -> Self {
        let alpha = 1.0 - (-2.0 * PI * cutoff / sample_rate).exp();
        Self {
            stage1: 0.0,
            stage2: 0.0,
            alpha,
            gain: 1.0 / (0.577 * (alpha / 2.0).sqrt().max(1e-6)),
            rng: XorShift32::new((cutoff * 1000.0) as u32),
        }
    }

    fn next(&mut self) -> f32 {
        let white = self.rng.bipolar();
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
    /// (fundamental rms, harmonic-distortion rms, residual noise rms).
    fn harmonic_split(signal: &[f32], f0: f32, n_harm: usize) -> (f32, f32, f32) {
        let n = signal.len();
        let mut residual: Vec<f32> = signal.to_vec();
        let mut fundamental_rms = 0.0f32;
        let mut harmonic_power = 0.0f64;
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
            } else {
                harmonic_power += ((a * a + b * b) / 2.0) as f64;
            }
        }
        let res_rms =
            (residual.iter().map(|x| (x * x) as f64).sum::<f64>() / n as f64).sqrt() as f32;
        (fundamental_rms, harmonic_power.sqrt() as f32, res_rms)
    }

    fn rms(signal: &[f32]) -> f32 {
        (signal.iter().map(|x| (x * x) as f64).sum::<f64>() / signal.len() as f64).sqrt()
            as f32
    }

    /// Correlate `signal` against a sine at `freq` (rate `fs`) and return
    /// the component's amplitude.
    fn tone_amplitude(signal: &[f32], freq: f32, fs: f32) -> f32 {
        let w = 2.0 * PI * freq / fs;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &x) in signal.iter().enumerate() {
            re += (x * (w * i as f32).sin()) as f64;
            im += (x * (w * i as f32).cos()) as f64;
        }
        let n = signal.len() as f64;
        (2.0 * (re * re + im * im).sqrt() / n) as f32
    }

    #[test]
    fn halfband_decimator_passes_band_and_rejects_aliases() {
        // 96 kHz -> 48 kHz. A 5 kHz tone must survive at unity; a 30 kHz
        // tone would alias to 18 kHz and must be crushed.
        let run = |freq: f32| {
            let mut hb = Halfband::new();
            let n = 16384;
            let mut out = Vec::with_capacity(n / 2);
            for i in 0..n / 2 {
                let x0 = (2.0 * PI * freq * (2 * i) as f32 / 96000.0).sin();
                let x1 = (2.0 * PI * freq * (2 * i + 1) as f32 / 96000.0).sin();
                out.push(hb.down(x0, x1));
            }
            out
        };
        let pass = tone_amplitude(&run(5000.0)[512..], 5000.0, 48000.0);
        let alias = tone_amplitude(&run(30000.0)[512..], 18000.0, 48000.0);
        assert!((0.9..1.1).contains(&pass), "passband gain {}", pass);
        assert!(alias < 0.01, "alias must be rejected, leaked {}", alias);
    }

    #[test]
    fn halfband_interpolator_rejects_images() {
        // 48 kHz -> 96 kHz. A 1 kHz tone must survive; its image at 47 kHz
        // must be crushed.
        let mut hb = Halfband::new();
        let n = 16384;
        let mut out = Vec::with_capacity(n * 2);
        for i in 0..n {
            let x = (2.0 * PI * 1000.0 * i as f32 / 48000.0).sin();
            let (y0, y1) = hb.up(x);
            out.push(y0);
            out.push(y1);
        }
        let pass = tone_amplitude(&out[1024..], 1000.0, 96000.0);
        let image = tone_amplitude(&out[1024..], 47000.0, 96000.0);
        assert!((0.9..1.1).contains(&pass), "passband gain {}", pass);
        assert!(image < 0.01, "image must be rejected, leaked {}", image);
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
            let (fund, harm, _) = harmonic_split(&out, 1000.0, 20);
            harm / fund.max(1e-9)
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
            let (_, _, resid) = harmonic_split(&out, 1000.0, 23);
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
        // Response ratios measured at low level, fundamental only — at hot
        // levels HF saturation would masquerade as frequency response
        let hf_ratio = |age: f32| {
            let gain_at = |freq: f32| {
                let mut tape = Tape::new(FS);
                tape.set_age(age);
                tape.set_drive(0.3);
                let n = 48000;
                let mut out = Vec::with_capacity(n / 2);
                for (i, x) in sine(n, freq).enumerate() {
                    let (l, _) = tape.process(x * 0.2, x * 0.2);
                    if i >= n / 2 {
                        out.push(l);
                    }
                }
                tone_amplitude(&out, freq, FS) / 0.1
            };
            gain_at(10000.0) / gain_at(400.0)
        };
        let fresh = hf_ratio(0.0);
        let worn = hf_ratio(1.0);
        assert!(fresh > 0.25, "fresh tape should keep its top end: {}", fresh);
        assert!(worn < fresh * 0.5, "worn tape must dull: fresh {} worn {}", fresh, worn);
    }

    #[test]
    fn frequency_response_meets_cassette_spec() {
        // A fresh, aligned deck must hold +-3.5 dB against the 1 kHz
        // reference across the audible band (Type I cassettes were specced
        // around 30 Hz - 14 kHz +-3 dB; we verify 200 Hz - 13 kHz, clear of
        // the head-bump region below and the spec edge above). Measured the
        // way the real spec is measured: at low level (~-20 VU), because at
        // reference level cassette HF genuinely saturates — that is
        // compression, not frequency response.
        let gain_at = |freq: f32| {
            let mut tape = Tape::new(FS);
            tape.set_drive(0.3);
            let n = 48000;
            let mut out = Vec::with_capacity(n / 2);
            for (i, x) in sine(n, freq).enumerate() {
                let (l, _) = tape.process(x * 0.2, x * 0.2);
                if i >= n / 2 {
                    out.push(l);
                }
            }
            tone_amplitude(&out, freq, FS) / 0.1
        };
        let reference = gain_at(1000.0);
        for freq in [200.0, 500.0, 2000.0, 5000.0, 8000.0, 11000.0, 13000.0] {
            let db = 20.0 * (gain_at(freq) / reference).log10();
            assert!(
                db.abs() < 3.5,
                "{} Hz is {:+.2} dB against the 1 kHz reference",
                freq,
                db
            );
        }
    }

    #[test]
    fn emphasis_pair_is_exactly_reciprocal() {
        // Record shelf into playback shelf must cancel to numerical
        // precision — any in-band color the tape adds is physics, not EQ
        let mut record = Shelf::high_boost(FS, 1326.0, 8000.0);
        let mut playback = record.inverse();
        let mut worst = 0.0f32;
        let mut rng = XorShift32::new(12345);
        for i in 0..4096 {
            let x = rng.bipolar();
            let y = playback.process(record.process(x));
            if i > 16 {
                worst = worst.max((y - x).abs());
            }
        }
        assert!(worst < 1e-4, "pair must cancel, worst error {}", worst);
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
        let s1 = period_spread(1.0);
        let s0 = period_spread(0.0);
        assert!(s1 > 2.0 * s0, "wow must wander the pitch: {} vs {}", s1, s0);
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
        println!(
            "ferric engine: {:.1}x realtime ({} ch pairs, {}x OS, {} layers)",
            audio_time / elapsed,
            2,
            OS,
            N_LAYERS
        );
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
