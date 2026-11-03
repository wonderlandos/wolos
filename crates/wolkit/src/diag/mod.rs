// SPDX-License-Identifier: BSD-2-Clause
//! Counters and the frame ring behind `wolkit_engine_stats_json`. The ring has the same shape as
//! the compositor's (`crates/realtime/src/frame_stats.rs`): one entry per frame with begin and end
//! times, damage and pass count, so the bench harness reads both the same way.

use std::time::Instant;

/// Frames kept.
pub const RING: usize = 256;

#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    pub begin_ns: u64,
    pub end_ns: u64,
    pub damage_px: u64,
    pub passes: u32,
    pub draws: u32,
}

pub struct FrameRing {
    frames: Box<[Frame; RING]>,
    next: usize,
    total: u64,
    epoch: Instant,
}

impl Default for FrameRing {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameRing {
    pub fn new() -> Self {
        Self { frames: Box::new([Frame::default(); RING]), next: 0, total: 0, epoch: Instant::now() }
    }

    pub fn now_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    pub fn record(&mut self, f: Frame) {
        self.frames[self.next] = f;
        self.next = (self.next + 1) % RING;
        self.total += 1;
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    /// (p50, p95, max) of frame durations in microseconds over the ring.
    pub fn percentiles(&self) -> (u64, u64, u64) {
        let n = (self.total as usize).min(RING);
        if n == 0 {
            return (0, 0, 0);
        }
        let mut d: Vec<u64> = self.frames[..n].iter().map(|f| f.end_ns.saturating_sub(f.begin_ns) / 1000).collect();
        d.sort_unstable();
        (d[n / 2], d[(n * 95 / 100).min(n - 1)], d[n - 1])
    }
}

/// Engine-wide counters.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counters {
    pub patches: u64,
    pub ops: u64,
    pub patch_errors: u64,
    pub events: u64,
}
