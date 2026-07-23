// Patina — a circuit-modeled polyphonic synthesizer.
//
// The engine (everything the audio thread touches) lives in this library so
// it can be driven by either front end:
//   - the standalone app (src/main.rs, feature "app"): window, audio device,
//     hardware MIDI input, song player
//   - the CLAP/VST3 plugin (src/plugin.rs, feature "plugin"): the host owns
//     audio, MIDI, and parameter automation
//   - the Audio Unit (src/au/, feature "au"): native AUv2 MusicDevice for
//     Logic Pro, GarageBand, and other AU hosts
// The plugin front ends share one parameter table (src/host_params.rs).

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
pub mod rng;
pub mod smoothing;
pub mod sampler;
pub mod song;
pub mod spectral;
pub mod spring;
pub mod substrate;
pub mod talker;
pub mod tape;
pub mod vocoder;
pub mod voice;
pub mod voice_manager;
pub mod vox;

#[cfg(feature = "app")]
pub mod aurora_gpu;
#[cfg(feature = "app")]
pub mod midi_handler;
#[cfg(any(feature = "app", feature = "editor"))]
pub mod panel;
#[cfg(any(feature = "app", feature = "editor"))]
pub mod panel_render;
#[cfg(feature = "app")]
pub mod ui;

#[cfg(any(feature = "plugin", feature = "au"))]
pub mod host_params;

#[cfg(feature = "editor")]
pub mod editor;

#[cfg(feature = "plugin")]
mod plugin;

#[cfg(all(feature = "au", target_os = "macos"))]
pub mod au;
