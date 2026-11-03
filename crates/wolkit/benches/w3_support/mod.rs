// SPDX-License-Identifier: BSD-2-Clause
//! Timing helpers and the budget check shared by the text and batch benches.
//!
//! The benches are plain `harness = false` programs (no external benchmark crate). Each prints
//! `bench <group>.<name> p50=<ns> p95=<ns> n=<samples>` lines and, with `--check` (or
//! `WOLKIT_BENCH_CHECK=1`), exits non-zero when a measurement exceeds its limit in
//! `benches/budgets.toml`. Limits are ceilings with CI noise in mind; lower them by hand after a
//! forge run (the file is a ratchet).

use std::collections::HashMap;

pub struct Bench {
    pub group: &'static str,
    limits: HashMap<String, f64>,
    check: bool,
    pub(crate) failures: Vec<String>,
}

fn parse_budgets(text: &str) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.trim().to_string();
        } else if let Some((k, v)) = line.split_once('=')
            && let Ok(v) = v.trim().replace('_', "").parse::<f64>()
        {
            out.insert(format!("{section}.{}", k.trim()), v);
        }
    }
    out
}

impl Bench {
    pub fn new(group: &'static str) -> Bench {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/benches/budgets.toml");
        let limits = std::fs::read_to_string(path)
            .map(|t| parse_budgets(&t))
            .unwrap_or_default();
        let check = std::env::args().any(|a| a == "--check")
            || std::env::var_os("WOLKIT_BENCH_CHECK").is_some();
        Bench {
            group,
            limits,
            check,
            failures: Vec::new(),
        }
    }

    /// Records a plain number (counts, rates) and checks it against its limit.
    pub fn value(&mut self, name: &str, v: f64) {
        println!("bench {}.{name} value={v}", self.group);
        self.limit(name, v);
    }

    pub(crate) fn limit(&mut self, name: &str, v: f64) {
        if let Some(max) = self.limits.get(&format!("{}.{name}", self.group))
            && v > *max
        {
            self.failures.push(format!(
                "{}.{name} = {v:.0} exceeds the budget {max:.0}",
                self.group
            ));
        }
    }

    pub fn finish(self) {
        if self.failures.is_empty() {
            return;
        }
        for f in &self.failures {
            eprintln!("over budget: {f}");
        }
        if self.check {
            std::process::exit(1);
        }
    }
}

#[test]
fn budget_file_parses() {
    let m = parse_budgets("[text]\nshape_hit_ns = 2_000 # why\n[batch]\ndraws = 10\n");
    assert_eq!(m["text.shape_hit_ns"], 2000.0);
    assert_eq!(m["batch.draws"], 10.0);
}
