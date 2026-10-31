// SPDX-License-Identifier: GPL-2.0-or-later
//! Scans a file and everything packed inside it, in memory: nothing is unpacked to disk.
//! Containers: gzip, zstd, xz, zip, tar, squashfs (AppImages, also behind their ELF runtime).
//! Limits keep a hostile archive from using more than a bounded amount of memory and time;
//! what falls outside them is listed in the report, and the verdict says `incomplete`.
use crate::scan::{Deep, Engine, Finding, Report, Skipped, Verdict, hexs, now};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Containers inside containers.
    pub depth: u32,
    /// Largest unpacked member held in memory.
    pub member: u64,
    /// All unpacked bytes together.
    pub total: u64,
    pub files: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            depth: 5,
            member: 512 << 20,
            total: 8 << 30,
            files: 500_000,
        }
    }
}

/// Reads at most `max` bytes of `r`; None past that, or on a damaged stream, with `why`.
fn slurp(mut r: impl Read, max: u64) -> Result<Vec<u8>, &'static str> {
    let mut v = Vec::new();
    match (&mut r).take(max + 1).read_to_end(&mut v) {
        Ok(_) if v.len() as u64 > max => Err("too large to unpack"),
        Ok(_) => Ok(v),
        Err(_) => Err("damaged"),
    }
}

/// A memory sink that fails past its limit: the only brake the xz decoder offers.
struct Capped(Vec<u8>, usize);

impl std::io::Write for Capped {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.0.len() + b.len() > self.1 {
            return Err(std::io::ErrorKind::OutOfMemory.into());
        }
        self.0.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Walk<'a> {
    e: &'a Engine,
    deep: Option<&'a dyn Deep>,
    lim: Limits,
    unpacked: u64,
    r: Report,
}

const FINDINGS_MAX: usize = 32;

