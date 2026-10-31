// SPDX-License-Identifier: GPL-2.0-or-later
//! The multi-pattern matcher. Flat byte arrays, so the database can be memory-mapped and shared
//! between processes with no parsing: only the pages a scan touches become resident.
//!
//! Every pattern gives one 8-byte literal anchor, picked to be rare. A scan reads one 4-byte word
//! every [`STRIDE`] bytes and tests it against a small bitmap holding all 5 words of every anchor;
//! any anchor occurrence covers one sampled word. Words that pass reach a second bitmap (the whole
//! 8 bytes), then the buckets, then the pattern's own program.

/// Literal bytes a pattern must offer: shorter ones would fire on ordinary files.
pub const ANCHOR: usize = 8;
/// Read one word in this many bytes: `ANCHOR - 4 + 1`.
pub const STRIDE: usize = ANCHOR - 3;
/// Bits in the word bitmap (512 KiB): it fits a second-level cache.
const WORD_BITS: u32 = 22;

/// One step of a pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tok {
    Lit(Vec<u8>),
    /// Exactly n unknown bytes.
    Any(u32),
    /// One byte whose masked bits equal the value.
    Nibble {
        mask: u8,
        value: u8,
    },
    /// min..=max unknown bytes; `u32::MAX` means anywhere further on.
    Gap {
        min: u32,
        max: u32,
    },
}

/// Where a pattern may start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Anywhere,
    /// At this offset from the file's start, or up to `shift` bytes later.
    Start {
        at: u32,
        shift: u32,
    },
    /// At this distance before the file's end, or up to `shift` bytes later.
    End {
        back: u32,
        shift: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub toks: Vec<Tok>,
    pub place: Place,
    /// `Kind` bit mask the pattern applies to; 0 = every file.
    pub kinds: u32,
    /// What a [`Hit`] reports.
    pub id: u32,
    /// Index of this pattern among the ones that must all match; 0 for a lone pattern.
    pub part: u8,
}

/// A pattern matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub id: u32,
    pub part: u8,
    pub offset: usize,
}

fn fnv(word: u64, bits: u32) -> u32 {
    (word.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - bits)) as u32
}

fn bit(map: &[u8], i: u32) -> bool {
    map[(i >> 3) as usize] & (1 << (i & 7)) != 0
}

fn set(map: &mut [u8], i: u32) {
    map[(i >> 3) as usize] |= 1 << (i & 7);
}

/// How common a byte is in programs and documents: lower is rarer.
fn weight(b: u8) -> u32 {
    match b {
        0 => 40,
        0xff | 0x20 | 0xcc | 0x90 => 16,
        b'a'..=b'z' | b'0'..=b'9' => 8,
        b'A'..=b'Z' | 0x01..=0x08 | 0x0a | 0x0d => 5,
        _ => 1,
    }
}

/// Different byte values an anchor must have: a run of padding occurs in every file.
const MIN_DISTINCT: usize = 4;

fn distinct(w: &[u8]) -> usize {
    let mut seen = [false; 256];
    w.iter()
        .filter(|b| !std::mem::replace(&mut seen[**b as usize], true))
        .count()
}

/// Offset in `lit` of its rarest [`ANCHOR`]-byte window. Repeated bytes count against it.
fn best_window(lit: &[u8]) -> Option<(usize, u32)> {
    lit.windows(ANCHOR)
        .enumerate()
        .filter(|(_, w)| distinct(w) >= MIN_DISTINCT)
        .map(|(i, w)| {
            let mut seen = [false; 256];
            let score = w
                .iter()
                .map(|b| {
                    let again = std::mem::replace(&mut seen[*b as usize], true);
                    weight(*b) * if again { 4 } else { 1 }
                })
                .sum();
            (i, score)
        })
        .min_by_key(|(_, s)| *s)
}

