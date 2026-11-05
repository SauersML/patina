// src/physical.rs
//
// Physical models as song instruments: a digital-waveguide string and a
// modal resonator bank. Each `track NAME string` / `track NAME modal=bell`
// line owns one polyphonic instrument on a reserved channel block, with
// its own desk strip and stem like a tape-deck slot. Notes strike, pluck
// or bow it; `excite=TRACK` routes another track's audio into it, so a
// drum loop can ring a plate or a voice can bow a wire.
//
// The string (Karplus-Strong grown into a two-segment waveguide, after
// Smith's bowed-string model in the STK):
//
//   junction (pluck, hammer or bow point, `str_pos` of the way from the bridge)
//     -> bridge segment (integer delay) -> loss filter -> dispersion allpasses -> x(-1)
//     -> nut segment (Lagrange-interpolated delay) -> x(-1) -> junction
//
// Every number that sets the pitch and the decay is solved, not tuned by
// ear: the nut segment's fractional length is chosen so the whole loop's
// phase delay at the fundamental is exactly one period (the loss filter's,
// the allpasses' and the interpolator's own phase delays included), and the
// loss filter's gain is chosen so the fundamental falls 60 dB in exactly
// `str_decay` seconds. `str_stiff` is the inharmonicity coefficient B of a
// stiff string (partial k at k f0 sqrt(1 + B k^2)); a cascade of identical
// first-order allpasses is fitted to it at note-on.
//
// The modal bank is Hypha's reference (compute/specs/modal.py), ported
// line for line: a sum of two-pole resonators, one per mode, every mode's
// frequency, T60 and gain from the preset (or a `modes` line), the shapes
// (decay, damp, stretch, bright, spread) applied on top. The tests compare
// it against the reference's own renders.

use crate::oscillator::PROGRAM_V;
use crate::rng::Rng;
use crate::song::Param;
use std::f64::consts::PI;

/// Instruments live on a reserved channel block just below the extra kit
/// boards: instrument i is channel INSTRUMENT_CHANNEL_BASE + i.
pub const MAX_INSTRUMENTS: usize = 32;
pub const INSTRUMENT_CHANNEL_BASE: u16 =
    crate::drums::KIT_CHANNEL_BASE - MAX_INSTRUMENTS as u16;

/// Map a song channel to an instrument slot, if it is one.
pub fn instrument_for_channel(channel: u16) -> Option<usize> {
    let base = INSTRUMENT_CHANNEL_BASE;
    if (base..base + MAX_INSTRUMENTS as u16).contains(&channel) {
        Some((channel - base) as usize)
    } else {
        None
    }
}

/// The waveguide string's own knobs.
pub fn is_string_param(p: Param) -> bool {
    matches!(
        p,
        Param::StrExcite
            | Param::StrDecay
            | Param::StrDamp
            | Param::StrStiff
            | Param::StrPos
            | Param::StrHard
            | Param::StrPressure
            | Param::StrMute
    )
}

/// The modal bank's own knobs.
pub fn is_modal_param(p: Param) -> bool {
    matches!(
        p,
        Param::ModalExcite
            | Param::ModalDecay
            | Param::ModalDamp
            | Param::ModalStretch
            | Param::ModalBright
            | Param::ModalSpread
            | Param::ModalModes
            | Param::ModalHard
            | Param::ModalBurst
            | Param::ModalMute
    )
}

/// Every knob an instrument owns (the parser checks the family against
/// the track's kind; `excite_gain` belongs to both bodies).
pub fn is_instrument_param(p: Param) -> bool {
    is_string_param(p) || is_modal_param(p) || p == Param::ExciteGain
}

/// The volts a struck or plucked body puts on the bus at full velocity:
/// the same order as one oscillator (PROGRAM_V), so an instrument sits
/// beside the synth tracks without trimming.
const STRING_V: f32 = 0.6 * PROGRAM_V;
const MODAL_V: f32 = 14.0 * PROGRAM_V;

/// The lowest fundamental a voice's delay lines are sized for: below the
/// piano's A0 with room for bends.
const MIN_STRING_HZ: f64 = 16.0;

/// First-order allpass sections in the dispersion cascade.
const DISPERSION_SECTIONS: usize = 32;

/// Voices per instrument: a struck string or bar rings for seconds, so
/// polyphony is the ring-out, not the held notes.
const STRING_VOICES: usize = 16;
const MODAL_VOICES: usize = 8;

/// How hard an excite track drives a body at `excite_gain=1`: a drum
/// track exciting a plate or a string comes out about as loud as the
/// drums. (A pitched source on a body tuned to it resonates louder: the
/// matched modes ring up; turn `excite_gain` down.)
const STRING_EXCITE: f32 = 0.45;
const MODAL_EXCITE: f32 = 0.18;

/// The noise bow's level (unit-variance noise times this): eight times
/// the reference's 0.02, so a bowed bank sits level with a struck one
const NOISE_BOW: f64 = 0.16;

/// Mode cap of one bank voice.
pub const MAX_MODES: usize = 512;

/// Below this the voice is silent and stops computing (-120 dB on the
/// internal 1.0 scale).
const SILENCE: f64 = 1e-6;

// ---------------------------------------------------------------------------
// Mode sets
// ---------------------------------------------------------------------------

/// One modal body: every mode's frequency ratio, base T60 (seconds) and
/// gain. A preset's gains are None: the bank's `modal_bright` shape gives
/// them (ratio^(-1 + bright)), as in the reference.
#[derive(Clone, Debug, PartialEq)]
pub struct ModeSet {
    pub ratios: Vec<f64>,
    pub t60: Vec<f64>,
    pub gains: Vec<Option<f64>>,
}

/// The built-in bodies. The first five are the reference's (bars, plates
/// and bowls from the classic physics); gong and cymbal are dense circular
/// plates after Chladni's law (f ~ (m + 2n)^p), a few hundred modes each.
pub const MODAL_PRESETS: &[&str] = &["bar", "glass", "plate", "bell", "string", "gong", "cymbal"];

impl ModeSet {
    fn uniform(ratios: Vec<f64>, t60: f64) -> Self {
        let n = ratios.len();
        ModeSet {
            ratios,
            t60: vec![t60; n],
            gains: vec![None; n],
        }
    }

    /// A built-in body by name.
    pub fn preset(name: &str) -> Option<ModeSet> {
        Some(match name {
            // free-free bar, asymptotic ((2k+1)/3)^2 after the fundamental
            "bar" => {
                let mut r = vec![1.0];
                r.extend((2..13).map(|k| ((2 * k + 1) as f64 / 3.0).powi(2)));
                ModeSet::uniform(r, 2.5)
            }
            "glass" => ModeSet::uniform(vec![1.0, 2.32, 4.25, 6.63, 9.38, 12.5, 16.0, 19.9], 6.0),
            // square plate: sqrt(m^2 + n^2), normalised to the (1,1) mode
            "plate" => {
                let mut sums: Vec<u32> = (1..8u32)
                    .flat_map(|m| (1..8u32).map(move |n| m * m + n * n))
                    .collect();
                sums.sort_unstable();
                sums.dedup();
                let r = sums
                    .iter()
                    .take(40)
                    .map(|&s| (s as f64).sqrt() / 2f64.sqrt())
                    .collect();
                ModeSet::uniform(r, 3.0)
            }
            // church bell: hum, prime, minor third, fifth, nominal ...
            "bell" => ModeSet::uniform(
                vec![0.5, 1.0, 1.2, 1.5, 2.0, 2.5, 3.0, 4.0, 5.33, 6.67, 8.0],
                5.0,
            ),
            "string" => ModeSet::uniform((1..41).map(|k| k as f64).collect(), 3.0),
            "gong" => chladni(1.75, 0.004, 60.0, 8.0),
            "cymbal" => chladni(1.4, 0.011, 120.0, 4.0),
            _ => return None,
        })
    }

    /// One `modes` token: `ratio`, `ratio:t60` or `ratio:t60:gain`.
    pub fn parse_mode(tok: &str) -> Result<(f64, f64, Option<f64>), String> {
        let mut it = tok.split(':');
        let num = |s: Option<&str>, what: &str| -> Result<Option<f64>, String> {
            match s {
                None | Some("") => Ok(None),
                Some(s) => s
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .map(Some)
                    .ok_or_else(|| format!("invalid mode {} '{}' in '{}'", what, s, tok)),
            }
        };
        let ratio = num(it.next(), "ratio")?.ok_or_else(|| format!("empty mode '{}'", tok))?;
        let t60 = num(it.next(), "t60")?.unwrap_or(3.0);
        let gain = num(it.next(), "gain")?;
        if it.next().is_some() {
            return Err(format!("a mode is ratio[:t60[:gain]], got '{}'", tok));
        }
        if !(ratio > 0.0 && ratio <= 1000.0) {
            return Err(format!("mode ratio {} must be in (0, 1000]", ratio));
        }
        if !(t60 > 0.0 && t60 <= 600.0) {
            return Err(format!("mode t60 {} must be in (0, 600] seconds", t60));
        }
        if let Some(g) = gain {
            if !(0.0..=1000.0).contains(&g) {
                return Err(format!("mode gain {} must be in 0..1000", g));
            }
        }
        Ok((ratio, t60, gain))
    }

