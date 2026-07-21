// Patina — a circuit-modeled polyphonic synthesizer.
//
// The engine (everything the audio thread touches) lives in this library so
// it can be driven by either front end:
//   - the standalone app (src/main.rs, feature "app"): window, audio device,
//     hardware MIDI input, song player
//   - the CLAP/VST3 plugin (src/plugin.rs, feature "plugin"): the host owns
//     audio, MIDI, and parameter automation

pub mod adaa;
pub mod chorus;
pub mod drums;
pub mod envelope;
pub mod filter;
pub mod fuzz;
pub mod hpf;
pub mod lfo;
pub mod noise;
pub mod oscillator;
pub mod patch;
pub mod render;
pub mod reverb;
pub mod sampler;
pub mod song;
pub mod spring;
pub mod substrate;
pub mod tape;
pub mod vocoder;
pub mod voice;
pub mod voice_manager;
pub mod vox;

#[cfg(feature = "app")]
pub mod aurora_gpu;
#[cfg(feature = "app")]
pub mod midi_handler;
#[cfg(feature = "app")]
pub mod panel_render;
#[cfg(feature = "app")]
pub mod ui;

#[cfg(feature = "plugin")]
mod plugin;
