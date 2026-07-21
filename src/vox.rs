// The voice of the machine — a formant speech synthesizer, plus the
// plumbing that lets it talk through the vocoder.
//
// The voice here is built the way speech was built before data: a glottal
// pulse train (Rosenberg's two-piece flow model, 1971) and a noise source
// excite a cascade of second-order resonators standing in for the vocal
// tract — Dennis Klatt's cascade formant synthesizer (JASA 1980), the
// architecture inside DECtalk and, before it, the hand-operated Voder.
// Nothing is sampled and nothing is learned: every phoneme is a column of
// numbers (formant frequencies, bandwidths, source amplitudes), so every
// aspect of the delivery — pitch, duration, loudness, breath — is a knob
// that stays live down to the individual phoneme.
//
// In a song, a vox track's notes carry lyrics as ARPAbet phonemes:
//
//   track vocal vox
//     C4:2=HH-EH   G4:2=L-OW:300@0.8
//
// The syllable rides the note: onset consonants speak at note-on, the
// vowel sustains while the key is held (pitch follows the lowest held
// note), and coda consonants speak at note-off — which is how singers
// treat codas too. `:ms` fixes any phoneme's length in milliseconds
// (a vowel given an explicit length stops sustaining); `@amp` scales its
// loudness. Note velocity scales the whole syllable.
//
// The speech itself normally never reaches the output: it is the
// MODULATOR of the channel vocoder (vocoder.rs), articulating the synth
// voices that the same notes play as the CARRIER. `vox_dry` brings up the
// raw voice itself; `wav=` on the track replaces the internal voice with
// any recorded voice as modulator.

use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};

use crate::noise::NoiseSource;
use crate::oscillator::PROGRAM_V;
use crate::vocoder::Vocoder;

/// Reserved song channel for the voice box (the drums own u16::MAX).
pub const VOX_CHANNEL: u16 = u16::MAX - 1;

/// Fourth formant: fixed, like the higher tract resonances it stands for.
const F4: f32 = 3300.0;
const B4: f32 = 280.0;

/// A vowel with no explicit length inside a multi-vowel syllable.
const INNER_VOWEL_MS: f32 = 160.0;

// ---------------------------------------------------------------------------
// Phonemes
// ---------------------------------------------------------------------------

/// ARPAbet, the same alphabet CMUdict uses. Vowels and diphthongs first,
/// then glides/liquids, nasals, fricatives, stops, affricates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[rustfmt::skip]
pub enum Phoneme {
    AA, AE, AH, AO, EH, ER, IH, IY, UH, UW,
    AY, AW, EY, OW, OY,
    W, Y, L, R,
    M, N, NG,
    F, TH, S, SH, HH,
    V, DH, Z, ZH,
    P, B, T, D, K, G,
    CH, JH,
}

/// One phoneme of a lyric with its per-phoneme overrides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LyricPhone {
    pub ph: Phoneme,
    /// Explicit duration in milliseconds; None = the phoneme's own timing
    /// (and vowels sustain to the note-off).
    pub ms: Option<f32>,
    /// Loudness scale for this phoneme alone.
    pub amp: f32,
    /// CMUdict-style stress on vowels: 1 primary, 2 secondary, 0 reduced.
    /// None = let the prosody planner decide (first vowel primary).
    pub stress: Option<u8>,
}

/// How a syllable relates to its phrase edge, from lyric punctuation:
/// `=HH-OW-M.` falls (statement), `=HH-OW-M?` rises (question).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    None,
    Fall,
    Rise,
}

/// A parsed lyric: the phonemes plus any phrase-boundary mark.
#[derive(Clone, Debug, PartialEq)]
pub struct Syllable {
    pub phones: Vec<LyricPhone>,
    pub boundary: Boundary,
}

/// Continuant targets: formants, bandwidths, source amplitudes, timing.
#[derive(Clone, Copy)]
struct Spec {
    f: [f32; 3],
    bw: [f32; 3],
    voiced: f32,
    fric: f32,
    fric_f: f32,
    fric_bw: f32,
    asp: f32,
    ms: f32,
    slew_ms: f32,
}

impl Phoneme {
    pub fn from_name(name: &str) -> Option<Self> {
        use Phoneme::*;
        Some(match name.to_ascii_uppercase().as_str() {
            "AA" => AA, "AE" => AE, "AH" => AH, "AO" => AO, "EH" => EH,
            "ER" => ER, "IH" => IH, "IY" => IY, "UH" => UH, "UW" => UW,
            "AY" => AY, "AW" => AW, "EY" => EY, "OW" => OW, "OY" => OY,
            "W" => W, "Y" => Y, "L" => L, "R" => R,
            "M" => M, "N" => N, "NG" => NG,
            "F" => F, "TH" => TH, "S" => S, "SH" => SH, "HH" => HH,
            "V" => V, "DH" => DH, "Z" => Z, "ZH" => ZH,
            "P" => P, "B" => B, "T" => T, "D" => D, "K" => K, "G" => G,
            "CH" => CH, "JH" => JH,
            _ => return None,
        })
    }

    /// Does this phoneme carry the syllable (sustain under a held note)?
    pub fn is_vowel(self) -> bool {
        use Phoneme::*;
        matches!(self, AA | AE | AH | AO | EH | ER | IH | IY | UH | UW
            | AY | AW | EY | OW | OY)
    }

    /// Diphthongs glide from their Spec targets to a second vowel's.
    fn glide_to(self) -> Option<[f32; 3]> {
        use Phoneme::*;
        Some(match self {
            AY | OY => Phoneme::IY.spec().f, // "eye", "boy" end high-front
            AW => Phoneme::UW.spec().f,      // "how" ends high-back
            EY => Phoneme::IY.spec().f,      // "day"
            OW => Phoneme::UW.spec().f,      // "go"
            _ => return None,
        })
    }

    /// Stops: (closure ms, voiced bar, burst center, burst bw, burst amp).
    fn stop(self) -> Option<(f32, bool, f32, f32, f32)> {
        use Phoneme::*;
        Some(match self {
            P => (55.0, false, 900.0, 1600.0, 0.55),
            B => (50.0, true, 900.0, 1600.0, 0.45),
            T => (55.0, false, 4400.0, 2600.0, 0.8),
            D => (45.0, true, 4400.0, 2600.0, 0.6),
            K => (60.0, false, 1990.0, 1500.0, 0.75),
            G => (50.0, true, 1990.0, 1500.0, 0.6),
            // Affricates: a stop closure released into a long SH/ZH hiss
            CH => (60.0, false, 2300.0, 1300.0, 0.9),
            JH => (50.0, true, 2300.0, 1300.0, 0.7),
            _ => return None,
        })
    }