/// Bytes before the anchor (fixed distance only) and the anchor: from the pattern's fixed head.
fn anchor_of(p: &Pattern) -> Option<(u32, [u8; ANCHOR])> {
    let (mut fixed, mut best) = (0u32, None::<(u32, u32, [u8; ANCHOR])>);
    for t in &p.toks {
        match t {
            Tok::Lit(l) => {
                if let Some((i, score)) = best_window(l)
                    && best.is_none_or(|b| score < b.1)
                {
                    let mut a = [0; ANCHOR];
                    a.copy_from_slice(&l[i..i + ANCHOR]);
                    best = Some((fixed + i as u32, score, a));
                }
                fixed += l.len() as u32;
            }
            Tok::Any(n) => fixed += n,
            Tok::Nibble { .. } => fixed += 1,
            Tok::Gap { .. } => break,
        }
    }
    best.map(|(at, _, a)| (at, a))
}

/// True when the pattern can be indexed.
pub fn indexable(p: &Pattern) -> bool {
    anchor_of(p).is_some()
}

fn encode(p: &Pattern, lead: u32, out: &mut Vec<u8>) {
    let (place, a, b) = match p.place {
        Place::Anywhere => (0u8, 0, 0),
        Place::Start { at, shift } => (1, at, shift),
        Place::End { back, shift } => (2, back, shift),
    };
    let mut body = Vec::new();
    for t in &p.toks {
        match t {
            Tok::Lit(l) => {
                body.push(1);
                body.extend((l.len() as u32).to_le_bytes());
                body.extend(l);
            }
            Tok::Any(n) => {
                body.push(2);
                body.extend(n.to_le_bytes());
            }
            Tok::Nibble { mask, value } => body.extend([3, *mask, *value]),
            Tok::Gap { min, max } => {
                body.push(4);
                body.extend(min.to_le_bytes());
                body.extend(max.to_le_bytes());
            }
        }
    }
    out.extend(p.id.to_le_bytes());
    out.extend(p.kinds.to_le_bytes());
    out.extend(lead.to_le_bytes());
    out.extend(a.to_le_bytes());
    out.extend(b.to_le_bytes());
    out.extend((body.len() as u32).to_le_bytes());
    out.extend([place, p.part, 0, 0]);
    out.extend(body);
}

const HEAD: usize = 28;

/// Compile patterns (each must be [`indexable`]) to the flat form [`Set::new`] reads:
/// `| word bits u32 | second bits u32 | bucket bits u32 | patterns len u32 | word map | second map
/// | bucket starts (u32 * (2^bits + 1)) | entries (u32) | patterns |`.
pub fn compile(patterns: &[Pattern]) -> Vec<u8> {
    let n = patterns.len().max(1) as u64;
    let second_bits = (n * 16).next_power_of_two().trailing_zeros().max(16);
    let bucket_bits = n.next_power_of_two().trailing_zeros().max(4);
    let mut words = vec![0u8; 1 << (WORD_BITS - 3)];
    let mut second = vec![0u8; 1 << (second_bits - 3)];
    let mut blob = Vec::new();
    let mut keyed: Vec<(u32, u32)> = Vec::with_capacity(patterns.len());
    for p in patterns {
        let (lead, a) = anchor_of(p).expect("indexable");
        for w in a.windows(4) {
            let w = u32::from_le_bytes(w.try_into().expect("4 bytes"));
            set(&mut words, fnv(w.into(), WORD_BITS));
        }
        let whole = u64::from_le_bytes(a);
        set(&mut second, fnv(whole, second_bits));
        keyed.push((fnv(whole, bucket_bits), blob.len() as u32));
        encode(p, lead, &mut blob);
    }
    keyed.sort_unstable();
    let mut starts = vec![0u32; (1usize << bucket_bits) + 1];
    for (b, _) in &keyed {
        starts[*b as usize + 1] += 1;
    }
    for i in 1..starts.len() {
        starts[i] += starts[i - 1];
    }
    let mut out = Vec::with_capacity(16 + words.len() + second.len() + blob.len());
    for v in [WORD_BITS, second_bits, bucket_bits, blob.len() as u32] {
        out.extend(v.to_le_bytes());
    }
    out.extend(&words);
    out.extend(&second);
    starts.iter().for_each(|s| out.extend(s.to_le_bytes()));
    keyed.iter().for_each(|k| out.extend(k.1.to_le_bytes()));
    out.extend(&blob);
    out
}

