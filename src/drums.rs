// The rhythm section: a TR-909 voice board, circuit-modeled, living INSIDE
// the instrument — same volt conventions, same output bus, same effects,
// same power supply as the keyboard voices.
//
// What the 909's analog voices actually are (service notes, sheets 2-4):
//
//   BASS DRUM   A bridged-T resonator (the "shell") kicked by the trigger
//               pulse, with a transistor pulling the bridge to sweep the
//               pitch down over the first tens of milliseconds, a separate
//               ATTACK click path, and a diode/transistor waveshaper that
//               clips the hottest early cycles into the "knock".
//   SNARE       TWO bridged-T oscillators (~185 Hz and ~330 Hz — a deliberate
//               non-harmonic pair) plus a noise path with its own filter and
//               envelope; TUNE moves both shells, SNAPPY is the noise VCA.
//   RIM SHOT    A stack of bridged-T resonators rung hard for milliseconds
//               and clipped — all attack, no body.
//   HAND CLAP   Noise through a ~1 kHz band-pass, gated by a multi-pulse
//               retrigger envelope (the "flam" of many palms) into a longer
//               reverb-tail envelope.
//   HI-HATS     On the hardware these are 6-bit ROM samples — the one part
//               of the 909 everyone agrees sounds "mid". Per the brief we
//               DON'T sample: the hats here are a six-oscillator metal bank
//               (the 808/606 lineage) pushed through the 909's own analog
//               post-processing — steep high-pass, VCA, and the shared
//               choke between closed and open — with a METAL knob to blend
//               the clangy bank against pure filtered noise.
//   (Toms and the sequencer are deliberately omitted; songs and MIDI are
//   the sequencer.)
//
// Modern extensions, added AFTER the stock circuit at each point where a
// techno engineer actually mods or gains-stages the machine:
//   - per-voice DRIVE on the kick (clean sine-wave sub through stock knock
//     into full transistor overdrive — the 909's narrow middle ground
//     widened in both directions)
//   - SWEEP depth control on the kick's pitch envelope (stock at center)
//   - DECAY ranges extended past the panel stops (rumble-length kicks,
//     gated-short snares)
//   - a bus DRIVE (the "mixer channel slammed into the red" that half of
//     techno is built on), ADAA-antialiased
//
// Triggering: velocity IS the accent bus. On the hardware the accent line
// adds voltage to the trigger pulse, which both raises the VCA peak and
// excites the resonators harder (hotter first cycles -> more waveshaper
// bite). Both effects are modeled.
//
// Epistemic status: topologies and signal flows are SCHEMATIC (TR-909
// service notes); the resonator center frequencies, envelope time
// constants, and sweep depths are the documented/measured values for the
// circuit blocks, trimmed by ear inside their service tolerances; knob
// range extensions and the hat metal bank are labeled CHOICE.

use crate::adaa::AdaaTanh;
use crate::hpf::HighPassLadder;
use crate::noise::NoiseSource;

/// The reserved song/MIDI channel that routes notes to the drum board.
pub const DRUM_CHANNEL: u16 = u16::MAX;

/// GM drum map (the relevant rows), so any drum-mode controller or DAW
/// track speaks to the board without configuration.
const NOTE_KICK: u8 = 36; // C1  (35 accepted too)
const NOTE_RIM: u8 = 37; // C#1
const NOTE_SNARE: u8 = 38; // D1  (40 accepted too)
const NOTE_CLAP: u8 = 39; // D#1
const NOTE_CH: u8 = 42; // F#1 (44 pedal hat accepted too)
const NOTE_OH: u8 = 46; // A#1

/// Drum names for the song DSL (`track beat kit=909`): BD SD RS CP CH OH.
pub fn note_from_name(s: &str) -> Option<u8> {
    Some(match s.to_ascii_uppercase().as_str() {
        "BD" | "KICK" => NOTE_KICK,
        "SD" | "SNARE" => NOTE_SNARE,
        "RS" | "RIM" => NOTE_RIM,
        "CP" | "CLAP" => NOTE_CLAP,
        "CH" | "HH" => NOTE_CH,
        "OH" => NOTE_OH,
        _ => return None,
    })
}

