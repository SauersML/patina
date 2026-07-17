// CLAP / VST3 plugin front end (nih-plug). The host owns audio, MIDI, and
// parameter automation; everything else is the same VoiceManager the
// standalone app drives.
//
// Bundle with:
//   cargo xtask bundle patina --release --no-default-features --features plugin

use nih_plug::prelude::*;
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::chorus::ChorusMode;
use crate::oscillator::Waveform;
use crate::voice_manager::VoiceManager;

const NUM_VOICES: usize = 8;

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

#[derive(Params)]
struct PatinaParams {
    // Oscillator
    #[id = "waveform"]
    waveform: EnumParam<WaveformParam>,
    #[id = "volume"]
    volume: FloatParam,
    #[id = "detune"]
    detune: FloatParam,
    #[id = "pw"]
    pulse_width: FloatParam,
    #[id = "noise"]
    noise: FloatParam,

    // LFO
    #[id = "lforate"]
    lfo_rate: FloatParam,
    #[id = "lfoshape"]
    lfo_shape: FloatParam,
    #[id = "lfopitch"]
    lfo_pitch: FloatParam,
    #[id = "lfofilt"]
    lfo_filter: FloatParam,
    #[id = "lfopwm"]
    lfo_pwm: FloatParam,

    // Amplitude envelope
    #[id = "attack"]
    attack: FloatParam,
    #[id = "decay"]
    decay: FloatParam,
    #[id = "sustain"]
    sustain: FloatParam,
    #[id = "release"]
    release: FloatParam,

    // Filter
    #[id = "cutoff"]
    cutoff: FloatParam,
    #[id = "reso"]
    resonance: FloatParam,
    #[id = "drive"]
    drive: FloatParam,
    #[id = "sat"]
    saturation: FloatParam,
    #[id = "hpf"]
    hpf_cutoff: FloatParam,

    // Filter envelope
    #[id = "fenvamt"]
    fenv_amount: FloatParam,
    #[id = "fenvatk"]
    fenv_attack: FloatParam,
    #[id = "fenvdec"]
    fenv_decay: FloatParam,
    #[id = "fenvsus"]
    fenv_sustain: FloatParam,
    #[id = "fenvrel"]
    fenv_release: FloatParam,

    // Effects
    #[id = "fuzz"]
    fuzz: FloatParam,
    #[id = "spring"]
    spring: FloatParam,
    #[id = "rvbdecay"]
    reverb_decay: FloatParam,
    #[id = "rvbwet"]
    reverb_wet: FloatParam,
    #[id = "chmode"]
    chorus_mode: EnumParam<ChorusModeParam>,
    #[id = "chrate"]
    chorus_rate: FloatParam,
    #[id = "chdepth"]
    chorus_depth: FloatParam,
    #[id = "tpwow"]
    tape_wow: FloatParam,
    #[id = "tpflut"]
    tape_flutter: FloatParam,
    #[id = "tpdrive"]
    tape_drive: FloatParam,
    #[id = "tpage"]
    tape_age: FloatParam,
}

fn pct(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_unit(" %")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

fn seconds(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed {
            min,
            max,
            factor: FloatRange::skew_factor(-2.0),
        },
    )
    .with_unit(" s")
    .with_value_to_string(formatters::v2s_f32_rounded(3))
}

