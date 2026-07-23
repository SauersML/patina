// CLAP / VST3 plugin front end (nice-plug). The host owns audio, MIDI, and
// parameter automation; everything else is the same VoiceManager the
// standalone app drives.
//
// SINGLE SOURCE OF TRUTH: every parameter this front end exposes comes from
// the shared table in src/host_params.rs — id, display name, range,
// default, formatting, and (via `song::Param`) the VoiceManager setter it
// drives. The host parameter list, default values, state save/restore, and
// per-block application are all derived from that table, and the Audio Unit
// front end (src/au/) derives from the very same table. The selector params
// (circuit, waveform, oscillator 2/3 waveform, sync, chorus mode) stay typed
// enums here so CLAP/VST3 hosts render them as dropdowns; a test pins each
// one's names and default to the table.
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

// The four distinct selector shapes. Each variant list mirrors the matching
// ChoiceDef in the shared table (pinned by `selector_enums_mirror_the_table`),
// in the engine's own value order so the host index applies as itself.
#[derive(Enum, PartialEq, Clone, Copy)]
enum WaveformParam {
    Sine,
    Square,
    Sawtooth,
    Triangle,
}

#[derive(Enum, PartialEq, Clone, Copy)]
enum CircuitParam {
    Moog,
    #[name = "ARP"]
    Arp,
}

#[derive(Enum, PartialEq, Clone, Copy)]
enum SyncParam {
    Off,
    On,
}

#[derive(Enum, PartialEq, Clone, Copy)]
enum ChorusModeParam {
    Off,
    I,
    II,
    III,
    IV,
}

/// A host selector backed by one of the typed EnumParams above. The wrapped
/// param renders as a named dropdown; `index()` reads back the chosen
/// position so it can drive the engine through `ChoiceDef::param`.
enum ChoiceKind {
    Wave(EnumParam<WaveformParam>),
    Circuit(EnumParam<CircuitParam>),
    Sync(EnumParam<SyncParam>),
    Chorus(EnumParam<ChorusModeParam>),
}

impl ChoiceKind {
    /// Build the right typed EnumParam for a table selector, defaulting to
    /// the table's default index.
    fn from_def(def: &ChoiceDef) -> Self {
        let name = def.name;
        match def.id {
            "waveform" | "osc2_wave" | "osc3_wave" => {
                ChoiceKind::Wave(EnumParam::new(name, WaveformParam::from_index(def.default)))
            }
            "circuit" => {
                ChoiceKind::Circuit(EnumParam::new(name, CircuitParam::from_index(def.default)))
            }
            "sync" => ChoiceKind::Sync(EnumParam::new(name, SyncParam::from_index(def.default))),
            "chorus_mode" => {
                ChoiceKind::Chorus(EnumParam::new(name, ChorusModeParam::from_index(def.default)))
            }
            other => panic!("no typed EnumParam for selector `{other}`"),
        }
    }

    fn as_ptr(&self) -> ParamPtr {
        match self {
            ChoiceKind::Wave(p) => p.as_ptr(),
            ChoiceKind::Circuit(p) => p.as_ptr(),
            ChoiceKind::Sync(p) => p.as_ptr(),
            ChoiceKind::Chorus(p) => p.as_ptr(),
        }
    }