    pub fn len(&self) -> usize {
        self.ratios.len()
    }
}

/// A dense circular plate: modes (m diameters, n circles) at
/// ((m + 2n) / 2)^p, the degenerate pairs split by `split` so they beat,
/// up to `top` times the lowest mode.
fn chladni(p: f64, split: f64, top: f64, t60: f64) -> ModeSet {
    let mut r = Vec::new();
    for s in 2..200u32 {
        let base = (s as f64 / 2.0).powf(p);
        if base > top {
            break;
        }
        // every (m, n) with m + 2n = s; m > 0 modes come in sin/cos pairs
        let mut j = 0.0;
        for n in 0..=s / 2 {
            let m = s - 2 * n;
            let copies = if m == 0 { 1 } else { 2 };
            for _ in 0..copies {
                r.push(base * (1.0 + split * j));
                j += 1.0;
            }
        }
    }
    r.sort_by(|a, b| a.partial_cmp(b).unwrap());
    r.truncate(MAX_MODES);
    let lowest = r[0];
    for x in &mut r {
        *x /= lowest;
    }
    ModeSet::uniform(r, t60)
}

// ---------------------------------------------------------------------------
// Instrument specs (what a song track asks for)
// ---------------------------------------------------------------------------

/// The body a track plays.
#[derive(Clone, Debug, PartialEq)]
pub enum InstrumentKind {
    String,
    Modal(ModeSet),
}

/// One instrument track: its body and the channel whose audio excites it.
#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentSpec {
    pub kind: InstrumentKind,
    pub excite: Option<u16>,
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// The waveguide's knobs, in song units.
#[derive(Clone, Copy, Debug)]
pub struct StringParams {
    /// 0 pluck, 1 strike, 2 bow, 3 none (only the excite input)
    pub excite: u8,
    /// T60 of the fundamental, seconds
    pub decay: f32,
    /// high partials decay faster: T60(f) = decay / (1 + damp (f/f1 - 1))
    pub damp: f32,
    /// inharmonicity coefficient B
    pub stiff: f32,
    /// pluck / hammer / bow point, fraction of the length from the bridge
    pub pos: f32,
    /// plectrum and hammer hardness (0 felt .. 1 steel)
    pub hard: f32,
    /// bow pressure
    pub pressure: f32,
    /// how hard a note-off damps the string (0 rings on, 1 stops it)
    pub mute: f32,
}

impl Default for StringParams {
    fn default() -> Self {
        StringParams {
            excite: 0,
            decay: 4.0,
            damp: 0.5,
            stiff: 0.0,
            pos: 0.13,
            hard: 0.5,
            pressure: 0.5,
            mute: 0.0,
        }
    }
}

/// The modal bank's knobs (the reference's names, `modal_` dropped).
#[derive(Clone, Copy, Debug)]
pub struct ModalParams {
    /// 0 strike, 1 noise burst, 2 bow, 3 none (only the excite input)
    pub excite: u8,
    pub decay: f32,
    pub damp: f32,
    pub stretch: f32,
    pub bright: f32,
    pub spread: f32,
    pub modes: usize,
    pub hard: f32,
    pub burst: f32,
    pub mute: f32,
}

impl Default for ModalParams {
    fn default() -> Self {
        ModalParams {
            excite: 0,
            decay: 1.0,
            damp: 0.5,
            stretch: 0.0,
            bright: 0.0,
            spread: 0.0,
            modes: MAX_MODES,
            hard: 0.7,
            burst: 0.05,
            mute: 0.0,
        }
    }
}

/// The envelope a bow, a noise bow or the excite input's gate follows,
/// from the track's own `attack` and `release` (seconds).
#[derive(Clone, Copy, Debug)]
pub struct Gate {
    pub attack: f32,
    pub release: f32,
}

impl Default for Gate {
    fn default() -> Self {
        Gate {
            attack: 0.1,
            release: 0.2,
        }
    }
}

impl Gate {
    /// Per-sample linear steps up and down.
    fn steps(&self, sr: f64) -> (f64, f64) {
        (
            1.0 / (self.attack.max(0.002) as f64 * sr),
            1.0 / (self.release.max(0.005) as f64 * sr),
        )
    }
}

fn note_hz(note: u8, cents: f32) -> f64 {
    440.0 * ((note as f64 - 69.0 + cents as f64 / 100.0) / 12.0).exp2()
}

// ---------------------------------------------------------------------------
// The waveguide string
// ---------------------------------------------------------------------------

/// Phase delay (samples) of the first-order allpass (a + z^-1)/(1 + a z^-1).
/// The dispersion cascade: `n` identical first-order allpasses of
/// coefficient `a` (n = 0: no cascade).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Disp {
    n: usize,
    a: f64,
}

/// Phase delay of the dispersion cascade at `w`.
fn disp_delay(w: f64, d: &Disp) -> f64 {
    if d.n == 0 {
        0.0
    } else {
        d.n as f64 * allpass_delay(w, d.a)
    }
}

fn allpass_delay(w: f64, a: f64) -> f64 {
    2.0 / w * ((1.0 - a) / (1.0 + a) * (w / 2.0).tan()).atan()
}

/// Phase delay of the loss filter b/(1 - p z^-1).
fn onepole_delay(w: f64, p: f64) -> f64 {
    (p * w.sin()).atan2(1.0 - p * w.cos()) / w
}

/// |1 - p e^{-jw}|, the loss filter's denominator magnitude.
fn onepole_den(w: f64, p: f64) -> f64 {
    (1.0 - 2.0 * p * w.cos() + p * p).sqrt()
}

/// A DC blocker's coefficient, (1 - z^-1)/(1 - R z^-1) at `hz`. The loop
/// itself has none (its phase lead would flatten the low overtones); what
/// goes into the loop is DC-free instead, see `StringVoice::start`.
fn dc_coef(sr: f64, hz: f64) -> f64 {
    1.0 - 2.0 * PI * hz / sr
}

/// The four Lagrange weights for a read `d` samples back (d >= 2): taps at
/// delays floor(d)-1 .. floor(d)+2, the fraction centred between the middle two.
fn lagrange(d: f64) -> (usize, [f64; 4]) {
    let i = d.floor();
    let x = d - i + 1.0;
    let h = [
        -(x - 1.0) * (x - 2.0) * (x - 3.0) / 6.0,
        x * (x - 2.0) * (x - 3.0) / 2.0,
        -x * (x - 1.0) * (x - 3.0) / 2.0,
        x * (x - 1.0) * (x - 2.0) / 6.0,
    ];
    (i as usize - 1, h)
}

/// The interpolated read's response at `w`: (phase delay in samples, gain).
fn lagrange_response(d: f64, w: f64) -> (f64, f64) {
    let (first, h) = lagrange(d);
    let (mut re, mut im) = (0.0, 0.0);
    for (k, hk) in h.iter().enumerate() {
        let ph = -w * k as f64;
        re += hk * ph.cos();
        im += hk * ph.sin();
    }
    (first as f64 - im.atan2(re) / w, (re * re + im * im).sqrt())
}

/// What a voice's loop is built from, solved for one pitch.
#[derive(Clone, Copy, Debug)]
struct LoopDesign {
    /// bridge segment, whole samples
    na: usize,
    /// nut segment, fractional samples (Lagrange read)
    nb: f64,
    /// loss filter: y = g (1 - p) x + p y
    g: f64,
    p: f64,
    /// dispersion: the allpass cascade (`n` sections of coefficient `a`)
    a: Disp,
}

/// Solve a loop for fundamental `f1`: the loss filter for the decay, the
/// dispersion cascade for the stiffness, then the two delay lengths so the
/// loop's total phase delay at f1 is exactly one period.
fn design_loop(sr: f64, f1: f64, sp: &StringParams, cached_a: Option<Disp>) -> LoopDesign {
    let w1 = 2.0 * PI * f1 / sr;
    let period = sr / f1;
    let (p, g1) = design_loss(sr, f1, sp.decay as f64, sp.damp as f64);
    let a = cached_a.unwrap_or_else(|| fit_dispersion(sr, f1, sp.stiff as f64, p));
    let disp = disp_delay(w1, &a);
    let pure = (period - disp - onepole_delay(w1, p)).max(3.0);
    let pos = (sp.pos as f64).clamp(0.01, 0.5);
    // the excitation point: the bridge side's round trip, filters and all,
    // is `pos` of the loop (as near as a stiff string's cascade allows)
    let na = ((pos * period - disp - onepole_delay(w1, p)).round().max(1.0) as usize)
        .clamp(1, (pure - 2.0).floor().max(1.0) as usize);
    let (nb, lag_gain) = solve_nut(pure - na as f64, w1);
    // the interpolator's droop at f1 is part of the loop gain
    let g = (g1 * onepole_den(w1, p) / (1.0 - p) / lag_gain).min(0.99995);
    LoopDesign { na, nb, g, p, a }
}

