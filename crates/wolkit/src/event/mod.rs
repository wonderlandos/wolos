// SPDX-License-Identifier: BSD-2-Clause
//! Event record v1 (interface I12, frozen) and the bounded queue that holds records until the app
//! drains them with `wolkit_engine_read_events`. There are no callbacks: everything the engine
//! tells the app is a record here.
//!
//! ```text
//! u32 len | u16 kind | u16 flags | u32 window | u32 node | u32 seq | payload ... | zero padding
//! ```
//! `len` is the whole record, a multiple of 8. `seq` is the sequence number of the last patch the
//! engine had applied when it made the event, so an app can ignore an edit that predates a patch
//! it has since sent (controlled inputs). Little-endian. The payload starts at byte 20: read its
//! fields unaligned.

use crate::scene::ids::{Ix, to_u32};

/// Bytes of the fixed part of a record.
pub const HEADER: usize = 20;
/// Pending records (plan section 2.8).
pub const MAX_PENDING: usize = 4096;
/// Bytes the queue may hold.
pub const MAX_BYTES: usize = 4 << 20;
/// Largest record: a 64 KiB text and its fields. A read buffer this big always makes progress.
pub const MAX_RECORD: usize = 64 * 1024 + 256;

/// `flags`: events were dropped just before this record.
pub const FLAG_OVERFLOW: u16 = 1;

/// Event kinds (`WOLKIT_EVENT_*`), append-only. Payload layouts are in `docs/modules/libwolkit.md`.
pub mod kind {
    pub const CLICK: u16 = 1;
    pub const POINTER_ENTER: u16 = 2;
    pub const POINTER_LEAVE: u16 = 3;
    pub const KEY: u16 = 4;
    pub const TEXT_CHANGED: u16 = 5;
    pub const TEXT_SUBMIT: u16 = 6;
    pub const TOGGLE: u16 = 7;
    pub const SLIDER: u16 = 8;
    pub const SELECT: u16 = 9;
    pub const SCROLL: u16 = 10;
    pub const FOCUS: u16 = 11;
    pub const RESIZE: u16 = 12;
    pub const CLOSE_REQUEST: u16 = 13;
    pub const DROP: u16 = 14;
    pub const DRAG_START: u16 = 15;
    pub const THEME_CHANGED: u16 = 16;
    pub const FRAME_TICK: u16 = 17;
    pub const ANIMATION_DONE: u16 = 18;
    pub const LIST_RANGE: u16 = 19;
    pub const SURFACE_RESIZED: u16 = 20;
    pub const A11Y_ACTION: u16 = 21;
    pub const VISIBILITY: u16 = 22;
    pub const CONTEXT_MENU: u16 = 23;
    pub const SHORTCUT: u16 = 24;
    pub const DIAGNOSTIC: u16 = 25;
    pub const NAMES: &[(&str, u16)] = &[
        ("CLICK", CLICK),
        ("POINTER_ENTER", POINTER_ENTER),
        ("POINTER_LEAVE", POINTER_LEAVE),
        ("KEY", KEY),
        ("TEXT_CHANGED", TEXT_CHANGED),
        ("TEXT_SUBMIT", TEXT_SUBMIT),
        ("TOGGLE", TOGGLE),
        ("SLIDER", SLIDER),
        ("SELECT", SELECT),
        ("SCROLL", SCROLL),
        ("FOCUS", FOCUS),
        ("RESIZE", RESIZE),
        ("CLOSE_REQUEST", CLOSE_REQUEST),
        ("DROP", DROP),
        ("DRAG_START", DRAG_START),
        ("THEME_CHANGED", THEME_CHANGED),
        ("FRAME_TICK", FRAME_TICK),
        ("ANIMATION_DONE", ANIMATION_DONE),
        ("LIST_RANGE", LIST_RANGE),
        ("SURFACE_RESIZED", SURFACE_RESIZED),
        ("A11Y_ACTION", A11Y_ACTION),
        ("VISIBILITY", VISIBILITY),
        ("CONTEXT_MENU", CONTEXT_MENU),
        ("SHORTCUT", SHORTCUT),
        ("DIAGNOSTIC", DIAGNOSTIC),
    ];
}

/// The `BIND_EVENT` mask bit of an event kind: `1 << (kind - 1)`. Window events (resize, close
/// request, theme, frame tick, visibility, diagnostic) are not bound to nodes and always arrive.
pub const fn bit(kind: u16) -> u32 {
    1u32 << (kind - 1)
}

/// Kinds whose newest record replaces an older pending one of the same node, and that are dropped
/// first when the queue is full.
pub const fn coalescable(kind: u16) -> bool {
    matches!(
        kind,
        kind::SCROLL
            | kind::SLIDER
            | kind::RESIZE
            | kind::FRAME_TICK
            | kind::SURFACE_RESIZED
            | kind::LIST_RANGE
            | kind::THEME_CHANGED
            | kind::POINTER_ENTER
            | kind::POINTER_LEAVE
    )
}

const NONE: usize = usize::MAX;
const DEAD: u16 = 0;

