// SPDX-License-Identifier: BSD-2-Clause
//! Motion arithmetic: eases, springs, and what a style change animates.
use wolkit::style::prop::PropId;
use wolkit::style::tokens::*;
use wolkit::style::transition::*;
use wolkit::style::value::*;

const EASES: [Ease; 5] = [
    Ease::Standard,
    Ease::Linear,
    Ease::In,
    Ease::Out,
    Ease::InOut,
];

#[test]
fn eases_start_at_zero_end_at_one_and_never_go_backwards() {
    for e in EASES {
        assert!(ease(e, 0.0).abs() < 1e-4, "{e:?}");
        assert!((ease(e, 1.0) - 1.0).abs() < 1e-4, "{e:?}");
        assert_eq!(ease(e, -1.0), ease(e, 0.0), "clamped below");
        assert_eq!(ease(e, 2.0), ease(e, 1.0), "clamped above");
        let mut prev = -1.0;
        for i in 0..=200 {
            let v = ease(e, i as f32 / 200.0);
            assert!(v >= prev - 1e-4, "{e:?} goes back at {i}: {v} < {prev}");
            assert!((-0.001..=1.001).contains(&v), "{e:?} leaves 0..1: {v}");
            prev = v;
        }
    }
    assert!((ease(Ease::Linear, 0.3) - 0.3).abs() < 1e-6);
}

#[test]
fn the_standard_ease_matches_the_theme_curve() {
    // cubic-bezier(0.2, 0.8, 0.2, 1) is quick at first: well over half way at a third of the time.
    assert!(ease(Ease::Standard, 0.33) > 0.6);
    // Out is faster than linear early, In slower.
    assert!(ease(Ease::Out, 0.25) > 0.25);
    assert!(ease(Ease::In, 0.25) < 0.25);
    // InOut starts slower than linear and has made up for it by three quarters.
    assert!(ease(Ease::InOut, 0.1) < 0.1);
    assert!(ease(Ease::InOut, 0.75) > 0.75);
}

fn settle(p: SpringParams, from: f32, to: f32) -> (f32, f32, bool) {
    let mut s = SpringState {
        pos: from,
        vel: 0.0,
    };
    let (mut peak, mut t) = (from, 0.0);
    let mut rest = false;
    while t < 10.0 {
        s = spring_step(s, to, p, 1.0 / 60.0);
        t += 1.0 / 60.0;
        peak = if to > from {
            peak.max(s.pos)
        } else {
            peak.min(s.pos)
        };
        if s.at_rest(to) {
            rest = true;
            break;
        }
    }
    (t, peak, rest)
}

#[test]
fn every_spring_settles_on_the_target() {
    for sp in [Spring::Gentle, Spring::Snappy, Spring::Bouncy] {
        let p = spring_params(sp).unwrap();
        let (t, peak, rest) = settle(p, 0.0, 100.0);
        assert!(rest, "{sp:?} did not settle in 10 s");
        assert!(t < 3.0, "{sp:?} took {t:.2} s");
        match sp {
            Spring::Bouncy => assert!(peak > 105.0, "bouncy overshoots: {peak}"),
            Spring::Gentle | Spring::Snappy => {
                assert!(peak < 103.0, "{sp:?} is close to critically damped: {peak}")
            }
            Spring::None => unreachable!(),
        }
    }
    assert!(spring_params(Spring::None).is_none());
}

#[test]
fn springs_are_stable_for_big_steps() {
    let p = spring_params(Spring::Bouncy).unwrap();
    let mut s = SpringState { pos: 0.0, vel: 0.0 };
    for _ in 0..100 {
        s = spring_step(s, 50.0, p, 0.25);
        assert!(s.pos.is_finite() && s.pos.abs() < 500.0);
    }
    assert!(s.at_rest(50.0));
    assert_eq!(spring_step(s, 50.0, p, 0.0), s, "no time, no change");
}

fn changed(f: impl Fn(&mut Props)) -> (Props, Props) {
    let a = Props::default();
    let mut b = a.clone();
    f(&mut b);
    (a, b)
}

#[test]
fn only_listed_and_changed_properties_animate() {
    let (a, b) = changed(|p| {
        p.paint.opacity = 0.0;
        p.paint.radius = [8.0; 4];
        p.motion.transition = Transition {
            props: PropId::Opacity.bit(),
            ms: 150,
            ease: Ease::Out,
        };
    });
    let (mask, motion) = plan(&a, &b, false);
    assert_eq!(
        mask,
        PropId::Opacity.bit(),
        "the radius changed but is not listed"
    );
    assert_eq!(
        motion,
        Motion::Curve {
            ms: 150,
            ease: Ease::Out
        }
    );
    let (mask, _) = plan(&a, &a, false);
    assert_eq!(mask, 0, "nothing changed");
    let (mask, _) = plan(&a, &b, true);
    assert_eq!(mask, 0, "reduced motion: no animation");
}

#[test]
fn a_spring_replaces_the_curve() {
    let (a, b) = changed(|p| {
        p.paint.transform.sx = 1.1;
        p.motion.transition = Transition {
            props: PropId::Transform.bit(),
            ms: 250,
            ease: Ease::Standard,
        };
        p.motion.spring = Spring::Snappy;
    });
    let (mask, motion) = plan(&a, &b, false);
    assert_eq!(mask, PropId::Transform.bit());
    assert!(matches!(motion, Motion::Spring(p) if p.stiffness == 320.0));
}

#[test]
fn blend_mixes_numbers_and_switches_colours_half_way() {
    let (a, b) = changed(|p| {
        p.paint.opacity = 0.0;
        p.paint.transform = Transform {
            tx: 10.0,
            ty: 0.0,
            sx: 2.0,
            sy: 2.0,
            rotate: 90.0,
        };
        p.paint.radius = [10.0; 4];
        p.paint.outline_color = ColorRef::Token(Token::Danger);
    });
    let mask = PropId::Opacity.bit()
        | PropId::Transform.bit()
        | PropId::RadiusTl.bit()
        | PropId::OutlineColor.bit();
    let half = blend(&a, &b, 0.5, mask);
    assert_eq!(half.paint.opacity, 0.5);
    assert_eq!(
        half.paint.transform,
        Transform {
            tx: 5.0,
            ty: 0.0,
            sx: 1.5,
            sy: 1.5,
            rotate: 45.0
        }
    );
    assert_eq!(half.paint.radius[0], 5.0);
    assert_eq!(
        half.paint.radius[1], 10.0,
        "a corner that is not listed jumps to its new value"
    );
    assert_eq!(half.paint.outline_color, ColorRef::Token(Token::Danger));
    let early = blend(&a, &b, 0.25, mask);
    assert_eq!(early.paint.outline_color, a.paint.outline_color);
    assert_eq!(blend(&a, &b, 1.0, mask).paint.opacity, 0.0);
    assert_eq!(blend(&a, &b, 0.0, mask).paint.opacity, 1.0);
}
