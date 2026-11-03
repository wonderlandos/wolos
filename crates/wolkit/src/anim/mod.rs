// SPDX-License-Identifier: BSD-2-Clause
//! The animation clock (plan section 2.2): springs and tweens run in Rust on the frame clock, not
//! in the app. An `ANIM` op names a node, a number property, a target and how to get there; the
//! clock reads the property's current value, moves it a little every frame with the arithmetic of
//! `style::transition` (easing curves, the spring integrator) and writes it back through
//! [`SceneWrite::set_prop`], which marks the node for the next frame. Under reduced motion, and
//! on a backend with no frame clock (the Kit host), the value snaps to the target.
//!
//! A new `ANIM` for a property that is already moving replaces the old one and starts from where
//! it is now, so a spring keeps its velocity. At most [`MAX_ACTIVE`] run at once.

use crate::scene::ids::Ix;
use crate::scene::patch::ANIM_SPRING;
use crate::scene::{AnimReq, Idx, NodeId, PropValue, SceneWrite};
use crate::style::tokens::{SpringParams, spring_params};
use crate::style::transition::{SpringState, ease, spring_step};
use crate::style::value::{Ease, Spring};

/// Animations running at once in one window (the scene queues at most this many requests).
pub const MAX_ACTIVE: usize = 4096;
/// The motion durations of the theme contract (`--motion-fast`, `-normal`, `-slow`), in ns.
pub const DURATIONS_NS: [u64; 3] = [150_000_000, 250_000_000, 400_000_000];
/// The longest step handed to the spring integrator at once.
const STEP_NS: u64 = 8_000_000;

#[derive(Clone, Copy, Debug)]
enum Kind {
    Tween {
        start_ns: u64,
        duration_ns: u64,
    },
    Spring {
        state: SpringState,
        params: SpringParams,
        last_ns: u64,
    },
}

#[derive(Clone, Copy, Debug)]
struct Active {
    node: NodeId,
    prop: u16,
    from: f32,
    to: f32,
    kind: Kind,
}

/// One finished animation, for `ANIMATION_DONE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Done {
    pub node: NodeId,
    pub prop: u16,
}

#[derive(Default)]
pub struct Clock {
    active: Vec<Active>,
}

fn current(scene: &dyn SceneWrite, i: Idx, prop: u16) -> f32 {
    scene
        .prop(i, prop)
        .map_or_else(|| default_of(prop), |v| v.f32())
}

/// What an unset number property is, for the ones that animate.
fn default_of(prop: u16) -> f32 {
    use crate::scene::prop::id;
    match prop {
        id::OPACITY | id::SHRINK => 1.0,
        _ => 0.0,
    }
}

