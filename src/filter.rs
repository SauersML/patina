// The VCF: TWO white-box circuit emulations, selected by CircuitModel.
//
// == Moog transistor ladder ==
//
// The large-signal ODE system of the ladder (D'Angelo & Valimaki 2014; the
// form used in Paschou, Esqueda, Valimaki & Mourjopoulos, APSIPA 2017,
// eqs. 1-4), in real electrical units:
//
//   (1/wc) dV1/dt = -tanh(A*V1) - tanh(A*(Vin + k*V4))
//   (1/wc) dV2/dt =  tanh(A*V1) - tanh(A*V2)
//   (1/wc) dV3/dt =  tanh(A*V2) - tanh(A*V3)
//   (1/wc) dV4/dt =  tanh(A*V3) - tanh(A*V4)
//
// with A = 1/(2*VT), VT = 25.85 mV thermal voltage, k in [0,4] the feedback
// coefficient, output from V4 (inverted). DC solution: V4 = -Vin/(1+k).
// Both stage currents saturate INDEPENDENTLY (two tanh terms of absolute
// stage voltages), so the ladder colors the signal even in its flat
// passband — the "warmth" is structural, not an add-on.
//
// == ARP 4072 (the post-lawsuit, genuinely-ARP circuit) ==
//
// Traced from the Yusynth 4072 clone schematic (Yves Usson, 2006, from the
// potted module) and the 2600 service manual section 2.9: four identical
// cascaded sections, each a matched differential pair driving an op-amp
// integrator (470 pF selected/matched caps) — a gm-C cascade, NOT a ladder:
//
//   (1/wc) dV1/dt = S(Vin - k*V4 - V1)      S(x) = 2*VT*tanh(x/(2*VT))
//   (1/wc) dVi/dt = S(V(i-1) - Vi)          i = 2..4
//
// The pair sees only the DIFFERENCE between adjacent stages. In the
// passband the stages track their inputs and the difference is tiny, so
// the 4072 stays clean where the ladder already saturates; distortion
// appears only where the filter is actually working (near and above fc).
// That is the honest, derivable reason the ARP sounds "cleaner/darker"
// and the Moog "warmer" — not a marketing adjective, a topology.
// (The pairs carry 220R emitter degeneration, but at audio-cutoff tail
// currents I*RE << VT, so the knee stays at ~2*VT — DERIVED.)
//
// Resonance: DC-coupLED feedback (no Moog-style 2.5 uF regeneration cap)
// from stage 4 to the first pair's opposite base through a DUAL-GANG pot
// whose second gang compensates the output level — the 4072 keeps its
// bass at high resonance where the Moog thins by 1/(1+k). The service
// manual's own checkout targets: unity passband gain (R163 trim), 24
// dB/oct, self-oscillation 13-14 V p-p, fc range 10 Hz - 10 kHz (the
// famously miscalculated bandwidth ceiling of the original 4072).
//
// == Method (both circuits) ==
//
// IMPLICIT MIDPOINT RULE (A-stable), keeping the resonance path inside the
// numerical solution instead of inserting an artificial unit delay. The
// nonlinear system is solved each step by Newton-Raphson with the analytic
// Jacobian; both circuits' Jacobians are lower-bidiagonal plus one corner
// entry (the feedback), so J*delta = -F is solved in CLOSED FORM by
// forward elimination — no matrix library, O(1) per iteration. Cutoff is
// placed exactly by tan() prewarping. 2x oversampled.
//
// Epistemic status:
//   DERIVED    both ODE systems, midpoint discretization, Newton solves,
//              prewarping, passband/rolloff/oscillation behavior, the
//              4072 pair knee at 2*VT despite degeneration
//   SCHEMATIC  ladder: AC-coupled regeneration (2.5 uF, dwg #1149),
//              SELECTED threshold trim, oscillation below knob max;
//              4072: gm-C topology, DC-coupled feedback, dual-gang
//              level compensation, ~100:1 input divider (R6 100K/R7 1K),
//              ~10 kHz cutoff ceiling, 13-14 V p-p self-oscillation and
//              unity passband gain (service manual checkout 2.9.3)
//   CHOICE     the FS-to-volts drive mapping, the ladder's partial
//              resonance make-up (labeled convenience), the exact
//              compensation-gang law (0.9*k), both output stages
//
// Runs 2x oversampled to control aliasing from the tanh harmonics, per the
// literature's recommendation for nonlinear ladder models.

use std::f32::consts::PI;

use crate::adaa::AdaaTanh;
use crate::oscillator::CircuitModel;

