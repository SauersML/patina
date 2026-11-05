// src/body.rs
//
// Bodies: a track inside a modelled object or room (`track bell body=glass`,
// or the whole mix bus with a `body cathedral` line), heard through a
// convolution with an impulse response generated here from the body's
// physics, and a morph that carries it from one body into another
// (`body=room..cathedral`, `automate bell.body_morph`). Nothing is sampled
// or downloaded.
//
// Every body is one parametric model, so any two can be blended into a
// plausible third:
//   - modes: the structure's own resonances, each a frequency, a decay
//     (T60) and a level: a wine glass's ring modes (m = 2..7 of a thin ring,
//     f ∝ m(m²-1)/sqrt(m²+1): ratios 1, 2.83, 5.42, 8.77, 12.9, 17.7 over
//     760 Hz, off the equal-tempered notes as a real glass is, each a
//     slightly split doublet that beats, as a real glass does), a steel box's panel bending modes (f_mn = (π/2) sqrt(D/ρh)
//     ((m/a)² + (n/b)²) for 1.2 mm steel), plywood panels and a port for
//     the wooden crate, a 2 m tube's axial series (n c / 2L), a room's own
//     low modes. Struck, each rises from zero as a decaying sine.
//   - the cavity: a shoebox (dims, metres) whose early reflections come
//     from the image-source method out to 1.5 mixing times (sqrt(V) ms),
//     walls reflecting sqrt(1-α) with α from Sabine's T60 = 0.161 V / (S α),
//     and whose diffuse tail is noise at the level the diffuse field holds
//     against the direct sound (4/R against 1/(4π d²), R = S α / (1-α)),
//     every frequency decaying at its own T60: shaped frame by frame in a
//     short-time transform from three anchors (250 Hz and below, 1-2 kHz,
//     8 kHz and up, log-frequency between), so there are no crossovers
//   - the direct path, from the source to the listener
// Times are measured from the direct sound's arrival, so a body adds no
// delay of its own, and the response is scaled to unit energy, so a
// broadband part keeps its loudness inside any body. A long-ringing mode
// gives a steady tone right on it far more than that (a glass mode, 4 s
// long and half a hertz wide, +40 dB), as a real body does; the glass's
// modes sit between the equal-tempered notes. Left and right are two
// listening points an ear spacing apart (less in a small body); each mode
// reaches both with one sign and size, a moment apart, as a wave from its
// own direction, and the tails are independent noise.
//
// Bodies: glass, metal (a steel box), wood (a wooden crate), tube (a 2 m
// steel tube), room, hall, cathedral.
//
// Morph: `body_morph` 0..1 runs through 17 bodies, the two ends and 15
// interpolated between them (dimensions and decay times geometrically,
// the modes paired in order of frequency, the unmatched ones fading), all
// generated in parallel before the first sample, and crossfades between
// the two nearest. Every body shares one input history, so a body the
// morph arrives at already rings with everything played into the track
// before.
//
// The convolution is partitioned in tiers, so it adds no latency: the
// first 64 taps direct, taps 64-2047 in 64-sample blocks, 2048-16383 in
// 2048-sample blocks, the rest in 16384-sample blocks (FFT 32768), each
// block computed the moment its input is complete, one block before it
// sounds.

use std::sync::Arc;

use crate::rng;
use crate::spectral::fft;

/// Taps convolved directly, sample by sample.
const DIRECT: usize = 64;
/// The block tiers: (block length, the tap where the tier ends). A tier
/// of block B starts at tap B (the previous tier's end), so each block is
/// complete one block before it sounds.
const TIERS: [(usize, usize); 3] = [(64, 2048), (2048, 16384), (16384, usize::MAX)];
/// The longest impulse response, seconds.
const MAX_IR_S: f32 = 12.0;
/// Bodies a morph passes through, end to end.
const MORPH_STEPS: usize = 16;
const SOUND_C: f32 = 343.0;

/// One of the bodies a song can name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BodyKind {
    Glass,
    Metal,
    Wood,
    Tube,
    Room,
    Hall,
    Cathedral,
}

