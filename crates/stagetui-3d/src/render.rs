//! A software renderer: triangles → pixels, on every CPU core.
//!
//! 1. Vertices move into the camera's space; triangles crossing the near plane are cut.
//! 2. The picture is split into bands of rows; each core takes bands. In a band,
//!    triangles are first rasterized into a depth buffer that remembers *which*
//!    triangle is nearest at each pixel, then every pixel is lit once.
//! 3. Lines (the floor grid, the wireframe) are drawn on top, hidden by nearer surfaces.
//! 4. With `ss = 2`, each pixel is the average of 2×2 samples (smooth edges).

use std::sync::Mutex;

use crate::math::{M4, V3, v3};
use crate::model::Model;

/// Vertical field of view.
pub const FOV_Y: f32 = 40.0 * std::f32::consts::PI / 180.0;
const NEAR: f32 = 0.01;
/// Output rows per band.
const BAND: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Around the model, in radians.
    pub yaw: f32,
    /// Up/down, in radians (−π/2…π/2).
    pub pitch: f32,
    pub distance: f32,
    pub target: V3,
}

impl Camera {
    pub fn eye(&self) -> V3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + v3(cp * sy, sp, cp * cy) * self.distance
    }

    /// The camera's right, up and backward directions.
    pub fn axes(&self) -> (V3, V3, V3) {
        let back = (self.eye() - self.target).norm();
        let right = v3(0.0, 1.0, 0.0).cross(back).norm();
        let right = if right.len() < 0.5 {
            v3(1.0, 0.0, 0.0)
        } else {
            right
        };
        (right, back.cross(right), back)
    }

    pub fn view(&self) -> M4 {
        let (r, u, b) = self.axes();
        let e = self.eye();
        [
            [r.x, r.y, r.z, -r.dot(e)],
            [u.x, u.y, u.z, -u.dot(e)],
            [b.x, b.y, b.z, -b.dot(e)],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }
}

/// Colors and switches.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub bg: [u8; 3],
    pub fg: [u8; 3],
    pub accent: [u8; 3],
    pub wireframe: bool,
    pub grid: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct Attr {
    /// Position in camera space.
    p: V3,
    n: V3,
    uv: [f32; 2],
    c: [f32; 4],
}

fn lerp_attr(a: &Attr, b: &Attr, t: f32) -> Attr {
    let l = |x: f32, y: f32| x + (y - x) * t;
    Attr {
        p: a.p + (b.p - a.p) * t,
        n: a.n + (b.n - a.n) * t,
        uv: [l(a.uv[0], b.uv[0]), l(a.uv[1], b.uv[1])],
        c: [
            l(a.c[0], b.c[0]),
            l(a.c[1], b.c[1]),
            l(a.c[2], b.c[2]),
            l(a.c[3], b.c[3]),
        ],
    }
}

/// A triangle on the screen (supersampled pixels).
struct Tri {
    x: [f32; 3],
    y: [f32; 3],
    /// 1 / depth at each corner.
    iw: [f32; 3],
    a: [Attr; 3],
    material: u32,
}

/// A line on the screen: two ends (x, y, 1/depth), color, strength, thickness.
struct Seg {
    a: [f32; 3],
    b: [f32; 3],
    color: [f32; 3],
    alpha: f32,
    thick: bool,
}

struct Projector {
    focal: f32,
    aspect: f32,
    w: f32,
    h: f32,
}

impl Projector {
    fn project(&self, p: V3) -> [f32; 3] {
        let d = -p.z;
        let nx = p.x * self.focal / self.aspect / d;
        let ny = p.y * self.focal / d;
        [
            (nx + 1.0) * 0.5 * self.w,
            (1.0 - ny) * 0.5 * self.h,
            1.0 / d,
        ]
    }
}

/// sRGB byte → linear light.
fn to_linear_table() -> [f32; 256] {
    let mut t = [0.0; 256];
    for (i, v) in t.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        *v = if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        };
    }
    t
}

