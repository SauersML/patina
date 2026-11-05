// src/glitch.rs
//
// Glitch FX: a beat repeat (stutter), a reverser and a tape stop, as one
// insert on a track's strip (`automate drums.stutter`) or across the mix
// bus (`automate stutter`). In signal order:
//
//   in ─► STUTTER ─► REVERSE ─► TAPE STOP ─► out
//
//   - stutter N   beat repeat: from the moment it engages, the next 1/N
//                 note is captured and repeated until it is set back to 0
//                 (N = 8 is an eighth note, 64 a 64th). The first pass is
//                 the live signal, so the repeat keeps the downbeat it was
//                 triggered on. Changing N while it runs changes the repeat
//                 length on the captured audio: a ramp from 8 to 64 is the
//                 classic roll into a drop.
//   - reverse N   plays the signal backwards in 1/N-note slices, one slice
//                 behind: each slice is the previous one reversed, so it
//                 starts sounding the instant it engages.
//   - tape_stop B the tape decelerates to a halt over B beats, at a
//                 constant rate like a deck losing power: pitch and level
//                 fall together (a playback head's voltage is proportional
//                 to tape speed), then silence until it is set back to 0,
//                 when the music comes back live.
//
// Slice and stop lengths follow the song's tempo (beats are quarter notes:
// a 1/N note is 4/N beats). Every seam, the repeat wrap, a reversed
// slice's edges, engaging and letting go, is declicked: slice edges with a
// short raised-cosine gate, switches with an equal-power crossfade.
//
// Each stage costs nothing until a song first touches it, then holds its
// own buffer; idle stages pass the signal through untouched, and an
// insert no song touched does not exist, so songs without glitches render
// bit-identically.

/// Longest slice a stutter or reverse can hold (a bar at 60 bpm).
const MAX_SLICE_S: f32 = 4.0;
/// Longest tape stop.
const MAX_STOP_S: f32 = 8.0;
/// The switch crossfade, live to effect and back.
const XFADE_S: f32 = 0.003;
/// The edge gate on every repeat and reversed slice (at most a quarter of
/// the slice, so a 1/64 at a fast tempo still sounds).
const EDGE_S: f32 = 0.0015;

/// Samples in a 1/`div` note at `spb` samples per beat.
fn slice_len(div: f32, spb: f64, cap: usize) -> usize {
    ((4.0 / div as f64) * spb).round().clamp(1.0, cap as f64) as usize
}

/// Raised-cosine edge gain for position `pos` of a `len`-sample slice.
#[inline]
fn edge_gain(pos: usize, len: usize, edge: usize, fade_in: bool) -> f32 {
    let edge = edge.min(len / 4).max(1);
    let x = if fade_in && pos < edge {
        pos as f32 / edge as f32
    } else if len - pos <= edge {
        (len - pos) as f32 / edge as f32
    } else {
        return 1.0;
    };
    0.5 - 0.5 * (std::f32::consts::PI * x).cos()
}

/// An equal-power crossfade between the stage's previous and new output.
#[derive(Clone, Copy, Default)]
struct Switch {
    left: usize,
    len: usize,
    /// true = fading the effect in, false = fading it out to the dry signal
    to_wet: bool,
}

impl Switch {
    fn start(&mut self, to_wet: bool, len: usize) {
        self.left = len;
        self.len = len;
        self.to_wet = to_wet;
    }

    fn running(&self) -> bool {
        self.left > 0
    }

    /// Mix `dry` and `wet` and step the fade.
    fn mix(&mut self, dry: [f32; 2], wet: [f32; 2]) -> [f32; 2] {
        let t = 1.0 - self.left as f32 / self.len.max(1) as f32;
        self.left -= 1;
        let a = if self.to_wet { t } else { 1.0 - t };
        let (gw, gd) = (
            (a * std::f32::consts::FRAC_PI_2).sin(),
            (a * std::f32::consts::FRAC_PI_2).cos(),
        );
        [dry[0] * gd + wet[0] * gw, dry[1] * gd + wet[1] * gw]
    }
}

/// The beat repeat.
#[derive(Clone)]
struct Stutter {
    div: f32,
    slice: Vec<[f32; 2]>,
    rec: usize,
    pos: usize,
    len: usize,
    /// Which pass over the slice (0 = the first, live).
    pass: u32,
    active: bool,
    out: Switch,
}

impl Stutter {
    fn new(sr: f32) -> Self {
        Self {
            div: 0.0,
            slice: vec![[0.0; 2]; (MAX_SLICE_S * sr) as usize],
            rec: 0,
            pos: 0,
            len: 1,
            pass: 0,
            active: false,
            out: Switch::default(),
        }
    }

