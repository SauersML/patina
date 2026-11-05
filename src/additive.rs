// The additive voice (`track NAME additive`): one note is up to 512 sine
// partials, each with a ratio, an amplitude and a pan, shaped by nine
// automatable spectral knobs. A port of Hypha's numpy reference
// (tests/data/spec/additive.py), checked against its renders.
//
// Partial k (1..N) of a note at f0, from its table entry (ratio r, amp a,
// pan p; the default table is the harmonic series, r = k, a = 1, p = 0):
//
//   freq = f0 r (1 + stretch (r - 1)) 2^(spread d_k / 1200)
//   amp  = a  r^-tilt  (1 - odd if k is even)
//            (1 - comb_depth (0.5 + 0.5 cos(2 pi r comb_freq + comb_phase)))
//   pan  = p + width (+1 even, -1 odd) (0.5 + 0.5 sin(2.4 k)), in -1..1
//
// d_k is a fixed random detune in -1..1 per partial. Partials past N, at
// or above 0.95 Nyquist, or pushed below 0 Hz by a negative stretch are
// silent, and the amplitudes are scaled so their total power stays the
// same whatever N, tilt and the comb do (sum of amp^2 = 0.3^2).
//
// Every knob is read once a 64-sample block, at the block's start, as the
// target for its end; frequency, amplitude and pan glide linearly across
// the block. A song's lanes arrive as steps (32 a beat), so the knobs the
// blocks read first glide toward the song's values (one pole, ~15 ms):
// fast automation runs without clicks or zipper noise. Each partial keeps
// its own phase through any change (silent ones too, as in the
// reference). Inside a block a partial is a complex rotator whose step
// itself rotates (the linear frequency glide); a sounding partial carries
// its rotator from block to block, held to unit length, and a partial
// coming back in restarts from its exact accumulated phase, so it costs a
// few multiplies a sample and no sine. A block whose knobs and pitch have
// not moved reuses its spectrum.
//
// The note's gate (the track's attack= and release=, linear) and its
// velocity scale the voice per sample.

use crate::oscillator::PROGRAM_V;
use crate::physical::{Bend, Gate};
use crate::rng::Rng;
use crate::song::Param;
use std::f64::consts::PI;

pub const MAX_PARTIALS: usize = 512;
const BLOCK: usize = 64;
/// partials rendered side by side (independent rotator chains)
const LANES: usize = 4;
const VOICES: usize = 8;
/// The reference's total partial amplitude (root of the summed powers).
const LEVEL: f64 = 0.3;
/// Volts on the bus per reference unit, so a held saw-shaped note sits
/// with a synth card's at the same velocity.
const ADD_V: f32 = 1.1 * PROGRAM_V;

/// The additive voice's knobs.
pub fn is_additive_param(p: Param) -> bool {
    matches!(
        p,
        Param::AddPartials
            | Param::AddTilt
            | Param::AddOdd
            | Param::AddStretch
            | Param::AddSpread
            | Param::AddCombDepth
            | Param::AddCombFreq
            | Param::AddCombPhase
            | Param::AddWidth
    )
}

/// A partial table: ratio, amplitude and pan of each partial, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct PartialSet {
    pub ratio: Vec<f64>,
    pub amp: Vec<f64>,
    pub pan: Vec<f64>,
}

impl PartialSet {
    /// The harmonic series: partial k at ratio k, amplitude 1, centred.
    pub fn harmonic() -> Self {
        PartialSet {
            ratio: (1..=MAX_PARTIALS).map(|k| k as f64).collect(),
            amp: vec![1.0; MAX_PARTIALS],
            pan: vec![0.0; MAX_PARTIALS],
        }
    }

