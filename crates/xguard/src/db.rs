// SPDX-License-Identifier: GPL-2.0-or-later
//! The signature database file (`*.xgdb`): sorted hash tables, rule names and a compiled pattern
//! set in one memory-mapped file. Nothing is parsed or copied at open: a process pays only for
//! the pages it touches, and every process shares them.
use crate::automaton::{self, Hit, Pattern, Set};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

const MAGIC: &[u8; 8] = b"XGDB0001";
const SECTIONS: usize = 10;
const HEADER: usize = 32 + SECTIONS * 16;

/// Hash tables. Each record: file or section size, 8 bytes of the hash, rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Table {
    Md5 = 0,
    Sha256 = 1,
    SectionMd5 = 2,
    SectionSha256 = 3,
    /// Known good: a whole-file match overrides every finding.
    AllowMd5 = 4,
    AllowSha256 = 5,
}
const TABLES: usize = 6;
const REC: usize = 16;

/// First 8 bytes of a digest: with the size, plenty to tell files apart.
pub fn key(digest: &[u8]) -> u64 {
    u64::from_le_bytes(digest[..8].try_into().expect("8 bytes"))
}

/// Collects signatures and writes the file.
#[derive(Default)]
pub struct Builder {
    names: BTreeMap<String, u32>,
    tables: [Vec<(u32, u64, u32)>; TABLES],
    patterns: Vec<Pattern>,
    /// rule id -> (name, parts).
    rules: Vec<(u32, u8)>,
}

impl Builder {
    fn name(&mut self, name: &str) -> u32 {
        let next = self.names.len() as u32;
        *self.names.entry(name.to_owned()).or_insert(next)
    }

    pub fn hash(&mut self, table: Table, size: u32, digest: &[u8], name: &str) {
        let n = self.name(name);
        self.tables[table as usize].push((size, key(digest), n));
    }

    /// A rule: all `parts` must match. False (nothing added) when one cannot be indexed.
    pub fn rule(&mut self, name: &str, parts: Vec<Pattern>) -> bool {
        if parts.is_empty() || parts.len() > 64 || !parts.iter().all(automaton::indexable) {
            return false;
        }
        let id = self.rules.len() as u32;
        let n = self.name(name);
        self.rules.push((n, parts.len() as u8));
        for (i, mut p) in parts.into_iter().enumerate() {
            p.id = id;
            p.part = i as u8;
            self.patterns.push(p);
        }
        true
    }

    /// The file's bytes. Same input, same output.
    pub fn build(mut self, version: u32, built: u64) -> Vec<u8> {
        let mut order = vec![0u32; self.names.len()];
        let mut blob = Vec::new();
        let mut index = vec![0u32; self.names.len() + 1];
        for (rank, (name, id)) in self.names.iter().enumerate() {
            order[*id as usize] = rank as u32;
            blob.extend(name.as_bytes());
            index[rank + 1] = blob.len() as u32;
        }
        let mut sections: Vec<Vec<u8>> = Vec::with_capacity(SECTIONS);
        sections.push(index.iter().flat_map(|v| v.to_le_bytes()).collect());
        sections.push(blob);
        for t in &mut self.tables {
            t.sort_unstable();
            t.dedup_by_key(|r| (r.0, r.1));
            let mut s = Vec::with_capacity(t.len() * REC);
            for (size, h, n) in t.iter() {
                s.extend(size.to_le_bytes());
                s.extend(order[*n as usize].to_le_bytes());
                s.extend(h.to_le_bytes());
            }
            sections.push(s);
        }
        sections.push(
            self.rules
                .iter()
                .flat_map(|(n, parts)| {
                    let mut r = order[*n as usize].to_le_bytes().to_vec();
                    r.extend([*parts, 0, 0, 0]);
                    r
                })
                .collect(),
        );
        sections.push(automaton::compile(&self.patterns));
        let mut out = vec![0u8; HEADER];
        out[..8].copy_from_slice(MAGIC);
        out[8..12].copy_from_slice(&version.to_le_bytes());
        out[12..16].copy_from_slice(&(self.patterns.len() as u32).to_le_bytes());
        out[16..24].copy_from_slice(&built.to_le_bytes());
        for (i, s) in sections.iter().enumerate() {
            while !out.len().is_multiple_of(8) {
                out.push(0);
            }
            let at = 32 + i * 16;
            let start = out.len() as u64;
            out[at..at + 8].copy_from_slice(&start.to_le_bytes());
            out[at + 8..at + 16].copy_from_slice(&(s.len() as u64).to_le_bytes());
            out.extend(s);
        }
        out
    }
}