    fn process(&mut self, x: [f32; 2], spb: f64, edge: usize, xf: usize) -> [f32; 2] {
        let on = self.div >= 1.0;
        if on && !self.active {
            self.active = true;
            self.rec = 0;
            self.pos = 0;
            self.pass = 0;
            self.out = Switch::default();
        } else if !on && self.active && !self.out.running() {
            self.out.start(false, xf);
        }
        if !self.active {
            return x;
        }
        let cap = self.slice.len();
        if self.rec < cap {
            self.slice[self.rec] = x;
            self.rec += 1;
        }
        if on {
            // a new length lands at once; after the first pass it can
            // only reach as far as the tape recorded
            let mut len = slice_len(self.div, spb, cap);
            if self.pass > 0 {
                len = len.min(self.rec);
            }
            self.len = len;
        }
        if self.pos >= self.len {
            self.pos = 0;
            self.pass += 1;
        }
        let src = if self.pass == 0 { x } else { self.slice[self.pos] };
        let g = edge_gain(self.pos, self.len, edge, self.pass > 0);
        let wet = [src[0] * g, src[1] * g];
        self.pos += 1;
        if self.out.running() {
            let y = self.out.mix(x, wet);
            if !self.out.running() {
                self.active = false;
            }
            return y;
        }
        wet
    }
}

/// The reverser: always recording once it exists, so the slice before it
/// engages is there to play backwards.
#[derive(Clone)]
struct Reverse {
    div: f32,
    hist: Vec<[f32; 2]>,
    /// Samples written so far (the history's write head, unwrapped).
    count: usize,
    /// Where the current slice started (in `count` units) and its length.
    seg_start: usize,
    len: usize,
    pos: usize,
    active: bool,
    sw: Switch,
}

impl Reverse {
    fn new(sr: f32) -> Self {
        Self {
            div: 0.0,
            hist: vec![[0.0; 2]; (2.0 * MAX_SLICE_S * sr) as usize + 2],
            count: 0,
            seg_start: 0,
            len: 1,
            pos: 0,
            active: false,
            sw: Switch::default(),
        }
    }

    fn process(&mut self, x: [f32; 2], spb: f64, edge: usize, xf: usize) -> [f32; 2] {
        let cap = self.hist.len();
        self.hist[self.count % cap] = x;
        self.count += 1;
        let on = self.div >= 1.0;
        if on && !self.active {
            self.active = true;
            self.seg_start = self.count - 1;
            self.len = slice_len(self.div, spb, (cap - 2) / 2);
            self.pos = 0;
            self.sw.start(true, xf);
        } else if !on && self.active && !(self.sw.running() && !self.sw.to_wet) {
            self.sw.start(false, xf);
        }
        if !self.active {
            return x;
        }
        if self.pos >= self.len {
            // the next slice: the one that just played in, backwards
            self.seg_start += self.len;
            self.pos = 0;
            if on {
                self.len = slice_len(self.div, spb, (cap - 2) / 2);
            }
        }
        // sample `pos` of this slice is the previous slice read from its
        // end: never further back than two slices, never unwritten (before
        // the first sample ever written, silence)
        let wet = match self.seg_start.checked_sub(1 + self.pos) {
            Some(at) => {
                let s = self.hist[at % cap];
                let g = edge_gain(self.pos, self.len, edge, true);
                [s[0] * g, s[1] * g]
            }
            None => [0.0; 2],
        };
        self.pos += 1;
        if self.sw.running() {
            let to_wet = self.sw.to_wet;
            let y = self.sw.mix(x, wet);
            if !self.sw.running() && !to_wet {
                self.active = false;
            }
            return y;
        }
        wet
    }
}

/// The tape stop.
#[derive(Clone)]
struct TapeStop {
    beats: f32,
    buf: Vec<[f32; 2]>,
    /// Samples written since it engaged, the read head, its speed and the
    /// speed it loses per sample.
    count: usize,
    read: f64,
    speed: f64,
    decel: f64,
    active: bool,
    out: Switch,
}

impl TapeStop {
    fn new(sr: f32) -> Self {
        Self {
            beats: 0.0,
            // the head falls behind by half the stop at most
            buf: vec![[0.0; 2]; (0.5 * MAX_STOP_S * sr) as usize + 8],
            count: 0,
            read: 0.0,
            speed: 1.0,
            decel: 0.0,
            active: false,
            out: Switch::default(),
        }
    }

    fn at(&self, i: isize) -> [f32; 2] {
        let cap = self.buf.len() as isize;
        let i = i.clamp(0, self.count as isize - 1);
        self.buf[(i % cap) as usize]
    }

