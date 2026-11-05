// src/granular.rs
//
// The granular engine: an insert on a track's strip (`automate pad.gran`)
// or across the mix bus (`automate gran`). It records what it hears into a
// live capture buffer and plays it back as a cloud of short windowed grains:
//
//   in ─► CAPTURE BUFFER (12 s, stops writing while frozen) ─┐
//    │                                                       ▼
//    │          grains: Hann-windowed reads at a delay, a speed and a pan,
//    │          one every 1/density seconds (more or less, see jitter)
//    ▼                                                       │
//   dry ×(1 − gran) ─────────────────────── + ◄── × gran ────┘
//
// Every grain draws its own settings when it starts:
//   - where: `gran_pos` seconds behind the live input, plus up to
//     ±`gran_spray` seconds at random (in a frozen buffer, `gran_pos`
//     scrubs through what was caught)
//   - speed: `gran_pitch` semitones, ±`gran_pitch_scatter` at random. A
//     grain reads its stretch faster or slower, as tape would, but every
//     grain lasts `gran_size` and new ones keep starting from the live
//     point, so the cloud's pitch moves while it keeps the music's tempo
//   - direction: backwards with probability `gran_reverse`
//   - place: a balance pan within ±`gran_pan_scatter`
//   - when: one grain every 1/`gran_density` seconds, the gaps drawn
//     between that clock (`gran_jitter` 0) and a Poisson stream (1)
//
// Level: N = density × size grains overlap at once. With N = 2 and no
// scatter, the windows sum to exactly one and the cloud IS the input,
// delayed by `gran_pos` (the test pins this to the sample). Denser clouds
// scale by sqrt(2/N), the power sum of uncorrelated grains, so a thick
// cloud sits near the input's own loudness instead of N/2 times over it.
//
// A grain never reads audio that is not there yet: a pitched-up grain
// reads faster than the buffer fills, so its start is held far enough back
// that it cannot overtake the write head, and a reversed grain starts from
// the end of a stretch already recorded. Reads are 4-point Hermite.
//
// Antialiased: a grain read faster than the recording would fold
// everything above sr / (2 × speed) back down, so the buffer keeps four
// more copies of itself, each half the rate of the one before (a 63-tap
// Kaiser half-band, flat to 0.21 of its input rate and 80 dB down from
// 0.29), and a grain at speed s reads the copy 2^ceil(log2 s) slower, at
// a speed of one or less. An octave-up cloud of a 15 kHz tone is silent
// where an unfiltered read would put a full-level 18 kHz alias.
//
// Freezing and thawing never click. A grain that would run into the
// stopped head (freezing) or into audio the restarted head overwrites
// (thawing) fades out over 5 ms, its read held inside the recording; the
// rest play on. At a thaw the recording's seam is faded too: the last
// 5 ms before the freeze fade out and the first 5 ms after it fade in, so
// a grain that reads across the seam dips rather than jumps.
//
// A pitch set (`gran_pitches=0,7,12`) gives each grain one of its
// intervals at random, on top of `gran_pitch` and its scatter: a cloud
// that sounds a chord of the input rather than a smear around it.
//
// The insert exists only for a song that uses it (built at the downbeat so
// the buffer is already full of the track when grains first start), and
// at `gran` 0 it passes its input through untouched.
//
// The same engine also runs as a send (`gran_send=` on the tracks that
// feed it, a `grains` line for its knobs): it hears the sum of the sends
// and returns only its cloud, `gran` its return level (default 1), into
// the mix bus ahead of the spring, echo and reverb.

use crate::rng;

/// The capture buffer, seconds.
const BUF_S: f32 = 12.0;
/// Grains sounding at once, at most (a grain that would be one more is
/// skipped).
const MAX_GRAINS: usize = 256;
/// The wet/dry blend's smoothing time constant, seconds.
const WET_TAU_S: f32 = 0.01;
/// A grain's fade-out, and the recording seam's fades, at a freeze or a
/// thaw, seconds.
const FADE_S: f32 = 0.005;
/// The send engine's channel (a `grains` line's knobs, `automate
/// grains.gran_density`): one past the sampler slots, below the voice box.
pub const GRAN_SEND_CHANNEL: u16 = u16::MAX - 2;
/// Intervals a pitch set holds, at most.
pub const MAX_PITCHES: usize = 16;
/// Copies of the recording, each at half the rate of the one before (0 is
/// the recording itself): grains up to 16 times faster read alias-free.
const LEVELS: usize = 5;
/// The half-band filter's half length: taps at -HB..=HB.
const HB: usize = 31;
/// Each copy's delay behind the recording, in recording samples: the
/// half-bands' group delays, HB (2^k - 1).
fn level_delay(k: usize) -> f64 {
    (HB as f64) * ((1u64 << k) - 1) as f64
}

