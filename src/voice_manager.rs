use crate::voice::Voice;
use crate::reverb::Reverb;
use crate::chorus::{Chorus, ChorusMode};
use crate::oscillator::Waveform;
use crate::tape::Tape;
use std::collections::VecDeque;

/// Samples kept for the UI oscilloscope display.
const SCOPE_LEN: usize = 2048;

/// Canonical values of every automatable parameter, updated by the setters
/// below. The UI reads this each frame so sliders follow song automation,
/// and song automation and slider moves stay in sync.
#[derive(Clone, Copy)]
pub struct ParamValues {
    pub volume: f32,
    pub waveform: Waveform,
    pub detune: f32,
    pub cutoff: f32,
    pub resonance: f32,
    pub drive: f32,
    pub saturation: f32,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub filter_env_amount: f32,
    pub filter_attack: f32,
    pub filter_decay: f32,
    pub filter_sustain: f32,
    pub filter_release: f32,
    pub reverb_decay: f32,
    pub reverb_wet: f32,
    pub chorus_mode: ChorusMode,
    pub chorus_rate: f32,
    pub chorus_depth: f32,
    pub tape_wow: f32,
    pub tape_flutter: f32,
    pub tape_drive: f32,
    pub tape_age: f32,
}

impl Default for ParamValues {
    fn default() -> Self {
        Self {
            volume: 0.5,
            waveform: Waveform::Sawtooth,
            detune: 7.0,
            cutoff: 15000.0,
            resonance: 0.0,
            drive: 1.0,
            saturation: 1.0,
            attack: 0.1,
            decay: 0.1,
            sustain: 0.7,
            release: 0.2,
            filter_env_amount: 0.0,
            filter_attack: 0.005,
            filter_decay: 0.3,
            filter_sustain: 0.0,
            filter_release: 0.3,
            reverb_decay: 0.5,
            reverb_wet: 0.5,
            chorus_mode: ChorusMode::Off,
            chorus_rate: 0.5,
            chorus_depth: 0.3,
            tape_wow: 0.0,
            tape_flutter: 0.0,
            tape_drive: 0.0,
            tape_age: 0.0,
        }
    }
}

/// One-pole high-pass at ~10 Hz that strips DC offset from the output bus.
struct DcBlocker {
    x1: f32,
    y1: f32,
}

