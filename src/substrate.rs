// The Substrate: chassis-level modeling.
//
// A hardware synthesizer is not a set of independent processors — it is one
// electrical object. Every module hangs off the same power supply, sits on
// the same warming chassis, and shares copper with its neighbors. This
// module models that shared physical environment as THREE states that every
// voice reads and (through its current draw and dissipation) writes:
//
//   RAIL     The Moog supply spec is +/-0.075% regulation with 5 mV p-p
//            ripple, and the 904A blueprint draws the local rail filter
//            (10 ohm / 100 uF) right on the schematic. The rail is modeled
//            as a regulated source behind that output impedance: summed
//            voice current sags it (tau ~ 1 ms), and mains ripple rides on
//            it. The rail feeds every expo converter, so sag and ripple
//            become CORRELATED pitch/cutoff movement — a hard bass
//            transient microscopically flattens everything at once.
//
//   HEAT     The service manuals demand a 30-minute oscillator warm-up and
//            call VCF warm-up "mandatory". Chassis temperature is a state:
//            the instrument powers on slightly flat with filters low and
//            settles exponentially; playing adds dissipation heat with a
//            long time constant. Each voice applies its own thermal
//            sensitivity, so the bank's tuning audibly converges during
//            the first minutes — the alignment target being reached.
//
//   BOARD    Adjacent voice cards couple capacitively (handled in the
//            voice manager: each voice receives its neighbor's
//            DIFFERENTIATED pre-filter signal at ~-64 dB, because
//            capacitors differentiate), and the summing bus has the finite
//            slew rate of the discrete Model 3 mixer, softening only the
//            very fastest, hottest edges (transient intermodulation).
//
// Epistemic status, honestly: the MECHANISMS here are physical and the
// rail/ripple magnitudes come from the published supply spec, but the
// COUPLING COEFFICIENTS (how much rail movement survives the compensated
// expo converters, the cold-start detune, crosstalk level) are physically
// plausible conjecture pending measurement against hardware. They are kept
// deliberately small — well under a cent, ~-64 dB — so that if they are
// wrong, they are wrong quietly. Validating them requires a real 900-series
// system and a day with a spectrum analyzer; the constants below document
// exactly what to measure.

use std::f32::consts::TAU;

/// Fractional rail sag at "full" program current (0.075% regulation window).
const SAG_FULL: f32 = 0.00075;
/// Ripple: 5 mV p-p on a 12 V rail.
const RIPPLE_FRAC: f32 = 2.5e-3 / 12.0;
const RIPPLE_HZ: f32 = 120.0;
/// How much of rail movement survives the (partially supply-compensated,
/// per US 3,991,645) exponential converters, in octaves per fractional volt.
const RAIL_TO_PITCH_OCT: f32 = 0.35;
/// Cold-start detune in cents (settles to 0 as the chassis warms).
const COLD_PITCH_CENTS: f32 = 4.0;
/// Cold-start filter cutoff deficit in octaves (VCFs drift more than VCOs).
const COLD_CUTOFF_OCT: f32 = 0.18;
/// Chassis warm-up time constant, seconds (~settled inside ten minutes).
const WARMUP_TAU: f64 = 150.0;
/// Dissipation heating from playing: slow and slight.
const PLAY_HEAT_TAU: f64 = 60.0;
const PLAY_HEAT_CENTS: f32 = 0.8;

pub struct Substrate {
    sample_rate: f32,
    sag: f32, // fractional rail sag, smoothed
    sag_a: f32,
    ripple_phase: f32,
    /// 0 = cold power-on, 1 = fully warmed. f64: its per-sample increment
    /// is far below f32 resolution.
    warmth: f64,
    warm_k: f64,
    /// Dissipation heat from sustained playing, 0..1-ish.
    play_heat: f64,
    heat_k: f64,
}

/// What the rest of the instrument reads back, once per sample.
#[derive(Clone, Copy)]
pub struct SubstrateState {
    /// Multiplier on every oscillator frequency (correlated component;
    /// voices scale it by their own sensitivity).
    pub pitch_mult: f32,
    /// Added to every filter's cutoff modulation, in octaves.
    pub cutoff_oct: f32,
}