/// The nut segment's read position whose phase delay at `w1` is `want`
/// samples (Newton on the interpolator's own response), and its gain there.
fn solve_nut(want: f64, w1: f64) -> (f64, f64) {
    let want = want.max(2.0);
    let mut d = want;
    for _ in 0..4 {
        let (pd, _) = lagrange_response(d, w1);
        d = (d + want - pd).max(2.0);
    }
    (d, lagrange_response(d, w1).1)
}

/// The loss filter: per-pass gain at the fundamental for exactly `t60`
/// seconds, and a pole that makes the partial at 8 f1 (or 0.4 Nyquist)
/// decay as T60(f) = t60 / (1 + damp (f/f1 - 1)) says. Returns (p, G1).
fn design_loss(sr: f64, f1: f64, t60: f64, damp: f64) -> (f64, f64) {
    let per_pass = |f: f64| -> f64 {
        let t = t60 / (1.0 + damp * (f / f1 - 1.0)).max(1e-3);
        10f64.powf(-3.0 / (t * f1))
    };
    let g1 = per_pass(f1);
    let w1 = 2.0 * PI * f1 / sr;
    let w2 = (8.0 * w1).min(0.4 * PI);
    if damp <= 0.0 || w2 < 1.5 * w1 {
        return (0.0, g1);
    }
    let f2 = w2 * sr / (2.0 * PI);
    let want = per_pass(f2) / g1;
    let ratio = |p: f64| onepole_den(w1, p) / onepole_den(w2, p);
    let (mut lo, mut hi) = (0.0, 0.999);
    if ratio(hi) > want {
        return (hi, g1);
    }
    for _ in 0..50 {
        let mid = 0.5 * (lo + hi);
        if ratio(mid) > want {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (0.5 * (lo + hi), g1)
}

/// The dispersion cascade for inharmonicity `b`: the partials of a stiff
/// string sit at k f0 sqrt(1 + b k^2) (f0 the flexible string's pitch, so
/// the first partial is the note). For 8, 16 and 32 identical first-order
/// allpasses, the coefficient that puts the loop's phase at 2 pi k at those
/// partials (k = 2..16, below 0.45 pi; the error weighted in cents) is
/// found by a scan over each section's DC delay and a golden-section
/// refinement; the cascade with the least error wins. More, smaller
/// sections follow the stiff string's gently falling delay further up;
/// the cascade may take 85% of the loop, so very stiff high strings run
/// out of room and stretch less than asked.
fn fit_dispersion(sr: f64, f1: f64, b: f64, p: f64) -> Disp {
    if b <= 1e-8 {
        return Disp::default();
    }
    let w1 = 2.0 * PI * f1 / sr;
    let period = sr / f1;
    let f0 = f1 / (1.0 + b).sqrt();
    let targets: Vec<(f64, f64)> = (2..=16)
        .map(|k| {
            let k = k as f64;
            (k, 2.0 * PI * f0 * k * (1.0 + b * k * k).sqrt() / sr)
        })
        .filter(|&(_, w)| w < 0.45 * PI)
        .collect();
    if targets.is_empty() {
        return Disp::default();
    }
    let fixed = |w: f64| onepole_delay(w, p);
    let room = (0.85 * period).min(period - 6.0);
    let mut best = (f64::INFINITY, Disp::default());
    for n in [8usize, 16, 32] {
        let m = n as f64;
        // each section's DC delay t (a = (1 - t)/(1 + t)), from one sample
        // up to its share of the room
        let t_max = room / m;
        if t_max <= 1.05 {
            continue;
        }
        let cost = |lt: f64| -> f64 {
            let t = lt.exp();
            let a = (1.0 - t) / (1.0 + t);
            let at_f1 = m * allpass_delay(w1, a);
            if at_f1 > room {
                return f64::INFINITY;
            }
            let pure = period - at_f1 - fixed(w1);
            targets
                .iter()
                .map(|&(k, w)| {
                    let e = (w * (pure + m * allpass_delay(w, a) + fixed(w)) - 2.0 * PI * k) / k;
                    e * e
                })
                .sum()
        };
        let (lo, hi) = (0.0, t_max.ln());
        let steps = 48;
        let mut at = (0, f64::INFINITY);
        for i in 0..=steps {
            let c = cost(lo + (hi - lo) * i as f64 / steps as f64);
            if c < at.1 {
                at = (i, c);
            }
        }
        let h = (hi - lo) / steps as f64;
        let (mut x0, mut x3) = (lo + h * (at.0 as f64 - 1.0), lo + h * (at.0 as f64 + 1.0));
        x0 = x0.max(lo);
        x3 = x3.min(hi);
        let gr = 0.5 * (5f64.sqrt() - 1.0);
        let mut x1 = x3 - gr * (x3 - x0);
        let mut x2 = x0 + gr * (x3 - x0);
        let (mut c1, mut c2) = (cost(x1), cost(x2));
        for _ in 0..40 {
            if c1 < c2 {
                x3 = x2;
                x2 = x1;
                c2 = c1;
                x1 = x3 - gr * (x3 - x0);
                c1 = cost(x1);
            } else {
                x0 = x1;
                x1 = x2;
                c1 = c2;
                x2 = x0 + gr * (x3 - x0);
                c2 = cost(x2);
            }
        }
        let lt = 0.5 * (x0 + x3);
        let c = cost(lt);
        if c < best.0 {
            let t = lt.exp();
            best = (c, Disp { n, a: (1.0 - t) / (1.0 + t) });
        }
    }
    best.1
}

/// A circular delay line read at integer or fractional delays (delay 1 is
/// the newest sample).
struct Line {
    buf: Vec<f32>,
    w: usize,
}

impl Line {
    fn new(len: usize) -> Self {
        Line {
            buf: vec![0.0; len.max(8)],
            w: 0,
        }
    }

    #[inline]
    fn push(&mut self, x: f32) {
        self.buf[self.w] = x;
        self.w += 1;
        if self.w == self.buf.len() {
            self.w = 0;
        }
    }

    #[inline]
    fn at(&self, delay: usize) -> f32 {
        let n = self.buf.len();
        self.buf[(self.w + n - delay) % n]
    }

    #[inline]
    fn frac(&self, first: usize, h: &[f32; 4]) -> f32 {
        self.at(first) * h[0] + self.at(first + 1) * h[1] + self.at(first + 2) * h[2] + self.at(first + 3) * h[3]
    }

    fn clear(&mut self) {
        self.buf.iter_mut().for_each(|x| *x = 0.0);
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Strike {
    /// a lowpassed noise burst one period long (the plectrum)
    Pluck,
    /// a raised-cosine force pulse (the hammer)
    Hammer,
}

/// A note bend in flight: `oct` octaves, progress `x` stepping `inc` a
/// sample, shaped by the track's bend curve (`power`, see voice.rs).
#[derive(Clone, Copy, Debug, Default)]
struct Bend {
    oct: f64,
    x: f64,
    inc: f64,
    power: f64,
}

impl Bend {
    fn offset(&self) -> f64 {
        if self.oct == 0.0 {
            0.0
        } else {
            self.oct * (1.0 - (1.0 - self.x.min(1.0)).powf(self.power))
        }
    }

    fn moving(&self) -> bool {
        self.oct != 0.0 && self.x < 1.0
    }
}

/// One string.
struct StringVoice {
    key: u8,
    held: bool,
    active: bool,
    age: u64,
    /// the note's own pitch and the bend riding on it
    base_hz: f64,
    hz: f64,
    bend: Bend,
    bridge: Line,
    nut: Line,
    design: LoopDesign,
    taps_first: usize,
    taps: [f32; 4],
    lp_b: f32,
    lp_p: f32,
    lp_z: f32,
    /// a DC blocker on the excite input
    dc_r: f32,
    in_x: f32,
    in_y: f32,
    ap_n: usize,
    ap_a: f32,
    ap_z: [f32; DISPERSION_SECTIONS],
    /// extra loss on the nut reflection after a damped note-off
    nut_gain: f32,
    nut_target: f32,
    /// the excitation in flight
    strike: Option<Strike>,
    exc_n: usize,
    exc_len: usize,
    exc_amp: f32,
    exc_c: f32,
    exc_z: f32,
    /// subtracted over the first period: the excitation's own mean, so
    /// nothing reaches the loop's 0 Hz mode (a rectangle one period long
    /// is zero at every harmonic, so the tone is untouched)
    exc_dc: f32,
    exc_dc_len: usize,
    rng: Rng,
    /// the bow and the excite input's gate (0..1), the bow's speed and
    /// its reciprocal
    gate: f64,
    bow_speed: f32,
    bow_inv: f32,
    n_sample: u64,
    /// output peak follower, for sleeping
    level: f64,
    quiet: u32,
}

/// What every string of one instrument reads while it renders.
#[derive(Clone, Copy)]
struct StringCtx {
    sr: f64,
    p: StringParams,
    gate_steps: (f64, f64),
    /// the bow table's slope at unit bow speed, from the pressure
    bow_width: f32,
}

impl StringVoice {
    fn new(sr: f64, index: usize) -> Self {
        let cap = (sr / MIN_STRING_HZ).ceil() as usize + 16;
        StringVoice {
            key: 0,
            held: false,
            active: false,
            age: 0,
            base_hz: 110.0,
            hz: 110.0,
            bend: Bend::default(),
            bridge: Line::new(cap / 2 + 16),
            nut: Line::new(cap),
            design: LoopDesign {
                na: 1,
                nb: 2.0,
                g: 0.0,
                p: 0.0,
                a: Disp::default(),
            },
            taps_first: 1,
            taps: [0.0, 1.0, 0.0, 0.0],
            lp_b: 0.0,
            lp_p: 0.0,
            lp_z: 0.0,
            dc_r: dc_coef(sr, 5.0) as f32,
            in_x: 0.0,
            in_y: 0.0,
            ap_n: 0,
            ap_a: 0.0,
            ap_z: [0.0; DISPERSION_SECTIONS],
            nut_gain: 1.0,
            nut_target: 1.0,
            strike: None,
            exc_n: 0,
            exc_len: 0,
            exc_amp: 0.0,
            exc_c: 0.0,
            exc_z: 0.0,
            exc_dc: 0.0,
            exc_dc_len: 0,
            rng: Rng::new(crate::rng::seed(0x5712 + index as u32)),
            gate: 0.0,
            bow_speed: 0.0,
            bow_inv: 0.0,
            n_sample: 0,
            level: 0.0,
            quiet: 0,
        }
    }

    fn apply_design(&mut self, d: LoopDesign) {
        self.design = d;
        let (first, h) = lagrange(d.nb);
        self.taps_first = first;
        self.taps = [h[0] as f32, h[1] as f32, h[2] as f32, h[3] as f32];
        self.lp_b = (d.g * (1.0 - d.p)) as f32;
        self.lp_p = d.p as f32;
        self.ap_n = d.a.n;
        self.ap_a = d.a.a as f32;
    }

    /// Re-solve the nut segment for the current (bent) pitch, keeping the
    /// bridge segment, the loss filter and the dispersion as they are.
    fn retune(&mut self, sr: f64) {
        let w1 = 2.0 * PI * self.hz / sr;
        let d = self.design;
        let disp = disp_delay(w1, &d.a);
        let want = sr / self.hz - disp - onepole_delay(w1, d.p) - d.na as f64;
        let cap = (self.nut.buf.len() - 4) as f64;
        let (nb, _) = solve_nut(want.min(cap), w1);
        let (first, h) = lagrange(nb);
        self.design.nb = nb;
        self.taps_first = first;
        self.taps = [h[0] as f32, h[1] as f32, h[2] as f32, h[3] as f32];
    }

    fn start(&mut self, ctx: &StringCtx, disp: Disp, key: u8, hz: f64, vel: f32, age: u64) {
        let sr = ctx.sr;
        let fresh = !self.active;
        self.key = key;
        self.held = true;
        self.active = true;
        self.age = age;
        self.base_hz = hz;
        self.hz = hz;
        self.bend = Bend::default();
        if fresh {
            self.bridge.clear();
            self.nut.clear();
            self.lp_z = 0.0;
            self.in_x = 0.0;
            self.in_y = 0.0;
            self.ap_z = [0.0; DISPERSION_SECTIONS];
            self.gate = 0.0;
        }
        self.apply_design(design_loop(sr, hz, &ctx.p, Some(disp)));
        self.nut_gain = 1.0;
        self.nut_target = 1.0;
        self.level = 1.0;
        self.quiet = 0;
        let vel = vel.clamp(0.0, 1.0);
        let hard = ctx.p.hard.clamp(0.0, 1.0);
        self.strike = match ctx.p.excite {
            0 => Some(Strike::Pluck),
            1 => Some(Strike::Hammer),
            _ => None,
        };
        self.exc_n = 0;
        self.exc_z = 0.0;
        match self.strike {
            Some(Strike::Pluck) => {
                // a noise burst one period long through the plectrum's
                // lowpass: harder picks and harder playing are brighter
                self.exc_len = (sr / hz).round().max(2.0) as usize;
                let fc = (hz * (1.5 + 40.0 * (hard * hard) as f64 * (0.3 + 0.7 * vel as f64)))
                    .min(0.45 * sr);
                self.exc_c = (-2.0 * PI * fc / sr).exp() as f32;
                self.exc_amp = vel * 1.6;
                // the burst's mean, from a dry run of the same noise
                let (mut rng, mut z, mut sum) = (self.rng, 0.0f32, 0.0f32);
                for _ in 0..self.exc_len {
                    z = rng.bipolar() * (1.0 - self.exc_c) + self.exc_c * z;
                    sum += z * self.exc_amp;
                }
                self.exc_dc = sum / self.exc_len as f32;
            }
            Some(Strike::Hammer) => {
                // contact time shrinks with a harder felt and a harder
                // blow; the impulse (the pulse's area) follows the velocity
                let tc = (0.0045 - 0.004 * hard as f64) * (1.25 - 0.5 * vel as f64);
                self.exc_len = (tc * sr).round().max(2.0) as usize;
                self.exc_amp = vel * 4.0 * (0.0005 * sr) as f32 / self.exc_len as f32;
                // the pulse's area, spread back out over one period
                self.exc_dc = self.exc_amp * 0.5 * self.exc_len as f32 / (sr / hz) as f32;
            }
            None => self.exc_dc = 0.0,
        }
        self.exc_dc_len = (sr / hz).round() as usize;
        // the bow's speed follows the velocity
        self.bow_speed = 0.015 + 0.115 * vel;
        self.bow_inv = 1.0 / self.bow_speed;
    }

    fn release(&mut self, ctx: &StringCtx) {
        self.held = false;
        let mute = ctx.p.mute.clamp(0.0, 1.0) as f64;
        if mute > 0.0 {
            // per pass: from the string's own T60 toward 60 ms
            let t60 = (ctx.p.decay as f64).max(0.05);
            let t_rel = (t60.ln() + mute * (0.06f64.ln() - t60.ln())).exp();
            let extra = 10f64.powf(-3.0 / (t_rel * self.hz) + 3.0 / (t60 * self.hz));
            self.nut_target = extra as f32;
        }
    }

    /// One sample. `input` is the excite track's audio (already scaled).
    #[inline]
    fn render(&mut self, ctx: &StringCtx, input: f32) -> f32 {
        let (up, down) = ctx.gate_steps;
        if self.held {
            self.gate = (self.gate + up).min(1.0);
        } else {
            self.gate = (self.gate - down).max(0.0);
        }
        self.nut_gain += (self.nut_target - self.nut_gain) * 0.002;

        // the bridge reflection: loss, dispersion, inversion
        let a_out = self.bridge.at(self.design.na);
        let mut x = self.lp_b * a_out + self.lp_p * self.lp_z;
        self.lp_z = x;
        if self.ap_n > 0 {
            let a = self.ap_a;
            for z in self.ap_z[..self.ap_n].iter_mut() {
                let y = a * x + *z;
                *z = x - a * y;
                x = y;
            }
        }
        let br = -x;
        // the nut reflection (and the damper's extra loss)
        let nr = -self.nut.frac(self.taps_first, &self.taps) * self.nut_gain;

        // the excitation at the junction: the excite input (DC-blocked),
        // the pluck or hammer, less their mean over the first period
        let gated = input * self.gate as f32;
        let mut e = gated - self.in_x + self.dc_r * self.in_y;
        self.in_x = gated;
        self.in_y = e;
        if self.exc_dc_len > 0 {
            e -= self.exc_dc;
            self.exc_dc_len -= 1;
        }
        if let Some(kind) = self.strike {
            match kind {
                Strike::Pluck => {
                    let noise = self.rng.bipolar();
                    self.exc_z = noise * (1.0 - self.exc_c) + self.exc_c * self.exc_z;
                    e += self.exc_z * self.exc_amp;
                }
                Strike::Hammer => {
                    let t = (self.exc_n as f32 + 0.5) / self.exc_len as f32;
                    e += self.exc_amp * 0.5 * (1.0 - (std::f32::consts::TAU * t).cos());
                }
            }
            self.exc_n += 1;
            if self.exc_n >= self.exc_len {
                self.strike = None;
            }
        }
        let (into_bridge, into_nut) = if ctx.p.excite == 2 {
            // the bow: friction at the bow point, the STK's bow table with
            // its width scaled to the bow's speed, so the speed (the note's
            // velocity) sets the loudness and pressure alone the motion.
            // The bow leaves the string as the gate closes.
            let contact = self.gate as f32;
            let vb = self.bow_speed * contact;
            let dv = vb - (br + nr);
            let slope = ctx.bow_width * self.bow_inv;
            let f = contact * dv * ((dv * slope).abs() + 0.75).powi(-4).min(1.0);
            (nr + f + e, br + f + e)
        } else {
            (nr + e, br + e)
        };
        self.bridge.push(into_bridge);
        self.nut.push(into_nut);

        // the bend moves the nut end, a finger sliding along the string
        self.n_sample += 1;
        if self.bend.moving() {
            self.bend.x += self.bend.inc;
            if self.n_sample % 16 == 0 || self.bend.x >= 1.0 {
                self.hz = self.base_hz * self.bend.offset().exp2();
                self.retune(ctx.sr);
            }
        }

        let mag = (a_out as f64).abs();
        if mag > self.level {
            self.level = mag;
        } else {
            self.level *= 0.9995;
        }
        if !self.held && self.strike.is_none() && self.gate == 0.0 && self.level < SILENCE {
            self.quiet += 1;
            if self.quiet > 2048 {
                self.active = false;
            }
        } else {
            self.quiet = 0;
        }
        if !a_out.is_finite() {
            // never let a bad input poison the loop for good
            self.active = false;
            self.bridge.clear();
            self.nut.clear();
            return 0.0;
        }
        a_out
    }
}

/// A polyphonic string instrument.
pub struct StringInstrument {
    sr: f64,
    pub p: StringParams,
    gate_steps: (f64, f64),
    voices: Vec<StringVoice>,
    counter: u64,
    /// fitted dispersion coefficients, by (pitch, stiffness, decay, damp)
    disp_cache: Vec<([u32; 4], Disp)>,
    /// the output's DC blocker
    out_r: f32,
    out_x: f32,
    out_y: f32,
}

impl StringInstrument {
    pub fn new(sr: f32) -> Self {
        let sr = sr as f64;
        StringInstrument {
            sr,
            p: StringParams::default(),
            gate_steps: Gate::default().steps(sr),
            voices: (0..STRING_VOICES).map(|i| StringVoice::new(sr, i)).collect(),
            counter: 0,
            disp_cache: Vec::new(),
            out_r: dc_coef(sr, 5.0) as f32,
            out_x: 0.0,
            out_y: 0.0,
        }
    }

    fn ctx(&self) -> StringCtx {
        StringCtx {
            sr: self.sr,
            p: self.p,
            gate_steps: self.gate_steps,
            // pressure 0..1: a light bow (0.6) to a heavy one (0.1); 0.5
            // holds Helmholtz motion near the bridge over every decay
            bow_width: 0.6 * 6f32.powf(-self.p.pressure.clamp(0.0, 1.0)),
        }
    }

    /// The pitch range a string's loop can hold.
    fn clamp_hz(&self, hz: f64) -> f64 {
        hz.clamp(MIN_STRING_HZ * 1.5, self.sr / 6.0)
    }

    fn dispersion_for(&mut self, hz: f64) -> Disp {
        let p = self.p;
        let key = [
            (hz as f32).to_bits(),
            p.stiff.to_bits(),
            p.decay.to_bits(),
            p.damp.to_bits(),
        ];
        if let Some(&(_, a)) = self.disp_cache.iter().find(|(k, _)| *k == key) {
            return a;
        }
        let (pole, _) = design_loss(self.sr, hz, p.decay as f64, p.damp as f64);
        let a = fit_dispersion(self.sr, hz, p.stiff as f64, pole);
        if self.disp_cache.len() >= 128 {
            self.disp_cache.clear();
        }
        self.disp_cache.push((key, a));
        a
    }

    pub fn note_on(&mut self, key: u8, hz: f64, vel: f32) {
        self.counter += 1;
        let age = self.counter;
        let hz = self.clamp_hz(hz);
        // a key already ringing is struck again where it is; else a free
        // voice, else the quietest released one, else the oldest
        let idx = self
            .voices
            .iter()
            .position(|v| v.active && v.key == key)
            .or_else(|| self.voices.iter().position(|v| !v.active))
            .or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| !v.held)
                    .min_by(|a, b| a.1.level.total_cmp(&b.1.level))
                    .map(|(i, _)| i)
            })
            .unwrap_or_else(|| {
                (0..self.voices.len())
                    .min_by_key(|&i| self.voices[i].age)
                    .unwrap()
            });
        let disp = self.dispersion_for(hz);
        let ctx = self.ctx();
        self.voices[idx].start(&ctx, disp, key, hz, vel, age);
    }

    pub fn note_off(&mut self, key: u8) {
        let ctx = self.ctx();
        for v in self.voices.iter_mut().filter(|v| v.held && v.key == key) {
            v.release(&ctx);
        }
    }

    pub fn bend(&mut self, key: u8, semitones: f32, seconds: f32, power: f32) {
        let sr = self.sr;
        for v in self.voices.iter_mut().filter(|v| v.held && v.key == key) {
            v.base_hz = v.hz;
            v.bend = Bend {
                oct: semitones as f64 / 12.0,
                x: 0.0,
                inc: 1.0 / (seconds.max(1e-3) as f64 * sr),
                power: power as f64,
            };
        }
    }

    pub fn set_gate(&mut self, gate: Gate) {
        self.gate_steps = gate.steps(self.sr);
    }

    /// The decay or damping moved: sounding strings re-solve their loss
    /// filters (and the nut segment for the new filter's phase) at once.
    pub fn changed(&mut self) {
        let ctx = self.ctx();
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let na = v.design.na;
            let mut d = design_loop(ctx.sr, v.hz, &ctx.p, Some(v.design.a));
            d.na = na;
            v.apply_design(d);
            v.retune(ctx.sr);
        }
    }

    pub fn render(&mut self, input: f32) -> f32 {
        let ctx = self.ctx();
        let mut out = 0.0;
        for v in self.voices.iter_mut().filter(|v| v.active) {
            out += v.render(&ctx, input);
        }
        // the bow's steady drag leaves the loop an offset: block it at the
        // output, where it cannot touch the string's own motion
        let y = out - self.out_x + self.out_r * self.out_y;
        self.out_x = out;
        self.out_y = y;
        y * STRING_V
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
}

// ---------------------------------------------------------------------------
// The modal bank
// ---------------------------------------------------------------------------

/// The reference's strike: a one-sample impulse through a one-pole lowpass
/// for 64 samples (a soft mallet to a hard one), scaled to a sum of 40.
pub fn strike_shape(sr: f64, hardness: f64) -> [f64; 64] {
    let a = (-2.0 * PI * (500.0 + 15000.0 * hardness) / sr).exp();
    let mut x = [0.0; 64];
    x[0] = 1.0;
    for i in 1..64 {
        x[i] = x[i - 1] * a;
    }
    let norm = (1.0 - a) / (1.0 - a.powi(64)) * 40.0;
    for v in &mut x {
        *v *= norm;
    }
    x
}

/// One bank of two-pole resonators, one per mode.
struct ModalVoice {
    key: u8,
    held: bool,
    active: bool,
    age: u64,
    base_hz: f64,
    hz: f64,
    bend: Bend,
    vel: f64,
    n: usize,
    a1: Vec<f64>,
    a2: Vec<f64>,
    gin: Vec<f64>,
    gl: Vec<f64>,
    gr: Vec<f64>,
    y1: Vec<f64>,
    y2: Vec<f64>,
    /// the excitation in flight
    strike: Option<[f64; 64]>,
    burst_n: usize,
    burst_len: usize,
    burst_c: f64,
    burst_z: f64,
    rng: Rng,
    gate: f64,
    /// how far the damper has shortened every T60 (1 = not at all)
    damper: f64,
    n_sample: u64,
    energy: f64,
}

impl ModalVoice {
    fn new(index: usize) -> Self {
        ModalVoice {
            key: 0,
            held: false,
            active: false,
            age: 0,
            base_hz: 220.0,
            hz: 220.0,
            bend: Bend::default(),
            vel: 0.0,
            n: 0,
            a1: vec![0.0; MAX_MODES],
            a2: vec![0.0; MAX_MODES],
            gin: vec![0.0; MAX_MODES],
            gl: vec![0.0; MAX_MODES],
            gr: vec![0.0; MAX_MODES],
            y1: vec![0.0; MAX_MODES],
            y2: vec![0.0; MAX_MODES],
            strike: None,
            burst_n: 0,
            burst_len: 0,
            burst_c: 0.0,
            burst_z: 0.0,
            rng: Rng::new(crate::rng::seed(0x40da1 + index as u32)),
            gate: 0.0,
            damper: 1.0,
            n_sample: 0,
            energy: 0.0,
        }
    }

    /// The reference's coefficient pass: every mode's pole and gain for
    /// the voice's pitch, the bank's shapes and the damper.
    fn configure(&mut self, set: &ModeSet, p: &ModalParams, sr: f64) {
        let count = set.len().min(p.modes.max(1)).min(MAX_MODES);
        let stretch = p.stretch as f64;
        let mut kept = 0usize;
        let mut norm = 0.0;
        for m in 0..count {
            let r0 = set.ratios[m];
            let r = r0 * (1.0 + stretch * (r0 - 1.0));
            let f = self.hz * r;
            if !(f > 0.0 && f < sr / 2.0 * 0.95) {
                self.a1[m] = 0.0;
                self.a2[m] = 0.0;
                self.gin[m] = 0.0;
                self.y1[m] = 0.0;
                self.y2[m] = 0.0;
                continue;
            }
            let t60 = p.decay as f64 * set.t60[m] / (1.0 + p.damp as f64 * (r - 1.0)).max(0.05)
                * self.damper;
            let rr = 10f64.powf(-3.0 / (t60 * sr));
            let w = 2.0 * PI * f / sr;
            let g = set.gains[m].unwrap_or(1.0 / r) * r.powf(p.bright as f64);
            self.a1[m] = 2.0 * rr * w.cos();
            self.a2[m] = -rr * rr;
            // gin holds g for now; (1 - r) and the normalisation below
            self.gin[m] = g;
            self.gl[m] = 1.0 - rr; // stash (1 - r) until the pan pass
            norm += g * g;
            let pan = if kept % 2 == 0 { -p.spread } else { p.spread } as f64;
            self.gr[m] = pan;
            kept += 1;
        }
        let norm = if norm > 0.0 { 1.0 / norm.sqrt() } else { 0.0 };
        for m in 0..count {
            if self.a2[m] == 0.0 {
                continue;
            }
            let one_minus_r = self.gl[m];
            let pan = self.gr[m];
            self.gin[m] *= norm * one_minus_r;
            self.gl[m] = ((1.0 - pan) / 2.0).sqrt();
            self.gr[m] = ((1.0 + pan) / 2.0).sqrt();
        }
        for m in count..self.n {
            self.y1[m] = 0.0;
            self.y2[m] = 0.0;
        }
        self.n = count;
    }

    #[inline]
    fn render(&mut self, set: &ModeSet, p: &ModalParams, sr: f64, steps: (f64, f64), input: f64) -> (f64, f64) {
        let (up, down) = steps;
        if self.held {
            self.gate = (self.gate + up).min(1.0);
        } else {
            self.gate = (self.gate - down).max(0.0);
        }
        let mut x = input * self.gate;
        if let Some(s) = &self.strike {
            x += s[self.burst_n] * self.vel;
            self.burst_n += 1;
            if self.burst_n == 64 {
                self.strike = None;
            }
        } else if self.burst_n < self.burst_len {
            // a noise burst, fading out over its length
            let t = self.burst_n as f64 / self.burst_len as f64;
            let noise = self.rng.bipolar() as f64;
            self.burst_z = noise * (1.0 - self.burst_c) + self.burst_c * self.burst_z;
            x += self.burst_z * (1.0 - t) * (1.0 - t) * self.vel * 6.0;
            self.burst_n += 1;
        }
        if p.excite == 2 {
            // the noise bow: sustained for as long as the gate is open
            x += self.rng.bipolar() as f64 * 1.7320508 * NOISE_BOW * self.vel * self.gate;
        }
        // four modes abreast with a running sum each (one sum would chain
        // every mode behind the last and keep the loop from vectorising)
        let n = self.n;
        let (mut al, mut ar) = ([0.0f64; 4], [0.0f64; 4]);
        let modes = self.a1[..n]
            .chunks_exact(4)
            .zip(self.a2[..n].chunks_exact(4))
            .zip(self.gin[..n].chunks_exact(4))
            .zip(self.gl[..n].chunks_exact(4))
            .zip(self.gr[..n].chunks_exact(4))
            .zip(self.y1[..n].chunks_exact_mut(4))
            .zip(self.y2[..n].chunks_exact_mut(4));
        for ((((((a1, a2), gin), gl), gr), y1), y2) in modes {
            for j in 0..4 {
                let y = a1[j] * y1[j] + a2[j] * y2[j] + gin[j] * x;
                y2[j] = y1[j];
                y1[j] = y;
                al[j] += y * gl[j];
                ar[j] += y * gr[j];
            }
        }
        for m in n / 4 * 4..n {
            let y = self.a1[m] * self.y1[m] + self.a2[m] * self.y2[m] + self.gin[m] * x;
            self.y2[m] = self.y1[m];
            self.y1[m] = y;
            al[0] += y * self.gl[m];
            ar[0] += y * self.gr[m];
        }
        let l = (al[0] + al[1]) + (al[2] + al[3]);
        let r = (ar[0] + ar[1]) + (ar[2] + ar[3]);
        self.n_sample += 1;
        if self.bend.moving() {
            self.bend.x += self.bend.inc;
            if self.n_sample % 32 == 0 || self.bend.x >= 1.0 {
                self.hz = self.base_hz * self.bend.offset().exp2();
                self.configure(set, p, sr);
            }
        }
        // sleep once the bank has rung down and nothing drives it
        if self.n_sample % 256 == 0 {
            self.energy = self.y1[..n].iter().map(|y| y * y).sum();
            if !self.held
                && self.strike.is_none()
                && self.burst_n >= self.burst_len
                && self.gate == 0.0
                && self.energy < SILENCE * SILENCE
            {
                self.active = false;
            }
        }
        (l, r)
    }
}

/// A polyphonic modal instrument.
pub struct ModalInstrument {
    sr: f64,
    pub p: ModalParams,
    pub modes: ModeSet,
    gate_steps: (f64, f64),
    voices: Vec<ModalVoice>,
    counter: u64,
}

impl ModalInstrument {
    pub fn new(sr: f32, modes: ModeSet) -> Self {
        let sr = sr as f64;
        ModalInstrument {
            sr,
            p: ModalParams::default(),
            modes,
            gate_steps: Gate::default().steps(sr),
            voices: (0..MODAL_VOICES).map(ModalVoice::new).collect(),
            counter: 0,
        }
    }

    pub fn note_on(&mut self, key: u8, hz: f64, vel: f32) {
        self.counter += 1;
        let idx = self
            .voices
            .iter()
            .position(|v| v.active && v.key == key)
            .or_else(|| self.voices.iter().position(|v| !v.active))
            .or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| !v.held)
                    .min_by(|a, b| a.1.energy.total_cmp(&b.1.energy))
                    .map(|(i, _)| i)
            })
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.age)
                    .map(|(i, _)| i)
                    .unwrap()
            });
        let (sr, p) = (self.sr, self.p);
        let v = &mut self.voices[idx];
        if !v.active {
            v.y1.iter_mut().for_each(|y| *y = 0.0);
            v.y2.iter_mut().for_each(|y| *y = 0.0);
            v.gate = 0.0;
        }
        v.key = key;
        v.held = true;
        v.active = true;
        v.age = self.counter;
        v.base_hz = hz;
        v.hz = hz;
        v.bend = Bend::default();
        v.vel = vel.clamp(0.0, 1.0) as f64;
        v.damper = 1.0;
        v.energy = 1.0;
        v.configure(&self.modes, &p, sr);
        v.strike = None;
        v.burst_n = 0;
        v.burst_len = 0;
        match p.excite {
            0 => {
                let hard = (p.hard as f64 * (0.6 + 0.4 * v.vel)).clamp(0.0, 1.0);
                v.strike = Some(strike_shape(sr, hard));
            }
            1 => {
                v.burst_len = ((p.burst as f64) * sr).round().max(1.0) as usize;
                let fc = (500.0 + 15000.0 * p.hard as f64).min(0.45 * sr);
                v.burst_c = (-2.0 * PI * fc / sr).exp();
                v.burst_z = 0.0;
            }
            _ => {}
        }
    }

    pub fn note_off(&mut self, key: u8) {
        let (sr, p) = (self.sr, self.p);
        let mute = p.mute.clamp(0.0, 1.0) as f64;
        for i in 0..self.voices.len() {
            if !(self.voices[i].held && self.voices[i].key == key) {
                continue;
            }
            let v = &mut self.voices[i];
            v.held = false;
            if mute > 0.0 {
                v.damper = 1.0 - 0.99 * mute;
                v.configure(&self.modes, &p, sr);
            }
        }
    }

    pub fn bend(&mut self, key: u8, semitones: f32, seconds: f32, power: f32) {
        let sr = self.sr;
        for v in self.voices.iter_mut().filter(|v| v.held && v.key == key) {
            v.base_hz = v.hz;
            v.bend = Bend {
                oct: semitones as f64 / 12.0,
                x: 0.0,
                inc: 1.0 / (seconds.max(1e-3) as f64 * sr),
                power: power as f64,
            };
        }
    }

    pub fn set_gate(&mut self, gate: Gate) {
        self.gate_steps = gate.steps(self.sr);
    }

    /// A knob moved: every sounding bank takes the new shapes now.
    pub fn changed(&mut self) {
        let (sr, p) = (self.sr, self.p);
        for v in self.voices.iter_mut().filter(|v| v.active) {
            v.configure(&self.modes, &p, sr);
        }
    }

    /// One sample in the reference's own units (before the bus scale).
    pub fn render_raw(&mut self, input: f64) -> (f64, f64) {
        let (mut l, mut r) = (0.0, 0.0);
        let (sr, steps) = (self.sr, self.gate_steps);
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let (vl, vr) = v.render(&self.modes, &self.p, sr, steps, input);
            l += vl;
            r += vr;
        }
        (l, r)
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
}