/// The bounded queue of encoded records.
pub struct EventQueue {
    buf: Vec<u8>,
    /// First unread byte.
    start: usize,
    /// Live records.
    count: usize,
    dropped: bool,
    /// Offset of the newest record of each coalescable kind.
    newest: [usize; 32],
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::new()
    }
}

fn rd32(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}

fn rd16(b: &[u8], at: usize) -> u16 {
    b.get(at..at + 2)
        .and_then(|s| s.try_into().ok())
        .map_or(0, u16::from_le_bytes)
}

impl EventQueue {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(16 * 1024),
            start: 0,
            count: 0,
            dropped: false,
            newest: [NONE; 32],
        }
    }

    /// Live records waiting.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Whether records were dropped since the last one that carried the flag.
    pub fn overflowed(&self) -> bool {
        self.dropped
    }

    /// Queue a record: `head` then `tail` are the payload. Coalescable kinds replace the newest
    /// pending record of the same kind and node when the payload size matches.
    pub fn push(&mut self, kind: u16, window: u32, node: u32, seq: u32, head: &[u8], tail: &[u8]) {
        let payload = head.len() + tail.len();
        let total = (HEADER + payload + 7) & !7;
        if total > MAX_RECORD + 8 {
            return;
        }
        if coalescable(kind) && (kind.ix()) <= 32 {
            let at = self.newest[kind.ix() - 1];
            if at != NONE
                && at >= self.start
                && rd16(&self.buf, at + 4) == kind
                && rd32(&self.buf, at + 8) == window
                && rd32(&self.buf, at + 12) == node
                && rd32(&self.buf, at).ix() == total
            {
                self.write_record(at, total, kind, 0, window, node, seq, head, tail);
                return;
            }
        }
        while self.count >= MAX_PENDING || self.buf.len() - self.start + total > MAX_BYTES {
            if !self.drop_oldest_coalescable() {
                // Nothing can be dropped: the newest, if it is expendable, is the one lost.
                self.dropped = true;
                if coalescable(kind) {
                    return;
                }
                // A record that matters is kept past the cap by dropping the oldest of anything.
                self.drop_oldest();
            }
        }
        self.compact_if_needed();
        let at = self.buf.len();
        self.buf.resize(at + total, 0);
        let flags = if std::mem::take(&mut self.dropped) {
            FLAG_OVERFLOW
        } else {
            0
        };
        self.write_record(at, total, kind, flags, window, node, seq, head, tail);
        self.count += 1;
        if coalescable(kind) && (kind.ix()) <= 32 {
            self.newest[kind.ix() - 1] = at;
        }
    }

    #[allow(clippy::too_many_arguments)] // one record is these eight fields
    fn write_record(
        &mut self,
        at: usize,
        total: usize,
        kind: u16,
        flags: u16,
        window: u32,
        node: u32,
        seq: u32,
        head: &[u8],
        tail: &[u8],
    ) {
        let Some(r) = self.buf.get_mut(at..at + total) else {
            return;
        };
        r[0..4].copy_from_slice(&(to_u32(total)).to_le_bytes());
        r[4..6].copy_from_slice(&kind.to_le_bytes());
        if flags != 0 || r[6..8] == [0, 0] {
            r[6..8].copy_from_slice(&flags.to_le_bytes());
        }
        r[8..12].copy_from_slice(&window.to_le_bytes());
        r[12..16].copy_from_slice(&node.to_le_bytes());
        r[16..20].copy_from_slice(&seq.to_le_bytes());
        r[HEADER..HEADER + head.len()].copy_from_slice(head);
        r[HEADER + head.len()..HEADER + head.len() + tail.len()].copy_from_slice(tail);
        r[HEADER + head.len() + tail.len()..].fill(0);
    }

    fn records(&self) -> impl Iterator<Item = (usize, usize, u16)> + '_ {
        let mut at = self.start;
        std::iter::from_fn(move || {
            if at >= self.buf.len() {
                return None;
            }
            let len = rd32(&self.buf, at).ix();
            if len < HEADER || at + len > self.buf.len() {
                return None;
            }
            let r = (at, len, rd16(&self.buf, at + 4));
            at += len;
            Some(r)
        })
    }

    fn drop_oldest_coalescable(&mut self) -> bool {
        let found = self
            .records()
            .find(|(_, _, k)| *k != DEAD && coalescable(*k));
        if let Some((at, _, _)) = found {
            self.buf[at + 4..at + 6].copy_from_slice(&DEAD.to_le_bytes());
            self.count -= 1;
            self.dropped = true;
            return true;
        }
        false
    }

    fn drop_oldest(&mut self) {
        let found = self.records().find(|(_, _, k)| *k != DEAD);
        if let Some((at, _, _)) = found {
            self.buf[at + 4..at + 6].copy_from_slice(&DEAD.to_le_bytes());
            self.count -= 1;
            self.dropped = true;
        }
    }

    fn compact_if_needed(&mut self) {
        if self.start > 0
            && (self.start >= self.buf.len()
                || self.start > 64 * 1024 && self.start * 2 > self.buf.len())
        {
            self.buf.drain(..self.start);
            self.start = 0;
            self.newest = [NONE; 32];
        }
    }

    /// Copy whole records into `out`; the number of bytes written. A record that does not fit
    /// stays queued (a buffer of [`MAX_RECORD`] bytes always makes progress).
    pub fn read(&mut self, out: &mut [u8]) -> usize {
        let mut written = 0;
        loop {
            if self.start >= self.buf.len() {
                break;
            }
            let at = self.start;
            let len = rd32(&self.buf, at).ix();
            if len < HEADER || at + len > self.buf.len() {
                self.buf.clear();
                self.start = 0;
                self.count = 0;
                break;
            }
            if rd16(&self.buf, at + 4) == DEAD {
                self.start += len;
                continue;
            }
            if written + len > out.len() {
                break;
            }
            out[written..written + len].copy_from_slice(&self.buf[at..at + len]);
            written += len;
            self.start += len;
            self.count -= 1;
        }
        if self.start >= self.buf.len() {
            self.buf.clear();
            self.start = 0;
            self.newest = [NONE; 32];
        }
        written
    }
}

