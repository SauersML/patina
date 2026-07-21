// Oscillator modeled on the 901B / ARP 2600 architecture: ONE physical core
// (a bandlimited sawtooth ramp) with every other waveform DERIVED from it,
// the way the converter circuits do it — not four unrelated ideal generators.
//
//   ramp core ── sawtooth output
//       ├─ fold ──────────── triangle (slight asymmetry, like the trim
//       │                    adjustments the service manual provides)
//       ├─ fold + rounding ─ sine ("Q3's nonlinearity rounds the peaks of
//       │                    the triangle to approximate a sine wave")
//       └─ comparator ────── pulse (per-unit duty error inside the 901B's
//                            48–52% service window)
//
// Output amplitudes follow the 901B alignment targets (saw 0.50 Vac,
// sine 0.50, triangle 0.65, pulse 1.2) converted to peak ratios rather than
// normalizing every wave to the same level — switching waveforms changes
// how hard the filter is driven, exactly as on the hardware.

use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Waveform {
    Sine,
    Square,
    Sawtooth,
    Triangle,
}

/// Which hardware's converter circuits shape the waveforms. The Moog 901-B
/// and ARP 4027 use genuinely different circuits — the service documents
/// say to pick a profile, not average them into one vague "analog".
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CircuitModel {
    /// 901-B: pulse swings between the +11.5/-6 rails (bipolar,
    /// asymmetric); sine from the 1N34 diode-ladder with its soft knee.
    Moog,
    /// 4027-1: pulse rectified positive-going-only (CR7/CR8/CR14), so PWM
    /// shifts the filter's operating point; sine from a single transistor
    /// rounding ("Q3's nonlinearity"), smoother than the diode ladder.
    Arp,
}

// The oscillator speaks VOLTS. Both service manuals put program level at
// 10 V peak-to-peak ("R50 and R51 reduce the amplitude of the pulse wave to
// about 10 volts, peak to peak"; "most oscillator waveforms are
// approximately 10 V p-p"), so the saw is +/-5 V and the other waves carry
// the 901-B alignment ratios relative to it.
pub const PROGRAM_V: f32 = 5.0;
const AMP_SAW: f32 = PROGRAM_V;
const AMP_SINE: f32 = 0.82 * PROGRAM_V;
const AMP_TRI: f32 = 1.3 * PROGRAM_V;
const AMP_PULSE: f32 = 1.38 * PROGRAM_V;