/// Thermal voltage at room temperature, volts.
const VT: f32 = 0.02585;
/// The input attenuation stage: program level (10 V p-p) is dropped to
/// ladder operating level before the differential pairs. The literature
/// had to fit this empirically too ("preceded by a passive attenuation
/// stage... the attenuation value was chosen empirically" — Paschou et
/// al.); ours is voiced so a full 5 V swing lands at tanh argument 0.4
/// with drive = 1, the operating point the patches were built around.
const INPUT_ATTEN: f32 = 0.4 * 2.0 * VT / crate::oscillator::PROGRAM_V;
/// 4072 input divider: the op-amp mixer runs at program level, then R6
/// (100K) into R7 (1K) drops it to pair level (SCHEMATIC, Yusynth trace).
const ARP_ATTEN: f32 = 1.0 / 101.0;
/// The 4072's miscalculated bandwidth ceiling: the service manual gives
/// the VCF range as 10 Hz - 10 kHz, and the design error capping the
/// original 4072's top end is documented lore ARP later fixed. Patina
/// keeps the flaw — it is part of what that filter is.
const ARP_FC_MAX: f32 = 10_500.0;
/// Where the 4072's op-amp output stages run out of rail (+/-15 V supply).
const ARP_RAIL: f32 = 13.5;
/// Newton-Raphson stopping tolerance on the residual, volts.
const NEWTON_TOL: f32 = 1e-6;
const NEWTON_MAX_ITERS: usize = 8;

pub struct LadderFilter {
    model: CircuitModel,
    sample_rate: f32,
    target_cutoff: f32,
    cutoff: f32, // smoothed
    target_resonance: f32,
    resonance: f32, // smoothed, knob 0..4
    drive: f32,
    saturation: f32,
    /// Ladder state, volts.
    v: [f32; 4],
    /// Previous ladder input, volts (the midpoint rule averages the input).
    vin_prev: f32,
    /// Per-stage cutoff tolerance (capacitor/transistor spread, +/-0.4%).
    mismatch: [f32; 4],
    /// Per-unit regeneration threshold trim (the SELECTED resistor).
    res_cal: f32,
    /// AC-coupled regeneration: tracked DC of the feedback tap (~25 Hz).
    fb_dc: f32,
    fb_dc_a: f32,
    thermal_drift: f32,
    rng: u32,
    sat_adaa: AdaaTanh,
    /// Diagnostic: worst Newton iteration count since last read.
    max_iters_seen: usize,
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

#[inline]
fn rand01(state: &mut u32) -> f32 {
    (xorshift(state) >> 8) as f32 / (1u32 << 24) as f32
}

impl LadderFilter {
    pub fn new(sample_rate: f32, seed: u32) -> Self {
        let mut rng = seed.wrapping_mul(0x85EB_CA6B) | 1;
        let mut mismatch = [1.0f32; 4];
        for m in &mut mismatch {
            *m = 1.0 + (rand01(&mut rng) - 0.5) * 0.004;
        }
        let res_cal = 1.0 + (rand01(&mut rng) - 0.5) * 0.03;
        Self {
            model: CircuitModel::Moog,
            sample_rate,
            target_cutoff: 15000.0,
            cutoff: 15000.0,
            target_resonance: 0.0,
            resonance: 0.0,
            drive: 1.0,
            saturation: 1.0,
            v: [0.0; 4],
            vin_prev: 0.0,
            mismatch,
            res_cal,
            fb_dc: 0.0,
            // Updated once per 2x substep, so the rate is 2*sample_rate
            fb_dc_a: 1.0 - (-2.0 * PI * 25.0 / (2.0 * sample_rate)).exp(),
            thermal_drift: 0.0,
            rng,
            sat_adaa: AdaaTanh::new(),
            max_iters_seen: 0,
        }
    }

    pub fn set_cutoff(&mut self, cutoff: f32) {
        self.target_cutoff = cutoff.clamp(16.0, self.sample_rate * 0.45);
    }

    pub fn set_model(&mut self, model: CircuitModel) {
        self.model = model;
    }

    pub fn set_resonance(&mut self, resonance: f32) {
        self.target_resonance = resonance.clamp(0.0, 4.0);
    }

    pub fn set_drive(&mut self, drive: f32) {
        self.drive = drive.clamp(0.1, 10.0);
    }

    pub fn set_saturation(&mut self, saturation: f32) {
        self.saturation = saturation.clamp(0.0, 2.0);
    }

    /// Diagnostic for tests: worst Newton iteration count since last call.
    pub fn take_max_iters(&mut self) -> usize {
        std::mem::replace(&mut self.max_iters_seen, 0)
    }

