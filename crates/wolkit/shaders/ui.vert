// SPDX-License-Identifier: BSD-2-Clause
// libWolkit uber-pipeline, vertex stage (plan section 2.6, src/gpu/mod.rs).
//
// One 64-byte instance (src/paint/batch.rs `Instance`) becomes one quad covering exactly the
// pixels the CPU rasteriser touches for it: floor of the origin to ceil of the far edge. The
// quad is a 4-vertex triangle strip with no vertex buffer; everything else is passed flat.
#version 450

layout(location = 0) in vec4 i_rect;
layout(location = 1) in vec4 i_extra;
layout(location = 2) in uint i_color0;
layout(location = 3) in uint i_color1;
layout(location = 4) in uvec4 i_radii;
layout(location = 5) in uvec4 i_uv;
layout(location = 6) in uint i_clip;
layout(location = 7) in uint i_params;

layout(push_constant) uniform Push {
    vec2 offset; // chunk transform, device pixels
    vec2 target; // render target size, pixels
} pc;

layout(location = 0) flat out vec4 v_rect;
layout(location = 1) flat out vec4 v_extra;
layout(location = 2) flat out uvec4 v_colors; // color0, color1, clip, params
layout(location = 3) flat out uvec4 v_radii;
layout(location = 4) flat out uvec4 v_uv;
layout(location = 5) flat out vec2 v_lo;

// Bit 10 of params: the rectangle is already in absolute window pixels (shadow masks).
const uint ABSOLUTE = 1024u;
const uint K_GLYPH = 4u;

void main() {
    bool absolute = (i_params & ABSOLUTE) != 0u;
    // Glyphs carry the sub-pixel part of the offset in their position already (batch.rs), so
    // they move by the whole-pixel part only and stay on the CPU rasteriser's pixel grid.
    vec2 off = absolute ? vec2(0.0) : ((i_params & 15u) == K_GLYPH ? floor(pc.offset) : pc.offset);
    vec2 lo = floor(i_rect.xy + off);
    vec2 hi = ceil(i_rect.xy + i_rect.zw + off);
    vec2 corner = vec2(float(gl_VertexIndex & 1), float((gl_VertexIndex >> 1) & 1));
    vec2 p = mix(lo, hi, corner);
    gl_Position = vec4(p / pc.target * 2.0 - 1.0, 0.0, 1.0);
    v_rect = i_rect;
    v_extra = i_extra;
    v_colors = uvec4(i_color0, i_color1, i_clip, i_params);
    v_radii = i_radii;
    v_uv = i_uv;
    v_lo = lo;
}
