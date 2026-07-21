use crate::oscillator::{CircuitModel, Oscillator, Waveform, PROGRAM_V};
use crate::envelope::Envelope;
use crate::filter::LadderFilter;
use crate::hpf::HighPassLadder;
use crate::substrate::SubstrateState;

/// How much velocity opens the filter, in octaves at full velocity swing.
const VEL_TRACK: f32 = 0.8;

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

pub struct Voice {
    pub oscs: [Oscillator; 3],
    circuit: CircuitModel,
    pub envelope: Envelope,
    pub filter_env: Envelope,
    pub filter: LadderFilter,
    pub hpf: HighPassLadder,
    pub note: Option<u8>,
    velocity: f32,
    age: u64,
    held: bool,
    pan_l: f32,
    pan_r: f32,
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
    /// Filter keyboard tracking, octaves of cutoff per octave of pitch.
    /// The 2600 trims this to full 1 V/oct so a self-oscillating filter
    /// plays in tune; the Minimoog offers fractional settings.
    key_track: f32,
    /// The 2600's prewired cross-oscillator FM: osc 2 modulates osc 1's
    /// frequency through the exponential converter (audio-rate, in CV
    /// space — the source of the growl).
    fm_amount: f32,
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
    /// This card's sensitivity to the shared chassis state (rail and heat):
    /// every board reacts to the same environment, each by its own amount.
    substrate_sens: f32,
    /// 902-style VCA control feedthrough, post-trim residue: the envelope's
    /// edge couples into the audio — the "thump" of a fast hardware attack.
    vca_feedthrough: f32,
    prev_env: f32,
    /// Pre-filter node history, exposed so the neighbor card can pick up
    /// its capacitively-coupled (differentiated) bleed.
    prev_prefilter: f32,
    prefilter_delta: f32,
    /// Which song channel configured this voice (0 = the live panel).
    channel: u16,
    /// This voice's base pulse width; the manager passes only the LFO's
    /// PWM offset, so different channels can hold different widths.
    pulse_width: f32,
    sample_rate: f32,
}

impl Voice {
    pub fn new(sample_rate: f32, index: usize, total: usize) -> Self {
        let seed = (index as u32).wrapping_add(1);

        // Spread voices across a modest stereo field, equal-power panned
        let spread = if total > 1 {
            index as f32 / (total - 1) as f32 - 0.5
        } else {
            0.0
        };
        let pan = spread * 0.5; // -0.25 .. +0.25
        let theta = (pan + 1.0) * std::f32::consts::FRAC_PI_4;

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
        let vca_feedthrough = rand01() * 0.35;
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
            age: 0,
            held: false,
            pan_l: theta.cos(),
            pan_r: theta.sin(),
            filter_env_amount: 0.0,
            voct_error,
            common_drift: 0.0,
            drift_rng: seed.wrapping_mul(0x27D4_EB2F) | 1,
            glide_offset: 0.0,
            glide_rate: 1.0,
            sub_level: 0.0,
            osc_pitch_semi: [0.0, 0.0],
            osc_level: [0.72, 0.72],
            detune_cents: 7.0,
            key_track: 0.4,
            fm_amount: 0.0,
            prev_osc2: 0.0,
            sync_on: false,
            ring_amount: 0.0,
            ring_leak,
            dc_x1: 0.0,
            dc_y1: 0.0,
            substrate_sens,
            vca_feedthrough,
            prev_env: 0.0,
            prev_prefilter: 0.0,
            prefilter_delta: 0.0,
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
        self.oscs[1]
            .set_freq_mult(fine * (self.osc_pitch_semi[0] / 12.0).exp2());
        self.oscs[2]
            .set_freq_mult((self.osc_pitch_semi[1] / 12.0).exp2() / fine);
    }

    pub fn set_filter_env_amount(&mut self, octaves: f32) {
        self.filter_env_amount = octaves.clamp(-5.0, 5.0);
    }