    /// The chosen variant index — the value `Param::apply` maps to an enum.
    fn index(&self) -> usize {
        match self {
            ChoiceKind::Wave(p) => p.value().to_index(),
            ChoiceKind::Circuit(p) => p.value().to_index(),
            ChoiceKind::Sync(p) => p.value().to_index(),
            ChoiceKind::Chorus(p) => p.value().to_index(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn variants(&self) -> &'static [&'static str] {
        match self {
            ChoiceKind::Wave(_) => WaveformParam::variants(),
            ChoiceKind::Circuit(_) => CircuitParam::variants(),
            ChoiceKind::Sync(_) => SyncParam::variants(),
            ChoiceKind::Chorus(_) => ChorusModeParam::variants(),
        }
    }
}

/// One host-facing selector: the shared definition plus its typed control.
struct ChoiceSlot {
    def: ChoiceDef,
    kind: ChoiceKind,
}

struct PatinaParams {
    floats: Vec<FloatSlot>,
    choices: Vec<ChoiceSlot>,
}

impl Default for PatinaParams {
    fn default() -> Self {
        let mut floats = Vec::new();
        let mut choices = Vec::new();
        // Both vecs are filled in table-encounter order, so a per-type
        // running iterator in `param_map` re-interleaves them correctly.
        for def in host_params::param_defs() {
            match def {
                ParamDef::Float(fd) => {
                    floats.push(FloatSlot { param: build_float_param(&fd), def: fd })
                }
                ParamDef::Choice(cd) => {
                    let kind = ChoiceKind::from_def(&cd);
                    choices.push(ChoiceSlot { def: cd, kind });
                }
            }
        }
        Self { floats, choices }
    }
}

// SAFETY (per the trait contract): the returned pointers stay valid for as
// long as this object lives, which the wrapper guarantees by holding the
// same Arc the plugin returns from `params()`.
unsafe impl Params for PatinaParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        // Emit in the table's canonical order so every format agrees.
        let mut floats = self.floats.iter();
        let mut choices = self.choices.iter();
        host_params::param_defs()
            .iter()
            .map(|def| {
                let ptr = match def {
                    ParamDef::Choice(_) => {
                        choices.next().expect("one ChoiceSlot per choice def").kind.as_ptr()
                    }
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
    /// Last index applied for each selector (indexed like `params.choices`).
    applied_choices: Vec<Option<usize>>,
}

impl Default for PatinaPlugin {
    fn default() -> Self {
        let params = Arc::new(PatinaParams::default());
        let applied_floats = vec![f32::NAN; params.floats.len()];
        let applied_choices = vec![None; params.choices.len()];
        Self {
            params,
            vm: VoiceManager::new(44100.0, NUM_VOICES),
            sample_rate: 44100.0,
            applied_floats,
            applied_choices,
        }
    }
}

impl PatinaPlugin {
    fn rebuild_engine(&mut self) {
        self.vm = VoiceManager::new(self.sample_rate, NUM_VOICES);
        self.applied_floats.fill(f32::NAN);
        self.applied_choices.iter_mut().for_each(|c| *c = None);
        self.apply_params();
    }

    /// Push host parameter values into the engine, straight from the table.
    fn apply_params(&mut self) {
        let params = Arc::clone(&self.params);

        for (slot, last) in params.floats.iter().zip(self.applied_floats.iter_mut()) {
            let value = slot.param.value();
            if !slot.def.guarded || value != *last {
                slot.def.param.apply(&mut self.vm, value);
                *last = value;
            }
        }

        // The selectors swap voice banks / circuit models — strictly
        // change-only. The index feeds Param::apply, which maps it to the
        // engine enum position.
        for (slot, last) in params.choices.iter().zip(self.applied_choices.iter_mut()) {
            let index = slot.kind.index();
            if *last != Some(index) {
                slot.def.param.apply(&mut self.vm, index as f32);
                *last = Some(index);
            }
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
        // Every table selector got a typed EnumParam whose variant names and
        // default index match the shared definition exactly.
        for slot in &params.choices {
            assert_eq!(
                slot.kind.variants(),
                slot.def.variants,
                "variants for `{}`",
                slot.def.id
            );
            assert_eq!(slot.kind.index(), slot.def.default, "default for `{}`", slot.def.id);
        }
        // And the plugin backs every selector the table declares.
        let table_choices = host_params::param_defs()
            .into_iter()
            .filter(|d| matches!(d, ParamDef::Choice(_)))
            .count();
        assert_eq!(params.choices.len(), table_choices);
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
