use crate::envelope::Envelope;
use crate::filter::LadderFilter;
use crate::hpf::HighPassLadder;
use crate::oscillator::{CircuitModel, Oscillator, Waveform, PROGRAM_V};
use crate::shaper::{Shaper, ShaperMode};
use crate::sources::{NoiseMode, NoiseOsc, SuperSaw};
use crate::substrate::SubstrateState;

/// How much velocity opens the filter, in octaves at full velocity swing —
/// the default of a patch's `vel_filter`.
pub(crate) const VEL_TRACK: f32 = 0.8;

/// How deep velocity reaches into the VCA — the default of a patch's
/// `vel_amp`. At 0.7 the softest strike still sounds at 30% of the
/// hardest, the gentle curve every patch was voiced against.
pub(crate) const VEL_AMP: f32 = 0.7;

/// A note's pitch CV in octaves from A440, the volts the keyboard's
/// hold capacitor carries (and glide slews from).
#[inline]
pub(crate) fn note_cv(note: u8) -> f32 {
    (note as f32 - 69.0) / 12.0
}

/// Finite sawtooth-core reset time (US 3,943,456 variable-rate integrator):
/// the discharge takes real time, so f_actual = f / (1 + f * T_RESET) and
/// high notes land slightly flat, like an analog VCO between calibrations.
const RESET_TIME: f32 = 1.5e-6;

/// The VCA never fully closes — a silent voice still leaks its free-running
/// oscillators. The 902 alignment spec sets the CEILING ("At 0, signal
/// output should be -60db maximum"); the Polymoog factory noise table shows
/// where passing units actually sit (-77 to -84 dBm per preset, with
/// bleed-through explicitly listened for and tolerated below that). A
/// serviced floor of -70 dB. Voices render continuously; digital silence
/// between notes is not a thing hardware does.
const VCA_FLOOR: f32 = 3.2e-4;

/// Mixer gain into the VCF's summing junction (dimensionless; the signals
/// are volts). Deliberately hot: three oscillators up SHOULD push the
/// ladder into its tanh curvature — the ARP 2600 manual lists "VCF
/// OVERDRIVEN" under NO PROBLEM.
const MIXER_GAIN: f32 = 0.45;

/// Exponential-converter calibration reference (~C4). V/oct scaling error
/// accumulates in cents per octave away from this point.
const CAL_REF_HZ: f32 = 261.63;

/// The mean-tracker that keeps exponential FM pitch-neutral, as a TIME
/// constant rather than a per-sample coefficient (see render's FM block).
const FM_MEAN_TAU_S: f32 = 0.09;

/// One-pole smoothing coefficient for a given time constant.
#[inline]
pub(crate) fn smoothing_coef(tau_seconds: f32, sample_rate: f32) -> f32 {
    if !(sample_rate > 0.0) || tau_seconds <= 0.0 {
        return 1.0;
    }
    (1.0 - (-1.0 / (tau_seconds * sample_rate)).exp()).clamp(0.0, 1.0)
}

pub struct Voice {
    pub oscs: [Oscillator; 3],
    circuit: CircuitModel,
    pub envelope: Envelope,
    pub filter_env: Envelope,
    pub filter: LadderFilter,
    pub hpf: HighPassLadder,
    pub note: Option<u8>,
    velocity: f32,
    /// The patch's velocity sensitivity: `vel_amp` is how far a soft
    /// strike drops the VCA (0 = velocity ignored, 1 = full swing to
    /// silence), `vel_filter` how many octaves the velocity swing moves
    /// the cutoff.
    vel_amp: f32,
    vel_filter: f32,
    age: u64,
    held: bool,
    /// Position and energy compensation assigned by the unison stack.
    /// Card index is deliberately irrelevant: allocation order must never
    /// collapse a stack onto one side of the stereo field.
    pan_l: f32,
    pan_r: f32,
    unison_gain: f32,
    filter_env_amount: f32, // octaves, -5..+5
    /// Per-oscillator V/octave scaling tolerance in cents per octave —
    /// the matched-transistor expo converters are never perfectly trimmed,
    /// so intervals stretch differently on every voice and chords bloom.
    voct_error: [f32; 3],
    /// Shared drift: the three oscillators sit on one controller and supply,
    /// so most of their movement is common (a serviced 901 bank beats no
    /// faster than once per two seconds), with small residue per core.
    common_drift: f32,
    drift_rng: u32,
    /// Portamento as the SH-101/303 lineage does it: a LINEAR slew of the
    /// CV at constant octaves-per-second, applied BEFORE the expo
    /// converter. Constant rate means a semitone snaps and a leap takes
    /// proportionally longer — and the pitch ARRIVES and stops, no
    /// asymptotic flat crawl into every note (the old RC glide's
    /// perpetual last-few-cents sag was the "silly" in the sound).
    /// `glide_offset` is the remaining octave distance; `glide_rate` the
    /// per-sample slew step (1.0 = effectively instant).
    glide_offset: f32,
    glide_rate: f32,
    /// Channel-local pitch automation. Separate song tracks can therefore
    /// glide chord voices to different exact notes without a global bend.
    /// The ratio is slewed here so automation never steps.
    pitch_shift_ratio: f32,
    pitch_shift_target: f32,
    pitch_shift_k: f32,
    /// External pitch CV (octaves from A440) — the voice box's
    /// performance line. While present it replaces note pitch and
    /// glide: the curve carries its own portamento, scoops and vibrato.
    cv_override: Option<f32>,
    /// Juno-style sub-oscillator level: the first oscillator's divide-by-two
    /// square, mixed in before the filter.
    sub_level: f32,
    /// A real voice is three INDEPENDENT oscillator sections (Minimoog /
    /// 2600 architecture), not three clones: per-oscillator pitch offsets
    /// in semitones (osc 2 and 3; osc 1 is the reference) and mix levels.
    /// Levels default center-dominant — equal-amplitude unison cancels too
    /// deeply when phases oppose, which reads as "hollow".
    osc_pitch_semi: [f32; 2],
    osc_level: [f32; 2],
    detune_cents: f32,
    /// Unison spread offset for this card, in cents — set by the manager
    /// when a note claims several cards at once. Each card in the stack
    /// is a genuinely different circuit (its own tolerances and drift),
    /// so unison here is an ensemble of instruments, not copies.
    unison_cents: f32,
    /// Filter keyboard tracking, octaves of cutoff per octave of pitch.
    /// The 2600 trims this to full 1 V/oct so a self-oscillating filter
    /// plays in tune; the Minimoog offers fractional settings.
    key_track: f32,
    /// The 2600's prewired cross-oscillator FM: osc 2 modulates osc 1's
    /// frequency through the exponential converter (audio-rate, in CV
    /// space — the source of the growl).
    fm_amount: f32,
    /// Slow mean of the exponential FM multiplier, divided back out so
    /// FM is pitch-neutral (see render's FM block).
    fm_mean: f32,
    /// Its tracking coefficient for `FM_MEAN_TAU_S` at this rate.
    fm_mean_k: f32,
    prev_osc2: f32,
    /// Hard sync, 2600-style: VCO1 masters VCO2's ramp reset.
    sync_on: bool,
    /// Ring modulator: (o1 * o2) / PROGRAM_V, the ARP manual's literal
    /// transfer ("the product of the two input voltages divided by 5"),
    /// crossfaded into the filter input. Carrier leakage is this unit's
    /// residual after the < 10 mV null trim.
    ring_amount: f32,
    ring_leak: (f32, f32),
    /// Post-filter DC block (ARP R162 "eliminates the DC from the output";
    /// Moog dwg #1149 shows 2.5 uF output coupling). Needed because the
    /// unipolar/asymmetric pulses legitimately push DC through the ladder.
    dc_x1: f32,
    dc_y1: f32,
    /// Post-ladder coupling pole, derived from the rate so the corner sits
    /// at the same frequency instead of doubling at 96 kHz.
    dc_pole: f32,
    /// This card's sensitivity to the shared chassis state (rail and heat):
    /// every board reacts to the same environment, each by its own amount.
    substrate_sens: f32,
    /// Pre-filter node history, exposed so the neighbor card can pick up
    /// its capacitively-coupled (differentiated) bleed.
    prev_prefilter: f32,
    prefilter_delta: f32,
    /// Which song channel configured this voice (0 = the live panel).
    channel: u16,
    /// This voice's base pulse width; the manager passes only the LFO's
    /// PWM offset, so different channels can hold different widths.
    pulse_width: f32,
    /// The pitch envelope (the kick-bass dive): every new gate starts the
    /// note `pitch_env_oct` octaves away from its own pitch and lands on
    /// it after `pitch_env_time` seconds, along (1 - x)^power in octave
    /// space; then the tail holds the note's pitch exactly. 0 = off.
    pitch_env_oct: f32,
    pitch_env_power: f32,
    pitch_env_inc: f32,
    pitch_env_secs: f32,
    /// Progress through the dive, 0..1 (1 = landed).
    pitch_env_x: f32,
    /// Key sync: every new gate restarts the oscillator cores (and the
    /// supersaw bank) at a fixed phase, so each hit starts the same way.
    key_sync: bool,
    /// The drive stage between the mixer and the ladder (shaper.rs).
    shaper: Shaper,
    /// The seven-saw bank on oscillator 1 (sources.rs) and how far it
    /// replaces oscillator 1's own wave (0 = off).
    supersaw: SuperSaw,
    supersaw_amt: f32,
    /// Oscillator 1's level into the mixer (the supersaw included): 1 is
    /// the reference every patch was voiced at; 0 leaves a card to its
    /// other sources (a pure noise riser).
    osc1_level: f32,
    supersaw_detune: f32,
    supersaw_mix: f32,
    /// This card's own filtered noise (sources.rs).
    noise_osc: NoiseOsc,
    /// A note bend (`C4:2>G3` in a song): `bend_oct` octaves reached over
    /// the note's held length, along 1 - (1 - x)^power; it then holds the
    /// target. 0 = no bend. Every new note clears it.
    bend_oct: f32,
    bend_x: f32,
    bend_inc: f32,
    bend_power: f32,
    sample_rate: f32,
}

