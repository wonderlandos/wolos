// SPDX-License-Identifier: GPL-2.0-or-later
//! docs/README.md lists every decision record in `docs/adr/` once, and its Status column starts with
//! the same word as the record's own `Status:` line (accepted, proposed or superseded; a record that
//! is superseded also names the same record in both). Why: the index is the first place a reader
//! looks; an index that says "proposed" over an accepted record sends them to the wrong rule.
#[path = "support/repo.rs"]
mod repo;

use repo::{read, repo};

/// First word of a status text, lowercased, without trailing punctuation.
fn word(status: &str) -> String {
    status
        .trim()
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// `superseded by NNNN` -> `NNNN`.
fn superseded_by(status: &str) -> Option<String> {
    let rest = status.trim().to_ascii_lowercase();
    let rest = rest.strip_prefix("superseded by ")?;
    Some(rest.chars().take_while(char::is_ascii_digit).collect())
}

#[test]
fn adr_index_status_matches_the_records() {
    let index = read("docs/README.md");
    let rows: Vec<(String, String)> = index
        .lines()
        .filter_map(|l| {
            let cells: Vec<&str> = l.split('|').map(str::trim).collect();
            let num = cells.get(1)?.strip_prefix('[')?.split(']').next()?;
            (num.len() == 4 && num.bytes().all(|b| b.is_ascii_digit())).then(|| {
                (
                    num.to_string(),
                    cells.get(3).copied().unwrap_or("").to_string(),
                )
            })
        })
        .collect();

    let mut files: Vec<String> = std::fs::read_dir(repo("docs/adr"))
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".md") && n[..4].bytes().all(|b| b.is_ascii_digit()))
        .collect();
    files.sort();

    let mut bad = Vec::new();
    for f in &files {
        let num = &f[..4];
        let listed: Vec<_> = rows.iter().filter(|(n, _)| n == num).collect();
        if listed.len() != 1 {
            bad.push(format!(
                "{f}: listed {} times in docs/README.md",
                listed.len()
            ));
            continue;
        }
        let text = read(&format!("docs/adr/{f}"));
        let Some(status) = text
            .lines()
            .find_map(|l| l.strip_prefix("Status:"))
            .map(str::to_string)
        else {
            bad.push(format!("{f}: no Status: line"));
            continue;
        };
        let idx = &listed[0].1;
        if word(&status) != word(idx) {
            bad.push(format!(
                "{f}: record says '{}', docs/README.md says '{}'",
                word(&status),
                word(idx)
            ));
        } else if superseded_by(&status) != superseded_by(idx) {
            bad.push(format!("{f}: 'superseded by' differs from docs/README.md"));
        }
    }
    for (n, _) in &rows {
        if !files.iter().any(|f| &f[..4] == n) {
            bad.push(format!("docs/README.md lists {n}, which has no record"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
