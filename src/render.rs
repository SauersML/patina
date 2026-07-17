// Offline song rendering: `patina --play song --render out.wav` bounces a
// song through the full engine faster than realtime and writes a stereo
// 32-bit float WAV — analysis-grade, so even the tape noise floor (well
// below the 16-bit LSB) survives the file format.

use std::fs::File;
use std::io::{BufWriter, Result, Write};

pub fn render_to_wav(events: &[crate::song::SongEvent], path: &str) -> Result<()> {
    let sample_rate = 48000.0f32;
    println!("Rendering {} events...", events.len());
    let start = std::time::Instant::now();
    let frames = crate::song::render_offline(events, sample_rate);
    let audio_seconds = frames.len() as f32 / sample_rate;
    println!(
        "Rendered {:.1}s of audio in {:.2}s ({:.1}x realtime)",
        audio_seconds,
        start.elapsed().as_secs_f32(),
        audio_seconds / start.elapsed().as_secs_f32().max(1e-6),
    );
    write_wav(path, &frames, sample_rate as u32)?;
    println!("Wrote {}", path);
    Ok(())
}

fn write_wav(path: &str, frames: &[(f32, f32)], sample_rate: u32) -> Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    let data_len = (frames.len() * 8) as u32; // 2 channels x 4 bytes

    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_len).to_le_bytes())?;
    w.write_all(b"WAVE")?;

    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&3u16.to_le_bytes())?; // IEEE float
    w.write_all(&2u16.to_le_bytes())?; // stereo
    w.write_all(&sample_rate.to_le_bytes())?;
    w.write_all(&(sample_rate * 8).to_le_bytes())?; // byte rate
    w.write_all(&8u16.to_le_bytes())?; // block align
    w.write_all(&32u16.to_le_bytes())?; // bits per sample

    w.write_all(b"data")?;
    w.write_all(&data_len.to_le_bytes())?;
    for &(l, r) in frames {
        w.write_all(&l.to_le_bytes())?;
        w.write_all(&r.to_le_bytes())?;
    }
    Ok(())
}