    /// Formant and source targets. Vowel formants are the classic male
    /// averages (Peterson & Barney 1952, as tabulated by Klatt); consonant
    /// loci and frication spectra follow Klatt 1980's synthesis tables.
    #[rustfmt::skip]
    fn spec(self) -> Spec {
        use Phoneme::*;
        // (f1 f2 f3, bw1 bw2 bw3, voiced, fric, fric_f, fric_bw, asp, ms, slew)
        let t = |f: [f32; 3], bw: [f32; 3], voiced: f32, fric: f32,
                 fric_f: f32, fric_bw: f32, asp: f32, ms: f32, slew_ms: f32| Spec {
            f, bw, voiced, fric, fric_f, fric_bw, asp, ms, slew_ms,
        };
        match self {
            // Vowels: sustained, fully voiced
            AA => t([730.0, 1090.0, 2440.0], [90.0, 100.0, 150.0], 1.0, 0.0, 0.0, 1000.0, 0.02, 180.0, 45.0),
            AE => t([660.0, 1720.0, 2410.0], [80.0, 100.0, 150.0], 1.0, 0.0, 0.0, 1000.0, 0.02, 180.0, 45.0),
            AH => t([640.0, 1190.0, 2390.0], [80.0, 100.0, 150.0], 1.0, 0.0, 0.0, 1000.0, 0.02, 150.0, 45.0),
            AO => t([570.0,  840.0, 2410.0], [80.0, 100.0, 150.0], 1.0, 0.0, 0.0, 1000.0, 0.02, 180.0, 45.0),
            EH => t([530.0, 1840.0, 2480.0], [70.0, 100.0, 150.0], 1.0, 0.0, 0.0, 1000.0, 0.02, 170.0, 45.0),
            ER => t([490.0, 1350.0, 1690.0], [80.0, 100.0, 140.0], 1.0, 0.0, 0.0, 1000.0, 0.02, 180.0, 45.0),
            IH => t([390.0, 1990.0, 2550.0], [60.0,  90.0, 150.0], 1.0, 0.0, 0.0, 1000.0, 0.02, 140.0, 45.0),
            IY => t([270.0, 2290.0, 3010.0], [50.0,  90.0, 160.0], 1.0, 0.0, 0.0, 1000.0, 0.02, 170.0, 45.0),
            UH => t([440.0, 1020.0, 2240.0], [70.0, 100.0, 150.0], 1.0, 0.0, 0.0, 1000.0, 0.02, 140.0, 45.0),
            UW => t([300.0,  870.0, 2240.0], [60.0, 100.0, 150.0], 1.0, 0.0, 0.0, 1000.0, 0.02, 180.0, 45.0),
            // Diphthongs start at their first element (glide_to has the rest)
            AY | AW => Spec { ms: 220.0, ..AA.spec() },
            EY      => Spec { ms: 200.0, ..EH.spec() },
            OW | OY => Spec { ms: 220.0, ..AO.spec() },
            // Glides and liquids: voiced, slow-slewing formant gestures
            W  => t([300.0,  610.0, 2200.0], [60.0,  90.0, 150.0], 0.9, 0.0, 0.0, 1000.0, 0.0, 75.0, 55.0),
            Y  => t([270.0, 2290.0, 3010.0], [50.0,  90.0, 160.0], 0.9, 0.0, 0.0, 1000.0, 0.0, 65.0, 55.0),
            L  => t([360.0, 1300.0, 2700.0], [70.0, 100.0, 160.0], 0.9, 0.0, 0.0, 1000.0, 0.0, 70.0, 40.0),
            R  => t([310.0, 1060.0, 1380.0], [70.0, 100.0, 120.0], 0.9, 0.0, 0.0, 1000.0, 0.0, 80.0, 55.0),
            // Nasals: murmur through a damped tract
            M  => t([280.0,  900.0, 2200.0], [120.0, 200.0, 250.0], 0.55, 0.0, 0.0, 1000.0, 0.0, 70.0, 25.0),
            N  => t([280.0, 1700.0, 2600.0], [120.0, 200.0, 250.0], 0.55, 0.0, 0.0, 1000.0, 0.0, 65.0, 25.0),
            NG => t([280.0, 2300.0, 2750.0], [120.0, 200.0, 250.0], 0.55, 0.0, 0.0, 1000.0, 0.0, 90.0, 25.0),
            // Voiceless fricatives: shaped noise, no voicing
            F  => t([340.0, 1100.0, 2080.0], [90.0, 120.0, 180.0], 0.0, 0.35, 4500.0, 3500.0, 0.0, 95.0, 25.0),
            TH => t([320.0, 1290.0, 2540.0], [90.0, 120.0, 180.0], 0.0, 0.3, 5000.0, 3500.0, 0.0, 85.0, 25.0),
            S  => t([320.0, 1390.0, 2530.0], [90.0, 120.0, 180.0], 0.0, 1.0, 6000.0, 2600.0, 0.0, 105.0, 25.0),
            SH => t([300.0, 1840.0, 2750.0], [90.0, 120.0, 180.0], 0.0, 1.0, 2300.0, 1300.0, 0.0, 105.0, 25.0),
            HH => t([500.0, 1500.0, 2500.0], [150.0, 200.0, 250.0], 0.0, 0.0, 0.0, 1000.0, 0.7, 75.0, 30.0),
            // Voiced fricatives: buzz and noise together
            V  => t([340.0, 1100.0, 2080.0], [90.0, 120.0, 180.0], 0.55, 0.25, 4500.0, 3500.0, 0.0, 60.0, 25.0),
            DH => t([320.0, 1290.0, 2540.0], [90.0, 120.0, 180.0], 0.55, 0.2, 5000.0, 3500.0, 0.0, 50.0, 25.0),
            Z  => t([320.0, 1390.0, 2530.0], [90.0, 120.0, 180.0], 0.5, 0.7, 6000.0, 2600.0, 0.0, 85.0, 25.0),
            ZH => t([300.0, 1840.0, 2750.0], [90.0, 120.0, 180.0], 0.5, 0.7, 2300.0, 1300.0, 0.0, 85.0, 25.0),
            // Stops carry their vowel-transition locus here; closure and
            // burst are built as separate segments
            P | B => t([400.0, 1100.0, 2150.0], [90.0, 120.0, 180.0], 0.0, 0.0, 0.0, 1000.0, 0.0, 12.0, 8.0),
            T | D => t([320.0, 1800.0, 2600.0], [90.0, 120.0, 180.0], 0.0, 0.0, 0.0, 1000.0, 0.0, 12.0, 8.0),
            K | G => t([300.0, 1990.0, 2850.0], [90.0, 120.0, 180.0], 0.0, 0.0, 0.0, 1000.0, 0.0, 14.0, 8.0),
            CH | JH => t([300.0, 1840.0, 2750.0], [90.0, 120.0, 180.0], 0.0, 0.0, 0.0, 1000.0, 0.0, 70.0, 8.0),
        }
    }
}