impl BodyKind {
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "glass" => Self::Glass,
            "metal" => Self::Metal,
            "wood" => Self::Wood,
            "tube" => Self::Tube,
            "room" => Self::Room,
            "hall" => Self::Hall,
            "cathedral" => Self::Cathedral,
            _ => return None,
        })
    }

    pub const NAMES: &'static str = "glass, metal, wood, tube, room, hall, cathedral";

    pub fn spec(self) -> BodySpec {
        let mut seed = rng::seed(match self {
            Self::Glass => 11,
            Self::Metal => 12,
            Self::Wood => 13,
            Self::Tube => 14,
            Self::Room => 15,
            Self::Hall => 16,
            Self::Cathedral => 17,
        });
        match self {
            Self::Glass => {
                // a thin ring's flexural modes, m = 2..7, each a doublet
                let ring = |m: f32| m * (m * m - 1.0) / (m * m + 1.0).sqrt();
                let f1 = 760.0;
                let t60 = [4.0, 3.2, 2.4, 1.8, 1.3, 1.0];
                let gain = [1.0, 0.6, 0.45, 0.3, 0.22, 0.15];
                let mut modes = Vec::new();
                for (i, m) in (2..=7).enumerate() {
                    let f = f1 * ring(m as f32) / ring(2.0);
                    modes.push(Mode { freq: f, t60: t60[i], gain: gain[i] });
                    modes.push(Mode { freq: f * 1.0015, t60: t60[i], gain: 0.7 * gain[i] });
                }
                BodySpec {
                    dims: [0.07, 0.07, 0.11],
                    t60: [0.12, 0.08, 0.05],
                    modes,
                    mode_share: 0.95,
                    direct: 0.0,
                }
            }
            Self::Metal => {
                // 1.2 mm steel panels: E 200 GPa, ν 0.3, ρ 7850
                let k = plate_coefficient(200e9, 0.3, 7850.0, 0.0012);
                let mut modes = Vec::new();
                for &(a, b) in &[(0.6f32, 0.45f32), (0.6, 0.35), (0.45, 0.35)] {
                    plate_modes(&mut modes, k, a, b, 150.0, 8000.0, 40, &mut seed, |f| {
                        (3.0 * (f / 400.0).powf(-0.4)).clamp(0.4, 3.5)
                    });
                }
                BodySpec {
                    dims: [0.6, 0.45, 0.35],
                    t60: [0.8, 0.6, 0.35],
                    modes,
                    mode_share: 0.5,
                    direct: 0.1,
                }
            }
            Self::Wood => {
                // 12 mm plywood: E 8 GPa, ν 0.3, ρ 600; and the port
                let k = plate_coefficient(8e9, 0.3, 600.0, 0.012);
                let mut modes = vec![Mode { freq: 140.0, t60: 0.3, gain: 1.0 }];
                for &(a, b) in &[(0.5f32, 0.35f32), (0.5, 0.3)] {
                    plate_modes(&mut modes, k, a, b, 150.0, 4000.0, 15, &mut seed, |f| {
                        (0.35 * (f / 300.0).powf(-0.5)).clamp(0.06, 0.4)
                    });
                }
                BodySpec {
                    dims: [0.5, 0.35, 0.3],
                    t60: [0.3, 0.18, 0.08],
                    modes,
                    mode_share: 0.4,
                    direct: 0.25,
                }
            }
            Self::Tube => {
                // the axial series of a 2 m tube, and a few shell modes
                let mut modes: Vec<Mode> = (1..=40)
                    .map(|n| {
                        let f = n as f32 * SOUND_C / 4.0;
                        Mode {
                            freq: f,
                            t60: (1.6 * (f / 100.0).powf(-0.3)).clamp(0.3, 1.6),
                            gain: 1.0 / (n as f32).sqrt(),
                        }
                    })
                    .collect();
                for _ in 0..10 {
                    modes.push(Mode {
                        freq: rng::range(&mut seed, 1000.0, 6000.0),
                        t60: 1.5,
                        gain: 0.3,
                    });
                }
                BodySpec {
                    dims: [2.0, 0.08, 0.08],
                    t60: [1.2, 0.9, 0.5],
                    modes,
                    mode_share: 0.7,
                    direct: 0.05,
                }
            }
            Self::Room => {
                // the room's own low modes, below 150 Hz
                let dims = [4.5f32, 3.6, 2.7];
                let mut modes = Vec::new();
                for nx in 0..4 {
                    for ny in 0..4 {
                        for nz in 0..3 {
                            if nx + ny + nz == 0 {
                                continue;
                            }
                            let f = 0.5
                                * SOUND_C
                                * ((nx as f32 / dims[0]).powi(2)
                                    + (ny as f32 / dims[1]).powi(2)
                                    + (nz as f32 / dims[2]).powi(2))
                                .sqrt();
                            if f < 150.0 {
                                modes.push(Mode { freq: f, t60: 0.6, gain: 1.0 });
                            }
                        }
                    }
                }
                BodySpec {
                    dims,
                    t60: [0.55, 0.45, 0.35],
                    modes,
                    mode_share: 0.05,
                    direct: 1.0,
                }
            }
            Self::Hall => BodySpec {
                dims: [32.0, 22.0, 14.0],
                t60: [2.4, 2.0, 1.3],
                modes: Vec::new(),
                mode_share: 0.0,
                direct: 1.0,
            },
            Self::Cathedral => BodySpec {
                dims: [80.0, 30.0, 35.0],
                t60: [8.0, 6.5, 3.5],
                modes: Vec::new(),
                mode_share: 0.0,
                direct: 1.0,
            },
        }
    }
}

/// sqrt(D / ρh) × π/2 for a plate: f_mn = this × ((m/a)² + (n/b)²).
fn plate_coefficient(e: f32, nu: f32, rho: f32, h: f32) -> f32 {
    let d = e * h * h * h / (12.0 * (1.0 - nu * nu));
    std::f32::consts::FRAC_PI_2 * (d / (rho * h)).sqrt()
}

/// About `keep` of an a × b plate's bending modes between lo and hi Hz,
/// chosen at random, each with the decay `t60(f)`.
#[allow(clippy::too_many_arguments)]
fn plate_modes(
    out: &mut Vec<Mode>,
    k: f32,
    a: f32,
    b: f32,
    lo: f32,
    hi: f32,
    keep: usize,
    seed: &mut u32,
    t60: impl Fn(f32) -> f32,
) {
    let mut all = Vec::new();
    for m in 1..80 {
        for n in 1..80 {
            let f = k * ((m as f32 / a).powi(2) + (n as f32 / b).powi(2));
            if f >= lo && f <= hi {
                all.push(f);
            }
        }
    }
    let p = (keep as f32 / all.len().max(1) as f32).min(1.0);
    for f in all {
        if rng::unipolar(seed) < p {
            out.push(Mode {
                freq: f,
                t60: t60(f),
                gain: rng::range(seed, 0.4, 1.0) / (f / 300.0).sqrt(),
            });
        }
    }
}

