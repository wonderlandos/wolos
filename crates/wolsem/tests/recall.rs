// SPDX-License-Identifier: BSD-2-Clause
//! Recall of the hybrid rank on the real manual (505 sections of the site's graph) and the real
//! Settings rows, with `tests/data/mini.wst`: a .wst cut from potion-multilingual-128M to the
//! words of these files and of the shell's seven-language copy tables (see docs/modules/search.md,
//! "The test table"). No passage has a pt-BR, ja, fr, de, ru or it vector: a question in those
//! languages meets the English vectors, as it will on a computer.

use std::path::PathBuf;
use std::time::Instant;

use serde_json::Value;
use wolsem::{Index, ItemIn, Opts, Table};

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn json(name: &str) -> Value {
    let s = std::fs::read_to_string(data(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("{name}: {e}"))
}

struct Corpus {
    ids: Vec<String>,
    titles: Vec<String>,
    texts: Vec<String>,
}

impl Corpus {
    fn load(name: &str) -> Corpus {
        let v = json(name);
        let a = v.as_array().cloned().unwrap_or_default();
        let get = |k: &str| {
            a.iter()
                .map(|x| x[k].as_str().unwrap_or("").to_string())
                .collect::<Vec<_>>()
        };
        Corpus {
            ids: get("id"),
            titles: get("title"),
            texts: get("text"),
        }
    }

    fn items(&self) -> Vec<ItemIn<'_>> {
        (0..self.ids.len())
            .map(|i| ItemIn {
                id: &self.ids[i],
                title: &self.titles[i],
                text: &self.texts[i],
            })
            .collect()
    }
}

fn table() -> Table {
    Table::open(&data("mini.wst")).unwrap_or_else(|e| panic!("{e}"))
}

/// The page of a manual id (`/manual/dark-mode#how` -> `/manual/dark-mode`); other ids whole.
fn page_of(id: &str) -> &str {
    id.split('#').next().unwrap_or(id)
}

/// (hits in the top 3, queries) for the queries of `set` in `lang` (all if None).
fn recall(
    ix: &Index,
    table: Option<&Table>,
    set: &str,
    lang: Option<&str>,
    opts: &Opts,
    whole_id: bool,
) -> (usize, usize, Vec<String>) {
    let q = json("queries.json");
    let (mut ok, mut n, mut missed) = (0, 0, Vec::new());
    for it in q[set].as_array().cloned().unwrap_or_default() {
        if lang.is_some_and(|l| it["lang"].as_str() != Some(l)) {
            continue;
        }
        n += 1;
        let want: Vec<String> = it["want"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|w| w.as_str().unwrap_or("").to_string())
            .collect();
        let query = it["q"].as_str().unwrap_or("");
        let hits = ix.search(table, query, None, opts);
        let top: Vec<&str> = hits
            .iter()
            .take(3)
            .map(|h| {
                if whole_id {
                    ix.id(h.index)
                } else {
                    page_of(ix.id(h.index))
                }
            })
            .collect();
        // A page that holds the wanted row counts for a wanted page (Settings).
        if want
            .iter()
            .any(|w| top.contains(&w.as_str()) || (!whole_id && top.contains(&page_of(w))))
        {
            ok += 1;
        } else {
            missed.push(format!(
                "{} [{}] -> {:?}",
                query,
                it["lang"].as_str().unwrap_or(""),
                top
            ));
        }
    }
    (ok, n, missed)
}

#[test]
fn manual_questions_find_their_page_in_every_language() {
    let t = table();
    let c = Corpus::load("sections.json");
    let ix = Index::build(&t, &c.items(), 256);
    let opts = Opts::default();
    let mut line = String::new();
    for lang in ["en", "pt-BR", "ja", "fr", "de", "ru", "it"] {
        let (ok, n, missed) = recall(&ix, Some(&t), "manual", Some(lang), &opts, false);
        let (kok, _, _) = recall(&ix, None, "manual", Some(lang), &opts, false);
        line += &format!("{lang}: hybrid {ok}/{n} keyword-only {kok}/{n}; ");
        for m in &missed {
            eprintln!("manual miss: {m}");
        }
        if lang == "en" {
            assert!(ok * 10 >= n * 8, "English recall@3 under 80%: {ok}/{n}");
        } else {
            assert!(ok >= 1, "{lang}: no question found its page ({n} asked)");
        }
    }
    eprintln!("RECALL manual@3 {line}");
    let (ok, n, _) = recall(&ix, Some(&t), "manual", None, &opts, false);
    eprintln!("RECALL manual@3 all {ok}/{n}");
    assert!(
        ok * 10 >= n * 7,
        "recall@3 over all languages under 70%: {ok}/{n}"
    );
}