/// The half-band's taps at offsets -HB..=HB: a Kaiser-windowed (beta 8)
/// sinc at a quarter of the rate, unity at DC.
fn halfband() -> Vec<f32> {
    let i0 = |x: f64| {
        let (mut sum, mut term) = (1.0f64, 1.0f64);
        for k in 1..40 {
            term *= (x / (2.0 * k as f64)).powi(2);
            sum += term;
        }
        sum
    };
    let beta = 8.0;
    let mut h: Vec<f64> = (0..=2 * HB)
        .map(|i| {
            let n = i as f64 - HB as f64;
            let sinc = if n == 0.0 {
                0.5
            } else {
                (std::f64::consts::PI * n / 2.0).sin() / (std::f64::consts::PI * n)
            };
            let r = n / HB as f64;
            sinc * i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0(beta)
        })
        .collect();
    let sum: f64 = h.iter().sum();
    h.iter_mut().for_each(|v| *v /= sum);
    h.into_iter().map(|v| v as f32).collect()
}

/// One sounding grain.
#[derive(Clone, Copy)]
struct Grain {
    /// Read position, in absolute buffer samples (the write head's units).
    pos: f64,
    /// Samples of source per output sample: + forward, − backward.
    step: f64,
    age: u32,
    len: u32,
    /// The window's phasor, e^(i 2pi age/len), and its per-sample turn
    /// (a rotation instead of a cosine per grain per sample).
    wc: f64,
    ws: f64,
    rc: f64,
    rs: f64,
    /// Pan and level gains, left and right.
    gl: f32,
    gr: f32,
    /// A fading grain's level (1 → 0) and its step per sample (0: not
    /// fading).
    fade: f32,
    fade_step: f32,
    /// The copy of the recording it reads (see LEVELS).
    level: u8,
}

#[derive(Clone)]
pub struct Granular {
    sr: f32,
    buf: Vec<[f32; 2]>,
    /// Samples written so far (the next write lands at head % len).
    head: u64,
    /// The slower copies, 1..LEVELS: copy k's sample j is the half-band of
    /// copy k-1 around its sample 2j - HB, so it stands for recording time
    /// 2^k j - level_delay(k). Each ring holds a little over BUF_S.
    levels: Vec<Vec<[f32; 2]>>,
    hb: Vec<f32>,
    frozen: bool,
    /// Samples still to fade in after a thaw.
    seam_in: u32,
    grains: Vec<Grain>,
    rng: u32,
    /// Samples until the next grain starts.
    until_next: f64,
    wet: f32,
    wet_k: f32,
    // The knobs, in their song-file units
    mix: f32,
    size_ms: f32,
    density: f32,
    pos_s: f32,
    spray_s: f32,
    pitch: f32,
    pitch_scatter: f32,
    pan_scatter: f32,
    reverse: f32,
    jitter: f32,
    /// The pitch set (`gran_pitches=`), semitones: each grain draws one.
    /// Empty (n_pitches 0) draws nothing.
    pitches: [f32; MAX_PITCHES],
    n_pitches: usize,
}

/// The engine's knobs, by song-file name (see `Granular::set`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Knob {
    Mix,
    Size,
    Density,
    Pos,
    Spray,
    Pitch,
    PitchScatter,
    PanScatter,
    Reverse,
    Freeze,
    Jitter,
}

/// 4-point, 3rd-order Hermite (Catmull-Rom) between y1 and y2 at t in [0,1).
/// At t = 0 it is exactly y1, so integer-position reads are exact copies.
#[inline]
fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + y1
}