/// Parse a lyric: dash-separated ARPAbet, each phoneme optionally
/// carrying a stress digit (vowels: `OW1`, `AH0`), `:ms` (length in
/// milliseconds), and/or `@amp` — `HH-OW1:180@0.7-L-D`. A trailing `.`
/// marks a phrase-final fall, `?` a rise: `=HH-OW1-M.`
pub fn parse_lyric(s: &str) -> Result<Syllable, String> {
    let (s, boundary) = if let Some(r) = s.strip_suffix('.') {
        (r, Boundary::Fall)
    } else if let Some(r) = s.strip_suffix('?') {
        (r, Boundary::Rise)
    } else {
        (s.strip_suffix(',').unwrap_or(s), Boundary::None)
    };
    let mut out = Vec::new();
    for raw in s.split('-').filter(|t| !t.is_empty()) {
        let mut part = raw;
        let mut amp = 1.0f32;
        let mut ms = None;
        if let Some(i) = part.rfind('@') {
            amp = part[i + 1..]
                .parse::<f32>()
                .map_err(|_| format!("lyric '{}': bad amplitude", raw))?;
            part = &part[..i];
        }
        if let Some(i) = part.rfind(':') {
            let v = part[i + 1..]
                .parse::<f32>()
                .map_err(|_| format!("lyric '{}': bad duration (milliseconds)", raw))?;
            if v <= 0.0 {
                return Err(format!("lyric '{}': duration must be positive", raw));
            }
            ms = Some(v);
            part = &part[..i];
        }
        let (name, stress) = match part.as_bytes().last() {
            Some(c) if c.is_ascii_digit() && part.len() > 1 => {
                let d = c - b'0';
                if d > 2 {
                    return Err(format!("lyric '{}': stress is 0, 1, or 2", raw));
                }
                (&part[..part.len() - 1], Some(d))
            }
            _ => (part, None),
        };
        let ph = Phoneme::from_name(name).ok_or_else(|| {
            format!(
                "unknown phoneme '{}' (ARPAbet: AA AE AH AO EH ER IH IY UH UW \
                 AY AW EY OW OY W Y L R M N NG F TH S SH HH V DH Z ZH P B T D K G CH JH)",
                name
            )
        })?;
        if stress.is_some() && !ph.is_vowel() {
            return Err(format!("lyric '{}': stress digits go on vowels", raw));
        }
        out.push(LyricPhone { ph, ms, amp: amp.clamp(0.0, 2.0), stress });
    }
    if out.is_empty() {
        return Err("empty lyric".into());
    }
    Ok(Syllable { phones: out, boundary })
}

// ---------------------------------------------------------------------------
// The synthesizer
// ---------------------------------------------------------------------------

/// Klatt digital resonator: a two-pole bandpass with unity gain at its
/// center, the standard vocal-tract building block.
#[derive(Clone, Copy, Default)]
struct Resonator {
    b: f32,
    c: f32,
    a: f32,
    y1: f32,
    y2: f32,
}

impl Resonator {
    #[inline]
    fn set(&mut self, f: f32, bw: f32, sample_rate: f32) {
        let c = -(-TAU * bw / sample_rate).exp();
        let b = 2.0 * (-PI * bw / sample_rate).exp() * (TAU * f / sample_rate).cos();
        self.b = b;
        self.c = c;
        self.a = 1.0 - b - c;
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.a * x + self.b * self.y1 + self.c * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// One scheduled stretch of sound: targets the tract slews toward, and how
/// long they hold. `SUSTAIN` = until the note lifts.
#[derive(Clone, Copy)]
struct Seg {
    f: [f32; 3],
    bw: [f32; 3],
    voiced: f32,
    fric: f32,
    fric_f: f32,
    fric_bw: f32,
    asp: f32,
    dur: usize,
    glide_to: Option<[f32; 3]>,
    slew_ms: f32,
    /// Pitch-accent bump (semitones) fired when this segment begins —
    /// the stressed-syllable rise-fall.
    accent: f32,
}

const SUSTAIN: usize = usize::MAX;

pub struct VoxSource {
    sample_rate: f32,
    // The score: current segment, upcoming segments, and the coda held
    // back for note-off
    cur: Option<Seg>,
    pos: usize,
    queue: VecDeque<Seg>,
    coda: Vec<Seg>,
    pending: Option<Syllable>,
    held: Vec<u8>,
    // Prosody: where we are in the phrase and what the pitch is doing
    // about it. All of it scales with the `intonation` knob.
    intonation: f32,
    syl_index: u32,
    phrase_done: bool,
    boundary: Boundary,
    decl_st: f32,   // declination, semitones (slewed via f0_target)
    accent_st: f32, // pitch-accent envelope, decaying
    fall_st: f32,   // phrase-final fall/rise envelope
    fall_target: f32,
    // Smoothed tract state
    f: [f32; 3],
    bw: [f32; 3],
    voiced: f32,
    fric: f32,
    fric_f: f32,
    fric_bw: f32,
    asp: f32,
    // Source state
    f0: f32,
    f0_target: f32,
    phase: f32,
    g_prev: f32,
    vib_phase: f32,
    vibrato: f32, // 0..1 panel knob -> cents inside
    jitter_lp: f32,
    shimmer_lp: f32,
    breath: f32,
    /// Spectral-tilt lowpass on the glottal pulse: vocal effort opens it
    /// (louder = brighter), softness closes it.
    tilt_lp: f32,
    noise: NoiseSource,
    res: [Resonator; 4],
    fric_res: Resonator,
}

impl VoxSource {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            cur: None,
            pos: 0,
            queue: VecDeque::new(),
            coda: Vec::new(),
            pending: None,
            held: Vec::new(),
            intonation: 0.12,
            syl_index: 0,
            phrase_done: true,
            boundary: Boundary::None,
            decl_st: 0.0,
            accent_st: 0.0,
            fall_st: 0.0,
            fall_target: 0.0,
            f: [500.0, 1500.0, 2500.0],
            bw: [90.0, 110.0, 160.0],
            voiced: 0.0,
            fric: 0.0,
            fric_f: 4000.0,
            fric_bw: 2000.0,
            asp: 0.0,
            f0: 110.0,
            f0_target: 110.0,
            phase: 0.0,
            g_prev: 0.0,
            vib_phase: 0.0,
            vibrato: 0.25,
            jitter_lp: 0.0,
            shimmer_lp: 0.0,
            breath: 0.12,
            tilt_lp: 0.0,
            noise: NoiseSource::new(),
            res: [Resonator::default(); 4],
            fric_res: Resonator::default(),
        }
    }

