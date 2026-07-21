// Offline song rendering: `patina --play song --render out.wav` bounces a
// song through the full engine faster than realtime and writes a stereo
// 32-bit float WAV — analysis-grade, so even the tape noise floor (well
// below the 16-bit LSB) survives the file format.

use std::fs::File;
use std::io::{BufWriter, Result, Write};

/// Peak in dBFS.
fn peak_db(frames: &[(f32, f32)]) -> f32 {
    let peak = frames
        .iter()
        .fold(0.0f32, |a, &(l, r)| a.max(l.abs()).max(r.abs()));
    20.0 * peak.max(1e-9).log10()
}

/// Whole-file RMS in dBFS (both channels pooled).
fn rms_db(frames: &[(f32, f32)]) -> f32 {
    let ms = frames
        .iter()
        .map(|&(l, r)| (l as f64 * l as f64 + r as f64 * r as f64) * 0.5)
        .sum::<f64>()
        / frames.len().max(1) as f64;
    10.0 * (ms.max(1e-18)).log10() as f32
}

/// Integrated loudness per ITU-R BS.1770-4: K-weighting (high shelf +
/// RLB high-pass, the spec's 48 kHz coefficients — the render rate),
/// 400 ms blocks at 75% overlap, -70 LUFS absolute gate, then a -10 LU
/// relative gate. Returns LUFS (or -inf-ish for silence).
fn lufs_integrated(frames: &[(f32, f32)]) -> f32 {
    const B1: [f64; 3] = [1.53512485958697, -2.69169618940638, 1.19839281085285];
    const A1: [f64; 2] = [-1.69065929318241, 0.73248077421585];
    const B2: [f64; 3] = [1.0, -2.0, 1.0];
    const A2: [f64; 2] = [-1.99004745483398, 0.99007225036621];
    // K-weight both channels, accumulate per-sample weighted square sum
    let mut sq = vec![0.0f64; frames.len()];
    for ch in 0..2 {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0f64, 0.0, 0.0, 0.0);
        let (mut u1, mut u2, mut v1, mut v2) = (0.0f64, 0.0, 0.0, 0.0);
        for (i, &(l, r)) in frames.iter().enumerate() {
            let x = if ch == 0 { l } else { r } as f64;
            let y = B1[0] * x + B1[1] * x1 + B1[2] * x2 - A1[0] * y1 - A1[1] * y2;
            x2 = x1;
            x1 = x;
            y2 = y1;
            y1 = y;
            let z = B2[0] * y + B2[1] * u1 + B2[2] * u2 - A2[0] * v1 - A2[1] * v2;
            u2 = u1;
            u1 = y;
            v2 = v1;
            v1 = z;
            sq[i] += z * z;
        }
    }
    let block = 19200; // 400 ms at 48 kHz
    let hop = block / 4;
    if sq.len() < block {
        return -70.0;
    }
    let loudness = |ms: f64| -0.691 + 10.0 * ms.max(1e-18).log10();
    let blocks: Vec<f64> = (0..=(sq.len() - block) / hop)
        .map(|k| sq[k * hop..k * hop + block].iter().sum::<f64>() / block as f64)
        .collect();
    let gated: Vec<f64> = blocks.iter().copied().filter(|&m| loudness(m) > -70.0).collect();
    if gated.is_empty() {
        return -70.0;
    }
    let thresh = loudness(gated.iter().sum::<f64>() / gated.len() as f64) - 10.0;
    let final_set: Vec<f64> = gated.into_iter().filter(|&m| loudness(m) > thresh).collect();
    if final_set.is_empty() {
        return -70.0;
    }
    loudness(final_set.iter().sum::<f64>() / final_set.len() as f64) as f32
}

/// One wav per track channel, soloed through the same engine: what each
/// instrument contributed, with its own sends ringing in the shared tanks.
/// Channels that share a strip (all `kit=` tracks, all sampler tracks)
/// bounce once under the first track's name.
///
/// Stems are NOT normalized — they are measurement files, written at the
/// exact gain the mix hears, and each is reported as a level-table row
/// (peak / RMS / LUFS) so measured mixing needs no hand math.
pub fn render_stems(song: &crate::song::Song, dir: &str) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut done: Vec<u16> = Vec::new();
    let mut table: Vec<(String, f32, f32, f32)> = Vec::new();
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
        let frames = crate::song::render_offline_solo(song, 48000.0, Some(key));
        table.push((name.clone(), peak_db(&frames), rms_db(&frames), lufs_integrated(&frames)));
        write_wav(&path, &frames, 48000)?;
    }
    println!("\n{:<16} {:>10} {:>10} {:>10}", "stem", "peak dBFS", "rms dBFS", "LUFS");
    for (name, peak, rms, lufs) in &table {
        println!("{:<16} {:>10.1} {:>10.1} {:>10.1}", name, peak, rms, lufs);
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

pub fn render_to_wav(song: &crate::song::Song, path: &str, normalize: bool) -> Result<()> {
    let sample_rate = 48000.0f32;
    println!("Rendering {} events...", song.events.len());
    let start = std::time::Instant::now();
    let mut frames = crate::song::render_offline(song, sample_rate);

    // Master to -1 dBFS peak: the engine's gain staging is patch-dependent,
    // and a bounce should use the medium's headroom. `--no-normalize`
    // keeps the engine's exact gain (measurement renders, A/B at matched
    // gain against stems).
    let peak = frames
        .iter()
        .fold(0.0f32, |a, &(l, r)| a.max(l.abs()).max(r.abs()));
    if normalize && peak > 1e-6 {
        let gain = 0.891 / peak; // -1 dBFS
        for (l, r) in &mut frames {
            *l *= gain;
            *r *= gain;
        }
        println!("Normalized: peak {:.3} -> -1 dBFS ({:+.1} dB)", peak, 20.0 * gain.log10());
    }
    println!(
        "Levels: peak {:.1} dBFS, rms {:.1} dBFS, {:.1} LUFS",
        peak_db(&frames),
        rms_db(&frames),
        lufs_integrated(&frames)
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    /// BS.1770 calibration: a 997 Hz sine at -18 dBFS peak in BOTH
    /// channels must read -18.0 LUFS — the -0.691 offset exists to
    /// cancel the K-weighting's +0.69 dB at 1 kHz, and the two channels'
    /// half-power mean squares sum back to the single-channel figure.
    #[test]
    fn lufs_reads_the_reference_tone() {
        let amp = 10f32.powf(-18.0 / 20.0);
        let frames: Vec<(f32, f32)> = (0..480000)
            .map(|i| {
                let s = (std::f32::consts::TAU * 997.0 * i as f32 / 48000.0).sin() * amp;
                (s, s)
            })
            .collect();
        let l = lufs_integrated(&frames);
        assert!((l - (-18.0)).abs() < 0.1, "reference tone read {l} LUFS");
        assert!((peak_db(&frames) - (-18.0)).abs() < 0.1);
        assert!((rms_db(&frames) - (-21.0)).abs() < 0.1, "sine rms is peak - 3 dB");
        // and silence gates out instead of returning garbage
        assert_eq!(lufs_integrated(&vec![(0.0, 0.0); 96000]), -70.0);
    }
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
