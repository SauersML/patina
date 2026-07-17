//! CPU rendering pipeline for Patina's front panel.
//!
//! The technique is the one production synth UIs (Arturia, u-he) actually
//! use: bake the chrome as high-quality raster layers — real Gaussian blur,
//! real soft shadows, per-pixel material shading — and composite a crisp
//! vector layer (arcs, pointers, type) on top at draw time.
//!
//! Everything here is generated, never shipped: fBm walnut, an aurora light
//! field, sphere-shaded knob caps, and true frosted glass made by cropping
//! the backdrop behind each panel, blurring it, and masking it with an
//! anti-aliased rounded-rect SDF.

use eframe::egui::{Color32, ColorImage};

// ---------------------------------------------------------------------------
// Noise
// ---------------------------------------------------------------------------

fn vhash(ix: i32, iy: i32, seed: u32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(374_761_393)
        ^ (iy as u32).wrapping_mul(668_265_263)
        ^ seed.wrapping_mul(2_654_435_761);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}

fn vnoise(x: f32, y: f32, seed: u32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let a = vhash(ix, iy, seed);
    let b = vhash(ix + 1, iy, seed);
    let c = vhash(ix, iy + 1, seed);
    let d = vhash(ix + 1, iy + 1, seed);
    a + (b - a) * sx + (c - a) * sy + (a - b - c + d) * sx * sy
}

pub fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut amp, mut freq, mut sum, mut norm) = (0.5, 1.0, 0.0, 0.0);
    for i in 0..octaves {
        sum += vnoise(x * freq, y * freq, seed + i) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

// ---------------------------------------------------------------------------
// Float RGB raster
// ---------------------------------------------------------------------------

fn to_color_image(w: usize, h: usize, rgb: &[[f32; 3]]) -> ColorImage {
    let pixels = rgb
        .iter()
        .map(|c| {
            Color32::from_rgb(
                (c[0].clamp(0.0, 1.0) * 255.0) as u8,
                (c[1].clamp(0.0, 1.0) * 255.0) as u8,
                (c[2].clamp(0.0, 1.0) * 255.0) as u8,
            )
        })
        .collect();
    ColorImage { size: [w, h], pixels }
}

fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let radius = (sigma * 3.0).ceil() as i32;
    let mut k: Vec<f32> = (-radius..=radius)
        .map(|i| (-(i as f32 * i as f32) / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = k.iter().sum();
    for v in &mut k {
        *v /= sum;
    }
    k
}

/// Separable Gaussian blur over an RGB buffer, edge-clamped.
fn blur_rgb(src: &[[f32; 3]], w: usize, h: usize, sigma: f32) -> Vec<[f32; 3]> {
    let kernel = gaussian_kernel(sigma);
    let radius = (kernel.len() / 2) as i32;
    let mut tmp = vec![[0.0f32; 3]; w * h];
    let mut out = vec![[0.0f32; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 3];
            for (ki, kv) in kernel.iter().enumerate() {
                let sx = (x as i32 + ki as i32 - radius).clamp(0, w as i32 - 1) as usize;
                let p = src[y * w + sx];
                acc[0] += p[0] * kv;
                acc[1] += p[1] * kv;
                acc[2] += p[2] * kv;
            }
            tmp[y * w + x] = acc;
        }
    }
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 3];
            for (ki, kv) in kernel.iter().enumerate() {
                let sy = (y as i32 + ki as i32 - radius).clamp(0, h as i32 - 1) as usize;
                let p = tmp[sy * w + x];
                acc[0] += p[0] * kv;
                acc[1] += p[1] * kv;
                acc[2] += p[2] * kv;
            }
            out[y * w + x] = acc;
        }
    }
    out
}

