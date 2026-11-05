// src/inserts.rs
//
// A track strip's inserts: the things a console channel puts between the
// tape return and the fader. Every one is OUT by default, and out means
// bit-identical: a strip that never had an insert set never runs one.
//
//   Eq3      low shelf, peaking mid, high shelf (RBJ cookbook biquads)
//   Dist     a drive stage with five circuits on a switch: tanh, hard
//            clip, foldback (the three waveshapers antialiased by
//            first-order ADAA), bitcrush and sample-rate reduction (the
//            two lo-fi converters, deliberately un-antialiased: their
//            aliasing is the sound)
//   Shaper   a tempo-synced volume shaper: a named curve (`pump`, `saw`,
//            `ramp`, `sine`, `square`, `tri`) or a trance-gate step
//            pattern (`x.x.xx.x`), riding the song's beat clock
//
// Levels here are BUS units: the strip runs in volts, and the caller
// converts with the same factor the strip compressor uses, so "1.0" is
// 0 dBFS at unity master volume and a drive threshold means the same
// thing on every track.

use std::f64::consts::{LN_10, LN_2, PI};

/// First-order antiderivative antialiasing for a memoryless shaper `f`
/// with antiderivative `big_f` (Parker et al., DAFx-16):
/// y[n] = (F(x[n]) - F(x[n-1])) / (x[n] - x[n-1]), falling back to
/// f(midpoint) when the step is too small to divide by. In f64: the
/// antiderivatives grow with the drive, and f32 differences of them are
/// audibly noisy at the ill-conditioned edge.
#[derive(Clone, Copy, Default)]
struct Adaa1 {
    x1: f64,
    f1: f64,
}

impl Adaa1 {
    fn seed(&mut self, x: f64, big_f: fn(f64) -> f64) {
        self.x1 = x;
        self.f1 = big_f(x);
    }

    #[inline]
    fn process(&mut self, x: f64, f: fn(f64) -> f64, big_f: fn(f64) -> f64) -> f64 {
        let f0 = big_f(x);
        let dx = x - self.x1;
        let y = if dx.abs() < 1e-6 {
            f(0.5 * (x + self.x1))
        } else {
            (f0 - self.f1) / dx
        };
        self.x1 = x;
        self.f1 = f0;
        y
    }
}

fn tanh_f(x: f64) -> f64 {
    x.tanh()
}

/// ln(cosh(x)), stable for large |x|.
fn ln_cosh(x: f64) -> f64 {
    let a = x.abs();
    a + (-2.0 * a).exp().ln_1p() - LN_2
}

fn clip_f(x: f64) -> f64 {
    x.clamp(-1.0, 1.0)
}

fn clip_big_f(x: f64) -> f64 {
    let a = x.abs();
    if a <= 1.0 {
        0.5 * x * x
    } else {
        a - 0.5
    }
}

/// Triangle foldback: unity inside +/-1, and every excursion past a rail
/// reflects back off it (period 4 in the input).
fn fold_f(x: f64) -> f64 {
    let u = (x + 1.0).rem_euclid(4.0);
    1.0 - (u - 2.0).abs()
}

/// Antiderivative of `fold_f`, piecewise quadratic and periodic (each
/// period integrates to zero), so it stays bounded at any drive.
fn fold_big_f(x: f64) -> f64 {
    let u = (x + 1.0).rem_euclid(4.0);
    if u <= 2.0 {
        0.5 * u * u - u
    } else {
        3.0 * (u - 2.0) - 0.5 * (u * u - 4.0)
    }
}

/// The distortion's circuit switch (`dist_mode`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DistMode {
    Tanh,
    Clip,
    Fold,
    Crush,
    Rate,
}

impl DistMode {
    pub fn from_value(v: f32) -> Self {
        match v.round() as i32 {
            i32::MIN..=0 => DistMode::Tanh,
            1 => DistMode::Clip,
            2 => DistMode::Fold,
            3 => DistMode::Crush,
            _ => DistMode::Rate,
        }
    }

    fn shaper(self) -> Option<(fn(f64) -> f64, fn(f64) -> f64)> {
        match self {
            DistMode::Tanh => Some((tanh_f, ln_cosh)),
            DistMode::Clip => Some((clip_f, clip_big_f)),
            DistMode::Fold => Some((fold_f, fold_big_f)),
            DistMode::Crush | DistMode::Rate => None,
        }
    }
}