/// One resonance: Hz, seconds to fall 60 dB, amplitude.
#[derive(Clone, Copy, Debug)]
pub struct Mode {
    pub freq: f32,
    pub t60: f32,
    pub gain: f32,
}

/// A body, in the one parametric form every body shares.
#[derive(Clone, Debug)]
pub struct BodySpec {
    /// The cavity, metres.
    pub dims: [f32; 3],
    /// The cavity's decay below 500 Hz, in the middle and above 4 kHz.
    pub t60: [f32; 3],
    pub modes: Vec<Mode>,
    /// The modes' share of the response's energy.
    pub mode_share: f32,
    /// The direct path's level against the physical one (0 = the
    /// listener only hears the body, as inside a glass).
    pub direct: f32,
}

impl BodySpec {
    /// The body `t` of the way from `a` to `b`.
    pub fn between(a: &BodySpec, b: &BodySpec, t: f32) -> BodySpec {
        let t = t.clamp(0.0, 1.0);
        let geo = |x: f32, y: f32| x.max(1e-6).powf(1.0 - t) * y.max(1e-6).powf(t);
        let lin = |x: f32, y: f32| x + (y - x) * t;
        let mut ma = a.modes.clone();
        let mut mb = b.modes.clone();
        ma.sort_by(|x, y| x.freq.total_cmp(&y.freq));
        mb.sort_by(|x, y| x.freq.total_cmp(&y.freq));
        let n = ma.len().max(mb.len());
        let mut modes = Vec::with_capacity(n);
        for i in 0..n {
            match (ma.get(i), mb.get(i)) {
                (Some(x), Some(y)) => modes.push(Mode {
                    freq: geo(x.freq, y.freq),
                    t60: geo(x.t60, y.t60),
                    gain: lin(x.gain, y.gain),
                }),
                (Some(x), None) => modes.push(Mode { gain: x.gain * (1.0 - t), ..*x }),
                (None, Some(y)) => modes.push(Mode { gain: y.gain * t, ..*y }),
                (None, None) => {}
            }
        }
        BodySpec {
            dims: [geo(a.dims[0], b.dims[0]), geo(a.dims[1], b.dims[1]), geo(a.dims[2], b.dims[2])],
            t60: [geo(a.t60[0], b.t60[0]), geo(a.t60[1], b.t60[1]), geo(a.t60[2], b.t60[2])],
            modes,
            mode_share: lin(a.mode_share, b.mode_share),
            direct: lin(a.direct, b.direct),
        }
    }

    /// The response's length at `sr`: until its slowest part has fallen
    /// 63 dB, past the mixing time.
    fn ir_len(&self, sr: f32) -> usize {
        let longest = self
            .modes
            .iter()
            .filter(|m| m.gain > 0.0)
            .map(|m| m.t60)
            .chain(self.t60.iter().copied())
            .fold(0.0f32, f32::max);
        (((longest * 1.05 + self.mixing_time()) * sr) as usize).clamp(2048, (MAX_IR_S * sr) as usize)
    }

    /// When the reflections have become a diffuse field: sqrt(V) ms.
    fn mixing_time(&self) -> f32 {
        let [lx, ly, lz] = self.dims;
        ((lx * ly * lz).sqrt() * 0.001).clamp(0.001, 0.1)
    }

