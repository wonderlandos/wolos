// SPDX-License-Identifier: BSD-2-Clause
//! Exact-area path coverage: flatten a path to lines and accumulate signed area per pixel.
//!
//! Only IEEE-754 `+ - * /`, `floor`, `ceil`, `abs`, `min`, `max` and `sqrt` are used, in a fixed
//! order, so the output bytes are identical on every machine and build (no fused multiply-add,
//! no libm transcendental). The fill rule is "non-zero, clamped": overlapping contours wound the
//! same way saturate at full coverage, which is what font outlines need.

/// Flattening tolerance in pixels.
const TOLERANCE: f32 = 0.05;
/// Upper bound on the pieces one curve is cut into.
const MAX_SEGMENTS: u32 = 64;

/// A reusable signed-area accumulation buffer for one `w x h` pixel mask.
#[derive(Default)]
pub struct Accumulator {
    w: usize,
    h: usize,
    stride: usize,
    acc: Vec<f32>,
}

impl Accumulator {
    pub fn new() -> Accumulator {
        Accumulator::default()
    }

    /// Prepares a zeroed `w x h` mask, reusing the allocation when it is big enough.
    pub fn reset(&mut self, w: usize, h: usize) {
        self.w = w;
        self.h = h;
        self.stride = w + 2;
        let n = self.stride * h + 2;
        self.acc.clear();
        self.acc.resize(n, 0.0);
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    /// Adds one directed line, clipped to the mask.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            return;
        }
        if y0 == y1 || self.w == 0 || self.h == 0 {
            return;
        }
        let wf = self.w as f32;
        // Orient top to bottom; the sign carries the original direction.
        let (sign, tx, ty, bx, by) = if y0 < y1 { (1.0f32, x0, y0, x1, y1) } else { (-1.0, x1, y1, x0, y0) };
        let dxdy = (bx - tx) / (by - ty);
        // Break the line where it crosses x = 0 and x = w; every piece then lies wholly on one
        // side, so clamping its end points to [0, w] is exact (outside pieces become vertical
        // edges on the border).
        let mut ys = [ty, ty, ty, by];
        let mut n = 1;
        if dxdy != 0.0 {
            for c in [0.0f32, wf] {
                let yc = ty + (c - tx) / dxdy;
                if yc > ty && yc < by {
                    ys[n] = yc;
                    n += 1;
                }
            }
        }
        ys[n] = by;
        let ys = &mut ys[..=n];
        if n == 3 && ys[1] > ys[2] {
            ys.swap(1, 2);
        }
        for i in 0..n {
            let (ya, yb) = (ys[i], ys[i + 1]);
            if yb <= ya {
                continue;
            }
            let xa = (tx + dxdy * (ya - ty)).clamp(0.0, wf);
            let xb = (tx + dxdy * (yb - ty)).clamp(0.0, wf);
            self.line_inner(xa, ya, xb, yb, sign);
        }
    }

    /// `p0.y < p1.y`, x within `[0, w]`; `dir` is +1 or -1.
    fn line_inner(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, dir: f32) {
        // Same orientation convention as the unsplit call: a line going up the page counts as -1.
        if y1 <= y0 || y1 <= 0.0 || y0 >= self.h as f32 {
            return;
        }
        let dxdy = (x1 - x0) / (y1 - y0);
        let mut x = x0;
        let mut ystart = y0;
        if ystart < 0.0 {
            x -= ystart * dxdy;
            ystart = 0.0;
        }
        let row0 = ystart.floor() as usize;
        let row_end = self.h.min(y1.ceil() as usize);
        let wf = self.w as f32;
        for y in row0..row_end {
            let top = (y as f32).max(ystart);
            let bot = ((y + 1) as f32).min(y1);
            let dy = bot - top;
            if dy <= 0.0 {
                continue;
            }
            let xnext = (x + dxdy * dy).clamp(0.0, wf);
            let xc = x.clamp(0.0, wf);
            let d = dy * dir;
            let (xa, xb) = if xc < xnext { (xc, xnext) } else { (xnext, xc) };
            let xaf = xa.floor();
            let xai = xaf as usize;
            let xbc = xb.ceil();
            let xbi = xbc as usize;
            let base = y * self.stride;
            if xbi <= xai + 1 {
                let xmf = 0.5 * (xc + xnext) - xaf;
                self.acc[base + xai] += d - d * xmf;
                self.acc[base + xai + 1] += d * xmf;
            } else {
                let s = 1.0 / (xb - xa);
                let xaf_frac = xa - xaf;
                let a0 = 0.5 * s * (1.0 - xaf_frac) * (1.0 - xaf_frac);
                let xbf = xb - xbc + 1.0;
                let am = 0.5 * s * xbf * xbf;
                self.acc[base + xai] += d * a0;
                if xbi == xai + 2 {
                    self.acc[base + xai + 1] += d * (1.0 - a0 - am);
                } else {
                    let a1 = s * (1.5 - xaf_frac);
                    self.acc[base + xai + 1] += d * (a1 - a0);
                    for xi in (xai + 2)..(xbi - 1) {
                        self.acc[base + xi] += d * s;
                    }
                    let a2 = a1 + (xbi - xai - 3) as f32 * s;
                    self.acc[base + xbi - 1] += d * (1.0 - a2 - am);
                }
                self.acc[base + xbi] += d * am;
            }
            x += dxdy * dy;
        }
    }

    /// Writes the coverage (`0..=255`) of the accumulated mask into `out` (`w * h` bytes).
    pub fn finish(&self, out: &mut [u8]) {
        debug_assert!(out.len() >= self.w * self.h);
        for y in 0..self.h {
            let mut sum = 0.0f32;
            let row = &self.acc[y * self.stride..y * self.stride + self.w];
            let dst = &mut out[y * self.w..(y + 1) * self.w];
            for (o, a) in dst.iter_mut().zip(row) {
                sum += *a;
                let v = sum.abs().min(1.0);
                *o = (v * 255.0 + 0.5) as u8;
            }
        }
    }
}

