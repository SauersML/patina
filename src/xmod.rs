// src/xmod.rs
//
// Cross-track modulation: one track's signal drives another track at audio
// rate, and tracks feed one another, loops included. Every route is set on
// the track it drives, naming the track it listens to:
//
//   fm_from=SRC fm=D        linear FM. The track's oscillators run at
//                           f0 (1 + D m), m being SRC's signal (1 = 0 dBFS
//                           at the bus). A sine of level A at fm gives
//                           sidebands at f0 +- k fm with Bessel amplitudes
//                           J_k(D A f0 / fm). The frequency stops at 0 Hz
//                           where D m < -1 (no through-zero).
//   filter_fm_from=SRC filter_fm=D
//                           the cutoff moves D octaves per unit of m.
//   ringmod_from=SRC ringmod=D
//                           the track times (1 - D + D m): D = 1 is a ring
//                           modulator (the carrier is gone, f +- fm are
//                           left), D = 0.5 classic AM.
//   feed_from=SRC feed=G    routed feedback: SRC's signal comes into this
//                           track's strip ahead of its glitch, EQ and
//                           drive, through a delay (feed_time, seconds), a
//                           lowpass (feed_tone, Hz), a DC blocker, the gain
//                           G and a safety limiter (feed_limit, dBFS). A
//                           track may feed itself, two tracks each other: a
//                           loop with gain above 1 rings up to the limiter's
//                           ceiling and stays there, never past it.
//
// The source signal is the source strip after its inserts, before its fader
// (and duck, shaper, compressor and pan), so `gain=0` silences a track that
// should only be heard through the one it drives. Every route reads its
// source as it stood one sample before, so any graph runs in one pass,
// whatever order the tracks are in; a stem bounce still runs every strip a
// route needs. A song with no routes builds none of this.

use crate::oscillator::PROGRAM_V;
use crate::song::Param;

/// Strips run in volts; the bus reads volts at 0.7 / PROGRAM_V at unity
/// volume, so a routed signal of 1 is a 0 dBFS signal on the bus.
pub const TO_BUS: f32 = 0.7 / PROGRAM_V;

/// The longest feed delay.
pub const MAX_FEED_S: f32 = 2.0;

/// Where a route lands on the track it drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dest {
    Fm,
    FilterFm,
    Ring,
    Feed,
}

impl Dest {
    pub const ALL: [Dest; 4] = [Dest::Fm, Dest::FilterFm, Dest::Ring, Dest::Feed];

    /// The track-line option that names the source.
    pub fn option(self) -> &'static str {
        match self {
            Dest::Fm => "fm_from",
            Dest::FilterFm => "filter_fm_from",
            Dest::Ring => "ringmod_from",
            Dest::Feed => "feed_from",
        }
    }

    pub fn from_option(key: &str) -> Option<Dest> {
        Dest::ALL.into_iter().find(|d| d.option() == key)
    }

    /// The route a parameter belongs to: its depth, or one of the feed's
    /// knobs.
    pub fn of_param(p: Param) -> Option<Dest> {
        match p {
            Param::XFm => Some(Dest::Fm),
            Param::XFilterFm => Some(Dest::FilterFm),
            Param::XRing => Some(Dest::Ring),
            Param::Feed | Param::FeedTime | Param::FeedTone | Param::FeedLimit => Some(Dest::Feed),
            _ => None,
        }
    }

    /// FM and filter FM drive a voice's oscillators and filter, which a
    /// synth card (and the voice box's carrier) and a tape-deck slot
    /// have; ring and feed act on the strip, which every track has.
    pub fn needs_pitch_or_filter(self) -> bool {
        matches!(self, Dest::Fm | Dest::FilterFm)
    }
}

/// One route: `src`'s signal into `dst`'s `dest`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Route {
    pub dest: Dest,
    pub src: u16,
    pub dst: u16,
}

/// Every route a song declared, and the scratch the engine gathers their
/// sources into once a sample (`VoiceManager::gather_routes`).
#[derive(Default)]
pub struct RouteTable {
    pub routes: Vec<Route>,
    /// Each route's source this sample (its tap one sample ago, volts).
    pub vals: Vec<(f32, f32)>,
    /// Every channel a route touches, source or driven, once each.
    pub chans: Vec<u16>,
    /// Tape-deck slots driven by FM or filter FM.
    pub slots: Vec<(u16, usize)>,
}