pub struct Oscillator {
    phase: f64,
    frequency: AtomicU32,
    /// Fixed frequency ratio for unison detune (2^(cents/1200)).
    freq_mult: f32,
    sample_rate: f32,
    waveform: Waveform,
    model: CircuitModel,
    drift: f32,
    rng: u32,
    /// Effective comparator threshold this sample (base width + unit error).
    duty: f32,
    /// This unit's fixed comparator error: duty sits near, not at, center.
    duty_error: f32,
    /// Waveshaper asymmetry for the triangle fold / sine rounding.
    skew: f32,
    /// Integrator sag: a real ramp core charging against finite source
    /// impedance bows slightly instead of rising linearly. The quadratic
    /// bend adds low-order even harmonics that soften the ideal saw's buzz.
    curvature: f32,
    /// Sub-oscillator, per the Juno-106's MC5534 ("divided by two
    /// rectangular"): a square one octave down, phase-locked to the core by
    /// construction — its phase advances at exactly half the core rate.
    sub_phase: f64,
    last_sub: f32,
    /// The SOURCE MIXER: per-core levels for [saw, pulse, triangle,
    /// sine]. All four converters run in parallel off the ONE ramp core —
    /// exactly as on the hardware (901-B and 4027 converter boards, the
    /// SH-101 mixer) — so mixed waveforms are phase-locked by
    /// construction. All zeros = classic selector mode (`waveform`).
    mix: [f32; 4],
    /// 901-B residual tuning error, ADDITIVE IN HERTZ: the tracking
    /// resistors "lower the oscillator frequency by a given number of
    /// cycles, REGARDLESS of the magnitude of the control voltage"
    /// (service manual, retracking procedure), trimmed until banks beat
    /// no faster than one every two seconds. So a Moog bank beats slowly
    /// on high notes and howls on low ones. The 4027-1's V/oct trim is
    /// multiplicative instead, so this offset applies to Moog only.
    hz_offset: f32,
    /// If this core wrapped during the last step: fraction of the sample
    /// period elapsed SINCE the wrap (for slaving another core to it).
    last_wrap_frac: Option<f32>,
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

impl Oscillator {
    pub fn new(sample_rate: f32, frequency: f32, seed: u32) -> Self {
        let mut rng = seed.wrapping_mul(0x9E37_79B9) | 1;
        // Random start phase so unison oscillators never phase-lock
        let phase = rand01(&mut rng) as f64;
        // Component-tolerance constants, fixed for the life of the "board"
        let duty_error = (rand01(&mut rng) - 0.5) * 0.02; // within +/-1%
        let skew = (rand01(&mut rng) - 0.5) * 0.06;
        let curvature = 0.02 + rand01(&mut rng) * 0.05;
        // Within the 901B acceptance: bank beat rate <= 0.5 Hz means
        // each unit sits within ~+/-0.25 Hz of true after trimming
        let hz_offset = (rand01(&mut rng) - 0.5) * 0.5;
        Self {
            phase,
            frequency: AtomicU32::new(frequency.to_bits()),
            freq_mult: 1.0,
            sample_rate,
            waveform: Waveform::Sawtooth,
            model: CircuitModel::Moog,
            drift: 0.0,
            rng,
            duty: 0.5 + duty_error,
            duty_error,
            skew,
            sub_phase: 0.0,
            last_sub: 0.0,
            last_wrap_frac: None,
            curvature,
            hz_offset,
            mix: [0.0; 4],
        }
    }

    /// Set the source-mixer levels [saw, pulse, tri, sine]. Any level
    /// above zero switches this core into mixer mode; all zeros returns
    /// it to the classic selector.
    pub fn set_mix(&mut self, mix: [f32; 4]) {
        self.mix = mix.map(|m| m.clamp(0.0, 1.0));
    }

    /// Trim culture differs by maker: the 4027-1 converter boards carry a
    /// symmetry trimmer (R115) and a sine purity trimmer (R121), and the
    /// calibration procedure sets duty to "exactly 50%"; the 901B's
    /// service windows are wider (48-52% duty, "if symmetry is still not
    /// possible, R8 and R9 may have to be changed"). ARP units run closer
    /// to ideal.
    #[inline]
    fn trim(&self) -> f32 {
        match self.model {
            CircuitModel::Moog => 1.0,
            CircuitModel::Arp => 0.35,
        }
    }

    /// Fraction of the last sample period since this core's ramp wrapped,
    /// if it did — the sync signal a slave oscillator locks to.
    pub fn wrap_frac(&self) -> Option<f32> {
        self.last_wrap_frac
    }