    fn process(&mut self, x: [f32; 2], spb: f64, sr: f32, xf: usize) -> [f32; 2] {
        let on = self.beats > 0.0;
        if on && !self.active {
            self.active = true;
            self.count = 0;
            self.read = 0.0;
            self.speed = 1.0;
            let secs = (self.beats as f64 * spb / sr as f64).min(MAX_STOP_S as f64);
            self.decel = 1.0 / (secs * sr as f64).max(1.0);
            self.out = Switch::default();
        } else if !on && self.active && !self.out.running() {
            self.out.start(false, xf);
        }
        if !self.active {
            return x;
        }
        let cap = self.buf.len();
        self.buf[self.count % cap] = x;
        self.count += 1;
        // 4-point Hermite read at the slowing head; its level falls with
        // its speed, so the stop sinks into silence instead of a held DC
        let i = self.read.floor() as isize;
        let t = (self.read - i as f64) as f32;
        let (p0, p1, p2, p3) = (self.at(i - 1), self.at(i), self.at(i + 1), self.at(i + 2));
        let g = self.speed as f32;
        let mut wet = [0.0f32; 2];
        for c in 0..2 {
            let c0 = p1[c];
            let c1 = 0.5 * (p2[c] - p0[c]);
            let c2 = p0[c] - 2.5 * p1[c] + 2.0 * p2[c] - 0.5 * p3[c];
            let c3 = 0.5 * (p3[c] - p0[c]) + 1.5 * (p1[c] - p2[c]);
            wet[c] = (((c3 * t + c2) * t + c1) * t + c0) * g;
        }
        self.read += self.speed;
        self.speed = (self.speed - self.decel).max(0.0);
        if self.out.running() {
            let y = self.out.mix(x, wet);
            if !self.out.running() {
                self.active = false;
            }
            return y;
        }
        wet
    }
}

/// One glitch insert: the three stages in series.
#[derive(Clone)]
pub struct Glitch {
    sr: f32,
    /// Samples per beat: the song's tempo, which slice and stop lengths
    /// follow.
    spb: f64,
    stutter: Option<Box<Stutter>>,
    reverse: Option<Box<Reverse>>,
    tape: Option<Box<TapeStop>>,
}