/// Linear light (0..1 in 4096 steps) → sRGB byte.
fn to_srgb_table() -> Vec<u8> {
    (0..4096)
        .map(|i| {
            let c = i as f32 / 4095.0;
            let s = if c <= 0.003_130_8 {
                c * 12.92
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            };
            (s * 255.0 + 0.5).clamp(0.0, 255.0) as u8
        })
        .collect()
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Draw `model` as seen by `cam`: `width × height` RGBA bytes.
#[allow(clippy::too_many_arguments)]
pub fn render(
    model: &Model,
    edges: Option<&[[u32; 2]]>,
    cam: &Camera,
    look: &Look,
    width: u32,
    height: u32,
    ss: u32,
) -> Vec<u8> {
    let (w, h, ss) = (
        width.max(1) as usize,
        height.max(1) as usize,
        ss.clamp(1, 4) as usize,
    );
    let (sw, sh) = (w * ss, h * ss);
    let lin = to_linear_table();
    let srgb = to_srgb_table();
    let col = |c: [u8; 3]| [lin[c[0] as usize], lin[c[1] as usize], lin[c[2] as usize]];
    let (bg, fg, accent) = (col(look.bg), col(look.fg), col(look.accent));

    let view = cam.view();
    let rot = [
        [view[0][0], view[0][1], view[0][2]],
        [view[1][0], view[1][1], view[1][2]],
        [view[2][0], view[2][1], view[2][2]],
    ];
    let pr = Projector {
        focal: 1.0 / (FOV_Y / 2.0).tan(),
        aspect: w as f32 / h as f32,
        w: sw as f32,
        h: sh as f32,
    };

    // 1. Camera space.
    let attrs: Vec<Attr> = (0..model.positions.len())
        .map(|i| Attr {
            p: crate::math::point(&view, model.positions[i]),
            n: crate::math::apply3(&rot, model.normals[i]),
            uv: model.uvs[i],
            c: model.colors[i],
        })
        .collect();

    let mut tris: Vec<Tri> = Vec::with_capacity(model.triangles.len());
    for (t, &mat) in model.triangles.iter().zip(&model.tri_material) {
        let corners = t.map(|i| attrs[i as usize]);
        let front = corners.iter().filter(|a| -a.p.z >= NEAR).count();
        if front == 0 {
            continue;
        }
        let poly: Vec<Attr> = if front == 3 {
            corners.to_vec()
        } else {
            // Cut along the near plane (keep the part in front of the camera).
            let mut out = Vec::with_capacity(4);
            for k in 0..3 {
                let (a, b) = (&corners[k], &corners[(k + 1) % 3]);
                let (da, db) = (-a.p.z - NEAR, -b.p.z - NEAR);
                if da >= 0.0 {
                    out.push(*a);
                }
                if (da >= 0.0) != (db >= 0.0) {
                    out.push(lerp_attr(a, b, da / (da - db)));
                }
            }
            out
        };
        let screen: Vec<[f32; 3]> = poly.iter().map(|a| pr.project(a.p)).collect();
        for k in 1..poly.len().saturating_sub(1) {
            let idx = [0, k, k + 1];
            let x = idx.map(|i| screen[i][0]);
            let y = idx.map(|i| screen[i][1]);
            let area = (x[1] - x[0]) * (y[2] - y[0]) - (y[1] - y[0]) * (x[2] - x[0]);
            let off = x.iter().all(|&v| v < 0.0)
                || x.iter().all(|&v| v > sw as f32)
                || y.iter().all(|&v| v < 0.0)
                || y.iter().all(|&v| v > sh as f32);
            if area.abs() < 1e-9 || off || !area.is_finite() {
                continue;
            }
            tris.push(Tri {
                x,
                y,
                iw: idx.map(|i| screen[i][2]),
                a: idx.map(|i| poly[i]),
                material: mat,
            });
        }
    }

    // Lines: the floor grid and the wireframe.
    let mut segs: Vec<Seg> = Vec::new();
    let mut line = |a: V3, b: V3, color: [f32; 3], alpha: f32, thick: bool| {
        let (pa, pb) = (crate::math::point(&view, a), crate::math::point(&view, b));
        let (da, db) = (-pa.z - NEAR, -pb.z - NEAR);
        if da < 0.0 && db < 0.0 {
            return;
        }
        let (pa, pb) = if da < 0.0 {
            (pa + (pb - pa) * (da / (da - db)), pb)
        } else if db < 0.0 {
            (pa, pa + (pb - pa) * (da / (da - db)))
        } else {
            (pa, pb)
        };
        segs.push(Seg {
            a: pr.project(pa),
            b: pr.project(pb),
            color,
            alpha,
            thick,
        });
    };
    if look.grid {
        let y = model.floor;
        let grid_color = mix(bg, fg, 0.55);
        let (n, step) = (8i32, 0.25f32);
        let reach = n as f32 * step;
        for i in -n..=n {
            let k = i as f32 * step;
            // Short pieces, so the grid fades out toward its edges.
            for j in -n..n {
                let (s0, s1) = (j as f32 * step, (j + 1) as f32 * step);
                let r = ((s0 + s1) * 0.5).hypot(k) / (reach * 1.05);
                let alpha = 0.35 * (1.0 - r).max(0.0).powf(1.5);
                if alpha > 0.01 {
                    line(v3(s0, y, k), v3(s1, y, k), grid_color, alpha, false);
                    line(v3(k, y, s0), v3(k, y, s1), grid_color, alpha, false);
                }
            }
        }
    }
    if let (true, Some(edges)) = (look.wireframe, edges) {
        for e in edges {
            let (a, b) = (
                model.positions[e[0] as usize],
                model.positions[e[1] as usize],
            );
            line(a, b, accent, 0.85, ss > 1);
        }
    }

    // Which bands each triangle and line touches.
    let rows = BAND * ss;
    let bands = h.div_ceil(BAND);
    let mut tri_bins: Vec<Vec<u32>> = vec![Vec::new(); bands];
    for (i, t) in tris.iter().enumerate() {
        let lo = t.y.iter().copied().fold(f32::MAX, f32::min).max(0.0) as usize / rows;
        let hi =
            (t.y.iter()
                .copied()
                .fold(f32::MIN, f32::max)
                .min(sh as f32 - 1.0)
                .max(0.0) as usize
                / rows)
                .min(bands - 1);
        for bin in &mut tri_bins[lo.min(bands - 1)..=hi] {
            bin.push(i as u32);
        }
    }
    let mut seg_bins: Vec<Vec<u32>> = vec![Vec::new(); bands];
    for (i, s) in segs.iter().enumerate() {
        let (y0, y1) = (s.a[1].min(s.b[1]) - 1.0, s.a[1].max(s.b[1]) + 1.0);
        if y1 < 0.0 || y0 > sh as f32 {
            continue;
        }
        let lo = (y0.max(0.0) as usize / rows).min(bands - 1);
        let hi = (y1.min(sh as f32 - 1.0).max(0.0) as usize / rows).min(bands - 1);
        for bin in &mut seg_bins[lo..=hi] {
            bin.push(i as u32);
        }
    }

    // 2–4. Bands, on every core.
    let mut out = vec![0u8; w * h * 4];
    let jobs: Mutex<Vec<(usize, &mut [u8])>> =
        Mutex::new(out.chunks_mut(w * BAND * 4).enumerate().collect());
    // Enough helpers for the work: small pictures don't need every core.
    let work = (sw * sh) / (64 * 1024) + tris.len() / 10_000 + 1;
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(work)
        .clamp(1, 16);
    let ctx = Ctx {
        model,
        tris: &tris,
        segs: &segs,
        tri_bins: &tri_bins,
        seg_bins: &seg_bins,
        sw,
        sh,
        w,
        ss,
        bg,
        lin: &lin,
        srgb: &srgb,
    };
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut scratch = Scratch::default();
                loop {
                    let job = jobs.lock().unwrap_or_else(|e| e.into_inner()).pop();
                    let Some((band, pixels)) = job else { break };
                    ctx.band(band, pixels, &mut scratch);
                }
            });
        }
    });
    out
}

