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

/// The params the deck owns. One list: `set_param` uses it both to decide
/// whether to claim a value and to route it, so "claimed" and "applied"
/// can never disagree.
pub fn is_sampler_param(param: Param) -> bool {
    matches!(
        param,
        Param::SmpPitch
            | Param::SmpStart
            | Param::SmpGain
            | Param::SmpPan
            | Param::SmpAttack
            | Param::SmpRelease
            | Param::SmpCutoff
            | Param::SmpRes
    )
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
    /// Per-slot resonant lowpass (smp_cutoff / smp_res — automatable):
    /// the sampler filter of the SP/Akai school. 20 kHz + no resonance
    /// is bypassed.
    pub cutoff: f32,
    pub res: f32,
    /// Extra speed multiplier on top of keytracking — `beats=N` computes
    /// this so a loop spans exactly N beats at the song's tempo.
    pub speed: f32,
    /// Zero-order-hold playback (set by `bits=`/`rate=` crunching): the
    /// vintage DAC's imaging instead of band-limited reconstruction.
    pub zoh: bool,
    /// Formant-preserving pitch (TD-PSOLA): keytracking and smp_pitch
    /// re-space pitch-synchronous grains instead of varispeeding the
    /// tape, so a voice changes NOTE without changing THROAT — and
    /// duration is independent of pitch (`speed` becomes time-stretch).
    /// Loop/reverse are ignored in this mode.
    pub psola: bool,
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
            cutoff: 20000.0,
            res: 0.0,
            speed: 1.0,
            zoh: false,
            psola: false,
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
    /// State-variable filter integrator states: [ic1_l, ic2_l, ic1_r, ic2_r].
    svf: [f32; 4],
    /// PSOLA: source-time cursor (frames), next grain onset (output
    /// samples since note-on), output sample counter, active grains as
    /// (epoch index, position within grain).
    ps_cursor: f64,
    ps_next: f64,
    ps_n: f64,
    ps_grains: [(usize, f64); 4],
    ps_live: [bool; 4],
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
            svf: [0.0; 4],
            ps_cursor: 0.0,
            ps_next: 0.0,
            ps_n: 0.0,
            ps_grains: [(0, 0.0); 4],
            ps_live: [false; 4],
        }
    }
}

/// Pitch-synchronous analysis for psola playback: track f0 by
/// autocorrelation, then walk the reel one period at a time snapping
/// each mark to the local energy peak. Unvoiced stretches get uniform
/// 5 ms marks flagged unvoiced (played at unit spacing — Hann OLA at
/// 50% overlap reconstructs them nearly exactly).
pub fn analyze_epochs(data: &SampleData) -> Vec<(f64, f64, bool)> {
    let n = data.left.len();
    let rate = data.rate as f32;
    let mono: Vec<f32> = (0..n)
        .map(|i| 0.5 * (data.left[i] + data.right[i]))
        .collect();
    let frame = (rate * 0.025) as usize;
    let hop = (rate * 0.010) as usize;
    let lo = (rate / 500.0) as usize;
    let hi = ((rate / 70.0) as usize).min(frame.saturating_sub(1));
    let nf = if n > frame { (n - frame) / hop.max(1) } else { 0 };
    let mut f0 = vec![0.0f32; nf.max(1)];
    for j in 0..nf {
        let seg = &mono[j * hop..j * hop + frame];
        let mean = seg.iter().sum::<f32>() / frame as f32;
        let e0: f32 = seg.iter().map(|s| (s - mean) * (s - mean)).sum();
        if e0 < 1e-4 || hi <= lo {
            continue;
        }
        let (mut best, mut best_c) = (0usize, 0.0f32);
        for lag in lo..hi {
            let mut c = 0.0f32;
            let mut i = 0;
            while i + lag < frame {
                c += (seg[i] - mean) * (seg[i + lag] - mean);
                i += 2; // stride 2: analysis-grade, half the work
            }
            if c > best_c {
                best_c = c;
                best = lag;
            }
        }
        let mut e_half = 0.0f32;
        let mut i = 0;
        while i + best < frame {
            let a = seg[i] - mean;
            e_half += a * a;
            i += 2;
        }
        if best > 0 && best_c > 0.3 * e_half.max(1e-9) {
            f0[j] = rate / best as f32;
        }
    }
    // smoothed envelope for peak snapping
    let k = (rate / 1000.0) as usize + 1;
    let mut env = vec![0.0f32; n];
    let mut acc = 0.0f32;
    for i in 0..n {
        acc += mono[i].abs();
        if i >= k {
            acc -= mono[i - k].abs();
        }
        env[i] = acc;
    }
    // At least one frame per unvoiced mark: a reel claiming an absurdly
    // low rate would otherwise step by zero and walk this loop forever.
    let uv_step = ((rate * 0.005) as f64).max(1.0);
    let mut epochs = Vec::new();
    let mut i = 0.0f64;
    while (i as usize) < n.saturating_sub(2) {
        let j = ((i as usize) / hop.max(1)).min(f0.len().saturating_sub(1));
        let f = f0[j];
        if f > 0.0 {
            let t_frames = (rate / f) as f64;
            let a = ((i + 0.75 * t_frames) as usize).min(n.saturating_sub(2));
            let b = (((i + 1.25 * t_frames) as usize).min(n - 1)).max(a + 1);
            let mut peak = a;
            for m in a..b {
                if env[m] > env[peak] {
                    peak = m;
                }
            }
            epochs.push((peak as f64, t_frames, true));
            i = peak as f64;
        } else {
            epochs.push((i, uv_step, false));
            i += uv_step;
        }
    }
    epochs
}

