// SPDX-License-Identifier: BSD-2-Clause
//! Motion: easing curves, the spring integrator, and the plan of what animates when a node's
//! style changes. The animation clock itself belongs to the engine loop; this module is the
//! arithmetic and the decision, both pure.
use super::prop::PropId;
use super::tokens::{SpringParams, spring_params};
use super::value::*;

/// Control points of the cubic Bezier behind an ease (CSS `cubic-bezier(x1, y1, x2, y2)`).
pub fn bezier(e: Ease) -> Option<[f32; 4]> {
    match e {
        // `--ease` of the theme contract.
        Ease::Standard => Some([0.2, 0.8, 0.2, 1.0]),
        Ease::Linear => None,
        Ease::In => Some([0.4, 0.0, 1.0, 1.0]),
        Ease::Out => Some([0.0, 0.0, 0.2, 1.0]),
        Ease::InOut => Some([0.4, 0.0, 0.2, 1.0]),
    }
}

fn bez(a: f32, b: f32, t: f32) -> f32 {
    // Coordinates of a unit Bezier with control values a and b, at parameter t.
    let u = 1.0 - t;
    3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
}

fn bez_d(a: f32, b: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * a + 6.0 * u * t * (b - a) + 3.0 * t * t * (1.0 - b)
}

/// Progress (0 to 1) of an ease at time fraction `t` (clamped to 0..1).
pub fn ease(e: Ease, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let Some([x1, y1, x2, y2]) = bezier(e) else { return t };
    // Solve x(s) = t for s: Newton steps, then bisection when the slope is flat.
    let mut s = t;
    for _ in 0..8 {
        let err = bez(x1, x2, s) - t;
        if err.abs() < 1e-6 {
            return bez(y1, y2, s);
        }
        let d = bez_d(x1, x2, s);
        if d.abs() < 1e-6 {
            break;
        }
        s = (s - err / d).clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    s = t;
    for _ in 0..40 {
        let x = bez(x1, x2, s);
        if (x - t).abs() < 1e-6 {
            break;
        }
        if x < t {
            lo = s;
        } else {
            hi = s;
        }
        s = (lo + hi) / 2.0;
    }
    bez(y1, y2, s)
}

/// A damped spring toward a target: position and velocity per unit of the animated value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringState {
    pub pos: f32,
    pub vel: f32,
}

impl SpringState {
    pub fn at_rest(&self, target: f32) -> bool {
        (self.pos - target).abs() < 0.001 && self.vel.abs() < 0.01
    }
}

/// Advance a spring by `dt` seconds (sub-stepped at 240 Hz, semi-implicit Euler: stable for every
/// preset). A spring that is at rest snaps to the target.
pub fn spring_step(s: SpringState, target: f32, p: SpringParams, dt: f32) -> SpringState {
    let mut s = s;
    let step = 1.0 / 240.0;
    let mut left = dt.clamp(0.0, 0.25);
    while left > 0.0 {
        let h = left.min(step);
        let force = -p.stiffness * (s.pos - target) - p.damping * s.vel;
        s.vel += force / p.mass * h;
        s.pos += s.vel * h;
        left -= h;
    }
    if s.at_rest(target) {
        SpringState { pos: target, vel: 0.0 }
    } else {
        s
    }
}

/// How one property moves from its old value to its new one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    Curve { ms: u16, ease: Ease },
    Spring(SpringParams),
}

/// The properties that animate when a node enters `new` from `old`, and how. Empty under reduced
/// motion. A property animates when it changed, the new style lists it in `transition`, and
/// (for a spring) only numeric properties follow the spring.
pub fn plan(old: &Props, new: &Props, reduce_motion: bool) -> (u128, Motion) {
    let tr = new.motion.transition;
    let motion = match spring_params(new.motion.spring) {
        Some(p) => Motion::Spring(p),
        None => Motion::Curve { ms: tr.ms, ease: tr.ease },
    };
    if reduce_motion || tr.props == 0 {
        return (0, motion);
    }
    let changed = super::select::diff(old, new);
    (changed & tr.props, motion)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// The numeric part of a style at progress `t` between two styles, for the properties in `mask`.
/// Colour references cannot be mixed without a theme; a colour switches at `t >= 0.5` here and the
/// painter mixes the resolved colours itself using the same `t`.
pub fn blend(from: &Props, to: &Props, t: f32, mask: u128) -> Props {
    use PropId as P;
    let mut p = to.clone();
    let on = |id: PropId| mask & id.bit() != 0;
    let pick = t >= 0.5;
    if on(P::Opacity) {
        p.paint.opacity = lerp(from.paint.opacity, to.paint.opacity, t);
    }
    if on(P::OutlineWidth) {
        p.paint.outline_width = lerp(from.paint.outline_width, to.paint.outline_width, t);
    }
    for (i, id) in [P::RadiusTl, P::RadiusTr, P::RadiusBr, P::RadiusBl].into_iter().enumerate() {
        if on(id) {
            p.paint.radius[i] = lerp(from.paint.radius[i], to.paint.radius[i], t);
        }
    }
    if on(P::Transform) {
        let (a, b) = (from.paint.transform, to.paint.transform);
        p.paint.transform = Transform {
            tx: lerp(a.tx, b.tx, t),
            ty: lerp(a.ty, b.ty, t),
            sx: lerp(a.sx, b.sx, t),
            sy: lerp(a.sy, b.sy, t),
            rotate: lerp(a.rotate, b.rotate, t),
        };
    }
    if on(P::BoxShadow) {
        for i in 0..2 {
            let (a, b) = (from.paint.shadow[i], to.paint.shadow[i]);
            p.paint.shadow[i] = Shadow { dx: lerp(a.dx, b.dx, t), dy: lerp(a.dy, b.dy, t), blur: lerp(a.blur, b.blur, t), color: if pick { b.color } else { a.color } };
        }
    }
    if on(P::Background) && !pick {
        p.paint.background = from.paint.background;
    }
    if on(P::BorderColor) && !pick {
        p.paint.border_color = from.paint.border_color;
    }
    if on(P::OutlineColor) && !pick {
        p.paint.outline_color = from.paint.outline_color;
    }
    if on(P::Color) && !pick {
        p.text.color = from.text.color;
    }
    p
}
