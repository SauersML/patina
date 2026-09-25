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

/// What `--render` / `--render-stems` write. Float32 is the default:
/// analysis-grade, the tape noise floor survives. Pcm16 is the delivery
/// format — half the size, readable by every tool — TPDF-dithered once,
/// after normalization, so the quantization is never done twice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SampleFormat {
    Float32,
    Pcm16,
}

impl SampleFormat {
    fn bytes(self) -> usize {
        match self {
            SampleFormat::Float32 => 4,
            SampleFormat::Pcm16 => 2,
        }
    }
}

/// One stem per track, all from ONE pass through the engine: every voice
/// renders once, and each track's strips feed a private copy of the
/// master chain (its sends ringing in its own reverb/spring/chorus, its
/// own tape), so a stem is that track exactly as it sits in the mix —
/// same voice stealing, same rail sag from the whole arrangement.
/// Tracks that share a strip (all `kit=` tracks, all vox tracks) bounce
/// once under the first track's name; each sampler track is its own.
/// The mix of the same pass is written alongside as `_mix.wav`.
///
/// Stems are NOT normalized — they are measurement files, written at the
/// exact gain the mix hears, and each is reported as a level-table row
/// (peak / RMS / LUFS) so measured mixing needs no hand math.
pub fn render_stems(song: &crate::song::Song, dir: &str, format: SampleFormat) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut names: Vec<String> = Vec::new();
    let mut groups: Vec<Vec<u16>> = Vec::new();
    for (name, channel) in &song.tracks {
        if groups.iter().any(|g| g.contains(channel)) {
            continue;
        }
        names.push(name.clone());
        groups.push(vec![*channel]);
    }
    let dir = dir.trim_end_matches('/');
    println!(
        "Rendering {} stems and the mix in one pass...",
        groups.len()
    );
    let start = std::time::Instant::now();
    let (mut render, buses) = crate::song::render_offline_stems(song, 48000.0, &groups);
    let frames = render.len();
    let mut mix = WavWriter::create(&format!("{dir}/_mix.wav"), frames, format)?;
    let mut stems = names
        .iter()
        .map(|name| WavWriter::create(&format!("{dir}/{name}.wav"), frames, format))
        .collect::<Result<Vec<_>>>()?;

    // The voices run on this thread; the stems' master chains (reverb,
    // tape — the expensive half) run on workers, one block behind, so
    // they overlap the next block's voices instead of adding to them.
    let workers = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .saturating_sub(1)
        .clamp(1, buses.len().max(1));
    let mut lanes: Vec<Vec<(usize, crate::voice_manager::StemBus)>> =
        (0..workers).map(|_| Vec::new()).collect();
    for (i, bus) in buses.into_iter().enumerate() {
        lanes[i % workers].push((i, bus));
    }
    type Job = (
        crate::voice_manager::StemLog,
        Vec<std::sync::Arc<Vec<crate::voice_manager::StemIn>>>,
    );
    type Done = Vec<(usize, Vec<(f32, f32)>)>;
    std::thread::scope(|scope| -> Result<()> {
        let mut to_workers = Vec::new();
        let mut from_workers = Vec::new();
        for mut lane in lanes {
            let (job_tx, job_rx) = std::sync::mpsc::sync_channel::<Job>(1);
            let (done_tx, done_rx) = std::sync::mpsc::sync_channel::<Done>(1);
            scope.spawn(move || {
                for (log, inputs) in job_rx {
                    let done = lane
                        .iter_mut()
                        .map(|(i, bus)| {
                            let mut out = Vec::with_capacity(inputs[*i].len());
                            bus.process(&inputs[*i], &log, &mut out);
                            (*i, out)
                        })
                        .collect();
                    if done_tx.send(done).is_err() {
                        break;
                    }
                }
            });
            to_workers.push(job_tx);
            from_workers.push(done_rx);
        }
        let mut in_flight = false;
        let collect = |stems: &mut Vec<WavWriter>| -> Result<()> {
            let mut blocks: Vec<Vec<(f32, f32)>> = vec![Vec::new(); stems.len()];
            for rx in &from_workers {
                for (i, out) in rx.recv().map_err(|_| Error::other("stem worker died"))? {
                    blocks[i] = out;
                }
            }
            for (stem, block) in stems.iter_mut().zip(blocks) {
                for frame in block {
                    stem.push(frame)?;
                }
            }
            Ok(())
        };
        const BLOCK: usize = 8192;
        loop {
            let mut n = 0;
            while n < BLOCK {
                match render.next() {
                    Some(frame) => mix.push(frame)?,
                    None => break,
                }
                n += 1;
            }
            // the previous block's stems finished while this one rendered
            if in_flight {
                collect(&mut stems)?;
            }
            let block = render.take_stem_block();
            let log = block.log();
            for tx in &to_workers {
                tx.send((log.clone(), block.inputs.clone()))
                    .map_err(|_| Error::other("stem worker died"))?;
            }
            in_flight = true;
            if n < BLOCK {
                break;
            }
        }
        collect(&mut stems)?;
        drop(to_workers);
        Ok(())
    })?;

    let mut table: Vec<(String, (f32, f32, f32))> = Vec::new();
    for (name, stem) in names.iter().zip(stems) {
        table.push((name.clone(), stem.finish(false)?));
    }
    table.push(("_mix".into(), mix.finish(false)?));
    let seconds = frames as f64 / 48000.0;
    let elapsed = start.elapsed().as_secs_f64();
    println!("peak concurrent voices: {}/64", render.peak_voices());
    println!(
        "Rendered {seconds:.1}s x {} stems in {elapsed:.2}s ({:.1}x realtime)",
        table.len() - 1,
        seconds / elapsed.max(1e-6)
    );
    println!(
        "\n{:<16} {:>10} {:>10} {:>10}",
        "stem", "peak dBFS", "rms dBFS", "LUFS"
    );
    for (name, (peak, rms, lufs)) in &table {
        println!("{:<16} {:>10.1} {:>10.1} {:>10.1}", name, peak, rms, lufs);
    }
    println!("Wrote {}/", dir);
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

