// SPDX-License-Identifier: GPL-2.0-or-later
//! QA-PWR-6 / P2-15 (dry run 0.13.0, p2-cli-power-user): every help text fits a 120-column
//! terminal: `wol help`, and `wol help <command>` for each command of the tree (every prefix).
use controller::cli::{TREE, help};

const COLUMNS: usize = 120;

fn wide(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.chars().count() > COLUMNS)
        .map(|l| format!("{} columns: {l}", l.chars().count()))
        .collect()
}

#[test]
fn every_help_text_fits_120_columns() {
    let mut over = wide(&help(&[]));
    let mut prefixes: Vec<Vec<String>> = Vec::new();
    for c in TREE.iter().filter(|c| !c.hidden) {
        for n in 1..=c.path.len() {
            let p: Vec<String> = c.path[..n].iter().map(|s| s.to_string()).collect();
            if !prefixes.contains(&p) {
                prefixes.push(p);
            }
        }
    }
    for p in &prefixes {
        over.extend(
            wide(&help(p))
                .into_iter()
                .map(|l| format!("wol help {}: {l}", p.join(" "))),
        );
    }
    over.sort();
    over.dedup();
    assert!(
        over.is_empty(),
        "{} help lines over {COLUMNS} columns:\n{}",
        over.len(),
        over.join("\n")
    );
}

/// Wrapping keeps every word, in order, and the table's help column.
#[test]
fn wrapped_help_keeps_every_word() {
    let text = help(&[]);
    let words = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
    let mut want = vec![
        "usage:",
        "wol",
        "<command>",
        "[args]",
        "[--json]",
        "[--yes]",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    for c in TREE.iter().filter(|c| !c.hidden) {
        want.extend(words(&controller::cli::usage_line(c, false)));
        want.extend(words(c.help));
    }
    assert_eq!(words(&text), want);
}
