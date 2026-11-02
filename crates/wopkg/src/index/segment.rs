// SPDX-License-Identifier: GPL-2.0-or-later
//! Segment file: `WOPKGSG1`, record count, input fingerprint, TSV records, lowercase haystack
//! (one line per record), then record and haystack offsets (u32 LE). Written to `<path>.tmp` and
//! renamed, so a reader sees the old or the new segment, never half of one.
use super::Record;
use crate::ffi::{Error, Result};
use std::io::Write;
use std::path::Path;

const MAGIC: &[u8; 8] = b"WOPKGSG1";

pub fn write(path: &Path, fingerprint: &str, records: &[Record]) -> Result<()> {
    let mut recs = Vec::new();
    let mut hay = Vec::new();
    let (mut ro, mut ho) = (
        Vec::with_capacity(records.len()),
        Vec::with_capacity(records.len()),
    );
    for r in records {
        ro.push(recs.len() as u32);
        ho.push(hay.len() as u32);
        recs.extend_from_slice(r.to_line().as_bytes());
        hay.extend_from_slice(r.haystack().replace('\n', " ").as_bytes());
        hay.push(b'\n');
    }
    if recs.len() > u32::MAX as usize || hay.len() > u32::MAX as usize {
        return Err(Error::invalid("A source is too large to index."));
    }
    let mut out = Vec::with_capacity(recs.len() + hay.len() + 64 + 8 * records.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(records.len() as u32).to_le_bytes());
    out.extend_from_slice(&(fingerprint.len() as u32).to_le_bytes());
    out.extend_from_slice(fingerprint.as_bytes());
    out.extend_from_slice(&(recs.len() as u64).to_le_bytes());
    out.extend_from_slice(&recs);
    out.extend_from_slice(&(hay.len() as u64).to_le_bytes());
    out.extend_from_slice(&hay);
    for o in ro.iter().chain(ho.iter()) {
        out.extend_from_slice(&o.to_le_bytes());
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)
            .map_err(|_| Error::unavailable("The index folder cannot be created."))?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp)
        .map_err(|_| Error::unavailable("The index cannot be written."))?;
    f.write_all(&out)
        .and_then(|_| f.sync_all())
        .map_err(|_| Error::unavailable("The index cannot be written."))?;
    std::fs::rename(&tmp, path).map_err(|_| Error::unavailable("The index cannot be written."))
}

/// A mapped segment.
pub struct Segment {
    map: memmap2::Mmap,
    n: usize,
    fp: (usize, usize),
    recs: (usize, usize),
    hay: (usize, usize),
    offs: usize,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        b.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

/// Where the parts of a segment file are. Every range is inside the file.
#[derive(Debug, PartialEq, Eq)]
pub struct Layout {
    pub n: usize,
    pub fp: (usize, usize),
    pub recs: (usize, usize),
    pub hay: (usize, usize),
    pub offs: usize,
}

/// Checks a segment file's header and sizes with checked arithmetic. Pure: the fuzz entry point.
pub fn layout(b: &[u8]) -> Option<Layout> {
    if b.get(..8)? != MAGIC {
        return None;
    }
    let n = usize::try_from(u32_at(b, 8)?).ok()?;
    let fl = usize::try_from(u32_at(b, 12)?).ok()?;
    let fp = (16, 16usize.checked_add(fl)?);
    let rl = usize::try_from(u64_at(b, fp.1)?).ok()?;
    let r0 = fp.1.checked_add(8)?;
    let recs = (r0, r0.checked_add(rl)?);
    let hl = usize::try_from(u64_at(b, recs.1)?).ok()?;
    let h0 = recs.1.checked_add(8)?;
    let hay = (h0, h0.checked_add(hl)?);
    let offs = hay.1;
    if b.len() != offs.checked_add(n.checked_mul(8)?)? {
        return None;
    }
    Some(Layout {
        n,
        fp,
        recs,
        hay,
        offs,
    })
}

impl Segment {
    pub fn open(path: &Path) -> Option<Segment> {
        let f = std::fs::File::open(path).ok()?;
        // SAFETY: segments are replaced by rename, never written in place, so the mapping stays valid.
        let map = unsafe { memmap2::Mmap::map(&f) }.ok()?;
        let l = layout(&map[..])?;
        Some(Segment {
            map,
            n: l.n,
            fp: l.fp,
            recs: l.recs,
            hay: l.hay,
            offs: l.offs,
        })
    }

    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    pub fn fingerprint(&self) -> &str {
        self.map
            .get(self.fp.0..self.fp.1)
            .and_then(|b| std::str::from_utf8(b).ok())
            .unwrap_or("")
    }

