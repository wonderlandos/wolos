// SPDX-License-Identifier: GPL-2.0-or-later
//! ADR 0059 D1: the experience is data. `docs/modules/experience.md` lists the 28 surfaces (owner,
//! tier, journey) and what each shipped app provides itself; every shipped launcher entry carries
//! `X-WolOS-Surfaces=`. This test fails when a surface row has no journey, when a journey it names
//! is neither a file in `wolos-iso/tests/wolvm/journeys` nor listed below as not written yet, when an
//! entry claims an id the table does not have, when an entry and its row of the app table differ, and
//! when an entry uses a `X-WolOS-` key the page does not document. Why: a surface claimed in one
//! place and missing in another is how the title strip stayed on the old variant after the Auto
//! switch; a list in prose rots, a table a test reads does not.
#[path = "support/siblings.rs"]
mod siblings;

use std::collections::BTreeSet;
use std::path::PathBuf;

fn repo(p: &str) -> PathBuf {
    siblings::root().join(p)
}

fn read(p: &str) -> String {
    std::fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

const PAGE: &str = "docs/modules/experience.md";

/// Journeys the table names that are not files yet: (journey, the package that writes it). A journey
/// that appears as a file in `wolos-iso/tests/wolvm/journeys` must leave this list (the list only
/// shrinks), and a name no row uses is a stale line.
const JOURNEYS_NOT_YET: &[(&str, &str)] = &[
    ("e2e-window-memory", "E2E-WIN"),
    ("e2e-menus", "E2E-MENU"),
    ("e2e-share-file", "E2E-SHARE"),
    ("e2e-default-apps", "E2E-OPEN"),
    ("e2e-clipboard-image", "E2E-CLIP"),
    ("e2e-quicklook", "E2E-QL"),
    ("e2e-search-notes", "E2E-SEARCH"),
    ("e2e-notifications", "E2E-QA"),
    ("e2e-media-keys", "E2E-ATLAS"),
    ("e2e-resume", "E2E-STATE"),
    ("e2e-services", "E2E-SERVICES"),
    ("e2e-print-pdf", "E2E-ATLAS"),
    ("e2e-strip-follows-auto", "APPS-E2E"),
    ("e2e-memory-saver", "E2E-QA"),
    ("e2e-secrets", "E2E-LIB-2"),
    ("e2e-links-open-in-atlas", "E2E-ATLAS"),
    ("e2e-files-tags", "E2E-FILES"),
];

/// Launcher entries that exist on other branches today and do not carry the key yet: the change that
/// adds the key also removes the name here. The list only shrinks.
const ENTRIES_NOT_YET: &[&str] = &[
    "org.wolos.monitor.desktop",
    "org.wolos.webapp.doctor.desktop",
    "org.wolos.looks.desktop",
    "org.wolos.installer.desktop",
];

/// Where launcher entries that WolOS ships live in the source trees.
const ENTRY_DIRS: &[&str] = &[
    "desktop/overlay/usr/share/applications",
    "desktop/overlay/usr/share/wolos/applications",
    "webapps/launchers",
];

#[derive(Debug, PartialEq)]
struct Surface {
    id: String,
    tier: u8,
    owner: String,
    journeys: Vec<String>,
}

fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().trim_matches('`').to_string())
        .collect()
}

/// The rows of the table under `## <heading>` (the lines that start with `|`, without the header
/// and the rule line).
fn table(page: &str, heading: &str) -> Vec<Vec<String>> {
    let mut in_section = false;
    let mut rows = Vec::new();
    for l in page.lines() {
        if let Some(h) = l.strip_prefix("## ") {
            in_section = h.trim() == heading;
            continue;
        }
        if in_section && l.starts_with('|') {
            rows.push(cells(l));
        }
    }
    rows.into_iter()
        .filter(|r| !r.is_empty() && !r[0].starts_with("---") && r[0] != "Id" && r[0] != "Entry")
        .collect()
}