/// A compiled set over borrowed (usually memory-mapped) bytes.
#[derive(Clone, Copy)]
pub struct Set<'a> {
    words: &'a [u8],
    second: &'a [u8],
    starts: &'a [u8],
    entries: &'a [u8],
    blob: &'a [u8],
    second_bits: u32,
    bucket_bits: u32,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

impl<'a> Set<'a> {
    /// None: the bytes are not a compiled set.
    pub fn new(b: &'a [u8]) -> Option<Self> {
        let g = |i: usize| b.get(i..i + 4).map(|_| u32_at(b, i));
        let (w, s, k, blob) = (g(0)?, g(4)?, g(8)?, g(12)? as usize);
        if w != WORD_BITS || !(16..=40).contains(&s) || !(4..=31).contains(&k) {
            return None;
        }
        let (rest, a) = (b.get(16..)?, 1usize << (w - 3));
        let c = 1usize << (s - 3);
        let n = ((1usize << k) + 1) * 4;
        let words = rest.get(..a)?;
        let second = rest.get(a..a + c)?;
        let starts = rest.get(a + c..a + c + n)?;
        let count = u32_at(starts, n - 4) as usize * 4;
        let entries = rest.get(a + c + n..a + c + n + count)?;
        let blob = rest.get(a + c + n + count..a + c + n + count + blob)?;
        Some(Self {
            words,
            second,
            starts,
            entries,
            blob,
            second_bits: s,
            bucket_bits: k,
        })
    }

    /// Calls `hit` for every match in `data`; `kind` is the file's `Kind` bit.
    /// Stops when `hit` returns false.
    pub fn scan(&self, data: &[u8], kind: u32, hit: &mut dyn FnMut(Hit) -> bool) {
        if data.len() < ANCHOR {
            return;
        }
        let last = data.len() - ANCHOR;
        let mut i = STRIDE - 1;
        // Word at i covers anchors starting at i-4..=i.
        while i + 4 <= data.len() {
            let w = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
            if bit(self.words, fnv(w.into(), WORD_BITS)) {
                for at in i.saturating_sub(4)..=i.min(last) {
                    let a = u64::from_le_bytes(data[at..at + 8].try_into().expect("8 bytes"));
                    if bit(self.second, fnv(a, self.second_bits))
                        && !self.bucket(data, at, a, kind, hit)
                    {
                        return;
                    }
                }
            }
            i += STRIDE;
        }
        // The tail: an anchor that starts past the last sampled word.
        for at in i.saturating_sub(4)..=last {
            let a = u64::from_le_bytes(data[at..at + 8].try_into().expect("8 bytes"));
            if bit(self.second, fnv(a, self.second_bits)) && !self.bucket(data, at, a, kind, hit) {
                return;
            }
        }
    }

    fn bucket(
        &self,
        data: &[u8],
        at: usize,
        anchor: u64,
        kind: u32,
        hit: &mut dyn FnMut(Hit) -> bool,
    ) -> bool {
        let b = fnv(anchor, self.bucket_bits) as usize * 4;
        let (from, to) = (u32_at(self.starts, b), u32_at(self.starts, b + 4));
        for e in from..to {
            let off = u32_at(self.entries, e as usize * 4) as usize;
            if let Some(h) = self.check(off, data, at, kind)
                && !hit(h)
            {
                return false;
            }
        }
        true
    }

    fn check(&self, off: usize, data: &[u8], at: usize, kind: u32) -> Option<Hit> {
        let b = &self.blob[off..];
        let (id, kinds, lead) = (u32_at(b, 0), u32_at(b, 4), u32_at(b, 8) as usize);
        if kinds != 0 && kinds & kind == 0 {
            return None;
        }
        let start = at.checked_sub(lead)?;
        let (x, y, len) = (
            u32_at(b, 12) as usize,
            u32_at(b, 16) as usize,
            u32_at(b, 20),
        );
        let (place, part) = (b[24], b[25]);
        let ok = match place {
            1 => start >= x && start - x <= y,
            2 => {
                let from = data.len().checked_sub(x);
                from.is_some_and(|f| start >= f && start - f <= y)
            }
            _ => true,
        };
        let prog = &b[HEAD..HEAD + len as usize];
        (ok && run(prog, data, start)).then_some(Hit {
            id,
            part,
            offset: start,
        })
    }
}

