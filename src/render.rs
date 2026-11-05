// Offline song rendering: `patina --play song --render out.wav` bounces a
// song through the full engine faster than realtime and writes a stereo
// 32-bit float WAV — analysis-grade, so even the tape noise floor (well
// below the 16-bit LSB) survives the file format. `--bits 16` writes
// dithered 16-bit PCM instead, for delivery.

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

/// What `--render`, `--render-stems` and `--say` write. Float32 is the
/// default: analysis-grade, the tape noise floor survives. Pcm16 is the
/// delivery format, half the size and readable by every tool: TPDF
/// dither, applied once after normalization, so the bounce is quantized
/// exactly once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SampleFormat {
    Float32,
    Pcm16,
}

/// One wav per track channel, soloed through the same engine: what each
/// instrument contributed, with its own sends ringing in the shared tanks.
/// Tracks that share a channel (vox tracks, a `kit=<track>` playing an
/// earlier kit track's board) bounce once under the first track's name.
///
/// Stems are NOT normalized — they are measurement files, written at the
/// exact gain the mix hears, and each is reported as a level-table row
/// (peak / RMS / LUFS) so measured mixing needs no hand math.
pub fn render_stems(song: &crate::song::Song, dir: &str, format: SampleFormat) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    // One job per strip: every track is its own strip, sample tracks and
    // kit tracks included (each tape slot and each 909 board mixes on its
    // own channel). Sample tracks used to be folded onto the deck's base
    // channel, which a solo matches for slot 0 alone: the first sample
    // track's stem was written and every other sample track silently got
    // none; every kit track used to share one board and one stem.
    let mut jobs: Vec<(String, u16)> = Vec::new();
    for (name, channel) in &song.tracks {
        // (`bus mix` is the whole mix, not a stem; a group bus's stem is
        // its tracks through it)
        if jobs.iter().any(|(_, k)| k == channel)
            || *channel == crate::voice_manager::MIX_CHANNEL
        {
            continue;
        }
        jobs.push((name.clone(), *channel));
    }
    // Each stem is a whole independent bounce, so they run side by side,
    // one per core (each renders its cards serially); results are reported
    // in track order whatever order they finish in.
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(jobs.len())
        .max(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<std::sync::Mutex<Option<Result<(f32, f32, f32, usize)>>>> =
        jobs.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                crate::voice_manager::prefer_performance_cores();
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((name, key)) = jobs.get(i) else {
                        break;
                    };
                    let path = format!("{}/{}.wav", dir.trim_end_matches('/'), name);
                    let mut frames = crate::song::render_offline_solo(song, 48000.0, Some(*key));
                    let out = write_wav(&path, &mut frames, false, format)
                        .map(|(peak, rms, lufs)| (peak, rms, lufs, frames.peak_voices()));
                    // every stem plays every note, so every stem loses the
                    // same voices: the first stem says so for the song
                    if i == 0 {
                        for w in frames.steal_warnings() {
                            eprintln!("warning: {}", w);
                        }
                    }
                    *results[i].lock().unwrap() = Some(out);
                }
            });
        }
    });
    // A solo bounce mutes the other strips but still plays every voice
    // (the 64-board pool and its stealing must match the mix), so the
    // voice peak is the whole song's, identical for every stem: report
    // it once, not as if it were each stem's own.
    let mut table: Vec<(String, f32, f32, f32)> = Vec::new();
    let mut song_voices = 0;
    for ((name, key), result) in jobs.iter().zip(results) {
        let (peak, rms, lufs, voices) = result.into_inner().unwrap().expect("every stem ran")?;
        println!(
            "stem: {}/{}.wav (channel {})",
            dir.trim_end_matches('/'),
            name,
            key
        );
        song_voices = song_voices.max(voices);
        table.push((name.clone(), peak, rms, lufs));
    }
    println!("peak concurrent voices (whole song): {}/64", song_voices);
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
            crate::song::EventKind::Bend {
                note,
                channel,
                semitones,
                seconds,
            } => format!(
                "\"type\":\"bend\",\"note\":{},\"semitones\":{:.4},\"secs\":{:.6},\"ch\":{}",
                note, semitones, seconds, channel
            ),
            crate::song::EventKind::Clock { beat, bpm } => {
                format!("\"type\":\"clock\",\"beat\":{:.6},\"bpm\":{:.6}", beat, bpm)
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

pub fn render_to_wav(
    song: &crate::song::Song,
    path: &str,
    normalize: bool,
    format: SampleFormat,
) -> Result<()> {
    crate::voice_manager::prefer_performance_cores();
    println!("Rendering {} events...", song.events.len());
    let start = std::time::Instant::now();
    let mut frames =
        crate::song::render_offline(song, 48000.0).with_card_lanes(crate::song::card_lanes());
    let seconds = frames.len() as f64 / 48000.0;
    let (peak, rms, lufs) = write_wav(path, &mut frames, normalize, format)?;
    for w in frames.steal_warnings() {
        eprintln!("warning: {}", w);
    }
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

/// Write the raw bounce once, as float. Peak normalization then scales the
/// file in bounded chunks, preserving the single stochastic engine
/// performance; a 16-bit file is converted from it in place, the gain
/// folded into that one pass.
fn write_wav(
    path: &str,
    frames: impl ExactSizeIterator<Item = (f32, f32)>,
    normalize: bool,
    format: SampleFormat,
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
        write_header(&mut w, data_len, SampleFormat::Float32)?;
        for frame in frames {
            meter.push(frame);
            w.write_all(&frame.0.to_le_bytes())?;
            w.write_all(&frame.1.to_le_bytes())?;
        }
        w.flush()?;
    }
    let gain = if normalize && meter.peak > 1e-6 {
        let gain = 0.891 / meter.peak;
        if format == SampleFormat::Float32 {
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
        }
        println!(
            "Normalized: peak {:.3} -> -1 dBFS ({:+.1} dB)",
            meter.peak,
            20.0 * gain.log10()
        );
        gain
    } else {
        1.0
    };
    if format == SampleFormat::Pcm16 {
        float_to_pcm16_in_place(&mut file, data_len, gain)?;
    }
    Ok(meter.levels(gain))
}

/// The 44-byte header of a stereo 48 kHz WAV holding `data_len` bytes.
fn write_header(w: &mut impl Write, data_len: u32, format: SampleFormat) -> Result<()> {
    // (format tag, bytes per sample): 3 = IEEE float, 1 = integer PCM
    let (tag, bytes) = match format {
        SampleFormat::Float32 => (3u16, 4u16),
        SampleFormat::Pcm16 => (1u16, 2u16),
    };
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_len).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&tag.to_le_bytes())?;
    w.write_all(&2u16.to_le_bytes())?;
    w.write_all(&48000u32.to_le_bytes())?;
    w.write_all(&(48000u32 * 2 * bytes as u32).to_le_bytes())?;
    w.write_all(&(2 * bytes).to_le_bytes())?;
    w.write_all(&(8 * bytes).to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data_len.to_le_bytes())
}

