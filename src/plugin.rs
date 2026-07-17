// CLAP / VST3 plugin front end (nice-plug). The host owns audio, MIDI, and
// parameter automation; everything else is the same VoiceManager the
// standalone app drives.
//
// SINGLE SOURCE OF TRUTH: every float parameter the plugin exposes is one
// line in `float_specs()` — id, display name, range, default, formatting,
// and the VoiceManager setter it drives. The host parameter list, default
// values, state save/restore, and per-block application are all derived
// from that table. Adding an engine knob = one engine setter + one line
// here. (The two selector params, waveform and chorus mode, are typed
// enums and live alongside the table.)
//
// Bundle with:
//   cargo xtask bundle patina --release --no-default-features --features plugin

use nice_plug::prelude::*;
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::chorus::ChorusMode;
use crate::oscillator::Waveform;
use crate::voice_manager::VoiceManager;

const NUM_VOICES: usize = 8;
/// Standard pitch-wheel range, in semitones each direction.
const PITCH_BEND_SEMITONES: f32 = 2.0;

// ---------------------------------------------------------------------------
// The parameter table
// ---------------------------------------------------------------------------

/// One host-facing float parameter: its identity, its control, and the
/// engine setter it drives.
struct FloatSpec {
    id: &'static str,
    param: FloatParam,
    apply: fn(&mut VoiceManager, f32),
    /// Guarded setters are only called when the value changes — they swap
    /// voice banks, re-randomize offsets, or re-run self-calibration.
    guarded: bool,
}

fn spec(
    id: &'static str,
    param: FloatParam,
    apply: fn(&mut VoiceManager, f32),
) -> FloatSpec {
    FloatSpec { id, param, apply, guarded: false }
}

fn guarded(
    id: &'static str,
    param: FloatParam,
    apply: fn(&mut VoiceManager, f32),
) -> FloatSpec {
    FloatSpec { id, param, apply, guarded: true }
}

fn pct(name: &'static str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
        .with_unit(" %")
}

fn seconds(name: &'static str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-2.0) },
    )
    .with_unit(" s")
    .with_value_to_string(formatters::v2s_f32_rounded(3))
}

fn hz(name: &'static str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-2.0) },
    )
    .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
    .with_string_to_value(formatters::s2v_f32_hz_then_khz())
}

fn plain(name: &'static str, default: f32, min: f32, max: f32, unit: &'static str) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min, max })
        .with_unit(unit)
        .with_value_to_string(formatters::v2s_f32_rounded(2))
}

