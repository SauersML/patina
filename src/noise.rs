// Shared noise source, per the 903A and the Juno-106 architecture: ONE
// transistor noise generator feeds every voice, so noise in chords and
// unison stays correlated instead of six independent random streams
// (which sounds wider and more diffuse than the hardware).
//
// The ARP 2600 manual describes the source as "an amplified, reversed
// junction of a selected transistor" followed by clipping — so: white
// noise, soft transistor clipping, then a gentle high-frequency rolloff
// from the amplifier stage.

pub struct NoiseSource {
    rng: u32,
    lp: f32,
}

#[inline]
fn xorshift(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

impl NoiseSource {
    pub fn new() -> Self {
        Self { rng: 0x1234_5677, lp: 0.0 }
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        let white = (xorshift(&mut self.rng) >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
        // Avalanche-amplifier clipping: soft, slightly hot
        let x = white * 1.6;
        let clipped = x * (27.0 + x * x) / (27.0 + 9.0 * x * x);
        // Amplifier bandwidth rolloff (~6 kHz one-pole)
        self.lp += 0.58 * (clipped - self.lp);
        self.lp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_and_audible() {
        let mut noise = NoiseSource::new();
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
}