// ---------------------------------------------------------------------------
// The instrument a track owns
// ---------------------------------------------------------------------------

pub enum Body {
    String(StringInstrument),
    Modal(ModalInstrument),
}

pub struct Instrument {
    pub body: Body,
    /// the channel whose audio excites the body, and how hard
    pub excite: Option<u16>,
    pub excite_gain: f32,
}

impl Instrument {
    pub fn new(spec: &InstrumentSpec, sr: f32) -> Self {
        let body = match &spec.kind {
            InstrumentKind::String => Body::String(StringInstrument::new(sr)),
            InstrumentKind::Modal(set) => Body::Modal(ModalInstrument::new(sr, set.clone())),
        };
        let mut inst = Instrument {
            body,
            excite: spec.excite,
            excite_gain: 1.0,
        };
        // an excited body listens only, unless the track asks for strikes too
        if spec.excite.is_some() {
            match &mut inst.body {
                Body::String(s) => s.p.excite = 3,
                Body::Modal(m) => m.p.excite = 3,
            }
        }
        inst
    }

    pub fn note_on(&mut self, key: u8, cents: f32, vel: f32) {
        let hz = note_hz(key, cents);
        match &mut self.body {
            Body::String(s) => s.note_on(key, hz, vel),
            Body::Modal(m) => m.note_on(key, hz, vel),
        }
    }

