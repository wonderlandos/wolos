// Dual-Kawase upsample: 8 taps, writes into a double-size target.
uniform vec2 halfpixel;
uniform float offset;
#define TEX_LO halfpixel
#define TEX_HI (vec2(1.0) - halfpixel)

void main() {
    vec2 o = halfpixel * offset;
    vec4 sum = TEX(v_coords + vec2(-o.x * 2.0, 0.0));
    sum += TEX(v_coords + vec2(-o.x, o.y)) * 2.0;
    sum += TEX(v_coords + vec2(0.0, o.y * 2.0));
    sum += TEX(v_coords + vec2(o.x, o.y)) * 2.0;
    sum += TEX(v_coords + vec2(o.x * 2.0, 0.0));
    sum += TEX(v_coords + vec2(o.x, -o.y)) * 2.0;
    sum += TEX(v_coords + vec2(0.0, -o.y * 2.0));
    sum += TEX(v_coords + vec2(-o.x, -o.y)) * 2.0;
    gl_FragColor = sum / 12.0 * alpha;
}