pub fn render_to_wav(
    song: &crate::song::Song,
    path: &str,
    normalize: bool,
    format: SampleFormat,
) -> Result<()> {
    println!("Rendering {} events...", song.events.len());
    let start = std::time::Instant::now();
    let mut frames = crate::song::render_offline(song, 48000.0);
    let seconds = frames.len() as f64 / 48000.0;
    let (peak, rms, lufs) = write_wav(path, &mut frames, normalize, format)?;
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

/// Write a whole bounce: stream it once, then normalize/convert.
fn write_wav(
    path: &str,
    frames: impl ExactSizeIterator<Item = (f32, f32)>,
    normalize: bool,
    format: SampleFormat,
) -> Result<(f32, f32, f32)> {
    let mut w = WavWriter::create(path, frames.len(), format)?;
    for frame in frames {
        w.push(frame)?;
    }
    w.finish(normalize)
}

fn wav_header(w: &mut impl Write, frames: usize, format: SampleFormat) -> Result<u32> {
    let bytes = format.bytes();
    let data_len = frames
        .checked_mul(2 * bytes)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n <= u32::MAX - 36)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                "bounce exceeds the RIFF WAV size limit",
            )
        })?;
    let (tag, bits) = match format {
        SampleFormat::Float32 => (3u16, 32u16),
        SampleFormat::Pcm16 => (1u16, 16u16),
    };
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_len).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&tag.to_le_bytes())?;
    w.write_all(&2u16.to_le_bytes())?;
    w.write_all(&48000u32.to_le_bytes())?;
    w.write_all(&(48000u32 * 2 * bytes as u32).to_le_bytes())?;
    w.write_all(&(2 * bytes as u16).to_le_bytes())?;
    w.write_all(&bits.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data_len.to_le_bytes())?;
    Ok(data_len)
}

/// A streaming stereo 48 kHz WAV: frames are metered and written as they
/// arrive, as float32 — to the target itself, or for 16-bit output to a
/// side file that `finish` converts once the final gain is known. Peak
/// normalization rescales in bounded chunks, so a long bounce never has
/// to sit in memory.
struct WavWriter {
    path: String,
    float_path: String,
    frames: usize,
    format: SampleFormat,
    out: BufWriter<std::fs::File>,
    meter: Meter,
}

impl WavWriter {
    fn create(path: &str, frames: usize, format: SampleFormat) -> Result<Self> {
        // validate the final size before touching the disk
        wav_header(&mut std::io::sink(), frames, format)?;
        let float_path = match format {
            SampleFormat::Float32 => path.to_string(),
            SampleFormat::Pcm16 => format!("{path}.f32.tmp"),
        };
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&float_path)?;
        let mut out = BufWriter::with_capacity(64 * 1024, file);
        wav_header(&mut out, frames, SampleFormat::Float32)?;
        Ok(WavWriter {
            path: path.to_string(),
            float_path,
            frames,
            format,
            out,
            meter: Meter::default(),
        })
    }

    #[inline]
    fn push(&mut self, frame: (f32, f32)) -> Result<()> {
        self.meter.push(frame);
        self.out.write_all(&frame.0.to_le_bytes())?;
        self.out.write_all(&frame.1.to_le_bytes())
    }

    /// Flush, apply normalization (-1 dBFS peak) and the output format;
    /// returns (peak dBFS, rms dBFS, LUFS) of the file as written.
    fn finish(self, normalize: bool) -> Result<(f32, f32, f32)> {
        let mut file = self.out.into_inner().map_err(|e| e.into_error())?;
        file.flush()?;
        let gain = if normalize && self.meter.peak > 1e-6 {
            let gain = 0.891 / self.meter.peak;
            println!(
                "Normalized: peak {:.3} -> -1 dBFS ({:+.1} dB)",
                self.meter.peak,
                20.0 * gain.log10()
            );
            gain
        } else {
            1.0
        };
        let data_len = (self.frames * 8) as u64;
        match self.format {
            SampleFormat::Float32 => {
                if gain != 1.0 {
                    rescale_f32(&mut file, data_len, gain)?;
                }
            }
            SampleFormat::Pcm16 => {
                let mut out =
                    BufWriter::with_capacity(64 * 1024, std::fs::File::create(&self.path)?);
                wav_header(&mut out, self.frames, SampleFormat::Pcm16)?;
                file.seek(SeekFrom::Start(44))?;
                let mut src = std::io::BufReader::with_capacity(64 * 1024, &mut file);
                let mut buf = [0u8; 4];
                // TPDF dither: two independent uniforms, +-1 LSB triangle
                let mut rng = crate::rng::Rng::new(crate::rng::seed(0x16B1_D17E));
                for _ in 0..data_len / 4 {
                    src.read_exact(&mut buf)?;
                    let x = f32::from_le_bytes(buf) * gain * 32767.0;
                    let tpdf = rng.unipolar() - rng.unipolar();
                    let q = (x + tpdf).round().clamp(-32768.0, 32767.0) as i16;
                    out.write_all(&q.to_le_bytes())?;
                }
                out.flush()?;
                drop(src);
                drop(file);
                std::fs::remove_file(&self.float_path)?;
            }
        }
        Ok(self.meter.levels(gain))
    }
}