impl DcBlocker {
    fn new() -> Self {
        Self { x1: 0.0, y1: 0.0 }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + 0.9955 * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

pub struct VoiceManager {
    pub voices: Vec<Voice>,
    reverb: Reverb,
    chorus: Chorus,
    tape: Tape,
    note_counter: u64,
    pub params: ParamValues,
    pub scope: VecDeque<f32>,
    gain: f32, // smoothed master gain
    dc_left: DcBlocker,
    dc_right: DcBlocker,
}

impl VoiceManager {
    pub fn new(sample_rate: f32, num_voices: usize) -> Self {
        let params = ParamValues::default();
        Self {
            voices: (0..num_voices)
                .map(|i| Voice::new(sample_rate, i, num_voices))
                .collect(),
            reverb: Reverb::new(sample_rate),
            chorus: Chorus::new(sample_rate),
            tape: Tape::new(sample_rate),
            note_counter: 0,
            params,
            scope: VecDeque::with_capacity(SCOPE_LEN),
            gain: params.volume,
            dc_left: DcBlocker::new(),
            dc_right: DcBlocker::new(),
        }
    }

    /// Marks which MIDI notes are currently held, for the UI keyboard display.
    pub fn held_note_states(&self) -> [bool; 128] {
        let mut states = [false; 128];
        for voice in &self.voices {
            if voice.is_held() {
                if let Some(note) = voice.note {
                    states[note as usize] = true;
                }
            }
        }
        states
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
        // Applied as a smoothed master gain in render_next
        self.params.volume = volume.clamp(0.0, 1.0);
    }

    pub fn set_waveform(&mut self, waveform: Waveform) {
        self.params.waveform = waveform;
        for voice in &mut self.voices {
            voice.set_waveform(waveform);
        }
    }

    pub fn set_detune(&mut self, cents: f32) {
        self.params.detune = cents.clamp(0.0, 50.0);
        for voice in &mut self.voices {
            voice.set_detune(self.params.detune);
        }
    }

    pub fn set_attack(&mut self, attack: f32) {
        self.params.attack = attack;
        for voice in &self.voices {
            voice.envelope.set_attack(attack);
        }
    }

    pub fn set_decay(&mut self, decay: f32) {
        self.params.decay = decay;
        for voice in &self.voices {
            voice.envelope.set_decay(decay);
        }
    }

    pub fn set_sustain(&mut self, sustain: f32) {
        self.params.sustain = sustain;
        for voice in &self.voices {
            voice.envelope.set_sustain(sustain);
        }
    }

    pub fn set_release(&mut self, release: f32) {
        self.params.release = release;
        for voice in &self.voices {
            voice.envelope.set_release(release);
        }
    }

    pub fn set_filter_env_amount(&mut self, octaves: f32) {
        self.params.filter_env_amount = octaves.clamp(-5.0, 5.0);
        for voice in &mut self.voices {
            voice.set_filter_env_amount(self.params.filter_env_amount);
        }
    }

    pub fn set_filter_attack(&mut self, attack: f32) {
        self.params.filter_attack = attack;
        for voice in &self.voices {
            voice.filter_env.set_attack(attack);
        }
    }

    pub fn set_filter_decay(&mut self, decay: f32) {
        self.params.filter_decay = decay;
        for voice in &self.voices {
            voice.filter_env.set_decay(decay);
        }
    }

    pub fn set_filter_sustain(&mut self, sustain: f32) {
        self.params.filter_sustain = sustain;
        for voice in &self.voices {
            voice.filter_env.set_sustain(sustain);
        }
    }

    pub fn set_filter_release(&mut self, release: f32) {
        self.params.filter_release = release;
        for voice in &self.voices {
            voice.filter_env.set_release(release);
        }
    }

    pub fn set_filter_cutoff(&mut self, cutoff: f32) {
        self.params.cutoff = cutoff;
        for voice in &mut self.voices {
            voice.set_filter_cutoff(cutoff);
        }
    }

    pub fn set_filter_resonance(&mut self, resonance: f32) {
        self.params.resonance = resonance;
        for voice in &mut self.voices {
            voice.set_filter_resonance(resonance);
        }
    }

    pub fn set_filter_drive(&mut self, drive: f32) {
        self.params.drive = drive;
        for voice in &mut self.voices {
            voice.filter.set_drive(drive);
        }
    }

    pub fn set_filter_saturation(&mut self, saturation: f32) {
        self.params.saturation = saturation;
        for voice in &mut self.voices {
            voice.filter.set_saturation(saturation);
        }
    }

    pub fn render_next(&mut self) -> (f32, f32) {
        let mut left = 0.0;
        let mut right = 0.0;
        for voice in &mut self.voices {
            if voice.is_active() {
                let (l, r) = voice.render_next();
                left += l;
                right += r;
            }
        }

        // Smoothed master gain: fixed headroom, no zipper on volume automation
        self.gain += (self.params.volume - self.gain) * 0.0008;
        let g = self.gain * 0.7;
        left *= g;
        right *= g;

        // Reverb and chorus each handle their own dry/wet mix internally;
        // tape sits last, as if the whole mix were bounced to cassette
        let (left, right) = self.reverb.process(left, right);
        let (left, right) = self.chorus.process(left, right);
        let (left, right) = self.tape.process(left, right);

        let left = soft_limit(self.dc_left.process(left));
        let right = soft_limit(self.dc_right.process(right));

        if self.scope.len() >= SCOPE_LEN {
            self.scope.pop_front();
        }
        self.scope.push_back((left + right) * 0.5);

        (left, right)
    }

    pub fn set_reverb_decay(&mut self, decay: f32) {
        self.params.reverb_decay = decay.clamp(0.0, 0.99);
        self.reverb.set_decay(self.params.reverb_decay);
    }

    pub fn set_reverb_wet(&mut self, wet: f32) {
        self.params.reverb_wet = wet.clamp(0.0, 1.0);
        self.reverb.set_wet(self.params.reverb_wet);
    }

    pub fn set_chorus_mode(&mut self, mode: ChorusMode) {
        self.params.chorus_mode = mode;
        self.chorus.set_mode(mode);
    }

    pub fn set_chorus_rate(&mut self, rate: f32) {
        self.params.chorus_rate = rate.clamp(0.1, 10.0);
        self.chorus.set_rate(self.params.chorus_rate);
    }

    pub fn set_chorus_depth(&mut self, depth: f32) {
        self.params.chorus_depth = depth.clamp(0.0, 1.0);
        self.chorus.set_depth(self.params.chorus_depth);
    }

    pub fn set_tape_wow(&mut self, wow: f32) {
        self.params.tape_wow = wow.clamp(0.0, 1.0);
        self.tape.set_wow(self.params.tape_wow);
    }

    pub fn set_tape_flutter(&mut self, flutter: f32) {
        self.params.tape_flutter = flutter.clamp(0.0, 1.0);
        self.tape.set_flutter(self.params.tape_flutter);
    }

    pub fn set_tape_drive(&mut self, drive: f32) {
        self.params.tape_drive = drive.clamp(0.0, 1.0);
        self.tape.set_drive(self.params.tape_drive);
    }

    pub fn set_tape_age(&mut self, age: f32) {
        self.params.tape_age = age.clamp(0.0, 1.0);
        self.tape.set_age(self.params.tape_age);
    }
}

fn soft_limit(x: f32) -> f32 {
    x.tanh()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_sound_and_decays() {
        let sr = 44100;
        let mut vm = VoiceManager::new(sr as f32, 8);
        vm.set_filter_env_amount(2.0);
        vm.note_on(57, 0.9);
        vm.note_on(64, 0.8);

        let mut peak: f32 = 0.0;
        for _ in 0..sr {
            let (l, r) = vm.render_next();
            assert!(l.is_finite() && r.is_finite(), "non-finite sample");
            assert!(l.abs() <= 1.0 && r.abs() <= 1.0, "sample out of range");
            peak = peak.max(l.abs()).max(r.abs());
        }
        assert!(peak > 0.05, "held notes should be audible, peak={}", peak);

        vm.note_off(57);
        vm.note_off(64);
        // Render 4 s of tail; the last quarter second should be near-silent
        let mut tail: f32 = 0.0;
        for i in 0..(4 * sr) {
            let (l, r) = vm.render_next();
            if i >= 4 * sr - sr / 4 {
                tail = tail.max(l.abs()).max(r.abs());
            }
        }
        assert!(tail < 0.02, "output should decay after release, tail={}", tail);
    }

    #[test]
    fn cutoff_modulation_brightens() {
        // With a big positive filter-env amount the note's spectrum should
        // open up: compare short-window energy of high-passed signal right at
        // the attack (envelope peak) vs later (envelope decayed).
        let sr = 44100.0;
        let mut vm = VoiceManager::new(sr, 8);
        vm.set_attack(0.003);
        vm.set_sustain(1.0);
        vm.set_filter_cutoff(200.0);
        vm.set_filter_env_amount(4.0);
        vm.set_filter_decay(0.1);
        vm.set_filter_sustain(0.0);
        vm.note_on(45, 1.0);

        // crude one-pole highpass probe; compare HF share of total energy so
        // overall amplitude differences cancel out
        let mut hp_state = 0.0f32;
        let mut probe = |x: f32| {
            hp_state += 0.3 * (x - hp_state);
            (x - hp_state).abs()
        };

        let (mut early_hf, mut early_total) = (0.0f32, 1e-9f32);
        let (mut late_hf, mut late_total) = (0.0f32, 1e-9f32);
        for i in 0..(sr as usize) {
            let (l, _) = vm.render_next();
            let hf = probe(l);
            if i > 200 && i < 2200 {
                early_hf += hf;
                early_total += l.abs();
            } else if i > 30000 && i < 32000 {
                late_hf += hf;
                late_total += l.abs();
            }
        }
        let early_ratio = early_hf / early_total;
        let late_ratio = late_hf / late_total;
        assert!(
            early_ratio > late_ratio * 1.3,
            "attack should be brighter than decayed sustain: early={early_ratio}, late={late_ratio}"
        );
    }
}

#[cfg(test)]
mod noise_diag {
    use super::*;

    struct HighBand {
        stages: [(f32, f32); 4], // (prev_in, prev_out)
        alpha: f32,
    }
    impl HighBand {
        fn new(fs: f32, fc: f32) -> Self {
            let rc = 1.0 / (2.0 * std::f32::consts::PI * fc);
            let dt = 1.0 / fs;
            Self { stages: [(0.0, 0.0); 4], alpha: rc / (rc + dt) }
        }
        fn process(&mut self, mut x: f32) -> f32 {
            for s in &mut self.stages {
                let y = self.alpha * (s.1 + x - s.0);
                s.0 = x;
                s.1 = y;
                x = y;
            }
            x
        }
    }

    fn measure(vm: &mut VoiceManager, seconds: f32) -> (f32, f32) {
        let fs = 48000.0;
        let n = (fs * seconds) as usize;
        let mut hp = HighBand::new(fs, 9000.0);
        let (mut total, mut high) = (0.0f64, 0.0f64);
        for _ in 0..n {
            let (l, _) = vm.render_next();
            let h = hp.process(l);
            total += (l * l) as f64;
            high += (h * h) as f64;
        }
        (
            ((total / n as f64).sqrt()) as f32,
            ((high / n as f64).sqrt()) as f32,
        )
    }

    #[test]
    fn noise_floor_by_stage() {
        let fs = 48000.0;
        let song = |vm: &mut VoiceManager| {
            vm.set_filter_cutoff(900.0);
            vm.set_volume(0.5);
        };

        // Stage 0: idle engine, nothing playing, all effects neutral
        let mut vm = VoiceManager::new(fs, 8);
        vm.set_reverb_wet(0.0);
        song(&mut vm);
        let (rms, hf) = measure(&mut vm, 1.0);
        println!("idle, no effects:            rms {:.6}  hf {:.6}", rms, hf);

        // Stage 1: held note, all effects neutral
        let mut vm = VoiceManager::new(fs, 8);
        vm.set_reverb_wet(0.0);
        song(&mut vm);
        vm.note_on(45, 0.7);
        measure(&mut vm, 0.5); // settle attack
        let (rms, hf) = measure(&mut vm, 1.0);
        println!("note, no effects:            rms {:.6}  hf {:.6}", rms, hf);

        // Stage 2: + reverb at song level
        let mut vm = VoiceManager::new(fs, 8);
        song(&mut vm);
        vm.set_reverb_wet(0.55);
        vm.note_on(45, 0.7);
        measure(&mut vm, 0.5);
        let (rms, hf) = measure(&mut vm, 1.0);
        println!("note + reverb:               rms {:.6}  hf {:.6}", rms, hf);

        // Stage 3: + tape at song-start settings
        let mut vm = VoiceManager::new(fs, 8);
        song(&mut vm);
        vm.set_reverb_wet(0.55);
        vm.set_tape_wow(0.3);
        vm.set_tape_flutter(0.15);
        vm.set_tape_drive(0.35);
        vm.set_tape_age(0.2);
        vm.note_on(45, 0.7);
        measure(&mut vm, 0.5);
        let (rms, hf) = measure(&mut vm, 1.0);
        println!("note + reverb + tape(start): rms {:.6}  hf {:.6}", rms, hf);

        // Stage 4: tape at song-end settings
        let mut vm = VoiceManager::new(fs, 8);
        song(&mut vm);
        vm.set_reverb_wet(0.7);
        vm.set_tape_wow(0.75);
        vm.set_tape_flutter(0.35);
        vm.set_tape_drive(0.7);
        vm.set_tape_age(0.85);
        vm.note_on(45, 0.7);
        measure(&mut vm, 0.5);
        let (rms, hf) = measure(&mut vm, 1.0);
        println!("note + reverb + tape(end):   rms {:.6}  hf {:.6}", rms, hf);

        // Stage 5: idle engine but tape engaged (hiss only)
        let mut vm = VoiceManager::new(fs, 8);
        song(&mut vm);
        vm.set_reverb_wet(0.0);
        vm.set_tape_age(0.85);
        vm.set_tape_drive(0.7);
        let (rms, hf) = measure(&mut vm, 1.0);
        println!("idle + tape(end):            rms {:.6}  hf {:.6}", rms, hf);

        // Stage 6: idle, tape idle, reverb on (does reverb self-noise?)
        let mut vm = VoiceManager::new(fs, 8);
        song(&mut vm);
        vm.set_reverb_wet(0.7);
        let (rms, hf) = measure(&mut vm, 1.0);
        println!("idle + reverb only:          rms {:.6}  hf {:.6}", rms, hf);
    }
}
