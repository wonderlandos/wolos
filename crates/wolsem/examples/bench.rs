// SPDX-License-Identifier: BSD-2-Clause
//! Developer probe: where a query's time goes (`bench <dir>`), 2020 items.

use std::time::Instant;
use wolsem::{Index, ItemIn, Opts, Table};

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_default());
    let t0 = Instant::now();
    let table = Table::open(&dir.join("mini.wst")).unwrap_or_else(|e| panic!("{e}"));
    println!(
        "open table ({} tokens, {} bits): {:?}",
        table.len(),
        table.bits(),
        t0.elapsed()
    );
    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("sections.json")).unwrap_or_default(),
    )
    .unwrap_or_default();
    let arr = v.as_array().cloned().unwrap_or_default();
    let g = |k: &str| {
        arr.iter()
            .map(|x| x[k].as_str().unwrap_or("").to_string())
            .collect::<Vec<_>>()
    };
    let (ids, titles, texts) = (g("id"), g("title"), g("text"));
    let mut items: Vec<ItemIn> = (0..ids.len())
        .map(|i| ItemIn {
            id: &ids[i],
            title: &titles[i],
            text: &texts[i],
        })
        .collect();
    let one = items.clone();
    for _ in 0..3 {
        items.extend(one.iter().copied());
    }
    let t0 = Instant::now();
    let ix = Index::build(&table, &items, 256);
    println!("build {} items: {:?}", ix.len(), t0.elapsed());
    let qs = [
        "dark mode",
        "how do I install an app",
        "modo escuro",
        "tirar uma captura da tela",
        "wifi password",
        "the laptop battery runs out fast",
    ];
    let o = Opts::default();
    let rounds = 200;
    let t0 = Instant::now();
    for _ in 0..rounds {
        for q in qs {
            let _ = table.embed(q, 256);
        }
    }
    println!("embed query: {:?}", t0.elapsed() / (rounds * 6));
    let qv: Vec<Vec<f32>> = qs.iter().filter_map(|q| table.embed(q, 256)).collect();
    let t0 = Instant::now();
    for _ in 0..rounds {
        for q in &qv {
            let _ = ix.semantic(q, 32);
        }
    }
    println!("semantic scan: {:?}", t0.elapsed() / (rounds * 6));
    let t0 = Instant::now();
    for _ in 0..rounds {
        for q in qs {
            let _ = ix.search(None, q, None, &o);
        }
    }
    println!("keyword only: {:?}", t0.elapsed() / (rounds * 6));
    let t0 = Instant::now();
    for _ in 0..rounds {
        for q in qs {
            let _ = ix.search(Some(&table), q, None, &o);
        }
    }
    println!("full search: {:?}", t0.elapsed() / (rounds * 6));
}
