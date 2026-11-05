//! The seismograph: felt intensity over time as a PNG, for a reader who
//! sees images. Log scale of Wk-weighted acceleration with the perception
//! threshold and the ISO 2631-1 comfort bands, every hit marked above it.
//! A minimal encoder (4-bit palette, stored deflate blocks, crc32, adler32)
//! and a 5x7 bitmap font keep it free of dependencies.

use super::{clock, Felt, Venue, FPS, PERCEPTION_PEAK};

const W: usize = 1580;
const H: usize = 640;
/// Plot area
const X0: usize = 96;
const X1: usize = W - 350;
const Y0: usize = 150;
const Y1: usize = H - 56;
/// Log scale, m/s²
const LO: f64 = 0.001;
const HI: f64 = 4.0;

// Palette: chart surface and inks, then one blue for the three traces
// (light to dark: peak, 125 ms r.m.s., 1 s r.m.s.)
const SURFACE: u8 = 0;
const INK: u8 = 1;
const INK2: u8 = 2;
const MUTED: u8 = 3;
const GRID: u8 = 4;
const AXIS: u8 = 5;
const PEAK: u8 = 6;
const FAST: u8 = 7;
const SLOW: u8 = 8;
const STILL: u8 = 9;
const PALETTE: [[u8; 3]; 10] = [
    [0xfc, 0xfc, 0xfb],
    [0x0b, 0x0b, 0x0b],
    [0x52, 0x51, 0x4e],
    [0x89, 0x87, 0x81],
    [0xe1, 0xe0, 0xd9],
    [0xc3, 0xc2, 0xb7],
    [0xcd, 0xe2, 0xfb],
    [0x86, 0xb6, 0xef],
    [0x10, 0x42, 0x81],
    [0xf0, 0xef, 0xec],
];

struct Canvas {
    px: Vec<u8>,
}

impl Canvas {
    fn new() -> Self {
        Canvas {
            px: vec![SURFACE; W * H],
        }
    }

    fn set(&mut self, x: usize, y: usize, c: u8) {
        if x < W && y < H {
            self.px[y * W + x] = c;
        }
    }

    fn rect(&mut self, x0: usize, y0: usize, x1: usize, y1: usize, c: u8) {
        for y in y0..y1.min(H) {
            for x in x0..x1.min(W) {
                self.px[y * W + x] = c;
            }
        }
    }

    fn hline(&mut self, x0: usize, x1: usize, y: usize, c: u8, dash: usize) {
        for x in x0..x1 {
            if dash == 0 || (x - x0) / dash % 2 == 0 {
                self.set(x, y, c);
            }
        }
    }

    fn vline(&mut self, x: usize, y0: usize, y1: usize, c: u8, dash: usize) {
        for y in y0..y1 {
            if dash == 0 || (y - y0) / dash % 2 == 0 {
                self.set(x, y, c);
            }
        }
    }

    /// Text at 2x (12 px per character), top-left at (x, y).
    fn text(&mut self, x: usize, y: usize, s: &str, c: u8) -> usize {
        let mut cx = x;
        for ch in s.chars() {
            let rows = FONT
                .iter()
                .find(|g| g.0 == ch)
                .or_else(|| FONT.iter().find(|g| g.0 == '?'))
                .unwrap()
                .1;
            for (r, bits) in rows.iter().enumerate() {
                for col in 0..5 {
                    if bits & (0x10 >> col) != 0 {
                        self.rect(cx + 2 * col, y + 2 * r, cx + 2 * col + 2, y + 2 * r + 2, c);
                    }
                }
            }
            cx += 12;
        }
        cx
    }

    fn text_right(&mut self, x: usize, y: usize, s: &str, c: u8) {
        let w = 12 * s.chars().count();
        self.text(x.saturating_sub(w), y, s, c);
    }
}

fn y_of(v: f64) -> usize {
    let f = (v.max(LO).min(HI) / LO).log10() / (HI / LO).log10();
    Y1 - (f * (Y1 - Y0) as f64).round() as usize
}

