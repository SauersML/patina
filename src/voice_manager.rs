use crate::voice::Voice;
use crate::reverb::Reverb;
use crate::chorus::{Chorus, ChorusMode};

pub struct VoiceManager {
    pub voices: Vec<Voice>,
    reverb: Reverb,
    chorus: Chorus,
    note_counter: u64,
}

impl VoiceManager {
    pub fn new(sample_rate: f32, num_voices: usize) -> Self {
        Self {
            voices: (0..num_voices).map(|_| Voice::new(sample_rate)).collect(),
            reverb: Reverb::new(sample_rate),
            chorus: Chorus::new(sample_rate),
            note_counter: 0,
        }
    }

    pub fn note_on(&mut self, note: u8, velocity: f32) {
        self.note_counter += 1;
        let age = self.note_counter;

        // Retrigger if this note is already held
        if let Some(voice) = self.voices.iter_mut().find(|v| v.is_held() && v.note == Some(note)) {
            voice.trigger(note, velocity, age);
            return;
        }

        // Prefer a fully idle voice, then the longest-releasing voice,
        // then steal the oldest held voice
        let index = self
            .voices
            .iter()
            .position(|v| !v.is_active())
            .or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| !v.is_held())
                    .min_by_key(|(_, v)| v.age())
                    .map(|(i, _)| i)
            })
            .or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.age())
                    .map(|(i, _)| i)
            });

        if let Some(i) = index {
            self.voices[i].trigger(note, velocity, age);
        }
    }

    pub fn note_off(&mut self, note: u8) {
        for voice in self.voices.iter_mut() {
            if voice.is_held() && voice.note == Some(note) {
                voice.release();
            }
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        for voice in &self.voices {
            voice.oscillator.set_volume(volume);
        }
    }

    pub fn set_attack(&mut self, attack: f32) {
        for voice in &self.voices {
            voice.envelope.set_attack(attack);
        }
    }

    pub fn set_decay(&mut self, decay: f32) {
        for voice in &self.voices {
            voice.envelope.set_decay(decay);
        }
    }

    pub fn set_sustain(&mut self, sustain: f32) {
        for voice in &self.voices {
            voice.envelope.set_sustain(sustain);
        }
    }

    pub fn set_release(&mut self, release: f32) {
        for voice in &self.voices {
            voice.envelope.set_release(release);
        }
    }

    pub fn set_filter_cutoff(&mut self, cutoff: f32) {
        for voice in &mut self.voices {
            voice.set_filter_cutoff(cutoff);
        }
    }

    pub fn set_filter_resonance(&mut self, resonance: f32) {
        for voice in &mut self.voices {
            voice.set_filter_resonance(resonance);
        }
    }

    pub fn set_filter_drive(&mut self, drive: f32) {
        for voice in &mut self.voices {
            voice.filter.set_drive(drive);
        }
    }

    pub fn set_filter_saturation(&mut self, saturation: f32) {
        for voice in &mut self.voices {
            voice.filter.set_saturation(saturation);
        }
    }

    pub fn render_next(&mut self) -> (f32, f32) {
        let mut mix = 0.0;
        for voice in &mut self.voices {
            if voice.is_active() {
                mix += voice.render_next();
            }
        }

        // Fixed headroom rather than per-sample renormalization by active-voice
        // count, which made held notes jump in volume as other notes came and went
        mix *= 0.35;

        // Reverb and chorus each handle their own dry/wet mix internally
        let (left, right) = self.reverb.process(mix, mix);
        let (left, right) = self.chorus.process(left, right);

        (soft_limit(left), soft_limit(right))
    }

    pub fn set_reverb_decay(&mut self, decay: f32) {
        self.reverb.set_decay(decay.clamp(0.0, 0.99));
    }

    pub fn set_reverb_wet(&mut self, wet: f32) {
        self.reverb.set_wet(wet.clamp(0.0, 1.0));
    }

    pub fn set_chorus_mode(&mut self, mode: ChorusMode) {
        self.chorus.set_mode(mode);
    }

    pub fn set_chorus_rate(&mut self, rate: f32) {
        self.chorus.set_rate(rate);
    }

    pub fn set_chorus_depth(&mut self, depth: f32) {
        self.chorus.set_depth(depth);
    }
}

fn soft_limit(x: f32) -> f32 {
    x.tanh()
}
