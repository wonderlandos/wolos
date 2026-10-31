// SPDX-License-Identifier: GPL-2.0-or-later
//! What a scan does to bytes, and the stable result it returns.
use crate::db::{Db, Table};
use md5::{Digest as _, Md5};
use sha2::Sha256;

/// File kinds a signature can be limited to (bit values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Kind {
    Other = 1,
    Pe = 2,
    Elf = 4,
    MachO = 8,
    Pdf = 16,
    Ole2 = 32,
    Graphics = 64,
    Swf = 128,
    Java = 256,
}

/// Kind by the file's first bytes.
pub fn kind(d: &[u8]) -> Kind {
    let has = |m: &[u8]| d.starts_with(m);
    if has(b"MZ") {
        Kind::Pe
    } else if has(b"\x7fELF") {
        Kind::Elf
    } else if has(b"%PDF-") {
        Kind::Pdf
    } else if has(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
        Kind::Ole2
    } else if has(&[0xfe, 0xed, 0xfa, 0xce])
        || has(&[0xfe, 0xed, 0xfa, 0xcf])
        || has(&[0xce, 0xfa, 0xed, 0xfe])
        || has(&[0xcf, 0xfa, 0xed, 0xfe])
    {
        Kind::MachO
    } else if has(&[0xca, 0xfe, 0xba, 0xbe]) {
        // Also fat Mach-O; the class-file version fields tell them apart.
        if d.len() > 8 && u32::from_be_bytes([d[4], d[5], d[6], d[7]]) < 45 {
            Kind::MachO
        } else {
            Kind::Java
        }
    } else if has(b"FWS") || has(b"CWS") || has(b"ZWS") {
        Kind::Swf
    } else if has(b"\x89PNG") || has(b"GIF8") || has(&[0xff, 0xd8, 0xff]) {
        Kind::Graphics
    } else {
        Kind::Other
    }
}

/// `(raw offset, raw size)` of a PE's sections; empty when `d` is not a sound PE.
pub fn pe_sections(d: &[u8]) -> Vec<(usize, usize)> {
    let u16_at = |a: usize| d.get(a..a + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |a: usize| {
        d.get(a..a + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let parse = || -> Option<Vec<(usize, usize)>> {
        let pe = u32_at(0x3c)?;
        if d.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
            return None;
        }
        let count = usize::from(u16_at(pe + 6)?).min(96);
        let table = pe + 24 + usize::from(u16_at(pe + 20)?);
        (0..count)
            .filter_map(|i| {
                let e = table + i * 40;
                Some((u32_at(e + 20)?, u32_at(e + 16)?))
            })
            .filter(|(o, n)| *n > 0 && o.checked_add(*n).is_some_and(|end| end <= d.len()))
            .map(Some)
            .collect()
    };
    parse().unwrap_or_default()
}

/// One thing found.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Finding {
    /// Signature family or rule name.
    pub name: String,
    /// `hash`, `pattern` or `rule`.
    pub how: String,
    /// The file, or the path inside the archive (`app.zip/dir/file`).
    pub path: String,
}

/// Something that was not read, and why.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Skipped {
    pub path: String,
    pub why: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// Nothing known was found in everything that was read.
    Clean,
    Found,
    /// Nothing known was found, but part of the input was not read (`skipped` says what).
    Incomplete,
}

/// The stable scan result (`archicture/modules/security.md`). New fields are only added.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Report {
    pub verdict: Verdict,
    pub findings: Vec<Finding>,
    pub skipped: Vec<Skipped>,
    pub files: u64,
    pub bytes: u64,
    /// Whole-input SHA-256 in hex; empty for a scan of bytes with no file.
    pub sha256: String,
    /// Newest signature build, seconds since 1970; 0 with no database.
    pub signatures_built: u64,
    /// Age of the newest signatures in days; None with no database.
    pub signatures_age_days: Option<u64>,
    /// Signature database versions, in load order.
    pub signatures_versions: Vec<u32>,
}

/// Extra rule engine run on every file (YARA-X in the worker).
pub trait Deep {
    /// Names of the rules that match.
    fn matches(&self, data: &[u8]) -> Vec<String>;
}

/// Open databases.
#[derive(Default)]
pub struct Engine {
    dbs: Vec<Db>,
}

/// Findings per file: one is enough to act on.
const MAX_FINDINGS: usize = 8;

