// src/buscomp.rs — the mix-bus compressor
//
// A circuit model of the stereo bus compressor in the centre section of
// the SSL 4000 E/G-series console: the compressor strapped across the mix
// bus of a great many 1980s records, and the origin of the word "glue".
// Chosen because it is THE console bus compressor of the cassette era — a
// mix printed through it onto a two-track or a cassette master is the sound
// the reference recordings have: transients tucked into the bed instead of
// landing on top of it, crest factor in the low teens instead of the high
// teens. The circuit facts relied on are the widely documented ones:
//
//   TOPOLOGY      feed-forward: the sidechain listens to the compressor's
//                 INPUT, and one control voltage drives a VCA in each
//                 channel. Stereo-linked: both channels feed one timing
//                 circuit, so the image never leans under compression.
//   GAIN CELL     a dbx 202-family Blackmer log-antilog VCA. Its gain is
//                 exponential in its control voltage (a constant mV per
//                 dB), so the whole control path is a DECIBEL computer:
//                 threshold, ratio and every time constant act on dB.
//   FRONT PANEL   continuous threshold; ratio switched 2:1 / 4:1 / 10:1;
//                 attack switched 0.1, 0.3, 1, 3, 10, 30 ms; release
//                 switched 0.1, 0.3, 0.6, 1.2 s and AUTO; make-up gain
//                 0..+15 dB; an IN switch; a moving-coil meter reading
//                 gain reduction straight off the control voltage.
//
// Two controls the original card does not have are added, each a common
// later modification that earns its place in THIS instrument: a MIX
// control (parallel compression: the dry bus summed back under the VCA
// output), and a sidechain HIGH-PASS, because a 909 kick on the same bus
// otherwise pumps everything above it. Both are transparent at their home
// positions (mix 100%, 20 Hz).
//
// The console's bus runs at a calibrated operating level; Patina's bus
// does not (songs peak anywhere from -21 to -4 dBFS), so the threshold dial
// is calibrated in dBFS PEAK and spans -48..0 rather than the hardware's
// +-15 dB about its reference.
//
// The mechanisms, in signal order:
//
//   SIDECHAIN FILTER  a 2-pole (Butterworth) state-variable high-pass in
//                     front of each rectifier — the modification above. At
//                     its 20 Hz home position it stands in for the card's
//                     input coupling, which never let DC or subsonics steer
//                     the gain.
//
//   RECTIFIER         a precision full-wave rectifier per channel, the two
//                     outputs diode-ORed onto the one detector: the louder
//                     side drives both VCAs. That IS the stereo link — a hit
//                     on one side ducks both sides by the same dB. Peak-
//                     reading, like the card.
//
//   LOG CONVERTER     the rectified signal, raised to the converter's power
//     & THRESHOLD     law, is summed with a threshold current and the SUM is
//                     converted to a voltage by a transistor's exponential
//                     law. The log of a sum is where the soft knee comes
//                     from: log(I_sig + I_thr) - log(I_thr) is, in dB, a
//                     softplus of (level - threshold) — a smooth corner with
//                     the full ratio above it and nothing below it. No knee
//                     knob, no knee "shape": the knee is the converter's
//                     physics. With a 4th-power law the knee is exactly
//                     KNEE_DB * ln(1 + (peak / threshold)^4), KNEE_DB =
//                     5/ln 10 = 2.17 dB wide, and the detector needs no log
//                     or exponential at all.
//
//   RATIO             the switch scales the above-threshold voltage by
//                     (1 - 1/R). That is exactly R:1 above the knee, because
//                     the VCA subtracts it in dB.
//
//   TIMING            the dB control voltage lives on a timing capacitor.
//                     Attack: an op-amp charges it through a diode and the
//                     attack resistor whenever the computed reduction exceeds
//                     it. Release: a second, diode-steered path relaxes it
//                     back toward the computed reduction through the release
//                     resistor. Because this happens in dB, a release is a
//                     constant fraction of the remaining dB per unit time at
//                     any depth, and a fast attack on bass follows the
//                     waveform — the real unit's distortion, not a bug.
//
//   AUTO RELEASE      the AUTO position swaps the release resistor for the
//                     classic dual-time-constant network: the timing cap C1
//                     bleeds through R_F into a second, larger cap C2, and
//                     only C2 returns to the computed reduction (through
//                     R_S). A short transient charges C1 but barely touches
//                     C2, so the gain recovers on R_F*C1 (fast); sustained
//                     compression charges C2 as well, and then the pair
//                     recovers together on R_S*(C1+C2) (slow). Program-
//                     dependent release, from charge sharing — solved
//                     exactly per sample (charge conservation plus the
//                     closed-form decay of the difference), so it is stable
//                     and rate-independent at any sample rate.
//
//   CONTROL PORT      a small RC on the VCA's control input (CV_PORT_HZ)
//                     rounds the fastest attack edges before they reach the
//                     cell. With the attack RC in front of it, it band-limits
//                     the gain modulation itself: the gain riding a pumping
//                     bass line leaves no alias product above -100 dBc on a
//                     17 kHz tone. What does remain at the two fastest attack
//                     positions is the sampled peak detector folding HF key
//                     content (-70 dBc at 0.1 ms, -80 dBc from 1 ms, i.e.
//                     ~-90 dBFS on real program); see
//                     `fast_gain_modulation_does_not_alias`.
//
//   VCA               out = G_npn*I_npn - G_pnp*I_pnp. The cell is class-AB
//                     and translinear: the input current splits between its
//                     NPN and PNP halves with I_npn*I_pnp = IQ^2, so
//                     I_npn - I_pnp = I and I_npn + I_pnp = sqrt(I^2 + 4 IQ^2)
//                     exactly. The halves' dB-per-volt constants differ by a
//                     fraction of a percent (VCA_MISMATCH), so away from unity
//                     gain their gains differ by an amount PROPORTIONAL TO
//                     THE REDUCTION. That one mismatch is the cell's whole
//                     coloration. (G_npn - G_pnp)*sqrt(I^2 + 4 IQ^2)/2 is a
//                     smooth even-order term — second harmonic ~0.1% at
//                     10 dB of reduction, exactly none at unity gain, where
//                     the symmetry trim nulls it — and its quiescent part is
//                     a DC step that follows the gain: CONTROL-VOLTAGE
//                     FEEDTHROUGH, ~-96 dBFS at 10 dB of reduction, which the
//                     bus's DC servo removes downstream. The even term is
//                     antialiased with its exact antiderivative (first-order
//                     ADAA, as adaa.rs does for tanh). Each channel's cell
//                     has its own mismatch, like two real chips.
//
//   MAKE-UP & MIX     make-up gain after the VCA; the MIX control blends the
//                     made-up VCA output against the untouched bus.
//
// The unit is a WIRE until the IN switch is thrown — bit-identical output,
// not "very close". The IN switch routes the audio, not the sidechain: the
// detector listens while the unit is out, so the meter shows what IN would
// take and throwing IN lands on a settled control voltage. Thrown
// mid-program it crossfades over ENGAGE_S, so it never clicks; set before
// the first sample (a song that opens with the unit in) it is simply in.
// While the unit is audible every continuous control glides (threshold and
// the HPF like the pots they are, make-up and mix like gain controls) and
// the ratio switch glides its slope, so nothing steps the gain; while it is
// not, the controls land at once.
//
// Left out on purpose: the cell's own noise (~-95 dB below its clip point
// on the hardware; Patina's bus has no calibrated relation to that clip
// point, and the cassette's hiss sits 40 dB above it anyway), and the
// VCA's temperature coefficient (0.33%/K of its dB scale — a fraction of a
// dB of ratio drift over a warm-up, below what a meter shows).

