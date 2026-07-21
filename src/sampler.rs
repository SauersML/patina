// src/sampler.rs
//
// The tape deck: a polyphonic sampler in the Mellotron/SP lineage, fed by
// `track <name> sample=file.wav` in songs. Any recording becomes an
// instrument on the keys.
//
// Design, in this machine's terms: each sampler track is a SLOT — one reel
// of tape plus the settings of its transport. Notes start playback heads
// on that reel. A head is a varispeed capstan (keytracked resampling with
// 4-point Hermite interpolation), an amplitude envelope (linear attack,
// RC release), and a pan pot. The heads mix onto the same volt bus as the
// keyboard voices and the 909 — the sampler's output sags the power rail,
// runs through the fuzz/spring/reverb/chorus/tape chain, and bends with
// the pitch wheel and vibrato bus like everything else. The sampler is IN
// the instrument, not beside it.
//
// What a slot can do:
//   - keytrack repitch around a root note (classic sampler), or `fixed`
//     (every key plays natural speed — chop kits, vocal stabs)
//   - sustain-loop a region with an equal-power crossfade (`loop=a:b
//     xfade=0.05`) — the Mellotron that never runs out of tape
//   - `chop=N`: slice the region into N equal pads mapped chromatically
//     up from the root (the MPC workflow; slices play at natural speed)
//   - play `mode=gate` (note-off enters release) or `mode=oneshot`
//     (trigger and forget; chop defaults to this)
//   - `reverse` the transport
//   - `mono` (a.k.a. `choke`): each new note chokes the last, like the
//     909's hat pair or a turntablist's crossfader
//   - and be AUTOMATED per track while playing: smp_pitch is a varispeed
//     knob in semitones (tape-stop swoops, minor-third drops), smp_start
//     scrubs where in the region new notes drop the needle, smp_gain /
//     smp_pan / smp_attack / smp_release reshape the envelope live.
//
// Epistemic status: this is a digital sampler modeled on no single
// circuit; the varispeed math is exact, the envelope is the standard
// linear/RC pair, and all range choices are labeled by their defaults.

use std::sync::Arc;

use crate::oscillator::PROGRAM_V;
use crate::song::Param;

/// Sampler slots live on a reserved channel block just below the voice
/// box (u16::MAX - 1) and the drum board (u16::MAX): slot i is channel
/// SAMPLER_CHANNEL_BASE + i.
pub const MAX_SLOTS: usize = 32;
pub const SAMPLER_CHANNEL_BASE: u16 = u16::MAX - 2 - MAX_SLOTS as u16;

/// Map a song/MIDI channel to a sampler slot index, if it is one.
pub fn slot_for_channel(channel: u16) -> Option<usize> {
    let base = SAMPLER_CHANNEL_BASE;
    if (base..base + MAX_SLOTS as u16).contains(&channel) {
        Some((channel - base) as usize)
    } else {
        None
    }
}

/// Heads across the whole deck (not per slot): enough for thick pads on
/// one slot or a busy multi-slot arrangement, small enough to stay cheap.
const NUM_HEADS: usize = 24;

/// One-shot regions get a short fade approaching the region edge so a
/// `start=`/`end=` cut mid-waveform can't click.
const EDGE_DECLICK_SECS: f32 = 0.003;

/// A choked head dies in ~4 ms — fast enough to read as a cut, slow
/// enough not to snap.
const CHOKE_RELEASE_SECS: f32 = 0.004;

/// The reel: decoded audio at its source rate. Shared by Arc so cloning a
/// slot (song loading, tests) never copies minutes of audio.
pub struct SampleData {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub rate: u32,
}

