// SPDX-License-Identifier: BSD-2-Clause
//! Virtual list geometry: where row `i` starts, which rows are visible, with a measured height for
//! rows that have been laid out and an estimate for the rest. A Fenwick tree of the differences
//! keeps `offset` and `row_at` at O(log n) for 100k rows, and fixed height lists stay O(1).
use crate::scene::ids::{Ix, to_u32};
use crate::style::num::f32_to_u32;
use std::ops::Range;

#[derive(Clone, Debug)]
pub struct VlistModel {
    count: u32,
    estimate: f32,
    /// Fenwick tree over (measured - estimate) of each row; empty while every row is an estimate.
    tree: Vec<f64>,
    measured: Vec<f32>,
}

/// Most rows a list may have (plan: 100k rows scroll scenario, with headroom).
pub const MAX_ROWS: u32 = 4_000_000;

impl VlistModel {
    pub fn new(count: u32, estimate: f32) -> Self {
        VlistModel {
            count: count.min(MAX_ROWS),
            estimate: estimate.max(1.0),
            tree: Vec::new(),
            measured: Vec::new(),
        }
    }

    pub fn count(&self) -> u32 {
        self.count
    }
    pub fn estimate(&self) -> f32 {
        self.estimate
    }

    /// New count or estimate; measured heights are kept for rows still in range.
    pub fn reset(&mut self, count: u32, estimate: f32) {
        let est = estimate.max(1.0);
        let count = count.min(MAX_ROWS);
        if est != self.estimate {
            self.tree.clear();
            self.measured.clear();
            self.estimate = est;
        }
        if count < self.count {
            // Drop measurements past the end: rebuild if any existed there.
            if self.measured.iter().skip(count.ix()).any(|m| *m != 0.0) {
                let keep: Vec<(u32, f32)> = self
                    .measured
                    .iter()
                    .enumerate()
                    .take(count.ix())
                    .filter(|(_, m)| **m != 0.0)
                    .map(|(i, m)| (to_u32(i), *m))
                    .collect();
                self.tree.clear();
                self.measured.clear();
                self.count = count;
                for (i, m) in keep {
                    self.set_measured(i, m);
                }
            }
        }
        self.count = count;
    }

    fn ensure(&mut self) {
        if self.tree.is_empty() && self.count > 0 {
            self.tree = vec![0.0; self.count.ix() + 1];
            self.measured = vec![0.0; self.count.ix()];
        }
        if self.tree.len() < self.count.ix() + 1 {
            // The count grew: rebuild from the measurements.
            let old = std::mem::take(&mut self.measured);
            self.tree = vec![0.0; self.count.ix() + 1];
            self.measured = vec![0.0; self.count.ix()];
            for (i, m) in old.into_iter().enumerate() {
                if m != 0.0 {
                    self.set_measured(to_u32(i), m);
                }
            }
        }
    }

    fn add(&mut self, i: u32, d: f64) {
        let mut k = i.ix() + 1;
        while k < self.tree.len() {
            self.tree[k] += d;
            k += k & (!k).wrapping_add(1);
        }
    }

    /// Prefix sum of the differences for rows `[0, i)`.
    fn prefix(&self, i: u32) -> f64 {
        let mut k = (i.ix()).min(self.tree.len().saturating_sub(1));
        let mut s = 0.0;
        while k > 0 {
            s += self.tree[k];
            k -= k & (!k).wrapping_add(1);
        }
        s
    }

    pub fn height_of(&self, i: u32) -> f32 {
        match self.measured.get(i.ix()) {
            Some(m) if *m != 0.0 => *m,
            _ => self.estimate,
        }
    }

    /// Record the laid-out height of row `i` (ignored outside the list).
    pub fn set_measured(&mut self, i: u32, h: f32) {
        if i >= self.count || !h.is_finite() || h <= 0.0 {
            return;
        }
        self.ensure();
        let old = self.height_of(i);
        if old == h && self.measured[i.ix()] != 0.0 {
            return;
        }
        self.add(i, f64::from(h) - f64::from(old));
        self.measured[i.ix()] = h;
    }

    /// Top edge of row `i` (`i == count` gives the total height).
    pub fn offset(&self, i: u32) -> f32 {
        let i = i.min(self.count);
        (f64::from(i) * f64::from(self.estimate) + self.prefix(i)) as f32
    }

    pub fn total(&self) -> f32 {
        self.offset(self.count)
    }

    /// The row containing y (clamped to the list).
    pub fn row_at(&self, y: f32) -> u32 {
        if self.count == 0 {
            return 0;
        }
        if self.tree.is_empty() {
            return f32_to_u32((y.max(0.0) / self.estimate).floor()).min(self.count - 1);
        }
        let (mut lo, mut hi) = (0u32, self.count - 1);
        while lo < hi {
            let mid = lo + (hi - lo).div_ceil(2);
            if self.offset(mid) <= y {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo
    }

    /// Rows to keep alive for a viewport at scroll offset `scroll` and height `viewport`:
    /// the visible ones plus `overscan` on each side.
    pub fn visible(&self, scroll: f32, viewport: f32, overscan: u32) -> Range<u32> {
        if self.count == 0 || viewport <= 0.0 {
            return 0..0;
        }
        let first = self.row_at(scroll.max(0.0));
        let last = self.row_at((scroll + viewport - 0.001).max(0.0));
        first.saturating_sub(overscan)..(last + 1 + overscan).min(self.count)
    }

    /// Scroll offset that puts row `i` at the top (clamped so the list does not scroll past the end).
    pub fn scroll_to(&self, i: u32, viewport: f32) -> f32 {
        self.offset(i).min((self.total() - viewport).max(0.0))
    }
}

/// Clamp a scroll offset to the content.
pub fn clamp_scroll(offset: f32, content: f32, viewport: f32) -> f32 {
    offset.clamp(0.0, (content - viewport).max(0.0))
}

/// Kinetic scrolling: a velocity (px/s) that decays by `friction` per second. Advance by `dt`
/// seconds: returns the distance moved and the new velocity (zero below 1 px/s).
pub fn fling(velocity: f32, dt: f32, friction: f32) -> (f32, f32) {
    if velocity.abs() < 1.0 || dt <= 0.0 {
        return (0.0, 0.0);
    }
    let k = (-friction).exp();
    let decay = k.powf(dt);
    let dist = if friction > 0.0 {
        velocity * (1.0 - decay) / friction
    } else {
        velocity * dt
    };
    let v = velocity * decay;
    (dist, if v.abs() < 1.0 { 0.0 } else { v })
}