/// The waveshapers' drive at full `dist`, dB.
pub const DIST_MAX_DRIVE_DB: f64 = 36.0;

/// A strip's drive stage. `dist` (0..1) is the amount: for the three
/// waveshapers it is input drive (0..+36 dB, and the output comes back
/// down by half the drive in dB, so a harder setting is denser and a bit
/// louder rather than 36 dB louder); for bitcrush it is the word length
/// (16 bits down to 2); for rate reduction the hold rate (the full rate
/// down 7 octaves, 48 kHz -> 375 Hz). `dist_mix` blends it with the dry
/// signal.
#[derive(Clone)]
pub struct Dist {
    amount: f32,
    amount_cur: f32,
    mix: f32,
    mix_cur: f32,
    mode: DistMode,
    smooth_k: f32,
    adaa: [Adaa1; 2],
    held: [f32; 2],
    phase: f32,
    /// Running since the last bypass; a shaper coming back in seeds its
    /// ADAA memory from the signal so its first quotient spans one step.
    running: bool,
}

impl Dist {
    pub fn new(sample_rate: f32) -> Self {
        Dist {
            amount: 0.0,
            amount_cur: 0.0,
            mix: 1.0,
            mix_cur: 1.0,
            mode: DistMode::Tanh,
            smooth_k: crate::smoothing::approach(crate::smoothing::KNOB_SMOOTH_S, sample_rate),
            adaa: [Adaa1::default(); 2],
            held: [0.0; 2],
            phase: 0.0,
            running: false,
        }
    }

    pub fn set_amount(&mut self, v: f32) {
        if v.is_finite() {
            self.amount = v.clamp(0.0, 1.0);
        }
    }

    pub fn set_mix(&mut self, v: f32) {
        if v.is_finite() {
            self.mix = v.clamp(0.0, 1.0);
        }
    }

    /// Guarded: re-asserting the switch position must not reseed the
    /// shaper memory (design.md, setter idempotence).
    pub fn set_mode(&mut self, v: f32) {
        let mode = DistMode::from_value(v);
        if mode != self.mode {
            self.mode = mode;
            self.running = false;
        }
    }

    pub fn mode(&self) -> DistMode {
        self.mode
    }

    /// Whether the stage is in the signal path at all.
    pub fn is_active(&self) -> bool {
        self.amount > 0.0 || self.amount_cur > 1e-5
    }

    /// One stereo sample in bus units.
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.amount_cur += (self.amount - self.amount_cur) * self.smooth_k;
        self.mix_cur += (self.mix - self.mix_cur) * self.smooth_k;
        if self.amount == 0.0 && self.amount_cur <= 1e-5 {
            self.amount_cur = 0.0;
            self.running = false;
            return (l, r);
        }
        let a = self.amount_cur as f64;
        let input = [l, r];
        let mut wet = [0.0f32; 2];
        match self.mode.shaper() {
            Some((f, big_f)) => {
                let g = (a * DIST_MAX_DRIVE_DB / 20.0 * LN_10).exp();
                let out_g = 1.0 / g.sqrt();
                for c in 0..2 {
                    let u = input[c] as f64 * g;
                    if !self.running {
                        self.adaa[c].seed(u, big_f);
                    }
                    wet[c] = (self.adaa[c].process(u, f, big_f) * out_g) as f32;
                }
            }
            None if self.mode == DistMode::Crush => {
                let bits = 16.0 - 14.0 * a;
                let q = (bits - 1.0).exp2();
                for c in 0..2 {
                    wet[c] = ((input[c] as f64 * q).round() / q) as f32;
                }
            }
            None => {
                // Sample-and-hold at the reduced rate: a fractional
                // clock, so the rate sweeps smoothly under automation
                let step = (-7.0 * a).exp2() as f32;
                self.phase += step;
                if self.phase >= 1.0 || !self.running {
                    self.phase = self.phase.fract();
                    self.held = input;
                }
                wet = self.held;
            }
        }
        self.running = true;
        let m = self.mix_cur;
        if m >= 1.0 {
            // fully wet is exactly the wet signal (the blend's arithmetic
            // would leave the dry signal's rounding in it)
            return (wet[0], wet[1]);
        }
        (l + m * (wet[0] - l), r + m * (wet[1] - r))
    }
}

