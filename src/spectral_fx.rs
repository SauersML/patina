// src/spectral_fx.rs
//
// Spectral inserts: one short-time Fourier processor on a track's strip
// (`automate pad.spec_freeze`) or across the mix bus (`automate
// spec_tilt`), built on the vocoder's FFT (spectral.rs). Per frame (2048
// points, 75% overlap, Hann in and out; ~23 Hz bins at 48 kHz), in order:
//
//   in ─► FFT ─► MORPH ─► BLUR ─► FREEZE ─► SHIFT ─► GATE ─► TILT ─► MIX ─► iFFT ─► out
//
//   - morph   magnitudes crossfade toward another track's spectrum (the
//             track named by `spec_from=` on this track's line). Each bin
//             keeps the phase of whichever side gives it more, so 0 is
//             this track and 1 is the other one, exactly.
//   - blur    each bin's magnitude follows its input through a one-pole
//             with a time constant of `spec_blur` seconds: transients smear
//             and endings ring on. Where the smeared level outlasts the
//             input, the bin's phase keeps turning at the last frequency
//             measured there, so tails ring as tones, not as noise.
//   - freeze  0..1. The moment it leaves 0 the frame is caught; from then
//             on the caught magnitudes hold, each bin turning at the
//             frequency it had (a phase vocoder), so a frozen chord rings
//             steadily instead of buzzing at the frame rate. Between 0 and
//             1 the held frame and the live one crossfade; back at 0 it
//             lets go, and the next rise catches a new frame.
//   - shift   `spec_shift` Hz (added: harmonics go inharmonic, a ring-mod
//             shimmer) and `spec_pitch` semitones (multiplied: a pitch
//             shift that keeps time). Each spectral peak moves with the
//             bins around it, its lobe shape intact, and the bins keep
//             their phase offsets to the peak (Laroche and Dolson's
//             phase-locked vocoder), so a shifted tone stays one tone.
//   - gate    keeps the bins within `spec_gate` dB of the frame's loudest
//             (a 6 dB soft knee) and drops the rest: noise and room fall
//             away, the strongest partials stay. 0 = out.
//   - tilt    `spec_tilt` dB per octave about 1 kHz (+ brighter, - darker).
//   - mix     `spec_mix` blends the processed spectrum with the input's.
//
// Every stage at rest passes its bins untouched, so the whole insert at
// rest is the input, delayed by one frame: 2048 samples, 43 ms at 48 kHz.
// That delay is the insert's price, constant from the downbeat (the song
// player builds it before the first sample, never mid-song). Left and right
// share one complex transform (L + iR), unpacked by symmetry.

use crate::spectral::fft;
use std::f32::consts::{PI, TAU};

const N: usize = 2048;
const HOP: usize = N / 4;
const BINS: usize = N / 2 + 1;
/// Hann analysis and synthesis at a quarter-frame hop overlap-add to 1.5.
const OLA_GAIN: f32 = 1.5;

/// The insert's knobs, by song-file name (see `SpectralFx::set`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Knob {
    Freeze,
    Blur,
    Shift,
    Pitch,
    Morph,
    Gate,
    Tilt,
    Mix,
}

/// Wrap a phase into (-pi, pi].
#[inline]
fn princarg(x: f32) -> f32 {
    x - TAU * ((x + PI) / TAU).floor()
}

/// One channel's running state across frames.
#[derive(Clone)]
struct Chan {
    prev_phase: Vec<f32>,
    prev_phase_side: Vec<f32>,
    smooth: Vec<f32>,
    run_phase: Vec<f32>,
    hold_omega: Vec<f32>,
    frz_mag: Vec<f32>,
    frz_phase: Vec<f32>,
    frz_omega: Vec<f32>,
    syn_phase: Vec<f32>,
    /// The shift stage's peak list, kept so a frame never allocates on
    /// the audio thread
    peaks: Vec<usize>,
}

impl Chan {
    fn new() -> Self {
        let z = vec![0.0f32; BINS];
        Self {
            prev_phase: z.clone(),
            prev_phase_side: z.clone(),
            smooth: z.clone(),
            run_phase: z.clone(),
            hold_omega: z.clone(),
            frz_mag: z.clone(),
            frz_phase: z.clone(),
            frz_omega: z.clone(),
            syn_phase: z,
            peaks: Vec::with_capacity(BINS),
        }
    }
}