    /// The body's impulse response at `sr`: (left, right), unit energy.
    pub fn impulse_response(&self, sr: f32, seed: u32) -> (Vec<f32>, Vec<f32>) {
        let len = self.ir_len(sr);
        let t_mix = self.mixing_time();
        let [lx, ly, lz] = self.dims;
        let vol = lx * ly * lz;
        let area = 2.0 * (lx * ly + lx * lz + ly * lz);
        let mut out = [vec![0.0f32; len], vec![0.0f32; len]];
        let mut modal = [vec![0.0f32; len], vec![0.0f32; len]];
        // Sabine's absorption for the middle band, and the diffuse field
        let alpha = (0.161 * vol / (area * self.t60[1].max(0.01))).clamp(0.01, 0.95);
        let beta = (1.0 - alpha).sqrt();
        let r_const = area * alpha / (1.0 - alpha);
        let src = [0.35 * lx, 0.6 * ly, 0.4 * lz];
        // two listening points an ear spacing apart (less in a small body)
        let sep = (0.1 * lx).min(0.17);
        for c in 0..2 {
            let mut s = seed ^ rng::seed(c as u32 + 101);
            let listener = [0.55 * lx + (c as f32 - 0.5) * sep, 0.3 * ly, 0.5 * lz];
            let dist = |p: [f32; 3]| {
                ((p[0] - listener[0]).powi(2) + (p[1] - listener[1]).powi(2) + (p[2] - listener[2]).powi(2))
                    .sqrt()
            };
            let d0 = dist(src).max(0.01);
            let h = &mut out[c];
            // the direct path, at the response's start
            h[0] += self.direct;
            // early reflections: image sources out to 1.5 mixing times
            let reach = SOUND_C * 1.5 * t_mix + d0;
            let span = |l: f32| (reach / (2.0 * l)).ceil() as i32 + 1;
            let mut early_e = 0.0f32;
            let mut images = 0;
            'img: for nx in -span(lx)..=span(lx) {
                for ny in -span(ly)..=span(ly) {
                    for nz in -span(lz)..=span(lz) {
                        for u in 0..8u32 {
                            let (ux, uy, uz) = ((u & 1) as i32, ((u >> 1) & 1) as i32, ((u >> 2) & 1) as i32);
                            let order = (2 * nx - ux).abs() + (2 * ny - uy).abs() + (2 * nz - uz).abs();
                            if order == 0 {
                                continue;
                            }
                            let mirror = |n: i32, flip: i32, l: f32, p: f32| {
                                2.0 * n as f32 * l + if flip == 1 { -p } else { p }
                            };
                            let d = dist([
                                mirror(nx, ux, lx, src[0]),
                                mirror(ny, uy, ly, src[1]),
                                mirror(nz, uz, lz, src[2]),
                            ]);
                            if d > reach {
                                continue;
                            }
                            let t = (d - d0) / SOUND_C * sr;
                            if t < 0.0 || t as usize + 1 >= len {
                                continue;
                            }
                            let g = beta.powi(order) * d0 / d;
                            let i = t as usize;
                            let f = t - i as f32;
                            h[i] += g * (1.0 - f);
                            h[i + 1] += g * f;
                            early_e += g * g;
                            images += 1;
                            if images >= 6000 {
                                break 'img;
                            }
                        }
                    }
                }
            }
            // the diffuse tail: the field's energy against the unit direct
            // sound (4/R against 1/4πd²), less what the reflections hold
            let rev_e = (4.0 / r_const.max(1e-6)) * 4.0 * std::f32::consts::PI * d0 * d0;
            let tail_e = (rev_e - early_e).max(0.1 * rev_e);
            let tail = diffuse_tail(self.t60, len, sr, &mut s);
            // it fades in over the mixing time, from half a mixing time on
            let t0 = ((t_mix * sr) as usize).max(1);
            let fade = |n: usize| {
                if n < t0 / 2 {
                    0.0
                } else if n < t0 / 2 + t0 {
                    let u = (n - t0 / 2) as f32 / t0 as f32;
                    0.5 - 0.5 * (std::f32::consts::PI * u).cos()
                } else {
                    1.0
                }
            };
            let env_e: f32 = tail.iter().enumerate().map(|(n, v)| (v * fade(n)).powi(2)).sum();
            let g = (tail_e / env_e.max(1e-12)).sqrt();
            for (n, v) in tail.iter().enumerate() {
                h[n] += g * fade(n) * v;
            }
        }
        // the modes: struck, each rises from zero as a decaying sine. The
        // listening points are close, so a mode reaches both with one sign
        // and size, a moment apart, as a wave from its own direction
        let mut ms = seed ^ rng::seed(77);
        for mode in &self.modes {
            // every mode draws, heard or not, so a morph's neighbours agree
            let sign = if rng::unipolar(&mut ms) < 0.5 { -1.0 } else { 1.0 };
            let a = sign * rng::range(&mut ms, 0.6, 1.0) as f64 * mode.gain as f64;
            let lag = (0.5 * sep * rng::range(&mut ms, -1.0, 1.0) / SOUND_C * sr) as f64;
            if mode.gain <= 0.0 || mode.freq >= 0.45 * sr {
                continue;
            }
            let w = std::f64::consts::TAU * mode.freq as f64 / sr as f64;
            let decay = (-6.9078 / (mode.t60.max(0.005) as f64 * sr as f64)).exp();
            let (cw, sw) = (w.cos() * decay, w.sin() * decay);
            let base = (0.5 * sep / SOUND_C * sr) as f64;
            for (c, m) in modal.iter_mut().enumerate() {
                let d = base + if c == 0 { -lag } else { lag };
                let start = d.ceil().max(0.0) as usize;
                let ph = w * (start as f64 - d);
                let (mut re, mut im) = (a * ph.cos(), a * ph.sin());
                for v in m.iter_mut().skip(start) {
                    *v += im as f32;
                    (re, im) = (re * cw - im * sw, re * sw + im * cw);
                }
            }
        }
        // the shares: the modes hold mode_share of the energy, the cavity
        // and the direct path the rest; then unit energy overall
        let energy = |v: &[Vec<f32>; 2]| -> f32 {
            0.5 * v.iter().map(|h| h.iter().map(|x| x * x).sum::<f32>()).sum::<f32>()
        };
        let share = self.mode_share.clamp(0.0, 1.0);
        let (ec, em) = (energy(&out), energy(&modal));
        let gc = if ec > 0.0 { ((1.0 - share) / ec).sqrt() } else { 0.0 };
        let gm = if em > 0.0 { (share / em).sqrt() } else { 0.0 };
        for c in 0..2 {
            for n in 0..len {
                out[c][n] = out[c][n] * gc + modal[c][n] * gm;
            }
        }
        let total = energy(&out);
        let g = if total > 0.0 { 1.0 / total.sqrt() } else { 0.0 };
        for h in out.iter_mut() {
            for v in h.iter_mut() {
                *v *= g;
            }
        }
        let [l, r] = out;
        (l, r)
    }
}

