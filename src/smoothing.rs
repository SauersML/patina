// Time constants, spelled once.
//
// A bare per-sample coefficient — `state += 0.001 * (target - state)` — is
// not a time constant, it is a time constant DIVIDED BY THE SAMPLE RATE at
// whatever rate the author happened to be running. Ship it and the knob
// smoothing that took 21 ms in the studio takes 10 ms at 96 kHz and 23 ms
// at 44.1 kHz; the DC blocker that sat at 34 Hz sits at 69 Hz; the tape's
// 5 ms dropout ramp becomes a 2.6 ms click. None of that is audible as
// "wrong", which is exactly why it survives: it just makes the instrument
// quietly a different instrument at every rate.
//
// Most of the codebase already spells these correctly inline
// (`envelope.rs`, `vocoder.rs`, `talker.rs`, `vox.rs`, the reverb and
// spring filters). This module is where the spelling lives so the
// remaining hand-written copies can stop drifting from it.

use std::f32::consts::TAU;

/// Per-sample coefficient `k` for the exponential approach
/// `state += k * (target - state)`, reaching 1 - 1/e of the way to the
/// target after `tau_seconds`.
#[inline]
pub fn approach(tau_seconds: f32, sample_rate: f32) -> f32 {
    (1.0 - (-1.0 / (tau_seconds.max(1e-6) * sample_rate)).exp()).clamp(0.0, 1.0)
}

/// Per-sample coefficient for a one-pole lowpass with its -3 dB corner at
/// `cutoff_hz`, for `state += k * (x - state)`.
#[inline]
pub fn one_pole(cutoff_hz: f32, sample_rate: f32) -> f32 {
    (1.0 - (-TAU * cutoff_hz / sample_rate).exp()).clamp(0.0, 1.0)
}

/// Pole radius `R` for the standard DC blocker `y = x - x1 + R * y1`,
/// placing its corner at `cutoff_hz`.
#[inline]
pub fn dc_blocker_pole(cutoff_hz: f32, sample_rate: f32) -> f32 {
    (-TAU * cutoff_hz / sample_rate).exp()
}

/// The DC-blocker corner used across the instrument's signal path: the
/// voice's post-ladder coupling, the master bus, and the fuzz pedal.
///
/// The value is not arbitrary and not new — all three sites carried a
/// hard-coded pole of `0.9955`, which IS this corner, but only at 48 kHz.
/// Naming it keeps that exact voicing where it was tuned and makes the
/// other rates agree with it instead of drifting to 31.7 Hz (44.1 k) or
/// 69 Hz (96 k).
///
/// NOTE: the circuit Moog drawing #1149 specifies (2.5 uF into the ~2 K
/// following stage) is 1/(2*pi*R*C) = 31.8 Hz, not 34.5. Moving to the
/// schematic figure would be more faithful but WOULD revoice every
/// existing render slightly, so it is deliberately left at the tuned
/// value; `the_named_constants_preserve_the_48k_voicing` enforces that.
pub const DC_BLOCK_HZ: f32 = 34.5;

/// Knob de-zipper time constant: long enough to kill stepping on a
/// coarsely-quantised MIDI CC, short enough that a knob still feels
/// attached to the sound. Was a hard-coded `0.001`/sample, which is this
/// at 48 kHz.
pub const KNOB_SMOOTH_S: f32 = 0.0208;

/// The slower de-zipper, for controls where a touch more lag is wanted
/// than on a filter knob: master volume, mixer-strip gain, bus drive and
/// tone. Was a hard-coded `0.0008`/sample, which is this at 48 kHz.
pub const GAIN_SMOOTH_S: f32 = 0.026;

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: a time constant must mean the same number of
    /// SECONDS at every rate.
    #[test]
    fn a_time_constant_is_the_same_time_at_every_rate() {
        for tau in [0.005f32, 0.0208, 0.026] {
            for sr in [44100.0f32, 48000.0, 88200.0, 96000.0] {
                let k = approach(tau, sr);
                // step response: how long to cross 1 - 1/e = 0.632?
                let mut state = 0.0f32;
                let mut n = 0usize;
                while state < 1.0 - std::f32::consts::E.recip() {
                    state += k * (1.0 - state);
                    n += 1;
                }
                let measured = n as f32 / sr;
                assert!(
                    (measured / tau - 1.0).abs() < 0.02,
                    "tau {tau}s at {sr} Hz measured {measured}s"
                );
            }
        }
    }

    #[test]
    fn one_pole_lands_its_corner() {
        for hz in [34.5f32, 300.0, 5000.0] {
            for sr in [44100.0f32, 48000.0, 96000.0] {
                // measure the -3 dB point by driving a sine at `hz`
                let k = one_pole(hz, sr);
                let mut state = 0.0f32;
                let mut peak = 0.0f32;
                let n = (sr as usize / hz as usize) * 200;
                for i in 0..n {
                    let x = (TAU * hz * i as f32 / sr).sin();
                    state += k * (x - state);
                    if i > n / 2 {
                        peak = peak.max(state.abs());
                    }
                }
                let db = 20.0 * peak.log10();
                assert!(
                    (db + 3.0).abs() < 0.6,
                    "one_pole({hz}, {sr}) measured {db:.2} dB, want -3"
                );
            }
        }
    }

    /// The named constants must reproduce the hard-coded values they
    /// replaced, at the 48 kHz those values were tuned at — otherwise this
    /// "refactor" silently revoiced the instrument.
    #[test]
    fn the_named_constants_preserve_the_48k_voicing() {
        let pole = dc_blocker_pole(DC_BLOCK_HZ, 48000.0);
        assert!(
            (pole - 0.9955).abs() < 1e-4,
            "DC blocker pole at 48k is {pole}, was 0.9955"
        );
        let k = approach(KNOB_SMOOTH_S, 48000.0);
        assert!(
            (k - 0.001).abs() < 1e-5,
            "knob smoothing at 48k is {k}, was 0.001"
        );
    }
}
