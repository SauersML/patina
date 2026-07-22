use crate::song::Param;
use crate::voice::Voice;
use crate::drums::{DrumMachine, DRUM_CHANNEL};
use crate::sampler::{slot_for_channel, SamplerBank, SamplerSlot};
use crate::vox::{Syllable, VoxBox, VOX_CHANNEL};
use crate::reverb::Reverb;
use crate::chorus::{Chorus, ChorusMode};
use crate::oscillator::{CircuitModel, Waveform, PROGRAM_V};
use crate::tape::Tape;
use crate::fuzz::Fuzz;
use crate::noise::NoiseSource;
use crate::spring::SpringReverb;
use crate::lfo::Lfo;
use crate::substrate::{SlewLimiter, Substrate};
use std::collections::{HashMap, VecDeque};

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
    // The three-oscillator voice: osc 1 is the reference; 2 and 3 have
    // their own waveform, interval (semitones), and mix level
    pub osc2_wave: Waveform,
    pub osc2_pitch: f32,
    pub osc2_level: f32,
    pub osc3_wave: Waveform,
    pub osc3_pitch: f32,
    pub osc3_level: f32,
    pub circuit: CircuitModel,
    pub key_track: f32,
    pub osc_fm: f32,
    pub sync: bool,
    pub ring: f32,
    pub pulse_width: f32,
    /// Oscillator 1's source mixer [saw, pulse, tri, sine]; all zero =
    /// classic waveform selector mode.
    pub mix_saw: f32,
    pub mix_pulse: f32,
    pub mix_tri: f32,
    pub mix_sine: f32,
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
    /// Keyboard register the UI should sit at; patches set it so a bass
    /// preset arrives with the keys already down where it lives.
    pub ui_octave: f32,
    pub chorus_rate: f32,
    pub chorus_depth: f32,
    pub tape_wow: f32,
    pub tape_flutter: f32,
    pub tape_drive: f32,
    pub tape_age: f32,
    // The rhythm section's panel (all 0..1 knob rotations)
    pub bd_level: f32,
    pub bd_tune: f32,
    pub bd_attack: f32,
    pub bd_decay: f32,
    pub bd_sweep: f32,
    pub bd_drive: f32,
    pub sd_level: f32,
    pub sd_tune: f32,
    pub sd_tone: f32,
    pub sd_snappy: f32,
    pub sd_decay: f32,
    pub rs_level: f32,
    pub rs_tune: f32,
    pub cp_level: f32,
    pub cp_decay: f32,
    pub hh_level: f32,
    pub hh_tune: f32,
    pub hh_metal: f32,
    pub ch_decay: f32,
    pub oh_decay: f32,
    pub dr_drive: f32,
    // The voice box's panel (bus-level, like the effects)
    pub vox_level: f32,
    pub vox_dry: f32,
    pub vox_breath: f32,
    pub vox_vibrato: f32,
    /// 0 = TalkBox voicing, 1 = full-range vocoder.
    pub vox_mode: f32,
    /// Talker circuit: 0 = '97 caricature voicing, 1 = legible.
    pub vox_clarity: f32,
    /// How much the voice performs its own pitch prosody (accents,
    /// declination, final falls). Low for singing, high for speech.
    pub vox_intonation: f32,
}