use crate::smoothing::{approach, one_pole, GAIN_SMOOTH_S, KNOB_SMOOTH_S};
use std::f32::consts::{LN_10, PI};

// --- Front panel ---
/// Ratio switch positions.
pub const RATIOS: [f32; 3] = [2.0, 4.0, 10.0];
/// Attack switch positions, seconds.
pub const ATTACKS_S: [f32; 6] = [0.0001, 0.0003, 0.001, 0.003, 0.01, 0.03];
/// Release switch positions, seconds; the index past the end is AUTO.
pub const RELEASES_S: [f32; 4] = [0.1, 0.3, 0.6, 1.2];
/// The release switch's AUTO position.
pub const RELEASE_AUTO: usize = RELEASES_S.len();
/// Sidechain high-pass range, Hz.
pub const SC_HPF_MIN_HZ: f32 = 20.0;
pub const SC_HPF_MAX_HZ: f32 = 400.0;

// --- Detector ---
/// Width of the log converter's knee, dB: 20 / (4 ln 10), from the
/// converter's 4th-power law. At the threshold itself the reduction is
/// (1 - 1/R) * KNEE_DB * ln 2 = 1.5 dB * (1 - 1/R).
pub const KNEE_DB: f32 = 5.0 / LN_10;
/// Above this the converter is saturated (+120 dB over threshold); below
/// the floor it reads nothing (-100 dB under it). Keeps the 4th power
/// finite and out of denormals.
const KEY_CEILING: f32 = 1e6;
const KEY_FLOOR: f32 = 1e-5;

// --- AUTO release network (see the header) ---
// Component values chosen so the network's two limits land on the release
// switch's own extremes: R_F*C1 = 0.1 s after a transient, R_S*(C1+C2)
// = 1.2 s after sustained compression. C1 is the same timing capacitor the
// fixed positions use. kOhm * uF = ms.
const C1_UF: f32 = 1.0;
const C2_UF: f32 = 4.7;
const R_F_KOHM: f32 = 100.0;
const R_S_KOHM: f32 = 1200.0 / (C1_UF + C2_UF);

// --- VCA ---
/// Control-port RC corner, Hz.
const CV_PORT_HZ: f32 = 8000.0;
/// Class-AB quiescent current of the cell, in bus units (full scale = 1).
const IQ: f64 = 0.01;
/// Fractional mismatch between each cell's NPN and PNP dB-per-volt
/// constants — one per channel, as two chips differ.
const VCA_MISMATCH: [f32; 2] = [0.0042, 0.0031];

// --- Engagement and glides ---
/// The IN switch crossfades over this long (same as the tape deck's).
const ENGAGE_S: f32 = 0.01;
/// Ratio-switch glide: long enough that no switch throw steps the gain.
const RATIO_GLIDE_S: f32 = 0.01;

/// dB per neper: 20 / ln 10.
const DB_PER_NEPER: f32 = 20.0 / LN_10;

/// Snap states that have decayed past any consequence to zero, so a long
/// silence never leaves the detector or the sidechain running on
/// denormals. 1e-20 is far below every quantity these states carry.
#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

#[inline]
fn db_to_gain(db: f32) -> f32 {
    (db / DB_PER_NEPER).exp()
}

/// The compressor's static curve: steady gain reduction in dB for a signal
/// whose PEAK sits `over_db` above the threshold, at ratio `ratio`.
pub fn static_reduction_db(over_db: f32, ratio: f32) -> f32 {
    (1.0 - 1.0 / ratio) * KNEE_DB * 10f32.powf(over_db / 5.0).ln_1p()
}

/// Topology-preserving state-variable high-pass (Butterworth, Q = 1/sqrt 2).
/// Its coefficients can move every sample without disturbing its state.
#[derive(Clone, Copy)]
struct SidechainHpf {
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: f32,
    ic2: f32,
}

impl SidechainHpf {
    const K: f32 = std::f32::consts::SQRT_2;

    fn new(cutoff: f32, sample_rate: f32) -> Self {
        let mut f = Self {
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            ic1: 0.0,
            ic2: 0.0,
        };
        f.tune(cutoff, sample_rate);
        f
    }

    fn tune(&mut self, cutoff: f32, sample_rate: f32) {
        // Pinned under Nyquist so the lowest supported rates stay stable
        let g = (PI * cutoff.min(0.45 * sample_rate) / sample_rate).tan();
        self.a1 = 1.0 / (1.0 + g * (g + Self::K));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        x - Self::K * v1 - v2
    }
}

/// One Blackmer gain cell: exponential gain, and the half-to-half mismatch
/// that is its only coloration (see the header).
#[derive(Clone, Copy)]
struct GainCell {
    mismatch: f32,
    /// Previous input and the antiderivative of the halves' sum there
    /// (first-order ADAA state).
    x1: f64,
    sum_integral1: f64,
}

impl GainCell {
    const TWO_IQ: f64 = 2.0 * IQ;

    fn new(mismatch: f32) -> Self {
        Self {
            mismatch,
            x1: 0.0,
            sum_integral1: Self::sum_integral(0.0),
        }
    }

    /// I_npn + I_pnp for a signal current `x`: sqrt(x^2 + 4 IQ^2).
    #[inline]
    fn halves_sum(x: f64) -> f64 {
        x.hypot(Self::TWO_IQ)
    }

    /// Its antiderivative: (x*sqrt(x^2 + c^2) + c^2 * asinh(x / c)) / 2.
    #[inline]
    fn sum_integral(x: f64) -> f64 {
        let c = Self::TWO_IQ;
        0.5 * (x * Self::halves_sum(x) + c * c * (x / c).asinh())
    }

    /// Take `x` as the previous input, so the first quotient after the unit
    /// is switched in spans one real step.
    fn seed(&mut self, x: f32) {
        self.x1 = x as f64;
        self.sum_integral1 = Self::sum_integral(self.x1);
    }

    #[inline]
    fn process(&mut self, x: f32, gain: f32, reduction_db: f32) -> f32 {
        let xd = x as f64;
        let integral = Self::sum_integral(xd);
        let dx = xd - self.x1;
        let halves = if dx.abs() > 1e-5 {
            (integral - self.sum_integral1) / dx
        } else {
            Self::halves_sum(0.5 * (xd + self.x1))
        };
        self.x1 = xd;
        self.sum_integral1 = integral;
        // G_npn and G_pnp sit `mismatch * reduction` dB apart; to first
        // order (a fraction of a percent) their half-difference relative
        // to the common gain is this
        let asymmetry = -0.5 * self.mismatch * reduction_db / DB_PER_NEPER;
        gain * (x + asymmetry * halves as f32)
    }
}

pub struct BusComp {
    sample_rate: f32,

    // The front panel, as set
    engaged: bool,
    threshold_db: f32,
    ratio: usize,
    attack: usize,
    release: usize,
    makeup_db: f32,
    mix: f32,
    sc_hpf_hz: f32,
    /// The ratio switch as a control-voltage scale, 1 - 1/R, and the
    /// make-up as a linear gain: what the glides below head for.
    slope_target: f32,
    makeup_target: f32,

