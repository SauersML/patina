// src/binaural.rs
//
// Binaural placement: a track's strip puts its sound at a point around a
// listener's head (`automate bird.azimuth`), in place of the pan. Nothing
// is downloaded: the head-related transfer function comes from a
// structural model, Brown and Duda's (1998) spherical head with pinna
// echoes, plus a shoulder reflection, and the sound travels to each ear
// along a real path through the air, so time differences and Doppler fall
// out of the geometry.
//
//                     ┌─► left path ─► head shadow ─► pinna + shoulder ─► air ─► 1/d ─► L
//   (l + r)/2 ─► air ─┤
//     delay line      └─► right path ─► (the same, the right ear's own) ─────────────► R
//
// Geometry: the head is a sphere of radius 8.75 cm at the origin, ears at
// (±a, 0, 0); x is right, y ahead, z up. The source sits at azimuth θ
// (degrees: 0 ahead, 90 right, -90 left, 180 behind), elevation φ and
// distance r, plus a cartesian offset (pos_x, pos_y, pos_z, metres) for
// straight-line paths, and `orbit` turns the azimuth continuously (degrees
// a second). For each ear:
//   - path: a straight line where the ear can see the source; where the
//     head is in the way, the tangent line plus the arc around the sphere
//     (Woodworth). At 90 degrees and 2 m the far ear's path is 0.227 m
//     longer: an interaural time difference of 0.66 ms, as measured on
//     real heads. The read point moves with the path, so a moving source
//     is Doppler shifted (a source closing at 20 m/s rises ~6%).
//   - head shadow: Brown-Duda's one-pole one-zero, corner c/a (1.25 kHz
//     at the bilinear transform's 2c/a), its zero set by the angle θe
//     between the source and the ear's axis: α = 1.05 + 0.95 cos(1.2 θe),
//     +6 dB of treble at the ear the source faces, -11 dB at 180 degrees,
//     so level differences grow with frequency as they do on a head.
//   - pinna: five echoes (gains 0.5, -1, 0.5, -0.25, 0.25) at delays
//     A cos(θp/2) sin(D(90° - φ)) + B samples at 44.1 kHz (A = 1, 5, 5, 5, 5;
//     B = 2, 4, 7, 11, 13; D = 1, 0.5, 0.5, 0.5, 0.5), θp the azimuth seen
//     from that ear: the notches that move with elevation. The shoulder
//     adds one echo (gain 0.3) 0.2..1.2 ms late, by elevation. Sources
//     behind lose up to 5 dB above 3 kHz to the pinna's flange.
//   - air: 1/d from a 1 m reference (an ear 0.25 m away at most +12 dB),
//     and ISO 9613-style absorption: 0.15 dB a metre at 10 kHz, as a
//     one-pole whose cutoff falls with distance.
//
// `doppler` (0..1, default 1) scales the travel time: 1 is physical (a
// source 10 m out is heard 29 ms after it plays, and moving it bends its
// pitch), 0 keeps only the head's own time differences.
//
// The sends (reverb, spring, echo, chorus) hear the source at a constant
// level, delayed by its travel time but not attenuated: a room fills the
// same however far away the source stands, so pulling a source back
// changes the direct-to-room balance, the strongest distance cue there is.

/// Head radius (m) and the speed of sound (m/s).
const HEAD_A: f64 = 0.0875;
const SOUND_C: f64 = 343.0;
/// Nearest a source may come to the head's centre (m).
const MIN_DIST: f64 = 0.15;
/// The travel delay line: 100 m at 48 kHz is 14,000 samples.
const LINE: usize = 1 << 16;
/// Each ear's short line for its pinna and shoulder echoes.
const EAR_LINE: usize = 256;
/// Smoothing time constant of every position control (two stages), s.
const POS_TAU_S: f32 = 0.025;