/// One RBJ biquad, transposed direct form II, stereo, in f64 (a 20 Hz
/// shelf at 192 kHz puts the poles a hair from the unit circle).
#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [[f64; 2]; 2],
}

impl Biquad {
    fn identity() -> Self {
        Biquad {
            b: [1.0, 0.0, 0.0],
            a: [0.0, 0.0],
            z: [[0.0; 2]; 2],
        }
    }

    fn set(&mut self, b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) {
        self.b = [b0 / a0, b1 / a0, b2 / a0];
        self.a = [a1 / a0, a2 / a0];
    }

    #[inline]
    fn process(&mut self, c: usize, x: f64) -> f64 {
        let z = &mut self.z[c];
        let y = self.b[0] * x + z[0];
        z[0] = self.b[1] * x - self.a[0] * y + z[1];
        z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

#[derive(Clone, Copy, PartialEq)]
enum BandKind {
    LowShelf,
    Peak,
    HighShelf,
}

#[derive(Clone)]
struct Band {
    kind: BandKind,
    gain_db: f32,
    freq: f32,
    q: f32,
    /// In the path once its gain has ever left 0 dB; it then stays in
    /// (a band at 0 dB is the identity), so riding a gain back through
    /// zero never drops the filter's memory mid-note.
    on: bool,
    filt: Biquad,
}

impl Band {
    fn new(kind: BandKind, freq: f32) -> Self {
        Band {
            kind,
            gain_db: 0.0,
            freq,
            q: 0.7,
            on: false,
            filt: Biquad::identity(),
        }
    }

    fn design(&mut self, sample_rate: f32) {
        let fs = sample_rate as f64;
        let f0 = (self.freq as f64).clamp(10.0, 0.45 * fs);
        let w0 = 2.0 * PI * f0 / fs;
        let (sn, cs) = w0.sin_cos();
        let big_a = 10f64.powf(self.gain_db as f64 / 40.0);
        match self.kind {
            BandKind::Peak => {
                let alpha = sn / (2.0 * (self.q as f64).max(0.05));
                self.filt.set(
                    1.0 + alpha * big_a,
                    -2.0 * cs,
                    1.0 - alpha * big_a,
                    1.0 + alpha / big_a,
                    -2.0 * cs,
                    1.0 - alpha / big_a,
                );
            }
            BandKind::LowShelf | BandKind::HighShelf => {
                // Shelf slope S = 1: the steepest shelf without a bump
                let alpha = sn / 2.0 * std::f64::consts::SQRT_2;
                let k = 2.0 * big_a.sqrt() * alpha;
                let (ap, am) = (big_a + 1.0, big_a - 1.0);
                if self.kind == BandKind::LowShelf {
                    self.filt.set(
                        big_a * (ap - am * cs + k),
                        2.0 * big_a * (am - ap * cs),
                        big_a * (ap - am * cs - k),
                        ap + am * cs + k,
                        -2.0 * (am + ap * cs),
                        ap + am * cs - k,
                    );
                } else {
                    self.filt.set(
                        big_a * (ap + am * cs + k),
                        -2.0 * big_a * (am + ap * cs),
                        big_a * (ap + am * cs - k),
                        ap - am * cs + k,
                        2.0 * (am - ap * cs),
                        ap - am * cs - k,
                    );
                }
            }
        }
    }
}

/// Which knob of the three-band EQ a strip setting turns.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum EqKnob {
    LowGain,
    LowFreq,
    MidGain,
    MidFreq,
    MidQ,
    HighGain,
    HighFreq,
}

/// A channel EQ: low shelf (default corner 100 Hz), peaking mid (1 kHz,
/// Q 0.7), high shelf (8 kHz); every gain in dB, every band out at 0 dB.
#[derive(Clone)]
pub struct Eq3 {
    bands: [Band; 3],
    sample_rate: f32,
}

impl Eq3 {
    pub fn new(sample_rate: f32) -> Self {
        Eq3 {
            bands: [
                Band::new(BandKind::LowShelf, 100.0),
                Band::new(BandKind::Peak, 1000.0),
                Band::new(BandKind::HighShelf, 8000.0),
            ],
            sample_rate,
        }
    }

