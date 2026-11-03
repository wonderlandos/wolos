// SPDX-License-Identifier: BSD-2-Clause
// libWolkit uber-pipeline, fragment stage (plan section 2.6, src/gpu/mod.rs).
//
// Every primitive is evaluated the way src/cpu/raster.rs evaluates it, so the GPU image stays
// within the golden tolerance of the CPU reference (at most 0.1% of pixels off, by at most 2 per
// channel): signed-distance rounded rectangles with `0.5 - d` coverage quantised to 0..255,
// integer premultiplication and `round(x / 255)`, gradient `t` quantised before the mix, glyph
// and shadow coverage fetched from R8 atlases (shadow masks are the CPU's own three-box blur,
// nine-sliced), and clip coverage multiplied in integers. `precise` keeps the compiler from
// fusing multiply-adds, which would move edge pixels.
#version 450

layout(set = 0, binding = 0, std430) readonly buffer Clips { vec4 rows[]; } clips;
layout(set = 0, binding = 1) uniform sampler2DArray glyph_pages;
layout(set = 0, binding = 2) uniform sampler2D shadow_atlas;
layout(set = 0, binding = 3) uniform sampler2D layer_src;

layout(push_constant) uniform Push {
    vec2 offset;
    vec2 target;
} pc;

layout(location = 0) flat in vec4 v_rect;
layout(location = 1) flat in vec4 v_extra;
layout(location = 2) flat in uvec4 v_colors;
layout(location = 3) flat in uvec4 v_radii;
layout(location = 4) flat in uvec4 v_uv;
layout(location = 5) flat in vec2 v_lo;

layout(location = 0) out vec4 o_color;

const uint K_RECT = 0u;
const uint K_GRADIENT = 1u;
const uint K_BORDER = 2u;
const uint K_SHADOW = 3u;
const uint K_GLYPH = 4u;
const uint K_LAYER = 5u;
const uint ABSOLUTE = 1024u;

uint div255(uint x) {
    uint t = x + 128u;
    return (t + (t >> 8)) >> 8;
}

uint cov_u8(float c) {
    precise float v = clamp(c, 0.0, 1.0) * 255.0 + 0.5;
    return uint(v);
}

float rrect_cov(vec2 p, vec4 r, vec4 radii) {
    precise float hw = r.z * 0.5;
    precise float hh = r.w * 0.5;
    precise float qx = p.x - (r.x + hw);
    precise float qy = p.y - (r.y + hh);
    float rad;
    if (qx < 0.0 && qy < 0.0) {
        rad = radii.x;
    } else if (qy < 0.0) {
        rad = radii.y;
    } else if (qx >= 0.0) {
        rad = radii.z;
    } else {
        rad = radii.w;
    }
    precise float dx = abs(qx) - (hw - rad);
    precise float dy = abs(qy) - (hh - rad);
    precise float ox = max(dx, 0.0);
    precise float oy = max(dy, 0.0);
    precise float l = sqrt(ox * ox + oy * oy);
    precise float d = l + min(max(dx, dy), 0.0) - rad;
    return clamp(0.5 - d, 0.0, 1.0);
}

uvec4 unpack(uint v) {
    return uvec4(v & 255u, (v >> 8) & 255u, (v >> 16) & 255u, v >> 24);
}

uvec4 premul(uvec4 c) {
    return uvec4(div255(c.r * c.a), div255(c.g * c.a), div255(c.b * c.a), c.a);
}

uvec4 mix_u8(uvec4 a, uvec4 b, uint t) {
    return uvec4(
        div255(a.r * (255u - t) + b.r * t),
        div255(a.g * (255u - t) + b.g * t),
        div255(a.b * (255u - t) + b.b * t),
        div255(a.a * (255u - t) + b.a * t));
}

uint fetch_r8(float v) {
    return uint(v * 255.0 + 0.5);
}

int slice(int m, int split, int stretch) {
    if (m < split) {
        return m;
    }
    if (m < split + stretch) {
        return split;
    }
    return m - stretch;
}