impl Glitch {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate,
            spb: sample_rate as f64 * 0.5,
            stutter: None,
            reverse: None,
            tape: None,
        }
    }

    /// Beat repeat of 1/`div` notes (below 1 = off).
    pub fn set_stutter(&mut self, div: f32) {
        if !div.is_finite() {
            return;
        }
        let sr = self.sr;
        self.stutter.get_or_insert_with(|| Box::new(Stutter::new(sr))).div = div;
    }

    /// Reverse in 1/`div`-note slices (below 1 = off).
    pub fn set_reverse(&mut self, div: f32) {
        if !div.is_finite() {
            return;
        }
        let sr = self.sr;
        self.reverse.get_or_insert_with(|| Box::new(Reverse::new(sr))).div = div;
    }

    /// Stop the tape over `beats` (0 = running).
    pub fn set_tape_stop(&mut self, beats: f32) {
        if !beats.is_finite() {
            return;
        }
        let sr = self.sr;
        self.tape.get_or_insert_with(|| Box::new(TapeStop::new(sr))).beats = beats;
    }

    /// The song's tempo, in samples per beat (120 bpm until told).
    pub fn set_tempo(&mut self, samples_per_beat: f64) {
        if samples_per_beat.is_finite() && samples_per_beat > 0.0 {
            self.spb = samples_per_beat;
        }
    }

    /// One stereo sample.
    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let spb = self.spb;
        let edge = (EDGE_S * self.sr) as usize;
        let xf = ((XFADE_S * self.sr) as usize).max(1);
        let mut x = [l, r];
        if let Some(s) = self.stutter.as_mut() {
            x = s.process(x, spb, edge, xf);
        }
        if let Some(s) = self.reverse.as_mut() {
            x = s.process(x, spb, edge, xf);
        }
        if let Some(s) = self.tape.as_mut() {
            x = s.process(x, spb, self.sr, xf);
        }
        (x[0], x[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;
    /// 120 bpm.
    const SPB: f64 = 24000.0;

    /// A signal that never repeats: a sweep plus a slow ramp.
    fn signal(n: usize) -> f32 {
        let t = n as f32 / SR;
        (std::f32::consts::TAU * (200.0 + 300.0 * t) * t).sin() * 0.5 + 0.001 * t
    }

    /// Untouched, an insert passes every sample through exactly.
    #[test]
    fn idle_stages_pass_through() {
        let mut g = Glitch::new(SR);
        g.set_tempo(SPB);
        g.set_stutter(0.0);
        g.set_reverse(0.0);
        g.set_tape_stop(0.0);
        for n in 0..10000 {
            let x = signal(n);
            assert_eq!(g.process(x, -x), (x, -x));
        }
    }

    /// A 1/16 stutter at 120 bpm repeats a 6000-sample slice: past the
    /// first (live) pass the output is periodic in exactly 6000 samples,
    /// the slice is the audio from the moment it engaged, and letting go
    /// returns the live signal.
    #[test]
    fn a_stutter_repeats_a_sixteenth() {
        let mut g = Glitch::new(SR);
        g.set_tempo(SPB);
        let mut out = Vec::new();
        let on = 5000;
        for n in 0..40000 {
            if n == on {
                g.set_stutter(16.0);
            }
            if n == 32000 {
                g.set_stutter(0.0);
            }
            out.push(g.process(signal(n), signal(n)).0);
        }
        let len = 6000;
        let edge = (EDGE_S * SR) as usize;
        for k in (on + len + edge)..(on + 4 * len - edge) {
            let e = (k - on) % len;
            if e < edge || e >= len - edge {
                continue;
            }
            assert_eq!(out[k], out[k - len], "sample {k} breaks the repeat");
            assert_eq!(out[k], signal(on + e), "sample {k} is not the captured slice");
        }
        // the first pass is the live signal
        assert_eq!(out[on + 100], signal(on + 100));
        // after the release crossfade, live again
        assert_eq!(out[33000], signal(33000));
    }

    /// Changing the division while it runs shortens the repeat on the
    /// captured audio: the roll keeps its downbeat.
    #[test]
    fn a_roll_shortens_the_repeat() {
        let mut g = Glitch::new(SR);
        g.set_tempo(SPB);
        let on = 1000;
        let mut out = Vec::new();
        for n in 0..30000 {
            if n == on {
                g.set_stutter(8.0);
            }
            if n == on + 15000 {
                g.set_stutter(32.0);
            }
            out.push(g.process(signal(n), signal(n)).0);
        }
        // at 1/32 = 3000 samples, past the switch: periodic in 3000, and
        // each repeat starts with the slice's own first samples
        let edge = (EDGE_S * SR) as usize;
        for k in (on + 18000 + edge)..(on + 27000) {
            let e = (k - on - 15000) % 3000;
            if e < edge || e >= 3000 - edge {
                continue;
            }
            assert_eq!(out[k], out[k - 3000]);
            assert_eq!(out[k], signal(on + e));
        }
    }

    /// Reverse plays each 1/8 slice backwards, one slice behind: inside a
    /// slice the output walks back through the slice before it.
    #[test]
    fn reverse_plays_the_last_slice_backwards() {
        let mut g = Glitch::new(SR);
        g.set_tempo(SPB);
        g.set_reverse(0.0);
        let on = 20000;
        let mut out = Vec::new();
        for n in 0..60000 {
            if n == on {
                g.set_reverse(8.0);
            }
            out.push(g.process(signal(n), signal(n)).0);
        }
        let len = 12000; // an eighth at 120 bpm
        let edge = (EDGE_S * SR) as usize;
        let xf = (XFADE_S * SR) as usize;
        for k in (on + xf)..(on + 3 * len) {
            let e = (k - on) % len;
            if e < edge || e >= len - edge {
                continue;
            }
            let seg_start = on + (k - on) / len * len;
            assert_eq!(out[k], signal(seg_start - 1 - e), "sample {k}");
        }
    }

    /// The tape stop: a 1 kHz tone slowing to a halt over 2 beats (1 s at
    /// 120 bpm). Its frequency falls along the speed line 1 - t/T, its
    /// level with it, then silence; letting go brings the live tone back.
    #[test]
    fn the_tape_stops_along_a_straight_line() {
        let mut g = Glitch::new(SR);
        g.set_tempo(SPB);
        let tone = |n: usize| (std::f32::consts::TAU * 1000.0 * n as f32 / SR).sin();
        let on = 4800;
        let mut out = Vec::new();
        for n in 0..(on + 72000) {
            if n == on {
                g.set_tape_stop(2.0);
            }
            if n == on + 60000 {
                g.set_tape_stop(0.0);
            }
            out.push(g.process(tone(n), tone(n)).0);
        }
        // zero crossings in 50 ms windows: frequency follows 1000 (1 - t/T)
        for (t, want) in [(0.1f32, 900.0f32), (0.4, 600.0), (0.7, 300.0)] {
            let a = on + (t * SR) as usize - 1200;
            let w = &out[a..a + 2400];
            let ups = w.windows(2).filter(|p| p[0] <= 0.0 && p[1] > 0.0).count();
            let hz = ups as f32 / 0.05;
            assert!((hz - want).abs() < 0.06 * want, "at {t} s: {hz} Hz, want {want}");
        }
        // stopped: silence from the end of the stop to the release
        let stopped = &out[on + 48500..on + 59000];
        assert!(stopped.iter().all(|s| s.abs() < 1e-6), "a stopped tape is silent");
        // released: live again after the crossfade
        let k = on + 61000;
        assert_eq!(out[k], tone(k));
    }
}