    pub fn set_breath(&mut self, v: f32) {
        self.breath = v.clamp(0.0, 1.0);
    }

    pub fn set_vibrato(&mut self, v: f32) {
        self.vibrato = v.clamp(0.0, 1.0);
    }

    /// How much the voice performs on its own: pitch accents on stressed
    /// syllables, declination across the phrase, final falls and rises.
    /// Low for singing (the notes are the melody), high for speech.
    pub fn set_intonation(&mut self, v: f32) {
        self.intonation = v.clamp(0.0, 1.0);
    }

    /// The next note-on will sing this. Set from the song's lyric events,
    /// which land just before their note-ons.
    pub fn set_syllable(&mut self, syl: Syllable) {
        self.pending = Some(syl);
    }

    pub fn note_on(&mut self, note: u8, velocity: f32) {
        if !self.held.contains(&note) {
            self.held.push(note);
        }
        self.retune();
        if let Some(syl) = self.pending.take() {
            // Phrase bookkeeping: the count restarts after a boundary
            // syllable, and declination steps down as the phrase goes on
            if self.phrase_done {
                self.syl_index = 0;
                self.phrase_done = false;
                self.fall_st = 0.0;
                self.fall_target = 0.0;
            } else {
                self.syl_index += 1;
            }
            self.decl_st =
                (0.8 - 0.35 * self.syl_index as f32).max(-2.2) * self.intonation;
            self.boundary = syl.boundary;

            let gain = 0.4 + 0.6 * velocity.clamp(0.0, 1.0);
            let (main, coda) = self.build_syllable(&syl.phones, gain);
            // A new syllable interrupts whatever was still queued (fast
            // passages drop their codas, like a hurried singer)
            self.queue.clear();
            if let Some(cur) = &mut self.cur {
                if cur.dur == SUSTAIN {
                    cur.dur = self.pos; // end the old vowel now
                }
            }
            self.queue.extend(main);
            self.coda = coda;
        }
    }

    pub fn note_off(&mut self, note: u8) {
        self.held.retain(|&n| n != note);
        if !self.held.is_empty() {
            self.retune();
            return;
        }
        // Last key up: close the syllable — finish the vowel, speak the coda
        if let Some(cur) = &mut self.cur {
            if cur.dur == SUSTAIN {
                cur.dur = self.pos + (0.02 * self.sample_rate) as usize;
            }
        }
        let min_vowel = (0.09 * self.sample_rate) as usize;
        for seg in self.queue.iter_mut() {
            if seg.dur == SUSTAIN {
                seg.dur = min_vowel; // staccato: the vowel still speaks, briefly
            }
        }
        let mut coda = std::mem::take(&mut self.coda);
        // Phrase edges: the pitch falls (or rises) through the coda, and
        // a statement-final coda stretches — phrase-final lengthening
        match self.boundary {
            Boundary::Fall => {
                self.fall_target = -3.5 * self.intonation;
                self.phrase_done = true;
                for seg in &mut coda {
                    if seg.dur != SUSTAIN {
                        seg.dur = (seg.dur as f32 * 1.4) as usize;
                    }
                }
            }
            Boundary::Rise => {
                self.fall_target = 4.0 * self.intonation;
                self.phrase_done = true;
            }
            Boundary::None => {}
        }
        self.boundary = Boundary::None;
        self.queue.extend(coda);
    }

    /// Speech pitch follows the lowest held key — the voice sits on the
    /// root while the carrier chord stacks above it.
    fn retune(&mut self) {
        if let Some(&low) = self.held.iter().min() {
            let hz = 440.0 * ((low as f32 - 69.0) / 12.0).exp2();
            self.f0_target = hz.clamp(50.0, 350.0);
        }
    }

    /// Stress → loudness, length, and pitch-accent size.
    fn stress_shape(&self, stress: Option<u8>) -> (f32, f32, f32) {
        match stress.unwrap_or(1) {
            0 => (0.78, 0.7, 0.0),
            2 => (1.0, 1.05, 1.2 * self.intonation),
            _ => (1.15, 1.3, 2.4 * self.intonation),
        }
    }

    fn seg_from(&self, lp: &LyricPhone, dur: usize) -> Seg {
        let s = lp.ph.spec();
        let (amp_scale, _, accent) = if lp.ph.is_vowel() {
            self.stress_shape(lp.stress)
        } else {
            (1.0, 1.0, 0.0)
        };
        Seg {
            f: s.f,
            bw: s.bw,
            voiced: s.voiced * lp.amp * amp_scale,
            fric: s.fric * lp.amp,
            fric_f: s.fric_f,
            fric_bw: s.fric_bw,
            asp: s.asp * lp.amp,
            dur,
            glide_to: lp.ph.glide_to(),
            slew_ms: s.slew_ms,
            accent,
        }
    }

    fn push_phone(&self, out: &mut Vec<Seg>, lp: &LyricPhone, sustains: bool) {
        let ms_to_samples = |ms: f32| (ms * 0.001 * self.sample_rate) as usize;
        if let Some((closure_ms, voiced_bar, bf, bbw, bamp)) = lp.ph.stop() {
            // Closure: silence (or the low voiced bar of B/D/G) at the
            // stop's articulation locus, so formants transition INTO the
            // closure the way they do out of it
            let locus = lp.ph.spec();
            out.push(Seg {
                f: locus.f,
                bw: locus.bw,
                voiced: if voiced_bar { 0.12 * lp.amp } else { 0.0 },
                fric: 0.0,
                fric_f: bf,
                fric_bw: bbw,
                asp: 0.0,
                dur: ms_to_samples(lp.ms.unwrap_or(closure_ms)),
                glide_to: None,
                slew_ms: 15.0,
                accent: 0.0,
            });
            // Burst: a short noise transient shaped by the place of
            // articulation (affricates hold theirs much longer)
            out.push(Seg {
                f: locus.f,
                bw: locus.bw,
                voiced: 0.0,
                fric: bamp * lp.amp,
                fric_f: bf,
                fric_bw: bbw,
                asp: 0.0,
                dur: ms_to_samples(locus.ms),
                glide_to: None,
                slew_ms: locus.slew_ms,
                accent: 0.0,
            });
            return;
        }
        let spec = lp.ph.spec();
        let dur = if let Some(ms) = lp.ms {
            ms_to_samples(ms)
        } else if sustains {
            SUSTAIN
        } else if lp.ph.is_vowel() {
            let (_, dur_scale, _) = self.stress_shape(lp.stress);
            ms_to_samples(INNER_VOWEL_MS * dur_scale)
        } else {
            ms_to_samples(spec.ms)
        };
        out.push(self.seg_from(lp, dur));
    }