    /// One `partials` token: `ratio[:amp[:pan]]` (amp 1 and pan 0 when
    /// left out).
    pub fn parse_partial(tok: &str) -> Result<(f64, f64, f64), String> {
        let mut it = tok.split(':');
        let num = |s: Option<&str>, what: &str| -> Result<Option<f64>, String> {
            s.map(|s| {
                s.parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| format!("partial '{}': invalid {} '{}'", tok, what, s))
            })
            .transpose()
        };
        let r = num(it.next(), "ratio")?.unwrap_or(0.0);
        let a = num(it.next(), "amp")?.unwrap_or(1.0);
        let p = num(it.next(), "pan")?.unwrap_or(0.0);
        if it.next().is_some() {
            return Err(format!("partial '{}': ratio[:amp[:pan]]", tok));
        }
        if !(r > 0.0 && r <= 1024.0) {
            return Err(format!("partial '{}': the ratio must be in (0, 1024]", tok));
        }
        if !(0.0..=16.0).contains(&a) {
            return Err(format!("partial '{}': the amp must be 0..16", tok));
        }
        if !(-1.0..=1.0).contains(&p) {
            return Err(format!("partial '{}': the pan must be -1..1", tok));
        }
        Ok((r, a, p))
    }

    pub fn from_list(list: &[(f64, f64, f64)]) -> Result<Self, String> {
        if list.is_empty() {
            return Err("a partial table needs at least one partial".into());
        }
        if list.len() > MAX_PARTIALS {
            return Err(format!("at most {} partials", MAX_PARTIALS));
        }
        Ok(PartialSet {
            ratio: list.iter().map(|t| t.0).collect(),
            amp: list.iter().map(|t| t.1).collect(),
            pan: list.iter().map(|t| t.2).collect(),
        })
    }

    pub fn len(&self) -> usize {
        self.ratio.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ratio.is_empty()
    }
}

/// The spectral knobs, in song units (see the param table).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdditiveParams {
    pub partials: f64,
    pub tilt: f64,
    pub odd: f64,
    pub stretch: f64,
    /// cents
    pub spread: f64,
    pub comb_depth: f64,
    pub comb_freq: f64,
    /// radians
    pub comb_phase: f64,
    pub width: f64,
}

impl Default for AdditiveParams {
    fn default() -> Self {
        AdditiveParams {
            partials: 64.0,
            tilt: 1.0,
            odd: 0.0,
            stretch: 0.0,
            spread: 0.0,
            comb_depth: 0.0,
            comb_freq: 0.1,
            comb_phase: 0.0,
            width: 0.0,
        }
    }
}

/// What one note's partials do at a block edge: frequency and the
/// left/right amplitudes (amp times the pan law), per partial.
#[derive(Clone)]
struct Edge {
    freq: Vec<f64>,
    left: Vec<f64>,
    right: Vec<f64>,
    /// partials past this one are silent
    live: usize,
}

impl Edge {
    fn new(n: usize) -> Self {
        Edge {
            freq: vec![0.0; n],
            left: vec![0.0; n],
            right: vec![0.0; n],
            live: 0,
        }
    }
}

/// The shared tables every voice of one instrument reads.
struct Shared {
    set: PartialSet,
    /// per partial: ln ratio, the fixed detune, the stereo fan
    ln_ratio: Vec<f64>,
    detune: Vec<f64>,
    fan: Vec<f64>,
}

impl Shared {
    fn new(set: PartialSet) -> Self {
        let mut rng = Rng::new(0x0add_1715);
        let n = set.len();
        Shared {
            ln_ratio: set.ratio.iter().map(|r| r.ln()).collect(),
            detune: (0..n).map(|_| rng.bipolar() as f64).collect(),
            fan: (1..=n)
                .map(|k| {
                    let side = if k % 2 == 0 { 1.0 } else { -1.0 };
                    side * (0.5 + 0.5 * (k as f64 * 2.4).sin())
                })
                .collect(),
            set,
        }
    }

