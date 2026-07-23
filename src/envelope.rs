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
    /// Voice-steal discharge: the assigner grounds the timing cap through
    /// a saturated transistor before gating the new note's attack. A
    /// saturated switch is a near-constant-current sink, so this segment
    /// is LINEAR (unlike every other segment, which is RC) and arrives at
    /// exactly zero in a bounded number of samples.
    Steal,
    Attack,
    Decay,
    Sustain,
    Release,
    Idle,
}

/// How long the discharge gate is held on when a card is reassigned to a
/// different note. Short enough to be a transient rather than a hole,
/// long enough that a full-level voice does not step to zero (a step is
/// the click; a 2.5 ms slope is not).
const STEAL_SECONDS: f32 = 0.0025;
/// Below this the cap is empty enough that the attack can start straight
/// away — no audible discontinuity to smooth over.
const STEAL_FLOOR: f32 = 1e-3;

/// How fast the Sustain stage chases a moved sustain control. A TIME, so
/// live sustain automation ramps the same way at every host rate.
const SUSTAIN_TRACK_TAU_S: f32 = 0.0045;

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
    /// Per-sample discharge step while in `Steal` (constant current).
    steal_step: f32,
    /// Sustain-tracking coefficient for `SUSTAIN_TRACK_TAU_S` at this rate.
    sustain_track_k: f32,
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
            steal_step: 0.0,
            sustain_track_k: crate::voice::smoothing_coef(SUSTAIN_TRACK_TAU_S, sample_rate),
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
            EnvelopeStage::Steal => {
                self.current_level -= self.steal_step;
                if self.current_level <= 0.0 {
                    self.current_level = 0.0;
                    self.stage = EnvelopeStage::Attack;
                }
            }
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
                self.current_level +=
                    (sustain_level - self.current_level) * self.sustain_track_k;
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

    /// Same-note retrigger on a card that is still gated: the timing cap
    /// keeps its charge and the attack resumes from there, exactly as a
    /// hardware ADSR does when the gate is re-struck. Click-free, and the
    /// partial re-attack is the analog behaviour — but it is ONLY right
    /// when the card keeps the note it already has. Use `note_on_stolen`
    /// whenever a card is reassigned.
    pub fn note_on(&mut self) {
        self.stage = EnvelopeStage::Attack;
    }

    /// A card being handed a DIFFERENT note (voice steal, or a re-press
    /// on a card still ringing out its release). Restarting from the
    /// leftover level would skip the attack entirely — a slow-attack
    /// patch would fire the second note instantly — so the assigner
    /// discharges the cap first. Zeroing it outright would step the VCA
    /// and click, so the discharge is a short linear ramp; only when the
    /// cap is already near-empty does the attack begin immediately.
    pub fn note_on_stolen(&mut self) {
        if self.current_level > STEAL_FLOOR {
            self.steal_step =
                self.current_level / (STEAL_SECONDS * self.sample_rate).max(1.0);
            self.stage = EnvelopeStage::Steal;
        } else {
            self.current_level = 0.0;
            self.stage = EnvelopeStage::Attack;
        }
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

    /// A card handed a DIFFERENT note must replay its attack. Restarting
    /// from the leftover charge made a 500 ms attack fire instantly on
    /// every stolen/reassigned voice.
    #[test]
    fn a_reassigned_card_replays_its_attack() {
        let sr = 48000.0;
        let mut env = Envelope::new(sr);
        env.set_attack(0.5);
        env.set_decay(2.0);
        env.set_sustain(1.0);
        env.note_on();
        let mut level = 0.0;
        for _ in 0..sr as usize {
            level = env.next_sample();
        }
        assert!(level > 0.99, "first note should be at full level, got {level}");

        env.note_on_stolen();
        // The discharge gate empties the cap inside a few milliseconds
        let mut emptied_at = None;
        for n in 0..(0.01 * sr) as usize {
            if env.next_sample() <= 0.0 {
                emptied_at = Some(n);
                break;
            }
        }
        let n = emptied_at.expect("steal discharge never reached zero");
        let ms = n as f32 / sr * 1000.0;
        assert!(ms < 4.0, "steal discharge took {ms:.2} ms");

        // ...and THEN the 500 ms attack runs in full. Before the fix the
        // level here was already pinned at 1.0.
        for _ in 0..(0.05 * sr) as usize {
            level = env.next_sample();
        }
        assert!(
            level < 0.35,
            "a reassigned card must replay its slow attack, level {level} 50 ms in"
        );
        assert!(level > 0.05, "...but it must be rising, level {level}");
    }

    /// The discharge is a ramp, not a reset: zeroing a loud voice outright
    /// is exactly the click the "retrigger from the current level" comment
    /// was avoiding.
    #[test]
    fn the_steal_discharge_slews_instead_of_stepping() {
        let sr = 48000.0;
        let mut env = Envelope::new(sr);
        env.set_attack(0.5);
        env.set_sustain(1.0);
        let mut level = 0.0;
        env.set_attack(0.001);
        env.note_on();
        for _ in 0..(0.1 * sr) as usize {
            level = env.next_sample();
        }
        assert!(level > 0.99, "expected a full-level voice, got {level}");
        env.set_attack(0.5); // slow, so post-discharge steps stay small too
        env.note_on_stolen();
        let mut worst = 0.0f32;
        let mut prev = level;
        for _ in 0..(0.005 * sr) as usize {
            let l = env.next_sample();
            worst = worst.max((l - prev).abs());
            prev = l;
        }
        // 1.0 discharged over 2.5 ms at 48 kHz is 1/120 per sample
        assert!(worst < 0.01, "steal must slew, not step: worst jump {worst}");
    }

    /// The deliberate behaviour, pinned: a card that KEEPS its note and is
    /// still gated re-attacks from the charge already on the cap (the
    /// hardware retrigger). It must not be "fixed" into a discharge.
    #[test]
    fn a_held_retrigger_still_resumes_from_the_cap() {
        let sr = 48000.0;
        let mut env = Envelope::new(sr);
        env.set_attack(0.5);
        env.set_sustain(1.0);
        env.note_on();
        let mut level = 0.0;
        for _ in 0..(0.2 * sr) as usize {
            level = env.next_sample();
        }
        assert!(level > 0.3, "expected a partly charged cap, got {level}");
        env.note_on();
        let after = env.next_sample();
        assert!(
            after >= level,
            "a held retrigger must not discharge the cap: {level} -> {after}"
        );
    }

    /// The Sustain stage chases a moved sustain control over a fixed
    /// TIME. As a bare per-sample coefficient the same automation ramped
    /// in 4.5 ms at 44.1 kHz and 2.1 ms at 96 kHz.
    #[test]
    fn sustain_tracking_takes_the_same_time_at_every_rate() {
        let tau_ms = |sr: f32| -> f32 {
            let mut env = Envelope::new(sr);
            env.set_attack(0.001);
            env.set_decay(0.001);
            env.set_sustain(1.0);
            env.note_on();
            for _ in 0..(0.2 * sr) as usize {
                env.next_sample();
            }
            // Now sitting in Sustain; move the control and time the ramp
            env.set_sustain(0.0);
            let mut n = 0usize;
            while env.next_sample() > 1.0 / std::f32::consts::E {
                n += 1;
                assert!(n < sr as usize, "the sustain tracker never moved");
            }
            n as f32 / sr * 1000.0
        };
        let a = tau_ms(44100.0);
        let b = tau_ms(96000.0);
        assert!(
            (a / (SUSTAIN_TRACK_TAU_S * 1000.0) - 1.0).abs() < 0.05
                && (a / b - 1.0).abs() < 0.05,
            "sustain tracking took {a:.2} ms at 44.1 kHz and {b:.2} ms at \
             96 kHz, expected {:.2} ms at both",
            SUSTAIN_TRACK_TAU_S * 1000.0
        );
    }
}
