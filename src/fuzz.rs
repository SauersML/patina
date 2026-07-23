// Germanium fuzz stage, after the GBJT analysis of Holmes, Holters &
// van Walstijn (DAFx-17). A germanium common-emitter stage clips with a
// much softer knee than silicon (the Ebers-Moll exponential at ~26 mV
// thermal voltage) and its bias point sits asymmetrically, so overdriving
// it produces strong odd harmonics plus the gentle even-order content the
// paper measures from real AC128/OC44 devices ("the imperfect nature of
// the transistors, whose saturating behavior is slightly asymmetric").
//
// Modeled as a biased soft-knee waveshaper: u = g*(x + BIAS), y = tanh(u),
// with the tanh evaluated through first-order antiderivative antialiasing
// (Parker et al.) and a DC blocker to remove the bias-shift thump.
// One knob: 0 = true bypass, 1 = full Fuzz-Face scream.

use crate::adaa::AdaaTanh;

/// Record-bias asymmetry: pushes the operating point off-center so the
/// clipping is uneven and even harmonics appear.
const BIAS: f32 = 0.14;

struct DcBlock {
    x1: f32,
    y1: f32,
}

impl DcBlock {
    fn new() -> Self {
        Self { x1: 0.0, y1: 0.0 }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + 0.9955 * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

pub struct Fuzz {
    amount: f32,
    smoothed: f32,
    adaa: [AdaaTanh; 2],
    dc: [DcBlock; 2],
}

impl Fuzz {
    pub fn new() -> Self {
        Self {
            amount: 0.0,
            smoothed: 0.0,
            adaa: [AdaaTanh::new(), AdaaTanh::new()],
            dc: [DcBlock::new(), DcBlock::new()],
        }
    }

    pub fn set_amount(&mut self, amount: f32) {
        self.amount = amount.clamp(0.0, 1.0);
    }

    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        // The ADAA shaper and the DC blocker both keep state, and neither
        // can ever leave a non-finite value: ln(cosh(NaN)) is NaN and the
        // blocker's pole feeds its own NaN back. The pedal is first on the
        // master bus, so one bad sample used to kill the instrument for the
        // life of the process. Screening the input is O(1).
        let left = if left.is_finite() { left } else { 0.0 };
        let right = if right.is_finite() { right } else { 0.0 };

        self.smoothed += (self.amount - self.smoothed) * 0.001;
        let w = self.smoothed;
        if w < 0.002 && self.amount < 0.002 {
            return (left, right);
        }

        // Perceptual gain taper: gentle grit low on the knob, screaming
        // germanium saturation at the top
        let g = 1.0 + w * w * 40.0;
        // Make-up keeps small-signal loudness roughly constant as g rises
        let makeup = 0.85 / (g * 0.7).tanh().max(0.3);
        // Short crossfade near zero so engaging the knob never clicks
        let fade = (w * 50.0).min(1.0);

        let mut out = [left, right];
        for (ch, sample) in out.iter_mut().enumerate() {
            let u = g * (*sample * 0.7 + BIAS);
            let shaped = self.adaa[ch].process(u);
            let fuzzed = self.dc[ch].process(shaped) * makeup;
            *sample = *sample * (1.0 - fade) + fuzzed * fade;
        }
        (out[0], out[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    #[test]
    fn bypass_at_zero() {
        let mut fuzz = Fuzz::new();
        fuzz.set_amount(0.0);
        for n in 0..1000 {
            let x = (TAU * 220.0 * n as f32 / 44100.0).sin() * 0.5;
            let (l, r) = fuzz.process(x, x);
            assert_eq!(l, x);
            assert_eq!(r, x);
        }
    }

    /// The ADAA shaper and the DC blocker both carry state across samples,
    /// and neither can leave a non-finite value once it is in: `ln_cosh`
    /// keeps returning NaN and the blocker's pole feeds NaN back forever.
    /// The pedal sits FIRST on the master bus, so one bad sample from any
    /// voice used to take the whole instrument down for good.
    #[test]
    fn a_nan_does_not_kill_the_pedal() {
        let mut fuzz = Fuzz::new();
        fuzz.set_amount(1.0);
        for _ in 0..8000 {
            fuzz.process(0.0, 0.0);
        }
        fuzz.process(f32::NAN, f32::INFINITY);
        let mut energy = 0.0f32;
        for n in 0..44100 {
            let x = (TAU * 220.0 * n as f32 / 44100.0).sin() * 0.5;
            let (l, r) = fuzz.process(x, x);
            assert!(l.is_finite() && r.is_finite(), "poisoned at sample {n}");
            if n > 4410 {
                energy += l * l;
            }
        }
        assert!(energy > 1.0, "fuzz should be passing audio again: {energy}");
    }

    #[test]
    fn saturates_and_stays_bounded() {
        let mut fuzz = Fuzz::new();
        fuzz.set_amount(1.0);
        // Let the engage smoothing settle
        for _ in 0..8000 {
            fuzz.process(0.0, 0.0);
        }
        let mut peak = 0.0f32;
        let mut crest_num = 0.0f32;
        let mut crest_den = 0.0f32;
        for n in 0..44100 {
            let x = (TAU * 110.0 * n as f32 / 44100.0).sin() * 0.8;
            let (l, _) = fuzz.process(x, x);
            assert!(l.is_finite());
            peak = peak.max(l.abs());
            if n > 22050 {
                crest_num += l * l;
                crest_den += 1.0;
            }
        }
        assert!(peak < 2.0, "fuzz output should stay bounded, peak={peak}");
        // Heavily clipped output has RMS well above a sine's 0.707 ratio to
        // its plateau; the AC-coupling tilt and edge overshoot (real fuzz
        // pedal behavior) keep it below an ideal square's 1.0
        let rms = (crest_num / crest_den).sqrt();
        assert!(
            rms > peak * 0.42,
            "full fuzz should square the wave up: rms={rms}, peak={peak}"
        );
    }
}