impl Walk<'_> {
    fn skip(&mut self, path: &str, why: &str) {
        self.r.skipped.push(Skipped {
            path: path.into(),
            why: why.into(),
        });
    }

    fn full(&self) -> bool {
        self.r.findings.len() >= FINDINGS_MAX
    }

    /// A member's bytes -> scanned, then opened if it is a container.
    fn member(&mut self, path: &str, depth: u32, r: impl Read) {
        let room = self
            .lim
            .member
            .min(self.lim.total.saturating_sub(self.unpacked));
        match slurp(r, room) {
            Ok(d) => {
                self.unpacked += d.len() as u64;
                self.file(&d, path, depth);
            }
            Err(why) => self.skip(path, why),
        }
    }

    fn file(&mut self, d: &[u8], path: &str, depth: u32) {
        if self.full() {
            return;
        }
        if self.r.files >= self.lim.files {
            self.skip(path, "too many files");
            return;
        }
        self.r.files += 1;
        self.r.bytes += d.len() as u64;
        // A tar's members are read one by one: reading the whole first would only repeat it.
        if sniff(d) != Some(Packed::Tar) {
            self.r.findings.extend(self.e.findings(d, None, path));
            if let Some(deep) = self.deep {
                self.r
                    .findings
                    .extend(deep.matches(d).into_iter().map(|name| Finding {
                        name,
                        how: "rule".into(),
                        path: path.into(),
                    }));
            }
        }
        self.open(d, path, depth);
    }

    fn open(&mut self, d: &[u8], path: &str, depth: u32) {
        let packed = sniff(d);
        if packed.is_none() {
            return;
        }
        if depth >= self.lim.depth {
            self.skip(path, "packed too deep");
            return;
        }
        let depth = depth + 1;
        match packed {
            Some(Packed::Gzip) => self.stream(path, depth, flate2::read::MultiGzDecoder::new(d)),
            Some(Packed::Zstd) => match ruzstd::decoding::StreamingDecoder::new(d) {
                Ok(z) => self.stream(path, depth, z),
                Err(_) => self.skip(path, "damaged"),
            },
            Some(Packed::Xz) => {
                let mut out = Capped(Vec::new(), self.lim.member as usize);
                let mut input = std::io::BufReader::new(d);
                match lzma_rs::xz_decompress(&mut input, &mut out) {
                    Ok(()) => self.stream(path, depth, Cursor::new(out.0)),
                    Err(_) => self.skip(path, "damaged or too large to unpack"),
                }
            }
            Some(Packed::Tar) => self.tar(d, path, depth),
            Some(Packed::Zip) => self.zip(d, path, depth),
            Some(Packed::Squash(at)) => self.squash(d, at, path, depth),
            None => {}
        }
    }

    /// One compressed stream: its content is one member named after the file.
    fn stream(&mut self, path: &str, depth: u32, r: impl Read) {
        self.member(&format!("{path}!"), depth, r);
    }

    fn tar(&mut self, d: &[u8], path: &str, depth: u32) {
        let mut at = 0;
        let mut long: Option<String> = None;
        while let Some(h) = d.get(at..at + 512) {
            if h.iter().all(|b| *b == 0) {
                return;
            }
            let field = |r: std::ops::Range<usize>| {
                let f = &h[r];
                let n = f.iter().position(|b| *b == 0).unwrap_or(f.len());
                String::from_utf8_lossy(&f[..n]).into_owned()
            };
            let Some(size) = octal(&h[124..136]) else {
                self.skip(path, "damaged");
                return;
            };
            let (body, next) = (at + 512, at + 512 + size.div_ceil(512) * 512);
            let Some(content) = d.get(body..body.saturating_add(size)) else {
                self.skip(path, "damaged");
                return;
            };
            let prefix = field(345..500);
            let name = long.take().unwrap_or_else(|| match prefix.as_str() {
                "" => field(0..100),
                p => format!("{p}/{}", field(0..100)),
            });
            match h[156] {
                b'L' => {
                    let n = content.iter().position(|b| *b == 0).unwrap_or(size);
                    long = Some(String::from_utf8_lossy(&content[..n]).into_owned());
                }
                b'0' | 0 | b'7' => {
                    self.unpacked += size as u64;
                    self.file(content, &format!("{path}/{name}"), depth);
                }
                _ => {}
            }
            at = next;
        }
    }

    /// From the central directory. Stored and deflated members; anything else is listed as unread.
    fn zip(&mut self, d: &[u8], path: &str, depth: u32) {
        let Some((mut at, count)) = zip_directory(d) else {
            self.skip(path, "damaged");
            return;
        };
        for i in 0..count {
            let Some(e) = zip_entry(d, at) else {
                self.skip(path, "damaged");
                return;
            };
            at = e.next;
            let name = format!("{path}/{}", e.name);
            if e.name.ends_with('/') {
                continue;
            }
            let Some(body) = e.body(d) else {
                self.skip(&format!("{path}/#{i}"), "damaged");
                continue;
            };
            match (e.flags & 1, e.method) {
                (0, 0) => self.member(&name, depth, body),
                (0, 8) => self.member(&name, depth, flate2::read::DeflateDecoder::new(body)),
                (1, _) => self.skip(&name, "locked with a password"),
                _ => self.skip(&name, "unknown packing"),
            }
        }
    }

    fn squash(&mut self, d: &[u8], at: u64, path: &str, depth: u32) {
        use backhand::InnerNode;
        let cur = std::io::BufReader::new(Cursor::new(d));
        let Ok(fs) = backhand::FilesystemReader::from_reader_with_offset(cur, at) else {
            self.skip(path, "damaged");
            return;
        };
        for node in fs.files() {
            if let InnerNode::File(f) = &node.inner {
                let name = format!("{path}{}", node.fullpath.display());
                let content = fs.file(f);
                self.member(&name, depth, content.reader());
            }
        }
    }
}

/// One central-directory record.
struct ZipEntry {
    name: String,
    flags: u16,
    method: u16,
    packed: usize,
    local: usize,
    next: usize,
}

impl ZipEntry {
    /// The packed bytes, from the local header.
    fn body<'a>(&self, d: &'a [u8]) -> Option<&'a [u8]> {
        let h = d.get(self.local..self.local.checked_add(30)?)?;
        if &h[..4] != b"PK\x03\x04" {
            return None;
        }
        let n = usize::from(u16::from_le_bytes([h[26], h[27]]));
        let x = usize::from(u16::from_le_bytes([h[28], h[29]]));
        let start = self.local.checked_add(30 + n + x)?;
        d.get(start..start.checked_add(self.packed)?)
    }
}

/// (directory offset, entries) from the end record, searched back over any trailing comment.
fn zip_directory(d: &[u8]) -> Option<(usize, usize)> {
    let tail = d.len().saturating_sub(65_557);
    let at = memchr::memmem::rfind(&d[tail..], b"PK\x05\x06")? + tail;
    let e = d.get(at..at + 22)?;
    let count = usize::from(u16::from_le_bytes([e[10], e[11]]));
    Some((
        u32::from_le_bytes([e[16], e[17], e[18], e[19]]) as usize,
        count,
    ))
}

