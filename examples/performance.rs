//! Repeatable CPU benchmark: cargo run --release --no-default-features --example performance
use patina::voice_manager::{ParamValues, VoiceManager};
use std::{hint::black_box, time::Instant};

fn cpu_seconds() -> f64 {
    unsafe extern "C" {
        fn clock() -> std::os::raw::c_long;
    }
    #[cfg(unix)]
    const TICKS: f64 = 1_000_000.0;
    #[cfg(windows)]
    const TICKS: f64 = 1_000.0;
    unsafe { clock() as f64 / TICKS }
}

fn main() {
    for (name, capacity, active, tracks) in [
        ("idle-live", 16, 0, false),
        ("chord-live", 16, 8, false),
        ("sparse-bounce", 64, 4, true),
        ("dense-bounce", 64, 32, true),
    ] {
        let mut vm = VoiceManager::new(48000.0, capacity);
        vm.warm_up();
        for i in 0..active {
            let ch = if tracks { i as u16 + 1 } else { 0 };
            if tracks {
                vm.set_channel_params(ch, ParamValues::default());
            }
            vm.note_on_channel(36 + i as u8, 0.7, ch);
        }
        for _ in 0..4800 {
            black_box(vm.render_next());
        }
        let cpu_start = cpu_seconds();
        let start = Instant::now();
        let frames = 96000;
        for _ in 0..frames {
            black_box(vm.render_next());
        }
        let cpu_ns = (cpu_seconds() - cpu_start) * 1e9 / frames as f64;
        println!("{name}: {cpu_ns:.1} CPU ns/frame");
        println!(
            "{name}: {:.1} ns/frame ({:.1}x realtime)",
            start.elapsed().as_nanos() as f64 / frames as f64,
            2.0 / start.elapsed().as_secs_f64()
        );
    }
}