impl Substrate {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            sag: 0.0,
            // 10 ohm / 100 uF local rail filter: tau = 1 ms
            sag_a: 1.0 - (-1.0 / (0.001 * sample_rate)).exp(),
            ripple_phase: 0.0,
            warmth: 0.0,
            warm_k: 1.0 / (WARMUP_TAU * sample_rate as f64),
            play_heat: 0.0,
            heat_k: 1.0 / (PLAY_HEAT_TAU * sample_rate as f64),
        }
    }

    /// Step the environment. `current_proxy` is the summed magnitude of the
    /// voice outputs from the PREVIOUS sample — the rail responds to the
    /// current that was just drawn, a genuine one-sample-lagged feedback
    /// loop through the power supply.
    pub fn step(&mut self, current_proxy: f32) -> SubstrateState {
        // Rail sag toward the load, through the local RC
        let target = SAG_FULL * current_proxy.min(4.0) * 0.25;
        self.sag += self.sag_a * (target - self.sag);

        self.ripple_phase += RIPPLE_HZ / self.sample_rate;
        if self.ripple_phase >= 1.0 {
            self.ripple_phase -= 1.0;
        }
        // Ripple depth grows slightly under load, like a real bridge supply
        let ripple = (TAU * self.ripple_phase).sin()
            * RIPPLE_FRAC
            * (0.5 + 2.0 * self.sag / SAG_FULL.max(1e-9) * 0.25);

        // Chassis heat: exponential warm-up plus slight dissipation heating
        self.warmth += (1.0 - self.warmth) * self.warm_k;
        self.play_heat += ((current_proxy.min(2.0) as f64) * 0.5 - self.play_heat) * self.heat_k;

        let cold = (1.0 - self.warmth) as f32;
        let heat_cents = self.play_heat as f32 * PLAY_HEAT_CENTS;
        let pitch_cents = -COLD_PITCH_CENTS * cold - heat_cents;

        let rail_oct = -(self.sag + ripple) * RAIL_TO_PITCH_OCT;
        let pitch_oct = pitch_cents / 1200.0 + rail_oct;

        SubstrateState {
            // exp2 via the cheap identity for tiny x: 2^x ~= 1 + x ln2
            pitch_mult: 1.0 + pitch_oct * std::f32::consts::LN_2,
            cutoff_oct: -COLD_CUTOFF_OCT * cold + rail_oct * 2.0,
        }
    }

    /// Test hook: jump the chassis to fully warmed.
    pub fn force_warm(&mut self) {
        self.warmth = 1.0;
    }
}

/// Finite slew rate of the discrete Model 3 summing amplifier. Only the
/// fastest, hottest edges are touched — the mechanism behind transient
/// intermodulation softening in hardware mixers.
pub struct SlewLimiter {
    state: f32,
    max_step: f32,
}

impl SlewLimiter {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            state: 0.0,
            // The 1458's 0.5 V/us at 10 V full scale, in FS per sample.
            // At audio sample rates this only engages on multi-voice
            // summed transients — exactly when hardware TIM appears.
            max_step: 50_000.0 / sample_rate,
        }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let delta = (x - self.state).clamp(-self.max_step, self.max_step);
        self.state += delta;
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rail_sags_under_load_and_recovers() {
        let mut sub = Substrate::new(44100.0);
        sub.force_warm();
        // Quiet: pitch at unity
        let mut quiet = sub.step(0.0);
        for _ in 0..4410 {
            quiet = sub.step(0.0);
        }
        // Slam it: pitch dips below the quiet value (rail sag flattens pitch)
        let mut loud = quiet;
        for _ in 0..441 {
            loud = sub.step(3.0);
        }
        assert!(
            loud.pitch_mult < quiet.pitch_mult,
            "rail sag should flatten pitch: {} vs {}",
            loud.pitch_mult,
            quiet.pitch_mult
        );
        // The whole effect stays microscopic (well under a cent of range)
        assert!((quiet.pitch_mult - loud.pitch_mult).abs() < 0.001);
        // Release the load: recovers toward the quiet value
        let mut rec = loud;
        for _ in 0..4410 {
            rec = sub.step(0.0);
        }
        assert!(rec.pitch_mult > loud.pitch_mult);
    }

    #[test]
    fn chassis_warms_up_and_tuning_converges() {
        let sr = 1000.0; // fast virtual clock for the test
        let mut sub = Substrate::new(sr);
        let cold = sub.step(0.0);
        // Cold start is flat and the filter sits low
        assert!(cold.pitch_mult < 1.0);
        assert!(cold.cutoff_oct < -0.05);
        // Simulate ~20 minutes
        let mut warmed = cold;
        for _ in 0..(1200.0 * sr) as usize {
            warmed = sub.step(0.0);
        }
        assert!(
            warmed.pitch_mult > cold.pitch_mult && warmed.pitch_mult > 0.9995,
            "tuning should converge as the chassis warms: {} -> {}",
            cold.pitch_mult,
            warmed.pitch_mult
        );
        assert!(warmed.cutoff_oct.abs() < 0.02);
    }

    #[test]
    fn slew_limiter_bounds_edges_but_passes_audio() {
        let sr = 44100.0;
        let mut slew = SlewLimiter::new(sr);
        // A hot multi-voice summed transient cannot arrive in one sample...
        let y = slew.process(3.0);
        assert!(y < 3.0 && y > 0.0);
        // ...but ordinary audio-rate content passes unchanged
        let mut slew = SlewLimiter::new(sr);
        let mut max_err = 0.0f32;
        for n in 0..4410 {
            let x = (TAU * 5000.0 * n as f32 / sr).sin() * 0.8;
            let y = slew.process(x);
            if n > 10 {
                max_err = max_err.max((y - x).abs());
            }
        }
        assert!(max_err < 1e-3, "5 kHz sine should pass cleanly, err={max_err}");
    }
}