    pub fn note_off(&mut self, key: u8) {
        match &mut self.body {
            Body::String(s) => s.note_off(key),
            Body::Modal(m) => m.note_off(key),
        }
    }

    pub fn bend(&mut self, key: u8, semitones: f32, seconds: f32, power: f32) {
        match &mut self.body {
            Body::String(s) => s.bend(key, semitones, seconds, power),
            Body::Modal(m) => m.bend(key, semitones, seconds, power),
        }
    }

    /// The track's attack and release (bow, noise bow, excite gate).
    pub fn set_gate(&mut self, gate: Gate) {
        match &mut self.body {
            Body::String(s) => s.set_gate(gate),
            Body::Modal(m) => m.set_gate(gate),
        }
    }

    /// Set one of this body's knobs. False when the knob belongs to the
    /// other body (or to no instrument at all).
    pub fn set_param(&mut self, param: Param, value: f32) -> bool {
        if value.is_nan() {
            return true;
        }
        let v = param.clamp(value);
        if param == Param::ExciteGain {
            self.excite_gain = v;
            return true;
        }
        match &mut self.body {
            Body::String(s) => {
                let p = &mut s.p;
                match param {
                    Param::StrExcite => p.excite = v.round() as u8,
                    Param::StrDecay => p.decay = v,
                    Param::StrDamp => p.damp = v,
                    Param::StrStiff => p.stiff = v,
                    Param::StrPos => p.pos = v,
                    Param::StrHard => p.hard = v,
                    Param::StrPressure => p.pressure = v,
                    Param::StrMute => p.mute = v,
                    _ => return false,
                }
                if matches!(param, Param::StrDecay | Param::StrDamp) {
                    s.changed();
                }
                true
            }
            Body::Modal(m) => {
                let p = &mut m.p;
                match param {
                    Param::ModalExcite => p.excite = v.round() as u8,
                    Param::ModalDecay => p.decay = v,
                    Param::ModalDamp => p.damp = v,
                    Param::ModalStretch => p.stretch = v,
                    Param::ModalBright => p.bright = v,
                    Param::ModalSpread => p.spread = v,
                    Param::ModalModes => p.modes = v.round() as usize,
                    Param::ModalHard => p.hard = v,
                    Param::ModalBurst => p.burst = v,
                    Param::ModalMute => p.mute = v,
                    _ => return false,
                }
                if !matches!(
                    param,
                    Param::ModalExcite | Param::ModalHard | Param::ModalBurst | Param::ModalMute
                ) {
                    m.changed();
                }
                true
            }
        }
    }

