// First-order antiderivative antialiasing (Parker et al., DAFx-16;
// Bilbao et al., IEEE SPL 2017) for the tanh nonlinearity, as used for the
// Moog ladder in Paschou et al., APSIPA 2017.
//
// The antialiased form of y = tanh(x) is
//
//     y[n] = (F0(x[n]) - F0(x[n-1])) / (x[n] - x[n-1]),
//
// where F0(x) = ln(cosh(x)) is the antiderivative. When consecutive inputs
// are nearly equal the quotient is ill-conditioned and the midpoint form
// tanh((x[n] + x[n-1])/2) is used instead. This suppresses the aliasing the
// tanh harmonics would otherwise fold back into the audio band, at the cost
// of a benign half-sample delay.

pub struct AdaaTanh {
    x1: f32,
    f1: f32, // ln(cosh(x1))
}

/// Numerically stable ln(cosh(x)): |x| + ln(1 + e^(-2|x|)) - ln 2.
#[inline]
fn ln_cosh(x: f32) -> f32 {
    let a = x.abs();
    a + (-2.0 * a).exp().ln_1p() - std::f32::consts::LN_2
}

impl AdaaTanh {
    pub fn new() -> Self {
        Self { x1: 0.0, f1: 0.0 }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let f0 = ln_cosh(x);
        let dx = x - self.x1;
        let out = if dx.abs() < 1e-3 {
            (0.5 * (x + self.x1)).tanh()
        } else {
            (f0 - self.f1) / dx
        };
        self.x1 = x;
        self.f1 = f0;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_tanh_for_slow_signals() {
        let mut adaa = AdaaTanh::new();
        // A slow ramp: after the first sample (startup transient from the
        // zero-initialized state), ADAA should track plain tanh closely
        for n in 0..1000 {
            let x = -2.0 + 4.0 * n as f32 / 1000.0;
            let y = adaa.process(x);
            if n == 0 {
                continue;
            }
            // Compare against tanh at the half-sample-delayed point
            let x_mid = x - 0.002;
            assert!(
                (y - x_mid.tanh()).abs() < 0.01,
                "ADAA diverged at x={x}: {y} vs {}",
                x_mid.tanh()
            );
        }
    }

    #[test]
    fn bounded_for_fast_signals() {
        let mut adaa = AdaaTanh::new();
        for n in 0..1000 {
            // Nyquist-rate alternation at high amplitude
            let x = if n % 2 == 0 { 8.0 } else { -8.0 };
            let y = adaa.process(x);
            assert!(y.abs() <= 1.0 + 1e-4, "ADAA out of range: {y}");
        }
    }
}
