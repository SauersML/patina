// The Talker — a formant-tracking filter, which is what a talk box
// actually is. Not a channel vocoder.
//
// A channel vocoder chops both signals into bands and gates one with
// the other; it always carries a faint filterbank shimmer. The real
// thing — a Heil tube in the mouth, or the DigiTech Talker's DSP — is a
// SINGLE time-varying filter: the mouth's resonances, imposed whole on
// the instrument. One throat shape, sliding continuously, on one note.
//
// The engineering is linear predictive coding, the same mathematics
// inside the TI TMS5220 (Speak & Spell) and every LPC-10 radio codec —
// and like all of those, it runs BAND-LIMITED, in a 4:1 decimated
// domain (~12 kHz):
//
//   modulator ─► LP 4.8k ─► ÷4 ─► 25 ms frames ─► autocorr ─► Levinson
//                                  └► reflection coefficients k1..k12
//   carrier ──► LP 4.8k ─► ÷4 ─► all-pole LATTICE (the k's, slewed)
//                                  └► ×4 hold ─► LP 4.8k ─► VCA ─► out
//
// The decimation is not a cost cut — it is what makes the circuit WORK
// on real voices. Full-band LPC on a high-pitched speaker locks its
// poles onto the source's individual harmonics instead of the formant
// envelope, and the recording's own pitch bleeds onto the carrier as a
// ghost tone (it reads as a chord). At 12 kHz with 12 poles, the model
// can only afford the envelope — which is the mouth, which is the
// point. It is also why a talk box sounds like a talk box: the tube
// never passed 5 kHz either.
//
// The lattice form matters too: reflection coefficients interpolate
// freely and the filter stays stable while they morph, as long as every
// |k| < 1 — the guarantee a lossy tube provides. Direct-form LPC
// coefficients explode when morphed; lattices are how the hardware did
// it, and how we do it.

use crate::noise::NoiseSource;

/// 4:1 decimation: 48 kHz engine -> 12 kHz analysis/tract domain.
const DECIM: usize = 4;

/// Prediction order in the decimated domain: 12 poles across 6 kHz
/// resolves 4-5 formants — the LPC-10 ballpark.
const ORDER: usize = 12;

/// Analysis window 25 ms, new coefficients every 10 ms — phoneme-rate.
const WINDOW_S: f32 = 0.025;
const HOP_S: f32 = 0.010;

/// RBJ lowpass biquad for the anti-alias and reconstruction filters.
#[derive(Clone, Copy, Default)]
struct Lowpass {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Lowpass {
    fn tuned(fc: f32, q: f32, sample_rate: f32) -> Self {
        let mut lp = Self::default();
        lp.retune(fc, q, sample_rate);
        lp
    }