impl ParamValues {
    /// The song-channel base: a genuinely BASIC voice, unlike the live
    /// panel's musical default (which arrives with two extra sawtooths at
    /// 0.72, detune, and the saturation stage engaged). A patch file
    /// describes its sound against silence-plus-one-oscillator, so what
    /// you write is what you hear — nothing inherited "on it".
    pub fn neutral() -> Self {
        Self {
            osc2_level: 0.0,
            osc3_level: 0.0,
            detune: 0.0,
            sub: 0.0,
            key_track: 0.0,
            saturation: 0.0,
            drive: 1.0,
            cutoff: 18000.0,
            resonance: 0.0,
            filter_env_amount: 0.0,
            attack: 0.01,
            decay: 0.2,
            sustain: 0.8,
            release: 0.3,
            ..Self::default()
        }
    }
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
            osc2_wave: Waveform::Sawtooth,
            osc2_pitch: 0.0,
            osc2_level: 0.72,
            osc3_wave: Waveform::Sawtooth,
            osc3_pitch: 0.0,
            osc3_level: 0.72,
            circuit: CircuitModel::Moog,
            key_track: 0.4,
            osc_fm: 0.0,
            sync: false,
            ring: 0.0,
            pulse_width: 0.5,
            mix_saw: 0.0,
            mix_pulse: 0.0,
            mix_tri: 0.0,
            mix_sine: 0.0,
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
            ui_octave: 4.0,
            chorus_rate: 0.5,
            chorus_depth: 0.3,
            tape_wow: 0.0,
            tape_flutter: 0.0,
            tape_drive: 0.0,
            tape_age: 0.0,
            // 909 panel at rest: levels up, character knobs at the
            // factory-fresh center detents
            bd_level: 0.8,
            bd_tune: 0.35,
            bd_attack: 0.5,
            bd_decay: 0.45,
            bd_sweep: 0.5,
            bd_drive: 0.25,
            sd_level: 0.75,
            sd_tune: 0.4,
            sd_tone: 0.5,
            sd_snappy: 0.6,
            sd_decay: 0.5,
            rs_level: 0.7,
            rs_tune: 0.5,
            cp_level: 0.75,
            cp_decay: 0.5,
            hh_level: 0.7,
            hh_tune: 0.5,
            hh_metal: 0.65,
            ch_decay: 0.35,
            oh_decay: 0.5,
            dr_drive: 0.0,
            vox_level: 0.8,
            vox_dry: 0.0,
            vox_breath: 0.12,
            vox_vibrato: 0.25,
            vox_mode: 0.0,
            vox_clarity: 0.0,
            vox_intonation: 0.12,
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

/// One track's mixer strip. `gain`/`pan` sit between the voice and the
/// bus; the sends feed the spring, reverb and chorus tanks directly; and
/// `duck` is the sidechain — every kick trigger snaps the envelope to 1,
/// the strip's gain dips by `duck * env`, and `duck_release` sets how
/// fast it breathes back.
#[derive(Clone, Copy)]
pub struct ChannelMix {
    pub gain: f32,
    pub pan: f32,
    pub rev_send: f32,
    pub spr_send: f32,
    pub cho_send: f32,
    pub duck: f32,
    duck_decay: f32,
    cur_gain: f32,
    cur_pan: f32,
    duck_env: f32,
}

impl ChannelMix {
    fn new(sample_rate: f32) -> Self {
        ChannelMix {
            gain: 1.0,
            pan: 0.0,
            rev_send: 0.0,
            spr_send: 0.0,
            cho_send: 0.0,
            duck: 0.0,
            duck_decay: duck_decay_for(0.18, sample_rate),
            cur_gain: 1.0,
            cur_pan: 0.0,
            duck_env: 0.0,
        }
    }
}

fn duck_decay_for(release_secs: f32, sample_rate: f32) -> f32 {
    (-1.0 / (release_secs.max(0.01) * sample_rate)).exp()
}

/// Pass one channel's contribution through its mixer strip: ducked gain,
/// constant-center balance pan, and taps into the three send buses.
fn strip(
    mixes: &HashMap<u16, ChannelMix>,
    ch: u16,
    l: f32,
    r: f32,
    spr: &mut (f32, f32),
    rev: &mut (f32, f32),
    cho: &mut (f32, f32),
) -> (f32, f32) {
    let Some(m) = mixes.get(&ch) else { return (l, r) };
    let g = m.cur_gain * (1.0 - m.duck * m.duck_env);
    let (mut l, mut r) = (l * g, r * g);
    if m.cur_pan > 0.0 {
        l *= 1.0 - m.cur_pan;
    } else {
        r *= 1.0 + m.cur_pan;
    }
    spr.0 += l * m.spr_send;
    spr.1 += r * m.spr_send;
    rev.0 += l * m.rev_send;
    rev.1 += r * m.rev_send;
    cho.0 += l * m.cho_send;
    cho.1 += r * m.cho_send;
    (l, r)
}

pub struct VoiceManager {
    pub voices: Vec<Voice>,
    /// The rhythm section: one 909-style analog drum board sharing the
    /// chassis, the output bus, and the effects with the keyboard voices.
    pub drums: DrumMachine,
    /// The voice box: a formant speech synthesizer driving a channel
    /// vocoder whose carrier is the vox-channel synth voices.
    pub vox: VoxBox,
    /// The tape deck: sampler slots (`sample=` tracks) whose playback
    /// heads mix onto the same volt bus as everything else.
    pub sampler: SamplerBank,
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
    /// Sample clock, for the chord-detection window on glide.
    samples_rendered: u64,
    last_note_on_sample: u64,
    /// Per-song-channel parameter snapshots (the per-track patches).
    channel_params: HashMap<u16, ParamValues>,
    /// Per-track mixer strip: gain, pan, effect sends, sidechain duck.
    /// Channels absent from the map pass through untouched.
    channel_mix: HashMap<u16, ChannelMix>,
    /// Solo one channel (stem bounces): every other channel still
    /// renders — oscillators free-run — but never reaches the bus.
    solo: Option<u16>,
    pub params: ParamValues,
    pub scope: VecDeque<f32>,
    gain: f32, // smoothed master gain
    dc_left: DcBlocker,
    dc_right: DcBlocker,
}

impl VoiceManager {
    pub fn new(sample_rate: f32, num_voices: usize) -> Self {
        let params = ParamValues::default();
        // The vocoder's carrier patch: the voices a vox track's notes
        // play. A bright, steady saw stack — the vocoder needs harmonics
        // to sculpt and a sustain that doesn't sag mid-word. Vox tracks
        // can still automate any of it (`automate choir.cutoff`).
        let mut carrier = ParamValues::default();
        carrier.cutoff = 16000.0;
        carrier.attack = 0.008;
        carrier.sustain = 1.0;
        carrier.release = 0.12;
        carrier.key_track = 0.0;
        carrier.sub = 0.3;
        let mut channel_params = HashMap::new();
        channel_params.insert(VOX_CHANNEL, carrier);
        Self {
            voices: (0..num_voices)
                .map(|i| Voice::new(sample_rate, i, num_voices))
                .collect(),
            drums: DrumMachine::new(sample_rate),
            vox: VoxBox::new(sample_rate),
            sampler: SamplerBank::new(sample_rate),
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
            samples_rendered: 0,
            last_note_on_sample: u64::MAX,
            channel_params,
            channel_mix: HashMap::new(),
            solo: None,
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
        self.note_on_channel(note, velocity, 0);
    }

    /// Store a per-channel parameter snapshot; voices triggered on that
    /// channel are configured from it (the song engine's per-track patches).
    pub fn set_channel_params(&mut self, channel: u16, p: ParamValues) {
        self.channel_params.insert(channel, p);
    }

    /// Update one parameter on a channel and re-assert it on that
    /// channel's sounding voices — per-track automation. Bus-level
    /// parameters (effects, LFO, noise) fall through to the global path.
    /// Solo one channel for a stem bounce (None restores the full mix).
    pub fn set_solo(&mut self, channel: Option<u16>) {
        self.solo = channel;
    }

    pub fn set_track_mix(&mut self, channel: u16, param: crate::song::Param, value: f32) {
        use crate::song::Param as P;
        let sr = self.sample_rate;
        let m = self
            .channel_mix
            .entry(channel)
            .or_insert_with(|| ChannelMix::new(sr));
        // one clamp, from the table row of whichever strip control this is
        let value = param.clamp(value);
        match param {
            P::TrackGain => m.gain = value,
            P::TrackPan => m.pan = value,
            P::ReverbSend => m.rev_send = value,
            P::SpringSend => m.spr_send = value,
            P::ChorusSend => m.cho_send = value,
            P::DuckAmount => m.duck = value,
            P::DuckRelease => m.duck_decay = duck_decay_for(value, sr),
            _ => {}
        }
    }

    pub fn set_channel_param(&mut self, channel: u16, param: crate::song::Param, value: f32) {
        use crate::song::Param as P;
        if matches!(
            param,
            P::TrackGain | P::TrackPan | P::ReverbSend | P::SpringSend
                | P::ChorusSend | P::DuckAmount | P::DuckRelease
        ) {
            self.set_track_mix(channel, param, value);
            return;
        }
        if channel == 0 {
            param.apply(self, value);
            return;
        }
        // A sampler track's transport automation lands on its own slot
        if let Some(slot) = slot_for_channel(channel) {
            if self.sampler.set_param(slot, param, value) {
                return;
            }
        }
        let mut p = self.channel_params.get(&channel).copied().unwrap_or(self.params);
        if param.apply_to_params(&mut p, value) {
            self.channel_params.insert(channel, p);
            for voice in self.voices.iter_mut().filter(|v| v.channel() == channel) {
                voice.apply_params(&p);
            }
        } else {
            param.apply(self, value);
        }
    }

    pub fn note_on_channel(&mut self, note: u8, velocity: f32, channel: u16) {
        // The rhythm section is not a keyboard voice: a drum-channel note
        // is a trigger pulse onto the 909 board's trigger bus, velocity
        // riding the accent line
        if channel == DRUM_CHANNEL {
            // The kick is the sidechain source: every BD trigger snaps
            // the duck envelope of every strip that asked to be ducked
            if note == 35 || note == 36 {
                for m in self.channel_mix.values_mut() {
                    if m.duck > 0.0 {
                        m.duck_env = 1.0;
                    }
                }
            }
            self.drums.trigger_note(note, velocity);
            return;
        }
        // Sampler notes start playback heads on their slot's reel; they
        // never claim a keyboard voice
        if let Some(slot) = slot_for_channel(channel) {
            self.sampler.note_on(slot, note, velocity);
            return;
        }
        // A vox note both speaks (the modulator articulates a syllable)
        // and plays: it falls through to ordinary voice allocation, and
        // those voices become the vocoder's carrier
        if channel == VOX_CHANNEL {
            self.vox.note_on(note, velocity);
        }
        self.note_counter += 1;
        let age = self.note_counter;
        // A fresh press owns the note again; it is no longer the pedal's
        self.sustained[note as usize] = false;

        // Glide starts from the most recently played note, mono-synth
        // style — EXCEPT within a chord: notes struck together (within
        // ~25 ms) must not swoop in formation from one pitch, which was
        // the old glissando-comedy bug. Chord members start in tune.
        let chord_window = (self.sample_rate * 0.025) as u64;
        let is_chord = self
            .samples_rendered
            .checked_sub(self.last_note_on_sample)
            .map(|d| d < chord_window)
            .unwrap_or(false);
        let glide_from = if is_chord { None } else { self.last_note_cv };
        self.last_note_on_sample = self.samples_rendered;
        self.last_note_cv = Some((note as f32 - 69.0) / 12.0);

        let chan_params = if channel > 0 {
            self.channel_params.get(&channel).copied()
        } else {
            None
        };

        // Retrigger if this note is already held on this channel
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.is_held() && v.note == Some(note) && v.channel() == channel)
        {
            if let Some(p) = &chan_params {
                voice.apply_params(p);
            }
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
            self.voices[i].set_channel(channel);
            if let Some(p) = &chan_params {
                self.voices[i].apply_params(p);
            } else if channel == 0 {
                // Panel voices follow the live global params; a voice
                // previously claimed by a song channel comes home here
                let p = self.params;
                self.voices[i].apply_params(&p);
            }
            self.voices[i].trigger(note, velocity, age, glide_from);
        }
    }

