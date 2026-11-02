// SPDX-License-Identifier: GPL-2.0-or-later
//! zsync delta downloads (AppImage updates): the `.zsync` control file lists a weak rolling sum and
//! a truncated MD4 per block of the new file. Blocks found anywhere in the old file are copied,
//! only the rest is fetched with `Range`, and the result must match the control file's SHA-1.
use crate::ffi::{Error, Result};
use md4::{Digest as _, Md4};
use sha1::Sha1;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Control {
    pub filename: String,
    pub blocksize: usize,
    pub length: u64,
    pub seq_matches: usize,
    pub rsum_bytes: usize,
    pub checksum_bytes: usize,
    /// Relative or absolute URL of the new file.
    pub url: String,
    pub sha1: String,
    /// `MTime:` of the new file as Unix seconds, when the control file has one.
    pub mtime: Option<i64>,
    /// (rsum, checksum) per block, truncated as stored.
    pub blocks: Vec<(u32, Vec<u8>)>,
}

/// zsync's rolling checksum over one block: `a` = sum of bytes, `b` = sum of (len - i) * byte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rsum {
    pub a: u16,
    pub b: u16,
}

impl Rsum {
    pub fn of(block: &[u8]) -> Rsum {
        let (mut a, mut b) = (0u16, 0u16);
        let n = block.len();
        for (i, &c) in block.iter().enumerate() {
            a = a.wrapping_add(c as u16);
            b = b.wrapping_add(((n - i) as u16).wrapping_mul(c as u16));
        }
        Rsum { a, b }
    }
    /// Slide the window by one byte: `out` leaves, `inn` enters.
    pub fn roll(&mut self, out: u8, inn: u8, bs: usize) {
        self.a = self.a.wrapping_sub(out as u16).wrapping_add(inn as u16);
        self.b = self
            .b
            .wrapping_sub((bs as u16).wrapping_mul(out as u16))
            .wrapping_add(self.a);
    }
    /// The stored form: big-endian `a`,`b`, last `n` bytes.
    pub fn truncated(self, n: usize) -> u32 {
        let full = ((self.a as u32) << 16) | self.b as u32;
        if n >= 4 {
            full
        } else {
            full & ((1u32 << (8 * n)) - 1)
        }
    }
}

fn md4(block: &[u8]) -> [u8; 16] {
    Md4::digest(block).into()
}

/// Largest file a control file may describe (1 TiB): keeps every offset product below `u64`.
pub const MAX_LENGTH: u64 = 1 << 40;

/// Parses a `.zsync` control file. Pure (no I/O): the fuzz entry point.
pub fn parse(bytes: &[u8]) -> Result<Control> {
    let bad = || Error::untrusted("The update description is damaged.");
    let end = memchr::memmem::find(bytes, b"\n\n").ok_or_else(bad)?;
    let head = std::str::from_utf8(&bytes[..end]).map_err(|_| bad())?;
    let get = |k: &str| {
        head.lines()
            .find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_owned()))
    };
    let blocksize: usize = get("Blocksize:")
        .and_then(|v| v.parse().ok())
        .filter(|b: &usize| b.is_power_of_two() && *b >= 256 && *b <= 1 << 20)
        .ok_or_else(bad)?;
    let length: u64 = get("Length:")
        .and_then(|v| v.parse().ok())
        .ok_or_else(bad)?;
    let hl: Vec<usize> = get("Hash-Lengths:")
        .unwrap_or("1,4,16".into())
        .split(',')
        .filter_map(|v| v.parse().ok())
        .collect();
    let [seq_matches, rsum_bytes, checksum_bytes] = hl[..] else {
        return Err(bad());
    };
    if !(1..=2).contains(&seq_matches)
        || !(1..=4).contains(&rsum_bytes)
        || !(3..=16).contains(&checksum_bytes)
    {
        return Err(bad());
    }
    let sha1 = get("SHA-1:")
        .filter(|s| s.len() == 40)
        .ok_or_else(bad)?
        .to_ascii_lowercase();
    if length > MAX_LENGTH {
        return Err(bad());
    }
    let bs = u64::try_from(blocksize).map_err(|_| bad())?;
    let n = usize::try_from(length.div_ceil(bs)).map_err(|_| bad())?;
    let table = bytes.get(end + 2..).ok_or_else(bad)?;
    let per = rsum_bytes + checksum_bytes;
    if table.len() < n.checked_mul(per).ok_or_else(bad)? {
        return Err(bad());
    }
    let blocks = (0..n)
        .map(|i| {
            let e = &table[i * per..(i + 1) * per];
            let r = e[..rsum_bytes]
                .iter()
                .fold(0u32, |acc, b| (acc << 8) | *b as u32);
            (r, e[rsum_bytes..].to_vec())
        })
        .collect();
    Ok(Control {
        filename: get("Filename:").unwrap_or_default(),
        blocksize,
        length,
        seq_matches,
        rsum_bytes,
        checksum_bytes,
        url: get("URL:").ok_or_else(bad)?,
        sha1,
        mtime: get("MTime:").as_deref().and_then(rfc2822),
        blocks,
    })
}