/// One frame's spectrum of one channel as the stages pass it along:
/// cartesian bins plus their polar form and instantaneous frequency
/// (radians per sample).
struct Frame {
    re: [f32; BINS],
    im: [f32; BINS],
    mag: [f32; BINS],
    ph: [f32; BINS],
    omega: [f32; BINS],
}

impl Frame {
    /// Rebuild bin k from its polar form.
    #[inline]
    fn set_polar(&mut self, k: usize, mag: f32, ph: f32) {
        self.mag[k] = mag;
        self.ph[k] = ph;
        let (s, c) = ph.sin_cos();
        self.re[k] = mag * c;
        self.im[k] = mag * s;
    }

    /// Scale bin k's magnitude, keeping its phase exactly.
    #[inline]
    fn scale(&mut self, k: usize, g: f32) {
        self.re[k] *= g;
        self.im[k] *= g;
        self.mag[k] *= g;
    }
}

#[derive(Clone)]
pub struct SpectralFx {
    sr: f32,
    window: Vec<f32>,
    inp: [Vec<f32>; 2],
    side: [Vec<f32>; 2],
    ola: [Vec<f32>; 2],
    in_pos: usize,
    out_pos: usize,
    fill: usize,
    ch: [Chan; 2],
    /// A morph source is wired in (the side input carries a track).
    has_side: bool,
    /// A frame is held (freeze has left 0 since it last rested).
    caught: bool,
    tilt_gain: Vec<f32>,
    // knobs
    freeze: f32,
    blur_s: f32,
    shift_hz: f32,
    pitch: f32,
    morph: f32,
    gate_db: f32,
    tilt: f32,
    mix: f32,
}

impl SpectralFx {
    pub fn new(sample_rate: f32) -> Self {
        let window = (0..N)
            .map(|i| 0.5 - 0.5 * (TAU * i as f32 / N as f32).cos())
            .collect();
        let z = vec![0.0f32; N];
        Self {
            sr: sample_rate,
            window,
            inp: [z.clone(), z.clone()],
            side: [z.clone(), z.clone()],
            ola: [z.clone(), z],
            in_pos: 0,
            out_pos: 0,
            fill: 0,
            ch: [Chan::new(), Chan::new()],
            has_side: false,
            caught: false,
            tilt_gain: vec![1.0; BINS],
            freeze: 0.0,
            blur_s: 0.0,
            shift_hz: 0.0,
            pitch: 0.0,
            morph: 0.0,
            gate_db: 0.0,
            tilt: 0.0,
            mix: 1.0,
        }
    }

    /// The insert's delay, samples.
    pub const LATENCY: usize = N;

    /// Wire a morph source into the side input (`spec_from=`).
    pub fn set_side(&mut self, on: bool) {
        self.has_side = on;
    }

    /// Turn one knob (values already clamped to the parameter's range).
    pub fn set(&mut self, knob: Knob, v: f32) {
        match knob {
            Knob::Freeze => self.freeze = v.clamp(0.0, 1.0),
            Knob::Blur => self.blur_s = v.max(0.0),
            Knob::Shift => self.shift_hz = v,
            Knob::Pitch => self.pitch = v,
            Knob::Morph => self.morph = v.clamp(0.0, 1.0),
            Knob::Gate => self.gate_db = v.max(0.0),
            Knob::Tilt => {
                if v != self.tilt {
                    self.tilt = v;
                    let hz = self.sr / N as f32;
                    for (k, g) in self.tilt_gain.iter_mut().enumerate() {
                        // octaves from 1 kHz, the DC bin read as 10 Hz;
                        // never more than 48 dB either way
                        let oct = ((k as f32 * hz).max(10.0) / 1000.0).log2();
                        *g = 10f32.powf((v * oct).clamp(-48.0, 48.0) / 20.0);
                    }
                }
            }
            Knob::Mix => self.mix = v.clamp(0.0, 1.0),
        }
    }