/// A de-zippered depth. A value set before the strip's first sample lands
/// at once (a track line's `fm=2` holds from the downbeat, not a ramp up).
#[derive(Clone, Copy, Default)]
struct Knob {
    target: f32,
    cur: f32,
}

impl Knob {
    #[inline]
    fn step(&mut self, k: f32) -> f32 {
        self.cur += (self.target - self.cur) * k;
        self.cur
    }
}

/// One strip's routing state (`ChannelMix::xmod`): what it lends its
/// sources' listeners, and what its own routes bring it this sample.
#[derive(Clone)]
pub struct StripXmod {
    /// The strip's signal after its inserts, before its fader, in volts:
    /// written as the strip runs, read and cleared by the next sample's
    /// gather (a strip that did not run lends silence).
    pub tap: (f32, f32),
    fm: Knob,
    filter_fm: Knob,
    ring: Knob,
    feed_gain: Knob,
    started: bool,
    /// This sample's FM: the ratio the strip's oscillators run at.
    pub pitch_ratio: f32,
    /// This sample's filter FM, octaves.
    pub cutoff_oct: f32,
    /// This sample's ring modulator (bus units); None when not routed.
    ring_m: Option<f32>,
    /// This sample's routed feed, volts.
    feed_in: (f32, f32),
    feed: Option<Box<FeedPath>>,
}

impl Default for StripXmod {
    fn default() -> Self {
        Self {
            tap: (0.0, 0.0),
            fm: Knob::default(),
            filter_fm: Knob::default(),
            ring: Knob::default(),
            feed_gain: Knob::default(),
            started: false,
            pitch_ratio: 1.0,
            cutoff_oct: 0.0,
            ring_m: None,
            feed_in: (0.0, 0.0),
            feed: None,
        }
    }
}

impl StripXmod {
    /// Open this strip to a route of kind `dest`.
    pub fn enable(&mut self, dest: Dest, sample_rate: f32) {
        match dest {
            Dest::Ring => self.ring_m = Some(self.ring_m.unwrap_or(0.0)),
            Dest::Feed => {
                if self.feed.is_none() {
                    self.feed = Some(Box::new(FeedPath::new(sample_rate)));
                }
            }
            Dest::Fm | Dest::FilterFm => {}
        }
    }

    /// Turn one of the strip's routing knobs (already clamped).
    pub fn set(&mut self, param: Param, value: f32, sample_rate: f32) {
        let fresh = !self.started;
        let knob = |k: &mut Knob| {
            k.target = value;
            if fresh {
                k.cur = value;
            }
        };
        match param {
            Param::XFm => knob(&mut self.fm),
            Param::XFilterFm => knob(&mut self.filter_fm),
            Param::XRing => knob(&mut self.ring),
            Param::Feed => knob(&mut self.feed_gain),
            Param::FeedTime | Param::FeedTone | Param::FeedLimit => {
                let path = self
                    .feed
                    .get_or_insert_with(|| Box::new(FeedPath::new(sample_rate)));
                match param {
                    Param::FeedTime => path.set_time(value, fresh),
                    Param::FeedTone => path.set_tone(value),
                    _ => path.set_limit(value),
                }
            }
            _ => {}
        }
    }

    /// The start of a sample: clear what the last one lent and brought,
    /// and move the depths one step toward their targets.
    #[inline]
    pub fn begin(&mut self, k: f32) {
        self.started = true;
        self.tap = (0.0, 0.0);
        self.pitch_ratio = 1.0;
        self.cutoff_oct = 0.0;
        self.feed_in = (0.0, 0.0);
        self.fm.step(k);
        self.filter_fm.step(k);
        self.ring.step(k);
        self.feed_gain.step(k);
    }