#[inline]
fn xorshift(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

/// White noise in -1..1 (full bandwidth, unlike the ARP-style NoiseSource
/// whose amplifier rolls off ~6 kHz — the hats need the top octaves).
#[inline]
fn white(state: &mut u32) -> f32 {
    (xorshift(state) >> 8) as f32 / (1u32 << 23) as f32 - 1.0
}

/// Per-sample decay coefficient for an RC discharge with time constant
/// `tau` seconds. T60 = 6.91 * tau.
#[inline]
fn rc_coef(tau: f32, sample_rate: f32) -> f32 {
    (-1.0 / (tau.max(1e-4) * sample_rate)).exp()
}

// ---------------------------------------------------------------------------
// Bass drum
// ---------------------------------------------------------------------------

/// The 909 kick: a swept, kicked resonator into a waveshaper into a VCA.
///
/// The shell is modeled as the ringing mode of the bridged-T (a damped
/// sinusoid whose instantaneous frequency follows the pitch-envelope
/// transistor). Two RC discharges shape the sweep, as on the board: a fast
/// one (the "impact", ~4 ms) and a slower settling one (~45 ms). The
/// waveshaper AFTER the resonator is what turns the hottest early cycles
/// into the knock — by the tail the level has fallen out of the diode
/// knee and the sub ring is nearly pure. DRIVE moves the operating point:
/// below stock the shaper barely engages (clean deep sub the hardware
/// can't quite do), above stock every cycle folds (distorted techno kick).
struct Kick {
    sample_rate: f32,
    phase: f32,
    amp: f32,     // shell envelope, 0..1
    amp_coef: f32,
    sweep_fast: f32, // pitch-envelope caps, 0..1 each
    sweep_slow: f32,
    click: f32,   // click resonator envelope
    click_phase: f32,
    accent: f32,  // trigger voltage this hit, 0..1
    t_since: f32, // seconds since trigger (for the short anti-thump ramp)
    shaper: AdaaTanh,
    // Panel
    level: f32,
    tune: f32,
    attack: f32,
    decay: f32,
    sweep: f32,
    drive: f32,
}

impl Kick {
    fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            amp: 0.0,
            amp_coef: 0.0,
            sweep_fast: 0.0,
            sweep_slow: 0.0,
            click: 0.0,
            click_phase: 0.0,
            accent: 0.0,
            t_since: 1.0,
            shaper: AdaaTanh::new(),
            level: 0.8,
            tune: 0.35,
            attack: 0.5,
            decay: 0.45,
            sweep: 0.5,
            drive: 0.25,
        }
    }

    fn trigger(&mut self, vel: f32) {
        self.accent = vel.clamp(0.0, 1.0);
        // The trigger pulse dumps into the bridged-T: the ring restarts
        // from a defined phase every hit — the 909's machine-tight attack
        // (unlike an 808 rung mid-swing)
        self.phase = 0.0;
        self.amp = 0.55 + 0.45 * self.accent;
        // Decay: stock panel spans ~0.1 s to ~0.5 s of T60; the top half
        // of our extended knob stretches to 2.5 s (CHOICE: rumble kicks)
        let t60 = 0.10 * 25.0f32.powf(self.decay);
        self.amp_coef = rc_coef(t60 / 6.91, self.sample_rate);
        self.sweep_fast = 1.0;
        self.sweep_slow = 1.0;
        // Click path: the attack knob's transient burst, accent-hot
        self.click = self.attack * (0.4 + 0.6 * self.accent);
        self.click_phase = 0.0;
        self.t_since = 0.0;
    }

    #[inline]
    fn render(&mut self) -> f32 {
        if self.amp < 1e-5 && self.click < 1e-5 {
            return 0.0;
        }
        self.t_since += 1.0 / self.sample_rate;

        // Shell frequency: TUNE spans the service-manual bracket (~42 to
        // ~88 Hz fundamental), and the two pitch-envelope discharges ride
        // on top. SWEEP scales their depth; 0.5 is the stock board.
        let f0 = 42.0 * (88.0f32 / 42.0).powf(self.tune);
        let depth = self.sweep * 2.0; // 0.5 -> 1.0 = stock depths
        self.sweep_fast *= rc_coef(0.004, self.sample_rate);
        self.sweep_slow *= rc_coef(0.045, self.sample_rate);
        let f = f0 * (1.0 + depth * (2.2 * self.sweep_fast + 0.55 * self.sweep_slow));

        self.phase += f / self.sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        self.amp *= self.amp_coef;
        // A ~0.4 ms rise so the VCA doesn't step (the hardware trigger
        // shaping does the same); the audible snap comes from the click
        let ramp = (self.t_since / 0.0004).min(1.0);
        let shell = (self.phase * std::f32::consts::TAU).sin() * self.amp * ramp;

        // Click circuit: a damped ~1.7 kHz ping (the trigger pulse through
        // its little band-pass), 1.5 ms time constant
        self.click *= rc_coef(0.0015, self.sample_rate);
        self.click_phase += 1700.0 / self.sample_rate;
        let click = (self.click_phase * std::f32::consts::TAU).cos() * self.click;

        // Waveshaper: DRIVE sets how far into the diode/transistor curve
        // the shell swings. Accent-hot hits push deeper (SCHEMATIC: the
        // accent voltage raises the level INTO the shaper, not after it).
        // Slight positive bias -> even harmonics, like the single-ended
        // stage; the bias is removed after so no DC reaches the VCA.
        let gain = 0.55 * 16.0f32.powf(self.drive) * (0.8 + 0.4 * self.accent);
        let x = (shell + click * 0.7) * gain + 0.06 * gain.min(3.0);
        let shaped = self.shaper.process(x) - (0.06 * gain.min(3.0)).tanh();

        // Back to volts; makeup keeps DRIVE about character, not loudness
        let v = shaped * 8.0 / gain.max(1.0).sqrt() / gain.min(1.0).max(0.25);
        v * self.level
    }
}

// ---------------------------------------------------------------------------
// Snare drum
// ---------------------------------------------------------------------------

/// Two detuned shells plus a filtered, enveloped noise path.
struct Snare {
    sample_rate: f32,
    phase1: f32,
    phase2: f32,
    amp: f32,
    amp_coef: f32,
    bend: f32, // onset pitch bend cap
    noise_env: f32,
    noise_coef: f32,
    noise_lp: f32,
    noise_hp: f32,
    accent: f32,
    shaper: AdaaTanh,
    level: f32,
    tune: f32,
    tone: f32,
    snappy: f32,
    decay: f32,
}