    /// Turn one knob. Guarded: re-asserting a setting is free and leaves
    /// the filters' memory alone.
    pub fn set(&mut self, knob: EqKnob, v: f32) {
        if !v.is_finite() {
            return;
        }
        let (band, field) = match knob {
            EqKnob::LowGain => (0, 0),
            EqKnob::LowFreq => (0, 1),
            EqKnob::MidGain => (1, 0),
            EqKnob::MidFreq => (1, 1),
            EqKnob::MidQ => (1, 2),
            EqKnob::HighGain => (2, 0),
            EqKnob::HighFreq => (2, 1),
        };
        let b = &mut self.bands[band];
        let slot = match field {
            0 => &mut b.gain_db,
            1 => &mut b.freq,
            _ => &mut b.q,
        };
        if *slot == v {
            return;
        }
        *slot = v;
        if b.gain_db != 0.0 {
            b.on = true;
        }
        b.design(self.sample_rate);
    }

    pub fn is_active(&self) -> bool {
        self.bands.iter().any(|b| b.on)
    }

    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mut x = [l as f64, r as f64];
        for b in self.bands.iter_mut().filter(|b| b.on) {
            for (c, v) in x.iter_mut().enumerate() {
                *v = b.filt.process(c, *v);
            }
        }
        (x[0] as f32, x[1] as f32)
    }
}

/// A volume shaper's curve over one cycle: 1 = open, 0 = fully shaped.
#[derive(Clone, Debug, PartialEq)]
pub enum ShapeCurve {
    /// The sidechain pump: shut at the downbeat, an ease-out back to
    /// open by three quarters of the cycle.
    Pump,
    /// Rising ramp, shut to open.
    Saw,
    /// Falling ramp, open to shut.
    Ramp,
    /// Shut at the downbeat, open at half-cycle.
    Sine,
    /// Open the first half, shut the second.
    Square,
    /// Shut, open at half-cycle, shut.
    Tri,
    /// A trance gate: one level per step.
    Steps(Vec<f32>),
}

/// A tempo-synced volume shaper (`shape=` on a track line).
#[derive(Clone, Debug, PartialEq)]
pub struct Shaper {
    pub curve: ShapeCurve,
    /// Beats per cycle.
    pub beats: f64,
}

/// A trance-gate step's default length: a sixteenth note.
pub const GATE_STEP_BEATS: f64 = 0.25;

impl Shaper {
    /// `pump`, `saw`, `ramp`, `sine`, `square`, `tri` (one beat per cycle
    /// unless `beats` says otherwise), or a step pattern: `x` open, `.`
    /// `-` or `_` shut, `0`..`9` a level in ninths (`9` = open), one
    /// sixteenth per step unless `beats` gives the whole pattern's length.
    pub fn parse(spec: &str, beats: Option<f64>) -> Result<Self, String> {
        let curve = match spec {
            "pump" => ShapeCurve::Pump,
            "saw" => ShapeCurve::Saw,
            "ramp" => ShapeCurve::Ramp,
            "sine" => ShapeCurve::Sine,
            "square" => ShapeCurve::Square,
            "tri" => ShapeCurve::Tri,
            _ => {
                let steps = spec
                    .chars()
                    .map(|c| match c {
                        'x' | 'X' => Ok(1.0),
                        '.' | '-' | '_' => Ok(0.0),
                        '0'..='9' => Ok(c.to_digit(10).unwrap() as f32 / 9.0),
                        _ => Err(format!(
                            "shape '{}': a curve (pump saw ramp sine square tri) or a step \
                             pattern of x . and 0-9, got '{}'",
                            spec, c
                        )),
                    })
                    .collect::<Result<Vec<f32>, String>>()?;
                if steps.is_empty() {
                    return Err("shape= needs a curve or a step pattern".into());
                }
                ShapeCurve::Steps(steps)
            }
        };
        let beats = match beats {
            Some(b) => b,
            None => match &curve {
                ShapeCurve::Steps(s) => s.len() as f64 * GATE_STEP_BEATS,
                _ => 1.0,
            },
        };
        if !(beats.is_finite() && beats > 0.0) {
            return Err("shape_beats must be positive".into());
        }
        Ok(Shaper { curve, beats })
    }

