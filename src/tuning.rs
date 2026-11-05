// Free tuning: what pitch a song note plays.
//
// A tuning is a scale (degrees in cents above its root, the first 0) and
// the period it repeats at (an octave, 1200 cents, unless the scale says
// otherwise: Bohlen-Pierce repeats at the tritave). Keys map to scale steps
// one for one, up from the root key, which keeps its equal-tempered pitch
// against A4 (`a4=`, 440 Hz by default); that is the Scala keyboard
// mapping. A 12-note scale lands on the keys you would expect (D is the
// third degree from C); a 19- or 31-note one runs up the keyboard a step a
// key, so an octave spans 19 or 31 keys.
//
// The engine plays any pitch as a MIDI key and an offset in cents: the
// song parser picks the key (the nearest one free on the track, so two
// microtonal notes never share a key's note-off) and sends the offset with
// the note. 12-TET with A4 at 440 sends no offsets at all.

/// A scale and where it sits: key `root` sounds `root_hz`.
#[derive(Clone, Debug, PartialEq)]
pub struct Tuning {
    /// degree pitches in cents above the root; degrees[0] = 0
    pub degrees: Vec<f64>,
    /// cents per period (1200 for an octave)
    pub period: f64,
    pub root: u8,
    pub root_hz: f64,
}

/// Built-in scales, by name.
pub const BUILTINS: &[&str] = &[
    "12tet",
    "just",
    "pythagorean",
    "meantone",
    "werckmeister3",
    "edoN",
    "bp",
];

/// 12-TET frequency of MIDI key `key` with A4 at `a4` Hz.
pub fn equal_hz(key: f64, a4: f64) -> f64 {
    a4 * ((key - 69.0) / 12.0).exp2()
}

fn ratio_cents(r: f64) -> f64 {
    1200.0 * r.log2()
}

/// A built-in scale: (degrees, period).
fn builtin(name: &str) -> Option<(Vec<f64>, f64)> {
    let from_ratios = |rs: &[f64]| rs.iter().map(|&r| ratio_cents(r)).collect::<Vec<_>>();
    Some(match name {
        "12tet" => ((0..12).map(|k| 100.0 * k as f64).collect(), 1200.0),
        // five-limit just intonation on the root
        "just" => (
            from_ratios(&[
                1.0,
                16.0 / 15.0,
                9.0 / 8.0,
                6.0 / 5.0,
                5.0 / 4.0,
                4.0 / 3.0,
                45.0 / 32.0,
                3.0 / 2.0,
                8.0 / 5.0,
                5.0 / 3.0,
                9.0 / 5.0,
                15.0 / 8.0,
            ]),
            1200.0,
        ),
        // pure fifths from Eb to G#
        "pythagorean" => (
            from_ratios(&[
                1.0,
                256.0 / 243.0,
                9.0 / 8.0,
                32.0 / 27.0,
                81.0 / 64.0,
                4.0 / 3.0,
                729.0 / 512.0,
                3.0 / 2.0,
                128.0 / 81.0,
                27.0 / 16.0,
                16.0 / 9.0,
                243.0 / 128.0,
            ]),
            1200.0,
        ),
        // quarter-comma meantone: fifths of 5^(1/4), Eb to G#
        "meantone" => {
            let fifth = ratio_cents(5f64.powf(0.25));
            // fifths from the root for each semitone (C=0, C#=7, D=2, ...)
            let chain = [0, 7, 2, -3, 4, -1, 6, 1, 8, 3, -2, 5];
            (
                chain
                    .iter()
                    .map(|&n| (n as f64 * fifth).rem_euclid(1200.0))
                    .collect(),
                1200.0,
            )
        }
        "werckmeister3" => (
            vec![
                0.0, 90.225, 192.18, 294.135, 390.225, 498.045, 588.27, 696.09, 792.18, 888.27,
                996.09, 1092.18,
            ],
            1200.0,
        ),
        // Bohlen-Pierce, equal: 13 steps to the tritave (3:1)
        "bp" => {
            let tritave = ratio_cents(3.0);
            ((0..13).map(|k| k as f64 * tritave / 13.0).collect(), tritave)
        }
        _ => {
            let n: usize = name.strip_prefix("edo")?.parse().ok()?;
            if !(1..=1200).contains(&n) {
                return None;
            }
            ((0..n).map(|k| 1200.0 * k as f64 / n as f64).collect(), 1200.0)
        }
    })
}

