// Offline song rendering: `patina --play song --render out.wav` bounces a
// song through the full engine faster than realtime and writes a stereo
// 32-bit float WAV — analysis-grade, so even the tape noise floor (well
// below the 16-bit LSB) survives the file format.

use std::fs::OpenOptions;
use std::io::{BufWriter, Error, ErrorKind, Read, Result, Seek, SeekFrom, Write};

/// Streaming BS.1770 meter at the WAV render rate (48 kHz). Keep four
/// 100 ms energy sums, then one number per overlapping 400 ms block.
/// A one-hour bounce needs ~288 KB of block history instead of 1.38 GB
/// of weighted samples. Block energies retain exact two-stage gating.
#[derive(Default)]
struct Meter {
    filters: [[f64; 8]; 2],
    hop_energy: f64,
    hops: [f64; 4],
    completed_hops: usize,
    frames: usize,
    peak: f32,
    square_sum: f64,
    blocks: Vec<f64>,
}

impl Meter {
    fn push(&mut self, (l, r): (f32, f32)) {
        const B1: [f64; 3] = [1.53512485958697, -2.69169618940638, 1.19839281085285];
        const A1: [f64; 2] = [-1.69065929318241, 0.73248077421585];
        const A2: [f64; 2] = [-1.99004745483398, 0.99007225036621];
        self.peak = self.peak.max(l.abs()).max(r.abs());
        self.square_sum += (l as f64 * l as f64 + r as f64 * r as f64) * 0.5;
        let mut energy = 0.0;
        for (input, state) in [l, r].into_iter().zip(&mut self.filters) {
            let [x1, x2, y1, y2, u1, u2, v1, v2] = *state;
            let x = input as f64;
            let y = B1[0] * x + B1[1] * x1 + B1[2] * x2 - A1[0] * y1 - A1[1] * y2;
            let z = y - 2.0 * u1 + u2 - A2[0] * v1 - A2[1] * v2;
            *state = [x, x1, y, y1, y, u1, z, v1];
            energy += z * z;
        }
        self.hop_energy += energy;
        self.frames += 1;
        if self.frames % 4800 == 0 {
            self.hops[self.completed_hops % 4] = self.hop_energy;
            self.hop_energy = 0.0;
            self.completed_hops += 1;
            if self.completed_hops >= 4 {
                self.blocks.push(self.hops.iter().sum::<f64>() / 19200.0);
            }
        }
    }

    fn levels(&self, gain: f32) -> (f32, f32, f32) {
        let gain_sq = gain as f64 * gain as f64;
        let peak = 20.0 * (self.peak * gain).max(1e-9).log10();
        let rms = 10.0
            * (self.square_sum * gain_sq / self.frames.max(1) as f64)
                .max(1e-18)
                .log10() as f32;
        let loudness = |energy: f64| -0.691 + 10.0 * energy.max(1e-18).log10();
        let (sum, count) = self.blocks.iter().fold((0.0, 0usize), |(sum, n), &m| {
            let m = m * gain_sq;
            if loudness(m) > -70.0 {
                (sum + m, n + 1)
            } else {
                (sum, n)
            }
        });
        if count == 0 {
            return (peak, rms, -70.0);
        }
        let threshold = (loudness(sum / count as f64) - 10.0).max(-70.0);
        let (sum, count) = self.blocks.iter().fold((0.0, 0usize), |(sum, n), &m| {
            let m = m * gain_sq;
            if loudness(m) > threshold {
                (sum + m, n + 1)
            } else {
                (sum, n)
            }
        });
        let lufs = if count == 0 {
            -70.0
        } else {
            loudness(sum / count as f64) as f32
        };
        (peak, rms, lufs)
    }
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
        let mut frames = crate::song::render_offline_solo(song, 48000.0, Some(key));
        let (peak, rms, lufs) = write_wav(&path, &mut frames, false)?;
        println!("peak concurrent voices: {}/64", frames.peak_voices());
        table.push((name.clone(), peak, rms, lufs));
    }
    println!(
        "\n{:<16} {:>10} {:>10} {:>10}",
        "stem", "peak dBFS", "rms dBFS", "LUFS"
    );
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
            crate::song::EventKind::NoteOn {
                note,
                velocity,
                channel,
            } => format!(
                "\"type\":\"on\",\"note\":{},\"vel\":{:.4},\"ch\":{}",
                note, velocity, channel
            ),
            crate::song::EventKind::NoteOff { note, channel } => {
                format!("\"type\":\"off\",\"note\":{},\"ch\":{}", note, channel)
            }
            crate::song::EventKind::Param {
                param,
                value,
                channel,
            } => format!(
                "\"type\":\"param\",\"param\":\"{:?}\",\"value\":{:.6},\"ch\":{}",
                param, value, channel
            ),
            crate::song::EventKind::VoxLead { note, .. } => {
                format!("\"type\":\"lyric\",\"note\":{}", note)
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
    println!("Rendering {} events...", song.events.len());
    let start = std::time::Instant::now();
    let mut frames = crate::song::render_offline(song, 48000.0);
    let seconds = frames.len() as f64 / 48000.0;
    let (peak, rms, lufs) = write_wav(path, &mut frames, normalize)?;
    println!("peak concurrent voices: {}/64", frames.peak_voices());
    println!("Levels: peak {peak:.1} dBFS, rms {rms:.1} dBFS, {lufs:.1} LUFS");
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "Rendered {seconds:.1}s of audio in {elapsed:.2}s ({:.1}x realtime)",
        seconds / elapsed.max(1e-6)
    );
    println!("Wrote {}", path);
    Ok(())
}

