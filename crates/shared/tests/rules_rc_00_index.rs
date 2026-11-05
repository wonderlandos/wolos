// SPDX-License-Identifier: BSD-2-Clause
//! The index of the root-cause standards (ADR 0051 "Put the house in order"): the classes of
//! docs/STANDARDS.md section 9, their RH rules in docs/RULES.md, the rules_rc_*.rs files that
//! check them and the table of the decision record must name the same ids. A standard written in
//! one place and missing in another fails here, so the documents cannot drift from the tests.
#[path = "support/rc.rs"]
mod rc;
use std::collections::BTreeSet;

/// (class, the RH rules that enforce it in this crate).
const CLASSES: &[(&str, &[&str])] = &[
    ("RC-01", &["RH-011", "RH-012"]),
    ("RC-03", &["RH-031", "RH-032"]),
    ("RC-13", &["RH-131"]),
    ("RC-23", &["RH-231", "RH-232"]),
    ("RC-16", &["RH-161", "RH-162"]),
    ("RC-14", &["RH-141", "RH-142"]),
    ("RC-22", &["RH-221", "RH-222", "RH-223", "RH-224"]),
    ("RC-19", &["RH-191", "RH-192"]),
    ("RC-07", &["RH-071"]),
    ("RC-02", &["RH-021"]),
    ("RC-39", &["RH-391"]),
    ("RC-29", &["RH-291"]),
    ("RC-30", &["RH-301"]),
    ("RC-34", &["RH-341"]),
    ("RC-45", &["RH-451"]),
];

const ADR: &str = "docs/adr/0051-house-in-order.md";

#[test]
fn every_rh_rule_names_a_test_file_that_checks_it() {
    let rows = rc::rh_rows();
    let ids: BTreeSet<&str> = CLASSES.iter().flat_map(|c| c.1.iter().copied()).collect();
    let listed: BTreeSet<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(listed, ids, "docs/RULES.md RH rows and this index differ");
    for (id, cells) in &rows {
        assert_eq!(
            cells.len(),
            7,
            "{id}: a row is | ID | Rule | Why | Class | How checked |"
        );
        let class = &cells[4];
        assert!(
            CLASSES
                .iter()
                .any(|c| c.0 == class && c.1.contains(&id.as_str())),
            "{id}: docs/RULES.md says class {class}, this index does not"
        );
        let how = &cells[5];
        let file = how
            .split_whitespace()
            .find(|w| w.starts_with("rules_rc_"))
            .unwrap_or("");
        let text = std::fs::read_to_string(rc::root().join("crates/shared/tests").join(file))
            .unwrap_or_else(|e| panic!("{id}: `{how}` names no test file ({e})"));
        assert!(
            text.contains(&format!("\"{id}\"")),
            "{id}: {file} does not check it"
        );
    }
}

#[test]
fn standards_section_nine_lists_every_class_with_its_rules() {
    let doc = rc::read("docs/STANDARDS.md");
    let nine = doc
        .split("\n## 9.")
        .nth(1)
        .expect("docs/STANDARDS.md has a section 9");
    let nine = nine.split("\n## ").next().unwrap_or(nine);
    for (class, ids) in CLASSES {
        let line = nine
            .lines()
            .find(|l| l.contains(&format!("**{class}**")))
            .unwrap_or_else(|| panic!("STANDARDS section 9 has no line for {class}"));
        for id in *ids {
            assert!(
                line.contains(id),
                "STANDARDS section 9 {class} does not name {id}"
            );
        }
    }
}

#[test]
fn the_decision_record_table_must_match_the_index() {
    let adr = rc::read(ADR);
    assert!(
        adr.lines().nth(1).is_some_and(|l| l.starts_with("Status:")),
        "{ADR}: line 2 is the status"
    );
    for (class, ids) in CLASSES {
        let row = adr
            .lines()
            .find(|l| l.starts_with(&format!("| {class} ")))
            .unwrap_or_else(|| panic!("{ADR}: the standards table has no row for {class}"));
        for id in *ids {
            assert!(row.contains(id), "{ADR}: {class} row does not name {id}");
        }
    }
}

#[test]
fn brief_ids_are_read_as_the_plan_writes_them() {
    for b in ["STEP-0", "A2", "A10", "B5", "C8", "D2a", "D7"] {
        assert!(rc::is_brief(b), "{b}");
    }
    for b in ["", "?", "E1", "A", "a2", "STEP"] {
        assert!(!rc::is_brief(b), "{b}");
    }
}
