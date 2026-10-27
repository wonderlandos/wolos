// Dual-Kawase downsample: 5 taps, writes into a half-size target.
uniform vec2 halfpixel;
uniform float offset;
#define TEX_LO halfpixel
#define TEX_HI (vec2(1.0) - halfpixel)

void main() {
    vec2 o = halfpixel * offset;
    vec4 sum = TEX(v_coords) * 4.0;
    sum += TEX(v_coords - o);
    sum += TEX(v_coords + o);
    sum += TEX(v_coords + vec2(o.x, -o.y));
    sum += TEX(v_coords - vec2(o.x, -o.y));
    gl_FragColor = sum / 8.0 * alpha;
}