    // The panel as the circuit hears it: pots and gain controls glide
    threshold_heard: f32,
    /// 1 / threshold, linear: the threshold current's scale.
    inv_threshold: f32,
    slope_heard: f32,
    makeup_heard: f32, // linear
    mix_heard: f32,
    sc_hpf_heard: f32,
    knob_k: f32,
    gain_k: f32,
    ratio_k: f32,

    // Sidechain and detector
    sc_hpf: [SidechainHpf; 2],
    /// Per-sample charge coefficient of the attack path.
    attack_k: f32,
    /// Per-sample coefficient of the fixed release path.
    release_k: f32,
    /// AUTO network: the per-sample decay of the C1-C2 voltage difference,
    /// each capacitor's share of the total capacitance, and C2's return.
    share_decay: f32,
    c1_share: f32,
    c2_share: f32,
    slow_k: f32,
    /// Timing capacitor C1 (THE control voltage, dB of reduction) and the
    /// AUTO network's C2, dB.
    cv_c1: f32,
    cv_c2: f32,
    /// The control voltage at the VCA's port, after its RC.
    cv_port: f32,
    port_k: f32,

    cells: [GainCell; 2],

    // Engagement
    engage: f32,
    engage_step: f32,
    /// Whether any audio has passed yet. Before it has, there is nothing
    /// to crossfade from: a unit switched in before the first sample (a
    /// song or patch that starts with it IN) is simply in.
    rolling: bool,
}

impl BusComp {
    pub fn new(sample_rate: f32) -> Self {
        let dt = 1.0 / sample_rate;
        let tau_c1 = R_F_KOHM * C1_UF * 1e-3;
        let tau_c2 = R_F_KOHM * C2_UF * 1e-3;
        let tau_slow = R_S_KOHM * C2_UF * 1e-3;
        let mut comp = Self {
            sample_rate,
            engaged: false,
            threshold_db: -20.0,
            ratio: 1,
            attack: 4,
            release: RELEASE_AUTO,
            makeup_db: 0.0,
            mix: 1.0,
            sc_hpf_hz: SC_HPF_MIN_HZ,
            slope_target: 1.0 - 1.0 / RATIOS[1],
            makeup_target: 1.0,
            threshold_heard: -20.0,
            inv_threshold: db_to_gain(20.0),
            slope_heard: 1.0 - 1.0 / RATIOS[1],
            makeup_heard: 1.0,
            mix_heard: 1.0,
            sc_hpf_heard: SC_HPF_MIN_HZ,
            knob_k: approach(KNOB_SMOOTH_S, sample_rate),
            gain_k: approach(GAIN_SMOOTH_S, sample_rate),
            ratio_k: approach(RATIO_GLIDE_S, sample_rate),
            sc_hpf: [SidechainHpf::new(SC_HPF_MIN_HZ, sample_rate); 2],
            attack_k: 0.0,
            release_k: 0.0,
            share_decay: (-dt * (1.0 / tau_c1 + 1.0 / tau_c2)).exp(),
            c1_share: C1_UF / (C1_UF + C2_UF),
            c2_share: C2_UF / (C1_UF + C2_UF),
            slow_k: approach(tau_slow, sample_rate),
            cv_c1: 0.0,
            cv_c2: 0.0,
            cv_port: 0.0,
            port_k: one_pole(CV_PORT_HZ, sample_rate),
            cells: [
                GainCell::new(VCA_MISMATCH[0]),
                GainCell::new(VCA_MISMATCH[1]),
            ],
            engage: 0.0,
            engage_step: 1.0 / (ENGAGE_S * sample_rate).max(1.0),
            rolling: false,
        };
        comp.set_attack(comp.attack);
        comp.set_release(comp.release);
        comp
    }

    // --- Front panel -------------------------------------------------------
    // Every setter stores a value or a time constant; none rebuilds or
    // resets anything, so re-asserting a setting every block is free.
    // Non-finite values are refused: one NaN on a timing capacitor would
    // hold the gain at NaN for good.

    /// The IN switch. Thrown mid-program it crossfades over ENGAGE_S;
    /// thrown before the first sample it is simply in.
    pub fn set_engaged(&mut self, on: bool) {
        self.engaged = on;
        if !self.rolling {
            self.engage = if on { 1.0 } else { 0.0 };
        }
    }

    /// Threshold, dBFS peak.
    pub fn set_threshold(&mut self, db: f32) {
        if db.is_finite() {
            self.threshold_db = db;
        }
    }

    /// Ratio switch position: 0 = 2:1, 1 = 4:1, 2 = 10:1.
    pub fn set_ratio(&mut self, index: usize) {
        self.ratio = index.min(RATIOS.len() - 1);
        self.slope_target = 1.0 - 1.0 / RATIOS[self.ratio];
    }

    /// Attack switch position, 0..=5 (0.1 ms .. 30 ms).
    pub fn set_attack(&mut self, index: usize) {
        self.attack = index.min(ATTACKS_S.len() - 1);
        self.attack_k = approach(ATTACKS_S[self.attack], self.sample_rate);
    }

    /// Release switch position, 0..=3 (0.1 s .. 1.2 s), 4 = AUTO.
    pub fn set_release(&mut self, index: usize) {
        self.release = index.min(RELEASE_AUTO);
        if self.release < RELEASE_AUTO {
            self.release_k = approach(RELEASES_S[self.release], self.sample_rate);
        }
    }

    /// Make-up gain, dB.
    pub fn set_makeup(&mut self, db: f32) {
        if db.is_finite() && db != self.makeup_db {
            self.makeup_db = db;
            self.makeup_target = db_to_gain(db);
        }
    }

    /// Parallel mix: 0 = the dry bus, 1 = the compressor alone.
    pub fn set_mix(&mut self, mix: f32) {
        if mix.is_finite() {
            self.mix = mix.clamp(0.0, 1.0);
        }
    }

    /// Sidechain high-pass corner, Hz.
    pub fn set_sc_hpf(&mut self, hz: f32) {
        if hz.is_finite() {
            self.sc_hpf_hz = hz.clamp(SC_HPF_MIN_HZ, SC_HPF_MAX_HZ);
        }
    }

    /// The meter: gain reduction in dB (positive = reducing), read off the
    /// control voltage at the VCA port exactly as the card's moving-coil
    /// meter is. The sidechain listens while the unit is out too, so this
    /// reads what the unit WOULD take — set the threshold by it, then
    /// throw IN.
    pub fn gain_reduction_db(&self) -> f32 {
        self.cv_port
    }

    // --- The circuit -------------------------------------------------------

    /// Everything the circuit hears jumps to the panel. Used whenever the
    /// unit is not audible — out, or at the instant IN is thrown — because
    /// a glide only exists to keep an AUDIBLE move from zipping: a song
    /// that switches the unit in with its threshold set must not have its
    /// opening hit pass under a threshold still gliding from power-on.
    #[inline]
    fn settle_controls(&mut self) {
        if self.threshold_heard != self.threshold_db {
            self.threshold_heard = self.threshold_db;
            self.inv_threshold = db_to_gain(-self.threshold_db);
        }
        self.slope_heard = self.slope_target;
        self.makeup_heard = self.makeup_target;
        self.mix_heard = self.mix;
        if self.sc_hpf_heard != self.sc_hpf_hz {
            self.sc_hpf_heard = self.sc_hpf_hz;
            for f in &mut self.sc_hpf {
                f.tune(self.sc_hpf_hz, self.sample_rate);
            }
        }
    }

