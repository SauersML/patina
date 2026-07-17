// Analog-style ADSR envelope.
//
// All segments are exponential (one-pole RC curves) like hardware envelopes:
// the attack charges toward an overshoot target so it stays punchy, and decay
// and release approach their targets asymptotically, which reads to the ear
// as far more natural than linear ramps.
//
// The overshoot target is a circuit fact, and it differs by machine:
//   Moog 911: RC charge with the comparator set so the curve keeps its
//   exponential knee — target 1.25 (validated against the Polymoog
//   factory contour windows).
//   ARP 4020 (2600 service manual 2.5.3): C1/C2 charge from Q6's LATCHED
//   collector (near the +15 rail, less drops, ~ +14 V) and the attack
//   terminates when the cap crosses +10 V — target ~1.38, so the ARP
//   attack rises more linearly and hits its top harder.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::oscillator::CircuitModel;

#[derive(Clone, Copy, PartialEq)]
pub enum EnvelopeStage {
    Attack,
    Decay,
    Sustain,
    Release,
    Idle,
}

pub struct Envelope {
    attack: AtomicU32,
    decay: AtomicU32,
    sustain: AtomicU32,
    release: AtomicU32,
    /// Attack charges toward this level but transitions to Decay at 1.0 —
    /// the analog "fast at first, then curving" shape. Per-circuit.
    overshoot: AtomicU32,
    stage: EnvelopeStage,
    current_level: f32,
    sample_rate: f32,
}

