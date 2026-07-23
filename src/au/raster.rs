// A tiny software rasterizer for egui's tessellated output.
//
// Why software, not GPU: modern Logic always hosts Audio Units out of
// process (in AUHostingServiceXPC). A raw OpenGL/Metal context drawn in
// that service can't be composited back into Logic's window through the
// AppKit ViewBridge, so a GPU-backed plugin view shows up blank / 1×1.
// A CPU-drawn bitmap set as a layer-backed NSView's `layer.contents`
// remotes cleanly (it becomes an IOSurface) and sizes correctly. So the
// AU editor renders egui here on the CPU and blits the result.
//
// Scope: exactly what Patina's editor emits — textured, vertex-colored
// triangles, no paint callbacks. Colors are egui `Color32` (sRGBA,
// premultiplied); we blend in that space (as egui's own glow backend does
// by default) which is visually identical for an opaque panel. Textures
// are sampled nearest-neighbour: egui rasterizes glyphs and bakes our
// panels at the target pixels-per-point, so quads map ~1:1 to the atlas
// and nearest is both correct-looking and cheap.

use egui::epaint::{ClippedPrimitive, Color32, Primitive, TextureId, Vertex};
use egui::TexturesDelta;
use std::collections::HashMap;

struct Tex {
    w: usize,
    h: usize,
    /// Premultiplied sRGBA, row-major top-to-bottom.
    px: Vec<Color32>,
}

impl Tex {
    #[inline]
    fn sample(&self, u: f32, v: f32) -> Color32 {
        if self.w == 0 || self.h == 0 {
            return Color32::TRANSPARENT;
        }
        // Nearest texel, clamped.
        let x = ((u * self.w as f32) as isize).clamp(0, self.w as isize - 1) as usize;
        let y = ((v * self.h as f32) as isize).clamp(0, self.h as isize - 1) as usize;
        self.px[y * self.w + x]
    }
}

/// The rasterizer owns the framebuffer and the texture store, so a redraw
/// is one call. Reused across frames; only reallocates on a size change.
pub struct Raster {
    /// Physical pixels, RGBA8 premultiplied, top-to-bottom.
    pub fb: Vec<u8>,
    pub w: usize,
    pub h: usize,
    textures: HashMap<TextureId, Tex>,
}

impl Raster {
    pub fn new() -> Self {
        Self { fb: Vec::new(), w: 0, h: 0, textures: HashMap::new() }
    }

    pub fn resize(&mut self, w: usize, h: usize) {
        if w != self.w || h != self.h {
            self.w = w;
            self.h = h;
            self.fb = vec![0; w * h * 4];
        }
    }

