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

/// audit8 standards #14: the `Status:` line starts with one spelling, lowercase (accepted,
/// proposed or superseded), so a reader and a grep find the same word in every record.
#[test]
fn adr_status_words_are_lowercase() {
    let mut bad = Vec::new();
    for e in std::fs::read_dir(repo("docs/adr")).unwrap().flatten() {
        let f = e.file_name().into_string().unwrap();
        if !f.ends_with(".md") || !f[..4].bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let text = read(&format!("docs/adr/{f}"));
        let status = text.lines().find_map(|l| l.strip_prefix("Status:"));
        let first: String = status
            .unwrap_or("")
            .trim()
            .chars()
            .take_while(char::is_ascii_alphabetic)
            .collect();
        if !["accepted", "proposed", "superseded"].contains(&first.as_str()) {
            bad.push(format!("{f}: Status starts with '{first}'"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// audit8 standards #13 (docs/STANDARDS.md section 5): a module document names its ADR in the
/// title. These have no decision record of their own yet; a new document needs one, or a line
/// here with the reason. shards.md (wave 3 CR-4, `wol shard`): no record decides the curated
/// shard list yet. windows.md (tiling and snapping, lead order 2026-11-04): none decides the
/// window arrangement yet.
#[test]
fn module_docs_name_their_adr_in_the_title() {
    const NO_ADR_YET: &[&str] = &[
        "hig.md",
        "performance.md",
        "vm-graphics.md",
        "shards.md",
        "windows.md",
    ];
    let mut bad = Vec::new();
    for e in std::fs::read_dir(repo("docs/modules")).unwrap().flatten() {
        let f = e.file_name().into_string().unwrap();
        if !f.ends_with(".md") || NO_ADR_YET.contains(&f.as_str()) {
            continue;
        }
        let text = read(&format!("docs/modules/{f}"));
        let title = text.lines().next().unwrap_or("");
        if !title.contains("(ADR ") {
            bad.push(format!("{f}: title '{title}' names no ADR"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