/// Receives a path and writes its flattened lines into an [`Accumulator`] after an affine
/// `x' = x * sx + ox`, `y' = oy - y * sy` (font units are y-up, masks are y-down).
pub struct Flattener<'a> {
    acc: &'a mut Accumulator,
    sx: f32,
    sy: f32,
    ox: f32,
    oy: f32,
    start: (f32, f32),
    cur: (f32, f32),
    open: bool,
}

impl<'a> Flattener<'a> {
    pub fn new(acc: &'a mut Accumulator, sx: f32, sy: f32, ox: f32, oy: f32) -> Flattener<'a> {
        Flattener { acc, sx, sy, ox, oy, start: (0.0, 0.0), cur: (0.0, 0.0), open: false }
    }

    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.sx + self.ox, self.oy - y * self.sy)
    }

    fn emit(&mut self, to: (f32, f32)) {
        self.acc.line(self.cur.0, self.cur.1, to.0, to.1);
        self.cur = to;
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        let p = self.map(x, y);
        self.start = p;
        self.cur = p;
        self.open = true;
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        let p = self.map(x, y);
        self.emit(p);
    }

    pub fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let c = self.map(cx, cy);
        let p = self.map(x, y);
        let p0 = self.cur;
        let ddx = p0.0 - 2.0 * c.0 + p.0;
        let ddy = p0.1 - 2.0 * c.1 + p.1;
        let dd = (ddx * ddx + ddy * ddy).sqrt();
        let n = segments(0.25 * dd);
        for i in 1..n {
            let t = i as f32 / n as f32;
            let mt = 1.0 - t;
            let px = mt * mt * p0.0 + 2.0 * mt * t * c.0 + t * t * p.0;
            let py = mt * mt * p0.1 + 2.0 * mt * t * c.1 + t * t * p.1;
            self.emit((px, py));
        }
        self.emit(p);
    }

    pub fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let c1 = self.map(c1x, c1y);
        let c2 = self.map(c2x, c2y);
        let p = self.map(x, y);
        let p0 = self.cur;
        let d1x = p0.0 - 2.0 * c1.0 + c2.0;
        let d1y = p0.1 - 2.0 * c1.1 + c2.1;
        let d2x = c1.0 - 2.0 * c2.0 + p.0;
        let d2y = c1.1 - 2.0 * c2.1 + p.1;
        let dd = (d1x * d1x + d1y * d1y).max(d2x * d2x + d2y * d2y).sqrt();
        let n = segments(0.75 * dd);
        for i in 1..n {
            let t = i as f32 / n as f32;
            let mt = 1.0 - t;
            let a = mt * mt * mt;
            let b = 3.0 * mt * mt * t;
            let c = 3.0 * mt * t * t;
            let d = t * t * t;
            let px = a * p0.0 + b * c1.0 + c * c2.0 + d * p.0;
            let py = a * p0.1 + b * c1.1 + c * c2.1 + d * p.1;
            self.emit((px, py));
        }
        self.emit(p);
    }

    pub fn close(&mut self) {
        if self.open && self.cur != self.start {
            let s = self.start;
            self.emit(s);
        }
        self.open = false;
    }
}