    /// Split a lyric into what speaks at note-on (onset consonants plus
    /// the sustaining nucleus) and what waits for note-off (the coda).
    fn build_syllable(&self, phones: &[LyricPhone], gain: f32) -> (Vec<Seg>, Vec<Seg>) {
        // Unmarked stress defaults: the first vowel carries the syllable,
        // any further vowels reduce (English's trochaic habit)
        let mut first_vowel = true;
        let scaled: Vec<LyricPhone> = phones
            .iter()
            .map(|lp| {
                let stress = if lp.ph.is_vowel() {
                    let s = lp.stress.or(if first_vowel { Some(1) } else { Some(0) });
                    first_vowel = false;
                    s
                } else {
                    lp.stress
                };
                LyricPhone { amp: lp.amp * gain, stress, ..*lp }
            })
            .collect();
        let last_vowel = scaled.iter().rposition(|p| p.ph.is_vowel());
        let split = last_vowel.map(|i| i + 1).unwrap_or(scaled.len());

        let mut main = Vec::new();
        for (i, lp) in scaled[..split].iter().enumerate() {
            self.push_phone(&mut main, lp, Some(i) == last_vowel);
        }
        // No vowel at all ("S-S", a hiss pad): the final continuant sustains
        if last_vowel.is_none() && scaled.last().map_or(false, |lp| lp.ms.is_none()) {
            if let Some(seg) = main.last_mut() {
                seg.dur = SUSTAIN;
            }
        }
        let mut coda = Vec::new();
        for lp in &scaled[split..] {
            self.push_phone(&mut coda, lp, false);
        }
        (main, coda)
    }

    /// Rosenberg glottal flow: cosine opening over 40% of the period,
    /// quarter-cosine closing over 16%, closed the rest.
    #[inline]
    fn glottal_flow(t: f32) -> f32 {
        if t < 0.4 {
            0.5 * (1.0 - (PI * t / 0.4).cos())
        } else if t < 0.56 {
            (PI * (t - 0.4) / 0.32).cos()
        } else {
            0.0
        }
    }

    /// One mono sample of speech, unit-level (roughly ±1).
    pub fn render(&mut self) -> f32 {
        // Advance the score
        if self.cur.map_or(false, |s| s.dur != SUSTAIN && self.pos >= s.dur) {
            self.cur = None;
        }
        if self.cur.is_none() {
            if let Some(s) = self.queue.pop_front() {
                // A stressed nucleus fires its pitch accent as it begins;
                // the f0 slew shapes the rise, the decay below the fall
                if s.accent > self.accent_st {
                    self.accent_st = s.accent;
                }
                self.cur = Some(s);
                self.pos = 0;
            }
        }
        self.pos = self.pos.saturating_add(1);

        // Targets: the current segment's, or silence (formants hold their
        // last positions — the mouth doesn't snap shut)
        let (tf, tbw, mut tv, tfr, tff, tfbw, tasp, slew_ms) = match &self.cur {
            Some(s) => {
                let mut tf = s.f;
                let mut slew = s.slew_ms;
                if let Some(g) = s.glide_to {
                    // Diphthong: hold the first element, then drift
                    if self.pos > (0.15 * self.sample_rate) as usize {
                        tf = g;
                        slew = 90.0;
                    }
                }
                (tf, s.bw, s.voiced, s.fric, s.fric_f, s.fric_bw, s.asp, slew)
            }
            None => (self.f, self.bw, 0.0, 0.0, self.fric_f, self.fric_bw, 0.0, 25.0),
        };
        // Breath pressure eases over a long-held note — a sustained vowel
        // settles instead of holding organ-flat
        if self.cur.map_or(false, |s| s.dur == SUSTAIN) {
            tv *= 0.88 + 0.12 * (-(self.pos as f32) / (1.2 * self.sample_rate)).exp();
        }

        // Slew the tract toward its targets: formants at the segment's
        // pace, source amplitudes on a fast 12 ms envelope
        let kf = 1.0 - (-1000.0 / (slew_ms * self.sample_rate)).exp();
        let ka = 1.0 - (-1000.0 / (12.0 * self.sample_rate)).exp();
        for i in 0..3 {
            self.f[i] += (tf[i] - self.f[i]) * kf;
            self.bw[i] += (tbw[i] - self.bw[i]) * kf;
        }
        self.voiced += (tv - self.voiced) * ka;
        self.fric += (tfr - self.fric) * ka;
        self.asp += (tasp - self.asp) * ka;
        self.fric_f += (tff - self.fric_f) * kf;
        self.fric_bw += (tfbw - self.fric_bw) * kf;

        // Pitch: slew to the note, then let the prosody speak on top —
        // declination into the target (slewed with it), the decaying
        // pitch-accent bump, the phrase-final fall or rise — then vibrato
        // and the jitter no throat can suppress
        let n = self.noise.next();
        self.jitter_lp += 0.0015 * (n - self.jitter_lp);
        self.shimmer_lp += 0.0004 * (n - self.shimmer_lp);
        let target = self.f0_target * (self.decl_st / 12.0).exp2();
        self.f0 += (target - self.f0) * (1.0 - (-1000.0 / (35.0 * self.sample_rate)).exp());
        self.accent_st *= 1.0 - 1.0 / (0.2 * self.sample_rate); // ~200 ms decay
        self.fall_st +=
            (self.fall_target - self.fall_st) * (1.0 - (-1000.0 / (150.0 * self.sample_rate)).exp());
        self.vib_phase = (self.vib_phase + 5.3 / self.sample_rate).fract();
        let vib_cents = self.vibrato * 40.0 * (TAU * self.vib_phase).sin();
        let f0 = self.f0
            * ((self.accent_st + self.fall_st) / 12.0
                + vib_cents / 1200.0
                + self.jitter_lp * 0.012)
                .exp2();

        // Glottal source: differentiated Rosenberg pulse through the
        // effort-dependent tilt filter (soft voice = darker pulse), with
        // shimmer on the pulse amplitude and breath in the throat
        self.phase += f0 / self.sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let g = Self::glottal_flow(self.phase);
        let dg = (g - self.g_prev) * self.sample_rate / (f0 * 6.0);
        self.g_prev = g;
        let tilt_fc = 700.0 + 5200.0 * self.voiced.clamp(0.0, 1.0);
        let kt = 1.0 - (-TAU * tilt_fc / self.sample_rate).exp();
        self.tilt_lp += kt * (dg - self.tilt_lp);
        let shimmer = 1.0 + self.shimmer_lp * 0.35;
        let source = self.tilt_lp * self.voiced * shimmer
            + n * (self.asp + self.breath * self.voiced * 0.5);

        // The tract: cascade resonators F1-F3 moving, F4 fixed
        let mut x = source;
        for i in 0..3 {
            self.res[i].set(self.f[i], self.bw[i], self.sample_rate);
            x = self.res[i].tick(x);
        }
        self.res[3].set(F4, B4, self.sample_rate);
        x = self.res[3].tick(x);

        // Frication: noise through its own place-of-articulation resonator
        self.fric_res.set(self.fric_f, self.fric_bw, self.sample_rate);
        let hiss = self.fric_res.tick(n) * self.fric;

        // Voiced tract vs frication balance: vowels lead, but the
        // consonants stay hot enough to trip the vocoder's unvoiced
        // detector — diction beats strict realism here
        x * 0.28 + hiss * 0.45
    }