impl Snare {
    fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            phase1: 0.0,
            phase2: 0.31, // the two bridged-Ts don't start in phase
            amp: 0.0,
            amp_coef: 0.0,
            bend: 0.0,
            noise_env: 0.0,
            noise_coef: 0.0,
            noise_lp: 0.0,
            noise_hp: 0.0,
            accent: 0.0,
            shaper: AdaaTanh::new(),
            level: 0.75,
            tune: 0.4,
            tone: 0.5,
            snappy: 0.6,
            decay: 0.5,
        }
    }

    fn trigger(&mut self, vel: f32) {
        self.accent = vel.clamp(0.0, 1.0);
        self.phase1 = 0.0;
        self.phase2 = 0.31;
        self.amp = 0.55 + 0.45 * self.accent;
        // Shell T60 ~60..180 ms across the extended decay knob
        let t60 = 0.06 + 0.12 * self.decay;
        self.amp_coef = rc_coef(t60 / 6.91, self.sample_rate);
        self.bend = 1.0;
        self.noise_env = (0.5 + 0.5 * self.accent) * self.snappy;
        // Noise T60 ~90..420 ms
        let noise_t60 = 0.09 + 0.33 * self.decay;
        self.noise_coef = rc_coef(noise_t60 / 6.91, self.sample_rate);
    }

    #[inline]
    fn render(&mut self, noise: f32) -> f32 {
        if self.amp < 1e-5 && self.noise_env < 1e-5 {
            return 0.0;
        }
        // Shells: ~185 and ~330 Hz at panel center (service notes), the
        // pair ratio fixed by the two bridged-T networks; TUNE moves both.
        // A fast onset bend (~4 ms) gives the 909 snare its "doip".
        self.bend *= rc_coef(0.004, self.sample_rate);
        let f1 = 160.0 * (1.0 + 0.5 * self.tune) * (1.0 + 0.9 * self.bend);
        let f2 = f1 * 1.78;
        self.phase1 += f1 / self.sample_rate;
        self.phase1 -= self.phase1.floor();
        self.phase2 += f2 / self.sample_rate;
        self.phase2 -= self.phase2.floor();
        self.amp *= self.amp_coef;
        let shell = ((self.phase1 * std::f32::consts::TAU).sin()
            + 0.6 * (self.phase2 * std::f32::consts::TAU).sin())
            * self.amp;

        // Noise path: fixed ~400 Hz high-pass, TONE sets the low-pass
        // (dark 1.8 kHz .. open 10 kHz, log taper like the pot)
        self.noise_env *= self.noise_coef;
        let lp_fc = 1800.0 * (10000.0f32 / 1800.0).powf(self.tone);
        let lp_k = 1.0 - (-std::f32::consts::TAU * lp_fc / self.sample_rate).exp();
        self.noise_lp += lp_k * (noise - self.noise_lp);
        let hp_k = 1.0 - (-std::f32::consts::TAU * 400.0 / self.sample_rate).exp();
        self.noise_hp += hp_k * (self.noise_lp - self.noise_hp);
        let snap = (self.noise_lp - self.noise_hp) * self.noise_env;

        // TONE also sets how hard the shells hit the shaper (the knob
        // feeds both dividers on the board): dark = round, up = crack
        let gain = 1.0 + 2.2 * self.tone + 0.5 * self.accent;
        let shaped = self.shaper.process((shell + snap * 1.6) * gain);
        shaped * 6.0 / gain.sqrt() * self.level
    }
}

// ---------------------------------------------------------------------------
// Rim shot
// ---------------------------------------------------------------------------

/// Three bridged-T resonators rung for milliseconds and clipped hard.
struct Rim {
    sample_rate: f32,
    phases: [f32; 3],
    amps: [f32; 3],
    coefs: [f32; 3],
    accent: f32,
    shaper: AdaaTanh,
    level: f32,
    tune: f32,
}

/// Rim modes: frequency Hz, relative level, T60 seconds (SCHEMATIC
/// bracket; the stack is all transient).
const RIM_MODES: [(f32, f32, f32); 3] =
    [(220.0, 1.0, 0.040), (500.0, 0.85, 0.032), (1020.0, 0.5, 0.022)];

impl Rim {
    fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            phases: [0.0; 3],
            amps: [0.0; 3],
            coefs: [0.0; 3],
            accent: 0.0,
            shaper: AdaaTanh::new(),
            level: 0.7,
            tune: 0.5,
        }
    }

    fn trigger(&mut self, vel: f32) {
        self.accent = vel.clamp(0.0, 1.0);
        for (i, (_, lvl, t60)) in RIM_MODES.iter().enumerate() {
            self.phases[i] = 0.0;
            self.amps[i] = lvl * (0.6 + 0.4 * self.accent);
            self.coefs[i] = rc_coef(t60 / 6.91, self.sample_rate);
        }
    }

    #[inline]
    fn render(&mut self) -> f32 {
        if self.amps[0] < 1e-5 {
            return 0.0;
        }
        let tune_mult = 0.8 + 0.4 * self.tune;
        let mut sum = 0.0;
        for (i, (f, _, _)) in RIM_MODES.iter().enumerate() {
            self.phases[i] += f * tune_mult / self.sample_rate;
            self.phases[i] -= self.phases[i].floor();
            self.amps[i] *= self.coefs[i];
            sum += (self.phases[i] * std::f32::consts::TAU).sin() * self.amps[i];
        }
        // Hard into the clipper: the rim IS its distortion
        let shaped = self.shaper.process(sum * (2.5 + self.accent));
        shaped * 5.0 * self.level
    }
}