/// The decay at `f` Hz from a body's three bands: `t60[0]` at 250 Hz and
/// below, `t60[1]` from 1 to 2 kHz, `t60[2]` at 8 kHz and above, and
/// geometric between, along log frequency.
fn t60_at(t60: [f32; 3], f: f32) -> f32 {
    let lf = f.max(1.0).log2();
    let (a, b, c, d) = (250f32.log2(), 1000f32.log2(), 2000f32.log2(), 8000f32.log2());
    let geo = |x: f32, y: f32, u: f32| x.max(1e-3).powf(1.0 - u) * y.max(1e-3).powf(u);
    if lf <= a {
        t60[0]
    } else if lf < b {
        geo(t60[0], t60[1], (lf - a) / (b - a))
    } else if lf <= c {
        t60[1]
    } else if lf < d {
        geo(t60[1], t60[2], (lf - c) / (d - c))
    } else {
        t60[2]
    }
}

/// A diffuse field's decay: white noise shaped frame by frame in a
/// short-time transform, each bin falling at its own frequency's T60 from
/// frame to frame, and within a frame at the middle band's (so the frames'
/// seams carry no steps), every gain at most one.
fn diffuse_tail(t60: [f32; 3], len: usize, sr: f32, seed: &mut u32) -> Vec<f32> {
    const N: usize = 1024;
    const HOP: usize = N / 4;
    let tref = t60[1].max(0.005);
    let x: Vec<f32> = (0..len).map(|_| rng::bipolar(seed)).collect();
    let win: Vec<f32> =
        (0..N).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / N as f32).cos()).collect();
    // the within-frame decay about the frame's centre
    let inner: Vec<f32> = (0..N)
        .map(|i| win[i] * (-6.9078 * (i as f32 - N as f32 / 2.0) / (tref * sr)).exp())
        .collect();
    let rate: Vec<f32> =
        (0..=N / 2).map(|k| 6.9078 / t60_at(t60, k as f32 * sr / N as f32).max(0.005)).collect();
    let mut out = vec![0.0f32; len];
    let (mut re, mut im) = (vec![0.0f32; N], vec![0.0f32; N]);
    // Hann in and out at a quarter-frame hop sums to 1.5
    let mut start = HOP as isize - N as isize;
    while start < len as isize {
        let t = (start as f32 + N as f32 / 2.0) / sr;
        if t > 0.0 && rate.iter().all(|r| r * t > 40.0) {
            break;
        }
        for i in 0..N {
            let n = start + i as isize;
            re[i] = if n >= 0 && (n as usize) < len { x[n as usize] * inner[i] } else { 0.0 };
            im[i] = 0.0;
        }
        fft(&mut re, &mut im, false);
        for k in 0..N {
            let g = (-rate[k.min(N - k)] * t.max(0.0)).exp();
            re[k] *= g;
            im[k] *= g;
        }
        fft(&mut re, &mut im, true);
        for i in 0..N {
            let n = start + i as isize;
            if n >= 0 && (n as usize) < len {
                out[n as usize] += re[i] * win[i] / 1.5;
            }
        }
        start += HOP as isize;
    }
    out
}

/// One impulse response: its direct taps, and each tier's partitions.
/// Each partition is the transform of left + i·right, so one inverse
/// transform gives both channels.
struct Ir {
    direct: Vec<(f32, f32)>,
    tiers: Vec<Vec<(Vec<f32>, Vec<f32>)>>,
}

impl Ir {
    fn new(l: &[f32], r: &[f32]) -> Self {
        let len = l.len().max(r.len());
        let at = |v: &[f32], i: usize| v.get(i).copied().unwrap_or(0.0);
        let direct = (0..DIRECT).map(|i| (at(l, i), at(r, i))).collect();
        let tiers = TIERS
            .iter()
            .map(|&(b, end)| {
                let parts = end.min(len).saturating_sub(b).div_ceil(b);
                (0..parts)
                    .map(|p| {
                        let (mut re, mut im) = (vec![0.0f32; 2 * b], vec![0.0f32; 2 * b]);
                        for k in 0..b {
                            re[k] = at(l, b + p * b + k);
                            im[k] = at(r, b + p * b + k);
                        }
                        fft(&mut re, &mut im, false);
                        (re, im)
                    })
                    .collect()
            })
            .collect();
        Self { direct, tiers }
    }
}

/// One tier's shared input: the last two blocks, and the transforms of
/// every block window still inside the longest response (newest at head).
#[derive(Clone)]
struct Tier {
    b: usize,
    fill: usize,
    inb: Vec<f32>,
    fdl: Vec<(Vec<f32>, Vec<f32>)>,
    head: usize,
    yr: Vec<f32>,
    yi: Vec<f32>,
}

impl Tier {
    fn new(b: usize, parts: usize) -> Self {
        Self {
            b,
            fill: 0,
            inb: vec![0.0; 2 * b],
            fdl: vec![(vec![0.0; 2 * b], vec![0.0; 2 * b]); parts.max(1)],
            head: 0,
            yr: vec![0.0; 2 * b],
            yi: vec![0.0; 2 * b],
        }
    }

    /// The block just completed: transform the window, push it.
    fn push(&mut self) {
        self.head = (self.head + 1) % self.fdl.len();
        let (re, im) = &mut self.fdl[self.head];
        re.copy_from_slice(&self.inb);
        im.iter_mut().for_each(|v| *v = 0.0);
        fft(re, im, false);
        self.inb.copy_within(self.b.., 0);
    }