fn surfaces(page: &str) -> Vec<Surface> {
    table(page, "Surfaces")
        .into_iter()
        .filter(|r| r.len() >= 5)
        .map(|r| Surface {
            id: r[0].clone(),
            tier: r[2].parse().unwrap_or(0),
            owner: r[3].clone(),
            journeys: r[4]
                .split([',', ' '])
                .filter(|j| !j.is_empty())
                .map(String::from)
                .collect(),
        })
        .collect()
}

fn ids(list: &str) -> BTreeSet<String> {
    list.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// The table's own faults: ids S01..S28 in order, a tier of 1..3, an owner and a journey per row.
fn table_faults(rows: &[Surface]) -> Vec<String> {
    let mut bad = Vec::new();
    if rows.len() != 28 {
        bad.push(format!("{} surface rows, not 28", rows.len()));
    }
    for (i, s) in rows.iter().enumerate() {
        if s.id != format!("S{:02}", i + 1) {
            bad.push(format!("row {}: id {} out of order", i + 1, s.id));
        }
        if !(1..=3).contains(&s.tier) {
            bad.push(format!("{}: tier must be 1, 2 or 3", s.id));
        }
        if s.owner.is_empty() {
            bad.push(format!("{}: no owner package", s.id));
        }
        if s.journeys.is_empty() {
            bad.push(format!("{}: no journey", s.id));
        }
        for j in &s.journeys {
            if !j.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                bad.push(format!("{}: journey '{j}' is not a file stem", s.id));
            }
        }
    }
    bad
}

/// The value of `key=` in a desktop entry's `[Desktop Entry]` group, None when absent.
fn entry_key<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .take_while(|l| !l.starts_with("[Desktop Action"))
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
}

/// Faults of one entry against the table ids and the app table (`provides` is its row).
fn entry_faults(
    name: &str,
    text: &str,
    table_ids: &BTreeSet<String>,
    provides: Option<&BTreeSet<String>>,
) -> Vec<String> {
    let mut bad = Vec::new();
    let Some(v) = entry_key(text, "X-WolOS-Surfaces") else {
        bad.push(format!("{name}: no X-WolOS-Surfaces= line (write it empty if the entry is a launcher)"));
        return bad;
    };
    let claimed = ids(v);
    for id in &claimed {
        if !table_ids.contains(id) {
            bad.push(format!("{name}: claims {id}, which is not a surface of {PAGE}"));
        }
    }
    match provides {
        None => bad.push(format!("{name}: no row in the app table of {PAGE}")),
        Some(row) if *row != claimed => bad.push(format!(
            "{name}: the entry says {claimed:?}, the app table says {row:?}"
        )),
        Some(_) => {}
    }
    for l in text.lines() {
        if let Some(k) = l.split('=').next().filter(|k| k.starts_with("X-WolOS-")) {
            if !["X-WolOS-Surfaces", "X-WolOS-Share", "X-WolOS-Service"].contains(&k) {
                bad.push(format!("{name}: key {k} is not documented in {PAGE}"));
            }
        }
    }
    bad
}

/// Every launcher entry WolOS ships from these source trees: (file name, text).
fn entries() -> Vec<(String, String)> {
    let mut dirs: Vec<PathBuf> = ENTRY_DIRS.iter().map(|d| repo(d)).collect();
    if let Ok(apps) = std::fs::read_dir(repo("wolos-crystal/apps")) {
        dirs.extend(apps.flatten().map(|a| a.path()));
    }
    let mut out = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".desktop") {
                if let Ok(t) = std::fs::read_to_string(e.path()) {
                    out.push((name, t));
                }
            }
        }
    }
    out.sort();
    out
}

fn app_table(page: &str) -> Vec<(String, BTreeSet<String>)> {
    table(page, "What each shipped app provides itself")
        .into_iter()
        .filter(|r| r.len() >= 2 && r[0].ends_with(".desktop"))
        .map(|r| (r[0].clone(), ids(&r[1])))
        .collect()
}

