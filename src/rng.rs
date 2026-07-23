// The instrument's one pseudo-random source.
//
// Every stochastic thing in Patina draws from here: the 903A transistor
// noise, the 909's 4006 shift registers, oscillator and filter component
// tolerances, the tape deck's Barkhausen avalanches and dropout schedule,
// the BBD's hiss and per-voice detune.
//
// It is deliberately NOT `rand::thread_rng()`. Every one of those call
// sites runs on the audio thread, most of them once per sample, and
// `thread_rng` costs a thread-local lookup plus a `Rc` refcount touch and a
// 136-byte ChaCha block state per draw. A synthesizer that spends its
// real-time budget on TLS lookups gets its plugin killed by the host — so
// the rule in this codebase is: no thread-local anything below the audio
// callback. This is the generator that rule points at.
//
// The algorithm is Marsaglia's 32-bit xorshift (13, 17, 5): three shifts
// and three XORs, period 2^32-1, and it passes every randomness test that
// matters for noise you are going to lowpass and mix at -60 dBFS anyway.
// It was written out identically in five different modules before this one
// existed; unifying it is what keeps the five from drifting apart.

/// Advance the state and return the raw word. Never returns the same value
/// twice in a row within a period, and never reaches zero (a zero state is
/// a fixed point, which is why every seed is forced odd).
#[inline]
pub fn next_u32(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

/// Uniform in `[0, 1)`. 24 significant bits — exactly the f32 mantissa, so
/// every value is representable and the distribution is flat.
#[inline]
pub fn unipolar(state: &mut u32) -> f32 {
    (next_u32(state) >> 8) as f32 / (1u32 << 24) as f32
}

/// Uniform in `[-1, 1)`. The white-noise primitive.
#[inline]
pub fn bipolar(state: &mut u32) -> f32 {
    (next_u32(state) >> 8) as f32 / (1u32 << 23) as f32 - 1.0
}

/// Uniform in `[lo, hi)`.
#[inline]
pub fn range(state: &mut u32, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * unipolar(state)
}

/// Decorrelate a small integer (a voice index, a cutoff in Hz, a literal
/// written in the source) into a seed whose low bits are not neighbours'
/// low bits. Forces odd, because zero is xorshift's fixed point.
#[inline]
pub fn seed(n: u32) -> u32 {
    n.wrapping_mul(0x9E37_79B9) | 1
}

/// The same generator with the state carried inside, for the places that
/// want a value rather than a `&mut u32`. Identical arithmetic — it
/// delegates — so the two spellings can never drift.
#[derive(Clone, Copy)]
pub struct Rng(u32);

impl Rng {
    /// Seeds are used verbatim (only forced odd): call sites that need
    /// neighbouring seeds decorrelated should pass `seed(n)`.
    pub fn new(seed: u32) -> Self {
        Self(seed | 1)
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        next_u32(&mut self.0)
    }

    /// Uniform in `[0, 1)`.
    #[inline]
    pub fn unipolar(&mut self) -> f32 {
        unipolar(&mut self.0)
    }

    /// Uniform in `[-1, 1)`.
    #[inline]
    pub fn bipolar(&mut self) -> f32 {
        bipolar(&mut self.0)
    }

    /// Uniform in `[lo, hi)`.
    #[inline]
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        range(&mut self.0, lo, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_respected() {
        let mut s = seed(1);
        let (mut umin, mut umax) = (f32::MAX, f32::MIN);
        let (mut bmin, mut bmax) = (f32::MAX, f32::MIN);
        let (mut rmin, mut rmax) = (f32::MAX, f32::MIN);
        for _ in 0..200_000 {
            let u = unipolar(&mut s);
            let b = bipolar(&mut s);
            let r = range(&mut s, 0.002, 0.045);
            assert!((0.0..1.0).contains(&u), "unipolar out of range: {u}");
            assert!((-1.0..1.0).contains(&b), "bipolar out of range: {b}");
            assert!((0.002..0.045).contains(&r), "range out of range: {r}");
            umin = umin.min(u);
            umax = umax.max(u);
            bmin = bmin.min(b);
            bmax = bmax.max(b);
            rmin = rmin.min(r);
            rmax = rmax.max(r);
        }
        // and they actually cover the range they claim
        assert!(umin < 0.001 && umax > 0.999, "unipolar span {umin}..{umax}");
        assert!(bmin < -0.998 && bmax > 0.998, "bipolar span {bmin}..{bmax}");
        assert!(rmin < 0.0025 && rmax > 0.0445, "range span {rmin}..{rmax}");
    }

    /// The struct form must be bit-identical to the free-function form, or
    /// the two spellings have drifted and the unification is a lie.
    #[test]
    fn the_struct_and_the_free_functions_agree_bit_for_bit() {
        let mut raw = 0x0DDB_1A5Eu32 | 1;
        let mut wrapped = Rng::new(0x0DDB_1A5E);
        for _ in 0..10_000 {
            assert_eq!(next_u32(&mut raw), wrapped.next_u32());
            assert_eq!(unipolar(&mut raw), wrapped.unipolar());
            assert_eq!(bipolar(&mut raw), wrapped.bipolar());
            assert_eq!(range(&mut raw, -3.0, 7.0), wrapped.range(-3.0, 7.0));
        }
    }

    /// White noise, not a buzz: flat-ish mean, full-scale variance, and no
    /// short period.
    #[test]
    fn behaves_like_white_noise() {
        let mut s = seed(0xBEEF);
        let n = 1 << 20;
        let mut sum = 0.0f64;
        let mut sum_sq = 0.0f64;
        for _ in 0..n {
            let x = bipolar(&mut s) as f64;
            sum += x;
            sum_sq += x * x;
        }
        let mean = sum / n as f64;
        let rms = (sum_sq / n as f64).sqrt();
        assert!(mean.abs() < 0.01, "noise should be centred, mean {mean}");
        // uniform on [-1,1) has rms 1/sqrt(3) = 0.577
        assert!((rms - 0.577).abs() < 0.01, "noise rms {rms}");
    }

    #[test]
    fn a_zero_seed_still_generates() {
        let mut r = Rng::new(0);
        let first = r.next_u32();
        assert_ne!(first, 0);
        assert_ne!(r.next_u32(), first);
    }
}