/// A curve knob (-1..1) as the exponent of its (1 - x)^p shape: 0 is a
/// straight line, +1 moves sixteen times faster at the start, -1 holds
/// back until the end.
#[inline]
fn curve_power(curve: f32) -> f32 {
    (4.0 * curve.clamp(-1.0, 1.0)).exp2()
}

impl Voice {
    pub fn new(sample_rate: f32, index: usize) -> Self {
        let seed = (index as u32).wrapping_add(1);

        let filter_env = Envelope::new(sample_rate);
        filter_env.set_attack(0.005);
        filter_env.set_decay(0.3);
        filter_env.set_sustain(0.0);
        filter_env.set_release(0.3);

        // Component-tolerance randoms, fixed for the life of the "board"
        let mut rng = seed.wrapping_mul(0xB529_7A4D) | 1;
        let mut rand01 = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            (rng >> 8) as f32 / (1u32 << 24) as f32
        };
        let voct_error = [
            (rand01() - 0.5) * 3.0, // +/-1.5 cents per octave
            (rand01() - 0.5) * 3.0,
            (rand01() - 0.5) * 3.0,
        ];
        let substrate_sens = 0.8 + rand01() * 0.4;
        // Ring-mod null trim residue: < 10 mV at 5 V program level
        let ring_leak = (rand01() * 0.002, rand01() * 0.002);