fn segments(deviation: f32) -> u32 {
    if !deviation.is_finite() {
        return 1;
    }
    let n = (deviation / TOLERANCE).sqrt().ceil();
    (n as u32).clamp(1, MAX_SEGMENTS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(acc: &mut Accumulator, x0: f32, y0: f32, x1: f32, y1: f32) {
        acc.line(x0, y0, x1, y0);
        acc.line(x1, y0, x1, y1);
        acc.line(x1, y1, x0, y1);
        acc.line(x0, y1, x0, y0);
    }

    #[test]
    fn pixel_aligned_square_is_full() {
        let mut a = Accumulator::new();
        a.reset(8, 8);
        square(&mut a, 2.0, 2.0, 6.0, 6.0);
        let mut out = vec![0u8; 64];
        a.finish(&mut out);
        for y in 0..8 {
            for x in 0..8 {
                let inside = (2..6).contains(&x) && (2..6).contains(&y);
                assert_eq!(out[y * 8 + x], if inside { 255 } else { 0 }, "{x},{y}");
            }
        }
    }

    #[test]
    fn half_pixel_edges_give_half_coverage() {
        let mut a = Accumulator::new();
        a.reset(4, 4);
        square(&mut a, 0.5, 0.0, 3.5, 4.0);
        let mut out = vec![0u8; 16];
        a.finish(&mut out);
        assert_eq!(out[0], 128);
        assert_eq!(out[1], 255);
        assert_eq!(out[3], 128);
    }

    #[test]
    fn clipping_outside_the_mask_keeps_inside_exact() {
        let mut a = Accumulator::new();
        a.reset(4, 4);
        square(&mut a, -10.0, -10.0, 20.0, 2.0);
        let mut out = vec![0u8; 16];
        a.finish(&mut out);
        assert!(out[..8].iter().all(|&v| v == 255), "{out:?}");
        assert!(out[8..].iter().all(|&v| v == 0), "{out:?}");
    }

    #[test]
    fn winding_direction_does_not_matter_for_abs() {
        let mut a = Accumulator::new();
        a.reset(4, 4);
        square(&mut a, 1.0, 1.0, 3.0, 3.0);
        let mut b = Accumulator::new();
        b.reset(4, 4);
        b.line(1.0, 1.0, 1.0, 3.0);
        b.line(1.0, 3.0, 3.0, 3.0);
        b.line(3.0, 3.0, 3.0, 1.0);
        b.line(3.0, 1.0, 1.0, 1.0);
        let (mut oa, mut ob) = (vec![0u8; 16], vec![0u8; 16]);
        a.finish(&mut oa);
        b.finish(&mut ob);
        assert_eq!(oa, ob);
    }

    #[test]
    fn circle_area_is_close() {
        let mut a = Accumulator::new();
        a.reset(32, 32);
        {
            let mut f = Flattener::new(&mut a, 1.0, 1.0, 0.0, 32.0);
            // Four quadratic arcs approximating a circle r=10 around (16,16) in y-up space.
            let (cx, cy, r) = (16.0f32, 16.0f32, 10.0f32);
            let k = r * 0.414_213_57;
            f.move_to(cx + r, cy);
            f.quad_to(cx + r, cy + k, cx + r * 0.707_106_8, cy + r * 0.707_106_8);
            f.quad_to(cx + k, cy + r, cx, cy + r);
            f.quad_to(cx - k, cy + r, cx - r * 0.707_106_8, cy + r * 0.707_106_8);
            f.quad_to(cx - r, cy + k, cx - r, cy);
            f.quad_to(cx - r, cy - k, cx - r * 0.707_106_8, cy - r * 0.707_106_8);
            f.quad_to(cx - k, cy - r, cx, cy - r);
            f.quad_to(cx + k, cy - r, cx + r * 0.707_106_8, cy - r * 0.707_106_8);
            f.quad_to(cx + r, cy - k, cx + r, cy);
            f.close();
        }
        let mut out = vec![0u8; 32 * 32];
        a.finish(&mut out);
        let area: f32 = out.iter().map(|&v| f32::from(v) / 255.0).sum();
        let want = std::f32::consts::PI * 100.0;
        assert!((area - want).abs() < 2.0, "area {area} want {want}");
    }
}