/// Scale a float32 WAV's data in place, in bounded chunks.
fn rescale_f32(file: &mut std::fs::File, data_len: u64, gain: f32) -> Result<()> {
    let mut chunk = [0u8; 64 * 1024];
    let mut offset = 44u64;
    let end = offset + data_len;
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

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("patina-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// One pass, many stems: a lone track's stem is the mix bit for bit
    /// (same strips into an identical chain), the pass's `_mix.wav` is the
    /// plain bounce bit for bit, and every track of a busier song gets a
    /// sounding stem — blocks and worker threads change nothing.
    #[test]
    fn one_pass_stems_match_the_mix() {
        let solo = crate::song::parse_song_text(
            "bpm 240\ntail 0.5\nautomate reverb_wet\n0.3 0.6:8\n\
             track a reverb_send=0.5\n(C4 E4 G4 C5)x4\n",
        )
        .unwrap();
        let dir = temp_dir("stems-solo");
        render_stems(&solo, dir.to_str().unwrap(), SampleFormat::Float32).unwrap();
        let a = std::fs::read(dir.join("a.wav")).unwrap();
        let mix = std::fs::read(dir.join("_mix.wav")).unwrap();
        assert_eq!(a, mix, "a lone track's stem must equal the mix");
        let plain = dir.join("plain.wav");
        write_wav(
            plain.to_str().unwrap(),
            crate::song::render_offline(&solo, 48000.0),
            false,
            SampleFormat::Float32,
        )
        .unwrap();
        assert_eq!(std::fs::read(&plain).unwrap(), mix);
        std::fs::remove_dir_all(&dir).unwrap();

        let duo = crate::song::parse_song_text(
            "bpm 240\ntail 0.3\ntrack lo\n(C3:2)x4\ntrack hi pan=0.5\n(G5 . E5 .)x4\n\
             track beat kit=909\n(BD CH SD CH)x4\n",
        )
        .unwrap();
        let dir = temp_dir("stems-duo");
        render_stems(&duo, dir.to_str().unwrap(), SampleFormat::Float32).unwrap();
        for name in ["lo", "hi", "beat", "_mix"] {
            let bytes = std::fs::read(dir.join(format!("{name}.wav"))).unwrap();
            let energy: f64 = bytes[44..]
                .chunks_exact(4)
                .map(|b| (f32::from_le_bytes(b.try_into().unwrap()) as f64).powi(2))
                .sum();
            assert!(energy > 1e-3, "stem {name} is silent");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `--bits 16`: a PCM header, and every sample within the one LSB of
    /// TPDF dither of the (normalized) float it came from.
    #[test]
    fn pcm16_output_is_dithered_float() {
        let frames: Vec<_> = (0..5000)
            .map(|i| {
                (
                    (i as f32 * 0.03).sin() * 0.5,
                    (i as f32 * 0.05).cos() * 0.25,
                )
            })
            .collect();
        for normalize in [false, true] {
            let path = std::env::temp_dir().join(format!(
                "patina-pcm16-{}-{normalize}.wav",
                std::process::id()
            ));
            let p = path.to_str().unwrap();
            write_wav(p, frames.iter().copied(), normalize, SampleFormat::Pcm16).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            std::fs::remove_file(&path).unwrap();
            assert!(!std::path::Path::new(&format!("{p}.f32.tmp")).exists());
            assert_eq!(bytes.len(), 44 + frames.len() * 4);
            assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 1, "PCM tag");
            assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 16, "bits");
            let gain = if normalize { 0.891 / 0.5 } else { 1.0 };
            for (data, &(l, r)) in bytes[44..].chunks_exact(4).zip(&frames) {
                let ql = i16::from_le_bytes([data[0], data[1]]) as f32;
                let qr = i16::from_le_bytes([data[2], data[3]]) as f32;
                assert!((ql - l * gain * 32767.0).abs() <= 1.5);
                assert!((qr - r * gain * 32767.0).abs() <= 1.5);
            }
        }
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