    /// One stereo sample in volts. `input` is the excite track's raw
    /// output (volts, mono).
    pub fn render(&mut self, input: f32) -> (f32, f32) {
        let x = input / PROGRAM_V * self.excite_gain;
        match &mut self.body {
            Body::String(s) => {
                let o = s.render(x * STRING_EXCITE);
                (o, o)
            }
            Body::Modal(m) => {
                let (l, r) = m.render_raw((x * MODAL_EXCITE) as f64);
                (l as f32 * MODAL_V, r as f32 * MODAL_V)
            }
        }
    }

    pub fn is_silent(&self) -> bool {
        match &self.body {
            Body::String(s) => s.active_voices() == 0,
            Body::Modal(m) => m.active_voices() == 0,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    /// The frequency of the partial near `guess` (within 3%) between `t0`
    /// and `t1` seconds: the strongest component on a fine grid, refined
    /// by the phase advance of the heterodyned, Hann-windowed component
    /// (a window of 0.1 s or eight periods) over one window, then t0..t1.
    pub fn measure_hz(x: &[f32], sr: f32, guess: f64, t0: f64, t1: f64) -> f64 {
        // at least eight periods, so the neighbouring partials stay out
        let span = (8.0 / guess).max(0.1);
        let win = ((sr as f64) * span).round() as usize;
        let at = |hz: f64, t: f64| -> (f64, f64) {
            let start = (t * sr as f64) as usize;
            let (mut re, mut im) = (0.0, 0.0);
            for i in 0..win {
                let n = start + i;
                let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / win as f64).cos();
                let ph = -2.0 * PI * hz * n as f64 / sr as f64;
                re += x[n] as f64 * w * ph.cos();
                im += x[n] as f64 * w * ph.sin();
            }
            (im.atan2(re), (re * re + im * im).sqrt())
        };
        let mut best = (guess, 0.0);
        let steps = 120;
        for k in 0..=steps {
            let hz = guess * (0.97 + 0.06 * k as f64 / steps as f64);
            let m = at(hz, t0).1;
            if m > best.1 {
                best = (hz, m);
            }
        }
        let refine = |hz: f64, ta: f64, tb: f64| -> f64 {
            let mut d = at(hz, tb).0 - at(hz, ta).0;
            d -= (d / (2.0 * PI)).round() * 2.0 * PI;
            hz + d / (2.0 * PI * (tb - ta))
        };
        let near = refine(best.0, t0, t0 + span);
        refine(near, t0, t1)
    }

    /// The magnitude of the component at `hz` in a 0.1 s Hann window at `t`.
    pub fn level_at(x: &[f32], sr: f32, hz: f64, t: f64) -> f64 {
        let win = ((sr as f64) * 0.1).round() as usize;
        let start = (t * sr as f64) as usize;
        let (mut re, mut im) = (0.0, 0.0);
        for i in 0..win {
            let n = start + i;
            let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / win as f64).cos();
            let ph = -2.0 * PI * hz * n as f64 / sr as f64;
            re += x[n] as f64 * w * ph.cos();
            im += x[n] as f64 * w * ph.sin();
        }
        (re * re + im * im).sqrt() / win as f64
    }

