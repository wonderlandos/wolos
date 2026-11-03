// SPDX-License-Identifier: BSD-2-Clause
//! Patch stream v1 (interface I10, frozen). One `wolkit_window_apply` carries a whole batch of
//! scene changes; this module is the decoder and validator (nothing here touches a scene) and a
//! [`Writer`] that builds streams for tests and Rust callers. The Crystal shard has its own encoder
//! for the same bytes.
//!
//! ```text
//! header, 24 bytes:  u32 magic "WKP1"  u16 format (1)  u16 flags (0)  u32 seq  u32 op_count
//!                    u32 pool_len      u32 reserved (0)
//! ops:               u16 opcode  u16 (bit 15 MUST_UNDERSTAND | bits 0..14 length in 4-byte words,
//!                    header included)  then words of payload
//! string pool:       pool_len bytes at the very end; entries are  u32 len  bytes  zero padding to
//!                    a multiple of 4. A string reference is the byte offset of an entry.
//! ```
//!
//! Everything is little-endian. A whole patch is a multiple of 4 bytes and at most
//! [`MAX_PATCH`]. An unknown opcode without MUST_UNDERSTAND is skipped, so a newer app can send
//! an optional op to an older engine; with it, the whole patch is refused. A known op has exactly
//! its documented number of words.

use super::kind::Kind;
use super::prop::{self, MAX_WORDS, PropDef};
use super::strings::NO_STR;
use crate::error::{Error, Result};

pub const MAGIC: u32 = u32::from_le_bytes(*b"WKP1");
pub const FORMAT: u16 = 1;
pub const HEADER: usize = 24;
/// Bytes per `wolkit_window_apply` (plan section 2.8).
pub const MAX_PATCH: usize = 1 << 20;
/// Bytes of one string (text per node).
pub const MAX_STRING: usize = 64 << 10;
/// Classes one `SET_STYLE` can name.
pub const MAX_CLASSES: usize = 8;
/// `INSERT`/`MOVE` index meaning "after the last child".
pub const APPEND: u32 = u32::MAX;
pub const MUST_UNDERSTAND: u16 = 0x8000;

/// Opcodes (`WOLKIT_OP_*`), append-only.
pub mod op {
    pub const CREATE: u16 = 1;
    pub const INSERT: u16 = 2;
    pub const REMOVE: u16 = 3;
    pub const MOVE: u16 = 4;
    pub const SET_PROP: u16 = 5;
    pub const SET_TEXT: u16 = 6;
    pub const SET_STYLE: u16 = 7;
    pub const SET_STATE: u16 = 8;
    pub const SET_KEY: u16 = 9;
    pub const BIND_EVENT: u16 = 10;
    pub const SET_A11Y: u16 = 11;
    pub const ANIM: u16 = 12;
    pub const LIST_RANGE: u16 = 13;
    pub const FOCUS: u16 = 14;
    pub const FREE: u16 = 15;
    pub const NAMES: &[(&str, u16)] = &[
        ("CREATE", CREATE),
        ("INSERT", INSERT),
        ("REMOVE", REMOVE),
        ("MOVE", MOVE),
        ("SET_PROP", SET_PROP),
        ("SET_TEXT", SET_TEXT),
        ("SET_STYLE", SET_STYLE),
        ("SET_STATE", SET_STATE),
        ("SET_KEY", SET_KEY),
        ("BIND_EVENT", BIND_EVENT),
        ("SET_A11Y", SET_A11Y),
        ("ANIM", ANIM),
        ("LIST_RANGE", LIST_RANGE),
        ("FOCUS", FOCUS),
        ("FREE", FREE),
    ];
}

/// `ANIM` modes.
pub const ANIM_TWEEN: u32 = 0;
pub const ANIM_SPRING: u32 = 1;

/// A property value: up to [`MAX_WORDS`] words.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PropValue {
    pub n: u8,
    pub w: [u32; MAX_WORDS],
}

impl PropValue {
    pub fn new(words: &[u32]) -> Self {
        let mut v = PropValue::default();
        let n = words.len().min(MAX_WORDS);
        v.w[..n].copy_from_slice(&words[..n]);
        v.n = n as u8;
        v
    }