    /// `common_drift` is the voice-shared component (the oscillators sit on
    /// one controller card and supply); each core adds its own smaller
    /// residual walk on top, so a bank beats slowly rather than wobbling.
    /// `pitch_mult` is the global modulation ratio (vibrato applied in CV
    /// space, so it is exponential — a frequency ratio, not added hertz).
    /// `pulse_width` is the comparator threshold; this unit's duty error
    /// rides on top of it.
    /// `sync`: if Some(frac), the master core wrapped `frac` of a sample
    /// period ago — hard-reset this core's ramp at that exact sub-sample
    /// position (the 2600's VCO1->VCO2 sync). The reset discontinuity is
    /// bandlimited with a polyBLEP scaled to the TRUE mid-ramp jump height;
    /// unscaled corrections are the classic source of sync aliasing.
    pub fn next_sample(
        &mut self,
        common_drift: f32,
        pitch_mult: f32,
        pulse_width: f32,
        sync: Option<f32>,
    ) -> f32 {
        let frequency = f32::from_bits(self.frequency.load(Ordering::Relaxed));

        // Small individual drift; the larger, shared component comes in from
        // the voice so all three oscillators move together. The 4027-1 is
        // internally temperature-compensated (service manual 3.2.3, plus
        // the T.C. resistor in the expo converter); the 901 walks wider —
        // its own manual demands a 30-minute warm-up before adjusting.
        let walk = match self.model {
            CircuitModel::Moog => 1.2e-5,
            CircuitModel::Arp => 0.45e-5,
        };
        self.drift = (self.drift + (rand01(&mut self.rng) - 0.5) * walk) * 0.9995;
        // The 901B's additive-Hz tracking residue (see hz_offset)
        let f_off = match self.model {
            CircuitModel::Moog => self.hz_offset,
            CircuitModel::Arp => 0.0,
        };
        let detuned_frequency = (frequency * self.freq_mult * (1.0 + self.drift + common_drift)
            * pitch_mult
            + f_off)
            .max(0.01);
        self.duty = (pulse_width + self.duty_error * self.trim()).clamp(0.03, 0.97);

        let dt = detuned_frequency as f64 / self.sample_rate as f64;
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.last_wrap_frac = Some((self.phase / dt.max(1e-12)) as f32);
        } else {
            self.last_wrap_frac = None;
        }

        // Hard sync: reset the ramp at the master's wrap position. The
        // pre-reset phase p_d sets the actual jump height for bandlimiting.
        let mut sync_p_d: Option<f32> = None;
        if let Some(frac) = sync {
            let dtf = dt as f32;
            let new_phase = (frac * dtf).clamp(0.0, 0.999_99);
            let p_d = ((self.phase as f32) - new_phase).rem_euclid(1.0);
            self.phase = new_phase as f64;
            self.last_wrap_frac = Some(frac);
            sync_p_d = Some(p_d);
        }

        // Sub square at half rate, sharing the core's increment so it can
        // never drift against it; bandlimited with its own polyBLEP
        self.sub_phase += dt * 0.5;
        self.sub_phase %= 1.0;
        {
            let ts = self.sub_phase as f32;
            let dts = (dt * 0.5) as f32;
            let naive = if ts < 0.5 { 1.0 } else { -1.0 };
            self.last_sub = PROGRAM_V
                * (naive - self.polyblep(ts, dts) + self.polyblep((ts + 0.5) % 1.0, dts));
        }

        let t = self.phase as f32;
        // Sync corrections: the standard wrap polyBLEP assumes a full-height
        // jump; a sync reset from mid-ramp p_d jumps only part way, so the
        // saw's correction is rescaled and a jump-free pulse reset has its
        // spurious correction cancelled. (Tri/sine sync rides the plain
        // reset — mild, and those shapes are rarely synced.)
        let dtf = (dt as f32).max(1e-9);
        let sync_fix = match (sync_p_d, self.waveform) {
            (Some(p_d), Waveform::Sawtooth) => (1.0 - p_d) * self.polyblep(t, dtf),
            (Some(p_d), Waveform::Square) if p_d < self.duty => {
                // No physical jump (hi -> hi): cancel the wrap-edge blep
                let (hi, lo) = match self.model {
                    CircuitModel::Moog => (1.0f32, -0.92f32),
                    CircuitModel::Arp => (1.0f32, 0.0f32),
                };
                (hi - lo) * 0.5 * self.polyblep(t, dtf)
            }
            _ => 0.0,
        };
        // Output levels are a circuit fact per maker. Moog 901B alignment
        // targets are deliberately UNEQUAL (saw 0.50 Vac, sine 0.50, tri
        // 0.65, pulse 1.2 — peak ratios below), so switching waveforms
        // changes how hard the filter is driven. The 4027-1 converter
        // boards normalize EVERY output to "about 10 volts, peak to peak"
        // (2600 service manual 2.3.1-2.3.2): equal drive, pulse unipolar
        // 0..+10 V.
        let (amp_saw, amp_sine, amp_tri, amp_pulse) = match self.model {
            CircuitModel::Moog => (AMP_SAW, AMP_SINE, AMP_TRI, AMP_PULSE),
            CircuitModel::Arp => (PROGRAM_V, PROGRAM_V, PROGRAM_V, 2.0 * PROGRAM_V),
        };
        // The four converters, each computed only when audible. They all
        // read the SAME ramp (t), so every mixed combination is
        // phase-locked — one core, parallel converter boards, exactly the
        // hardware topology.
        let saw_out = |o: &Self| {
            let s = o.polyblep_saw(t, detuned_frequency) + sync_fix;
            // Integrator sag is a 901-era discrete-core trait; the
            // 4027-1 generation rides a cleaner, more linear ramp —
            // scaled by the same trim culture as skew and duty
            amp_saw * (s + o.curvature * o.trim() * (s * s - 1.0 / 3.0))
        };
        let pulse_out =
            |o: &Self| amp_pulse * (o.polyblep_pulse(t, detuned_frequency) + sync_fix);
        let tri_out =
            |o: &Self| amp_tri * o.fold_triangle(o.polyblep_triangle(t, detuned_frequency));
        let sine_out = |o: &Self| {
            let tri = o.fold_triangle(o.polyblep_triangle(t, detuned_frequency));
            let shaped = match o.model {
                // 901-B: the 1N34 diode-ladder rounding network
                CircuitModel::Moog => o.diode_sine(tri),
                // 4027-1: single-transistor peak rounding, no diode knee
                CircuitModel::Arp => Self::transistor_round(tri),
            };
            amp_sine * shaped
        };