    pub fn haystack(&self) -> &[u8] {
        self.map.get(self.hay.0..self.hay.1).unwrap_or_default()
    }

    fn off(&self, table: usize, i: usize) -> usize {
        // `table` is 0 or 1 and `i` < n, so this stays below offs + 8n (checked in `layout`).
        let at = table
            .checked_mul(self.n)
            .and_then(|t| t.checked_add(i))
            .and_then(|k| k.checked_mul(4))
            .and_then(|k| k.checked_add(self.offs));
        at.and_then(|at| u32_at(&self.map, at))
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(0)
    }

    /// Record index whose haystack line holds byte `pos`.
    pub fn record_at(&self, pos: usize) -> usize {
        let (mut lo, mut hi) = (0, self.n);
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            if self.off(1, mid) <= pos {
                lo = mid
            } else {
                hi = mid
            }
        }
        lo
    }

    /// Haystack line of record `i` (without the newline).
    pub fn hay_line(&self, i: usize) -> &[u8] {
        let h = self.haystack();
        let s = self.off(1, i);
        let e = if i + 1 < self.n {
            self.off(1, i + 1)
        } else {
            h.len()
        };
        h.get(s..e.saturating_sub(1).max(s)).unwrap_or_default()
    }

    pub fn record(&self, i: usize) -> Option<Record> {
        let r = self.map.get(self.recs.0..self.recs.1)?;
        let s = self.off(0, i);
        let e = if i + 1 < self.n {
            self.off(0, i + 1)
        } else {
            r.len()
        };
        Record::from_line(std::str::from_utf8(r.get(s..e)?).ok()?)
    }

    /// TSV name field of record `i`, without decoding the rest (completions).
    pub fn name(&self, i: usize) -> &str {
        let Some(line) = self
            .map
            .get(self.recs.0..self.recs.1)
            .and_then(|r| r.get(self.off(0, i)..))
        else {
            return "";
        };
        let mut f = line.splitn(4, |b| *b == b'\t');
        f.next();
        f.next();
        f.next()
            .and_then(|n| std::str::from_utf8(n).ok())
            .unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Kind, Record};
    use super::*;

    fn rec(name: &str) -> Record {
        Record {
            kind: Kind::Pacman,
            repo: "extra".into(),
            name: name.into(),
            id: name.into(),
            version: "1-1".into(),
            summary: format!("The {name} tool"),
            key: name.into(),
            votes: 0,
        }
    }

    #[test]
    fn crafted_headers_are_refused() {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(&u32::MAX.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&u64::MAX.to_le_bytes());
        assert!(layout(&b).is_none());
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&(u64::MAX - 4).to_le_bytes());
        assert!(layout(&b).is_none());
        assert!(layout(b"").is_none());
        // fuzz/regressions/wopkg_parsers/segment-offset-overflow: a record length that puts
        // the next field at usize::MAX - 3.
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&(u64::MAX - 27).to_le_bytes());
        assert!(layout(&b).is_none());
        assert!(u32_at(&b, usize::MAX - 1).is_none());
        assert!(u64_at(&b, usize::MAX - 3).is_none());
        let fuzz = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fuzz/regressions/wopkg_parsers/segment-offset-overflow");
        if let Ok(bytes) = std::fs::read(fuzz) {
            let _ = layout(&bytes);
        }
    }

    #[test]
    fn write_then_map() {
        let d = crate::testutil::temp("seg");
        let p = d.join("extra.seg");
        write(&p, "fp-1", &[rec("alpha"), rec("Beta")]).unwrap();
        let s = Segment::open(&p).unwrap();
        assert_eq!((s.len(), s.fingerprint()), (2, "fp-1"));
        assert_eq!(s.record(1).unwrap().name, "Beta");
        assert_eq!(s.name(0), "alpha");
        assert_eq!(s.hay_line(1), b"beta beta the beta tool");
        let pos = memchr::memmem::find(s.haystack(), b"beta").unwrap();
        assert_eq!(s.record_at(pos), 1);
        std::fs::write(&p, b"WOPKGSG1junk").unwrap();
        assert!(Segment::open(&p).is_none(), "a damaged segment is ignored");
    }
}