    pub fn words(&self) -> &[u32] {
        &self.w[..self.n as usize]
    }

    pub fn f32(&self) -> f32 {
        f32::from_bits(self.w[0])
    }
}

/// One decoded op. Strings borrow from the patch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op<'a> {
    Create { node: u32, kind: Kind },
    Insert { parent: u32, child: u32, index: u32 },
    Remove { node: u32 },
    Move { node: u32, parent: u32, index: u32 },
    /// `text` is the string of a `Str` property (the value word is its pool reference); else "".
    SetProp { node: u32, def: &'static PropDef, value: PropValue, text: &'a str },
    SetText { node: u32, text: &'a str },
    SetStyle { node: u32, n: u8, classes: [u32; MAX_CLASSES] },
    SetState { node: u32, state: u32 },
    SetKey { node: u32, key: u64 },
    BindEvent { node: u32, mask: u32 },
    SetA11y { node: u32, role: u32, name: &'a str, description: &'a str },
    Anim { node: u32, prop: u32, target: u32, mode: u32, param: u32 },
    ListRange { node: u32, count: u32, height: f32, overscan: u32 },
    Focus { node: u32 },
    Free { node: u32 },
}

/// Reusable buffers of the validator, so a steady-state apply allocates nothing.
#[derive(Default)]
pub struct Scratch {
    /// Start offsets of the pool entries, ascending.
    entries: Vec<u32>,
}

/// A patch that passed [`Patch::parse`].
#[derive(Clone, Copy)]
pub struct Patch<'a> {
    buf: &'a [u8],
    pool: &'a [u8],
    pub seq: u32,
    pub op_count: u32,
}

fn bad(offset: usize, what: &str) -> Error {
    Error::invalid(format!("The patch is not valid at byte {offset}: {what}"))
}

fn rd32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn rd16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

impl<'a> Patch<'a> {
    /// Check the whole stream: header, framing, every op's shape and values, the string pool
    /// (UTF-8 once, here) and every string reference. Nothing about a scene is checked here.
    pub fn parse(buf: &'a [u8], scratch: &mut Scratch) -> Result<Patch<'a>> {
        if buf.len() > MAX_PATCH {
            return Err(Error::limit(format!(
                "A patch is at most {MAX_PATCH} bytes; send the changes in several."
            )));
        }
        if buf.len() < HEADER || buf.len() % 4 != 0 {
            return Err(bad(0, "a patch is a 24-byte header and a whole number of 4-byte words."));
        }
        let (magic, format) = (rd32(buf, 0).unwrap_or(0), rd16(buf, 4).unwrap_or(0));
        if magic != MAGIC {
            return Err(bad(0, "this is not a Wolkit patch (the magic is wrong)."));
        }
        if format != FORMAT {
            return Err(Error::unavailable(format!(
                "This engine reads patch format {FORMAT}, not {format}; update libwolkit."
            )));
        }
        if rd16(buf, 6) != Some(0) || rd32(buf, 20) != Some(0) {
            return Err(bad(6, "the reserved header fields must be zero."));
        }
        let seq = rd32(buf, 8).unwrap_or(0);
        let op_count = rd32(buf, 12).unwrap_or(0);
        let pool_len = rd32(buf, 16).unwrap_or(0) as usize;
        if pool_len % 4 != 0 || pool_len > buf.len() - HEADER {
            return Err(bad(16, "the string pool does not fit in the patch."));
        }
        let pool_start = buf.len() - pool_len;
        let pool = &buf[pool_start..];
        // The pool: entries back to back, each checked as UTF-8 once.
        scratch.entries.clear();
        let mut off = 0usize;
        while off < pool.len() {
            let len = rd32(pool, off).ok_or_else(|| bad(pool_start + off, "a string entry is cut short."))? as usize;
            if len > MAX_STRING {
                return Err(Error::limit(format!("A string is at most {MAX_STRING} bytes.")));
            }
            let end = off + 4 + len;
            let text = pool
                .get(off + 4..end)
                .ok_or_else(|| bad(pool_start + off, "a string entry runs past the pool."))?;
            if std::str::from_utf8(text).is_err() {
                return Err(bad(pool_start + off, "a string is not valid UTF-8."));
            }
            scratch.entries.push(off as u32);
            off = (end + 3) & !3;
        }
        if off != pool.len() {
            return Err(bad(pool_start, "the string pool has a stray tail."));
        }
        let patch = Patch {
            buf: &buf[..pool_start],
            pool,
            seq,
            op_count,
        };
        // The ops.
        let (mut at, mut seen) = (HEADER, 0u32);
        while at < pool_start {
            let (op, next) = patch.decode(at, Some(&scratch.entries))?;
            let _ = op;
            at = next;
            seen += 1;
            if seen > op_count {
                break;
            }
        }
        if seen != op_count || at != pool_start {
            return Err(bad(12, "the operation count in the header does not match the stream."));
        }
        Ok(patch)
    }

    /// The ops, in order. Unknown optional ops are skipped.
    pub fn ops(&self) -> Ops<'a> {
        Ops {
            patch: *self,
            at: HEADER,
        }
    }

