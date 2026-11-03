// SPDX-License-Identifier: BSD-2-Clause
//! Frame pacing (plan section 4.2): draw at most once per display frame, and not at all when
//! nothing changed. A change while no frame callback is outstanding draws at once (an idle window
//! answers the first keystroke without waiting a frame); a change while one is outstanding waits
//! for it. When the callback fires and nothing is dirty, no new one is asked for, so an idle
//! window gets no wakeups.
//!
//! Presentation feedback teaches the refresh interval and the last present time; a present that
//! comes more than half an interval late while frames were being drawn back to back counts the
//! frames it missed. The ring has the shape of the compositor's frame stats (begin, end, damage).

/// Frames kept in the ring.
pub const RING: usize = 240;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameRecord {
    pub begin_ns: u64,
    pub end_ns: u64,
    pub damage_px: u64,
    /// When it reached the screen (0 until presentation feedback says so).
    pub presented_ns: u64,
}

#[derive(Clone, Debug)]
pub struct FramePacer {
    /// A frame callback is outstanding.
    waiting: bool,
    /// Something changed since the last draw.
    dirty: bool,
    /// Keep drawing every frame (an animation runs, or the app asked for frame ticks).
    continuous: bool,
    /// Refresh interval, from presentation feedback (60 Hz until it says otherwise).
    pub refresh_ns: u64,
    pub last_present_ns: u64,
    pub presented: u64,
    pub discarded: u64,
    pub missed: u64,
    pub drawn: u64,
    /// The last draw followed the previous one by one frame (back-to-back drawing).
    back_to_back: bool,
    ring: Vec<FrameRecord>,
    at: usize,
}

impl Default for FramePacer {
    fn default() -> Self {
        FramePacer {
            waiting: false,
            dirty: false,
            continuous: false,
            refresh_ns: 16_666_667,
            last_present_ns: 0,
            presented: 0,
            discarded: 0,
            missed: 0,
            drawn: 0,
            back_to_back: false,
            ring: Vec::with_capacity(RING),
            at: 0,
        }
    }
}

impl FramePacer {
    /// Something needs drawing. True when the caller should draw now.
    pub fn request(&mut self) -> bool {
        self.dirty = true;
        !self.waiting
    }

    /// Keep frames coming while `on` (animations, frame ticks), or stop.
    pub fn set_continuous(&mut self, on: bool) {
        self.continuous = on;
    }

    pub fn is_waiting(&self) -> bool {
        self.waiting
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The frame callback fired. True when the caller should draw now.
    pub fn on_done(&mut self) -> bool {
        self.waiting = false;
        let draw = self.dirty || self.continuous;
        self.back_to_back = draw;
        draw
    }

    /// A frame was drawn and committed with a new frame callback.
    pub fn drew(&mut self, rec: FrameRecord) {
        self.waiting = true;
        self.dirty = false;
        self.drawn += 1;
        if self.ring.len() < RING {
            self.ring.push(rec);
        } else {
            self.ring[self.at] = rec;
        }
        self.at = (self.at + 1) % RING;
    }

    /// The compositor dropped the frame callback's surface state (hidden, or the window closed):
    /// stop waiting so the next change draws.
    pub fn reset(&mut self) {
        self.waiting = false;
    }

    /// Presentation feedback: the last frame reached the screen at `ns` (the compositor's
    /// presentation clock), `refresh_ns` apart (0 when unknown).
    pub fn presented(&mut self, ns: u64, refresh_ns: u32) {
        if refresh_ns > 0 {
            self.refresh_ns = u64::from(refresh_ns);
        }
        if self.last_present_ns > 0 && self.back_to_back && ns > self.last_present_ns {
            let gap = ns - self.last_present_ns;
            // More than one and a half intervals: the frames in between were missed.
            if gap * 2 > self.refresh_ns * 3 {
                self.missed += (gap + self.refresh_ns / 2) / self.refresh_ns - 1;
            }
        }
        self.last_present_ns = ns;
        self.presented += 1;
        let last = if self.at == 0 {
            self.ring.len().saturating_sub(1)
        } else {
            self.at - 1
        };
        if let Some(r) = self.ring.get_mut(last) {
            r.presented_ns = ns;
        }
    }

    pub fn discarded(&mut self) {
        self.discarded += 1;
    }

    /// When the next frame should reach the screen, predicted from the last present.
    pub fn next_vblank(&self, now_ns: u64) -> u64 {
        if self.last_present_ns == 0 || self.refresh_ns == 0 || now_ns < self.last_present_ns {
            return now_ns;
        }
        let n = (now_ns - self.last_present_ns) / self.refresh_ns + 1;
        self.last_present_ns + n * self.refresh_ns
    }

    /// The recorded frames, oldest first.
    pub fn frames(&self) -> Vec<FrameRecord> {
        let mut v = Vec::with_capacity(self.ring.len());
        if self.ring.len() == RING {
            v.extend_from_slice(&self.ring[self.at..]);
            v.extend_from_slice(&self.ring[..self.at]);
        } else {
            v.extend_from_slice(&self.ring);
        }
        v
    }
}