#[test]
fn settings_questions_find_their_row_without_translated_rows() {
    let t = table();
    let c = Corpus::load("settings_items.json");
    let ix = Index::build(&t, &c.items(), 256);
    let opts = Opts::strict();
    let (ok, n, missed) = recall(&ix, Some(&t), "settings", None, &opts, true);
    for m in &missed {
        eprintln!("settings miss: {m}");
    }
    let (kok, _, _) = recall(&ix, None, "settings", None, &opts, true);
    eprintln!("RECALL settings@3 hybrid {ok}/{n} keyword-only {kok}/{n}");
    // The case the lead named: no pt-BR text anywhere in the index.
    let hits = ix.search(Some(&t), "deixar o texto maior", None, &opts);
    let top: Vec<&str> = hits.iter().take(3).map(|h| ix.id(h.index)).collect();
    assert!(top.contains(&"Accessibility/Larger text"), "{top:?}");
    assert!(ok * 10 >= n * 7, "settings recall@3 under 70%: {ok}/{n}");
}

#[test]
fn nonsense_finds_nothing_and_empty_finds_nothing() {
    let t = table();
    let c = Corpus::load("settings_items.json");
    let ix = Index::build(&t, &c.items(), 256);
    let opts = Opts::strict();
    let q = json("queries.json");
    for s in q["nothing"].as_array().cloned().unwrap_or_default() {
        let s = s.as_str().unwrap_or("");
        let hits = ix.search(Some(&t), s, None, &opts);
        assert!(
            hits.is_empty(),
            "{s:?} -> {:?}",
            hits.iter().map(|h| ix.id(h.index)).collect::<Vec<_>>()
        );
    }
    assert!(ix.search(Some(&t), "", None, &opts).is_empty());
    assert!(ix.search(Some(&t), "   ", None, &opts).is_empty());
}

#[test]
fn a_query_over_two_thousand_items_takes_under_a_millisecond() {
    let t = table();
    let c = Corpus::load("sections.json");
    let mut items = c.items();
    let again = items.clone();
    for _ in 0..3 {
        items.extend(again.iter().copied());
    }
    assert!(items.len() >= 2000);
    let ix = Index::build(&t, &items, 256);
    let opts = Opts::default();
    let queries = [
        "dark mode",
        "how do I install an app",
        "modo escuro",
        "tirar uma captura da tela",
        "wifi password",
        "the laptop battery runs out fast",
    ];
    let mut times = Vec::new();
    for _ in 0..50 {
        for q in queries {
            let t0 = Instant::now();
            let h = ix.search(Some(&t), q, None, &opts);
            times.push(t0.elapsed());
            assert!(!h.is_empty());
        }
    }
    times.sort();
    let (median, worst) = (times[times.len() / 2], times[times.len() - 1]);
    eprintln!(
        "TIMING {} items: median {:?} worst {:?} ({})",
        ix.len(),
        median,
        worst,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    if !cfg!(debug_assertions) {
        assert!(median.as_micros() < 1000, "median {median:?}");
    }
}

/// A pasted page of text (100 000 characters) is cut to the query length before any work: search,
/// spelling and embedding took seconds on it, on the interface thread.
#[test]
fn a_pasted_page_of_text_is_cut_before_any_work() {
    let t = table();
    let c = Corpus::load("sections.json");
    let ix = Index::build(&t, &c.items(), 256);
    let word = "pasword wireles blutooth firewal securty ";
    let paste = word.repeat(100_000 / word.len() + 1);
    assert!(paste.chars().count() >= 100_000);
    let t0 = Instant::now();
    let _ = ix.search(Some(&t), &paste, None, &Opts::default());
    let fixed = ix.did_you_mean(&paste);
    let _ = ix.nearest(&t, &paste, None);
    let spent = t0.elapsed();
    assert!(
        spent.as_millis() < 1500,
        "{spent:?} for a 100k-character paste"
    );
    assert!(
        fixed.is_some_and(|f| f.chars().count() < 2 * wolsem::QUERY_MAX),
        "the suggestion is cut too"
    );
    assert_eq!(wolsem::capped(&paste).chars().count(), wolsem::QUERY_MAX);
    assert_eq!(wolsem::capped("é"), "é");
}
