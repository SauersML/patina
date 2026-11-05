// `patina feel <song.song | file.wav> [options]`: play a song or a WAV
// through a modelled big sound system and report what a body in the
// floor or soil right under the subwoofer would feel, over time.
//
// The chain, every stage a linear filter with documented parameters
// (design.md, "Feel" holds the sources and every assumption):
//
//   mix -> mono -> LR4 sub crossover -> the sub's low-frequency limit
//       -> sound pressure at 1 m (the --level calibration)
//       -> the venue's ground or floor (field | club | house)
//       -> vertical acceleration a(t) at the body, m/s²
//       -> ISO 2631-1 Wk weighting -> running r.m.s., MTVV, VDV
//       -> hits, tempo, sections -> the felt report, JSON
//
// Analysis only: nothing here touches the engine, so a render is the
// same bit for bit whether or not anyone feels it.

use std::f64::consts::{FRAC_1_SQRT_2, PI, TAU};
use std::fmt::Write as _;

/// The analysis rate. The sub feed is band-limited by the crossover
/// (24 dB/oct from 90 Hz, so >130 dB down at the 4 kHz Nyquist), and
/// every rate here is a standard one, so the ground-motion WAV plays on
/// anything.
pub const FS: f64 = 8000.0;
/// Standard gravity, for reporting acceleration in g.
pub const GRAVITY: f64 = 9.80665;
/// Reference sound pressure, Pa.
const P_REF: f64 = 20e-6;
/// Density of air at 20 °C, kg/m³.
const RHO_AIR: f64 = 1.204;
/// ISO 2631-1:1997 Annex C.3: half of alert, fit people can just detect a
/// Wk-weighted vertical vibration of 0.015 m/s² peak (interquartile
/// range about 0.01 to 0.02 m/s²).
pub const PERCEPTION_PEAK: f64 = 0.015;
/// The level patina's `--render` normalizes a bounce to (-1 dBFS).
const BOUNCE_PEAK: f64 = 0.891;
/// Moving mass per unit cone area of a large PA woofer (an 18" driver is
/// about 0.27 kg over 0.115 m²): turns the cones' volume acceleration
/// into the force the motors put on them.
const CONE_KG_PER_M2: f64 = 2.35;

/// ISO 2631-1:1997 Annex C.2.3, likely reactions to the overall
/// weighted r.m.s. (m/s²). The standard's bands overlap on purpose.
pub const COMFORT: [(f64, f64, &str); 6] = [
    (0.0, 0.315, "not uncomfortable"),
    (0.315, 0.63, "a little uncomfortable"),
    (0.5, 1.0, "fairly uncomfortable"),
    (0.8, 1.6, "uncomfortable"),
    (1.25, 2.5, "very uncomfortable"),
    (2.0, f64::INFINITY, "extremely uncomfortable"),
];

/// The comfort band (or the two overlapping bands) an r.m.s. falls in.
pub fn comfort(rms: f64) -> String {
    let bands: Vec<&str> = COMFORT
        .iter()
        .filter(|(lo, hi, _)| rms >= *lo && rms < *hi)
        .map(|b| b.2)
        .collect();
    match bands.as_slice() {
        [one] => format!("'{one}'"),
        [a, b] => format!(
            "between '{}' and '{}'",
            a.trim_end_matches(" uncomfortable"),
            b
        ),
        _ => "'not uncomfortable'".into(),
    }
}

// ---------------------------------------------------------------------------
// Filters: analog prototypes, their exact response, and f64 biquads
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Cx {
    re: f64,
    im: f64,
}

impl Cx {
    fn div(self, o: Cx) -> Cx {
        let d = o.re * o.re + o.im * o.im;
        Cx {
            re: (self.re * o.re + self.im * o.im) / d,
            im: (self.im * o.re - self.re * o.im) / d,
        }
    }
    fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
}

/// One analog second-order section,
/// H(s) = (b0 + b1 s + b2 s²) / (a0 + a1 s + a2 s²).
#[derive(Clone, Copy, Debug)]
pub struct Section {
    pub b: [f64; 3],
    pub a: [f64; 3],
}

impl Section {
    pub fn lowpass(f: f64, q: f64) -> Self {
        let w = TAU * f;
        Section {
            b: [1.0, 0.0, 0.0],
            a: [1.0, 1.0 / (q * w), 1.0 / (w * w)],
        }
    }

    pub fn highpass(f: f64, q: f64) -> Self {
        let w = TAU * f;
        Section {
            b: [0.0, 0.0, 1.0 / (w * w)],
            a: [1.0, 1.0 / (q * w), 1.0 / (w * w)],
        }
    }

    fn response(&self, f: f64) -> Cx {
        let w = TAU * f;
        let num = Cx {
            re: self.b[0] - self.b[2] * w * w,
            im: self.b[1] * w,
        };
        let den = Cx {
            re: self.a[0] - self.a[2] * w * w,
            im: self.a[1] * w,
        };
        num.div(den)
    }

    /// Bilinear transform. At 8 kHz the frequency warping is under 0.02%
    /// at 100 Hz, so no prewarping is needed below the Wk band edge.
    fn digital(&self, fs: f64) -> Biquad {
        let k = 2.0 * fs;
        let [b0, b1, b2] = self.b;
        let [a0, a1, a2] = self.a;
        let n = a0 + a1 * k + a2 * k * k;
        Biquad {
            b0: (b0 + b1 * k + b2 * k * k) / n,
            b1: (2.0 * b0 - 2.0 * b2 * k * k) / n,
            b2: (b0 - b1 * k + b2 * k * k) / n,
            a1: (2.0 * a0 - 2.0 * a2 * k * k) / n,
            a2: (a0 - a1 * k + a2 * k * k) / n,
            z1: 0.0,
            z2: 0.0,
        }
    }
}

/// A cascade of analog sections: what each stage of the chain is.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub sections: Vec<Section>,
}

impl Filter {
    /// |H(j 2π f)|, exact (the analog prototype, not the digital filter).
    pub fn gain(&self, f: f64) -> f64 {
        self.sections
            .iter()
            .fold(1.0, |g, s| g * s.response(f).abs())
    }

    pub fn then(mut self, other: Filter) -> Filter {
        self.sections.extend(other.sections);
        self
    }