/// `Tue, 03 Nov 2026 10:00:00 +0000` (what zsyncmake writes) to Unix seconds.
pub fn rfc2822(s: &str) -> Option<i64> {
    let s = s.split_once(',').map_or(s, |(_, r)| r).trim();
    let mut it = s.split_ascii_whitespace();
    let day: i64 = it.next()?.parse().ok()?;
    let mon = it.next()?;
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| m.eq_ignore_ascii_case(mon))?;
    let month = i64::try_from(month).ok()? + 1;
    let year: i64 = it.next()?.parse().ok()?;
    let mut hms = it.next()?.split(':').map(|v| v.parse::<i64>().ok());
    let (h, m, sec) = (
        hms.next()??,
        hms.next()??,
        hms.next().flatten().unwrap_or(0),
    );
    let zone = it.next().unwrap_or("+0000");
    let (sign, z) = match zone.as_bytes().first() {
        Some(b'-') => (-1, &zone[1..]),
        Some(b'+') => (1, &zone[1..]),
        _ => (1, "0000"),
    };
    let z: i64 = z.parse().ok()?;
    if !(1..=31).contains(&day) || !(1970..=9999).contains(&year) || h > 23 || m > 59 || sec > 60 {
        return None;
    }
    // Days from civil (Howard Hinnant), all values small: no overflow.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let offset = sign * ((z / 100) * 3600 + (z % 100) * 60);
    Some(days * 86_400 + h * 3600 + m * 60 + sec - offset)
}

/// Write a `.zsync` control file for `data` (tests, and our own AppImage index).
pub fn make(data: &[u8], filename: &str, url: &str, blocksize: usize) -> Vec<u8> {
    let (rsum_bytes, checksum_bytes) = (4, 16);
    let mut out = format!(
        "zsync: 0.6.2\nFilename: {filename}\nBlocksize: {blocksize}\nLength: {}\nHash-Lengths: 1,{rsum_bytes},{checksum_bytes}\nURL: {url}\nSHA-1: {}\n\n",
        data.len(),
        crate::fetch::hex(&Sha1::digest(data))
    )
    .into_bytes();
    for chunk in data.chunks(blocksize) {
        let mut b = chunk.to_vec();
        b.resize(blocksize, 0);
        out.extend_from_slice(&Rsum::of(&b).truncated(4).to_be_bytes());
        out.extend_from_slice(&md4(&b)[..checksum_bytes]);
    }
    out
}

/// Which new-file blocks the old file already has: block index -> offset in `old`.
pub fn known_blocks(c: &Control, old: &[u8]) -> HashMap<usize, usize> {
    known_blocks_counting(c, old).0
}