impl SampleData {
    pub fn frames(&self) -> usize {
        self.left.len()
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PlayMode {
    /// Note-off enters the release stage (default).
    Gate,
    /// Trigger and forget: the head runs to the region edge regardless.
    OneShot,
}

/// A slot's transport settings. Everything here is live — automation
/// mutates it mid-song and playing heads read it every sample.
#[derive(Clone, Copy)]
pub struct SlotConfig {
    /// The key that plays the reel at natural speed (MIDI note).
    pub root: u8,
    /// Playback region in seconds; `end <= 0` means "to the end of tape".
    pub start: f32,
    pub end: f32,
    /// Sustain loop points in seconds (within the sample), if looping.
    pub loop_pts: Option<(f32, f32)>,
    /// Loop crossfade length in seconds (equal-power).
    pub xfade: f32,
    /// 0 = off; N = slice the region into N chromatic pads from the root.
    pub chop: usize,
    pub mode: PlayMode,
    pub reverse: bool,
    /// Choke group of one: a new note fast-releases the previous head.
    pub mono: bool,
    /// false = every key plays natural speed (`fixed`; implied by chop).
    pub keytrack: bool,
    pub gain: f32,
    /// -1 (left) .. +1 (right), constant-power, center unity.
    pub pan: f32,
    pub attack: f32,
    pub release: f32,
    /// How much velocity shapes level: 0 = none, 1 = fully velocity.
    pub vel_amt: f32,
    /// Varispeed offset in semitones (smp_pitch — automatable).
    pub pitch_semis: f32,
    /// Where new notes drop the needle: 0..1 across the region
    /// (smp_start — automatable).
    pub scrub: f32,
}

impl Default for SlotConfig {
    fn default() -> Self {
        Self {
            root: 60,
            start: 0.0,
            end: 0.0,
            loop_pts: None,
            xfade: 0.01,
            chop: 0,
            mode: PlayMode::Gate,
            reverse: false,
            mono: false,
            keytrack: true,
            gain: 0.8,
            pan: 0.0,
            attack: 0.002,
            release: 0.05,
            vel_amt: 1.0,
            pitch_semis: 0.0,
            scrub: 0.0,
        }
    }
}

/// One loaded reel plus its transport: what a `sample=` track contributes
/// to a Song, and what the engine registers per slot.
#[derive(Clone)]
pub struct SamplerSlot {
    pub data: Arc<SampleData>,
    pub cfg: SlotConfig,
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Off,
    Attack,
    Sustain,
    Release,
}

/// A playback head: one sounding note on one slot. The region, loop, and
/// direction are latched at note-on (a chop pad keeps its slice even if
/// the slot is re-automated); speed, gain, pan, and envelope times are
/// read live so automation breathes through held notes.
struct Head {
    stage: Stage,
    slot: usize,
    note: u8,
    held: bool,
    /// Position on the reel in SOURCE frames (fractional).
    pos: f64,
    /// Latched region and loop, in source frames.
    region: (f64, f64),
    looping: Option<(f64, f64)>,
    xfade_frames: f64,
    reverse: bool,
    keytrack: bool,
    env: f32,
    vel_gain: f32,
    /// Overrides the slot release when choked.
    choke: bool,
    age: u64,
}

impl Head {
    fn idle() -> Self {
        Self {
            stage: Stage::Off,
            slot: 0,
            note: 0,
            held: false,
            pos: 0.0,
            region: (0.0, 0.0),
            looping: None,
            xfade_frames: 0.0,
            reverse: false,
            keytrack: true,
            env: 0.0,
            vel_gain: 1.0,
            choke: false,
            age: 0,
        }
    }
}

/// 4-point cubic Hermite (Catmull-Rom) read of one channel at a
/// fractional source-frame position, edge-clamped.
#[inline]
fn hermite(buf: &[f32], pos: f64) -> f32 {
    let n = buf.len();
    if n == 0 {
        return 0.0;
    }
    let i = pos.floor() as isize;
    let t = (pos - i as f64) as f32;
    let at = |k: isize| -> f32 {
        let k = k.clamp(0, n as isize - 1) as usize;
        buf[k]
    };
    let (xm1, x0, x1, x2) = (at(i - 1), at(i), at(i + 1), at(i + 2));
    let c = (x1 - xm1) * 0.5;
    let v = x0 - x1;
    let w = c + v;
    let a = w + v + (x2 - x0) * 0.5;
    let b = w + a;
    (((a * t) - b) * t + c) * t + x0
}

pub struct SamplerBank {
    sample_rate: f32,
    slots: Vec<Option<SamplerSlot>>,
    heads: Vec<Head>,
    counter: u64,
}

impl SamplerBank {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            slots: (0..MAX_SLOTS).map(|_| None).collect(),
            heads: (0..NUM_HEADS).map(|_| Head::idle()).collect(),
            counter: 0,
        }
    }