    /// True while anything is sounding or queued.
    pub fn speaking(&self) -> bool {
        self.cur.is_some() || !self.queue.is_empty()
    }

    /// The prosody's current pitch offset in semitones (test telemetry).
    #[cfg(test)]
    pub(crate) fn prosody_st(&self) -> f32 {
        self.decl_st + self.accent_st + self.fall_st
    }
}

// ---------------------------------------------------------------------------
// WAV modulator input
// ---------------------------------------------------------------------------

/// Minimal RIFF/WAVE reader: PCM16, PCM24, or float32, mono-summed.
/// Returns (samples, source sample rate).
pub fn load_wav_mono(path: &str) -> Result<(Vec<f32>, u32), String> {
    let data = std::fs::read(path).map_err(|e| format!("wav '{}': {}", path, e))?;
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("wav '{}': not a RIFF/WAVE file", path));
    }
    let mut pos = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None; // format, channels, rate, bits
    let mut samples: Option<Vec<f32>> = None;
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
                let (format, channels, _, bits) =
                    fmt.ok_or_else(|| format!("wav '{}': data before fmt", path))?;
                let ch = channels.max(1) as usize;
                let raw = &data[body..body + size];
                let mono: Vec<f32> = match (format, bits) {
                    (1, 16) => raw
                        .chunks_exact(2 * ch)
                        .map(|fr| {
                            fr.chunks_exact(2)
                                .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
                                .sum::<f32>()
                                / ch as f32
                        })
                        .collect(),
                    (1, 24) => raw
                        .chunks_exact(3 * ch)
                        .map(|fr| {
                            fr.chunks_exact(3)
                                .map(|b| {
                                    i32::from_le_bytes([0, b[0], b[1], b[2]]) as f32
                                        / 2147483648.0
                                })
                                .sum::<f32>()
                                / ch as f32
                        })
                        .collect(),
                    (3, 32) => raw
                        .chunks_exact(4 * ch)
                        .map(|fr| {
                            fr.chunks_exact(4)
                                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                                .sum::<f32>()
                                / ch as f32
                        })
                        .collect(),
                    (f, b) => {
                        return Err(format!(
                            "wav '{}': unsupported format {} / {} bits (use PCM16, PCM24, or float32)",
                            path, f, b
                        ))
                    }
                };
                samples = Some(mono);
            }
            _ => {}
        }
        pos = body + size + (size & 1); // chunks are word-aligned
    }
    let (_, _, rate, _) = fmt.ok_or_else(|| format!("wav '{}': no fmt chunk", path))?;
    let samples = samples.ok_or_else(|| format!("wav '{}': no data chunk", path))?;
    if samples.is_empty() {
        return Err(format!("wav '{}': empty", path));
    }
    Ok((samples, rate))
}

// ---------------------------------------------------------------------------
// The voice box: source + vocoder, wired to the bus
// ---------------------------------------------------------------------------

pub struct VoxBox {
    pub source: VoxSource,
    vocoder: Vocoder,
    talker: crate::talker::Talker,
    spectral: crate::spectral::Spectral,
    mode: crate::vocoder::VocoderMode,
    sample_rate: f32,
    // Optional recorded modulator: any voice, poured through the same circuit
    wav: Option<Vec<f32>>,
    wav_pos: usize,
    wav_active: bool,
    held: u32,
    // Smoothed output gains
    level: f32,
    level_t: f32,
    dry: f32,
    dry_t: f32,
}

