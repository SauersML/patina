// CLAP / VST3 plugin front end (nice-plug). The host owns audio, MIDI, and
// parameter automation; everything else is the same VoiceManager the
// standalone app drives.
//
// SINGLE SOURCE OF TRUTH: every parameter this front end exposes comes from
// the shared table in src/host_params.rs — id, display name, range,
// default, formatting, and the VoiceManager setter it drives. The host
// parameter list, default values, state save/restore, and per-block
// application are all derived from that table, and the Audio Unit front end
// (src/au/) derives from the very same table. The two selector params,
// waveform and chorus mode, stay typed enums here so CLAP/VST3 hosts render
// them as dropdowns; a test pins them to the table.
//
// Bundle with:
//   cargo xtask bundle patina --release --no-default-features --features plugin

use nice_plug::prelude::*;
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::host_params::{
    self, ChoiceDef, Display, FloatDef, ParamDef, NUM_VOICES, PITCH_BEND_SEMITONES,
};
use crate::voice_manager::VoiceManager;

// ---------------------------------------------------------------------------
// nice-plug params derived from the shared table
// ---------------------------------------------------------------------------

/// One host-facing float parameter: the shared definition plus the
/// nice-plug control built from it.
struct FloatSlot {
    def: FloatDef,
    param: FloatParam,
}

fn build_float_param(def: &FloatDef) -> FloatParam {
    let range = match def.display {
        Display::Seconds | Display::Hertz => FloatRange::Skewed {
            min: def.min,
            max: def.max,
            factor: FloatRange::skew_factor(-2.0),
        },
        _ => FloatRange::Linear { min: def.min, max: def.max },
    };
    let param = FloatParam::new(def.name, def.default, range);
    match def.display {
        Display::Percent => param
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage())
            .with_unit(" %"),
        Display::Fraction => param
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage()),
        Display::Seconds => param
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
        Display::Hertz => param
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
        Display::Plain(unit) => param
            .with_unit(unit)
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
    }
}

#[derive(Enum, PartialEq, Clone, Copy)]
enum WaveformParam {
    Sine,
    Triangle,
    Sawtooth,
    Square,
}

#[derive(Enum, PartialEq, Clone, Copy)]
enum ChorusModeParam {
    Off,
    I,
    II,
    III,
    IV,
}

struct PatinaParams {
    waveform: EnumParam<WaveformParam>,
    chorus_mode: EnumParam<ChorusModeParam>,
    /// The table's selector entries; their `apply` drives the engine.
    waveform_def: ChoiceDef,
    chorus_def: ChoiceDef,
    floats: Vec<FloatSlot>,
}

impl Default for PatinaParams {
    fn default() -> Self {
        let mut waveform_def = None;
        let mut chorus_def = None;
        let mut floats = Vec::new();
        for def in host_params::param_defs() {
            match def {
                ParamDef::Float(fd) => {
                    floats.push(FloatSlot { param: build_float_param(&fd), def: fd })
                }
                ParamDef::Choice(cd) if cd.id == "waveform" => waveform_def = Some(cd),
                ParamDef::Choice(cd) => chorus_def = Some(cd),
            }
        }
        Self {
            waveform: EnumParam::new("Waveform", WaveformParam::Sawtooth),
            chorus_mode: EnumParam::new("Chorus Mode", ChorusModeParam::Off),
            waveform_def: waveform_def.expect("table has a waveform selector"),
            chorus_def: chorus_def.expect("table has a chorus mode selector"),
            floats,
        }
    }
}

// SAFETY (per the trait contract): the returned pointers stay valid for as
// long as this object lives, which the wrapper guarantees by holding the
// same Arc the plugin returns from `params()`.
unsafe impl Params for PatinaParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        // Emit in the table's canonical order so every format agrees.
        let mut floats = self.floats.iter();
        host_params::param_defs()
            .iter()
            .map(|def| {
                let ptr = match def {
                    ParamDef::Choice(c) if c.id == "waveform" => self.waveform.as_ptr(),
                    ParamDef::Choice(_) => self.chorus_mode.as_ptr(),
                    ParamDef::Float(_) => {
                        floats.next().expect("one FloatSlot per float def").param.as_ptr()
                    }
                };
                (def.id().to_string(), ptr, String::new())
            })
            .collect()
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
    applied_waveform: Option<usize>,
    applied_chorus_mode: Option<usize>,
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

        for (slot, last) in params.floats.iter().zip(self.applied_floats.iter_mut()) {
            let value = slot.param.value();
            if !slot.def.guarded || value != *last {
                (slot.def.apply)(&mut self.vm, value);
                *last = value;
            }
        }

        // The selectors swap voice banks — strictly change-only
        let waveform = params.waveform.value() as usize;
        if self.applied_waveform != Some(waveform) {
            (params.waveform_def.apply)(&mut self.vm, waveform);
            self.applied_waveform = Some(waveform);
        }

        let chorus_mode = params.chorus_mode.value() as usize;
        if self.applied_chorus_mode != Some(chorus_mode) {
            (params.chorus_def.apply)(&mut self.vm, chorus_mode);
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
                    NoteEvent::NoteOn { note, velocity, channel, .. } => {
                        host_params::note_on(&mut self.vm, channel, note, velocity);
                    }
                    NoteEvent::NoteOff { note, channel, .. }
                    | NoteEvent::Choke { note, channel, .. } => {
                        host_params::note_off(&mut self.vm, channel, note);
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

// ---------------------------------------------------------------------------
// Drift pins: the typed enums above must mirror the shared table
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_enums_mirror_the_table() {
        let params = PatinaParams::default();
        assert_eq!(WaveformParam::variants(), params.waveform_def.variants);
        assert_eq!(ChorusModeParam::variants(), params.chorus_def.variants);
        assert_eq!(
            params.waveform.default_plain_value() as usize,
            params.waveform_def.default
        );
        assert_eq!(
            params.chorus_mode.default_plain_value() as usize,
            params.chorus_def.default
        );
    }

    #[test]
    fn param_map_follows_table_order() {
        let params = PatinaParams::default();
        let ids: Vec<String> =
            params.param_map().into_iter().map(|(id, _, _)| id).collect();
        let table_ids: Vec<String> =
            host_params::param_defs().iter().map(|d| d.id().to_string()).collect();
        assert_eq!(ids, table_ids);
    }
}