/// [`known_blocks`] and how many strong (MD4) checks it ran. With short weak sums many positions
/// look like a block by chance; when the file says two blocks in a row must match (`seq_matches`
/// 2, what `zsyncmake` writes for big files) the next window has to look like the next block too,
/// which removes almost all of those chance hits before the strong check.
pub fn known_blocks_counting(c: &Control, old: &[u8]) -> (HashMap<usize, usize>, u64) {
    let bs = c.blocksize;
    let mut by_rsum: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, (r, _)) in c.blocks.iter().enumerate() {
        by_rsum.entry(*r).or_default().push(i);
    }
    let mut found: HashMap<usize, usize> = HashMap::new();
    let mut strong = 0u64;
    if old.len() < bs {
        return (found, strong);
    }
    let pair = c.seq_matches == 2;
    let mut pos = 0usize;
    let mut rs = Rsum::of(&old[..bs]);
    // The window one block ahead, kept in step with `rs` while it fits in the file.
    let mut ahead = (pos + 2 * bs <= old.len()).then(|| Rsum::of(&old[bs..2 * bs]));
    while pos + bs <= old.len() {
        let mut jumped = false;
        if let Some(cands) = by_rsum.get(&rs.truncated(c.rsum_bytes)) {
            let mut sum: Option<[u8; 16]> = None;
            for &i in cands {
                if pair
                    && let (Some(a), Some((next, _))) = (ahead, c.blocks.get(i + 1))
                    && a.truncated(c.rsum_bytes) != *next
                {
                    continue;
                }
                let s = *sum.get_or_insert_with(|| {
                    strong += 1;
                    md4(&old[pos..pos + bs])
                });
                if c.blocks[i].1[..] == s[..c.checksum_bytes] {
                    found.entry(i).or_insert(pos);
                    jumped = true;
                }
            }
        }
        if jumped && pos + 2 * bs <= old.len() {
            pos += bs;
            rs = Rsum::of(&old[pos..pos + bs]);
            ahead = (pos + 2 * bs <= old.len()).then(|| Rsum::of(&old[pos + bs..pos + 2 * bs]));
            continue;
        }
        if pos + bs >= old.len() {
            break;
        }
        rs.roll(old[pos], old[pos + bs], bs);
        ahead = match ahead {
            Some(mut a) if pos + 2 * bs < old.len() => {
                a.roll(old[pos + bs], old[pos + 2 * bs], bs);
                Some(a)
            }
            _ => None,
        };
        pos += 1;
    }
    (found, strong)
}

/// Contiguous ranges of blocks still missing, as byte ranges of the new file (inclusive end).
pub fn missing_ranges(c: &Control, known: &HashMap<usize, usize>) -> Vec<(u64, u64)> {
    let mut out: Vec<(u64, u64)> = Vec::new();
    let bs = c.blocksize as u64;
    for i in 0..c.blocks.len() {
        if known.contains_key(&i) {
            continue;
        }
        let (s, e) = (i as u64 * bs, ((i as u64 + 1) * bs).min(c.length) - 1);
        match out.last_mut() {
            Some(last) if last.1 + 1 == s => last.1 = e,
            _ => out.push((s, e)),
        }
    }
    out
}

