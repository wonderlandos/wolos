// SPDX-License-Identifier: BSD-2-Clause
// libWolkit uber-pipeline in WGSL, for wgpu (Vulkan natively, WebGPU in the browser). A line by
// line port of crates/wolkit/shaders/ui.vert and ui.frag (the Vulkan renderer's SPIR-V), so both
// GPU backends evaluate every primitive the way src/cpu/raster.rs does: signed-distance rounded
// rectangles with `0.5 - d` coverage quantised to 0..255, integer premultiplication and
// `round(x / 255)`, glyph and shadow coverage from R8 atlases, pictures sampled bilinearly from
// premultiplied RGBA8 pages with weights quantised to 0..255, clip coverage multiplied in integers.
// tests/wgsl_port.rs holds this file to the GLSL: same constants, same bindings, same kinds.
//
// Differences from the GLSL, all forced by WebGPU:
// * no push constants: `Push` is a uniform at a dynamic offset (one 256-byte slot per draw);
// * clip space y points up, so the vertex stage flips y (framebuffer pixels are still top-left);
// * no `precise`: WGSL has no such qualifier (the golden tolerance absorbs a fused multiply-add);
// * texelFetch is textureLoad, and the textures have no sampler.

struct Push {
    offset: vec2<f32>, // chunk transform, device pixels
    target_size: vec2<f32>, // render target size, pixels
};

@group(0) @binding(0) var<storage, read> clips: array<vec4<f32>>;
@group(0) @binding(1) var glyph_pages: texture_2d_array<f32>;
@group(0) @binding(2) var shadow_atlas: texture_2d<f32>;
@group(0) @binding(3) var layer_src: texture_2d<f32>;
@group(0) @binding(4) var image_pages: texture_2d_array<f32>;
@group(1) @binding(0) var<uniform> pc: Push;

struct Inst {
    @location(0) rect: vec4<f32>,
    @location(1) extra: vec4<f32>,
    @location(2) color0: u32,
    @location(3) color1: u32,
    @location(4) radii: vec4<u32>,
    @location(5) uv: vec4<u32>,
    @location(6) clip: u32,
    @location(7) params: u32,
};

struct Vary {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) rect: vec4<f32>,
    @location(1) @interpolate(flat) extra: vec4<f32>,
    @location(2) @interpolate(flat) colors: vec4<u32>, // color0, color1, clip, params
    @location(3) @interpolate(flat) radii: vec4<u32>,
    @location(4) @interpolate(flat) uv: vec4<u32>,
    @location(5) @interpolate(flat) lo: vec2<f32>,
};

const K_RECT: u32 = 0u;
const K_GRADIENT: u32 = 1u;
const K_BORDER: u32 = 2u;
const K_SHADOW: u32 = 3u;
const K_GLYPH: u32 = 4u;
const K_LAYER: u32 = 5u;
const K_IMAGE: u32 = 6u;
// Bit 10 of params: the rectangle is already in absolute window pixels (shadow masks).
const ABSOLUTE: u32 = 1024u;

@vertex
fn vs(@builtin(vertex_index) vi: u32, i: Inst) -> Vary {
    let absolute = (i.params & ABSOLUTE) != 0u;
    // Glyphs carry the sub-pixel part of the offset in their position already (batch.rs), so
    // they move by the whole-pixel part only and stay on the CPU rasteriser's pixel grid.
    var off = pc.offset;
    if (absolute) {
        off = vec2<f32>(0.0);
    } else if ((i.params & 15u) == K_GLYPH) {
        off = floor(pc.offset);
    }
    let lo = floor(i.rect.xy + off);
    let hi = ceil(i.rect.xy + i.rect.zw + off);
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let p = mix(lo, hi, corner);
    let ndc = p / pc.target_size * 2.0 - 1.0;
    var o: Vary;
    o.pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    o.rect = i.rect;
    o.extra = i.extra;
    o.colors = vec4<u32>(i.color0, i.color1, i.clip, i.params);
    o.radii = i.radii;
    o.uv = i.uv;
    o.lo = lo;
    return o;
}

fn div255(x: u32) -> u32 {
    let t = x + 128u;
    return (t + (t >> 8u)) >> 8u;
}

