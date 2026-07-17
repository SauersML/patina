// Analog-style ADSR envelope.
//
// All segments are exponential (one-pole RC curves) like hardware envelopes:
// the attack charges toward an overshoot target so it stays punchy, and decay
// and release approach their targets asymptotically, which reads to the ear
// as far more natural than linear ramps.

use std::sync::atomic::{AtomicU32, Ordering};

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
    stage: EnvelopeStage,
    current_level: f32,
    sample_rate: f32,
}

/// Attack charges toward this level but transitions to Decay at 1.0,
/// keeping the analog "fast at first, then curving" shape.
const ATTACK_TARGET: f32 = 1.25;

impl Envelope {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            attack: AtomicU32::new(0.1f32.to_bits()),
            decay: AtomicU32::new(0.1f32.to_bits()),
            sustain: AtomicU32::new(0.7f32.to_bits()),
            release: AtomicU32::new(0.2f32.to_bits()),
            stage: EnvelopeStage::Idle,
            current_level: 0.0,
            sample_rate,
        }
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
                // ln(ATTACK_TARGET / (ATTACK_TARGET - 1.0)) so 0 -> 1 takes ~attack_time
                let k = self.coef(attack_time, 1.61);
                self.current_level += (ATTACK_TARGET - self.current_level) * k;
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