#[test]
fn the_page_lists_28_surfaces_each_with_tier_owner_and_journey() {
    let rows = surfaces(&read(PAGE));
    let bad = table_faults(&rows);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn every_journey_of_the_table_is_a_file_or_listed_as_not_written_yet() {
    let rows = surfaces(&read(PAGE));
    let dir = repo("tests/wolvm/journeys");
    assert!(
        dir.is_dir(),
        "{}: the wolos-iso sibling is missing ($WOLOS_ISO)",
        dir.display()
    );
    let named: BTreeSet<&str> = rows
        .iter()
        .flat_map(|s| s.journeys.iter().map(String::as_str))
        .collect();
    let mut bad = Vec::new();
    for j in &named {
        let exists = dir.join(format!("{j}.yaml")).is_file();
        let pending = JOURNEYS_NOT_YET.iter().any(|(n, _)| n == j);
        match (exists, pending) {
            (false, false) => bad.push(format!(
                "{j}: no tests/wolvm/journeys/{j}.yaml and not in JOURNEYS_NOT_YET"
            )),
            (true, true) => bad.push(format!("{j}: the file exists, remove it from JOURNEYS_NOT_YET")),
            _ => {}
        }
    }
    for (j, owner) in JOURNEYS_NOT_YET {
        if !named.contains(j) {
            bad.push(format!("JOURNEYS_NOT_YET lists {j} ({owner}), which no row names"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn every_shipped_entry_declares_what_it_provides_and_the_app_table_agrees() {
    let page = read(PAGE);
    let table_ids: BTreeSet<String> = surfaces(&page).into_iter().map(|s| s.id).collect();
    let rows = app_table(&page);
    let found = entries();
    assert!(found.len() >= 5, "found only {} launcher entries", found.len());
    let mut bad = Vec::new();
    for (name, text) in &found {
        let has_key = entry_key(text, "X-WolOS-Surfaces").is_some();
        if ENTRIES_NOT_YET.contains(&name.as_str()) {
            if has_key {
                bad.push(format!("{name}: has the key, remove it from ENTRIES_NOT_YET"));
            }
            continue;
        }
        let row = rows.iter().find(|(n, _)| n == name).map(|(_, v)| v);
        bad.extend(entry_faults(name, text, &table_ids, row));
    }
    for (name, _) in &rows {
        if !found.iter().any(|(n, _)| n == name) {
            bad.push(format!("the app table lists {name}, which is no shipped entry"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// The checker catches what it exists for (a fail-then-pass proof on synthetic input).
#[test]
fn the_checks_fail_on_the_faults_they_name() {
    let good = "## Surfaces\n\n| Id | Surface | Tier | Owner | Journey |\n|---|---|---|---|---|\n";
    let mut page = String::from(good);
    for i in 1..=28 {
        let j = if i == 7 { "" } else { "e2e-x" };
        page.push_str(&format!("| S{i:02} | a thing | 1 | E2E-X | {j} |\n"));
    }
    let rows = surfaces(&page);
    let bad = table_faults(&rows);
    assert_eq!(bad, ["S07: no journey"], "a row without a journey");

    let ids28: BTreeSet<String> = (1..=28).map(|i| format!("S{i:02}")).collect();
    let row: BTreeSet<String> = ["S04".to_string()].into();
    let entry = "[Desktop Entry]\nName=A\nX-WolOS-Surfaces=S04;S99\nX-WolOS-Colour=red\n";
    let bad = entry_faults("a.desktop", entry, &ids28, Some(&row));
    assert!(bad.iter().any(|b| b.contains("claims S99")), "{bad:?}");
    assert!(bad.iter().any(|b| b.contains("the app table says")), "{bad:?}");
    assert!(bad.iter().any(|b| b.contains("X-WolOS-Colour")), "{bad:?}");

    let bare = "[Desktop Entry]\nName=A\n";
    let bad = entry_faults("a.desktop", bare, &ids28, Some(&row));
    assert!(bad[0].contains("no X-WolOS-Surfaces"), "{bad:?}");

    let ok = "[Desktop Entry]\nName=A\nX-WolOS-Surfaces=S04\n";
    assert!(entry_faults("a.desktop", ok, &ids28, Some(&row)).is_empty());
    assert!(entry_faults("a.desktop", ok, &ids28, None)[0].contains("no row"));
}
