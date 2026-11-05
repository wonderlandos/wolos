// SPDX-License-Identifier: GPL-2.0-or-later
//! Fixture test of the frozen CDP subset: every `"Domain.name"` literal in the sources is listed in
//! `subset.rs`, and every listed name is used somewhere (no stale rows).
use std::collections::BTreeSet;

const DOMAINS: &[&str] = &[
    "Accessibility",
    "Browser",
    "CSS",
    "DOM",
    "DOMDebugger",
    "Debugger",
    "Emulation",
    "Fetch",
    "IO",
    "Input",
    "Log",
    "Memory",
    "Network",
    "Page",
    "Performance",
    "Runtime",
    "Storage",
    "Target",
    "Tracing",
];

fn sources() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = vec![];
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".rs")
            || name == "subset.rs"
            || name == "pagejs.rs"
            || name == "cursor.rs"
        {
            continue;
        }
        let text = std::fs::read_to_string(e.path()).unwrap();
        // the unit tests use names like "Page.load" as glob subjects
        let code = text.split("#[cfg(test)]").next().unwrap().to_string();
        out.push((name, code));
    }
    out
}

fn literals(code: &str) -> BTreeSet<String> {
    // a double quote, a known domain, a dot, a lower-case method name, a double quote
    let mut found = BTreeSet::new();
    let b = code.as_bytes();
    for (i, _) in code.match_indices('"') {
        let rest = &code[i + 1..];
        for d in DOMAINS {
            if let Some(after) = rest.strip_prefix(d).and_then(|r| r.strip_prefix('.')) {
                let name: String = after
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect();
                let closes = after[name.len()..].starts_with('"');
                if closes && name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
                    found.insert(format!("{d}.{name}"));
                }
            }
        }
        let _ = b;
    }
    found
}

#[test]
fn every_cdp_name_in_the_sources_is_in_the_frozen_subset() {
    let mut missing = vec![];
    for (file, code) in sources() {
        for l in literals(&code) {
            if !atlas_drive::subset::is_frozen(&l) {
                missing.push(format!("{file}: {l}"));
            }
        }
    }
    assert!(missing.is_empty(), "not in subset.rs: {missing:?}");
}

#[test]
fn every_row_of_the_subset_is_used() {
    let all: BTreeSet<String> = sources().iter().flat_map(|(_, c)| literals(c)).collect();
    let stale: Vec<&&str> = atlas_drive::subset::METHODS
        .iter()
        .chain(atlas_drive::subset::EVENTS.iter())
        .filter(|n| !all.contains(**n))
        .collect();
    assert!(stale.is_empty(), "unused rows: {stale:?}");
}

#[test]
fn the_subset_is_about_the_size_the_design_froze() {
    let (m, e) = (
        atlas_drive::subset::METHODS.len(),
        atlas_drive::subset::EVENTS.len(),
    );
    assert!((60..=90).contains(&m), "{m} methods");
    assert!((20..=40).contains(&e), "{e} events");
    // sorted and unique, so a diff of the list is a diff of the subset
    for list in [atlas_drive::subset::METHODS, atlas_drive::subset::EVENTS] {
        let mut s = list.to_vec();
        s.sort();
        s.dedup();
        assert_eq!(s.len(), list.len());
    }
}