    /// The digital filter at `fs`.
    pub fn run(&self, fs: f64) -> Cascade {
        Cascade {
            stages: self.sections.iter().map(|s| s.digital(fs)).collect(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// A running digital filter (transposed direct form II, f64).
#[derive(Clone, Debug)]
pub struct Cascade {
    stages: Vec<Biquad>,
}

impl Cascade {
    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        self.stages.iter_mut().fold(x, |x, s| s.process(x))
    }
}

/// ISO 2631-1:1997 frequency weighting Wk (vertical whole-body), built
/// from the standard's Annex A definitions: band limiting (high-pass
/// 0.4 Hz and low-pass 100 Hz, both Butterworth), the acceleration-
/// velocity transition (f3 = f4 = 12.5 Hz, Q4 = 0.63) and the upward
/// step (f5 = 2.37 Hz, Q5 = 0.91, f6 = 3.35 Hz, Q6 = 0.91).
pub fn wk() -> Filter {
    let (w3, w4, q4) = (TAU * 12.5, TAU * 12.5, 0.63);
    let (w5, q5, w6, q6) = (TAU * 2.37, 0.91, TAU * 3.35, 0.91);
    let r = (w5 / w6).powi(2);
    Filter {
        sections: vec![
            Section::highpass(0.4, FRAC_1_SQRT_2),
            Section::lowpass(100.0, FRAC_1_SQRT_2),
            Section {
                b: [1.0, 1.0 / w3, 0.0],
                a: [1.0, 1.0 / (q4 * w4), 1.0 / (w4 * w4)],
            },
            Section {
                b: [r, r / (q5 * w5), r / (w5 * w5)],
                a: [1.0, 1.0 / (q6 * w6), 1.0 / (w6 * w6)],
            },
        ],
    }
}

/// ISO 2631-1:1997 Table 3, Wk at one-third-octave centres (the
/// standard's published factors), for checking `wk()`.
pub const WK_TABLE: [(f64, f64); 12] = [
    (1.0, 0.482),
    (2.0, 0.531),
    (4.0, 0.967),
    (6.3, 1.054),
    (8.0, 1.036),
    (12.5, 0.902),
    (16.0, 0.768),
    (31.5, 0.405),
    (50.0, 0.246),
    (63.0, 0.186),
    (80.0, 0.132),
    (100.0, 0.0887),
];

// ---------------------------------------------------------------------------
// The sound system
// ---------------------------------------------------------------------------

/// The sound system: a mono sub feed through a 4th-order Linkwitz-Riley
/// low-pass, the sub's own low-frequency limit, and a playback level.
#[derive(Clone, Debug)]
pub struct Rig {
    /// LR4 crossover to the tops, Hz. The tops' contribution to floor
    /// vibration is ignored.
    pub crossover_hz: f64,
    /// The sub's low-frequency limit (-3 dB), Hz.
    pub sub_hz: f64,
    /// 4: vented or horn box (a B4 alignment, 24 dB/oct). 2: sealed box
    /// (Qtc 0.707, 12 dB/oct).
    pub sub_order: u8,
    /// Peak SPL at 1 m (half space, on the ground) a full-scale sub-band
    /// signal produces: where the system's limiter sits.
    pub spl_db: f64,
    /// "festival", "club", "home" or "custom".
    pub level_name: String,
}

/// The `--level` presets: peak SPL at 1 m for a full-scale signal.
pub fn level_preset(name: &str) -> Option<f64> {
    match name {
        // One L-Acoustics KS28-class cabinet at its rated maximum:
        // 143 dB peak at 1 m, half space (manufacturer's data sheet)
        "festival" => Some(143.0),
        // A club double-18 run near its limit (estimate)
        "club" => Some(135.0),
        // A large home subwoofer near its limit (estimate)
        "home" => Some(112.0),
        _ => None,
    }
}

impl Rig {
    pub fn filter(&self) -> Filter {
        let mut f = Filter {
            sections: vec![
                Section::lowpass(self.crossover_hz, FRAC_1_SQRT_2),
                Section::lowpass(self.crossover_hz, FRAC_1_SQRT_2),
            ],
        };
        if self.sub_order == 2 {
            f.sections
                .push(Section::highpass(self.sub_hz, FRAC_1_SQRT_2));
        } else {
            // Butterworth 4th order: Q = 1/(2 cos(π/8)), 1/(2 cos(3π/8))
            f.sections.push(Section::highpass(self.sub_hz, 0.541_196_1));
            f.sections.push(Section::highpass(self.sub_hz, 1.306_563));
        }
        f
    }

    /// Peak pressure at 1 m for a full-scale signal, Pa.
    pub fn full_scale_pa(&self) -> f64 {
        P_REF * 10f64.powf(self.spl_db / 20.0)
    }

    /// The cones' peak volume acceleration at full scale, m³/s²: a
    /// monopole on the ground radiates p(r) = ρ0 Q'(t - r/c) / (2π r).
    pub fn full_scale_volume_accel(&self) -> f64 {
        TAU * 1.0 * self.full_scale_pa() / RHO_AIR
    }
}

// ---------------------------------------------------------------------------
// The venues: from the sub's sound pressure to acceleration at the body
// ---------------------------------------------------------------------------

/// The pressure on the ground near the sub: the 1 m level falling as 1/r
/// from the sub's acoustic centre (a monopole on the ground), held at its
/// cabinet-face value inside the cabinet's half-depth.
const R_CAB: f64 = 0.5;

fn near_field(r: f64) -> f64 {
    1.0 / r.max(R_CAB)
}

/// Festival field: the sub on soft soil, the body buried under it.
#[derive(Clone, Debug)]
pub struct Field {
    /// Shear-wave speed of the soft layer, m/s.
    pub vs: f64,
    pub rho: f64,
    pub nu: f64,
    /// Material quality factor (damping ratio 1/(2Q)).
    pub q: f64,
    /// Soft-layer thickness over firmer ground, m.
    pub layer: f64,
    pub vs_base: f64,
    pub rho_base: f64,
    /// Body depth under the ground surface, m.
    pub depth: f64,
    /// Horizontal distance from under the sub's centre, m.
    pub distance: f64,
}

impl Default for Field {
    fn default() -> Self {
        Field {
            vs: 150.0,
            rho: 1800.0,
            nu: 0.35,
            q: 10.0,
            layer: 1.5,
            vs_base: 450.0,
            rho_base: 2000.0,
            depth: 0.3,
            distance: 0.0,
        }
    }
}

/// The radius of soil the field model loads: the sub's near field within
/// about a quarter shear wavelength at 40 Hz (150/40/4 = 0.94 m). Beyond
/// it the loads arrive out of phase and the quasi-static sum stops.
const FIELD_LOAD_R: f64 = 1.0;

/// Club: a concrete slab on grade, the body lying on it.
#[derive(Clone, Debug)]
pub struct Slab {
    pub thickness: f64,
    pub e: f64,
    pub nu: f64,
    pub rho: f64,
    /// Modulus of subgrade reaction, N/m³.
    pub k: f64,
    pub rho_sub: f64,
    pub vs_sub: f64,
    pub nu_sub: f64,
    /// Distance along the slab from the sub's centre, m.
    pub distance: f64,
}

impl Default for Slab {
    fn default() -> Self {
        Slab {
            thickness: 0.15,
            e: 30e9,
            nu: 0.2,
            rho: 2400.0,
            k: 100e6,
            rho_sub: 1900.0,
            vs_sub: 200.0,
            nu_sub: 0.3,
            distance: 1.0,
        }
    }
}

/// The slab carries the sub's direct field out to this radius (a club
/// room's half-width). The slab is local (its radius of relative
/// stiffness is about 0.55 m), so loads beyond barely reach the body.
const CLUB_LOAD_R: f64 = 4.0;

/// House: a suspended timber floor, the body at its midspan.
#[derive(Clone, Debug)]
pub struct Timber {
    /// Joist span, m.
    pub span: f64,
    /// Floor width across the joists, m.
    pub width: f64,
    pub joist_b: f64,
    pub joist_h: f64,
    /// Joist spacing, m.
    pub spacing: f64,
    pub e: f64,
    /// Floor mass per area (joists, boards, ceiling below, furnishing), kg/m².
    pub mass: f64,
    /// Modal damping ratio.
    pub zeta: f64,
    /// Distance from midspan along the span, m.
    pub distance: f64,
}

impl Default for Timber {
    fn default() -> Self {
        Timber {
            span: 3.0,
            width: 4.0,
            joist_b: 0.047,
            joist_h: 0.22,
            spacing: 0.4,
            e: 11e9,
            mass: 45.0,
            zeta: 0.03,
            distance: 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Venue {
    Field(Field),
    Club(Slab),
    House(Timber),
}

impl Venue {
    pub fn name(&self) -> &'static str {
        match self {
            Venue::Field(_) => "field",
            Venue::Club(_) => "club",
            Venue::House(_) => "house",
        }
    }

    /// What the body is in or on.
    pub fn ground(&self) -> &'static str {
        match self {
            Venue::Field(_) => "the soil",
            Venue::Club(_) => "the slab",
            Venue::House(_) => "the floor",
        }
    }

    /// The venue's own rig and level, before any option overrides them.
    pub fn default_rig(&self) -> Rig {
        let (crossover_hz, sub_hz, sub_order, level) = match self {
            Venue::Field(_) => (90.0, 32.0, 4, "festival"),
            Venue::Club(_) => (90.0, 32.0, 4, "club"),
            Venue::House(_) => (80.0, 25.0, 2, "home"),
        };
        Rig {
            crossover_hz,
            sub_hz,
            sub_order,
            spl_db: level_preset(level).unwrap(),
            level_name: level.into(),
        }
    }

    pub fn coupling(&self) -> Coupling {
        match self {
            Venue::Field(f) => field_coupling(f),
            Venue::Club(s) => club_coupling(s),
            Venue::House(t) => house_coupling(t),
        }
    }
}

/// A venue reduced to what the chain needs: the static compliance at the
/// body, one resonance, and the force the pressure puts on the ground.
/// a/p = C ω0² s² / (s² + (ω0/Q) s + ω0²): compliance-controlled
/// (rising 12 dB/oct) below f0, peaking by about Q at f0, mass-controlled
/// (flat) above.
#[derive(Clone, Debug)]
pub struct Coupling {
    /// Static vertical displacement at the body per pascal of the sub's
    /// 1 m pressure, m/Pa.
    pub static_m_per_pa: f64,
    pub f0: f64,
    pub q: f64,
    /// Total vertical force of the sub's pressure on the loaded ground,
    /// N per pascal at 1 m.
    pub load_n_per_pa: f64,
    /// One line on the physics, for the report.
    pub describe: String,
    /// The derived parameters, for the JSON.
    pub derived: Vec<(&'static str, f64)>,
}

impl Coupling {
    pub fn filter(&self) -> Filter {
        let w0 = TAU * self.f0;
        Filter {
            sections: vec![Section {
                b: [0.0, 0.0, self.static_m_per_pa * w0 * w0],
                a: [w0 * w0, w0 / self.q, 1.0],
            }],
        }
    }

    /// Seconds for the venue's free ring to fall 20 dB: the slow pole's
    /// decay rate, from the exact 2nd-order roots.
    pub fn ring_20db(&self) -> f64 {
        let w0 = TAU * self.f0;
        let z = 1.0 / (2.0 * self.q);
        let sigma = if z < 1.0 {
            z * w0
        } else {
            w0 * (z - (z * z - 1.0).sqrt())
        };
        10f64.ln() / sigma
    }
}

/// Sum a surface load over the plane: the displacement at a body
/// `d` metres from the sub's centre, for a load q(r) (per pascal at 1 m,
/// r from the sub) and a kernel (displacement per newton at horizontal
/// distance ρ from the load). Polar coordinates centred on the body, so
/// a kernel singular there is integrated with its ρ dρ.
fn integrate_load(
    d: f64,
    rho_max: f64,
    load: impl Fn(f64) -> f64,
    kernel: impl Fn(f64) -> f64,
) -> f64 {
    let (nr, nphi) = (1600, 256);
    let dr = rho_max / nr as f64;
    let dphi = TAU / nphi as f64;
    let mut sum = 0.0;
    for i in 0..nr {
        let rho = (i as f64 + 0.5) * dr;
        let k = kernel(rho) * rho * dr;
        if k == 0.0 {
            continue;
        }
        let ring = if d == 0.0 {
            load(rho) * TAU
        } else {
            (0..nphi)
                .map(|j| {
                    let phi = (j as f64 + 0.5) * dphi;
                    let (x, y) = (d + rho * phi.cos(), rho * phi.sin());
                    load(x.hypot(y))
                })
                .sum::<f64>()
                * dphi
        };
        sum += k * ring;
    }
    sum
}

/// The total force of a radial load q(r) inside radius `r_max`, N per Pa.
fn load_force(r_max: f64, load: impl Fn(f64) -> f64) -> f64 {
    let n = 4000;
    let dr = r_max / n as f64;
    (0..n)
        .map(|i| {
            let r = (i as f64 + 0.5) * dr;
            load(r) * TAU * r * dr
        })
        .sum()
}

/// Lysmer's analog wave speed for vertical vibration of a loaded patch:
/// V_La = 3.4 Vs / (π (1 - ν)) (Gazetas 1991).
fn lysmer_speed(vs: f64, nu: f64) -> f64 {
    3.4 * vs / (PI * (1.0 - nu))
}

fn field_coupling(f: &Field) -> Coupling {
    let g = f.rho * f.vs * f.vs;
    let z = f.depth;
    // Boussinesq: vertical displacement at depth z, horizontal distance
    // ρ, from a unit vertical point load on an elastic half-space
    let kernel = |rho: f64| {
        let r = rho.hypot(z);
        (2.0 * (1.0 - f.nu) + z * z / (r * r)) / (4.0 * PI * g * r)
    };
    let load = |r: f64| {
        if r <= FIELD_LOAD_R {
            near_field(r)
        } else {
            0.0
        }
    };
    let halfspace = integrate_load(f.distance, f.distance + FIELD_LOAD_R, load, kernel);
    // Firmer ground under the soft layer stiffens it: a loaded patch on a
    // stratum is 1 + 1.28 a/H stiffer than on the half-space (Gazetas 1991)
    let stratum = 1.0 + 1.28 * FIELD_LOAD_R / f.layer;
    let static_m_per_pa = halfspace / stratum;
    // The soft layer's vertical (compression-extension) resonance,
    // f0 = V_La / 4H. Its shear resonance Vs/4H moves the ground
    // sideways, which this vertical model does not report.
    let v_la = lysmer_speed(f.vs, f.nu);
    let f0 = v_la / (4.0 * f.layer);
    // Peak amplification of a damped layer on an elastic base:
    // 1 / (α + π ζ / 2), α the impedance ratio (Kramer 1996, 7.2)
    let alpha = (f.rho * f.vs) / (f.rho_base * f.vs_base);
    let zeta = 1.0 / (2.0 * f.q);
    let q = 1.0 / (alpha + PI * zeta / 2.0);
    let where_ = if f.distance > 0.0 {
        format!(", {:.2} m to the side", f.distance)
    } else {
        String::new()
    };
    Coupling {
        static_m_per_pa,
        f0,
        q,
        load_n_per_pa: load_force(FIELD_LOAD_R, load),
        describe: format!(
            "festival field, soft soil (Vs {:.0} m/s, Q {:.0}) {:.1} m deep over firmer ground \
             (Vs {:.0} m/s); you are buried {:.2} m under the sub{}. Soft-layer resonance \
             {:.1} Hz (V_La/4H), Q {:.1}",
            f.vs, f.q, f.layer, f.vs_base, f.depth, where_, f0, q
        ),
        derived: vec![
            ("shear_modulus_pa", g),
            ("lysmer_speed_m_s", v_la),
            ("shear_resonance_hz", f.vs / (4.0 * f.layer)),
            ("impedance_ratio", alpha),
            ("stratum_factor", stratum),
            ("load_radius_m", FIELD_LOAD_R),
        ],
    }
}

/// The Kelvin function kei(x): series (Abramowitz & Stegun 9.9.12) to
/// x = 10, the large-x asymptote (9.10.?) beyond.
pub fn kei(x: f64) -> f64 {
    if x > 10.0 {
        kei_asymptote(x)
    } else {
        kei_series(x)
    }
}

fn kei_asymptote(x: f64) -> f64 {
    {
        // kei x = Im K0(x e^{iπ/4}), K0(z) ~ √(π/2z) e^{-z} Σ a_k z^{-k},
        // a_k = (-1)^k ((2k-1)!!)² / (k! 8^k)
        let (zr, zi) = (x * FRAC_1_SQRT_2, x * FRAC_1_SQRT_2);
        let mul = |a: (f64, f64), b: (f64, f64)| (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0);
        let inv = (zr / (x * x), -zi / (x * x));
        let (mut sum, mut term, mut coef) = ((1.0, 0.0), (1.0, 0.0), 1.0);
        for k in 1..12 {
            let odd = (2 * k - 1) as f64;
            coef *= -odd * odd / (8.0 * k as f64);
            term = mul(term, inv);
            sum = (sum.0 + coef * term.0, sum.1 + coef * term.1);
        }
        // √(π / 2z) e^{-z}: |z| = x, arg z = π/4
        let mag = (PI / (2.0 * x)).sqrt() * (-zr).exp();
        let arg = -PI / 8.0 - zi;
        let pre = (mag * arg.cos(), mag * arg.sin());
        mul(pre, sum).1
    }
}

fn kei_series(x: f64) -> f64 {
    let q = x * x / 4.0;
    let (mut ber, mut bei, mut tail) = (0.0, 0.0, 0.0);
    // term_k = (x²/4)^(2k) / ((2k)!)²  and  (x²/4)^(2k+1) / ((2k+1)!)²
    let mut even = 1.0;
    let mut odd = q;
    let mut psi = 1.0 - 0.577_215_664_901_532_9; // ψ(2)
    let mut sign = 1.0;
    for k in 0..40 {
        ber += sign * even;
        bei += sign * odd;
        tail += sign * psi * odd;
        let (n1, n2) = ((2 * k + 2) as f64, (2 * k + 3) as f64);
        even = odd * q / (n1 * n1);
        odd = even * q / (n2 * n2);
        psi += 1.0 / n1 + 1.0 / n2; // ψ(2k+4) = ψ(2k+2) + 1/(2k+2) + 1/(2k+3)
        sign = -sign;
        if odd.abs() < 1e-18 && even.abs() < 1e-18 {
            break;
        }
    }
    let log = if x > 0.0 { -(x / 2.0).ln() * bei } else { 0.0 };
    log - PI / 4.0 * ber + tail
}

fn club_coupling(s: &Slab) -> Coupling {
    // Westergaard: a thin plate on a Winkler (dense-liquid) subgrade.
    // Point-load deflection w(ρ) = -(ℓ² / 2πD) kei(ρ/ℓ), ℓ = (D/k)^¼
    let d_plate = s.e * s.thickness.powi(3) / (12.0 * (1.0 - s.nu * s.nu));
    let ell = (d_plate / s.k).powf(0.25);
    let kernel = |rho: f64| -(ell * ell / (TAU * d_plate)) * kei(rho / ell);
    let load = |r: f64| {
        if r <= CLUB_LOAD_R {
            near_field(r)
        } else {
            0.0
        }
    };
    let reach = (12.0 * ell).min(s.distance + CLUB_LOAD_R);
    let static_m_per_pa = integrate_load(s.distance, reach, load, kernel);
    // The slab's mass on the subgrade spring, damped by radiation into
    // the ground: per area, Lysmer's dashpot ρ V_La
    let m = s.rho * s.thickness;
    let f0 = (s.k / m).sqrt() / TAU;
    let c = s.rho_sub * lysmer_speed(s.vs_sub, s.nu_sub);
    let zeta = c / (2.0 * (s.k * m).sqrt());
    let q = 1.0 / (2.0 * zeta);
    Coupling {
        static_m_per_pa,
        f0,
        q,
        load_n_per_pa: load_force(CLUB_LOAD_R, load),
        describe: format!(
            "club, a {:.0} mm concrete slab on grade (subgrade {:.0} MN/m³); you lie on it \
             {:.2} m from the sub. Slab-on-subgrade resonance {:.0} Hz, overdamped (Q {:.2}) \
             by radiation into the ground",
            s.thickness * 1000.0,
            s.k / 1e6,
            s.distance,
            f0,
            q
        ),
        derived: vec![
            ("plate_rigidity_nm", d_plate),
            ("relative_stiffness_radius_m", ell),
            ("mass_per_area_kg_m2", m),
            ("damping_ratio", zeta),
            ("load_radius_m", CLUB_LOAD_R),
        ],
    }
}

fn house_coupling(t: &Timber) -> Coupling {
    // One-way joisted floor, simply supported: fundamental
    // f1 = (π / 2L²) √(EI/m) per metre width
    let ei = t.e * t.joist_b * t.joist_h.powi(3) / 12.0 / t.spacing;
    let f0 = PI / (2.0 * t.span * t.span) * (ei / t.mass).sqrt();
    let w0 = TAU * f0;
    // The sub stands on the floor at midspan: its direct field, weighted
    // by the mode shape sin(πx/L), is the pressure that drives the mode
    let (nx, ny) = (240, 320);
    let (mut num, mut den, mut force) = (0.0, 0.0, 0.0);
    let da = (t.span / nx as f64) * (t.width / ny as f64);
    for i in 0..nx {
        let x = (i as f64 + 0.5) * t.span / nx as f64;
        let phi = (PI * x / t.span).sin();
        for j in 0..ny {
            let y = (j as f64 + 0.5) * t.width / ny as f64 - t.width / 2.0;
            let p = near_field((x - t.span / 2.0).hypot(y));
            num += p * phi;
            den += phi;
            force += p * da;
        }
    }
    let q_eff = num / den;
    // Modal: w = φ(body) ∫ q φ dA / (M1 ω1²), ∫ φ dA = (2/π) L B,
    // M1 = m L B / 2
    let shape = (PI * t.distance / t.span).cos();
    let static_m_per_pa = 4.0 / PI * q_eff * shape / (t.mass * w0 * w0);
    let q = 1.0 / (2.0 * t.zeta);
    Coupling {
        static_m_per_pa,
        f0,
        q,
        load_n_per_pa: force,
        describe: format!(
            "house, a suspended timber floor ({:.0}x{:.0} mm joists at {:.0} mm, {:.1} m span, \
             {:.0} kg/m²); you are in it at {}. Floor fundamental {:.1} Hz, Q {:.0} ({:.0}% damping)",
            t.joist_b * 1000.0,
            t.joist_h * 1000.0,
            t.spacing * 1000.0,
            t.span,
            t.mass,
            if t.distance > 0.0 {
                format!("{:.2} m from midspan", t.distance)
            } else {
                "midspan, under the sub".into()
            },
            f0,
            q,
            t.zeta * 100.0
        ),
        derived: vec![
            ("bending_stiffness_nm2_per_m", ei),
            ("mode_weighted_pressure_per_pa", q_eff),
            ("mode_shape_at_body", shape),
        ],
    }
}

// ---------------------------------------------------------------------------
// The chain
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Settings {
    pub venue: Venue,
    pub rig: Rig,
    /// Songs: normalize the bounce to -1 dBFS peak first, the file
    /// `--render` would write.
    pub normalize: bool,
}

impl Settings {
    pub fn new(venue: Venue) -> Self {
        let rig = venue.default_rig();
        Settings {
            venue,
            rig,
            normalize: true,
        }
    }
}

/// The 50 fps felt envelope. Frame i covers [i/50, (i+1)/50) s; the
/// running r.m.s. values integrate the τ seconds up to the frame's end
/// (ISO 2631-1 6.3.1, linear integration).
#[derive(Clone, Debug, Default)]
pub struct Frames {
    pub wk_rms: Vec<f32>,
    pub wk_rms_125ms: Vec<f32>,
    pub wk_rms_1s: Vec<f32>,
    pub wk_peak: Vec<f32>,
    pub peak: Vec<f32>,
}

pub const FPS: f64 = 50.0;

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub duration: f64,
    /// Peak vertical acceleration, m/s².
    pub peak: f64,
    pub peak_wk: f64,
    pub rms_wk: f64,
    /// Maximum transient vibration value: the largest 1 s running r.m.s.
    pub mtvv: f64,
    pub max_125ms: f64,
    /// Vibration dose value, m/s^1.75.
    pub vdv: f64,
    pub crest: f64,
    /// Fraction of frames whose Wk peak reaches the perception threshold.
    pub felt_fraction: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Hit {
    /// Onset: where the weighted signal first reaches half the hit's peak.
    pub t: f64,
    pub peak: f64,
    pub peak_wk: f64,
    /// Dominant frequency over the hit's first 160 ms.
    pub hz: Option<f64>,
    /// From the envelope's peak to 20 dB below it.
    pub ring_ms: Option<f64>,
    pub ring_extrapolated: bool,
    /// The last 40% of the gap to the next hit (up to 1 s of it).
    pub between_rms_wk: Option<f64>,
    pub between_peak: Option<f64>,
    pub between_hz: Option<f64>,
    pub felt: bool,
    /// Song tracks with a note starting just before the hit.
    pub sources: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct FeltSection {
    pub start: f64,
    pub end: f64,
    pub still: bool,
    pub rms_wk: f64,
    pub mtvv: f64,
    pub peak: f64,
    pub peak_wk: f64,
    /// Level change from the previous section, dB (r.m.s.).
    pub change_db: Option<f64>,
    /// Index range into `Felt::hits`.
    pub hits: std::ops::Range<usize>,
    pub interval: Option<f64>,
    pub between_rms_wk: Option<f64>,
    pub between_hz: Option<f64>,
    /// Dominant frequency of the loudest second.
    pub hz: Option<f64>,
    /// Song tracks that play in the section.
    pub tracks: Vec<String>,
    /// Those of them that can reach the sub (see `SongNotes::low`).
    pub low_tracks: Vec<String>,
}

pub struct Felt {
    pub settings: Settings,
    pub coupling: Coupling,
    /// Vertical acceleration at the body, m/s² (positive down: the push of
    /// a compression), at `FS`.
    pub accel: Vec<f32>,
    /// The same, Wk-weighted.
    pub weighted: Vec<f32>,
    /// Full-band stereo peak of the input before any gain.
    pub input_peak: f64,
    /// Gain applied before the rig (the bounce normalization).
    pub gain: f64,
    /// Peak of the sub feed after the gain, digital (1 = full scale).
    pub sub_peak: f64,
    pub frames: Frames,
    pub summary: Summary,
    pub hits: Vec<Hit>,
    pub sections: Vec<FeltSection>,
}

/// Note starts by track name, from a song, to say what plays and what
/// lands on each hit. `low` keeps only the notes that can reach the sub:
/// kick drums, samples (their pitch is unknown) and notes at E3 (165 Hz)
/// or below; a hit names only those.
#[derive(Clone, Debug, Default)]
pub struct SongNotes {
    pub notes: Vec<(f64, String)>,
    pub low: Vec<(f64, String)>,
}

impl SongNotes {
    pub fn from_song(song: &crate::song::Song) -> Self {
        let name = |channel: u16| {
            song.tracks
                .iter()
                .find(|(_, c)| *c == channel)
                .map(|(n, _)| n.clone())
        };
        let (mut notes, mut low) = (Vec::new(), Vec::new());
        for e in &song.events {
            if let crate::song::EventKind::NoteOn {
                note,
                channel,
                velocity,
            } = e.kind
            {
                if velocity <= 0.0 {
                    continue;
                }
                let Some(n) = name(channel) else { continue };
                let reaches_sub = if crate::drums::kit_for_channel(channel).is_some() {
                    crate::drums::DrumVoice::for_note(note) == Some(crate::drums::DrumVoice::Kick)
                } else if (crate::sampler::SAMPLER_CHANNEL_BASE..crate::vox::VOX_CHANNEL)
                    .contains(&channel)
                {
                    true
                } else {
                    note <= 52
                };
                if reaches_sub {
                    low.push((e.time, n.clone()));
                }
                notes.push((e.time, n));
            }
        }
        SongNotes { notes, low }
    }

    fn in_range(&self, t0: f64, t1: f64) -> Vec<String> {
        names_between(&self.notes, t0, t1)
    }

    fn low_in_range(&self, t0: f64, t1: f64) -> Vec<String> {
        names_between(&self.low, t0, t1)
    }
}

fn names_between(notes: &[(f64, String)], t0: f64, t1: f64) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (t, n) in notes {
        if *t >= t0 && *t < t1 && !out.contains(n) {
            out.push(n.clone());
        }
    }
    out
}

/// Run the whole chain on stereo frames at `rate` Hz.
pub fn feel(
    frames: impl Iterator<Item = (f32, f32)>,
    rate: f64,
    settings: Settings,
    notes: Option<&SongNotes>,
) -> Felt {
    // Sound system at the input rate: mono, crossover, the sub's limit.
    // The crossover is also the anti-alias filter for the step down to FS
    // (linear interpolation of a signal oversampled 100x).
    let mut rig = settings.rig.filter().run(rate);
    let step = rate / FS;
    let mut sub: Vec<f64> = Vec::new();
    let mut prev = 0.0;
    let mut next_out = 0.0;
    let mut input_peak = 0.0f32;
    for (i, (l, r)) in frames.enumerate() {
        input_peak = input_peak.max(l.abs()).max(r.abs());
        let y = rig.process(0.5 * (l as f64 + r as f64));
        let i = i as f64;
        while next_out <= i {
            let frac = next_out - (i - 1.0);
            sub.push(prev + (y - prev) * frac.min(1.0));
            next_out += step;
        }
        prev = y;
    }
    let input_peak = input_peak as f64;
    let gain = if settings.normalize && input_peak > 1e-6 {
        BOUNCE_PEAK / input_peak
    } else {
        1.0
    };
    let sub_peak = sub.iter().fold(0.0f64, |m, x| m.max(x.abs())) * gain;
    let coupling = settings.venue.coupling();
    let mut ground = coupling.filter().run(FS);
    let mut weigh = wk().run(FS);
    let pa = gain * settings.rig.full_scale_pa();
    let mut accel = Vec::with_capacity(sub.len());
    let mut weighted = Vec::with_capacity(sub.len());
    for x in sub {
        let a = ground.process(x * pa);
        accel.push(a as f32);
        weighted.push(weigh.process(a) as f32);
    }
    let mut felt = Felt {
        settings,
        coupling,
        accel,
        weighted,
        input_peak,
        gain,
        sub_peak,
        frames: Frames::default(),
        summary: Summary::default(),
        hits: Vec::new(),
        sections: Vec::new(),
    };
    analyze(&mut felt, notes);
    felt
}

// ---------------------------------------------------------------------------
// Analysis: envelope, hits, tempo, sections
// ---------------------------------------------------------------------------

/// 5 ms analysis hop at FS.
const HOP: usize = 40;
/// Onset envelope window (25 ms) and ring envelope window (40 ms: one
/// period at 25 Hz, so a low ring does not ripple).
const WIN_ON: usize = 200;
const WIN_RING: usize = 320;
/// An onset is a rise of this much over the quietest point 15-60 ms before.
const RISE_DB: f64 = 6.0;
/// Bands (Hz, 0 = open) the onset detector also watches, with
/// Linkwitz-Riley slopes: a held bass note sits in one or two of them.
const ONSET_BANDS: [(f64, f64); 4] = [(0.0, 40.0), (40.0, 63.0), (63.0, 100.0), (100.0, 0.0)];
/// Hits closer than this are one hit (16th notes at 187 bpm).
const MIN_GAP_HOPS: usize = 16;
/// Hits fainter than half the perception threshold are not reported.
const HIT_FLOOR: f64 = 0.5 * PERCEPTION_PEAK;

fn prefix_squares(x: &[f32]) -> Vec<f64> {
    let mut p = Vec::with_capacity(x.len() + 1);
    let mut s = 0.0;
    p.push(0.0);
    for &v in x {
        s += v as f64 * v as f64;
        p.push(s);
    }
    p
}

fn rms_between(prefix: &[f64], lo: usize, hi: usize) -> f64 {
    let n = prefix.len() - 1;
    let (lo, hi) = (lo.min(n), hi.min(n));
    if hi <= lo {
        return 0.0;
    }
    ((prefix[hi] - prefix[lo]) / (hi - lo) as f64)
        .max(0.0)
        .sqrt()
}

/// r.m.s. in dB re 1 m/s², centred windows at every hop.
fn envelope_db(prefix: &[f64], win: usize) -> Vec<f64> {
    let n = prefix.len() - 1;
    (0..n / HOP + 1)
        .map(|k| {
            let c = k * HOP;
            let r = rms_between(prefix, c.saturating_sub(win / 2), c + win / 2);
            20.0 * r.max(1e-10).log10()
        })
        .collect()
}

/// Dominant frequency 15-200 Hz of a stretch of signal: Hann-windowed DFT
/// on a 1 Hz grid, refined by a parabola through the peak.
pub fn dominant_hz(x: &[f32]) -> Option<f64> {
    let n = x.len();
    if n < 64 {
        return None;
    }
    let w: Vec<f64> = (0..n)
        .map(|i| x[i] as f64 * (0.5 - 0.5 * (TAU * i as f64 / (n - 1) as f64).cos()))
        .collect();
    let power = |f: f64| {
        let (c, s) = ((TAU * f / FS).cos(), (TAU * f / FS).sin());
        let (mut re, mut im) = (1.0, 0.0);
        let (mut sr, mut si) = (0.0, 0.0);
        for &v in &w {
            sr += v * re;
            si -= v * im;
            let nre = re * c - im * s;
            im = re * s + im * c;
            re = nre;
        }
        sr * sr + si * si
    };
    let grid: Vec<f64> = (15..=200).map(|f| power(f as f64)).collect();
    let (i, &best) = grid.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
    if best <= 0.0 {
        return None;
    }
    let mut f = 15.0 + i as f64;
    if i > 0 && i + 1 < grid.len() {
        let (a, b, c) = (
            grid[i - 1].max(1e-30).ln(),
            best.ln(),
            grid[i + 1].max(1e-30).ln(),
        );
        let den = a - 2.0 * b + c;
        if den < 0.0 {
            f += (0.5 * (a - c) / den).clamp(-0.5, 0.5);
        }
    }
    Some(f)
}

/// The most common spacing of a train of times: the interval with the
/// most others within 4% of it, refined to their mean. Returns the
/// interval and the fraction of spacings that fit it.
pub fn regular_interval(times: &[f64]) -> Option<(f64, f64)> {
    let iois: Vec<f64> = times
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d > 0.05 && *d < 4.0)
        .collect();
    if iois.len() < 2 {
        return None;
    }
    let support = |c: f64| iois.iter().filter(|d| (**d - c).abs() <= 0.04 * c).count();
    let best = iois
        .iter()
        .copied()
        .max_by(|a, b| support(*a).cmp(&support(*b)).then(b.total_cmp(a)))?;
    let fit: Vec<f64> = iois
        .iter()
        .copied()
        .filter(|d| (*d - best).abs() <= 0.04 * best)
        .collect();
    let mean = fit.iter().sum::<f64>() / fit.len() as f64;
    Some((mean, fit.len() as f64 / iois.len() as f64))
}

fn analyze(felt: &mut Felt, notes: Option<&SongNotes>) {
    let a = &felt.accel;
    let aw = &felt.weighted;
    let n = aw.len();
    let duration = n as f64 / FS;
    let prefix = prefix_squares(aw);

    // ---- 50 fps envelope and the whole-song numbers
    let per = (FS / FPS) as usize;
    let nframes = n.div_ceil(per);
    let mut fr = Frames::default();
    for i in 0..nframes {
        let (lo, hi) = (i * per, ((i + 1) * per).min(n));
        fr.wk_rms.push(rms_between(&prefix, lo, hi) as f32);
        fr.wk_rms_125ms
            .push(rms_between(&prefix, hi.saturating_sub(1000), hi) as f32);
        fr.wk_rms_1s
            .push(rms_between(&prefix, hi.saturating_sub(8000), hi) as f32);
        fr.wk_peak
            .push(aw[lo..hi].iter().fold(0.0f32, |m, v| m.max(v.abs())));
        fr.peak
            .push(a[lo..hi].iter().fold(0.0f32, |m, v| m.max(v.abs())));
    }
    let max = |v: &[f32]| v.iter().fold(0.0f32, |m, x| m.max(*x)) as f64;
    let rms_wk = rms_between(&prefix, 0, n);
    let peak_wk = max(&fr.wk_peak);
    let vdv = (aw.iter().map(|v| (*v as f64).powi(4)).sum::<f64>() / FS).powf(0.25);
    felt.summary = Summary {
        duration,
        peak: max(&fr.peak),
        peak_wk,
        rms_wk,
        mtvv: max(&fr.wk_rms_1s),
        max_125ms: max(&fr.wk_rms_125ms),
        vdv,
        crest: if rms_wk > 0.0 { peak_wk / rms_wk } else { 0.0 },
        felt_fraction: fr
            .wk_peak
            .iter()
            .filter(|p| **p as f64 >= PERCEPTION_PEAK)
            .count() as f64
            / nframes.max(1) as f64,
    };

    // ---- hits: onsets in the weighted envelope
    // Signal 0 is the whole weighted signal; 1-4 are its bands
    // (ONSET_BANDS). A held bass sits in one or two bands, so a kick on
    // top of it, whose falling pitch sweeps through the others, still
    // rises there when the whole signal barely does.
    let mut prefixes: Vec<Vec<f64>> = Vec::with_capacity(1 + ONSET_BANDS.len());
    prefixes.push(prefix.clone());
    for &(lo, hi) in &ONSET_BANDS {
        let mut f = Filter::default();
        if lo > 0.0 {
            f.sections.push(Section::highpass(lo, FRAC_1_SQRT_2));
            f.sections.push(Section::highpass(lo, FRAC_1_SQRT_2));
        }
        if hi > 0.0 {
            f.sections.push(Section::lowpass(hi, FRAC_1_SQRT_2));
            f.sections.push(Section::lowpass(hi, FRAC_1_SQRT_2));
        }
        let mut run = f.run(FS);
        let band: Vec<f32> = aw.iter().map(|v| run.process(*v as f64) as f32).collect();
        prefixes.push(prefix_squares(&band));
    }
    let envs: Vec<Vec<f64>> = prefixes.iter().map(|p| envelope_db(p, WIN_ON)).collect();
    let env_on = &envs[0];
    let env_ring = envelope_db(&prefix, WIN_RING);
    // Candidates: hops where the envelope stands RISE_DB over its quietest
    // point 15-60 ms before (silence before the start), in the whole
    // signal or in a band that carries some share of it (within 20 dB).
    // A candidate's crest is where the whole envelope's rise stops (1.5 dB
    // under its running top, at most 200 ms on); it must reach the hit
    // floor. A pick then holds the envelope's top over its first 90 ms too,
    // or later while still rising: a kick's tail beating against a held
    // bass swells again within 90 ms when the two are 11 Hz or more apart.
    // Candidates up to that top, or within 80 ms of the last pick, belong
    // to it, and replace it only when their crest is RISE_DB higher (a
    // ripple in a ring just before the real hit).
    let crest = |k: usize, hold: usize| {
        let (mut best, mut j) = (k, k);
        while j + 1 < env_on.len()
            && j + 1 - k <= 40
            && (j + 1 - k <= hold || env_on[j + 1] > env_on[best] - 1.5)
        {
            j += 1;
            if env_on[j] > env_on[best] {
                best = j;
            }
        }
        best
    };
    let around = |kc: usize| {
        let (s0, s1) = (kc.saturating_sub(3) * HOP, ((kc + 3) * HOP).min(n));
        aw[s0.min(s1)..s1]
            .iter()
            .enumerate()
            .fold((s0, 0.0f32), |(bi, bv), (i, v)| {
                if v.abs() > bv {
                    (s0 + i, v.abs())
                } else {
                    (bi, bv)
                }
            })
    };
    // A candidate's onset: where a signal that rose (its 25 ms r.m.s. at
    // sample resolution, so the time does not jump a half-cycle between
    // near-identical hits), after the dip it rose from (at most 20 ms
    // before the candidate), last comes up within 20 dB of its top, or 3 dB
    // over that dip if that is higher; the earliest of them. That is the
    // steep front of the attack, before a kick's pitch sweep settles.
    // A hit brings new energy: the 160 ms after its onset carry at least
    // twice the 160 ms before, in the whole signal, or in a band while the
    // whole signal gains a quarter. A swell where a held bass comes back
    // out of a kick's ring (the two beating) is not a hit, nor is a faint
    // onset inside a louder hit's ring.
    let onset_of = |k: usize, kc: usize, rose: &[(usize, f64, usize)]| -> Option<f64> {
        let (mut t, mut new_energy) = (f64::INFINITY, false);
        for &(si, base, dip) in rose {
            let lo = dip.max(k.saturating_sub(4)) * HOP;
            let p = &prefixes[si];
            let rms_at = |j: usize| rms_between(p, j.saturating_sub(WIN_ON / 2), j + WIN_ON / 2);
            let top_hop =
                (k..=kc.max(k)).fold(k, |b, j| if envs[si][j] > envs[si][b] { j } else { b });
            let top = (top_hop * HOP).min(n.saturating_sub(1));
            if top < lo {
                continue;
            }
            let half = (0.1 * rms_at(top)).max(1.41 * 10f64.powf(base / 20.0));
            // the last crossing before the top: the final rise into it, not
            // a ripple in the ring before
            let j = match (lo..=top).rev().find(|&j| rms_at(j) < half) {
                Some(b) if b < top => b + 1,
                Some(_) => top,
                None => lo,
            };
            let ts = if j > lo {
                let (y0, y1) = (rms_at(j - 1), rms_at(j));
                (j - 1) as f64 + (half - y0) / (y1 - y0).max(1e-12)
            } else {
                j as f64
            } / FS;
            let on = (ts * FS) as usize;
            let gain = |p: &[f64], after_s: f64| {
                let w = (0.16 * FS) as usize;
                let before = rms_between(p, on.saturating_sub(w), on);
                let after = rms_between(p, on, on + (after_s * FS) as usize);
                after * after / (before * before).max(1e-30)
            };
            // and it is there at once: a ripple just before the next hit
            // does not borrow that hit's energy
            let fresh = gain(p, 0.05) >= 2.0
                && if si == 0 {
                    gain(p, 0.16) >= 2.0
                } else {
                    gain(p, 0.16) >= 2.0 && gain(&prefix, 0.16) >= 1.25
                };
            if fresh {
                new_energy = true;
                t = t.min(ts);
            }
        }
        (new_energy && t.is_finite()).then_some(t)
    };
    // (hop, crest hop, top hop over the first 90 ms, onset time)
    let mut picks: Vec<(usize, usize, usize, f64)> = Vec::new();
    // The signals standing RISE_DB over their dip since `from` at hop k
    let rising = |k: usize, from: isize| -> Vec<(usize, f64, usize)> {
        envs.iter()
            .enumerate()
            .filter(|(si, e)| *si == 0 || e[k] >= env_on[k] - 20.0)
            .filter_map(|(si, e)| {
                let (base, dip) = (from..=k as isize - 3)
                    .map(|j| {
                        if j < 0 {
                            (-200.0, 0)
                        } else {
                            (e[j as usize], j as usize)
                        }
                    })
                    .fold((f64::INFINITY, k), |b, x| if x.0 < b.0 { x } else { b });
                (e[k] - base >= RISE_DB).then_some((si, base, dip))
            })
            .collect()
    };
    // A pick's onset looks at every signal that rises within 20 ms of it:
    // a kick's falling pitch reaches the bands one after another
    let onset_near = |k: usize, kc: usize, from: isize| {
        let mut all: Vec<(usize, f64, usize)> = Vec::new();
        for j in k..=(k + 4).min(env_on.len() - 1) {
            for r in rising(j, from) {
                if !all.iter().any(|a| a.0 == r.0) {
                    all.push(r);
                }
            }
        }
        onset_of(k, kc.max(k + 4).min(env_on.len() - 1), &all)
    };
    for k in 0..env_on.len() {
        // after a crest, a new rise is measured from the dip that follows it
        let from = picks
            .last()
            .map_or(k as isize - 12, |p| (k as isize - 12).max(p.2 as isize));
        if rising(k, from).is_empty() {
            continue;
        }
        let kc = crest(k, 0);
        if (around(kc).1 as f64) < HIT_FLOOR {
            continue;
        }
        if let Some(last) = picks.last() {
            if k <= last.2 || k < last.0 + MIN_GAP_HOPS {
                if env_on[kc] > env_on[last.1] + RISE_DB {
                    if let Some(t) = onset_near(k, kc, from) {
                        let k = k.max((t * FS) as usize / HOP);
                        *picks.last_mut().unwrap() = (k, kc.max(k), crest(k, 18), t);
                    }
                }
                continue;
            }
        }
        if let Some(t) = onset_near(k, kc, from) {
            let k = k.max((t * FS) as usize / HOP);
            picks.push((k, kc.max(k), crest(k, 18), t));
        }
    }
    let onsets: Vec<(f64, usize, f64)> = picks
        .iter()
        .map(|&(k, _, top, t)| (t, k, around(top).1 as f64))
        .collect();

    let mut hits = Vec::with_capacity(onsets.len());
    for (i, &(t, k, pw)) in onsets.iter().enumerate() {
        let next = onsets.get(i + 1).map(|o| o.0);
        let next_hop = onsets.get(i + 1).map(|o| o.1).unwrap_or(env_ring.len());
        let s0 = ((t - 0.005) * FS).max(0.0) as usize;
        let s1 = (((t + 0.1) * FS) as usize)
            .min(next.map(|x| (x * FS) as usize).unwrap_or(n))
            .min(n)
            .max(s0 + 1);
        let peak = a[s0..s1.min(n)].iter().fold(0.0f32, |m, v| m.max(v.abs())) as f64;
        let body_end = ((t + 0.16).min(next.unwrap_or(f64::INFINITY)) * FS) as usize;
        let hz = dominant_hz(&a[(t * FS) as usize..body_end.min(n)]);
        // Ring-down on the 40 ms envelope, from its peak in the first 100 ms
        let kp_hi = (k + 20).min(next_hop).min(env_ring.len()).max(k + 1);
        let (kp, peak_db) = (k..kp_hi)
            .map(|j| (j, env_ring[j]))
            .fold((k, f64::NEG_INFINITY), |b, x| if x.1 > b.1 { x } else { b });
        let stop = next_hop
            .saturating_sub(4)
            .min(kp + (3.0 * FS) as usize / HOP)
            .min(env_ring.len());
        let mut ring_ms = None;
        let mut extrapolated = false;
        if let Some(j) = (kp..stop).find(|&j| env_ring[j] <= peak_db - 20.0) {
            ring_ms = Some((j - kp) as f64 * HOP as f64 / FS * 1000.0);
        } else if stop > kp + 4 {
            // Interrupted before -20 dB: extrapolate the decay rate, fitted
            // from where it is 3 dB down to its lowest point
            let jmin = (kp..stop)
                .min_by(|x, y| env_ring[*x].total_cmp(&env_ring[*y]))
                .unwrap();
            let j3 = (kp..=jmin).find(|&j| env_ring[j] <= peak_db - 3.0);
            if let Some(j3) = j3 {
                if jmin > j3 + 2 && peak_db - env_ring[jmin] >= 6.0 {
                    let pts: Vec<(f64, f64)> = (j3..=jmin)
                        .map(|j| (j as f64 * HOP as f64 / FS, env_ring[j]))
                        .collect();
                    let m = pts.len() as f64;
                    let (sx, sy) = pts.iter().fold((0.0, 0.0), |s, p| (s.0 + p.0, s.1 + p.1));
                    let (mx, my) = (sx / m, sy / m);
                    let (sxy, sxx) = pts.iter().fold((0.0, 0.0), |s, p| {
                        (s.0 + (p.0 - mx) * (p.1 - my), s.1 + (p.0 - mx).powi(2))
                    });
                    let slope = sxy / sxx.max(1e-12); // dB/s
                    if slope < -1.0 {
                        let t3 = (j3 - kp) as f64 * HOP as f64 / FS;
                        ring_ms = Some((t3 + 17.0 / -slope) * 1000.0);
                        extrapolated = true;
                    }
                }
            }
        }
        // What is felt between this hit and the next
        let (mut between_rms_wk, mut between_peak, mut between_hz) = (None, None, None);
        if let Some(tn) = next {
            let gap = tn - t;
            let b0 = (t + 0.6 * gap).max(tn - 1.0);
            let b1 = tn - 0.01;
            if b1 - b0 >= 0.02 {
                let (i0, i1) = ((b0 * FS) as usize, ((b1 * FS) as usize).min(n));
                let r = rms_between(&prefix, i0, i1);
                between_rms_wk = Some(r);
                between_peak = Some(a[i0..i1].iter().fold(0.0f32, |m, v| m.max(v.abs())) as f64);
                if r >= PERCEPTION_PEAK / 2f64.sqrt() / 4.0 {
                    between_hz = dominant_hz(&a[i0..i1]);
                }
            }
        }
        let sources = notes
            .map(|s| s.low_in_range(t - 0.08, t + 0.01))
            .unwrap_or_default();
        hits.push(Hit {
            t,
            peak,
            peak_wk: pw,
            hz,
            ring_ms,
            ring_extrapolated: extrapolated,
            between_rms_wk,
            between_peak,
            between_hz,
            felt: pw >= PERCEPTION_PEAK,
            sources,
        });
    }

    let sections = segment(&prefix, &hits, n);
    let mut out = Vec::with_capacity(sections.len());
    let mut prev_rms: Option<f64> = None;
    for (start, end) in sections {
        let (f0, f1) = ((start * FPS) as usize, ((end * FPS) as usize).min(nframes));
        let (i0, i1) = ((start * FS) as usize, ((end * FS) as usize).min(n));
        let rms_wk = rms_between(&prefix, i0, i1);
        let pk_wk = max(&fr.wk_peak[f0..f1.max(f0)]);
        let h0 = hits.partition_point(|h| h.t < start);
        let h1 = hits.partition_point(|h| h.t < end);
        let sec_hits = &hits[h0..h1];
        let times: Vec<f64> = sec_hits.iter().map(|h| h.t).collect();
        let interval = regular_interval(&times)
            .filter(|(_, frac)| *frac >= 0.5)
            .map(|(iv, _)| iv);
        let median = |mut v: Vec<f64>| {
            if v.is_empty() {
                return None;
            }
            v.sort_by(f64::total_cmp);
            Some(v[v.len() / 2])
        };
        let between_rms_wk = median(sec_hits.iter().filter_map(|h| h.between_rms_wk).collect());
        let between_hz = median(sec_hits.iter().filter_map(|h| h.between_hz).collect());
        // The loudest second's dominant frequency
        let loud = (f0..f1.max(f0 + 1).min(nframes))
            .max_by(|x, y| fr.wk_rms_1s[*x].total_cmp(&fr.wk_rms_1s[*y]))
            .unwrap_or(f0);
        let le = (((loud + 1) as f64 / FPS) * FS) as usize;
        let hz = if pk_wk >= HIT_FLOOR {
            dominant_hz(&a[le.saturating_sub(8000).max(i0)..le.min(n)])
        } else {
            None
        };
        let floor = PERCEPTION_PEAK / 2f64.sqrt() / 4.0;
        let change_db = prev_rms.map(|p| 20.0 * (rms_wk.max(floor) / p.max(floor)).log10());
        prev_rms = Some(rms_wk);
        out.push(FeltSection {
            start,
            end,
            // still: no hit you'd feel, and at most 2% of the time over
            // the threshold (a ring crossing the boundary is not a section)
            still: !hits[h0..h1].iter().any(|h| h.felt)
                && fr.wk_peak[f0..f1.max(f0)]
                    .iter()
                    .filter(|p| **p as f64 >= PERCEPTION_PEAK)
                    .count()
                    * 50
                    <= (f1 - f0.min(f1)).max(1),
            rms_wk,
            mtvv: max(&fr.wk_rms_1s[f0..f1.max(f0)]),
            peak: max(&fr.peak[f0..f1.max(f0)]),
            peak_wk: pk_wk,
            change_db,
            hits: h0..h1,
            interval,
            between_rms_wk,
            between_hz,
            hz,
            // (a note on the boundary belongs to the section it starts)
            tracks: notes
                .map(|s| s.in_range(start - 0.01, end - 0.03))
                .unwrap_or_default(),
            low_tracks: notes
                .map(|s| s.low_in_range(start - 0.01, end - 0.03))
                .unwrap_or_default(),
        });
    }
    felt.frames = fr;
    felt.hits = hits;
    felt.sections = out;
}

/// Split the piece where the felt level changes by 3 dB or more: an
/// optimal piecewise-constant fit of the 2 s Wk level (four values a
/// second; anything under a quarter of the perception threshold counts
/// as still), each section at least 4 s long or 2 s if it is still (a
/// break); then neighbours less than 3 dB apart merge, and each boundary
/// moves to the sharpest step within 2 s of it (an upward one onto its
/// first hit).
fn segment(prefix: &[f64], hits: &[Hit], n: usize) -> Vec<(f64, f64)> {
    let duration = n as f64 / FS;
    let floor_db = 20.0 * (PERCEPTION_PEAK / 2f64.sqrt() / 4.0).log10();
    let level = |t0: f64, t1: f64| {
        let r = rms_between(prefix, (t0.max(0.0) * FS) as usize, (t1 * FS) as usize);
        (20.0 * r.max(1e-10).log10()).max(floor_db)
    };
    const MIN_LOUD: usize = 16;
    const MIN_STILL: usize = 8;
    let nb = (duration * 4.0).ceil() as usize;
    if nb < 2 * MIN_STILL {
        return vec![(0.0, duration)];
    }
    let lv: Vec<f64> = (0..nb)
        .map(|i| {
            let c = (i as f64 + 0.5) * 0.25;
            level(c - 1.0, (c + 1.0).min(duration))
        })
        .collect();
    let (mut s1, mut s2) = (vec![0.0; nb + 1], vec![0.0; nb + 1]);
    for i in 0..nb {
        s1[i + 1] = s1[i] + lv[i];
        s2[i + 1] = s2[i] + lv[i] * lv[i];
    }
    let mean = |i: usize, j: usize| (s1[j] - s1[i]) / (j - i) as f64;
    let cost = |i: usize, j: usize| {
        let s = s1[j] - s1[i];
        (s2[j] - s2[i]) - s * s / (j - i) as f64
    };
    // A new section costs what a 3 dB step between two 8 s halves does
    let lambda = 9.0 * 16.0;
    let allowed = |i: usize, j: usize| {
        let len = j - i;
        len >= MIN_LOUD || (len >= MIN_STILL && mean(i, j) <= floor_db + 1.0) || (i == 0 && j == nb)
    };
    let mut best = vec![f64::INFINITY; nb + 1];
    let mut from = vec![0usize; nb + 1];
    best[0] = 0.0;
    for j in 1..=nb {
        for i in 0..j {
            if !best[i].is_finite() || !allowed(i, j) {
                continue;
            }
            let c = best[i] + cost(i, j) + lambda;
            if c < best[j] {
                best[j] = c;
                from[j] = i;
            }
        }
    }
    let mut cuts = vec![nb];
    let mut j = nb;
    while j > 0 {
        j = from[j];
        cuts.push(j);
    }
    cuts.reverse();
    cuts.dedup();
    let mut segs: Vec<(usize, usize)> = cuts.windows(2).map(|w| (w[0], w[1])).collect();
    loop {
        let closest = (1..segs.len())
            .map(|i| {
                (
                    i,
                    (mean(segs[i].0, segs[i].1) - mean(segs[i - 1].0, segs[i - 1].1)).abs(),
                )
            })
            .min_by(|x, y| x.1.total_cmp(&y.1));
        match closest {
            Some((i, d)) if d < 3.0 => {
                segs[i - 1].1 = segs[i].1;
                segs.remove(i);
            }
            _ => break,
        }
    }
    // Each boundary onto the sharpest step near it
    let mut bounds: Vec<f64> = vec![0.0];
    for i in 1..segs.len() {
        let t = segs[i].0 as f64 * 0.25;
        let up = mean(segs[i].0, segs[i].1) > mean(segs[i - 1].0, segs[i - 1].1);
        let lo = (t - 2.0).max(bounds.last().unwrap() + 1.0);
        let hi = (t + 2.0).min(segs[i].1 as f64 * 0.25 - 1.0);
        let mut bt = t;
        if hi > lo {
            let mut best_step = f64::NEG_INFINITY;
            let mut x = lo;
            while x <= hi {
                let step = level(x, x + 1.0) - level(x - 1.0, x);
                let step = if up { step } else { -step };
                if step > best_step {
                    best_step = step;
                    bt = x;
                }
                x += 1.0 / FPS;
            }
        }
        if up {
            if let Some(h) = hits.iter().find(|h| (h.t - bt).abs() <= 0.3) {
                bt = h.t;
            }
        }
        bounds.push(bt);
    }
    bounds.push(duration);
    bounds.windows(2).map(|w| (w[0], w[1])).collect()
}

// ---------------------------------------------------------------------------
// The felt report
// ---------------------------------------------------------------------------

/// m:ss
pub fn clock(t: f64) -> String {
    let s = t.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Two significant figures (three below 0.01 of a unit would be noise).
fn sig2(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return "0".into();
    }
    let digits = (1 - x.abs().log10().floor() as i32).max(0) as usize;
    format!("{:.*}", digits, x)
}

fn g(x: f64) -> String {
    format!("{} g", sig2(x / GRAVITY))
}

fn join_names(names: &[String]) -> String {
    const SHOWN: usize = 4;
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        n if n <= SHOWN => format!("{} and {}", names[..n - 1].join(", "), names[n - 1]),
        n => format!(
            "{} and {} more",
            names[..SHOWN - 1].join(", "),
            n - (SHOWN - 1)
        ),
    }
}

fn spacing(iv: f64) -> String {
    let bpm = 60.0 / iv;
    if (60.0..=210.0).contains(&bpm) {
        format!("one every {:.3} s ({:.0} bpm)", iv, bpm)
    } else {
        format!("one every {:.3} s ({:.0} a minute)", iv, bpm)
    }
}

/// The plain-text felt report, time-ordered.
pub fn report(felt: &Felt, input: &str) -> String {
    let s = &felt.settings;
    let c = &felt.coupling;
    let sm = &felt.summary;
    let ground = s.venue.ground();
    let mut o = String::new();
    let _ = writeln!(o, "Felt under the sub: {input}");
    let _ = writeln!(o, "Where: {}.", c.describe);
    let fs_pa = s.rig.full_scale_pa();
    let _ = writeln!(
        o,
        "Rig: LR4 crossover at {:.0} Hz, {} sub rolling off below {:.0} Hz, {} level: \
         {:.0} dB peak at 1 m for a full-scale signal ({:.0} Pa; {:.0} N of air pressure \
         on {}; the cones push {:.0} N).",
        s.rig.crossover_hz,
        if s.rig.sub_order == 2 {
            "sealed"
        } else {
            "vented"
        },
        s.rig.sub_hz,
        s.rig.level_name,
        s.rig.spl_db,
        fs_pa,
        fs_pa * c.load_n_per_pa,
        ground,
        CONE_KG_PER_M2 * s.rig.full_scale_volume_accel(),
    );
    let chain = s.rig.filter().then(c.filter());
    let _ = writeln!(
        o,
        "Transfer, full scale to you: {} at 40 Hz, {} at 50 Hz, {} at 63 Hz (m/s²); the \
         ground rings {:.0} ms to -20 dB on its own.",
        sig2(chain.gain(40.0) * fs_pa),
        sig2(chain.gain(50.0) * fs_pa),
        sig2(chain.gain(63.0) * fs_pa),
        c.ring_20db() * 1000.0
    );
    let dbfs = |x: f64| 20.0 * x.max(1e-9).log10();
    if s.normalize && felt.gain != 1.0 {
        let _ = writeln!(
            o,
            "Input: bounce normalized to -1 dBFS ({:+.1} dB); the sub feed peaks at {:.1} dBFS.",
            dbfs(felt.gain),
            dbfs(felt.sub_peak)
        );
    } else {
        let _ = writeln!(
            o,
            "Input: peak {:.1} dBFS as is; the sub feed peaks at {:.1} dBFS.",
            dbfs(felt.input_peak),
            dbfs(felt.sub_peak)
        );
    }
    let _ = writeln!(o);
    for sec in &felt.sections {
        let span = format!("{}-{}", clock(sec.start), clock(sec.end));
        let change = match sec.change_db {
            Some(d) if d >= 0.0 => format!(", up {:.0} dB", d),
            Some(d) => format!(", down {:.0} dB", -d),
            None => String::new(),
        };
        if sec.still {
            // Under a tenth of the threshold nothing reaches the body;
            // above that the ground moves (under the tracks that can
            // reach the sub, or a ring from before), not enough to feel
            let what = if sec.peak_wk < 0.1 * PERCEPTION_PEAK {
                match join_names(&sec.tracks) {
                    who if who.is_empty() => format!("nothing reaches {ground}"),
                    who => format!("nothing from {who} reaches {ground}"),
                }
            } else {
                match join_names(&sec.low_tracks) {
                    _ if sec.peak_wk >= PERCEPTION_PEAK => format!(
                        "{ground} is nearly still; only the tail of what came before \
                         crosses the perception threshold"
                    ),
                    who if who.is_empty() => {
                        format!("{ground} moves, but below the perception threshold")
                    }
                    who => format!(
                        "{who} {} {ground}, but below the perception threshold",
                        if sec.low_tracks.len() == 1 {
                            "moves"
                        } else {
                            "move"
                        }
                    ),
                }
            };
            let _ = writeln!(
                o,
                "{span} still{change}: {what} (Wk peak {} m/s², {}% of the threshold).",
                sig2(sec.peak_wk),
                sig2(100.0 * sec.peak_wk / PERCEPTION_PEAK)
            );
            continue;
        }
        let hits = &felt.hits[sec.hits.clone()];
        let mut line = format!("{span}{change}: ");
        if let Some(first) = hits.first() {
            let mut count: Vec<(String, usize)> = Vec::new();
            for h in hits {
                for src in &h.sources {
                    match count.iter_mut().find(|c| &c.0 == src) {
                        Some(c) => c.1 += 1,
                        None => count.push((src.clone(), 1)),
                    }
                }
            }
            // the tracks on (nearly) every hit; failing that, on most
            let on = |share: f64| -> Vec<String> {
                count
                    .iter()
                    .filter(|c| c.1 as f64 >= share * hits.len() as f64)
                    .map(|c| c.0.clone())
                    .collect()
            };
            let from = match on(0.9) {
                v if v.is_empty() => on(0.5),
                v => v,
            };
            let lead = if felt.hits.first().map(|h| h.t) == Some(first.t) {
                "first hit"
            } else {
                "hits from"
            };
            let _ = write!(
                line,
                "{lead} {}{}, a {} shove at {} peak",
                clock(first.t),
                if from.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", join_names(&from))
                },
                first
                    .hz
                    .map(|f| format!("{:.0} Hz", f))
                    .unwrap_or_else(|| "low".into()),
                g(first.peak),
            );
            match first.ring_ms {
                Some(r) if first.ring_extrapolated => {
                    let _ = write!(line, " that rings for about {:.0} ms", r);
                }
                Some(r) => {
                    let _ = write!(line, " that rings for {:.0} ms", r);
                }
                None => {
                    let _ = write!(line, " that holds into the next");
                }
            }
            match sec.interval {
                Some(iv) if hits.len() > 2 => {
                    let _ = write!(line, ", then {}", spacing(iv));
                }
                _ => {}
            }
            let _ = write!(
                line,
                "; {} hit{}",
                hits.len(),
                if hits.len() == 1 { "" } else { "s" }
            );
            let loudest = hits
                .iter()
                .max_by(|x, y| x.peak.total_cmp(&y.peak))
                .unwrap();
            if hits.len() > 1 && loudest.peak > first.peak * 1.12 {
                let _ = write!(
                    line,
                    ", the hardest {} at {}",
                    g(loudest.peak),
                    clock(loudest.t)
                );
            }
            line.push('.');
            if hits.len() > 1 {
                match (sec.between_rms_wk, sec.between_hz) {
                    (Some(r), Some(f)) if r * 2f64.sqrt() >= PERCEPTION_PEAK => {
                        let peak = hits
                            .iter()
                            .filter_map(|h| h.between_peak)
                            .fold(0.0, f64::max);
                        let _ = write!(
                            line,
                            " Between hits {} hums at {:.0} Hz, up to {}.",
                            ground,
                            f,
                            g(peak)
                        );
                    }
                    (Some(r), Some(f)) if r * 2f64.sqrt() >= PERCEPTION_PEAK / 4.0 => {
                        let _ = write!(
                            line,
                            " Between hits a faint {:.0} Hz hum, under the threshold.",
                            f
                        );
                    }
                    _ => {
                        let _ = write!(line, " Between hits it goes still.");
                    }
                }
            }
        } else {
            let _ = write!(
                line,
                "a steady {} hum, no separate hits, {} peak.",
                sec.hz
                    .map(|f| format!("{:.0} Hz", f))
                    .unwrap_or_else(|| "low".into()),
                g(sec.peak)
            );
        }
        let _ = write!(
            line,
            " Wk {} m/s² r.m.s. (loudest second {}), {}; {:.1}x the perception threshold at peak.",
            sig2(sec.rms_wk),
            sig2(sec.mtvv),
            comfort(sec.mtvv),
            sec.peak_wk / PERCEPTION_PEAK
        );
        let _ = writeln!(o, "{line}");
    }
    let felt_hits = felt.hits.iter().filter(|h| h.felt).count();
    let times: Vec<f64> = felt.hits.iter().map(|h| h.t).collect();
    let _ = writeln!(o);
    let _ = writeln!(
        o,
        "Whole piece ({}): peak {} ({} m/s²), Wk peak {} m/s², Wk r.m.s. {} m/s², MTVV {} m/s² \
         (1 s), {} m/s² (125 ms), VDV {} m/s^1.75, crest factor {:.1}; felt {:.0}% of the time; \
         {} hits ({} above the threshold){}.",
        clock(sm.duration),
        g(sm.peak),
        sig2(sm.peak),
        sig2(sm.peak_wk),
        sig2(sm.rms_wk),
        sig2(sm.mtvv),
        sig2(sm.max_125ms),
        sig2(sm.vdv),
        sm.crest,
        sm.felt_fraction * 100.0,
        felt.hits.len(),
        felt_hits,
        match regular_interval(&times) {
            Some((iv, frac)) if frac >= 0.5 => format!(", mostly {}", spacing(iv)),
            _ => String::new(),
        }
    );
    o
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

fn jnum(x: f64) -> String {
    if !x.is_finite() {
        return "null".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    let s = format!("{:.5e}", x);
    // trim to a plain number when it reads better
    let v: f64 = s.parse().unwrap_or(x);
    let plain = format!("{}", v);
    if plain.len() <= s.len() {
        plain
    } else {
        s
    }
}

fn jopt(x: Option<f64>) -> String {
    x.map(jnum).unwrap_or_else(|| "null".into())
}

fn jstr(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn jarr(v: &[f32]) -> String {
    let mut o = String::with_capacity(v.len() * 8);
    o.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        let x = *x as f64;
        if x == 0.0 {
            o.push('0');
        } else {
            let _ = write!(o, "{}", format!("{:.4e}", x).parse::<f64>().unwrap_or(x));
        }
    }
    o.push(']');
    o
}

/// Every number the report is made from, for hypha to compute with.
pub fn json(felt: &Felt, input: &str, report_text: &str) -> String {
    let s = &felt.settings;
    let c = &felt.coupling;
    let sm = &felt.summary;
    let fs_pa = s.rig.full_scale_pa();
    let mut o = String::new();
    o.push_str("{\n");
    let _ = writeln!(o, "  \"input\": {},", jstr(input));
    let _ = writeln!(o, "  \"duration_s\": {},", jnum(sm.duration));
    let _ = writeln!(o, "  \"analysis_rate_hz\": {},", jnum(FS));
    // settings
    let venue_params: Vec<(&str, f64)> = match &s.venue {
        Venue::Field(f) => vec![
            ("vs_m_s", f.vs),
            ("density_kg_m3", f.rho),
            ("poisson", f.nu),
            ("material_q", f.q),
            ("layer_m", f.layer),
            ("vs_base_m_s", f.vs_base),
            ("density_base_kg_m3", f.rho_base),
            ("depth_m", f.depth),
            ("distance_m", f.distance),
        ],
        Venue::Club(k) => vec![
            ("slab_m", k.thickness),
            ("concrete_e_pa", k.e),
            ("concrete_poisson", k.nu),
            ("concrete_density_kg_m3", k.rho),
            ("subgrade_modulus_n_m3", k.k),
            ("subgrade_density_kg_m3", k.rho_sub),
            ("subgrade_vs_m_s", k.vs_sub),
            ("subgrade_poisson", k.nu_sub),
            ("distance_m", k.distance),
        ],
        Venue::House(t) => vec![
            ("span_m", t.span),
            ("width_m", t.width),
            ("joist_b_m", t.joist_b),
            ("joist_h_m", t.joist_h),
            ("joist_spacing_m", t.spacing),
            ("timber_e_pa", t.e),
            ("mass_kg_m2", t.mass),
            ("damping_ratio", t.zeta),
            ("distance_m", t.distance),
        ],
    };
    let obj = |v: &[(&str, f64)]| {
        v.iter()
            .map(|(k, x)| format!("{}: {}", jstr(k), jnum(*x)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let _ = writeln!(o, "  \"settings\": {{");
    let _ = writeln!(o, "    \"venue\": {},", jstr(s.venue.name()));
    let _ = writeln!(o, "    \"venue_params\": {{{}}},", obj(&venue_params));
    let _ = writeln!(o, "    \"level\": {},", jstr(&s.rig.level_name));
    let _ = writeln!(o, "    \"spl_peak_db_1m\": {},", jnum(s.rig.spl_db));
    let _ = writeln!(o, "    \"full_scale_pa_1m\": {},", jnum(fs_pa));
    let _ = writeln!(
        o,
        "    \"full_scale_ground_load_n\": {},",
        jnum(fs_pa * c.load_n_per_pa)
    );
    let _ = writeln!(
        o,
        "    \"full_scale_volume_accel_m3_s2\": {},",
        jnum(s.rig.full_scale_volume_accel())
    );
    let _ = writeln!(
        o,
        "    \"full_scale_cone_force_n\": {},",
        jnum(CONE_KG_PER_M2 * s.rig.full_scale_volume_accel())
    );
    let _ = writeln!(o, "    \"crossover_hz\": {},", jnum(s.rig.crossover_hz));
    let _ = writeln!(o, "    \"sub_hz\": {},", jnum(s.rig.sub_hz));
    let _ = writeln!(
        o,
        "    \"sub_alignment\": {},",
        jstr(if s.rig.sub_order == 2 {
            "sealed, 2nd order"
        } else {
            "vented, 4th order"
        })
    );
    let _ = writeln!(o, "    \"normalize\": {}", s.normalize);
    let _ = writeln!(o, "  }},");
    // coupling
    let chain = s.rig.filter().then(c.filter());
    let weighted = chain.clone().then(wk());
    let transfer: Vec<String> = [
        20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0,
    ]
    .iter()
    .map(|&f| {
        format!(
            "{{\"hz\": {}, \"ground_ms2_per_pa\": {}, \"full_scale_ms2\": {}, \"full_scale_wk_ms2\": {}}}",
            jnum(f),
            jnum(c.filter().gain(f)),
            jnum(chain.gain(f) * fs_pa),
            jnum(weighted.gain(f) * fs_pa)
        )
    })
    .collect();
    let _ = writeln!(o, "  \"coupling\": {{");
    let _ = writeln!(o, "    \"describe\": {},", jstr(&c.describe));
    let _ = writeln!(o, "    \"static_m_per_pa\": {},", jnum(c.static_m_per_pa));
    let _ = writeln!(o, "    \"resonance_hz\": {},", jnum(c.f0));
    let _ = writeln!(o, "    \"q\": {},", jnum(c.q));
    let _ = writeln!(o, "    \"ring_20db_s\": {},", jnum(c.ring_20db()));
    let _ = writeln!(o, "    \"load_n_per_pa\": {},", jnum(c.load_n_per_pa));
    let _ = writeln!(o, "    \"derived\": {{{}}},", obj(&c.derived));
    let _ = writeln!(o, "    \"transfer\": [{}]", transfer.join(", "));
    let _ = writeln!(o, "  }},");
    let _ = writeln!(
        o,
        "  \"input_level\": {{\"peak\": {}, \"gain\": {}, \"sub_peak\": {}}},",
        jnum(felt.input_peak),
        jnum(felt.gain),
        jnum(felt.sub_peak)
    );
    let comfort_bands: Vec<String> = COMFORT
        .iter()
        .map(|(lo, hi, name)| {
            format!(
                "{{\"from\": {}, \"to\": {}, \"reaction\": {}}}",
                jnum(*lo),
                jnum(*hi),
                jstr(name)
            )
        })
        .collect();
    let _ = writeln!(
        o,
        "  \"thresholds\": {{\"perception_wk_peak\": {}, \"comfort_wk_rms\": [{}]}},",
        jnum(PERCEPTION_PEAK),
        comfort_bands.join(", ")
    );
    let times: Vec<f64> = felt.hits.iter().map(|h| h.t).collect();
    let tempo = regular_interval(&times).filter(|x| x.1 >= 0.5);
    let _ = writeln!(o, "  \"summary\": {{");
    for (k, v) in [
        ("peak_ms2", sm.peak),
        ("peak_g", sm.peak / GRAVITY),
        ("peak_wk_ms2", sm.peak_wk),
        ("rms_wk_ms2", sm.rms_wk),
        ("mtvv_1s_ms2", sm.mtvv),
        ("max_rms_125ms_ms2", sm.max_125ms),
        ("vdv_ms175", sm.vdv),
        ("crest_factor", sm.crest),
        ("felt_fraction", sm.felt_fraction),
        ("hits", felt.hits.len() as f64),
    ] {
        let _ = writeln!(o, "    {}: {},", jstr(k), jnum(v));
    }
    let _ = writeln!(o, "    \"interval_s\": {},", jopt(tempo.map(|t| t.0)));
    let _ = writeln!(o, "    \"bpm\": {},", jopt(tempo.map(|t| 60.0 / t.0)));
    let _ = writeln!(o, "    \"comfort\": {}", jstr(&comfort(sm.mtvv)));
    let _ = writeln!(o, "  }},");
    let f = &felt.frames;
    let _ = writeln!(o, "  \"envelope\": {{");
    let _ = writeln!(o, "    \"fps\": {},", jnum(FPS));
    let _ = writeln!(o, "    \"wk_rms\": {},", jarr(&f.wk_rms));
    let _ = writeln!(o, "    \"wk_rms_125ms\": {},", jarr(&f.wk_rms_125ms));
    let _ = writeln!(o, "    \"wk_rms_1s\": {},", jarr(&f.wk_rms_1s));
    let _ = writeln!(o, "    \"wk_peak\": {},", jarr(&f.wk_peak));
    let _ = writeln!(o, "    \"peak\": {}", jarr(&f.peak));
    let _ = writeln!(o, "  }},");
    let hits: Vec<String> = felt
        .hits
        .iter()
        .map(|h| {
            format!(
                "    {{\"t\": {}, \"peak_ms2\": {}, \"peak_g\": {}, \"peak_wk_ms2\": {}, \"hz\": {}, \
                 \"ring_ms\": {}, \"ring_extrapolated\": {}, \"between_wk_rms_ms2\": {}, \
                 \"between_peak_g\": {}, \"between_hz\": {}, \"felt\": {}, \"sources\": [{}]}}",
                jnum(h.t),
                jnum(h.peak),
                jnum(h.peak / GRAVITY),
                jnum(h.peak_wk),
                jopt(h.hz),
                jopt(h.ring_ms),
                h.ring_extrapolated,
                jopt(h.between_rms_wk),
                jopt(h.between_peak.map(|p| p / GRAVITY)),
                jopt(h.between_hz),
                h.felt,
                h.sources.iter().map(|s| jstr(s)).collect::<Vec<_>>().join(", ")
            )
        })
        .collect();
    let _ = writeln!(o, "  \"hits\": [\n{}\n  ],", hits.join(",\n"));
    let secs: Vec<String> = felt
        .sections
        .iter()
        .map(|x| {
            format!(
                "    {{\"start\": {}, \"end\": {}, \"still\": {}, \"rms_wk_ms2\": {}, \"mtvv_1s_ms2\": {}, \
                 \"peak_g\": {}, \"peak_wk_ms2\": {}, \"change_db\": {}, \"hits\": {}, \
                 \"interval_s\": {}, \"bpm\": {}, \"between_wk_rms_ms2\": {}, \"between_hz\": {}, \
                 \"hz\": {}, \"comfort\": {}, \"tracks\": [{}], \"low_tracks\": [{}]}}",
                jnum(x.start),
                jnum(x.end),
                x.still,
                jnum(x.rms_wk),
                jnum(x.mtvv),
                jnum(x.peak / GRAVITY),
                jnum(x.peak_wk),
                jopt(x.change_db),
                x.hits.len(),
                jopt(x.interval),
                jopt(x.interval.map(|i| 60.0 / i)),
                jopt(x.between_rms_wk),
                jopt(x.between_hz),
                jopt(x.hz),
                jstr(&comfort(x.mtvv)),
                x.tracks.iter().map(|s| jstr(s)).collect::<Vec<_>>().join(", "),
                x.low_tracks.iter().map(|s| jstr(s)).collect::<Vec<_>>().join(", ")
            )
        })
        .collect();
    let _ = writeln!(o, "  \"sections\": [\n{}\n  ],", secs.join(",\n"));
    let _ = writeln!(o, "  \"report\": {}", jstr(report_text));
    o.push_str("}\n");
    o
}

// ---------------------------------------------------------------------------
// The command
// ---------------------------------------------------------------------------

pub const USAGE: &str = "\
patina feel <song.song | file.wav | file.mid> [options]

  --venue field|club|house   where you are (default field)
                             field: buried in soft soil under a festival sub
                             club:  lying on a concrete slab on grade
                             house: in a suspended timber floor
  --level festival|club|home|<dB>
                             peak SPL at 1 m a full-scale signal reaches
                             (default: festival 143, club 135, home 112 dB)
  --depth M                  field: how deep you are (default 0.3)
  --distance M               sideways from the sub (field 0, club 1.0;
                             house: from midspan, 0)
  --crossover HZ             LR4 crossover to the tops (default 90; house 80)
  --sub-hz HZ                the sub's low-frequency limit (default 32; house 25)
  --sealed | --vented        the sub's box (house sealed, others vented)
  --out PREFIX               write PREFIX.feel.json (default: next to the input)
  --json PATH                the JSON at an exact path
  --no-normalize             songs: keep the engine's gain, not the -1 dBFS bounce
  --patch NAME               the voice a MIDI file plays (as with --play)
";

fn parse_f64(args: &[String], i: usize, name: &str) -> Result<f64, String> {
    args.get(i + 1)
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("{name} needs a number"))
}

pub fn cli(args: &[String]) -> Result<(), String> {
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return if args.is_empty() {
            Err("feel needs a song or a WAV".into())
        } else {
            Ok(())
        };
    }
    let input = args[0].clone();
    let mut venue = "field".to_string();
    let mut level: Option<String> = None;
    let (mut depth, mut distance) = (None, None);
    let (mut crossover, mut sub_hz, mut order) = (None, None, None);
    let (mut out, mut json_path, mut patch) = (None, None, None);
    let mut normalize = true;
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        let value = || {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{a} needs a value"))
        };
        match a {
            "--venue" => venue = value()?,
            "--level" => level = Some(value()?),
            "--depth" => depth = Some(parse_f64(args, i, a)?),
            "--distance" => distance = Some(parse_f64(args, i, a)?),
            "--crossover" => crossover = Some(parse_f64(args, i, a)?),
            "--sub-hz" => sub_hz = Some(parse_f64(args, i, a)?),
            "--out" => out = Some(value()?),
            "--json" => json_path = Some(value()?),
            "--patch" => patch = Some(value()?),
            "--sealed" | "--vented" => {
                order = Some(if a == "--sealed" { 2 } else { 4 });
                i += 1;
                continue;
            }
            "--no-normalize" => {
                normalize = false;
                i += 1;
                continue;
            }
            _ => return Err(format!("feel: unknown option '{a}'\n{USAGE}")),
        }
        i += 2;
    }
    let mut v = match venue.as_str() {
        "field" => Venue::Field(Field::default()),
        "club" => Venue::Club(Slab::default()),
        "house" => Venue::House(Timber::default()),
        other => return Err(format!("feel: --venue {other}: field, club or house")),
    };
    match &mut v {
        Venue::Field(f) => {
            if let Some(d) = depth {
                if !(0.05..=f.layer - 0.05).contains(&d) {
                    return Err(format!(
                        "--depth {d}: the model holds from 0.05 m to {:.2} m (inside the soft layer)",
                        f.layer - 0.05
                    ));
                }
                f.depth = d;
            }
            if let Some(d) = distance {
                if !(0.0..=2.0).contains(&d) {
                    return Err(format!(
                        "--distance {d}: the field's near-field model holds to 2 m from the sub"
                    ));
                }
                f.distance = d;
            }
        }
        Venue::Club(s) => {
            if depth.is_some() {
                return Err("--depth applies to the field; on the slab use --distance".into());
            }
            if let Some(d) = distance {
                if !(0.0..=3.0).contains(&d) {
                    return Err(format!("--distance {d}: the slab model holds to 3 m"));
                }
                s.distance = d;
            }
        }
        Venue::House(t) => {
            if depth.is_some() {
                return Err("--depth applies to the field; in the floor use --distance".into());
            }
            if let Some(d) = distance {
                if !(0.0..t.span / 2.0).contains(&d) {
                    return Err(format!(
                        "--distance {d}: from midspan to under {:.2} m (the joists' bearing)",
                        t.span / 2.0
                    ));
                }
                t.distance = d;
            }
        }
    }
    let mut settings = Settings::new(v);
    settings.normalize = normalize;
    if let Some(l) = level {
        match level_preset(&l) {
            Some(db) => {
                settings.rig.spl_db = db;
                settings.rig.level_name = l;
            }
            None => {
                let db: f64 = l
                    .trim_end_matches("dB")
                    .parse()
                    .map_err(|_| format!("--level {l}: festival, club, home or a dB value"))?;
                if !(60.0..=160.0).contains(&db) {
                    return Err(format!("--level {db}: 60 to 160 dB peak at 1 m"));
                }
                settings.rig.spl_db = db;
                settings.rig.level_name = "custom".into();
            }
        }
    }
    if let Some(x) = crossover {
        if !(40.0..=250.0).contains(&x) {
            return Err(format!("--crossover {x}: 40 to 250 Hz"));
        }
        settings.rig.crossover_hz = x;
    }
    if let Some(x) = sub_hz {
        if !(10.0..=80.0).contains(&x) {
            return Err(format!("--sub-hz {x}: 10 to 80 Hz"));
        }
        settings.rig.sub_hz = x;
    }
    if let Some(o) = order {
        settings.rig.sub_order = o;
    }

    let lower = input.to_ascii_lowercase();
    let stem = input
        .rsplit_once('.')
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| input.clone());
    let prefix = out.unwrap_or(stem);
    let json_path = json_path.unwrap_or_else(|| format!("{prefix}.feel.json"));

    let felt = if lower.ends_with(".wav") {
        let data = crate::sampler::load_wav_stereo(&input)?;
        eprintln!(
            "feel: {} ({:.1} s at {} Hz)",
            input,
            data.frames() as f64 / data.rate as f64,
            data.rate
        );
        settings.normalize = false;
        let frames = data.left.iter().copied().zip(data.right.iter().copied());
        feel(frames, data.rate as f64, settings, None)
    } else {
        let song = crate::song::load_song_with_patch(&input, patch.as_deref())?;
        crate::voice_manager::prefer_performance_cores();
        eprintln!(
            "feel: rendering {} in memory ({} events)",
            input,
            song.events.len()
        );
        let notes = SongNotes::from_song(&song);
        let frames =
            crate::song::render_offline(&song, 48000.0).with_card_lanes(crate::song::card_lanes());
        feel(frames, 48000.0, settings, Some(&notes))
    };
    let text = report(&felt, &input);
    print!("{text}");
    std::fs::write(&json_path, json(&felt, &input, &text))
        .map_err(|e| format!("{json_path}: {e}"))?;
    println!("\nWrote {json_path}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a test signal (m/s²) straight through the digital Wk filter and
    /// return the r.m.s. ratio after it settles.
    fn wk_gain_digital(f: f64) -> f64 {
        let mut w = wk().run(FS);
        let n = (FS * (20.0 + 20.0 / f)) as usize;
        let settle = (FS * 12.0) as usize;
        let (mut sin, mut sout) = (0.0, 0.0);
        for i in 0..n {
            let x = (TAU * f * i as f64 / FS).sin();
            let y = w.process(x);
            if i >= settle {
                sin += x * x;
                sout += y * y;
            }
        }
        (sout / sin).sqrt()
    }

    #[test]
    fn wk_matches_the_standards_table() {
        for &(f, table) in &WK_TABLE {
            let exact = wk().gain(f);
            let digital = wk_gain_digital(f);
            assert!(
                (exact / table - 1.0).abs() < 0.02,
                "Wk({f} Hz) = {exact:.4}, ISO 2631-1 Table 3 says {table}"
            );
            assert!(
                (digital / exact - 1.0).abs() < 0.002,
                "digital Wk({f} Hz) = {digital:.4}, analog {exact:.4}"
            );
        }
    }

    #[test]
    fn a_50_hz_sine_gives_its_weighted_rms() {
        // 1 m/s² peak at 50 Hz: Wk r.m.s. = 0.707 x Wk(50) = 0.174 m/s²
        let a: Vec<f32> = (0..(FS * 4.0) as usize)
            .map(|i| (TAU * 50.0 * i as f64 / FS).sin() as f32)
            .collect();
        let mut w = wk().run(FS);
        let aw: Vec<f32> = a.iter().map(|x| w.process(*x as f64) as f32).collect();
        let p = prefix_squares(&aw);
        let rms = rms_between(&p, (FS * 2.0) as usize, aw.len());
        let expect = FRAC_1_SQRT_2 * 0.246;
        assert!((rms / expect - 1.0).abs() < 0.01, "{rms} vs {expect}");
    }

    #[test]
    fn kei_matches_tables_and_integrates_to_one() {
        // kei(x) = Im K0(x e^{iπ/4}), checked against the integral
        // K0(z) = ∫ exp(-z cosh t) dt: kei(1) = -0.4949946365,
        // kei(5) = 0.0111875865, kei(10) = -3.075246e-4
        assert!((kei(1.0) + 0.494_994_636_5).abs() < 1e-9, "{}", kei(1.0));
        assert!((kei(5.0) - 0.011_187_586_5).abs() < 1e-9, "{}", kei(5.0));
        assert!((kei(10.0) + 3.075_246e-4).abs() < 1e-9, "{}", kei(10.0));
        assert!((kei(0.0) + PI / 4.0).abs() < 1e-12);
        // series and asymptote agree where they meet
        for x in [9.0, 10.0, 11.0] {
            assert!((kei_series(x) - kei_asymptote(x)).abs() < 1e-10, "{x}");
        }
        // ∫ x kei(x) dx = -1: a uniform pressure on a Winkler plate is p/k
        let s = Slab::default();
        let d = s.e * s.thickness.powi(3) / (12.0 * (1.0 - s.nu * s.nu));
        let ell = (d / s.k).powf(0.25);
        let w = integrate_load(
            0.3,
            14.0 * ell,
            |_| 1.0,
            |rho| -(ell * ell / (TAU * d)) * kei(rho / ell),
        );
        assert!(
            (w * s.k - 1.0).abs() < 0.01,
            "uniform load gives {} of p/k",
            w * s.k
        );
    }

    #[test]
    fn boussinesq_sum_matches_the_closed_form() {
        // A uniform pressure on a disc of radius a, on the axis at depth z
        let (g, nu, a, z) = (40e6, 0.35, 1.0, 0.3);
        let kernel = |rho: f64| {
            let r = rho.hypot(z);
            (2.0 * (1.0 - nu) + z * z / (r * r)) / (4.0 * PI * g * r)
        };
        let num = integrate_load(0.0, a, |_| 1.0, kernel);
        let r = (a * a + z * z).sqrt();
        let exact = (2.0 * (1.0 - nu) * (r - z) + z - z * z / r) / (2.0 * g);
        assert!((num / exact - 1.0).abs() < 1e-3, "{num} vs {exact}");
    }

    #[test]
    fn venues_differ_the_way_the_physics_says() {
        let field = Venue::Field(Field::default()).coupling();
        let club = Venue::Club(Slab::default()).coupling();
        let house = Venue::House(Timber::default()).coupling();
        // the wooden floor rings longest, the slab not at all
        assert!(house.ring_20db() > 4.0 * field.ring_20db());
        assert!(field.ring_20db() > club.ring_20db());
        assert!(club.q < 0.5, "a slab on grade is overdamped");
        // each boosts near its own resonance: house by ~Q, field by ~Q
        let peak_near = |c: &Coupling| {
            let f = c.filter();
            let hf = f.gain(2000.0);
            (c.f0 * 0.8..c.f0 * 1.3)
                .step_by_f(0.1)
                .into_iter()
                .fold(0.0f64, |m, x| m.max(f.gain(x)))
                / hf
        };
        assert!(peak_near(&house) > 12.0, "house {}", peak_near(&house));
        assert!(peak_near(&field) > 2.0, "field {}", peak_near(&field));
        assert!(peak_near(&club) < 1.05, "club {}", peak_near(&club));
        // the field's resonance sits in the kick band
        assert!((35.0..50.0).contains(&field.f0), "{}", field.f0);
        // the digital venue filter rings as the formula says
        for c in [&field, &club, &house] {
            // a 12-cycle burst at f0, then the free decay after it stops
            let mut flt = c.filter().run(FS);
            let burst = (12.0 * FS / c.f0) as usize;
            let ir: Vec<f64> = (0..burst + (FS * 2.0) as usize)
                .map(|i| {
                    let x = if i < burst {
                        (TAU * c.f0 * i as f64 / FS).sin()
                    } else {
                        0.0
                    };
                    flt.process(x)
                })
                .collect();
            let per = (FS / c.f0).ceil() as usize;
            let env: Vec<f64> = (0..ir.len() - per)
                .map(|i| ir[i..i + per].iter().fold(0.0f64, |m, x| m.max(x.abs())))
                .collect();
            let kp = burst - per;
            let k20 = (kp..env.len()).find(|&k| env[k] < env[kp] / 10.0).unwrap();
            let measured = (k20 - kp) as f64 / FS;
            assert!(
                (measured - c.ring_20db()).abs() < 0.1 * c.ring_20db() + per as f64 / FS,
                "{} ring {measured} vs {}",
                c.describe,
                c.ring_20db()
            );
        }
    }

    trait StepBy {
        fn step_by_f(self, step: f64) -> Vec<f64>;
    }
    impl StepBy for std::ops::Range<f64> {
        fn step_by_f(self, step: f64) -> Vec<f64> {
            let mut v = Vec::new();
            let mut x = self.start;
            while x < self.end {
                v.push(x);
                x += step;
            }
            v
        }
    }

    #[test]
    fn level_scales_acceleration_linearly() {
        let sine: Vec<(f32, f32)> = (0..48000 * 3)
            .map(|i| {
                let x = (TAU * 50.0 * i as f64 / 48000.0).sin() as f32 * 0.5;
                (x, x)
            })
            .collect();
        let run = |db: f64| {
            let mut s = Settings::new(Venue::Field(Field::default()));
            s.rig.spl_db = db;
            s.normalize = false;
            feel(sine.iter().copied(), 48000.0, s, None).summary.mtvv
        };
        let (a, b) = (run(130.0), run(136.0206));
        assert!((b / a - 2.0).abs() < 1e-3, "{a} {b}");
    }

    #[test]
    fn a_full_scale_sine_lands_where_the_transfer_function_says() {
        // 50 Hz at full scale: the felt peak is |rig x venue| x full-scale Pa
        let sine: Vec<(f32, f32)> = (0..48000 * 4)
            .map(|i| {
                let x = (TAU * 50.0 * i as f64 / 48000.0).sin() as f32;
                (x, x)
            })
            .collect();
        for venue in [
            Venue::Field(Field::default()),
            Venue::Club(Slab::default()),
            Venue::House(Timber::default()),
        ] {
            let mut s = Settings::new(venue);
            s.normalize = false;
            let expect =
                s.rig.filter().then(s.venue.coupling().filter()).gain(50.0) * s.rig.full_scale_pa();
            let expect_wk = expect * wk().gain(50.0) * FRAC_1_SQRT_2;
            let felt = feel(sine.iter().copied(), 48000.0, s, None);
            let n = felt.accel.len();
            let tail = &felt.accel[n / 2..];
            let peak = tail.iter().fold(0.0f32, |m, x| m.max(x.abs())) as f64;
            assert!(
                (peak / expect - 1.0).abs() < 0.01,
                "peak {peak} vs {expect}"
            );
            let p = prefix_squares(&felt.weighted);
            let rms = rms_between(&p, n / 2, n);
            assert!(
                (rms / expect_wk - 1.0).abs() < 0.01,
                "Wk {rms} vs {expect_wk}"
            );
        }
    }

    fn render(text: &str) -> (Felt, crate::song::Song) {
        let song = crate::song::parse_song_text(text).unwrap();
        let notes = SongNotes::from_song(&song);
        let felt = feel(
            crate::song::render_offline(&song, 48000.0),
            48000.0,
            Settings::new(Venue::Field(Field::default())),
            Some(&notes),
        );
        (felt, song)
    }

    #[test]
    fn hats_and_pads_feel_still() {
        let (hats, _) = render("bpm 140\ntail 1\ntrack hats kit=909\n(CH CH OH CH)x16\n");
        assert!(
            hats.summary.peak_wk < PERCEPTION_PEAK,
            "hats reach {} m/s² Wk",
            hats.summary.peak_wk
        );
        assert!(hats.hits.is_empty(), "{} hits from hats", hats.hits.len());
        let (pad, _) =
            render("bpm 100\ntail 2\ntrack pad patch=aurora\n[C4 E4 G4]:8 [A3 C4 E4]:8\n");
        assert!(
            pad.summary.peak_wk < PERCEPTION_PEAK,
            "a pad reaches {} m/s² Wk",
            pad.summary.peak_wk
        );
        assert!(pad.sections.iter().all(|s| s.still));
    }

    #[test]
    fn a_140_bpm_kick_is_felt_on_every_beat() {
        let (felt, _) = render(
            "bpm 140\ntail 1\ntrack kick kit=909\n(BD BD BD BD)x8\n\
             track hats kit=909\n(R CH R CH)x8\n",
        );
        assert_eq!(
            felt.hits.len(),
            32,
            "{:?}",
            felt.hits.iter().map(|h| h.t).collect::<Vec<_>>()
        );
        let t: Vec<f64> = felt.hits.iter().map(|h| h.t).collect();
        let (iv, frac) = regular_interval(&t).unwrap();
        assert!((iv - 60.0 / 140.0).abs() < 0.0002, "spacing {iv}: {t:?}");
        assert_eq!(frac, 1.0, "{t:?}");
        // each spacing within 3 ms: the felt attack wobbles about ±1.5 ms
        // with the modelled 909's analog drift (its click lands within
        // 0.4 ms of the beat in the raw bounce)
        for w in t.windows(2) {
            assert!((w[1] - w[0] - 60.0 / 140.0).abs() < 0.003, "{w:?}");
        }
        assert!(t[0].abs() < 0.005, "the first kick lands at {}", t[0]);
        assert!(felt
            .hits
            .iter()
            .all(|h| h.felt && h.sources.contains(&"kick".to_string())));
        let hz = felt.hits[5].hz.unwrap();
        assert!((35.0..90.0).contains(&hz), "a 909 kick lands at {hz} Hz");
    }

    #[test]
    fn kicks_over_a_held_bass_are_all_found() {
        // A sine bass about as strong as the kick once weighted: the whole
        // signal barely rises on each kick, and the kick's tail beats
        // against the bass (F1 D1 A1 E1, 11-19 Hz from the kick's 55-62 Hz)
        let (felt, _) = render(
            "bpm 140\ntail 1\ntrack kick kit=909\n(BD BD BD BD)x8\n\
             track bass patch=deepsine vel=0.8 gain=1.4\nF1:8 D1:8 A1:8 E1:8\n",
        );
        let t: Vec<f64> = felt.hits.iter().map(|h| h.t).collect();
        assert_eq!(t.len(), 32, "{t:?}");
        for (i, x) in t.iter().enumerate() {
            let beat = i as f64 * 60.0 / 140.0;
            assert!((x - beat).abs() < 0.02, "hit {i} at {x}, beat at {beat}");
        }
        let (iv, _) = regular_interval(&t).unwrap();
        assert!((iv - 60.0 / 140.0).abs() < 0.002, "spacing {iv}");
    }

    #[test]
    fn sections_mark_level_steps_and_silence() {
        // 6 s silence, 6 s at one level, 6 s at +12 dB, 6 s silence
        let rate = 48000.0;
        let frames: Vec<(f32, f32)> = (0..(rate * 24.0) as usize)
            .map(|i| {
                let t = i as f64 / rate;
                let amp = match t {
                    t if t < 6.0 => 0.0,
                    t if t < 12.0 => 0.12,
                    t if t < 18.0 => 0.48,
                    _ => 0.0,
                };
                let x = (amp * (TAU * 45.0 * t).sin()) as f32;
                (x, x)
            })
            .collect();
        let mut s = Settings::new(Venue::Field(Field::default()));
        s.normalize = false;
        let felt = feel(frames.into_iter(), rate, s, None);
        let b: Vec<f64> = felt.sections.iter().map(|x| x.start).collect();
        assert_eq!(
            b.len(),
            4,
            "{:?}",
            felt.sections
                .iter()
                .map(|x| (x.start, x.end, x.rms_wk))
                .collect::<Vec<_>>()
        );
        for (got, want) in b.iter().zip([0.0, 6.0, 12.0, 18.0]) {
            assert!((got - want).abs() < 0.3, "boundary {got} vs {want}");
        }
        assert!(
            felt.sections[0].still && felt.sections[3].still,
            "{:?}",
            felt.sections
                .iter()
                .map(|x| (x.start, x.end, x.peak_wk))
                .collect::<Vec<_>>()
        );
        assert!((felt.sections[2].change_db.unwrap() - 12.0).abs() < 1.0);
    }

    #[test]
    fn json_is_well_formed() {
        let (felt, _) = render("bpm 120\ntail 1\ntrack kick kit=909\n(BD R BD R)x2\n");
        let text = report(&felt, "test \"song\"");
        let j = json(&felt, "test \"song\"", &text);
        // balanced outside strings, every key we promise
        let (mut depth, mut in_str, mut esc) = (0i32, false, false);
        for ch in j.chars() {
            match (in_str, esc, ch) {
                (true, true, _) => esc = false,
                (true, false, '\\') => esc = true,
                (true, false, '"') => in_str = false,
                (false, _, '"') => in_str = true,
                (false, _, '{' | '[') => depth += 1,
                (false, _, '}' | ']') => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0);
        }
        assert_eq!(depth, 0);
        for key in [
            "\"envelope\"",
            "\"hits\"",
            "\"sections\"",
            "\"settings\"",
            "\"wk_rms_1s\"",
        ] {
            assert!(j.contains(key), "{key}");
        }
        assert!(!j.contains("NaN") && !j.contains("inf"));
    }
}