    fn cents(a: f64, b: f64) -> f64 {
        1200.0 * (a / b).log2()
    }

    fn string_with(p: StringParams) -> StringInstrument {
        let mut s = StringInstrument::new(SR);
        s.p = p;
        s
    }

    fn play_string(s: &mut StringInstrument, hz: f64, seconds: f64, held: f64) -> Vec<f32> {
        s.note_on(60, hz, 1.0);
        let n = (seconds * SR as f64) as usize;
        let off = (held * SR as f64) as usize;
        (0..n)
            .map(|i| {
                if i == off {
                    s.note_off(60);
                }
                s.render(0.0)
            })
            .collect()
    }

    /// The loop's phase delay is solved for the note: a plucked string
    /// sounds its fundamental within a twentieth of a cent, low to high,
    /// stiff or not (B moves the overtones, never the note).
    #[test]
    fn string_pitch_matches_the_loop_length() {
        for &hz in &[41.2, 110.0, 261.63, 440.0, 1046.5, 2093.0] {
            for &stiff in &[0.0, 0.0004] {
                let mut s = string_with(StringParams {
                    decay: 6.0,
                    stiff,
                    ..Default::default()
                });
                let x = play_string(&mut s, hz, 1.2, 1.2);
                let got = measure_hz(&x, SR, hz, 0.4, 0.9);
                assert!(
                    cents(got, hz).abs() < 0.05,
                    "{hz} Hz (B={stiff}) sounded at {got} ({:.3} cents)",
                    cents(got, hz)
                );
            }
        }
    }

    /// The fundamental falls 60 dB in `str_decay` seconds, and with damping
    /// the eighth partial falls as T60 / (1 + damp * 7) says.
    #[test]
    fn string_decay_matches_the_damping() {
        let hz = 110.0;
        for &(t60, damp) in &[(1.5f32, 0.0f32), (3.0, 0.0), (3.0, 1.0)] {
            let mut s = string_with(StringParams {
                decay: t60,
                damp,
                pos: 0.137,
                ..Default::default()
            });
            let x = play_string(&mut s, hz, 2.5, 2.5);
            for (k, want_t60) in [(1.0, t60 as f64), (8.0, t60 as f64 / (1.0 + damp as f64 * 7.0))] {
                // a span the partial falls about 40 dB over, above the floor
                let (a, b) = (0.3, 0.3 + (0.7 * want_t60).min(1.0));
                let la = level_at(&x, SR, hz * k, a);
                let lb = level_at(&x, SR, hz * k, b);
                let db = 20.0 * (lb / la).log10();
                let want = -60.0 * (b - a) / want_t60;
                assert!(
                    (db - want).abs() < 0.04 * want.abs() + 0.3,
                    "t60 {t60} damp {damp} partial {k}: fell {db:.2} dB in {:.2} s, want {want:.2}",
                    b - a
                );
            }
        }
    }