    /// The spectrum at `f0` under knobs `p`, into `e`.
    fn shape(&self, p: &AdditiveParams, f0: f64, sr: f64, e: &mut Edge) {
        let n = self.set.len();
        let count = (p.partials.clamp(1.0, MAX_PARTIALS as f64) as usize).min(n);
        let top = 0.95 * sr / 2.0;
        let mut power = 0.0;
        let mut live = 0;
        for k in 0..n {
            let r = self.set.ratio[k];
            let stretch = 1.0 + p.stretch * (r - 1.0);
            let detune = if p.spread == 0.0 {
                1.0
            } else {
                (p.spread * self.detune[k] / 1200.0).exp2()
            };
            let f = f0 * r * stretch * detune;
            e.freq[k] = f;
            let mut a = 0.0;
            if k < count && f < top && stretch > 0.0 {
                a = self.set.amp[k] * (-p.tilt * self.ln_ratio[k]).exp();
                if k % 2 == 1 {
                    a *= 1.0 - p.odd;
                }
                if p.comb_depth != 0.0 {
                    a *= 1.0
                        - p.comb_depth
                            * (0.5 + 0.5 * (2.0 * PI * r * p.comb_freq + p.comb_phase).cos());
                }
            }
            if a != 0.0 {
                live = k + 1;
            }
            power += a * a;
            e.left[k] = a;
        }
        let norm = LEVEL / (power.sqrt() + 1e-12);
        for k in 0..live {
            let pan = (self.set.pan[k] + p.width * self.fan[k]).clamp(-1.0, 1.0);
            let a = e.left[k] * norm;
            e.left[k] = a * ((1.0 - pan) / 2.0).sqrt();
            e.right[k] = a * ((1.0 + pan) / 2.0).sqrt();
        }
        for k in live..n {
            e.left[k] = 0.0;
            e.right[k] = 0.0;
        }
        e.live = live;
    }
}

struct Voice {
    key: u8,
    held: bool,
    active: bool,
    age: u64,
    vel: f64,
    gate: f64,
    /// the note's pitch (before the bend riding on it)
    base_hz: f64,
    hz: f64,
    bend: Bend,
    /// every partial's phase at the current block's start
    phase: Vec<f64>,
    /// the rotators the last block ended on (z and its step w), valid for
    /// the partials below `ran`: they carry on without a sine
    rot: Vec<[f64; 4]>,
    ran: usize,
    /// the spectrum at the current block's start and at its end, and the
    /// knobs and pitch it was last shaped from
    from: Edge,
    to: Edge,
    shaped: Option<(AdditiveParams, f64)>,
    /// the current block, rendered at its start, and where we are in it
    buf: [(f64, f64); BLOCK],
    pos: usize,
}

impl Voice {
    fn new(n: usize) -> Self {
        Voice {
            key: 0,
            held: false,
            active: false,
            age: 0,
            vel: 0.0,
            gate: 0.0,
            base_hz: 0.0,
            hz: 0.0,
            bend: Bend::default(),
            phase: vec![0.0; n],
            rot: vec![[1.0, 0.0, 1.0, 0.0]; n],
            ran: 0,
            from: Edge::new(n),
            to: Edge::new(n),
            shaped: None,
            buf: [(0.0, 0.0); BLOCK],
            pos: BLOCK,
        }
    }

