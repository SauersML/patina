//! Bit-identity check: renders every song in songs/ through the offline
//! engine and prints one FNV-1a hash of its sample bits per song, so a
//! change that should leave existing songs alone can prove it.
//!
//! How to run: compare two builds' song hashes by running
//! `HASH_OUT=/path/before.txt cargo test --release --test hash_songs -- --ignored --nocapture`
//! in a checkout of each (say, main and your change), then diff the files.
//!
//! HASH_ONLY=a.song,b.song limits it to those songs, HASH_THREADS sets how
//! many render at once (default 4), and HASH_OUT writes the sorted lines to
//! a file. Each line is `name hash frames` (the progress print adds the
//! render time), or `name ERR reason` for a song that cannot load (a
//! missing WAV, say).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

fn fnv(h: &mut u64, x: u32) {
    for b in x.to_le_bytes() {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x100000001b3);
    }
}

#[test]
#[ignore]
fn hash_songs() {
    let only = std::env::var("HASH_ONLY").ok();
    let mut names: Vec<String> = std::fs::read_dir("songs")
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".song"))
        .filter(|n| only.as_ref().map_or(true, |o| o.split(',').any(|x| x == n)))
        .collect();
    names.sort();
    let next = AtomicUsize::new(0);
    let out = Mutex::new(Vec::new());
    let threads: usize = std::env::var("HASH_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= names.len() {
                    break;
                }
                let name = &names[i];
                let t0 = std::time::Instant::now();
                let line = match patina::song::load_song(&format!("songs/{}", name)) {
                    Ok(song) => {
                        let mut h = 0xcbf29ce484222325u64;
                        let mut n = 0usize;
                        for (l, r) in patina::song::render_offline(&song, 48000.0) {
                            fnv(&mut h, l.to_bits());
                            fnv(&mut h, r.to_bits());
                            n += 1;
                        }
                        format!("{} {:016x} {}", name, h, n)
                    }
                    Err(e) => format!("{} ERR {}", name, e.lines().next().unwrap_or("")),
                };
                eprintln!("{} {:.1}s", line, t0.elapsed().as_secs_f64());
                out.lock().unwrap().push(line);
            });
        }
    });
    if let Ok(path) = std::env::var("HASH_OUT") {
        let mut lines = out.into_inner().unwrap();
        lines.sort();
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    }
}