struct Ctx<'a> {
    model: &'a Model,
    tris: &'a [Tri],
    segs: &'a [Seg],
    tri_bins: &'a [Vec<u32>],
    seg_bins: &'a [Vec<u32>],
    sw: usize,
    sh: usize,
    w: usize,
    ss: usize,
    bg: [f32; 3],
    lin: &'a [f32; 256],
    srgb: &'a [u8],
}

#[derive(Default)]
struct Scratch {
    depth: Vec<f32>,
    id: Vec<u32>,
    bary: Vec<[f32; 2]>,
    color: Vec<[f32; 3]>,
}

const EMPTY: u32 = u32::MAX;

impl Ctx<'_> {
    fn band(&self, band: usize, pixels: &mut [u8], s: &mut Scratch) {
        let out_rows = pixels.len() / (self.w * 4);
        let y0 = band * BAND * self.ss;
        let bh = out_rows * self.ss;
        let n = self.sw * bh;
        s.depth.clear();
        s.depth.resize(n, 0.0);
        s.id.clear();
        s.id.resize(n, EMPTY);
        s.bary.clear();
        s.bary.resize(n, [0.0, 0.0]);

        // Nearest triangle at each sample.
        for &ti in &self.tri_bins[band] {
            let t = &self.tris[ti as usize];
            let area =
                (t.x[1] - t.x[0]) * (t.y[2] - t.y[0]) - (t.y[1] - t.y[0]) * (t.x[2] - t.x[0]);
            let inv_area = 1.0 / area;
            let minx =
                t.x.iter()
                    .copied()
                    .fold(f32::MAX, f32::min)
                    .floor()
                    .max(0.0) as usize;
            let maxx = (t.x.iter().copied().fold(f32::MIN, f32::max).ceil() as isize)
                .min(self.sw as isize - 1);
            let miny =
                (t.y.iter()
                    .copied()
                    .fold(f32::MAX, f32::min)
                    .floor()
                    .max(y0 as f32)) as usize;
            let maxy = (t.y.iter().copied().fold(f32::MIN, f32::max).ceil() as isize)
                .min((y0 + bh) as isize - 1);
            if maxx < minx as isize || maxy < miny as isize {
                continue;
            }
            // Edge functions, as barycentric weights: l0 + l1 + l2 = 1 inside.
            let e = |i: usize, j: usize| {
                let (dx, dy) = ((t.x[j] - t.x[i]) * inv_area, (t.y[j] - t.y[i]) * inv_area);
                // value at (px, py) = dx * (py - yi) - dy * (px - xi)
                (dx, dy, t.x[i], t.y[i])
            };
            let edges = [e(1, 2), e(2, 0), e(0, 1)];
            for py in miny..=maxy as usize {
                let cy = py as f32 + 0.5;
                let row = (py - y0) * self.sw;
                let cx0 = minx as f32 + 0.5;
                let mut l = edges.map(|(dx, dy, xi, yi)| dx * (cy - yi) - dy * (cx0 - xi));
                let step = edges.map(|(_, dy, _, _)| -dy);
                for px in minx..=maxx as usize {
                    if l[0] >= 0.0 && l[1] >= 0.0 && l[2] >= 0.0 {
                        let iw = l[0] * t.iw[0] + l[1] * t.iw[1] + l[2] * t.iw[2];
                        let at = row + px;
                        if iw > s.depth[at] {
                            s.depth[at] = iw;
                            s.id[at] = ti;
                            s.bary[at] = [l[1], l[2]];
                        }
                    }
                    for k in 0..3 {
                        l[k] += step[k];
                    }
                }
            }
        }

        // Light each sample once.
        s.color.clear();
        s.color.resize(n, self.bg);
        for row in 0..bh {
            // A gentle light from above on the background.
            let fy = (y0 + row) as f32 / self.sh as f32;
            let back = mix(
                self.bg,
                [
                    self.bg[0] * 1.6 + 0.01,
                    self.bg[1] * 1.6 + 0.01,
                    self.bg[2] * 1.6 + 0.012,
                ],
                0.6 * (1.0 - fy),
            );
            for px in 0..self.sw {
                let at = row * self.sw + px;
                let id = s.id[at];
                s.color[at] = if id == EMPTY {
                    back
                } else {
                    self.shade(&self.tris[id as usize], s.bary[at], s.depth[at])
                };
            }
        }

        // Lines on top, behind nearer surfaces.
        for &si in &self.seg_bins[band] {
            let g = &self.segs[si as usize];
            let (dx, dy) = (g.b[0] - g.a[0], g.b[1] - g.a[1]);
            let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as usize;
            let mostly_flat = dx.abs() >= dy.abs();
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let (x, y, iw) = (
                    g.a[0] + dx * t,
                    g.a[1] + dy * t,
                    g.a[2] + (g.b[2] - g.a[2]) * t,
                );
                let spots: &[(isize, isize)] = if !g.thick {
                    &[(0, 0)]
                } else if mostly_flat {
                    &[(0, 0), (0, 1)]
                } else {
                    &[(0, 0), (1, 0)]
                };
                for &(ox, oy) in spots {
                    let (px, py) = (x as isize + ox, y as isize + oy);
                    if px < 0
                        || px >= self.sw as isize
                        || py < y0 as isize
                        || py >= (y0 + bh) as isize
                    {
                        continue;
                    }
                    let at = (py as usize - y0) * self.sw + px as usize;
                    if iw >= s.depth[at] * 0.997 {
                        s.color[at] = mix(s.color[at], g.color, g.alpha);
                    }
                }
            }
        }

        // Average the samples into pixels.
        let ss = self.ss;
        let k = 1.0 / (ss * ss) as f32;
        for oy in 0..out_rows {
            for ox in 0..self.w {
                let mut sum = [0.0f32; 3];
                for sy in 0..ss {
                    for sx in 0..ss {
                        let c = s.color[(oy * ss + sy) * self.sw + ox * ss + sx];
                        sum[0] += c[0];
                        sum[1] += c[1];
                        sum[2] += c[2];
                    }
                }
                let at = (oy * self.w + ox) * 4;
                for ch in 0..3 {
                    pixels[at + ch] = self.srgb[((sum[ch] * k).clamp(0.0, 1.0) * 4095.0) as usize];
                }
                pixels[at + 3] = 255;
            }
        }
    }

    fn shade(&self, t: &Tri, l: [f32; 2], iw: f32) -> [f32; 3] {
        // Perspective-correct weights.
        let l = [1.0 - l[0] - l[1], l[0], l[1]];
        let b = [
            l[0] * t.iw[0] / iw,
            l[1] * t.iw[1] / iw,
            l[2] * t.iw[2] / iw,
        ];
        let a = &t.a;
        let p = a[0].p * b[0] + a[1].p * b[1] + a[2].p * b[2];
        let mut n = (a[0].n * b[0] + a[1].n * b[1] + a[2].n * b[2]).norm();
        let uv = [
            a[0].uv[0] * b[0] + a[1].uv[0] * b[1] + a[2].uv[0] * b[2],
            a[0].uv[1] * b[0] + a[1].uv[1] * b[1] + a[2].uv[1] * b[2],
        ];
        let mut base = [0.0f32; 3];
        for (ch, v) in base.iter_mut().enumerate() {
            *v = a[0].c[ch] * b[0] + a[1].c[ch] * b[1] + a[2].c[ch] * b[2];
        }
        let m = &self.model.materials[t.material as usize];
        for (ch, v) in base.iter_mut().enumerate() {
            *v *= m.color[ch];
        }
        if let Some(tex) = m.texture.and_then(|i| self.model.textures.get(i)) {
            let px = tex.sample(uv[0], uv[1]);
            for (ch, v) in base.iter_mut().enumerate() {
                *v *= self.lin[px[ch] as usize];
            }
        }
        let view = (-p).norm();
        if n.len() < 0.5 {
            n = view;
        }
        if n.dot(view) < 0.0 {
            n = -n; // inside faces and flipped triangles are lit like outside ones
        }
        let key = v3(-0.35, 0.65, 0.68).norm();
        let fill = v3(0.7, 0.1, 0.45).norm();
        let diffuse = 0.16 + 0.78 * n.dot(key).max(0.0) + 0.22 * n.dot(fill).max(0.0);
        let half = (key + view).norm();
        let spec = 0.22 * n.dot(half).max(0.0).powi(48);
        let rim = 0.10 * (1.0 - n.dot(view).max(0.0)).powi(3);
        base.map(|c| c * diffuse + spec + rim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cube;

    fn look() -> Look {
        Look {
            bg: [20, 20, 30],
            fg: [220, 220, 230],
            accent: [180, 140, 250],
            wireframe: false,
            grid: false,
        }
    }

    fn cam() -> Camera {
        Camera {
            yaw: 0.6,
            pitch: 0.4,
            distance: 6.0,
            target: V3::default(),
        }
    }

    fn px(img: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
        let at = ((y * w + x) * 4) as usize;
        [img[at], img[at + 1], img[at + 2]]
    }

    #[test]
    fn a_cube_fills_the_middle_and_not_the_corners() {
        let m = cube();
        let img = render(&m, None, &cam(), &look(), 120, 80, 1);
        assert_eq!(img.len(), 120 * 80 * 4);
        let middle = px(&img, 120, 60, 40);
        let corner = px(&img, 120, 1, 1);
        assert!(
            middle.iter().map(|&c| c as u32).sum::<u32>() > 300,
            "{middle:?}"
        );
        assert!(
            corner.iter().map(|&c| c as u32).sum::<u32>() < 200,
            "{corner:?}"
        );
    }

    #[test]
    fn smooth_edges_and_the_same_picture_at_any_band_split() {
        let m = cube();
        let one = render(&m, None, &cam(), &look(), 64, 40, 1);
        let two = render(&m, None, &cam(), &look(), 64, 40, 2);
        assert_eq!(one.len(), two.len());
        // Supersampling adds in-between colors along the edges.
        let distinct = |img: &[u8]| {
            let mut v: Vec<[u8; 3]> = img
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| [p[0], p[1], p[2]])
                .collect();
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        assert!(distinct(&two) > distinct(&one));
        assert_eq!(
            render(&m, None, &cam(), &look(), 64, 40, 1),
            one,
            "the same every time"
        );
    }

    #[test]
    fn nearer_surfaces_hide_farther_ones() {
        // Two squares facing the camera: red near, blue far, same size on screen... red wins.
        let mut m = Model::default();
        for (z, color) in [(0.5f32, [1.0, 0.0, 0.0, 1.0]), (-0.5, [0.0, 0.0, 1.0, 1.0])] {
            let base = m.positions.len() as u32;
            m.positions.extend([
                v3(-1.0, -1.0, z),
                v3(1.0, -1.0, z),
                v3(1.0, 1.0, z),
                v3(-1.0, 1.0, z),
            ]);
            m.colors.extend([color; 4]);
            m.triangles
                .extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
        }
        m.finish();
        // Far one drawn last must still lose.
        m.triangles.reverse();
        let c = Camera {
            yaw: 0.0,
            pitch: 0.0,
            distance: 4.0,
            target: V3::default(),
        };
        let img = render(&m, None, &c, &look(), 50, 50, 1);
        let mid = px(&img, 50, 25, 25);
        assert!(mid[0] > 100 && mid[2] < 40, "{mid:?}");
    }

    #[test]
    fn the_camera_can_go_inside_without_trouble() {
        let m = cube();
        let c = Camera {
            distance: 0.5,
            ..cam()
        };
        let img = render(
            &m,
            Some(&m.edges()),
            &c,
            &Look {
                wireframe: true,
                grid: true,
                ..look()
            },
            40,
            30,
            2,
        );
        assert_eq!(img.len(), 40 * 30 * 4);
    }

    #[test]
    fn grid_and_wireframe_show() {
        let mut m = cube();
        m.fit(crate::model::Up::Y);
        let plain = render(&m, None, &cam(), &look(), 80, 60, 1);
        let lines = render(
            &m,
            Some(&m.edges()),
            &cam(),
            &Look {
                wireframe: true,
                grid: true,
                ..look()
            },
            80,
            60,
            1,
        );
        assert_ne!(plain, lines);
    }
}

/// How fast is it? `STAGETUI_3D_BENCH=model.glb cargo test --release -p stagetui-3d bench -- --ignored --nocapture`
#[cfg(test)]
#[test]
#[ignore]
fn bench() {
    let Ok(path) = std::env::var("STAGETUI_3D_BENCH") else {
        return;
    };
    let (m, _) = crate::model::load(std::path::Path::new(&path), None).unwrap();
    let look = Look {
        bg: [26, 27, 38],
        fg: [192, 202, 245],
        accent: [187, 154, 247],
        wireframe: false,
        grid: true,
    };
    for (w, h) in [(960, 544), (1920, 1088)] {
        for ss in [1, 2] {
            let cam = Camera {
                yaw: 0.6,
                pitch: 0.35,
                distance: 3.0,
                target: V3::default(),
            };
            let t = std::time::Instant::now();
            for i in 0..10 {
                let c = Camera {
                    yaw: cam.yaw + i as f32 * 0.1,
                    ..cam
                };
                std::hint::black_box(render(&m, None, &c, &look, w, h, ss));
            }
            println!(
                "{w}x{h} ss{ss}: {:.1} ms a frame ({} triangles)",
                t.elapsed().as_secs_f64() * 100.0,
                m.triangles.len()
            );
        }
    }
}