    /// Render the next block: the knobs now are its end's targets.
    fn block(&mut self, sh: &Shared, p: &AdditiveParams, sr: f64) {
        if self.bend.oct != 0.0 {
            self.bend.x += self.bend.inc * BLOCK as f64;
            self.hz = self.base_hz * self.bend.offset().exp2();
        }
        // nothing moved: the block ends where it starts
        if self.shaped == Some((*p, self.hz)) {
            self.to.clone_from(&self.from);
        } else {
            sh.shape(p, self.hz, sr, &mut self.to);
            self.shaped = Some((*p, self.hz));
        }
        let n = sh.set.len();
        let live = self.from.live.max(self.to.live);
        let w = 2.0 * PI / sr;
        let span = BLOCK as f64;
        let ramp = span * (span - 1.0) / 2.0;
        self.buf = [(0.0, 0.0); BLOCK];
        let mut k0 = 0;
        while k0 < live {
            // LANES partials abreast, their state in registers
            let mut zr = [0.0f64; LANES];
            let mut zi = [0.0f64; LANES];
            let mut wr = [1.0f64; LANES];
            let mut wi = [0.0f64; LANES];
            let mut dr = [1.0f64; LANES];
            let mut di = [0.0f64; LANES];
            let mut cl = [0.0f64; LANES];
            let mut dl = [0.0f64; LANES];
            let mut cr = [0.0f64; LANES];
            let mut dc = [0.0f64; LANES];
            for q in 0..LANES {
                let k = k0 + q;
                if k >= live {
                    break;
                }
                let dt = w * (self.to.freq[k] - self.from.freq[k]) / span;
                if k < self.ran {
                    // where the last block left it, held to unit length
                    let [a, b, c, d] = self.rot[k];
                    let gz = 1.5 - 0.5 * (a * a + b * b);
                    let gw = 1.5 - 0.5 * (c * c + d * d);
                    (zr[q], zi[q], wr[q], wi[q]) = (a * gz, b * gz, c * gw, d * gw);
                } else {
                    (zi[q], zr[q]) = self.phase[k].sin_cos();
                    (wi[q], wr[q]) = (w * self.from.freq[k]).sin_cos();
                }
                if dt != 0.0 {
                    (di[q], dr[q]) = dt.sin_cos();
                }
                cl[q] = self.from.left[k];
                dl[q] = (self.to.left[k] - cl[q]) / span;
                cr[q] = self.from.right[k];
                dc[q] = (self.to.right[k] - cr[q]) / span;
            }
            for out in self.buf.iter_mut() {
                let mut l = 0.0;
                let mut r = 0.0;
                for q in 0..LANES {
                    let nr = zr[q] * wr[q] - zi[q] * wi[q];
                    let ni = zr[q] * wi[q] + zi[q] * wr[q];
                    zr[q] = nr;
                    zi[q] = ni;
                    l += ni * cl[q];
                    r += ni * cr[q];
                    let vr = wr[q] * dr[q] - wi[q] * di[q];
                    let vi = wr[q] * di[q] + wi[q] * dr[q];
                    wr[q] = vr;
                    wi[q] = vi;
                    cl[q] += dl[q];
                    cr[q] += dc[q];
                }
                out.0 += l;
                out.1 += r;
            }
            for q in 0..LANES {
                if k0 + q < live {
                    self.rot[k0 + q] = [zr[q], zi[q], wr[q], wi[q]];
                }
            }
            k0 += LANES;
        }
        self.ran = live;
        // every partial's phase moves on, sounding or not
        for k in 0..n {
            let th = w * self.from.freq[k];
            let dt = w * (self.to.freq[k] - self.from.freq[k]) / span;
            self.phase[k] = (self.phase[k] + span * th + ramp * dt).rem_euclid(2.0 * PI);
        }
        std::mem::swap(&mut self.from, &mut self.to);
        self.pos = 0;
    }
}

pub struct AdditiveInstrument {
    sr: f64,
    /// the knobs the voices read at their blocks' edges, gliding (one
    /// pole, ~15 ms) to `target`, what the song set: a lane arrives as
    /// steps (32 a beat), and this turns them into curves
    pub p: AdditiveParams,
    pub target: AdditiveParams,
    glide: f64,
    shared: Shared,
    gate_steps: (f64, f64),
    voices: Vec<Voice>,
    counter: u64,
}

impl AdditiveInstrument {
    pub fn new(sr: f32, set: PartialSet) -> Self {
        let sr = sr as f64;
        let n = set.len();
        AdditiveInstrument {
            sr,
            p: AdditiveParams::default(),
            target: AdditiveParams::default(),
            glide: 1.0 - (-1.0 / (0.015 * sr)).exp(),
            shared: Shared::new(set),
            gate_steps: steps(&Gate::default(), sr),
            voices: (0..VOICES).map(|_| Voice::new(n)).collect(),
            counter: 0,
        }
    }