/// Bytes of the file: a shared mapping, or owned (tests).
enum Bytes {
    Map(memmap2::Mmap),
    Own(Vec<u8>),
}

impl std::ops::Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Bytes::Map(m) => m,
            Bytes::Own(v) => v,
        }
    }
}

/// An open database.
pub struct Db {
    b: Bytes,
    sec: [(usize, usize); SECTIONS],
}

impl Db {
    pub fn from_bytes(v: Vec<u8>) -> Result<Self, String> {
        Self::check(Bytes::Own(v))
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        // SAFETY: the file lives in a root-owned folder and is replaced by rename, never rewritten.
        let m = unsafe { memmap2::Mmap::map(&f) }.map_err(|e| e.to_string())?;
        Self::check(Bytes::Map(m))
    }

    fn check(b: Bytes) -> Result<Self, String> {
        if b.len() < HEADER || &b[..8] != MAGIC {
            return Err("not a signature database".into());
        }
        let mut sec = [(0, 0); SECTIONS];
        for (i, s) in sec.iter_mut().enumerate() {
            let g = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"));
            let (o, n) = (g(32 + i * 16) as usize, g(40 + i * 16) as usize);
            if o.checked_add(n).is_none_or(|e| e > b.len()) {
                return Err("damaged signature database".into());
            }
            *s = (o, n);
        }
        let d = Self { b, sec };
        if d.section(0).len() < 4
            || !d.section(0).len().is_multiple_of(4)
            || (2..8).any(|t| !d.section(t).len().is_multiple_of(REC))
            || !d.section(8).len().is_multiple_of(8)
            || Set::new(d.section(9)).is_none()
        {
            return Err("damaged signature database".into());
        }
        Ok(d)
    }

    fn section(&self, i: usize) -> &[u8] {
        let (o, n) = self.sec[i];
        &self.b[o..o + n]
    }

    pub fn version(&self) -> u32 {
        u32::from_le_bytes(self.b[8..12].try_into().expect("4 bytes"))
    }
    /// Seconds since 1970 when it was built.
    pub fn built(&self) -> u64 {
        u64::from_le_bytes(self.b[16..24].try_into().expect("8 bytes"))
    }
    /// (hash records, rules).
    pub fn counts(&self) -> (u64, u64) {
        let h: usize = (2..8).map(|t| self.section(t).len() / REC).sum();
        (h as u64, (self.section(8).len() / 8) as u64)
    }

    fn name(&self, n: u32) -> String {
        let (ix, blob) = (self.section(0), self.section(1));
        let at = |i: usize| {
            u32::from_le_bytes(ix[i * 4..i * 4 + 4].try_into().expect("4 bytes")) as usize
        };
        let n = n as usize;
        String::from_utf8_lossy(&blob[at(n)..at(n + 1)]).into_owned()
    }

    fn rec(t: &[u8], i: usize) -> (u32, u32, u64) {
        let r = &t[i * REC..(i + 1) * REC];
        (
            u32::from_le_bytes(r[..4].try_into().expect("4 bytes")),
            u32::from_le_bytes(r[4..8].try_into().expect("4 bytes")),
            key(&r[8..]),
        )
    }

    /// True when the table holds a record of this size (hash only when it does).
    pub fn has_size(&self, t: Table, size: u32) -> bool {
        let s = self.section(2 + t as usize);
        let n = s.len() / REC;
        let (mut lo, mut hi) = (0, n);
        while lo < hi {
            let m = (lo + hi) / 2;
            if Self::rec(s, m).0 < size {
                lo = m + 1;
            } else {
                hi = m;
            }
        }
        lo < n && Self::rec(s, lo).0 == size
    }

    /// The rule of a size + digest record.
    pub fn lookup(&self, t: Table, size: u32, digest: &[u8]) -> Option<String> {
        let s = self.section(2 + t as usize);
        let want = (size, key(digest));
        let (mut lo, mut hi) = (0, s.len() / REC);
        while lo < hi {
            let m = (lo + hi) / 2;
            let (sz, n, h) = Self::rec(s, m);
            match (sz, h).cmp(&want) {
                std::cmp::Ordering::Less => lo = m + 1,
                std::cmp::Ordering::Greater => hi = m,
                std::cmp::Ordering::Equal => return Some(self.name(n)),
            }
        }
        None
    }