impl Default for PatinaParams {
    fn default() -> Self {
        Self {
            waveform: EnumParam::new("Waveform", WaveformParam::Sawtooth),
            volume: pct("Volume", 0.5),
            detune: FloatParam::new("Detune", 7.0, FloatRange::Linear { min: 0.0, max: 30.0 })
                .with_unit(" ct")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),
            pulse_width: FloatParam::new(
                "Pulse Width",
                0.5,
                FloatRange::Linear { min: 0.05, max: 0.95 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            noise: pct("Noise", 0.0),

            lfo_rate: FloatParam::new(
                "LFO Rate",
                1.0,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 30.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            lfo_shape: pct("LFO Shape", 0.5),
            lfo_pitch: FloatParam::new(
                "LFO > Pitch",
                0.0,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 200.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" ct")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),
            lfo_filter: FloatParam::new(
                "LFO > Filter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 4.0 },
            )
            .with_unit(" oct")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            lfo_pwm: FloatParam::new(
                "LFO > PWM",
                0.0,
                FloatRange::Linear { min: 0.0, max: 0.45 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            attack: seconds("Attack", 0.1, 0.01, 2.0),
            decay: seconds("Decay", 0.1, 0.01, 2.0),
            sustain: pct("Sustain", 0.7),
            release: seconds("Release", 0.2, 0.01, 2.0),

            cutoff: FloatParam::new(
                "Cutoff",
                15000.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 20000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            resonance: FloatParam::new(
                "Resonance",
                0.0,
                FloatRange::Linear { min: 0.0, max: 4.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            drive: FloatParam::new("Drive", 1.0, FloatRange::Linear { min: 0.1, max: 5.0 })
                .with_value_to_string(formatters::v2s_f32_rounded(2)),
            saturation: FloatParam::new(
                "Saturation",
                1.0,
                FloatRange::Linear { min: 0.0, max: 2.0 },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            hpf_cutoff: FloatParam::new(
                "High-Pass",
                16.0,
                FloatRange::Skewed {
                    min: 16.0,
                    max: 8000.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),

            fenv_amount: FloatParam::new(
                "Filter Env",
                0.0,
                FloatRange::Linear { min: -5.0, max: 5.0 },
            )
            .with_unit(" oct")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            fenv_attack: seconds("Filter Attack", 0.005, 0.001, 2.0),
            fenv_decay: seconds("Filter Decay", 0.3, 0.01, 2.0),
            fenv_sustain: pct("Filter Sustain", 0.0),
            fenv_release: seconds("Filter Release", 0.3, 0.01, 2.0),

            fuzz: pct("Fuzz", 0.0),
            spring: pct("Spring Reverb", 0.0),
            reverb_decay: FloatParam::new(
                "Reverb Decay",
                0.5,
                FloatRange::Linear { min: 0.0, max: 0.99 },
            )
            .with_value_to_string(formatters::v2s_f32_percentage(0)),
            reverb_wet: pct("Reverb Mix", 0.5),
            chorus_mode: EnumParam::new("Chorus Mode", ChorusModeParam::Off),
            chorus_rate: FloatParam::new(
                "Chorus Rate",
                0.5,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            chorus_depth: pct("Chorus Depth", 0.3),
            tape_wow: pct("Tape Wow", 0.0),
            tape_flutter: pct("Tape Flutter", 0.0),
            tape_drive: pct("Tape Drive", 0.0),
            tape_age: pct("Tape Age", 0.0),
        }
    }
}

/// Last-applied values for the setters that are NOT safe or cheap to call
/// redundantly every block (voice replacement, RNG, or self-calibration).
struct Applied {
    waveform: Waveform,
    chorus_mode: ChorusMode,
    chorus_rate: f32,
    chorus_depth: f32,
    tape_drive: f32,
    tape_age: f32,
}

impl Default for Applied {
    fn default() -> Self {
        // Matches a freshly constructed VoiceManager
        Self {
            waveform: Waveform::Sawtooth,
            chorus_mode: ChorusMode::Off,
            chorus_rate: 0.5,
            chorus_depth: 0.3,
            tape_drive: 0.0,
            tape_age: 0.0,
        }
    }
}

pub struct PatinaPlugin {
    params: Arc<PatinaParams>,
    vm: VoiceManager,
    sample_rate: f32,
    applied: Applied,
}

impl Default for PatinaPlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(PatinaParams::default()),
            vm: VoiceManager::new(44100.0, NUM_VOICES),
            sample_rate: 44100.0,
            applied: Applied::default(),
        }
    }
}

impl PatinaPlugin {
    fn rebuild_engine(&mut self) {
        self.vm = VoiceManager::new(self.sample_rate, NUM_VOICES);
        self.applied = Applied::default();
        self.apply_params();
    }

    /// Push host parameter values into the engine. Cheap, slewed setters are
    /// applied unconditionally each block; the guarded ones only on change.
    fn apply_params(&mut self) {
        let params = Arc::clone(&self.params);
        let p = params.as_ref();
        let vm = &mut self.vm;

        vm.set_volume(p.volume.value());
        vm.set_detune(p.detune.value());
        vm.set_pulse_width(p.pulse_width.value());
        vm.set_noise(p.noise.value());

        vm.set_lfo_rate(p.lfo_rate.value());
        vm.set_lfo_shape(p.lfo_shape.value());
        vm.set_lfo_pitch(p.lfo_pitch.value());
        vm.set_lfo_filter(p.lfo_filter.value());
        vm.set_lfo_pwm(p.lfo_pwm.value());

        vm.set_attack(p.attack.value());
        vm.set_decay(p.decay.value());
        vm.set_sustain(p.sustain.value());
        vm.set_release(p.release.value());

        vm.set_filter_cutoff(p.cutoff.value());
        vm.set_filter_resonance(p.resonance.value());
        vm.set_filter_drive(p.drive.value());
        vm.set_filter_saturation(p.saturation.value());
        vm.set_hpf_cutoff(p.hpf_cutoff.value());

        vm.set_filter_env_amount(p.fenv_amount.value());
        vm.set_filter_attack(p.fenv_attack.value());
        vm.set_filter_decay(p.fenv_decay.value());
        vm.set_filter_sustain(p.fenv_sustain.value());
        vm.set_filter_release(p.fenv_release.value());

        vm.set_fuzz(p.fuzz.value());
        vm.set_spring(p.spring.value());
        vm.set_reverb_decay(p.reverb_decay.value());
        vm.set_reverb_wet(p.reverb_wet.value());
        vm.set_tape_wow(p.tape_wow.value());
        vm.set_tape_flutter(p.tape_flutter.value());

        let waveform = p.waveform.value().to_engine();
        if waveform != self.applied.waveform {
            vm.set_waveform(waveform);
            self.applied.waveform = waveform;
        }

        // Chorus mode swaps the voice bank; rate/depth re-randomize per-voice
        // offsets — only touch them when the value actually moved
        let chorus_mode = p.chorus_mode.value().to_engine();
        if chorus_mode != self.applied.chorus_mode {
            vm.set_chorus_mode(chorus_mode);
            self.applied.chorus_mode = chorus_mode;
        }
        let chorus_rate = p.chorus_rate.value();
        if chorus_rate != self.applied.chorus_rate {
            vm.set_chorus_rate(chorus_rate);
            self.applied.chorus_rate = chorus_rate;
        }
        let chorus_depth = p.chorus_depth.value();
        if chorus_depth != self.applied.chorus_depth {
            vm.set_chorus_depth(chorus_depth);
            self.applied.chorus_depth = chorus_depth;
        }

        // Tape drive re-runs the deck's self-calibration; age reschedules
        // dropouts — change-only as well
        let tape_drive = p.tape_drive.value();
        if tape_drive != self.applied.tape_drive {
            vm.set_tape_drive(tape_drive);
            self.applied.tape_drive = tape_drive;
        }
        let tape_age = p.tape_age.value();
        if tape_age != self.applied.tape_age {
            vm.set_tape_age(tape_age);
            self.applied.tape_age = tape_age;
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

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
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

nih_export_clap!(PatinaPlugin);
nih_export_vst3!(PatinaPlugin);
