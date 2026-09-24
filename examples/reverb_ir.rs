//! Write the reverb's stereo impulse response (wet only) for measurement:
//!   cargo run --release --no-default-features --example reverb_ir -- OUT.wav DECAY TONE [left|right|both]
use patina::reverb::Reverb;
use std::io::Write;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let sr = 48000.0;
    let mut r = Reverb::new(sr);
    r.set_decay(a[2].parse().unwrap());
    r.set_tone(a[3].parse().unwrap());
    r.set_wet(1.0);
    r.set_pre(0.0);
    let side = a.get(4).map(|s| s.as_str()).unwrap_or("both");
    let n = (sr * 6.0) as usize;
    let mut f = std::io::BufWriter::new(std::fs::File::create(&a[1]).unwrap());
    let data = (n * 8) as u32;
    for (b, v) in [(b"RIFF", 36 + data), (b"WAVE", 0)] {
        f.write_all(b).unwrap();
        if v != 0 { f.write_all(&v.to_le_bytes()).unwrap(); }
    }
    f.write_all(b"fmt ").unwrap();
    for v in [16u32] { f.write_all(&v.to_le_bytes()).unwrap(); }
    for v in [3u16, 2] { f.write_all(&v.to_le_bytes()).unwrap(); }
    for v in [48000u32, 48000 * 8] { f.write_all(&v.to_le_bytes()).unwrap(); }
    for v in [8u16, 32] { f.write_all(&v.to_le_bytes()).unwrap(); }
    f.write_all(b"data").unwrap();
    f.write_all(&data.to_le_bytes()).unwrap();
    for i in 0..n {
        let x = if i == 0 { 1.0 } else { 0.0 };
        let (l, rr) = match side { "left" => (x, 0.0), "right" => (0.0, x), _ => (x, x) };
        let (ol, or) = r.process(l, rr);
        f.write_all(&(ol - l).to_le_bytes()).unwrap();
        f.write_all(&(or - rr).to_le_bytes()).unwrap();
    }
}
