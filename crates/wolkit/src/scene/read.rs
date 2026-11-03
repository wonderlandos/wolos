// SPDX-License-Identifier: BSD-2-Clause
//! The view of the scene the pipeline stages code against (style, layout, paint, accessibility,
//! input). A stage reads the tree through [`SceneRead`] and hands what it computed back through
//! [`SceneWrite`]; it never builds nodes. Both are object safe, so a stage can take
//! `&dyn SceneRead`, and a test can implement them over a hand-made tree.
//!
//! Nodes are addressed by [`Idx`] (the slot). `NIL` means "none". A node that is not live has
//! every field at its default.

use super::ids::{Idx, NodeId};
use super::kind::Kind;
use super::patch::PropValue;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    pub fn union(&self, o: &Rect) -> Rect {
        if self.w <= 0.0 || self.h <= 0.0 {
            return *o;
        }
        if o.w <= 0.0 || o.h <= 0.0 {
            return *self;
        }
        let (x0, y0) = (self.x.min(o.x), self.y.min(o.y));
        let (x1, y1) = (
            (self.x + self.w).max(o.x + o.w),
            (self.y + self.h).max(o.y + o.h),
        );
        Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }
}

/// Accessibility data of a node; the strings borrow from the scene.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct A11y<'a> {
    pub role: u32,
    pub name: &'a str,
    pub description: &'a str,
}

pub trait SceneRead {
    /// One more than the largest slot that was ever live; every live node is below it.
    fn capacity(&self) -> Idx;
    fn is_live(&self, i: Idx) -> bool;
    /// The full id (slot and generation) of a live node, for events.
    fn id(&self, i: Idx) -> NodeId;
    fn kind(&self, i: Idx) -> Kind;
    fn parent(&self, i: Idx) -> Idx;
    fn first_child(&self, i: Idx) -> Idx;
    fn last_child(&self, i: Idx) -> Idx;
    fn next_sibling(&self, i: Idx) -> Idx;
    fn prev_sibling(&self, i: Idx) -> Idx;
    fn child_count(&self, i: Idx) -> u32;
    /// Engine-owned and app-owned state bits (`kind::state`).
    fn state(&self, i: Idx) -> u32;
    /// Class ids from `SET_STYLE`, in order (later wins).
    fn classes(&self, i: Idx) -> &[u32];
    fn text(&self, i: Idx) -> &str;
    fn key(&self, i: Idx) -> u64;
    /// Events the app asked for (`BIND_EVENT`), as a mask of `event::bit`.
    fn event_mask(&self, i: Idx) -> u32;
    fn a11y(&self, i: Idx) -> A11y<'_>;
    /// An inline property, if the app set one.
    fn prop(&self, i: Idx, prop: u16) -> Option<PropValue>;
    /// Every inline property of the node, in the order they were first set.
    fn each_prop(&self, i: Idx, f: &mut dyn FnMut(u16, &PropValue));
    /// Dirty bits (`kind::dirty`) of the node.
    fn dirty(&self, i: Idx) -> u8;
    /// The computed style id the style stage stored.
    fn computed(&self, i: Idx) -> u32;
    /// The layout rectangle in window space, before scroll offsets, as the layout stage stored it.
    fn rect(&self, i: Idx) -> Rect;
    /// The focused node, or NIL.
    fn focus(&self) -> Idx;
    /// Counts bumped by every change, so a cache can tell the tree or the content moved.
    fn version(&self) -> u64;
    /// Live nodes, the root included.
    fn node_count(&self) -> usize;
    /// The text behind a string id (a `Str` property holds one).
    fn string(&self, id: u32) -> &str;
}

/// What a stage may change.
pub trait SceneWrite: SceneRead {
    fn set_computed(&mut self, i: Idx, style: u32);
    fn set_rect(&mut self, i: Idx, r: Rect);
    /// Clear `bits` of the node's own dirty flags (and `SUBTREE` is recomputed by the caller's
    /// walk through [`SceneWrite::clear_subtree`]).
    fn clear_dirty(&mut self, i: Idx, bits: u8);
    fn clear_subtree(&mut self, i: Idx);
    fn mark_dirty(&mut self, i: Idx, bits: u8);
    /// Set or clear engine-owned state bits (hover, active, focus).
    fn set_engine_state(&mut self, i: Idx, mask: u32, on: bool);
    /// Store a scroll offset the engine moved (kinetic scrolling), without a patch.
    fn set_prop(&mut self, i: Idx, prop: u16, value: &PropValue);
}

/// Depth-first, parents before children, over the live tree from the root. Cheap enough for a
/// stage's main loop: no allocation, the links are the stack.
pub fn walk(scene: &dyn SceneRead, root: Idx, f: &mut dyn FnMut(Idx, usize)) {
    if !scene.is_live(root) {
        return;
    }
    let (mut cur, mut depth) = (root, 0usize);
    loop {
        f(cur, depth);
        let c = scene.first_child(cur);
        if c != super::ids::NIL {
            cur = c;
            depth += 1;
            continue;
        }
        loop {
            if cur == root {
                return;
            }
            let n = scene.next_sibling(cur);
            if n != super::ids::NIL {
                cur = n;
                break;
            }
            cur = scene.parent(cur);
            depth -= 1;
        }
    }
}
