// The REVERB knob — Patina's own voice, deliberately modern.
//
// The spring (spring.rs) is the 1971 circuit, kept authentic down to its
// flaws. This unit is the opposite commitment: the most beautiful tail we
// can build, with no vintage hardware to be faithful to. Design is the
// classic high-quality feedback-delay-network recipe (Jot's energy-exact
// decay, Dattorro's input diffusion, modulated tank lines):
//
//   in -> pre-delay -> band-limit -> 4 series allpass diffusers
//      -> 8-line tank, Householder unitary feedback
//         each line: fractional read (4 lines slowly modulated),
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
const MOD_RATES: [f32; 4] = [0.071, 0.113, 0.167, 0.229];
/// Modulation depth, ms (a few cents of pitch at these rates).
const MOD_DEPTH_MS: f32 = 0.16;

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
        self.buffer[self.write] = x;
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

    /// Retune without touching the state, so a knob ride mid-tail can't
    /// zero the tank's stored energy (that would tick audibly).
    fn set_cutoff(&mut self, cutoff: f32, sample_rate: f32) {
        self.a = 1.0 - (-TAU * cutoff / sample_rate).exp();
    }
}

pub struct Reverb {
    sample_rate: f32,
    pre_delay: DelayLine,
    pre_delay_samples: usize,
    in_lp: OnePoleLp,
    in_hp_tracker: OnePoleLp,
    diffusers: [Diffuser; 4],
    lines: [DelayLine; N],
    line_len: [f32; N],
    damping: [OnePoleLp; N],
    /// Per-line decay gain for the current T60 (Jot's condition).
    gains: [f32; N],
    lfo_phase: [f32; 4],
    lfo_inc: [f32; 4],
    mod_depth: f32,
    /// Wet-path low cut so long tails don't accumulate mud.
    out_hp_l: OnePoleLp,
    out_hp_r: OnePoleLp,
    wet: f32,
    dry: f32,
}

impl Reverb {
    pub fn new(sample_rate: f32) -> Self {
        let line_len =
            core::array::from_fn(|i| (LINE_MS[i] * 1e-3 * sample_rate).max(8.0));
        let lines = core::array::from_fn(|i| {
            DelayLine::new(line_len[i] as usize + (MOD_DEPTH_MS * 1e-3 * sample_rate) as usize + 8)
        });
        let mut r = Self {
            sample_rate,
            pre_delay: DelayLine::new((0.082 * sample_rate) as usize + 2),
            pre_delay_samples: (0.012 * sample_rate) as usize,
            in_lp: OnePoleLp::new(9500.0, sample_rate),
            in_hp_tracker: OnePoleLp::new(90.0, sample_rate),
            diffusers: core::array::from_fn(|i| Diffuser::new(sample_rate, DIFF_MS[i])),
            lines,
            line_len,
            damping: core::array::from_fn(|_| OnePoleLp::new(5500.0, sample_rate)),
            gains: [0.0; N],
            lfo_phase: [0.0, 0.25, 0.5, 0.75],
            lfo_inc: core::array::from_fn(|i| MOD_RATES[i] / sample_rate),
            mod_depth: MOD_DEPTH_MS * 1e-3 * sample_rate,
            out_hp_l: OnePoleLp::new(60.0, sample_rate),
            out_hp_r: OnePoleLp::new(60.0, sample_rate),
            wet: 0.3,
            dry: 0.7,
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
        self.dry = 1.0 - self.wet;
    }

    /// Pre-delay in seconds (0..80 ms). Separating the dry hit from the
    /// tail's onset is most of what "size" and "clarity" mean in a mix:
    /// a 40-60 ms gap keeps transients legible inside a dark room.
    pub fn set_pre(&mut self, seconds: f32) {
        self.pre_delay_samples =
            ((seconds.clamp(0.0, 0.08) * self.sample_rate) as usize)
                .min(self.pre_delay.buffer.len() - 2);
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

    /// The tank is linear, so the wet knob can live on the INPUT side:
    /// tank(in * wet) == tank(in) * wet, bit-for-bit the legacy mix — and
    /// a per-channel send bus becomes just another input into the same
    /// tank, heard at unity regardless of the global wet knob.
    pub fn process_with_send(
        &mut self,
        input_left: f32,
        input_right: f32,
        send_left: f32,
        send_right: f32,
    ) -> (f32, f32) {
        // Feed: mono sum through pre-delay and band limits into the
        // diffusion chain
        let mono = (input_left + input_right) * 0.5 * self.wet
            + (send_left + send_right) * 0.5;
        self.pre_delay.push(mono);
        let fed = self.pre_delay.read_int(self.pre_delay_samples);
        let fed = self.in_lp.process(fed);
        let fed = fed - self.in_hp_tracker.process(fed);
        let mut diffused = fed;
        for d in &mut self.diffusers {
            diffused = d.process(diffused);
        }

        // Tank read: lines 0..3 modulated, 4..7 static
        let mut outs = [0.0f32; N];
        for i in 0..N {
            let delay = if i < 4 {
                self.lfo_phase[i] = (self.lfo_phase[i] + self.lfo_inc[i]) % 1.0;
                self.line_len[i] + self.mod_depth * (TAU * self.lfo_phase[i]).sin()
            } else {
                self.line_len[i]
            };
            let v = self.lines[i].read_frac(delay);
            outs[i] = self.damping[i].process(v) * self.gains[i];
        }

        // Householder feedback: y_i = x_i - (2/N) * sum  (unitary, so the
        // per-line gains alone set the decay)
        let s = outs.iter().sum::<f32>() * (2.0 / N as f32);
        for i in 0..N {
            // Alternating injection signs decorrelate the lines from the
            // shared mono feed
            let inject = if i % 2 == 0 { diffused } else { -diffused };
            self.lines[i].push(inject + outs[i] - s);
        }

        // Stereo taps from disjoint line sets: genuine width, and the mono
        // sum keeps everything (no cancellation between L and R)
        let wet_l = (outs[0] - outs[2] + outs[4] - outs[6]) * 0.6;
        let wet_r = (outs[1] - outs[3] + outs[5] - outs[7]) * 0.6;
        let wet_l = wet_l - self.out_hp_l.process(wet_l);
        let wet_r = wet_r - self.out_hp_r.process(wet_r);

        (
            input_left * self.dry + wet_l,
            input_right * self.dry + wet_r,
        )
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
}