        let mixer_on = self.mix.iter().any(|&m| m > 0.0);
        let raw_sample = if mixer_on {
            let mut s = 0.0;
            if self.mix[0] > 0.0 {
                s += self.mix[0] * saw_out(self);
            }
            if self.mix[1] > 0.0 {
                s += self.mix[1] * pulse_out(self);
            }
            if self.mix[2] > 0.0 {
                s += self.mix[2] * tri_out(self);
            }
            if self.mix[3] > 0.0 {
                s += self.mix[3] * sine_out(self);
            }
            s
        } else {
            match self.waveform {
                Waveform::Sawtooth => saw_out(self),
                Waveform::Square => pulse_out(self),
                Waveform::Triangle => tri_out(self),
                Waveform::Sine => sine_out(self),
            }
        };

        self.soft_clip(raw_sample)
    }

    /// Slightly asymmetric triangle, as if the fold network's two halves
    /// aren't perfectly matched. Endpoints stay at +/-1. The asymmetry is
    /// scaled by the maker's trim culture (R115 symmetry trimmer on ARP).
    #[inline]
    fn fold_triangle(&self, tri: f32) -> f32 {
        tri + self.skew * self.trim() * (1.0 - tri * tri)
    }

    /// The sine shaper, per the 901-B schematic (drawing 1101): the triangle
    /// feeds a ladder of 1N34 germanium diode pairs (D3-D6 with the
    /// 120/18K/270/510-ohm string) that conduct progressively, flattening the
    /// crests piecewise — plus the Q4 transistor rounding. So: a soft cubic
    /// base with a gentle diode KNEE above ~0.7, whose position is this
    /// unit's "sine purity" trim (P5/P6 on the drawing).
    #[inline]
    fn diode_sine(&self, tri: f32) -> f32 {
        let x = tri.clamp(-1.0, 1.0);
        let rounded = x * (1.5 - 0.5 * x * x);
        // Second diode pair conducts above the knee, shaving the crest with
        // a slightly harder (piecewise) characteristic than the cubic
        let knee = 0.72 + self.skew;
        let over = (rounded.abs() - knee).max(0.0);
        (rounded - rounded.signum() * over * 0.35) * 1.06
    }

    /// ARP sine converter: "Q3's nonlinearity rounds the peaks of the
    /// triangle to approximate a sine wave" — one smooth transistor curve,
    /// its residue set by the R121 purity trimmer (the per-unit skew).
    #[inline]
    fn transistor_round(tri: f32) -> f32 {
        let x = tri.clamp(-1.0, 1.0);
        x * (1.5 - 0.5 * x * x)
    }

    pub fn set_model(&mut self, model: CircuitModel) {
        self.model = model;
    }