    /// Names of the rules that match `data`, at most `max`.
    pub fn patterns(&self, data: &[u8], kind: u32, max: usize) -> Vec<String> {
        let Some(set) = Set::new(self.section(9)) else {
            return Vec::new();
        };
        let rules = self.section(8);
        let mut seen: HashMap<u32, u64> = HashMap::new();
        let mut done = Vec::new();
        set.scan(data, kind, &mut |h: Hit| {
            let i = h.id as usize * 8;
            let (n, parts) = (
                u32::from_le_bytes(rules[i..i + 4].try_into().expect("4 bytes")),
                rules[i + 4],
            );
            let got = seen.entry(h.id).or_default();
            if *got == u64::MAX {
                return true;
            }
            *got |= 1 << h.part;
            if got.count_ones() == u32::from(parts) {
                *got = u64::MAX;
                done.push(n);
            }
            done.len() < max
        });
        done.into_iter().map(|n| self.name(n)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automaton::{Place, Tok};

    fn lit(s: &[u8]) -> Pattern {
        Pattern {
            toks: vec![Tok::Lit(s.to_vec())],
            place: Place::Anywhere,
            kinds: 0,
            id: 0,
            part: 0,
        }
    }

    fn sample() -> Db {
        let mut b = Builder::default();
        b.hash(Table::Md5, 68, &[1; 16], "Test.One");
        b.hash(Table::Md5, 68, &[2; 16], "Test.Two");
        b.hash(Table::Sha256, 9, &[3; 32], "Test.One");
        assert!(b.rule("Test.Lone", vec![lit(b"lone-marker-here")]));
        assert!(b.rule(
            "Test.Pair",
            vec![lit(b"first-half-of-it"), lit(b"second-half-of-it")]
        ));
        assert!(!b.rule("Test.Weak", vec![lit(b"tiny")]));
        Db::from_bytes(b.build(42, 1_700_000_000)).unwrap()
    }

    #[test]
    fn records_and_metadata_round_trip() {
        let d = sample();
        assert_eq!((d.version(), d.built()), (42, 1_700_000_000));
        assert_eq!(d.counts(), (3, 2));
        assert!(d.has_size(Table::Md5, 68));
        assert!(!d.has_size(Table::Md5, 69));
        assert!(!d.has_size(Table::SectionMd5, 68));
        assert_eq!(
            d.lookup(Table::Md5, 68, &[2; 16]).as_deref(),
            Some("Test.Two")
        );
        assert_eq!(d.lookup(Table::Md5, 68, &[9; 16]), None);
        assert_eq!(d.lookup(Table::Md5, 67, &[2; 16]), None);
        assert_eq!(
            d.lookup(Table::Sha256, 9, &[3; 32]).as_deref(),
            Some("Test.One")
        );
    }

    #[test]
    fn a_rule_of_several_parts_needs_them_all() {
        let d = sample();
        assert_eq!(d.patterns(b"xx lone-marker-here", 1, 8), ["Test.Lone"]);
        assert!(d.patterns(b"first-half-of-it", 1, 8).is_empty());
        assert_eq!(
            d.patterns(b"second-half-of-it first-half-of-it", 1, 8),
            ["Test.Pair"]
        );
        assert_eq!(
            d.patterns(b"lone-marker-here lone-marker-here", 1, 8).len(),
            1,
            "once per rule"
        );
    }

    #[test]
    fn damage_is_refused_not_trusted() {
        assert!(Db::from_bytes(vec![0; 10]).is_err());
        let mut v = Builder::default().build(1, 1);
        assert!(Db::from_bytes(v.clone()).is_ok());
        v[0] = b'Y';
        assert!(Db::from_bytes(v).is_err());
        let mut v = Builder::default().build(1, 1);
        v[40..48].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(Db::from_bytes(v).is_err());
    }

    #[test]
    fn the_same_input_gives_the_same_file() {
        let make = || {
            let mut b = Builder::default();
            b.hash(Table::Md5, 1, &[1; 16], "B");
            b.hash(Table::Md5, 2, &[1; 16], "A");
            b.build(1, 1)
        };
        assert_eq!(make(), make());
    }
}