fn cov_u8(c: f32) -> u32 {
    let v = clamp(c, 0.0, 1.0) * 255.0 + 0.5;
    return u32(v);
}

fn rrect_cov(p: vec2<f32>, r: vec4<f32>, radii: vec4<f32>) -> f32 {
    let hw = r.z * 0.5;
    let hh = r.w * 0.5;
    let qx = p.x - (r.x + hw);
    let qy = p.y - (r.y + hh);
    var rad: f32;
    if (qx < 0.0 && qy < 0.0) {
        rad = radii.x;
    } else if (qy < 0.0) {
        rad = radii.y;
    } else if (qx >= 0.0) {
        rad = radii.z;
    } else {
        rad = radii.w;
    }
    let dx = abs(qx) - (hw - rad);
    let dy = abs(qy) - (hh - rad);
    let ox = max(dx, 0.0);
    let oy = max(dy, 0.0);
    let l = sqrt(ox * ox + oy * oy);
    let d = l + min(max(dx, dy), 0.0) - rad;
    return clamp(0.5 - d, 0.0, 1.0);
}

fn unpack(v: u32) -> vec4<u32> {
    return vec4<u32>(v & 255u, (v >> 8u) & 255u, (v >> 16u) & 255u, v >> 24u);
}

fn premul(c: vec4<u32>) -> vec4<u32> {
    return vec4<u32>(div255(c.r * c.a), div255(c.g * c.a), div255(c.b * c.a), c.a);
}

fn mix_u8(a: vec4<u32>, b: vec4<u32>, t: u32) -> vec4<u32> {
    return vec4<u32>(
        div255(a.r * (255u - t) + b.r * t),
        div255(a.g * (255u - t) + b.g * t),
        div255(a.b * (255u - t) + b.b * t),
        div255(a.a * (255u - t) + b.a * t));
}

fn fetch_r8(v: f32) -> u32 {
    return u32(v * 255.0 + 0.5);
}

fn fetch_rgba(at: vec2<i32>, page: i32) -> vec4<u32> {
    let t = textureLoad(image_pages, at, page, 0);
    return vec4<u32>(fetch_r8(t.r), fetch_r8(t.g), fetch_r8(t.b), fetch_r8(t.a));
}

// One texel of the level at `base` (size `lim + 1`) on `page`, clamped to its edges.
fn texel(base: vec2<i32>, lim: vec2<i32>, q: vec2<i32>, page: i32) -> vec4<u32> {
    return fetch_rgba(base + clamp(q, vec2<i32>(0), lim), page);
}

fn slice(m: i32, split: i32, stretch: i32) -> i32 {
    if (m < split) {
        return m;
    }
    if (m < split + stretch) {
        return split;
    }
    return m - stretch;
}