    /// Bring a route's source (its tap one sample ago, volts) to `dest`.
    #[inline]
    pub fn deliver(&mut self, dest: Dest, src: (f32, f32)) {
        let m = 0.5 * (src.0 + src.1) * TO_BUS;
        match dest {
            Dest::Fm => self.pitch_ratio = fm_ratio(self.fm.cur, m),
            Dest::FilterFm => self.cutoff_oct = self.filter_fm.cur * m,
            Dest::Ring => self.ring_m = Some(m),
            Dest::Feed => {
                if let Some(path) = self.feed.as_mut() {
                    let (l, r) = path.process((src.0 * TO_BUS, src.1 * TO_BUS), self.feed_gain.cur);
                    self.feed_in = (l / TO_BUS, r / TO_BUS);
                }
            }
        }
    }

    /// The strip's input stage (volts): the routed feed comes in, then the
    /// ring modulator multiplies voices and feed together.
    #[inline]
    pub fn input(&self, l: f32, r: f32) -> (f32, f32) {
        let (mut l, mut r) = (l + self.feed_in.0, r + self.feed_in.1);
        if let Some(m) = self.ring_m {
            let d = self.ring.cur;
            if d != 0.0 {
                let k = 1.0 - d + d * m;
                l *= k;
                r *= k;
            }
        }
        (l, r)
    }
}

/// Linear FM: the frequency ratio at depth `depth` and modulator `m`,
/// stopped at zero (the oscillator cores do not run backwards).
#[inline]
pub fn fm_ratio(depth: f32, m: f32) -> f32 {
    (1.0 + depth * m).max(0.0)
}

/// A routed feed's path, in bus units: delay, lowpass, DC blocker, gain,
/// then a peak limiter whose envelope attacks instantly, so nothing it
/// passes ever exceeds its ceiling.
#[derive(Clone)]
pub struct FeedPath {
    buf: Vec<(f32, f32)>,
    w: usize,
    delay: f32,
    cur_delay: f32,
    delay_k: f32,
    tone_bypass: bool,
    lp_k: f32,
    lp: (f32, f32),
    dc_pole: f32,
    dc_x: (f32, f32),
    dc_y: (f32, f32),
    ceiling: f32,
    env: f32,
    release: f32,
    sample_rate: f32,
}

impl FeedPath {
    pub fn new(sample_rate: f32) -> Self {
        let len = (MAX_FEED_S * sample_rate).ceil() as usize + 4;
        let mut p = Self {
            buf: vec![(0.0, 0.0); len],
            w: 0,
            delay: 0.0,
            cur_delay: 0.0,
            // a moved delay glides (a tape-like bend), never jumps
            delay_k: crate::smoothing::approach(0.05, sample_rate),
            tone_bypass: true,
            lp_k: 1.0,
            lp: (0.0, 0.0),
            // a 10 Hz blocker: no DC can build up around a loop
            dc_pole: 1.0 - std::f32::consts::TAU * 10.0 / sample_rate,
            dc_x: (0.0, 0.0),
            dc_y: (0.0, 0.0),
            ceiling: 0.5,
            env: 0.0,
            release: (-1.0 / (0.05 * sample_rate)).exp(),
            sample_rate,
        };
        p.set_limit(Param::FeedLimit.clamp(-6.0));
        p
    }

    /// The delay, seconds (0 = the one sample every route has).
    pub fn set_time(&mut self, seconds: f32, jump: bool) {
        let max = (self.buf.len() - 3) as f32;
        self.delay = (seconds * self.sample_rate).clamp(0.0, max);
        if jump {
            self.cur_delay = self.delay;
        }
    }

    /// The lowpass corner, Hz (20 kHz and up = out).
    pub fn set_tone(&mut self, hz: f32) {
        self.tone_bypass = hz >= 19_999.0;
        let hz = hz.clamp(1.0, 0.45 * self.sample_rate);
        self.lp_k = 1.0 - (-std::f32::consts::TAU * hz / self.sample_rate).exp();
    }

    /// The safety limiter's ceiling, dBFS.
    pub fn set_limit(&mut self, db: f32) {
        self.ceiling = 10f32.powf(db / 20.0);
    }

    pub fn ceiling(&self) -> f32 {
        self.ceiling
    }