    fn polyblep(&self, t: f32, dt: f32) -> f32 {
        if t < dt {
            let t = t / dt;
            2.0 * t - t * t - 1.0
        } else if t > 1.0 - dt {
            let t = (t - 1.0) / dt;
            t * t + 2.0 * t + 1.0
        } else {
            0.0
        }
    }

    /// Pulse converter, per circuit model.
    /// MOOG (901-B/901-C): the comparator swings between the +11.5 V and
    /// -6 V rails (drawing 1101), so top and bottom are asymmetric mirror
    /// images — a little real DC moves with PWM.
    /// ARP (4027-1): "CR7 rectifies the output of A8 so the pulse wave is
    /// positive going only" — fully unipolar, so PWM shifts the ladder's
    /// operating point substantially and even harmonics track the width.
    /// Both models' DC is eliminated after the filter, in the voice
    /// (ARP R162; the 2.5 uF output coupling on Moog dwg #1149).
    fn polyblep_pulse(&self, t: f32, frequency: f32) -> f32 {
        let (hi, lo) = match self.model {
            CircuitModel::Moog => (1.0f32, -0.92f32),
            CircuitModel::Arp => (1.0f32, 0.0f32),
        };
        let edge = (hi - lo) * 0.5;
        let dt = frequency / self.sample_rate;
        let naive = if t < self.duty { hi } else { lo };
        naive - edge * self.polyblep(t, dt)
            + edge * self.polyblep((t + 1.0 - self.duty) % 1.0, dt)
    }

    fn polyblep_saw(&self, t: f32, frequency: f32) -> f32 {
        let dt = frequency / self.sample_rate;
        let naive = 2.0 * t - 1.0;
        naive - self.polyblep(t, dt)
    }

    fn polyblep_triangle(&self, t: f32, frequency: f32) -> f32 {
        let dt = frequency / self.sample_rate;
        let naive = if t < 0.5 {
            4.0 * t - 1.0
        } else {
            3.0 - 4.0 * t
        };
        naive - self.integrate_polyblep(t, dt) + self.integrate_polyblep((t + 0.5) % 1.0, dt)
    }

    fn integrate_polyblep(&self, t: f32, dt: f32) -> f32 {
        if t < dt {
            let t = t / dt;
            dt * (t * t * t / 3.0 - t * t / 2.0 - t + 1.0 / 3.0)
        } else if t > 1.0 - dt {
            let t = (t - 1.0) / dt;
            dt * (-t * t * t / 3.0 + t * t + t + 1.0 / 3.0)
        } else {
            0.0
        }
    }

    fn soft_clip(&self, x: f32) -> f32 {
        match self.model {
            // 901-C output stage (drawing #1126), in volts: the push-pull
            // pair runs between +12 and -6 rails, so positive swings have
            // about twice the headroom of negative ones — hard-driven
            // waves flatten their lower lobe first (gentle even
            // harmonics). Cubic knees per side, transparent at the
            // alignment levels.
            CircuitModel::Moog => {
                if x >= 0.0 {
                    let x = x.min(12.0);
                    x * (1.0 - x * x / 432.0)
                } else {
                    let x = x.max(-9.5);
                    x * (1.0 - x * x / 270.75)
                }
            }
            // 4027-1 converter boards buffer through op-amps on +/-15 V
            // rails: LINEAR at the 10 V p-p program level (that is the
            // point of the design), with a short knee into the ~13.5 V
            // rail limit.
            CircuitModel::Arp => {
                let a = x.abs();
                if a <= 11.5 {
                    x
                } else {
                    x.signum() * (11.5 + 2.0 * ((a - 11.5) / 2.0).tanh())
                }
            }
        }
    }

    /// The sub square computed during the last `next_sample` call.
    pub fn sub(&self) -> f32 {
        self.last_sub
    }

    pub fn set_frequency(&self, frequency: f32) {
        self.frequency.store(frequency.to_bits(), Ordering::Relaxed);
    }

    pub fn set_freq_mult(&mut self, mult: f32) {
        self.freq_mult = mult;
    }