    /// Apply egui's per-frame texture changes. Call before `paint`.
    pub fn update_textures(&mut self, delta: &TexturesDelta) {
        for (id, image_delta) in &delta.set {
            let [iw, ih] = image_delta.image.size();
            let new_px: Vec<Color32> = match &image_delta.image {
                egui::epaint::ImageData::Color(img) => img.pixels.clone(),
                // Coverage -> premultiplied white, exactly as the GPU
                // backends do (gamma None = egui's default 0.55 curve).
                egui::epaint::ImageData::Font(font) => font.srgba_pixels(None).collect(),
            };
            match image_delta.pos {
                None => {
                    self.textures.insert(*id, Tex { w: iw, h: ih, px: new_px });
                }
                Some([px, py]) => {
                    if let Some(tex) = self.textures.get_mut(id) {
                        for row in 0..ih {
                            let y = py + row;
                            if y >= tex.h {
                                break;
                            }
                            for col in 0..iw {
                                // Bound the COLUMN too: a `dst < len` test
                                // alone lets an overwide patch wrap onto the
                                // start of the next row and corrupt it.
                                let x = px + col;
                                if x >= tex.w {
                                    break;
                                }
                                tex.px[y * tex.w + x] = new_px[row * iw + col];
                            }
                        }
                    }
                }
            }
        }
        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    /// Fill the framebuffer with an opaque base color (premultiplied).
    pub fn clear(&mut self, rgb: [u8; 3]) {
        for px in self.fb.chunks_exact_mut(4) {
            px.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }

    /// Rasterize the tessellated primitives at the given scale (physical
    /// pixels per logical point). Positions and clip rects are in points.
    pub fn paint(&mut self, prims: &[ClippedPrimitive], ppp: f32) {
        // Borrow the framebuffer and the texture store as disjoint fields so
        // `triangle` (a free fn) can write pixels while `tex` is held.
        let (fw, fh) = (self.w, self.h);
        let fb = &mut self.fb;
        let textures = &self.textures;
        for prim in prims {
            let Primitive::Mesh(mesh) = &prim.primitive else { continue };
            let Some(tex) = textures.get(&mesh.texture_id) else { continue };

            // Clip rect -> physical pixel bounds, intersected with the frame.
            let cx0 = (prim.clip_rect.min.x * ppp).floor().max(0.0) as usize;
            let cy0 = (prim.clip_rect.min.y * ppp).floor().max(0.0) as usize;
            let cx1 = ((prim.clip_rect.max.x * ppp).ceil() as usize).min(fw);
            let cy1 = ((prim.clip_rect.max.y * ppp).ceil() as usize).min(fh);
            if cx0 >= cx1 || cy0 >= cy1 {
                continue;
            }

            for tri in mesh.indices.chunks_exact(3) {
                let v0 = &mesh.vertices[tri[0] as usize];
                let v1 = &mesh.vertices[tri[1] as usize];
                let v2 = &mesh.vertices[tri[2] as usize];
                triangle(fb, fw, v0, v1, v2, tex, ppp, cx0, cy0, cx1, cy1);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn triangle(
    fb: &mut [u8],
    fbw: usize,
    v0: &Vertex,
    v1: &Vertex,
    v2: &Vertex,
    tex: &Tex,
    ppp: f32,
    cx0: usize,
    cy0: usize,
    cx1: usize,
    cy1: usize,
) {
    let (ax, ay) = (v0.pos.x * ppp, v0.pos.y * ppp);
    let (bx, by) = (v1.pos.x * ppp, v1.pos.y * ppp);
    let (cx, cy) = (v2.pos.x * ppp, v2.pos.y * ppp);

        // Triangle bounding box, clamped to the clip window.
        let min_x = ax.min(bx).min(cx).floor().max(cx0 as f32) as usize;
        let max_x = (ax.max(bx).max(cx).ceil() as usize).min(cx1);
        let min_y = ay.min(by).min(cy).floor().max(cy0 as f32) as usize;
        let max_y = (ay.max(by).max(cy).ceil() as usize).min(cy1);
        if min_x >= max_x || min_y >= max_y {
            return;
        }

        // Edge-function area; egui isn't consistent about winding, so accept
        // either sign (no backface culling).
        let area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
        if area.abs() < 1e-6 {
            return;
        }
        let inv_area = 1.0 / area;

        for py in min_y..max_y {
            let fy = py as f32 + 0.5;
            for px in min_x..max_x {
                let fx = px as f32 + 0.5;
                // Barycentric weights via edge functions.
                let w0 = ((bx - fx) * (cy - fy) - (by - fy) * (cx - fx)) * inv_area;
                let w1 = ((cx - fx) * (ay - fy) - (cy - fy) * (ax - fx)) * inv_area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }

                let u = w0 * v0.uv.x + w1 * v1.uv.x + w2 * v2.uv.x;
                let v = w0 * v0.uv.y + w1 * v1.uv.y + w2 * v2.uv.y;
                let texel = tex.sample(u, v);

                // Interpolated vertex color (premultiplied) × texel, both in
                // 0..255 -> premultiplied source.
                let vc = [
                    w0 * v0.color[0] as f32 + w1 * v1.color[0] as f32 + w2 * v2.color[0] as f32,
                    w0 * v0.color[1] as f32 + w1 * v1.color[1] as f32 + w2 * v2.color[1] as f32,
                    w0 * v0.color[2] as f32 + w1 * v1.color[2] as f32 + w2 * v2.color[2] as f32,
                    w0 * v0.color[3] as f32 + w1 * v1.color[3] as f32 + w2 * v2.color[3] as f32,
                ];
                let sr = vc[0] * texel[0] as f32 * (1.0 / 255.0);
                let sg = vc[1] * texel[1] as f32 * (1.0 / 255.0);
                let sb = vc[2] * texel[2] as f32 * (1.0 / 255.0);
                let sa = vc[3] * texel[3] as f32 * (1.0 / 255.0);
                if sa <= 0.0 {
                    continue;
                }

                // Premultiplied "over": dst = src + dst·(1 - src_a).
                let inv = 1.0 - sa * (1.0 / 255.0);
                let idx = (py * fbw + px) * 4;
                let d = &mut fb[idx..idx + 4];
                d[0] = (sr + d[0] as f32 * inv).min(255.0) as u8;
                d[1] = (sg + d[1] as f32 * inv).min(255.0) as u8;
                d[2] = (sb + d[2] as f32 * inv).min(255.0) as u8;
                d[3] = (sa + d[3] as f32 * inv).min(255.0) as u8;
            }
        }
    }