impl Granular {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate,
            buf: vec![[0.0; 2]; (BUF_S * sample_rate) as usize],
            head: 0,
            levels: (1..LEVELS)
                .map(|k| vec![[0.0; 2]; ((BUF_S * sample_rate) as usize >> k) + 64])
                .collect(),
            hb: halfband(),
            frozen: false,
            seam_in: 0,
            grains: Vec::with_capacity(MAX_GRAINS),
            rng: rng::seed(0x6a7a),
            until_next: 0.0,
            wet: 0.0,
            wet_k: crate::smoothing::approach(WET_TAU_S, sample_rate),
            mix: 0.0,
            size_ms: 80.0,
            density: 20.0,
            pos_s: 0.0,
            spray_s: 0.0,
            pitch: 0.0,
            pitch_scatter: 0.0,
            pan_scatter: 0.0,
            reverse: 0.0,
            jitter: 0.0,
            pitches: [0.0; MAX_PITCHES],
            n_pitches: 0,
        }
    }

    /// The send form: an engine that hears a send bus and returns only its
    /// cloud (`process_send`), open from the first sample at full return.
    pub fn new_send(sample_rate: f32) -> Self {
        let mut g = Self::new(sample_rate);
        g.mix = 1.0;
        g.wet = 1.0;
        g
    }

    /// The pitch set each grain draws an interval from (semitones; at most
    /// MAX_PITCHES, the rest ignored). Empty: no draw, every grain at
    /// `gran_pitch`.
    pub fn set_pitches(&mut self, set: &[f32]) {
        let n = set.len().min(MAX_PITCHES);
        self.pitches[..n].copy_from_slice(&set[..n]);
        self.n_pitches = n;
    }

    /// Turn one knob (values already clamped to the parameter's range).
    pub fn set(&mut self, knob: Knob, v: f32) {
        match knob {
            Knob::Mix => self.mix = v.clamp(0.0, 1.0),
            Knob::Size => self.size_ms = v.max(1.0),
            Knob::Density => self.density = v.max(0.01),
            Knob::Pos => self.pos_s = v.max(0.0),
            Knob::Spray => self.spray_s = v.max(0.0),
            Knob::Pitch => self.pitch = v,
            Knob::PitchScatter => self.pitch_scatter = v.max(0.0),
            Knob::PanScatter => self.pan_scatter = v.clamp(0.0, 1.0),
            Knob::Reverse => self.reverse = v.clamp(0.0, 1.0),
            Knob::Freeze => {
                let frozen = v >= 0.5;
                if frozen != self.frozen {
                    self.switch_freeze(frozen);
                }
            }
            Knob::Jitter => self.jitter = v.clamp(0.0, 1.0),
        }
    }

    /// Freeze or thaw the recording. Grains that the switch would carry
    /// past the recording fade out; a thaw also fades the seam.
    fn switch_freeze(&mut self, frozen: bool) {
        let fade = (FADE_S * self.sr).max(1.0);
        for k in 0..self.grains.len() {
            let g = self.grains[k];
            if g.fade_step != 0.0 {
                continue;
            }
            let rest = (g.len - g.age) as f64;
            let end = g.pos + g.step * rest;
            let (oldest, newest) = self.readable(g.level as usize);
            let at_risk = if frozen {
                // the head stops: does the read reach it (taps included)?
                g.pos.max(end) > newest
            } else {
                // the head moves on, overwriting the oldest audio: does
                // the read fall behind what is left?
                g.pos < oldest || end < oldest + rest
            };
            if at_risk {
                self.grains[k].fade_step = -1.0 / fade as f32;
            }
        }
        if !frozen {
            // the seam: the last moments before the freeze fade out here,
            // the first after it fade in as they are written
            let n = fade as u64;
            for j in 0..n.min(self.head) {
                let k = self.head - 1 - j;
                let w = 0.5 - 0.5 * (std::f32::consts::PI * j as f32 / n as f32).cos();
                let i = (k % self.buf.len() as u64) as usize;
                self.buf[i] = [self.buf[i][0] * w, self.buf[i][1] * w];
            }
            // ...and the slower copies are filtered again over it
            let mut from = self.head.saturating_sub(n);
            for k in 1..LEVELS {
                let written = self.head >> k;
                from = from.saturating_sub(1) / 2;
                for j in from..written {
                    self.decimate(k, j);
                }
            }
            self.seam_in = n as u32;
        }
        self.frozen = frozen;
    }

    /// The readable span of copy `level`, in recording samples: the
    /// oldest and newest positions whose interpolation taps are all
    /// recorded.
    fn readable(&self, level: usize) -> (f64, f64) {
        let head = self.head as f64;
        if level == 0 {
            return (head - self.buf.len() as f64 + 2.0, head - 3.0);
        }
        let scale = (1u64 << level) as f64;
        let written = (self.head >> level) as f64;
        let cap = self.levels[level - 1].len() as f64;
        let at = |j: f64| scale * j - level_delay(level);
        (at(written - cap + 2.0), at(written - 3.0))
    }

    /// Copy `k`'s sample `j`: the half-band of copy k-1 around 2j - HB.
    fn decimate(&mut self, k: usize, j: u64) {
        let (lower, upper) = self.levels.split_at_mut(k - 1);
        let src: &[[f32; 2]] = if k == 1 { &self.buf } else { &lower[k - 2] };
        let dst = &mut upper[0];
        let n = src.len() as i64;
        let newest = 2 * j as i64;
        let (mut l, mut r) = (0.0f32, 0.0f32);
        // the half-band's even taps are zero but the centre
        for (i, &h) in self.hb.iter().enumerate() {
            if i != HB && (i + HB) % 2 == 0 {
                continue;
            }
            let x = src[(newest - i as i64).rem_euclid(n) as usize];
            l += h * x[0];
            r += h * x[1];
        }
        let m = dst.len() as u64;
        dst[(j % m) as usize] = [l, r];
    }

    /// The gap to the next grain, samples.
    fn interval(&mut self) -> f64 {
        let base = self.sr as f64 / self.density as f64;
        if self.jitter <= 0.0 {
            return base;
        }
        // between the clock and an exponential (Poisson) gap, same mean
        let u = (1.0 - rng::unipolar(&mut self.rng) as f64).max(1e-12);
        let j = self.jitter as f64;
        base * ((1.0 - j) + j * -u.ln())
    }

    /// Start one grain, if the pool has room and the buffer holds enough.
    fn spawn(&mut self) {
        if self.grains.len() >= MAX_GRAINS {
            return;
        }
        let len = ((self.size_ms * 0.001 * self.sr).round() as u32).max(8);
        // one interval of the pitch set, if there is one
        let chosen = if self.n_pitches > 0 {
            let i = (rng::unipolar(&mut self.rng) * self.n_pitches as f32) as usize;
            self.pitches[i.min(self.n_pitches - 1)]
        } else {
            0.0
        };
        let semis = self.pitch + chosen + self.pitch_scatter * rng::bipolar(&mut self.rng);
        let rate = (semis as f64 / 12.0).exp2();
        let backward = self.reverse > 0.0 && rng::unipolar(&mut self.rng) < self.reverse;
        let pan = self.pan_scatter * rng::bipolar(&mut self.rng);
        let spray = self.spray_s * rng::bipolar(&mut self.rng);

        // How far behind the write head the grain's stretch starts, and
        // the range a read of `len` samples at `rate` may start in
        let cap = self.buf.len() as f64;
        let span = rate * (len - 1) as f64;
        let exact = rate == 1.0 && !backward;
        // faster than the recording: read a slower copy, at one or less
        let copy = if rate > 1.0 {
            ((rate.log2() - 1e-9).ceil() as usize).min(LEVELS - 1)
        } else {
            0
        };
        // a slower copy lags the recording by its filters' delay, and its
        // taps reach a few of its own (longer) samples ahead
        let lag = if copy == 0 { 0.0 } else { level_delay(copy) + 4.0 * (1u64 << copy) as f64 };
        let (lo, hi) = if self.frozen {
            // nothing new arrives: the stretch and its interpolation taps
            // lie inside the ring (never-written slots are silence)
            (span + 3.0 + lag, cap - 4.0 - lag)
        } else {
            let lo = if backward {
                // it starts reading at the stretch's end, already recorded
                span + 3.0
            } else if exact {
                // integer reads at the head's own speed: exact copies, no
                // interpolation taps ahead of the read
                0.0
            } else {
                // a faster read must not overtake the head
                ((rate - 1.0) * (len - 1) as f64).max(0.0) + 3.0
            };
            // the head overwrites the oldest audio while the grain plays;
            // before the track has played that far back, it reads silence
            (lo + lag, cap - len as f64 - 6.0)
        };
        if hi < lo {
            return;
        }
        // whole samples, so a grain at the head's own speed copies exactly
        let d = ((self.pos_s + spray) as f64 * self.sr as f64).round().clamp(lo.ceil(), hi);
        let start = self.head as f64 - 1.0 - d;
        let (pos, step) = if backward {
            (start + span, -rate)
        } else {
            (start, rate)
        };
        // Level: unity for sparse grains, the uncorrelated power sum when
        // more than two overlap (see the header)
        let overlap = self.density * self.size_ms * 0.001;
        let level = (2.0 / overlap.max(2.0)).sqrt();
        let theta = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
        let s2 = std::f32::consts::SQRT_2;
        let turn = std::f64::consts::TAU / len as f64;
        self.grains.push(Grain {
            pos,
            step,
            age: 0,
            len,
            wc: 1.0,
            ws: 0.0,
            rc: turn.cos(),
            rs: turn.sin(),
            gl: theta.cos() * s2 * level,
            gr: theta.sin() * s2 * level,
            fade: 1.0,
            fade_step: 0.0,
            level: copy as u8,
        });
    }

    /// Copy `level` at recording position `pos`.
    #[inline]
    fn read(&self, level: usize, pos: f64) -> [f32; 2] {
        let (buf, pos) = if level == 0 {
            (&self.buf, pos)
        } else {
            let scale = (1u64 << level) as f64;
            (&self.levels[level - 1], (pos + level_delay(level)) / scale)
        };
        let cap = buf.len() as u64;
        let i = pos.floor();
        let t = (pos - i) as f32;
        let i = i as i64;
        let at = |k: i64| buf[(k.rem_euclid(cap as i64)) as usize];
        let (a, b, c, d) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        if t == 0.0 {
            return b;
        }
        [
            hermite(a[0], b[0], c[0], d[0], t),
            hermite(a[1], b[1], c[1], d[1], t),
        ]
    }

    /// One sample on a strip or the bus: the input blended with its cloud.
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.record(l, r);
        self.wet += (self.mix - self.wet) * self.wet_k;
        if self.mix == 0.0 && self.wet < 1e-6 {
            // out: recording only, so the cloud starts from a full buffer
            self.rest();
            return (l, r);
        }
        let (wl, wr) = self.cloud();
        let wet = self.wet;
        (l * (1.0 - wet) + wl * wet, r * (1.0 - wet) + wr * wet)
    }

    /// One sample of the send form: the send bus in, only the cloud out,
    /// at the return level `gran` sets.
    pub fn process_send(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.record(l, r);
        self.wet += (self.mix - self.wet) * self.wet_k;
        if self.mix == 0.0 && self.wet < 1e-6 {
            self.rest();
            return (0.0, 0.0);
        }
        let (wl, wr) = self.cloud();
        (wl * self.wet, wr * self.wet)
    }

    /// Closed: no grains sound, and the next opening starts the clock.
    fn rest(&mut self) {
        self.wet = 0.0;
        self.grains.clear();
        self.until_next = 0.0;
    }

    /// Write one sample into the capture buffer and its slower copies
    /// (nothing while frozen).
    fn record(&mut self, l: f32, r: f32) {
        if !self.frozen {
            let cap = self.buf.len() as u64;
            let (mut wl, mut wr) = (l, r);
            if self.seam_in > 0 {
                let n = (FADE_S * self.sr).max(1.0) as u32;
                let j = n - self.seam_in;
                let w = 0.5 - 0.5 * (std::f32::consts::PI * j as f32 / n as f32).cos();
                (wl, wr) = (l * w, r * w);
                self.seam_in -= 1;
            }
            self.buf[(self.head % cap) as usize] = [wl, wr];
            self.head += 1;
            // each slower copy gains a sample every second one below it
            let mut n = self.head;
            for k in 1..LEVELS {
                if n % 2 != 0 {
                    break;
                }
                n /= 2;
                self.decimate(k, n - 1);
            }
        }
    }

    /// Start the grains due this sample and sum every sounding one.
    fn cloud(&mut self) -> (f32, f32) {
        self.until_next -= 1.0;
        while self.until_next <= 0.0 {
            self.spawn();
            self.until_next += self.interval();
        }
        let (mut wl, mut wr) = (0.0f32, 0.0f32);
        let mut k = 0;
        while k < self.grains.len() {
            let g = self.grains[k];
            // periodic Hann: shifted copies at half its length sum to one
            let mut w = (0.5 - 0.5 * g.wc) as f32;
            let s = if g.fade_step == 0.0 {
                self.read(g.level as usize, g.pos)
            } else {
                // fading out after a freeze or a thaw: held inside what
                // is recorded, taps included
                w *= g.fade;
                let (oldest, newest) = self.readable(g.level as usize);
                if g.level == 0 {
                    let head = self.head as f64;
                    let p = g.pos.clamp(oldest, (head - 1.0).max(oldest));
                    // the last samples have no taps ahead: read them whole
                    self.read(0, if p > newest { p.floor() } else { p })
                } else {
                    self.read(g.level as usize, g.pos.clamp(oldest, newest.max(oldest)))
                }
            };
            wl += s[0] * w * g.gl;
            wr += s[1] * w * g.gr;
            let g = &mut self.grains[k];
            g.pos += g.step;
            g.age += 1;
            g.fade += g.fade_step;
            (g.wc, g.ws) = (g.wc * g.rc - g.ws * g.rs, g.ws * g.rc + g.wc * g.rs);
            if g.age >= g.len || g.fade <= 0.0 {
                self.grains.swap_remove(k);
            } else {
                k += 1;
            }
        }
        (wl, wr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    fn engine(knobs: &[(Knob, f32)]) -> Granular {
        let mut g = Granular::new(SR);
        for &(k, v) in knobs {
            g.set(k, v);
        }
        g
    }

    /// The amplitude of a sinusoid near `f` (within ~50 Hz): Hann-windowed
    /// Goertzel over 1024-sample blocks, averaged. A grain cloud spreads a
    /// tone over sidebands at the grain rate, which one long exact-bin
    /// Goertzel would miss.
    fn band(x: &[f32], f: f32) -> f32 {
        let n = 1024;
        let mut sum = 0.0f64;
        let blocks = x.len() / n;
        for b in 0..blocks {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for i in 0..n {
                let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
                let ph = std::f64::consts::TAU * f as f64 * i as f64 / SR as f64;
                let v = x[b * n + i] as f64 * w;
                re += v * ph.cos();
                im += v * ph.sin();
            }
            // a unit sine reads 1
            sum += (re * re + im * im).sqrt() / (0.25 * n as f64);
        }
        (sum / blocks as f64) as f32
    }

    /// Two grains overlapping, no scatter, no pitch: the Hann windows sum
    /// to one, so the cloud is the input delayed by gran_pos, to the sample.
    #[test]
    fn a_two_overlap_cloud_is_the_input_delayed() {
        // 50 ms grains (2400 samples) every 25 ms, 0.1 s behind (4800)
        let mut g = engine(&[
            (Knob::Mix, 1.0),
            (Knob::Size, 50.0),
            (Knob::Density, 40.0),
            (Knob::Pos, 0.1),
        ]);
        g.wet = 1.0;
        let x: Vec<f32> = (0..48000)
            .map(|n| {
                let t = n as f32 / SR;
                (std::f32::consts::TAU * 220.0 * t).sin() * 0.5
                    + (std::f32::consts::TAU * 1370.0 * t).sin() * 0.2 * (3.0 * t).cos()
            })
            .collect();
        let y: Vec<f32> = x.iter().map(|&v| g.process(v, -v).0).collect();
        let mut worst = 0.0f32;
        for n in 4800 + 2400..48000 {
            worst = worst.max((y[n] - x[n - 4800]).abs());
        }
        assert!(worst < 1e-5, "the cloud strays {worst} from the input 4800 samples back");
    }

    /// gran_pitch +12 reads at double speed: a 440 Hz input comes out at
    /// 880 Hz, and nothing is left at 440.
    #[test]
    fn a_grain_an_octave_up_reads_at_double_speed() {
        let mut g = engine(&[
            (Knob::Mix, 1.0),
            // a grain every 1200 samples, 11 whole cycles of 440 Hz, so
            // the grains land in phase and the octave is one clean line
            (Knob::Size, 60.0),
            (Knob::Density, 40.0),
            (Knob::Pitch, 12.0),
            (Knob::Pos, 0.2),
        ]);
        let y: Vec<f32> = (0..96000)
            .map(|n| {
                let v = (std::f32::consts::TAU * 440.0 * n as f32 / SR).sin();
                g.process(v, v).0
            })
            .collect();
        let tail = &y[48000..];
        let (up, base) = (band(tail, 880.0), band(tail, 440.0));
        assert!(up > 30.0 * base, "880 Hz {up:.4} vs 440 Hz {base:.4}");
        assert!(up > 0.5, "the octave-up cloud is too quiet: {up:.4}");
    }

    /// Pitched-up grains do not alias: an octave-up cloud turns a 6 kHz
    /// tone into 12 kHz at full level, and a 15 kHz tone (30 kHz, past
    /// Nyquist) into nothing, where an unfiltered read would fold it to
    /// 18 kHz; two octaves up the same from a 3 kHz and a 9 kHz tone.
    #[test]
    fn pitched_grains_do_not_alias() {
        let cloud = |f: f32, semis: f32| -> Vec<f32> {
            let mut g = engine(&[
                (Knob::Mix, 1.0),
                (Knob::Size, 60.0),
                (Knob::Density, 40.0),
                (Knob::Pitch, semis),
                (Knob::Pos, 0.2),
            ]);
            (0..96000)
                .map(|n| {
                    let v = (std::f32::consts::TAU * f * n as f32 / SR).sin();
                    g.process(v, v).0
                })
                .collect()
        };
        for (semis, pass, stop) in [(12.0, 6000.0, 15000.0), (24.0, 3000.0, 9000.0)] {
            let up = (semis / 12.0f32).exp2();
            let y = cloud(pass, semis);
            let kept = band(&y[48000..], pass * up);
            assert!(kept > 0.5, "{semis} semitones: {} Hz kept at {kept:.3}", pass * up);
            let y = cloud(stop, semis);
            let alias = SR - stop * up;
            let folded = band(&y[48000..], alias.abs());
            assert!(folded < 0.003, "{semis} semitones: {stop} Hz folds to {alias} Hz at {folded:.4}");
        }
        // every slower copy lines up with the recording in time: a 150 Hz
        // tone reads the same from each, at any position
        let mut g = engine(&[]);
        for n in 0..48000 {
            let v = (std::f32::consts::TAU * 150.0 * n as f32 / SR).sin();
            g.process(v, -v);
        }
        for k in 1..LEVELS {
            let (_, newest) = g.readable(k);
            for i in 0..200 {
                let p = newest - 4000.0 - 13.37 * i as f64;
                let (a, b) = (g.read(0, p), g.read(k, p));
                assert!((a[0] - b[0]).abs() < 1e-3, "copy {k} at {p}: {} vs {}", b[0], a[0]);
                assert!((a[1] - b[1]).abs() < 1e-3, "copy {k} right at {p}");
            }
        }
    }

    /// Freezing and thawing every 37 ms under a steady tone never clicks:
    /// the output's second difference stays near the tone's own curvature
    /// (a jump between unrelated samples would be a hundred times it), for
    /// grains at the head's own speed right at the head (the worst case:
    /// they meet the stopped head at once) and for scattered, pitched,
    /// reversed ones.
    #[test]
    fn freezing_and_thawing_fast_never_clicks() {
        let settings: [&[(Knob, f32)]; 2] = [
            &[(Knob::Mix, 1.0), (Knob::Size, 80.0), (Knob::Density, 40.0)],
            &[
                (Knob::Mix, 1.0),
                (Knob::Size, 60.0),
                (Knob::Density, 120.0),
                (Knob::Pos, 0.3),
                (Knob::Spray, 0.25),
                (Knob::PitchScatter, 7.0),
                (Knob::Reverse, 0.3),
                (Knob::Jitter, 1.0),
            ],
        ];
        for knobs in settings {
            let mut g = engine(knobs);
            let tone = |n: usize| 0.5 * (std::f32::consts::TAU * 220.0 * n as f32 / SR).sin();
            let mut y = Vec::new();
            for n in 0..(3.0 * SR) as usize {
                if n >= SR as usize && (n - SR as usize) % 1777 == 0 {
                    let frozen = ((n - SR as usize) / 1777) % 2 == 0;
                    g.set(Knob::Freeze, if frozen { 1.0 } else { 0.0 });
                }
                let x = tone(n);
                y.push(g.process(x, x).0);
            }
            let worst = y[SR as usize..]
                .windows(3)
                .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
                .fold(0.0f32, f32::max);
            // the tone alone: 0.5 (2 pi 220 / 48000)^2 = 4e-4; a held
            // sample's corner is the tone's slope, 0.014; unfaded, a grain
            // meeting the stopped head reads 0.39
            assert!(worst < 0.02, "a click: second difference {worst:.4}");
        }
    }

    /// Frozen, the buffer stops recording: a 440 Hz tone caught before the
    /// freeze keeps sounding after the input moves to 1 kHz, and the new
    /// tone never gets in.
    #[test]
    fn a_frozen_buffer_keeps_what_it_caught() {
        let mut g = engine(&[
            (Knob::Mix, 1.0),
            (Knob::Size, 80.0),
            (Knob::Density, 50.0),
            (Knob::Pos, 0.3),
            (Knob::Spray, 0.2),
        ]);
        for n in 0..48000 {
            let v = (std::f32::consts::TAU * 440.0 * n as f32 / SR).sin();
            g.process(v, v);
        }
        g.set(Knob::Freeze, 1.0);
        let y: Vec<f32> = (0..96000)
            .map(|n| {
                let v = (std::f32::consts::TAU * 1000.0 * n as f32 / SR).sin();
                g.process(v, v).0
            })
            .collect();
        let late = &y[48000..];
        let (kept, new) = (band(late, 440.0), band(late, 1000.0));
        assert!(kept > 0.3, "the frozen tone faded: {kept:.4}");
        assert!(new < 0.01 * kept, "the new tone got into a frozen buffer: {new:.5}");
    }

    /// gran_reverse 1 plays every grain backwards: a grain cut from a
    /// rising chirp falls. Zero crossings crowd together through a forward
    /// grain and spread apart through a reversed one.
    #[test]
    fn a_reversed_grain_plays_its_stretch_backwards() {
        let crossings_trend = |reverse: f32| -> f32 {
            // one 200 ms grain, the next one ten seconds away
            let mut g = engine(&[
                (Knob::Mix, 1.0),
                (Knob::Size, 200.0),
                (Knob::Density, 0.1),
                (Knob::Pos, 1.0),
                (Knob::Reverse, reverse),
            ]);
            g.wet = 1.0;
            // a chirp sweeping 200 Hz -> 2 kHz every second
            let mut ph = 0.0f64;
            let mut y = Vec::new();
            for n in 0..72000 {
                let t = (n % 48000) as f64 / SR as f64;
                ph += std::f64::consts::TAU * (200.0 + 1800.0 * t) / SR as f64;
                let v = ph.sin() as f32;
                // engage at 1.1 s, so the grain is cut from 0.1-0.3 s
                if n == 52800 {
                    g.until_next = 0.0;
                }
                y.push(g.process(v, v).0);
            }
            let grain = &y[52800..52800 + 9600];
            let zc: Vec<usize> = (1..grain.len())
                .filter(|&i| (grain[i - 1] < 0.0) != (grain[i] < 0.0))
                .collect();
            let half = zc.len() / 2;
            let gap = |s: &[usize]| (s[s.len() - 1] - s[0]) as f32 / (s.len() - 1) as f32;
            // later gaps over earlier gaps: < 1 rising, > 1 falling
            gap(&zc[half..]) / gap(&zc[..half])
        };
        let fwd = crossings_trend(0.0);
        let back = crossings_trend(1.0);
        assert!(fwd < 0.85, "a forward grain of a rising chirp rises: {fwd:.3}");
        assert!(back > 1.15, "a reversed grain of it falls: {back:.3}");
    }

    /// Pan scatter spreads a mono source across the field (left and right
    /// decorrelate), none keeps it centred, and a dense sprayed cloud sits
    /// within a few dB of the input's loudness.
    #[test]
    fn pan_scatter_spreads_and_a_dense_cloud_keeps_the_level() {
        let run = |pan: f32| -> (Vec<f32>, Vec<f32>) {
            let mut g = engine(&[
                (Knob::Mix, 1.0),
                (Knob::Size, 120.0),
                (Knob::Density, 80.0),
                (Knob::Pos, 1.0),
                (Knob::Spray, 0.8),
                (Knob::Jitter, 1.0),
                (Knob::PanScatter, pan),
            ]);
            let mut noise = 0x1234u32;
            (0..144000)
                .map(|_| {
                    let v = rng::bipolar(&mut noise) * 0.5;
                    g.process(v, v)
                })
                .skip(96000)
                .unzip()
        };
        let corr = |a: &[f32], b: &[f32]| {
            let (mut ab, mut aa, mut bb) = (0.0f64, 0.0f64, 0.0f64);
            for (&x, &y) in a.iter().zip(b) {
                ab += (x * y) as f64;
                aa += (x * x) as f64;
                bb += (y * y) as f64;
            }
            ab / (aa * bb).sqrt()
        };
        let (l0, r0) = run(0.0);
        let (l1, r1) = run(1.0);
        assert!(corr(&l0, &r0) > 0.9999, "no scatter keeps a mono source centred");
        let c = corr(&l1, &r1);
        assert!(c < 0.8, "full pan scatter: L/R correlation {c:.3}");
        // input rms 0.5/sqrt(3) = 0.289
        let rms = (l0.iter().map(|v| (v * v) as f64).sum::<f64>() / l0.len() as f64).sqrt();
        let db = 20.0 * (rms / 0.2887).log10();
        assert!(db.abs() < 3.0, "a 9.6-overlap cloud at {db:+.1} dB re the input");
    }

    /// A pitch set gives each grain one of its intervals: a {0, 12} cloud
    /// of a 440 Hz tone sounds at 440 and 880 Hz, about evenly, and
    /// nowhere between; {7} alone moves the whole cloud a fifth up.
    #[test]
    fn a_pitch_set_gives_each_grain_one_interval() {
        let cloud = |set: &[f32]| -> Vec<f32> {
            let mut g = engine(&[
                (Knob::Mix, 1.0),
                (Knob::Size, 60.0),
                (Knob::Density, 40.0),
                (Knob::Pos, 0.3),
                (Knob::Jitter, 1.0),
            ]);
            g.set_pitches(set);
            (0..144000)
                .map(|n| {
                    let v = (std::f32::consts::TAU * 440.0 * n as f32 / SR).sin();
                    g.process(v, v).0
                })
                .skip(48000)
                .collect()
        };
        let y = cloud(&[0.0, 12.0]);
        let (lo, hi, mid) = (band(&y, 440.0), band(&y, 880.0), band(&y, 660.0));
        assert!(lo > 0.15 && hi > 0.15, "both intervals sound: 440 {lo:.3}, 880 {hi:.3}");
        assert!((20.0 * (lo / hi).log10()).abs() < 4.0, "about evenly: 440 {lo:.3}, 880 {hi:.3}");
        assert!(mid < 0.1 * lo.min(hi), "nothing between: 660 {mid:.4}");
        let fifth = (7.0f32 / 12.0).exp2() * 440.0;
        let y = cloud(&[7.0]);
        assert!(band(&y, fifth) > 20.0 * band(&y, 440.0), "a one-interval set moves the cloud");
        // an empty set draws nothing: the same cloud as no set at all
        let mut a = engine(&[(Knob::Mix, 1.0), (Knob::PitchScatter, 5.0), (Knob::Jitter, 1.0)]);
        let mut b = a.clone();
        b.set_pitches(&[]);
        for n in 0..48000 {
            let v = ((n * 7919) % 1001) as f32 / 500.0 - 1.0;
            assert_eq!(a.process(v, v), b.process(v, v));
        }
    }

    /// The send form returns only the cloud: a two-overlap cloud of a
    /// signal is that signal gran_pos late, with no dry copy on top; at
    /// gran 0 it returns silence while it keeps recording.
    #[test]
    fn the_send_form_returns_only_the_cloud() {
        let mut g = Granular::new_send(SR);
        for (k, v) in [(Knob::Size, 50.0), (Knob::Density, 40.0), (Knob::Pos, 0.1)] {
            g.set(k, v);
        }
        let x: Vec<f32> = (0..48000)
            .map(|n| (std::f32::consts::TAU * 220.0 * n as f32 / SR).sin() * 0.5)
            .collect();
        let y: Vec<f32> = x.iter().map(|&v| g.process_send(v, v).0).collect();
        let mut worst = 0.0f32;
        for n in 4800 + 2400..48000 {
            worst = worst.max((y[n] - x[n - 4800]).abs());
        }
        assert!(worst < 1e-5, "the return strays {worst} from the send 4800 samples back");
        assert!(y[..4800].iter().all(|v| v.abs() < 1e-6), "no dry copy before the grains reach it");
        // closed, the return fades out over its 10 ms smoothing and then
        // is exactly silent
        g.set(Knob::Mix, 0.0);
        let tail: Vec<f32> = (0..24000).map(|n| g.process_send(x[n], x[n]).0).collect();
        assert!(tail[12000..].iter().all(|&v| v == 0.0), "closed, the send returns nothing");
        assert_eq!(g.head, 48000 + 24000);
    }

    /// The send's channel is no track's, slot's, kit's or instrument's.
    #[test]
    fn the_send_channel_is_its_own() {
        let c = GRAN_SEND_CHANNEL;
        assert!(crate::sampler::slot_for_channel(c).is_none());
        assert!(crate::drums::kit_for_channel(c).is_none());
        assert!(crate::physical::instrument_for_channel(c).is_none());
        assert!(c != crate::vox::VOX_CHANNEL && c != crate::drums::DRUM_CHANNEL);
        assert!(c > crate::sampler::SAMPLER_CHANNEL_BASE + crate::sampler::MAX_SLOTS as u16 - 1);
    }

    /// At gran 0 the insert is a wire: bit-identical output, while it keeps
    /// recording for when it opens.
    #[test]
    fn out_is_a_wire_that_keeps_recording() {
        let mut g = engine(&[(Knob::Size, 30.0), (Knob::Density, 90.0), (Knob::Pos, 0.05)]);
        for n in 0..20000 {
            let v = ((n * 7919) % 1001) as f32 / 500.0 - 1.0;
            assert_eq!(g.process(v, 0.5 * v), (v, 0.5 * v));
        }
        assert_eq!(g.head, 20000);
    }
}
