// SPDX-License-Identifier: BSD-2-Clause
//! The static token-embedding table (`.wst`) and the tokenizer rules both sides follow: this is
//! a port, line by line, of the site's reference reader (`site/src/lib/docs-ai/static-embed.js`),
//! which also builds the table (`site/scripts/lib/search-table.mjs`). Layout, little-endian:
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0 | 4 | magic `WSTB` |
//! | 4 | 2 | version (1) |
//! | 6 | 2 | dim D |
//! | 8 | 4 | V, the number of tokens |
//! | 12 | 4 | flags; the low byte is the bits per value, 4 or 8 |
//! | 16 | 4x6 | section offsets: fold, token offsets, blob, scales, vectors, end |
//!
//! Fold: `nRanges`, `nMaps` (u32), `nRanges` (start, end) pairs of separator code points (space,
//! control, punctuation: dropped, they end a word), `nMaps` sorted code points, `nMaps + 1`
//! offsets, then the replacement UTF-8 (accents off, lowercase; a combining mark maps to
//! nothing); a code point in neither is kept as it is. Token offsets: V+1 `u32` into the blob;
//! the blob holds the tokens' UTF-8 sorted bytewise (binary search; id = row). Scales: V `f32`.
//! Vectors: V rows of D values, most important dimension first, so the first d <= D values,
//! renormalised, are an embedding of their own (Matryoshka); 8 bits are `i8`, 4 bits are two
//! values per byte (low nibble the even index), stored `q + 8`; `x = q * scale[row]`.

use std::collections::BTreeSet;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::Path;

use crate::{Error, err};

const HEADER: usize = 40;
/// Sanity caps on the sections read into memory (the shipped table: 50 KB fold, 0.8 MB tokens).
const MAX_FOLD: usize = 8 << 20;
const MAX_BLOB: usize = 64 << 20;
/// Most tokens one text contributes; the rest is ignored.
pub const MAX_TOKENS: usize = 256;
/// Longest word, in code points, that is cut into pieces; a longer one is dropped.
pub const MAX_WORD: usize = 100;

