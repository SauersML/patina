// src/limiter.rs
//
// The master bus's peak control, the last thing before the arrangement
// trim: a soft-knee clipper (`clip`) and a look-ahead brickwall limiter
// (`limit`). Both OUT by default, and out means bit-identical.
//
//   clip    dB of drive into a clipper whose rail is the ceiling. Linear
//           below 70% of the rail, a tanh knee into it above, antialiased
//           by first-order ADAA. It shaves the fastest transients (a
//           kick's first millisecond) that a limiter would otherwise
//           duck the whole mix for.
//   limit   the threshold, dBFS: the mix is raised by -threshold and held
//           under the ceiling (`limit_ceiling`, default -1 dBFS) with
//           1.5 ms of look-ahead, so a peak's gain reduction is already
//           in place when it arrives — no overshoot, no clipped edge —
//           and released over `limit_release` seconds.
//
// The gain computer is the standard min-hold + box-average construction:
// the per-sample gain each input needs to sit under the ceiling is
// min-held over the look-ahead window D, release-limited, then averaged
// over D. Every gain inside that average is at most the need of the sample
// leaving the delay line, so the output can never pass the ceiling, and
// the average makes the attack a D-sample ramp rather than a step.
//
// Engaged, this stage replaces the engine's tanh soft limit: the DC
// blocker runs before it and only a safety clamp at 0 dBFS after it
// (voice_manager), so the ceiling is the bounce's true peak.

use std::f64::consts::LN_2;

const LOOKAHEAD_S: f32 = 0.0015;
/// Where the clipper's knee starts, as a fraction of its rail.
const KNEE: f64 = 0.7;

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn ln_cosh(x: f64) -> f64 {
    let a = x.abs();
    a + (-2.0 * a).exp().ln_1p() - LN_2
}

/// The soft-knee clip curve, rail at 1.
fn knee_f(u: f64) -> f64 {
    let a = u.abs();
    if a <= KNEE {
        u
    } else {
        u.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh())
    }
}

fn knee_big_f(u: f64) -> f64 {
    let a = u.abs();
    if a <= KNEE {
        0.5 * u * u
    } else {
        let w = 1.0 - KNEE;
        0.5 * KNEE * KNEE + KNEE * (a - KNEE) + w * w * ln_cosh((a - KNEE) / w)
    }
}

pub struct MasterPeak {
    sample_rate: f32,
    smooth_k: f32,
    /// Ceiling, dBFS, and its smoothed linear value.
    ceiling_db: f32,
    ceiling: f32,
    // --- clipper -------------------------------------------------------
    clip_db: f32,
    clip_gain: f32,
    clip_on: bool,
    clip_x1: [f64; 2],
    clip_f1: [f64; 2],
    // --- limiter -------------------------------------------------------
    threshold_db: f32,
    in_gain: f32,
    /// Once in, the limiter stays in the path (at threshold 0 it only
    /// catches overs): pulling it out would drop its look-ahead delay and
    /// jump the whole mix by 1.5 ms.
    limit_on: bool,
    release_k: f32,
    /// The look-ahead line, D samples.
    delay: Vec<(f32, f32)>,
    dpos: usize,
    /// The gain each input needs, and the release-limited gains, over the
    /// last D + 1 samples: the window must reach back to the sample
    /// leaving the line, D samples old.
    need: Vec<f32>,
    held: Vec<f32>,
    held_sum: f64,
    wpos: usize,
    gain: f32,
    /// The gain applied to the last sample, for meters and tests.
    last_gain: f32,
    /// Has processed a sample. Settings made before that (a song's
    /// downbeat) land where they are set instead of gliding there from
    /// the defaults: a -2 dBFS ceiling gliding down from -1 let the first
    /// kick through 0.5 dB over it.
    started: bool,
}

