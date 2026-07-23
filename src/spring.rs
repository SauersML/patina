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

/// Snap values that have decayed past all audibility to exactly zero.
/// The spring loop decays exponentially forever, so without this every
/// delay slot, allpass and damping pole sits full of DENORMALS for tens
/// of seconds after each ring-out -- and denormal arithmetic costs 10-100x
/// a normal multiply on x86, so the unit's CPU load rises after the music
/// stops. 1e-20 is -400 dBFS: inaudible, and far above the denormal cliff.
#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

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
        let y = flush(self.a * x + self.x1 - self.a * self.y1);
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
        self.state = flush(self.state + self.a * (x - self.state));
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
        self.delay[self.idx] = flush(input + v * self.feedback);
        self.idx = (self.idx + 1) % self.delay.len();
        v
    }
}

pub struct SpringReverb {
    springs: [Spring; 2],
    drive_hp: OnePoleHp,
    drive_lp: OnePoleLp,
    send_hp: OnePoleHp,
    send_lp: OnePoleLp,
    // lets the springs ring out after a send burst even at zero wet:
    // a peak-hold follower on what the PICKUPS are actually putting out
    send_tail: f32,
    tail_k: f32,
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
            send_hp: OnePoleHp::new(120.0, sample_rate),
            send_lp: OnePoleLp::new(4200.0, sample_rate),
            send_tail: 0.0,
            // 250 ms release, expressed in seconds so it means the same
            // thing at 44.1, 48 and 96 kHz
            tail_k: (-1.0 / (0.25 * sample_rate)).exp(),
            wet: 0.0,
            smoothed: 0.0,
        }
    }

    /// The one panel control: reverberated/direct ratio. Decay is fixed.
    pub fn set_wet(&mut self, wet: f32) {
        self.wet = wet.clamp(0.0, 1.0);
    }

    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        self.process_with_send(left, right, 0.0, 0.0)
    }

    /// The springs themselves are linear (dispersion, fixed decay), so
    /// the wet control scales the DRIVEN global signal into the springs
    /// — identical to scaling their output, the legacy law — and a
    /// per-channel send drives its own input electronics into the same
    /// pair of springs, heard at unity.
    pub fn process_with_send(
        &mut self,
        left: f32,
        right: f32,
        send_left: f32,
        send_right: f32,
    ) -> (f32, f32) {
        // Springs are a feedback loop with nothing to trap a bad value: one
        // non-finite sample rings around them forever. Screening the input
        // is O(1) and turns a permanent kill into a one-sample dropout.
        let left = if left.is_finite() { left } else { 0.0 };
        let right = if right.is_finite() { right } else { 0.0 };
        let send_left = if send_left.is_finite() { send_left } else { 0.0 };
        let send_right = if send_right.is_finite() { send_right } else { 0.0 };

        self.smoothed += (self.wet - self.smoothed) * 0.001;
        let w = self.smoothed;
        let send = (send_left + send_right) * 0.5;
        if w < 0.002 && self.wet < 0.002 && send.abs() < 1e-6 && self.send_tail < 1e-5 {
            return (left, right);
        }

        // Mono spring drive with soft input electronics
        let mono = (left + right) * 0.5;
        let driven = {
            let x = (mono * 1.8).clamp(-3.0, 3.0);
            (x * (27.0 + x * x) / (27.0 + 9.0 * x * x)) / 1.8
        };
        let x = self.drive_lp.process(self.drive_hp.process(driven));
        let sdriven = {
            let x = (send * 1.8).clamp(-3.0, 3.0);
            (x * (27.0 + x * x) / (27.0 + 9.0 * x * x)) / 1.8
        };
        let xs = self.send_lp.process(self.send_hp.process(sdriven));

        let feed = x * w + xs;
        let s0 = self.springs[0].process(feed);
        let s1 = self.springs[1].process(feed);

        // The two pickup returns split unevenly into the stereo outputs
        let wet_l = (s0 * 0.85 + s1 * 0.35) * 1.7;
        let wet_r = (s1 * 0.85 + s0 * 0.35) * 1.7;

        // Hold the engage gate open on what the PICKUPS are doing, not on
        // what the send bus did. Keyed to the send level it went quiet
        // ~0.5 s after a burst while the springs were still ringing near
        // -39 dBFS, and the output stepped straight to zero: an audible
        // click on the tail of every send hit. The springs decay to true
        // zero (see `flush`), so this still closes.
        let ring = wet_l.abs().max(wet_r.abs()).max(send.abs());
        self.send_tail = ring.max(self.send_tail * self.tail_k);

        (
            left * (1.0 - w) + wet_l,
            right * (1.0 - w) + wet_r,
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

    /// Regression: the engage gate was held open by a follower on the SEND
    /// INPUT level (`send_tail.max(send) * 0.9995`), which decayed far
    /// faster than the springs themselves. After a 50 ms send burst at zero
    /// wet the gate closed at t = 0.519 s while the pickups were still
    /// putting out -39 dBFS, and the output stepped straight to hard zero:
    /// an audible click on the tail of every send hit.
    #[test]
    fn the_send_tail_is_not_cut_off_mid_ring() {
        let sr = 48000.0f32;
        let mut spring = SpringReverb::new(sr);
        spring.set_wet(0.0); // send-only: the gate is the one thing keeping it alive
        let burst = (0.05 * sr) as usize;
        let mut out = Vec::new();
        for n in 0..(3.0 * sr) as usize {
            let s = if n < burst {
                (TAU * 300.0 * n as f32 / sr).sin() * 0.8
            } else {
                0.0
            };
            let (l, _) = spring.process_with_send(0.0, 0.0, s, s);
            assert!(l.is_finite());
            out.push(l);
        }
        // Where does the output become permanently zero?
        let mut cut = out.len();
        while cut > 0 && out[cut - 1] == 0.0 {
            cut -= 1;
        }
        assert!(cut > burst, "the springs must ring at all");
        let level_at_cut = out[cut.saturating_sub(400)..cut]
            .iter()
            .fold(0.0f32, |a, &x| a.max(x.abs()));
        assert!(
            level_at_cut < 1e-4,
            "gate closed on a ring still at {:.1} dBFS (t = {:.3} s)",
            20.0 * level_at_cut.max(1e-12).log10(),
            cut as f32 / sr
        );
    }

    /// The same gate guards the panel knob, and it had the same fault:
    /// riding REVERB down to zero let `smoothed` cross the 0.002 threshold
    /// ~0.13 s later and the still-ringing springs were cut off there.
    #[test]
    fn closing_the_wet_knob_does_not_chop_the_ring() {
        let sr = 48000.0f32;
        let mut spring = SpringReverb::new(sr);
        spring.set_wet(1.0);
        for _ in 0..20000 {
            spring.process(0.0, 0.0);
        }
        for n in 0..(0.05 * sr) as usize {
            let x = (TAU * 300.0 * n as f32 / sr).sin() * 0.8;
            spring.process(x, x);
        }
        spring.set_wet(0.0); // knob slammed shut on a ringing tank
        let mut out = Vec::new();
        for _ in 0..(4.0 * sr) as usize {
            out.push(spring.process(0.0, 0.0).0);
        }
        let mut cut = out.len();
        while cut > 0 && out[cut - 1] == 0.0 {
            cut -= 1;
        }
        let level_at_cut = out[cut.saturating_sub(400)..cut]
            .iter()
            .fold(0.0f32, |a, &x| a.max(x.abs()));
        assert!(
            level_at_cut < 1e-4,
            "knob-off chopped a ring still at {:.1} dBFS (t = {:.3} s)",
            20.0 * level_at_cut.max(1e-12).log10(),
            cut as f32 / sr
        );
    }

    /// Regression: the spring loop decays exponentially forever, so once it
    /// passed the f32 denormal cliff every delay slot, allpass and damping
    /// pole held a denormal for tens of seconds. Denormal arithmetic is
    /// 10-100x slower on x86: the unit got MORE expensive after the note
    /// ended. Measured before the fix: ~457,000 denormal output samples in
    /// 20 s at 48 kHz.
    #[test]
    fn a_dead_ring_flushes_instead_of_going_denormal() {
        let sr = 48000.0f32;
        let mut spring = SpringReverb::new(sr);
        spring.set_wet(1.0);
        for _ in 0..20000 {
            spring.process(0.0, 0.0);
        }
        spring.process(1.0, 1.0);
        let mut denormals = 0usize;
        for _ in 0..(20.0 * sr) as usize {
            let (l, r) = spring.process(0.0, 0.0);
            for v in [l, r] {
                if v != 0.0 && v.abs() < f32::MIN_POSITIVE {
                    denormals += 1;
                }
            }
        }
        assert_eq!(denormals, 0, "denormal output samples");
        let (l, r) = spring.process(0.0, 0.0);
        assert_eq!((l, r), (0.0, 0.0), "springs should have come to rest");
    }

    /// One non-finite sample used to ring around the spring loop forever.
    #[test]
    fn a_nan_does_not_poison_the_springs() {
        let mut spring = SpringReverb::new(48000.0);
        spring.set_wet(0.5);
        spring.process(f32::NAN, f32::NAN);
        spring.process_with_send(0.0, 0.0, f32::INFINITY, f32::NAN);
        let mut energy = 0.0f32;
        for n in 0..48000 {
            let x = (TAU * 220.0 * n as f32 / 48000.0).sin() * 0.5;
            let (l, r) = spring.process(x, x);
            assert!(l.is_finite() && r.is_finite(), "poisoned at sample {n}");
            if n > 4800 {
                energy += l * l;
            }
        }
        assert!(energy > 1.0, "spring should be passing audio again: {energy}");
    }
}
