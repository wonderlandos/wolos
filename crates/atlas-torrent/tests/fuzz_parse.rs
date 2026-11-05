// SPDX-License-Identifier: BSD-2-Clause
//! Seeded random-mutation fuzzing of what strangers hand the helper (DL-SEC-19): `.torrent` files
//! and BEP 9 info dictionaries ([`metainfo::check`], [`metainfo::rebuild`]) and magnet links
//! ([`magnet::parse`]). No panic, and what passes keeps the promises the engine relies on: every
//! path component confined, bounded file count, the rebuilt file has the same info and at most 32
//! allowed trackers, a magnet has one 40-hex id and only allowed trackers.
//!
//! `cargo test -p atlas-torrent --test fuzz_parse` runs 2 s; `ATLAS_TORRENT_FUZZ_SECS=1800` runs
//! the lead's 30 minutes (`ATLAS_TORRENT_FUZZ_SEED=<n>` replays a run).
use atlas_torrent::{confine, magnet, metainfo, trackers};
use std::time::{Duration, Instant};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

fn bstr(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(s.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(s);
}

fn one_file(name: &str, len: u64, trackers: &[&str]) -> Vec<u8> {
    let n = len.div_ceil(16384).max(1) as usize;
    let mut v = b"d".to_vec();
    if let Some(first) = trackers.first() {
        bstr(&mut v, b"announce");
        bstr(&mut v, first.as_bytes());
        bstr(&mut v, b"announce-list");
        v.push(b'l');
        for t in trackers {
            v.push(b'l');
            bstr(&mut v, t.as_bytes());
            v.push(b'e');
        }
        v.push(b'e');
    }
    v.extend_from_slice(b"4:infod6:lengthi");
    v.extend_from_slice(len.to_string().as_bytes());
    v.push(b'e');
    bstr(&mut v, b"name");
    bstr(&mut v, name.as_bytes());
    v.extend_from_slice(b"12:piece lengthi16384e6:pieces");
    bstr(&mut v, &vec![7u8; n * 20]);
    v.extend_from_slice(b"ee");
    v
}

fn multi(paths: &[&[&str]]) -> Vec<u8> {
    let mut v = b"d4:infod5:filesl".to_vec();
    for p in paths {
        v.extend_from_slice(b"d6:lengthi1000e4:pathl");
        for c in *p {
            bstr(&mut v, c.as_bytes());
        }
        v.extend_from_slice(b"ee");
    }
    v.extend_from_slice(b"e4:name3:dir12:piece lengthi16384e6:pieces20:");
    v.extend(std::iter::repeat_n(1u8, 20));
    v.extend_from_slice(b"ee");
    v
}

fn torrent_seeds() -> Vec<Vec<u8>> {
    let t = one_file(
        "f.bin",
        40000,
        &["udp://t.example.org:80/a", "http://127.0.0.1:1/"],
    );
    let bare = {
        let p = one_file("g", 100, &[]);
        p[b"d4:info".len()..p.len() - 1].to_vec()
    };
    vec![
        t,
        one_file("video.mkv", 1 << 20, &[]),
        multi(&[&["a", "b.txt"], &["c"], &["sub", "deep", "x.desktop"]]),
        multi(&[&["..", "evil"]]),
        bare,
        b"d8:announce3:x:y4:infodee".to_vec(),
    ]
}

const MAGNET_SEEDS: &[&str] = &[
    "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335aa7c1367a88a&dn=a+b&tr=udp%3A%2F%2Ft.example.org%3A1",
    "magnet:?xt=urn:btih:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&tr=http://127.0.0.1:631/&tr=http://[::1]:1/",
    "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335aa7c1367a88a&xs=http://e/x&dn=%E2%80%AEexe.pdf",
];

const TOKENS: &[&[u8]] = &[
    b"d",
    b"e",
    b"l",
    b"i",
    b"0:",
    b"4:info",
    b"4:name",
    b"5:files",
    b"4:path",
    b"6:length",
    b"i-1e",
    b"i0e",
    b"99999999:",
    b"2:..",
    b"1:/",
    b"1:.",
    b"%",
    b"&tr=",
    b"&xt=urn:btih:",
    b"http://",
    b"udp://127.1:1",
    b"%00",
    b"\xe2\x80\xae",
    b"announce-list",
];

fn mutate(r: &mut Rng, seed: &[u8]) -> Vec<u8> {
    let mut m = seed.to_vec();
    for _ in 0..=r.below(6) {
        let at = r.below(m.len() + 1);
        match r.below(6) {
            0 if !m.is_empty() => {
                let i = r.below(m.len());
                m[i] = r.next() as u8;
            }
            1 if !m.is_empty() => {
                let end = (at + r.below(16)).min(m.len());
                m.drain(at.min(end)..end);
            }
            2 => {
                let t = TOKENS[r.below(TOKENS.len())];
                m.splice(at..at, t.iter().copied());
            }
            3 if !m.is_empty() => {
                // repeat a slice (deep nesting, long lists)
                let s = r.below(m.len());
                let e = (s + 1 + r.below(32)).min(m.len());
                let piece = m[s..e].to_vec();
                for _ in 0..r.below(64) {
                    m.splice(at..at, piece.iter().copied());
                }
            }
            4 => m.truncate(at),
            _ => m.insert(at, r.next() as u8),
        }
    }
    m.truncate(metainfo::MAX_TORRENT + 16);
    m
}

fn check_torrent(b: &[u8]) {
    if let Ok(sum) = metainfo::check(b) {
        assert!(sum.files.len() <= metainfo::MAX_FILES);
        assert!(confine::component(sum.name.as_bytes()).is_ok());
        for (p, _) in &sum.files {
            assert!(!p.is_empty() && p.len() <= confine::MAX_DEPTH);
            for c in p {
                assert!(confine::component(c.as_bytes()).is_ok(), "{c:?}");
            }
        }
        let (out, kept) = metainfo::rebuild(b, false).expect("a checked file rebuilds");
        assert!(kept.len() <= trackers::MAX_TRACKERS);
        assert!(kept.iter().all(|t| trackers::allowed(t, false)), "{kept:?}");
        assert_eq!(metainfo::check(&out).expect("the rebuilt file checks"), sum);
    } else {
        let _ = metainfo::rebuild(b, false);
    }
}

fn check_magnet(link: &[u8]) {
    let Ok(s) = std::str::from_utf8(link) else {
        return;
    };
    if let Some(m) = magnet::parse(s) {
        assert_eq!(m.id.len(), 40);
        assert!(
            m.id.bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert!(m.trackers.len() <= magnet::MAX_TRACKERS);
        assert!(
            m.trackers.iter().all(|t| trackers::allowed(t, false)),
            "{:?}",
            m.trackers
        );
        if let Some(n) = &m.name {
            assert!(confine::component(n.as_bytes()).is_ok());
        }
        assert!(magnet::parse(&m.canonical()).is_some(), "{}", m.canonical());
    }
}

#[test]
fn strangers_data_never_panics_and_keeps_the_promises() {
    let secs: u64 = std::env::var("ATLAS_TORRENT_FUZZ_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let seed: u64 = std::env::var("ATLAS_TORRENT_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5eed_a71a5);
    let mut r = Rng(seed | 1);
    let ts = torrent_seeds();
    for t in &ts {
        check_torrent(t);
    }
    let t0 = Instant::now();
    let mut n: u64 = 0;
    while t0.elapsed() < Duration::from_secs(secs) {
        let i = r.below(ts.len());
        let t = mutate(&mut r, &ts[i]);
        check_torrent(&t);
        let j = r.below(MAGNET_SEEDS.len());
        let m = mutate(&mut r, MAGNET_SEEDS[j].as_bytes());
        check_magnet(&m);
        n += 1;
    }
    eprintln!("fuzz_parse: {n} torrent + {n} magnet cases in {secs} s (seed {seed})");
    assert!(n > 0);
}
