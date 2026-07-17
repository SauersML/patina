// Patch (preset) system, in the spirit of US 3,981,218 ("Preset System for
// Electronic Musical Instrument"): one selection switches every functional
// block of the synthesizer at once. A patch is a plain-text list of
// `param value` lines using the SAME parameter names as song automation, so
// patches, songs, and knobs all speak one language. Applying a patch just
// calls the live setters — the UI follows automatically, and you can click
// through presets while holding a chord to morph the sound underneath it.

use crate::song::Param;
use crate::voice_manager::{ParamValues, VoiceManager};
use crate::chorus::ChorusMode;
use crate::oscillator::Waveform;

/// The factory bank, embedded so the binary is self-contained.
pub const FACTORY: &[(&str, &str)] = &[
    ("Init", include_str!("../patches/init.patch")),
    ("Velvet", include_str!("../patches/velvet-strings.patch")),
    ("Acid", include_str!("../patches/acid-bath.patch")),
    ("Pluck", include_str!("../patches/night-pluck.patch")),
    ("Ghost", include_str!("../patches/tape-ghost.patch")),
    ("Fathom", include_str!("../patches/fathom-bass.patch")),
    ("Cry", include_str!("../patches/germanium-cry.patch")),
    ("Cathedral", include_str!("../patches/cathedral.patch")),
];

/// A `#` starts a comment at line start or after whitespace (same rule as
/// the song DSL, so F#4-style tokens would survive if ever used here).
fn strip_comment(raw: &str) -> &str {
    let bytes = raw.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            return &raw[..i];
        }
    }
    raw
}

pub fn apply(vm: &mut VoiceManager, text: &str) -> Result<(), String> {
    for (no, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let name = it.next().unwrap();
        let value: f32 = it
            .next()
            .ok_or_else(|| format!("line {}: '{}' has no value", no + 1, name))?
            .parse()
            .map_err(|_| format!("line {}: bad value for '{}'", no + 1, name))?;
        let param = Param::from_name(name)
            .ok_or_else(|| format!("line {}: unknown parameter '{}'", no + 1, name))?;
        param.apply(vm, value);
    }
    Ok(())
}

/// Snapshot the current parameters as patch text (the inverse of `apply`).
pub fn serialize(p: &ParamValues) -> String {
    let waveform = match p.waveform {
        Waveform::Sine => 0,
        Waveform::Square => 1,
        Waveform::Sawtooth => 2,
        Waveform::Triangle => 3,
    };
    let chorus_mode = match p.chorus_mode {
        ChorusMode::Off => 0,
        ChorusMode::I => 1,
        ChorusMode::II => 2,
        ChorusMode::III => 3,
        ChorusMode::IV => 4,
    };
    format!(
        "# Patina patch\n\
         volume {}\nwaveform {}\ndetune {}\nnoise {}\nglide {}\npulse_width {}\n\
         lfo_rate {}\nlfo_shape {}\nlfo_pitch {}\nlfo_filter {}\nlfo_pwm {}\n\
         cutoff {}\nresonance {}\ndrive {}\nsaturation {}\nhpf {}\n\
         filter_env {}\nfilter_attack {}\nfilter_decay {}\nfilter_sustain {}\nfilter_release {}\n\
         attack {}\ndecay {}\nsustain {}\nrelease {}\n\
         fuzz {}\nspring {}\nreverb_decay {}\nreverb_wet {}\n\
         chorus_mode {}\nchorus_rate {}\nchorus_depth {}\n\
         tape_wow {}\ntape_flutter {}\ntape_drive {}\ntape_age {}\n",
        p.volume, waveform, p.detune, p.noise, p.glide, p.pulse_width,
        p.lfo_rate, p.lfo_shape, p.lfo_pitch, p.lfo_filter, p.lfo_pwm,
        p.cutoff, p.resonance, p.drive, p.saturation, p.hpf_cutoff,
        p.filter_env_amount, p.filter_attack, p.filter_decay, p.filter_sustain, p.filter_release,
        p.attack, p.decay, p.sustain, p.release,
        p.fuzz, p.spring, p.reverb_decay, p.reverb_wet,
        chorus_mode, p.chorus_rate, p.chorus_depth,
        p.tape_wow, p.tape_flutter, p.tape_drive, p.tape_age,
    )
}

/// Save the current sound to patches/user-N.patch, N = first free slot.
/// Returns the path written.
pub fn save_user_patch(p: &ParamValues) -> std::io::Result<String> {
    std::fs::create_dir_all("patches")?;
    let mut n = 1;
    let path = loop {
        let candidate = format!("patches/user-{n}.patch");
        if !std::path::Path::new(&candidate).exists() {
            break candidate;
        }
        n += 1;
    };
    std::fs::write(&path, serialize(p))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every factory patch must apply with zero unknown parameters, and
    /// leave the engine in a sane audible state.
    #[test]
    fn factory_bank_applies_cleanly() {
        for (name, text) in FACTORY {
            let mut vm = VoiceManager::new(44100.0, 8);
            apply(&mut vm, text).unwrap_or_else(|e| panic!("patch '{name}': {e}"));
            assert!(
                vm.params.volume > 0.0,
                "patch '{name}' should set an audible volume"
            );
            assert!(vm.params.cutoff >= 16.0);
        }
    }

    /// serialize -> apply must round-trip the parameter block.
    #[test]
    fn snapshot_round_trips() {
        let mut vm = VoiceManager::new(44100.0, 8);
        apply(&mut vm, FACTORY[2].1).unwrap(); // Acid
        let snap = serialize(&vm.params);

        let mut vm2 = VoiceManager::new(44100.0, 8);
        apply(&mut vm2, &snap).unwrap();
        assert_eq!(vm.params.cutoff, vm2.params.cutoff);
        assert_eq!(vm.params.resonance, vm2.params.resonance);
        assert_eq!(vm.params.glide, vm2.params.glide);
        assert_eq!(vm.params.fuzz, vm2.params.fuzz);
        assert_eq!(vm.params.filter_env_amount, vm2.params.filter_env_amount);
    }
}