/// A decoded record header, for tests and the Rust side of the C examples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub len: u32,
    pub kind: u16,
    pub flags: u16,
    pub window: u32,
    pub node: u32,
    pub seq: u32,
}

/// Split a buffer from `read` into records: header and payload slice.
pub fn parse(buf: &[u8]) -> Vec<(Header, &[u8])> {
    let (mut at, mut out) = (0, Vec::new());
    while at + HEADER <= buf.len() {
        let len = rd32(buf, at).ix();
        if len < HEADER || !len.is_multiple_of(8) || at + len > buf.len() {
            break;
        }
        out.push((
            Header {
                len: to_u32(len),
                kind: rd16(buf, at + 4),
                flags: rd16(buf, at + 6),
                window: rd32(buf, at + 8),
                node: rd32(buf, at + 12),
                seq: rd32(buf, at + 16),
            },
            &buf[at + HEADER..at + len],
        ));
        at += len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::ids::to_u8;

    fn drain(q: &mut EventQueue) -> Vec<(Header, Vec<u8>)> {
        let mut buf = vec![0u8; MAX_BYTES];
        let n = q.read(&mut buf);
        parse(&buf[..n])
            .into_iter()
            .map(|(h, p)| (h, p.to_vec()))
            .collect()
    }

    #[test]
    fn records_are_aligned_and_ordered() {
        let mut q = EventQueue::new();
        q.push(kind::CLICK, 1, 5, 9, &[1, 0, 0, 0], &[]);
        q.push(kind::TEXT_CHANGED, 1, 6, 9, &[3, 0, 0, 0], b"abc");
        let r = drain(&mut q);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].0.kind, r[0].0.node, r[0].0.seq), (kind::CLICK, 5, 9));
        assert_eq!(r[0].0.len % 8, 0);
        assert_eq!(&r[1].1[..7], &[3, 0, 0, 0, b'a', b'b', b'c']);
        assert!(q.is_empty());
    }

    #[test]
    fn newest_scroll_replaces_the_pending_one() {
        let mut q = EventQueue::new();
        for i in 0..100u32 {
            q.push(kind::SCROLL, 1, 2, i, &[to_u8(i), 0, 0, 0, 0, 0, 0, 0], &[]);
        }
        assert_eq!(q.len(), 1);
        let r = drain(&mut q);
        assert_eq!((r[0].0.seq, r[0].1[0]), (99, 99));
    }

    #[test]
    fn overflow_drops_coalescable_first_and_flags_the_next_record() {
        let mut q = EventQueue::new();
        for i in 0..to_u32(MAX_PENDING) {
            // Distinct nodes defeat replacement, so the queue really fills.
            q.push(kind::SCROLL, 1, i, 0, &[0; 8], &[]);
        }
        assert_eq!(q.len(), MAX_PENDING);
        q.push(kind::CLICK, 1, 1, 0, &[0; 16], &[]);
        assert_eq!(q.len(), MAX_PENDING);
        let r = drain(&mut q);
        let click = r.iter().find(|(h, _)| h.kind == kind::CLICK).unwrap();
        assert_eq!(click.0.flags & FLAG_OVERFLOW, FLAG_OVERFLOW);
        assert_eq!(r.len(), MAX_PENDING);
    }

    #[test]
    fn queue_of_clicks_stays_bounded() {
        let mut q = EventQueue::new();
        for i in 0..to_u32(MAX_PENDING * 2) {
            q.push(kind::CLICK, 1, i, 0, &[0; 16], &[]);
        }
        assert!(q.len() <= MAX_PENDING);
        assert!(q.overflowed() || !q.is_empty());
    }

    #[test]
    fn a_small_buffer_gets_whole_records_only() {
        let mut q = EventQueue::new();
        q.push(kind::CLICK, 1, 1, 0, &[0; 16], &[]);
        q.push(kind::CLICK, 1, 2, 0, &[0; 16], &[]);
        let mut b = [0u8; 50];
        assert_eq!(q.read(&mut b), 40);
        assert_eq!(q.len(), 1);
        let mut tiny = [0u8; 10];
        assert_eq!(q.read(&mut tiny), 0);
        assert_eq!(q.len(), 1);
    }
}