/// True when the program matches at `at`. Gaps try each place the next literal occurs.
fn run(prog: &[u8], data: &[u8], mut at: usize) -> bool {
    let mut p = 0;
    while p < prog.len() {
        match prog[p] {
            1 => {
                let n = u32_at(prog, p + 1) as usize;
                let lit = &prog[p + 5..p + 5 + n];
                if data.get(at..at + n) != Some(lit) {
                    return false;
                }
                at += n;
                p += 5 + n;
            }
            2 => {
                at += u32_at(prog, p + 1) as usize;
                if at > data.len() {
                    return false;
                }
                p += 5;
            }
            3 => {
                match data.get(at) {
                    Some(b) if b & prog[p + 1] == prog[p + 2] => {}
                    _ => return false,
                }
                at += 1;
                p += 3;
            }
            _ => {
                let (min, max) = (u32_at(prog, p + 1) as usize, u32_at(prog, p + 5));
                let rest = &prog[p + 9..];
                let lo = at.saturating_add(min).min(data.len());
                let hi = if max == u32::MAX {
                    data.len()
                } else {
                    at.saturating_add(max as usize).min(data.len())
                };
                if rest.is_empty() {
                    return at.saturating_add(min) <= data.len();
                }
                return (lo..=hi).take(GAP_TRIES).any(|s| run(rest, data, s)) || {
                    // A long open gap: jump between occurrences of the next literal.
                    rest[0] == 1 && next_literal(rest, data, lo, hi)
                };
            }
        }
    }
    true
}

const GAP_TRIES: usize = 256;