    /// One implicit-midpoint step of period `t_step` toward input `vin`
    /// (volts). `g_base` = tan(pi * fc * t_step), the prewarped one-pole
    /// coefficient; per-stage g_i = g_base * mismatch_i. `k` is the
    /// feedback coefficient; `fb_dc` has already been subtracted from the
    /// feedback tap outside.
    #[inline]
    fn midpoint_step(&mut self, vin_avg_a: f32, g: [f32; 4], k: f32) {
        let a_half = 0.5 / (2.0 * VT); // A/2, for midpoint state averages
        let p = self.v;
        // c_i = 4*VT*g_i converts the unit-tanh drive to volts per step
        let c = [
            4.0 * VT * g[0],
            4.0 * VT * g[1],
            4.0 * VT * g[2],
            4.0 * VT * g[3],
        ];
        // gj_i = c_i * A/2, the Jacobian scale per row: equals g_i exactly
        let gj = g;

        // Warm start from the previous state
        let mut v = p;
        let mut iters = 0;
        loop {
            iters += 1;
            // Midpoint arguments
            let s0 = vin_avg_a + (0.5 / (2.0 * VT)) * k * (v[3] + p[3]) - self.fb_dc * k / (2.0 * VT);
            let s1 = a_half * (v[0] + p[0]);
            let s2 = a_half * (v[1] + p[1]);
            let s3 = a_half * (v[2] + p[2]);
            let s4 = a_half * (v[3] + p[3]);
            let t0 = s0.tanh();
            let t1 = s1.tanh();
            let t2 = s2.tanh();
            let t3 = s3.tanh();
            let t4 = s4.tanh();

            // Residuals (eqs. 15)
            let f1 = v[0] - p[0] + c[0] * (t1 + t0);
            let f2 = v[1] - p[1] - c[1] * (t1 - t2);
            let f3 = v[2] - p[2] - c[2] * (t2 - t3);
            let f4 = v[3] - p[3] - c[3] * (t3 - t4);

            let worst = f1.abs().max(f2.abs()).max(f3.abs()).max(f4.abs());
            if worst < NEWTON_TOL || iters > NEWTON_MAX_ITERS {
                if iters > self.max_iters_seen {
                    self.max_iters_seen = iters;
                }
                break;
            }

            // Analytic Jacobian, sech^2 terms
            let u0 = 1.0 - t0 * t0;
            let u1 = 1.0 - t1 * t1;
            let u2 = 1.0 - t2 * t2;
            let u3 = 1.0 - t3 * t3;
            let u4 = 1.0 - t4 * t4;

            let d1 = 1.0 + gj[0] * u1;
            let d2 = 1.0 + gj[1] * u2;
            let d3 = 1.0 + gj[2] * u3;
            let d4 = 1.0 + gj[3] * u4;
            // dF1/dV4: the feedback corner entry
            let e = gj[0] * k * u0;

            // Solve J*delta = -F by forward elimination: rows 2-4 make
            // delta_2..4 affine in delta_1; substitute into row 1.
            let b2 = gj[1] * u1 / d2;
            let a2 = -f2 / d2;
            let b3 = gj[2] * u2 * b2 / d3;
            let a3 = (-f3 + gj[2] * u2 * a2) / d3;
            let b4 = gj[3] * u3 * b3 / d4;
            let a4 = (-f4 + gj[3] * u3 * a3) / d4;

            let delta1 = (-f1 - e * a4) / (d1 + e * b4);
            let delta2 = a2 + b2 * delta1;
            let delta3 = a3 + b3 * delta1;
            let delta4 = a4 + b4 * delta1;

            v[0] += delta1;
            v[1] += delta2;
            v[2] += delta3;
            v[3] += delta4;
        }
        self.v = v;
    }

