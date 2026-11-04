// SPDX-License-Identifier: BSD-2-Clause
//! Developer tool: cut a .wst to the tokens that texts use (`prune <full.wst> <texts.txt> <out.wst>`),
//! one text per line. The test fixture and an image table cut to the words it needs are made so.

use std::collections::BTreeSet;
use wolsem::Table;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 4 {
        eprintln!("usage: prune <full.wst> <texts.txt> <out.wst>");
        return;
    }
    let t = Table::open(std::path::Path::new(&a[1])).unwrap_or_else(|e| panic!("{e}"));
    let text = std::fs::read_to_string(&a[2]).unwrap_or_default();
    let mut keep: BTreeSet<usize> = BTreeSet::new();
    for line in text.lines() {
        keep.extend(t.tokens(line));
    }
    let bytes = t.subset(&keep).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(&a[3], &bytes).unwrap_or_else(|e| panic!("{e}"));
    println!(
        "{} of {} tokens, {} bytes",
        keep.len(),
        t.len(),
        bytes.len()
    );
}
