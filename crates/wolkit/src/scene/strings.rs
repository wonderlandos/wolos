// SPDX-License-Identifier: BSD-2-Clause
//! Interned, reference-counted strings. Labels repeat (a list of rows with the same button text),
//! so the scene keeps one copy; a node holds a [`StrId`]. Setting a text that is already interned
//! allocates nothing, which is what keeps the steady-state apply allocation-free.

use crate::scene::ids::{Ix, to_u32};

/// Index of an interned string; [`NO_STR`] is the empty string and is never stored.
pub type StrId = u32;
pub const NO_STR: StrId = u32::MAX;
/// Most bytes of interned text per engine window.
pub const MAX_BYTES: usize = 64 << 20;

const EMPTY: u32 = u32::MAX;
/// `Entry::off` of an entry that is on the free list (not in the table, no bytes).
const FREED: u32 = u32::MAX;

#[derive(Clone, Copy)]
struct Entry {
    off: u32,
    len: u32,
    refs: u32,
    hash: u32,
}

/// Text that nothing refers to any more stays in the table (`refs` 0, bytes kept) so the next
/// `intern` of the same text revives it without touching memory: a label that flips between two
/// values costs no allocation. The unreferenced entries are dropped together, on `compact`, once
/// they are most of the heap.
#[derive(Default)]
pub struct Strings {
    heap: String,
    entries: Vec<Entry>,
    free: Vec<u32>,
    buckets: Vec<u32>,
    /// Entries with `refs > 0`.
    used: usize,
    /// Bytes of entries with `refs == 0` that are still in the table.
    dead_bytes: usize,
}