    pub fn note_on(&mut self, key: u8, hz: f64, vel: f32) {
        if self.active_voices() == 0 {
            // nothing sounding: no glide to make
            self.p = self.target;
        }
        self.counter += 1;
        // the same key again, a free voice, the quietest released one,
        // else the oldest (a stolen voice keeps its phases and its gate,
        // so it slides into the new note rather than clicking)
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
                    .min_by(|a, b| a.1.gate.total_cmp(&b.1.gate))
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
        let sh = &self.shared;
        let v = &mut self.voices[idx];
        let fresh = !v.active;
        v.key = key;
        v.held = true;
        v.active = true;
        v.age = self.counter;
        v.vel = vel.clamp(0.0, 1.0) as f64;
        v.base_hz = hz;
        v.hz = hz;
        v.bend = Bend::default();
        if fresh {
            // a new note starts its block here, from the spectrum now
            v.gate = 0.0;
            v.phase.iter_mut().for_each(|x| *x = 0.0);
            v.ran = 0;
            sh.shape(&p, hz, sr, &mut v.from);
            v.shaped = Some((p, hz));
            v.pos = BLOCK;
        }
    }

    pub fn note_off(&mut self, key: u8) {
        for v in self.voices.iter_mut().filter(|v| v.held && v.key == key) {
            v.held = false;
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
        self.gate_steps = steps(&gate, self.sr);
    }

    /// Set one knob; false when it is not the additive voice's.
    pub fn set_param(&mut self, param: Param, v: f32) -> bool {
        let v = v as f64;
        let p = &mut self.target;
        match param {
            Param::AddPartials => p.partials = v,
            Param::AddTilt => p.tilt = v,
            Param::AddOdd => p.odd = v,
            Param::AddStretch => p.stretch = v,
            Param::AddSpread => p.spread = v,
            Param::AddCombDepth => p.comb_depth = v,
            Param::AddCombFreq => p.comb_freq = v,
            Param::AddCombPhase => p.comb_phase = v,
            Param::AddWidth => p.width = v,
            _ => return false,
        }
        true
    }

    /// The knobs one sample further toward the song's.
    fn glide(&mut self) {
        let (p, t, k) = (&mut self.p, &self.target, self.glide);
        if p == t {
            return;
        }
        for (c, &t) in [
            (&mut p.partials, &t.partials),
            (&mut p.tilt, &t.tilt),
            (&mut p.odd, &t.odd),
            (&mut p.stretch, &t.stretch),
            (&mut p.spread, &t.spread),
            (&mut p.comb_depth, &t.comb_depth),
            (&mut p.comb_freq, &t.comb_freq),
            (&mut p.comb_phase, &t.comb_phase),
            (&mut p.width, &t.width),
        ] {
            *c += (t - *c) * k;
            if (t - *c).abs() <= 1e-9 * t.abs().max(1.0) {
                *c = t;
            }
        }
    }

    /// One stereo sample in the reference's units (before the bus scale).
    pub fn render_raw(&mut self) -> (f64, f64) {
        self.glide();
        let (up, down) = self.gate_steps;
        let (sr, p) = (self.sr, self.p);
        let (mut l, mut r) = (0.0, 0.0);
        for v in self.voices.iter_mut().filter(|v| v.active) {
            if v.pos == BLOCK {
                v.block(&self.shared, &p, sr);
            }
            let (vl, vr) = v.buf[v.pos];
            v.pos += 1;
            if v.held {
                v.gate = (v.gate + up).min(1.0);
            } else {
                v.gate -= down;
                if v.gate <= 0.0 {
                    v.gate = 0.0;
                    v.active = false;
                    continue;
                }
            }
            let g = v.gate * v.vel;
            l += vl * g;
            r += vr * g;
        }
        (l, r)
    }

    /// One stereo sample in volts.
    pub fn render(&mut self) -> (f32, f32) {
        let (l, r) = self.render_raw();
        (l as f32 * ADD_V, r as f32 * ADD_V)
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
}

/// Per-sample linear gate steps (an attack of 0 opens at once).
fn steps(g: &Gate, sr: f64) -> (f64, f64) {
    let up = if g.attack <= 0.0 {
        1.0
    } else {
        1.0 / (g.attack as f64 * sr)
    };
    (up, 1.0 / (g.release.max(0.005) as f64 * sr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physical::tests::read_npy;

    const SR: f64 = 48000.0;

    /// The reference's lanes for one case: knobs as functions of time.
    type Lanes = fn(f64) -> AdditiveParams;

    fn reference_detune() -> Vec<f64> {
        let bytes = include_bytes!("../tests/data/spec/add_detune.f64");
        bytes
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
            .collect()
    }

    /// Render `secs` of one note at `f0` as the reference does: the knobs
    /// at t = 0 when the note starts, then each block's end before it.
    fn render(f0: f64, secs: f64, lanes: Lanes) -> Vec<(f64, f64)> {
        let mut inst = AdditiveInstrument::new(SR as f32, PartialSet::harmonic());
        inst.shared.detune = reference_detune();
        inst.target = lanes(0.0);
        inst.set_gate(Gate {
            attack: 0.0,
            release: 0.2,
        });
        inst.note_on(60, f0, 1.0);
        let n = (secs * SR) as usize;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            if i % BLOCK == 0 {
                // the reference's lanes are already smooth: no glide
                inst.p = lanes((i + BLOCK) as f64 / SR);
                inst.target = inst.p;
            }
            out.push(inst.render_raw());
        }
        out
    }

    fn worst(ours: &[(f64, f64)], theirs: &[[f32; 2]]) -> f64 {
        assert_eq!(ours.len(), theirs.len());
        ours.iter()
            .zip(theirs)
            .map(|(a, b)| (a.0 - b[0] as f64).abs().max((a.1 - b[1] as f64).abs()))
            .fold(0.0, f64::max)
    }

    fn open_cluster(t: f64) -> AdditiveParams {
        AdditiveParams {
            partials: 1.0 + 255.0 * (t / 4.0).min(1.0),
            tilt: 1.6 - 0.9 * (t / 4.0).min(1.0),
            spread: 40.0 * (t / 5.0).min(1.0),
            width: (t / 3.0).min(1.0),
            ..AdditiveParams::default()
        }
    }

    fn comb_sweep(t: f64) -> AdditiveParams {
        AdditiveParams {
            partials: 256.0,
            tilt: 0.6,
            comb_depth: 0.95,
            comb_freq: 0.06,
            comb_phase: 2.0 * PI * 0.5 * t,
            ..AdditiveParams::default()
        }
    }

    fn odd_morph_stretch(t: f64) -> AdditiveParams {
        AdditiveParams {
            partials: 128.0,
            odd: 0.5 + 0.5 * (PI * t).sin(),
            stretch: 0.004 * t,
            ..AdditiveParams::default()
        }
    }

    /// Every case of the reference, sample for sample: its first 4096
    /// samples (Hypha's own files) and its last 2048 (rendered by the
    /// same script), within float32 rounding of the reference.
    #[test]
    fn additive_matches_the_reference_renders() {
        let cases: [(&str, f64, f64, Lanes, Option<&[u8]>); 4] = [
            (
                "static_saw64",
                110.0,
                2.0,
                |_| AdditiveParams::default(),
                None,
            ),
            (
                "open_cluster",
                110.0,
                6.0,
                open_cluster,
                Some(include_bytes!("../tests/data/spec/add_open_cluster_tail.npy")),
            ),
            (
                "comb_sweep",
                82.5,
                4.0,
                comb_sweep,
                Some(include_bytes!("../tests/data/spec/add_comb_sweep_tail.npy")),
            ),
            (
                "odd_morph_stretch",
                110.0,
                4.0,
                odd_morph_stretch,
                Some(include_bytes!(
                    "../tests/data/spec/add_odd_morph_stretch_tail.npy"
                )),
            ),
        ];
        let heads: [&[u8]; 4] = [
            include_bytes!("../tests/data/spec/add_static_saw64.npy"),
            include_bytes!("../tests/data/spec/add_open_cluster.npy"),
            include_bytes!("../tests/data/spec/add_comb_sweep.npy"),
            include_bytes!("../tests/data/spec/add_odd_morph_stretch.npy"),
        ];
        for ((name, f0, secs, lanes, tail), head) in cases.into_iter().zip(heads) {
            let ours = render(f0, secs, lanes);
            let theirs = read_npy(head);
            let e = worst(&ours[..theirs.len()], &theirs);
            println!("{name}: head within {e:.1e}");
            assert!(e < 2e-6, "{name}: head off by {e:e}");
            if let Some(tail) = tail {
                let theirs = read_npy(tail);
                let e = worst(&ours[ours.len() - theirs.len()..], &theirs);
                println!("{name}: tail within {e:.1e}");
                assert!(e < 2e-6, "{name}: tail off by {e:e}");
            }
        }
    }

    fn spectrum_at(x: &[f64], hz: f64) -> f64 {
        // the amplitude of the component at `hz` (Hann window)
        let n = x.len();
        let (mut re, mut im, mut wsum) = (0.0, 0.0, 0.0);
        for (i, &s) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos();
            let ph = 2.0 * PI * hz * i as f64 / SR;
            re += s * w * ph.cos();
            im += s * w * ph.sin();
            wsum += w;
        }
        2.0 * (re * re + im * im).sqrt() / wsum
    }

    /// A partial table plays its own ratios, amplitudes and pans, under
    /// the shapes: tilt by ratio, and power kept at the reference level.
    #[test]
    fn a_partial_table_sounds_its_ratios_amps_and_pans() {
        let set = PartialSet::from_list(&[(1.0, 1.0, 0.0), (2.76, 0.5, -1.0), (5.4, 0.25, 1.0)])
            .unwrap();
        let mut inst = AdditiveInstrument::new(SR as f32, set);
        inst.target.tilt = 0.0;
        inst.set_gate(Gate {
            attack: 0.0,
            release: 0.2,
        });
        inst.note_on(57, 220.0, 1.0);
        let out: Vec<(f64, f64)> = (0..24000).map(|_| inst.render_raw()).collect();
        let l: Vec<f64> = out[4800..].iter().map(|s| s.0).collect();
        let r: Vec<f64> = out[4800..].iter().map(|s| s.1).collect();
        let norm = LEVEL / (1.0f64 + 0.25 + 0.0625).sqrt();
        let half = (0.5f64).sqrt();
        let want = [
            (220.0, norm * half, norm * half),
            (220.0 * 2.76, 0.0, 0.0),
            (220.0 * 5.4, 0.0, 0.0),
        ];
        // centred fundamental, partial 2 hard left, partial 3 hard right
        assert!((spectrum_at(&l, want[0].0) - want[0].1).abs() < 1e-4);
        assert!((spectrum_at(&r, want[0].0) - want[0].2).abs() < 1e-4);
        assert!((spectrum_at(&l, 220.0 * 2.76) - norm * 0.5).abs() < 1e-4);
        assert!(spectrum_at(&r, 220.0 * 2.76) < 1e-5);
        assert!((spectrum_at(&r, 220.0 * 5.4) - norm * 0.25).abs() < 1e-4);
        assert!(spectrum_at(&l, 220.0 * 5.4) < 1e-5);
        // a stretched table moves partial k to r (1 + s (r - 1))
        inst.target.stretch = 0.01;
        let out: Vec<f64> = (0..24000).map(|_| inst.render_raw().0).collect();
        let f = 220.0 * 2.76 * (1.0 + 0.01 * 1.76);
        assert!(spectrum_at(&out[4800..], f) > 0.9 * norm * 0.5);
    }

    /// odd = 1 leaves only the odd harmonics; tilt 2 falls 12 dB an
    /// octave; the comb notches the partials its cosine peaks on.
    #[test]
    fn the_spectral_shapes_do_what_they_say() {
        let note = |p: AdditiveParams| {
            let mut inst = AdditiveInstrument::new(SR as f32, PartialSet::harmonic());
            inst.target = p;
            inst.set_gate(Gate {
                attack: 0.0,
                release: 0.2,
            });
            inst.note_on(45, 110.0, 1.0);
            let x: Vec<f64> = (0..28800).map(|_| inst.render_raw().0).collect();
            x[4800..].to_vec()
        };
        let x = note(AdditiveParams {
            odd: 1.0,
            ..Default::default()
        });
        let a1 = spectrum_at(&x, 110.0);
        assert!(spectrum_at(&x, 220.0) < 1e-6 * a1);
        assert!((spectrum_at(&x, 330.0) / a1 - 1.0 / 3.0).abs() < 1e-3);
        let x = note(AdditiveParams {
            tilt: 2.0,
            ..Default::default()
        });
        let db = 20.0 * (spectrum_at(&x, 440.0) / spectrum_at(&x, 220.0)).log10();
        assert!((db + 12.04).abs() < 0.05, "{db}");
        // comb_freq 0.25 with phase pi at full depth: the cosine peaks on
        // partials 2, 6, 10 (notched), troughs on 4, 8, 12 (untouched),
        // and the odd ones sit halfway
        let x = note(AdditiveParams {
            tilt: 0.0,
            comb_depth: 1.0,
            comb_freq: 0.25,
            comb_phase: PI,
            ..Default::default()
        });
        let a4 = spectrum_at(&x, 440.0);
        assert!(a4 > 0.01, "{a4}");
        for k in [2.0, 6.0, 10.0] {
            assert!(spectrum_at(&x, 110.0 * k) < 1e-5 * a4, "partial {k}");
        }
        assert!((spectrum_at(&x, 880.0) / a4 - 1.0).abs() < 1e-3);
        assert!((spectrum_at(&x, 330.0) / a4 - 0.5).abs() < 1e-3);
    }

    /// A bent note glides to its target and holds there; a tuned note
    /// (cents from the key) plays its exact pitch.
    #[test]
    fn notes_bend_and_tune() {
        use crate::physical::tests::measure_hz;
        let mut inst = crate::physical::Instrument::new(
            &crate::physical::InstrumentSpec {
                kind: crate::physical::InstrumentKind::Additive(PartialSet::harmonic()),
                excite: None,
            },
            SR as f32,
        );
        inst.note_on(57, 13.7, 1.0);
        inst.bend(57, 7.0, 0.25, 1.0);
        let x: Vec<f32> = (0..48000).map(|_| inst.render(0.0).0).collect();
        let want = 220.0 * ((13.7 + 700.0) / 1200.0f64).exp2();
        let got = measure_hz(&x, SR as f32, want, 0.4, 0.9);
        assert!((1200.0 * (got / want).log2()).abs() < 0.01, "{got} vs {want}");
    }

    /// A knob jump glides over one block: the output never steps.
    #[test]
    fn knob_jumps_glide_without_clicks() {
        let mut inst = AdditiveInstrument::new(SR as f32, PartialSet::harmonic());
        inst.set_gate(Gate {
            attack: 0.0,
            release: 0.2,
        });
        inst.note_on(45, 110.0, 1.0);
        // the largest sample-to-sample step before, across and after
        let (mut before, mut across, mut after) = (0.0f64, 0.0f64, 0.0f64);
        let mut prev = (0.0, 0.0);
        for i in 0..72000 {
            if i == 24000 + 17 {
                // mid-block: taken at the next block's start
                inst.target.partials = 256.0;
                inst.target.tilt = 0.3;
                inst.target.width = 1.0;
                inst.target.comb_depth = 1.0;
                inst.target.comb_phase = 2.0;
            }
            let s = inst.render_raw();
            let d = (s.0 - prev.0).abs().max((s.1 - prev.1).abs());
            match i {
                4800..=23999 => before = before.max(d),
                24000..=24300 => across = across.max(d),
                48000.. => after = after.max(d),
                _ => {}
            }
            prev = s;
        }
        // the glide never steps further than either spectrum moves alone
        assert!(across <= 1.05 * before.max(after), "{before} {across} {after}");
    }
}