    pub fn set_glide_rate(&mut self, rate: f32) {
        self.glide_rate = rate.clamp(1e-7, 1.0);
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
    /// engine's per-track patches. Only voice-level parameters apply;
    /// bus effects, the LFO, and the noise source stay shared.
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
        let new_cv = (note as f32 - 69.0) / 12.0;
        if self.glide_rate < 0.999 {
            if let Some(prev_cv) = glide_from_cv {
                self.glide_offset = (prev_cv - new_cv).clamp(-5.0, 5.0);
            }
        } else {
            self.glide_offset = 0.0;
        }

        let frequency = Oscillator::note_to_frequency(note);
        let octaves_from_ref = (frequency / CAL_REF_HZ).log2();
        for (osc, err_cents_per_oct) in self.oscs.iter().zip(self.voct_error) {
            // V/oct tracking error grows with distance from the calibration
            // point, then the finite reset time flattens the top end
            let scale = (err_cents_per_oct * octaves_from_ref / 1200.0
                * std::f32::consts::LN_2)
                .exp();
            let f = frequency * scale;
            osc.set_frequency(f / (1.0 + f * RESET_TIME));
        }
        self.envelope.note_on();
        self.filter_env.note_on();
        self.note = Some(note);
        self.velocity = velocity.clamp(0.0, 1.0);
        self.age = age;
        self.held = true;
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
        let pulse_width = (self.pulse_width + pw_offset).clamp(0.05, 0.95);
        let amp_env = self.envelope.next_sample();
        let filter_env = self.filter_env.next_sample();

        // Voice-shared drift walk (common controller and supply), roughly
        // twice the size of each core's individual residue
        self.drift_rng ^= self.drift_rng << 13;
        self.drift_rng ^= self.drift_rng >> 17;
        self.drift_rng ^= self.drift_rng << 5;
        let r = (self.drift_rng >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
        self.common_drift = (self.common_drift + r * 2.4e-5) * 0.9995;

        // Glide: the CV settles toward the target note; exponential in
        // octave space, so the audible swoop is geometric in frequency
        let pitch_mult = if self.glide_offset != 0.0 {
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
        let pitch_mult = pitch_mult * (1.0 + (substrate.pitch_mult - 1.0) * temp_sens);

        // The voice's coupling graph is acyclic (every stage buffered, as
        // the schematics show), so integrating stages in topological order
        // with midpoint-averaged couplings IS the coherent-system solve.
        // Osc 1 steps first (its wrap this sample masters osc 2's sync);
        // the FM coupling into osc 1 therefore uses osc 2's previous
        // sample — the one-sample transport of the sync/FM pair, chosen so
        // the sync reset lands at its exact sub-sample position.
        let fm_mult = if self.fm_amount > 1e-4 {
            (self.fm_amount * self.prev_osc2 * 2.0).exp2()
        } else {
            1.0
        };
        let o1 = self.oscs[0].next_sample(self.common_drift, pitch_mult * fm_mult, pulse_width, None);
        let sync = if self.sync_on { self.oscs[0].wrap_frac() } else { None };
        let o2 = self.oscs[1].next_sample(self.common_drift, pitch_mult, pulse_width, sync);
        let o3 = self.oscs[2].next_sample(self.common_drift, pitch_mult, pulse_width, None);
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
        let osc = osc_mix * MIXER_GAIN
            + self.oscs[0].sub() * self.sub_level * 0.9
            + noise
            + bleed;

        // Remember the pre-filter node for the neighbor's trace capacitance
        self.prefilter_delta = osc - self.prev_prefilter;
        self.prev_prefilter = osc;

        // Cutoff modulation in octaves: filter envelope, key tracking, velocity
        let note = self.note.unwrap_or(60) as f32;
        let key_oct = (note - 60.0) / 12.0 * self.key_track;
        let vel_oct = (self.velocity - 0.5) * VEL_TRACK;
        let mod_oct = filter_env * self.filter_env_amount
            + key_oct
            + vel_oct
            + lfo_cutoff_oct
            + substrate.cutoff_oct * temp_sens;
        let cutoff_mult = mod_oct.exp2();

        let filtered = self.hpf.process(self.filter.process(osc, cutoff_mult));
        // Post-filter DC block (ARP R162 / Moog output coupling): removes
        // the operating-point DC the unipolar and asymmetric pulses push
        // through the ladder, before the VCA can gate it into thumps
        let filtered = {
            let y = filtered - self.dc_x1 + 0.9955 * self.dc_y1;
            self.dc_x1 = filtered;
            self.dc_y1 = y;
            y
        };

        // Gentle velocity curve on amplitude
        let vel_amp = 0.3 + 0.7 * self.velocity * self.velocity;
        // 902 control feedthrough: the envelope's edge couples into the
        // audio path (post-trim residue) — fast attacks thump, physically
        let feedthrough = (amp_env - self.prev_env) * self.vca_feedthrough * PROGRAM_V;
        self.prev_env = amp_env;
        // The VCA never fully closes: the -60 dB floor keeps the
        // free-running oscillators faintly alive between notes
        let sample = filtered * (amp_env * vel_amp + VCA_FLOOR) + feedthrough;

        (sample * self.pan_l, sample * self.pan_r)
    }

    pub fn set_filter_cutoff(&mut self, cutoff: f32) {
        self.filter.set_cutoff(cutoff);
    }

    pub fn set_filter_resonance(&mut self, resonance: f32) {
        self.filter.set_resonance(resonance);
    }
}
