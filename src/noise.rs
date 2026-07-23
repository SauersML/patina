// Shared noise source, per the 903A and the Juno-106 architecture: ONE
// transistor noise generator feeds every voice, so noise in chords and
// unison stays correlated instead of six independent random streams
// (which sounds wider and more diffuse than the hardware).
//
// The ARP 2600 manual describes the source as "an amplified, reversed
// junction of a selected transistor" followed by clipping — so: white
// noise, soft transistor clipping, then a gentle high-frequency rolloff
// from the amplifier stage.

/// The amplifier stage's bandwidth, in hertz. This is the ONE place the
/// corner lives; the per-sample coefficient is DERIVED from it and the
/// host rate, so the two cannot drift apart. A hardcoded coefficient is a
/// silent 44.1 kHz assumption: the 0.58 this used to carry put the corner
/// at 6.5 kHz at 44.1 kHz, 7.1 kHz at 48 kHz and 14.2 kHz at 96 kHz, so
/// the noise — and every render, patch and unvoiced consonant that leans
/// on it — got brighter the faster the host ran.
const AMP_CORNER_HZ: f32 = 6000.0;

pub struct NoiseSource {
    rng: u32,
    /// Trapezoidal (zero-delay) integrator state for the rolloff.
    lp_s: f32,
    /// tan-prewarped one-pole coefficient. Prewarping (the same form the
    /// 904B ladder in hpf.rs uses) puts the -3 dB point EXACTLY on
    /// `AMP_CORNER_HZ` at any rate; the plain `1 - exp(-w/sr)`
    /// discretization still drags the corner around by 5% between 44.1
    /// and 96 kHz.
    lp_a: f32,
}


/// Trapezoidal one-pole coefficient placing the -3 dB corner exactly on
/// `fc` at `sample_rate`. `fc` is held below Nyquist so `tan` cannot run
/// off to infinity at absurd rates.
#[inline]
fn tpt_lowpass_a(fc: f32, sample_rate: f32) -> f32 {
    if !(sample_rate > 0.0) {
        return 1.0;
    }
    let fc = fc.min(sample_rate * 0.45);
    let g = (std::f32::consts::PI * fc / sample_rate).tan();
    g / (1.0 + g)
}

impl NoiseSource {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            rng: 0x1234_5677,
            lp_s: 0.0,
            lp_a: tpt_lowpass_a(AMP_CORNER_HZ, sample_rate),
        }
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        let white = crate::rng::bipolar(&mut self.rng);
        // Avalanche-amplifier clipping: soft, slightly hot
        let x = white * 1.6;
        let clipped = x * (27.0 + x * x) / (27.0 + 9.0 * x * x);
        // Amplifier bandwidth rolloff, one pole at AMP_CORNER_HZ
        let v = (clipped - self.lp_s) * self.lp_a;
        let lp = v + self.lp_s;
        self.lp_s = lp + v;
        lp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_and_audible() {
        let mut noise = NoiseSource::new(44100.0);
        let mut peak = 0.0f32;
        let mut sum_sq = 0.0f32;
        for _ in 0..44100 {
            let s = noise.next();
            assert!(s.is_finite());
            peak = peak.max(s.abs());
            sum_sq += s * s;
        }
        let rms = (sum_sq / 44100.0).sqrt();
        assert!(peak <= 1.5, "noise out of range: {peak}");
        assert!(rms > 0.2, "noise too quiet: rms={rms}");
    }

    /// Impulse response of the realized rolloff, so the test measures the
    /// filter the audio actually goes through rather than re-deriving it.
    fn rolloff_impulse(sr: f32, n: usize) -> Vec<f32> {
        let a = tpt_lowpass_a(AMP_CORNER_HZ, sr);
        let mut s = 0.0f32;
        (0..n)
            .map(|i| {
                let x = if i == 0 { 1.0 } else { 0.0 };
                let v = (x - s) * a;
                let lp = v + s;
                s = lp + v;
                lp
            })
            .collect()
    }

    /// Magnitude response at `f` from that impulse response.
    fn rolloff_mag(sr: f32, f: f32) -> f32 {
        let h = rolloff_impulse(sr, 65536);
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, &x) in h.iter().enumerate() {
            let w = std::f32::consts::TAU * f * n as f32 / sr;
            re += x * w.cos();
            im -= x * w.sin();
        }
        (re * re + im * im).sqrt()
    }

    /// The amplifier's bandwidth is a component fact, not a per-sample
    /// number: the -3 dB corner must land on AMP_CORNER_HZ whatever the
    /// host runs at. The hardcoded 0.58 tracked the sample rate instead —
    /// 6.5 kHz at 44.1 k, 7.1 kHz at 48 k, 14.2 kHz at 96 k.
    #[test]
    fn the_amplifier_corner_holds_at_every_sample_rate() {
        for sr in [44100.0f32, 48000.0, 88200.0, 96000.0, 192000.0] {
            let dc = rolloff_mag(sr, 0.0);
            let target = dc / std::f32::consts::SQRT_2;
            // Bisect the measured response for its -3 dB point
            let (mut lo, mut hi) = (100.0f32, sr * 0.45);
            for _ in 0..40 {
                let mid = 0.5 * (lo + hi);
                if rolloff_mag(sr, mid) > target {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let corner = 0.5 * (lo + hi);
            assert!(
                (corner / AMP_CORNER_HZ - 1.0).abs() < 0.02,
                "at {sr} Hz the amplifier corner landed at {corner:.0} Hz, \
                 not {AMP_CORNER_HZ:.0} Hz"
            );
        }
    }

    /// ...and the audible form of the same fact: the shape of the rolloff
    /// through the audio band must be the same curve at every rate.
    #[test]
    fn the_rolloff_curve_does_not_track_the_sample_rate() {
        let curve = |sr: f32| -> Vec<f32> {
            let dc = rolloff_mag(sr, 0.0);
            [500.0f32, 1500.0, 3000.0, 6000.0, 9000.0]
                .iter()
                .map(|&f| rolloff_mag(sr, f) / dc)
                .collect()
        };
        let a = curve(44100.0);
        for sr in [48000.0f32, 96000.0] {
            let b = curve(sr);
            for (i, (x, y)) in a.iter().zip(&b).enumerate() {
                assert!(
                    (x / y - 1.0).abs() < 0.06,
                    "rolloff point {i} moved between 44.1 k and {sr}: {x:.4} vs {y:.4}"
                );
            }
        }
    }
}
