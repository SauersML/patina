// Offline song rendering: `patina --play song --render out.wav` bounces a
// song through the full engine faster than realtime and writes a stereo
// 32-bit float WAV — analysis-grade, so even the tape noise floor (well
// below the 16-bit LSB) survives the file format.

use std::fs::File;
use std::io::{BufWriter, Result, Write};

/// One wav per track channel, soloed through the same engine: what each
/// instrument contributed, with its own sends ringing in the shared tanks.
/// Channels that share a strip (all `kit=` tracks, all sampler tracks)
/// bounce once under the first track's name.
pub fn render_stems(song: &crate::song::Song, dir: &str) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut done: Vec<u16> = Vec::new();
    for (name, channel) in &song.tracks {
        // group channels that mix as one strip
        let key = if *channel == crate::drums::DRUM_CHANNEL {
            crate::drums::DRUM_CHANNEL
        } else if *channel >= crate::sampler::SAMPLER_CHANNEL_BASE {
            crate::sampler::SAMPLER_CHANNEL_BASE
        } else {
            *channel
        };
        if done.contains(&key) {
            continue;
        }
        done.push(key);
        let path = format!("{}/{}.wav", dir.trim_end_matches('/'), name);
        println!("stem: {} (channel {})", path, key);
        let mut frames = crate::song::render_offline_solo(song, 48000.0, Some(key));
        let peak = frames
            .iter()
            .fold(0.0f32, |a, &(l, r)| a.max(l.abs()).max(r.abs()));
        if peak > 1e-6 {
            // one shared gain across stems would preserve the mix, but a
            // stem is a working file: give each the medium's headroom
            let gain = 0.891 / peak;
            for (l, r) in &mut frames {
                *l *= gain;
                *r *= gain;
            }
        }
        write_wav(&path, &frames, 48000)?;
    }
    Ok(())
}

/// The parsed song as JSON: exact event times in seconds (post tempo
/// map), note/param/channel payloads, and the track name map — so a
/// visualization never needs its own .song parser to stay honest.
pub fn export_events(song: &crate::song::Song, path: &str) -> Result<()> {
    use std::io::Write;
    let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
    writeln!(w, "{{")?;
    writeln!(w, "  \"tracks\": {{")?;
    for (i, (name, ch)) in song.tracks.iter().enumerate() {
        let comma = if i + 1 < song.tracks.len() { "," } else { "" };
        writeln!(w, "    \"{}\": {}{}", name, ch, comma)?;
    }
    writeln!(w, "  }},")?;
    writeln!(w, "  \"events\": [")?;
    let n = song.events.len();
    for (i, e) in song.events.iter().enumerate() {
        let body = match &e.kind {
            crate::song::EventKind::NoteOn { note, velocity, channel } => format!(
                "\"type\":\"on\",\"note\":{},\"vel\":{:.4},\"ch\":{}",
                note, velocity, channel
            ),
            crate::song::EventKind::NoteOff { note, channel } => {
                format!("\"type\":\"off\",\"note\":{},\"ch\":{}", note, channel)
            }
            crate::song::EventKind::Param { param, value, channel } => format!(
                "\"type\":\"param\",\"param\":\"{:?}\",\"value\":{:.6},\"ch\":{}",
                param, value, channel
            ),
            crate::song::EventKind::Lyric { channel, .. } => {
                format!("\"type\":\"lyric\",\"ch\":{}", channel)
            }
        };
        let comma = if i + 1 < n { "," } else { "" };
        writeln!(w, "    {{\"t\":{:.6},{}}}{}", e.time, body, comma)?;
    }
    writeln!(w, "  ]")?;
    writeln!(w, "}}")?;
    println!("Wrote {} ({} events)", path, n);
    Ok(())
}

pub fn render_to_wav(song: &crate::song::Song, path: &str) -> Result<()> {
    let sample_rate = 48000.0f32;
    println!("Rendering {} events...", song.events.len());
    let start = std::time::Instant::now();
    let mut frames = crate::song::render_offline(song, sample_rate);

    // Master to -1 dBFS peak: the engine's gain staging is patch-dependent,
    // and a bounce should use the medium's headroom
    let peak = frames
        .iter()
        .fold(0.0f32, |a, &(l, r)| a.max(l.abs()).max(r.abs()));
    if peak > 1e-6 {
        let gain = 0.891 / peak; // -1 dBFS
        for (l, r) in &mut frames {
            *l *= gain;
            *r *= gain;
        }
        println!("Normalized: peak {:.3} -> -1 dBFS ({:+.1} dB)", peak, 20.0 * gain.log10());
    }
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