/// THE table. Order here is the order hosts display parameters in.
/// Ranges and defaults mirror the engine's own clamps and ParamValues.
#[rustfmt::skip]
fn float_specs() -> Vec<FloatSpec> {
    vec![
        // Oscillator
        spec("volume",   pct("Volume", 0.5),                                    |vm, v| vm.set_volume(v)),
        spec("detune",   plain("Detune", 7.0, 0.0, 30.0, " ct"),                |vm, v| vm.set_detune(v)),
        spec("pw",       pct("Pulse Width", 0.5).with_unit(""),                 |vm, v| vm.set_pulse_width(v)),
        spec("noise",    pct("Noise", 0.0),                                     |vm, v| vm.set_noise(v)),

        // LFO
        spec("lforate",  hz("LFO Rate", 1.0, 0.1, 30.0),                        |vm, v| vm.set_lfo_rate(v)),
        spec("lfoshape", pct("LFO Shape", 0.5),                                 |vm, v| vm.set_lfo_shape(v)),
        spec("lfopitch", plain("LFO > Pitch", 0.0, 0.0, 200.0, " ct"),          |vm, v| vm.set_lfo_pitch(v)),
        spec("lfofilt",  plain("LFO > Filter", 0.0, 0.0, 4.0, " oct"),          |vm, v| vm.set_lfo_filter(v)),
        spec("lfopwm",   plain("LFO > PWM", 0.0, 0.0, 0.45, ""),                |vm, v| vm.set_lfo_pwm(v)),

        // Amplitude envelope
        spec("attack",   seconds("Attack", 0.1, 0.01, 2.0),                     |vm, v| vm.set_attack(v)),
        spec("decay",    seconds("Decay", 0.1, 0.01, 2.0),                      |vm, v| vm.set_decay(v)),
        spec("sustain",  pct("Sustain", 0.7),                                   |vm, v| vm.set_sustain(v)),
        spec("release",  seconds("Release", 0.2, 0.01, 2.0),                    |vm, v| vm.set_release(v)),

        // Filter
        spec("cutoff",   hz("Cutoff", 15000.0, 20.0, 20000.0),                  |vm, v| vm.set_filter_cutoff(v)),
        spec("reso",     plain("Resonance", 0.0, 0.0, 4.0, ""),                 |vm, v| vm.set_filter_resonance(v)),
        spec("drive",    plain("Drive", 1.0, 0.1, 5.0, ""),                     |vm, v| vm.set_filter_drive(v)),
        spec("sat",      plain("Saturation", 1.0, 0.0, 2.0, ""),                |vm, v| vm.set_filter_saturation(v)),
        spec("hpf",      hz("High-Pass", 16.0, 16.0, 8000.0),                   |vm, v| vm.set_hpf_cutoff(v)),

        // Filter envelope
        spec("fenvamt",  plain("Filter Env", 0.0, -5.0, 5.0, " oct"),           |vm, v| vm.set_filter_env_amount(v)),
        spec("fenvatk",  seconds("Filter Attack", 0.005, 0.001, 2.0),           |vm, v| vm.set_filter_attack(v)),
        spec("fenvdec",  seconds("Filter Decay", 0.3, 0.01, 2.0),               |vm, v| vm.set_filter_decay(v)),
        spec("fenvsus",  pct("Filter Sustain", 0.0),                            |vm, v| vm.set_filter_sustain(v)),
        spec("fenvrel",  seconds("Filter Release", 0.3, 0.01, 2.0),             |vm, v| vm.set_filter_release(v)),

        // Effects
        spec("fuzz",     pct("Fuzz", 0.0),                                      |vm, v| vm.set_fuzz(v)),
        spec("spring",   pct("Spring Reverb", 0.0),                             |vm, v| vm.set_spring(v)),
        spec("rvbdecay", pct("Reverb Decay", 0.5).with_unit(""),                |vm, v| vm.set_reverb_decay(v)),
        spec("rvbwet",   pct("Reverb Mix", 0.5),                                |vm, v| vm.set_reverb_wet(v)),
        guarded("chrate",  hz("Chorus Rate", 0.5, 0.1, 10.0),                   |vm, v| vm.set_chorus_rate(v)),
        guarded("chdepth", pct("Chorus Depth", 0.3),                            |vm, v| vm.set_chorus_depth(v)),
        spec("tpwow",    pct("Tape Wow", 0.0),                                  |vm, v| vm.set_tape_wow(v)),
        spec("tpflut",   pct("Tape Flutter", 0.0),                              |vm, v| vm.set_tape_flutter(v)),
        guarded("tpdrive", pct("Tape Drive", 0.0),                              |vm, v| vm.set_tape_drive(v)),
        guarded("tpage",   pct("Tape Age", 0.0),                                |vm, v| vm.set_tape_age(v)),
    ]
}

// ---------------------------------------------------------------------------
// Selector (enum) parameters
// ---------------------------------------------------------------------------

#[derive(Enum, PartialEq, Clone, Copy)]
enum WaveformParam {
    Sine,
    Triangle,
    Sawtooth,
    Square,
}

impl WaveformParam {
    fn to_engine(self) -> Waveform {
        match self {
            WaveformParam::Sine => Waveform::Sine,
            WaveformParam::Triangle => Waveform::Triangle,
            WaveformParam::Sawtooth => Waveform::Sawtooth,
            WaveformParam::Square => Waveform::Square,
        }
    }
}

#[derive(Enum, PartialEq, Clone, Copy)]
enum ChorusModeParam {
    Off,
    I,
    II,
    III,
    IV,
}

impl ChorusModeParam {
    fn to_engine(self) -> ChorusMode {
        match self {
            ChorusModeParam::Off => ChorusMode::Off,
            ChorusModeParam::I => ChorusMode::I,
            ChorusModeParam::II => ChorusMode::II,
            ChorusModeParam::III => ChorusMode::III,
            ChorusModeParam::IV => ChorusMode::IV,
        }
    }
}

// ---------------------------------------------------------------------------
// Params object built from the table
// ---------------------------------------------------------------------------

struct PatinaParams {
    waveform: EnumParam<WaveformParam>,
    chorus_mode: EnumParam<ChorusModeParam>,
    floats: Vec<FloatSpec>,
}

impl Default for PatinaParams {
    fn default() -> Self {
        Self {
            waveform: EnumParam::new("Waveform", WaveformParam::Sawtooth),
            chorus_mode: EnumParam::new("Chorus Mode", ChorusModeParam::Off),
            floats: float_specs(),
        }
    }
}

// SAFETY (per the trait contract): the returned pointers stay valid for as
// long as this object lives, which the wrapper guarantees by holding the
// same Arc the plugin returns from `params()`.
unsafe impl Params for PatinaParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        let mut map = vec![("waveform".to_string(), self.waveform.as_ptr(), String::new())];
        for float_spec in &self.floats {
            // The chorus selector belongs right before the chorus knobs
            if float_spec.id == "chrate" {
                map.push(("chmode".to_string(), self.chorus_mode.as_ptr(), String::new()));
            }
            map.push((float_spec.id.to_string(), float_spec.param.as_ptr(), String::new()));
        }
        map
    }
}