/// The seismograph of `felt` as PNG bytes.
pub fn render(felt: &Felt, input: &str) -> Vec<u8> {
    let mut c = Canvas::new();
    let dur = felt.summary.duration.max(1e-3);
    let x_of = |t: f64| X0 + ((t / dur).clamp(0.0, 1.0) * (X1 - X0) as f64).round() as usize;

    // Title, setting and legend
    let name = input.rsplit('/').next().unwrap_or(input);
    c.text(X0, 14, &format!("Felt under the sub: {name}"), INK);
    let rig = &felt.settings.rig;
    let place = match &felt.settings.venue {
        Venue::Field(f) => format!("buried {:.2} m in a festival field", f.depth),
        Venue::Club(s) => format!("on a concrete slab, {:.1} m from the sub", s.distance),
        Venue::House(t) => format!("in a timber floor, {:.1} m from midspan", t.distance),
    };
    c.text(
        X0,
        40,
        &format!(
            "{place}; {} level, {:.0} dB peak at 1 m. Wk-weighted vertical acceleration, m/s²",
            rig.level_name, rig.spl_db
        ),
        INK2,
    );
    let mut lx = X0;
    let swatch = |c: &mut Canvas, x: usize, col: u8| c.rect(x, 72, x + 24, 84, col);
    swatch(&mut c, lx, PEAK);
    lx = c.text(lx + 32, 72, "peak", INK2) + 28;
    swatch(&mut c, lx, FAST);
    lx = c.text(lx + 32, 72, "125 ms r.m.s.", INK2) + 28;
    c.rect(lx, 77, lx + 24, 80, SLOW);
    lx = c.text(lx + 32, 72, "1 s r.m.s. (the comfort bands)", INK2) + 28;
    c.rect(lx + 11, 70, lx + 13, 86, INK);
    lx = c.text(lx + 32, 72, "hit", INK2) + 28;
    c.rect(lx, 72, lx + 24, 84, STILL);
    c.text(lx + 32, 72, "still", INK2);

    // Still sections washed, section boundaries as hairlines
    for s in &felt.sections {
        if s.still {
            c.rect(x_of(s.start), Y0, x_of(s.end), Y1, STILL);
        }
    }

    // Decades and the time axis
    for (v, label) in [(0.001, "0.001"), (0.01, "0.01"), (0.1, "0.1"), (1.0, "1")] {
        let y = y_of(v);
        c.hline(X0, X1, y, GRID, 0);
        c.text_right(X0 - 10, y - 7, label, MUTED);
    }
    let step = [5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0]
        .into_iter()
        .find(|s| dur / s <= 12.0)
        .unwrap_or(600.0);
    let mut t = 0.0;
    while t <= dur + 1e-9 {
        let x = x_of(t);
        c.vline(x, Y0, Y1, GRID, 0);
        let label = clock(t);
        c.text(x.saturating_sub(6 * label.len()), Y1 + 12, &label, MUTED);
        t += step;
    }
    for s in felt.sections.iter().skip(1) {
        c.vline(x_of(s.start), Y0, Y1, AXIS, 4);
    }

    // The traces, one pixel column at a time: peak and 125 ms r.m.s. as
    // the column's top, the 1 s r.m.s. as a 3 px line
    let fr = &felt.frames;
    let n = fr.wk_peak.len();
    let col_max = |v: &[f32], x: usize| {
        let f0 = (((x - X0) as f64 / (X1 - X0) as f64) * dur * FPS) as usize;
        let f1 = ((((x + 1 - X0) as f64 / (X1 - X0) as f64) * dur * FPS) as usize).max(f0 + 1);
        v[f0.min(n)..f1.min(n)]
            .iter()
            .fold(0.0f32, |m, x| m.max(*x)) as f64
    };
    let mut last: Option<usize> = None;
    for x in X0..X1 {
        if n == 0 {
            break;
        }
        let p = col_max(&fr.wk_peak, x);
        if p > LO {
            c.vline(x, y_of(p), Y1, PEAK, 0);
        }
        let r = col_max(&fr.wk_rms_125ms, x);
        if r > LO {
            c.vline(x, y_of(r), Y1, FAST, 0);
        }
        let y = y_of(col_max(&fr.wk_rms_1s, x));
        let (a, b) = match last {
            Some(l) => (l.min(y), l.max(y)),
            None => (y, y),
        };
        c.rect(x, a.saturating_sub(1), x + 1, b + 2, SLOW);
        last = Some(y);
    }
    c.hline(X0, X1, Y1, AXIS, 0);

    // Threshold and comfort bands, labelled at the right
    let marks: [(f64, &str, u8); 6] = [
        (PERCEPTION_PEAK, "perceived (Wk peak)", INK2),
        (0.315, "a little uncomfortable", MUTED),
        (0.5, "fairly uncomfortable", MUTED),
        (0.8, "uncomfortable", MUTED),
        (1.25, "very uncomfortable", MUTED),
        (2.0, "extremely uncomfortable", MUTED),
    ];
    for (v, label, col) in marks {
        let y = y_of(v);
        c.hline(X0, X1, y, col, if col == INK2 { 0 } else { 6 });
        c.text(X1 + 10, y - 7, &format!("{v} {label}"), col);
    }

    // Hits: a tick each above the plot, dark when felt
    for h in &felt.hits {
        let x = x_of(h.t);
        c.vline(x, Y0 - 22, Y0 - 6, if h.felt { INK } else { MUTED }, 0);
    }
    c.text(X1 + 10, Y0 - 22, &format!("{} hits", felt.hits.len()), INK2);
    encode(&c.px)
}