@fragment
fn fs(v: Vary) -> @location(0) vec4<f32> {
    let params = v.colors.w;
    let kind = params & 15u;
    let frag = v.pos.xy;
    let p = frag - pc.offset;
    let pix = vec2<i32>(floor(frag));
    let m = pix - vec2<i32>(v.lo);
    let radii = vec4<f32>(v.radii) / 16.0;
    var src: vec4<u32>;
    var cov: u32 = 255u;
    if (kind == K_RECT) {
        cov = cov_u8(rrect_cov(p, v.rect, radii));
        src = premul(unpack(v.colors.x));
    } else if (kind == K_GRADIENT) {
        cov = cov_u8(rrect_cov(p, v.rect, radii));
        let f = vec4<f32>(v.uv) / 65535.0;
        let a = v.rect.xy + f.xy * v.rect.zw;
        let b = v.rect.xy + f.zw * v.rect.zw;
        let d = b - a;
        let dd = d.x * d.x + d.y * d.y;
        var inv = 0.0;
        if (dd > 0.0) {
            inv = 1.0 / dd;
        }
        let t = clamp(((p.x - a.x) * d.x + (p.y - a.y) * d.y) * inv, 0.0, 1.0);
        src = premul(mix_u8(unpack(v.colors.x), unpack(v.colors.y), cov_u8(t)));
    } else if (kind == K_BORDER) {
        let bw = min(v.extra.x, min(v.rect.z, v.rect.w) * 0.5);
        let co = rrect_cov(p, v.rect, radii);
        let inner = vec4<f32>(v.rect.xy + bw, v.rect.zw - 2.0 * bw);
        var ci = 0.0;
        if (inner.z > 0.0 && inner.w > 0.0) {
            ci = rrect_cov(p, inner, max(radii - bw, vec4<f32>(0.0)));
        }
        let c = co * (1.0 - ci);
        cov = cov_u8(c);
        src = premul(unpack(v.colors.x));
    } else if (kind == K_SHADOW) {
        let sz = vec2<i32>(v.rect.zw);
        if (m.x < 0 || m.y < 0 || m.x >= sz.x || m.y >= sz.y) {
            discard;
        }
        let e = vec4<i32>(v.extra);
        let q = vec2<i32>(slice(m.x, e.x, e.z), slice(m.y, e.y, e.w));
        cov = fetch_r8(textureLoad(shadow_atlas, vec2<i32>(v.uv.xy) + q, 0).r);
        src = premul(unpack(v.colors.x));
    } else if (kind == K_GLYPH) {
        if (m.x < 0 || m.y < 0 || m.x >= i32(v.uv.z) || m.y >= i32(v.uv.w)) {
            discard;
        }
        let page = i32((params >> 4u) & 63u);
        cov = fetch_r8(textureLoad(glyph_pages, vec2<i32>(v.uv.xy) + m, page, 0).r);
        src = premul(unpack(v.colors.x));
    } else if (kind == K_IMAGE) {
        // v.rect: the part drawn; v.extra: the picture's origin and texels per pixel.
        cov = cov_u8(rrect_cov(p, v.rect, vec4<f32>(0.0)));
        let u = (p.x - v.extra.x) * v.extra.z - 0.5;
        let w = (p.y - v.extra.y) * v.extra.w - 0.5;
        let fu = floor(u);
        let fv = floor(w);
        let wx = cov_u8(u - fu);
        let wy = cov_u8(w - fv);
        let q = vec2<i32>(i32(fu), i32(fv));
        let base = vec2<i32>(v.uv.xy);
        let lim = vec2<i32>(v.uv.zw) - 1;
        let page = i32((params >> 4u) & 63u);
        let top = mix_u8(texel(base, lim, q, page), texel(base, lim, q + vec2<i32>(1, 0), page), wx);
        let bot = mix_u8(texel(base, lim, q + vec2<i32>(0, 1), page), texel(base, lim, q + vec2<i32>(1, 1), page), wx);
        let t = mix_u8(top, bot, wy);
        let a = unpack(v.colors.x).a;
        src = vec4<u32>(div255(t.r * a), div255(t.g * a), div255(t.b * a), div255(t.a * a));
    } else {
        // K_LAYER: composite an offscreen layer (premultiplied) with alpha color0.a.
        let t = textureLoad(layer_src, pix, 0);
        let s = vec4<u32>(fetch_r8(t.r), fetch_r8(t.g), fetch_r8(t.b), fetch_r8(t.a));
        if (s.a == 0u) {
            discard;
        }
        src = s;
        cov = unpack(v.colors.x).a;
    }
    let clip = v.colors.z;
    if (clip != 0u && kind != K_LAYER) {
        let cr = clips[clip * 3u];
        let rr = clips[clip * 3u + 1u];
        let radius = clips[clip * 3u + 2u].x;
        // The row's rectangle is the intersection of every clip and shares its edges with the
        // rounded clip inside it, so the two coverages are combined with min.
        var c = cov_u8(rrect_cov(p, cr, vec4<f32>(0.0)));
        if (radius > 0.0) {
            c = min(c, cov_u8(rrect_cov(p, rr, vec4<f32>(radius))));
        }
        cov = div255(cov * c);
    }
    if (cov == 0u) {
        discard;
    }
    let o = vec4<u32>(div255(src.r * cov), div255(src.g * cov), div255(src.b * cov), div255(src.a * cov));
    // A quarter step up: a float-to-unorm write that truncates still stores exactly `o`.
    return (vec4<f32>(o) + 0.25) / 255.0;
}