    /// One implicit-midpoint step of the 4072 gm-C cascade. Each stage's
    /// pair sees the DIFFERENCE of adjacent stage voltages inside a single
    /// tanh; the resonance feedback k*V4 sums (DC-coupled) against the
    /// input at the first pair. `vin_avg` is the midpoint-averaged input,
    /// volts at pair level.
    #[inline]
    fn midpoint_step_arp(&mut self, vin_avg: f32, g: [f32; 4], k: f32) {
        let a = 1.0 / (2.0 * VT);
        let p = self.v;
        // 4*VT*g * tanh(x/(2VT)) linearizes to 2*g*x: a trapezoidal
        // one-pole with exact tan() prewarping, same scaling as the ladder
        let c = [
            4.0 * VT * g[0],
            4.0 * VT * g[1],
            4.0 * VT * g[2],
            4.0 * VT * g[3],
        ];

        let mut v = p;
        let mut iters = 0;
        loop {
            iters += 1;
            let m1 = 0.5 * (v[0] + p[0]);
            let m2 = 0.5 * (v[1] + p[1]);
            let m3 = 0.5 * (v[2] + p[2]);
            let m4 = 0.5 * (v[3] + p[3]);
            // Stage drive differences, pair-level volts
            let x1 = vin_avg - k * m4 - m1;
            let x2 = m1 - m2;
            let x3 = m2 - m3;
            let x4 = m3 - m4;
            let t1 = (a * x1).tanh();
            let t2 = (a * x2).tanh();
            let t3 = (a * x3).tanh();
            let t4 = (a * x4).tanh();

            let f1 = v[0] - p[0] - c[0] * t1;
            let f2 = v[1] - p[1] - c[1] * t2;
            let f3 = v[2] - p[2] - c[2] * t3;
            let f4 = v[3] - p[3] - c[3] * t4;

            let worst = f1.abs().max(f2.abs()).max(f3.abs()).max(f4.abs());
            if worst < NEWTON_TOL || iters > NEWTON_MAX_ITERS {
                if iters > self.max_iters_seen {
                    self.max_iters_seen = iters;
                }
                break;
            }

            let u1 = 1.0 - t1 * t1;
            let u2 = 1.0 - t2 * t2;
            let u3 = 1.0 - t3 * t3;
            let u4 = 1.0 - t4 * t4;

            // J: dFi/dvi = 1 + g_i*u_i, dFi/dv(i-1) = -g_i*u_i, and the
            // feedback corner dF1/dv4 = +g_1*u_1*k — bidiagonal plus
            // corner, eliminated in closed form exactly like the ladder
            let d1 = 1.0 + g[0] * u1;
            let d2 = 1.0 + g[1] * u2;
            let d3 = 1.0 + g[2] * u3;
            let d4 = 1.0 + g[3] * u4;
            let e = g[0] * u1 * k;

            let b2 = g[1] * u2 / d2;
            let a2 = -f2 / d2;
            let b3 = g[2] * u3 * b2 / d3;
            let a3 = (-f3 + g[2] * u3 * a2) / d3;
            let b4 = g[3] * u4 * b3 / d4;
            let a4 = (-f4 + g[3] * u4 * a3) / d4;

            let delta1 = (-f1 - e * a4) / (d1 + e * b4);
            v[0] += delta1;
            v[1] += a2 + b2 * delta1;
            v[2] += a3 + b3 * delta1;
            v[3] += a4 + b4 * delta1;
        }
        self.v = v;
    }