/// Cached ZDF state-variable-filter coefficients for one slot; recomputed
/// only when its (cutoff, res) pair actually changes, never per sample.
#[derive(Clone, Copy)]
struct FiltCoeffs {
    cutoff: f32,
    res: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
    bypass: bool,
}

impl FiltCoeffs {
    fn stale() -> Self {
        Self { cutoff: -1.0, res: -1.0, a1: 0.0, a2: 0.0, a3: 0.0, k: 0.0, bypass: true }
    }

    fn update(&mut self, cutoff: f32, res: f32, sample_rate: f32) {
        if self.cutoff == cutoff && self.res == res {
            return;
        }
        self.cutoff = cutoff;
        self.res = res;
        self.bypass = cutoff >= 19000.0 && res <= 0.02;
        // Simper's trapezoidal SVF: stable under audio-rate coefficient
        // swings, which automated filter sweeps are
        let g = (std::f32::consts::PI * cutoff.clamp(20.0, 0.45 * sample_rate) / sample_rate).tan();
        self.k = 2.0 - 1.85 * res.clamp(0.0, 1.0);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    /// One lowpass tick on a (ic1, ic2) state pair.
    #[inline]
    fn tick(&self, x: f32, ic1: &mut f32, ic2: &mut f32) -> f32 {
        let v3 = x - *ic2;
        let v1 = self.a1 * *ic1 + self.a2 * v3;
        let v2 = *ic2 + self.a2 * *ic1 + self.a3 * v3;
        *ic1 = 2.0 * v1 - *ic1;
        *ic2 = 2.0 * v2 - *ic2;
        v2
    }
}

// ---------------------------------------------------------------------------
// Band-limited playback: windowed-sinc interpolation
// ---------------------------------------------------------------------------
//
// The read head is a Kaiser-windowed sinc kernel (16 taps at unit speed,
// beta = 8.6, ~-90 dB sidelobes) evaluated from one precomputed table.
// Pitching DOWN the kernel is used as-is; pitching UP it is time-stretched
// by the speed ratio, which lowers its cutoff to the post-shift Nyquist —
// proper decimation anti-aliasing, so playing high above the root stays
// clean instead of folding. Weights are normalized per read (exact unity
// DC, no ripple) and shared between the two channels.

/// Kernel half-width in source frames at unit stretch.
const SINC_HALF: usize = 8;
/// Table resolution: samples per unit of kernel time.
const SINC_RES: usize = 64;
/// Anti-alias stretch cap: beyond 4x down-pitch the residual images sit
/// below the -90 dB window floor of what a 64-tap read can promise.
const SINC_MAX_STRETCH: f32 = 4.0;

/// Zeroth-order modified Bessel function (power series — converges fast
/// for the argument range a Kaiser window uses).
fn bessel_i0(x: f32) -> f32 {
    let mut sum = 1.0f32;
    let mut term = 1.0f32;
    for k in 1..=24 {
        let t = x / (2.0 * k as f32);
        term *= t * t;
        sum += term;
        if term < 1e-9 * sum {
            break;
        }
    }
    sum
}

/// One-sided kernel table: sinc(0.97 x) * kaiser(x / HALF), x in [0, HALF].
fn sinc_table() -> &'static [f32] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let beta = 8.6f32;
        let denom = bessel_i0(beta);
        let n = SINC_HALF * SINC_RES + 2; // one guard entry past the edge
        (0..n)
            .map(|i| {
                let x = i as f32 / SINC_RES as f32;
                if x >= SINC_HALF as f32 {
                    return 0.0;
                }
                // Cutoff at 0.97 Nyquist: a hair of headroom for the
                // finite transition band of a 16-tap kernel
                let c = 0.97f32;
                let s = if x < 1e-6 {
                    c
                } else {
                    (std::f32::consts::PI * c * x).sin() / (std::f32::consts::PI * x)
                };
                let t = x / SINC_HALF as f32;
                s * bessel_i0(beta * (1.0 - t * t).sqrt()) / denom
            })
            .collect()
    })
}

/// Band-limited stereo read at a fractional source position. `stretch`
/// >= 1 widens the kernel for anti-aliased down-shifting (pass the read
/// speed when it exceeds 1). Edge-clamped; weights shared across L/R.
fn sinc_read(left: &[f32], right: &[f32], pos: f64, stretch: f32) -> (f32, f32) {
    let table = sinc_table();
    let n = left.len() as isize;
    if n == 0 {
        return (0.0, 0.0);
    }
    let stretch = stretch.clamp(1.0, SINC_MAX_STRETCH);
    let half = (SINC_HALF as f32 * stretch).ceil() as isize;
    let base = pos.floor() as isize;
    let frac = (pos - base as f64) as f32;
    let (mut al, mut ar, mut ws) = (0.0f32, 0.0f32, 0.0f32);
    let scale = SINC_RES as f32 / stretch;
    for j in (1 - half)..=half {
        let d = (j as f32 - frac).abs() * scale;
        let i = d as usize;
        if i + 1 >= table.len() {
            continue;
        }
        let t = d - i as f32;
        let w = table[i] + (table[i + 1] - table[i]) * t;
        let k = (base + j).clamp(0, n - 1) as usize;
        al += left[k] * w;
        ar += right[k] * w;
        ws += w;
    }
    if ws.abs() < 1e-6 {
        (0.0, 0.0)
    } else {
        (al / ws, ar / ws)
    }
}