/// Write the raw bounce once. Peak normalization then scales the file in
/// bounded chunks, preserving the single stochastic engine performance.
fn write_wav(
    path: &str,
    frames: impl ExactSizeIterator<Item = (f32, f32)>,
    normalize: bool,
) -> Result<(f32, f32, f32)> {
    let data_len = frames
        .len()
        .checked_mul(8)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n <= u32::MAX - 36)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                "bounce exceeds the RIFF WAV size limit",
            )
        })?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    let mut meter = Meter::default();
    {
        let mut w = BufWriter::with_capacity(64 * 1024, &mut file);
        w.write_all(b"RIFF")?;
        w.write_all(&(36 + data_len).to_le_bytes())?;
        w.write_all(b"WAVEfmt ")?;
        w.write_all(&16u32.to_le_bytes())?;
        w.write_all(&3u16.to_le_bytes())?;
        w.write_all(&2u16.to_le_bytes())?;
        w.write_all(&48000u32.to_le_bytes())?;
        w.write_all(&(48000u32 * 8).to_le_bytes())?;
        w.write_all(&8u16.to_le_bytes())?;
        w.write_all(&32u16.to_le_bytes())?;
        w.write_all(b"data")?;
        w.write_all(&data_len.to_le_bytes())?;
        for frame in frames {
            meter.push(frame);
            w.write_all(&frame.0.to_le_bytes())?;
            w.write_all(&frame.1.to_le_bytes())?;
        }
        w.flush()?;
    }
    let gain = if normalize && meter.peak > 1e-6 {
        let gain = 0.891 / meter.peak;
        let mut chunk = [0u8; 64 * 1024];
        let mut offset = 44u64;
        let end = offset + data_len as u64;
        while offset < end {
            let n = ((end - offset) as usize).min(chunk.len());
            file.seek(SeekFrom::Start(offset))?;
            file.read_exact(&mut chunk[..n])?;
            for bytes in chunk[..n].chunks_exact_mut(4) {
                let sample = f32::from_le_bytes(bytes.try_into().unwrap()) * gain;
                bytes.copy_from_slice(&sample.to_le_bytes());
            }
            file.seek(SeekFrom::Start(offset))?;
            file.write_all(&chunk[..n])?;
            offset += n as u64;
        }
        file.flush()?;
        println!(
            "Normalized: peak {:.3} -> -1 dBFS ({:+.1} dB)",
            meter.peak,
            20.0 * gain.log10()
        );
        gain
    } else {
        1.0
    };
    Ok(meter.levels(gain))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meter(frames: &[(f32, f32)]) -> Meter {
        let mut meter = Meter::default();
        for &frame in frames {
            meter.push(frame);
        }
        meter
    }

    #[test]
    fn lufs_reads_the_reference_tone() {
        let amp = 10f32.powf(-18.0 / 20.0);
        let frames: Vec<_> = (0..480000)
            .map(|i| {
                let s = (std::f32::consts::TAU * 997.0 * i as f32 / 48000.0).sin() * amp;
                (s, s)
            })
            .collect();
        let (peak, rms, lufs) = meter(&frames).levels(1.0);
        assert!((lufs + 18.0).abs() < 0.1, "reference tone read {lufs} LUFS");
        assert!((peak + 18.0).abs() < 0.1);
        assert!((rms + 21.0).abs() < 0.1);
        assert_eq!(meter(&vec![(0.0, 0.0); 96000]).levels(1.0).2, -70.0);
    }

    #[test]
    fn streaming_loudness_matches_full_buffer_gating() {
        // Loud passages, quiet passages, silence, asymmetric stereo, and
        // a partial last hop exercise both gates and window boundaries.
        let frames: Vec<_> = (0..480000 + 173)
            .map(|i| {
                let amp = [0.0, 0.25, 0.00005, 0.01, 0.8][i / 96000 % 5];
                let x = (i as f32 * 0.12).sin() * amp;
                (x, x * 0.3)
            })
            .collect();
        let meter = meter(&frames);
        assert_eq!(meter.blocks.len(), (frames.len() - 19200) / 4800 + 1);
        for gain in [0.01, 1.0, 10.0] {
            let scaled: Vec<_> = frames.iter().map(|&(l, r)| (l * gain, r * gain)).collect();
            let expected = reference_lufs(&scaled);
            assert!((meter.levels(gain).2 - expected).abs() < 0.0001);
        }
        assert!(Meter::default().blocks.is_empty());
    }

    #[test]
    fn wav_stream_preserves_samples_and_normalizes_across_chunks() {
        let frames: Vec<_> = (0..17003)
            .map(|i| ((i as f32 * 0.07).sin() * 0.4, (i as f32 * 0.11).cos() * 0.2))
            .collect();
        let peak = meter(&frames).peak;
        for normalize in [false, true] {
            let path = std::env::temp_dir().join(format!(
                "patina-stream-{}-{normalize}.wav",
                std::process::id()
            ));
            write_wav(path.to_str().unwrap(), frames.iter().copied(), normalize).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            std::fs::remove_file(path).unwrap();
            assert_eq!(bytes.len(), 44 + frames.len() * 8);
            assert_eq!(&bytes[..4], b"RIFF");
            assert_eq!(
                u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize,
                frames.len() * 8
            );
            let gain = if normalize { 0.891 / peak } else { 1.0 };
            for (data, &(l, r)) in bytes[44..].chunks_exact(8).zip(&frames) {
                assert_eq!(f32::from_le_bytes(data[..4].try_into().unwrap()), l * gain);
                assert_eq!(f32::from_le_bytes(data[4..].try_into().unwrap()), r * gain);
            }
        }
    }

    #[test]
    fn oversized_wav_is_rejected_before_opening_output() {
        let frames = (0..u32::MAX as usize).map(|_| (0.0, 0.0));
        let error = write_wav("/no-such-directory/oversized.wav", frames, false).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }
    fn reference_lufs(frames: &[(f32, f32)]) -> f32 {
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
        let gated: Vec<f64> = blocks
            .iter()
            .copied()
            .filter(|&m| loudness(m) > -70.0)
            .collect();
        if gated.is_empty() {
            return -70.0;
        }
        let thresh = loudness(gated.iter().sum::<f64>() / gated.len() as f64) - 10.0;
        let final_set: Vec<f64> = gated
            .into_iter()
            .filter(|&m| loudness(m) > thresh)
            .collect();
        if final_set.is_empty() {
            return -70.0;
        }
        loudness(final_set.iter().sum::<f64>() / final_set.len() as f64) as f32
    }
}