impl Engine {
    pub fn new(dbs: Vec<Db>) -> Self {
        Self { dbs }
    }

    /// Every `*.xgdb` in `dir`, in name order. An unreadable one is an error: the caller must not
    /// report clean on a partial set.
    pub fn open(dir: &std::path::Path) -> Result<Self, String> {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "xgdb"))
            .collect();
        files.sort();
        let dbs = files
            .iter()
            .map(|p| Db::open(p).map_err(|e| format!("{}: {e}", p.display())))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { dbs })
    }

    pub fn built(&self) -> u64 {
        self.dbs.iter().map(Db::built).max().unwrap_or(0)
    }

    pub fn versions(&self) -> Vec<u32> {
        self.dbs.iter().map(Db::version).collect()
    }

    /// (hash records, rules) over all databases.
    pub fn counts(&self) -> (u64, u64) {
        self.dbs
            .iter()
            .map(Db::counts)
            .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
    }

    /// A report with no results yet, stamped with the databases' age at `now`.
    pub fn blank(&self, now: u64) -> Report {
        let built = self.built();
        Report {
            verdict: Verdict::Clean,
            findings: Vec::new(),
            skipped: Vec::new(),
            files: 0,
            bytes: 0,
            sha256: String::new(),
            signatures_built: built,
            signatures_age_days: (built != 0).then(|| now.saturating_sub(built) / 86_400),
            signatures_versions: self.versions(),
        }
    }

    fn table_hit(&self, t: Table, size: u32, digest: &[u8]) -> Option<String> {
        self.dbs.iter().find_map(|d| d.lookup(t, size, digest))
    }

    fn sized(&self, t: Table, size: u32) -> bool {
        self.dbs.iter().any(|d| d.has_size(t, size))
    }

    /// Findings for one file's bytes, already hashed by `sha256`. Hashes are computed only when
    /// a signature of that size exists, so most files cost one table probe.
    pub fn findings(&self, data: &[u8], sha256: Option<&[u8]>, path: &str) -> Vec<Finding> {
        let Ok(size) = u32::try_from(data.len()) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut add = |name: String, how: &str| {
            out.push(Finding {
                name,
                how: how.into(),
                path: path.into(),
            })
        };
        let lazy_md5 = std::cell::OnceCell::new();
        let md5 = || *lazy_md5.get_or_init(|| <[u8; 16]>::from(Md5::digest(data)));
        let lazy_sha = std::cell::OnceCell::new();
        let sha = || {
            *lazy_sha.get_or_init(|| match sha256 {
                Some(s) => <[u8; 32]>::try_from(s).unwrap_or_default(),
                None => <[u8; 32]>::from(Sha256::digest(data)),
            })
        };
        if self.sized(Table::AllowMd5, size)
            && self.table_hit(Table::AllowMd5, size, &md5()).is_some()
            || self.sized(Table::AllowSha256, size)
                && self.table_hit(Table::AllowSha256, size, &sha()).is_some()
        {
            return Vec::new();
        }
        if self.sized(Table::Md5, size)
            && let Some(n) = self.table_hit(Table::Md5, size, &md5())
        {
            add(n, "hash");
        }
        if self.sized(Table::Sha256, size)
            && let Some(n) = self.table_hit(Table::Sha256, size, &sha())
        {
            add(n, "hash");
        }
        let k = kind(data);
        if k == Kind::Pe {
            for (o, n) in pe_sections(data) {
                let (s, n32) = (&data[o..o + n], n as u32);
                if self.sized(Table::SectionMd5, n32)
                    && let Some(name) =
                        self.table_hit(Table::SectionMd5, n32, &<[u8; 16]>::from(Md5::digest(s)))
                {
                    add(name, "hash");
                }
                if self.sized(Table::SectionSha256, n32)
                    && let Some(name) = self.table_hit(
                        Table::SectionSha256,
                        n32,
                        &<[u8; 32]>::from(Sha256::digest(s)),
                    )
                {
                    add(name, "hash");
                }
            }
        }
        for d in &self.dbs {
            for n in d.patterns(data, k as u32, MAX_FINDINGS) {
                add(n, "pattern");
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out.dedup();
        out.truncate(MAX_FINDINGS);
        out
    }
}

/// Hex of a digest.
pub fn hexs(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Seconds since 1970.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::automaton::{Pattern, Place, Tok};
    use crate::db::Builder;

    pub(crate) const EICAR: &[u8] =
        b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

    /// A database that knows the standard test file by hash and by pattern.
    pub(crate) fn test_db() -> Db {
        let mut b = Builder::default();
        b.hash(
            Table::Sha256,
            EICAR.len() as u32,
            &Sha256::digest(EICAR),
            "Test.File",
        );
        assert!(b.rule(
            "Test.Pattern",
            vec![Pattern {
                toks: vec![Tok::Lit(b"EICAR-STANDARD-ANTIVIRUS-TEST-FILE".to_vec())],
                place: Place::Anywhere,
                kinds: 0,
                id: 0,
                part: 0,
            }]
        ));
        Db::from_bytes(b.build(7, 1_000_000)).unwrap()
    }

    #[test]
    fn kinds_by_magic_table() {
        for (d, k) in [
            (&b"MZ\x90"[..], Kind::Pe),
            (b"\x7fELF\x02", Kind::Elf),
            (b"%PDF-1.7", Kind::Pdf),
            (b"\xca\xfe\xba\xbe\0\0\0\x34xx", Kind::Java),
            (b"\xca\xfe\xba\xbe\0\0\0\x02xx", Kind::MachO),
            (b"#!/bin/sh", Kind::Other),
            (b"", Kind::Other),
        ] {
            assert_eq!(kind(d), k, "{d:?}");
        }
    }

    #[test]
    fn the_test_file_is_found_by_hash_and_pattern() {
        let e = Engine::new(vec![test_db()]);
        let f = e.findings(EICAR, None, "x");
        let names: Vec<&str> = f.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Test.File", "Test.Pattern"]);
        assert_eq!(f[0].how, "hash");
        let mut padded = b"padding ".to_vec();
        padded.extend(EICAR);
        let f = e.findings(&padded, None, "x");
        assert_eq!(f.len(), 1, "another size: the pattern only");
        assert!(e.findings(b"an ordinary file", None, "x").is_empty());
    }

    #[test]
    fn known_good_overrides_a_finding() {
        let mut b = Builder::default();
        b.hash(
            Table::AllowSha256,
            EICAR.len() as u32,
            &Sha256::digest(EICAR),
            "fp",
        );
        let e = Engine::new(vec![test_db(), Db::from_bytes(b.build(1, 1)).unwrap()]);
        assert!(e.findings(EICAR, None, "x").is_empty());
    }

    #[test]
    fn pe_sections_are_hashed_only_when_a_size_matches() {
        let mut pe = vec![0u8; 0x400];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3c] = 0x80;
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        pe[0x86] = 2; // sections
        pe[0x94] = 0x10; // optional header
        let t = 0x80 + 24 + 0x10;
        pe[t + 16] = 0x10; // raw size
        pe[t + 21] = 0x02; // raw offset 0x200
        pe[t + 40 + 16] = 0xff; // runs past the file end: dropped
        pe[t + 40 + 17] = 0xff;
        pe[0x200..0x210].copy_from_slice(b"section-contents");
        assert_eq!(pe_sections(&pe), [(0x200, 0x10)]);
        let mut b = Builder::default();
        b.hash(
            Table::SectionMd5,
            16,
            &Md5::digest(b"section-contents"),
            "Test.Section",
        );
        let e = Engine::new(vec![Db::from_bytes(b.build(1, 1)).unwrap()]);
        let f = e.findings(&pe, None, "x");
        assert_eq!(f[0].name, "Test.Section");
        assert!(pe_sections(b"MZ").is_empty());
        let mut lying = pe.clone();
        lying[0x3c] = 0xff;
        lying[0x3d] = 0xff;
        lying[0x3e] = 0xff;
        lying[0x3f] = 0xff;
        assert!(pe_sections(&lying).is_empty());
    }

    #[test]
    fn report_says_how_old_the_signatures_are() {
        let e = Engine::new(vec![test_db()]);
        let r = e.blank(1_000_000 + 3 * 86_400 + 5);
        assert_eq!(r.signatures_age_days, Some(3));
        assert_eq!(r.signatures_versions, [7]);
        assert_eq!(Engine::default().blank(9).signatures_age_days, None);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["verdict"], "clean");
    }
}