    /// The coming block's output for one response's partitions: partition
    /// p (taps (p+1)b..(p+2)b) against the window that ended p blocks ago.
    fn block(&mut self, parts: &[(Vec<f32>, Vec<f32>)], out: &mut [(f32, f32)]) {
        let n = 2 * self.b;
        let depth = parts.len().min(self.fdl.len());
        if depth == 0 {
            out.iter_mut().for_each(|v| *v = (0.0, 0.0));
            return;
        }
        let (yr, yi) = (&mut self.yr[..n], &mut self.yi[..n]);
        yr.fill(0.0);
        yi.fill(0.0);
        let len = self.fdl.len();
        for (p, (hr, hi)) in parts.iter().enumerate().take(depth) {
            let (xr, xi) = &self.fdl[(self.head + len - p) % len];
            // equal-length slices, so the loop runs without bounds checks
            // and vectorises
            let (xr, xi, hr, hi) = (&xr[..n], &xi[..n], &hr[..n], &hi[..n]);
            for k in 0..n {
                yr[k] += xr[k] * hr[k] - xi[k] * hi[k];
                yi[k] += xr[k] * hi[k] + xi[k] * hr[k];
            }
        }
        fft(yr, yi, true);
        for (i, o) in out.iter_mut().enumerate() {
            *o = (yr[self.b + i], yi[self.b + i]);
        }
    }
}

/// A response the convolver is playing, with each tier's output block.
#[derive(Clone)]
struct Voice {
    index: usize,
    outs: Vec<Vec<(f32, f32)>>,
}

/// A mono-in, stereo-out convolver with no latency, over one shared
/// input history, playing any of its responses and able to start one at
/// any sample.
#[derive(Clone)]
struct Convolver {
    hist: Vec<f32>,
    hpos: usize,
    tiers: Vec<Tier>,
    irs: Arc<Vec<Ir>>,
    voices: Vec<Voice>,
}

impl Convolver {
    fn new(irs: Vec<Ir>) -> Self {
        let tiers = TIERS
            .iter()
            .enumerate()
            .map(|(t, &(b, _))| Tier::new(b, irs.iter().map(|ir| ir.tiers[t].len()).max().unwrap_or(0)))
            .collect();
        Self {
            hist: vec![0.0; DIRECT],
            hpos: 0,
            tiers,
            irs: Arc::new(irs),
            voices: Vec::new(),
        }
    }

    /// Play exactly these responses. One newly played gets this block's
    /// output from the shared history, so it rings with everything
    /// already played in, as if it had been playing all along.
    fn play(&mut self, slots: &[usize]) {
        self.voices.retain(|v| slots.contains(&v.index));
        for &s in slots {
            if self.voices.iter().any(|v| v.index == s) {
                continue;
            }
            let ir = &self.irs[s];
            let mut v = Voice {
                index: s,
                outs: TIERS.iter().map(|&(b, _)| vec![(0.0, 0.0); b]).collect(),
            };
            for (t, tier) in self.tiers.iter_mut().enumerate() {
                tier.block(&ir.tiers[t], &mut v.outs[t]);
            }
            self.voices.push(v);
        }
    }

    /// One sample in; each playing response's (left, right) through `each`.
    fn process(&mut self, x: f32, mut each: impl FnMut(usize, (f32, f32))) {
        self.hist[self.hpos] = x;
        for tier in self.tiers.iter_mut() {
            tier.inb[tier.b + tier.fill] = x;
        }
        for v in &self.voices {
            let ir = &self.irs[v.index];
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for (k, &(hl, hr)) in ir.direct.iter().enumerate() {
                let s = self.hist[(self.hpos + DIRECT - k) % DIRECT];
                l += hl * s;
                r += hr * s;
            }
            for (tier, out) in self.tiers.iter().zip(&v.outs) {
                let (a, b) = out[tier.fill];
                l += a;
                r += b;
            }
            each(v.index, (l, r));
        }
        self.hpos = (self.hpos + 1) % DIRECT;
        for (t, tier) in self.tiers.iter_mut().enumerate() {
            tier.fill += 1;
            if tier.fill == tier.b {
                tier.fill = 0;
                tier.push();
                for v in self.voices.iter_mut() {
                    tier.block(&self.irs[v.index].tiers[t], &mut v.outs[t]);
                }
            }
        }
    }
}

/// The body insert on a strip or the bus: one body, or a morph between two.
#[derive(Clone)]
pub struct Body {
    conv: Convolver,
    morphs: bool,
    mix: f32,
    mix_now: f32,
    morph: f32,
    morph_now: f32,
    k: f32,
    fresh: bool,
}

impl Body {
    pub fn new(sample_rate: f32, a: BodyKind, b: Option<BodyKind>) -> Self {
        Self::from_specs(sample_rate, a.spec(), b.map(BodyKind::spec))
    }

    /// Every body the morph passes through is generated here, side by
    /// side on as many threads, so nothing is built mid-song.
    pub fn from_specs(sample_rate: f32, a: BodySpec, b: Option<BodySpec>) -> Self {
        let specs: Vec<BodySpec> = match &b {
            Some(b) => (0..=MORPH_STEPS)
                .map(|i| BodySpec::between(&a, b, i as f32 / MORPH_STEPS as f32))
                .collect(),
            None => vec![a.clone()],
        };
        let seed = rng::seed(0xb0d1);
        let irs: Vec<Ir> = std::thread::scope(|scope| {
            let jobs: Vec<_> = specs
                .iter()
                .map(|spec| {
                    scope.spawn(move || {
                        let (l, r) = spec.impulse_response(sample_rate, seed);
                        Ir::new(&l, &r)
                    })
                })
                .collect();
            jobs.into_iter().map(|j| j.join().expect("a body generates")).collect()
        });
        let mut conv = Convolver::new(irs);
        conv.play(&[0]);
        Self {
            conv,
            morphs: b.is_some(),
            mix: 1.0,
            mix_now: 1.0,
            morph: 0.0,
            morph_now: 0.0,
            k: crate::smoothing::approach(0.05, sample_rate),
            fresh: true,
        }
    }

