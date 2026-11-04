// SPDX-License-Identifier: BSD-2-Clause
//! Items to find, their vectors, and the hybrid rank: cosine over int8 vectors fused with a
//! keyword score by reciprocal rank (RRF, Cormack et al. 2009: `sum 1/(k + rank)`).

use std::collections::BTreeSet;

use crate::kw::Kw;
use crate::map::Procrustes;
use crate::table::{Table, normalise};
use crate::{Error, err, fuzzy};

/// One thing to find. `id` is what the caller opens (a settings row, a manual anchor, a verb);
/// the vectors are computed once, from this (English) text.
#[derive(Clone, Copy, Debug)]
pub struct ItemIn<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub text: &'a str,
}

/// Ranking knobs; [`Opts::default`] is what the OS tools use.
#[derive(Clone, Copy, Debug)]
pub struct Opts {
    /// At most this many hits.
    pub k: usize,
    /// Dimensions of the query and item vectors compared (the table's MRL cut).
    pub dims: usize,
    /// A meaning match needs at least this cosine ...
    pub min_cos: f32,
    /// ... and stand this many standard deviations above the mean cosine of all items (a
    /// question about nothing the items say has a best match that does not stand out) ...
    pub min_z: f32,
    /// ... and must be within this of the best cosine (a long tail is noise).
    pub within: f32,
    /// The RRF constant.
    pub rrf_k: f32,
}

impl Opts {
    /// For a short list of titles that word hits already answer (Settings, the Search bar's own
    /// items): a meaning match needs to stand out more, so a question about nothing finds nothing.
    pub fn strict() -> Opts {
        Opts {
            min_cos: 0.30,
            min_z: 3.5,
            ..Opts::default()
        }
    }
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            k: 8,
            dims: 256,
            min_cos: 0.26,
            min_z: 3.0,
            within: 0.2,
            rrf_k: 60.0,
        }
    }
}

/// One result: the item, the fused score and the two scores behind it (cosine 0 and keyword 0
/// when that half did not find it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub index: usize,
    pub score: f32,
    pub cos: f32,
    pub kw: f32,
}

/// Reciprocal-rank fusion of ranked lists of item numbers (best first): the items by descending
/// `sum 1/(k + rank)`, rank counted from 1; ties keep the order of first appearance.
pub fn rrf(lists: &[&[usize]], k: f32) -> Vec<(usize, f32)> {
    let mut score: Vec<(usize, f32)> = Vec::new();
    for list in lists {
        for (r, item) in list.iter().enumerate() {
            let add = 1.0 / (k + r as f32 + 1.0);
            match score.iter_mut().find(|s| s.0 == *item) {
                Some(s) => s.1 += add,
                None => score.push((*item, add)),
            }
        }
    }
    score.sort_by(|a, b| b.1.total_cmp(&a.1));
    score
}

pub struct Index {
    dims: usize,
    ids: Vec<String>,
    titles: Vec<String>,
    q: Vec<i8>,
    scale: Vec<f32>,
    kw: Kw,
}

fn quantise(v: &[f32]) -> (Vec<i8>, f32) {
    let m = v.iter().fold(0f32, |m, x| m.max(x.abs()));
    if m == 0.0 {
        return (vec![0; v.len()], 0.0);
    }
    let s = m / 127.0;
    (
        v.iter()
            .map(|x| i8::try_from((x / s).round().clamp(-127.0, 127.0) as i128).unwrap_or(0))
            .collect(),
        s,
    )
}

impl Index {
    /// Embeds every item with the table (title counted six times) and builds the keyword index.
    pub fn build(table: &Table, items: &[ItemIn], dims: usize) -> Index {
        let dims = dims.min(table.dim());
        let vecs = items
            .iter()
            .map(|it| table.embed_parts(&[(it.title, 6.0), (it.text, 1.0)], dims))
            .collect::<Vec<_>>();
        Index::assemble(items, dims, vecs.iter().map(|v| v.as_deref()))
    }

    /// An index with no vectors: search is by words only (a computer without the table).
    pub fn words_only(items: &[ItemIn]) -> Index {
        Index::assemble(items, 1, items.iter().map(|_| None))
    }

    /// An index over items whose vectors were computed elsewhere (the site's export): one int8
    /// vector with its scale per item, `dims` long; a vector of the wrong length is no vector.
    pub fn with_vectors(
        items: &[ItemIn],
        dims: usize,
        vecs: &[(Vec<i8>, f32)],
    ) -> Result<Index, Error> {
        if vecs.len() != items.len() {
            return err("an index needs one vector per item");
        }
        let floats: Vec<Option<Vec<f32>>> = vecs
            .iter()
            .map(|(q, s)| {
                (q.len() >= dims).then(|| {
                    let v: Vec<f32> = q[..dims].iter().map(|x| f32::from(*x) * s).collect();
                    normalise(v)
                })?
            })
            .collect();
        Ok(Index::assemble(
            items,
            dims,
            floats.iter().map(|v| v.as_deref()),
        ))
    }