// ---------------------------------------------------------------------------
// Hand clap
// ---------------------------------------------------------------------------

/// Band-passed noise through the flam envelope: a burst generator
/// retriggers the fast discharge every ~12 ms (three palms), then hands
/// off to the longer "room" tail.
struct Clap {
    sample_rate: f32,
    t: f32, // seconds since trigger
    burst: f32,
    tail: f32,
    tail_coef: f32,
    bursts_left: u32,
    next_burst: f32,
    // Two-pole resonant band-pass state (Chamberlin SVF at ~1.1 kHz)
    svf_band: f32,
    svf_low: f32,
    accent: f32,
    level: f32,
    decay: f32,
}

impl Clap {
    fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            t: 1.0,
            burst: 0.0,
            tail: 0.0,
            tail_coef: 0.0,
            bursts_left: 0,
            next_burst: 0.0,
            svf_band: 0.0,
            svf_low: 0.0,
            accent: 0.0,
            level: 0.75,
            decay: 0.5,
        }
    }

    fn trigger(&mut self, vel: f32) {
        self.accent = vel.clamp(0.0, 1.0);
        self.t = 0.0;
        self.burst = 0.7 + 0.3 * self.accent;
        self.bursts_left = 2; // two RETRIGGERS after the first palm
        self.next_burst = 0.012;
        self.tail = 0.0;
        // Tail T60 ~90..500 ms across the knob (stock sits mid)
        let t60 = 0.09 + 0.41 * self.decay;
        self.tail_coef = rc_coef(t60 / 6.91, self.sample_rate);
    }

    #[inline]
    fn render(&mut self, noise: f32) -> f32 {
        if self.burst < 1e-5 && self.tail < 1e-5 && self.bursts_left == 0 {
            return 0.0;
        }
        self.t += 1.0 / self.sample_rate;

        // The retrigger sawtooth: every 12 ms the fast discharge restarts
        if self.bursts_left > 0 && self.t >= self.next_burst {
            self.burst = 0.7 + 0.3 * self.accent;
            self.bursts_left -= 1;
            self.next_burst += 0.012;
            if self.bursts_left == 0 {
                // Hand-off: the room tail begins where the flam ends
                self.tail = 0.55 * (0.7 + 0.3 * self.accent);
            }
        }
        self.burst *= rc_coef(0.007, self.sample_rate);
        self.tail *= self.tail_coef;
        let env = self.burst + self.tail;

        // Band-pass ~1.1 kHz, Q ~ 2 (Chamberlin SVF, stable at this fc)
        let f = 2.0 * (std::f32::consts::PI * 1100.0 / self.sample_rate).sin();
        let q = 0.5; // 1/Q
        self.svf_low += f * self.svf_band;
        let high = noise - self.svf_low - q * self.svf_band;
        self.svf_band += f * high;

        self.svf_band * env * 7.0 * self.level
    }
}

// ---------------------------------------------------------------------------
// Hi-hats (the creative section: metal bank, 909 post-processing)
// ---------------------------------------------------------------------------

/// Six-oscillator metal bank -> steep high-pass -> two VCAs (closed/open)
/// with the hardware's choke: a closed hit slams the open VCA shut.
///
/// The bank frequencies are inharmonic on purpose (no two share a small
/// integer ratio), so their sum beats into the dense clangy spectrum that
/// reads as "cymbal" once everything below ~5 kHz is thrown away. METAL
/// blends the bank against plain white noise: 1.0 is all bank (606-like
/// ping), 0.0 is all noise (softer, tape-ish), the default sits mostly
/// metal. All CHOICE by design — this replaces the 909's ROMs.
struct Hats {
    sample_rate: f32,
    phases: [f32; 6],
    rng: u32,
    hpf: HighPassLadder,
    /// Post one-pole high-pass (~2.5 kHz): the board's coupling caps into
    /// the VCA add another corner on top of the swept ladder, and the
    /// bank's fundamentals must be gone-gone, not just -60 dB.
    post_hp: f32,
    lp: f32, // gentle top rolloff so the bank doesn't saw at the ear
    ch_env: f32,
    ch_coef: f32,
    oh_env: f32,
    oh_coef: f32,
    oh_fast: f32, // the open hat's initial transient stage
    accent_ch: f32,
    accent_oh: f32,
    level: f32,
    tune: f32,
    metal: f32,
    ch_decay: f32,
    oh_decay: f32,
}

/// The bank (Hz at TUNE center). Chosen inharmonic, dense above 300 Hz;
/// the HPF keeps only their beating upper structure.
const HAT_BANK: [f32; 6] = [325.7, 447.8, 615.6, 812.3, 1214.9, 1618.2];

