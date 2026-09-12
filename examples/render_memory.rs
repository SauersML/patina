//! Measure long-bounce memory with /usr/bin/time -l (macOS) or -v (Linux):
//! cargo run --release --no-default-features --example render_memory -- 60 /tmp/bounce.wav
use patina::{render::render_to_wav, song::parse_song_text};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3, "usage: render_memory SECONDS OUTPUT.wav");
    let seconds: f64 = args[1].parse().expect("seconds must be a number");
    assert!(seconds.is_finite() && seconds >= 1.0);
    let song = parse_song_text(&format!(
        "bpm 60\ntail 0\ntrack tone\nC4:0.25\nautomate volume\n>{seconds} 0\n"
    ))
    .expect("benchmark song must parse");
    render_to_wav(&song, &args[2], true).expect("WAV render failed");
}