/// Zero-order-hold read: the un-reconstructed DAC of the vintage boxes.
/// Crunched slots (`bits=`/`rate=`) play through this on purpose — the
/// imaging spray above the converter rate IS the SP-1200 sparkle.
#[inline]
fn zoh_read(left: &[f32], right: &[f32], pos: f64) -> (f32, f32) {
    let n = left.len();
    if n == 0 {
        return (0.0, 0.0);
    }
    let i = (pos.floor() as isize).clamp(0, n as isize - 1) as usize;
    (left[i], right[i])
}

/// Load-time converter emulation: optionally resample the reel to a
/// vintage converter rate (band-limited, via the same sinc kernel) and
/// truncate to a word size, as the old converters did (truncation, no
/// dither — the grit is authentic, the DC blocker downstream eats the
/// half-LSB offset). `bits=12 rate=26040` is the SP-1200's converter.
pub fn crunch(data: &SampleData, bits: Option<u32>, rate: Option<u32>) -> SampleData {
    let mut left = data.left.clone();
    let mut right = data.right.clone();
    let mut out_rate = data.rate;
    if let Some(to) = rate.filter(|&t| t != data.rate && t > 0) {
        let ratio = data.rate as f64 / to as f64;
        let stretch = (ratio as f32).max(1.0).min(8.0);
        let frames = (data.frames() as f64 / ratio).floor().max(1.0) as usize;
        let mut l = Vec::with_capacity(frames);
        let mut r = Vec::with_capacity(frames);
        for i in 0..frames {
            let (a, b) = sinc_read(&data.left, &data.right, i as f64 * ratio, stretch);
            l.push(a);
            r.push(b);
        }
        left = l;
        right = r;
        out_rate = to;
    }
    if let Some(b) = bits {
        let levels = (1u32 << b.clamp(2, 24).saturating_sub(1)) as f32;
        let q = |x: &mut f32| *x = (x.clamp(-1.0, 1.0) * levels).floor() / levels;
        left.iter_mut().for_each(&q);
        right.iter_mut().for_each(&q);
    }
    SampleData { left, right, rate: out_rate }
}

pub struct SamplerBank {
    sample_rate: f32,
    slots: Vec<Option<SamplerSlot>>,
    heads: Vec<Head>,
    /// Pitch-synchronous analysis per slot (psola mode), computed once
    /// at registration: (source frame, source period frames, voiced).
    epochs: Vec<Option<Arc<Vec<(f64, f64, bool)>>>>,
    filt: Vec<FiltCoeffs>,
    counter: u64,
}