    pub fn set_waveform(&mut self, waveform: Waveform) {
        self.waveform = waveform;
    }

    pub fn note_to_frequency(note: u8) -> f32 {
        440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(waveform: Waveform) -> f32 {
        let mut osc = Oscillator::new(44100.0, 220.0, 42);
        osc.set_waveform(waveform);
        let mut peak = 0.0f32;
        for _ in 0..44100 {
            peak = peak.max(osc.next_sample(0.0, 1.0, 0.5, None).abs());
        }
        peak
    }

    /// 901B alignment: outputs are NOT normalized to one level. Triangle and
    /// pulse run hotter than saw; sine runs a touch quieter.
    #[test]
    fn service_target_amplitude_ratios() {
        let saw = peak(Waveform::Sawtooth);
        let tri = peak(Waveform::Triangle);
        let sine = peak(Waveform::Sine);
        let pulse = peak(Waveform::Square);
        assert!(tri > saw * 1.1, "triangle should run hot: {tri} vs saw {saw}");
        assert!(pulse > saw * 1.2, "pulse should run hot: {pulse} vs saw {saw}");
        assert!(sine < saw, "sine should be slightly quieter: {sine} vs {saw}");
    }

    /// The derived sine is a rounded triangle, not sin(): it must carry a
    /// few percent of harmonic residue.
    #[test]
    fn sine_is_imperfect() {
        let sr = 44100.0;
        let freq = 441.0; // exactly 100 samples per period
        let mut osc = Oscillator::new(sr, freq, 7);
        osc.set_waveform(Waveform::Sine);
        // Collect one steady period and correlate against a pure sinusoid
        let mut samples = Vec::new();
        for _ in 0..4410 {
            samples.push(osc.next_sample(0.0, 1.0, 0.5, None));
        }
        let period = &samples[4310..4410];
        // Fundamental amplitude via DFT bin
        let n = period.len() as f32;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (k, &s) in period.iter().enumerate() {
            let a = std::f32::consts::TAU * k as f32 / n;
            re += s * a.cos();
            im += s * a.sin();
        }
        let fundamental = 2.0 * (re * re + im * im).sqrt() / n;
        let total_rms = (period.iter().map(|s| s * s).sum::<f32>() / n).sqrt();
        let fund_rms = fundamental / std::f32::consts::SQRT_2;
        let residue = (total_rms * total_rms - fund_rms * fund_rms).max(0.0).sqrt();
        let thd = residue / fund_rms;
        assert!(
            thd > 0.005 && thd < 0.12,
            "transistor sine should have a few percent THD, got {thd}"
        );
    }

    /// The sub square runs at exactly half the core frequency and never
    /// drifts against it (shared phase increment).
    #[test]
    fn sub_is_locked_one_octave_down() {
        let sr = 44100.0;
        let mut osc = Oscillator::new(sr, 441.0, 5);
        osc.set_waveform(Waveform::Sawtooth);
        let mut core_wraps = 0;
        let mut sub_rises = 0;
        let mut prev_sub = 0.0f32;
        for _ in 0..44100 {
            osc.next_sample(0.0, 1.0, 0.5, None);
            // Core wrap: reported exactly by the core itself
            if osc.wrap_frac().is_some() {
                core_wraps += 1;
            }
            let sub = osc.sub();
            if prev_sub < 0.0 && sub > 0.0 {
                sub_rises += 1;
            }
            prev_sub = sub;
        }
        // 441 Hz core; the sub must sit at 220.5 Hz -> ~220 rising edges
        // in one second
        assert!(
            (430..=452).contains(&core_wraps),
            "core wraps {core_wraps}"
        );
        assert!(
            (210..=231).contains(&sub_rises),
            "sub should run at ~220 Hz: got {sub_rises} rises"
        );
    }

    /// 4027-1 converter boards normalize every waveform to ~10 V p-p
    /// (2600 service manual 2.3.1-2.3.2); the pulse is positive-going
    /// only, 0..+10 V. The Moog levels stay on the unequal 901B alignment
    /// ratios, so the two circuits load the filter differently.
    #[test]
    fn arp_waveforms_all_sit_at_ten_volts_pp() {
        let sr = 96000.0;
        for wf in [
            Waveform::Sawtooth,
            Waveform::Square,
            Waveform::Triangle,
            Waveform::Sine,
        ] {
            let mut osc = Oscillator::new(sr, 220.0, 5);
            osc.set_model(CircuitModel::Arp);
            osc.set_waveform(wf);
            let mut samples: Vec<f32> = (0..(sr as usize))
                .map(|_| osc.next_sample(0.0, 1.0, 0.5, None))
                .collect();
            samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
            // Rails read like a scope: percentiles ignore the bandlimited
            // edge ripple (which a real edge has too, above the trace)
            let lo = samples[(samples.len() as f32 * 0.05) as usize];
            let hi = samples[(samples.len() as f32 * 0.95) as usize];
            let pp = hi - lo;
            assert!(
                (8.4..=11.5).contains(&pp),
                "{wf:?} should be ~10 V p-p on the ARP converters, got {pp:.2}"
            );
            if wf == Waveform::Square {
                assert!(
                    lo > -0.6,
                    "ARP pulse is positive-going only (CR14), low rail {lo:.2}"
                );
            }
        }
    }

    /// The 901B's residual tuning error is additive in HERTZ (tracking
    /// resistors shift cycles regardless of control voltage), so a Moog
    /// bank's beat rate is roughly constant across the keyboard — and the
    /// trimmed 4027-1 shows far less of it.
    #[test]
    fn moog_tracking_error_is_additive_hertz() {
        let sr = 44100.0;
        let measure_offset = |model: CircuitModel, f0: f32, seed: u32| -> f32 {
            let mut osc = Oscillator::new(sr, f0, seed);
            osc.set_model(model);
            osc.set_waveform(Waveform::Sawtooth);
            let secs = 8.0;
            let n = (secs * sr) as usize;
            let mut wraps = 0u32;
            for _ in 0..n {
                osc.next_sample(0.0, 1.0, 0.5, None);
                if osc.wrap_frac().is_some() {
                    wraps += 1;
                }
            }
            wraps as f32 / secs - f0
        };
        let mut moog_low = 0.0f32;
        let mut moog_high = 0.0f32;
        let mut arp_low = 0.0f32;
        let seeds = [3u32, 17, 29, 41, 53, 67];
        for &s in &seeds {
            moog_low += measure_offset(CircuitModel::Moog, 55.0, s).abs();
            moog_high += measure_offset(CircuitModel::Moog, 880.0, s).abs();
            arp_low += measure_offset(CircuitModel::Arp, 55.0, s).abs();
        }
        let k = seeds.len() as f32;
        let (moog_low, moog_high, arp_low) = (moog_low / k, moog_high / k, arp_low / k);
        // Same Hz-scale error at both ends of the keyboard (not cents)
        assert!(
            moog_low > 0.04 && moog_high > 0.04 && moog_high < 4.0 * moog_low,
            "Moog offset should be Hz-additive: low {moog_low:.3} Hz, high {moog_high:.3} Hz"
        );
        assert!(
            arp_low < 0.5 * moog_low,
            "trimmed 4027-1 should sit much closer to true: arp {arp_low:.3} Hz vs moog {moog_low:.3} Hz"
        );
    }

    /// The source mixer draws every waveform from the ONE ramp core, so a
    /// mix equals the sum of the solo waveforms from identically seeded
    /// cores — phase-locked by construction, no beating second voice.
    #[test]
    fn source_mixer_is_phase_locked() {
        let sr = 48000.0;
        let run = |mix: [f32; 4]| -> Vec<f32> {
            let mut o = Oscillator::new(sr, 220.0, 5);
            o.set_mix(mix);
            (0..9600).map(|_| o.next_sample(0.0, 1.0, 0.5, None)).collect()
        };
        // Low levels keep the output stage in its linear region
        let mixed = run([0.3, 0.2, 0.0, 0.0]);
        let saw = run([0.3, 0.0, 0.0, 0.0]);
        let pulse = run([0.0, 0.2, 0.0, 0.0]);
        let mut worst = 0.0f32;
        for i in 0..mixed.len() {
            worst = worst.max((mixed[i] - (saw[i] + pulse[i])).abs());
        }
        assert!(
            worst < 0.05,
            "mix must equal the sum of phase-locked components, worst diff {worst}"
        );
        // all-zero mix = classic selector behavior
        let sel = {
            let mut o = Oscillator::new(sr, 220.0, 5);
            o.set_waveform(Waveform::Sawtooth);
            (0..960).map(|_| o.next_sample(0.0, 1.0, 0.5, None)).collect::<Vec<_>>()
        };
        let mix_off = {
            let mut o = Oscillator::new(sr, 220.0, 5);
            o.set_waveform(Waveform::Sawtooth);
            o.set_mix([0.0; 4]);
            (0..960).map(|_| o.next_sample(0.0, 1.0, 0.5, None)).collect::<Vec<_>>()
        };
        assert_eq!(sel, mix_off);
    }

    /// Hard sync locks the slave's periodicity to the master: with sync,
    /// the slave's waveform must repeat at the MASTER's period even though
    /// its own rate is a non-integer multiple.
    #[test]
    fn hard_sync_locks_to_master_period() {
        let sr = 44100.0;
        let f_master = 441.0; // exactly 100 samples per period
        let run = |use_sync: bool| -> f32 {
            let mut master = Oscillator::new(sr, f_master, 2);
            let mut slave = Oscillator::new(sr, f_master * 1.37, 9);
            master.set_waveform(Waveform::Sawtooth);
            slave.set_waveform(Waveform::Sawtooth);
            let mut out = Vec::with_capacity(8820);
            for _ in 0..8820 {
                master.next_sample(0.0, 1.0, 0.5, None);
                let s = if use_sync { master.wrap_frac() } else { None };
                out.push(slave.next_sample(0.0, 1.0, 0.5, s));
            }
            // Mismatch of the waveform against itself one master period on
            let period = 100usize;
            let mut err = 0.0f32;
            let mut norm = 1e-9f32;
            for n in 4410..(8820 - period) {
                err += (out[n] - out[n + period]).abs();
                norm += out[n].abs();
            }
            err / norm
        };
        let synced = run(true);
        let free = run(false);
        assert!(
            synced < 0.1,
            "synced slave should repeat at the master period, mismatch {synced}"
        );
        assert!(
            free > 0.3,
            "free-running slave at ratio 1.37 should not, mismatch {free}"
        );
    }

    /// PWM moves the DC operating point in both circuit models: the Moog
    /// pulse is bipolar-asymmetric (rail-limited), the ARP pulse is fully
    /// unipolar (rectified), so its mean IS the duty cycle.
    #[test]
    fn pulse_width_shifts_duty() {
        let measure = |width: f32, model: CircuitModel| -> f32 {
            let mut osc = Oscillator::new(44100.0, 441.0, 3);
            osc.set_waveform(Waveform::Square);
            osc.set_model(model);
            let mut sum = 0.0f32;
            let n = 44100;
            for _ in 0..n {
                sum += osc.next_sample(0.0, 1.0, width, None);
            }
            sum / n as f32
        };
        // Moog: mean crosses zero around center width
        let m_narrow = measure(0.2, CircuitModel::Moog);
        let m_wide = measure(0.8, CircuitModel::Moog);
        assert!(m_narrow < -0.4 && m_wide > 0.4, "Moog: {m_narrow}..{m_wide}");
        // ARP: strictly positive, monotone in duty
        let a_narrow = measure(0.2, CircuitModel::Arp);
        let a_center = measure(0.5, CircuitModel::Arp);
        let a_wide = measure(0.8, CircuitModel::Arp);
        assert!(a_narrow > 0.0, "ARP pulse mean must be positive: {a_narrow}");
        assert!(
            a_narrow < a_center && a_center < a_wide,
            "ARP mean must track duty: {a_narrow} < {a_center} < {a_wide}"
        );
    }
}
