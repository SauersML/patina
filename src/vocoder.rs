// The channel vocoder — speech as a control signal for an analog circuit.
//
// The lineage runs from Homer Dudley's Bell Labs Voder/Vocoder (1939)
// through the Sennheiser VSM 201 (1977), the 20-band unit behind Kraftwerk
// and Herbie Hancock, and the DigiTech Talker's formant-following pedal
// take on the same idea. The topology is unchanged since Dudley:
//
//   modulator ──► analysis filterbank ──► rectifier + RC lag ──┐  (per band)
//                                                              ▼
//   carrier ────► synthesis filterbank ──────────────► VCA ──► Σ ──► out
//
// Each band of the SPEECH signal measures "how much energy does the mouth
// put here right now", and that envelope opens a VCA on the same band of
// the CARRIER — the synth's own oscillators. The instrument talks; the
// speech never reaches the output at all.
//
// Circuit notes, per the VSM 201 service manual's architecture:
//   - Band filters are 4th-order bandpass (two cascaded 2nd-order
//     sections) — steep enough that adjacent bands don't smear formants.
//   - Envelope followers are full-wave rectifiers into an RC lag: ~3 ms
//     attack so plosives keep their edge, ~20 ms release so speech doesn't
//     stutter on every glottal pulse.
//   - The top bands blend a noise generator into the carrier: fricatives
//     (S, SH, F) are noise in the mouth, and a purely harmonic carrier has
//     nothing up there to articulate. The VSM 201 does this with its
//     voiced/unvoiced detector; the Talker just leaks treble noise. We
//     hand each high band a fixed noise feed and let its own envelope
//     decide — sibilance appears exactly when the speech has it.
//
// GAIN-STAGING CONTRACT (learned the hard way; encoded in
// `carrier_level_saturates_output`): each band's VCA is an OTA — a tanh
// INSIDE the band sum — and on any real carrier those tanh stages run
// saturated. Consequences a mixer must know:
//   - Pushing the CARRIER (or the modulator) changes TIMBRE, not level.
//     A hotter carrier drives the tanh harder and buzzes; output RMS
//     barely moves.
//   - The only level control that scales the output is POST-tanh makeup:
//     the per-mode `makeup` here, and `vox_level` downstream (whose
//     0..2 range exists precisely because of this cap).
// Do not "fix" a quiet vocoder chord by raising the carrier — raise
// vox_level. Do not expect vox mix moves to survive a carrier rebalance
// symmetrically — they won't, and that is the circuit, not a bug.

use crate::noise::NoiseSource;

/// Number of channels: 20, like the VSM 201, over 120 Hz - 7.2 kHz.
/// The extra resolution around F2 (800-2500 Hz) is where vowel identity
/// lives — fewer, wider bands smear EH into IH and words stop parsing.
const BANDS: usize = 20;
const F_LO: f32 = 120.0;
const F_HI: f32 = 7200.0;

/// Per-section Q of the band filters (two sections cascade per band).
const BAND_Q: f32 = 4.0;

/// The two circuits behind the front-panel switch.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum VocoderMode {
    /// The '97 DigiTech Talker's "TalkBox" setting: the response of a
    /// driver-and-tube rig (Heil style) done electronically. The tube
    /// chokes the lows, the mouth cavity honks the mids around 1-2 kHz,
    /// little survives past 5 kHz — and the articulation is effectively
    /// instantaneous, with amp grit on the way out.
    TalkBox,
    /// The full-range studio board: every band equal, gentler VCAs.
    Vocoder,
    /// The true Talker circuit (talker.rs): LPC formant tracking — one
    /// continuous filter, no bands at all. Routed in VoxBox.
    Talker,
    /// The FFT cross-synthesizer (spectral.rs): ~500 effective bands,
    /// cepstral envelopes, whitened carrier. Words fully clear, tone
    /// fully the instrument's. Routed in VoxBox.
    Spectral,
}