impl SamplerBank {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            slots: (0..MAX_SLOTS).map(|_| None).collect(),
            heads: (0..NUM_HEADS).map(|_| Head::idle()).collect(),
            epochs: (0..MAX_SLOTS).map(|_| None).collect(),
            filt: (0..MAX_SLOTS).map(|_| FiltCoeffs::stale()).collect(),
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
        self.epochs[index] = if slot.cfg.psola {
            Some(Arc::new(analyze_epochs(&slot.data)))
        } else {
            None
        };
        self.slots[index] = Some(slot);
    }

    /// Live per-slot automation. Returns false for non-sampler params so
    /// the caller can fall through to the ordinary channel path.
    pub fn set_param(&mut self, index: usize, param: Param, value: f32) -> bool {
        // The deck claims its own params whether or not the slot has
        // loaded (a set before load is a no-op, not a global fall-through)
        // and whatever the value is — one list, so the two answers cannot
        // drift apart.
        if !is_sampler_param(param) {
            return false;
        }
        // Automation values are whatever the song or host wrote, and
        // f32::clamp returns NaN unchanged. A single non-finite write would
        // live in the transport forever: a NaN read position never reaches
        // the end of tape, so the head sounds NaN until the plugin is
        // reloaded. Drop the write, keep the param.
        if !value.is_finite() {
            return true;
        }
        let Some(slot) = self.slots.get_mut(index).and_then(|s| s.as_mut()) else {
            return true;
        };
        let c = &mut slot.cfg;
        match param {
            Param::SmpPitch => c.pitch_semis = value.clamp(-48.0, 48.0),
            Param::SmpStart => c.scrub = value.clamp(0.0, 1.0),
            Param::SmpGain => c.gain = value.clamp(0.0, 2.0),
            Param::SmpPan => c.pan = value.clamp(-1.0, 1.0),
            Param::SmpAttack => c.attack = value.clamp(0.001, 4.0),
            Param::SmpRelease => c.release = value.clamp(0.003, 8.0),
            Param::SmpCutoff => c.cutoff = value.clamp(60.0, 20000.0),
            Param::SmpRes => c.res = value.clamp(0.0, 1.0),
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

        // Scrub: drop the needle partway into the (possibly sliced)
        // region — from the far end when the transport runs backwards
        let len = r1 - r0;
        let scrub = (cfg.scrub as f64).clamp(0.0, 0.98) * len;
        let start = if cfg.reverse { r1 - 1.0 - scrub } else { r0 + scrub };

        // Latch the sustain loop, clamped into the region; a degenerate
        // loop disables itself. `chop` can slice the region below one
        // frame (128 pads out of a 64-frame click), which would leave the
        // loop-start window inverted — f64::clamp panics on min > max, so
        // the window is built from the region, not assumed wider than it.
        let looping = cfg.loop_pts.and_then(|(a, b)| {
            let a = (a as f64 * rate).clamp(r0, (r1 - 1.0).max(r0));
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
        // Song velocities are parsed floats and never validated (`C4@NaN`
        // is a legal token). f32::clamp passes NaN through, and a NaN
        // vel_gain would multiply into the slot bus for the life of the
        // plugin — every later note on any slot included.
        let vel = if velocity.is_nan() { 0.0 } else { velocity.clamp(0.0, 1.0) };
        self.heads[idx] = Head {
            stage: Stage::Attack,
            slot: slot_idx,
            note,
            held: true,
            pos: start,
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
            svf: [0.0; 4],
            ps_cursor: 0.0,
            ps_next: 0.0,
            ps_n: 0.0,
            ps_grains: [(0, 0.0); 4],
            ps_live: [false; 4],
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
        let mut slots = [(0.0f32, 0.0f32); MAX_SLOTS];
        self.render_next_slots(pitch_mult, &mut slots);
        slots.iter().fold((0.0, 0.0), |a, s| (a.0 + s.0, a.1 + s.1))
    }

    /// Per-slot render: each slot's heads land in its own bucket so the
    /// mixer can give every sample track a REAL strip. (The old single
    /// summed output meant one strip governed the whole deck — per-track
    /// gain/pan/sends/duck on all but the first slot were silently dead.)
    pub fn render_next_slots(
        &mut self,
        pitch_mult: f32,
        out: &mut [(f32, f32); MAX_SLOTS],
    ) {
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

            // Varispeed: keytrack interval * live pitch knob * tempo-fit
            // speed * bend bus
            let semis = if h.keytrack {
                (h.note as i16 - cfg.root as i16) as f32 + cfg.pitch_semis
            } else {
                cfg.pitch_semis
            };
            let step = (src_rate / out_rate)
                * ((semis / 12.0).exp2() as f64)
                * cfg.speed.clamp(0.03, 32.0) as f64
                * pitch_mult.max(0.01) as f64;
            // Backstop: a non-finite transport can never reach the end of
            // tape (every comparison against NaN is false), so the head
            // would sound NaN forever and poison the whole mix bus. Lift
            // it off the reel instead. The setters above keep automation
            // finite; this catches anything latched at load time.
            if !step.is_finite() || !h.pos.is_finite() {
                h.stage = Stage::Off;
                continue;
            }

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
            // pre-blending the wrap destination). Band-limited sinc for
            // hi-fi slots — the kernel stretched by the read speed when
            // pitching up, so nothing folds — or the raw ZOH of a
            // crunched slot's vintage DAC.
            let (r0, r1) = h.region;
            let stretch = step as f32;
            let zoh = cfg.zoh;
            let mut l = 0.0f32;
            let mut r = 0.0f32;

            // PSOLA: the note changes, the throat doesn't. Grains cut at
            // the reel's glottal epochs, re-spaced at the target period;
            // the cursor advances at natural (or `speed`-stretched) time,
            // so pitch and duration are independent.
            if cfg.psola {
                // A reel too short to hold one analysis step yields no
                // epochs at all; grain launching would index an empty
                // table. Fall through to the ordinary varispeed read.
                if let Some(ep) = self.epochs[h.slot].as_ref().filter(|e| !e.is_empty()) {
                    if h.ps_n == 0.0 {
                        h.ps_cursor = h.pos.max(r0);
                    }
                    let ratio = ((semis / 12.0).exp2() * pitch_mult.max(0.01)) as f64;
                    let rate_ratio = src_rate / out_rate;
                    // launch a grain when its time comes
                    if h.ps_n >= h.ps_next {
                        // nearest epoch to the cursor
                        let mut eix = match ep.binary_search_by(|e| {
                            e.0.partial_cmp(&h.ps_cursor).unwrap()
                        }) {
                            Ok(i) => i,
                            Err(i) => i.min(ep.len() - 1),
                        };
                        if eix > 0
                            && (ep[eix].0 - h.ps_cursor).abs()
                                > (ep[eix - 1].0 - h.ps_cursor).abs()
                        {
                            eix -= 1;
                        }
                        for g in 0..h.ps_grains.len() {
                            if !h.ps_live[g] {
                                h.ps_grains[g] = (eix, 0.0);
                                h.ps_live[g] = true;
                                break;
                            }
                        }
                        let (_, period, voiced) = ep[eix];
                        let r_eff = if voiced { ratio } else { 1.0 };
                        h.ps_next += (period / r_eff) / rate_ratio;
                    }
                    // mix the live grains
                    for g in 0..h.ps_grains.len() {
                        if !h.ps_live[g] {
                            continue;
                        }
                        let (eix, gpos) = h.ps_grains[g];
                        let (center, period, _v) = ep[eix];
                        let half = period.min(600.0);
                        let width = 2.0 * half;
                        if gpos >= width {
                            h.ps_live[g] = false;
                            continue;
                        }
                        let src_pos = center - half + gpos;
                        if src_pos >= 0.0 && src_pos < r1 {
                            let w = 0.5
                                - 0.5 * (std::f64::consts::TAU * gpos / width).cos();
                            let (a, b) = sinc_read(
                                &data.left, &data.right, src_pos, 1.0,
                            );
                            l += a * w as f32;
                            r += b * w as f32;
                        }
                        h.ps_grains[g].1 = gpos + rate_ratio;
                    }
                    h.ps_cursor +=
                        rate_ratio * cfg.speed.clamp(0.03, 32.0) as f64;
                    h.ps_n += 1.0;
                    if h.ps_cursor >= r1 {
                        if h.held {
                            // hold the tail like a gate reaching tape end
                            h.stage = Stage::Release;
                            h.held = false;
                        } else {
                            h.stage = Stage::Release;
                        }
                    }
                    // shared filter/pan path, mirrored from below
                    let fc = &mut self.filt[h.slot];
                    fc.update(cfg.cutoff, cfg.res, self.sample_rate);
                    if fc.bypass {
                        h.svf[1] = l;
                        h.svf[3] = r;
                        h.svf[0] = 0.0;
                        h.svf[2] = 0.0;
                    } else {
                        let [mut i1l, mut i2l, mut i1r, mut i2r] = h.svf;
                        l = fc.tick(l, &mut i1l, &mut i2l);
                        r = fc.tick(r, &mut i1r, &mut i2r);
                        h.svf = [i1l, i2l, i1r, i2r];
                    }
                    let ph = (cfg.pan.clamp(-1.0, 1.0) + 1.0)
                        * std::f32::consts::FRAC_PI_4;
                    let g = h.env * h.vel_gain * cfg.gain * PROGRAM_V;
                    out[h.slot].0 += l * g * ph.cos() * std::f32::consts::SQRT_2;
                    out[h.slot].1 += r * g * ph.sin() * std::f32::consts::SQRT_2;
                    continue;
                }
            }
            let mut read = |pos: f64, g: f32| {
                let (a, b) = if zoh {
                    zoh_read(&data.left, &data.right, pos)
                } else {
                    sinc_read(&data.left, &data.right, pos, stretch)
                };
                l += a * g;
                r += b * g;
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

            // The slot's lowpass (coefficients cached per slot, state per
            // head). Bypassed wide-open; the integrator tracks the input
            // meanwhile so a sweep re-engaging the filter doesn't thump.
            let fc = &mut self.filt[h.slot];
            fc.update(cfg.cutoff, cfg.res, self.sample_rate);
            if fc.bypass {
                h.svf[1] = l;
                h.svf[3] = r;
                h.svf[0] = 0.0;
                h.svf[2] = 0.0;
            } else {
                let [mut i1l, mut i2l, mut i1r, mut i2r] = h.svf;
                l = fc.tick(l, &mut i1l, &mut i2l);
                r = fc.tick(r, &mut i1r, &mut i2r);
                h.svf = [i1l, i2l, i1r, i2r];
            }

            // Edge declick so a region cut mid-waveform can't click
            let declick = EDGE_DECLICK_SECS as f64 * src_rate;
            let dist = if h.reverse { h.pos - r0 } else { r1 - h.pos };
            let edge = (dist / declick.max(1.0)).clamp(0.0, 1.0) as f32;

            // Constant-power pan, center unity
            let ph = (cfg.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
            let g = h.env * h.vel_gain * cfg.gain * edge * PROGRAM_V;
            out[h.slot].0 += l * g * ph.cos() * std::f32::consts::SQRT_2;
            out[h.slot].1 += r * g * ph.sin() * std::f32::consts::SQRT_2;

            // Advance the transport
            h.pos += if h.reverse { -step } else { step };
            match h.looping {
                Some((la, lb)) => {
                    if h.reverse {
                        if h.pos <= la {
                            h.pos = lb - 1.0; // reverse loop: hard wrap (no xfade)
                        }
                    } else if h.pos >= lb {
                        // The crossfade already played la..la+xfade; land
                        // past it. Wrap MODULO the loop body, not by one
                        // length: a step wider than the loop (a short loop
                        // played far above the root, or `beats=` fitting a
                        // 32x speed) would otherwise overshoot further
                        // every wrap and walk clean off the end of the
                        // tape — where the declick reads zero and the head
                        // never terminates, because looping heads are
                        // never tested for end-of-tape.
                        let body = (lb - h.xfade_frames) - la;
                        h.pos = la + h.xfade_frames + (h.pos - lb).rem_euclid(body.max(1e-9));
                    }
                }
                None => {}
            }
            let out_of_tape = if h.reverse { h.pos <= r0 } else { h.pos >= r1 - 1.0 };
            if out_of_tape && h.looping.is_none() {
                h.stage = Stage::Off;
            }
        }
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
                let mut format = u16::from_le_bytes(data[body..body + 2].try_into().unwrap());
                let channels = u16::from_le_bytes(data[body + 2..body + 4].try_into().unwrap());
                let rate = u32::from_le_bytes(data[body + 4..body + 8].try_into().unwrap());
                let bits = u16::from_le_bytes(data[body + 14..body + 16].try_into().unwrap());
                // WAVE_FORMAT_EXTENSIBLE (what most DAWs export): the
                // real format is the SubFormat GUID's leading 16 bits
                if format == 0xFFFE && size >= 40 {
                    format = u16::from_le_bytes(data[body + 24..body + 26].try_into().unwrap());
                }
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
                    (1, 32) | (3, 32) => 4,
                    (f, b) => {
                        return Err(format!(
                            "wav '{}': unsupported format {} / {} bits (use PCM 16/24/32 or float32)",
                            path, f, b
                        ))
                    }
                };
                let decode = |b: &[u8]| -> f32 {
                    match (format, bits) {
                        (1, 16) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
                        (1, 24) => i32::from_le_bytes([0, b[0], b[1], b[2]]) as f32 / 2147483648.0,
                        (1, 32) => {
                            i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2147483648.0
                        }
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
    // A 0 Hz reel divides by zero everywhere downstream (the psola epoch
    // walker steps by zero and never terminates); reject it at the door.
    if out.rate == 0 {
        return Err(format!("wav '{}': sample rate is 0", path));
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

    fn rms(bank: &mut SamplerBank, n: usize) -> f32 {
        let mut acc = 0.0f64;
        for _ in 0..n {
            let (l, _) = bank.render_next(1.0);
            acc += (l as f64) * (l as f64);
        }
        ((acc / n as f64) as f32).sqrt()
    }

    /// The accuracy claim: pitching UP is band-limited. A 15 kHz partial
    /// played +12 lands at 30 kHz — beyond Nyquist — and must be filtered
    /// by the stretched kernel, not folded back as an 18 kHz alias.
    /// (Hermite/ZOH reads keep nearly all of it.)
    #[test]
    fn upshift_is_antialiased() {
        let hot = |bank: &mut SamplerBank, note: u8| {
            bank.note_on(0, note, 1.0);
            for _ in 0..480 {
                bank.render_next(1.0); // past the attack
            }
            rms(bank, 9600)
        };
        // Re-tune the reel to 15 kHz
        let mut bank = SamplerBank::new(48000.0);
        bank.set_slot(0, SamplerSlot {
            data: sine_reel(48000, 1.0, 15000.0),
            cfg: SlotConfig { attack: 0.001, ..Default::default() },
        });
        let natural = hot(&mut bank, 60);
        let mut bank2 = SamplerBank::new(48000.0);
        bank2.set_slot(0, SamplerSlot {
            data: sine_reel(48000, 1.0, 15000.0),
            cfg: SlotConfig { attack: 0.001, ..Default::default() },
        });
        let shifted = hot(&mut bank2, 72);
        assert!(natural > 0.1, "natural read lost the partial, rms={natural}");
        assert!(
            shifted < natural * 0.05,
            "aliasable content must be filtered when pitching up: {natural} -> {shifted}"
        );
    }

    /// The vintage converters: `crunch` truncates onto the exact bit grid
    /// and resamples the reel to the converter rate.
    #[test]
    fn crunch_quantizes_and_resamples() {
        let data = sine_reel(48000, 1.0, 440.0);
        let c = crunch(&data, Some(8), None);
        let levels = 128.0f32;
        for &s in c.left.iter().step_by(97) {
            let snapped = (s * levels).round() / levels;
            assert!(
                (s - snapped).abs() < 1e-6,
                "sample {} not on the 8-bit grid",
                s
            );
        }
        let c = crunch(&data, None, Some(24000));
        assert_eq!(c.rate, 24000);
        assert!((c.frames() as i64 - 24000).abs() <= 2);
        // The 440 Hz tone survives decimation at full level
        let peak = c.left.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.05, "tone lost in resample, peak={peak}");
    }

    /// The slot filter: an 8 kHz partial through a 500 Hz lowpass.
    #[test]
    fn slot_filter_darkens() {
        let open = {
            let mut bank = SamplerBank::new(48000.0);
            bank.set_slot(0, SamplerSlot {
                data: sine_reel(48000, 1.0, 8000.0),
                cfg: SlotConfig { attack: 0.001, ..Default::default() },
            });
            bank.note_on(0, 60, 1.0);
            rms(&mut bank, 9600)
        };
        let dark = {
            let mut bank = SamplerBank::new(48000.0);
            bank.set_slot(0, SamplerSlot {
                data: sine_reel(48000, 1.0, 8000.0),
                cfg: SlotConfig { attack: 0.001, cutoff: 500.0, ..Default::default() },
            });
            bank.note_on(0, 60, 1.0);
            rms(&mut bank, 9600)
        };
        assert!(open > 0.1, "open filter should pass the tone, rms={open}");
        assert!(
            dark < open * 0.05,
            "500 Hz lowpass should crush an 8 kHz tone: {open} -> {dark}"
        );
    }

    /// `beats=` fit: a half-speed slot takes twice the tape time.
    #[test]
    fn speed_scales_playback_time() {
        let mut bank = bank_with(SlotConfig {
            attack: 0.001,
            mode: PlayMode::OneShot,
            speed: 0.5,
            ..Default::default()
        });
        bank.note_on(0, 60, 1.0);
        for _ in 0..(48000 + 24000) {
            bank.render_next(1.0);
        }
        assert!(bank.any_active(), "half-speed one-shot ended a reel-length early");
        for _ in 0..48000 {
            bank.render_next(1.0);
        }
        assert!(!bank.any_active(), "half-speed one-shot should end by 2 reel-lengths");
    }

    /// Reverse honors the needle-drop scrub: scrub 0.5 on a rising ramp
    /// starts the backwards read from the middle, not the end.
    #[test]
    fn reverse_scrub_starts_midway() {
        let n = 48000usize;
        let ramp: Vec<f32> = (0..n).map(|i| i as f32 / n as f32).collect();
        let data = Arc::new(SampleData { left: ramp.clone(), right: ramp, rate: 48000 });
        let mut bank = SamplerBank::new(48000.0);
        let cfg = SlotConfig { reverse: true, scrub: 0.5, attack: 0.001, ..Default::default() };
        bank.set_slot(0, SamplerSlot { data, cfg });
        bank.note_on(0, 60, 1.0);
        let mut v = 0.0;
        for _ in 0..480 {
            v = bank.render_next(1.0).0;
        }
        let v = v / (PROGRAM_V * cfg.gain);
        assert!(
            (v - 0.5).abs() < 0.05,
            "reverse scrub 0.5 should read near ramp level 0.5, got {v}"
        );
    }

    /// WAVE_FORMAT_EXTENSIBLE (the modern DAW default) decodes via its
    /// SubFormat GUID.
    #[test]
    fn extensible_wav_loads() {
        let rate: u32 = 44100;
        let n = 64usize;
        let mut d = Vec::new();
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&(4 + 8 + 40 + 8 + n as u32 * 2).to_le_bytes());
        d.extend_from_slice(b"WAVEfmt ");
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&0xFFFEu16.to_le_bytes()); // extensible
        d.extend_from_slice(&1u16.to_le_bytes()); // mono
        d.extend_from_slice(&rate.to_le_bytes());
        d.extend_from_slice(&(rate * 2).to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&16u16.to_le_bytes());
        d.extend_from_slice(&22u16.to_le_bytes()); // cbSize
        d.extend_from_slice(&16u16.to_le_bytes()); // valid bits
        d.extend_from_slice(&0u32.to_le_bytes()); // channel mask
        d.extend_from_slice(&1u16.to_le_bytes()); // SubFormat: PCM
        d.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71]);
        d.extend_from_slice(b"data");
        d.extend_from_slice(&(n as u32 * 2).to_le_bytes());
        for i in 0..n {
            d.extend_from_slice(&((i as i16) * 100).to_le_bytes());
        }
        let path = std::env::temp_dir().join("patina-extensible-test.wav");
        std::fs::write(&path, d).unwrap();
        let s = load_wav_stereo(path.to_str().unwrap()).unwrap();
        assert_eq!(s.rate, 44100);
        assert_eq!(s.frames(), n);
        assert!((s.left[10] - 1000.0 / 32768.0).abs() < 1e-4);
    }

    /// Worst-case throughput: every head sounding, all reading through
    /// the widest stretched kernel (+24 semis = 4x stretch, 64 taps).
    /// Run by hand: cargo test --release perf_worst_case -- --ignored --nocapture
    #[test]
    #[ignore]
    fn perf_worst_case() {
        let mut bank = SamplerBank::new(48000.0);
        bank.set_slot(0, SamplerSlot {
            data: sine_reel(48000, 30.0, 440.0),
            cfg: SlotConfig {
                loop_pts: Some((0.5, 29.0)),
                xfade: 0.2,
                cutoff: 2000.0,
                res: 0.3,
                ..Default::default()
            },
        });
        for i in 0..24 {
            bank.note_on(0, 84 + (i % 3) as u8, 1.0); // +24..+26: full stretch
        }
        let n = 48000 * 4;
        let t = std::time::Instant::now();
        let mut acc = 0.0f32;
        for _ in 0..n {
            acc += bank.render_next(1.0).0;
        }
        let secs = t.elapsed().as_secs_f64();
        println!(
            "24 heads at 4x stretch: {:.1}x realtime (sink {acc})",
            (n as f64 / 48000.0) / secs
        );
    }

    /// A loop shorter than one read step must still LOOP. The wrap has to
    /// fold the whole overshoot back, or every wrap lands further past the
    /// loop end than the last and the head walks off the reel — and a
    /// looping head is never checked for end-of-tape, so it stays there
    /// forever, silent (the edge declick reads zero past the region).
    #[test]
    fn a_loop_narrower_than_the_step_still_loops() {
        let n = 48000usize;
        let ramp: Vec<f32> = (0..n).map(|i| i as f32 / n as f32).collect();
        let data = Arc::new(SampleData { left: ramp.clone(), right: ramp, rate: 48000 });
        let mut bank = SamplerBank::new(48000.0);
        // ~9.6-frame loop at the middle of the reel, read 32 frames a step
        // (no crossfade, so the probe reads the ramp itself)
        let cfg = SlotConfig {
            loop_pts: Some((0.5, 0.5002)),
            xfade: 0.0,
            speed: 32.0,
            attack: 0.001,
            ..Default::default()
        };
        bank.set_slot(0, SamplerSlot { data, cfg });
        bank.note_on(0, 60, 1.0);
        for _ in 0..2000 {
            bank.render_next(1.0); // reach and pass the loop point
        }
        for _ in 0..20000 {
            let v = bank.render_next(1.0).0 / (PROGRAM_V * cfg.gain);
            assert!(
                (v - 0.5).abs() < 0.02,
                "the head left its loop: read {v:.3}, loop sits at ramp level 0.5"
            );
        }
        assert!(bank.any_active(), "looped head died");
    }

    /// A short reel sliced into many pads: every slice can be under one
    /// source frame. The latched loop must clamp into that sliver without
    /// asking f64::clamp for an inverted range (min > max = panic).
    #[test]
    fn sub_frame_chop_slices_survive_a_loop() {
        let n = 64usize; // 64 frames / 128 pads = half a frame per pad
        let s = vec![0.5f32; n];
        let data = Arc::new(SampleData { left: s.clone(), right: s, rate: 48000 });
        let mut bank = SamplerBank::new(48000.0);
        let cfg = SlotConfig {
            chop: 128,
            loop_pts: Some((0.0, f32::MAX)), // what bare `loop` parses to
            ..Default::default()
        };
        bank.set_slot(0, SamplerSlot { data, cfg });
        for note in [60u8, 61, 127] {
            bank.note_on(0, note, 1.0);
            for _ in 0..256 {
                let (l, r) = bank.render_next(1.0);
                assert!(l.is_finite() && r.is_finite());
            }
        }
    }

    /// A reel too short to hold a single pitch period yields no epochs;
    /// psola playback must decline rather than index an empty table.
    #[test]
    fn psola_on_a_two_frame_reel_does_not_panic() {
        let data = Arc::new(SampleData {
            left: vec![0.2, -0.2],
            right: vec![0.2, -0.2],
            rate: 48000,
        });
        let mut bank = SamplerBank::new(48000.0);
        bank.set_slot(0, SamplerSlot {
            data,
            cfg: SlotConfig { psola: true, ..Default::default() },
        });
        bank.note_on(0, 67, 1.0);
        for _ in 0..1000 {
            let (l, r) = bank.render_next(1.0);
            assert!(l.is_finite() && r.is_finite());
        }
    }

    /// A malformed reel claiming 0 Hz would divide the epoch walker by
    /// zero and loop forever; the loader must reject it first.
    #[test]
    fn zero_rate_wav_is_rejected() {
        let n = 32usize;
        let mut d = Vec::new();
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&(4 + 8 + 16 + 8 + n as u32 * 2).to_le_bytes());
        d.extend_from_slice(b"WAVEfmt ");
        d.extend_from_slice(&16u32.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes()); // PCM
        d.extend_from_slice(&1u16.to_le_bytes()); // mono
        d.extend_from_slice(&0u32.to_le_bytes()); // sample rate 0
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&16u16.to_le_bytes());
        d.extend_from_slice(b"data");
        d.extend_from_slice(&(n as u32 * 2).to_le_bytes());
        d.extend_from_slice(&vec![0u8; n * 2]);
        let path = std::env::temp_dir().join("patina-zero-rate-test.wav");
        std::fs::write(&path, d).unwrap();
        assert!(load_wav_stereo(path.to_str().unwrap()).is_err());
    }

    /// Velocity and automation arrive unvalidated (a song may write
    /// `@NaN` or `pitch=NaN`). f32::clamp passes NaN straight through, so
    /// an unguarded write poisons the head's gain or read position for
    /// the life of the plugin — every later note mixes into NaN too.
    #[test]
    fn non_finite_input_cannot_poison_the_deck() {
        let mut bank = bank_with(SlotConfig { attack: 0.001, ..Default::default() });
        bank.note_on(0, 60, f32::NAN);
        for _ in 0..4800 {
            let (l, r) = bank.render_next(1.0);
            assert!(l.is_finite() && r.is_finite(), "NaN velocity poisoned the mix");
        }

        let mut bank = bank_with(SlotConfig { attack: 0.001, ..Default::default() });
        bank.note_on(0, 60, 1.0);
        bank.set_param(0, Param::SmpPitch, f32::NAN);
        bank.set_param(0, Param::SmpStart, f32::NAN);
        for _ in 0..4800 {
            let (l, r) = bank.render_next(1.0);
            assert!(l.is_finite() && r.is_finite(), "NaN automation poisoned the mix");
        }
        // And a head that somehow already holds a non-finite transport
        // must die instead of sounding forever
        let mut bank = bank_with(SlotConfig { pitch_semis: f32::NAN, ..Default::default() });
        bank.note_on(0, 60, 1.0);
        for _ in 0..4800 {
            let (l, r) = bank.render_next(1.0);
            assert!(l.is_finite() && r.is_finite(), "NaN slot config poisoned the mix");
        }
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

    #[test]
    fn psola_shifts_pitch_but_not_formant_or_duration() {
        // a 150 Hz pulse train ringing a 1400 Hz one-pole resonator:
        // pitch lives in the pulse spacing, the formant in the ring
        let rate = 48000usize;
        let dur = 0.6f32;
        let n = (rate as f32 * dur) as usize;
        let mut x = vec![0.0f32; n];
        let period = rate / 150;
        let mut ring = 0.0f32;
        let mut ring2 = 0.0f32;
        let w = std::f32::consts::TAU * 1400.0 / rate as f32;
        let rq = 0.995f32;
        for i in 0..n {
            let imp = if i % period == 0 { 1.0 } else { 0.0 };
            let y = imp + 2.0 * rq * w.cos() * ring - rq * rq * ring2;
            ring2 = ring;
            ring = y;
            x[i] = y * 0.05;
        }
        let data = std::sync::Arc::new(SampleData {
            left: x.clone(),
            right: x,
            rate: rate as u32,
        });
        let mut cfg = SlotConfig::default();
        cfg.root = 60;
        cfg.psola = true;
        cfg.attack = 0.001;
        cfg.release = 0.01;
        let mut bank = SamplerBank::new(rate as f32);
        bank.set_slot(0, SamplerSlot { data, cfg });
        bank.note_on(0, 67, 1.0); // +7 st
        let mut out = Vec::with_capacity(n + rate / 2);
        for _ in 0..(n + rate / 4) {
            let (l, _r) = bank.render_next(1.0);
            out.push(l);
        }
        // duration: energy must persist to ~90% of natural length
        let tail = &out[(n as f32 * 0.85) as usize..(n as f32 * 0.95) as usize];
        let tail_rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
        assert!(tail_rms > 1e-4, "psola must preserve duration, tail rms {tail_rms}");
        // pitch: autocorrelation over the middle
        let seg = &out[rate / 5..rate / 5 + 4096];
        let mean = seg.iter().sum::<f32>() / seg.len() as f32;
        let mut best = 0usize;
        let mut best_c = 0.0f32;
        for lag in (rate / 400)..(rate / 90) {
            let mut c = 0.0f32;
            for i in 0..(seg.len() - lag) {
                c += (seg[i] - mean) * (seg[i + lag] - mean);
            }
            if c > best_c {
                best_c = c;
                best = lag;
            }
        }
        let f0 = rate as f32 / best as f32;
        let target = 150.0 * (7.0f32 / 12.0).exp2();
        assert!(
            (f0 / target).log2().abs() < 0.12,
            "psola pitch: got {f0:.1} Hz, want ~{target:.1}"
        );
        // formant: spectral peak in 900..2200 must stay near 1400 Hz
        let m = 8192.min(seg.len());
        let mut best_f = 0.0f32;
        let mut best_e = 0.0f32;
        let mut f = 900.0f32;
        while f < 2200.0 {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, s) in seg[..m].iter().enumerate() {
                let ph = std::f32::consts::TAU * f * i as f32 / rate as f32;
                re += s * ph.cos();
                im += s * ph.sin();
            }
            let e = re * re + im * im;
            if e > best_e {
                best_e = e;
                best_f = f;
            }
            f += 25.0;
        }
        assert!(
            (best_f - 1400.0).abs() < 220.0,
            "psola must hold the formant: peak at {best_f:.0} Hz, want ~1400"
        );
    }
}