/// Brown-Duda pinna echoes: gain, A, B (samples at 44.1 kHz), D.
const PINNA: [(f32, f32, f32, f32); 5] = [
    (0.5, 1.0, 2.0, 1.0),
    (-1.0, 5.0, 4.0, 0.5),
    (0.5, 5.0, 7.0, 0.5),
    (-0.25, 5.0, 11.0, 0.5),
    (0.25, 5.0, 13.0, 0.5),
];

/// The placement's controls, by song-file name (see `Binaural::set`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Knob {
    Azimuth,
    Elevation,
    Distance,
    Orbit,
    PosX,
    PosY,
    PosZ,
    Doppler,
}

/// A position control's two-stage smoother.
#[derive(Clone, Copy)]
struct Glide {
    target: f32,
    a: f32,
    b: f32,
}

impl Glide {
    fn new(v: f32) -> Self {
        Self { target: v, a: v, b: v }
    }
    #[inline]
    fn step(&mut self, k: f32) -> f32 {
        self.a += (self.target - self.a) * k;
        self.b += (self.a - self.b) * k;
        self.b
    }
    fn snap(&mut self) {
        self.a = self.target;
        self.b = self.target;
    }
}

/// One ear's filters.
#[derive(Clone)]
struct Ear {
    /// Head shadow one-pole one-zero state.
    x1: f32,
    y1: f32,
    /// After the shadow: the pinna's and shoulder's taps read here.
    line: Vec<f32>,
    rear_lp: f32,
    air_lp: f32,
}

impl Ear {
    fn new() -> Self {
        Self {
            x1: 0.0,
            y1: 0.0,
            line: vec![0.0; EAR_LINE],
            rear_lp: 0.0,
            air_lp: 0.0,
        }
    }
}

/// Where one ear hears the source from, this sample.
struct EarView {
    /// Path length, m.
    path: f64,
    /// Angle between the source direction and the ear's outward axis, rad.
    theta_e: f64,
    /// The source's azimuth as seen from this ear's side, degrees
    /// (+ toward this ear), and its elevation, degrees.
    az_deg: f32,
    el_deg: f32,
}

/// Each ear's view of a source at `p` (head-centred, metres).
fn ear_view(p: [f64; 3], side: f64) -> EarView {
    let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(MIN_DIST);
    let cos_e = (p[0] * side / r).clamp(-1.0, 1.0);
    let theta_e = cos_e.acos();
    // Woodworth: straight where the ear sees the source, else the tangent
    // and the arc around the sphere
    let theta_t = (HEAD_A / r).clamp(-1.0, 1.0).acos();
    let path = if theta_e < theta_t {
        (r * r + HEAD_A * HEAD_A - 2.0 * HEAD_A * r * cos_e).max(0.0).sqrt()
    } else {
        (r * r - HEAD_A * HEAD_A).max(0.0).sqrt() + HEAD_A * (theta_e - theta_t)
    };
    let horiz = (p[0] * p[0] + p[1] * p[1]).sqrt();
    let az = (p[0] * side).atan2(p[1]).to_degrees() as f32;
    let el = p[2].atan2(horiz).to_degrees() as f32;
    EarView {
        path,
        theta_e,
        az_deg: az,
        el_deg: el,
    }
}

#[derive(Clone)]
pub struct Binaural {
    sr: f32,
    line: Vec<f32>,
    w: usize,
    ears: [Ear; 2],
    az: Glide,
    el: Glide,
    dist: Glide,
    px: Glide,
    py: Glide,
    pz: Glide,
    doppler: Glide,
    orbit: f32,
    /// Degrees the orbit has turned so far.
    spun: f64,
    k: f32,
    /// No sample yet: a control lands where it is set.
    fresh: bool,
    /// The travel delays (samples) last sample, so a jump is slewed.
    last_delay: [f64; 3],
    /// Each path's travel time (samples) at every sample in the line, so a
    /// sound is read from where the source was when it left (left ear,
    /// right ear, the room's centre).
    hist: [Vec<f32>; 3],
    room_lp: f32,
}