impl Hats {
    fn new(sample_rate: f32) -> Self {
        let mut hpf = HighPassLadder::new(sample_rate);
        hpf.set_cutoff(5200.0);
        Self {
            sample_rate,
            phases: [0.13, 0.41, 0.71, 0.02, 0.55, 0.87],
            rng: 0x9E37_79B1,
            hpf,
            post_hp: 0.0,
            lp: 0.0,
            ch_env: 0.0,
            ch_coef: 0.0,
            oh_env: 0.0,
            oh_coef: 0.0,
            oh_fast: 0.0,
            accent_ch: 0.0,
            accent_oh: 0.0,
            level: 0.7,
            tune: 0.5,
            metal: 0.65,
            ch_decay: 0.35,
            oh_decay: 0.5,
        }
    }

    fn trigger_closed(&mut self, vel: f32) {
        self.accent_ch = vel.clamp(0.0, 1.0);
        self.ch_env = 0.6 + 0.4 * self.accent_ch;
        // CH T60 ~25..140 ms
        let t60 = 0.025 + 0.115 * self.ch_decay;
        self.ch_coef = rc_coef(t60 / 6.91, self.sample_rate);
        // THE CHOKE: the closed stick chokes the open hat's VCA (shared
        // envelope hardware on the board) — the disco "tss-t"
        if self.oh_env > 1e-4 {
            self.oh_coef = rc_coef(0.008 / 6.91, self.sample_rate);
        }
    }

    fn trigger_open(&mut self, vel: f32) {
        self.accent_oh = vel.clamp(0.0, 1.0);
        self.oh_env = 0.55 + 0.45 * self.accent_oh;
        self.oh_fast = 0.5;
        // OH T60 ~0.15..1.0 s
        let t60 = 0.15 + 0.85 * self.oh_decay;
        self.oh_coef = rc_coef(t60 / 6.91, self.sample_rate);
    }

    #[inline]
    fn render(&mut self) -> f32 {
        if self.ch_env < 1e-5 && self.oh_env < 1e-5 {
            return 0.0;
        }
        // The bank: six square waves. PolyBLEP is unnecessary here — every
        // partial that could alias audibly is >20 dB under the HPF'd mass
        // of edges, and the hardware lineage (606/808) is itself a pile of
        // untamed squares. A one-pole at ~14 kHz rounds the very top.
        let tune_mult = 0.75 + 0.6 * self.tune;
        let mut bank = 0.0;
        for (i, f) in HAT_BANK.iter().enumerate() {
            self.phases[i] += f * tune_mult / self.sample_rate;
            self.phases[i] -= self.phases[i].floor();
            bank += if self.phases[i] < 0.5 { 1.0 } else { -1.0 };
        }
        bank /= 6.0;
        let noise = white(&mut self.rng);
        let source = bank * self.metal + noise * (1.0 - self.metal);

        // 909 post-processing: steep high-pass (TUNE rides the corner),
        // then the two VCAs off the shared source
        self.hpf.set_cutoff(5200.0 * (0.7 + 0.6 * self.tune));
        let bright = self.hpf.process(source);
        let hp_k = 1.0 - (-std::f32::consts::TAU * 2500.0 / self.sample_rate).exp();
        self.post_hp += hp_k * (bright - self.post_hp);
        let bright = bright - self.post_hp;
        let lp_k = 1.0 - (-std::f32::consts::TAU * 14000.0 / self.sample_rate).exp();
        self.lp += lp_k * (bright - self.lp);

        self.ch_env *= self.ch_coef;
        self.oh_env *= self.oh_coef;
        // The open hat's two-stage decay: a fast sizzle settling into the
        // long ring (audible on every 909 record's off-beat)
        self.oh_fast *= rc_coef(0.030, self.sample_rate);
        let env = self.ch_env + self.oh_env * (1.0 + self.oh_fast);

        self.lp * env * 5.5 * self.level
    }
}

// ---------------------------------------------------------------------------
// The board
// ---------------------------------------------------------------------------

pub struct DrumMachine {
    kick: Kick,
    snare: Snare,
    rim: Rim,
    clap: Clap,
    hats: Hats,
    /// One noise transistor shared by snare and clap, exactly like the
    /// board (the ARP-style source reused — "an amplified, reversed
    /// junction of a selected transistor").
    noise: NoiseSource,
    /// Bus drive: the modern stage. 0 = wire. Smoothed so automation
    /// can't zipper the gain.
    drive: f32,
    drive_target: f32,
    bus_shaper_l: AdaaTanh,
    bus_shaper_r: AdaaTanh,
}

