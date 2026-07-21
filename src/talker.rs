// The Talker — a formant-tracking filter, which is what a talk box
// actually is. Not a channel vocoder.
//
// A channel vocoder chops both signals into bands and gates one with
// the other; it always carries a faint filterbank shimmer. The real
// thing — a Heil tube in the mouth, or the DigiTech Talker's DSP — is a
// SINGLE time-varying filter: the mouth's resonances, imposed whole on
// the instrument. One throat shape, sliding continuously, on one note.
//
// The engineering here is linear predictive coding, the same mathematics
// inside the TI TMS5220 (Speak & Spell) and every LPC-10 radio codec:
//
//   modulator ──► 25 ms frames ──► autocorrelation ──► Levinson-Durbin
//                                   └► reflection coefficients k1..k16
//   carrier ──► all-pole LATTICE filter (the k's, slewed) ──► VCA ──► out
//
// The lattice form matters: reflection coefficients can be interpolated
// freely and the filter stays stable as long as every |k| < 1 — which is
// exactly what a vocal tract (a lossy tube) guarantees. Direct-form LPC
// coefficients explode when you morph them; lattices are how the
// hardware did it, and how we do it.

use crate::noise::NoiseSource;

/// Prediction order: 16 poles at 48 kHz resolves 4-5 formants plus
/// spectral tilt, the LPC-vocoder standard for full-band speech.
const ORDER: usize = 16;

/// Analysis window 25 ms, new coefficients every 10 ms — phoneme-rate.
const WINDOW_S: f32 = 0.025;
const HOP_S: f32 = 0.010;

pub struct Talker {
    sample_rate: f32,
    // Modulator history for analysis
    ring: Vec<f32>,
    write: usize,
    hop: usize,
    since_hop: usize,
    // Reflection coefficients: analysis targets and the slewed live set
    k_target: [f32; ORDER],
    k: [f32; ORDER],
    k_slew: f32,
    // Lattice state
    b: [f32; ORDER + 1],
    // Loudness follower (the modulator's energy opens the VCA)
    env_target: f32,
    env: f32,
    attack: f32,
    release: f32,
    // Fricative path: modulator HF fraction blends noise into the carrier
    m_lp: f32,
    hf_env: f32,
    lf_env: f32,
    unvoiced: f32,
    noise: NoiseSource,
}