fn zip_entry(d: &[u8], at: usize) -> Option<ZipEntry> {
    let h = d.get(at..at.checked_add(46)?)?;
    if &h[..4] != b"PK\x01\x02" {
        return None;
    }
    let u16_at = |i: usize| u16::from_le_bytes([h[i], h[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]) as usize;
    let (n, x, c) = (
        usize::from(u16_at(28)),
        usize::from(u16_at(30)),
        usize::from(u16_at(32)),
    );
    Some(ZipEntry {
        name: String::from_utf8_lossy(d.get(at + 46..at + 46 + n)?).into_owned(),
        flags: u16_at(8),
        method: u16_at(10),
        packed: u32_at(20),
        local: u32_at(42),
        next: at + 46 + n + x + c,
    })
}

fn octal(f: &[u8]) -> Option<usize> {
    let s = std::str::from_utf8(f).ok()?;
    let s = s.trim_matches(|c: char| c == '\0' || c == ' ');
    if s.is_empty() {
        return Some(0);
    }
    usize::from_str_radix(s, 8).ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Packed {
    Gzip,
    Zstd,
    Xz,
    Tar,
    Zip,
    /// Squashfs at this offset.
    Squash(u64),
}

fn sniff(d: &[u8]) -> Option<Packed> {
    let has = |m: &[u8]| d.starts_with(m);
    if has(&[0x1f, 0x8b]) {
        Some(Packed::Gzip)
    } else if has(&[0x28, 0xb5, 0x2f, 0xfd]) {
        Some(Packed::Zstd)
    } else if has(&[0xfd, b'7', b'z', b'X', b'Z', 0]) {
        Some(Packed::Xz)
    } else if has(b"PK\x03\x04") || has(b"PK\x05\x06") {
        Some(Packed::Zip)
    } else if d.get(257..262) == Some(b"ustar") {
        Some(Packed::Tar)
    } else if has(b"hsqs") {
        Some(Packed::Squash(0))
    } else if has(b"\x7fELF") {
        elf_end(d)
            .filter(|e| d.get(*e..*e + 4) == Some(b"hsqs"))
            .map(|e| Packed::Squash(e as u64))
    } else {
        None
    }
}

/// End of an ELF-64 image: section headers come last. An AppImage's filesystem starts there.
fn elf_end(d: &[u8]) -> Option<usize> {
    if d.get(4) != Some(&2) || d.get(5) != Some(&1) {
        return None;
    }
    let shoff = u64::from_le_bytes(d.get(40..48)?.try_into().ok()?) as usize;
    let size = usize::from(u16::from_le_bytes(d.get(58..60)?.try_into().ok()?));
    let num = usize::from(u16::from_le_bytes(d.get(60..62)?.try_into().ok()?));
    shoff.checked_add(size.checked_mul(num)?)
}

/// One input, whole.
pub fn scan_bytes(
    e: &Engine,
    d: &[u8],
    name: &str,
    deep: Option<&dyn Deep>,
    lim: Limits,
) -> Report {
    let mut w = Walk {
        e,
        deep,
        lim,
        unpacked: 0,
        r: e.blank(now()),
    };
    w.r.sha256 = hexs(&Sha256::digest(d));
    w.file(d, name, 0);
    let mut r = w.r;
    r.verdict = if !r.findings.is_empty() {
        Verdict::Found
    } else if !r.skipped.is_empty() {
        Verdict::Incomplete
    } else {
        Verdict::Clean
    };
    r
}

/// Files above this are read in overlapping pieces, with no look inside.
pub const WHOLE_MAX: u64 = 2 << 30;
const PIECE: usize = 256 << 20;
const OVERLAP: usize = 1 << 20;

/// A file on disk. Read with plain reads (a mapping would fault if the file shrank meanwhile).
pub fn scan_file(
    e: &Engine,
    f: &mut std::fs::File,
    name: &str,
    deep: Option<&dyn Deep>,
    lim: Limits,
) -> std::io::Result<Report> {
    let size = f.metadata()?.len();
    if size <= WHOLE_MAX {
        let mut d = Vec::with_capacity(size as usize);
        f.read_to_end(&mut d)?;
        return Ok(scan_bytes(e, &d, name, deep, lim));
    }
    let mut r = e.blank(now());
    let (mut buf, mut keep) = (vec![0u8; PIECE], 0);
    let mut sha = Sha256::new();
    loop {
        let n = read_full(f, &mut buf[keep..])?;
        if n == 0 {
            break;
        }
        sha.update(&buf[keep..keep + n]);
        r.bytes += n as u64;
        r.findings.extend(e.findings(&buf[..keep + n], None, name));
        if r.findings.len() >= FINDINGS_MAX {
            break;
        }
        let end = keep + n;
        keep = OVERLAP.min(end);
        buf.copy_within(end - keep..end, 0);
    }
    r.files = 1;
    r.sha256 = hexs(&sha.finalize());
    r.skipped.push(Skipped {
        path: name.into(),
        why: "too large to look inside".into(),
    });
    r.verdict = if r.findings.is_empty() {
        Verdict::Incomplete
    } else {
        Verdict::Found
    };
    Ok(r)
}

fn read_full(f: &mut std::fs::File, mut b: &mut [u8]) -> std::io::Result<usize> {
    let want = b.len();
    while !b.is_empty() {
        match f.read(b)? {
            0 => break,
            n => b = &mut b[n..],
        }
    }
    Ok(want - b.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::tests::{EICAR, test_db};
    use std::io::Write;

    fn engine() -> Engine {
        Engine::new(vec![test_db()])
    }
    /// A real filesystem image holding one file. None: mksquashfs is not on this computer.
    fn squashfs(comp: &str, d: &[u8]) -> Option<Vec<u8>> {
        let dir = std::env::temp_dir().join(format!("xg-sq-{}-{comp}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("in/usr/bin")).unwrap();
        std::fs::write(dir.join("in/usr/bin/tool"), d).unwrap();
        let img = dir.join("img");
        let made = std::process::Command::new("mksquashfs")
            .arg(dir.join("in"))
            .arg(&img)
            .args(["-comp", comp, "-noappend", "-quiet", "-no-progress"])
            .stdout(std::process::Stdio::null())
            .status()
            .ok()?;
        assert!(made.success(), "{comp}");
        let out = std::fs::read(&img).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        Some(out)
    }
    /// An ELF-64 header whose section headers end where the filesystem starts.
    fn behind_runtime(fs: &[u8]) -> Vec<u8> {
        let mut d = vec![0u8; 8192];
        d[..6].copy_from_slice(b"\x7fELF\x02\x01");
        d[40..48].copy_from_slice(&4096u64.to_le_bytes());
        d[58..60].copy_from_slice(&64u16.to_le_bytes());
        d[60..62].copy_from_slice(&64u16.to_le_bytes());
        d.extend_from_slice(fs);
        d
    }
    fn scan(d: &[u8]) -> Report {
        scan_bytes(&engine(), d, "in", None, Limits::default())
    }
    fn found(r: &Report) -> Vec<String> {
        r.findings.iter().map(|f| f.path.clone()).collect()
    }
    #[test]
    fn appimages_are_read_inside_for_each_compression() {
        for comp in ["gzip", "zstd", "xz"] {
            let Some(fs) = squashfs(comp, EICAR) else {
                eprintln!("mksquashfs missing: not tested here");
                return;
            };
            let inside = |r: Report| r.findings.iter().any(|f| f.path.ends_with("usr/bin/tool"));
            assert!(inside(scan(&fs)), "{comp}: bare image");
            assert!(inside(scan(&behind_runtime(&fs))), "{comp}: as an AppImage");
            let clean = squashfs(comp, b"nothing to see").unwrap();
            assert!(
                scan(&behind_runtime(&clean)).findings.is_empty(),
                "{comp}: clean"
            );
        }
    }

    fn gz(d: &[u8]) -> Vec<u8> {
        let mut w = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        w.write_all(d).unwrap();
        w.finish().unwrap()
    }
    /// A one-file zip with a deflated member (CRC left zero: readers here do not need it).
    fn zipped(name: &str, d: &[u8]) -> Vec<u8> {
        let mut w = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        w.write_all(d).unwrap();
        let packed = w.finish().unwrap();
        let l16 = |v: usize| (v as u16).to_le_bytes();
        let l32 = |v: usize| (v as u32).to_le_bytes();
        let mut z = b"PK\x03\x04\x14\0\0\0\x08\0\0\0\0\0\0\0\0\0".to_vec();
        z.extend(l32(packed.len()));
        z.extend(l32(d.len()));
        z.extend(l16(name.len()));
        z.extend(l16(0));
        z.extend(name.as_bytes());
        z.extend(&packed);
        let dir = z.len();
        z.extend(b"PK\x01\x02\x14\0\x14\0\0\0\x08\0\0\0\0\0\0\0\0\0");
        z.extend(l32(packed.len()));
        z.extend(l32(d.len()));
        z.extend(l16(name.len()));
        z.extend([0u8; 12]);
        z.extend(l32(0));
        z.extend(name.as_bytes());
        let size = z.len() - dir;
        z.extend(b"PK\x05\x06\0\0\0\0\x01\0\x01\0");
        z.extend(l32(size));
        z.extend(l32(dir));
        z.extend(l16(0));
        z
    }

    #[test]
    fn plain_clean_and_found() {
        let r = scan(b"nothing to see");
        assert_eq!((r.verdict, r.files), (Verdict::Clean, 1));
        assert_eq!(r.sha256.len(), 64);
        let r = scan(EICAR);
        assert_eq!(r.verdict, Verdict::Found);
    }

    #[test]
    fn found_inside_every_container_without_touching_disk() {
        let tar = wol_core::package::ustar(&[("a.txt", b"fine"), ("bad.com", EICAR)]);
        let tgz = gz(&tar);
        for (d, want) in [
            (gz(EICAR), "in!"),
            (zipped("dir/bad.com", EICAR), "in/dir/bad.com"),
            (tar.clone(), "in/bad.com"),
            (tgz.clone(), "in!/bad.com"),
            (zipped("inner.tgz", &tgz), "in/inner.tgz!/bad.com"),
        ] {
            let r = scan(&d);
            assert_eq!(r.verdict, Verdict::Found, "{want}");
            assert!(
                found(&r).contains(&want.to_string()),
                "{want}: {:?}",
                found(&r)
            );
        }
    }

    #[test]
    fn a_wol_bundle_is_read_down_to_its_package() {
        // .wol: ustar with a pacman package (tar in zstd or gzip) inside.
        let pkg = gz(&wol_core::package::ustar(&[("usr/bin/app", EICAR)]));
        let wol = wol_core::package::ustar(&[("Info.json", b"{}"), ("payload.pkg", &pkg)]);
        assert_eq!(found(&scan(&wol))[0], "in/payload.pkg!/usr/bin/app");
    }

    #[test]
    fn a_nested_bomb_stops_and_says_so() {
        let mut d = EICAR.to_vec();
        for _ in 0..4 {
            d = gz(&d);
        }
        let lim = Limits {
            depth: 2,
            ..Limits::default()
        };
        let r = scan_bytes(&engine(), &d, "in", None, lim);
        assert_eq!(r.verdict, Verdict::Incomplete);
        assert_eq!(r.skipped[0].why, "packed too deep");
        // A small file that unpacks to more than the limit.
        let bomb = gz(&vec![0u8; 8 << 20]);
        let lim = Limits {
            member: 1 << 20,
            ..Limits::default()
        };
        let r = scan_bytes(&engine(), &bomb, "in", None, lim);
        assert_eq!(r.verdict, Verdict::Incomplete);
        assert_eq!(r.skipped[0].why, "too large to unpack");
    }

    #[test]
    fn damaged_archives_are_incomplete_not_clean() {
        let mut d = gz(b"some content that compresses");
        d.truncate(d.len() / 2);
        assert_eq!(scan(&d).verdict, Verdict::Incomplete);
        assert_eq!(
            scan(b"PK\x03\x04 not really a zip").verdict,
            Verdict::Incomplete
        );
    }

    #[test]
    fn deep_rules_run_on_every_member() {
        struct D;
        impl Deep for D {
            fn matches(&self, data: &[u8]) -> Vec<String> {
                if data.windows(5).any(|w| w == b"hello") {
                    vec!["Rule.Hello".into()]
                } else {
                    Vec::new()
                }
            }
        }
        let r = scan_bytes(
            &engine(),
            &gz(b"oh hello"),
            "in",
            Some(&D),
            Limits::default(),
        );
        assert_eq!(r.findings[0].how, "rule");
        assert_eq!(r.findings[0].path, "in!");
    }

    #[test]
    fn a_large_file_is_read_in_pieces() {
        // The boundary logic with a file just over the piece size is covered by the real file path.
        let p = std::env::temp_dir().join(format!("xguard-big-{}", std::process::id()));
        std::fs::write(&p, EICAR).unwrap();
        let r = scan_file(
            &engine(),
            &mut std::fs::File::open(&p).unwrap(),
            "in",
            None,
            Limits::default(),
        )
        .unwrap();
        std::fs::remove_file(&p).ok();
        assert_eq!(r.verdict, Verdict::Found);
    }
}