    pub fn set_mix(&mut self, v: f32) {
        self.mix = v.clamp(0.0, 1.0);
        if self.fresh {
            self.mix_now = self.mix;
        }
    }

    pub fn set_morph(&mut self, v: f32) {
        self.morph = v.clamp(0.0, 1.0);
        if self.fresh {
            self.morph_now = self.morph;
        }
    }

    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.fresh = false;
        self.mix_now += (self.mix - self.mix_now) * self.k;
        let (lo, frac) = if self.morphs {
            self.morph_now += (self.morph - self.morph_now) * self.k;
            if (self.morph - self.morph_now).abs() < 1e-6 {
                self.morph_now = self.morph;
            }
            let s = self.morph_now * MORPH_STEPS as f32;
            let lo = (s.floor() as usize).min(MORPH_STEPS - 1);
            (lo, (s - lo as f32).clamp(0.0, 1.0))
        } else {
            (0, 0.0)
        };
        // the one or two bodies the morph stands between
        let (want, n): ([usize; 2], usize) = if frac <= 0.0 {
            ([lo, lo], 1)
        } else if frac >= 1.0 {
            ([lo + 1, lo + 1], 1)
        } else {
            ([lo, lo + 1], 2)
        };
        if self.conv.voices.len() != n || self.conv.voices.iter().any(|v| !want[..n].contains(&v.index)) {
            self.conv.play(&want[..n]);
        }
        let x = 0.5 * (l + r);
        let (mut wl, mut wr) = (0.0f32, 0.0f32);
        self.conv.process(x, |i, (a, b)| {
            // the neighbours are nearly the same body, so they add in
            // amplitude: a linear crossfade
            let g = if n == 1 {
                1.0
            } else if i == lo {
                1.0 - frac
            } else {
                frac
            };
            wl += a * g;
            wr += b * g;
        });
        let m = self.mix_now;
        (l * (1.0 - m) + wl * m, r * (1.0 - m) + wr * m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    fn run(c: &mut Convolver, x: f32) -> (f32, f32) {
        let mut y = (0.0, 0.0);
        c.process(x, |_, s| y = s);
        y
    }

    /// A delta impulse response convolves to identity, and a delta
    /// anywhere in the response (the direct taps, either block tier, their
    /// seams) delays by exactly its position; a random response matches
    /// the textbook sum sample for sample; and a response started midway
    /// rings with what was played before it started.
    #[test]
    fn a_delta_convolves_to_identity_and_any_response_matches_direct() {
        let x: Vec<f32> = (0..40000).map(|n| ((n * 7919) % 1013) as f32 / 506.0 - 1.0).collect();
        for at in [0usize, 63, 64, 2047, 2048, 16383, 16384, 30000] {
            let mut h = vec![0.0f32; 32000];
            h[at] = 1.0;
            let mut c = Convolver::new(vec![Ir::new(&h, &h)]);
            c.play(&[0]);
            for (n, &v) in x.iter().enumerate() {
                let y = run(&mut c, v);
                let want = if n >= at { x[n - at] } else { 0.0 };
                assert!(
                    (y.0 - want).abs() < 1e-5 && (y.1 - want).abs() < 1e-5,
                    "a delta at {at}: sample {n} is {y:?}, want {want}"
                );
            }
        }
        let mut s = 99u32;
        let hl: Vec<f32> = (0..20000).map(|_| rng::bipolar(&mut s) * 0.02).collect();
        let hr: Vec<f32> = (0..20000).map(|_| rng::bipolar(&mut s) * 0.02).collect();
        let direct = |n: usize| {
            let (mut l, mut r) = (0.0f64, 0.0f64);
            for k in 0..=n.min(19999) {
                l += hl[k] as f64 * x[n - k] as f64;
                r += hr[k] as f64 * x[n - k] as f64;
            }
            (l as f32, r as f32)
        };
        // response 1 plays from the start; response 0 (silence) until
        // sample 19001, then response 1 takes over mid-block
        let silent = vec![0.0f32; 10];
        let mut c = Convolver::new(vec![Ir::new(&silent, &silent), Ir::new(&hl, &hr)]);
        c.play(&[1]);
        let mut late = c.clone();
        late.play(&[0]);
        let mut worst = 0.0f32;
        for n in 0..26000 {
            if n == 19001 {
                late.play(&[1]);
            }
            let (y, z) = (run(&mut c, x[n]), run(&mut late, x[n]));
            if n % 97 == 0 || (19001..19100).contains(&n) {
                let (l, r) = direct(n);
                worst = worst.max((y.0 - l).abs()).max((y.1 - r).abs());
                if n >= 19001 {
                    worst = worst.max((z.0 - l).abs()).max((z.1 - r).abs());
                }
            }
        }
        assert!(worst < 1e-4, "partitioned against direct: {worst}");
    }

    /// The decay of a response: Schroeder's backward integral, the slope
    /// from -5 to -25 dB, extrapolated to 60.
    fn t60(h: &[f32]) -> f32 {
        let mut e: Vec<f64> = h.iter().map(|v| (v * v) as f64).collect();
        for i in (0..e.len() - 1).rev() {
            e[i] += e[i + 1];
        }
        let total = e[0];
        let db = |i: usize| 10.0 * (e[i] / total).log10();
        let a = (0..e.len()).find(|&i| db(i) < -5.0).unwrap();
        let b = (0..e.len()).find(|&i| db(i) < -25.0).unwrap();
        3.0 * (b - a) as f32 / SR
    }

    /// An octave band of a response (a constant-peak band-pass, Q 1.41).
    fn octave(h: &[f32], f: f32) -> Vec<f32> {
        let w = std::f32::consts::TAU * f / SR;
        let alpha = w.sin() / (2.0 * 1.41);
        let a0 = 1.0 + alpha;
        let (b0, b2, a1, a2) = (alpha / a0, -alpha / a0, -2.0 * w.cos() / a0, (1.0 - alpha) / a0);
        let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        h.iter()
            .map(|&x| {
                let y = b0 * x + b2 * x2 - a1 * y1 - a2 * y2;
                (x2, x1, y2, y1) = (x1, x, y1, y);
                y
            })
            .collect()
    }

    /// A cathedral decays at its middle T60 in the 1 kHz octave and faster
    /// at 8 kHz, a room at its own; both keep unit energy; and a glass
    /// rings at a ring's mode ratios (1, 2.83, 5.42, 8.77 over 760 Hz).
    #[test]
    fn bodies_decay_and_ring_as_built() {
        let (cl, cr) = BodyKind::Cathedral.spec().impulse_response(SR, 5);
        let (rl, _) = BodyKind::Room.spec().impulse_response(SR, 5);
        let (c1k, c8k, r1k) = (t60(&octave(&cl, 1000.0)), t60(&octave(&cl, 8000.0)), t60(&octave(&rl, 1000.0)));
        assert!((5.8..7.2).contains(&c1k), "cathedral 1 kHz T60 {c1k:.2} s (built 6.5)");
        assert!((2.8..4.2).contains(&c8k), "cathedral 8 kHz T60 {c8k:.2} s (built 3.5)");
        assert!((0.38..0.52).contains(&r1k), "room 1 kHz T60 {r1k:.2} s (built 0.45)");
        // every body: finite, at unit energy
        for k in [
            BodyKind::Glass,
            BodyKind::Metal,
            BodyKind::Wood,
            BodyKind::Tube,
            BodyKind::Room,
            BodyKind::Hall,
        ] {
            let (l, r) = k.spec().impulse_response(SR, 5);
            let e = 0.5 * l.iter().chain(&r).map(|v| v * v).sum::<f32>();
            assert!((e - 1.0).abs() < 1e-3, "{k:?} energy {e:.4}");
        }
        let e = 0.5 * cl.iter().chain(&cr).map(|v| v * v).sum::<f32>();
        assert!((e - 1.0).abs() < 1e-3, "cathedral energy {e:.4}");
        let (gl, _) = BodyKind::Glass.spec().impulse_response(SR, 5);
        let amp = |f: f32| {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (n, &v) in gl.iter().enumerate().take(96000) {
                let ph = std::f64::consts::TAU * f as f64 * n as f64 / SR as f64;
                re += v as f64 * ph.cos();
                im += v as f64 * ph.sin();
            }
            (re * re + im * im).sqrt()
        };
        for ratio in [1.0f32, 2.829, 5.423, 8.770] {
            let f = 760.0 * ratio;
            let (on, off) = (amp(f), amp(f * 1.06));
            assert!(on > 10.0 * off, "glass mode at {f:.0} Hz: {on:.3} against {off:.3} beside it");
        }
    }

    /// The morph runs from one body to the other: halfway it decays
    /// between the two, and the insert at morph 0 and morph 1 is each end's
    /// response exactly.
    #[test]
    fn the_morph_carries_one_body_into_the_other() {
        let a = BodyKind::Room.spec();
        let b = BodyKind::Cathedral.spec();
        let seed = rng::seed(0xb0d1);
        let band = |spec: &BodySpec| {
            let (l, _) = spec.impulse_response(SR, seed);
            t60(&octave(&l, 1000.0))
        };
        let (ta, tm, tb) = (band(&a), band(&BodySpec::between(&a, &b, 0.5)), band(&b));
        assert!(ta < tm && tm < tb, "room {ta:.2} < halfway {tm:.2} < cathedral {tb:.2}");
        let click = |m: f32| -> Vec<f32> {
            let mut body = Body::from_specs(SR, a.clone(), Some(b.clone()));
            body.set_morph(m);
            (0..48000).map(|n| body.process(if n == 0 { 1.0 } else { 0.0 }, 0.0).0).collect()
        };
        for m in [0.0, 1.0] {
            let y = click(m);
            let (h, _) = BodySpec::between(&a, &b, m).impulse_response(SR, seed);
            for (n, &v) in y.iter().enumerate() {
                // a click of 1 on the left is 0.5 into the body
                let want = 0.5 * h.get(n).copied().unwrap_or(0.0);
                assert!((v - want).abs() < 1e-5, "morph {m} at {n}: {v} against {want}");
            }
        }
    }

    /// body_mix 0 is a wire.
    #[test]
    fn mix_zero_is_dry() {
        let mut body = Body::new(SR, BodyKind::Metal, None);
        body.set_mix(0.0);
        for n in 0..5000 {
            let v = ((n * 31) % 17) as f32 / 17.0 - 0.5;
            assert_eq!(body.process(v, -v), (v, -v));
        }
    }
}

