use crate::oscillator::{Oscillator, Waveform};
use crate::envelope::Envelope;
use crate::filter::LadderFilter;

/// Fixed keyboard tracking: how much the filter follows note pitch, in
/// octaves of cutoff per octave of pitch.
const KEY_TRACK: f32 = 0.4;
/// How much velocity opens the filter, in octaves at full velocity swing.
const VEL_TRACK: f32 = 0.8;

pub struct Voice {
    pub oscs: [Oscillator; 3],
    pub envelope: Envelope,
    pub filter_env: Envelope,
    pub filter: LadderFilter,
    pub note: Option<u8>,
    velocity: f32,
    age: u64,
    held: bool,
    pan_l: f32,
    pan_r: f32,
    filter_env_amount: f32, // octaves, -5..+5
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

        let mut voice = Self {
            oscs: [
                Oscillator::new(sample_rate, 440.0, seed),
                Oscillator::new(sample_rate, 440.0, seed.wrapping_add(101)),
                Oscillator::new(sample_rate, 440.0, seed.wrapping_add(211)),
            ],
            envelope: Envelope::new(sample_rate),
            filter_env,
            filter: LadderFilter::new(sample_rate, seed),
            note: None,
            velocity: 0.0,
            age: 0,
            held: false,
            pan_l: theta.cos(),
            pan_r: theta.sin(),
            filter_env_amount: 0.0,
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

    pub fn trigger(&mut self, note: u8, velocity: f32, age: u64) {
        let frequency = Oscillator::note_to_frequency(note);
        for osc in &self.oscs {
            osc.set_frequency(frequency);
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

    pub fn render_next(&mut self) -> (f32, f32) {
        let amp_env = self.envelope.next_sample();
        let filter_env = self.filter_env.next_sample();

        let osc = (self.oscs[0].next_sample()
            + self.oscs[1].next_sample()
            + self.oscs[2].next_sample())
            * (1.0 / 3.0);

        // Cutoff modulation in octaves: filter envelope, key tracking, velocity
        let note = self.note.unwrap_or(60) as f32;
        let key_oct = (note - 60.0) / 12.0 * KEY_TRACK;
        let vel_oct = (self.velocity - 0.5) * VEL_TRACK;
        let mod_oct = filter_env * self.filter_env_amount + key_oct + vel_oct;
        let cutoff_mult = mod_oct.exp2();

        let filtered = self.filter.process(osc, cutoff_mult);

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