    #[inline]
    fn glide_controls(&mut self) {
        if self.threshold_heard != self.threshold_db {
            self.threshold_heard =
                glide_to(self.threshold_heard, self.threshold_db, self.knob_k, 1e-4);
            self.inv_threshold = db_to_gain(-self.threshold_heard);
        }
        self.slope_heard = glide_to(self.slope_heard, self.slope_target, self.ratio_k, 1e-7);
        self.makeup_heard = glide_to(self.makeup_heard, self.makeup_target, self.gain_k, 1e-7);
        self.mix_heard = glide_to(self.mix_heard, self.mix, self.gain_k, 1e-7);
        if self.sc_hpf_heard != self.sc_hpf_hz {
            // A pot sweeping a frequency: glide in octaves
            let next = self.sc_hpf_heard * (self.sc_hpf_hz / self.sc_hpf_heard).powf(self.knob_k);
            self.sc_hpf_heard = if (next / self.sc_hpf_hz - 1.0).abs() < 1e-4 {
                self.sc_hpf_hz
            } else {
                next
            };
            for f in &mut self.sc_hpf {
                f.tune(self.sc_hpf_heard, self.sample_rate);
            }
        }
    }

    /// The reduction the log converter and ratio switch ask for, dB, given
    /// the diode-ORed rectifier output.
    #[inline]
    fn computed_reduction(&self, rectified: f32) -> f32 {
        let u = (rectified * self.inv_threshold).min(KEY_CEILING);
        if u < KEY_FLOOR {
            return 0.0;
        }
        let u2 = u * u;
        self.slope_heard * KNEE_DB * (u2 * u2).ln_1p()
    }

    /// The detector: sidechain, rectifiers, log converter, ratio and the
    /// timing network. Returns the control voltage at the VCA port, dB.
    #[inline]
    fn detect(&mut self, left: f32, right: f32) -> f32 {
        // Precision rectifiers, diode-ORed: the louder side wins
        let sl = self.sc_hpf[0].process(left).abs();
        let sr = self.sc_hpf[1].process(right).abs();
        let target = self.computed_reduction(sl.max(sr));

        // Attack: diode-steered charge of C1 toward the computed reduction
        if target > self.cv_c1 {
            self.cv_c1 += self.attack_k * (target - self.cv_c1);
        }
        if self.release == RELEASE_AUTO {
            // C1 and C2 share charge through R_F: the charge-weighted mean
            // is conserved, their difference decays in closed form
            let mean = self.c1_share * self.cv_c1 + self.c2_share * self.cv_c2;
            let diff = (self.cv_c1 - self.cv_c2) * self.share_decay;
            self.cv_c1 = mean + self.c2_share * diff;
            self.cv_c2 = mean - self.c1_share * diff;
            // Only C2 returns to the computed reduction, through R_S
            if target < self.cv_c2 {
                self.cv_c2 += self.slow_k * (target - self.cv_c2);
            }
        } else {
            if target < self.cv_c1 {
                self.cv_c1 += self.release_k * (target - self.cv_c1);
            }
            // Out of circuit, C2 is held at C1's voltage, so throwing the
            // switch to AUTO connects it without a charge-sharing jump
            self.cv_c2 = self.cv_c1;
        }
        self.cv_c1 = flush(self.cv_c1);
        self.cv_c2 = flush(self.cv_c2);
        self.cv_port = flush(self.cv_port + self.port_k * (self.cv_c1 - self.cv_port));
        self.cv_port
    }

    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let first_sample = !self.rolling;
        self.rolling = true;
        // Nothing non-finite reaches the timing capacitors
        let l = if left.is_finite() { left } else { 0.0 };
        let r = if right.is_finite() { right } else { 0.0 };
        // The sidechain is always powered: the IN switch routes the AUDIO.
        // So the meter shows what the unit would do while it is out, and
        // throwing IN lands on a control voltage that has already settled
        // on the program instead of a discharged one (which, with a slow
        // attack and the make-up up, swelled the bus for 30 ms).
        if first_sample || self.engage == 0.0 {
            self.settle_controls();
        } else {
            self.glide_controls();
        }
        let reduction = self.detect(l, r);
        if !self.engaged && self.engage == 0.0 {
            // Out: a wire
            return (left, right);
        }
        if self.engage == 0.0 {
            // Switching in: the cells' antialiasing starts from this input
            self.cells[0].seed(l);
            self.cells[1].seed(r);
        }
        self.engage = if self.engaged {
            (self.engage + self.engage_step).min(1.0)
        } else {
            (self.engage - self.engage_step).max(0.0)
        };

        let gain = db_to_gain(-reduction);
        let wet_l = self.cells[0].process(l, gain, reduction) * self.makeup_heard;
        let wet_r = self.cells[1].process(r, gain, reduction) * self.makeup_heard;
        let out_l = l + self.mix_heard * (wet_l - l);
        let out_r = r + self.mix_heard * (wet_r - r);
        (l + self.engage * (out_l - l), r + self.engage * (out_r - r))
    }
}