/// One pitch of a scale list: cents if it has a '.', else a ratio
/// (`3/2`, `2`), as Scala reads them.
fn scale_pitch(tok: &str) -> Result<f64, String> {
    let tok = tok.trim();
    let bad = || format!("invalid scale pitch '{}'", tok);
    let v = if tok.contains('.') {
        tok.parse::<f64>().map_err(|_| bad())?
    } else if let Some((a, b)) = tok.split_once('/') {
        let (a, b) = (
            a.trim().parse::<f64>().map_err(|_| bad())?,
            b.trim().parse::<f64>().map_err(|_| bad())?,
        );
        if !(a > 0.0 && b > 0.0) {
            return Err(bad());
        }
        ratio_cents(a / b)
    } else {
        let a = tok.parse::<f64>().map_err(|_| bad())?;
        if a <= 0.0 {
            return Err(bad());
        }
        ratio_cents(a)
    };
    if !v.is_finite() {
        return Err(bad());
    }
    Ok(v)
}

/// A scale from its pitches after the root, the last one the period (the
/// Scala convention): `[204, 386.3, 702, 1200]` is a four-note scale.
fn from_list(pitches: &[f64]) -> Result<(Vec<f64>, f64), String> {
    let (&period, steps) = pitches
        .split_last()
        .ok_or("a scale needs at least its period")?;
    if !(period > 0.0) {
        return Err(format!("the period {} must be above the root", period));
    }
    let mut degrees = vec![0.0];
    for &c in steps {
        if !(c > *degrees.last().unwrap() && c < period) {
            return Err(format!(
                "scale pitches must rise from the root to the period, got {} after {}",
                c,
                degrees.last().unwrap()
            ));
        }
        degrees.push(c);
    }
    Ok((degrees, period))
}

/// A Scala .scl file: `!` comment lines, a description line, the count,
/// then that many pitches (cents with a '.', else ratios), anything after
/// a pitch on its line ignored.
pub fn parse_scl(text: &str) -> Result<(Vec<f64>, f64), String> {
    let mut lines = text.lines().filter(|l| !l.trim_start().starts_with('!'));
    lines.next().ok_or("an empty .scl file")?;
    let count: usize = lines
        .next()
        .and_then(|l| l.split_whitespace().next())
        .and_then(|t| t.parse().ok())
        .ok_or("a .scl file needs its note count on the second line")?;
    let mut pitches = Vec::with_capacity(count);
    for l in lines {
        let Some(tok) = l.split_whitespace().next() else {
            continue;
        };
        pitches.push(scale_pitch(tok)?);
        if pitches.len() == count {
            break;
        }
    }
    if pitches.len() != count || count == 0 {
        return Err(format!(
            "the .scl file promises {} pitches and lists {}",
            count,
            pitches.len()
        ));
    }
    from_list(&pitches)
}

impl Tuning {
    /// 12-TET with A4 at 440 Hz: what every track plays unless told.
    pub fn standard() -> Self {
        Tuning {
            degrees: (0..12).map(|k| 100.0 * k as f64).collect(),
            period: 1200.0,
            root: 60,
            root_hz: equal_hz(60.0, 440.0),
        }
    }