fn u(x: u32) -> usize {
    usize::try_from(x).unwrap_or(usize::MAX)
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

/// Chinese characters (BERT's list): each is a word of its own.
const CJK: [(u32, u32); 8] = [
    (0x3400, 0x4dbf),
    (0x4e00, 0x9fff),
    (0xf900, 0xfaff),
    (0x20000, 0x2a6df),
    (0x2a700, 0x2b73f),
    (0x2b740, 0x2b81f),
    (0x2b820, 0x2ceaf),
    (0x2f800, 0x2fa1f),
];

fn is_cjk(c: u32) -> bool {
    CJK.iter().any(|(a, b)| c >= *a && c <= *b)
}

/// A Hangul syllable as the jamo its NFD makes of it (the table cannot list 11 172 of them).
fn hangul(cp: u32) -> String {
    let s = cp - 0xac00;
    let (l, v, t) = (0x1100 + s / 588, 0x1161 + (s % 588) / 28, s % 28);
    let mut out: String = [l, v].iter().filter_map(|c| char::from_u32(*c)).collect();
    if t > 0 {
        out.extend(char::from_u32(0x11a7 + t));
    }
    out
}

/// What the table's fold section says about code points.
struct Fold {
    seps: Vec<(u32, u32)>,
    cps: Vec<u32>,
    offs: Vec<u32>,
    blob: String,
}

impl Fold {
    fn parse(b: &[u8]) -> Result<Fold, Error> {
        let short = || Error("short fold section".into());
        let n_ranges = u(le32(b, 0).ok_or_else(short)?);
        let n_maps = u(le32(b, 4).ok_or_else(short)?);
        if n_ranges > 0x20000 || n_maps > 0x40000 {
            return err("fold section size out of range");
        }
        let (ranges_at, cps_at) = (8, 8 + n_ranges * 8);
        let (offs_at, blob_at) = (cps_at + n_maps * 4, cps_at + n_maps * 4 + (n_maps + 1) * 4);
        let seps = (0..n_ranges)
            .map(|i| Some((le32(b, ranges_at + i * 8)?, le32(b, ranges_at + i * 8 + 4)?)))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(short)?;
        let cps = (0..n_maps)
            .map(|i| le32(b, cps_at + i * 4))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(short)?;
        let offs = (0..=n_maps)
            .map(|i| le32(b, offs_at + i * 4))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(short)?;
        let len = offs.last().copied().map_or(0, u);
        let blob = std::str::from_utf8(b.get(blob_at..blob_at + len).ok_or_else(short)?)
            .map_err(|_| Error("fold replacements are not UTF-8".into()))?
            .to_string();
        if cps.windows(2).any(|w| w[0] >= w[1]) || offs.windows(2).any(|w| w[0] > w[1]) {
            return err("fold section is not sorted");
        }
        if (0..n_maps).any(|i| blob.get(u(offs[i])..u(offs[i + 1])).is_none()) {
            return err("fold replacement not on a character boundary");
        }
        Ok(Fold {
            seps,
            cps,
            offs,
            blob,
        })
    }

    fn mapped(&self, cp: u32) -> Option<&str> {
        let i = self.cps.binary_search(&cp).ok()?;
        self.blob.get(u(self.offs[i])..u(self.offs[i + 1]))
    }

    fn is_sep(&self, cp: u32) -> bool {
        let i = self.seps.partition_point(|r| r.1 < cp);
        self.seps.get(i).is_some_and(|r| r.0 <= cp)
    }
}

/// Where the rows live: in memory, or in the file, read one row at a time (no `unsafe`, so no
/// `mmap`: the table is 15 MB and a query touches a few rows).
enum Rows {
    Mem(Vec<u8>),
    File(File, u64),
}

/// A loaded table: the fold section, tokens and scales in memory, the vectors where [`Rows`] says.
pub struct Table {
    dim: usize,
    bits: usize,
    v: usize,
    fold: Fold,
    fold_raw: Vec<u8>,
    tokoff: Vec<u32>,
    blob: String,
    scale: Vec<f32>,
    rows: Rows,
    max_piece: usize,
}

impl Table {
    /// A table from its bytes (tests, small tables, build tools).
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Table, Error> {
        let h = header(&bytes)?;
        if bytes.len() != u(h.offs[5]) {
            return err("table is not the length its header says");
        }
        let mut t = Table::parse(&bytes[..u(h.offs[4])], &h)?;
        t.rows = Rows::Mem(bytes[u(h.offs[4])..].to_vec());
        Ok(t)
    }

    /// A table in a file: everything but the vectors is read now, a vector row when asked.
    pub fn open(path: &Path) -> Result<Table, Error> {
        let f = File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        let mut head = [0u8; HEADER];
        f.read_exact_at(&mut head, 0)
            .map_err(|e| Error(e.to_string()))?;
        let h = header(&head)?;
        let len = f.metadata().map_err(|e| Error(e.to_string()))?.len();
        if len != u64::from(h.offs[5]) {
            return err("table is not the length its header says");
        }
        let mut front = vec![0u8; u(h.offs[4])];
        f.read_exact_at(&mut front, 0)
            .map_err(|e| Error(e.to_string()))?;
        let mut t = Table::parse(&front, &h)?;
        t.rows = Rows::File(f, u64::from(h.offs[4]));
        Ok(t)
    }

    fn parse(front: &[u8], h: &Header) -> Result<Table, Error> {
        let [o_fold, o_tok, o_blob, o_scale, o_vec, o_end] = h.offs.map(u);
        let (dim, v) = (h.dim, h.v);
        if !(HEADER <= o_fold
            && o_fold <= o_tok
            && o_tok <= o_blob
            && o_blob <= o_scale
            && o_scale <= o_vec
            && o_vec <= o_end)
        {
            return err("table sections out of order");
        }
        let row_bytes = if h.bits == 8 { dim } else { dim / 2 };
        if o_end - o_vec != v * row_bytes
            || o_vec - o_scale != 4 * v
            || o_blob - o_tok != 4 * (v + 1)
        {
            return err("table section sizes do not match dim and token count");
        }
        let fold_raw = front[o_fold..o_tok].to_vec();
        let fold = Fold::parse(&fold_raw)?;
        let tokoff = (0..=v)
            .map(|i| le32(front, o_tok + 4 * i))
            .collect::<Option<Vec<_>>>()
            .ok_or(Error("short table".into()))?;
        let blob_len = tokoff.last().copied().map_or(0, u);
        let blob_bytes = front
            .get(o_blob..o_blob + blob_len)
            .ok_or(Error("token blob outside the table".into()))?;
        let blob = std::str::from_utf8(blob_bytes)
            .map_err(|_| Error("token blob is not UTF-8".into()))?
            .to_string();
        let mut max_piece = 1;
        let mut prev: Option<&str> = None;
        for w in tokoff.windows(2) {
            let tok = blob
                .get(u(w[0])..u(w[1]))
                .ok_or(Error("token offset not on a character boundary".into()))?;
            if prev.is_some_and(|p| p >= tok) {
                return err("tokens are not sorted and distinct");
            }
            max_piece = max_piece.max(tok.chars().count());
            prev = Some(tok);
        }
        let scale = (0..v)
            .map(|i| le32(front, o_scale + 4 * i).map(f32::from_bits))
            .collect::<Option<Vec<_>>>()
            .ok_or(Error("short table".into()))?;
        Ok(Table {
            dim,
            bits: h.bits,
            v,
            fold,
            fold_raw,
            tokoff,
            blob,
            scale,
            rows: Rows::Mem(Vec::new()),
            max_piece,
        })
    }

    /// Dimensions of a full vector.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Bits per stored value (4 or 8).
    pub fn bits(&self) -> usize {
        self.bits
    }

    /// Number of tokens.
    pub fn len(&self) -> usize {
        self.v
    }

    /// True for a table with no token.
    pub fn is_empty(&self) -> bool {
        self.v == 0
    }

    /// Token `i` as written in the table (`##` marks a piece inside a word).
    pub fn token(&self, i: usize) -> &str {
        match (self.tokoff.get(i), self.tokoff.get(i + 1)) {
            (Some(a), Some(b)) => self.blob.get(u(*a)..u(*b)).unwrap_or(""),
            _ => "",
        }
    }

    /// The id of exactly this token.
    pub fn find(&self, s: &str) -> Option<usize> {
        let (mut lo, mut hi) = (0, self.v);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.token(mid).cmp(s) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid),
            }
        }
        None
    }

    /// The words of a text as this table sees them: lowercase, accents off, split at spaces and
    /// punctuation, one word per Chinese character (`words` of the reference reader).
    pub fn words(&self, text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        for ch in text.chars() {
            let cp = u32::from(ch);
            if (0xac00..=0xd7a3).contains(&cp) {
                cur.push_str(&hangul(cp));
            } else if is_cjk(cp) {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                out.push(
                    self.fold
                        .mapped(cp)
                        .map_or_else(|| ch.to_string(), str::to_string),
                );
            } else if let Some(m) = self.fold.mapped(cp) {
                cur.push_str(m);
            } else if self.fold.is_sep(cp) {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            } else {
                cur.push(ch);
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out
    }

    /// Piece ids of one word, greedy longest match from the left (`##` marks a piece inside a
    /// word); None when a part has no piece, or the word is over [`MAX_WORD`].
    fn pieces(&self, word: &str, buf: &mut String) -> Option<Vec<usize>> {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() > MAX_WORD {
            return None;
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let mut j = chars.len().min(i + self.max_piece);
            let mut found = None;
            while j > i {
                buf.clear();
                if i > 0 {
                    buf.push_str("##");
                }
                buf.extend(&chars[i..j]);
                if let Some(id) = self.find(buf) {
                    found = Some(id);
                    break;
                }
                j -= 1;
            }
            out.push(found?);
            i = j;
        }
        Some(out)
    }

    /// The piece ids of a text (at most [`MAX_TOKENS`]): a word with a part no piece covers
    /// contributes nothing (no unknown token), and no [CLS], [SEP] or punctuation piece is added.
    pub fn tokens(&self, text: &str) -> Vec<usize> {
        let mut out = Vec::new();
        let mut buf = String::new();
        for w in self.words(text) {
            if let Some(p) = self.pieces(&w, &mut buf) {
                for id in p {
                    out.push(id);
                    if out.len() >= MAX_TOKENS {
                        return out;
                    }
                }
            }
        }
        out
    }

    /// [`Table::tokens`] for a question typed by a person: a word cut into many tiny pieces
    /// (`qwertyuiop`: seven pieces for ten letters) is noise that would pull the meaning of the
    /// rest towards anything, so it is left out; a long compound ("Bildschirmfoto": four pieces
    /// for fourteen) is kept.
    pub fn query_tokens(&self, text: &str) -> Vec<usize> {
        let mut out = Vec::new();
        let mut buf = String::new();
        for w in self.words(text) {
            let n = w.chars().count();
            if let Some(p) = self.pieces(&w, &mut buf) {
                // Kana and Hangul run together without spaces: their pieces are short by nature.
                if n >= 4 && p.len() * 20 > n * 11 && w.chars().all(|c| u32::from(c) < 0x800) {
                    continue;
                }
                for id in p {
                    out.push(id);
                    if out.len() >= MAX_TOKENS {
                        return out;
                    }
                }
            }
        }
        out
    }

    /// Adds `weight * row(id)` (first `acc.len()` values) to `acc`; false if the row could not be read.
    fn add_row(&self, id: usize, weight: f32, acc: &mut [f32], buf: &mut Vec<u8>) -> bool {
        let d = acc.len().min(self.dim);
        let Some(&s) = self.scale.get(id) else {
            return false;
        };
        let row_bytes = if self.bits == 8 {
            self.dim
        } else {
            self.dim / 2
        };
        let need = if self.bits == 8 { d } else { d.div_ceil(2) };
        buf.clear();
        buf.resize(need, 0);
        let ok = match &self.rows {
            Rows::Mem(m) => m
                .get(id * row_bytes..id * row_bytes + need)
                .map(|r| buf.copy_from_slice(r))
                .is_some(),
            Rows::File(f, base) => f
                .read_exact_at(buf, base + u64::try_from(id * row_bytes).unwrap_or(0))
                .is_ok(),
        };
        if !ok {
            return false;
        }
        let w = weight * s;
        if self.bits == 8 {
            for (a, b) in acc.iter_mut().zip(buf.iter()) {
                *a += w * f32::from(i8::from_le_bytes([*b]));
            }
        } else {
            for (k, a) in acc.iter_mut().enumerate().take(d) {
                let b = buf[k >> 1];
                let q = if k & 1 == 1 { b >> 4 } else { b & 15 };
                *a += w * (f32::from(q) - 8.0);
            }
        }
        true
    }

    /// One unit vector of `dims` dimensions (at most the table's) for the parts of a text, each
    /// with a weight: the mean of its tokens' rows, normalised. None if no token is known.
    pub fn embed_parts(&self, parts: &[(&str, f32)], dims: usize) -> Option<Vec<f32>> {
        self.embed_with(parts, dims, Table::tokens)
    }

    /// [`Table::embed`] for a question typed by a person ([`Table::query_tokens`]).
    pub fn embed_query(&self, text: &str, dims: usize) -> Option<Vec<f32>> {
        self.embed_with(&[(text, 1.0)], dims, Table::query_tokens)
    }

    fn embed_with(
        &self,
        parts: &[(&str, f32)],
        dims: usize,
        tokens: fn(&Table, &str) -> Vec<usize>,
    ) -> Option<Vec<f32>> {
        let d = dims.min(self.dim);
        let mut acc = vec![0f32; d];
        let (mut buf, mut n) = (Vec::with_capacity(d), 0usize);
        for (text, w) in parts {
            for id in tokens(self, text) {
                if self.add_row(id, *w, &mut acc, &mut buf) {
                    n += 1;
                }
            }
        }
        (n > 0).then(|| normalise(acc)).flatten()
    }

    /// [`Table::embed_parts`] for one text.
    pub fn embed(&self, text: &str, dims: usize) -> Option<Vec<f32>> {
        self.embed_parts(&[(text, 1.0)], dims)
    }

    /// A smaller table with only the tokens `keep` (ids), the same fold section, dimensions and
    /// bits: for test fixtures and for an image table cut to the words it needs.
    pub fn subset(&self, keep: &BTreeSet<usize>) -> Result<Vec<u8>, Error> {
        let mut ids: Vec<usize> = keep.iter().copied().filter(|i| *i < self.v).collect();
        ids.sort_by(|a, b| self.token(*a).cmp(self.token(*b)));
        let v = ids.len();
        let row_bytes = if self.bits == 8 {
            self.dim
        } else {
            self.dim / 2
        };
        let pad = |n: usize| (4 - n % 4) % 4;
        let mut blob = Vec::new();
        let mut tokoff = Vec::with_capacity(v + 1);
        for i in &ids {
            tokoff.push(u32::try_from(blob.len()).map_err(|_| Error("blob too large".into()))?);
            blob.extend_from_slice(self.token(*i).as_bytes());
        }
        tokoff.push(u32::try_from(blob.len()).map_err(|_| Error("blob too large".into()))?);
        let mut body: Vec<u8> = Vec::new();
        let mut offs = [0u32; 6];
        let mut at = HEADER;
        let mut push = |n: usize, bytes: &[u8], body: &mut Vec<u8>| -> Result<u32, Error> {
            let o = u32::try_from(at).map_err(|_| Error("table too large".into()))?;
            body.extend_from_slice(bytes);
            body.extend(std::iter::repeat_n(0u8, n));
            at += bytes.len() + n;
            Ok(o)
        };
        offs[0] = push(pad(self.fold_raw.len()), &self.fold_raw, &mut body)?;
        let tok_bytes: Vec<u8> = tokoff.iter().flat_map(|o| o.to_le_bytes()).collect();
        offs[1] = push(0, &tok_bytes, &mut body)?;
        offs[2] = push(pad(blob.len()), &blob, &mut body)?;
        let scales: Vec<u8> = ids
            .iter()
            .flat_map(|i| self.scale[*i].to_le_bytes())
            .collect();
        offs[3] = push(0, &scales, &mut body)?;
        let mut vecs = Vec::with_capacity(v * row_bytes);
        let mut buf = vec![0u8; row_bytes];
        for i in &ids {
            match &self.rows {
                Rows::Mem(m) => buf.copy_from_slice(
                    m.get(i * row_bytes..(i + 1) * row_bytes)
                        .ok_or(Error("short rows".into()))?,
                ),
                Rows::File(f, base) => f
                    .read_exact_at(&mut buf, base + u64::try_from(i * row_bytes).unwrap_or(0))
                    .map_err(|e| Error(e.to_string()))?,
            }
            vecs.extend_from_slice(&buf);
        }
        offs[4] = push(0, &vecs, &mut body)?;
        offs[5] = u32::try_from(at).map_err(|_| Error("table too large".into()))?;
        let mut out = b"WSTB".to_vec();
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&u16::try_from(self.dim).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&u32::try_from(v).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&u32::try_from(self.bits).unwrap_or(0).to_le_bytes());
        for o in offs {
            out.extend_from_slice(&o.to_le_bytes());
        }
        out.extend_from_slice(&body);
        Ok(out)
    }
}

