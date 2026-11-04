// SPDX-License-Identifier: BSD-2-Clause
//! The keyword half of the hybrid rank: BM25 over the words of each item's title (counted three
//! times) and text, the last word of a query also matching as a prefix ("fir" finds Firewall).

use std::collections::BTreeMap;

use crate::table::tokenize_words;

const K1: f32 = 1.2;
const B: f32 = 0.75;
const TITLE_WEIGHT: u16 = 3;
/// A prefix match counts this much of an exact one.
const PREFIX_WEIGHT: f32 = 0.6;

pub(crate) struct Kw {
    post: BTreeMap<String, Vec<(u32, u16)>>,
    len: Vec<f32>,
    avg: f32,
}

/// Words of a title, plus a form with the hyphens taken out (`Wi-Fi` is also `wifi`).
fn title_words(title: &str) -> Vec<String> {
    let mut w = tokenize_words(title);
    for chunk in title.split_whitespace().filter(|c| c.contains('-')) {
        let joined: String = tokenize_words(chunk).concat();
        if !joined.is_empty() {
            w.push(joined);
        }
    }
    w
}

impl Kw {
    pub(crate) fn build(items: &[(&str, &str)]) -> Kw {
        let mut post: BTreeMap<String, Vec<(u32, u16)>> = BTreeMap::new();
        let mut len = Vec::with_capacity(items.len());
        for (i, (title, text)) in items.iter().enumerate() {
            let mut tf: BTreeMap<String, u16> = BTreeMap::new();
            let mut n = 0f32;
            for w in title_words(title) {
                *tf.entry(w).or_default() += TITLE_WEIGHT;
                n += f32::from(TITLE_WEIGHT);
            }
            for w in tokenize_words(text) {
                *tf.entry(w).or_default() += 1;
                n += 1.0;
            }
            len.push(n);
            let id = u32::try_from(i).unwrap_or(u32::MAX);
            for (w, c) in tf {
                post.entry(w).or_default().push((id, c));
            }
        }
        let avg = (len.iter().sum::<f32>() / len.len().max(1) as f32).max(1.0);
        Kw { post, len, avg }
    }

    /// Words that are in some title or text (for spelling suggestions).
    pub(crate) fn words(&self) -> impl Iterator<Item = &str> {
        self.post.keys().map(String::as_str)
    }

    /// Adds the BM25 of `list` (the postings of one query word) to `out`, and marks the items in `seen`.
    fn add_list(&self, list: &[(u32, u16)], weight: f32, out: &mut [f32], seen: &mut [bool]) {
        let n = self.len.len() as f32;
        let df = list.len() as f32;
        let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
        for (id, tf) in list {
            let i = usize::try_from(*id).unwrap_or(usize::MAX);
            let tf = f32::from(*tf);
            let l = self.len.get(i).copied().unwrap_or(self.avg);
            let s = idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * l / self.avg));
            if let Some(o) = out.get_mut(i) {
                *o += weight * s;
                seen[i] = true;
            }
        }
    }

    /// Items that hold the word (or, for the word being typed, a word that starts with it).
    fn postings(&self, w: &str, prefix: bool) -> Vec<(&[(u32, u16)], f32)> {
        let mut v: Vec<(&[(u32, u16)], f32)> = Vec::new();
        if let Some(l) = self.post.get(w) {
            v.push((l.as_slice(), 1.0));
        }
        if prefix && w.chars().count() >= 2 {
            for (word, list) in self.post.range(w.to_string()..) {
                if !word.starts_with(w) {
                    break;
                }
                if word != w {
                    v.push((list.as_slice(), PREFIX_WEIGHT));
                }
            }
        }
        v
    }

    /// BM25 score per item for `query` (0 where it does not count). Words that most items hold
    /// ("the", "of") are ignored unless the query is only such words, and an item must hold
    /// at least half of the other words: "weather in Paris" does not find an item for "in".
    pub(crate) fn score(&self, query: &str) -> Vec<f32> {
        let n = self.len.len();
        let mut words = tokenize_words(crate::capped(query));
        words.dedup();
        let last = words.len().saturating_sub(1);
        let common = |w: &str| {
            self.post
                .get(w)
                .is_some_and(|l| l.len() > 8 && l.len() * 8 > n)
        };
        let keep: Vec<usize> = (0..words.len()).filter(|k| !common(&words[*k])).collect();
        let keep = if keep.is_empty() {
            (0..words.len()).collect()
        } else {
            keep
        };
        let (mut out, mut cover) = (vec![0f32; n], vec![0usize; n]);
        for k in &keep {
            let mut seen = vec![false; n];
            for (list, w) in self.postings(&words[*k], *k == last) {
                self.add_list(list, w, &mut out, &mut seen);
            }
            for (c, s) in cover.iter_mut().zip(&seen) {
                *c += usize::from(*s);
            }
        }
        for (o, c) in out.iter_mut().zip(&cover) {
            if c * 2 < keep.len() {
                *o = 0.0;
            }
        }
        out
    }
}
