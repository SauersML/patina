use crate::oscillator::Oscillator;
use crate::envelope::Envelope;
use crate::filter::LadderFilter;

pub struct Voice {
    pub oscillator: Oscillator,
    pub envelope: Envelope,
    pub filter: LadderFilter,
    pub note: Option<u8>,
    velocity: f32,
    age: u64,
    held: bool,
}

impl Voice {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            oscillator: Oscillator::new(sample_rate, 440.0),
            envelope: Envelope::new(sample_rate),
            filter: LadderFilter::new(sample_rate),
            note: None,
            velocity: 0.0,
            age: 0,
            held: false,
        }
    }

    pub fn trigger(&mut self, note: u8, velocity: f32, age: u64) {
        let frequency = Oscillator::note_to_frequency(note);
        self.oscillator.set_frequency(frequency);
        self.envelope.note_on();
        self.note = Some(note);
        self.velocity = velocity.clamp(0.0, 1.0);
        self.age = age;
        self.held = true;
    }

    pub fn release(&mut self) {
        self.envelope.note_off();
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

    pub fn render_next(&mut self) -> f32 {
        let osc_sample = self.oscillator.next_sample();
        let env_sample = self.envelope.next_sample();
        self.filter.process(osc_sample * env_sample) * self.velocity
    }

    pub fn set_filter_cutoff(&mut self, cutoff: f32) {
        self.filter.set_cutoff(cutoff);
    }

    pub fn set_filter_resonance(&mut self, resonance: f32) {
        self.filter.set_resonance(resonance);
    }
}