    /// One stereo sample in (bus units), the routed signal out.
    #[inline]
    pub fn process(&mut self, x: (f32, f32), gain: f32) -> (f32, f32) {
        let n = self.buf.len();
        self.buf[self.w] = x;
        self.cur_delay += (self.delay - self.cur_delay) * self.delay_k;
        let pos = self.w as f32 - self.cur_delay;
        let base = pos.floor();
        let frac = pos - base;
        let i0 = (base as isize).rem_euclid(n as isize) as usize;
        let i1 = (i0 + 1) % n;
        let (a, b) = (self.buf[i0], self.buf[i1]);
        let (mut l, mut r) = (a.0 + (b.0 - a.0) * frac, a.1 + (b.1 - a.1) * frac);
        self.w = (self.w + 1) % n;
        if self.tone_bypass {
            self.lp = (l, r);
        } else {
            self.lp.0 += (l - self.lp.0) * self.lp_k;
            self.lp.1 += (r - self.lp.1) * self.lp_k;
            (l, r) = self.lp;
        }
        let yl = l - self.dc_x.0 + self.dc_pole * self.dc_y.0;
        let yr = r - self.dc_x.1 + self.dc_pole * self.dc_y.1;
        self.dc_x = (l, r);
        self.dc_y = (yl, yr);
        let (l, r) = (yl * gain, yr * gain);
        if !(l.is_finite() && r.is_finite()) {
            // a non-finite sample anywhere upstream: drop the loop's memory
            // rather than circulate it
            self.reset();
            return (0.0, 0.0);
        }
        let peak = l.abs().max(r.abs());
        self.env = peak.max(self.env * self.release);
        if self.env > self.ceiling {
            let g = self.ceiling / self.env;
            (l * g, r * g)
        } else {
            (l, r)
        }
    }

