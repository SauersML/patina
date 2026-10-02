// The ECHO knob — a stereo delay for the space between notes.
//
// Ambient music is built as much from repeats as from reverb: Eno's
// Discreet Music is two tape machines and a delay; Fripp's loops are the
// same machine. A reverb fills the room; an echo answers the note, in
// time, so the line stays legible while it multiplies. This unit is that
// second machine, voiced for clarity rather than vintage grit:
//
//   in (L, R) -> [wet insert + per-track send] -> stereo line (up to 2.5 s)
//      -> loop: 2-pole lowpass (TONE) and a fixed 1-pole high-pass, so each
//         repeat is a little darker AND a little thinner than the last —
//         the lows never pile up under the music, the highs never sizzle
//      -> soft saturation inside the loop: unity-gain for small signals,
//         bounded for large ones, so even full feedback cannot run away
//      -> PING-PONG crossfeed: 0 = each side repeats on itself, 1 = the
//         mono input starts on the left and every repeat changes sides
//   out = in + the filtered taps (the first repeat is already filtered,
//         so the echo sits behind the dry note instead of doubling it)
//
// TIME glides to a new setting over ~120 ms (the read head moving, like a
// varispeed tape echo) instead of jumping, so automating it never clicks.
// The read is 4-point Hermite, so a gliding head stays clean.
//
// No modulation in the loop on purpose: wow belongs to the tape stage,
// and an echo that drifts out of tune with the dry note blurs the line.

use crate::smoothing::{approach, one_pole};

/// Longest delay the line holds, seconds (the knob's ceiling).
pub const MAX_TIME_S: f32 = 2.5;
/// The loop's fixed high-pass corner, Hz: below this every repeat loses a
/// little more, so 20 repeats of a bass note are not 20 bass notes.
const LOOP_HP_HZ: f32 = 120.0;
/// How long the read head takes to settle on a new TIME (1 - 1/e), s.
const TIME_GLIDE_S: f32 = 0.12;

/// Snap values that have decayed past all audibility to exactly zero, so a
/// long feedback tail never fills the loop with denormals (see reverb.rs).
#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

/// Unity-slope soft limiter for the feedback path: x for small x, bounded
/// at +-2 so the loop cannot grow without limit.
#[inline]
fn soft(x: f32) -> f32 {
    2.0 * (0.5 * x).tanh()
}

struct Line {
    buf: Vec<f32>,
    write: usize,
}

impl Line {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(8)],
            write: 0,
        }
    }

    #[inline]
    fn push(&mut self, x: f32) {
        self.buf[self.write] = flush(x);
        self.write = (self.write + 1) % self.buf.len();
    }

    /// The sample `delay` samples behind the most recent write, read with
    /// 4-point Hermite interpolation. `delay` >= 1.
    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let len = self.buf.len();
        let delay = delay.clamp(1.0, (len - 4) as f32);
        let d = delay.floor();
        let t = delay - d;
        let d = d as usize;
        // index of the sample exactly `k` behind the last write
        let at = |k: usize| self.buf[(len + self.write - 1 - k) % len];
        let (xm1, x0, x1, x2) = (at(d - 1), at(d), at(d + 1), at(d + 2));
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        ((c3 * t + c2) * t + c1) * t + x0
    }
}

/// One side's loop filters: two cascaded one-pole lowpasses (TONE) and the
/// fixed high-pass.
struct LoopFilter {
    lp1: f32,
    lp2: f32,
    hp_state: f32,
}

impl LoopFilter {
    fn new() -> Self {
        Self {
            lp1: 0.0,
            lp2: 0.0,
            hp_state: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32, k_lp: f32, k_hp: f32) -> f32 {
        self.lp1 = flush(self.lp1 + k_lp * (x - self.lp1));
        self.lp2 = flush(self.lp2 + k_lp * (self.lp1 - self.lp2));
        self.hp_state = flush(self.hp_state + k_hp * (self.lp2 - self.hp_state));
        self.lp2 - self.hp_state
    }
}

pub struct Echo {
    sample_rate: f32,
    lines: [Line; 2],
    filters: [LoopFilter; 2],
    wet: f32,
    feedback: f32,
    pingpong: f32,
    time_target: f32,
    time_samples: f32,
    k_time: f32,
    k_lp: f32,
    k_hp: f32,
}

impl Echo {
    pub fn new(sample_rate: f32) -> Self {
        let len = (MAX_TIME_S * sample_rate) as usize + 8;
        let mut echo = Self {
            sample_rate,
            lines: [Line::new(len), Line::new(len)],
            filters: [LoopFilter::new(), LoopFilter::new()],
            wet: 0.0,
            feedback: 0.35,
            pingpong: 0.0,
            time_target: 0.5 * sample_rate,
            time_samples: 0.5 * sample_rate,
            k_time: approach(TIME_GLIDE_S, sample_rate),
            k_lp: 0.0,
            k_hp: one_pole(LOOP_HP_HZ, sample_rate),
        };
        echo.set_tone(3000.0);
        echo
    }