        let mut voice = Self {
            oscs: [
                Oscillator::new(sample_rate, 440.0, seed),
                Oscillator::new(sample_rate, 440.0, seed.wrapping_add(101)),
                Oscillator::new(sample_rate, 440.0, seed.wrapping_add(211)),
            ],
            circuit: CircuitModel::Moog,
            channel: 0,
            pulse_width: 0.5,
            sample_rate,
            envelope: Envelope::new(sample_rate),
            filter_env,
            filter: LadderFilter::new(sample_rate, seed),
            hpf: HighPassLadder::new(sample_rate),
            note: None,
            velocity: 0.0,
            vel_amp: VEL_AMP,
            vel_filter: VEL_TRACK,
            age: 0,
            held: false,
            pan_l: std::f32::consts::FRAC_1_SQRT_2,
            pan_r: std::f32::consts::FRAC_1_SQRT_2,
            unison_gain: 1.0,
            filter_env_amount: 0.0,
            voct_error,
            common_drift: 0.0,
            drift_rng: seed.wrapping_mul(0x27D4_EB2F) | 1,
            glide_offset: 0.0,
            glide_rate: 1.0,
            pitch_shift_ratio: 1.0,
            pitch_shift_target: 1.0,
            pitch_shift_k: smoothing_coef(0.025, sample_rate),
            cv_override: None,
            sub_level: 0.0,
            osc_pitch_semi: [0.0, 0.0],
            osc_level: [0.72, 0.72],
            detune_cents: 7.0,
            unison_cents: 0.0,
            key_track: 0.4,
            fm_amount: 0.0,
            fm_mean: 1.0,
            fm_mean_k: smoothing_coef(FM_MEAN_TAU_S, sample_rate),
            prev_osc2: 0.0,
            sync_on: false,
            ring_amount: 0.0,
            ring_leak,
            dc_x1: 0.0,
            dc_y1: 0.0,
            dc_pole: crate::smoothing::dc_blocker_pole(crate::smoothing::DC_BLOCK_HZ, sample_rate),
            substrate_sens,
            prev_prefilter: 0.0,
            prefilter_delta: 0.0,
            pitch_env_oct: 0.0,
            pitch_env_power: curve_power(0.5),
            pitch_env_inc: 1.0,
            pitch_env_secs: 0.1,
            pitch_env_x: 1.0,
            key_sync: false,
            shaper: Shaper::new(sample_rate),
            supersaw: SuperSaw::new(seed),
            supersaw_amt: 0.0,
            osc1_level: 1.0,
            supersaw_detune: 0.4,
            supersaw_mix: 0.6,
            noise_osc: NoiseOsc::new(sample_rate, seed),
            bend_oct: 0.0,
            bend_x: 1.0,
            bend_inc: 1.0,
            bend_power: 1.0,
        };
        voice.set_detune(7.0);
        voice
    }

    pub fn set_waveform(&mut self, waveform: Waveform) {
        for osc in &mut self.oscs {
            osc.set_waveform(waveform);
        }
    }

    /// Unison spread in cents: oscillator 0 stays centered, 1 and 2 detune
    /// symmetrically up and down — on top of their interval offsets.
    pub fn set_detune(&mut self, cents: f32) {
        self.detune_cents = cents.clamp(0.0, 50.0);
        self.update_freq_mults();
    }

    /// Interval offset for oscillator 2 or 3 in semitones (-24..+24) —
    /// saw + saw-detuned + wave-an-octave-down is the classic voice.
    pub fn set_osc_pitch(&mut self, which: usize, semitones: f32) {
        if which >= 1 && which <= 2 {
            self.osc_pitch_semi[which - 1] = semitones.clamp(-24.0, 24.0);
            self.update_freq_mults();
        }
    }

    pub fn set_osc_level(&mut self, which: usize, level: f32) {
        if which >= 1 && which <= 2 {
            self.osc_level[which - 1] = level.clamp(0.0, 1.0);
        }
    }

    pub fn set_osc_waveform(&mut self, which: usize, waveform: Waveform) {
        if which >= 1 && which <= 2 {
            self.oscs[which].set_waveform(waveform);
        }
    }

    fn update_freq_mults(&mut self) {
        let fine = (self.detune_cents / 1200.0 * std::f32::consts::LN_2).exp();
        self.oscs[0].set_freq_mult(1.0);
        self.oscs[1].set_freq_mult(fine * (self.osc_pitch_semi[0] / 12.0).exp2());
        self.oscs[2].set_freq_mult((self.osc_pitch_semi[1] / 12.0).exp2() / fine);
    }

    pub fn set_filter_env_amount(&mut self, octaves: f32) {
        self.filter_env_amount = octaves.clamp(-5.0, 5.0);
    }

    pub fn set_vel_amp(&mut self, depth: f32) {
        self.vel_amp = depth.clamp(0.0, 1.0);
    }

    pub fn set_vel_filter(&mut self, octaves: f32) {
        self.vel_filter = octaves.clamp(0.0, 4.0);
    }

    pub fn set_glide_rate(&mut self, rate: f32) {
        self.glide_rate = rate.clamp(1e-7, 1.0);
    }

    /// Move this card by a channel-local number of semitones. The actual
    /// pitch follows through a short analog-CV-style slew in render.
    pub fn set_pitch_shift_semitones(&mut self, semitones: f32) {
        self.pitch_shift_target = (semitones.clamp(-24.0, 24.0) / 12.0).exp2();
    }

    #[cfg(test)]
    pub(crate) fn pitch_shift_target_for_test(&self) -> f32 {
        self.pitch_shift_target
    }

    #[cfg(test)]
    pub(crate) fn unison_cents_for_test(&self) -> f32 {
        self.unison_cents
    }

    pub fn set_cv_override(&mut self, cv: Option<f32>) {
        self.cv_override = cv;
    }

    /// Diagnostic: remaining glide distance in octaves (0 = arrived).
    pub fn glide_remaining(&self) -> f32 {
        self.glide_offset
    }

    pub fn set_sub_level(&mut self, level: f32) {
        self.sub_level = level.clamp(0.0, 1.0);
    }

    pub fn set_circuit(&mut self, model: CircuitModel) {
        self.circuit = model;
        for osc in &mut self.oscs {
            osc.set_model(model);
        }
        // The circuit profile is the whole signal chain: converter
        // circuits, the filter architecture (ladder vs 4072 gm-C), and
        // the envelope circuit (911 vs 4020 attack targets)
        self.filter.set_model(model);
        self.envelope.set_circuit(model);
        self.filter_env.set_circuit(model);
    }

    pub fn channel(&self) -> u16 {
        self.channel
    }

    pub fn set_channel(&mut self, channel: u16) {
        self.channel = channel;
    }

    pub fn set_pulse_width(&mut self, width: f32) {
        self.pulse_width = width.clamp(0.05, 0.95);
    }

    /// Source-mixer levels [saw, pulse, tri, sine] for oscillator 1's
    /// core — the phase-locked parallel converter boards.
    pub fn set_osc1_mix(&mut self, mix: [f32; 4]) {
        self.oscs[0].set_mix(mix);
    }

    /// Configure this voice from a full parameter snapshot — the song
    /// engine's per-track patches. Bus effects and noise stay shared;
    /// per-track LFO routing is resolved by the voice manager.
    pub fn apply_params(&mut self, p: &crate::voice_manager::ParamValues) {
        self.set_waveform(p.waveform);
        self.set_osc_waveform(1, p.osc2_wave);
        self.set_osc_waveform(2, p.osc3_wave);
        self.set_osc_pitch(1, p.osc2_pitch);
        self.set_osc_pitch(2, p.osc3_pitch);
        self.set_osc_level(1, p.osc2_level);
        self.set_osc_level(2, p.osc3_level);
        self.set_detune(p.detune);
        self.set_sub_level(p.sub);
        self.set_circuit(p.circuit);
        self.set_key_track(p.key_track);
        self.set_fm_amount(p.osc_fm);
        self.set_sync(p.sync);
        self.set_ring(p.ring);
        self.set_filter_env_amount(p.filter_env_amount);
        self.set_vel_amp(p.vel_amp);
        self.set_vel_filter(p.vel_filter);
        self.set_pulse_width(p.pulse_width);
        self.set_osc1_mix([p.mix_saw, p.mix_pulse, p.mix_tri, p.mix_sine]);
        self.envelope.set_attack(p.attack);
        self.envelope.set_decay(p.decay);
        self.envelope.set_sustain(p.sustain);
        self.envelope.set_release(p.release);
        self.filter_env.set_attack(p.filter_attack);
        self.filter_env.set_decay(p.filter_decay);
        self.filter_env.set_sustain(p.filter_sustain);
        self.filter_env.set_release(p.filter_release);
        self.filter.set_cutoff(p.cutoff);
        self.filter.set_resonance(p.resonance);
        self.filter.set_drive(p.drive);
        self.filter.set_saturation(p.saturation);
        self.hpf.set_cutoff(p.hpf_cutoff);
        let rate = if p.glide < 1e-3 {
            1.0
        } else {
            // `glide` is seconds per OCTAVE of travel (linear slew)
            1.0 / (p.glide * self.sample_rate)
        };
        self.set_glide_rate(rate);
        self.apply_sources(p);
    }

    /// The kick-bass, wall and riser controls: pitch envelope, key sync,
    /// drive stage, supersaw bank, noise oscillator, note-bend curve.
    pub fn apply_sources(&mut self, p: &crate::voice_manager::ParamValues) {
        self.set_pitch_env(p.pitch_env, p.pitch_env_time, p.pitch_env_curve);
        self.key_sync = p.key_sync >= 0.5;
        self.shaper.set_drive_db(p.shaper);
        self.shaper.set_mode(ShaperMode::from_value(p.shaper_mode));
        self.supersaw_amt = p.supersaw.clamp(0.0, 1.0);
        self.osc1_level = p.osc1_level.clamp(0.0, 1.0);
        self.supersaw_detune = p.supersaw_detune.clamp(0.0, 1.0);
        self.supersaw_mix = p.supersaw_mix.clamp(0.0, 1.0);
        self.noise_osc.set_level(p.noise_osc);
        self.noise_osc.set_cutoff(p.noise_cutoff);
        self.noise_osc.set_res(p.noise_res);
        self.noise_osc.set_mode(NoiseMode::from_value(p.noise_mode));
        self.bend_power = curve_power(p.bend_curve);
    }

    /// The dive: start `semitones` from the note, land after `seconds`,
    /// shaped by `curve` (-1..1). Re-set mid-note it keeps its progress.
    pub fn set_pitch_env(&mut self, semitones: f32, seconds: f32, curve: f32) {
        self.pitch_env_oct = if semitones.is_finite() {
            semitones.clamp(-48.0, 96.0) / 12.0
        } else {
            0.0
        };
        if seconds.is_finite() {
            self.pitch_env_secs = seconds.clamp(0.001, 4.0);
        }
        self.pitch_env_inc = 1.0 / (self.pitch_env_secs * self.sample_rate);
        self.pitch_env_power = curve_power(if curve.is_finite() { curve } else { 0.0 });
    }

    /// Bend the sounding note by `semitones` over `seconds` (the note's
    /// held length), along this card's bend curve.
    pub fn start_bend(&mut self, semitones: f32, seconds: f32) {
        if !semitones.is_finite() || !seconds.is_finite() {
            return;
        }
        self.bend_oct = semitones.clamp(-48.0, 48.0) / 12.0;
        self.bend_x = 0.0;
        self.bend_inc = 1.0 / (seconds.max(1e-3) * self.sample_rate);
    }

    /// Diagnostic: the pitch envelope's and the bend's current offsets,
    /// in octaves.
    #[cfg(test)]
    pub(crate) fn pitch_offsets_for_test(&self) -> (f32, f32) {
        let env = if self.pitch_env_x < 1.0 {
            self.pitch_env_oct * (1.0 - self.pitch_env_x).powf(self.pitch_env_power)
        } else {
            0.0
        };
        let bend = self.bend_oct * (1.0 - (1.0 - self.bend_x.min(1.0)).powf(self.bend_power));
        (env, bend)
    }

    pub fn set_unison_cents(&mut self, cents: f32) {
        self.unison_cents = cents.clamp(-50.0, 50.0);
    }

    /// Place this card in the stereo field: the note's own position
    /// (`note_position`, -1..+1, scaled by `spread`) plus its place inside
    /// one unison stack (`stack_position`, -1..+1 across the stack). A stack
    /// always fans a little (±0.10) and widens with the spread. Energy
    /// normalization keeps changing the unison count from becoming a
    /// hidden volume control.
    pub fn set_stereo_position(
        &mut self,
        note_position: f32,
        stack_position: f32,
        count: usize,
        spread: f32,
    ) {
        let spread = spread.clamp(0.0, 1.0);
        let stack = if count > 1 {
            stack_position.clamp(-1.0, 1.0) * (0.10 + 0.40 * spread)
        } else {
            0.0
        };
        let pan = (note_position.clamp(-1.0, 1.0) * spread * 0.85 + stack).clamp(-1.0, 1.0);
        let theta = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
        self.pan_l = theta.cos();
        self.pan_r = theta.sin();
        self.unison_gain = 1.0 / (count.max(1) as f32).sqrt();
    }

    pub fn set_key_track(&mut self, amount: f32) {
        self.key_track = amount.clamp(0.0, 1.0);
    }

    pub fn set_fm_amount(&mut self, amount: f32) {
        self.fm_amount = amount.clamp(0.0, 1.0);
    }

    pub fn set_sync(&mut self, on: bool) {
        self.sync_on = on;
    }

    pub fn set_ring(&mut self, amount: f32) {
        self.ring_amount = amount.clamp(0.0, 1.0);
    }

    /// `glide_from_cv` is the most recently played note's CV in octaves
    /// (relative to A440); when glide is active the new note starts from
    /// there and settles exponentially, like the hold capacitor charging
    /// through the glide pot.
    pub fn trigger(&mut self, note: u8, velocity: f32, age: u64, glide_from_cv: Option<f32>) {
        // A card keeps its charge only when it keeps its note: re-striking
        // the key of a voice that is still gated is the classic hardware
        // retrigger. Anything else — a steal, or a re-press on a card
        // still ringing out — is a REASSIGNMENT, and must attack from
        // scratch or a slow attack would be skipped entirely.
        let same_note = self.held && self.note == Some(note);
        self.strike(note, velocity, age, glide_from_cv, !same_note);
    }

    /// A mono channel's new gate on the card it already owns (SH-101
    /// style: one voice, no reassignment). The envelopes re-attack from
    /// whatever charge their caps hold, so a detached line flows without
    /// the steal discharge's dip.
    pub fn restrike(&mut self, note: u8, velocity: f32, age: u64, glide_from_cv: Option<f32>) {
        self.strike(note, velocity, age, glide_from_cv, false);
    }

    /// A mono channel's legato move: the gate never dropped, so only the
    /// pitch CV changes. Neither envelope re-strikes, and velocity stays
    /// what the gate's leading edge sampled — a VCA gain step at every
    /// slurred note would click.
    pub fn legato(&mut self, note: u8, age: u64, glide_from_cv: Option<f32>) {
        // a new note: the last one's bend does not carry over (the pitch
        // envelope does not re-fire — the gate never dropped)
        self.bend_oct = 0.0;
        self.tune(note, glide_from_cv);
        self.age = age;
        self.held = true;
    }

    /// Gate the card onto `note`. `reassign` = the card is handed a note
    /// it does not own (discharge, then attack); otherwise its envelopes
    /// resume from their current level.
    fn strike(
        &mut self,
        note: u8,
        velocity: f32,
        age: u64,
        glide_from_cv: Option<f32>,
        reassign: bool,
    ) {
        // A reassigned card must not carry an old chord's widening CV into
        // its new note. Retriggers keep it, because they are the same tone.
        if reassign {
            self.pitch_shift_ratio = 1.0;
            self.pitch_shift_target = 1.0;
        }
        self.tune(note, glide_from_cv);
        // A new gate fires the pitch envelope from the top and clears any
        // bend; key sync restarts the cores so every hit starts alike
        self.pitch_env_x = 0.0;
        self.bend_oct = 0.0;
        if self.key_sync {
            for osc in &mut self.oscs {
                osc.reset_phase(0.25);
            }
            self.supersaw.sync();
        }
        if reassign {
            self.envelope.note_on_stolen();
            self.filter_env.note_on_stolen();
        } else {
            self.envelope.note_on();
            self.filter_env.note_on();
        }
        self.velocity = velocity.clamp(0.0, 1.0);
        self.age = age;
        self.held = true;
    }

    /// Set the keyboard CV to `note`: the glide distance from the source
    /// CV, then each oscillator's converter with its own tolerances.
    fn tune(&mut self, note: u8, glide_from_cv: Option<f32>) {
        let new_cv = note_cv(note);
        if self.glide_rate < 0.999 {
            // No source CV (the first note, or a chord member that must
            // start in tune): begin AT the target. Leaving the offset
            // alone would apply a stale distance left over from an
            // interrupted glide to a note it has nothing to do with.
            self.glide_offset = match glide_from_cv {
                Some(prev_cv) => (prev_cv - new_cv).clamp(-5.0, 5.0),
                None => 0.0,
            };
        } else {
            self.glide_offset = 0.0;
        }

        let frequency = Oscillator::note_to_frequency(note)
            * (self.unison_cents / 1200.0 * std::f32::consts::LN_2).exp();
        let octaves_from_ref = (frequency / CAL_REF_HZ).log2();
        for (osc, err_cents_per_oct) in self.oscs.iter().zip(self.voct_error) {
            // V/oct tracking error grows with distance from the calibration
            // point, then the finite reset time flattens the top end
            let scale =
                (err_cents_per_oct * octaves_from_ref / 1200.0 * std::f32::consts::LN_2).exp();
            let f = frequency * scale;
            osc.set_frequency(f / (1.0 + f * RESET_TIME));
        }
        self.note = Some(note);
    }

    pub fn release(&mut self) {
        self.envelope.note_off();
        self.filter_env.note_off();
        self.held = false;
    }

    pub fn is_held(&self) -> bool {
        self.held
    }

    pub fn age(&self) -> u64 {
        self.age
    }

    pub fn is_active(&self) -> bool {
        self.held || !self.envelope.is_idle()
    }

    /// Clear the last coupling delta when the card sleeps. The manager
    /// uses this before channel lookups; direct voice rendering uses the
    /// same rule, including the performance-line override.
    /// Whether `skip_if_idle` would put this card to sleep (read-only).
    pub(crate) fn is_sleeping(&self) -> bool {
        !self.held && self.envelope.is_idle() && self.cv_override.is_none()
    }

    pub(crate) fn skip_if_idle(&mut self) -> bool {
        if !self.held && self.envelope.is_idle() && self.cv_override.is_none() {
            self.prefilter_delta = 0.0;
            true
        } else {
            false
        }
    }

    /// The change of this card's pre-filter node last sample — what the
    /// neighbor's trace capacitance picks up.
    pub fn prefilter_delta(&self) -> f32 {
        self.prefilter_delta
    }

    /// `pitch_mult`, `lfo_cutoff_oct`, and `pw_offset` carry the global
    /// LFO modulation — one LFO drives every voice together (`pw_offset`
    /// rides on this voice's own base pulse width). `substrate` is the
    /// shared chassis state (rail sag, ripple, heat), and `bleed` the
    /// neighboring card's capacitively coupled signal.
    pub fn render_next(
        &mut self,
        noise: f32,
        pitch_mult: f32,
        lfo_cutoff_oct: f32,
        pw_offset: f32,
        substrate: SubstrateState,
        bleed: f32,
    ) -> (f32, f32) {
        if self.skip_if_idle() {
            return (0.0, 0.0);
        }
        let pulse_width = (self.pulse_width + pw_offset).clamp(0.05, 0.95);
        let amp_env = self.envelope.next_sample();
        let filter_env = self.filter_env.next_sample();

        self.pitch_shift_ratio +=
            (self.pitch_shift_target - self.pitch_shift_ratio) * self.pitch_shift_k;

        // Voice-shared drift walk (common controller and supply), roughly
        // twice the size of each core's individual residue
        self.drift_rng ^= self.drift_rng << 13;
        self.drift_rng ^= self.drift_rng >> 17;
        self.drift_rng ^= self.drift_rng << 5;
        let r = (self.drift_rng >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
        self.common_drift = (self.common_drift + r * 2.4e-5) * 0.9995;

        // Glide: the CV settles toward the target note; exponential in
        // octave space, so the audible swoop is geometric in frequency
        let pitch_mult = if let Some(cv) = self.cv_override {
            // Performance line: absolute pitch, relative to the note the
            // oscillators were tuned to at trigger
            let note_cv = note_cv(self.note.unwrap_or(69));
            pitch_mult * (cv - note_cv).exp2()
        } else if self.glide_offset != 0.0 {
            // Linear slew, exact arrival: step toward the target and STOP
            if self.glide_offset.abs() <= self.glide_rate {
                self.glide_offset = 0.0;
                pitch_mult
            } else {
                self.glide_offset -= self.glide_rate * self.glide_offset.signum();
                pitch_mult * self.glide_offset.exp2()
            }
        } else {
            pitch_mult
        };
        // Chassis coupling: this card's share of rail sag/ripple and heat.
        // The ARP boards are temperature-compensated (T.C. resistor in the
        // expo converter; 4027-1 "internally compensated") so they ride
        // the chassis weather at a fraction of the Moog's sway — the Moog
        // manual demands a 30-minute warm-up before even adjusting one.
        let temp_sens = self.substrate_sens
            * match self.circuit {
                CircuitModel::Moog => 1.0,
                CircuitModel::Arp => 0.4,
            };
        let pitch_mult =
            pitch_mult * self.pitch_shift_ratio * (1.0 + (substrate.pitch_mult - 1.0) * temp_sens);
        // The pitch envelope's dive, then the note bend — both in CV
        // space, on top of everything above (the tail lands exactly on
        // the note: past x = 1 the factor is gone, not merely 1.0)
        let pitch_mult = if self.pitch_env_oct != 0.0 && self.pitch_env_x < 1.0 {
            let off = self.pitch_env_oct * (1.0 - self.pitch_env_x).powf(self.pitch_env_power);
            self.pitch_env_x += self.pitch_env_inc;
            pitch_mult * off.exp2()
        } else {
            pitch_mult
        };
        let pitch_mult = if self.bend_oct != 0.0 {
            let shape = if self.bend_x >= 1.0 {
                1.0
            } else {
                1.0 - (1.0 - self.bend_x).powf(self.bend_power)
            };
            self.bend_x = (self.bend_x + self.bend_inc).min(1.0);
            pitch_mult * (self.bend_oct * shape).exp2()
        } else {
            pitch_mult
        };

        // The voice's coupling graph is acyclic (every stage buffered, as
        // the schematics show), so integrating stages in topological order
        // with midpoint-averaged couplings IS the coherent-system solve.
        // Osc 1 steps first (its wrap this sample masters osc 2's sync);
        // the FM coupling into osc 1 therefore uses osc 2's previous
        // sample — the one-sample transport of the sync/FM pair, chosen so
        // the sync reset lands at its exact sub-sample position.
        let fm_mult = if self.fm_amount > 1e-4 {
            let raw = (self.fm_amount * self.prev_osc2 * 2.0).exp2();
            // Exponential FM is not pitch-neutral: any DC in the modulator
            // (a non-50% pulse) shifts the note wholesale, and even a
            // zero-mean modulator reads sharp on average (E[2^x] > 1).
            // Track the multiplier's slow mean (tau FM_MEAN_TAU_S) and
            // divide it out: the audio-rate sidebands — the TIMBRE — pass
            // through, the tuning stays put. The coefficient is derived
            // from the rate, or the tracker would follow twice as much of
            // the modulator at 96 kHz and cancel real FM movement.
            self.fm_mean += (raw - self.fm_mean) * self.fm_mean_k;
            raw / self.fm_mean.max(1e-3)
        } else {
            self.fm_mean = 1.0;
            1.0
        };
        // Every core always steps (phase, drift and sync stay exactly as if
        // heard); only the output stages of a converter nobody can hear are
        // skipped. Osc 2 is heard through its level, the FM bus, or the ring
        // modulator; osc 3 only through its level; the sub only through its.
        let o1 = self.oscs[0].step(
            self.common_drift,
            pitch_mult * fm_mult,
            pulse_width,
            None,
            true,
            self.sub_level > 0.0,
        );
        let sync = if self.sync_on {
            self.oscs[0].wrap_frac()
        } else {
            None
        };
        // The supersaw bank rides oscillator 1's exact frequency and
        // crossfades in over its wave
        let o1 = if self.supersaw_amt > 0.0 {
            let bank = self.supersaw.render(
                self.oscs[0].last_hz(),
                self.sample_rate,
                self.supersaw_detune,
                self.supersaw_mix,
            );
            o1 + (bank - o1) * self.supersaw_amt
        } else {
            o1
        };
        let o1 = if self.osc1_level != 1.0 {
            o1 * self.osc1_level
        } else {
            o1
        };
        let o2_heard =
            self.osc_level[0] > 0.0 || self.fm_amount > 1e-4 || self.ring_amount > 1e-4;
        let o2 = self.oscs[1].step(self.common_drift, pitch_mult, pulse_width, sync, o2_heard, false);
        let o3_heard = self.osc_level[1] > 0.0;
        let o3 = self.oscs[2].step(self.common_drift, pitch_mult, pulse_width, None, o3_heard, false);
        self.prev_osc2 = (o2 / (0.9 * PROGRAM_V)).clamp(-1.0, 1.0);

        // Ring modulator (ARP: "the product of the two input voltages
        // divided by 5" — PROGRAM_V, since we carry volts), with this
        // unit's carrier leakage after the null trim
        let mut osc_mix = o1 + o2 * self.osc_level[0] + o3 * self.osc_level[1];
        if self.ring_amount > 1e-4 {
            let ring = o1 * o2 / PROGRAM_V + self.ring_leak.0 * o1 + self.ring_leak.1 * o2;
            osc_mix = osc_mix * (1.0 - self.ring_amount) + ring * 2.4 * self.ring_amount;
        }

        // Volts everywhere: the mixer sums program-level signals into the
        // VCF's summing junction
        let osc = osc_mix * MIXER_GAIN + self.oscs[0].sub() * self.sub_level * 0.9 + noise + bleed;
        let osc = if self.noise_osc.active() {
            osc + self.noise_osc.render()
        } else {
            osc
        };

        // Remember the pre-filter node for the neighbor's trace capacitance
        self.prefilter_delta = osc - self.prev_prefilter;
        self.prev_prefilter = osc;

        // Cutoff modulation in octaves: filter envelope, key tracking, velocity
        let note = self.note.unwrap_or(60) as f32;
        let key_oct = (note - 60.0) / 12.0 * self.key_track;
        let vel_oct = (self.velocity - 0.5) * self.vel_filter;
        let mod_oct = filter_env * self.filter_env_amount
            + key_oct
            + vel_oct
            + lfo_cutoff_oct
            + substrate.cutoff_oct * temp_sens;
        let cutoff_mult = mod_oct.exp2();

        // The drive stage, so the ladder after it is a post-filter
        let driven = if self.shaper.active() {
            self.shaper.process(osc)
        } else {
            self.shaper.bypassed();
            osc
        };
        let filtered = self.hpf.process(self.filter.process(driven, cutoff_mult));
        // Post-filter DC block (ARP R162 / Moog output coupling): removes
        // the operating-point DC the unipolar and asymmetric pulses push
        // through the ladder, before the VCA can gate it into thumps
        let filtered = {
            let y = filtered - self.dc_x1 + self.dc_pole * self.dc_y1;
            self.dc_x1 = filtered;
            self.dc_y1 = y;
            y
        };

        // Square-law velocity curve on amplitude, 1 - vel_amp * (1 - v^2),
        // written so the default 0.7 is the original 0.3 + 0.7 v^2 to the
        // last bit (1 - 0.7f32 is exactly 0.3f32)
        let vel_gain = (1.0 - self.vel_amp) + self.vel_amp * self.velocity * self.velocity;
        // The VCA never fully closes: the -60 dB floor keeps the
        // free-running oscillators faintly alive between notes
        let sample = filtered * (amp_env * vel_gain + VCA_FLOOR) * self.unison_gain;

        (sample * self.pan_l, sample * self.pan_r)
    }

    pub fn set_filter_cutoff(&mut self, cutoff: f32) {
        self.filter.set_cutoff(cutoff);
    }

    pub fn set_filter_resonance(&mut self, resonance: f32) {
        self.filter.set_resonance(resonance);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oscillator::Waveform;

    /// Autocorrelation f0 tracker (same approach as vox.rs tests).
    fn track_f0(samples: &[f32], sr: f32, lo: f32, hi: f32) -> f32 {
        let min_lag = (sr / hi) as usize;
        let max_lag = ((sr / lo) as usize).min(samples.len() / 2);
        let mut best = (f32::MIN, min_lag);
        for lag in min_lag..=max_lag {
            let n = samples.len() - lag;
            let (mut num, mut d1, mut d2) = (0.0f32, 0.0f32, 0.0f32);
            for i in 0..n {
                num += samples[i] * samples[i + lag];
                d1 += samples[i] * samples[i];
                d2 += samples[i + lag] * samples[i + lag];
            }
            let r = num / (d1 * d2).sqrt().max(1e-12);
            if r > best.0 {
                best = (r, lag);
            }
        }
        sr / best.1 as f32
    }

    fn held_voice_f0(fm: f32, osc2_wave: Waveform, osc2_semis: f32, pw: f32) -> f32 {
        let sr = 48000.0;
        let mut v = Voice::new(sr, 0);
        v.set_waveform(Waveform::Sine);
        v.set_osc_waveform(1, osc2_wave);
        v.set_osc_pitch(1, osc2_semis);
        v.set_osc_level(1, 0.0); // modulator inaudible: FM path only
        v.set_pulse_width(pw);
        v.set_fm_amount(fm);
        v.set_filter_cutoff(20000.0);
        v.trigger(69, 1.0, 0, None); // A4
        let neutral = SubstrateState {
            pitch_mult: 1.0,
            cutoff_oct: 0.0,
        };
        // settle past the fm_mean tracker's time constant
        for _ in 0..(sr as usize) {
            v.render_next(0.0, 1.0, 0.0, 0.0, neutral, 0.0);
        }
        let samples: Vec<f32> = (0..(sr / 2.0) as usize)
            .map(|_| v.render_next(0.0, 1.0, 0.0, 0.0, neutral, 0.0).0)
            .collect();
        track_f0(&samples, sr, 200.0, 900.0)
    }

    /// FM must change timbre, not tuning. Modulator an OCTAVE up so the
    /// composite stays periodic at f0 (an autocorrelation tracker can't
    /// name the pitch of a +7-semitone FM tone), with a DC-heavy 30%
    /// pulse — the recipe class that shipped a flat lead. The note must
    /// stay within a few cents of the FM-off pitch.
    #[test]
    fn fm_is_pitch_neutral() {
        let dry = held_voice_f0(0.0, Waveform::Square, 12.0, 0.3);
        let fm = held_voice_f0(0.3, Waveform::Square, 12.0, 0.3);
        let cents = 1200.0 * (fm / dry).log2();
        assert!(
            cents.abs() < 8.0,
            "FM shifted pitch by {cents:.1} cents (dry {dry:.2} Hz, fm {fm:.2} Hz)"
        );
    }

    const NEUTRAL: SubstrateState = SubstrateState {
        pitch_mult: 1.0,
        cutoff_oct: 0.0,
    };

    /// End-to-end: a card still sounding one note, handed a DIFFERENT
    /// note, must play the new one through its attack. This is the
    /// reported bug — with a slow attack the second note arrived instantly
    /// because the card inherited the first note's envelope level.
    #[test]
    fn a_stolen_card_replays_its_slow_attack() {
        let sr = 48000.0;
        let mut v = Voice::new(sr, 0);
        v.set_waveform(Waveform::Sawtooth);
        v.set_filter_cutoff(18000.0);
        v.set_filter_env_amount(0.0);
        v.envelope.set_attack(0.5);
        v.envelope.set_decay(2.0);
        v.envelope.set_sustain(1.0);

        v.trigger(60, 1.0, 0, None);
        for _ in 0..sr as usize {
            v.render_next(0.0, 1.0, 0.0, 0.0, NEUTRAL, 0.0);
        }
        let mut sustained = 0.0f32;
        for _ in 0..(0.05 * sr) as usize {
            let (l, _) = v.render_next(0.0, 1.0, 0.0, 0.0, NEUTRAL, 0.0);
            sustained = sustained.max(l.abs());
        }
        assert!(sustained > 0.1, "first note never got loud: {sustained}");

        // The same card is now handed a different note
        v.trigger(67, 1.0, 1, None);
        // Skip the discharge ramp, then measure the new note's level
        for _ in 0..(0.005 * sr) as usize {
            v.render_next(0.0, 1.0, 0.0, 0.0, NEUTRAL, 0.0);
        }
        let mut after = 0.0f32;
        for _ in 0..(0.045 * sr) as usize {
            let (l, _) = v.render_next(0.0, 1.0, 0.0, 0.0, NEUTRAL, 0.0);
            after = after.max(l.abs());
        }
        assert!(
            after < 0.4 * sustained,
            "a stolen card must replay its 500 ms attack: 50 ms in it is at \
             {after:.4} vs a sustained {sustained:.4}"
        );
    }

    /// Glide with no source CV (the first note, or a chord member that
    /// must start in tune) has to start AT the target. Leaving the offset
    /// alone let an interrupted glide's leftover distance be applied to an
    /// unrelated later note, which lands it octaves out.
    #[test]
    fn a_card_with_no_source_cv_starts_in_tune() {
        let sr = 48000.0;
        let mut v = Voice::new(sr, 0);
        v.set_glide_rate(1.0 / (2.0 * sr)); // 2 seconds per octave
                                            // A glide two octaves long, interrupted well before it arrives
        v.trigger(48, 1.0, 0, Some((72.0 - 69.0) / 12.0));
        assert!(
            v.glide_remaining().abs() > 1.0,
            "expected a long pending glide, got {}",
            v.glide_remaining()
        );
        v.trigger(60, 1.0, 1, None);
        assert_eq!(
            v.glide_remaining(),
            0.0,
            "a stale glide distance was carried into an unrelated note"
        );
    }

    /// The output coupling is a CAPACITOR: 2.5 uF into the following
    /// stage puts its corner at ~32 Hz and leaves it there. A hardcoded
    /// pole made it a 32 Hz high-pass at 44.1 kHz and a 69 Hz one at
    /// 96 kHz, so the same bass patch lost its bottom octave depending on
    /// what the host was running at.
    #[test]
    fn the_output_coupling_corner_does_not_track_the_sample_rate() {
        // E1 (~41 Hz) sits right on the coupling's knee, where a moved
        // corner shows up as a level change
        let level_at = |sr: f32| -> f32 {
            let mut v = Voice::new(sr, 0);
            v.set_waveform(Waveform::Sine);
            v.set_filter_cutoff(16000.0);
            v.set_filter_env_amount(0.0);
            v.envelope.set_attack(0.005);
            v.envelope.set_decay(0.01);
            v.envelope.set_sustain(1.0);
            v.trigger(28, 1.0, 0, None);
            for _ in 0..(0.5 * sr) as usize {
                v.render_next(0.0, 1.0, 0.0, 0.0, NEUTRAL, 0.0);
            }
            let mut peak = 0.0f32;
            for _ in 0..(0.5 * sr) as usize {
                let (l, _) = v.render_next(0.0, 1.0, 0.0, 0.0, NEUTRAL, 0.0);
                peak = peak.max(l.abs());
            }
            peak
        };
        let a = level_at(44100.0);
        let b = level_at(96000.0);
        assert!(a > 0.05, "the low note never sounded: {a}");
        assert!(
            (a / b - 1.0).abs() < 0.08,
            "a 41 Hz note came out at {a:.4} at 44.1 kHz but {b:.4} at 96 kHz \
             — the DC block's corner is tracking the sample rate"
        );
    }

    /// The FM mean-tracker is specified as a time constant, so it must
    /// cancel the same amount of the modulator at every rate. Left as a
    /// per-sample number it ran at 91 ms at 44.1 kHz and 42 ms at 96 kHz,
    /// following — and cancelling — twice as much real FM movement.
    #[test]
    fn the_fm_mean_tracker_holds_its_time_constant() {
        // How far the tracker has travelled after ONE time constant of
        // real time. If the coefficient is hardcoded it races ahead at
        // the higher rate and cancels FM movement it should have passed.
        let travelled_after_one_tau = |sr: f32| -> f32 {
            let mut v = Voice::new(sr, 0);
            v.set_waveform(Waveform::Sine);
            v.set_osc_waveform(1, Waveform::Square);
            v.set_osc_pitch(1, 12.0);
            v.set_osc_level(1, 0.0);
            v.set_pulse_width(0.7); // DC-heavy modulator: mean well off 1.0
            v.set_fm_amount(0.6);
            v.set_filter_cutoff(18000.0);
            v.trigger(69, 1.0, 0, None);
            for _ in 0..(FM_MEAN_TAU_S * sr) as usize {
                v.render_next(0.0, 1.0, 0.0, 0.0, NEUTRAL, 0.0);
            }
            v.fm_mean // starts at exactly 1.0
        };
        let a = travelled_after_one_tau(44100.0);
        let b = travelled_after_one_tau(96000.0);
        assert!(
            (a - 1.0).abs() > 0.01,
            "the tracker never moved, so this proves nothing: {a}"
        );
        assert!(
            ((a - 1.0) / (b - 1.0) - 1.0).abs() < 0.06,
            "after one time constant of REAL time the FM mean-tracker had \
             reached {a:.4} at 44.1 kHz but {b:.4} at 96 kHz"
        );
    }

    /// The defaults ARE the old hard-wired curve, bit for bit: every
    /// patch and song written before velocity became a patch control must
    /// render exactly as it did.
    #[test]
    fn default_velocity_response_is_the_original_curve() {
        for step in 0..=127u8 {
            let v = step as f32 / 127.0;
            let amp = (1.0 - VEL_AMP) + VEL_AMP * v * v;
            assert_eq!(
                amp.to_bits(),
                (0.3f32 + 0.7 * v * v).to_bits(),
                "velocity {step}"
            );
            assert_eq!(
                ((v - 0.5) * VEL_TRACK).to_bits(),
                ((v - 0.5) * 0.8f32).to_bits()
            );
        }
    }

    /// The kick-bass dive, read straight off oscillator 1's running
    /// frequency: a card with the pitch envelope against the same card
    /// without it (same tolerances, same drift walk) must differ by
    /// exactly the set curve, starting `pitch_env` semitones up and
    /// landing on the note after `pitch_env_time`, then holding it.
    #[test]
    fn the_pitch_envelope_dives_onto_the_note_and_holds_it() {
        let sr = 48000.0;
        let (semis, secs, curve) = (48.0f32, 0.12f32, 0.6f32);
        let run = |env: bool| -> Vec<f32> {
            let mut v = Voice::new(sr, 3);
            v.set_waveform(Waveform::Sine);
            if env {
                v.set_pitch_env(semis, secs, curve);
            }
            v.trigger(29, 1.0, 0, None); // F1
            (0..(0.3 * sr) as usize)
                .map(|_| {
                    v.render_next(0.0, 1.0, 0.0, 0.0, NEUTRAL, 0.0);
                    v.oscs[0].last_hz()
                })
                .collect()
        };
        let (dive, flat) = (run(true), run(false));
        let p = (4.0 * curve).exp2();
        for (n, (d, f)) in dive.iter().zip(&flat).enumerate() {
            let x = n as f32 / (secs * sr);
            let want = if x < 1.0 {
                (semis / 12.0 * (1.0 - x).powf(p)).exp2()
            } else {
                1.0
            };
            let got = d / f;
            assert!(
                (got / want - 1.0).abs() < 2e-3,
                "sample {n} ({:.1} ms): ratio {got} vs set {want}",
                n as f32 / sr * 1000.0
            );
            if x >= 1.0 {
                assert_eq!(d, f, "the tail must sit exactly on the note (sample {n})");
            }
        }
        assert!((dive[0] / flat[0] / 16.0 - 1.0).abs() < 2e-3, "starts 4 octaves up");
    }

    /// A new gate re-fires the dive; a mono legato move does not.
    #[test]
    fn the_dive_fires_on_gates_not_on_legato_moves() {
        let sr = 48000.0;
        let mut v = Voice::new(sr, 0);
        v.set_pitch_env(24.0, 0.05, 0.0);
        v.trigger(40, 1.0, 0, None);
        for _ in 0..(0.1 * sr) as usize {
            v.render_next(0.0, 1.0, 0.0, 0.0, NEUTRAL, 0.0);
        }
        assert_eq!(v.pitch_offsets_for_test().0, 0.0, "landed");
        v.legato(43, 1, None);
        assert_eq!(v.pitch_offsets_for_test().0, 0.0, "legato keeps the gate");
        v.restrike(45, 1.0, 2, None);
        assert!((v.pitch_offsets_for_test().0 - 2.0).abs() < 1e-6, "a new gate dives again");
    }

    /// Key sync: every hit of a synced card restarts its oscillators at
    /// the same phase, so two hits a different time apart start alike;
    /// a free-running card picks up wherever its cores happen to be.
    #[test]
    fn key_sync_makes_every_hit_start_alike() {
        let sr = 48000.0;
        let first_cycle = |sync: bool, gap: usize| -> Vec<f32> {
            let mut v = Voice::new(sr, 1);
            v.set_waveform(Waveform::Sine);
            v.key_sync = sync;
            v.trigger(36, 1.0, 0, None);
            for _ in 0..gap {
                v.render_next(0.0, 1.0, 0.0, 0.0, NEUTRAL, 0.0);
            }
            v.release();
            v.trigger(36, 1.0, 1, None);
            // the core's own output, sample by sample from the gate
            (0..64).map(|_| v.oscs[0].next_sample(0.0, 1.0, 0.5, None)).collect()
        };
        let diff = |a: &[f32], b: &[f32]| {
            a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max)
        };
        let (a, b) = (first_cycle(true, 1000), first_cycle(true, 1400));
        assert!(diff(&a, &b) < 0.01, "synced hits start apart by {}", diff(&a, &b));
        assert!(a[0].abs() < 0.05, "a synced sine starts at its zero crossing: {}", a[0]);
        let (c, d) = (first_cycle(false, 1000), first_cycle(false, 1400));
        assert!(diff(&c, &d) > 0.5, "free-running hits should start apart");
    }

    /// One card's output, `seconds` into a held note.
    fn held_note(seconds: f32, set: impl Fn(&mut Voice), velocity: f32) -> Vec<f32> {
        let sr = 48000.0;
        let mut v = Voice::new(sr, 0);
        v.set_waveform(Waveform::Sawtooth);
        v.set_filter_resonance(0.0);
        v.envelope.set_attack(0.005);
        v.envelope.set_sustain(1.0);
        set(&mut v);
        v.trigger(48, velocity, 0, None);
        (0..(seconds * sr) as usize)
            .map(|_| v.render_next(0.0, 1.0, 0.0, 0.0, NEUTRAL, 0.0).0)
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// `vel_amp 0` takes velocity out of the VCA. With `vel_filter 0` as
    /// well, velocity has no path left into the card: a soft and a hard
    /// strike are the same signal, sample for sample.
    #[test]
    fn vel_amp_zero_makes_the_level_velocity_blind() {
        let deaf = |v: &mut Voice| {
            v.set_filter_cutoff(4000.0);
            v.set_vel_amp(0.0);
            v.set_vel_filter(0.0);
        };
        let soft = held_note(0.3, deaf, 0.15);
        let hard = held_note(0.3, deaf, 1.0);
        assert!(rms(&hard) > 0.05, "the note never sounded");
        assert!(
            soft == hard,
            "vel_amp 0 still let velocity reach the output"
        );

        // ...where the default curve drops a soft strike well down
        let open = |v: &mut Voice| {
            v.set_filter_cutoff(4000.0);
            v.set_vel_filter(0.0);
        };
        let ratio = rms(&held_note(0.3, open, 0.15)) / rms(&held_note(0.3, open, 1.0));
        assert!(
            (ratio - (0.3 + 0.7 * 0.15 * 0.15)).abs() < 0.01,
            "default vel_amp should scale a soft strike to ~0.32, got {ratio:.3}"
        );
    }

    /// `vel_filter` is octaves of cutoff across the velocity swing: a
    /// full strike opens (v - 0.5) * vel_filter octaves, so at
    /// `vel_filter 2` it sounds like the same patch an octave brighter.
    #[test]
    fn vel_filter_scales_the_cutoff_shift() {
        let bright = |cutoff: f32, vel_filter: f32| {
            let x = held_note(
                0.4,
                |v| {
                    v.set_filter_cutoff(cutoff);
                    v.set_vel_amp(0.0);
                    v.set_vel_filter(vel_filter);
                },
                1.0,
            );
            rms(&x[(0.2 * 48000.0) as usize..])
        };
        let base = bright(500.0, 0.0);
        let one_octave = bright(1000.0, 0.0);
        let two_octaves = bright(2000.0, 0.0);
        assert!(
            one_octave > base * 1.1,
            "the probe cannot hear an octave: {base} vs {one_octave}"
        );
        let by_vel = bright(500.0, 2.0);
        assert!(
            (by_vel / one_octave - 1.0).abs() < 0.02,
            "vel_filter 2 at full velocity should open one octave: {by_vel} vs {one_octave}"
        );
        let by_vel = bright(500.0, 4.0);
        assert!(
            (by_vel / two_octaves - 1.0).abs() < 0.02,
            "vel_filter 4 at full velocity should open two octaves: {by_vel} vs {two_octaves}"
        );
    }
}
