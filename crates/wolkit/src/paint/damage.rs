// SPDX-License-Identifier: BSD-2-Clause
//! Damage tracking: what part of the window must be repainted.
//!
//! The damage of a frame is the union of the old and the new bounds of every paint-dirty node
//! (a node that moves damages where it was and where it is). The region is kept as at most
//! [`MAX_RECTS`] rectangles; when more arrive, the two whose union wastes the least area are
//! merged. Idle damage is empty. A caret blink or a hover damages only its own bounds.
//!
//! [`DamageRing`] keeps the last few frames so a swapchain that reports *buffer age* (v2 dmabuf
//! presentation) can repaint only what changed since that buffer was last shown.

use crate::paint::geom::Rect;
use crate::scene::ids::Ix;

/// Most rectangles a region keeps before merging.
pub const MAX_RECTS: usize = 8;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Damage {
    rects: Vec<Rect>,
}

impl Damage {
    pub fn new() -> Damage {
        Damage {
            rects: Vec::with_capacity(MAX_RECTS + 1),
        }
    }

    pub fn full(window: Rect) -> Damage {
        let mut d = Damage::new();
        d.add(window);
        d
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    pub fn clear(&mut self) {
        self.rects.clear();
    }

    /// Total area of the rectangles (rectangles never overlap after `add`).
    pub fn area(&self) -> f32 {
        self.rects.iter().map(Rect::area).sum()
    }

    /// The bounding box of the whole region.
    pub fn bounds(&self) -> Rect {
        self.rects.iter().fold(Rect::ZERO, |a, r| a.union(r))
    }

    /// Adds a rectangle. Overlapping or touching rectangles merge, so the region stays a set of
    /// disjoint rectangles.
    pub fn add(&mut self, r: Rect) {
        if r.is_empty() {
            return;
        }
        let mut r = r;
        loop {
            let hit = self
                .rects
                .iter()
                .position(|o| o.contains(&r) || touches(o, &r));
            match hit {
                Some(i) => {
                    let o = self.rects.swap_remove(i);
                    if o.contains(&r) {
                        r = o;
                        // r absorbed; it may now touch others only if o did, which was disjoint.
                        self.rects.push(r);
                        return;
                    }
                    r = r.union(&o);
                }
                None => break,
            }
        }
        self.rects.push(r);
        while self.rects.len() > MAX_RECTS {
            self.merge_cheapest();
        }
    }

    fn merge_cheapest(&mut self) {
        let n = self.rects.len();
        let mut best = (0usize, 1usize, f32::INFINITY);
        for i in 0..n {
            for j in i + 1..n {
                let u = self.rects[i].union(&self.rects[j]);
                let waste = u.area() - self.rects[i].area() - self.rects[j].area();
                if waste < best.2 {
                    best = (i, j, waste);
                }
            }
        }
        let b = self.rects.swap_remove(best.1);
        let a = self.rects[best.0];
        self.rects[best.0] = a.union(&b);
        // The merged rectangle may now overlap others; re-normalise.
        let merged = self.rects.swap_remove(best.0);
        self.add(merged);
    }

    pub fn union_with(&mut self, o: &Damage) {
        for r in &o.rects {
            self.add(*r);
        }
    }

    /// Restricts the region to `window`.
    pub fn clip(&mut self, window: Rect) {
        // Intersecting disjoint rectangles keeps them disjoint, so no re-merge is needed.
        self.rects.retain_mut(|r| {
            *r = r.intersect(&window);
            !r.is_empty()
        });
    }
}

fn touches(a: &Rect, b: &Rect) -> bool {
    a.overlaps(b)
}

/// Tracks the painted bounds of nodes between frames.
#[derive(Debug, Default)]
pub struct DamageTracker {
    bounds: Vec<Rect>,
    region: Damage,
}

impl DamageTracker {
    pub fn new() -> DamageTracker {
        DamageTracker {
            bounds: Vec::new(),
            region: Damage::new(),
        }
    }

    /// A node (by slot) is now painted at `new`. Damages the union of its old and new bounds, or
    /// nothing at all when it did not move or change size and `repaint` is false.
    pub fn update(&mut self, slot: u32, new: Rect, repaint: bool) {
        let i = slot.ix();
        if i >= self.bounds.len() {
            self.bounds.resize(i + 1, Rect::ZERO);
        }
        let old = self.bounds[i];
        self.bounds[i] = new;
        if repaint || old != new {
            self.region.add(old.union(&new));
        }
    }

    /// A node left the tree; its old bounds are damaged.
    pub fn remove(&mut self, slot: u32) {
        if let Some(b) = self.bounds.get_mut(slot.ix()) {
            let old = std::mem::replace(b, Rect::ZERO);
            self.region.add(old);
        }
    }

    /// Damages an arbitrary rectangle (resize, scale change, overlay shown).
    pub fn add(&mut self, r: Rect) {
        self.region.add(r);
    }

    /// The painted bounds recorded for a slot.
    pub fn bounds_of(&self, slot: u32) -> Rect {
        self.bounds.get(slot.ix()).copied().unwrap_or(Rect::ZERO)
    }

    pub fn pending(&self) -> &Damage {
        &self.region
    }