/// Separable Gaussian blur over a scalar mask.
fn blur_mask(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let kernel = gaussian_kernel(sigma);
    let radius = (kernel.len() / 2) as i32;
    let mut tmp = vec![0.0f32; w * h];
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (ki, kv) in kernel.iter().enumerate() {
                let sx = (x as i32 + ki as i32 - radius).clamp(0, w as i32 - 1) as usize;
                acc += src[y * w + sx] * kv;
            }
            tmp[y * w + x] = acc;
        }
    }
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (ki, kv) in kernel.iter().enumerate() {
                let sy = (y as i32 + ki as i32 - radius).clamp(0, h as i32 - 1) as usize;
                acc += tmp[sy * w + x] * kv;
            }
            out[y * w + x] = acc;
        }
    }
    out
}

/// Anti-aliased coverage of a rounded rect at pixel center (x, y); the rect
/// spans [0,w]×[0,h] with corner radius r.
fn rounded_coverage(x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let cx = x - w * 0.5;
    let cy = y - h * 0.5;
    let qx = cx.abs() - (w * 0.5 - r);
    let qy = cy.abs() - (h * 0.5 - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    let sdf = outside + inside - r;
    (0.5 - sdf).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Backdrop — aurora light field
// ---------------------------------------------------------------------------

/// Deep teal-navy field: soft aurora glows over a faint fBm nebula, with a
/// vignette and dither. Rendered at the actual window size.
pub fn render_backdrop(w: usize, h: usize) -> Vec<[f32; 3]> {
    let mut out = vec![[0.0f32; 3]; w * h];
    let blobs: [(f32, f32, f32, f32, f32, f32, f32); 5] = [
        (0.14, 0.16, 0.52, 0.24, 0.68, 0.80, 0.115),
        (0.88, 0.08, 0.46, 0.30, 0.74, 0.58, 0.070),
        (0.64, 0.98, 0.62, 0.85, 0.55, 0.26, 0.080),
        (0.30, 0.80, 0.55, 0.22, 0.50, 0.72, 0.065),
        (0.50, 0.45, 0.90, 0.10, 0.28, 0.36, 0.045),
    ];
    for y in 0..h {
        let fy = y as f32 / h as f32;
        for x in 0..w {
            let fx = x as f32 / w as f32;
            let base = 0.050 - fy * 0.016;
            let (mut r, mut g, mut b) = (base * 0.72, base * 1.00, base * 1.24);
            // Nebula: broad fBm modulation keeps the field from being flat
            let neb = fbm(fx * 3.0, fy * 3.0, 3, 17) - 0.5;
            g += neb * 0.014;
            b += neb * 0.020;
            for (bx, by, rad, br, bg, bb, s) in blobs {
                let dx = (fx - bx) / rad;
                let dy = (fy - by) / rad;
                let fall = (-(dx * dx + dy * dy) * 2.1).exp() * s;
                r += br * fall;
                g += bg * fall;
                b += bb * fall;
            }
            // Vignette
            let vx = (fx - 0.5) * 2.0;
            let vy = (fy - 0.5) * 2.0;
            let vig = 1.0 - 0.30 * (vx * vx + vy * vy).powf(1.2);
            r *= vig;
            g *= vig;
            b *= vig;
            let d = (vhash(x as i32, y as i32, 3) - 0.5) * 0.0065;
            out[y * w + x] = [r + d, g + d, b + d];
        }
    }
    out
}

pub fn backdrop_image(w: usize, h: usize, rgb: &[[f32; 3]]) -> ColorImage {
    to_color_image(w, h, rgb)
}

// ---------------------------------------------------------------------------
// Frosted glass — the real thing
// ---------------------------------------------------------------------------

const SHADOW_PAD: usize = 22;

/// Bake a frosted-glass panel over the given backdrop region.
///
/// `panel` = (x, y, w, h) in backdrop pixels, `corner` the corner radius.
/// The output is RGBA sized (w + 2·pad, h + 2·pad): the pad carries a real
/// blurred drop shadow. Inside the mask: the backdrop cropped and Gaussian-
/// blurred (σ=11), lifted and cool-tinted, with a baked top light sweep,
/// bottom inner shade, and a 1.2px inner border glint.
pub fn frost_panel(
    backdrop: &[[f32; 3]],
    bw: usize,
    bh: usize,
    panel: (f32, f32, f32, f32),
    corner: f32,
) -> ColorImage {
    let (px_, py_, pw_, ph_) = panel;
    let pw = pw_.round().max(8.0) as usize;
    let ph = ph_.round().max(8.0) as usize;
    let pad = SHADOW_PAD;
    let ow = pw + pad * 2;
    let oh = ph + pad * 2;

    // 1. Crop the backdrop under the panel (edge-clamped), with blur margin
    let sigma = 11.0f32;
    let margin = (sigma * 3.0).ceil() as i32;
    let cw = pw + 2 * margin as usize;
    let ch = ph + 2 * margin as usize;
    let x0 = px_.round() as i32 - margin;
    let y0 = py_.round() as i32 - margin;
    let mut crop = vec![[0.0f32; 3]; cw * ch];
    for y in 0..ch {
        let sy = (y0 + y as i32).clamp(0, bh as i32 - 1) as usize;
        for x in 0..cw {
            let sx = (x0 + x as i32).clamp(0, bw as i32 - 1) as usize;
            crop[y * cw + x] = backdrop[sy * bw + sx];
        }
    }

    // 2. True frost: Gaussian blur of what's behind the glass
    let blurred = blur_rgb(&crop, cw, ch, sigma);

    // 3. Panel coverage mask, then its blurred copy as the drop shadow
    let mut mask = vec![0.0f32; ow * oh];
    for y in 0..oh {
        for x in 0..ow {
            mask[y * ow + x] = rounded_coverage(
                x as f32 - pad as f32 + 0.5,
                y as f32 - pad as f32 + 0.5,
                pw as f32,
                ph as f32,
                corner,
            );
        }
    }
    let shadow = blur_mask(&mask, ow, oh, 7.0);

    // 4. Composite
    let mut pixels = Vec::with_capacity(ow * oh);
    for y in 0..oh {
        for x in 0..ow {
            let m = mask[y * ow + x];
            // Drop shadow, biased downward
            let sy = (y as i32 - 5).clamp(0, oh as i32 - 1) as usize;
            let sh = (shadow[sy * ow + x] - m).max(0.0) * 0.55;

            if m <= 0.002 {
                pixels.push(Color32::from_rgba_unmultiplied(
                    0,
                    0,
                    0,
                    (sh * 255.0) as u8,
                ));
                continue;
            }

            let bx = x as i32 - pad as i32 + margin;
            let by = y as i32 - pad as i32 + margin;
            let p = blurred
                [(by.clamp(0, ch as i32 - 1) as usize) * cw + bx.clamp(0, cw as i32 - 1) as usize];

            // Lift + cool tint: frosted, not smoked
            let mut r = p[0] * 1.04 + 0.034;
            let mut g = p[1] * 1.04 + 0.040;
            let mut b = p[2] * 1.04 + 0.050;

            let fx = (x as f32 - pad as f32) / pw as f32;
            let fy = (y as f32 - pad as f32) / ph as f32;

            // Top light sweep, angled slightly like a real pane
            let sweep = (1.0 - (fy * 2.6 + fx * 0.25)).clamp(0.0, 1.0);
            let sw = sweep * sweep * 0.085;
            r += sw;
            g += sw;
            b += sw;

            // Bottom inner shade seats the glass
            let seat = ((fy - 0.86) / 0.14).clamp(0.0, 1.0) * 0.10;
            r -= seat;
            g -= seat;
            b -= seat;

            // Inner border glint from the mask gradient
            let edge = (m
                - rounded_coverage(
                    x as f32 - pad as f32 + 0.5,
                    y as f32 - pad as f32 + 0.5 + 1.2,
                    pw as f32,
                    ph as f32,
                    corner,
                ))
            .max(0.0);
            let glint = edge * if fy < 0.5 { 0.55 } else { 0.18 };
            r += glint;
            g += glint;
            b += glint;

            // Blend rim: shadow shows through the anti-aliased edge
            let alpha = (m + sh * (1.0 - m)).clamp(0.0, 1.0);
            let inv = sh * (1.0 - m);
            let scale = m / alpha.max(1e-4);
            r = r * scale;
            g = g * scale;
            b = b * scale;
            let _ = inv;
            pixels.push(Color32::from_rgba_unmultiplied(
                (r.clamp(0.0, 1.0) * 255.0) as u8,
                (g.clamp(0.0, 1.0) * 255.0) as u8,
                (b.clamp(0.0, 1.0) * 255.0) as u8,
                (alpha * 255.0) as u8,
            ));
        }
    }
    ColorImage { size: [ow, oh], pixels }
}

pub const fn frost_pad() -> f32 {
    SHADOW_PAD as f32
}

// ---------------------------------------------------------------------------
// Walnut
// ---------------------------------------------------------------------------

/// Aged walnut rail, 2× supersampled: fBm-warped growth rings, along-grain
/// streaks and pores, satin finish falling off vertically.
pub fn render_wood(w: usize, h: usize) -> ColorImage {
    let (sw, sh) = (w * 2, h * 2);
    let dark = (0.110, 0.064, 0.035);
    let light = (0.42, 0.275, 0.152);
    let mut hi = vec![[0.0f32; 3]; sw * sh];
    for y in 0..sh {
        let fy = y as f32 / sh as f32;
        for x in 0..sw {
            let (xf, yf) = (x as f32 * 0.5, y as f32 * 0.5);
            let warp = fbm(xf * 0.006, yf * 0.028, 4, 11);
            let t = xf * 0.011 + warp * 6.5;
            let ring = ((t * std::f32::consts::TAU * 0.75).sin() * 0.5 + 0.5).powf(1.8);
            let fine = ((t * std::f32::consts::TAU * 3.1).sin() * 0.5 + 0.5) * 0.35;
            let streak = fbm(xf * 0.045, yf * 0.95, 3, 47);
            let pores = fbm(xf * 0.55, yf * 0.16, 2, 83);
            let shade = 0.44 * ring + 0.14 * fine + 0.24 * streak + 0.12 * warp + 0.06 * pores;
            let mut r = dark.0 + (light.0 - dark.0) * shade;
            let mut g = dark.1 + (light.1 - dark.1) * shade;
            let mut b = dark.2 + (light.2 - dark.2) * shade;
            // Satin varnish: highlight band in the upper third
            let gloss = (-(fy - 0.16) * (fy - 0.16) * 60.0).exp() * 0.075;
            let finish = 1.08 - 0.26 * fy + gloss;
            r *= finish;
            g *= finish;
            b *= finish;
            if fy < 0.015 {
                r = r * 0.55 + 0.30;
                g = g * 0.55 + 0.235;
                b = b * 0.55 + 0.16;
            }
            if fy > 0.972 {
                r *= 0.42;
                g *= 0.42;
                b *= 0.42;
            }
            hi[y * sw + x] = [r, g, b];
        }
    }
    // Box downsample 2× for clean anti-aliased grain
    let mut out = vec![[0.0f32; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 3];
            for dy in 0..2 {
                for dx in 0..2 {
                    let p = hi[(y * 2 + dy) * sw + x * 2 + dx];
                    acc[0] += p[0];
                    acc[1] += p[1];
                    acc[2] += p[2];
                }
            }
            out[y * w + x] = [acc[0] * 0.25, acc[1] * 0.25, acc[2] * 0.25];
        }
    }
    to_color_image(w, h, &out)
}

// ---------------------------------------------------------------------------
// Knob cap
// ---------------------------------------------------------------------------

/// Knob cap sprite, 2× supersampled: brushed dark bezel ring around a
/// sphere-shaded glass dome, tight Frutiger gloss lobe, aqua bounce along
/// the lower rim.
pub fn render_knob(size: usize) -> ColorImage {
    let s = size * 2;
    let c = s as f32 / 2.0;
    let radius = c - 2.0;
    let ring_inner = 0.78f32; // dome/bezel boundary as a fraction of radius
    let l = {
        let v = (-0.42f32, -0.62f32, 0.66f32);
        let len = (v.0 * v.0 + v.1 * v.1 + v.2 * v.2).sqrt();
        (v.0 / len, v.1 / len, v.2 / len)
    };
    let mut hi = vec![(0.0f32, 0.0f32, 0.0f32, 0.0f32); s * s];
    for y in 0..s {
        for x in 0..s {
            let dx = (x as f32 - c) / radius;
            let dy = (y as f32 - c) / radius;
            let d = (dx * dx + dy * dy).sqrt();
            if d >= 1.0 {
                continue;
            }
            let (mut r, mut g, mut b);
            if d > ring_inner {
                // Brushed bezel: angular streaks, lit from above
                let angle = dy.atan2(dx);
                let streak = vnoise(angle * 60.0, d * 8.0, 29) * 0.10;
                let lit = (0.5 - dy * 0.45).clamp(0.1, 0.9);
                let base = 0.10 + lit * 0.13 + streak;
                r = base * 0.98;
                g = base * 1.02;
                b = base * 1.10;
                // Bevel: bright crest at the ring's outer and inner edges
                let crest = (-(d - 0.985) * (d - 0.985) * 8000.0).exp() * 0.10
                    + (-(d - ring_inner - 0.015) * (d - ring_inner - 0.015) * 8000.0).exp() * 0.07;
                r += crest;
                g += crest;
                b += crest;
            } else {
                // Glass dome
                let nd = d / ring_inner;
                let nz = (1.0 - nd * nd).sqrt();
                let lambert = (dx / ring_inner * l.0 + dy / ring_inner * l.1 + nz * l.2).max(0.0);
                let base = 0.055 + lambert * 0.15;
                r = base * 0.94;
                g = base * 1.02;
                b = base * 1.14;
                // Gloss lobe
                let sx = dx / ring_inner + 0.36;
                let sy = dy / ring_inner + 0.46;
                let spec = (-(sx * sx + sy * sy) * 10.0).exp();
                r += spec * 0.38;
                g += spec * 0.41;
                b += spec * 0.44;
                // Aqua bounce along the lower inside rim
                let rim = (nd - 0.66).max(0.0) / 0.34;
                let below = (dy + 0.15).max(0.0);
                let bounce = rim * below * 0.17;
                g += bounce * 0.75;
                b += bounce;
            }
            let alpha = ((1.0 - d) * radius).clamp(0.0, 1.0);
            hi[y * s + x] = (r, g, b, alpha);
        }
    }
    // Downsample
    let mut pixels = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let mut acc = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for dy in 0..2 {
                for dx in 0..2 {
                    let p = hi[(y * 2 + dy) * s + x * 2 + dx];
                    acc.0 += p.0;
                    acc.1 += p.1;
                    acc.2 += p.2;
                    acc.3 += p.3;
                }
            }
            pixels.push(Color32::from_rgba_unmultiplied(
                ((acc.0 * 0.25).clamp(0.0, 1.0) * 255.0) as u8,
                ((acc.1 * 0.25).clamp(0.0, 1.0) * 255.0) as u8,
                ((acc.2 * 0.25).clamp(0.0, 1.0) * 255.0) as u8,
                ((acc.3 * 0.25).clamp(0.0, 1.0) * 255.0) as u8,
            ));
        }
    }
    ColorImage { size: [size, size], pixels }
}
