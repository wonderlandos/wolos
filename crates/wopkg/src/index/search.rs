// SPDX-License-Identifier: GPL-2.0-or-later
//! Unified search: substring scan per segment, one ranked result per app (grouping key) with every
//! source that has it, best source first.
use super::segment::Segment;
use super::{Kind, Record};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Source {
    #[serde(flatten)]
    pub record: Record,
    pub installed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Hit {
    pub key: String,
    pub name: String,
    pub summary: String,
    pub score: i32,
    pub sources: Vec<Source>,
}

/// How well `r` answers `q` (lowercase, non-empty).
pub fn score(r: &Record, q: &str) -> i32 {
    let n = r.name.to_lowercase();
    let d = (n.len() as i32 - q.len() as i32).clamp(0, 50) * 2;
    let base = if n == q {
        1000
    } else if r.key == q {
        900
    } else if n.starts_with(q) {
        700 - d
    } else if n.split(['-', '_', '.', ' ']).any(|w| w == q) {
        500 - d
    } else if n.contains(q) {
        350 - d
    } else {
        100
    };
    let pop = if r.votes > 0 {
        (32 - r.votes.leading_zeros()) as i32
    } else {
        0
    };
    base + (7 - r.source_rank() as i32) * 5 + pop.min(20)
}

/// Records of `seg` whose haystack line holds every token.
pub fn matches(seg: &Segment, tokens: &[&str]) -> Vec<usize> {
    let Some(first) = tokens.iter().max_by_key(|t| t.len()) else {
        return Vec::new();
    };
    let finder = memchr::memmem::Finder::new(first.as_bytes());
    let rest: Vec<memchr::memmem::Finder> = tokens
        .iter()
        .filter(|t| *t != first)
        .map(|t| memchr::memmem::Finder::new(t.as_bytes()))
        .collect();
    let mut out: Vec<usize> = Vec::new();
    for pos in finder.find_iter(seg.haystack()) {
        let i = seg.record_at(pos);
        if out.last() == Some(&i) {
            continue;
        }
        let line = seg.hay_line(i);
        if rest.iter().all(|f| f.find(line).is_some()) {
            out.push(i);
        }
    }
    out
}

pub fn search(
    segments: &[Segment],
    query: &str,
    limit: usize,
    installed: &dyn Fn(&Record) -> bool,
) -> Vec<Hit> {
    let q = query.trim().to_lowercase();
    let tokens: Vec<&str> = q.split_whitespace().collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    let joined = tokens.join(" ");
    let mut groups: HashMap<String, Hit> = HashMap::new();
    // Arch's AppStream says which package is which app; nothing else joins two sources.
    let mut app_of: HashMap<String, String> = HashMap::new();
    for seg in segments {
        if seg.record(0).is_some_and(|r| r.kind == Kind::Catalog) {
            for r in (0..seg.len()).filter_map(|i| seg.record(i)) {
                app_of.insert(r.id, r.key);
            }
        }
    }
    let packages = std::cell::OnceCell::new();
    let package = |id: &str| -> Option<Record> {
        let at: &HashMap<&str, (usize, usize)> = packages.get_or_init(|| {
            let mut at = HashMap::new();
            for (n, seg) in segments.iter().enumerate() {
                if seg.record(0).is_some_and(|r| r.kind == Kind::Pacman) {
                    at.extend((0..seg.len()).map(|i| (seg.name(i), (n, i))));
                }
            }
            at
        });
        let &(n, i) = at.get(id)?;
        segments[n].record(i)
    };
    for seg in segments {
        for i in matches(seg, &tokens) {
            let Some(mut r) = seg.record(i) else { continue };
            let s = score(&r, &joined);
            // A catalogue hit stands for its package, found by the name people know it by.
            if r.kind == Kind::Catalog {
                match package(&r.id) {
                    Some(real) => r = real,
                    None => continue,
                }
            }
            if r.kind == Kind::Pacman
                && let Some(k) = app_of.get(&r.id)
            {
                r.key = k.clone();
            }
            let src = Source {
                installed: installed(&r),
                record: r,
            };
            let g = groups.entry(src.record.key.clone()).or_insert_with(|| Hit {
                key: src.record.key.clone(),
                name: src.record.name.clone(),
                summary: src.record.summary.clone(),
                score: i32::MIN,
                sources: Vec::new(),
            });
            if s > g.score {
                g.score = s;
                g.name = src.record.name.clone();
                if !src.record.summary.is_empty() {
                    g.summary = src.record.summary.clone();
                }
            }
            g.sources.push(src);
        }
    }
    let mut hits: Vec<Hit> = groups.into_values().collect();
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.key.cmp(&b.key)));
    hits.truncate(limit);
    for h in &mut hits {
        h.sources.sort_by_key(|s| {
            (
                s.record.source_rank(),
                s.record.repo.clone(),
                s.record.name.len(),
            )
        });
        // One entry per source repo and installable id.
        h.sources
            .dedup_by(|a, b| a.record.repo == b.record.repo && a.record.id == b.record.id);
    }
    hits
}