    /// Insert level: how much of the whole bus feeds the line (0 = only
    /// the per-track sends reach it).
    pub fn set_wet(&mut self, wet: f32) {
        self.wet = wet.clamp(0.0, 1.0);
    }

    /// Delay time, seconds. The read head glides there (TIME_GLIDE_S).
    pub fn set_time(&mut self, seconds: f32) {
        self.time_target = seconds.clamp(0.0, MAX_TIME_S) * self.sample_rate;
    }

    /// Fraction of each repeat fed back into the line (before the loop
    /// filters take their share).
    pub fn set_feedback(&mut self, feedback: f32) {
        self.feedback = feedback.clamp(0.0, 0.98);
    }

    /// The loop lowpass corner, Hz: each pass through the line darkens the
    /// repeat by this 2-pole filter.
    pub fn set_tone(&mut self, cutoff_hz: f32) {
        let fc = cutoff_hz.clamp(200.0, 0.45 * self.sample_rate);
        self.k_lp = one_pole(fc, self.sample_rate);
    }

    /// 0 = each side echoes itself; 1 = repeats alternate sides.
    pub fn set_pingpong(&mut self, amount: f32) {
        self.pingpong = amount.clamp(0.0, 1.0);
    }

    pub fn process_with_send(
        &mut self,
        input_left: f32,
        input_right: f32,
        send_left: f32,
        send_right: f32,
    ) -> (f32, f32) {
        // One non-finite sample in a feedback loop circulates forever;
        // screening the inputs turns that into a one-sample dropout.
        let fin = |x: f32| if x.is_finite() { x } else { 0.0 };
        let (input_left, input_right) = (fin(input_left), fin(input_right));
        let (send_left, send_right) = (fin(send_left), fin(send_right));

        self.time_samples += self.k_time * (self.time_target - self.time_samples);
        let idle = self.wet == 0.0 && send_left == 0.0 && send_right == 0.0;
        let feed_l = input_left * self.wet + send_left;
        let feed_r = input_right * self.wet + send_right;

        let tap_l = self.lines[0].read(self.time_samples);
        let tap_r = self.lines[1].read(self.time_samples);
        let y_l = self.filters[0].process(tap_l, self.k_lp, self.k_hp);
        let y_r = self.filters[1].process(tap_r, self.k_lp, self.k_hp);

        // Ping-pong: the input leans toward the left line (fully mono-left
        // at 1), and the feedback crosses sides by the same amount.
        let p = self.pingpong;
        let mono = 0.5 * (feed_l + feed_r);
        let in_l = feed_l + p * (mono - feed_l) + p * mono;
        let in_r = feed_r * (1.0 - p);
        let fb = self.feedback;
        let back_l = (1.0 - p) * y_l + p * y_r;
        let back_r = (1.0 - p) * y_r + p * y_l;
        self.lines[0].push(soft(in_l + fb * back_l));
        self.lines[1].push(soft(in_r + fb * back_r));

        if idle && y_l == 0.0 && y_r == 0.0 {
            return (input_left, input_right);
        }
        (input_left + y_l, input_right + y_r)
    }

