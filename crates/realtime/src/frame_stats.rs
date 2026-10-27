//! Per-frame compositor timing ring (ADR 0007, memory-bandwidth #1/#5). wol-hypr records one
//! `Frame` per rendered monitor frame; `wol-realtime hypr stats` reads `Stats` through hyprctl.
//! Bytes are derived, not counted: damage_px x 4 B x 2 (read + write) x passes.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub begin_ns: u64,
    pub end_ns: u64,
    pub damage_px: u64,
    pub screen_px: u64,
    pub passes: u32,
}

impl Frame {
    pub fn bytes(&self) -> u64 {
        self.damage_px.min(self.screen_px) * 8 * u64::from(self.passes.max(1))
    }
}

pub const CAP: usize = 512;

pub struct Ring {
    buf: [Frame; CAP],
    next: usize,
    len: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub frames: usize,
    pub p50_us: u64,
    pub p95_us: u64,
    /// Mean damaged share of the screen, percent.
    pub damage_pct: f64,
    /// Frames that redrew the whole screen.
    pub full_frames: usize,
    pub bytes_per_frame: u64,
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

impl Ring {
    pub const fn new() -> Self {
        Self {
            buf: [Frame {
                begin_ns: 0,
                end_ns: 0,
                damage_px: 0,
                screen_px: 0,
                passes: 0,
            }; CAP],
            next: 0,
            len: 0,
        }
    }

    /// O(1), no allocation (called on the render thread).
    pub fn push(&mut self, f: Frame) {
        self.buf[self.next] = f;
        self.next = (self.next + 1) % CAP;
        self.len = (self.len + 1).min(CAP);
    }

    pub fn stats(&self) -> Stats {
        let frames = &self.buf[..self.len];
        if frames.is_empty() {
            return Stats::default();
        }
        let mut us: Vec<u64> = frames
            .iter()
            .map(|f| f.end_ns.saturating_sub(f.begin_ns) / 1000)
            .collect();
        us.sort_unstable();
        let pct = |p: usize| us[((us.len() - 1) * p).div_ceil(100)];
        let n = frames.len();
        Stats {
            frames: n,
            p50_us: pct(50),
            p95_us: pct(95),
            damage_pct: frames
                .iter()
                .map(|f| f.damage_px.min(f.screen_px) as f64 / f.screen_px.max(1) as f64)
                .sum::<f64>()
                * 100.0
                / n as f64,
            full_frames: frames
                .iter()
                .filter(|f| f.screen_px > 0 && f.damage_px >= f.screen_px)
                .count(),
            bytes_per_frame: frames.iter().map(Frame::bytes).sum::<u64>() / n as u64,
        }
    }
}

impl Stats {
    pub fn to_json(&self) -> String {
        format!(
            r#"{{"frames":{},"p50_us":{},"p95_us":{},"damage_pct":{:.2},"full_frames":{},"bytes_per_frame":{}}}"#,
            self.frames,
            self.p50_us,
            self.p95_us,
            self.damage_pct,
            self.full_frames,
            self.bytes_per_frame
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(ms: u64, damage: u64, passes: u32) -> Frame {
        Frame {
            begin_ns: 1_000_000,
            end_ns: 1_000_000 + ms * 1_000_000,
            damage_px: damage,
            screen_px: 1920 * 1200,
            passes,
        }
    }

    #[test]
    fn empty_ring_is_zero() {
        assert_eq!(Ring::new().stats(), Stats::default());
    }

    #[test]
    fn typing_frames_vs_one_full_redraw() {
        let mut r = Ring::new();
        for _ in 0..99 {
            r.push(f(1, 23_040, 1)); // 1% of 1920x1200
        }
        r.push(f(9, 1920 * 1200, 2));
        let s = r.stats();
        assert_eq!((s.frames, s.p50_us, s.full_frames), (100, 1000, 1));
        assert_eq!(s.p95_us, 1000);
        assert!((s.damage_pct - 1.99).abs() < 0.01, "{}", s.damage_pct);
        // 99 x 23040 x 8 + 2304000 x 8 x 2, over 100 frames
        assert_eq!(s.bytes_per_frame, (99 * 23_040 * 8 + 2_304_000 * 16) / 100);
        assert!(s.to_json().contains(r#""full_frames":1"#));
    }

    #[test]
    fn ring_keeps_the_last_cap_frames() {
        let mut r = Ring::new();
        for i in 0..(CAP as u64 + 10) {
            r.push(f(if i < 10 { 50 } else { 2 }, 0, 1));
        }
        let s = r.stats();
        assert_eq!((s.frames, s.p95_us), (CAP, 2000));
    }
}
