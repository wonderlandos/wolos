// SPDX-License-Identifier: BSD-2-Clause
//! A small chunked rope for the text editor.
//!
//! The text is a vector of UTF-8 chunks of at most [`MAX_CHUNK`] bytes. A `text_input` holds at
//! most 64 KiB, so the vector stays under about 128 entries and locating a byte offset is a short
//! linear scan; an edit touches one or two chunks and never copies the whole text. The type is
//! safe Rust throughout (the 1.0 editor avoids dependencies with `unsafe` for a path that takes
//! untrusted clipboard and IME text). Offsets are byte offsets and must lie on character
//! boundaries; every method checks that and returns `false`/`None` instead of panicking.

use std::ops::Range;

/// A chunk is split once it grows past this many bytes.
pub const MAX_CHUNK: usize = 1024;
/// Neighbouring chunks are merged while their sum stays at or below this.
const MERGE_BELOW: usize = 512;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rope {
    chunks: Vec<String>,
    len: usize,
}

impl Rope {
    pub fn new() -> Rope {
        Rope::default()
    }

    pub fn from_text(s: &str) -> Rope {
        let mut r = Rope::new();
        r.insert(0, s);
        r
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn chunks(&self) -> impl Iterator<Item = &str> {
        self.chunks.iter().map(String::as_str)
    }

    /// The chunk holding byte `at` and the offset of that chunk's start. `at == len` maps to
    /// the end of the last chunk.
    fn locate(&self, at: usize) -> Option<(usize, usize)> {
        let mut start = 0;
        for (i, c) in self.chunks.iter().enumerate() {
            if at < start + c.len() || (at == start + c.len() && i + 1 == self.chunks.len()) {
                return Some((i, start));
            }
            start += c.len();
        }
        None
    }

    pub fn is_char_boundary(&self, at: usize) -> bool {
        if at > self.len {
            return false;
        }
        if at == self.len || at == 0 {
            return true;
        }
        match self.locate(at) {
            Some((i, start)) => self.chunks[i].is_char_boundary(at - start),
            None => false,
        }
    }

    pub fn char_at(&self, at: usize) -> Option<char> {
        let (i, start) = self.locate(at)?;
        self.chunks[i][at - start..].chars().next()
    }

    pub fn char_before(&self, at: usize) -> Option<char> {
        let p = self.prev_char_boundary(at)?;
        self.char_at(p)
    }

    /// The boundary of the previous character, or `None` at the start.
    pub fn prev_char_boundary(&self, at: usize) -> Option<usize> {
        if at == 0 || at > self.len {
            return None;
        }
        let mut p = at - 1;
        while !self.is_char_boundary(p) {
            p -= 1;
        }
        Some(p)
    }

    /// The boundary after the character at `at`, or `None` at the end.
    pub fn next_char_boundary(&self, at: usize) -> Option<usize> {
        let c = self.char_at(at)?;
        Some(at + c.len_utf8())
    }

    /// Copies out `range` (empty string when the range is invalid).
    pub fn slice(&self, range: Range<usize>) -> String {
        let mut out = String::new();
        if range.start >= range.end || range.end > self.len {
            return out;
        }
        if !self.is_char_boundary(range.start) || !self.is_char_boundary(range.end) {
            return out;
        }
        let mut start = 0;
        for c in &self.chunks {
            let end = start + c.len();
            if end > range.start && start < range.end {
                let a = range.start.max(start) - start;
                let b = range.end.min(end) - start;
                out.push_str(&c[a..b]);
            }
            start = end;
            if start >= range.end {
                break;
            }
        }
        out
    }

    /// Copies out the whole text.
    pub fn to_text(&self) -> String {
        let mut s = String::with_capacity(self.len);
        for c in &self.chunks {
            s.push_str(c);
        }
        s
    }

    /// Inserts `s` at `at`. Returns false (and changes nothing) when `at` is not a boundary.
    pub fn insert(&mut self, at: usize, s: &str) -> bool {
        if s.is_empty() {
            return at <= self.len && self.is_char_boundary(at);
        }
        if !self.is_char_boundary(at) {
            return false;
        }
        if self.chunks.is_empty() {
            self.chunks.push(String::new());
        }
        let (i, start) = self.locate(at).unwrap_or((
            self.chunks.len() - 1,
            self.len - self.chunks[self.chunks.len() - 1].len(),
        ));
        self.chunks[i].insert_str(at - start, s);
        self.len += s.len();
        self.split_if_big(i);
        true
    }

    fn split_if_big(&mut self, mut i: usize) {
        while self.chunks[i].len() > MAX_CHUNK {
            let mid = {
                let c = &self.chunks[i];
                let mut m = c.len() / 2;
                while !c.is_char_boundary(m) {
                    m -= 1;
                }
                m
            };
            let tail = self.chunks[i].split_off(mid);
            self.chunks.insert(i + 1, tail);
            if self.chunks[i].len() <= MAX_CHUNK {
                i += 1;
            }
        }
    }

    /// Removes `range`. Returns false (and changes nothing) when it is not on boundaries.
    pub fn remove(&mut self, range: Range<usize>) -> bool {
        if range.start > range.end || range.end > self.len {
            return false;
        }
        if range.start == range.end {
            return self.is_char_boundary(range.start);
        }
        if !self.is_char_boundary(range.start) || !self.is_char_boundary(range.end) {
            return false;
        }
        let mut start = 0;
        for c in &mut self.chunks {
            let end = start + c.len();
            if end > range.start && start < range.end {
                let a = range.start.max(start) - start;
                let b = range.end.min(end) - start;
                c.replace_range(a..b, "");
            }
            start = end;
            if start >= range.end {
                break;
            }
        }
        self.len -= range.end - range.start;
        self.chunks.retain(|c| !c.is_empty());
        self.merge_small();
        true
    }

    fn merge_small(&mut self) {
        let mut i = 0;
        while i + 1 < self.chunks.len() {
            if self.chunks[i].len() + self.chunks[i + 1].len() <= MERGE_BELOW {
                let next = self.chunks.remove(i + 1);
                self.chunks[i].push_str(&next);
            } else {
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_remove_matches_string() {
        let mut r = Rope::new();
        let mut s = String::new();
        let mut seed = 0x1234_5678_9abc_def1u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let alphabet = ["a", "bc", "é", "日本", "🧑‍🚀", " ", "xyz123"];
        for step in 0..4000 {
            if s.is_empty() || next() % 3 != 0 {
                let piece = alphabet[(next() % alphabet.len() as u64) as usize]
                    .repeat(1 + (next() % 40) as usize);
                let mut at = (next() % (s.len() as u64 + 1)) as usize;
                while !s.is_char_boundary(at) {
                    at -= 1;
                }
                assert!(r.insert(at, &piece));
                s.insert_str(at, &piece);
            } else {
                let mut a = (next() % s.len() as u64) as usize;
                while !s.is_char_boundary(a) {
                    a -= 1;
                }
                let mut b = (a + (next() % 300) as usize).min(s.len());
                while !s.is_char_boundary(b) {
                    b -= 1;
                }
                assert!(r.remove(a..b));
                s.replace_range(a..b, "");
            }
            assert_eq!(r.len(), s.len(), "step {step}");
            if step % 97 == 0 {
                assert_eq!(r.to_text(), s);
            }
        }
        assert_eq!(r.to_text(), s);
        assert!(r.chunks().all(|c| c.len() <= MAX_CHUNK));
    }

    #[test]
    fn rejects_mid_character_offsets() {
        let mut r = Rope::from_text("é");
        assert!(!r.insert(1, "x"));
        assert!(!r.remove(0..1));
        assert_eq!(r.to_text(), "é");
        assert_eq!(r.slice(0..1), "");
        assert!(!r.is_char_boundary(1));
        assert!(r.is_char_boundary(2));
    }

    #[test]
    fn boundaries_and_chars() {
        let r = Rope::from_text("a日🧑");
        assert_eq!(r.next_char_boundary(0), Some(1));
        assert_eq!(r.next_char_boundary(1), Some(4));
        assert_eq!(r.prev_char_boundary(4), Some(1));
        assert_eq!(r.prev_char_boundary(0), None);
        assert_eq!(r.next_char_boundary(r.len()), None);
        assert_eq!(r.char_before(4), Some('日'));
    }

    #[test]
    fn large_text_spans_chunks() {
        let big = "0123456789".repeat(1000);
        let mut r = Rope::from_text(&big);
        assert!(r.chunk_count() > 1);
        assert_eq!(r.slice(5000..5010), "0123456789");
        r.remove(100..9900);
        assert_eq!(r.len(), 200);
        assert!(r.chunk_count() <= 2);
    }
}