    fn assemble<'a>(
        items: &[ItemIn],
        dims: usize,
        vecs: impl Iterator<Item = Option<&'a [f32]>>,
    ) -> Index {
        let (mut q, mut scale) = (
            Vec::with_capacity(items.len() * dims),
            Vec::with_capacity(items.len()),
        );
        for v in vecs {
            match v {
                Some(v) => {
                    let (qv, s) = quantise(v);
                    q.extend(qv);
                    scale.push(s);
                }
                None => {
                    q.extend(std::iter::repeat_n(0i8, dims));
                    scale.push(0.0);
                }
            }
        }
        Index {
            dims,
            ids: items.iter().map(|i| i.id.to_string()).collect(),
            titles: items.iter().map(|i| i.title.to_string()).collect(),
            q,
            scale,
            kw: Kw::build(&items.iter().map(|i| (i.title, i.text)).collect::<Vec<_>>()),
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn id(&self, i: usize) -> &str {
        self.ids.get(i).map_or("", String::as_str)
    }

    pub fn title(&self, i: usize) -> &str {
        self.titles.get(i).map_or("", String::as_str)
    }

    /// Cosine of the unit vector `q` with every item that has a vector, best `k` first.
    pub fn semantic(&self, q: &[f32], k: usize) -> Vec<(usize, f32)> {
        self.semantic_stats(q, k).0
    }

    /// [`Index::semantic`] with the mean and standard deviation of the cosines of all items.
    pub fn semantic_stats(&self, q: &[f32], k: usize) -> (Vec<(usize, f32)>, f32, f32) {
        let d = self.dims.min(q.len());
        // The query in int8 too: an integer dot product vectorises, and 1/127 is below the noise.
        let (qq, qs) = quantise(&q[..d]);
        let mut all: Vec<(usize, f32)> = Vec::with_capacity(self.ids.len());
        for (i, s) in self.scale.iter().enumerate() {
            if *s == 0.0 {
                continue;
            }
            let row = &self.q[i * self.dims..i * self.dims + d];
            let dot: i32 = row
                .iter()
                .zip(&qq)
                .map(|(a, b)| i32::from(*a) * i32::from(*b))
                .sum();
            all.push((i, dot as f32 * s * qs));
        }
        let n = all.len().max(1) as f32;
        let mean = all.iter().map(|a| a.1).sum::<f32>() / n;
        let sd = (all.iter().map(|a| (a.1 - mean).powi(2)).sum::<f32>() / n).sqrt();
        let k = k.min(all.len());
        if k == 0 {
            return (Vec::new(), mean, sd);
        }
        if k < all.len() {
            all.select_nth_unstable_by(k - 1, |a, b| b.1.total_cmp(&a.1));
            all.truncate(k);
        }
        all.sort_by(|a, b| b.1.total_cmp(&a.1));
        (all, mean, sd)
    }

    /// The query's unit vector in this index's space: embedded by `table`, carried by `map`
    /// when the query is in another language.
    pub fn query_vector(
        &self,
        table: &Table,
        query: &str,
        map: Option<&Procrustes>,
    ) -> Option<Vec<f32>> {
        let v = table.embed_query(crate::capped(query), self.dims)?;
        Some(match map {
            Some(m) => m.apply(&v),
            None => v,
        })
    }

    /// Hybrid rank of `query`: meaning (when a table is given and knows a word of the query)
    /// fused with the keyword score by RRF.
    pub fn search(
        &self,
        table: Option<&Table>,
        query: &str,
        map: Option<&Procrustes>,
        o: &Opts,
    ) -> Vec<Hit> {
        let query = crate::capped(query);
        let kws = self.kw.score(query);
        let mut kw_rank: Vec<usize> = (0..kws.len()).filter(|i| kws[*i] > 0.0).collect();
        kw_rank.sort_by(|a, b| kws[*b].total_cmp(&kws[*a]));
        kw_rank.truncate(o.k * 4);
        let mut sem: Vec<(usize, f32)> = Vec::new();
        if let Some(qv) = table.and_then(|t| self.query_vector(t, query, map)) {
            let (top, mean, sd) = self.semantic_stats(&qv, o.k * 4);
            sem = top;
            let best = sem.first().map_or(0.0, |s| s.1);
            sem.retain(|s| {
                s.1 >= o.min_cos && s.1 >= best - o.within && (s.1 - mean) >= o.min_z * sd
            });
        }
        let sem_rank: Vec<usize> = sem.iter().map(|s| s.0).collect();
        let fused = rrf(&[&sem_rank, &kw_rank], o.rrf_k);
        fused
            .into_iter()
            .take(o.k)
            .map(|(index, score)| Hit {
                index,
                score,
                cos: sem.iter().find(|s| s.0 == index).map_or(0.0, |s| s.1),
                kw: kws.get(index).copied().unwrap_or(0.0),
            })
            .collect()
    }

    /// The query with each word that is in no title or text replaced by the nearest one that is
    /// ("pasword" -> "password"); None when nothing was misspelt.
    pub fn did_you_mean(&self, query: &str) -> Option<String> {
        let mut changed = false;
        let words: BTreeSet<&str> = self.kw.words().collect();
        let all = crate::table::tokenize_words(crate::capped(query));
        let last = all.len().saturating_sub(1);
        let fixed: Vec<String> = all
            .into_iter()
            .enumerate()
            .map(
                |(k, w)| match fuzzy::nearest(words.iter().copied(), &w, k == last) {
                    Some(n) if n != w && !words.contains(w.as_str()) => {
                        changed = true;
                        n.to_string()
                    }
                    _ => w,
                },
            )
            .collect();
        changed.then(|| fixed.join(" "))
    }

    /// The item nearest in meaning to the query whatever its cosine ("Did you mean ..."): the
    /// caller decides whether it is near enough.
    pub fn nearest(
        &self,
        table: &Table,
        query: &str,
        map: Option<&Procrustes>,
    ) -> Option<(usize, f32)> {
        let qv = self.query_vector(table, query, map)?;
        self.semantic(&qv, 1).first().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_rewards_agreement_over_one_high_rank() {
        let fused = rrf(&[&[1, 2, 3], &[2, 3, 9]], 60.0);
        let order: Vec<usize> = fused.iter().map(|f| f.0).collect();
        assert_eq!(order[..2], [2, 3], "{order:?}");
        assert_eq!(order.len(), 4);
        assert!(rrf(&[], 60.0).is_empty());
        assert!(rrf(&[&[]], 60.0).is_empty());
    }

    fn items() -> Vec<ItemIn<'static>> {
        vec![
            ItemIn {
                id: "sound.balance",
                title: "Balance",
                text: "Moves sound between left and right speakers",
            },
            ItemIn {
                id: "net.wifi",
                title: "Wi-Fi",
                text: "Join a wireless network with a password",
            },
            ItemIn {
                id: "net.fw",
                title: "Firewall",
                text: "Blocks unwanted connections from the network",
            },
        ]
    }

    #[test]
    fn keyword_only_search_finds_prefixes_and_hyphenless_titles() {
        let table = crate::table::tests_support::empty();
        let ix = Index::build(&table, &items(), 4);
        let hits = |q: &str| {
            ix.search(None, q, None, &Opts::default())
                .iter()
                .map(|h| ix.id(h.index).to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(hits("fire")[0], "net.fw");
        assert_eq!(hits("wifi")[0], "net.wifi");
        assert_eq!(hits("speakers")[0], "sound.balance");
        assert!(hits("no such thing").is_empty());
        assert!(hits("").is_empty());
    }

    #[test]
    fn did_you_mean_fixes_a_typo_and_leaves_good_words() {
        let table = crate::table::tests_support::empty();
        let ix = Index::build(&table, &items(), 4);
        assert_eq!(
            ix.did_you_mean("wireles pasword").as_deref(),
            Some("wireless password")
        );
        assert_eq!(ix.did_you_mean("wireless password"), None);
        assert_eq!(ix.did_you_mean("zzzzzz"), None);
    }

    #[test]
    fn imported_vectors_rank_by_cosine() {
        let its = items();
        let vecs = vec![
            (vec![127, 0, 0, 0], 1.0),
            (vec![0, 127, 0, 0], 1.0),
            (vec![0, 0, 127, 0], 1.0),
        ];
        let ix = Index::with_vectors(&its, 4, &vecs).unwrap_or_else(|e| panic!("{e}"));
        let r = ix.semantic(&[0.0, 1.0, 0.0, 0.0], 2);
        assert_eq!(r[0].0, 1);
        assert!((r[0].1 - 1.0).abs() < 0.02);
        assert!(Index::with_vectors(&its, 4, &vecs[..2]).is_err());
    }
}