/// A 2nd-order bandpass section (constant peak gain).
#[derive(Clone, Copy, Default)]
struct Bandpass {
    b0: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Bandpass {
    fn tuned(fc: f32, q: f32, sample_rate: f32) -> Self {
        // The band centres are fixed in Hz (120 Hz .. 7.2 kHz) but the
        // host picks the sample rate, and the top of the bank is above
        // Nyquist for every rate under ~14.4 kHz. There the RBJ form's
        // alpha goes negative and a2 leaves the unit circle: the filter
        // is not detuned, it EXPLODES, and inf/NaN reaches the host's
        // buffer (measured: TalkBox mode non-finite within 700 samples
        // at 8 kHz, the lowest rate the engine accepts). Keep every
        // centre inside the band the rate can actually carry.
        let fc = fc.clamp(1.0, 0.45 * sample_rate);
        let w0 = std::f32::consts::TAU * fc / sample_rate;
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: alpha / a0,
            a1: -2.0 * w0.cos() / a0,
            a2: (1.0 - alpha) / a0,
            ..Self::default()
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        // b1 = 0 and b2 = -b0 for the RBJ constant-peak bandpass
        let y = self.b0 * (x - self.x2) - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

struct Channel {
    fc: f32,
    analysis: [Bandpass; 2],
    synthesis: [Bandpass; 2],
    env: f32,
    /// How much of this band's carrier feed is the noise bus (fricative
    /// articulation for the high bands).
    noise_mix: f32,
    /// The mode's frequency response, per band (the talk-box tube).
    weight: f32,
}

pub struct Vocoder {
    channels: Vec<Channel>,
    noise: NoiseSource,
    sample_rate: f32,
    attack: f32,
    release: f32,
    /// VCA drive and post-gain: the TalkBox mode leans on the amp.
    drive: f32,
    makeup: f32,
    /// The voiced/unvoiced switch (VSM 201): when the speech frame is
    /// mostly high-band energy — a fricative or a stop burst — the whole
    /// carrier crossfades to noise for that instant, so consonants cut
    /// through instead of smearing into the chord. This is the single
    /// biggest intelligibility feature a channel vocoder has.
    unvoiced_mix: f32,
}

impl Vocoder {
    pub fn new(sample_rate: f32) -> Self {
        let channels = (0..BANDS)
            .map(|k| {
                let fc = F_LO * (F_HI / F_LO).powf(k as f32 / (BANDS - 1) as f32);
                Channel {
                    fc,
                    analysis: [Bandpass::tuned(fc, BAND_Q, sample_rate); 2],
                    synthesis: [Bandpass::tuned(fc, BAND_Q, sample_rate); 2],
                    env: 0.0,
                    noise_mix: if fc >= 3500.0 {
                        0.7
                    } else if fc >= 2000.0 {
                        0.25
                    } else {
                        0.0
                    },
                    weight: 1.0,
                }
            })
            .collect();
        let mut v = Self {
            channels,
            noise: NoiseSource::new(sample_rate),
            sample_rate,
            attack: 0.0,
            release: 0.0,
            drive: 1.6,
            makeup: 2.5,
            unvoiced_mix: 0.0,
        };
        v.set_mode(VocoderMode::TalkBox);
        v
    }

    pub fn set_mode(&mut self, mode: VocoderMode) {
        let (attack, release) = match mode {
            // A tube in the mouth articulates at the speed of sound, and
            // the Talker's tracker was nearly as fast
            VocoderMode::TalkBox => (0.0015, 0.012),
            VocoderMode::Vocoder | VocoderMode::Talker | VocoderMode::Spectral => (0.003, 0.020),
        };
        self.attack = 1.0 - (-1.0 / (attack * self.sample_rate)).exp();
        self.release = 1.0 - (-1.0 / (release * self.sample_rate)).exp();
        match mode {
            VocoderMode::TalkBox => {
                self.drive = 2.6;
                self.makeup = 2.6;
            }
            VocoderMode::Vocoder | VocoderMode::Talker | VocoderMode::Spectral => {
                // Makeup sized for a SINGLE carrier voice split across
                // 20 bands (measured ~10 dB shy on one-note leads when
                // this was 2.5, tuned on chord beds)
                self.drive = 1.6;
                self.makeup = 4.2;
            }
        }
        for ch in &mut self.channels {
            ch.weight = match mode {
                VocoderMode::Vocoder | VocoderMode::Talker | VocoderMode::Spectral => 1.0,
                VocoderMode::TalkBox => {
                    // The tube-and-mouth passband: a broad presence bump
                    // centered near 1.4 kHz, the driver's lows choked off,
                    // a steeper shelf past 5 kHz (just enough spit left)
                    let x = (ch.fc / 1400.0).log2();
                    let bump = (-0.5 * (x / 1.35) * (x / 1.35)).exp();
                    let low_choke = if ch.fc < 260.0 { 0.25 } else { 1.0 };
                    let high_roll = if ch.fc > 5000.0 { 0.3 } else { 1.0 };
                    (0.1 + 1.5 * bump) * low_choke * high_roll
                }
            };
        }
    }

    /// One sample through the whole board: `modulator` is the speech
    /// (unit-level), `carrier` the instrument (program volts). The output
    /// is in carrier volts — the speech only ever steers.
    #[inline]
    pub fn process(&mut self, modulator: f32, carrier: f32) -> f32 {
        // One noise generator serves every high band, like the shared
        // avalanche transistor on the voice boards
        let noise = self.noise.next() * 4.0;
        // The voiced/unvoiced detector reads last sample's band envelopes
        // (one sample of lag, milliseconds ahead of the RC followers):
        // a frame that is mostly high-band energy is a consonant
        let (mut hf, mut total) = (0.0f32, 1e-9f32);
        for ch in &self.channels {
            total += ch.env;
            if ch.fc > 3300.0 {
                hf += ch.env;
            }
        }
        let unvoiced_target = if hf > 0.42 * total { 1.0 } else { 0.0 };
        // ~4 ms crossfade: fast enough for a T burst, no clicks
        self.unvoiced_mix +=
            (unvoiced_target - self.unvoiced_mix) * (250.0 / self.sample_rate).min(1.0);

        let mut out = 0.0;
        for ch in &mut self.channels {
            let mut m = modulator;
            for bp in &mut ch.analysis {
                m = bp.tick(m);
            }
            // Full-wave rectifier into the RC lag
            let rect = m.abs();
            let k = if rect > ch.env { self.attack } else { self.release };
            ch.env += (rect - ch.env) * k;

            let nm = ch.noise_mix.max(self.unvoiced_mix);
            let mut c = carrier * (1.0 - nm) + noise * nm;
            for bp in &mut ch.synthesis {
                c = bp.tick(c);
            }
            // The VCA: an OTA stage, so a hard-driven band saturates softly
            out += (c * ch.env * ch.weight * self.drive).tanh() * self.makeup;
        }
        out
    }

    /// Sum of the band envelopes — cheap "is anyone talking" telemetry for
    /// panel meters.
    pub fn activity(&self) -> f32 {
        self.channels.iter().map(|c| c.env).sum()
    }
}

impl VocoderMode {
    pub fn from_value(v: f32) -> Self {
        match v.round() as i32 {
            i32::MIN..=0 => VocoderMode::TalkBox,
            1 => VocoderMode::Vocoder,
            2 => VocoderMode::Talker,
            _ => VocoderMode::Spectral,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defining property of a vocoder: a silent modulator mutes the
    /// carrier completely, and a talking modulator lets it through.
    #[test]
    fn modulator_gates_carrier() {
        let sr = 48000.0;
        let mut v = Vocoder::new(sr);
        // Carrier: a loud 110 Hz saw (harmonics across the whole bank)
        let saw = |n: usize| (((n as f32 * 110.0 / sr) % 1.0) * 2.0 - 1.0) * 5.0;

        // Phase 1: modulator silent
        let mut quiet = 0.0f32;
        for n in 0..(sr as usize / 2) {
            quiet = quiet.max(v.process(0.0, saw(n)).abs());
        }
        // Phase 2: modulator = 150 Hz buzz (a voiced "speech" stand-in)
        let mut loud = 0.0f32;
        for n in 0..(sr as usize / 2) {
            let m = if (n as f32 * 150.0 / sr) % 1.0 < 0.5 { 0.5 } else { -0.5 };
            loud = loud.max(v.process(m, saw(n)).abs());
        }
        assert!(quiet < 0.05, "silent modulator must mute the carrier, got {quiet}");
        assert!(loud > 20.0 * quiet.max(1e-6), "speech should open the VCAs: {loud} vs {quiet}");
        assert!(loud.is_finite());
    }

    /// Band selectivity: a modulator tone in one register must open that
    /// register of the carrier and not the other end of the bank.
    #[test]
    fn formants_transfer_to_the_carrier() {
        let sr = 48000.0;
        let mut v = Vocoder::new(sr);
        // Modulator: 400 Hz sine. Carrier: white-ish two-tone rich saw.
        let mut out = Vec::with_capacity(sr as usize);
        for n in 0..(sr as usize) {
            let t = n as f32 / sr;
            let m = (std::f32::consts::TAU * 400.0 * t).sin() * 0.6;
            let c = (((t * 110.0) % 1.0) * 2.0 - 1.0) * 5.0;
            out.push(v.process(m, c));
        }
        let goertzel = |freq: f32| -> f32 {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, &s) in out[out.len() / 2..].iter().enumerate() {
                let a = std::f32::consts::TAU * freq * i as f32 / sr;
                re += s * a.cos();
                im += s * a.sin();
            }
            (re * re + im * im).sqrt()
        };
        // Carrier harmonics near 440 Hz should sound; harmonics near 3 kHz
        // should stay shut (no modulator energy up there)
        let low = goertzel(440.0);
        let high = goertzel(2970.0);
        assert!(
            low > 6.0 * high,
            "400 Hz speech energy must open the low bands only: low={low}, high={high}"
        );
    }

    /// The gain-staging contract from the module header: the per-band
    /// tanh caps its own output, so doubling the carrier must NOT double
    /// the output (timbre moves, level doesn't) — while post-tanh makeup
    /// scales it exactly. If this test starts failing because the
    /// architecture changed, rewrite the header contract too.
    #[test]
    fn carrier_level_saturates_output() {
        let sr = 48000.0;
        let rms_with = |carrier_gain: f32| -> f32 {
            let mut v = Vocoder::new(sr);
            v.set_mode(VocoderMode::Vocoder);
            let mut acc = 0.0f64;
            let n = sr as usize / 2;
            for k in 0..n {
                let t = k as f32 / sr;
                // Voiced buzz modulator, saw carrier in program volts
                let m = if (t * 130.0) % 1.0 < 0.5 { 0.5 } else { -0.5 };
                let c = (((t * 110.0) % 1.0) * 2.0 - 1.0) * 5.0 * carrier_gain;
                let y = v.process(m, c);
                if k > n / 2 {
                    acc += (y * y) as f64;
                }
            }
            ((acc / (n / 2) as f64) as f32).sqrt()
        };
        let unity = rms_with(1.0);
        let doubled = rms_with(2.0);
        let db = 20.0 * (doubled / unity).log10();
        // A linear board would move +6.0 dB; the saturated bands give
        // back well under half of that (measured ~+2.6: the loudest
        // bands are pinned, the quiet ones still linear)
        assert!(
            db < 3.0,
            "per-band tanh must cap the level: +6 dB carrier moved output {db:+.2} dB"
        );
        assert!(unity > 0.05, "the board should still pass signal, rms={unity}");
    }

    /// TalkBox mode is the tube: compared to the studio vocoder, the
    /// bottom octave must be choked relative to the mid presence region.
    #[test]
    fn talkbox_chokes_the_lows() {
        let sr = 48000.0;
        let balance = |mode: VocoderMode| -> f32 {
            let mut v = Vocoder::new(sr);
            v.set_mode(mode);
            let mut out = Vec::with_capacity(sr as usize);
            for n in 0..(sr as usize) {
                let t = n as f32 / sr;
                // Broadband buzz modulator opens every band
                let m = if (t * 130.0) % 1.0 < 0.5 { 0.5 } else { -0.5 };
                let c = (((t * 65.0) % 1.0) * 2.0 - 1.0) * 5.0;
                out.push(v.process(m, c));
            }
            let goertzel = |freq: f32| -> f32 {
                let (mut re, mut im) = (0.0f32, 0.0f32);
                for (i, &s) in out[out.len() / 2..].iter().enumerate() {
                    let a = std::f32::consts::TAU * freq * i as f32 / sr;
                    re += s * a.cos();
                    im += s * a.sin();
                }
                (re * re + im * im).sqrt()
            };
            goertzel(130.0) / goertzel(1495.0).max(1e-9)
        };
        let talkbox = balance(VocoderMode::TalkBox);
        let studio = balance(VocoderMode::Vocoder);
        assert!(
            talkbox < 0.4 * studio,
            "the tube should choke lows vs the studio board: talkbox={talkbox}, studio={studio}"
        );
    }
}