impl Talker {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            ring: vec![0.0; (WINDOW_S * sample_rate) as usize + 1],
            write: 0,
            hop: (HOP_S * sample_rate) as usize,
            since_hop: 0,
            k_target: [0.0; ORDER],
            k: [0.0; ORDER],
            k_slew: 1.0 - (-1.0 / (0.004 * sample_rate)).exp(),
            b: [0.0; ORDER + 1],
            env_target: 0.0,
            env: 0.0,
            attack: 1.0 - (-1.0 / (0.002 * sample_rate)).exp(),
            release: 1.0 - (-1.0 / (0.015 * sample_rate)).exp(),
            m_lp: 0.0,
            hf_env: 0.0,
            lf_env: 0.0,
            unvoiced: 0.0,
            noise: NoiseSource::new(),
        }
    }

    /// Frame analysis: windowed autocorrelation to ORDER lags, then
    /// Levinson-Durbin recursion for the reflection coefficients.
    fn analyze(&mut self) {
        let n = self.ring.len();
        // Unwrap the ring into time order, pre-emphasized (whitens the
        // glottal tilt so the poles chase formants, not the source) and
        // Hann-windowed
        let mut frame = vec![0.0f32; n];
        let mut prev = 0.0f32;
        for i in 0..n {
            let x = self.ring[(self.write + i) % n];
            let pe = x - 0.97 * prev;
            prev = x;
            let w = 0.5
                - 0.5 * (std::f32::consts::TAU * i as f32 / (n - 1) as f32).cos();
            frame[i] = pe * w;
        }

        let mut r = [0.0f32; ORDER + 1];
        for (lag, r_lag) in r.iter_mut().enumerate() {
            let mut acc = 0.0;
            for i in lag..n {
                acc += frame[i] * frame[i - lag];
            }
            *r_lag = acc;
        }
        // Silence (or near-silence): relax the tract to neutral
        if r[0] < 1e-7 {
            self.k_target = [0.0; ORDER];
            self.env_target = 0.0;
            return;
        }
        // A whisper of white noise on the diagonal keeps the recursion
        // conditioned (the fixed-point hardware did the same trick)
        r[0] *= 1.0001;

        // Levinson-Durbin, keeping the reflection coefficients
        let mut a = [0.0f32; ORDER + 1];
        let mut e = r[0];
        for i in 1..=ORDER {
            let mut acc = r[i];
            for j in 1..i {
                acc -= a[j] * r[i - j];
            }
            let ki = (acc / e).clamp(-0.985, 0.985);
            let mut new_a = a;
            new_a[i] = ki;
            for j in 1..i {
                new_a[j] = a[j] - ki * a[i - j];
            }
            a = new_a;
            e *= 1.0 - ki * ki;
            self.k_target[i - 1] = ki;
        }

        // Loudness from the frame RMS of the raw (un-emphasized) signal
        let mut energy = 0.0;
        for i in 0..n {
            let x = self.ring[(self.write + i) % n];
            energy += x * x;
        }
        self.env_target = (energy / n as f32).sqrt();
    }

    /// One sample: push the modulator, and play the carrier through the
    /// mouth. Carrier in program volts; output in program volts.
    #[inline]
    pub fn process(&mut self, modulator: f32, carrier: f32) -> f32 {
        self.ring[self.write] = modulator;
        self.write = (self.write + 1) % self.ring.len();
        self.since_hop += 1;
        if self.since_hop >= self.hop {
            self.since_hop = 0;
            self.analyze();
        }

        // Fricative detector: HF share of the modulator (one-pole split
        // at ~3 kHz) blends noise into the carrier — consonants are
        // breath, and a low saw has nothing up there to shape
        let klp = 1.0 - (-std::f32::consts::TAU * 3000.0 / self.sample_rate).exp();
        self.m_lp += klp * (modulator - self.m_lp);
        let hf = modulator - self.m_lp;
        self.hf_env += 0.002 * (hf.abs() - self.hf_env);
        self.lf_env += 0.002 * (modulator.abs() - self.lf_env);
        let target = if self.hf_env > 0.55 * self.lf_env.max(1e-6) { 0.85 } else { 0.0 };
        self.unvoiced += (target - self.unvoiced) * (250.0 / self.sample_rate).min(1.0);

        // Slew the live tract toward the analysis targets, VCA follower too
        for i in 0..ORDER {
            self.k[i] += (self.k_target[i] - self.k[i]) * self.k_slew;
        }
        let ke = if self.env_target > self.env { self.attack } else { self.release };
        self.env += (self.env_target - self.env) * ke;

        // The carrier (plus consonant breath) through the lattice tract
        let x = carrier * (1.0 - self.unvoiced) + self.noise.next() * 4.0 * self.unvoiced;
        let mut f = x * 0.25; // headroom: the poles ring hard at formants
        for i in (0..ORDER).rev() {
            f += self.k[i] * self.b[i];
            // The 0.9995 is wall loss: a real tract is lossy, and it keeps
            // the lattice bounded while coefficients morph mid-frame
            self.b[i + 1] = (self.b[i] - self.k[i] * f) * 0.9995;
        }
        self.b[0] = f;

        // The mouth's loudness opens the VCA; soft OTA limit on the way out
        (f * self.env * 2.2).tanh() * 3.2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defining behavior: the modulator's resonance must appear ON
    /// the carrier, and a silent mouth passes nothing.
    #[test]
    fn tracks_formants_onto_the_carrier() {
        let sr = 48000.0;
        let mut t = Talker::new(sr);
        // Modulator: noise through a strong 900 Hz two-pole "mouth"
        let (mut y1, mut y2) = (0.0f32, 0.0f32);
        let c = -(-std::f32::consts::TAU * 120.0 / sr).exp();
        let bcoef = 2.0 * (-std::f32::consts::PI * 120.0 / sr).exp()
            * (std::f32::consts::TAU * 900.0 / sr).cos();
        let a = 1.0 - bcoef - c;
        let mut noise = NoiseSource::new();
        let mut out = Vec::with_capacity(sr as usize);
        for n in 0..(sr as usize) {
            let m = {
                let y = a * noise.next() + bcoef * y1 + c * y2;
                y2 = y1;
                y1 = y;
                y * 0.5
            };
            // Carrier: 110 Hz saw, program volts
            let cwave = (((n as f32 * 110.0 / sr) % 1.0) * 2.0 - 1.0) * 5.0;
            out.push(t.process(m, cwave));
        }
        assert!(out.iter().all(|s| s.is_finite()), "lattice must stay stable");
        let goertzel = |freq: f32| -> f32 {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, &s) in out[out.len() / 2..].iter().enumerate() {
                let ph = std::f32::consts::TAU * freq * i as f32 / sr;
                re += s * ph.cos();
                im += s * ph.sin();
            }
            (re * re + im * im).sqrt()
        };
        // Carrier harmonics near the mouth's 900 Hz resonance must beat
        // harmonics far from it (2970 Hz) decisively
        let near = goertzel(880.0);
        let far = goertzel(2970.0);
        assert!(
            near > 4.0 * far,
            "the tracked formant should shape the carrier: near={near}, far={far}"
        );

        // Silent modulator: after the VCA's release tail (~15 ms) has
        // rung out, the carrier must be gone
        let mut quiet = 0.0f32;
        for n in 0..(sr as usize / 4) {
            let cwave = (((n as f32 * 110.0 / sr) % 1.0) * 2.0 - 1.0) * 5.0;
            let s = t.process(0.0, cwave).abs();
            if n > (0.15 * sr) as usize {
                quiet = quiet.max(s);
            }
        }
        assert!(quiet < 0.05, "silent mouth must mute the carrier, got {quiet}");
    }
}
