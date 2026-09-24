//! Audition the factory bank the way the live panel plays it: a fresh engine
//! comes on in Init, the patch is clicked on top, and the keys are played.
//! Writes one 32-bit float WAV per patch (no normalization) plus a cold
//! (un-warmed) Init take, for listening and measurement.
//!
//!   cargo run --release --no-default-features --example audition -- OUT_DIR
use patina::patch::{load, FACTORY};
use patina::voice_manager::VoiceManager;
use std::io::Write;

const SR: f32 = 48000.0;

fn write_wav(path: &str, frames: &[(f32, f32)]) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let data = (frames.len() * 8) as u32;
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&3u16.to_le_bytes()).unwrap(); // IEEE float
    f.write_all(&2u16.to_le_bytes()).unwrap();
    f.write_all(&(SR as u32).to_le_bytes()).unwrap();
    f.write_all(&(SR as u32 * 8).to_le_bytes()).unwrap();
    f.write_all(&8u16.to_le_bytes()).unwrap();
    f.write_all(&32u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data.to_le_bytes()).unwrap();
    for (l, r) in frames {
        f.write_all(&l.to_le_bytes()).unwrap();
        f.write_all(&r.to_le_bytes()).unwrap();
    }
}

/// (seconds from start, note offset from the patch's C, velocity, held seconds)
fn phrase() -> Vec<(f32, i32, f32, f32)> {
    let mut ev = vec![];
    // 0.0: one long note, then its release
    ev.push((0.0, 0, 0.8, 1.6));
    // 3.0: a held chord
    for n in [0, 4, 7, 11] {
        ev.push((3.0, n, 0.7, 2.0));
    }
    // 6.0: a quick legato-ish line, then staccato
    let line = [0, 2, 3, 5, 7, 5, 3, 2, 0, 7, 12, 7];
    for (i, n) in line.iter().enumerate() {
        let vel = if i % 4 == 0 { 0.95 } else { 0.55 };
        ev.push((6.0 + i as f32 * 0.18, *n, vel, if i < 8 { 0.17 } else { 0.06 }));
    }
    // 9.0: an octave below and two octaves above
    ev.push((9.0, -12, 0.8, 1.0));
    ev.push((10.5, 24, 0.8, 1.0));
    ev
}

fn render(text: &str, warm: bool) -> Vec<(f32, f32)> {
    let mut vm = VoiceManager::new(SR, 10);
    if warm {
        vm.warm_up();
    }
    load(&mut vm, text).unwrap();
    let base = 12 * (vm.params.ui_octave as i32 + 1);
    let ev = phrase();
    let total = (13.5 * SR) as usize;
    let mut out = Vec::with_capacity(total);
    for i in 0..total {
        let t = i as f32 / SR;
        let t0 = (i as f32 - 1.0) / SR;
        for &(on, n, vel, held) in &ev {
            let note = (base + n) as u8;
            if on <= t && on > t0 || (i == 0 && on == 0.0) {
                vm.note_on(note, vel);
            }
            let off = on + held;
            if off <= t && off > t0 {
                vm.note_off(note);
            }
        }
        out.push(vm.render_next());
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).expect("usage: audition OUT_DIR [PATCH OUT_NAME 'param value'...]");
    std::fs::create_dir_all(dir).unwrap();
    // Probe mode: one patch with lines laid on top, to isolate a circuit.
    if let Some(want) = args.get(2) {
        let (_, text) = FACTORY.iter().find(|(n, _)| n.eq_ignore_ascii_case(want)).expect("no such patch");
        let text = format!("{text}\n{}", args[4..].join("\n"));
        write_wav(&format!("{dir}/{}.wav", args[3]), &render(&text, true));
        return;
    }
    for (name, text) in FACTORY {
        let slug = name.to_lowercase().replace(' ', "-");
        write_wav(&format!("{dir}/{slug}.wav"), &render(text, true));
    }
    write_wav(&format!("{dir}/init-cold.wav"), &render(FACTORY[0].1, false));
}