/// Resolve the control file's URL against the URL it came from.
pub fn resolve(base: &str, rel: &str) -> String {
    if rel.contains("://") {
        return rel.to_owned();
    }
    match base.rfind('/') {
        Some(i) => format!("{}/{}", &base[..i], rel),
        None => rel.to_owned(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Stats {
    pub reused_bytes: u64,
    pub fetched_bytes: u64,
    pub ranges: usize,
}

/// Build the new file at `out` from `old` plus fetched ranges; check the SHA-1.
pub fn build(
    net: &crate::fetch::Net,
    c: &Control,
    file_url: &str,
    old: &Path,
    out: &Path,
    progress: &mut dyn FnMut(&serde_json::Value),
) -> Result<Stats> {
    net.check_url(file_url)?;
    let old_bytes = std::fs::read(old).unwrap_or_default();
    let known = known_blocks(c, &old_bytes);
    let ranges = missing_ranges(c, &known);
    let mut f = std::fs::File::create(out)
        .map_err(|_| Error::unavailable("The update cannot be written."))?;
    f.set_len(c.length)
        .map_err(|_| Error::unavailable("The disk is full."))?;
    let bs = c.blocksize;
    let mut reused = 0u64;
    let damaged = || Error::untrusted("The update description is damaged.");
    let length = usize::try_from(c.length).map_err(|_| damaged())?;
    for (&i, &at) in &known {
        let start = i.checked_mul(bs).ok_or_else(damaged)?;
        let n = bs.min(length.checked_sub(start).ok_or_else(damaged)?);
        let end = at
            .checked_add(n)
            .filter(|e| *e <= old_bytes.len())
            .ok_or_else(damaged)?;
        let pos = u64::try_from(start).map_err(|_| damaged())?;
        f.seek(SeekFrom::Start(pos))
            .and_then(|_| f.write_all(&old_bytes[at..end]))
            .map_err(|_| Error::unavailable("The update cannot be written."))?;
        reused = reused.saturating_add(u64::try_from(n).map_err(|_| damaged())?);
    }
    let mut fetched = 0u64;
    let total_missing: u64 = ranges
        .iter()
        .map(|(s, e)| e.saturating_sub(*s).saturating_add(1))
        .fold(0, u64::saturating_add);
    for (s, e) in &ranges {
        let resp = net
            .agent
            .get(file_url)
            .header("Range", &format!("bytes={s}-{e}"))
            .call()
            .map_err(|e| {
                Error::unavailable(format!("The update server could not be reached ({e})."))
            })?;
        let status = resp.status().as_u16();
        let mut body = resp.into_body().into_reader();
        let want = e
            .checked_sub(*s)
            .and_then(|d| d.checked_add(1))
            .filter(|_| *e < c.length)
            .ok_or_else(damaged)?;
        if status == 200 {
            // No Range support: skip to the slice we need in the whole body.
            std::io::copy(&mut body.by_ref().take(*s), &mut std::io::sink()).ok();
        } else if status != 206 {
            return Err(Error::unavailable(format!(
                "The update server answered {status}."
            )));
        }
        // Streamed straight to the file: a range can be most of a large AppImage.
        f.seek(SeekFrom::Start(*s))
            .map_err(|_| Error::unavailable("The update cannot be written."))?;
        let got = std::io::copy(&mut body.by_ref().take(want), &mut f)
            .map_err(|_| Error::unavailable("The update cannot be written."))?;
        if got != want {
            return Err(Error::unavailable(
                "The update download stopped. Try again.",
            ));
        }
        fetched = fetched.saturating_add(want);
        progress(
            &serde_json::json!({"event":"bytes","url":file_url,"done":fetched,"total":total_missing}),
        );
    }
    f.sync_all().ok();
    drop(f);
    let got = sha1_file(out).unwrap_or_default();
    if got != c.sha1 {
        let _ = std::fs::remove_file(out);
        return Err(Error::untrusted(
            "The updated file does not match its description, so it was deleted.",
        ));
    }
    Ok(Stats {
        reused_bytes: reused,
        fetched_bytes: fetched,
        ranges: ranges.len(),
    })
}

pub fn sha1_file(p: &Path) -> Option<String> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(crate::fetch::hex(&h.finalize()))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::testutil::{Served, serve, temp};

    pub fn noise(n: usize, seed: u32) -> Vec<u8> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    #[test]
    fn crafted_control_files_are_refused_not_overflowed() {
        let head = |len: &str, bs: &str| {
            format!(
                "zsync: 0.6.2
Filename: x
Blocksize: {bs}
Length: {len}
Hash-Lengths: 2,4,16
URL: x
SHA-1: {}

",
                "0".repeat(40)
            )
            .into_bytes()
        };
        assert!(parse(&head(&u64::MAX.to_string(), "256")).is_err());
        assert!(parse(&head(&(MAX_LENGTH + 1).to_string(), "256")).is_err());
        // A huge length with no table behind it.
        assert!(parse(&head(&MAX_LENGTH.to_string(), "256")).is_err());
        assert!(parse(&head("100", "255")).is_err());
        assert!(
            parse(
                b"

"
            )
            .is_err()
        );
        assert!(parse(b"").is_err());
        let mut ok = head("256", "256");
        ok.extend_from_slice(&[0; 20]);
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn mtime_parses() {
        assert_eq!(rfc2822("Thu, 01 Jan 1970 00:00:00 +0000"), Some(0));
        assert_eq!(
            rfc2822("Mon, 02 Nov 2026 01:00:00 +0100"),
            Some(1_793_577_600)
        );
        assert_eq!(rfc2822("garbage"), None);
    }

    #[test]
    fn rolling_sum_matches_direct() {
        let d = noise(5000, 7);
        let bs = 1024;
        let mut r = Rsum::of(&d[..bs]);
        for p in 0..2000 {
            assert_eq!(r, Rsum::of(&d[p..p + bs]), "at {p}");
            r.roll(d[p], d[p + bs], bs);
        }
    }

    #[test]
    fn control_roundtrip_and_block_reuse() {
        let old = noise(64 * 1024, 1);
        let mut new = old.clone();
        new.splice(1000..1000, noise(300, 9)); // an insertion shifts everything after it
        new[40_000..40_100].copy_from_slice(&noise(100, 3));
        let z = make(&new, "app.AppImage", "app.AppImage", 1024);
        let c = parse(&z).unwrap();
        assert_eq!(
            (c.length as usize, c.blocks.len(), c.url.as_str()),
            (new.len(), 65, "app.AppImage")
        );
        let known = known_blocks(&c, &old);
        assert!(known.len() >= 60, "most blocks reused: {}", known.len());
        let miss = missing_ranges(&c, &known);
        assert!(miss.len() <= 4, "{miss:?}");
        assert!(parse(b"zsync: 1\nBlocksize: 3\n\n").is_err());
    }

    #[test]
    fn delta_build_fetches_only_changed_ranges() {
        let old = noise(200 * 1024, 11);
        let mut new = old.clone();
        new[100_000..100_010].copy_from_slice(b"0123456789");
        new.extend_from_slice(&noise(5000, 5));
        let srv = serve(vec![("/app.AppImage", Served::bytes(new.clone()))]);
        let d = temp("zsync");
        std::fs::write(d.join("old"), &old).unwrap();
        let c = parse(&make(&new, "app", "app.AppImage", 2048)).unwrap();
        let net = crate::fetch::Net::new(true);
        let url = resolve(&srv.url("/app.AppImage.zsync"), &c.url);
        assert_eq!(url, srv.url("/app.AppImage"));
        let s = build(&net, &c, &url, &d.join("old"), &d.join("new"), &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(d.join("new")).unwrap(), new);
        assert!(s.fetched_bytes < 12 * 1024, "{s:?}");
        let mut wrong = c.clone();
        wrong.sha1 = "0".repeat(40);
        assert!(
            build(
                &net,
                &wrong,
                &url,
                &d.join("old"),
                &d.join("bad"),
                &mut |_| {}
            )
            .is_err()
        );
        assert!(!d.join("bad").exists());
    }

    /// A control file as `zsyncmake` writes it for large files: `Hash-Lengths: 2,2,5`.
    fn compact(data: &[u8], bs: usize) -> Control {
        let mut c = parse(&make(data, "a", "a", bs)).unwrap();
        c.seq_matches = 2;
        c.rsum_bytes = 2;
        c.checksum_bytes = 5;
        for (r, s) in &mut c.blocks {
            *r &= 0xffff;
            s.truncate(5);
        }
        c
    }

    #[test]
    fn short_weak_sums_do_not_make_the_scan_crawl() {
        // With 2-byte sums about one position in 16 looks like a block; checking every one with the
        // strong sum took 38 s on a 49 MB AppImage. Two matching blocks in a row are required.
        let old = noise(8 << 20, 21);
        let mut new = old.clone();
        new[3_000_000..3_000_010].copy_from_slice(b"0123456789");
        let c = compact(&new, 2048);
        let (known, strong) = known_blocks_counting(&c, &old);
        assert!(strong < 20_000, "strong checks: {strong}");
        // Every block except those around the edit is found, none wrongly.
        assert!(
            known.len() >= c.blocks.len() - 4,
            "{} of {}",
            known.len(),
            c.blocks.len()
        );
        for (&i, &at) in &known {
            let s = i * 2048;
            let n = 2048.min(new.len() - s);
            assert_eq!(old[at..at + n], new[s..s + n]);
        }
    }
}
