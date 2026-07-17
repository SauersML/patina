use crate::voice::Voice;
use crate::reverb::Reverb;
use crate::chorus::{Chorus, ChorusMode};
use crate::oscillator::Waveform;
use crate::tape::Tape;
use crate::fuzz::Fuzz;
use crate::noise::NoiseSource;
use crate::spring::SpringReverb;
use crate::lfo::Lfo;
use crate::substrate::{SlewLimiter, Substrate};
use std::collections::VecDeque;

/// Capacitive trace-to-trace coupling between adjacent voice cards. The
/// coupling differentiates (it is a capacitor), so the bleed is presence-
/// tilted; this coefficient puts it around -64 dB.
const CROSSTALK: f32 = 0.0008;

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
    pub hpf_cutoff: f32,
    pub fuzz: f32,
    pub noise: f32,
    pub spring: f32,
    pub glide: f32, // portamento time in seconds, 0 = off
    pub sub: f32,   // sub-oscillator level, 0..1
    pub pulse_width: f32,
    pub lfo_rate: f32,
    pub lfo_shape: f32,
    pub lfo_pitch: f32,  // vibrato depth in cents
    pub lfo_filter: f32, // cutoff modulation in octaves
    pub lfo_pwm: f32,    // pulse-width swing, 0..0.45
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
            hpf_cutoff: 16.0,
            fuzz: 0.0,
            noise: 0.0,
            spring: 0.0,
            glide: 0.0,
            sub: 0.0,
            pulse_width: 0.5,
            lfo_rate: 1.0,
            lfo_shape: 0.5,
            lfo_pitch: 0.0,
            lfo_filter: 0.0,
            lfo_pwm: 0.0,
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
    fuzz: Fuzz,
    noise_source: NoiseSource,
    noise_gain: f32, // smoothed
    spring: SpringReverb,
    lfo: Lfo,
    substrate: Substrate,
    prev_current: f32,
    slew_left: SlewLimiter,
    slew_right: SlewLimiter,
    sample_rate: f32,
    /// CV (octaves from A440) of the most recently triggered note — the
    /// "hold capacitor" that glide charges from (US 3,991,645).
    last_note_cv: Option<f32>,
    // Performance controllers (live playing state, not patch parameters):
    // pitch bend as a slewed frequency ratio, mod wheel adding vibrato
    // depth, and the sustain pedal with its held-note bookkeeping
    bend_target: f32,
    bend_ratio: f32,
    mod_wheel: f32,
    pedal_down: bool,
    sustained: [bool; 128],
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
            fuzz: Fuzz::new(),
            noise_source: NoiseSource::new(),
            noise_gain: 0.0,
            spring: SpringReverb::new(sample_rate),
            lfo: Lfo::new(sample_rate),
            substrate: Substrate::new(sample_rate),
            prev_current: 0.0,
            slew_left: SlewLimiter::new(sample_rate),
            slew_right: SlewLimiter::new(sample_rate),
            sample_rate,
            last_note_cv: None,
            bend_target: 1.0,
            bend_ratio: 1.0,
            mod_wheel: 0.0,
            pedal_down: false,
            sustained: [false; 128],
            note_counter: 0,
            params,
            scope: VecDeque::with_capacity(SCOPE_LEN),
            gain: params.volume,
            dc_left: DcBlocker::new(),
            dc_right: DcBlocker::new(),
        }
    }

    /// Skip the chassis warm-up (offline bounces record a warmed instrument).
    pub fn warm_up(&mut self) {
        self.substrate.force_warm();
    }

    /// Sample rate the engine runs at, for UI time-axis labeling.
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
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
        // A fresh press owns the note again; it is no longer the pedal's
        self.sustained[note as usize] = false;

        // Glide starts from the most recently played note, mono-synth style
        let glide_from = self.last_note_cv;
        self.last_note_cv = Some((note as f32 - 69.0) / 12.0);

        // Retrigger if this note is already held
        if let Some(voice) = self.voices.iter_mut().find(|v| v.is_held() && v.note == Some(note)) {
            voice.trigger(note, velocity, age, glide_from);
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
            self.voices[i].trigger(note, velocity, age, glide_from);
        }
    }

    pub fn note_off(&mut self, note: u8) {
        // While the sustain pedal is down, released keys keep ringing; the
        // release is deferred until the pedal lifts
        if self.pedal_down {
            if self.voices.iter().any(|v| v.is_held() && v.note == Some(note)) {
                self.sustained[note as usize] = true;
            }
            return;
        }
        for voice in self.voices.iter_mut() {
            if voice.is_held() && voice.note == Some(note) {
                voice.release();
            }
        }
    }

    /// Pitch bend in semitones (a wheel typically spans +/-2). Slewed in
    /// render_next so stepped MIDI bend values never zipper.
    pub fn set_pitch_bend(&mut self, semitones: f32) {
        self.bend_target = (semitones.clamp(-24.0, 24.0) / 12.0).exp2();
    }

    /// Mod wheel (CC1, 0..1): performance vibrato on top of the LFO>Pitch
    /// knob — full wheel adds 75 cents of swing.
    pub fn set_mod_wheel(&mut self, value: f32) {
        self.mod_wheel = value.clamp(0.0, 1.0);
    }

    /// Sustain pedal (CC64). On lift, every note released under the pedal
    /// is let go at once.
    pub fn set_sustain_pedal(&mut self, down: bool) {
        self.pedal_down = down;
        if !down {
            for voice in self.voices.iter_mut() {
                if let Some(note) = voice.note {
                    if voice.is_held() && self.sustained[note as usize] {
                        voice.release();
                    }
                }
            }
            self.sustained = [false; 128];
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

    pub fn set_hpf_cutoff(&mut self, cutoff: f32) {
        self.params.hpf_cutoff = cutoff.clamp(16.0, 8000.0);
        for voice in &mut self.voices {
            voice.hpf.set_cutoff(self.params.hpf_cutoff);
        }
    }

    pub fn set_fuzz(&mut self, amount: f32) {
        self.params.fuzz = amount.clamp(0.0, 1.0);
        self.fuzz.set_amount(self.params.fuzz);
    }

    pub fn set_noise(&mut self, level: f32) {
        self.params.noise = level.clamp(0.0, 1.0);
    }

    pub fn set_spring(&mut self, wet: f32) {
        self.params.spring = wet.clamp(0.0, 1.0);
        self.spring.set_wet(self.params.spring);
    }

    pub fn set_pulse_width(&mut self, width: f32) {
        self.params.pulse_width = width.clamp(0.05, 0.95);
    }

    pub fn set_sub(&mut self, level: f32) {
        self.params.sub = level.clamp(0.0, 1.0);
        for voice in &mut self.voices {
            voice.set_sub_level(self.params.sub);
        }
    }

    pub fn set_glide(&mut self, seconds: f32) {
        self.params.glide = seconds.clamp(0.0, 2.0);
        // RC coefficient: reach ~95% of the interval in `glide` seconds
        let k = if self.params.glide < 1e-3 {
            1.0
        } else {
            1.0 - (-3.0 / (self.params.glide * self.sample_rate)).exp()
        };
        for voice in &mut self.voices {
            voice.set_glide_coef(k);
        }
    }

    pub fn set_lfo_rate(&mut self, rate: f32) {
        self.params.lfo_rate = rate.clamp(0.1, 30.0);
        self.lfo.set_rate(self.params.lfo_rate);
    }

    pub fn set_lfo_shape(&mut self, shape: f32) {
        self.params.lfo_shape = shape.clamp(0.0, 1.0);
        self.lfo.set_shape(self.params.lfo_shape);
    }

    pub fn set_lfo_pitch(&mut self, cents: f32) {
        self.params.lfo_pitch = cents.clamp(0.0, 200.0);
    }

    pub fn set_lfo_filter(&mut self, octaves: f32) {
        self.params.lfo_filter = octaves.clamp(0.0, 4.0);
    }

    pub fn set_lfo_pwm(&mut self, depth: f32) {
        self.params.lfo_pwm = depth.clamp(0.0, 0.45);
    }

    pub fn render_next(&mut self) -> (f32, f32) {
        // One shared noise generator distributed to every active voice
        // (903A / Juno-106 architecture) — filtered per voice, gated by
        // each voice's envelope, but a single correlated source
        self.noise_gain += (self.params.noise - self.noise_gain) * 0.001;
        let noise = if self.noise_gain > 1e-4 {
            self.noise_source.next() * self.noise_gain * 0.8
        } else {
            0.0
        };

        // One global LFO drives every voice together — vibrato in CV space
        // (an exponential frequency ratio), filter in octaves, PWM on the
        // pulse comparator threshold
        let lfo = self.lfo.next();
        // Pitch bend slews (~ms scale) toward its target; mod wheel adds
        // performance vibrato on top of the patch's own LFO>Pitch depth
        self.bend_ratio += (self.bend_target - self.bend_ratio) * 0.002;
        let vibrato_cents = self.params.lfo_pitch + self.mod_wheel * 75.0;
        let pitch_mult = if vibrato_cents > 0.01 {
            (lfo * vibrato_cents / 1200.0).exp2() * self.bend_ratio
        } else {
            self.bend_ratio
        };
        let lfo_cutoff_oct = lfo * self.params.lfo_filter;
        let pulse_width = self.params.pulse_width + lfo * self.params.lfo_pwm;

        // The shared chassis: rail sag/ripple driven by last sample's summed
        // current draw, warm-up heat — read by every voice this sample
        let substrate = self.substrate.step(self.prev_current);

        // Each card's neighbor bleed (capacitive: the neighbor's
        // differentiated pre-filter node from last sample)
        let mut deltas = [0.0f32; 16];
        let n = self.voices.len().min(16);
        for (i, voice) in self.voices.iter().enumerate().take(16) {
            if voice.is_active() {
                deltas[i] = voice.prefilter_delta();
            }
        }

        let mut left = 0.0;
        let mut right = 0.0;
        for (i, voice) in self.voices.iter_mut().enumerate() {
            if voice.is_active() {
                let bleed = deltas[(i + n - 1) % n.max(1)] * CROSSTALK;
                let (l, r) = voice.render_next(
                    noise,
                    pitch_mult,
                    lfo_cutoff_oct,
                    pulse_width,
                    substrate,
                    bleed,
                );
                left += l;
                right += r;
            }
        }
        // What the supply just delivered — next sample's rail load
        self.prev_current = left.abs() + right.abs();

        // Smoothed master gain: fixed headroom, no zipper on volume automation
        // The summing amp sees the full multi-voice swing; its finite slew
        // rate shaves only the hottest, fastest edges (transient
        // intermodulation) — then the master gain scales the result
        left = self.slew_left.process(left);
        right = self.slew_right.process(right);
        self.gain += (self.params.volume - self.gain) * 0.0008;
        let g = self.gain * 0.7;
        left *= g;
        right *= g;

        // Fuzz first (a pedal in front of everything), then reverb and
        // chorus with their own internal dry/wet; tape sits last, as if the
        // whole mix were bounced to cassette
        let (left, right) = self.fuzz.process(left, right);
        let (left, right) = self.spring.process(left, right);
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

    /// US 3,991,645: glide lags the CV before the expo converter, so a new
    /// note starts at the previous note's pitch and settles exponentially.
    #[test]
    fn glide_swoops_from_previous_note() {
        let sr = 44100.0;
        // Schmitt-trigger crossing counter: hysteresis at 20% of the window
        // peak, so harmonic ripple near zero can't double-count cycles
        let count_crossings = |samples: &[f32]| -> usize {
            let peak = samples.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
            let th = peak * 0.2;
            let mut low = true;
            let mut count = 0;
            for &s in samples {
                if low && s > th {
                    count += 1;
                    low = false;
                } else if !low && s < -th {
                    low = true;
                }
            }
            count
        };
        let mut vm = VoiceManager::new(sr, 8);
        vm.set_glide(0.3);
        // Sine, zero detune, no reverb: exactly one rising crossing per cycle
        vm.set_waveform(Waveform::Sine);
        vm.set_detune(0.0);
        vm.set_attack(0.003);
        vm.set_sustain(1.0);
        vm.set_release(0.01);
        vm.set_reverb_wet(0.0);

        // Establish the "previous note", then let it fully die away so the
        // measurement hears only the gliding voice
        vm.note_on(57, 0.9);
        for _ in 0..22050 {
            vm.render_next();
        }
        vm.note_off(57);
        for _ in 0..8820 {
            vm.render_next();
        }
        vm.note_on(69, 0.9);

        let mut early = Vec::with_capacity(4410);
        let mut late = Vec::with_capacity(4410);
        for i in 0..(sr as usize) {
            let (l, _) = vm.render_next();
            if i < 4410 {
                early.push(l);
            } else if i >= 39690 {
                late.push(l);
            }
        }
        let early_f = count_crossings(&early);
        let late_f = count_crossings(&late);
        assert!(
            (early_f as f32) < late_f as f32 * 0.85,
            "glide should start near the old pitch: early={early_f}, late={late_f} crossings"
        );
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

    #[test]
    fn sustain_pedal_holds_released_notes() {
        let sr = 44100;
        let mut vm = VoiceManager::new(sr as f32, 8);
        vm.set_sustain_pedal(true);
        vm.note_on(60, 0.9);
        vm.note_off(60); // released under the pedal — must keep ringing
        for _ in 0..(sr / 2) {
            vm.render_next();
        }
        assert!(
            vm.voices.iter().any(|v| v.is_held()),
            "pedal down: released note should still be held"
        );

        vm.set_sustain_pedal(false);
        assert!(
            vm.voices.iter().all(|v| !v.is_held()),
            "pedal lift should release the sustained note"
        );

        // A key still physically down must survive the pedal lift
        vm.set_sustain_pedal(true);
        vm.note_on(64, 0.9);
        vm.set_sustain_pedal(false);
        assert!(
            vm.voices.iter().any(|v| v.is_held() && v.note == Some(64)),
            "pedal lift must not cut a key that is still down"
        );
    }

    #[test]
    fn pitch_bend_shifts_frequency() {
        let sr = 44100.0;
        // Zero-crossing rate as a crude frequency probe, FX bypassed
        let crossings_with_bend = |semitones: f32| {
            let mut vm = VoiceManager::new(sr, 8);
            vm.set_reverb_wet(0.0);
            vm.set_detune(0.0);
            vm.set_pitch_bend(semitones);
            vm.note_on(69, 1.0);
            let mut crossings = 0u32;
            let mut prev = 0.0f32;
            for i in 0..(sr as usize) {
                let (l, _) = vm.render_next();
                // Skip the bend slew and attack before counting
                if i > 20000 {
                    if prev <= 0.0 && l > 0.0 {
                        crossings += 1;
                    }
                    prev = l;
                }
            }
            crossings
        };
        let base = crossings_with_bend(0.0);
        let bent = crossings_with_bend(2.0);
        let ratio = bent as f32 / base as f32;
        // +2 semitones = x1.1225
        assert!(
            (1.06..1.19).contains(&ratio),
            "bend +2 st should raise pitch ~12%: base={base}, bent={bent}, ratio={ratio}"
        );
    }
}