    /// Load a reel into a slot (kills anything still playing from it —
    /// the old tape is off the deck).
    pub fn set_slot(&mut self, index: usize, slot: SamplerSlot) {
        if index >= MAX_SLOTS {
            return;
        }
        for h in self.heads.iter_mut().filter(|h| h.slot == index) {
            h.stage = Stage::Off;
        }
        self.slots[index] = Some(slot);
    }

    /// Live per-slot automation. Returns false for non-sampler params so
    /// the caller can fall through to the ordinary channel path.
    pub fn set_param(&mut self, index: usize, param: Param, value: f32) -> bool {
        let Some(slot) = self.slots.get_mut(index).and_then(|s| s.as_mut()) else {
            // Still claim sampler params (a set before the slot loads is
            // a no-op, not a global fall-through)
            return matches!(
                param,
                Param::SmpPitch
                    | Param::SmpStart
                    | Param::SmpGain
                    | Param::SmpPan
                    | Param::SmpAttack
                    | Param::SmpRelease
            );
        };
        let c = &mut slot.cfg;
        match param {
            Param::SmpPitch => c.pitch_semis = value.clamp(-48.0, 48.0),
            Param::SmpStart => c.scrub = value.clamp(0.0, 1.0),
            Param::SmpGain => c.gain = value.clamp(0.0, 2.0),
            Param::SmpPan => c.pan = value.clamp(-1.0, 1.0),
            Param::SmpAttack => c.attack = value.clamp(0.001, 4.0),
            Param::SmpRelease => c.release = value.clamp(0.003, 8.0),
            _ => return false,
        }
        true
    }