/// Exponential glide that lands exactly once within `snap` of the target:
/// a gain control must be able to reach 0 or 1 exactly (mix 0 is the dry
/// bus to the bit).
#[inline]
fn glide_to(current: f32, target: f32, k: f32, snap: f32) -> f32 {
    let next = current + k * (target - current);
    if (next - target).abs() < snap {
        target
    } else {
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    const FS: f32 = 48000.0;

    fn db(x: f32) -> f32 {
        20.0 * x.log10()
    }

    fn amp(db: f32) -> f32 {
        10f32.powf(db / 20.0)
    }

    /// Amplitude of the `freq` component of `x`, Hann-windowed.
    fn tone_amplitude(x: &[f32], sr: f32, freq: f32) -> f32 {
        let n = x.len();
        let w = TAU * freq as f64 / sr as f64;
        let c = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for (i, &v) in x.iter().enumerate() {
            let win = 0.5 - 0.5 * (TAU * i as f64 / n as f64).cos();
            let s0 = v as f64 * win + c * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let p = (s1 * s1 + s2 * s2 - c * s1 * s2).max(0.0).sqrt();
        (4.0 * p / n as f64) as f32
    }

    fn sine_at(sr: f32, freq: f32, amplitude: f32, n: usize) -> f32 {
        amplitude * (TAU * freq as f64 * n as f64 / sr as f64).sin() as f32
    }

    /// A unit switched in at these panel settings.
    fn comp(sr: f32, threshold: f32, ratio: usize, attack: usize, release: usize) -> BusComp {
        let mut c = BusComp::new(sr);
        c.set_threshold(threshold);
        c.set_ratio(ratio);
        c.set_attack(attack);
        c.set_release(release);
        c.set_engaged(true);
        c
    }

    /// Largest sample-to-sample step over `range` of `y`.
    fn max_step(y: &[f32], range: std::ops::Range<usize>) -> f32 {
        range.map(|n| (y[n] - y[n - 1]).abs()).fold(0.0, f32::max)
    }

    /// Where a rising trace first crosses `level`, in seconds, with linear
    /// interpolation. Sample i holds the state at time (i + 1) / sr.
    fn crossing(trace: &[f32], level: f32, sr: f32) -> f32 {
        let i = trace
            .iter()
            .position(|&v| v >= level)
            .expect("never crossed");
        let prev = if i == 0 { 0.0 } else { trace[i - 1] };
        let frac = (level - prev) / (trace[i] - prev);
        (i as f32 + frac) / sr
    }

    /// Where a falling trace (sample i at time (i + 1) / sr after `start`
    /// at time 0) first drops to `level`, interpolated.
    fn falling_crossing(start: f32, trace: &[f32], level: f32, sr: f32) -> f32 {
        let i = trace.iter().position(|&v| v <= level).expect("never fell");
        let prev = if i == 0 { start } else { trace[i - 1] };
        let frac = (prev - level) / (prev - trace[i]).max(1e-12);
        (i as f32 + frac) / sr
    }

    /// The documented static curve — threshold, ratio and the converter's
    /// knee — measured with a steady tone at 21 levels for every ratio.
    #[test]
    fn the_static_curve_is_the_documented_ratio_threshold_and_knee() {
        let threshold = -20.0;
        for (index, &ratio) in RATIOS.iter().enumerate() {
            let mut curve = Vec::new();
            for step in 0..=20 {
                let level = -40.0 + 2.0 * step as f32;
                let mut c = comp(FS, threshold, index, 2, 3);
                let a = amp(level);
                let n = (0.6 * FS) as usize;
                let tail = 4800;
                let mut out = Vec::with_capacity(tail);
                for k in 0..n {
                    let x = sine_at(FS, 1000.0, a, k);
                    let (y, _) = c.process(x, x);
                    if k >= n - tail {
                        out.push(y);
                    }
                }
                let out_level = db(tone_amplitude(&out, FS, 1000.0));
                let measured = level - out_level;
                let expected = static_reduction_db(level - threshold, ratio);
                assert!(
                    (measured - expected).abs() < 0.1,
                    "{ratio}:1 at {level} dBFS: reduced {measured:.3} dB, the curve says {expected:.3}"
                );
                curve.push((level, out_level));
            }
            // Far below the knee: untouched. Above it: exactly R:1.
            assert!(
                (curve[0].0 - curve[0].1).abs() < 0.01,
                "{ratio}:1 acts 20 dB under threshold"
            );
            let (lo, hi) = (curve[17], curve[20]); // +14 and +20 dB over
            let slope = (hi.1 - lo.1) / (hi.0 - lo.0);
            assert!(
                (slope * ratio - 1.0).abs() < 0.02,
                "{ratio}:1 measures {:.2}:1 above the knee",
                1.0 / slope
            );
        }
        // The knee's own signature: at the threshold, K ln 2 of the full
        // above-threshold reduction
        let at = static_reduction_db(0.0, 4.0);
        assert!((at - 0.75 * KNEE_DB * std::f32::consts::LN_2).abs() < 1e-5);
    }

    /// Step response of the attack RC through the control-port RC: the
    /// time at which it reaches `fraction` of its final value.
    fn expected_attack_crossing(tau_a: f32, fraction: f32) -> f32 {
        let tau_p = 1.0 / (2.0 * PI * CV_PORT_HZ);
        let y = |t: f32| {
            1.0 - (tau_a * (-t / tau_a).exp() - tau_p * (-t / tau_p).exp()) / (tau_a - tau_p)
        };
        let (mut lo, mut hi) = (0.0f32, 20.0 * tau_a);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if y(mid) < fraction {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    }

    /// A square wave: its rectified level is constant, so a burst of it
    /// is a clean step of computed reduction and the meter traces the
    /// timing network itself.
    fn square(n: usize, half_period: usize, a: f32) -> f32 {
        if (n / half_period) % 2 == 0 {
            a
        } else {
            -a
        }
    }

    #[test]
    fn attack_time_constants_match_the_switch() {
        let sr = 96000.0;
        let e = 1.0 - (-1.0f32).exp();
        for (index, &tau) in ATTACKS_S.iter().enumerate() {
            let mut c = comp(sr, -20.0, 1, index, 3);
            let n = (tau * 12.0 * sr) as usize + 2000;
            let trace: Vec<f32> = (0..n)
                .map(|k| {
                    let x = square(k, 12, amp(0.0));
                    c.process(x, x);
                    c.gain_reduction_db()
                })
                .collect();
            let settled = trace[n - 1];
            let expected_final = static_reduction_db(20.0, 4.0);
            assert!(
                (settled - expected_final).abs() < 0.2,
                "attack {tau}: settled at {settled:.2} dB, not {expected_final:.2}"
            );
            let measured = crossing(&trace, e * settled, sr);
            let expected = expected_attack_crossing(tau, e);
            assert!(
                (measured / expected - 1.0).abs() < 0.03,
                "attack {:.1} ms: 63% at {:.4} ms, expected {:.4} ms",
                tau * 1e3,
                measured * 1e3,
                expected * 1e3
            );
        }
    }

    #[test]
    fn release_time_constants_match_the_switch() {
        let sr = 48000.0;
        for (index, &tau) in RELEASES_S.iter().enumerate() {
            let mut c = comp(sr, -20.0, 1, 2, index);
            for k in 0..(0.3 * sr) as usize {
                let x = square(k, 24, amp(0.0));
                c.process(x, x);
            }
            let start = c.gain_reduction_db();
            let trace: Vec<f32> = (0..(4.0 * tau * sr) as usize)
                .map(|_| {
                    c.process(0.0, 0.0);
                    c.gain_reduction_db()
                })
                .collect();
            let measured = falling_crossing(start, &trace, start / std::f32::consts::E, sr);
            assert!(
                (measured / tau - 1.0).abs() < 0.02,
                "release {tau} s: recovered to 1/e in {measured:.4} s"
            );
        }
    }

    /// Time for the reduction held at the end of a burst to fall to 1/e.
    fn recovery_after(burst_s: f32, release: usize) -> f32 {
        let sr = 48000.0;
        let mut c = comp(sr, -20.0, 1, 2, release);
        for k in 0..(burst_s * sr) as usize {
            let x = sine_at(sr, 1000.0, amp(0.0), k);
            c.process(x, x);
        }
        let start = c.gain_reduction_db();
        let trace: Vec<f32> = (0..(6.0 * sr) as usize)
            .map(|_| {
                c.process(0.0, 0.0);
                c.gain_reduction_db()
            })
            .collect();
        falling_crossing(start, &trace, start / std::f32::consts::E, sr)
    }

    /// AUTO: a short transient lets go fast, sustained compression lets go
    /// slowly — from the same network, with no switch thrown. The fixed
    /// positions are one RC and do not care what came before.
    #[test]
    fn auto_release_is_program_dependent() {
        let transient = recovery_after(0.03, RELEASE_AUTO);
        let sustained = recovery_after(3.0, RELEASE_AUTO);
        println!("AUTO release: {transient:.3} s after 30 ms, {sustained:.3} s after 3 s");
        assert!(
            transient < 0.2,
            "a 30 ms hit should recover on the fast constant: {transient:.3} s"
        );
        assert!(
            sustained > 0.9,
            "3 s of compression should recover on the slow constant: {sustained:.3} s"
        );
        assert!(
            sustained > 5.0 * transient,
            "AUTO is not program-dependent: {transient:.3} s vs {sustained:.3} s"
        );
        let fixed_short = recovery_after(0.03, 1);
        let fixed_long = recovery_after(3.0, 1);
        assert!(
            (fixed_short / fixed_long - 1.0).abs() < 0.03,
            "a fixed release must not depend on the program: {fixed_short:.3} vs {fixed_long:.3} s"
        );
    }

    /// Stereo link: a hit on one side reduces BOTH sides by the same dB,
    /// and it does not matter which side it lands on.
    #[test]
    fn a_hit_on_one_channel_reduces_both_equally() {
        let sr = 48000.0;
        let run = |hit_left: bool| {
            let mut c = comp(sr, -20.0, 1, 2, 3);
            let n = (0.5 * sr) as usize;
            let window = (0.35 * sr) as usize..n;
            let (mut loud, mut quiet, mut meter) = (Vec::new(), Vec::new(), Vec::new());
            let (mut loud_in, mut quiet_in) = (Vec::new(), Vec::new());
            for k in 0..n {
                let hit = sine_at(sr, 1000.0, amp(-4.0), k);
                let bed = sine_at(sr, 440.0, amp(-40.0), k);
                let (l, r) = if hit_left {
                    c.process(hit, bed)
                } else {
                    c.process(bed, hit)
                };
                meter.push(c.gain_reduction_db());
                if window.contains(&k) {
                    let (lo, qu) = if hit_left { (l, r) } else { (r, l) };
                    loud.push(lo);
                    quiet.push(qu);
                    loud_in.push(hit);
                    quiet_in.push(bed);
                }
            }
            let loud_gr =
                db(tone_amplitude(&loud_in, sr, 1000.0) / tone_amplitude(&loud, sr, 1000.0));
            let quiet_gr =
                db(tone_amplitude(&quiet_in, sr, 440.0) / tone_amplitude(&quiet, sr, 440.0));
            (loud_gr, quiet_gr, meter)
        };
        let (loud_l, quiet_l, meter_l) = run(true);
        let (loud_r, quiet_r, meter_r) = run(false);
        assert!(loud_l > 8.0, "the hit should compress hard: {loud_l:.2} dB");
        assert!(
            (loud_l - quiet_l).abs() < 0.05,
            "hit left: loud side -{loud_l:.3} dB, quiet side -{quiet_l:.3} dB"
        );
        assert!(
            (loud_r - quiet_r).abs() < 0.05,
            "hit right: loud side -{loud_r:.3} dB, quiet side -{quiet_r:.3} dB"
        );
        assert_eq!(meter_l, meter_r, "one detector: the side must not matter");
        let meter_now = meter_l[meter_l.len() - 1];
        assert!(
            (meter_now - loud_l).abs() < 0.1,
            "the meter reads {meter_now:.2} dB while the VCAs take {loud_l:.2}"
        );
    }

    /// Out means a wire: bit-identical, before the unit is ever used, with
    /// every control moved, and again after it has been in and come out.
    #[test]
    fn bypassed_is_bit_identical() {
        let mut rng = crate::rng::Rng::new(0xB05C0);
        let check = |c: &mut BusComp, rng: &mut crate::rng::Rng, n: usize| {
            for _ in 0..n {
                let (l, r) = (rng.bipolar() * 1.5, rng.bipolar() * 1.5);
                let (yl, yr) = c.process(l, r);
                assert_eq!((yl.to_bits(), yr.to_bits()), (l.to_bits(), r.to_bits()));
            }
        };
        let mut c = BusComp::new(FS);
        c.set_threshold(-48.0);
        c.set_ratio(2);
        c.set_attack(0);
        c.set_release(0);
        c.set_makeup(15.0);
        c.set_mix(0.3);
        c.set_sc_hpf(300.0);
        check(&mut c, &mut rng, 48000);
        c.set_engaged(true);
        for _ in 0..24000 {
            c.process(rng.bipolar(), rng.bipolar());
        }
        c.set_engaged(false);
        for _ in 0..(ENGAGE_S * FS) as usize + 1 {
            c.process(rng.bipolar(), rng.bipolar());
        }
        check(&mut c, &mut rng, 48000);
        assert!(
            c.gain_reduction_db() > 1.0,
            "the sidechain listens while out"
        );

        // Switched in with the MIX at 0 the unit is the dry bus to the bit
        let mut c = comp(FS, -40.0, 2, 0, 0);
        c.set_mix(0.0);
        for _ in 0..24000 {
            c.process(rng.bipolar(), rng.bipolar());
        }
        check(&mut c, &mut rng, 48000);
    }

    /// Throwing IN or OUT mid-program must not click, even at the fastest
    /// attack with the make-up all the way up; and in silence, not a bit.
    #[test]
    fn engaging_and_leaving_is_click_free() {
        let mut c = BusComp::new(FS);
        c.set_makeup(15.0);
        for _ in 0..4800 {
            c.process(0.0, 0.0);
        }
        c.set_engaged(true);
        for _ in 0..4800 {
            assert_eq!(c.process(0.0, 0.0), (0.0, 0.0), "engaging in silence");
        }

        for attack in [0usize, 2, 5] {
            let mut c = comp(FS, -30.0, 2, attack, 0);
            c.set_engaged(false);
            c.set_makeup(15.0);
            let n = (0.8 * FS) as usize;
            let (on, off) = ((0.25 * FS) as usize, (0.55 * FS) as usize);
            let mut y = Vec::with_capacity(n);
            for k in 0..n {
                if k == on {
                    c.set_engaged(true);
                }
                if k == off {
                    c.set_engaged(false);
                }
                y.push(c.process(sine_at(FS, 220.0, amp(-6.0), k), 0.0).0);
            }
            let window = (0.02 * FS) as usize;
            let steady = max_step(&y, 4800..on)
                .max(max_step(&y, (0.45 * FS) as usize..off))
                .max(max_step(&y, (0.7 * FS) as usize..n));
            let switched = max_step(&y, on..on + window).max(max_step(&y, off..off + window));
            assert!(
                switched < 1.2 * steady,
                "attack {attack}: switching step {switched} vs program step {steady}"
            );
        }
    }

    /// A song that opens with the unit IN and its knobs set is compressed
    /// from its first sample: no crossfade from a dry bus that was never
    /// playing, no threshold gliding in from the power-on position.
    #[test]
    fn a_unit_set_before_the_tape_rolls_is_in_from_the_first_sample() {
        let mut c = BusComp::new(FS);
        c.set_threshold(-40.0);
        c.set_ratio(2);
        c.set_attack(0);
        c.set_makeup(6.0);
        c.set_engaged(true);
        // A hit right on the downbeat, held. The opening cycle may overshoot
        // the settled ones by the attack's own first-transient overshoot
        // (the timing cap starts discharged), but a dry crossfade or a
        // threshold gliding in from -20 dB let it through near -12 dBFS
        let (mut opening, mut settled) = (0.0f32, 0.0f32);
        for k in 0..(0.3 * FS) as usize {
            let x = sine_at(FS, 100.0, amp(-10.0), k);
            let y = c.process(x, x).0.abs();
            if k < (0.02 * FS) as usize {
                opening = opening.max(y);
            } else if k > (0.2 * FS) as usize {
                settled = settled.max(y);
            }
        }
        let allowed = -10.0 - static_reduction_db(30.0, 10.0) + 6.0;
        assert!(
            db(settled) < allowed + 4.0,
            "not compressing at the song's settings: {:.1} dBFS",
            db(settled)
        );
        assert!(
            db(opening) < -22.0 && db(opening) < db(settled) + 4.0,
            "the opening hit came through at {:.1} dBFS, the settled program at {:.1}",
            db(opening),
            db(settled)
        );
    }

    /// Automating any control — including throwing a switch — mid-program
    /// never steps the output harder than the program itself moves.
    #[test]
    fn automation_never_clicks() {
        type Move = (&'static str, fn(&mut BusComp), fn(&mut BusComp));
        let moves: [Move; 9] = [
            (
                "threshold down",
                |c| c.set_threshold(-20.0),
                |c| c.set_threshold(-45.0),
            ),
            (
                "threshold up",
                |c| c.set_threshold(-45.0),
                |c| c.set_threshold(-10.0),
            ),
            ("ratio", |c| c.set_ratio(0), |c| c.set_ratio(2)),
            ("attack", |c| c.set_attack(5), |c| c.set_attack(0)),
            (
                "release to auto",
                |c| c.set_release(0),
                |c| c.set_release(RELEASE_AUTO),
            ),
            (
                "release from auto",
                |c| c.set_release(RELEASE_AUTO),
                |c| c.set_release(0),
            ),
            ("makeup", |c| c.set_makeup(0.0), |c| c.set_makeup(15.0)),
            ("mix", |c| c.set_mix(1.0), |c| c.set_mix(0.2)),
            (
                "sidechain hpf",
                |c| c.set_sc_hpf(20.0),
                |c| c.set_sc_hpf(400.0),
            ),
        ];
        for (name, before, after) in moves {
            for attack in [1usize, 3] {
                let mut c = comp(FS, -30.0, 1, attack, 1);
                before(&mut c);
                let n = (0.9 * FS) as usize;
                let at = (0.4 * FS) as usize;
                let mut y = Vec::with_capacity(n);
                for k in 0..n {
                    if k == at {
                        after(&mut c);
                    }
                    // A bass line under a steady tone: the sidechain filter
                    // has something to take away
                    let x = sine_at(FS, 220.0, amp(-12.0), k) + sine_at(FS, 55.0, amp(-10.0), k);
                    y.push(c.process(x, x).0);
                }
                let steady = max_step(&y, (0.2 * FS) as usize..at)
                    .max(max_step(&y, (0.75 * FS) as usize..n));
                let moved = max_step(&y, at..at + (0.1 * FS) as usize);
                assert!(
                    moved < 1.2 * steady,
                    "{name} (attack {attack}): step {moved} vs program step {steady}"
                );
            }
        }
    }

    /// The same compressor at every host rate: the timing network and the
    /// knee are specified in seconds and dB, not samples.
    #[test]
    fn every_rate_hears_the_same_compressor() {
        let probe = [0.002f32, 0.01, 0.05, 0.19, 0.25, 0.4, 0.6, 0.9];
        let run = |sr: f32| {
            let mut c = comp(sr, -20.0, 1, 3, RELEASE_AUTO);
            let n = sr as usize;
            let mut meter = Vec::with_capacity(n);
            let mut tail = Vec::new();
            for k in 0..n {
                let t = k as f32 / sr;
                let level = if t < 0.2 {
                    -5.0
                } else if t < 0.5 {
                    -60.0
                } else {
                    -12.0
                };
                let x = sine_at(sr, 1000.0, amp(level), k);
                let (y, _) = c.process(x, x);
                meter.push(c.gain_reduction_db());
                if t >= 0.8 {
                    tail.push(y);
                }
            }
            let at: Vec<f32> = probe.iter().map(|&t| meter[(t * sr) as usize]).collect();
            (at, db(tone_amplitude(&tail, sr, 1000.0)))
        };
        let (reference, ref_level) = run(96000.0);
        for sr in [44100.0, 48000.0] {
            let (at, level) = run(sr);
            for (i, (&a, &b)) in at.iter().zip(&reference).enumerate() {
                assert!(
                    (a - b).abs() < 0.1,
                    "at {} s: {a:.3} dB at {sr} Hz vs {b:.3} dB at 96 kHz",
                    probe[i]
                );
            }
            assert!(
                (level - ref_level).abs() < 0.05,
                "{sr} Hz settles at {level:.3} dBFS, 96 kHz at {ref_level:.3}"
            );
        }
    }

    /// Hard settings, absurd input, non-finite input and long silence:
    /// finite output, bounded gain, and no denormals left in any state.
    #[test]
    fn extremes_silence_and_denormals_are_stable() {
        for sr in [crate::MIN_SAMPLE_RATE as f32, 11025.0, 44100.0, 192000.0] {
            for release in [0usize, RELEASE_AUTO] {
                let mut c = comp(sr, -48.0, 2, 0, release);
                c.set_makeup(15.0);
                c.set_mix(0.5);
                c.set_sc_hpf(400.0);
                let mut rng = crate::rng::Rng::new(7);
                let mut peak_out = 0.0f32;
                for k in 0..(sr as usize) {
                    let x = if k % 4000 == 0 {
                        1000.0
                    } else {
                        rng.bipolar() * 4.0
                    };
                    let (l, r) = c.process(x, -x);
                    assert!(l.is_finite() && r.is_finite(), "sr {sr}: non-finite at {k}");
                    peak_out = peak_out.max(l.abs()).max(r.abs());
                }
                assert!(
                    peak_out < 6.0 * 1000.0,
                    "sr {sr}: gain ran away, {peak_out}"
                );
                for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                    let (l, r) = c.process(x, 0.5);
                    assert!(l.is_finite() && r.is_finite(), "{x} got through");
                }
                let (l, _) = c.process(0.5, 0.5);
                assert!(l.is_finite(), "a non-finite input poisoned the unit");
                // Silence and subnormal input, long enough to decay past 1e-20
                for k in 0..(120.0 * sr.min(48000.0)) as usize {
                    let x = if k % 2 == 0 { 1e-40 } else { 0.0 };
                    c.process(x, x);
                }
                let states = [
                    c.cv_c1,
                    c.cv_c2,
                    c.cv_port,
                    c.sc_hpf[0].ic1,
                    c.sc_hpf[0].ic2,
                    c.sc_hpf[1].ic1,
                    c.sc_hpf[1].ic2,
                ];
                for s in states {
                    assert!(s == 0.0 || s.is_normal(), "sr {sr}: denormal state {s:e}");
                }
            }
        }
    }

    /// The cell colours only while it is reducing: a clean wire at unity
    /// gain, a faint second harmonic under reduction that follows each
    /// chip's own mismatch, and a DC step that follows the gain.
    #[test]
    fn the_vca_colours_only_under_reduction() {
        // Unity: far under the threshold the cell is transparent
        let mut c = comp(FS, 0.0, 1, 4, 3);
        for k in 0..48000 {
            let x = sine_at(FS, 1000.0, amp(-40.0), k);
            let (y, _) = c.process(x, x);
            if k > 4800 {
                assert!((y - x).abs() < 1e-7, "not a wire at unity: {y} vs {x}");
            }
        }

        // 10 dB of reduction
        let threshold = -30.0;
        let mut level = threshold;
        while static_reduction_db(level - threshold, 4.0) < 10.0 {
            level += 0.01;
        }
        let mut c = comp(FS, threshold, 1, 2, 3);
        let n = FS as usize;
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for k in 0..n {
            let x = sine_at(FS, 1000.0, amp(level), k);
            let (l, r) = c.process(x, x);
            if k >= n / 2 {
                left.push(l);
                right.push(r);
            }
        }
        assert!((c.gain_reduction_db() - 10.0).abs() < 0.2);
        let h2 = |y: &[f32]| tone_amplitude(y, FS, 2000.0) / tone_amplitude(y, FS, 1000.0);
        let (h2_l, h2_r) = (h2(&left), h2(&right));
        println!(
            "cell colour at 10 dB: h2 {:.4}% left, {:.4}% right",
            h2_l * 100.0,
            h2_r * 100.0
        );
        assert!(
            (0.0007..0.0015).contains(&h2_l),
            "second harmonic at 10 dB of reduction: {:.4}%",
            h2_l * 100.0
        );
        let chips = VCA_MISMATCH[1] / VCA_MISMATCH[0];
        assert!(
            (h2_r / h2_l / chips - 1.0).abs() < 0.05,
            "each cell has its own mismatch: {h2_l:.2e} vs {h2_r:.2e}"
        );
        let h3 = tone_amplitude(&left, FS, 3000.0) / tone_amplitude(&left, FS, 1000.0);
        assert!(
            h3 < h2_l,
            "the cell's own colour is even-order: h3 {h3:.2e}"
        );

        // Control-voltage feedthrough: the quiescent DC under held reduction
        for _ in 0..48 {
            c.process(0.0, 0.0);
        }
        let (dc, _) = c.process(0.0, 0.0);
        let gr = c.gain_reduction_db();
        println!(
            "feedthrough at {gr:.1} dB of reduction: {:.1} dBFS",
            db(dc.abs())
        );
        assert!(gr > 9.0, "reduction should still be held: {gr}");
        assert!(
            (-110.0..-85.0).contains(&db(dc.abs())),
            "feedthrough at {gr:.1} dB of reduction: {:.1} dBFS",
            db(dc.abs())
        );
    }

    /// Worst spurious product, relative to the tone, among the frequencies
    /// where only aliasing can put energy (sr - tone - 100 n).
    fn worst_alias(y: &[f32], sr: f32, tone: f32) -> f32 {
        let reference = tone_amplitude(y, sr, tone);
        let mut worst = 0.0f32;
        let mut f = sr - tone;
        while f > 100.0 {
            if f < 0.5 * sr {
                worst = worst.max(tone_amplitude(y, sr, f));
            }
            f -= 100.0;
        }
        db(worst / reference)
    }

    /// A pumping bass line riding the gain of a 17 kHz tone, at 10:1 and
    /// 44.1 kHz. The attack and control-port RCs band-limit the gain
    /// modulation itself; ADAA keeps the cell's even term from folding;
    /// what is left is the sampled detector reading HF key content, and it
    /// shrinks with the attack time as an RC should.
    #[test]
    fn fast_gain_modulation_does_not_alias() {
        let sr = 44100.0;
        let tone = 17030.0;
        let render = |attack: usize, key_hears_tone: bool, cell_colour: bool| {
            let mut c = comp(sr, -30.0, 2, attack, 0);
            let warm = (0.5 * sr) as usize;
            let n = (2.0 * sr) as usize;
            let mut y = Vec::with_capacity(n);
            for k in 0..warm + n {
                let bass = sine_at(sr, 50.0, 0.5, k);
                let x = bass + sine_at(sr, tone, 0.1, k);
                let key = if key_hears_tone { x } else { bass };
                let reduction = c.detect(key, key);
                let gain = db_to_gain(-reduction);
                let out = if cell_colour {
                    c.cells[0].process(x, gain, reduction)
                } else {
                    x * gain
                };
                if k >= warm {
                    y.push(out);
                }
            }
            worst_alias(&y, sr, tone)
        };
        let modulation = render(0, false, false);
        let cell = render(0, false, true);
        let fastest = render(0, true, true);
        let one_ms = render(2, true, true);
        let three_ms = render(3, true, true);
        println!(
            "aliasing: modulation {modulation:.1}, cell {cell:.1}, 0.1 ms {fastest:.1}, 1 ms {one_ms:.1}, 3 ms {three_ms:.1} dBc"
        );
        assert!(
            modulation < -100.0,
            "gain modulation aliases at {modulation:.1} dBc"
        );
        assert!(
            cell < -84.0,
            "the cell's even term aliases at {cell:.1} dBc"
        );
        assert!(fastest < -67.0, "0.1 ms attack: {fastest:.1} dBc");
        assert!(one_ms < -77.0, "1 ms attack: {one_ms:.1} dBc");
        assert!(three_ms < -90.0, "3 ms attack: {three_ms:.1} dBc");
    }

    /// House rule (design.md): re-asserting a setting every block must
    /// leave the circuit exactly where it was.
    #[test]
    fn reasserting_settings_is_a_no_op() {
        let mut c = comp(FS, -24.0, 1, 3, RELEASE_AUTO);
        c.set_makeup(4.0);
        c.set_mix(0.8);
        c.set_sc_hpf(120.0);
        for k in 0..24000 {
            let x = sine_at(FS, 300.0, 0.5, k);
            c.process(x, x);
        }
        let snapshot = |c: &BusComp| {
            (
                c.cv_c1,
                c.cv_c2,
                c.cv_port,
                c.attack_k,
                c.release_k,
                c.slope_target,
                c.makeup_target,
                c.sc_hpf[0].ic1,
                c.sc_hpf_heard,
            )
        };
        let before = snapshot(&c);
        c.set_engaged(true);
        c.set_threshold(-24.0);
        c.set_ratio(1);
        c.set_attack(3);
        c.set_release(RELEASE_AUTO);
        c.set_makeup(4.0);
        c.set_mix(0.8);
        c.set_sc_hpf(120.0);
        assert!(
            before == snapshot(&c),
            "re-asserting the panel moved the circuit"
        );
    }

    /// Cheap enough to leave on: the whole unit, switched in with AUTO
    /// release, against the 48 kHz real-time budget.
    #[test]
    fn cheap_enough_for_live_use() {
        let mut c = comp(FS, -30.0, 1, 1, RELEASE_AUTO);
        let mut rng = crate::rng::Rng::new(11);
        let input: Vec<f32> = (0..4096).map(|_| rng.bipolar() * 0.5).collect();
        let n = if cfg!(debug_assertions) {
            48_000
        } else {
            960_000
        };
        let start = std::time::Instant::now();
        let mut acc = 0.0f32;
        for k in 0..n {
            let x = input[k % input.len()];
            let (l, r) = c.process(x, -x);
            acc += l + r;
        }
        let elapsed = start.elapsed().as_secs_f64();
        assert!(acc.is_finite());
        let ns_per_frame = elapsed * 1e9 / n as f64;
        let budget_share = ns_per_frame / (1e9 / FS as f64);
        println!(
            "bus compressor: {ns_per_frame:.0} ns per stereo frame, {:.2}% of one core at 48 kHz",
            budget_share * 100.0
        );
        if !cfg!(debug_assertions) {
            assert!(
                budget_share < 0.02,
                "{ns_per_frame:.0} ns per frame is {:.1}% of the real-time budget",
                budget_share * 100.0
            );
        }
    }
}