/// Turn a float WAV of `data_len` data bytes into 16-bit PCM in the same
/// file, scaling by `gain` on the way. Each 4-byte float becomes a 2-byte
/// sample, so the write position always trails the read position and no
/// second file is needed. The dither is TPDF (two independent uniforms,
/// a triangle one LSB either side) from a fixed seed, so a bounce is
/// reproducible to the bit.
fn float_to_pcm16_in_place(file: &mut std::fs::File, data_len: u32, gain: f32) -> Result<()> {
    let mut rng = crate::rng::Rng::new(crate::rng::seed(0x16B1_D17E));
    let mut chunk = [0u8; 64 * 1024];
    let mut out = [0u8; 32 * 1024];
    let end = 44 + data_len as u64;
    let (mut read, mut write) = (44u64, 44u64);
    while read < end {
        let n = ((end - read) as usize).min(chunk.len());
        file.seek(SeekFrom::Start(read))?;
        file.read_exact(&mut chunk[..n])?;
        for (src, dst) in chunk[..n].chunks_exact(4).zip(out.chunks_exact_mut(2)) {
            let x = f32::from_le_bytes(src.try_into().unwrap()) * gain * 32767.0;
            let tpdf = rng.unipolar() - rng.unipolar();
            let q = (x + tpdf).round().clamp(-32768.0, 32767.0) as i16;
            dst.copy_from_slice(&q.to_le_bytes());
        }
        file.seek(SeekFrom::Start(write))?;
        file.write_all(&out[..n / 2])?;
        read += n as u64;
        write += n as u64 / 2;
    }
    file.set_len(write)?;
    file.seek(SeekFrom::Start(0))?;
    write_header(file, data_len / 2, SampleFormat::Pcm16)?;
    file.flush()
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

    /// Every sample track is its own strip, so `render_stems` must write a
    /// stem for each — the second tape slot's stem holding the second slot.
    #[test]
    fn each_sample_track_solos_to_its_own_audio() {
        let dir = std::env::temp_dir().join(format!("patina-stems-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = |name: &str, hz: f32| {
            let n = 24000usize;
            let mut d = Vec::new();
            d.extend_from_slice(b"RIFF");
            d.extend_from_slice(&(36 + n as u32 * 2).to_le_bytes());
            d.extend_from_slice(b"WAVEfmt ");
            d.extend_from_slice(&16u32.to_le_bytes());
            d.extend_from_slice(&1u16.to_le_bytes());
            d.extend_from_slice(&1u16.to_le_bytes());
            d.extend_from_slice(&48000u32.to_le_bytes());
            d.extend_from_slice(&96000u32.to_le_bytes());
            d.extend_from_slice(&2u16.to_le_bytes());
            d.extend_from_slice(&16u16.to_le_bytes());
            d.extend_from_slice(b"data");
            d.extend_from_slice(&(n as u32 * 2).to_le_bytes());
            for i in 0..n {
                let x = (std::f32::consts::TAU * hz * i as f32 / 48000.0).sin() * 12000.0;
                d.extend_from_slice(&(x as i16).to_le_bytes());
            }
            let path = dir.join(name);
            std::fs::write(&path, d).unwrap();
            path.to_str().unwrap().to_string()
        };
        let (a, b) = (wav("a.wav", 220.0), wav("b.wav", 330.0));
        let song = crate::song::parse_song_text(&format!(
            "bpm 120\ntrack one sample={a} root=C3\nC3:1\ntrack two sample={b} root=C3\n>2\nC3:1\n"
        ))
        .unwrap();
        let stems = dir.join("stems");
        render_stems(&song, stems.to_str().unwrap(), SampleFormat::Float32).unwrap();
        let energy = |name: &str| {
            let path = stems.join(format!("{name}.wav"));
            let data = crate::sampler::load_wav_stereo(path.to_str().unwrap())
                .unwrap_or_else(|e| panic!("no stem for sample track '{name}': {e}"));
            data.left.iter().map(|x| (*x as f64).powi(2)).sum::<f64>()
        };
        assert!(energy("one") > 1.0, "the first sample track's stem must hold its audio");
        assert!(energy("two") > 1.0, "the second sample track's stem must hold its audio");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Every `kit=909` track is its own board and strip, so each writes
    /// its own stem holding its own hits; a `kit=<track>` that plays an
    /// earlier kit's board rides that track's stem.
    #[test]
    fn each_kit_track_gets_its_own_stem() {
        let dir = std::env::temp_dir().join(format!("patina-kit-stems-{}", std::process::id()));
        // One voice per track, each struck at its own beat: kick on 0,
        // snare on 1, rim on 2 (and the shared track's open hat on 3)
        let song = crate::song::parse_song_text(
            "bpm 120\ntail 0.5\n\
             track one kit=909\nBD R:3\n\
             track two kit=909\nR SD R:2\n\
             track three kit=909\nR:2 RS R\n\
             track shared kit=one\nR:3 OH\n",
        )
        .unwrap();
        render_stems(&song, dir.to_str().unwrap(), SampleFormat::Float32).unwrap();
        // Energy per half-second window (one beat at 120 bpm)
        let beats = |name: &str| -> Vec<f64> {
            let path = dir.join(format!("{name}.wav"));
            let data = crate::sampler::load_wav_stereo(path.to_str().unwrap())
                .unwrap_or_else(|e| panic!("no stem for kit track '{name}': {e}"));
            data.left
                .chunks(24000)
                .zip(data.right.chunks(24000))
                .map(|(l, r)| l.iter().chain(r).map(|x| (*x as f64).powi(2)).sum())
                .collect()
        };
        for (name, beat) in [("one", 0), ("two", 1), ("three", 2)] {
            let e = beats(name);
            assert!(e[beat] > 1.0, "{name}'s stem must hold its hit: {e:?}");
            // nothing before its own hit: the other boards are not in it
            for (b, &x) in e.iter().enumerate().take(beat) {
                assert!(x < e[beat] * 1e-6, "{name}'s stem heard beat {b}: {e:?}");
            }
        }
        // the shared track's open hat lands in track one's stem
        let one = beats("one");
        assert!(
            one[3] > one[2] * 10.0,
            "kit=one should ride one's stem: {one:?}"
        );
        assert!(
            !dir.join("shared.wav").exists(),
            "a shared board bounces once"
        );
        std::fs::remove_dir_all(&dir).ok();
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
            write_wav(
                path.to_str().unwrap(),
                frames.iter().copied(),
                normalize,
                SampleFormat::Float32,
            )
            .unwrap();
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
        let error = write_wav(
            "/no-such-directory/oversized.wav",
            frames,
            false,
            SampleFormat::Float32,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    /// `--bits 16`: a PCM header, the file half the float's size, every
    /// sample within one LSB of dither of the (normalized) float it came
    /// from, and the same bytes on a second bounce.
    #[test]
    fn pcm16_output_is_the_dithered_float() {
        // longer than one conversion chunk, and an odd frame count
        let frames: Vec<_> = (0..20001)
            .map(|i| ((i as f32 * 0.03).sin() * 0.5, (i as f32 * 0.05).cos() * 0.25))
            .collect();
        let peak = meter(&frames).peak;
        for normalize in [false, true] {
            let path = std::env::temp_dir().join(format!(
                "patina-pcm16-{}-{normalize}.wav",
                std::process::id()
            ));
            let p = path.to_str().unwrap();
            write_wav(p, frames.iter().copied(), normalize, SampleFormat::Pcm16).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            write_wav(p, frames.iter().copied(), normalize, SampleFormat::Pcm16).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "dither is seeded");
            std::fs::remove_file(&path).unwrap();
            assert_eq!(bytes.len(), 44 + frames.len() * 4);
            assert_eq!(&bytes[..4], b"RIFF");
            let le32 = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
            let le16 = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
            assert_eq!(le32(4) as usize, 36 + frames.len() * 4, "RIFF size");
            assert_eq!(le16(20), 1, "PCM tag");
            assert_eq!(le32(28), 48000 * 4, "byte rate");
            assert_eq!(le16(32), 4, "block align");
            assert_eq!(le16(34), 16, "bits");
            assert_eq!(le32(40) as usize, frames.len() * 4, "data size");
            let gain = if normalize { 0.891 / peak } else { 1.0 };
            let mut dithered = 0;
            for (data, &(l, r)) in bytes[44..].chunks_exact(4).zip(&frames) {
                for (q, x) in [(&data[..2], l), (&data[2..], r)] {
                    let q = i16::from_le_bytes([q[0], q[1]]) as f32;
                    let want = x * gain * 32767.0;
                    assert!((q - want).abs() <= 1.5, "{q} vs {want}");
                    dithered += (q != want.round()) as usize;
                }
            }
            assert!(dithered > frames.len() / 4, "no dither: {dithered}");
        }
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