    pub fn note_on(&mut self, slot_idx: usize, note: u8, velocity: f32) {
        let Some(slot) = self.slots.get(slot_idx).and_then(|s| s.as_ref()) else {
            return;
        };
        let data = &slot.data;
        let cfg = slot.cfg;
        let rate = data.rate as f64;
        let frames = data.frames() as f64;
        if frames < 2.0 {
            return;
        }

        // The playback region in source frames
        let mut r0 = (cfg.start as f64 * rate).clamp(0.0, frames - 1.0);
        let mut r1 = if cfg.end > 0.0 {
            (cfg.end as f64 * rate).clamp(r0 + 1.0, frames)
        } else {
            frames
        };

        // Chop: the key picks a slice of the region, wrapped so every key
        // lands on some pad
        if cfg.chop > 1 {
            let n = cfg.chop as i32;
            let k = (note as i32 - cfg.root as i32).rem_euclid(n) as f64;
            let slice = (r1 - r0) / n as f64;
            r1 = r0 + slice * (k + 1.0);
            r0 += slice * k;
        }

        // Scrub: drop the needle partway into the (possibly sliced) region
        let len = r1 - r0;
        let start = r0 + (cfg.scrub as f64).clamp(0.0, 0.98) * len;

        // Latch the sustain loop, clamped into the region; a degenerate
        // loop disables itself
        let looping = cfg.loop_pts.and_then(|(a, b)| {
            let a = (a as f64 * rate).clamp(r0, r1 - 1.0);
            let b = (b as f64 * rate).clamp(r0, r1);
            (b - a >= 4.0).then_some((a, b))
        });
        let xfade_frames = looping
            .map(|(a, b)| ((cfg.xfade as f64) * rate).clamp(0.0, (b - a) * 0.5))
            .unwrap_or(0.0);

        if cfg.mono {
            for h in self.heads.iter_mut() {
                if h.slot == slot_idx && h.stage != Stage::Off {
                    h.stage = Stage::Release;
                    h.held = false;
                    h.choke = true;
                }
            }
        }

        // A head: prefer an idle one, then the quietest releasing one,
        // then steal the oldest
        let idx = self
            .heads
            .iter()
            .position(|h| h.stage == Stage::Off)
            .or_else(|| {
                self.heads
                    .iter()
                    .enumerate()
                    .filter(|(_, h)| h.stage == Stage::Release)
                    .min_by(|a, b| a.1.env.total_cmp(&b.1.env))
                    .map(|(i, _)| i)
            })
            .or_else(|| {
                self.heads
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, h)| h.age)
                    .map(|(i, _)| i)
            });
        let Some(idx) = idx else { return };

        self.counter += 1;
        let vel = velocity.clamp(0.0, 1.0);
        self.heads[idx] = Head {
            stage: Stage::Attack,
            slot: slot_idx,
            note,
            held: true,
            pos: if cfg.reverse { r1 - 1.0 } else { start },
            region: (r0, r1),
            looping,
            xfade_frames,
            reverse: cfg.reverse,
            // Chop pads are drum pads: natural speed unless keytracking
            // is explicitly wanted (there is no option for that — CHOICE)
            keytrack: cfg.keytrack && cfg.chop <= 1,
            env: 0.0,
            // Velocity curve ^1.5: quiet notes usefully quiet, full
            // velocity unity — the response players expect from pads
            vel_gain: 1.0 - cfg.vel_amt + cfg.vel_amt * vel.powf(1.5),
            choke: false,
            age: self.counter,
        };
    }

    pub fn note_off(&mut self, slot_idx: usize, note: u8) {
        for h in self.heads.iter_mut() {
            if h.slot == slot_idx && h.note == note && h.held {
                h.held = false;
                if h.stage != Stage::Off {
                    let oneshot = self
                        .slots
                        .get(h.slot)
                        .and_then(|s| s.as_ref())
                        .map(|s| s.cfg.mode == PlayMode::OneShot)
                        .unwrap_or(false);
                    if !oneshot {
                        h.stage = Stage::Release;
                    }
                }
            }
        }
    }

    /// Mix all heads for one output sample, in volts. `pitch_mult` is the
    /// shared bend/vibrato bus ratio — tape speed follows the wheel.
    pub fn render_next(&mut self, pitch_mult: f32) -> (f32, f32) {
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        let out_rate = self.sample_rate as f64;

        for h in self.heads.iter_mut() {
            if h.stage == Stage::Off {
                continue;
            }
            let Some(slot) = self.slots[h.slot].as_ref() else {
                h.stage = Stage::Off;
                continue;
            };
            let cfg = &slot.cfg;
            let data = &slot.data;
            let src_rate = data.rate as f64;

            // Varispeed: keytrack interval * live pitch knob * bend bus
            let semis = if h.keytrack {
                (h.note as i16 - cfg.root as i16) as f32 + cfg.pitch_semis
            } else {
                cfg.pitch_semis
            };
            let step = (src_rate / out_rate)
                * ((semis / 12.0).exp2() as f64)
                * pitch_mult.max(0.01) as f64;

            // The envelope
            match h.stage {
                Stage::Attack => {
                    h.env += 1.0 / (cfg.attack.max(0.001) * self.sample_rate);
                    if h.env >= 1.0 {
                        h.env = 1.0;
                        h.stage = Stage::Sustain;
                    }
                }
                Stage::Release => {
                    let secs = if h.choke { CHOKE_RELEASE_SECS } else { cfg.release };
                    h.env *= (-1.0 / (secs.max(0.003) * self.sample_rate)).exp();
                    if h.env < 1e-3 {
                        h.stage = Stage::Off;
                        continue;
                    }
                }
                _ => {}
            }

            // Read the tape (with the loop's equal-power crossfade region
            // pre-blending the wrap destination)
            let (r0, r1) = h.region;
            let mut l = 0.0f32;
            let mut r = 0.0f32;
            let mut read = |pos: f64, g: f32| {
                l += hermite(&data.left, pos) * g;
                r += hermite(&data.right, pos) * g;
            };
            match h.looping {
                Some((la, lb)) if !h.reverse && h.xfade_frames > 0.0 && h.pos >= lb - h.xfade_frames => {
                    let t = ((h.pos - (lb - h.xfade_frames)) / h.xfade_frames) as f32;
                    let ph = t * std::f32::consts::FRAC_PI_2;
                    read(h.pos, ph.cos());
                    read(la + (h.pos - (lb - h.xfade_frames)), ph.sin());
                }
                _ => read(h.pos, 1.0),
            }

            // Edge declick so a region cut mid-waveform can't click
            let declick = EDGE_DECLICK_SECS as f64 * src_rate;
            let dist = if h.reverse { h.pos - r0 } else { r1 - h.pos };
            let edge = (dist / declick.max(1.0)).clamp(0.0, 1.0) as f32;

            // Constant-power pan, center unity
            let ph = (cfg.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
            let g = h.env * h.vel_gain * cfg.gain * edge * PROGRAM_V;
            left += l * g * ph.cos() * std::f32::consts::SQRT_2;
            right += r * g * ph.sin() * std::f32::consts::SQRT_2;

            // Advance the transport
            h.pos += if h.reverse { -step } else { step };
            match h.looping {
                Some((la, lb)) => {
                    if h.reverse {
                        if h.pos <= la {
                            h.pos = lb - 1.0; // reverse loop: hard wrap (no xfade)
                        }
                    } else if h.pos >= lb {
                        // The crossfade already played la..la+xfade; land past it
                        h.pos = la + (h.pos - lb) + h.xfade_frames;
                    }
                }
                None => {}
            }
            let out_of_tape = if h.reverse { h.pos <= r0 } else { h.pos >= r1 - 1.0 };
            if out_of_tape && h.looping.is_none() {
                h.stage = Stage::Off;
            }
        }

        (left, right)
    }

    /// True if any head is sounding (tests and the panel meter).
    pub fn any_active(&self) -> bool {
        self.heads.iter().any(|h| h.stage != Stage::Off)
    }
}