    /// Release a note on one specific channel only.
    pub fn note_off_channel(&mut self, note: u8, channel: u16) {
        // Drum voices are one-shots fired by their trigger pulse; the
        // gate's falling edge does nothing on the hardware either
        if channel == DRUM_CHANNEL {
            return;
        }
        if let Some(slot) = slot_for_channel(channel) {
            self.sampler.note_off(slot, note);
            return;
        }
        // The voice hears the key lift (last one up speaks the coda);
        // the carrier voices release normally below
        if channel == VOX_CHANNEL {
            self.vox.note_off(note);
        }
        if self.pedal_down {
            if self
                .voices
                .iter()
                .any(|v| v.is_held() && v.note == Some(note) && v.channel() == channel)
            {
                self.sustained[note as usize] = true;
            }
            return;
        }
        for voice in self.voices.iter_mut() {
            if voice.is_held() && voice.note == Some(note) && voice.channel() == channel {
                voice.release();
            }
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
        self.mod_wheel = Param::ModWheel.clamp(value);
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
        self.params.volume = Param::Volume.clamp(volume);
    }

    pub fn set_waveform(&mut self, waveform: Waveform) {
        self.params.waveform = waveform;
        for voice in &mut self.voices {
            voice.set_waveform(waveform);
        }
    }

    pub fn set_detune(&mut self, cents: f32) {
        // engine-safety limit, deliberately wider than the table's 0..30
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
        self.params.filter_env_amount = Param::FilterEnvAmount.clamp(octaves);
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
        self.params.hpf_cutoff = Param::HpfCutoff.clamp(cutoff);
        for voice in &mut self.voices {
            voice.hpf.set_cutoff(self.params.hpf_cutoff);
        }
    }

    pub fn set_fuzz(&mut self, amount: f32) {
        self.params.fuzz = Param::FuzzAmount.clamp(amount);
        self.fuzz.set_amount(self.params.fuzz);
    }

    pub fn set_noise(&mut self, level: f32) {
        self.params.noise = Param::NoiseLevel.clamp(level);
    }

    pub fn set_spring(&mut self, wet: f32) {
        self.params.spring = Param::SpringWet.clamp(wet);
        self.spring.set_wet(self.params.spring);
    }

    pub fn set_osc1_mix_component(&mut self, which: usize, level: f32) {
        // the four converter levels share MixSaw's 0..1 table row
        let level = Param::MixSaw.clamp(level);
        match which {
            0 => self.params.mix_saw = level,
            1 => self.params.mix_pulse = level,
            2 => self.params.mix_tri = level,
            _ => self.params.mix_sine = level,
        }
        let mix = [
            self.params.mix_saw,
            self.params.mix_pulse,
            self.params.mix_tri,
            self.params.mix_sine,
        ];
        for voice in self.voices.iter_mut().filter(|v| v.channel() == 0) {
            voice.set_osc1_mix(mix);
        }
    }

    pub fn set_pulse_width(&mut self, width: f32) {
        self.params.pulse_width = Param::PulseWidth.clamp(width);
        for voice in self.voices.iter_mut().filter(|v| v.channel() == 0) {
            voice.set_pulse_width(self.params.pulse_width);
        }
    }

    pub fn set_sub(&mut self, level: f32) {
        self.params.sub = Param::SubLevel.clamp(level);
        for voice in &mut self.voices {
            voice.set_sub_level(self.params.sub);
        }
    }

    pub fn set_osc_wave(&mut self, which: usize, waveform: Waveform) {
        match which {
            1 => self.params.osc2_wave = waveform,
            2 => self.params.osc3_wave = waveform,
            _ => return,
        }
        for voice in &mut self.voices {
            voice.set_osc_waveform(which, waveform);
        }
    }

    pub fn set_osc_pitch(&mut self, which: usize, semitones: f32) {
        // oscillators 2 and 3 share Osc2Pitch's table row
        let semitones = Param::Osc2Pitch.clamp(semitones);
        match which {
            1 => self.params.osc2_pitch = semitones,
            2 => self.params.osc3_pitch = semitones,
            _ => return,
        }
        for voice in &mut self.voices {
            voice.set_osc_pitch(which, semitones);
        }
    }

    pub fn set_osc_level(&mut self, which: usize, level: f32) {
        let level = Param::Osc2Level.clamp(level);
        match which {
            1 => self.params.osc2_level = level,
            2 => self.params.osc3_level = level,
            _ => return,
        }
        for voice in &mut self.voices {
            voice.set_osc_level(which, level);
        }
    }

    pub fn set_circuit(&mut self, model: CircuitModel) {
        self.params.circuit = model;
        for voice in &mut self.voices {
            voice.set_circuit(model);
        }
    }

    pub fn set_key_track(&mut self, amount: f32) {
        self.params.key_track = Param::KeyTrack.clamp(amount);
        for voice in &mut self.voices {
            voice.set_key_track(self.params.key_track);
        }
    }

    pub fn set_osc_fm(&mut self, amount: f32) {
        self.params.osc_fm = Param::OscFm.clamp(amount);
        for voice in &mut self.voices {
            voice.set_fm_amount(self.params.osc_fm);
        }
    }

    pub fn set_ui_octave(&mut self, oct: f32) {
        self.params.ui_octave = Param::UiOctave.clamp(oct);
    }

    pub fn set_sync(&mut self, on: bool) {
        self.params.sync = on;
        for voice in &mut self.voices {
            voice.set_sync(on);
        }
    }

    pub fn set_ring(&mut self, amount: f32) {
        self.params.ring = Param::RingAmount.clamp(amount);
        for voice in &mut self.voices {
            voice.set_ring(self.params.ring);
        }
    }

    pub fn set_glide(&mut self, seconds: f32) {
        // Seconds per OCTAVE of travel: the SH-101/303-style linear CV
        // slew. A semitone snaps, a leap takes proportionally longer, and
        // the pitch arrives exactly (no RC tail).
        self.params.glide = seconds.clamp(0.0, 5.0);
        let rate = if self.params.glide < 1e-3 {
            1.0
        } else {
            1.0 / (self.params.glide * self.sample_rate)
        };
        for voice in &mut self.voices {
            voice.set_glide_rate(rate);
        }
    }

    pub fn set_lfo_rate(&mut self, rate: f32) {
        self.params.lfo_rate = Param::LfoRate.clamp(rate);
        self.lfo.set_rate(self.params.lfo_rate);
    }

    pub fn set_lfo_shape(&mut self, shape: f32) {
        self.params.lfo_shape = Param::LfoShape.clamp(shape);
        self.lfo.set_shape(self.params.lfo_shape);
    }

    pub fn set_lfo_pitch(&mut self, cents: f32) {
        self.params.lfo_pitch = Param::LfoPitch.clamp(cents);
    }

    pub fn set_lfo_filter(&mut self, octaves: f32) {
        self.params.lfo_filter = Param::LfoFilter.clamp(octaves);
    }

    pub fn set_lfo_pwm(&mut self, depth: f32) {
        self.params.lfo_pwm = Param::LfoPwm.clamp(depth);
    }

    // --- The voice box --------------------------------------------------

    /// Queue a syllable for the voice; the next vox note-on sings it.
    pub fn set_lyric(&mut self, channel: u16, syl: Syllable) {
        if channel == VOX_CHANNEL {
            self.vox.source.set_syllable(syl);
        }
    }

    /// Speak a syllable ahead of its note (the vowel-on-the-beat lead):
    /// the onset consonants start now; the vox note-on that follows
    /// pitches and holds the nucleus.
    pub fn vox_speak(&mut self, syl: &Syllable, note: u8, velocity: f32) {
        self.vox.speak(syl, note, velocity);
    }

    /// Load a recorded modulator (any voice) for the vocoder.
    pub fn set_vox_wav(&mut self, samples: &[f32], source_rate: u32) {
        self.vox.set_wav(samples, source_rate);
    }

    // --- The tape deck --------------------------------------------------

    /// Load a reel + transport into a sampler slot (song registration).
    pub fn set_sampler_slot(&mut self, index: usize, slot: SamplerSlot) {
        self.sampler.set_slot(index, slot);
    }

    /// Un-addressed (global / MIDI CC) sampler automation: every slot.
    pub fn set_sampler_all(&mut self, param: crate::song::Param, value: f32) {
        for i in 0..crate::sampler::MAX_SLOTS {
            self.sampler.set_param(i, param, value);
        }
    }

    pub fn set_vox_level(&mut self, v: f32) {
        self.params.vox_level = Param::VoxLevel.clamp(v);
        self.vox.set_level(self.params.vox_level);
    }

    pub fn set_vox_dry(&mut self, v: f32) {
        self.params.vox_dry = Param::VoxDry.clamp(v);
        self.vox.set_dry(self.params.vox_dry);
    }

    pub fn set_vox_clarity(&mut self, v: f32) {
        self.params.vox_clarity = Param::VoxClarity.clamp(v);
        self.vox.set_clarity(self.params.vox_clarity);
    }

    pub fn set_vox_pitch(&mut self, samples: &[f32], source_rate: u32) {
        self.vox.set_pitch_curve(samples, source_rate);
    }

    pub fn set_vox_breath(&mut self, v: f32) {
        self.params.vox_breath = Param::VoxBreath.clamp(v);
        self.vox.source.set_breath(self.params.vox_breath);
    }

    pub fn set_vox_vibrato(&mut self, v: f32) {
        self.params.vox_vibrato = Param::VoxVibrato.clamp(v);
        self.vox.source.set_vibrato(self.params.vox_vibrato);
    }

    pub fn set_vox_mode(&mut self, v: f32) {
        // 0 TalkBox / 1 studio vocoder / 2 Talker (LPC) / 3 spectral.
        // (A hand-written clamp here once stopped at 1.0, silently
        // rerouting circuits 2 and 3 — bounds now come from the table.)
        self.params.vox_mode = Param::VoxModeSel.clamp(v);
        self.vox.set_mode(crate::vocoder::VocoderMode::from_value(self.params.vox_mode));
    }

    pub fn set_vox_intonation(&mut self, v: f32) {
        self.params.vox_intonation = Param::VoxIntonation.clamp(v);
        self.vox.source.set_intonation(self.params.vox_intonation);
    }

    pub fn render_next(&mut self) -> (f32, f32) {
        self.samples_rendered = self.samples_rendered.wrapping_add(1);
        // One shared noise generator distributed to every active voice
        // (903A / Juno-106 architecture) — filtered per voice, gated by
        // each voice's envelope, but a single correlated source
        self.noise_gain += (self.params.noise - self.noise_gain) * 0.001;
        let noise = if self.noise_gain > 1e-4 {
            // ARP spec: noise is ~20 V p-p at full level, twice program
            self.noise_source.next() * self.noise_gain * 0.8 * PROGRAM_V
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
        // Each voice holds its own base width (per-channel patches); only
        // the shared LFO's swing travels from here
        let pw_offset = lfo * self.params.lfo_pwm;

        // The shared chassis: rail sag/ripple driven by last sample's summed
        // current draw, warm-up heat — read by every voice this sample
        let substrate = self.substrate.step(self.prev_current);

        // Each card's neighbor bleed (capacitive: the neighbor's
        // differentiated pre-filter node from last sample)
        let mut deltas = [0.0f32; 16];
        let n = self.voices.len().min(16);
        for (i, voice) in self.voices.iter().enumerate().take(16) {
            deltas[i] = voice.prefilter_delta();
        }

        // Every voice renders every sample, always: the oscillators
        // free-run from power-on and the VCAs only close to their -60 dB
        // floor, so a "silent" instrument is still faintly alive — like
        // the hardware, and unlike digital silence
        // Advance every mixer strip once per sample: smoothed gain and
        // pan (no zipper under automation), duck envelopes breathing back
        for m in self.channel_mix.values_mut() {
            m.cur_gain += (m.gain - m.cur_gain) * 0.001;
            m.cur_pan += (m.pan - m.cur_pan) * 0.001;
            m.duck_env *= m.duck_decay;
        }

        let mut left = 0.0;
        let mut right = 0.0;
        // Per-channel effect-send buses, accumulated in volts like the bus
        let mut send_spr = (0.0f32, 0.0f32);
        let mut send_rev = (0.0f32, 0.0f32);
        let mut send_cho = (0.0f32, 0.0f32);
        // Voices on the vox channel never reach the bus directly: they
        // are the vocoder's carrier, and only what the speech lets
        // through comes back
        let mut carrier = 0.0;
        // The performance line: when a vox pitch curve is playing, it IS
        // the carrier's pitch — portamento, scoops and vibrato included
        let vox_cv = self.vox.pitch_cv();
        for (i, voice) in self.voices.iter_mut().enumerate() {
            let bleed = deltas[(i + n - 1) % n.max(1)] * CROSSTALK;
            if voice.channel() == VOX_CHANNEL {
                voice.set_cv_override(vox_cv);
            }
            let (l, r) = voice.render_next(
                noise,
                pitch_mult,
                lfo_cutoff_oct,
                pw_offset,
                substrate,
                bleed,
            );
            let ch = voice.channel();
            if ch == VOX_CHANNEL {
                carrier += l + r;
            } else if self.solo.map_or(true, |s| s == ch) {
                let (l, r) = strip(
                    &self.channel_mix, ch, l, r,
                    &mut send_spr, &mut send_rev, &mut send_cho,
                );
                left += l;
                right += r;
            }
        }

        // The voice box: speech (formant voice or recorded wav) vocoding
        // the carrier, plus however much raw voice vox_dry lets out. Mono,
        // center — one mouth (with its own strip on the desk).
        let vox_out = self.vox.process(carrier);
        if self.solo.map_or(true, |s| s == VOX_CHANNEL) {
            let (vl, vr) = strip(
                &self.channel_mix, VOX_CHANNEL, vox_out, vox_out,
                &mut send_spr, &mut send_rev, &mut send_cho,
            );
            left += vl;
            right += vr;
        }

        // The rhythm section renders on the same bus, in the same volts.
        // It shares everything downstream — summing amp slew, fuzz,
        // reverbs, chorus, tape — and its current draw loads the same
        // rail, so a hard kick microscopically sags every oscillator:
        // the drum machine is IN the instrument, not beside it.
        let (dl, dr) = self.drums.render_next();
        if self.solo.map_or(true, |s| s == DRUM_CHANNEL) {
            let (dl, dr) = strip(
                &self.channel_mix, DRUM_CHANNEL, dl, dr,
                &mut send_spr, &mut send_rev, &mut send_cho,
            );
            left += dl;
            right += dr;
        }

        // The tape deck too: same bus, same volts, same rail load — and
        // its varispeed follows the shared bend/vibrato bus, so the pitch
        // wheel bends tape and oscillators together. One strip for the
        // whole deck (its slots keep their own smp_gain/smp_pan).
        // Every sample track gets a REAL strip: per-slot buckets, each
        // through its own channel's gain/pan/sends/duck.
        let mut slot_out = [(0.0f32, 0.0f32); crate::sampler::MAX_SLOTS];
        self.sampler.render_next_slots(pitch_mult, &mut slot_out);
        for (i, &(sl, sr)) in slot_out.iter().enumerate() {
            if sl == 0.0 && sr == 0.0 {
                continue;
            }
            let ch = crate::sampler::SAMPLER_CHANNEL_BASE + i as u16;
            if self.solo.map_or(true, |s| s == ch) {
                let (sl, sr) = strip(
                    &self.channel_mix, ch, sl, sr,
                    &mut send_spr, &mut send_rev, &mut send_cho,
                );
                left += sl;
                right += sr;
            }
        }

        // What the supply just delivered — next sample's rail load
        // (normalized back from volts so the substrate scale is unchanged)
        self.prev_current = (left.abs() + right.abs()) / PROGRAM_V;

        // Smoothed master gain: fixed headroom, no zipper on volume automation
        // The summing amp sees the full multi-voice swing IN VOLTS; its
        // finite slew rate shaves only the hottest, fastest edges
        // (transient intermodulation). Volts convert to sample units here,
        // once, at the bus — nowhere else.
        left = self.slew_left.process(left);
        right = self.slew_right.process(right);
        self.gain += (self.params.volume - self.gain) * 0.0008;
        let g = self.gain * 0.7 / PROGRAM_V;
        left *= g;
        right *= g;

        // Fuzz first (a pedal in front of everything), then reverb and
        // chorus with their own internal dry/wet — each fed its per-track
        // send bus at unity alongside the global knob; tape sits last, as
        // if the whole mix were bounced to cassette
        let (left, right) = self.fuzz.process(left, right);
        let (left, right) =
            self.spring
                .process_with_send(left, right, send_spr.0 * g, send_spr.1 * g);
        let (left, right) =
            self.reverb
                .process_with_send(left, right, send_rev.0 * g, send_rev.1 * g);
        let (left, right) =
            self.chorus
                .process_with_send(left, right, send_cho.0 * g, send_cho.1 * g);
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
        self.params.reverb_decay = Param::ReverbDecay.clamp(decay);
        self.reverb.set_decay(self.params.reverb_decay);
    }

    pub fn set_reverb_wet(&mut self, wet: f32) {
        self.params.reverb_wet = Param::ReverbWet.clamp(wet);
        self.reverb.set_wet(self.params.reverb_wet);
    }

    pub fn set_chorus_mix(&mut self, mix: f32) {
        self.chorus.set_mix(mix);
    }

    pub fn set_chorus_mode(&mut self, mode: ChorusMode) {
        self.params.chorus_mode = mode;
        self.chorus.set_mode(mode);
    }

    pub fn set_chorus_rate(&mut self, rate: f32) {
        self.params.chorus_rate = Param::ChorusRate.clamp(rate);
        self.chorus.set_rate(self.params.chorus_rate);
    }

    pub fn set_chorus_depth(&mut self, depth: f32) {
        self.params.chorus_depth = Param::ChorusDepth.clamp(depth);
        self.chorus.set_depth(self.params.chorus_depth);
    }

    pub fn set_tape_wow(&mut self, wow: f32) {
        self.params.tape_wow = Param::TapeWow.clamp(wow);
        self.tape.set_wow(self.params.tape_wow);
    }

    pub fn set_tape_flutter(&mut self, flutter: f32) {
        self.params.tape_flutter = Param::TapeFlutter.clamp(flutter);
        self.tape.set_flutter(self.params.tape_flutter);
    }

    pub fn set_tape_drive(&mut self, drive: f32) {
        self.params.tape_drive = Param::TapeDrive.clamp(drive);
        self.tape.set_drive(self.params.tape_drive);
    }

    pub fn set_tape_age(&mut self, age: f32) {
        self.params.tape_age = Param::TapeAge.clamp(age);
        self.tape.set_age(self.params.tape_age);
    }

    // --- The rhythm section's panel ---------------------------------------

    pub fn set_bd_level(&mut self, v: f32) {
        self.params.bd_level = Param::BdLevel.clamp(v);
        self.drums.set_bd_level(self.params.bd_level);
    }

    pub fn set_bd_tune(&mut self, v: f32) {
        self.params.bd_tune = Param::BdTune.clamp(v);
        self.drums.set_bd_tune(self.params.bd_tune);
    }

    pub fn set_bd_attack(&mut self, v: f32) {
        self.params.bd_attack = Param::BdAttack.clamp(v);
        self.drums.set_bd_attack(self.params.bd_attack);
    }

    pub fn set_bd_decay(&mut self, v: f32) {
        self.params.bd_decay = Param::BdDecay.clamp(v);
        self.drums.set_bd_decay(self.params.bd_decay);
    }

    pub fn set_bd_sweep(&mut self, v: f32) {
        self.params.bd_sweep = Param::BdSweep.clamp(v);
        self.drums.set_bd_sweep(self.params.bd_sweep);
    }

    pub fn set_bd_drive(&mut self, v: f32) {
        self.params.bd_drive = Param::BdDrive.clamp(v);
        self.drums.set_bd_drive(self.params.bd_drive);
    }

    pub fn set_sd_level(&mut self, v: f32) {
        self.params.sd_level = Param::SdLevel.clamp(v);
        self.drums.set_sd_level(self.params.sd_level);
    }

    pub fn set_sd_tune(&mut self, v: f32) {
        self.params.sd_tune = Param::SdTune.clamp(v);
        self.drums.set_sd_tune(self.params.sd_tune);
    }

    pub fn set_sd_tone(&mut self, v: f32) {
        self.params.sd_tone = Param::SdTone.clamp(v);
        self.drums.set_sd_tone(self.params.sd_tone);
    }

    pub fn set_sd_snappy(&mut self, v: f32) {
        self.params.sd_snappy = Param::SdSnappy.clamp(v);
        self.drums.set_sd_snappy(self.params.sd_snappy);
    }

    pub fn set_sd_decay(&mut self, v: f32) {
        self.params.sd_decay = Param::SdDecay.clamp(v);
        self.drums.set_sd_decay(self.params.sd_decay);
    }

    pub fn set_rs_level(&mut self, v: f32) {
        self.params.rs_level = Param::RsLevel.clamp(v);
        self.drums.set_rs_level(self.params.rs_level);
    }

    pub fn set_rs_tune(&mut self, v: f32) {
        self.params.rs_tune = Param::RsTune.clamp(v);
        self.drums.set_rs_tune(self.params.rs_tune);
    }

    pub fn set_cp_level(&mut self, v: f32) {
        self.params.cp_level = Param::CpLevel.clamp(v);
        self.drums.set_cp_level(self.params.cp_level);
    }

    pub fn set_cp_decay(&mut self, v: f32) {
        self.params.cp_decay = Param::CpDecay.clamp(v);
        self.drums.set_cp_decay(self.params.cp_decay);
    }

    pub fn set_hh_level(&mut self, v: f32) {
        self.params.hh_level = Param::HhLevel.clamp(v);
        self.drums.set_hh_level(self.params.hh_level);
    }

    pub fn set_hh_tune(&mut self, v: f32) {
        self.params.hh_tune = Param::HhTune.clamp(v);
        self.drums.set_hh_tune(self.params.hh_tune);
    }

    pub fn set_hh_metal(&mut self, v: f32) {
        self.params.hh_metal = Param::HhMetal.clamp(v);
        self.drums.set_hh_metal(self.params.hh_metal);
    }

    pub fn set_ch_decay(&mut self, v: f32) {
        self.params.ch_decay = Param::ChDecay.clamp(v);
        self.drums.set_ch_decay(self.params.ch_decay);
    }

    pub fn set_oh_decay(&mut self, v: f32) {
        self.params.oh_decay = Param::OhDecay.clamp(v);
        self.drums.set_oh_decay(self.params.oh_decay);
    }

    pub fn set_drum_drive(&mut self, v: f32) {
        self.params.dr_drive = Param::DrumDrive.clamp(v);
        self.drums.set_drive(self.params.dr_drive);
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

    /// Glide is a linear constant-rate slew now: it must ARRIVE exactly
    /// (no RC tail), a small interval must travel faster than a leap,
    /// and chord members struck together must NOT glide in formation.
    #[test]
    fn glide_slews_linearly_and_chords_stay_in_tune() {
        let sr = 44100.0;
        let mut vm = VoiceManager::new(sr, 8);
        vm.warm_up();
        vm.set_glide(0.2); // 0.2 s per octave
        vm.note_on(45, 0.8);
        for _ in 0..(0.1 * sr) as usize {
            vm.render_next();
        }
        // Legato leap of one octave: should be mid-glide at 0.1 s and
        // ARRIVED (exactly zero) shortly after 0.2 s
        vm.note_on(57, 0.8);
        for _ in 0..(0.1 * sr) as usize {
            vm.render_next();
        }
        let v57 = vm.voices.iter().find(|v| v.note == Some(57)).unwrap();
        let mid = v57.glide_remaining().abs();
        assert!(
            mid > 0.3 && mid < 0.7,
            "octave glide at halfway should have ~half remaining, got {mid}"
        );
        for _ in 0..(0.15 * sr) as usize {
            vm.render_next();
        }
        let v57 = vm.voices.iter().find(|v| v.note == Some(57)).unwrap();
        assert_eq!(
            v57.glide_remaining(),
            0.0,
            "linear glide must arrive EXACTLY, no asymptotic tail"
        );
        // A chord struck together must not swoop from the last note
        for _ in 0..(0.2 * sr) as usize {
            vm.render_next();
        }
        vm.note_on(60, 0.8);
        vm.note_on(64, 0.8);
        vm.note_on(67, 0.8);
        for &n in &[64u8, 67u8] {
            let v = vm.voices.iter().find(|v| v.note == Some(n)).unwrap();
            assert_eq!(
                v.glide_remaining(),
                0.0,
                "chord member {n} must start in tune, not glide in formation"
            );
        }
        // ...but the chord's FIRST note may glide from the previous line
        let v60 = vm.voices.iter().find(|v| v.note == Some(60)).unwrap();
        assert!(v60.glide_remaining().abs() > 0.0);
    }

    /// Channel isolation: a dark channel patch and the bright panel must
    /// coexist — the channel's voice keeps its own filter while panel
    /// voices follow the global params.
    #[test]
    fn channel_patches_isolate_voices() {
        let sr = 44100.0;
        let brightness_of = |use_channel: bool| -> f32 {
            let mut vm = VoiceManager::new(sr, 8);
            vm.warm_up();
            vm.set_waveform(Waveform::Sawtooth);
            vm.set_filter_cutoff(12000.0);
            vm.set_reverb_wet(0.0);
            vm.set_attack(0.005);
            vm.set_sustain(1.0);
            let mut dark = vm.params;
            dark.cutoff = 300.0;
            vm.set_channel_params(1, dark);
            if use_channel {
                vm.note_on_channel(69, 0.9, 1);
            } else {
                vm.note_on_channel(69, 0.9, 0);
            }
            let n = sr as usize / 2;
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                out.push(vm.render_next().0);
            }
            let goertzel = |freq: f32| -> f32 {
                let (mut re, mut im) = (0.0f32, 0.0f32);
                for (i, &s) in out[n / 2..].iter().enumerate() {
                    let a = std::f32::consts::TAU * freq * i as f32 / sr;
                    re += s * a.cos();
                    im += s * a.sin();
                }
                (re * re + im * im).sqrt()
            };
            // 7th harmonic of A440: passes a 12 kHz filter, dies at 300 Hz
            goertzel(7.0 * 440.0) / goertzel(440.0).max(1e-9)
        };
        let panel = brightness_of(false);
        let channel = brightness_of(true);
        assert!(
            channel < 0.25 * panel,
            "channel-1 voice should be far darker than the panel voice: channel {channel:.4}, panel {panel:.4}"
        );
    }

    /// The ring modulator multiplies the two oscillators: sine carriers at
    /// f1 and f2 must yield sum and difference tones with the carriers
    /// suppressed (ARP: nulls trimmed below 10 mV of a 5 V program level).
    #[test]
    fn ring_produces_sidebands_and_suppresses_carriers() {
        let sr = 44100.0;
        let mut vm = VoiceManager::new(sr, 8);
        vm.warm_up();
        vm.set_waveform(Waveform::Sine);
        vm.set_detune(0.0);
        vm.set_ring(1.0);
        vm.set_osc_pitch(1, 7.02); // ~perfect fifth: f2 = 1.5 f1
        vm.set_attack(0.005);
        vm.set_sustain(1.0);
        vm.set_reverb_wet(0.0);
        vm.note_on(69, 0.9); // f1 = 440, f2 ~= 660
        let n = sr as usize;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(vm.render_next().0);
        }
        let goertzel = |freq: f32| -> f32 {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, &s) in out[n / 2..].iter().enumerate() {
                let a = std::f32::consts::TAU * freq * i as f32 / sr;
                re += s * a.cos();
                im += s * a.sin();
            }
            (re * re + im * im).sqrt()
        };
        let sum_tone = goertzel(440.0 + 660.5);
        let diff_tone = goertzel(660.5 - 440.0);
        let carrier1 = goertzel(440.0);
        let sidebands = sum_tone.max(diff_tone);
        assert!(
            sidebands > 3.0 * carrier1,
            "ring should make sidebands dominate the carrier: sum={sum_tone:.1}, \
             diff={diff_tone:.1}, carrier={carrier1:.1}"
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