    /// One stereo sample in (plus the morph source's), one out, a frame
    /// later.
    #[inline]
    pub fn process(&mut self, l: f32, r: f32, side: (f32, f32)) -> (f32, f32) {
        self.inp[0][self.in_pos] = l;
        self.inp[1][self.in_pos] = r;
        self.side[0][self.in_pos] = side.0;
        self.side[1][self.in_pos] = side.1;
        self.in_pos = (self.in_pos + 1) % N;
        let y = (self.ola[0][self.out_pos], self.ola[1][self.out_pos]);
        self.ola[0][self.out_pos] = 0.0;
        self.ola[1][self.out_pos] = 0.0;
        self.out_pos = (self.out_pos + 1) % N;
        self.fill += 1;
        if self.fill >= HOP {
            self.fill = 0;
            self.frame();
        }
        y
    }

    /// The windowed frame of a stereo ring as one complex transform
    /// (L + iR), unpacked into each channel's half spectrum.
    fn analyze(&self, ring: &[Vec<f32>; 2]) -> [([f32; BINS], [f32; BINS]); 2] {
        let mut zr = [0.0f32; N];
        let mut zi = [0.0f32; N];
        for i in 0..N {
            let at = (self.in_pos + i) % N;
            zr[i] = ring[0][at] * self.window[i];
            zi[i] = ring[1][at] * self.window[i];
        }
        fft(&mut zr, &mut zi, false);
        let mut out = [([0.0f32; BINS], [0.0f32; BINS]); 2];
        for k in 0..BINS {
            let nk = (N - k) % N;
            // L = (Z[k] + conj Z[N-k]) / 2, R = (Z[k] - conj Z[N-k]) / 2i
            out[0].0[k] = 0.5 * (zr[k] + zr[nk]);
            out[0].1[k] = 0.5 * (zi[k] - zi[nk]);
            out[1].0[k] = 0.5 * (zi[k] + zi[nk]);
            out[1].1[k] = -0.5 * (zr[k] - zr[nk]);
        }
        out
    }

    fn frame(&mut self) {
        let x = self.analyze(&self.inp);
        let s = if self.has_side {
            Some(self.analyze(&self.side))
        } else {
            None
        };
        let mut y = [([0.0f32; BINS], [0.0f32; BINS]); 2];
        for c in 0..2 {
            let side = s.as_ref().map(|s| (&s[c].0, &s[c].1));
            y[c] = self.channel(c, (&x[c].0, &x[c].1), side);
        }
        if self.freeze > 0.0 {
            self.caught = true;
        } else {
            self.caught = false;
        }
        // Pack Y = YL + i YR over the full circle (each half Hermitian),
        // invert: the real part is left, the imaginary part right
        let mut zr = [0.0f32; N];
        let mut zi = [0.0f32; N];
        for k in 0..N {
            let (src, conj) = if k < BINS { (k, 1.0) } else { (N - k, -1.0) };
            let (lr, li) = (y[0].0[src], conj * y[0].1[src]);
            let (rr, ri) = (y[1].0[src], conj * y[1].1[src]);
            zr[k] = lr - ri;
            zi[k] = li + rr;
        }
        fft(&mut zr, &mut zi, true);
        for i in 0..N {
            let at = (self.out_pos + i) % N;
            let w = self.window[i] / OLA_GAIN;
            self.ola[0][at] += zr[i] * w;
            self.ola[1][at] += zi[i] * w;
        }
    }

