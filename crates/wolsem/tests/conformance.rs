// SPDX-License-Identifier: BSD-2-Clause
//! The Rust reader and the site's JavaScript reader (`site/src/lib/docs-ai/static-embed.js`) agree:
//! `tests/data/conformance.json` holds what the JavaScript made of 27 texts (accents, kana,
//! Hangul, Cyrillic, symbols, empty, over-long) on `tests/data/mini.wst`: the pieces, and the
//! first sixteen values of the 256-dimension vector. Regenerate it with the script next to the
//! site's reader (docs/modules/search.md, "The test table") when the format changes.

use std::path::PathBuf;

use wolsem::Table;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

#[test]
fn the_rust_reader_cuts_and_embeds_like_the_javascript_one() {
    let t = Table::open(&data("mini.wst")).unwrap_or_else(|e| panic!("{e}"));
    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(data("conformance.json")).unwrap_or_default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let cases = v["cases"].as_array().cloned().unwrap_or_default();
    assert!(cases.len() >= 20);
    let dims = usize::try_from(v["dims"].as_u64().unwrap_or(0)).unwrap_or(0);
    let mut with_tokens = 0;
    for c in cases {
        let text = c["text"].as_str().unwrap_or("");
        let want: Vec<&str> = c["tokens"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
            .unwrap_or_default();
        let got: Vec<String> = t
            .tokens(text)
            .into_iter()
            .map(|i| t.token(i).to_string())
            .collect();
        assert_eq!(got, want, "pieces of {text:?}");
        let vec: Vec<f64> = c["vec"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
            .unwrap_or_default();
        match t.embed(text, dims) {
            None => assert!(
                vec.is_empty() && want.is_empty(),
                "{text:?} has pieces but no vector"
            ),
            Some(e) => {
                with_tokens += 1;
                assert_eq!(vec.len(), 16);
                for (k, w) in vec.iter().enumerate() {
                    assert!(
                        (f64::from(e[k]) - w).abs() < 2e-4,
                        "{text:?} value {k}: {} against {w}",
                        e[k]
                    );
                }
            }
        }
    }
    assert!(with_tokens >= 15, "only {with_tokens} texts had pieces");
}
