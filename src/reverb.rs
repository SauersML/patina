// The REVERB knob — Patina's own voice, deliberately modern.
//
// The spring (spring.rs) is the 1971 circuit, kept authentic down to its
// flaws. This unit is the opposite commitment: the most beautiful tail we
// can build, with no vintage hardware to be faithful to. Design is the
// classic high-quality feedback-delay-network recipe (Jot's energy-exact
// decay, Dattorro's input diffusion, modulated tank lines):
//
//   in (L, R) -> pre-delay -> band-limit -> 4 series allpass diffusers
//      -> 8-line tank, Householder unitary feedback
//         each line: fractional read (every line slowly modulated),
//         one-pole damping in the loop, per-line gain
//         g_i = 10^(-3 L_i / (T60 sr))  -- every line decays at the SAME
//         rate, so the tail's color stays constant as it fades
//      -> stereo taps from disjoint line sets (real width, mono-safe)
//
// The line modulation is the load-bearing choice: a static FDN of any
// size eventually exposes its modes as metallic ringing; a few cents of
// slow, incommensurate delay modulation sweeps the modes continuously and
// the ear hears "air" instead of "metal".

use std::f32::consts::TAU;

const N: usize = 8;
/// Tank line lengths, ms — mutually non-commensurate, 31..74 ms spread.
const LINE_MS: [f32; N] = [31.71, 37.11, 40.23, 44.14, 51.43, 58.22, 66.18, 73.66];
/// Diffuser lengths, ms (Dattorro's figure-of-merit set).
const DIFF_MS: [f32; 4] = [4.77, 3.60, 12.73, 9.30];
const DIFF_G: f32 = 0.70;
/// LFO rates for the modulated lines, Hz — incommensurate on purpose.
const MOD_RATES: [f32; N] = [0.071, 0.113, 0.167, 0.229, 0.089, 0.137, 0.193, 0.263];
/// Modulation depth, ms: 1-3 cents of pitch at these rates. At the old
/// 0.16 ms, on half the lines, the tail's spectrum held narrow peaks 10 dB
/// above its local mean (top 1%) where a fully diffuse tail sits at 6.5;
/// sweeping every line this far brings it to 7.5, and deeper buys nothing.
const MOD_DEPTH_MS: f32 = 1.0;
/// How far each tank line set leans toward its own input side (0 = the
/// old mono feed, 1 = left lines hear only the left input).
const STEREO_FEED: f32 = 0.6;

/// Snap values that have decayed past all audibility to exactly zero.
///
/// An FDN tail is a pure exponential with nothing to stop it: once the
/// stored energy passes ~1e-38 every line and every damping pole is full
/// of DENORMALS, and it stays that way for tens of seconds because each
/// pass only shaves a fraction of a dB. Denormal arithmetic costs 10-100x
/// a normal multiply on x86, so the reverb's CPU load *rises* after the
/// music stops — the classic cause of dropouts on an otherwise idle host.
/// 1e-20 is -400 dBFS: seventeen orders of magnitude below the quietest
/// thing anyone has ever heard, and eighteen above the denormal cliff.
#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

#[derive(Clone)]
struct DelayLine {
    buffer: Vec<f32>,
    write: usize,
}

impl DelayLine {
    fn new(len: usize) -> Self {
        Self {
            buffer: vec![0.0; len.max(4)],
            write: 0,
        }
    }

    #[inline]
    fn push(&mut self, x: f32) {
        self.buffer[self.write] = flush(x);
        self.write = (self.write + 1) % self.buffer.len();
    }

    /// Read `delay` samples back (fractional, linear interpolation).
    #[inline]
    fn read_frac(&self, delay: f32) -> f32 {
        let len = self.buffer.len();
        let delay = delay.clamp(1.0, (len - 2) as f32);
        let d0 = delay as usize;
        let frac = delay - d0 as f32;
        let i0 = (len + self.write - 1 - d0) % len;
        let i1 = (len + i0 - 1) % len;
        self.buffer[i0] * (1.0 - frac) + self.buffer[i1] * frac
    }

    #[inline]
    fn read_int(&self, delay: usize) -> f32 {
        let len = self.buffer.len();
        let i = (len + self.write - 1 - delay.min(len - 2)) % len;
        self.buffer[i]
    }
}

/// Schroeder allpass diffuser.
#[derive(Clone)]
struct Diffuser {
    line: DelayLine,
    delay: usize,
}