impl DrumMachine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            kick: Kick::new(sample_rate),
            snare: Snare::new(sample_rate),
            rim: Rim::new(sample_rate),
            clap: Clap::new(sample_rate),
            hats: Hats::new(sample_rate),
            noise: NoiseSource::new(),
            drive: 0.0,
            drive_target: 0.0,
            bus_shaper_l: AdaaTanh::new(),
            bus_shaper_r: AdaaTanh::new(),
        }
    }

    /// Trigger by (GM) note number; velocity is the accent voltage.
    pub fn trigger_note(&mut self, note: u8, velocity: f32) {
        match note {
            35 | 36 => self.kick.trigger(velocity),
            38 | 40 => self.snare.trigger(velocity),
            37 => self.rim.trigger(velocity),
            39 => self.clap.trigger(velocity),
            42 | 44 => self.hats.trigger_closed(velocity),
            46 => self.hats.trigger_open(velocity),
            _ => {}
        }
    }

    /// True while any voice is still audibly ringing.
    pub fn is_active(&self) -> bool {
        self.kick.amp > 1e-4
            || self.kick.click > 1e-4
            || self.snare.amp > 1e-4
            || self.snare.noise_env > 1e-4
            || self.rim.amps[0] > 1e-4
            || self.clap.burst > 1e-4
            || self.clap.tail > 1e-4
            || self.hats.ch_env > 1e-4
            || self.hats.oh_env > 1e-4
    }

    /// One sample of the whole board, in volts, placed on a narrow stereo
    /// image (kick and snare dead center like every techno mix; the small
    /// voices just off-axis, as if the board's individual outs were panned
    /// at the desk).
    pub fn render_next(&mut self) -> (f32, f32) {
        let noise = self.noise.next();

        let bd = self.kick.render();
        let sd = self.snare.render(noise);
        let rs = self.rim.render();
        let cp = self.clap.render(noise);
        let hh = self.hats.render();

        // Equal-power-ish constant pans (CHOICE, desk layout)
        let mut l = bd + sd + rs * 0.62 + cp * 0.44 + hh * 0.46;
        let mut r = bd + sd + rs * 0.38 + cp * 0.56 + hh * 0.54;

        // Bus drive: the mixer channel into the red. Unity when off.
        self.drive += (self.drive_target - self.drive) * 0.0008;
        if self.drive > 1e-3 {
            let g = 1.0 + 7.0 * self.drive;
            let pv = crate::oscillator::PROGRAM_V;
            let wet_l = pv * self.bus_shaper_l.process(l * g / pv) / g.sqrt();
            let wet_r = pv * self.bus_shaper_r.process(r * g / pv) / g.sqrt();
            let mix = (self.drive * 1.5).min(1.0);
            l = l * (1.0 - mix) + wet_l * mix;
            r = r * (1.0 - mix) + wet_r * mix;
        }
        (l, r)
    }

    // --- Panel ---------------------------------------------------------

    pub fn set_bd_level(&mut self, v: f32) {
        self.kick.level = v.clamp(0.0, 1.0);
    }
    pub fn set_bd_tune(&mut self, v: f32) {
        self.kick.tune = v.clamp(0.0, 1.0);
    }
    pub fn set_bd_attack(&mut self, v: f32) {
        self.kick.attack = v.clamp(0.0, 1.0);
    }
    pub fn set_bd_decay(&mut self, v: f32) {
        self.kick.decay = v.clamp(0.0, 1.0);
    }
    pub fn set_bd_sweep(&mut self, v: f32) {
        self.kick.sweep = v.clamp(0.0, 1.0);
    }
    pub fn set_bd_drive(&mut self, v: f32) {
        self.kick.drive = v.clamp(0.0, 1.0);
    }
    pub fn set_sd_level(&mut self, v: f32) {
        self.snare.level = v.clamp(0.0, 1.0);
    }
    pub fn set_sd_tune(&mut self, v: f32) {
        self.snare.tune = v.clamp(0.0, 1.0);
    }
    pub fn set_sd_tone(&mut self, v: f32) {
        self.snare.tone = v.clamp(0.0, 1.0);
    }
    pub fn set_sd_snappy(&mut self, v: f32) {
        self.snare.snappy = v.clamp(0.0, 1.0);
    }
    pub fn set_sd_decay(&mut self, v: f32) {
        self.snare.decay = v.clamp(0.0, 1.0);
    }
    pub fn set_rs_level(&mut self, v: f32) {
        self.rim.level = v.clamp(0.0, 1.0);
    }
    pub fn set_rs_tune(&mut self, v: f32) {
        self.rim.tune = v.clamp(0.0, 1.0);
    }
    pub fn set_cp_level(&mut self, v: f32) {
        self.clap.level = v.clamp(0.0, 1.0);
    }
    pub fn set_cp_decay(&mut self, v: f32) {
        self.clap.decay = v.clamp(0.0, 1.0);
    }
    pub fn set_hh_level(&mut self, v: f32) {
        self.hats.level = v.clamp(0.0, 1.0);
    }
    pub fn set_hh_tune(&mut self, v: f32) {
        self.hats.tune = v.clamp(0.0, 1.0);
    }
    pub fn set_hh_metal(&mut self, v: f32) {
        self.hats.metal = v.clamp(0.0, 1.0);
    }
    pub fn set_ch_decay(&mut self, v: f32) {
        self.hats.ch_decay = v.clamp(0.0, 1.0);
    }
    pub fn set_oh_decay(&mut self, v: f32) {
        self.hats.oh_decay = v.clamp(0.0, 1.0);
    }
    pub fn set_drive(&mut self, v: f32) {
        self.drive_target = v.clamp(0.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    fn render(dm: &mut DrumMachine, n: usize) -> Vec<f32> {
        (0..n).map(|_| dm.render_next().0).collect()
    }

    fn goertzel(samples: &[f32], freq: f32) -> f32 {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, &s) in samples.iter().enumerate() {
            let a = std::f32::consts::TAU * freq * i as f32 / SR;
            re += s * a.cos();
            im += s * a.sin();
        }
        (re * re + im * im).sqrt()
    }

    /// Rising zero crossings in a window — a crude frequency probe.
    fn crossings(samples: &[f32]) -> usize {
        let mut count = 0;
        for w in samples.windows(2) {
            if w[0] <= 0.0 && w[1] > 0.0 {
                count += 1;
            }
        }
        count
    }

    #[test]
    fn kick_pitch_sweeps_down_to_the_tuned_fundamental() {
        let mut dm = DrumMachine::new(SR);
        dm.set_bd_tune(0.35);
        dm.set_bd_drive(0.0);
        dm.trigger_note(36, 1.0);
        let out = render(&mut dm, (0.6 * SR) as usize);
        // Early window (first 15 ms) must run much faster than the tail
        let early = crossings(&out[..(0.015 * SR) as usize]);
        let tail = &out[(0.3 * SR) as usize..(0.5 * SR) as usize];
        let tail_hz = crossings(tail) as f32 / 0.2;
        let f0 = 42.0 * (88.0f32 / 42.0).powf(0.35);
        assert!(
            (tail_hz - f0).abs() < f0 * 0.15,
            "tail should ring near the tuned fundamental {f0:.0} Hz, got {tail_hz:.0}"
        );
        // 15 ms at f0 would give ~1 crossing; the sweep packs in several
        assert!(early >= 2, "onset should be swept sharply up: {early} crossings");
    }

    #[test]
    fn kick_decay_knob_spans_punch_to_rumble() {
        let tail_energy = |decay: f32| -> f32 {
            let mut dm = DrumMachine::new(SR);
            dm.set_bd_decay(decay);
            dm.trigger_note(36, 1.0);
            let out = render(&mut dm, SR as usize);
            out[(0.5 * SR) as usize..].iter().map(|s| s * s).sum()
        };
        let short = tail_energy(0.0);
        let long = tail_energy(1.0);
        assert!(
            long > short * 100.0,
            "decay range should be dramatic: short={short:.4}, long={long:.4}"
        );
    }

    #[test]
    fn kick_drive_moves_from_clean_sub_to_grit() {
        let h3_ratio = |drive: f32| -> f32 {
            let mut dm = DrumMachine::new(SR);
            dm.set_bd_drive(drive);
            dm.set_bd_sweep(0.0); // hold pitch still so bins are clean
            dm.set_bd_attack(0.0);
            dm.set_bd_decay(0.8);
            dm.trigger_note(36, 1.0);
            let out = render(&mut dm, SR as usize);
            let f0 = 42.0 * (88.0f32 / 42.0).powf(0.35);
            let win = &out[(0.1 * SR) as usize..(0.6 * SR) as usize];
            goertzel(win, 3.0 * f0) / goertzel(win, f0).max(1e-9)
        };
        let clean = h3_ratio(0.0);
        let stock = h3_ratio(0.25);
        let hot = h3_ratio(1.0);
        assert!(
            clean < stock && stock < hot,
            "drive should widen clean->grit monotonically: {clean:.4} / {stock:.4} / {hot:.4}"
        );
        assert!(hot > 10.0 * clean.max(1e-6), "full drive should be properly dirty");
    }

    #[test]
    fn snare_carries_both_shell_modes_and_snappy_noise() {
        let mut dm = DrumMachine::new(SR);
        dm.set_sd_snappy(0.0); // shells only
        dm.set_sd_tune(0.4);
        dm.trigger_note(38, 1.0);
        let out = render(&mut dm, (0.25 * SR) as usize);
        let win = &out[(0.02 * SR) as usize..]; // past the onset bend
        let f1 = 160.0 * (1.0 + 0.5 * 0.4);
        let m1 = goertzel(win, f1);
        let m2 = goertzel(win, f1 * 1.78);
        let off = goertzel(win, f1 * 1.35); // between the modes
        assert!(m1 > 3.0 * off && m2 > 2.0 * off,
            "both shell modes should stand above the floor: m1={m1:.2} m2={m2:.2} off={off:.2}");

        // Snappy adds broadband top the shells don't have
        let hf = |snappy: f32| -> f32 {
            let mut dm = DrumMachine::new(SR);
            dm.set_sd_snappy(snappy);
            dm.trigger_note(38, 1.0);
            let out = render(&mut dm, (0.2 * SR) as usize);
            goertzel(&out, 5000.0)
        };
        assert!(hf(1.0) > 4.0 * hf(0.0), "snappy should gate the noise path in");
    }

    #[test]
    fn clap_flams_then_tails() {
        let mut dm = DrumMachine::new(SR);
        dm.set_cp_decay(0.5);
        dm.trigger_note(39, 1.0);
        let out = render(&mut dm, (0.4 * SR) as usize);
        // Envelope follower; the three palms make local maxima ~12 ms apart
        let mut env = 0.0f32;
        let envelope: Vec<f32> = out
            .iter()
            .map(|s| {
                let r = s.abs();
                env = if r > env { r } else { env * 0.9993 };
                env
            })
            .collect();
        // Count re-attacks: envelope rising by >20% after having fallen
        let mut reattacks = 0;
        let mut peak = 0.0f32;
        let mut fallen = false;
        for &e in &envelope[..(0.06 * SR) as usize] {
            if e > peak {
                if fallen && e > peak * 1.02 {
                    reattacks += 1;
                    fallen = false;
                }
                peak = e;
            } else if e < peak * 0.75 {
                fallen = true;
                peak = e / 1.02;
            }
        }
        assert!(reattacks >= 2, "the flam should retrigger, got {reattacks} re-attacks");
        // And the tail must outlive the flam window
        let tail_rms: f32 = out[(0.1 * SR) as usize..(0.2 * SR) as usize]
            .iter()
            .map(|s| s * s)
            .sum::<f32>()
            .sqrt();
        assert!(tail_rms > 1e-3, "the room tail should ring past the flam");
    }

    #[test]
    fn hats_live_above_five_kilohertz_and_choke() {
        let mut dm = DrumMachine::new(SR);
        dm.trigger_note(46, 1.0); // open
        let out = render(&mut dm, (0.3 * SR) as usize);
        let high = goertzel(&out, 8000.0);
        let low = goertzel(&out, 800.0);
        assert!(high > 6.0 * low, "hat energy should sit high: 8k={high:.2} 800={low:.2}");

        // Choke: open hat, then closed 100 ms later — the ring must die
        let ring_with = |choke: bool| -> f32 {
            let mut dm = DrumMachine::new(SR);
            dm.set_oh_decay(1.0);
            dm.trigger_note(46, 1.0);
            for _ in 0..(0.1 * SR) as usize {
                dm.render_next();
            }
            if choke {
                dm.trigger_note(42, 0.8);
            }
            // measure 60..160 ms after the (possible) choke
            for _ in 0..(0.06 * SR) as usize {
                dm.render_next();
            }
            let mut e = 0.0;
            for _ in 0..(0.1 * SR) as usize {
                let (l, _) = dm.render_next();
                e += l * l;
            }
            e
        };
        let open_ring = ring_with(false);
        let choked = ring_with(true);
        assert!(
            choked < 0.3 * open_ring,
            "closed hat must choke the open hat: open={open_ring:.4}, choked={choked:.4}"
        );
    }

    #[test]
    fn closed_hat_is_shorter_than_open() {
        let length = |note: u8| -> usize {
            let mut dm = DrumMachine::new(SR);
            dm.trigger_note(note, 1.0);
            let out = render(&mut dm, SR as usize);
            let peak = out.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
            out.iter()
                .rposition(|s| s.abs() > peak * 0.05)
                .unwrap_or(0)
        };
        let ch = length(42);
        let oh = length(46);
        assert!(oh > 2 * ch, "open hat should ring far longer: ch={ch}, oh={oh}");
    }

    #[test]
    fn rim_is_a_fast_bright_knock() {
        let mut dm = DrumMachine::new(SR);
        dm.trigger_note(37, 1.0);
        let out = render(&mut dm, (0.3 * SR) as usize);
        let peak = out.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(peak > 0.5, "rim should crack, peak={peak}");
        // Gone within 120 ms
        let tail = out[(0.12 * SR) as usize..]
            .iter()
            .fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(tail < peak * 0.02, "rim must be all transient, tail={tail}");
    }

    #[test]
    fn accent_hits_harder_and_dirtier() {
        let measure = |vel: f32| -> (f32, f32) {
            let mut dm = DrumMachine::new(SR);
            dm.set_bd_sweep(0.0);
            dm.set_bd_attack(0.0);
            dm.trigger_note(36, vel);
            let out = render(&mut dm, (0.5 * SR) as usize);
            let peak = out.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
            let f0 = 42.0 * (88.0f32 / 42.0).powf(0.35);
            // Measure while the envelope still holds the shell inside the
            // shaper's knee — that is where the accent bite lives
            let win = &out[(0.01 * SR) as usize..(0.15 * SR) as usize];
            (peak, goertzel(win, 3.0 * f0) / goertzel(win, f0).max(1e-9))
        };
        let (soft_peak, soft_h3) = measure(0.2);
        let (hard_peak, hard_h3) = measure(1.0);
        assert!(hard_peak > soft_peak * 1.3, "accent should raise the VCA peak");
        assert!(
            hard_h3 > soft_h3,
            "accent should push the shaper harder too: {soft_h3:.4} vs {hard_h3:.4}"
        );
    }

    #[test]
    fn board_goes_quiet_and_reports_it() {
        let mut dm = DrumMachine::new(SR);
        dm.trigger_note(36, 1.0);
        dm.trigger_note(38, 1.0);
        dm.trigger_note(46, 1.0);
        assert!(dm.is_active());
        let mut peak = 0.0f32;
        for _ in 0..(4.0 * SR) as usize {
            let (l, r) = dm.render_next();
            assert!(l.is_finite() && r.is_finite());
            peak = peak.max(l.abs());
        }
        assert!(peak > 1.0, "the board speaks in volts and should be hot");
        assert!(!dm.is_active(), "everything decays to silence");
    }

    #[test]
    fn drum_names_map() {
        assert_eq!(note_from_name("BD"), Some(36));
        assert_eq!(note_from_name("sd"), Some(38));
        assert_eq!(note_from_name("Oh"), Some(46));
        assert_eq!(note_from_name("C4"), None);
    }
}