    /// The text of the pool entry at `offset`, if there is one.
    fn string(&self, offset: u32, entries: Option<&[u32]>) -> Option<&'a str> {
        if offset == NO_STR {
            return Some("");
        }
        if let Some(e) = entries {
            e.binary_search(&offset).ok()?;
        }
        let off = offset as usize;
        let len = rd32(self.pool, off)? as usize;
        std::str::from_utf8(self.pool.get(off + 4..off + 4 + len)?).ok()
    }

    /// Decode the op at `at`. With `entries` (validation) every string reference must be the
    /// start of a pool entry; without (iteration) the stream was validated already.
    fn decode(&self, at: usize, entries: Option<&[u32]>) -> Result<(Option<Op<'a>>, usize)> {
        let buf = self.buf;
        let (code, lw) = match (rd16(buf, at), rd16(buf, at + 2)) {
            (Some(c), Some(l)) => (c, l),
            _ => return Err(bad(at, "an operation is cut short.")),
        };
        let words = (lw & !MUST_UNDERSTAND) as usize;
        let must = lw & MUST_UNDERSTAND != 0;
        if words == 0 {
            return Err(bad(at, "an operation has no length."));
        }
        let end = at + words * 4;
        if end > buf.len() {
            return Err(bad(at, "an operation runs past the end of the patch."));
        }
        let w = |i: usize| rd32(buf, at + 4 + i * 4).unwrap_or(0);
        let n = words - 1;
        let want = |k: usize| -> Result<()> {
            if n == k {
                Ok(())
            } else {
                Err(bad(at, &format!("this operation takes {k} words, not {n}.")))
            }
        };
        let s = |off: u32| {
            self.string(off, entries)
                .ok_or_else(|| bad(at, "a string reference does not point at a string."))
        };
        let op = match code {
            op::CREATE => {
                want(2)?;
                let kind = Kind::from_u32(w(1)).ok_or_else(|| bad(at, "that is not a node kind."))?;
                Op::Create { node: w(0), kind }
            }
            op::INSERT => {
                want(3)?;
                Op::Insert { parent: w(0), child: w(1), index: w(2) }
            }
            op::REMOVE => {
                want(1)?;
                Op::Remove { node: w(0) }
            }
            op::MOVE => {
                want(3)?;
                Op::Move { node: w(0), parent: w(1), index: w(2) }
            }
            op::SET_PROP => {
                if n < 3 {
                    return Err(bad(at, "SET_PROP needs a node, a property and a value."));
                }
                let def = prop::def(w(1)).ok_or_else(|| bad(at, "that is not a property id."))?;
                want(2 + def.ty.arity())?;
                let mut value = [0u32; MAX_WORDS];
                for (i, v) in value.iter_mut().enumerate().take(def.ty.arity()) {
                    *v = w(2 + i);
                }
                let value = PropValue::new(&value[..def.ty.arity()]);
                prop::check_value(def, value.words()).map_err(|m| bad(at, m))?;
                let text = if def.ty == prop::Ty::Str { s(value.w[0])? } else { "" };
                Op::SetProp { node: w(0), def, value, text }
            }
            op::SET_TEXT => {
                want(2)?;
                Op::SetText { node: w(0), text: s(w(1))? }
            }
            op::SET_STYLE => {
                if n < 2 {
                    return Err(bad(at, "SET_STYLE names a node and up to 8 classes."));
                }
                let count = w(1) as usize;
                if count != n - 2 || count > MAX_CLASSES {
                    return Err(bad(at, "SET_STYLE names a node and up to 8 classes."));
                }
                let mut classes = [0u32; MAX_CLASSES];
                for (i, c) in classes.iter_mut().enumerate().take(count) {
                    *c = w(2 + i);
                }
                Op::SetStyle { node: w(0), n: count as u8, classes }
            }
            op::SET_STATE => {
                want(2)?;
                Op::SetState { node: w(0), state: w(1) }
            }
            op::SET_KEY => {
                want(3)?;
                Op::SetKey { node: w(0), key: u64::from(w(1)) | (u64::from(w(2)) << 32) }
            }
            op::BIND_EVENT => {
                want(2)?;
                Op::BindEvent { node: w(0), mask: w(1) }
            }
            op::SET_A11Y => {
                want(4)?;
                let role = w(1);
                if !super::kind::ROLES.iter().any(|r| r.0 == role) {
                    return Err(bad(at, "that is not an accessibility role."));
                }
                Op::SetA11y { node: w(0), role, name: s(w(2))?, description: s(w(3))? }
            }
            op::ANIM => {
                want(5)?;
                let d = prop::def(w(1)).ok_or_else(|| bad(at, "that is not a property id."))?;
                if d.ty != prop::Ty::F32 || w(3) > ANIM_SPRING || w(4) > 3 {
                    return Err(bad(at, "only number properties animate, as a tween or a spring."));
                }
                if !f32::from_bits(w(2)).is_finite() {
                    return Err(bad(at, "an animation target is a finite number."));
                }
                Op::Anim { node: w(0), prop: w(1), target: w(2), mode: w(3), param: w(4) }
            }
            op::LIST_RANGE => {
                want(4)?;
                let h = f32::from_bits(w(2));
                if !h.is_finite() || h <= 0.0 || w(1) as usize > super::ids::MAX_NODES * 64 {
                    return Err(bad(at, "a list needs a row height above zero and a sane count."));
                }
                Op::ListRange { node: w(0), count: w(1), height: h, overscan: w(3).min(64) }
            }
            op::FOCUS => {
                want(1)?;
                Op::Focus { node: w(0) }
            }
            op::FREE => {
                want(1)?;
                Op::Free { node: w(0) }
            }
            other if must => {
                return Err(Error::unavailable(format!(
                    "This patch uses operation {other}, which this engine does not know; update libwolkit."
                )));
            }
            _ => return Ok((None, end)),
        };
        Ok((Some(op), end))
    }
}

