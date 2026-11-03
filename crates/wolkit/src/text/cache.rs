// SPDX-License-Identifier: BSD-2-Clause
//! The shaped-run LRU.
//!
//! Keyed by (text hash, font, size, features, direction, script). A hit is one hash of the text,
//! one map probe, a text compare (so a 64-bit hash collision can never return wrong glyphs) and
//! an `Arc` clone; no allocation. Entries live in a slab threaded by a doubly linked recency list,
//! so touching and evicting are O(1).

use crate::num::round_u32;
use crate::scene::ids::{Ix, to_u32};
use std::collections::HashMap;
use std::sync::Arc;

use harfrust::Script;

use crate::text::fontdb::{FontDb, FontId};
use crate::text::shape::{Dir, FontFeature, ShapeParams, ShapedRun, shape_run};

/// FNV-1a, 64 bit. Not DoS-resistant, which is fine: the cache verifies the text on a hit and an
/// attacker who controls text can only slow their own window.
pub fn hash_bytes(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Key {
    text_hash: u64,
    font: FontId,
    size_q: u32,
    features: u64,
    dir: Dir,
    script: u32,
}

struct Entry {
    key: Key,
    text: Box<str>,
    run: Arc<ShapedRun>,
    prev: u32,
    next: u32,
}

const NIL: u32 = u32::MAX;

/// Hit and miss counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 {
        let n = self.hits + self.misses;
        if n == 0 {
            1.0
        } else {
            self.hits as f64 / n as f64
        }
    }
}

pub struct ShapeCache {
    map: HashMap<Key, u32>,
    slab: Vec<Entry>,
    head: u32, // most recent
    tail: u32, // least recent
    cap: usize,
    pub stats: CacheStats,
}

fn size_q(size: f32) -> u32 {
    round_u32(size.max(0.0) * 64.0)
}

fn features_hash(f: &[FontFeature]) -> u64 {
    let mut h = FNV_OFFSET;
    for x in f {
        h = hash_bytes(h, &x.tag);
        h = hash_bytes(h, &x.value.to_le_bytes());
    }
    h
}

impl ShapeCache {
    /// A cache holding at most `capacity` runs (at least 1).
    pub fn new(capacity: usize) -> ShapeCache {
        let cap = capacity.max(1);
        ShapeCache {
            map: HashMap::with_capacity(cap),
            slab: Vec::with_capacity(cap),
            head: NIL,
            tail: NIL,
            cap,
            stats: CacheStats::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.slab.clear();
        self.head = NIL;
        self.tail = NIL;
    }

    fn unlink(&mut self, i: u32) {
        let (p, n) = (self.slab[i.ix()].prev, self.slab[i.ix()].next);
        if p != NIL {
            self.slab[p.ix()].next = n;
        } else {
            self.head = n;
        }
        if n != NIL {
            self.slab[n.ix()].prev = p;
        } else {
            self.tail = p;
        }
    }

    fn push_front(&mut self, i: u32) {
        self.slab[i.ix()].prev = NIL;
        self.slab[i.ix()].next = self.head;
        if self.head != NIL {
            self.slab[self.head.ix()].prev = i;
        }
        self.head = i;
        if self.tail == NIL {
            self.tail = i;
        }
    }

    /// The shaped run for `text`, shaping it on a miss.
    pub fn get(&mut self, db: &FontDb, text: &str, p: &ShapeParams<'_>) -> Arc<ShapedRun> {
        let key = Key {
            text_hash: hash_bytes(FNV_OFFSET, text.as_bytes()),
            font: p.font,
            size_q: size_q(p.size),
            features: features_hash(p.features),
            dir: p.dir,
            script: script_id(p.script),
        };
        if let Some(&i) = self.map.get(&key)
            && &*self.slab[i.ix()].text == text
        {
            self.stats.hits += 1;
            if self.head != i {
                self.unlink(i);
                self.push_front(i);
            }
            return Arc::clone(&self.slab[i.ix()].run);
        }
        self.stats.misses += 1;
        let run = Arc::new(shape_run(db, text, p));
        self.insert(key, text, Arc::clone(&run));
        run
    }

    fn insert(&mut self, key: Key, text: &str, run: Arc<ShapedRun>) {
        // A collision on the same key with different text replaces the old entry.
        if let Some(&old) = self.map.get(&key) {
            self.unlink(old);
            self.slab[old.ix()].text = text.into();
            self.slab[old.ix()].run = run;
            self.push_front(old);
            return;
        }
        let entry = Entry {
            key,
            text: text.into(),
            run,
            prev: NIL,
            next: NIL,
        };
        let i = if self.slab.len() < self.cap {
            self.slab.push(entry);
            to_u32(self.slab.len() - 1)
        } else {
            let victim = self.tail;
            self.unlink(victim);
            let old_key = self.slab[victim.ix()].key;
            self.map.remove(&old_key);
            self.stats.evictions += 1;
            self.slab[victim.ix()] = entry;
            victim
        };
        self.map.insert(key, i);
        self.push_front(i);
    }
}

fn script_id(s: Script) -> u32 {
    u32::from_be_bytes(s.tag().into_bytes())
}