    /// The curve's level at a song beat, 0..1.
    pub fn level(&self, beat: f64) -> f32 {
        let phase = (beat / self.beats).rem_euclid(1.0);
        let p = phase as f32;
        match &self.curve {
            ShapeCurve::Pump => {
                let t = (p / 0.75).min(1.0);
                (t * std::f32::consts::FRAC_PI_2).sin()
            }
            ShapeCurve::Saw => p,
            ShapeCurve::Ramp => 1.0 - p,
            ShapeCurve::Sine => 0.5 - 0.5 * (std::f32::consts::TAU * p).cos(),
            ShapeCurve::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            ShapeCurve::Tri => 1.0 - (2.0 * p - 1.0).abs(),
            ShapeCurve::Steps(s) => {
                let i = ((phase * s.len() as f64) as usize).min(s.len() - 1);
                s[i]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    fn sine(freq: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / SR).sin())
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn antiderivatives_match_their_shapers() {
        for (f, big_f) in [
            (tanh_f as fn(f64) -> f64, ln_cosh as fn(f64) -> f64),
            (clip_f, clip_big_f),
            (fold_f, fold_big_f),
        ] {
            for i in -800..800 {
                let x = i as f64 * 0.01 + 0.003;
                let h = 1e-5;
                let d = (big_f(x + h) - big_f(x - h)) / (2.0 * h);
                assert!((d - f(x)).abs() < 1e-4, "F' != f at {x}: {d} vs {}", f(x));
            }
        }
        // foldback reflects off the rails
        assert!((fold_f(0.5) - 0.5).abs() < 1e-12);
        assert!((fold_f(1.5) - 0.5).abs() < 1e-12);
        assert!((fold_f(-1.5) + 0.5).abs() < 1e-12);
    }

    #[test]
    fn out_is_untouched_and_each_mode_does_its_job() {
        let x = sine(220.0, 0.5, 9600);
        // out: exactly the input
        let mut d = Dist::new(SR);
        for &v in &x {
            assert_eq!(d.process(v, -v), (v, -v));
        }
        let run = |mode: f32, amount: f32| -> Vec<f32> {
            let mut d = Dist::new(SR);
            d.set_mode(mode);
            d.set_amount(amount);
            // let the drive knob settle
            for _ in 0..4800 {
                d.process(0.0, 0.0);
            }
            x.iter().map(|&v| d.process(v, v).0).collect()
        };
        let peak = |y: &[f32]| y.iter().fold(0f32, |m, v| m.max(v.abs()));
        // tanh and clip squash the crest of a sine toward a square's
        for mode in [0.0, 1.0] {
            let y = run(mode, 1.0);
            let crest = 20.0 * (peak(&y[4800..]) / rms(&y[4800..])).log10();
            assert!(crest < 1.5, "mode {mode}: crest {crest:.2} dB");
        }
        // foldback never leaves the rails, at any drive
        let y = run(2.0, 1.0);
        assert!(peak(&y) <= 1.0 / (DIST_MAX_DRIVE_DB as f32 / 40.0 * 10f32.ln()).exp() + 1e-3);
        // bitcrush at full is two bits: a handful of levels
        let y = run(3.0, 1.0);
        let mut levels: Vec<i32> = y[4800..].iter().map(|v| (v * 1000.0).round() as i32).collect();
        levels.sort();
        levels.dedup();
        assert!(levels.len() <= 5, "bitcrush levels {levels:?}");
        // rate reduction holds each value for many samples
        let y = run(4.0, 1.0);
        let changes = y[4800..].windows(2).filter(|w| w[0] != w[1]).count();
        assert!(changes < 4800 / 100, "held {changes} changes");
    }

    /// The lo-fi converters change the grain, not the level: rate
    /// reduction holds samples of the signal and bitcrush rounds them, so
    /// a pad-like signal (detuned saws) keeps its RMS.
    #[test]
    fn the_lofi_modes_keep_the_level() {
        let n = 48000;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / SR;
                [174.6f32, 176.1, 207.7, 261.6, 349.2]
                    .iter()
                    .map(|f| 0.03 * (2.0 * (t * f).fract() - 1.0))
                    .sum::<f32>()
            })
            .collect();
        for (mode, amount) in [(4.0, 0.1), (4.0, 0.5), (4.0, 0.8), (3.0, 0.5)] {
            let mut d = Dist::new(SR);
            d.set_mode(mode);
            d.set_amount(amount);
            for _ in 0..4800 {
                d.process(0.0, 0.0);
            }
            let y: Vec<f32> = x.iter().map(|&v| d.process(v, v).0).collect();
            let db = 20.0 * (rms(&y[4800..]) / rms(&x[4800..])).log10();
            assert!(db.abs() < 0.3, "mode {mode} at {amount}: {db:+.2} dB");
        }
    }

