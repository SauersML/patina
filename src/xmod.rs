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
//                           loop with gain above 1 rings up to just under
//                           the limiter's ceiling and stays there, never
//                           past it.
//
// Sound: FM'd cards run their oscillators 4x oversampled (voice.rs: the
// folded lines of a sawtooth swept 0-2x sit about 65 dB under the true
// ones, against about 40 at the base rate); the ring modulator multiplies
// at 2x through half-band filters (a product of two tones under 24 kHz
// stays under 48 kHz, so only sums in the filter's 24-29 kHz transition
// come back, above 19 kHz). The feed's limiter is a
// saturator, as a real feedback loop's amplifier is: unity below its knee
// (-4.4 dB under the ceiling), a smooth tanh shoulder above it, run at 2x
// so the harmonics a saturated loop makes do not fold and recirculate. A
// loop with gain over 1 settles into a steady tone in the shoulder. A gain
// rider ahead of it (instant-attack envelope, gliding gain) keeps the
// drive at the ceiling at most however high the feed gain (so a loop is
// rounded, not squared: put a drive on the strip for that), and a last soft
// guard at the base rate takes the decimator's ringing, so nothing passes
// the ceiling. Every depth, the tone and the ceiling glide (20 ms); the
// delay glides like tape.
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
    /// An FM route drives this strip: its cards run their oscillators
    /// oversampled (voice.rs), so the sidebands do not alias.
    pub fm_routed: bool,
    /// This sample's filter FM, octaves.
    pub cutoff_oct: f32,
    /// This sample's ring modulator (bus units); None when not routed.
    ring_m: Option<f32>,
    /// The ring modulator's 2x oversampling (built with its route).
    ring_os: Option<Box<RingOs>>,
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
            fm_routed: false,
            cutoff_oct: 0.0,
            ring_m: None,
            ring_os: None,
            feed_in: (0.0, 0.0),
            feed: None,
        }
    }
}

