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
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

impl Segment {
    pub fn open(path: &Path) -> Option<Segment> {
        let f = std::fs::File::open(path).ok()?;
        // SAFETY: segments are replaced by rename, never written in place, so the mapping stays valid.
        let map = unsafe { memmap2::Mmap::map(&f) }.ok()?;
        let b = &map[..];
        if b.get(..8)? != MAGIC {
            return None;
        }
        let n = u32_at(b, 8)? as usize;
        let fl = u32_at(b, 12)? as usize;
        let fp = (16, 16 + fl);
        let rl = u64_at(b, fp.1)? as usize;
        let recs = (fp.1 + 8, fp.1 + 8 + rl);
        let hl = u64_at(b, recs.1)? as usize;
        let hay = (recs.1 + 8, recs.1 + 8 + hl);
        let offs = hay.1;
        if b.len() != offs.checked_add(n.checked_mul(8)?)? {
            return None;
        }
        Some(Segment {
            map,
            n,
            fp,
            recs,
            hay,
            offs,
        })
    }

    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    pub fn fingerprint(&self) -> &str {
        std::str::from_utf8(&self.map[self.fp.0..self.fp.1]).unwrap_or("")
    }

    pub fn haystack(&self) -> &[u8] {
        &self.map[self.hay.0..self.hay.1]
    }

    fn off(&self, table: usize, i: usize) -> usize {
        u32_at(&self.map, self.offs + table * self.n * 4 + i * 4).unwrap_or(0) as usize
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
        &h[s..e.saturating_sub(1).max(s)]
    }

    pub fn record(&self, i: usize) -> Option<Record> {
        let r = &self.map[self.recs.0..self.recs.1];
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
        let r = &self.map[self.recs.0..self.recs.1];
        let s = self.off(0, i);
        let line = &r[s..];
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