    #[test]
    fn eq_bands_cut_and_boost_where_they_sit() {
        let gain_at = |knob: EqKnob, db: f32, freq: f32| {
            let mut eq = Eq3::new(SR);
            eq.set(knob, db);
            let x = sine(freq, 0.1, 48000);
            let y: Vec<f32> = x.iter().map(|&v| eq.process(v, v).0).collect();
            20.0 * (rms(&y[24000..]) / rms(&x[24000..])).log10()
        };
        assert!((gain_at(EqKnob::LowGain, 6.0, 30.0) - 6.0).abs() < 0.5);
        assert!(gain_at(EqKnob::LowGain, 6.0, 5000.0).abs() < 0.2);
        assert!((gain_at(EqKnob::MidGain, -9.0, 1000.0) + 9.0).abs() < 0.3);
        assert!(gain_at(EqKnob::MidGain, -9.0, 60.0).abs() < 0.5);
        assert!((gain_at(EqKnob::HighGain, 6.0, 16000.0) - 6.0).abs() < 0.5);
        assert!(gain_at(EqKnob::HighGain, 6.0, 200.0).abs() < 0.2);
        // never touched: out, bit for bit
        let mut eq = Eq3::new(SR);
        eq.set(EqKnob::MidFreq, 400.0);
        assert!(!eq.is_active());
    }

    /// Automation re-asserts every value every block (design.md): the
    /// EQ must not redesign, and the drive switch must not reseed, when
    /// the setting has not changed.
    #[test]
    fn reasserting_a_setting_is_a_no_op() {
        let mut eq = Eq3::new(SR);
        eq.set(EqKnob::LowGain, 6.0);
        for v in sine(50.0, 0.3, 2000) {
            eq.process(v, v);
        }
        let z = eq.bands[0].filt.z;
        eq.set(EqKnob::LowGain, 6.0);
        eq.set(EqKnob::LowFreq, 100.0);
        assert_eq!(eq.bands[0].filt.z, z);
        let mut d = Dist::new(SR);
        d.set_mode(2.0);
        d.set_amount(0.5);
        for v in sine(50.0, 0.3, 2000) {
            d.process(v, v);
        }
        d.set_mode(2.0);
        assert!(d.running, "re-asserting the mode reseeded the shaper");
        d.set_mode(1.0);
        assert!(!d.running);
    }

    #[test]
    fn shapes_and_patterns() {
        let s = Shaper::parse("x..x", None).unwrap();
        assert_eq!(s.beats, 1.0);
        assert_eq!(s.level(0.0), 1.0);
        assert_eq!(s.level(0.3), 0.0);
        assert_eq!(s.level(0.8), 1.0);
        assert_eq!(s.level(1.3), 0.0, "cycles repeat");
        let p = Shaper::parse("pump", Some(2.0)).unwrap();
        assert!(p.level(0.0) < 1e-6 && (p.level(1.6) - 1.0).abs() < 1e-6);
        assert!(Shaper::parse("x?x", None).is_err());
        assert!(Shaper::parse("saw", Some(0.0)).is_err());
        let d = Shaper::parse("9.4", None).unwrap();
        assert_eq!(d.beats, 0.75);
        assert_eq!(d.level(0.1), 1.0);
        assert!((d.level(0.6) - 4.0 / 9.0).abs() < 1e-6);
    }
}