/// Iterator over a validated patch.
pub struct Ops<'a> {
    patch: Patch<'a>,
    at: usize,
}

impl<'a> Iterator for Ops<'a> {
    type Item = Op<'a>;

    fn next(&mut self) -> Option<Op<'a>> {
        while self.at < self.patch.buf.len() {
            let (op, next) = self.patch.decode(self.at, None).ok()?;
            self.at = next;
            if op.is_some() {
                return op;
            }
        }
        None
    }
}

/// Builds a patch stream. Reuse one writer per window: `clear`, add ops, `finish`.
#[derive(Default)]
pub struct Writer {
    ops: Vec<u8>,
    pool: Vec<u8>,
    out: Vec<u8>,
    count: u32,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.ops.clear();
        self.pool.clear();
        self.out.clear();
        self.count = 0;
    }

    pub fn op_count(&self) -> u32 {
        self.count
    }

    /// Bytes the finished patch would take.
    pub fn len(&self) -> usize {
        HEADER + self.ops.len() + self.pool.len()
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Append an op with raw payload words (the typed helpers below call this).
    pub fn raw(&mut self, code: u16, words: &[u32]) -> &mut Self {
        let lw = (words.len() as u16 + 1) | MUST_UNDERSTAND;
        self.ops.extend_from_slice(&code.to_le_bytes());
        self.ops.extend_from_slice(&lw.to_le_bytes());
        for w in words {
            self.ops.extend_from_slice(&w.to_le_bytes());
        }
        self.count += 1;
        self
    }

    /// Put a string in the pool; the result is the reference to give an op.
    pub fn string(&mut self, s: &str) -> u32 {
        if s.is_empty() {
            return NO_STR;
        }
        let off = self.pool.len() as u32;
        self.pool.extend_from_slice(&(s.len() as u32).to_le_bytes());
        self.pool.extend_from_slice(s.as_bytes());
        while self.pool.len() % 4 != 0 {
            self.pool.push(0);
        }
        off
    }

    pub fn create(&mut self, node: u32, kind: Kind) -> &mut Self {
        self.raw(op::CREATE, &[node, kind as u32])
    }

    pub fn insert(&mut self, parent: u32, child: u32, index: u32) -> &mut Self {
        self.raw(op::INSERT, &[parent, child, index])
    }

    pub fn remove(&mut self, node: u32) -> &mut Self {
        self.raw(op::REMOVE, &[node])
    }

    pub fn move_to(&mut self, node: u32, parent: u32, index: u32) -> &mut Self {
        self.raw(op::MOVE, &[node, parent, index])
    }

    pub fn set_prop(&mut self, node: u32, prop: u16, value: &[u32]) -> &mut Self {
        let mut words = [0u32; 2 + MAX_WORDS];
        words[0] = node;
        words[1] = u32::from(prop);
        let n = value.len().min(MAX_WORDS);
        words[2..2 + n].copy_from_slice(&value[..n]);
        self.raw(op::SET_PROP, &words[..2 + n])
    }

    pub fn set_f32(&mut self, node: u32, prop: u16, v: f32) -> &mut Self {
        self.set_prop(node, prop, &[v.to_bits()])
    }

    pub fn set_text(&mut self, node: u32, text: &str) -> &mut Self {
        let r = self.string(text);
        self.raw(op::SET_TEXT, &[node, r])
    }

    pub fn set_style(&mut self, node: u32, classes: &[u32]) -> &mut Self {
        let mut words = [0u32; 2 + MAX_CLASSES];
        let n = classes.len().min(MAX_CLASSES);
        words[0] = node;
        words[1] = n as u32;
        words[2..2 + n].copy_from_slice(&classes[..n]);
        self.raw(op::SET_STYLE, &words[..2 + n])
    }

    pub fn set_state(&mut self, node: u32, state: u32) -> &mut Self {
        self.raw(op::SET_STATE, &[node, state])
    }

    pub fn set_key(&mut self, node: u32, key: u64) -> &mut Self {
        self.raw(op::SET_KEY, &[node, key as u32, (key >> 32) as u32])
    }

    pub fn bind_event(&mut self, node: u32, mask: u32) -> &mut Self {
        self.raw(op::BIND_EVENT, &[node, mask])
    }

    pub fn set_a11y(&mut self, node: u32, role: u32, name: &str, description: &str) -> &mut Self {
        let (n, d) = (self.string(name), self.string(description));
        self.raw(op::SET_A11Y, &[node, role, n, d])
    }

    pub fn anim(&mut self, node: u32, prop: u16, target: f32, mode: u32, param: u32) -> &mut Self {
        self.raw(op::ANIM, &[node, u32::from(prop), target.to_bits(), mode, param])
    }

    pub fn list_range(&mut self, node: u32, count: u32, height: f32, overscan: u32) -> &mut Self {
        self.raw(op::LIST_RANGE, &[node, count, height.to_bits(), overscan])
    }

    pub fn focus(&mut self, node: u32) -> &mut Self {
        self.raw(op::FOCUS, &[node])
    }

    pub fn free(&mut self, node: u32) -> &mut Self {
        self.raw(op::FREE, &[node])
    }

    /// The finished patch with sequence number `seq`; valid until the next `clear`.
    pub fn finish(&mut self, seq: u32) -> &[u8] {
        self.out.clear();
        self.out.extend_from_slice(&MAGIC.to_le_bytes());
        self.out.extend_from_slice(&FORMAT.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&seq.to_le_bytes());
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&(self.pool.len() as u32).to_le_bytes());
        self.out.extend_from_slice(&0u32.to_le_bytes());
        self.out.extend_from_slice(&self.ops);
        self.out.extend_from_slice(&self.pool);
        &self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::prop::id;

    fn sample() -> Vec<u8> {
        let mut w = Writer::new();
        w.create(1, Kind::Box).create(2, Kind::Text).insert(0, 1, APPEND).insert(1, 2, 0);
        w.set_text(2, "Hello").set_f32(1, id::GROW, 1.0).set_style(1, &[3, 4]);
        w.set_a11y(2, 2, "Greeting", "").set_state(1, 16).focus(1);
        w.finish(7).to_vec()
    }

    #[test]
    fn round_trips() {
        let b = sample();
        let mut sc = Scratch::default();
        let p = Patch::parse(&b, &mut sc).unwrap();
        assert_eq!((p.seq, p.op_count), (7, 10));
        let ops: Vec<_> = p.ops().collect();
        assert_eq!(ops.len(), 10);
        assert_eq!(ops[0], Op::Create { node: 1, kind: Kind::Box });
        assert_eq!(ops[4], Op::SetText { node: 2, text: "Hello" });
        assert!(matches!(ops[6], Op::SetStyle { n: 2, .. }));
        assert!(matches!(ops[7], Op::SetA11y { name: "Greeting", description: "", .. }));
    }

    #[test]
    fn every_truncation_is_refused_without_panicking() {
        let b = sample();
        let mut sc = Scratch::default();
        for cut in 0..b.len() {
            assert!(Patch::parse(&b[..cut], &mut sc).is_err(), "cut {cut}");
        }
    }

    #[test]
    fn single_byte_changes_never_panic() {
        let b = sample();
        let mut sc = Scratch::default();
        for i in 0..b.len() {
            for v in [0u8, 1, 0x7f, 0x80, 0xff] {
                let mut c = b.clone();
                c[i] = v;
                if let Ok(p) = Patch::parse(&c, &mut sc) {
                    let _ = p.ops().count();
                }
            }
        }
    }

    #[test]
    fn unknown_ops_follow_their_flag() {
        let mut w = Writer::new();
        w.create(1, Kind::Box);
        let mut b = w.finish(1).to_vec();
        // Splice an unknown op (opcode 900) before the end of the ops, optional then required.
        for (flag, ok) in [(0u16, true), (MUST_UNDERSTAND, false)] {
            let mut c = b.clone();
            let mut op = Vec::new();
            op.extend_from_slice(&900u16.to_le_bytes());
            op.extend_from_slice(&((2u16) | flag).to_le_bytes());
            op.extend_from_slice(&0u32.to_le_bytes());
            let at = c.len();
            c.splice(at..at, op);
            c[12..16].copy_from_slice(&2u32.to_le_bytes());
            let r = Patch::parse(&c, &mut Scratch::default());
            assert_eq!(r.is_ok(), ok);
            if let Ok(p) = r {
                assert_eq!(p.ops().count(), 1);
            }
        }
        b[4] = 2;
        assert!(Patch::parse(&b, &mut Scratch::default()).is_err());
    }

    #[test]
    fn bad_strings_and_values_are_refused() {
        let mut sc = Scratch::default();
        let mut w = Writer::new();
        w.create(1, Kind::Text);
        w.raw(op::SET_TEXT, &[1, 4]); // 4 is not a pool entry
        assert!(Patch::parse(w.finish(1), &mut sc).is_err());
        let mut w = Writer::new();
        let r = w.string("ok");
        w.raw(op::SET_TEXT, &[1, r]);
        let mut b = w.finish(1).to_vec();
        let n = b.len();
        b[n - 4] = 0xff; // invalid UTF-8 in the pool
        assert!(Patch::parse(&b, &mut sc).is_err());
        let mut w = Writer::new();
        w.set_prop(1, id::BACKGROUND, &[99]);
        assert!(Patch::parse(w.finish(1), &mut sc).is_err());
        let mut w = Writer::new();
        w.set_f32(1, id::GROW, f32::NAN);
        assert!(Patch::parse(w.finish(1), &mut sc).is_err());
        let mut w = Writer::new();
        w.raw(op::CREATE, &[1]);
        assert!(Patch::parse(w.finish(1), &mut sc).is_err());
        assert!(Patch::parse(&vec![0u8; MAX_PATCH + 4], &mut sc).is_err());
    }
}