    /// Process one sample. `cutoff_mult` is the per-sample modulation
    /// multiplier on the (smoothed) base cutoff — filter envelope, key
    /// tracking, velocity, LFO, substrate all arrive through it.
    pub fn process(&mut self, input: f32, cutoff_mult: f32) -> f32 {
        // Slow thermal drift, bounded random walk (SCHEMATIC: matched-pair
        // temperature sensitivity; magnitude as before)
        self.thermal_drift =
            (self.thermal_drift + (rand01(&mut self.rng) - 0.5) * 1e-4) * 0.9995;

        // ~4 ms parameter slew removes zipper noise from stepped automation
        self.cutoff += (self.target_cutoff - self.cutoff) * 0.006;
        self.resonance += (self.target_resonance - self.resonance) * 0.006;

        let fc_top = match self.model {
            // The 4072's documented bandwidth ceiling
            CircuitModel::Arp => ARP_FC_MAX.min(self.sample_rate * 0.49),
            CircuitModel::Moog => self.sample_rate * 0.49,
        };
        let fc = (self.cutoff * cutoff_mult * (1.0 + self.thermal_drift))
            .clamp(16.0, fc_top);

        // 2x oversampling; exact placement via tan prewarp per substep
        let t_step = 0.5 / self.sample_rate;
        let g_base = (PI * fc * t_step).tan();
        let g = [
            g_base * self.mismatch[0],
            g_base * self.mismatch[1],
            g_base * self.mismatch[2],
            g_base * self.mismatch[3],
        ];

        match self.model {
            CircuitModel::Moog => {
                // Regeneration pot R22 is 50K REVERSE-AUDIO (dwg #1149):
                // feedback rises fast in the first half of travel and
                // creeps at the top. Scaled so the oscillation threshold
                // (k=4) lands at knob 3/4 — the service manual's
                // "regeneration 7-8 of 10" falls out of the taper instead
                // of being pasted on (SCHEMATIC)
                let x = self.resonance * 0.25;
                let k = 4.267 * self.res_cal * (1.0 - (1.0 - x) * (1.0 - x));

                // Program volts through the input attenuator; drive is the
                // attenuator's setting (more drive = more ladder current)
                let vin = input * INPUT_ATTEN * self.drive;
                let vin_avg_a = (vin + self.vin_prev) * 0.5 / (2.0 * VT);
                self.vin_prev = vin;

                for _ in 0..2 {
                    self.midpoint_step(vin_avg_a, g, k);
                    // AC-coupled regeneration (dwg #1149): track the
                    // feedback tap's DC so only audio circulates
                    self.fb_dc += self.fb_dc_a * (self.v[3] - self.fb_dc);
                }

                // Output: -V4, back to program level. sqrt(drive) make-up
                // keeps the drive knob about grit rather than volume
                // (CHOICE); unity through-gain at resonance minimum per
                // the 904A calibration
                let mut out = -self.v[3] / (INPUT_ATTEN * self.drive.sqrt().max(0.5));

                // Partial make-up for the exact 1/(1+k) passband loss. The
                // hardware does NOT do this — players ride the volume;
                // labeled a convenience (CHOICE)
                out *= 1.0 + self.resonance * 0.3;

                // Output saturation stage with antiderivative antialiasing,
                // referenced to program level
                if self.saturation > 0.02 {
                    let pv = crate::oscillator::PROGRAM_V;
                    out = pv * self.sat_adaa.process(out * self.saturation / pv)
                        / self.saturation;
                }
                out
            }
            CircuitModel::Arp => {
                // The 2600's resonance slider is 100K LINEAR (panel silk):
                // k rises evenly along the whole travel, reaching the
                // oscillation threshold only at the top — "when the Q is
                // at maximum, the VCF will oscillate". The margin is
                // trimmed (service manual 'VCF cal') so the oscillation
                // settles at the spec'd 13-14 V p-p, not more (SCHEMATIC)
                let k = self.resonance * self.res_cal * 1.02;

                let vin = input * ARP_ATTEN * self.drive;
                let vin_avg = (vin + self.vin_prev) * 0.5;
                self.vin_prev = vin;

                // DC-coupled feedback: no regeneration-cap tracking. The
                // manual's checkout self-oscillates the VCF at 10 Hz —
                // only a DC-coupled loop can do that.
                for _ in 0..2 {
                    self.midpoint_step_arp(vin_avg, g, k);
                }

                // Back to program level (gain trimmed for unity passband,
                // service manual 2.9.3 step 4), plus the dual-gang pot's
                // level compensation: the second resonance gang restores
                // what 1/(1+k) takes away, so the 4072 keeps its body at
                // high resonance. The gang law 0.65*k is the one constant
                // jointly trimmed to BOTH manual specs: passband held near
                // unity under resonance AND 13-14 V p-p self-oscillation
                let mut out = self.v[3] / (ARP_ATTEN * self.drive.sqrt().max(0.5));
                out *= 1.0 + 0.65 * k;

                // The TL074/LM3900 output stages clip at the +/-15 V
                // rails; transparent at program level, engaged by
                // self-oscillation (which the manual specs at 13-14 V p-p)
                out = ARP_RAIL * self.sat_adaa.process(out / ARP_RAIL);
                out
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    /// Steady-state amplitude of `freq` through the filter at the given
    /// settings, measured over the second half of a one-second run.
    fn gain_at(filter_cfg: impl Fn(&mut LadderFilter), freq: f32, level: f32) -> f32 {
        let sr = 88200.0; // high rate so measurements sit far from Nyquist
        let mut f = LadderFilter::new(sr, 3);
        f.set_saturation(0.0);
        filter_cfg(&mut f);
        let mut peak = 0.0f32;
        let n = sr as usize;
        for i in 0..n {
            let x = level * (TAU * freq * i as f32 / sr).sin();
            let y = f.process(x, 1.0);
            if i > n / 2 {
                peak = peak.max(y.abs());
            }
        }
        peak / level
    }

    /// Four cascaded one-poles: |H(fc)|/|H(passband)| = (1/sqrt(2))^4 =
    /// -12 dB exactly. This is the service manual's own calibration
    /// definition of cutoff. Measured relative to the filter's own
    /// passband so the drive law cancels.
    #[test]
    fn minus_12_db_lands_on_the_set_cutoff() {
        let cfg = |f: &mut LadderFilter| {
            f.set_cutoff(1000.0);
            f.set_resonance(0.0);
            f.set_drive(0.2); // small signal: linear regime
        };
        let g_fc = gain_at(cfg, 1000.0, 0.5);
        let g_pass = gain_at(cfg, 62.5, 0.5);
        let db = 20.0 * (g_fc / g_pass).log10();
        assert!(
            (-13.5..=-10.5).contains(&db),
            "gain at fc should be -12 dB re passband, got {db:.2} dB"
        );
    }

    /// 24 dB/octave: between 2*fc and 4*fc the exact 4-pole ratio is
    /// (1+4)^2 / (1+16)^2 = 25/289 ~ -21.3 dB (approaching the asymptote).
    #[test]
    fn rolloff_is_four_pole() {
        let cfg = |f: &mut LadderFilter| {
            f.set_cutoff(1000.0);
            f.set_resonance(0.0);
            f.set_drive(0.2);
        };
        let g2 = gain_at(cfg, 2000.0, 0.5);
        let g4 = gain_at(cfg, 4000.0, 0.5);
        let ratio = g4 / g2;
        assert!(
            (0.05..=0.13).contains(&ratio),
            "expected ~25/289 between 2fc and 4fc, got {ratio:.4}"
        );
    }

    /// The ODE's DC solution is V4 = -Vin/(1+k): resonance thins the
    /// passband by exactly 1/(1+k) (before the labeled make-up gain).
    #[test]
    fn passband_matches_the_ode_dc_solution() {
        // 400 Hz: deep in the passband of an 8 kHz cutoff, but far enough
        // above the 25 Hz AC-coupled regeneration corner that the feedback
        // is effectively full-strength
        let knob = 3.0f32;
        let g0 = gain_at(
            |f| {
                f.set_cutoff(8000.0);
                f.set_resonance(0.0);
                f.set_drive(0.2);
            },
            400.0,
            0.25,
        );
        let gk = gain_at(
            |f| {
                f.set_cutoff(8000.0);
                f.set_resonance(knob);
                f.set_drive(0.2);
            },
            400.0,
            0.25,
        );
        // knob -> k through the reverse-audio taper of R22
        let x = knob * 0.25;
        let k = 4.267 * (1.0 - (1.0 - x) * (1.0 - x));
        let expected = (1.0 + 0.3 * knob) / (1.0 + k); // make-up * ODE loss
        let measured = gk / g0;
        assert!(
            (measured / expected - 1.0).abs() < 0.15,
            "passband ratio should track 1/(1+k): measured {measured:.3}, expected {expected:.3}"
        );
    }

    /// Symmetric tanh saturation produces odd-dominant distortion: driven
    /// hard, the third harmonic must dominate the second (the APSIPA
    /// measurement result for the real filter).
    #[test]
    fn distortion_is_odd_dominant() {
        let sr = 88200.0;
        let mut f = LadderFilter::new(sr, 3);
        f.set_cutoff(8000.0);
        f.set_resonance(0.0);
        f.set_drive(6.0);
        f.set_saturation(0.0);
        let f0 = 441.0;
        let n = sr as usize;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let x = 4.5 * (TAU * f0 * i as f32 / sr).sin();
            out.push(f.process(x, 1.0));
        }
        let goertzel = |freq: f32| -> f32 {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, &s) in out[n / 2..].iter().enumerate() {
                let a = TAU * freq * i as f32 / sr;
                re += s * a.cos();
                im += s * a.sin();
            }
            (re * re + im * im).sqrt()
        };
        let h2 = goertzel(2.0 * f0);
        let h3 = goertzel(3.0 * f0);
        assert!(
            h3 > 2.0 * h2,
            "odd harmonics should dominate: H2={h2:.1}, H3={h3:.1}"
        );
    }

    /// At k = 4 the linearized poles reach the imaginary axis; the tanh
    /// nonlinearity settles the oscillation to finite amplitude.
    #[test]
    fn self_oscillates_at_max_resonance() {
        let sr = 44100.0;
        let mut filter = LadderFilter::new(sr, 7);
        filter.set_cutoff(1500.0);
        filter.set_resonance(4.0);
        for _ in 0..8000 {
            filter.process(0.0, 1.0);
        }
        filter.process(2.5, 1.0);
        let mut tail = 0.0f32;
        for i in 0..44100 {
            let y = filter.process(0.0, 1.0);
            assert!(y.is_finite());
            if i > 44100 - 4410 {
                tail = tail.max(y.abs());
            }
        }
        assert!(
            tail > 0.01,
            "filter should self-oscillate at resonance 4, tail peak = {tail}"
        );
    }

    /// The reverse-audio regeneration pot puts the ladder's oscillation
    /// threshold at knob ~3/4 ("regeneration 7-8 of 10"), NOT at the top:
    /// knob 3.4/4 must scream, knob 2/4 must not.
    #[test]
    fn moog_oscillation_threshold_sits_at_seven_of_ten() {
        let sr = 44100.0;
        let tail_at = |knob: f32| -> f32 {
            let mut f = LadderFilter::new(sr, 7);
            f.set_cutoff(1500.0);
            f.set_resonance(knob);
            for _ in 0..8000 {
                f.process(0.0, 1.0);
            }
            f.process(2.5, 1.0);
            let mut tail = 0.0f32;
            for i in 0..44100 {
                let y = f.process(0.0, 1.0);
                if i > 44100 - 4410 {
                    tail = tail.max(y.abs());
                }
            }
            tail
        };
        let above = tail_at(3.4);
        let below = tail_at(2.0);
        assert!(
            above > 0.05 && below < 0.01,
            "oscillation should start near knob 3/4: at 3.4 tail={above:.4}, at 2.0 tail={below:.4}"
        );
    }

    /// Resonance at the bottom of the range separates the circuits: the
    /// Moog's regeneration is AC-coupled (2.5 uF, ~25 Hz corner) so the
    /// loop cannot sustain oscillation at a 20 Hz cutoff; the ARP's
    /// DC-coupled feedback self-oscillates all the way down (the manual's
    /// own checkout produces a 10 Hz sine).
    #[test]
    fn low_cutoff_resonance_separates_the_circuits() {
        let sr = 44100.0;
        let tail_of = |model: CircuitModel| -> f32 {
            let mut f = LadderFilter::new(sr, 7);
            f.set_model(model);
            f.set_cutoff(20.0);
            f.set_resonance(4.0);
            for _ in 0..12000 {
                f.process(0.0, 1.0);
            }
            // Prime the loop with a driven 20 Hz burst (growth from a
            // one-sample ping is far too slow at this cutoff), then let
            // go: a sustaining loop holds the amplitude, a lossy one
            // rings down
            for i in 0..44100 {
                let x = 4.0 * (TAU * 20.0 * i as f32 / sr).sin();
                f.process(x, 1.0);
            }
            let mut tail = 0.0f32;
            let n = 3 * 44100; // a 20 Hz cycle is 2205 samples
            for i in 0..n {
                let y = f.process(0.0, 1.0);
                assert!(y.is_finite());
                if i > n - 8820 {
                    tail = tail.max(y.abs());
                }
            }
            tail
        };
        let arp = tail_of(CircuitModel::Arp);
        let moog = tail_of(CircuitModel::Moog);
        assert!(
            arp > 1.0 && moog < 0.25 * arp,
            "at 20 Hz cutoff the ARP should oscillate and the Moog's AC-coupled loop should not: arp {arp:.3} V, moog {moog:.3} V"
        );
    }

    #[test]
    fn passband_is_transparent_at_zero_resonance() {
        let g = gain_at(
            |f| {
                f.set_cutoff(12000.0);
                f.set_resonance(0.0);
            },
            220.0,
            0.5,
        );
        assert!(
            (0.7..=1.3).contains(&g),
            "passband gain should be near unity, got {g}"
        );
    }

    /// The 4072 gm-C cascade is still four one-poles: same -12 dB at fc,
    /// same 24 dB/oct — the topologies differ in nonlinearity and
    /// feedback behavior, not in the small-signal transfer function.
    #[test]
    fn arp_rolloff_is_four_pole() {
        let cfg = |f: &mut LadderFilter| {
            f.set_model(CircuitModel::Arp);
            f.set_cutoff(1000.0);
            f.set_resonance(0.0);
            f.set_drive(0.2);
        };
        let g_fc = gain_at(cfg, 1000.0, 0.5);
        let g_pass = gain_at(cfg, 62.5, 0.5);
        let db_fc = 20.0 * (g_fc / g_pass).log10();
        assert!(
            (-13.5..=-10.5).contains(&db_fc),
            "4072 gain at fc should be -12 dB re passband, got {db_fc:.2} dB"
        );
        let g2 = gain_at(cfg, 2000.0, 0.5);
        let g4 = gain_at(cfg, 4000.0, 0.5);
        let ratio = g4 / g2;
        assert!(
            (0.05..=0.13).contains(&ratio),
            "expected ~25/289 between 2fc and 4fc, got {ratio:.4}"
        );
    }

    /// THE audible difference between the circuits, derived from topology:
    /// the ladder's stages saturate on absolute voltages (colors the
    /// passband); the 4072's pairs see only adjacent-stage differences
    /// (clean in the passband). Same hot signal, same settings: the
    /// ladder must produce far more passband harmonic distortion.
    #[test]
    fn arp_is_cleaner_than_the_ladder_in_the_passband() {
        let sr = 88200.0;
        let f0 = 441.0;
        let n = sr as usize;
        let h3_of = |model: CircuitModel| -> f32 {
            let mut f = LadderFilter::new(sr, 3);
            f.set_model(model);
            f.set_cutoff(8000.0);
            f.set_resonance(0.0);
            f.set_drive(6.0);
            f.set_saturation(0.0);
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let x = 4.5 * (TAU * f0 * i as f32 / sr).sin();
                out.push(f.process(x, 1.0));
            }
            let goertzel = |freq: f32| -> f32 {
                let (mut re, mut im) = (0.0f32, 0.0f32);
                for (i, &s) in out[n / 2..].iter().enumerate() {
                    let a = TAU * freq * i as f32 / sr;
                    re += s * a.cos();
                    im += s * a.sin();
                }
                (re * re + im * im).sqrt()
            };
            goertzel(3.0 * f0) / goertzel(f0).max(1e-9)
        };
        let moog = h3_of(CircuitModel::Moog);
        let arp = h3_of(CircuitModel::Arp);
        assert!(
            arp < 0.25 * moog,
            "4072 should be far cleaner in the passband: ladder H3/H1={moog:.4}, 4072 H3/H1={arp:.4}"
        );
    }