impl StripXmod {
    /// Open this strip to a route of kind `dest`.
    pub fn enable(&mut self, dest: Dest, sample_rate: f32) {
        match dest {
            Dest::Ring => {
                self.ring_m = Some(self.ring_m.unwrap_or(0.0));
                if self.ring_os.is_none() {
                    self.ring_os = Some(Box::new(RingOs::new()));
                }
            }
            Dest::Feed => {
                if self.feed.is_none() {
                    self.feed = Some(Box::new(FeedPath::new(sample_rate)));
                }
            }
            Dest::Fm => self.fm_routed = true,
            Dest::FilterFm => {}
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
                    Param::FeedTone => path.set_tone(value, fresh),
                    _ => path.set_limit(value, fresh),
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
    /// ring modulator multiplies voices and feed together, at twice the
    /// sample rate (a product's sum frequencies reach twice the highest
    /// input, so at the base rate they would fold back). A ring-routed
    /// strip always runs the oversampler, depth 0 included, so its small
    /// fixed latency never jumps.
    #[inline]
    pub fn input(&mut self, l: f32, r: f32) -> (f32, f32) {
        let (l, r) = (l + self.feed_in.0, r + self.feed_in.1);
        match (self.ring_m, self.ring_os.as_mut()) {
            (Some(m), Some(os)) => os.process(l, r, m, self.ring.cur),
            (Some(m), None) => {
                let k = 1.0 - self.ring.cur + self.ring.cur * m;
                (l * k, r * k)
            }
            _ => (l, r),
        }
    }
}

/// The ring modulator at 2x: signal and modulator up through half-band
/// interpolators, multiplied, and back down through half-band decimators
/// (69 dB stopband), so the sum frequencies above Nyquist are filtered,
/// not folded.
#[derive(Clone)]
struct RingOs {
    up: [crate::tape::Halfband; 3],
    down: [crate::tape::Halfband; 2],
    /// Last sample's depth, so it moves in a line across the two steps.
    d: f32,
}

impl RingOs {
    fn new() -> Self {
        let hb = crate::tape::Halfband::new;
        Self {
            up: [hb(), hb(), hb()],
            down: [hb(), hb()],
            d: f32::NAN,
        }
    }

    #[inline]
    fn process(&mut self, l: f32, r: f32, m: f32, d: f32) -> (f32, f32) {
        let d0 = if self.d.is_finite() { self.d } else { d };
        self.d = d;
        let (l0, l1) = self.up[0].up(l);
        let (r0, r1) = self.up[1].up(r);
        let (m0, m1) = self.up[2].up(m);
        let dm = 0.5 * (d0 + d);
        let k0 = 1.0 - dm + dm * m0;
        let k1 = 1.0 - d + d * m1;
        (self.down[0].down(l0 * k0, l1 * k1), self.down[1].down(r0 * k0, r1 * k1))
    }
}

/// Linear FM: the frequency ratio at depth `depth` and modulator `m`,
/// stopped at zero (the oscillator cores do not run backwards).
#[inline]
pub fn fm_ratio(depth: f32, m: f32) -> f32 {
    (1.0 + depth * m).max(0.0)
}

/// The saturator's knee as a fraction of the ceiling (-4.4 dB): the feed
/// is untouched below it.
pub const FEED_KNEE: f32 = 0.6;
/// Where the oversampled shoulder tops out, of the ceiling; the base-rate
/// guard spans from here to the ceiling.
const FEED_SHOULDER: f32 = 0.92;
/// The rider keeps the drive into the shoulder at most this many times
/// the ceiling.
const FEED_DRIVE_MAX: f32 = 1.0;
/// The 2x half-band pair's delay below 4 kHz, base samples: the delay line
/// reads this much sooner, so a loop's lap stays the set time.
const FEED_OS_DELAY: f32 = 1.796;

/// A routed feed's path, in bus units: delay, lowpass, DC blocker, gain,
/// then the safety limiter: a gain rider, a 2x oversampled soft
/// saturator and a soft guard at the ceiling, which nothing passes.
#[derive(Clone)]
pub struct FeedPath {
    buf: Vec<(f32, f32)>,
    w: usize,
    delay: f32,
    cur_delay: f32,
    delay_k: f32,
    /// The lowpass coefficient, gliding to `lp_target` (1 = out).
    lp_k: f32,
    lp_target: f32,
    lp: (f32, f32),
    dc_pole: f32,
    dc_x: (f32, f32),
    dc_y: (f32, f32),
    /// The ceiling (linear), gliding to `ceiling_target`.
    ceiling: f32,
    ceiling_target: f32,
    knob_k: f32,
    env: f32,
    release: f32,
    gain: f32,
    gain_k: f32,
    up: [crate::tape::Halfband; 2],
    down: [crate::tape::Halfband; 2],
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
            lp_k: 1.0,
            lp_target: 1.0,
            lp: (0.0, 0.0),
            // a 10 Hz blocker: no DC can build up around a loop
            dc_pole: 1.0 - std::f32::consts::TAU * 10.0 / sample_rate,
            dc_x: (0.0, 0.0),
            dc_y: (0.0, 0.0),
            ceiling: 0.5,
            ceiling_target: 0.5,
            knob_k: crate::smoothing::approach(crate::smoothing::KNOB_SMOOTH_S, sample_rate),
            env: 0.0,
            release: (-1.0 / (0.05 * sample_rate)).exp(),
            gain: 1.0,
            gain_k: crate::smoothing::approach(0.0007, sample_rate),
            up: [crate::tape::Halfband::new(); 2],
            down: [crate::tape::Halfband::new(); 2],
            sample_rate,
        };
        p.set_limit(Param::FeedLimit.clamp(-6.0), true);
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

    /// The lowpass corner, Hz (20 kHz and up = out); it glides unless
    /// `jump` (set before the first sample).
    pub fn set_tone(&mut self, hz: f32, jump: bool) {
        self.lp_target = if hz >= 19_999.0 {
            1.0
        } else {
            let hz = hz.clamp(1.0, 0.45 * self.sample_rate);
            1.0 - (-std::f32::consts::TAU * hz / self.sample_rate).exp()
        };
        if jump {
            self.lp_k = self.lp_target;
        }
    }

    /// The safety limiter's ceiling, dBFS; it glides unless `jump`.
    pub fn set_limit(&mut self, db: f32, jump: bool) {
        self.ceiling_target = 10f32.powf(db / 20.0);
        if jump {
            self.ceiling = self.ceiling_target;
        }
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
        let pos = self.w as f32 - (self.cur_delay - FEED_OS_DELAY).max(0.0);
        let base = pos.floor();
        let frac = pos - base;
        let i0 = (base as isize).rem_euclid(n as isize) as usize;
        let i1 = (i0 + 1) % n;
        let (a, b) = (self.buf[i0], self.buf[i1]);
        let (mut l, mut r) = (a.0 + (b.0 - a.0) * frac, a.1 + (b.1 - a.1) * frac);
        self.w = (self.w + 1) % n;
        self.lp_k += (self.lp_target - self.lp_k) * self.knob_k;
        if self.lp_k >= 0.999_999 {
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
        self.ceiling += (self.ceiling_target - self.ceiling) * self.knob_k;
        let c = self.ceiling;
        // The rider: an instant-attack peak envelope sets the gain that
        // keeps the drive within FEED_DRIVE_MAX of the ceiling, and the
        // gain glides there (no per-sample jumps to buzz)
        let peak = l.abs().max(r.abs());
        self.env = peak.max(self.env * self.release);
        let most = FEED_DRIVE_MAX * c;
        let target = if self.env > most { most / self.env } else { 1.0 };
        self.gain += (target - self.gain) * self.gain_k;
        let g = self.gain.min(1.0);
        // The saturator at 2x
        let (knee, top) = (FEED_KNEE * c, FEED_SHOULDER * c);
        let (l0, l1) = self.up[0].up(l * g);
        let (r0, r1) = self.up[1].up(r * g);
        let sat = |x: f32| soft_ceiling(x, knee, top);
        let l = self.down[0].down(sat(l0), sat(l1));
        let r = self.down[1].down(sat(r0), sat(r1));
        // The guard: the decimator rings a little past what it was fed
        (soft_ceiling(l, top, c), soft_ceiling(r, top, c))
    }

    fn reset(&mut self) {
        for s in self.buf.iter_mut() {
            *s = (0.0, 0.0);
        }
        self.lp = (0.0, 0.0);
        self.dc_x = (0.0, 0.0);
        self.dc_y = (0.0, 0.0);
        self.env = 0.0;
        self.gain = 1.0;
        self.up = [crate::tape::Halfband::new(); 2];
        self.down = [crate::tape::Halfband::new(); 2];
    }
}

/// Unity up to `knee`, then a tanh shoulder that meets `ceiling` only at
/// infinity: smooth (its slope is continuous at the knee), and nothing it
/// passes reaches the ceiling.
#[inline]
pub fn soft_ceiling(x: f32, knee: f32, ceiling: f32) -> f32 {
    let a = x.abs();
    if a <= knee {
        return x;
    }
    let w = (ceiling - knee).max(1e-9);
    (knee + w * ((a - knee) / w).tanh()).copysign(x)
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
        // a 1 kHz burst under a Hann window, well below the knee
        let n = 480;
        let burst: Vec<f32> = (0..n)
            .map(|i| {
                let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
                0.25 * w * (std::f32::consts::TAU * 1000.0 * i as f32 / sr).sin()
            })
            .collect();
        let out: Vec<f32> = (0..3 * d)
            .map(|i| {
                let x = burst.get(i).copied().unwrap_or(0.0);
                p.process((x, x), 1.0).0
            })
            .collect();
        // it comes out d samples later (the oversampled saturator's own
        // delay is taken off the line), at its own size
        let corr = |lag: usize| -> f64 {
            burst.iter().enumerate().map(|(i, &b)| (b * out[i + lag]) as f64).sum()
        };
        let best = (d - 5..d + 6).max_by(|&a, &b| corr(a).partial_cmp(&corr(b)).unwrap()).unwrap();
        assert_eq!(best, d);
        let e = |v: &[f32]| v.iter().map(|x| (x * x) as f64).sum::<f64>().sqrt();
        let ratio = e(&out) / e(&burst);
        assert!((ratio - 1.0).abs() < 2e-3, "came out at {ratio} of its size");
    }

    #[test]
    fn feed_limiter_never_passes_its_ceiling() {
        let sr = 48_000.0;
        let mut p = FeedPath::new(sr);
        p.set_limit(-6.0, true);
        let ceiling = p.ceiling();
        let mut max = 0.0f32;
        for i in 0..sr as usize {
            let x = 3.0 * (i as f32 * 0.05).sin();
            let (l, r) = p.process((x, -x), 2.0);
            max = max.max(l.abs()).max(r.abs());
        }
        assert!(max <= ceiling * 1.000_001, "{max} over {ceiling}");
        // driven hard, it sits in its shoulder, about a dB under the ceiling
        assert!(max > ceiling * 0.8, "the limiter should ride the ceiling, got {max}");
    }

    #[test]
    fn ring_depth_one_is_a_product_and_half_is_am() {
        let mut x = StripXmod::default();
        x.enable(Dest::Ring, 48_000.0);
        x.set(Param::XRing, 1.0, 48_000.0);
        // a source tap whose bus level is 0.5, against a steady 2 V (the
        // half-band filters settle within a few dozen samples)
        let v = 0.5 / TO_BUS;
        let mut steady = |x: &mut StripXmod| {
            let mut l = 0.0;
            for _ in 0..200 {
                x.begin(0.0);
                x.deliver(Dest::Ring, (v, v));
                l = x.input(2.0, 2.0).0;
            }
            l
        };
        let l = steady(&mut x);
        assert!((l - 1.0).abs() < 1e-5, "{l}");
        let mut y = x.clone();
        y.ring.cur = 0.5;
        y.ring.target = 0.5;
        let l = steady(&mut y);
        // 2 * (1 - 0.5 + 0.5 * 0.5) = 1.5
        assert!((l - 1.5).abs() < 1e-5, "{l}");
    }

    /// Windowed (Hann) power of `x` at `f` Hz.
    fn power_at(x: &[f32], f: f64, sr: f64) -> f64 {
        let w = std::f64::consts::TAU * f / sr;
        let (c, mut s1, mut s2) = (2.0 * w.cos(), 0.0f64, 0.0f64);
        for (i, &v) in x.iter().enumerate() {
            let hann = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / x.len() as f64).cos();
            let s0 = v as f64 * hann + c * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        s1 * s1 + s2 * s2 - c * s1 * s2
    }

    /// 17 kHz times 16 kHz is 1 kHz and 33 kHz; at 48 kHz the 33 kHz line
    /// would fold to 15 kHz at full size. Multiplied at 2x it is filtered.
    /// (Products of two tones under 24 kHz reach at most 48 kHz, so 2x is
    /// enough: only sums between 24 and 28.8 kHz, in the half-band's
    /// transition, come back, and they land above 19 kHz.)
    #[test]
    fn the_ring_modulator_filters_its_sum_tone_instead_of_folding_it() {
        let sr = 48_000.0f64;
        let mut x = StripXmod::default();
        x.enable(Dest::Ring, sr as f32);
        x.set(Param::XRing, 1.0, sr as f32);
        let (mut out, mut naive) = (Vec::new(), Vec::new());
        for i in 0..9600 {
            let t = i as f64 / sr;
            let a = (std::f64::consts::TAU * 17_000.0 * t).sin() as f32;
            let m = (std::f64::consts::TAU * 16_000.0 * t).sin() as f32;
            x.begin(0.0);
            x.deliver(Dest::Ring, (m / TO_BUS, m / TO_BUS));
            let (l, _) = x.input(a, a);
            if i >= 4800 {
                out.push(l);
                naive.push(a * m);
            }
        }
        let rel = |v: &[f32]| 10.0 * (power_at(v, 15_000.0, sr) / power_at(v, 1_000.0, sr)).log10();
        let (os, raw) = (rel(&out), rel(&naive));
        assert!(raw > -1.0, "the plain product folds its sum to 15 kHz at full size: {raw:.1} dB");
        assert!(os < -60.0, "the folded sum tone at {os:.1} dB under the difference tone");
    }

    /// Below the knee the feed is untouched; driven to the ceiling, the
    /// saturator's harmonics are made at 2x, so those between 24 and 72
    /// kHz are filtered instead of folding among the real ones.
    #[test]
    fn the_feed_saturator_is_clean_below_its_knee_and_does_not_alias_above_it() {
        let sr = 48_000.0f64;
        let tone = |amp: f32, f: f64| -> Vec<f32> {
            let mut p = FeedPath::new(sr as f32);
            p.set_limit(-6.0, true);
            let c = p.ceiling();
            (0..19_200)
                .map(|i| {
                    let x = amp * c * (std::f64::consts::TAU * f * i as f64 / sr).sin() as f32;
                    p.process((x, x), 1.0).0
                })
                .skip(9600)
                .collect()
        };
        // half the ceiling, under the knee: no harmonics at all
        let quiet = tone(0.5, 1000.0);
        let fund = power_at(&quiet, 1000.0, sr);
        for k in 2..=5 {
            let h = 10.0 * (power_at(&quiet, 1000.0 * k as f64, sr) / fund).log10();
            assert!(h < -100.0, "harmonic {k} at {h:.1} dB under the knee");
        }
        // driven to the ceiling at 7 kHz: the 5th, 7th and 9th harmonics
        // (35, 49, 63 kHz) would fold to 13, 1 and 15 kHz, none of them a
        // harmonic of 7 kHz. Against the same curve run at 48 kHz:
        let hot = tone(1.0, 7000.0);
        let c = 10f32.powf(-6.0 / 20.0);
        let plain: Vec<f32> = (9600..19_200)
            .map(|i| {
                let x = c * (std::f64::consts::TAU * 7000.0 * i as f64 / sr).sin() as f32;
                soft_ceiling(x, FEED_KNEE * c, FEED_SHOULDER * c)
            })
            .collect();
        let folded = |v: &[f32]| {
            let fund = power_at(v, 7000.0, sr);
            let a: f64 = [13_000.0, 1_000.0, 15_000.0].iter().map(|&f| power_at(v, f, sr)).sum();
            10.0 * (a / fund).log10()
        };
        let third = 10.0 * (power_at(&hot, 21_000.0, sr) / power_at(&hot, 7000.0, sr)).log10();
        assert!(third > -60.0, "the shoulder should make real harmonics: {third:.1} dB");
        let (os, base) = (folded(&hot), folded(&plain));
        assert!(os < -70.0, "folded harmonics at {os:.1} dB");
        assert!(base > os + 20.0, "a 48 kHz saturator folds far more: {base:.1} vs {os:.1} dB");
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

    /// A track another track FMs plays on oversampled cards; the
    /// modulator's own card stays at the base rate.
    #[test]
    fn an_fm_routed_track_plays_on_oversampled_cards() {
        let mut vm = VoiceManager::new(SR, 8);
        vm.warm_up();
        vm.set_channel_params(MODULATOR, sine_patch());
        vm.set_channel_params(CARRIER, sine_patch());
        vm.add_route(Route { dest: Dest::Fm, src: MODULATOR, dst: CARRIER });
        vm.note_on_channel(50, 1.0, MODULATOR);
        vm.note_on_channel(81, 1.0, CARRIER);
        for _ in 0..64 {
            vm.render_next();
        }
        let playing: Vec<(u16, bool)> = vm
            .voices
            .iter()
            .filter(|v| v.is_active())
            .map(|v| (v.channel(), v.fm_oversampled()))
            .collect();
        assert_eq!(playing.len(), 2, "{playing:?}");
        for (ch, os) in playing {
            assert_eq!(os, ch == CARRIER, "channel {ch}");
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