impl MasterPeak {
    pub fn new(sample_rate: f32) -> Self {
        let d = ((LOOKAHEAD_S * sample_rate).round() as usize).max(1);
        MasterPeak {
            sample_rate,
            smooth_k: crate::smoothing::approach(crate::smoothing::KNOB_SMOOTH_S, sample_rate),
            ceiling_db: -1.0,
            ceiling: db_to_gain(-1.0),
            clip_db: 0.0,
            clip_gain: 1.0,
            clip_on: false,
            clip_x1: [0.0; 2],
            clip_f1: [0.0; 2],
            threshold_db: 0.0,
            in_gain: 1.0,
            limit_on: false,
            release_k: crate::smoothing::approach(0.08, sample_rate),
            delay: vec![(0.0, 0.0); d],
            dpos: 0,
            need: vec![1.0; d + 1],
            held: vec![1.0; d + 1],
            held_sum: (d + 1) as f64,
            wpos: 0,
            gain: 1.0,
            last_gain: 1.0,
            started: false,
        }
    }

    /// Clipper drive, dB (0 = out).
    pub fn set_clip(&mut self, db: f32) {
        if !db.is_finite() {
            return;
        }
        self.clip_db = db.max(0.0);
        if self.clip_db > 0.0 && !self.clip_on {
            self.clip_on = true;
            // the first quotient must span one real step
            self.clip_x1 = [0.0; 2];
            self.clip_f1 = [0.0; 2];
        }
    }

    /// Put the limiter in the path at its current threshold (0: it only
    /// catches overs), so a threshold set later lands without dropping
    /// the look-ahead line's 1.5 ms of silence into the mix.
    pub fn arm(&mut self) {
        self.limit_on = true;
    }

    /// Limiter threshold, dBFS (0 = out until first set below 0).
    pub fn set_threshold(&mut self, db: f32) {
        if !db.is_finite() {
            return;
        }
        self.threshold_db = db.min(0.0);
        if self.threshold_db < 0.0 {
            self.limit_on = true;
        }
    }

    pub fn set_ceiling(&mut self, db: f32) {
        if db.is_finite() {
            self.ceiling_db = db.min(0.0);
        }
    }

    pub fn set_release(&mut self, seconds: f32) {
        if seconds.is_finite() {
            self.release_k = crate::smoothing::approach(seconds.max(1e-4), self.sample_rate);
        }
    }

    pub fn is_active(&self) -> bool {
        self.clip_on || self.limit_on
    }

    /// The gain the limiter applied to the last sample (1 = none).
    pub fn gain_reduction(&self) -> f32 {
        self.last_gain
    }

    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if !self.is_active() {
            return (l, r);
        }
        if !self.started {
            self.started = true;
            self.ceiling = db_to_gain(self.ceiling_db);
            self.clip_gain = db_to_gain(self.clip_db);
            self.in_gain = db_to_gain(-self.threshold_db);
        }
        let k = self.smooth_k;
        self.ceiling += (db_to_gain(self.ceiling_db) - self.ceiling) * k;
        let c = self.ceiling;
        let (mut l, mut r) = (l, r);

        if self.clip_on {
            self.clip_gain += (db_to_gain(self.clip_db) - self.clip_gain) * k;
            let g = (self.clip_gain / c) as f64;
            let mut out = [0.0f32; 2];
            for (ch, x) in [l, r].into_iter().enumerate() {
                let u = x as f64 * g;
                let f0 = knee_big_f(u);
                let dx = u - self.clip_x1[ch];
                let y = if dx.abs() < 1e-6 {
                    knee_f(0.5 * (u + self.clip_x1[ch]))
                } else {
                    (f0 - self.clip_f1[ch]) / dx
                };
                self.clip_x1[ch] = u;
                self.clip_f1[ch] = f0;
                out[ch] = (y * c as f64) as f32;
            }
            (l, r) = (out[0], out[1]);
        }