void main() {
    uint params = v_colors.w;
    uint kind = params & 15u;
    vec2 frag = gl_FragCoord.xy;
    precise vec2 p = frag - pc.offset;
    ivec2 pix = ivec2(floor(frag));
    ivec2 m = pix - ivec2(v_lo);
    vec4 radii = vec4(v_radii) / 16.0;
    uvec4 src;
    uint cov = 255u;
    if (kind == K_RECT) {
        cov = cov_u8(rrect_cov(p, v_rect, radii));
        src = premul(unpack(v_colors.x));
    } else if (kind == K_GRADIENT) {
        cov = cov_u8(rrect_cov(p, v_rect, radii));
        vec4 f = vec4(v_uv) / 65535.0;
        precise vec2 a = v_rect.xy + f.xy * v_rect.zw;
        precise vec2 b = v_rect.xy + f.zw * v_rect.zw;
        precise vec2 d = b - a;
        precise float dd = d.x * d.x + d.y * d.y;
        precise float inv = dd > 0.0 ? 1.0 / dd : 0.0;
        precise float t = clamp(((p.x - a.x) * d.x + (p.y - a.y) * d.y) * inv, 0.0, 1.0);
        src = premul(mix_u8(unpack(v_colors.x), unpack(v_colors.y), cov_u8(t)));
    } else if (kind == K_BORDER) {
        precise float bw = min(v_extra.x, min(v_rect.z, v_rect.w) * 0.5);
        float co = rrect_cov(p, v_rect, radii);
        precise vec4 inner = vec4(v_rect.xy + bw, v_rect.zw - 2.0 * bw);
        float ci = 0.0;
        if (inner.z > 0.0 && inner.w > 0.0) {
            ci = rrect_cov(p, inner, max(radii - bw, vec4(0.0)));
        }
        precise float c = co * (1.0 - ci);
        cov = cov_u8(c);
        src = premul(unpack(v_colors.x));
    } else if (kind == K_SHADOW) {
        ivec2 sz = ivec2(v_rect.zw);
        if (m.x < 0 || m.y < 0 || m.x >= sz.x || m.y >= sz.y) {
            discard;
        }
        ivec4 e = ivec4(v_extra);
        ivec2 q = ivec2(slice(m.x, e.x, e.z), slice(m.y, e.y, e.w));
        cov = fetch_r8(texelFetch(shadow_atlas, ivec2(v_uv.xy) + q, 0).r);
        src = premul(unpack(v_colors.x));
    } else if (kind == K_GLYPH) {
        if (m.x < 0 || m.y < 0 || m.x >= int(v_uv.z) || m.y >= int(v_uv.w)) {
            discard;
        }
        int page = int((params >> 4) & 63u);
        cov = fetch_r8(texelFetch(glyph_pages, ivec3(ivec2(v_uv.xy) + m, page), 0).r);
        src = premul(unpack(v_colors.x));
    } else {
        // K_LAYER: composite an offscreen layer (premultiplied) with alpha color0.a.
        vec4 t = texelFetch(layer_src, pix, 0);
        uvec4 s = uvec4(fetch_r8(t.r), fetch_r8(t.g), fetch_r8(t.b), fetch_r8(t.a));
        if (s.a == 0u) {
            discard;
        }
        src = s;
        cov = unpack(v_colors.x).a;
    }
    uint clip = v_colors.z;
    if (clip != 0u && kind != K_LAYER) {
        vec4 cr = clips.rows[clip * 3u];
        vec4 rr = clips.rows[clip * 3u + 1u];
        float radius = clips.rows[clip * 3u + 2u].x;
        // The row's rectangle is the intersection of every clip and shares its edges with the
        // rounded clip inside it, so the two coverages are combined with min (a product would
        // count a shared fractional edge twice); one of them is 0 or 255 except on such edges.
        uint c = cov_u8(rrect_cov(p, cr, vec4(0.0)));
        if (radius > 0.0) {
            c = min(c, cov_u8(rrect_cov(p, rr, vec4(radius))));
        }
        cov = div255(cov * c);
    }
    if (cov == 0u) {
        discard;
    }
    uvec4 o = uvec4(div255(src.r * cov), div255(src.g * cov), div255(src.b * cov), div255(src.a * cov));
    // A quarter step up: drivers that truncate instead of rounding on the float-to-unorm write
    // still store exactly `o` (v + 0.25 truncates and rounds to v; 255.25 / 255 clamps to 1).
    o_color = (vec4(o) + 0.25) / 255.0;
}