    /// One channel's frame through every stage.
    fn channel(
        &mut self,
        c: usize,
        x: (&[f32; BINS], &[f32; BINS]),
        side: Option<(&[f32; BINS], &[f32; BINS])>,
    ) -> ([f32; BINS], [f32; BINS]) {
        let hop = HOP as f32;
        let bin_w = TAU / N as f32;
        let ch = &mut self.ch[c];
        let mut f = Frame {
            re: *x.0,
            im: *x.1,
            mag: [0.0; BINS],
            ph: [0.0; BINS],
            omega: [0.0; BINS],
        };
        for k in 0..BINS {
            f.mag[k] = f.re[k].hypot(f.im[k]);
            f.ph[k] = f.im[k].atan2(f.re[k]);
            let expect = bin_w * k as f32 * hop;
            f.omega[k] = bin_w * k as f32 + princarg(f.ph[k] - ch.prev_phase[k] - expect) / hop;
            ch.prev_phase[k] = f.ph[k];
        }

        // MORPH: magnitudes crossfade; each bin takes the louder side's phase
        if let Some((sr_, si_)) = side {
            let m = self.morph;
            for k in 0..BINS {
                let mb = sr_[k].hypot(si_[k]);
                let pb = si_[k].atan2(sr_[k]);
                let expect = bin_w * k as f32 * hop;
                let wb_k = bin_w * k as f32 + princarg(pb - ch.prev_phase_side[k] - expect) / hop;
                ch.prev_phase_side[k] = pb;
                if m <= 0.0 {
                    continue;
                }
                let (wa, wb) = ((1.0 - m) * f.mag[k], m * mb);
                if wb > wa {
                    f.set_polar(k, wa + wb, pb);
                    f.omega[k] = wb_k;
                } else if f.mag[k] > 0.0 {
                    f.scale(k, (wa + wb) / f.mag[k]);
                }
            }
        } else if self.morph > 0.0 {
            // no source wired: the other side is silence
            for k in 0..BINS {
                f.scale(k, 1.0 - self.morph);
            }
        }

        // BLUR: each bin's level smeared over spec_blur seconds; where the
        // smear outlasts the input, the phase turns on at the last
        // frequency the bin had
        if self.blur_s > 0.0 {
            let a = (-hop / (self.blur_s * self.sr)).exp();
            for k in 0..BINS {
                ch.smooth[k] = a * ch.smooth[k] + (1.0 - a) * f.mag[k];
                if f.mag[k] >= 0.5 * ch.smooth[k] {
                    ch.run_phase[k] = f.ph[k];
                    ch.hold_omega[k] = f.omega[k];
                    if f.mag[k] > 0.0 {
                        f.scale(k, ch.smooth[k] / f.mag[k]);
                    }
                } else {
                    ch.run_phase[k] = princarg(ch.run_phase[k] + ch.hold_omega[k] * hop);
                    f.set_polar(k, ch.smooth[k], ch.run_phase[k]);
                    f.omega[k] = ch.hold_omega[k];
                }
            }
        } else {
            // at rest it tracks, so engaging starts from the present
            ch.smooth.copy_from_slice(&f.mag);
            ch.run_phase.copy_from_slice(&f.ph);
            ch.hold_omega.copy_from_slice(&f.omega);
        }

        // FREEZE: the caught frame, each bin turning at its own frequency,
        // crossfaded with the live one
        if self.freeze > 0.0 {
            let fz = self.freeze;
            for k in 0..BINS {
                if !self.caught {
                    ch.frz_mag[k] = f.mag[k];
                    ch.frz_phase[k] = f.ph[k];
                    ch.frz_omega[k] = f.omega[k];
                } else {
                    ch.frz_phase[k] = princarg(ch.frz_phase[k] + ch.frz_omega[k] * hop);
                }
                let (wl, wf) = ((1.0 - fz) * f.mag[k], fz * ch.frz_mag[k]);
                if wf > wl {
                    f.set_polar(k, wl + wf, ch.frz_phase[k]);
                    f.omega[k] = ch.frz_omega[k];
                } else if f.mag[k] > 0.0 {
                    f.scale(k, (wl + wf) / f.mag[k]);
                }
            }
        }

        // SHIFT: move each peak's region of bins to the peak's new
        // frequency, the region's phases locked to the peak's
        let ratio = (self.pitch / 12.0).exp2();
        if ratio != 1.0 || self.shift_hz != 0.0 {
            let hz = self.sr / N as f32;
            let peaks = &mut ch.peaks;
            peaks.clear();
            for k in 2..BINS - 2 {
                let m = f.mag[k];
                if m > 1e-9
                    && m > f.mag[k - 1]
                    && m >= f.mag[k + 1]
                    && m > f.mag[k - 2]
                    && m >= f.mag[k + 2]
                {
                    peaks.push(k);
                }
            }
            let mut om = [0.0f32; BINS];
            let mut op = [0.0f32; BINS];
            let mut written = [false; BINS];
            for (i, &p) in peaks.iter().enumerate() {
                // the region: from the trough after the last peak to the
                // trough before the next
                let lo = if i == 0 {
                    0
                } else {
                    let a = peaks[i - 1];
                    (a..p).min_by(|&u, &v| f.mag[u].total_cmp(&f.mag[v])).unwrap_or(p)
                };
                let hi = if i + 1 == peaks.len() {
                    BINS
                } else {
                    let b = peaks[i + 1];
                    (p..b).min_by(|&u, &v| f.mag[u].total_cmp(&f.mag[v])).unwrap_or(p)
                };
                let fp = f.omega[p] / TAU * self.sr;
                let ft = fp * ratio + self.shift_hz;
                if ft <= 0.0 || ft >= 0.5 * self.sr {
                    continue;
                }
                // the lobe moves by the peak's true shift, to the bin
                let delta = ((ft - fp) / hz).round() as isize;
                let target = p as isize + delta;
                if target < 1 || target >= BINS as isize - 1 {
                    continue;
                }
                let syn = princarg(ch.syn_phase[target as usize] + TAU * ft / self.sr * hop);
                for k in lo..hi {
                    let j = k as isize + delta;
                    if j < 1 || j >= BINS as isize - 1 {
                        continue;
                    }
                    let j = j as usize;
                    om[j] += f.mag[k];
                    op[j] = syn + f.ph[k] - f.ph[p];
                    written[j] = true;
                }
            }
            for k in 0..BINS {
                let ph = if written[k] { princarg(op[k]) } else { 0.0 };
                f.set_polar(k, om[k], ph);
                ch.syn_phase[k] = ph;
            }
        } else {
            ch.syn_phase.copy_from_slice(&f.ph);
        }

        // GATE: keep the bins within spec_gate dB of the loudest, soft knee
        if self.gate_db > 0.0 {
            let peak = f.mag.iter().fold(0.0f32, |a, &v| a.max(v));
            let thr = peak * 10f32.powf(-self.gate_db / 20.0);
            for k in 0..BINS {
                // 3 dB either side of the threshold, a smoothstep in dB
                let db = 20.0 * (f.mag[k] / thr.max(1e-30)).max(1e-12).log10();
                let t = ((db + 3.0) / 6.0).clamp(0.0, 1.0);
                f.scale(k, t * t * (3.0 - 2.0 * t));
            }
        }

        // TILT
        if self.tilt != 0.0 {
            for k in 0..BINS {
                f.scale(k, self.tilt_gain[k]);
            }
        }

        // MIX, and a real signal's DC and Nyquist bins are real
        let mut re = f.re;
        let mut im = f.im;
        if self.mix < 1.0 {
            let (w, d) = (self.mix, 1.0 - self.mix);
            for k in 0..BINS {
                re[k] = w * re[k] + d * x.0[k];
                im[k] = w * im[k] + d * x.1[k];
            }
        }
        im[0] = 0.0;
        im[BINS - 1] = 0.0;
        (re, im)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    fn sine(f: f32, n: usize) -> f32 {
        (TAU * f * n as f32 / SR).sin()
    }

    /// A sinusoid's amplitude near `f` (a unit sine reads 1): Hann-windowed
    /// Goertzel over 4096-sample blocks, averaged.
    fn band(x: &[f32], f: f32) -> f32 {
        let n = 4096;
        let blocks = x.len() / n;
        let mut sum = 0.0f64;
        for b in 0..blocks {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for i in 0..n {
                let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
                let ph = std::f64::consts::TAU * f as f64 * i as f64 / SR as f64;
                re += x[b * n + i] as f64 * w * ph.cos();
                im += x[b * n + i] as f64 * w * ph.sin();
            }
            sum += (re * re + im * im).sqrt() / (0.25 * n as f64);
        }
        (sum / blocks as f64) as f32
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len() as f64).sqrt() as f32
    }

