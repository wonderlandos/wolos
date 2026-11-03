// SPDX-License-Identifier: BSD-2-Clause
//! Hit testing: the deepest node under a point. Overlays (anchored layers for popovers and menus)
//! are on top of everything, the newest first; a modal layer that is open absorbs every point
//! outside it, so a click beside a sheet reaches the sheet (its scrim), never the window behind.
//!
//! Layout rectangles are in window space before scroll offsets (W1's convention), so a point is
//! moved by the scroll offset of every scroll container on the way down. Later siblings are on
//! top, and a higher `z-index` beats tree order within a parent. A part that clips (`overflow`
//! hidden, scroll or auto) hides children outside its rectangle.

use crate::scene::prop::id;
use crate::scene::{Idx, Kind, NIL, SceneRead, walk};

/// Overlay and modal nodes in tree order, recomputed when the scene version moves.
#[derive(Default)]
pub struct Layers {
    version: Option<u64>,
    /// `(node, modal)`.
    pub list: Vec<(Idx, bool)>,
}

/// Whether the node is not drawn (display none or visibility hidden) or ignores the pointer.
fn skipped(scene: &dyn SceneRead, n: Idx, pointer: bool) -> bool {
    let is = |p: u16| scene.prop(n, p).is_some_and(|v| v.w[0] == 1);
    is(id::VISIBILITY) || is(id::DISPLAY) || (pointer && is(id::POINTER_EVENTS))
}

/// Whether `n` and every ancestor are drawn.
pub fn shown(scene: &dyn SceneRead, mut n: Idx) -> bool {
    let mut guard = 0;
    while n != NIL && guard <= crate::scene::MAX_DEPTH + 1 {
        if skipped(scene, n, false) {
            return false;
        }
        n = scene.parent(n);
        guard += 1;
    }
    true
}

pub fn is_modal(scene: &dyn SceneRead, n: Idx) -> bool {
    scene.prop(n, id::MODAL).is_some_and(|v| v.w[0] == 1)
}

impl Layers {
    /// Bring the list up to date with the scene.
    pub fn refresh(&mut self, scene: &dyn SceneRead) {
        if self.version == Some(scene.version()) {
            return;
        }
        self.version = Some(scene.version());
        self.list.clear();
        let list = &mut self.list;
        walk(scene, 0, &mut |i, _| {
            let modal = is_modal(scene, i);
            if (modal || scene.kind(i) == Kind::Overlay) && list.len() < 256 {
                list.push((i, modal));
            }
        });
        list.retain(|(i, _)| shown(scene, *i));
    }

    /// The innermost open modal layer (the last in tree order), or NIL.
    pub fn top_modal(&self) -> Idx {
        self.list
            .iter()
            .rev()
            .find(|(_, m)| *m)
            .map_or(NIL, |(i, _)| *i)
    }

    /// Forget the cache (the next `refresh` walks again).
    pub fn invalidate(&mut self) {
        self.version = None;
    }
}

/// The sum of the scroll offsets of the containers above `n` (not `n` itself). A window point
/// plus this sum is the point in `n`'s layout coordinates; `n`'s rectangle minus it is where it
/// is drawn.
pub fn scroll_above(scene: &dyn SceneRead, n: Idx) -> (f32, f32) {
    let (mut x, mut y) = (0.0, 0.0);
    let mut p = scene.parent(n);
    let mut guard = 0;
    while p != NIL && guard <= crate::scene::MAX_DEPTH + 1 {
        if let Some(v) = scene.prop(p, id::SCROLL_OFFSET) {
            x += f32::from_bits(v.w[0]);
            y += f32::from_bits(v.w[1]);
        }
        p = scene.parent(p);
        guard += 1;
    }
    (x, y)
}

/// Where `n` is drawn, in window coordinates (its layout rectangle moved by the scroll above).
pub fn drawn_rect(scene: &dyn SceneRead, n: Idx) -> crate::scene::Rect {
    let mut r = scene.rect(n);
    let (sx, sy) = scroll_above(scene, n);
    r.x -= sx;
    r.y -= sy;
    r
}

fn z(scene: &dyn SceneRead, n: Idx) -> i32 {
    scene
        .prop(n, id::Z_INDEX)
        .map_or(0, |v| v.w[0].cast_signed())
}

/// The deepest node at layout point `(x, y)` inside `n`, if `n` or a descendant is there.
fn at(scene: &dyn SceneRead, n: Idx, x: f32, y: f32, depth: usize) -> Option<Idx> {
    if depth > crate::scene::MAX_DEPTH + 1 {
        return None;
    }
    let inside = scene.rect(n).contains(x, y);
    let clips = scene.prop(n, id::OVERFLOW).is_some_and(|v| v.w[0] != 0);
    if clips && !inside {
        return None;
    }
    // Children see the point moved by this node's scroll offset.
    let (cx, cy) = scene.prop(n, id::SCROLL_OFFSET).map_or((x, y), |v| {
        (x + f32::from_bits(v.w[0]), y + f32::from_bits(v.w[1]))
    });
    let hit_child = |c: Idx| -> Option<Idx> {
        if skipped(scene, c, true) || scene.kind(c) == Kind::Overlay {
            return None;
        }
        at(scene, c, cx, cy, depth + 1)
    };
    let mut c = scene.first_child(n);
    let mut layered = false;
    while c != NIL && !layered {
        layered = z(scene, c) != 0;
        c = scene.next_sibling(c);
    }
    if layered {
        // Highest z-index first; among equals the later sibling (drawn on top).
        let mut kids: Vec<(i32, usize, Idx)> = Vec::new();
        let mut c = scene.first_child(n);
        while c != NIL && kids.len() <= crate::scene::MAX_NODES {
            kids.push((z(scene, c), kids.len(), c));
            c = scene.next_sibling(c);
        }
        kids.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        if let Some(h) = kids.iter().find_map(|k| hit_child(k.2)) {
            return Some(h);
        }
    } else {
        let mut c = scene.last_child(n);
        while c != NIL {
            if let Some(h) = hit_child(c) {
                return Some(h);
            }
            c = scene.prev_sibling(c);
        }
    }
    (inside && !skipped(scene, n, true)).then_some(n)
}

/// The deepest node under window point `(x, y)`; 0 (the window) when nothing else is there.
pub fn hit_test(scene: &dyn SceneRead, layers: &Layers, x: f32, y: f32) -> Idx {
    if !scene.is_live(0) {
        return 0;
    }
    let top_modal = layers.top_modal();
    for &(l, _) in layers.list.iter().rev() {
        // A layer below the innermost modal one cannot be reached.
        if top_modal != NIL && l != top_modal && !scene_is_inside(scene, top_modal, l) {
            continue;
        }
        let (sx, sy) = scroll_above(scene, l);
        if let Some(h) = at(scene, l, x + sx, y + sy, 0) {
            return h;
        }
    }
    if top_modal != NIL {
        return top_modal;
    }
    at(scene, 0, x, y, 0).unwrap_or(0)
}

/// Whether `node` is `anc` or below it.
pub fn scene_is_inside(scene: &dyn SceneRead, anc: Idx, node: Idx) -> bool {
    let mut n = node;
    let mut guard = 0;
    while n != NIL && guard <= crate::scene::MAX_DEPTH + 1 {
        if n == anc {
            return true;
        }
        n = scene.parent(n);
        guard += 1;
    }
    false
}