impl Clock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Animations running.
    pub fn len(&self) -> usize {
        self.active.len()
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// Start the requests the scene queued. `animate` is false under reduced motion and on a
    /// backend with no frame clock: the value then jumps to its target.
    pub fn start(
        &mut self,
        scene: &mut dyn SceneWrite,
        reqs: &mut Vec<AnimReq>,
        now_ns: u64,
        animate: bool,
        done: &mut Vec<Done>,
    ) {
        for r in reqs.drain(..) {
            let i = r.node.slot();
            if !scene.is_live(i) || scene.id(i) != r.node {
                continue;
            }
            let from = current(scene, i, r.prop);
            let spring = (r.mode == ANIM_SPRING).then_some(match r.param {
                1 => Spring::Snappy,
                2 => Spring::Bouncy,
                _ => Spring::Gentle,
            });
            let ms_ns = DURATIONS_NS.get(r.param.ix()).copied().unwrap_or(0);
            // A running animation of the same property hands over its position and velocity.
            let at = self
                .active
                .iter()
                .position(|a| a.node == r.node && a.prop == r.prop);
            let (from, vel) = match at {
                Some(k) => {
                    let a = self.active.swap_remove(k);
                    (
                        from,
                        match a.kind {
                            Kind::Spring { state, .. } => state.vel,
                            Kind::Tween { .. } => 0.0,
                        },
                    )
                }
                None => (from, 0.0),
            };
            if !animate || (spring.is_none() && ms_ns == 0) || (from == r.target && vel == 0.0) {
                scene.set_prop(i, r.prop, &PropValue::new(&[r.target.to_bits()]));
                done.push(Done {
                    node: r.node,
                    prop: r.prop,
                });
                continue;
            }
            if self.active.len() >= MAX_ACTIVE {
                scene.set_prop(i, r.prop, &PropValue::new(&[r.target.to_bits()]));
                continue;
            }
            let kind = match spring.and_then(spring_params) {
                Some(params) => Kind::Spring {
                    state: SpringState { pos: from, vel },
                    params,
                    last_ns: now_ns,
                },
                None => Kind::Tween {
                    start_ns: now_ns,
                    duration_ns: ms_ns,
                },
            };
            self.active.push(Active {
                node: r.node,
                prop: r.prop,
                from,
                to: r.target,
                kind,
            });
        }
    }

    /// Move every animation to `now_ns`; finished ones are removed and reported in `done`.
    /// Allocates nothing while the list does not grow.
    pub fn step(&mut self, scene: &mut dyn SceneWrite, now_ns: u64, done: &mut Vec<Done>) {
        let mut k = 0;
        while k < self.active.len() {
            let a = &mut self.active[k];
            let i = a.node.slot();
            if !scene.is_live(i) || scene.id(i) != a.node {
                self.active.swap_remove(k);
                continue;
            }
            let (value, finished) = match &mut a.kind {
                Kind::Tween {
                    start_ns,
                    duration_ns,
                } => {
                    let t = now_ns.saturating_sub(*start_ns) as f32 / *duration_ns as f32;
                    if t >= 1.0 {
                        (a.to, true)
                    } else {
                        (a.from + (a.to - a.from) * ease(Ease::Standard, t), false)
                    }
                }
                Kind::Spring {
                    state,
                    params,
                    last_ns,
                } => {
                    let mut left = now_ns.saturating_sub(*last_ns);
                    while left > 0 {
                        let h = left.min(STEP_NS);
                        *state = spring_step(*state, a.to, *params, h as f32 / 1e9);
                        left -= h;
                    }
                    *last_ns = now_ns;
                    if state.pos == a.to && state.vel == 0.0 {
                        (a.to, true)
                    } else {
                        (state.pos, false)
                    }
                }
            };
            scene.set_prop(i, a.prop, &PropValue::new(&[value.to_bits()]));
            if finished {
                done.push(Done {
                    node: a.node,
                    prop: a.prop,
                });
                self.active.swap_remove(k);
            } else {
                k += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::patch::{ANIM_TWEEN, APPEND, Patch, Scratch, Writer};
    use crate::scene::prop::id;
    use crate::scene::{Journal, Kind as NodeKind, Scene, SceneRead};

    fn scene() -> Scene {
        let mut w = Writer::new();
        w.create(1, NodeKind::Box)
            .insert(0, 1, APPEND)
            .set_f32(1, id::OPACITY, 0.0);
        let mut s = Scene::new();
        s.apply(
            &Patch::parse(w.finish(1), &mut Scratch::default()).unwrap(),
            &mut Journal::default(),
        )
        .unwrap();
        s
    }

    fn run(mode: u32, param: u32, animate: bool) -> (Vec<f32>, usize) {
        let mut s = scene();
        let mut w = Writer::new();
        w.anim(1, id::OPACITY, 1.0, mode, param);
        s.apply(
            &Patch::parse(w.finish(2), &mut Scratch::default()).unwrap(),
            &mut Journal::default(),
        )
        .unwrap();
        let mut reqs = Vec::new();
        s.take_anims(&mut reqs);
        let (mut clock, mut done, mut seen) = (Clock::new(), Vec::new(), Vec::new());
        clock.start(&mut s, &mut reqs, 0, animate, &mut done);
        for frame in 1..=300u64 {
            clock.step(&mut s, frame * 16_000_000, &mut done);
            seen.push(s.prop(1, id::OPACITY).unwrap().f32());
        }
        (seen, done.len())
    }

    #[test]
    fn a_tween_reaches_its_target_in_its_duration_and_reports_once() {
        let (seen, done) = run(ANIM_TWEEN, 0, true);
        assert!(seen[0] > 0.0 && seen[0] < 1.0, "{seen:?}");
        assert!(seen.windows(2).all(|w| w[1] >= w[0]), "monotonic");
        assert_eq!(seen[9], 1.0, "150 ms is done by frame 10: {seen:?}");
        assert_eq!(done, 1);
    }

    #[test]
    fn a_spring_settles_on_its_target() {
        let (seen, done) = run(ANIM_SPRING, 0, true);
        assert_eq!(*seen.last().unwrap(), 1.0, "{seen:?}");
        assert_eq!(done, 1);
        assert!(seen.iter().any(|v| *v > 0.0 && *v < 1.0));
    }

    #[test]
    fn reduced_motion_snaps() {
        let (seen, done) = run(ANIM_TWEEN, 2, false);
        assert_eq!(seen[0], 1.0);
        assert_eq!(done, 1);
    }
}