    /// The dual-gang resonance pot's compensation: the 4072 keeps its
    /// passband body at high resonance where the ladder thins toward
    /// 1/(1+k).
    #[test]
    fn arp_passband_holds_at_high_resonance() {
        let ratio_of = |model: CircuitModel| -> f32 {
            let g0 = gain_at(
                |f| {
                    f.set_model(model);
                    f.set_cutoff(8000.0);
                    f.set_resonance(0.0);
                    f.set_drive(0.2);
                },
                400.0,
                0.25,
            );
            let gk = gain_at(
                |f| {
                    f.set_model(model);
                    f.set_cutoff(8000.0);
                    f.set_resonance(3.0);
                    f.set_drive(0.2);
                },
                400.0,
                0.25,
            );
            gk / g0
        };
        let arp = ratio_of(CircuitModel::Arp);
        let moog = ratio_of(CircuitModel::Moog);
        // (the ladder figure includes its labeled partial make-up gain;
        // its raw ODE loss at this k is 1/(1+3.36) ~ 0.23)
        assert!(
            arp > 0.65 && arp > 1.4 * moog,
            "compensated 4072 passband should hold where the ladder thins: 4072 {arp:.3}, ladder {moog:.3}"
        );
    }

    /// Service manual 2.9.3 step 15: resonance max, output "a 13-14 volt
    /// peak to peak sine wave". DC-coupled feedback self-oscillates.
    #[test]
    fn arp_self_oscillates_hot() {
        let sr = 44100.0;
        let mut filter = LadderFilter::new(sr, 7);
        filter.set_model(CircuitModel::Arp);
        filter.set_cutoff(500.0);
        filter.set_resonance(4.0);
        for _ in 0..8000 {
            filter.process(0.0, 1.0);
        }
        filter.process(2.5, 1.0);
        let (mut hi, mut lo) = (0.0f32, 0.0f32);
        for i in 0..44100 {
            let y = filter.process(0.0, 1.0);
            assert!(y.is_finite());
            if i > 44100 - 8820 {
                hi = hi.max(y);
                lo = lo.min(y);
            }
        }
        let pp = hi - lo;
        assert!(
            (9.0..=16.0).contains(&pp),
            "4072 self-oscillation should be ~13-14 V p-p, got {pp:.1}"
        );
    }