fn hash(s: &str) -> u32 {
    // FNV-1a, 32 bit: the table is keyed by content the app chose, so no secret is needed; a
    // collision only costs a longer probe, bounded by the table size.
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

impl Strings {
    pub fn new() -> Self {
        Self::default()
    }

    /// The text of `id`; "" for [`NO_STR`] or an id that nothing refers to.
    pub fn get(&self, id: StrId) -> &str {
        self.entries
            .get(id.ix())
            .filter(|e| e.refs > 0)
            .and_then(|e| self.heap.get(e.off.ix()..(e.off + e.len).ix()))
            .unwrap_or("")
    }

    /// Strings in use.
    pub fn len(&self) -> usize {
        self.used
    }

    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    /// Bytes held (live plus not yet compacted).
    pub fn bytes(&self) -> usize {
        self.heap.len()
    }

    fn mask(&self) -> usize {
        self.buckets.len() - 1
    }

    fn find(&self, s: &str, h: u32) -> Option<u32> {
        if self.buckets.is_empty() {
            return None;
        }
        let mut i = h.ix() & self.mask();
        loop {
            match self.buckets[i] {
                EMPTY => return None,
                idx => {
                    let e = self.entries[idx.ix()];
                    if e.hash == h && self.heap.get(e.off.ix()..(e.off + e.len).ix()) == Some(s) {
                        return Some(idx);
                    }
                }
            }
            i = (i + 1) & self.mask();
        }
    }

    /// Rebuild the table from the entries in use, sized for `extra` more.
    fn rebuild(&mut self, extra: usize) {
        let want = ((self.used + extra) * 2).next_power_of_two().max(16);
        let mut buckets = vec![EMPTY; want];
        let mask = want - 1;
        for (idx, e) in self.entries.iter().enumerate() {
            if e.refs > 0 {
                let mut i = e.hash.ix() & mask;
                while buckets[i] != EMPTY {
                    i = (i + 1) & mask;
                }
                buckets[i] = to_u32(idx);
            }
        }
        self.buckets = buckets;
    }

    /// Entries in the table: in use plus unreferenced.
    fn in_table(&self) -> usize {
        self.entries.len() - self.free.len()
    }

    /// Add a reference to `s`, storing it when new. `None` when the engine's text budget is full.
    pub fn intern(&mut self, s: &str) -> Option<StrId> {
        if s.is_empty() {
            return Some(NO_STR);
        }
        let h = hash(s);
        if let Some(idx) = self.find(s, h) {
            let e = &mut self.entries[idx.ix()];
            if e.refs == 0 {
                self.used += 1;
                self.dead_bytes -= e.len.ix();
            }
            e.refs += 1;
            return Some(idx);
        }
        if self.heap.len() - self.dead_bytes + s.len() > MAX_BYTES
            || self.heap.len() + s.len() >= u32::MAX.ix() / 2
        {
            self.compact();
            if self.heap.len() + s.len() > MAX_BYTES {
                return None;
            }
        }
        if (self.in_table() + 1) * 2 > self.buckets.len() {
            if self.dead_bytes > 0 {
                self.compact();
            } else {
                self.rebuild(1);
            }
        }
        let entry = Entry {
            off: to_u32(self.heap.len()),
            len: to_u32(s.len()),
            refs: 1,
            hash: h,
        };
        self.heap.push_str(s);
        let idx = match self.free.pop() {
            Some(i) => {
                self.entries[i.ix()] = entry;
                i
            }
            None => {
                self.entries.push(entry);
                to_u32(self.entries.len() - 1)
            }
        };
        let mut i = h.ix() & self.mask();
        while self.buckets[i] != EMPTY {
            i = (i + 1) & self.mask();
        }
        self.buckets[i] = idx;
        self.used += 1;
        Some(idx)
    }

    /// One more reference to a string already held.
    pub fn retain(&mut self, id: StrId) {
        if let Some(e) = self.entries.get_mut(id.ix()).filter(|e| e.refs > 0) {
            e.refs += 1;
        }
    }

    /// Drop one reference. The text stays cached until `compact`.
    pub fn release(&mut self, id: StrId) {
        let Some(e) = self.entries.get_mut(id.ix()).filter(|e| e.refs > 0) else {
            return;
        };
        e.refs -= 1;
        if e.refs == 0 {
            self.used -= 1;
            self.dead_bytes += e.len.ix();
            // Churn must not grow the heap without bound: drop the cache once it is most of it.
            if self.dead_bytes > 64 * 1024 && self.dead_bytes * 2 > self.heap.len() {
                self.compact();
            }
        }
    }

    /// Drop every unreferenced text and rewrite the heap without it.
    pub fn compact(&mut self) {
        if self.dead_bytes == 0 && self.free.len() + self.used == self.entries.len() {
            return;
        }
        let mut heap = String::with_capacity(self.heap.len() - self.dead_bytes);
        for (idx, e) in self.entries.iter_mut().enumerate() {
            if e.refs > 0 {
                let at = to_u32(heap.len());
                if let Some(s) = self.heap.get(e.off.ix()..(e.off + e.len).ix()) {
                    heap.push_str(s);
                }
                e.off = at;
            } else if e.off != FREED {
                e.off = FREED;
                e.len = 0;
                self.free.push(to_u32(idx));
            }
        }
        self.heap = heap;
        self.dead_bytes = 0;
        self.rebuild(0);
    }

    /// Drop spare capacity (Memory Saver).
    pub fn trim(&mut self) {
        self.compact();
        self.heap.shrink_to_fit();
        self.entries.shrink_to_fit();
        self.free.shrink_to_fit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_shares_and_frees() {
        let mut s = Strings::new();
        let a = s.intern("Delete").unwrap();
        let b = s.intern("Delete").unwrap();
        assert_eq!(a, b);
        assert_eq!(s.get(a), "Delete");
        s.release(a);
        assert_eq!(s.get(a), "Delete");
        s.release(b);
        assert_eq!(s.get(a), "");
        assert_eq!(s.len(), 0);
        // Unreferenced text is cached: the same text comes back as the same entry.
        assert_eq!(s.intern("Delete").unwrap(), a);
        s.release(a);
        s.compact();
        let c = s.intern("Other").unwrap();
        assert_eq!(c, a, "a freed entry is reused");
        assert_eq!(s.intern("").unwrap(), NO_STR);
        assert_eq!(s.get(NO_STR), "");
    }

    #[test]
    fn many_strings_survive_growth_and_compaction() {
        let mut s = Strings::new();
        let ids: Vec<_> = (0..500)
            .map(|i| s.intern(&format!("row {i}")).unwrap())
            .collect();
        for (i, id) in ids.iter().enumerate().filter(|(i, _)| i % 2 == 0) {
            let _ = i;
            s.release(*id);
        }
        s.compact();
        for (i, id) in ids.iter().enumerate().filter(|(i, _)| i % 2 == 1) {
            assert_eq!(s.get(*id), format!("row {i}"));
        }
        assert_eq!(s.intern("row 1").unwrap(), ids[1]);
    }
}