    fn run(fx: &mut SpectralFx, n: usize, input: impl Fn(usize) -> (f32, f32)) -> Vec<(f32, f32)> {
        (0..n).map(|i| {
            let (l, r) = input(i);
            fx.process(l, r, (0.0, 0.0))
        })
        .collect()
    }

    /// Every stage at rest: the output is the input, a frame late.
    #[test]
    fn at_rest_it_is_the_input_one_frame_late() {
        let mut fx = SpectralFx::new(SR);
        let x = |n: usize| {
            let l = 0.6 * sine(311.0, n) + 0.3 * sine(2717.0, n) * (n as f32 * 1e-4).cos();
            let r = ((n * 7919) % 1009) as f32 / 1009.0 - 0.5;
            (l, r)
        };
        let y = run(&mut fx, 24000, x);
        let mut worst = 0.0f32;
        for n in 2 * N..24000 {
            let (l, r) = x(n - SpectralFx::LATENCY);
            worst = worst.max((y[n].0 - l).abs()).max((y[n].1 - r).abs());
        }
        assert!(worst < 2e-5, "at rest the insert strays {worst} from the delayed input");
    }

    /// Frozen frames stay constant: a 440 Hz tone caught at 0.5 s keeps
    /// ringing at a steady level after the input jumps to noise, and none
    /// of the noise gets through.
    #[test]
    fn a_frozen_frame_holds_steady() {
        let mut fx = SpectralFx::new(SR);
        let mut noise = 0x2468u32;
        let mut y = Vec::new();
        for n in 0..96000 {
            if n == 24000 {
                fx.set(Knob::Freeze, 1.0);
            }
            let v = if n < 26000 {
                0.5 * sine(440.0, n)
            } else {
                crate::rng::bipolar(&mut noise)
            };
            y.push(fx.process(v, v, (0.0, 0.0)).0);
        }
        let held = &y[48000..96000];
        // steady: 2048-sample windows within 0.25 dB of one another
        let levels: Vec<f32> = held.chunks(2048).map(|c| 20.0 * rms(c).log10()).collect();
        let (lo, hi) = levels.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(hi - lo < 0.25, "the frozen level wanders {:.3} dB", hi - lo);
        let (tone, other) = (band(held, 440.0), band(held, 3000.0));
        assert!((tone - 0.5).abs() < 0.05, "the frozen 440 Hz reads {tone:.3}, caught at 0.5");
        assert!(other < 1e-3, "noise reached a frozen frame: {other:.5}");
    }