/// Package names starting with `prefix`, sorted, unique (shell completion).
pub fn complete(segments: &[Segment], prefix: &str, limit: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for seg in segments {
        if seg.record(0).is_some_and(|r| r.kind == Kind::Catalog) {
            continue;
        }
        for i in 0..seg.len() {
            let n = seg.name(i);
            if n.starts_with(prefix) && !n.contains(' ') {
                out.push(n.to_owned());
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::super::segment;
    use super::*;

    #[test]
    fn two_sources_are_one_app_only_when_appstream_says_so() {
        let d = crate::testutil::temp("search-ids");
        let write = |name: &str, recs: Vec<Record>| {
            segment::write(&d.join(name), "t", &recs).unwrap();
            Segment::open(&d.join(name)).unwrap()
        };
        let segs = vec![
            write(
                "extra",
                vec![
                    rec(Kind::Pacman, "extra", "gimp", "gimp", "gimp", "Editor"),
                    rec(Kind::Pacman, "extra", "krita", "krita", "krita", "Painter"),
                ],
            ),
            write(
                "flathub",
                vec![
                    rec(
                        Kind::Flatpak,
                        "flathub",
                        "GIMP",
                        "org.gimp.GIMP",
                        "org.gimp.gimp",
                        "Editor",
                    ),
                    rec(
                        Kind::Flatpak,
                        "flathub",
                        "Krita",
                        "org.kde.krita",
                        "org.kde.krita",
                        "Painter",
                    ),
                ],
            ),
            write(
                "appstream-extra",
                vec![rec(
                    Kind::Catalog,
                    "extra",
                    "GNU Image Manipulation Program",
                    "gimp",
                    "org.gimp.gimp",
                    "Editor",
                )],
            ),
        ];
        let hits = search(&segs, "gimp", 10, &|_| false);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].sources.len(), 2, "{hits:?}");
        // Same word, no metadata tying them: two entries, never a guess.
        assert_eq!(search(&segs, "krita", 10, &|_| false).len(), 2);
        // The name people know finds the package.
        let by_name = search(&segs, "manipulation", 10, &|_| false);
        assert_eq!(by_name[0].sources[0].record.id, "gimp");
        assert_eq!(by_name[0].sources[0].record.kind, Kind::Pacman);
    }

    fn rec(kind: Kind, repo: &str, name: &str, id: &str, key: &str, summary: &str) -> Record {
        Record {
            kind,
            repo: repo.into(),
            name: name.into(),
            id: id.into(),
            version: "1".into(),
            summary: summary.into(),
            key: key.into(),
            votes: 0,
        }
    }

    fn segs() -> Vec<Segment> {
        let d = crate::testutil::temp("search");
        let parts: [(&str, Vec<Record>); 3] = [
            (
                "extra",
                vec![
                    rec(
                        Kind::Pacman,
                        "extra",
                        "firefox",
                        "firefox",
                        "firefox",
                        "Web browser",
                    ),
                    rec(
                        Kind::Pacman,
                        "extra",
                        "firefox-i18n-de",
                        "firefox-i18n-de",
                        "firefox-i18n-de",
                        "German for Firefox",
                    ),
                    rec(Kind::Pacman, "extra", "vim", "vim", "vim", "Text editor"),
                ],
            ),
            (
                "flathub",
                vec![rec(
                    Kind::Flatpak,
                    "flathub",
                    "Firefox",
                    "org.mozilla.firefox",
                    "firefox",
                    "Fast web browser",
                )],
            ),
            (
                "aur",
                vec![rec(
                    Kind::Aur,
                    "aur",
                    "firefox-nightly",
                    "firefox-nightly",
                    "firefox",
                    "Nightly",
                )],
            ),
        ];
        parts
            .into_iter()
            .map(|(n, r)| {
                let p = d.join(format!("{n}.seg"));
                segment::write(&p, "fp", &r).unwrap();
                Segment::open(&p).unwrap()
            })
            .collect()
    }

    #[test]
    fn one_result_per_app_best_source_first() {
        let s = segs();
        let hits = search(&s, "Firefox", 10, &|r| r.kind == Kind::Flatpak);
        assert_eq!(hits[0].key, "firefox");
        let kinds: Vec<Kind> = hits[0].sources.iter().map(|s| s.record.kind).collect();
        assert_eq!(kinds, [Kind::Pacman, Kind::Flatpak, Kind::Aur]);
        assert!(hits[0].sources[1].installed);
        assert_eq!(hits[1].key, "firefox-i18n-de");
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn all_tokens_must_match_and_summary_counts() {
        let s = segs();
        let h = search(&s, "text editor", 10, &|_| false);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].key, "vim");
        assert!(search(&s, "  ", 10, &|_| false).is_empty());
        assert!(search(&s, "nomatchatall", 10, &|_| false).is_empty());
    }

    #[test]
    fn ranking_table() {
        let r = |n: &str| rec(Kind::Pacman, "extra", n, n, n, "");
        let q = "code";
        let mut v = [
            r("vscode-helper"),
            r("code-oss-ext"),
            r("code"),
            r("my-code"),
        ];
        v.sort_by_key(|x| -score(x, q));
        let names: Vec<&str> = v.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["code", "code-oss-ext", "my-code", "vscode-helper"]);
        let wolos = rec(Kind::Pacman, "wolos", "code", "code", "code", "");
        assert!(score(&wolos, q) > score(&r("code"), q));
    }

    #[test]
    fn completion_by_prefix() {
        let s = segs();
        assert_eq!(
            complete(&s, "fire", 10),
            ["firefox", "firefox-i18n-de", "firefox-nightly"]
        );
        assert_eq!(complete(&s, "fire", 1), ["firefox"]);
    }
}
