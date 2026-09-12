//! Headless live-engine allocation check; no audio device, window, or WAV.
//! cargo run --release --no-default-features -j 1 --example live_budget
use patina::{drums::DRUM_CHANNEL, oscillator::CircuitModel, voice_manager::VoiceManager};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
    time::Instant,
};

struct CountedAllocator;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            ALLOCATIONS.fetch_add(1, Relaxed);
            LIVE_BYTES.fetch_add(layout.size(), Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_BYTES.fetch_sub(layout.size(), Relaxed);
        System.dealloc(ptr, layout);
    }

    unsafe fn realloc(&self, ptr: *mut u8, old: Layout, size: usize) -> *mut u8 {
        let new = System.realloc(ptr, old, size);
        if !new.is_null() {
            ALLOCATIONS.fetch_add(1, Relaxed);
            LIVE_BYTES.fetch_add(size, Relaxed);
            LIVE_BYTES.fetch_sub(old.size(), Relaxed);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: CountedAllocator = CountedAllocator;

fn main() {
    for rate in [48_000.0, 96_000.0] {
        let bytes_before = LIVE_BYTES.load(Relaxed);
        let start = Instant::now();
        let mut vm = VoiceManager::new(rate, 10);
        let startup = start.elapsed();
        let engine_bytes = LIVE_BYTES.load(Relaxed) - bytes_before;
        let allocations_before = ALLOCATIONS.load(Relaxed);
        let mut peak = 0.0f32;
        let mut fingerprint = 0u64;

        // Include the very first callback, then notes, stealing, sustain,
        // drum triggers and control automation. Do not warm allocation paths
        // before measuring: the first key press must also be allocation free.
        for block in 0..400 {
            if block % 12 == 1 {
                vm.note_on(48 + ((block / 12) % 24) as u8, 0.8);
                vm.note_on_channel(36, 0.7, DRUM_CHANNEL);
            }
            if block % 12 == 7 {
                vm.note_off(48 + ((block / 12) % 24) as u8);
            }
            vm.set_sustain_pedal(block % 96 < 72);
            vm.set_pitch_bend((block % 32) as f32 / 16.0 - 1.0);
            vm.set_filter_cutoff(500.0 + (block % 40) as f32 * 100.0);
            vm.set_filter_drive(0.1 + (block % 40) as f32 * 0.2);
            vm.set_hpf_cutoff(16.0 + (block % 40) as f32 * 10.0);
            vm.set_circuit(if block < 200 {
                CircuitModel::Moog
            } else {
                CircuitModel::Arp
            });
            for _ in 0..128 {
                let (l, r) = black_box(vm.render_next());
                assert!(l.is_finite() && r.is_finite());
                peak = peak.max(l.abs()).max(r.abs());
                fingerprint =
                    fingerprint.rotate_left(7) ^ ((l.to_bits() as u64) << 32 | r.to_bits() as u64);
            }
        }
        vm.set_sustain_pedal(false);
        for note in 0..128 {
            vm.note_off(note);
        }
        for _ in 0..rate as usize {
            black_box(vm.render_next());
        }
        let allocations = ALLOCATIONS.load(Relaxed) - allocations_before;
        let growth = LIVE_BYTES.load(Relaxed) - bytes_before - engine_bytes;
        assert!(peak > 0.001, "workload must produce audio");
        assert_eq!(allocations, 0, "live playback allocated");
        assert_eq!(growth, 0, "live playback grew the heap");
        println!(
            "{rate:.0} Hz / 10 voices: startup {:.2} ms, engine heap {engine_bytes} bytes, \
             playback allocations {allocations}, heap growth {growth} bytes, \
             audio fingerprint {fingerprint:016x}",
            startup.as_secs_f64() * 1000.0
        );
    }
}