impl VoxBox {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            source: VoxSource::new(sample_rate),
            vocoder: Vocoder::new(sample_rate),
            talker: crate::talker::Talker::new(sample_rate),
            spectral: crate::spectral::Spectral::new(sample_rate),
            mode: crate::vocoder::VocoderMode::TalkBox,
            sample_rate,
            wav: None,
            wav_pos: 0,
            wav_active: false,
            held: 0,
            level: 0.8,
            level_t: 0.8,
            dry: 0.0,
            dry_t: 0.0,
        }
    }

    pub fn set_level(&mut self, v: f32) {
        self.level_t = v.clamp(0.0, 1.0);
    }

    pub fn set_dry(&mut self, v: f32) {
        self.dry_t = v.clamp(0.0, 1.0);
    }

    pub fn set_mode(&mut self, mode: crate::vocoder::VocoderMode) {
        self.mode = mode;
        self.vocoder.set_mode(mode);
    }

    /// Talker circuit only: caricature (0) <-> legible (1).
    pub fn set_clarity(&mut self, v: f32) {
        self.talker.set_clarity(v);
    }

    /// Load a recorded modulator, resampled to the engine rate and
    /// peak-normalized. It starts from the top at the next phrase (a vox
    /// note-on with no other vox notes held).
    pub fn set_wav(&mut self, samples: &[f32], source_rate: u32) {
        let ratio = source_rate as f64 / self.sample_rate as f64;
        let out_len = (samples.len() as f64 / ratio) as usize;
        let mut out = Vec::with_capacity(out_len);
        for i in 0..out_len {
            let t = i as f64 * ratio;
            let i0 = t as usize;
            let frac = (t - i0 as f64) as f32;
            let a = samples[i0];
            let b = samples[(i0 + 1).min(samples.len() - 1)];
            out.push(a + (b - a) * frac);
        }
        let peak = out.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        if peak > 1e-6 {
            let g = 0.9 / peak;
            for s in &mut out {
                *s *= g;
            }
        }
        self.wav = Some(out);
        self.wav_active = false;
        self.wav_pos = 0;
    }

    pub fn note_on(&mut self, note: u8, velocity: f32) {
        // The recording starts at a phrase start and then FLOWS: the brief
        // all-keys-up gap between legato chords must not rewind it. It
        // rearms only once it has played out.
        if self.wav.is_some() && self.held == 0 && !self.wav_active {
            self.wav_pos = 0;
            self.wav_active = true;
        }
        self.held += 1;
        self.source.note_on(note, velocity);
    }

    pub fn note_off(&mut self, note: u8) {
        self.held = self.held.saturating_sub(1);
        self.source.note_off(note);
    }

    /// One sample: take the modulator (recorded voice if loaded, the
    /// formant voice otherwise), vocode the carrier with it, and mix in
    /// however much raw voice `vox_dry` asks for. Carrier and output are
    /// in program volts.
    #[inline]
    pub fn process(&mut self, carrier: f32) -> f32 {
        let m = if self.wav_active {
            let w = self.wav.as_ref().unwrap();
            let s = w[self.wav_pos];
            self.wav_pos += 1;
            if self.wav_pos >= w.len() {
                self.wav_active = false;
            }
            s
        } else if self.wav.is_some() {
            0.0
        } else {
            self.source.render()
        };
        // Two different machines behind one knob: the band vocoder, or
        // the LPC formant tracker (vox_mode 2) — one continuous filter,
        // the true talk-box circuit
        let vocoded = match self.mode {
            crate::vocoder::VocoderMode::Talker => self.talker.process(m, carrier),
            crate::vocoder::VocoderMode::Spectral => self.spectral.process(m, carrier),
            _ => self.vocoder.process(m, carrier),
        };
        self.level += (self.level_t - self.level) * 0.001;
        self.dry += (self.dry_t - self.dry) * 0.001;
        vocoded * self.level + m * self.dry * (0.9 * PROGRAM_V)
    }

    /// Band-envelope sum, for panel metering.
    pub fn activity(&self) -> f32 {
        self.vocoder.activity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyric_parsing() {
        let l = parse_lyric("HH-EH:180@0.7-L-OW").unwrap();
        assert_eq!(l.boundary, Boundary::None);
        assert_eq!(l.phones.len(), 4);
        assert_eq!(
            l.phones[0],
            LyricPhone { ph: Phoneme::HH, ms: None, amp: 1.0, stress: None }
        );
        assert_eq!(l.phones[1].ph, Phoneme::EH);
        assert_eq!(l.phones[1].ms, Some(180.0));
        assert!((l.phones[1].amp - 0.7).abs() < 1e-6);
        assert_eq!(l.phones[3].ph, Phoneme::OW);
        // lowercase is fine
        assert_eq!(parse_lyric("s-ih-ng").unwrap().phones[2].ph, Phoneme::NG);
        // stress digits ride the vowels; punctuation marks the phrase edge
        let l = parse_lyric("HH-OW1-M.").unwrap();
        assert_eq!(l.boundary, Boundary::Fall);
        assert_eq!(l.phones[1].stress, Some(1));
        assert_eq!(parse_lyric("Y-UW0?").unwrap().boundary, Boundary::Rise);
        assert_eq!(parse_lyric("AA2:90").unwrap().phones[0].stress, Some(2));
        assert!(parse_lyric("AA3").is_err(), "stress is 0-2");
        assert!(parse_lyric("S1-AA").is_err(), "stress goes on vowels");
        assert!(parse_lyric("QX").is_err());
        assert!(parse_lyric("").is_err());
        assert!(parse_lyric("AA:-5").is_err());
    }

    fn zcr(samples: &[f32]) -> f32 {
        let mut c = 0;
        for w in samples.windows(2) {
            if (w[0] >= 0.0) != (w[1] >= 0.0) {
                c += 1;
            }
        }
        c as f32 / samples.len() as f32
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// "S-AA": the fricative onset must be hissy (high zero-crossing
    /// rate), the vowel voiced (low ZCR, more energy), and the vowel must
    /// sustain as long as the note is held.
    #[test]
    fn syllables_speak_in_order() {
        let sr = 48000.0;
        let mut v = VoxSource::new(sr);
        v.set_syllable(parse_lyric("S-AA").unwrap());
        v.note_on(45, 0.9);
        let mut out = Vec::new();
        for _ in 0..(sr as usize) {
            out.push(v.render());
        }
        let n = sr as usize;
        let hiss = &out[n / 100..n / 12]; // ~10-83 ms: inside the S
        let vowel = &out[n / 2..n * 3 / 4]; // deep in the sustained AA
        assert!(zcr(hiss) > 2.0 * zcr(vowel), "S should hiss: {} vs {}", zcr(hiss), zcr(vowel));
        assert!(
            rms(vowel) > 1.5 * rms(hiss),
            "AA should be louder than S: vowel rms={}, hiss rms={}",
            rms(vowel),
            rms(hiss)
        );
        assert!(rms(vowel) > 0.03, "vowel must actually sound, rms={}", rms(vowel));
        assert!(out.iter().all(|s| s.is_finite() && s.abs() < 4.0));

        // Release: the voice must fall silent shortly after note-off
        v.note_off(45);
        let mut tail = Vec::new();
        for _ in 0..(sr as usize / 2) {
            tail.push(v.render());
        }
        let quiet = &tail[tail.len() - 4800..];
        assert!(rms(quiet) < 0.01, "voice should stop after release, rms={}", rms(quiet));
    }

    /// Vowel identity: AA and IY must differ where F2 lives.
    #[test]
    fn vowels_have_distinct_formants() {
        let sr = 48000.0;
        let energy_at = |lyric: &str, freq: f32| -> f32 {
            let mut v = VoxSource::new(sr);
            v.set_vibrato(0.0);
            v.set_syllable(parse_lyric(lyric).unwrap());
            v.note_on(45, 0.9); // A2 = 110 Hz
            let n = sr as usize / 2;
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                out.push(v.render());
            }
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, &s) in out[n / 2..].iter().enumerate() {
                let a = TAU * freq * i as f32 / sr;
                re += s * a.cos();
                im += s * a.sin();
            }
            (re * re + im * im).sqrt()
        };
        // Probe near IY's F2 (2290 Hz) vs AA's F2 (1090 Hz)
        let iy_hi = energy_at("IY", 2310.0);
        let aa_hi = energy_at("AA", 2310.0);
        let aa_lo = energy_at("AA", 1100.0);
        let iy_lo = energy_at("IY", 1100.0);
        assert!(iy_hi > 3.0 * aa_hi, "IY needs F2 energy at 2.3k: {iy_hi} vs {aa_hi}");
        assert!(aa_lo > 3.0 * iy_lo, "AA needs F2 energy at 1.1k: {aa_lo} vs {iy_lo}");
    }

    /// An explicit `:ms` on the vowel overrides sustain: the phoneme ends
    /// on its own clock even though the note stays down.
    #[test]
    fn explicit_duration_overrides_sustain() {
        let sr = 48000.0;
        let mut v = VoxSource::new(sr);
        v.set_syllable(parse_lyric("AA:120").unwrap());
        v.note_on(45, 0.9);
        let mut out = Vec::new();
        for _ in 0..(sr as usize) {
            out.push(v.render()); // note held the whole second
        }
        let during = &out[(0.03 * sr) as usize..(0.10 * sr) as usize];
        let after = &out[(0.6 * sr) as usize..];
        assert!(rms(during) > 0.03, "vowel speaks, rms={}", rms(during));
        assert!(
            rms(after) < 0.1 * rms(during),
            "vowel must end at its :ms even while held: {} vs {}",
            rms(after),
            rms(during)
        );
    }

    /// The coda waits for the note-off: "AA-T" keeps the T's burst in its
    /// pocket until the key lifts.
    #[test]
    fn coda_speaks_at_note_off() {
        let sr = 48000.0;
        let mut v = VoxSource::new(sr);
        v.set_syllable(parse_lyric("AA-T").unwrap());
        v.note_on(45, 0.9);
        let mut held = Vec::new();
        for _ in 0..(sr as usize / 2) {
            held.push(v.render());
        }
        v.note_off(45);
        let mut released = Vec::new();
        for _ in 0..(sr as usize / 4) {
            released.push(v.render());
        }
        // While held: pure vowel, low ZCR everywhere. After release the T
        // burst appears: a stretch with fricative-grade ZCR.
        let vowel_zcr = zcr(&held[held.len() / 2..]);
        let burst_zcr = released
            .windows(480)
            .step_by(240)
            .map(|w| zcr(w))
            .fold(0.0f32, f32::max);
        assert!(
            burst_zcr > 2.5 * vowel_zcr,
            "T burst should follow release: {burst_zcr} vs vowel {vowel_zcr}"
        );
    }

    /// Stress shapes loudness: a reduced vowel (AA0) must sing quieter
    /// than a stressed one (AA1).
    #[test]
    fn stress_scales_loudness() {
        let sr = 48000.0;
        let level_of = |lyric: &str| -> f32 {
            let mut v = VoxSource::new(sr);
            v.set_vibrato(0.0);
            v.set_syllable(parse_lyric(lyric).unwrap());
            v.note_on(45, 0.8);
            let n = (0.4 * sr) as usize;
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                out.push(v.render());
            }
            rms(&out[n / 2..])
        };
        let stressed = level_of("AA1");
        let reduced = level_of("AA0");
        assert!(
            stressed > 1.25 * reduced,
            "AA1 should outsing AA0: {stressed} vs {reduced}"
        );
    }

    /// The phrase has a shape: pitch accents bump each syllable onset,
    /// declination lowers later syllables, and a `.` boundary drives the
    /// pitch down through the coda after the last key lifts.
    #[test]
    fn prosody_declines_and_falls() {
        let sr = 48000.0;
        let mut v = VoxSource::new(sr);
        v.set_intonation(1.0);
        let sing = |v: &mut VoxSource, secs: f32| {
            for _ in 0..(secs * sr) as usize {
                v.render();
            }
        };
        // Three-syllable phrase; the last is marked with a final fall
        v.set_syllable(parse_lyric("AA").unwrap());
        v.note_on(45, 0.8);
        sing(&mut v, 0.05);
        let first = v.prosody_st();
        sing(&mut v, 0.25);
        v.note_off(45);
        sing(&mut v, 0.1);
        v.set_syllable(parse_lyric("AA").unwrap());
        v.note_on(45, 0.8);
        sing(&mut v, 0.3);
        v.note_off(45);
        sing(&mut v, 0.1);
        v.set_syllable(parse_lyric("HH-OW-M.").unwrap());
        v.note_on(45, 0.8);
        sing(&mut v, 0.05);
        let last_onset = v.prosody_st();
        sing(&mut v, 0.25);
        v.note_off(45);
        sing(&mut v, 0.35); // the coda speaks while the fall takes hold
        let after_fall = v.prosody_st();
        assert!(
            first > last_onset + 0.4,
            "declination: first syllable should sit higher ({first} vs {last_onset})"
        );
        assert!(
            after_fall < -2.0,
            "a `.` boundary must drive the pitch down through the coda, got {after_fall}"
        );
        // A fresh phrase after the boundary starts high again
        v.set_syllable(parse_lyric("AA").unwrap());
        v.note_on(45, 0.8);
        sing(&mut v, 0.05);
        assert!(v.prosody_st() > 0.0, "new phrase should reset declination");
    }

    /// VoxBox: speech articulates the carrier; silence doesn't.
    #[test]
    fn voxbox_vocodes_the_carrier() {
        let sr = 48000.0;
        let mut vb = VoxBox::new(sr);
        vb.set_level(1.0);
        let saw = |n: usize| (((n as f32 * 110.0 / sr) % 1.0) * 2.0 - 1.0) * 5.0;
        // No note: modulator silent, carrier must stay shut
        let mut quiet = 0.0f32;
        for n in 0..(sr as usize / 4) {
            quiet = quiet.max(vb.process(saw(n)).abs());
        }
        vb.source.set_syllable(parse_lyric("AA").unwrap());
        vb.note_on(45, 1.0);
        let mut loud = 0.0f32;
        for n in 0..(sr as usize / 2) {
            loud = loud.max(vb.process(saw(n)).abs());
        }
        assert!(loud > 10.0 * quiet.max(0.02), "speech must open the vocoder: {loud} vs {quiet}");
    }

    /// WAV round trip: write a file with the engine's own writer format
    /// (float32 stereo), read it back mono, use it as the modulator.
    #[test]
    fn wav_modulator_loads_and_speaks() {
        let dir = std::env::temp_dir().join("patina_vox_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mod.wav").to_string_lossy().into_owned();
        // 0.5 s of 200 Hz square at 24 kHz, PCM16 mono
        let rate = 24000u32;
        let n = rate as usize / 2;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + n as u32 * 2).to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(n as u32 * 2).to_le_bytes());
        for i in 0..n {
            let v = if (i as f32 * 200.0 / rate as f32) % 1.0 < 0.5 { 12000i16 } else { -12000 };
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(&path, &bytes).unwrap();

        let (samples, r) = load_wav_mono(&path).unwrap();
        assert_eq!(r, rate);
        assert!(samples.len() > 11000);

        let sr = 48000.0;
        let mut vb = VoxBox::new(sr);
        vb.set_wav(&samples, r);
        vb.note_on(45, 1.0);
        let saw = |k: usize| (((k as f32 * 110.0 / sr) % 1.0) * 2.0 - 1.0) * 5.0;
        let mut peak = 0.0f32;
        for k in 0..(sr as usize / 4) {
            peak = peak.max(vb.process(saw(k)).abs());
        }
        assert!(peak > 0.3, "recorded modulator should articulate the carrier, peak={peak}");
        // After the wav runs out, the box goes quiet (no fallback buzz)
        for _ in 0..(sr as usize / 2) {
            vb.process(saw(0));
        }
        let mut tail = 0.0f32;
        for k in 0..4800 {
            tail = tail.max(vb.process(saw(k)).abs());
        }
        assert!(tail < 0.05, "spent wav must leave the carrier shut, tail={tail}");
    }
}