impl Diffuser {
    fn new(sample_rate: f32, ms: f32) -> Self {
        let delay = (ms * 1e-3 * sample_rate) as usize;
        Self {
            line: DelayLine::new(delay + 2),
            delay,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let d = self.line.read_int(self.delay);
        let v = x - DIFF_G * d;
        self.line.push(v);
        d + DIFF_G * v
    }
}

#[derive(Clone)]
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

    /// Retune without touching the state, so a knob ride mid-tail can't
    /// zero the tank's stored energy (that would tick audibly).
    fn set_cutoff(&mut self, cutoff: f32, sample_rate: f32) {
        self.a = 1.0 - (-TAU * cutoff / sample_rate).exp();
    }
}

/// One side's feed into the tank: pre-delay, band limits, diffusion.
/// Left and right each have their own, so the tank hears WHERE a sound
/// is, not only that it happened.
#[derive(Clone)]
struct Feed {
    pre_delay: DelayLine,
    in_lp: OnePoleLp,
    in_hp_tracker: OnePoleLp,
    diffusers: [Diffuser; 4],
}

impl Feed {
    fn new(sample_rate: f32) -> Self {
        Self {
            pre_delay: DelayLine::new((0.082 * sample_rate) as usize + 2),
            in_lp: OnePoleLp::new(9500.0, sample_rate),
            in_hp_tracker: OnePoleLp::new(90.0, sample_rate),
            diffusers: core::array::from_fn(|i| Diffuser::new(sample_rate, DIFF_MS[i])),
        }
    }

    fn process(&mut self, x: f32, pre_delay_samples: usize) -> f32 {
        self.pre_delay.push(x);
        let fed = self.pre_delay.read_int(pre_delay_samples);
        let fed = self.in_lp.process(fed);
        let mut diffused = fed - self.in_hp_tracker.process(fed);
        for d in &mut self.diffusers {
            diffused = d.process(diffused);
        }
        diffused
    }
}