        if self.limit_on {
            self.in_gain += (db_to_gain(-self.threshold_db) - self.in_gain) * k;
            let (xl, xr) = (l * self.in_gain, r * self.in_gain);
            let w = self.need.len();
            let peak = xl.abs().max(xr.abs());
            let need = if peak > c { c / peak } else { 1.0 };
            self.need[self.wpos] = need;
            let min_need = self.need.iter().fold(1.0f32, |m, &v| m.min(v));
            // release: recover toward unity, never above what is needed
            self.gain = min_need.min(self.gain + (1.0 - self.gain) * self.release_k);
            self.held_sum += self.gain as f64 - self.held[self.wpos] as f64;
            self.held[self.wpos] = self.gain;
            let smooth = (self.held_sum / w as f64) as f32;
            self.wpos = (self.wpos + 1) % w;
            if self.wpos == 0 {
                // re-sum once per window so the running sum cannot drift
                self.held_sum = self.held.iter().map(|&v| v as f64).sum();
            }
            // the sample leaving the look-ahead line is D samples old
            let (dl, dr) = std::mem::replace(&mut self.delay[self.dpos], (xl, xr));
            self.dpos = (self.dpos + 1) % self.delay.len();
            self.last_gain = smooth;
            // the construction guarantees the ceiling; the clamp only
            // catches the last bit of float rounding
            l = (dl * smooth).clamp(-c, c);
            r = (dr * smooth).clamp(-c, c);
        }
        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    #[test]
    fn knee_antiderivative_matches() {
        for i in -400..400 {
            let x = i as f64 * 0.01 + 0.0037;
            let h = 1e-5;
            let d = (knee_big_f(x + h) - knee_big_f(x - h)) / (2.0 * h);
            assert!((d - knee_f(x)).abs() < 1e-4, "at {x}");
        }
    }

    #[test]
    fn out_is_bit_identical() {
        let mut m = MasterPeak::new(SR);
        m.set_ceiling(-3.0);
        m.set_release(0.2);
        for i in 0..1000 {
            let x = (i as f32 * 0.37).sin() * 2.0;
            assert_eq!(m.process(x, -x), (x, -x));
        }
    }

    /// A program with spiky peaks over a dense floor: the limiter holds
    /// every sample under the ceiling and lowers the crest factor.
    #[test]
    fn the_limiter_holds_the_ceiling_and_tames_the_crest() {
        let mut m = MasterPeak::new(SR);
        m.set_threshold(-12.0);
        let ceiling = db_to_gain(-1.0);
        let mut state = 1u32;
        let mut peak = 0f32;
        let (mut e_in, mut e_out, mut p_in) = (0f64, 0f64, 0f32);
        for i in 0..(SR as usize * 2) {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (state >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
            let hit = if i % 12000 < 200 { 6.0 * (-((i % 12000) as f32) / 40.0).exp() } else { 0.0 };
            let x = 0.1 * noise + hit * 0.2;
            let (y, _) = m.process(x, x);
            if i > 4800 {
                peak = peak.max(y.abs());
                p_in = p_in.max(x.abs());
                e_in += (x * x) as f64;
                e_out += (y * y) as f64;
            }
        }
        assert!(peak <= ceiling + 1e-6, "peak {peak} over the ceiling {ceiling}");
        let n = (SR as usize * 2 - 4801) as f64;
        let crest_in = 20.0 * (p_in as f64 / (e_in / n).sqrt()).log10();
        let crest_out = 20.0 * (peak as f64 / (e_out / n).sqrt()).log10();
        assert!(crest_out < crest_in - 6.0, "crest {crest_in:.1} -> {crest_out:.1}");
    }

    #[test]
    fn the_clipper_never_passes_its_rail() {
        let mut m = MasterPeak::new(SR);
        m.set_clip(18.0);
        let rail = db_to_gain(-1.0);
        for i in 0..48000 {
            let x = (i as f32 * 0.05).sin() * 0.9;
            let (y, _) = m.process(x, x);
            assert!(y.abs() <= rail + 1e-5, "{y}");
        }
    }
}
