// 905-style spring reverberation. The Moog manual is specific about what
// this unit is and is not:
//
//   "utilizes a dual spring-type acoustic delay line to produce a
//    succession of decaying echoes"
//   "A single panel control determines the ratio" of reverberated to
//    direct signal — the control "does not alter the characteristic
//    decay time"
//   With a static input it behaves "like a formant filter, strongly
//    coloring the timbre"
//
// So: two dispersive spring paths with a FIXED mechanical decay, band-
// limited drive/pickup electronics, and wet/dry as the only control.
// Dispersion comes from a chain of first-order allpasses inside each
// spring's feedback loop — low frequencies travel slower down a spring,
// which smears each echo into the characteristic descending "boing".

use std::f32::consts::TAU;

/// First-order allpass, H(z) = (a + z^-1) / (1 + a z^-1).
struct Allpass {
    a: f32,
    x1: f32,
    y1: f32,
}

impl Allpass {
    fn new(a: f32) -> Self {
        Self { a, x1: 0.0, y1: 0.0 }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = self.a * x + self.x1 - self.a * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

struct OnePoleLp {
    state: f32,
    a: f32,
}

impl OnePoleLp {
    fn new(cutoff: f32, sample_rate: f32) -> Self {
        Self {
            state: 0.0,
            a: 1.0 - (-TAU * cutoff / sample_rate).exp(),
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        self.state += self.a * (x - self.state);
        self.state
    }
}

struct OnePoleHp {
    lp: OnePoleLp,
}

impl OnePoleHp {
    fn new(cutoff: f32, sample_rate: f32) -> Self {
        Self { lp: OnePoleLp::new(cutoff, sample_rate) }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        x - self.lp.process(x)
    }
}

/// One spring: a delay loop containing a dispersion chain and damping.
/// Feedback is FIXED — the mechanical decay of a physical spring.
struct Spring {
    delay: Vec<f32>,
    idx: usize,
    dispersion: Vec<Allpass>,
    damping: OnePoleLp,
    feedback: f32,
}

impl Spring {
    fn new(sample_rate: f32, delay_s: f32, stages: usize, ap_coef: f32, damp_hz: f32, feedback: f32) -> Self {
        let len = ((delay_s * sample_rate) as usize).max(8);
        Self {
            delay: vec![0.0; len],
            idx: 0,
            dispersion: (0..stages).map(|_| Allpass::new(ap_coef)).collect(),
            damping: OnePoleLp::new(damp_hz, sample_rate),
            feedback,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let mut v = self.delay[self.idx];
        for ap in &mut self.dispersion {
            v = ap.process(v);
        }
        v = self.damping.process(v);
        self.delay[self.idx] = input + v * self.feedback;
        self.idx = (self.idx + 1) % self.delay.len();
        v
    }
}

pub struct SpringReverb {
    springs: [Spring; 2],
    drive_hp: OnePoleHp,
    drive_lp: OnePoleLp,
    wet: f32,
    smoothed: f32,
}

impl SpringReverb {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            // Two mismatched springs: different transit times, dispersion
            // depths, damping, and (fixed) mechanical decay
            springs: [
                Spring::new(sample_rate, 0.037, 36, 0.58, 3400.0, 0.75),
                Spring::new(sample_rate, 0.047, 44, 0.62, 2800.0, 0.73),
            ],
            // Drive/pickup electronics: springs pass roughly 120 Hz - 4 kHz
            drive_hp: OnePoleHp::new(120.0, sample_rate),
            drive_lp: OnePoleLp::new(4200.0, sample_rate),
            wet: 0.0,
            smoothed: 0.0,
        }
    }

    /// The one panel control: reverberated/direct ratio. Decay is fixed.
    pub fn set_wet(&mut self, wet: f32) {
        self.wet = wet.clamp(0.0, 1.0);
    }

    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        self.smoothed += (self.wet - self.smoothed) * 0.001;
        let w = self.smoothed;
        if w < 0.002 && self.wet < 0.002 {
            return (left, right);
        }

        // Mono spring drive with soft input electronics
        let mono = (left + right) * 0.5;
        let driven = {
            let x = (mono * 1.8).clamp(-3.0, 3.0);
            (x * (27.0 + x * x) / (27.0 + 9.0 * x * x)) / 1.8
        };
        let x = self.drive_lp.process(self.drive_hp.process(driven));

        let s0 = self.springs[0].process(x);
        let s1 = self.springs[1].process(x);

        // The two pickup returns split unevenly into the stereo outputs
        let wet_l = (s0 * 0.85 + s1 * 0.35) * 1.7;
        let wet_r = (s1 * 0.85 + s0 * 0.35) * 1.7;

        (
            left * (1.0 - w) + wet_l * w,
            right * (1.0 - w) + wet_r * w,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypass_when_dry() {
        let mut spring = SpringReverb::new(44100.0);
        spring.set_wet(0.0);
        for n in 0..1000 {
            let x = (n as f32 * 0.01).sin() * 0.5;
            let (l, r) = spring.process(x, x);
            assert_eq!(l, x);
            assert_eq!(r, x);
        }
    }

    #[test]
    fn impulse_rings_then_decays() {
        let mut spring = SpringReverb::new(44100.0);
        spring.set_wet(1.0);
        // Settle the engage smoothing
        for _ in 0..8000 {
            spring.process(0.0, 0.0);
        }
        spring.process(1.0, 1.0);

        let mut early = 0.0f32; // 100-400 ms: echoes should be alive
        let mut late = 0.0f32; // last 200 ms of 4 s: decayed well down
        for n in 0..(4 * 44100) {
            let (l, _) = spring.process(0.0, 0.0);
            assert!(l.is_finite());
            if (4410..17640).contains(&n) {
                early = early.max(l.abs());
            } else if n > 4 * 44100 - 8820 {
                late = late.max(l.abs());
            }
        }
        assert!(early > 0.01, "spring should ring after the impulse, got {early}");
        assert!(
            late < early * 0.2,
            "spring should decay: early={early}, late={late}"
        );
    }
}