    pub fn process(&mut self, input_left: f32, input_right: f32) -> (f32, f32) {
        self.process_with_send(input_left, input_right, 0.0, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn impulse_response(echo: &mut Echo, secs: f32, send: bool) -> Vec<(f32, f32)> {
        let n = (secs * SR) as usize;
        (0..n)
            .map(|i| {
                let x = if i == 0 { 1.0 } else { 0.0 };
                if send {
                    let (l, r) = echo.process_with_send(0.0, 0.0, x, x);
                    (l, r)
                } else {
                    echo.process(x, x)
                }
            })
            .collect()
    }

    fn peak_index(xs: impl Iterator<Item = f32>) -> (usize, f32) {
        xs.enumerate().fold(
            (0, 0.0),
            |(bi, bv), (i, v)| if v.abs() > bv { (i, v.abs()) } else { (bi, bv) },
        )
    }

    #[test]
    fn transparent_when_nothing_feeds_it() {
        let mut echo = Echo::new(SR);
        echo.set_wet(0.0);
        for i in 0..20_000 {
            let x = ((i as f32) * 0.013).sin() * 0.7;
            let (l, r) = echo.process(x, -x);
            assert_eq!((l, r), (x, -x));
        }
    }

    #[test]
    fn first_repeat_lands_at_the_set_time() {
        for &(t, sr) in &[(0.375f32, 48_000.0f32), (1.2, 44_100.0), (0.25, 96_000.0)] {
            let mut echo = Echo::new(sr);
            echo.set_time(t);
            echo.time_samples = echo.time_target; // skip the glide-in
            echo.set_feedback(0.0);
            echo.set_tone(12_000.0);
            let n = ((t + 0.2) * sr) as usize;
            let out: Vec<f32> = (0..n)
                .map(|i| {
                    echo.process_with_send(0.0, 0.0, (i == 0) as u8 as f32, 0.0)
                        .0
                })
                .collect();
            let (i, _) = peak_index(out.into_iter());
            let want = t * sr;
            assert!(
                (i as f32 - want).abs() <= 0.002 * sr,
                "repeat at {} samples, wanted {} (sr {})",
                i,
                want,
                sr
            );
        }
    }

    #[test]
    fn repeats_decay_by_the_feedback_and_get_darker() {
        let mut echo = Echo::new(SR);
        echo.set_time(0.3);
        echo.time_samples = echo.time_target;
        echo.set_feedback(0.6);
        echo.set_tone(2_000.0);
        let ir: Vec<f32> = impulse_response(&mut echo, 2.0, true)
            .into_iter()
            .map(|(l, _)| l)
            .collect();
        let win = |k: usize| {
            let a = (k as f32 * 0.3 * SR) as usize - 200;
            ir[a..a + 4_000].iter().map(|v| v * v).sum::<f32>()
        };
        let (e1, e2, e3) = (win(1), win(2), win(3));
        assert!(e1 > 0.0 && e2 < e1 && e3 < e2, "{e1} {e2} {e3}");
        // energy ratio per repeat is at most fb^2 (the loop filters only
        // take more away)
        assert!(e2 / e1 <= 0.6f32.powi(2) * 1.05, "ratio {}", e2 / e1);
    }

    #[test]
    fn full_feedback_stays_bounded() {
        let mut echo = Echo::new(SR);
        echo.set_time(0.05);
        echo.set_feedback(0.98);
        echo.set_tone(15_000.0);
        echo.set_wet(1.0);
        let mut seed = 1u32;
        let mut peak = 0.0f32;
        for _ in 0..(20.0 * SR) as usize {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = (seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5;
            let (l, r) = echo.process(x, x);
            assert!(l.is_finite() && r.is_finite());
            peak = peak.max(l.abs()).max(r.abs());
        }
        assert!(peak < 8.0, "peak {peak}");
    }

    #[test]
    fn pingpong_alternates_sides() {
        let mut echo = Echo::new(SR);
        echo.set_time(0.25);
        echo.time_samples = echo.time_target;
        echo.set_feedback(0.7);
        echo.set_tone(12_000.0);
        echo.set_pingpong(1.0);
        let ir = impulse_response(&mut echo, 0.9, true);
        let side = |k: usize| {
            let a = (k as f32 * 0.25 * SR) as usize - 300;
            let (l, r) = ir[a..a + 1_500]
                .iter()
                .fold((0.0, 0.0), |(el, er), &(l, r)| (el + l * l, er + r * r));
            (l, r)
        };
        let (l1, r1) = side(1);
        let (l2, r2) = side(2);
        let (l3, r3) = side(3);
        assert!(l1 > 100.0 * r1.max(1e-12), "first repeat left: {l1} {r1}");
        assert!(r2 > 100.0 * l2.max(1e-12), "second repeat right: {l2} {r2}");
        assert!(l3 > 100.0 * r3.max(1e-12), "third repeat left: {l3} {r3}");
    }

    #[test]
    fn repeats_lose_their_lows() {
        // a 60 Hz tone through many repeats ends far quieter than a 1 kHz
        // tone through the same repeats: the loop high-pass at work
        let tail_energy = |hz: f32| {
            let mut echo = Echo::new(SR);
            echo.set_time(0.1);
            echo.time_samples = echo.time_target;
            echo.set_feedback(0.9);
            echo.set_tone(12_000.0);
            let burst = (0.08 * SR) as usize;
            let mut e = 0.0;
            for i in 0..(1.6 * SR) as usize {
                let x = if i < burst {
                    (TAU_F * hz * i as f32 / SR).sin()
                } else {
                    0.0
                };
                let (l, _) = echo.process_with_send(0.0, 0.0, x, x);
                if i > (1.2 * SR) as usize {
                    e += l * l;
                }
            }
            e
        };
        const TAU_F: f32 = std::f32::consts::TAU;
        assert!(tail_energy(60.0) < 0.1 * tail_energy(1_000.0));
    }

    #[test]
    fn non_finite_input_does_not_poison_the_loop() {
        let mut echo = Echo::new(SR);
        echo.set_wet(1.0);
        echo.set_feedback(0.9);
        echo.process(f32::NAN, f32::INFINITY);
        for _ in 0..(1.0 * SR) as usize {
            let (l, r) = echo.process(0.1, 0.1);
            assert!(l.is_finite() && r.is_finite());
        }
    }
}
