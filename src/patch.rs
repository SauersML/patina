// Patch (preset) system, in the spirit of US 3,981,218 ("Preset System for
// Electronic Musical Instrument"): one selection switches every functional
// block of the synthesizer at once. A patch is a plain-text list of
// `param value` lines using the SAME parameter names as song automation, so
// patches, songs, and knobs all speak one language. Applying a patch just
// calls the live setters — the UI follows automatically, and you can click
// through presets while holding a chord to morph the sound underneath it.

use crate::song::{Param, ParseFinite, PARAM_DEFS};
use crate::voice_manager::{ParamValues, VoiceManager};

/// The factory bank, embedded so the binary is self-contained.
pub const FACTORY: &[(&str, &str)] = &[
    ("Init", include_str!("../patches/init.patch")),
    ("Ember", include_str!("../patches/ember.patch")),
    ("Tidewater", include_str!("../patches/tidewater.patch")),
    ("Glasswing", include_str!("../patches/glasswing.patch")),
    ("Sea of Dials", include_str!("../patches/sea-of-dials.patch")),
    ("Aurora", include_str!("../patches/aurora.patch")),
    ("Vellum", include_str!("../patches/vellum.patch")),
    ("Choir", include_str!("../patches/cassette-choir.patch")),
    ("Thunder", include_str!("../patches/thunder-organ.patch")),
    ("Lantern", include_str!("../patches/lantern.patch")),
    ("Tears", include_str!("../patches/tears.patch")),
    ("Fathom", include_str!("../patches/fathom.patch")),
    ("Warehouse", include_str!("../patches/warehouse.patch")),
    ("Bocuma", include_str!("../patches/bocuma.patch")),
    ("Kaini", include_str!("../patches/kaini.patch")),
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

/// Init's setting for `param`, if Init names it. Init is the power-on
/// state and the reset position of every control, in the app panel and in
/// every plugin host alike, so each of them reads its default here rather
/// than keeping a list of its own.
pub fn init_value(param: Param) -> Option<f32> {
    let name = param.name();
    FACTORY[0]
        .1
        .lines()
        .filter_map(|raw| {
            let mut it = strip_comment(raw).split_whitespace();
            (it.next()? == name).then(|| it.next()?.parse::<f32>().ok())?
        })
        .last()
}

/// Select a patch: every block of the panel moves at once (US 3,981,218),
/// so the patch is laid over Init rather than over whatever the last patch
/// left behind. A patch that does not mention a control gets Init's
/// setting for it. Before this, a patch's sound depended on the previous
/// selection: a 49-line patch clicked after Warehouse kept Warehouse's
/// unison, reverb tone and mixer levels.
pub fn load(vm: &mut VoiceManager, text: &str) -> Result<(), String> {
    apply(vm, FACTORY[0].1)?;
    apply(vm, text)
}

/// Lay `text`'s lines over the current state (song automation, MIDI and
/// incremental edits); selecting a whole patch goes through [`load`].
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
            .parse_finite()
            .map_err(|_| format!("line {}: bad value for '{}'", no + 1, name))?;
        let param = Param::from_name(name)
            .ok_or_else(|| format!("line {}: unknown parameter '{}'", no + 1, name))?;
        param.apply(vm, value);
    }
    Ok(())
}

/// Snapshot the current parameters as patch text (the inverse of `apply`):
/// one line for every parameter the snapshot holds, in table order (so the
/// `waveform` macro line comes before the per-oscillator overrides). The
/// list is the parameter table itself, so a control added to the engine is
/// saved without anyone remembering to add it here. It used to be a
/// hand-kept list that had already lost unison, reverb tone and predelay,
/// and the drum bus tone.
pub fn serialize(p: &ParamValues) -> String {
    let mut out = String::from("# Patina patch\n");
    for def in PARAM_DEFS {
        if let Some(v) = def.param.read(p) {
            out.push_str(&format!("{} {}\n", def.name, v));
        }
    }
    out
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

    /// Selecting a patch lands on the same panel whatever was selected
    /// before it: nothing a patch leaves unmentioned leaks through from the
    /// previous one.
    #[test]
    fn a_patch_sounds_the_same_whatever_came_before() {
        for (name, text) in FACTORY {
            let mut fresh = VoiceManager::new(44100.0, 8);
            load(&mut fresh, text).unwrap();
            for (prev, prev_text) in FACTORY {
                let mut vm = VoiceManager::new(44100.0, 8);
                load(&mut vm, prev_text).unwrap();
                load(&mut vm, text).unwrap();
                assert_eq!(
                    serialize(&vm.params),
                    serialize(&fresh.params),
                    "'{name}' after '{prev}' differs from '{name}' alone"
                );
                assert_eq!(
                    vm.params.unison, fresh.params.unison,
                    "'{name}' after '{prev}'"
                );
                assert_eq!(
                    vm.params.ui_octave, fresh.params.ui_octave,
                    "'{name}' after '{prev}'"
                );
                assert_eq!(
                    vm.params.reverb_tone, fresh.params.reverb_tone,
                    "'{name}' after '{prev}'"
                );
            }
        }
    }

    /// serialize -> apply must round-trip the parameter block.
    #[test]
    fn snapshot_round_trips() {
        let mut vm = VoiceManager::new(44100.0, 8);
        apply(&mut vm, FACTORY[2].1).unwrap();
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
