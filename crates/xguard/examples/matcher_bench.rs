// SPDX-License-Identifier: GPL-2.0-or-later
//! Why the matcher is hand-written (ADR 0022): its speed and memory against general multi-pattern
//! automatons, on the same literal anchors.
//! Usage: matcher_bench <ndb-file> <file-list> [limit]
use std::time::Instant;
use xguard::automaton::{self, ANCHOR, Pattern, Place, Set, Tok};

fn rss() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with("VmRSS"))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let text = String::from_utf8_lossy(&std::fs::read(&a[1]).unwrap()).into_owned();
    let limit: usize = a.get(3).map_or(usize::MAX, |s| s.parse().unwrap());
    let mut lits: Vec<Vec<u8>> = Vec::new();
    for l in text.lines().take(limit) {
        let body = l.split(':').nth(3).unwrap_or("");
        if let Some(t) = xguard::sigs::body(body)
            && let Some(Tok::Lit(v)) = t
                .into_iter()
                .find(|t| matches!(t, Tok::Lit(v) if v.len() >= ANCHOR))
        {
            lits.push(v[..v.len().min(32)].to_vec());
        }
    }
    lits.retain(|l| {
        l.windows(ANCHOR).any(|w| {
            let mut v = w.to_vec();
            v.sort_unstable();
            v.dedup();
            v.len() >= 4
        })
    });
    lits.sort_unstable();
    lits.dedup();
    let mut files = Vec::new();
    for f in std::fs::read_to_string(&a[2]).unwrap().lines() {
        if let Ok(d) = std::fs::read(f) {
            files.push(d);
        }
    }
    let bytes: usize = files.iter().map(Vec::len).sum();
    eprintln!("patterns={} corpus={} MB", lits.len(), bytes >> 20);
    let base = rss();

    let t = Instant::now();
    let ps: Vec<Pattern> = lits
        .iter()
        .enumerate()
        .map(|(i, l)| Pattern {
            toks: vec![Tok::Lit(l.clone())],
            place: Place::Anywhere,
            kinds: 0,
            id: i as u32,
            part: 0,
        })
        .collect();
    let c = automaton::compile(&ps);
    let build = t.elapsed();
    let set = Set::new(&c).unwrap();
    for _ in 0..2 {
        let (t, mut n) = (Instant::now(), 0u64);
        for d in &files {
            set.scan(d, 1, &mut |_| {
                n += 1;
                true
            });
        }
        let e = t.elapsed();
        eprintln!(
            "xguard: build {build:?} file {} MB, {:.0} MB/s, hits {n}",
            c.len() >> 20,
            bytes as f64 / 1e6 / e.as_secs_f64()
        );
    }
    drop(ps);

    let before = rss();
    let t = Instant::now();
    let ac = aho_corasick::AhoCorasick::builder()
        .kind(Some(aho_corasick::AhoCorasickKind::ContiguousNFA))
        .build(&lits)
        .unwrap();
    let build = t.elapsed();
    let (t, mut n) = (Instant::now(), 0u64);
    for d in &files {
        n += ac.find_overlapping_iter_count(d);
    }
    let e = t.elapsed();
    eprintln!(
        "aho-corasick contiguous: build {build:?} heap {} MB rss+{} MB, {:.0} MB/s, hits {n}",
        ac.memory_usage() >> 20,
        rss().saturating_sub(before) >> 10,
        bytes as f64 / 1e6 / e.as_secs_f64()
    );
    drop(ac);

    let before = rss();
    let t = Instant::now();
    let da: daachorse::DoubleArrayAhoCorasick<u32> =
        daachorse::DoubleArrayAhoCorasick::new(&lits).unwrap();
    let build = t.elapsed();
    let (t, mut n) = (Instant::now(), 0u64);
    for d in &files {
        n += da.find_overlapping_iter(d).count() as u64;
    }
    let e = t.elapsed();
    eprintln!(
        "daachorse: build {build:?} heap {} MB rss+{} MB, {:.0} MB/s, hits {n}, base rss {} MB",
        da.heap_bytes() >> 20,
        rss().saturating_sub(before) >> 10,
        bytes as f64 / 1e6 / e.as_secs_f64(),
        base >> 10
    );
}

trait CountAll {
    fn find_overlapping_iter_count(&self, d: &[u8]) -> u64;
}

impl CountAll for aho_corasick::AhoCorasick {
    fn find_overlapping_iter_count(&self, d: &[u8]) -> u64 {
        let mut st = aho_corasick::automaton::OverlappingState::start();
        let mut n = 0;
        let input = aho_corasick::Input::new(d);
        loop {
            self.find_overlapping(input.clone(), &mut st);
            if st.get_match().is_none() {
                return n;
            }
            n += 1;
        }
    }
}