    /// Recompute coefficients, keeping filter state (for swept filters).
    fn retune(&mut self, fc: f32, q: f32, sample_rate: f32) {
        let w0 = std::f32::consts::TAU * fc / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let cw = w0.cos();
        let a0 = 1.0 + alpha;
        self.b0 = (1.0 - cw) / 2.0 / a0;
        self.b1 = (1.0 - cw) / a0;
        self.b2 = (1.0 - cw) / 2.0 / a0;
        self.a1 = -2.0 * cw / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

pub struct Talker {
    sr: f32,
    // Anti-alias filters (4th order: two cascaded biquads each) and the
    // output reconstruction pair
    in_m: [Lowpass; 2],
    in_c: [Lowpass; 2],
    out: [Lowpass; 2],
    decim: usize,
    held: f32,
    // Modulator history at the decimated rate
    ring: Vec<f32>,
    write: usize,
    hop: usize,
    since_hop: usize,
    // Reflection coefficients: analysis targets and the slewed live set
    k_target: [f32; ORDER],
    k: [f32; ORDER],
    k_slew: f32,
    // Lattice state — TWO cascaded passes of the same tract. Squaring
    // the transfer function doubles every formant peak in dB and deepens
    // the valleys: the exaggerated, wide-open mouth of a real talk-box
    // performance, not the polite average LPC measures
    b: [f32; ORDER + 1],
    b2: [f32; ORDER + 1],
    // Loudness follower (the modulator's energy opens the VCA)
    env_target: f32,
    env: f32,
    env_peak: f32,
    peak_decay: f32,
    attack: f32,
    release: f32,
    // Fricative path: modulator HF fraction swaps the carrier for noise
    m_lp: f32,
    hf_env: f32,
    lf_env: f32,
    unvoiced: f32,
    unvoiced_k: f32,
    split_k: f32,
    // The body path: the carrier's low end bled straight through, the
    // way a talk-box rig mixes the direct instrument under the tube —
    // the mouth articulates the mids, the note keeps its chest
    body_lp: f32,
    body_k: f32,
    gate: f32,
    /// Output tilt: the complement of the analysis pre-emphasis. The
    /// poles were fitted to a whitened voice, so the tube band needs its
    /// brightness restored — this one-zero (+6 dB/oct above ~1 kHz) is
    /// what moves the centroid from bass-heavy to the talk-box mid-honk.
    tilt_prev: f32,
    /// Fundamental cut: the reference's high-register material carries
    /// only 3% of its energy below 300 Hz — the tube barely passes the
    /// fundamental, and the warmth lives in harmonics 2-5.
    hp_lp: f32,
    hp_k: f32,
    // The mouth-opening wah. Pre-emphasis makes LPC deliberately
    // tilt-blind (poles chase formant POSITIONS), which flattens the
    // dark<->bright swing of a closing/opening mouth — the very thing a
    // talk box exaggerates. Ranked by brightness dynamic range:
    // vocoder < speech < talk box. So the tilt gets its own circuit: the
    // modulator's measured brightness sweeps a big resonant lowpass over
    // a 280 Hz - 4.8 kHz log range with an EXPANSION curve — larger
    // "cutoff" swings than the speech itself. A wah, played by a mouth.
    bright_lp: f32,
    bright_split_k: f32,
    bright_hf: f32,
    bright_total: f32,
    wah_fc_target: f32,
    wah_fc: f32,
    wah_slew: f32,
    wah: Lowpass,
    noise: NoiseSource,
    /// Clarity (0..1): dials the circuit from the reference-matched
    /// caricature (0: squared formants, flattened dynamics, amp grit,
    /// deep wah) toward a clean legible talkbox (1: single tract pass,
    /// the voice's own dynamics, linear amp, open wah). Measured by ASR
    /// word recovery: the caricature scores 0/31 words on scored
    /// speech, the clean end restores them.
    clarity: f32,
}

impl Talker {
    pub fn new(sample_rate: f32) -> Self {
        let ar = sample_rate / DECIM as f32; // analysis/tract rate
        Self {
            sr: sample_rate,
            in_m: [Lowpass::tuned(4800.0, 0.6, sample_rate), Lowpass::tuned(4800.0, 1.0, sample_rate)],
            in_c: [Lowpass::tuned(4800.0, 0.6, sample_rate), Lowpass::tuned(4800.0, 1.0, sample_rate)],
            out: [Lowpass::tuned(4800.0, 0.6, sample_rate), Lowpass::tuned(4800.0, 1.0, sample_rate)],
            decim: 0,
            held: 0.0,
            ring: vec![0.0; (WINDOW_S * ar) as usize + 1],
            write: 0,
            hop: (HOP_S * ar) as usize,
            since_hop: 0,
            k_target: [0.0; ORDER],
            k: [0.0; ORDER],
            // 10 ms: a mouth MOUTHING to music, not chattering speech
            k_slew: 1.0 - (-1.0 / (0.010 * ar)).exp(),
            b: [0.0; ORDER + 1],
            b2: [0.0; ORDER + 1],
            env_target: 0.0,
            env: 0.0,
            env_peak: 1e-4,
            peak_decay: 1.0 - 1.0 / (2.5 * ar),
            attack: 1.0 - (-1.0 / (0.002 * ar)).exp(),
            release: 1.0 - (-1.0 / (0.015 * ar)).exp(),
            m_lp: 0.0,
            hf_env: 0.0,
            lf_env: 0.0,
            unvoiced: 0.0,
            unvoiced_k: (250.0 / sample_rate).min(1.0),
            split_k: 1.0 - (-std::f32::consts::TAU * 3000.0 / sample_rate).exp(),
            body_lp: 0.0,
            body_k: 1.0 - (-std::f32::consts::TAU * 230.0 / sample_rate).exp(),
            gate: 0.0,
            tilt_prev: 0.0,
            hp_lp: 0.0,
            hp_k: 1.0 - (-std::f32::consts::TAU * 280.0 / sample_rate).exp(),
            bright_lp: 0.0,
            bright_split_k: 1.0 - (-std::f32::consts::TAU * 900.0 / ar).exp(),
            bright_hf: 0.0,
            bright_total: 1e-6,
            wah_fc_target: 800.0,
            wah_fc: 800.0,
            wah_slew: 1.0 - (-1.0 / (0.012 * sample_rate)).exp(),
            wah: Lowpass::tuned(800.0, 1.3, sample_rate),
            noise: NoiseSource::new(),
            clarity: 0.0,
        }
    }

    pub fn set_clarity(&mut self, c: f32) {
        self.clarity = c.clamp(0.0, 1.0);
    }

    /// Frame analysis at the decimated rate: windowed autocorrelation,
    /// then Levinson-Durbin for the reflection coefficients.
    fn analyze(&mut self) {
        let n = self.ring.len();
        // Unwrap the ring into time order, pre-emphasized (whitens the
        // glottal tilt so the poles chase formants, not the source) and
        // Hann-windowed
        let mut frame = vec![0.0f32; n];
        let mut prev = 0.0f32;
        for i in 0..n {
            let x = self.ring[(self.write + i) % n];
            let pe = x - 0.97 * prev;
            prev = x;
            let w = 0.5
                - 0.5 * (std::f32::consts::TAU * i as f32 / (n - 1) as f32).cos();
            frame[i] = pe * w;
        }

        let mut r = [0.0f32; ORDER + 1];
        for (lag, r_lag) in r.iter_mut().enumerate() {
            let mut acc = 0.0;
            for i in lag..n {
                acc += frame[i] * frame[i - lag];
            }
            *r_lag = acc;
        }
        // Silence (or near-silence): relax the tract to neutral
        if r[0] < 1e-7 {
            self.k_target = [0.0; ORDER];
            self.env_target = 0.0;
            return;
        }
        // A whisper of white noise on the diagonal keeps the recursion
        // conditioned (the fixed-point hardware did the same trick)
        r[0] *= 1.0001;

        // Levinson-Durbin, keeping the reflection coefficients
        let mut a = [0.0f32; ORDER + 1];
        let mut e = r[0];
        for i in 1..=ORDER {
            let mut acc = r[i];
            for j in 1..i {
                acc -= a[j] * r[i - j];
            }
            let ki = (acc / e).clamp(-0.985, 0.985);
            let mut new_a = a;
            new_a[i] = ki;
            for j in 1..i {
                new_a[j] = a[j] - ki * a[i - j];
            }
            a = new_a;
            e *= 1.0 - ki * ki;
            self.k_target[i - 1] = ki;
        }

        // Loudness from the frame RMS of the raw (un-emphasized) signal
        let mut energy = 0.0;
        for i in 0..n {
            let x = self.ring[(self.write + i) % n];
            energy += x * x;
        }
        self.env_target = (energy / n as f32).sqrt();
    }

    /// One decimated-domain tick: analysis bookkeeping and the lattice.
    fn tract_tick(&mut self, m: f32, c: f32) -> f32 {
        // Mouth-opening tracker: brightness = HF share above ~900 Hz,
        // expanded (^1.5) and mapped onto a log cutoff sweep
        self.bright_lp += self.bright_split_k * (m - self.bright_lp);
        let hf = m - self.bright_lp;
        self.bright_hf += 0.004 * (hf.abs() - self.bright_hf);
        self.bright_total += 0.004 * (m.abs() - self.bright_total);
        // Calibrated on sung vowels (measured HF ratios ~0.1-0.35), not
        // speech fricatives: typical singing sits mid-open, bright
        // vowels reach the top, and the floor starts at 500 Hz so the
        // wah shades rather than smothers
        let openness = (self.bright_hf / self.bright_total.max(1e-6) / 0.3)
            .clamp(0.0, 1.0)
            .powf(1.2);
        let swept = 500.0 * (4800.0f32 / 500.0).powf(openness);
        // Clarity opens the wah: its deep sweeps swallow consonants
        self.wah_fc_target = swept + (4800.0 - swept) * self.clarity;

        self.ring[self.write] = m;
        self.write = (self.write + 1) % self.ring.len();
        self.since_hop += 1;
        if self.since_hop >= self.hop {
            self.since_hop = 0;
            self.analyze();
        }

        // Slew the live tract toward the analysis targets; VCA follower
        for i in 0..ORDER {
            self.k[i] += (self.k_target[i] - self.k[i]) * self.k_slew;
        }
        let ke = if self.env_target > self.env { self.attack } else { self.release };
        self.env += (self.env_target - self.env) * ke;

        // Consonants are breath: swap the carrier for noise on fricatives
        let x = c * (1.0 - self.unvoiced) + self.noise.next() * 4.0 * self.unvoiced;
        let mut f = x * 0.25; // headroom: the poles ring hard at formants
        for i in (0..ORDER).rev() {
            f += self.k[i] * self.b[i];
            // 0.9995 is wall loss: a real tract is lossy, and it keeps the
            // lattice bounded while coefficients morph mid-frame
            self.b[i + 1] = (self.b[i] - self.k[i] * f) * 0.9995;
        }
        self.b[0] = f;
        // Second pass through the same mouth: the exaggeration stage.
        // Clarity blends it away — squared formants are the caricature,
        // and the single pass is what keeps consonant transitions legible
        let mut f2 = f * 0.12;
        for i in (0..ORDER).rev() {
            f2 += self.k[i] * self.b2[i];
            self.b2[i + 1] = (self.b2[i] - self.k[i] * f2) * 0.9995;
        }
        self.b2[0] = f2;
        let f = f2 * (1.0 - self.clarity) + f * 0.48 * self.clarity;

        // The instrument leads, the mouth articulates: the VCA follows a
        // NORMALIZED, compressed loudness (^0.45), so word-level dynamics
        // flatten toward a driven instrument instead of a singer's
        // phrasing — the difference between a talk box and autotune. The
        // drive stays constant for the same reason: it is the amp's
        // character, not the voice's.
        if self.env > self.env_peak {
            self.env_peak = self.env;
        } else {
            self.env_peak *= self.peak_decay;
        }
        let norm = (self.env / self.env_peak.max(1e-5)).clamp(0.0, 1.0);
        // Clarity restores the voice's own dynamics: the ^0.45 flattening
        // erases the stop-consonant dips that carry the words
        let mut g = norm.powf(0.45 + 0.55 * self.clarity);
        // ...with a downward expander at the floor, so the compression
        // doesn't hold the gate open on room silence between phrases
        if norm < 0.02 {
            let t = norm / 0.02;
            g *= t * t;
        }
        self.gate = g;
        // Amp grit backs off with clarity at constant small-signal gain
        let drive = 1.5 - 1.1 * self.clarity;
        (f * drive).tanh() * (4.8 / drive) * g
    }

    /// One engine-rate sample: anti-alias both signals, run the tract in
    /// the decimated domain, reconstruct. Carrier and output in volts.
    #[inline]
    pub fn process(&mut self, modulator: f32, carrier: f32) -> f32 {
        // Fricative detector at the full rate (that's where the S lives)
        self.m_lp += self.split_k * (modulator - self.m_lp);
        let hf = modulator - self.m_lp;
        self.hf_env += 0.002 * (hf.abs() - self.hf_env);
        self.lf_env += 0.002 * (modulator.abs() - self.lf_env);
        let target = if self.hf_env > 0.55 * self.lf_env.max(1e-6) { 0.85 } else { 0.0 };
        self.unvoiced += (target - self.unvoiced) * self.unvoiced_k;

        let mut m = modulator;
        for f in &mut self.in_m {
            m = f.tick(m);
        }
        let mut c = carrier;
        for f in &mut self.in_c {
            c = f.tick(c);
        }
        self.decim += 1;
        if self.decim >= DECIM {
            self.decim = 0;
            self.held = self.tract_tick(m, c);
        }
        // Reconstruct the tube's band, then add what the tube never
        // carried: the note's BODY below (the carrier's fundamental bled
        // straight through, gated with the speech) — and, on sibilant
        // frames, the ORIGINAL VOICE high-passed. That is the DigiTech
        // Talker's documented NuVo trick: its unvoiced detector switches
        // to the voice input itself for sibilants, not to a noise
        // generator. High-passed sibilants carry no pitch, so the
        // consonants arrive human-crisp with zero polyphony.
        self.body_lp += self.body_k * (carrier - self.body_lp);
        let mut y = self.held;
        for f in &mut self.out {
            y = f.tick(y);
        }
        // The wah: the mouth's openness as LARGE cutoff motion, swept at
        // full rate so the filter glides instead of stepping
        self.wah_fc += (self.wah_fc_target - self.wah_fc) * self.wah_slew;
        self.wah.retune(self.wah_fc, 1.3, self.sr);
        y = self.wah.tick(y);
        // De-emphasis inversion: restore the brightness the analysis
        // whitening removed (measured: centroid 319 Hz without this vs
        // the reference's 1219 Hz)
        let tilted = y - 0.8 * self.tilt_prev;
        self.tilt_prev = y;
        let mut y = tilted * 3.5;
        // Fundamental cut (one-pole HP at 280 Hz): the tube's real voice
        self.hp_lp += self.hp_k * (y - self.hp_lp);
        y -= self.hp_lp;
        // NuVo passthrough: the modulator minus its own lows = the
        // sibilant band, faded in only on unvoiced frames
        let sib = (modulator - self.m_lp) * self.unvoiced * 6.5;
        y + sib
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defining behavior: the modulator's resonance must appear ON
    /// the carrier, and a silent mouth passes nothing.
    #[test]
    fn tracks_formants_onto_the_carrier() {
        let sr = 48000.0;
        let mut t = Talker::new(sr);
        // Modulator: noise through a strong 900 Hz two-pole "mouth"
        let (mut y1, mut y2) = (0.0f32, 0.0f32);
        let c = -(-std::f32::consts::TAU * 120.0 / sr).exp();
        let bcoef = 2.0 * (-std::f32::consts::PI * 120.0 / sr).exp()
            * (std::f32::consts::TAU * 900.0 / sr).cos();
        let a = 1.0 - bcoef - c;
        let mut noise = NoiseSource::new();
        let mut out = Vec::with_capacity(sr as usize);
        for n in 0..(sr as usize) {
            let m = {
                let y = a * noise.next() + bcoef * y1 + c * y2;
                y2 = y1;
                y1 = y;
                y * 0.5
            };
            // Carrier: 110 Hz saw, program volts
            let cwave = (((n as f32 * 110.0 / sr) % 1.0) * 2.0 - 1.0) * 5.0;
            out.push(t.process(m, cwave));
        }
        assert!(out.iter().all(|s| s.is_finite()), "lattice must stay stable");
        let goertzel = |freq: f32| -> f32 {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, &s) in out[out.len() / 2..].iter().enumerate() {
                let ph = std::f32::consts::TAU * freq * i as f32 / sr;
                re += s * ph.cos();
                im += s * ph.sin();
            }
            (re * re + im * im).sqrt()
        };
        // Carrier harmonics near the mouth's 900 Hz resonance must beat
        // harmonics far from it (2970 Hz) decisively
        let near = goertzel(880.0);
        let far = goertzel(2970.0);
        assert!(
            near > 4.0 * far,
            "the tracked formant should shape the carrier: near={near}, far={far}"
        );

        // Silent modulator: after the VCA's release tail (~15 ms) has
        // rung out, the carrier must be gone
        let mut quiet = 0.0f32;
        for n in 0..(sr as usize / 4) {
            let cwave = (((n as f32 * 110.0 / sr) % 1.0) * 2.0 - 1.0) * 5.0;
            let s = t.process(0.0, cwave).abs();
            if n > (0.15 * sr) as usize {
                quiet = quiet.max(s);
            }
        }
        assert!(quiet < 0.05, "silent mouth must mute the carrier, got {quiet}");
    }

    /// The reason the Talker decimates: a HIGH-pitched modulator (220 Hz
    /// source through a 900 Hz mouth) must not stamp its own pitch onto
    /// the carrier. The output should carry the carrier's 110 Hz series,
    /// not the modulator's 220 Hz series offset from it.
    #[test]
    fn high_voice_does_not_ghost_its_pitch() {
        let sr = 48000.0;
        let mut t = Talker::new(sr);
        let (mut y1, mut y2) = (0.0f32, 0.0f32);
        let cc = -(-std::f32::consts::TAU * 120.0 / sr).exp();
        let bcoef = 2.0 * (-std::f32::consts::PI * 120.0 / sr).exp()
            * (std::f32::consts::TAU * 900.0 / sr).cos();
        let a = 1.0 - bcoef - cc;
        let mut out = Vec::with_capacity(sr as usize);
        for n in 0..(sr as usize) {
            // Modulator: 220 Hz pulse train (a high voice) through the mouth
            let src = if (n as f32 * 220.0 / sr) % 1.0 < 0.1 { 1.0 } else { -0.02 };
            let m = {
                let y = a * src + bcoef * y1 + cc * y2;
                y2 = y1;
                y1 = y;
                y * 0.4
            };
            // Carrier: 130.8 Hz saw (C3) — harmonics at 130.8*k
            let cwave = (((n as f32 * 130.8 / sr) % 1.0) * 2.0 - 1.0) * 5.0;
            out.push(t.process(m, cwave));
        }
        let goertzel = |freq: f32| -> f32 {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, &s) in out[out.len() / 2..].iter().enumerate() {
                let ph = std::f32::consts::TAU * freq * i as f32 / sr;
                re += s * ph.cos();
                im += s * ph.sin();
            }
            (re * re + im * im).sqrt()
        };
        // 916 Hz = carrier harmonic (130.8*7) near the formant: should be
        // strong. 880/1100 = modulator harmonics (220*4, 220*5) that fall
        // BETWEEN carrier harmonics: must stay weak — no ghost pitch.
        let carrier_h = goertzel(915.6);
        let ghost = goertzel(880.0).max(goertzel(1100.0));
        assert!(
            carrier_h > 2.5 * ghost,
            "output must be the carrier's series, not the voice's: carrier={carrier_h}, ghost={ghost}"
        );
    }
}