impl Binaural {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate,
            line: vec![0.0; LINE],
            w: 0,
            ears: [Ear::new(), Ear::new()],
            az: Glide::new(0.0),
            el: Glide::new(0.0),
            dist: Glide::new(1.0),
            px: Glide::new(0.0),
            py: Glide::new(0.0),
            pz: Glide::new(0.0),
            doppler: Glide::new(1.0),
            orbit: 0.0,
            spun: 0.0,
            k: crate::smoothing::approach(POS_TAU_S, sample_rate),
            fresh: true,
            last_delay: [f64::NAN; 3],
            hist: [vec![0.0; LINE], vec![0.0; LINE], vec![0.0; LINE]],
            room_lp: 0.0,
        }
    }

    /// Set one control (values already clamped to the parameter's range).
    pub fn set(&mut self, knob: Knob, v: f32) {
        let g = match knob {
            Knob::Azimuth => &mut self.az,
            Knob::Elevation => &mut self.el,
            Knob::Distance => &mut self.dist,
            Knob::PosX => &mut self.px,
            Knob::PosY => &mut self.py,
            Knob::PosZ => &mut self.pz,
            Knob::Doppler => &mut self.doppler,
            Knob::Orbit => {
                self.orbit = v;
                return;
            }
        };
        g.target = v;
        if self.fresh {
            g.snap();
        }
    }

    /// Jump every control to where it is set (no glide).
    pub fn snap(&mut self) {
        for g in [
            &mut self.az,
            &mut self.el,
            &mut self.dist,
            &mut self.px,
            &mut self.py,
            &mut self.pz,
            &mut self.doppler,
        ] {
            g.snap();
        }
    }

    /// The source position this sample, head-centred metres.
    fn position(&mut self) -> [f64; 3] {
        let k = self.k;
        self.spun += self.orbit as f64 / self.sr as f64;
        if self.spun.abs() > 360.0 {
            self.spun %= 360.0;
        }
        let az = (self.az.step(k) as f64 + self.spun).to_radians();
        let el = (self.el.step(k) as f64).to_radians();
        let d = self.dist.step(k) as f64;
        let (x, y, z) = (self.px.step(k) as f64, self.py.step(k) as f64, self.pz.step(k) as f64);
        [
            d * el.cos() * az.sin() + x,
            d * el.cos() * az.cos() + y,
            d * el.sin() + z,
        ]
    }

    /// Read the travel line `delay` samples back (4-point Hermite).
    #[inline]
    fn read(&self, delay: f64) -> f32 {
        let pos = self.w as f64 - 1.0 - delay;
        let i = pos.floor();
        let t = (pos - i) as f32;
        let i = i as i64;
        let m = (LINE - 1) as i64;
        let at = |k: i64| self.line[(k & m) as usize];
        let (y0, y1, y2, y3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * t + c2) * t + c1) * t + y1
    }

    /// The travel time of path `which` for the sample arriving now: the
    /// sound left when the source stood where its travel time then equals
    /// how long ago that was, D = T(now - D), found by fixed-point
    /// iteration from last sample's D (it converges while the source moves
    /// slower than sound). That is the moving-source Doppler shift
    /// f c / (c + v) exactly, not its first-order f (1 - v/c). Slewed so it
    /// never moves half a sample per sample.
    #[inline]
    fn travel(&mut self, which: usize) -> f64 {
        let h = &self.hist[which];
        let m = (LINE - 1) as i64;
        let newest = self.w as f64 - 1.0;
        let at = |d: f64| -> f64 {
            let pos = newest - d;
            let i = pos.floor();
            let f = pos - i;
            let a = h[(i as i64 & m) as usize] as f64;
            let b = h[((i as i64 + 1) & m) as usize] as f64;
            a + (b - a) * f
        };
        let last = self.last_delay[which];
        let mut d = if last.is_nan() { at(0.0) } else { last };
        for _ in 0..4 {
            d = at(d).clamp(0.0, (LINE - 8) as f64);
        }
        let d = if last.is_nan() {
            d
        } else {
            last + (d - last).clamp(-0.5, 0.5)
        };
        self.last_delay[which] = d;
        d
    }

    /// Place one stereo sample: (left ear, right ear) and the room feed
    /// (the source at a constant level, delayed by its travel time) for
    /// the strip's sends.
    pub fn process(&mut self, l: f32, r: f32) -> ((f32, f32), (f32, f32)) {
        self.fresh = false;
        self.line[self.w] = 0.5 * (l + r);
        self.w = (self.w + 1) & (LINE - 1);
        let p = self.position();
        let dop = self.doppler.step(self.k).clamp(0.0, 1.0) as f64;
        let r_c = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(MIN_DIST);
        let spm = self.sr as f64 / SOUND_C;
        let max_delay = (LINE - 8) as f64;
        let views = [ear_view(p, -1.0), ear_view(p, 1.0)];
        // each path's travel time now: the ear's path, less what doppler 0
        // takes away (the centre's distance), plus a/c so it is never
        // negative; the room hears the source from the head's centre
        let now = [
            (views[0].path - (1.0 - dop) * r_c + HEAD_A) * spm,
            (views[1].path - (1.0 - dop) * r_c + HEAD_A) * spm,
            (dop * r_c + HEAD_A) * spm,
        ];
        let newest = (self.w + LINE - 1) & (LINE - 1);
        for j in 0..3 {
            let v = now[j].min(max_delay) as f32;
            if self.last_delay[j].is_nan() {
                // the first sample: the source has always been here
                self.hist[j].iter_mut().for_each(|h| *h = v);
            }
            self.hist[j][newest] = v;
        }
        let room_d = self.travel(2);
        let room = self.read(room_d);
        // air absorption: 0.15 dB a metre at 10 kHz, as a one-pole
        let air_k = {
            let ratio2 = (10f64.powf(0.015 * r_c) - 1.0).max(1e-9);
            let fc = (10_000.0 / ratio2.sqrt()).clamp(500.0, 0.45 * self.sr as f64);
            crate::smoothing::one_pole(fc as f32, self.sr)
        };
        self.room_lp += (room - self.room_lp) * air_k;
        let mut out = [0.0f32; 2];
        let t = 2.0 * self.sr as f64;
        let beta = 2.0 * SOUND_C / HEAD_A;
        let pinna_scale = self.sr / 44_100.0;
        for e in 0..2 {
            let v = &views[e];
            let delay = self.travel(e);
            let x = self.read(delay);
            // head shadow (Brown-Duda), bilinear
            let theta_deg = v.theta_e.to_degrees();
            let alpha = 1.05 + 0.95 * (theta_deg / 150.0 * 180.0).to_radians().cos();
            let (b0, b1) = (alpha * t + beta, beta - alpha * t);
            let (a0, a1) = (t + beta, beta - t);
            let ear = &mut self.ears[e];
            let y = ((b0 * x as f64 + b1 * ear.x1 as f64 - a1 * ear.y1 as f64) / a0) as f32;
            ear.x1 = x;
            ear.y1 = y;
            // pinna and shoulder echoes off the shadowed signal
            let n = EAR_LINE;
            let wpos = (self.w + n - 1) % n;
            ear.line[wpos] = y;
            let tap = |line: &[f32], delay: f32| -> f32 {
                let d = delay.clamp(0.0, (n - 2) as f32);
                let i = d.floor();
                let f = d - i;
                let a = line[(wpos + n - i as usize) % n];
                let b = line[(wpos + n - i as usize - 1) % n];
                a + (b - a) * f
            };
            let half_az = (v.az_deg * 0.5).to_radians().cos();
            let mut s = y;
            for &(rho, a, b, d) in PINNA.iter() {
                let tau = (a * half_az * (d * (90.0 - v.el_deg)).to_radians().sin() + b) * pinna_scale;
                s += rho * tap(&ear.line, tau);
            }
            // the shoulder: later the higher the source, 0.2..1.2 ms
            let sh_ms = 1.2 * (180.0 - v.az_deg.abs().min(179.0)) / 180.0
                * (1.0 - 0.00004 * ((v.el_deg - 80.0) * 180.0 / (180.0 + v.az_deg.abs())).powi(2));
            let sh = (sh_ms.clamp(0.2, 1.2) * 0.001 * self.sr).min((n - 2) as f32);
            s += 0.3 * tap(&ear.line, sh);
            // the echoes sum to 1 at DC; the shoulder's 0.3 comes back off,
            // so the low end passes at unity
            let s = s / 1.3;
            // behind the head the pinna's flange dulls the top end
            let back = (-(v.az_deg.to_radians().cos())).max(0.0) * (1.0 - v.el_deg.abs() / 90.0);
            let lp_k = crate::smoothing::one_pole(3000.0, self.sr);
            ear.rear_lp += (s - ear.rear_lp) * lp_k;
            let s = s - 0.44 * back * (s - ear.rear_lp);
            ear.air_lp += (s - ear.air_lp) * air_k;
            let gain = (1.0 / v.path.max(0.25)) as f32;
            out[e] = ear.air_lp * gain;
        }
        ((out[0], out[1]), (self.room_lp, self.room_lp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48000.0;

    fn placed(knobs: &[(Knob, f32)]) -> Binaural {
        let mut b = Binaural::new(SR);
        for &(k, v) in knobs {
            b.set(k, v);
        }
        b
    }

    /// Band energy of `x` near `f` (Hann-windowed Goertzel over 4096
    /// blocks), as an amplitude.
    fn band(x: &[f32], f: f32) -> f32 {
        let n = 4096;
        let blocks = x.len() / n;
        let mut sum = 0.0f64;
        for b in 0..blocks {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for i in 0..n {
                let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
                let ph = std::f64::consts::TAU * f as f64 * i as f64 / SR as f64;
                re += x[b * n + i] as f64 * w * ph.cos();
                im += x[b * n + i] as f64 * w * ph.sin();
            }
            sum += (re * re + im * im).sqrt() / (0.25 * n as f64);
        }
        (sum / blocks as f64) as f32
    }

    /// A source at 90 degrees right, 2 m out: the right ear hears it first,
    /// by the 0.66 ms a 17.5 cm head gives, and louder, by little in the
    /// bass and by well over 10 dB at 6 kHz. Mirrored at -90, the left ear
    /// leads by the same.
    #[test]
    fn interaural_time_and_level_have_the_right_sign_and_size() {
        for (az, lead_right) in [(90.0f32, true), (-90.0, false)] {
            let mut b = placed(&[(Knob::Azimuth, az), (Knob::Distance, 2.0)]);
            // a click: the time difference is the onset difference
            let mut l = Vec::new();
            let mut r = Vec::new();
            for n in 0..4000 {
                let x = if n == 100 { 1.0 } else { 0.0 };
                let ((a, c), _) = b.process(x, x);
                l.push(a);
                r.push(c);
            }
            // the onset: the first sample above a tenth of each ear's peak
            let onset = |v: &[f32]| {
                let pk = v.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
                v.iter().position(|&s| s.abs() > 0.1 * pk).unwrap() as f32
            };
            let itd_ms = (onset(&l) - onset(&r)) / SR * 1000.0;
            let want = if lead_right { 0.66 } else { -0.66 };
            assert!(
                (itd_ms - want).abs() < 0.08,
                "azimuth {az}: left minus right onset {itd_ms:.3} ms, want {want}"
            );
            // levels: sines at 300 Hz and 6 kHz
            for (f, lo, hi) in [(300.0f32, 0.0, 3.0), (6000.0, 10.0, 25.0)] {
                let mut b = placed(&[(Knob::Azimuth, az), (Knob::Distance, 2.0)]);
                let mut l = Vec::new();
                let mut r = Vec::new();
                for n in 0..40000 {
                    let x = (std::f32::consts::TAU * f * n as f32 / SR).sin();
                    let ((a, c), _) = b.process(x, x);
                    l.push(a);
                    r.push(c);
                }
                let near_over_far = 20.0 * (band(&r[8000..], f) / band(&l[8000..], f)).log10();
                let ild = if lead_right { near_over_far } else { -near_over_far };
                assert!(
                    ild >= lo && ild <= hi,
                    "azimuth {az}, {f} Hz: near ear {ild:+.1} dB over the far, want {lo}..{hi}"
                );
            }
        }
    }

    /// Straight ahead, both ears hear the same thing: no time or level
    /// difference, and at 1 m the low end comes through at unity.
    #[test]
    fn straight_ahead_is_centred_at_unity() {
        let mut b = placed(&[(Knob::Azimuth, 0.0), (Knob::Distance, 1.0)]);
        let mut l = Vec::new();
        let mut r = Vec::new();
        for n in 0..40000 {
            let x = (std::f32::consts::TAU * 60.0 * n as f32 / SR).sin();
            let ((a, c), _) = b.process(x, x);
            l.push(a);
            r.push(c);
        }
        assert!(l.iter().zip(&r).all(|(a, c)| (a - c).abs() < 1e-6), "ahead is mirrored");
        let level = 20.0 * band(&l[8000..], 60.0).log10();
        assert!(level.abs() < 0.5, "60 Hz at 1 m ahead: {level:+.2} dB");
    }

    /// A source closing at 20 m/s rises by c/(c - v) (6.2%), receding at
    /// 20 m/s it falls by c/(c + v) (5.5%): the moving-source Doppler shift
    /// to within 0.1%.
    #[test]
    fn a_moving_source_is_doppler_shifted() {
        for (v, want) in [(-20.0f32, 343.0f32 / 323.0), (20.0, 343.0 / 363.0)] {
            let mut b = placed(&[(Knob::Azimuth, 0.0), (Knob::Distance, 60.0)]);
            let mut y = Vec::new();
            for n in 0..96000 {
                // the distance lane, moving every sample
                b.set(Knob::Distance, 60.0 + v * n as f32 / SR);
                let x = (std::f32::consts::TAU * 1000.0 * n as f32 / SR).sin();
                y.push(b.process(x, x).0 .0);
            }
            // zero crossings over the second second give the frequency
            let tail = &y[48000..96000];
            let zc = (1..tail.len()).filter(|&i| (tail[i - 1] < 0.0) != (tail[i] < 0.0)).count();
            let f = zc as f32 / 2.0;
            let ratio = f / 1000.0;
            assert!(
                (ratio - want).abs() < 0.001,
                "moving at {v} m/s: {f:.1} Hz, want {:.1}",
                1000.0 * want
            );
        }
    }

    /// Distance: the direct sound falls 6 dB per doubling and darkens with
    /// the air, while the room feed holds its level.
    #[test]
    fn distance_falls_off_and_the_room_holds() {
        let level = |d: f32, f: f32| -> (f32, f32) {
            let mut b = placed(&[(Knob::Distance, d)]);
            let mut y = Vec::new();
            let mut room = Vec::new();
            for n in 0..60000 {
                let x = (std::f32::consts::TAU * f * n as f32 / SR).sin();
                let ((a, _), (rm, _)) = b.process(x, x);
                y.push(a);
                room.push(rm);
            }
            (band(&y[20000..], f), band(&room[20000..], f))
        };
        let (near, room_near) = level(2.0, 200.0);
        let (far, room_far) = level(4.0, 200.0);
        let fall = 20.0 * (near / far).log10();
        assert!((fall - 6.0).abs() < 0.3, "2 m to 4 m: the direct sound falls {fall:.2} dB");
        assert!((room_near / room_far - 1.0).abs() < 0.01, "the room feed holds its level");
        // 50 m of air takes ~7 dB more off 10 kHz than off 1 kHz, beyond
        // what the head itself does to the two at 1 m
        let tilt = |d: f32| 20.0 * (level(d, 1000.0).0 / level(d, 10_000.0).0).log10();
        let air = tilt(50.0) - tilt(1.0);
        assert!(air > 5.0 && air < 10.0, "50 m of air dulls 10 kHz by {air:.1} dB re 1 kHz");
    }
}