// ---------------------------------------------------------------------------
// The plugin
// ---------------------------------------------------------------------------

pub struct PatinaPlugin {
    params: Arc<PatinaParams>,
    vm: VoiceManager,
    sample_rate: f32,
    /// Last value pushed through each guarded setter (indexed like
    /// `params.floats`); NaN forces the first application.
    applied_floats: Vec<f32>,
    applied_waveform: Option<Waveform>,
    applied_chorus_mode: Option<ChorusMode>,
}

impl Default for PatinaPlugin {
    fn default() -> Self {
        let params = Arc::new(PatinaParams::default());
        let applied_floats = vec![f32::NAN; params.floats.len()];
        Self {
            params,
            vm: VoiceManager::new(44100.0, NUM_VOICES),
            sample_rate: 44100.0,
            applied_floats,
            applied_waveform: None,
            applied_chorus_mode: None,
        }
    }
}

impl PatinaPlugin {
    fn rebuild_engine(&mut self) {
        self.vm = VoiceManager::new(self.sample_rate, NUM_VOICES);
        self.applied_floats.fill(f32::NAN);
        self.applied_waveform = None;
        self.applied_chorus_mode = None;
        self.apply_params();
    }

    /// Push host parameter values into the engine, straight from the table.
    fn apply_params(&mut self) {
        let params = Arc::clone(&self.params);

        for (float_spec, last) in params.floats.iter().zip(self.applied_floats.iter_mut()) {
            let value = float_spec.param.value();
            if !float_spec.guarded || value != *last {
                (float_spec.apply)(&mut self.vm, value);
                *last = value;
            }
        }

        let waveform = params.waveform.value().to_engine();
        if self.applied_waveform != Some(waveform) {
            self.vm.set_waveform(waveform);
            self.applied_waveform = Some(waveform);
        }

        // Swaps the chorus voice bank — strictly change-only
        let chorus_mode = params.chorus_mode.value().to_engine();
        if self.applied_chorus_mode != Some(chorus_mode) {
            self.vm.set_chorus_mode(chorus_mode);
            self.applied_chorus_mode = Some(chorus_mode);
        }
    }
}

impl Plugin for PatinaPlugin {
    const NAME: &'static str = "Patina";
    const VENDOR: &'static str = "Sauers";
    const URL: &'static str = "https://github.com/SauersML/patina";
    const EMAIL: &'static str = "sauerslabs@gmail.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        aux_input_ports: &[],
        aux_output_ports: &[],
        names: PortNames::const_default(),
    }];

    // MidiCCs so pitch bend, mod wheel, and sustain pedal arrive too
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        self.rebuild_engine();
        true
    }

    fn reset(&mut self) {
        self.rebuild_engine();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.apply_params();

        let mut next_event = context.next_event();
        for (sample_id, channel_samples) in buffer.iter_samples().enumerate() {
            while let Some(event) = next_event {
                if event.timing() > sample_id as u32 {
                    break;
                }
                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        self.vm.note_on(note, velocity);
                    }
                    NoteEvent::NoteOff { note, .. } | NoteEvent::Choke { note, .. } => {
                        self.vm.note_off(note);
                    }
                    // 0.5 is center; a standard wheel spans +/-2 semitones
                    NoteEvent::MidiPitchBend { value, .. } => {
                        self.vm.set_pitch_bend((value - 0.5) * 2.0 * PITCH_BEND_SEMITONES);
                    }
                    NoteEvent::MidiCC { cc, value, .. } => match cc {
                        control_change::MODULATION_MSB => self.vm.set_mod_wheel(value),
                        control_change::DAMPER_PEDAL => {
                            self.vm.set_sustain_pedal(value >= 0.5)
                        }
                        _ => (),
                    },
                    _ => (),
                }
                next_event = context.next_event();
            }

            let (left, right) = self.vm.render_next();
            let mut channels = channel_samples.into_iter();
            if let Some(sample) = channels.next() {
                *sample = left;
            }
            if let Some(sample) = channels.next() {
                *sample = right;
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for PatinaPlugin {
    const CLAP_ID: &'static str = "com.sauers.patina";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Circuit-modeled polyphonic synthesizer with tape, spring, and germanium fuzz");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> =
        Some("https://github.com/SauersML/patina/issues");
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for PatinaPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"PatinaSynth00001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nice_export_clap!(PatinaPlugin);
nice_export_vst3!(PatinaPlugin);