#[derive(Clone)]
pub struct Reverb {
    sample_rate: f32,
    feeds: [Feed; 2],
    pre_delay_samples: usize,
    lines: [DelayLine; N],
    line_len: [f32; N],
    damping: [OnePoleLp; N],
    /// Per-line decay gain for the current T60 (Jot's condition).
    gains: [f32; N],
    lfo_phase: [f32; N],
    lfo_inc: [f32; N],
    mod_depth: f32,
    /// Wet-path low cut so long tails don't accumulate mud.
    out_hp_l: OnePoleLp,
    out_hp_r: OnePoleLp,
    wet: f32,
}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let line_len = core::array::from_fn(|i| (LINE_MS[i] * 1e-3 * sample_rate).max(8.0));
        let lines = core::array::from_fn(|i| {
            DelayLine::new(line_len[i] as usize + (MOD_DEPTH_MS * 1e-3 * sample_rate) as usize + 8)
        });
        let mut r = Self {
            sample_rate,
            feeds: [Feed::new(sample_rate), Feed::new(sample_rate)],
            pre_delay_samples: (0.012 * sample_rate) as usize,
            lines,
            line_len,
            damping: core::array::from_fn(|_| OnePoleLp::new(5500.0, sample_rate)),
            gains: [0.0; N],
            lfo_phase: core::array::from_fn(|i| i as f32 / N as f32),
            lfo_inc: core::array::from_fn(|i| MOD_RATES[i] / sample_rate),
            mod_depth: MOD_DEPTH_MS * 1e-3 * sample_rate,
            out_hp_l: OnePoleLp::new(60.0, sample_rate),
            out_hp_r: OnePoleLp::new(60.0, sample_rate),
            wet: 0.3,
        };
        r.set_decay(0.55);
        r
    }

    /// Map the panel's 0..1 to a T60 and set every line's gain so the
    /// whole tank decays at exactly that rate.
    pub fn set_decay(&mut self, decay: f32) {
        let d = decay.clamp(0.0, 1.0);
        let t60 = 0.25 + 5.0 * d * d; // 0.25 s .. 5.25 s
        for i in 0..N {
            self.gains[i] = 10f32.powf(-3.0 * self.line_len[i] / (t60 * self.sample_rate));
        }
    }

    pub fn set_wet(&mut self, wet: f32) {
        self.wet = wet.clamp(0.0, 1.0);
    }

    /// Pre-delay in seconds (0..80 ms). Separating the dry hit from the
    /// tail's onset is most of what "size" and "clarity" mean in a mix:
    /// a 40-60 ms gap keeps transients legible inside a dark room.
    pub fn set_pre(&mut self, seconds: f32) {
        self.pre_delay_samples = ((seconds.clamp(0.0, 0.08) * self.sample_rate) as usize)
            .min(self.feeds[0].pre_delay.buffer.len() - 2);
    }

    /// Tail damping cutoff, Hz. The in-loop lowpass is the tail's COLOR:
    /// ~2.5 kHz is a dusty dark room, 5.5 kHz the unit's bright default.
    /// State is preserved across retunes so rides don't tick.
    pub fn set_tone(&mut self, cutoff: f32) {
        let fc = cutoff.clamp(800.0, 12000.0);
        for d in &mut self.damping {
            d.set_cutoff(fc, self.sample_rate);
        }
    }

    pub fn process(&mut self, input_left: f32, input_right: f32) -> (f32, f32) {
        self.process_with_send(input_left, input_right, 0.0, 0.0)
    }

    /// The tank is linear, so the wet knob lives on the input side and the
    /// dry path remains at unity. Per-channel sends join the same parallel
    /// tank independently of the master wet amount.
    pub fn process_with_send(
        &mut self,
        input_left: f32,
        input_right: f32,
        send_left: f32,
        send_right: f32,
    ) -> (f32, f32) {
        // The tank is a closed feedback network with no nonlinearity to
        // trap a bad value: one non-finite sample entering it circulates
        // forever and the reverb -- which sits across the whole master bus
        // -- is dead until the plugin is reloaded. Screening the input is
        // O(1) and turns a permanent kill into a one-sample dropout.
        // (tape.rs takes the same position on its own magnetic state.)
        let input_left = if input_left.is_finite() {
            input_left
        } else {
            0.0
        };
        let input_right = if input_right.is_finite() {
            input_right
        } else {
            0.0
        };
        let send_left = if send_left.is_finite() {
            send_left
        } else {
            0.0
        };
        let send_right = if send_right.is_finite() {
            send_right
        } else {
            0.0
        };

        // Feed: each side through its own pre-delay, band limits and
        // diffusion. The tank used to hear the mono sum, so a note panned
        // hard left bloomed exactly as it would have from the center.
        // Each line takes its side at the level it used to take the mono
        // sum, so a centered sound fills the tank exactly as before.
        let pre = self.pre_delay_samples;
        let diffused_l = self.feeds[0].process(input_left * self.wet + send_left, pre);
        let diffused_r = self.feeds[1].process(input_right * self.wet + send_right, pre);
        // Mid/side at the injection: every line keeps the full mid (a
        // centered sound fills the tank exactly as it did), and the side
        // leans each line set toward its own speaker by STEREO_FEED. Full
        // side weight would throw 3 dB more reverb at a hard-panned sound
        // than it used to; 0.6 keeps the image and moves that level 1.3 dB.
        let mid = (diffused_l + diffused_r) * 0.5;
        let side = (diffused_l - diffused_r) * 0.5 * STEREO_FEED;
        let (feed_l, feed_r) = (mid + side, mid - side);

        // Tank read: every line slowly modulated (MOD_RATES)
        let mut outs = [0.0f32; N];
        for i in 0..N {
            self.lfo_phase[i] = (self.lfo_phase[i] + self.lfo_inc[i]) % 1.0;
            let delay = self.line_len[i] + self.mod_depth * (TAU * self.lfo_phase[i]).sin();
            let v = self.lines[i].read_frac(delay);
            outs[i] = self.damping[i].process(v) * self.gains[i];
        }

        // Householder feedback: y_i = x_i - (2/N) * sum  (unitary, so the
        // per-line gains alone set the decay)
        let s = outs.iter().sum::<f32>() * (2.0 / N as f32);
        for i in 0..N {
            // Even lines (the left taps) take the left feed and odd lines
            // the right, so the tail opens on the side the sound came from
            // before the Householder mix spreads it. The alternating signs
            // decorrelate the two sets; for a centered sound this is the
            // mono feed exactly, sign for sign.
            let inject = if i % 2 == 0 { feed_l } else { -feed_r };
            self.lines[i].push(inject + outs[i] - s);
        }

        // Stereo taps from disjoint line sets: genuine width, and the mono
        // sum keeps everything (no cancellation between L and R)
        let wet_l = (outs[0] - outs[2] + outs[4] - outs[6]) * 0.6;
        let wet_r = (outs[1] - outs[3] + outs[5] - outs[7]) * 0.6;
        let wet_l = wet_l - self.out_hp_l.process(wet_l);
        let wet_r = wet_r - self.out_hp_r.process(wet_r);

        (input_left + wet_l, input_right + wet_r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_at_zero_wet() {
        let mut reverb = Reverb::new(48000.0);
        reverb.set_wet(0.0);
        for n in 0..4800 {
            let x = (n as f32 * 0.05).sin() * 0.5;
            let (l, r) = reverb.process(x, -x);
            assert_eq!(l, x);
            assert_eq!(r, -x);
        }
    }

    /// Impulse-response tail level must track the requested T60: with the
    /// per-line gain law, level falls 60/T60 dB per second.
    #[test]
    fn tail_decays_at_the_requested_rate() {
        let sr = 48000.0;
        let mut reverb = Reverb::new(sr);
        reverb.set_wet(1.0);
        reverb.set_decay(0.55); // T60 = 0.25 + 5*0.55^2 ~ 1.76 s
        let t60 = 0.25 + 5.0 * 0.55 * 0.55;
        reverb.process(1.0, 1.0);
        let n = (3.0 * sr) as usize;
        let mut rms_a = 0.0f64;
        let mut rms_b = 0.0f64;
        for i in 0..n {
            let (l, r) = reverb.process(0.0, 0.0);
            assert!(l.is_finite() && r.is_finite());
            let e = (l * l + r * r) as f64;
            if ((0.4 * sr) as usize..(0.6 * sr) as usize).contains(&i) {
                rms_a += e;
            }
            if ((1.4 * sr) as usize..(1.6 * sr) as usize).contains(&i) {
                rms_b += e;
            }
        }
        let drop_db = 10.0 * (rms_a / rms_b.max(1e-30)).log10() as f32;
        let expected = 60.0 / t60; // dB per second, measured over 1 s
        assert!(
            (drop_db / expected - 1.0).abs() < 0.35,
            "tail should drop ~{expected:.1} dB/s, measured {drop_db:.1}"
        );
    }

    /// The tail must be dense and smooth — no discrete slap echoes. In any
    /// late window the peak should not tower over the RMS.
    #[test]
    fn tail_is_dense_not_echoey() {
        let sr = 48000.0;
        let mut reverb = Reverb::new(sr);
        reverb.set_wet(1.0);
        reverb.set_decay(0.7);
        reverb.process(1.0, 1.0);
        let start = (0.3 * sr) as usize;
        let end = (0.5 * sr) as usize;
        let mut peak = 0.0f32;
        let mut rms = 0.0f64;
        for i in 0..end {
            let (l, _) = reverb.process(0.0, 0.0);
            if i >= start {
                peak = peak.max(l.abs());
                rms += (l * l) as f64;
            }
        }
        let rms = ((rms / (end - start) as f64) as f32).sqrt();
        assert!(rms > 1e-6, "tail should still be alive at 300-500 ms");
        assert!(
            peak < 8.0 * rms,
            "tail should be diffuse: peak {peak:.5} vs rms {rms:.5}"
        );
    }

    /// The tank hears where a sound is: a hit on the left opens its tail
    /// on the left before the network spreads it, while a centered hit
    /// stays balanced.
    #[test]
    fn a_panned_hit_blooms_on_its_own_side() {
        let sr = 48000.0;
        let early = |l_in: f32, r_in: f32| {
            let mut reverb = Reverb::new(sr);
            reverb.set_wet(1.0);
            reverb.set_pre(0.0);
            let (mut el, mut er) = (0.0f64, 0.0f64);
            for i in 0..(0.1 * sr) as usize {
                let x = if i == 0 { 1.0 } else { 0.0 };
                let (l, r) = reverb.process(x * l_in, x * r_in);
                el += ((l - x * l_in) as f64).powi(2);
                er += ((r - x * r_in) as f64).powi(2);
            }
            10.0 * (el / er).log10()
        };
        let left = early(1.0, 0.0);
        assert!(left > 6.0, "a left hit's first 100 ms leans only {left:.1} dB left");
        // The two sides tap different lines, so their first echoes land at
        // different times: ~0.9 dB of early lean is the tap layout (the
        // whole tail is balanced to 0.1 dB), not a feed imbalance.
        let center = early(1.0, 1.0);
        assert!(center.abs() < 1.5, "a centered hit leans {center:.1} dB");
    }

    /// A mono impulse must come back with real stereo width: the L/R
    /// tails read from disjoint tank lines and should decorrelate.
    #[test]
    fn tail_has_stereo_width() {
        let sr = 48000.0;
        let mut reverb = Reverb::new(sr);
        reverb.set_wet(1.0);
        reverb.set_decay(0.7);
        reverb.process(1.0, 1.0);
        let (mut ll, mut rr, mut lr) = (0.0f64, 0.0f64, 0.0f64);
        for _ in 0..(sr as usize) {
            let (l, r) = reverb.process(0.0, 0.0);
            ll += (l * l) as f64;
            rr += (r * r) as f64;
            lr += (l * r) as f64;
        }
        let corr = lr / (ll * rr).sqrt().max(1e-30);
        assert!(
            corr.abs() < 0.6,
            "L/R tails should decorrelate, correlation {corr:.3}"
        );
        // ...but both channels must carry comparable energy
        let balance = ll / rr.max(1e-30);
        assert!(
            (0.25..4.0).contains(&balance),
            "channel energy should be balanced, L/R ratio {balance:.2}"
        );
    }

    /// Unitary feedback times sub-unity gains: bounded under sustained
    /// hot input at maximum decay.
    #[test]
    fn stable_at_maximum_decay() {
        let sr = 48000.0;
        let mut reverb = Reverb::new(sr);
        reverb.set_wet(1.0);
        reverb.set_decay(1.0);
        let mut peak = 0.0f32;
        for n in 0..(5 * sr as usize) {
            let x = (TAU * 180.0 * n as f32 / sr).sin() * 4.5;
            let (l, r) = reverb.process(x, x);
            assert!(l.is_finite() && r.is_finite());
            peak = peak.max(l.abs().max(r.abs()));
        }
        assert!(peak < 60.0, "reverb must stay bounded, peak {peak}");
    }

    /// Regression: the tail is a pure exponential, so once it passed the
    /// f32 denormal cliff every tank line and damping pole was full of
    /// denormals — and STAYED that way for tens of seconds, because each
    /// pass only shaves a fraction of a dB. Denormal arithmetic is 10-100x
    /// slower on x86, so the reverb's CPU cost went UP after the music
    /// stopped. Measured before the fix: ~565,000 denormal output samples
    /// in 20 s at 48 kHz, still going at the end.
    #[test]
    fn a_dead_tail_flushes_instead_of_going_denormal() {
        for sr in [44100.0f32, 48000.0, 96000.0] {
            let mut reverb = Reverb::new(sr);
            reverb.set_wet(1.0);
            reverb.set_decay(0.3);
            reverb.process(1.0, 1.0);
            let mut denormals = 0usize;
            let n = (20.0 * sr) as usize;
            for _ in 0..n {
                let (l, r) = reverb.process(0.0, 0.0);
                for v in [l, r] {
                    if v != 0.0 && v.abs() < f32::MIN_POSITIVE {
                        denormals += 1;
                    }
                }
            }
            assert_eq!(denormals, 0, "denormal output samples at {sr} Hz");
            // and the tank really is at rest, not merely quiet
            let (l, r) = reverb.process(0.0, 0.0);
            assert_eq!((l, r), (0.0, 0.0), "tank should have flushed at {sr} Hz");
        }
    }

    /// The tank is a closed feedback network across the whole master bus:
    /// one non-finite sample used to circulate in it forever, so the
    /// instrument stayed dead until the plugin was reloaded.
    #[test]
    fn a_nan_does_not_poison_the_tank() {
        let mut reverb = Reverb::new(48000.0);
        reverb.set_wet(0.5);
        reverb.process(f32::NAN, f32::NAN);
        reverb.process_with_send(0.0, 0.0, f32::INFINITY, f32::NAN);
        let mut energy = 0.0f32;
        for n in 0..48000 {
            let x = (TAU * 220.0 * n as f32 / 48000.0).sin() * 0.5;
            let (l, r) = reverb.process(x, x);
            assert!(l.is_finite() && r.is_finite(), "poisoned at sample {n}");
            if n > 4800 {
                energy += l * l;
            }
        }
        assert!(
            energy > 1.0,
            "reverb should be passing audio again: {energy}"
        );
    }
}