    /// `tuning NAME BASE [root=NOTE] [a4=HZ]`'s BASE and options, with
    /// `read` loading a `scl=` file. BASE is a built-in name, `cents=` or
    /// `ratios=` (comma-separated pitches after the root, the last the
    /// period), or `scl=FILE`.
    pub fn parse(
        tokens: &[&str],
        parse_note: impl Fn(&str) -> Result<u8, String>,
        read: impl Fn(&str) -> Result<String, String>,
    ) -> Result<Self, String> {
        let mut scale: Option<(Vec<f64>, f64)> = None;
        let mut root: u8 = 60;
        let mut a4 = 440.0f64;
        for &tok in tokens {
            if let Some(v) = tok.strip_prefix("root=") {
                root = parse_note(v)?;
            } else if let Some(v) = tok.strip_prefix("a4=") {
                a4 = v
                    .parse::<f64>()
                    .ok()
                    .filter(|x| x.is_finite() && (100.0..=1000.0).contains(x))
                    .ok_or_else(|| format!("a4={} must be a frequency in 100..1000 Hz", v))?;
            } else if scale.is_some() {
                return Err(format!("tuning: one scale per tuning, then '{}'?", tok));
            } else if let Some(v) = tok.strip_prefix("cents=") {
                let p = v.split(',').map(scale_pitch_cents).collect::<Result<Vec<_>, _>>()?;
                scale = Some(from_list(&p)?);
            } else if let Some(v) = tok.strip_prefix("ratios=") {
                let p = v
                    .split(',')
                    .map(|t| {
                        if t.contains('.') {
                            // a decimal ratio, not cents
                            t.parse::<f64>()
                                .ok()
                                .filter(|r| *r > 0.0 && r.is_finite())
                                .map(ratio_cents)
                                .ok_or_else(|| format!("invalid ratio '{}'", t))
                        } else {
                            scale_pitch(t)
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                scale = Some(from_list(&p)?);
            } else if let Some(path) = tok.strip_prefix("scl=") {
                scale = Some(parse_scl(&read(path)?).map_err(|e| format!("{}: {}", path, e))?);
            } else {
                scale = Some(builtin(tok).ok_or_else(|| {
                    format!(
                        "unknown tuning '{}': a built-in ({}), cents=..., ratios=... or scl=FILE",
                        tok,
                        BUILTINS.join(" ")
                    )
                })?);
            }
        }
        let (degrees, period) = scale.ok_or("tuning needs a scale: just, edo19, cents=..., scl=...")?;
        Ok(Tuning {
            degrees,
            period,
            root,
            root_hz: equal_hz(root as f64, a4),
        })
    }

    /// A built-in by name on C with A4 at 440 (a track's `tuning=just`).
    pub fn named(name: &str) -> Option<Self> {
        let (degrees, period) = builtin(name)?;
        Some(Tuning {
            degrees,
            period,
            root: 60,
            root_hz: equal_hz(60.0, 440.0),
        })
    }

    /// The pitch key `key` plays, in cents above the root.
    pub fn cents(&self, key: i32) -> f64 {
        let n = self.degrees.len() as i32;
        let steps = key - self.root as i32;
        let (q, r) = (steps.div_euclid(n), steps.rem_euclid(n));
        q as f64 * self.period + self.degrees[r as usize]
    }

    /// The frequency key `key` plays.
    pub fn hz(&self, key: i32) -> f64 {
        self.root_hz * (self.cents(key) / 1200.0).exp2()
    }
}

/// `cents=` list items: plain numbers are cents here (no ratio reading).
fn scale_pitch_cents(t: &str) -> Result<f64, String> {
    t.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("invalid cents '{}'", t))
}

/// The MIDI key nearest `hz` (12-TET, A4 440) and the cents from that
/// key's pitch to `hz`; None outside the keyboard.
pub fn nearest_key(hz: f64) -> Option<(u8, f64)> {
    if !(hz > 0.0 && hz.is_finite()) {
        return None;
    }
    let k = 69.0 + 12.0 * (hz / 440.0).log2();
    let key = k.round();
    if !(0.0..=127.0).contains(&key) {
        return None;
    }
    Some((key as u8, 100.0 * (k - key)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(s: &str) -> Result<u8, String> {
        match s {
            "C4" => Ok(60),
            "D4" => Ok(62),
            "A4" => Ok(69),
            _ => Err(format!("note {s}")),
        }
    }

    fn no_files(_: &str) -> Result<String, String> {
        Err("no files".into())
    }

    /// Every built-in maps the root to itself and the keys to their
    /// degrees: just intonation's E is 5/4 over C (386.31 cents), not
    /// 400; edo19 puts nineteen keys in an octave; bp's thirteenth step
    /// is the tritave.
    #[test]
    fn scales_put_the_keys_where_they_belong() {
        let just = Tuning::named("just").unwrap();
        assert_eq!(just.cents(60), 0.0);
        assert!((just.cents(64) - 1200.0 * (5.0f64 / 4.0).log2()).abs() < 1e-9);
        assert!((just.cents(67 + 12) - (1200.0 + 1200.0 * 1.5f64.log2())).abs() < 1e-9);
        assert!((just.cents(59) - (1200.0 * (15.0f64 / 8.0).log2() - 1200.0)).abs() < 1e-9);
        let edo19 = Tuning::named("edo19").unwrap();
        assert!((edo19.cents(60 + 19) - 1200.0).abs() < 1e-9);
        assert!((edo19.cents(61) - 1200.0 / 19.0).abs() < 1e-9);
        let bp = Tuning::named("bp").unwrap();
        assert!((bp.hz(60 + 13) / bp.hz(60) - 3.0).abs() < 1e-12);
        let mt = Tuning::named("meantone").unwrap();
        // quarter-comma: the major third is pure 5/4
        assert!((mt.cents(64) - 1200.0 * 1.25f64.log2()).abs() < 1e-9);
        let std = Tuning::standard();
        for k in 0..128 {
            assert!((std.hz(k) - equal_hz(k as f64, 440.0)).abs() < 1e-9 * std.hz(k));
        }
    }

    /// root= keeps that key at its equal-tempered pitch, a4= moves
    /// everything; cents= and ratios= read Scala-style lists.
    #[test]
    fn tuning_lines_parse() {
        let t = Tuning::parse(&["just", "root=D4"], note, no_files).unwrap();
        assert!((t.hz(62) - equal_hz(62.0, 440.0)).abs() < 1e-9);
        assert!((t.hz(66) / t.hz(62) - 1.25).abs() < 1e-12);
        let t = Tuning::parse(&["12tet", "a4=432"], note, no_files).unwrap();
        assert!((t.hz(69) - 432.0).abs() < 1e-9);
        let t = Tuning::parse(&["cents=240,480,720,960,1200"], note, no_files).unwrap();
        assert_eq!(t.degrees.len(), 5);
        assert!((t.cents(65) - 1200.0).abs() < 1e-9);
        let t = Tuning::parse(&["ratios=9/8,5/4,3/2,2"], note, no_files).unwrap();
        assert!((t.hz(62) / t.hz(60) - 1.25).abs() < 1e-12);
        assert!((t.hz(64) / t.hz(60) - 2.0).abs() < 1e-12);
        for bad in [
            &["edo0"][..],
            &["nonsense"],
            &["cents=300,200,1200"],
            &["cents=100,1200,1100"],
            &["just", "edo12"],
            &["a4=5", "just"],
            &["scl=x.scl"],
            &[],
        ] {
            assert!(Tuning::parse(bad, note, no_files).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn scala_files_read() {
        let scl = "! slendro.scl\n!\nA slendro\n 5\n!\n 240.0\n 480.0 ! a comment\n 720.0\n 960.0\n 2/1\n";
        let (d, p) = parse_scl(scl).unwrap();
        assert_eq!(d, vec![0.0, 240.0, 480.0, 720.0, 960.0]);
        assert!((p - 1200.0).abs() < 1e-9);
        assert!(parse_scl("x\n3\n100.0\n200.0\n").is_err());
    }

    #[test]
    fn the_nearest_key_and_its_cents_are_exact() {
        assert_eq!(nearest_key(440.0), Some((69, 0.0)));
        let (k, c) = nearest_key(220.0 * (13.7f64 / 1200.0).exp2()).unwrap();
        assert_eq!(k, 57);
        assert!((c - 13.7).abs() < 1e-9);
        let (k, c) = nearest_key(equal_hz(60.0, 440.0) * (-49.0f64 / 1200.0).exp2()).unwrap();
        assert_eq!(k, 60);
        assert!((c + 49.0).abs() < 1e-9);
        assert_eq!(nearest_key(0.0), None);
        assert_eq!(nearest_key(1e6), None);
    }
}
