use crate::oscillator::{Oscillator, Waveform};
use crate::envelope::Envelope;
use crate::filter::LadderFilter;
use crate::hpf::HighPassLadder;

/// Fixed keyboard tracking: how much the filter follows note pitch, in
/// octaves of cutoff per octave of pitch.
const KEY_TRACK: f32 = 0.4;
/// How much velocity opens the filter, in octaves at full velocity swing.
const VEL_TRACK: f32 = 0.8;

/// Finite sawtooth-core reset time (US 3,943,456 variable-rate integrator):
/// the discharge takes real time, so f_actual = f / (1 + f * T_RESET) and
/// high notes land slightly flat, like an analog VCO between calibrations.
const RESET_TIME: f32 = 1.5e-6;

/// Exponential-converter calibration reference (~C4). V/oct scaling error
/// accumulates in cents per octave away from this point.
const CAL_REF_HZ: f32 = 261.63;

pub struct Voice {
    pub oscs: [Oscillator; 3],
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
    /// Portamento, per US 3,991,645: the keyboard CV charges the hold
    /// capacitor through a glide resistance, so pitch settles exponentially
    /// in CV (octave) space BEFORE the expo converter. `glide_offset` is the
    /// remaining octave distance from the target note; `glide_k` the
    /// per-sample RC coefficient (1.0 = instant).
    glide_offset: f32,
    glide_k: f32,
    /// Juno-style sub-oscillator level: the first oscillator's divide-by-two
    /// square, mixed in before the filter.
    sub_level: f32,
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

        let mut voice = Self {
            oscs: [
                Oscillator::new(sample_rate, 440.0, seed),
                Oscillator::new(sample_rate, 440.0, seed.wrapping_add(101)),
                Oscillator::new(sample_rate, 440.0, seed.wrapping_add(211)),
            ],
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
            glide_k: 1.0,
            sub_level: 0.0,
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
    /// symmetrically up and down.
    pub fn set_detune(&mut self, cents: f32) {
        let cents = cents.clamp(0.0, 50.0);
        let ratio = (cents / 1200.0 * std::f32::consts::LN_2).exp();
        self.oscs[0].set_freq_mult(1.0);
        self.oscs[1].set_freq_mult(ratio);
        self.oscs[2].set_freq_mult(1.0 / ratio);
    }

    pub fn set_filter_env_amount(&mut self, octaves: f32) {
        self.filter_env_amount = octaves.clamp(-5.0, 5.0);
    }

    pub fn set_glide_coef(&mut self, k: f32) {
        self.glide_k = k.clamp(1e-5, 1.0);
    }

    pub fn set_sub_level(&mut self, level: f32) {
        self.sub_level = level.clamp(0.0, 1.0);
    }

    /// `glide_from_cv` is the most recently played note's CV in octaves
    /// (relative to A440); when glide is active the new note starts from
    /// there and settles exponentially, like the hold capacitor charging
    /// through the glide pot.
    pub fn trigger(&mut self, note: u8, velocity: f32, age: u64, glide_from_cv: Option<f32>) {
        let new_cv = (note as f32 - 69.0) / 12.0;
        if self.glide_k < 0.999 {
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

    /// `pitch_mult`, `lfo_cutoff_oct`, and `pulse_width` carry the global
    /// LFO modulation — one LFO drives every voice together.
    pub fn render_next(
        &mut self,
        noise: f32,
        pitch_mult: f32,
        lfo_cutoff_oct: f32,
        pulse_width: f32,
    ) -> (f32, f32) {
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
        let pitch_mult = if self.glide_offset.abs() > 1e-5 {
            self.glide_offset -= self.glide_offset * self.glide_k;
            pitch_mult * self.glide_offset.exp2()
        } else {
            pitch_mult
        };

        let osc = (self.oscs[0].next_sample(self.common_drift, pitch_mult, pulse_width)
            + self.oscs[1].next_sample(self.common_drift, pitch_mult, pulse_width)
            + self.oscs[2].next_sample(self.common_drift, pitch_mult, pulse_width))
            * (1.0 / 3.0)
            + self.oscs[0].sub() * self.sub_level * 0.9
            + noise;

        // Cutoff modulation in octaves: filter envelope, key tracking, velocity
        let note = self.note.unwrap_or(60) as f32;
        let key_oct = (note - 60.0) / 12.0 * KEY_TRACK;
        let vel_oct = (self.velocity - 0.5) * VEL_TRACK;
        let mod_oct = filter_env * self.filter_env_amount + key_oct + vel_oct + lfo_cutoff_oct;
        let cutoff_mult = mod_oct.exp2();

        let filtered = self.hpf.process(self.filter.process(osc, cutoff_mult));

        // Gentle velocity curve on amplitude
        let vel_amp = 0.3 + 0.7 * self.velocity * self.velocity;
        let sample = filtered * amp_env * vel_amp;

        (sample * self.pan_l, sample * self.pan_r)
    }

    pub fn set_filter_cutoff(&mut self, cutoff: f32) {
        self.filter.set_cutoff(cutoff);
    }

    pub fn set_filter_resonance(&mut self, resonance: f32) {
        self.filter.set_resonance(resonance);
    }
}