    fn reset(&mut self) {
        for s in self.buf.iter_mut() {
            *s = (0.0, 0.0);
        }
        self.lp = (0.0, 0.0);
        self.dc_x = (0.0, 0.0);
        self.dc_y = (0.0, 0.0);
        self.env = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_path_delays_by_the_set_time() {
        let sr = 48_000.0;
        let mut p = FeedPath::new(sr);
        p.set_time(0.01, true);
        let d = (0.01 * sr) as usize;
        let mut out = Vec::new();
        for i in 0..2 * d {
            let x = if i == 0 { 0.25 } else { 0.0 };
            out.push(p.process((x, x), 1.0).0);
        }
        // the impulse comes out d samples later, at its own size (the DC
        // blocker's 10 Hz corner takes a hair of it)
        let (at, peak) = out
            .iter()
            .enumerate()
            .fold((0, 0.0f32), |b, (i, &v)| if v.abs() > b.1.abs() { (i, v) } else { b });
        assert_eq!(at, d);
        assert!((peak - 0.25).abs() < 1e-3, "{peak}");
    }

    #[test]
    fn feed_limiter_never_passes_its_ceiling() {
        let sr = 48_000.0;
        let mut p = FeedPath::new(sr);
        p.set_limit(-6.0);
        let ceiling = p.ceiling();
        let mut max = 0.0f32;
        for i in 0..sr as usize {
            let x = 3.0 * (i as f32 * 0.05).sin();
            let (l, r) = p.process((x, -x), 2.0);
            max = max.max(l.abs()).max(r.abs());
        }
        assert!(max <= ceiling * 1.000_001, "{max} over {ceiling}");
        assert!(max > ceiling * 0.9, "the limiter should ride the ceiling, got {max}");
    }

    #[test]
    fn ring_depth_one_is_a_product_and_half_is_am() {
        let mut x = StripXmod::default();
        x.enable(Dest::Ring, 48_000.0);
        x.set(Param::XRing, 1.0, 48_000.0);
        x.begin(0.0);
        // a source tap whose bus level is 0.5
        let v = 0.5 / TO_BUS;
        x.deliver(Dest::Ring, (v, v));
        let (l, _) = x.input(2.0, 2.0);
        assert!((l - 1.0).abs() < 1e-6, "{l}");
        x.set(Param::XRing, 0.5, 48_000.0);
        let mut y = x.clone();
        y.ring.cur = 0.5;
        let (l, _) = y.input(2.0, 2.0);
        // 2 * (1 - 0.5 + 0.5 * 0.5) = 1.5
        assert!((l - 1.5).abs() < 1e-6, "{l}");
    }

    // ---- the engine: real tracks driving real tracks ----

    use crate::oscillator::{CircuitModel, Waveform};
    use crate::voice_manager::{ParamValues, VoiceManager};

    const SR: f32 = 48_000.0;
    const MODULATOR: u16 = 1;
    const CARRIER: u16 = 2;

    fn hz(note: u8) -> f64 {
        440.0 * 2f64.powf((note as f64 - 69.0) / 12.0)
    }

    /// One plain sine card: oscillator 1 alone, the ladder wide open, a
    /// held level.
    fn sine_patch() -> ParamValues {
        let mut p = ParamValues::neutral();
        p.waveform = Waveform::Sine;
        p.osc2_wave = Waveform::Sine;
        p.osc3_wave = Waveform::Sine;
        p.cutoff = 20_000.0;
        p.attack = 0.002;
        p.sustain = 1.0;
        p.circuit = CircuitModel::Arp;
        p
    }

    /// Two sine tracks, the modulator and the carrier, with these routes
    /// and knobs, both notes struck at once; returns each sample's taps
    /// (modulator, carrier), mono in bus units, after `skip` seconds.
    fn run(
        carrier: ParamValues,
        notes: (u8, u8),
        routes: &[Route],
        knobs: &[(u16, Param, f32)],
        skip: f32,
        secs: f32,
    ) -> (Vec<f32>, Vec<f32>) {
        let mut vm = VoiceManager::new(SR, 8);
        vm.warm_up();
        vm.set_channel_params(MODULATOR, sine_patch());
        vm.set_channel_params(CARRIER, carrier);
        // both strips tap, routed or not
        vm.add_route(Route { dest: Dest::Ring, src: MODULATOR, dst: MODULATOR });
        vm.add_route(Route { dest: Dest::Ring, src: CARRIER, dst: CARRIER });
        for &r in routes {
            vm.add_route(r);
        }
        for &(ch, p, v) in knobs {
            vm.set_channel_param(ch, p, v);
        }
        vm.note_on_channel(notes.0, 1.0, MODULATOR);
        vm.note_on_channel(notes.1, 1.0, CARRIER);
        let mono = |t: Option<(f32, f32)>| t.map_or(0.0, |(l, r)| 0.5 * (l + r) * TO_BUS);
        let (mut m, mut c) = (Vec::new(), Vec::new());
        for i in 0..((skip + secs) * SR) as usize {
            vm.render_next();
            if i >= (skip * SR) as usize {
                m.push(mono(vm.route_tap(MODULATOR)));
                c.push(mono(vm.route_tap(CARRIER)));
            }
        }
        (m, c)
    }

    /// Amplitude of the component at `f` Hz (Hann-windowed Goertzel).
    fn amp(x: &[f32], f: f64) -> f64 {
        let n = x.len();
        let w = std::f64::consts::TAU * f / SR as f64;
        let (mut re, mut im, mut wsum) = (0.0f64, 0.0f64, 0.0f64);
        for (i, &v) in x.iter().enumerate() {
            let win = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
            re += v as f64 * win * (w * i as f64).cos();
            im -= v as f64 * win * (w * i as f64).sin();
            wsum += win;
        }
        2.0 * (re * re + im * im).sqrt() / wsum
    }

    /// Where a component near `near` Hz really sits: the oscillators are
    /// analog (the warm chassis pulls them a little off their nominal
    /// pitch), and a window probed half a bin off a line reads it low.
    fn peak_freq(x: &[f32], near: f64) -> f64 {
        let mut best = (near, 0.0f64);
        let mut f = near - 4.0;
        while f <= near + 4.0 {
            let a = amp(x, f);
            if a > best.1 {
                best = (f, a);
            }
            f += 0.02;
        }
        let (f0, h) = (best.0, 0.02);
        let (a, b, c) = (amp(x, f0 - h).ln(), best.1.ln(), amp(x, f0 + h).ln());
        let den = a - 2.0 * b + c;
        if den.abs() > 1e-12 {
            f0 + 0.5 * h * (a - c) / den
        } else {
            f0
        }
    }

    /// Bessel function of the first kind, by its series.
    fn bessel_j(n: i32, x: f64) -> f64 {
        let n_abs = n.unsigned_abs() as i32;
        let mut sum = 0.0;
        let mut fact_m = 1.0;
        for m in 0..40 {
            if m > 0 {
                fact_m *= m as f64;
            }
            let fact_mn: f64 = (1..=(m + n_abs)).map(|k| k as f64).product();
            sum += (-1f64).powi(m) / (fact_m * fact_mn) * (x / 2.0).powi(2 * m + n_abs);
        }
        if n < 0 && n_abs % 2 == 1 {
            -sum
        } else {
            sum
        }
    }

    #[test]
    fn fm_from_another_track_puts_bessel_sidebands_at_carrier_plus_minus_k_mod() {
        // carrier A5 (880 Hz), modulator D3 (146.8 Hz): no line of the
        // carrier's own harmonics lands near its sidebands
        let notes = (50u8, 81u8);
        let route = [Route { dest: Dest::Fm, src: MODULATOR, dst: CARRIER }];
        // the modulator's level on the bus sets the index: measure it, and
        // where both oscillators really sit
        let (m, dry) = run(sine_patch(), notes, &route, &[], 0.25, 1.0);
        let (fm, fc) = (peak_freq(&m, hz(notes.0)), peak_freq(&dry, hz(notes.1)));
        let a_mod = amp(&m, fm);
        let target_beta = 1.2;
        let depth = (target_beta * fm / (a_mod * fc)) as f32;
        let (_, wet) = run(sine_patch(), notes, &route, &[(CARRIER, Param::XFm, depth)], 0.25, 1.0);
        let beta = depth as f64 * a_mod * fc / fm;
        let c0 = amp(&wet, fc);
        for k in [-4i32, -3, -2, -1, 1, 2, 3, 4] {
            let f = fc + k as f64 * fm;
            let ratio = amp(&wet, f) / c0;
            let want = (bessel_j(k, beta) / bessel_j(0, beta)).abs();
            // the outer lines also carry the modulator's own third
            // harmonic (a rounded-triangle sine, -40 dB)
            let tol = [0.0, 0.02, 0.03, 0.06, 0.06][k.unsigned_abs() as usize];
            assert!(
                (ratio - want).abs() <= tol * want,
                "sideband k={k} at {f:.1} Hz: {ratio:.4} of the carrier, Bessel says {want:.4} (beta {beta:.3})"
            );
            // the line is a line: half a modulator period off it, nothing
            let off = amp(&wet, f + 0.5 * fm) / c0;
            assert!(off < 0.01 * want.max(0.05), "k={k}: {off} between the lines");
            // and with the depth at 0 there is no sideband at all
            let none = amp(&dry, f) / amp(&dry, fc);
            assert!(none < 1e-3, "k={k} without FM: {none}");
        }
    }

    #[test]
    fn filter_fm_moves_the_cutoff_by_its_depth_in_octaves() {
        // a B6 sine (1976 Hz) three octaves over a 250 Hz ladder, and a
        // 33 Hz modulator, so the cutoff sweeps slowly enough to read
        let notes = (24u8, 95u8);
        let mut low = sine_patch();
        low.cutoff = 250.0;
        let route = [Route { dest: Dest::FilterFm, src: MODULATOR, dst: CARRIER }];
        let (m, _) = run(low, notes, &route, &[], 0.5, 0.5);
        let peak = m.iter().fold(0.0f32, |a, &v| a.max(v.abs()));
        // half an octave either way at the modulator's peaks
        let depth = 0.5 / peak;
        let (m, c) = run(low, notes, &route, &[(CARRIER, Param::XFilterFm, depth)], 0.5, 1.0);
        let fc = peak_freq(&c, hz(notes.1));
        // the carrier's line in 3 ms around each of the modulator's crests
        // (or troughs; the route reads the modulator one sample late), so
        // the modulator's own bleed into the card's filter stays out
        let around = |sign: f32| {
            let half = (0.0015 * SR) as usize;
            let mut acc = Vec::new();
            let mut i = half + 2;
            while i + half < c.len() {
                let v = m[i - 1] * sign;
                if v > 0.99 * peak && m[i - 2] * sign <= v && m[i] * sign <= v {
                    acc.push(amp(&c[i - half..i + half], fc));
                    i += half;
                }
                i += 1;
            }
            assert!(acc.len() >= 5, "only {} modulator extremes found", acc.len());
            acc.iter().sum::<f64>() / acc.len() as f64
        };
        let (up, down) = (around(1.0), around(-1.0));
        // the same card at a fixed cutoff half an octave up and down
        let fixed = |oct: f32| {
            let mut p = low;
            p.cutoff = 250.0 * oct.exp2();
            let c = run(p, notes, &[], &[], 0.5, 0.25).1;
            amp(&c, peak_freq(&c, hz(notes.1)))
        };
        let (want_up, want_down) = (fixed(0.5), fixed(-0.5));
        let db = |a: f64, b: f64| 20.0 * (a / b).log10();
        assert!(db(up, want_up).abs() < 0.5, "crest {:.2} dB off the fixed +0.5 oct", db(up, want_up));
        assert!(db(down, want_down).abs() < 0.5, "trough {:.2} dB off the fixed -0.5 oct", db(down, want_down));
        // an octave of sweep three octaves under the note moves it ~23 dB
        assert!(db(up, down) > 20.0, "crest to trough only {:.1} dB", db(up, down));
    }

    #[test]
    fn ringmod_multiplies_the_track_by_its_source() {
        let notes = (50u8, 81u8);
        let run_ring = |d: f32| ring_run(notes, d);
        let (m, dry) = run_ring(0.0);
        let (fm, fc) = (peak_freq(&m, hz(notes.0)), peak_freq(&dry, hz(notes.1)));
        let a_mod = amp(&m, fm);
        let a_car = amp(&dry, fc);
        let (_, ring) = run_ring(1.0);
        // full depth: the carrier is gone, f +- fm carry half the product
        assert!(amp(&ring, fc) < 0.01 * a_car * a_mod, "carrier leaks: {}", amp(&ring, fc));
        for f in [fc - fm, fc + fm] {
            let want = 0.5 * a_car * a_mod;
            let got = amp(&ring, f);
            assert!((got - want).abs() < 0.01 * want, "{f:.1} Hz: {got} vs {want}");
        }
        // half depth is AM: half the carrier stays
        let (_, am) = run_ring(0.5);
        assert!((amp(&am, fc) - 0.5 * a_car).abs() < 0.005 * a_car);
        let side = amp(&am, fc + fm);
        assert!((side - 0.25 * a_car * a_mod).abs() < 0.01 * 0.25 * a_car * a_mod);
    }

    /// The carrier ring-modulated by the modulator at depth `d`.
    fn ring_run(notes: (u8, u8), d: f32) -> (Vec<f32>, Vec<f32>) {
        let mut vm = VoiceManager::new(SR, 8);
        vm.warm_up();
        vm.set_channel_params(MODULATOR, sine_patch());
        vm.set_channel_params(CARRIER, sine_patch());
        vm.add_route(Route { dest: Dest::Ring, src: MODULATOR, dst: CARRIER });
        vm.set_channel_param(CARRIER, Param::XRing, d);
        vm.note_on_channel(notes.0, 1.0, MODULATOR);
        vm.note_on_channel(notes.1, 1.0, CARRIER);
        let mono = |t: Option<(f32, f32)>| t.map_or(0.0, |(l, r)| 0.5 * (l + r) * TO_BUS);
        let (mut m, mut c) = (Vec::new(), Vec::new());
        for i in 0..(1.25 * SR) as usize {
            vm.render_next();
            if i >= (0.25 * SR) as usize {
                m.push(mono(vm.route_tap(MODULATOR)));
                c.push(mono(vm.route_tap(CARRIER)));
            }
        }
        (m, c)
    }

    /// One track feeding itself: a 20 ms burst, then the loop alone.
    fn self_feed(gain: f32, time: f32, limit: f32, secs: f32) -> Vec<f32> {
        let mut vm = VoiceManager::new(SR, 8);
        vm.warm_up();
        let mut p = sine_patch();
        p.release = 0.001;
        vm.set_channel_params(CARRIER, p);
        vm.add_route(Route { dest: Dest::Feed, src: CARRIER, dst: CARRIER });
        vm.set_channel_param(CARRIER, Param::Feed, gain);
        vm.set_channel_param(CARRIER, Param::FeedTime, time);
        vm.set_channel_param(CARRIER, Param::FeedLimit, limit);
        let mut out = Vec::new();
        vm.note_on_channel(81, 1.0, CARRIER);
        for i in 0..(secs * SR) as usize {
            if i == (0.02 * SR) as usize {
                vm.note_off_channel(81, CARRIER);
            }
            vm.render_next();
            out.push(vm.route_tap(CARRIER).map_or(0.0, |(l, r)| 0.5 * (l + r) * TO_BUS));
        }
        out
    }

    #[test]
    fn feed_loop_echoes_at_its_delay_and_gain() {
        let (g, t) = (0.5f32, 0.1f32);
        let x = self_feed(g, t, 0.0, 0.5);
        // each pass around the loop takes the delay plus the route's one
        // sample, and comes back at the feed's gain
        let lap = (t * SR) as usize + 1;
        let burst = (0.03 * SR) as usize;
        let energy = |k: usize| x[k * lap..k * lap + burst].iter().map(|v| (v * v) as f64).sum::<f64>().sqrt();
        let (e0, e1, e2) = (energy(0), energy(1), energy(2));
        assert!((e1 / e0 - g as f64).abs() < 0.01, "first echo {:.4} of the burst", e1 / e0);
        assert!((e2 / e1 - g as f64).abs() < 0.01, "second echo {:.4} of the first", e2 / e1);
        // and nothing between the echoes
        let gap = x[burst + 200..lap - 200].iter().fold(0.0f32, |a, &v| a.max(v.abs()));
        assert!(gap < 1e-3 * x[..burst].iter().fold(0.0f32, |a, &v| a.max(v.abs())));
    }

    #[test]
    fn a_runaway_feed_loop_rings_at_the_limiter_ceiling_and_never_past_it() {
        let ceiling_db = -12.0f32;
        let ceiling = 10f32.powf(ceiling_db / 20.0);
        // loop gain 2: the burst grows on every lap until the limiter holds it
        let x = self_feed(2.0, 0.003, ceiling_db, 3.0);
        let tail = &x[(1.0 * SR) as usize..];
        let peak = tail.iter().fold(0.0f32, |a, &v| a.max(v.abs()));
        let rms = (tail.iter().map(|v| (v * v) as f64).sum::<f64>() / tail.len() as f64).sqrt();
        assert!(tail.iter().all(|v| v.is_finite()));
        // the card's own -60 dB floor rides on top of the limited feed
        assert!(peak <= ceiling * 1.01, "loop peak {peak} over the {ceiling} ceiling");
        assert!(rms > 0.3 * ceiling as f64, "the loop should keep ringing, rms {rms}");
    }

    #[test]
    fn two_tracks_feeding_each_other_stay_bounded() {
        let mut vm = VoiceManager::new(SR, 8);
        vm.warm_up();
        vm.set_channel_params(MODULATOR, sine_patch());
        vm.set_channel_params(CARRIER, sine_patch());
        vm.add_route(Route { dest: Dest::Feed, src: MODULATOR, dst: CARRIER });
        vm.add_route(Route { dest: Dest::Feed, src: CARRIER, dst: MODULATOR });
        vm.add_route(Route { dest: Dest::Fm, src: CARRIER, dst: MODULATOR });
        for ch in [MODULATOR, CARRIER] {
            vm.set_channel_param(ch, Param::Feed, 2.0);
            vm.set_channel_param(ch, Param::FeedTime, 0.0021);
            vm.set_channel_param(ch, Param::Dist, 0.6);
            vm.set_channel_param(ch, Param::EqMid, 18.0);
        }
        vm.set_channel_param(MODULATOR, Param::XFm, 8.0);
        vm.note_on_channel(45, 1.0, MODULATOR);
        vm.note_on_channel(52, 1.0, CARRIER);
        let mut peak = 0.0f32;
        for _ in 0..(2.0 * SR) as usize {
            let (l, r) = vm.render_next();
            assert!(l.is_finite() && r.is_finite());
            peak = peak.max(l.abs()).max(r.abs());
        }
        assert!(peak <= 1.0);
    }

    #[test]
    fn fm_ratio_is_linear_and_stops_at_zero() {
        assert_eq!(fm_ratio(0.0, 0.7), 1.0);
        assert!((fm_ratio(2.0, 0.25) - 1.5).abs() < 1e-6);
        assert_eq!(fm_ratio(4.0, -0.5), 0.0);
    }
}