    /// Takes the frame's damage, clipped to the window, and resets the tracker's region.
    pub fn take(&mut self, window: Rect, out: &mut Damage) {
        out.clear();
        out.union_with(&self.region);
        out.clip(window);
        self.region.clear();
    }
}

/// The damage of the last few frames, for buffer-age presentation.
#[derive(Debug)]
pub struct DamageRing {
    frames: Vec<Damage>,
    next: usize,
    filled: usize,
}

impl DamageRing {
    pub fn new(depth: usize) -> DamageRing {
        DamageRing {
            frames: (0..depth.max(1)).map(|_| Damage::new()).collect(),
            next: 0,
            filled: 0,
        }
    }

    /// Records the damage of the frame just drawn.
    pub fn push(&mut self, d: &Damage) {
        self.frames[self.next].clear();
        self.frames[self.next].union_with(d);
        self.next = (self.next + 1) % self.frames.len();
        self.filled = (self.filled + 1).min(self.frames.len());
    }

    /// What to repaint into a buffer of the given age (frames since it was last the target).
    /// Age 0 (undefined contents) or older than the ring remembers returns the whole window.
    /// The current frame's damage `now` is always included.
    pub fn for_age(&self, age: usize, now: &Damage, window: Rect) -> Damage {
        if age == 0 || age > self.filled + 1 {
            return Damage::full(window);
        }
        let mut out = Damage::new();
        out.union_with(now);
        let n = self.frames.len();
        for k in 1..age {
            let idx = (self.next + n - k) % n;
            out.union_with(&self.frames[idx]);
        }
        out.clip(window);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 800.0,
        h: 600.0,
    };

    #[test]
    fn idle_frame_has_no_damage() {
        let mut t = DamageTracker::new();
        t.update(1, Rect::new(10.0, 10.0, 50.0, 20.0), true);
        let mut d = Damage::new();
        t.take(WIN, &mut d);
        assert!(!d.is_empty());
        // Nothing painted again: no damage.
        t.take(WIN, &mut d);
        assert!(d.is_empty());
        // Same bounds, no repaint requested: still nothing.
        t.update(1, Rect::new(10.0, 10.0, 50.0, 20.0), false);
        t.take(WIN, &mut d);
        assert!(d.is_empty());
    }

    #[test]
    fn caret_blink_damages_only_its_bounds() {
        let mut t = DamageTracker::new();
        let caret = Rect::new(100.0, 40.0, 1.0, 16.0);
        t.update(7, caret, true);
        let mut d = Damage::new();
        t.take(WIN, &mut d);
        t.update(7, caret, true);
        t.take(WIN, &mut d);
        assert_eq!(d.rects(), &[caret]);
    }

    #[test]
    fn moved_node_damages_old_and_new() {
        let mut t = DamageTracker::new();
        t.update(3, Rect::new(0.0, 0.0, 10.0, 10.0), true);
        let mut d = Damage::new();
        t.take(WIN, &mut d);
        t.update(3, Rect::new(100.0, 0.0, 10.0, 10.0), false);
        t.take(WIN, &mut d);
        assert_eq!(d.rects().len(), 1);
        assert_eq!(d.bounds(), Rect::new(0.0, 0.0, 110.0, 10.0));
    }

    #[test]
    fn removed_node_damages_where_it_was() {
        let mut t = DamageTracker::new();
        t.update(2, Rect::new(5.0, 5.0, 20.0, 20.0), true);
        let mut d = Damage::new();
        t.take(WIN, &mut d);
        t.remove(2);
        t.take(WIN, &mut d);
        assert_eq!(d.rects(), &[Rect::new(5.0, 5.0, 20.0, 20.0)]);
    }

    #[test]
    fn region_stays_disjoint_and_bounded() {
        let mut d = Damage::new();
        for i in 0..40 {
            d.add(Rect::new(
                i as f32 * 30.0,
                (i % 5) as f32 * 100.0,
                20.0,
                20.0,
            ));
        }
        assert!(d.rects().len() <= MAX_RECTS);
        for (i, a) in d.rects().iter().enumerate() {
            for b in &d.rects()[i + 1..] {
                assert!(!a.overlaps(b), "{a:?} overlaps {b:?}");
            }
        }
        // Every added rectangle is covered.
        let mut full = Damage::new();
        full.union_with(&d);
        for i in 0..40 {
            let r = Rect::new(i as f32 * 30.0, (i % 5) as f32 * 100.0, 20.0, 20.0);
            assert!(d.rects().iter().any(|x| x.contains(&r)), "lost {r:?}");
        }
    }

    #[test]
    fn damage_is_clipped_to_the_window() {
        let mut t = DamageTracker::new();
        t.add(Rect::new(790.0, 590.0, 100.0, 100.0));
        let mut d = Damage::new();
        t.take(WIN, &mut d);
        assert_eq!(d.rects(), &[Rect::new(790.0, 590.0, 10.0, 10.0)]);
    }

    #[test]
    fn buffer_age_unions_recent_frames() {
        let mut ring = DamageRing::new(4);
        let a = Damage::full(Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = Damage::full(Rect::new(100.0, 0.0, 10.0, 10.0));
        ring.push(&a);
        ring.push(&b);
        let now = Damage::full(Rect::new(0.0, 100.0, 10.0, 10.0));
        // Age 1: the buffer shows the previous frame; only `now` is new.
        assert_eq!(ring.for_age(1, &now, WIN).rects().len(), 1);
        // Age 2: also what the previous frame changed.
        assert_eq!(ring.for_age(2, &now, WIN).rects().len(), 2);
        // Age 0 or too old: everything.
        assert_eq!(ring.for_age(0, &now, WIN).bounds(), WIN);
        assert_eq!(ring.for_age(9, &now, WIN).bounds(), WIN);
    }
}
