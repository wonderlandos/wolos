// Liquid glass over the blurred backdrop: rounded-rect SDF mask, edge lensing along the SDF
// normal, chromatic split at the rim, saturation boost, tint, specular rim light.
// Same SDF as glass::sdf_rounded_rect (unit-tested there).
uniform vec2 size;
uniform vec4 uv_rect;
uniform float radius;
uniform float edge;
uniform float refraction;
uniform float specular;
uniform float chroma;
uniform vec4 glass_tint;
uniform float saturation;
#define TEX_LO uv_rect.xy
#define TEX_HI (uv_rect.xy + uv_rect.zw)

float sdf(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    vec2 half_size = size * 0.5;
    vec2 p = (v_coords - uv_rect.xy) / uv_rect.zw * size - half_size;
    float d = sdf(p, half_size, radius);
    float cover = clamp(0.5 - d, 0.0, 1.0);
    if (cover <= 0.0) {
        gl_FragColor = vec4(0.0);
        return;
    }
    vec2 n = vec2(sdf(p + vec2(1.0, 0.0), half_size, radius) - sdf(p - vec2(1.0, 0.0), half_size, radius),
                  sdf(p + vec2(0.0, 1.0), half_size, radius) - sdf(p - vec2(0.0, 1.0), half_size, radius));
    n = n / max(length(n), 1e-4);
    float band = 1.0 - clamp(-d / max(edge, 1e-3), 0.0, 1.0);
    band = band * band;
    vec2 px = uv_rect.zw / size;
    vec2 uv = v_coords - n * band * refraction * px;
    vec2 ca = n * band * chroma * px;
    vec3 c = vec3(TEX(uv + ca).r, TEX(uv).g, TEX(uv - ca).b);
    float luma = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c = mix(vec3(luma), c, saturation);
    c = mix(c, glass_tint.rgb, glass_tint.a);
    float rim = 1.0 - clamp(-d / 1.5, 0.0, 1.0);
    float light = 0.25 + 0.75 * max(dot(n, vec2(-0.6, -0.8)), 0.0);
    c += specular * rim * light;
    gl_FragColor = vec4(c, 1.0) * cover * alpha;
}