    /// A stiff string's overtones stretch as k f0 sqrt(1 + B k^2): a piano
    /// bass string, a guitar's low string and a mid piano string, through
    /// the sixteenth partial.
    #[test]
    fn stiff_string_overtones_stretch() {
        for &(hz, b) in &[(41.2, 0.0004f64), (110.0, 0.0006), (440.0, 0.0002)] {
            let mut s = string_with(StringParams {
                decay: 8.0,
                damp: 0.2,
                stiff: b as f32,
                pos: 0.11,
                ..Default::default()
            });
            let x = play_string(&mut s, hz, 1.5, 1.5);
            let f0 = hz / (1.0 + b).sqrt();
            for k in [2.0, 3.0, 5.0, 8.0, 12.0, 16.0] {
                let want = f0 * k * (1.0 + b * k * k).sqrt();
                let got = measure_hz(&x, SR, want, 0.3, 0.8);
                assert!(
                    cents(got, want).abs() < 2.0,
                    "{hz} Hz B {b} partial {k}: {got:.3} Hz vs {want:.3} ({:.2} cents; \
                     harmonic would be {:.1} cents off)",
                    cents(got, want),
                    cents(k * hz, want)
                );
            }
        }
    }

    /// Plucked a quarter of the way along, every fourth harmonic has a node
    /// at the plucking point and stays out.
    #[test]
    fn the_pluck_point_notches_its_harmonics() {
        let hz = 110.0;
        let mut s = string_with(StringParams {
            decay: 4.0,
            damp: 0.0,
            pos: 0.25,
            hard: 1.0,
            ..Default::default()
        });
        let x = play_string(&mut s, hz, 1.0, 1.0);
        let l = |k: f64| level_at(&x, SR, hz * k, 0.2);
        for k in [4.0, 8.0] {
            let notch = l(k);
            let around = 0.5 * (l(k - 1.0) + l(k + 1.0));
            assert!(
                20.0 * (around / notch).log10() > 15.0,
                "harmonic {k} should sit in the pluck point's notch: {notch} vs {around}"
            );
        }
    }

    /// A bowed string sustains at its pitch for as long as the bow moves,
    /// then rings down when it lifts.
    #[test]
    fn a_bowed_string_sustains_at_pitch() {
        let hz = 196.0;
        let mut s = string_with(StringParams {
            excite: 2,
            decay: 3.0,
            ..Default::default()
        });
        s.set_gate(Gate {
            attack: 0.05,
            release: 0.05,
        });
        let x = play_string(&mut s, hz, 2.5, 1.6);
        let got = measure_hz(&x, SR, hz, 0.8, 1.3);
        assert!(cents(got, hz).abs() < 5.0, "bowed at {got} Hz");
        let rms = |a: f64, b: f64| {
            let s = &x[(a * SR as f64) as usize..(b * SR as f64) as usize];
            (s.iter().map(|v| (v * v) as f64).sum::<f64>() / s.len() as f64).sqrt()
        };
        let (early, late) = (rms(0.6, 0.9), rms(1.2, 1.5));
        assert!(early > 1e-3, "the bow speaks: {early}");
        assert!(
            (20.0 * (late / early).log10()).abs() < 3.0,
            "sustained under the bow: {early} then {late}"
        );
        assert!(rms(2.2, 2.5) < 0.5 * late, "rings down after the bow lifts");
    }

    /// A bend slides the nut end: the string lands on the target pitch.
    #[test]
    fn a_bent_string_lands_on_its_target() {
        let hz = 110.0;
        let mut s = string_with(StringParams {
            decay: 8.0,
            ..Default::default()
        });
        s.note_on(60, hz, 1.0);
        let mut x = Vec::new();
        for i in 0..(1.6 * SR) as usize {
            if i == (0.1 * SR) as usize {
                s.bend(60, 7.0, 0.3, 1.0);
            }
            x.push(s.render(0.0));
        }
        let want = hz * (7.0f64 / 12.0).exp2();
        let got = measure_hz(&x, SR, want, 0.8, 1.4);
        assert!(cents(got, want).abs() < 0.2, "bent to {got}, want {want}");
    }

    /// A float32 (n, 2) array from one of Hypha's reference renders
    /// (tests/data/spec, written by the spec scripts beside them).
    pub fn read_npy(bytes: &[u8]) -> Vec<[f32; 2]> {
        assert_eq!(&bytes[..6], b"\x93NUMPY");
        let header_len = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
        let header = std::str::from_utf8(&bytes[10..10 + header_len]).unwrap();
        assert!(header.contains("'<f4'") && header.contains("False"), "{header}");
        bytes[10 + header_len..]
            .chunks_exact(8)
            .map(|c| {
                [
                    f32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                    f32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                ]
            })
            .collect()
    }

    /// The modal bank against the reference (specs/modal.py): the first
    /// 4096 samples of every struck case, sample for sample. (wire_bow is
    /// driven by numpy's Gaussian noise, which no port reproduces; it is
    /// the same bank as these, fed differently.)
    #[test]
    fn modal_bank_matches_the_reference_renders() {
        let cases: [(&str, &[u8], &str, f32, f32, f32, f32, f64); 5] = [
            ("bar_strike", include_bytes!("../tests/data/spec/modal_bar_strike.npy"), "bar", 0.5, 0.0, 0.0, 0.0, 220.0),
            ("glass_strike", include_bytes!("../tests/data/spec/modal_glass_strike.npy"), "glass", 0.5, 0.0, 0.0, 0.0, 660.0),
            ("bell_strike", include_bytes!("../tests/data/spec/modal_bell_strike.npy"), "bell", 0.5, 0.0, 0.3, 0.6, 330.0),
            ("plate_strike", include_bytes!("../tests/data/spec/modal_plate_strike.npy"), "plate", 1.0, 0.0, 0.0, 0.0, 180.0),
            ("bar_stretch", include_bytes!("../tests/data/spec/modal_bar_stretch.npy"), "bar", 0.5, 0.15, 0.0, 0.0, 220.0),
        ];
        for (name, bytes, preset, damp, stretch, bright, spread, f0) in cases {
            let want = read_npy(bytes);
            assert_eq!(want.len(), 4096, "{name}");
            let mut m = ModalInstrument::new(48000.0, ModeSet::preset(preset).unwrap());
            m.p.damp = damp;
            m.p.stretch = stretch;
            m.p.bright = bright;
            m.p.spread = spread;
            m.note_on(69, f0, 1.0);
            let peak = want.iter().fold(0f32, |p, s| p.max(s[0].abs()).max(s[1].abs())) as f64;
            let mut worst = 0f64;
            for (i, w) in want.iter().enumerate() {
                let (l, r) = m.render_raw(0.0);
                worst = worst.max((l - w[0] as f64).abs()).max((r - w[1] as f64).abs());
                assert!(worst < 1e-5 * peak, "{name}: sample {i} off by {worst:e} (peak {peak:e})");
            }
        }
    }

    /// The modal bank rings at its preset's ratios, each mode with the
    /// reference's T60.
    #[test]
    fn modal_peaks_sit_at_the_ratios_and_decay_as_specified() {
        let f0 = 220.0;
        let set = ModeSet::preset("bar").unwrap();
        let mut m = ModalInstrument::new(SR, set.clone());
        m.note_on(57, f0, 1.0);
        let x: Vec<f32> = (0..(2.0 * SR) as usize).map(|_| m.render_raw(0.0).0 as f32).collect();
        let p = ModalParams::default();
        for (i, &r) in set.ratios.iter().enumerate().take(5) {
            let hz = f0 * r;
            let got = measure_hz(&x, SR, hz, 0.2, 0.4);
            assert!(cents(got, hz).abs() < 0.1, "mode {i} at {got}, want {hz}");
            let t60 = p.decay as f64 * set.t60[i] / (1.0 + p.damp as f64 * (r - 1.0));
            let (a, b) = (0.2, 0.2 + 0.25 * t60.min(4.0));
            let db = 20.0 * (level_at(&x, SR, hz, b) / level_at(&x, SR, hz, a)).log10();
            let want = -60.0 * (b - a) / t60;
            assert!((db - want).abs() < 0.03 * want.abs() + 0.3, "mode {i}: {db:.2} dB vs {want:.2}");
            // and nothing in between
            let between = f0 * 0.5 * (r + set.ratios.get(i + 1).copied().unwrap_or(r * 1.3));
            assert!(level_at(&x, SR, between, 0.2) < 0.05 * level_at(&x, SR, hz, 0.2));
        }
    }

    /// The excite input: a sine at a mode's frequency rings the bank far
    /// harder than one between modes.
    #[test]
    fn an_excite_input_rings_the_modes_it_matches() {
        let f0 = 220.0;
        let set = ModeSet::preset("glass").unwrap();
        let drive = |hz: f64| {
            let mut m = ModalInstrument::new(SR, set.clone());
            m.p.excite = 3;
            m.note_on(57, f0, 1.0);
            let mut peak = 0.0f64;
            for i in 0..(1.0 * SR) as usize {
                let x = (2.0 * PI * hz * i as f64 / SR as f64).sin() * 0.1;
                let (l, _) = m.render_raw(x);
                peak = peak.max(l.abs());
            }
            peak
        };
        let on = drive(f0 * 2.32);
        let off = drive(f0 * 3.3);
        assert!(on > 20.0 * off, "on-mode {on} vs off-mode {off}");
    }
}