struct Header {
    dim: usize,
    bits: usize,
    v: usize,
    offs: [u32; 6],
}

fn header(b: &[u8]) -> Result<Header, Error> {
    if b.get(..4) != Some(b"WSTB") {
        return err("not a .wst table");
    }
    if le16(b, 4) != Some(1) {
        return err("unknown .wst version");
    }
    let (Some(dim), Some(v), Some(flags)) = (le16(b, 6), le32(b, 8), le32(b, 12)) else {
        return err("short table");
    };
    let bits = u(flags & 0xff);
    if bits != 4 && bits != 8 {
        return err("a .wst table has 4 or 8 bits per value");
    }
    let mut offs = [0u32; 6];
    for (k, o) in offs.iter_mut().enumerate() {
        *o = le32(b, 16 + 4 * k).ok_or(Error("short table".into()))?;
    }
    let (dim, v) = (usize::from(dim), u(v));
    if dim == 0 || (bits == 4 && dim % 2 == 1) || v > 4_000_000 {
        return err("table size out of range");
    }
    // Every offset is checked here, before any slice or allocation uses one: in order, and the
    // sections as long as the dimensions and the token count say.
    let [o_fold, o_tok, o_blob, o_scale, o_vec, o_end] = offs.map(u);
    if !(HEADER <= o_fold
        && o_fold <= o_tok
        && o_tok <= o_blob
        && o_blob <= o_scale
        && o_scale <= o_vec
        && o_vec <= o_end)
    {
        return err("table sections out of order");
    }
    let row_bytes = if bits == 8 { dim } else { dim / 2 };
    if o_end - o_vec != v * row_bytes || o_vec - o_scale != 4 * v || o_blob - o_tok != 4 * (v + 1) {
        return err("table section sizes do not match dim and token count");
    }
    if o_tok - o_fold > MAX_FOLD || o_scale - o_blob > MAX_BLOB {
        return err("table fold or token section out of range");
    }
    Ok(Header { dim, bits, v, offs })
}