    /// The 4072's documented flaw: cutoff tops out around 10 kHz however
    /// far the control is pushed. The ladder opens fully.
    #[test]
    fn arp_cutoff_ceiling_is_real() {
        let g_of = |model: CircuitModel| -> f32 {
            gain_at(
                |f| {
                    f.set_model(model);
                    f.set_cutoff(20000.0);
                    f.set_resonance(0.0);
                    f.set_drive(0.2);
                },
                15000.0,
                0.25,
            ) / gain_at(
                |f| {
                    f.set_model(model);
                    f.set_cutoff(20000.0);
                    f.set_resonance(0.0);
                    f.set_drive(0.2);
                },
                1000.0,
                0.25,
            )
        };
        let arp = g_of(CircuitModel::Arp);
        let moog = g_of(CircuitModel::Moog);
        assert!(
            arp < 0.2 && moog > 0.3,
            "15 kHz through a 'fully open' filter: 4072 should be capped ({arp:.3}), ladder open ({moog:.3})"
        );
    }

    /// Newton must converge fast everywhere musical: sweep cutoff and
    /// resonance under hot drive and confirm the iteration cap is never
    /// the thing that stops it.
    #[test]
    fn newton_converges_across_the_map() {
        let sr = 44100.0;
        let mut f = LadderFilter::new(sr, 11);
        f.set_drive(8.0);
        let mut worst = 0usize;
        for (cut, res) in [
            (60.0, 0.0),
            (60.0, 4.0),
            (1000.0, 3.9),
            (8000.0, 4.0),
            (18000.0, 2.0),
            (18000.0, 4.0),
        ] {
            f.set_cutoff(cut);
            f.set_resonance(res);
            for i in 0..22050 {
                let x = 4.75 * (TAU * 220.0 * i as f32 / sr).sin();
                f.process(x, 1.0);
            }
            worst = worst.max(f.take_max_iters());
        }
        assert!(
            worst <= NEWTON_MAX_ITERS,
            "Newton hit the iteration cap: {worst}"
        );
    }
}