// ---------------------------------------------------------------------------
// PNG: 4-bit palette, zlib with stored deflate blocks

fn crc32(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *t = c;
    }
    !bytes.iter().fold(!0u32, |c, b| {
        table[((c ^ *b as u32) & 0xff) as usize] ^ (c >> 8)
    })
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in bytes.chunks(5552) {
        for x in chunk {
            a += *x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(65535).collect()
    };
    for (i, b) in blocks.iter().enumerate() {
        z.push((i + 1 == blocks.len()) as u8);
        let len = b.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(data).to_be_bytes());
    z
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn encode(px: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(H * (1 + W / 2));
    for row in px.chunks(W) {
        raw.push(0); // filter: none
        for pair in row.chunks(2) {
            raw.push(pair[0] << 4 | pair.get(1).copied().unwrap_or(0));
        }
    }
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(W as u32).to_be_bytes());
    ihdr.extend_from_slice(&(H as u32).to_be_bytes());
    ihdr.extend_from_slice(&[4, 3, 0, 0, 0]); // 4-bit, palette
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"PLTE", &PALETTE.concat());
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// 5x7 glyphs, one byte a row, bit 4 the leftmost column.
const FONT: &[(char, [u8; 7])] = &[
    (' ', [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
    ('!', [0x04, 0x04, 0x04, 0x04, 0x00, 0x00, 0x04]),
    ('"', [0x0a, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00]),
    ('#', [0x0a, 0x0a, 0x1f, 0x0a, 0x1f, 0x0a, 0x0a]),
    ('%', [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03]),
    ('\'', [0x04, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00]),
    ('(', [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02]),
    (')', [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08]),
    ('*', [0x00, 0x04, 0x15, 0x0e, 0x15, 0x04, 0x00]),
    ('+', [0x00, 0x04, 0x04, 0x1f, 0x04, 0x04, 0x00]),
    (',', [0x00, 0x00, 0x00, 0x00, 0x0c, 0x04, 0x08]),
    ('-', [0x00, 0x00, 0x00, 0x1f, 0x00, 0x00, 0x00]),
    ('.', [0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x0c]),
    ('/', [0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x00]),
    ('0', [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e]),
    ('1', [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e]),
    ('2', [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f]),
    ('3', [0x1f, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0e]),
    ('4', [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02]),
    ('5', [0x1f, 0x10, 0x1e, 0x01, 0x01, 0x11, 0x0e]),
    ('6', [0x06, 0x08, 0x10, 0x1e, 0x11, 0x11, 0x0e]),
    ('7', [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08]),
    ('8', [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e]),
    ('9', [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x02, 0x0c]),
    (':', [0x00, 0x0c, 0x0c, 0x00, 0x0c, 0x0c, 0x00]),
    (';', [0x00, 0x0c, 0x0c, 0x00, 0x0c, 0x04, 0x08]),
    ('<', [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02]),
    ('=', [0x00, 0x00, 0x1f, 0x00, 0x1f, 0x00, 0x00]),
    ('>', [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08]),
    ('?', [0x0e, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04]),
    ('A', [0x0e, 0x11, 0x11, 0x11, 0x1f, 0x11, 0x11]),
    ('B', [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e]),
    ('C', [0x0e, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0e]),
    ('D', [0x1c, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1c]),
    ('E', [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f]),
    ('F', [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10]),
    ('G', [0x0e, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0f]),
    ('H', [0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11]),
    ('I', [0x0e, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0e]),
    ('J', [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0c]),
    ('K', [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11]),
    ('L', [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f]),
    ('M', [0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11]),
    ('N', [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11]),
    ('O', [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e]),
    ('P', [0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10]),
    ('Q', [0x0e, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0d]),
    ('R', [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11]),
    ('S', [0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e]),
    ('T', [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
    ('U', [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e]),
    ('V', [0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04]),
    ('W', [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0a]),
    ('X', [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11]),
    ('Y', [0x11, 0x11, 0x11, 0x0a, 0x04, 0x04, 0x04]),
    ('Z', [0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f]),
    ('[', [0x0e, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0e]),
    (']', [0x0e, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0e]),
    ('_', [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1f]),
    ('a', [0x00, 0x00, 0x0e, 0x01, 0x0f, 0x11, 0x0f]),
    ('b', [0x10, 0x10, 0x16, 0x19, 0x11, 0x11, 0x1e]),
    ('c', [0x00, 0x00, 0x0e, 0x10, 0x10, 0x11, 0x0e]),
    ('d', [0x01, 0x01, 0x0d, 0x13, 0x11, 0x11, 0x0f]),
    ('e', [0x00, 0x00, 0x0e, 0x11, 0x1f, 0x10, 0x0e]),
    ('f', [0x06, 0x09, 0x08, 0x1c, 0x08, 0x08, 0x08]),
    ('g', [0x00, 0x0f, 0x11, 0x11, 0x0f, 0x01, 0x0e]),
    ('h', [0x10, 0x10, 0x16, 0x19, 0x11, 0x11, 0x11]),
    ('i', [0x04, 0x00, 0x0c, 0x04, 0x04, 0x04, 0x0e]),
    ('j', [0x02, 0x00, 0x06, 0x02, 0x02, 0x12, 0x0c]),
    ('k', [0x10, 0x10, 0x12, 0x14, 0x18, 0x14, 0x12]),
    ('l', [0x0c, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0e]),
    ('m', [0x00, 0x00, 0x1a, 0x15, 0x15, 0x11, 0x11]),
    ('n', [0x00, 0x00, 0x16, 0x19, 0x11, 0x11, 0x11]),
    ('o', [0x00, 0x00, 0x0e, 0x11, 0x11, 0x11, 0x0e]),
    ('p', [0x00, 0x00, 0x1e, 0x11, 0x1e, 0x10, 0x10]),
    ('q', [0x00, 0x00, 0x0d, 0x13, 0x0f, 0x01, 0x01]),
    ('r', [0x00, 0x00, 0x16, 0x19, 0x10, 0x10, 0x10]),
    ('s', [0x00, 0x00, 0x0e, 0x10, 0x0e, 0x01, 0x1e]),
    ('t', [0x08, 0x08, 0x1c, 0x08, 0x08, 0x09, 0x06]),
    ('u', [0x00, 0x00, 0x11, 0x11, 0x11, 0x13, 0x0d]),
    ('v', [0x00, 0x00, 0x11, 0x11, 0x11, 0x0a, 0x04]),
    ('w', [0x00, 0x00, 0x11, 0x11, 0x15, 0x15, 0x0a]),
    ('x', [0x00, 0x00, 0x11, 0x0a, 0x04, 0x0a, 0x11]),
    ('y', [0x00, 0x00, 0x11, 0x11, 0x0f, 0x01, 0x0e]),
    ('z', [0x00, 0x00, 0x1f, 0x02, 0x04, 0x08, 0x1f]),
    ('²', [0x0c, 0x12, 0x04, 0x08, 0x1e, 0x00, 0x00]),
    ('·', [0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00]),
    ('–', [0x00, 0x00, 0x00, 0x0e, 0x00, 0x00, 0x00]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_match_their_references() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn stored_blocks_carry_the_data_back() {
        let data: Vec<u8> = (0..150_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let z = zlib_stored(&data);
        assert_eq!(&z[..2], &[0x78, 0x01]);
        assert_eq!((z[0] as u32 * 256 + z[1] as u32) % 31, 0);
        let (mut i, mut back) = (2, Vec::new());
        loop {
            let last = z[i] & 1;
            let len = u16::from_le_bytes([z[i + 1], z[i + 2]]) as usize;
            let nlen = u16::from_le_bytes([z[i + 3], z[i + 4]]);
            assert_eq!(nlen, !(len as u16));
            back.extend_from_slice(&z[i + 5..i + 5 + len]);
            i += 5 + len;
            if last == 1 {
                break;
            }
        }
        assert_eq!(back, data);
        assert_eq!(&z[i..], &adler32(&data).to_be_bytes());
    }

    #[test]
    fn a_felt_kick_draws_a_whole_png() {
        let rate = 48000.0;
        let frames = (0..(rate * 4.0) as usize).map(|i| {
            let t = i as f64 / rate;
            let x = (0.5
                * (-t.rem_euclid(0.5) * 20.0).exp()
                * (std::f64::consts::TAU * 55.0 * t).sin()) as f32;
            (x, x)
        });
        let felt = super::super::feel(
            frames,
            rate,
            super::super::Settings::new(Venue::Field(Default::default())),
            None,
        );
        let png = render(&felt, "kick.wav");
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            W as u32
        );
        assert_eq!(
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
            H as u32
        );
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        assert!(felt.hits.len() >= 7, "{} hits", felt.hits.len());
    }

    #[test]
    fn every_character_the_plot_writes_has_a_glyph() {
        let text = "Felt under the sub: buried 0.30 m in a festival field; on a concrete \
                    slab, timber floor, midspan level dB peak at Wk-weighted vertical \
                    acceleration, m/s² 125 ms r.m.s. 1 s (the comfort bands) hit still \
                    perceived extremely very fairly a little uncomfortable 0123456789 hits \
                    ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz _-().,:;'%";
        for ch in text.chars() {
            assert!(FONT.iter().any(|g| g.0 == ch), "no glyph for {ch:?}");
        }
    }
}