    /// Blur smears each bin over spec_blur seconds: after a tone stops,
    /// it rings on and decays by 1/e every time constant (8.7 dB a second
    /// at spec_blur 1), as a tone at its own pitch.
    #[test]
    fn blur_rings_on_with_its_time_constant() {
        let mut fx = SpectralFx::new(SR);
        fx.set(Knob::Blur, 1.0);
        let y: Vec<f32> = run(&mut fx, 4 * 48000, |n| {
            let v = if n < 48000 { 0.5 * sine(660.0, n) } else { 0.0 };
            (v, v)
        })
        .into_iter()
        .map(|s| s.0)
        .collect();
        // the input stops at 1 s; it reaches the output a frame later
        let at = |t: f32| band(&y[(t * SR) as usize..(t * SR) as usize + 8192], 660.0);
        let (a, b) = (at(1.5), at(2.5));
        let per_s = 20.0 * (a / b).log10();
        assert!((per_s - 8.69).abs() < 1.0, "the smear decays {per_s:.2} dB/s, want 8.69");
        assert!(band(&y[(1.5 * SR) as usize..(2.5 * SR) as usize], 1500.0) < 0.05 * a);
    }

    /// spec_shift adds Hz (1 kHz + 300 = 1.3 kHz), spec_pitch multiplies
    /// (440 Hz + 12 semitones = 880 Hz), and neither leaves the original.
    #[test]
    fn shift_adds_hertz_and_pitch_multiplies() {
        for (knob, v, from, to) in [(Knob::Shift, 300.0, 1000.0, 1300.0), (Knob::Pitch, 12.0, 440.0, 880.0)] {
            let mut fx = SpectralFx::new(SR);
            fx.set(knob, v);
            let y: Vec<f32> = run(&mut fx, 72000, |n| (0.5 * sine(from, n), 0.0))
                .into_iter()
                .map(|s| s.0)
                .collect();
            let tail = &y[24000..];
            let (moved, left) = (band(tail, to), band(tail, from));
            assert!((moved - 0.5).abs() < 0.08, "{knob:?} {v}: {to} Hz reads {moved:.3}");
            assert!(left < 0.02, "{knob:?} {v}: {from} Hz left at {left:.3}");
        }
    }

