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

/// The sample rates this engine is built for, and will accept from a host.
///
/// This is a real limit, not a formality. The circuits are modeled at
/// fixed frequencies in Hz — the 909 hat bank's 5.2 kHz high-pass, the
/// clap's 1.1 kHz band-pass, the talk box's 4.8 kHz tube edge, the
/// vocoder's 7.2 kHz top band, the tape's 8 kHz record shelf. Every one
/// of those is pinned under Nyquist internally, but below ~8 kHz there is
/// no Nyquist left to pin them under: the clap's band-pass centre alone
/// is above half the rate, and a resonator asked to sit above Nyquist
/// does not detune, it diverges (measured: the 909 clap reached 1e23 at a
/// 1 kHz rate).
///
/// A host that asks for a rate outside this band gets a clean refusal —
/// which is what `kAudioUnitErr_FormatNotSupported` is for — rather than
/// a plugin that returns garbage and gets the whole instrument blamed.
/// The upper bound is above every rate any host offers (192 kHz is the
/// practical maximum; 768 kHz is headroom).
pub const MIN_SAMPLE_RATE: f64 = 8000.0;
pub const MAX_SAMPLE_RATE: f64 = 768_000.0;

/// Is this a rate the engine can actually be built at?
pub fn supported_sample_rate(rate: f64) -> bool {
    rate.is_finite() && (MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&rate)
}

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