fn next_literal(rest: &[u8], data: &[u8], lo: usize, hi: usize) -> bool {
    let n = u32_at(rest, 1) as usize;
    let lit = &rest[5..5 + n];
    let end = hi.saturating_add(n).min(data.len());
    let window = &data[lo..end];
    memchr::memmem::find_iter(window, lit).any(|o| run(rest, data, lo + o))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &[u8]) -> Tok {
        Tok::Lit(s.to_vec())
    }
    fn pat(id: u32, toks: Vec<Tok>) -> Pattern {
        Pattern {
            toks,
            place: Place::Anywhere,
            kinds: 0,
            id,
            part: 0,
        }
    }
    fn hits(ps: &[Pattern], data: &[u8], kind: u32) -> Vec<(u32, usize)> {
        let c = compile(ps);
        let s = Set::new(&c).unwrap();
        let mut v = Vec::new();
        s.scan(data, kind, &mut |h| {
            v.push((h.id, h.offset));
            true
        });
        v.sort_unstable();
        v
    }

    #[test]
    fn a_literal_is_found_at_every_alignment_and_at_both_ends() {
        let p = [pat(7, vec![lit(b"QZXJ-marker-KWVY")])];
        for pad in 0..40 {
            let mut d = vec![b'.'; pad];
            d.extend(b"QZXJ-marker-KWVY");
            d.extend(vec![b'.'; 39 - pad]);
            assert_eq!(hits(&p, &d, 1), vec![(7, pad)], "front pad {pad}");
        }
        assert_eq!(hits(&p, b"QZXJ-marker-KWVY", 1), vec![(7, 0)]);
        assert!(hits(&p, b"QZXJ-marker", 1).is_empty());
    }

    #[test]
    fn short_or_gappy_literals_are_refused() {
        assert!(!indexable(&pat(1, vec![lit(b"short")])));
        assert!(!indexable(&pat(
            1,
            vec![Tok::Gap { min: 0, max: 9 }, lit(b"long-enough-literal")]
        )));
        assert!(indexable(&pat(1, vec![lit(b"exactly8")])));
        assert!(
            !indexable(&pat(1, vec![lit(&[0; 32])])),
            "padding is no anchor"
        );
        assert!(
            !indexable(&pat(1, vec![lit(b"abcabcabcabcabc")])),
            "too few values"
        );
    }

    #[test]
    fn wildcards_gaps_and_nibbles() {
        let p = [pat(
            1,
            vec![
                lit(b"HEADER--"),
                Tok::Any(2),
                Tok::Nibble {
                    mask: 0xf0,
                    value: 0x40,
                },
                Tok::Gap { min: 1, max: 4 },
                lit(b"TAIL"),
            ],
        )];
        assert_eq!(hits(&p, b"..HEADER--xxAyTAIL", 1), vec![(1, 2)]);
        assert_eq!(hits(&p, b"..HEADER--xxAyyyyTAIL", 1), vec![(1, 2)]);
        assert!(hits(&p, b"..HEADER--xxATAIL", 1).is_empty(), "gap min");
        assert!(hits(&p, b"..HEADER--xxAyyyyyTAIL", 1).is_empty(), "gap max");
        assert!(hits(&p, b"..HEADER--xx1yTAIL", 1).is_empty(), "nibble");
    }

    #[test]
    fn an_open_gap_finds_a_literal_far_away() {
        let p = [pat(
            1,
            vec![
                lit(b"FIRSTPART"),
                Tok::Gap {
                    min: 0,
                    max: u32::MAX,
                },
                lit(b"SECONDPART"),
            ],
        )];
        let mut d = b"FIRSTPART".to_vec();
        d.extend(vec![0u8; 1 << 20]);
        d.extend(b"SECONDPART");
        assert_eq!(hits(&p, &d, 1), vec![(1, 0)]);
        assert!(hits(&p, &d[..d.len() - 1], 1).is_empty());
    }

    #[test]
    fn places_and_kinds_limit_a_pattern() {
        let mut p = pat(1, vec![lit(b"PLACED-PATTERN")]);
        p.place = Place::Start { at: 4, shift: 2 };
        let at = |n: usize| {
            let mut d = vec![b'-'; n];
            d.extend(b"PLACED-PATTERN");
            d.extend(b"-----");
            d
        };
        assert!(hits(std::slice::from_ref(&p), &at(3), 1).is_empty());
        assert_eq!(hits(std::slice::from_ref(&p), &at(6), 1), vec![(1, 6)]);
        assert!(hits(std::slice::from_ref(&p), &at(7), 1).is_empty());
        p.place = Place::End { back: 20, shift: 0 };
        assert!(
            hits(std::slice::from_ref(&p), &at(9), 1).is_empty(),
            "19 from the end"
        );
        p.place = Place::End { back: 19, shift: 0 };
        assert_eq!(hits(std::slice::from_ref(&p), &at(0), 1), vec![(1, 0)]);
        p.place = Place::Anywhere;
        p.kinds = 0b10;
        assert!(hits(std::slice::from_ref(&p), &at(0), 0b01).is_empty());
        assert_eq!(hits(std::slice::from_ref(&p), &at(0), 0b10).len(), 1);
    }

    #[test]
    fn thousands_of_patterns_find_each_of_their_own() {
        let ps: Vec<Pattern> = (0..20_000u32)
            .map(|i| pat(i, vec![lit(format!("sig-{i:08}-{}", i * 7).as_bytes())]))
            .collect();
        let c = compile(&ps);
        let s = Set::new(&c).unwrap();
        for i in [0u32, 1, 999, 19_999] {
            let d = format!("padding padding sig-{i:08}-{} padding", i * 7);
            let mut v = Vec::new();
            s.scan(d.as_bytes(), 1, &mut |h| {
                v.push(h.id);
                true
            });
            assert!(v.contains(&i), "{i}: {v:?}");
        }
    }

    #[test]
    fn a_false_return_stops_the_scan() {
        let p = [pat(1, vec![lit(b"REPEATED-THING")])];
        let c = compile(&p);
        let s = Set::new(&c).unwrap();
        let mut n = 0;
        s.scan(&b"REPEATED-THING ".repeat(5), 1, &mut |_| {
            n += 1;
            false
        });
        assert_eq!(n, 1);
    }

    #[test]
    fn garbage_is_not_a_set() {
        assert!(Set::new(b"").is_none());
        assert!(Set::new(&[0xff; 64]).is_none());
        let c = compile(&[pat(1, vec![lit(b"REPEATED-THING")])]);
        assert!(Set::new(&c[..c.len() - 3]).is_none());
    }
}