// ---------------------------------------------------------------------------
// WAV loading (stereo)
// ---------------------------------------------------------------------------

/// RIFF/WAVE reader for the sampler: PCM16, PCM24, or float32. Keeps the
/// first two channels (mono duplicates onto both); returns SampleData at
/// the file's own rate — the heads resample on playback.
pub fn load_wav_stereo(path: &str) -> Result<SampleData, String> {
    let data = std::fs::read(path).map_err(|e| format!("wav '{}': {}", path, e))?;
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("wav '{}': not a RIFF/WAVE file", path));
    }
    let mut pos = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut out: Option<SampleData> = None;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        if body + size > data.len() {
            break;
        }
        match id {
            b"fmt " if size >= 16 => {
                let format = u16::from_le_bytes(data[body..body + 2].try_into().unwrap());
                let channels = u16::from_le_bytes(data[body + 2..body + 4].try_into().unwrap());
                let rate = u32::from_le_bytes(data[body + 4..body + 8].try_into().unwrap());
                let bits = u16::from_le_bytes(data[body + 14..body + 16].try_into().unwrap());
                fmt = Some((format, channels, rate, bits));
            }
            b"data" => {
                let (format, channels, rate, bits) =
                    fmt.ok_or_else(|| format!("wav '{}': data before fmt", path))?;
                let ch = channels.max(1) as usize;
                let raw = &data[body..body + size];
                let bytes = match (format, bits) {
                    (1, 16) => 2,
                    (1, 24) => 3,
                    (3, 32) => 4,
                    (f, b) => {
                        return Err(format!(
                            "wav '{}': unsupported format {} / {} bits (use PCM16, PCM24, or float32)",
                            path, f, b
                        ))
                    }
                };
                let decode = |b: &[u8]| -> f32 {
                    match (format, bits) {
                        (1, 16) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
                        (1, 24) => i32::from_le_bytes([0, b[0], b[1], b[2]]) as f32 / 2147483648.0,
                        _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                    }
                };
                let frames = raw.len() / (bytes * ch);
                let mut left = Vec::with_capacity(frames);
                let mut right = Vec::with_capacity(frames);
                for fr in raw.chunks_exact(bytes * ch) {
                    let l = decode(&fr[0..bytes]);
                    let r = if ch > 1 { decode(&fr[bytes..2 * bytes]) } else { l };
                    left.push(l);
                    right.push(r);
                }
                out = Some(SampleData { left, right, rate });
            }
            _ => {}
        }
        pos = body + size + (size & 1);
    }
    let out = out.ok_or_else(|| format!("wav '{}': no data chunk", path))?;
    if out.frames() == 0 {
        return Err(format!("wav '{}': empty data chunk", path));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 440 Hz sine reel, one second at 48 kHz.
    fn sine_reel(rate: u32, secs: f32, hz: f32) -> Arc<SampleData> {
        let n = (rate as f32 * secs) as usize;
        let w = std::f32::consts::TAU * hz / rate as f32;
        let s: Vec<f32> = (0..n).map(|i| (i as f32 * w).sin() * 0.5).collect();
        Arc::new(SampleData { left: s.clone(), right: s, rate })
    }

    fn bank_with(cfg: SlotConfig) -> SamplerBank {
        let mut bank = SamplerBank::new(48000.0);
        bank.set_slot(0, SamplerSlot { data: sine_reel(48000, 1.0, 440.0), cfg });
        bank
    }

    /// Count rising zero crossings over n samples of the left channel.
    fn crossings(bank: &mut SamplerBank, n: usize) -> usize {
        let mut prev = 0.0f32;
        let mut count = 0;
        for _ in 0..n {
            let (l, _) = bank.render_next(1.0);
            if prev <= 0.0 && l > 0.0 {
                count += 1;
            }
            prev = l;
        }
        count
    }

    #[test]
    fn keytrack_repitches() {
        // Root at 60 → playing 72 doubles the frequency: ~880 crossings/s
        let mut bank = bank_with(SlotConfig { attack: 0.001, ..Default::default() });
        bank.note_on(0, 72, 1.0);
        let c = crossings(&mut bank, 24000); // half a second
        assert!((410..470).contains(&c), "expected ~440 crossings, got {}", c);
    }

    #[test]
    fn fixed_mode_ignores_the_key() {
        let cfg = SlotConfig { keytrack: false, attack: 0.001, ..Default::default() };
        let mut bank = bank_with(cfg);
        bank.note_on(0, 72, 1.0);
        let c = crossings(&mut bank, 24000);
        assert!((200..240).contains(&c), "expected ~220 crossings, got {}", c);
    }

    #[test]
    fn loop_sustains_past_the_reel() {
        let cfg = SlotConfig {
            loop_pts: Some((0.1, 0.3)),
            xfade: 0.02,
            ..Default::default()
        };
        let mut bank = bank_with(cfg);
        bank.note_on(0, 60, 1.0);
        // Render three reel-lengths; the loop must still be sounding
        let mut alive = 0.0f32;
        for _ in 0..(48000 * 3) {
            alive = bank.render_next(1.0).0.abs().max(alive * 0.999);
        }
        assert!(bank.any_active(), "looped head died");
        assert!(alive > 0.01, "looped head fell silent");
    }

    #[test]
    fn oneshot_runs_out_of_tape_and_gate_releases() {
        let mut bank = bank_with(SlotConfig { mode: PlayMode::OneShot, ..Default::default() });
        bank.note_on(0, 60, 1.0);
        bank.note_off(0, 60); // ignored: one-shots don't gate
        for _ in 0..24000 {
            bank.render_next(1.0);
        }
        assert!(bank.any_active(), "one-shot stopped at note-off");
        for _ in 0..(48000 + 4800) {
            bank.render_next(1.0);
        }
        assert!(!bank.any_active(), "one-shot outlived the reel");

        let mut bank = bank_with(SlotConfig { release: 0.02, ..Default::default() });
        bank.note_on(0, 60, 1.0);
        for _ in 0..4800 {
            bank.render_next(1.0);
        }
        bank.note_off(0, 60);
        for _ in 0..9600 {
            bank.render_next(1.0); // 0.2 s >> 5 release time constants
        }
        assert!(!bank.any_active(), "gated head ignored its release");
    }

    #[test]
    fn chop_slices_map_chromatically() {
        // 8 slices over 1 s: pad k starts at k/8 s. The reel is a ramp so
        // the first sample read identifies the slice.
        let n = 48000usize;
        let ramp: Vec<f32> = (0..n).map(|i| i as f32 / n as f32).collect();
        let data = Arc::new(SampleData { left: ramp.clone(), right: ramp, rate: 48000 });
        let mut bank = SamplerBank::new(48000.0);
        let cfg = SlotConfig { chop: 8, attack: 0.001, mode: PlayMode::OneShot, ..Default::default() };
        bank.set_slot(0, SamplerSlot { data, cfg });

        for (key, slice) in [(60u8, 0.0f32), (63, 3.0 / 8.0), (72, 4.0 / 8.0)] {
            bank.note_on(0, key, 1.0);
            // Skip the attack, then read: position ≈ slice start
            let mut v = 0.0;
            for _ in 0..480 {
                v = bank.render_next(1.0).0;
            }
            let v = v / (PROGRAM_V * cfg.gain); // undo the volt/gain scale
            assert!(
                (v - slice).abs() < 0.05,
                "key {} read {:.3}, expected slice at {:.3}",
                key,
                v,
                slice
            );
            bank.note_off(0, key);
            for _ in 0..48000 {
                bank.render_next(1.0);
            }
        }
    }

    #[test]
    fn mono_chokes_the_previous_note() {
        let mut bank = bank_with(SlotConfig { mono: true, ..Default::default() });
        bank.note_on(0, 60, 1.0);
        for _ in 0..4800 {
            bank.render_next(1.0);
        }
        bank.note_on(0, 67, 1.0);
        // After the choke fade reaches the -60 dB floor (~28 ms at the
        // 4 ms time constant) only one head survives
        for _ in 0..1920 {
            bank.render_next(1.0);
        }
        let sounding = bank.heads.iter().filter(|h| h.stage != Stage::Off).count();
        assert_eq!(sounding, 1, "choke left {} heads sounding", sounding);
    }

    #[test]
    fn varispeed_automation_bends_a_held_note() {
        // A long reel: both measurements must stay on tape at 2x speed
        let mut bank = SamplerBank::new(48000.0);
        bank.set_slot(0, SamplerSlot {
            data: sine_reel(48000, 4.0, 440.0),
            cfg: SlotConfig { attack: 0.001, ..Default::default() },
        });
        bank.note_on(0, 60, 1.0);
        let base = crossings(&mut bank, 24000);
        assert!(bank.set_param(0, Param::SmpPitch, 12.0));
        let up = crossings(&mut bank, 24000);
        assert!(
            up > base * 3 / 2,
            "smp_pitch +12 should ~double the rate ({} → {})",
            base,
            up
        );
    }

    #[test]
    fn reverse_plays_backwards() {
        let n = 48000usize;
        let ramp: Vec<f32> = (0..n).map(|i| i as f32 / n as f32).collect();
        let data = Arc::new(SampleData { left: ramp.clone(), right: ramp, rate: 48000 });
        let mut bank = SamplerBank::new(48000.0);
        let cfg = SlotConfig { reverse: true, attack: 0.001, ..Default::default() };
        bank.set_slot(0, SamplerSlot { data, cfg });
        bank.note_on(0, 60, 1.0);
        let mut a = 0.0;
        for _ in 0..480 {
            a = bank.render_next(1.0).0;
        }
        let mut b = 0.0;
        for _ in 0..4800 {
            b = bank.render_next(1.0).0;
        }
        assert!(a > b, "reverse ramp should descend ({:.3} → {:.3})", a, b);
    }

    #[test]
    fn channel_mapping() {
        assert_eq!(slot_for_channel(SAMPLER_CHANNEL_BASE), Some(0));
        assert_eq!(slot_for_channel(SAMPLER_CHANNEL_BASE + 31), Some(31));
        assert_eq!(slot_for_channel(crate::vox::VOX_CHANNEL), None);
        assert_eq!(slot_for_channel(crate::drums::DRUM_CHANNEL), None);
        assert_eq!(slot_for_channel(0), None);
        // The block sits strictly below the voice box
        assert!(SAMPLER_CHANNEL_BASE + MAX_SLOTS as u16 <= crate::vox::VOX_CHANNEL);
    }
}