/// `v / |v|`, or None for a zero or non-finite vector.
pub(crate) fn normalise(mut v: Vec<f32>) -> Option<Vec<f32>> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if !n.is_finite() || n < 1e-12 {
        return None;
    }
    for x in &mut v {
        *x /= n;
    }
    Some(v)
}

/// The words of a text without a table: lowercase, split at anything that is not a letter or a
/// digit. For the keyword half of a search, which has no fold section to ask.
pub fn tokenize_words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        let mut lc = c.to_lowercase();
        let l = match (lc.next(), lc.next()) {
            (Some(l), None) => l,
            _ => c,
        };
        let word = l.is_alphanumeric() || ('\u{300}'..='\u{36f}').contains(&l);
        if word {
            cur.push(l);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 4-value table written by hand (8 bits): tokens and rows; separators (space, `'`..`-`, `?`)
    /// and mappings (A-Z lowercase, `É` to `e`, a combining acute to nothing).
    pub(crate) fn toy(tokens: &[(&str, [i8; 4])]) -> Vec<u8> {
        let mut toks: Vec<_> = tokens.to_vec();
        toks.sort_by(|a, b| a.0.cmp(b.0));
        let v = toks.len();
        let mut blob = Vec::new();
        let mut offs = Vec::new();
        for (t, _) in &toks {
            offs.push(u32::try_from(blob.len()).unwrap_or(0));
            blob.extend_from_slice(t.as_bytes());
        }
        offs.push(u32::try_from(blob.len()).unwrap_or(0));
        // Fold: separators space, '-', ',', '?', '\'' ; maps 'É' (0xc9) -> "e", U+0301 -> "".
        let seps: [(u32, u32); 3] = [(0x20, 0x20), (0x27, 0x2d), (0x3f, 0x3f)];
        let upper: Vec<(u32, String)> = (0x41u32..=0x5a)
            .map(|c| {
                (
                    c,
                    char::from_u32(c + 32).map_or(String::new(), String::from),
                )
            })
            .collect();
        let mut maps: Vec<(u32, String)> = upper;
        maps.push((0xc9, "e".into()));
        maps.push((0x301, String::new()));
        let mut fold = Vec::new();
        for x in [
            u32::try_from(seps.len()).unwrap_or(0),
            u32::try_from(maps.len()).unwrap_or(0),
        ] {
            fold.extend_from_slice(&x.to_le_bytes());
        }
        for (a, b) in seps {
            fold.extend_from_slice(&a.to_le_bytes());
            fold.extend_from_slice(&b.to_le_bytes());
        }
        for (c, _) in &maps {
            fold.extend_from_slice(&c.to_le_bytes());
        }
        let mut o = 0u32;
        fold.extend_from_slice(&o.to_le_bytes());
        for (_, r) in &maps {
            o += u32::try_from(r.len()).unwrap_or(0);
            fold.extend_from_slice(&o.to_le_bytes());
        }
        for (_, r) in &maps {
            fold.extend_from_slice(r.as_bytes());
        }
        fold.resize(fold.len().next_multiple_of(4), 0);
        let o_fold = HEADER;
        let o_tok = o_fold + fold.len();
        let o_blob = o_tok + 4 * (v + 1);
        let o_scale = (o_blob + blob.len()).next_multiple_of(4);
        let o_vec = o_scale + 4 * v;
        let o_end = o_vec + 4 * v;
        let mut b = b"WSTB".to_vec();
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        for x in [v, 8, o_fold, o_tok, o_blob, o_scale, o_vec, o_end] {
            b.extend_from_slice(&u32::try_from(x).unwrap_or(0).to_le_bytes());
        }
        b.extend_from_slice(&fold);
        for o in offs {
            b.extend_from_slice(&o.to_le_bytes());
        }
        b.extend_from_slice(&blob);
        b.resize(o_scale, 0);
        for _ in &toks {
            b.extend_from_slice(&0.5f32.to_le_bytes());
        }
        for (_, row) in &toks {
            b.extend(row.iter().map(|x| x.to_le_bytes()[0]));
        }
        b
    }

    fn table(tokens: &[(&str, [i8; 4])]) -> Table {
        Table::from_bytes(toy(tokens)).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn words_fold_split_and_keep_each_chinese_character() {
        let t = table(&[("a", [1, 0, 0, 0])]);
        assert_eq!(
            t.words("Wi-Fi isn't ON, ok?"),
            ["wi", "fi", "isn", "t", "on", "ok"]
        );
        assert_eq!(t.words("  "), Vec::<String>::new());
        assert_eq!(
            t.words("Éa e\u{301}"),
            ["ea", "e"],
            "accent off in the table's fold: the mapped code points"
        );
        assert_eq!(
            t.words("文字を大きく"),
            ["文", "字", "を", "大", "きく"],
            "one word per Chinese character"
        );
        assert_eq!(
            t.words("한"),
            ["\u{1112}\u{1161}\u{11ab}"],
            "Hangul becomes jamo"
        );
    }

    #[test]
    fn wordpiece_takes_the_longest_piece_and_drops_unknown_words() {
        let t = table(&[
            ("text", [1, 0, 0, 0]),
            ("larger", [0, 1, 0, 0]),
            ("##er", [0, 0, 1, 0]),
            ("large", [0, 0, 0, 1]),
        ]);
        let name = |ids: Vec<usize>| {
            ids.into_iter()
                .map(|i| t.token(i).to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(name(t.tokens("Larger TEXT")), ["larger", "text"]);
        assert_eq!(
            name(t.tokens("largest")),
            Vec::<String>::new(),
            "no piece for 'st': dropped whole"
        );
        assert_eq!(name(t.tokens("texter")), ["text", "##er"]);
        assert!(
            t.tokens(&"a".repeat(101)).is_empty(),
            "over 100 code points: dropped"
        );
        let many = "text ".repeat(300);
        assert_eq!(t.tokens(&many).len(), MAX_TOKENS);
    }

    #[test]
    fn embed_is_the_normalised_mean_and_cuts_to_fewer_dimensions() {
        let t = table(&[("a", [100, 0, 0, 0]), ("b", [0, 100, 50, 0])]);
        let v = t.embed("a b", 4).unwrap_or_default();
        let n: f32 = v.iter().map(|x| x * x).sum();
        assert!((n - 1.0).abs() < 1e-5);
        assert!(v[0] > 0.0 && v[1] > 0.0 && v[3] == 0.0);
        let cut = t.embed("b", 2).unwrap_or_default();
        assert_eq!(cut.len(), 2);
        assert!(
            (cut[0]).abs() < 1e-6 && (cut[1] - 1.0).abs() < 1e-5,
            "{cut:?}"
        );
        assert!(t.embed("zzz", 4).is_none());
    }

    /// 4-bit rows: two values to a byte, the low nibble the even index, stored q + 8.
    #[test]
    fn four_bit_rows_are_read_low_nibble_first() {
        let mut b = toy(&[("a", [0, 0, 0, 0])]);
        b[12] = 4; // flags: 4 bits
        let o_end = usize::try_from(le32(&b, 36).unwrap_or(0)).unwrap_or(0);
        b.truncate(o_end - 2); // the row is dim/2 = 2 bytes now
        let end = u32::try_from(b.len()).unwrap_or(0);
        b[36..40].copy_from_slice(&end.to_le_bytes());
        let n = b.len();
        b[n - 2] = (8 + 3) | ((8 - 2) << 4); // values 3, -2
        b[n - 1] = 8 | (8 + 7) << 4; // values 0, 7
        let t = Table::from_bytes(b).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(t.bits(), 4);
        let v = t.embed("a", 4).unwrap_or_default();
        let want = [3.0f32, -2.0, 0.0, 7.0];
        let norm = want.iter().map(|x| x * x).sum::<f32>().sqrt();
        for (g, w) in v.iter().zip(want) {
            assert!((g - w / norm).abs() < 1e-5, "{v:?}");
        }
        let two = t.embed("a", 3).unwrap_or_default();
        assert_eq!(two.len(), 3, "an odd cut reads the last byte's low nibble");
    }

    #[test]
    fn a_broken_table_is_refused_not_trusted() {
        let good = toy(&[("a", [1, 2, 3, 4]), ("b", [1, 2, 3, 4])]);
        assert!(Table::from_bytes(good.clone()).is_ok());
        assert!(Table::from_bytes(good[..good.len() - 1].to_vec()).is_err());
        assert!(Table::from_bytes(b"nope".to_vec()).is_err());
        let blob_at = usize::try_from(le32(&good, 24).unwrap_or(0)).unwrap_or(0);
        let mut bad = good.clone();
        bad[blob_at] = 0xff; // a byte of the blob that is no longer UTF-8
        assert!(Table::from_bytes(bad).is_err());
        let mut swapped = good.clone();
        swapped.swap(blob_at, blob_at + 1);
        assert!(Table::from_bytes(swapped).is_err(), "unsorted tokens");
        let mut bits = good;
        bits[12] = 5;
        assert!(Table::from_bytes(bits).is_err(), "5 bits");
    }

    #[test]
    fn a_table_in_a_file_gives_the_same_vectors_and_a_subset_keeps_them() {
        let dir = std::env::temp_dir().join(format!("wolsem-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let path = dir.join("t.wst");
        let bytes = toy(&[
            ("a", [100, 0, 0, 0]),
            ("b", [0, 100, 50, 0]),
            ("c", [5, 5, 5, 5]),
        ]);
        std::fs::write(&path, &bytes).unwrap_or_else(|e| panic!("{e}"));
        let a = Table::from_bytes(bytes).unwrap_or_else(|e| panic!("{e}"));
        let b = Table::open(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(a.embed("a b", 4), b.embed("a b", 4));
        let keep: BTreeSet<usize> = [0, 1].into_iter().collect();
        let small = Table::from_bytes(b.subset(&keep).unwrap_or_else(|e| panic!("{e}")))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(small.len(), 2);
        assert_eq!(small.embed("a b", 4), a.embed("a b", 4));
        assert!(small.embed("c", 4).is_none());
        assert_eq!(small.words("É"), ["e"], "the fold section survives");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    /// A table that knows no useful token (keyword-only tests).
    pub(crate) fn empty() -> super::Table {
        super::Table::from_bytes(super::tests::toy(&[("zz", [1, 0, 0, 0])]))
            .unwrap_or_else(|e| panic!("{e}"))
    }
}