impl Envelope {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            attack: AtomicU32::new(0.1f32.to_bits()),
            decay: AtomicU32::new(0.1f32.to_bits()),
            sustain: AtomicU32::new(0.7f32.to_bits()),
            release: AtomicU32::new(0.2f32.to_bits()),
            overshoot: AtomicU32::new(1.25f32.to_bits()),
            stage: EnvelopeStage::Idle,
            current_level: 0.0,
            sample_rate,
        }
    }

    /// Select the envelope circuit: 911 (Moog) or 4020 (ARP).
    pub fn set_circuit(&self, model: CircuitModel) {
        let target: f32 = match model {
            CircuitModel::Moog => 1.25,
            CircuitModel::Arp => 1.38,
        };
        self.overshoot.store(target.to_bits(), Ordering::Relaxed);
    }

    /// One-pole coefficient that covers the segment in roughly `time` seconds.
    #[inline]
    fn coef(&self, time: f32, speed: f32) -> f32 {
        1.0 - (-speed / (time.max(0.0005) * self.sample_rate)).exp()
    }

    pub fn next_sample(&mut self) -> f32 {
        match self.stage {
            EnvelopeStage::Attack => {
                let attack_time = f32::from_bits(self.attack.load(Ordering::Relaxed));
                let target = f32::from_bits(self.overshoot.load(Ordering::Relaxed));
                // speed = ln(target / (target - 1)) so 0 -> 1 takes ~attack_time
                let speed = (target / (target - 1.0)).ln();
                let k = self.coef(attack_time, speed);
                self.current_level += (target - self.current_level) * k;
                if self.current_level >= 1.0 {
                    self.current_level = 1.0;
                    self.stage = EnvelopeStage::Decay;
                }
            }
            EnvelopeStage::Decay => {
                let decay_time = f32::from_bits(self.decay.load(Ordering::Relaxed));
                let sustain_level = f32::from_bits(self.sustain.load(Ordering::Relaxed));
                let k = self.coef(decay_time, 4.0);
                self.current_level += (sustain_level - self.current_level) * k;
                if (self.current_level - sustain_level).abs() < 1e-4 {
                    self.current_level = sustain_level;
                    self.stage = EnvelopeStage::Sustain;
                }
            }
            EnvelopeStage::Sustain => {
                // Track the sustain control smoothly so live tweaks don't step
                let sustain_level = f32::from_bits(self.sustain.load(Ordering::Relaxed));
                self.current_level += (sustain_level - self.current_level) * 0.005;
            }
            EnvelopeStage::Release => {
                let release_time = f32::from_bits(self.release.load(Ordering::Relaxed));
                let k = self.coef(release_time, 4.0);
                self.current_level -= self.current_level * k;
                if self.current_level < 1e-4 {
                    self.current_level = 0.0;
                    self.stage = EnvelopeStage::Idle;
                }
            }
            EnvelopeStage::Idle => {
                self.current_level = 0.0;
            }
        }
        self.current_level
    }

    /// Retriggers from the current level, so restarts are click-free.
    pub fn note_on(&mut self) {
        self.stage = EnvelopeStage::Attack;
    }

    pub fn note_off(&mut self) {
        if self.stage != EnvelopeStage::Idle {
            self.stage = EnvelopeStage::Release;
        }
    }

    pub fn set_attack(&self, attack: f32) {
        self.attack.store(attack.to_bits(), Ordering::Relaxed);
    }

    pub fn set_decay(&self, decay: f32) {
        self.decay.store(decay.to_bits(), Ordering::Relaxed);
    }

    pub fn set_sustain(&self, sustain: f32) {
        self.sustain.store(sustain.to_bits(), Ordering::Relaxed);
    }

    pub fn set_release(&self, release: f32) {
        self.release.store(release.to_bits(), Ordering::Relaxed);
    }

    pub fn is_idle(&self) -> bool {
        self.stage == EnvelopeStage::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Polymoog factory contour spec is a usable tolerance for RC
    /// envelopes: "attack to 7.5 VDC +/-1.0 VDC in 33 ms +/-5 ms". Our
    /// attack set to 33 ms must reach full level inside a comparable
    /// window (widened for the different attack topology).
    #[test]
    fn attack_lands_in_the_factory_window() {
        let sr = 44100.0;
        let mut env = Envelope::new(sr);
        env.set_attack(0.033);
        env.note_on();
        let mut samples = 0usize;
        while env.next_sample() < 0.999 {
            samples += 1;
            assert!(samples < 4410, "attack never completed");
        }
        let ms = samples as f32 / sr * 1000.0;
        assert!(
            (20.0..=50.0).contains(&ms),
            "33 ms attack should complete in roughly 33 ms, took {ms:.1} ms"
        );
    }

    /// The 4020 charges toward a higher rail (+14 latched vs the 911's
    /// curve): with the SAME time-to-peak, the ARP attack is more linear
    /// (lower at the midpoint), landing its top harder.
    #[test]
    fn arp_attack_is_more_linear_than_moog() {
        let sr = 44100.0;
        let mid_level = |model: CircuitModel| -> f32 {
            let mut e = Envelope::new(sr);
            e.set_circuit(model);
            e.set_attack(0.1);
            e.note_on();
            let mut level = 0.0;
            for _ in 0..(0.05 * sr) as usize {
                level = e.next_sample();
            }
            level
        };
        let moog = mid_level(CircuitModel::Moog);
        let arp = mid_level(CircuitModel::Arp);
        assert!(
            arp < moog - 0.01,
            "ARP attack should sag below Moog at the midpoint: arp {arp:.3}, moog {moog:.3}"
        );
    }

    /// Release must be exponential (RC), not linear: after one time
    /// constant the level sits near 1/e of the start, not on a straight
    /// line to zero.
    #[test]
    fn release_is_exponential() {
        let sr = 44100.0;
        let mut env = Envelope::new(sr);
        env.set_attack(0.001);
        env.set_sustain(1.0);
        env.set_release(0.4);
        env.note_on();
        for _ in 0..8820 {
            env.next_sample();
        }
        env.note_off();
        // Our release coefficient spans the set time with speed 4.0, so
        // one time constant is release/4 = 100 ms
        let tau_samples = (0.4 / 4.0 * sr) as usize;
        let mut level = 1.0;
        for _ in 0..tau_samples {
            level = env.next_sample();
        }
        assert!(
            (0.25..=0.5).contains(&level),
            "after one tau the level should be ~1/e, got {level}"
        );
    }
}