    /// The gate keeps the bins near the loudest: a tone over a noise floor
    /// 40 dB down keeps the tone and loses the floor.
    #[test]
    fn the_gate_keeps_the_strong_bins() {
        let input = |n: usize, s: &mut u32| 0.5 * sine(1000.0, n) + 0.005 * crate::rng::bipolar(s);
        let mut s1 = 7u32;
        let mut plain = SpectralFx::new(SR);
        let a: Vec<f32> = (0..72000).map(|n| { let v = input(n, &mut s1); plain.process(v, v, (0.0, 0.0)).0 }).collect();
        let mut s2 = 7u32;
        let mut gated = SpectralFx::new(SR);
        gated.set(Knob::Gate, 20.0);
        let b: Vec<f32> = (0..72000).map(|n| { let v = input(n, &mut s2); gated.process(v, v, (0.0, 0.0)).0 }).collect();
        // the floor, read in bands far from the tone
        let floor = |x: &[f32]| {
            [4000.0, 7000.0, 11000.0, 15000.0].iter().map(|&f| band(x, f)).sum::<f32>() + 1e-9
        };
        let (fa, fb) = (floor(&a[24000..]), floor(&b[24000..]));
        assert!(20.0 * (fa / fb).log10() > 25.0, "the floor fell {:.1} dB", 20.0 * (fa / fb).log10());
        let (ta, tb) = (band(&a[24000..], 1000.0), band(&b[24000..], 1000.0));
        assert!(20.0 * (tb / ta).log10() > -0.5, "the tone lost {:.2} dB", 20.0 * (tb / ta).log10());
    }

    /// Tilt is dB per octave about 1 kHz: -6 dB/oct puts 500 Hz up 6 dB
    /// and 2 kHz down 6, and leaves 1 kHz alone.
    #[test]
    fn tilt_turns_about_one_kilohertz() {
        let mut fx = SpectralFx::new(SR);
        fx.set(Knob::Tilt, -6.0);
        let y: Vec<f32> = run(&mut fx, 72000, |n| {
            let v = 0.2 * (sine(500.0, n) + sine(1000.0, n) + sine(2000.0, n));
            (v, v)
        })
        .into_iter()
        .map(|s| s.0)
        .collect();
        let t = &y[24000..];
        for (f, want) in [(500.0, 6.0), (1000.0, 0.0), (2000.0, -6.0)] {
            let got = 20.0 * (band(t, f) / 0.2).log10();
            assert!((got - want).abs() < 0.3, "{f} Hz: {got:+.2} dB, want {want:+.1}");
        }
    }

    /// Morph crossfades magnitudes toward the side input: 0 is this
    /// track, 1 the other, exactly, and halfway both at half level.
    #[test]
    fn morph_crossfades_to_the_other_track() {
        let go = |m: f32| -> Vec<f32> {
            let mut fx = SpectralFx::new(SR);
            fx.set_side(true);
            fx.set(Knob::Morph, m);
            (0..72000)
                .map(|n| {
                    let a = 0.5 * sine(440.0, n);
                    let b = 0.5 * sine(1250.0, n);
                    fx.process(a, a, (b, b)).0
                })
                .skip(24000)
                .collect()
        };
        for (m, a_want, b_want) in [(0.0, 0.5, 0.0), (0.5, 0.25, 0.25), (1.0, 0.0, 0.5)] {
            let y = go(m);
            let (a, b) = (band(&y, 440.0), band(&y, 1250.0));
            assert!((a - a_want).abs() < 0.02, "morph {m}: 440 Hz {a:.3}, want {a_want}");
            assert!((b - b_want).abs() < 0.02, "morph {m}: 1250 Hz {b:.3}, want {b_want}");
        }
    }
}
